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
