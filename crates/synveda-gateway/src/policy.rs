//! The policy admin API (AUTHZ-2, ADR-0014 decision 8): pack listing, the
//! tenant default, and per-node assignments on `/v1/policy/*` and
//! `/v1/hierarchy/nodes/{id}/policy`. Behind tenant resolution like every
//! `/v1` route, uniform-404 ownership first, then the PDP
//! (`PolicyRead`/`PolicyAssign`) — decided under the pack currently
//! effective at the target, like every governed action.
//!
//! AUD-1 wiring point: assignment and default mutations are audit
//! emission points; until the hash-chained log lands they are visible in
//! traces and `synveda_policy_operations_total` (and every PDP decision in
//! the decision log and `synveda_authz_decisions_total`).

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgConnection;
use synveda_policy::{
    Action, EMBEDDED_PACKS, EffectivePack, PackOrigin, REGULATED_STRICT, Resource,
};
use synveda_store::{policy_assignments, policy_packs, rls};
use synveda_types::{Error, Result, ScopeId, TenantId};

use crate::app::AppState;
use crate::authz;
use crate::error::ApiError;
use crate::hierarchy::{body, commit, found, tenant_id};
use crate::telemetry::POLICY_OPERATIONS_TOTAL;

/// Counts the operation and renders the result — the same outcome
/// taxonomy as the hierarchy routes: `ok`, `rejected` (the caller's
/// fault), `error` (ours or an operator's).
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
    metrics::counter!(POLICY_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    match result {
        Ok(response) => response.into_response(),
        Err(error) => ApiError(error).into_response(),
    }
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

#[derive(Serialize)]
struct PackSummary {
    name: String,
    version: i64,
    /// `embedded` (compiled into the binary) or `stored` (a tenant row).
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_at: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
struct PacksResponse {
    packs: Vec<PackSummary>,
}

/// `GET /v1/policy/packs` — the packs assignable in this tenant: the
/// embedded product packs and the tenant's stored packs.
pub(crate) async fn packs(State(state): State<AppState>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        authz::require(
            &state.pdp,
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
        Ok(Json(PacksResponse { packs }))
    }
    .await;
    respond("packs", result)
}

#[derive(Serialize)]
struct DefaultResponse {
    /// The stored tenant default, when one exists.
    pack_name: Option<String>,
    /// What applies where nothing is assigned: the stored default, or the
    /// embedded `regulated-strict` (seed §2.1).
    effective: String,
}

/// `GET /v1/policy/default` — the tenant's default pack.
pub(crate) async fn get_default(State(state): State<AppState>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        authz::require(
            &state.pdp,
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
        Ok(Json(DefaultResponse {
            pack_name,
            effective,
        }))
    }
    .await;
    respond("get_default", result)
}

#[derive(Deserialize)]
pub(crate) struct SetPackBody {
    name: String,
}

/// `PUT /v1/policy/default` — set the tenant default pack.
pub(crate) async fn set_default(
    State(state): State<AppState>,
    payload: std::result::Result<Json<SetPackBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        authz::require(
            &state.pdp,
            &mut tx,
            Action::PolicyAssign,
            Resource::Tenant(tenant_id),
            None,
        )
        .await?;
        known_pack(&mut tx, tenant_id, &body.name).await?;
        policy_assignments::set_default(&mut *tx, tenant_id, &body.name).await?;
        commit(tx).await?;
        Ok(Json(DefaultResponse {
            pack_name: Some(body.name.clone()),
            effective: body.name,
        }))
    }
    .await;
    respond("set_default", result)
}

/// `DELETE /v1/policy/default` — clear the tenant default; the embedded
/// `regulated-strict` applies wherever nothing is assigned.
pub(crate) async fn clear_default(State(state): State<AppState>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        authz::require(
            &state.pdp,
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
        commit(tx).await?;
        Ok(StatusCode::NO_CONTENT)
    }
    .await;
    respond("clear_default", result)
}

#[derive(Serialize)]
struct OriginResponse {
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope_id: Option<ScopeId>,
}

#[derive(Serialize)]
struct EffectiveResponse {
    name: String,
    version: i64,
    origin: OriginResponse,
    /// The node's own assignment row, when it carries one.
    assignment: Option<synveda_types::PolicyAssignment>,
}

fn origin_response(effective: &EffectivePack) -> OriginResponse {
    match effective.origin {
        PackOrigin::Assigned(scope_id) => OriginResponse {
            kind: "assigned",
            scope_id: Some(scope_id),
        },
        PackOrigin::TenantDefault => OriginResponse {
            kind: "tenant-default",
            scope_id: None,
        },
        PackOrigin::Default => OriginResponse {
            kind: "default",
            scope_id: None,
        },
        PackOrigin::Fallback => OriginResponse {
            kind: "fallback",
            scope_id: None,
        },
    }
}

/// `GET /v1/hierarchy/nodes/{id}/policy` — the pack effective at the node
/// and where it came from (its own assignment, an ancestor's, the tenant
/// default, or the embedded default).
pub(crate) async fn get_node_policy(
    State(state): State<AppState>,
    Path(id): Path<ScopeId>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let node = found(
            synveda_store::hierarchy::node(&mut *tx, id).await?,
            tenant_id,
            id,
        )?;
        let input = authz::gather(&mut tx, Some(&node)).await?;
        state.pdp.require(
            &input.principal,
            Action::PolicyRead,
            Resource::Scope(id),
            &input.context(),
        )?;
        let effective = state
            .pdp
            .effective(tenant_id, Resource::Scope(id), &input.context());
        let assignment = input
            .assignments
            .iter()
            .find(|assignment| assignment.scope_id == id)
            .cloned();
        Ok(Json(EffectiveResponse {
            origin: origin_response(&effective),
            name: effective.name,
            version: effective.version,
            assignment,
        }))
    }
    .await;
    respond("get_node_policy", result)
}

/// `PUT /v1/hierarchy/nodes/{id}/policy` — assign a pack at the node; its
/// subtree runs it from the next request on.
pub(crate) async fn assign_node_policy(
    State(state): State<AppState>,
    Path(id): Path<ScopeId>,
    payload: std::result::Result<Json<SetPackBody>, JsonRejection>,
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
        authz::require(
            &state.pdp,
            &mut tx,
            Action::PolicyAssign,
            Resource::Scope(id),
            Some(&node),
        )
        .await?;
        known_pack(&mut tx, tenant_id, &body.name).await?;
        let assignment = policy_assignments::assign(&mut *tx, tenant_id, id, &body.name).await?;
        commit(tx).await?;
        Ok(Json(assignment))
    }
    .await;
    respond("assign_node_policy", result)
}

/// `DELETE /v1/hierarchy/nodes/{id}/policy` — remove the node's
/// assignment; it falls back to the inherited pack.
pub(crate) async fn unassign_node_policy(
    State(state): State<AppState>,
    Path(id): Path<ScopeId>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let node = found(
            synveda_store::hierarchy::node(&mut *tx, id).await?,
            tenant_id,
            id,
        )?;
        authz::require(
            &state.pdp,
            &mut tx,
            Action::PolicyAssign,
            Resource::Scope(id),
            Some(&node),
        )
        .await?;
        if !policy_assignments::unassign(&mut *tx, tenant_id, id).await? {
            return Err(Error::NotFound {
                entity: format!("pack assignment on scope {id}"),
            });
        }
        commit(tx).await?;
        Ok(StatusCode::NO_CONTENT)
    }
    .await;
    respond("unassign_node_policy", result)
}
