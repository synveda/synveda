//! The quarantine review queue (MEM-2, ADR-0021 decision 5).
//!
//! A quarantined observe event's staging row exists — redacted, under
//! RLS, idempotent — but no work signal was sent; the
//! `observe_quarantine` row gates it. Review is one-shot
//! (`pending → released | rejected`, schema-enforced by migration
//! 0013's transition trigger): release sends the standard work signal in
//! the caller's transaction, so the pipeline cannot distinguish a
//! released event from an admitted one (the ADR-0020 decision 7
//! consumer contract, unchanged); reject leaves the staging row as
//! immutable provenance that never enters the pipeline. Reach this
//! module inside [`crate::rls::begin_tenant_tx`].

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use synveda_types::{
    Error, IdentityId, ObserveEventId, QuarantineState, Result, ScopeId, TenantId,
};

/// A quarantined event, joined with its staging row — everything a
/// reviewer sees. The payload is the *redacted* content; the raw
/// finding text was never stored anywhere (ADR-0021 decision 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantinedEvent {
    /// The gated staging row's id.
    pub event_id: ObserveEventId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// The event's home scope (where the write landed).
    pub scope_id: ScopeId,
    /// The submitting identity.
    pub owner_id: IdentityId,
    /// The harness session the event belongs to.
    pub session_id: String,
    /// The event kind (`transcript_delta` | `tool_result` | `decision`).
    pub kind: String,
    /// The redacted event body.
    pub payload: serde_json::Value,
    /// The scan's finding summary: `[{rule, category, count}]`.
    pub findings: serde_json::Value,
    /// Where the review stands.
    pub state: QuarantineState,
    /// When the event was quarantined.
    pub created_at: DateTime<Utc>,
    /// The reviewing subject, once reviewed.
    pub reviewer_subject: Option<String>,
    /// When the review happened.
    pub reviewed_at: Option<DateTime<Utc>>,
    /// The reviewer's optional note.
    pub review_reason: Option<String>,
}

/// A reviewer's verdict on one quarantined event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDecision {
    /// Send the work signal; the event joins the pipeline.
    Release,
    /// Never signal; the staging row stays provenance-only.
    Reject,
}

impl ReviewDecision {
    /// The state a review with this decision lands in.
    #[must_use]
    pub const fn resulting_state(self) -> QuarantineState {
        match self {
            ReviewDecision::Release => QuarantineState::Released,
            ReviewDecision::Reject => QuarantineState::Rejected,
        }
    }
}

/// Maps a sqlx error at the storage boundary into the shared taxonomy.
fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err
        && db.code().as_deref() == Some("42501")
    {
        return crate::rls::backstop_error(db);
    }
    Error::Storage {
        message: err.to_string(),
    }
}

struct QuarantineRow {
    event_id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    scope_id: uuid::Uuid,
    owner_id: uuid::Uuid,
    session_id: String,
    kind: String,
    payload: serde_json::Value,
    findings: serde_json::Value,
    state: String,
    created_at: DateTime<Utc>,
    reviewer_subject: Option<String>,
    reviewed_at: Option<DateTime<Utc>>,
    review_reason: Option<String>,
}

impl TryFrom<QuarantineRow> for QuarantinedEvent {
    type Error = Error;

