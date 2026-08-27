//! The quarantine review queue (MEM-2, ADR-0021 decision 5; re-anchored on
//! session events by CPR-12, ADR-0078 decision 4).
//!
//! A quarantined event's row exists — redacted, ordered, under RLS, idempotent
//! by the client's own event id — but the `session_event_quarantine` row makes
//! it ineligible for capture. Review is one-shot (`pending → released |
//! rejected`, schema-enforced by migration 0046's transition trigger): release
//! makes a future batch eligible to freeze it; reject leaves the event as
//! immutable provenance that never enters extraction. Reach this module
//! inside [`crate::rls::begin_tenant_tx`].
//!
//! ## Why the gate is a second table and not a column
//!
//! `session_events` holds SELECT and INSERT and no UPDATE (migration 0044), and
//! that is the whole of its immutability guarantee. A `quarantined` column
//! would need an UPDATE grant to clear, which would hand every caller the
//! ability to rewrite an event's payload too. So the reviewable state lives
//! here and the event row carries only `redactions` — decided once, at
//! admission, and true of that payload forever.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use synveda_types::{Error, QuarantineState, Result, ScopeId, SessionEventId, SessionId, TenantId};

/// A quarantined event, joined with its event row and its run — everything a
/// reviewer sees. The payload is the *redacted* content; the raw finding text
/// was never stored anywhere (ADR-0021 decision 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantinedEvent {
    /// The gated event's id.
    pub event_id: SessionEventId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// The governed scope the run was decided at — where the memory would
    /// have landed, and what a subtree-filtered review queue narrows by.
    pub scope_id: ScopeId,
    /// The run the event belongs to.
    pub session_id: SessionId,
    /// The token subject that opened that run.
    pub principal_id: String,
    /// What happened — one of `SessionEventType`'s twelve names.
    pub event_type: String,
    /// The client's own id for the event.
    pub client_event_id: String,
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
    /// Make the event eligible for the next capture snapshot.
    Release,
    /// Keep the event as provenance only.
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
    session_id: uuid::Uuid,
    principal_id: String,
    event_type: String,
    client_event_id: String,
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
            event_id: SessionEventId::from_uuid(row.event_id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            scope_id: ScopeId::from_uuid(row.scope_id),
            session_id: SessionId::from_uuid(row.session_id),
            principal_id: row.principal_id,
            event_type: row.event_type,
            client_event_id: row.client_event_id,
            payload: row.payload,
            findings: row.findings,
            // The check constraint pins the column to the vocabulary; a parse
            // failure here means out-of-band schema drift.
            state: row.state.parse()?,
            created_at: row.created_at,
            reviewer_subject: row.reviewer_subject,
            reviewed_at: row.reviewed_at,
            review_reason: row.review_reason,
        })
    }
}

/// The pending review queue, oldest first, optionally filtered to a set of
/// scopes (the caller resolves a subtree to its scope ids — storage knows
/// nothing of authorization, seed §2.4).
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
        select q.event_id, q.tenant_id, q.scope_id, q.session_id,
               s.principal_id, e.event_type, e.client_event_id, e.payload,
               q.findings, q.state, q.created_at,
               q.reviewer_subject, q.reviewed_at, q.review_reason
        from session_event_quarantine q
        join session_events e
          on e.tenant_id = q.tenant_id and e.id = q.event_id
        join sessions s
          on s.tenant_id = q.tenant_id and s.id = q.session_id
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

/// One quarantined event by id, whatever its state. `None` when no quarantine
/// row exists (the caller's uniform 404).
#[tracing::instrument(name = "store.quarantine.get", skip_all, err(Display))]
pub async fn get(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    event_id: SessionEventId,
) -> Result<Option<QuarantinedEvent>> {
    let row = sqlx::query_as!(
        QuarantineRow,
        r#"
        select q.event_id, q.tenant_id, q.scope_id, q.session_id,
               s.principal_id, e.event_type, e.client_event_id, e.payload,
               q.findings, q.state, q.created_at,
               q.reviewer_subject, q.reviewed_at, q.review_reason
        from session_event_quarantine q
        join session_events e
          on e.tenant_id = q.tenant_id and e.id = q.event_id
        join sessions s
          on s.tenant_id = q.tenant_id and s.id = q.session_id
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

/// Reviews one pending event, changing its future capture eligibility in the
/// caller's transaction so the verdict and its audit event commit together.
///
/// Returns the reviewed row; `None` when no quarantine row exists (uniform
/// 404); [`Error::Conflict`] when it was already reviewed (review is
/// one-shot).
#[tracing::instrument(
    name = "store.quarantine.review",
    skip_all,
    fields(event.id = %event_id),
    err(Display)
)]
pub async fn review(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    event_id: SessionEventId,
    decision: ReviewDecision,
    reviewer_subject: &str,
    reason: Option<&str>,
) -> Result<Option<QuarantinedEvent>> {
    let updated = sqlx::query_as!(
        QuarantineRow,
        r#"
        with reviewed as (
            update session_event_quarantine
            set state = $3,
                reviewer_subject = $4,
                reviewed_at = now(),
                review_reason = $5
            where tenant_id = $1 and event_id = $2 and state = 'pending'
            returning event_id, tenant_id, scope_id, session_id, findings,
                      state, created_at, reviewer_subject, reviewed_at,
                      review_reason
        )
        select r.event_id, r.tenant_id, r.scope_id, r.session_id,
               s.principal_id, e.event_type, e.client_event_id, e.payload,
               r.findings, r.state, r.created_at,
               r.reviewer_subject, r.reviewed_at, r.review_reason
        from reviewed r
        join session_events e
          on e.tenant_id = r.tenant_id and e.id = r.event_id
        join sessions s
          on s.tenant_id = r.tenant_id and s.id = r.session_id
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
        // Nothing pending: distinguish "no such quarantine" (the caller's
        // uniform 404) from "already reviewed" (one-shot).
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
    // A release changes eligibility only. The next explicit or terminal
    // capture freezes a new immutable input digest when this event belongs in
    // it; there is no per-event work signal and no direct active-content
    // writer (CPR-18, ADR-0083).
    Ok(Some(event))
}
