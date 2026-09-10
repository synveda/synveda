//! Destroying and recreating the application database (CPR-2, ADR-0069).
//!
//! The other half of the epoch guard. [`epoch::verify`](crate::epoch::verify)
//! refuses a database from before the cut; this is the one supported way to
//! get past that refusal, and it is deliberately not an upgrade — it drops the
//! database and builds a fresh one at [`CURRENT_EPOCH`](crate::epoch::CURRENT_EPOCH).
//!
//! ## Why the database rather than the volume
//!
//! A single PostgreSQL server can host separate Synveda and identity-provider
//! databases. Removing its volume would destroy every database in that recovery
//! unit, so this command uses `DROP DATABASE` — the smallest operation that
//! leaves nothing of the old Synveda epoch behind and touches no peer database.
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

use sqlx::{ConnectOptions, Connection, PgConnection};
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
const REQUIRED_EXTENSIONS: &[(&str, &str)] = &[("vector", "0.8.6"), ("btree_gin", "1.3")];

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExistingTarget {
    oid: i64,
    owner_oid: i64,
    owner: String,
    allows_connections: bool,
    is_template: bool,
}

fn reset_target_identity_matches(
    actual: &crate::runtime_role::DatabaseIdentity,
    maintenance: &crate::runtime_role::DatabaseIdentity,
    database: &str,
    target: &ExistingTarget,
) -> bool {
    actual.database == database
        && actual.database_oid == target.oid
        && actual.cluster_system_identifier == maintenance.cluster_system_identifier
        && actual.postmaster_started_at == maintenance.postmaster_started_at
}

async fn verify_reset_target_identity(
    connection: &mut PgConnection,
    maintenance: &crate::runtime_role::DatabaseIdentity,
    database: &str,
    target: &ExistingTarget,
) -> Result<()> {
    let actual = crate::runtime_role::database_identity_connection(connection).await?;
    if !reset_target_identity_matches(&actual, maintenance, database, target) {
        return Err(Error::Invalid {
            message: "the reset target connection did not retain the proved live PostgreSQL database identity"
                .to_owned(),
        });
    }
    Ok(())
}

