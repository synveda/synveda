//! CPR-2 store tests: the schema epoch guard and the reset that is the only
//! way past it (ADR-0068 decision 3, ADR-0069).
//!
//! These tests need a live Postgres and, unlike the rest of the suite, they
//! need **databases of their own**: every one of them either drops a table,
//! rewrites the epoch marker, or destroys the whole database, so a shared one
//! would be a suite that breaks the suite. They read private URL files for the
//! server, credentials and port, mint their own database name from them, and
//! drop it on the way out — the same rule `scripts/db-test.sh` follows one
//! layer up. Without `DATABASE_URL` every test skips quietly (CI has no
//! database); run them with `make db-test`.

use std::sync::atomic::{AtomicU32, Ordering};

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{ConnectOptions, Connection, PgPool};
use synveda_store::epoch::{
    self, CURRENT_BASELINE_REVISION, CURRENT_EPOCH, RESET_COMMAND, SchemaEpochError,
};
use synveda_store::runtime_role::DatabaseRoles;
use synveda_types::{TenantId, TenantStatus};

struct Server {
    admin_url: String,
    admin: PgConnectOptions,
    migrator_url: String,
    migrator: PgConnectOptions,
    roles: DatabaseRoles,
}

fn private_database_url(setting: &str) -> Option<String> {
    let path = std::env::var_os(setting)?;
    Some(
        std::fs::read_to_string(path)
            .expect("read isolated lifecycle database URL")
            .trim_end_matches('\n')
            .to_owned(),
    )
}

/// The isolated lifecycle server the harness names, or `None` when there is
/// nothing destructive to test against.
fn server() -> Option<Server> {
    let admin_url = match private_database_url("SYNVEDA_EPOCH_TEST_ADMIN_DATABASE_URL_FILE") {
        Some(url) => url,
        None => {
            eprintln!(
                "skipping schema epoch tests: the isolated lifecycle fixture is not set \
                 (run `make db-test`)"
            );
            return None;
        }
    };
    let migrator_url = private_database_url("SYNVEDA_EPOCH_TEST_MIGRATOR_DATABASE_URL_FILE")
        .expect("lifecycle fixture supplies the migrator URL file");
    let role_json = match (
        std::env::var("SYNVEDA_DATABASE_ROLES").ok(),
        std::env::var("SYNVEDA_DATABASE_ROLES_FILE").ok(),
    ) {
        (Some(value), None) => value,
        (None, Some(path)) => {
            std::fs::read_to_string(path).expect("read lifecycle database role contract")
        }
        _ => panic!("lifecycle fixture supplies exactly one database role contract"),
    };
    let roles = DatabaseRoles::parse_json(&role_json).expect("parse lifecycle database roles");
    let admin = synveda_store::database_url::parse(
        "SYNVEDA_EPOCH_TEST_ADMIN_DATABASE_URL_FILE",
        &admin_url,
    )
    .expect("admin lifecycle URL");
    let migrator = synveda_store::database_url::parse(
        "SYNVEDA_EPOCH_TEST_MIGRATOR_DATABASE_URL_FILE",
        &migrator_url,
    )
    .expect("migrator lifecycle URL");
    Some(Server {
        admin_url,
        admin,
        migrator_url,
        migrator,
        roles,
    })
}

/// A database of this test's own, dropped when the guard goes out of scope.
///
/// The name is derived from the process id and a counter, so two tests in
/// this binary — and two `make db-test` runs on one machine — cannot collide.
struct Scratch {
    admin: PgConnectOptions,
    options: PgConnectOptions,
    admin_url: String,
    migrator_url: String,
    roles: DatabaseRoles,
    name: String,
    rt: tokio::runtime::Runtime,
}

static NEXT: AtomicU32 = AtomicU32::new(0);

