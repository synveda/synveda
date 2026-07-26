//! The inject API (CTX-3, ADR-0026): the read path's session-start
//! primitive (seed §3). `POST /v1/inject` returns a token-budgeted,
//! watermarked context block for the caller's identity — the CTX-2
//! product path (identity → HIER-2 chain → PDP composition plan →
//! compose) with the CTX-1 hybrid engine wired between plan and
//! compose when the caller supplies a task.
//!
//! The response is 200 with a (possibly empty) block whenever authn and
//! tenant resolution succeed: an empty plan — quarantined caller,
//! unplaced identity, every chain scope denied — composes the empty
//! block. Policy outcomes are results, not errors, and the surface
//! leaks nothing about why a block is thin; the audit event records the
//! real reason (ADR-0026 decision 1).
//!
//! Dependency failures degrade instead of failing (the AC's posture):
//! an embed error or deadline drops the dense leg (sparse-only, still
//! ranked); a retrieval error drops ranking entirely (pinned material
//! plus recency-ordered derived still compose). Degradations ride the
//! `X-Synveda-Degraded` header and the body's `degraded` array. Only a
//! store failure is an honest 5xx — there is no partial block without
//! Postgres (ADR-0026 decision 4).
//!
//! Two tenant transactions bracket the embed call — no transaction
//! spans a network call (the MEM-3 rule): the first gathers decision
//! inputs, the second searches, composes, and chains the one
//! `context.injected` audit event with the block's watermark and the
//! per-scope decisions aggregated (ADR-0019 decision 4) before commit.

use std::time::Instant;

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::HeaderValue;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_ingest::embedding::Embedder as _;
use synveda_policy::Resource;
use synveda_retrieval::{
    ComposeRequest, ComposedBlock, LapsedScope, MemoryReadInputs, QueryVector, SearchFilter,
    SearchRequest, compose, composition_plan, hybrid_search,
};
use synveda_store::rls;
use synveda_types::{Error, RecordId, Result, ScopeId};

use crate::app::AppState;
use crate::audit;
use crate::authz;
use crate::error::ApiError;
use crate::hierarchy::{body, tenant_id};
use crate::telemetry::{CONTEXT_INJECTS_TOTAL, INJECT_STAGE_SECONDS};

/// Task cap: a task is a query, not a document (CTX-6 owns session
/// compression). Mirrors the observe discipline of bounded text inputs.
const MAX_TASK_CHARS: usize = 4096;

/// Cap for `session_id`; mirrors the observe staging table's CHECK.
const MAX_SESSION_ID_CHARS: usize = 200;

/// Ranked ids handed to compose: matches compose's per-(scope, kind)
/// candidate cap (ADR-0025 decision 5) so ranking never starves it.
const RELEVANCE_LIMIT: usize = 64;

#[derive(Deserialize)]
pub(crate) struct InjectBody {
    /// What the session is about — the retrieval query. Absent means a
    /// taskless session start: no retrieval leg, recency-ordered derived
    /// (ADR-0025 decision 5's else-branch; not a degradation).
    task: Option<String>,
    /// Opaque harness session identifier, audit-correlated with the
    /// session's observe batches.
    session_id: Option<String>,
    /// A caller-side budget for this call (pre-compact room). Narrows
    /// only: the effective budget is `min(pack budget, this)`
    /// (ADR-0026 decision 7).
    budget_tokens: Option<u32>,
    /// A caller-side sensitivity ceiling. Narrows only, exactly as the
    /// budget does (AUTHZ-5, ADR-0038 decision 12): the plan is the PDP's
    /// answer, and this can only take tiers out of it.
    ///
    /// An agent that knows it is about to paste into a pull request asks
    /// for `internal` and gets a block it can be careless with.
    max_sensitivity: Option<synveda_types::Sensitivity>,
}

