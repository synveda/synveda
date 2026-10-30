//! Acceptance evidence for Capture retry scheduling and provider backpressure.
//!
//! Every case runs the real worker claim path, embedded PDP, forced-RLS store
//! and hash-chained audit as the ordinary runtime role against a scripted
//! OpenAI-compatible provider. Ordering comes from database state witnesses
//! and provider notifications, never from sleeping until a deadline: a test
//! that needs a deadline to pass rewrites it to database time, which is what
//! an elapsed wait would produce.

#[path = "../../synveda-store/tests/support/tenant_fixture.rs"]
mod tenant_fixture;

#[path = "support/configuration.rs"]
mod configuration_support;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use chrono::{DateTime, Utc};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, behavior_test_router as router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_ingest::capture_worker;
use synveda_ingest::extraction::{AnyExtractor, VllmExtractor};
use synveda_ingest::retry::RetryPolicy;
use synveda_store::capture as capture_store;
use synveda_store::{access, identities, rls, scopes};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::capture::{CaptureBatch, CaptureBatchState, CaptureTerminalReason};
use synveda_types::{
    CaptureBatchId, GrantId, IdentityId, IdentityKind, ScopeId, TenantId, TenantStatus,
};
use tokio::sync::Notify;
use tower::ServiceExt;

const SECRET: &[u8] = b"capture-retry-acceptance";
const ADMIN: &str = "capture-retry-admin";
const OUTSIDER: &str = "capture-retry-outsider";
const BODY_SENTINEL: &str = "SYNVEDA_SECRET_SENTINEL_PROVIDER_BODY";

async fn serial() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn state(url: &str) -> AppState {
    AppState {
        pool: PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(5))
            .connect_lazy(url)
            .expect("parse database url"),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp: Arc::new(synveda_policy::Pdp::new().expect("build embedded PDP")),
        service_token_max_ttl: Duration::from_secs(3600),
        embedder: Arc::new(synveda_ingest::embedding::AnyEmbedder::Deterministic(
            synveda_ingest::embedding::DeterministicEmbedder::new(),
        )),
        context_embed_timeout: Duration::from_millis(100),
        git_sources: None,
        git_exports: None,
        keys: Arc::new(synveda_store::keys::KeyRing::new(
            synveda_crypto::Kms::Disabled,
        )),
    }
}

fn issue(subject: &str, tenant_id: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue(subject, tenant_id, Duration::from_secs(300))
}

async fn seed_identity(tx: &mut sqlx::PgConnection, tenant_id: TenantId, subject: &str) {
    let own = scopes::ensure_principal_scope(tx, tenant_id, subject, subject)
        .await
        .expect("mint principal scope");
    identities::create(
        tx,
        IdentityId::new(),
        tenant_id,
        Some(subject),
        IdentityKind::User,
        None,
        Some(subject),
        own.id,
    )
    .await
    .expect("create identity");
}