impl Scratch {
    /// An **empty** database: created, extensions installed, not migrated.
    /// This is what a first install meets.
    fn empty(server: &Server) -> Self {
        let name = format!(
            "synveda_epoch_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");
        let options = server.migrator.clone().database(&name);
        let role = server.roles.migrator();
        for role in [
            server.roles.migrator(),
            server.roles.gateway(),
            server.roles.worker(),
        ]
        .into_iter()
        .chain(
            server
                .roles
                .runtime()
                .administrators()
                .iter()
                .map(String::as_str),
        ) {
            assert!(
                role.chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_'),
                "lifecycle roles use the test fixture's closed identifier vocabulary"
            );
        }
        rt.block_on(async {
            let mut admin = server
                .admin
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
            sqlx::query(&format!(
                "create database \"{name}\" with owner \"{role}\" template template0 \
                 encoding 'UTF8' allow_connections false"
            ))
            .execute(&mut admin)
            .await
            .expect("create the scratch database");
            let administrators = server
                .roles
                .runtime()
                .administrators()
                .iter()
                .map(|administrator| format!("\"{administrator}\""))
                .collect::<Vec<_>>()
                .join(", ");
            let mut transaction = sqlx::Connection::begin(&mut admin)
                .await
                .expect("begin scratch database ACL transaction");
            for statement in [
                format!("set local role \"{role}\""),
                format!("revoke all on database \"{name}\" from public"),
                format!("grant create, connect, temporary on database \"{name}\" to \"{role}\""),
                format!(
                    "grant connect on database \"{name}\" to \"{}\", \"{}\"",
                    server.roles.gateway(),
                    server.roles.worker(),
                ),
                format!("grant connect on database \"{name}\" to {administrators}"),
                "set local role none".to_owned(),
                format!("alter database \"{name}\" with allow_connections true"),
            ] {
                sqlx::query(&statement)
                    .execute(&mut *transaction)
                    .await
                    .expect("converge one exact scratch database ACL step");
            }
            transaction
                .commit()
                .await
                .expect("commit exact scratch database ACLs");
            drop(admin);

            let admin_pool = connect_pool(&server.admin.clone().database(&name)).await;
            sqlx::query(&format!("alter schema public owner to \"{role}\""))
                .execute(&admin_pool)
                .await
                .expect("converge public schema owner");
            sqlx::query("revoke all on schema public from public")
                .execute(&admin_pool)
                .await
                .expect("revoke public schema access");
            for extension in ["vector", "btree_gin"] {
                sqlx::query(&format!("create extension if not exists {extension}"))
                    .execute(&admin_pool)
                    .await
                    .expect("install an extension");
            }
            admin_pool.close().await;
        });
        Self {
            admin: server.admin.clone(),
            options,
            admin_url: server.admin_url.clone(),
            migrator_url: replace_database(&server.migrator_url, &name),
            roles: server.roles.clone(),
            name,
            rt,
        }
    }

    /// `DATABASE_URL` with the database name swapped — the server,
    /// credentials and port stay the caller's, exactly as
    /// `scripts/db-test.sh` does it one layer up.
    fn url(&self) -> String {
        self.migrator_url.clone()
    }

    fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
        self.rt.block_on(future)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let options = self.admin.clone().database("postgres");
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

fn replace_database(source: &str, database: &str) -> String {
    let head = source.split('?').next().unwrap_or(source);
    let base = &head[..head.rfind('/').expect("a database name in lifecycle URL")];
    format!("{base}/{database}{}", &source[head.len()..])
}

async fn connect_pool(options: &PgConnectOptions) -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options.clone())
        .await
        .expect("connect to the scratch database")
}

async fn protected_membership_fingerprint(
    connection: &mut sqlx::PgConnection,
    roles: &DatabaseRoles,
) -> String {
    let protected = vec![
        "synveda_app".to_owned(),
        roles.migrator().to_owned(),
        roles.gateway().to_owned(),
        roles.worker().to_owned(),
    ];
    sqlx::query_scalar::<_, String>(
        r#"select coalesce(
             pg_catalog.string_agg(
               membership.roleid::text || ':' || membership.member::text || ':' ||
               membership.grantor::text || ':' || membership.admin_option::text || ':' ||
               membership.inherit_option::text || ':' || membership.set_option::text,
               ',' order by membership.roleid, membership.member, membership.grantor
             ),
             ''
           )
             from pg_catalog.pg_auth_members as membership
             join pg_catalog.pg_roles as granted on granted.oid = membership.roleid
             join pg_catalog.pg_roles as member on member.oid = membership.member
            where granted.rolname = any($1::text[])
               or member.rolname = any($1::text[])"#,
    )
    .bind(protected)
    .fetch_one(connection)
    .await
    .expect("fingerprint protected memberships")
}

