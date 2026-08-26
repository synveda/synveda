//! The provisioning credential's admin routes (AUTH-4, ADR-0059
//! decision 13).
//!
//! These live on `/v1`, not on `/scim/v2`, and the split is deliberate:
//! issuing the credential is an act of the product's own authority, decided
//! by the PDP at the tenant resource, while the plane it opens is the
//! directory's. A credential that could mint another credential would make
//! the directory the authority on its own access.
//!
//! `DirectoryManage` is one action for reading the inventory and mutating
//! it. The inventory is a list of live keys to a tenant's directory plane,
//! and a role that could see which credentials exist without being able to
//! rotate them would hold nothing but reconnaissance.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{Action, Resource};
use synveda_store::{directory, rls};
use synveda_types::{Error, Result, ScimCredential, ScimCredentialId};

use crate::app::AppState;
use crate::audit;
use crate::authz;
use crate::error::ApiError;
use crate::request::{body, commit, tenant_id};
use crate::telemetry::SCIM_CREDENTIAL_OPERATIONS_TOTAL;

/// The longest life a provisioning credential may be issued for, and the
/// default when a caller names none.
///
/// AUTH-3's lifetime-cap doctrine (ADR-0018 decision 5) applied to a
/// credential that cannot be short-lived: Entra holds one static string
/// and never refreshes it, so the cap is the only thing that makes this
/// key expire at all. A year is long enough not to break provisioning
/// unattended and short enough that a forgotten credential is not
/// permanent.
const MAX_LIFETIME_DAYS: i64 = 365;
/// The default when a caller names no window: a quarter, which is a
/// rotation cadence somebody can actually keep.
const DEFAULT_LIFETIME_DAYS: i64 = 90;

/// `POST /v1/scim/credentials`.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct IssueRequest {
    /// What an operator recognises it by when deciding to rotate.
    pub label: String,
    /// How long it lives. Clamped to [`MAX_LIFETIME_DAYS`].
    #[serde(default)]
    pub expires_in_days: Option<i64>,
}

/// The issue response — **the only time the token is ever readable**.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct IssuedCredential {
    /// The credential's record.
    #[serde(flatten)]
    pub credential: ScimCredentialView,
    /// The value to paste into Entra's "Secret Token" or Okta's
    /// authorisation header. Never stored, never logged, shown once.
    pub token: String,
}

/// The non-secret provisioning credential metadata exposed after issuance.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ScimCredentialView {
    #[schema(value_type = String, format = "uuid")]
    id: ScimCredentialId,
    label: String,
    expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
    last_used_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    created_by: String,
}

