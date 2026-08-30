//! Runtime database-principal verification for product processes.
//!
//! Deployment bootstrap owns login creation. Application processes use this
//! read-only sentinel to refuse owner, superuser, `BYPASSRLS`, non-login or
//! ungranted credentials before they become ready.

use serde::Deserialize;
use sqlx::{Connection, PgConnection, PgPool};
use synveda_types::{Error, Result};

/// Exact request and worker logins which may inherit the data-bearing
/// `synveda_app` capability.
///
/// PostgreSQL 17 records membership options per grantor. The names therefore
/// come from deployment configuration rather than being inferred from the
/// catalog: otherwise a rogue inheriting member added before process boot
/// could become part of the inferred "expected" set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedRuntimeRoles {
    names: [String; 2],
    administrators: Vec<String>,
    administrative_members: Vec<String>,
    administrative_grantors: Vec<String>,
}

impl ExpectedRuntimeRoles {
    /// Builds the closed two-login set used by every runtime sentinel.
    fn new(
        mut names: Vec<String>,
        mut administrators: Vec<String>,
        mut administrative_memberships: Vec<(String, String)>,
    ) -> Result<Self> {
        if names.len() != 2 || names.iter().any(|name| !valid_role_name(name)) {
            return Err(Error::Invalid {
                message: "the runtime database role set must contain exactly two non-empty PostgreSQL role names of at most 63 UTF-8 bytes".to_owned(),
            });
        }
        names.sort_unstable();
        if names[0] == names[1] {
            return Err(Error::Invalid {
                message: "the runtime database role set must contain two distinct roles".to_owned(),
            });
        }
        if administrators.is_empty()
            || administrators.len() > 8
            || administrators.iter().any(|name| !valid_role_name(name))
        {
            return Err(Error::Invalid {
                message: "the database administrator set must contain between one and eight PostgreSQL role names".to_owned(),
            });
        }
        administrators.sort_unstable();
        if administrators.windows(2).any(|roles| roles[0] == roles[1]) {
            return Err(Error::Invalid {
                message: "the database administrator set must not contain duplicates".to_owned(),
            });
        }
        if administrators.iter().any(|name| names.contains(name)) {
            return Err(Error::Invalid {
                message: "runtime and database administrator roles must be distinct".to_owned(),
            });
        }
        if administrative_memberships.len() > 8
            || administrative_memberships.iter().any(|(member, grantor)| {
                !valid_role_name(member)
                    || !valid_role_name(grantor)
                    || member == grantor
                    || !administrators.contains(member)
            })
        {
            return Err(Error::Invalid {
                message: "the administrative membership set may contain at most eight exact, non-self member/grantor pairs whose members are configured database administrators"
                    .to_owned(),
            });
        }
        administrative_memberships.sort_unstable();
        if administrative_memberships
            .windows(2)
            .any(|pairs| pairs[0] == pairs[1])
        {
            return Err(Error::Invalid {
                message: "the administrative membership set must not contain duplicates".to_owned(),
            });
        }
        let (administrative_members, administrative_grantors) =
            administrative_memberships.into_iter().unzip();
        let names: [String; 2] = names.try_into().map_err(|_| Error::Invalid {
            message: "the runtime database role set must contain exactly two roles".to_owned(),
        })?;
        Ok(Self {
            names,
            administrators,
            administrative_members,
            administrative_grantors,
        })
    }

    /// Sorted role names for static SQL array parameters.
    pub fn as_slice(&self) -> &[String] {
        &self.names
    }

    /// Whether one process login belongs to the deployment's closed set.
    pub fn contains(&self, name: &str) -> bool {
        self.names.iter().any(|candidate| candidate == name)
    }

    /// Explicitly trusted provider/operator role managers. PostgreSQL ADMIN
    /// OPTION lets these principals re-grant a data path, so this allowlist is
    /// trusted authority and provenance, not an isolation boundary.
    pub fn administrators(&self) -> &[String] {
        &self.administrators
    }

    fn administrative_members(&self) -> &[String] {
        &self.administrative_members
    }

    fn administrative_grantors(&self) -> &[String] {
        &self.administrative_grantors
    }
}

/// Provider-neutral deployment role contract shared by Compose, direct
/// binaries and later Kubernetes packaging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseRoles {
    migrator: String,
    gateway: String,
    worker: String,
    runtime: ExpectedRuntimeRoles,
    forbidden_databases: Vec<String>,
    isolated_peer_roles: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DatabaseRolesJson {
    migrator: String,
    gateway: String,
    worker: String,
    administrators: Vec<String>,
    administrative_memberships: Vec<AdministrativeMembershipJson>,
    forbidden_databases: Vec<String>,
    isolated_peer_roles: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdministrativeMembershipJson {
    member: String,
    grantor: String,
}

impl DatabaseRoles {
    /// Builds the closed role contract from deployment-supplied role names.
    pub fn new(
        migrator: String,
        gateway: String,
        worker: String,
        administrators: Vec<String>,
        administrative_memberships: Vec<(String, String)>,
        mut forbidden_databases: Vec<String>,
        mut isolated_peer_roles: Vec<String>,
    ) -> Result<Self> {
        if !valid_role_name(&migrator) || !valid_role_name(&gateway) || !valid_role_name(&worker) {
            return Err(Error::Invalid {
                message:
                    "every database role must use between one and 63 UTF-8 bytes and contain no NUL"
                        .to_owned(),
            });
        }
        if migrator == gateway || migrator == worker || gateway == worker {
            return Err(Error::Invalid {
                message: "migrator, gateway and worker database roles must be distinct".to_owned(),
            });
        }
        if migrator == "synveda_app"
            || gateway == "synveda_app"
            || worker == "synveda_app"
            || administrators.iter().any(|role| role == "synveda_app")
        {
            return Err(Error::Invalid {
                message: "the fixed synveda_app capability role cannot be used as a login, migrator or database administrator"
                    .to_owned(),
            });
        }
        if administrative_memberships.iter().any(|(member, grantor)| {
            [
                "synveda_app",
                migrator.as_str(),
                gateway.as_str(),
                worker.as_str(),
            ]
            .contains(&member.as_str())
                || [
                    "synveda_app",
                    migrator.as_str(),
                    gateway.as_str(),
                    worker.as_str(),
                ]
                .contains(&grantor.as_str())
        }) {
            return Err(Error::Invalid {
                message: "administrative membership members and grantors must be distinct from every protected application role"
                    .to_owned(),
            });
        }
        let runtime = ExpectedRuntimeRoles::new(
            vec![gateway.clone(), worker.clone()],
            administrators,
            administrative_memberships,
        )?;
        if runtime
            .administrators()
            .iter()
            .any(|administrator| administrator == &migrator)
        {
            return Err(Error::Invalid {
                message: "the migrator and database administrator roles must be distinct"
                    .to_owned(),
            });
        }
        if forbidden_databases.is_empty()
            || forbidden_databases.len() > 8
            || forbidden_databases
                .iter()
                .any(|database| !valid_database_name(database))
        {
            return Err(Error::Invalid {
                message: "the forbidden database set must contain between one and eight non-empty PostgreSQL database names of at most 63 UTF-8 bytes".to_owned(),
            });
        }
        forbidden_databases.sort_unstable();
        if forbidden_databases
            .windows(2)
            .any(|databases| databases[0] == databases[1])
        {
            return Err(Error::Invalid {
                message: "the forbidden database set must not contain duplicates".to_owned(),
            });
        }
        if isolated_peer_roles.len() > 8
            || isolated_peer_roles
                .iter()
                .any(|role| !valid_role_name(role))
        {
            return Err(Error::Invalid {
                message:
                    "the isolated peer role set must contain at most eight PostgreSQL role names"
                        .to_owned(),
            });
        }
        isolated_peer_roles.sort_unstable();
        if isolated_peer_roles
            .windows(2)
            .any(|roles| roles[0] == roles[1])
        {
            return Err(Error::Invalid {
                message: "the isolated peer role set must not contain duplicates".to_owned(),
            });
        }
        let protected_roles = [
            "synveda_app",
            migrator.as_str(),
            gateway.as_str(),
            worker.as_str(),
        ];
        if isolated_peer_roles.iter().any(|peer| {
            protected_roles.contains(&peer.as_str())
                || runtime
                    .administrators()
                    .iter()
                    .chain(runtime.administrative_members().iter())
                    .chain(runtime.administrative_grantors().iter())
                    .any(|administrator| administrator == peer)
        }) {
            return Err(Error::Invalid {
                message:
                    "isolated peer roles must be distinct from application and administrative roles"
                        .to_owned(),
            });
        }
        Ok(Self {
            migrator,
            gateway,
            worker,
            runtime,
            forbidden_databases,
            isolated_peer_roles,
        })
    }

    /// Parses the closed JSON object without ever including configured names
    /// in an error. Provider-assigned login names are supported within the
    /// same bounded PostgreSQL identifier vocabulary as the reference roles.
    pub fn parse_json(value: &str) -> Result<Self> {
        let raw: DatabaseRolesJson = serde_json::from_str(value).map_err(|_| Error::Invalid {
            message: "SYNVEDA_DATABASE_ROLES must be a JSON object with migrator, gateway and worker strings, administrators, forbidden_databases and isolated_peer_roles string arrays, and an administrative_memberships array of exact member/grantor objects".to_owned(),
        })?;
        Self::new(
            raw.migrator,
            raw.gateway,
            raw.worker,
            raw.administrators,
            raw.administrative_memberships
                .into_iter()
                .map(|membership| (membership.member, membership.grantor))
                .collect(),
            raw.forbidden_databases,
            raw.isolated_peer_roles,
        )
    }

    /// Exact migration owner.
    pub fn migrator(&self) -> &str {
        &self.migrator
    }

    /// Exact request-plane login.
    pub fn gateway(&self) -> &str {
        &self.gateway
    }

    /// Exact worker-plane login.
    pub fn worker(&self) -> &str {
        &self.worker
    }

    /// Closed set allowed to inherit `synveda_app`.
    pub fn runtime(&self) -> &ExpectedRuntimeRoles {
        &self.runtime
    }

    /// Exact peer/maintenance databases which no Synveda role may connect to.
    /// Each deployment topology supplies its own closed existing set; a typo
    /// or missing declared database fails the authority proof.
    pub fn forbidden_databases(&self) -> &[String] {
        &self.forbidden_databases
    }

    /// Exact non-Synveda roles which must have no effective CONNECT authority
    /// into the selected Synveda database. Bundled identity uses this to keep
    /// its login isolated even if a later inherited grant changes authority.
    pub fn isolated_peer_roles(&self) -> &[String] {
        &self.isolated_peer_roles
    }

    fn trusted_extension_owners(&self) -> Vec<String> {
        let mut owners = self.runtime.administrators().to_vec();
        owners.extend(self.runtime.administrative_grantors().iter().cloned());
        owners.sort_unstable();
        owners.dedup();
        owners
    }
}

fn valid_role_name(value: &str) -> bool {
    !value.is_empty() && value.len() <= 63 && !value.contains('\0')
}

fn valid_database_name(value: &str) -> bool {
    !value.is_empty() && value.len() <= 63 && !value.contains('\0')
}

fn transient_authority_sqlstate(code: &str) -> bool {
    code.starts_with("08")
        || code.starts_with("40")
        || code.starts_with("53")
        || code.starts_with("58")
        || code.starts_with("XX")
        || matches!(code, "55P03" | "57014" | "57P01" | "57P02" | "57P03")
}

/// Fixed authority queries distinguish a missing database from a database
/// which permanently refuses the proof. Only explicit transport/resource/
/// cancellation classes remain retryable; permission and catalogue-shape
/// failures close the process with one content-free SQLSTATE.
fn authority_sql_error(context: &'static str, error: sqlx::Error) -> Error {
    let sqlstate = error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .filter(|code| code.len() == 5 && code.bytes().all(|byte| byte.is_ascii_alphanumeric()))
        .map(|code| code.into_owned());
    let transient = match &error {
        sqlx::Error::Database(_) => sqlstate
            .as_deref()
            .is_some_and(transient_authority_sqlstate),
        sqlx::Error::Io(_)
        | sqlx::Error::Tls(_)
        | sqlx::Error::Protocol(_)
        | sqlx::Error::PoolTimedOut
        | sqlx::Error::PoolClosed
        | sqlx::Error::WorkerCrashed
        | sqlx::Error::BeginFailed => true,
        _ => false,
    };
    let suffix = sqlstate
        .as_deref()
        .map_or_else(String::new, |code| format!(" (SQLSTATE {code})"));
    if transient {
        Error::Storage {
            message: format!("{context}: database authority is temporarily unavailable{suffix}"),
        }
    } else {
        Error::Invalid {
            message: format!("{context}: fixed database authority proof was refused{suffix}"),
        }
    }
}

/// Verified current PostgreSQL login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRuntimeRole {
    /// Exact database principal selected by the process DSN.
    pub name: String,
}

/// Runtime-visible identity of one PostgreSQL database target.
///
/// Deployment bootstrap compares this value across administrative, gateway
/// and worker connections so individually valid credentials cannot silently
/// split one installation across different live primary instances or
/// databases. The postmaster start marker distinguishes separately started
/// writable forks that retain the same system identifier and database OID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseIdentity {
    /// Database selected by the connection.
    pub database: String,
    /// PostgreSQL cluster system identifier, read through the ordinary login.
    pub cluster_system_identifier: String,
    /// OID of the selected database inside that cluster.
    pub database_oid: i64,
    /// Start time of the live postmaster serving this connection.
    pub postmaster_started_at: chrono::DateTime<chrono::Utc>,
}

/// Reads the target identity from a writable primary without disclosing a
/// configured URL or credential. A hot standby shares its cluster identifier
/// and database OID with the primary, so identity alone is not sufficient for
/// a process that owns governed mutations.
#[tracing::instrument(name = "store.runtime_role.database_identity", skip_all, err(Display))]
pub async fn database_identity(pool: &PgPool) -> Result<DatabaseIdentity> {
    let mut connection = pool.acquire().await.map_err(|error| {
        authority_sql_error("acquire runtime database target connection", error)
    })?;
    database_identity_connection(&mut connection).await
}

/// Connection-scoped form used by the gateway's indivisible authority proof.
pub async fn database_identity_connection(
    connection: &mut PgConnection,
) -> Result<DatabaseIdentity> {
    let identity = sqlx::query!(
        r#"select pg_catalog.current_database() as "database!",
                  control.system_identifier::text as "cluster_system_identifier!",
                  database.oid::bigint as "database_oid!",
                  pg_catalog.pg_postmaster_start_time() as "postmaster_started_at!",
                  pg_catalog.pg_is_in_recovery() as "in_recovery!",
                  pg_catalog.current_setting('transaction_read_only')::boolean
                    as "transaction_read_only!"
             from pg_catalog.pg_control_system() as control
             join pg_catalog.pg_database as database
               on database.datname = pg_catalog.current_database()"#,
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| authority_sql_error("read runtime database target identity", error))?;
    if identity.in_recovery || identity.transaction_read_only {
        return Err(Error::Invalid {
            message: "runtime database target must be a writable PostgreSQL primary".to_owned(),
        });
    }
    Ok(DatabaseIdentity {
        database: identity.database,
        cluster_system_identifier: identity.cluster_system_identifier,
        database_oid: identity.database_oid,
        postmaster_started_at: identity.postmaster_started_at,
    })
}

/// Reads the OID of one deployment-declared peer database on the same cluster
/// connection used for an authority snapshot. Deployment preflight uses this
/// to compare a content-free bootstrap witness without receiving the peer's
/// login or credentials.
pub async fn peer_database_oid_connection(
    connection: &mut PgConnection,
    peer_database: &str,
) -> Result<Option<i64>> {
    sqlx::query_scalar!(
        r#"select database.oid::bigint as "oid!"
             from pg_catalog.pg_database as database
            where database.datname = $1"#,
        peer_database,
    )
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| authority_sql_error("read deployment peer database identity", error))
}

