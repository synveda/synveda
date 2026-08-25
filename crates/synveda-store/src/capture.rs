//! Durable session capture batches and reviewable candidates (CPR-18,
//! ADR-0083).
//!
//! This module freezes evidence and persists extraction results. It decides
//! neither session access nor Knowledge publication: the gateway and worker
//! take those PDP decisions before calling these functions, and accepted
//! candidates enter the Knowledge command layer above this crate.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgConnection, PgExecutor};
use synveda_types::capture::{
    CaptureBatch, CaptureBatchState, CaptureCandidate, CaptureCandidateDecision,
    CaptureCandidateState, CaptureDecisionAction, CaptureDecisionState, CaptureMatch,
    CaptureSourceKind, MAX_CAPTURE_ATTEMPTS,
};
use synveda_types::knowledge::{
    KnowledgeOrigin, KnowledgeRevisionContent, KnowledgeType, normalise_knowledge_tags,
    validate_content_hash, validate_knowledge_principal, validate_knowledge_revision_content,
};
use synveda_types::session::{Session, SessionEventType};
use synveda_types::{
    CaptureBatchId, CaptureCandidateDecisionId, CaptureCandidateId, Error, ImportArtifactId,
    ImportJobId, KnowledgeItemId, KnowledgeRevisionId, ProjectId, ProposalId, Result, ScopeId,
    SessionEventId, SessionId, TenantId, WorkspaceId,
};
use uuid::Uuid;

/// Maximum eligible events frozen into one extraction batch.
pub const MAX_CAPTURE_EVENTS: i64 = 2_000;
/// Maximum candidates scanned by one list request before PDP filtering.
pub const CAPTURE_SCAN_LIMIT: i64 = 500;
/// Low-level capture persistence transitions.
pub const CAPTURE_MUTATIONS_TOTAL: &str = "synveda_capture_mutations_total";

/// Result of asking to freeze the current eligible event snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenBatch {
    /// Stable batch, new or replayed.
    pub batch: CaptureBatch,
    /// Whether this call inserted it and its evidence links.
    pub created: bool,
}

/// One exact event loaded from a frozen batch for extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenEvent {
    /// Immutable event id.
    pub id: SessionEventId,
    /// Event vocabulary.
    pub event_type: SessionEventType,
    /// Redacted payload.
    pub payload: Value,
    /// Client-declared event time.
    pub occurred_at: DateTime<Utc>,
    /// Admission finding summary.
    pub redactions: Option<Value>,
    /// Stable batch position.
    pub ordinal: i32,
}

/// One candidate ready to insert after extraction and validation.
#[derive(Debug, Clone)]
pub struct NewCaptureCandidate {
    /// Stable id.
    pub id: CaptureCandidateId,
    /// Position in the batch.
    pub ordinal: i32,
    /// Proposed governing scope.
    pub proposed_scope_id: ScopeId,
    /// Proposed project association.
    pub proposed_project_id: Option<ProjectId>,
    /// Proposed personal owner.
    pub proposed_owner_principal_id: Option<String>,
    /// Proposed Knowledge type.
    pub knowledge_type: KnowledgeType,
    /// Source origin.
    pub origin: KnowledgeOrigin,
    /// Complete proposed content.
    pub content: KnowledgeRevisionContent,
    /// Canonical content hash.
    pub content_hash: String,
    /// Exact frozen events supporting the proposal.
    pub source_event_ids: Vec<SessionEventId>,
    /// Independently authorised current-Knowledge comparisons.
    pub matches: Vec<CaptureMatch>,
}

/// Keyset for batch listings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchCursor {
    /// Creation instant.
    pub created_at: DateTime<Utc>,
    /// Stable tie-breaker.
    pub id: CaptureBatchId,
}

/// Batch listing filters.
#[derive(Debug, Clone, Default)]
pub struct BatchFilter {
    /// One session.
    pub session_id: Option<SessionId>,
    /// One project.
    pub project_id: Option<ProjectId>,
    /// One exact job state.
    pub state: Option<CaptureBatchState>,
    /// Resume after this key.
    pub after: Option<BatchCursor>,
}

/// Keyset for candidate listings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateCursor {
    /// Creation instant.
    pub created_at: DateTime<Utc>,
    /// Stable tie-breaker.
    pub id: CaptureCandidateId,
}

/// Candidate listing filters.
#[derive(Debug, Clone, Default)]
pub struct CandidateFilter {
    /// One batch.
    pub batch_id: Option<CaptureBatchId>,
    /// One session.
    pub session_id: Option<SessionId>,
    /// One project.
    pub project_id: Option<ProjectId>,
    /// One exact review state.
    pub state: Option<CaptureCandidateState>,
    /// Resume after this key.
    pub after: Option<CandidateCursor>,
}