fn replace_credentials(source: &str, username: &str, password: &str) -> String {
    let (scheme, rest) = source
        .split_once("://")
        .expect("lifecycle URL has a scheme");
    let (_, endpoint) = rest
        .rsplit_once('@')
        .expect("lifecycle URL has credentials");
    format!("{scheme}://{username}:{password}@{endpoint}")
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

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate");

        let metadata = epoch::verify(&pool).await.expect("a current database");
        assert_eq!(metadata.epoch, CURRENT_EPOCH);
        assert_eq!(metadata.baseline_revision, CURRENT_BASELINE_REVISION);
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

/// SQLx commits a transactional migration and its success ledger before it
/// can update timing metadata or Synveda can begin the epoch-stamp
/// transaction. A process death at that exact seam must be restart-idempotent.
#[test]
fn an_exact_applied_baseline_without_a_stamp_is_recovered() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("acquire migration connection");
        synveda_store::MIGRATOR
            .run(&mut *connection)
            .await
            .expect("commit the transactional baseline");
        drop(connection);

        assert!(has_marker(&pool).await);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("select count(*) from schema_metadata")
                .fetch_one(&pool)
                .await
                .expect("count pending marker rows"),
            0,
            "the fixture is exactly after SQLx and before the stamp"
        );
        epoch::preflight(&pool)
            .await
            .expect("the exact pending stamp is recoverable");

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("resume and stamp the committed baseline");
        let metadata = epoch::verify(&pool).await.expect("recovered current epoch");
        assert_eq!(metadata.epoch, CURRENT_EPOCH);
        assert_eq!(metadata.baseline_revision, CURRENT_BASELINE_REVISION);
        assert_eq!(tenant_count(&pool).await, 0);
        pool.close().await;
    });
}

/// An empty marker is not by itself recovery authority. Even one checksum
/// byte of ledger drift keeps the hard cut closed and leaves both catalogues
/// untouched.
#[test]
fn an_empty_marker_with_a_drifted_ledger_is_refused_without_a_stamp() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("acquire migration connection");
        synveda_store::MIGRATOR
            .run(&mut *connection)
            .await
            .expect("commit the transactional baseline");
        drop(connection);
        sqlx::query(
            "update _sqlx_migrations \
             set checksum = pg_catalog.decode(pg_catalog.repeat('00', 48), 'hex')",
        )
        .execute(&pool)
        .await
        .expect("drift the sole ledger checksum");
        let ledger_before: Vec<(i64, Vec<u8>, bool)> = sqlx::query_as(
            "select version, checksum, success from _sqlx_migrations order by version",
        )
        .fetch_all(&pool)
        .await
        .expect("read drifted ledger");

        let error = synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect_err("a drifted ledger is not pending-stamp authority");
        assert!(error.to_string().contains(RESET_COMMAND));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("select count(*) from schema_metadata")
                .fetch_one(&pool)
                .await
                .expect("count marker rows after refusal"),
            0
        );
        let ledger_after: Vec<(i64, Vec<u8>, bool)> = sqlx::query_as(
            "select version, checksum, success from _sqlx_migrations order by version",
        )
        .fetch_all(&pool)
        .await
        .expect("read unchanged drifted ledger");
        assert_eq!(ledger_after, ledger_before);
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
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate");
        let first = epoch::verify(&pool).await.expect("a current database");
        let tenant = admit_a_tenant(&pool).await;

        // The idempotent second run every `synveda init` and every test
        // harness performs.
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate again");
        let second = epoch::verify(&pool).await.expect("still current");

        assert_eq!(second.epoch, first.epoch);
        assert_eq!(second.baseline_revision, first.baseline_revision);
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

