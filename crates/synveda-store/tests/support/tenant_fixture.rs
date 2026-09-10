//! Test-only tenant admission through the exact migrator credential.
//!
//! Integration suites keep `DATABASE_URL` on the ordinary gateway role. The
//! one global write that precedes a tenant transaction uses a separately named
//! file-only migrator credential, verifies that principal against the shared
//! deployment role contract, admits one tenant, and closes the connection.

use std::path::Path;

use sqlx::ConnectOptions as _;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Transaction};
use synveda_types::{Tenant, TenantId, TenantStatus};

const MAX_CONFIGURATION_BYTES: u64 = 4096;

/// Matches [`synveda_store::tenants::create`] while deliberately ignoring the
/// supplied runtime executor for the global admission write. Keeping it in the
/// call makes each fixture's transition from admission authority to ordinary
/// tenant work visible at the call site.
pub async fn create<E>(
    runtime_executor: E,
    id: TenantId,
    slug: &str,
    name: &str,
    status: TenantStatus,
) -> synveda_types::Result<Tenant> {
    drop(runtime_executor);
    let url = private_configuration(
        "SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE",
        "test migrator database URL",
    );
    let roles_json =
        private_configuration("SYNVEDA_DATABASE_ROLES_FILE", "test database role contract");
    let roles = synveda_store::runtime_role::DatabaseRoles::parse_json(&roles_json)
        .expect("parse the isolated test database role contract");
    let options =
        synveda_store::database_url::parse("SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE", &url)
            .expect("parse the isolated test migrator URL");
    let mut connection = options
        .connect()
        .await
        .expect("connect the isolated test migrator");
    synveda_store::runtime_role::initialize_product_session_connection(&mut connection)
        .await
        .expect("normalise the test migrator session");
    synveda_store::runtime_role::verify_migrator_connection(&mut connection, &roles)
        .await
        .expect("verify the exact test migrator authority");
    synveda_store::tenants::create(&mut connection, id, slug, name, status).await
}

/// Opens the ordinary transaction used after global tenant admission. Keeping
/// this beside [`create`] makes the authority handoff in test fixtures
/// explicit and prevents a raw pool transaction from silently depending on an
/// owner or BYPASSRLS credential.
#[allow(dead_code)]
pub async fn begin(pool: &PgPool, tenant: TenantId) -> Transaction<'static, Postgres> {
    synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant-scoped test transaction")
}

