//! `GET /v1/me` (CPR-4, ADR-0071 decision 2): the one call a client makes
//! first, and the only one it needs before it knows what to show.
//!
//! # Why this exists beside `/v1/whoami`
//!
//! `whoami` answers "who does the gateway think is calling", which is an
//! introspection endpoint for a debugging human and the propagation-path proof
//! it was built as (ADR-0008). A client starting up needs four more things
//! before it can render anything: which tenant, **what exists** (workspaces and
//! projects), **what is missing** (the onboarding state), and what this caller
//! may do. Four calls to learn that is four round trips before the first pixel,
//! and — worse — four chances for a client to invent its own answer to "is
//! this person set up yet" out of a 404.
//!
//! So the onboarding state is **the server's answer, not a client's
//! inference**. `needs_workspace` is a fact the gateway computes from the same
//! rows it would refuse a project creation against; a client that derived it
//! from an empty list would be re-implementing a rule that lives here, and
//! would get it wrong the first time a caller could read no workspaces rather
//! than there being none.
//!
//! # It chains one event
//!
//! Unlike `whoami?capabilities=true`, which decides only about the caller and
//! chains nothing (ADR-0058 decision 4), this route **discloses governed
//! inventory** — which workspaces and projects exist. So it chains one
//! summarised `authz.decision`, the same shape the batch capability probe
//! uses. The alternative is a route that serves the same listing
//! `GET /v1/workspaces` chains an event for, without one: an audit trail with
//! a documented way around it is not one.

use axum::extract::State;
use axum::response::{IntoResponse, Json, Response};
use serde::Serialize;
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{Action, Resource};
use synveda_store::{identities, projects, rls, scopes, workspaces};
use synveda_types::{Error, IdentityId, Result, ScopeId, TenantId};
use utoipa::ToSchema;

use crate::app::AppState;
use crate::audit;
use crate::capabilities::TenantCapabilities;
use crate::error::ApiError;
use crate::hierarchy::{commit, tenant_id};
use crate::telemetry::WORKSPACE_OPERATIONS_TOTAL;
use crate::workspaces::{ProjectView, WorkspaceView};

/// Everything a client needs before it renders anything.
#[derive(Debug, Serialize, ToSchema)]
pub struct MeView {
    /// Who is calling.
    pub principal: PrincipalView,
    /// The tenant they resolved to.
    pub tenant: TenantView,
    /// Where they are in setting up — the server's answer, not a client's
    /// inference from an empty list.
    pub onboarding: OnboardingView,
    /// Every workspace this caller may read, by slug.
    pub workspaces: Vec<WorkspaceView>,
    /// Every project this caller may read, by workspace then slug. Flat rather
    /// than nested under its workspace: a client that wants the tree has
    /// `workspace_id` on every row, and one that wants a recent-projects list
    /// would otherwise have to flatten what we just nested.
    pub projects: Vec<ProjectView>,
    /// What this caller may do on the tenant plane, asked of the PDP.
    ///
    /// **A forecast, never a grant** (ADR-0058 decision 2): nothing downstream
    /// reads this to decide anything, every act still takes its own decision
    /// at its own seam, and a client uses this to choose what to *offer*.
    pub capabilities: TenantCapabilities,
}

/// The authenticated principal.
#[derive(Debug, Serialize, ToSchema)]
pub struct PrincipalView {
    /// The verified token's `sub` claim — the name every audit event, role
    /// binding and idempotency key is keyed by.
    pub subject: String,
    /// The identity row, when this subject has provisioned one. Absent for a
    /// dev token and for a service client that never completed login.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub identity_id: Option<IdentityId>,
    /// The IdP's `name` claim at provisioning time, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// `human` or `service`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Whether the base layer forbids this caller everything (AUTH-2,
    /// ADR-0013 decision 5). True also for an IdP subject that never
    /// provisioned — fail closed.
    pub quarantined: bool,
}

/// The tenant, as this plane serves it.
#[derive(Debug, Serialize, ToSchema)]
pub struct TenantView {
    /// The isolation key.
    #[schema(value_type = String, format = "uuid")]
    pub id: TenantId,
    /// Human-stable handle.
    pub slug: String,
    /// Display name.
    pub name: String,
    /// `active` or `suspended`.
    pub status: String,
}

