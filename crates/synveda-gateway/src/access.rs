//! The access plane (CPR-5, ADR-0072): who may act on a workspace or a
//! project, the groups authority is handed to, and the invitations that hand
//! it out.
//!
//! Fourteen operations across ten paths, behind tenant resolution like every
//! `/v1` route, behind the PDP like every governed one, and chaining an audit
//! event for every mutation.
//!
//! # Where these decisions are taken
//!
//! Every decision here named `Resource::Tenant` until CPR-6, and ADR-0072
//! decision 9 recorded that as a stated debt rather than a design. It is paid:
//! a membership read or a grant names the **scope** it is about — the
//! workspace, the project, or the scope a tenant-wide grant route was given —
//! curating a group names the **group**, and revoking names **the grant**,
//! which is what lets a pack price taking away a directory-managed grant
//! differently from taking away one somebody typed (ADR-0073 decision 3).
//!
//! The two that stay on the tenant plane stay there for reasons rather than
//! for want of a resource: creating a group has no group to name yet, and
//! redeeming an invitation must work for somebody who holds nothing anywhere.
//! Both are decided at the tenant **root scope** when the tenant has one.
//!
//! # Grants are a PDP input
//!
//! This plane records who holds which **role key** where, and since CPR-6 the
//! PDP reads them: `synveda_store::anchors` resolves the caller's grants into
//! an ordered anchor set, and the role keys that reach the resource arrive in
//! `context.roles` beside the old hierarchy's binding roles (ADR-0073
//! decision 5). A workspace `owner` therefore administers their workspace with
//! no tenant-wide role bound anywhere, and a project-only grantee reaches the
//! project and not the workspace above it.
//!
//! Nothing translates between the two vocabularies, and nothing needs to: the
//! two trees are disjoint, so a grant's scope is never a hierarchy node and a
//! node binding is never at a governed scope.
//!
//! # Creation is idempotent; the group update carries a precondition
//!
//! The plane's two rules, unchanged from CPR-4: `POST` takes a required
//! `Idempotency-Key`, `PATCH` takes a required `expected_revision`. The one
//! exception is redeeming an invitation, which is idempotent by construction —
//! see [`accept_invite`].

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{Action, Resource, ResourceEntity};
use synveda_store::access::AccessEntry as StoreAccessEntry;
use synveda_store::anchors::AnchorSelection;
use synveda_store::{access, projects, rls, scopes, workspaces};
use synveda_types::access::{
    DEFAULT_INVITE_TTL_SECS, GrantSource, GrantSubject, Group, GroupSource, InviteStatus,
    PendingInvite, RoleKey, ScopeGrant, SubjectKind, validate_invite_ttl,
};
use synveda_types::scope::Scope;
use synveda_types::workspace::{LifecycleStatus, Project, Workspace};
use synveda_types::{
    Error, GrantId, GroupId, InviteId, ProjectId, Result, ScopeId, TenantId, WorkspaceId,
};
use utoipa::ToSchema;

use crate::app::AppState;
use crate::audit;
use crate::authz::{self, Authorized};
use crate::error::ApiError;
use crate::idempotency::{Claim, Dispatch};
use crate::request::{body, commit, tenant_id};
use crate::telemetry::ACCESS_OPERATIONS_TOTAL;
use crate::workspaces::{ApiErrorBody, string_enum, subject};

// ── Views ────────────────────────────────────────────────────────────────────

/// One principal's one role at one scope, with everything a reader needs to
/// answer **why**.
///
/// The `source`, `scope_id`, `inherited` and `via_group` fields together are
/// the whole of "access-source visibility": a person looking at a project's
/// member list can see that Robin is there because somebody granted them
/// `member` at the workspace, or because they are in the `engineering` group,
/// or because a directory said so — without reading an audit log.
#[derive(Debug, Serialize, ToSchema)]
pub struct MemberView {
    /// The grant this came from — what a revocation names.
    #[schema(value_type = String, format = "uuid")]
    pub grant_id: GrantId,
    /// The principal, by verified token subject.
    pub principal_id: String,
    /// What they hold here.
    #[schema(schema_with = role_key_schema)]
    pub role: RoleKey,
    /// Where the grant came from.
    #[schema(schema_with = grant_source_schema)]
    pub source: GrantSource,
    /// The scope the grant is actually written at — **not** necessarily the
    /// one that was asked about.
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    /// Whether it was inherited from an ancestor scope rather than written
    /// here. A client that offers "remove" on an inherited row is offering
    /// something the API will refuse, so this is the field that decides.
    pub inherited: bool,
    /// The group it reached them through, when it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via_group: Option<GroupRefView>,
    /// Whether a directory manages it, and it therefore cannot be edited here.
    pub directory_managed: bool,
    /// When the grant was made.
    pub granted_at: DateTime<Utc>,
}

impl From<StoreAccessEntry> for MemberView {
    fn from(entry: StoreAccessEntry) -> Self {
        MemberView {
            grant_id: entry.grant_id,
            principal_id: entry.principal_id,
            role: entry.role_key,
            source: entry.source,
            scope_id: entry.scope_id,
            inherited: entry.inherited,
            via_group: entry.via_group.map(|group| GroupRefView {
                id: group.id,
                slug: group.slug,
            }),
            directory_managed: entry.directory_managed,
            granted_at: entry.granted_at,
        }
    }
}

/// A group, named enough to render without a second call.
#[derive(Debug, Serialize, ToSchema)]
pub struct GroupRefView {
    /// The group's id.
    #[schema(value_type = String, format = "uuid")]
    pub id: GroupId,
    /// Its handle.
    pub slug: String,
}

/// The member listing.
#[derive(Debug, Serialize, ToSchema)]
pub struct MemberList {
    /// Everybody who holds a role here, nearest grant first. One entry per
    /// (principal, role): somebody holding two roles appears twice, because
    /// the two came from different grants and are revoked separately.
    pub members: Vec<MemberView>,
}

/// A group, as the API serves it.
#[derive(Debug, Serialize, ToSchema)]
pub struct GroupView {
    /// Stable id.
    #[schema(value_type = String, format = "uuid")]
    pub id: GroupId,
    /// Tenant-unique handle. Immutable.
    pub slug: String,
    /// Display name.
    pub display_name: String,
    /// Optional prose.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whose group it is.
    #[schema(schema_with = group_source_schema)]
    pub source: GroupSource,
    /// The external id a directory knows it by, when one does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory_ref: Option<String>,
    /// Whether the group is in use. An **archived** group confers nothing:
    /// every grant naming it resolves to nobody.
    #[schema(schema_with = lifecycle_status_schema)]
    pub status: LifecycleStatus,
    /// The revision an update must name as its precondition.
    pub revision: i64,
    /// Its members, by principal id.
    pub members: Vec<String>,
    /// Who created it, when a caller did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    /// When it was created.
    pub created_at: DateTime<Utc>,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
}

impl GroupView {
    fn build(group: Group, members: Vec<String>) -> Self {
        GroupView {
            id: group.id,
            slug: group.slug,
            display_name: group.display_name,
            description: group.description,
            source: group.source,
            directory_ref: group.directory_ref,
            status: group.status,
            revision: group.revision,
            members,
            created_by: group.created_by,
            created_at: group.created_at,
            updated_at: group.updated_at,
        }
    }
}

/// The group listing.
#[derive(Debug, Serialize, ToSchema)]
pub struct GroupList {
    /// The tenant's groups, by slug. Archived ones included: a listing that
    /// omitted them would make an archived group indistinguishable from one
    /// that never existed.
    pub groups: Vec<GroupView>,
}

/// A grant, as the API serves it.
///
/// The subject is **flattened** into `subject_kind` plus one of two id fields
/// rather than nested as a tagged union. A tagged union is the tidier model and
/// the worse contract: it renders as a `oneOf` that the frontend generator
/// would have to discriminate, and this document's whole point is that the
/// types on both ends are derived rather than hand-reconciled.
#[derive(Debug, Serialize, ToSchema)]
pub struct GrantView {
    /// Stable id — what a revocation names.
    #[schema(value_type = String, format = "uuid")]
    pub id: GrantId,
    /// The scope the grant is at. Its subtree inherits it.
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    /// Which of the two subject fields is populated.
    #[schema(schema_with = subject_kind_schema)]
    pub subject_kind: SubjectKind,
    /// The principal, for a `principal` grant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal_id: Option<String>,
    /// The group, for a `group` grant.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub group_id: Option<GroupId>,
    /// What the subject holds.
    #[schema(schema_with = role_key_schema)]
    pub role: RoleKey,
    /// Where it came from.
    #[schema(schema_with = grant_source_schema)]
    pub source: GrantSource,
    /// The invitation that produced it, for an `invite` grant.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub invite_id: Option<InviteId>,
    /// Who granted it, when a caller did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub granted_by: Option<String>,
    /// Whether a directory manages it, and it therefore cannot be revoked
    /// here.
    pub directory_managed: bool,
    /// When it was made.
    pub created_at: DateTime<Utc>,
}

