//! The scope admin plane (CPR-7, ADR-0074 decision 5): `/v1/admin/scopes`.
//!
//! Six routes over the CPR-3 store services — list, create, get, patch
//! (rename, re-describe, archive, **move**), ancestors, descendants — each
//! mutation PDP-decided against the scope it is about, audited, creation
//! idempotent under the CPR-4 discipline. There is no delete, for the
//! workspace plane's reason: a scope is what audit events, versions and
//! grants name, so retiring one is a status transition. Pack assignment
//! (`…/policy`) and the VedaFlow curator file (`…/curators`) are re-homed
//! under the same prefix by this prompt rather than invented — their
//! capability survived the tree they hung on.
//!
//! A move is the one mutation decided **twice**: once at the scope being
//! moved and once at the destination, because a move is the only
//! administration that touches two subtrees at once, and authority at one
//! end of it is exactly half the authority the act needs. Both decisions
//! and both ends are in the audit event.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{Action, Resource};
use synveda_store::{rls, scopes};
use synveda_types::scope::{Scope, ScopeKind, ScopeStatus};
use synveda_types::{Error, Result, ScopeId, TenantId};
use utoipa::ToSchema;

use crate::app::AppState;
use crate::audit;
use crate::authz;
use crate::error::ApiError;
use crate::request::{body, commit, found, tenant_id};
use crate::telemetry::SCOPE_OPERATIONS_TOTAL;

/// Counts the operation and renders the result, collapsing the taxonomy
/// into three outcomes: `ok`, `rejected` (the caller's fault), `error`
/// (ours or an operator's). Error-path audit events chain here, where
/// every handler result already funnels (AUD-1, ADR-0019 decision 5).
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
    metrics::counter!(SCOPE_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    match result {
        Ok(response) => response.into_response(),
        Err(error) => {
            audit::record_rejection(state, op, &error).await;
            ApiError(error).into_response()
        }
    }
}

/// One governed scope as the admin surface serves it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ScopeView {
    /// The scope's stable id.
    #[schema(value_type = String, format = "uuid")]
    pub id: ScopeId,
    /// The scope's shape.
    #[schema(value_type = String)]
    pub kind: ScopeKind,
    /// Its parent; absent only on the tenant root.
    #[schema(value_type = Option<String>, format = "uuid")]
    pub parent_scope_id: Option<ScopeId>,
    /// Sibling-unique handle, immutable.
    pub slug: String,
    /// Display name, renameable.
    pub display_name: String,
    /// `active` or `archived`.
    #[schema(value_type = String)]
    pub status: ScopeStatus,
    /// The subject this scope belongs to, on a `principal`-shaped scope.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal_id: Option<String>,
    /// The open labelling bag.
    pub attributes: serde_json::Value,
    /// When the scope was created.
    pub created_at: DateTime<Utc>,
    /// When the scope last changed.
    pub updated_at: DateTime<Utc>,
}

impl From<Scope> for ScopeView {
    fn from(scope: Scope) -> Self {
        ScopeView {
            id: scope.id,
            kind: scope.kind,
            parent_scope_id: scope.parent_scope_id,
            slug: scope.slug,
            display_name: scope.display_name,
            status: scope.status,
            principal_id: scope.principal_id,
            attributes: scope.attributes,
            created_at: scope.created_at,
            updated_at: scope.updated_at,
        }
    }
}

/// `GET /v1/admin/scopes` — one level of the tree: the children of
/// `parent_id`, or the tenant root and its children when no parent is
/// named. The tree is browsed a level at a time because that is what a
/// console renders and all a CLI needs to walk.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
// `IntoParams` defaults to `Path`, and this is a query string: the route is
// `/v1/admin/scopes`, which has no `{parent_id}` in it. Declared rather than
// inferred because the contract is what the console's client is generated
// from (CPR-8) — a parameter in the wrong place produces a client that
// builds a URL the gateway has never served.
#[into_params(parameter_in = Query)]
pub struct ListParams {
    /// The scope whose children to list; absent means the tenant root.
    #[serde(default)]
    #[param(value_type = String, format = "uuid")]
    pub parent_id: Option<ScopeId>,
}

