//! The schema epoch: what makes a pre-1.0 hard cut a refusal rather than a
//! silent half-upgrade (CPR-2, ADR-0068 decision 3, ADR-0069).
//!
//! Synveda's context-platform redesign replaces the model underneath every
//! table. There is no translator from the old shape to the new one, by
//! decision rather than by omission — mapping five organisational ranks onto
//! an unranked tree, and re-deciding the derived/published boundary per row,
//! would put both judgements in a script nobody reviews as carefully as a
//! policy, bought with nothing pre-1.0 (ADR-0068, option 3). So a database
//! from before the cut has exactly one supported path forward, and it is
//! destruction.
//!
//! Three surfaces make that true rather than documented:
//!
//! * [`preflight`] refuses to migrate a database that has a schema but no
//!   epoch marker. That is the pre-cut database, and it is refused *before*
//!   the migrator touches it, so a refused database is left exactly as it was.
//! * [`stamp`] writes the marker after a successful migration, recording the
//!   epoch, the head reached, the moment, and the product version that minted
//!   it.
//! * [`verify`] is the startup guard. The gateway refuses to serve a database
//!   at any epoch but the one this build was written for, and `/readyz` asks
//!   the same question on every probe so a database that arrives late cannot
//!   slip past a check that ran while it was down.
//!
//! Every refusal prints [`RESET_COMMAND`] verbatim. There is one copy of that
//! string, here, because a reset instruction that names a command which does
//! not exist is worse than no instruction at all.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

/// The schema epoch this build serves.
///
/// Epoch 1 is the context platform's founding marker; epoch 2 is the
/// hierarchy cutover (CPR-7, ADR-0074) — the chain was rewritten in place
/// (the scope substrate moved to `0004`, `role_bindings` and the hierarchy
/// tables left it), so a database at epoch 1 holds a schema this build can
/// neither read nor migrate, and is refused with the reset instruction.
/// Everything before epoch 1 — the 38-migration enterprise-memory schema
/// this programme cuts from — carries no marker at all, which is how a
/// pre-cut database presents to [`verify`]: not as epoch 0, but as
/// [`SchemaEpochError::Missing`].
///
/// Bump this only when the model underneath changes incompatibly. Adding a
/// migration is not that; every ordinary release leaves this number alone.
pub const CURRENT_EPOCH: i32 = 2;

/// The exact command that makes a refused database usable again. Quoted
/// verbatim by every refusal below, and by the gateway, the CLI and the
/// installation documentation — one string, so a message cannot name a verb
/// the binary does not have.
pub const RESET_COMMAND: &str = "synveda reset --database --force";

/// The epoch marker as the database holds it (`schema_metadata`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaMetadata {
    /// The schema epoch. Compared against [`CURRENT_EPOCH`] by [`verify`].
    pub epoch: i32,
    /// The migration head reached, as its four-digit file prefix (`0039`).
    /// Diagnostic: it tells two databases at the same epoch apart.
    pub migration_head: String,
    /// When this database became a Synveda database.
    pub created_at: DateTime<Utc>,
    /// The product version that created the epoch. Never rewritten.
    pub created_by_version: String,
    /// When the migration head last moved.
    pub updated_at: DateTime<Utc>,
}

/// Why a database is not one this build may serve.
///
/// Every variant but [`Unreachable`](Self::Unreachable) is a refusal: the
/// database is reachable and is not at this epoch. `Unreachable` is a
/// don't-know, and callers treat it as such — the gateway boots without a
/// database on purpose so `/readyz` can report an outage instead of the
/// process crash-looping (ADR-0007), and an outage must not be reported as a
/// wrong epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaEpochError {
    /// The database could not be asked. Not a verdict about the epoch.
    Unreachable(String),
    /// There is no marker: either no `schema_metadata` table, or no row in
    /// it. This is what a database from before the cut looks like.
    Missing,
    /// The marker exists but is not the shape this build reads.
    Malformed(String),
    /// The marker names an epoch this build has moved past.
    Older {
        /// The epoch the database carries.
        found: i32,
    },
    /// The marker names an epoch newer than this build serves — the
    /// installation is behind, and a reset would destroy readable data.
    Newer {
        /// The epoch the database carries.
        found: i32,
    },
}

impl SchemaEpochError {
    /// Whether this is a verdict about the epoch rather than an outage.
    ///
    /// The distinction is load-bearing at exactly one seam: the gateway
    /// refuses to boot on a refusal and boots anyway on an outage.
    #[must_use]
    pub fn is_refusal(&self) -> bool {
        !matches!(self, Self::Unreachable(_))
    }
}

/// The paragraph every refusal ends with. One copy, because it names a
/// command and destroys a database.
const HARD_RESET_ADVICE: &str = "\
Synveda is pre-1.0 and the context-platform redesign is a hard cut: there is
no migration from the previous schema, no compatibility path, and nothing that
translates old rows into the new model. A database from before the cut is
refused rather than upgraded.

