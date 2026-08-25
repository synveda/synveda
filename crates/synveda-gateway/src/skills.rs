//! Versioned Agent Skills catalogue and public API (CPR-23, ADR-0085).
//!
//! A catalogue entry has a stable id, immutable versions and explicit
//! project/principal bindings. Every install, update and binding transition is
//! a typed VedaFlow `Skill/apply` change. Bundle bytes remain content-addressed
//! VedaFlow objects; neither this module nor the gateway executes bundled code.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Acquire, PgConnection};
use synveda_audit::{AuditAction, Outcome};
use synveda_ingest::{BundleScan, RubricScore, ScanOutcome};
use synveda_policy::{Action, Resource};
use synveda_store::anchors::AnchorSelection;
use synveda_store::skills::{
    self as store, ResolvedBinding, StoredBinding, StoredSkill, StoredTestRun, StoredUsageEvent,
    StoredVersion, StoredVersionFile,
};
use synveda_store::{configuration, rls, scopes};
use synveda_types::json::canonicalise;
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{
    AssetKind, Error, IdentityId, IdentityKind, ProposalEffect, ProposalId, ProposalState, Result,
    ScanSeverity, ScopeId, Sensitivity, SessionId, SkillBindingId, SkillBundle, SkillCommand,
    SkillFile, SkillFilePath, SkillId, SkillMutationOutcome, SkillMutationResult, SkillName,
    SkillProvenance, SkillTestHarness, SkillTestOutcome, SkillTestRunId, SkillUsageEventId,
    SkillUsageEvidence, SkillUsageStage, SkillVersionFileRef, SkillVersionId, TenantId,
    validate_skill_usage_client_event_id,
};
use synveda_vedaflow::{self as vedaflow, PolicySnapshot, Signer, SkillAsset};
use utoipa::ToSchema;

use crate::app::AppState;
use crate::approvals::{self, Requested};
use crate::audit;
use crate::authz::{self, Authorized, DecisionInput};
use crate::error::ApiError;
use crate::idempotency::{Claim, Dispatch};
use crate::request::{body, commit, found, tenant_id};
use crate::workspaces::{ApiErrorBody, string_enum, subject};

/// Skill API outcomes by operation and `ok|rejected|error`.
pub const SKILL_OPERATIONS_TOTAL: &str = "synveda_skill_operations_total";

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;
const VALIDATION_HARNESS_VERSION: &str = "synveda-validation-sandbox/1";

fn sensitivity_schema() -> utoipa::openapi::schema::Object {
    string_enum(Sensitivity::ALL.iter().map(|value| value.as_str()))
}

fn mutation_outcome_schema() -> utoipa::openapi::schema::Object {
    string_enum(["applied", "pending_review", "rejected"].into_iter())
}

fn source_kind_schema() -> utoipa::openapi::schema::Object {
    string_enum(
        synveda_types::SkillSourceKind::ALL
            .iter()
            .map(|value| value.as_str()),
    )
}

fn usage_stage_schema() -> utoipa::openapi::schema::Object {
    string_enum(SkillUsageStage::ALL.iter().map(|value| value.as_str()))
}

fn usage_evidence_schema() -> utoipa::openapi::schema::Object {
    string_enum(SkillUsageEvidence::ALL.iter().map(|value| value.as_str()))
}

fn test_harness_schema() -> utoipa::openapi::schema::Object {
    string_enum(SkillTestHarness::ALL.iter().map(|value| value.as_str()))
}

fn test_outcome_schema() -> utoipa::openapi::schema::Object {
    string_enum(SkillTestOutcome::ALL.iter().map(|value| value.as_str()))
}

/// One file supplied for an immutable version.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillFileBody {
    /// Bundle-relative path.
    pub path: String,
    /// UTF-8 text stored and installed byte-for-byte.
    pub content: String,
}

/// Provenance supplied for an imported or authored bundle.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillProvenanceBody {
    /// Source class.
    #[schema(schema_with = source_kind_schema)]
    pub kind: String,
    /// Non-secret source reference.
    #[serde(default)]
    pub reference: Option<String>,
    /// Exact upstream revision, when present.
    #[serde(default)]
    pub revision: Option<String>,
    /// Forward-compatible source metadata.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub metadata: Value,
}

impl Default for SkillProvenanceBody {
    fn default() -> Self {
        Self {
            kind: synveda_types::SkillSourceKind::Authored.as_str().to_owned(),
            reference: None,
            revision: None,
            metadata: Value::Object(Default::default()),
        }
    }
}

impl TryFrom<SkillProvenanceBody> for SkillProvenance {
    type Error = Error;

    fn try_from(value: SkillProvenanceBody) -> Result<Self> {
        let provenance = Self {
            kind: value.kind.parse()?,
            reference: value.reference,
            revision: value.revision,
            metadata: value.metadata,
        };
        provenance.validate()?;
        Ok(provenance)
    }
}

/// Install the first immutable version of a stable skill.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallSkillBody {
    /// Scope governing the catalogue aggregate.
    #[schema(value_type = String, format = "uuid")]
    pub governing_scope_id: ScopeId,
    /// Agent Skills bundle name.
    pub name: String,
    /// Classification of every file in this version.
    #[schema(schema_with = sensitivity_schema)]
    pub sensitivity: String,
    /// Whole Agent Skills-compatible bundle.
    pub files: Vec<SkillFileBody>,
    /// Retained bundle provenance.
    #[serde(default)]
    pub provenance: SkillProvenanceBody,
}

/// Add and select a new immutable version.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateSkillBody {
    /// Exact current version required when the change applies.
    #[schema(value_type = String, format = "uuid")]
    pub expected_current_version_id: SkillVersionId,
    /// Classification of every file in this version.
    #[schema(schema_with = sensitivity_schema)]
    pub sensitivity: String,
    /// Complete replacement bundle; history remains immutable.
    pub files: Vec<SkillFileBody>,
    /// Retained bundle provenance.
    #[serde(default)]
    pub provenance: SkillProvenanceBody,
}

/// Create a project- or principal-scope binding.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSkillBindingBody {
    /// Project or principal scope receiving the binding.
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    /// Stable catalogue entry.
    #[schema(value_type = String, format = "uuid")]
    pub skill_id: SkillId,
    /// Exact version pin; absent follows the current version.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub pinned_version_id: Option<SkillVersionId>,
    /// Initial activation state.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Change a binding using optimistic concurrency.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateSkillBindingBody {
    /// Exact binding revision required when the change applies.
    pub expected_revision: u64,
    /// Complete resulting activation state.
    pub enabled: bool,
    /// Complete resulting pin state; null follows current.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub pinned_version_id: Option<SkillVersionId>,
    /// Stable reason code (`disable`, `enable`, `pin`, `unpin`).
    pub reason: String,
}

/// Roll a binding back by pinning an older immutable version.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RollbackSkillBindingBody {
    /// Exact binding revision required when the change applies.
    pub expected_revision: u64,
    /// Older version of the same skill.
    #[schema(value_type = String, format = "uuid")]
    pub version_id: SkillVersionId,
}

/// Append one idempotent usage observation.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordSkillUsageBody {
    /// Active binding observed.
    #[schema(value_type = String, format = "uuid")]
    pub binding_id: SkillBindingId,
    /// Exact immutable version involved.
    #[schema(value_type = String, format = "uuid")]
    pub version_id: SkillVersionId,
    /// Session carrying the event, when applicable.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub session_id: Option<SessionId>,
    /// Client idempotency key.
    pub client_event_id: String,
    /// Observable lifecycle stage.
    #[schema(schema_with = usage_stage_schema)]
    pub stage: String,
    /// Whether a host observed the act or a model reported it.
    #[schema(schema_with = usage_evidence_schema)]
    pub evidence: String,
    /// Resource/script path for the stages that name one.
    #[serde(default)]
    pub resource_path: Option<String>,
    /// Bounded, content-free evidence.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub metadata: Value,
    /// Client occurrence time.
    pub occurred_at: DateTime<Utc>,
}

/// Run the non-executing built-in validation sandbox.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RunSkillTestBody {
    /// Only `validation_sandbox` runs inside the gateway.
    #[schema(schema_with = test_harness_schema)]
    pub harness: String,
}

/// Stable result envelope for every governed Skill mutation.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SkillMutationView {
    /// VedaFlow change id.
    #[schema(value_type = String, format = "uuid")]
    pub change_id: ProposalId,
    /// Governance result.
    #[schema(schema_with = mutation_outcome_schema)]
    pub outcome: String,
    /// Stable Skill aggregate affected by the change.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub skill_id: Option<SkillId>,
    /// Immutable version created or selected by the change.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub version_id: Option<SkillVersionId>,
    /// Revisioned binding created or changed by the change.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub binding_id: Option<SkillBindingId>,
    /// Binding revision produced by the change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding_revision: Option<u64>,
}

impl From<SkillMutationResult> for SkillMutationView {
    fn from(value: SkillMutationResult) -> Self {
        let outcome = match value.outcome {
            SkillMutationOutcome::Applied => "applied",
            SkillMutationOutcome::PendingReview => "pending_review",
            SkillMutationOutcome::Rejected => "rejected",
        };
        Self {
            change_id: value.change_id,
            outcome: outcome.to_owned(),
            skill_id: value.skill_id,
            version_id: value.version_id,
            binding_id: value.binding_id,
            binding_revision: value.binding_revision,
        }
    }
}

/// Immutable version metadata. File bytes use the dedicated file route.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SkillVersionView {
    /// Immutable version identifier.
    #[schema(value_type = String, format = "uuid")]
    pub id: SkillVersionId,
    /// Stable Skill aggregate identifier.
    #[schema(value_type = String, format = "uuid")]
    pub skill_id: SkillId,
    /// Monotonic version number within the aggregate.
    pub ordinal: u64,
    /// Stable digest over exact bundle paths and object addresses.
    pub bundle_digest: String,
    /// Governing sensitivity classification.
    #[schema(schema_with = sensitivity_schema)]
    pub sensitivity: String,
    /// Parsed Agent Skills manifest with extension metadata preserved.
    #[schema(value_type = Object)]
    pub manifest: Value,
    /// Provenance source class.
    #[schema(schema_with = source_kind_schema)]
    pub source_kind: String,
    /// Version-specific provenance evidence.
    #[schema(value_type = Object)]
    pub provenance: Value,
    /// Content-free scanner evidence.
    #[schema(value_type = Object)]
    pub scan: Value,
    /// Scanner ruleset that produced the evidence.
    pub scan_ruleset_version: u32,
    /// Automated quality score from zero through one hundred.
    pub quality_score: u8,
    /// Rubric version that produced the score.
    pub rubric_version: u32,
    /// Declared tools are metadata and grant no authority.
    pub declared_tools_are_authorization: bool,
    /// Immutable version creation time.
    pub created_at: DateTime<Utc>,
    /// Principal that created the version through VedaFlow.
    #[schema(value_type = String, format = "uuid")]
    pub created_by: IdentityId,
}

impl TryFrom<StoredVersion> for SkillVersionView {
    type Error = Error;

    fn try_from(value: StoredVersion) -> Result<Self> {
        Ok(Self {
            id: value.id,
            skill_id: value.skill_id,
            ordinal: value.ordinal,
            bundle_digest: store::hex_32(&value.bundle_digest),
            sensitivity: value.sensitivity.as_str().to_owned(),
            manifest: value.manifest,
            source_kind: value.source_kind.as_str().to_owned(),
            provenance: serde_json::to_value(value.provenance).map_err(|err| Error::Internal {
                message: format!("encode stored skill provenance: {err}"),
            })?,
            scan: value.scan_report,
            scan_ruleset_version: value.scan_ruleset_version,
            quality_score: value.quality_score,
            rubric_version: value.rubric_version,
            declared_tools_are_authorization: false,
            created_at: value.created_at,
            created_by: value.created_by,
        })
    }
}

