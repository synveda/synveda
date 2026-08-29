//! The application side of the tenant-isolation backstop (TEN-2, ADR-0009).
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
/// backstop trip (SQLSTATE 42501). The taxonomy stays coarse (FND-3:
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
/// (`store.rls.denied`, AUD-1/ADR-0019 decision 5) classifies with this
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
/// Note: enforcement only bites for roles subject to RLS (`synveda_app`, or
/// any non-superuser without BYPASSRLS). The dev compose superuser bypasses
/// policies entirely — see ADR-0009.
#[tracing::instrument(name = "store.rls.begin_tenant_tx", skip_all, fields(tenant.id = %tenant_id), err(Display))]
pub async fn begin_tenant_tx(
    pool: &PgPool,
    tenant_id: TenantId,
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
