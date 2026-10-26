//! Postgres storage for the context platform. Stable aggregates keep immutable
//! revisions where history matters; tenant domain tables are protected by
//! enabled and forced RLS.
//!
//! Tenant isolation backstop (ADR-0009): tenant-scoped tables carry
//! forced RLS policies keyed to a transaction-local GUC. Reach them through
//! [`rls::begin_tenant_tx`]; the epoch baseline defines every tenant policy.
//!
//! Audit chain tables (ADR-0019): `audit_log` and `audit_chain_heads`
//! are migrated here with the rest of the schema, but their queries live in
//! `synveda-audit` — the sibling crate owns the chain semantics; this crate
//! owns the one embedded migrator.
//!
//! Governed scopes and their product-level subtypes (ADR-0070;
//! ADR-0071): [`scopes`] is the tree every asset, binding and decision hangs
//! off; [`workspaces`], [`projects`] and [`repositories`] are the product
//! nouns above it, each owning one scope created in the same transaction as
//! itself. [`idempotency`] is what makes retrying a creation safe.
//!
//! Membership and access assignment (ADR-0072): [`access`] is groups,
//! scope grants and invitations, and the resolution that answers "who may act
//! here" — inheritance through `scope_closure`, groups resolved rather than
//! expanded, and a `principal`-shaped scope that inherits nothing. It stores
//! **role keys** and no permission matrix: what a key permits is the policy
//! pack's, and a second mapping here would be a second decision point.
//!
//! Schema epoch (ADR-0068 decision 3, ADR-0069): the migrator is
//! guarded on both ends. [`epoch::preflight`] refuses to advance a database
//! written before the context-platform cut, [`epoch::stamp`] records the epoch
//! it produced, and [`epoch::verify`] is what every process asks before it
//! serves. [`reset`] is the destructive path for pre-cut databases; a
//! published v0.4.3 baseline advances through an append-only migration
//! instead. There is no translator from the old model to this one.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod access;
pub mod anchors;
pub mod artifact_control;
pub mod capture;
pub mod configuration;
pub mod console_sessions;
pub mod context;
pub mod context_dependencies;
pub mod database_url;
pub mod directory;
pub mod directory_sync;
pub mod epoch;
pub mod git_exports;
pub mod idempotency;
pub mod identities;
pub mod imports;
pub mod keys;
pub mod knowledge;
pub mod knowledge_conflicts;
pub mod knowledge_freshness;
pub mod knowledge_lifecycle;
pub mod knowledge_search;
pub mod one_time_login;
pub mod operations;
pub mod packs;
pub mod policy_assignments;
pub mod policy_packs;
pub mod procedure_learning;
pub mod projects;
pub mod prompts;
pub mod quarantine;
pub mod relaxations;
pub mod repositories;
pub mod reset;
pub mod rls;
pub mod runtime_role;
pub mod scopes;
pub mod server_version;
pub mod session_index;
pub mod sessions;
pub mod skills;
pub mod source_control;
pub mod source_reconciliation;
pub mod task_state;
pub mod tenant_archive;
pub mod tenant_erasure;
pub mod tenant_inventory;
pub mod tenant_lifecycle;
pub mod tenant_secrets;
pub mod tenants;
pub mod token_revocations;
pub mod tool_registry;
pub mod vector_patch;
pub mod workspaces;

mod migration;

use std::time::{Duration, Instant};

use migration::MigrationPhase;
pub use migration::{MigrationBounds, MigrationReport};
use sqlx::migrate::{Migrate, Migrator};
use sqlx::{Connection, PgExecutor, PgPool};
use synveda_types::{Error, Result};

/// The published lineage: the byte-pinned v0.4.3 baseline and its forward
/// migrations, embedded from `crates/synveda-store/migrations/`. Only
/// databases already recorded on this lineage advance through it.
pub static MIGRATOR: Migrator = sqlx::migrate!();

/// The portable lineage every fresh installation receives: one PostgreSQL 14
/// compatible baseline equivalent to the published head (ADR-0152), embedded
/// from `crates/synveda-store/portable-migrations/`.
pub static PORTABLE_MIGRATOR: Migrator = sqlx::migrate!("./portable-migrations");

