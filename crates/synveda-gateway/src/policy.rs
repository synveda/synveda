//! The policy admin API (AUTHZ-2, ADR-0014 decision 8): pack listing, the
//! tenant default, and per-node assignments on `/v1/policy/*` and
//! `/v1/admin/scopes/{scope_id}/policy`. Behind tenant resolution like every
//! `/v1` route, uniform-404 ownership first, then the PDP
//! (`PolicyRead`/`PolicyAssign`) — decided under the pack currently
//! effective at the target, like every governed action.
//!
//! Audited since AUD-1 (ADR-0019): assignment and default mutations chain
//! their semantic events in their own transactions; reads chain their
//! allowed decision; denials chain at the `respond` seam.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::postgres::PgConnection;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{
    Action, EMBEDDED_PACKS, EffectivePack, PackOrigin, REGULATED_STRICT, Resource,
};
use synveda_store::{policy_assignments, policy_packs, rls, scopes};
use synveda_types::{Error, Result, ScopeId, TenantId};

use crate::app::AppState;
use crate::audit;
use crate::authz;
use crate::error::ApiError;
use crate::request::{body, commit, found, tenant_id};
use crate::telemetry::POLICY_OPERATIONS_TOTAL;

/// Counts the operation and renders the result — the same outcome
/// taxonomy as the hierarchy routes: `ok`, `rejected` (the caller's
/// fault), `error` (ours or an operator's). Error-path audit events chain
/// here (AUD-1, ADR-0019 decision 5).
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
    metrics::counter!(POLICY_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    match result {
        Ok(response) => response.into_response(),
        Err(error) => {
            audit::record_rejection(state, op, &error).await;
            ApiError(error).into_response()
        }
    }
}

/// The allowed-read decision event (ADR-0019 decision 4) — reads commit
/// their transactions since AUD-1.
pub(crate) async fn read_event(
    tx: &mut PgConnection,
    tenant_id: TenantId,
    op: &'static str,
    resource: Resource,
    authorized: &authz::Authorized,
) -> Result<()> {
    audit::record(
        tx,
        tenant_id,
        AuditAction::AuthzDecision,
        resource.to_string(),
        Outcome::Allow,
        json!({"op": op, "authz": audit::decision_context(Action::PolicyRead, authorized)}),
    )
    .await
    .map(|_| ())
}

/// A pack's name must denote something decisions can resolve: an embedded
/// product pack or one of the tenant's stored packs. Checked *after* the
/// PDP has admitted the caller, so a denied caller learns nothing about
/// which names exist.
async fn known_pack(conn: &mut PgConnection, tenant_id: TenantId, name: &str) -> Result<()> {
    if EMBEDDED_PACKS.iter().any(|(pack, _)| *pack == name) {
        return Ok(());
    }
    if policy_packs::get(&mut *conn, tenant_id, name)
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
    name: String,
    version: i64,
    /// `embedded` (compiled into the binary) or `stored` (a tenant row).
    #[schema(value_type = String)]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_at: Option<DateTime<Utc>>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub(crate) struct PacksResponse {
    packs: Vec<PackSummary>,
}

/// `GET /v1/policy/packs` — the packs assignable in this tenant: the
/// embedded product packs and the tenant's stored packs.
#[utoipa::path(
    get,
    path = "/v1/policy/packs",
    operation_id = "list_policy_packs",
    tag = "policy",
    responses(
        (status = 200, description = "Embedded and tenant-stored policy packs", body = PacksResponse),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Policy metadata is not visible to the caller", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn packs(State(state): State<AppState>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let authorized = authz::require(
            &state,
            &mut tx,
            Action::PolicyRead,
            Resource::Tenant(tenant_id),
            None,
        )
        .await?;
        let mut packs: Vec<PackSummary> = EMBEDDED_PACKS
            .iter()
            .map(|(name, version)| PackSummary {
                name: (*name).to_owned(),
                version: *version,
                kind: "embedded",
                updated_at: None,
            })
            .collect();
        packs.extend(
            policy_packs::stored(&mut *tx, tenant_id)
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
            tenant_id,
            "packs",
            Resource::Tenant(tenant_id),
            &authorized,
        )
        .await?;
        commit(tx).await?;
        Ok(Json(PacksResponse { packs }))
    }
    .await;
    respond(&state, "packs", result).await
}

#[derive(Serialize, utoipa::ToSchema)]
pub(crate) struct DefaultResponse {
    /// The stored tenant default, when one exists.
    pack_name: Option<String>,
    /// What applies where nothing is assigned: the stored default, or the
    /// embedded `regulated-strict` (seed §2.1).
    effective: String,
}

