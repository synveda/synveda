//! Explainable context candidates, selections and feedback (CPR-20,
//! ADR-0084).
//!
//! These queries are persistence primitives below authorisation. Callers must
//! decide the owning session and every exact Knowledge reference before
//! inserting or exposing a row. Forced RLS remains the tenant backstop.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use synveda_types::configuration::ConfigurationContextChannel;
use synveda_types::knowledge::{KnowledgeLifecycleState, KnowledgeRelationType};
use synveda_types::{
    CaptureCandidateId, ContextCandidate, ContextCandidateId, ContextFeedback, ContextFeedbackId,
    ContextFeedbackType, ContextGraphDirection, ContextGraphStep, ContextReasonCode, ContextRunId,
    ContextSelection, ContextSelectionId, Error, KnowledgeItemId, KnowledgeRelationId,
    KnowledgeRevisionId, Result, ScopeId, SessionId, TenantId,
};
use uuid::Uuid;

/// Low-level planner persistence operations.
pub const CONTEXT_MUTATIONS_TOTAL: &str = "synveda_context_mutations_total";

/// Candidate fields supplied after an exact Knowledge decision.
#[derive(Debug, Clone)]
pub struct NewContextCandidate {
    /// Candidate id.
    pub id: ContextCandidateId,
    /// Context run.
    pub context_run_id: ContextRunId,
    /// Stable position in the bounded pool.
    pub ordinal: i32,
    /// Governed content channel.
    pub channel: ConfigurationContextChannel,
    /// Stable item, omitted by hashes-only retention.
    pub knowledge_item_id: Option<KnowledgeItemId>,
    /// Immutable revision, omitted by hashes-only retention.
    pub knowledge_revision_id: Option<KnowledgeRevisionId>,
    /// Unreviewed proposal, omitted for Knowledge and hashes-only retention.
    pub capture_candidate_id: Option<CaptureCandidateId>,
    /// Canonical revision digest.
    pub content_hash: String,
    /// Governed scope, omitted with Knowledge addresses.
    pub scope_id: Option<ScopeId>,
    /// Observed lifecycle, omitted with Knowledge addresses.
    pub lifecycle_state: Option<KnowledgeLifecycleState>,
    /// Integer lexical contribution.
    pub keyword_score_micros: i32,
    /// Integer semantic contribution.
    pub semantic_score_micros: i32,
    /// Ordinary anchor score from which a graph path began.
    pub anchor_score_micros: i32,
    /// Relationship contribution on the retained best path.
    pub edge_weight_micros: i32,
    /// Hop penalty on the retained best path.
    pub hop_penalty_micros: i32,
    /// Integer freshness contribution.
    pub freshness_score_micros: i32,
    /// Integer explicit-pin contribution.
    pub pin_score_micros: i32,
    /// Integer current-state contribution.
    pub current_state_score_micros: i32,
    /// Final deterministic score.
    pub final_score_micros: i32,
    /// Consideration reasons.
    pub reason_codes: Vec<ContextReasonCode>,
    /// Visible exclusion reason.
    pub exclusion_reason: Option<ContextReasonCode>,
}

/// Selection fields supplied after budgeted assembly.
#[derive(Debug, Clone)]
pub struct NewContextSelection {
    /// Selection id.
    pub id: ContextSelectionId,
    /// Context run.
    pub context_run_id: ContextRunId,
    /// Exact candidate selected.
    pub context_candidate_id: ContextCandidateId,
    /// One-based rank.
    pub rank: i32,
    /// Governed content channel.
    pub channel: ConfigurationContextChannel,
    /// Stable item, omitted by hashes-only retention.
    pub knowledge_item_id: Option<KnowledgeItemId>,
    /// Immutable revision, omitted by hashes-only retention.
    pub knowledge_revision_id: Option<KnowledgeRevisionId>,
    /// Unreviewed proposal, omitted for Knowledge and hashes-only retention.
    pub capture_candidate_id: Option<CaptureCandidateId>,
    /// Canonical revision digest.
    pub content_hash: String,
    /// Estimated tokens charged.
    pub token_count: i32,
    /// Selection reasons.
    pub reason_codes: Vec<ContextReasonCode>,
}

