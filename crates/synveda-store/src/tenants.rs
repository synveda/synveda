//! Tenant storage (ADR-0008).
//!
//! Admit and resolve only: [`create`] admits a tenant, [`by_id`] resolves a
//! token's `tid` claim to a row. Lifecycle transitions belong to tenant lifecycle.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use synveda_types::{Error, Result, Tenant, TenantId, TenantStatus};
use uuid::Uuid;

/// Raw row; converted with `TryFrom` so `status` decodes through the
/// `synveda-types` enum (same pattern as [`crate::records`]).
struct TenantRow {
    id: Uuid,
    slug: String,
    name: String,
    status: String,
    created_at: DateTime<Utc>,
}

impl TryFrom<TenantRow> for Tenant {
    type Error = Error;

    fn try_from(row: TenantRow) -> Result<Self> {
        Ok(Tenant {
            id: TenantId::from_uuid(row.id),
            slug: row.slug,
            name: row.name,
            // The CHECK constraint keeps this inside the vocabulary; a parse
            // failure means schema and code have drifted — a bug.
            status: row.status.parse().map_err(|err| Error::Internal {
                message: format!("stored value outside vocabulary: {err}"),
            })?,
            created_at: row.created_at,
        })
    }
}

fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        // 23505 unique_violation: duplicate id or slug.
        if db.code().as_deref() == Some("23505") {
            return Error::Conflict {
                message: db.to_string(),
            };
        }
        // 23514 check_violation: slug or status outside the vocabulary —
        // the caller sent something invalid, not a storage fault.
        if db.code().as_deref() == Some("23514") {
            return Error::Invalid {
                message: db.to_string(),
            };
        }
    }
    Error::Storage {
        message: err.to_string(),
    }
}

/// Admits a tenant. Fails with [`Error::Conflict`] if the id or slug already
/// exists, [`Error::Invalid`] if the slug is malformed.
#[tracing::instrument(name = "store.tenants.create", skip_all, fields(tenant.id = %id), err(Display))]
pub async fn create(
    executor: impl PgExecutor<'_>,
    id: TenantId,
    slug: &str,
    name: &str,
    status: TenantStatus,
) -> Result<Tenant> {
    let row = sqlx::query_as!(
        TenantRow,
        r#"
        insert into tenants (id, slug, name, status)
        values ($1, $2, $3, $4)
        returning id, slug, name, status, created_at
        "#,
        id.as_uuid(),
        slug,
        name,
        status.as_str(),
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    row.try_into()
}

/// Lists all active tenants, admission order — the current policy-pack
/// refresher's iteration set (ADR-0012 decision 5). This global
/// scan is unpaged; gateway horizontal scale has not selected or enforced a tenant work ceiling.
#[tracing::instrument(name = "store.tenants.active", skip_all, err(Display))]
pub async fn active(executor: impl PgExecutor<'_>) -> Result<Vec<Tenant>> {
    let rows = sqlx::query_as!(
        TenantRow,
        "select id, slug, name, status, created_at from tenants where status = 'active' order by id",
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Lists every admitted tenant id regardless of status, admission order, for
/// deployment-wide key custody. Refuses rather than returning a partial set
/// when more than `limit` tenants exist.
#[tracing::instrument(name = "store.tenants.all_ids", skip_all, err(Display))]
pub async fn all_ids(executor: impl PgExecutor<'_>, limit: usize) -> Result<Vec<TenantId>> {
    let rows = sqlx::query_scalar!(
        "select id from tenants order by id limit $1",
        i64::try_from(limit).unwrap_or(i64::MAX).saturating_add(1),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    if rows.len() > limit {
        return Err(Error::Invalid {
            message: format!("the deployment holds more than {limit} tenants"),
        });
    }
    Ok(rows.into_iter().map(TenantId::from_uuid).collect())
}

/// Lists every tenant whose keys operator custody still manages: active and
/// suspended, in admission order. An erasing tenant's keys are deleted by
/// erasure without being opened, and an erased tenant has none (ADR-0157).
/// Refuses rather than returning a partial set when more than `limit` exist.
pub async fn custody_ids(executor: impl PgExecutor<'_>, limit: usize) -> Result<Vec<TenantId>> {
    let rows: Vec<uuid::Uuid> = sqlx::query_scalar(
        "select id from tenants where status in ('active', 'suspended') order by id limit $1",
    )
    .bind(i64::try_from(limit).unwrap_or(i64::MAX).saturating_add(1))
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    if rows.len() > limit {
        return Err(Error::Invalid {
            message: format!("the deployment holds more than {limit} custody tenants"),
        });
    }
    Ok(rows.into_iter().map(TenantId::from_uuid).collect())
}

/// Resolves a tenant by id — the lookup behind every request's tenant
/// resolution. Returns `None` for an unknown id; the *caller* decides what
/// suspension means (the gateway treats non-active as unresolvable).
#[tracing::instrument(name = "store.tenants.by_id", skip_all, fields(tenant.id = %id), err(Display))]
pub async fn by_id(executor: impl PgExecutor<'_>, id: TenantId) -> Result<Option<Tenant>> {
    let row = sqlx::query_as!(
        TenantRow,
        "select id, slug, name, status, created_at from tenants where id = $1",
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}
