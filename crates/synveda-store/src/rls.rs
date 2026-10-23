//! The application side of the tenant-isolation backstop (ADR-0009).
//!
//! Migration 0003 installs forced row-level-security policies keyed to the
//! `synveda.tenant_id` GUC on every tenant-scoped table. This module owns
//! setting that GUC: every tenant-scoped database interaction happens inside
//! a transaction opened by [`begin_tenant_tx`]. A connection that skipped it
//! reads and writes zero tenant-scoped rows — the backstop fails closed.
//!
//! The tenant is an explicit argument, never read from the task-local: the
//! store stays below the identity seam (ADR-0008).

use sqlx::{PgPool, Postgres, Transaction};
use synveda_types::{Error, Result, TenantId};

use crate::runtime_role::DatabaseRoles;

/// The stable prefix every module's `storage_error` mapper puts on a
/// backstop trip (SQLSTATE 42501). The taxonomy stays coarse (
/// detail in messages, not variants); [`is_backstop_trip`] is the one
/// interpreter of this marker.
const BACKSTOP_PREFIX: &str = "row-level security or privilege violation";

/// The taxonomy rendering of a backstop trip: always [`Error::Internal`] —
/// the app-level tenant scoping failed, which is our bug, never the
/// caller's. Every store module's 42501 arm builds its error here so the
/// marker prefix stays in one place.
pub fn backstop_error(detail: impl std::fmt::Display) -> Error {
    Error::Internal {
        message: format!("{BACKSTOP_PREFIX}: {detail}"),
    }
}

/// Whether `error` is an RLS-backstop trip — the gateway's audit seam
/// (`store.rls.denied`, ADR-0019 decision 5) classifies with this
/// instead of parsing messages itself.
#[must_use]
pub fn is_backstop_trip(error: &Error) -> bool {
    matches!(error, Error::Internal { message } if message.starts_with(BACKSTOP_PREFIX))
}

/// Begins a transaction scoped to `tenant_id` for RLS purposes.
///
/// The GUC is transaction-local (`set_config(..., is_local := true)`): it
/// vanishes on commit or rollback, so a pooled connection can never carry a
/// tenant into the next request that borrows it. Callers commit the returned
/// transaction as usual; dropping it rolls back, GUC included.
///
/// Enforcement applies only to roles subject to RLS. An owner, superuser or
/// role with `BYPASSRLS` could bypass policies, so deployment contracts refuse
/// those attributes for gateway and worker roles — see ADR-0009.
#[tracing::instrument(name = "store.rls.begin_tenant_tx", skip_all, fields(tenant.id = %tenant_id), err(Display))]
pub async fn begin_tenant_tx(
    pool: &PgPool,
    tenant_id: TenantId,
) -> Result<Transaction<'static, Postgres>> {
    begin_admission_tx(pool, tenant_id, false).await
}

/// Begins an exclusive tenant admission transaction before authorization and
/// domain locks. Invalidation writers must choose this at entry, never upgrade
/// a shared transaction. No provider/network wait may hold either gate.
pub async fn begin_tenant_write_tx(
    pool: &PgPool,
    tenant_id: TenantId,
) -> Result<Transaction<'static, Postgres>> {
    begin_admission_tx(pool, tenant_id, true).await
}

async fn begin_admission_tx(
    pool: &PgPool,
    tenant_id: TenantId,
    exclusive: bool,
) -> Result<Transaction<'static, Postgres>> {
    let mut tx = pool.begin().await.map_err(|err| Error::Storage {
        message: format!("begin tenant transaction: {err}"),
    })?;
    sqlx::query_scalar!(
        "select set_config('synveda.tenant_id', $1, true)",
        tenant_id.as_uuid().to_string(),
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|err| Error::Storage {
        message: format!("set tenant GUC: {err}"),
    })?;
    sqlx::query!("select set_config('lock_timeout', '5s', true)")
        .fetch_one(&mut *tx)
        .await
        .map_err(|err| Error::Storage {
            message: format!("bound tenant admission lock wait: {err}"),
        })?;
    // Hash collisions can only add contention, never weaken tenant isolation.
    // Acquire before reading product state so a waiter gets a fresh snapshot
    // on its subsequent READ COMMITTED statements.
    if exclusive {
        sqlx::query!(
            "select pg_advisory_xact_lock(hashtextextended('synveda.admission:' || $1::text, 0))",
            tenant_id.as_uuid().to_string(),
        )
        .execute(&mut *tx)
        .await
        .map_err(|err| Error::Storage {
            message: format!("acquire exclusive tenant admission: {err}"),
        })?;
    } else {
        sqlx::query!(
            "select pg_advisory_xact_lock_shared(hashtextextended('synveda.admission:' || $1::text, 0))",
            tenant_id.as_uuid().to_string(),
        )
        .execute(&mut *tx)
        .await
        .map_err(|err| Error::Storage {
            message: format!("acquire shared tenant admission: {err}"),
        })?;
    }
    Ok(tx)
}

/// Begins one tenant-scoped transaction only after the same physical
/// connection has proved the closed migrator authority contract.
///
/// Tenant admission is the one governed bootstrap mutation that must run as
/// the schema owner. Proving a pooled connection and then borrowing another
/// for the write would leave a target/role swap between proof and effect. This
/// helper keeps session initialisation, repeatable-read authority proof, the
/// transaction-local tenant GUC and every caller write on one transaction.
#[tracing::instrument(
    name = "store.rls.begin_migrator_tenant_tx",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn begin_migrator_tenant_tx(
    pool: &PgPool,
    tenant_id: TenantId,
    database_roles: &DatabaseRoles,
) -> Result<Transaction<'static, Postgres>> {
    let mut tx = pool.begin().await.map_err(|error| Error::Storage {
        message: format!("begin migrator tenant transaction: {error}"),
    })?;
    crate::runtime_role::configure_authority_snapshot_connection(&mut tx).await?;
    crate::runtime_role::initialize_product_session_connection(&mut tx).await?;
    crate::runtime_role::verify_migrator_connection(&mut tx, database_roles).await?;
    crate::epoch::verify_connection(&mut tx)
        .await
        .map_err(|error| Error::Invalid {
            message: format!("the migrator database is not at the required schema epoch: {error}"),
        })?;
    sqlx::query_scalar!(
        "select set_config('synveda.tenant_id', $1, true)",
        tenant_id.as_uuid().to_string(),
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|error| Error::Storage {
        message: format!("set tenant GUC after migrator authority proof: {error}"),
    })?;
    Ok(tx)
}
