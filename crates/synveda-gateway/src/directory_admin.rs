//! The pull sync's operator surface (AUTH-5, ADR-0060 decision 10): seeing
//! what the circuit breaker refused, and signing for it.
//!
//! Two routes and one action. `GET /v1/directory/sync` shows a tenant's pass
//! state — including the size of a refusal and any authorisation standing —
//! and `POST /v1/directory/seal-authorisations` grants one.
//!
//! ## Why this is on `/v1` and nowhere else
//!
//! The release is **unreachable from the SCIM plane, from a provisioning
//! credential, and from the connector**, and that is the load-bearing half
//! of decision 10. ADR-0059 decision 12 refuses to let the directory lift a
//! seal, because after a directory compromise the party holding the
//! provisioning credential is the attacker and "a hold that the directory
//! can release is not a hold". The same sentence with the sign flipped is
//! this module's reason for existing: a breaker the directory can wave
//! through is not a breaker, and waving it through is exactly how somebody
//! who owns a directory converts a read into mass deprovisioning.
//!
//! Confinement is structural rather than checked here. A SCIM credential is
//! refused by the `/v1` router before any handler runs (ADR-0059 decision
//! 13), and the sync job holds no token at all — it is an
//! `ActorKind::System` task inside this process, with no way to originate a
//! `/v1` request. The acceptance suite drives both and requires both to be
//! refused, because a control whose custody is only described is one nobody
//! has checked.
//!
//! ## Why the read and the signature share an action
//!
//! Unlike the service-identity plane's read/manage pair, and unlike
//! `DirectoryManage` — which is one action for the opposite reason, that a
//! credential inventory without rotation is only reconnaissance. Here a
//! signer who cannot see the number they are bounding is being asked to
//! sign blind, which is precisely what the ceiling exists to prevent.

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{Action, Resource};
use synveda_store::{directory_sync, rls};
use synveda_types::{Error, Result};

use crate::app::AppState;
use crate::audit;
use crate::authz;
use crate::error::ApiError;
use crate::request::{body, commit, tenant_id};

/// The longest window an authorisation may be granted for.
///
/// A day, and short by design: this is permission to destroy access
/// irreversibly, granted in response to an incident that is happening now.
/// A window that outlives the incident is a standing pre-approval for the
/// *next* directory failure, which is the event the breaker exists to catch.
const MAX_WINDOW_SECS: f64 = 86_400.0;
/// The default when a caller names no window — long enough for the next
/// scheduled pass on any sane interval, short enough to expire unnoticed.
const DEFAULT_WINDOW_SECS: f64 = 7_200.0;

/// `GET /v1/directory/sync` — what the last pass did.
#[derive(Debug, Serialize)]
pub struct SyncStatus {
    /// Which connector last wrote this state.
    pub connector: String,
    /// Passes that completed. An absence count means nothing without it.
    pub passes_completed: i64,
    /// The last attempt, complete or not.
    pub last_pass_at: Option<DateTime<Utc>>,
    /// The last one that finished. A gap between this and `last_pass_at` is
    /// a connector that runs and never completes — the state in which
    /// nobody is sealed and nothing looks wrong.
    pub last_complete_pass_at: Option<DateTime<Utc>>,
    /// Set iff the most recent complete pass refused to seal.
    pub breaker_tripped_at: Option<DateTime<Utc>>,
    /// How many that pass declined to seal — the number an operator is
    /// being asked to bound.
    pub breaker_would_have_sealed: Option<i32>,
    /// The authorisation standing right now, if any.
    pub seal_authorisation: Option<AuthorisationView>,
}

/// A standing authorisation, as an operator sees it.
#[derive(Debug, Serialize)]
pub struct AuthorisationView {
    /// When it was signed.
    pub granted_at: DateTime<Utc>,
    /// When it stops covering anything.
    pub expires_at: DateTime<Utc>,
    /// The most it permits. A pass proposing more trips again.
    pub ceiling: i32,
    /// Who signed it.
    pub granted_by: String,
    /// Why.
    pub reason: String,
}

/// `POST /v1/directory/seal-authorisations`.
#[derive(Debug, Deserialize)]
pub struct AuthoriseRequest {
    /// The most this authorisation permits a pass to seal.
    pub ceiling: i32,
    /// Why, in the operator's words. Required, and stored — an
    /// authorisation nobody can read the reason for explains nothing later.
    pub reason: String,
    /// How long it stands. Clamped to [`MAX_WINDOW_SECS`].
    #[serde(default)]
    pub expires_in_secs: Option<f64>,
}

