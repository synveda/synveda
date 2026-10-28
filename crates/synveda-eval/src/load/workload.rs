//! One simulated agent: its public-API calls and how each outcome is
//! classified. An agent's own calls run sequentially, as a harness's hooks
//! do, and every measurement starts at the scheduled time, so time spent
//! behind the agent's previous call is part of the next call's latency.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::{Mutex, Semaphore};

use super::profile::{EventSizes, Mix};
use super::schedule::{Op, Rng};
use super::stats::{Measure, Outcome, Tally};

/// Every operation name a tally may hold; a checkpoint naming another is
/// refused on resume.
pub const KNOWN_OPS: [&str; 25] = [
    "append",
    "context_run",
    "knowledge_search",
    "knowledge_create",
    "knowledge_create_unreviewed",
    "stale_edit",
    "stale_apply",
    "denied_probe",
    "session_open",
    "session_end",
    "capture_request",
    "capture_poll",
    "capture_lag",
    "candidate_list",
    "candidate_accept",
    "knowledge_edit",
    "proposal_read",
    "proposal_approve",
    "proposal_apply",
    "me",
    "context_run_degraded",
    "knowledge_search_degraded",
    "isolation_breach",
    "provider_embed",
    "unsent",
];

const MAX_PENDING_BATCHES: usize = 8;
const ACCEPTS_PER_BATCH: usize = 2;
const SYLLABLES: [&str; 20] = [
    "ka", "lo", "mi", "ne", "ru", "sa", "te", "vo", "zi", "da", "fe", "go", "hu", "ji", "pa",
    "qui", "ro", "su", "ta", "wy",
];
const WORDS: [&str; 24] = [
    "release",
    "checklist",
    "rollback",
    "correlation",
    "header",
    "migration",
    "review",
    "convention",
    "retry",
    "budget",
    "latency",
    "schema",
    "deploy",
    "incident",
    "runbook",
    "cache",
    "queue",
    "policy",
    "audit",
    "session",
    "context",
    "fixture",
    "gateway",
    "worker",
];

/// What the workload expects a call to return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    Success,
    Refusal,
    /// Accepted for review, or refused as stale before review.
    PendingOrStale,
    /// A reviewed change whose result reports `applied`.
    Applied,
    /// A reviewed change with a stale precondition: refused at apply, or
    /// already closed as rejected.
    StaleApply,
}

/// Pure classification of a completed call. Reviewed applies answer `200`
/// with the change's recorded result, so they are judged by its closed
/// `outcome` value rather than the status alone.
pub fn classify(expect: Expect, status: StatusCode, body: &Value) -> Outcome {
    if status == StatusCode::TOO_MANY_REQUESTS {
        return Outcome::Limited;
    }
    let refused = matches!(
        status,
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
    );
    let stale = matches!(
        status,
        StatusCode::CONFLICT | StatusCode::PRECONDITION_FAILED
    );
    let outcome = body["outcome"].as_str();
    match expect {
        Expect::Success | Expect::PendingOrStale if status.is_success() => Outcome::Ok,
        Expect::Applied if status.is_success() && outcome == Some("applied") => Outcome::Ok,
        Expect::Refusal if refused => Outcome::Denied,
        Expect::PendingOrStale | Expect::StaleApply if stale => Outcome::Stale,
        Expect::StaleApply if status.is_success() && outcome == Some("rejected") => Outcome::Stale,
        _ => Outcome::Unexpected,
    }
}

/// Only a successful cross-tenant read is a breach; a 5xx or transport
/// failure on the probe is already counted as an unexpected outcome.
fn isolation_breach(op: &str, status: Option<StatusCode>) -> bool {
    op == "denied_probe" && status.is_some_and(|status| status.is_success())
}

/// A bearer and the project its Sessions run in.
#[derive(Debug, Clone)]
pub struct Actor {
    pub token: String,
    pub workspace_id: String,
    pub project_id: String,
}

/// The tenant's named reviewers and the distinct identity that applies an
/// approved change, as the evaluation fixture grants them.
#[derive(Debug, Clone)]
pub struct Reviewers {
    pub approvers: [String; 2],
    pub publisher: String,
}