/// Applies all pending migrations under the deployment's explicit database
/// role contract and the default [`MigrationBounds`]. Idempotent; concurrent
/// runners serialize on SQLx's advisory lock.
///
/// Guarded on both ends since the schema epoch guard (ADR-0069): a database written before the
/// context-platform epoch is refused *before* the migrator touches it, so a
/// refused database is left exactly as it was found, and the epoch marker is
/// stamped after a successful run.
#[tracing::instrument(name = "store.migrate", skip_all, err(Display))]
pub async fn migrate(pool: &PgPool, database_roles: &runtime_role::DatabaseRoles) -> Result<()> {
    migrate_bounded(pool, database_roles, MigrationBounds::default())
        .await
        .map(|_| ())
}

/// [`migrate`] under explicit wait limits, reporting each wait.
///
/// Changing an installed schema first waits for every gateway and worker
/// session to leave the database; an idempotent rerun at the current head
/// does not.
#[tracing::instrument(name = "store.migrate.bounded", skip_all, err(Display))]
pub async fn migrate_bounded(
    pool: &PgPool,
    database_roles: &runtime_role::DatabaseRoles,
    bounds: MigrationBounds,
) -> Result<MigrationReport> {
    let connection = pool.acquire().await.map_err(|error| {
        MigrationPhase::Session.error(Error::Storage {
            message: format!("acquire migration connection: {error}"),
        })
    })?;
    migrate_reporting_connection(connection.detach(), database_roles, bounds).await
}

/// Checks whether this candidate build can serve the existing database,
/// without running the SQLx migrator or changing persistent database state.
///
/// The candidate uses the same migrator login as the deployment migration
/// phase so it can prove the complete role, ACL, extension, routine, trigger
/// and forced-RLS catalogue. After session normalization, the compatibility
/// catalogue, marker and ledger proof occurs in one database-enforced
/// read-only, repeatable-read transaction. A mismatch tells the operator to
/// retain the running image; it never emits reset guidance.
#[tracing::instrument(name = "store.migrate.check", skip_all, err(Display))]
pub async fn check_migration_compatibility(
    pool: &PgPool,
    database_roles: &runtime_role::DatabaseRoles,
) -> Result<epoch::SchemaMetadata> {
    let mut connection = pool.acquire().await.map_err(|error| Error::Storage {
        message: format!("acquire candidate compatibility connection: {error}"),
    })?;
    runtime_role::initialize_product_session_connection(&mut connection).await?;
    let mut compatibility = connection
        .begin_with("begin isolation level repeatable read read only")
        .await
        .map_err(|error| Error::Storage {
            message: format!("begin read-only candidate compatibility snapshot: {error}"),
        })?;
    runtime_role::configure_authority_snapshot_connection(&mut compatibility).await?;

    let checked = async {
        let (metadata, stage) = epoch::verify_embedded_head_connection(&mut compatibility)
            .await
            .map_err(|_| Error::Invalid {
                message: "candidate image does not match the existing database schema contract; keep the currently running image; the database was not changed"
                    .to_owned(),
            })?;
        match stage {
            epoch::MigrationPreflight::Prior => {
                runtime_role::verify_v043_migrator_read_only_connection(
                    &mut compatibility,
                    database_roles,
                )
                .await?;
            }
            epoch::MigrationPreflight::Forward => {
                runtime_role::verify_forward_migrator_read_only_connection(
                    &mut compatibility,
                    database_roles,
                )
                .await?;
            }
            epoch::MigrationPreflight::Login => {
                runtime_role::verify_login_migrator_read_only_connection(&mut compatibility, database_roles).await?;
            }
            epoch::MigrationPreflight::Delivery => {
                runtime_role::verify_delivery_migrator_read_only_connection(&mut compatibility,database_roles).await?;
            }
            epoch::MigrationPreflight::Source => {
                runtime_role::verify_source_migrator_read_only_connection(&mut compatibility,database_roles).await?;
            }
            epoch::MigrationPreflight::Artifact => {
                runtime_role::verify_artifact_migrator_read_only_connection(&mut compatibility,database_roles).await?;
            }
            epoch::MigrationPreflight::Reconcile => {
                runtime_role::verify_reconcile_migrator_read_only_connection(&mut compatibility,database_roles).await?;
            }
            epoch::MigrationPreflight::GithubSource => {
                runtime_role::verify_github_source_migrator_read_only_connection(&mut compatibility, database_roles).await?;
            }
            epoch::MigrationPreflight::TaskState=>{runtime_role::verify_task_state_migrator_read_only_connection(&mut compatibility,database_roles).await?;}
            epoch::MigrationPreflight::Procedure => {
                runtime_role::verify_procedure_migrator_read_only_connection(
                    &mut compatibility,
                    database_roles,
                )
                .await?;
            }
            epoch::MigrationPreflight::Convergence | epoch::MigrationPreflight::PortableBaseline => {
                runtime_role::verify_convergence_migrator_read_only_connection(
                    &mut compatibility,
                    database_roles,
                )
                .await?;
            }
            epoch::MigrationPreflight::Admissibility
            | epoch::MigrationPreflight::PortableAdmissibility => {
                runtime_role::verify_admissibility_migrator_read_only_connection(
                    &mut compatibility,
                    database_roles,
                )
                .await?;
            }
            epoch::MigrationPreflight::Revocation | epoch::MigrationPreflight::PortableRevocation => {
                runtime_role::verify_revocation_migrator_read_only_connection(
                    &mut compatibility,
                    database_roles,
                )
                .await?;
            }
            epoch::MigrationPreflight::Restorable | epoch::MigrationPreflight::PortableRestorable => {
                runtime_role::verify_restorable_migrator_read_only_connection(
                    &mut compatibility,
                    database_roles,
                )
                .await?;
            }
            epoch::MigrationPreflight::Current | epoch::MigrationPreflight::Portable => {
                runtime_role::verify_migrator_read_only_connection(
                    &mut compatibility,
                    database_roles,
                )
                .await?;
            }
            _ => {
                return Err(Error::Invalid {
                    message: "candidate database is not at a completely stamped migration head; preserve it and investigate before switching images".to_owned(),
                });
            }
        }
        Ok(metadata)
    }
    .await;

    compatibility
        .rollback()
        .await
        .map_err(|error| Error::Storage {
            message: format!("finish read-only candidate compatibility snapshot: {error}"),
        })?;
    checked
}

