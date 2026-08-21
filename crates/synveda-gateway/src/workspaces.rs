//! The workspace, project and repository plane (CPR-4, ADR-0071):
//! `/v1/workspaces/*` and `/v1/projects/*`, behind tenant resolution like
//! every `/v1` route, behind the PDP like every governed one, and chaining an
//! audit event for every mutation.
//!
//! Nothing is synchronised to make this work: no row of `hierarchy_nodes`
//! becomes a scope, and no scope is mirrored into the hierarchy (ADR-0068
//! decision 3).
//!
//! # Creation is idempotent; updates carry a precondition
//!
//! `POST` takes a required `Idempotency-Key` (see [`crate::idempotency`]);
//! `PATCH` takes a required `expected_revision`. The two together are what
//! makes this plane safe to drive from an agent that retries: a duplicate
//! create is answered with the original resource, and a concurrent update is
//! refused rather than silently taking the last writer.
//!
//! # Where the decisions are anchored
//!
//! Since CPR-6 every decision here names the thing it is about (ADR-0073
//! decision 3): a read or an update names the workspace or the project, a
//! project creation names the workspace it would land in, and the two
//! tenant-plane calls — listing workspaces and creating one — name the tenant
//! **root scope**, or the tenant itself on a deployment that has not minted a
//! root yet. CPR-4 anchored all fourteen at the tenant and said so as a
//! stated debt; this is that debt paid.
//!
//! The consequence a reader should expect: the ownership check now runs
//! *before* the decision on every per-object route, because deciding about a
//! workspace requires having fetched it. That is the order ADR-0012 decision 7
//! sets and the hierarchy plane has always used — a foreign object is a 404,
//! never a policy-denial oracle.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{Action, Resource, ResourceEntity};
use synveda_store::anchors::AnchorSelection;
use synveda_store::{projects, repositories, rls, workspaces};
use synveda_types::repository::{ProjectRepository, RepositoryProvider};
use synveda_types::workspace::{LifecycleStatus, Project, Workspace};
use synveda_types::{
    Error, IdentityId, ProjectId, RepositoryId, Result, ScopeId, TenantId, WorkspaceId,
};
use utoipa::ToSchema;

use crate::app::AppState;
use crate::audit;
use crate::authz::{self, Authorized};
use crate::error::ApiError;
use crate::idempotency::{Claim, Dispatch};
use crate::request::{body, commit, tenant_id};
use crate::telemetry::WORKSPACE_OPERATIONS_TOTAL;

// ── Views ────────────────────────────────────────────────────────────────────

/// A workspace, as the API serves it.
///
/// A view rather than `synveda_types::workspace::Workspace` itself, because
/// this is the **contract** and the domain type is not: the two agree today
/// and the day they need to differ — a computed field, a withheld one — the
/// contract must be able to say so without the storage type moving. `tenant_id`
/// is deliberately absent: every `/v1` response is already scoped to the
/// caller's tenant, and echoing it invites a client to key on it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WorkspaceView {
    /// The workspace's stable id.
    #[schema(value_type = String, format = "uuid")]
    pub id: WorkspaceId,
    /// The governed scope this workspace owns — what policy, role bindings and
    /// every asset attach to.
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    /// Tenant-unique handle, identical to the scope's slug. Immutable.
    pub slug: String,
    /// Display name.
    pub display_name: String,
    /// Optional prose.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether the workspace is in use.
    #[schema(schema_with = lifecycle_status_schema)]
    pub status: LifecycleStatus,
    /// The revision an update must name as its precondition.
    pub revision: i64,
    /// Who created it; absent when the deployment did.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub created_by: Option<IdentityId>,
    /// When it was created.
    pub created_at: DateTime<Utc>,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
}

impl From<Workspace> for WorkspaceView {
    fn from(workspace: Workspace) -> Self {
        WorkspaceView {
            id: workspace.id,
            scope_id: workspace.scope_id,
            slug: workspace.slug,
            display_name: workspace.display_name,
            description: workspace.description,
            status: workspace.status,
            revision: workspace.revision,
            created_by: workspace.created_by,
            created_at: workspace.created_at,
            updated_at: workspace.updated_at,
        }
    }
}

/// A project, as the API serves it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProjectView {
    /// The project's stable id.
    #[schema(value_type = String, format = "uuid")]
    pub id: ProjectId,
    /// The workspace it belongs to. Immutable.
    #[schema(value_type = String, format = "uuid")]
    pub workspace_id: WorkspaceId,
    /// The governed scope this project owns, beneath the workspace's.
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    /// Workspace-unique handle, identical to the scope's slug. Immutable.
    pub slug: String,
    /// Display name.
    pub display_name: String,
    /// Optional prose.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether the project is in use.
    #[schema(schema_with = lifecycle_status_schema)]
    pub status: LifecycleStatus,
    /// The revision an update must name as its precondition.
    pub revision: i64,
    /// Who created it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub created_by: Option<IdentityId>,
    /// When it was created.
    pub created_at: DateTime<Utc>,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
}

impl From<Project> for ProjectView {
    fn from(project: Project) -> Self {
        ProjectView {
            id: project.id,
            workspace_id: project.workspace_id,
            scope_id: project.scope_id,
            slug: project.slug,
            display_name: project.display_name,
            description: project.description,
            status: project.status,
            revision: project.revision,
            created_by: project.created_by,
            created_at: project.created_at,
            updated_at: project.updated_at,
        }
    }
}

/// A repository attached to a project, as the API serves it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RepositoryView {
    /// The attachment's id — the handle `DELETE` takes.
    #[schema(value_type = String, format = "uuid")]
    pub id: RepositoryId,
    /// The project it belongs to.
    #[schema(value_type = String, format = "uuid")]
    pub project_id: ProjectId,
    /// Where it is hosted.
    #[schema(schema_with = repository_provider_schema)]
    pub provider: RepositoryProvider,
    /// **The identity**: canonical, credential-free, and never a filesystem
    /// path. Two clients that describe one repository differently are served
    /// the same value here, which is what makes it an identity.
    pub canonical_uri: String,
    /// The owning path on the host, when the remote had one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_owner: Option<String>,
    /// The repository's own name.
    pub repository_name: String,
    /// The advisory default branch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    /// The stable content fingerprint of a local checkout, when one was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_fingerprint: Option<String>,
    /// The caller's labelling bag, echoed back.
    #[schema(value_type = Object)]
    pub metadata: serde_json::Value,
    /// Who attached it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub created_by: Option<IdentityId>,
    /// When it was attached.
    pub created_at: DateTime<Utc>,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
}

