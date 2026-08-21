//! The quarantine review API (MEM-2, ADR-0021 decisions 5–7):
//! `/v1/quarantine` behind tenant resolution, uniform-404 ownership, and
//! the PDP (`QuarantineRead` for the queue, `QuarantineReview` for
//! release/reject).
//!
//! **The plane is tenant-anchored** (CPR-7, ADR-0074 decision 7). Since
//! placement became identity, every quarantined event lands at a
//! `principal`-shaped scope — and a principal scope inherits nothing, so
//! no grant but one written directly at that person's own scope carries a
//! role there. Anchoring the verdict at the event's scope therefore made
//! this plane unreachable for every event it exists for. It decides at the
//! tenant resource instead, which is where the queue's tenant-wide branch
//! has always decided and which matches the packs' own treatment of this
//! control: how a security control is reviewed does not loosen per pack.
//! A `scope_id` on the queue stays a **filter**, not the anchor.
//!
//! A quarantined observe event staged redacted but signal-less; the
//! reviewer — a `reviewer`, `owner` or `administrator` in the tenant —
//! sees the redacted payload plus the finding summary and
//! decides flow: release sends the standard work signal in the review's
//! own transaction (the pipeline cannot tell a released event from an
//! admitted one), reject leaves the staging row provenance-only. Review
//! is one-shot; a second verdict is a 409. An event whose home scope was
//! since deleted (a revoked agent's leaf) answers the uniform 404 —
//! its rows await disposal (MEM-6/TEN-5).

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{Action, Resource};
use synveda_store::quarantine::{QuarantinedEvent, ReviewDecision};
use synveda_store::{quarantine, rls, scopes};
use synveda_types::{Error, IdentityId, ObserveEventId, QuarantineState, Result, ScopeId};

use crate::app::AppState;
use crate::audit;
use crate::authz;
use crate::error::ApiError;
use crate::request::{commit, found, tenant_id};
use crate::telemetry::QUARANTINE_OPERATIONS_TOTAL;

/// The queue page cap; `limit` above it is a 400, not a silent trim.
const MAX_LIMIT: i64 = 500;
const DEFAULT_LIMIT: i64 = 100;

/// The review-reason cap; mirrors the table's CHECK constraint.
const MAX_REASON_CHARS: usize = 1000;

