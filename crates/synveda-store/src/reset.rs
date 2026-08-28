//! Destroying and recreating the application database (CPR-2, ADR-0069).
//!
//! The other half of the epoch guard. [`epoch::verify`](crate::epoch::verify)
//! refuses a database from before the cut; this is the one supported way to
//! get past that refusal, and it is deliberately not an upgrade — it drops the
//! database and builds a fresh one at [`CURRENT_EPOCH`](crate::epoch::CURRENT_EPOCH).
//!
//! ## Why the database rather than the volume
//!
//! The single-node profile's Postgres holds more than this product's schema:
//! Temporal's `temporal` and `temporal_visibility` databases live in the same
//! `pg-data` volume (`deploy/compose/docker-compose.yml`). Removing the volume
//! would be the blunt version of this and would take those with it, so what
//! this does is `DROP DATABASE` — the smallest thing that leaves nothing of
//! the old epoch behind and touches nothing that was never ours.
//!
//! ## The one place this crate builds SQL from a string
//!
//! Queries are compile-time checked everywhere a value is involved.
//! `DROP DATABASE` takes an *identifier*,
//! which no protocol placeholder can carry — there is no parameterised form of
//! this statement in Postgres. So the name is validated against a deliberately
//! narrow grammar ([`is_safe_identifier`]), double-quoted, and used; the
//! validation is the check the placeholder would otherwise have been, and it
//! is stricter than Postgres's own rules because a database name that needs
//! escaping to be safe is a database name this command declines to destroy.

use sqlx::postgres::PgPoolOptions;
use sqlx::{ConnectOptions, PgPool};
use synveda_types::{Error, Result};

use crate::epoch::SchemaMetadata;

/// What one reset did.
#[derive(Debug, Clone)]
pub struct Recreated {
    /// The database's name, as it was destroyed and recreated.
    pub database: String,
    /// Whether a database of that name was there to destroy. `false` on the
    /// first run against a server that never had one, which is not an error —
    /// this command's contract is the state it leaves, not the state it found.
    pub existed_before: bool,
    /// Extensions this created, and any it could not.
    pub extensions: Vec<ExtensionOutcome>,
    /// The epoch marker the fresh database now carries.
    pub metadata: SchemaMetadata,
}

/// One required extension's fate.
#[derive(Debug, Clone)]
pub struct ExtensionOutcome {
    /// The extension's name.
    pub name: &'static str,
}

/// The extensions a Synveda database cannot be migrated without.
///
/// `vector` stores labelled Knowledge embeddings; `btree_gin` supports the
/// baseline's mixed scalar/text indexes. Both are named here so reset reports
/// a missing server package before the baseline reaches an opaque DDL error.
const REQUIRED_EXTENSIONS: &[&str] = &["vector", "btree_gin"];

/// Drops the database the URL names, creates it again, installs the
/// extensions, and migrates it to the current epoch.
///
/// Idempotent: the drop is `if exists` and everything after it builds the same
/// database from nothing, so a second run leaves exactly what the first did.
///
/// This connects to the server's `postgres` maintenance database to do the
/// DDL, because a session cannot drop the database it is connected to.
#[tracing::instrument(name = "store.reset.recreate", skip_all, err(Display))]
pub async fn recreate(database_url: &str) -> Result<Recreated> {
    let options = crate::database_url::parse("DATABASE_URL", database_url)?;
    let database = options
        .get_database()
        .ok_or_else(|| Error::Invalid {
            message: "DATABASE_URL must name the PostgreSQL database to reset".to_owned(),
        })?
        .to_owned();
    if !is_safe_identifier(&database) {
        return Err(Error::Invalid {
            message: format!(
                "`{database}` is not a database name this command will destroy: it \
                 must be 1..=63 characters of ASCII letters, digits and underscores, \
                 starting with a letter or an underscore. Drop and recreate it by \
                 hand instead."
            ),
        });
    }

    // One connection, not a pool: this does four statements and then goes
    // away, and a pool against the maintenance database would be four
    // connections holding it open while we try to drop something.
    let maintenance = options.clone().database("postgres");
    let mut admin = maintenance.connect().await.map_err(|err| Error::Storage {
        message: format!(
            "connect to the `postgres` maintenance database on {}:{}: {err}\n\
                 (dropping a database means connecting to a different one; the \
                 role in DATABASE_URL needs to be able to reach `postgres` and to \
                 CREATEDB)",
            options.get_host(),
            options.get_port(),
        ),
    })?;

    let existed_before: bool = sqlx::query_scalar!(
        r#"select exists (select from pg_database where datname = $1) as "e!""#,
        database
    )
    .fetch_one(&mut admin)
    .await
    .map_err(|err| Error::Storage {
        message: format!("look for database `{database}`: {err}"),
    })?;

    // `WITH (FORCE)` terminates the sessions still on it (Postgres 13+).
    // Without it a single leftover connection — a gateway that was not
    // stopped, a `psql` in another terminal — turns this into a refusal that
    // reads as a permissions problem.
    let quoted = quote_identifier(&database);
    sqlx::query(&format!("drop database if exists {quoted} with (force)"))
        .execute(&mut admin)
        .await
        .map_err(|err| Error::Storage {
            message: format!("drop database `{database}`: {err}"),
        })?;
    sqlx::query(&format!("create database {quoted}"))
        .execute(&mut admin)
        .await
        .map_err(|err| Error::Storage {
            message: format!("create database `{database}`: {err}"),
        })?;
    drop(admin);

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(|err| Error::Storage {
            message: format!("connect to the fresh database `{database}`: {err}"),
        })?;

    let extensions = install_extensions(&pool).await?;
    let metadata = crate::migrate_reporting(&pool).await?;
    pool.close().await;

    Ok(Recreated {
        database,
        existed_before,
        extensions,
        metadata,
    })
}

