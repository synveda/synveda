//! Public Knowledge HTTP surface and current-state search (CPR-17,
//! ADR-0082).
//!
//! Every mutation constructs a typed [`synveda_types::knowledge::KnowledgeCommand`]
//! and enters [`crate::knowledge::command_idempotent`]. This module has no
//! direct store mutation call. Every read first resolves tenant ownership and
//! then decides the exact item and current revision sensitivity; source scopes
//! and relation endpoints are decided again before their descriptors or edges
//! are exposed.

use std::collections::HashMap;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use synveda_audit::{AuditAction, Outcome};
use synveda_ingest::FindingCategory;
use synveda_ingest::embedding::Embedder as _;
use synveda_policy::{Action, Resource, ResourceEntity};
use synveda_store::anchors::AnchorSelection;
use synveda_store::context as context_store;
use synveda_store::knowledge::{self as store, KnowledgeSnapshot};
use synveda_store::knowledge_freshness;
use synveda_store::knowledge_search::{self as search, Candidate, Filters, ListCursor};
use synveda_store::{rls, scopes};
use synveda_types::knowledge::{
    FreshnessAssessment, KnowledgeCommand, KnowledgeExpectedRevision, KnowledgeLifecycleState,
    KnowledgeMutationOutcome, KnowledgeMutationResult, KnowledgeOrigin, KnowledgeRelation,
    KnowledgeRelationType, KnowledgeRevision, KnowledgeRevisionContent, KnowledgeSource,
    KnowledgeSourceDraft, KnowledgeSourceType, KnowledgeType, assess_freshness,
    normalise_knowledge_tags,
};
use synveda_types::{
    ContextSelectionId, Error, KnowledgeItemId, KnowledgeRevisionId, ProjectId, ProposalId, Result,
    ScopeId, Sensitivity, SessionEventId, SessionId, TenantId, WorkspaceId,
};
use utoipa::ToSchema;

use crate::app::AppState;
use crate::audit;
use crate::authz::{self, Authorized};
use crate::error::ApiError;
use crate::idempotency::{Claim, Dispatch};
use crate::request::{body, commit, tenant_id};
use crate::workspaces::{ApiErrorBody, string_enum, subject};

/// Knowledge API outcomes by operation and `ok|rejected|error`.
pub const KNOWLEDGE_API_OPERATIONS_TOTAL: &str = "synveda_knowledge_api_operations_total";

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;
const SEARCH_DEPTH: i64 = 1_000;
const RRF_K: f64 = 60.0;
const MAX_QUERY_CHARS: usize = 4_096;

// ── OpenAPI enum schemas ───────────────────────────────────────────────────

fn knowledge_type_schema() -> utoipa::openapi::schema::Object {
    string_enum(KnowledgeType::ALL.iter().map(|value| value.as_str()))
}

fn origin_schema() -> utoipa::openapi::schema::Object {
    string_enum(KnowledgeOrigin::ALL.iter().map(|value| value.as_str()))
}

fn lifecycle_schema() -> utoipa::openapi::schema::Object {
    string_enum(
        KnowledgeLifecycleState::ALL
            .iter()
            .map(|value| value.as_str()),
    )
}

fn source_type_schema() -> utoipa::openapi::schema::Object {
    string_enum(KnowledgeSourceType::ALL.iter().map(|value| value.as_str()))
}

fn relation_type_schema() -> utoipa::openapi::schema::Object {
    string_enum(
        KnowledgeRelationType::ALL
            .iter()
            .map(|value| value.as_str()),
    )
}

fn sensitivity_schema() -> utoipa::openapi::schema::Object {
    string_enum(Sensitivity::ALL.iter().map(|value| value.as_str()))
}

fn mutation_outcome_schema() -> utoipa::openapi::schema::Object {
    string_enum(["applied", "pending_review", "rejected"].into_iter())
}

fn delete_mode_schema() -> utoipa::openapi::schema::Object {
    string_enum(["archive", "forget"].into_iter())
}

// ── Public read views ──────────────────────────────────────────────────────

/// One immutable Knowledge revision as served to an authorised reader.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KnowledgeRevisionView {
    /// Immutable revision id.
    #[schema(value_type = String, format = "uuid")]
    pub id: KnowledgeRevisionId,
    /// Stable item this revision belongs to.
    #[schema(value_type = String, format = "uuid")]
    pub knowledge_item_id: KnowledgeItemId,
    /// Monotonic number within the item.
    pub revision_number: i64,
    /// Human title.
    pub title: String,
    /// Canonical Markdown body.
    pub body_markdown: String,
    /// Retrieval/listing summary.
    pub summary: String,
    /// Canonical lower-case tags.
    pub tags: Vec<String>,
    /// Policy sensitivity.
    #[schema(schema_with = sensitivity_schema)]
    pub sensitivity: String,
    /// Confidence on a 0–1000 integer scale.
    pub confidence_permille: i32,
    /// Beginning of valid time.
    pub valid_from: DateTime<Utc>,
    /// End of valid time, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<DateTime<Utc>>,
    /// Verification due time, when configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_after: Option<DateTime<Utc>>,
    /// Whether verification is due at response time.
    pub stale: bool,
    /// Explainable effective freshness reasons; empty means current.
    pub freshness_reasons: Vec<String>,
    /// Bounded verification evidence.
    #[schema(value_type = Object)]
    pub verification_metadata: Value,
    /// Canonical BLAKE3-256 digest.
    pub content_hash: String,
    /// Forward-compatible product metadata.
    #[schema(value_type = Object)]
    pub metadata: Value,
    /// Author label, when recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    /// Database-stamped transaction time.
    pub transaction_time: DateTime<Utc>,
}

impl KnowledgeRevisionView {
    pub(crate) fn from_revision(revision: KnowledgeRevision, at: DateTime<Utc>) -> Self {
        let content = revision.content;
        Self {
            id: revision.id,
            knowledge_item_id: revision.knowledge_item_id,
            revision_number: revision.revision_number,
            title: content.title,
            body_markdown: content.body_markdown,
            summary: content.summary,
            tags: content.tags,
            sensitivity: content.sensitivity.as_str().to_owned(),
            confidence_permille: content.confidence_permille,
            valid_from: content.valid_from,
            valid_to: content.valid_to,
            stale_after: content.stale_after,
            stale: content.stale_after.is_some_and(|due| due <= at),
            freshness_reasons: content
                .stale_after
                .filter(|due| *due <= at)
                .map(|_| vec!["explicit_date".to_owned()])
                .unwrap_or_default(),
            verification_metadata: content.verification_metadata,
            content_hash: revision.content_hash,
            metadata: content.metadata,
            created_by: revision.created_by,
            transaction_time: revision.transaction_time,
        }
    }
}

/// One visible relation. Both endpoint ids passed independent PDP decisions.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KnowledgeRelationView {
    /// Stable relation id.
    #[schema(value_type = String, format = "uuid")]
    pub id: synveda_types::KnowledgeRelationId,
    /// Visible source item.
    #[schema(value_type = String, format = "uuid")]
    pub source_item_id: KnowledgeItemId,
    /// Visible target item.
    #[schema(value_type = String, format = "uuid")]
    pub target_item_id: KnowledgeItemId,
    /// Exact revision asserting the relation.
    #[schema(value_type = String, format = "uuid")]
    pub asserting_revision_id: KnowledgeRevisionId,
    /// Closed relation vocabulary.
    #[schema(schema_with = relation_type_schema)]
    pub relation_type: String,
    /// Forward-compatible relation metadata.
    #[schema(value_type = Object)]
    pub metadata: Value,
    /// Assertion time.
    pub created_at: DateTime<Utc>,
}

impl From<KnowledgeRelation> for KnowledgeRelationView {
    fn from(relation: KnowledgeRelation) -> Self {
        Self {
            id: relation.id,
            source_item_id: relation.source_item_id,
            target_item_id: relation.target_item_id,
            asserting_revision_id: relation.asserting_revision_id,
            relation_type: relation.relation_type.as_str().to_owned(),
            metadata: relation.metadata,
            created_at: relation.created_at,
        }
    }
}

/// Stable Knowledge head plus its exact current immutable revision.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KnowledgeItemView {
    /// Stable aggregate id.
    #[schema(value_type = String, format = "uuid")]
    pub id: KnowledgeItemId,
    /// Governing scope.
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    /// Associated project.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub project_id: Option<ProjectId>,
    /// Owning principal, for personal Knowledge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_principal_id: Option<String>,
    /// Knowledge type.
    #[schema(schema_with = knowledge_type_schema)]
    pub knowledge_type: String,
    /// Creation origin.
    #[schema(schema_with = origin_schema)]
    pub origin: String,
    /// Governed lifecycle state.
    #[schema(schema_with = lifecycle_schema)]
    pub lifecycle_state: String,
    /// Current immutable revision.
    pub current_revision: KnowledgeRevisionView,
    /// Creation actor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    /// Last head-change actor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_by: Option<String>,
    /// Aggregate creation time.
    pub created_at: DateTime<Utc>,
    /// Last head change.
    pub updated_at: DateTime<Utc>,
    /// Fused search score, absent outside a query listing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_score: Option<f64>,
    /// Visible relations only; omitted from collection rows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relationships: Vec<KnowledgeRelationView>,
}

impl KnowledgeItemView {
    pub(crate) fn from_snapshot(
        snapshot: KnowledgeSnapshot,
        at: DateTime<Utc>,
        score: Option<f64>,
    ) -> Self {
        let KnowledgeSnapshot { item, revision, .. } = snapshot;
        Self {
            id: item.id,
            scope_id: item.scope_id,
            project_id: item.project_id,
            owner_principal_id: item.owner_principal_id,
            knowledge_type: item.knowledge_type.as_str().to_owned(),
            origin: item.origin.as_str().to_owned(),
            lifecycle_state: item.lifecycle_state.as_str().to_owned(),
            current_revision: KnowledgeRevisionView::from_revision(revision, at),
            created_by: item.created_by,
            updated_by: item.updated_by,
            created_at: item.created_at,
            updated_at: item.updated_at,
            match_score: score,
            relationships: Vec::new(),
        }
    }