/// [`migrate`], returning the epoch marker it produced. The reset path prints
/// it; everything else has no use for it and calls `migrate`.
pub async fn migrate_reporting(
    pool: &PgPool,
    database_roles: &runtime_role::DatabaseRoles,
) -> Result<epoch::SchemaMetadata> {
    migrate_bounded(pool, database_roles, MigrationBounds::default())
        .await
        .map(|report| report.metadata)
}

/// Connection-owned migration path used when a caller has already proved the
/// exact physical target. Keeping the proof and every migration effect on one
/// connection prevents a routing proxy from changing clusters on pool
/// reacquisition.
pub(crate) async fn migrate_reporting_connection(
    mut connection: sqlx::PgConnection,
    database_roles: &runtime_role::DatabaseRoles,
    bounds: MigrationBounds,
) -> Result<MigrationReport> {
    // SQLx's lock is session-scoped and reentrant. Its inner run releases only
    // its own acquisition; this outer acquisition also covers the preflight
    // and epoch stamp. Own (never return to a pool) this physical connection so
    // cancellation or an inner migration error cannot strand advisory locks.
    // Reset passes its already-proved target connection through this same seam.
    let started = Instant::now();
    let result = async {
        runtime_role::initialize_product_session_connection(&mut connection)
            .await
            .map_err(|error| MigrationPhase::Session.error(error))?;
        migration::bound_lock_wait(
            &mut connection,
            MigrationPhase::BoundaryLock,
            bounds.boundary_lock(),
        )
        .await?;
        let waiting = Instant::now();
        Migrate::lock(&mut connection)
            .await
            .map_err(|error| migration::boundary_lock_failure(error, bounds))?;
        let boundary_wait = waiting.elapsed();
        let mut report = migrate_locked_connection(&mut connection, database_roles, bounds).await?;
        report.boundary_wait = boundary_wait;
        report.total = started.elapsed();
        Ok(report)
    }
    .await;
    let closed = connection.close().await.map_err(|error| {
        MigrationPhase::Session.error(Error::Storage {
            message: format!("close migration boundary connection: {error}"),
        })
    });
    let report = result?;
    closed?;
    Ok(report)
}

