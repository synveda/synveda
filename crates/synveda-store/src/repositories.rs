//! Project repositories (CPR-4, ADR-0071 decision 4): what a project is
//! *about*, addressed by canonical identity rather than by where somebody's
//! checkout happens to sit.
//!
//! The canonicalisation itself is [`synveda_types::repository::identify`] —
//! this module stores what it resolved. The division matters: two clients on
//! two machines must compute the same identity from different-looking inputs,
//! so the rule lives once, above storage, where a CLI, an adapter and a test
//! all reach the same function.
//!
//! Attachments are **immutable and detachable**, never editable. Re-pointing a
//! project at a different repository is a detach and an attach, so the audit
//! chain records two acts rather than one row quietly coming to mean something
//! else (0041's trigger enforces it for every role).

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use synveda_types::repository::{
    ProjectRepository, RepositoryIdentity, validate_branch, validate_metadata,
};
use synveda_types::{Error, IdentityId, ProjectId, RepositoryId, Result, TenantId};
use uuid::Uuid;

use crate::workspaces::storage_error;

/// Counter: repository attachments, labelled `operation` = `attach` |
/// `detach`. Emitted here, described by the gateway (ADR-0007).
pub const REPOSITORY_MUTATIONS_TOTAL: &str = "synveda_repository_mutations_total";

/// What [`attach`] needs: the resolved identity plus the facts the caller
/// supplied that identity does not decide.
#[derive(Debug, Clone)]
pub struct NewRepository {
    /// The attachment's identity.
    pub id: RepositoryId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// The project it belongs to.
    pub project_id: ProjectId,
    /// What [`synveda_types::repository::identify`] resolved. Taken whole
    /// rather than field by field, so a caller cannot supply a provider that
    /// disagrees with the URI it was derived from.
    pub identity: RepositoryIdentity,
    /// Advisory default branch.
    pub default_branch: Option<String>,
    /// Caller-supplied labelling bag.
    pub metadata: serde_json::Value,
    /// The identity attaching it, when one is.
    pub created_by: Option<IdentityId>,
}

