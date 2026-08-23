//! Retention selection, expiry, destruction and staging disposal
//! (MEM-6, ADR-0040).
//!
//! Every query here takes its cutoffs as arguments. Nothing in this module
//! knows what a horizon is, reads a pack, or asks the clock: the caller
//! resolves the effective pack at the scope in question and hands over the
//! instants it produced (ADR-0040 decisions 1 and 10). That is what keeps
//! "a retention policy change re-evaluates existing records" true — there
//! is no stored state to disagree with a pack that has since changed.
//!
//! The pinned exemption (seed §4.2, ADR-0040 decision 8) is a `kind =
//! 'derived'` clause on both the selection and the delete, so a record
//! re-pinned between the two survives the second.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use synveda_types::{Error, RecordClass, RecordId, Result, ScopeId, TenantId};
use uuid::Uuid;

use crate::records::storage_error;

/// A record the horizons have caught: enough to audit the expiry, and
/// deliberately not its content (ADR-0040 compliance notes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueRecord {
    /// The record.
    pub id: RecordId,
    /// What it asserts — the class whose horizon selected it.
    pub class: RecordClass,
    /// The instant its retention clock started (ADR-0040 decision 3).
    pub valid_from: DateTime<Utc>,
}

/// What one staging disposal destroyed, per plane.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DisposedStaging {
    /// Staging rows destroyed.
    pub events: u64,
    /// Quarantine markers destroyed alongside them.
    pub quarantined: u64,
    /// Of those, markers still awaiting a review nobody did — counted
    /// separately because "three reviews that aged out" is a fact an
    /// auditor should be told rather than one they have to notice
    /// (ADR-0040 decision 7).
    pub quarantined_pending: u64,
}

/// Every scope of the tenant that holds derived material — the sweep's
/// work list, so a pass resolves one pack per *populated* scope rather
/// than one per hierarchy node.
///
/// Served by `records_tenant_scope_idx` (migration 0016), which CTX-1
/// added for the dense leg's selective regime.
#[tracing::instrument(
    name = "store.retention.populated_scopes",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn populated_scopes(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
) -> Result<Vec<ScopeId>> {
    let rows = sqlx::query_scalar!(
        r#"
        select distinct scope_id as "scope_id!"
        from records
        where tenant_id = $1 and kind = 'derived'
        -- A sealed scope is retention-held: its material is exempt from
        -- every horizon, because a hold whose whole purpose is to survive
        -- a schedule must not be implemented as one (AUTH-4, ADR-0059
        -- decision 8). Excluded at enumeration rather than per record, so
        -- the sweep never forms a work list it must then remember not to
        -- act on.
        and not exists (
            select 1 from identities i
            where i.tenant_id = $1 and i.scope_id = records.scope_id
              and i.status = 'departed'
        )
        order by scope_id
        "#,
        tenant_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    Ok(rows.into_iter().map(ScopeId::from_uuid).collect())
}

/// Whether the tenant holds any closed version at all — the cheap look
/// that lets an idle tenant skip the destruction stage's work list without
/// a pack resolution (the FLOW-4/AUTHZ-4 sweep discipline).
#[tracing::instrument(
    name = "store.retention.holds_closed_versions",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn holds_closed_versions(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
) -> Result<bool> {
    sqlx::query_scalar!(
        r#"select exists(select 1 from records_history where tenant_id = $1) as "exists!""#,
        tenant_id.as_uuid(),
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)
}

/// Whether the tenant holds any session-event payload at all — the same
/// cheap look for the disposal stage.
#[tracing::instrument(
    name = "store.retention.holds_staging",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn holds_staging(executor: impl PgExecutor<'_>, tenant_id: TenantId) -> Result<bool> {
    sqlx::query_scalar!(
        r#"select exists(select 1 from session_events where tenant_id = $1) as "exists!""#,
        tenant_id.as_uuid(),
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)
}

/// Derived records at `scope_id` whose `valid_from` is at or before the
/// cutoff their class was given, oldest first, capped at `limit`.
///
/// `cutoffs` carries only the classes that *have* a horizon: a class the
/// pack keeps is absent, not present with a cutoff at the beginning of
/// time (ADR-0040 decision 4). An empty slice therefore selects nothing,
/// which is the correct answer for a pack that expires nothing and the
/// only safe behaviour for a caller that forgot to filter.
#[tracing::instrument(
    name = "store.retention.due_at_scope",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %scope_id, classes = cutoffs.len()),
    err(Display)
)]
pub async fn due_at_scope(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    scope_id: ScopeId,
    cutoffs: &[(RecordClass, DateTime<Utc>)],
    limit: i64,
) -> Result<Vec<DueRecord>> {
    if cutoffs.is_empty() {
        return Ok(Vec::new());
    }
    let classes: Vec<String> = cutoffs
        .iter()
        .map(|(class, _)| class.as_str().to_owned())
        .collect();
    let instants: Vec<DateTime<Utc>> = cutoffs.iter().map(|(_, at)| *at).collect();
    let rows = sqlx::query!(
        r#"
        select r.id as "id!", r.class as "class!", r.valid_from as "valid_from!"
        from records r
        join unnest($3::text[], $4::timestamptz[]) as horizon(class, cutoff)
          on horizon.class = r.class
        where r.tenant_id = $1
          and r.scope_id = $2
          and r.kind = 'derived'
          and r.valid_from <= horizon.cutoff
        order by r.valid_from, r.id
        limit $5
        "#,
        tenant_id.as_uuid(),
        scope_id.as_uuid(),
        &classes,
        &instants,
        limit,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    rows.into_iter()
        .map(|row| {
            Ok(DueRecord {
                id: RecordId::from_uuid(row.id),
                class: row.class.parse().map_err(|err: Error| Error::Internal {
                    message: format!("stored value outside vocabulary: {err}"),
                })?,
                valid_from: row.valid_from,
            })
        })
        .collect()
}

/// Expires `ids`: the temporal delete (ADR-0040 decision 5).
///
/// The FND-4 trigger archives each current version into `records_history`
/// with its transaction period closed, so the record stops existing going
/// forward while `as_of` keeps answering — and the CTX-1 sidecar drops its
/// document on the next indexer pass, because `records_history.tx_to` is
/// half of that change feed (ADR-0024 decision 4).
///
/// Returns exactly what left, which is what the audit event describes: an
/// id re-pinned or already gone since selection is simply absent from the
/// result rather than a failure.
#[tracing::instrument(
    name = "store.retention.expire",
    skip_all,
    fields(tenant.id = %tenant_id, ids.count = ids.len()),
    err(Display)
)]
pub async fn expire(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    ids: &[RecordId],
) -> Result<Vec<DueRecord>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<Uuid> = ids.iter().map(RecordId::as_uuid).collect();
    let rows = sqlx::query!(
        r#"
        delete from records
        where tenant_id = $1 and id = any($2) and kind = 'derived'
        returning id as "id!", class as "class!", valid_from as "valid_from!"
        "#,
        tenant_id.as_uuid(),
        &ids,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    rows.into_iter()
        .map(|row| {
            Ok(DueRecord {
                id: RecordId::from_uuid(row.id),
                class: row.class.parse().map_err(|err: Error| Error::Internal {
                    message: format!("stored value outside vocabulary: {err}"),
                })?,
                valid_from: row.valid_from,
            })
        })
        .collect()
}

