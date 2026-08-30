//! The service-identity admin API (AUTH-3, ADR-0018 decision 3):
//! registering headless agents on `/v1/service-identities`. Behind tenant
//! resolution like every `/v1` route, uniform-404 ownership first, then
//! the PDP (`ServiceIdentityRead`/`ServiceIdentityManage`).
//!
//! Registration binds an IdP client's subject to an anchor node: a
//! personal user-kind leaf is created under the anchor and the identity
//! row (kind `service`) points at it — the exact JIT placement shape
//! (ADR-0013 decision 2), so quarantine derivation and the scope-chain
//! machinery apply unchanged. The token the agent later presents is the
//! IdP's client-credentials access token; the enforcement seam confines
//! it to the anchor's subtree (ADR-0018 decisions 4–5). Revocation
//! deletes the row and the leaf; re-anchoring is the existing PDP-gated
//! hierarchy move of the leaf.
//!
//! Audited since AUD-1 (ADR-0019): registration and revocation chain
//! their semantic events in their own transactions; reads chain their
//! allowed decision; denials and seam token rejections chain at the
//! `respond` seam.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{Action, Resource};
use synveda_store::{access, directory, identities, rls, scopes};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::scope::ScopeKind;
use synveda_types::{Error, GrantId, Identity, IdentityId, IdentityKind, Result, ScopeId};

use crate::app::AppState;
use crate::audit;
use crate::authz;
use crate::request::{body, commit, found, tenant_id};
use crate::telemetry::SERVICE_IDENTITY_OPERATIONS_TOTAL;

/// Counts the operation and renders the result — the same outcome
/// taxonomy as the hierarchy, policy, and role routes. Error-path audit
/// events chain here (AUD-1, ADR-0019 decision 5).
async fn respond<T: IntoResponse>(
    state: &AppState,
    op: &'static str,
    result: Result<T>,
) -> Response {
    let outcome = crate::response::outcome(&result);
    metrics::counter!(SERVICE_IDENTITY_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome)
        .increment(1);
    crate::response::finish(state, op, result).await
}

#[derive(Deserialize, utoipa::ToSchema)]
#[schema(as = RegisterServiceIdentityBody)]
pub(crate) struct RegisterBody {
    /// The stable subject identifier expected from the agent's
    /// client-credentials access tokens.
    subject: String,
    /// The anchor node whose subtree confines the agent's tokens.
    #[schema(value_type = String, format = "uuid")]
    scope_id: ScopeId,
    /// Display name for the agent's personal leaf; defaults to the
    /// subject.
    display_name: Option<String>,
}

/// A service identity as the public application API exposes it.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ServiceIdentityView {
    #[schema(value_type = String, format = "uuid")]
    id: IdentityId,
    subject: Option<String>,
    kind: String,
    email: Option<String>,
    display_name: Option<String>,
    #[schema(value_type = String, format = "uuid")]
    scope_id: ScopeId,
    status: String,
    departed_at: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<Identity> for ServiceIdentityView {
    fn from(identity: Identity) -> Self {
        Self {
            id: identity.id,
            subject: identity.subject,
            kind: identity.kind.to_string(),
            email: identity.email,
            display_name: identity.display_name,
            scope_id: identity.scope_id,
            status: identity.status.to_string(),
            departed_at: identity.departed_at,
            created_at: identity.created_at,
        }
    }
}

#[derive(Serialize, utoipa::ToSchema)]
pub(crate) struct ServiceIdentitiesResponse {
    identities: Vec<ServiceIdentityView>,
}

/// `POST /v1/service-identities` — register an agent at an anchor node.
/// `ServiceIdentityManage` on the anchor: a steward registers agents in
/// their subtree, visibly (ADR-0018 decision 3).
#[utoipa::path(
    post,
    path = "/v1/service-identities",
    operation_id = "register_service_identity",
    tag = "service-identities",
    request_body = RegisterBody,
    responses(
        (status = 201, description = "The registered service identity", body = ServiceIdentityView),
        (status = 400, description = "The subject or anchor is invalid", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Service identity management is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "The anchor is absent or outside the tenant", body = crate::workspaces::ApiErrorBody),
        (status = 409, description = "The subject or principal scope already exists", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "service_identity.register", skip_all)]