struct BatchRow {
    id: Uuid,
    tenant_id: Uuid,
    source_kind: String,
    session_id: Option<Uuid>,
    import_job_id: Option<Uuid>,
    scope_id: Uuid,
    workspace_id: Uuid,
    project_id: Option<Uuid>,
    principal_id: String,
    input_hash: String,
    event_count: i32,
    state: String,
    extractor_method: Option<String>,
    model_version: Option<String>,
    attempts: i32,
    candidate_count: i32,
    error_code: Option<String>,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<BatchRow> for CaptureBatch {
    type Error = Error;

    fn try_from(row: BatchRow) -> Result<Self> {
        Ok(Self {
            id: CaptureBatchId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            source_kind: stored(&row.source_kind)?,
            session_id: row.session_id.map(SessionId::from_uuid),
            import_job_id: row.import_job_id.map(ImportJobId::from_uuid),
            scope_id: ScopeId::from_uuid(row.scope_id),
            workspace_id: WorkspaceId::from_uuid(row.workspace_id),
            project_id: row.project_id.map(ProjectId::from_uuid),
            principal_id: row.principal_id,
            input_hash: row.input_hash,
            event_count: row.event_count,
            state: stored(&row.state)?,
            extractor_method: row.extractor_method,
            model_version: row.model_version,
            attempts: row.attempts,
            candidate_count: row.candidate_count,
            error_code: row.error_code,
            created_at: row.created_at,
            started_at: row.started_at,
            completed_at: row.completed_at,
            updated_at: row.updated_at,
        })
    }
}

struct CandidateRow {
    id: Uuid,
    tenant_id: Uuid,
    batch_id: Uuid,
    source_kind: String,
    session_id: Option<Uuid>,
    import_job_id: Option<Uuid>,
    ordinal: i32,
    proposed_scope_id: Uuid,
    proposed_project_id: Option<Uuid>,
    proposed_owner_principal_id: Option<String>,
    knowledge_type: String,
    origin: String,
    title: String,
    body_markdown: String,
    summary: String,
    tags: Vec<String>,
    sensitivity: String,
    confidence_permille: i32,
    valid_from: DateTime<Utc>,
    valid_to: Option<DateTime<Utc>>,
    stale_after: Option<DateTime<Utc>>,
    verification_metadata: Value,
    metadata: Value,
    content_hash: String,
    state: String,
    resulting_change_id: Option<Uuid>,
    resulting_outcome: Option<String>,
    resulting_knowledge_item_id: Option<Uuid>,
    resulting_revision_id: Option<Uuid>,
    decided_by: Option<String>,
    decision_reason: Option<String>,
    decided_at: Option<DateTime<Utc>>,
    content_erased: bool,
    created_at: DateTime<Utc>,
}

fn candidate_without_links(row: CandidateRow) -> Result<CaptureCandidate> {
    Ok(CaptureCandidate {
        id: CaptureCandidateId::from_uuid(row.id),
        tenant_id: TenantId::from_uuid(row.tenant_id),
        batch_id: CaptureBatchId::from_uuid(row.batch_id),
        source_kind: stored(&row.source_kind)?,
        session_id: row.session_id.map(SessionId::from_uuid),
        import_job_id: row.import_job_id.map(ImportJobId::from_uuid),
        ordinal: row.ordinal,
        proposed_scope_id: ScopeId::from_uuid(row.proposed_scope_id),
        proposed_project_id: row.proposed_project_id.map(ProjectId::from_uuid),
        proposed_owner_principal_id: row.proposed_owner_principal_id,
        knowledge_type: stored(&row.knowledge_type)?,
        origin: stored(&row.origin)?,
        content: KnowledgeRevisionContent {
            title: row.title,
            body_markdown: row.body_markdown,
            summary: row.summary,
            tags: row.tags,
            sensitivity: stored(&row.sensitivity)?,
            confidence_permille: row.confidence_permille,
            valid_from: row.valid_from,
            valid_to: row.valid_to,
            stale_after: row.stale_after,
            verification_metadata: row.verification_metadata,
            metadata: row.metadata,
        },
        content_hash: row.content_hash,
        state: stored(&row.state)?,
        source_event_ids: Vec::new(),
        source_artifact_ids: Vec::new(),
        matches: Vec::new(),
        resulting_change_id: row.resulting_change_id.map(ProposalId::from_uuid),
        resulting_outcome: row.resulting_outcome.as_deref().map(stored).transpose()?,
        resulting_knowledge_item_id: row
            .resulting_knowledge_item_id
            .map(KnowledgeItemId::from_uuid),
        resulting_revision_id: row
            .resulting_revision_id
            .map(KnowledgeRevisionId::from_uuid),
        decided_by: row.decided_by,
        decision_reason: row.decision_reason,
        decided_at: row.decided_at,
        content_erased: row.content_erased,
        created_at: row.created_at,
    })
}

fn stored<T: std::str::FromStr<Err = Error>>(value: &str) -> Result<T> {
    value.parse().map_err(|error| Error::Internal {
        message: format!("stored value outside vocabulary: {error}"),
    })
}

fn storage_error(error: sqlx::Error) -> Error {
    if let sqlx::Error::Database(database) = &error {
        match database.code().as_deref() {
            Some("23505") => {
                return Error::Conflict {
                    message: database.to_string(),
                };
            }
            Some("23503" | "23514" | "22001") => {
                return Error::Invalid {
                    message: database.to_string(),
                };
            }
            Some("42501") => return crate::rls::backstop_error(database),
            _ => {}
        }
    }
    Error::Storage {
        message: error.to_string(),
    }
}

/// BLAKE3-256 over unambiguous, length-prefixed evidence tuples.
fn snapshot_hash(events: &[(Uuid, String, String)]) -> String {
    let mut hash = blake3::Hasher::new();
    hash.update(b"synveda.capture.snapshot.v1\0");
    for (id, event_type, payload_hash) in events {
        hash.update(id.as_bytes());
        hash.update(&(event_type.len() as u64).to_be_bytes());
        hash.update(event_type.as_bytes());
        hash.update(&(payload_hash.len() as u64).to_be_bytes());
        hash.update(payload_hash.as_bytes());
    }
    hash.finalize().to_hex().to_string()
}

/// Freezes the current eligible event set, replaying the existing batch when
/// the same immutable snapshot was already requested.
pub async fn freeze_batch(conn: &mut PgConnection, session: &Session) -> Result<FrozenBatch> {
    let events = sqlx::query!(
        r#"
        select event.id, event.event_type, event.payload_hash, event.sequence
        from session_events event
        left join session_event_quarantine quarantine
          on quarantine.tenant_id = event.tenant_id
         and quarantine.event_id = event.id
        where event.tenant_id = $1 and event.session_id = $2
          and event.event_type in (
              'message.user', 'message.assistant', 'tool.invoked', 'tool.result',
              'file.changed', 'command.executed', 'memory.asserted'
          )
          and (quarantine.event_id is null or quarantine.state = 'released')
        order by event.sequence
        limit $3
        "#,
        session.tenant_id.as_uuid(),
        session.id.as_uuid(),
        MAX_CAPTURE_EVENTS + 1,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    if events.len() as i64 > MAX_CAPTURE_EVENTS {
        return Err(Error::Invalid {
            message: format!(
                "session {} has more than {MAX_CAPTURE_EVENTS} eligible events; close smaller runs",
                session.id
            ),
        });
    }
    let evidence: Vec<(Uuid, String, String)> = events
        .iter()
        .map(|event| {
            (
                event.id,
                event.event_type.clone(),
                event.payload_hash.clone(),
            )
        })
        .collect();
    let input_hash = snapshot_hash(&evidence);
    let id = CaptureBatchId::new();
    let inserted = sqlx::query_as!(
        BatchRow,
        r#"
        insert into capture_batches
            (id, tenant_id, session_id, scope_id, workspace_id, project_id,
             principal_id, input_hash, event_count)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        on conflict (tenant_id, session_id, input_hash)
            where source_kind = 'session'
        do nothing
        returning id, tenant_id, source_kind, session_id, import_job_id,
                  scope_id, workspace_id, project_id,
                  principal_id, input_hash, event_count, state, extractor_method,
                  model_version, attempts, candidate_count, error_code,
                  created_at, started_at, completed_at, updated_at
        "#,
        id.as_uuid(),
        session.tenant_id.as_uuid(),
        session.id.as_uuid(),
        session.scope_id.as_uuid(),
        session.workspace_id.as_uuid(),
        session.project_id.map(|value| value.as_uuid()) as Option<Uuid>,
        session.principal_id,
        input_hash,
        events.len() as i32,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    if let Some(row) = inserted {
        if !events.is_empty() {
            let ids: Vec<Uuid> = events.iter().map(|event| event.id).collect();
            let ordinals: Vec<i32> = (1..=events.len() as i32).collect();
            sqlx::query!(
                r#"
                insert into capture_batch_events
                    (tenant_id, batch_id, session_id, event_id, ordinal)
                select $1, $2, $3, frozen.event_id, frozen.ordinal
                from unnest($4::uuid[], $5::int[]) as frozen(event_id, ordinal)
                "#,
                session.tenant_id.as_uuid(),
                id.as_uuid(),
                session.id.as_uuid(),
                &ids,
                &ordinals,
            )
            .execute(&mut *conn)
            .await
            .map_err(storage_error)?;
        }
        metrics::counter!(CAPTURE_MUTATIONS_TOTAL, "operation" => "batch_created").increment(1);
        return Ok(FrozenBatch {
            batch: row.try_into()?,
            created: true,
        });
    }
    let batch = by_snapshot(&mut *conn, session.tenant_id, session.id, &input_hash)
        .await?
        .ok_or_else(|| Error::Internal {
            message: "capture snapshot conflict committed without a readable batch".to_owned(),
        })?;
    Ok(FrozenBatch {
        batch,
        created: false,
    })
}

async fn by_snapshot(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    session_id: SessionId,
    input_hash: &str,
) -> Result<Option<CaptureBatch>> {
    let row = sqlx::query_as!(
        BatchRow,
        r#"
        select id, tenant_id, source_kind, session_id, import_job_id,
               scope_id, workspace_id, project_id,
               principal_id, input_hash, event_count, state, extractor_method,
               model_version, attempts, candidate_count, error_code,
               created_at, started_at, completed_at, updated_at
        from capture_batches
        where tenant_id = $1 and session_id = $2 and input_hash = $3
        "#,
        tenant_id.as_uuid(),
        session_id.as_uuid(),
        input_hash,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Reads one batch.
pub async fn get_batch(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: CaptureBatchId,
) -> Result<Option<CaptureBatch>> {
    let row = sqlx::query_as!(
        BatchRow,
        r#"
        select id, tenant_id, source_kind, session_id, import_job_id,
               scope_id, workspace_id, project_id,
               principal_id, input_hash, event_count, state, extractor_method,
               model_version, attempts, candidate_count, error_code,
               created_at, started_at, completed_at, updated_at
        from capture_batches where tenant_id = $1 and id = $2
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Lists bounded batch candidates for per-row PDP decisions.
pub async fn list_batches(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    filter: &BatchFilter,
) -> Result<Vec<CaptureBatch>> {
    let rows = sqlx::query_as!(
        BatchRow,
        r#"
        select id, tenant_id, source_kind, session_id, import_job_id,
               scope_id, workspace_id, project_id,
               principal_id, input_hash, event_count, state, extractor_method,
               model_version, attempts, candidate_count, error_code,
               created_at, started_at, completed_at, updated_at
        from capture_batches
        where tenant_id = $1
          and ($2::uuid is null or session_id = $2)
          and ($3::uuid is null or project_id = $3)
          and ($4::text is null or state = $4)
          and ($5::timestamptz is null
               or created_at < $5
               or (created_at = $5 and id < $6))
        order by created_at desc, id desc
        limit $7
        "#,
        tenant_id.as_uuid(),
        filter.session_id.map(|value| value.as_uuid()) as Option<Uuid>,
        filter.project_id.map(|value| value.as_uuid()) as Option<Uuid>,
        filter.state.map(CaptureBatchState::as_str) as Option<&str>,
        filter.after.map(|value| value.created_at) as Option<DateTime<Utc>>,
        filter.after.map(|value| value.id.as_uuid()) as Option<Uuid>,
        CAPTURE_SCAN_LIMIT,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Claims the oldest pending (or expired-running) batch for one tenant.
pub async fn claim_batch(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    lease_owner: &str,
    lease_seconds: i64,
) -> Result<Option<CaptureBatch>> {
    if lease_owner.trim().is_empty() || lease_owner.chars().count() > 255 {
        return Err(Error::Invalid {
            message: "capture lease owner must be non-blank and at most 255 characters".to_owned(),
        });
    }
    let row = sqlx::query_as!(
        BatchRow,
        r#"
        with candidate as (
            select id
            from capture_batches
            where tenant_id = $1
              and source_kind = 'session'
              and attempts < $2
              and (state = 'pending'
                   or (state = 'running' and lease_expires_at <= now()))
            order by created_at, id
            for update skip locked
            limit 1
        )
        update capture_batches batch
           set state = 'running', attempts = attempts + 1,
               lease_owner = $3,
               lease_expires_at = now() + make_interval(secs => $4::double precision),
               started_at = coalesce(started_at, now()), updated_at = now(),
               error_code = null
          from candidate
         where batch.tenant_id = $1 and batch.id = candidate.id
        returning batch.id, batch.tenant_id, batch.source_kind, batch.session_id,
                  batch.import_job_id, batch.scope_id, batch.workspace_id,
                  batch.project_id, batch.principal_id,
                  batch.input_hash, batch.event_count, batch.state,
                  batch.extractor_method, batch.model_version, batch.attempts,
                  batch.candidate_count, batch.error_code, batch.created_at,
                  batch.started_at, batch.completed_at, batch.updated_at
        "#,
        tenant_id.as_uuid(),
        MAX_CAPTURE_ATTEMPTS,
        lease_owner,
        lease_seconds.clamp(1, 3_600) as f64,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Loads the exact frozen events of one claimed batch.
pub async fn frozen_events(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    batch_id: CaptureBatchId,
) -> Result<Vec<FrozenEvent>> {
    let rows = sqlx::query!(
        r#"
        select event.id, event.event_type, event.payload, event.occurred_at,
               event.redactions, frozen.ordinal
        from capture_batch_events frozen
        join session_events event
          on event.tenant_id = frozen.tenant_id and event.id = frozen.event_id
        where frozen.tenant_id = $1 and frozen.batch_id = $2
        order by frozen.ordinal
        "#,
        tenant_id.as_uuid(),
        batch_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter()
        .map(|row| {
            Ok(FrozenEvent {
                id: SessionEventId::from_uuid(row.id),
                event_type: stored(&row.event_type)?,
                payload: row.payload,
                occurred_at: row.occurred_at,
                redactions: row.redactions,
                ordinal: row.ordinal,
            })
        })
        .collect()
}

/// Replaces no prior output: it inserts a claimed batch's candidate set once
/// and marks the job completed atomically.
pub async fn complete_batch(
    conn: &mut PgConnection,
    batch: &CaptureBatch,
    lease_owner: &str,
    method: &str,
    model_version: &str,
    candidates: &[NewCaptureCandidate],
) -> Result<CaptureBatch> {
    if batch.source_kind != CaptureSourceKind::Session
        || batch.session_id.is_none()
        || batch.import_job_id.is_some()
    {
        return Err(Error::Invalid {
            message: "the session extractor can complete only a session-sourced capture batch"
                .to_owned(),
        });
    }
    let session_id = batch.session_id.expect("session source checked above");
    for candidate in candidates {
        validate_knowledge_principal(
            candidate.proposed_owner_principal_id.as_deref(),
            "candidate owner",
        )?;
        validate_knowledge_revision_content(&candidate.content)?;
        validate_content_hash(&candidate.content_hash)?;
        if candidate.content_hash != crate::knowledge::revision_content_hash(&candidate.content) {
            return Err(Error::Invalid {
                message: "candidate content hash does not match its semantic content".to_owned(),
            });
        }
        if candidate.source_event_ids.is_empty() {
            return Err(Error::Invalid {
                message: "a capture candidate requires at least one frozen source event".to_owned(),
            });
        }
        if normalise_knowledge_tags(&candidate.content.tags)? != candidate.content.tags {
            return Err(Error::Invalid {
                message: "capture candidate tags must be lower-case, sorted and unique".to_owned(),
            });
        }
    }
    for candidate in candidates {
        let content = &candidate.content;
        sqlx::query!(
            r#"
            insert into capture_candidates
                (id, tenant_id, batch_id, session_id, ordinal,
                 proposed_scope_id, proposed_project_id,
                 proposed_owner_principal_id, knowledge_type, origin,
                 title, body_markdown, summary, tags, sensitivity,
                 confidence_permille, valid_from, valid_to, stale_after,
                 verification_metadata, metadata, content_hash)
            values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                    $13, $14, $15, $16, $17, $18, $19, $20, $21, $22)
            "#,
            candidate.id.as_uuid(),
            batch.tenant_id.as_uuid(),
            batch.id.as_uuid(),
            session_id.as_uuid(),
            candidate.ordinal,
            candidate.proposed_scope_id.as_uuid(),
            candidate.proposed_project_id.map(|value| value.as_uuid()) as Option<Uuid>,
            candidate.proposed_owner_principal_id.as_deref() as Option<&str>,
            candidate.knowledge_type.as_str(),
            candidate.origin.as_str(),
            content.title,
            content.body_markdown,
            content.summary,
            &content.tags,
            content.sensitivity.as_str(),
            content.confidence_permille,
            content.valid_from,
            content.valid_to,
            content.stale_after,
            content.verification_metadata,
            content.metadata,
            candidate.content_hash,
        )
        .execute(&mut *conn)
        .await
        .map_err(storage_error)?;

        let event_ids: Vec<Uuid> = candidate
            .source_event_ids
            .iter()
            .map(|value| value.as_uuid())
            .collect();
        let ordinals: Vec<i32> = (1..=event_ids.len() as i32).collect();
        sqlx::query!(
            r#"
            insert into capture_candidate_events
                (tenant_id, candidate_id, batch_id, event_id, ordinal)
            select $1, $2, $3, source.event_id, source.ordinal
            from unnest($4::uuid[], $5::int[]) as source(event_id, ordinal)
            "#,
            batch.tenant_id.as_uuid(),
            candidate.id.as_uuid(),
            batch.id.as_uuid(),
            &event_ids,
            &ordinals,
        )
        .execute(&mut *conn)
        .await
        .map_err(storage_error)?;
        for matched in &candidate.matches {
            sqlx::query!(
                r#"
                insert into capture_candidate_matches
                    (tenant_id, candidate_id, knowledge_item_id,
                     knowledge_revision_id, match_kind, similarity_permille,
                     reason_code)
                values ($1, $2, $3, $4, $5, $6, $7)
                "#,
                batch.tenant_id.as_uuid(),
                candidate.id.as_uuid(),
                matched.knowledge_item_id.as_uuid(),
                matched.knowledge_revision_id.as_uuid(),
                matched.kind.as_str(),
                matched.similarity_permille,
                matched.reason_code,
            )
            .execute(&mut *conn)
            .await
            .map_err(storage_error)?;
        }
    }
    let row = sqlx::query_as!(
        BatchRow,
        r#"
        update capture_batches
           set state = 'completed', extractor_method = $4, model_version = $5,
               candidate_count = $6, lease_owner = null,
               lease_expires_at = null, completed_at = now(), updated_at = now()
         where tenant_id = $1 and id = $2 and state = 'running'
           and lease_owner = $3
        returning id, tenant_id, source_kind, session_id, import_job_id,
                  scope_id, workspace_id, project_id,
                  principal_id, input_hash, event_count, state, extractor_method,
                  model_version, attempts, candidate_count, error_code,
                  created_at, started_at, completed_at, updated_at
        "#,
        batch.tenant_id.as_uuid(),
        batch.id.as_uuid(),
        lease_owner,
        method,
        model_version,
        candidates.len() as i32,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?
    .ok_or_else(|| Error::Conflict {
        message: format!(
            "capture batch {} lease is no longer owned by {lease_owner}",
            batch.id
        ),
    })?;
    metrics::counter!(CAPTURE_MUTATIONS_TOTAL, "operation" => "batch_completed").increment(1);
    row.try_into()
}

/// Releases a failed attempt for retry or terminally fails it at the attempt
/// bound. The stored error is a stable code, never dependency content.
pub async fn fail_batch(
    conn: &mut PgConnection,
    batch: &CaptureBatch,
    lease_owner: &str,
    error_code: &str,
) -> Result<CaptureBatch> {
    let terminal = batch.attempts >= MAX_CAPTURE_ATTEMPTS;
    let row = sqlx::query_as!(
        BatchRow,
        r#"
        update capture_batches
           set state = case when $4 then 'failed' else 'pending' end,
               lease_owner = null, lease_expires_at = null,
               error_code = $5,
               completed_at = case when $4 then now() else null end,
               updated_at = now()
         where tenant_id = $1 and id = $2 and state = 'running'
           and lease_owner = $3
        returning id, tenant_id, source_kind, session_id, import_job_id,
                  scope_id, workspace_id, project_id,
                  principal_id, input_hash, event_count, state, extractor_method,
                  model_version, attempts, candidate_count, error_code,
                  created_at, started_at, completed_at, updated_at
        "#,
        batch.tenant_id.as_uuid(),
        batch.id.as_uuid(),
        lease_owner,
        terminal,
        error_code,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?
    .ok_or_else(|| Error::Conflict {
        message: format!(
            "capture batch {} lease is no longer owned by {lease_owner}",
            batch.id
        ),
    })?;
    metrics::counter!(CAPTURE_MUTATIONS_TOTAL, "operation" => if terminal {
        "batch_failed"
    } else {
        "batch_retried"
    })
    .increment(1);
    row.try_into()
}

/// Reads one candidate with its exact evidence and visible match hints.
pub async fn get_candidate(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    id: CaptureCandidateId,
) -> Result<Option<CaptureCandidate>> {
    let row = sqlx::query_as!(
        CandidateRow,
        r#"
        select id, tenant_id, batch_id, source_kind, session_id, import_job_id,
               ordinal, proposed_scope_id,
               proposed_project_id, proposed_owner_principal_id, knowledge_type,
               origin, title, body_markdown, summary, tags, sensitivity,
               confidence_permille, valid_from, valid_to, stale_after,
               verification_metadata, metadata, content_hash, state,
               resulting_change_id, resulting_outcome,
               resulting_knowledge_item_id, resulting_revision_id, decided_by,
               decision_reason, decided_at, content_erased, created_at
        from capture_candidates where tenant_id = $1 and id = $2
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    let Some(row) = row else { return Ok(None) };
    hydrate_candidate(conn, candidate_without_links(row)?)
        .await
        .map(Some)
}

async fn hydrate_candidate(
    conn: &mut PgConnection,
    mut candidate: CaptureCandidate,
) -> Result<CaptureCandidate> {
    let event_ids = sqlx::query_scalar!(
        r#"
        select event_id
        from capture_candidate_events
        where tenant_id = $1 and candidate_id = $2
        order by ordinal
        "#,
        candidate.tenant_id.as_uuid(),
        candidate.id.as_uuid(),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    candidate.source_event_ids = event_ids
        .into_iter()
        .map(SessionEventId::from_uuid)
        .collect();
    let artifact_ids = sqlx::query_scalar!(
        r#"
        select artifact_id
        from capture_candidate_import_artifacts
        where tenant_id = $1 and candidate_id = $2
        order by ordinal
        "#,
        candidate.tenant_id.as_uuid(),
        candidate.id.as_uuid(),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    candidate.source_artifact_ids = artifact_ids
        .into_iter()
        .map(ImportArtifactId::from_uuid)
        .collect();
    let matches = sqlx::query!(
        r#"
        select knowledge_item_id, knowledge_revision_id, match_kind,
               similarity_permille, reason_code
        from capture_candidate_matches
        where tenant_id = $1 and candidate_id = $2
        order by similarity_permille desc, knowledge_item_id
        "#,
        candidate.tenant_id.as_uuid(),
        candidate.id.as_uuid(),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    candidate.matches = matches
        .into_iter()
        .map(|row| {
            Ok(CaptureMatch {
                knowledge_item_id: KnowledgeItemId::from_uuid(row.knowledge_item_id),
                knowledge_revision_id: KnowledgeRevisionId::from_uuid(row.knowledge_revision_id),
                kind: stored(&row.match_kind)?,
                similarity_permille: row.similarity_permille,
                reason_code: row.reason_code,
            })
        })
        .collect::<Result<_>>()?;
    Ok(candidate)
}

/// Lists bounded candidate rows for per-session PDP filtering.
pub async fn list_candidates(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    filter: &CandidateFilter,
) -> Result<Vec<CaptureCandidate>> {
    let rows = sqlx::query_as!(
        CandidateRow,
        r#"
        select id, tenant_id, batch_id, source_kind, session_id, import_job_id,
               ordinal, proposed_scope_id,
               proposed_project_id, proposed_owner_principal_id, knowledge_type,
               origin, title, body_markdown, summary, tags, sensitivity,
               confidence_permille, valid_from, valid_to, stale_after,
               verification_metadata, metadata, content_hash, state,
               resulting_change_id, resulting_outcome,
               resulting_knowledge_item_id, resulting_revision_id, decided_by,
               decision_reason, decided_at, content_erased, created_at
        from capture_candidates
        where tenant_id = $1
          and ($2::uuid is null or batch_id = $2)
          and ($3::uuid is null or session_id = $3)
          and ($4::uuid is null or proposed_project_id = $4)
          and ($5::text is null or state = $5)
          and ($6::timestamptz is null
               or created_at < $6
               or (created_at = $6 and id < $7))
        order by created_at desc, id desc
        limit $8
        "#,
        tenant_id.as_uuid(),
        filter.batch_id.map(|value| value.as_uuid()) as Option<Uuid>,
        filter.session_id.map(|value| value.as_uuid()) as Option<Uuid>,
        filter.project_id.map(|value| value.as_uuid()) as Option<Uuid>,
        filter.state.map(CaptureCandidateState::as_str) as Option<&str>,
        filter.after.map(|value| value.created_at) as Option<DateTime<Utc>>,
        filter.after.map(|value| value.id.as_uuid()) as Option<Uuid>,
        CAPTURE_SCAN_LIMIT,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    let mut candidates = Vec::with_capacity(rows.len());
    for row in rows {
        candidates.push(hydrate_candidate(conn, candidate_without_links(row)?).await?);
    }
    Ok(candidates)
}

/// Opens a durable candidate decision, replaying the same request and
/// refusing a changed key/payload.
pub async fn begin_decision(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    candidate_id: CaptureCandidateId,
    action: CaptureDecisionAction,
    actor_subject: &str,
    idempotency_key: &str,
    payload: &Value,
) -> Result<CaptureCandidateDecision> {
    let canonical = synveda_types::json::canonicalise(payload);
    let payload_hash = blake3::hash(canonical.to_string().as_bytes())
        .to_hex()
        .to_string();
    let request_hash = blake3::hash(format!("{}\0{}", action.as_str(), canonical).as_bytes())
        .to_hex()
        .to_string();
    let id = CaptureCandidateDecisionId::new();
    let inserted = sqlx::query!(
        r#"
        insert into capture_candidate_decisions
            (id, tenant_id, candidate_id, action, actor_subject,
             idempotency_key, request_hash, payload, payload_hash)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        on conflict (tenant_id, candidate_id) do nothing
        returning id
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
        candidate_id.as_uuid(),
        action.as_str(),
        actor_subject,
        idempotency_key,
        request_hash,
        canonical,
        payload_hash,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    let decision = get_decision(&mut *conn, tenant_id, candidate_id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: "capture decision insert produced no readable row".to_owned(),
        })?;
    if inserted.is_none()
        && (decision.actor_subject != actor_subject
            || decision.idempotency_key != idempotency_key
            || decision.request_hash != request_hash)
    {
        return Err(Error::Conflict {
            message: format!(
                "capture candidate {candidate_id} already has a different decision request"
            ),
        });
    }
    Ok(decision)
}

/// Reads one candidate's decision intent/result.
pub async fn get_decision(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    candidate_id: CaptureCandidateId,
) -> Result<Option<CaptureCandidateDecision>> {
    let row = sqlx::query!(
        r#"
        select id, candidate_id, action, state, actor_subject, idempotency_key,
               request_hash, payload, payload_hash, resulting_change_id,
               resulting_outcome, resulting_knowledge_item_id,
               resulting_revision_id, error_code, created_at, completed_at
        from capture_candidate_decisions
        where tenant_id = $1 and candidate_id = $2
        "#,
        tenant_id.as_uuid(),
        candidate_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(|row| {
        Ok(CaptureCandidateDecision {
            id: CaptureCandidateDecisionId::from_uuid(row.id),
            candidate_id: CaptureCandidateId::from_uuid(row.candidate_id),
            action: stored(&row.action)?,
            state: stored(&row.state)?,
            actor_subject: row.actor_subject,
            idempotency_key: row.idempotency_key,
            request_hash: row.request_hash,
            payload: row.payload,
            payload_hash: row.payload_hash,
            resulting_change_id: row.resulting_change_id.map(ProposalId::from_uuid),
            resulting_outcome: row.resulting_outcome.as_deref().map(stored).transpose()?,
            resulting_knowledge_item_id: row
                .resulting_knowledge_item_id
                .map(KnowledgeItemId::from_uuid),
            resulting_revision_id: row
                .resulting_revision_id
                .map(KnowledgeRevisionId::from_uuid),
            error_code: row.error_code,
            created_at: row.created_at,
            completed_at: row.completed_at,
        })
    })
    .transpose()
}

/// Result of atomically finalising a candidate decision.
pub struct CompletedDecision {
    /// Candidate after the terminal transition (or the already-committed
    /// result observed by a concurrent retry).
    pub candidate: CaptureCandidate,
    /// `true` only for the request that won the `running -> succeeded`
    /// transition. Callers use this to emit one audit event under a retry
    /// race rather than one event per waiter.
    pub completed_now: bool,
}

/// Finalises a successful Knowledge-backed decision and candidate state.
///
/// A concurrent retry can observe the already-committed terminal result; in
/// that case [`CompletedDecision::completed_now`] is false and the caller
/// must not emit the transition's audit event again.
pub async fn complete_decision(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    candidate_id: CaptureCandidateId,
    candidate_state: CaptureCandidateState,
    actor_subject: &str,
    reason: Option<&str>,
    result: Option<&synveda_types::knowledge::KnowledgeMutationResult>,
) -> Result<CompletedDecision> {
    if candidate_state == CaptureCandidateState::Pending {
        return Err(Error::Invalid {
            message: "a completed capture decision cannot remain pending".to_owned(),
        });
    }
    let (change, outcome, item, revision) = result.map_or((None, None, None, None), |result| {
        (
            Some(result.change_id.as_uuid()),
            Some(result.outcome.as_str()),
            result.knowledge_item_id.map(|value| value.as_uuid()),
            result.revision_id.map(|value| value.as_uuid()),
        )
    });
    let transition = sqlx::query!(
        r#"
        update capture_candidate_decisions
           set state = 'succeeded', resulting_change_id = $3,
               resulting_outcome = $4, resulting_knowledge_item_id = $5,
               resulting_revision_id = $6, completed_at = now()
         where tenant_id = $1 and candidate_id = $2 and state = 'running'
        "#,
        tenant_id.as_uuid(),
        candidate_id.as_uuid(),
        change,
        outcome,
        item,
        revision,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;
    if transition.rows_affected() == 0 {
        let decision = get_decision(&mut *conn, tenant_id, candidate_id)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity: format!("capture decision for candidate {candidate_id}"),
            })?;
        if decision.state != CaptureDecisionState::Succeeded {
            return Err(Error::Conflict {
                message: format!(
                    "capture candidate {candidate_id} decision is {}",
                    decision.state
                ),
            });
        }
        let candidate = get_candidate(conn, tenant_id, candidate_id)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity: format!("capture candidate {candidate_id}"),
            })?;
        return Ok(CompletedDecision {
            candidate,
            completed_now: false,
        });
    }
    let candidate_transition = sqlx::query!(
        r#"
        update capture_candidates
           set state = $3, resulting_change_id = $4, resulting_outcome = $5,
               resulting_knowledge_item_id = $6, resulting_revision_id = $7,
               decided_by = $8, decision_reason = $9, decided_at = now()
         where tenant_id = $1 and id = $2 and state = 'pending'
        "#,
        tenant_id.as_uuid(),
        candidate_id.as_uuid(),
        candidate_state.as_str(),
        change,
        outcome,
        item,
        revision,
        actor_subject,
        reason,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;
    if candidate_transition.rows_affected() != 1 {
        return Err(Error::Conflict {
            message: format!(
                "capture candidate {candidate_id} changed while its decision completed"
            ),
        });
    }
    let candidate = get_candidate(conn, tenant_id, candidate_id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("capture candidate {candidate_id}"),
        })?;
    Ok(CompletedDecision {
        candidate,
        completed_now: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_hash_is_ordered_and_unambiguous() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let one = snapshot_hash(&[(a, "message.user".to_owned(), "a".repeat(64))]);
        let replay = snapshot_hash(&[(a, "message.user".to_owned(), "a".repeat(64))]);
        let reordered = snapshot_hash(&[
            (b, "message.user".to_owned(), "b".repeat(64)),
            (a, "message.user".to_owned(), "a".repeat(64)),
        ]);
        let ordered = snapshot_hash(&[
            (a, "message.user".to_owned(), "a".repeat(64)),
            (b, "message.user".to_owned(), "b".repeat(64)),
        ]);
        assert_eq!(one, replay);
        assert_ne!(ordered, reordered);
        assert_eq!(one.len(), 64);
    }
}