/// Drops the database the URL names, creates it again, installs the
/// extensions, and migrates it to the current epoch.
///
/// Idempotent: the drop is `if exists` and everything after it builds the same
/// database from nothing, so a second run leaves exactly what the first did.
///
/// This connects to the server's `postgres` maintenance database to do the
/// DDL, because a session cannot drop the database it is connected to.
#[tracing::instrument(name = "store.reset.recreate", skip_all, err(Display))]
pub async fn recreate(
    admin_database_url: &str,
    migrator_database_url: &str,
    database_roles: &crate::runtime_role::DatabaseRoles,
) -> Result<Recreated> {
    let admin_options =
        crate::database_url::parse("SYNVEDA_RESET_ADMIN_DATABASE_URL", admin_database_url)?;
    let migrator_options = crate::database_url::parse("DATABASE_URL", migrator_database_url)?;
    let database = migrator_options
        .get_database()
        .ok_or_else(|| Error::Invalid {
            message: "DATABASE_URL must name the PostgreSQL database to reset".to_owned(),
        })?
        .to_owned();
    if matches!(database.as_str(), "postgres" | "template0" | "template1") {
        return Err(Error::Invalid {
            message: "the PostgreSQL maintenance databases cannot be reset".to_owned(),
        });
    }
    if database_roles
        .forbidden_databases()
        .iter()
        .any(|forbidden| forbidden == &database)
    {
        return Err(Error::Invalid {
            message: "a configured isolated peer database cannot be reset".to_owned(),
        });
    }
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
    if admin_options.get_host() != migrator_options.get_host()
        || admin_options.get_port() != migrator_options.get_port()
    {
        return Err(Error::Invalid {
            message: "reset administrator and migrator URLs must name one PostgreSQL server"
                .to_owned(),
        });
    }
    if migrator_options.get_username() != database_roles.migrator() {
        return Err(Error::Invalid {
            message: "DATABASE_URL login does not match the configured migrator role".to_owned(),
        });
    }

    // One connection, not a pool: this does four statements and then goes
    // away, and a pool against the maintenance database would be four
    // connections holding it open while we try to drop something.
    let maintenance = admin_options.clone().database("postgres");
    let mut admin = maintenance.connect().await.map_err(|err| Error::Storage {
        message: format!(
            "connect to the `postgres` maintenance database on {}:{}: {err}\n\
                 (dropping a database means connecting to a different one; the \
                 reset administrator needs to be able to reach `postgres` and to \
                 CREATEDB)",
            admin_options.get_host(),
            admin_options.get_port(),
        ),
    })?;

    sqlx::query!(
        "select pg_catalog.pg_advisory_lock(pg_catalog.hashtext('synveda.reset.database'))"
    )
    .execute(&mut admin)
    .await
    .map_err(|err| Error::Storage {
        message: format!("acquire the database reset lock: {err}"),
    })?;
    let maintenance_identity =
        crate::runtime_role::database_identity_connection(&mut admin).await?;
    crate::runtime_role::verify_reset_cluster_prerequisites_connection(
        &mut admin,
        &database,
        database_roles,
    )
    .await?;
    let initial_target = existing_target(&mut admin, &database).await?;
    let existed_before = initial_target.is_some();

    if let Some(expected_target) = &initial_target {
        let mut migrator =
            migrator_options
                .clone()
                .connect()
                .await
                .map_err(|err| Error::Storage {
                    message: format!(
                        "connect the configured migrator to the existing reset target: {err}"
                    ),
                })?;
        crate::runtime_role::initialize_product_session_connection(&mut migrator).await?;
        let mut snapshot = sqlx::Connection::begin(&mut migrator)
            .await
            .map_err(|err| Error::Storage {
                message: format!("begin the existing reset-target authority snapshot: {err}"),
            })?;
        crate::runtime_role::configure_authority_snapshot_connection(&mut snapshot).await?;
        let target_identity =
            crate::runtime_role::database_identity_connection(&mut snapshot).await?;
        crate::runtime_role::verify_migrator_prerequisites_connection(
            &mut snapshot,
            database_roles,
        )
        .await?;
        if target_identity.database != database
            || target_identity.database_oid != expected_target.oid
            || target_identity.cluster_system_identifier
                != maintenance_identity.cluster_system_identifier
            || target_identity.postmaster_started_at != maintenance_identity.postmaster_started_at
        {
            return Err(Error::Invalid {
                message:
                    "the reset administrator and migrator did not prove one live PostgreSQL target"
                        .to_owned(),
            });
        }
        snapshot.commit().await.map_err(|err| Error::Storage {
            message: format!("commit the existing reset-target authority snapshot: {err}"),
        })?;
        migrator.close().await.map_err(|err| Error::Storage {
            message: format!("close the existing reset-target proof connection: {err}"),
        })?;
    }

    // The migrator proof used a different connection because DROP DATABASE
    // cannot run while connected to its target. Re-read the shared catalog
    // immediately before the destructive statement and refuse an OID/owner
    // swap rather than destroying the new occupant of a familiar name.
    let final_target = existing_target(&mut admin, &database).await?;
    if final_target != initial_target {
        return Err(Error::Invalid {
            message: "the reset target changed after authority verification; nothing was destroyed"
                .to_owned(),
        });
    }

    // `WITH (FORCE)` terminates the sessions still on it (Postgres 13+).
    // Without it a single leftover connection — a gateway that was not
    // stopped, a `psql` in another terminal — turns this into a refusal that
    // reads as a permissions problem.
    let quoted = quote_identifier(&database);
    let quoted_migrator = quote_identifier(database_roles.migrator());
    if existed_before {
        sqlx::query(&format!("drop database {quoted} with (force)"))
            .execute(&mut admin)
            .await
            .map_err(|err| Error::Storage {
                message: format!("drop the verified database `{database}`: {err}"),
            })?;
    }
    sqlx::query(&format!(
        "create database {quoted} with owner {quoted_migrator} template template0 \
         encoding 'UTF8' allow_connections false"
    ))
    .execute(&mut admin)
    .await
    .map_err(|err| Error::Storage {
        message: format!("create database `{database}`: {err}"),
    })?;
    let fresh_target = existing_target(&mut admin, &database)
        .await?
        .filter(|target| {
            target.owner == database_roles.migrator()
                && !target.allows_connections
                && !target.is_template
        })
        .ok_or_else(|| Error::Invalid {
            message: "the freshly created reset target did not retain its exact owner and isolated connection state"
                .to_owned(),
        })?;
    if let Err(error) = configure_database_acl(
        &mut admin,
        &database,
        &quoted,
        &quoted_migrator,
        database_roles,
    )
    .await
    {
        return Err(remove_failed_target(
            &mut admin,
            &database,
            &quoted,
            &fresh_target,
            false,
            error,
        )
        .await);
    }

    let extensions = match configure_target_schema(
        admin_options.clone().database(&database),
        &quoted_migrator,
        database_roles,
        &maintenance_identity,
        &database,
        &fresh_target,
    )
    .await
    {
        Ok(extensions) => extensions,
        Err(error) => {
            return Err(remove_failed_target(
                &mut admin,
                &database,
                &quoted,
                &fresh_target,
                true,
                error,
            )
            .await);
        }
    };

    let migration = async {
        let mut migrator = migrator_options
            .connect()
            .await
            .map_err(|err| Error::Storage {
                message: format!("connect to the fresh database `{database}`: {err}"),
            })?;
        verify_reset_target_identity(
            &mut migrator,
            &maintenance_identity,
            &database,
            &fresh_target,
        )
        .await?;
        let metadata = crate::migrate_reporting_connection(&mut migrator, database_roles).await?;
        migrator.close().await.map_err(|err| Error::Storage {
            message: format!("close the migrated reset-target connection: {err}"),
        })?;
        Ok(metadata)
    }
    .await;
    let metadata = match migration {
        Ok(metadata) => metadata,
        Err(error) => {
            return Err(remove_failed_target(
                &mut admin,
                &database,
                &quoted,
                &fresh_target,
                true,
                error,
            )
            .await);
        }
    };
    drop(admin);

    Ok(Recreated {
        database,
        existed_before,
        extensions,
        metadata,
    })
}