/// Fields for one immutable visible graph path step.
#[derive(Debug, Clone)]
pub struct NewContextGraphStep {
    /// Context run.
    pub context_run_id: ContextRunId,
    /// Candidate reached by the complete path.
    pub context_candidate_id: ContextCandidateId,
    /// Zero-based path position.
    pub ordinal: i32,
    /// One-based hop.
    pub hop: u8,
    /// Exact relation, omitted by hashes-only retention.
    pub relation_id: Option<KnowledgeRelationId>,
    /// Content-free relation evidence hash.
    pub relation_hash: String,
    /// Relation vocabulary.
    pub relation_type: KnowledgeRelationType,
    /// Traversal direction.
    pub direction: ContextGraphDirection,
    /// Exact start item, omitted by hashes-only retention.
    pub from_item_id: Option<KnowledgeItemId>,
    /// Exact start revision, omitted by hashes-only retention.
    pub from_revision_id: Option<KnowledgeRevisionId>,
    /// Exact reached item, omitted by hashes-only retention.
    pub to_item_id: Option<KnowledgeItemId>,
    /// Exact reached revision, omitted by hashes-only retention.
    pub to_revision_id: Option<KnowledgeRevisionId>,
    /// Exact source revision asserting the relation, omitted by hashes-only
    /// retention.
    pub asserting_revision_id: Option<KnowledgeRevisionId>,
    /// Starting revision content hash.
    pub from_content_hash: String,
    /// Reached revision content hash.
    pub to_content_hash: String,
    /// Supporting contribution or zero for a contradiction warning.
    pub edge_weight_micros: i32,
    /// Whether the step contributes supporting evidence.
    pub supporting: bool,
}

/// Feedback fields supplied after session and revision decisions.
#[derive(Debug, Clone)]
pub struct NewContextFeedback {
    /// Feedback id.
    pub id: ContextFeedbackId,
    /// Context run.
    pub context_run_id: ContextRunId,
    /// Exact selection.
    pub context_selection_id: ContextSelectionId,
    /// Exact immutable revision.
    pub knowledge_revision_id: KnowledgeRevisionId,
    /// Feedback vocabulary.
    pub feedback_type: ContextFeedbackType,
    /// Authenticated subject.
    pub principal_id: String,
    /// Request idempotency key.
    pub idempotency_key: String,
}

/// A usage candidate includes its session so the gateway can independently
/// authorise the runtime record before exposing the Knowledge selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeUsage {
    /// Context run that selected the revision.
    pub context_run_id: ContextRunId,
    /// Session that owns the run.
    pub session_id: SessionId,
    /// Exact immutable revision.
    pub knowledge_revision_id: KnowledgeRevisionId,
    /// Selection time.
    pub selected_at: DateTime<Utc>,
    /// Selection id, the stable keyset tie-breaker.
    pub selection_id: ContextSelectionId,
    /// Visible reason codes.
    pub reason_codes: Vec<ContextReasonCode>,
}

#[derive(Debug)]
struct CandidateRow {
    id: Uuid,
    tenant_id: Uuid,
    context_run_id: Uuid,
    ordinal: i32,
    channel: String,
    knowledge_item_id: Option<Uuid>,
    knowledge_revision_id: Option<Uuid>,
    capture_candidate_id: Option<Uuid>,
    content_hash: String,
    scope_id: Option<Uuid>,
    lifecycle_state: Option<String>,
    keyword_score_micros: i32,
    semantic_score_micros: i32,
    anchor_score_micros: i32,
    edge_weight_micros: i32,
    hop_penalty_micros: i32,
    freshness_score_micros: i32,
    pin_score_micros: i32,
    current_state_score_micros: i32,
    final_score_micros: i32,
    reason_codes: Vec<String>,
    exclusion_reason: Option<String>,
    created_at: DateTime<Utc>,
}

#[derive(Debug)]
struct SelectionRow {
    id: Uuid,
    tenant_id: Uuid,
    context_run_id: Uuid,
    context_candidate_id: Uuid,
    rank: i32,
    channel: String,
    knowledge_item_id: Option<Uuid>,
    knowledge_revision_id: Option<Uuid>,
    capture_candidate_id: Option<Uuid>,
    content_hash: String,
    token_count: i32,
    reason_codes: Vec<String>,
    created_at: DateTime<Utc>,
}

