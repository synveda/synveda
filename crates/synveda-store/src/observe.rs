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

/// One event as submitted for admission — payload already redacted by
/// the scan seam (MEM-2, ADR-0021): raw findings never reach this
/// module in any mode.
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
    /// The scan's finding summary (rule ids, categories, counts — never
    /// matched text), stamped on the staging row as provenance. `None`
    /// when the payload was clean.
    pub redactions: Option<serde_json::Value>,
    /// Stage without a work signal and open a pending review row
    /// (ADR-0021 decision 5). The pipeline cannot see the event until a
    /// reviewer releases it.
    pub quarantine: bool,
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
    /// True when *this* submission staged signal-less behind a pending
    /// review row. Always false for duplicates: the winning delivery's
    /// disposition stands, whatever it was.
    pub quarantined: bool,
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
/// the non-duplicate events into staging, enqueues one work signal per
/// inserted non-quarantined row, and opens one pending review row per
/// inserted quarantined row (MEM-2, ADR-0021 decision 5) — all on the
/// caller's transaction. Returns one [`AdmittedEvent`] per input event,
/// in input order.
///
/// Duplicates — a key already admitted by an earlier delivery, or repeated
/// within this batch — are reported, not errored: for an at-least-once
/// client, redelivery is the success case (ADR-0020 decision 2). The
/// winning delivery's disposition stands: a redelivered quarantined event
/// neither re-quarantines nor signals.
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
    // Nullable array elements can't ride a typed vec; `redactions` goes
    // over as a jsonb array whose elements are the per-event summary or
    // json null (mapped back to a SQL null in the insert).
    let redactions = serde_json::Value::Array(
        first_occurrence
            .iter()
            .map(|event| event.redactions.clone().unwrap_or(serde_json::Value::Null))
            .collect(),
    );

    let inserted_keys: Vec<String> = sqlx::query_scalar!(
        r#"
        insert into observe_events
            (id, tenant_id, scope_id, owner_id, session_id,
             idempotency_key, kind, payload, occurred_at, redactions)
        select u.id, $1, $2, $3, $4, u.idempotency_key, u.kind, u.payload,
               u.occurred_at,
               nullif($10::jsonb -> (u.ord - 1)::int, 'null'::jsonb)
        from unnest($5::uuid[], $6::text[], $7::text[], $8::jsonb[],
                    $9::timestamptz[])
                with ordinality as u(id, idempotency_key, kind, payload,
                                     occurred_at, ord)
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
        redactions,
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

    // One work signal per row actually inserted *and not quarantined*,
    // on the same transaction: the pipeline can never see a delivery
    // twice, a quarantined event stays invisible to it until release
    // (ADR-0021 decision 5), and a rollback retracts rows, signals, and
    // review markers together.
    let quarantined_by_key: HashMap<&str, &NewObserveEvent> = first_occurrence
        .iter()
        .filter(|event| event.quarantine)
        .map(|event| (event.idempotency_key.as_str(), *event))
        .collect();
    let mut signals: Vec<serde_json::Value> = Vec::new();
    let mut review_ids: Vec<Uuid> = Vec::new();
    let mut review_findings: Vec<serde_json::Value> = Vec::new();
    for key in &keys {
        let Some((id, false)) = admitted.get(key.as_str()) else {
            continue;
        };
        match quarantined_by_key.get(key.as_str()) {
            Some(event) => {
                review_ids.push(id.as_uuid());
                // A quarantined event always has findings — that is what
                // quarantined it — but the column is NOT NULL, so an
                // empty list is the defensive shape, never a SQL null.
                review_findings.push(
                    event
                        .redactions
                        .clone()
                        .unwrap_or(serde_json::Value::Array(Vec::new())),
                );
            }
            None => signals.push(serde_json::json!({
                "tenant_id": tenant_id,
                "event_id": id,
            })),
        }
    }
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
    if !review_ids.is_empty() {
        sqlx::query!(
            r#"
            insert into observe_quarantine (event_id, tenant_id, scope_id, findings)
            select u.event_id, $1, $2, u.findings
            from unnest($3::uuid[], $4::jsonb[]) as u(event_id, findings)
            "#,
            tenant_id.as_uuid(),
            scope_id.as_uuid(),
            &review_ids,
            &review_findings,
        )
        .execute(&mut *conn)
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
            let duplicate = redelivery || repeat;
            Ok(AdmittedEvent {
                id,
                idempotency_key: event.idempotency_key.clone(),
                duplicate,
                quarantined: !duplicate && event.quarantine,
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

/// One work signal read from the observe queue, invisible to other
/// readers until its visibility timeout elapses or it is archived
/// (ADR-0020 decision 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueuedSignal {
    /// PGMQ message id — the archive handle.
    pub msg_id: i64,
    /// How many times the message has been read, this read included.
    /// The consumer dead-letters past its threshold (ADR-0022
    /// decision 6).
    pub read_ct: i32,
    /// The tenant whose transaction can see the staged content.
    pub tenant_id: TenantId,
    /// The staged event the signal names.
    pub event_id: ObserveEventId,
}

/// One message read from the observe queue: a well-formed work signal,
/// or a body only a database-credentialed writer could have produced
/// (both admission paths serialize [`QueuedSignal`]'s shape). The
/// consumer archives malformed messages defensively — they can never
/// become processable and must not wedge the queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObserveMessage {
    /// A well-formed `{tenant_id, event_id}` work signal.
    Signal(QueuedSignal),
    /// A message whose body did not parse; carries the archive handle.
    Malformed {
        /// PGMQ message id — the archive handle.
        msg_id: i64,
    },
}

/// Parses the well-formed signal body — the shape [`buffer_batch`] and
/// the quarantine release path serialize. Hand-rolled: this crate takes
/// no serde-derive dependency for one two-field object.
fn parse_signal_body(message: &serde_json::Value) -> Option<(TenantId, ObserveEventId)> {
    let field = |name: &str| {
        message
            .get(name)
            .and_then(serde_json::Value::as_str)
            .and_then(|text| text.parse::<Uuid>().ok())
    };
    Some((
        TenantId::from_uuid(field("tenant_id")?),
        ObserveEventId::from_uuid(field("event_id")?),
    ))
}

/// Reads up to `qty` work signals off the observe queue with a `vt_secs`
/// visibility timeout: each returned message is invisible to other
/// readers until the timeout elapses or [`archive_signal`] consumes it.
/// Runs on a plain pool connection — queue messages are content-free by
/// design (ADR-0020 decision 1), so no tenant transaction is needed
/// until the staging row is loaded.
#[tracing::instrument(name = "store.observe.read_signals", skip_all, err(Display))]
pub async fn read_signals(
    conn: &mut PgConnection,
    vt_secs: i32,
    qty: i32,
) -> Result<Vec<ObserveMessage>> {
    let rows = sqlx::query!(
        r#"
        select msg_id as "msg_id!", read_ct as "read_ct!",
               message as "message!"
        from pgmq.read($1::text, $2::int, $3::int)
        "#,
        OBSERVE_QUEUE,
        vt_secs,
        qty,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    Ok(rows
        .into_iter()
        .map(|row| match parse_signal_body(&row.message) {
            Some((tenant_id, event_id)) => ObserveMessage::Signal(QueuedSignal {
                msg_id: row.msg_id,
                read_ct: row.read_ct,
                tenant_id,
                event_id,
            }),
            None => ObserveMessage::Malformed { msg_id: row.msg_id },
        })
        .collect())
}

/// Archives one queue message, consuming it. Returns `false` when the
/// message was no longer in the queue — under redelivery races that
/// means another consumer already committed this signal's work, and the
/// caller must treat the signal as done (ADR-0022 decision 2: callers
/// commit records and archive in one transaction, so the delete's row
/// lock serializes contenders and the loser sees `false`).
#[tracing::instrument(name = "store.observe.archive_signal", skip_all, fields(msg.id = msg_id), err(Display))]
pub async fn archive_signal(conn: &mut PgConnection, msg_id: i64) -> Result<bool> {
    sqlx::query_scalar!(
        r#"select pgmq.archive($1::text, $2::bigint) as "archived!""#,
        OBSERVE_QUEUE,
        msg_id,
    )
    .fetch_one(conn)
    .await
    .map_err(storage_error)
}

/// One staged observe event as the pipeline loads it: redacted content
/// plus the placement and provenance the extraction stages need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedEvent {
    /// The staging row's id.
    pub id: ObserveEventId,
    /// The home scope the event was admitted at.
    pub scope_id: ScopeId,
    /// The identity whose session produced the event.
    pub owner_id: IdentityId,
    /// The client's session identifier — the AC's source session.
    pub session_id: String,
    /// What the event reports; drives extraction routing.
    pub kind: ObserveKind,
    /// The redacted event body (ADR-0021: `[REDACTED:*]` placeholders
    /// are opaque tokens from here on).
    pub payload: serde_json::Value,
    /// Client-asserted event time — the record's valid-from.
    pub occurred_at: DateTime<Utc>,
    /// When admission committed; the pipeline-lag clock starts here.
    pub received_at: DateTime<Utc>,
    /// The admission scan's finding summary, carried into record
    /// provenance. `None` when the payload was clean.
    pub redactions: Option<serde_json::Value>,
}

/// Loads one staged event by id, inside the caller's tenant transaction
/// (ADR-0020 decision 7). `None` when the row does not exist — a signal
/// naming a missing row is archived with a warning, never retried.
#[tracing::instrument(name = "store.observe.load_event", skip_all, fields(event.id = %event_id), err(Display))]
pub async fn load_event(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    event_id: ObserveEventId,
) -> Result<Option<StagedEvent>> {
    let row = sqlx::query!(
        r#"
        select scope_id, owner_id, session_id, kind, payload, occurred_at,
               received_at, redactions
        from observe_events
        where tenant_id = $1 and id = $2
        "#,
        tenant_id.as_uuid(),
        event_id.as_uuid(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    row.map(|row| {
        Ok(StagedEvent {
            id: event_id,
            scope_id: ScopeId::from_uuid(row.scope_id),
            owner_id: IdentityId::from_uuid(row.owner_id),
            session_id: row.session_id,
            // The CHECK constraint keeps `kind` inside the vocabulary; a
            // parse failure means schema and code drifted — a bug.
            kind: row.kind.parse().map_err(|err| Error::Internal {
                message: format!("stored value outside vocabulary: {err}"),
            })?,
            payload: row.payload,
            occurred_at: row.occurred_at,
            received_at: row.received_at,
            redactions: row.redactions,
        })
    })
    .transpose()
}