/// Refuses authority-sensitive database defaults and proves the effective
/// settings on the same session that reads the role catalog.
pub async fn verify_session_safety_connection(connection: &mut PgConnection) -> Result<()> {
    let safe = sqlx::query_scalar!(
        r#"select (
              current_user = session_user
              and pg_catalog.current_setting('row_security') = 'on'
              and pg_catalog.current_setting('session_replication_role') = 'origin'
              and pg_catalog.current_setting('transaction_read_only') = 'off'
              and pg_catalog.current_setting('synchronous_commit') = 'on'
              and pg_catalog.current_setting('search_path') = 'public'
              and pg_catalog.current_schemas(false) in (
                    array['public'::name],
                    array[]::name[]
                  )
              and coalesce(
                    nullif(pg_catalog.current_setting('synveda.tenant_id', true), ''),
                    ''
                  ) = ''
              and coalesce(
                    nullif(pg_catalog.current_setting('synveda.knowledge_erasure', true), ''),
                    'off'
                  ) = 'off'
              and coalesce(
                    nullif(pg_catalog.current_setting('synveda.retention_purge', true), ''),
                    'off'
                  ) = 'off'
              and not exists (
                select 1
                  from pg_catalog.pg_db_role_setting as settings
                  left join pg_catalog.pg_roles as role on role.oid = settings.setrole
                 where settings.setdatabase in (
                         0,
                         (select database.oid from pg_catalog.pg_database as database
                           where database.datname = pg_catalog.current_database())
                       )
                   and (settings.setrole = 0 or role.rolname = current_user)
              )
            ) as "safe!""#,
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| authority_sql_error("verify runtime database session settings", error))?;
    if !safe {
        return Err(Error::Invalid {
            message: "the database session must name only the public schema (whether or not pre-migration USAGE exists), with row security and durable commit semantics, a writable transaction, no maintenance/tenant GUC, and no applicable role or database setting".to_owned(),
        });
    }
    Ok(())
}

/// Establishes the fixed product-session vocabulary on a newly opened
/// gateway, worker, preflight or migration connection. The catalogue
/// sentinel still verifies both effective values and persistent defaults.
pub async fn initialize_product_session_connection(connection: &mut PgConnection) -> Result<()> {
    sqlx::query!(
        r#"select pg_catalog.set_config('search_path', 'public', false) as "search_path!",
                  pg_catalog.set_config('synveda.tenant_id', '', false) as "tenant_id!",
                  pg_catalog.set_config('synveda.knowledge_erasure', 'off', false)
                    as "knowledge_erasure!",
                  pg_catalog.set_config('synveda.retention_purge', 'off', false)
                    as "retention_purge!""#,
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| authority_sql_error("initialize product database session", error))?;
    verify_session_safety_connection(connection).await
}

/// Proves that one physical pool connection selected the configured login
/// without relying on the URL parser alone.
pub async fn verify_selected_principal_connection(
    connection: &mut PgConnection,
    expected_principal: &str,
) -> Result<()> {
    let selected = sqlx::query_scalar!(
        r#"select current_user = session_user and current_user = $1 as "selected!""#,
        expected_principal,
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| authority_sql_error("verify selected database principal", error))?;
    if !selected {
        return Err(Error::Invalid {
            message: "the physical database connection did not select the configured login"
                .to_owned(),
        });
    }
    Ok(())
}

/// Makes the caller-owned SQLx transaction the repeatable-read snapshot used
/// by one authority proof. The caller retains RAII rollback on cancellation.
pub async fn configure_authority_snapshot_connection(connection: &mut PgConnection) -> Result<()> {
    sqlx::query!("set transaction isolation level repeatable read")
        .execute(&mut *connection)
        .await
        .map_err(|error| authority_sql_error("configure database authority snapshot", error))?;
    sqlx::query!("set local statement_timeout = '4s'")
        .execute(&mut *connection)
        .await
        .map_err(|error| authority_sql_error("bound database authority statement time", error))?;
    sqlx::query!("set local lock_timeout = '1s'")
        .execute(&mut *connection)
        .await
        .map(|_| ())
        .map_err(|error| authority_sql_error("bound database authority lock wait", error))
}

/// Privilege-bearing catalogs outside the application schema. None of the
/// capability, runtime or migrator roles needs these grants. Explicit PUBLIC
/// grants are also refused except for language ACLs, whose built-in defaults
/// include PUBLIC USAGE for PL/pgSQL.
async fn verify_no_global_or_default_acl(
    connection: &mut PgConnection,
    roles: &[String],
) -> Result<()> {
    let unsafe_acl = sqlx::query_scalar!(
        r#"select exists (
              select 1
                from pg_catalog.pg_largeobject_metadata as object,
                     lateral pg_catalog.aclexplode(object.lomacl) as acl
               where acl.grantee = 0
                  or acl.grantee in (
                    select role.oid from pg_catalog.pg_roles as role
                     where role.rolname = any($1::text[])
                  )
              union all
              select 1
                from pg_catalog.pg_foreign_data_wrapper as object,
                     lateral pg_catalog.aclexplode(object.fdwacl) as acl
               where acl.grantee = 0
                  or acl.grantee in (
                    select role.oid from pg_catalog.pg_roles as role
                     where role.rolname = any($1::text[])
                  )
              union all
              select 1
                from pg_catalog.pg_foreign_server as object,
                     lateral pg_catalog.aclexplode(object.srvacl) as acl
               where acl.grantee = 0
                  or acl.grantee in (
                    select role.oid from pg_catalog.pg_roles as role
                     where role.rolname = any($1::text[])
                  )
              union all
              select 1
                from pg_catalog.pg_language as object,
                     lateral pg_catalog.aclexplode(object.lanacl) as acl
               where acl.grantee in (
                 select role.oid from pg_catalog.pg_roles as role
                  where role.rolname = any($1::text[])
               )
              union all
              select 1
                from pg_catalog.pg_tablespace as object,
                     lateral pg_catalog.aclexplode(object.spcacl) as acl
               where acl.grantee = 0
                  or acl.grantee in (
                    select role.oid from pg_catalog.pg_roles as role
                     where role.rolname = any($1::text[])
                  )
              union all
              select 1
                from pg_catalog.pg_parameter_acl as object,
                     lateral pg_catalog.aclexplode(object.paracl) as acl
               where acl.grantee = 0
                  or acl.grantee in (
                    select role.oid from pg_catalog.pg_roles as role
                     where role.rolname = any($1::text[])
                  )
              union all
              select 1
                from pg_catalog.pg_default_acl as defaults
               where defaults.defaclrole in (
                       select role.oid from pg_catalog.pg_roles as role
                        where role.rolname = any($1::text[])
                     )
                  or exists (
                    select 1
                      from pg_catalog.aclexplode(defaults.defaclacl) as acl
                     where acl.grantee in (
                       select role.oid from pg_catalog.pg_roles as role
                        where role.rolname = any($1::text[])
                     )
                  )
            ) as "unsafe!""#,
        roles,
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| authority_sql_error("verify global and default database ACLs", error))?;
    if unsafe_acl {
        return Err(Error::Invalid {
            message: "application database roles must have no global-object or default-privilege ACLs, and global objects must not carry explicit PUBLIC grants".to_owned(),
        });
    }
    Ok(())
}

/// Refuses effective CONNECT authority across both directions of the
/// deployment's exact database-isolation boundary. PostgreSQL grants CONNECT
/// to PUBLIC by default and inherited memberships can silently reopen it, so
/// direct ACL inspection is insufficient. Missing configured database or role
/// names are refused rather than inferred from the catalogue.
async fn verify_forbidden_database_connect(
    connection: &mut PgConnection,
    database_roles: &DatabaseRoles,
) -> Result<()> {
    let protected_roles = vec![
        "synveda_app".to_owned(),
        database_roles.migrator().to_owned(),
        database_roles.gateway().to_owned(),
        database_roles.worker().to_owned(),
    ];
    let safe = sqlx::query_scalar!(
        r#"select (
              select count(*)::integer
                from pg_catalog.pg_database as database
               where database.datname = any($1::text[])
            ) = pg_catalog.cardinality($1::text[])
            and (
              select count(*)::integer
                from pg_catalog.pg_roles as role
               where role.rolname = any($3::text[])
            ) = pg_catalog.cardinality($3::text[])
            and not exists (
              select 1
                from pg_catalog.pg_database as database
                join pg_catalog.pg_roles as role
                  on role.rolname = any($2::text[])
               where database.datname = any($1::text[])
                 and pg_catalog.has_database_privilege(
                       role.oid,
                       database.oid,
                       'CONNECT'
                     )
            )
            and not exists (
              select 1
                from pg_catalog.pg_roles as role
               where role.rolname = any($3::text[])
                 and pg_catalog.has_database_privilege(
                       role.oid,
                       pg_catalog.current_database(),
                       'CONNECT'
                     )
            ) as "safe!""#,
        database_roles.forbidden_databases(),
        &protected_roles,
        database_roles.isolated_peer_roles(),
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| {
        authority_sql_error("verify isolated peer database CONNECT authority", error)
    })?;
    if !safe {
        return Err(Error::Invalid {
            message: "every configured isolated peer database and role must exist, application roles must have no effective CONNECT authority into peer databases, and peer roles must have no effective CONNECT authority into Synveda".to_owned(),
        });
    }
    Ok(())
}

/// Verifies the schema-owned capability role inherited by every runtime
/// login. A safe login is not sufficient if `SET ROLE synveda_app` could
/// acquire ownership or elevated cluster capabilities.
#[tracing::instrument(
    name = "store.runtime_role.verify_capability_role",
    skip_all,
    err(Display)
)]
pub async fn verify_capability_role(pool: &PgPool, database_roles: &DatabaseRoles) -> Result<()> {
    let mut connection = pool.acquire().await.map_err(|error| {
        authority_sql_error("acquire database capability verification connection", error)
    })?;
    initialize_product_session_connection(&mut connection).await?;
    let mut authority = connection.begin().await.map_err(|error| {
        authority_sql_error("begin database capability authority snapshot", error)
    })?;
    configure_authority_snapshot_connection(&mut authority).await?;
    verify_capability_role_connection(&mut authority, database_roles).await?;
    authority.commit().await.map_err(|error| {
        authority_sql_error("finish database capability authority snapshot", error)
    })
}

/// Connection-scoped capability, expected-runtime and application-ACL proof.
pub async fn verify_capability_role_connection(
    connection: &mut PgConnection,
    database_roles: &DatabaseRoles,
) -> Result<()> {
    verify_session_safety_connection(connection).await?;
    verify_administrative_membership_catalog(connection, database_roles).await?;
    verify_capability_shape_connection(connection, database_roles).await?;
    verify_expected_runtime_catalog(connection, database_roles.runtime()).await?;
    verify_expected_migrator_catalog(connection, database_roles).await?;
    verify_application_acl(connection, database_roles).await?;
    verify_routine_catalog_fingerprint(connection, database_roles.migrator()).await?;
    verify_trigger_catalog_fingerprint(connection, database_roles.migrator()).await?;
    verify_rls_catalog_fingerprint(connection).await?;
    verify_tenant_helper_definition(connection, database_roles.migrator()).await?;
    let mut protected_roles = Vec::with_capacity(4);
    protected_roles.push("synveda_app".to_owned());
    protected_roles.push(database_roles.migrator().to_owned());
    protected_roles.extend(database_roles.runtime().as_slice().iter().cloned());
    verify_forbidden_database_connect(connection, database_roles).await?;
    verify_no_global_or_default_acl(connection, &protected_roles).await
}

/// Pre-migration role and cluster authority proof. The application ACL does
/// not exist on a clean database yet, so deployment preflight must use this
/// boundary and the post-migration product sentinel must use the full proof.
pub async fn verify_capability_prerequisites_connection(
    connection: &mut PgConnection,
    database_roles: &DatabaseRoles,
) -> Result<()> {
    verify_session_safety_connection(connection).await?;
    verify_administrative_membership_catalog(connection, database_roles).await?;
    verify_capability_shape_connection(connection, database_roles).await?;
    verify_expected_runtime_catalog(connection, database_roles.runtime()).await?;
    verify_expected_migrator_catalog(connection, database_roles).await?;
    let mut protected_roles = Vec::with_capacity(4);
    protected_roles.push("synveda_app".to_owned());
    protected_roles.push(database_roles.migrator().to_owned());
    protected_roles.extend(database_roles.runtime().as_slice().iter().cloned());
    verify_forbidden_database_connect(connection, database_roles).await?;
    verify_no_global_or_default_acl(connection, &protected_roles).await
}

/// Proves that provider-created ADMIN-only grants into every protected role
/// are the exact configured member/grantor pairs. The role-specific sentinels
/// reject unexpected rows; this symmetric check also rejects a missing
/// configured row, which an allowlist alone cannot detect.
async fn verify_administrative_membership_catalog(
    connection: &mut PgConnection,
    database_roles: &DatabaseRoles,
) -> Result<()> {
    let protected_roles = vec![
        "synveda_app".to_owned(),
        database_roles.migrator().to_owned(),
        database_roles.gateway().to_owned(),
        database_roles.worker().to_owned(),
    ];
    let safe = sqlx::query_scalar!(
        r#"select not exists (
              select 1
                from pg_catalog.pg_roles as protected
                cross join pg_catalog.unnest($2::text[]) with ordinality
                  as expected_member(role_name, position)
                join pg_catalog.unnest($3::text[]) with ordinality
                  as expected_grantor(role_name, position)
                  using (position)
               where protected.rolname = any($1::text[])
                 and not exists (
                   select 1
                     from pg_catalog.pg_auth_members as membership
                     join pg_catalog.pg_roles as member
                       on member.oid = membership.member
                     join pg_catalog.pg_roles as grantor
                       on grantor.oid = membership.grantor
                    where membership.roleid = protected.oid
                      and member.rolname = expected_member.role_name
                      and grantor.rolname = expected_grantor.role_name
                      and membership.admin_option
                      and not membership.inherit_option
                      and not membership.set_option
                 )
            ) and not exists (
              select 1
                from pg_catalog.pg_auth_members as membership
                join pg_catalog.pg_roles as protected
                  on protected.oid = membership.roleid
                join pg_catalog.pg_roles as member
                  on member.oid = membership.member
                join pg_catalog.pg_roles as grantor
                  on grantor.oid = membership.grantor
               where protected.rolname = any($1::text[])
                 and membership.admin_option
                 and not (
                   not membership.inherit_option
                   and not membership.set_option
                   and exists (
                     select 1
                       from pg_catalog.unnest($2::text[]) with ordinality
                         as expected_member(role_name, position)
                       join pg_catalog.unnest($3::text[]) with ordinality
                         as expected_grantor(role_name, position)
                         using (position)
                      where expected_member.role_name = member.rolname
                        and expected_grantor.role_name = grantor.rolname
                   )
                 )
            ) and not exists (
              select 1
                from pg_catalog.pg_auth_members as membership
                join pg_catalog.pg_roles as grantor
                  on grantor.oid = membership.grantor
               where grantor.rolname = any($1::text[])
            ) as "safe!""#,
        &protected_roles,
        database_roles.runtime().administrative_members(),
        database_roles.runtime().administrative_grantors(),
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| {
        authority_sql_error("verify configured administrative role memberships", error)
    })?;
    if !safe {
        return Err(Error::Invalid {
            message: "every protected database role must carry exactly the configured provider ADMIN-only member/grantor pairs"
                .to_owned(),
        });
    }
    Ok(())
}

