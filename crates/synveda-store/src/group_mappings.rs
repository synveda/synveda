//! The group-mapping override table (AUTH-2, ADR-0013 decision 3): exact
//! IdP group names mapped to hierarchy scopes, consulted before the
//! `synveda-{dept}-{team}` convention during JIT provisioning.
//!
//! Admin-curated at the store level for now (like policy packs
//! pre-AUTHZ-2); an API surface arrives with the admin console. The table
//! is tenant-scoped (forced RLS, ADR-0009): reach it inside
//! [`crate::rls::begin_tenant_tx`].

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use synveda_types::{Error, Result, ScopeId, TenantId};
use uuid::Uuid;

/// One override: an exact IdP group name bound to a scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupMapping {
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// The IdP group name, matched exactly.
    pub group_name: String,
    /// The scope users in this group are placed under.
    pub scope_id: ScopeId,
    /// When the mapping was created or last re-pointed.
    pub created_at: DateTime<Utc>,
}

struct GroupMappingRow {
    tenant_id: Uuid,
    group_name: String,
    scope_id: Uuid,
    created_at: DateTime<Utc>,
}

impl From<GroupMappingRow> for GroupMapping {
    fn from(row: GroupMappingRow) -> Self {
        GroupMapping {
            tenant_id: TenantId::from_uuid(row.tenant_id),
            group_name: row.group_name,
            scope_id: ScopeId::from_uuid(row.scope_id),
            created_at: row.created_at,
        }
    }
}

/// Maps a sqlx error at the storage boundary into the shared taxonomy.
fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        // 23503 foreign_key_violation: no such tenant or scope.
        if db.code().as_deref() == Some("23503") {
            return Error::NotFound {
                entity: "mapping target scope".to_owned(),
            };
        }
        // 23514 check_violation: group name outside the length bound.
        if db.code().as_deref() == Some("23514") {
            return Error::Invalid {
                message: db.to_string(),
            };
        }
        // 42501 insufficient_privilege: the RLS backstop (ADR-0009).
        if db.code().as_deref() == Some("42501") {
            return Error::Internal {
                message: format!("row-level security or privilege violation: {db}"),
            };
        }
    }
    Error::Storage {
        message: err.to_string(),
    }
}

/// Creates or re-points the mapping for `group_name`.
#[tracing::instrument(
    name = "store.group_mappings.upsert",
    skip_all,
    fields(tenant.id = %tenant_id, mapping.group = group_name, scope.id = %scope_id),
    err(Display)
)]
pub async fn upsert(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    group_name: &str,
    scope_id: ScopeId,
) -> Result<GroupMapping> {
    let row = sqlx::query_as!(
        GroupMappingRow,
        r#"
        insert into group_mappings (tenant_id, group_name, scope_id)
        values ($1, $2, $3)
        on conflict (tenant_id, group_name) do update
            set scope_id = excluded.scope_id, created_at = now()
        returning tenant_id, group_name, scope_id, created_at
        "#,
        tenant_id.as_uuid(),
        group_name,
        scope_id.as_uuid(),
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    Ok(row.into())
}

/// Removes the mapping for `group_name`. Returns whether a row existed.
#[tracing::instrument(
    name = "store.group_mappings.remove",
    skip_all,
    fields(tenant.id = %tenant_id, mapping.group = group_name),
    err(Display)
)]
pub async fn remove(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    group_name: &str,
) -> Result<bool> {
    let result = sqlx::query!(
        "delete from group_mappings where tenant_id = $1 and group_name = $2",
        tenant_id.as_uuid(),
        group_name,
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(result.rows_affected() > 0)
}

/// The overrides matching any of `groups`, in group-name order — the
/// deterministic precedence JIT resolution relies on (ADR-0013 decision 3).
#[tracing::instrument(
    name = "store.group_mappings.for_groups",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn for_groups(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    groups: &[String],
) -> Result<Vec<GroupMapping>> {
    let rows = sqlx::query_as!(
        GroupMappingRow,
        r#"
        select tenant_id, group_name, scope_id, created_at
        from group_mappings
        where tenant_id = $1 and group_name = any($2)
        order by group_name
        "#,
        tenant_id.as_uuid(),
        groups,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    Ok(rows.into_iter().map(Into::into).collect())
}