impl From<ScopeGrant> for GrantView {
    fn from(grant: ScopeGrant) -> Self {
        GrantView {
            id: grant.id,
            scope_id: grant.scope_id,
            subject_kind: grant.subject_kind,
            principal_id: grant.principal_id,
            group_id: grant.group_id,
            role: grant.role_key,
            source: grant.source,
            invite_id: grant.invite_id,
            granted_by: grant.granted_by,
            directory_managed: grant.source.is_directory_managed(),
            created_at: grant.created_at,
        }
    }
}

/// The grant listing.
#[derive(Debug, Serialize, ToSchema)]
pub struct GrantList {
    /// The grants this filter selected, oldest first. These are the **rows**,
    /// not the authority in force anywhere: a workspace grant appears once
    /// here and reaches every project inside it. `GET /v1/projects/{id}/members`
    /// is the other question.
    pub grants: Vec<GrantView>,
}

/// An invitation, as the API serves it.
///
/// The token is **not here**, on any route, ever. It appears once, in
/// [`CreatedInviteView`], in the response to the request that minted it.
#[derive(Debug, Serialize, ToSchema)]
pub struct InviteView {
    /// Stable id — what a withdrawal names.
    #[schema(value_type = String, format = "uuid")]
    pub id: InviteId,
    /// The scope it grants at.
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    /// What it grants.
    #[schema(schema_with = role_key_schema)]
    pub role: RoleKey,
    /// Who it was meant for, when the inviter said.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Where it stands. `expired` is **derived** from `expires_at` at read
    /// time and never stored, so an invitation stops working at the instant it
    /// says it will rather than when a sweep next runs.
    #[schema(schema_with = invite_status_schema)]
    pub status: InviteStatus,
    /// When it stops being redeemable.
    pub expires_at: DateTime<Utc>,
    /// Who issued it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    /// When.
    pub created_at: DateTime<Utc>,
    /// The principal that redeemed it, when somebody has.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted_by: Option<String>,
    /// When it was redeemed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted_at: Option<DateTime<Utc>>,
}

impl InviteView {
    fn at(invite: PendingInvite, now: DateTime<Utc>) -> Self {
        InviteView {
            status: invite.effective_status(now),
            id: invite.id,
            scope_id: invite.scope_id,
            role: invite.role_key,
            email: invite.email,
            expires_at: invite.expires_at,
            created_by: invite.created_by,
            created_at: invite.created_at,
            accepted_by: invite.accepted_by,
            accepted_at: invite.accepted_at,
        }
    }
}

/// The response to creating an invitation — the **one and only** place the
/// token exists.
///
/// It is not stored (only its SHA-256 is), not logged, not in the audit
/// payload and not on any other route. An inviter who loses it withdraws the
/// invitation and issues another, which is one action; the alternative is a
/// product that can show you somebody else's live credential.
#[derive(Debug, Serialize, ToSchema)]
pub struct CreatedInviteView {
    /// The invitation.
    pub invite: InviteView,
    /// The token. **Shown once.**
    pub token: String,
    /// The URL the recipient posts to, with their own credential, to redeem
    /// it. Enough for a local or demo deployment to invite by copying a link —
    /// email delivery is deliberately not a requirement of this feature.
    pub accept_url: String,
}

/// The invitation listing.
#[derive(Debug, Serialize, ToSchema)]
pub struct InviteList {
    /// Every invitation issued at this scope, newest first — redeemed and
    /// withdrawn ones included, because "who was invited here and what
    /// happened" is the question this answers.
    pub invites: Vec<InviteView>,
}

/// The response to redeeming an invitation.
#[derive(Debug, Serialize, ToSchema)]
pub struct AcceptedInviteView {
    /// The grant it minted — or the one the acceptor already held.
    pub grant: GrantView,
    /// The scope they now hold it at.
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
}

fn role_key_schema() -> utoipa::openapi::schema::Object {
    string_enum(RoleKey::ALL.iter().map(RoleKey::as_str))
}

fn grant_source_schema() -> utoipa::openapi::schema::Object {
    string_enum(GrantSource::ALL.iter().map(GrantSource::as_str))
}

fn group_source_schema() -> utoipa::openapi::schema::Object {
    string_enum(GroupSource::ALL.iter().map(GroupSource::as_str))
}

fn subject_kind_schema() -> utoipa::openapi::schema::Object {
    string_enum(SubjectKind::ALL.iter().map(SubjectKind::as_str))
}

fn invite_status_schema() -> utoipa::openapi::schema::Object {
    string_enum(InviteStatus::ALL.iter().map(InviteStatus::as_str))
}

fn lifecycle_status_schema() -> utoipa::openapi::schema::Object {
    string_enum(LifecycleStatus::ALL.iter().map(LifecycleStatus::as_str))
}

// ── Request bodies ───────────────────────────────────────────────────────────

/// `POST /v1/workspaces/{workspace_id}/invites`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInviteBody {
    /// The role the invitation carries.
    #[schema(schema_with = role_key_schema)]
    pub role: RoleKey,
    /// Who it is meant for. Optional: an invitation with no address is a link
    /// the inviter copies, redeemable once by whoever presents it first.
    #[serde(default)]
    pub email: Option<String>,
    /// How long it stands, in seconds. Defaults to seven days and is capped at
    /// thirty — an invitation that never expires is a key left under the mat.
    #[serde(default)]
    pub expires_in_secs: Option<i64>,
}

/// `POST /v1/projects/{project_id}/members` and `POST /v1/admin/grants`.
///
/// Exactly one of `principal_id` and `group_id`. Two flat fields rather than a
/// tagged union, for [`GrantView`]'s reason.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GrantSubjectBody {
    /// The principal, by verified token subject.
    #[serde(default)]
    pub principal_id: Option<String>,
    /// The group.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub group_id: Option<GroupId>,
    /// What they get.
    #[schema(schema_with = role_key_schema)]
    pub role: RoleKey,
    /// The scope. Required on `/v1/admin/grants`, where the caller chooses;
    /// **refused** on the project route, where the path already says which
    /// scope — a body that could name a different one would make the path a
    /// suggestion.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub scope_id: Option<ScopeId>,
}

impl GrantSubjectBody {
    /// The subject, or the refusal for a body that named both or neither.
    fn subject(&self) -> Result<GrantSubject> {
        match (self.principal_id.as_deref(), self.group_id) {
            (Some(principal_id), None) => Ok(GrantSubject::Principal {
                principal_id: principal_id.to_owned(),
            }),
            (None, Some(group_id)) => Ok(GrantSubject::Group { group_id }),
            (Some(_), Some(_)) => Err(Error::Invalid {
                message: "a grant names one subject: send principal_id or group_id, not both"
                    .to_owned(),
            }),
            (None, None) => Err(Error::Invalid {
                message: "a grant names a subject: send principal_id or group_id".to_owned(),
            }),
        }
    }

    /// The canonical image of this body, for the idempotency digest.
    fn canonical(&self, route: &str) -> serde_json::Value {
        json!({
            "route": route,
            "principal_id": self.principal_id,
            "group_id": self.group_id,
            "role": self.role.as_str(),
            "scope_id": self.scope_id,
        })
    }
}

/// `POST /v1/admin/groups`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateGroupBody {
    /// Tenant-unique handle: `^[a-z0-9][a-z0-9-]{0,62}$`. Immutable.
    pub slug: String,
    /// Display name.
    pub display_name: String,
    /// Optional prose. Blank is refused; omit it instead.
    #[serde(default)]
    pub description: Option<String>,
    /// Its members at creation, by principal id.
    #[serde(default)]
    pub members: Vec<String>,
}

