//! CPR-2 store tests: the schema epoch guard and the reset that is the only
//! way past it (ADR-0068 decision 3, ADR-0069).
//!
//! These tests need a live Postgres and, unlike the rest of the suite, they
//! need **databases of their own**: every one of them either drops a table,
//! rewrites the epoch marker, or destroys the whole database, so a shared one
//! would be a suite that breaks the suite. They read `DATABASE_URL` for the
//! server, credentials and port, mint their own database name from it, and
//! drop it on the way out — the same rule `scripts/db-test.sh` follows one
//! layer up. Without `DATABASE_URL` every test skips quietly (CI has no
//! database); run them with `make db-test`.

use std::sync::atomic::{AtomicU32, Ordering};

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{ConnectOptions, PgPool};
use synveda_store::epoch::{self, CURRENT_EPOCH, RESET_COMMAND, SchemaEpochError};
use synveda_types::{TenantId, TenantStatus};

/// The server `DATABASE_URL` names, or `None` when there is nothing to test
/// against.
fn server() -> Option<PgConnectOptions> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) if !url.is_empty() => url,
        _ => {
            eprintln!(
                "skipping schema epoch tests: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
            return None;
        }
    };
    Some(url.parse().expect("DATABASE_URL is a Postgres URL"))
}

/// A database of this test's own, dropped when the guard goes out of scope.
///
/// The name is derived from the process id and a counter, so two tests in
/// this binary — and two `make db-test` runs on one machine — cannot collide.
struct Scratch {
    options: PgConnectOptions,
    name: String,
    rt: tokio::runtime::Runtime,
}

static NEXT: AtomicU32 = AtomicU32::new(0);

impl Scratch {
    /// An **empty** database: created, extensions installed, not migrated.
    /// This is what a first install meets.
    fn empty(server: &PgConnectOptions) -> Self {
        let name = format!(
            "synveda_epoch_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");
        let options = server.clone().database(&name);
        rt.block_on(async {
            let mut admin = server
                .clone()
                .database("postgres")
                .connect()
                .await
                .expect("connect to the maintenance database");
            // The name is this file's own, built from a pid and a counter,
            // and reaches no other caller.
            sqlx::query(&format!("drop database if exists \"{name}\" with (force)"))
                .execute(&mut admin)
                .await
                .expect("drop any leftover");
            sqlx::query(&format!("create database \"{name}\""))
                .execute(&mut admin)
                .await
                .expect("create the scratch database");
            drop(admin);

            let pool = connect_pool(&options).await;
            for extension in ["vector", "btree_gin"] {
                sqlx::query(&format!("create extension if not exists {extension}"))
                    .execute(&pool)
                    .await
                    .expect("install an extension");
            }
            pool.close().await;
        });
        Self { options, name, rt }
    }

    /// `DATABASE_URL` with the database name swapped — the server,
    /// credentials and port stay the caller's, exactly as
    /// `scripts/db-test.sh` does it one layer up.
    fn url(&self) -> String {
        let source = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        let head = source.split('?').next().unwrap_or(&source);
        let base = &head[..head.rfind('/').expect("a database name in DATABASE_URL")];
        format!("{base}/{}{}", self.name, &source[head.len()..])
    }

    fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
        self.rt.block_on(future)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let options = self.options.clone().database("postgres");
        let name = self.name.clone();
        // A failed drop is not worth failing a passing test over; `db-test`
        // already counts leftovers out loud.
        let _ = self.rt.block_on(async move {
            let mut admin = options.connect().await.ok()?;
            sqlx::query(&format!("drop database if exists \"{name}\" with (force)"))
                .execute(&mut admin)
                .await
                .ok()
        });
    }
}

async fn connect_pool(options: &PgConnectOptions) -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options.clone())
        .await
        .expect("connect to the scratch database")
}

/// Puts something in the database, so "nothing was carried across" and
/// "nothing was touched" are claims about rows rather than about tables.
async fn admit_a_tenant(pool: &PgPool) -> TenantId {
    let id = TenantId::new();
    synveda_store::tenants::create(
        pool,
        id,
        &format!("epoch-{}", uuid::Uuid::now_v7().simple()),
        "Epoch fixture",
        TenantStatus::Active,
    )
    .await
    .expect("admit a tenant");
    id
}

