//! The usage projection and the sweeper's watermark (FLOW-4, ADR-0033).
//!
//! Neither table holds a governed fact. `memory_usage` is a summary of
//! `context.injected` events that are already under the audit chain's
//! hash, and `promotion_watermarks` records only how far that fold has
//! got. Truncating both and replaying from seq 1 must reproduce the
//! projection exactly (ADR-0033 decision 3) — [`reset`] is the operation
//! that makes the property testable, and the DELETE grant in migration
//! 0020 exists for it.

use chrono::{DateTime, Utc};
use sqlx::postgres::{PgConnection, PgExecutor};
use synveda_types::{Error, RecordId, Result, TenantId, UsageFacts};

/// One (record, member) pair's contribution from a single fold.
///
/// The caller pre-aggregates: a batch may not carry the same
/// `(record_id, subject)` twice, because `ON CONFLICT` cannot affect one
/// row twice in a single statement. That is not merely an optimisation —
/// a duplicate inside one batch is a runtime error from Postgres.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageDelta {
    /// The recalled record.
    pub record_id: RecordId,
    /// The member who recalled it, as the audit chain names them.
    pub subject: String,
    /// How many times, inside this batch.
    pub recalls: i64,
    /// Earliest recall in this batch.
    pub first_recall_at: DateTime<Utc>,
    /// Latest recall in this batch.
    pub last_recall_at: DateTime<Utc>,
}

/// What the projection says about one record, aggregated over its
/// members.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageRow {
    /// The record.
    pub record_id: RecordId,
    /// Total recalls across every member.
    pub recalls: i64,
    /// How many distinct members recalled it.
    pub distinct_members: i64,
    /// When it was first recalled by anyone.
    pub first_recall_at: DateTime<Utc>,
    /// When it was last recalled by anyone.
    pub last_recall_at: DateTime<Utc>,
}

impl UsageRow {
    /// The threshold view a rule tests against.
    #[must_use]
    pub fn facts(&self) -> UsageFacts {
        UsageFacts {
            recalls: self.recalls.max(0).unsigned_abs(),
            distinct_members: self.distinct_members.max(0).unsigned_abs(),
        }
    }
}

fn storage_error(context: &str, err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err
        && db.code().as_deref() == Some("42501")
    {
        return crate::rls::backstop_error(db);
    }
    Error::Storage {
        message: format!("{context}: {err}"),
    }
}

/// Folds one batch of recalls into the projection.
///
/// Set-based by construction: one statement per batch regardless of how
/// many pairs it carries, because the fold runs behind a sweep whose
/// whole job is to keep up with the chain (ADR-0033 decision 14).
#[tracing::instrument(
    name = "store.promotion.fold",
    skip_all,
    fields(tenant.id = %tenant_id, deltas = deltas.len()),
    err(Display)
)]
pub async fn fold(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    deltas: &[UsageDelta],
) -> Result<()> {
    if deltas.is_empty() {
        return Ok(());
    }
    let record_ids: Vec<uuid::Uuid> = deltas.iter().map(|d| d.record_id.as_uuid()).collect();
    let subjects: Vec<String> = deltas.iter().map(|d| d.subject.clone()).collect();
    let recalls: Vec<i64> = deltas.iter().map(|d| d.recalls).collect();
    let first: Vec<DateTime<Utc>> = deltas.iter().map(|d| d.first_recall_at).collect();
    let last: Vec<DateTime<Utc>> = deltas.iter().map(|d| d.last_recall_at).collect();

    sqlx::query!(
        r#"
        insert into memory_usage
            (tenant_id, record_id, subject, recalls, first_recall_at, last_recall_at)
        select $1, u.record_id, u.subject, u.recalls, u.first_recall_at,
               u.last_recall_at
        from unnest($2::uuid[], $3::text[], $4::bigint[], $5::timestamptz[],
                    $6::timestamptz[])
                as u(record_id, subject, recalls, first_recall_at, last_recall_at)
        on conflict (tenant_id, record_id, subject) do update
            set recalls = memory_usage.recalls + excluded.recalls,
                first_recall_at = least(memory_usage.first_recall_at,
                                        excluded.first_recall_at),
                last_recall_at = greatest(memory_usage.last_recall_at,
                                          excluded.last_recall_at)
        "#,
        tenant_id.as_uuid(),
        &record_ids,
        &subjects,
        &recalls,
        &first,
        &last,
    )
    .execute(&mut *conn)
    .await
    .map_err(|err| storage_error("fold usage", err))?;
    Ok(())
}

