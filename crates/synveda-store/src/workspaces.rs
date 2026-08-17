//! Workspaces (CPR-4, ADR-0071): the product-level subtype of a
//! `workspace`-shaped governed scope.
//!
//! ## The transaction is the guarantee
//!
//! [`create`] mints a scope and a workspace row, in that order, and the caller
//! MUST wrap it in a transaction — on the data path that means
//! [`crate::rls::begin_tenant_tx`]. There is no compensating delete and there
//! must not be one: a failure between the two statements rolls the first one
//! back, so the failure modes are "both" and "neither" and there is no third.
//! The order is deliberate — the scope first, so that a duplicate slug is
//! refused by the scope tree's own sibling rule before a workspace row exists
//! to be undone.
//!
//! The tenant root is minted on the way past when nobody has made one
//! ([`crate::scopes::ensure_tenant_root`]), because a person creating their
//! first workspace has no reason to have created a tenant scope first.
//!
//! ## Governance attaches above this module
//!
//! Like [`crate::scopes`], nothing here decides authorisation and nothing here
//! chains an audit event: the PDP decision goes in front of the call and the
//! event goes in the same transaction, both at the gateway (seed §2.2 puts
//! exactly one decision point on the request path).
//!
//! ## Tenancy
//!
//! Every query filters on `tenant_id` in SQL as well as relying on the
//! forced-RLS backstop, for [`crate::scopes`]'s reason: these functions are
//! also called on owner connections, where RLS does not bite. Another tenant's
//! workspace reads as absent rather than forbidden (ADR-0008).

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use synveda_types::scope::{ScopeKind, ScopeStatus, validate_display_name, validate_slug};
use synveda_types::workspace::{LifecycleStatus, Workspace, validate_description};
use synveda_types::{Error, IdentityId, Result, ScopeId, TenantId, WorkspaceId};
use uuid::Uuid;

use crate::scopes::{self, NewScope};

/// Counter: workspace and project mutations, labelled `subtype` =
/// `workspace` | `project` and `operation` = `create` | `update`. Emitted
/// here, described by the gateway where the recorder lives (ADR-0007).
pub const SUBTYPE_MUTATIONS_TOTAL: &str = "synveda_subtype_mutations_total";

/// What [`create`] needs.
///
/// The id is the caller's to choose (UUIDv7, mintable anywhere — ADR-0005):
/// the aggregate id is stable for the workspace's whole life and the caller is
/// usually about to reference it in the same transaction.
#[derive(Debug, Clone)]
pub struct NewWorkspace {
    /// The workspace's identity.
    pub id: WorkspaceId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Tenant-unique handle; becomes the owned scope's slug too.
    pub slug: String,
    /// Display name.
    pub display_name: String,
    /// Optional prose.
    pub description: Option<String>,
    /// The identity creating it, when one is.
    pub created_by: Option<IdentityId>,
}

/// A partial update, with the precondition it is applied under.
///
/// `description` is a double option on purpose: absent leaves the description
/// alone, `Some(None)` clears it, and `Some(Some(text))` replaces it. A single
/// `Option` cannot say "clear" and "leave" apart, and a surface that cannot
/// clear a field grows a second endpoint that can.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceUpdate {
    /// New display name, when renaming.
    pub display_name: Option<String>,
    /// New description; see the struct docs for the three cases.
    pub description: Option<Option<String>>,
    /// New lifecycle status. Mirrored onto the owned scope in the same
    /// transaction — an archived workspace whose scope still read `active`
    /// would compose, resolve and accept writes exactly as before.
    pub status: Option<LifecycleStatus>,
}

impl WorkspaceUpdate {
    /// Whether this update would change anything. An empty PATCH is a client
    /// bug, and answering it with a 200 and a bumped revision hides one.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.display_name.is_none() && self.description.is_none() && self.status.is_none()
    }
}

/// Raw row; converted with `TryFrom` so `status` decodes through the
/// `synveda-types` enum (the pattern [`crate::tenants`] set).
struct WorkspaceRow {
    id: Uuid,
    tenant_id: Uuid,
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

impl TryFrom<WorkspaceRow> for Workspace {
    type Error = Error;