async fn tenant_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar::<_, i64>("select count(*) from tenants")
        .fetch_one(pool)
        .await
        .expect("count tenants")
}

/// Removes the marker while leaving product data in place. This is the shape
/// a pre-epoch database presents to the guard; the migration ledger is left
/// deliberately irrelevant because preflight must refuse before sqlx reads it.
async fn remove_epoch_marker(pool: &PgPool) {
    sqlx::query("drop table schema_metadata")
        .execute(pool)
        .await
        .expect("drop the marker");
}

// ── a fresh database ─────────────────────────────────────────────────────

/// The first install: an empty database, migrated, accepted, and carrying a
/// marker that says who made it and when.
#[test]
fn a_fresh_empty_database_bootstraps_to_the_current_epoch() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;

        // Nothing there yet: no marker, and the guard says so rather than
        // erroring on a missing table.
        assert_eq!(epoch::verify(&pool).await, Err(SchemaEpochError::Missing));
        // An empty database is not a pre-cut database, so migrating is
        // allowed — this is the case `preflight` must let through.
        epoch::preflight(&pool).await.expect("an empty database");

        synveda_store::migrate(&pool).await.expect("migrate");

        let metadata = epoch::verify(&pool).await.expect("a current database");
        assert_eq!(metadata.epoch, CURRENT_EPOCH);
        assert_eq!(
            metadata.migration_head,
            format!(
                "{:04}",
                synveda_store::MIGRATOR
                    .migrations
                    .last()
                    .expect("an embedded migration")
                    .version
            ),
            "the marker has to name the head actually reached"
        );
        assert_eq!(metadata.created_by_version, env!("CARGO_PKG_VERSION"));
        assert!(
            (chrono::Utc::now() - metadata.created_at).num_minutes() < 5,
            "the epoch was created just now: {}",
            metadata.created_at
        );

        // And the schema is really there — the marker is not a claim about a
        // migration that did not run.
        assert_eq!(tenant_count(&pool).await, 0);
        admit_a_tenant(&pool).await;
        assert_eq!(tenant_count(&pool).await, 1);
        pool.close().await;
    });
}

/// A database already at this epoch starts normally, and migrating it again
/// changes only the head — never who created it.
#[test]
fn a_current_epoch_database_starts_normally_and_keeps_its_provenance() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool).await.expect("migrate");
        let first = epoch::verify(&pool).await.expect("a current database");
        let tenant = admit_a_tenant(&pool).await;

        // The idempotent second run every `synveda init` and every test
        // harness performs.
        synveda_store::migrate(&pool).await.expect("migrate again");
        let second = epoch::verify(&pool).await.expect("still current");

        assert_eq!(second.epoch, first.epoch);
        assert_eq!(
            second.created_at, first.created_at,
            "the epoch was not recreated"
        );
        assert_eq!(
            second.created_by_version, first.created_by_version,
            "`created_by_version` answers which release minted this database, \
             not which one last ran"
        );
        assert_eq!(second.migration_head, first.migration_head);
        // And it did not disturb what was in it.
        assert_eq!(tenant_count(&pool).await, 1);
        assert!(
            synveda_store::tenants::by_id(&pool, tenant)
                .await
                .expect("resolve")
                .is_some()
        );
        pool.close().await;
    });
}

// ── a database from before the cut ───────────────────────────────────────

/// The whole point of the feature: a database written before the context
/// platform is refused, told what to run, and **left exactly as it was**.
#[test]
fn a_database_from_before_the_cut_is_refused_and_not_touched() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool).await.expect("migrate");
        admit_a_tenant(&pool).await;
        remove_epoch_marker(&pool).await;

        // The startup guard.
        let refusal = epoch::verify(&pool).await.expect_err("a pre-cut database");
        assert_eq!(refusal, SchemaEpochError::Missing);
        assert!(refusal.is_refusal(), "an outage is not a verdict");
        let message = refusal.to_string();
        assert!(message.contains(RESET_COMMAND), "{message}");
        assert!(message.contains("hard cut"), "{message}");

        // The migrator. Refused *before* it runs, so this is also the
        // assertion that no migration attempted a translation and stopped
        // half way.
        let err = synveda_store::migrate(&pool)
            .await
            .expect_err("migrating a pre-cut database");
        assert!(err.to_string().contains(RESET_COMMAND), "{err}");

        // Untouched: still no marker, and the row that was there is still
        // there, unread and unmoved.
        assert!(
            !has_marker(&pool).await,
            "a refused migration wrote the marker anyway"
        );
        assert_eq!(tenant_count(&pool).await, 1);
        pool.close().await;
    });
}