async fn verify_pre_migration_authority(
    connection: &mut sqlx::PgConnection,
    preflight: epoch::MigrationPreflight,
    database_roles: &runtime_role::DatabaseRoles,
) -> Result<()> {
    let mut authority = connection.begin().await.map_err(|error| Error::Storage {
        message: format!("begin pre-migration authority snapshot: {error}"),
    })?;
    runtime_role::configure_authority_snapshot_connection(&mut authority).await?;
    verify_migration_prerequisites_connection(&mut authority, database_roles).await?;
    verify_stage_migrator_connection(&mut authority, preflight, database_roles).await?;
    authority.commit().await.map_err(|error| Error::Storage {
        message: format!("finish pre-migration authority snapshot: {error}"),
    })
}

async fn verify_stage_migrator_connection(
    authority: &mut sqlx::PgConnection,
    preflight: epoch::MigrationPreflight,
    database_roles: &runtime_role::DatabaseRoles,
) -> Result<()> {
    use epoch::MigrationPreflight as Stage;
    match preflight {
        Stage::Clean => {
            runtime_role::verify_migration_extension_prerequisites_connection(
                authority,
                database_roles,
            )
            .await?;
        }
        Stage::PendingStampPrior | Stage::Prior => {
            runtime_role::verify_v043_migrator_connection(authority, database_roles).await?;
        }
        Stage::PendingStampForward | Stage::Forward => {
            runtime_role::verify_forward_migrator_connection(authority, database_roles).await?;
        }
        Stage::PendingStampLogin | Stage::Login => {
            runtime_role::verify_login_migrator_connection(authority, database_roles).await?;
        }
        Stage::PendingStampDelivery | Stage::Delivery => {
            runtime_role::verify_delivery_migrator_connection(authority, database_roles).await?;
        }
        Stage::PendingStampSource | Stage::Source => {
            runtime_role::verify_source_migrator_connection(authority, database_roles).await?;
        }
        Stage::PendingStampArtifact | Stage::Artifact => {
            runtime_role::verify_artifact_migrator_connection(authority, database_roles).await?;
        }
        Stage::PendingStampReconcile | Stage::Reconcile => {
            runtime_role::verify_reconcile_migrator_connection(authority, database_roles).await?;
        }
        Stage::PendingStampGithubSource | Stage::GithubSource => {
            runtime_role::verify_github_source_migrator_connection(authority, database_roles)
                .await?;
        }
        Stage::PendingStampTaskState | Stage::TaskState => {
            runtime_role::verify_task_state_migrator_connection(authority, database_roles).await?;
        }
        Stage::PendingStampProcedure | Stage::Procedure => {
            runtime_role::verify_procedure_migrator_connection(authority, database_roles).await?;
        }
        Stage::PendingStampConvergence
        | Stage::Convergence
        | Stage::PendingStampPortableBaseline
        | Stage::PortableBaseline => {
            runtime_role::verify_convergence_migrator_connection(authority, database_roles).await?;
        }
        Stage::PendingStampAdmissibility
        | Stage::Admissibility
        | Stage::PendingStampPortableAdmissibility
        | Stage::PortableAdmissibility => {
            runtime_role::verify_admissibility_migrator_connection(authority, database_roles)
                .await?;
        }
        Stage::PendingStampRevocation
        | Stage::Revocation
        | Stage::PendingStampPortableRevocation
        | Stage::PortableRevocation => {
            runtime_role::verify_revocation_migrator_connection(authority, database_roles).await?;
        }
        Stage::PendingStampRestorable
        | Stage::Restorable
        | Stage::PendingStampPortableRestorable
        | Stage::PortableRestorable => {
            runtime_role::verify_restorable_migrator_connection(authority, database_roles).await?;
        }
        Stage::PendingStampCurrent
        | Stage::Current
        | Stage::PendingStampPortable
        | Stage::Portable => {
            if matches!(preflight, Stage::Current | Stage::Portable) {
                epoch::verify_connection(authority)
                    .await
                    .map_err(|error| Error::Invalid {
                        message: error.to_string(),
                    })?;
            }
            runtime_role::verify_migrator_connection(authority, database_roles).await?;
        }
    }
    Ok(())
}