/// One level of the scope tree: the parent the level hangs from, and its
/// children.
#[derive(Debug, Serialize, ToSchema)]
pub struct ListResponse {
    /// The level's parent, when the response is rooted at one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<ScopeView>,
    /// The parent's children, sorted by slug.
    pub scopes: Vec<ScopeView>,
}

#[utoipa::path(
    get,
    path = "/v1/admin/scopes",
    operation_id = "list_scopes",
    tag = "admin-scopes",
    params(ListParams),
    responses(
        (status = 200, description = "One level of the tenant's scope tree", body = ListResponse),
        (status = 401, description = "No credential, or an unverifiable one", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "The named parent is not this tenant's", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "admin_scopes.list", skip_all)]
pub(crate) async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let (parent, children) = match params.parent_id {
            Some(parent_id) => {
                let parent = found(
                    scopes::get(&mut *tx, tenant_id, parent_id).await?,
                    tenant_id,
                    parent_id,
                )?;
                let children = scopes::children(&mut *tx, tenant_id, parent_id).await?;
                (Some(parent), children)
            }
            None => {
                let root = scopes::tenant_root(&mut *tx, tenant_id).await?;
                match root {
                    Some(root) => {
                        let children = scopes::children(&mut *tx, tenant_id, root.id).await?;
                        (Some(root), children)
                    }
                    None => (None, Vec::new()),
                }
            }
        };
        let input = authz::gather(
            &state,
            &mut tx,
            parent.as_ref(),
            synveda_store::anchors::AnchorSelection::none(),
            Vec::new(),
        )
        .await?;
        let authorized = authz::decide(
            &state,
            &input,
            Action::ScopeRead,
            parent
                .as_ref()
                .map_or(Resource::Tenant(tenant_id), |scope| {
                    Resource::Scope(scope.id)
                }),
        )?;
        crate::policy::read_event(
            &mut tx,
            tenant_id,
            "admin_scopes.list",
            parent
                .as_ref()
                .map_or(Resource::Tenant(tenant_id), |scope| {
                    Resource::Scope(scope.id)
                }),
            &authorized,
        )
        .await?;
        commit(tx).await?;
        // The level's parent rides its own field; `scopes` holds its
        // children only, sorted by slug for a stable rendering.
        let mut views: Vec<ScopeView> = children.into_iter().map(Into::into).collect();
        views.sort_by(|a, b| a.slug.cmp(&b.slug));
        Ok(Json(ListResponse {
            parent: parent.map(Into::into),
            scopes: views,
        }))
    }
    .await;
    respond(&state, "list", result).await
}

/// `POST /v1/admin/scopes` — create a scope under a parent. The tenant
/// root is minted by the substrate, so `parent_id` is required.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateScopeBody {
    /// The parent. The tenant root is minted by the first thing that needs
    /// a parent and cannot be created here.
    #[schema(value_type = String, format = "uuid")]
    pub parent_id: ScopeId,
    /// The scope's shape — `org_unit`, `workspace`, `project` or
    /// `principal`. The old rank vocabulary (`org`, `division`,
    /// `department`, `team`, `user`) fails validation by name.
    #[schema(value_type = String)]
    pub kind: ScopeKind,
    /// Sibling-unique handle, immutable.
    pub slug: String,
    /// Display name.
    pub display_name: String,
    /// Open labelling bag; never an authorisation input.
    #[serde(default)]
    pub attributes: Option<serde_json::Value>,
}

