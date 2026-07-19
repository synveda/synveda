//! Observe ingestion buffer (MEM-1, ADR-0020).
//!
//! Admission is idempotent at the buffer: event content lands in the
//! RLS-forced `observe_events` staging table keyed by
//! `(tenant_id, idempotency_key)` with `ON CONFLICT DO NOTHING`, and only
//! rows actually inserted are enqueued on the PGMQ `observe` queue — a
//! redelivered event can never enter the pipeline twice. Queue messages
//! carry `{tenant_id, event_id}` and nothing else; content never leaves
//! the RLS backstop (ADR-0009). Callers reach this module inside a
//! [`crate::rls::begin_tenant_tx`] transaction, so the staging rows, the
//! queue signals, and the caller's audit event commit or vanish together.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use synveda_types::{Error, IdentityId, ObserveEventId, ObserveKind, Result, ScopeId, TenantId};
use uuid::Uuid;

/// The PGMQ queue carrying observe work signals (created by migration
/// 0012; consumed by the extraction pipeline from MEM-2/3 on).
pub const OBSERVE_QUEUE: &str = "observe";

/// One event as submitted for admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewObserveEvent {
    /// Client-minted admission key; one key admits one event per tenant,
    /// first writer wins (ADR-0020 decision 2).
    pub idempotency_key: String,
    /// What the event reports.
    pub kind: ObserveKind,
    /// The event body; opaque to the buffer, shaped by the extraction
    /// pipeline's contract (MEM-3).
    pub payload: serde_json::Value,
    /// Client-asserted event time.
    pub occurred_at: DateTime<Utc>,
}

/// The admission outcome for one submitted event, in submission order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedEvent {
    /// The buffered event: freshly minted on acceptance, the *original*
    /// delivery's id when this submission was a duplicate.
    pub id: ObserveEventId,
    /// The submission's admission key.
    pub idempotency_key: String,
    /// True when an earlier delivery (or an earlier event in this same
    /// batch) already admitted this key.
    pub duplicate: bool,
}

/// Maps a sqlx error at the storage boundary into the shared taxonomy.
fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err
        && db.code().as_deref() == Some("42501")
    {
        // The RLS backstop (TEN-2, ADR-0009) or a missing grant: an
        // application defect, never the caller's fault.
        return crate::rls::backstop_error(db);
    }
    Error::Storage {
        message: err.to_string(),
    }
}

