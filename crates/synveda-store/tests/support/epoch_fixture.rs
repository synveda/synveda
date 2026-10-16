//! Shared isolated lifecycle database custody for store and gateway upgrade tests.
#![allow(dead_code)]
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{ConnectOptions, PgPool};
use std::sync::atomic::{AtomicU32, Ordering};
use synveda_store::runtime_role::DatabaseRoles;
use synveda_types::{TenantId, TenantStatus};
pub(crate) struct Server {
    pub(crate) admin_url: String,
    pub(crate) admin: PgConnectOptions,
    pub(crate) migrator_url: String,
    pub(crate) migrator: PgConnectOptions,
    pub(crate) roles: DatabaseRoles,
}

pub(crate) fn private_database_url(setting: &str) -> Option<String> {
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
pub(crate) fn server() -> Option<Server> {
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
pub(crate) struct Scratch {
    pub(crate) admin: PgConnectOptions,
    pub(crate) options: PgConnectOptions,
    pub(crate) admin_url: String,
    pub(crate) migrator_url: String,
    pub(crate) roles: DatabaseRoles,
    pub(crate) name: String,
    pub(crate) rt: tokio::runtime::Runtime,
}

pub(crate) static NEXT: AtomicU32 = AtomicU32::new(0);

impl Scratch {
    /// An **empty** database: created, extensions installed, not migrated.
    /// This is what a first install meets.
    pub(crate) fn empty(server: &Server) -> Self {
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
    pub(crate) fn url(&self) -> String {
        self.migrator_url.clone()
    }

    pub(crate) fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
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

pub(crate) fn replace_database(source: &str, database: &str) -> String {
    let head = source.split('?').next().unwrap_or(source);
    let base = &head[..head.rfind('/').expect("a database name in lifecycle URL")];
    format!("{base}/{database}{}", &source[head.len()..])
}

pub(crate) async fn connect_pool(options: &PgConnectOptions) -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options.clone())
        .await
        .expect("connect to the scratch database")
}

pub(crate) async fn protected_membership_fingerprint(
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

pub(crate) fn replace_credentials(source: &str, username: &str, password: &str) -> String {
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
pub(crate) async fn admit_a_tenant(pool: &PgPool) -> TenantId {
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

pub(crate) async fn tenant_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar::<_, i64>("select count(*) from tenants")
        .fetch_one(pool)
        .await
        .expect("count tenants")
}

pub(crate) async fn migration_ledger(
    pool: &PgPool,
) -> Vec<(
    i64,
    String,
    chrono::DateTime<chrono::Utc>,
    bool,
    Vec<u8>,
    i64,
)> {
    sqlx::query_as(
        "select version, description, installed_on, success, checksum, execution_time \
           from _sqlx_migrations order by version",
    )
    .fetch_all(pool)
    .await
    .expect("read migration ledger")
}

pub(crate) fn released_v043_migrator() -> sqlx::migrate::Migrator {
    sqlx::migrate::Migrator {
        migrations: std::borrow::Cow::Owned(vec![synveda_store::MIGRATOR.migrations[0].clone()]),
        ..sqlx::migrate::Migrator::DEFAULT
    }
}

pub(crate) fn forward_migrator() -> sqlx::migrate::Migrator {
    sqlx::migrate::Migrator {
        migrations: std::borrow::Cow::Owned(synveda_store::MIGRATOR.migrations[..2].to_vec()),
        ..sqlx::migrate::Migrator::DEFAULT
    }
}

/// Removes the marker while leaving product data in place. This is the shape
/// a pre-epoch database presents to the guard; the migration ledger is left
/// deliberately irrelevant because preflight must refuse before sqlx reads it.
pub(crate) async fn remove_epoch_marker(pool: &PgPool) {
    sqlx::query("drop table schema_metadata")
        .execute(pool)
        .await
        .expect("drop the marker");
}

// ── a fresh database ─────────────────────────────────────────────────────