/// Reset-specific cluster proof which is valid whether the named product
/// database exists or is absent. Local target objects are proved separately
/// through the migrator before an existing database may be destroyed.
pub(crate) async fn verify_reset_cluster_prerequisites_connection(
    connection: &mut PgConnection,
    target_database: &str,
    database_roles: &DatabaseRoles,
) -> Result<()> {
    verify_administrative_membership_catalog(connection, database_roles).await?;
    let protected_roles = vec![
        "synveda_app".to_owned(),
        database_roles.migrator().to_owned(),
        database_roles.gateway().to_owned(),
        database_roles.worker().to_owned(),
    ];
    let safe = sqlx::query_scalar!(
        r#"with target as (
              select database.oid, database.datacl
                from pg_catalog.pg_database as database
                join pg_catalog.pg_roles as owner on owner.oid = database.datdba
               where database.datname = $1
                 and owner.rolname = $2
                 and database.datallowconn
                 and not database.datistemplate
                 and not database.dathasloginevt
                 and database.datconnlimit = -1
                 and database.encoding = pg_catalog.pg_char_to_encoding('UTF8')
            ), protected as (
              select role.*
                from pg_catalog.pg_roles as role
               where role.rolname = any($8::text[])
            )
            select current_user = session_user
              and current_user = any($5::text[])
              and exists (
                select 1
                  from pg_catalog.pg_roles as administrator
                 where administrator.rolname = current_user
                   and administrator.rolsuper
              )
              and not pg_catalog.pg_is_in_recovery()
              and pg_catalog.current_setting('transaction_read_only') = 'off'
              and (
                select count(*) = 4
                   and coalesce(pg_catalog.bool_and(
                     role.rolinherit
                     and not role.rolsuper
                     and not role.rolcreatedb
                     and not role.rolcreaterole
                     and not role.rolreplication
                     and not role.rolbypassrls
                     and role.rolconnlimit = -1
                     and (
                       role.rolname = 'synveda_app'
                       and not role.rolcanlogin
                       and role.rolvaliduntil is null
                       or role.rolname <> 'synveda_app'
                       and role.rolcanlogin
                       and (
                         role.rolvaliduntil is null
                         or role.rolvaliduntil > pg_catalog.statement_timestamp()
                       )
                     )
                   ), false)
                  from protected as role
              )
              and (
                select count(*)::integer
                  from pg_catalog.pg_roles as administrator
                 where administrator.rolname = any($5::text[])
              ) = pg_catalog.cardinality($5::text[])
              and (
                not exists (
                  select 1 from pg_catalog.pg_database where datname = $1
                )
                or (select count(*) from target) = 1
              )
              and not exists (
                select 1
                  from pg_catalog.pg_database as database
                  join protected as owner on owner.oid = database.datdba
                 where not (
                   database.datname = $1
                   and owner.rolname = $2
                   and exists (select 1 from target where target.oid = database.oid)
                 )
              )
              and not exists (
                select 1
                  from pg_catalog.pg_db_role_setting as setting
                 where setting.setrole in (select role.oid from protected as role)
              )
              and (
                select count(*)
                  from pg_catalog.pg_auth_members as membership
                  join pg_catalog.pg_roles as granted on granted.oid = membership.roleid
                  join pg_catalog.pg_roles as member on member.oid = membership.member
                  join pg_catalog.pg_roles as grantor on grantor.oid = membership.grantor
                 where granted.rolname = 'synveda_app'
                   and member.rolname in ($3, $4)
                   and grantor.rolname = any($5::text[])
                   and not membership.admin_option
                   and membership.inherit_option
                   and membership.set_option
              ) = 2
              and not exists (
                select 1
                  from pg_catalog.pg_auth_members as membership
                  join pg_catalog.pg_roles as granted on granted.oid = membership.roleid
                  join pg_catalog.pg_roles as member on member.oid = membership.member
                  join pg_catalog.pg_roles as grantor on grantor.oid = membership.grantor
                 where (
                     granted.rolname = any($8::text[])
                     or member.rolname = any($8::text[])
                   )
                   and not (
                     granted.rolname = 'synveda_app'
                     and member.rolname in ($3, $4)
                     and grantor.rolname = any($5::text[])
                     and not membership.admin_option
                     and membership.inherit_option
                     and membership.set_option
                     or granted.rolname = any($8::text[])
                     and membership.admin_option
                     and not membership.inherit_option
                     and not membership.set_option
                     and exists (
                       select 1
                         from pg_catalog.unnest($6::text[]) with ordinality
                           as expected_member(role_name, position)
                         join pg_catalog.unnest($7::text[]) with ordinality
                           as expected_grantor(role_name, position)
                           using (position)
                        where expected_member.role_name = member.rolname
                          and expected_grantor.role_name = grantor.rolname
                     )
                   )
              )
              and not exists (
                select 1
                  from target,
                       lateral pg_catalog.aclexplode(
                         coalesce(
                           target.datacl,
                           pg_catalog.acldefault(
                             'd',
                             (select role.oid from pg_catalog.pg_roles as role
                               where role.rolname = $2)
                           )
                         )
                       ) as acl
                 where not (
                   acl.grantee = (
                     select role.oid from pg_catalog.pg_roles as role
                      where role.rolname = $2
                   )
                   and acl.grantor = acl.grantee
                   and acl.privilege_type in ('CREATE', 'CONNECT', 'TEMPORARY')
                   and not acl.is_grantable
                   or acl.grantee in (
                     select role.oid from pg_catalog.pg_roles as role
                      where role.rolname in ($3, $4)
                   )
                   and acl.grantor = (
                     select role.oid from pg_catalog.pg_roles as role
                      where role.rolname = $2
                   )
                   and acl.privilege_type = 'CONNECT'
                   and not acl.is_grantable
                   or acl.grantee in (
                     select role.oid from pg_catalog.pg_roles as role
                      where role.rolname = any($5::text[])
                   )
                   and acl.grantor = (
                     select role.oid from pg_catalog.pg_roles as role
                      where role.rolname = $2
                   )
                   and acl.privilege_type = 'CONNECT'
                   and not acl.is_grantable
                 )
              )
              and (
                select count(*)
                  from target,
                       lateral pg_catalog.aclexplode(target.datacl) as acl
                 where acl.grantee = (
                         select role.oid from pg_catalog.pg_roles as role
                          where role.rolname = $2
                       )
                   and acl.grantor = acl.grantee
                   and acl.privilege_type in ('CREATE', 'CONNECT', 'TEMPORARY')
                   and not acl.is_grantable
              ) = case when exists (select 1 from target) then 3 else 0 end
              and (
                select count(*)
                  from target,
                       lateral pg_catalog.aclexplode(target.datacl) as acl
                 where acl.grantee in (
                         select role.oid from pg_catalog.pg_roles as role
                          where role.rolname in ($3, $4)
                       )
                   and acl.grantor = (
                         select role.oid from pg_catalog.pg_roles as role
                          where role.rolname = $2
                       )
                   and acl.privilege_type = 'CONNECT'
                   and not acl.is_grantable
              ) = case when exists (select 1 from target) then 2 else 0 end
              and (
                select count(*)
                  from target,
                       lateral pg_catalog.aclexplode(target.datacl) as acl
                 where acl.grantee in (
                         select role.oid from pg_catalog.pg_roles as role
                          where role.rolname = any($5::text[])
                       )
                   and acl.grantor = (
                         select role.oid from pg_catalog.pg_roles as role
                          where role.rolname = $2
                       )
                   and acl.privilege_type = 'CONNECT'
                   and not acl.is_grantable
              ) = case
                    when exists (select 1 from target)
                    then pg_catalog.cardinality($5::text[])
                    else 0
                  end
              and not exists (
                select 1
                  from pg_catalog.pg_database as database,
                       lateral pg_catalog.aclexplode(database.datacl) as acl
                 where database.datname <> $1
                   and acl.grantee in (select role.oid from protected as role)
              )
              and not exists (
                select 1
                  from pg_catalog.pg_shdepend as dependency
                 where dependency.refclassid = 'pg_catalog.pg_authid'::regclass
                   and dependency.refobjid in (select role.oid from protected as role)
                   and not exists (
                     select 1
                       from target
                      where dependency.dbid = target.oid
                         or dependency.dbid = 0
                        and dependency.classid = 'pg_catalog.pg_database'::regclass
                        and dependency.objid = target.oid
                        and dependency.objsubid = 0
                   )
              ) as "safe!""#,
        target_database,
        database_roles.migrator(),
        database_roles.gateway(),
        database_roles.worker(),
        database_roles.runtime().administrators(),
        database_roles.runtime().administrative_members(),
        database_roles.runtime().administrative_grantors(),
        &protected_roles,
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| authority_sql_error("verify database reset cluster prerequisites", error))?;
    if !safe {
        return Err(Error::Invalid {
            message: "database reset requires the exact protected role, membership, ownership, database ACL and cluster-dependency contract before mutation"
                .to_owned(),
        });
    }
    verify_forbidden_database_connect(connection, database_roles).await
}

async fn verify_capability_shape_connection(
    connection: &mut PgConnection,
    database_roles: &DatabaseRoles,
) -> Result<()> {
    let expected_runtime_roles = database_roles.runtime();
    let safe = sqlx::query_scalar!(
        r#"select exists (
             select 1
               from pg_catalog.pg_roles as app
              where app.rolname = 'synveda_app'
                and not app.rolcanlogin
                and app.rolinherit
                and not app.rolsuper
                and not app.rolcreatedb
                and not app.rolcreaterole
                and not app.rolreplication
                and not app.rolbypassrls
                and app.rolconnlimit = -1
                and app.rolvaliduntil is null
                and not exists (
                  select 1 from pg_catalog.pg_database
                   where datdba = app.oid
                )
                and not exists (
                  select 1
                    from pg_catalog.pg_database as database,
                         lateral pg_catalog.aclexplode(
                           coalesce(
                             database.datacl,
                             pg_catalog.acldefault('d', database.datdba)
                           )
                         ) as acl
                   where acl.grantee = app.oid
                )
                and not exists (
                  select 1 from pg_catalog.pg_namespace
                   where nspowner = app.oid
                )
                and not exists (
                  select 1 from pg_catalog.pg_class
                   where relowner = app.oid
                  union all
                  select 1 from pg_catalog.pg_proc
                   where proowner = app.oid
                  union all
                  select 1 from pg_catalog.pg_type
                   where typowner = app.oid
                )
                and not exists (
                  select 1 from pg_catalog.pg_auth_members
                   where member = app.oid
                )
                and (
                  select count(*)
                    from pg_catalog.pg_auth_members as membership
                    join pg_catalog.pg_roles as member
                      on member.oid = membership.member
                    join pg_catalog.pg_roles as grantor
                      on grantor.oid = membership.grantor
                   where membership.roleid = app.oid
                     and member.rolname = any($1::text[])
                     and grantor.rolname = any($2::text[])
                     and not membership.admin_option
                     and membership.inherit_option
                     and membership.set_option
                ) = 2
                and not exists (
                  select 1
                    from pg_catalog.pg_auth_members as membership
                    join pg_catalog.pg_roles as member
                      on member.oid = membership.member
                    join pg_catalog.pg_roles as grantor
                      on grantor.oid = membership.grantor
                   where membership.roleid = app.oid
                     and not (
                       member.rolname = any($1::text[])
                       and grantor.rolname = any($2::text[])
                       and not membership.admin_option
                       and membership.inherit_option
                       and membership.set_option
                     )
                     and not (
                       exists (
                         select 1
                           from pg_catalog.unnest($4::text[]) with ordinality
                                  as expected_member(role_name, position)
                           join pg_catalog.unnest($5::text[]) with ordinality
                                  as expected_grantor(role_name, position)
                             using (position)
                          where expected_member.role_name = member.rolname
                            and expected_grantor.role_name = grantor.rolname
                       )
                       and membership.admin_option
                       and not membership.inherit_option
                       and not membership.set_option
                     )
                )
                and not exists (
                  select 1 from pg_catalog.pg_db_role_setting
                   where setrole = app.oid
                )
                and not exists (
                  select 1
                    from pg_catalog.pg_shdepend as dependency
                    join pg_catalog.pg_database as current_database
                      on current_database.datname = pg_catalog.current_database()
                   where dependency.refclassid = 'pg_catalog.pg_authid'::regclass
                     and dependency.refobjid = app.oid
                     and not (
                       dependency.deptype = 'a'
                       and dependency.dbid = current_database.oid
                       and (
                         dependency.classid = 'pg_catalog.pg_namespace'::regclass
                         and dependency.objsubid = 0
                         and dependency.objid = (
                           select namespace.oid
                             from pg_catalog.pg_namespace as namespace
                            where namespace.nspname = 'public'
                              and namespace.nspowner = (
                                select migrator.oid
                                  from pg_catalog.pg_roles as migrator
                                 where migrator.rolname = $3
                              )
                         )
                         or dependency.classid = 'pg_catalog.pg_class'::regclass
                         and dependency.objid in (
                           select object.oid
                             from pg_catalog.pg_class as object
                            join pg_catalog.pg_namespace as namespace
                               on namespace.oid = object.relnamespace
                            where namespace.nspname = 'public'
                              and object.relowner = (
                                select migrator.oid
                                  from pg_catalog.pg_roles as migrator
                                 where migrator.rolname = $3
                              )
                         )
                         and (
                           dependency.objsubid = 0
                           and (
                             select object.relkind
                               from pg_catalog.pg_class as object
                              where object.oid = dependency.objid
                           ) in ('r', 'p', 'v', 'm')
                           or dependency.objsubid > 0
                           and (
                             select object.relkind
                               from pg_catalog.pg_class as object
                              where object.oid = dependency.objid
                           ) in ('r', 'p')
                           and exists (
                             select 1
                               from pg_catalog.pg_attribute as attribute
                              where attribute.attrelid = dependency.objid
                                and attribute.attnum = dependency.objsubid
                                and attribute.attnum > 0
                                and not attribute.attisdropped
                           )
                         )
                         or dependency.classid = 'pg_catalog.pg_proc'::regclass
                         and dependency.objsubid = 0
                         and dependency.objid in (
                           select routine.oid
                             from pg_catalog.pg_proc as routine
                             join pg_catalog.pg_namespace as namespace
                               on namespace.oid = routine.pronamespace
                            where namespace.nspname = 'public'
                              and routine.proowner = (
                                select migrator.oid
                                  from pg_catalog.pg_roles as migrator
                                 where migrator.rolname = $3
                              )
                         )
                         or dependency.classid = 'pg_catalog.pg_type'::regclass
                         and dependency.objsubid = 0
                         and dependency.objid in (
                           select data_type.oid
                             from pg_catalog.pg_type as data_type
                             join pg_catalog.pg_namespace as namespace
                               on namespace.oid = data_type.typnamespace
                            where namespace.nspname = 'public'
                              and data_type.typowner = (
                                select migrator.oid
                                  from pg_catalog.pg_roles as migrator
                                 where migrator.rolname = $3
                              )
                         )
                       )
                     )
                )
           ) as "safe!""#,
        expected_runtime_roles.as_slice(),
        expected_runtime_roles.administrators(),
        database_roles.migrator(),
        expected_runtime_roles.administrative_members(),
        expected_runtime_roles.administrative_grantors(),
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| authority_sql_error("verify runtime database capability role", error))?;
    if !safe {
        return Err(Error::Invalid {
            message: "synveda_app must be an inheriting NOLOGIN role with no elevated \
                      cluster capabilities, ownership, outbound membership, database ACL or \
                      role setting; only the configured gateway and worker logins may inherit \
                      it, while provider management grants must be ADMIN-only"
                .to_owned(),
        });
    }
    Ok(())
}