/// How far the sweeper has folded this tenant's chain. Zero when it has
/// never run — `audit_log.seq` is 1-based, so zero cannot collide with a
/// real event.
#[tracing::instrument(
    name = "store.promotion.watermark",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn watermark(executor: impl PgExecutor<'_>, tenant_id: TenantId) -> Result<i64> {
    let seq = sqlx::query_scalar!(
        "select last_seq from promotion_watermarks where tenant_id = $1",
        tenant_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(|err| storage_error("read promotion watermark", err))?;
    Ok(seq.unwrap_or(0))
}

/// [`watermark`], locked for the caller's transaction: the per-tenant
/// sweep lock.
///
/// A fold is read-then-add, so two sweeps that both read the same
/// watermark both fold the same events and the `+ excluded.recalls`
/// upsert counts each recall twice — evidence inflated by however many
/// sweepers happened to overlap. Locking the watermark row makes the
/// second sweeper wait and then read the advanced value, so the events
/// it would have double-counted are simply not in its range.
///
/// The row is created first so there is something to lock on a tenant
/// that has never been swept — the AUD-1 chain-head pattern (ADR-0019
/// decision 1), and idempotent under concurrency for the same reason.
#[tracing::instrument(
    name = "store.promotion.watermark_for_update",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn watermark_for_update(conn: &mut PgConnection, tenant_id: TenantId) -> Result<i64> {
    sqlx::query!(
        "insert into promotion_watermarks (tenant_id, last_seq) values ($1, 0)
         on conflict (tenant_id) do nothing",
        tenant_id.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(|err| storage_error("ensure promotion watermark", err))?;
    let seq = sqlx::query_scalar!(
        "select last_seq from promotion_watermarks where tenant_id = $1 for update",
        tenant_id.as_uuid(),
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(|err| storage_error("lock promotion watermark", err))?;
    Ok(seq)
}

/// Records how far the fold has got. Runs in the same transaction as the
/// [`fold`] it describes — a watermark that advanced without its rows,
/// or rows without their watermark, would be double-counting or lost
/// counting, and the transaction is what makes neither possible.
#[tracing::instrument(
    name = "store.promotion.advance",
    skip_all,
    fields(tenant.id = %tenant_id, seq = last_seq),
    err(Display)
)]
pub async fn advance(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    last_seq: i64,
) -> Result<()> {
    sqlx::query!(
        r#"
        insert into promotion_watermarks (tenant_id, last_seq)
        values ($1, $2)
        on conflict (tenant_id) do update
            set last_seq = greatest(promotion_watermarks.last_seq, excluded.last_seq),
                updated_at = now()
        "#,
        tenant_id.as_uuid(),
        last_seq,
    )
    .execute(executor)
    .await
    .map_err(|err| storage_error("advance promotion watermark", err))?;
    Ok(())
}

/// What the projection says about these records, aggregated over their
/// members. Records with no usage are absent rather than zero — the
/// caller is asking about candidates it just folded, and a record nobody
/// recalled is not one.
#[tracing::instrument(
    name = "store.promotion.usage_for",
    skip_all,
    fields(tenant.id = %tenant_id, records = record_ids.len()),
    err(Display)
)]
pub async fn usage_for(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    record_ids: &[RecordId],
) -> Result<Vec<UsageRow>> {
    if record_ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<uuid::Uuid> = record_ids.iter().map(RecordId::as_uuid).collect();
    let rows = sqlx::query!(
        r#"
        select record_id as "record_id!",
               -- `sum(bigint)` is numeric in Postgres; the projection's
               -- counts are bigints and stay bigints.
               sum(recalls)::bigint as "recalls!",
               count(*) as "distinct_members!",
               min(first_recall_at) as "first_recall_at!",
               max(last_recall_at) as "last_recall_at!"
        from memory_usage
        where tenant_id = $1 and record_id = any($2)
        group by record_id
        "#,
        tenant_id.as_uuid(),
        &ids,
    )
    .fetch_all(executor)
    .await
    .map_err(|err| storage_error("read usage", err))?;
    Ok(rows
        .into_iter()
        .map(|row| UsageRow {
            record_id: RecordId::from_uuid(row.record_id),
            recalls: row.recalls,
            distinct_members: row.distinct_members,
            first_recall_at: row.first_recall_at,
            last_recall_at: row.last_recall_at,
        })
        .collect())
}

/// Discards the projection and the watermark for one tenant, so the next
/// sweep refolds the chain from seq 1.
///
/// The rebuild in ADR-0033 decision 3, as an operation rather than an
/// aspiration: this is derived state, and the test that proves it is
/// derived calls exactly this and then asserts the projection comes back
/// identical.
#[tracing::instrument(
    name = "store.promotion.reset",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn reset(conn: &mut PgConnection, tenant_id: TenantId) -> Result<()> {
    sqlx::query!(
        "delete from memory_usage where tenant_id = $1",
        tenant_id.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(|err| storage_error("clear usage projection", err))?;
    sqlx::query!(
        "delete from promotion_watermarks where tenant_id = $1",
        tenant_id.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(|err| storage_error("clear promotion watermark", err))?;
    Ok(())
}