#[derive(Serialize)]
struct InjectResponse {
    /// The rendered block, watermark line included (CTX-2, ADR-0025).
    text: String,
    /// BLAKE3 over the ordered entry hashes — the block's identity.
    block_hash: String,
    /// Every composed record, in block order.
    record_ids: Vec<RecordId>,
    /// Estimated tokens of `text`; never exceeds `budget_tokens`.
    tokens: u32,
    /// The effective budget the block was composed under.
    budget_tokens: u32,
    /// The valid-time instant the block was composed at: same instant +
    /// same database state re-composes byte-identically (the CTX-2 AC).
    as_of: DateTime<Utc>,
    /// The commit each planned scope's published channel served, in scope
    /// order — tech plan §2.5's "inject responses cite commit hashes",
    /// paid for out of the response rather than the token budget.
    ///
    /// Present since FLOW-7, because a pinned scope serves an older
    /// commit deliberately and a caller that cannot see which commit it
    /// got cannot tell (ADR-0036 decision 10). It was already in the
    /// audit event; a fact only an auditor can reach is not a citation.
    channels: Vec<ChannelView>,
    /// Degradations applied, worst-first (`embedder`, `retrieval`).
    /// Empty on the full path; also the `X-Synveda-Degraded` header.
    degraded: Vec<&'static str>,
}

fn validate(payload: &InjectBody) -> Result<()> {
    let invalid = |message: String| Err(Error::Invalid { message });
    if let Some(task) = &payload.task {
        let chars = task.chars().count();
        if chars == 0 || chars > MAX_TASK_CHARS {
            return invalid(format!("task must be 1..={MAX_TASK_CHARS} characters"));
        }
    }
    if let Some(session_id) = &payload.session_id {
        let chars = session_id.chars().count();
        if chars == 0 || chars > MAX_SESSION_ID_CHARS {
            return invalid(format!(
                "session_id must be 1..={MAX_SESSION_ID_CHARS} characters"
            ));
        }
    }
    if payload.budget_tokens == Some(0) {
        return invalid("budget_tokens must be at least 1".to_owned());
    }
    Ok(())
}

/// `POST /v1/inject` — compose the caller's context block (ADR-0026).
pub(crate) async fn create(
    State(state): State<AppState>,
    payload: std::result::Result<Json<InjectBody>, JsonRejection>,
) -> Response {
    let result = handle(&state, payload).await;
    match result {
        Ok(response) => {
            let outcome = if !response.degraded.is_empty() {
                "degraded"
            } else if response.record_ids.is_empty() {
                "empty"
            } else {
                "ok"
            };
            metrics::counter!(CONTEXT_INJECTS_TOTAL, "outcome" => outcome).increment(1);
            let header = response
                .degraded
                .join(",")
                .parse::<HeaderValue>()
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
            metrics::counter!(CONTEXT_INJECTS_TOTAL, "outcome" => outcome).increment(1);
            audit::record_rejection(&state, "inject", &error).await;
            ApiError(error).into_response()
        }
    }
}

