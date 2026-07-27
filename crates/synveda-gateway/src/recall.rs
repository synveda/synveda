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
//! CTX-5 widens this into the deep query the tech plan §3 describes —
//! graph traversal, as-of, one MCP tool — by adding a query alternative
//! to this surface under the same audit action, not by replacing it.

use std::time::Instant;

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::Resource;
use synveda_retrieval::{
    Admitted, ComposeRequest, LapsedScope, MemoryReadInputs, admit, composition_plan,
};
use synveda_store::rls;
use synveda_types::{
    Channel, Error, RecordClass, RecordId, RecordKind, Result, ScopeId, Sensitivity,
};

use crate::app::AppState;
use crate::audit;
use crate::authz;
use crate::error::ApiError;
use crate::hierarchy::{body, tenant_id};
use crate::telemetry::{CONTEXT_RECALLS_TOTAL, RECALL_RECORDS_TOTAL};

/// How many records one recall may name (ADR-0041 decision 7).
///
/// Comfortably above any block's plausible index tier, and far below a
/// corpus: recall is a deep read of named material, not a bulk export.
/// A blunt instrument compared with a rate limit, which is AUTH-6's.
const MAX_RECALL_IDS: usize = 32;

/// Cap for `session_id`; the inject surface's, for the same reason.
const MAX_SESSION_ID_CHARS: usize = 200;

#[derive(Deserialize)]
pub(crate) struct RecallBody {
    /// The records to serve — the handles an index entry rendered as
    /// `(recall <id>)`.
    ids: Vec<RecordId>,
    /// Opaque harness session identifier, audit-correlated with the
    /// session's injects and observe batches.
    session_id: Option<String>,
}

#[derive(Serialize)]
struct RecallResponse {
    /// The records the *current* plan admits, in gradient order — nearest
    /// scope first, published before derived, exactly as they would have
    /// been ordered in a block.
    ///
    /// Ids the plan does not admit are absent, without distinction: an id
    /// that never existed and an id the caller may not read read the same
    /// from here (ADR-0041 decision 6).
    entries: Vec<RecallEntry>,
    /// How many ids were asked for. The caller can already count the
    /// difference, so stating it leaks nothing — and it makes "I asked
    /// for six and got four" a fact the response carries rather than one
    /// the caller has to notice.
    requested: usize,
    /// The valid-time instant the admission was decided at.
    as_of: DateTime<Utc>,
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
    if payload.ids.is_empty() || payload.ids.len() > MAX_RECALL_IDS {
        return invalid(format!("ids must name 1..={MAX_RECALL_IDS} records"));
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

/// `POST /v1/recall` — serve the bodies behind named handles (ADR-0041).
pub(crate) async fn create(
    State(state): State<AppState>,
    payload: std::result::Result<Json<RecallBody>, JsonRejection>,
) -> Response {
    match handle(&state, payload).await {
        Ok(response) => {
            let outcome = if response.entries.is_empty() {
                "empty"
            } else {
                "ok"
            };
            metrics::counter!(CONTEXT_RECALLS_TOTAL, "outcome" => outcome).increment(1);
            Json(response).into_response()
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
        scopes.permitted = tracing::field::Empty,
        records.requested = tracing::field::Empty,
        records.served = tracing::field::Empty,
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
    let requested = payload.ids.len();
    let span = tracing::Span::current();
    span.record("records.requested", requested);
    metrics::counter!(RECALL_RECORDS_TOTAL, "side" => "requested").increment(requested as u64);

    // The same decision inputs inject gathers, in the same order, from
    // the same caches — because this must be the same decision. There is
    // no embed call on this path, so one transaction carries the whole
    // request.
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let input = authz::gather_at_home(state, &mut tx).await?;
    let lapsed_chains = authz::gather_lapsed(state, &mut tx, &input).await?;
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
    span.record("scopes.permitted", plan.scopes.len());

    // The instant, stamped once at the seam exactly as inject stamps it
    // (ADR-0026 decision 6): everything below is deterministic given it,
    // and the valid-window and horizon predicates both read it.
    let as_of = Utc::now();
    let request = ComposeRequest::naming(plan.scopes, payload.ids.clone(), as_of);
    let stage = Instant::now();
    // The one admission function. A quarantined caller, an unplaced
    // identity, or a plan every scope denied admits nothing — the empty
    // answer, never an error, because a policy outcome on a read is a
    // result (ADR-0026 decision 1).
    let admission = admit(&mut tx, tenant_id, &request).await?;
    tracing::debug!(
        elapsed_ms = stage.elapsed().as_millis() as u64,
        admitted = admission.records.len(),
        "recall admission decided"
    );

    // Gradient order, so a recall reads like the block it came from.
    let mut records = admission.records;
    records.sort_by(|a, b| {
        a.position
            .cmp(&b.position)
            .then_with(|| channel_rank(a).cmp(&channel_rank(b)))
            .then_with(|| a.version.id.cmp(&b.version.id))
    });
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
            "as_of": as_of,
            // What was asked for and what was served, as counts. Never the
            // refused ids: the surface answers uniformly for those, and a
            // payload enumerating them would put the oracle back on the
            // chain (ADR-0041 decision 8).
            "requested": requested,
            "served": entries.len(),
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

    Ok(RecallResponse {
        entries,
        requested,
        as_of,
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