/// `PATCH /v1/admin/groups/{group_id}`.
///
/// `members` is a **full replacement**, not a delta: a membership list has no
/// precondition of its own, so add/remove pairs race — two callers each
/// removing one person can both succeed and leave a list neither intended. A
/// replacement under `expected_revision` cannot.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateGroupBody {
    /// The revision the caller last saw. Required, for the reason the
    /// workspace plane's is: an update without a precondition is a
    /// last-writer-wins update.
    pub expected_revision: i64,
    /// New display name.
    #[serde(default)]
    pub display_name: Option<String>,
    /// New description; `null` clears it.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    /// New lifecycle status. An archived group confers nothing.
    #[serde(default)]
    #[schema(schema_with = lifecycle_status_schema)]
    pub status: Option<LifecycleStatus>,
    /// The complete membership after this update.
    #[serde(default)]
    pub members: Option<Vec<String>>,
}

/// Distinguishes an absent field from an explicit `null` — the workspace
/// plane's helper, for its reason.
fn double_option<'de, T, D>(deserializer: D) -> std::result::Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
}

/// `GET /v1/admin/grants` query.
///
/// `IntoParams` rather than `ToSchema`: these are query parameters, and the
/// document describes them on the operation rather than as a named schema
/// somebody could mistake for a body.
#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct GrantQuery {
    /// Only grants written **at** this scope.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "uuid")]
    pub scope_id: Option<ScopeId>,
    /// Only grants naming this principal directly.
    #[serde(default)]
    pub principal_id: Option<String>,
}

// ── Shared handler plumbing ──────────────────────────────────────────────────

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
    metrics::counter!(ACCESS_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    match result {
        Ok(response) => response.into_response(),
        Err(error) => {
            audit::record_rejection(state, op, &error).await;
            ApiError(error).into_response()
        }
    }
}

/// What a decision on this plane is **about** (CPR-6, ADR-0073 decision 3).
///
/// CPR-5 named the tenant for all thirteen and recorded why: the Cedar entity
/// model still materialised `Scope` from `hierarchy_nodes`, so a governed
/// scope had no chain to decide against. It has one now, so a membership read
/// or a grant names the scope it is actually about, curating a group names the
/// group, and revoking names **the grant** — which is what lets a pack price
/// taking away a directory-managed grant differently from taking away one
/// somebody typed.
enum Subject<'a> {
    /// The tenant plane: the tenant-wide listings and the group collection.
    /// Decided at the tenant **root scope** when the tenant has one, and at the
    /// tenant itself before anything has minted one.
    Tenant,
    /// The tenant plane for an action the schema gives **no scope resource**:
    /// `GroupManage` on the collection and `InviteAccept`. Naming the root
    /// scope for those would build a request the Cedar schema refuses, which
    /// fails closed as an internal error rather than as a denial — so they name
    /// the tenant, and the anchors still carry a tenant-root grant into
    /// `context.roles` (ADR-0073 decision 5).
    TenantOnly,
    /// One governed scope, with the workspace or project that owns it when a
    /// subtype does.
    Scope(&'a Scope, Option<ResourceEntity>, AnchorSelection),
    /// One group.
    Group(GroupId),
    /// One grant, at the scope it is written at.
    Grant(&'a ScopeGrant, &'a Scope),
}

/// Takes one decision about `subject`, and returns the resource it decided
/// against so the audit event names the same thing the PDP did.
async fn require(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    action: Action,
    tenant_id: TenantId,
    subject: Subject<'_>,
) -> Result<(Authorized, Resource)> {
    let (anchor, selection, resources, resource) = match subject {
        Subject::Tenant => {
            let root = scopes::tenant_root(&mut *tx, tenant_id).await?;
            let resource = match &root {
                Some(scope) => Resource::Scope(scope.id),
                None => Resource::Tenant(tenant_id),
            };
            (root, AnchorSelection::none(), Vec::new(), resource)
        }
        Subject::Scope(scope, entity, selection) => {
            let resource = match entity {
                Some(ResourceEntity::Workspace { id, .. }) => Resource::Workspace(id),
                Some(ResourceEntity::Project { id, .. }) => Resource::Project(id),
                _ => Resource::Scope(scope.id),
            };
            (
                Some(scope.clone()),
                selection,
                entity.into_iter().collect(),
                resource,
            )
        }
        Subject::TenantOnly => {
            // The root is still gathered, so the profile assigned to it is the
            // one that decides; only the *resource* differs.
            let root = scopes::tenant_root(&mut *tx, tenant_id).await?;
            (
                root,
                AnchorSelection::none(),
                Vec::new(),
                Resource::Tenant(tenant_id),
            )
        }
        Subject::Group(id) => {
            // A group is anchored nowhere in the tree, so it is decided under
            // the tenant root's profile — the same shape `DirectoryManage` has
            // and for the same reason: a subtree-scoped authority over a
            // tenant-wide set could only ever be a half-truth.
            let root = scopes::tenant_root(&mut *tx, tenant_id).await?;
            (
                root,
                AnchorSelection::none(),
                vec![ResourceEntity::Group { id }],
                Resource::Group(id),
            )
        }
        Subject::Grant(grant, scope) => (
            Some(scope.clone()),
            AnchorSelection::none(),
            vec![ResourceEntity::Grant {
                id: grant.id,
                scope_id: grant.scope_id,
                role: grant.role_key,
                source: grant.source,
            }],
            Resource::Grant(grant.id),
        ),
    };
    let input = authz::gather(state, tx, anchor.as_ref(), selection, resources).await?;
    let authorized = authz::decide(state, &input, action, resource)?;
    Ok((authorized, resource))
}

/// [`Subject::Scope`] for a workspace.
fn workspace_subject<'a>(workspace: &Workspace, scope: &'a Scope) -> Subject<'a> {
    Subject::Scope(
        scope,
        Some(ResourceEntity::Workspace {
            id: workspace.id,
            scope_id: workspace.scope_id,
        }),
        AnchorSelection::workspace(workspace.id),
    )
}

/// [`Subject::Scope`] for a project.
fn project_subject<'a>(project: &Project, scope: &'a Scope) -> Subject<'a> {
    Subject::Scope(
        scope,
        Some(ResourceEntity::Project {
            id: project.id,
            scope_id: project.scope_id,
            workspace_id: project.workspace_id,
        }),
        AnchorSelection::project(project.id),
    )
}

/// The workspace and the governed scope it owns, or the uniform 404.
async fn workspace_and_scope(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    id: WorkspaceId,
) -> Result<(Workspace, Scope)> {
    let workspace = workspaces::get(&mut *tx, tenant_id, id)
        .await?
        .ok_or_else(|| workspace_not_found(id))?;
    let scope = scopes::get(&mut *tx, tenant_id, workspace.scope_id)
        .await?
        .ok_or_else(|| workspace_not_found(id))?;
    Ok((workspace, scope))
}

/// The project and the governed scope it owns, or the uniform 404.
async fn project_and_scope(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    id: ProjectId,
) -> Result<(Project, Scope)> {
    let project = projects::get(&mut *tx, tenant_id, id)
        .await?
        .ok_or_else(|| project_not_found(id))?;
    let scope = scopes::get(&mut *tx, tenant_id, project.scope_id)
        .await?
        .ok_or_else(|| project_not_found(id))?;
    Ok((project, scope))
}

/// The allowed-read decision event (ADR-0019 decision 4): a read has no
/// semantic event of its own, so the decision itself chains.
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

/// The payload image of a grant.
///
/// It carries the subject, the role, the scope and the source, which together
/// are what an auditor needs to answer "who could act here on that day". It
/// deliberately carries no invitation *token* — the invitation's **id** is the
/// provenance, and the token is a live credential.
fn grant_image(grant: &ScopeGrant) -> serde_json::Value {
    json!({
        "id": grant.id,
        "scope_id": grant.scope_id,
        "subject_kind": grant.subject_kind.as_str(),
        "principal_id": grant.principal_id,
        "group_id": grant.group_id,
        "role": grant.role_key.as_str(),
        "source": grant.source.as_str(),
        "invite_id": grant.invite_id,
    })
}

/// The payload image of an invitation. Never the token, and never the hash
/// either: a hash of 256 bits of entropy is not brute-forceable, but a chain
/// that carried one would be a chain somebody has to reason about.
fn invite_image(invite: &PendingInvite) -> serde_json::Value {
    json!({
        "id": invite.id,
        "scope_id": invite.scope_id,
        "role": invite.role_key.as_str(),
        "email": invite.email,
        "status": invite.status.as_str(),
        "expires_at": invite.expires_at,
    })
}

fn group_image(group: &Group) -> serde_json::Value {
    json!({
        "id": group.id,
        "slug": group.slug,
        "display_name": group.display_name,
        "description": group.description,
        "source": group.source.as_str(),
        "status": group.status.as_str(),
        "revision": group.revision,
    })
}

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