pub(crate) async fn register(
    State(state): State<AppState>,
    payload: std::result::Result<Json<RegisterBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let anchor = found(
            scopes::get(&mut *tx, tenant_id, body.scope_id).await?,
            tenant_id,
            body.scope_id,
        )?;
        // An agent cannot be anchored at somebody's own scope: a principal
        // nests under the anchor an operator names, and the substrate's
        // own placement rule refuses a principal under a principal
        // (CPR-7, ADR-0074 — the rank rule's old job, as a shape rule).
        let authorized = authz::require(
            &state,
            &mut tx,
            Action::ServiceIdentityManage,
            Resource::Scope(anchor.id),
            Some(&anchor),
        )
        .await?;
        // Directory correspondence is the outer identity lock domain. A
        // first login may bind this subject before transferring its
        // principal-scope owner grant, while registration creates the scope
        // before inserting the identity row. Serialising here establishes
        // directory -> principal -> scope/identity order for both paths and
        // prevents those operations from waiting on each other in reverse.
        directory::lock_correspondence(&mut tx, tenant_id).await?;
        let identity_id = IdentityId::new();
        let display_name = body.display_name.as_deref().unwrap_or(&body.subject);
        // The agent's own scope: a `principal`-shaped scope under the
        // operator's anchor, so ADR-0018 decision 4's confinement is tree
        // position — the scope above the agent's own.
        let leaf = scopes::create(
            &mut tx,
            &scopes::NewScope {
                id: ScopeId::new(),
                tenant_id,
                kind: ScopeKind::Principal,
                parent_scope_id: Some(anchor.id),
                slug: scopes::principal_slug(&body.subject),
                display_name: display_name.to_owned(),
                attributes: serde_json::json!({}),
                principal_id: Some(body.subject.clone()),
                created_by: None,
            },
        )
        .await?;
        let identity = identities::create(
            &mut tx,
            identity_id,
            tenant_id,
            Some(&body.subject),
            IdentityKind::Service,
            None,
            body.display_name.as_deref(),
            leaf.id,
        )
        .await?;
        // A service principal's leaf has the same closed privacy boundary as
        // a user's own scope. Registration therefore mints the same direct
        // owner grant as `ensure_principal_scope`, atomically with the leaf
        // and identity (ADR-0074 decision 8). Without it the service could
        // read its private material through the base privacy clause but could
        // never govern that material under any shipped policy pack.
        let owner_grant = access::create_grant(
            &mut tx,
            &access::NewGrant {
                id: GrantId::new(),
                tenant_id,
                scope_id: leaf.id,
                subject: GrantSubject::Principal {
                    principal_id: body.subject.clone(),
                },
                role_key: RoleKey::Owner,
                source: GrantSource::Owner,
                invite_id: None,
                granted_by: None,
            },
        )
        .await?;
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::ServiceIdentityRegistered,
            Resource::Scope(anchor.id).to_string(),
            Outcome::Success,
            json!({
                "authz": audit::decision_context(Action::ServiceIdentityManage, &authorized),
                "identity": {"id": identity.id, "subject": identity.subject},
                "leaf_scope_id": leaf.id,
                "anchor": {"slug": anchor.slug},
            }),
        )
        .await?;
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::AccessGranted,
            Resource::Scope(leaf.id).to_string(),
            Outcome::Success,
            json!({
                "origin": "service-identity-registration",
                "grant": {
                    "id": owner_grant.id,
                    "scope_id": owner_grant.scope_id,
                    "subject": body.subject,
                    "role": owner_grant.role_key,
                    "source": owner_grant.source,
                },
            }),
        )
        .await?;
        commit(tx).await?;
        // The leaf is a committed hierarchy mutation (ADR-0016 decision 5,
        // ADR-0017 decision 5).
        state.invalidate_scopes(tenant_id);
        tracing::info!(
            identity.id = %identity.id,
            scope.id = %leaf.id,
            anchor.id = %anchor.id,
            "service identity registered"
        );
        Ok((
            StatusCode::CREATED,
            Json(ServiceIdentityView::from(identity)),
        ))
    }
    .await;
    respond(&state, "register", result).await
}

/// `GET /v1/service-identities` — the tenant's registered agents. A
/// tenant-plane read: `ServiceIdentityRead` at the tenant.
#[utoipa::path(
    get,
    path = "/v1/service-identities",
    operation_id = "list_service_identities",
    tag = "service-identities",
    responses(
        (status = 200, description = "Registered service identities", body = ServiceIdentitiesResponse),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Service identity inventory is not visible", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "service_identity.list", skip_all)]
pub(crate) async fn list(State(state): State<AppState>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let authorized = authz::require(
            &state,
            &mut tx,
            Action::ServiceIdentityRead,
            Resource::Tenant(tenant_id),
            None,
        )
        .await?;
        let identities = identities::services(&mut *tx, tenant_id).await?;
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::AuthzDecision,
            Resource::Tenant(tenant_id).to_string(),
            Outcome::Allow,
            json!({
                "op": "list",
                "authz": audit::decision_context(Action::ServiceIdentityRead, &authorized),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(ServiceIdentitiesResponse {
            identities: identities.into_iter().map(Into::into).collect(),
        }))
    }
    .await;
    respond(&state, "list", result).await
}

/// `GET /v1/service-identities/{id}` — one registration.
/// `ServiceIdentityRead` on the anchor.
#[utoipa::path(
    get,
    path = "/v1/service-identities/{id}",
    operation_id = "get_service_identity",
    tag = "service-identities",
    params(("id" = String, Path, format = "uuid")),
    responses(
        (status = 200, description = "One registered service identity", body = ServiceIdentityView),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "The service identity is not visible", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "The identity is absent, non-service or outside the tenant", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "service_identity.get", skip_all)]