async fn verify_expected_runtime_catalog(
    connection: &mut PgConnection,
    expected_runtime_roles: &ExpectedRuntimeRoles,
) -> Result<()> {
    let safe = sqlx::query_scalar!(
        r#"select (
              select count(*) = 2 and pg_catalog.bool_and(
                role.rolcanlogin
                and role.rolinherit
                and not role.rolsuper
                and not role.rolcreatedb
                and not role.rolcreaterole
                and not role.rolreplication
                and not role.rolbypassrls
                and role.rolconnlimit = -1
                and (role.rolvaliduntil is null
                     or role.rolvaliduntil > pg_catalog.statement_timestamp())
                and (
                  select count(*)
                    from pg_catalog.pg_auth_members as membership
                    join pg_catalog.pg_roles as granted
                      on granted.oid = membership.roleid
                    join pg_catalog.pg_roles as grantor
                      on grantor.oid = membership.grantor
                   where membership.member = role.oid
                     and granted.rolname = 'synveda_app'
                     and grantor.rolname = any($2::text[])
                     and not membership.admin_option
                     and membership.inherit_option
                     and membership.set_option
                ) = 1
                and not exists (
                  select 1
                    from pg_catalog.pg_auth_members as membership
                    join pg_catalog.pg_roles as granted
                      on granted.oid = membership.roleid
                    join pg_catalog.pg_roles as grantor
                      on grantor.oid = membership.grantor
                   where membership.member = role.oid
                     and not (
                       granted.rolname = 'synveda_app'
                       and grantor.rolname = any($2::text[])
                       and not membership.admin_option
                       and membership.inherit_option
                       and membership.set_option
                     )
                )
                and not exists (
                  select 1
                    from pg_catalog.pg_auth_members as membership
                    join pg_catalog.pg_roles as member
                      on member.oid = membership.member
                    join pg_catalog.pg_roles as grantor
                      on grantor.oid = membership.grantor
                   where membership.roleid = role.oid
                     and not (
                       exists (
                         select 1
                           from pg_catalog.unnest($3::text[]) with ordinality
                                  as expected_member(role_name, position)
                           join pg_catalog.unnest($4::text[]) with ordinality
                                  as expected_grantor(role_name, position)
                             using (position)
                          where expected_member.role_name = member.rolname
                            and expected_grantor.role_name = grantor.rolname
                       )
                       and membership.admin_option
                       and not membership.inherit_option
                       and not membership.set_option
                     )
                )
                and not exists (
                  select 1 from pg_catalog.pg_db_role_setting as settings
                   where settings.setrole = role.oid
                )
                and not exists (
                  select 1
                    from pg_catalog.pg_shdepend as dependency
                    join pg_catalog.pg_database as current_database
                      on current_database.datname = pg_catalog.current_database()
                   where dependency.refclassid = 'pg_catalog.pg_authid'::regclass
                     and dependency.refobjid = role.oid
                     and not (
                       dependency.deptype = 'a'
                       and dependency.dbid = 0
                       and dependency.classid = 'pg_catalog.pg_database'::regclass
                       and dependency.objid = current_database.oid
                       and dependency.objsubid = 0
                     )
                )
                and exists (
                  select 1
                    from pg_catalog.pg_database as database,
                         lateral pg_catalog.aclexplode(
                           coalesce(
                             database.datacl,
                             pg_catalog.acldefault('d', database.datdba)
                           )
                         ) as acl
                   where database.datname = pg_catalog.current_database()
                     and acl.grantee = role.oid
                     and acl.grantor = database.datdba
                     and acl.privilege_type = 'CONNECT'
                     and not acl.is_grantable
                )
                and not exists (
                  select 1
                    from pg_catalog.pg_database as database,
                         lateral pg_catalog.aclexplode(
                           coalesce(
                             database.datacl,
                             pg_catalog.acldefault('d', database.datdba)
                           )
                         ) as acl
                   where acl.grantee = role.oid
                     and not (
                       database.datname = pg_catalog.current_database()
                       and acl.grantor = database.datdba
                       and acl.privilege_type = 'CONNECT'
                       and not acl.is_grantable
                     )
                )
              )
                from pg_catalog.pg_roles as role
               where role.rolname = any($1::text[])
            )
            and not exists (
              select 1
                from pg_catalog.pg_database as database,
                     lateral pg_catalog.aclexplode(
                       coalesce(database.datacl, pg_catalog.acldefault('d', database.datdba))
                     ) as acl
               where database.datname = pg_catalog.current_database()
                 and acl.grantee = 0
            )
            and not exists (
              select 1
                from pg_catalog.pg_namespace as namespace,
                     lateral pg_catalog.aclexplode(namespace.nspacl) as acl
               where acl.grantee in (
                 select role.oid from pg_catalog.pg_roles as role
                  where role.rolname = any($1::text[])
               )
              union all
              select 1
                from pg_catalog.pg_attribute as attribute,
                     lateral pg_catalog.aclexplode(attribute.attacl) as acl
               where acl.grantee in (
                 select role.oid from pg_catalog.pg_roles as role
                  where role.rolname = any($1::text[])
               )
              union all
              select 1
                from pg_catalog.pg_class as object,
                     lateral pg_catalog.aclexplode(object.relacl) as acl
               where acl.grantee in (
                 select role.oid from pg_catalog.pg_roles as role
                  where role.rolname = any($1::text[])
               )
              union all
              select 1
                from pg_catalog.pg_proc as routine,
                     lateral pg_catalog.aclexplode(routine.proacl) as acl
               where acl.grantee in (
                 select role.oid from pg_catalog.pg_roles as role
                  where role.rolname = any($1::text[])
               )
              union all
              select 1
                from pg_catalog.pg_type as data_type,
                     lateral pg_catalog.aclexplode(data_type.typacl) as acl
               where acl.grantee in (
                 select role.oid from pg_catalog.pg_roles as role
                  where role.rolname = any($1::text[])
               )
            ) as "safe!""#,
        expected_runtime_roles.as_slice(),
        expected_runtime_roles.administrators(),
        expected_runtime_roles.administrative_members(),
        expected_runtime_roles.administrative_grantors(),
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| authority_sql_error("verify configured runtime database roles", error))?;
    if !safe {
        return Err(Error::Invalid {
            message: "the configured gateway and worker roles must be distinct ordinary LOGIN roles with current credentials, exact non-admin inherited synveda_app membership, only direct non-grantable CONNECT on this database, and no ownership, other membership, setting or direct object ACL".to_owned(),
        });
    }
    Ok(())
}

async fn verify_expected_migrator_catalog(
    connection: &mut PgConnection,
    database_roles: &DatabaseRoles,
) -> Result<()> {
    let safe = sqlx::query_scalar!(
        r#"select exists (
              select 1
                from pg_catalog.pg_roles as role
                join pg_catalog.pg_database as database
                  on database.datname = pg_catalog.current_database()
                join pg_catalog.pg_namespace as namespace
                  on namespace.nspname = 'public'
               where role.rolname = $1
                 and role.oid = database.datdba
                 and role.oid = namespace.nspowner
                 and role.rolcanlogin
                 and role.rolinherit
                 and not role.rolsuper
                 and not role.rolcreatedb
                 and not role.rolcreaterole
                 and not role.rolreplication
                 and not role.rolbypassrls
                 and role.rolconnlimit = -1
                 and (
                   role.rolvaliduntil is null
                   or role.rolvaliduntil > pg_catalog.statement_timestamp()
                 )
                 and not exists (
                   select 1
                     from pg_catalog.pg_database as other_database
                    where other_database.datdba = role.oid
                      and other_database.oid <> database.oid
                 )
                 and not exists (
                   select 1
                     from pg_catalog.pg_namespace as other_namespace
                    where other_namespace.nspowner = role.oid
                      and other_namespace.oid <> namespace.oid
                 )
                 and not exists (
                   select 1
                     from pg_catalog.pg_namespace as extra_namespace
                    where extra_namespace.nspname <> 'public'
                      and pg_catalog.left(extra_namespace.nspname, 3) <> 'pg_'
                      and extra_namespace.nspname <> 'information_schema'
                 )
                 and not exists (
                   select 1
                     from lateral pg_catalog.aclexplode(
                       coalesce(namespace.nspacl, pg_catalog.acldefault('n', namespace.nspowner))
                     ) as acl
                    where not (
                      acl.grantee = role.oid
                      and acl.grantor = role.oid
                      and acl.privilege_type in ('CREATE', 'USAGE')
                      and not acl.is_grantable
                    )
                      and not (
                        acl.grantee = (
                          select app.oid from pg_catalog.pg_roles as app
                           where app.rolname = 'synveda_app'
                        )
                        and acl.grantor = role.oid
                        and acl.privilege_type = 'USAGE'
                        and not acl.is_grantable
                      )
                 )
                 and (
                   select count(*)
                     from lateral pg_catalog.aclexplode(
                       coalesce(namespace.nspacl, pg_catalog.acldefault('n', namespace.nspowner))
                     ) as acl
                    where acl.grantee = role.oid
                      and acl.grantor = role.oid
                      and acl.privilege_type in ('CREATE', 'USAGE')
                      and not acl.is_grantable
                 ) = 2
                 and not exists (
                   select 1
                     from pg_catalog.pg_auth_members as membership
                    where membership.member = role.oid
                 )
                 and not exists (
                   select 1
                     from pg_catalog.pg_auth_members as membership
                     join pg_catalog.pg_roles as member
                       on member.oid = membership.member
                     join pg_catalog.pg_roles as grantor
                       on grantor.oid = membership.grantor
                    where membership.roleid = role.oid
                      and not (
                        exists (
                          select 1
                            from pg_catalog.unnest($5::text[]) with ordinality
                                   as expected_member(role_name, position)
                            join pg_catalog.unnest($6::text[]) with ordinality
                                   as expected_grantor(role_name, position)
                              using (position)
                           where expected_member.role_name = member.rolname
                             and expected_grantor.role_name = grantor.rolname
                        )
                        and membership.admin_option
                        and not membership.inherit_option
                        and not membership.set_option
                      )
                 )
                 and not exists (
                   select 1
                     from pg_catalog.pg_db_role_setting as setting
                    where setting.setrole = role.oid
                 )
                 and not exists (
                   select 1
                     from pg_catalog.pg_database as other_database,
                          lateral pg_catalog.aclexplode(
                            coalesce(
                              other_database.datacl,
                              pg_catalog.acldefault('d', other_database.datdba)
                            )
                          ) as acl
                    where other_database.oid <> database.oid
                      and acl.grantee = role.oid
                 )
                 and not exists (
                   select 1
                     from lateral pg_catalog.aclexplode(
                       coalesce(database.datacl, pg_catalog.acldefault('d', database.datdba))
                     ) as acl
                    where not (
                      acl.grantee = role.oid
                      and acl.grantor = role.oid
                      and acl.privilege_type in ('CREATE', 'CONNECT', 'TEMPORARY')
                      and not acl.is_grantable
                    )
                      and not (
                        acl.grantee in (
                          (select runtime_role.oid from pg_catalog.pg_roles as runtime_role
                            where runtime_role.rolname = $3),
                          (select runtime_role.oid from pg_catalog.pg_roles as runtime_role
                            where runtime_role.rolname = $4)
                        )
                        and acl.grantor = role.oid
                        and acl.privilege_type = 'CONNECT'
                        and not acl.is_grantable
                      )
                      and not exists (
                        select 1
                          from pg_catalog.pg_roles as administrator
                         where administrator.oid = acl.grantee
                           and administrator.rolname = any($2::text[])
                           and acl.privilege_type = 'CONNECT'
                           and not acl.is_grantable
                           and acl.grantor = role.oid
                      )
                 )
                 and (
                   select count(*)
                     from lateral pg_catalog.aclexplode(
                       coalesce(database.datacl, pg_catalog.acldefault('d', database.datdba))
                     ) as acl
                    where acl.grantee = role.oid
                      and acl.grantor = role.oid
                      and acl.privilege_type in ('CREATE', 'CONNECT', 'TEMPORARY')
                      and not acl.is_grantable
                 ) = 3
                 and (
                   select count(*)
                     from lateral pg_catalog.aclexplode(
                       coalesce(database.datacl, pg_catalog.acldefault('d', database.datdba))
                     ) as acl
                    where acl.grantee in (
                      (select runtime_role.oid from pg_catalog.pg_roles as runtime_role
                        where runtime_role.rolname = $3),
                      (select runtime_role.oid from pg_catalog.pg_roles as runtime_role
                        where runtime_role.rolname = $4)
                    )
                      and acl.grantor = role.oid
                      and acl.privilege_type = 'CONNECT'
                      and not acl.is_grantable
                 ) = 2
                 and (
                   select count(*)
                     from lateral pg_catalog.aclexplode(
                       coalesce(database.datacl, pg_catalog.acldefault('d', database.datdba))
                     ) as acl
                     join pg_catalog.pg_roles as administrator
                       on administrator.oid = acl.grantee
                    where administrator.rolname = any($2::text[])
                      and acl.grantor = role.oid
                      and acl.privilege_type = 'CONNECT'
                      and not acl.is_grantable
                 ) = pg_catalog.cardinality($2::text[])
        ) as "safe!""#,
        database_roles.migrator(),
        database_roles.runtime().administrators(),
        database_roles.gateway(),
        database_roles.worker(),
        database_roles.runtime().administrative_members(),
        database_roles.runtime().administrative_grantors(),
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| authority_sql_error("verify configured migration database role", error))?;
    if !safe {
        return Err(Error::Invalid {
            message: "the configured migrator must be the ordinary owner of only this database and its public application schema, with the exact runtime and trusted-administrator CONNECT ACLs and no other capability, membership, setting or database ACL"
                .to_owned(),
        });
    }
    verify_migrator_ownership(connection, database_roles.migrator()).await
}

async fn verify_application_acl(
    connection: &mut PgConnection,
    database_roles: &DatabaseRoles,
) -> Result<()> {
    let safe = sqlx::query_scalar!(
        r#"select exists (
              select 1
                from pg_catalog.pg_roles as app
                join pg_catalog.pg_roles as migrator
                  on migrator.rolname = $1
                join pg_catalog.pg_database as database
                  on database.datname = pg_catalog.current_database()
                join pg_catalog.pg_namespace as namespace
                  on namespace.nspname = 'public'
               where app.rolname = 'synveda_app'
                 and database.datdba = migrator.oid
                 and namespace.nspowner = migrator.oid
                 and not exists (
                   select 1
                     from pg_catalog.pg_namespace as candidate,
                          lateral pg_catalog.aclexplode(candidate.nspacl) as acl
                    where acl.grantee = app.oid
                      and not (
                        candidate.oid = namespace.oid
                        and acl.grantor = migrator.oid
                        and acl.privilege_type = 'USAGE'
                        and not acl.is_grantable
                      )
                 )
                 and not exists (
                   select 1
                     from pg_catalog.pg_namespace as candidate,
                          lateral pg_catalog.aclexplode(candidate.nspacl) as acl
                    where candidate.oid = namespace.oid
                      and acl.grantee = 0
                      and acl.privilege_type = 'CREATE'
                 )
                 and not exists (
                   select 1
                     from pg_catalog.pg_attribute as attribute
                     join pg_catalog.pg_class as object
                       on object.oid = attribute.attrelid
                     join pg_catalog.pg_namespace as object_namespace
                       on object_namespace.oid = object.relnamespace,
                          lateral pg_catalog.aclexplode(attribute.attacl) as acl
                    where object_namespace.oid = namespace.oid
                      and acl.grantee = 0
                   union all
                   select 1
                     from pg_catalog.pg_class as object
                     join pg_catalog.pg_namespace as object_namespace
                       on object_namespace.oid = object.relnamespace,
                          lateral pg_catalog.aclexplode(object.relacl) as acl
                    where object_namespace.oid = namespace.oid
                      and acl.grantee = 0
                   union all
                   select 1
                     from pg_catalog.pg_type as data_type
                     join pg_catalog.pg_namespace as type_namespace
                       on type_namespace.oid = data_type.typnamespace,
                          lateral pg_catalog.aclexplode(data_type.typacl) as acl
                    where type_namespace.oid = namespace.oid
                      and acl.grantee = 0
                 )
                 and not exists (
                   select 1
                     from pg_catalog.pg_proc as routine
                     join pg_catalog.pg_namespace as routine_namespace
                       on routine_namespace.oid = routine.pronamespace,
                          lateral pg_catalog.aclexplode(
                            coalesce(
                              routine.proacl,
                              pg_catalog.acldefault('f', routine.proowner)
                            )
                          ) as acl
                    where pg_catalog.left(routine_namespace.nspname, 3) <> 'pg_'
                      and routine_namespace.nspname <> 'information_schema'
                      and routine.prosecdef
                      and acl.grantee = 0
                      and acl.privilege_type = 'EXECUTE'
                 )
                 and not exists (
                   select 1
                     from pg_catalog.pg_namespace as candidate,
                          lateral pg_catalog.aclexplode(candidate.nspacl) as acl
                    where candidate.oid = namespace.oid
                      and (
                        acl.grantee not in (migrator.oid, app.oid)
                        or acl.grantor <> migrator.oid
                      )
                   union all
                   select 1
                     from pg_catalog.pg_attribute as attribute
                     join pg_catalog.pg_class as object
                       on object.oid = attribute.attrelid
                     join pg_catalog.pg_namespace as object_namespace
                       on object_namespace.oid = object.relnamespace,
                          lateral pg_catalog.aclexplode(attribute.attacl) as acl
                    where object_namespace.oid = namespace.oid
                      and (
                        acl.grantee not in (object.relowner, app.oid)
                        or acl.grantor <> object.relowner
                      )
                   union all
                   select 1
                     from pg_catalog.pg_class as object
                     join pg_catalog.pg_namespace as object_namespace
                       on object_namespace.oid = object.relnamespace,
                          lateral pg_catalog.aclexplode(object.relacl) as acl
                    where object_namespace.oid = namespace.oid
                      and (
                        acl.grantee not in (object.relowner, app.oid)
                        or acl.grantor <> object.relowner
                      )
                   union all
                   select 1
                     from pg_catalog.pg_proc as routine
                     join pg_catalog.pg_namespace as routine_namespace
                       on routine_namespace.oid = routine.pronamespace,
                          lateral pg_catalog.aclexplode(routine.proacl) as acl
                    where routine_namespace.oid = namespace.oid
                      and (
                        acl.grantee not in (routine.proowner, app.oid)
                        or acl.grantor <> routine.proowner
                      )
                   union all
                   select 1
                     from pg_catalog.pg_type as data_type
                     join pg_catalog.pg_namespace as type_namespace
                       on type_namespace.oid = data_type.typnamespace,
                          lateral pg_catalog.aclexplode(data_type.typacl) as acl
                    where type_namespace.oid = namespace.oid
                      and (
                        acl.grantee not in (data_type.typowner, app.oid)
                        or acl.grantor <> data_type.typowner
                      )
                 )
                 and not exists (
                   select 1
                     from pg_catalog.pg_attribute as attribute
                     join pg_catalog.pg_class as object
                       on object.oid = attribute.attrelid
                     join pg_catalog.pg_namespace as object_namespace
                       on object_namespace.oid = object.relnamespace,
                          lateral pg_catalog.aclexplode(attribute.attacl) as acl
                    where acl.grantee = app.oid
                      and not (
                        object_namespace.oid = namespace.oid
                        and object.relowner = migrator.oid
                        and object.relkind in ('r', 'p')
                        and acl.grantor = migrator.oid
                        and acl.privilege_type = 'UPDATE'
                        and not acl.is_grantable
                      )
                 )
                 and not exists (
                   select 1
                     from pg_catalog.pg_class as object
                     join pg_catalog.pg_namespace as object_namespace
                       on object_namespace.oid = object.relnamespace,
                          lateral pg_catalog.aclexplode(object.relacl) as acl
                    where acl.grantee = app.oid
                      and not (
                        object_namespace.oid = namespace.oid
                        and object.relowner = migrator.oid
                        and object.relkind in ('r', 'p', 'v', 'm')
                        and acl.grantor = migrator.oid
                        and acl.privilege_type in ('SELECT', 'INSERT', 'UPDATE', 'DELETE')
                        and not acl.is_grantable
                      )
                 )
                 and not exists (
                   select 1
                     from pg_catalog.pg_proc as routine
                     join pg_catalog.pg_namespace as routine_namespace
                       on routine_namespace.oid = routine.pronamespace,
                          lateral pg_catalog.aclexplode(routine.proacl) as acl
                    where acl.grantee = app.oid
                      and not (
                        routine_namespace.oid = namespace.oid
                        and routine.proowner = migrator.oid
                        and acl.grantor = migrator.oid
                        and acl.privilege_type = 'EXECUTE'
                        and not acl.is_grantable
                      )
                 )
                 and not exists (
                   select 1
                     from pg_catalog.pg_type as data_type
                     join pg_catalog.pg_namespace as type_namespace
                       on type_namespace.oid = data_type.typnamespace,
                          lateral pg_catalog.aclexplode(data_type.typacl) as acl
                    where acl.grantee = app.oid
                      and not (
                        type_namespace.oid = namespace.oid
                        and data_type.typowner = migrator.oid
                        and acl.grantor = migrator.oid
                        and acl.privilege_type = 'USAGE'
                        and not acl.is_grantable
                      )
                 )
                 and not exists (
                   select 1
                     from pg_catalog.pg_class as object
                     join pg_catalog.pg_namespace as object_namespace
                       on object_namespace.oid = object.relnamespace
                    where object_namespace.oid = namespace.oid
                      and object.relowner <> migrator.oid
                      and not exists (
                        select 1
                          from pg_catalog.pg_depend as dependency
                          join pg_catalog.pg_extension as extension
                            on extension.oid = dependency.refobjid
                         where dependency.classid = 'pg_catalog.pg_class'::regclass
                           and dependency.objid = object.oid
                           and dependency.deptype = 'e'
                           and extension.extname in ('btree_gin', 'vector')
                           and extension.extnamespace = namespace.oid
                      )
                 )
                 and not exists (
                   select 1
                     from pg_catalog.pg_proc as routine
                     join pg_catalog.pg_namespace as routine_namespace
                       on routine_namespace.oid = routine.pronamespace
                    where routine_namespace.oid = namespace.oid
                      and routine.proowner <> migrator.oid
                      and not exists (
                        select 1
                          from pg_catalog.pg_depend as dependency
                          join pg_catalog.pg_extension as extension
                            on extension.oid = dependency.refobjid
                         where dependency.classid = 'pg_catalog.pg_proc'::regclass
                           and dependency.objid = routine.oid
                           and dependency.deptype = 'e'
                           and extension.extname in ('btree_gin', 'vector')
                           and extension.extnamespace = namespace.oid
                      )
                 )
                 and not exists (
                   select 1
                     from pg_catalog.pg_type as data_type
                     join pg_catalog.pg_namespace as type_namespace
                       on type_namespace.oid = data_type.typnamespace
                    where type_namespace.oid = namespace.oid
                      and data_type.typowner <> migrator.oid
                      and not exists (
                        select 1
                          from pg_catalog.pg_depend as dependency
                          join pg_catalog.pg_extension as extension
                            on extension.oid = dependency.refobjid
                         where dependency.classid = 'pg_catalog.pg_type'::regclass
                           and dependency.objid = data_type.oid
                           and dependency.deptype = 'e'
                           and extension.extname in ('btree_gin', 'vector')
                           and extension.extnamespace = namespace.oid
                      )
                 )
                 and not exists (
                   select 1
                     from pg_catalog.pg_extension as extension
                    where extension.extname in ('btree_gin', 'vector')
                      and (
                        extension.extnamespace <> namespace.oid
                        or extension.extowner not in (
                          select administrator.oid
                            from pg_catalog.pg_roles as administrator
                           where administrator.rolname = any($3::text[])
                        )
                        or extension.extname = 'btree_gin'
                           and extension.extversion <> '1.3'
                        or extension.extname = 'vector'
                           and extension.extversion <> '0.8.6'
                      )
                 )
                 and (
                   select count(*)
                     from pg_catalog.pg_extension as extension
                    where extension.extname in ('btree_gin', 'vector')
                      and extension.extnamespace = namespace.oid
                 ) = 2
                 and not exists (
                   select 1
                     from pg_catalog.pg_default_acl as defaults
                    where defaults.defaclrole in (app.oid, migrator.oid)
                       or exists (
                         select 1
                           from pg_catalog.aclexplode(defaults.defaclacl) as acl
                          where acl.grantee = app.oid
                             or acl.grantee in (
                               select role.oid from pg_catalog.pg_roles as role
                                where role.rolname = any($2::text[])
                             )
                       )
                 )
            ) as "safe!""#,
        database_roles.migrator(),
        database_roles.runtime().as_slice(),
        database_roles.runtime().administrators(),
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| authority_sql_error("verify application database ACL and ownership", error))?;
    if !safe {
        return Err(Error::Invalid {
            message: "the public application schema, objects, trusted extension ownership and synveda_app ACLs must match the migration-owned closed privilege surface".to_owned(),
        });
    }
    verify_extension_authority_connection(connection, database_roles).await?;
    verify_application_acl_fingerprint(connection).await?;
    Ok(())
}