impl From<ProjectRepository> for RepositoryView {
    fn from(repository: ProjectRepository) -> Self {
        RepositoryView {
            id: repository.id,
            project_id: repository.project_id,
            provider: repository.provider,
            canonical_uri: repository.canonical_uri,
            repository_owner: repository.repository_owner,
            repository_name: repository.repository_name,
            default_branch: repository.default_branch,
            local_fingerprint: repository.local_fingerprint,
            metadata: repository.metadata,
            created_by: repository.created_by,
            created_at: repository.created_at,
            updated_at: repository.updated_at,
        }
    }
}

/// The workspace listing.
///
/// An envelope rather than a bare array, so that paging can arrive without
/// breaking every client — and so that a future `not_answered` has somewhere
/// to live, which is the shape the capability probe already uses for the same
/// reason (ADR-0058 decision 5).
#[derive(Debug, Serialize, ToSchema)]
pub struct WorkspaceList {
    /// Every workspace the caller may read, by slug. Archived ones included:
    /// a listing that silently omitted them would make an archived workspace
    /// indistinguishable from one that never existed.
    pub workspaces: Vec<WorkspaceView>,
}

/// The project listing.
#[derive(Debug, Serialize, ToSchema)]
pub struct ProjectList {
    /// The workspace's projects, by slug. Archived ones included.
    pub projects: Vec<ProjectView>,
}

/// The repository listing.
#[derive(Debug, Serialize, ToSchema)]
pub struct RepositoryList {
    /// What the project is about, oldest attachment first.
    pub repositories: Vec<RepositoryView>,
}

/// The `status` vocabulary, built from
/// [`synveda_types::workspace::LifecycleStatus`] itself rather than
/// transcribed — so the OpenAPI enum and the Rust enum cannot disagree, and
/// nobody has to maintain a second copy of a two-word list that would then
/// grow to three.
fn lifecycle_status_schema() -> utoipa::openapi::schema::Object {
    string_enum(LifecycleStatus::ALL.iter().map(LifecycleStatus::as_str))
}

/// The `provider` vocabulary, built from
/// [`synveda_types::repository::RepositoryProvider`] the same way.
fn repository_provider_schema() -> utoipa::openapi::schema::Object {
    string_enum(
        RepositoryProvider::ALL
            .iter()
            .map(RepositoryProvider::as_str),
    )
}

pub(crate) fn string_enum<'a>(
    values: impl Iterator<Item = &'a str>,
) -> utoipa::openapi::schema::Object {
    utoipa::openapi::ObjectBuilder::new()
        .schema_type(utoipa::openapi::schema::Type::String)
        .enum_values(Some(values.collect::<Vec<_>>()))
        .build()
}

// ── Request bodies ───────────────────────────────────────────────────────────

/// `POST /v1/workspaces`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateWorkspaceBody {
    /// Tenant-unique handle: `^[a-z0-9][a-z0-9-]{0,62}$`. Becomes the owned
    /// scope's slug too, and is immutable afterwards.
    pub slug: String,
    /// Display name.
    pub display_name: String,
    /// Optional prose. Blank is refused; omit it instead.
    #[serde(default)]
    pub description: Option<String>,
}

/// `POST /v1/workspaces/{workspace_id}/projects`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProjectBody {
    /// Workspace-unique handle, same grammar as a workspace slug.
    pub slug: String,
    /// Display name.
    pub display_name: String,
    /// Optional prose.
    #[serde(default)]
    pub description: Option<String>,
}

/// `PATCH /v1/workspaces/{workspace_id}` and
/// `PATCH /v1/projects/{project_id}`.
///
/// `description` has three cases and the wire says them apart: absent leaves
/// it alone, `null` clears it, a string replaces it.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateBody {
    /// The revision the caller last saw. Required: an update without a
    /// precondition is a last-writer-wins update, which is the failure this
    /// field exists to remove rather than a convenience it offers.
    pub expected_revision: i64,
    /// New display name.
    #[serde(default)]
    pub display_name: Option<String>,
    /// New description; `null` clears it.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    /// New lifecycle status. Mirrored onto the owned scope.
    #[serde(default)]
    #[schema(schema_with = lifecycle_status_schema)]
    pub status: Option<LifecycleStatus>,
}

/// `POST /v1/projects/{project_id}/repositories`.
///
/// The server derives `provider`, `canonical_uri`, `repository_owner` and
/// `repository_name`; a client sends what it knows and never what it
/// concluded, because two clients concluding separately is how one repository
/// becomes two rows.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AttachRepositoryBody {
    /// The remote, in any form git accepts: `https://host/owner/name`,
    /// `git@host:owner/name.git`, `ssh://git@host/owner/name`. **The
    /// identity**, whenever it is known. A filesystem path is refused.
    #[serde(default)]
    pub remote_uri: Option<String>,
    /// A stable content id for a repository with no remote — a git
    /// root-commit object id, 40–128 hex characters. Never a path.
    #[serde(default)]
    pub local_fingerprint: Option<String>,
    /// What to call it. Derived from the remote when there is one; **required**
    /// when there is not, because a fingerprint names nothing a human reads.
    #[serde(default)]
    pub name: Option<String>,
    /// Override the provider the host implies — for a self-hosted GitHub
    /// Enterprise or GitLab on a company domain.
    #[serde(default)]
    #[schema(schema_with = repository_provider_schema)]
    pub provider: Option<RepositoryProvider>,
    /// Advisory default branch. Nothing in the product resolves it.
    #[serde(default)]
    pub default_branch: Option<String>,
    /// Caller-supplied labelling bag; a JSON object, at most 8 KiB encoded.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub metadata: Option<serde_json::Value>,
}

/// Distinguishes an absent field from an explicit `null`.
///
/// serde collapses both onto `None` for a plain `Option`, so a surface that
/// wants to offer "clear this" needs the outer layer this adds — otherwise the
/// only way to clear a description is a second endpoint whose entire purpose
/// is saying `null` out loud.
fn double_option<'de, T, D>(deserializer: D) -> std::result::Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
}

// ── Shared handler plumbing ──────────────────────────────────────────────────