/// `GET /v1/policy/default` — the tenant's default pack.
#[utoipa::path(
    get,
    path = "/v1/policy/default",
    operation_id = "get_default_policy",
    tag = "policy",
    responses(
        (status = 200, description = "Stored and effective tenant policy defaults", body = DefaultResponse),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Policy metadata is not visible to the caller", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn get_default(State(state): State<AppState>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let authorized = authz::require(
            &state,
            &mut tx,
            Action::PolicyRead,
            Resource::Tenant(tenant_id),
            None,
        )
        .await?;
        let pack_name = policy_assignments::default_pack(&mut *tx, tenant_id).await?;
        let effective = pack_name
            .clone()
            .unwrap_or_else(|| REGULATED_STRICT.to_owned());
        read_event(
            &mut tx,
            tenant_id,
            "get_default",
            Resource::Tenant(tenant_id),
            &authorized,
        )
        .await?;
        commit(tx).await?;
        Ok(Json(DefaultResponse {
            pack_name,
            effective,
        }))
    }
    .await;
    respond(&state, "get_default", result).await
}

#[derive(Deserialize, utoipa::ToSchema)]
pub(crate) struct SetPackBody {
    name: String,
}

/// `PUT /v1/policy/default` — set the tenant default pack.
#[utoipa::path(
    put,
    path = "/v1/policy/default",
    operation_id = "set_default_policy",
    tag = "policy",
    request_body = SetPackBody,
    responses(
        (status = 200, description = "The resulting tenant policy default", body = DefaultResponse),
        (status = 400, description = "The pack name is unknown", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Policy assignment is not permitted", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn set_default(
    State(state): State<AppState>,
    payload: std::result::Result<Json<SetPackBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let authorized = authz::require(
            &state,
            &mut tx,
            Action::PolicyAssign,
            Resource::Tenant(tenant_id),
            None,
        )
        .await?;
        known_pack(&mut tx, tenant_id, &body.name).await?;
        policy_assignments::set_default(&mut *tx, tenant_id, &body.name).await?;
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::PolicyDefaultSet,
            Resource::Tenant(tenant_id).to_string(),
            Outcome::Success,
            json!({
                "authz": audit::decision_context(Action::PolicyAssign, &authorized),
                "pack": body.name,
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(DefaultResponse {
            pack_name: Some(body.name.clone()),
            effective: body.name,
        }))
    }
    .await;
    respond(&state, "set_default", result).await
}

/// `DELETE /v1/policy/default` — clear the tenant default; the embedded
/// `regulated-strict` applies wherever nothing is assigned.
#[utoipa::path(
    delete,
    path = "/v1/policy/default",
    operation_id = "clear_default_policy",
    tag = "policy",
    responses(
        (status = 204, description = "The stored tenant default was cleared"),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Policy assignment is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "No stored tenant default exists", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn clear_default(State(state): State<AppState>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let authorized = authz::require(
            &state,
            &mut tx,
            Action::PolicyAssign,
            Resource::Tenant(tenant_id),
            None,
        )
        .await?;
        if !policy_assignments::clear_default(&mut *tx, tenant_id).await? {
            return Err(Error::NotFound {
                entity: "tenant default pack".to_owned(),
            });
        }
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::PolicyDefaultCleared,
            Resource::Tenant(tenant_id).to_string(),
            Outcome::Success,
            json!({"authz": audit::decision_context(Action::PolicyAssign, &authorized)}),
        )
        .await?;
        commit(tx).await?;
        Ok(StatusCode::NO_CONTENT)
    }
    .await;
    respond(&state, "clear_default", result).await
}

