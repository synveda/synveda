//! The `/v1` client (EVAL-1, ADR-0028 decision 1).
//!
//! An actor's own bearer and no other way in. The wire structs are declared
//! here rather than imported because this crate depends on no Synveda crate
//! at all — the same price the TypeScript adapter pays, for the same reason:
//! what an outside caller can see is exactly what an eval should measure.
//!
//! **Anchored on the session plane by CPR-12** (ADR-0078 decisions 1 and 5).
//! Events are appended to a run this harness opened, capture candidates are
//! reviewed through the public application API, and context is composed by
//! that run's `context-runs` endpoint. Every fixture session label is looked
//! up through [`Client::session_for`], which opens one run per label and
//! reuses it.
//!
//! CPR-20 re-cuts deep query onto the ordinary session-scoped Knowledge lens
//! and corpus enumeration/id probes onto its stricter `SessionDiagnostics`
//! lens. Neither abuses a budgeted context run and neither restores a global
//! recall route. Prompt 30 re-measures the suites against accepted Knowledge.
//!
//! The audit search is untouched (ADR-0046 decision 4): the sweep says what
//! a *reader is served*, `GET /v1/audit/events` says what the *pipeline
//! committed*, and only the second of those two lenses still has a route.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::scenario::Environment;

/// Generous next to the ContextRun latency objective: a deadline here is
/// meant to end a hung run, not to be the thing under measurement. The latency axis
/// measures what the call took, not what it was allowed to take.
const TIMEOUT: Duration = Duration::from_secs(30);

pub struct Client {
    gateway_url: String,
    http: reqwest::Client,
    session_selections: BTreeMap<String, SessionSelection>,
}

#[derive(Clone)]
struct SessionSelection {
    workspace_id: String,
    project_id: Option<String>,
}

/// A random discriminator for an idempotency key.
fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{nanos:x}-{:x}", std::process::id())
}

/// `POST /v1/sessions/{id}/context-runs` (CTX-3, ADR-0026; re-anchored on the
/// session plane by CPR-12, ADR-0078 decision 5).
///
/// `session_id` is no longer on the wire: the run is in the path, and its id is
/// a real aggregate the harness opened rather than a label a fixture chose.
#[derive(Debug, Serialize)]
pub struct ContextRunRequest<'a> {
    #[serde(rename = "query", skip_serializing_if = "Option::is_none")]
    pub task: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<u32>,
}

/// The composed block. A ContextRun renders selected immutable Knowledge
/// revisions. Addresses are recovered from the block's current watermark;
/// candidate scores and exclusion details are read from the ContextRun trace,
/// never reconstructed from obsolete progressive-rendering telemetry.
#[derive(Debug, Deserialize)]
pub struct ContextRunResponse {
    #[serde(rename = "rendered")]
    pub text: String,
    /// The watermark (ADR-0025 decision 7). It rides into the report so a
    /// measurement can be traced back to exactly the block that produced
    /// it, months later, from the audit chain.
    pub block_hash: String,
    /// Parsed from the watermark rather than served as a field; see
    /// [`ContextRunResponse::knowledge_item_ids`].
    #[serde(default, skip)]
    pub knowledge_item_ids: Vec<String>,
    pub tokens: u32,
    pub budget_tokens: u32,
}

impl ContextRunResponse {
    /// Fills `knowledge_item_ids` from the block's watermark line.
    ///
    /// Called once, on the way out of the client, so every caller reads the
    /// same field it always read.
    fn hydrate(&mut self) {
        self.knowledge_item_ids.clear();
        let Some((_, marker)) = self.text.rsplit_once("[Synveda Knowledge: ") else {
            return;
        };
        let ids = marker.split(']').next().unwrap_or_default().trim();
        if ids.is_empty() {
            return;
        }
        self.knowledge_item_ids = ids
            .split(',')
            .filter_map(|address| address.trim().strip_prefix("knowledge:"))
            .filter_map(|address| address.split('@').next())
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .collect();
    }
}

/// `POST /v1/sessions/{id}/events` (MEM-1, ADR-0020; re-anchored by CPR-12,
/// ADR-0078 decision 1).
#[derive(Debug, Serialize)]
pub struct SessionEventBatchRequest<'a> {
    pub events: Vec<SessionEventInput<'a>>,
}