/// CPR-45 amended the contents of the pre-release epoch-3 baseline without
/// changing the domain epoch. An epoch-3 marker from before the immutable
/// revision discriminator must be refused before SQLx compares checksums.
#[test]
fn an_unversioned_epoch_three_baseline_is_refused_before_migration() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate");
        admit_a_tenant(&pool).await;
        let ledger_before: Vec<(i64, Vec<u8>, bool)> = sqlx::query_as(
            "select version, checksum, success from _sqlx_migrations order by version",
        )
        .fetch_all(&pool)
        .await
        .expect("read migration ledger");

        sqlx::query("alter table schema_metadata drop column baseline_revision")
            .execute(&pool)
            .await
            .expect("model the unversioned interim epoch-3 marker");

        let refusal = epoch::verify(&pool).await.expect_err("unversioned epoch 3");
        assert_eq!(refusal, SchemaEpochError::OlderRevision { found: 0 });
        assert!(refusal.to_string().contains(RESET_COMMAND));
        let migrate_error = synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect_err("revision refusal precedes SQLx");
        assert!(migrate_error.to_string().contains(RESET_COMMAND));

        let ledger_after: Vec<(i64, Vec<u8>, bool)> = sqlx::query_as(
            "select version, checksum, success from _sqlx_migrations order by version",
        )
        .fetch_all(&pool)
        .await
        .expect("read unchanged migration ledger");
        assert_eq!(ledger_after, ledger_before, "SQLx never reached the ledger");
        assert_eq!(tenant_count(&pool).await, 1, "product rows were untouched");
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
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate");
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
        let err = synveda_store::migrate(&pool, &scratch.roles)
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
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate");
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
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate");
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
    drop(scratch);

    // (b) our shape, holding a value the guard will not accept. The CHECK
    // constraints stop *us* writing one; they do not stop a restore, a
    // hand-edit or another product's tooling, and this is the code that
    // meets that.
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate");
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
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate");

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
        let migrate_error = synveda_store::migrate(&pool, &scratch.roles)
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

        sqlx::query("update schema_metadata set epoch = $1, baseline_revision = $2")
            .bind(CURRENT_EPOCH)
            .bind(CURRENT_BASELINE_REVISION + 1)
            .execute(&pool)
            .await
            .expect("advance only the baseline revision");
        let newer_revision = epoch::verify(&pool)
            .await
            .expect_err("a newer baseline revision");
        assert_eq!(
            newer_revision,
            SchemaEpochError::NewerRevision {
                found: CURRENT_BASELINE_REVISION + 1,
            }
        );
        let message = newer_revision.to_string();
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
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate");
        admit_a_tenant(&pool).await;
        remove_epoch_marker(&pool).await;
        assert!(
            epoch::verify(&pool).await.is_err(),
            "the fixture is refused"
        );
        pool.close().await;

        let first = synveda_store::reset::recreate(&scratch.admin_url, &url, &scratch.roles)
            .await
            .expect("reset a refused database");
        assert!(first.existed_before, "there was a database to destroy");
        assert_eq!(first.metadata.epoch, CURRENT_EPOCH);
        assert_eq!(first.metadata.baseline_revision, CURRENT_BASELINE_REVISION);
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
        assert_eq!(metadata.baseline_revision, CURRENT_BASELINE_REVISION);
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
        let second = synveda_store::reset::recreate(&scratch.admin_url, &url, &scratch.roles)
            .await
            .expect("reset again");
        assert!(second.existed_before);
        assert_eq!(second.metadata.epoch, CURRENT_EPOCH);
        assert_eq!(second.metadata.baseline_revision, CURRENT_BASELINE_REVISION);
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
            .admin
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

        let outcome = synveda_store::reset::recreate(&scratch.admin_url, &url, &scratch.roles)
            .await
            .expect("build a database from nothing");
        assert!(!outcome.existed_before);
        assert_eq!(outcome.metadata.epoch, CURRENT_EPOCH);
        assert_eq!(
            outcome.metadata.baseline_revision,
            CURRENT_BASELINE_REVISION
        );

        let pool = connect_pool(&scratch.options).await;
        epoch::verify(&pool).await.expect("a current database");
        pool.close().await;
    });
}

/// A safe-looking name is not sufficient authority to destroy a database.
/// Wrong ownership is refused while its catalog identity, data and protected
/// membership graph remain byte-for-byte unchanged.
#[test]
fn reset_refuses_a_wrong_owner_without_mutation() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    let url = scratch.url();
    scratch.block_on(async {
        let mut admin = server
            .admin
            .clone()
            .database("postgres")
            .connect()
            .await
            .expect("connect to the maintenance database");
        sqlx::query(&format!("drop database \"{}\" with (force)", scratch.name))
            .execute(&mut admin)
        .await
        .expect("drop the exact-owner scratch target");
        let wrong_owner = &server.roles.runtime().administrators()[0];
        sqlx::query(&format!(
            "create database \"{}\" with owner \"{wrong_owner}\" template template0 encoding 'UTF8'",
            scratch.name,
        ))
        .execute(&mut admin)
        .await
        .expect("create the wrong-owner target");
        let before_identity = sqlx::query_as::<_, (i64, String)>(
            "select database.oid::bigint, owner.rolname::text \
               from pg_catalog.pg_database as database \
               join pg_catalog.pg_roles as owner on owner.oid = database.datdba \
              where database.datname = $1",
        )
        .bind(&scratch.name)
        .fetch_one(&mut admin)
        .await
        .expect("read wrong-owner target identity");
        let before_memberships = protected_membership_fingerprint(&mut admin, &scratch.roles).await;

        let mut target_admin = server
            .admin
            .clone()
            .database(&scratch.name)
            .connect()
            .await
            .expect("connect to the wrong-owner target");
        sqlx::query("create table reset_wrong_owner_sentinel (id integer primary key)")
            .execute(&mut target_admin)
            .await
            .expect("create wrong-owner sentinel");
        sqlx::query("insert into reset_wrong_owner_sentinel (id) values (1)")
            .execute(&mut target_admin)
            .await
            .expect("write wrong-owner sentinel");
        target_admin
            .close()
            .await
            .expect("close sentinel connection");

        let error = synveda_store::reset::recreate(&scratch.admin_url, &url, &scratch.roles)
            .await
            .expect_err("wrong ownership must refuse before DROP DATABASE");
        assert!(
            error.to_string().contains("database reset requires"),
            "{error}"
        );
        let after_identity = sqlx::query_as::<_, (i64, String)>(
            "select database.oid::bigint, owner.rolname::text \
               from pg_catalog.pg_database as database \
               join pg_catalog.pg_roles as owner on owner.oid = database.datdba \
              where database.datname = $1",
        )
        .bind(&scratch.name)
        .fetch_one(&mut admin)
        .await
        .expect("re-read wrong-owner target identity");
        assert_eq!(after_identity, before_identity);
        let after_memberships = protected_membership_fingerprint(&mut admin, &scratch.roles).await;
        assert_eq!(after_memberships, before_memberships);

        let target_admin = connect_pool(&server.admin.clone().database(&scratch.name)).await;
        let sentinel: i64 = sqlx::query_scalar("select count(*) from reset_wrong_owner_sentinel")
            .fetch_one(&target_admin)
            .await
            .expect("read preserved wrong-owner sentinel");
        assert_eq!(sentinel, 1);
        target_admin.close().await;
    });
}