/// Every scope holding closed versions older than `cutoff` — the
/// destruction stage's work list, which cannot come from `records`: a
/// record that has already expired leaves no live row at its scope, and
/// its history is exactly what this stage exists to destroy.
///
/// Called only when some pack of the tenant configures a destruction
/// horizon at all, with `cutoff` the *shortest* of them (the sweep then
/// applies each scope's own). In the product default — no pack destroys
/// anything — this query is never issued.
#[tracing::instrument(
    name = "store.retention.scopes_with_closed_versions",
    skip_all,
    fields(tenant.id = %tenant_id, cutoff = %cutoff),
    err(Display)
)]
pub async fn scopes_with_closed_versions(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    cutoff: DateTime<Utc>,
) -> Result<Vec<ScopeId>> {
    let rows = sqlx::query_scalar!(
        r#"
        select distinct scope_id as "scope_id!"
        from records_history
        where tenant_id = $1 and tx_to <= $2
        -- A sealed scope is retention-held: its material is exempt from
        -- every horizon, because a hold whose whole purpose is to survive
        -- a schedule must not be implemented as one (AUTH-4, ADR-0059
        -- decision 8). Excluded at enumeration rather than per record, so
        -- the sweep never forms a work list it must then remember not to
        -- act on.
        and not exists (
            select 1 from identities i
            where i.tenant_id = $1 and i.scope_id = records_history.scope_id
              and i.status = 'departed'
        )
        order by scope_id
        "#,
        tenant_id.as_uuid(),
        cutoff,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    Ok(rows.into_iter().map(ScopeId::from_uuid).collect())
}

/// Destroys closed versions at `scope_id` whose transaction period ended
/// at or before `cutoff` — the second horizon, and the only statement in
/// the product that removes recorded content from the database (ADR-0040
/// decision 5).
///
/// Runs behind migration 0025's named flag: `records_history` is
/// append-only by trigger, the trigger lets a DELETE through only while
/// `synveda.retention_purge` is on, and the flag is set transaction-locally
/// here and cleared before returning. RLS is untouched throughout, so this
/// cannot reach another tenant's history however the flag is set (ADR-0040
/// decision 6).
///
/// Batched through the primary key, oldest first, so a pass is bounded and
/// a partial pass is simply a shorter one.
#[tracing::instrument(
    name = "store.retention.destroy_history",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %scope_id, cutoff = %cutoff, destroyed),
    err(Display)
)]
pub async fn destroy_history(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    scope_id: ScopeId,
    cutoff: DateTime<Utc>,
    limit: i64,
) -> Result<u64> {
    set_purge_flag(&mut *conn, true).await?;
    let result = sqlx::query!(
        r#"
        delete from records_history h
        using (
            select id, tx_from
            from records_history
            where tenant_id = $1 and scope_id = $2 and tx_to <= $3
            order by tx_to
            limit $4
        ) due
        where h.tenant_id = $1 and h.id = due.id and h.tx_from = due.tx_from
        "#,
        tenant_id.as_uuid(),
        scope_id.as_uuid(),
        cutoff,
        limit,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error);
    // Clear the flag whether or not the delete worked: a transaction that
    // rolls back releases it anyway, but one that continues to another
    // statement must not still be holding history open.
    set_purge_flag(&mut *conn, false).await?;
    let destroyed = result?.rows_affected();
    tracing::Span::current().record("destroyed", destroyed);
    Ok(destroyed)
}