/// How far along setting up this caller is.
#[derive(Debug, Serialize, ToSchema)]
pub struct OnboardingView {
    /// The single word a client branches on.
    pub state: OnboardingState,
    /// The tenant's root scope, once anything has needed one. Absent on a
    /// deployment where nobody has created a workspace yet: the root is minted
    /// by the first thing that needs a parent, so that nobody is asked to
    /// declare an organisation before they can hold a record.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub tenant_scope_id: Option<ScopeId>,
    /// How many workspaces this caller can see.
    pub workspace_count: usize,
    /// How many projects this caller can see.
    pub project_count: usize,
}

/// The onboarding vocabulary. Closed, so a client's branch is exhaustive and
/// a new state is a compile error somewhere rather than a silently unhandled
/// string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingState {
    /// The caller is quarantined: the base layer forbids everything, so there
    /// is no next step they can take themselves. Distinct from
    /// `needs_workspace` deliberately — a client that showed "create your
    /// first workspace" to somebody whose every request will be denied would
    /// be inviting them into a wall.
    Blocked,
    /// Nothing exists yet (or nothing this caller may read). Next step:
    /// `POST /v1/workspaces`.
    NeedsWorkspace,
    /// A workspace exists but holds no project. Next step:
    /// `POST /v1/workspaces/{workspace_id}/projects`.
    NeedsProject,
    /// There is somewhere to work.
    Ready,
}

impl OnboardingState {
    /// The stable wire name — what the client branches on and what the
    /// summarised audit event records.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            OnboardingState::Blocked => "blocked",
            OnboardingState::NeedsWorkspace => "needs_workspace",
            OnboardingState::NeedsProject => "needs_project",
            OnboardingState::Ready => "ready",
        }
    }

    /// Resolves the state from what the caller can actually see.
    ///
    /// Quarantine wins over everything, which is the one ordering that
    /// matters: it is the only state whose next step is "ask an
    /// administrator" rather than "press this button".
    #[must_use]
    pub const fn resolve(quarantined: bool, workspaces: usize, projects: usize) -> Self {
        if quarantined {
            OnboardingState::Blocked
        } else if workspaces == 0 {
            OnboardingState::NeedsWorkspace
        } else if projects == 0 {
            OnboardingState::NeedsProject
        } else {
            OnboardingState::Ready
        }
    }
}