async fn existing_target(
    connection: &mut PgConnection,
    database: &str,
) -> Result<Option<ExistingTarget>> {
    sqlx::query_as!(
        ExistingTarget,
        r#"select database.oid::bigint as "oid!",
                  owner.oid::bigint as "owner_oid!",
                  owner.rolname as "owner!",
                  database.datallowconn as "allows_connections!",
                  database.datistemplate as "is_template!"
             from pg_catalog.pg_database as database
             join pg_catalog.pg_roles as owner on owner.oid = database.datdba
            where database.datname = $1"#,
        database,
    )
    .fetch_optional(&mut *connection)
    .await
    .map_err(|err| Error::Storage {
        message: format!("read reset-target identity: {err}"),
    })
}

async fn configure_database_acl(
    admin: &mut PgConnection,
    database: &str,
    quoted_database: &str,
    quoted_migrator: &str,
    database_roles: &crate::runtime_role::DatabaseRoles,
) -> Result<()> {
    let quoted_gateway = quote_identifier(database_roles.gateway());
    let quoted_worker = quote_identifier(database_roles.worker());
    let quoted_administrators = database_roles
        .runtime()
        .administrators()
        .iter()
        .map(|role| quote_identifier(role))
        .collect::<Vec<_>>()
        .join(", ");
    let mut transaction =
        sqlx::Connection::begin(&mut *admin)
            .await
            .map_err(|err| Error::Storage {
                message: format!("begin reset database-ACL transaction: {err}"),
            })?;
    let result = async {
        sqlx::query(&format!("set local role {quoted_migrator}"))
            .execute(&mut *transaction)
            .await
            .map_err(|err| Error::Storage {
                message: format!("enter the migration owner for reset database ACLs: {err}"),
            })?;
        for (statement, context) in [
            (
                format!("revoke all on database {quoted_database} from public"),
                "revoke PUBLIC reset-target access",
            ),
            (
                format!(
                    "grant create, connect, temporary on database {quoted_database} to {quoted_migrator}"
                ),
                "record the migration owner's reset-target ACL",
            ),
            (
                format!(
                    "grant connect on database {quoted_database} to {quoted_gateway}, {quoted_worker}"
                ),
                "grant runtime reset-target access",
            ),
            (
                format!(
                    "grant connect on database {quoted_database} to {quoted_administrators}"
                ),
                "grant trusted-administrator reset-target access",
            ),
        ] {
            sqlx::query(&statement)
                .execute(&mut *transaction)
                .await
                .map_err(|err| Error::Storage {
                    message: format!("{context}: {err}"),
                })?;
        }
        verify_database_acl(&mut transaction, database, database_roles).await?;
        sqlx::query("set local role none")
            .execute(&mut *transaction)
            .await
            .map_err(|err| Error::Storage {
                message: format!("leave the migration owner after reset database ACLs: {err}"),
            })?;
        sqlx::query(&format!(
            "alter database {quoted_database} with allow_connections true"
        ))
        .execute(&mut *transaction)
        .await
        .map_err(|err| Error::Storage {
            message: format!("enable the converged reset target: {err}"),
        })?;
        Ok(())
    }
    .await;
    match result {
        Ok(()) => transaction.commit().await.map_err(|err| Error::Storage {
            message: format!("commit reset database ACLs: {err}"),
        }),
        Err(error) => {
            transaction
                .rollback()
                .await
                .map_err(|rollback| Error::Storage {
                    message: format!(
                        "{error}; rolling back reset database ACLs failed: {rollback}"
                    ),
                })?;
            Err(error)
        }
    }
}

