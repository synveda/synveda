//! The recall API (CTX-4, ADR-0041): the read path's deep primitive
//! (seed §3), in the floor CTX-4 needs — `POST /v1/recall` serves the
//! bodies behind the handles an inject block's index tier handed out.
//!
//! The whole security posture of this route is one sentence: **a handle
//! is a name, not a capability** (ADR-0041 decision 5). Nothing is
//! carried over from the block that named a record — no token, no cursor,
//! no signature, because none exists. The route re-runs
//! `composition_plan` exactly as inject does, and serves what the plan
//! admits *now*: an id the caller could read five minutes ago under a
//! role they have since lost is not served, and neither is one whose
//! scope a lapse stopped reaching, whose class a retention schedule has
//! since cut, or whose channel a pack has since closed.
//!
//! It does not decide any of that itself. `retrieval::admit` is the one
//! function that answers "may this caller see this record", and this
//! route calls it with the ids named instead of a sweep — so there is no
//! second admission path for a handle to reach around (seed §2.2).
//!
//! Refusals are uniform and silent (ADR-0041 decision 6): an id that does
//! not exist, belongs to another tenant, sits at a denied scope, sits
//! above the caller's tier, or has passed its horizon all produce the
//! same outcome — the entry is simply absent. A recall must not become an
//! oracle for "does this record exist".
//!
//! CTX-5 (ADR-0042) widened this into the deep query the tech plan §3
//! describes, by *adding* to this surface rather than replacing it:
//!
//! - **A query alternative.** `ids` xor `query` — one route, one audit
//!   action, one admission function. A query embeds through the CTX-3
//!   seam, ranks through the CTX-1 engine, and hands the fused ids to the
//!   same `only` narrowing a handle uses, so ranking can only ever remove.
//! - **A universe wider than the chain.** The candidate scopes are the
//!   ones that could *contribute* to this request — where the named ids
//!   live or are published, or every occupied scope for a query — and
//!   every one of them is an individual per-`(scope, tier)` decision.
//!   This is where `standard`'s department default and a curator's bound
//!   subtree finally become readable, seven ADRs after ADR-0024 parked
//!   them (decision 2).
//! - **As-of, as two explicit instants.** `as_of` is transaction time and
//!   `valid_at` is valid time. The corpus rewinds; the authority never
//!   does — the PDP decides with the caller's current roles, packs and
//!   lapses whatever instant is asked for, because a permission that
//!   outlived its decision is the thing this route was built not to have
//!   (decision 8).
//!
//! Graph traversal is GRPH-3's, feature-flagged and degradable, and it
//! joins at the fused-id list without touching admission (ADR-0042
//! option 12).

use std::time::Instant;

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_ingest::embedding::Embedder;
use synveda_policy::Resource;
use synveda_retrieval::{
    Admitted, CandidateScope, ComposeRequest, LapsedScope, MemoryReadInputs, QueryVector,
    SearchFilter, SearchRequest, admit, composition_plan, hybrid_search,
};
use synveda_store::{rls, search};
use synveda_types::{
    Channel, Error, RecordClass, RecordId, RecordKind, Result, ScopeId, ScopeTier, Sensitivity,
};

use crate::app::AppState;
use crate::audit;
use crate::authz;
use crate::error::ApiError;
use crate::request::{body, tenant_id};
use crate::telemetry::{CONTEXT_RECALLS_TOTAL, RECALL_RECORDS_TOTAL, RECALL_STAGE_SECONDS};

/// How many records one recall may serve — named or found (ADR-0041
/// decision 7, extended to the query form by ADR-0042 decision 1).
///
/// Comfortably above any block's plausible index tier, and far below a
/// corpus: recall is a deep read of specific material, not a bulk export.
/// The cap was never about ids, it was about how much corpus one call may
/// carry, so the query form's `limit` answers to the same number. A blunt
/// instrument compared with a rate limit, which is AUTH-6's.
const MAX_RECALL_IDS: usize = 32;

