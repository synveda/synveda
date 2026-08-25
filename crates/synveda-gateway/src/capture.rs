//! Session capture batches and reviewable Knowledge candidates (CPR-18,
//! ADR-0083).
//!
//! Extraction freezes an exact event snapshot and stops at candidates. The
//! only transition from a candidate to current Knowledge is through the
//! ordinary [`crate::knowledge`] command service, so personal auto-apply and
//! governed review remain two outcomes of one VedaFlow path.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{Action, Resource};
use synveda_store::anchors::AnchorSelection;
use synveda_store::capture::{self as store, BatchFilter, CandidateFilter};
use synveda_store::imports;
use synveda_store::knowledge::{self as knowledge_store, KnowledgeSnapshot};
use synveda_store::{projects, rls, scopes};
use synveda_types::capture::{
    CaptureBatch, CaptureBatchState, CaptureCandidate, CaptureCandidateState,
    CaptureDecisionAction, CaptureDecisionState, CaptureMatch, CaptureSourceKind,
};
use synveda_types::knowledge::{
    KnowledgeCommand, KnowledgeExpectedRevision, KnowledgeMutationResult, KnowledgeRevisionContent,
    KnowledgeSourceDraft, KnowledgeSourceType,
};
use synveda_types::{
    CaptureBatchId, CaptureCandidateId, Error, ImportArtifactId, ImportJobId, KnowledgeItemId,
    KnowledgeRevisionId, ProjectId, ProposalId, Result, ScopeId, SessionEventId, SessionId,
    TenantId,
};
use utoipa::ToSchema;

use crate::app::AppState;
use crate::audit;
use crate::authz::{self, Authorized};
use crate::error::ApiError;
use crate::idempotency::{Claim, Dispatch};
use crate::knowledge_api::KnowledgeContentBody;
use crate::request::{body, commit, tenant_id};
use crate::sessions;
use crate::workspaces::{ApiErrorBody, string_enum, subject};

/// Capture HTTP outcomes by operation and `ok|rejected|error`.
pub const CAPTURE_API_OPERATIONS_TOTAL: &str = "synveda_capture_api_operations_total";

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

fn capture_source_schema() -> utoipa::openapi::schema::Object {
    string_enum(CaptureSourceKind::ALL.iter().map(|value| value.as_str()))
}

fn batch_state_schema() -> utoipa::openapi::schema::Object {
    string_enum(CaptureBatchState::ALL.iter().map(|value| value.as_str()))
}

fn candidate_state_schema() -> utoipa::openapi::schema::Object {
    string_enum(
        CaptureCandidateState::ALL
            .iter()
            .map(|value| value.as_str()),
    )
}

fn match_kind_schema() -> utoipa::openapi::schema::Object {
    string_enum(
        synveda_types::capture::CaptureMatchKind::ALL
            .iter()
            .map(|value| value.as_str()),
    )
}

fn mutation_outcome_schema() -> utoipa::openapi::schema::Object {
    string_enum(["applied", "pending_review", "rejected"].into_iter())
}

/// One durable extraction job over an exact session-event snapshot.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CaptureBatchView {
    /// Stable batch id.
    #[schema(value_type = String, format = "uuid")]
    pub id: CaptureBatchId,
    /// Session or OKF import.
    #[schema(schema_with = capture_source_schema)]
    pub source_kind: String,
    /// Source session, for session extraction.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub session_id: Option<SessionId>,
    /// Source import job, for OKF materialisation.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub import_job_id: Option<ImportJobId>,
    /// Governed scope copied from the session.
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    /// Project association, when the session had one.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub project_id: Option<ProjectId>,
    /// Exact immutable Configuration version, absent only for the built-in
    /// fail-safe.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub configuration_version_id: Option<synveda_types::ConfigurationVersionId>,
    /// Canonical hash of the frozen runtime document.
    pub configuration_hash: String,
    /// Content-free digest of the ordered frozen evidence set.
    pub input_hash: String,
    /// Frozen event count.
    pub event_count: i32,
    /// Processing state.
    #[schema(schema_with = batch_state_schema)]
    pub state: String,
    /// Extractor implementation, once known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extractor_method: Option<String>,
    /// Model or deterministic ruleset version, once known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_version: Option<String>,
    /// Processing attempts.
    pub attempts: i32,
    /// Reviewable candidates produced.
    pub candidate_count: i32,
    /// Content-free stable failure code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// Creation instant.
    pub created_at: DateTime<Utc>,
    /// First processing instant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    /// Terminal instant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

impl From<CaptureBatch> for CaptureBatchView {
    fn from(batch: CaptureBatch) -> Self {
        Self {
            id: batch.id,
            source_kind: batch.source_kind.as_str().to_owned(),
            session_id: batch.session_id,
            import_job_id: batch.import_job_id,
            scope_id: batch.scope_id,
            project_id: batch.project_id,
            configuration_version_id: batch.configuration_version_id,
            configuration_hash: batch.configuration_hash,
            input_hash: batch.input_hash,
            event_count: batch.event_count,
            state: batch.state.as_str().to_owned(),
            extractor_method: batch.extractor_method,
            model_version: batch.model_version,
            attempts: batch.attempts,
            candidate_count: batch.candidate_count,
            error_code: batch.error_code,
            created_at: batch.created_at,
            started_at: batch.started_at,
            completed_at: batch.completed_at,
        }
    }
}

/// One independently authorised current-Knowledge comparison.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CaptureMatchView {
    /// Existing stable aggregate.
    #[schema(value_type = String, format = "uuid")]
    pub knowledge_item_id: KnowledgeItemId,
    /// Exact revision compared during extraction.
    #[schema(value_type = String, format = "uuid")]
    pub knowledge_revision_id: KnowledgeRevisionId,
    /// Duplicate, conflict or possible supersession.
    #[schema(schema_with = match_kind_schema)]
    pub kind: String,
    /// Deterministic score in `0..=1000`.
    pub similarity_permille: i32,
    /// Stable, content-free classifier reason.
    pub reason_code: String,
}

impl From<CaptureMatch> for CaptureMatchView {
    fn from(matched: CaptureMatch) -> Self {
        Self {
            knowledge_item_id: matched.knowledge_item_id,
            knowledge_revision_id: matched.knowledge_revision_id,
            kind: matched.kind.as_str().to_owned(),
            similarity_permille: matched.similarity_permille,
            reason_code: matched.reason_code,
        }
    }
}

/// One reviewable proposal. It is not active Knowledge.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CaptureCandidateView {
    /// Stable candidate id.
    #[schema(value_type = String, format = "uuid")]
    pub id: CaptureCandidateId,
    /// Owning batch.
    #[schema(value_type = String, format = "uuid")]
    pub batch_id: CaptureBatchId,
    /// Session or OKF import.
    #[schema(schema_with = capture_source_schema)]
    pub source_kind: String,
    /// Source session, for session extraction.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub session_id: Option<SessionId>,
    /// Source import job, for OKF materialisation.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub import_job_id: Option<ImportJobId>,
    /// Stable position within the batch.
    pub ordinal: i32,
    /// Proposed governing scope.
    #[schema(value_type = String, format = "uuid")]
    pub proposed_scope_id: ScopeId,
    /// Proposed project association.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub proposed_project_id: Option<ProjectId>,
    /// Proposed personal owner.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed_owner_principal_id: Option<String>,
    /// Proposed Knowledge type.
    pub knowledge_type: String,
    /// Proposed origin.
    pub origin: String,
    /// Proposed immutable revision content.
    pub content: KnowledgeContentBody,
    /// Canonical proposed-content hash.
    pub content_hash: String,
    /// Review state.
    #[schema(schema_with = candidate_state_schema)]
    pub state: String,
    /// Exact immutable source event ids.
    #[schema(value_type = Vec<String>)]
    pub source_event_ids: Vec<SessionEventId>,
    /// Exact immutable OKF artifacts, for imported candidates.
    #[schema(value_type = Vec<String>)]
    pub source_artifact_ids: Vec<ImportArtifactId>,
    /// Only matches that passed a fresh Knowledge read decision for this caller.
    pub matches: Vec<CaptureMatchView>,
    /// VedaFlow change opened by the decision.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub resulting_change_id: Option<ProposalId>,
    /// Applied, pending review or rejected.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(schema_with = mutation_outcome_schema)]
    pub resulting_outcome: Option<String>,
    /// Resulting Knowledge aggregate.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub resulting_knowledge_item_id: Option<KnowledgeItemId>,
    /// Resulting immutable revision, once applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub resulting_revision_id: Option<KnowledgeRevisionId>,
    /// Actor that made the terminal decision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decided_by: Option<String>,
    /// Bounded dismissal reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_reason: Option<String>,
    /// Decision instant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<DateTime<Utc>>,
    /// Whether governed erasure removed the proposal plaintext.
    pub content_erased: bool,
    /// Creation instant.
    pub created_at: DateTime<Utc>,
}

