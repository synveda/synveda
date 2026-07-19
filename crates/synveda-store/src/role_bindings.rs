//! Role bindings (AUTHZ-3, ADR-0015).
//!
//! A binding attaches one product role to a subject at a hierarchy node
//! (`scope_id = None` binds at the tenant itself, in force everywhere).
//! Bindings are request-time data — governed handlers read
//! [`for_subject_on_scopes`] for the resource's chain inside their own
//! transaction and hand the rows to the PDP, so a new binding is in force
//! on the very next request. Tenant-scoped (forced RLS, ADR-0009): reach
//! this table inside [`crate::rls::begin_tenant_tx`].
//!
//! What a role *means* is the PDP's business (seed §2.4); the store only
//! keeps the vocabulary closed (a check constraint mirroring
//! [`synveda_types::Role`]).

use std::str::FromStr;

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use synveda_types::{Error, Result, Role, RoleBinding, ScopeId, TenantId};

/// Maps a sqlx error at the storage boundary into the shared taxonomy.
fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        // 23503 foreign_key_violation: no such tenant or scope.
        if db.code().as_deref() == Some("23503") {
            return Error::NotFound {
                entity: "tenant or scope".to_owned(),
            };
        }
        // 23514 check_violation: unknown role or malformed subject.
        if db.code().as_deref() == Some("23514") {
            return Error::Invalid {
                message: db.to_string(),
            };
        }
        // 42501 insufficient_privilege: the RLS backstop (ADR-0009).
        if db.code().as_deref() == Some("42501") {
            return crate::rls::backstop_error(db);
        }
    }
    Error::Storage {
        message: err.to_string(),
    }
}

/// Binds `role` to `subject` at `scope_id` (`None` is tenant-wide).
/// Idempotent: re-binding refreshes `updated_at`.
#[tracing::instrument(
    name = "store.role_bindings.bind",
    skip_all,
    fields(tenant.id = %tenant_id, role = %role),
    err(Display)
)]
pub async fn bind(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    subject: &str,
    scope_id: Option<ScopeId>,
    role: Role,
) -> Result<RoleBinding> {
    let row = sqlx::query_as!(
        BindingRow,
        r#"
        insert into role_bindings (tenant_id, subject, scope_id, role)
        values ($1, $2, $3, $4)
        on conflict (tenant_id, subject, scope_id, role)
            do update set updated_at = now()
        returning tenant_id, subject, scope_id, role, updated_at
        "#,
        tenant_id.as_uuid(),
        subject,
        scope_id.map(|id| id.as_uuid()),
        role.as_str(),
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    row.try_into()
}

/// Removes one binding. Returns whether a binding was actually removed.
#[tracing::instrument(
    name = "store.role_bindings.unbind",
    skip_all,
    fields(tenant.id = %tenant_id, role = %role),
    err(Display)
)]
pub async fn unbind(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    subject: &str,
    scope_id: Option<ScopeId>,
    role: Role,
) -> Result<bool> {
    let result = sqlx::query!(
        r#"
        delete from role_bindings
        where tenant_id = $1 and subject = $2
          and scope_id is not distinct from $3 and role = $4
        "#,
        tenant_id.as_uuid(),
        subject,
        scope_id.map(|id| id.as_uuid()),
        role.as_str(),
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(result.rows_affected() > 0)
}

/// The subject's bindings relevant to a decision on a resource whose
/// chain is `scope_ids`: rows bound at any of those nodes, plus the
/// subject's tenant-wide rows — exactly what a governed handler hands the
/// PDP (ADR-0015 decision 3). An empty `scope_ids` (a tenant resource)
/// returns the tenant-wide rows only.
#[tracing::instrument(
    name = "store.role_bindings.for_subject_on_scopes",
    skip_all,
    fields(tenant.id = %tenant_id, scope.count = scope_ids.len()),
    err(Display)
)]
pub async fn for_subject_on_scopes(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    subject: &str,
    scope_ids: &[ScopeId],
) -> Result<Vec<RoleBinding>> {
    let ids: Vec<uuid::Uuid> = scope_ids.iter().map(ScopeId::as_uuid).collect();
    let rows = sqlx::query_as!(
        BindingRow,
        r#"
        select tenant_id, subject, scope_id, role, updated_at
        from role_bindings
        where tenant_id = $1 and subject = $2
          and (scope_id is null or scope_id = any($3))
        "#,
        tenant_id.as_uuid(),
        subject,
        &ids,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// The bindings at one node (`GET .../nodes/{id}/roles`).
#[tracing::instrument(
    name = "store.role_bindings.for_scope",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %scope_id),
    err(Display)
)]
pub async fn for_scope(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    scope_id: ScopeId,
) -> Result<Vec<RoleBinding>> {
    let rows = sqlx::query_as!(
        BindingRow,
        r#"
        select tenant_id, subject, scope_id, role, updated_at
        from role_bindings
        where tenant_id = $1 and scope_id = $2
        order by subject, role
        "#,
        tenant_id.as_uuid(),
        scope_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Every binding of the tenant (`GET /v1/roles/bindings`) — the "who
/// holds what where" view an administrator or auditor reads.
#[tracing::instrument(
    name = "store.role_bindings.all",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn all(executor: impl PgExecutor<'_>, tenant_id: TenantId) -> Result<Vec<RoleBinding>> {
    let rows = sqlx::query_as!(
        BindingRow,
        r#"
        select tenant_id, subject, scope_id, role, updated_at
        from role_bindings
        where tenant_id = $1
        order by scope_id nulls first, subject, role
        "#,
        tenant_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

struct BindingRow {
    tenant_id: uuid::Uuid,
    subject: String,
    scope_id: Option<uuid::Uuid>,
    role: String,
    updated_at: DateTime<Utc>,
}

impl TryFrom<BindingRow> for RoleBinding {
    type Error = Error;

    fn try_from(row: BindingRow) -> Result<Self> {
        // The check constraint keeps the column inside the vocabulary; a
        // row that still fails to parse means the constraint and the enum
        // have drifted — an internal fault, not caller input.
        let role = Role::from_str(&row.role).map_err(|_| Error::Internal {
            message: format!("stored role {:?} is outside the vocabulary", row.role),
        })?;
        Ok(RoleBinding {
            tenant_id: TenantId::from_uuid(row.tenant_id),
            subject: row.subject,
            scope_id: row.scope_id.map(ScopeId::from_uuid),
            role,
            updated_at: row.updated_at,
        })
    }
}
