//! Test-only access to isolated deployment credentials.
//!
//! Middle-layer crates cannot depend on `synveda-store` without violating the
//! production dependency graph. Their database acceptance tests use this
//! small, content-free seam to prove that a separately named file credential
//! has the configured principal and targets the same live database as the
//! ordinary runtime pool before exercising a privileged adversarial case.

use std::path::Path;
use std::str::FromStr as _;

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{ConnectOptions as _, PgConnection, PgPool};

const MAX_CONFIGURATION_BYTES: u64 = 4096;

#[allow(dead_code)]
pub async fn administrator_pool(runtime_pool: &PgPool) -> PgPool {
    let roles = roles_configuration();
    let source = private_configuration(
        "SYNVEDA_TEST_ADMIN_DATABASE_URL_FILE",
        "test administrator database URL",
    );
    let options = PgConnectOptions::from_str(&source)
        .expect("parse the isolated test administrator database URL");
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .expect("connect the isolated test administrator");
    let mut connection = pool.acquire().await.expect("acquire test administrator");
    let (current_user, session_user, can_login, bypasses_rls): (String, String, bool, bool) =
        sqlx::query_as(
            "select current_user::text, session_user::text, role.rolcanlogin, \
                    not pg_catalog.row_security_active('public.scopes'::regclass) \
             from pg_catalog.pg_roles as role where role.rolname = current_user",
        )
        .fetch_one(&mut *connection)
        .await
        .expect("read test administrator identity");
    assert_eq!(
        current_user, session_user,
        "test administrator must not enter through SET ROLE"
    );
    assert!(can_login, "test administrator must be a login role");
    assert!(
        bypasses_rls,
        "test administrator must exercise the explicit RLS-bypass seam"
    );
    let administrators = configured_names(&roles, "administrators");
    assert!(
        administrators.iter().any(|name| name == &current_user),
        "test administrator is outside the configured authority set"
    );
    assert_same_database(runtime_pool, &mut connection).await;
    drop(connection);
    pool
}

#[allow(dead_code)]
pub async fn migrator_connection(runtime_pool: &PgPool) -> PgConnection {
    let roles = roles_configuration();
    let expected = configured_name(&roles, "migrator");
    let source = private_configuration(
        "SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE",
        "test migrator database URL",
    );
    let options =
        PgConnectOptions::from_str(&source).expect("parse the isolated test migrator database URL");
    let mut connection = options
        .connect()
        .await
        .expect("connect the isolated test migrator");
    let facts: (
        String,
        String,
        bool,
        bool,
        bool,
        bool,
        bool,
        bool,
        bool,
        bool,
    ) = sqlx::query_as(
        "select current_user::text, session_user::text, role.rolcanlogin, \
                    role.rolinherit, role.rolsuper, role.rolcreatedb, \
                    role.rolcreaterole, role.rolreplication, role.rolbypassrls, \
                    pg_catalog.pg_get_userbyid(database.datdba) = current_user \
             from pg_catalog.pg_roles as role \
             join pg_catalog.pg_database as database \
               on database.datname = pg_catalog.current_database() \
             where role.rolname = current_user",
    )
    .fetch_one(&mut connection)
    .await
    .expect("read test migrator identity");
    assert_eq!(
        facts.0, expected,
        "test migrator principal differs from configuration"
    );
    assert_eq!(
        facts.0, facts.1,
        "test migrator must not enter through SET ROLE"
    );
    assert!(
        facts.2 && facts.3,
        "test migrator must be an inheriting login role"
    );
    assert!(
        !facts.4 && !facts.5 && !facts.6 && !facts.7 && !facts.8,
        "test migrator must have no cluster-elevated capability"
    );
    assert!(facts.9, "test migrator must own the selected database");
    assert_same_database(runtime_pool, &mut connection).await;
    connection
}

async fn assert_same_database(runtime_pool: &PgPool, privileged: &mut PgConnection) {
    let mut runtime = runtime_pool
        .acquire()
        .await
        .expect("acquire ordinary runtime database");
    let runtime_identity = database_identity(&mut runtime).await;
    let privileged_identity = database_identity(privileged).await;
    assert_eq!(
        runtime_identity, privileged_identity,
        "test runtime and privileged credentials must target one live database"
    );
}

async fn database_identity(connection: &mut PgConnection) -> (String, String, String, String) {
    sqlx::query_as(
        "select pg_catalog.current_database()::text, \
                control.system_identifier::text, database.oid::text, \
                pg_catalog.pg_postmaster_start_time()::text \
         from pg_catalog.pg_database as database \
         cross join pg_catalog.pg_control_system() as control \
         where database.datname = pg_catalog.current_database()",
    )
    .fetch_one(connection)
    .await
    .expect("read test database identity")
}

fn roles_configuration() -> serde_json::Value {
    let source =
        private_configuration("SYNVEDA_DATABASE_ROLES_FILE", "test database role contract");
    serde_json::from_str(&source).expect("parse the isolated test database role contract")
}

fn configured_name<'a>(roles: &'a serde_json::Value, field: &str) -> &'a str {
    roles
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| panic!("test database role contract requires {field}"))
}

fn configured_names<'a>(roles: &'a serde_json::Value, field: &str) -> Vec<&'a str> {
    roles
        .get(field)
        .and_then(serde_json::Value::as_array)
        .unwrap_or_else(|| panic!("test database role contract requires {field}"))
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| panic!("test database role contract has invalid {field}"))
        })
        .collect()
}

fn private_configuration(setting: &str, label: &str) -> String {
    let path = std::env::var_os(setting)
        .unwrap_or_else(|| panic!("{setting} is required for privileged database acceptance"));
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
        std::fs::read_to_string(path).unwrap_or_else(|_| panic!("{label} must contain UTF-8"));
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