/// Installs the extensions the compose profile's `initdb` script installs.
///
/// The extensions are the database's rather than the migrations' — that is the
/// split `deploy/compose/postgres/initdb/01-extensions.sql` and
/// `scripts/db-test.sh` already make, and creating a database means taking on
/// what they did.
async fn install_extensions(pool: &PgPool) -> Result<Vec<ExtensionOutcome>> {
    let mut outcomes = Vec::new();
    for name in REQUIRED_EXTENSIONS {
        // A constant, never a value: `name` comes from the array above and
        // reaches no other caller.
        let statement = format!("create extension if not exists {name}");
        match sqlx::query(&statement).execute(pool).await {
            Ok(_) => outcomes.push(ExtensionOutcome { name }),
            Err(err) => {
                return Err(Error::Storage {
                    message: format!(
                        "create extension `{name}`: {err}\n\
                         (Synveda's schema cannot be built without it — \
                         `vector` stores Knowledge embeddings and `btree_gin` \
                         supports indexed scalar/text predicates. Install the extension on this Postgres \
                         server, or point DATABASE_URL at the bundled one.)"
                    ),
                });
            }
        }
    }
    Ok(outcomes)
}

/// Wraps a validated name in double quotes.
///
/// [`is_safe_identifier`] has already refused anything that would need
/// escaping, so this cannot be the thing that makes a name safe — it is here
/// so a name that happens to collide with a keyword still works.
fn quote_identifier(name: &str) -> String {
    format!("\"{name}\"")
}

/// The grammar this command will destroy a database under.
///
/// Narrower than Postgres's own: no quotes, no spaces, no dots, no dollar
/// signs, ASCII only. Everything a `DROP DATABASE` could be talked into doing
/// with a hostile name is outside it, and so is every name a Synveda
/// deployment actually uses (`synveda`, `synveda_test_4711`).
fn is_safe_identifier(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The validator is the placeholder this statement cannot have, so it is
    /// tested as one: what a hostile name would do if it got through is
    /// `drop database "synveda"; drop database "temporal"; --"`.
    #[test]
    fn only_ordinary_database_names_are_destroyable() {
        for good in ["synveda", "synveda_test_4711", "_scratch", "S"] {
            assert!(is_safe_identifier(good), "{good} is an ordinary name");
        }
        for bad in [
            "",
            "synveda\"; drop database temporal; --",
            "synveda; drop database temporal",
            "syn veda",
            "9lives",
            "synveda-dev",
            "public.synveda",
            "café",
            "$$",
            &"x".repeat(64),
        ] {
            assert!(
                !is_safe_identifier(bad),
                "`{bad}` reached a DROP DATABASE statement"
            );
        }
    }

    /// Quoting is belt to the validator's braces, and it has to survive a
    /// keyword — `user` and `table` are legal database names.
    #[test]
    fn a_keyword_name_is_still_quoted() {
        assert_eq!(quote_identifier("user"), "\"user\"");
    }

    /// The required set is what the migrations actually call, and the
    /// optional one is what the product does not. Pinned here because the
    /// difference is the difference between "your Postgres is missing
    /// something" and "you cannot run Synveda on this Postgres".
    #[test]
    fn the_required_extensions_are_the_ones_a_migration_needs() {
        assert_eq!(REQUIRED_EXTENSIONS, &["vector", "btree_gin"]);
    }

    #[tokio::test]
    async fn invalid_reset_urls_never_disclose_their_credentials() {
        const SENTINEL: &str = "SYNVEDA_STORE_RESET_SECRET";
        let database_urls = [
            format!("postgres://admin:{SENTINEL}@localhost"),
            format!("https://admin:{SENTINEL}@localhost/synveda"),
            format!("postgres://admin@localhost/synveda?access_token={SENTINEL}"),
        ];
        for database_url in &database_urls {
            let error = recreate(database_url)
                .await
                .expect_err("unsafe reset URL must be refused before connecting");
            assert!(!error.to_string().contains(SENTINEL), "{error}");
            assert!(!error.to_string().contains(database_url), "{error}");
        }
    }
}
