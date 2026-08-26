//! Cedar pack catalogue (AUTHZ-2, CPR-30, ADR-0014, ADR-0089).
//!
//! Pack source and metadata remain policy artifacts. Runtime selection moved
//! whole to governed Configuration versions and bindings; this module exposes
//! no assignment/default mutation path.

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use sqlx::PgConnection;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{Action, EMBEDDED_PACKS, EffectivePack, PackOrigin, Resource};
use synveda_store::{policy_packs, rls};
use synveda_types::{Error, Result, ScopeId, TenantId};

use crate::app::AppState;
use crate::audit;
use crate::authz;
use crate::request::{commit, tenant_id};
use crate::telemetry::POLICY_OPERATIONS_TOTAL;

async fn respond<T: IntoResponse>(
    state: &AppState,
    operation: &'static str,
    result: Result<T>,
) -> Response {
    let outcome = crate::response::outcome(&result);
    metrics::counter!(POLICY_OPERATIONS_TOTAL, "op" => operation, "outcome" => outcome)
        .increment(1);
    crate::response::finish(state, operation, result).await
}

/// Shared allowed-read event used by scope administration.
pub(crate) async fn read_event(
    tx: &mut PgConnection,
    tenant: TenantId,
    operation: &'static str,
    resource: Resource,
    authorized: &authz::Authorized,
) -> Result<()> {
    audit::record(
        tx,
        tenant,
        AuditAction::AuthzDecision,
        resource.to_string(),
        Outcome::Allow,
        json!({
            "op": operation,
            "authz": audit::decision_context(Action::PolicyRead, authorized),
        }),
    )
    .await
    .map(|_| ())
}

/// Validate that a document's pack selector resolves to embedded or
/// tenant-owned source. Called after ConfigurationWrite authorisation.
pub(crate) async fn known_pack(
    connection: &mut PgConnection,
    tenant: TenantId,
    name: &str,
) -> Result<()> {
    if EMBEDDED_PACKS.iter().any(|(pack, _)| *pack == name)
        || policy_packs::get(&mut *connection, tenant, name)
            .await?
            .is_some()
    {
        return Ok(());
    }
    Err(Error::Invalid {
        message: format!(
            "unknown pack {name:?}: not an embedded product pack or a stored pack of this tenant"
        ),
    })
}

#[derive(Serialize, utoipa::ToSchema)]
pub(crate) struct PackSummary {
    pub(crate) name: String,
    pub(crate) version: i64,
    #[schema(value_type = String)]
    pub(crate) kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) updated_at: Option<DateTime<Utc>>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub(crate) struct PacksResponse {
    pub(crate) packs: Vec<PackSummary>,
}

/// List immutable pack sources available to Configuration documents.
#[utoipa::path(
    get,
    path = "/v1/policy/packs",
    operation_id = "list_policy_packs",
    tag = "policy",
    responses(
        (status = 200, description = "Embedded and tenant-stored policy packs", body = PacksResponse),
        (status = 403, description = "Policy metadata is not visible", body = crate::workspaces::ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "policy.packs", skip_all)]
pub(crate) async fn packs(State(state): State<AppState>) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let authorized = authz::require(
            &state,
            &mut tx,
            Action::PolicyRead,
            Resource::Tenant(tenant),
            None,
        )
        .await?;
        let mut packs = EMBEDDED_PACKS
            .iter()
            .map(|(name, version)| PackSummary {
                name: (*name).to_owned(),
                version: *version,
                kind: "embedded",
                updated_at: None,
            })
            .collect::<Vec<_>>();
        packs.extend(
            policy_packs::stored(&mut *tx, tenant)
                .await?
                .into_iter()
                .map(|pack| PackSummary {
                    name: pack.name,
                    version: pack.version,
                    kind: "stored",
                    updated_at: Some(pack.updated_at),
                }),
        );
        read_event(
            &mut tx,
            tenant,
            "policy.packs",
            Resource::Tenant(tenant),
            &authorized,
        )
        .await?;
        commit(tx).await?;
        Ok(Json(PacksResponse { packs }))
    }
    .await;
    respond(&state, "packs", result).await
}

/// Stable API origin vocabulary retained by the capability surface. These
/// values describe the PDP's derived selector, not a mutable assignment row.
#[derive(Serialize, utoipa::ToSchema)]
pub(crate) struct OriginView {
    #[schema(value_type = String)]
    pub(crate) kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub(crate) scope_id: Option<ScopeId>,
}

pub(crate) fn origin_view(effective: &EffectivePack) -> OriginView {
    match effective.origin {
        PackOrigin::Assigned(scope_id) => OriginView {
            kind: "configuration-binding",
            scope_id: Some(scope_id),
        },
        PackOrigin::TenantDefault => OriginView {
            kind: "tenant-configuration",
            scope_id: None,
        },
        PackOrigin::Default => OriginView {
            kind: "fail-safe",
            scope_id: None,
        },
        PackOrigin::Fallback => OriginView {
            kind: "fallback",
            scope_id: None,
        },
    }
}