    fn try_from(row: QuarantineRow) -> Result<Self> {
        Ok(QuarantinedEvent {
            event_id: ObserveEventId::from_uuid(row.event_id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            scope_id: ScopeId::from_uuid(row.scope_id),
            owner_id: IdentityId::from_uuid(row.owner_id),
            session_id: row.session_id,
            kind: row.kind,
            payload: row.payload,
            findings: row.findings,
            // The check constraint pins the column to the vocabulary; a
            // parse failure here means out-of-band schema drift.
            state: row.state.parse()?,
            created_at: row.created_at,
            reviewer_subject: row.reviewer_subject,
            reviewed_at: row.reviewed_at,
            review_reason: row.review_reason,
        })
    }
}

/// The pending review queue, oldest first, optionally filtered to a set
/// of scopes (the caller resolves a subtree to its scope ids — storage
/// knows nothing of authorization, seed §2.4).
#[tracing::instrument(name = "store.quarantine.pending", skip_all, err(Display))]
pub async fn pending(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    scope_ids: Option<&[ScopeId]>,
    limit: i64,
) -> Result<Vec<QuarantinedEvent>> {
    let scope_uuids: Option<Vec<uuid::Uuid>> =
        scope_ids.map(|ids| ids.iter().map(ScopeId::as_uuid).collect());
    let rows = sqlx::query_as!(
        QuarantineRow,
        r#"
        select q.event_id, q.tenant_id, q.scope_id, e.owner_id,
               e.session_id, e.kind, e.payload,
               q.findings, q.state, q.created_at,
               q.reviewer_subject, q.reviewed_at, q.review_reason
        from observe_quarantine q
        join observe_events e on e.id = q.event_id
        where q.tenant_id = $1
          and q.state = 'pending'
          and ($2::uuid[] is null or q.scope_id = any($2))
        order by q.created_at, q.event_id
        limit $3
        "#,
        tenant_id.as_uuid(),
        scope_uuids.as_deref(),
        limit,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// One quarantined event by id, whatever its state. `None` when no
/// quarantine row exists (the caller's uniform 404).
#[tracing::instrument(name = "store.quarantine.get", skip_all, err(Display))]
pub async fn get(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    event_id: ObserveEventId,
) -> Result<Option<QuarantinedEvent>> {
    let row = sqlx::query_as!(
        QuarantineRow,
        r#"
        select q.event_id, q.tenant_id, q.scope_id, e.owner_id,
               e.session_id, e.kind, e.payload,
               q.findings, q.state, q.created_at,
               q.reviewer_subject, q.reviewed_at, q.review_reason
        from observe_quarantine q
        join observe_events e on e.id = q.event_id
        where q.tenant_id = $1 and q.event_id = $2
        "#,
        tenant_id.as_uuid(),
        event_id.as_uuid(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Reviews one pending event: flips its state and — on release — sends
/// the standard work signal, both on the caller's transaction, so the
/// review, its signal, and the caller's audit event commit or vanish
/// together. Returns the reviewed row; `None` when no quarantine row
/// exists (uniform 404); [`Error::Conflict`] when it was already
/// reviewed (review is one-shot).
#[tracing::instrument(
    name = "store.quarantine.review",
    skip_all,
    fields(event.id = %event_id),
    err(Display)
)]
pub async fn review(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    event_id: ObserveEventId,
    decision: ReviewDecision,
    reviewer_subject: &str,
    reason: Option<&str>,
) -> Result<Option<QuarantinedEvent>> {
    let updated = sqlx::query_as!(
        QuarantineRow,
        r#"
        with reviewed as (
            update observe_quarantine
            set state = $3,
                reviewer_subject = $4,
                reviewed_at = now(),
                review_reason = $5
            where tenant_id = $1 and event_id = $2 and state = 'pending'
            returning event_id, tenant_id, scope_id, findings, state,
                      created_at, reviewer_subject, reviewed_at,
                      review_reason
        )
        select r.event_id, r.tenant_id, r.scope_id, e.owner_id,
               e.session_id, e.kind, e.payload,
               r.findings, r.state, r.created_at,
               r.reviewer_subject, r.reviewed_at, r.review_reason
        from reviewed r
        join observe_events e on e.id = r.event_id
        "#,
        tenant_id.as_uuid(),
        event_id.as_uuid(),
        decision.resulting_state().as_str(),
        reviewer_subject,
        reason,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    let Some(row) = updated else {
        // Nothing pending: distinguish "no such quarantine" (the
        // caller's uniform 404) from "already reviewed" (one-shot).
        return match get(&mut *conn, tenant_id, event_id).await? {
            Some(event) => Err(Error::Conflict {
                message: format!(
                    "quarantine review is one-shot: event is already {}",
                    event.state
                ),
            }),
            None => Ok(None),
        };
    };
    let event: QuarantinedEvent = row.try_into()?;
    if decision == ReviewDecision::Release {
        // The standard content-free signal (ADR-0020 decision 1): the
        // consumer contract cannot tell a released event from an
        // admitted one.
        sqlx::query_scalar!(
            r#"select pgmq.send($1, $2::jsonb) as "msg_id!""#,
            crate::observe::OBSERVE_QUEUE,
            serde_json::json!({
                "tenant_id": tenant_id,
                "event_id": event.event_id,
            }),
        )
        .fetch_one(&mut *conn)
        .await
        .map_err(storage_error)?;
    }
    Ok(Some(event))
}
