//! Exact tenant rows for the portable archive and their restore (ADR-0157).
//!
//! Export reads the tenant's portable tables as PostgreSQL COPY text inside one
//! administrator snapshot with pinned output settings, ordered by primary key,
//! so the same rows always produce the same bytes. Restore loads those bytes in restore
//! order with user triggers suspended inside the single restore transaction,
//! keeps foreign keys enforced, and recomputes every table's bytes before
//! commit. Table and column identifiers come only from the reviewed inventory
//! and the live catalogue, never from an archive.

use futures_util::StreamExt as _;
use sqlx::PgConnection;
use synveda_types::{Error, Result, Tenant, TenantId};

use crate::tenant_inventory::{Archive, TENANT_TABLES, TenantTable, is_plain_identifier};

/// The settings that make COPY text deterministic across sessions and
/// supported server majors.
const PINNED_COPY_SETTINGS: [&str; 5] = [
    "set local datestyle = 'ISO, YMD'",
    "set local intervalstyle = 'postgres'",
    "set local timezone = 'UTC'",
    "set local extra_float_digits = 3",
    "set local bytea_output = 'hex'",
];

/// One portable table as this database defines it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableShape {
    /// The inventory entry.
    pub table: &'static TenantTable,
    /// Stored (non-generated) columns in catalogue order.
    pub columns: Vec<String>,
    /// Primary-key columns in key order.
    pub key: Vec<String>,
}

/// Every table the archive carries, in restore order.
pub fn portable_tables() -> impl Iterator<Item = &'static TenantTable> {
    TENANT_TABLES
        .iter()
        .filter(|table| table.archive == Archive::Portable)
}

/// Begins one `REPEATABLE READ` archive transaction as a configured database
/// administrator that bypasses row security, scoped to `tenant` by GUC and
/// by every statement's explicit tenant filter.
///
/// A complete archive includes rows the source-eligibility policies hide
/// from every product role, such as withdrawn Session events retained as
/// suppression evidence, so no forced-RLS principal can read or verify it.
pub async fn begin_administrator_tx(
    pool: &sqlx::PgPool,
    roles: &crate::runtime_role::DatabaseRoles,
    tenant: TenantId,
    read_only: bool,
) -> Result<sqlx::Transaction<'static, sqlx::Postgres>> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| storage("begin archive transaction", &error))?;
    let isolation = if read_only {
        "set transaction isolation level repeatable read, read only"
    } else {
        "set transaction isolation level repeatable read"
    };
    sqlx::query(isolation)
        .execute(&mut *tx)
        .await
        .map_err(|error| storage("set archive isolation", &error))?;
    let (current, session, login, bypasses): (String, String, bool, bool) = sqlx::query_as(
        "select current_user::text, session_user::text, role.rolcanlogin, \
                not pg_catalog.row_security_active('public.scopes'::regclass) \
           from pg_catalog.pg_roles as role where role.rolname = current_user",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|error| storage("read archive principal", &error))?;
    let configured = roles.runtime().administrators().contains(&current);
    if current != session || !login || !bypasses || !configured {
        return Err(Error::Invalid {
            message: "the tenant archive requires a configured database administrator login that bypasses row security"
                .to_owned(),
        });
    }
    crate::epoch::verify_connection(&mut tx)
        .await
        .map_err(|error| Error::Invalid {
            message: format!("the archive database is not at the required schema epoch: {error}"),
        })?;
    sqlx::query!(
        r#"select set_config('synveda.tenant_id', $1, true) as "tenant!", set_config('lock_timeout', '5s', true) as "lock_timeout!""#,
        tenant.as_uuid().to_string(),
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|error| storage("set archive tenant", &error))?;
    Ok(tx)
}