#[derive(Debug, Serialize)]
pub struct SessionEventInput<'a> {
    #[serde(rename = "client_event_id")]
    pub idempotency_key: String,
    #[serde(rename = "event_type")]
    pub kind: &'a str,
    pub payload: serde_json::Value,
    pub occurred_at: String,
}

#[derive(Debug, Deserialize)]
pub struct SessionEventBatchResponse {
    #[serde(rename = "appended")]
    pub accepted: usize,
    pub duplicates: usize,
    pub quarantined: usize,
    pub denied: usize,
    /// Per-event outcomes, which is what makes the extraction measurement
    /// attributable: the `event_id` acked here is the same id the served
    /// Knowledge source carries and the same id the capture candidate names,
    /// so one key joins the seed, the read, and the audit chain.
    #[serde(default)]
    pub events: Vec<SessionEventOutcome>,
}

#[derive(Debug, Deserialize)]
pub struct SessionEventOutcome {
    #[serde(rename = "client_event_id")]
    pub idempotency_key: String,
    pub outcome: String,
    /// The stored row, absent for a denied event: nothing was persisted.
    #[serde(default)]
    pub event: Option<StoredEventRef>,
}

#[derive(Debug, Deserialize)]
pub struct StoredEventRef {
    pub id: String,
}

impl SessionEventOutcome {
    /// The stored event's id, when one was stored.
    pub fn event_id(&self) -> Option<&str> {
        self.event.as_ref().map(|event| event.id.as_str())
    }
}

/// `GET /v1/me`, reduced to the principal anchor needed for private
/// candidate publication.
#[derive(Debug, Deserialize)]
pub struct MeResponse {
    #[serde(default)]
    pub anchors: Vec<MeAnchor>,
}

#[derive(Debug, Deserialize)]
pub struct MeAnchor {
    pub scope_id: String,
    pub source: String,
}

impl MeResponse {
    fn principal_scope_id(&self) -> Result<&str, String> {
        self.anchors
            .iter()
            .find(|anchor| anchor.source == "principal_scope")
            .map(|anchor| anchor.scope_id.as_str())
            .ok_or_else(|| {
                "/v1/me named no principal scope for private evaluation Knowledge".to_owned()
            })
    }
}

/// `POST /v1/sessions` — the run a measurement is attributed to.
#[derive(Debug, Serialize)]
pub struct OpenSessionRequest<'a> {
    pub workspace_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<&'a str>,
    pub client_name: &'a str,
    pub external_session_id: &'a str,
}

/// A run, reduced to its address. The body carries more; the id is the
/// whole of what a caller needs to post events or compose against it.
#[derive(Debug, Deserialize)]
pub struct SessionRef {
    pub id: String,
}

/// One explicit session-capture job. Evaluation never treats extraction as
/// publication: it waits for this candidate-only batch and then reviews the
/// candidates through the same public VedaFlow-backed action as the product.
#[derive(Debug, Clone, Deserialize)]
pub struct CaptureBatchRef {
    pub id: String,
    pub state: String,
    pub candidate_count: i32,
    #[serde(default)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CaptureCandidateRef {
    pub id: String,
    pub state: String,
    pub proposed_scope_id: String,
    pub content: serde_json::Value,
    pub source_event_ids: Vec<String>,
    #[serde(default)]
    pub resulting_change_id: Option<String>,
    #[serde(default)]
    pub resulting_outcome: Option<String>,
    #[serde(default)]
    pub resulting_knowledge_item_id: Option<String>,
    #[serde(default)]
    pub resulting_revision_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CaptureCandidatePage {
    candidates: Vec<CaptureCandidateRef>,
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CaptureDecisionRef {
    candidate: CaptureCandidateRef,
}

/// Optional governed edits made while accepting a candidate. This is how the
/// Q&A/security corpora place their premise through the current Knowledge
/// mutation path; it is not a database fixture or a post-publication move.
#[derive(Debug, Clone)]
pub struct CaptureReplacement {
    pub item_id: String,
    pub revision_id: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CaptureAcceptOptions<'a> {
    pub scope_id: Option<&'a str>,
    pub sensitivity: Option<&'a str>,
    /// Source event id to the exact current Knowledge head it supersedes.
    /// The candidate API, not the extractor, performs the governed change.
    pub replacements: Option<&'a BTreeMap<String, CaptureReplacement>>,
}

/// Diagnostic, cursor-paginated enumeration over one session's Knowledge
/// universe. `session_id` remains the eval label; the client resolves it to a
/// real run before calling the public application API.
#[derive(Debug)]
pub struct KnowledgeSweepRequest<'a> {
    pub as_of: &'a str,
    pub session_id: &'a str,
    /// Asked for explicitly rather than left to the surface's default, so
    /// "I asked for N and got N" is a fact this caller can state without
    /// knowing the product's cap (ADR-0046 decision 3).
    pub limit: usize,
}

/// Ordinary deep query over current Knowledge, separate from composition.
#[derive(Debug, Serialize)]
pub struct KnowledgeQueryRequest<'a> {
    pub query: &'a str,
    pub limit: usize,
}

