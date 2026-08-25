//! The `/v1` client (EVAL-1, ADR-0028 decision 1).
//!
//! An actor's own bearer and no other way in. The wire structs are declared
//! here rather than imported because this crate depends on no Synveda crate
//! at all — the same price the TypeScript adapter pays, for the same reason:
//! what an outside caller can see is exactly what an eval should measure.
//!
//! **Re-anchored on the session plane by CPR-12** (ADR-0078 decisions 1 and
//! 5). What was a global `/v1/observe` with a `session_id` field is now
//! `POST /v1/sessions/{id}/events` against a run this harness opened, and
//! what was `/v1/inject` is that run's `context-runs`. Every fixture label
//! that used to *be* a session id is now looked up through
//! [`Client::session_for`], which opens one run per label and reuses it.
//!
//! CPR-20 re-cuts deep query onto the ordinary session-scoped Knowledge lens
//! and corpus enumeration/id probes onto its stricter `SessionDiagnostics`
//! lens. Neither abuses a budgeted context run and neither restores a global
//! recall route. Prompt 30 re-measures the suites against accepted Knowledge.
//!
//! The audit search is untouched (ADR-0046 decision 4): the sweep says what
//! a *reader is served*, `GET /v1/audit/events` says what the *pipeline
//! committed*, and only the second of those two lenses still has a route.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Generous next to inject's 150ms SLO: a deadline here is meant to end a
/// hung run, not to be the thing under measurement. The latency axis
/// measures what the call took, not what it was allowed to take.
const TIMEOUT: Duration = Duration::from_secs(30);

pub struct Client {
    gateway_url: String,
    http: reqwest::Client,
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
pub struct InjectRequest<'a> {
    #[serde(rename = "query", skip_serializing_if = "Option::is_none")]
    pub task: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<u32>,
}

/// Only what a measurement needs. The body also carries its own
/// `degraded` list; the header is the one this reads, because the header
/// is what ADR-0026 decision 4 makes the warning.
///
/// EVAL-4 stopped ignoring four fields the gateway was already sending
/// (ADR-0047 decision 1): the per-entry tier and the index counters, which
/// are how a demotion is told from an absence, and the staleness scores,
/// which are MEM-6's unvalidated heuristic (ADR-0040) measured for the
/// first time.
/// The composed block.
///
/// **Four fields the response no longer carries.** `/v1/inject` served
/// `record_ids`, `tiers`, `index_entries`, `index_tokens` and
/// `staleness_permille` as fields; a context run's body is deliberately
/// minimal (ADR-0076 decision 7) and serves the rendered block. `record_ids`
/// is recovered from the block's own watermark line, which is where a block
/// names what it composed; the other four have no substitute here and are
/// left empty, which grades every per-tier and staleness assertion as a miss
/// rather than as a pass. EVAL-4's tier axis therefore needs Prompt 18 before
/// it measures anything again.
#[derive(Debug, Deserialize)]
pub struct InjectResponse {
    #[serde(rename = "rendered")]
    pub text: String,
    /// The watermark (ADR-0025 decision 7). It rides into the report so a
    /// measurement can be traced back to exactly the block that produced
    /// it, months later, from the audit chain.
    pub block_hash: String,
    /// Parsed from the watermark rather than served as a field; see
    /// [`InjectResponse::record_ids`].
    #[serde(default, skip)]
    pub record_ids: Vec<String>,
    /// How much of each composed record the block carried, in block order:
    /// `body` or `index` (CTX-4, ADR-0041 decision 9).
    #[serde(default)]
    pub tiers: Vec<String>,
    #[serde(default)]
    pub index_entries: usize,
    #[serde(default)]
    pub index_tokens: u32,
    /// Per mille freshness at `as_of`, in block order (MEM-6, ADR-0040
    /// decision 12).
    #[serde(default)]
    pub staleness_permille: Vec<u16>,
    pub tokens: u32,
    pub budget_tokens: u32,
}