/// Counts the operation and renders the result, the same three-outcome
/// taxonomy the hierarchy, policy, role and capability planes use.
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
    metrics::counter!(WORKSPACE_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    match result {
        Ok(response) => response.into_response(),
        Err(error) => {
            audit::record_rejection(state, op, &error).await;
            ApiError(error).into_response()
        }
    }
}

/// The verified token subject, from the task-local the tenant middleware set.
pub(crate) fn subject() -> Result<String> {
    synveda_identity::current_tenant()
        .map(|context| context.claims.subject)
        .ok_or_else(|| Error::Internal {
            message: "workspace route ran outside a tenant scope".to_owned(),
        })
}

/// What a decision on this plane is **about** (CPR-6, ADR-0073 decision 3).
///
/// Until CPR-6 every one of them named the tenant, and ADR-0071 decision 3
/// said why: the Cedar entity model still described the old hierarchy, so a
/// governed scope had no chain to decide against and "administer workspaces"
/// was the only sentence a pack could write. It can now say *which* one.
enum Subject<'a> {
    /// The tenant plane — listing workspaces, creating one. Decided at the
    /// tenant **root scope** when the tenant has one, because that is where a
    /// tenant-wide grant is written, and at the tenant itself on a deployment
    /// where nobody has created anything yet.
    Tenant,
    /// One workspace.
    Workspace(&'a Workspace),
    /// One project.
    Project(&'a Project),
}

/// Takes one decision about `subject` and chains nothing — the caller decides
/// whether the act that follows carries a semantic event or the decision
/// stands alone.
///
/// Returns the resource it decided against as well as the verdict, so the
/// audit event names the same thing the PDP did rather than the tenant every
/// event on this plane used to name.
async fn require(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    action: Action,
    tenant_id: TenantId,
    subject: Subject<'_>,
) -> Result<(Authorized, Resource)> {
    let (anchor, selection, resources, resource) = match subject {
        Subject::Tenant => {
            let root = synveda_store::scopes::tenant_root(&mut *tx, tenant_id).await?;
            let resource = match &root {
                Some(scope) => Resource::Scope(scope.id),
                None => Resource::Tenant(tenant_id),
            };
            (root, AnchorSelection::none(), Vec::new(), resource)
        }
        Subject::Workspace(workspace) => (
            synveda_store::scopes::get(&mut *tx, tenant_id, workspace.scope_id).await?,
            AnchorSelection::workspace(workspace.id),
            vec![ResourceEntity::Workspace {
                id: workspace.id,
                scope_id: workspace.scope_id,
            }],
            Resource::Workspace(workspace.id),
        ),
        Subject::Project(project) => (
            synveda_store::scopes::get(&mut *tx, tenant_id, project.scope_id).await?,
            AnchorSelection::project(project.id),
            vec![ResourceEntity::Project {
                id: project.id,
                scope_id: project.scope_id,
                workspace_id: project.workspace_id,
            }],
            Resource::Project(project.id),
        ),
    };
    let input = authz::gather(state, tx, anchor.as_ref(), selection, resources).await?;
    let authorized = authz::decide(state, &input, action, resource)?;
    Ok((authorized, resource))
}

/// The allowed-read decision event (ADR-0019 decision 4): a read has no
/// semantic event of its own, so the decision itself chains — which is why
/// the read handlers here commit their transactions.
async fn read_event(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    op: &'static str,
    action: Action,
    authorized: &Authorized,
    resource: Resource,
    detail: serde_json::Value,
) -> Result<()> {
    audit::record(
        tx,
        tenant_id,
        AuditAction::AuthzDecision,
        resource.to_string(),
        Outcome::Allow,
        json!({
            "op": op,
            "authz": audit::decision_context(action, authorized),
            "detail": detail,
        }),
    )
    .await
    .map(|_| ())
}

/// The payload image of a workspace — stable fields only, no timestamps that
/// would differ from the response body.
fn workspace_image(workspace: &Workspace) -> serde_json::Value {
    json!({
        "id": workspace.id,
        "scope_id": workspace.scope_id,
        "slug": workspace.slug,
        "display_name": workspace.display_name,
        "description": workspace.description,
        "status": workspace.status.as_str(),
        "revision": workspace.revision,
    })
}

fn project_image(project: &Project) -> serde_json::Value {
    json!({
        "id": project.id,
        "workspace_id": project.workspace_id,
        "scope_id": project.scope_id,
        "slug": project.slug,
        "display_name": project.display_name,
        "description": project.description,
        "status": project.status.as_str(),
        "revision": project.revision,
    })
}

/// The payload image of a repository attachment.
///
/// It carries `canonical_uri`, which is safe precisely because
/// canonicalisation drops the credential: a caller that pasted
/// `https://x-access-token:ghp_…@github.com/acme/repo` has handed the gateway
/// a live token, and the audit chain is the last place it should land — so
/// this is one of the seams where that property is load-bearing rather than
/// tidy (seed: no secret in an audit payload).
fn repository_image(repository: &ProjectRepository) -> serde_json::Value {
    json!({
        "id": repository.id,
        "project_id": repository.project_id,
        "provider": repository.provider.as_str(),
        "canonical_uri": repository.canonical_uri,
        "repository_owner": repository.repository_owner,
        "repository_name": repository.repository_name,
        "default_branch": repository.default_branch,
        "has_local_fingerprint": repository.local_fingerprint.is_some(),
    })
}

/// The uniform 404 for a workspace that is missing *or* another tenant's.
fn workspace_not_found(id: WorkspaceId) -> Error {
    Error::NotFound {
        entity: format!("workspace {id}"),
    }
}

fn project_not_found(id: ProjectId) -> Error {
    Error::NotFound {
        entity: format!("project {id}"),
    }
}

// ── Workspaces ───────────────────────────────────────────────────────────────

/// `GET /v1/workspaces` — the tenant's workspaces.
#[utoipa::path(
    get,
    path = "/v1/workspaces",
    operation_id = "list_workspaces",
    tag = "workspaces",
    responses(
        (status = 200, description = "The workspaces this caller may read", body = WorkspaceList),
        (status = 401, description = "No usable credential", body = ApiErrorBody),
        (status = 403, description = "The PDP denied `workspace.read`", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn list(State(state): State<AppState>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let (authorized, resource) = require(
            &state,
            &mut tx,
            Action::WorkspaceRead,
            tenant_id,
            Subject::Tenant,
        )
        .await?;
        let workspaces = workspaces::list(&mut *tx, tenant_id).await?;
        read_event(
            &mut tx,
            tenant_id,
            "workspace.list",
            Action::WorkspaceRead,
            &authorized,
            resource,
            json!({"count": workspaces.len()}),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(WorkspaceList {
            workspaces: workspaces.into_iter().map(Into::into).collect(),
        }))
    }
    .await;
    respond(&state, "workspace.list", result).await
}

/// `POST /v1/workspaces` — create a workspace and the governed scope it owns.
#[utoipa::path(
    post,
    path = "/v1/workspaces",
    operation_id = "create_workspace",
    tag = "workspaces",
    request_body = CreateWorkspaceBody,
    params(
        ("Idempotency-Key" = String, Header,
         description = "Required. A unique value per request, reused verbatim on retry."),
    ),
    responses(
        (status = 201, description = "Created", body = WorkspaceView),
        (status = 200, description = "This key already created this workspace", body = WorkspaceView),
        (status = 400, description = "Malformed body, or no `Idempotency-Key`", body = ApiErrorBody),
        (status = 403, description = "The PDP denied `workspace.create`", body = ApiErrorBody),
        (status = 409, description = "The slug is taken, or the key was reused for a different request", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: std::result::Result<Json<CreateWorkspaceBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant_id = tenant_id()?;
        let subject = subject()?;
        let claim = Claim::from_headers(
            &headers,
            "workspace.create",
            &subject,
            &json!({
                "route": "POST /v1/workspaces",
                "slug": body.slug,
                "display_name": body.display_name,
                "description": body.description,
            }),
        )?;

        let replayed = match crate::idempotency::dispatch(&state.pool, tenant_id, &claim).await? {
            Dispatch::Replay(id) => Some(id),
            Dispatch::Create => match create_workspace(&state, tenant_id, &body, &claim).await {
                Ok(workspace) => {
                    return Ok((StatusCode::CREATED, Json(WorkspaceView::from(workspace))));
                }
                Err(conflict @ Error::Conflict { .. }) => Some(
                    crate::idempotency::resolve_conflict(&state.pool, tenant_id, &claim, conflict)
                        .await?,
                ),
                Err(other) => return Err(other),
            },
        };
        let id = WorkspaceId::from_uuid(replayed.expect("replay id"));
        let workspace = replay_workspace(&state, tenant_id, id, &claim).await?;
        Ok((StatusCode::OK, Json(WorkspaceView::from(workspace))))
    }
    .await;
    respond(&state, "workspace.create", result).await
}

/// The fresh-creation path: decide, create, remember the key, chain, commit.
///
/// All four in one transaction. The idempotency record in particular: written
/// afterwards it could be lost between two commits, and the client's retry
/// would then create a second workspace — the failure the record exists to
/// prevent, arriving through the door built to prevent it.
async fn create_workspace(
    state: &AppState,
    tenant_id: TenantId,
    body: &CreateWorkspaceBody,
    claim: &Claim,
) -> Result<Workspace> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let (authorized, _) = require(
        state,
        &mut tx,
        Action::WorkspaceCreate,
        tenant_id,
        Subject::Tenant,
    )
    .await?;
    let created_by = actor_identity(&mut tx, tenant_id).await?;
    let workspace = workspaces::create(
        &mut tx,
        &workspaces::NewWorkspace {
            id: WorkspaceId::new(),
            tenant_id,
            slug: body.slug.clone(),
            display_name: body.display_name.clone(),
            description: body.description.clone(),
            created_by,
        },
    )
    .await?;
    let owner = mint_owner_grant(&mut tx, tenant_id, workspace.scope_id, &claim.subject).await?;
    claim
        .remember(&mut tx, tenant_id, workspace.id.as_uuid())
        .await?;
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::AccessGranted,
        Resource::Scope(workspace.scope_id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::WorkspaceCreate, &authorized),
            "grant": {
                "id": owner.id,
                "scope_id": owner.scope_id,
                "subject_kind": owner.subject_kind.as_str(),
                "principal_id": owner.principal_id,
                "role": owner.role_key.as_str(),
                "source": owner.source.as_str(),
            },
            "workspace_id": workspace.id,
        }),
    )
    .await?;
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::WorkspaceCreated,
        Resource::Scope(workspace.scope_id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::WorkspaceCreate, &authorized),
            "workspace": workspace_image(&workspace),
            "idempotency_key": claim.key,
        }),
    )
    .await?;
    commit(tx).await?;
    Ok(workspace)
}