    fn apply_freshness(&mut self, assessment: &FreshnessAssessment) {
        self.current_revision.stale = assessment.stale;
        self.current_revision.freshness_reasons = assessment
            .reasons
            .iter()
            .map(|reason| reason.as_str().to_owned())
            .collect();
    }
}

/// Cursor-paginated current Knowledge results.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KnowledgeListView {
    /// Policy-visible rows.
    pub items: Vec<KnowledgeItemView>,
    /// Resume position after the last candidate considered. May be present on
    /// an empty page when every candidate was denied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// `listing`, `lexical` or `hybrid`.
    pub retrieval_mode: String,
    /// Honest reason the semantic leg did not run, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degradation: Option<String>,
}

/// Immutable revision history page.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KnowledgeHistoryView {
    /// Policy-visible revisions, newest first.
    pub revisions: Vec<KnowledgeRevisionView>,
    /// Resume position after the last revision considered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// One independently authorised provenance descriptor.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KnowledgeSourceView {
    /// Stable source descriptor id.
    #[schema(value_type = String, format = "uuid")]
    pub id: synveda_types::KnowledgeSourceId,
    /// Scope whose policy admitted this descriptor.
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    /// Source family.
    #[schema(schema_with = source_type_schema)]
    pub source_type: String,
    /// Exact session event for observed Knowledge.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub session_event_id: Option<SessionEventId>,
    /// Logical locator; contains no source payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locator: Option<String>,
    /// External source revision/version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_revision: Option<String>,
    /// Source-content hash when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Bounded extension metadata.
    #[schema(value_type = Object)]
    pub metadata: Value,
    /// Registration time.
    pub created_at: DateTime<Utc>,
}

impl From<KnowledgeSource> for KnowledgeSourceView {
    fn from(source: KnowledgeSource) -> Self {
        Self {
            id: source.id,
            scope_id: source.scope_id,
            source_type: source.source_type.as_str().to_owned(),
            session_event_id: source.session_event_id,
            locator: source.locator,
            source_revision: source.source_revision,
            content_hash: source.content_hash,
            metadata: source.metadata,
            created_at: source.created_at,
        }
    }
}

/// Visible provenance attached to the current revision.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KnowledgeSourcesView {
    /// Independently authorised descriptors, in provenance order.
    pub sources: Vec<KnowledgeSourceView>,
}

/// One policy-visible context use of an exact immutable revision.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KnowledgeUsageView {
    /// Context run that selected the exact revision.
    #[schema(value_type = String, format = "uuid")]
    pub context_run_id: synveda_types::ContextRunId,
    /// Session whose access was independently decided.
    #[schema(value_type = String, format = "uuid")]
    pub session_id: SessionId,
    /// Exact selection, suitable for explicit feedback.
    #[schema(value_type = String, format = "uuid")]
    pub context_selection_id: ContextSelectionId,
    /// Exact revision used.
    #[schema(value_type = String, format = "uuid")]
    pub revision_id: KnowledgeRevisionId,
    /// Selection time.
    pub selected_at: DateTime<Utc>,
    /// Visible reason codes.
    pub reason_codes: Vec<String>,
}

/// Cursor envelope for policy-visible usage history.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KnowledgeUsageListView {
    /// Recorded context uses.
    pub usages: Vec<KnowledgeUsageView>,
    /// Resume cursor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Every Knowledge mutation's stable VedaFlow result envelope.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KnowledgeMutationView {
    /// VedaFlow change/proposal id.
    #[schema(value_type = String, format = "uuid")]
    pub change_id: ProposalId,
    /// Governance outcome.
    #[schema(schema_with = mutation_outcome_schema)]
    pub outcome: String,
    /// Stable result aggregate when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub knowledge_item_id: Option<KnowledgeItemId>,
    /// Resulting immutable revision when applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub revision_id: Option<KnowledgeRevisionId>,
    /// Durable operation for long-running work such as erasure.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub operation_id: Option<synveda_types::DurableOperationId>,
}

impl From<KnowledgeMutationResult> for KnowledgeMutationView {
    fn from(result: KnowledgeMutationResult) -> Self {
        let outcome = match result.outcome {
            KnowledgeMutationOutcome::Applied => "applied",
            KnowledgeMutationOutcome::PendingReview => "pending_review",
            KnowledgeMutationOutcome::Rejected => "rejected",
        };
        Self {
            change_id: result.change_id,
            outcome: outcome.to_owned(),
            knowledge_item_id: result.knowledge_item_id,
            revision_id: result.revision_id,
            operation_id: result.operation_id,
        }
    }
}

// ── Request bodies and filters ─────────────────────────────────────────────

/// Complete content for a new immutable revision.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeContentBody {
    /// Human title.
    pub title: String,
    /// Markdown body.
    pub body_markdown: String,
    /// Short summary.
    pub summary: String,
    /// Canonicalised by the server to lower-case, sorted and unique.
    #[serde(default)]
    pub tags: Vec<String>,
    /// `public`, `internal`, `confidential` or `restricted`.
    #[schema(schema_with = sensitivity_schema)]
    pub sensitivity: String,
    /// Integer confidence from 0 through 1000.
    pub confidence_permille: i32,
    /// Defaults to server time when omitted.
    #[serde(default)]
    pub valid_from: Option<DateTime<Utc>>,
    /// Exclusive end of valid time.
    #[serde(default)]
    pub valid_to: Option<DateTime<Utc>>,
    /// Verification due time.
    #[serde(default)]
    pub stale_after: Option<DateTime<Utc>>,
    /// Bounded verification evidence.
    #[serde(default = "empty_object")]
    #[schema(value_type = Object)]
    pub verification_metadata: Value,
    /// Forward-compatible product metadata.
    #[serde(default = "empty_object")]
    #[schema(value_type = Object)]
    pub metadata: Value,
}

/// A normalised provenance descriptor submitted with a revision.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeSourceBody {
    /// Descriptor disclosure scope. Defaults to the item's governing scope.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub scope_id: Option<ScopeId>,
    /// Source family.
    #[schema(schema_with = source_type_schema)]
    pub source_type: String,
    /// Exact immutable event for `session_event`.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub session_event_id: Option<SessionEventId>,
    /// Stable logical locator for located source families.
    #[serde(default)]
    pub locator: Option<String>,
    /// External revision/version label.
    #[serde(default)]
    pub source_revision: Option<String>,
    /// Lower-case BLAKE3-256 source-content hash.
    #[serde(default)]
    pub content_hash: Option<String>,
    /// Bounded extension metadata.
    #[serde(default = "empty_object")]
    #[schema(value_type = Object)]
    pub metadata: Value,
}

/// `POST /v1/knowledge`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateKnowledgeBody {
    /// Governing scope.
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    /// Optional project association.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub project_id: Option<ProjectId>,
    /// Optional personal owner.
    #[serde(default)]
    pub owner_principal_id: Option<String>,
    /// Knowledge type.
    #[schema(schema_with = knowledge_type_schema)]
    pub knowledge_type: String,
    /// `authored` is the ordinary browser default.
    #[serde(default = "authored_origin")]
    #[schema(schema_with = origin_schema)]
    pub origin: String,
    /// First immutable revision.
    pub content: KnowledgeContentBody,
    /// Provenance. Omission creates one manual descriptor at `scope_id`.
    #[serde(default)]
    pub sources: Vec<KnowledgeSourceBody>,
}

/// `PATCH /v1/knowledge/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EditKnowledgeBody {
    /// Exact current revision the editor inspected.
    #[schema(value_type = String, format = "uuid")]
    pub expected_revision_id: KnowledgeRevisionId,
    /// Complete replacement content.
    pub content: KnowledgeContentBody,
    /// Provenance for this exact revision. Omission records a manual edit.
    #[serde(default)]
    pub sources: Vec<KnowledgeSourceBody>,
}

/// `POST /v1/knowledge/{id}/verify`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct VerifyKnowledgeBody {
    /// Exact current revision the verifier inspected.
    #[schema(value_type = String, format = "uuid")]
    pub expected_revision_id: KnowledgeRevisionId,
    /// Complete bounded verification evidence.
    #[schema(value_type = Object)]
    pub verification_metadata: Value,
}

/// `POST /v1/knowledge/{id}/supersede`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SupersedeKnowledgeBody {
    /// Exact old head inspected.
    #[schema(value_type = String, format = "uuid")]
    pub expected_revision_id: KnowledgeRevisionId,
    /// Replacement governing scope.
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    /// Replacement project association.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub project_id: Option<ProjectId>,
    /// Replacement owner.
    #[serde(default)]
    pub owner_principal_id: Option<String>,
    /// Replacement type.
    #[schema(schema_with = knowledge_type_schema)]
    pub knowledge_type: String,
    /// Replacement origin.
    #[schema(schema_with = origin_schema)]
    pub origin: String,
    /// Replacement's first revision.
    pub content: KnowledgeContentBody,
    /// Replacement provenance.
    #[serde(default)]
    pub sources: Vec<KnowledgeSourceBody>,
}

/// One merge input and stale-write precondition.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MergeInputBody {
    /// Stable input item.
    #[schema(value_type = String, format = "uuid")]
    pub item_id: KnowledgeItemId,
    /// Exact input head inspected.
    #[schema(value_type = String, format = "uuid")]
    pub revision_id: KnowledgeRevisionId,
}