#[derive(Debug)]
struct GraphStepRow {
    tenant_id: Uuid,
    context_run_id: Uuid,
    context_candidate_id: Uuid,
    ordinal: i32,
    hop: i16,
    relation_id: Option<Uuid>,
    relation_hash: String,
    relation_type: String,
    direction: String,
    from_item_id: Option<Uuid>,
    from_revision_id: Option<Uuid>,
    to_item_id: Option<Uuid>,
    to_revision_id: Option<Uuid>,
    asserting_revision_id: Option<Uuid>,
    from_content_hash: String,
    to_content_hash: String,
    edge_weight_micros: i32,
    supporting: bool,
    created_at: DateTime<Utc>,
}

#[derive(Debug)]
struct FeedbackRow {
    id: Uuid,
    tenant_id: Uuid,
    context_run_id: Uuid,
    context_selection_id: Uuid,
    knowledge_revision_id: Uuid,
    feedback_type: String,
    principal_id: String,
    created_at: DateTime<Utc>,
}

fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        match db.code().as_deref() {
            Some("23505" | "40001") => {
                return Error::Conflict {
                    message: db.to_string(),
                };
            }
            Some("23503" | "23514" | "22001") => {
                return Error::Invalid {
                    message: db.to_string(),
                };
            }
            Some("42501") => return crate::rls::backstop_error(db),
            _ => {}
        }
    }
    Error::Storage {
        message: err.to_string(),
    }
}

fn reasons(values: Vec<String>) -> Result<Vec<ContextReasonCode>> {
    values
        .into_iter()
        .map(|value| value.parse())
        .collect::<Result<Vec<_>>>()
        .map_err(|error| Error::Internal {
            message: format!("stored context reason outside vocabulary: {error}"),
        })
}

impl TryFrom<CandidateRow> for ContextCandidate {
    type Error = Error;

    fn try_from(row: CandidateRow) -> Result<Self> {
        Ok(Self {
            id: ContextCandidateId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            context_run_id: ContextRunId::from_uuid(row.context_run_id),
            ordinal: row.ordinal,
            channel: row.channel.parse().map_err(|error| Error::Internal {
                message: format!("stored context channel outside vocabulary: {error}"),
            })?,
            knowledge_item_id: row.knowledge_item_id.map(KnowledgeItemId::from_uuid),
            knowledge_revision_id: row
                .knowledge_revision_id
                .map(KnowledgeRevisionId::from_uuid),
            capture_candidate_id: row.capture_candidate_id.map(CaptureCandidateId::from_uuid),
            content_hash: row.content_hash,
            scope_id: row.scope_id.map(ScopeId::from_uuid),
            lifecycle_state: row
                .lifecycle_state
                .map(|value| value.parse())
                .transpose()
                .map_err(|error| Error::Internal {
                    message: format!("stored Knowledge lifecycle outside vocabulary: {error}"),
                })?,
            keyword_score_micros: row.keyword_score_micros,
            semantic_score_micros: row.semantic_score_micros,
            anchor_score_micros: row.anchor_score_micros,
            edge_weight_micros: row.edge_weight_micros,
            hop_penalty_micros: row.hop_penalty_micros,
            freshness_score_micros: row.freshness_score_micros,
            pin_score_micros: row.pin_score_micros,
            current_state_score_micros: row.current_state_score_micros,
            final_score_micros: row.final_score_micros,
            reason_codes: reasons(row.reason_codes)?,
            exclusion_reason: row
                .exclusion_reason
                .map(|value| value.parse())
                .transpose()
                .map_err(|error| Error::Internal {
                    message: format!("stored context exclusion outside vocabulary: {error}"),
                })?,
            created_at: row.created_at,
        })
    }
}

impl TryFrom<SelectionRow> for ContextSelection {
    type Error = Error;