async fn verify_database_acl(
    connection: &mut PgConnection,
    database: &str,
    database_roles: &crate::runtime_role::DatabaseRoles,
) -> Result<()> {
    let safe = sqlx::query_scalar!(
        r#"select exists (
              select 1
                from pg_catalog.pg_database as database
                join pg_catalog.pg_roles as owner on owner.oid = database.datdba
               where database.datname = $1
                 and owner.rolname = $2
                 and not database.datallowconn
                 and not database.datistemplate
                 and not exists (
                   select 1
                     from pg_catalog.aclexplode(database.datacl) as acl
                    where not (
                      acl.grantee = owner.oid
                      and acl.grantor = owner.oid
                      and acl.privilege_type in ('CREATE', 'CONNECT', 'TEMPORARY')
                      and not acl.is_grantable
                      or acl.grantee in (
                        select role.oid from pg_catalog.pg_roles as role
                         where role.rolname in ($3, $4)
                      )
                      and acl.grantor = owner.oid
                      and acl.privilege_type = 'CONNECT'
                      and not acl.is_grantable
                      or acl.grantee in (
                        select role.oid from pg_catalog.pg_roles as role
                         where role.rolname = any($5::text[])
                      )
                      and acl.grantor = owner.oid
                      and acl.privilege_type = 'CONNECT'
                      and not acl.is_grantable
                    )
                 )
                 and (
                   select count(*)
                     from pg_catalog.aclexplode(database.datacl) as acl
                    where acl.grantee = owner.oid
                      and acl.grantor = owner.oid
                      and acl.privilege_type in ('CREATE', 'CONNECT', 'TEMPORARY')
                      and not acl.is_grantable
                 ) = 3
                 and (
                   select count(*)
                     from pg_catalog.aclexplode(database.datacl) as acl
                    where acl.grantee in (
                            select role.oid from pg_catalog.pg_roles as role
                             where role.rolname in ($3, $4)
                          )
                      and acl.grantor = owner.oid
                      and acl.privilege_type = 'CONNECT'
                      and not acl.is_grantable
                 ) = 2
                 and (
                   select count(*)
                     from pg_catalog.aclexplode(database.datacl) as acl
                    where acl.grantee in (
                            select role.oid from pg_catalog.pg_roles as role
                             where role.rolname = any($5::text[])
                          )
                      and acl.grantor = owner.oid
                      and acl.privilege_type = 'CONNECT'
                      and not acl.is_grantable
                 ) = pg_catalog.cardinality($5::text[])
            ) as "safe!""#,
        database,
        database_roles.migrator(),
        database_roles.gateway(),
        database_roles.worker(),
        database_roles.runtime().administrators(),
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|err| Error::Storage {
        message: format!("verify freshly created reset-target database ACLs: {err}"),
    })?;
    if !safe {
        return Err(Error::Invalid {
            message: "the freshly created reset-target database ACLs do not have the exact migration-owner grantor and configured grantee set"
                .to_owned(),
        });
    }
    Ok(())
}

