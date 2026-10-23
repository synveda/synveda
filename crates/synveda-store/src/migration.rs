//! Bounds, phases and the writer drain around one migration boundary.

use std::time::{Duration, Instant};

use sqlx::PgConnection;
use sqlx::migrate::MigrateError;
use synveda_types::{Error, Result};

use crate::epoch::{MigrationPreflight, SchemaMetadata};
use crate::runtime_role::DatabaseRoles;

// Kept equal to the server-side pause between session counts.
const WRITER_DRAIN_POLL: Duration = Duration::from_millis(500);
const LOCK_NOT_AVAILABLE: &str = "55P03";

/// Time limits for one migration run. Every limit is refused outside
/// one second to [`MigrationBounds::MAXIMUM`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MigrationBounds {
    boundary_lock: Duration,
    ddl_lock: Duration,
    writer_drain: Duration,
}

impl MigrationBounds {
    /// The largest accepted limit for any phase.
    pub const MAXIMUM: Duration = Duration::from_secs(600);

    /// Builds the limits for waiting on another migrator's boundary, on a
    /// conflicting table lock inside a migration, and for the previous
    /// release's gateway and worker sessions to disconnect.
    pub fn new(
        boundary_lock: Duration,
        ddl_lock: Duration,
        writer_drain: Duration,
    ) -> Result<Self> {
        for (name, bound) in [
            ("boundary lock wait", boundary_lock),
            ("schema-change lock wait", ddl_lock),
            ("writer drain wait", writer_drain),
        ] {
            if bound < Duration::from_secs(1) || bound > Self::MAXIMUM {
                return Err(Error::Invalid {
                    message: format!(
                        "migration {name} must be between 1 and {} seconds",
                        Self::MAXIMUM.as_secs()
                    ),
                });
            }
        }
        Ok(Self {
            boundary_lock,
            ddl_lock,
            writer_drain,
        })
    }

    pub(crate) fn boundary_lock(self) -> Duration {
        self.boundary_lock
    }

    pub(crate) fn ddl_lock(self) -> Duration {
        self.ddl_lock
    }

    pub(crate) fn writer_drain(self) -> Duration {
        self.writer_drain
    }

    /// The default limits with a deployment-specific writer drain, sized to
    /// cover the gateway and worker termination grace.
    pub fn with_writer_drain(writer_drain: Duration) -> Result<Self> {
        let default = Self::default();
        Self::new(default.boundary_lock, default.ddl_lock, writer_drain)
    }
}

impl Default for MigrationBounds {
    /// A short schema-change lock wait keeps a blocked `ACCESS EXCLUSIVE`
    /// request from queueing every later reader behind it.
    fn default() -> Self {
        Self {
            boundary_lock: Duration::from_secs(60),
            ddl_lock: Duration::from_secs(10),
            writer_drain: Duration::from_secs(120),
        }
    }
}

/// What one completed migration run did and how long each wait took.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationReport {
    /// The marker head before this run; `None` for a fresh database.
    pub starting_head: Option<String>,
    /// The marker this run stamped.
    pub metadata: SchemaMetadata,
    /// Time spent waiting for another migrator's boundary.
    pub boundary_wait: Duration,
    /// Time spent waiting for gateway and worker sessions to disconnect.
    pub writer_drain: Duration,
    /// Time spent applying SQLx migrations.
    pub schema_change: Duration,
    /// Time from connection to the committed stamp.
    pub total: Duration,
}

/// The step a migration failure belongs to, named in every error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MigrationPhase {
    Session,
    BoundaryLock,
    Preflight,
    WriterDrain,
    Authority,
    SchemaChange,
    Stamp,
}