impl InjectResponse {
    /// Fills `record_ids` from the block's watermark line.
    ///
    /// Called once, on the way out of the client, so every caller reads the
    /// same field it always read.
    fn hydrate(&mut self) {
        if let Some(marker) = self.text.split("records=").nth(1) {
            let ids = marker.split("-->").next().unwrap_or_default().trim();
            if !ids.is_empty() && ids != "none" {
                self.record_ids = ids.split(',').map(|id| id.trim().to_owned()).collect();
                return;
            }
        }
        let Some((_, marker)) = self.text.rsplit_once("[Synveda Knowledge: ") else {
            return;
        };
        let ids = marker.split(']').next().unwrap_or_default().trim();
        if ids.is_empty() {
            return;
        }
        self.record_ids = ids
            .split(',')
            .filter_map(|address| address.trim().split('@').next())
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .collect();
    }
}

impl InjectResponse {
    /// The tier a record composed at, or `None` when the block does not
    /// carry it. Absent rather than defaulted: "not in the block" and
    /// "in the block at the body tier" are the two answers this whole
    /// suite turns on, and a default would merge them. A response whose
    /// `tiers` array is shorter than its `record_ids` reads as `None`
    /// too, which grades as a miss — conservative, and the right way
    /// round for a contract violation nobody has seen.
    #[must_use]
    pub fn tier_of(&self, record_id: &str) -> Option<&str> {
        let position = self.record_ids.iter().position(|id| id == record_id)?;
        // A gateway that sent fewer tiers than records would otherwise
        // read as "this record was not carried"; say so instead.
        self.tiers.get(position).map(String::as_str)
    }
}

/// `POST /v1/sessions/{id}/events` (MEM-1, ADR-0020; re-anchored by CPR-12,
/// ADR-0078 decision 1).
#[derive(Debug, Serialize)]
pub struct ObserveRequest<'a> {
    pub events: Vec<ObserveEvent<'a>>,
}

#[derive(Debug, Serialize)]
pub struct ObserveEvent<'a> {
    #[serde(rename = "client_event_id")]
    pub idempotency_key: String,
    #[serde(rename = "event_type")]
    pub kind: &'a str,
    pub payload: serde_json::Value,
    pub occurred_at: String,
}

#[derive(Debug, Deserialize)]
pub struct ObserveResponse {
    #[serde(rename = "appended")]
    pub accepted: usize,
    pub duplicates: usize,
    pub quarantined: usize,
    pub denied: usize,
    /// Per-event outcomes, which is what makes the extraction measurement
    /// attributable: the `event_id` acked here is the same id the served
    /// record's `provenance` carries and the same id the `memory.extracted`
    /// payload names, so one key joins the seed, the read, and the chain.
    #[serde(default)]
    pub events: Vec<ObserveEventOutcome>,
}

#[derive(Debug, Deserialize)]
pub struct ObserveEventOutcome {
    #[serde(rename = "client_event_id")]
    pub idempotency_key: String,
    /// The stored row, absent for a denied event: nothing was persisted.
    #[serde(default)]
    pub event: Option<StoredEventRef>,
}

#[derive(Debug, Deserialize)]
pub struct StoredEventRef {
    pub id: String,
}

impl ObserveEventOutcome {
    /// The stored event's id, when one was stored.
    pub fn event_id(&self) -> Option<&str> {
        self.event.as_ref().map(|event| event.id.as_str())
    }
}

/// `GET /v1/me`, as much of it as opening a run needs.
#[derive(Debug, Deserialize)]
pub struct MeResponse {
    #[serde(default)]
    pub workspaces: Vec<MeWorkspace>,
}

#[derive(Debug, Deserialize)]
pub struct MeWorkspace {
    pub id: String,
}

/// `POST /v1/workspaces`.
#[derive(Debug, Serialize)]
pub struct NewWorkspaceRequest<'a> {
    pub slug: &'a str,
    pub display_name: &'a str,
}

/// `POST /v1/sessions` — the run a measurement is attributed to.
#[derive(Debug, Serialize)]
pub struct OpenSessionRequest<'a> {
    pub workspace_id: &'a str,
    pub client_name: &'a str,
    pub external_session_id: &'a str,
}