/// Shared, bounded context for every agent of one run.
pub struct Shared {
    pub http: reqwest::Client,
    pub gateway: String,
    pub in_flight: Semaphore,
    pub timeout: Duration,
    pub mix: Mix,
    pub sizes: EventSizes,
    /// Session ids of the other tenant, for cross-tenant refusal probes.
    pub foreign_sessions: [Mutex<Vec<String>>; 2],
    pub reviewers: [Reviewers; 2],
}

type Request = (Method, String, Option<String>, Option<Value>);

/// Sends one call within the generator's in-flight bound.
async fn send(
    shared: &Shared,
    bearer: &str,
    request: &Request,
) -> Result<(StatusCode, Value), Outcome> {
    let _permit = shared
        .in_flight
        .acquire()
        .await
        .map_err(|_| Outcome::Transport)?;
    let (method, path, key, body) = request;
    let mut builder = shared
        .http
        .request(method.clone(), format!("{}{path}", shared.gateway))
        .bearer_auth(bearer)
        .timeout(shared.timeout);
    if let Some(key) = key {
        builder = builder.header("Idempotency-Key", key);
    }
    if let Some(body) = body {
        builder = builder.json(body);
    }
    let response = builder.send().await.map_err(|err| {
        if err.is_timeout() {
            Outcome::Timeout
        } else {
            Outcome::Transport
        }
    })?;
    let status = response.status();
    Ok((
        status,
        response.json::<Value>().await.unwrap_or(Value::Null),
    ))
}

/// Performs one call and records its outcome and latencies; returns the body
/// of an expected success.
async fn measured(
    shared: &Shared,
    tally: &Mutex<Tally>,
    bearer: &str,
    op: &'static str,
    intended: Instant,
    expect: Expect,
    request: Request,
) -> Option<Value> {
    let started = Instant::now();
    let result = send(shared, bearer, &request).await;
    let finished = Instant::now();
    let (outcome, status, body) = match result {
        Ok((status, body)) => (classify(expect, status, &body), Some(status), Some(body)),
        Err(outcome) => (outcome, None, None),
    };
    let mut tally = tally.lock().await;
    tally.outcome(op, outcome);
    if let (Outcome::Unexpected, Some(status)) = (outcome, status) {
        tally.unexpected_status(op, status.as_u16());
    }
    tally.latency(op, Measure::Response, finished - intended);
    tally.latency(op, Measure::Service, finished - started);
    tally.latency(
        op,
        Measure::Lag,
        started.saturating_duration_since(intended),
    );
    if isolation_breach(op, status) {
        tally.outcome("isolation_breach", Outcome::Unexpected);
    }
    (outcome == Outcome::Ok).then_some(body).flatten()
}