/// Cap for `session_id`; the inject surface's, for the same reason.
const MAX_SESSION_ID_CHARS: usize = 200;

/// Cap for the query text. Generous — this is the deep surface and a real
/// question is longer than a session-start task — and bounded, because an
/// unbounded string reaches the embedder and the BM25 parser.
const MAX_QUERY_CHARS: usize = 4_000;

/// How deep each retrieval leg goes before admission narrows it.
///
/// Wider than [`MAX_RECALL_IDS`] on purpose: admission removes, so a fused
/// set the size of the answer would return short whenever anything in it
/// turned out to be inadmissible.
const RECALL_PER_LEG: usize = 128;

#[derive(Deserialize)]
pub(crate) struct RecallBody {
    /// The records to serve — the handles an index entry rendered as
    /// `(recall <id>)`. Exclusive with `query`.
    #[serde(default)]
    ids: Option<Vec<RecordId>>,
    /// The question to answer — the deep query (CTX-5). Exclusive with
    /// `ids`: naming records *and* asking a question is two requests, and
    /// the intersection is a third nobody asked for.
    #[serde(default)]
    query: Option<String>,
    /// Transaction time: serve bodies as the database held them at this
    /// instant. Defaults to now, which is the surface CTX-4 shipped.
    #[serde(default)]
    as_of: Option<DateTime<Utc>>,
    /// Valid time: serve the assertions that held *about the world* at
    /// this instant. Defaults to `as_of`, so one `--as-of` is the diagonal
    /// query — "as we knew it then, about then".
    #[serde(default)]
    valid_at: Option<DateTime<Utc>>,
    /// How many records a query may return, capped at
    /// [`MAX_RECALL_IDS`]. Ignored by the ids form, which is bounded by
    /// what it named.
    #[serde(default)]
    limit: Option<usize>,
    /// Opaque harness session identifier, audit-correlated with the
    /// session's injects and observe batches.
    session_id: Option<String>,
}

impl RecallBody {
    fn mode(&self) -> &'static str {
        match (&self.ids, &self.query) {
            (_, Some(_)) => "query",
            (Some(_), _) => "ids",
            // Neither, which validation only admits alongside an instant.
            _ => "sweep",
        }
    }
}

#[derive(Serialize)]
struct RecallResponse {
    /// The records the *current* plan admits — in gradient order for the
    /// ids form, so a recall reads like the block its handles came from,
    /// and in relevance order for a query, because the best match belongs
    /// first or `limit` truncates the wrong end (ADR-0042 decision 13).
    ///
    /// Records the plan does not admit are absent, without distinction: an
    /// id that never existed and one the caller may not read read the same
    /// from here (ADR-0041 decision 6).
    entries: Vec<RecallEntry>,
    /// Which shape was asked (ADR-0042 decision 1).
    mode: &'static str,
    /// How many records were asked for — ids named, or a query's `limit`.
    /// The caller can already count the difference, so stating it leaks
    /// nothing, and it makes "I asked for six and got four" a fact the
    /// response carries rather than one the caller has to notice.
    requested: usize,
    /// The transaction-time instant: what the database held when.
    as_of: DateTime<Utc>,
    /// The valid-time instant: what the assertions were about.
    valid_at: DateTime<Utc>,
    /// How many scopes could have contributed, before the cap.
    scopes_considered: usize,
    /// How many were actually decided.
    scopes_decided: usize,
    /// Whether the scope cap dropped candidates (ADR-0042 decision 5).
    /// A bounded answer presented as a complete one is the one failure
    /// this surface cannot afford, so this is never inferred from a count.
    truncated: bool,
    /// Legs that were unavailable, mirrored in `X-Synveda-Degraded`.
    /// Only the embedder degrades here: a query is still a ranked answer
    /// over the same corpus without its dense leg.
    degraded: Vec<&'static str>,
}