/// Stable skill head and its current immutable version.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SkillView {
    /// Stable Skill aggregate identifier.
    #[schema(value_type = String, format = "uuid")]
    pub id: SkillId,
    /// Scope governing installation and updates.
    #[schema(value_type = String, format = "uuid")]
    pub governing_scope_id: ScopeId,
    /// Tenant-unique Agent Skills bundle name.
    pub name: String,
    /// Current immutable version pointer.
    #[schema(value_type = String, format = "uuid")]
    pub current_version_id: SkillVersionId,
    /// Current immutable version metadata.
    pub current_version: SkillVersionView,
    /// Aggregate creation time.
    pub created_at: DateTime<Utc>,
    /// Principal that installed the aggregate.
    #[schema(value_type = String, format = "uuid")]
    pub created_by: IdentityId,
    /// Last current-pointer update time.
    pub updated_at: DateTime<Utc>,
    /// Principal that last advanced the current pointer.
    #[schema(value_type = String, format = "uuid")]
    pub updated_by: IdentityId,
}

/// Cursor-paginated catalogue page.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SkillListView {
    /// Policy-visible catalogue entries.
    pub skills: Vec<SkillView>,
    /// Cursor after the last candidate considered.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub next_cursor: Option<SkillId>,
}

/// One immutable file descriptor.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SkillVersionFileView {
    /// Relative bundle path.
    pub path: String,
    /// Content-addressed VedaFlow object hash.
    pub object_hash: String,
    /// Unicode scalar count retained for bounded clients.
    pub chars: u32,
    /// File-reference creation time.
    pub created_at: DateTime<Utc>,
}

impl From<StoredVersionFile> for SkillVersionFileView {
    fn from(value: StoredVersionFile) -> Self {
        Self {
            path: value.path.to_string(),
            object_hash: store::hex_32(&value.object_hash),
            chars: value.chars,
            created_at: value.created_at,
        }
    }
}

/// One authorised file with its exact immutable content.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SkillVersionFileContentView {
    /// Exact immutable version containing the file.
    #[schema(value_type = String, format = "uuid")]
    pub version_id: SkillVersionId,
    /// Relative bundle path.
    pub path: String,
    /// Content-addressed VedaFlow object hash.
    pub object_hash: String,
    /// Exact authorised text content.
    pub content: String,
}

/// One revisioned binding.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SkillBindingView {
    /// Stable binding identifier.
    #[schema(value_type = String, format = "uuid")]
    pub id: SkillBindingId,
    /// Bound project or principal scope.
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    /// Bound Skill aggregate.
    #[schema(value_type = String, format = "uuid")]
    pub skill_id: SkillId,
    /// Exact version pin, or absent to follow the current pointer.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub pinned_version_id: Option<SkillVersionId>,
    /// Whether sessions may discover this binding.
    pub enabled: bool,
    /// Optimistic-concurrency revision.
    pub revision: u64,
    /// Binding creation time.
    pub created_at: DateTime<Utc>,
    /// Principal that created the binding.
    #[schema(value_type = String, format = "uuid")]
    pub created_by: IdentityId,
    /// Last binding transition time.
    pub updated_at: DateTime<Utc>,
    /// Principal that made the last binding transition.
    #[schema(value_type = String, format = "uuid")]
    pub updated_by: IdentityId,
}

impl From<StoredBinding> for SkillBindingView {
    fn from(value: StoredBinding) -> Self {
        Self {
            id: value.id,
            scope_id: value.scope_id,
            skill_id: value.skill_id,
            pinned_version_id: value.pinned_version_id,
            enabled: value.enabled,
            revision: value.revision,
            created_at: value.created_at,
            created_by: value.created_by,
            updated_at: value.updated_at,
            updated_by: value.updated_by,
        }
    }
}

/// Binding collection.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SkillBindingListView {
    /// Policy-visible bindings.
    pub bindings: Vec<SkillBindingView>,
    /// Cursor after the last candidate considered.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub next_cursor: Option<SkillBindingId>,
}

/// Exact version made available by one enabled binding.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AvailableSkillView {
    /// Binding that makes this version available.
    pub binding: SkillBindingView,
    /// Agent Skills bundle name.
    pub name: String,
    /// Exact version resolved from the binding.
    pub version: SkillVersionView,
    /// Content-addressed SKILL.md object.
    pub manifest_object_hash: String,
}

/// Visible available skills after binding and PDP evaluation.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AvailableSkillListView {
    /// Project or principal scope resolved for the session.
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    /// Enabled and policy-visible exact versions.
    pub skills: Vec<AvailableSkillView>,
}

/// One append-only usage event.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SkillUsageEventView {
    /// Stable append-only usage event identifier.
    #[schema(value_type = String, format = "uuid")]
    pub id: SkillUsageEventId,
    /// Binding that advertised the version.
    #[schema(value_type = String, format = "uuid")]
    pub binding_id: SkillBindingId,
    /// Exact immutable version involved.
    #[schema(value_type = String, format = "uuid")]
    pub version_id: SkillVersionId,
    /// Governed session, when the lifecycle event occurred in one.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub session_id: Option<SessionId>,
    /// Principal associated with the lifecycle event.
    #[schema(value_type = String, format = "uuid")]
    pub principal_id: IdentityId,
    /// Adapter-provided idempotency key.
    pub client_event_id: String,
    /// Observable lifecycle stage.
    #[schema(schema_with = usage_stage_schema)]
    pub stage: String,
    /// Host-observed or model-reported evidence class.
    #[schema(schema_with = usage_evidence_schema)]
    pub evidence: String,
    /// Resource or script path, when the stage names one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_path: Option<String>,
    /// Bounded content-free evidence.
    #[schema(value_type = Object)]
    pub metadata: Value,
    /// Client occurrence time.
    pub occurred_at: DateTime<Utc>,
    /// Server receipt time.
    pub received_at: DateTime<Utc>,
}

impl From<StoredUsageEvent> for SkillUsageEventView {
    fn from(value: StoredUsageEvent) -> Self {
        Self {
            id: value.id,
            binding_id: value.binding_id,
            version_id: value.version_id,
            session_id: value.session_id,
            principal_id: value.principal_id,
            client_event_id: value.client_event_id,
            stage: value.stage.as_str().to_owned(),
            evidence: value.evidence.as_str().to_owned(),
            resource_path: value.resource_path.map(|path| path.to_string()),
            metadata: value.metadata,
            occurred_at: value.occurred_at,
            received_at: value.received_at,
        }
    }
}

/// Usage collection.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SkillUsageListView {
    /// Append-only usage evidence.
    pub events: Vec<SkillUsageEventView>,
    /// Cursor after the last returned event.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub next_cursor: Option<SkillUsageEventId>,
}

/// One immutable controlled-harness result.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SkillTestRunView {
    /// Stable test-run identifier.
    #[schema(value_type = String, format = "uuid")]
    pub id: SkillTestRunId,
    /// Exact immutable version tested.
    #[schema(value_type = String, format = "uuid")]
    pub version_id: SkillVersionId,
    /// Controlled harness class.
    #[schema(schema_with = test_harness_schema)]
    pub harness: String,
    /// Exact harness implementation version.
    pub harness_version: String,
    /// Terminal test outcome.
    #[schema(schema_with = test_outcome_schema)]
    pub outcome: String,
    /// Scanner ruleset used by the run.
    pub scan_ruleset_version: u32,
    /// Quality rubric used by the run.
    pub rubric_version: u32,
    /// Content-free validation evidence.
    #[schema(value_type = Object)]
    pub evidence: Value,
    /// Test-run creation time.
    pub created_at: DateTime<Utc>,
    /// Principal that requested the test.
    #[schema(value_type = String, format = "uuid")]
    pub created_by: IdentityId,
}

impl From<StoredTestRun> for SkillTestRunView {
    fn from(value: StoredTestRun) -> Self {
        Self {
            id: value.id,
            version_id: value.version_id,
            harness: value.harness.as_str().to_owned(),
            harness_version: value.harness_version,
            outcome: value.outcome.as_str().to_owned(),
            scan_ruleset_version: value.scan_ruleset_version,
            rubric_version: value.rubric_version,
            evidence: value.evidence,
            created_at: value.created_at,
            created_by: value.created_by,
        }
    }
}

/// Test-run collection.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SkillTestRunListView {
    /// Immutable controlled-harness results.
    pub runs: Vec<SkillTestRunView>,
    /// Cursor after the last returned run.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub next_cursor: Option<SkillTestRunId>,
}

fn default_true() -> bool {
    true
}

// ── Bundle validation and retained evidence ───────────────────────────────

async fn scan_file(file: &SkillFile) -> Result<ScanOutcome> {
    let payload = json!({"path": file.path.as_str(), "content": file.content});
    let span = tracing::Span::current();
    tokio::task::spawn_blocking(move || {
        let _entered = span.enter();
        synveda_ingest::scan(payload)
    })
    .await
    .map_err(|err| Error::Internal {
        message: format!("skill redaction scan task failed: {err}"),
    })
}

/// Runs the executable-content scanner over every bundle file.
pub(crate) async fn scan_security(files: &[SkillFile]) -> Result<BundleScan> {
    let files = files.to_vec();
    let span = tracing::Span::current();
    tokio::task::spawn_blocking(move || {
        let _entered = span.enter();
        synveda_ingest::scan_bundle(&files)
    })
    .await
    .map_err(|err| Error::Internal {
        message: format!("skill security scan task failed: {err}"),
    })
}

/// Runs the deterministic quality rubric over exact bundle bytes.
pub(crate) async fn score_quality(files: &[SkillFile]) -> Result<RubricScore> {
    let files = files.to_vec();
    let span = tracing::Span::current();
    tokio::task::spawn_blocking(move || {
        let _entered = span.enter();
        synveda_ingest::score_bundle(&files)
    })
    .await
    .map_err(|err| Error::Internal {
        message: format!("skill quality scoring task failed: {err}"),
    })
}

fn scan_payload(scan: &BundleScan, threshold: ScanSeverity) -> Value {
    json!({
        "ruleset_version": scan.ruleset_version,
        "worst": scan.worst().map(|value| value.as_str()),
        "blocks_at": threshold.as_str(),
        "findings": scan.files.iter().flat_map(|file| {
            file.findings.iter().map(move |finding| json!({
                "path": file.path,
                "rule": finding.rule,
                "severity": finding.severity.as_str(),
                "line": finding.line,
                "count": finding.count,
            }))
        }).collect::<Vec<_>>(),
    })
}

/// Refuse prose fields that would put a credential in governance metadata.
pub(crate) async fn refuse_if_secret(what: &str, text: &str) -> Result<()> {
    let payload = json!({"content": text});
    let span = tracing::Span::current();
    let scan = tokio::task::spawn_blocking(move || {
        let _entered = span.enter();
        synveda_ingest::scan(payload)
    })
    .await
    .map_err(|err| Error::Internal {
        message: format!("redaction scan task failed: {err}"),
    })?;
    if scan.findings.is_empty() {
        return Ok(());
    }
    Err(Error::Invalid {
        message: format!("the {what} contains material classified as a secret and is not stored"),
    })
}