async fn configure_target_schema(
    options: sqlx::postgres::PgConnectOptions,
    quoted_migrator: &str,
    database_roles: &crate::runtime_role::DatabaseRoles,
    maintenance_identity: &crate::runtime_role::DatabaseIdentity,
    database: &str,
    fresh_target: &ExistingTarget,
) -> Result<Vec<ExtensionOutcome>> {
    let mut connection = options.connect().await.map_err(|err| Error::Storage {
        message: format!("connect the reset administrator to the fresh target: {err}"),
    })?;
    verify_reset_target_identity(
        &mut connection,
        maintenance_identity,
        database,
        fresh_target,
    )
    .await?;
    let mut transaction = sqlx::Connection::begin(&mut connection)
        .await
        .map_err(|err| Error::Storage {
            message: format!("begin reset target-schema transaction: {err}"),
        })?;
    let result = async {
        sqlx::query(&format!("alter schema public owner to {quoted_migrator}"))
            .execute(&mut *transaction)
            .await
            .map_err(|err| Error::Storage {
                message: format!("converge the reset-target public schema owner: {err}"),
            })?;
        sqlx::query("revoke all on schema public from public")
            .execute(&mut *transaction)
            .await
            .map_err(|err| Error::Storage {
                message: format!("revoke PUBLIC reset-target schema access: {err}"),
            })?;
        let extensions = install_extensions(&mut transaction).await?;
        verify_target_schema(&mut transaction, database_roles).await?;
        Ok(extensions)
    }
    .await;
    let extensions =
        match result {
            Ok(extensions) => {
                transaction.commit().await.map_err(|err| Error::Storage {
                    message: format!("commit reset target-schema prerequisites: {err}"),
                })?;
                extensions
            }
            Err(error) => {
                transaction.rollback().await.map_err(|rollback| Error::Storage {
                message: format!(
                    "{error}; rolling back reset target-schema prerequisites failed: {rollback}"
                ),
            })?;
                return Err(error);
            }
        };
    connection.close().await.map_err(|err| Error::Storage {
        message: format!("close the reset target-schema connection: {err}"),
    })?;
    Ok(extensions)
}