#[tracing::instrument(
    name = "gateway.inject",
    skip_all,
    fields(
        scopes.permitted = tracing::field::Empty,
        block.entries = tracing::field::Empty,
        block.tokens = tracing::field::Empty,
        degraded = tracing::field::Empty,
    ),
    err(Display)
)]
async fn handle(
    state: &AppState,
    payload: std::result::Result<Json<InjectBody>, JsonRejection>,
) -> Result<InjectResponse> {
    let payload = body(payload)?;
    validate(&payload)?;
    let tenant_id = tenant_id()?;

    // Transaction 1: the per-request decision inputs (identity,
    // assignments, bindings — ADR-0016 decision 6 keeps them
    // per-request), the chain from the HIER-2 cache. Read-only;
    // dropped (rolled back) before any network call.
    let stage = Instant::now();
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let input = authz::gather_at_home(state, &mut tx).await?;
    // The scopes a standing lapse reaches, each with its own chain and
    // assignments — read in the same transaction, because the effective
    // pack is a property of the resource (AUTHZ-4, ADR-0037 decision 10).
    // Empty for every caller holding no grant, which is almost all of them.
    let lapsed_chains = authz::gather_lapsed(state, &mut tx, &input).await?;
    drop(tx);
    let lapsed: Vec<LapsedScope<'_>> = lapsed_chains
        .iter()
        .map(|resolved| LapsedScope {
            lapse: &resolved.lapse,
            chain: &resolved.chain,
            assignments: &resolved.assignments,
        })
        .collect();
    let plan = composition_plan(
        &state.pdp,
        &MemoryReadInputs {
            principal: &input.principal,
            chain: &input.chain,
            assignments: &input.assignments,
            default_pack: input.default_pack.as_deref(),
            role_bindings: &input.role_bindings,
            lapses: &input.lapses,
            lapsed: &lapsed,
        },
    )?;
    metrics::histogram!(INJECT_STAGE_SECONDS, "stage" => "plan")
        .record(stage.elapsed().as_secs_f64());
    tracing::Span::current().record("scopes.permitted", plan.scopes.len());

    // Effective budget: the caller narrows, never widens (decision 7).
    let budget_tokens = match payload.budget_tokens {
        Some(requested) => plan.budget_tokens.min(requested),
        None => plan.budget_tokens,
    };
    // The valid-time instant, stamped once at the seam (decision 6):
    // everything below is deterministic given it.
    let as_of = Utc::now();

    let mut degraded: Vec<&'static str> = Vec::new();

    // The retrieval leg — only for a task over a non-empty plan (an
    // empty plan composes the empty block regardless; skipping spares
    // the embed round-trip).
    let query = payload
        .task
        .as_ref()
        .filter(|_| !plan.scopes.is_empty())
        .cloned();

    // The embed call: the MEM-4 seam, outside any transaction, under
    // the read-path deadline (decision 3). Failure or expiry drops the
    // dense leg, never the request.
    let vector = match &query {
        Some(task) => {
            let stage = Instant::now();
            let embedded = tokio::time::timeout(
                state.inject_embed_timeout,
                state.embedder.embed(std::slice::from_ref(task)),
            )
            .await;
            metrics::histogram!(INJECT_STAGE_SECONDS, "stage" => "embed")
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

    // Transaction 2: search (RLS-scoped hydration), compose, and the
    // audit append — chain-head lock last (ADR-0019) — then commit.
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let relevance = match &query {
        Some(task) => {
            let request = SearchRequest {
                query: task.clone(),
                vector,
                // The plan's own pairs: the retrieval legs never learn
                // what a tier means, they are handed the answer
                // (ADR-0038 decision 3).
                filter: SearchFilter {
                    tiers: plan
                        .scopes
                        .iter()
                        .flat_map(|scope| {
                            synveda_types::ScopeTier::expand(scope.scope_id, &scope.sensitivities)
                        })
                        .collect(),
                },
                limit: RELEVANCE_LIMIT,
                per_leg: RELEVANCE_LIMIT,
            };
            let stage = Instant::now();
            let searched = hybrid_search(&mut tx, &state.search_index, tenant_id, &request).await;
            metrics::histogram!(INJECT_STAGE_SECONDS, "stage" => "search")
                .record(stage.elapsed().as_secs_f64());
            match searched {
                Ok(results) => Some(
                    results
                        .into_iter()
                        .map(|retrieved| retrieved.record.id)
                        .collect(),
                ),
                Err(error) => {
                    // The engine already degrades one-sidedly (ADR-0024);
                    // an error here means no ranking at all — compose
                    // still serves pinned + recency-ordered derived.
                    tracing::warn!(error = %error, "hybrid search failed; composing unranked");
                    degraded.push("retrieval");
                    None
                }
            }
        }
        None => None,
    };

    let scope_decisions = plan.decisions;
    let home_resource = match input.identity.as_ref() {
        Some(identity) => Resource::Scope(identity.scope_id).to_string(),
        None => "scope none".to_owned(),
    };
    let mut request = ComposeRequest::new(plan.scopes, budget_tokens, as_of);
    if let Some(ceiling) = payload.max_sensitivity {
        request = request.narrowed_to(ceiling);
    }
    request.relevance = relevance;
    let stage = Instant::now();
    let block = compose(&mut tx, tenant_id, &request).await?;
    metrics::histogram!(INJECT_STAGE_SECONDS, "stage" => "compose")
        .record(stage.elapsed().as_secs_f64());

    let stage = Instant::now();
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::ContextInjected,
        home_resource,
        Outcome::Success,
        json!({
            "session_id": payload.session_id,
            // Correlation without content (ADR-0021 discipline): the
            // task rides as a hash or not at all.
            "task_hash": payload.task.as_deref().map(task_hash),
            "as_of": as_of,
            "block_hash": block.block_hash,
            // The full per-entry watermark (ADR-0025 decision 7, as
            // ADR-0031 decision 11 upgraded it): the VedaFlow object
            // address of exactly the version that composed, plus the
            // channel it composed from — the trust label an auditor
            // reads before the content.
            "entries": block.entries.iter().map(|entry| json!({
                "record_id": entry.record_id,
                "object_hash": entry.object_hash,
                "channel": entry.channel,
            })).collect::<Vec<_>>(),
            // Where each scope's published channel pointed when the
            // block was composed: tech plan §2.5's "inject responses
            // cite commit hashes", paid for out of the audit event
            // rather than the token budget.
            "channels": block.channels.iter().map(|channel| json!({
                "scope_id": channel.scope_id,
                "ref": channel.channel,
                "commit": channel.commit,
                // Whether a pin chose that commit (FLOW-7, ADR-0036
                // decision 10): "what did the agent know" has a different
                // answer at a scope that froze what it serves, and the
                // trail must not have to be reconstructed to learn it.
                "pinned": channel.pinned,
            })).collect::<Vec<_>>(),
            "tokens": block.tokens,
            "budget_tokens": block.budget_tokens,
            "dropped_conflicts": block.dropped_conflicts,
            "skipped_over_budget": block.skipped_over_budget,
            "degraded": degraded,
            // The aggregated per-scope MemoryRead decisions (ADR-0019
            // decision 4): one event, every chain scope's verdict. The
            // per-call decision log stays the full-fidelity record.
            // Since AUTHZ-5 each verdict carries the *tiers* the walk
            // permitted, not just an allow (ADR-0038 decision 13): "who
            // could see this scope's restricted material on date D" is the
            // question a regulator asks, and this is what answers it.
            "decisions": scope_decisions.iter().map(|decision| json!({
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
        message: format!("commit inject transaction: {err}"),
    })?;
    metrics::histogram!(INJECT_STAGE_SECONDS, "stage" => "audit")
        .record(stage.elapsed().as_secs_f64());

    let span = tracing::Span::current();
    span.record("block.entries", block.entries.len());
    span.record("block.tokens", block.tokens);
    span.record("degraded", degraded.join(","));
    Ok(render(block, as_of, degraded))
}

/// BLAKE3 of the task text, hex — audit correlation without content.
fn task_hash(task: &str) -> String {
    blake3::hash(task.as_bytes()).to_hex().to_string()
}

fn render(
    block: ComposedBlock,
    as_of: DateTime<Utc>,
    degraded: Vec<&'static str>,
) -> InjectResponse {
    InjectResponse {
        record_ids: block.entries.iter().map(|entry| entry.record_id).collect(),
        channels: block
            .channels
            .iter()
            .map(|channel| ChannelView {
                scope_id: channel.scope_id,
                r#ref: channel.channel.clone(),
                commit: channel.commit.clone(),
                pinned: channel.pinned,
            })
            .collect(),
        text: block.text,
        block_hash: block.block_hash,
        tokens: block.tokens,
        budget_tokens: block.budget_tokens,
        as_of,
        degraded,
    }
}

/// One channel citation on a block.
#[derive(Serialize)]
struct ChannelView {
    scope_id: ScopeId,
    /// The ref name, e.g. `memory/published`.
    r#ref: String,
    /// The commit this scope served.
    commit: String,
    /// True when a pin chose that commit — the scope is deliberately not
    /// serving its latest reviewed material (ADR-0036 decision 10).
    pinned: bool,
}