/// The other half of "missing": the table is there and the row is not.
///
/// Reached by anything that truncates it, and by a database created by
/// running the migrator directly rather than through `synveda_store::migrate`
/// — which is why the two cases share a verdict. A marker with no row says as
/// much about the model as no marker at all.
#[test]
fn a_marker_with_no_row_is_refused() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool).await.expect("migrate");
        sqlx::query("delete from schema_metadata")
            .execute(&pool)
            .await
            .expect("empty the marker");

        let refusal = epoch::verify(&pool).await.expect_err("no row");
        assert_eq!(refusal, SchemaEpochError::Missing);
        assert!(refusal.to_string().contains(RESET_COMMAND));
        pool.close().await;
    });
}

/// A marker this build cannot read is refused rather than guessed at.
///
/// Two shapes, and they are different failures: a table wearing the name with
/// other columns in it, and our own table holding a value the guard will not
/// stand behind. Both mean "nothing here can tell which model these rows are
/// in", which is the same answer as no marker at all — but said differently,
/// because an operator looking at a corrupted marker and one looking at an
/// old database are looking for different things.
#[test]
fn a_marker_this_build_cannot_read_is_refused() {
    let Some(server) = server() else { return };

    // (a) somebody else's `schema_metadata`.
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool).await.expect("migrate");
        sqlx::query("alter table schema_metadata rename column created_by_version to author")
            .execute(&pool)
            .await
            .expect("reshape the marker");

        let refusal = epoch::verify(&pool).await.expect_err("a foreign shape");
        assert!(
            matches!(refusal, SchemaEpochError::Malformed(_)),
            "expected a malformed verdict, got {refusal:?}"
        );
        assert!(refusal.is_refusal());
        assert!(refusal.to_string().contains(RESET_COMMAND));
        pool.close().await;
    });

    // (b) our shape, holding a value the guard will not accept. The CHECK
    // constraints stop *us* writing one; they do not stop a restore, a
    // hand-edit or another product's tooling, and this is the code that
    // meets that.
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool).await.expect("migrate");
        sqlx::query(
            "alter table schema_metadata
                 drop constraint schema_metadata_created_by_version_check",
        )
        .execute(&pool)
        .await
        .expect("drop the constraint this test is bypassing");
        sqlx::query("update schema_metadata set created_by_version = ''")
            .execute(&pool)
            .await
            .expect("blank the creating version");

        let refusal = epoch::verify(&pool).await.expect_err("a blank provenance");
        assert!(
            matches!(refusal, SchemaEpochError::Malformed(_)),
            "expected a malformed verdict, got {refusal:?}"
        );
        pool.close().await;
    });
}

/// An epoch this build has moved past is refused, and one it has not caught
/// up to is refused differently.
///
/// Epoch 2 is the immediately preceding development epoch and therefore a
/// concrete refusal, while a newer epoch still means the binary must move
/// forward rather than telling an operator to destroy readable data.
#[test]
fn an_epoch_that_is_not_this_one_is_refused_in_both_directions() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool).await.expect("migrate");

        sqlx::query("update schema_metadata set epoch = $1")
            .bind(CURRENT_EPOCH - 1)
            .execute(&pool)
            .await
            .expect("age the marker");
        let older = epoch::verify(&pool).await.expect_err("an older epoch");
        assert_eq!(
            older,
            SchemaEpochError::Older {
                found: CURRENT_EPOCH - 1
            }
        );
        let message = older.to_string();
        assert!(message.contains(RESET_COMMAND), "{message}");
        let migrate_error = synveda_store::migrate(&pool)
            .await
            .expect_err("epoch 2 must be refused before checksum comparison");
        assert!(
            migrate_error.to_string().contains(RESET_COMMAND),
            "{migrate_error}"
        );

        // The other direction is not a reset. A database from a newer build
        // holds data this one cannot read, so telling its operator to destroy
        // it would be the worst advice this guard could give.
        sqlx::query("update schema_metadata set epoch = $1")
            .bind(CURRENT_EPOCH + 1)
            .execute(&pool)
            .await
            .expect("advance the marker");
        let newer = epoch::verify(&pool).await.expect_err("a newer epoch");
        assert_eq!(
            newer,
            SchemaEpochError::Newer {
                found: CURRENT_EPOCH + 1
            }
        );
        let message = newer.to_string();
        assert!(message.contains("Upgrade this installation"), "{message}");
        assert!(message.contains("would destroy it"), "{message}");
        pool.close().await;
    });
}

