//! Postgres storage for the context platform. Stable aggregates keep immutable
//! revisions where history matters; tenant domain tables are protected by
//! enabled and forced RLS.
//!
//! Tenant isolation backstop (TEN-2, ADR-0009): tenant-scoped tables carry
//! forced RLS policies keyed to a transaction-local GUC. Reach them through
//! [`rls::begin_tenant_tx`]; the epoch baseline defines every tenant policy.
//!
//! Audit chain tables (AUD-1, ADR-0019): `audit_log` and `audit_chain_heads`
//! are migrated here with the rest of the schema, but their queries live in
//! `synveda-audit` — the sibling crate owns the chain semantics; this crate
//! owns the one embedded migrator.
//!
//! Governed scopes and their product-level subtypes (CPR-3 ADR-0070; CPR-4
//! ADR-0071): [`scopes`] is the tree every asset, binding and decision hangs
//! off; [`workspaces`], [`projects`] and [`repositories`] are the product
//! nouns above it, each owning one scope created in the same transaction as
//! itself. [`idempotency`] is what makes retrying a creation safe.
//!
//! Membership and access assignment (CPR-5, ADR-0072): [`access`] is groups,
//! scope grants and invitations, and the resolution that answers "who may act
//! here" — inheritance through `scope_closure`, groups resolved rather than
//! expanded, and a `principal`-shaped scope that inherits nothing. It stores
//! **role keys** and no permission matrix: what a key permits is the policy
//! pack's, and a second mapping here would be a second decision point.
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

pub mod access;
pub mod anchors;
pub mod capture;
pub mod configuration;
pub mod console_sessions;
pub mod context;
pub mod database_url;
pub mod directory;
pub mod directory_sync;
pub mod epoch;
pub mod idempotency;
pub mod identities;
pub mod imports;
pub mod keys;
pub mod knowledge;
pub mod knowledge_conflicts;
pub mod knowledge_freshness;
pub mod knowledge_lifecycle;
pub mod knowledge_search;
pub mod packs;
pub mod policy_assignments;
pub mod policy_packs;
pub mod projects;
pub mod prompts;
pub mod quarantine;
pub mod relaxations;
pub mod repositories;
pub mod reset;
pub mod rls;
pub mod runtime_role;
pub mod scopes;
pub mod sessions;
pub mod skills;
pub mod tenant_secrets;
pub mod tenants;
pub mod tool_registry;
pub mod workspaces;

use sqlx::migrate::Migrator;
use sqlx::{Connection, PgExecutor, PgPool};
use synveda_types::{Error, Result};

/// The workspace's sqlx migrations, embedded at compile time from
/// `crates/synveda-store/migrations/`.
pub static MIGRATOR: Migrator = sqlx::migrate!();

/// Applies all pending migrations under the deployment's explicit database
/// role contract. Idempotent; safe to run concurrently (sqlx serialises
/// runners with an advisory lock).
///
/// Guarded on both ends since CPR-2 (ADR-0069): a database written before the
/// context-platform epoch is refused *before* the migrator touches it, so a
/// refused database is left exactly as it was found, and the epoch marker is
/// stamped after a successful run.
#[tracing::instrument(name = "store.migrate", skip_all, err(Display))]
pub async fn migrate(pool: &PgPool, database_roles: &runtime_role::DatabaseRoles) -> Result<()> {
    migrate_reporting(pool, database_roles).await.map(|_| ())
}

/// [`migrate`], returning the epoch marker it produced. The reset path prints
/// it; everything else has no use for it and calls `migrate`.
pub async fn migrate_reporting(
    pool: &PgPool,
    database_roles: &runtime_role::DatabaseRoles,
) -> Result<epoch::SchemaMetadata> {
    let mut connection = pool.acquire().await.map_err(|error| Error::Storage {
        message: format!("acquire migration connection: {error}"),
    })?;
    migrate_reporting_connection(&mut connection, database_roles).await
}

/// Connection-owned migration path used when a caller has already proved the
/// exact physical target. Keeping the proof and every migration effect on one
/// connection prevents a routing proxy from changing clusters on pool
/// reacquisition.
pub(crate) async fn migrate_reporting_connection(
    connection: &mut sqlx::PgConnection,
    database_roles: &runtime_role::DatabaseRoles,
) -> Result<epoch::SchemaMetadata> {
    let preflight = epoch::migration_preflight_connection(&mut *connection)
        .await
        .map_err(|refusal| Error::Storage {
            message: refusal.to_string(),
        })?;
    runtime_role::initialize_product_session_connection(&mut *connection).await?;
    {
        let mut authority = connection.begin().await.map_err(|error| Error::Storage {
            message: format!("begin pre-migration authority snapshot: {error}"),
        })?;
        runtime_role::configure_authority_snapshot_connection(&mut authority).await?;
        verify_migration_prerequisites_connection(&mut authority, database_roles).await?;
        match preflight {
            epoch::MigrationPreflight::Clean => {
                runtime_role::verify_migration_extension_prerequisites_connection(
                    &mut authority,
                    database_roles,
                )
                .await?;
            }
            epoch::MigrationPreflight::PendingStamp => {
                runtime_role::verify_migrator_connection(&mut authority, database_roles).await?;
            }
            epoch::MigrationPreflight::Current => {
                epoch::verify_connection(&mut authority)
                    .await
                    .map_err(|error| Error::Invalid {
                        message: error.to_string(),
                    })?;
                runtime_role::verify_migrator_connection(&mut authority, database_roles).await?;
            }
        }
        authority.commit().await.map_err(|error| Error::Storage {
            message: format!("finish pre-migration authority snapshot: {error}"),
        })?;
    }
    MIGRATOR
        .run(&mut *connection)
        .await
        .map_err(|err| Error::Storage {
            message: format!("migration failed: {err}"),
        })?;
    let mut authority = connection.begin().await.map_err(|error| Error::Storage {
        message: format!("begin post-migration authority snapshot: {error}"),
    })?;
    runtime_role::configure_authority_snapshot_connection(&mut authority).await?;
    runtime_role::verify_migrator_connection(&mut authority, database_roles).await?;
    let metadata = epoch::stamp_connection(&mut authority, env!("CARGO_PKG_VERSION")).await?;
    authority.commit().await.map_err(|error| Error::Storage {
        message: format!("finish post-migration authority snapshot: {error}"),
    })?;
    Ok(metadata)
}

/// Refuses a migration before SQLx begins when deployment infrastructure has
/// not installed the exact capability role and extension prerequisites. The
/// baseline deliberately owns neither CREATEROLE nor extension authority.
async fn verify_migration_prerequisites_connection(
    connection: &mut sqlx::PgConnection,
    database_roles: &runtime_role::DatabaseRoles,
) -> Result<()> {
    runtime_role::verify_migrator_prerequisites_connection(connection, database_roles)
        .await
        .map(|_| ())
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