/// Admits a batch of observe events for `owner_id` at `scope_id`: inserts
/// the non-duplicate events into staging and enqueues one work signal per
/// inserted row, all on the caller's transaction. Returns one
/// [`AdmittedEvent`] per input event, in input order.
///
/// Duplicates — a key already admitted by an earlier delivery, or repeated
/// within this batch — are reported, not errored: for an at-least-once
/// client, redelivery is the success case (ADR-0020 decision 2).
#[tracing::instrument(
    name = "store.observe.buffer_batch",
    skip_all,
    fields(events = events.len(), session.id = %session_id),
    err(Display)
)]
pub async fn buffer_batch(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    scope_id: ScopeId,
    owner_id: IdentityId,
    session_id: &str,
    events: &[NewObserveEvent],
) -> Result<Vec<AdmittedEvent>> {
    // In-batch repeats never reach the database: only the first occurrence
    // of a key is submitted, so any conflict the insert sees is a real
    // cross-delivery redelivery.
    let mut first_occurrence: Vec<&NewObserveEvent> = Vec::with_capacity(events.len());
    let mut minted: HashMap<&str, ObserveEventId> = HashMap::with_capacity(events.len());
    for event in events {
        if !minted.contains_key(event.idempotency_key.as_str()) {
            minted.insert(event.idempotency_key.as_str(), ObserveEventId::new());
            first_occurrence.push(event);
        }
    }

    let ids: Vec<Uuid> = first_occurrence
        .iter()
        .map(|event| minted[event.idempotency_key.as_str()].as_uuid())
        .collect();
    let keys: Vec<String> = first_occurrence
        .iter()
        .map(|event| event.idempotency_key.clone())
        .collect();
    let kinds: Vec<String> = first_occurrence
        .iter()
        .map(|event| event.kind.as_str().to_owned())
        .collect();
    let payloads: Vec<serde_json::Value> = first_occurrence
        .iter()
        .map(|event| event.payload.clone())
        .collect();
    let occurred: Vec<DateTime<Utc>> = first_occurrence
        .iter()
        .map(|event| event.occurred_at)
        .collect();

    let inserted_keys: Vec<String> = sqlx::query_scalar!(
        r#"
        insert into observe_events
            (id, tenant_id, scope_id, owner_id, session_id,
             idempotency_key, kind, payload, occurred_at)
        select u.id, $1, $2, $3, $4, u.idempotency_key, u.kind, u.payload,
               u.occurred_at
        from unnest($5::uuid[], $6::text[], $7::text[], $8::jsonb[],
                    $9::timestamptz[])
            as u(id, idempotency_key, kind, payload, occurred_at)
        on conflict (tenant_id, idempotency_key) do nothing
        returning idempotency_key
        "#,
        tenant_id.as_uuid(),
        scope_id.as_uuid(),
        owner_id.as_uuid(),
        session_id,
        &ids,
        &keys,
        &kinds,
        &payloads,
        &occurred,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;

    // Keys the insert skipped were admitted by an earlier delivery; report
    // them with the original event's id so acks are idempotent.
    let mut admitted: HashMap<String, (ObserveEventId, bool)> = inserted_keys
        .into_iter()
        .map(|key| {
            let id = minted[key.as_str()];
            (key, (id, false))
        })
        .collect();
    let redelivered: Vec<String> = keys
        .iter()
        .filter(|key| !admitted.contains_key(key.as_str()))
        .cloned()
        .collect();
    if !redelivered.is_empty() {
        let originals = sqlx::query!(
            r#"
            select id, idempotency_key
            from observe_events
            where tenant_id = $1 and idempotency_key = any($2::text[])
            "#,
            tenant_id.as_uuid(),
            &redelivered,
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(storage_error)?;
        for row in originals {
            admitted.insert(
                row.idempotency_key,
                (ObserveEventId::from_uuid(row.id), true),
            );
        }
    }

    // One work signal per row actually inserted, on the same transaction:
    // the pipeline can never see a delivery twice, and a rollback retracts
    // rows and signals together.
    let signals: Vec<serde_json::Value> = keys
        .iter()
        .filter_map(|key| match admitted.get(key.as_str()) {
            Some((id, false)) => Some(serde_json::json!({
                "tenant_id": tenant_id,
                "event_id": id,
            })),
            _ => None,
        })
        .collect();
    if !signals.is_empty() {
        sqlx::query_scalar!(
            r#"select pgmq.send_batch($1, $2::jsonb[]) as "msg_id!""#,
            OBSERVE_QUEUE,
            &signals,
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(storage_error)?;
    }

    let mut seen_in_batch: HashSet<&str> = HashSet::with_capacity(events.len());
    events
        .iter()
        .map(|event| {
            let key = event.idempotency_key.as_str();
            // Every key was either inserted or resolved to its original
            // row above; anything else is a defect, never a guess.
            let (id, redelivery) = admitted.get(key).copied().ok_or_else(|| Error::Internal {
                message: format!("observe admission lost track of key {key:?}"),
            })?;
            let repeat = !seen_in_batch.insert(key);
            Ok(AdmittedEvent {
                id,
                idempotency_key: event.idempotency_key.clone(),
                duplicate: redelivery || repeat,
            })
        })
        .collect()
}

/// The queue depth of the observe buffer — enqueued signals not yet read
/// or archived by the pipeline. Diagnostic surface for the demo and tests;
/// the pipeline's own consumption metrics arrive with MEM-3.
#[tracing::instrument(name = "store.observe.queue_depth", skip_all, err(Display))]
pub async fn queue_depth(conn: &mut PgConnection) -> Result<i64> {
    sqlx::query_scalar!(r#"select count(*) as "count!" from pgmq.q_observe"#)
        .fetch_one(conn)
        .await
        .map_err(storage_error)
}