Reset it — this DESTROYS everything in that database:

    synveda reset --database --force";

impl std::fmt::Display for SchemaEpochError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable(detail) => {
                write!(f, "could not read the schema epoch marker: {detail}")
            }
            Self::Missing => write!(
                f,
                "this database carries no Synveda schema epoch marker, so it \
                 was written before the context platform (epoch {CURRENT_EPOCH}).\n\n{HARD_RESET_ADVICE}"
            ),
            Self::Malformed(detail) => write!(
                f,
                "this database's schema epoch marker cannot be read ({detail}), \
                 so nothing here can tell which model its rows are in.\n\n{HARD_RESET_ADVICE}"
            ),
            Self::Older { found } => write!(
                f,
                "this database is at schema epoch {found}; this build serves \
                 epoch {CURRENT_EPOCH}.\n\n{HARD_RESET_ADVICE}"
            ),
            Self::Newer { found } => write!(
                f,
                "this database is at schema epoch {found} and this build serves \
                 epoch {CURRENT_EPOCH}, so this installation is behind the \
                 database.\n\nUpgrade this installation rather than resetting. \
                 The database holds data a newer Synveda\ncan read, and \
                 `{RESET_COMMAND}` would destroy it."
            ),
        }
    }
}

impl std::error::Error for SchemaEpochError {}

/// Reads the marker, without judging it.
///
/// The error mapping is the interesting part. A missing *table* and a missing
/// *row* are the same fact — there is no marker — and both are
/// [`SchemaEpochError::Missing`]. A table that exists in some other shape is
/// [`SchemaEpochError::Malformed`], because "somebody else's `schema_metadata`"
/// and "ours, corrupted" are indistinguishable from here and neither is safe
/// to serve. Everything else is an outage.
pub async fn read(pool: &PgPool) -> Result<SchemaMetadata, SchemaEpochError> {
    let row = sqlx::query!(
        r#"
        select epoch              as "epoch!",
               migration_head     as "migration_head!",
               created_at         as "created_at!",
               created_by_version as "created_by_version!",
               updated_at         as "updated_at!"
        from schema_metadata
        "#
    )
    .fetch_optional(pool)
    .await
    .map_err(classify)?
    .ok_or(SchemaEpochError::Missing)?;

    // Validated here as well as by the CHECK constraints, because the
    // constraints only bind a table migration 0039 created. A marker that
    // arrived some other way reaches exactly this code.
    //
    // The epoch itself is deliberately *not* range-checked here. Any integer
    // is a legible epoch — smaller is older, larger is newer — and
    // [`verify`] is where that comparison belongs. A guard that called a low
    // number malformed would report the ordinary case (a database this build
    // has moved past) as corruption, and send its operator looking for the
    // wrong problem.
    if row.migration_head.trim().is_empty() || row.created_by_version.trim().is_empty() {
        return Err(SchemaEpochError::Malformed(
            "the migration head or the creating version is blank".to_owned(),
        ));
    }

    Ok(SchemaMetadata {
        epoch: row.epoch,
        migration_head: row.migration_head,
        created_at: row.created_at,
        created_by_version: row.created_by_version,
        updated_at: row.updated_at,
    })
}

/// The startup guard: reads the marker and accepts only [`CURRENT_EPOCH`].
///
/// Called by the gateway before it serves anything, by `/readyz` on every
/// probe, and by every CLI command that opens a database directly. The one
/// path that does not call it is the one that creates the epoch —
/// [`crate::migrate`] — which is [`preflight`]'s job instead.
#[tracing::instrument(name = "store.epoch.verify", skip_all)]
pub async fn verify(pool: &PgPool) -> Result<SchemaMetadata, SchemaEpochError> {
    let metadata = read(pool).await?;
    match metadata.epoch {
        epoch if epoch == CURRENT_EPOCH => Ok(metadata),
        found if found < CURRENT_EPOCH => Err(SchemaEpochError::Older { found }),
        found => Err(SchemaEpochError::Newer { found }),
    }
}

/// Refuses to migrate a database that has a schema but no epoch marker.
///
/// Run before the migrator, so a refused database is left byte for byte as it
/// was found rather than half-advanced.
///
/// The test is deliberately not a list of table names: "any table in `public`
/// that is neither the marker nor sqlx's own bookkeeping" survives every
/// deletion the rest of this programme performs, where a sentinel like
/// `tenants` or `records` would become a check against a table nobody has any
/// more. A database holding only `_sqlx_migrations` is a migration that
/// failed on its first run, not a pre-cut database, and is allowed through.
#[tracing::instrument(name = "store.epoch.preflight", skip_all)]
pub async fn preflight(pool: &PgPool) -> Result<(), SchemaEpochError> {
    let row = sqlx::query!(
        r#"
        select exists (
                   select
                   from pg_class c
                   join pg_namespace n on n.oid = c.relnamespace
                   where n.nspname = 'public'
                     and c.relkind = 'r'
                     and c.relname not in ('schema_metadata', '_sqlx_migrations')
               ) as "has_tables!",
               to_regclass('public.schema_metadata') is not null as "has_marker!"
        "#
    )
    .fetch_one(pool)
    .await
    .map_err(classify)?;

    if row.has_tables && !row.has_marker {
        return Err(SchemaEpochError::Missing);
    }
    Ok(())
}