// ── Members ──────────────────────────────────────────────────────────────────

/// `GET /v1/workspaces/{workspace_id}/members` — who may act here.
#[utoipa::path(
    get,
    path = "/v1/workspaces/{workspace_id}/members",
    operation_id = "list_workspace_members",
    tag = "access",
    params(("workspace_id" = String, Path, description = "The workspace's id")),
    responses(
        (status = 200, description = "Everybody who holds a role here", body = MemberList),
        (status = 403, description = "The PDP denied `membership.read`", body = ApiErrorBody),
        (status = 404, description = "No such workspace in this tenant", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn list_workspace_members(
    State(state): State<AppState>,
    Path(workspace_id): Path<WorkspaceId>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let (workspace, scope) = workspace_and_scope(&mut tx, tenant_id, workspace_id).await?;
        let scope_id = scope.id;
        let (authorized, resource) = require(
            &state,
            &mut tx,
            Action::MembershipRead,
            tenant_id,
            workspace_subject(&workspace, &scope),
        )
        .await?;
        let members = access::members_of(&mut *tx, tenant_id, scope_id).await?;
        read_event(
            &mut tx,
            tenant_id,
            "members.list",
            Action::MembershipRead,
            &authorized,
            resource,
            json!({"workspace_id": workspace_id, "scope_id": scope_id, "count": members.len()}),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(MemberList {
            members: members.into_iter().map(Into::into).collect(),
        }))
    }
    .await;
    respond(&state, "members.list", result).await
}

/// `GET /v1/projects/{project_id}/members` — who may act here, **including
/// what the workspace above it grants**.
#[utoipa::path(
    get,
    path = "/v1/projects/{project_id}/members",
    operation_id = "list_project_members",
    tag = "access",
    params(("project_id" = String, Path, description = "The project's id")),
    responses(
        (status = 200, description = "Everybody who holds a role here, inherited grants included", body = MemberList),
        (status = 403, description = "The PDP denied `membership.read`", body = ApiErrorBody),
        (status = 404, description = "No such project in this tenant", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn list_project_members(
    State(state): State<AppState>,
    Path(project_id): Path<ProjectId>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let (project, scope) = project_and_scope(&mut tx, tenant_id, project_id).await?;
        let scope_id = scope.id;
        let (authorized, resource) = require(
            &state,
            &mut tx,
            Action::MembershipRead,
            tenant_id,
            project_subject(&project, &scope),
        )
        .await?;
        let members = access::members_of(&mut *tx, tenant_id, scope_id).await?;
        read_event(
            &mut tx,
            tenant_id,
            "members.list",
            Action::MembershipRead,
            &authorized,
            resource,
            json!({"project_id": project_id, "scope_id": scope_id, "count": members.len()}),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(MemberList {
            members: members.into_iter().map(Into::into).collect(),
        }))
    }
    .await;
    respond(&state, "members.list", result).await
}

/// `POST /v1/projects/{project_id}/members` — a **project-only** grant.
#[utoipa::path(
    post,
    path = "/v1/projects/{project_id}/members",
    operation_id = "add_project_member",
    tag = "access",
    params(
        ("project_id" = String, Path, description = "The project's id"),
        ("Idempotency-Key" = String, Header,
         description = "Required. A unique value per request, reused verbatim on retry."),
    ),
    request_body = GrantSubjectBody,
    responses(
        (status = 201, description = "Granted", body = GrantView),
        (status = 200, description = "This key already made this grant", body = GrantView),
        (status = 400, description = "No subject, both subjects, a `scope_id`, or no `Idempotency-Key`", body = ApiErrorBody),
        (status = 403, description = "The PDP denied `membership.grant`", body = ApiErrorBody),
        (status = 404, description = "No such project in this tenant", body = ApiErrorBody),
        (status = 409, description = "They already hold this role here, or the key was reused", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn add_project_member(
    State(state): State<AppState>,
    Path(project_id): Path<ProjectId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<GrantSubjectBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        if body.scope_id.is_some() {
            // The path already names the scope. A body that could name a
            // different one would make the path a suggestion, and the audit
            // event would record a project this grant is not on.
            return Err(Error::Invalid {
                message: "this route grants at the project in its path; \
                          use POST /v1/admin/grants to name a scope"
                    .to_owned(),
            });
        }
        let subject_claim = body.subject()?;
        let tenant_id = tenant_id()?;
        let actor = subject()?;
        let claim = Claim::from_headers(
            &headers,
            "member.add",
            &actor,
            &json!({
                "project_id": project_id,
                "body": body.canonical("POST /v1/projects/{project_id}/members"),
            }),
        )?;

        let replayed = match crate::idempotency::dispatch(&state.pool, tenant_id, &claim).await? {
            Dispatch::Replay(id) => Some(id),
            Dispatch::Create => {
                match add_member(
                    &state,
                    tenant_id,
                    project_id,
                    &subject_claim,
                    body.role,
                    &claim,
                )
                .await
                {
                    Ok(grant) => return Ok((StatusCode::CREATED, Json(GrantView::from(grant)))),
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
        let id = GrantId::from_uuid(replayed.expect("replay id"));
        let grant = replay_grant(&state, tenant_id, id, &claim).await?;
        Ok((StatusCode::OK, Json(GrantView::from(grant))))
    }
    .await;
    respond(&state, "member.add", result).await
}

async fn add_member(
    state: &AppState,
    tenant_id: TenantId,
    project_id: ProjectId,
    subject: &GrantSubject,
    role: RoleKey,
    claim: &Claim,
) -> Result<ScopeGrant> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let (project, scope) = project_and_scope(&mut tx, tenant_id, project_id).await?;
    let scope_id = scope.id;
    let (authorized, _) = require(
        state,
        &mut tx,
        Action::MembershipGrant,
        tenant_id,
        project_subject(&project, &scope),
    )
    .await?;
    let grant = access::create_grant(
        &mut *tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id,
            scope_id,
            subject: subject.clone(),
            role_key: role,
            source: GrantSource::Direct,
            invite_id: None,
            granted_by: Some(claim.subject.clone()),
        },
    )
    .await?;
    claim
        .remember(&mut tx, tenant_id, grant.id.as_uuid())
        .await?;
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::AccessGranted,
        Resource::Scope(scope_id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::MembershipGrant, &authorized),
            "grant": grant_image(&grant),
            "project_id": project_id,
            "idempotency_key": claim.key,
        }),
    )
    .await?;
    commit(tx).await?;
    Ok(grant)
}

/// The replay path: the same decision, then the grant the key produced.
///
/// The decision is taken again on purpose, for CPR-4's reason: a replay is
/// still a request to grant access, and a caller whose authority was revoked
/// between the attempt and the retry must be refused. A replay that skipped
/// the PDP would be a cached authorisation.
async fn replay_grant(
    state: &AppState,
    tenant_id: TenantId,
    id: GrantId,
    claim: &Claim,
) -> Result<ScopeGrant> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    // A grant is the one thing on this plane that is revoked by deleting the
    // row, so a replay can genuinely find nothing. That is a 404 naming the
    // situation rather than a silent re-grant.
    let grant = access::get_grant(&mut *tx, tenant_id, id)
        .await?
        .ok_or_else(|| crate::idempotency::vanished(claim, id.as_uuid()))?;
    let scope = scopes::get(&mut *tx, tenant_id, grant.scope_id)
        .await?
        .ok_or_else(|| crate::idempotency::vanished(claim, id.as_uuid()))?;
    let (authorized, resource) = require(
        state,
        &mut tx,
        Action::MembershipGrant,
        tenant_id,
        Subject::Grant(&grant, &scope),
    )
    .await?;
    read_event(
        &mut tx,
        tenant_id,
        "grant.replay",
        Action::MembershipGrant,
        &authorized,
        resource,
        json!({"grant_id": id, "idempotency_key": claim.key}),
    )
    .await?;
    commit(tx).await?;
    Ok(grant)
}