/// `POST /v1/knowledge/merge`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MergeKnowledgeBody {
    /// Two or more current inputs.
    pub inputs: Vec<MergeInputBody>,
    /// Result governing scope.
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    /// Result project association.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub project_id: Option<ProjectId>,
    /// Result owner.
    #[serde(default)]
    pub owner_principal_id: Option<String>,
    /// Result type.
    #[schema(schema_with = knowledge_type_schema)]
    pub knowledge_type: String,
    /// Result origin.
    #[schema(schema_with = origin_schema)]
    pub origin: String,
    /// Result's first revision.
    pub content: KnowledgeContentBody,
}

/// Archive/restore body.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LifecycleKnowledgeBody {
    /// Exact current revision inspected.
    #[schema(value_type = String, format = "uuid")]
    pub expected_revision_id: KnowledgeRevisionId,
    /// Bounded human reason.
    pub reason: String,
}

/// `DELETE /v1/knowledge/{id}`. Deletion without a mode is invalid.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteKnowledgeBody {
    /// `archive` preserves content; `forget` runs governed erasure.
    #[schema(schema_with = delete_mode_schema)]
    pub mode: String,
    /// Exact current revision inspected.
    #[schema(value_type = String, format = "uuid")]
    pub expected_revision_id: KnowledgeRevisionId,
    /// Bounded human reason.
    pub reason: String,
}

/// Query parameters for the Knowledge collection.
#[derive(Debug, Clone, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListKnowledgeParams {
    /// Lexical/semantic query, at most 4096 characters.
    #[serde(default)]
    pub query: Option<String>,
    /// Workspace governed subtree.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "uuid")]
    pub workspace_id: Option<WorkspaceId>,
    /// Exact project association.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "uuid")]
    pub project_id: Option<ProjectId>,
    /// Governed scope subtree.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "uuid")]
    pub scope_id: Option<ScopeId>,
    /// Exact owner principal.
    #[serde(default)]
    pub owner: Option<String>,
    /// Knowledge type.
    #[serde(default)]
    pub knowledge_type: Option<String>,
    /// Creation origin.
    #[serde(default)]
    pub origin: Option<String>,
    /// Lifecycle state. Omission means active only.
    #[serde(default)]
    pub lifecycle_state: Option<String>,
    /// One canonical tag.
    #[serde(default)]
    pub tag: Option<String>,
    /// Source family.
    #[serde(default)]
    pub source: Option<String>,
    /// Updated on or after this instant.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "date-time")]
    pub updated_from: Option<DateTime<Utc>>,
    /// Updated strictly before this instant.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "date-time")]
    pub updated_before: Option<DateTime<Utc>>,
    /// Whether verification is due.
    #[serde(default)]
    pub stale: Option<bool>,
    /// Semantic valid-time instant. Defaults to now.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "date-time")]
    pub as_of: Option<DateTime<Utc>>,
    /// Transaction-time instant. Defaults to now and cannot be in the future.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "date-time")]
    pub as_known_at: Option<DateTime<Utc>>,
    /// Include stale, superseded and archived aggregate heads.
    #[serde(default)]
    pub include_history: bool,
    /// Include unresolved or future transitional heads.
    #[serde(default)]
    pub include_transitional: bool,
    /// Opaque cursor returned by the previous page.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Rows to serve, 1–200; default 50.
    #[serde(default)]
    pub limit: Option<i64>,
}

/// History paging.
#[derive(Debug, Clone, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct HistoryParams {
    /// Opaque revision cursor.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Rows to serve, 1–200; default 50.
    #[serde(default)]
    pub limit: Option<i64>,
}

/// Usage paging (first populated by CPR-18).
#[derive(Debug, Clone, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct UsageParams {
    /// Opaque cursor returned by a prior usage page.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Rows to serve, 1–200; default 50.
    #[serde(default)]
    pub limit: Option<i64>,
}

fn empty_object() -> Value {
    json!({})
}

fn authored_origin() -> String {
    "authored".to_owned()
}

// ── Validation, cursors and shared command plumbing ────────────────────────

fn list_limit(raw: Option<i64>) -> Result<i64> {
    let limit = raw.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(Error::Invalid {
            message: format!("limit must be between 1 and {MAX_LIMIT}"),
        });
    }
    Ok(limit)
}

fn parse_optional<T>(raw: Option<&str>, field: &str) -> Result<Option<T>>
where
    T: std::str::FromStr<Err = Error>,
{
    raw.map(str::parse)
        .transpose()
        .map_err(|error| Error::Invalid {
            message: format!("invalid `{field}`: {error}"),
        })
}

fn normalise_query(raw: Option<&str>) -> Result<Option<String>> {
    let Some(raw) = raw else { return Ok(None) };
    let query = raw.trim();
    if query.is_empty() || query.chars().count() > MAX_QUERY_CHARS {
        return Err(Error::Invalid {
            message: format!("query must be 1..={MAX_QUERY_CHARS} characters after trimming"),
        });
    }
    Ok(Some(query.to_owned()))
}

fn parse_filters(params: &ListKnowledgeParams, now: DateTime<Utc>) -> Result<Filters> {
    if params
        .updated_before
        .zip(params.updated_from)
        .is_some_and(|(before, from)| before <= from)
    {
        return Err(Error::Invalid {
            message: "updated_before must be later than updated_from".to_owned(),
        });
    }
    let tag = match &params.tag {
        Some(tag) => Some(
            normalise_knowledge_tags(std::slice::from_ref(tag))?
                .into_iter()
                .next()
                .expect("one tag remains one tag"),
        ),
        None => None,
    };
    let as_known_at = params.as_known_at.unwrap_or(now);
    if as_known_at > now {
        return Err(Error::Invalid {
            message: "as_known_at cannot be in the future".to_owned(),
        });
    }
    Ok(Filters {
        scope_ids: Vec::new(),
        workspace_id: params.workspace_id,
        project_id: params.project_id,
        scope_id: params.scope_id,
        owner_principal_id: params.owner.as_ref().map(|owner| owner.trim().to_owned()),
        knowledge_type: parse_optional(params.knowledge_type.as_deref(), "knowledge_type")?,
        origin: parse_optional(params.origin.as_deref(), "origin")?,
        lifecycle: parse_optional(params.lifecycle_state.as_deref(), "lifecycle_state")?,
        tag,
        source_type: parse_optional(params.source.as_deref(), "source")?,
        updated_from: params.updated_from,
        updated_before: params.updated_before,
        stale: params.stale,
        at: params.as_of.unwrap_or(now),
        as_known_at,
        include_history: params.include_history,
        include_transitional: params.include_transitional,
    })
}

fn filter_digest(params: &ListKnowledgeParams, query: Option<&str>) -> String {
    let canonical = synveda_types::json::canonicalise(&json!({
        "query": query,
        "workspace_id": params.workspace_id,
        "project_id": params.project_id,
        "scope_id": params.scope_id,
        "owner": params.owner,
        "knowledge_type": params.knowledge_type,
        "origin": params.origin,
        "lifecycle_state": params.lifecycle_state,
        "tag": params.tag,
        "source": params.source,
        "updated_from": params.updated_from,
        "updated_before": params.updated_before,
        "stale": params.stale,
        "as_of": params.as_of,
        "as_known_at": params.as_known_at,
        "include_history": params.include_history,
        "include_transitional": params.include_transitional,
    }));
    blake3::hash(canonical.to_string().as_bytes())
        .to_hex()
        .to_string()
}

#[derive(Debug, Clone, Copy)]
enum PageCursor {
    List(ListCursor),
    Search {
        score: f64,
        updated_at: DateTime<Utc>,
        item_id: KnowledgeItemId,
    },
}

fn encode_cursor(cursor: PageCursor, digest: &str) -> String {
    let raw = match cursor {
        PageCursor::List(cursor) => format!(
            "k1|list|{digest}|{}|{}",
            cursor.updated_at.to_rfc3339(),
            cursor.item_id
        ),
        PageCursor::Search {
            score,
            updated_at,
            item_id,
        } => format!(
            "k1|search|{digest}|{:016x}|{}|{}",
            score.to_bits(),
            updated_at.to_rfc3339(),
            item_id
        ),
    };
    URL_SAFE_NO_PAD.encode(raw)
}

fn decode_cursor(raw: &str, digest: &str, search: bool) -> Result<PageCursor> {
    let invalid = || Error::Invalid {
        message: "`cursor` is not one this Knowledge listing issued for these filters".to_owned(),
    };
    let bytes = URL_SAFE_NO_PAD.decode(raw).map_err(|_| invalid())?;
    let decoded = String::from_utf8(bytes).map_err(|_| invalid())?;
    let parts: Vec<&str> = decoded.split('|').collect();
    if parts.first() != Some(&"k1") || parts.get(2) != Some(&digest) {
        return Err(invalid());
    }
    match (search, parts.as_slice()) {
        (false, [_, "list", _, updated_at, item_id]) => Ok(PageCursor::List(ListCursor {
            updated_at: DateTime::parse_from_rfc3339(updated_at)
                .map_err(|_| invalid())?
                .with_timezone(&Utc),
            item_id: item_id.parse().map_err(|_| invalid())?,
        })),
        (true, [_, "search", _, score, updated_at, item_id]) => {
            let bits = u64::from_str_radix(score, 16).map_err(|_| invalid())?;
            let score = f64::from_bits(bits);
            if !score.is_finite() {
                return Err(invalid());
            }
            Ok(PageCursor::Search {
                score,
                updated_at: DateTime::parse_from_rfc3339(updated_at)
                    .map_err(|_| invalid())?
                    .with_timezone(&Utc),
                item_id: item_id.parse().map_err(|_| invalid())?,
            })
        }
        _ => Err(invalid()),
    }
}

fn encode_history_cursor(item_id: KnowledgeItemId, revision_number: i64) -> String {
    URL_SAFE_NO_PAD.encode(format!("kh1|{item_id}|{revision_number}"))
}