#[utoipa::path(
    post,
    path = "/v1/admin/scopes",
    operation_id = "create_scope",
    tag = "admin-scopes",
    request_body = CreateScopeBody,
    params(("Idempotency-Key" = String, Header, description = "Required. Same key + same body replays with 200; a different body is 409.")),
    responses(
        (status = 201, description = "The scope was created", body = ScopeView),
        (status = 200, description = "An idempotent replay of the same request", body = ScopeView),
        (status = 400, description = "The body failed validation — including every old scope kind", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No credential, or an unverifiable one", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "The parent is not this tenant's", body = crate::workspaces::ApiErrorBody),
        (status = 409, description = "The idempotency key names a different request", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "admin_scopes.create", skip_all)]
pub(crate) async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: std::result::Result<Json<CreateScopeBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant_id = tenant_id()?;
        let subject = subject()?;
        let claim = crate::idempotency::Claim::from_headers(
            &headers,
            "scope.create",
            &subject,
            &json!({
                "route": "POST /v1/admin/scopes",
                "parent_id": body.parent_id,
                "kind": body.kind,
                "slug": body.slug,
                "display_name": body.display_name,
                "attributes": body.attributes.clone().unwrap_or(serde_json::json!({})),
            }),
        )?;

        let replayed = match crate::idempotency::dispatch(&state.pool, tenant_id, &claim).await? {
            crate::idempotency::Dispatch::Replay(id) => Some(id),
            crate::idempotency::Dispatch::Create => {
                match create_scope(&state, tenant_id, &body, &claim).await {
                    Ok(scope) => {
                        return Ok((StatusCode::CREATED, Json(ScopeView::from(scope))));
                    }
                    Err(conflict @ Error::Conflict { .. }) => Some(
                        crate::idempotency::resolve_conflict(
                            &state.pool,
                            tenant_id,
                            &claim,
                            conflict,
                        )
                        .await?,
                    ),
                    Err(other) => return Err(other),
                }
            }
        };
        let id = ScopeId::from_uuid(replayed.expect("replay id"));
        let scope = replay_scope(&state, tenant_id, id).await?;
        Ok((StatusCode::OK, Json(ScopeView::from(scope))))
    }
    .await;
    respond(&state, "create", result).await
}

async fn create_scope(
    state: &AppState,
    tenant_id: TenantId,
    body: &CreateScopeBody,
    claim: &crate::idempotency::Claim,
) -> Result<Scope> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    // Creation is decided at the parent — the scope it would land in
    // (CPR-4's shape). Ownership check first: a foreign parent is a 404,
    // never a policy denial oracle.
    let parent = found(
        scopes::get(&mut *tx, tenant_id, body.parent_id).await?,
        tenant_id,
        body.parent_id,
    )?;
    let input = authz::gather(
        state,
        &mut tx,
        Some(&parent),
        synveda_store::anchors::AnchorSelection::none(),
        Vec::new(),
    )
    .await?;
    let authorized = authz::decide(
        state,
        &input,
        Action::ScopeCreate,
        Resource::Scope(parent.id),
    )?;
    let created_by = actor_identity(&mut tx, tenant_id).await?;
    let new = scopes::NewScope {
        id: ScopeId::new(),
        tenant_id,
        kind: body.kind,
        parent_scope_id: Some(parent.id),
        slug: body.slug.clone(),
        display_name: body.display_name.clone(),
        attributes: body.attributes.clone().unwrap_or(serde_json::json!({})),
        principal_id: None,
        created_by,
    };
    let scope = scopes::create(&mut tx, &new).await?;
    claim
        .remember(&mut tx, tenant_id, scope.id.as_uuid())
        .await?;
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::ScopeCreated,
        Resource::Scope(scope.id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::ScopeCreate, &authorized),
            "scope": {"id": scope.id, "kind": scope.kind, "slug": scope.slug,
                      "display_name": scope.display_name},
            "parent": {"id": parent.id, "slug": parent.slug},
        }),
    )
    .await?;
    commit(tx).await?;
    state.invalidate_scopes(tenant_id);
    Ok(scope)
}