pub(crate) async fn get(State(state): State<AppState>, Path(id): Path<IdentityId>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let (identity, anchor) = found_service(&mut tx, tenant_id, id).await?;
        let authorized = authz::require(
            &state,
            &mut tx,
            Action::ServiceIdentityRead,
            Resource::Scope(anchor.id),
            Some(&anchor),
        )
        .await?;
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::AuthzDecision,
            Resource::Scope(anchor.id).to_string(),
            Outcome::Allow,
            json!({
                "op": "get",
                "authz": audit::decision_context(Action::ServiceIdentityRead, &authorized),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(ServiceIdentityView::from(identity)))
    }
    .await;
    respond(&state, "get", result).await
}

/// `DELETE /v1/service-identities/{id}` — revoke: delete the identity row
/// and its personal leaf. `ServiceIdentityManage` on the anchor. Effective
/// on the next request: an unregistered IdP subject is quarantined at the
/// seam (ADR-0013 decision 6).
#[utoipa::path(
    delete,
    path = "/v1/service-identities/{id}",
    operation_id = "remove_service_identity",
    tag = "service-identities",
    params(("id" = String, Path, format = "uuid")),
    responses(
        (status = 204, description = "The service identity was revoked"),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Service identity management is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "The identity is absent, non-service or outside the tenant", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "service_identity.remove", skip_all)]
pub(crate) async fn remove(State(state): State<AppState>, Path(id): Path<IdentityId>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let (identity, anchor) = found_service(&mut tx, tenant_id, id).await?;
        let authorized = authz::require(
            &state,
            &mut tx,
            Action::ServiceIdentityManage,
            Resource::Scope(anchor.id),
            Some(&anchor),
        )
        .await?;
        // Row first (its FK pins the scope), then the scope: archived
        // rather than row-deleted, because nothing in the governed model
        // deletes a scope — the identity row going is what makes the
        // agent's credential refuse, and the archived scope keeps what
        // audit events name addressable.
        if !identities::delete_service(&mut *tx, tenant_id, identity.id).await? {
            return Err(not_found(id));
        }
        scopes::set_status(
            &mut *tx,
            tenant_id,
            identity.scope_id,
            synveda_types::scope::ScopeStatus::Archived,
        )
        .await?;
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::ServiceIdentityRevoked,
            Resource::Scope(anchor.id).to_string(),
            Outcome::Success,
            json!({
                "authz": audit::decision_context(Action::ServiceIdentityManage, &authorized),
                "identity": {"id": identity.id, "subject": identity.subject},
                "anchor": {"slug": anchor.slug},
            }),
        )
        .await?;
        commit(tx).await?;
        // The leaf's cached chain and fragment must go (ADR-0016, ADR-0017).
        state.invalidate_scopes(tenant_id);
        tracing::info!(identity.id = %id, "service identity revoked");
        Ok(StatusCode::NO_CONTENT)
    }
    .await;
    respond(&state, "remove", result).await
}

fn not_found(id: IdentityId) -> Error {
    Error::NotFound {
        entity: format!("service identity {id}"),
    }
}

/// The uniform 404 for missing, foreign, and non-service identities — a
/// user identity probed through this route reveals nothing — plus the
/// agent's anchor node (its personal leaf's parent), which the PDP
/// decisions here target (ADR-0018 decision 3).
async fn found_service(
    tx: &mut sqlx::PgConnection,
    tenant_id: synveda_types::TenantId,
    id: IdentityId,
) -> Result<(Identity, synveda_types::scope::Scope)> {
    let identity = identities::by_id(&mut *tx, tenant_id, id)
        .await?
        .filter(|identity| {
            identity.tenant_id == tenant_id && identity.kind == IdentityKind::Service
        })
        .ok_or_else(|| not_found(id))?;
    let own = scopes::get(&mut *tx, tenant_id, identity.scope_id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!("service identity {id} lost its scope"),
        })?;
    let anchor_id = own.parent_scope_id.ok_or_else(|| Error::Internal {
        message: format!("service identity {id}'s scope has no parent"),
    })?;
    let anchor = scopes::get(&mut *tx, tenant_id, anchor_id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!("service identity {id}'s anchor vanished"),
        })?;
    Ok((identity, anchor))
}
