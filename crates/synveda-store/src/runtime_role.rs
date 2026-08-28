//! Runtime database-principal verification for product processes.
//!
//! Deployment bootstrap owns login creation. Application processes use this
//! read-only sentinel to refuse owner, superuser, `BYPASSRLS`, non-login or
//! ungranted credentials before they become ready.

use sqlx::PgPool;
use synveda_types::{Error, Result};

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
    let identity = sqlx::query!(
        r#"select current_database() as "database!",
                  control.system_identifier::text as "cluster_system_identifier!",
                  database.oid::bigint as "database_oid!",
                  pg_catalog.pg_postmaster_start_time() as "postmaster_started_at!",
                  pg_catalog.pg_is_in_recovery() as "in_recovery!",
                  current_setting('transaction_read_only')::boolean
                    as "transaction_read_only!"
             from pg_catalog.pg_control_system() as control
             join pg_catalog.pg_database as database
               on database.datname = current_database()"#,
    )
    .fetch_one(pool)
    .await
    .map_err(|error| Error::Storage {
        message: format!("read runtime database target identity: {error}"),
    })?;
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

/// Verifies the schema-owned capability role inherited by every runtime
/// login. A safe login is not sufficient if `SET ROLE synveda_app` could
/// acquire ownership or elevated cluster capabilities.
#[tracing::instrument(
    name = "store.runtime_role.verify_capability_role",
    skip_all,
    err(Display)
)]
pub async fn verify_capability_role(pool: &PgPool) -> Result<()> {
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
                and not exists (
                  select 1 from pg_catalog.pg_database
                   where datdba = app.oid
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
                )
                and not exists (
                  select 1 from pg_catalog.pg_auth_members
                   where member = app.oid
                )
           ) as "safe!""#,
    )
    .fetch_one(pool)
    .await
    .map_err(|error| Error::Storage {
        message: format!("verify runtime database capability role: {error}"),
    })?;
    if !safe {
        return Err(Error::Invalid {
            message: "synveda_app must be an inheriting NOLOGIN role with no elevated \
                      cluster capabilities or role memberships, must own no database, and \
                      must own no schema, relation or routine in the selected database"
                .to_owned(),
        });
    }
    Ok(())
}

/// Verifies that the pool's session/current principal is the expected
/// ordinary member of the schema-owned `synveda_app` capability role.
#[tracing::instrument(name = "store.runtime_role.verify", skip_all, err(Display))]
pub async fn verify(pool: &PgPool, expected_principal: &str) -> Result<VerifiedRuntimeRole> {
    verify_capability_role(pool).await?;
    let facts = sqlx::query!(
        r#"select current_user as "name!", session_user as "session_name!",
                  rolcanlogin as "can_login!",
                  rolinherit as "inherits!",
                  rolsuper as "superuser!", rolcreatedb as "create_db!",
                  rolcreaterole as "create_role!", rolreplication as "replication!",
                  rolbypassrls as "bypass_rls!",
                  pg_has_role(current_user, 'synveda_app', 'member') as "app_member!",
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
                  ) as "application_object_owner!",
                  exists (
                    select 1
                      from pg_catalog.pg_auth_members as memberships
                      join pg_catalog.pg_roles as granted_roles
                        on granted_roles.oid = memberships.roleid
                     where memberships.member = roles.oid
                       and granted_roles.rolname <> 'synveda_app'
                  ) as "unexpected_membership!"
             from pg_catalog.pg_roles as roles
            where rolname = current_user and current_user = $1"#,
        expected_principal,
    )
    .fetch_optional(pool)
    .await
    .map_err(|error| Error::Storage {
        message: format!("verify runtime database role: {error}"),
    })?
    .ok_or_else(|| Error::Invalid {
        message: "the process database session does not use the exact expected PostgreSQL login"
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
        || !facts.app_member
        || !facts.app_membership_safe
        || facts.app_membership_unsafe
        || facts.database_owner
        || facts.schema_owner
        || facts.application_object_owner
        || facts.unexpected_membership
    {
        return Err(Error::Invalid {
            message: "the process database session and current principal must be the same \
                      expected LOGIN role with INHERIT, must own no database and no schema, \
                      relation or routine in the selected database, must be \
                      non-superuser and non-BYPASSRLS, unable to create roles/databases or \
                      replicate, and an inheriting non-admin member only of synveda_app"
                .to_owned(),
        });
    }
    Ok(VerifiedRuntimeRole { name: facts.name })
}