/// The database time at which this transaction's snapshot began.
pub async fn snapshot_time(connection: &mut PgConnection) -> Result<chrono::DateTime<chrono::Utc>> {
    sqlx::query_scalar!(r#"select now() as "now!""#)
        .fetch_one(&mut *connection)
        .await
        .map_err(|error| storage("read snapshot time", &error))
}

/// Pins COPY text formatting for the rest of the transaction.
pub async fn pin_copy_format(connection: &mut PgConnection) -> Result<()> {
    for statement in PINNED_COPY_SETTINGS {
        sqlx::query(statement)
            .execute(&mut *connection)
            .await
            .map_err(|error| storage("pin COPY format", &error))?;
    }
    Ok(())
}

/// Bounds each statement of a long lifecycle transaction.
pub async fn bound_statements(connection: &mut PgConnection, seconds: u64) -> Result<()> {
    sqlx::query("select set_config('statement_timeout', $1, true)")
        .bind(format!("{}s", seconds.max(1)))
        .execute(&mut *connection)
        .await
        .map(|_| ())
        .map_err(|error| storage("bound lifecycle statements", &error))
}

/// Reads one portable table's stored columns and primary key.
pub async fn table_shape(
    connection: &mut PgConnection,
    table: &'static TenantTable,
) -> Result<TableShape> {
    let qualified = format!("public.{}", table.name);
    let columns: Vec<String> = sqlx::query_scalar(
        "select attname::text from pg_catalog.pg_attribute \
          where attrelid = $1::regclass and attnum > 0 and not attisdropped \
            and attgenerated = '' order by attnum",
    )
    .bind(&qualified)
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| storage("read archive columns", &error))?;
    let key: Vec<String> = sqlx::query_scalar(
        "select a.attname::text from pg_catalog.pg_index i \
           join lateral unnest(i.indkey) with ordinality as k(attnum, position) on true \
           join pg_catalog.pg_attribute a on a.attrelid = i.indrelid and a.attnum = k.attnum \
          where i.indrelid = $1::regclass and i.indisprimary order by k.position",
    )
    .bind(&qualified)
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| storage("read archive key", &error))?;
    if columns.is_empty()
        || key.is_empty()
        || !columns.iter().chain(&key).all(|c| is_plain_identifier(c))
    {
        return Err(Error::Invalid {
            message: format!(
                "{} has no archivable column list or primary key",
                table.name
            ),
        });
    }
    Ok(TableShape {
        table,
        columns,
        key,
    })
}

fn column_list(names: &[String]) -> String {
    names
        .iter()
        .map(|name| format!("\"{name}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

fn copy_out_statement(shape: &TableShape, tenant: TenantId) -> String {
    debug_assert!(is_plain_identifier(shape.table.name));
    format!(
        "copy (select {} from public.\"{}\" where tenant_id = '{}' order by {}) to stdout",
        column_list(&shape.columns),
        shape.table.name,
        tenant.as_uuid(),
        column_list(&shape.key),
    )
}

/// Streams one table's rows for `tenant`, one complete COPY text row per
/// call. Refuses a row longer than `max_row_bytes` without buffering it.
/// Returns the row count.
pub async fn copy_out_rows(
    connection: &mut PgConnection,
    shape: &TableShape,
    tenant: TenantId,
    max_row_bytes: usize,
    mut on_row: impl FnMut(&[u8]) -> Result<()>,
) -> Result<u64> {
    let mut stream = connection
        .copy_out_raw(&copy_out_statement(shape, tenant))
        .await
        .map_err(|error| storage("begin archive COPY", &error))?;
    let mut row = Vec::new();
    let mut rows = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| storage("read archive COPY", &error))?;
        for piece in chunk.split_inclusive(|byte| *byte == b'\n') {
            if row.len() + piece.len() > max_row_bytes {
                return Err(Error::Invalid {
                    message: format!(
                        "a row of {} exceeds the {max_row_bytes}-byte archive frame bound",
                        shape.table.name
                    ),
                });
            }
            row.extend_from_slice(piece);
            if row.ends_with(b"\n") {
                on_row(&row)?;
                rows += 1;
                row.clear();
            }
        }
    }
    if !row.is_empty() {
        return Err(Error::Invalid {
            message: format!("COPY output for {} ended inside a row", shape.table.name),
        });
    }
    Ok(rows)
}

/// The ordered COPY digest and row count of one table for `tenant`.
pub async fn table_digest(
    connection: &mut PgConnection,
    shape: &TableShape,
    tenant: TenantId,
    max_row_bytes: usize,
) -> Result<(u64, [u8; 32])> {
    let mut hasher = blake3::Hasher::new();
    let rows = copy_out_rows(connection, shape, tenant, max_row_bytes, |row| {
        hasher.update(row);
        Ok(())
    })
    .await?;
    Ok((rows, *hasher.finalize().as_bytes()))
}

/// Loads COPY text rows into one table. `segments` are complete rows.
pub async fn copy_in_rows<'a>(
    connection: &mut PgConnection,
    shape: &TableShape,
    segments: impl IntoIterator<Item = &'a [u8]>,
) -> Result<u64> {
    let statement = format!(
        "copy public.\"{}\" ({}) from stdin",
        shape.table.name,
        column_list(&shape.columns)
    );
    let mut copy = connection
        .copy_in_raw(&statement)
        .await
        .map_err(|error| storage("begin restore COPY", &error))?;
    for segment in segments {
        copy.send(segment)
            .await
            .map_err(|error| storage("send restore COPY", &error))?;
    }
    copy.finish()
        .await
        .map_err(|error| storage(&format!("restore {}", shape.table.name), &error))
}