/// Diagnostic fetch-by-id probe. Every returned item is still decided exactly.
#[derive(Debug)]
pub struct KnowledgeIdsRequest<'a> {
    pub ids: Vec<String>,
    pub session_id: &'a str,
}

#[derive(Debug, Deserialize)]
struct KnowledgeQueryResponse {
    items: Vec<KnowledgeQueryEntry>,
    #[serde(rename = "retrieval_mode")]
    _retrieval_mode: String,
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KnowledgeQueryEntry {
    knowledge: KnowledgeWireItem,
    #[serde(default)]
    sources: Vec<KnowledgeWireSource>,
}

#[derive(Debug, Deserialize)]
struct KnowledgeWireItem {
    id: String,
    knowledge_type: String,
    current_revision: KnowledgeWireRevision,
}

#[derive(Debug, Deserialize)]
struct KnowledgeWireRevision {
    body_markdown: String,
}

#[derive(Debug, Deserialize)]
struct KnowledgeWireSource {
    session_event_id: Option<String>,
    #[serde(default)]
    metadata: serde_json::Value,
}

impl KnowledgeQueryResponse {
    fn into_eval(self, mode: &str) -> KnowledgeResults {
        let entries = self
            .items
            .into_iter()
            .map(|entry| {
                let source_event_ids = entry
                    .sources
                    .iter()
                    .filter_map(|source| source.session_event_id.clone())
                    .collect::<Vec<_>>();
                let mut provenance = entry
                    .sources
                    .first()
                    .map(|source| source.metadata.clone())
                    .filter(serde_json::Value::is_object)
                    .unwrap_or_else(|| serde_json::json!({}));
                if let (Some(object), Some(event_id)) = (
                    provenance.as_object_mut(),
                    entry
                        .sources
                        .first()
                        .and_then(|source| source.session_event_id.clone()),
                ) {
                    object.insert("event_id".to_owned(), serde_json::Value::String(event_id));
                }
                KnowledgeResult {
                    knowledge_item_id: entry.knowledge.id,
                    class: entry.knowledge.knowledge_type,
                    content: entry.knowledge.current_revision.body_markdown,
                    source_event_ids,
                    provenance,
                }
            })
            .collect();
        KnowledgeResults {
            entries,
            mode: mode.to_owned(),
            truncated: self.next_cursor.is_some(),
            // The new lens decides exact items, not aggregate scopes. The
            // old report columns remain zero until Prompt 30 replaces them
            // with Knowledge/PDP evidence rather than fabricating a count.
            scopes_considered: 0,
            scopes_decided: 0,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct KnowledgeResults {
    pub entries: Vec<KnowledgeResult>,
    /// Which shape the surface decided it was asked (ADR-0042 decision 1).
    /// Checked rather than assumed: a request the surface read as `ids` or
    /// `query` would answer a different question, and a measurement of the
    /// wrong question is worse than no measurement.
    pub mode: String,
    /// The *scope* cap, not the Knowledge-item cap — which is exactly why a caller
    /// cannot read `false` here as "this page is complete" (ADR-0046
    /// decision 3).
    pub truncated: bool,
    pub scopes_considered: usize,
    pub scopes_decided: usize,
}

/// One served Knowledge item, as the extraction measurement reads it.
#[derive(Debug, Deserialize)]
pub struct KnowledgeResult {
    pub knowledge_item_id: String,
    pub class: String,
    /// Untruncated: recall does not elide what the caller named
    /// (ADR-0041 decision 7).
    pub content: String,
    /// Source session, extraction method, model version, confidence
    /// (seed §4.2). The attribution key and the model identity both live
    /// here.
    pub provenance: serde_json::Value,
    /// Every exact session-event source authorised on this Knowledge result.
    /// A revision may merge provenance; retaining only the first source would
    /// make the evaluation join silently lose evidence.
    #[serde(default)]
    pub source_event_ids: Vec<String>,
}

impl KnowledgeResult {
    /// The source session event this Knowledge revision was derived from,
    /// when provenance names one. Absence is a fact about the revision,
    /// never a default: material written by a path that did not retain it is
    /// attributed to nothing rather than to the wrong fixture.
    pub fn source_event_id(&self) -> Option<&str> {
        self.source_event_ids
            .first()
            .map(String::as_str)
            .or_else(|| self.provenance.get("event_id")?.as_str())
    }

    /// All exact source events, preserving the public response's provenance
    /// order. Callers doing attribution must use this rather than assuming a
    /// revision has only one source.
    pub fn source_event_ids(&self) -> impl Iterator<Item = &str> {
        self.source_event_ids.iter().map(String::as_str)
    }

    pub fn source_session_id(&self) -> Option<&str> {
        self.provenance.get("session_id")?.as_str()
    }

    /// The model the API actually served, as the pipeline retained it —
    /// not the alias the request asked for (ADR-0046 decision 12).
    pub fn model_version(&self) -> Option<&str> {
        self.provenance.get("model_version")?.as_str()
    }
}

#[derive(Debug, Deserialize)]
pub struct Proposal {
    /// Exact immutable commit a verdict must echo.
    pub commit: String,
    /// `open` | `approved` | `rejected` | `withdrawn` | `published`.
    pub state: String,
}

/// A call's result and what it cost, because the cost is a measurement.
pub struct Timed<T> {
    pub value: T,
    pub elapsed_ms: f64,
    /// The degradation ladder's header (ADR-0026 decision 4). A degraded
    /// measurement is still a measurement, and the report says so rather
    /// than quietly averaging it in.
    pub degraded: Vec<String>,
}

impl Client {
    pub fn new(environment: &Environment) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(TIMEOUT)
            .build()
            .map_err(|err| format!("build the HTTP client: {err}"))?;
        let session_selections = environment
            .actors
            .values()
            .filter_map(|actor| {
                actor.workspace_id.as_ref().map(|workspace_id| {
                    (
                        actor.token.clone(),
                        SessionSelection {
                            workspace_id: workspace_id.clone(),
                            project_id: actor.project_id.clone(),
                        },
                    )
                })
            })
            .collect();
        Ok(Self {
            gateway_url: environment.gateway_url.trim_end_matches('/').to_owned(),
            http,
            session_selections,
        })
    }

    /// What this caller can see, for resolving its principal scope.
    pub async fn me(&self, bearer: &str) -> Result<MeResponse, String> {
        self.get("/v1/me", bearer, &[]).await
    }

    /// The run a scenario's `session_id` label maps to.
    ///
    /// A label was enough when a session was an opaque correlation string; a
    /// run is an aggregate now (CPR-10), so the harness opens one per label
    /// and keys its calls on the real id. Idempotent, so a scenario re-run
    /// lands on the run it opened last time.
    pub async fn session_for(&self, bearer: &str, label: &str) -> Result<String, String> {
        let selection = self.session_selections.get(bearer).ok_or_else(|| {
            format!(
                "evaluation actor opening session `{label}` must name workspace_id in the \
                 current environment; the harness does not discover or create product state"
            )
        })?;
        Ok(self
            .open_session(
                bearer,
                &OpenSessionRequest {
                    workspace_id: &selection.workspace_id,
                    project_id: selection.project_id.as_deref(),
                    client_name: "synveda-eval",
                    external_session_id: label,
                },
            )
            .await?
            .value
            .id)
    }

    /// Opens the run a measurement is attributed to.
    ///
    /// Idempotent on the harness's own external id, so a re-run of the same
    /// scenario lands on the same run rather than minting a second one.
    pub async fn open_session(
        &self,
        bearer: &str,
        request: &OpenSessionRequest<'_>,
    ) -> Result<Timed<SessionRef>, String> {
        let key = format!("eval-open-{}", request.external_session_id);
        self.post_idempotent("/v1/sessions", bearer, request, &key)
            .await
    }

    /// Composes context for `session`.
    pub async fn compose_context(
        &self,
        bearer: &str,
        session: &str,
        request: &ContextRunRequest<'_>,
    ) -> Result<Timed<ContextRunResponse>, String> {
        let key = format!("eval-ctx-{}", uuid_like());
        let mut timed: Timed<ContextRunResponse> = self
            .post_idempotent(
                &format!("/v1/sessions/{session}/context-runs"),
                bearer,
                request,
                &key,
            )
            .await?;
        timed.value.hydrate();
        Ok(timed)
    }

    /// Appends observations to `session`.
    pub async fn append_events(
        &self,
        bearer: &str,
        session: &str,
        request: &SessionEventBatchRequest<'_>,
    ) -> Result<Timed<SessionEventBatchResponse>, String> {
        self.post(&format!("/v1/sessions/{session}/events"), bearer, request)
            .await
    }

    /// Freeze, await and review one session snapshot through the public
    /// capture and Knowledge/VedaFlow APIs. A deterministic rerun resolves to
    /// the same batch and terminal decisions through idempotency.
    pub async fn capture_and_accept(
        &self,
        bearer: &str,
        session: &str,
        key: &str,
        timeout: Duration,
        options: CaptureAcceptOptions<'_>,
    ) -> Result<Vec<CaptureCandidateRef>, String> {
        let path = format!("/v1/sessions/{session}/capture-batches");
        let started: Timed<CaptureBatchRef> = self
            .post_idempotent(&path, bearer, &serde_json::json!({}), key)
            .await?;
        let batch_id = started.value.id;
        let began = Instant::now();
        let batch = loop {
            let batch: CaptureBatchRef = self
                .get(&format!("/v1/capture-batches/{batch_id}"), bearer, &[])
                .await?;
            match batch.state.as_str() {
                "completed" => break batch,
                "failed" => {
                    return Err(format!(
                        "capture batch {batch_id} failed: {}",
                        batch.error_code.as_deref().unwrap_or("unknown")
                    ));
                }
                _ if began.elapsed() >= timeout => {
                    return Err(format!(
                        "capture batch {batch_id} did not complete within {}s",
                        timeout.as_secs()
                    ));
                }
                _ => tokio::time::sleep(Duration::from_millis(250)).await,
            }
        };
        let page: CaptureCandidatePage = self
            .get(
                "/v1/capture-candidates",
                bearer,
                &[
                    ("batch_id".to_owned(), batch_id.clone()),
                    ("limit".to_owned(), "200".to_owned()),
                ],
            )
            .await?;
        if page.next_cursor.is_some() || page.candidates.len() != batch.candidate_count as usize {
            return Err(format!(
                "capture batch {batch_id} reports {} candidates but its one bounded page served {}{}",
                batch.candidate_count,
                page.candidates.len(),
                if page.next_cursor.is_some() {
                    " with another page"
                } else {
                    ""
                }
            ));
        }

        // A generic corpus seed is private to its author. Shared Q&A and
        // security fixtures name their governed target explicitly below; an
        // omitted target must not silently publish into the session workspace
        // and turn a deterministic eval into an outstanding review queue.
        let private_scope = if options.scope_id.is_none() {
            let me = self.me(bearer).await?;
            Some(me.principal_scope_id()?.to_owned())
        } else {
            None
        };
        let target_scope = options.scope_id.or(private_scope.as_deref());

        let mut accepted = Vec::with_capacity(page.candidates.len());
        for candidate in page.candidates {
            if candidate.state != "pending" {
                accepted.push(candidate);
                continue;
            }
            let mut edits = serde_json::json!({});
            if let Some(scope_id) = target_scope {
                edits["scope_id"] = serde_json::Value::String(scope_id.to_owned());
                // A project association is valid only at that project's own
                // governing scope. Moving a session candidate to its author's
                // private scope, a workspace, or the tenant must explicitly
                // clear the session project; omission means "retain" on this
                // tri-state API.
                if scope_id != candidate.proposed_scope_id {
                    edits["project_id"] = serde_json::Value::Null;
                }
            }
            if let Some(sensitivity) = options.sensitivity {
                let mut content = candidate.content.clone();
                content["sensitivity"] = serde_json::Value::String(sensitivity.to_owned());
                edits["content"] = content;
            }
            let replacement = options.replacements.and_then(|replacements| {
                candidate
                    .source_event_ids
                    .iter()
                    .find_map(|event_id| replacements.get(event_id))
            });
            let (action, body) = replacement.map_or_else(
                || ("accept", edits.clone()),
                |target| {
                    (
                        "replace",
                        serde_json::json!({
                            "item_id": target.item_id,
                            "expected_revision_id": target.revision_id,
                            "replacement": edits,
                        }),
                    )
                },
            );
            let decision: Timed<CaptureDecisionRef> = self
                .post_idempotent(
                    &format!("/v1/capture-candidates/{}/{action}", candidate.id),
                    bearer,
                    &body,
                    &format!("eval-{action}-{}", candidate.id),
                )
                .await?;
            accepted.push(decision.value.candidate);
        }
        Ok(accepted)
    }

    /// Enumerates the current visible Knowledge corpus through the diagnostic
    /// lens, following every opaque cursor until `limit` or exhaustion.
    pub async fn knowledge_sweep(
        &self,
        bearer: &str,
        request: &KnowledgeSweepRequest<'_>,
    ) -> Result<Timed<KnowledgeResults>, String> {
        if request.limit == 0 {
            return Err("a Knowledge sweep limit must be at least one".to_owned());
        }
        let session = self.session_for(bearer, request.session_id).await?;
        let mut cursor: Option<String> = None;
        let mut entries = Vec::new();
        let mut elapsed_ms = 0.0;
        let mut degraded = Vec::new();
        let mut truncated = false;
        loop {
            let remaining = request.limit.saturating_sub(entries.len());
            if remaining == 0 {
                truncated = cursor.is_some();
                break;
            }
            let page_limit = remaining.min(100);
            let timed: Timed<KnowledgeQueryResponse> = self
                .post(
                    &format!("/v1/sessions/{session}/knowledge-evaluation"),
                    bearer,
                    &serde_json::json!({
                        "as_of": request.as_of,
                        "cursor": cursor.as_deref(),
                        "limit": page_limit,
                    }),
                )
                .await?;
            elapsed_ms += timed.elapsed_ms;
            for warning in timed.degraded {
                if !degraded.contains(&warning) {
                    degraded.push(warning);
                }
            }
            let next = timed.value.next_cursor.clone();
            let mut page = timed.value.into_eval("sweep");
            entries.append(&mut page.entries);
            let Some(next) = next else { break };
            if cursor.as_deref() == Some(next.as_str()) {
                return Err("Knowledge evaluation repeated its cursor".to_owned());
            }
            cursor = Some(next);
        }
        Ok(Timed {
            value: KnowledgeResults {
                entries,
                mode: "sweep".to_owned(),
                truncated,
                scopes_considered: 0,
                scopes_decided: 0,
            },
            elapsed_ms,
            degraded,
        })
    }

    /// Ranked, non-budgeted current Knowledge query under ordinary
    /// `SessionRead` authority.
    pub async fn knowledge_query(
        &self,
        bearer: &str,
        session: &str,
        request: &KnowledgeQueryRequest<'_>,
    ) -> Result<Timed<KnowledgeResults>, String> {
        let timed: Timed<KnowledgeQueryResponse> = self
            .post(
                &format!("/v1/sessions/{session}/knowledge-query"),
                bearer,
                request,
            )
            .await?;
        Ok(Timed {
            value: timed.value.into_eval("query"),
            elapsed_ms: timed.elapsed_ms,
            degraded: timed.degraded,
        })
    }

    /// Fetches exact ids through the diagnostic lens, chunking at the public
    /// API bound. Denied ids return no object-shaped result.
    pub async fn knowledge_ids(
        &self,
        bearer: &str,
        request: &KnowledgeIdsRequest<'_>,
    ) -> Result<Timed<KnowledgeResults>, String> {
        if request.ids.is_empty() {
            return Ok(Timed {
                value: KnowledgeResults {
                    entries: Vec::new(),
                    mode: "ids".to_owned(),
                    truncated: false,
                    scopes_considered: 0,
                    scopes_decided: 0,
                },
                elapsed_ms: 0.0,
                degraded: Vec::new(),
            });
        }
        let session = self.session_for(bearer, request.session_id).await?;
        let mut entries = Vec::new();
        let mut elapsed_ms = 0.0;
        let mut degraded = Vec::new();
        for ids in request.ids.chunks(100) {
            let timed: Timed<KnowledgeQueryResponse> = self
                .post(
                    &format!("/v1/sessions/{session}/knowledge-evaluation"),
                    bearer,
                    &serde_json::json!({"ids": ids, "limit": ids.len()}),
                )
                .await?;
            elapsed_ms += timed.elapsed_ms;
            for warning in timed.degraded {
                if !degraded.contains(&warning) {
                    degraded.push(warning);
                }
            }
            entries.extend(timed.value.into_eval("ids").entries);
        }
        Ok(Timed {
            value: KnowledgeResults {
                entries,
                mode: "ids".to_owned(),
                truncated: false,
                scopes_considered: 0,
                scopes_decided: 0,
            },
            elapsed_ms,
            degraded,
        })
    }

    /// One approver's verdict. The caller repeats this with a different
    /// bearer until the proposal leaves `open`, because how many distinct
    /// approvers and which roles is the pack's answer, not the harness's.
    pub async fn approve(&self, bearer: &str, proposal: &str) -> Result<Timed<Proposal>, String> {
        let current: Proposal = self
            .get(&format!("/v1/proposals/{proposal}"), bearer, &[])
            .await?;
        self.post(
            &format!("/v1/proposals/{proposal}/approve"),
            bearer,
            &serde_json::json!({"expected_commit": current.commit}),
        )
        .await
    }

    /// Applies an approved typed VedaFlow change. Unlike `publish`, this is
    /// the current aggregate-command route used by Knowledge, Skills, Tools
    /// and governed Configuration.
    pub async fn apply(
        &self,
        bearer: &str,
        proposal: &str,
    ) -> Result<Timed<serde_json::Value>, String> {
        self.post(
            &format!("/v1/proposals/{proposal}/apply"),
            bearer,
            &serde_json::json!({}),
        )
        .await
    }

    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        bearer: &str,
        query: &[(String, String)],
    ) -> Result<T, String> {
        let response = self
            .http
            .get(format!("{}{path}", self.gateway_url))
            .query(query)
            .bearer_auth(bearer)
            .header(
                "x-synveda-client",
                concat!("synveda-eval/", env!("CARGO_PKG_VERSION")),
            )
            .send()
            .await
            .map_err(|err| format!("GET {path}: {err}"))?;
        let status = response.status();
        let raw = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!("GET {path} returned {status}: {}", detail(&raw)));
        }
        serde_json::from_str(&raw)
            .map_err(|err| format!("GET {path} returned an unreadable body: {err}"))
    }