impl From<CaptureCandidate> for CaptureCandidateView {
    fn from(candidate: CaptureCandidate) -> Self {
        let content = candidate.content;
        Self {
            id: candidate.id,
            batch_id: candidate.batch_id,
            source_kind: candidate.source_kind.as_str().to_owned(),
            session_id: candidate.session_id,
            import_job_id: candidate.import_job_id,
            ordinal: candidate.ordinal,
            proposed_scope_id: candidate.proposed_scope_id,
            proposed_project_id: candidate.proposed_project_id,
            proposed_owner_principal_id: candidate.proposed_owner_principal_id,
            knowledge_type: candidate.knowledge_type.as_str().to_owned(),
            origin: candidate.origin.as_str().to_owned(),
            content: KnowledgeContentBody {
                title: content.title,
                body_markdown: content.body_markdown,
                summary: content.summary,
                tags: content.tags,
                sensitivity: content.sensitivity.as_str().to_owned(),
                confidence_permille: content.confidence_permille,
                valid_from: Some(content.valid_from),
                valid_to: content.valid_to,
                stale_after: content.stale_after,
                verification_metadata: content.verification_metadata,
                metadata: content.metadata,
            },
            content_hash: candidate.content_hash,
            state: candidate.state.as_str().to_owned(),
            source_event_ids: candidate.source_event_ids,
            source_artifact_ids: candidate.source_artifact_ids,
            matches: candidate.matches.into_iter().map(Into::into).collect(),
            resulting_change_id: candidate.resulting_change_id,
            resulting_outcome: candidate
                .resulting_outcome
                .map(|value| value.as_str().to_owned()),
            resulting_knowledge_item_id: candidate.resulting_knowledge_item_id,
            resulting_revision_id: candidate.resulting_revision_id,
            decided_by: candidate.decided_by,
            decision_reason: candidate.decision_reason,
            decided_at: candidate.decided_at,
            content_erased: candidate.content_erased,
            created_at: candidate.created_at,
        }
    }
}

/// One page of capture batches.
#[derive(Debug, Serialize, ToSchema)]
pub struct CaptureBatchListView {
    /// Visible batches.
    pub batches: Vec<CaptureBatchView>,
    /// Opaque resume cursor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// One page of capture candidates.
#[derive(Debug, Serialize, ToSchema)]
pub struct CaptureCandidateListView {
    /// Visible reviewable candidates.
    pub candidates: Vec<CaptureCandidateView>,
    /// Opaque resume cursor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Result of accepting, merging, replacing or dismissing a candidate.
#[derive(Debug, Serialize, ToSchema)]
pub struct CaptureDecisionView {
    /// Candidate after its terminal decision.
    pub candidate: CaptureCandidateView,
    /// Whether this request executed the decision or replayed it.
    pub replayed: bool,
}

/// Optional edits applied while accepting a candidate.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AcceptCandidateBody {
    /// Override the proposed governing scope.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub scope_id: Option<ScopeId>,
    /// Override the proposed project association.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub project_id: Option<Option<ProjectId>>,
    /// Override the proposed owner.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<String>)]
    pub owner_principal_id: Option<Option<String>>,
    /// Override the proposed Knowledge type.
    #[serde(default)]
    pub knowledge_type: Option<String>,
    /// Complete replacement content for edit-and-accept.
    #[serde(default)]
    pub content: Option<KnowledgeContentBody>,
}

/// Merge a candidate with visible current Knowledge.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MergeCandidateBody {
    /// Existing current inputs and their exact inspected heads.
    pub inputs: Vec<crate::knowledge_api::MergeInputBody>,
    /// Optional result placement/content edits.
    #[serde(default)]
    pub result: AcceptCandidateBody,
}

/// Replace one visible current Knowledge item with this candidate.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplaceCandidateBody {
    /// Existing item to supersede.
    #[schema(value_type = String, format = "uuid")]
    pub item_id: KnowledgeItemId,
    /// Exact existing head inspected.
    #[schema(value_type = String, format = "uuid")]
    pub expected_revision_id: KnowledgeRevisionId,
    /// Optional replacement placement/content edits.
    #[serde(default)]
    pub replacement: AcceptCandidateBody,
}

/// Dismissal records no Knowledge mutation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DismissCandidateBody {
    /// Optional bounded human reason.
    #[serde(default)]
    pub reason: Option<String>,
}

/// Batch accept currently applies every pending candidate at its proposed
/// placement. Per-candidate editing remains on the candidate endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AcceptBatchBody {}

/// Distinguishes an omitted edit field from an explicit JSON `null`, which
/// is required to move a candidate out of a project or personal owner.
fn double_option<'de, T, D>(deserializer: D) -> std::result::Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Batch collection filters.
#[derive(Debug, Clone, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListBatchesParams {
    /// Exact session.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "uuid")]
    pub session_id: Option<SessionId>,
    /// Exact project.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "uuid")]
    pub project_id: Option<ProjectId>,
    /// Batch state.
    #[serde(default)]
    pub state: Option<String>,
    /// Opaque cursor.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Rows to serve, 1–200.
    #[serde(default)]
    pub limit: Option<i64>,
}

/// Candidate collection filters.
#[derive(Debug, Clone, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListCandidatesParams {
    /// Exact batch.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "uuid")]
    pub batch_id: Option<CaptureBatchId>,
    /// Exact session.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "uuid")]
    pub session_id: Option<SessionId>,
    /// Proposed project.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "uuid")]
    pub project_id: Option<ProjectId>,
    /// Review state.
    #[serde(default)]
    pub state: Option<String>,
    /// Opaque cursor.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Rows to serve, 1–200.
    #[serde(default)]
    pub limit: Option<i64>,
}

fn list_limit(raw: Option<i64>) -> Result<i64> {
    let value = raw.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&value) {
        return Err(Error::Invalid {
            message: format!("limit must be between 1 and {MAX_LIMIT}"),
        });
    }
    Ok(value)
}

fn encode_cursor(prefix: &str, at: DateTime<Utc>, id: impl std::fmt::Display) -> String {
    URL_SAFE_NO_PAD.encode(format!(
        "{prefix}|{}|{id}",
        at.to_rfc3339_opts(SecondsFormat::Nanos, true)
    ))
}

fn decode_cursor(raw: &str, prefix: &str) -> Result<(DateTime<Utc>, uuid::Uuid)> {
    let invalid = || Error::Invalid {
        message: "invalid capture cursor".to_owned(),
    };
    let decoded = URL_SAFE_NO_PAD.decode(raw).map_err(|_| invalid())?;
    let decoded = String::from_utf8(decoded).map_err(|_| invalid())?;
    let mut parts = decoded.split('|');
    if parts.next() != Some(prefix) {
        return Err(invalid());
    }
    let at = DateTime::parse_from_rfc3339(parts.next().ok_or_else(invalid)?)
        .map_err(|_| invalid())?
        .with_timezone(&Utc);
    let id = parts
        .next()
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    if parts.next().is_some() {
        return Err(invalid());
    }
    Ok((at, id))
}