/// One served record: the body, and the labels that let an agent weigh it
/// (tech plan §3 — "results carry provenance + channel labels so the
/// agent can weigh derived vs published").
#[derive(Serialize)]
struct RecallEntry {
    record_id: RecordId,
    /// The scope that admitted it — since FLOW-5 not always where it
    /// lives, and always the one whose decision covered this caller.
    scope_id: ScopeId,
    /// Published or derived at that scope: the trust label.
    channel: Channel,
    /// Authored/canonical or pipeline-derived (seed §4.2).
    kind: RecordKind,
    class: RecordClass,
    sensitivity: Sensitivity,
    /// The full content. Recall does not truncate — the caller named this
    /// record, which is what makes this the deep surface (ADR-0041
    /// decision 7).
    content: String,
    /// Source session, extraction method, model version, confidence
    /// (seed §4.2) — the provenance half of the label set.
    provenance: serde_json::Value,
    /// The valid window, so an agent can tell a current fact from one
    /// that is on its way out.
    valid_from: DateTime<Utc>,
    valid_to: Option<DateTime<Utc>>,
    /// The VedaFlow object address of exactly the version served —
    /// the same watermark an inject entry carries, so a recall is as
    /// recomputable as a block (ADR-0031 decision 11).
    object_hash: String,
    /// Freshness at `as_of`, per mille (MEM-6, ADR-0040 decision 12).
    staleness_permille: u16,
}

fn validate(payload: &RecallBody) -> Result<()> {
    let invalid = |message: String| Err(Error::Invalid { message });
    match (&payload.ids, &payload.query) {
        (Some(_), Some(_)) => {
            return invalid("recall takes ids or query, never both".to_owned());
        }
        (None, None) => {
            // A bare recall must say what it wants. An instant is a
            // perfectly good answer to that — "everything I may read, as
            // it stood then" is the complete historical read, and the one
            // shape a query cannot give (ADR-0042 decision 14).
            if payload.as_of.is_none() {
                return invalid("recall takes ids, a query, or an as_of instant".to_owned());
            }
        }
        (Some(ids), None) => {
            if ids.is_empty() || ids.len() > MAX_RECALL_IDS {
                return invalid(format!("ids must name 1..={MAX_RECALL_IDS} records"));
            }
        }
        (None, Some(query)) => {
            let chars = query.chars().count();
            if chars == 0 || chars > MAX_QUERY_CHARS {
                return invalid(format!("query must be 1..={MAX_QUERY_CHARS} characters"));
            }
        }
    }
    if let Some(limit) = payload.limit
        && (limit == 0 || limit > MAX_RECALL_IDS)
    {
        return invalid(format!("limit must be 1..={MAX_RECALL_IDS}"));
    }
    if let Some(session_id) = &payload.session_id {
        let chars = session_id.chars().count();
        if chars == 0 || chars > MAX_SESSION_ID_CHARS {
            return invalid(format!(
                "session_id must be 1..={MAX_SESSION_ID_CHARS} characters"
            ));
        }
    }
    Ok(())
}

/// `POST /v1/recall` — bodies behind named handles (ADR-0041), or the
/// answer to a question (ADR-0042).
pub(crate) async fn create(
    State(state): State<AppState>,
    payload: std::result::Result<Json<RecallBody>, JsonRejection>,
) -> Response {
    match handle(&state, payload).await {
        Ok(response) => {
            let outcome = if !response.degraded.is_empty() {
                "degraded"
            } else if response.entries.is_empty() {
                "empty"
            } else {
                "ok"
            };
            metrics::counter!(
                CONTEXT_RECALLS_TOTAL,
                "outcome" => outcome,
                "mode" => response.mode,
            )
            .increment(1);
            let header = response
                .degraded
                .join(",")
                .parse::<axum::http::HeaderValue>()
                .ok()
                .filter(|_| !response.degraded.is_empty());
            let mut rendered = Json(response).into_response();
            if let Some(value) = header {
                rendered.headers_mut().insert("x-synveda-degraded", value);
            }
            rendered
        }
        Err(error) => {
            let outcome = match &error {
                Error::Unauthenticated { .. }
                | Error::PolicyDenied { .. }
                | Error::NotFound { .. }
                | Error::Invalid { .. }
                | Error::Conflict { .. }
                | Error::RateLimited { .. } => "rejected",
                _ => "error",
            };
            metrics::counter!(CONTEXT_RECALLS_TOTAL, "outcome" => outcome).increment(1);
            audit::record_rejection(&state, "recall", &error).await;
            ApiError(error).into_response()
        }
    }
}

