//! Storage traits and their Postgres/pgvector/AGE implementations, including the
//! bitemporal record tables and, later, the `VectorIndex` trait that isolates
//! pgvector from the Qdrant scale-out path (tech plan §1.1).
//!
//! Bitemporal layout (FND-4, ADR-0006): each bitemporal entity is a
//! current/history table pair. Transaction time is maintained exclusively by
//! database triggers; valid time is application data. See
//! `migrations/0001_bitemporal_records.sql` and the [`records`] module.
//!
//! Tenant isolation backstop (TEN-2, ADR-0009): tenant-scoped tables carry
//! forced RLS policies keyed to a transaction-local GUC. Reach them through
//! [`rls::begin_tenant_tx`]; see `migrations/0003_tenant_rls.sql`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod hierarchy;
pub mod records;
pub mod rls;
pub mod tenants;

use sqlx::migrate::Migrator;
use sqlx::{PgExecutor, PgPool};
use synveda_types::{Error, Result};

/// The workspace's sqlx migrations, embedded at compile time from
/// `crates/synveda-store/migrations/`.
pub static MIGRATOR: Migrator = sqlx::migrate!();

/// Applies all pending migrations. Idempotent; safe to run concurrently
/// (sqlx serialises runners with an advisory lock).
#[tracing::instrument(name = "store.migrate", skip_all, err(Display))]
pub async fn migrate(pool: &PgPool) -> Result<()> {
    MIGRATOR.run(pool).await.map_err(|err| Error::Storage {
        message: format!("migration failed: {err}"),
    })
}

/// Round-trips the database connection (`SELECT 1`). The store leg of the
/// readiness path (FND-5, ADR-0007); reads no application data.
#[tracing::instrument(name = "store.ping", skip_all, err(Display))]
pub async fn ping(executor: impl PgExecutor<'_>) -> Result<()> {
    sqlx::query_scalar!("select 1")
        .fetch_one(executor)
        .await
        .map(|_| ())
        .map_err(|err| Error::Storage {
            message: err.to_string(),
        })
}