/// A run, reduced to its address. The body carries more; the id is the
/// whole of what a caller needs to post events or compose against it.
#[derive(Debug, Deserialize)]
pub struct SessionRef {
    pub id: String,
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
    scope_id: String,
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
    fn into_eval(self, mode: &str) -> RecallResponse {
        let entries = self
            .items
            .into_iter()
            .map(|entry| {
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
                RecallEntry {
                    record_id: entry.knowledge.id,
                    scope_id: entry.knowledge.scope_id,
                    class: entry.knowledge.knowledge_type,
                    content: entry.knowledge.current_revision.body_markdown,
                    provenance,
                }
            })
            .collect();
        RecallResponse {
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
pub struct RecallResponse {
    pub entries: Vec<RecallEntry>,
    /// Which shape the surface decided it was asked (ADR-0042 decision 1).
    /// Checked rather than assumed: a request the surface read as `ids` or
    /// `query` would answer a different question, and a measurement of the
    /// wrong question is worse than no measurement.
    pub mode: String,
    /// The *scope* cap, not the record cap — which is exactly why a caller
    /// cannot read `false` here as "this page is complete" (ADR-0046
    /// decision 3).
    pub truncated: bool,
    pub scopes_considered: usize,
    pub scopes_decided: usize,
}

/// One served record, as the extraction measurement reads it.
#[derive(Debug, Deserialize)]
pub struct RecallEntry {
    pub record_id: String,
    /// Where the record lives. EVAL-4 needs it as a promotion's
    /// `source_scope_id`: material sits at its author's personal leaf, and
    /// naming that leaf is how a climb says where it is coming from
    /// (ADR-0034 decision 2).
    pub scope_id: String,
    pub class: String,
    /// Untruncated: recall does not elide what the caller named
    /// (ADR-0041 decision 7).
    pub content: String,
    /// Source session, extraction method, model version, confidence
    /// (seed §4.2). The attribution key and the model identity both live
    /// here.
    pub provenance: serde_json::Value,
}

impl RecallEntry {
    /// The observe event this record was extracted from, when provenance
    /// names one. Absent is a fact about the record, never a default:
    /// material written by a path that did not record it would be
    /// attributed to nothing rather than to the wrong fixture.
    pub fn source_event_id(&self) -> Option<&str> {
        self.provenance.get("event_id")?.as_str()
    }

    pub fn source_session_id(&self) -> Option<&str> {
        self.provenance.get("session_id")?.as_str()
    }

    /// The model the API actually served, as the pipeline recorded it —
    /// not the alias the request asked for (ADR-0046 decision 12).
    pub fn model_version(&self) -> Option<&str> {
        self.provenance.get("model_version")?.as_str()
    }
}

/// `POST /v1/proposals` (FLOW-3/FLOW-5, ADR-0032/ADR-0034) — a climb.
///
/// EVAL-4 needs this because nothing else can put material above a leaf:
/// observe writes land at the caller's home scope (ADR-0020) and a service
/// identity's home is a `principal`-shaped scope under its anchor (ADR-0018
/// decision 2), so a corpus that spans scope tiers is a corpus that was
/// promoted through review (ADR-0047 decision 3).
#[derive(Debug, Serialize)]
pub struct ProposalRequest<'a> {
    /// The scope whose published channel would move. Requirements resolve
    /// here and only here.
    pub scope_id: &'a str,
    /// Where the material is now — the author's own leaf. Must be the
    /// target or a descendant of it.
    pub source_scope_id: &'a str,
    pub record_ids: Vec<String>,
    pub title: String,
    /// What running this proposal would do. Absent is `published`, which
    /// is what a climb is. `classify` is EVAL-5's (ADR-0048 decision 7):
    /// the only mechanism in the product that installs a tier above the
    /// working one, and therefore the only way a leak suite can have
    /// `restricted` material whose premise is real.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<&'a str>,
    /// The tier a `classify` proposal installs. Required for that effect
    /// and refused for any other — a publication does not move a tier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensitivity: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
pub struct Proposal {
    pub id: String,
    /// Exact immutable commit a verdict must echo.
    pub commit: String,
    /// `open` | `approved` | `rejected` | `withdrawn` | `published`.
    pub state: String,
    /// What the proposal still lacks, in the pack's own words. The runner
    /// approves until this says `nothing` rather than hard-coding the
    /// approval matrix, so a pack that asks for a different set is
    /// followed rather than fought (ADR-0032).
    #[serde(default)]
    pub outstanding: String,
    #[serde(default)]
    pub target_scope_path: String,
}

#[derive(Debug, Deserialize)]
pub struct Published {
    pub scope_id: String,
    pub commit: String,
    /// How many records the publication added to the channel.
    pub added: usize,
}

/// `GET /v1/audit/events` (AUD-2, ADR-0045 decision 3).
#[derive(Debug, Deserialize)]
pub struct AuditEventsResponse {
    pub events: Vec<AuditEvent>,
    pub truncated: bool,
    pub next_cursor: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct AuditEvent {
    /// The chain position this fact was read at, so an attribution number
    /// names the range it came from rather than floating free.
    pub seq: i64,
    /// `success` or `failure`. A failed `memory.extracted` is a
    /// dead-lettered event — a different fact from "extracted no records",
    /// and one that must not read as the latter.
    pub outcome: String,
    pub payload: serde_json::Value,
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
    pub fn new(gateway_url: &str) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(TIMEOUT)
            .build()
            .map_err(|err| format!("build the HTTP client: {err}"))?;
        Ok(Self {
            gateway_url: gateway_url.trim_end_matches('/').to_owned(),
            http,
        })
    }

    /// What this caller can see, for choosing a workspace to run in.
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
        let me = self.me(bearer).await?;
        let workspace = match me.workspaces.first() {
            Some(workspace) => workspace.id.clone(),
            None => {
                self.create_workspace(
                    bearer,
                    &NewWorkspaceRequest {
                        slug: "eval",
                        display_name: "eval",
                    },
                )
                .await?
                .value
                .id
            }
        };
        Ok(self
            .open_session(
                bearer,
                &OpenSessionRequest {
                    workspace_id: &workspace,
                    client_name: "synveda-eval",
                    external_session_id: label,
                },
            )
            .await?
            .value
            .id)
    }

    /// Creates a workspace, for a tenant that has none.
    pub async fn create_workspace(
        &self,
        bearer: &str,
        request: &NewWorkspaceRequest<'_>,
    ) -> Result<Timed<SessionRef>, String> {
        self.post_idempotent("/v1/workspaces", bearer, request, request.slug)
            .await
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
    pub async fn inject(
        &self,
        bearer: &str,
        session: &str,
        request: &InjectRequest<'_>,
    ) -> Result<Timed<InjectResponse>, String> {
        let key = format!("eval-ctx-{}", uuid_like());
        let mut timed: Timed<InjectResponse> = self
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
    pub async fn observe(
        &self,
        bearer: &str,
        session: &str,
        request: &ObserveRequest<'_>,
    ) -> Result<Timed<ObserveResponse>, String> {
        self.post(&format!("/v1/sessions/{session}/events"), bearer, request)
            .await
    }

    /// Enumerates the current visible Knowledge corpus through the diagnostic
    /// lens, following every opaque cursor until `limit` or exhaustion.
    pub async fn knowledge_sweep(
        &self,
        bearer: &str,
        request: &KnowledgeSweepRequest<'_>,
    ) -> Result<Timed<RecallResponse>, String> {
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
            value: RecallResponse {
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
    ) -> Result<Timed<RecallResponse>, String> {
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
    ) -> Result<Timed<RecallResponse>, String> {
        if request.ids.is_empty() {
            return Ok(Timed {
                value: RecallResponse {
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
            value: RecallResponse {
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

    pub async fn propose(
        &self,
        bearer: &str,
        request: &ProposalRequest<'_>,
    ) -> Result<Timed<Proposal>, String> {
        self.post("/v1/proposals", bearer, request).await
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

    /// Runs the approved proposal's effect. Takes `MemoryRead` as well as
    /// the review authority — nobody publishes what they cannot read
    /// (ADR-0031 decision 12) — so it is the curator's call and never the
    /// steward's.
    pub async fn publish(&self, bearer: &str, proposal: &str) -> Result<Timed<Published>, String> {
        self.post(
            &format!("/v1/proposals/{proposal}/publish"),
            bearer,
            &serde_json::json!({}),
        )
        .await
    }

    /// One page of the chain, filtered to a single action. Paging is the
    /// caller's — `next_cursor` back in as `after` — because a helper that
    /// swallowed the pages would also swallow `truncated`, and a truncation
    /// nobody sees is the failure this whole surface refuses (ADR-0045
    /// decision 9).
    pub async fn audit_events(
        &self,
        bearer: &str,
        action: &str,
        after: Option<i64>,
        limit: usize,
    ) -> Result<AuditEventsResponse, String> {
        let mut query = vec![
            ("action".to_owned(), action.to_owned()),
            ("limit".to_owned(), limit.to_string()),
        ];
        if let Some(after) = after {
            query.push(("after".to_owned(), after.to_string()));
        }
        self.get("/v1/audit/events", bearer, &query).await
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
        let request = InjectRequest {
            task: None,
            budget_tokens: None,
        };
        assert_eq!(serde_json::to_string(&request).expect("serialises"), "{}");

        // And the field is `query` on the wire (CPR-12, ADR-0078 decision 5):
        // a body still saying `task` would be ignored rather than refused,
        // which is exactly the silence a rename must not leave behind.
        let asked = InjectRequest {
            task: Some("why retries"),
            budget_tokens: None,
        };
        assert_eq!(
            serde_json::to_string(&asked).expect("serialises"),
            r#"{"query":"why retries"}"#
        );
    }

    /// Where `record_ids` comes from now (CPR-12): a context run's body is
    /// minimal by ADR-0076 decision 7 and does not carry them, so they are
    /// read back off the block's own watermark — the line where a block
    /// names what it composed.
    #[test]
    fn record_ids_are_recovered_from_the_watermark() {
        let mut block = InjectResponse {
            text: "…\n<!-- synveda block=b3 records=r1, r2 -->".to_owned(),
            block_hash: "b3".to_owned(),
            record_ids: Vec::new(),
            tiers: Vec::new(),
            index_entries: 0,
            index_tokens: 0,
            staleness_permille: Vec::new(),
            tokens: 10,
            budget_tokens: 100,
        };
        block.hydrate();
        assert_eq!(block.record_ids, vec!["r1".to_owned(), "r2".to_owned()]);

        // A block that composed nothing says so, and must not read as one
        // record literally named "none".
        let mut empty = InjectResponse {
            text: "<!-- synveda block=b3 records=none -->".to_owned(),
            record_ids: vec!["stale".to_owned()],
            ..block
        };
        empty.hydrate();
        assert_eq!(empty.record_ids, vec!["stale".to_owned()], "left untouched");
    }

    /// The join EVAL-4 grades on (ADR-0047 decision 2). Containment
    /// cannot do this: an index entry carries a truncated head, so
    /// "demoted" and "absent" would be one answer, and they are the two
    /// the whole suite turns on.
    #[test]
    fn a_records_tier_reads_by_position_and_absence_is_its_own_answer() {
        // Built rather than parsed: `record_ids` and `tiers` are no longer
        // on the wire, so a JSON fixture here would be testing a body no
        // gateway sends. The positional join is what this asserts, and
        // Prompt 18 is what has to make the fields real again.
        let block = InjectResponse {
            text: "…".to_owned(),
            block_hash: "b3".to_owned(),
            record_ids: vec!["r1".to_owned(), "r2".to_owned()],
            tiers: vec!["body".to_owned(), "index".to_owned()],
            index_entries: 1,
            index_tokens: 40,
            staleness_permille: vec![1000, 820],
            tokens: 120,
            budget_tokens: 1500,
        };
        assert_eq!(block.tier_of("r1"), Some("body"));
        assert_eq!(block.tier_of("r2"), Some("index"));
        assert_eq!(block.tier_of("r3"), None, "not carried at all");

        // A gateway that sent fewer tiers than records must not default
        // one, or a truncated array would silently become a body and a
        // demotion would read as a whole record.
        let ragged = InjectResponse {
            tiers: vec!["body".to_owned()],
            ..block
        };
        assert_eq!(ragged.tier_of("r1"), Some("body"));
        assert_eq!(ragged.tier_of("r2"), None);
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