/// Suspends or restores user triggers on every portable table for the rest
/// of the restore transaction. Other sessions never see the suspension: it
/// is reversed before commit and the restore runs with writers drained.
pub async fn set_user_triggers(connection: &mut PgConnection, enabled: bool) -> Result<()> {
    let verb = if enabled { "enable" } else { "disable" };
    for table in portable_tables() {
        debug_assert!(is_plain_identifier(table.name));
        sqlx::query(&format!(
            "alter table public.\"{}\" {verb} trigger user",
            table.name
        ))
        .execute(&mut *connection)
        .await
        .map_err(|error| storage(&format!("{verb} restore triggers"), &error))?;
    }
    Ok(())
}

/// Defers every deferrable constraint to commit, or checks them now.
pub async fn set_constraints_deferred(connection: &mut PgConnection, deferred: bool) -> Result<()> {
    let statement = if deferred {
        "set constraints all deferred"
    } else {
        "set constraints all immediate"
    };
    sqlx::query(statement)
        .execute(&mut *connection)
        .await
        .map(|_| ())
        .map_err(|error| storage("check restored constraints", &error))
}

/// Recreates the tenant row exactly as archived, suspended.
pub async fn insert_restored_tenant(connection: &mut PgConnection, tenant: &Tenant) -> Result<()> {
    sqlx::query!(
        "insert into tenants (id, slug, name, status, created_at) values ($1, $2, $3, 'suspended', $4)",
        tenant.id.as_uuid(),
        tenant.slug,
        tenant.name,
        tenant.created_at,
    )
    .execute(&mut *connection)
    .await
    .map(|_| ())
    .map_err(|error| match &error {
        sqlx::Error::Database(db) if db.code().as_deref() == Some("23505") => Error::Conflict {
            message: "tenant_collision".to_owned(),
        },
        _ => storage("restore tenant row", &error),
    })
}

/// Whether a tenant with this slug already exists. Used only to refuse a
/// colliding restore before any write.
pub async fn slug_taken(connection: &mut PgConnection, slug: &str) -> Result<bool> {
    sqlx::query_scalar!(
        r#"select exists(select 1 from tenants where slug = $1) as "taken!""#,
        slug
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| storage("check restore slug", &error))
}

/// Waits up to `bound` until no gateway or worker session is connected, the
/// same drain an installed-schema migration requires. A restore suspends
/// triggers inside its transaction, so it must not contend with writers.
pub async fn drain_runtime_writers(
    pool: &sqlx::PgPool,
    roles: &crate::runtime_role::DatabaseRoles,
    bound: std::time::Duration,
) -> Result<()> {
    let mut connection = pool
        .acquire()
        .await
        .map_err(|error| storage("acquire writer-drain connection", &error))?;
    crate::migration::drain_runtime_writers(&mut connection, roles, bound)
        .await
        .map(|_| ())
}

fn storage(context: &str, error: &sqlx::Error) -> Error {
    Error::Storage {
        message: format!("{context}: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_archive_carries_every_table_but_derived_and_excluded_ones() {
        let carried: Vec<_> = portable_tables().map(|table| table.name).collect();
        assert_eq!(carried.len(), TENANT_TABLES.len() - 2);
        assert!(!carried.contains(&"knowledge_revision_embeddings"));
        assert!(!carried.contains(&"console_session_index"));
    }

    #[test]
    fn copy_out_names_only_quoted_catalogue_identifiers_and_the_tenant() {
        let table = crate::tenant_inventory::tenant_table("scopes").unwrap();
        let shape = TableShape {
            table,
            columns: vec!["tenant_id".to_owned(), "id".to_owned()],
            key: vec!["tenant_id".to_owned(), "id".to_owned()],
        };
        let tenant = TenantId::new();
        assert_eq!(
            copy_out_statement(&shape, tenant),
            format!(
                "copy (select \"tenant_id\", \"id\" from public.\"scopes\" where tenant_id = '{}' order by \"tenant_id\", \"id\") to stdout",
                tenant.as_uuid()
            )
        );
    }
}