/// Returns the PostgreSQL backend running this transaction. Concurrency tests
/// use it only with `pg_blocking_pids`, never as application identity.
#[allow(dead_code)]
pub async fn backend_pid(transaction: &mut Transaction<'_, Postgres>) -> i32 {
    sqlx::query_scalar!(r#"select pg_catalog.pg_backend_pid() as "pid!""#)
        .fetch_one(&mut **transaction)
        .await
        .expect("read test transaction backend pid")
}

/// Proves that `waiter_pid` is currently blocked by `holder_pid`.
///
/// The bounded catalogue poll makes the lock relationship the assertion; no
/// wall-clock sleep is used as a proxy for whether a racing statement ran.
#[allow(dead_code)]
pub async fn wait_until_blocked_by(
    observer: &mut Transaction<'_, Postgres>,
    waiter_pid: i32,
    holder_pid: i32,
) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let blocked = sqlx::query_scalar!(
                r#"
                select $2::int = any(pg_catalog.pg_blocking_pids($1::int)) as "blocked!"
                "#,
                waiter_pid,
                holder_pid,
            )
            .fetch_one(&mut **observer)
            .await
            .expect("inspect PostgreSQL blocker graph");
            if blocked {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("waiter did not enter the expected PostgreSQL lock wait");
}

/// Opens the explicit administrator credential used only by serial tamper
/// acceptance. The configured principal and live database identity are proved
/// against the ordinary runtime pool before a test receives the handle.
#[allow(dead_code)]
pub async fn administrator_pool(runtime_pool: &PgPool) -> PgPool {
    let url = private_configuration(
        "SYNVEDA_TEST_ADMIN_DATABASE_URL_FILE",
        "test administrator database URL",
    );
    let roles_json =
        private_configuration("SYNVEDA_DATABASE_ROLES_FILE", "test database role contract");
    let roles = synveda_store::runtime_role::DatabaseRoles::parse_json(&roles_json)
        .expect("parse the isolated test database role contract");
    let options = synveda_store::database_url::parse("SYNVEDA_TEST_ADMIN_DATABASE_URL_FILE", &url)
        .expect("parse the isolated test administrator URL");
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .expect("connect the isolated test administrator");
    let (current_user, session_user, can_login, bypasses_rls): (String, String, bool, bool) =
        sqlx::query_as(
            "select current_user::text, session_user::text, role.rolcanlogin, \
                not pg_catalog.row_security_active('public.scopes'::regclass) \
         from pg_catalog.pg_roles as role where role.rolname = current_user",
        )
        .fetch_one(&pool)
        .await
        .expect("read test administrator identity");
    assert_eq!(
        current_user, session_user,
        "test administrator must not enter through SET ROLE"
    );
    assert!(can_login, "test administrator must be a login role");
    assert!(
        bypasses_rls,
        "test administrator must exercise the explicit RLS-bypass trigger seam"
    );
    assert!(
        roles.runtime().administrators().contains(&current_user),
        "test administrator is outside the configured authority set"
    );
    let runtime_identity = synveda_store::runtime_role::database_identity(runtime_pool)
        .await
        .expect("read runtime test database identity");
    let administrator_identity = synveda_store::runtime_role::database_identity(&pool)
        .await
        .expect("read administrator test database identity");
    assert_eq!(
        runtime_identity, administrator_identity,
        "test administrator and runtime credentials must target one live database"
    );
    pool
}

/// Opens the exact database-owner/migrator credential used only by serial
/// owner-trigger acceptance. The ordinary runtime pool must identify the same
/// live database before the handle is returned.
#[allow(dead_code)]
pub async fn migrator_pool(runtime_pool: &PgPool) -> PgPool {
    let url = private_configuration(
        "SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE",
        "test migrator database URL",
    );
    let roles_json =
        private_configuration("SYNVEDA_DATABASE_ROLES_FILE", "test database role contract");
    let roles = synveda_store::runtime_role::DatabaseRoles::parse_json(&roles_json)
        .expect("parse the isolated test database role contract");
    let options =
        synveda_store::database_url::parse("SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE", &url)
            .expect("parse the isolated test migrator URL");
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .expect("connect the isolated test migrator");
    {
        let mut connection = pool.acquire().await.expect("acquire test migrator");
        synveda_store::runtime_role::initialize_product_session_connection(&mut connection)
            .await
            .expect("normalise the test migrator session");
        synveda_store::runtime_role::verify_migrator_connection(&mut connection, &roles)
            .await
            .expect("verify the exact test migrator authority");
    }
    let runtime_identity = synveda_store::runtime_role::database_identity(runtime_pool)
        .await
        .expect("read runtime test database identity");
    let migrator_identity = synveda_store::runtime_role::database_identity(&pool)
        .await
        .expect("read migrator test database identity");
    assert_eq!(
        runtime_identity, migrator_identity,
        "test migrator and runtime credentials must target one live database"
    );
    pool
}

fn private_configuration(setting: &str, label: &str) -> String {
    let path = std::env::var_os(setting).unwrap_or_else(|| {
        panic!("{setting} is required when DATABASE_URL enables database-backed tests")
    });
    let path = Path::new(&path);
    let metadata =
        std::fs::symlink_metadata(path).unwrap_or_else(|_| panic!("{label} file is unavailable"));
    assert!(
        metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
        "{label} must be a regular non-symlink file"
    );
    assert!(
        (1..=MAX_CONFIGURATION_BYTES).contains(&metadata.len()),
        "{label} must contain 1..={MAX_CONFIGURATION_BYTES} bytes"
    );
    let value =
        std::fs::read_to_string(path).unwrap_or_else(|_| panic!("{label} file must contain UTF-8"));
    let value = value.strip_suffix('\n').unwrap_or(&value);
    assert!(
        !value.is_empty()
            && !value
                .chars()
                .any(|character| matches!(character, '\r' | '\n' | '\0')),
        "{label} must contain exactly one non-empty line"
    );
    value.to_owned()
}
