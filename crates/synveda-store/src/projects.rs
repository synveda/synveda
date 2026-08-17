//! Projects (CPR-4, ADR-0071): the product-level subtype of a
//! `project`-shaped governed scope, sitting inside a workspace.
//!
//! [`crate::workspaces`]'s shape, one level down, and its module documentation
//! applies verbatim: the scope and the row are created in one transaction so
//! the outcomes are "both" and "neither"; governance attaches above this
//! module; every read is tenant-filtered in SQL as well as by RLS.
//!
//! The one rule that is this module's own: a project's scope is a **child of
//! its workspace's scope**, which migration 0041 makes a foreign key rather
//! than a convention — so a project's scope cannot later be moved out from
//! under the workspace whose policy governs it.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use synveda_types::scope::{ScopeKind, validate_display_name, validate_slug};
use synveda_types::workspace::{LifecycleStatus, Project, validate_description};
use synveda_types::{Error, IdentityId, ProjectId, Result, ScopeId, TenantId, WorkspaceId};
use uuid::Uuid;

use crate::scopes::{self, NewScope};
use crate::workspaces::{SUBTYPE_MUTATIONS_TOTAL, scope_status, storage_error};

/// What [`create`] needs.
#[derive(Debug, Clone)]
pub struct NewProject {
    /// The project's identity.
    pub id: ProjectId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// The workspace it belongs to. Must be active — see [`create`].
    pub workspace_id: WorkspaceId,
    /// Workspace-unique handle; becomes the owned scope's slug too.
    pub slug: String,
    /// Display name.
    pub display_name: String,
    /// Optional prose.
    pub description: Option<String>,
    /// The identity creating it, when one is.
    pub created_by: Option<IdentityId>,
}

/// A partial update, with the same three-case `description` as
/// [`crate::workspaces::WorkspaceUpdate`].
///
/// There is deliberately no `workspace_id`: moving a project between
/// workspaces would move its scope across a policy boundary, which is a create
/// and an archive rather than an update, and 0041's trigger refuses it for
/// every role.
#[derive(Debug, Clone, Default)]
pub struct ProjectUpdate {
    /// New display name, when renaming.
    pub display_name: Option<String>,
    /// New description.
    pub description: Option<Option<String>>,
    /// New lifecycle status, mirrored onto the owned scope.
    pub status: Option<LifecycleStatus>,
}

impl ProjectUpdate {
    /// Whether this update would change anything.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.display_name.is_none() && self.description.is_none() && self.status.is_none()
    }
}