#[tracing::instrument(
    name = "gateway.recall",
    skip_all,
    fields(
        mode = tracing::field::Empty,
        scopes.permitted = tracing::field::Empty,
        scopes.decided = tracing::field::Empty,
        records.requested = tracing::field::Empty,
        records.served = tracing::field::Empty,
        degraded = tracing::field::Empty,
    ),
    err(Display)
)]
async fn handle(
    state: &AppState,
    payload: std::result::Result<Json<RecallBody>, JsonRejection>,
) -> Result<RecallResponse> {
    let payload = body(payload)?;
    validate(&payload)?;
    let tenant_id = tenant_id()?;
    let mode = payload.mode();
    let limit = payload.limit.unwrap_or(MAX_RECALL_IDS);
    let requested = payload.ids.as_ref().map_or(limit, Vec::len);
    let span = tracing::Span::current();
    span.record("mode", mode);
    span.record("records.requested", requested);
    metrics::counter!(RECALL_RECORDS_TOTAL, "side" => "requested").increment(requested as u64);

    // The two instants (ADR-0042 decision 7). Stamped once at the seam
    // exactly as inject stamps its one (ADR-0026 decision 6), so
    // everything below is deterministic given them — and `valid_at`
    // defaults to `as_of` rather than to now, which is what makes a single
    // `--as-of` the diagonal query the demo asks for.
    let now = Utc::now();
    let as_of = payload.as_of.unwrap_or(now);
    let valid_at = payload.valid_at.or(payload.as_of).unwrap_or(now);
    // Only a real rewind switches the read to `records_versions`; asking
    // for "now" keeps the present-tense queries the hot path uses.
    let tx_at = payload.as_of.filter(|at| *at < now);

    // Transaction 1: the decision inputs, the candidate universe, and the
    // rows deciding it needs. Read-only, and dropped before the embed call
    // — no transaction spans a network call (the MEM-3 rule).
    let stage = Instant::now();
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let input = authz::gather_at_home(state, &mut tx).await?;
    let lapsed_chains = authz::gather_lapsed(state, &mut tx, &input).await?;

    // The universe: the scopes that could contribute to *this* request
    // (ADR-0042 decision 2). Naming ids bounds it to where those records
    // live or are published — which is why CTX-4's navigation path keeps
    // costing what it cost — and a query takes every occupied scope.
    // Both are unions of a residence read and a channel read, because a
    // published tree may name a record living below it (ADR-0034
    // decision 6) and residence alone would miss it.
    let occupied = match &payload.ids {
        Some(ids) => {
            let mut scopes = search::scopes_holding(&mut tx, tenant_id, ids).await?;
            scopes.extend(
                synveda_vedaflow::scopes_naming(
                    &mut tx,
                    tenant_id,
                    ids,
                    synveda_vedaflow::ChannelRef::memory(Channel::Published),
                )
                .await?,
            );
            scopes
        }
        None => {
            let mut scopes = search::occupied_scopes(&mut tx, tenant_id).await?;
            scopes.extend(
                synveda_vedaflow::scopes_with_channel(
                    &mut tx,
                    tenant_id,
                    synveda_vedaflow::ChannelRef::memory(Channel::Published),
                )
                .await?,
            );
            scopes
        }
    };
    let universe = authz::gather_universe(state, &mut tx, &input, &occupied).await?;
    drop(tx);

    let lapsed: Vec<LapsedScope<'_>> = lapsed_chains
        .iter()
        .map(|resolved| LapsedScope {
            lapse: &resolved.lapse,
            chain: &resolved.chain,
            assignments: &resolved.assignments,
        })
        .collect();
    let candidates: Vec<CandidateScope<'_>> = universe
        .candidates
        .iter()
        .map(|resolved| CandidateScope {
            scope_id: resolved.scope_id,
            chain: &resolved.chain,
            assignments: &resolved.assignments,
        })
        .collect();
    // One walk, one answer to "may this caller see this record" — the
    // chain, the lapse targets and the widened candidates all decided by
    // the same per-`(scope, tier)` body (ADR-0042 decision 3).
    let plan = composition_plan(
        &state.pdp,
        &MemoryReadInputs {
            principal: &input.principal,
            chain: &input.chain,
            anchors: input.anchors.as_slice(),
            groups: &input.groups,
            assignments: &input.assignments,
            default_pack: input.default_pack.as_deref(),
            // The anchors carry every grant this caller holds — a grant at
            // a candidate scope is precisely the grant the widened
            // universe exists to reach, and deciding that scope without it
            // would ask the question and get the wrong answer.
            lapses: &input.lapses,
            lapsed: &lapsed,
            candidates: &candidates,
        },
    )?;
    let plan_elapsed = stage.elapsed();
    metrics::histogram!(RECALL_STAGE_SECONDS, "stage" => "plan").record(plan_elapsed.as_secs_f64());
    span.record("scopes.permitted", plan.scopes.len());
    span.record("scopes.decided", plan.decisions.len());
    tracing::debug!(
        plan_ms = plan_elapsed.as_millis() as u64,
        considered = universe.considered,
        decided = plan.decisions.len(),
        truncated = universe.truncated,
        "recall plan decided"
    );

    let mut degraded: Vec<&'static str> = Vec::new();

    // The embed call: the MEM-4 seam, outside any transaction, under the
    // read-path deadline (ADR-0026 decision 3). Skipped for an empty plan,
    // which answers nothing regardless.
    let query = payload
        .query
        .as_ref()
        .filter(|_| !plan.scopes.is_empty())
        .cloned();
    let vector = match &query {
        Some(text) => {
            let stage = Instant::now();
            let embedded = tokio::time::timeout(
                state.inject_embed_timeout,
                state.embedder.embed(std::slice::from_ref(text)),
            )
            .await;
            metrics::histogram!(RECALL_STAGE_SECONDS, "stage" => "embed")
                .record(stage.elapsed().as_secs_f64());
            match embedded {
                Ok(Ok(mut vectors)) if !vectors.is_empty() => Some(QueryVector {
                    model: state.embedder.model().to_owned(),
                    vector: vectors.remove(0),
                }),
                Ok(Ok(_)) => {
                    tracing::warn!("embedder returned no vector; degrading to sparse-only");
                    degraded.push("embedder");
                    None
                }
                Ok(Err(error)) => {
                    tracing::warn!(error = %error, "query embed failed; degrading to sparse-only");
                    degraded.push("embedder");
                    None
                }
                Err(_) => {
                    tracing::warn!(
                        deadline_ms = state.inject_embed_timeout.as_millis() as u64,
                        "query embed deadline expired; degrading to sparse-only"
                    );
                    degraded.push("embedder");
                    None
                }
            }
        }
        None => None,
    };

    // Transaction 2: search, admit, chain the event, commit.
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let ranked: Option<Vec<RecordId>> = match &query {
        Some(text) => {
            let request = SearchRequest {
                query: text.clone(),
                vector,
                filter: SearchFilter {
                    tiers: plan
                        .scopes
                        .iter()
                        .flat_map(|scope| ScopeTier::expand(scope.scope_id, &scope.sensitivities))
                        .collect(),
                },
                limit: RECALL_PER_LEG,
                per_leg: RECALL_PER_LEG,
                // Valid time, so ranking and admission agree about which
                // facts are current (MEM-5, ADR-0039 decision 11). The
                // indexes hold current truth by construction, so this is
                // also the honest limit of a ranked as-of query
                // (ADR-0042 decision 14).
                at: valid_at,
            };
            let stage = Instant::now();
            let searched = hybrid_search(&mut tx, &state.search_index, tenant_id, &request).await;
            metrics::histogram!(RECALL_STAGE_SECONDS, "stage" => "search")
                .record(stage.elapsed().as_secs_f64());
            // Not a degradation: a deep query with no retrieval is a
            // different answer, not a partial one. Inject degrades because
            // its caller cannot see the error; recall reports because its
            // caller asked (ADR-0042 decision 12).
            Some(
                searched?
                    .into_iter()
                    .map(|retrieved| retrieved.record.id)
                    .collect(),
            )
        }
        None => None,
    };

    // Whichever shape asked, admission runs over *named* ids: a handle
    // names them and a query ranks them, and `only` can only ever remove
    // (ADR-0041 decision 5). There is no sweeping recall.
    let named: Vec<RecordId> = match (&payload.ids, &ranked) {
        (Some(ids), _) => ids.clone(),
        (None, Some(ids)) => ids.clone(),
        // A query over an empty plan, or one whose embed and search were
        // both skipped: nothing to admit, and the empty answer is a
        // result rather than an error (ADR-0026 decision 1).
        (None, None) => Vec::new(),
    };
    let mut request = if mode == "sweep" {
        // Nothing named and nothing asked: the plan itself, as it stood at
        // the instant. The only shape that reaches a record the live
        // corpus no longer holds (ADR-0042 decision 14).
        ComposeRequest::sweeping(plan.scopes, valid_at)
    } else {
        ComposeRequest::naming(plan.scopes, named, valid_at)
    };
    if let Some(at) = tx_at {
        request = request.as_of(at);
    }
    // Relevance orders derived material within a scope; `only` has already
    // bounded the set, so this changes rank, never membership.
    request.relevance = ranked.clone();

    let stage = Instant::now();
    // The one admission function. A quarantined caller, an unplaced
    // identity, or a plan every scope denied admits nothing — the empty
    // answer, never an error, because a policy outcome on a read is a
    // result (ADR-0026 decision 1).
    let admission = admit(&mut tx, tenant_id, &request).await?;
    metrics::histogram!(RECALL_STAGE_SECONDS, "stage" => "admit")
        .record(stage.elapsed().as_secs_f64());
    tracing::debug!(
        admitted = admission.records.len(),
        "recall admission decided"
    );

    let mut records = admission.records;
    match &ranked {
        // Relevance order: the best match belongs first, or `limit`
        // truncates the wrong end (ADR-0042 decision 13).
        Some(ids) => {
            let rank: std::collections::HashMap<RecordId, usize> = ids
                .iter()
                .enumerate()
                .map(|(rank, id)| (*id, rank))
                .collect();
            records.sort_by(|a, b| {
                rank.get(&a.version.id)
                    .unwrap_or(&usize::MAX)
                    .cmp(rank.get(&b.version.id).unwrap_or(&usize::MAX))
                    .then_with(|| a.version.id.cmp(&b.version.id))
            });
            records.truncate(limit);
        }
        // Gradient order, so a recall reads like the block it came from.
        None => {
            records.sort_by(|a, b| {
                a.position
                    .cmp(&b.position)
                    .then_with(|| channel_rank(a).cmp(&channel_rank(b)))
                    .then_with(|| b.version.state.valid_from.cmp(&a.version.state.valid_from))
                    .then_with(|| a.version.id.cmp(&b.version.id))
            });
            // A sweep is bounded by `limit` like a query: it named
            // nothing, so nothing else bounds it. The ids form is already
            // bounded by what it named.
            if mode == "sweep" {
                records.truncate(limit);
            }
        }
    }
    let entries: Vec<RecallEntry> = records.iter().map(render).collect();

    span.record("records.served", entries.len());
    metrics::counter!(RECALL_RECORDS_TOTAL, "side" => "served").increment(entries.len() as u64);

    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::ContextRecalled,
        match input.identity.as_ref() {
            Some(identity) => Resource::Scope(identity.scope_id).to_string(),
            None => "scope none".to_owned(),
        },
        Outcome::Success,
        json!({
            "session_id": payload.session_id,
            "mode": mode,
            "as_of": as_of,
            "valid_at": valid_at,
            // The question as a hash, never its text — the ADR-0021
            // discipline that already governs inject's `task_hash`.
            "query_hash": payload.query.as_ref().map(|text| {
                blake3::hash(text.as_bytes()).to_hex().to_string()
            }),
            // What was asked for and what was served, as counts. Never the
            // refused ids: the surface answers uniformly for those, and a
            // payload enumerating them would put the oracle back on the
            // chain (ADR-0041 decision 8).
            "requested": requested,
            "served": entries.len(),
            // What the widened universe cost and whether it was complete
            // (ADR-0042 decision 5) — a truncation the chain does not
            // record is a truncation nobody can audit.
            "scopes_considered": universe.considered,
            "scopes_decided": plan.decisions.len(),
            "truncated": universe.truncated,
            "degraded": degraded,
            // The same per-entry watermark an inject event carries, so
            // "what did that agent actually read in March" is one question
            // over one shape.
            "entries": entries.iter().map(|entry| json!({
                "record_id": entry.record_id,
                "object_hash": entry.object_hash,
                "channel": entry.channel,
                "scope_id": entry.scope_id,
                "staleness_permille": entry.staleness_permille,
            })).collect::<Vec<_>>(),
            "decisions": plan.decisions.iter().map(|decision| json!({
                "scope_id": decision.scope_id,
                "allowed": decision.allowed,
                "sensitivities": decision.sensitivities
                    .iter()
                    .map(|tier| tier.as_str())
                    .collect::<Vec<_>>(),
                "pack": format!("{}@{}", decision.pack_name, decision.pack_version),
                "lapse_id": decision.lapse,
            })).collect::<Vec<_>>(),
        }),
    )
    .await?;
    tx.commit().await.map_err(|err| Error::Storage {
        message: format!("commit recall transaction: {err}"),
    })?;

    span.record("degraded", degraded.join(","));
    Ok(RecallResponse {
        entries,
        mode,
        requested,
        as_of,
        valid_at,
        scopes_considered: universe.considered,
        scopes_decided: plan.decisions.len(),
        truncated: universe.truncated,
        degraded,
    })
}