/// An admitted tenant with an administrator and an ungranted outsider. The
/// pack binding is test Configuration; every request still crosses the PDP.
async fn admitted_tenant() -> Option<(AppState, TenantId, ScopeId)> {
    let url = tenant_fixture::database_url("capture retry test")?;
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::epoch::verify(&pool)
        .await
        .expect("database is at the current head");
    let tenant_id = TenantId::new();
    tenant_fixture::create(
        &pool,
        tenant_id,
        &format!("retry-{}", tenant_id.as_uuid().simple()),
        "capture retry acceptance",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    let mut tx = rls::begin_tenant_tx(&pool, tenant_id)
        .await
        .expect("begin bootstrap transaction");
    let root = scopes::ensure_tenant_root(&mut tx, tenant_id)
        .await
        .expect("mint tenant root");
    seed_identity(&mut tx, tenant_id, ADMIN).await;
    seed_identity(&mut tx, tenant_id, OUTSIDER).await;
    access::create_grant(
        &mut tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id,
            scope_id: root.id,
            subject: GrantSubject::Principal {
                principal_id: ADMIN.to_owned(),
            },
            role_key: RoleKey::Administrator,
            source: GrantSource::Automation,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .expect("seed administrator grant");
    configuration_support::bind_pack(&mut tx, tenant_id, root.id, synveda_policy::STANDARD).await;
    tx.commit().await.expect("commit bootstrap");
    pool.close().await;
    Some((state(&url), tenant_id, root.id))
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    key: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
    if let Some(key) = key {
        request = request.header("idempotency-key", key);
    }
    let request = match body {
        Some(body) => request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("build JSON request"),
        None => request.body(Body::empty()).expect("build request"),
    };
    let response = app.clone().oneshot(request).await.expect("router responds");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect response")
        .to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("JSON response")
    };
    (status, body)
}

/// One workspace project in which every batch gets its own session, so each
/// frozen snapshot holds exactly one event and one provider call.
struct Source {
    workspace_id: String,
    project_id: String,
}

async fn source(app: &Router, token: &str, slug: &str) -> Source {
    let (status, workspace) = call(
        app,
        "POST",
        "/v1/workspaces",
        token,
        Some(&format!("workspace-{slug}")),
        Some(json!({"slug": slug, "display_name": slug})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workspace}");
    let workspace_id = workspace["id"].as_str().expect("workspace id").to_owned();
    let (status, project) = call(
        app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/projects"),
        token,
        Some(&format!("project-{slug}")),
        Some(json!({"slug": slug, "display_name": slug})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    Source {
        workspace_id,
        project_id: project["id"].as_str().expect("project id").to_owned(),
    }
}

/// Opens a session, appends one event and freezes it into a new batch.
async fn freeze(app: &Router, token: &str, source: &Source, key: &str) -> CaptureBatchId {
    let (status, session) = call(
        app,
        "POST",
        "/v1/sessions",
        token,
        Some(&format!("session-{key}")),
        Some(json!({
            "workspace_id": source.workspace_id,
            "project_id": source.project_id,
            "client_name": "capture-retry-acceptance",
            "client_version": "1",
            "task_summary": "Extract durable project knowledge",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{session}");
    let session_id = session["id"].as_str().expect("session id");
    let (status, appended) = call(
        app,
        "POST",
        &format!("/v1/sessions/{session_id}/events"),
        token,
        None,
        Some(json!({"events": [{
            "event_type": "message.user",
            "client_event_id": key,
            "occurred_at": "2026-10-26T10:00:00Z",
            "payload": {"text": format!("Remember the retry convention for {key}.")},
        }]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{appended}");
    let (status, frozen) = call(
        app,
        "POST",
        &format!("/v1/sessions/{session_id}/capture-batches"),
        token,
        Some(key),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{frozen}");
    assert_eq!(frozen["event_count"], 1);
    frozen["id"]
        .as_str()
        .expect("batch id")
        .parse()
        .expect("parse batch id")
}

#[derive(Clone, Copy)]
enum Reply {
    Status(u16, Option<&'static str>),
    Invalid,
    Block,
    Candidate,
}

#[derive(Clone, Default)]
struct Script {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    calls: Arc<AtomicUsize>,
    in_flight: Arc<AtomicUsize>,
    max_in_flight: Arc<AtomicUsize>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

impl Script {
    fn push(&self, replies: impl IntoIterator<Item = Reply>) {
        self.replies.lock().expect("script lock").extend(replies);
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn max_in_flight(&self) -> usize {
        self.max_in_flight.load(Ordering::SeqCst)
    }
}

/// Decrements the in-flight gauge when a request finishes or its client
/// cancels it.
struct InFlight(Arc<AtomicUsize>);

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn candidate_response() -> Response {
    axum::Json(json!({
        "model": "retry-script@1",
        "choices": [{"message": {"content": json!({"candidates": [{
            "knowledge_type": "convention",
            "title": "Retry convention",
            "body_markdown": "Capture retries wait for their database deadline.",
            "summary": "Capture retries wait for their deadline.",
            "tags": ["retry"],
            "confidence": 0.9
        }]}).to_string()}}]
    }))
    .into_response()
}

async fn scripted(State(script): State<Script>) -> Response {
    script.calls.fetch_add(1, Ordering::SeqCst);
    let now = script.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
    script.max_in_flight.fetch_max(now, Ordering::SeqCst);
    let _in_flight = InFlight(Arc::clone(&script.in_flight));
    let reply = script
        .replies
        .lock()
        .expect("script lock")
        .pop_front()
        .unwrap_or(Reply::Candidate);
    match reply {
        Reply::Status(status, retry_after) => {
            let mut response = Response::new(Body::from(BODY_SENTINEL));
            *response.status_mut() = StatusCode::from_u16(status).expect("scripted status");
            if let Some(value) = retry_after {
                response.headers_mut().insert(
                    axum::http::header::RETRY_AFTER,
                    value.parse().expect("retry-after header"),
                );
            }
            response
        }
        Reply::Invalid => axum::Json(json!({
            "model": "retry-script@1",
            "choices": [{"message": {"content": BODY_SENTINEL}}]
        }))
        .into_response(),
        Reply::Block => {
            script.entered.notify_one();
            script.release.notified().await;
            candidate_response()
        }
        Reply::Candidate => candidate_response(),
    }
}

struct Provider {
    base_url: String,
    script: Script,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Provider {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn provider() -> Provider {
    let script = Script::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind scripted provider");
    let address = listener.local_addr().expect("provider address");
    let app = Router::new()
        .route("/v1/chat/completions", post(scripted))
        .with_state(script.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve scripted provider");
    });
    Provider {
        base_url: format!("http://{address}"),
        script,
        task,
    }
}

fn extractor(provider: &Provider) -> AnyExtractor {
    AnyExtractor::Vllm(
        VllmExtractor::new("retry-script@1".to_owned(), provider.base_url.clone())
            .expect("configure scripted extractor"),
    )
}

fn deps(state: &AppState, provider: &Provider) -> capture_worker::Deps {
    capture_worker::Deps {
        pool: state.pool.clone(),
        pdp: Arc::clone(&state.pdp),
        extractor: Arc::new(extractor(provider)),
    }
}

/// One worker process: a fresh lease owner, as after a restart.
fn worker(policy: RetryPolicy, provider_concurrency: u16) -> capture_worker::Config {
    capture_worker::Config {
        poll_interval: Duration::from_secs(1),
        lease_duration: Duration::from_secs(30),
        batches_per_tenant: 1,
        lease_owner: format!("capture-retry-{}", CaptureBatchId::new()),
        retry: policy,
        provider_concurrency,
    }
}

fn policy(base: u64, max_delay: u64, max_age: u64) -> RetryPolicy {
    RetryPolicy::new(
        Duration::from_secs(base),
        Duration::from_secs(max_delay),
        Duration::from_secs(max_age),
    )
    .expect("ordered retry policy")
}

async fn sweep(
    state: &AppState,
    provider: &Provider,
    config: &capture_worker::Config,
    tenant_id: TenantId,
) -> capture_worker::SweepSummary {
    capture_worker::sweep_tenant_once(&deps(state, provider), config, tenant_id)
        .await
        .expect("capture sweep")
}

async fn batch(state: &AppState, tenant_id: TenantId, id: CaptureBatchId) -> CaptureBatch {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin batch inspection");
    let batch = capture_store::get_batch(&mut *tx, tenant_id, id)
        .await
        .expect("read batch")
        .expect("batch is visible to its tenant");
    tx.rollback().await.expect("finish batch inspection");
    batch
}

async fn database_now(state: &AppState, tenant_id: TenantId) -> DateTime<Utc> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin clock read");
    let now = sqlx::query_scalar("select statement_timestamp()")
        .fetch_one(&mut *tx)
        .await
        .expect("read database time");
    tx.rollback().await.expect("finish clock read");
    now
}

/// The database-time witness of an elapsed backoff: the deadline moves to
/// just before the current statement time, exactly as waiting would leave it.
async fn deadline_passes(state: &AppState, tenant_id: TenantId, id: CaptureBatchId) {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin deadline witness");
    let changed = sqlx::query(
        "update capture_batches \
            set next_attempt_at = statement_timestamp() - interval '1 millisecond' \
          where tenant_id = $1 and id = $2 and state = 'pending' \
            and next_attempt_at > statement_timestamp()",
    )
    .bind(tenant_id.as_uuid())
    .bind(id.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("advance retry deadline");
    assert_eq!(changed.rows_affected(), 1, "one future deadline passes");
    tx.commit().await.expect("commit deadline witness");
}

async fn candidates(state: &AppState, tenant_id: TenantId, id: CaptureBatchId) -> i64 {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin candidate count");
    let count = sqlx::query_scalar(
        "select count(*) from capture_candidates where tenant_id = $1 and batch_id = $2",
    )
    .bind(tenant_id.as_uuid())
    .bind(id.as_uuid())
    .fetch_one(&mut *tx)
    .await
    .expect("count candidates");
    tx.rollback().await.expect("finish candidate count");
    count
}

async fn batch_audit(
    state: &AppState,
    tenant_id: TenantId,
    id: CaptureBatchId,
    action: &str,
) -> Vec<Value> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin audit read");
    let events = synveda_audit::tail(&mut tx, tenant_id, 1_000)
        .await
        .expect("read audit tail");
    tx.rollback().await.expect("finish audit read");
    events
        .into_iter()
        .filter(|event| {
            event.action.as_str() == action && event.payload["batch_id"] == id.to_string()
        })
        .map(|event| event.payload)
        .collect()
}

fn assert_content_free(payloads: &[Value]) {
    for payload in payloads {
        assert!(
            !payload.to_string().contains("SYNVEDA_SECRET_SENTINEL"),
            "provider content reached audit: {payload}"
        );
    }
}

#[tokio::test]
async fn retries_wait_for_their_database_deadline_across_workers_restart_and_clock_skew() {
    let _guard = serial().await;
    let Some((state, tenant_id, _)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let source = source(&app, &token, "retry-deadline").await;
    let batch_id = freeze(&app, &token, &source, "retry-deadline-one").await;
    let provider = provider().await;
    provider.script.push([Reply::Status(503, None)]);
    let schedule = policy(60, 600, 3_600);

    let first = sweep(&state, &provider, &worker(schedule, 3), tenant_id).await;
    assert_eq!(first.failed_attempts, 1, "{first:?}");
    let scheduled = batch(&state, tenant_id, batch_id).await;
    assert_eq!(scheduled.state, CaptureBatchState::Pending);
    assert_eq!(scheduled.attempts, 1);
    assert_eq!(
        scheduled.error_code.as_deref(),
        Some("provider_unavailable")
    );
    assert_eq!(scheduled.terminal_reason, None);
    let deadline = scheduled.next_attempt_at.expect("persisted retry deadline");
    let expected = schedule.delay(batch_id, 1, None);
    let stored = (deadline - scheduled.updated_at)
        .to_std()
        .expect("deadline follows the failure");
    assert!(
        stored >= expected.saturating_sub(Duration::from_secs(1))
            && stored <= expected + Duration::from_secs(1),
        "stored delay {stored:?} is the deterministic {expected:?}"
    );
    assert!(deadline > database_now(&state, tenant_id).await);

    // A peer worker and a restarted process each have their own lease owner;
    // neither can claim before the stored deadline.
    for config in [worker(schedule, 3), worker(policy(1, 1, 60), 3)] {
        let early = sweep(&state, &provider, &config, tenant_id).await;
        assert_eq!(
            (early.completed, early.failed_attempts),
            (0, 0),
            "{early:?}"
        );
    }
    // Eligibility is database time only: a worker clock an hour ahead would
    // think the deadline has passed, but no claim reads a worker clock.
    let skewed_worker_clock = Utc::now() + chrono::Duration::hours(1);
    assert!(skewed_worker_clock > deadline);
    let skewed = sweep(&state, &provider, &worker(schedule, 3), tenant_id).await;
    assert_eq!(skewed.completed, 0);
    assert_eq!(provider.script.calls(), 1, "no early provider call");
    assert_eq!(batch(&state, tenant_id, batch_id).await.attempts, 1);

    deadline_passes(&state, tenant_id, batch_id).await;
    let (left_worker, right_worker) = (worker(schedule, 3), worker(schedule, 3));
    let (left, right) = tokio::join!(
        sweep(&state, &provider, &left_worker, tenant_id),
        sweep(&state, &provider, &right_worker, tenant_id),
    );
    assert_eq!(left.completed + right.completed, 1, "{left:?} {right:?}");
    assert_eq!(provider.script.calls(), 2);
    let completed = batch(&state, tenant_id, batch_id).await;
    assert_eq!(completed.state, CaptureBatchState::Completed);
    assert_eq!((completed.attempts, completed.next_attempt_at), (2, None));
    assert_eq!(candidates(&state, tenant_id, batch_id).await, 1);

    let failures = batch_audit(&state, tenant_id, batch_id, "capture.batch.completed").await;
    assert_eq!(failures.len(), 2, "one retry and one completion");
    let retry = failures
        .iter()
        .find(|payload| payload["state"] == "pending")
        .expect("scheduled retry evidence");
    assert_eq!(retry["error_code"], "provider_unavailable");
    assert_eq!(retry["retryable"], true);
    assert!(retry["next_attempt_at"].is_string());
    assert_content_free(&failures);
}

#[tokio::test]
async fn permanent_failures_close_after_one_attempt_with_inspectable_codes() {
    let _guard = serial().await;
    let Some((state, tenant_id, _)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let source = source(&app, &token, "retry-permanent").await;
    let mut batches = Vec::new();
    for key in ["permanent-auth", "permanent-request", "permanent-output"] {
        batches.push(freeze(&app, &token, &source, key).await);
    }
    let provider = provider().await;
    provider.script.push([
        Reply::Status(401, Some("1")),
        Reply::Status(400, None),
        Reply::Invalid,
    ]);
    for _ in &batches {
        let summary = sweep(&state, &provider, &worker(policy(1, 10, 60), 3), tenant_id).await;
        assert_eq!(summary.failed_attempts, 1, "{summary:?}");
    }
    assert_eq!(provider.script.calls(), 3, "one call per batch");
    for (id, code) in batches.iter().zip([
        "provider_unauthorized",
        "provider_rejected",
        "provider_response_invalid",
    ]) {
        let closed = batch(&state, tenant_id, *id).await;
        assert_eq!(closed.state, CaptureBatchState::Failed);
        assert_eq!(closed.attempts, 1);
        assert_eq!(closed.error_code.as_deref(), Some(code));
        assert_eq!(
            closed.terminal_reason,
            Some(CaptureTerminalReason::Permanent)
        );
        assert_eq!(closed.next_attempt_at, None);
        assert_eq!(candidates(&state, tenant_id, *id).await, 0);

        let (status, view) = call(
            &app,
            "GET",
            &format!("/v1/capture-batches/{id}"),
            &token,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{view}");
        assert_eq!(view["state"], "failed");
        assert_eq!(view["error_code"], code);
        assert_eq!(view["terminal_reason"], "permanent");
        assert_eq!(view["max_attempts"], 5);
        assert!(view.get("next_attempt_at").is_none());
        assert!(!view.to_string().contains("SYNVEDA_SECRET_SENTINEL"));

        let audit = batch_audit(&state, tenant_id, *id, "capture.batch.completed").await;
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0]["retryable"], false);
        assert_eq!(audit[0]["terminal_reason"], "permanent");
        assert_content_free(&audit);
    }
}

#[tokio::test]
async fn retry_after_is_capped_and_retry_age_closes_the_cycle() {
    let _guard = serial().await;
    let Some((state, tenant_id, _)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let source = source(&app, &token, "retry-bounds").await;
    let capped = freeze(&app, &token, &source, "retry-bounds-capped").await;
    let aged = freeze(&app, &token, &source, "retry-bounds-aged").await;
    let provider = provider().await;
    provider.script.push([
        Reply::Status(429, Some("999999999")),
        Reply::Status(503, Some("30")),
    ]);

    let summary = sweep(
        &state,
        &provider,
        &worker(policy(1, 10, 3_600), 3),
        tenant_id,
    )
    .await;
    assert_eq!(summary.failed_attempts, 1);
    let limited = batch(&state, tenant_id, capped).await;
    assert_eq!(limited.state, CaptureBatchState::Pending);
    assert_eq!(limited.error_code.as_deref(), Some("provider_rate_limited"));
    let delay = (limited.next_attempt_at.expect("deadline") - limited.updated_at)
        .to_std()
        .expect("future deadline");
    assert!(
        delay >= Duration::from_secs(9) && delay <= Duration::from_secs(11),
        "an untrusted hint is capped at the ten-second maximum delay, got {delay:?}"
    );

    // A cycle whose next eligibility would exceed its age bound closes
    // instead of scheduling another provider call.
    let summary = sweep(&state, &provider, &worker(policy(1, 10, 10), 3), tenant_id).await;
    assert_eq!(summary.failed_attempts, 1);
    let closed = batch(&state, tenant_id, aged).await;
    assert_eq!(closed.state, CaptureBatchState::Failed);
    assert_eq!(closed.attempts, 1);
    assert_eq!(closed.error_code.as_deref(), Some("provider_unavailable"));
    assert_eq!(
        closed.terminal_reason,
        Some(CaptureTerminalReason::RetryAgeExhausted)
    );
    assert_eq!(provider.script.calls(), 2);
}

#[tokio::test]
async fn exhausted_cycles_close_and_a_governed_manual_retry_reopens_them_once() {
    let _guard = serial().await;
    let Some((state, tenant_id, root)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let source = source(&app, &token, "retry-manual").await;
    let exhausted = freeze(&app, &token, &source, "retry-manual-exhausted").await;
    let provider = provider().await;
    let schedule = policy(1, 10, 3_600);
    provider.script.push([Reply::Status(503, None); 5]);
    for attempt in 1..=5 {
        let summary = sweep(&state, &provider, &worker(schedule, 3), tenant_id).await;
        assert_eq!(summary.failed_attempts, 1, "{summary:?}");
        if attempt < 5 {
            deadline_passes(&state, tenant_id, exhausted).await;
        }
    }
    let closed = batch(&state, tenant_id, exhausted).await;
    assert_eq!(closed.state, CaptureBatchState::Failed);
    assert_eq!(closed.attempts, 5);
    assert_eq!(
        closed.terminal_reason,
        Some(CaptureTerminalReason::AttemptsExhausted)
    );
    assert_eq!(provider.script.calls(), 5);
    let failures = batch_audit(&state, tenant_id, exhausted, "capture.batch.completed").await;
    assert_eq!(failures.len(), 5);

    // A raw runtime-role edit cannot reset the fence or reopen the batch.
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin fence edit");
    let reset =
        sqlx::query("update capture_batches set attempts = 0 where tenant_id = $1 and id = $2")
            .bind(tenant_id.as_uuid())
            .bind(exhausted.as_uuid())
            .execute(&mut *tx)
            .await
            .expect_err("the attempt fence is monotonic");
    assert!(reset.to_string().contains("monotonic fence"), "{reset}");
    tx.rollback().await.expect("roll back refused edit");
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin reopen edit");
    let reopen = sqlx::query(
        "update capture_batches set state = 'pending', completed_at = null, terminal_reason = null \
          where tenant_id = $1 and id = $2",
    )
    .bind(tenant_id.as_uuid())
    .bind(exhausted.as_uuid())
    .execute(&mut *tx)
    .await
    .expect_err("only a bounded manual retry reopens a failed batch");
    assert!(
        reopen.to_string().contains("bounded manual retry"),
        "{reopen}"
    );
    tx.rollback().await.expect("roll back refused reopen");

    let path = format!("/v1/capture-batches/{exhausted}/retry");
    let (status, _) = call(&app, "POST", &path, &token, None, None).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an idempotency key is required"
    );
    let (status, body) = call(
        &app,
        "POST",
        &path,
        &issue(OUTSIDER, tenant_id),
        Some("outsider-retry"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let Some((foreign_state, foreign_tenant, _)) = admitted_tenant().await else {
        return;
    };
    let (status, _) = call(
        &router(foreign_state),
        "POST",
        &path,
        &issue(ADMIN, foreign_tenant),
        Some("foreign-retry"),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a foreign batch does not exist"
    );
    assert_eq!(batch(&state, tenant_id, exhausted).await.manual_retries, 0);

    let (status, reopened) = call(&app, "POST", &path, &token, Some("retry-one"), None).await;
    assert_eq!(status, StatusCode::OK, "{reopened}");
    assert_eq!(reopened["state"], "pending");
    assert_eq!(reopened["attempts"], 5, "the fence is not reset");
    assert_eq!(reopened["manual_retries"], 1);
    assert_eq!(reopened["max_attempts"], 10);
    assert!(reopened.get("terminal_reason").is_none());
    assert_eq!(reopened["error_code"], "provider_unavailable");
    let (status, replay) = call(&app, "POST", &path, &token, Some("retry-one"), None).await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(
        replay["manual_retries"], 1,
        "a replayed key reopens nothing"
    );
    let (status, refused) = call(&app, "POST", &path, &token, Some("retry-two"), None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    let requested = batch_audit(
        &state,
        tenant_id,
        exhausted,
        "capture.batch.retry_requested",
    )
    .await;
    assert_eq!(requested.len(), 1);
    assert_eq!(requested[0]["prior_error_code"], "provider_unavailable");
    assert_eq!(requested[0]["prior_terminal_reason"], "attempts_exhausted");
    assert_eq!(requested[0]["attempts"], 5);
    assert_eq!(requested[0]["authz"]["action"], "session.write");
    assert!(requested[0]["authz"]["pack"].is_string());

    let summary = sweep(&state, &provider, &worker(schedule, 3), tenant_id).await;
    assert_eq!(summary.completed, 1, "{summary:?}");
    let recovered = batch(&state, tenant_id, exhausted).await;
    assert_eq!(recovered.state, CaptureBatchState::Completed);
    assert_eq!(recovered.attempts, 6);
    assert_eq!(candidates(&state, tenant_id, exhausted).await, 1);
    assert_eq!(
        batch_audit(&state, tenant_id, exhausted, "capture.batch.completed")
            .await
            .len(),
        6
    );
    let (status, _) = call(&app, "POST", &path, &token, Some("retry-three"), None).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a completed batch is not retried"
    );

    // Manual cycles are bounded: two reopenings, then refusal.
    let bounded = freeze(&app, &token, &source, "retry-manual-bounded").await;
    let bounded_path = format!("/v1/capture-batches/{bounded}/retry");
    for cycle in 0..=2 {
        provider.script.push([Reply::Status(401, None)]);
        let summary = sweep(&state, &provider, &worker(schedule, 3), tenant_id).await;
        assert_eq!(summary.failed_attempts, 1, "{summary:?}");
        let (status, body) = call(
            &app,
            "POST",
            &bounded_path,
            &token,
            Some(&format!("bounded-{cycle}")),
            None,
        )
        .await;
        if cycle < 2 {
            assert_eq!(status, StatusCode::OK, "{body}");
        } else {
            assert_eq!(status, StatusCode::CONFLICT, "{body}");
        }
    }
    let spent = batch(&state, tenant_id, bounded).await;
    assert_eq!((spent.attempts, spent.manual_retries), (3, 2));

    // A Configuration change makes the frozen batch stale: capture again
    // under the current document instead.
    let stale = freeze(&app, &token, &source, "retry-manual-stale").await;
    provider.script.push([Reply::Status(401, None)]);
    sweep(&state, &provider, &worker(schedule, 3), tenant_id).await;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin Configuration change");
    configuration_support::set_trace_retention(
        &mut tx,
        tenant_id,
        root,
        synveda_types::TraceRetentionMode::Redacted,
    )
    .await;
    tx.commit().await.expect("commit Configuration change");
    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/capture-batches/{stale}/retry"),
        &token,
        Some("stale-retry"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(batch(&state, tenant_id, stale).await.manual_retries, 0);
}

async fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !ready() {
            tokio::task::yield_now().await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{what}"));
}

async fn held_slots(state: &AppState, tenant_id: TenantId, key: [u8; 32]) -> i64 {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin slot inspection");
    let held = sqlx::query_scalar(
        "select count(*) from capture_provider_slots \
          where provider_key = $1 and expires_at > statement_timestamp()",
    )
    .bind(&key[..])
    .fetch_one(&mut *tx)
    .await
    .expect("count held slots");
    tx.rollback().await.expect("finish slot inspection");
    held
}

/// The crash witness: both the orphaned claim and its slot reach their
/// database-time expiry, as they would once the lease duration elapsed.
async fn holder_expires(state: &AppState, tenant_id: TenantId, id: CaptureBatchId, key: [u8; 32]) {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin expiry witness");
    let claim = sqlx::query(
        "update capture_batches set lease_expires_at = statement_timestamp() \
          where tenant_id = $1 and id = $2 and state = 'running'",
    )
    .bind(tenant_id.as_uuid())
    .bind(id.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("expire orphaned claim");
    assert_eq!(claim.rows_affected(), 1);
    let slot = sqlx::query(
        "update capture_provider_slots set expires_at = statement_timestamp() \
          where provider_key = $1 and expires_at > statement_timestamp()",
    )
    .bind(&key[..])
    .execute(&mut *tx)
    .await
    .expect("expire orphaned slot");
    assert_eq!(slot.rows_affected(), 1);
    tx.commit().await.expect("commit expiry witness");
}

#[tokio::test]
async fn provider_slots_bound_every_worker_and_recover_from_a_lost_holder() {
    let _guard = serial().await;
    let Some((state, tenant_id, _)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let source = source(&app, &token, "retry-slots").await;
    let held = freeze(&app, &token, &source, "retry-slots-held").await;
    let waiting = freeze(&app, &token, &source, "retry-slots-waiting").await;
    let provider = provider().await;
    let key = extractor(&provider)
        .provider_key()
        .expect("provider identity");
    let schedule = policy(1, 10, 3_600);
    provider.script.push([Reply::Block]);

    let holder = {
        let deps = deps(&state, &provider);
        let config = worker(schedule, 1);
        tokio::spawn(
            async move { capture_worker::sweep_tenant_once(&deps, &config, tenant_id).await },
        )
    };
    tokio::time::timeout(Duration::from_secs(10), provider.script.entered.notified())
        .await
        .expect("the first worker reaches the provider");
    assert_eq!(held_slots(&state, tenant_id, key).await, 1);

    let refused = sweep(&state, &provider, &worker(schedule, 1), tenant_id).await;
    assert_eq!(refused.saturated, 1, "{refused:?}");
    let untouched = batch(&state, tenant_id, waiting).await;
    assert_eq!(
        (untouched.state, untouched.attempts),
        (CaptureBatchState::Pending, 0),
        "a saturated claim rolls back without spending an attempt"
    );
    assert_eq!(provider.script.calls(), 1);

    provider.script.release.notify_one();
    let summary = holder.await.expect("holder joins").expect("holder sweep");
    assert_eq!(summary.completed, 1, "{summary:?}");
    assert_eq!(
        held_slots(&state, tenant_id, key).await,
        0,
        "completion returns the slot"
    );
    let next = sweep(&state, &provider, &worker(schedule, 1), tenant_id).await;
    assert_eq!(next.completed, 1, "{next:?}");

    // Process loss: the holder disappears mid-call. Its claim and slot stay
    // held until their database-time expiry; nothing else may call meanwhile.
    let orphaned = freeze(&app, &token, &source, "retry-slots-orphaned").await;
    let queued = freeze(&app, &token, &source, "retry-slots-queued").await;
    provider.script.push([Reply::Block]);
    let lost = {
        let deps = deps(&state, &provider);
        let config = worker(schedule, 1);
        tokio::spawn(
            async move { capture_worker::sweep_tenant_once(&deps, &config, tenant_id).await },
        )
    };
    tokio::time::timeout(Duration::from_secs(10), provider.script.entered.notified())
        .await
        .expect("the lost worker reaches the provider");
    lost.abort();
    let _ = lost.await;
    let in_flight = Arc::clone(&provider.script.in_flight);
    wait_until("the provider observes the cancelled call", || {
        in_flight.load(Ordering::SeqCst) == 0
    })
    .await;
    let blocked = sweep(&state, &provider, &worker(schedule, 1), tenant_id).await;
    assert_eq!(blocked.saturated, 1, "{blocked:?}");
    assert_eq!(batch(&state, tenant_id, queued).await.attempts, 0);
    assert_eq!(batch(&state, tenant_id, orphaned).await.attempts, 1);

    holder_expires(&state, tenant_id, orphaned, key).await;
    let recovered = sweep(&state, &provider, &worker(schedule, 1), tenant_id).await;
    assert_eq!(recovered.completed, 1, "{recovered:?}");
    let reclaimed = batch(&state, tenant_id, orphaned).await;
    assert_eq!(
        (reclaimed.state, reclaimed.attempts),
        (CaptureBatchState::Completed, 2)
    );
    let drained = sweep(&state, &provider, &worker(schedule, 1), tenant_id).await;
    assert_eq!(drained.completed, 1, "{drained:?}");

    for id in [held, waiting, orphaned, queued] {
        assert_eq!(
            candidates(&state, tenant_id, id).await,
            1,
            "one fenced candidate set"
        );
    }
    assert_eq!(
        provider.script.calls(),
        5,
        "four completions and one cancelled call"
    );
    assert_eq!(
        provider.script.max_in_flight(),
        1,
        "the ceiling held across workers"
    );
    assert_eq!(held_slots(&state, tenant_id, key).await, 0);
}

/// Per-batch fault script for the multi-process drill. A request is matched
/// by the marker in its frozen event text, so work belonging to other fixture
/// tenants gets an empty answer and cannot consume a scripted fault.
#[derive(Clone, Default)]
struct Drill {
    calls: Arc<Mutex<std::collections::BTreeMap<String, usize>>>,
    in_flight: Arc<AtomicUsize>,
    max_in_flight: Arc<AtomicUsize>,
    released: Arc<std::sync::atomic::AtomicBool>,
    release: Arc<Notify>,
}

impl Drill {
    fn calls(&self, marker: &str) -> usize {
        self.calls
            .lock()
            .expect("drill lock")
            .get(marker)
            .copied()
            .unwrap_or(0)
    }
}

fn drill_marker(request: &Value) -> Option<String> {
    request["messages"]
        .as_array()?
        .iter()
        .filter(|message| message["role"] == "user")
        .filter_map(|message| message["content"].as_str())
        .filter_map(|content| serde_json::from_str::<Value>(content).ok())
        .filter_map(|input| input["payload"]["text"].as_str().map(str::to_owned))
        .find_map(|text| {
            text.split_whitespace()
                .find(|word| word.starts_with("drill-"))
                .map(|word| word.trim_end_matches('.').to_owned())
        })
}

async fn drilled(State(drill): State<Drill>, axum::Json(request): axum::Json<Value>) -> Response {
    let Some(marker) = drill_marker(&request) else {
        return axum::Json(json!({
            "model": "retry-script@1",
            "choices": [{"message": {"content": "{\"candidates\":[]}"}}]
        }))
        .into_response();
    };
    let call = {
        let mut calls = drill.calls.lock().expect("drill lock");
        let entry = calls.entry(marker.clone()).or_insert(0);
        *entry += 1;
        *entry
    };
    let now = drill.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
    drill.max_in_flight.fetch_max(now, Ordering::SeqCst);
    let _in_flight = InFlight(Arc::clone(&drill.in_flight));
    let fault = |status: u16, retry_after: &'static str| {
        let mut response = Response::new(Body::from(BODY_SENTINEL));
        *response.status_mut() = StatusCode::from_u16(status).expect("drill status");
        response.headers_mut().insert(
            axum::http::header::RETRY_AFTER,
            retry_after.parse().expect("retry-after header"),
        );
        response
    };
    match (marker.split('-').nth(1), call) {
        (Some("unavailable"), 1) => fault(503, "1"),
        (Some("limited"), 1) => fault(429, "1"),
        (Some("timeout"), 1) => std::future::pending::<Response>().await,
        (Some("block"), _) => {
            while !drill.released.load(Ordering::SeqCst) {
                drill.release.notified().await;
            }
            candidate_response()
        }
        _ => candidate_response(),
    }
}

async fn drill_provider() -> (String, Drill, tokio::task::JoinHandle<()>) {
    let drill = Drill::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind drill provider");
    let address = listener.local_addr().expect("drill address");
    let app = Router::new()
        .route("/v1/chat/completions", post(drilled))
        .with_state(drill.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve drill provider");
    });
    (format!("http://{address}"), drill, task)
}

struct WorkerProcess {
    child: std::process::Child,
    addr: String,
    profile: &'static str,
}

impl Drop for WorkerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn loopback_addr() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve loopback port");
    let addr = listener.local_addr().expect("loopback port").to_string();
    drop(listener);
    addr
}

fn spawn_capture_worker(
    worker_url_file: &str,
    roles: &str,
    provider: &str,
    profile: &'static str,
) -> WorkerProcess {
    let addr = loopback_addr();
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_synveda-worker"))
        .env_remove("DATABASE_URL")
        .env("DATABASE_URL_FILE", worker_url_file)
        .env("SYNVEDA_DATABASE_ROLES", roles)
        .env_remove("SYNVEDA_DATABASE_ROLES_FILE")
        .env_remove("SYNVEDA_EXPECTED_DATABASE_ROLE")
        .env_remove("SYNVEDA_EXPECTED_DATABASE_ROLE_FILE")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("ANTHROPIC_API_KEY_FILE")
        .env_remove("SYNVEDA_OIDC_ISSUERS")
        .env_remove("SYNVEDA_OIDC_ISSUERS_FILE")
        .env_remove("SYNVEDA_KMS_KEY")
        .env_remove("SYNVEDA_KMS_KEY_FILE")
        .env_remove("SYNVEDA_KMS_KEY_REF")
        .env_remove("SYNVEDA_KMS_KEY_REF_FILE")
        .env("SYNVEDA_WORKER_LISTEN_ADDR", &addr)
        .env("SYNVEDA_WORKER_PROFILE", profile)
        .env("SYNVEDA_WORKER_SHUTDOWN_SECS", "3")
        .env("SYNVEDA_EMBEDDER", "deterministic")
        .env("SYNVEDA_EXTRACTOR", "vllm")
        .env("SYNVEDA_EXTRACTOR_MODEL", "retry-script@1")
        .env("SYNVEDA_VLLM_BASE_URL", provider)
        // A blocked survivor must still hold its call when the lost owner's
        // slot is reclaimed. That call spans saturation, owner loss, lease
        // expiry and refill, which took up to 10 s on hosted runners.
        .env("SYNVEDA_EXTRACTOR_TIMEOUT_SECS", "30")
        .env("SYNVEDA_CAPTURE_POLL_MS", "100")
        .env("SYNVEDA_CAPTURE_LEASE_SECS", "4")
        .env("SYNVEDA_CAPTURE_RETRY_BASE_SECS", "1")
        .env("SYNVEDA_CAPTURE_RETRY_MAX_DELAY_SECS", "2")
        .env("SYNVEDA_CAPTURE_RETRY_MAX_AGE_SECS", "300")
        .env("SYNVEDA_CAPTURE_PROVIDER_CONCURRENCY", "2")
        .env("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:9")
        .env("OTEL_BSP_EXPORT_TIMEOUT", "100")
        .env("RUST_LOG", "error")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn synveda-worker");
    WorkerProcess {
        child,
        addr,
        profile,
    }
}

async fn worker_get(addr: &str, path: &str) -> Option<(u16, String)> {
    let response = reqwest::Client::new()
        .get(format!("http://{addr}{path}"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .ok()?;
    let status = response.status().as_u16();
    Some((status, response.text().await.ok()?))
}

fn metric(text: &str, name: &str) -> f64 {
    text.lines()
        .filter(|line| line.starts_with(name))
        .filter_map(|line| line.rsplit(' ').next()?.parse::<f64>().ok())
        .sum()
}

async fn eventually<F, Fut>(what: &str, budget: Duration, mut ready: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    tokio::time::timeout(budget, async {
        while !ready().await {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{what}"));
}

/// Local multi-process qualification: one combined and two Capture-only
/// worker processes share a provider ceiling of two through blocked calls,
/// owner-process loss, lease expiry, 429, 503 and a client timeout, and each
/// batch keeps one durable candidate set despite repeated provider calls.
#[tokio::test]
async fn three_worker_processes_hold_the_provider_ceiling_through_faults_and_owner_loss() {
    let _guard = serial().await;
    let (Ok(worker_url_file), Ok(roles_file)) = (
        std::env::var("SYNVEDA_TEST_WORKER_DATABASE_URL_FILE"),
        std::env::var("SYNVEDA_DATABASE_ROLES_FILE"),
    ) else {
        eprintln!("isolated worker login not set; skipping the multi-process Capture drill");
        return;
    };
    let roles = std::fs::read_to_string(roles_file)
        .expect("read role contract")
        .trim_end()
        .to_owned();
    let Some((state, tenant_id, _)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let source = source(&app, &token, "retry-drill").await;
    let markers = [
        "drill-block-1",
        "drill-block-2",
        "drill-block-3",
        "drill-unavailable-1",
        "drill-limited-1",
        "drill-timeout-1",
    ];
    let mut batches = Vec::new();
    for marker in markers {
        batches.push((marker, freeze(&app, &token, &source, marker).await));
    }
    let (provider, drill, _server) = drill_provider().await;
    let mut workers = vec![
        spawn_capture_worker(&worker_url_file, &roles, &provider, "combined"),
        spawn_capture_worker(&worker_url_file, &roles, &provider, "capture-only"),
        spawn_capture_worker(&worker_url_file, &roles, &provider, "capture-only"),
    ];
    let started = std::time::Instant::now();
    for process in &workers {
        let addr = process.addr.clone();
        eventually("worker readiness", Duration::from_secs(30), || {
            let addr = addr.clone();
            async move {
                worker_get(&addr, "/readyz")
                    .await
                    .is_some_and(|(status, _)| status == 200)
            }
        })
        .await;
    }

    // Two blocked calls hold both slots; the third process is refused.
    let saturated = drill.clone();
    eventually("two calls hold both slots", Duration::from_secs(30), || {
        let drill = saturated.clone();
        async move { drill.in_flight.load(Ordering::SeqCst) == 2 }
    })
    .await;
    let addrs: Vec<String> = workers.iter().map(|process| process.addr.clone()).collect();
    eventually(
        "a worker records provider saturation",
        Duration::from_secs(30),
        || {
            let addrs = addrs.clone();
            async move {
                let mut refused = 0.0;
                for addr in &addrs {
                    if let Some((_, text)) = worker_get(addr, "/metrics").await {
                        refused += metric(
                            &text,
                            "synveda_capture_batches_total{outcome=\"provider_saturated\"}",
                        );
                    }
                }
                refused >= 1.0
            }
        },
    )
    .await;
    assert_eq!(drill.max_in_flight.load(Ordering::SeqCst), 2);

    // Owner loss: kill one slot holder outright. Its call is cancelled and
    // its claim and slot wait for their database-time expiry.
    let mut holder = None;
    for (index, process) in workers.iter().enumerate() {
        if let Some((_, text)) = worker_get(&process.addr, "/metrics").await
            && metric(
                &text,
                "synveda_capture_provider_slots_held{method=\"vllm\"}",
            ) >= 1.0
        {
            holder = Some(index);
            break;
        }
    }
    let lost = workers.remove(holder.expect("one process holds a provider slot"));
    let lost_profile = lost.profile;
    drop(lost);
    let lost_at = std::time::Instant::now();
    let cancelled = drill.clone();
    eventually(
        "the lost owner's call is cancelled",
        Duration::from_secs(10),
        || {
            let drill = cancelled.clone();
            async move { drill.in_flight.load(Ordering::SeqCst) <= 1 }
        },
    )
    .await;
    // The lease expires within four seconds; a survivor then takes the
    // freed slot without the ceiling ever exceeding two.
    let refilled = drill.clone();
    eventually(
        "a survivor reclaims the expired slot",
        Duration::from_secs(20),
        || {
            let drill = refilled.clone();
            async move { drill.in_flight.load(Ordering::SeqCst) == 2 }
        },
    )
    .await;
    let slot_recovery = lost_at.elapsed();
    drill.released.store(true, Ordering::SeqCst);
    drill.release.notify_waiters();

    let watched = batches.clone();
    let watcher = state.clone();
    eventually(
        "every drill batch completes",
        Duration::from_secs(120),
        || {
            let batches = watched.clone();
            let state = watcher.clone();
            async move {
                for (_, id) in &batches {
                    if batch(&state, tenant_id, *id).await.state != CaptureBatchState::Completed {
                        return false;
                    }
                }
                true
            }
        },
    )
    .await;
    let drained = started.elapsed();

    let mut evidence = Vec::new();
    for (marker, id) in &batches {
        let completed = batch(&state, tenant_id, *id).await;
        assert_eq!(
            candidates(&state, tenant_id, *id).await,
            1,
            "{marker}: one candidate set"
        );
        let calls = drill.calls(marker);
        assert_eq!(
            usize::try_from(completed.attempts).expect("positive attempts"),
            calls,
            "{marker}: one provider call per committed attempt"
        );
        assert!(
            calls <= 2,
            "{marker}: at most one repeated provider call, got {calls}"
        );
        let recovery = completed
            .completed_at
            .zip(completed.started_at)
            .map(|(end, start)| (end - start).num_milliseconds());
        evidence.push(json!({
            "marker": marker,
            "attempts": completed.attempts,
            "provider_calls": calls,
            "first_claim_to_completion_ms": recovery,
            "queued_ms": completed.started_at.map(|start| (start - completed.created_at).num_milliseconds()),
        }));
    }
    for marker in ["drill-unavailable-1", "drill-limited-1", "drill-timeout-1"] {
        assert_eq!(drill.calls(marker), 2, "{marker} retried exactly once");
    }
    let repeated_blocks = markers
        .iter()
        .filter(|marker| marker.starts_with("drill-block") && drill.calls(marker) == 2)
        .count();
    assert_eq!(
        repeated_blocks,
        1,
        "only the lost owner's blocked batch is called again: {}",
        json!({
            "lost_owner_to_slot_refill_ms": slot_recovery.as_millis() as u64,
            "batches": evidence,
        })
    );
    assert!(
        drill.max_in_flight.load(Ordering::SeqCst) <= 2,
        "the ceiling held"
    );

    let mut waits = Vec::new();
    for process in &workers {
        let (_, text) = worker_get(&process.addr, "/metrics")
            .await
            .expect("survivor metrics");
        // Workers sweep every fixture tenant, so only these drill-wide
        // refusal counters are reported; per-batch timings come from rows.
        waits.push(json!({
            "profile": process.profile,
            "provider_saturated": metric(&text, "synveda_capture_batches_total{outcome=\"provider_saturated\"}"),
            "slot_limit": metric(&text, "synveda_capture_provider_slot_limit{method=\"vllm\"}"),
        }));
    }
    for process in &mut workers {
        let _ = std::process::Command::new("kill")
            .arg("-TERM")
            .arg(process.child.id().to_string())
            .status();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = process.child.try_wait().expect("poll worker") {
                assert!(status.success(), "{} drained cleanly", process.profile);
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "worker exits inside its bound"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    println!(
        "{}",
        json!({
            "drill": "capture-three-process-backpressure",
            "provider_concurrency": 2,
            "max_in_flight": drill.max_in_flight.load(Ordering::SeqCst),
            "lost_profile": lost_profile,
            "lost_owner_to_slot_refill_ms": slot_recovery.as_millis() as u64,
            "first_ready_to_all_completed_ms": drained.as_millis() as u64,
            "batches": evidence,
            "survivor_metrics": waits,
        })
    );
}