/// Managed-service administrators may be allowlisted for extension ownership
/// without having superuser authority. The local destructive reset command
/// refuses that shape before creating an absent target.
#[test]
fn reset_refuses_an_allowlisted_non_superuser_without_mutation() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    let url = scratch.url();
    scratch.block_on(async {
        let role = format!(
            "synveda_reset_admin_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let password = "ResetOnlyCredential_93";
        let mut admin = server
            .admin
            .clone()
            .database("postgres")
            .connect()
            .await
            .expect("connect to the maintenance database");
        sqlx::query(&format!("drop database \"{}\" with (force)", scratch.name))
            .execute(&mut admin)
            .await
            .expect("make the reset target absent");
        sqlx::query(&format!(
            "create role \"{role}\" login inherit nosuperuser nocreatedb nocreaterole \
             noreplication nobypassrls password '{password}'"
        ))
        .execute(&mut admin)
        .await
        .expect("create an ordinary reset administrator");
        sqlx::query(&format!("grant connect on database postgres to \"{role}\""))
            .execute(&mut admin)
            .await
            .expect("allow the ordinary administrator to reach maintenance");
        let roles = DatabaseRoles::new(
            server.roles.migrator().to_owned(),
            server.roles.gateway().to_owned(),
            server.roles.worker().to_owned(),
            vec![
                server.roles.runtime().administrators()[0].clone(),
                role.clone(),
            ],
            Vec::new(),
            server.roles.forbidden_databases().to_vec(),
            server.roles.isolated_peer_roles().to_vec(),
        )
        .expect("extended reset administrator contract");
        let before_memberships = protected_membership_fingerprint(&mut admin, &roles).await;
        let non_superuser_url = replace_credentials(&scratch.admin_url, &role, password);

        let error = synveda_store::reset::recreate(&non_superuser_url, &url, &roles)
            .await
            .expect_err("an ordinary allowlisted administrator cannot run local reset");
        assert!(
            error.to_string().contains("database reset requires"),
            "{error}"
        );
        let exists: bool = sqlx::query_scalar(
            "select exists (select 1 from pg_catalog.pg_database where datname = $1)",
        )
        .bind(&scratch.name)
        .fetch_one(&mut admin)
        .await
        .expect("prove the reset target stayed absent");
        assert!(!exists);
        let after_memberships = protected_membership_fingerprint(&mut admin, &roles).await;
        assert_eq!(after_memberships, before_memberships);

        sqlx::query(&format!(
            "revoke connect on database postgres from \"{role}\""
        ))
        .execute(&mut admin)
        .await
        .expect("remove the test administrator ACL");
        sqlx::query(&format!("drop role \"{role}\""))
            .execute(&mut admin)
            .await
            .expect("drop the ordinary reset administrator");
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
        let hostile = replace_database(&server.migrator_url, "syn%22veda");
        let err = synveda_store::reset::recreate(&server.admin_url, &hostile, &server.roles)
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