/// Digest exact `(path, object address)` pairs in stable path order.
fn bundle_digest(files: &[SkillVersionFileRef]) -> Result<[u8; 32]> {
    let mut parsed = files
        .iter()
        .map(|file| {
            Ok((
                file.path.to_string(),
                vedaflow::hash::ObjectHash::from_bytes(store::decode_hex_32(
                    &file.object_hash,
                    "skill file object address",
                )?),
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    parsed.sort_by(|left, right| left.0.cmp(&right.0));
    let borrowed = parsed
        .iter()
        .map(|(path, hash)| (path.as_str(), *hash))
        .collect::<Vec<_>>();
    Ok(vedaflow::bundle_digest(&borrowed))
}

struct PreparedBundle {
    manifest: Value,
    files: Vec<SkillVersionFileRef>,
    bundle_digest: String,
    scan: Value,
    scan_ruleset_version: u32,
    quality_score: u8,
    rubric_version: u32,
}

async fn prepare_bundle(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    authorization: &CommandAuthorization,
    name: SkillName,
    sensitivity: Sensitivity,
    supplied: Vec<SkillFileBody>,
) -> Result<PreparedBundle> {
    let bundle = SkillBundle {
        name,
        files: supplied
            .into_iter()
            .map(|file| {
                Ok(SkillFile {
                    path: file.path.parse()?,
                    content: file.content,
                })
            })
            .collect::<Result<Vec<_>>>()?,
    };
    let frontmatter = bundle.validate()?;
    for file in &bundle.files {
        let redaction = scan_file(file).await?;
        if !redaction.findings.is_empty() {
            return Err(Error::Invalid {
                message: format!(
                    "{} was stopped by the redaction scanner; the bundle was not stored",
                    file.path
                ),
            });
        }
    }
    let effective = state.pdp.effective(
        tenant,
        Resource::Scope(authorization.target.id),
        &authorization.input.context(),
    );
    let security = scan_security(&bundle.files).await?;
    if security.blocked_by(&effective.scan) {
        let worst = security.worst().unwrap_or(ScanSeverity::Critical);
        return Err(Error::Invalid {
            message: format!(
                "skill {} was refused by the security scanning gate at {worst}; the bundle was not stored",
                bundle.name
            ),
        });
    }
    let quality = score_quality(&bundle.files).await?;
    let scan = scan_payload(&security, effective.scan.threshold());
    let mut refs = Vec::with_capacity(bundle.files.len());
    for file in bundle.files {
        let chars = u32::try_from(file.content.chars().count()).unwrap_or(u32::MAX);
        let path = file.path.clone();
        let object = vedaflow::put_skill(
            tx,
            tenant,
            &SkillAsset {
                scope_id: authorization.target.id,
                skill: bundle.name.clone(),
                sensitivity,
                file,
            },
        )
        .await?;
        refs.push(SkillVersionFileRef {
            path,
            object_hash: object.hash.to_hex(),
            chars,
        });
    }
    refs.sort_by(|left, right| left.path.cmp(&right.path));
    let digest = bundle_digest(&refs)?;
    Ok(PreparedBundle {
        manifest: serde_json::to_value(frontmatter).map_err(|err| Error::Internal {
            message: format!("encode validated skill manifest: {err}"),
        })?,
        files: refs,
        bundle_digest: store::hex_32(&digest),
        scan,
        scan_ruleset_version: security.ruleset_version,
        quality_score: quality.score,
        rubric_version: quality.rubric_version,
    })
}

// ── Governed command layer ────────────────────────────────────────────────

struct CommandAuthorization {
    target: Scope,
    input: DecisionInput,
    write_allowed: Authorized,
    proposal_allowed: Authorized,
}

struct AppliedSkillEffect {
    skill_id: Option<SkillId>,
    version_id: Option<SkillVersionId>,
    binding_id: Option<SkillBindingId>,
    binding_revision: Option<u64>,
}

fn identity_of(input: &DecisionInput, act: &str) -> Result<IdentityId> {
    input
        .identity
        .as_ref()
        .map(|identity| identity.id)
        .ok_or_else(|| Error::Invalid {
            message: format!("{act} requires a provisioned identity"),
        })
}

async fn scope_for(tx: &mut PgConnection, tenant: TenantId, scope_id: ScopeId) -> Result<Scope> {
    found(
        scopes::get(&mut *tx, tenant, scope_id).await?,
        tenant,
        scope_id,
    )
}

async fn read_skill_and_version(
    tx: &mut PgConnection,
    tenant: TenantId,
    skill_id: SkillId,
    version_id: Option<SkillVersionId>,
) -> Result<(StoredSkill, StoredVersion)> {
    let skill = store::by_id(&mut *tx, tenant, skill_id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("skill {skill_id}"),
        })?;
    let version_id = version_id.unwrap_or(skill.current_version_id);
    let version = store::version(&mut *tx, tenant, version_id)
        .await?
        .filter(|version| version.skill_id == skill.id)
        .ok_or_else(|| Error::NotFound {
            entity: format!("skill version {version_id}"),
        })?;
    Ok((skill, version))
}

async fn authorize_skill_read(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    skill: &StoredSkill,
    version: &StoredVersion,
) -> Result<Authorized> {
    let scope = scope_for(tx, tenant, skill.governing_scope_id).await?;
    let input = authz::gather(state, tx, Some(&scope), AnchorSelection::none(), Vec::new()).await?;
    authz::decide_skill_read(
        state,
        &input,
        Resource::Scope(scope.id),
        version.sensitivity,
    )
}

async fn authorize_command(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    command: &SkillCommand,
) -> Result<CommandAuthorization> {
    let target = scope_for(tx, tenant, command.scope_id()).await?;
    if matches!(
        command,
        SkillCommand::Bind { .. } | SkillCommand::SetBinding { .. }
    ) && !matches!(target.kind, ScopeKind::Project | ScopeKind::Principal)
    {
        return Err(Error::Invalid {
            message: "a skill binding targets a project or principal scope".to_owned(),
        });
    }
    let input = authz::gather(
        state,
        tx,
        Some(&target),
        AnchorSelection::none(),
        Vec::new(),
    )
    .await?;
    let write_allowed = authz::decide(
        state,
        &input,
        Action::SkillWrite,
        Resource::Scope(target.id),
    )?;
    let proposal_allowed = authz::decide(
        state,
        &input,
        Action::ProposalOpen,
        Resource::Scope(target.id),
    )?;

    let existing = match command {
        SkillCommand::Install { .. } => None,
        SkillCommand::Update {
            skill_id,
            governing_scope_id,
            name,
            expected_current_version_id,
            ..
        } => {
            let (skill, version) =
                read_skill_and_version(tx, tenant, *skill_id, Some(*expected_current_version_id))
                    .await?;
            if skill.governing_scope_id != *governing_scope_id || &skill.name != name {
                return Err(Error::NotFound {
                    entity: format!("skill {skill_id}"),
                });
            }
            Some((skill, version))
        }
        SkillCommand::Bind {
            skill_id,
            pinned_version_id,
            ..
        } => Some(read_skill_and_version(tx, tenant, *skill_id, *pinned_version_id).await?),
        SkillCommand::SetBinding {
            binding_id,
            scope_id,
            pinned_version_id,
            ..
        } => {
            let binding = store::binding(&mut *tx, tenant, *binding_id)
                .await?
                .filter(|binding| binding.scope_id == *scope_id)
                .ok_or_else(|| Error::NotFound {
                    entity: format!("skill binding {binding_id}"),
                })?;
            Some(read_skill_and_version(tx, tenant, binding.skill_id, *pinned_version_id).await?)
        }
    };
    if let Some((skill, version)) = existing {
        authorize_skill_read(state, tx, tenant, &skill, &version).await?;
    }
    Ok(CommandAuthorization {
        target,
        input,
        write_allowed,
        proposal_allowed,
    })
}

fn command_payload_hash(command: &SkillCommand) -> Result<String> {
    let value = canonicalise(
        &serde_json::to_value(command).map_err(|err| Error::Invalid {
            message: format!("encode Skill command: {err}"),
        })?,
    );
    Ok(blake3::hash(value.to_string().as_bytes())
        .to_hex()
        .to_string())
}

async fn open_command(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    command: &SkillCommand,
    authorization: &CommandAuthorization,
    claim: Option<&Claim>,
) -> Result<SkillMutationResult> {
    let actor = identity_of(&authorization.input, "changing a skill")?;
    let payload_hash = command_payload_hash(command)?;
    let manifest = canonicalise(&json!({
        "command": command.kind(),
        "payload_hash": payload_hash,
        "skill_id": command.skill_id(),
        "version_id": command.version_id(),
        "binding_id": command.binding_id(),
    }));
    let bytes = serde_json::to_vec(&manifest).map_err(|err| Error::Internal {
        message: format!("encode Skill change manifest: {err}"),
    })?;
    let object = vedaflow::put_object(tx, tenant, AssetKind::Skill, &bytes).await?;
    let snapshot = PolicySnapshot::new(
        authorization.proposal_allowed.decision.pack_name.clone(),
        authorization.proposal_allowed.decision.pack_version,
    );
    let members = vec![("command".to_owned(), object.hash)];
    let proposal = vedaflow::proposals::open(
        tx,
        tenant,
        &vedaflow::NewProposal {
            target_scope: authorization.target.id,
            source_scope: authorization.target.id,
            asset: AssetKind::Skill,
            effect: ProposalEffect::Apply,
            members: &members,
            sensitivity: command.sensitivity(),
            title: &format!("{} Skill", command.kind()),
            proposer: actor,
            proposer_subject: &authorization.input.principal.subject,
            committed_at: Utc::now(),
            policy_snapshot: &snapshot,
            evidence: None,
        },
        &Signer::Unsigned,
    )
    .await?;
    store::insert_change(&mut *tx, tenant, proposal.id, command, &payload_hash).await?;
    let requirement = approvals::resolve(
        state,
        tx,
        tenant,
        &authorization.input,
        &Requested {
            target: &authorization.target,
            asset: AssetKind::Skill,
            sensitivity: command.sensitivity(),
            entries: &["command".to_owned()],
        },
    )
    .await?;
    let outstanding = requirement.outstanding(&[]);
    audit::record(
        tx,
        tenant,
        AuditAction::SkillChangeOpened,
        Resource::Scope(authorization.target.id).to_string(),
        Outcome::Success,
        json!({
            "change_id": proposal.id,
            "command": command.kind(),
            "payload_hash": payload_hash,
            "manifest_hash": object.hash.to_hex(),
            "skill_id": command.skill_id(),
            "version_id": command.version_id(),
            "binding_id": command.binding_id(),
            "authz": audit::decision_context(Action::ProposalOpen, &authorization.proposal_allowed),
            "approvals": approvals::audit_context(&requirement, &outstanding),
        }),
    )
    .await?;
    let result = if outstanding.is_empty() {
        apply_loaded(
            state,
            tx,
            tenant,
            proposal.id,
            command,
            &payload_hash,
            actor,
        )
        .await?
    } else {
        SkillMutationResult {
            change_id: proposal.id,
            outcome: SkillMutationOutcome::PendingReview,
            skill_id: command.skill_id(),
            version_id: command.version_id(),
            binding_id: command.binding_id(),
            binding_revision: None,
        }
    };
    if let Some(claim) = claim {
        claim.remember(tx, tenant, proposal.id.as_uuid()).await?;
    }
    Ok(result)
}

async fn apply_loaded(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    change_id: ProposalId,
    command: &SkillCommand,
    payload_hash: &str,
    actor: IdentityId,
) -> Result<SkillMutationResult> {
    let authorization = authorize_command(state, tx, tenant, command).await?;
    let mut effect_tx = tx.begin().await.map_err(|err| Error::Storage {
        message: format!("begin Skill effect savepoint: {err}"),
    })?;
    let effect: Result<AppliedSkillEffect> = async {
        verify_bundle_effect(state, &mut effect_tx, tenant, command, &authorization).await?;
        Ok(match command {
            SkillCommand::Install { .. } => {
                let (skill, version) =
                    store::install(&mut effect_tx, tenant, command, actor).await?;
                AppliedSkillEffect {
                    skill_id: Some(skill.id),
                    version_id: Some(version.id),
                    binding_id: None,
                    binding_revision: None,
                }
            }
            SkillCommand::Update {
                skill_id,
                version_id,
                ..
            } => {
                store::update(&mut effect_tx, tenant, command, actor)
                    .await?
                    .ok_or_else(|| Error::Conflict {
                        message: format!(
                            "skill {skill_id} no longer has the expected current version"
                        ),
                    })?;
                AppliedSkillEffect {
                    skill_id: Some(*skill_id),
                    version_id: Some(*version_id),
                    binding_id: None,
                    binding_revision: None,
                }
            }
            SkillCommand::Bind {
                skill_id,
                binding_id,
                ..
            } => {
                let binding = store::bind(&mut effect_tx, tenant, command, actor).await?;
                AppliedSkillEffect {
                    skill_id: Some(*skill_id),
                    version_id: command.version_id(),
                    binding_id: Some(*binding_id),
                    binding_revision: Some(binding.revision),
                }
            }
            SkillCommand::SetBinding { binding_id, .. } => {
                let binding = store::set_binding(&mut effect_tx, tenant, command, actor)
                    .await?
                    .ok_or_else(|| Error::Conflict {
                        message: format!(
                            "skill binding {binding_id} no longer has the expected revision"
                        ),
                    })?;
                AppliedSkillEffect {
                    skill_id: Some(binding.skill_id),
                    version_id: command.version_id(),
                    binding_id: Some(binding.id),
                    binding_revision: Some(binding.revision),
                }
            }
        })
    }
    .await;
    let effect = match effect {
        Ok(effect) => {
            effect_tx.commit().await.map_err(|err| Error::Storage {
                message: format!("commit Skill effect savepoint: {err}"),
            })?;
            effect
        }
        Err(error @ (Error::Conflict { .. } | Error::NotFound { .. } | Error::Invalid { .. })) => {
            effect_tx.rollback().await.map_err(|err| Error::Storage {
                message: format!("roll back rejected Skill effect: {err}"),
            })?;
            let reason = match error {
                Error::Conflict { .. } => "stale_precondition",
                Error::NotFound { .. } => "target_not_found",
                Error::Invalid { .. } => "invalid_effect",
                _ => unreachable!(),
            };
            if !vedaflow::proposals::close(
                tx,
                tenant,
                change_id,
                ProposalState::Rejected,
                actor,
                Some(reason),
            )
            .await?
            {
                return Err(Error::Conflict {
                    message: format!(
                        "Skill change {change_id} closed before rejection was recorded"
                    ),
                });
            }
            audit::record(
                tx,
                tenant,
                AuditAction::SkillChangeRejected,
                Resource::Scope(authorization.target.id).to_string(),
                Outcome::Deny,
                json!({
                    "change_id": change_id,
                    "command": command.kind(),
                    "payload_hash": payload_hash,
                    "reason_code": reason,
                }),
            )
            .await?;
            return Ok(SkillMutationResult {
                change_id,
                outcome: SkillMutationOutcome::Rejected,
                skill_id: command.skill_id(),
                version_id: command.version_id(),
                binding_id: command.binding_id(),
                binding_revision: None,
            });
        }
        Err(error) => {
            effect_tx.rollback().await.map_err(|err| Error::Storage {
                message: format!("roll back failed Skill effect: {err}"),
            })?;
            return Err(error);
        }
    };
    let applied_result = SkillMutationResult {
        change_id,
        outcome: SkillMutationOutcome::Applied,
        skill_id: effect.skill_id,
        version_id: effect.version_id,
        binding_id: effect.binding_id,
        binding_revision: effect.binding_revision,
    };
    if !store::finish_change(&mut *tx, tenant, change_id, &applied_result).await? {
        return Err(Error::Conflict {
            message: format!("Skill change {change_id} was already applied"),
        });
    }
    if !vedaflow::proposals::close(tx, tenant, change_id, ProposalState::Applied, actor, None)
        .await?
    {
        return Err(Error::Conflict {
            message: format!("Skill change {change_id} closed before its effect completed"),
        });
    }
    audit::record(
        tx,
        tenant,
        AuditAction::SkillChangeApplied,
        Resource::Scope(authorization.target.id).to_string(),
        Outcome::Success,
        json!({
            "change_id": change_id,
            "command": command.kind(),
            "payload_hash": payload_hash,
            "skill_id": applied_result.skill_id,
            "version_id": applied_result.version_id,
            "binding_id": applied_result.binding_id,
            "binding_revision": applied_result.binding_revision,
            "authz": audit::decision_context(Action::SkillWrite, &authorization.write_allowed),
        }),
    )
    .await?;
    Ok(applied_result)
}

/// Rebuild and rescan the exact immutable objects named by a pending install
/// or update immediately before its VedaFlow effect runs. A review can remain
/// open while a policy pack or scanner deployment changes; approval of the
/// old result is not authority to admit bytes the current gate refuses.
async fn verify_bundle_effect(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    command: &SkillCommand,
    authorization: &CommandAuthorization,
) -> Result<()> {
    let (
        name,
        governing_scope_id,
        sensitivity,
        expected_digest,
        expected_manifest,
        file_refs,
        admitted_scan,
        admitted_scan_ruleset,
        admitted_quality_score,
        admitted_rubric_version,
    ) = match command {
        SkillCommand::Install {
            name,
            governing_scope_id,
            sensitivity,
            bundle_digest,
            manifest,
            files,
            scan,
            scan_ruleset_version,
            quality_score,
            rubric_version,
            ..
        }
        | SkillCommand::Update {
            name,
            governing_scope_id,
            sensitivity,
            bundle_digest,
            manifest,
            files,
            scan,
            scan_ruleset_version,
            quality_score,
            rubric_version,
            ..
        } => (
            name,
            *governing_scope_id,
            *sensitivity,
            bundle_digest,
            manifest,
            files,
            scan,
            *scan_ruleset_version,
            *quality_score,
            *rubric_version,
        ),
        SkillCommand::Bind { .. } | SkillCommand::SetBinding { .. } => return Ok(()),
    };

    if bundle_digest(file_refs)? != store::decode_hex_32(expected_digest, "skill bundle digest")? {
        return Err(Error::Invalid {
            message: "Skill change bundle digest does not match its immutable file addresses"
                .to_owned(),
        });
    }

    let addresses = file_refs
        .iter()
        .map(|file| {
            Ok(vedaflow::hash::ObjectHash::from_bytes(
                store::decode_hex_32(&file.object_hash, "skill file object address")?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let objects = vedaflow::read_objects(tx, tenant, &addresses).await?;
    let mut files = Vec::with_capacity(file_refs.len());
    for (file_ref, address) in file_refs.iter().zip(addresses) {
        let object = objects.get(&address).ok_or_else(|| Error::Invalid {
            message: format!("Skill change names missing bundle object {address}"),
        })?;
        if object.kind != AssetKind::Skill {
            return Err(Error::Invalid {
                message: format!("Skill change object {address} has the wrong asset kind"),
            });
        }
        let asset = SkillAsset::from_bytes(&object.content)?;
        if asset.scope_id != governing_scope_id
            || asset.skill != *name
            || asset.sensitivity != sensitivity
            || asset.file.path != file_ref.path
            || asset.file.content.chars().count()
                != usize::try_from(file_ref.chars).unwrap_or(usize::MAX)
        {
            return Err(Error::Invalid {
                message: format!(
                    "Skill change file {} failed its scope/name/sensitivity/path binding",
                    file_ref.path
                ),
            });
        }
        files.push(asset.file);
    }

    let bundle = SkillBundle {
        name: name.clone(),
        files,
    };
    let manifest = serde_json::to_value(bundle.validate()?).map_err(|err| Error::Internal {
        message: format!("encode revalidated Skill manifest: {err}"),
    })?;
    if canonicalise(&manifest) != canonicalise(expected_manifest) {
        return Err(Error::Invalid {
            message: "Skill change manifest does not match its immutable bundle bytes".to_owned(),
        });
    }

    let effective = state.pdp.effective(
        tenant,
        Resource::Scope(governing_scope_id),
        &authorization.input.context(),
    );
    let scan = scan_security(&bundle.files).await?;
    if scan.blocked_by(&effective.scan) {
        return Err(Error::Invalid {
            message: format!(
                "Skill change was refused by the current security scanner at {}",
                scan.worst().unwrap_or(ScanSeverity::Critical)
            ),
        });
    }
    let quality = score_quality(&bundle.files).await?;
    if quality.score < effective.quality.min_score {
        return Err(Error::Invalid {
            message: format!(
                "Skill change scored {}/100 below the current policy minimum of {}",
                quality.score, effective.quality.min_score
            ),
        });
    }

    // Within one ruleset/rubric, recomputation must be byte-for-byte stable.
    // A newer deployment may legitimately produce a newer result; the old
    // admitted evidence remains attached to the immutable command while the
    // current result above is the one that gates application.
    if scan.ruleset_version == admitted_scan_ruleset {
        let admitted_findings = canonicalise(admitted_scan);
        let threshold = admitted_scan
            .get("blocks_at")
            .and_then(Value::as_str)
            .and_then(|value| value.parse::<ScanSeverity>().ok())
            .unwrap_or(effective.scan.threshold());
        if canonicalise(&scan_payload(&scan, threshold)) != admitted_findings {
            return Err(Error::Invalid {
                message: "Skill scanner produced different evidence for the admitted ruleset"
                    .to_owned(),
            });
        }
    }
    if quality.rubric_version == admitted_rubric_version && quality.score != admitted_quality_score
    {
        return Err(Error::Invalid {
            message: "Skill rubric produced a different score for the admitted rubric version"
                .to_owned(),
        });
    }
    Ok(())
}

async fn verify_change_binding(
    tx: &mut PgConnection,
    tenant: TenantId,
    proposal: &vedaflow::StoredProposal,
    change: &store::StoredChange,
) -> Result<()> {
    let members = vedaflow::proposals::members(tx, tenant, proposal.commit).await?;
    let [member] = members.as_slice() else {
        return Err(invalid_change(proposal.id));
    };
    let object = vedaflow::read_object(tx, tenant, member.object)
        .await?
        .ok_or_else(|| invalid_change(proposal.id))?;
    let manifest: Value =
        serde_json::from_slice(&object.content).map_err(|_| invalid_change(proposal.id))?;
    let valid = member.name == "command"
        && object.kind == AssetKind::Skill
        && command_payload_hash(&change.command)? == change.payload_hash
        && manifest.get("command").and_then(Value::as_str) == Some(change.command.kind())
        && manifest.get("payload_hash").and_then(Value::as_str)
            == Some(change.payload_hash.as_str());
    if !valid {
        return Err(invalid_change(proposal.id));
    }
    Ok(())
}

fn invalid_change(id: ProposalId) -> Error {
    Error::Internal {
        message: format!("Skill change {id} failed its VedaFlow payload-integrity check"),
    }
}

async fn authorize_change_read(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    proposal: &vedaflow::StoredProposal,
) -> Result<Authorized> {
    let scope = scope_for(tx, tenant, proposal.target_scope_id).await?;
    let input = authz::gather(state, tx, Some(&scope), AnchorSelection::none(), Vec::new()).await?;
    authz::decide(
        state,
        &input,
        Action::ProposalRead,
        Resource::Scope(scope.id),
    )
}

fn outcome(state: ProposalState) -> Result<SkillMutationOutcome> {
    match state {
        ProposalState::Open => Ok(SkillMutationOutcome::PendingReview),
        ProposalState::Applied => Ok(SkillMutationOutcome::Applied),
        ProposalState::Rejected | ProposalState::Withdrawn => Ok(SkillMutationOutcome::Rejected),
        ProposalState::Published => Err(Error::Internal {
            message: "a Skill/apply proposal was published as a channel".to_owned(),
        }),
    }
}

async fn change_result(
    tx: &mut PgConnection,
    tenant: TenantId,
    proposal: &vedaflow::StoredProposal,
) -> Result<SkillMutationResult> {
    let change = store::change(&mut *tx, tenant, proposal.id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!("Skill proposal {} has no typed effect", proposal.id),
        })?;
    Ok(store::mutation_result(&change, outcome(proposal.state)?))
}

/// Apply an approved Skill change. Called only by the generic proposal route.
pub async fn apply_reviewed(state: &AppState, id: ProposalId) -> Result<SkillMutationResult> {
    let tenant = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
    let proposal = vedaflow::proposals::read(&mut tx, tenant, id)
        .await?
        .filter(|proposal| {
            proposal.asset == AssetKind::Skill && proposal.effect == ProposalEffect::Apply
        })
        .ok_or_else(|| Error::NotFound {
            entity: format!("Skill change {id}"),
        })?;
    authorize_change_read(state, &mut tx, tenant, &proposal).await?;
    if proposal.state != ProposalState::Open {
        return change_result(&mut tx, tenant, &proposal).await;
    }
    let change = store::change(&mut *tx, tenant, id)
        .await?
        .ok_or_else(|| invalid_change(id))?;
    verify_change_binding(&mut tx, tenant, &proposal, &change).await?;
    let authorization = authorize_command(state, &mut tx, tenant, &change.command).await?;
    let requirement = approvals::resolve(
        state,
        &mut tx,
        tenant,
        &authorization.input,
        &Requested {
            target: &authorization.target,
            asset: AssetKind::Skill,
            sensitivity: change.command.sensitivity(),
            entries: &["command".to_owned()],
        },
    )
    .await?;
    let recorded = vedaflow::proposals::approvals(&mut tx, tenant, id).await?;
    let cast = vedaflow::proposals::cast_for(&recorded, proposal.commit);
    let outstanding = requirement.outstanding(&cast);
    if !outstanding.is_empty() {
        return Err(Error::Conflict {
            message: format!("Skill change {id} still needs {}", outstanding.describe()),
        });
    }
    let actor = identity_of(&authorization.input, "applying a skill change")?;
    let result = apply_loaded(
        state,
        &mut tx,
        tenant,
        id,
        &change.command,
        &change.payload_hash,
        actor,
    )
    .await?;
    commit(tx).await?;
    Ok(result)
}

/// Render a typed Skill change's current result.
pub async fn result(state: &AppState, id: ProposalId) -> Result<SkillMutationResult> {
    let tenant = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
    let proposal = vedaflow::proposals::read(&mut tx, tenant, id)
        .await?
        .filter(|proposal| {
            proposal.asset == AssetKind::Skill && proposal.effect == ProposalEffect::Apply
        })
        .ok_or_else(|| Error::NotFound {
            entity: format!("Skill change {id}"),
        })?;
    authorize_change_read(state, &mut tx, tenant, &proposal).await?;
    let result = change_result(&mut tx, tenant, &proposal).await?;
    commit(tx).await?;
    Ok(result)
}

async fn begin_target_authorization(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    scope_id: ScopeId,
) -> Result<CommandAuthorization> {
    let target = scope_for(tx, tenant, scope_id).await?;
    let input = authz::gather(
        state,
        tx,
        Some(&target),
        AnchorSelection::none(),
        Vec::new(),
    )
    .await?;
    let write_allowed =
        authz::decide(state, &input, Action::SkillWrite, Resource::Scope(scope_id))?;
    let proposal_allowed = authz::decide(
        state,
        &input,
        Action::ProposalOpen,
        Resource::Scope(scope_id),
    )?;
    Ok(CommandAuthorization {
        target,
        input,
        write_allowed,
        proposal_allowed,
    })
}

async fn replay_or_conflict(
    state: &AppState,
    tenant: TenantId,
    claim: &Claim,
    error: Error,
) -> Result<(StatusCode, Json<SkillMutationView>)> {
    let id = crate::idempotency::resolve_conflict(&state.pool, tenant, claim, error).await?;
    let rendered = result(state, ProposalId::from_uuid(id)).await?;
    Ok((StatusCode::OK, Json(rendered.into())))
}

async fn submit_bundle(
    state: &AppState,
    headers: &HeaderMap,
    skill_id: Option<SkillId>,
    install: Option<InstallSkillBody>,
    update: Option<UpdateSkillBody>,
) -> Result<(StatusCode, Json<SkillMutationView>)> {
    let tenant = tenant_id()?;
    let actor_subject = subject()?;
    let (operation, canonical) = match (&install, &update, skill_id) {
        (Some(body), None, None) => (
            "skill.install",
            serde_json::to_value(body).map_err(|err| Error::Invalid {
                message: format!("encode skill install request: {err}"),
            })?,
        ),
        (None, Some(body), Some(id)) => ("skill.update", json!({"skill_id": id, "body": body})),
        _ => {
            return Err(Error::Internal {
                message: "invalid bundle submission shape".to_owned(),
            });
        }
    };
    let claim = Claim::from_headers(headers, operation, &actor_subject, &canonical)?;
    if let Dispatch::Replay(id) = crate::idempotency::dispatch(&state.pool, tenant, &claim).await? {
        return Ok((
            StatusCode::OK,
            Json(result(state, ProposalId::from_uuid(id)).await?.into()),
        ));
    }

    let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
    let (scope_id, name, sensitivity, supplied, provenance, expected) =
        match (install, update, skill_id) {
            (Some(body), None, None) => (
                body.governing_scope_id,
                body.name.parse()?,
                body.sensitivity.parse()?,
                body.files,
                SkillProvenance::try_from(body.provenance)?,
                None,
            ),
            (None, Some(body), Some(id)) => {
                let skill =
                    store::by_id(&mut *tx, tenant, id)
                        .await?
                        .ok_or_else(|| Error::NotFound {
                            entity: format!("skill {id}"),
                        })?;
                (
                    skill.governing_scope_id,
                    skill.name,
                    body.sensitivity.parse()?,
                    body.files,
                    SkillProvenance::try_from(body.provenance)?,
                    Some((id, body.expected_current_version_id)),
                )
            }
            _ => unreachable!(),
        };
    let preliminary = begin_target_authorization(state, &mut tx, tenant, scope_id).await?;
    if let Some((id, current)) = expected {
        let (skill, version) = read_skill_and_version(&mut tx, tenant, id, Some(current)).await?;
        authorize_skill_read(state, &mut tx, tenant, &skill, &version).await?;
    }
    let prepared = prepare_bundle(
        state,
        &mut tx,
        tenant,
        &preliminary,
        name.clone(),
        sensitivity,
        supplied,
    )
    .await?;
    let command = match expected {
        None => SkillCommand::Install {
            skill_id: SkillId::new(),
            version_id: SkillVersionId::new(),
            governing_scope_id: scope_id,
            name,
            sensitivity,
            bundle_digest: prepared.bundle_digest,
            manifest: prepared.manifest,
            files: prepared.files,
            provenance,
            scan: prepared.scan,
            scan_ruleset_version: prepared.scan_ruleset_version,
            quality_score: prepared.quality_score,
            rubric_version: prepared.rubric_version,
        },
        Some((id, expected_current_version_id)) => SkillCommand::Update {
            skill_id: id,
            expected_current_version_id,
            version_id: SkillVersionId::new(),
            governing_scope_id: scope_id,
            name,
            sensitivity,
            bundle_digest: prepared.bundle_digest,
            manifest: prepared.manifest,
            files: prepared.files,
            provenance,
            scan: prepared.scan,
            scan_ruleset_version: prepared.scan_ruleset_version,
            quality_score: prepared.quality_score,
            rubric_version: prepared.rubric_version,
        },
    };
    let authorization = authorize_command(state, &mut tx, tenant, &command).await?;
    let created = open_command(
        state,
        &mut tx,
        tenant,
        &command,
        &authorization,
        Some(&claim),
    )
    .await;
    match created {
        Ok(created) => {
            commit(tx).await?;
            Ok((StatusCode::CREATED, Json(created.into())))
        }
        Err(conflict @ Error::Conflict { .. }) => {
            drop(tx);
            replay_or_conflict(state, tenant, &claim, conflict).await
        }
        Err(error) => Err(error),
    }
}

async fn submit_binding_command(
    state: &AppState,
    headers: &HeaderMap,
    operation: &'static str,
    canonical: Value,
    command: SkillCommand,
) -> Result<(StatusCode, Json<SkillMutationView>)> {
    let tenant = tenant_id()?;
    let claim = Claim::from_headers(headers, operation, &subject()?, &canonical)?;
    if let Dispatch::Replay(id) = crate::idempotency::dispatch(&state.pool, tenant, &claim).await? {
        return Ok((
            StatusCode::OK,
            Json(result(state, ProposalId::from_uuid(id)).await?.into()),
        ));
    }
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
    let authorization = authorize_command(state, &mut tx, tenant, &command).await?;
    let created = open_command(
        state,
        &mut tx,
        tenant,
        &command,
        &authorization,
        Some(&claim),
    )
    .await;
    match created {
        Ok(created) => {
            commit(tx).await?;
            Ok((StatusCode::CREATED, Json(created.into())))
        }
        Err(conflict @ Error::Conflict { .. }) => {
            drop(tx);
            replay_or_conflict(state, tenant, &claim, conflict).await
        }
        Err(error) => Err(error),
    }
}

/// Install a stable skill and first immutable version through VedaFlow.
#[utoipa::path(
    post,
    path = "/v1/skills",
    operation_id = "install_skill",
    tag = "skills",
    params(("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry.")),
    request_body = InstallSkillBody,
    responses(
        (status = 201, description = "Change opened", body = SkillMutationView),
        (status = 400, description = "Invalid bundle", body = ApiErrorBody),
        (status = 403, description = "Denied", body = ApiErrorBody),
        (status = 409, description = "Conflict", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "skills.install", skip_all)]
pub(crate) async fn install(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: std::result::Result<Json<InstallSkillBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        submit_bundle(&state, &headers, None, Some(body), None).await
    }
    .await;
    respond(&state, "install", result).await
}

/// Create a new immutable version through VedaFlow.
#[utoipa::path(
    patch,
    path = "/v1/skills/{id}",
    operation_id = "update_skill",
    tag = "skills",
    params(
        ("id" = String, Path, format = "uuid"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry.")
    ),
    request_body = UpdateSkillBody,
    responses(
        (status = 201, description = "Change opened", body = SkillMutationView),
        (status = 404, description = "Not found", body = ApiErrorBody),
        (status = 409, description = "Stale version", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "skills.update", skip_all)]
pub(crate) async fn update(
    State(state): State<AppState>,
    Path(id): Path<SkillId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<UpdateSkillBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        submit_bundle(&state, &headers, Some(id), None, Some(body)).await
    }
    .await;
    respond(&state, "update", result).await
}

/// Create a project/principal binding through VedaFlow.
#[utoipa::path(
    post,
    path = "/v1/skill-bindings",
    operation_id = "create_skill_binding",
    tag = "skills",
    params(("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry.")),
    request_body = CreateSkillBindingBody,
    responses(
        (status = 201, description = "Change opened", body = SkillMutationView),
        (status = 404, description = "Skill not visible", body = ApiErrorBody),
        (status = 409, description = "Conflict", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "skills.bind", skip_all)]
pub(crate) async fn create_binding(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: std::result::Result<Json<CreateSkillBindingBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let canonical = serde_json::to_value(&body).map_err(|err| Error::Invalid {
            message: format!("encode skill binding request: {err}"),
        })?;
        let command = SkillCommand::Bind {
            binding_id: SkillBindingId::new(),
            skill_id: body.skill_id,
            scope_id: body.scope_id,
            pinned_version_id: body.pinned_version_id,
            enabled: body.enabled,
        };
        submit_binding_command(&state, &headers, "skill.bind", canonical, command).await
    }
    .await;
    respond(&state, "bind", result).await
}

/// Update, enable, disable, pin or unpin a binding through VedaFlow.
#[utoipa::path(
    patch,
    path = "/v1/skill-bindings/{id}",
    operation_id = "update_skill_binding",
    tag = "skills",
    params(
        ("id" = String, Path, format = "uuid"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry.")
    ),
    request_body = UpdateSkillBindingBody,
    responses(
        (status = 201, description = "Change opened", body = SkillMutationView),
        (status = 404, description = "Binding not found", body = ApiErrorBody),
        (status = 409, description = "Stale revision", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "skills.binding.update", skip_all)]
pub(crate) async fn update_binding(
    State(state): State<AppState>,
    Path(id): Path<SkillBindingId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<UpdateSkillBindingBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        if body.reason.trim().is_empty() || body.reason.chars().count() > 200 {
            return Err(Error::Invalid {
                message: "binding reason must contain 1..=200 characters".to_owned(),
            });
        }
        refuse_if_secret("binding reason", &body.reason).await?;
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let binding =
            store::binding(&mut *tx, tenant, id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("skill binding {id}"),
                })?;
        drop(tx);
        let canonical = json!({"binding_id": id, "body": body});
        let command = SkillCommand::SetBinding {
            binding_id: id,
            scope_id: binding.scope_id,
            expected_revision: body.expected_revision,
            enabled: body.enabled,
            pinned_version_id: body.pinned_version_id,
            reason: body.reason,
        };
        submit_binding_command(&state, &headers, "skill.binding.update", canonical, command).await
    }
    .await;
    respond(&state, "binding_update", result).await
}

/// Roll a binding back by changing its pin, never its version history.
#[utoipa::path(
    post,
    path = "/v1/skill-bindings/{id}/rollback",
    operation_id = "rollback_skill_binding",
    tag = "skills",
    params(
        ("id" = String, Path, format = "uuid"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry.")
    ),
    request_body = RollbackSkillBindingBody,
    responses(
        (status = 201, description = "Rollback change opened", body = SkillMutationView),
        (status = 404, description = "Binding/version not found", body = ApiErrorBody),
        (status = 409, description = "Stale revision", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "skills.binding.rollback", skip_all)]
pub(crate) async fn rollback_binding(
    State(state): State<AppState>,
    Path(id): Path<SkillBindingId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<RollbackSkillBindingBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let binding =
            store::binding(&mut *tx, tenant, id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("skill binding {id}"),
                })?;
        drop(tx);
        let canonical = json!({"binding_id": id, "body": body});
        let command = SkillCommand::SetBinding {
            binding_id: id,
            scope_id: binding.scope_id,
            expected_revision: body.expected_revision,
            enabled: true,
            pinned_version_id: Some(body.version_id),
            reason: "rollback".to_owned(),
        };
        submit_binding_command(
            &state,
            &headers,
            "skill.binding.rollback",
            canonical,
            command,
        )
        .await
    }
    .await;
    respond(&state, "binding_rollback", result).await
}

// ── Public catalogue reads ────────────────────────────────────────────────

/// Cursor controls for the Skill catalogue.
#[derive(Debug, Clone, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListSkillsParams {
    /// Resume after this stable Skill identifier.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "uuid")]
    pub cursor: Option<SkillId>,
    /// Rows considered, 1–200; denied rows are omitted.
    #[serde(default)]
    pub limit: Option<i64>,
}

/// Cursor controls for immutable versions, newest first.
#[derive(Debug, Clone, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListVersionsParams {
    /// Resume below this version ordinal.
    #[serde(default)]
    pub before_ordinal: Option<u64>,
    /// Rows to return, from one through two hundred.
    #[serde(default)]
    pub limit: Option<i64>,
}

