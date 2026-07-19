//! The hierarchy admin API (HIER-1, ADR-0011): CRUD on `/v1/hierarchy/*`,
//! behind tenant resolution like every `/v1` route, and — since AUTHZ-1 —
//! behind the PDP: every handler authorizes through
//! [`crate::authz::require`] after its uniform-404 ownership check and
//! before acting (ADR-0012 decision 7, discharging ADR-0011 decision 8).
//!
//! AUD-1 wiring point: every mutation here is an audit emission point;
//! until the hash-chained log lands they are visible in traces and
//! `synveda_hierarchy_operations_total` (and every PDP decision in the
//! policy decision log and `synveda_authz_decisions_total`).

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use synveda_policy::{Action, Resource};
use synveda_store::{hierarchy, rls};
use synveda_types::{Error, HierarchyNode, Result, ScopeId, ScopeKind, TenantId};

use crate::app::AppState;
use crate::authz;
use crate::error::ApiError;
use crate::telemetry::HIERARCHY_OPERATIONS_TOTAL;

/// The resolved tenant from the task-local, or the invariant error: these
/// handlers only run behind the tenant-resolution middleware. Shared with
/// the policy routes (`crate::policy`).
pub(crate) fn tenant_id() -> Result<TenantId> {
    synveda_identity::current_tenant()
        .map(|context| context.tenant.id)
        .ok_or_else(|| Error::Internal {
            message: "hierarchy route ran outside a tenant scope".to_owned(),
        })
}

/// Counts the operation and renders the result, collapsing the taxonomy
/// into three outcomes: `ok`, `rejected` (the caller's fault), `error`
/// (ours or an operator's).
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
    metrics::counter!(HIERARCHY_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    match result {
        Ok(response) => response.into_response(),
        Err(error) => ApiError(error).into_response(),
    }
}

/// Maps a malformed JSON body onto the taxonomy instead of axum's default
/// plain-text rejection.
pub(crate) fn body<T>(payload: std::result::Result<Json<T>, JsonRejection>) -> Result<T> {
    payload
        .map(|Json(inner)| inner)
        .map_err(|rejection| Error::Invalid {
            message: format!("invalid request body: {rejection}"),
        })
}

#[derive(Deserialize)]
pub(crate) struct CreateNodeBody {
    parent_id: Option<ScopeId>,
    kind: ScopeKind,
    slug: String,
    name: String,
}

/// `POST /v1/hierarchy/nodes` — create a node (the org root when
/// `parent_id` is absent).
pub(crate) async fn create(
    State(state): State<AppState>,
    payload: std::result::Result<Json<CreateNodeBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        // Create targets the parent scope — the tenant itself for the root
        // (ADR-0012 decision 7). Ownership check first: a foreign parent is
        // a 404, never a policy denial oracle.
        match body.parent_id {
            None => {
                authz::require(
                    &state,
                    &mut tx,
                    Action::HierarchyCreate,
                    Resource::Tenant(tenant_id),
                    None,
                )
                .await?;
            }
            Some(parent_id) => {
                let parent = found(
                    hierarchy::node(&mut *tx, parent_id).await?,
                    tenant_id,
                    parent_id,
                )?;
                authz::require(
                    &state,
                    &mut tx,
                    Action::HierarchyCreate,
                    Resource::Scope(parent_id),
                    Some(&parent),
                )
                .await?;
            }
        }
        let node = hierarchy::create(
            &mut tx,
            ScopeId::new(),
            tenant_id,
            body.parent_id,
            body.kind,
            &body.slug,
            &body.name,
        )
        .await?;
        commit(tx).await?;
        // Any committed hierarchy mutation flushes the tenant's chains
        // and entity fragments (ADR-0016 decision 5, ADR-0017 decision 5)
        // — uniformly, though a fresh leaf strictly invalidates nothing.
        state.invalidate_hierarchy(tenant_id);
        Ok((StatusCode::CREATED, Json(node)))
    }
    .await;
    respond("create", result)
}

/// `GET /v1/hierarchy/root` — the tenant's org root.
pub(crate) async fn root(State(state): State<AppState>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        authz::require(
            &state,
            &mut tx,
            Action::HierarchyRead,
            Resource::Tenant(tenant_id),
            None,
        )
        .await?;
        let node = hierarchy::root(&mut *tx, tenant_id)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity: "hierarchy root".to_owned(),
            })?;
        Ok(Json(node))
    }
    .await;
    respond("root", result)
}

/// `GET /v1/hierarchy/nodes/{id}`.
pub(crate) async fn get(State(state): State<AppState>, Path(id): Path<ScopeId>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let node = found(hierarchy::node(&mut *tx, id).await?, tenant_id, id)?;
        authz::require(
            &state,
            &mut tx,
            Action::HierarchyRead,
            Resource::Scope(id),
            Some(&node),
        )
        .await?;
        Ok(Json(node))
    }
    .await;
    respond("get", result)
}

/// `GET /v1/hierarchy/nodes/{id}/children` — direct children, slug order.
pub(crate) async fn children(State(state): State<AppState>, Path(id): Path<ScopeId>) -> Response {
    let result = listing(&state, id, "children").await;
    respond("children", result)
}

