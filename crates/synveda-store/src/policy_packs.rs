//! The per-tenant policy pack store (AUTHZ-1, ADR-0012 decision 5).
//!
//! One active pack row per tenant; [`apply`] upserts and owns the version
//! bump, the gateway's refresher polls [`active`] and hot-swaps compiled
//! packs into the PDP, and [`clear`] drops a tenant back to the embedded
//! `bootstrap` pack. `policy_packs` is tenant-scoped (forced RLS,
//! ADR-0009): reach it inside [`crate::rls::begin_tenant_tx`].
//!
//! The store neither parses nor validates Cedar — that is `synveda-policy`
//! (storage knows nothing of policy, seed §2.4). Callers compile-check
//! before applying; the refresher rejects a bad pack at reload time and
//! keeps the tenant's last-good pack.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use synveda_types::{Error, Result, TenantId};

/// A tenant's stored policy pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyPack {
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Pack name (slug grammar), e.g. `regulated-strict`.
    pub name: String,
    /// Monotonically increasing per tenant; bumped by every [`apply`].
    pub version: i64,
    /// Cedar policy source.
    pub source: String,
    /// When the pack was last applied.
    pub updated_at: DateTime<Utc>,
}

/// Maps a sqlx error at the storage boundary into the shared taxonomy.
fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        // 23503 foreign_key_violation: no such tenant.
        if db.code().as_deref() == Some("23503") {
            return Error::NotFound {
                entity: "tenant".to_owned(),
            };
        }
        // 23514 check_violation: malformed name or empty source.
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

/// Applies a pack for the tenant: inserts at version 1, or replaces the
/// existing row and bumps its version — every apply is a new version, so
/// the reloader's unchanged-skip and the decision log both see the change.
#[tracing::instrument(
    name = "store.policy_packs.apply",
    skip_all,
    fields(tenant.id = %tenant_id, policy.pack = name),
    err(Display)
)]
pub async fn apply(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    name: &str,
    source: &str,
) -> Result<PolicyPack> {
    let row = sqlx::query_as!(
        PolicyPackRow,
        r#"
        insert into policy_packs (tenant_id, name, version, source)
        values ($1, $2, 1, $3)
        on conflict (tenant_id) do update
            set name = excluded.name,
                source = excluded.source,
                version = policy_packs.version + 1,
                updated_at = now()
        returning tenant_id, name, version, source, updated_at
        "#,
        tenant_id.as_uuid(),
        name,
        source,
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    Ok(row.into())
}

/// The tenant's active pack, or `None` when it runs the embedded default.
#[tracing::instrument(
    name = "store.policy_packs.active",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn active(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
) -> Result<Option<PolicyPack>> {
    let row = sqlx::query_as!(
        PolicyPackRow,
        r#"
        select tenant_id, name, version, source, updated_at
        from policy_packs where tenant_id = $1
        "#,
        tenant_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    Ok(row.map(Into::into))
}

/// Removes the tenant's stored pack (back to the embedded default).
/// Returns whether a row was removed.
#[tracing::instrument(
    name = "store.policy_packs.clear",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn clear(executor: impl PgExecutor<'_>, tenant_id: TenantId) -> Result<bool> {
    let result = sqlx::query!(
        "delete from policy_packs where tenant_id = $1",
        tenant_id.as_uuid(),
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(result.rows_affected() > 0)
}

struct PolicyPackRow {
    tenant_id: uuid::Uuid,
    name: String,
    version: i64,
    source: String,
    updated_at: DateTime<Utc>,
}

impl From<PolicyPackRow> for PolicyPack {
    fn from(row: PolicyPackRow) -> Self {
        PolicyPack {
            tenant_id: TenantId::from_uuid(row.tenant_id),
            name: row.name,
            version: row.version,
            source: row.source,
            updated_at: row.updated_at,
        }
    }
}