struct RepositoryRow {
    id: Uuid,
    tenant_id: Uuid,
    project_id: Uuid,
    provider: String,
    canonical_uri: String,
    repository_owner: Option<String>,
    repository_name: String,
    default_branch: Option<String>,
    local_fingerprint: Option<String>,
    metadata: serde_json::Value,
    created_by: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<RepositoryRow> for ProjectRepository {
    type Error = Error;

    fn try_from(row: RepositoryRow) -> Result<Self> {
        Ok(ProjectRepository {
            id: RepositoryId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            project_id: ProjectId::from_uuid(row.project_id),
            provider: row.provider.parse().map_err(|err| Error::Internal {
                message: format!("stored value outside vocabulary: {err}"),
            })?,
            canonical_uri: row.canonical_uri,
            repository_owner: row.repository_owner,
            repository_name: row.repository_name,
            default_branch: row.default_branch,
            local_fingerprint: row.local_fingerprint,
            metadata: row.metadata,
            created_by: row.created_by.map(IdentityId::from_uuid),
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

/// Attaches a repository to a project.
///
/// Fails with [`Error::Invalid`] for a malformed branch or metadata bag,
/// [`Error::Conflict`] when this project already names the same repository
/// (case-insensitively — `Acme/payments` and `acme/payments` are one
/// repository), and [`Error::NotFound`] when the project is not this tenant's,
/// which the caller checks before calling so the message names the project
/// rather than a foreign key.
#[tracing::instrument(
    name = "store.repositories.attach",
    skip_all,
    fields(
        tenant.id = %new.tenant_id,
        project.id = %new.project_id,
        repository.id = %new.id,
        repository.provider = %new.identity.provider,
    ),
    err(Display)
)]
pub async fn attach(
    executor: impl PgExecutor<'_>,
    new: &NewRepository,
) -> Result<ProjectRepository> {
    validate_branch(new.default_branch.as_deref())?;
    validate_metadata(&new.metadata)?;

    let row = sqlx::query_as!(
        RepositoryRow,
        r#"
        insert into project_repositories
            (id, tenant_id, project_id, provider, canonical_uri, repository_owner,
             repository_name, default_branch, local_fingerprint, metadata, created_by)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        returning id, tenant_id, project_id, provider, canonical_uri, repository_owner,
                  repository_name, default_branch, local_fingerprint, metadata,
                  created_by, created_at, updated_at
        "#,
        new.id.as_uuid(),
        new.tenant_id.as_uuid(),
        new.project_id.as_uuid(),
        new.identity.provider.as_str(),
        new.identity.canonical_uri,
        new.identity.repository_owner.as_deref() as Option<&str>,
        new.identity.repository_name,
        new.default_branch.as_deref() as Option<&str>,
        new.identity.local_fingerprint.as_deref() as Option<&str>,
        new.metadata,
        new.created_by.map(|by| by.as_uuid()) as Option<Uuid>,
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;

    metrics::counter!(REPOSITORY_MUTATIONS_TOTAL, "operation" => "attach").increment(1);
    row.try_into()
}

/// Lists a project's repositories, oldest attachment first.
#[tracing::instrument(
    name = "store.repositories.for_project",
    skip_all,
    fields(tenant.id = %tenant_id, project.id = %project_id),
    err(Display)
)]
pub async fn for_project(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    project_id: ProjectId,
) -> Result<Vec<ProjectRepository>> {
    let rows = sqlx::query_as!(
        RepositoryRow,
        r#"
        select id, tenant_id, project_id, provider, canonical_uri, repository_owner,
               repository_name, default_branch, local_fingerprint, metadata,
               created_by, created_at, updated_at
        from project_repositories
        where tenant_id = $1 and project_id = $2
        order by created_at, id
        "#,
        tenant_id.as_uuid(),
        project_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Fetches one attachment, scoped to its project.
///
/// Takes the project id as well as the repository id, so a handle from one
/// project cannot address a row in another: the route's path says which
/// project, and a lookup that ignored it would make the path decorative.
#[tracing::instrument(
    name = "store.repositories.get",
    skip_all,
    fields(tenant.id = %tenant_id, project.id = %project_id, repository.id = %id),
    err(Display)
)]
pub async fn get(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    project_id: ProjectId,
    id: RepositoryId,
) -> Result<Option<ProjectRepository>> {
    let row = sqlx::query_as!(
        RepositoryRow,
        r#"
        select id, tenant_id, project_id, provider, canonical_uri, repository_owner,
               repository_name, default_branch, local_fingerprint, metadata,
               created_by, created_at, updated_at
        from project_repositories
        where tenant_id = $1 and project_id = $2 and id = $3
        "#,
        tenant_id.as_uuid(),
        project_id.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Detaches a repository. Returns whether a row was removed, so the caller can
/// answer a repeated delete as a 404 rather than a silent success.
#[tracing::instrument(
    name = "store.repositories.detach",
    skip_all,
    fields(tenant.id = %tenant_id, project.id = %project_id, repository.id = %id),
    err(Display)
)]
pub async fn detach(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    project_id: ProjectId,
    id: RepositoryId,
) -> Result<bool> {
    let removed = sqlx::query!(
        r#"
        delete from project_repositories
        where tenant_id = $1 and project_id = $2 and id = $3
        "#,
        tenant_id.as_uuid(),
        project_id.as_uuid(),
        id.as_uuid(),
    )
    .execute(executor)
    .await
    .map_err(storage_error)?
    .rows_affected()
        > 0;
    if removed {
        metrics::counter!(REPOSITORY_MUTATIONS_TOTAL, "operation" => "detach").increment(1);
    }
    Ok(removed)
}