/// Exact clean-database extension and event-trigger proof used before SQLx is
/// allowed to execute the epoch baseline. Application objects do not exist at
/// this point, so this is deliberately narrower than the full migrator
/// sentinel while retaining the same executable extension fingerprint.
pub(crate) async fn verify_migration_extension_prerequisites_connection(
    connection: &mut PgConnection,
    database_roles: &DatabaseRoles,
) -> Result<()> {
    verify_extension_authority_connection(connection, database_roles).await
}

async fn verify_extension_authority_connection(
    connection: &mut PgConnection,
    database_roles: &DatabaseRoles,
) -> Result<()> {
    let trusted_plpgsql_owners = database_roles.trusted_extension_owners();
    let safe = sqlx::query_scalar!(
        r#"select not exists (
                     select 1 from pg_catalog.pg_event_trigger
                   )
                   and (
                     select count(*)
                       from pg_catalog.pg_extension as extension
                       join pg_catalog.pg_namespace as namespace
                         on namespace.oid = extension.extnamespace
                       join pg_catalog.pg_roles as owner
                         on owner.oid = extension.extowner
                      where (
                        extension.extname = 'btree_gin'
                        and extension.extversion = '1.3'
                        and namespace.nspname = 'public'
                        and owner.rolname = any($1::text[])
                      ) or (
                        extension.extname = 'vector'
                        and extension.extversion = '0.8.6'
                        and namespace.nspname = 'public'
                        and owner.rolname = any($1::text[])
                      ) or (
                        extension.extname = 'plpgsql'
                        and extension.extversion = '1.0'
                        and namespace.nspname = 'pg_catalog'
                        and owner.rolname = any($2::text[])
                      )
                   ) = 3 as "safe!""#,
        database_roles.runtime().administrators(),
        &trusted_plpgsql_owners,
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| {
        authority_sql_error(
            "verify trusted extension and event-trigger authority",
            error,
        )
    })?;
    if !safe {
        return Err(Error::Invalid {
            message: "the database must have no event trigger and its exact plpgsql, btree_gin and vector extension owners must be declared deployment trust roots".to_owned(),
        });
    }
    verify_extension_fingerprint(connection).await
}

async fn verify_extension_fingerprint(connection: &mut PgConnection) -> Result<()> {
    let safe = sqlx::query_file_scalar!("sql/extension_fingerprint.sql")
        .fetch_one(&mut *connection)
        .await
        .map_err(|error| {
            authority_sql_error("verify trusted extension executable fingerprint", error)
        })?;
    if safe != Some(true) {
        return Err(Error::Invalid {
            message: "the trusted extension member identities and executable definitions do not match the pinned PostgreSQL 17 contract".to_owned(),
        });
    }
    Ok(())
}

// BLAKE3 of the sorted, provider-neutral application ACL inventory produced
// by the current epoch-3 baseline. Names and privileges are included;
// grantors are checked relationally above so provider-assigned migrator names
// do not change the contract.
const APPLICATION_ACL_FINGERPRINT: &str =
    "c35da4e5e77eb8969a23f612ca75b9953ea2f8937018ec4cf099905235eddbf6";
const APPLICATION_ACL_ROW_COUNT: usize = 334;

async fn verify_application_acl_fingerprint(connection: &mut PgConnection) -> Result<()> {
    let actual = application_acl_fingerprint(connection).await?;
    if actual != APPLICATION_ACL_FINGERPRINT {
        return Err(Error::Invalid {
            message:
                "the synveda_app ACL inventory does not match the current schema epoch 3 baseline"
                    .to_owned(),
        });
    }
    Ok(())
}

async fn application_acl_fingerprint(connection: &mut PgConnection) -> Result<String> {
    let rows = sqlx::query!(
        r#"select inventory.kind as "kind!",
                  inventory.schema_name as "schema_name!",
                  inventory.object_name as "object_name!",
                  inventory.detail as "detail!",
                  inventory.privilege as "privilege!"
             from (
               select 'namespace'::text as kind,
                      namespace.nspname::text as schema_name,
                      ''::text as object_name,
                      ''::text as detail,
                      acl.privilege_type::text as privilege
                 from pg_catalog.pg_namespace as namespace,
                      lateral pg_catalog.aclexplode(namespace.nspacl) as acl
                 join pg_catalog.pg_roles as app on app.oid = acl.grantee
                where app.rolname = 'synveda_app'
               union all
               select 'attribute'::text,
                      namespace.nspname::text,
                      object.relname::text,
                      attribute.attname::text,
                      acl.privilege_type::text
                 from pg_catalog.pg_attribute as attribute
                 join pg_catalog.pg_class as object on object.oid = attribute.attrelid
                 join pg_catalog.pg_namespace as namespace on namespace.oid = object.relnamespace,
                      lateral pg_catalog.aclexplode(attribute.attacl) as acl
                 join pg_catalog.pg_roles as app on app.oid = acl.grantee
                where app.rolname = 'synveda_app'
               union all
               select 'relation'::text,
                      namespace.nspname::text,
                      object.relname::text,
                      object.relkind::text,
                      acl.privilege_type::text
                 from pg_catalog.pg_class as object
                 join pg_catalog.pg_namespace as namespace on namespace.oid = object.relnamespace,
                      lateral pg_catalog.aclexplode(object.relacl) as acl
                 join pg_catalog.pg_roles as app on app.oid = acl.grantee
                where app.rolname = 'synveda_app'
               union all
               select 'routine'::text,
                      namespace.nspname::text,
                      routine.proname::text,
                      pg_catalog.pg_get_function_identity_arguments(routine.oid)::text,
                      acl.privilege_type::text
                 from pg_catalog.pg_proc as routine
                 join pg_catalog.pg_namespace as namespace on namespace.oid = routine.pronamespace,
                      lateral pg_catalog.aclexplode(routine.proacl) as acl
                 join pg_catalog.pg_roles as app on app.oid = acl.grantee
                where app.rolname = 'synveda_app'
               union all
               select 'type'::text,
                      namespace.nspname::text,
                      data_type.typname::text,
                      data_type.typtype::text,
                      acl.privilege_type::text
                 from pg_catalog.pg_type as data_type
                 join pg_catalog.pg_namespace as namespace on namespace.oid = data_type.typnamespace,
                      lateral pg_catalog.aclexplode(data_type.typacl) as acl
                 join pg_catalog.pg_roles as app on app.oid = acl.grantee
                where app.rolname = 'synveda_app'
             ) as inventory
            order by inventory.kind collate "C", inventory.schema_name collate "C",
                     inventory.object_name collate "C", inventory.detail collate "C",
                     inventory.privilege collate "C"
            limit $1"#,
        i64::try_from(APPLICATION_ACL_ROW_COUNT + 1).map_err(|_| Error::Invalid {
            message: "the application ACL inventory bound is invalid".to_owned(),
        })?,
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| authority_sql_error("read application ACL inventory", error))?;

    if rows.len() != APPLICATION_ACL_ROW_COUNT {
        return Err(Error::Invalid {
            message:
                "the synveda_app ACL inventory row count does not match the current schema epoch 3 baseline"
                    .to_owned(),
        });
    }

    let mut hasher = blake3::Hasher::new();
    for row in rows {
        for field in [
            row.kind,
            row.schema_name,
            row.object_name,
            row.detail,
            row.privilege,
        ] {
            hasher.update(field.as_bytes());
            hasher.update(&[0]);
        }
        hasher.update(b"\n");
    }
    Ok(hasher.finalize().to_hex().to_string())
}

// BLAKE3 of every non-extension routine owned by the current epoch-3
// baseline. `pg_get_functiondef` captures the executable body and routine
// attributes; the owner is represented as a provider-neutral equality bit.
// An exact row count and per-definition byte ceiling make the catalogue read
// bounded even after hostile owner-level drift.
const ROUTINE_CATALOG_FINGERPRINT: &str =
    "8591bcfffbda3ec7b3908009816280b32d7127a429b6615d23276d3ee921b114";
const ROUTINE_CATALOG_ROW_COUNT: usize = 67;
const ROUTINE_DEFINITION_MAX_BYTES: i32 = 131_072;
const ROUTINE_CONFIGURATION_MAX_ITEMS: i32 = 32;
const ROUTINE_CONFIGURATION_ITEM_MAX_BYTES: i32 = 4096;

async fn verify_routine_catalog_fingerprint(
    connection: &mut PgConnection,
    migrator: &str,
) -> Result<()> {
    let actual = routine_catalog_fingerprint(connection, migrator).await?;
    if actual != ROUTINE_CATALOG_FINGERPRINT {
        return Err(Error::Invalid {
            message: "the application routine definition inventory does not match the current schema epoch 3 baseline"
                .to_owned(),
        });
    }
    Ok(())
}

async fn routine_catalog_fingerprint(
    connection: &mut PgConnection,
    migrator: &str,
) -> Result<String> {
    let rows = sqlx::query!(
        r#"select namespace.nspname::text as "schema_name!",
                  routine.proname::text as "routine_name!",
                  pg_catalog.pg_get_function_identity_arguments(routine.oid)::text
                    as "identity_arguments!",
                  routine.prokind::text as "routine_kind!",
                  (owner.rolname = $1) as "owned_by_migrator!",
                  case
                    when pg_catalog.octet_length(routine.prosrc) > $2
                      or pg_catalog.octet_length(
                           coalesce(routine.probin, '')
                         ) > $2
                      or pg_catalog.octet_length(
                           coalesce(routine.prosqlbody::text, '')
                         ) > $2
                      or pg_catalog.octet_length(
                           coalesce(routine.proargdefaults::text, '')
                         ) > $2
                      or coalesce(pg_catalog.cardinality(routine.proconfig), 0) > $3
                      or exists (
                        select 1
                          from pg_catalog.unnest(
                                 coalesce(routine.proconfig, array[]::text[])
                               ) as setting(value)
                         where pg_catalog.octet_length(setting.value) > $4
                      )
                    then '<oversized>'
                    when routine.prokind = 'a' then '<unsupported-kind>'
                    when pg_catalog.octet_length(
                           pg_catalog.pg_get_functiondef(routine.oid)
                         ) <= $2
                    then pg_catalog.pg_get_functiondef(routine.oid)
                    else '<oversized>'
                  end as "definition!"
             from pg_catalog.pg_proc as routine
             join pg_catalog.pg_namespace as namespace
               on namespace.oid = routine.pronamespace
             join pg_catalog.pg_roles as owner on owner.oid = routine.proowner
            where namespace.nspname = 'public'
              and not exists (
                select 1
                  from pg_catalog.pg_depend as dependency
                  join pg_catalog.pg_extension as extension
                    on extension.oid = dependency.refobjid
                 where dependency.classid = 'pg_catalog.pg_proc'::regclass
                   and dependency.objid = routine.oid
                   and dependency.objsubid = 0
                   and dependency.deptype = 'e'
                   and extension.extname in ('btree_gin', 'vector')
                   and extension.extnamespace = namespace.oid
              )
            order by namespace.nspname collate "C", routine.proname collate "C",
                     pg_catalog.pg_get_function_identity_arguments(routine.oid) collate "C",
                     routine.prokind::text collate "C"
            limit $5"#,
        migrator,
        ROUTINE_DEFINITION_MAX_BYTES,
        ROUTINE_CONFIGURATION_MAX_ITEMS,
        ROUTINE_CONFIGURATION_ITEM_MAX_BYTES,
        i64::try_from(ROUTINE_CATALOG_ROW_COUNT + 1).map_err(|_| Error::Invalid {
            message: "the application routine inventory bound is invalid".to_owned(),
        })?,
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| authority_sql_error("read application routine definition inventory", error))?;

    if rows.len() != ROUTINE_CATALOG_ROW_COUNT {
        return Err(Error::Invalid {
            message: "the application routine inventory row count does not match the current schema epoch 3 baseline"
                .to_owned(),
        });
    }

    let mut hasher = blake3::Hasher::new();
    for row in rows {
        if matches!(
            row.definition.as_str(),
            "<oversized>" | "<unsupported-kind>"
        ) {
            return Err(Error::Invalid {
                message: "the application routine inventory contains an unsupported or oversized definition"
                    .to_owned(),
            });
        }
        for field in [
            row.schema_name,
            row.routine_name,
            row.identity_arguments,
            row.routine_kind,
            row.owned_by_migrator.to_string(),
            row.definition,
        ] {
            hasher.update(field.as_bytes());
            hasher.update(&[0]);
        }
        hasher.update(b"\n");
    }
    Ok(hasher.finalize().to_hex().to_string())
}