fn decode_history_cursor(raw: &str, item_id: KnowledgeItemId) -> Result<i64> {
    let invalid = || Error::Invalid {
        message: "`cursor` is not one this Knowledge history issued".to_owned(),
    };
    let bytes = URL_SAFE_NO_PAD.decode(raw).map_err(|_| invalid())?;
    let decoded = String::from_utf8(bytes).map_err(|_| invalid())?;
    let mut parts = decoded.split('|');
    let version = parts.next();
    let item = parts.next();
    let revision = parts.next();
    if version != Some("kh1")
        || item != Some(item_id.to_string().as_str())
        || parts.next().is_some()
    {
        return Err(invalid());
    }
    revision
        .ok_or_else(invalid)?
        .parse::<i64>()
        .map_err(|_| invalid())
}

fn encode_usage_cursor(
    item_id: KnowledgeItemId,
    selected_at: DateTime<Utc>,
    selection_id: ContextSelectionId,
) -> String {
    URL_SAFE_NO_PAD.encode(format!(
        "ku1|{item_id}|{}|{selection_id}",
        selected_at.to_rfc3339()
    ))
}

fn decode_usage_cursor(
    raw: &str,
    item_id: KnowledgeItemId,
) -> Result<(DateTime<Utc>, ContextSelectionId)> {
    let invalid = || Error::Invalid {
        message: "`cursor` is not one this Knowledge usage listing issued".to_owned(),
    };
    let bytes = URL_SAFE_NO_PAD.decode(raw).map_err(|_| invalid())?;
    let decoded = String::from_utf8(bytes).map_err(|_| invalid())?;
    let parts: Vec<&str> = decoded.split('|').collect();
    match parts.as_slice() {
        ["ku1", item, selected_at, selection_id] if *item == item_id.to_string() => Ok((
            DateTime::parse_from_rfc3339(selected_at)
                .map_err(|_| invalid())?
                .with_timezone(&Utc),
            selection_id.parse().map_err(|_| invalid())?,
        )),
        _ => Err(invalid()),
    }
}

pub(crate) fn content(
    body: &KnowledgeContentBody,
    default_valid_from: DateTime<Utc>,
) -> Result<KnowledgeRevisionContent> {
    reject_secrets(&json!({
        "title": body.title,
        "body_markdown": body.body_markdown,
        "summary": body.summary,
        "verification_metadata": body.verification_metadata,
        "metadata": body.metadata,
    }))?;
    Ok(KnowledgeRevisionContent {
        title: body.title.clone(),
        body_markdown: body.body_markdown.clone(),
        summary: body.summary.clone(),
        tags: normalise_knowledge_tags(&body.tags)?,
        sensitivity: body.sensitivity.parse()?,
        confidence_permille: body.confidence_permille,
        valid_from: body.valid_from.unwrap_or(default_valid_from),
        valid_to: body.valid_to,
        stale_after: body.stale_after,
        verification_metadata: body.verification_metadata.clone(),
        metadata: body.metadata.clone(),
    })
}

fn sources(
    inputs: &[KnowledgeSourceBody],
    default_scope: ScopeId,
) -> Result<Vec<KnowledgeSourceDraft>> {
    let default;
    let inputs = if inputs.is_empty() {
        default = vec![KnowledgeSourceBody {
            scope_id: Some(default_scope),
            source_type: "manual".to_owned(),
            session_event_id: None,
            locator: None,
            source_revision: None,
            content_hash: None,
            metadata: json!({}),
        }];
        &default
    } else {
        inputs
    };
    if inputs.len() > 200 {
        return Err(Error::Invalid {
            message: "a Knowledge revision has at most 200 sources".to_owned(),
        });
    }
    inputs
        .iter()
        .map(|source| {
            let source_type: KnowledgeSourceType = source.source_type.parse()?;
            reject_secrets(&json!({
                "locator": source.locator,
                "source_revision": source.source_revision,
                "metadata": source.metadata,
            }))?;
            if source_type == KnowledgeSourceType::Url {
                let locator = source.locator.as_deref().ok_or_else(|| Error::Invalid {
                    message: "a url source requires a locator".to_owned(),
                })?;
                let parsed = url::Url::parse(locator).map_err(|_| Error::Invalid {
                    message: "a url source locator must be an absolute URL".to_owned(),
                })?;
                if !matches!(parsed.scheme(), "http" | "https")
                    || !parsed.username().is_empty()
                    || parsed.password().is_some()
                {
                    return Err(Error::Invalid {
                        message: "a url source must be http(s) and contain no credentials"
                            .to_owned(),
                    });
                }
            }
            Ok(KnowledgeSourceDraft {
                id: synveda_types::KnowledgeSourceId::new(),
                scope_id: source.scope_id.unwrap_or(default_scope),
                source_type,
                session_event_id: source.session_event_id,
                locator: source.locator.clone(),
                source_revision: source.source_revision.clone(),
                content_hash: source.content_hash.clone(),
                metadata: source.metadata.clone(),
            })
        })
        .collect()
}

fn reject_secrets(value: &Value) -> Result<()> {
    let scan = synveda_ingest::scan(value.clone());
    let rules: Vec<&str> = scan
        .findings
        .iter()
        .filter(|finding| finding.category == FindingCategory::Secret)
        .map(|finding| finding.rule)
        .collect();
    if rules.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid {
            message: format!(
                "Knowledge content contains material matching secret rules {}; remove the secret and store only a governed reference",
                rules.join(", ")
            ),
        })
    }
}

pub(crate) async fn execute_command<F>(
    state: &AppState,
    headers: &HeaderMap,
    operation: &'static str,
    canonical: Value,
    make_command: F,
) -> Result<(StatusCode, Json<KnowledgeMutationView>)>
where
    F: FnOnce() -> Result<KnowledgeCommand>,
{
    let tenant_id = tenant_id()?;
    let actor = subject()?;
    let claim = Claim::from_headers(headers, operation, &actor, &canonical)?;
    match crate::idempotency::dispatch(&state.pool, tenant_id, &claim).await? {
        Dispatch::Replay(id) => {
            let result = crate::knowledge::replay_command(state, ProposalId::from_uuid(id)).await?;
            Ok((StatusCode::OK, Json(result.into())))
        }
        Dispatch::Create => {
            let command = make_command()?;
            match crate::knowledge::command_idempotent(state, command, &claim).await {
                Ok(result) => Ok((StatusCode::CREATED, Json(result.into()))),
                Err(conflict @ Error::Conflict { .. }) => {
                    let id = crate::idempotency::resolve_conflict(
                        &state.pool,
                        tenant_id,
                        &claim,
                        conflict,
                    )
                    .await?;
                    let result =
                        crate::knowledge::replay_command(state, ProposalId::from_uuid(id)).await?;
                    Ok((StatusCode::OK, Json(result.into())))
                }
                Err(error) => Err(error),
            }
        }
    }
}

// ── Shared read enforcement ────────────────────────────────────────────────

fn item_not_found(item_id: KnowledgeItemId) -> Error {
    Error::NotFound {
        entity: format!("Knowledge item {item_id}"),
    }
}

async fn snapshot(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    item_id: KnowledgeItemId,
) -> Result<KnowledgeSnapshot> {
    store::current(&mut *tx, tenant_id, item_id)
        .await?
        .ok_or_else(|| item_not_found(item_id))
}

pub(crate) async fn authorize_snapshot(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    snapshot: &KnowledgeSnapshot,
) -> Result<Authorized> {
    let scope = scopes::get(&mut *tx, tenant_id, snapshot.item.scope_id)
        .await?
        .ok_or_else(|| item_not_found(snapshot.item.id))?;
    let selection = snapshot
        .item
        .project_id
        .map_or_else(AnchorSelection::none, AnchorSelection::project);
    let input = authz::gather(
        state,
        tx,
        Some(&scope),
        selection,
        vec![ResourceEntity::KnowledgeItem {
            id: snapshot.item.id,
            scope_id: snapshot.item.scope_id,
        }],
    )
    .await?;
    authz::decide_knowledge_read(
        state,
        &input,
        Resource::KnowledgeItem(snapshot.item.id),
        snapshot.revision.content.sensitivity,
    )
}

pub(crate) async fn listing_gate(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
) -> Result<(Authorized, Resource)> {
    let input = authz::gather_at_home(state, tx).await?;
    let scope_id =
        input
            .chain
            .first()
            .map(|scope| scope.id)
            .ok_or_else(|| Error::PolicyDenied {
                action: Action::KnowledgeRead.as_str().to_owned(),
                resource: "Knowledge listing".to_owned(),
                reason: "the caller has no governed principal scope".to_owned(),
            })?;
    let resource = Resource::Scope(scope_id);
    let allowed = authz::decide_knowledge_read(state, &input, resource, Sensitivity::Public)?;
    Ok((allowed, resource))
}

pub(crate) async fn read_event(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    op: &'static str,
    authorized: &Authorized,
    resource: Resource,
    detail: Value,
) -> Result<()> {
    audit::record(
        tx,
        tenant_id,
        AuditAction::AuthzDecision,
        resource.to_string(),
        Outcome::Allow,
        json!({
            "op": op,
            "authz": audit::decision_context(Action::KnowledgeRead, authorized),
            "detail": detail,
        }),
    )
    .await
    .map(|_| ())
}