/// `DELETE /v1/projects/{project_id}/members/{principal_id}`.
#[utoipa::path(
    delete,
    path = "/v1/projects/{project_id}/members/{principal_id}",
    operation_id = "remove_project_member",
    tag = "access",
    params(
        ("project_id" = String, Path, description = "The project's id"),
        ("principal_id" = String, Path,
         description = "The principal's id — its verified token subject"),
    ),
    responses(
        (status = 204, description = "Every grant written at this project for this principal is revoked"),
        (status = 403, description = "The PDP denied `membership.grant`", body = ApiErrorBody),
        (status = 404, description = "No such project, or they hold nothing here", body = ApiErrorBody),
        (status = 409, description = "Their access is inherited, group-derived, or a directory's", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn remove_project_member(
    State(state): State<AppState>,
    Path((project_id, principal_id)): Path<(ProjectId, String)>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let (project, scope) = project_and_scope(&mut tx, tenant_id, project_id).await?;
        let scope_id = scope.id;
        let (authorized, _) = require(
            &state,
            &mut tx,
            Action::MembershipGrant,
            tenant_id,
            project_subject(&project, &scope),
        )
        .await?;
        let revoked = access::remove_member(&mut tx, tenant_id, scope_id, &principal_id).await?;
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::AccessRevoked,
            Resource::Scope(scope_id).to_string(),
            Outcome::Success,
            json!({
                "authz": audit::decision_context(Action::MembershipGrant, &authorized),
                "project_id": project_id,
                "principal_id": principal_id,
                "revoked": revoked.iter().map(grant_image).collect::<Vec<_>>(),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(StatusCode::NO_CONTENT)
    }
    .await;
    respond(&state, "member.remove", result).await
}

// ── Invitations ──────────────────────────────────────────────────────────────

/// `GET /v1/workspaces/{workspace_id}/invites`.
#[utoipa::path(
    get,
    path = "/v1/workspaces/{workspace_id}/invites",
    operation_id = "list_workspace_invites",
    tag = "access",
    params(("workspace_id" = String, Path, description = "The workspace's id")),
    responses(
        (status = 200, description = "Every invitation issued here", body = InviteList),
        (status = 403, description = "The PDP denied `membership.read`", body = ApiErrorBody),
        (status = 404, description = "No such workspace in this tenant", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn list_invites(
    State(state): State<AppState>,
    Path(workspace_id): Path<WorkspaceId>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let (workspace, scope) = workspace_and_scope(&mut tx, tenant_id, workspace_id).await?;
        let scope_id = scope.id;
        let (authorized, resource) = require(
            &state,
            &mut tx,
            Action::MembershipRead,
            tenant_id,
            workspace_subject(&workspace, &scope),
        )
        .await?;
        let invites = access::list_invites(&mut *tx, tenant_id, scope_id).await?;
        read_event(
            &mut tx,
            tenant_id,
            "invite.list",
            Action::MembershipRead,
            &authorized,
            resource,
            json!({"workspace_id": workspace_id, "count": invites.len()}),
        )
        .await?;
        commit(tx).await?;
        let now = Utc::now();
        Ok(Json(InviteList {
            invites: invites
                .into_iter()
                .map(|invite| InviteView::at(invite, now))
                .collect(),
        }))
    }
    .await;
    respond(&state, "invite.list", result).await
}

/// `POST /v1/workspaces/{workspace_id}/invites` — issue one.
#[utoipa::path(
    post,
    path = "/v1/workspaces/{workspace_id}/invites",
    operation_id = "create_workspace_invite",
    tag = "access",
    params(
        ("workspace_id" = String, Path, description = "The workspace's id"),
        ("Idempotency-Key" = String, Header,
         description = "Required. A unique value per request, reused verbatim on retry."),
    ),
    request_body = CreateInviteBody,
    responses(
        (status = 201, description = "Issued. **The token is in this response and nowhere else.**", body = CreatedInviteView),
        (status = 400, description = "A malformed email, a lifetime over the cap, or no `Idempotency-Key`", body = ApiErrorBody),
        (status = 403, description = "The PDP denied `membership.grant`", body = ApiErrorBody),
        (status = 404, description = "No such workspace in this tenant", body = ApiErrorBody),
        (status = 409, description = "The key was reused for a different request", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn create_invite(
    State(state): State<AppState>,
    Path(workspace_id): Path<WorkspaceId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<CreateInviteBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let ttl = body.expires_in_secs.unwrap_or(DEFAULT_INVITE_TTL_SECS);
        validate_invite_ttl(ttl)?;
        let tenant_id = tenant_id()?;
        let actor = subject()?;
        let claim = Claim::from_headers(
            &headers,
            "invite.create",
            &actor,
            &json!({
                "route": "POST /v1/workspaces/{workspace_id}/invites",
                "workspace_id": workspace_id,
                "role": body.role.as_str(),
                "email": body.email,
                "expires_in_secs": ttl,
            }),
        )?;

        // A replay cannot re-show the token — it exists once, in the response
        // to the request that minted it, and this plane's whole premise is that
        // it is never stored. So a replayed invitation creation is a **409
        // naming the situation** rather than a 200 with a missing field or a
        // second token for one invitation. The refusal tells the caller to look
        // at the listing, where the invitation they already made is.
        if let Dispatch::Replay(id) =
            crate::idempotency::dispatch(&state.pool, tenant_id, &claim).await?
        {
            return Err(already_issued(&claim, id, workspace_id));
        }
        match mint_invite(&state, tenant_id, workspace_id, &body, ttl, &claim).await {
            Ok(created) => Ok((StatusCode::CREATED, Json(created))),
            // The race the rest of this plane replays through: two concurrent
            // requests carrying one key both miss the lookup, and the second
            // insert loses on the primary key. Everywhere else that caller is
            // handed the original resource; here it cannot be, because the
            // original response carried a token. So it gets the same refusal a
            // sequential replay gets, rather than a raw storage conflict.
            Err(conflict @ Error::Conflict { .. }) => {
                match crate::idempotency::dispatch(&state.pool, tenant_id, &claim).await? {
                    Dispatch::Replay(id) => Err(already_issued(&claim, id, workspace_id)),
                    Dispatch::Create => Err(conflict),
                }
            }
            Err(other) => Err(other),
        }
    }
    .await;
    respond(&state, "invite.create", result).await
}

/// The refusal for a key that already issued an invitation.
///
/// An invitation token is shown once and is not stored, so a replay cannot
/// serve the original resource the way every other creation on this plane does.
/// A 200 with the token field missing would look successful and be unusable, so
/// this names the invitation and points at the listing instead.
fn already_issued(claim: &Claim, id: uuid::Uuid, workspace_id: WorkspaceId) -> Error {
    Error::Conflict {
        message: format!(
            "idempotency key {:?} already issued invitation {}; an invitation token is \
             shown once and cannot be re-served. Withdraw it and issue another, or find \
             it in GET /v1/workspaces/{workspace_id}/invites",
            claim.key,
            InviteId::from_uuid(id)
        ),
    }
}

async fn mint_invite(
    state: &AppState,
    tenant_id: TenantId,
    workspace_id: WorkspaceId,
    body: &CreateInviteBody,
    ttl: i64,
    claim: &Claim,
) -> Result<CreatedInviteView> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let (workspace, scope) = workspace_and_scope(&mut tx, tenant_id, workspace_id).await?;
    let scope_id = scope.id;
    let (authorized, _) = require(
        state,
        &mut tx,
        Action::MembershipGrant,
        tenant_id,
        workspace_subject(&workspace, &scope),
    )
    .await?;
    let minted = synveda_identity::invite::mint(tenant_id)?;
    let invite = access::create_invite(
        &mut *tx,
        &access::NewInvite {
            id: InviteId::new(),
            tenant_id,
            scope_id,
            role_key: body.role,
            email: body.email.clone(),
            token_hash: minted.hash,
            expires_at: Utc::now() + Duration::seconds(ttl),
            created_by: Some(claim.subject.clone()),
        },
    )
    .await?;
    claim
        .remember(&mut tx, tenant_id, invite.id.as_uuid())
        .await?;
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::InviteCreated,
        Resource::Scope(scope_id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::MembershipGrant, &authorized),
            "invite": invite_image(&invite),
            "workspace_id": workspace_id,
            "idempotency_key": claim.key,
        }),
    )
    .await?;
    commit(tx).await?;

    let accept_url = format!(
        "{}/v1/invites/{}/accept",
        state.public_origin.trim_end_matches('/'),
        minted.token
    );
    Ok(CreatedInviteView {
        invite: InviteView::at(invite, Utc::now()),
        token: minted.token,
        accept_url,
    })
}

/// `DELETE /v1/workspaces/{workspace_id}/invites/{invite_id}` — withdraw one.
#[utoipa::path(
    delete,
    path = "/v1/workspaces/{workspace_id}/invites/{invite_id}",
    operation_id = "revoke_workspace_invite",
    tag = "access",
    params(
        ("workspace_id" = String, Path, description = "The workspace's id"),
        ("invite_id" = String, Path, description = "The invitation's id"),
    ),
    responses(
        (status = 204, description = "Withdrawn"),
        (status = 403, description = "The PDP denied `membership.grant`", body = ApiErrorBody),
        (status = 404, description = "No such invitation on this workspace", body = ApiErrorBody),
        (status = 409, description = "It was already redeemed or withdrawn", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn revoke_invite(
    State(state): State<AppState>,
    Path((workspace_id, invite_id)): Path<(WorkspaceId, InviteId)>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let (workspace, scope) = workspace_and_scope(&mut tx, tenant_id, workspace_id).await?;
        let scope_id = scope.id;
        let (authorized, _) = require(
            &state,
            &mut tx,
            Action::MembershipGrant,
            tenant_id,
            workspace_subject(&workspace, &scope),
        )
        .await?;
        // An invitation addressed through the wrong workspace is a 404, not
        // somebody else's invitation withdrawn: the handle is scoped to the
        // path that names it.
        let found = access::get_invite(&mut *tx, tenant_id, invite_id).await?;
        if found.is_none_or(|invite| invite.scope_id != scope_id) {
            return Err(Error::NotFound {
                entity: format!("invitation {invite_id} on workspace {workspace_id}"),
            });
        }
        let invite =
            access::revoke_invite(&mut tx, tenant_id, invite_id, Some(&subject()?)).await?;
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::InviteRevoked,
            Resource::Scope(scope_id).to_string(),
            Outcome::Success,
            json!({
                "authz": audit::decision_context(Action::MembershipGrant, &authorized),
                "invite": invite_image(&invite),
                "workspace_id": workspace_id,
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(StatusCode::NO_CONTENT)
    }
    .await;
    respond(&state, "invite.revoke", result).await
}

/// `POST /v1/invites/{invite_token}/accept` — redeem one.
///
/// # It takes no `Idempotency-Key`, and that is not an exception
///
/// The token *is* the key: it is one-time by construction, so a retry finds the
/// invitation already accepted. When the retrying caller is the principal who
/// accepted it, the store replays — same grant, `200` — because that caller is
/// the timeout retry the whole mechanism exists for, arriving late. When it is
/// somebody else, it is a `409`, because that is what one-time means.
///
/// # The token is in the path, so the path is not recorded
///
/// `crate::app::make_request_span` records the matched route rather than the
/// raw URI for this route (and only this route), because a trace is an ordinary
/// log and this URI carries a live credential.
#[utoipa::path(
    post,
    path = "/v1/invites/{invite_token}/accept",
    operation_id = "accept_invite",
    tag = "access",
    params(("invite_token" = String, Path,
            description = "The invitation token. Presented with the recipient's own credential: \
                           the token says which access to add, never who is asking.")),
    responses(
        (status = 201, description = "Redeemed; the grant is yours", body = AcceptedInviteView),
        (status = 200, description = "You had already redeemed this one", body = AcceptedInviteView),
        (status = 400, description = "That is not an invitation token", body = ApiErrorBody),
        (status = 403, description = "The PDP denied `invite.accept`", body = ApiErrorBody),
        (status = 404, description = "No invitation in this tenant matches", body = ApiErrorBody),
        (status = 409, description = "Expired, withdrawn, or already somebody else's", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn accept_invite(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let actor = subject()?;
        // Refuse a string that is not one of ours before any lookup, so a
        // pasted URL fragment gets a sentence rather than a bare 404. The
        // refusal never echoes what was presented — it may well be a live
        // token from somewhere else.
        let named_tenant = synveda_identity::invite::parse(&token)?;
        let token_hash = synveda_identity::invite::hash(&token);

        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        // Redeeming is decided on the **tenant plane** and stays there: an
        // invitation's whole point is that its holder may hold nothing
        // anywhere yet, so a decision anchored at the scope being granted
        // would refuse exactly the people this route exists for (ADR-0072
        // decision 8).
        let (authorized, _) = require(
            &state,
            &mut tx,
            Action::InviteAccept,
            tenant_id,
            Subject::TenantOnly,
        )
        .await?;
        if named_tenant != tenant_id {
            // The hash would miss anyway — it covers the tenant — but saying
            // so plainly beats a 404 that reads as "your invitation is gone".
            return Err(Error::NotFound {
                entity: "invitation (it was issued by a different deployment or tenant)".to_owned(),
            });
        }
        let accepted =
            access::accept_invite(&mut tx, tenant_id, &token_hash, &actor, Utc::now()).await?;
        if !accepted.replayed {
            audit::record(
                &mut tx,
                tenant_id,
                AuditAction::InviteAccepted,
                Resource::Scope(accepted.grant.scope_id).to_string(),
                Outcome::Success,
                json!({
                    "authz": audit::decision_context(Action::InviteAccept, &authorized),
                    "invite": invite_image(&accepted.invite),
                    "grant": grant_image(&accepted.grant),
                }),
            )
            .await?;
        } else {
            read_event(
                &mut tx,
                tenant_id,
                "invite.accept.replay",
                Action::InviteAccept,
                &authorized,
                Resource::Scope(accepted.grant.scope_id),
                json!({"invite_id": accepted.invite.id}),
            )
            .await?;
        }
        commit(tx).await?;
        let status = if accepted.replayed {
            StatusCode::OK
        } else {
            StatusCode::CREATED
        };
        Ok((
            status,
            Json(AcceptedInviteView {
                scope_id: accepted.grant.scope_id,
                grant: GrantView::from(accepted.grant),
            }),
        ))
    }
    .await;
    respond(&state, "invite.accept", result).await
}

// ── Groups ───────────────────────────────────────────────────────────────────

/// `GET /v1/admin/groups`.
#[utoipa::path(
    get,
    path = "/v1/admin/groups",
    operation_id = "list_groups",
    tag = "access",
    responses(
        (status = 200, description = "The tenant's groups", body = GroupList),
        (status = 403, description = "The PDP denied `membership.read`", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn list_groups(State(state): State<AppState>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let (authorized, resource) = require(
            &state,
            &mut tx,
            Action::MembershipRead,
            tenant_id,
            Subject::Tenant,
        )
        .await?;
        let groups = access::list_groups(&mut *tx, tenant_id).await?;
        // One query for every group's membership rather than one per group:
        // an admin screen listing forty groups must not be forty round trips.
        let memberships = access::all_group_members(&mut *tx, tenant_id).await?;
        read_event(
            &mut tx,
            tenant_id,
            "group.list",
            Action::MembershipRead,
            &authorized,
            resource,
            json!({"count": groups.len()}),
        )
        .await?;
        commit(tx).await?;
        let mut by_group: std::collections::HashMap<GroupId, Vec<String>> =
            std::collections::HashMap::new();
        for member in memberships {
            by_group
                .entry(member.group_id)
                .or_default()
                .push(member.principal_id);
        }
        Ok(Json(GroupList {
            groups: groups
                .into_iter()
                .map(|group| {
                    let members = by_group.remove(&group.id).unwrap_or_default();
                    GroupView::build(group, members)
                })
                .collect(),
        }))
    }
    .await;
    respond(&state, "group.list", result).await
}

/// `POST /v1/admin/groups`.
#[utoipa::path(
    post,
    path = "/v1/admin/groups",
    operation_id = "create_group",
    tag = "access",
    params(
        ("Idempotency-Key" = String, Header,
         description = "Required. A unique value per request, reused verbatim on retry."),
    ),
    request_body = CreateGroupBody,
    responses(
        (status = 201, description = "Created", body = GroupView),
        (status = 200, description = "This key already created this group", body = GroupView),
        (status = 400, description = "A malformed slug or name, or no `Idempotency-Key`", body = ApiErrorBody),
        (status = 403, description = "The PDP denied `group.manage`", body = ApiErrorBody),
        (status = 409, description = "The slug is taken, or the key was reused", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn create_group(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: std::result::Result<Json<CreateGroupBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant_id = tenant_id()?;
        let actor = subject()?;
        let claim = Claim::from_headers(
            &headers,
            "group.create",
            &actor,
            &json!({
                "route": "POST /v1/admin/groups",
                "slug": body.slug,
                "display_name": body.display_name,
                "description": body.description,
                "members": body.members,
            }),
        )?;

        let replayed = match crate::idempotency::dispatch(&state.pool, tenant_id, &claim).await? {
            Dispatch::Replay(id) => Some(id),
            Dispatch::Create => match make_group(&state, tenant_id, &body, &claim).await {
                Ok(view) => return Ok((StatusCode::CREATED, Json(view))),
                Err(conflict @ Error::Conflict { .. }) => Some(
                    crate::idempotency::resolve_conflict(&state.pool, tenant_id, &claim, conflict)
                        .await?,
                ),
                Err(other) => return Err(other),
            },
        };
        let id = GroupId::from_uuid(replayed.expect("replay id"));
        let view = replay_group(&state, tenant_id, id, &claim).await?;
        Ok((StatusCode::OK, Json(view)))
    }
    .await;
    respond(&state, "group.create", result).await
}

async fn make_group(
    state: &AppState,
    tenant_id: TenantId,
    body: &CreateGroupBody,
    claim: &Claim,
) -> Result<GroupView> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    // Creating one is the tenant plane's: there is no group to name yet, and
    // `GroupManage` has no scope resource in the schema.
    let (authorized, _) = require(
        state,
        &mut tx,
        Action::GroupManage,
        tenant_id,
        Subject::TenantOnly,
    )
    .await?;
    let group = access::create_group(
        &mut *tx,
        &access::NewGroup {
            id: GroupId::new(),
            tenant_id,
            slug: body.slug.clone(),
            display_name: body.display_name.clone(),
            description: body.description.clone(),
            // Nothing on this plane creates a directory group: a directory
            // group is created by a directory, and the adapter that does it is
            // a later prompt. The column exists so that when it lands, a
            // person's group and a directory's are the same row shape.
            source: GroupSource::Direct,
            directory_ref: None,
            created_by: Some(claim.subject.clone()),
        },
    )
    .await?;
    let members = if body.members.is_empty() {
        Vec::new()
    } else {
        access::set_group_members(
            &mut tx,
            tenant_id,
            group.id,
            &body.members,
            Some(&claim.subject),
        )
        .await?
    };
    claim
        .remember(&mut tx, tenant_id, group.id.as_uuid())
        .await?;
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::GroupCreated,
        Resource::Tenant(tenant_id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::GroupManage, &authorized),
            "group": group_image(&group),
            "member_count": members.len(),
            "idempotency_key": claim.key,
        }),
    )
    .await?;
    commit(tx).await?;
    Ok(GroupView::build(group, members))
}

async fn replay_group(
    state: &AppState,
    tenant_id: TenantId,
    id: GroupId,
    claim: &Claim,
) -> Result<GroupView> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let group = access::get_group(&mut *tx, tenant_id, id)
        .await?
        .ok_or_else(|| crate::idempotency::vanished(claim, id.as_uuid()))?;
    let (authorized, resource) = require(
        state,
        &mut tx,
        Action::GroupManage,
        tenant_id,
        Subject::Group(group.id),
    )
    .await?;
    let members = access::group_members(&mut *tx, tenant_id, id).await?;
    read_event(
        &mut tx,
        tenant_id,
        "group.create.replay",
        Action::GroupManage,
        &authorized,
        resource,
        json!({"group_id": id, "idempotency_key": claim.key}),
    )
    .await?;
    commit(tx).await?;
    Ok(GroupView::build(
        group,
        members
            .into_iter()
            .map(|member| member.principal_id)
            .collect(),
    ))
}

/// `PATCH /v1/admin/groups/{group_id}`.
#[utoipa::path(
    patch,
    path = "/v1/admin/groups/{group_id}",
    operation_id = "update_group",
    tag = "access",
    params(("group_id" = String, Path, description = "The group's id")),
    request_body = UpdateGroupBody,
    responses(
        (status = 200, description = "The updated group", body = GroupView),
        (status = 400, description = "Malformed body, or nothing to update", body = ApiErrorBody),
        (status = 403, description = "The PDP denied `group.manage`", body = ApiErrorBody),
        (status = 404, description = "No such group in this tenant", body = ApiErrorBody),
        (status = 409, description = "Stale `expected_revision`, or the group is a directory's", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn update_group(
    State(state): State<AppState>,
    Path(group_id): Path<GroupId>,
    payload: std::result::Result<Json<UpdateGroupBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant_id = tenant_id()?;
        let actor = subject()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let before = access::get_group(&mut *tx, tenant_id, group_id)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity: format!("group {group_id}"),
            })?;
        let (authorized, _) = require(
            &state,
            &mut tx,
            Action::GroupManage,
            tenant_id,
            Subject::Group(group_id),
        )
        .await?;
        let before_members = access::group_members(&mut *tx, tenant_id, group_id).await?;
        let after = access::update_group(
            &mut tx,
            tenant_id,
            group_id,
            body.expected_revision,
            &access::GroupUpdate {
                display_name: body.display_name.clone(),
                description: body.description.clone(),
                status: body.status,
                members: body.members.clone(),
            },
            Some(&actor),
        )
        .await?;
        let after_members = access::group_members(&mut *tx, tenant_id, group_id).await?;
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::GroupUpdated,
            Resource::Tenant(tenant_id).to_string(),
            Outcome::Success,
            json!({
                "authz": audit::decision_context(Action::GroupManage, &authorized),
                "expected_revision": body.expected_revision,
                "before": group_image(&before),
                "after": group_image(&after),
                // Counts and the difference, never the whole list: a
                // hundred-person group would otherwise put a hundred names in
                // the chain on every edit, and what an auditor needs is who
                // moved.
                "membership": membership_delta(&before_members, &after_members),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(GroupView::build(
            after,
            after_members
                .into_iter()
                .map(|member| member.principal_id)
                .collect(),
        )))
    }
    .await;
    respond(&state, "group.update", result).await
}

/// What changed about a group's membership, for the audit payload.
fn membership_delta(
    before: &[synveda_types::access::GroupMember],
    after: &[synveda_types::access::GroupMember],
) -> serde_json::Value {
    let before_set: std::collections::BTreeSet<&str> = before
        .iter()
        .map(|member| member.principal_id.as_str())
        .collect();
    let after_set: std::collections::BTreeSet<&str> = after
        .iter()
        .map(|member| member.principal_id.as_str())
        .collect();
    json!({
        "before_count": before_set.len(),
        "after_count": after_set.len(),
        "added": after_set.difference(&before_set).collect::<Vec<_>>(),
        "removed": before_set.difference(&after_set).collect::<Vec<_>>(),
    })
}

// ── Grants ───────────────────────────────────────────────────────────────────

/// `GET /v1/admin/grants`.
#[utoipa::path(
    get,
    path = "/v1/admin/grants",
    operation_id = "list_grants",
    tag = "access",
    params(GrantQuery),
    responses(
        (status = 200, description = "The grants this filter selected", body = GrantList),
        (status = 400, description = "An unknown query parameter", body = ApiErrorBody),
        (status = 403, description = "The PDP denied `membership.read`", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn list_grants(
    State(state): State<AppState>,
    query: std::result::Result<Query<GrantQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let result = async {
        let Query(query) = query.map_err(|rejection| Error::Invalid {
            message: rejection.body_text(),
        })?;
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let (authorized, resource) = require(
            &state,
            &mut tx,
            Action::MembershipRead,
            tenant_id,
            Subject::Tenant,
        )
        .await?;
        let grants = access::list_grants(
            &mut *tx,
            tenant_id,
            &access::GrantFilter {
                scope_id: query.scope_id,
                principal_id: query.principal_id.clone(),
            },
        )
        .await?;
        read_event(
            &mut tx,
            tenant_id,
            "grant.list",
            Action::MembershipRead,
            &authorized,
            resource,
            json!({
                "scope_id": query.scope_id,
                "principal_id": query.principal_id,
                "count": grants.len(),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(GrantList {
            grants: grants.into_iter().map(Into::into).collect(),
        }))
    }
    .await;
    respond(&state, "grant.list", result).await
}

/// `POST /v1/admin/grants` — grant at any scope the caller names.
#[utoipa::path(
    post,
    path = "/v1/admin/grants",
    operation_id = "create_grant",
    tag = "access",
    params(
        ("Idempotency-Key" = String, Header,
         description = "Required. A unique value per request, reused verbatim on retry."),
    ),
    request_body = GrantSubjectBody,
    responses(
        (status = 201, description = "Granted", body = GrantView),
        (status = 200, description = "This key already made this grant", body = GrantView),
        (status = 400, description = "No `scope_id`, no subject, both subjects, or no `Idempotency-Key`", body = ApiErrorBody),
        (status = 403, description = "The PDP denied `membership.grant`", body = ApiErrorBody),
        (status = 404, description = "No such scope or group in this tenant", body = ApiErrorBody),
        (status = 409, description = "They already hold this role there, or the key was reused", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn create_grant(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: std::result::Result<Json<GrantSubjectBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let scope_id = body.scope_id.ok_or_else(|| Error::Invalid {
            message: "a grant names the scope it is at: send scope_id".to_owned(),
        })?;
        let subject_claim = body.subject()?;
        let tenant_id = tenant_id()?;
        let actor = subject()?;
        let claim = Claim::from_headers(
            &headers,
            "grant.create",
            &actor,
            &body.canonical("POST /v1/admin/grants"),
        )?;

        let replayed = match crate::idempotency::dispatch(&state.pool, tenant_id, &claim).await? {
            Dispatch::Replay(id) => Some(id),
            Dispatch::Create => {
                match grant_at(
                    &state,
                    tenant_id,
                    scope_id,
                    &subject_claim,
                    body.role,
                    &claim,
                )
                .await
                {
                    Ok(grant) => return Ok((StatusCode::CREATED, Json(GrantView::from(grant)))),
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
        let id = GrantId::from_uuid(replayed.expect("replay id"));
        let grant = replay_grant(&state, tenant_id, id, &claim).await?;
        Ok((StatusCode::OK, Json(GrantView::from(grant))))
    }
    .await;
    respond(&state, "grant.create", result).await
}

async fn grant_at(
    state: &AppState,
    tenant_id: TenantId,
    scope_id: ScopeId,
    subject: &GrantSubject,
    role: RoleKey,
    claim: &Claim,
) -> Result<ScopeGrant> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    // The named scope is the resource: granting at a project is an authority
    // over that project, and a caller who holds the workspace above it has it
    // by inheritance rather than by a tenant-wide role (ADR-0073 decision 3).
    let scope = scopes::get(&mut *tx, tenant_id, scope_id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("scope {scope_id}"),
        })?;
    let (authorized, _) = require(
        state,
        &mut tx,
        Action::MembershipGrant,
        tenant_id,
        Subject::Scope(&scope, None, AnchorSelection::none()),
    )
    .await?;
    // Ownership before the mutation: another tenant's scope is a 404 and never
    // a foreign-key conflict that half-describes what is there.
    if scopes::get(&mut *tx, tenant_id, scope_id).await?.is_none() {
        return Err(Error::NotFound {
            entity: format!("scope {scope_id}"),
        });
    }
    if let GrantSubject::Group { group_id } = subject
        && access::get_group(&mut *tx, tenant_id, *group_id)
            .await?
            .is_none()
    {
        return Err(Error::NotFound {
            entity: format!("group {group_id}"),
        });
    }
    let grant = access::create_grant(
        &mut *tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id,
            scope_id,
            subject: subject.clone(),
            role_key: role,
            source: GrantSource::Direct,
            invite_id: None,
            granted_by: Some(claim.subject.clone()),
        },
    )
    .await?;
    claim
        .remember(&mut tx, tenant_id, grant.id.as_uuid())
        .await?;
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::AccessGranted,
        Resource::Scope(scope_id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::MembershipGrant, &authorized),
            "grant": grant_image(&grant),
            "idempotency_key": claim.key,
        }),
    )
    .await?;
    commit(tx).await?;
    Ok(grant)
}

/// `DELETE /v1/admin/grants/{grant_id}` — revoke one.
#[utoipa::path(
    delete,
    path = "/v1/admin/grants/{grant_id}",
    operation_id = "revoke_grant",
    tag = "access",
    params(("grant_id" = String, Path, description = "The grant's id")),
    responses(
        (status = 204, description = "Revoked"),
        (status = 403, description = "The PDP denied `membership.grant`", body = ApiErrorBody),
        (status = 404, description = "No such grant in this tenant", body = ApiErrorBody),
        (status = 409, description = "A directory manages this grant", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn revoke_grant(
    State(state): State<AppState>,
    Path(grant_id): Path<GrantId>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        // The **grant itself** is the resource, which is what lets a pack say
        // something about revoking a directory-managed one or an owner's
        // (ADR-0073 decision 3). Read before the decision so the entity the PDP
        // evaluates is the row that is about to go.
        let existing = access::get_grant(&mut *tx, tenant_id, grant_id)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity: format!("grant {grant_id}"),
            })?;
        let scope = scopes::get(&mut *tx, tenant_id, existing.scope_id)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity: format!("grant {grant_id}"),
            })?;
        let (authorized, _) = require(
            &state,
            &mut tx,
            Action::MembershipGrant,
            tenant_id,
            Subject::Grant(&existing, &scope),
        )
        .await?;
        let grant = access::revoke_grant(&mut tx, tenant_id, grant_id).await?;
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::AccessRevoked,
            Resource::Scope(grant.scope_id).to_string(),
            Outcome::Success,
            json!({
                "authz": audit::decision_context(Action::MembershipGrant, &authorized),
                "revoked": [grant_image(&grant)],
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(StatusCode::NO_CONTENT)
    }
    .await;
    respond(&state, "grant.revoke", result).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grant_body_names_exactly_one_subject() {
        let principal: GrantSubjectBody =
            serde_json::from_str(r#"{"principal_id": "sam", "role": "member"}"#).unwrap();
        assert_eq!(
            principal.subject().unwrap(),
            GrantSubject::Principal {
                principal_id: "sam".to_owned()
            }
        );

        let neither: GrantSubjectBody = serde_json::from_str(r#"{"role": "member"}"#).unwrap();
        let error = neither.subject().expect_err("refused");
        assert!(error.to_string().contains("principal_id"), "{error}");

        let both: GrantSubjectBody = serde_json::from_str(&format!(
            r#"{{"principal_id": "sam", "group_id": "{}", "role": "member"}}"#,
            GroupId::new()
        ))
        .unwrap();
        assert!(both.subject().is_err(), "both subjects is not a grant");
    }

    #[test]
    fn an_update_without_a_precondition_is_refused_by_the_wire() {
        assert!(
            serde_json::from_str::<UpdateGroupBody>(r#"{"display_name": "Engineering"}"#).is_err(),
            "expected_revision is required, not defaulted"
        );
        let with: UpdateGroupBody =
            serde_json::from_str(r#"{"expected_revision": 2, "members": []}"#).unwrap();
        assert_eq!(
            with.members,
            Some(Vec::new()),
            "emptying a group is a change"
        );
    }

    #[test]
    fn unknown_fields_are_refused_rather_than_ignored() {
        assert!(
            serde_json::from_str::<CreateInviteBody>(r#"{"role": "member", "token": "hunter2"}"#)
                .is_err(),
            "a caller must not be able to propose the token"
        );
        assert!(
            serde_json::from_str::<CreateGroupBody>(
                r#"{"slug": "eng", "display_name": "Eng", "source": "directory"}"#
            )
            .is_err(),
            "a caller must not be able to claim a directory owns their group"
        );
    }

    /// The wire vocabularies are built from the domain enums, so this asserts
    /// the mechanism rather than a transcription.
    #[test]
    fn the_wire_vocabularies_come_from_the_domain_enums() {
        for (schema, expected) in [
            (
                role_key_schema(),
                json!([
                    "owner",
                    "member",
                    "viewer",
                    "reviewer",
                    "curator",
                    "administrator"
                ]),
            ),
            (
                grant_source_schema(),
                json!(["owner", "direct", "invite", "directory", "automation"]),
            ),
            (group_source_schema(), json!(["direct", "directory"])),
            (subject_kind_schema(), json!(["principal", "group"])),
            (
                invite_status_schema(),
                json!(["pending", "accepted", "revoked", "expired"]),
            ),
        ] {
            let rendered = serde_json::to_value(&schema).expect("schema serialises");
            assert_eq!(rendered["enum"], expected);
        }
    }

    #[test]
    fn a_membership_delta_reports_who_moved_and_not_the_whole_list() {
        let member = |id: &str| synveda_types::access::GroupMember {
            tenant_id: TenantId::new(),
            group_id: GroupId::new(),
            principal_id: id.to_owned(),
            source: GrantSource::Direct,
            added_by: None,
            created_at: Utc::now(),
        };
        let before = vec![member("sam"), member("robin")];
        let after = vec![member("robin"), member("kim")];
        let delta = membership_delta(&before, &after);
        assert_eq!(delta["before_count"], 2);
        assert_eq!(delta["after_count"], 2);
        assert_eq!(delta["added"], json!(["kim"]));
        assert_eq!(delta["removed"], json!(["sam"]));
    }
}