async fn verify_target_schema(
    connection: &mut PgConnection,
    database_roles: &crate::runtime_role::DatabaseRoles,
) -> Result<()> {
    let safe = sqlx::query_scalar!(
        r#"select current_user = session_user
              and current_user = any($2::text[])
              and exists (
                select 1
                  from pg_catalog.pg_namespace as namespace
                  join pg_catalog.pg_roles as owner on owner.oid = namespace.nspowner
                 where namespace.nspname = 'public'
                   and owner.rolname = $1
                   and not exists (
                     select 1
                       from pg_catalog.aclexplode(namespace.nspacl) as acl
                      where not (
                        acl.grantee = owner.oid
                        and acl.grantor = owner.oid
                        and acl.privilege_type in ('CREATE', 'USAGE')
                        and not acl.is_grantable
                      )
                   )
                   and (
                     select count(*)
                       from pg_catalog.aclexplode(namespace.nspacl) as acl
                      where acl.grantee = owner.oid
                        and acl.grantor = owner.oid
                        and acl.privilege_type in ('CREATE', 'USAGE')
                        and not acl.is_grantable
                   ) = 2
              )
              and (
                select count(*)
                  from pg_catalog.pg_extension as extension
                  join pg_catalog.pg_namespace as namespace
                    on namespace.oid = extension.extnamespace
                 where extension.extname in ('btree_gin', 'vector')
                   and namespace.nspname = 'public'
                   and extension.extowner = (
                     select role.oid from pg_catalog.pg_roles as role
                      where role.rolname = current_user
                   )
                   and (
                     extension.extname = 'btree_gin' and extension.extversion = '1.3'
                     or extension.extname = 'vector' and extension.extversion = '0.8.6'
                   )
              ) = 2
              and not exists (
                select 1
                  from pg_catalog.pg_extension as extension
                 where extension.extname not in ('plpgsql', 'btree_gin', 'vector')
              ) as "safe!""#,
        database_roles.migrator(),
        database_roles.runtime().administrators(),
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|err| Error::Storage {
        message: format!("verify reset target-schema prerequisites: {err}"),
    })?;
    if !safe {
        return Err(Error::Invalid {
            message: "the reset target public schema and exact extension prerequisites did not converge under the configured owners"
                .to_owned(),
        });
    }
    Ok(())
}

async fn remove_failed_target(
    admin: &mut PgConnection,
    database: &str,
    quoted_database: &str,
    fresh_target: &ExistingTarget,
    expected_connections: bool,
    cause: Error,
) -> Error {
    let current_target = match existing_target(admin, database).await {
        Ok(target) => target,
        Err(cleanup) => {
            return Error::Storage {
                message: format!(
                    "{cause}; proving the incomplete fresh reset target before cleanup failed: {cleanup}"
                ),
            };
        }
    };
    if current_target.as_ref().is_none_or(|target| {
        target.oid != fresh_target.oid
            || target.owner_oid != fresh_target.owner_oid
            || target.owner != fresh_target.owner
            || target.allows_connections != expected_connections
            || target.is_template
    }) {
        return Error::Storage {
            message: format!(
                "{cause}; the incomplete fresh reset target changed identity or isolation state and was left untouched"
            ),
        };
    }
    match sqlx::query(&format!("drop database {quoted_database} with (force)"))
        .execute(&mut *admin)
        .await
    {
        Ok(_) => Error::Storage {
            message: format!("{cause}; the incomplete fresh reset target was removed"),
        },
        Err(cleanup) => Error::Storage {
            message: format!(
                "{cause}; removing the incomplete fresh reset target also failed: {cleanup}"
            ),
        },
    }
}