/// Scope and cursor controls for binding listings.
#[derive(Debug, Clone, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListBindingsParams {
    /// Project or principal scope whose bindings are listed.
    #[param(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    /// Resume after this binding identifier.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "uuid")]
    pub cursor: Option<SkillBindingId>,
    /// Rows to return, from one through two hundred.
    #[serde(default)]
    pub limit: Option<i64>,
}

/// Scope selecting the exact Skill versions available to a session.
#[derive(Debug, Clone, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AvailableSkillsParams {
    /// Project or principal scope at which bindings resolve.
    #[param(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
}

/// Cursor controls for usage evidence.
#[derive(Debug, Clone, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListUsageParams {
    /// Resume after this usage event identifier.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "uuid")]
    pub cursor: Option<SkillUsageEventId>,
    /// Rows to return, from one through two hundred.
    #[serde(default)]
    pub limit: Option<i64>,
}

/// Cursor controls for controlled-harness test runs.
#[derive(Debug, Clone, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListTestRunsParams {
    /// Resume after this test-run identifier.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "uuid")]
    pub cursor: Option<SkillTestRunId>,
    /// Rows to return, from one through two hundred.
    #[serde(default)]
    pub limit: Option<i64>,
}

/// Immutable-version collection.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SkillVersionListView {
    /// Immutable versions, newest first.
    pub versions: Vec<SkillVersionView>,
    /// Ordinal cursor for the next page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<u64>,
}