    fn try_from(row: SelectionRow) -> Result<Self> {
        Ok(Self {
            id: ContextSelectionId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            context_run_id: ContextRunId::from_uuid(row.context_run_id),
            context_candidate_id: ContextCandidateId::from_uuid(row.context_candidate_id),
            rank: row.rank,
            channel: row.channel.parse().map_err(|error| Error::Internal {
                message: format!("stored context channel outside vocabulary: {error}"),
            })?,
            knowledge_item_id: row.knowledge_item_id.map(KnowledgeItemId::from_uuid),
            knowledge_revision_id: row
                .knowledge_revision_id
                .map(KnowledgeRevisionId::from_uuid),
            capture_candidate_id: row.capture_candidate_id.map(CaptureCandidateId::from_uuid),
            content_hash: row.content_hash,
            token_count: row.token_count,
            reason_codes: reasons(row.reason_codes)?,
            created_at: row.created_at,
        })
    }
}

impl TryFrom<FeedbackRow> for ContextFeedback {
    type Error = Error;

    fn try_from(row: FeedbackRow) -> Result<Self> {
        Ok(Self {
            id: ContextFeedbackId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            context_run_id: ContextRunId::from_uuid(row.context_run_id),
            context_selection_id: ContextSelectionId::from_uuid(row.context_selection_id),
            knowledge_revision_id: KnowledgeRevisionId::from_uuid(row.knowledge_revision_id),
            feedback_type: row.feedback_type.parse().map_err(|error| Error::Internal {
                message: format!("stored context feedback outside vocabulary: {error}"),
            })?,
            principal_id: row.principal_id,
            created_at: row.created_at,
        })
    }
}

impl TryFrom<GraphStepRow> for ContextGraphStep {
    type Error = Error;

    fn try_from(row: GraphStepRow) -> Result<Self> {
        Ok(Self {
            tenant_id: TenantId::from_uuid(row.tenant_id),
            context_run_id: ContextRunId::from_uuid(row.context_run_id),
            context_candidate_id: ContextCandidateId::from_uuid(row.context_candidate_id),
            ordinal: row.ordinal,
            hop: u8::try_from(row.hop).map_err(|_| Error::Internal {
                message: format!("stored context graph hop is invalid: {}", row.hop),
            })?,
            relation_id: row.relation_id.map(KnowledgeRelationId::from_uuid),
            relation_hash: row.relation_hash,
            relation_type: row.relation_type.parse().map_err(|error| Error::Internal {
                message: format!("stored Knowledge relation outside vocabulary: {error}"),
            })?,
            direction: row.direction.parse().map_err(|error| Error::Internal {
                message: format!("stored context graph direction outside vocabulary: {error}"),
            })?,
            from_item_id: row.from_item_id.map(KnowledgeItemId::from_uuid),
            from_revision_id: row.from_revision_id.map(KnowledgeRevisionId::from_uuid),
            to_item_id: row.to_item_id.map(KnowledgeItemId::from_uuid),
            to_revision_id: row.to_revision_id.map(KnowledgeRevisionId::from_uuid),
            asserting_revision_id: row
                .asserting_revision_id
                .map(KnowledgeRevisionId::from_uuid),
            from_content_hash: row.from_content_hash,
            to_content_hash: row.to_content_hash,
            edge_weight_micros: row.edge_weight_micros,
            supporting: row.supporting,
            created_at: row.created_at,
        })
    }
}

fn reason_names(values: &[ContextReasonCode]) -> Vec<String> {
    values
        .iter()
        .copied()
        .map(ContextReasonCode::as_str)
        .map(str::to_owned)
        .collect()
}