    fn try_from(row: WorkspaceRow) -> Result<Self> {
        Ok(Workspace {
            id: WorkspaceId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            scope_id: ScopeId::from_uuid(row.scope_id),
            slug: row.slug,
            display_name: row.display_name,
            description: row.description,
            // The CHECK constraint keeps this inside its vocabulary; a parse
            // failure means schema and code have drifted — a bug.
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

/// Maps a sqlx error at the storage boundary into the shared taxonomy.
///
/// The same classification [`crate::scopes`] uses, and the same reasoning: a
/// unique or foreign-key violation is a conflict with concurrent state, a
/// check violation is a caller who sent something invalid, and the
/// immutability trigger firing is an application defect rather than the
/// caller's fault.
pub(crate) fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        if matches!(
            db.code().as_deref(),
            Some("23505") | Some("23503") | Some("40P01")
        ) {
            return Error::Conflict {
                message: db.to_string(),
            };
        }
        if db.code().as_deref() == Some("23514") {
            return Error::Invalid {
                message: db.to_string(),
            };
        }
        if db.code().as_deref() == Some("P0001") {
            return Error::Internal {
                message: db.message().to_owned(),
            };
        }
        if db.code().as_deref() == Some("42501") {
            return crate::rls::backstop_error(db);
        }
    }
    Error::Storage {
        message: err.to_string(),
    }
}

fn not_found(id: WorkspaceId) -> Error {
    Error::NotFound {
        entity: format!("workspace {id}"),
    }
}

/// Creates a workspace and the governed scope it owns, in the caller's
/// transaction.
///
/// Fails with [`Error::Invalid`] for a malformed slug, name or description and
/// [`Error::Conflict`] when the slug is taken. The tenant root is created if
/// this is the first thing that needed one.
///
/// Must run inside a transaction (see module docs).
#[tracing::instrument(
    name = "store.workspaces.create",
    skip_all,
    fields(tenant.id = %new.tenant_id, workspace.id = %new.id, scope.id = tracing::field::Empty),
    err(Display)
)]
pub async fn create(conn: &mut PgConnection, new: &NewWorkspace) -> Result<Workspace> {
    validate_slug(&new.slug)?;
    validate_display_name(&new.display_name)?;
    validate_description(new.description.as_deref())?;

    let root = scopes::ensure_tenant_root(&mut *conn, new.tenant_id).await?;
    let scope = scopes::create(
        &mut *conn,
        &NewScope {
            id: ScopeId::new(),
            tenant_id: new.tenant_id,
            kind: ScopeKind::Workspace,
            parent_scope_id: Some(root.id),
            slug: new.slug.clone(),
            display_name: new.display_name.clone(),
            attributes: serde_json::json!({}),
            created_by: new.created_by,
        },
    )
    .await?;
    tracing::Span::current().record("scope.id", tracing::field::display(scope.id));

    let row = sqlx::query_as!(
        WorkspaceRow,
        r#"
        insert into workspaces
            (id, tenant_id, scope_id, scope_kind, slug, display_name, description,
             status, created_by)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        returning id, tenant_id, scope_id, slug, display_name, description,
                  status, revision, created_by, created_at, updated_at
        "#,
        new.id.as_uuid(),
        new.tenant_id.as_uuid(),
        scope.id.as_uuid(),
        ScopeKind::Workspace.as_str(),
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
        "subtype" => "workspace",
        "operation" => "create",
    )
    .increment(1);
    row.try_into()
}

/// Fetches one workspace.
#[tracing::instrument(
    name = "store.workspaces.get",
    skip_all,
    fields(tenant.id = %tenant_id, workspace.id = %id),
    err(Display)
)]
pub async fn get(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: WorkspaceId,
) -> Result<Option<Workspace>> {
    let row = sqlx::query_as!(
        WorkspaceRow,
        r#"
        select id, tenant_id, scope_id, slug, display_name, description,
               status, revision, created_by, created_at, updated_at
        from workspaces
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

/// Fetches a workspace by its handle.
#[tracing::instrument(
    name = "store.workspaces.by_slug",
    skip_all,
    fields(tenant.id = %tenant_id, workspace.slug = slug),
    err(Display)
)]
pub async fn by_slug(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    slug: &str,
) -> Result<Option<Workspace>> {
    let row = sqlx::query_as!(
        WorkspaceRow,
        r#"
        select id, tenant_id, scope_id, slug, display_name, description,
               status, revision, created_by, created_at, updated_at
        from workspaces
        where tenant_id = $1 and slug = $2
        "#,
        tenant_id.as_uuid(),
        slug,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Lists a tenant's workspaces, ordered by slug.
///
/// Archived workspaces are included: the caller decides what to show, and a
/// listing that silently omitted them would make an archived workspace
/// indistinguishable from one that never existed.
#[tracing::instrument(
    name = "store.workspaces.list",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn list(executor: impl PgExecutor<'_>, tenant_id: TenantId) -> Result<Vec<Workspace>> {
    let rows = sqlx::query_as!(
        WorkspaceRow,
        r#"
        select id, tenant_id, scope_id, slug, display_name, description,
               status, revision, created_by, created_at, updated_at
        from workspaces
        where tenant_id = $1
        order by slug
        "#,
        tenant_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Applies an update under a revision precondition.
///
/// `expected_revision` is the revision the caller last saw. A mismatch is
/// [`Error::Conflict`] and nothing is written — that is the whole of the
/// lost-update protection, and it is why the column is monotonic in a trigger
/// rather than in this function.
///
/// A status change is mirrored onto the owned scope in the same transaction.
/// Fails with [`Error::NotFound`] for a workspace that is not this tenant's
/// and [`Error::Invalid`] for an empty update or a malformed field.
///
/// Must run inside a transaction (the scope mirror is a second statement).
#[tracing::instrument(
    name = "store.workspaces.update",
    skip_all,
    fields(
        tenant.id = %tenant_id,
        workspace.id = %id,
        workspace.expected_revision = expected_revision,
    ),
    err(Display)
)]
pub async fn update(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    id: WorkspaceId,
    expected_revision: i64,
    update: &WorkspaceUpdate,
) -> Result<Workspace> {
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
        WorkspaceRow,
        r#"
        update workspaces
           set display_name = coalesce($4, display_name),
               description  = case when $5 then $6 else description end,
               status       = coalesce($7, status),
               revision     = revision + 1,
               updated_at   = now()
         where id = $1 and tenant_id = $2 and revision = $3
        returning id, tenant_id, scope_id, slug, display_name, description,
                  status, revision, created_by, created_at, updated_at
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
        // Zero rows means one of two things and the caller must be able to
        // tell them apart: a workspace that is not this tenant's is a 404, a
        // revision that moved under the caller is a 409.
        return Err(match get(&mut *conn, tenant_id, id).await? {
            Some(current) => Error::Conflict {
                message: format!(
                    "workspace {id} is at revision {}, not {expected_revision}",
                    current.revision
                ),
            },
            None => not_found(id),
        });
    };
    let workspace: Workspace = row.try_into()?;

    if let Some(status) = update.status {
        scopes::set_status(
            &mut *conn,
            tenant_id,
            workspace.scope_id,
            scope_status(status),
        )
        .await?;
    }

    metrics::counter!(
        SUBTYPE_MUTATIONS_TOTAL,
        "subtype" => "workspace",
        "operation" => "update",
    )
    .increment(1);
    Ok(workspace)
}

/// The scope status a subtype status implies. Two vocabularies with the same
/// two words, mapped in one place so they cannot drift apart — and shared with
/// [`crate::projects`], which mirrors the same way.
pub(crate) const fn scope_status(status: LifecycleStatus) -> ScopeStatus {
    match status {
        LifecycleStatus::Active => ScopeStatus::Active,
        LifecycleStatus::Archived => ScopeStatus::Archived,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_update_changes_nothing_and_says_so() {
        assert!(WorkspaceUpdate::default().is_empty());
        assert!(
            !WorkspaceUpdate {
                description: Some(None),
                ..Default::default()
            }
            .is_empty(),
            "clearing a description is a change"
        );
    }

    #[test]
    fn the_two_status_vocabularies_map_one_to_one() {
        for status in LifecycleStatus::ALL {
            assert_eq!(scope_status(*status).as_str(), status.as_str());
        }
    }
}