/// Sets migration 0025's purge flag transaction-locally.
async fn set_purge_flag(conn: &mut PgConnection, on: bool) -> Result<()> {
    sqlx::query_scalar!(
        "select set_config('synveda.retention_purge', $1, true)",
        if on { "on" } else { "off" },
    )
    .fetch_one(&mut *conn)
    .await
    .map(|_| ())
    .map_err(|err| Error::Storage {
        message: format!("set retention purge flag: {err}"),
    })
}

/// Disposes of session events received at or before `cutoff`, with their
/// quarantine markers — the disposal migration 0044 deferred here and
/// migration 0046 granted (ADR-0040 decision 7, ADR-0078 decision 4).
///
/// Markers go first, because the FK points that way and because a marker
/// without its event is meaningless. Pending markers are disposed of like any
/// other and counted separately.
///
/// Disposal frees `(tenant_id, session_id, client_event_id)`: the append's
/// idempotency gate covers exactly as long as this plane is kept, which is why
/// the config's floor is a day.
///
/// Runs behind the same named flag as the history purge (migration 0025):
/// migration 0046's triggers refuse every delete from `session_events` and
/// `session_event_quarantine` unless the transaction has declared itself a
/// retention disposal, so a handler that has not cannot retire a run's
/// transcript by accident.
#[tracing::instrument(
    name = "store.retention.dispose_staging",
    skip_all,
    fields(tenant.id = %tenant_id, cutoff = %cutoff, events, quarantined),
    err(Display)
)]
pub async fn dispose_staging(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    cutoff: DateTime<Utc>,
    limit: i64,
) -> Result<DisposedStaging> {
    let due: Vec<Uuid> = sqlx::query_scalar!(
        r#"
        select id as "id!"
        from session_events
        where tenant_id = $1 and received_at <= $2
        order by received_at, id
        limit $3
        "#,
        tenant_id.as_uuid(),
        cutoff,
        limit,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    if due.is_empty() {
        return Ok(DisposedStaging::default());
    }
    set_purge_flag(&mut *conn, true).await?;
    let markers = sqlx::query!(
        r#"
        delete from session_event_quarantine
        where tenant_id = $1 and event_id = any($2)
        returning state as "state!"
        "#,
        tenant_id.as_uuid(),
        &due,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    let quarantined_pending = markers.iter().filter(|row| row.state == "pending").count() as u64;
    let events = sqlx::query!(
        "delete from session_events where tenant_id = $1 and id = any($2)",
        tenant_id.as_uuid(),
        &due,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?
    .rows_affected();
    set_purge_flag(&mut *conn, false).await?;
    let disposed = DisposedStaging {
        events,
        quarantined: markers.len() as u64,
        quarantined_pending,
    };
    tracing::Span::current().record("events", disposed.events);
    tracing::Span::current().record("quarantined", disposed.quarantined);
    Ok(disposed)
}