// ── reset ────────────────────────────────────────────────────────────────

/// `synveda reset --database --force`'s engine: a database with an old epoch
/// and rows in it becomes a working current-epoch database with nothing in
/// it, and running it again leaves the same thing.
#[test]
fn reset_creates_a_working_current_epoch_database_and_is_idempotent() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    let url = scratch.url();
    scratch.block_on(async {
        // Something to destroy: a full pre-cut database with a tenant in it.
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool).await.expect("migrate");
        admit_a_tenant(&pool).await;
        remove_epoch_marker(&pool).await;
        assert!(
            epoch::verify(&pool).await.is_err(),
            "the fixture is refused"
        );
        pool.close().await;

        let first = synveda_store::reset::recreate(&url)
            .await
            .expect("reset a refused database");
        assert!(first.existed_before, "there was a database to destroy");
        assert_eq!(first.metadata.epoch, CURRENT_EPOCH);
        assert_eq!(first.metadata.created_by_version, env!("CARGO_PKG_VERSION"));
        let extension_names: Vec<_> = first
            .extensions
            .iter()
            .map(|extension| extension.name)
            .collect();
        assert_eq!(extension_names, ["vector", "btree_gin"]);

        // Working: the guard accepts it, the schema is there, and it takes a
        // write.
        let pool = connect_pool(&scratch.options).await;
        let metadata = epoch::verify(&pool).await.expect("a current database");
        assert_eq!(metadata.epoch, CURRENT_EPOCH);
        assert_eq!(
            tenant_count(&pool).await,
            0,
            "a row survived the reset — there is no path that carries one \
             across, and this is what says so"
        );
        admit_a_tenant(&pool).await;
        assert_eq!(tenant_count(&pool).await, 1);

        // Forced RLS survives, because a fresh database that lost the tenant
        // backstop would be a worse outcome than the refusal it replaced.
        let forced: bool = sqlx::query_scalar(
            "select relrowsecurity and relforcerowsecurity
             from pg_class where relname = 'knowledge_items'",
        )
        .fetch_one(&pool)
        .await
        .expect("read the RLS flags");
        assert!(
            forced,
            "the fresh database has no forced RLS on `knowledge_items`"
        );
        pool.close().await;

        // Idempotent: the same command again leaves the same database, with
        // the row this test just wrote gone.
        let second = synveda_store::reset::recreate(&url)
            .await
            .expect("reset again");
        assert!(second.existed_before);
        assert_eq!(second.metadata.epoch, CURRENT_EPOCH);
        let pool = connect_pool(&scratch.options).await;
        assert_eq!(tenant_count(&pool).await, 0);
        epoch::verify(&pool).await.expect("still current");
        pool.close().await;
    });
}

/// Reset on a server that has no such database at all — the state a first
/// install and a re-run after a manual `drop database` are both in.
#[test]
fn reset_builds_a_database_that_was_not_there() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    let url = scratch.url();
    scratch.block_on(async {
        // Take it away behind reset's back, so `existed_before` has something
        // to be false about.
        let mut admin = server
            .clone()
            .database("postgres")
            .connect()
            .await
            .expect("connect to the maintenance database");
        sqlx::query(&format!(
            "drop database if exists \"{}\" with (force)",
            scratch.name
        ))
        .execute(&mut admin)
        .await
        .expect("drop it");
        drop(admin);

        let outcome = synveda_store::reset::recreate(&url)
            .await
            .expect("build a database from nothing");
        assert!(!outcome.existed_before);
        assert_eq!(outcome.metadata.epoch, CURRENT_EPOCH);

        let pool = connect_pool(&scratch.options).await;
        epoch::verify(&pool).await.expect("a current database");
        pool.close().await;
    });
}