/// Immutable file collection.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SkillVersionFileListView {
    /// Immutable file descriptors in path order.
    pub files: Vec<SkillVersionFileView>,
}

fn limit(value: Option<i64>) -> Result<i64> {
    let value = value.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&value) {
        return Err(Error::Invalid {
            message: format!("limit must be between 1 and {MAX_LIMIT}"),
        });
    }
    Ok(value)
}

async fn skill_view(
    tx: &mut PgConnection,
    tenant: TenantId,
    skill: StoredSkill,
) -> Result<SkillView> {
    let version = store::version(&mut *tx, tenant, skill.current_version_id)
        .await?
        .filter(|version| version.skill_id == skill.id)
        .ok_or_else(|| Error::Internal {
            message: format!(
                "skill {} names missing current version {}",
                skill.id, skill.current_version_id
            ),
        })?;
    Ok(SkillView {
        id: skill.id,
        governing_scope_id: skill.governing_scope_id,
        name: skill.name.to_string(),
        current_version_id: skill.current_version_id,
        current_version: version.try_into()?,
        created_at: skill.created_at,
        created_by: skill.created_by,
        updated_at: skill.updated_at,
        updated_by: skill.updated_by,
    })
}

async fn exact_visible(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    skill_id: SkillId,
    version_id: Option<SkillVersionId>,
) -> Result<(StoredSkill, StoredVersion, Authorized)> {
    let (skill, version) = read_skill_and_version(tx, tenant, skill_id, version_id).await?;
    let authorized = authorize_skill_read(state, tx, tenant, &skill, &version)
        .await
        .map_err(|error| match error {
            Error::PolicyDenied { .. } => Error::NotFound {
                entity: format!("skill {skill_id}"),
            },
            other => other,
        })?;
    Ok((skill, version, authorized))
}