/// `GET /v1/me`.
#[utoipa::path(
    get,
    path = "/v1/me",
    operation_id = "get_me",
    tag = "me",
    responses(
        (status = 200, description = "The caller, their tenant, their onboarding state, what they can see and what they may do", body = MeView),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn get(State(state): State<AppState>) -> Response {
    let result = get_inner(&state).await;
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
    metrics::counter!(WORKSPACE_OPERATIONS_TOTAL, "op" => "me", "outcome" => outcome).increment(1);
    match result {
        Ok(view) => Json(view).into_response(),
        Err(error) => {
            audit::record_rejection(&state, "me", &error).await;
            ApiError(error).into_response()
        }
    }
}

async fn get_inner(state: &AppState) -> Result<MeView> {
    let context = synveda_identity::current_tenant().ok_or_else(|| Error::Internal {
        message: "me route ran outside a tenant scope".to_owned(),
    })?;
    let tenant_id = tenant_id()?;
    let subject = context.claims.subject.clone();

    // The capability block first, on its own transaction. It resolves the
    // principal's quarantine status through the same `gather` every decision
    // uses, so nothing here re-derives it (AUTH-2, ADR-0013 decision 6).
    let capabilities = crate::capabilities::at_tenant(state).await?;

    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let identity = identities::by_subject(&mut *tx, tenant_id, &subject).await?;
    let quarantined = match &identity {
        Some(identity) => identity.quarantined,
        // An IdP subject with no identity is quarantined and a dev subject is
        // not — the same fail-closed rule the PDP applies, restated here
        // because this view must not disagree with the decision point about
        // whether somebody can do anything at all.
        None => context.claims.provisioning.is_some(),
    };

    // **The listings are decided, not merely fetched.** A caller who may not
    // read workspaces sees none — which is the same answer they would get from
    // `GET /v1/workspaces`, except that here it is a denial folded into an
    // empty list rather than a 403, because `/v1/me` has to answer *something*
    // about a caller who can see nothing.
    //
    // Both decisions come off **one** `gather`. That is not an optimisation
    // for its own sake: a gather is four reads (the identity, the chain, the
    // assignments, the bindings, the standing lapses), and this route is the
    // one a client calls on every page load. Sharing the input changes no
    // verdict — it is the same rows either decision would have read, at the
    // same instant — and it is the shape ADR-0042 decision 6 measured for the
    // capability probe's fan-out.
    let input = crate::authz::gather(state, &mut tx, None).await?;
    let resource = Resource::Tenant(tenant_id);
    let may_read_workspaces =
        crate::authz::decide(state, &input, Action::WorkspaceRead, resource, None).is_ok();
    let may_read_projects =
        crate::authz::decide(state, &input, Action::ProjectRead, resource, None).is_ok();
    drop(input);
    let workspaces = if may_read_workspaces {
        workspaces::list(&mut *tx, tenant_id).await?
    } else {
        Vec::new()
    };
    let projects = if may_read_projects {
        projects::list(&mut *tx, tenant_id).await?
    } else {
        Vec::new()
    };
    let tenant_scope = scopes::tenant_root(&mut *tx, tenant_id).await?;

    let onboarding = OnboardingView {
        state: OnboardingState::resolve(quarantined, workspaces.len(), projects.len()),
        tenant_scope_id: tenant_scope.map(|scope| scope.id),
        workspace_count: workspaces.len(),
        project_count: projects.len(),
    };

    // One summarised event for the whole call — the batch probe's shape
    // (ADR-0058 decision 4), because this is one request that took several
    // decisions and disclosed one inventory.
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::AuthzDecision,
        Resource::Tenant(tenant_id).to_string(),
        Outcome::Allow,
        json!({
            "op": "me",
            "onboarding": onboarding.state.as_str(),
            "workspaces_disclosed": workspaces.len(),
            "projects_disclosed": projects.len(),
            "workspace_read": may_read_workspaces,
            "project_read": may_read_projects,
        }),
    )
    .await?;
    commit(tx).await?;

    Ok(MeView {
        principal: PrincipalView {
            subject,
            identity_id: identity.as_ref().map(|identity| identity.id),
            display_name: identity
                .as_ref()
                .and_then(|identity| identity.display_name.clone()),
            kind: identity
                .as_ref()
                .map(|identity| identity.kind.as_str().to_owned()),
            quarantined,
        },
        tenant: TenantView {
            id: context.tenant.id,
            slug: context.tenant.slug,
            name: context.tenant.name,
            status: context.tenant.status.as_str().to_owned(),
        },
        onboarding,
        workspaces: workspaces.into_iter().map(Into::into).collect(),
        projects: projects.into_iter().map(Into::into).collect(),
        capabilities,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarantine_wins_over_every_other_state() {
        assert_eq!(
            OnboardingState::resolve(true, 0, 0),
            OnboardingState::Blocked
        );
        assert_eq!(
            OnboardingState::resolve(true, 5, 5),
            OnboardingState::Blocked,
            "a quarantined caller who can see everything still has no next step"
        );
    }

    #[test]
    fn the_next_step_is_the_first_thing_missing() {
        assert_eq!(
            OnboardingState::resolve(false, 0, 0),
            OnboardingState::NeedsWorkspace
        );
        assert_eq!(
            OnboardingState::resolve(false, 1, 0),
            OnboardingState::NeedsProject
        );
        assert_eq!(
            OnboardingState::resolve(false, 1, 1),
            OnboardingState::Ready
        );
    }

    #[test]
    fn states_serialise_as_their_wire_names() {
        for (state, name) in [
            (OnboardingState::Blocked, "blocked"),
            (OnboardingState::NeedsWorkspace, "needs_workspace"),
            (OnboardingState::NeedsProject, "needs_project"),
            (OnboardingState::Ready, "ready"),
        ] {
            assert_eq!(state.as_str(), name);
            assert_eq!(
                serde_json::to_string(&state).unwrap(),
                format!("\"{name}\"")
            );
        }
    }
}