/// `GET /v1/hierarchy/nodes/{id}/ancestors` — nearest first, root last.
pub(crate) async fn ancestors(State(state): State<AppState>, Path(id): Path<ScopeId>) -> Response {
    let result = listing(&state, id, "ancestors").await;
    respond("ancestors", result)
}

/// `GET /v1/hierarchy/nodes/{id}/descendants` — the subtree, path order.
pub(crate) async fn descendants(
    State(state): State<AppState>,
    Path(id): Path<ScopeId>,
) -> Response {
    let result = listing(&state, id, "descendants").await;
    respond("descendants", result)
}

/// Shared shape of the three listing routes: 404 for an unknown anchor
/// node (an empty list must mean "no relatives", never "no such node").
async fn listing(state: &AppState, id: ScopeId, which: &str) -> Result<Json<Vec<HierarchyNode>>> {
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let node = found(hierarchy::node(&mut *tx, id).await?, tenant_id, id)?;
    authz::require(
        state,
        &mut tx,
        Action::HierarchyRead,
        Resource::Scope(id),
        Some(&node),
    )
    .await?;
    let nodes = match which {
        "children" => hierarchy::children(&mut *tx, id).await?,
        "ancestors" => hierarchy::ancestors(&mut *tx, id).await?,
        "descendants" => hierarchy::descendants(&mut *tx, id).await?,
        other => {
            return Err(Error::Internal {
                message: format!("unknown listing {other:?}"),
            });
        }
    };
    Ok(Json(nodes))
}

#[derive(Deserialize)]
pub(crate) struct UpdateNodeBody {
    /// New display name; slugs are immutable (ADR-0011).
    name: Option<String>,
    /// New parent — a subtree move.
    parent_id: Option<ScopeId>,
}

/// `PATCH /v1/hierarchy/nodes/{id}` — rename and/or move.
pub(crate) async fn update(
    State(state): State<AppState>,
    Path(id): Path<ScopeId>,
    payload: std::result::Result<Json<UpdateNodeBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        if body.name.is_none() && body.parent_id.is_none() {
            return Err(Error::Invalid {
                message: "nothing to update: provide name and/or parent_id".to_owned(),
            });
        }
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        // Ownership check before any mutation (see `found`).
        let node = found(hierarchy::node(&mut *tx, id).await?, tenant_id, id)?;
        authz::require(
            &state,
            &mut tx,
            Action::HierarchyUpdate,
            Resource::Scope(id),
            Some(&node),
        )
        .await?;
        if let Some(name) = &body.name {
            hierarchy::rename(&mut *tx, id, name).await?;
        }
        if let Some(parent_id) = body.parent_id {
            hierarchy::move_node(&mut tx, id, parent_id).await?;
        }
        let node = found(hierarchy::node(&mut *tx, id).await?, tenant_id, id)?;
        commit(tx).await?;
        // A committed rename/move reshapes cached chains — and a move,
        // the Cedar entity graph (ADR-0016, ADR-0017).
        state.invalidate_hierarchy(tenant_id);
        Ok(Json(node))
    }
    .await;
    respond("update", result)
}

/// `DELETE /v1/hierarchy/nodes/{id}` — leaf nodes only.
pub(crate) async fn delete(State(state): State<AppState>, Path(id): Path<ScopeId>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        // Ownership check before any mutation (see `found`).
        let node = found(hierarchy::node(&mut *tx, id).await?, tenant_id, id)?;
        authz::require(
            &state,
            &mut tx,
            Action::HierarchyDelete,
            Resource::Scope(id),
            Some(&node),
        )
        .await?;
        if !hierarchy::delete(&mut tx, id).await? {
            return Err(not_found(id));
        }
        commit(tx).await?;
        // The deleted leaf's cached chain and fragment must go
        // (ADR-0016, ADR-0017).
        state.invalidate_hierarchy(tenant_id);
        Ok(StatusCode::NO_CONTENT)
    }
    .await;
    respond("delete", result)
}

pub(crate) fn not_found(id: ScopeId) -> Error {
    Error::NotFound {
        entity: format!("scope {id}"),
    }
}

/// The uniform 404 for missing *and* foreign nodes. Under the production
/// `synveda_app` role RLS already hides other tenants' rows; this explicit
/// tenant check keeps the API correct even on connections that bypass RLS
/// (the dev-compose superuser — ADR-0009's accepted trade-off), and it is
/// why every mutation starts by fetching the node.
pub(crate) fn found(
    node: Option<HierarchyNode>,
    tenant_id: TenantId,
    id: ScopeId,
) -> Result<HierarchyNode> {
    node.filter(|node| node.tenant_id == tenant_id)
        .ok_or_else(|| not_found(id))
}

pub(crate) async fn commit(tx: sqlx::Transaction<'static, sqlx::Postgres>) -> Result<()> {
    tx.commit().await.map_err(|err| Error::Storage {
        message: format!("commit hierarchy transaction: {err}"),
    })
}