/// List visible stable skills. Each row is decided at its governing scope.
#[utoipa::path(
    get,
    path = "/v1/skills",
    operation_id = "list_skills",
    tag = "skills",
    params(ListSkillsParams),
    responses(
        (status = 200, description = "Visible skills", body = SkillListView),
        (status = 400, description = "Invalid cursor/limit", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "skills.list", skip_all)]
pub(crate) async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListSkillsParams>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let take = limit(params.limit)?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let candidates = store::list(&mut tx, tenant, params.cursor, take + 1).await?;
        let more = candidates.len() as i64 > take;
        let considered = candidates
            .into_iter()
            .take(take as usize)
            .collect::<Vec<_>>();
        let next_cursor = more
            .then(|| considered.last().map(|skill| skill.id))
            .flatten();
        let mut visible = Vec::new();
        for skill in considered {
            let Some(version) = store::version(&mut *tx, tenant, skill.current_version_id).await?
            else {
                return Err(Error::Internal {
                    message: format!("skill {} has no current version", skill.id),
                });
            };
            match authorize_skill_read(&state, &mut tx, tenant, &skill, &version).await {
                Ok(_) => visible.push(skill_view(&mut tx, tenant, skill).await?),
                Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => {}
                Err(error) => return Err(error),
            }
        }
        audit::record(
            &mut tx,
            tenant,
            AuditAction::AuthzDecision,
            "skill catalogue".to_owned(),
            Outcome::Allow,
            json!({"op": "skills.list", "visible": visible.len(), "considered": take}),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(SkillListView {
            skills: visible,
            next_cursor,
        }))
    }
    .await;
    respond(&state, "list", result).await
}

/// Get one stable skill and current version.
#[utoipa::path(
    get,
    path = "/v1/skills/{id}",
    operation_id = "get_skill",
    tag = "skills",
    params(("id" = String, Path, format = "uuid")),
    responses(
        (status = 200, description = "Skill", body = SkillView),
        (status = 404, description = "Absent or denied", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "skills.get", skip_all)]
pub(crate) async fn get(State(state): State<AppState>, Path(id): Path<SkillId>) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let (skill, version, authorized) = exact_visible(&state, &mut tx, tenant, id, None).await?;
        audit::record(
            &mut tx,
            tenant,
            AuditAction::SkillResolved,
            Resource::Scope(skill.governing_scope_id).to_string(),
            Outcome::Success,
            json!({
                "skill_id": skill.id,
                "version_id": version.id,
                "bundle_digest": store::hex_32(&version.bundle_digest),
                "authz": audit::decision_context(Action::SkillRead, &authorized),
                "content_served": false,
            }),
        )
        .await?;
        let view = skill_view(&mut tx, tenant, skill).await?;
        commit(tx).await?;
        Ok(Json(view))
    }
    .await;
    respond(&state, "get", result).await
}

/// List immutable versions, newest first.
#[utoipa::path(
    get,
    path = "/v1/skills/{id}/versions",
    operation_id = "list_skill_versions",
    tag = "skills",
    params(("id" = String, Path, format = "uuid"), ListVersionsParams),
    responses(
        (status = 200, description = "Versions", body = SkillVersionListView),
        (status = 404, description = "Absent or denied", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "skills.versions", skip_all)]
pub(crate) async fn list_versions(
    State(state): State<AppState>,
    Path(id): Path<SkillId>,
    Query(params): Query<ListVersionsParams>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let take = limit(params.limit)?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let skill = store::by_id(&mut *tx, tenant, id)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity: format!("skill {id}"),
            })?;
        let candidates =
            store::versions(&mut tx, tenant, id, params.before_ordinal, take + 1).await?;
        let more = candidates.len() as i64 > take;
        let considered = candidates
            .into_iter()
            .take(take as usize)
            .collect::<Vec<_>>();
        let next_cursor = more
            .then(|| considered.last().map(|version| version.ordinal))
            .flatten();
        let mut visible = Vec::new();
        for version in considered {
            match authorize_skill_read(&state, &mut tx, tenant, &skill, &version).await {
                Ok(_) => visible.push(version.try_into()?),
                Err(Error::PolicyDenied { .. }) => {}
                Err(error) => return Err(error),
            }
        }
        commit(tx).await?;
        Ok(Json(SkillVersionListView {
            versions: visible,
            next_cursor,
        }))
    }
    .await;
    respond(&state, "versions", result).await
}