async fn migrate_locked_connection(
    connection: &mut sqlx::PgConnection,
    database_roles: &runtime_role::DatabaseRoles,
    bounds: MigrationBounds,
) -> Result<MigrationReport> {
    let preflight = epoch::migration_preflight_connection(&mut *connection)
        .await
        .map_err(|refusal| {
            MigrationPhase::Preflight.error(Error::Storage {
                message: refusal.to_string(),
            })
        })?;
    let starting_head = match epoch::read_connection(&mut *connection).await {
        Ok(marker) => Some(marker.migration_head),
        Err(epoch::SchemaEpochError::Missing | epoch::SchemaEpochError::Unstamped) => None,
        Err(refusal) => {
            return Err(MigrationPhase::Preflight.error(Error::Storage {
                message: refusal.to_string(),
            }));
        }
    };
    let writer_drain = if migration::changes_installed_schema(preflight) {
        migration::drain_runtime_writers(connection, database_roles, bounds.writer_drain()).await?
    } else {
        Duration::ZERO
    };
    verify_pre_migration_authority(connection, preflight, database_roles)
        .await
        .map_err(|error| MigrationPhase::Authority.error(error))?;
    migration::bound_lock_wait(connection, MigrationPhase::SchemaChange, bounds.ddl_lock()).await?;
    let changing = Instant::now();
    lineage_migrator(preflight)
        .run_direct(&mut *connection)
        .await
        .map_err(|error| migration::schema_change_failure(error, bounds))?;
    let schema_change = changing.elapsed();
    let metadata = stamp_verified_head(connection, database_roles)
        .await
        .map_err(|error| MigrationPhase::Stamp.error(error))?;
    tracing::info!(
        starting_head = starting_head.as_deref().unwrap_or("none"),
        head = %metadata.migration_head,
        writer_drain_ms = writer_drain.as_millis(),
        schema_change_ms = schema_change.as_millis(),
        "migration boundary complete"
    );
    Ok(MigrationReport {
        starting_head,
        metadata,
        boundary_wait: Duration::ZERO,
        writer_drain,
        schema_change,
        total: Duration::ZERO,
    })
}

async fn stamp_verified_head(
    connection: &mut sqlx::PgConnection,
    database_roles: &runtime_role::DatabaseRoles,
) -> Result<epoch::SchemaMetadata> {
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

/// Fresh installations take the portable lineage; a database already on the
/// published lineage keeps advancing on it so its SQLx ledger stays truthful.
fn lineage_migrator(preflight: epoch::MigrationPreflight) -> &'static Migrator {
    match preflight {
        epoch::MigrationPreflight::Clean
        | epoch::MigrationPreflight::PendingStampPortableBaseline
        | epoch::MigrationPreflight::PortableBaseline
        | epoch::MigrationPreflight::PendingStampPortableAdmissibility
        | epoch::MigrationPreflight::PortableAdmissibility
        | epoch::MigrationPreflight::PendingStampPortableRevocation
        | epoch::MigrationPreflight::PortableRevocation
        | epoch::MigrationPreflight::PendingStampPortableRestorable
        | epoch::MigrationPreflight::PortableRestorable
        | epoch::MigrationPreflight::PendingStampPortable
        | epoch::MigrationPreflight::Portable => &PORTABLE_MIGRATOR,
        _ => &MIGRATOR,
    }
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
/// readiness path (ADR-0007); reads no application data.
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