/// Writes the marker after a successful migration.
///
/// Idempotent, and asymmetric on purpose: `migration_head` and `updated_at`
/// move on every run, `created_at` and `created_by_version` are written once
/// and never again. "Which release minted this database" is a fact about the
/// past; a column that tracked the current binary would answer a question
/// nobody asked and lose the one that was.
#[tracing::instrument(name = "store.epoch.stamp", skip_all, err(Display))]
pub async fn stamp(
    pool: &PgPool,
    product_version: &str,
) -> Result<SchemaMetadata, synveda_types::Error> {
    // The head as the database has it, not as this binary's migrator would
    // have applied it: they agree here (this runs after a successful run) and
    // the database is the one that gets quoted back in a refusal.
    let head: Option<i64> = sqlx::query_scalar!(r#"select max(version) from _sqlx_migrations"#)
        .fetch_one(pool)
        .await
        .map_err(storage)?;
    let head = head.map_or_else(|| "none".to_owned(), |version| format!("{version:04}"));

    let row = sqlx::query!(
        r#"
        insert into schema_metadata (id, epoch, migration_head, created_by_version)
        values (true, $1, $2, $3)
        on conflict (id) do update
            set migration_head = excluded.migration_head,
                updated_at     = now()
        returning epoch              as "epoch!",
                  migration_head     as "migration_head!",
                  created_at         as "created_at!",
                  created_by_version as "created_by_version!",
                  updated_at         as "updated_at!"
        "#,
        CURRENT_EPOCH,
        head,
        product_version,
    )
    .fetch_one(pool)
    .await
    .map_err(storage)?;

    Ok(SchemaMetadata {
        epoch: row.epoch,
        migration_head: row.migration_head,
        created_at: row.created_at,
        created_by_version: row.created_by_version,
        updated_at: row.updated_at,
    })
}

/// Turns a sqlx failure into a verdict, or into a don't-know.
///
/// `42P01` is undefined_table and `42703` is undefined_column: the first says
/// there is no marker, the second says there is something else wearing its
/// name. Decode failures land in the same place as the second — a column of
/// the wrong type is the same problem as a column that is not there.
fn classify(error: sqlx::Error) -> SchemaEpochError {
    match &error {
        sqlx::Error::Database(db) => match db.code().as_deref() {
            Some("42P01") => SchemaEpochError::Missing,
            Some("42703") => SchemaEpochError::Malformed(db.message().to_owned()),
            _ => SchemaEpochError::Unreachable(error.to_string()),
        },
        sqlx::Error::RowNotFound => SchemaEpochError::Missing,
        sqlx::Error::ColumnNotFound(_)
        | sqlx::Error::ColumnDecode { .. }
        | sqlx::Error::Decode(_)
        | sqlx::Error::TypeNotFound { .. } => SchemaEpochError::Malformed(error.to_string()),
        _ => SchemaEpochError::Unreachable(error.to_string()),
    }
}

fn storage(error: sqlx::Error) -> synveda_types::Error {
    synveda_types::Error::Storage {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every refusal has to name the command that fixes it, in full. An
    /// error that says "reset the database" and leaves somebody to find out
    /// how is the reason this string is a constant.
    #[test]
    fn every_refusal_prints_the_reset_command_except_the_one_that_must_not() {
        for refusal in [
            SchemaEpochError::Missing,
            SchemaEpochError::Malformed("no column `epoch`".to_owned()),
            SchemaEpochError::Older { found: 0 },
        ] {
            let rendered = refusal.to_string();
            assert!(
                rendered.contains(RESET_COMMAND),
                "a refusal that does not say how to recover: {rendered}"
            );
            assert!(refusal.is_refusal(), "{rendered}");
        }

        // The exception, and it is the point of having a `Newer` variant at
        // all: a database from a *later* build holds data this one cannot
        // read, so telling its operator to reset would be telling them to
        // destroy it. It names the command only to say not to run it.
        let newer = SchemaEpochError::Newer { found: 99 }.to_string();
        assert!(newer.contains("Upgrade this installation"), "{newer}");
        assert!(newer.contains("would destroy it"), "{newer}");

        // An outage is not a verdict, and must not read as one.
        let outage = SchemaEpochError::Unreachable("connection refused".to_owned());
        assert!(!outage.is_refusal());
        assert!(!outage.to_string().contains(RESET_COMMAND));
    }
}