fn batch_not_found(id: CaptureBatchId) -> Error {
    Error::NotFound {
        entity: format!("capture batch {id}"),
    }
}

fn candidate_not_found(id: CaptureCandidateId) -> Error {
    Error::NotFound {
        entity: format!("capture candidate {id}"),
    }
}

fn capture_action(source: CaptureSourceKind, requested: Action) -> Result<Action> {
    match (source, requested) {
        (CaptureSourceKind::Session, action @ (Action::SessionRead | Action::SessionWrite)) => {
            Ok(action)
        }
        (CaptureSourceKind::OkfImport, Action::SessionRead) => Ok(Action::KnowledgeRead),
        (CaptureSourceKind::OkfImport, Action::SessionWrite) => Ok(Action::KnowledgeWrite),
        _ => Err(Error::Internal {
            message: format!(
                "unsupported capture authorization mapping: {} through {}",
                source.as_str(),
                requested.as_str()
            ),
        }),
    }
}

#[derive(Clone, Copy)]
struct CaptureSourcePlacement {
    kind: CaptureSourceKind,
    session_id: Option<SessionId>,
    import_job_id: Option<ImportJobId>,
    scope_id: ScopeId,
    project_id: Option<ProjectId>,
}

impl From<&CaptureBatch> for CaptureSourcePlacement {
    fn from(batch: &CaptureBatch) -> Self {
        Self {
            kind: batch.source_kind,
            session_id: batch.session_id,
            import_job_id: batch.import_job_id,
            scope_id: batch.scope_id,
            project_id: batch.project_id,
        }
    }
}

impl From<&CaptureCandidate> for CaptureSourcePlacement {
    fn from(candidate: &CaptureCandidate) -> Self {
        Self {
            kind: candidate.source_kind,
            session_id: candidate.session_id,
            import_job_id: candidate.import_job_id,
            scope_id: candidate.proposed_scope_id,
            project_id: candidate.proposed_project_id,
        }
    }
}

async fn authorize_source(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    source: CaptureSourcePlacement,
    requested: Action,
) -> Result<(Authorized, Resource, Action)> {
    let effective = capture_action(source.kind, requested)?;
    match source.kind {
        CaptureSourceKind::Session => {
            let session_id = source.session_id.ok_or_else(|| Error::Internal {
                message: "session-sourced capture row has no session".to_owned(),
            })?;
            if source.import_job_id.is_some() {
                return Err(Error::Internal {
                    message: "session-sourced capture row names an import".to_owned(),
                });
            }
            let (_, allowed, resource) =
                sessions::load(state, tx, tenant, session_id, effective).await?;
            Ok((allowed, resource, effective))
        }
        CaptureSourceKind::OkfImport => {
            let import_id = source.import_job_id.ok_or_else(|| Error::Internal {
                message: "OKF-sourced capture row has no import".to_owned(),
            })?;
            if source.session_id.is_some() {
                return Err(Error::Internal {
                    message: "OKF-sourced capture row names a session".to_owned(),
                });
            }
            let job = imports::get_job(&mut *tx, tenant, import_id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("import job {import_id}"),
                })?;
            if job.scope_id != source.scope_id || Some(job.project_id) != source.project_id {
                return Err(Error::Internal {
                    message: format!("capture source placement disagrees with import {import_id}"),
                });
            }
            let scope = scopes::get(&mut *tx, tenant, source.scope_id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("scope {}", source.scope_id),
                })?;
            let input = authz::gather(
                state,
                tx,
                Some(&scope),
                AnchorSelection::project(job.project_id),
                Vec::new(),
            )
            .await?;
            let resource = Resource::Scope(source.scope_id);
            let allowed = if effective == Action::KnowledgeRead {
                authz::decide_knowledge_read(
                    state,
                    &input,
                    resource,
                    synveda_types::Sensitivity::Public,
                )?
            } else {
                authz::decide(state, &input, effective, resource)?
            };
            Ok((allowed, resource, effective))
        }
    }
}

async fn load_batch(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    id: CaptureBatchId,
    action: Action,
) -> Result<(CaptureBatch, Authorized, Resource, Action)> {
    let batch = store::get_batch(&mut *tx, tenant, id)
        .await?
        .ok_or_else(|| batch_not_found(id))?;
    let (allowed, resource, effective) =
        authorize_source(state, tx, tenant, (&batch).into(), action).await?;
    Ok((batch, allowed, resource, effective))
}

async fn load_candidate(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    id: CaptureCandidateId,
    action: Action,
) -> Result<(CaptureCandidate, Authorized, Resource, Action)> {
    let candidate = store::get_candidate(&mut *tx, tenant, id)
        .await?
        .ok_or_else(|| candidate_not_found(id))?;
    let (allowed, resource, effective) =
        authorize_source(state, tx, tenant, (&candidate).into(), action).await?;
    authorize_candidate_content(state, tx, tenant, &candidate).await?;
    Ok((candidate, allowed, resource, effective))
}

/// Candidate plaintext is governed at its proposed destination as well as by
/// its source session. This is what keeps a personal preference extracted in
/// a shared project session private from teammates who may read the run.
async fn authorize_candidate_content(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    candidate: &CaptureCandidate,
) -> Result<Authorized> {
    let scope = scopes::get(&mut *tx, tenant, candidate.proposed_scope_id)
        .await?
        .ok_or_else(|| candidate_not_found(candidate.id))?;
    let selection = candidate
        .proposed_project_id
        .map_or_else(AnchorSelection::none, AnchorSelection::project);
    let input = authz::gather(state, tx, Some(&scope), selection, Vec::new()).await?;
    authz::decide_knowledge_read(
        state,
        &input,
        Resource::Scope(candidate.proposed_scope_id),
        candidate.content.sensitivity,
    )
}

/// Reusable two-boundary decision for the explicitly configured unreviewed
/// context channel. The source session/import and the proposed Knowledge
/// destination must both remain visible before the planner may retain an id,
/// hash, count, reason or plaintext.
pub(crate) async fn authorize_context_candidate(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    candidate: &CaptureCandidate,
) -> Result<Value> {
    let (source_allowed, source_resource, source_action) =
        authorize_source(state, tx, tenant, candidate.into(), Action::SessionRead).await?;
    let destination_allowed = authorize_candidate_content(state, tx, tenant, candidate).await?;
    Ok(json!({
        "source": audit::decision_context(source_action, &source_allowed),
        "source_resource": source_resource.to_string(),
        "destination": audit::decision_context(Action::KnowledgeRead, &destination_allowed),
    }))
}

/// Remove every comparison the current caller cannot independently read.
/// The omission includes the item id, revision id, edge and count.
pub(crate) async fn retain_visible_matches(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    candidate: &mut CaptureCandidate,
) -> Result<()> {
    let mut visible = Vec::new();
    for matched in std::mem::take(&mut candidate.matches) {
        let Some(current) =
            knowledge_store::current(&mut *tx, tenant, matched.knowledge_item_id).await?
        else {
            continue;
        };
        let revisions =
            knowledge_store::revisions(&mut *tx, tenant, matched.knowledge_item_id).await?;
        let Some(revision) = revisions
            .into_iter()
            .find(|revision| revision.id == matched.knowledge_revision_id)
        else {
            continue;
        };
        let exact = KnowledgeSnapshot {
            item: current.item,
            revision,
            transaction_to: current.transaction_to,
        };
        match crate::knowledge_api::authorize_snapshot(state, tx, tenant, &exact).await {
            Ok(_) => visible.push(matched),
            Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => {}
            Err(error) => return Err(error),
        }
    }
    candidate.matches = visible;
    Ok(())
}