/// Appends one visible context candidate.
#[tracing::instrument(
    name = "store.context.insert_candidate",
    skip_all,
    fields(tenant.id = %tenant_id, context.run.id = %new.context_run_id, context.candidate.id = %new.id),
    err(Display)
)]
pub async fn insert_candidate(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    new: &NewContextCandidate,
) -> Result<ContextCandidate> {
    let reason_codes = reason_names(&new.reason_codes);
    let row = sqlx::query_as!(
        CandidateRow,
        r#"
        insert into context_candidates
            (id, tenant_id, context_run_id, ordinal, channel,
             knowledge_item_id, knowledge_revision_id, capture_candidate_id,
             content_hash, scope_id, lifecycle_state,
             keyword_score_micros, semantic_score_micros,
             anchor_score_micros, edge_weight_micros, hop_penalty_micros,
             freshness_score_micros, pin_score_micros,
             current_state_score_micros, final_score_micros, reason_codes,
             exclusion_reason)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
                $14, $15, $16, $17, $18, $19, $20, $21, $22)
        returning id, tenant_id, context_run_id, ordinal, channel,
                  knowledge_item_id, knowledge_revision_id,
                  capture_candidate_id, content_hash, scope_id,
                  lifecycle_state, keyword_score_micros, semantic_score_micros,
                  anchor_score_micros, edge_weight_micros, hop_penalty_micros,
                  freshness_score_micros, pin_score_micros,
                  current_state_score_micros, final_score_micros,
                  reason_codes as "reason_codes!: Vec<String>",
                  exclusion_reason, created_at
        "#,
        new.id.as_uuid(),
        tenant_id.as_uuid(),
        new.context_run_id.as_uuid(),
        new.ordinal,
        new.channel.as_str(),
        new.knowledge_item_id.map(|id| id.as_uuid()),
        new.knowledge_revision_id.map(|id| id.as_uuid()),
        new.capture_candidate_id.map(|id| id.as_uuid()),
        new.content_hash,
        new.scope_id.map(|id| id.as_uuid()),
        new.lifecycle_state.map(KnowledgeLifecycleState::as_str),
        new.keyword_score_micros,
        new.semantic_score_micros,
        new.anchor_score_micros,
        new.edge_weight_micros,
        new.hop_penalty_micros,
        new.freshness_score_micros,
        new.pin_score_micros,
        new.current_state_score_micros,
        new.final_score_micros,
        &reason_codes,
        new.exclusion_reason.map(ContextReasonCode::as_str),
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    metrics::counter!(
        CONTEXT_MUTATIONS_TOTAL,
        "aggregate" => "candidate",
        "operation" => "create"
    )
    .increment(1);
    row.try_into()
}

/// Appends one policy-visible step of a candidate's bounded graph path.
#[tracing::instrument(
    name = "store.context.insert_graph_step",
    skip_all,
    fields(
        tenant.id = %tenant_id,
        context.run.id = %new.context_run_id,
        context.candidate.id = %new.context_candidate_id,
        context.graph.hop = new.hop
    ),
    err(Display)
)]
pub async fn insert_graph_step(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    new: &NewContextGraphStep,
) -> Result<ContextGraphStep> {
    let row = sqlx::query_as!(
        GraphStepRow,
        r#"
        insert into context_graph_steps
            (tenant_id, context_run_id, context_candidate_id, ordinal, hop,
             relation_id, relation_hash, relation_type, direction,
             from_item_id, from_revision_id, to_item_id, to_revision_id,
             asserting_revision_id, from_content_hash, to_content_hash,
             edge_weight_micros, supporting)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                $13, $14, $15, $16, $17, $18)
        returning tenant_id, context_run_id, context_candidate_id, ordinal,
                  hop, relation_id, relation_hash, relation_type, direction,
                  from_item_id, from_revision_id, to_item_id, to_revision_id,
                  asserting_revision_id, from_content_hash, to_content_hash,
                  edge_weight_micros, supporting, created_at
        "#,
        tenant_id.as_uuid(),
        new.context_run_id.as_uuid(),
        new.context_candidate_id.as_uuid(),
        new.ordinal,
        i16::from(new.hop),
        new.relation_id.map(|id| id.as_uuid()),
        new.relation_hash,
        new.relation_type.as_str(),
        new.direction.as_str(),
        new.from_item_id.map(|id| id.as_uuid()),
        new.from_revision_id.map(|id| id.as_uuid()),
        new.to_item_id.map(|id| id.as_uuid()),
        new.to_revision_id.map(|id| id.as_uuid()),
        new.asserting_revision_id.map(|id| id.as_uuid()),
        new.from_content_hash,
        new.to_content_hash,
        new.edge_weight_micros,
        new.supporting,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    metrics::counter!(
        CONTEXT_MUTATIONS_TOTAL,
        "aggregate" => "graph_step",
        "operation" => "create"
    )
    .increment(1);
    row.try_into()
}