impl MigrationPhase {
    fn label(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::BoundaryLock => "boundary-lock",
            Self::Preflight => "preflight",
            Self::WriterDrain => "writer-drain",
            Self::Authority => "authority",
            Self::SchemaChange => "schema-change",
            Self::Stamp => "stamp",
        }
    }

    /// Prefixes the phase onto a store error without changing its kind.
    pub(crate) fn error(self, error: Error) -> Error {
        let prefix = |message: String| format!("migration phase {}: {message}", self.label());
        match error {
            Error::Storage { message } => Error::Storage {
                message: prefix(message),
            },
            Error::Invalid { message } => Error::Invalid {
                message: prefix(message),
            },
            other => other,
        }
    }

    fn storage(self, message: String) -> Error {
        self.error(Error::Storage { message })
    }
}

/// An already-stamped, unchanged head needs no drain: the idempotent rerun on
/// every start must succeed beside the running gateway and worker.
pub(crate) fn changes_installed_schema(preflight: MigrationPreflight) -> bool {
    !matches!(
        preflight,
        MigrationPreflight::Clean | MigrationPreflight::Current | MigrationPreflight::Portable
    )
}

/// Bounds every later lock wait on this session, including SQLx's advisory
/// boundary lock, so a blocked wait ends server-side with `55P03`.
pub(crate) async fn bound_lock_wait(
    connection: &mut PgConnection,
    phase: MigrationPhase,
    bound: Duration,
) -> Result<()> {
    let setting = format!("{}ms", bound.as_millis());
    sqlx::query_scalar!(
        r#"select pg_catalog.set_config('lock_timeout', $1, false) as "lock_timeout!""#,
        setting,
    )
    .fetch_one(&mut *connection)
    .await
    .map(|_| ())
    .map_err(|error| phase.storage(format!("bound migration lock wait: {}", described(&error))))
}

pub(crate) fn boundary_lock_failure(error: MigrateError, bounds: MigrationBounds) -> Error {
    let message = match &error {
        MigrateError::Execute(source) if is_lock_timeout(source) => format!(
            "another migration held the deployment migration boundary for more than {} seconds (SQLSTATE {LOCK_NOT_AVAILABLE}); let it finish, then rerun; the database was not changed",
            bounds.boundary_lock.as_secs()
        ),
        MigrateError::Execute(source) => {
            format!("acquire migration boundary lock: {}", described(source))
        }
        other => format!("acquire migration boundary lock: {other}"),
    };
    MigrationPhase::BoundaryLock.storage(message)
}

pub(crate) fn schema_change_failure(error: MigrateError, bounds: MigrationBounds) -> Error {
    let message = match &error {
        MigrateError::ExecuteMigration(source, version) if is_lock_timeout(source) => {
            format!(
                "migration {version:04} waited more than {} seconds for a table lock held by another session (SQLSTATE {LOCK_NOT_AVAILABLE}) and rolled back; stop the blocking session, such as a backup or long query, then rerun",
                bounds.ddl_lock.as_secs()
            )
        }
        MigrateError::ExecuteMigration(source, version) => {
            format!(
                "migration {version:04} failed and rolled back: {}",
                described(source)
            )
        }
        MigrateError::Execute(source) => format!("migration failed: {}", described(source)),
        other => format!("migration failed: {other}"),
    };
    MigrationPhase::SchemaChange.storage(message)
}

/// Waits, within the bound, until no session of the deployment's gateway or
/// worker login remains in this database. Release-to-release DDL must not run
/// under a writer whose binary predates it: until the stamp moves, that
/// binary's own epoch check still accepts the database.
pub(crate) async fn drain_runtime_writers(
    connection: &mut PgConnection,
    roles: &DatabaseRoles,
    bound: Duration,
) -> Result<Duration> {
    let started = Instant::now();
    let logins = [roles.gateway().to_owned(), roles.worker().to_owned()];
    let polls = bound.as_millis() / WRITER_DRAIN_POLL.as_millis() + 1;
    let mut sessions = 0;
    for poll in 0..polls {
        sessions = runtime_writer_sessions(connection, &logins).await?;
        if sessions == 0 {
            return Ok(started.elapsed());
        }
        if poll == 0 {
            tracing::warn!(
                phase = MigrationPhase::WriterDrain.label(),
                sessions,
                bound_secs = bound.as_secs(),
                "waiting for gateway and worker database sessions to close"
            );
        }
        if started.elapsed() >= bound {
            break;
        }
        pause_between_polls(connection).await?;
    }
    Err(MigrationPhase::WriterDrain.storage(format!(
        "{sessions} gateway or worker database session(s) stayed connected for {} seconds; stop every gateway and worker process before changing an installed schema, then rerun; the database was not changed",
        bound.as_secs()
    )))
}

