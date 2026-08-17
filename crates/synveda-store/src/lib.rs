//! Storage traits and their Postgres/pgvector implementations, including the
//! bitemporal record tables and, later, the `VectorIndex` trait that isolates
//! pgvector from the Qdrant scale-out path (tech plan §1.1).
//!
//! Knowledge graph (GRPH-1, ADR-0043): indexed adjacency in the same
//! database, not Apache AGE — the extension stays installed for the
//! GRPH-4 spike's evidence and is called by nothing. See [`graph`].
//!
//! Bitemporal layout (FND-4, ADR-0006): each bitemporal entity is a
//! current/history table pair. Transaction time is maintained exclusively by
//! database triggers; valid time is application data. See
//! `migrations/0001_bitemporal_records.sql` and the [`records`] module.
//!
//! Tenant isolation backstop (TEN-2, ADR-0009): tenant-scoped tables carry
//! forced RLS policies keyed to a transaction-local GUC. Reach them through
//! [`rls::begin_tenant_tx`]; see `migrations/0003_tenant_rls.sql`.
//!
//! Audit chain tables (AUD-1, ADR-0019): `audit_log` and `audit_chain_heads`
//! are migrated here with the rest of the schema, but their queries live in
//! `synveda-audit` — the sibling crate owns the chain semantics; this crate
//! owns the one embedded migrator.
//!
//! Schema epoch (CPR-2, ADR-0068 decision 3, ADR-0069): the migrator is
//! guarded on both ends. [`epoch::preflight`] refuses to advance a database
//! written before the context-platform cut, [`epoch::stamp`] records the epoch
//! it produced, and [`epoch::verify`] is what every process asks before it
//! serves. [`reset`] is the one supported way past a refusal, and it destroys
//! rather than translates — there is no migrator from the old model to this
//! one, by decision.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod console_sessions;
pub mod dedup;
pub mod directory;
pub mod directory_sync;
pub mod epoch;
pub mod graph;
pub mod group_mappings;
pub mod hierarchy;
pub mod identities;
pub mod keys;
pub mod lapses;
pub mod observe;
pub mod packs;
pub mod policy_assignments;
pub mod policy_packs;
pub mod promotion;
pub mod prompts;
pub mod quarantine;
pub mod records;
pub mod reset;
pub mod retention;
pub mod rls;
pub mod role_bindings;
pub mod scope_chain;
pub mod scopes;
pub mod search;
pub mod skill_reviews;
pub mod skills;
pub mod tenant_secrets;
pub mod tenants;

pub use scope_chain::ScopeChainCache;

use sqlx::migrate::Migrator;
use sqlx::{PgExecutor, PgPool};
use synveda_types::{Error, Result};

/// The workspace's sqlx migrations, embedded at compile time from
/// `crates/synveda-store/migrations/`.
pub static MIGRATOR: Migrator = sqlx::migrate!();

/// Applies all pending migrations. Idempotent; safe to run concurrently
/// (sqlx serialises runners with an advisory lock).
///
/// Guarded on both ends since CPR-2 (ADR-0069): a database written before the
/// context-platform epoch is refused *before* the migrator touches it, so a
/// refused database is left exactly as it was found, and the epoch marker is
/// stamped after a successful run.
#[tracing::instrument(name = "store.migrate", skip_all, err(Display))]
pub async fn migrate(pool: &PgPool) -> Result<()> {
    migrate_reporting(pool).await.map(|_| ())
}

/// [`migrate`], returning the epoch marker it produced. The reset path prints
/// it; everything else has no use for it and calls `migrate`.
pub async fn migrate_reporting(pool: &PgPool) -> Result<epoch::SchemaMetadata> {
    epoch::preflight(pool)
        .await
        .map_err(|refusal| Error::Storage {
            message: refusal.to_string(),
        })?;
    MIGRATOR.run(pool).await.map_err(|err| Error::Storage {
        message: format!("migration failed: {err}"),
    })?;
    epoch::stamp(pool, env!("CARGO_PKG_VERSION")).await
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
