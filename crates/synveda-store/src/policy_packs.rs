//! The per-tenant stored policy packs (AUTHZ-1 ADR-0012 decision 5;
//! named-per-tenant since AUTHZ-2, ADR-0014 decision 6).
//!
//! One row per (tenant, name); [`apply`] upserts and owns the per-name
//! version bump, the gateway's refresher polls [`stored`] and reconciles
//! compiled packs into the PDP, and [`clear`] removes a pack — refusing
//! while assignments or the tenant default still reference it, so a
//! dangling reference can only come from out-of-band writes. The product
//! pack names are reserved by a check constraint: `regulated-strict` means
//! the same thing in every tenant. `policy_packs` is tenant-scoped (forced
//! RLS, ADR-0009): reach it inside [`crate::rls::begin_tenant_tx`].
//!
//! The store neither parses nor validates Cedar — that is `synveda-policy`
//! (storage knows nothing of policy, seed §2.4). Callers compile-check
//! before applying; the refresher rejects a bad pack at reload time and
//! keeps the tenant's last-good pack.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use sqlx::postgres::PgConnection;
use synveda_types::{Error, Result, TenantId};

/// A tenant's stored policy pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyPack {
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Pack name (slug grammar; product names are reserved), e.g.
    /// `acme-strict`.
    pub name: String,
    /// Monotonically increasing per (tenant, name); bumped by every
    /// [`apply`].
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
        // 23514 check_violation: malformed name, reserved product name,
        // or empty source.
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

/// Applies a pack under the tenant's `name`: inserts at version 1, or
/// replaces the existing row and bumps its version — every apply is a new
/// version, so the reloader's unchanged-skip and the decision log both see
/// the change.
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
        on conflict (tenant_id, name) do update
            set source = excluded.source,
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

/// All of the tenant's stored packs — the refresher's reconciliation
/// input.
#[tracing::instrument(
    name = "store.policy_packs.stored",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn stored(executor: impl PgExecutor<'_>, tenant_id: TenantId) -> Result<Vec<PolicyPack>> {
    let rows = sqlx::query_as!(
        PolicyPackRow,
        r#"
        select tenant_id, name, version, source, updated_at
        from policy_packs where tenant_id = $1
        order by name
        "#,
        tenant_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// One stored pack by name.
#[tracing::instrument(
    name = "store.policy_packs.get",
    skip_all,
    fields(tenant.id = %tenant_id, policy.pack = name),
    err(Display)
)]
pub async fn get(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    name: &str,
) -> Result<Option<PolicyPack>> {
    let row = sqlx::query_as!(
        PolicyPackRow,
        r#"
        select tenant_id, name, version, source, updated_at
        from policy_packs where tenant_id = $1 and name = $2
        "#,
        tenant_id.as_uuid(),
        name,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    Ok(row.map(Into::into))
}

/// Removes a stored pack, refusing while any assignment or the tenant
/// default still references it (ADR-0014 decision 7: the dangling-name
/// fallback exists for out-of-band writes, not for the product path).
/// Returns whether a row was removed.
#[tracing::instrument(
    name = "store.policy_packs.clear",
    skip_all,
    fields(tenant.id = %tenant_id, policy.pack = name),
    err(Display)
)]
pub async fn clear(conn: &mut PgConnection, tenant_id: TenantId, name: &str) -> Result<bool> {
    let referenced = sqlx::query_scalar!(
        r#"
        select exists (
            select 1 from policy_pack_assignments
            where tenant_id = $1 and pack_name = $2
            union all
            select 1 from policy_pack_defaults
            where tenant_id = $1 and pack_name = $2
        ) as "referenced!"
        "#,
        tenant_id.as_uuid(),
        name,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    if referenced {
        return Err(Error::Conflict {
            message: format!(
                "pack {name:?} is still assigned (or the tenant default); \
                 reassign those scopes first"
            ),
        });
    }
    let result = sqlx::query!(
        "delete from policy_packs where tenant_id = $1 and name = $2",
        tenant_id.as_uuid(),
        name,
    )
    .execute(conn)
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