// BLAKE3 of every user-defined trigger attached to the public application
// schema. PostgreSQL does not encode enabled/replica state in
// `pg_get_triggerdef`, so that state and provider-neutral ownership bits are
// hashed separately from the deparsed definition.
const TRIGGER_CATALOG_FINGERPRINT: &str =
    "7602373bfa190dd5183cea75a69055f3f9354168b8ab5bd2bb2e4e8b8d14a32e";
const TRIGGER_CATALOG_ROW_COUNT: usize = 108;
const TRIGGER_DEFINITION_MAX_BYTES: i32 = 16_384;
const TRIGGER_ARGUMENT_MAX_COUNT: i16 = 128;

async fn verify_trigger_catalog_fingerprint(
    connection: &mut PgConnection,
    migrator: &str,
) -> Result<()> {
    let actual = trigger_catalog_fingerprint(connection, migrator).await?;
    if actual != TRIGGER_CATALOG_FINGERPRINT {
        return Err(Error::Invalid {
            message: "the application trigger definition inventory does not match the current schema epoch 3 baseline"
                .to_owned(),
        });
    }
    Ok(())
}

async fn trigger_catalog_fingerprint(
    connection: &mut PgConnection,
    migrator: &str,
) -> Result<String> {
    let rows = sqlx::query!(
        r#"select namespace.nspname::text as "schema_name!",
                  relation.relname::text as "relation_name!",
                  relation.relkind::text as "relation_kind!",
                  trigger.tgname::text as "trigger_name!",
                  function_namespace.nspname::text as "function_schema!",
                  trigger_function.proname::text as "function_name!",
                  pg_catalog.pg_get_function_identity_arguments(trigger_function.oid)::text
                    as "function_identity_arguments!",
                  (relation_owner.rolname = $1) as "relation_owned_by_migrator!",
                  (function_owner.rolname = $1) as "function_owned_by_migrator!",
                  trigger.tgenabled::text as "enabled_state!",
                  trigger.tgtype::text as "trigger_type!",
                  (trigger.tgparentid = 0) as "is_root_trigger!",
                  (trigger.tgconstraint = 0) as "is_non_constraint_trigger!",
                  trigger.tgdeferrable as "deferrable!",
                  trigger.tginitdeferred as "initially_deferred!",
                  case
                    when pg_catalog.octet_length(trigger.tgargs) > $2
                      or pg_catalog.octet_length(
                           coalesce(trigger.tgqual::text, '')
                         ) > $2
                      or trigger.tgnargs > $3
                    then '<oversized>'
                    when pg_catalog.octet_length(
                           pg_catalog.pg_get_triggerdef(trigger.oid, false)
                         ) <= $2
                    then pg_catalog.pg_get_triggerdef(trigger.oid, false)
                    else '<oversized>'
                  end as "definition!"
             from pg_catalog.pg_trigger as trigger
             join pg_catalog.pg_class as relation on relation.oid = trigger.tgrelid
             join pg_catalog.pg_namespace as namespace
               on namespace.oid = relation.relnamespace
             join pg_catalog.pg_roles as relation_owner
               on relation_owner.oid = relation.relowner
             join pg_catalog.pg_proc as trigger_function
               on trigger_function.oid = trigger.tgfoid
             join pg_catalog.pg_namespace as function_namespace
               on function_namespace.oid = trigger_function.pronamespace
             join pg_catalog.pg_roles as function_owner
               on function_owner.oid = trigger_function.proowner
            where namespace.nspname = 'public'
              and not trigger.tgisinternal
            order by namespace.nspname collate "C", relation.relname collate "C",
                     trigger.tgname collate "C"
            limit $4"#,
        migrator,
        TRIGGER_DEFINITION_MAX_BYTES,
        TRIGGER_ARGUMENT_MAX_COUNT,
        i64::try_from(TRIGGER_CATALOG_ROW_COUNT + 1).map_err(|_| Error::Invalid {
            message: "the application trigger inventory bound is invalid".to_owned(),
        })?,
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| authority_sql_error("read application trigger definition inventory", error))?;

    if rows.len() != TRIGGER_CATALOG_ROW_COUNT {
        return Err(Error::Invalid {
            message: "the application trigger inventory row count does not match the current schema epoch 3 baseline"
                .to_owned(),
        });
    }

    let mut hasher = blake3::Hasher::new();
    for row in rows {
        if row.definition == "<oversized>" {
            return Err(Error::Invalid {
                message: "the application trigger inventory contains an oversized definition"
                    .to_owned(),
            });
        }
        for field in [
            row.schema_name,
            row.relation_name,
            row.relation_kind,
            row.trigger_name,
            row.function_schema,
            row.function_name,
            row.function_identity_arguments,
            row.relation_owned_by_migrator.to_string(),
            row.function_owned_by_migrator.to_string(),
            row.enabled_state,
            row.trigger_type,
            row.is_root_trigger.to_string(),
            row.is_non_constraint_trigger.to_string(),
            row.deferrable.to_string(),
            row.initially_deferred.to_string(),
            row.definition,
        ] {
            hasher.update(field.as_bytes());
            hasher.update(&[0]);
        }
        hasher.update(b"\n");
    }
    Ok(hasher.finalize().to_hex().to_string())
}

const RLS_CATALOG_FINGERPRINT: &str =
    "eab620d633e874a6fa8c3c468b3d0ab51b8380e70a4df1a0103b97c1166e98be";
const RLS_CATALOG_ROW_COUNT: usize = 90;
const TENANT_HELPER_SOURCE: &str =
    "\n    select nullif(current_setting('synveda.tenant_id', true), '')::uuid\n";

async fn verify_tenant_helper_definition(
    connection: &mut PgConnection,
    migrator: &str,
) -> Result<()> {
    let accepted = sqlx::query_scalar!(
        r#"select count(*) = 1
                  and pg_catalog.bool_and(
                    owner.rolname = $1
                    and language.lanname = 'sql'
                    and routine.prosrc = $2
                    and pg_catalog.pg_get_function_identity_arguments(routine.oid) = ''
                    and pg_catalog.format_type(routine.prorettype, null) = 'uuid'
                    and routine.provolatile = 's'
                    and routine.proparallel = 's'
                    and not routine.prosecdef
                    and not routine.proleakproof
                    and not routine.proisstrict
                    and routine.prokind = 'f'
                    and routine.proconfig is null
                    and routine.proacl is null
                  ) as "accepted!"
             from pg_catalog.pg_proc as routine
             join pg_catalog.pg_namespace as namespace
               on namespace.oid = routine.pronamespace
             join pg_catalog.pg_roles as owner on owner.oid = routine.proowner
             join pg_catalog.pg_language as language on language.oid = routine.prolang
            where namespace.nspname = 'public'
              and routine.proname = 'synveda_current_tenant'"#,
        migrator,
        TENANT_HELPER_SOURCE,
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| authority_sql_error("verify tenant-isolation helper definition", error))?;
    if !accepted {
        return Err(Error::Invalid {
            message:
                "the tenant-isolation helper does not match the current schema epoch 3 baseline"
                    .to_owned(),
        });
    }
    Ok(())
}

async fn verify_rls_catalog_fingerprint(connection: &mut PgConnection) -> Result<()> {
    let actual = rls_catalog_fingerprint(connection).await?;
    if actual != RLS_CATALOG_FINGERPRINT {
        return Err(Error::Invalid {
            message:
                "the forced-RLS policy inventory does not match the current schema epoch 3 baseline"
                    .to_owned(),
        });
    }
    Ok(())
}

async fn rls_catalog_fingerprint(connection: &mut PgConnection) -> Result<String> {
    let rows = sqlx::query!(
        r#"select namespace.nspname as "schema_name!",
                  object.relname as "object_name!",
                  object.relkind::text as "object_kind!",
                  object.relrowsecurity as "row_security!",
                  object.relforcerowsecurity as "force_row_security!",
                  coalesce(policy.polname, '') as "policy_name!",
                  coalesce(policy.polpermissive::text, '') as "permissive!",
                  coalesce(policy.polcmd::text, '') as "command!",
                  case
                    when pg_catalog.cardinality(policy.polroles) > 16 then '<oversized>'
                    else coalesce(
                      (
                        select pg_catalog.string_agg(
                                 coalesce(role.rolname, 'PUBLIC'),
                                 ',' order by coalesce(role.rolname, 'PUBLIC') collate "C"
                               )
                          from pg_catalog.unnest(policy.polroles) as policy_role(oid)
                          left join pg_catalog.pg_roles as role on role.oid = policy_role.oid
                      ),
                      ''
                    )
                  end as "roles!",
                  case
                    when pg_catalog.octet_length(coalesce(
                           pg_catalog.pg_get_expr(policy.polqual, policy.polrelid), ''
                         )) <= 4096
                    then coalesce(
                      pg_catalog.pg_get_expr(policy.polqual, policy.polrelid), ''
                    )
                    else '<oversized>'
                  end as "using_expression!",
                  case
                    when pg_catalog.octet_length(coalesce(
                           pg_catalog.pg_get_expr(policy.polwithcheck, policy.polrelid), ''
                         )) <= 4096
                    then coalesce(
                      pg_catalog.pg_get_expr(policy.polwithcheck, policy.polrelid), ''
                    )
                    else '<oversized>'
                  end as "check_expression!"
             from pg_catalog.pg_class as object
             join pg_catalog.pg_namespace as namespace
               on namespace.oid = object.relnamespace
             left join pg_catalog.pg_policy as policy
               on policy.polrelid = object.oid
            where namespace.nspname = 'public'
              and object.relkind in ('r', 'p')
            order by namespace.nspname collate "C", object.relname collate "C",
                     policy.polname collate "C"
            limit $1"#,
        i64::try_from(RLS_CATALOG_ROW_COUNT + 1).map_err(|_| Error::Invalid {
            message: "the RLS policy inventory bound is invalid".to_owned(),
        })?,
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| authority_sql_error("read forced-RLS policy inventory", error))?;

    if rows.len() != RLS_CATALOG_ROW_COUNT {
        return Err(Error::Invalid {
            message: "the forced-RLS policy inventory row count does not match the current schema epoch 3 baseline"
                .to_owned(),
        });
    }

    let mut hasher = blake3::Hasher::new();
    for row in rows {
        for field in [
            row.schema_name,
            row.object_name,
            row.object_kind,
            row.row_security.to_string(),
            row.force_row_security.to_string(),
            row.policy_name,
            row.permissive,
            row.command,
            row.roles,
            row.using_expression,
            row.check_expression,
        ] {
            hasher.update(field.as_bytes());
            hasher.update(&[0]);
        }
        hasher.update(b"\n");
    }
    Ok(hasher.finalize().to_hex().to_string())
}

async fn verify_migrator_ownership(connection: &mut PgConnection, role_name: &str) -> Result<()> {
    let safe = sqlx::query_scalar!(
        r#"select not exists (
              select 1
                from pg_catalog.pg_shdepend as dependency
                join pg_catalog.pg_roles as role
                  on role.oid = dependency.refobjid
                join pg_catalog.pg_database as database
                  on database.datname = pg_catalog.current_database()
               where dependency.refclassid = 'pg_catalog.pg_authid'::regclass
                 and role.rolname = $1
                 and not (
                   dependency.deptype = 'o'
                   and dependency.objsubid = 0
                   and (
                     dependency.dbid = 0
                     and dependency.classid = 'pg_catalog.pg_database'::regclass
                     and dependency.objid = database.oid
                     or dependency.dbid = database.oid
                     and dependency.classid = 'pg_catalog.pg_namespace'::regclass
                     and dependency.objid = (
                       select namespace.oid
                         from pg_catalog.pg_namespace as namespace
                        where namespace.nspname = 'public'
                          and namespace.nspowner = role.oid
                     )
                     or dependency.dbid = database.oid
                     and dependency.classid = 'pg_catalog.pg_class'::regclass
                     and dependency.objid in (
                       select object.oid
                         from pg_catalog.pg_class as object
                         join pg_catalog.pg_namespace as namespace
                           on namespace.oid = object.relnamespace
                        where namespace.nspname = 'public'
                          and object.relowner = role.oid
                     )
                     or dependency.dbid = database.oid
                     and dependency.classid = 'pg_catalog.pg_proc'::regclass
                     and dependency.objid in (
                       select routine.oid
                         from pg_catalog.pg_proc as routine
                         join pg_catalog.pg_namespace as namespace
                           on namespace.oid = routine.pronamespace
                        where namespace.nspname = 'public'
                          and routine.proowner = role.oid
                     )
                     or dependency.dbid = database.oid
                     and dependency.classid = 'pg_catalog.pg_type'::regclass
                     and dependency.objid in (
                       select data_type.oid
                         from pg_catalog.pg_type as data_type
                         join pg_catalog.pg_namespace as namespace
                           on namespace.oid = data_type.typnamespace
                        where namespace.nspname = 'public'
                          and data_type.typowner = role.oid
                     )
                   )
                 )
            ) as "safe!""#,
        role_name,
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| authority_sql_error("verify migration role ownership surface", error))?;
    if !safe {
        return Err(Error::Invalid {
            message: "the migration role may own only this database and its public schema, relations, routines and types".to_owned(),
        });
    }
    Ok(())
}

/// Pre-migration proof for one configured runtime login. This proves the
/// selected session plus the complete cluster role/membership/database-ACL
/// shape without requiring application objects which migration creates.
pub async fn verify_runtime_prerequisites_connection(
    connection: &mut PgConnection,
    expected_principal: &str,
    database_roles: &DatabaseRoles,
) -> Result<VerifiedRuntimeRole> {
    if !database_roles.runtime().contains(expected_principal) {
        return Err(Error::Invalid {
            message:
                "the expected process login is not in the configured runtime database role set"
                    .to_owned(),
        });
    }
    verify_capability_prerequisites_connection(connection, database_roles).await?;
    let selected = sqlx::query_scalar!(
        r#"select current_user = session_user and current_user = $1 as "selected!""#,
        expected_principal,
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| authority_sql_error("verify selected pre-migration runtime login", error))?;
    if !selected {
        return Err(Error::Invalid {
            message: "the pre-migration runtime session does not use the configured login"
                .to_owned(),
        });
    }
    Ok(VerifiedRuntimeRole {
        name: expected_principal.to_owned(),
    })
}

/// Verifies that the pool's session/current principal is the expected
/// ordinary member of the schema-owned `synveda_app` capability role.
#[tracing::instrument(name = "store.runtime_role.verify", skip_all, err(Display))]
pub async fn verify(
    pool: &PgPool,
    expected_principal: &str,
    database_roles: &DatabaseRoles,
) -> Result<VerifiedRuntimeRole> {
    let mut connection = pool
        .acquire()
        .await
        .map_err(|error| authority_sql_error("acquire runtime database role connection", error))?;
    initialize_product_session_connection(&mut connection).await?;
    let mut authority = connection
        .begin()
        .await
        .map_err(|error| authority_sql_error("begin runtime role authority snapshot", error))?;
    configure_authority_snapshot_connection(&mut authority).await?;
    let verified = verify_connection(&mut authority, expected_principal, database_roles).await?;
    authority
        .commit()
        .await
        .map_err(|error| authority_sql_error("finish runtime role authority snapshot", error))?;
    Ok(verified)
}