async fn listing_gate(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    session_id: Option<SessionId>,
    project_id: Option<ProjectId>,
) -> Result<Option<(Authorized, Resource, Action)>> {
    if let Some(session_id) = session_id {
        let (_, allowed, resource) =
            sessions::load(state, tx, tenant, session_id, Action::SessionRead).await?;
        return Ok(Some((allowed, resource, Action::SessionRead)));
    }
    let project_gate = project_id.is_some();
    let anchor = if let Some(project_id) = project_id {
        let project = projects::get(&mut *tx, tenant, project_id)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity: format!("project {project_id}"),
            })?;
        scopes::get(&mut *tx, tenant, project.scope_id).await?
    } else {
        scopes::tenant_root(&mut *tx, tenant).await?
    };
    let Some(anchor) = anchor else {
        return Ok(None);
    };
    let input = authz::gather(
        state,
        tx,
        Some(&anchor),
        AnchorSelection::none(),
        Vec::new(),
    )
    .await?;
    let resource = Resource::Scope(anchor.id);
    let action = if project_gate {
        Action::KnowledgeRead
    } else {
        Action::SessionRead
    };
    let allowed = if action == Action::KnowledgeRead {
        authz::decide_knowledge_read(state, &input, resource, synveda_types::Sensitivity::Public)?
    } else {
        authz::decide(state, &input, action, resource)?
    };
    Ok(Some((allowed, resource, action)))
}

async fn capture_read_event(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    op: &'static str,
    action: Action,
    allowed: &Authorized,
    resource: Resource,
    detail: Value,
) -> Result<()> {
    sessions::read_event(tx, tenant, op, action, allowed, resource, detail).await
}

/// `POST /v1/sessions/{session_id}/capture-batches` — freeze the current
/// eligible evidence snapshot for asynchronous extraction.
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/capture-batches",
    operation_id = "create_capture_batch",
    tag = "capture",
    params(
        ("session_id" = String, Path, description = "Source session"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry."),
    ),
    responses(
        (status = 201, description = "Evidence snapshot frozen", body = CaptureBatchView),
        (status = 200, description = "Idempotent replay or already-frozen snapshot", body = CaptureBatchView),
        (status = 400, description = "Missing idempotency key or oversized session", body = ApiErrorBody),
        (status = 403, description = "The PDP denied session.write", body = ApiErrorBody),
        (status = 404, description = "No such session", body = ApiErrorBody),
        (status = 409, description = "Idempotency conflict", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn create_batch(
    State(state): State<AppState>,
    Path(session_id): Path<SessionId>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let actor = subject()?;
        let claim = Claim::from_headers(
            &headers,
            "capture.batch.create",
            &actor,
            &json!({"route": "POST /v1/sessions/{session_id}/capture-batches", "session_id": session_id}),
        )?;
        if let Dispatch::Replay(id) = crate::idempotency::dispatch(&state.pool, tenant, &claim).await?
        {
            let batch = replay_batch(&state, tenant, CaptureBatchId::from_uuid(id)).await?;
            return Ok((StatusCode::OK, Json(CaptureBatchView::from(batch))));
        }

        let frozen = match freeze_claimed_batch(&state, tenant, session_id, &claim).await {
            Ok(frozen) => frozen,
            Err(conflict @ Error::Conflict { .. }) => {
                let id = crate::idempotency::resolve_conflict(
                    &state.pool,
                    tenant,
                    &claim,
                    conflict,
                )
                .await?;
                let batch = replay_batch(&state, tenant, CaptureBatchId::from_uuid(id)).await?;
                return Ok((StatusCode::OK, Json(CaptureBatchView::from(batch))));
            }
            Err(error) => return Err(error),
        };
        let status = if frozen.created { StatusCode::CREATED } else { StatusCode::OK };
        Ok((status, Json(CaptureBatchView::from(frozen.batch))))
    }
    .await;
    respond(&state, "capture.batch.create", result).await
}

async fn replay_batch(
    state: &AppState,
    tenant: TenantId,
    id: CaptureBatchId,
) -> Result<CaptureBatch> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
    let (batch, _, _, _) = load_batch(state, &mut tx, tenant, id, Action::SessionWrite).await?;
    commit(tx).await?;
    Ok(batch)
}

async fn freeze_claimed_batch(
    state: &AppState,
    tenant: TenantId,
    session_id: SessionId,
    claim: &Claim,
) -> Result<store::FrozenBatch> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
    let (session, allowed, _) =
        sessions::load(state, &mut tx, tenant, session_id, Action::SessionWrite).await?;
    let configuration =
        synveda_store::configuration::effective_at_scope(&mut tx, tenant, session.scope_id).await?;
    if !configuration.document.capture.enabled || !configuration.document.capture.explicit_request {
        return Err(Error::PolicyDenied {
            action: "capture.extract".to_owned(),
            resource: Resource::Session(session_id).to_string(),
            reason: "effective configuration disables explicit capture".to_owned(),
        });
    }
    let frozen = store::freeze_batch(&mut tx, &session, &configuration).await?;
    claim
        .remember(&mut tx, tenant, frozen.batch.id.as_uuid())
        .await?;
    if frozen.created {
        audit::record(
            &mut tx,
            tenant,
            AuditAction::CaptureBatchCreated,
            Resource::Session(session_id).to_string(),
            Outcome::Success,
            json!({
                "batch_id": frozen.batch.id,
                "session_id": session_id,
                "input_hash": frozen.batch.input_hash,
                "event_count": frozen.batch.event_count,
                "configuration_version_id": frozen.batch.configuration_version_id,
                "configuration_hash": frozen.batch.configuration_hash,
                "authz": audit::decision_context(Action::SessionWrite, &allowed),
            }),
        )
        .await?;
    }
    commit(tx).await?;
    Ok(frozen)
}