async fn replay_scope(state: &AppState, tenant_id: TenantId, id: ScopeId) -> Result<Scope> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let scope = found(scopes::get(&mut *tx, tenant_id, id).await?, tenant_id, id)?;
    // A replay still takes the decision — a cached authorisation is exactly
    // what seed §2.2 forbids (CPR-4's rule, kept). Creation is decided at
    // the parent, so the parent is the anchor its chain is gathered from.
    let parent_id = scope.parent_scope_id.ok_or_else(|| Error::Internal {
        message: "replayed a root creation".to_owned(),
    })?;
    let parent = found(
        scopes::get(&mut *tx, tenant_id, parent_id).await?,
        tenant_id,
        parent_id,
    )?;
    authz::require(
        state,
        &mut tx,
        Action::ScopeCreate,
        Resource::Scope(parent.id),
        Some(&parent),
    )
    .await?;
    drop(tx);
    Ok(scope)
}

/// `GET /v1/admin/scopes/{scope_id}` — the scope and its path.
#[derive(Debug, Serialize, ToSchema)]
pub struct ScopeDetail {
    /// The scope itself.
    pub scope: ScopeView,
    /// The slug chain from the tenant root — display and ordering only.
    pub path: String,
}

#[utoipa::path(
    get,
    path = "/v1/admin/scopes/{scope_id}",
    operation_id = "get_scope",
    tag = "admin-scopes",
    params(("scope_id" = String, Path, description = "The scope id")),
    responses(
        (status = 200, description = "The scope, with its path", body = ScopeDetail),
        (status = 401, description = "No credential, or an unverifiable one", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "Not this tenant's scope", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "admin_scopes.get", skip_all)]
pub(crate) async fn get(State(state): State<AppState>, Path(scope_id): Path<ScopeId>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let scope = found(
            scopes::get(&mut *tx, tenant_id, scope_id).await?,
            tenant_id,
            scope_id,
        )?;
        let input = authz::gather(
            &state,
            &mut tx,
            Some(&scope),
            synveda_store::anchors::AnchorSelection::none(),
            Vec::new(),
        )
        .await?;
        let authorized =
            authz::decide(&state, &input, Action::ScopeRead, Resource::Scope(scope_id))?;
        crate::policy::read_event(
            &mut tx,
            tenant_id,
            "admin_scopes.get",
            Resource::Scope(scope_id),
            &authorized,
        )
        .await?;
        let path = scopes::path(&mut *tx, tenant_id, scope_id)
            .await?
            .unwrap_or_else(|| scope.slug.clone());
        commit(tx).await?;
        Ok(Json(ScopeDetail {
            scope: scope.into(),
            path,
        }))
    }
    .await;
    respond(&state, "get", result).await
}

/// `PATCH /v1/admin/scopes/{scope_id}` — rename, re-describe, archive or
/// move. Omitted fields change nothing; a `parent_scope_id` is a move.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct PatchScopeBody {
    /// The new display name.
    #[serde(default)]
    pub display_name: Option<String>,
    /// `active` or `archived`.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub status: Option<ScopeStatus>,
    /// Naming a parent **moves the scope and its subtree**.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub parent_scope_id: Option<ScopeId>,
    /// The new labelling bag, replacing the old one whole.
    #[serde(default)]
    pub attributes: Option<serde_json::Value>,
}