pub(crate) async fn respond<T: IntoResponse>(
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
    metrics::counter!(KNOWLEDGE_API_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome)
        .increment(1);
    match result {
        Ok(value) => value.into_response(),
        Err(error) => {
            audit::record_rejection(state, op, &error).await;
            ApiError(error).into_response()
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FusedCandidate {
    item_id: KnowledgeItemId,
    updated_at: DateTime<Utc>,
    score: f64,
}

fn fuse(lexical: &[Candidate], semantic: &[Candidate]) -> Vec<FusedCandidate> {
    let mut fused: HashMap<KnowledgeItemId, FusedCandidate> = HashMap::new();
    for (rank, candidate) in lexical.iter().enumerate() {
        let entry = fused.entry(candidate.item_id).or_insert(FusedCandidate {
            item_id: candidate.item_id,
            updated_at: candidate.updated_at,
            score: 0.0,
        });
        entry.score += 1.0 / (RRF_K + rank as f64 + 1.0);
    }
    for (rank, candidate) in semantic.iter().enumerate() {
        let entry = fused.entry(candidate.item_id).or_insert(FusedCandidate {
            item_id: candidate.item_id,
            updated_at: candidate.updated_at,
            score: 0.0,
        });
        entry.updated_at = entry.updated_at.max(candidate.updated_at);
        entry.score += 1.0 / (RRF_K + rank as f64 + 1.0);
    }
    let mut candidates: Vec<_> = fused.into_values().collect();
    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| right.item_id.cmp(&left.item_id))
    });
    candidates
}

fn after_search_cursor(candidate: &FusedCandidate, cursor: PageCursor) -> bool {
    let PageCursor::Search {
        score,
        updated_at,
        item_id,
    } = cursor
    else {
        return false;
    };
    candidate.score < score
        || (candidate.score.to_bits() == score.to_bits()
            && (candidate.updated_at < updated_at
                || (candidate.updated_at == updated_at && candidate.item_id < item_id)))
}

struct VisibilityPage<'a> {
    state: &'a AppState,
    tenant_id: TenantId,
    limit: usize,
    more_candidates: bool,
    filter_digest: &'a str,
    at: DateTime<Utc>,
    as_known_at: DateTime<Utc>,
    stale_filter: Option<bool>,
}

async fn hydrate_candidate(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    item_id: KnowledgeItemId,
    as_known_at: DateTime<Utc>,
) -> Result<Option<KnowledgeSnapshot>> {
    store::as_known_at(&mut *tx, tenant_id, item_id, as_known_at).await
}

async fn snapshot_freshness(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    snapshot: &KnowledgeSnapshot,
    at: DateTime<Utc>,
) -> Result<FreshnessAssessment> {
    let evidence = knowledge_freshness::evidence(
        tx,
        tenant_id,
        snapshot.revision.id,
        snapshot.item.project_id,
    )
    .await?;
    Ok(assess_freshness(
        snapshot.item.knowledge_type,
        snapshot.item.lifecycle_state,
        snapshot.revision.content.stale_after,
        evidence,
        at,
    ))
}

async fn visible_plain_page(
    tx: &mut sqlx::PgConnection,
    candidates: Vec<Candidate>,
    page: VisibilityPage<'_>,
) -> Result<(Vec<KnowledgeItemView>, Option<String>, usize)> {
    let total = candidates.len();
    let mut served = Vec::new();
    let mut consumed = 0usize;
    let mut last = None;
    for candidate in candidates {
        consumed += 1;
        last = Some(candidate);
        let Some(snapshot) =
            hydrate_candidate(tx, page.tenant_id, candidate.item_id, page.as_known_at).await?
        else {
            continue;
        };
        // Hydrate-and-verify exact transaction truth. A head change between
        // candidate generation and hydration cannot substitute newer content.
        if snapshot.item.updated_at != candidate.updated_at {
            continue;
        }
        match authorize_snapshot(page.state, tx, page.tenant_id, &snapshot).await {
            Ok(_) => {
                let freshness = snapshot_freshness(tx, page.tenant_id, &snapshot, page.at).await?;
                if page
                    .stale_filter
                    .is_some_and(|expected| expected != freshness.stale)
                {
                    continue;
                }
                let mut view = KnowledgeItemView::from_snapshot(snapshot, page.at, None);
                view.apply_freshness(&freshness);
                served.push(view);
            }
            Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => continue,
            Err(error) => return Err(error),
        }
        if served.len() >= page.limit {
            break;
        }
    }
    let has_more = consumed < total || page.more_candidates;
    let next = if has_more {
        last.map(|candidate| {
            encode_cursor(
                PageCursor::List(ListCursor {
                    updated_at: candidate.updated_at,
                    item_id: candidate.item_id,
                }),
                page.filter_digest,
            )
        })
    } else {
        None
    };
    Ok((served, next, consumed))
}

async fn visible_search_page(
    tx: &mut sqlx::PgConnection,
    candidates: Vec<FusedCandidate>,
    page: VisibilityPage<'_>,
) -> Result<(Vec<KnowledgeItemView>, Option<String>, usize)> {
    let total = candidates.len();
    let mut served = Vec::new();
    let mut consumed = 0usize;
    let mut last = None;
    for candidate in candidates {
        consumed += 1;
        last = Some(candidate);
        let Some(snapshot) =
            hydrate_candidate(tx, page.tenant_id, candidate.item_id, page.as_known_at).await?
        else {
            continue;
        };
        if snapshot.item.updated_at != candidate.updated_at {
            continue;
        }
        match authorize_snapshot(page.state, tx, page.tenant_id, &snapshot).await {
            Ok(_) => {
                let freshness = snapshot_freshness(tx, page.tenant_id, &snapshot, page.at).await?;
                if page
                    .stale_filter
                    .is_some_and(|expected| expected != freshness.stale)
                {
                    continue;
                }
                let mut view =
                    KnowledgeItemView::from_snapshot(snapshot, page.at, Some(candidate.score));
                view.apply_freshness(&freshness);
                served.push(view);
            }
            Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => continue,
            Err(error) => return Err(error),
        }
        if served.len() >= page.limit {
            break;
        }
    }
    let has_more = consumed < total || page.more_candidates;
    let next = if has_more {
        last.map(|candidate| {
            encode_cursor(
                PageCursor::Search {
                    score: candidate.score,
                    updated_at: candidate.updated_at,
                    item_id: candidate.item_id,
                },
                page.filter_digest,
            )
        })
    } else {
        None
    };
    Ok((served, next, consumed))
}

async fn semantic_query(state: &AppState, query: &str) -> (Option<Vec<f32>>, Option<String>) {
    if state.embedder.method() != "tei" {
        return (
            None,
            Some("deterministic_embedder_is_not_semantic".to_owned()),
        );
    }
    let inputs = vec![query.to_owned()];
    match tokio::time::timeout(state.context_embed_timeout, state.embedder.embed(&inputs)).await {
        Ok(Ok(mut vectors)) if vectors.len() == 1 && !vectors[0].is_empty() => {
            (vectors.pop(), None)
        }
        Ok(Ok(vectors)) => {
            tracing::warn!(
                vectors = vectors.len(),
                "Knowledge query embedder returned the wrong shape; lexical-only"
            );
            (None, Some("semantic_embedder_invalid_response".to_owned()))
        }
        Ok(Err(error)) => {
            tracing::warn!(%error, "Knowledge query embedding failed; lexical-only");
            (None, Some("semantic_embedder_unavailable".to_owned()))
        }
        Err(_) => {
            tracing::warn!(
                timeout_ms = state.context_embed_timeout.as_millis() as u64,
                "Knowledge query embedding timed out; lexical-only"
            );
            (None, Some("semantic_embedder_timeout".to_owned()))
        }
    }
}