/// Installs deployment-owned extensions before the application migration.
/// The canonical Compose bootstrap and `scripts/db-test.sh` perform the same
/// prerequisite explicitly; the epoch baseline owns no cluster role or
/// extension DDL.
async fn install_extensions(connection: &mut PgConnection) -> Result<Vec<ExtensionOutcome>> {
    let mut outcomes = Vec::new();
    for &(name, version) in REQUIRED_EXTENSIONS {
        // Constants, never values: both fields come from the closed array
        // above and reach no other caller.
        let statement = format!("create extension if not exists {name} version '{version}'");
        match sqlx::query(&statement).execute(&mut *connection).await {
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

/// Quotes one PostgreSQL identifier without interpreting any part of it.
fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
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
        assert_eq!(
            quote_identifier("role\"; drop database synveda; --"),
            "\"role\"\"; drop database synveda; --\""
        );
    }

    /// The required set is what the migrations actually call, and the
    /// optional one is what the product does not. Pinned here because the
    /// difference is the difference between "your Postgres is missing
    /// something" and "you cannot run Synveda on this Postgres".
    #[test]
    fn the_required_extensions_are_the_ones_a_migration_needs() {
        assert_eq!(
            REQUIRED_EXTENSIONS,
            &[("vector", "0.8.6"), ("btree_gin", "1.3")]
        );
    }

    #[test]
    fn reset_target_identity_rejects_each_routing_dimension_independently() {
        let started_at = chrono::DateTime::parse_from_rfc3339("2026-08-29T00:00:00Z")
            .expect("parse fixed test timestamp")
            .with_timezone(&chrono::Utc);
        let maintenance = crate::runtime_role::DatabaseIdentity {
            database: "postgres".to_owned(),
            cluster_system_identifier: "cluster-a".to_owned(),
            database_oid: 5,
            postmaster_started_at: started_at,
        };
        let target = ExistingTarget {
            oid: 16_384,
            owner_oid: 16_383,
            owner: "synveda_migrator".to_owned(),
            allows_connections: false,
            is_template: false,
        };
        let accepted = crate::runtime_role::DatabaseIdentity {
            database: "synveda".to_owned(),
            cluster_system_identifier: maintenance.cluster_system_identifier.clone(),
            database_oid: target.oid,
            postmaster_started_at: maintenance.postmaster_started_at,
        };
        assert!(reset_target_identity_matches(
            &accepted,
            &maintenance,
            "synveda",
            &target,
        ));

        let mut mismatches = Vec::new();
        let mut mismatch = accepted.clone();
        mismatch.database = "other".to_owned();
        mismatches.push(mismatch);
        let mut mismatch = accepted.clone();
        mismatch.database_oid += 1;
        mismatches.push(mismatch);
        let mut mismatch = accepted.clone();
        mismatch.cluster_system_identifier = "cluster-b".to_owned();
        mismatches.push(mismatch);
        let mut mismatch = accepted;
        mismatch.postmaster_started_at += chrono::Duration::seconds(1);
        mismatches.push(mismatch);
        for mismatch in mismatches {
            assert!(!reset_target_identity_matches(
                &mismatch,
                &maintenance,
                "synveda",
                &target,
            ));
        }
    }

    #[tokio::test]
    async fn invalid_reset_urls_never_disclose_their_credentials() {
        const SENTINEL: &str = "SYNVEDA_STORE_RESET_SECRET";
        let database_urls = [
            format!("postgres://admin:{SENTINEL}@localhost"),
            format!("https://admin:{SENTINEL}@localhost/synveda"),
            format!("postgres://admin@localhost/synveda?access_token={SENTINEL}"),
        ];
        let roles = crate::runtime_role::DatabaseRoles::new(
            "migrator".to_owned(),
            "gateway".to_owned(),
            "worker".to_owned(),
            vec!["administrator".to_owned()],
            Vec::new(),
            vec!["postgres".to_owned()],
            Vec::new(),
        )
        .expect("test roles");
        for database_url in &database_urls {
            let error = recreate(database_url, database_url, &roles)
                .await
                .expect_err("unsafe reset URL must be refused before connecting");
            assert!(!error.to_string().contains(SENTINEL), "{error}");
            assert!(!error.to_string().contains(database_url), "{error}");
        }
    }

    #[tokio::test]
    async fn a_configured_peer_database_is_refused_before_connecting() {
        let roles = crate::runtime_role::DatabaseRoles::new(
            "migrator".to_owned(),
            "gateway".to_owned(),
            "worker".to_owned(),
            vec!["administrator".to_owned()],
            Vec::new(),
            vec!["identity".to_owned()],
            Vec::new(),
        )
        .expect("test roles");
        let error = recreate(
            "postgres://administrator:unreachable@127.0.0.1:1/postgres",
            "postgres://migrator:unreachable@127.0.0.1:1/identity",
            &roles,
        )
        .await
        .expect_err("the forbidden target is a pure pre-connection refusal");
        assert!(error.to_string().contains("isolated peer"), "{error}");
    }
}
