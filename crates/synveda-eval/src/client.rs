//! The `/v1` client (EVAL-1, ADR-0028 decision 1).
//!
//! Four endpoints, an actor's own bearer, and no other way in. The wire
//! structs are declared here rather than imported because this crate
//! depends on no Synveda crate at all — the same price the TypeScript
//! adapter pays, for the same reason: what an outside caller can see is
//! exactly what an eval should measure.
//!
//! EVAL-2 added the last two (ADR-0046 decisions 1 and 4): the recall
//! sweep, which is how a caller enumerates what it may read, and the
//! audit search, which is how an auditor sees what the pipeline
//! committed. They answer different questions and the extraction
//! measurement needs both.

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

/// `POST /v1/inject` (CTX-3, ADR-0026).
#[derive(Debug, Serialize)]
pub struct InjectRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<&'a str>,
    pub session_id: &'a str,
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
#[derive(Debug, Deserialize)]
pub struct InjectResponse {
    pub text: String,
    /// The watermark (ADR-0025 decision 7). It rides into the report so a
    /// measurement can be traced back to exactly the block that produced
    /// it, months later, from the audit chain.
    pub block_hash: String,
    pub record_ids: Vec<String>,
    /// How much of each composed record the block carried, in block order:
    /// `body` or `index` (CTX-4, ADR-0041 decision 9). Parallel to
    /// `record_ids`, which is what makes a per-record tier readable.
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

/// `POST /v1/observe` (MEM-1, ADR-0020).
#[derive(Debug, Serialize)]
pub struct ObserveRequest<'a> {
    pub session_id: &'a str,
    pub events: Vec<ObserveEvent<'a>>,
}

#[derive(Debug, Serialize)]
pub struct ObserveEvent<'a> {
    pub idempotency_key: String,
    pub kind: &'a str,
    pub payload: serde_json::Value,
    pub occurred_at: String,
}

#[derive(Debug, Deserialize)]
pub struct ObserveResponse {
    pub accepted: usize,
    pub duplicates: usize,
    pub quarantined: usize,
    pub denied: usize,
    /// Per-event outcomes, which is what makes the extraction measurement
    /// attributable: the buffered `event_id` acked here is the same id the
    /// served record's `provenance` carries and the same id the
    /// `memory.extracted` payload names, so one key joins the seed, the
    /// sweep, and the chain.
    #[serde(default)]
    pub events: Vec<ObserveEventOutcome>,
}

#[derive(Debug, Deserialize)]
pub struct ObserveEventOutcome {
    pub idempotency_key: String,
    /// Absent for a denied event: nothing was persisted for it.
    pub event_id: Option<String>,
}

/// `POST /v1/recall` in its **sweep** shape (CTX-5, ADR-0042 decision 14):
/// no ids and no query, just an instant — "everything I may read, as it
/// stood then". The one shape that enumerates a corpus rather than ranking
/// one (ADR-0046 decision 1).
#[derive(Debug, Serialize)]
pub struct RecallSweepRequest<'a> {
    pub as_of: &'a str,
    pub session_id: &'a str,
    /// Asked for explicitly rather than left to the surface's default, so
    /// "I asked for N and got N" is a fact this caller can state without
    /// knowing the product's cap (ADR-0046 decision 3).
    pub limit: usize,
}

/// `POST /v1/recall` in its **query** shape (CTX-5, ADR-0042 decision 1):
/// ranked retrieval over the widened universe, with no composition budget
/// and no scope gradient in the way.
///
/// EVAL-4 uses it for one thing only — asking whether a record is
/// retrievable at all, which is a different question from whether it fits
/// in a block, and the only honest way to wait for the sparse sidecar
/// without waiting for the measurement (ADR-0047 decision 5).
#[derive(Debug, Serialize)]
pub struct RecallQueryRequest<'a> {
    pub query: &'a str,
    pub session_id: &'a str,
    pub limit: usize,
}

/// `POST /v1/recall` in its **ids** shape (CTX-4, ADR-0041): the handles an
/// index line rendered, re-decided by the current plan on the way in.
///
/// EVAL-5's sharpest probe (ADR-0048 decision 1). It removes retrieval
/// from the question entirely — no ranking, no index, no phrasing — and
/// asks the product to refuse a record by name. Refusals are uniform and
/// silent (ADR-0041), so a request naming ten inadmissible ids answers
/// with nothing rather than with an error, and "nothing" is exactly the
/// measurement.
#[derive(Debug, Serialize)]
pub struct RecallIdsRequest<'a> {
    pub ids: Vec<String>,
    pub session_id: &'a str,
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