/// `GET /v1/knowledge` — current policy-visible Knowledge.
#[utoipa::path(
    get,
    path = "/v1/knowledge",
    operation_id = "list_knowledge",
    tag = "knowledge",
    params(ListKnowledgeParams),
    responses(
        (status = 200, description = "Current policy-visible Knowledge", body = KnowledgeListView),
        (status = 400, description = "Invalid filter, cursor or limit", body = ApiErrorBody),
        (status = 403, description = "The PDP denied Knowledge reading", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.list", skip_all)]
pub(crate) async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListKnowledgeParams>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let at = Utc::now();
        let limit = list_limit(params.limit)?;
        let query = normalise_query(params.query.as_deref())?;
        let filters = parse_filters(&params, at)?;
        let digest = filter_digest(&params, query.as_deref());
        let scan_limit = (limit * 10).min(500);

        if query.is_none() {
            let cursor = params
                .cursor
                .as_deref()
                .map(|raw| decode_cursor(raw, &digest, false))
                .transpose()?
                .map(|cursor| match cursor {
                    PageCursor::List(cursor) => cursor,
                    PageCursor::Search { .. } => unreachable!("decoder mode"),
                });
            let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
            let (gate, resource) = listing_gate(&state, &mut tx).await?;
            let mut candidates =
                search::list_candidates(&mut tx, tenant_id, &filters, cursor, scan_limit + 1)
                    .await?;
            let more = candidates.len() as i64 > scan_limit;
            candidates.truncate(scan_limit as usize);
            let (items, next_cursor, considered) = visible_plain_page(
                &mut tx,
                candidates,
                VisibilityPage {
                    state: &state,
                    tenant_id,
                    limit: limit as usize,
                    more_candidates: more,
                    filter_digest: &digest,
                    at: filters.at,
                    as_known_at: filters.as_known_at,
                    stale_filter: filters.stale,
                },
            )
            .await?;
            read_event(
                &mut tx,
                tenant_id,
                "knowledge.list",
                &gate,
                resource,
                json!({
                    "filter_hash": digest,
                    "query": false,
                    "considered": considered,
                    "served": items.len(),
                    "retrieval_mode": "listing",
                }),
            )
            .await?;
            commit(tx).await?;
            return Ok(Json(KnowledgeListView {
                items,
                next_cursor,
                retrieval_mode: "listing".to_owned(),
                degradation: None,
            }));
        }

        // An embedding call is an external dependency. Take a short PDP gate
        // first, then release the transaction before crossing that boundary.
        let mut preflight = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        listing_gate(&state, &mut preflight).await?;
        drop(preflight);
        let query = query.expect("checked");
        let (query_vector, mut degradation) = semantic_query(&state, &query).await;

        let cursor = params
            .cursor
            .as_deref()
            .map(|raw| decode_cursor(raw, &digest, true))
            .transpose()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let (gate, resource) = listing_gate(&state, &mut tx).await?;
        let lexical =
            search::lexical_candidates(&mut tx, tenant_id, &filters, &query, SEARCH_DEPTH).await?;
        let mut semantic = Vec::new();
        let mut semantic_ran = false;
        if let Some(vector) = &query_vector {
            match search::semantic_candidates(
                &mut tx,
                tenant_id,
                &filters,
                state.embedder.model(),
                vector,
                SEARCH_DEPTH,
            )
            .await
            {
                Ok(rows) => {
                    semantic_ran = true;
                    semantic = rows;
                    if semantic.is_empty()
                        && search::embedding_count(&mut tx, tenant_id, state.embedder.model())
                            .await?
                            == 0
                    {
                        semantic_ran = false;
                        degradation = Some("semantic_index_not_ready".to_owned());
                    }
                }
                Err(Error::Invalid { .. }) => {
                    degradation = Some("semantic_dimension_not_indexed".to_owned());
                }
                Err(error) => return Err(error),
            }
        }
        let mut fused = fuse(&lexical, &semantic);
        if let Some(cursor) = cursor {
            fused.retain(|candidate| after_search_cursor(candidate, cursor));
        }
        let more = fused.len() as i64 > scan_limit;
        fused.truncate(scan_limit as usize);
        let (items, next_cursor, considered) = visible_search_page(
            &mut tx,
            fused,
            VisibilityPage {
                state: &state,
                tenant_id,
                limit: limit as usize,
                more_candidates: more,
                filter_digest: &digest,
                at: filters.at,
                as_known_at: filters.as_known_at,
                stale_filter: filters.stale,
            },
        )
        .await?;
        let retrieval_mode = if semantic_ran { "hybrid" } else { "lexical" };
        read_event(
            &mut tx,
            tenant_id,
            "knowledge.list",
            &gate,
            resource,
            json!({
                "filter_hash": digest,
                "query": true,
                "considered": considered,
                "served": items.len(),
                "retrieval_mode": retrieval_mode,
                "degradation": degradation,
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(KnowledgeListView {
            items,
            next_cursor,
            retrieval_mode: retrieval_mode.to_owned(),
            degradation,
        }))
    }
    .await;
    respond(&state, "knowledge.list", result).await
}

async fn visible_relations(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    item_id: KnowledgeItemId,
) -> Result<Vec<KnowledgeRelationView>> {
    let relations = store::relations(&mut *tx, tenant_id, item_id).await?;
    let mut visible = Vec::new();
    for relation in relations {
        let other_id = if relation.source_item_id == item_id {
            relation.target_item_id
        } else {
            relation.source_item_id
        };
        let Some(other) = store::current(&mut *tx, tenant_id, other_id).await? else {
            continue;
        };
        match authorize_snapshot(state, tx, tenant_id, &other).await {
            Ok(_) => visible.push(relation.into()),
            Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(visible)
}

/// `GET /v1/knowledge/{id}` — current content and visible relationships.
#[utoipa::path(
    get,
    path = "/v1/knowledge/{id}",
    operation_id = "get_knowledge",
    tag = "knowledge",
    params(("id" = String, Path, description = "Stable Knowledge item id")),
    responses(
        (status = 200, description = "Current Knowledge item", body = KnowledgeItemView),
        (status = 403, description = "The PDP denied Knowledge reading", body = ApiErrorBody),
        (status = 404, description = "No such visible item", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.get", skip_all, fields(knowledge.item.id = %id))]
pub(crate) async fn get(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeItemId>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let at = Utc::now();
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let snapshot = snapshot(&mut tx, tenant_id, id).await?;
        let allowed = authorize_snapshot(&state, &mut tx, tenant_id, &snapshot).await?;
        let relations = visible_relations(&state, &mut tx, tenant_id, id).await?;
        let freshness = snapshot_freshness(&mut tx, tenant_id, &snapshot, at).await?;
        let revision_id = snapshot.revision.id;
        let mut view = KnowledgeItemView::from_snapshot(snapshot, at, None);
        view.apply_freshness(&freshness);
        view.relationships = relations;
        read_event(
            &mut tx,
            tenant_id,
            "knowledge.get",
            &allowed,
            Resource::KnowledgeItem(id),
            json!({
                "knowledge_item_id": id,
                "revision_id": revision_id,
                "visible_relationships": view.relationships.len(),
            }),
        )
        .await?;
        commit(tx).await?;
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ETAG,
            HeaderValue::from_str(&format!("\"{revision_id}\"")).map_err(|_| Error::Internal {
                message: "Knowledge revision did not form an ETag".to_owned(),
            })?,
        );
        Ok((headers, Json(view)))
    }
    .await;
    respond(&state, "knowledge.get", result).await
}

/// `GET /v1/knowledge/{id}/history` — immutable revisions newest first.
#[utoipa::path(
    get,
    path = "/v1/knowledge/{id}/history",
    operation_id = "get_knowledge_history",
    tag = "knowledge",
    params(
        ("id" = String, Path, description = "Stable Knowledge item id"),
        HistoryParams,
    ),
    responses(
        (status = 200, description = "Visible immutable revision history", body = KnowledgeHistoryView),
        (status = 400, description = "Invalid cursor or limit", body = ApiErrorBody),
        (status = 403, description = "The PDP denied Knowledge reading", body = ApiErrorBody),
        (status = 404, description = "No such visible item", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.history", skip_all, fields(knowledge.item.id = %id))]
pub(crate) async fn history(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeItemId>,
    Query(params): Query<HistoryParams>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let limit = list_limit(params.limit)?;
        let cursor = params
            .cursor
            .as_deref()
            .map(|raw| decode_history_cursor(raw, id))
            .transpose()?;
        let at = Utc::now();
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let current = snapshot(&mut tx, tenant_id, id).await?;
        let allowed = authorize_snapshot(&state, &mut tx, tenant_id, &current).await?;
        let scope = scopes::get(&mut *tx, tenant_id, current.item.scope_id)
            .await?
            .ok_or_else(|| item_not_found(id))?;
        let input = authz::gather(
            &state,
            &mut tx,
            Some(&scope),
            current
                .item
                .project_id
                .map_or_else(AnchorSelection::none, AnchorSelection::project),
            vec![ResourceEntity::KnowledgeItem {
                id,
                scope_id: current.item.scope_id,
            }],
        )
        .await?;
        let mut revisions = store::revisions(&mut *tx, tenant_id, id).await?;
        revisions.reverse();
        if let Some(cursor) = cursor {
            revisions.retain(|revision| revision.revision_number < cursor);
        }
        let mut views = Vec::new();
        let mut considered = 0usize;
        let mut last = None;
        for revision in revisions {
            considered += 1;
            last = Some(revision.revision_number);
            match authz::decide_knowledge_read(
                &state,
                &input,
                Resource::KnowledgeItem(id),
                revision.content.sensitivity,
            ) {
                Ok(_) => views.push(KnowledgeRevisionView::from_revision(revision, at)),
                Err(Error::PolicyDenied { .. }) => continue,
                Err(error) => return Err(error),
            }
            if views.len() >= limit as usize {
                break;
            }
        }
        let more = considered
            < store::revisions(&mut *tx, tenant_id, id)
                .await?
                .into_iter()
                .filter(|revision| cursor.is_none_or(|cursor| revision.revision_number < cursor))
                .count();
        let next_cursor = if more {
            last.map(|revision| encode_history_cursor(id, revision))
        } else {
            None
        };
        read_event(
            &mut tx,
            tenant_id,
            "knowledge.history",
            &allowed,
            Resource::KnowledgeItem(id),
            json!({
                "knowledge_item_id": id,
                "considered": considered,
                "served": views.len(),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(KnowledgeHistoryView {
            revisions: views,
            next_cursor,
        }))
    }
    .await;
    respond(&state, "knowledge.history", result).await
}

/// `GET /v1/knowledge/{id}/sources` — independently governed provenance.
#[utoipa::path(
    get,
    path = "/v1/knowledge/{id}/sources",
    operation_id = "get_knowledge_sources",
    tag = "knowledge",
    params(("id" = String, Path, description = "Stable Knowledge item id")),
    responses(
        (status = 200, description = "Independently visible current-revision sources", body = KnowledgeSourcesView),
        (status = 403, description = "The PDP denied the item or every source scope", body = ApiErrorBody),
        (status = 404, description = "No such visible item", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.sources", skip_all, fields(knowledge.item.id = %id))]
pub(crate) async fn sources_for_item(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeItemId>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let snapshot = snapshot(&mut tx, tenant_id, id).await?;
        let item_allowed = authorize_snapshot(&state, &mut tx, tenant_id, &snapshot).await?;
        let source_scopes = search::source_scopes(&mut tx, tenant_id, snapshot.revision.id).await?;
        let mut visible_scopes = Vec::new();
        for scope_id in source_scopes {
            let Some(scope) = scopes::get(&mut *tx, tenant_id, scope_id).await? else {
                continue;
            };
            let input = authz::gather(
                &state,
                &mut tx,
                Some(&scope),
                AnchorSelection::none(),
                Vec::new(),
            )
            .await?;
            match authz::decide_knowledge_read(
                &state,
                &input,
                Resource::Scope(scope_id),
                snapshot.revision.content.sensitivity,
            ) {
                Ok(_) => visible_scopes.push(scope_id),
                Err(Error::PolicyDenied { .. }) => continue,
                Err(error) => return Err(error),
            }
        }
        let sources =
            store::visible_sources(&mut *tx, tenant_id, snapshot.revision.id, &visible_scopes)
                .await?;
        read_event(
            &mut tx,
            tenant_id,
            "knowledge.sources",
            &item_allowed,
            Resource::KnowledgeItem(id),
            json!({
                "knowledge_item_id": id,
                "revision_id": snapshot.revision.id,
                "visible_sources": sources.len(),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(KnowledgeSourcesView {
            sources: sources.into_iter().map(Into::into).collect(),
        }))
    }
    .await;
    respond(&state, "knowledge.sources", result).await
}

/// `GET /v1/knowledge/{id}/usage` — context selections of exact revisions.
#[utoipa::path(
    get,
    path = "/v1/knowledge/{id}/usage",
    operation_id = "get_knowledge_usage",
    tag = "knowledge",
    params(
        ("id" = String, Path, description = "Stable Knowledge item id"),
        UsageParams,
    ),
    responses(
        (status = 200, description = "Visible revision usage history", body = KnowledgeUsageListView),
        (status = 400, description = "Invalid cursor or limit", body = ApiErrorBody),
        (status = 403, description = "The PDP denied Knowledge reading", body = ApiErrorBody),
        (status = 404, description = "No such visible item", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.usage", skip_all, fields(knowledge.item.id = %id))]
pub(crate) async fn usage(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeItemId>,
    Query(params): Query<UsageParams>,
) -> Response {
    let result = async {
        let limit = list_limit(params.limit)?;
        let cursor = params
            .cursor
            .as_deref()
            .map(|raw| decode_usage_cursor(raw, id))
            .transpose()?;
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let snapshot = snapshot(&mut tx, tenant_id, id).await?;
        let allowed = authorize_snapshot(&state, &mut tx, tenant_id, &snapshot).await?;
        let scan_limit = (limit * 10).min(500);
        let mut candidates =
            context_store::usage_candidates(&mut tx, tenant_id, id, cursor, scan_limit + 1).await?;
        let mut more = candidates.len() as i64 > scan_limit;
        candidates.truncate(scan_limit as usize);
        let mut usages = Vec::new();
        let mut consumed = 0usize;
        let total = candidates.len();
        let mut last = None;
        for candidate in candidates {
            consumed += 1;
            last = Some((candidate.selected_at, candidate.selection_id));
            match crate::sessions::load(
                &state,
                &mut tx,
                tenant_id,
                candidate.session_id,
                Action::SessionRead,
            )
            .await
            {
                Ok(_) => {}
                Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => continue,
                Err(error) => return Err(error),
            }
            let Some(revision) =
                store::revision(&mut *tx, tenant_id, id, candidate.knowledge_revision_id).await?
            else {
                continue;
            };
            match crate::context_api::authorize_revision(
                &state,
                &mut tx,
                tenant_id,
                &snapshot,
                revision.content.sensitivity,
            )
            .await
            {
                Ok(_) => usages.push(KnowledgeUsageView {
                    context_run_id: candidate.context_run_id,
                    session_id: candidate.session_id,
                    context_selection_id: candidate.selection_id,
                    revision_id: candidate.knowledge_revision_id,
                    selected_at: candidate.selected_at,
                    reason_codes: candidate
                        .reason_codes
                        .into_iter()
                        .map(|reason| reason.as_str().to_owned())
                        .collect(),
                }),
                Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => continue,
                Err(error) => return Err(error),
            }
            if usages.len() as i64 == limit {
                break;
            }
        }
        more |= consumed < total;
        let next_cursor = if more {
            last.map(|(selected_at, selection_id)| {
                encode_usage_cursor(id, selected_at, selection_id)
            })
        } else {
            None
        };
        read_event(
            &mut tx,
            tenant_id,
            "knowledge.usage",
            &allowed,
            Resource::KnowledgeItem(id),
            json!({
                "knowledge_item_id": id,
                "revision_id": snapshot.revision.id,
                "served": usages.len(),
                "more": next_cursor.is_some(),
                "context_runs": usages.iter().map(|usage| json!({
                    "context_run_id": usage.context_run_id,
                    "session_id": usage.session_id,
                    "context_selection_id": usage.context_selection_id,
                    "knowledge_revision_id": usage.revision_id,
                })).collect::<Vec<_>>(),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(KnowledgeUsageListView {
            usages,
            next_cursor,
        }))
    }
    .await;
    respond(&state, "knowledge.usage", result).await
}

async fn mutation_scope(state: &AppState, item_id: KnowledgeItemId) -> Result<ScopeId> {
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let snapshot = snapshot(&mut tx, tenant_id, item_id).await?;
    authorize_snapshot(state, &mut tx, tenant_id, &snapshot).await?;
    Ok(snapshot.item.scope_id)
}

/// `POST /v1/knowledge` — create one governed aggregate and first revision.
#[utoipa::path(
    post,
    path = "/v1/knowledge",
    operation_id = "create_knowledge",
    tag = "knowledge",
    request_body = CreateKnowledgeBody,
    params(("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry.")),
    responses(
        (status = 201, description = "VedaFlow change created", body = KnowledgeMutationView),
        (status = 200, description = "Idempotent replay", body = KnowledgeMutationView),
        (status = 400, description = "Invalid body or missing idempotency key", body = ApiErrorBody),
        (status = 403, description = "The PDP denied Knowledge write or proposal open", body = ApiErrorBody),
        (status = 409, description = "Idempotency key conflict", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.create", skip_all)]
pub(crate) async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: std::result::Result<Json<CreateKnowledgeBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let canonical = json!({"route": "POST /v1/knowledge", "body": &body});
        execute_command(&state, &headers, "knowledge.create", canonical, || {
            let now = Utc::now();
            Ok(KnowledgeCommand::Create {
                item_id: KnowledgeItemId::new(),
                scope_id: body.scope_id,
                project_id: body.project_id,
                owner_principal_id: body.owner_principal_id.clone(),
                knowledge_type: body.knowledge_type.parse()?,
                origin: body.origin.parse()?,
                revision_id: KnowledgeRevisionId::new(),
                content: content(&body.content, now)?,
                sources: sources(&body.sources, body.scope_id)?,
            })
        })
        .await
    }
    .await;
    respond(&state, "knowledge.create", result).await
}

/// `PATCH /v1/knowledge/{id}` — append a governed immutable revision.
#[utoipa::path(
    patch,
    path = "/v1/knowledge/{id}",
    operation_id = "edit_knowledge",
    tag = "knowledge",
    params(
        ("id" = String, Path, description = "Stable Knowledge item id"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry."),
    ),
    request_body = EditKnowledgeBody,
    responses(
        (status = 201, description = "VedaFlow change created", body = KnowledgeMutationView),
        (status = 200, description = "Idempotent replay", body = KnowledgeMutationView),
        (status = 400, description = "Invalid body or missing idempotency key", body = ApiErrorBody),
        (status = 403, description = "The PDP denied the mutation", body = ApiErrorBody),
        (status = 404, description = "No such item in this tenant", body = ApiErrorBody),
        (status = 409, description = "Idempotency or revision conflict", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.edit", skip_all, fields(knowledge.item.id = %id))]
pub(crate) async fn edit(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeItemId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<EditKnowledgeBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let scope_id = mutation_scope(&state, id).await?;
        let canonical = json!({
            "route": "PATCH /v1/knowledge/{id}",
            "knowledge_item_id": id,
            "body": &body,
        });
        execute_command(&state, &headers, "knowledge.edit", canonical, || {
            Ok(KnowledgeCommand::Edit {
                item_id: id,
                expected_revision_id: body.expected_revision_id,
                revision_id: KnowledgeRevisionId::new(),
                content: content(&body.content, Utc::now())?,
                sources: sources(&body.sources, scope_id)?,
            })
        })
        .await
    }
    .await;
    respond(&state, "knowledge.edit", result).await
}

/// `POST /v1/knowledge/{id}/verify` — append verification evidence.
#[utoipa::path(
    post,
    path = "/v1/knowledge/{id}/verify",
    operation_id = "verify_knowledge",
    tag = "knowledge",
    params(
        ("id" = String, Path, description = "Stable Knowledge item id"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry."),
    ),
    request_body = VerifyKnowledgeBody,
    responses(
        (status = 201, description = "VedaFlow change created", body = KnowledgeMutationView),
        (status = 200, description = "Idempotent replay", body = KnowledgeMutationView),
        (status = 400, description = "Invalid body or missing idempotency key", body = ApiErrorBody),
        (status = 403, description = "The PDP denied the mutation", body = ApiErrorBody),
        (status = 404, description = "No such item in this tenant", body = ApiErrorBody),
        (status = 409, description = "Idempotency or revision conflict", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.verify", skip_all, fields(knowledge.item.id = %id))]
pub(crate) async fn verify(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeItemId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<VerifyKnowledgeBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        reject_secrets(&body.verification_metadata)?;
        let canonical = json!({
            "route": "POST /v1/knowledge/{id}/verify",
            "knowledge_item_id": id,
            "body": &body,
        });
        execute_command(&state, &headers, "knowledge.verify", canonical, || {
            Ok(KnowledgeCommand::Verify {
                item_id: id,
                expected_revision_id: body.expected_revision_id,
                revision_id: KnowledgeRevisionId::new(),
                verification_metadata: body.verification_metadata.clone(),
            })
        })
        .await
    }
    .await;
    respond(&state, "knowledge.verify", result).await
}

/// `POST /v1/knowledge/{id}/supersede` — explicitly replace an item.
#[utoipa::path(
    post,
    path = "/v1/knowledge/{id}/supersede",
    operation_id = "supersede_knowledge",
    tag = "knowledge",
    params(
        ("id" = String, Path, description = "Item being replaced"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry."),
    ),
    request_body = SupersedeKnowledgeBody,
    responses(
        (status = 201, description = "VedaFlow change created", body = KnowledgeMutationView),
        (status = 200, description = "Idempotent replay", body = KnowledgeMutationView),
        (status = 400, description = "Invalid body or missing idempotency key", body = ApiErrorBody),
        (status = 403, description = "The PDP denied an input or output", body = ApiErrorBody),
        (status = 404, description = "No such item in this tenant", body = ApiErrorBody),
        (status = 409, description = "Idempotency or revision conflict", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.supersede", skip_all, fields(knowledge.item.id = %id))]
pub(crate) async fn supersede(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeItemId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<SupersedeKnowledgeBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let canonical = json!({
            "route": "POST /v1/knowledge/{id}/supersede",
            "knowledge_item_id": id,
            "body": &body,
        });
        execute_command(&state, &headers, "knowledge.supersede", canonical, || {
            Ok(KnowledgeCommand::Supersede {
                item_id: id,
                expected_revision_id: body.expected_revision_id,
                replacement_item_id: KnowledgeItemId::new(),
                replacement_revision_id: KnowledgeRevisionId::new(),
                scope_id: body.scope_id,
                project_id: body.project_id,
                owner_principal_id: body.owner_principal_id.clone(),
                knowledge_type: body.knowledge_type.parse()?,
                origin: body.origin.parse()?,
                content: content(&body.content, Utc::now())?,
                sources: sources(&body.sources, body.scope_id)?,
            })
        })
        .await
    }
    .await;
    respond(&state, "knowledge.supersede", result).await
}

/// `POST /v1/knowledge/merge` — combine current items and all provenance.
#[utoipa::path(
    post,
    path = "/v1/knowledge/merge",
    operation_id = "merge_knowledge",
    tag = "knowledge",
    params(("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry.")),
    request_body = MergeKnowledgeBody,
    responses(
        (status = 201, description = "VedaFlow change created", body = KnowledgeMutationView),
        (status = 200, description = "Idempotent replay", body = KnowledgeMutationView),
        (status = 400, description = "Invalid body or missing idempotency key", body = ApiErrorBody),
        (status = 403, description = "The PDP denied an input or output", body = ApiErrorBody),
        (status = 404, description = "An input is absent in this tenant", body = ApiErrorBody),
        (status = 409, description = "Idempotency or revision conflict", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.merge", skip_all)]
pub(crate) async fn merge(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: std::result::Result<Json<MergeKnowledgeBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let canonical = json!({"route": "POST /v1/knowledge/merge", "body": &body});
        execute_command(&state, &headers, "knowledge.merge", canonical, || {
            Ok(KnowledgeCommand::Merge {
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
                scope_id: body.scope_id,
                project_id: body.project_id,
                owner_principal_id: body.owner_principal_id.clone(),
                knowledge_type: body.knowledge_type.parse()?,
                origin: body.origin.parse()?,
                content: content(&body.content, Utc::now())?,
                sources: Vec::new(),
            })
        })
        .await
    }
    .await;
    respond(&state, "knowledge.merge", result).await
}

async fn lifecycle_command(
    state: &AppState,
    headers: &HeaderMap,
    item_id: KnowledgeItemId,
    body: &LifecycleKnowledgeBody,
    operation: &'static str,
    route: &'static str,
    restore: bool,
) -> Result<(StatusCode, Json<KnowledgeMutationView>)> {
    let canonical = json!({
        "route": route,
        "knowledge_item_id": item_id,
        "body": body,
    });
    execute_command(state, headers, operation, canonical, || {
        if restore {
            Ok(KnowledgeCommand::Restore {
                item_id,
                expected_revision_id: body.expected_revision_id,
                reason: body.reason.clone(),
            })
        } else {
            Ok(KnowledgeCommand::Archive {
                item_id,
                expected_revision_id: body.expected_revision_id,
                reason: body.reason.clone(),
            })
        }
    })
    .await
}

/// `POST /v1/knowledge/{id}/archive`.
#[utoipa::path(
    post,
    path = "/v1/knowledge/{id}/archive",
    operation_id = "archive_knowledge",
    tag = "knowledge",
    params(
        ("id" = String, Path, description = "Stable Knowledge item id"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry."),
    ),
    request_body = LifecycleKnowledgeBody,
    responses(
        (status = 201, description = "VedaFlow change created", body = KnowledgeMutationView),
        (status = 200, description = "Idempotent replay", body = KnowledgeMutationView),
        (status = 400, description = "Invalid body or missing idempotency key", body = ApiErrorBody),
        (status = 403, description = "The PDP denied the mutation", body = ApiErrorBody),
        (status = 404, description = "No such item in this tenant", body = ApiErrorBody),
        (status = 409, description = "Idempotency or revision conflict", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.archive", skip_all, fields(knowledge.item.id = %id))]
pub(crate) async fn archive(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeItemId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<LifecycleKnowledgeBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        lifecycle_command(
            &state,
            &headers,
            id,
            &body,
            "knowledge.archive",
            "POST /v1/knowledge/{id}/archive",
            false,
        )
        .await
    }
    .await;
    respond(&state, "knowledge.archive", result).await
}

/// `POST /v1/knowledge/{id}/restore`.
#[utoipa::path(
    post,
    path = "/v1/knowledge/{id}/restore",
    operation_id = "restore_knowledge",
    tag = "knowledge",
    params(
        ("id" = String, Path, description = "Stable Knowledge item id"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry."),
    ),
    request_body = LifecycleKnowledgeBody,
    responses(
        (status = 201, description = "VedaFlow change created", body = KnowledgeMutationView),
        (status = 200, description = "Idempotent replay", body = KnowledgeMutationView),
        (status = 400, description = "Invalid body or missing idempotency key", body = ApiErrorBody),
        (status = 403, description = "The PDP denied the mutation", body = ApiErrorBody),
        (status = 404, description = "No such item in this tenant", body = ApiErrorBody),
        (status = 409, description = "Idempotency or revision conflict", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.restore", skip_all, fields(knowledge.item.id = %id))]
pub(crate) async fn restore(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeItemId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<LifecycleKnowledgeBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        lifecycle_command(
            &state,
            &headers,
            id,
            &body,
            "knowledge.restore",
            "POST /v1/knowledge/{id}/restore",
            true,
        )
        .await
    }
    .await;
    respond(&state, "knowledge.restore", result).await
}

/// `DELETE /v1/knowledge/{id}` — explicit archive or governed forget.
#[utoipa::path(
    delete,
    path = "/v1/knowledge/{id}",
    operation_id = "delete_knowledge",
    tag = "knowledge",
    params(
        ("id" = String, Path, description = "Stable Knowledge item id"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry."),
    ),
    request_body = DeleteKnowledgeBody,
    responses(
        (status = 201, description = "VedaFlow change created", body = KnowledgeMutationView),
        (status = 200, description = "Idempotent replay", body = KnowledgeMutationView),
        (status = 400, description = "Missing/invalid mode, body or idempotency key", body = ApiErrorBody),
        (status = 403, description = "The PDP denied archive or forget", body = ApiErrorBody),
        (status = 404, description = "No such item in this tenant", body = ApiErrorBody),
        (status = 409, description = "Idempotency or revision conflict", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.delete", skip_all, fields(knowledge.item.id = %id))]
pub(crate) async fn delete(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeItemId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<DeleteKnowledgeBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let operation = match body.mode.as_str() {
            "archive" => "knowledge.delete.archive",
            "forget" => "knowledge.delete.forget",
            other => {
                return Err(Error::Invalid {
                    message: format!(
                        "DELETE Knowledge mode must be `archive` or `forget`, got {other:?}"
                    ),
                });
            }
        };
        let canonical = json!({
            "route": "DELETE /v1/knowledge/{id}",
            "knowledge_item_id": id,
            "body": &body,
        });
        execute_command(&state, &headers, operation, canonical, || {
            if body.mode == "archive" {
                Ok(KnowledgeCommand::Archive {
                    item_id: id,
                    expected_revision_id: body.expected_revision_id,
                    reason: body.reason.clone(),
                })
            } else {
                Ok(KnowledgeCommand::Forget {
                    item_id: id,
                    expected_revision_id: body.expected_revision_id,
                    reason: body.reason.clone(),
                })
            }
        })
        .await
    }
    .await;
    respond(&state, "knowledge.delete", result).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_is_bound_to_filters_and_mode() {
        let digest = "a".repeat(64);
        let item_id = KnowledgeItemId::new();
        let at = Utc::now();
        let list = encode_cursor(
            PageCursor::List(ListCursor {
                updated_at: at,
                item_id,
            }),
            &digest,
        );
        assert!(matches!(
            decode_cursor(&list, &digest, false).expect("list"),
            PageCursor::List(_)
        ));
        assert!(decode_cursor(&list, &digest, true).is_err());
        assert!(decode_cursor(&list, &"b".repeat(64), false).is_err());
    }

    #[test]
    fn rrf_is_stable_and_rewards_two_legs() {
        let both = KnowledgeItemId::new();
        let lexical_only = KnowledgeItemId::new();
        let at = Utc::now();
        let result = fuse(
            &[
                Candidate {
                    item_id: lexical_only,
                    updated_at: at,
                    score: 1.0,
                },
                Candidate {
                    item_id: both,
                    updated_at: at,
                    score: 0.5,
                },
            ],
            &[Candidate {
                item_id: both,
                updated_at: at,
                score: 0.9,
            }],
        );
        assert_eq!(result[0].item_id, both);
        assert!(result[0].score > result[1].score);
    }

    #[test]
    fn url_sources_reject_userinfo_without_echoing_it() {
        let secret = "ghp_012345678901234567890123456789012345";
        let source = KnowledgeSourceBody {
            scope_id: None,
            source_type: "url".to_owned(),
            session_event_id: None,
            locator: Some(format!("https://token:{secret}@example.test/docs")),
            source_revision: None,
            content_hash: None,
            metadata: json!({}),
        };
        let error = sources(&[source], ScopeId::new()).expect_err("credentials refused");
        assert!(!error.to_string().contains(secret));
    }
}