#[utoipa::path(
    patch,
    path = "/v1/admin/scopes/{scope_id}",
    operation_id = "update_scope",
    tag = "admin-scopes",
    request_body = PatchScopeBody,
    params(("scope_id" = String, Path, description = "The scope id")),
    responses(
        (status = 200, description = "The scope after the change", body = ScopeView),
        (status = 400, description = "The change failed validation — a move into the scope's own subtree, a malformed name", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No credential, or an unverifiable one", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "Not this tenant's scope — or, for a move, not the destination's", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "admin_scopes.update", skip_all)]
pub(crate) async fn update(
    State(state): State<AppState>,
    Path(scope_id): Path<ScopeId>,
    payload: std::result::Result<Json<PatchScopeBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let scope = found(
            scopes::get(&mut *tx, tenant_id, scope_id).await?,
            tenant_id,
            scope_id,
        )?;
        let input = authz::gather(
            &state,
            &mut tx,
            Some(&scope),
            synveda_store::anchors::AnchorSelection::none(),
            Vec::new(),
        )
        .await?;
        let authorized = authz::decide(
            &state,
            &input,
            Action::ScopeUpdate,
            Resource::Scope(scope_id),
        )?;

        // A move is decided at both ends: authority over the subtree being
        // moved and over the destination it lands in are two halves of one
        // act, and either alone is the half an attacker wants. The
        // destination's decision is gathered from the destination's own
        // chain — the entity the decision names must be in the context for
        // `resource in principal.tenant` to walk.
        let mut destination = None;
        if let Some(parent_scope_id) = body.parent_scope_id
            && Some(parent_scope_id) != scope.parent_scope_id
        {
            let dest = found(
                scopes::get(&mut *tx, tenant_id, parent_scope_id).await?,
                tenant_id,
                parent_scope_id,
            )?;
            let dest_input = authz::gather(
                &state,
                &mut tx,
                Some(&dest),
                synveda_store::anchors::AnchorSelection::none(),
                Vec::new(),
            )
            .await?;
            authz::decide(
                &state,
                &dest_input,
                Action::ScopeUpdate,
                Resource::Scope(dest.id),
            )?;
            destination = Some(dest);
        }

        let mut scope = scope;
        // The end the scope is leaving, read before the move rewrites it:
        // "audited with both ends" (ADR-0074 decision 5) is not satisfied
        // by the destination alone — where a subtree came from is half of
        // what an auditor reconstructing a reorganisation needs.
        let moved_from = scope.parent_scope_id;
        let mut changes: Vec<&'static str> = Vec::new();
        if let Some(display_name) = body.display_name.clone() {
            scope = scopes::rename(&mut *tx, tenant_id, scope_id, &display_name).await?;
            changes.push("display_name");
        }
        if let Some(status) = body.status {
            scope = scopes::set_status(&mut *tx, tenant_id, scope_id, status).await?;
            changes.push("status");
        }
        if let Some(attributes) = body.attributes.clone() {
            scopes::set_attributes(&mut tx, tenant_id, scope_id, &attributes).await?;
            scope.attributes = attributes;
            changes.push("attributes");
        }
        let moved_to = destination
            .as_ref()
            .map(|dest| json!({"id": dest.id, "slug": dest.slug}));
        let moved_from = destination
            .as_ref()
            .and(moved_from)
            .map(|parent| json!({"id": parent}));
        if let Some(destination) = destination {
            scope = scopes::move_scope(&mut tx, tenant_id, scope_id, destination.id).await?;
            changes.push("parent_scope_id");
        }
        if changes.is_empty() {
            return Err(Error::Invalid {
                message: "patch changed nothing: name at least one of display_name, status, \
                          parent_scope_id, attributes"
                    .to_owned(),
            });
        }
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::ScopeUpdated,
            Resource::Scope(scope_id).to_string(),
            Outcome::Success,
            json!({
                "authz": audit::decision_context(Action::ScopeUpdate, &authorized),
                "changes": changes,
                "scope": {"id": scope.id, "slug": scope.slug, "display_name": scope.display_name},
                "moved_from": moved_from,
                "moved_to": moved_to,
            }),
        )
        .await?;
        commit(tx).await?;
        state.invalidate_scopes(tenant_id);
        Ok(Json(ScopeView::from(scope)))
    }
    .await;
    respond(&state, "update", result).await
}