impl From<ScimCredential> for ScimCredentialView {
    fn from(credential: ScimCredential) -> Self {
        Self {
            id: credential.id,
            label: credential.label,
            expires_at: credential.expires_at,
            revoked_at: credential.revoked_at,
            last_used_at: credential.last_used_at,
            created_at: credential.created_at,
            created_by: credential.created_by,
        }
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ScimCredentialsResponse {
    credentials: Vec<ScimCredentialView>,
}

/// Issues a credential.
#[utoipa::path(
    post,
    path = "/v1/scim/credentials",
    operation_id = "issue_scim_credential",
    tag = "directory",
    request_body = IssueRequest,
    responses(
        (status = 201, description = "Credential metadata and its one-time token", body = IssuedCredential),
        (status = 400, description = "The label or lifetime is invalid", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Directory credential management is not permitted", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "scim.credential.issue", skip_all)]
pub(crate) async fn issue(
    State(state): State<AppState>,
    payload: std::result::Result<Json<IssueRequest>, JsonRejection>,
) -> Response {
    let result = async {
        let request = body(payload)?;
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let authorized = authz::require(
            &state,
            &mut tx,
            Action::DirectoryManage,
            Resource::Tenant(tenant_id),
            None,
        )
        .await?;

        let expires_at = resolved_expiry(Utc::now(), request.expires_in_days);
        let minted = synveda_identity::scim::mint(tenant_id)?;
        let subject = synveda_identity::current_tenant()
            .map(|context| context.claims.subject.clone())
            .unwrap_or_default();
        let credential = directory::issue_credential(
            &mut *tx,
            ScimCredentialId::new(),
            tenant_id,
            &minted.hash,
            &request.label,
            expires_at,
            &subject,
        )
        .await?;
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::ScimCredentialIssued,
            format!("tenant {tenant_id}"),
            Outcome::Success,
            json!({
                "credential": {
                    "id": credential.id,
                    "label": credential.label,
                    "expires_at": credential.expires_at,
                },
                "authz": audit::decision_context(Action::DirectoryManage, &authorized),
            }),
        )
        .await?;
        commit(tx).await?;

        tracing::info!(
            tenant.id = %tenant_id,
            scim.credential = %credential.id,
            expires_at = %credential.expires_at,
            "provisioning credential issued"
        );
        Ok((
            StatusCode::CREATED,
            Json(IssuedCredential {
                credential: credential.into(),
                token: minted.token,
            }),
        ))
    }
    .await;
    respond(&state, "issue", result).await
}

/// `GET /v1/scim/credentials` — the inventory, revoked and expired ones
/// included, because rotation is a decision about a history rather than
/// about a current state.
#[utoipa::path(
    get,
    path = "/v1/scim/credentials",
    operation_id = "list_scim_credentials",
    tag = "directory",
    responses(
        (status = 200, description = "Provisioning credential inventory", body = ScimCredentialsResponse),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Directory credential management is not permitted", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "scim.credential.list", skip_all)]
pub(crate) async fn list(State(state): State<AppState>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        authz::require(
            &state,
            &mut tx,
            Action::DirectoryManage,
            Resource::Tenant(tenant_id),
            None,
        )
        .await?;
        let credentials = directory::credentials(&mut *tx, tenant_id).await?;
        commit(tx).await?;
        Ok(Json(ScimCredentialsResponse {
            credentials: credentials.into_iter().map(Into::into).collect(),
        }))
    }
    .await;
    respond(&state, "list", result).await
}

/// `POST /v1/scim/credentials/{id}/revoke`.
///
/// A stamp rather than a delete: which credential sealed which identity
/// has to stay answerable from the chain after the credential is gone.
#[utoipa::path(
    post,
    path = "/v1/scim/credentials/{id}/revoke",
    operation_id = "revoke_scim_credential",
    tag = "directory",
    params(("id" = String, Path, format = "uuid")),
    responses(
        (status = 204, description = "The provisioning credential was revoked"),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Directory credential management is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "The provisioning credential is absent or outside the tenant", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "scim.credential.revoke", skip_all, fields(scim.credential = %id))]
pub(crate) async fn revoke(
    State(state): State<AppState>,
    Path(id): Path<ScimCredentialId>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let authorized = authz::require(
            &state,
            &mut tx,
            Action::DirectoryManage,
            Resource::Tenant(tenant_id),
            None,
        )
        .await?;
        // Uniform 404 (ADR-0018 decision 3's rule): an unknown id and one
        // belonging to another tenant answer identically, so the route is
        // no existence oracle.
        if !directory::revoke_credential(&mut *tx, tenant_id, id).await? {
            return Err(Error::NotFound {
                entity: format!("provisioning credential {id}"),
            });
        }
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::ScimCredentialRevoked,
            format!("tenant {tenant_id}"),
            Outcome::Success,
            json!({"credential": {"id": id}, "authz": audit::decision_context(Action::DirectoryManage, &authorized)}),
        )
        .await?;
        commit(tx).await?;
        Ok(StatusCode::NO_CONTENT)
    }
    .await;
    respond(&state, "revoke", result).await
}

/// Counts the operation and renders the result — the taxonomy every other
/// admin plane uses, so a denial here looks like a denial anywhere else.
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
    metrics::counter!(SCIM_CREDENTIAL_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome)
        .increment(1);
    match result {
        Ok(response) => response.into_response(),
        Err(error) => {
            audit::record_rejection(state, op, &error).await;
            ApiError(error).into_response()
        }
    }
}

#[must_use]
fn resolved_expiry(now: DateTime<Utc>, requested_days: Option<i64>) -> DateTime<Utc> {
    now + Duration::days(
        requested_days
            .unwrap_or(DEFAULT_LIFETIME_DAYS)
            .clamp(1, MAX_LIFETIME_DAYS),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_credential_cannot_be_issued_past_the_cap_or_into_the_past() {
        let now = DateTime::parse_from_rfc3339("2026-08-05T00:00:00Z")
            .expect("parse")
            .with_timezone(&Utc);
        assert_eq!(resolved_expiry(now, None), now + Duration::days(90));
        assert_eq!(
            resolved_expiry(now, Some(10_000)),
            now + Duration::days(MAX_LIFETIME_DAYS)
        );
        // A zero or negative window would issue a credential that never
        // authenticates, which the schema refuses anyway — clamped here so
        // the refusal is a value rather than a 500.
        assert_eq!(resolved_expiry(now, Some(0)), now + Duration::days(1));
        assert_eq!(resolved_expiry(now, Some(-5)), now + Duration::days(1));
    }
}