/// Get exact immutable version metadata.
#[utoipa::path(
    get,
    path = "/v1/skills/{id}/versions/{version_id}",
    operation_id = "get_skill_version",
    tag = "skills",
    params(
        ("id" = String, Path, format = "uuid"),
        ("version_id" = String, Path, format = "uuid")
    ),
    responses(
        (status = 200, description = "Version", body = SkillVersionView),
        (status = 404, description = "Absent or denied", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
pub(crate) async fn get_version(
    State(state): State<AppState>,
    Path((id, version_id)): Path<(SkillId, SkillVersionId)>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let (_, version, _) = exact_visible(&state, &mut tx, tenant, id, Some(version_id)).await?;
        commit(tx).await?;
        let view: SkillVersionView = version.try_into()?;
        Ok(Json(view))
    }
    .await;
    respond(&state, "version_get", result).await
}

/// List exact immutable file descriptors.
#[utoipa::path(
    get,
    path = "/v1/skills/{id}/versions/{version_id}/files",
    operation_id = "list_skill_version_files",
    tag = "skills",
    params(
        ("id" = String, Path, format = "uuid"),
        ("version_id" = String, Path, format = "uuid")
    ),
    responses(
        (status = 200, description = "Files", body = SkillVersionFileListView),
        (status = 404, description = "Absent or denied", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
pub(crate) async fn list_files(
    State(state): State<AppState>,
    Path((id, version_id)): Path<(SkillId, SkillVersionId)>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        exact_visible(&state, &mut tx, tenant, id, Some(version_id)).await?;
        let files = store::files(&mut *tx, tenant, version_id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
        commit(tx).await?;
        Ok(Json(SkillVersionFileListView { files }))
    }
    .await;
    respond(&state, "files", result).await
}

/// Fetch one exact version file. The wildcard remains bundle-relative.
#[utoipa::path(
    get,
    path = "/v1/skills/{id}/versions/{version_id}/files/{path}",
    operation_id = "get_skill_version_file",
    tag = "skills",
    params(
        ("id" = String, Path, format = "uuid"),
        ("version_id" = String, Path, format = "uuid"),
        ("path" = String, Path)
    ),
    responses(
        (status = 200, description = "File content", body = SkillVersionFileContentView),
        (status = 404, description = "Absent or denied", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "skills.file.get", skip_all)]
pub(crate) async fn get_file(
    State(state): State<AppState>,
    Path((id, version_id, path)): Path<(SkillId, SkillVersionId, String)>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let path: SkillFilePath = path.parse()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let (skill, version, authorized) =
            exact_visible(&state, &mut tx, tenant, id, Some(version_id)).await?;
        let file = store::file(&mut *tx, tenant, version_id, &path)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity: format!("skill version file {path}"),
            })?;
        let object_hash = vedaflow::hash::ObjectHash::from_bytes(file.object_hash);
        let object = vedaflow::read_object(&mut tx, tenant, object_hash)
            .await?
            .ok_or_else(|| Error::Internal {
                message: format!("skill version file {path} names a missing object"),
            })?;
        let asset = SkillAsset::from_bytes(&object.content)?;
        if asset.scope_id != skill.governing_scope_id
            || asset.sensitivity != version.sensitivity
            || asset.file.path != path
        {
            return Err(Error::Internal {
                message: format!("skill version file {path} failed its object binding"),
            });
        }
        audit::record(
            &mut tx,
            tenant,
            AuditAction::SkillResolved,
            Resource::Scope(skill.governing_scope_id).to_string(),
            Outcome::Success,
            json!({
                "skill_id": id,
                "version_id": version_id,
                "path": path,
                "object_hash": object_hash.to_hex(),
                "authz": audit::decision_context(Action::SkillRead, &authorized),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(SkillVersionFileContentView {
            version_id,
            path: path.to_string(),
            object_hash: object_hash.to_hex(),
            content: asset.file.content,
        }))
    }
    .await;
    respond(&state, "file_get", result).await
}

async fn authorize_binding_scope(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    scope_id: ScopeId,
    sensitivity: Sensitivity,
    action: Action,
) -> Result<(Scope, DecisionInput, Authorized)> {
    let scope = scope_for(tx, tenant, scope_id).await?;
    let input = authz::gather(state, tx, Some(&scope), AnchorSelection::none(), Vec::new()).await?;
    let authorized = if action == Action::SkillRead {
        authz::decide_skill_read(state, &input, Resource::Scope(scope_id), sensitivity)?
    } else {
        authz::decide(state, &input, action, Resource::Scope(scope_id))?
    };
    Ok((scope, input, authorized))
}

async fn resolved_visible(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    resolved: &ResolvedBinding,
) -> Result<()> {
    authorize_binding_scope(
        state,
        tx,
        tenant,
        resolved.binding.scope_id,
        resolved.version.sensitivity,
        Action::SkillRead,
    )
    .await?;
    let skill = store::by_id(&mut *tx, tenant, resolved.binding.skill_id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("skill {}", resolved.binding.skill_id),
        })?;
    authorize_skill_read(state, tx, tenant, &skill, &resolved.version).await?;
    Ok(())
}

/// List revisioned bindings at one project/principal scope.
#[utoipa::path(
    get,
    path = "/v1/skill-bindings",
    operation_id = "list_skill_bindings",
    tag = "skills",
    params(ListBindingsParams),
    responses(
        (status = 200, description = "Bindings", body = SkillBindingListView),
        (status = 404, description = "Scope absent or denied", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
pub(crate) async fn list_bindings(
    State(state): State<AppState>,
    Query(params): Query<ListBindingsParams>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let take = limit(params.limit)?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        authorize_binding_scope(
            &state,
            &mut tx,
            tenant,
            params.scope_id,
            Sensitivity::Public,
            Action::ScopeRead,
        )
        .await?;
        let candidates =
            store::bindings_at(&mut tx, tenant, params.scope_id, params.cursor, take + 1).await?;
        let more = candidates.len() as i64 > take;
        let considered = candidates
            .into_iter()
            .take(take as usize)
            .collect::<Vec<_>>();
        let next_cursor = more
            .then(|| considered.last().map(|binding| binding.id))
            .flatten();
        let mut visible = Vec::new();
        for binding in considered {
            let (skill, version) = read_skill_and_version(
                &mut tx,
                tenant,
                binding.skill_id,
                binding.pinned_version_id,
            )
            .await?;
            match authorize_skill_read(&state, &mut tx, tenant, &skill, &version).await {
                Ok(_) => visible.push(binding.into()),
                Err(Error::PolicyDenied { .. }) => {}
                Err(error) => return Err(error),
            }
        }
        commit(tx).await?;
        Ok(Json(SkillBindingListView {
            bindings: visible,
            next_cursor,
        }))
    }
    .await;
    respond(&state, "bindings", result).await
}

/// Get one revisioned binding.
#[utoipa::path(
    get,
    path = "/v1/skill-bindings/{id}",
    operation_id = "get_skill_binding",
    tag = "skills",
    params(("id" = String, Path, format = "uuid")),
    responses(
        (status = 200, description = "Binding", body = SkillBindingView),
        (status = 404, description = "Absent or denied", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
pub(crate) async fn get_binding(
    State(state): State<AppState>,
    Path(id): Path<SkillBindingId>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let binding =
            store::binding(&mut *tx, tenant, id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("skill binding {id}"),
                })?;
        authorize_binding_scope(
            &state,
            &mut tx,
            tenant,
            binding.scope_id,
            Sensitivity::Public,
            Action::ScopeRead,
        )
        .await?;
        let (skill, version) =
            read_skill_and_version(&mut tx, tenant, binding.skill_id, binding.pinned_version_id)
                .await?;
        authorize_skill_read(&state, &mut tx, tenant, &skill, &version).await?;
        commit(tx).await?;
        let view: SkillBindingView = binding.into();
        Ok(Json(view))
    }
    .await;
    respond(&state, "binding_get", result).await
}

/// Resolve enabled bindings to exact immutable versions for a context scope.
#[utoipa::path(
    get,
    path = "/v1/skills/available",
    operation_id = "list_available_skills",
    tag = "skills",
    params(AvailableSkillsParams),
    responses(
        (status = 200, description = "Available exact versions", body = AvailableSkillListView),
        (status = 404, description = "Scope absent or denied", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "skills.available", skip_all)]
pub(crate) async fn available(
    State(state): State<AppState>,
    Query(params): Query<AvailableSkillsParams>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let (_, input, _) = authorize_binding_scope(
            &state,
            &mut tx,
            tenant,
            params.scope_id,
            Sensitivity::Public,
            Action::SkillRead,
        )
        .await?;
        let runtime = configuration::effective_at_scope(&mut tx, tenant, params.scope_id).await?;
        let mut scope_ids = vec![params.scope_id];
        if let Some(personal) = input.principal.scope_id
            && personal != params.scope_id
        {
            scope_ids.push(personal);
        }
        let candidates = if runtime.document.advertisement.skills {
            store::resolve_for_scopes(&mut tx, tenant, &scope_ids).await?
        } else {
            Vec::new()
        };
        let mut names = std::collections::HashSet::new();
        let mut visible = Vec::new();
        let mut policy_excluded = 0_u64;
        for resolved in candidates {
            match resolved_visible(&state, &mut tx, tenant, &resolved).await {
                Ok(()) => {
                    if names.insert(resolved.name.clone()) {
                        visible.push(AvailableSkillView {
                            binding: resolved.binding.into(),
                            name: resolved.name.to_string(),
                            version: resolved.version.try_into()?,
                            manifest_object_hash: store::hex_32(&resolved.manifest_object_hash),
                        });
                    }
                }
                Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => {
                    policy_excluded += 1;
                }
                Err(error) => return Err(error),
            }
        }
        audit::record(
            &mut tx,
            tenant,
            AuditAction::AuthzDecision,
            Resource::Scope(params.scope_id).to_string(),
            Outcome::Allow,
            json!({
                "op": "skills.available",
                "versions": visible.iter().map(|skill| json!({
                    "binding_id": skill.binding.id,
                    "version_id": skill.version.id,
                    "bundle_digest": skill.version.bundle_digest,
                })).collect::<Vec<_>>(),
                "policy_excluded": policy_excluded,
                "configuration_version_id": runtime.version_id,
                "configuration_hash": runtime.content_hash,
                "advertisement_enabled": runtime.document.advertisement.skills,
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(AvailableSkillListView {
            scope_id: params.scope_id,
            skills: visible,
        }))
    }
    .await;
    respond(&state, "available", result).await
}

// ── Usage evidence and controlled tests ──────────────────────────────────

fn validate_metadata(metadata: &Value, what: &str) -> Result<()> {
    if !metadata.is_object() {
        return Err(Error::Invalid {
            message: format!("{what} metadata must be a JSON object"),
        });
    }
    if serde_json::to_vec(metadata)
        .map_err(|err| Error::Invalid {
            message: format!("encode {what} metadata: {err}"),
        })?
        .len()
        > 16 * 1024
    {
        return Err(Error::Invalid {
            message: format!("{what} metadata exceeds 16384 bytes"),
        });
    }
    Ok(())
}

/// Record one version-specific usage stage idempotently.
#[utoipa::path(
    post,
    path = "/v1/skill-usage",
    operation_id = "record_skill_usage",
    tag = "skills",
    request_body = RecordSkillUsageBody,
    responses(
        (status = 201, description = "Usage appended", body = SkillUsageEventView),
        (status = 200, description = "Idempotent replay", body = SkillUsageEventView),
        (status = 400, description = "Invalid evidence", body = ApiErrorBody),
        (status = 404, description = "Binding/version absent or denied", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "skills.usage.record", skip_all)]
pub(crate) async fn record_usage(
    State(state): State<AppState>,
    payload: std::result::Result<Json<RecordSkillUsageBody>, JsonRejection>,
) -> Response {
    let result: Result<(StatusCode, Json<SkillUsageEventView>)> = async {
        let body = body(payload)?;
        validate_skill_usage_client_event_id(&body.client_event_id)?;
        validate_metadata(&body.metadata, "skill usage")?;
        refuse_if_secret("skill usage metadata", &body.metadata.to_string()).await?;
        let stage: SkillUsageStage = body.stage.parse()?;
        let evidence: SkillUsageEvidence = body.evidence.parse()?;
        if evidence == SkillUsageEvidence::HostObserved && body.session_id.is_none() {
            return Err(Error::Invalid {
                message: "host_observed skill usage must be bound to a session; otherwise use model_reported"
                    .to_owned(),
            });
        }
        let resource_path = body.resource_path.map(|value| value.parse()).transpose()?;
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let binding = store::binding(&mut *tx, tenant, body.binding_id)
            .await?
            .filter(|binding| binding.enabled)
            .ok_or_else(|| Error::NotFound {
                entity: format!("enabled skill binding {}", body.binding_id),
            })?;
        let (skill, version) = read_skill_and_version(
            &mut tx,
            tenant,
            binding.skill_id,
            binding.pinned_version_id,
        )
        .await?;
        if version.id != body.version_id {
            return Err(Error::Conflict {
                message: format!(
                    "binding {} currently resolves to version {}, not {}",
                    binding.id, version.id, body.version_id
                ),
            });
        }
        let (_, input, binding_allowed) = authorize_binding_scope(
            &state,
            &mut tx,
            tenant,
            binding.scope_id,
            version.sensitivity,
            Action::SkillRead,
        )
        .await?;
        let source_allowed = authorize_skill_read(&state, &mut tx, tenant, &skill, &version).await?;
        let actor = identity_of(&input, "recording skill usage")?;
        if evidence == SkillUsageEvidence::HostObserved
            && input
                .identity
                .as_ref()
                .is_none_or(|identity| identity.kind != IdentityKind::Service)
            && body.session_id.is_none()
        {
            return Err(Error::PolicyDenied {
                action: Action::SkillRead.as_str().to_owned(),
                resource: format!("skill usage for {}", version.id),
                reason: "host-observed evidence requires a trusted service or a governed session"
                    .to_owned(),
            });
        }
        let (stored, inserted) = store::record_usage(
            &mut tx,
            tenant,
            SkillUsageEventId::new(),
            binding.id,
            version.id,
            body.session_id,
            actor,
            &body.client_event_id,
            stage,
            evidence,
            resource_path.as_ref(),
            &body.metadata,
            body.occurred_at,
        )
        .await?;
        if !inserted
            && (stored.version_id != version.id
                || stored.session_id != body.session_id
                || stored.principal_id != actor
                || stored.stage != stage
                || stored.evidence != evidence
                || stored.resource_path != resource_path
                || stored.metadata != body.metadata
                || stored.occurred_at != body.occurred_at)
        {
            return Err(Error::Conflict {
                message: format!(
                    "client_event_id {:?} was already used for different skill usage",
                    body.client_event_id
                ),
            });
        }
        if inserted {
            audit::record(
                &mut tx,
                tenant,
                AuditAction::SkillUsageRecorded,
                Resource::Scope(binding.scope_id).to_string(),
                Outcome::Success,
                json!({
                    "event_id": stored.id,
                    "binding_id": binding.id,
                    "skill_id": skill.id,
                    "version_id": version.id,
                    "session_id": stored.session_id,
                    "stage": stage.as_str(),
                    "evidence": evidence.as_str(),
                    "authz": {
                        "binding": audit::decision_context(Action::SkillRead, &binding_allowed),
                        "source": audit::decision_context(Action::SkillRead, &source_allowed),
                    },
                }),
            )
            .await?;
        }
        commit(tx).await?;
        Ok((
            if inserted {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            },
            Json(SkillUsageEventView::from(stored)),
        ))
    }
    .await;
    respond(&state, "usage_record", result).await
}

/// List usage evidence for one exact version.
#[utoipa::path(
    get,
    path = "/v1/skills/{id}/versions/{version_id}/usage",
    operation_id = "list_skill_usage",
    tag = "skills",
    params(
        ("id" = String, Path, format = "uuid"),
        ("version_id" = String, Path, format = "uuid"),
        ListUsageParams
    ),
    responses(
        (status = 200, description = "Usage evidence", body = SkillUsageListView),
        (status = 404, description = "Absent or denied", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
pub(crate) async fn list_usage(
    State(state): State<AppState>,
    Path((id, version_id)): Path<(SkillId, SkillVersionId)>,
    Query(params): Query<ListUsageParams>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let take = limit(params.limit)?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        exact_visible(&state, &mut tx, tenant, id, Some(version_id)).await?;
        let candidates = store::usage(&mut tx, tenant, version_id, params.cursor, take + 1).await?;
        let more = candidates.len() as i64 > take;
        let considered = candidates
            .into_iter()
            .take(take as usize)
            .collect::<Vec<_>>();
        let next_cursor = more
            .then(|| considered.last().map(|event| event.id))
            .flatten();
        let events = considered.into_iter().map(Into::into).collect();
        commit(tx).await?;
        Ok(Json(SkillUsageListView {
            events,
            next_cursor,
        }))
    }
    .await;
    respond(&state, "usage_list", result).await
}

async fn version_bundle(
    tx: &mut PgConnection,
    tenant: TenantId,
    skill: &StoredSkill,
    version: &StoredVersion,
) -> Result<SkillBundle> {
    let refs = store::files(&mut *tx, tenant, version.id).await?;
    let addresses = refs
        .iter()
        .map(|file| vedaflow::hash::ObjectHash::from_bytes(file.object_hash))
        .collect::<Vec<_>>();
    let objects = vedaflow::read_objects(tx, tenant, &addresses).await?;
    let mut files = Vec::with_capacity(refs.len());
    for file in refs {
        let address = vedaflow::hash::ObjectHash::from_bytes(file.object_hash);
        let object = objects.get(&address).ok_or_else(|| Error::Internal {
            message: format!(
                "skill version {} names missing object {address}",
                version.id
            ),
        })?;
        let asset = SkillAsset::from_bytes(&object.content)?;
        if asset.scope_id != skill.governing_scope_id
            || asset.sensitivity != version.sensitivity
            || asset.file.path != file.path
        {
            return Err(Error::Internal {
                message: format!("skill version {} failed its file binding", version.id),
            });
        }
        files.push(asset.file);
    }
    Ok(SkillBundle {
        name: skill.name.clone(),
        files,
    })
}

/// Run the built-in non-executing validation sandbox.
#[utoipa::path(
    post,
    path = "/v1/skills/{id}/versions/{version_id}/tests",
    operation_id = "run_skill_test",
    tag = "skills",
    request_body = RunSkillTestBody,
    params(
        ("id" = String, Path, format = "uuid"),
        ("version_id" = String, Path, format = "uuid"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry.")
    ),
    responses(
        (status = 201, description = "Test run", body = SkillTestRunView),
        (status = 200, description = "Idempotent replay", body = SkillTestRunView),
        (status = 400, description = "Unsupported harness", body = ApiErrorBody),
        (status = 404, description = "Absent or denied", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "skills.test.run", skip_all)]
pub(crate) async fn run_test(
    State(state): State<AppState>,
    Path((id, version_id)): Path<(SkillId, SkillVersionId)>,
    headers: HeaderMap,
    payload: std::result::Result<Json<RunSkillTestBody>, JsonRejection>,
) -> Response {
    let result: Result<(StatusCode, Json<SkillTestRunView>)> = async {
        let body = body(payload)?;
        let harness: SkillTestHarness = body.harness.parse()?;
        if harness != SkillTestHarness::ValidationSandbox {
            return Err(Error::Invalid {
                message: "the gateway runs only validation_sandbox; controlled_client results enter through an identified adapter harness"
                    .to_owned(),
            });
        }
        let tenant = tenant_id()?;
        let canonical = json!({
            "skill_id": id,
            "version_id": version_id,
            "body": body,
        });
        let claim = Claim::from_headers(&headers, "skill.test", &subject()?, &canonical)?;
        if let Dispatch::Replay(run_id) =
            crate::idempotency::dispatch(&state.pool, tenant, &claim).await?
        {
            let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
            let (skill, _, _) =
                exact_visible(&state, &mut tx, tenant, id, Some(version_id)).await?;
            let scope = scope_for(&mut tx, tenant, skill.governing_scope_id).await?;
            let input = authz::gather(
                &state,
                &mut tx,
                Some(&scope),
                AnchorSelection::none(),
                Vec::new(),
            )
            .await?;
            authz::decide(
                &state,
                &input,
                Action::SkillWrite,
                Resource::Scope(scope.id),
            )?;
            let stored = store::test_run(
                &mut *tx,
                tenant,
                SkillTestRunId::from_uuid(run_id),
            )
            .await?
            .filter(|run| run.version_id == version_id)
            .ok_or_else(|| Error::Conflict {
                message: "the idempotent Skill test result is no longer available".to_owned(),
            })?;
            commit(tx).await?;
            return Ok((StatusCode::OK, Json(stored.into())));
        }
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let (skill, version, _) =
            exact_visible(&state, &mut tx, tenant, id, Some(version_id)).await?;
        let scope = scope_for(&mut tx, tenant, skill.governing_scope_id).await?;
        let input = authz::gather(&state, &mut tx, Some(&scope), AnchorSelection::none(), Vec::new())
            .await?;
        authz::decide(
            &state,
            &input,
            Action::SkillWrite,
            Resource::Scope(scope.id),
        )?;
        let actor = identity_of(&input, "testing a skill")?;
        let bundle = version_bundle(&mut tx, tenant, &skill, &version).await?;
        let manifest = bundle.validate()?;
        let scan = scan_security(&bundle.files).await?;
        let quality = score_quality(&bundle.files).await?;
        let evidence = json!({
            "executes_bundle_code": false,
            "validated_manifest": true,
            "files": bundle.files.len(),
            "declared_tools": manifest.allowed_tools,
            "declared_tools_are_authorization": false,
            "security_findings": scan.files.iter().map(|file| file.findings.len()).sum::<usize>(),
            "quality_score": quality.score,
            "agent_skills_spec_commit": synveda_types::AGENT_SKILLS_SPEC_COMMIT,
        });
        let stored = store::record_test_run(
            &mut *tx,
            tenant,
            SkillTestRunId::new(),
            version.id,
            harness,
            VALIDATION_HARNESS_VERSION,
            SkillTestOutcome::Passed,
            scan.ruleset_version,
            quality.rubric_version,
            &evidence,
            actor,
        )
        .await?;
        claim
            .remember(&mut tx, tenant, stored.id.as_uuid())
            .await?;
        audit::record(
            &mut tx,
            tenant,
            AuditAction::SkillTestRecorded,
            Resource::Scope(skill.governing_scope_id).to_string(),
            Outcome::Success,
            json!({
                "test_run_id": stored.id,
                "skill_id": skill.id,
                "version_id": version.id,
                "harness": harness.as_str(),
                "harness_version": VALIDATION_HARNESS_VERSION,
                "outcome": stored.outcome.as_str(),
                "executes_bundle_code": false,
            }),
        )
        .await?;
        commit(tx).await?;
        Ok((StatusCode::CREATED, Json(SkillTestRunView::from(stored))))
    }
    .await;
    respond(&state, "test_run", result).await
}

/// List controlled test runs for one immutable version.
#[utoipa::path(
    get,
    path = "/v1/skills/{id}/versions/{version_id}/tests",
    operation_id = "list_skill_tests",
    tag = "skills",
    params(
        ("id" = String, Path, format = "uuid"),
        ("version_id" = String, Path, format = "uuid"),
        ListTestRunsParams
    ),
    responses(
        (status = 200, description = "Test runs", body = SkillTestRunListView),
        (status = 404, description = "Absent or denied", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
pub(crate) async fn list_tests(
    State(state): State<AppState>,
    Path((id, version_id)): Path<(SkillId, SkillVersionId)>,
    Query(params): Query<ListTestRunsParams>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let take = limit(params.limit)?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        exact_visible(&state, &mut tx, tenant, id, Some(version_id)).await?;
        let candidates =
            store::test_runs(&mut tx, tenant, version_id, params.cursor, take + 1).await?;
        let more = candidates.len() as i64 > take;
        let considered = candidates
            .into_iter()
            .take(take as usize)
            .collect::<Vec<_>>();
        let next_cursor = more.then(|| considered.last().map(|run| run.id)).flatten();
        let runs = considered.into_iter().map(Into::into).collect();
        commit(tx).await?;
        Ok(Json(SkillTestRunListView { runs, next_cursor }))
    }
    .await;
    respond(&state, "tests", result).await
}

async fn respond<T: IntoResponse>(
    state: &AppState,
    operation: &'static str,
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
    metrics::counter!(SKILL_OPERATIONS_TOTAL, "op" => operation, "outcome" => outcome).increment(1);
    match result {
        Ok(value) => value.into_response(),
        Err(error) => {
            audit::record_rejection(state, operation, &error).await;
            ApiError(error).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, content: &str) -> SkillFileBody {
        SkillFileBody {
            path: path.to_owned(),
            content: content.to_owned(),
        }
    }

    #[test]
    fn official_manifest_fields_and_tools_are_metadata_only() {
        let bundle = SkillBundle {
            name: "release-notes".parse().unwrap(),
            files: vec![SkillFile {
                path: "SKILL.md".parse().unwrap(),
                content: "---\nname: release-notes\ndescription: Prepare release notes\nlicense: Apache-2.0\ncompatibility: Requires git\nallowed-tools: Read Grep\nmetadata:\n  owner: platform\n---\nDo the work.\n"
                    .to_owned(),
            }],
        };
        let manifest = bundle.validate().unwrap();
        assert_eq!(manifest.allowed_tools, ["Read", "Grep"]);
        assert_eq!(manifest.compatibility.as_deref(), Some("Requires git"));
        assert!(
            !SkillVersionView {
                id: SkillVersionId::new(),
                skill_id: SkillId::new(),
                ordinal: 1,
                bundle_digest: "00".repeat(32),
                sensitivity: "internal".to_owned(),
                manifest: serde_json::to_value(manifest).unwrap(),
                source_kind: "authored".to_owned(),
                provenance: json!({}),
                scan: json!({}),
                scan_ruleset_version: 1,
                quality_score: 100,
                rubric_version: 1,
                declared_tools_are_authorization: false,
                created_at: Utc::now(),
                created_by: IdentityId::new(),
            }
            .declared_tools_are_authorization
        );
    }

    #[test]
    fn bundle_digest_is_path_order_independent() {
        let a = SkillVersionFileRef {
            path: "SKILL.md".parse().unwrap(),
            object_hash: "11".repeat(32),
            chars: 1,
        };
        let b = SkillVersionFileRef {
            path: "references/a.md".parse().unwrap(),
            object_hash: "22".repeat(32),
            chars: 1,
        };
        assert_eq!(
            bundle_digest(&[a.clone(), b.clone()]).unwrap(),
            bundle_digest(&[b, a]).unwrap()
        );
    }

    #[test]
    fn api_bundle_fields_reject_bad_paths() {
        let supplied = file("../escape", "no");
        assert!(supplied.path.parse::<SkillFilePath>().is_err());
    }
}