/// `GET /v1/capture-batches`.
#[utoipa::path(
    get,
    path = "/v1/capture-batches",
    operation_id = "list_capture_batches",
    tag = "capture",
    params(ListBatchesParams),
    responses(
        (status = 200, description = "Visible capture batches", body = CaptureBatchListView),
        (status = 400, description = "Invalid filter", body = ApiErrorBody),
        (status = 403, description = "The PDP denied session.read", body = ApiErrorBody),
        (status = 404, description = "Named anchor does not exist", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn list_batches(
    State(state): State<AppState>,
    Query(params): Query<ListBatchesParams>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let limit = list_limit(params.limit)?;
        let state_filter = params.state.as_deref().map(str::parse).transpose()?;
        let after = params
            .cursor
            .as_deref()
            .map(|raw| decode_cursor(raw, "cb1"))
            .transpose()?
            .map(|(created_at, id)| store::BatchCursor {
                created_at,
                id: CaptureBatchId::from_uuid(id),
            });
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let gate = match listing_gate(
            &state,
            &mut tx,
            tenant,
            params.session_id,
            params.project_id,
        )
        .await
        {
            Ok(value) => value,
            Err(Error::PolicyDenied { .. }) => None,
            Err(error) => return Err(error),
        };
        let scanned = store::list_batches(
            &mut *tx,
            tenant,
            &BatchFilter {
                session_id: params.session_id,
                project_id: params.project_id,
                state: state_filter,
                after,
            },
        )
        .await?;
        let mut kept = Vec::new();
        let mut row_allowed = None;
        let mut last = None;
        let total = scanned.len();
        let mut consumed = 0usize;
        for batch in scanned {
            consumed += 1;
            last = Some((batch.created_at, batch.id));
            match authorize_source(
                &state,
                &mut tx,
                tenant,
                (&batch).into(),
                Action::SessionRead,
            )
            .await
            {
                Ok((allowed, resource, action)) => {
                    row_allowed.get_or_insert((allowed, resource, action));
                    kept.push(batch);
                }
                Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => {}
                Err(error) => return Err(error),
            }
            if kept.len() as i64 == limit {
                break;
            }
        }
        let more = consumed < total || total == store::CAPTURE_SCAN_LIMIT as usize;
        let next_cursor = if more {
            last.map(|(at, id)| encode_cursor("cb1", at, id))
        } else {
            None
        };
        let (allowed, resource, action) =
            gate.or(row_allowed).ok_or_else(|| Error::PolicyDenied {
                action: Action::SessionRead.as_str().to_owned(),
                resource: "capture batches".to_owned(),
                reason: "the caller has no readable capture anchor".to_owned(),
            })?;
        capture_read_event(
            &mut tx,
            tenant,
            "capture.batch.list",
            action,
            &allowed,
            resource,
            json!({"served": kept.len(), "more": next_cursor.is_some()}),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(CaptureBatchListView {
            batches: kept.into_iter().map(Into::into).collect(),
            next_cursor,
        }))
    }
    .await;
    respond(&state, "capture.batch.list", result).await
}

/// `GET /v1/capture-batches/{id}`.
#[utoipa::path(
    get,
    path = "/v1/capture-batches/{id}",
    operation_id = "get_capture_batch",
    tag = "capture",
    params(("id" = String, Path, description = "Capture batch id")),
    responses(
        (status = 200, description = "Capture batch", body = CaptureBatchView),
        (status = 403, description = "The PDP denied session.read", body = ApiErrorBody),
        (status = 404, description = "No such batch", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn get_batch(
    State(state): State<AppState>,
    Path(id): Path<CaptureBatchId>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let (batch, allowed, resource, action) =
            load_batch(&state, &mut tx, tenant, id, Action::SessionRead).await?;
        capture_read_event(
            &mut tx,
            tenant,
            "capture.batch.get",
            action,
            &allowed,
            resource,
            json!({"batch_id": id, "candidate_count": batch.candidate_count}),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(CaptureBatchView::from(batch)))
    }
    .await;
    respond(&state, "capture.batch.get", result).await
}

/// `GET /v1/capture-candidates`.
#[utoipa::path(
    get,
    path = "/v1/capture-candidates",
    operation_id = "list_capture_candidates",
    tag = "capture",
    params(ListCandidatesParams),
    responses(
        (status = 200, description = "Visible reviewable candidates", body = CaptureCandidateListView),
        (status = 400, description = "Invalid filter", body = ApiErrorBody),
        (status = 403, description = "The PDP denied session.read", body = ApiErrorBody),
        (status = 404, description = "Named anchor does not exist", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn list_candidates(
    State(state): State<AppState>,
    Query(params): Query<ListCandidatesParams>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let limit = list_limit(params.limit)?;
        let state_filter = params.state.as_deref().map(str::parse).transpose()?;
        let after = params
            .cursor
            .as_deref()
            .map(|raw| decode_cursor(raw, "cc1"))
            .transpose()?
            .map(|(created_at, id)| store::CandidateCursor {
                created_at,
                id: CaptureCandidateId::from_uuid(id),
            });
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let batch_source = if let Some(batch_id) = params.batch_id {
            Some(
                store::get_batch(&mut *tx, tenant, batch_id)
                    .await?
                    .ok_or_else(|| batch_not_found(batch_id))?,
            )
        } else {
            None
        };
        let gate = match listing_gate(
            &state,
            &mut tx,
            tenant,
            params
                .session_id
                .or_else(|| batch_source.as_ref().and_then(|batch| batch.session_id)),
            params
                .project_id
                .or_else(|| batch_source.as_ref().and_then(|batch| batch.project_id)),
        )
        .await
        {
            Ok(value) => value,
            Err(Error::PolicyDenied { .. }) => None,
            Err(error) => return Err(error),
        };
        let scanned = store::list_candidates(
            &mut tx,
            tenant,
            &CandidateFilter {
                batch_id: params.batch_id,
                session_id: params.session_id,
                project_id: params.project_id,
                state: state_filter,
                scope_ids: Vec::new(),
                after,
            },
        )
        .await?;
        let mut kept = Vec::new();
        let mut row_allowed = None;
        let mut last = None;
        let total = scanned.len();
        let mut consumed = 0usize;
        for mut candidate in scanned {
            consumed += 1;
            last = Some((candidate.created_at, candidate.id));
            match authorize_source(
                &state,
                &mut tx,
                tenant,
                (&candidate).into(),
                Action::SessionRead,
            )
            .await
            {
                Ok((allowed, resource, action)) => {
                    match authorize_candidate_content(&state, &mut tx, tenant, &candidate).await {
                        Ok(_) => {}
                        Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => continue,
                        Err(error) => return Err(error),
                    }
                    retain_visible_matches(&state, &mut tx, tenant, &mut candidate).await?;
                    row_allowed.get_or_insert((allowed, resource, action));
                    kept.push(candidate);
                }
                Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => {}
                Err(error) => return Err(error),
            }
            if kept.len() as i64 == limit {
                break;
            }
        }
        let more = consumed < total || total == store::CAPTURE_SCAN_LIMIT as usize;
        let next_cursor = if more {
            last.map(|(at, id)| encode_cursor("cc1", at, id))
        } else {
            None
        };
        let (allowed, resource, action) = gate.or(row_allowed).ok_or_else(|| Error::PolicyDenied {
            action: Action::SessionRead.as_str().to_owned(),
            resource: "capture candidates".to_owned(),
            reason: "the caller has no readable capture anchor".to_owned(),
        })?;
        capture_read_event(
            &mut tx,
            tenant,
            "capture.candidate.list",
            action,
            &allowed,
            resource,
            json!({
                "served": kept.len(),
                "visible_matches": kept.iter().map(|candidate| candidate.matches.len()).sum::<usize>(),
                "more": next_cursor.is_some(),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(CaptureCandidateListView {
            candidates: kept.into_iter().map(Into::into).collect(),
            next_cursor,
        }))
    }
    .await;
    respond(&state, "capture.candidate.list", result).await
}

fn candidate_sources(
    candidate: &CaptureCandidate,
    batch: &CaptureBatch,
    import_job: Option<&synveda_types::import::ImportJob>,
    import_artifacts: &[synveda_types::import::ImportArtifact],
) -> Result<Vec<KnowledgeSourceDraft>> {
    let declared_okf_sources = candidate
        .content
        .metadata
        .get("okf")
        .and_then(|okf| okf.get("frontmatter"))
        .and_then(|frontmatter| frontmatter.get("sources"))
        .and_then(Value::as_array);
    let declared_count = declared_okf_sources.map_or(0, Vec::len);
    if candidate.source_event_ids.len() + candidate.source_artifact_ids.len() + declared_count > 200
    {
        return Err(Error::Invalid {
            message: "a Knowledge revision has at most 200 provenance sources".to_owned(),
        });
    }
    match candidate.source_kind {
        CaptureSourceKind::Session => {
            if candidate.source_event_ids.is_empty()
                || !candidate.source_artifact_ids.is_empty()
                || import_job.is_some()
            {
                return Err(Error::Internal {
                    message: "session candidate has invalid provenance shape".to_owned(),
                });
            }
            Ok(candidate
                .source_event_ids
                .iter()
                .map(|event_id| KnowledgeSourceDraft {
                    id: synveda_types::KnowledgeSourceId::new(),
                    scope_id: batch.scope_id,
                    source_type: KnowledgeSourceType::SessionEvent,
                    session_event_id: Some(*event_id),
                    locator: None,
                    source_revision: None,
                    content_hash: None,
                    metadata: json!({
                        "capture_batch_id": candidate.batch_id,
                        "capture_candidate_id": candidate.id,
                    }),
                })
                .collect())
        }
        CaptureSourceKind::OkfImport => {
            if !candidate.source_event_ids.is_empty() || candidate.source_artifact_ids.is_empty() {
                return Err(Error::Internal {
                    message: "OKF candidate has invalid provenance shape".to_owned(),
                });
            }
            let job = import_job.ok_or_else(|| Error::Internal {
                message: "OKF candidate provenance has no import job".to_owned(),
            })?;
            let mut sources = Vec::with_capacity(candidate.source_artifact_ids.len());
            for artifact_id in &candidate.source_artifact_ids {
                let artifact = import_artifacts
                    .iter()
                    .find(|artifact| artifact.id == *artifact_id)
                    .ok_or_else(|| Error::Internal {
                        message: format!("OKF candidate source artifact {artifact_id} is missing"),
                    })?;
                sources.push(KnowledgeSourceDraft {
                    id: synveda_types::KnowledgeSourceId::new(),
                    scope_id: batch.scope_id,
                    source_type: KnowledgeSourceType::Okf,
                    session_event_id: None,
                    locator: Some(format!("{}#{}", job.source_locator, artifact.logical_path)),
                    source_revision: job.source_revision.clone(),
                    content_hash: Some(artifact.content_hash.clone()),
                    metadata: json!({
                        "import_job_id": job.id,
                        "bundle_digest": job.bundle_digest,
                        "artifact_id": artifact.id,
                        "logical_path": artifact.logical_path,
                        "artifact_kind": artifact.kind.as_str(),
                        "capture_batch_id": candidate.batch_id,
                        "capture_candidate_id": candidate.id,
                    }),
                });
            }
            for declared in declared_okf_sources.into_iter().flatten() {
                let resource = declared
                    .get("resource")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Error::Internal {
                        message: "validated OKF source has no resource".to_owned(),
                    })?
                    .to_owned();
                let lower = resource.to_ascii_lowercase();
                let source_type = if lower.starts_with("http://") || lower.starts_with("https://") {
                    KnowledgeSourceType::Url
                } else if lower.starts_with("git+") || lower.starts_with("repo:") {
                    KnowledgeSourceType::Repository
                } else {
                    KnowledgeSourceType::Document
                };
                sources.push(KnowledgeSourceDraft {
                    id: synveda_types::KnowledgeSourceId::new(),
                    scope_id: batch.scope_id,
                    source_type,
                    session_event_id: None,
                    locator: Some(resource),
                    source_revision: None,
                    content_hash: None,
                    metadata: json!({"okf": {"source": declared}}),
                });
            }
            Ok(sources)
        }
    }
}

struct CandidateMaterial {
    scope_id: ScopeId,
    project_id: Option<ProjectId>,
    owner_principal_id: Option<String>,
    knowledge_type: synveda_types::knowledge::KnowledgeType,
    content: KnowledgeRevisionContent,
    edited: bool,
}

fn material(
    candidate: &CaptureCandidate,
    edits: &AcceptCandidateBody,
) -> Result<CandidateMaterial> {
    let content = match &edits.content {
        Some(content) => crate::knowledge_api::content(content, Utc::now())?,
        None => candidate.content.clone(),
    };
    Ok(CandidateMaterial {
        scope_id: edits.scope_id.unwrap_or(candidate.proposed_scope_id),
        project_id: edits.project_id.unwrap_or(candidate.proposed_project_id),
        owner_principal_id: edits
            .owner_principal_id
            .clone()
            .unwrap_or_else(|| candidate.proposed_owner_principal_id.clone()),
        knowledge_type: edits
            .knowledge_type
            .as_deref()
            .map(str::parse)
            .transpose()?
            .unwrap_or(candidate.knowledge_type),
        content,
        edited: edits.scope_id.is_some()
            || edits.project_id.is_some()
            || edits.owner_principal_id.is_some()
            || edits.knowledge_type.is_some()
            || edits.content.is_some(),
    })
}

enum DecisionSpec {
    Accept(AcceptCandidateBody),
    Merge(MergeCandidateBody),
    Replace(ReplaceCandidateBody),
    Dismiss(DismissCandidateBody),
}

impl DecisionSpec {
    fn action(&self, candidate: &CaptureCandidate) -> Result<CaptureDecisionAction> {
        match self {
            Self::Accept(edits) => {
                if material(candidate, edits)?.edited {
                    Ok(CaptureDecisionAction::EditAndAccept)
                } else {
                    Ok(CaptureDecisionAction::Accept)
                }
            }
            Self::Merge(body) => {
                if body.inputs.is_empty() {
                    return Err(Error::Invalid {
                        message: "candidate merge requires at least one current Knowledge input"
                            .to_owned(),
                    });
                }
                material(candidate, &body.result)?;
                Ok(CaptureDecisionAction::Merge)
            }
            Self::Replace(body) => {
                material(candidate, &body.replacement)?;
                Ok(CaptureDecisionAction::Replace)
            }
            Self::Dismiss(_) => Ok(CaptureDecisionAction::Dismiss),
        }
    }

    fn terminal_state(action: CaptureDecisionAction) -> CaptureCandidateState {
        match action {
            CaptureDecisionAction::Accept => CaptureCandidateState::Accepted,
            CaptureDecisionAction::EditAndAccept => CaptureCandidateState::EditedAndAccepted,
            CaptureDecisionAction::Merge => CaptureCandidateState::Merged,
            CaptureDecisionAction::Replace => CaptureCandidateState::Replaced,
            CaptureDecisionAction::Dismiss => CaptureCandidateState::Dismissed,
        }
    }

    fn reason(&self) -> Option<&str> {
        match self {
            Self::Dismiss(body) => body.reason.as_deref(),
            _ => None,
        }
    }
}

fn command_for(
    candidate: &CaptureCandidate,
    batch: &CaptureBatch,
    import_job: Option<&synveda_types::import::ImportJob>,
    import_artifacts: &[synveda_types::import::ImportArtifact],
    spec: &DecisionSpec,
) -> Result<Option<KnowledgeCommand>> {
    let command = match spec {
        DecisionSpec::Accept(edits) => {
            let value = material(candidate, edits)?;
            KnowledgeCommand::Create {
                item_id: KnowledgeItemId::new(),
                scope_id: value.scope_id,
                project_id: value.project_id,
                owner_principal_id: value.owner_principal_id,
                knowledge_type: value.knowledge_type,
                origin: candidate.origin,
                revision_id: KnowledgeRevisionId::new(),
                content: value.content,
                sources: candidate_sources(candidate, batch, import_job, import_artifacts)?,
            }
        }
        DecisionSpec::Merge(body) => {
            if body.inputs.is_empty() {
                return Err(Error::Invalid {
                    message: "candidate merge requires at least one current Knowledge input"
                        .to_owned(),
                });
            }
            let value = material(candidate, &body.result)?;
            KnowledgeCommand::Merge {
                inputs: body
                    .inputs
                    .iter()
                    .map(|input| KnowledgeExpectedRevision {
                        item_id: input.item_id,
                        revision_id: input.revision_id,
                    })
                    .collect(),
                result_item_id: KnowledgeItemId::new(),
                result_revision_id: KnowledgeRevisionId::new(),
                scope_id: value.scope_id,
                project_id: value.project_id,
                owner_principal_id: value.owner_principal_id,
                knowledge_type: value.knowledge_type,
                origin: candidate.origin,
                content: value.content,
                sources: candidate_sources(candidate, batch, import_job, import_artifacts)?,
            }
        }
        DecisionSpec::Replace(body) => {
            let value = material(candidate, &body.replacement)?;
            KnowledgeCommand::Supersede {
                item_id: body.item_id,
                expected_revision_id: body.expected_revision_id,
                replacement_item_id: KnowledgeItemId::new(),
                replacement_revision_id: KnowledgeRevisionId::new(),
                scope_id: value.scope_id,
                project_id: value.project_id,
                owner_principal_id: value.owner_principal_id,
                knowledge_type: value.knowledge_type,
                origin: candidate.origin,
                content: value.content,
                sources: candidate_sources(candidate, batch, import_job, import_artifacts)?,
            }
        }
        DecisionSpec::Dismiss(_) => return Ok(None),
    };
    Ok(Some(command))
}

async fn decide_candidate(
    state: &AppState,
    headers: &HeaderMap,
    candidate_id: CaptureCandidateId,
    payload: Value,
    spec: DecisionSpec,
    key_override: Option<String>,
) -> Result<CaptureDecisionView> {
    let tenant = tenant_id()?;
    let actor = subject()?;
    // Validate the complete request before writing a durable intent. An
    // invalid edit must not permanently occupy the candidate's one decision
    // slot and prevent a corrected request.
    let (command, action_name) = {
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let (candidate, _, _, _) =
            load_candidate(state, &mut tx, tenant, candidate_id, Action::SessionWrite).await?;
        let batch = store::get_batch(&mut *tx, tenant, candidate.batch_id)
            .await?
            .ok_or_else(|| batch_not_found(candidate.batch_id))?;
        let import_job =
            match candidate.import_job_id {
                Some(id) => Some(imports::get_job(&mut *tx, tenant, id).await?.ok_or_else(
                    || Error::NotFound {
                        entity: format!("import job {id}"),
                    },
                )?),
                None => None,
            };
        let import_artifacts = match candidate.import_job_id {
            Some(id) => imports::artifacts(&mut *tx, tenant, id).await?,
            None => Vec::new(),
        };
        let action = spec.action(&candidate)?;
        let command = command_for(
            &candidate,
            &batch,
            import_job.as_ref(),
            &import_artifacts,
            &spec,
        )?;
        commit(tx).await?;
        (command, action)
    };
    let operation = match action_name {
        CaptureDecisionAction::Accept => "capture.candidate.accept",
        CaptureDecisionAction::EditAndAccept => "capture.candidate.edit_accept",
        CaptureDecisionAction::Merge => "capture.candidate.merge",
        CaptureDecisionAction::Replace => "capture.candidate.replace",
        CaptureDecisionAction::Dismiss => "capture.candidate.dismiss",
    };
    let owned_headers;
    let headers = if let Some(key) = key_override {
        let mut value = HeaderMap::new();
        value.insert(
            crate::idempotency::HEADER,
            key.parse().map_err(|_| Error::Invalid {
                message: "derived batch decision key was invalid".to_owned(),
            })?,
        );
        owned_headers = value;
        &owned_headers
    } else {
        headers
    };
    let claim = Claim::from_headers(headers, operation, &actor, &payload)?;

    let (candidate, decision) = {
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let (candidate, _, _, _) =
            load_candidate(state, &mut tx, tenant, candidate_id, Action::SessionWrite).await?;
        let decision = store::begin_decision(
            &mut tx,
            tenant,
            candidate_id,
            action_name,
            &actor,
            &claim.key,
            &payload,
        )
        .await?;
        commit(tx).await?;
        (candidate, decision)
    };

    if decision.state == CaptureDecisionState::Succeeded {
        let mut candidate = candidate;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        retain_visible_matches(state, &mut tx, tenant, &mut candidate).await?;
        commit(tx).await?;
        return Ok(CaptureDecisionView {
            candidate: candidate.into(),
            replayed: true,
        });
    }
    if candidate.state != CaptureCandidateState::Pending {
        return Err(Error::Conflict {
            message: format!(
                "capture candidate {candidate_id} is already {}",
                candidate.state
            ),
        });
    }

    let mutation: Option<KnowledgeMutationResult> = match command {
        None => None,
        Some(command) => match crate::idempotency::dispatch(&state.pool, tenant, &claim).await? {
            Dispatch::Replay(id) => {
                Some(crate::knowledge::replay_command(state, ProposalId::from_uuid(id)).await?)
            }
            Dispatch::Create => match crate::knowledge::command_idempotent(state, command, &claim)
                .await
            {
                Ok(result) => Some(result),
                Err(conflict @ Error::Conflict { .. }) => {
                    let id =
                        crate::idempotency::resolve_conflict(&state.pool, tenant, &claim, conflict)
                            .await?;
                    Some(crate::knowledge::replay_command(state, ProposalId::from_uuid(id)).await?)
                }
                Err(error) => return Err(error),
            },
        },
    };

    let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
    // A decision intent is durable, but it is not cached authority: repeat the
    // exact source decision immediately before finalising it.
    let (fresh_allowed, fresh_resource, fresh_action) = authorize_source(
        state,
        &mut tx,
        tenant,
        (&candidate).into(),
        Action::SessionWrite,
    )
    .await?;
    let completed = store::complete_decision(
        &mut tx,
        tenant,
        candidate_id,
        DecisionSpec::terminal_state(action_name),
        &actor,
        spec.reason(),
        mutation.as_ref(),
    )
    .await?;
    if completed.completed_now {
        audit::record(
            &mut tx,
            tenant,
            AuditAction::CaptureCandidateDecided,
            fresh_resource.to_string(),
            Outcome::Success,
            json!({
                "candidate_id": candidate_id,
                "batch_id": candidate.batch_id,
                "source_kind": candidate.source_kind.as_str(),
                "session_id": candidate.session_id,
                "import_job_id": candidate.import_job_id,
                "action": action_name.as_str(),
                "decision_id": decision.id,
                "request_hash": decision.request_hash,
                "resulting_change_id": mutation.as_ref().map(|value| value.change_id),
                "resulting_outcome": mutation.as_ref().map(|value| value.outcome.as_str()),
                "resulting_knowledge_item_id": mutation.as_ref().and_then(|value| value.knowledge_item_id),
                "resulting_revision_id": mutation.as_ref().and_then(|value| value.revision_id),
                "authz": audit::decision_context(fresh_action, &fresh_allowed),
            }),
        )
        .await?;
    }
    commit(tx).await?;
    let mut completed_candidate = completed.candidate;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
    retain_visible_matches(state, &mut tx, tenant, &mut completed_candidate).await?;
    commit(tx).await?;
    Ok(CaptureDecisionView {
        candidate: completed_candidate.into(),
        replayed: !completed.completed_now,
    })
}

/// `POST /v1/capture-candidates/{id}/accept`.
#[utoipa::path(
    post,
    path = "/v1/capture-candidates/{id}/accept",
    operation_id = "accept_capture_candidate",
    tag = "capture",
    params(
        ("id" = String, Path, description = "Capture candidate id"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry."),
    ),
    request_body = AcceptCandidateBody,
    responses(
        (status = 201, description = "Candidate decided through VedaFlow", body = CaptureDecisionView),
        (status = 200, description = "Idempotent replay", body = CaptureDecisionView),
        (status = 400, description = "Invalid edit or missing key", body = ApiErrorBody),
        (status = 403, description = "The PDP denied the session or Knowledge mutation", body = ApiErrorBody),
        (status = 404, description = "No such candidate or Knowledge input", body = ApiErrorBody),
        (status = 409, description = "Stale revision or changed retry", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn accept_candidate(
    State(state): State<AppState>,
    Path(id): Path<CaptureCandidateId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<AcceptCandidateBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let canonical =
            json!({"route": "POST /v1/capture-candidates/{id}/accept", "id": id, "body": &body});
        let view = decide_candidate(
            &state,
            &headers,
            id,
            canonical,
            DecisionSpec::Accept(body),
            None,
        )
        .await?;
        Ok((
            if view.replayed {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            },
            Json(view),
        ))
    }
    .await;
    respond(&state, "capture.candidate.accept", result).await
}

/// `POST /v1/capture-candidates/{id}/merge`.
#[utoipa::path(
    post,
    path = "/v1/capture-candidates/{id}/merge",
    operation_id = "merge_capture_candidate",
    tag = "capture",
    params(
        ("id" = String, Path, description = "Capture candidate id"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry."),
    ),
    request_body = MergeCandidateBody,
    responses(
        (status = 201, description = "Governed merge opened", body = CaptureDecisionView),
        (status = 200, description = "Idempotent replay", body = CaptureDecisionView),
        (status = 400, description = "Invalid merge", body = ApiErrorBody),
        (status = 403, description = "The PDP denied an input or output", body = ApiErrorBody),
        (status = 404, description = "No such candidate or input", body = ApiErrorBody),
        (status = 409, description = "Stale revision or changed retry", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn merge_candidate(
    State(state): State<AppState>,
    Path(id): Path<CaptureCandidateId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<MergeCandidateBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let canonical =
            json!({"route": "POST /v1/capture-candidates/{id}/merge", "id": id, "body": &body});
        let view = decide_candidate(
            &state,
            &headers,
            id,
            canonical,
            DecisionSpec::Merge(body),
            None,
        )
        .await?;
        Ok((
            if view.replayed {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            },
            Json(view),
        ))
    }
    .await;
    respond(&state, "capture.candidate.merge", result).await
}

/// `POST /v1/capture-candidates/{id}/replace`.
#[utoipa::path(
    post,
    path = "/v1/capture-candidates/{id}/replace",
    operation_id = "replace_capture_candidate",
    tag = "capture",
    params(
        ("id" = String, Path, description = "Capture candidate id"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry."),
    ),
    request_body = ReplaceCandidateBody,
    responses(
        (status = 201, description = "Governed supersession opened", body = CaptureDecisionView),
        (status = 200, description = "Idempotent replay", body = CaptureDecisionView),
        (status = 400, description = "Invalid replacement", body = ApiErrorBody),
        (status = 403, description = "The PDP denied an input or output", body = ApiErrorBody),
        (status = 404, description = "No such candidate or input", body = ApiErrorBody),
        (status = 409, description = "Stale revision or changed retry", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn replace_candidate(
    State(state): State<AppState>,
    Path(id): Path<CaptureCandidateId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<ReplaceCandidateBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let canonical =
            json!({"route": "POST /v1/capture-candidates/{id}/replace", "id": id, "body": &body});
        let view = decide_candidate(
            &state,
            &headers,
            id,
            canonical,
            DecisionSpec::Replace(body),
            None,
        )
        .await?;
        Ok((
            if view.replayed {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            },
            Json(view),
        ))
    }
    .await;
    respond(&state, "capture.candidate.replace", result).await
}

/// `POST /v1/capture-candidates/{id}/dismiss`.
#[utoipa::path(
    post,
    path = "/v1/capture-candidates/{id}/dismiss",
    operation_id = "dismiss_capture_candidate",
    tag = "capture",
    params(
        ("id" = String, Path, description = "Capture candidate id"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry."),
    ),
    request_body = DismissCandidateBody,
    responses(
        (status = 201, description = "Candidate dismissed", body = CaptureDecisionView),
        (status = 200, description = "Idempotent replay", body = CaptureDecisionView),
        (status = 400, description = "Invalid reason or missing key", body = ApiErrorBody),
        (status = 403, description = "The PDP denied session.write", body = ApiErrorBody),
        (status = 404, description = "No such candidate", body = ApiErrorBody),
        (status = 409, description = "Candidate already decided differently", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn dismiss_candidate(
    State(state): State<AppState>,
    Path(id): Path<CaptureCandidateId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<DismissCandidateBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        if body
            .reason
            .as_ref()
            .is_some_and(|reason| reason.trim().is_empty() || reason.chars().count() > 1_000)
        {
            return Err(Error::Invalid {
                message: "dismissal reason must be 1..=1000 characters when supplied".to_owned(),
            });
        }
        let canonical =
            json!({"route": "POST /v1/capture-candidates/{id}/dismiss", "id": id, "body": &body});
        let view = decide_candidate(
            &state,
            &headers,
            id,
            canonical,
            DecisionSpec::Dismiss(body),
            None,
        )
        .await?;
        Ok((
            if view.replayed {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            },
            Json(view),
        ))
    }
    .await;
    respond(&state, "capture.candidate.dismiss", result).await
}

/// `POST /v1/capture-batches/{id}/accept` — accept every still-pending
/// candidate with deterministic child idempotency keys.
#[utoipa::path(
    post,
    path = "/v1/capture-batches/{id}/accept",
    operation_id = "accept_capture_batch",
    tag = "capture",
    params(
        ("id" = String, Path, description = "Capture batch id"),
        ("Idempotency-Key" = String, Header, description = "Required; reused to derive stable per-candidate keys."),
    ),
    request_body = AcceptBatchBody,
    responses(
        (status = 200, description = "Every pending candidate decided", body = CaptureCandidateListView),
        (status = 400, description = "Missing key", body = ApiErrorBody),
        (status = 403, description = "The PDP denied a decision", body = ApiErrorBody),
        (status = 404, description = "No such batch", body = ApiErrorBody),
        (status = 409, description = "A candidate changed concurrently", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn accept_batch(
    State(state): State<AppState>,
    Path(id): Path<CaptureBatchId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<AcceptBatchBody>, JsonRejection>,
) -> Response {
    let result = async {
        let _ = body(payload)?;
        let tenant = tenant_id()?;
        let actor = subject()?;
        let parent = Claim::from_headers(
            &headers,
            "capture.batch.accept",
            &actor,
            &json!({"route": "POST /v1/capture-batches/{id}/accept", "id": id}),
        )?;
        let remember_parent = match crate::idempotency::dispatch(&state.pool, tenant, &parent).await?
        {
            Dispatch::Create => true,
            Dispatch::Replay(resource_id) if resource_id == id.as_uuid() => false,
            Dispatch::Replay(resource_id) => {
                return Err(Error::Internal {
                    message: format!(
                        "capture batch acceptance key resolved to {resource_id}, expected {id}"
                    ),
                });
            }
        };
        let candidates = {
            let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
            let (batch, _, _, _) =
                load_batch(&state, &mut tx, tenant, id, Action::SessionWrite).await?;
            let candidates = store::list_candidates(
                &mut tx,
                tenant,
                &CandidateFilter {
                    batch_id: Some(id),
                    ..CandidateFilter::default()
                },
            )
            .await?;
            if candidates.len() != batch.candidate_count as usize {
                return Err(Error::Conflict {
                    message: format!("capture batch {id} is not ready for complete acceptance"),
                });
            }
            commit(tx).await?;
            candidates
        };
        let mut views = Vec::new();
        for candidate in candidates {
            if candidate.state != CaptureCandidateState::Pending {
                let mut candidate = candidate;
                let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
                authorize_candidate_content(&state, &mut tx, tenant, &candidate).await?;
                retain_visible_matches(&state, &mut tx, tenant, &mut candidate).await?;
                commit(tx).await?;
                views.push(CaptureCandidateView::from(candidate));
                continue;
            }
            let key = blake3::hash(
                format!("capture.batch.accept\0{}\0{}", parent.key, candidate.id).as_bytes(),
            )
            .to_hex()
            .to_string();
            let canonical = json!({
                "route": "POST /v1/capture-batches/{id}/accept",
                "batch_id": id,
                "candidate_id": candidate.id,
            });
            let decided = decide_candidate(
                &state,
                &headers,
                candidate.id,
                canonical,
                DecisionSpec::Accept(AcceptCandidateBody::default()),
                Some(key),
            )
            .await?;
            views.push(decided.candidate);
        }
        if remember_parent {
            let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
            match parent.remember(&mut tx, tenant, id.as_uuid()).await {
                Ok(()) => commit(tx).await?,
                Err(conflict @ Error::Conflict { .. }) => {
                    tx.rollback().await.map_err(|error| Error::Storage {
                        message: format!("roll back capture batch acceptance race: {error}"),
                    })?;
                    let replayed = crate::idempotency::resolve_conflict(
                        &state.pool,
                        tenant,
                        &parent,
                        conflict,
                    )
                    .await?;
                    if replayed != id.as_uuid() {
                        return Err(Error::Internal {
                            message: format!(
                                "capture batch acceptance race resolved to {replayed}, expected {id}"
                            ),
                        });
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Ok(Json(CaptureCandidateListView {
            candidates: views,
            next_cursor: None,
        }))
    }
    .await;
    respond(&state, "capture.batch.accept", result).await
}

async fn respond<T: IntoResponse>(
    state: &AppState,
    op: &'static str,
    result: Result<T>,
) -> Response {
    let outcome = match &result {
        Ok(_) => "ok",
        Err(
            Error::Unauthenticated { .. }
            | Error::PolicyDenied { .. }
            | Error::NotFound { .. }
            | Error::Invalid { .. }
            | Error::Conflict { .. }
            | Error::RateLimited { .. },
        ) => "rejected",
        Err(_) => "error",
    };
    metrics::counter!(CAPTURE_API_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    match result {
        Ok(value) => value.into_response(),
        Err(error) => {
            audit::record_rejection(state, op, &error).await;
            ApiError(error).into_response()
        }
    }
}