/// What `POST /v1/proposals/{id}/classify` reports back (AUTHZ-5,
/// ADR-0038 decision 9).
#[derive(Debug, Deserialize)]
pub struct Classified {
    pub scope_id: String,
    pub sensitivity: String,
    pub records: Vec<ClassifiedRecord>,
}

#[derive(Debug, Deserialize)]
pub struct ClassifiedRecord {
    pub record_id: String,
    /// The tier it left, so a report can say what a reclassification cost
    /// rather than only what it installed.
    pub was: String,
}

#[derive(Debug, Deserialize)]
pub struct Proposal {
    pub id: String,
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

    pub async fn inject(
        &self,
        bearer: &str,
        request: &InjectRequest<'_>,
    ) -> Result<Timed<InjectResponse>, String> {
        self.post("/v1/inject", bearer, request).await
    }

    pub async fn observe(
        &self,
        bearer: &str,
        request: &ObserveRequest<'_>,
    ) -> Result<Timed<ObserveResponse>, String> {
        self.post("/v1/observe", bearer, request).await
    }

    pub async fn recall_sweep(
        &self,
        bearer: &str,
        request: &RecallSweepRequest<'_>,
    ) -> Result<Timed<RecallResponse>, String> {
        self.post("/v1/recall", bearer, request).await
    }

    pub async fn recall_query(
        &self,
        bearer: &str,
        request: &RecallQueryRequest<'_>,
    ) -> Result<Timed<RecallResponse>, String> {
        self.post("/v1/recall", bearer, request).await
    }

    pub async fn recall_ids(
        &self,
        bearer: &str,
        request: &RecallIdsRequest<'_>,
    ) -> Result<Timed<RecallResponse>, String> {
        self.post("/v1/recall", bearer, request).await
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
        self.post(
            &format!("/v1/proposals/{proposal}/approve"),
            bearer,
            &serde_json::json!({}),
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

    /// Runs an approved classification. The **author's** call, not a
    /// reviewer's, and that is forced rather than chosen (ADR-0048
    /// decision 7): `MemoryClassify` is permitted role-free at
    /// `principal.home`, the effect asks a `MemoryRead` at the working
    /// tier at the same scope, and the privacy floor closes another
    /// principal's personal leaf to every content role — so the one
    /// identity that can run this is the one whose leaf it is.
    pub async fn classify(
        &self,
        bearer: &str,
        proposal: &str,
    ) -> Result<Timed<Classified>, String> {
        self.post(
            &format!("/v1/proposals/{proposal}/classify"),
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
        let started = Instant::now();
        let response = self
            .http
            .post(format!("{}{path}", self.gateway_url))
            .bearer_auth(bearer)
            .header(
                "x-synveda-client",
                concat!("synveda-eval/", env!("CARGO_PKG_VERSION")),
            )
            .json(body)
            .send()
            .await
            .map_err(|err| format!("POST {path}: {err}"))?;
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
            return Err(format!("POST {path} returned {status}: {}", detail(&raw)));
        }
        let value = serde_json::from_str(&raw)
            .map_err(|err| format!("POST {path} returned an unreadable body: {err}"))?;
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
    fn an_inject_request_omits_what_it_does_not_set() {
        // A `null` task is not the same request as no task: the taskless
        // branch is chosen by absence (ADR-0026 decision 3).
        let request = InjectRequest {
            task: None,
            session_id: "s1",
            budget_tokens: None,
        };
        let json = serde_json::to_string(&request).expect("serialises");
        assert_eq!(json, r#"{"session_id":"s1"}"#);
    }

    /// The join EVAL-4 grades on (ADR-0047 decision 2). Containment
    /// cannot do this: an index entry carries a truncated head, so
    /// "demoted" and "absent" would be one answer, and they are the two
    /// the whole suite turns on.
    #[test]
    fn a_records_tier_reads_by_position_and_absence_is_its_own_answer() {
        let block: InjectResponse = serde_json::from_str(
            r#"{"text":"…","block_hash":"b3","record_ids":["r1","r2"],
                "tiers":["body","index"],"index_entries":1,"index_tokens":40,
                "staleness_permille":[1000,820],"tokens":120,"budget_tokens":1500}"#,
        )
        .expect("parses");
        assert_eq!(block.tier_of("r1"), Some("body"));
        assert_eq!(block.tier_of("r2"), Some("index"));
        assert_eq!(block.tier_of("r3"), None, "not carried at all");

        // A gateway that sent fewer tiers than records must not default
        // one, or a truncated array would silently become a body and a
        // demotion would read as a whole record.
        let ragged: InjectResponse = serde_json::from_str(
            r#"{"text":"…","block_hash":"b3","record_ids":["r1","r2"],"tiers":["body"],
                "tokens":10,"budget_tokens":100}"#,
        )
        .expect("parses");
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