/// Connection-scoped runtime authority proof.
pub async fn verify_connection(
    connection: &mut PgConnection,
    expected_principal: &str,
    database_roles: &DatabaseRoles,
) -> Result<VerifiedRuntimeRole> {
    verify_selected(connection, Some(expected_principal), database_roles).await
}

async fn verify_selected(
    connection: &mut PgConnection,
    expected_principal: Option<&str>,
    database_roles: &DatabaseRoles,
) -> Result<VerifiedRuntimeRole> {
    if expected_principal.is_some_and(|name| !database_roles.runtime().contains(name)) {
        return Err(Error::Invalid {
            message:
                "the expected process login is not in the configured runtime database role set"
                    .to_owned(),
        });
    }
    verify_capability_role_connection(connection, database_roles).await?;
    let facts = sqlx::query!(
        r#"select current_user as "name!", session_user as "session_name!",
                  rolcanlogin as "can_login!",
                  rolinherit as "inherits!",
                  rolsuper as "superuser!", rolcreatedb as "create_db!",
                  rolcreaterole as "create_role!", rolreplication as "replication!",
                  rolbypassrls as "bypass_rls!",
                  rolconnlimit as "connection_limit!",
                  (rolvaliduntil is null
                   or rolvaliduntil > pg_catalog.statement_timestamp())
                    as "credential_current!",
                  pg_catalog.pg_has_role(current_user, 'synveda_app', 'member') as "app_member!",
                  exists (
                    select 1
                      from pg_catalog.pg_auth_members as membership
                      join pg_catalog.pg_roles as granted_role
                        on granted_role.oid = membership.roleid
                     where membership.member = roles.oid
                       and granted_role.rolname = 'synveda_app'
                       and not membership.admin_option
                       and membership.inherit_option
                       and membership.set_option
                  ) as "app_membership_safe!",
                  exists (
                    select 1
                      from pg_catalog.pg_auth_members as membership
                      join pg_catalog.pg_roles as granted_role
                        on granted_role.oid = membership.roleid
                     where membership.member = roles.oid
                       and granted_role.rolname = 'synveda_app'
                       and (
                         membership.admin_option
                         or not membership.inherit_option
                         or not membership.set_option
                       )
                  ) as "app_membership_unsafe!",
                  exists (
                    select 1 from pg_catalog.pg_database
                     where datdba = roles.oid
                  ) as "database_owner!",
                  exists (
                    select 1 from pg_catalog.pg_namespace
                     where nspowner = roles.oid
                  ) as "schema_owner!",
                  exists (
                    select 1
                      from pg_catalog.pg_class as objects
                     where objects.relowner = roles.oid
                    union all
                    select 1
                      from pg_catalog.pg_proc as routines
                     where routines.proowner = roles.oid
                    union all
                    select 1
                      from pg_catalog.pg_type as data_type
                     where data_type.typowner = roles.oid
                  ) as "application_object_owner!",
                  exists (
                    select 1
                      from pg_catalog.pg_auth_members as memberships
                      join pg_catalog.pg_roles as granted_roles
                        on granted_roles.oid = memberships.roleid
                     where memberships.member = roles.oid
                       and granted_roles.rolname <> 'synveda_app'
                  ) as "unexpected_membership!",
                  exists (
                    select 1
                      from pg_catalog.pg_auth_members as memberships
                      join pg_catalog.pg_roles as member
                        on member.oid = memberships.member
                      join pg_catalog.pg_roles as grantor
                        on grantor.oid = memberships.grantor
                     where memberships.roleid = roles.oid
                       and not (
                         exists (
                           select 1
                             from pg_catalog.unnest($2::text[]) with ordinality
                                    as expected_member(role_name, position)
                             join pg_catalog.unnest($3::text[]) with ordinality
                                    as expected_grantor(role_name, position)
                               using (position)
                            where expected_member.role_name = member.rolname
                              and expected_grantor.role_name = grantor.rolname
                         )
                         and memberships.admin_option
                         and not memberships.inherit_option
                         and not memberships.set_option
                       )
                  ) as "unsafe_inbound_membership!",
                  exists (
                    select 1
                      from pg_catalog.pg_database as database,
                           lateral pg_catalog.aclexplode(
                             coalesce(
                               database.datacl,
                               pg_catalog.acldefault('d', database.datdba)
                             )
                           ) as acl
                     where database.datname = pg_catalog.current_database()
                       and acl.grantee = roles.oid
                       and acl.privilege_type = 'CONNECT'
                       and not acl.is_grantable
                  ) as "direct_connect!",
                  exists (
                    select 1
                      from pg_catalog.pg_database as database,
                           lateral pg_catalog.aclexplode(
                             coalesce(
                               database.datacl,
                               pg_catalog.acldefault('d', database.datdba)
                             )
                           ) as acl
                     where database.datname = pg_catalog.current_database()
                       and acl.grantee = roles.oid
                       and (
                         acl.privilege_type <> 'CONNECT'
                         or acl.is_grantable
                       )
                  ) as "unsafe_direct_database_acl!",
                  exists (
                    select 1
                      from pg_catalog.pg_database as database,
                           lateral pg_catalog.aclexplode(
                             coalesce(
                               database.datacl,
                               pg_catalog.acldefault('d', database.datdba)
                             )
                           ) as acl
                     where database.datname = pg_catalog.current_database()
                       and acl.grantee = 0
                  ) as "public_database_acl!",
                  exists (
                    select 1
                      from pg_catalog.pg_database as database,
                           lateral pg_catalog.aclexplode(
                             coalesce(
                               database.datacl,
                               pg_catalog.acldefault('d', database.datdba)
                             )
                           ) as acl
                     where database.datname <> pg_catalog.current_database()
                       and acl.grantee = roles.oid
                  ) as "other_database_acl!",
                  exists (
                    select 1
                      from pg_catalog.pg_namespace as namespace,
                           lateral pg_catalog.aclexplode(namespace.nspacl) as acl
                     where acl.grantee = roles.oid
                    union all
                    select 1
                      from pg_catalog.pg_attribute as attribute,
                           lateral pg_catalog.aclexplode(attribute.attacl) as acl
                     where acl.grantee = roles.oid
                    union all
                    select 1
                      from pg_catalog.pg_class as object,
                           lateral pg_catalog.aclexplode(object.relacl) as acl
                     where acl.grantee = roles.oid
                    union all
                    select 1
                      from pg_catalog.pg_proc as routine,
                           lateral pg_catalog.aclexplode(routine.proacl) as acl
                     where acl.grantee = roles.oid
                    union all
                    select 1
                      from pg_catalog.pg_type as data_type,
                           lateral pg_catalog.aclexplode(data_type.typacl) as acl
                     where acl.grantee = roles.oid
                    union all
                    select 1
                      from pg_catalog.pg_default_acl as defaults,
                           lateral pg_catalog.aclexplode(defaults.defaclacl) as acl
                     where acl.grantee = roles.oid
                  ) as "direct_object_acl!",
                  exists (
                    select 1
                      from pg_catalog.pg_db_role_setting
                     where setrole = roles.oid
                  ) as "role_setting!"
             from pg_catalog.pg_roles as roles
            where rolname = current_user
              and ($1::text is null or current_user = $1)"#,
        expected_principal,
        database_roles.runtime().administrative_members(),
        database_roles.runtime().administrative_grantors(),
    )
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| authority_sql_error("verify runtime database role", error))?
    .ok_or_else(|| Error::Invalid {
        message: "the process database session does not use the selected expected PostgreSQL login"
            .to_owned(),
    })?;
    if facts.name != facts.session_name
        || !facts.can_login
        || !facts.inherits
        || facts.superuser
        || facts.create_db
        || facts.create_role
        || facts.replication
        || facts.bypass_rls
        || facts.connection_limit != -1
        || !facts.credential_current
        || !facts.app_member
        || !facts.app_membership_safe
        || facts.app_membership_unsafe
        || facts.database_owner
        || facts.schema_owner
        || facts.application_object_owner
        || facts.unexpected_membership
        || facts.unsafe_inbound_membership
        || !facts.direct_connect
        || facts.unsafe_direct_database_acl
        || facts.public_database_acl
        || facts.other_database_acl
        || facts.direct_object_acl
        || facts.role_setting
    {
        return Err(Error::Invalid {
            message: "the process database session and current principal must be the same \
                      expected LOGIN role with INHERIT, must own no database and no schema, \
                      relation, routine or type in the selected database, must be \
                      non-superuser and non-BYPASSRLS, unable to create roles/databases or \
                      replicate, have no privilege-bearing inbound membership, and be an \
                      inheriting non-admin member only of synveda_app; \
                      it must have only a direct non-grantable CONNECT ACL on the selected \
                      database, no PUBLIC database ACL, no direct/default object ACL or role \
                      setting, and no direct ACL on another database"
                .to_owned(),
        });
    }
    Ok(VerifiedRuntimeRole { name: facts.name })
}

/// Verifies the narrow principal which owns and advances the application
/// schema. This role is deliberately distinct from every request/worker
/// login and has no cluster capability beyond ownership of the selected
/// database and its `public` schema.
#[tracing::instrument(name = "store.runtime_role.verify_migrator", skip_all, err(Display))]
pub async fn verify_migrator(
    pool: &PgPool,
    database_roles: &DatabaseRoles,
) -> Result<VerifiedRuntimeRole> {
    let mut connection = pool.acquire().await.map_err(|error| {
        authority_sql_error("acquire migration database role connection", error)
    })?;
    initialize_product_session_connection(&mut connection).await?;
    let mut authority = connection
        .begin()
        .await
        .map_err(|error| authority_sql_error("begin migration role authority snapshot", error))?;
    configure_authority_snapshot_connection(&mut authority).await?;
    let verified = verify_migrator_connection(&mut authority, database_roles).await?;
    authority
        .commit()
        .await
        .map_err(|error| authority_sql_error("finish migration role authority snapshot", error))?;
    Ok(verified)
}

/// Connection-scoped migration authority proof.
pub async fn verify_migrator_connection(
    connection: &mut PgConnection,
    database_roles: &DatabaseRoles,
) -> Result<VerifiedRuntimeRole> {
    verify_selected_migrator(connection, database_roles, true).await
}

/// Pre-migration migrator proof used by deployment preflight.
pub async fn verify_migrator_prerequisites_connection(
    connection: &mut PgConnection,
    database_roles: &DatabaseRoles,
) -> Result<VerifiedRuntimeRole> {
    verify_selected_migrator(connection, database_roles, false).await
}

