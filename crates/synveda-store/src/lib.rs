//! Storage traits and their Postgres/pgvector/AGE implementations, including the
//! bitemporal record tables and, later, the `VectorIndex` trait that isolates
//! pgvector from the Qdrant scale-out path (tech plan §1.1).
//!
//! Bitemporal layout (FND-4, ADR-0006): each bitemporal entity is a
//! current/history table pair. Transaction time is maintained exclusively by
//! database triggers; valid time is application data. See
//! `migrations/0001_bitemporal_records.sql` and the [`records`] module.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod records;

use sqlx::PgPool;
use sqlx::migrate::Migrator;
use synveda_types::{Error, Result};

/// The workspace's sqlx migrations, embedded at compile time from
/// `crates/synveda-store/migrations/`.
pub static MIGRATOR: Migrator = sqlx::migrate!();

/// Applies all pending migrations. Idempotent; safe to run concurrently
/// (sqlx serialises runners with an advisory lock).
pub async fn migrate(pool: &PgPool) -> Result<()> {
    MIGRATOR.run(pool).await.map_err(|err| Error::Storage {
        message: format!("migration failed: {err}"),
    })
}
