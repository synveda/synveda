//! The role admin API (AUTHZ-3, ADR-0015 decision 7): tenant bindings on
//! `/v1/roles/bindings` and per-node bindings on
//! `/v1/hierarchy/nodes/{id}/roles`. Behind tenant resolution like every
//! `/v1` route, uniform-404 ownership first, then the PDP
//! (`RoleRead`/`RoleAssign`, with the granted-or-revoked role in context
//! so the base layer's escalation guard decides — ADR-0015 decision 5).
//!
//! AUD-1 wiring point: binding mutations are audit emission points; until
//! the hash-chained log lands they are visible in traces and
//! `synveda_role_operations_total` (and every PDP decision in the
//! decision log and `synveda_authz_decisions_total`).

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use synveda_policy::{Action, Resource};
use synveda_store::{rls, role_bindings};
use synveda_types::{Error, Result, Role, RoleBinding, ScopeId};

use crate::app::AppState;
use crate::authz;
use crate::error::ApiError;
use crate::hierarchy::{body, commit, found, tenant_id};
use crate::telemetry::ROLE_OPERATIONS_TOTAL;

/// Counts the operation and renders the result — the same outcome
/// taxonomy as the hierarchy and policy routes: `ok`, `rejected` (the
/// caller's fault), `error` (ours or an operator's).
fn respond<T: IntoResponse>(op: &'static str, result: Result<T>) -> Response {
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
    metrics::counter!(ROLE_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    match result {
        Ok(response) => response.into_response(),
        Err(error) => ApiError(error).into_response(),
    }
}

/// A binding named by a mutation request: who and which role. `Role`
/// deserialises from the closed vocabulary, so an unknown role is a 400
/// before anything else runs.
#[derive(Deserialize)]
pub(crate) struct BindingBody {
    subject: String,
    role: Role,
}

#[derive(Serialize)]
struct BindingsResponse {
    bindings: Vec<RoleBinding>,
}

/// `GET /v1/roles/bindings` — every binding of the tenant: the "who holds
/// what where" view. A tenant-plane read: requires `RoleRead` at the
/// tenant, i.e. a tenant-wide steward/org-admin/auditor.
pub(crate) async fn list(State(state): State<AppState>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        authz::require(
            &state,
            &mut tx,
            Action::RoleRead,
            Resource::Tenant(tenant_id),
            None,
        )
        .await?;
        let bindings = role_bindings::all(&mut *tx, tenant_id).await?;
        Ok(Json(BindingsResponse { bindings }))
    }
    .await;
    respond("list", result)
}

/// `PUT /v1/roles/bindings` — bind a role tenant-wide (in force
/// everywhere, the tenant plane included — ADR-0015 decision 2).
pub(crate) async fn bind_tenant_wide(
    State(state): State<AppState>,
    payload: std::result::Result<Json<BindingBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        require_assign(
            &state,
            &mut tx,
            Resource::Tenant(tenant_id),
            None,
            body.role,
        )
        .await?;
        let binding =
            role_bindings::bind(&mut *tx, tenant_id, &body.subject, None, body.role).await?;
        commit(tx).await?;
        Ok(Json(binding))
    }
    .await;
    respond("bind", result)
}

/// `DELETE /v1/roles/bindings?subject=…&role=…` — remove a tenant-wide
/// binding. Revocation is gated like granting: the base guard covers
/// revoking org-admin too.
pub(crate) async fn unbind_tenant_wide(
    State(state): State<AppState>,
    query: std::result::Result<Query<BindingBody>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let result = async {
        let Query(params) = query.map_err(|rejection| Error::Invalid {
            message: rejection.to_string(),
        })?;
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        require_assign(
            &state,
            &mut tx,
            Resource::Tenant(tenant_id),
            None,
            params.role,
        )
        .await?;
        if !role_bindings::unbind(&mut *tx, tenant_id, &params.subject, None, params.role).await? {
            return Err(Error::NotFound {
                entity: "tenant-wide role binding".to_owned(),
            });
        }
        commit(tx).await?;
        Ok(StatusCode::NO_CONTENT)
    }
    .await;
    respond("unbind", result)
}

/// `GET /v1/hierarchy/nodes/{id}/roles` — the bindings at one node.
pub(crate) async fn list_node(State(state): State<AppState>, Path(id): Path<ScopeId>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let node = found(
            synveda_store::hierarchy::node(&mut *tx, id).await?,
            tenant_id,
            id,
        )?;
        authz::require(
            &state,
            &mut tx,
            Action::RoleRead,
            Resource::Scope(id),
            Some(&node),
        )
        .await?;
        let bindings = role_bindings::for_scope(&mut *tx, tenant_id, id).await?;
        Ok(Json(BindingsResponse { bindings }))
    }
    .await;
    respond("list_node", result)
}

/// `PUT /v1/hierarchy/nodes/{id}/roles` — bind a role at the node; its
/// subtree holds it from the next request on.
pub(crate) async fn bind_node(
    State(state): State<AppState>,
    Path(id): Path<ScopeId>,
    payload: std::result::Result<Json<BindingBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let node = found(
            synveda_store::hierarchy::node(&mut *tx, id).await?,
            tenant_id,
            id,
        )?;
        require_assign(&state, &mut tx, Resource::Scope(id), Some(&node), body.role).await?;
        let binding =
            role_bindings::bind(&mut *tx, tenant_id, &body.subject, Some(id), body.role).await?;
        commit(tx).await?;
        Ok(Json(binding))
    }
    .await;
    respond("bind_node", result)
}

/// `DELETE /v1/hierarchy/nodes/{id}/roles?subject=…&role=…` — remove one
/// binding at the node.
pub(crate) async fn unbind_node(
    State(state): State<AppState>,
    Path(id): Path<ScopeId>,
    query: std::result::Result<Query<BindingBody>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let result = async {
        let Query(params) = query.map_err(|rejection| Error::Invalid {
            message: rejection.to_string(),
        })?;
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let node = found(
            synveda_store::hierarchy::node(&mut *tx, id).await?,
            tenant_id,
            id,
        )?;
        require_assign(
            &state,
            &mut tx,
            Resource::Scope(id),
            Some(&node),
            params.role,
        )
        .await?;
        if !role_bindings::unbind(&mut *tx, tenant_id, &params.subject, Some(id), params.role)
            .await?
        {
            return Err(Error::NotFound {
                entity: format!("role binding on scope {id}"),
            });
        }
        commit(tx).await?;
        Ok(StatusCode::NO_CONTENT)
    }
    .await;
    respond("unbind_node", result)
}

/// Authorizes a binding mutation: `RoleAssign` on the resource with the
/// granted-or-revoked role in context (ADR-0015 decision 5).
async fn require_assign(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    resource: Resource,
    anchor: Option<&synveda_types::HierarchyNode>,
    grant: Role,
) -> Result<()> {
    let input = authz::gather(state, tx, anchor).await?;
    let mut context = input.context();
    context.grant = Some(grant);
    state
        .pdp
        .require(&input.principal, Action::RoleAssign, resource, &context)
}