/// `GET /v1/directory/sync`.
#[tracing::instrument(name = "directory.sync.status", skip_all)]
pub async fn status(State(state): State<AppState>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        authz::require(
            &state,
            &mut tx,
            Action::DirectorySealAuthorise,
            Resource::Tenant(tenant_id),
            None,
        )
        .await?;
        let sync = directory_sync::state(&mut *tx, tenant_id).await?;
        commit(tx).await?;

        // A tenant that has never been pulled has no state, and that is a
        // `404` rather than an empty body: "no sync has ever run here" and
        // "a sync ran and found nothing" are different answers, and an
        // operator chasing a stalled connector needs to tell them apart.
        let sync = sync.ok_or_else(|| Error::NotFound {
            entity: format!("directory sync for tenant {tenant_id}"),
        })?;
        Ok((
            StatusCode::OK,
            Json(SyncStatus {
                connector: sync.connector,
                passes_completed: sync.passes_completed,
                last_pass_at: sync.last_pass_at,
                last_complete_pass_at: sync.last_complete_pass_at,
                breaker_tripped_at: sync.breaker_tripped_at,
                breaker_would_have_sealed: sync.breaker_would_have_sealed,
                seal_authorisation: sync.authorisation.map(|granted| AuthorisationView {
                    granted_at: granted.granted_at,
                    expires_at: granted.expires_at,
                    ceiling: granted.ceiling,
                    granted_by: granted.granted_by,
                    reason: granted.reason,
                }),
            }),
        ))
    }
    .await;
    respond(result)
}

/// `POST /v1/directory/seal-authorisations`.
#[tracing::instrument(name = "directory.seal.authorise", skip_all)]
pub async fn authorise(
    State(state): State<AppState>,
    payload: std::result::Result<Json<AuthoriseRequest>, JsonRejection>,
) -> Response {
    let result = async {
        let request = body(payload)?;
        let tenant_id = tenant_id()?;

        // Refused before the PDP is asked, because these are malformed
        // requests rather than forbidden ones. A ceiling of zero authorises
        // nothing and would be spent by the first pass that consulted it,
        // clearing the standing authorisation having sealed nobody — a
        // failure that looks like the breaker misbehaving.
        if request.ceiling <= 0 {
            return Err(Error::Invalid {
                message: "ceiling must be at least 1: an authorisation to seal \
                          nobody is not an authorisation"
                    .to_owned(),
            });
        }
        let reason = request.reason.trim();
        if reason.is_empty() || reason.chars().count() > 512 {
            return Err(Error::Invalid {
                message: "reason is required and is at most 512 characters".to_owned(),
            });
        }

        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let authorized = authz::require(
            &state,
            &mut tx,
            Action::DirectorySealAuthorise,
            Resource::Tenant(tenant_id),
            None,
        )
        .await?;

        let window = request
            .expires_in_secs
            .unwrap_or(DEFAULT_WINDOW_SECS)
            .clamp(1.0, MAX_WINDOW_SECS);
        let subject = synveda_identity::current_tenant()
            .map(|context| context.claims.subject.clone())
            .unwrap_or_default();
        let granted = directory_sync::authorise_seals(
            &mut *tx,
            tenant_id,
            request.ceiling,
            window,
            &subject,
            reason,
        )
        .await?;
        if !granted {
            // No state row: this tenant has never been pulled, so there is
            // no trip to release. Refusing is the honest answer — granting
            // one would leave a standing permission waiting for the first
            // pass a future deployment ever runs.
            return Err(Error::NotFound {
                entity: format!("directory sync for tenant {tenant_id}"),
            });
        }

        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::DirectorySealAuthorised,
            format!("tenant {tenant_id}"),
            Outcome::Success,
            json!({
                "ceiling": request.ceiling,
                "reason": reason,
                // Seconds as an integer: an audit payload may hold no
                // non-integer number, because jsonb re-renders floats and
                // the chain's hash is over the rendered bytes.
                "window_secs": window.round() as i64,
                "authz": audit::decision_context(Action::DirectorySealAuthorise, &authorized),
            }),
        )
        .await?;
        commit(tx).await?;

        tracing::warn!(
            tenant.id = %tenant_id,
            ceiling = request.ceiling,
            granted_by = %subject,
            "seal authorisation granted: the next complete pass may seal up to \
             this many people, once"
        );
        Ok((
            StatusCode::CREATED,
            Json(json!({"ceiling": request.ceiling})),
        ))
    }
    .await;
    respond(result)
}

fn respond<T: IntoResponse>(result: Result<T>) -> Response {
    match result {
        Ok(ok) => ok.into_response(),
        Err(error) => ApiError(error).into_response(),
    }
}