/// Counts the operation and renders the result — the same outcome
/// taxonomy as every governed plane. Error-path audit events chain here
/// (AUD-1, ADR-0019 decision 5).
async fn respond<T: IntoResponse>(
    state: &AppState,
    op: &'static str,
    result: Result<T>,
) -> Response {
    let outcome = match &result {
        Ok(_) => "ok",
        Err(
            Error::Unauthenticated { .. }
            | Error::PolicyDenied { .. }
            | Error::NotFound { .. }
            | Error::Invalid { .. }
            | Error::Conflict { .. }
            | Error::RateLimited { .. },
        ) => "rejected",
        Err(_) => "error",
    };
    metrics::counter!(QUARANTINE_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    match result {
        Ok(response) => response.into_response(),
        Err(error) => {
            audit::record_rejection(state, op, &error).await;
            ApiError(error).into_response()
        }
    }
}

/// One quarantined event as the API renders it: the redacted payload,
/// the finding summary, and the review state — never raw finding text
/// (there is none anywhere to render, ADR-0021 decision 1).
#[derive(Serialize)]
struct QuarantineView {
    event_id: ObserveEventId,
    scope_id: ScopeId,
    owner_id: IdentityId,
    session_id: String,
    kind: String,
    payload: serde_json::Value,
    findings: serde_json::Value,
    state: QuarantineState,
    created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reviewer_subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reviewed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    review_reason: Option<String>,
}

impl From<QuarantinedEvent> for QuarantineView {
    fn from(event: QuarantinedEvent) -> Self {
        QuarantineView {
            event_id: event.event_id,
            scope_id: event.scope_id,
            owner_id: event.owner_id,
            session_id: event.session_id,
            kind: event.kind,
            payload: event.payload,
            findings: event.findings,
            state: event.state,
            created_at: event.created_at,
            reviewer_subject: event.reviewer_subject,
            reviewed_at: event.reviewed_at,
            review_reason: event.review_reason,
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct ListParams {
    /// Restrict the queue to this node's subtree; absent lists
    /// tenant-wide (and takes a tenant-resource decision).
    scope_id: Option<ScopeId>,
    limit: Option<i64>,
}

#[derive(Serialize)]
struct QueueResponse {
    pending: Vec<QuarantineView>,
}

/// `GET /v1/quarantine` — the pending review queue, oldest first.
/// `QuarantineRead` is decided at the tenant either way (module doc);
/// `scope_id` narrows *which* events come back, after the uniform-404
/// ownership check on the scope named.
#[tracing::instrument(name = "quarantine.list", skip_all)]
pub(crate) async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Response {
    let result = async {
        let limit = params.limit.unwrap_or(DEFAULT_LIMIT);
        if !(1..=MAX_LIMIT).contains(&limit) {
            return Err(Error::Invalid {
                message: format!("limit must be 1..={MAX_LIMIT}"),
            });
        }
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let (authorized, resource, scope_filter) = match params.scope_id {
            None => {
                let authorized = authz::require(
                    &state,
                    &mut tx,
                    Action::QuarantineRead,
                    Resource::Tenant(tenant_id),
                    None,
                )
                .await?;
                (authorized, Resource::Tenant(tenant_id), None)
            }
            Some(scope_id) => {
                // Ownership first, so a made-up id is a 404 and never a
                // denial oracle (ADR-0012 decision 7).
                found(
                    scopes::get(&mut *tx, tenant_id, scope_id).await?,
                    tenant_id,
                    scope_id,
                )?;
                let authorized = authz::require(
                    &state,
                    &mut tx,
                    Action::QuarantineRead,
                    Resource::Tenant(tenant_id),
                    None,
                )
                .await?;
                // The subtree filter: the scope and everything below it —
                // quarantined events live at the `principal` scopes under
                // it (CPR-7).
                let mut scopes: Vec<ScopeId> = scopes::descendants(&mut *tx, tenant_id, scope_id)
                    .await?
                    .into_iter()
                    .map(|node| node.id)
                    .collect();
                scopes.push(scope_id);
                (authorized, Resource::Scope(scope_id), Some(scopes))
            }
        };
        let pending =
            quarantine::pending(&mut tx, tenant_id, scope_filter.as_deref(), limit).await?;
        // An allowed admin-plane read chains its decision (ADR-0019
        // decision 4).
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::AuthzDecision,
            resource.to_string(),
            Outcome::Allow,
            json!({
                "op": "list",
                "authz": audit::decision_context(Action::QuarantineRead, &authorized),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(QueueResponse {
            pending: pending.into_iter().map(Into::into).collect(),
        }))
    }
    .await;
    respond(&state, "list", result).await
}

#[derive(Deserialize)]
pub(crate) struct ReviewBody {
    /// The reviewer's note, recorded on the row and in the audit event.
    reason: Option<String>,
}

/// `POST /v1/quarantine/{event_id}/release`.
#[tracing::instrument(name = "quarantine.release", skip_all)]
pub(crate) async fn release(
    State(state): State<AppState>,
    Path(event_id): Path<ObserveEventId>,
    payload: std::result::Result<Json<ReviewBody>, JsonRejection>,
) -> Response {
    let result = review(&state, event_id, payload, ReviewDecision::Release).await;
    respond(&state, "release", result).await
}

/// `POST /v1/quarantine/{event_id}/reject`.
#[tracing::instrument(name = "quarantine.reject", skip_all)]
pub(crate) async fn reject(
    State(state): State<AppState>,
    Path(event_id): Path<ObserveEventId>,
    payload: std::result::Result<Json<ReviewBody>, JsonRejection>,
) -> Response {
    let result = review(&state, event_id, payload, ReviewDecision::Reject).await;
    respond(&state, "reject", result).await
}

/// The shared review path: uniform-404 ownership (the quarantine row,
/// then its scope node), `QuarantineReview` at the tenant (module doc), the
/// one-shot state flip (plus the release signal) on this transaction,
/// and the chained semantic event — all atomic (ADR-0021 decision 7).
async fn review(
    state: &AppState,
    event_id: ObserveEventId,
    payload: std::result::Result<Json<ReviewBody>, JsonRejection>,
    decision: ReviewDecision,
) -> Result<Json<QuarantineView>> {
    let body = crate::request::body(payload)?;
    if let Some(reason) = &body.reason
        && (reason.is_empty() || reason.chars().count() > MAX_REASON_CHARS)
    {
        return Err(Error::Invalid {
            message: format!("reason must be 1..={MAX_REASON_CHARS} characters"),
        });
    }
    let tenant_id = tenant_id()?;
    let subject = synveda_identity::current_tenant()
        .map(|context| context.claims.subject)
        .ok_or_else(|| Error::Internal {
            message: "quarantine review ran outside a tenant scope".to_owned(),
        })?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let Some(event) = quarantine::get(&mut tx, tenant_id, event_id).await? else {
        return Err(Error::NotFound {
            entity: "quarantined event".to_owned(),
        });
    };
    // The event's scope is still resolved, because a since-deleted scope
    // (a revoked agent's) answers the uniform 404 like every dangling
    // resource — disposal owns those rows (module doc). It no longer
    // anchors the decision: see the module doc.
    found(
        scopes::get(&mut *tx, tenant_id, event.scope_id).await?,
        tenant_id,
        event.scope_id,
    )?;
    let authorized = authz::require(
        state,
        &mut tx,
        Action::QuarantineReview,
        Resource::Tenant(tenant_id),
        None,
    )
    .await?;
    let reviewed = quarantine::review(
        &mut tx,
        tenant_id,
        event_id,
        decision,
        &subject,
        body.reason.as_deref(),
    )
    .await?
    .ok_or_else(|| Error::NotFound {
        // The row vanished between the get and the update — only
        // possible via out-of-band deletion; the uniform answer stands.
        entity: "quarantined event".to_owned(),
    })?;
    let action = match decision {
        ReviewDecision::Release => AuditAction::QuarantineReleased,
        ReviewDecision::Reject => AuditAction::QuarantineRejected,
    };
    audit::record(
        &mut tx,
        tenant_id,
        action,
        Resource::Scope(reviewed.scope_id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::QuarantineReview, &authorized),
            "event_id": reviewed.event_id,
            "owner_id": reviewed.owner_id,
            "session_id": reviewed.session_id,
            // Rule ids and counts — the finding summary is already
            // content-free (ADR-0021 decision 1).
            "findings": reviewed.findings,
            "reason": reviewed.review_reason,
        }),
    )
    .await?;
    commit(tx).await?;
    Ok(Json(reviewed.into()))
}