/// The replay path: the same decision, then the resource the key produced.
///
/// The decision is taken again on purpose. A replay is still a request to
/// create a workspace, and a caller whose permission was revoked between the
/// first attempt and the retry must be refused — a replay that skipped the PDP
/// would be a cached authorisation, which is exactly what seed §2.2 forbids.
async fn replay_workspace(
    state: &AppState,
    tenant_id: TenantId,
    id: WorkspaceId,
    claim: &Claim,
) -> Result<Workspace> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let (authorized, resource) = require(
        state,
        &mut tx,
        Action::WorkspaceCreate,
        tenant_id,
        Subject::Tenant,
    )
    .await?;
    let workspace = workspaces::get(&mut *tx, tenant_id, id)
        .await?
        .ok_or_else(|| crate::idempotency::vanished(claim, id.as_uuid()))?;
    read_event(
        &mut tx,
        tenant_id,
        "workspace.create.replay",
        Action::WorkspaceCreate,
        &authorized,
        resource,
        json!({"workspace_id": id, "idempotency_key": claim.key}),
    )
    .await?;
    commit(tx).await?;
    Ok(workspace)
}

/// `GET /v1/workspaces/{workspace_id}`.
#[utoipa::path(
    get,
    path = "/v1/workspaces/{workspace_id}",
    operation_id = "get_workspace",
    tag = "workspaces",
    params(("workspace_id" = String, Path, description = "The workspace's id")),
    responses(
        (status = 200, description = "The workspace", body = WorkspaceView),
        (status = 403, description = "The PDP denied `workspace.read`", body = ApiErrorBody),
        (status = 404, description = "No such workspace in this tenant", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn get(
    State(state): State<AppState>,
    Path(workspace_id): Path<WorkspaceId>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        // Ownership first, then the decision **about this workspace**: a
        // foreign one is a 404 rather than a denial oracle (ADR-0012
        // decision 7), and the decision that follows names the thing itself.
        let workspace = workspaces::get(&mut *tx, tenant_id, workspace_id)
            .await?
            .ok_or_else(|| workspace_not_found(workspace_id))?;
        let (authorized, resource) = require(
            &state,
            &mut tx,
            Action::WorkspaceRead,
            tenant_id,
            Subject::Workspace(&workspace),
        )
        .await?;
        read_event(
            &mut tx,
            tenant_id,
            "workspace.get",
            Action::WorkspaceRead,
            &authorized,
            resource,
            json!({"workspace_id": workspace_id}),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(WorkspaceView::from(workspace)))
    }
    .await;
    respond(&state, "workspace.get", result).await
}

/// `PATCH /v1/workspaces/{workspace_id}` — rename, re-describe or retire.
#[utoipa::path(
    patch,
    path = "/v1/workspaces/{workspace_id}",
    operation_id = "update_workspace",
    tag = "workspaces",
    params(("workspace_id" = String, Path, description = "The workspace's id")),
    request_body = UpdateBody,
    responses(
        (status = 200, description = "The updated workspace", body = WorkspaceView),
        (status = 400, description = "Malformed body, or nothing to update", body = ApiErrorBody),
        (status = 403, description = "The PDP denied `workspace.update`", body = ApiErrorBody),
        (status = 404, description = "No such workspace in this tenant", body = ApiErrorBody),
        (status = 409, description = "`expected_revision` is not the current revision", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn update(
    State(state): State<AppState>,
    Path(workspace_id): Path<WorkspaceId>,
    payload: std::result::Result<Json<UpdateBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        // Ownership before the decision and before the mutation, as
        // everywhere: a foreign workspace is a 404 and never a revision oracle.
        let before = workspaces::get(&mut *tx, tenant_id, workspace_id)
            .await?
            .ok_or_else(|| workspace_not_found(workspace_id))?;
        let (authorized, _) = require(
            &state,
            &mut tx,
            Action::WorkspaceUpdate,
            tenant_id,
            Subject::Workspace(&before),
        )
        .await?;
        let after = workspaces::update(
            &mut tx,
            tenant_id,
            workspace_id,
            body.expected_revision,
            &workspaces::WorkspaceUpdate {
                display_name: body.display_name.clone(),
                description: body.description.clone(),
                status: body.status,
            },
        )
        .await?;
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::WorkspaceUpdated,
            Resource::Scope(after.scope_id).to_string(),
            Outcome::Success,
            json!({
                "authz": audit::decision_context(Action::WorkspaceUpdate, &authorized),
                "expected_revision": body.expected_revision,
                "before": workspace_image(&before),
                "after": workspace_image(&after),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(WorkspaceView::from(after)))
    }
    .await;
    respond(&state, "workspace.update", result).await
}

// ── Projects ─────────────────────────────────────────────────────────────────

/// `GET /v1/workspaces/{workspace_id}/projects`.
#[utoipa::path(
    get,
    path = "/v1/workspaces/{workspace_id}/projects",
    operation_id = "list_projects",
    tag = "projects",
    params(("workspace_id" = String, Path, description = "The workspace's id")),
    responses(
        (status = 200, description = "The workspace's projects", body = ProjectList),
        (status = 403, description = "The PDP denied `project.read`", body = ApiErrorBody),
        (status = 404, description = "No such workspace in this tenant", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn list_projects(
    State(state): State<AppState>,
    Path(workspace_id): Path<WorkspaceId>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        // 404 for an unknown workspace: an empty list must mean "no projects",
        // never "no such workspace".
        let workspace = workspaces::get(&mut *tx, tenant_id, workspace_id)
            .await?
            .ok_or_else(|| workspace_not_found(workspace_id))?;
        let (authorized, resource) = require(
            &state,
            &mut tx,
            Action::ProjectRead,
            tenant_id,
            Subject::Workspace(&workspace),
        )
        .await?;
        let projects = projects::in_workspace(&mut *tx, tenant_id, workspace_id).await?;
        read_event(
            &mut tx,
            tenant_id,
            "project.list",
            Action::ProjectRead,
            &authorized,
            resource,
            json!({"workspace_id": workspace_id, "count": projects.len()}),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(ProjectList {
            projects: projects.into_iter().map(Into::into).collect(),
        }))
    }
    .await;
    respond(&state, "project.list", result).await
}

/// `POST /v1/workspaces/{workspace_id}/projects`.
#[utoipa::path(
    post,
    path = "/v1/workspaces/{workspace_id}/projects",
    operation_id = "create_project",
    tag = "projects",
    params(
        ("workspace_id" = String, Path, description = "The workspace's id"),
        ("Idempotency-Key" = String, Header,
         description = "Required. A unique value per request, reused verbatim on retry."),
    ),
    request_body = CreateProjectBody,
    responses(
        (status = 201, description = "Created", body = ProjectView),
        (status = 200, description = "This key already created this project", body = ProjectView),
        (status = 400, description = "Malformed body, or no `Idempotency-Key`", body = ApiErrorBody),
        (status = 403, description = "The PDP denied `project.create`", body = ApiErrorBody),
        (status = 404, description = "No such workspace in this tenant", body = ApiErrorBody),
        (status = 409, description = "The slug is taken, the workspace is archived, or the key was reused", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn create_project(
    State(state): State<AppState>,
    Path(workspace_id): Path<WorkspaceId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<CreateProjectBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant_id = tenant_id()?;
        let subject = subject()?;
        let claim = Claim::from_headers(
            &headers,
            "project.create",
            &subject,
            &json!({
                "route": "POST /v1/workspaces/{workspace_id}/projects",
                "workspace_id": workspace_id,
                "slug": body.slug,
                "display_name": body.display_name,
                "description": body.description,
            }),
        )?;

        let replayed = match crate::idempotency::dispatch(&state.pool, tenant_id, &claim).await? {
            Dispatch::Replay(id) => Some(id),
            Dispatch::Create => {
                match make_project(&state, tenant_id, workspace_id, &body, &claim).await {
                    Ok(project) => {
                        return Ok((StatusCode::CREATED, Json(ProjectView::from(project))));
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
        let id = ProjectId::from_uuid(replayed.expect("replay id"));
        let project = replay_project(&state, tenant_id, id, &claim).await?;
        Ok((StatusCode::OK, Json(ProjectView::from(project))))
    }
    .await;
    respond(&state, "project.create", result).await
}

async fn make_project(
    state: &AppState,
    tenant_id: TenantId,
    workspace_id: WorkspaceId,
    body: &CreateProjectBody,
    claim: &Claim,
) -> Result<Project> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    // The parent workspace is the resource: creating a project inside one is
    // an authority over *that* workspace, which is exactly what a workspace
    // `owner` grant now confers without anybody minting a tenant-wide role.
    let workspace = workspaces::get(&mut *tx, tenant_id, workspace_id)
        .await?
        .ok_or_else(|| workspace_not_found(workspace_id))?;
    let (authorized, _) = require(
        state,
        &mut tx,
        Action::ProjectCreate,
        tenant_id,
        Subject::Workspace(&workspace),
    )
    .await?;
    let created_by = actor_identity(&mut tx, tenant_id).await?;
    let project = projects::create(
        &mut tx,
        &projects::NewProject {
            id: ProjectId::new(),
            tenant_id,
            workspace_id,
            slug: body.slug.clone(),
            display_name: body.display_name.clone(),
            description: body.description.clone(),
            created_by,
        },
    )
    .await?;
    let owner = mint_owner_grant(&mut tx, tenant_id, project.scope_id, &claim.subject).await?;
    claim
        .remember(&mut tx, tenant_id, project.id.as_uuid())
        .await?;
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::AccessGranted,
        Resource::Scope(project.scope_id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::ProjectCreate, &authorized),
            "grant": {
                "id": owner.id,
                "scope_id": owner.scope_id,
                "subject_kind": owner.subject_kind.as_str(),
                "principal_id": owner.principal_id,
                "role": owner.role_key.as_str(),
                "source": owner.source.as_str(),
            },
            "project_id": project.id,
        }),
    )
    .await?;
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::ProjectCreated,
        Resource::Scope(project.scope_id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::ProjectCreate, &authorized),
            "project": project_image(&project),
            "idempotency_key": claim.key,
        }),
    )
    .await?;
    commit(tx).await?;
    Ok(project)
}

async fn replay_project(
    state: &AppState,
    tenant_id: TenantId,
    id: ProjectId,
    claim: &Claim,
) -> Result<Project> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let project = projects::get(&mut *tx, tenant_id, id)
        .await?
        .ok_or_else(|| crate::idempotency::vanished(claim, id.as_uuid()))?;
    let workspace = workspaces::get(&mut *tx, tenant_id, project.workspace_id)
        .await?
        .ok_or_else(|| workspace_not_found(project.workspace_id))?;
    let (authorized, resource) = require(
        state,
        &mut tx,
        Action::ProjectCreate,
        tenant_id,
        Subject::Workspace(&workspace),
    )
    .await?;
    read_event(
        &mut tx,
        tenant_id,
        "project.create.replay",
        Action::ProjectCreate,
        &authorized,
        resource,
        json!({"project_id": id, "idempotency_key": claim.key}),
    )
    .await?;
    commit(tx).await?;
    Ok(project)
}

/// `GET /v1/projects/{project_id}`.
#[utoipa::path(
    get,
    path = "/v1/projects/{project_id}",
    operation_id = "get_project",
    tag = "projects",
    params(("project_id" = String, Path, description = "The project's id")),
    responses(
        (status = 200, description = "The project", body = ProjectView),
        (status = 403, description = "The PDP denied `project.read`", body = ApiErrorBody),
        (status = 404, description = "No such project in this tenant", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn get_project(
    State(state): State<AppState>,
    Path(project_id): Path<ProjectId>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let project = projects::get(&mut *tx, tenant_id, project_id)
            .await?
            .ok_or_else(|| project_not_found(project_id))?;
        let (authorized, resource) = require(
            &state,
            &mut tx,
            Action::ProjectRead,
            tenant_id,
            Subject::Project(&project),
        )
        .await?;
        read_event(
            &mut tx,
            tenant_id,
            "project.get",
            Action::ProjectRead,
            &authorized,
            resource,
            json!({"project_id": project_id}),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(ProjectView::from(project)))
    }
    .await;
    respond(&state, "project.get", result).await
}

/// `PATCH /v1/projects/{project_id}`.
#[utoipa::path(
    patch,
    path = "/v1/projects/{project_id}",
    operation_id = "update_project",
    tag = "projects",
    params(("project_id" = String, Path, description = "The project's id")),
    request_body = UpdateBody,
    responses(
        (status = 200, description = "The updated project", body = ProjectView),
        (status = 400, description = "Malformed body, or nothing to update", body = ApiErrorBody),
        (status = 403, description = "The PDP denied `project.update`", body = ApiErrorBody),
        (status = 404, description = "No such project in this tenant", body = ApiErrorBody),
        (status = 409, description = "`expected_revision` is not the current revision", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn update_project(
    State(state): State<AppState>,
    Path(project_id): Path<ProjectId>,
    payload: std::result::Result<Json<UpdateBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let before = projects::get(&mut *tx, tenant_id, project_id)
            .await?
            .ok_or_else(|| project_not_found(project_id))?;
        let (authorized, _) = require(
            &state,
            &mut tx,
            Action::ProjectUpdate,
            tenant_id,
            Subject::Project(&before),
        )
        .await?;
        let after = projects::update(
            &mut tx,
            tenant_id,
            project_id,
            body.expected_revision,
            &projects::ProjectUpdate {
                display_name: body.display_name.clone(),
                description: body.description.clone(),
                status: body.status,
            },
        )
        .await?;
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::ProjectUpdated,
            Resource::Scope(after.scope_id).to_string(),
            Outcome::Success,
            json!({
                "authz": audit::decision_context(Action::ProjectUpdate, &authorized),
                "expected_revision": body.expected_revision,
                "before": project_image(&before),
                "after": project_image(&after),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(ProjectView::from(after)))
    }
    .await;
    respond(&state, "project.update", result).await
}

// ── Repositories ─────────────────────────────────────────────────────────────

/// `GET /v1/projects/{project_id}/repositories`.
#[utoipa::path(
    get,
    path = "/v1/projects/{project_id}/repositories",
    operation_id = "list_repositories",
    tag = "repositories",
    params(("project_id" = String, Path, description = "The project's id")),
    responses(
        (status = 200, description = "What the project is about", body = RepositoryList),
        (status = 403, description = "The PDP denied `project.read`", body = ApiErrorBody),
        (status = 404, description = "No such project in this tenant", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn list_repositories(
    State(state): State<AppState>,
    Path(project_id): Path<ProjectId>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let project = projects::get(&mut *tx, tenant_id, project_id)
            .await?
            .ok_or_else(|| project_not_found(project_id))?;
        let (authorized, resource) = require(
            &state,
            &mut tx,
            Action::ProjectRead,
            tenant_id,
            Subject::Project(&project),
        )
        .await?;
        let repositories = repositories::for_project(&mut *tx, tenant_id, project_id).await?;
        read_event(
            &mut tx,
            tenant_id,
            "repository.list",
            Action::ProjectRead,
            &authorized,
            resource,
            json!({"project_id": project_id, "count": repositories.len()}),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(RepositoryList {
            repositories: repositories.into_iter().map(Into::into).collect(),
        }))
    }
    .await;
    respond(&state, "repository.list", result).await
}

/// `POST /v1/projects/{project_id}/repositories` — attach a repository.
#[utoipa::path(
    post,
    path = "/v1/projects/{project_id}/repositories",
    operation_id = "attach_repository",
    tag = "repositories",
    params(
        ("project_id" = String, Path, description = "The project's id"),
        ("Idempotency-Key" = String, Header,
         description = "Required. A unique value per request, reused verbatim on retry."),
    ),
    request_body = AttachRepositoryBody,
    responses(
        (status = 201, description = "Attached", body = RepositoryView),
        (status = 200, description = "This key already attached this repository", body = RepositoryView),
        (status = 400, description = "A filesystem path, a malformed remote, or no `Idempotency-Key`", body = ApiErrorBody),
        (status = 403, description = "The PDP denied `project.update`", body = ApiErrorBody),
        (status = 404, description = "No such project in this tenant", body = ApiErrorBody),
        (status = 409, description = "Already attached to this project, or the key was reused", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn attach_repository(
    State(state): State<AppState>,
    Path(project_id): Path<ProjectId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<AttachRepositoryBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant_id = tenant_id()?;
        let subject = subject()?;
        // Canonicalise before the claim, so the digest is over the *identity*
        // rather than over the spelling: `git@github.com:acme/x.git` and
        // `https://github.com/acme/x` are one request, and a retry that
        // switched forms must not read as a different one.
        let identity = synveda_types::repository::identify(
            body.remote_uri.as_deref(),
            body.local_fingerprint.as_deref(),
            body.name.as_deref(),
            body.provider,
        )?;
        let claim = Claim::from_headers(
            &headers,
            "repository.attach",
            &subject,
            &json!({
                "route": "POST /v1/projects/{project_id}/repositories",
                "project_id": project_id,
                "canonical_uri": identity.canonical_uri,
                "default_branch": body.default_branch,
            }),
        )?;

        let replayed = match crate::idempotency::dispatch(&state.pool, tenant_id, &claim).await? {
            Dispatch::Replay(id) => Some(id),
            Dispatch::Create => {
                match attach(&state, tenant_id, project_id, &body, identity, &claim).await {
                    Ok(repository) => {
                        return Ok((StatusCode::CREATED, Json(RepositoryView::from(repository))));
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
        let id = RepositoryId::from_uuid(replayed.expect("replay id"));
        let repository = replay_repository(&state, tenant_id, project_id, id, &claim).await?;
        Ok((StatusCode::OK, Json(RepositoryView::from(repository))))
    }
    .await;
    respond(&state, "repository.attach", result).await
}

async fn attach(
    state: &AppState,
    tenant_id: TenantId,
    project_id: ProjectId,
    body: &AttachRepositoryBody,
    identity: synveda_types::repository::RepositoryIdentity,
    claim: &Claim,
) -> Result<ProjectRepository> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let project = projects::get(&mut *tx, tenant_id, project_id)
        .await?
        .ok_or_else(|| project_not_found(project_id))?;
    let (authorized, resource) = require(
        state,
        &mut tx,
        Action::ProjectUpdate,
        tenant_id,
        Subject::Project(&project),
    )
    .await?;
    let created_by = actor_identity(&mut tx, tenant_id).await?;
    let repository = repositories::attach(
        &mut *tx,
        &repositories::NewRepository {
            id: RepositoryId::new(),
            tenant_id,
            project_id,
            identity,
            default_branch: body.default_branch.clone(),
            metadata: body.metadata.clone().unwrap_or_else(|| json!({})),
            created_by,
        },
    )
    .await?;
    claim
        .remember(&mut tx, tenant_id, repository.id.as_uuid())
        .await?;
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::ProjectRepositoryAttached,
        resource.to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::ProjectUpdate, &authorized),
            "repository": repository_image(&repository),
            "idempotency_key": claim.key,
        }),
    )
    .await?;
    commit(tx).await?;
    Ok(repository)
}

async fn replay_repository(
    state: &AppState,
    tenant_id: TenantId,
    project_id: ProjectId,
    id: RepositoryId,
    claim: &Claim,
) -> Result<ProjectRepository> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let project = projects::get(&mut *tx, tenant_id, project_id)
        .await?
        .ok_or_else(|| project_not_found(project_id))?;
    let (authorized, resource) = require(
        state,
        &mut tx,
        Action::ProjectUpdate,
        tenant_id,
        Subject::Project(&project),
    )
    .await?;
    // A repository is the one thing on this plane that can be detached, so a
    // replay can genuinely find nothing. That is a 404 naming the situation,
    // not a silent re-attach.
    let repository = repositories::get(&mut *tx, tenant_id, project_id, id)
        .await?
        .ok_or_else(|| crate::idempotency::vanished(claim, id.as_uuid()))?;
    read_event(
        &mut tx,
        tenant_id,
        "repository.attach.replay",
        Action::ProjectUpdate,
        &authorized,
        resource,
        json!({"repository_id": id, "idempotency_key": claim.key}),
    )
    .await?;
    commit(tx).await?;
    Ok(repository)
}

/// `DELETE /v1/projects/{project_id}/repositories/{repository_id}`.
#[utoipa::path(
    delete,
    path = "/v1/projects/{project_id}/repositories/{repository_id}",
    operation_id = "detach_repository",
    tag = "repositories",
    params(
        ("project_id" = String, Path, description = "The project's id"),
        ("repository_id" = String, Path, description = "The attachment's id"),
    ),
    responses(
        (status = 204, description = "Detached"),
        (status = 403, description = "The PDP denied `project.update`", body = ApiErrorBody),
        (status = 404, description = "No such attachment on this project", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn detach_repository(
    State(state): State<AppState>,
    Path((project_id, repository_id)): Path<(ProjectId, RepositoryId)>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let project = projects::get(&mut *tx, tenant_id, project_id)
            .await?
            .ok_or_else(|| project_not_found(project_id))?;
        let (authorized, resource) = require(
            &state,
            &mut tx,
            Action::ProjectUpdate,
            tenant_id,
            Subject::Project(&project),
        )
        .await?;
        let repository = repositories::get(&mut *tx, tenant_id, project_id, repository_id)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity: format!("repository {repository_id} on project {project_id}"),
            })?;
        if !repositories::detach(&mut *tx, tenant_id, project_id, repository_id).await? {
            return Err(Error::NotFound {
                entity: format!("repository {repository_id} on project {project_id}"),
            });
        }
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::ProjectRepositoryDetached,
            resource.to_string(),
            Outcome::Success,
            json!({
                "authz": audit::decision_context(Action::ProjectUpdate, &authorized),
                "repository": repository_image(&repository),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(StatusCode::NO_CONTENT)
    }
    .await;
    respond(&state, "repository.detach", result).await
}

/// Mints the `owner` grant for whoever created a workspace or a project
/// (CPR-5, ADR-0072 decision 1), in the creating transaction.
///
/// A collaboration space that nobody is a member of is not one — and the person
/// who made it is the one member the product can name without being told. So
/// creation grants `owner` at the scope it just minted, with the source
/// `owner`, which is the one source no route hands out.
///
/// It runs in the same transaction as the workspace and its scope, so the three
/// outcomes are all or none: there is no window in which a workspace exists
/// with nobody able to administer it.
///
/// It is keyed by the creator's **token subject**, so it works for a caller who
/// has never provisioned an identity — which is why grants are subject-keyed at
/// all (ADR-0072 decision 4). Contrast `created_by` on the row beside it, which
/// is an `IdentityId` and is therefore absent for exactly those callers.
async fn mint_owner_grant(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    scope_id: ScopeId,
    subject: &str,
) -> Result<synveda_types::access::ScopeGrant> {
    synveda_store::access::create_grant(
        &mut *tx,
        &synveda_store::access::NewGrant {
            id: synveda_types::GrantId::new(),
            tenant_id,
            scope_id,
            subject: synveda_types::access::GrantSubject::Principal {
                principal_id: subject.to_owned(),
            },
            role_key: synveda_types::access::RoleKey::Owner,
            source: synveda_types::access::GrantSource::Owner,
            invite_id: None,
            granted_by: Some(subject.to_owned()),
        },
    )
    .await
}

/// The acting identity's id, for the `created_by` provenance column.
///
/// `None` for a verified subject with no identity row — a dev HS256 token, or
/// a service client that never provisioned. Recorded as absent rather than
/// invented: "the deployment created this" and "somebody we cannot name
/// created this" are the same column value, and neither is a lie.
pub(crate) async fn actor_identity(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
) -> Result<Option<IdentityId>> {
    let subject = subject()?;
    Ok(
        synveda_store::identities::by_subject(&mut *tx, tenant_id, &subject)
            .await?
            .map(|identity| identity.id),
    )
}

/// The taxonomy error body, declared for the OpenAPI document.
///
/// A schema-only mirror of `synveda_types::Error`'s serialised form, which is
/// `{"kind": "...", ...}` with a per-variant remainder. It exists because the
/// contract has to say what a 4xx body looks like and the taxonomy lives two
/// crates down, where `utoipa` deliberately does not reach — the OpenAPI
/// derive is a property of the surface, and `synveda-types` is not one.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ApiErrorBody)]
pub struct ApiErrorBody {
    /// The stable machine-readable code — `invalid`, `conflict`, `not_found`,
    /// `policy_denied`, …
    pub kind: String,
    /// Present on most variants; what went wrong, safe to show a caller.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// `policy_denied` only: the action that was attempted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// `policy_denied` only: what was acted on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    /// `policy_denied` only: which policy produced the denial.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// `not_found` only: what was looked up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three-case `description` is the whole reason `double_option`
    /// exists, and the case a plain `Option` loses is the middle one.
    #[test]
    fn an_update_body_tells_absent_from_null() {
        let absent: UpdateBody = serde_json::from_str(r#"{"expected_revision": 3}"#).unwrap();
        assert_eq!(absent.description, None, "absent leaves it alone");

        let cleared: UpdateBody =
            serde_json::from_str(r#"{"expected_revision": 3, "description": null}"#).unwrap();
        assert_eq!(cleared.description, Some(None), "null clears it");

        let set: UpdateBody =
            serde_json::from_str(r#"{"expected_revision": 3, "description": "why"}"#).unwrap();
        assert_eq!(set.description, Some(Some("why".to_owned())));
    }

    #[test]
    fn an_update_without_a_precondition_is_refused_by_the_wire() {
        assert!(
            serde_json::from_str::<UpdateBody>(r#"{"display_name": "Payments"}"#).is_err(),
            "expected_revision is required, not defaulted"
        );
    }

    #[test]
    fn unknown_fields_are_refused_rather_than_ignored() {
        assert!(
            serde_json::from_str::<CreateWorkspaceBody>(
                r#"{"slug": "a", "display_name": "A", "colour": "blue"}"#
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<AttachRepositoryBody>(
                r#"{"remote_uri": "https://github.com/a/b", "path": "/tmp/b"}"#
            )
            .is_err(),
            "`path` must not be silently ignored: it is the thing this feature refuses"
        );
    }

    /// The OpenAPI vocabularies are built from the Rust ones, so this asserts
    /// the mechanism rather than a transcription.
    #[test]
    fn the_wire_vocabularies_come_from_the_domain_enums() {
        let status = lifecycle_status_schema();
        let rendered = serde_json::to_value(&status).expect("schema serialises");
        assert_eq!(rendered["enum"], json!(["active", "archived"]));

        let provider = repository_provider_schema();
        let rendered = serde_json::to_value(&provider).expect("schema serialises");
        assert_eq!(
            rendered["enum"],
            json!([
                "github",
                "gitlab",
                "bitbucket",
                "azure_devops",
                "generic_git",
                "local"
            ])
        );
    }
}