/// Appends one selected immutable revision.
#[tracing::instrument(
    name = "store.context.insert_selection",
    skip_all,
    fields(tenant.id = %tenant_id, context.run.id = %new.context_run_id, context.selection.id = %new.id),
    err(Display)
)]
pub async fn insert_selection(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    new: &NewContextSelection,
) -> Result<ContextSelection> {
    let reason_codes = reason_names(&new.reason_codes);
    let row = sqlx::query_as!(
        SelectionRow,
        r#"
        insert into context_selections
            (id, tenant_id, context_run_id, context_candidate_id, rank, channel,
             knowledge_item_id, knowledge_revision_id, capture_candidate_id,
             content_hash, token_count, reason_codes)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        returning id, tenant_id, context_run_id, context_candidate_id, rank, channel,
                  knowledge_item_id, knowledge_revision_id,
                  capture_candidate_id, content_hash, token_count,
                  reason_codes as "reason_codes!: Vec<String>", created_at
        "#,
        new.id.as_uuid(),
        tenant_id.as_uuid(),
        new.context_run_id.as_uuid(),
        new.context_candidate_id.as_uuid(),
        new.rank,
        new.channel.as_str(),
        new.knowledge_item_id.map(|id| id.as_uuid()),
        new.knowledge_revision_id.map(|id| id.as_uuid()),
        new.capture_candidate_id.map(|id| id.as_uuid()),
        new.content_hash,
        new.token_count,
        &reason_codes,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    metrics::counter!(
        CONTEXT_MUTATIONS_TOTAL,
        "aggregate" => "selection",
        "operation" => "create"
    )
    .increment(1);
    row.try_into()
}

/// Reads retained candidate detail in deterministic consideration order.
pub async fn candidates_for_run(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    run_id: ContextRunId,
) -> Result<Vec<ContextCandidate>> {
    let rows = sqlx::query_as!(
        CandidateRow,
        r#"
        select id, tenant_id, context_run_id, ordinal, channel,
               knowledge_item_id, knowledge_revision_id, capture_candidate_id,
               content_hash, scope_id, lifecycle_state,
               keyword_score_micros, semantic_score_micros,
               anchor_score_micros, edge_weight_micros, hop_penalty_micros,
               freshness_score_micros, pin_score_micros,
               current_state_score_micros, final_score_micros,
               reason_codes as "reason_codes!: Vec<String>", exclusion_reason,
               created_at
        from context_candidates
        where tenant_id = $1 and context_run_id = $2
        order by ordinal
        "#,
        tenant_id.as_uuid(),
        run_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Reads retained graph paths in candidate/path order.
pub async fn graph_steps_for_run(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    run_id: ContextRunId,
) -> Result<Vec<ContextGraphStep>> {
    let rows = sqlx::query_as!(
        GraphStepRow,
        r#"
        select tenant_id, context_run_id, context_candidate_id, ordinal, hop,
               relation_id, relation_hash, relation_type, direction,
               from_item_id, from_revision_id, to_item_id, to_revision_id,
               asserting_revision_id, from_content_hash, to_content_hash,
               edge_weight_micros, supporting, created_at
        from context_graph_steps
        where tenant_id = $1 and context_run_id = $2
        order by context_candidate_id, ordinal
        "#,
        tenant_id.as_uuid(),
        run_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Reads retained selections in delivery order.
pub async fn selections_for_run(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    run_id: ContextRunId,
) -> Result<Vec<ContextSelection>> {
    let rows = sqlx::query_as!(
        SelectionRow,
        r#"
        select id, tenant_id, context_run_id, context_candidate_id, rank, channel,
               knowledge_item_id, knowledge_revision_id, capture_candidate_id,
               content_hash, token_count,
               reason_codes as "reason_codes!: Vec<String>", created_at
        from context_selections
        where tenant_id = $1 and context_run_id = $2
        order by rank
        "#,
        tenant_id.as_uuid(),
        run_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Reads one selection, scoped to its run.
pub async fn selection(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    run_id: ContextRunId,
    selection_id: ContextSelectionId,
) -> Result<Option<ContextSelection>> {
    let row = sqlx::query_as!(
        SelectionRow,
        r#"
        select id, tenant_id, context_run_id, context_candidate_id, rank, channel,
               knowledge_item_id, knowledge_revision_id, capture_candidate_id,
               content_hash, token_count,
               reason_codes as "reason_codes!: Vec<String>", created_at
        from context_selections
        where tenant_id = $1 and context_run_id = $2 and id = $3
        "#,
        tenant_id.as_uuid(),
        run_id.as_uuid(),
        selection_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Appends one idempotent feedback assertion.
pub async fn insert_feedback(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    new: &NewContextFeedback,
) -> Result<ContextFeedback> {
    let row = sqlx::query_as!(
        FeedbackRow,
        r#"
        insert into context_feedback
            (id, tenant_id, context_run_id, context_selection_id,
             knowledge_revision_id, feedback_type, principal_id,
             idempotency_key)
        values ($1, $2, $3, $4, $5, $6, $7, $8)
        returning id, tenant_id, context_run_id, context_selection_id,
                  knowledge_revision_id, feedback_type, principal_id,
                  created_at
        "#,
        new.id.as_uuid(),
        tenant_id.as_uuid(),
        new.context_run_id.as_uuid(),
        new.context_selection_id.as_uuid(),
        new.knowledge_revision_id.as_uuid(),
        new.feedback_type.as_str(),
        new.principal_id,
        new.idempotency_key,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    metrics::counter!(
        CONTEXT_MUTATIONS_TOTAL,
        "aggregate" => "feedback",
        "operation" => "create"
    )
    .increment(1);
    row.try_into()
}

/// Resolves an idempotent feedback retry.
pub async fn feedback_by_key(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    run_id: ContextRunId,
    idempotency_key: &str,
) -> Result<Option<ContextFeedback>> {
    let row = sqlx::query_as!(
        FeedbackRow,
        r#"
        select id, tenant_id, context_run_id, context_selection_id,
               knowledge_revision_id, feedback_type, principal_id, created_at
        from context_feedback
        where tenant_id = $1 and context_run_id = $2 and idempotency_key = $3
        "#,
        tenant_id.as_uuid(),
        run_id.as_uuid(),
        idempotency_key,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Lists feedback attached to one run.
pub async fn feedback_for_run(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    run_id: ContextRunId,
) -> Result<Vec<ContextFeedback>> {
    let rows = sqlx::query_as!(
        FeedbackRow,
        r#"
        select id, tenant_id, context_run_id, context_selection_id,
               knowledge_revision_id, feedback_type, principal_id, created_at
        from context_feedback
        where tenant_id = $1 and context_run_id = $2
        order by created_at, id
        "#,
        tenant_id.as_uuid(),
        run_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Candidate page for a Knowledge item's usage history. The gateway advances
/// the cursor over denied sessions too, exactly like every governed listing.
pub async fn usage_candidates(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    item_id: KnowledgeItemId,
    before: Option<(DateTime<Utc>, ContextSelectionId)>,
    limit: i64,
) -> Result<Vec<KnowledgeUsage>> {
    let before_at = before.map(|value| value.0);
    let before_id = before.map(|value| value.1.as_uuid());
    let rows = sqlx::query!(
        r#"
        select selection.id as "selection_id!",
               selection.context_run_id as "context_run_id!",
               run.session_id as "session_id!",
               selection.knowledge_revision_id as "knowledge_revision_id!",
               selection.created_at as "selected_at!",
               selection.reason_codes as "reason_codes!: Vec<String>"
        from context_selections selection
        join session_context_runs run
          on run.tenant_id = selection.tenant_id
         and run.id = selection.context_run_id
        where selection.tenant_id = $1
          and selection.knowledge_item_id = $2
          and ($3::timestamptz is null
               or selection.created_at < $3
               or (selection.created_at = $3 and selection.id < $4))
        order by selection.created_at desc, selection.id desc
        limit $5
        "#,
        tenant_id.as_uuid(),
        item_id.as_uuid(),
        before_at,
        before_id,
        limit.max(1),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    rows.into_iter()
        .map(|row| {
            Ok(KnowledgeUsage {
                context_run_id: ContextRunId::from_uuid(row.context_run_id),
                session_id: SessionId::from_uuid(row.session_id),
                knowledge_revision_id: KnowledgeRevisionId::from_uuid(row.knowledge_revision_id),
                selected_at: row.selected_at,
                selection_id: ContextSelectionId::from_uuid(row.selection_id),
                reason_codes: reasons(row.reason_codes)?,
            })
        })
        .collect()
}