    async fn post<B: Serialize, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        bearer: &str,
        body: &B,
    ) -> Result<Timed<T>, String> {
        self.send(path, bearer, Some(body), None).await
    }

    /// [`Client::post`] carrying an `Idempotency-Key`, which every creation on
    /// the context-platform plane requires (ADR-0071).
    async fn post_idempotent<B: Serialize, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        bearer: &str,
        body: &B,
        key: &str,
    ) -> Result<Timed<T>, String> {
        self.send(path, bearer, Some(body), Some(key)).await
    }

    async fn send<B: Serialize, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        bearer: &str,
        body: Option<&B>,
        key: Option<&str>,
    ) -> Result<Timed<T>, String> {
        let started = Instant::now();
        let url = format!("{}{path}", self.gateway_url);
        let mut request = match body {
            Some(_) => self.http.post(&url),
            None => self.http.get(&url),
        };
        request = request.bearer_auth(bearer).header(
            "x-synveda-client",
            concat!("synveda-eval/", env!("CARGO_PKG_VERSION")),
        );
        if let Some(key) = key {
            request = request.header("idempotency-key", key);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request
            .send()
            .await
            .map_err(|err| format!("{path}: {err}"))?;
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

        let status = response.status();
        let degraded = response
            .headers()
            .get("x-synveda-degraded")
            .and_then(|value| value.to_str().ok())
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let raw = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!("{path} returned {status}: {}", detail(&raw)));
        }
        let value = serde_json::from_str(&raw)
            .map_err(|err| format!("{path} returned an unreadable body: {err}"))?;
        Ok(Timed {
            value,
            elapsed_ms,
            degraded,
        })
    }
}