/// `GET /v1/admin/scopes/{scope_id}/ancestors` — the chain to the tenant
/// root, nearest first.
#[derive(Debug, Serialize, ToSchema)]
pub struct ChainResponse {
    /// The chain or subtree, nearest first.
    pub scopes: Vec<ScopeView>,
}

#[utoipa::path(
    get,
    path = "/v1/admin/scopes/{scope_id}/ancestors",
    operation_id = "list_scope_ancestors",
    tag = "admin-scopes",
    params(("scope_id" = String, Path, description = "The scope id")),
    responses(
        (status = 200, description = "The scope's ancestors, nearest first", body = ChainResponse),
        (status = 401, description = "No credential, or an unverifiable one", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "Not this tenant's scope", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "admin_scopes.ancestors", skip_all)]
pub(crate) async fn ancestors(
    State(state): State<AppState>,
    Path(scope_id): Path<ScopeId>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let scope = found(
            scopes::get(&mut *tx, tenant_id, scope_id).await?,
            tenant_id,
            scope_id,
        )?;
        let input = authz::gather(
            &state,
            &mut tx,
            Some(&scope),
            synveda_store::anchors::AnchorSelection::none(),
            Vec::new(),
        )
        .await?;
        let authorized =
            authz::decide(&state, &input, Action::ScopeRead, Resource::Scope(scope_id))?;
        crate::policy::read_event(
            &mut tx,
            tenant_id,
            "admin_scopes.ancestors",
            Resource::Scope(scope_id),
            &authorized,
        )
        .await?;
        let chain = scopes::ancestors(&mut *tx, tenant_id, scope_id).await?;
        commit(tx).await?;
        Ok(Json(ChainResponse {
            scopes: chain.into_iter().map(Into::into).collect(),
        }))
    }
    .await;
    respond(&state, "ancestors", result).await
}

/// `GET /v1/admin/scopes/{scope_id}/descendants` — the whole subtree,
/// nearest first, the scope itself excluded.
#[utoipa::path(
    get,
    path = "/v1/admin/scopes/{scope_id}/descendants",
    operation_id = "list_scope_descendants",
    tag = "admin-scopes",
    params(("scope_id" = String, Path, description = "The scope id")),
    responses(
        (status = 200, description = "The scope's subtree, nearest first", body = ChainResponse),
        (status = 401, description = "No credential, or an unverifiable one", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "Not this tenant's scope", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "admin_scopes.descendants", skip_all)]
pub(crate) async fn descendants(
    State(state): State<AppState>,
    Path(scope_id): Path<ScopeId>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let scope = found(
            scopes::get(&mut *tx, tenant_id, scope_id).await?,
            tenant_id,
            scope_id,
        )?;
        let input = authz::gather(
            &state,
            &mut tx,
            Some(&scope),
            synveda_store::anchors::AnchorSelection::none(),
            Vec::new(),
        )
        .await?;
        let authorized =
            authz::decide(&state, &input, Action::ScopeRead, Resource::Scope(scope_id))?;
        crate::policy::read_event(
            &mut tx,
            tenant_id,
            "admin_scopes.descendants",
            Resource::Scope(scope_id),
            &authorized,
        )
        .await?;
        let subtree = scopes::descendants(&mut *tx, tenant_id, scope_id).await?;
        commit(tx).await?;
        Ok(Json(ChainResponse {
            scopes: subtree.into_iter().map(Into::into).collect(),
        }))
    }
    .await;
    respond(&state, "descendants", result).await
}

fn subject() -> Result<String> {
    synveda_identity::current_tenant()
        .map(|context| context.claims.subject)
        .ok_or_else(|| Error::Internal {
            message: "route ran outside a tenant scope".to_owned(),
        })
}

async fn actor_identity(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
) -> Result<Option<synveda_types::IdentityId>> {
    let subject = subject()?;
    Ok(
        synveda_store::identities::by_subject(&mut *tx, tenant_id, &subject)
            .await?
            .map(|identity| identity.id),
    )
}