fn channel_rank(record: &Admitted) -> u8 {
    match record.channel {
        Channel::Published => 0,
        _ => 1,
    }
}

fn render(record: &Admitted) -> RecallEntry {
    let state = &record.version.state;
    RecallEntry {
        record_id: record.version.id,
        scope_id: record.scope_id,
        channel: record.channel,
        kind: state.kind,
        class: state.class,
        sensitivity: state.sensitivity,
        content: state.content.clone(),
        provenance: state.provenance.clone(),
        valid_from: state.valid_from,
        valid_to: state.valid_to,
        object_hash: synveda_vedaflow::MemoryAsset {
            id: record.version.id,
            scope_id: state.scope_id,
            owner_id: state.owner_id,
            kind: state.kind,
            class: state.class,
            content: state.content.clone(),
            sensitivity: state.sensitivity,
            valid_from: state.valid_from,
            valid_to: state.valid_to,
        }
        .address()
        .to_hex(),
        staleness_permille: permille(record.staleness),
    }
}

/// The staleness score as an integer per mille — the inject response's
/// encoding, for the reason ADR-0019 decision 2 gives: a float is a number
/// nobody can compare later.
fn permille(staleness: f64) -> u16 {
    let scaled = (staleness * 1000.0).round();
    if scaled <= 0.0 {
        0
    } else if scaled >= 1000.0 {
        1000
    } else {
        scaled as u16
    }
}
