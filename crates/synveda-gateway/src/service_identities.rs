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
//! AUD-1 wiring point: registration and revocation are audit emission
//! points; until the hash-chained log lands they are visible in traces
//! and `synveda_service_identity_operations_total`.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use synveda_policy::{Action, Resource};
use synveda_store::{hierarchy, identities, rls};
use synveda_types::{
    Error, HierarchyNode, Identity, IdentityId, IdentityKind, Result, ScopeId, ScopeKind,
};

use synveda_identity::personal_slug;

use crate::app::AppState;
use crate::authz;
use crate::error::ApiError;
use crate::hierarchy::{body, commit, found, tenant_id};
use crate::telemetry::SERVICE_IDENTITY_OPERATIONS_TOTAL;

/// Counts the operation and renders the result — the same outcome
/// taxonomy as the hierarchy, policy, and role routes.
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
    metrics::counter!(SERVICE_IDENTITY_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome)
        .increment(1);
    match result {
        Ok(response) => response.into_response(),
        Err(error) => ApiError(error).into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct RegisterBody {
    /// The `sub` the IdP will put in the agent's client-credentials
    /// tokens (for Rauthy, the client id).
    subject: String,
    /// The anchor node whose subtree confines the agent's tokens.
    scope_id: ScopeId,
    /// Display name for the agent's personal leaf; defaults to the
    /// subject.
    display_name: Option<String>,
}

#[derive(Serialize)]
struct ServiceIdentitiesResponse {
    identities: Vec<Identity>,
}

/// `POST /v1/service-identities` — register an agent at an anchor node.
/// `ServiceIdentityManage` on the anchor: a steward registers agents in
/// their subtree, visibly (ADR-0018 decision 3).
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
            hierarchy::node(&mut *tx, body.scope_id).await?,
            tenant_id,
            body.scope_id,
        )?;
        // Registering an agent into quarantine is an operator error, not a
        // placement (ADR-0018 decision 2). User-kind anchors are refused
        // by the hierarchy's own rank rule below.
        if anchor.slug == identities::QUARANTINE_SLUG && anchor.depth == 1 {
            return Err(Error::Invalid {
                message: "service identities cannot be anchored at the quarantine scope".to_owned(),
            });
        }
        authz::require(
            &state,
            &mut tx,
            Action::ServiceIdentityManage,
            Resource::Scope(anchor.id),
            Some(&anchor),
        )
        .await?;
        let identity_id = IdentityId::new();
        let display_name = body.display_name.as_deref().unwrap_or(&body.subject);
        let leaf = hierarchy::create(
            &mut tx,
            ScopeId::new(),
            tenant_id,
            Some(anchor.id),
            ScopeKind::User,
            &personal_slug(None, &body.subject, identity_id),
            display_name,
        )
        .await?;
        let identity = identities::create(
            &mut tx,
            identity_id,
            tenant_id,
            &body.subject,
            IdentityKind::Service,
            None,
            body.display_name.as_deref(),
            leaf.id,
        )
        .await?;
        commit(tx).await?;
        // The leaf is a committed hierarchy mutation (ADR-0016 decision 5,
        // ADR-0017 decision 5).
        state.invalidate_hierarchy(tenant_id);
        tracing::info!(
            identity.id = %identity.id,
            scope.id = %leaf.id,
            anchor.id = %anchor.id,
            "service identity registered"
        );
        Ok((StatusCode::CREATED, Json(identity)))
    }
    .await;
    respond("register", result)
}

/// `GET /v1/service-identities` — the tenant's registered agents. A
/// tenant-plane read: `ServiceIdentityRead` at the tenant.
#[tracing::instrument(name = "service_identity.list", skip_all)]
pub(crate) async fn list(State(state): State<AppState>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        authz::require(
            &state,
            &mut tx,
            Action::ServiceIdentityRead,
            Resource::Tenant(tenant_id),
            None,
        )
        .await?;
        let identities = identities::services(&mut *tx, tenant_id).await?;
        Ok(Json(ServiceIdentitiesResponse { identities }))
    }
    .await;
    respond("list", result)
}

/// `GET /v1/service-identities/{id}` — one registration.
/// `ServiceIdentityRead` on the anchor.
#[tracing::instrument(name = "service_identity.get", skip_all)]
pub(crate) async fn get(State(state): State<AppState>, Path(id): Path<IdentityId>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let (identity, anchor) = found_service(&mut tx, tenant_id, id).await?;
        authz::require(
            &state,
            &mut tx,
            Action::ServiceIdentityRead,
            Resource::Scope(anchor.id),
            Some(&anchor),
        )
        .await?;
        Ok(Json(identity))
    }
    .await;
    respond("get", result)
}

/// `DELETE /v1/service-identities/{id}` — revoke: delete the identity row
/// and its personal leaf. `ServiceIdentityManage` on the anchor. Effective
/// on the next request: an unregistered IdP subject is quarantined at the
/// seam (ADR-0013 decision 6).
#[tracing::instrument(name = "service_identity.remove", skip_all)]
pub(crate) async fn remove(State(state): State<AppState>, Path(id): Path<IdentityId>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let (identity, anchor) = found_service(&mut tx, tenant_id, id).await?;
        authz::require(
            &state,
            &mut tx,
            Action::ServiceIdentityManage,
            Resource::Scope(anchor.id),
            Some(&anchor),
        )
        .await?;
        // Row first (its FK pins the leaf), then the leaf.
        if !identities::delete_service(&mut *tx, tenant_id, identity.id).await? {
            return Err(not_found(id));
        }
        if !hierarchy::delete(&mut tx, identity.scope_id).await? {
            return Err(Error::Internal {
                message: format!("service identity {id} lost its personal leaf mid-delete"),
            });
        }
        commit(tx).await?;
        // The leaf's cached chain and fragment must go (ADR-0016, ADR-0017).
        state.invalidate_hierarchy(tenant_id);
        tracing::info!(identity.id = %id, "service identity revoked");
        Ok(StatusCode::NO_CONTENT)
    }
    .await;
    respond("remove", result)
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
) -> Result<(Identity, HierarchyNode)> {
    let identity = identities::by_id(&mut *tx, tenant_id, id)
        .await?
        .filter(|identity| {
            identity.tenant_id == tenant_id && identity.kind == IdentityKind::Service
        })
        .ok_or_else(|| not_found(id))?;
    let leaf = hierarchy::node(&mut *tx, identity.scope_id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!("service identity {id} lost its personal leaf"),
        })?;
    let anchor_id = leaf.parent_id.ok_or_else(|| Error::Internal {
        message: format!("service identity {id}'s personal leaf has no parent"),
    })?;
    let anchor = hierarchy::node(&mut *tx, anchor_id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!("service identity {id}'s anchor vanished"),
        })?;
    Ok((identity, anchor))
}
