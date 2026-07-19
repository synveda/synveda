//! Audit emission seams (AUD-1, ADR-0019 decision 5). The gateway is the
//! one component that sees actor, tenant, decision, and transaction
//! together, so emission lives here — the PDP decides, the store persists,
//! this module records.
//!
//! Two shapes:
//! - [`record`] — success events, appended inside the handler's own tenant
//!   transaction immediately before commit, so the event and the action it
//!   records commit atomically. A failed append fails the operation: an
//!   action without its audit record must not exist.
//! - [`record_rejection`] / [`record_detached`] — error-path events (the
//!   handler's transaction is already rolled back) in a fresh short
//!   transaction. Best-effort by design: the original error always reaches
//!   the caller; an append failure here is logged and counted
//!   (`synveda_audit_append_failures_total`), never masked.

use serde_json::{Value, json};
use sqlx::PgConnection;
use synveda_audit::{Actor, AppendedEvent, AuditAction, AuditEvent, Outcome};
use synveda_policy::Action;
use synveda_store::rls;
use synveda_types::{Error, Result, TenantId};

use crate::app::AppState;
use crate::authz::{self, Authorized};

/// The OTel trace id of the live request span, when the subscriber has
/// one — links every chain row to its trace (ADR-0019 decision 7).
fn current_trace_id() -> Option<String> {
    use opentelemetry::trace::TraceContextExt;
    use tracing_opentelemetry::OpenTelemetrySpanExt;
    let context = tracing::Span::current().context();
    let span = context.span();
    let span_context = span.span_context();
    span_context
        .is_valid()
        .then(|| span_context.trace_id().to_string())
}

/// The ambient actor: the verified token subject the tenant-resolution
/// middleware established. `None` outside the authenticated plane.
fn ambient_actor() -> Option<Actor> {
    synveda_identity::current_tenant().map(|context| Actor::subject(context.claims.subject))
}

/// The `authz` payload block a semantic or decision event embeds — the
/// decision recorded without a second chain row (ADR-0019 decision 4).
pub(crate) fn decision_context(action: Action, authorized: &Authorized) -> Value {
    json!({
        "action": action.as_str(),
        "pack": format!(
            "{}@{}",
            authorized.decision.pack_name, authorized.decision.pack_version
        ),
        "determining": authorized.decision.determining,
        "roles": authorized.roles,
    })
}

/// Appends an event inside the caller's tenant transaction, with the
/// ambient actor — `Success` for semantic mutation events, `Allow` for
/// read-path decision events. Call immediately before commit: the
/// chain-head lock is the last lock the transaction takes (ADR-0019
/// decision 1).
pub(crate) async fn record(
    tx: &mut PgConnection,
    tenant_id: TenantId,
    action: AuditAction,
    resource: String,
    outcome: Outcome,
    payload: Value,
) -> Result<AppendedEvent> {
    let actor = ambient_actor().ok_or_else(|| Error::Internal {
        message: "audited operation ran outside a tenant scope".to_owned(),
    })?;
    record_as(tx, tenant_id, actor, action, resource, outcome, payload).await
}

/// [`record`] with an explicit actor and outcome — the login-plane seams
/// (JIT provisioning) run outside the task-local tenant scope and name
/// their actor themselves.
pub(crate) async fn record_as(
    tx: &mut PgConnection,
    tenant_id: TenantId,
    actor: Actor,
    action: AuditAction,
    resource: String,
    outcome: Outcome,
    payload: Value,
) -> Result<AppendedEvent> {
    synveda_audit::append(
        tx,
        tenant_id,
        &AuditEvent {
            occurred_at: chrono::Utc::now(),
            actor,
            action,
            resource,
            outcome,
            payload,
            trace_id: current_trace_id(),
        },
    )
    .await
}

/// Appends one event in its own short tenant transaction — the error-path
/// shape, where the handler's transaction is already rolled back, and the
/// pre-handler shape (suspended-tenant resolution denials). Best-effort:
/// failures are logged and counted, never propagated (ADR-0019
/// decision 5).
pub(crate) async fn record_detached(
    state: &AppState,
    tenant_id: TenantId,
    actor: Actor,
    action: AuditAction,
    resource: String,
    outcome: Outcome,
    payload: Value,
) {
    let event = AuditEvent {
        occurred_at: chrono::Utc::now(),
        actor,
        action,
        resource,
        outcome,
        payload,
        trace_id: current_trace_id(),
    };
    let appended = async {
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        synveda_audit::append(&mut tx, tenant_id, &event).await?;
        tx.commit().await.map_err(|err| Error::Storage {
            message: format!("commit audit transaction: {err}"),
        })
    }
    .await;
    if let Err(error) = appended {
        metrics::counter!(
            synveda_audit::AUDIT_APPEND_FAILURES_TOTAL,
            "action" => event.action.as_str(),
        )
        .increment(1);
        tracing::error!(
            tenant.id = %tenant_id,
            audit.action = event.action.as_str(),
            error = %error,
            "audit append failed on a best-effort path; the event is lost"
        );
    }
}

/// The `respond` seam (ADR-0019 decision 5): classifies a handler error
/// and appends the matching event. Three classes chain; everything else
/// (plain 404s, validation, storage failures) stays in metrics and traces.
pub(crate) async fn record_rejection(state: &AppState, op: &'static str, error: &Error) {
    let Some(context) = synveda_identity::current_tenant() else {
        // No resolved tenant means no chain to write to — and nothing
        // attributable to write (ADR-0019 decision 6).
        return;
    };
    let tenant_id = context.tenant.id;
    let actor = Actor::subject(context.claims.subject);
    match error {
        // Every PDP denial is a chain event (seed §2.5). The reason names
        // the pack@version and determining policies, never content.
        Error::PolicyDenied {
            action,
            resource,
            reason,
        } => {
            record_detached(
                state,
                tenant_id,
                actor,
                AuditAction::AuthzDecision,
                resource.clone(),
                Outcome::Deny,
                json!({"op": op, "action": action, "reason": reason}),
            )
            .await;
        }
        // The service-token lifetime seam (ADR-0018 decision 5).
        Error::Unauthenticated { message } if authz::is_service_token_rejection(error) => {
            record_detached(
                state,
                tenant_id,
                actor,
                AuditAction::TokenRejected,
                format!("tenant {tenant_id}"),
                Outcome::Deny,
                json!({"op": op, "reason": message}),
            )
            .await;
        }
        // The RLS backstop tripped (TEN-2, ADR-0009): an isolation
        // invariant broke — exactly what an auditor must be able to find.
        _ if rls::is_backstop_trip(error) => {
            record_detached(
                state,
                tenant_id,
                actor,
                AuditAction::RlsBackstopTripped,
                format!("tenant {tenant_id}"),
                Outcome::Failure,
                json!({"op": op, "error": error.to_string()}),
            )
            .await;
        }
        _ => {}
    }
}