/// Completes a pending typed change as the tenant's reviewers would: each
/// approver votes on the exact commit until it leaves `open`, then the
/// publisher applies it. Returns the applied result, or the immediate result
/// when nothing was pending.
async fn govern(
    shared: &Shared,
    tally: &Mutex<Tally>,
    tenant: usize,
    result: Value,
    apply: (&'static str, Expect),
) -> Option<Value> {
    if result["outcome"] != "pending_review" {
        return Some(result);
    }
    let change = result["change_id"].as_str()?.to_owned();
    let reviewers = &shared.reviewers[tenant];
    for approver in &reviewers.approvers {
        let read = (Method::GET, format!("/v1/proposals/{change}"), None, None);
        let proposal = measured(
            shared,
            tally,
            approver,
            "proposal_read",
            Instant::now(),
            Expect::Success,
            read,
        )
        .await?;
        if proposal["state"] != "open" {
            break;
        }
        let vote = (
            Method::POST,
            format!("/v1/proposals/{change}/approve"),
            None,
            Some(json!({"expected_commit": proposal["commit"]})),
        );
        measured(
            shared,
            tally,
            approver,
            "proposal_approve",
            Instant::now(),
            Expect::Success,
            vote,
        )
        .await?;
    }
    let request = (
        Method::POST,
        format!("/v1/proposals/{change}/apply"),
        None,
        Some(json!({})),
    );
    measured(
        shared,
        tally,
        &reviewers.publisher,
        apply.0,
        Instant::now(),
        apply.1,
        request,
    )
    .await
}

/// State a background flow shares with its agent.
#[derive(Clone)]
struct Flow {
    shared: Arc<Shared>,
    tally: Arc<Mutex<Tally>>,
    tenant: usize,
    bearer: String,
    item: Arc<std::sync::Mutex<Option<(String, String)>>>,
    batches: Arc<std::sync::Mutex<VecDeque<(String, Instant)>>>,
    busy: Arc<AtomicBool>,
}

impl Flow {
    fn set_item(&self, item: Option<(String, String)>) {
        if let Ok(mut slot) = self.item.lock() {
            *slot = item;
        }
    }

    /// Reviews a pending creation and remembers the applied item.
    async fn create(self, created: Value) {
        let applied = govern(
            &self.shared,
            &self.tally,
            self.tenant,
            created,
            ("proposal_apply", Expect::Applied),
        )
        .await;
        if let Some(applied) = applied
            && let (Some(item), Some(revision)) = (
                applied["knowledge_item_id"].as_str(),
                applied["revision_id"].as_str(),
            )
        {
            self.set_item(Some((item.to_owned(), revision.to_owned())));
        }
        self.busy.store(false, Ordering::Release);
    }

    /// Reviews an edit, then repeats it against the revision it replaced. A
    /// reviewed edit carries its expected revision as a precondition, so the
    /// stale one may be accepted for review and must then fail to apply.
    async fn edit(self, edited: Value, item: String, stale: Request) {
        let applied = govern(
            &self.shared,
            &self.tally,
            self.tenant,
            edited,
            ("proposal_apply", Expect::Applied),
        )
        .await;
        match applied
            .as_ref()
            .and_then(|body| body["revision_id"].as_str())
        {
            Some(next) => self.set_item(Some((item, next.to_owned()))),
            None => {
                self.set_item(None);
                self.busy.store(false, Ordering::Release);
                return;
            }
        }
        if let Some(pending) = measured(
            &self.shared,
            &self.tally,
            &self.bearer,
            "stale_edit",
            Instant::now(),
            Expect::PendingOrStale,
            stale,
        )
        .await
        {
            govern(
                &self.shared,
                &self.tally,
                self.tenant,
                pending,
                ("stale_apply", Expect::StaleApply),
            )
            .await;
        }
        self.busy.store(false, Ordering::Release);
    }

    /// Polls the oldest frozen Session; once its Capture completes, accepts a
    /// bounded number of candidates into the agent's private scope.
    async fn review(self, scope: String) {
        self.review_one(scope).await;
        self.busy.store(false, Ordering::Release);
    }

    async fn review_one(&self, scope: String) {
        let next = self
            .batches
            .lock()
            .ok()
            .and_then(|batches| batches.front().cloned());
        let Some((batch, requested)) = next else {
            return;
        };
        let (shared, tally) = (&*self.shared, &*self.tally);
        let poll = (
            Method::GET,
            format!("/v1/capture-batches/{batch}"),
            None,
            None,
        );
        let Some(state) = measured(
            shared,
            tally,
            &self.bearer,
            "capture_poll",
            Instant::now(),
            Expect::Success,
            poll,
        )
        .await
        else {
            return;
        };
        let completed = match state["state"].as_str() {
            Some("completed") => true,
            Some("failed") => false,
            _ => return,
        };
        if let Ok(mut batches) = self.batches.lock() {
            batches.pop_front();
        }
        {
            let mut tally = tally.lock().await;
            if completed {
                tally.outcome("capture_lag", Outcome::Ok);
                tally.latency("capture_lag", Measure::Response, requested.elapsed());
            } else {
                tally.outcome("capture_lag", Outcome::Unexpected);
                return;
            }
        }
        self.accept_candidates(&batch, scope).await;
    }

    /// Accepts up to `ACCEPTS_PER_BATCH` pending candidates of a completed
    /// batch into the agent's scope, each through reviewed governance.
    async fn accept_candidates(&self, batch: &str, scope: String) {
        let (shared, tally) = (&*self.shared, &*self.tally);
        let list = (
            Method::GET,
            format!("/v1/capture-candidates?batch_id={batch}&limit=50"),
            None,
            None,
        );
        let Some(page) = measured(
            shared,
            tally,
            &self.bearer,
            "candidate_list",
            Instant::now(),
            Expect::Success,
            list,
        )
        .await
        else {
            return;
        };
        let pending: Vec<String> = page["candidates"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|candidate| candidate["state"] == "pending")
            .filter_map(|candidate| candidate["id"].as_str().map(str::to_owned))
            .take(ACCEPTS_PER_BATCH)
            .collect();
        for candidate in pending {
            let accept = (
                Method::POST,
                format!("/v1/capture-candidates/{candidate}/accept"),
                Some(format!("load-accept-{candidate}")),
                Some(json!({"scope_id": scope, "project_id": null})),
            );
            if let Some(decision) = measured(
                shared,
                tally,
                &self.bearer,
                "candidate_accept",
                Instant::now(),
                Expect::Success,
                accept,
            )
            .await
            {
                let pending = json!({
                    "outcome": decision["candidate"]["resulting_outcome"],
                    "change_id": decision["candidate"]["resulting_change_id"],
                });
                govern(
                    shared,
                    tally,
                    self.tenant,
                    pending,
                    ("proposal_apply", Expect::Applied),
                )
                .await;
            }
        }
    }
}

/// One agent: its scheduled calls run in order, as a harness's hooks do,
/// while review by other identities happens in at most one bounded
/// background flow, so a reviewer's work never delays the agent's next call.
pub struct Agent {
    pub index: u32,
    pub actor: Actor,
    /// 0 for the measured tenant, 1 for the noisy tenant.
    pub tenant: usize,
    pub rng: Rng,
    session: Option<String>,
    appended: u32,
    sequence: u64,
    private_scope: Option<String>,
    item: Arc<std::sync::Mutex<Option<(String, String)>>>,
    batches: Arc<std::sync::Mutex<VecDeque<(String, Instant)>>>,
    busy: Arc<AtomicBool>,
}

impl Agent {
    pub fn new(index: u32, actor: Actor, tenant: usize, seed: u64) -> Self {
        Self {
            index,
            actor,
            tenant,
            rng: Rng::new(seed),
            session: None,
            appended: 0,
            sequence: 0,
            private_scope: None,
            item: Arc::new(std::sync::Mutex::new(None)),
            batches: Arc::new(std::sync::Mutex::new(VecDeque::new())),
            busy: Arc::new(AtomicBool::new(false)),
        }
    }

    fn key(&mut self, what: &str) -> String {
        self.sequence += 1;
        format!(
            "load-{}-{}-{what}-{}-{}",
            self.tenant,
            self.index,
            self.sequence,
            self.rng.next_u64()
        )
    }

    /// Shared topic words, which queries also use.
    fn sentence(&mut self, words: usize) -> String {
        (0..words)
            .map(|_| WORDS[self.rng.below(WORDS.len() as u64) as usize])
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Mostly invented words, so unrelated items are not near-duplicates of
    /// one another for the product's conflict detection.
    fn distinct(&mut self, words: usize) -> String {
        let mut text = self.sentence(3);
        for _ in 0..words {
            text.push(' ');
            for _ in 0..3 {
                text.push_str(SYLLABLES[self.rng.below(SYLLABLES.len() as u64) as usize]);
            }
        }
        text
    }

    /// A background flow, unless one is already running for this agent.
    fn flow(&self, shared: &Arc<Shared>, tally: &Arc<Mutex<Tally>>) -> Option<Flow> {
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()?;
        Some(Flow {
            shared: Arc::clone(shared),
            tally: Arc::clone(tally),
            tenant: self.tenant,
            bearer: self.actor.token.clone(),
            item: Arc::clone(&self.item),
            batches: Arc::clone(&self.batches),
            busy: Arc::clone(&self.busy),
        })
    }

    async fn own(
        &self,
        shared: &Shared,
        tally: &Mutex<Tally>,
        op: &'static str,
        intended: Instant,
        expect: Expect,
        request: Request,
    ) -> Option<Value> {
        measured(
            shared,
            tally,
            &self.actor.token,
            op,
            intended,
            expect,
            request,
        )
        .await
    }

    /// Runs one scheduled operation, opening or rotating the Session first
    /// when it needs one.
    pub async fn perform(
        &mut self,
        shared: &Arc<Shared>,
        tally: &Arc<Mutex<Tally>>,
        op: Op,
        intended: Instant,
    ) {
        match op {
            Op::Append => self.append(shared, tally, intended).await,
            Op::ContextRun => self.context_run(shared, tally, intended).await,
            Op::KnowledgeSearch => self.search(shared, tally, intended).await,
            Op::KnowledgeCreate => self.create(shared, tally, intended).await,
            Op::StaleEdit => self.stale_edit(shared, tally, intended).await,
            Op::DeniedProbe => self.denied_probe(shared, tally, intended).await,
        }
    }

    async fn session(&mut self, shared: &Shared, tally: &Mutex<Tally>) -> Option<String> {
        if let Some(session) = &self.session {
            return Some(session.clone());
        }
        let key = self.key("session");
        let body = json!({
            "workspace_id": self.actor.workspace_id,
            "project_id": self.actor.project_id,
            "client_name": "synveda-eval-load",
            "external_session_id": key,
            "task_summary": "Synthetic load session",
        });
        let request = (
            Method::POST,
            "/v1/sessions".to_owned(),
            Some(key),
            Some(body),
        );
        let created = self
            .own(
                shared,
                tally,
                "session_open",
                Instant::now(),
                Expect::Success,
                request,
            )
            .await?;
        let id = created["id"].as_str()?.to_owned();
        let mut foreign = shared.foreign_sessions[self.tenant].lock().await;
        if foreign.len() < 64 {
            foreign.push(id.clone());
        }
        self.session = Some(id.clone());
        self.appended = 0;
        Some(id)
    }

    async fn append(&mut self, shared: &Arc<Shared>, tally: &Arc<Mutex<Tally>>, intended: Instant) {
        let Some(session) = self.session(shared, tally).await else {
            return;
        };
        let large = self.rng.below(100) < u64::from(shared.sizes.large_percent);
        let target = if large {
            shared.sizes.large_bytes
        } else {
            shared.sizes.small_bytes
        } as usize;
        let mut text = String::with_capacity(target + 64);
        while text.len() < target {
            let sentence = self.distinct(12);
            text.push_str(&sentence);
            text.push_str(". ");
        }
        text.truncate(target);
        let key = self.key("event");
        let body = json!({"events": [{
            "client_event_id": key,
            "event_type": "message.assistant",
            "occurred_at": chrono::Utc::now().to_rfc3339(),
            "payload": {"text": text},
        }]});
        let request = (
            Method::POST,
            format!("/v1/sessions/{session}/events"),
            None,
            Some(body),
        );
        if self
            .own(shared, tally, "append", intended, Expect::Success, request)
            .await
            .is_some()
        {
            self.appended += 1;
        }
        if self.appended >= shared.mix.events_per_session {
            self.rotate(shared, tally, &session).await;
        }
    }

    async fn rotate(&mut self, shared: &Shared, tally: &Mutex<Tally>, session: &str) {
        self.session = None;
        let end = (
            Method::POST,
            format!("/v1/sessions/{session}/end"),
            None,
            Some(json!({"status": "ended"})),
        );
        self.own(
            shared,
            tally,
            "session_end",
            Instant::now(),
            Expect::Success,
            end,
        )
        .await;
        let key = self.key("capture");
        let capture = (
            Method::POST,
            format!("/v1/sessions/{session}/capture-batches"),
            Some(key),
            Some(json!({})),
        );
        if let Some(batch) = self
            .own(
                shared,
                tally,
                "capture_request",
                Instant::now(),
                Expect::Success,
                capture,
            )
            .await
            && let Some(id) = batch["id"].as_str()
            && let Ok(mut batches) = self.batches.lock()
            && batches.len() < MAX_PENDING_BATCHES
        {
            batches.push_back((id.to_owned(), Instant::now()));
        }
    }

    async fn context_run(
        &mut self,
        shared: &Arc<Shared>,
        tally: &Arc<Mutex<Tally>>,
        intended: Instant,
    ) {
        let Some(session) = self.session(shared, tally).await else {
            return;
        };
        let query = self.sentence(5);
        let key = self.key("context");
        let request = (
            Method::POST,
            format!("/v1/sessions/{session}/context-runs"),
            Some(key),
            Some(json!({"query": query, "budget_tokens": 1500})),
        );
        if let Some(body) = self
            .own(
                shared,
                tally,
                "context_run",
                intended,
                Expect::Success,
                request,
            )
            .await
            && body
                .pointer("/trace/degradation")
                .or_else(|| body.get("degradation"))
                .is_some_and(Value::is_string)
        {
            tally
                .lock()
                .await
                .outcome("context_run_degraded", Outcome::Ok);
        }
        let has_batch = self.batches.lock().is_ok_and(|batches| !batches.is_empty());
        if has_batch
            && let Some(scope) = self.private_scope(shared, tally).await
            && let Some(flow) = self.flow(shared, tally)
        {
            tokio::spawn(flow.review(scope));
        }
    }

    async fn search(&mut self, shared: &Shared, tally: &Mutex<Tally>, intended: Instant) {
        let query = self.sentence(3).replace(' ', "+");
        let path = format!(
            "/v1/knowledge?query={query}&workspace_id={}&limit=20",
            self.actor.workspace_id
        );
        if let Some(body) = self
            .own(
                shared,
                tally,
                "knowledge_search",
                intended,
                Expect::Success,
                (Method::GET, path, None, None),
            )
            .await
            && body.get("degradation").is_some_and(Value::is_string)
        {
            tally
                .lock()
                .await
                .outcome("knowledge_search_degraded", Outcome::Ok);
        }
    }

    fn content(&mut self) -> Value {
        let title = self.distinct(4);
        let body = self.distinct(30);
        json!({
            "title": title,
            "body_markdown": body,
            "summary": title,
            "tags": ["load"],
            "sensitivity": "internal",
            "confidence_permille": 800,
        })
    }

    async fn private_scope(&mut self, shared: &Shared, tally: &Mutex<Tally>) -> Option<String> {
        if self.private_scope.is_none() {
            let request = (Method::GET, "/v1/me".to_owned(), None, None);
            let me = self
                .own(
                    shared,
                    tally,
                    "me",
                    Instant::now(),
                    Expect::Success,
                    request,
                )
                .await?;
            self.private_scope = me["anchors"]
                .as_array()?
                .iter()
                .find(|anchor| anchor["source"] == "principal_scope")
                .and_then(|anchor| anchor["scope_id"].as_str())
                .map(str::to_owned);
        }
        self.private_scope.clone()
    }

    async fn create(&mut self, shared: &Arc<Shared>, tally: &Arc<Mutex<Tally>>, intended: Instant) {
        let Some(scope) = self.private_scope(shared, tally).await else {
            return;
        };
        let key = self.key("knowledge");
        let body = json!({
            "scope_id": scope,
            "knowledge_type": "fact",
            "origin": "authored",
            "content": self.content(),
        });
        let request = (
            Method::POST,
            "/v1/knowledge".to_owned(),
            Some(key),
            Some(body),
        );
        let Some(created) = self
            .own(
                shared,
                tally,
                "knowledge_create",
                intended,
                Expect::Success,
                request,
            )
            .await
        else {
            return;
        };
        match self.flow(shared, tally) {
            Some(flow) => {
                tokio::spawn(flow.create(created));
            }
            None => tally
                .lock()
                .await
                .outcome("knowledge_create_unreviewed", Outcome::Unsent),
        }
    }

    /// Edits the agent's own item; its review and the stale repeat run in the
    /// background flow.
    async fn stale_edit(
        &mut self,
        shared: &Arc<Shared>,
        tally: &Arc<Mutex<Tally>>,
        intended: Instant,
    ) {
        let current = self.item.lock().ok().and_then(|item| item.clone());
        let Some((item, revision)) = current else {
            return self.create(shared, tally, intended).await;
        };
        if self.busy.load(Ordering::Acquire) {
            return;
        }
        let path = format!("/v1/knowledge/{item}");
        let edit = json!({"expected_revision_id": revision, "content": self.content()});
        let key = self.key("edit");
        let request = (Method::PATCH, path.clone(), Some(key), Some(edit));
        let Some(edited) = self
            .own(
                shared,
                tally,
                "knowledge_edit",
                intended,
                Expect::Success,
                request,
            )
            .await
        else {
            return;
        };
        let stale_body = json!({"expected_revision_id": revision, "content": self.content()});
        let stale = (
            Method::PATCH,
            path,
            Some(self.key("stale")),
            Some(stale_body),
        );
        if let Some(flow) = self.flow(shared, tally) {
            tokio::spawn(flow.edit(edited, item, stale));
        }
    }

    async fn denied_probe(&mut self, shared: &Shared, tally: &Mutex<Tally>, intended: Instant) {
        let other = 1 - self.tenant;
        let target = {
            let sessions = shared.foreign_sessions[other].lock().await;
            if sessions.is_empty() {
                return;
            }
            sessions[self.rng.below(sessions.len() as u64) as usize].clone()
        };
        let request = (Method::GET, format!("/v1/sessions/{target}"), None, None);
        self.own(
            shared,
            tally,
            "denied_probe",
            intended,
            Expect::Refusal,
            request,
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_follow_the_expectation() {
        let applied = json!({"outcome": "applied"});
        let rejected = json!({"outcome": "rejected"});
        let pending = json!({"outcome": "pending_review"});
        let none = Value::Null;
        let cases = [
            (Expect::Applied, StatusCode::OK, &applied, Outcome::Ok),
            (
                Expect::Applied,
                StatusCode::OK,
                &pending,
                Outcome::Unexpected,
            ),
            (
                Expect::StaleApply,
                StatusCode::OK,
                &rejected,
                Outcome::Stale,
            ),
            (
                Expect::StaleApply,
                StatusCode::OK,
                &applied,
                Outcome::Unexpected,
            ),
            (
                Expect::StaleApply,
                StatusCode::CONFLICT,
                &none,
                Outcome::Stale,
            ),
            (Expect::Success, StatusCode::CREATED, &none, Outcome::Ok),
            (
                Expect::Success,
                StatusCode::NOT_FOUND,
                &none,
                Outcome::Unexpected,
            ),
            (
                Expect::Success,
                StatusCode::INTERNAL_SERVER_ERROR,
                &none,
                Outcome::Unexpected,
            ),
            (
                Expect::Refusal,
                StatusCode::NOT_FOUND,
                &none,
                Outcome::Denied,
            ),
            (Expect::Refusal, StatusCode::OK, &none, Outcome::Unexpected),
            (
                Expect::PendingOrStale,
                StatusCode::CREATED,
                &none,
                Outcome::Ok,
            ),
            (
                Expect::PendingOrStale,
                StatusCode::CONFLICT,
                &none,
                Outcome::Stale,
            ),
            (
                Expect::PendingOrStale,
                StatusCode::NOT_FOUND,
                &none,
                Outcome::Unexpected,
            ),
        ];
        for (expect, status, body, outcome) in cases {
            assert_eq!(
                classify(expect, status, body),
                outcome,
                "{expect:?} {status}"
            );
        }
        for expect in [
            Expect::Success,
            Expect::Refusal,
            Expect::PendingOrStale,
            Expect::Applied,
            Expect::StaleApply,
        ] {
            assert_eq!(
                classify(expect, StatusCode::TOO_MANY_REQUESTS, &none),
                Outcome::Limited
            );
        }
    }

    #[test]
    fn only_a_successful_cross_tenant_read_is_a_breach() {
        assert!(isolation_breach("denied_probe", Some(StatusCode::OK)));
        for status in [
            Some(StatusCode::NOT_FOUND),
            Some(StatusCode::FORBIDDEN),
            Some(StatusCode::INTERNAL_SERVER_ERROR),
            Some(StatusCode::SERVICE_UNAVAILABLE),
            None,
        ] {
            assert!(!isolation_breach("denied_probe", status), "{status:?}");
        }
        assert!(!isolation_breach("session_open", Some(StatusCode::OK)));
    }
}