async fn verify_selected_migrator(
    connection: &mut PgConnection,
    database_roles: &DatabaseRoles,
    require_application_acl: bool,
) -> Result<VerifiedRuntimeRole> {
    if require_application_acl {
        verify_capability_role_connection(connection, database_roles).await?;
    } else {
        verify_capability_prerequisites_connection(connection, database_roles).await?;
    }
    let facts = sqlx::query!(
        r#"select current_user as "name!", session_user as "session_name!",
                  role.rolcanlogin as "can_login!",
                  role.rolinherit as "inherits!",
                  role.rolsuper as "superuser!",
                  role.rolcreatedb as "create_db!",
                  role.rolcreaterole as "create_role!",
                  role.rolreplication as "replication!",
                  role.rolbypassrls as "bypass_rls!",
                  role.rolconnlimit as "connection_limit!",
                  (role.rolvaliduntil is null
                   or role.rolvaliduntil > pg_catalog.statement_timestamp())
                    as "credential_current!",
                  exists (
                    select 1 from pg_catalog.pg_database as database
                     where database.datdba = role.oid
                       and database.datname = pg_catalog.current_database()
                  ) as "owns_database!",
                  exists (
                    select 1 from pg_catalog.pg_database as database
                     where database.datdba = role.oid
                       and database.datname <> pg_catalog.current_database()
                  ) as "owns_other_database!",
                  exists (
                    select 1 from pg_catalog.pg_namespace as namespace
                     where namespace.nspname = 'public'
                       and namespace.nspowner = role.oid
                  ) as "owns_public_schema!",
                  exists (
                    select 1 from pg_catalog.pg_namespace as namespace
                     where namespace.nspname <> 'public'
                       and namespace.nspowner = role.oid
                  ) as "owns_other_schema!",
                  exists (
                    select 1 from pg_catalog.pg_auth_members as membership
                     where membership.member = role.oid
                  ) as "membership!",
                  exists (
                    select 1
                      from pg_catalog.pg_auth_members as membership
                      join pg_catalog.pg_roles as member
                        on member.oid = membership.member
                      join pg_catalog.pg_roles as grantor
                        on grantor.oid = membership.grantor
                     where membership.roleid = role.oid
                       and not (
                         exists (
                           select 1
                             from pg_catalog.unnest($2::text[]) with ordinality
                                    as expected_member(role_name, position)
                             join pg_catalog.unnest($3::text[]) with ordinality
                                    as expected_grantor(role_name, position)
                               using (position)
                            where expected_member.role_name = member.rolname
                              and expected_grantor.role_name = grantor.rolname
                         )
                         and membership.admin_option
                         and not membership.inherit_option
                         and not membership.set_option
                       )
                  ) as "unsafe_inbound_membership!",
                  exists (
                    select 1
                      from pg_catalog.pg_database as database,
                           lateral pg_catalog.aclexplode(
                             coalesce(
                               database.datacl,
                               pg_catalog.acldefault('d', database.datdba)
                             )
                           ) as acl
                     where database.datname <> pg_catalog.current_database()
                       and acl.grantee = role.oid
                  ) as "other_database_acl!",
                  exists (
                    select 1
                      from pg_catalog.pg_database as database,
                           lateral pg_catalog.aclexplode(
                             coalesce(
                               database.datacl,
                               pg_catalog.acldefault('d', database.datdba)
                             )
                           ) as acl
                     where database.datname = pg_catalog.current_database()
                       and acl.grantee = 0
                  ) as "public_database_acl!",
                  exists (
                    select 1
                      from pg_catalog.pg_database as database,
                           lateral pg_catalog.aclexplode(
                             coalesce(
                               database.datacl,
                               pg_catalog.acldefault('d', database.datdba)
                             )
                           ) as acl
                     where database.datname = pg_catalog.current_database()
                       and acl.grantee = role.oid
                       and not (
                         acl.grantor = role.oid
                         and acl.privilege_type in ('CREATE', 'CONNECT', 'TEMPORARY')
                         and not acl.is_grantable
                       )
                  ) as "unsafe_current_database_acl!",
                  (
                    select count(*)
                      from pg_catalog.pg_database as database,
                           lateral pg_catalog.aclexplode(
                             coalesce(
                               database.datacl,
                               pg_catalog.acldefault('d', database.datdba)
                             )
                           ) as acl
                     where database.datname = pg_catalog.current_database()
                       and acl.grantee = role.oid
                       and acl.grantor = role.oid
                       and acl.privilege_type in ('CREATE', 'CONNECT', 'TEMPORARY')
                       and not acl.is_grantable
                  ) as "current_database_acl_count!",
                  exists (
                    select 1 from pg_catalog.pg_db_role_setting
                     where setrole = role.oid
                  ) as "role_setting!"
            from pg_catalog.pg_roles as role
            where role.rolname = current_user
              and current_user = $1"#,
        database_roles.migrator(),
        database_roles.runtime().administrative_members(),
        database_roles.runtime().administrative_grantors(),
    )
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| authority_sql_error("verify migration database role", error))?
    .ok_or_else(|| Error::Invalid {
        message: "the migration session does not use the exact expected PostgreSQL login"
            .to_owned(),
    })?;
    if facts.name != facts.session_name
        || !facts.can_login
        || !facts.inherits
        || facts.superuser
        || facts.create_db
        || facts.create_role
        || facts.replication
        || facts.bypass_rls
        || facts.connection_limit != -1
        || !facts.credential_current
        || !facts.owns_database
        || facts.owns_other_database
        || !facts.owns_public_schema
        || facts.owns_other_schema
        || facts.membership
        || facts.unsafe_inbound_membership
        || facts.other_database_acl
        || facts.public_database_acl
        || facts.unsafe_current_database_acl
        || facts.current_database_acl_count != 3
        || facts.role_setting
    {
        return Err(Error::Invalid {
            message: "the migration session must use the exact ordinary LOGIN role which \
                      owns only the selected database and its public schema, has no role \
                      memberships, has no privilege-bearing inbound membership or direct \
                      ACL on another database, accepts no PUBLIC database ACL, owns no other \
                      schema, has no role settings, and has no elevated cluster capability"
                .to_owned(),
        });
    }
    verify_migrator_ownership(connection, &facts.name).await?;
    verify_no_global_or_default_acl(connection, std::slice::from_ref(&facts.name)).await?;
    Ok(VerifiedRuntimeRole { name: facts.name })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    const MAX_TEST_CONFIGURATION_BYTES: u64 = 4096;

    fn private_test_configuration(setting: &str) -> Option<String> {
        let path = std::env::var_os(setting)?;
        let path = Path::new(&path);
        let metadata = std::fs::symlink_metadata(path)
            .unwrap_or_else(|_| panic!("{setting} test configuration is unavailable"));
        assert!(
            metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
            "{setting} must name a regular non-symlink test file"
        );
        assert!(
            (1..=MAX_TEST_CONFIGURATION_BYTES).contains(&metadata.len()),
            "{setting} must contain 1..={MAX_TEST_CONFIGURATION_BYTES} bytes"
        );
        let value = std::fs::read_to_string(path)
            .unwrap_or_else(|_| panic!("{setting} test configuration must contain UTF-8"));
        let value = value.strip_suffix('\n').unwrap_or(&value);
        assert!(
            !value.is_empty() && !value.contains(['\0', '\r', '\n']),
            "{setting} must contain one non-empty line"
        );
        Some(value.to_owned())
    }

    fn live_test_roles() -> Option<DatabaseRoles> {
        let value = private_test_configuration("SYNVEDA_DATABASE_ROLES_FILE")?;
        Some(DatabaseRoles::parse_json(&value).expect("parse test database role contract"))
    }

    async fn live_test_connection(setting: &str) -> Option<PgConnection> {
        let value = private_test_configuration(setting)?;
        let options =
            crate::database_url::parse(setting, &value).expect("parse isolated test database URL");
        Some(
            sqlx::Connection::connect_with(&options)
                .await
                .expect("connect isolated test database"),
        )
    }

    async fn live_test_pool(setting: &str) -> Option<PgPool> {
        let value = private_test_configuration(setting)?;
        let options =
            crate::database_url::parse(setting, &value).expect("parse isolated test database URL");
        Some(
            sqlx::postgres::PgPoolOptions::new()
                .max_connections(1)
                .connect_with(options)
                .await
                .expect("connect isolated test database pool"),
        )
    }

    async fn assert_raw_default_session(pool: &PgPool) {
        let search_path = sqlx::query_scalar!(
            r#"select pg_catalog.current_setting('search_path') as "search_path!""#,
        )
        .fetch_one(pool)
        .await
        .expect("read raw PostgreSQL search path");
        assert_ne!(
            search_path, "public",
            "regression fixture must begin with the PostgreSQL default search path"
        );
    }

    #[test]
    fn authority_sql_errors_separate_outages_from_permanent_refusals() {
        for transient in [
            "08006", "40001", "40P01", "53300", "55P03", "57014", "57P01", "58030", "XX000",
        ] {
            assert!(
                transient_authority_sqlstate(transient),
                "{transient} must remain retryable"
            );
        }
        for permanent in ["42501", "42P01", "42883", "0A000", "23514"] {
            assert!(
                !transient_authority_sqlstate(permanent),
                "{permanent} must close the authority gate"
            );
        }
        assert!(matches!(
            authority_sql_error("test authority", sqlx::Error::PoolTimedOut),
            Error::Storage { .. }
        ));
        assert!(matches!(
            authority_sql_error("test authority", sqlx::Error::RowNotFound),
            Error::Invalid { .. }
        ));
    }

    #[test]
    fn database_role_contract_is_closed_and_provider_neutral() {
        let roles = DatabaseRoles::parse_json(
            r#"{"migrator":"owner@provider","gateway":"gateway-user","worker":"worker.user","administrators":["admin@provider"],"administrative_memberships":[{"member":"admin@provider","grantor":"bootstrap@provider"}],"forbidden_databases":["identity-db"],"isolated_peer_roles":["identity-role"]}"#,
        )
        .expect("parse provider-assigned role names");
        assert_eq!(roles.migrator(), "owner@provider");
        assert!(roles.runtime().contains("gateway-user"));
        assert_eq!(roles.runtime().administrators(), &["admin@provider"]);
        assert_eq!(
            roles.runtime().administrative_members(),
            &["admin@provider"]
        );
        assert_eq!(
            roles.runtime().administrative_grantors(),
            &["bootstrap@provider"]
        );
        assert_eq!(roles.forbidden_databases(), &["identity-db"]);
        assert_eq!(roles.isolated_peer_roles(), &["identity-role"]);

        for refused in [
            r#"{"migrator":"same","gateway":"same","worker":"worker","administrators":["admin"],"administrative_memberships":[],"forbidden_databases":["postgres"],"isolated_peer_roles":[]}"#,
            r#"{"migrator":"m","gateway":"g","worker":"w","administrators":[],"administrative_memberships":[],"forbidden_databases":["postgres"],"isolated_peer_roles":[],"extra":true}"#,
            r#"{"migrator":"m","gateway":"g","worker":"w","administrators":["g"],"administrative_memberships":[],"forbidden_databases":["postgres"],"isolated_peer_roles":[]}"#,
            r#"{"migrator":"m","gateway":"g","worker":"w","administrators":["admin"],"administrative_memberships":[],"forbidden_databases":[],"isolated_peer_roles":[]}"#,
            r#"{"migrator":"synveda_app","gateway":"g","worker":"w","administrators":["admin"],"administrative_memberships":[],"forbidden_databases":["postgres"],"isolated_peer_roles":[]}"#,
            r#"{"migrator":"m","gateway":"g","worker":"w","administrators":["synveda_app"],"administrative_memberships":[],"forbidden_databases":["postgres"],"isolated_peer_roles":[]}"#,
            r#"{"migrator":"m","gateway":"g","worker":"w","administrators":["admin"],"administrative_memberships":[{"member":"admin","grantor":"g"}],"forbidden_databases":["postgres"],"isolated_peer_roles":[]}"#,
            r#"{"migrator":"m","gateway":"g","worker":"w","administrators":["admin"],"administrative_memberships":[{"member":"admin","grantor":"admin"}],"forbidden_databases":["postgres"],"isolated_peer_roles":[]}"#,
            r#"{"migrator":"m","gateway":"g","worker":"w","administrators":["admin"],"administrative_memberships":[{"member":"other","grantor":"bootstrap"}],"forbidden_databases":["postgres"],"isolated_peer_roles":[]}"#,
            r#"{"migrator":"m","gateway":"g","worker":"w","administrators":["admin"],"administrative_memberships":[{"member":"admin","grantor":"bootstrap"},{"member":"admin","grantor":"bootstrap"}],"forbidden_databases":["postgres"],"isolated_peer_roles":[]}"#,
            r#"{"migrator":"m","gateway":"g","worker":"w","administrators":["admin"],"administrative_memberships":[],"forbidden_databases":["identity","identity"],"isolated_peer_roles":[]}"#,
            r#"{"migrator":"m","gateway":"g","worker":"w","administrators":["admin"],"administrative_memberships":[],"forbidden_databases":["postgres"],"isolated_peer_roles":["admin"]}"#,
            r#"{"migrator":"m","gateway":"g","worker":"w","administrators":["admin"],"administrative_memberships":[],"forbidden_databases":["postgres"],"isolated_peer_roles":["peer","peer"]}"#,
            r#"{"migrator":"m","gateway":"g","worker":"w","administrators":["admin"],"administrative_memberships":[]}"#,
        ] {
            assert!(DatabaseRoles::parse_json(refused).is_err());
        }
    }

    #[tokio::test]
    async fn live_catalog_fingerprints_match_the_revision_constants() {
        let Some(mut connection) = live_test_connection("SYNVEDA_TEST_DATABASE_URL_FILE").await
        else {
            eprintln!(
                "skipping live catalogue fingerprints: SYNVEDA_TEST_DATABASE_URL_FILE is not set"
            );
            return;
        };
        let roles = live_test_roles().expect("test database role contract is required");
        let actual = application_acl_fingerprint(&mut connection)
            .await
            .expect("fingerprint ACL inventory");
        assert_eq!(actual, APPLICATION_ACL_FINGERPRINT, "actual={actual}");
        let actual = rls_catalog_fingerprint(&mut connection)
            .await
            .expect("fingerprint forced-RLS inventory");
        assert_eq!(actual, RLS_CATALOG_FINGERPRINT, "actual={actual}");
        let routine = routine_catalog_fingerprint(&mut connection, roles.migrator())
            .await
            .expect("fingerprint application routines");
        let trigger = trigger_catalog_fingerprint(&mut connection, roles.migrator())
            .await
            .expect("fingerprint application triggers");
        assert_eq!(
            (routine.as_str(), trigger.as_str(),),
            (ROUTINE_CATALOG_FINGERPRINT, TRIGGER_CATALOG_FINGERPRINT),
            "actual routine={routine}, trigger={trigger}"
        );
    }

    #[tokio::test]
    #[ignore = "run only through the isolated authority-fingerprint fixture"]
    async fn report_live_catalog_fingerprints() {
        assert_eq!(
            std::env::var("SYNVEDA_REPORT_AUTHORITY_FINGERPRINTS").as_deref(),
            Ok("1"),
            "the authority fingerprint reporter requires its exact harness gate"
        );
        let mut connection = live_test_connection("SYNVEDA_TEST_DATABASE_URL_FILE")
            .await
            .expect("the exact gateway database URL is required");
        let roles = live_test_roles().expect("the exact database role contract is required");
        initialize_product_session_connection(&mut connection)
            .await
            .expect("normalise the exact gateway database session");
        let mut authority = sqlx::Connection::begin(&mut connection)
            .await
            .expect("begin authority fingerprint snapshot");
        configure_authority_snapshot_connection(&mut authority)
            .await
            .expect("configure authority fingerprint snapshot");
        let application_acl = application_acl_fingerprint(&mut authority)
            .await
            .expect("fingerprint ACL inventory");
        let routine_catalog = routine_catalog_fingerprint(&mut authority, roles.migrator())
            .await
            .expect("fingerprint application routines");
        let trigger_catalog = trigger_catalog_fingerprint(&mut authority, roles.migrator())
            .await
            .expect("fingerprint application triggers");
        let forced_rls = rls_catalog_fingerprint(&mut authority)
            .await
            .expect("fingerprint forced-RLS inventory");
        authority
            .commit()
            .await
            .expect("finish authority fingerprint snapshot");

        println!(
            "authority-fingerprints baseline_revision={} application_acl={application_acl} routine_catalog={routine_catalog} trigger_catalog={trigger_catalog} forced_rls={forced_rls}",
            crate::epoch::CURRENT_BASELINE_REVISION
        );
    }

    #[tokio::test]
    async fn pool_verifiers_initialize_a_raw_postgresql_session() {
        let Some(gateway_pool) = live_test_pool("SYNVEDA_TEST_DATABASE_URL_FILE").await else {
            eprintln!(
                "skipping raw-pool verifier acceptance: SYNVEDA_TEST_DATABASE_URL_FILE is not set"
            );
            return;
        };
        let roles = live_test_roles().expect("test database role contract is required");
        assert_raw_default_session(&gateway_pool).await;
        let verified = verify(&gateway_pool, roles.gateway(), &roles)
            .await
            .expect("verify raw gateway pool after product-session initialization");
        assert_eq!(verified.name, roles.gateway());
        let mut missing_peer_roles = roles.clone();
        missing_peer_roles.forbidden_databases = vec!["synveda_missing_peer".to_owned()];
        let error = verify(&gateway_pool, roles.gateway(), &missing_peer_roles)
            .await
            .expect_err("a missing configured peer database must fail closed");
        assert!(
            error
                .to_string()
                .contains("every configured isolated peer database and role must exist"),
            "unexpected missing-peer error: {error}"
        );
        let mut missing_peer_roles = roles.clone();
        missing_peer_roles.isolated_peer_roles = vec!["synveda_missing_peer_role".to_owned()];
        let error = verify(&gateway_pool, roles.gateway(), &missing_peer_roles)
            .await
            .expect_err("a missing configured peer role must fail closed");
        assert!(
            error
                .to_string()
                .contains("every configured isolated peer database and role must exist"),
            "unexpected missing-peer-role error: {error}"
        );

        let migrator_setting = "SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE";
        let migrator_capability_pool = live_test_pool(migrator_setting)
            .await
            .expect("exact migrator test database URL file is required");
        assert_raw_default_session(&migrator_capability_pool).await;
        verify_capability_role(&migrator_capability_pool, &roles)
            .await
            .expect("verify capability catalogue through a raw pool");

        let migrator_pool = live_test_pool(migrator_setting)
            .await
            .expect("exact migrator test database URL file is required");
        assert_raw_default_session(&migrator_pool).await;
        let verified = verify_migrator(&migrator_pool, &roles)
            .await
            .expect("verify exact migrator through a raw pool");
        assert_eq!(verified.name, roles.migrator());
    }

    #[tokio::test]
    #[ignore = "requires the exact isolated migrator credential and serial catalogue mutation"]
    async fn routine_and_trigger_drift_are_refused_and_transactionally_restored() {
        let Some(mut connection) =
            live_test_connection("SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE").await
        else {
            eprintln!(
                "skipping catalogue drift acceptance: SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE is not set"
            );
            return;
        };
        let roles = live_test_roles().expect("test database role contract is required");
        let mut runtime_connection = live_test_connection("SYNVEDA_TEST_DATABASE_URL_FILE")
            .await
            .expect("exact gateway test database URL file is required");
        let migrator_identity = database_identity_connection(&mut connection)
            .await
            .expect("identify exact test migrator target");
        let runtime_identity = database_identity_connection(&mut runtime_connection)
            .await
            .expect("identify exact test runtime target");
        assert_eq!(
            migrator_identity, runtime_identity,
            "test runtime and migrator credentials must target one live database"
        );
        initialize_product_session_connection(&mut connection)
            .await
            .expect("normalise exact test migrator session");
        verify_migrator_connection(&mut connection, &roles)
            .await
            .expect("accept the exact test migrator and migrated catalogue baseline");

        let mut transaction = sqlx::Connection::begin(&mut connection)
            .await
            .expect("begin routine body drift transaction");
        sqlx::query!(
            r#"create or replace function public.synveda_knowledge_tags_canonical(value text[])
               returns boolean
               language sql immutable parallel safe
               as 'select false'"#
        )
        .execute(&mut *transaction)
        .await
        .expect("replace one application routine body");
        let routine = routine_catalog_fingerprint(&mut transaction, roles.migrator())
            .await
            .expect("fingerprint body-drifted application routines");
        assert_ne!(routine, ROUTINE_CATALOG_FINGERPRINT);
        let error = verify_capability_role_connection(&mut transaction, &roles)
            .await
            .expect_err("routine body drift must close the authority sentinel");
        assert!(
            error
                .to_string()
                .contains("application routine definition inventory"),
            "unexpected routine-body-drift error: {error}"
        );
        transaction.rollback().await.expect("restore routine body");

        let mut transaction = sqlx::Connection::begin(&mut connection)
            .await
            .expect("begin routine drift transaction");
        sqlx::query!("alter function public.synveda_knowledge_tags_canonical(text[]) volatile")
            .execute(&mut *transaction)
            .await
            .expect("alter one application routine attribute");
        let routine = routine_catalog_fingerprint(&mut transaction, roles.migrator())
            .await
            .expect("fingerprint drifted application routines");
        assert_ne!(routine, ROUTINE_CATALOG_FINGERPRINT);
        let error = verify_capability_role_connection(&mut transaction, &roles)
            .await
            .expect_err("routine drift must close the authority sentinel");
        assert!(
            error
                .to_string()
                .contains("application routine definition inventory"),
            "unexpected routine-drift error: {error}"
        );
        transaction
            .rollback()
            .await
            .expect("restore routine definition");

        let mut transaction = sqlx::Connection::begin(&mut connection)
            .await
            .expect("begin trigger drift transaction");
        sqlx::query!("alter table public.audit_log disable trigger audit_log_no_update")
            .execute(&mut *transaction)
            .await
            .expect("disable one immutable-table trigger");
        let trigger = trigger_catalog_fingerprint(&mut transaction, roles.migrator())
            .await
            .expect("fingerprint drifted application triggers");
        assert_ne!(trigger, TRIGGER_CATALOG_FINGERPRINT);
        let error = verify_capability_role_connection(&mut transaction, &roles)
            .await
            .expect_err("trigger drift must close the authority sentinel");
        assert!(
            error
                .to_string()
                .contains("application trigger definition inventory"),
            "unexpected trigger-drift error: {error}"
        );
        transaction
            .rollback()
            .await
            .expect("restore trigger definition");

        verify_capability_role_connection(&mut connection, &roles)
            .await
            .expect("accept the restored catalogue baseline");
    }
}