/// Where an inherited thing came from.
///
/// `pub(crate)` since CNSL-2 (ADR-0058 decision 6): the capabilities probe
/// and the effective-roles listing both report an origin, and the whole
/// point of that decision is that the admin planes say "this came from
/// above" in **one** vocabulary rather than three that agree on the day
/// they are written.
#[derive(Serialize, utoipa::ToSchema)]
pub(crate) struct OriginView {
    #[schema(value_type = String)]
    pub(crate) kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub(crate) scope_id: Option<ScopeId>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub(crate) struct PolicyAssignmentView {
    #[schema(value_type = String, format = "uuid")]
    scope_id: ScopeId,
    pack_name: String,
    updated_at: DateTime<Utc>,
}

impl From<synveda_types::PolicyAssignment> for PolicyAssignmentView {
    fn from(value: synveda_types::PolicyAssignment) -> Self {
        Self {
            scope_id: value.scope_id,
            pack_name: value.pack_name,
            updated_at: value.updated_at,
        }
    }
}

#[derive(Serialize, utoipa::ToSchema)]
pub(crate) struct EffectiveResponse {
    name: String,
    version: i64,
    origin: OriginView,
    /// The node's own assignment row, when it carries one.
    assignment: Option<PolicyAssignmentView>,
}

pub(crate) fn origin_view(effective: &EffectivePack) -> OriginView {
    match effective.origin {
        PackOrigin::Assigned(scope_id) => OriginView {
            kind: "assigned",
            scope_id: Some(scope_id),
        },
        PackOrigin::TenantDefault => OriginView {
            kind: "tenant-default",
            scope_id: None,
        },
        PackOrigin::Default => OriginView {
            kind: "default",
            scope_id: None,
        },
        PackOrigin::Fallback => OriginView {
            kind: "fallback",
            scope_id: None,
        },
    }
}

/// `GET /v1/admin/scopes/{scope_id}/policy` — the pack effective at the scope
/// and where it came from (its own assignment, an ancestor's, the tenant
/// default, or the embedded default).
#[utoipa::path(
    get,
    path = "/v1/admin/scopes/{scope_id}/policy",
    operation_id = "get_scope_policy",
    tag = "policy",
    params(("scope_id" = String, Path, format = "uuid")),
    responses(
        (status = 200, description = "The effective pack, origin and direct assignment", body = EffectiveResponse),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Policy metadata is not visible to the caller", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "The scope is absent or outside the tenant", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn get_scope_policy(
    State(state): State<AppState>,
    Path(id): Path<ScopeId>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let scope = found(scopes::get(&mut *tx, tenant_id, id).await?, tenant_id, id)?;
        let input = authz::gather(
            &state,
            &mut tx,
            Some(&scope),
            synveda_store::anchors::AnchorSelection::none(),
            Vec::new(),
        )
        .await?;
        let authorized = authz::decide(&state, &input, Action::PolicyRead, Resource::Scope(id))?;
        let effective = state
            .pdp
            .effective(tenant_id, Resource::Scope(id), &input.context());
        let assignment = input
            .assignments
            .iter()
            .find(|assignment| assignment.scope_id == id)
            .cloned();
        read_event(
            &mut tx,
            tenant_id,
            "get_scope_policy",
            Resource::Scope(id),
            &authorized,
        )
        .await?;
        commit(tx).await?;
        Ok(Json(EffectiveResponse {
            origin: origin_view(&effective),
            name: effective.name,
            version: effective.version,
            assignment: assignment.map(Into::into),
        }))
    }
    .await;
    respond(&state, "get_scope_policy", result).await
}

/// `PUT /v1/admin/scopes/{scope_id}/policy` — assign a pack at the scope; its
/// subtree runs it from the next request on.
#[utoipa::path(
    put,
    path = "/v1/admin/scopes/{scope_id}/policy",
    operation_id = "assign_scope_policy",
    tag = "policy",
    params(("scope_id" = String, Path, format = "uuid")),
    request_body = SetPackBody,
    responses(
        (status = 200, description = "The resulting direct policy assignment", body = PolicyAssignmentView),
        (status = 400, description = "The pack name is unknown", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Policy assignment is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "The scope is absent or outside the tenant", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn assign_scope_policy(
    State(state): State<AppState>,
    Path(id): Path<ScopeId>,
    payload: std::result::Result<Json<SetPackBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let scope = found(scopes::get(&mut *tx, tenant_id, id).await?, tenant_id, id)?;
        let authorized = authz::require(
            &state,
            &mut tx,
            Action::PolicyAssign,
            Resource::Scope(id),
            Some(&scope),
        )
        .await?;
        known_pack(&mut tx, tenant_id, &body.name).await?;
        let assignment = policy_assignments::assign(&mut *tx, tenant_id, id, &body.name).await?;
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::PolicyNodeAssigned,
            Resource::Scope(id).to_string(),
            Outcome::Success,
            json!({
                "authz": audit::decision_context(Action::PolicyAssign, &authorized),
                "pack": body.name,
                "scope": {"slug": scope.slug},
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(PolicyAssignmentView::from(assignment)))
    }
    .await;
    respond(&state, "assign_scope_policy", result).await
}

/// `DELETE /v1/admin/scopes/{scope_id}/policy` — remove the scope's
/// assignment; it falls back to the inherited pack.
#[utoipa::path(
    delete,
    path = "/v1/admin/scopes/{scope_id}/policy",
    operation_id = "unassign_scope_policy",
    tag = "policy",
    params(("scope_id" = String, Path, format = "uuid")),
    responses(
        (status = 204, description = "The direct policy assignment was removed"),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Policy assignment is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "The scope or direct assignment does not exist", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn unassign_scope_policy(
    State(state): State<AppState>,
    Path(id): Path<ScopeId>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let scope = found(scopes::get(&mut *tx, tenant_id, id).await?, tenant_id, id)?;
        let authorized = authz::require(
            &state,
            &mut tx,
            Action::PolicyAssign,
            Resource::Scope(id),
            Some(&scope),
        )
        .await?;
        if !policy_assignments::unassign(&mut *tx, tenant_id, id).await? {
            return Err(Error::NotFound {
                entity: format!("pack assignment on scope {id}"),
            });
        }
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::PolicyNodeUnassigned,
            Resource::Scope(id).to_string(),
            Outcome::Success,
            json!({
                "authz": audit::decision_context(Action::PolicyAssign, &authorized),
                "scope": {"slug": scope.slug},
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(StatusCode::NO_CONTENT)
    }
    .await;
    respond(&state, "unassign_scope_policy", result).await
}