struct ProjectRow {
    id: Uuid,
    tenant_id: Uuid,
    workspace_id: Uuid,
    scope_id: Uuid,
    slug: String,
    display_name: String,
    description: Option<String>,
    status: String,
    revision: i64,
    created_by: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<ProjectRow> for Project {
    type Error = Error;

    fn try_from(row: ProjectRow) -> Result<Self> {
        Ok(Project {
            id: ProjectId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            workspace_id: WorkspaceId::from_uuid(row.workspace_id),
            scope_id: ScopeId::from_uuid(row.scope_id),
            slug: row.slug,
            display_name: row.display_name,
            description: row.description,
            status: row.status.parse().map_err(|err| Error::Internal {
                message: format!("stored value outside vocabulary: {err}"),
            })?,
            revision: row.revision,
            created_by: row.created_by.map(IdentityId::from_uuid),
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

fn not_found(id: ProjectId) -> Error {
    Error::NotFound {
        entity: format!("project {id}"),
    }
}

/// Creates a project and the governed scope it owns, beneath its workspace's
/// scope, in the caller's transaction.
///
/// The workspace must exist in this tenant ([`Error::NotFound`] otherwise) and
/// must be **active**: an archived workspace is one somebody retired, and
/// accepting new work into it would make the retirement advisory.
///
/// Must run inside a transaction (see [`crate::workspaces`]'s module docs).
#[tracing::instrument(
    name = "store.projects.create",
    skip_all,
    fields(
        tenant.id = %new.tenant_id,
        project.id = %new.id,
        workspace.id = %new.workspace_id,
        scope.id = tracing::field::Empty,
    ),
    err(Display)
)]
pub async fn create(conn: &mut PgConnection, new: &NewProject) -> Result<Project> {
    validate_slug(&new.slug)?;
    validate_display_name(&new.display_name)?;
    validate_description(new.description.as_deref())?;

    let workspace = crate::workspaces::get(&mut *conn, new.tenant_id, new.workspace_id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("workspace {}", new.workspace_id),
        })?;
    if !workspace.status.is_active() {
        return Err(Error::Conflict {
            message: format!(
                "workspace {} is {}; a project cannot be created in it",
                workspace.slug, workspace.status
            ),
        });
    }

    let scope = scopes::create(
        &mut *conn,
        &NewScope {
            id: ScopeId::new(),
            tenant_id: new.tenant_id,
            kind: ScopeKind::Project,
            parent_scope_id: Some(workspace.scope_id),
            slug: new.slug.clone(),
            display_name: new.display_name.clone(),
            attributes: serde_json::json!({}),
            created_by: new.created_by,
        },
    )
    .await?;
    tracing::Span::current().record("scope.id", tracing::field::display(scope.id));

    let row = sqlx::query_as!(
        ProjectRow,
        r#"
        insert into projects
            (id, tenant_id, workspace_id, scope_id, scope_kind, workspace_scope_id,
             slug, display_name, description, status, created_by)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        returning id, tenant_id, workspace_id, scope_id, slug, display_name,
                  description, status, revision, created_by, created_at, updated_at
        "#,
        new.id.as_uuid(),
        new.tenant_id.as_uuid(),
        new.workspace_id.as_uuid(),
        scope.id.as_uuid(),
        ScopeKind::Project.as_str(),
        workspace.scope_id.as_uuid(),
        new.slug,
        new.display_name,
        new.description.as_deref() as Option<&str>,
        LifecycleStatus::Active.as_str(),
        new.created_by.map(|by| by.as_uuid()) as Option<Uuid>,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;

    metrics::counter!(
        SUBTYPE_MUTATIONS_TOTAL,
        "subtype" => "project",
        "operation" => "create",
    )
    .increment(1);
    row.try_into()
}

/// Fetches one project.
#[tracing::instrument(
    name = "store.projects.get",
    skip_all,
    fields(tenant.id = %tenant_id, project.id = %id),
    err(Display)
)]
pub async fn get(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: ProjectId,
) -> Result<Option<Project>> {
    let row = sqlx::query_as!(
        ProjectRow,
        r#"
        select id, tenant_id, workspace_id, scope_id, slug, display_name,
               description, status, revision, created_by, created_at, updated_at
        from projects
        where id = $1 and tenant_id = $2
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Lists a workspace's projects, ordered by slug. Archived projects are
/// included, for [`crate::workspaces::list`]'s reason.
#[tracing::instrument(
    name = "store.projects.in_workspace",
    skip_all,
    fields(tenant.id = %tenant_id, workspace.id = %workspace_id),
    err(Display)
)]
pub async fn in_workspace(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    workspace_id: WorkspaceId,
) -> Result<Vec<Project>> {
    let rows = sqlx::query_as!(
        ProjectRow,
        r#"
        select id, tenant_id, workspace_id, scope_id, slug, display_name,
               description, status, revision, created_by, created_at, updated_at
        from projects
        where tenant_id = $1 and workspace_id = $2
        order by slug
        "#,
        tenant_id.as_uuid(),
        workspace_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Lists every project in the tenant, ordered by workspace then slug. What
/// `GET /v1/me` renders, so that a client's first call answers "where does my
/// work live" without one request per workspace.
#[tracing::instrument(
    name = "store.projects.list",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn list(executor: impl PgExecutor<'_>, tenant_id: TenantId) -> Result<Vec<Project>> {
    let rows = sqlx::query_as!(
        ProjectRow,
        r#"
        select p.id, p.tenant_id, p.workspace_id, p.scope_id, p.slug, p.display_name,
               p.description, p.status, p.revision, p.created_by, p.created_at, p.updated_at
        from projects p
        join workspaces w on w.id = p.workspace_id and w.tenant_id = p.tenant_id
        where p.tenant_id = $1
        order by w.slug, p.slug
        "#,
        tenant_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Applies an update under a revision precondition — see
/// [`crate::workspaces::update`], whose contract this shares exactly.
///
/// Must run inside a transaction (the scope mirror is a second statement).
#[tracing::instrument(
    name = "store.projects.update",
    skip_all,
    fields(
        tenant.id = %tenant_id,
        project.id = %id,
        project.expected_revision = expected_revision,
    ),
    err(Display)
)]
pub async fn update(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    id: ProjectId,
    expected_revision: i64,
    update: &ProjectUpdate,
) -> Result<Project> {
    if update.is_empty() {
        return Err(Error::Invalid {
            message: "nothing to update: provide display_name, description or status".to_owned(),
        });
    }
    if let Some(display_name) = &update.display_name {
        validate_display_name(display_name)?;
    }
    if let Some(description) = &update.description {
        validate_description(description.as_deref())?;
    }

    let row = sqlx::query_as!(
        ProjectRow,
        r#"
        update projects
           set display_name = coalesce($4, display_name),
               description  = case when $5 then $6 else description end,
               status       = coalesce($7, status),
               revision     = revision + 1,
               updated_at   = now()
         where id = $1 and tenant_id = $2 and revision = $3
        returning id, tenant_id, workspace_id, scope_id, slug, display_name,
                  description, status, revision, created_by, created_at, updated_at
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
        expected_revision,
        update.display_name.as_deref() as Option<&str>,
        update.description.is_some(),
        update.description.as_ref().and_then(Option::as_deref) as Option<&str>,
        update.status.map(|status| status.as_str()) as Option<&str>,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;

    let Some(row) = row else {
        return Err(match get(&mut *conn, tenant_id, id).await? {
            Some(current) => Error::Conflict {
                message: format!(
                    "project {id} is at revision {}, not {expected_revision}",
                    current.revision
                ),
            },
            None => not_found(id),
        });
    };
    let project: Project = row.try_into()?;

    if let Some(status) = update.status {
        scopes::set_status(
            &mut *conn,
            tenant_id,
            project.scope_id,
            scope_status(status),
        )
        .await?;
    }

    metrics::counter!(
        SUBTYPE_MUTATIONS_TOTAL,
        "subtype" => "project",
        "operation" => "update",
    )
    .increment(1);
    Ok(project)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_update_changes_nothing_and_says_so() {
        assert!(ProjectUpdate::default().is_empty());
        assert!(
            !ProjectUpdate {
                status: Some(LifecycleStatus::Archived),
                ..Default::default()
            }
            .is_empty()
        );
    }
}