/// The gateway's error taxonomy carries a `message`; anything else is
/// printed as it arrived, truncated.
fn detail(raw: &str) -> String {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| raw.chars().take(200).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_context_run_request_omits_what_it_does_not_set() {
        // A `null` task is not the same request as no task: the taskless
        // branch is chosen by absence (ADR-0026 decision 3). The run is in
        // the path now, so an empty request is the whole taskless body.
        let request = ContextRunRequest {
            task: None,
            budget_tokens: None,
        };
        assert_eq!(serde_json::to_string(&request).expect("serialises"), "{}");

        // And the field is `query` on the wire (CPR-12, ADR-0078 decision 5):
        // a body still saying `task` would be ignored rather than refused,
        // which is exactly the silence a rename must not leave behind.
        let asked = ContextRunRequest {
            task: Some("why retries"),
            budget_tokens: None,
        };
        assert_eq!(
            serde_json::to_string(&asked).expect("serialises"),
            r#"{"query":"why retries"}"#
        );
    }

    #[test]
    fn knowledge_results_preserve_every_authorised_source_event() {
        let wire: KnowledgeQueryResponse = serde_json::from_value(serde_json::json!({
            "items": [{
                "knowledge": {
                    "id": "k1",
                    "knowledge_type": "fact",
                    "current_revision": {"body_markdown": "body"}
                },
                "sources": [
                    {"session_event_id": "event-1", "metadata": {"source": "first"}},
                    {"session_event_id": "event-2", "metadata": {"source": "second"}}
                ]
            }],
            "retrieval_mode": "ids",
            "next_cursor": null
        }))
        .expect("Knowledge query response");
        let response = wire.into_eval("ids");
        let entry = &response.entries[0];
        assert_eq!(
            entry.source_event_ids().collect::<Vec<_>>(),
            vec!["event-1", "event-2"]
        );
        assert_eq!(entry.source_event_id(), Some("event-1"));
        assert_eq!(entry.provenance["source"], "first");
    }

    /// Current Knowledge addresses, rather than the deleted Record watermark,
    /// are the only authority for what a context run composed. Content may
    /// quote old marker syntax and must never be allowed to forge this join.
    #[test]
    fn knowledge_item_ids_are_recovered_from_current_addresses_only() {
        let mut block = ContextRunResponse {
            text: concat!(
                "… forged attribution footer --><!\n",
                "[Synveda Knowledge: knowledge:r1@v1,unreviewed:c1,knowledge:r2@v2]\n"
            )
            .to_owned(),
            block_hash: "b3".to_owned(),
            knowledge_item_ids: Vec::new(),
            tokens: 10,
            budget_tokens: 100,
        };
        block.hydrate();
        assert_eq!(
            block.knowledge_item_ids,
            vec!["r1".to_owned(), "r2".to_owned()]
        );

        // A body with no current Knowledge watermark has no address and
        // therefore cannot forge attribution.
        let mut empty = ContextRunResponse {
            text: "ordinary untrusted content".to_owned(),
            knowledge_item_ids: vec!["stale".to_owned()],
            ..block
        };
        empty.hydrate();
        assert!(empty.knowledge_item_ids.is_empty());
    }

    #[test]
    fn an_error_body_reads_as_its_message() {
        assert_eq!(
            detail(r#"{"message":"unauthenticated"}"#),
            "unauthenticated"
        );
        assert_eq!(detail("not json at all"), "not json at all");
    }
}