/// A database name this command will not destroy is refused before anything
/// is dropped. The unit tests in `reset.rs` pin the grammar; this pins that
/// the grammar is actually on the path.
#[test]
fn reset_refuses_a_database_name_it_will_not_quote() {
    let Some(server) = server() else { return };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    rt.block_on(async {
        let hostile = format!(
            "postgres://synveda:synveda-dev@{}:{}/{}",
            server.get_host(),
            server.get_port(),
            "syn%22veda"
        );
        let err = synveda_store::reset::recreate(&hostile)
            .await
            .expect_err("a name with a quote in it");
        assert!(
            err.to_string()
                .contains("not a database name this command will destroy"),
            "{err}"
        );
    });
}

// ── no old-to-new migrator ───────────────────────────────────────────────

/// Epoch 3 is one schema-only baseline: no predecessor chain, no reversible
/// pair and no top-level statement that can carry or seed data. Function
/// bodies may of course mutate current tables at runtime; the scanner removes
/// every PostgreSQL dollar-quoted body before classifying top-level verbs.
#[test]
fn no_old_to_new_data_migrator_exists() {
    let migrations = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");

    let mut files: Vec<_> = std::fs::read_dir(&migrations)
        .expect("read the migrations directory")
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sql"))
        .collect();
    files.sort();
    let names: Vec<_> = files
        .iter()
        .map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .expect("UTF-8 migration name")
        })
        .collect();
    assert_eq!(
        names,
        ["0001_context_platform.sql"],
        "epoch 3 is exactly one clean baseline"
    );

    let down: Vec<_> = names
        .iter()
        .filter(|name| name.ends_with(".down.sql"))
        .collect();
    assert!(
        down.is_empty(),
        "a down-migration would make the epoch look reversible and is where a \
         translation would hide: {down:?}"
    );

    let path = &files[0];
    let source = std::fs::read_to_string(path).expect("read the epoch migration");
    for statement in top_level_statements(&source) {
        let verb = statement.split_whitespace().next().unwrap_or_default();
        assert!(
            !matches!(
                verb,
                "select" | "insert" | "update" | "delete" | "copy" | "with"
            ),
            "{} runs top-level `{verb}`. The baseline creates schema only; a \
             statement here could carry or seed data outside the PDP.\n  {}",
            path.display(),
            statement.trim(),
        );
    }

    for retired in [
        "create table public.records",
        "records_versions",
        "memory_usage",
        "policy_lapses",
        "role_bindings",
        "'prompt', 'context_pack', 'memory'",
        "new.evidence is distinct from old.evidence",
        "create extension if not exists age",
        "create extension if not exists pgmq",
        "set row_security = off",
    ] {
        assert!(
            !source.to_lowercase().contains(retired),
            "the clean baseline retained `{retired}`"
        );
    }
}

/// A migration's statements, comments stripped and every dollar-quoted body
/// skipped. PostgreSQL permits both `$$` and tagged forms such as `$_$`.
fn top_level_statements(source: &str) -> Vec<String> {
    let without_bodies = strip_dollar_quoted(source);
    let stripped: String = without_bodies
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    stripped
        .split(';')
        .map(|statement| statement.trim().to_owned())
        .filter(|statement| !statement.is_empty())
        .collect()
}

fn strip_dollar_quoted(source: &str) -> String {
    let mut remaining = source;
    let mut stripped = String::with_capacity(source.len());
    while let Some(open) = remaining.find('$') {
        stripped.push_str(&remaining[..open]);
        let candidate = &remaining[open..];
        let Some(tag_tail) = candidate[1..].find('$') else {
            stripped.push_str(candidate);
            return stripped;
        };
        let tag_end = tag_tail + 1;
        let name = &candidate[1..tag_end];
        if !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            stripped.push('$');
            remaining = &candidate[1..];
            continue;
        }
        let delimiter = &candidate[..=tag_end];
        let body = &candidate[delimiter.len()..];
        let Some(close) = body.find(delimiter) else {
            stripped.push_str(candidate);
            return stripped;
        };
        stripped.push(' ');
        remaining = &body[close + delimiter.len()..];
    }
    stripped.push_str(remaining);
    stripped
}

async fn has_marker(pool: &PgPool) -> bool {
    sqlx::query_scalar::<_, bool>("select to_regclass('public.schema_metadata') is not null")
        .fetch_one(pool)
        .await
        .expect("look for the marker")
}