/// The store has no async runtime of its own; the server bounds the pause.
async fn pause_between_polls(connection: &mut PgConnection) -> Result<()> {
    sqlx::query!("select pg_catalog.pg_sleep(0.5)")
        .execute(&mut *connection)
        .await
        .map(|_| ())
        .map_err(|error| {
            MigrationPhase::WriterDrain.storage(format!(
                "pause between session counts: {}",
                described(&error)
            ))
        })
}

/// `usename` and `datname` are visible for every session without
/// `pg_read_all_stats`; each autocommit statement reads a fresh snapshot.
async fn runtime_writer_sessions(connection: &mut PgConnection, logins: &[String]) -> Result<i64> {
    sqlx::query_scalar!(
        r#"select count(*) as "sessions!"
             from pg_catalog.pg_stat_activity as activity
            where activity.datname = pg_catalog.current_database()
              and activity.pid <> pg_catalog.pg_backend_pid()
              and activity.usename::text = any($1::text[])"#,
        logins,
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| {
        MigrationPhase::WriterDrain.storage(format!(
            "count gateway and worker database sessions: {}",
            described(&error)
        ))
    })
}

fn is_lock_timeout(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(database)
        if database.code().as_deref() == Some(LOCK_NOT_AVAILABLE))
}

/// SQLSTATE and server message only; driver errors never carry bound values.
fn described(error: &sqlx::Error) -> String {
    match error {
        sqlx::Error::Database(database) => format!(
            "{} (SQLSTATE {})",
            database.message(),
            database.code().as_deref().unwrap_or("unknown")
        ),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_refuse_zero_and_oversized_limits() {
        let second = Duration::from_secs(1);
        assert!(MigrationBounds::new(second, second, second).is_ok());
        for bounds in [
            (Duration::ZERO, second, second),
            (second, Duration::from_millis(999), second),
            (second, second, MigrationBounds::MAXIMUM + second),
        ] {
            assert!(MigrationBounds::new(bounds.0, bounds.1, bounds.2).is_err());
        }
        assert_eq!(
            MigrationBounds::with_writer_drain(Duration::from_secs(95))
                .expect("a sized drain")
                .writer_drain,
            Duration::from_secs(95)
        );
    }

    #[test]
    fn phases_prefix_storage_and_invalid_errors_without_changing_their_kind() {
        let storage = MigrationPhase::WriterDrain.error(Error::Storage {
            message: "two sessions".to_owned(),
        });
        assert!(matches!(
            storage,
            Error::Storage { ref message } if message == "migration phase writer-drain: two sessions"
        ));
        let invalid = MigrationPhase::Authority.error(Error::Invalid {
            message: "drift".to_owned(),
        });
        assert!(matches!(
            invalid,
            Error::Invalid { ref message } if message == "migration phase authority: drift"
        ));
    }

    #[test]
    fn only_a_stamped_unchanged_head_skips_the_writer_drain() {
        for unchanged in [
            MigrationPreflight::Clean,
            MigrationPreflight::Current,
            MigrationPreflight::Portable,
        ] {
            assert!(!changes_installed_schema(unchanged));
        }
        for changed in [
            MigrationPreflight::Procedure,
            MigrationPreflight::Convergence,
            MigrationPreflight::PortableBaseline,
            MigrationPreflight::PendingStampConvergence,
            MigrationPreflight::PendingStampCurrent,
            MigrationPreflight::PendingStampPortable,
            MigrationPreflight::Prior,
        ] {
            assert!(changes_installed_schema(changed));
        }
    }
}
