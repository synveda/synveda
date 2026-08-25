//! Public OKF v0.2 exchange boundary (CPR-27, ADR-0087).
//!
//! Clients submit inert, bounded bytes. The gateway never opens a caller's
//! filesystem, fetches a URL, runs Git or executes bundle content. Import
//! plans are immutable dry runs; their sole materialisation target is the
//! ordinary capture-candidate review flow.

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use synveda_audit::{AuditAction, Outcome};
use synveda_okf::{
    ArtifactKind, BundleInput, ExportKnowledge, ExportRelation, ExportSource, InputEntry,
    InputEntryKind, KnowledgeFormatAdapter, OkfAdapter, SourceDescriptor, SourceKind,
};
use synveda_policy::{Action, Resource};
use synveda_store::anchors::AnchorSelection;
use synveda_store::imports::{self as store, NewImportArtifact, NewImportMapping, NewImportPlan};
use synveda_store::knowledge::{self as knowledge_store, KnowledgeSnapshot};
use synveda_store::knowledge_search::{self as search, Filters};
use synveda_store::{projects, rls, scopes};
use synveda_types::import::{
    ImportArtifact, ImportArtifactKind, ImportJob, ImportMapping, ImportMappingClassification,
};
use synveda_types::knowledge::KnowledgeLifecycleState;
use synveda_types::{
    Error, ImportArtifactId, ImportJobId, ImportMappingId, KnowledgeItemId, ProjectId, Result,
    Sensitivity, TenantId,
};
use utoipa::ToSchema;

use crate::app::AppState;
use crate::audit;
use crate::authz::{self, Authorized};
use crate::capture::{CaptureBatchView, CaptureCandidateView};
use crate::idempotency::{Claim, Dispatch};
use crate::knowledge_api::KnowledgeContentBody;
use crate::request::{body, commit, tenant_id};
use crate::workspaces::{ApiErrorBody, subject};

/// OKF API outcomes by operation and `ok|rejected|error`.
pub const OKF_API_OPERATIONS_TOTAL: &str = "synveda_okf_api_operations_total";
const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;
const MAX_EXPORT_ITEMS: i64 = 2_000;

/// One inert entry supplied by a local client.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OkfInputEntryBody {
    /// Bundle-relative slash-separated path.
    pub logical_path: String,
    /// `file`, `directory`, `symlink` or `special`.
    pub kind: String,
    /// Exact bytes encoded with standard base64; omitted for directory markers.
    #[serde(default)]
    pub content_base64: String,
}

/// Immutable import-plan request.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanOkfImportBody {
    /// Directory, zip, tar or Git source label.
    pub source_kind: String,
    /// Credential-free source identity. It is retained, never fetched.
    pub source_locator: String,
    /// Required for Git and retained for provenance.
    #[serde(default)]
    pub source_revision: Option<String>,
    /// `entries`, `zip`, `tar` or `tar_gzip`.
    pub encoding: String,
    /// Enumerated inert entries for directory or checked-out Git input.
    #[serde(default)]
    pub entries: Vec<OkfInputEntryBody>,
    /// Standard-base64 archive bytes for an archive encoding.
    #[serde(default)]
    pub archive_base64: Option<String>,
}

/// One immutable admitted artifact.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OkfArtifactView {
    /// Stable artifact id.
    #[schema(value_type = String, format = "uuid")]
    pub id: ImportArtifactId,
    /// Stable order.
    pub ordinal: i32,
    /// Safe logical path.
    pub logical_path: String,
    /// Concept, index or log.
    pub kind: String,
    /// Admitted-byte digest.
    pub content_hash: String,
    /// Parsed extension-preserving frontmatter.
    #[schema(value_type = Object)]
    pub frontmatter: Value,
    /// Markdown body after frontmatter.
    pub body_markdown: String,
}

impl From<ImportArtifact> for OkfArtifactView {
    fn from(artifact: ImportArtifact) -> Self {
        Self {
            id: artifact.id,
            ordinal: artifact.ordinal,
            logical_path: artifact.logical_path,
            kind: artifact.kind.as_str().to_owned(),
            content_hash: artifact.content_hash,
            frontmatter: artifact.frontmatter,
            body_markdown: artifact.body_markdown,
        }
    }
}

/// One immutable proposed concept mapping.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OkfMappingView {
    /// Stable mapping id.
    #[schema(value_type = String, format = "uuid")]
    pub id: ImportMappingId,
    /// Source artifact.
    #[schema(value_type = String, format = "uuid")]
    pub artifact_id: ImportArtifactId,
    /// Stable concept order.
    pub ordinal: i32,
    /// Exact producer-defined type, including unknown values.
    pub okf_type: String,
    /// Proposed Synveda type.
    pub knowledge_type: String,
    /// Complete proposed immutable content.
    pub content: KnowledgeContentBody,
    /// Semantic content digest.
    pub content_hash: String,
    /// Addition, update, duplicate or conflict.
    pub classification: String,
    /// Independently visible match, when any.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub matched_item_id: Option<KnowledgeItemId>,
    /// Exact visible revision compared.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub matched_revision_id: Option<synveda_types::KnowledgeRevisionId>,
    /// Proposed internal links.
    #[schema(value_type = Object)]
    pub proposed_relations: Value,
    /// Whether external lifecycle permits a candidate.
    pub materializable: bool,
    /// Candidate created on materialisation.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub candidate_id: Option<synveda_types::CaptureCandidateId>,
}

impl From<ImportMapping> for OkfMappingView {
    fn from(mapping: ImportMapping) -> Self {
        let content = mapping.content;
        Self {
            id: mapping.id,
            artifact_id: mapping.artifact_id,
            ordinal: mapping.ordinal,
            okf_type: mapping.okf_type,
            knowledge_type: mapping.knowledge_type.as_str().to_owned(),
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
            content_hash: mapping.content_hash,
            classification: mapping.classification.as_str().to_owned(),
            matched_item_id: mapping.matched_item_id,
            matched_revision_id: mapping.matched_revision_id,
            proposed_relations: mapping.proposed_relations,
            materializable: mapping.materializable,
            candidate_id: mapping.candidate_id,
        }
    }
}

/// Import operation summary.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OkfImportJobView {
    /// Stable job id.
    #[schema(value_type = String, format = "uuid")]
    pub id: ImportJobId,
    /// Target project.
    #[schema(value_type = String, format = "uuid")]
    pub project_id: ProjectId,
    /// Exact adapter format and version.
    pub format: String,
    /// Exact implemented format version.
    pub format_version: String,
    /// Pinned official specification commit.
    pub specification_commit: String,
    /// Directory, zip, tar or Git.
    pub source_kind: String,
    /// Credential-free retained source identity.
    pub source_locator: String,
    /// Upstream revision when reported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_revision: Option<String>,
    /// Canonical admitted-bundle digest.
    pub bundle_digest: String,
    /// Planned, materialized or failed.
    pub state: String,
    /// Immutable artifact count.
    pub artifact_count: i32,
    /// Immutable mapping count.
    pub mapping_count: i32,
    /// Reviewable candidate count.
    pub candidate_count: i32,
    /// Resulting candidate batch.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub capture_batch_id: Option<synveda_types::CaptureBatchId>,
    /// Content-free validation notices.
    pub notices: Vec<String>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Terminal time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

impl From<ImportJob> for OkfImportJobView {
    fn from(job: ImportJob) -> Self {
        Self {
            id: job.id,
            project_id: job.project_id,
            format: job.format,
            format_version: job.format_version,
            specification_commit: job.specification_commit,
            source_kind: job.source_kind,
            source_locator: job.source_locator,
            source_revision: job.source_revision,
            bundle_digest: job.bundle_digest,
            state: job.state.as_str().to_owned(),
            artifact_count: job.artifact_count,
            mapping_count: job.mapping_count,
            candidate_count: job.candidate_count,
            capture_batch_id: job.capture_batch_id,
            notices: job.notices,
            created_at: job.created_at,
            completed_at: job.completed_at,
        }
    }
}

/// Complete persisted dry-run plan.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OkfImportPlanView {
    /// Operation summary.
    pub job: OkfImportJobView,
    /// Immutable admitted artifacts.
    pub artifacts: Vec<OkfArtifactView>,
    /// Immutable proposed mappings.
    pub mappings: Vec<OkfMappingView>,
}

/// Keyset-paginated import jobs.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OkfImportJobListView {
    /// Visible jobs.
    pub jobs: Vec<OkfImportJobView>,
    /// Opaque continuation cursor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Candidate-only materialisation result.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OkfMaterializationView {
    /// Terminal job.
    pub job: OkfImportJobView,
    /// Completed capture batch.
    pub batch: CaptureBatchView,
    /// Reviewable candidates, never active Knowledge by this response alone.
    pub candidates: Vec<CaptureCandidateView>,
}

/// Explicit current-Knowledge export selection. Empty means all visible
/// current active/stale Knowledge in the project, bounded at 2000 items.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportOkfBody {
    /// Stable item ids to export.
    #[serde(default)]
    #[schema(value_type = Vec<String>)]
    pub item_ids: Vec<KnowledgeItemId>,
}

/// One deterministic OKF output file.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OkfExportFileView {
    /// Stable bundle-relative path.
    pub logical_path: String,
    /// Exact UTF-8 Markdown.
    pub content: String,
    /// Exact content digest.
    pub content_hash: String,
}

/// Deterministic export response.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OkfExportView {
    /// Exact supported version.
    pub format_version: String,
    /// Pinned official specification commit.
    pub specification_commit: String,
    /// Stable ordered output files.
    pub files: Vec<OkfExportFileView>,
    /// Digest over ordered paths and hashes.
    pub bundle_digest: String,
}

/// Import collection filters.
#[derive(Debug, Clone, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListOkfImportsParams {
    /// Exact project.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "uuid")]
    pub project_id: Option<ProjectId>,
    /// Planned, materialized or failed.
    #[serde(default)]
    pub state: Option<String>,
    /// Opaque continuation cursor.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Rows to serve, 1–200.
    #[serde(default)]
    pub limit: Option<i64>,
}

fn source_kind(value: &str) -> Result<SourceKind> {
    match value {
        "directory" => Ok(SourceKind::Directory),
        "zip" => Ok(SourceKind::Zip),
        "tar" => Ok(SourceKind::Tar),
        "git" => Ok(SourceKind::Git),
        _ => Err(Error::Invalid {
            message: format!("unknown OKF source kind: {value:?}"),
        }),
    }
}

fn input_entry_kind(value: &str) -> Result<InputEntryKind> {
    match value {
        "file" => Ok(InputEntryKind::File),
        "directory" => Ok(InputEntryKind::Directory),
        "symlink" => Ok(InputEntryKind::Symlink),
        "special" => Ok(InputEntryKind::Special),
        _ => Err(Error::Invalid {
            message: format!("unknown OKF entry kind: {value:?}"),
        }),
    }
}

fn bundle_input(body: &PlanOkfImportBody) -> Result<(SourceDescriptor, BundleInput)> {
    let kind = source_kind(&body.source_kind)?;
    let source = SourceDescriptor {
        kind,
        locator: body.source_locator.clone(),
        revision: body.source_revision.clone(),
    };
    let input = match body.encoding.as_str() {
        "entries" => {
            if body.archive_base64.is_some() {
                return Err(Error::Invalid {
                    message: "entry input must not also carry archive_base64".to_owned(),
                });
            }
            if !matches!(kind, SourceKind::Directory | SourceKind::Git) {
                return Err(Error::Invalid {
                    message: "entry input requires a directory or git source".to_owned(),
                });
            }
            if body.entries.len() > synveda_okf::MAX_ARTIFACTS {
                return Err(Error::Invalid {
                    message: format!("OKF input exceeds {} entries", synveda_okf::MAX_ARTIFACTS),
                });
            }
            let mut entries = Vec::with_capacity(body.entries.len());
            let mut encoded_total = 0usize;
            for entry in &body.entries {
                encoded_total = encoded_total
                    .checked_add(entry.content_base64.len())
                    .ok_or_else(|| Error::Invalid {
                        message: "OKF entry byte total overflowed".to_owned(),
                    })?;
                if encoded_total > synveda_okf::MAX_EXPANDED_BYTES.saturating_mul(2) {
                    return Err(Error::Invalid {
                        message: "OKF entry envelope exceeds the encoded-byte limit".to_owned(),
                    });
                }
                let entry_kind = input_entry_kind(&entry.kind)?;
                let bytes = STANDARD
                    .decode(&entry.content_base64)
                    .map_err(|_| Error::Invalid {
                        message: format!(
                            "OKF entry {} has invalid standard base64 content",
                            entry.logical_path
                        ),
                    })?;
                if entry_kind != InputEntryKind::File && !bytes.is_empty() {
                    return Err(Error::Invalid {
                        message: format!(
                            "non-file OKF entry {} must not carry content",
                            entry.logical_path
                        ),
                    });
                }
                entries.push(InputEntry {
                    logical_path: entry.logical_path.clone(),
                    kind: entry_kind,
                    bytes,
                });
            }
            BundleInput::Entries(entries)
        }
        encoding @ ("zip" | "tar" | "tar_gzip") => {
            if !body.entries.is_empty() {
                return Err(Error::Invalid {
                    message: "archive input must not also carry entries".to_owned(),
                });
            }
            if (encoding == "zip" && kind != SourceKind::Zip)
                || (encoding != "zip" && kind != SourceKind::Tar)
            {
                return Err(Error::Invalid {
                    message: "OKF archive encoding and source kind disagree".to_owned(),
                });
            }
            let encoded = body
                .archive_base64
                .as_deref()
                .ok_or_else(|| Error::Invalid {
                    message: "archive input requires archive_base64".to_owned(),
                })?;
            if encoded.len() > synveda_okf::MAX_ARCHIVE_BYTES.saturating_mul(2) {
                return Err(Error::Invalid {
                    message: "OKF archive envelope exceeds the encoded-byte limit".to_owned(),
                });
            }
            let bytes = STANDARD.decode(encoded).map_err(|_| Error::Invalid {
                message: "OKF archive has invalid standard base64 content".to_owned(),
            })?;
            match encoding {
                "zip" => BundleInput::Zip(bytes),
                "tar" => BundleInput::Tar(bytes),
                "tar_gzip" => BundleInput::TarGzip(bytes),
                _ => unreachable!("closed above"),
            }
        }
        value => {
            return Err(Error::Invalid {
                message: format!("unknown OKF bundle encoding: {value:?}"),
            });
        }
    };
    Ok((source, input))
}

fn list_limit(raw: Option<i64>) -> Result<i64> {
    let limit = raw.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(Error::Invalid {
            message: format!("limit must be between 1 and {MAX_LIMIT}"),
        });
    }
    Ok(limit)
}

fn encode_cursor(at: DateTime<Utc>, id: ImportJobId) -> String {
    URL_SAFE_NO_PAD.encode(format!(
        "oi1|{}|{id}",
        at.to_rfc3339_opts(SecondsFormat::Nanos, true)
    ))
}

fn decode_cursor(raw: &str) -> Result<store::ImportCursor> {
    let invalid = || Error::Invalid {
        message: "invalid OKF import cursor".to_owned(),
    };
    let decoded = URL_SAFE_NO_PAD.decode(raw).map_err(|_| invalid())?;
    let decoded = String::from_utf8(decoded).map_err(|_| invalid())?;
    let mut parts = decoded.split('|');
    if parts.next() != Some("oi1") {
        return Err(invalid());
    }
    let created_at = DateTime::parse_from_rfc3339(parts.next().ok_or_else(invalid)?)
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
    Ok(store::ImportCursor { created_at, id })
}

async fn authorize_project(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    id: ProjectId,
    action: Action,
) -> Result<(synveda_types::workspace::Project, Authorized, Resource)> {
    let project = projects::get(&mut *tx, tenant, id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("project {id}"),
        })?;
    let scope = scopes::get(&mut *tx, tenant, project.scope_id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("scope {}", project.scope_id),
        })?;
    let input = authz::gather(
        state,
        tx,
        Some(&scope),
        AnchorSelection::project(id),
        Vec::new(),
    )
    .await?;
    let resource = Resource::Scope(project.scope_id);
    let allowed = if action == Action::KnowledgeRead {
        authz::decide_knowledge_read(state, &input, resource, Sensitivity::Public)?
    } else {
        authz::decide(state, &input, action, resource)?
    };
    Ok((project, allowed, resource))
}

async fn load_job(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    id: ImportJobId,
    action: Action,
) -> Result<(ImportJob, Authorized, Resource)> {
    let job = store::get_job(&mut *tx, tenant, id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("import job {id}"),
        })?;
    let (_, allowed, resource) =
        authorize_project(state, tx, tenant, job.project_id, action).await?;
    Ok((job, allowed, resource))
}

async fn plan_view(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    job: ImportJob,
) -> Result<OkfImportPlanView> {
    let artifacts = store::artifacts(&mut *tx, tenant, job.id).await?;
    let mappings = store::mappings(&mut *tx, tenant, job.id).await?;
    Ok(OkfImportPlanView {
        job: job.into(),
        artifacts: artifacts.into_iter().map(Into::into).collect(),
        mappings: mappings.into_iter().map(Into::into).collect(),
    })
}

async fn read_event(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    op: &'static str,
    allowed: &Authorized,
    resource: Resource,
    detail: Value,
) -> Result<()> {
    audit::record(
        tx,
        tenant,
        AuditAction::AuthzDecision,
        resource.to_string(),
        Outcome::Allow,
        json!({
            "op": op,
            "authz": audit::decision_context(Action::KnowledgeRead, allowed),
            "detail": detail,
        }),
    )
    .await
    .map(|_| ())
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
    metrics::counter!(OKF_API_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    match result {
        Ok(response) => response.into_response(),
        Err(error) => {
            audit::record_rejection(state, op, &error).await;
            ApiError(error).into_response()
        }
    }
}

use crate::error::ApiError;

fn knowledge_filters(project_id: ProjectId, lifecycle: KnowledgeLifecycleState) -> Filters {
    Filters {
        scope_ids: Vec::new(),
        workspace_id: None,
        project_id: Some(project_id),
        scope_id: None,
        owner_principal_id: None,
        knowledge_type: None,
        origin: None,
        lifecycle: Some(lifecycle),
        tag: None,
        source_type: None,
        updated_from: None,
        updated_before: None,
        stale: None,
        at: Utc::now(),
        as_known_at: Utc::now(),
        include_history: false,
        include_transitional: false,
    }
}

async fn visible_project_knowledge(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    project_id: ProjectId,
) -> Result<Vec<KnowledgeSnapshot>> {
    let mut item_ids = Vec::new();
    for lifecycle in [
        KnowledgeLifecycleState::Active,
        KnowledgeLifecycleState::Stale,
    ] {
        let candidates = search::list_candidates(
            tx,
            tenant,
            &knowledge_filters(project_id, lifecycle),
            None,
            MAX_EXPORT_ITEMS + 1,
        )
        .await?;
        item_ids.extend(candidates.into_iter().map(|candidate| candidate.item_id));
    }
    item_ids.sort_unstable();
    item_ids.dedup();
    if item_ids.len() as i64 > MAX_EXPORT_ITEMS {
        return Err(Error::Invalid {
            message: format!(
                "project has more than {MAX_EXPORT_ITEMS} current Knowledge items; select explicit ids"
            ),
        });
    }
    let mut visible = Vec::new();
    for item_id in item_ids {
        let Some(snapshot) = knowledge_store::current(&mut *tx, tenant, item_id).await? else {
            continue;
        };
        match crate::knowledge_api::authorize_snapshot(state, tx, tenant, &snapshot).await {
            Ok(_) => visible.push(snapshot),
            Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => {}
            Err(error) => return Err(error),
        }
    }
    visible.sort_by_key(|snapshot| snapshot.item.id);
    Ok(visible)
}

fn matched_snapshot<'a>(
    concept: &synveda_okf::ProposedConcept,
    current: &'a [KnowledgeSnapshot],
) -> (ImportMappingClassification, Option<&'a KnowledgeSnapshot>) {
    if let Some(snapshot) = current
        .iter()
        .find(|snapshot| snapshot.revision.content_hash == concept.content_hash)
    {
        return (ImportMappingClassification::Duplicate, Some(snapshot));
    }
    if let Some(snapshot) = current.iter().find(|snapshot| {
        snapshot
            .revision
            .content
            .metadata
            .get("okf")
            .and_then(|okf| okf.get("logical_path"))
            .and_then(Value::as_str)
            == Some(concept.logical_path.as_str())
    }) {
        return (ImportMappingClassification::Update, Some(snapshot));
    }
    let title = concept.content.title.trim().to_lowercase();
    if let Some(snapshot) = current
        .iter()
        .find(|snapshot| snapshot.revision.content.title.trim().to_lowercase() == title)
    {
        return (ImportMappingClassification::Conflict, Some(snapshot));
    }
    (ImportMappingClassification::Addition, None)
}

fn stored_artifact_kind(kind: ArtifactKind) -> ImportArtifactKind {
    match kind {
        ArtifactKind::Concept => ImportArtifactKind::Concept,
        ArtifactKind::Index => ImportArtifactKind::Index,
        ArtifactKind::Log => ImportArtifactKind::Log,
    }
}

fn new_plan(
    tenant: TenantId,
    project: &synveda_types::workspace::Project,
    actor: String,
    inspection: synveda_okf::BundleInspection,
    current: &[KnowledgeSnapshot],
) -> NewImportPlan {
    let artifact_ids: Vec<ImportArtifactId> = inspection
        .artifacts
        .iter()
        .map(|_| ImportArtifactId::new())
        .collect();
    let artifacts = inspection
        .artifacts
        .into_iter()
        .enumerate()
        .map(|(index, artifact)| NewImportArtifact {
            id: artifact_ids[index],
            ordinal: (index + 1) as i32,
            logical_path: artifact.logical_path,
            kind: stored_artifact_kind(artifact.kind),
            content_hash: artifact.content_hash,
            frontmatter: artifact.frontmatter,
            body_markdown: artifact.body_markdown,
        })
        .collect();
    let mappings = inspection
        .concepts
        .into_iter()
        .enumerate()
        .map(|(index, concept)| {
            let (classification, matched) = matched_snapshot(&concept, current);
            NewImportMapping {
                id: ImportMappingId::new(),
                artifact_id: artifact_ids[concept.artifact_index],
                ordinal: (index + 1) as i32,
                okf_type: concept.okf_type,
                knowledge_type: concept.knowledge_type,
                content: concept.content,
                content_hash: concept.content_hash,
                classification,
                matched_item_id: matched.map(|snapshot| snapshot.item.id),
                matched_revision_id: matched.map(|snapshot| snapshot.revision.id),
                proposed_relations: json!(concept.links),
                materializable: concept.materializable
                    && classification != ImportMappingClassification::Duplicate,
            }
        })
        .collect();
    NewImportPlan {
        id: ImportJobId::new(),
        tenant_id: tenant,
        project_id: project.id,
        scope_id: project.scope_id,
        workspace_id: project.workspace_id,
        principal_id: actor,
        format: "okf".to_owned(),
        format_version: inspection.format_version,
        specification_commit: inspection.specification_commit,
        source_kind: inspection.source.kind.as_str().to_owned(),
        source_locator: inspection.source.locator,
        source_revision: inspection.source.revision,
        bundle_digest: inspection.bundle_digest,
        notices: inspection.notices,
        artifacts,
        mappings,
    }
}

/// Plan one bounded OKF v0.2 import. This never creates active Knowledge.
#[utoipa::path(
    post,
    path = "/v1/projects/{project_id}/okf/imports",
    operation_id = "plan_okf_import",
    tag = "okf",
    params(
        ("project_id" = String, Path, description = "Target project"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry."),
    ),
    request_body = PlanOkfImportBody,
    responses(
        (status = 201, description = "Immutable dry-run plan created", body = OkfImportPlanView),
        (status = 200, description = "Idempotent or unchanged-bundle replay", body = OkfImportPlanView),
        (status = 400, description = "Invalid or unsafe bundle", body = ApiErrorBody),
        (status = 403, description = "The PDP denied Knowledge writing", body = ApiErrorBody),
        (status = 404, description = "No such project", body = ApiErrorBody),
        (status = 409, description = "Changed idempotency request", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "okf.import.plan", skip_all, fields(project.id = %project_id))]
pub(crate) async fn plan_import(
    State(state): State<AppState>,
    Path(project_id): Path<ProjectId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<PlanOkfImportBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant = tenant_id()?;
        let actor = subject()?;
        let canonical = json!({
            "route": "/v1/projects/{project_id}/okf/imports",
            "project_id": project_id,
            "body": body,
        });
        let claim = Claim::from_headers(&headers, "okf.import.plan", &actor, &canonical)?;
        {
            let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
            authorize_project(&state, &mut tx, tenant, project_id, Action::KnowledgeWrite).await?;
            commit(tx).await?;
        }
        if let Dispatch::Replay(id) =
            crate::idempotency::dispatch(&state.pool, tenant, &claim).await?
        {
            let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
            let (job, _, _) = load_job(
                &state,
                &mut tx,
                tenant,
                ImportJobId::from_uuid(id),
                Action::KnowledgeRead,
            )
            .await?;
            let view = plan_view(&mut tx, tenant, job).await?;
            commit(tx).await?;
            return Ok((StatusCode::OK, Json(view)));
        }
        let (source, input) = bundle_input(&body)?;
        let inspection = OkfAdapter.inspect(source, input, Utc::now())?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let (fresh_project, allowed, resource) =
            authorize_project(&state, &mut tx, tenant, project_id, Action::KnowledgeWrite).await?;
        let current = visible_project_knowledge(&state, &mut tx, tenant, project_id).await?;
        let proposed = new_plan(tenant, &fresh_project, actor, inspection, &current);
        let planned = store::create_plan(&mut tx, &proposed).await?;
        claim
            .remember(&mut tx, tenant, planned.job.id.as_uuid())
            .await?;
        if planned.created {
            audit::record(
                &mut tx,
                tenant,
                AuditAction::OkfImportPlanned,
                resource.to_string(),
                Outcome::Success,
                json!({
                    "import_job_id": planned.job.id,
                    "project_id": project_id,
                    "format_version": planned.job.format_version,
                    "specification_commit": planned.job.specification_commit,
                    "bundle_digest": planned.job.bundle_digest,
                    "artifact_count": planned.job.artifact_count,
                    "mapping_count": planned.job.mapping_count,
                    "notice_count": planned.job.notices.len(),
                    "authz": audit::decision_context(Action::KnowledgeWrite, &allowed),
                }),
            )
            .await?;
        }
        let status = if planned.created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        };
        let view = plan_view(&mut tx, tenant, planned.job).await?;
        commit(tx).await?;
        Ok((status, Json(view)))
    }
    .await;
    respond(&state, "okf.import.plan", result).await
}

/// List visible import jobs with a true keyset cursor.
#[utoipa::path(
    get,
    path = "/v1/okf/imports",
    operation_id = "list_okf_imports",
    tag = "okf",
    params(ListOkfImportsParams),
    responses(
        (status = 200, description = "Visible import jobs", body = OkfImportJobListView),
        (status = 400, description = "Invalid filter", body = ApiErrorBody),
        (status = 403, description = "The PDP denied Knowledge reading", body = ApiErrorBody),
        (status = 404, description = "No such project", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn list_imports(
    State(state): State<AppState>,
    Query(params): Query<ListOkfImportsParams>,
) -> Response {
    let result =
        async {
            let tenant = tenant_id()?;
            let limit = list_limit(params.limit)?;
            let state_filter = params.state.as_deref().map(str::parse).transpose()?;
            let cursor = params.cursor.as_deref().map(decode_cursor).transpose()?;
            let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
            let mut gate = if let Some(project_id) = params.project_id {
                let (_, allowed, resource) =
                    authorize_project(&state, &mut tx, tenant, project_id, Action::KnowledgeRead)
                        .await?;
                Some((allowed, resource))
            } else {
                None
            };
            let scanned =
                store::list_jobs(&mut *tx, tenant, params.project_id, state_filter, cursor).await?;
            let total = scanned.len();
            let mut consumed = 0usize;
            let mut last = None;
            let mut jobs = Vec::new();
            for job in scanned {
                consumed += 1;
                last = Some((job.created_at, job.id));
                match authorize_project(
                    &state,
                    &mut tx,
                    tenant,
                    job.project_id,
                    Action::KnowledgeRead,
                )
                .await
                {
                    Ok((_, allowed, resource)) => {
                        gate.get_or_insert((allowed, resource));
                        jobs.push(job.into());
                    }
                    Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => {}
                    Err(error) => return Err(error),
                }
                if jobs.len() as i64 == limit {
                    break;
                }
            }
            if gate.is_none() {
                let input = authz::gather_at_home(&state, &mut tx).await?;
                let scope_id = input.chain.first().map(|scope| scope.id).ok_or_else(|| {
                    Error::PolicyDenied {
                        action: Action::KnowledgeRead.as_str().to_owned(),
                        resource: "OKF imports".to_owned(),
                        reason: "the caller has no governed principal scope".to_owned(),
                    }
                })?;
                let resource = Resource::Scope(scope_id);
                let allowed =
                    authz::decide_knowledge_read(&state, &input, resource, Sensitivity::Public)?;
                gate = Some((allowed, resource));
            }
            let more = consumed < total || total == store::IMPORT_SCAN_LIMIT as usize;
            let next_cursor = if more {
                last.map(|(at, id)| encode_cursor(at, id))
            } else {
                None
            };
            let (allowed, resource) = gate.expect("set above");
            read_event(
                &mut tx,
                tenant,
                "okf.import.list",
                &allowed,
                resource,
                json!({"served": jobs.len(), "more": next_cursor.is_some()}),
            )
            .await?;
            commit(tx).await?;
            Ok(Json(OkfImportJobListView { jobs, next_cursor }))
        }
        .await;
    respond(&state, "okf.import.list", result).await
}

/// Read one complete immutable dry-run plan.
#[utoipa::path(
    get,
    path = "/v1/okf/imports/{id}",
    operation_id = "get_okf_import",
    tag = "okf",
    params(("id" = String, Path, description = "Import job id")),
    responses(
        (status = 200, description = "Import plan", body = OkfImportPlanView),
        (status = 403, description = "The PDP denied Knowledge reading", body = ApiErrorBody),
        (status = 404, description = "No such import job", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn get_import(
    State(state): State<AppState>,
    Path(id): Path<ImportJobId>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let (job, allowed, resource) =
            load_job(&state, &mut tx, tenant, id, Action::KnowledgeRead).await?;
        let view = plan_view(&mut tx, tenant, job).await?;
        read_event(
            &mut tx,
            tenant,
            "okf.import.get",
            &allowed,
            resource,
            json!({
                "import_job_id": id,
                "artifact_count": view.artifacts.len(),
                "mapping_count": view.mappings.len(),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(view))
    }
    .await;
    respond(&state, "okf.import.get", result).await
}

/// Turn an immutable plan into reviewable candidates only.
#[utoipa::path(
    post,
    path = "/v1/okf/imports/{id}/materialize",
    operation_id = "materialize_okf_import",
    tag = "okf",
    params(
        ("id" = String, Path, description = "Import job id"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry."),
    ),
    responses(
        (status = 201, description = "Reviewable candidates created", body = OkfMaterializationView),
        (status = 200, description = "Idempotent replay", body = OkfMaterializationView),
        (status = 403, description = "The PDP denied Knowledge writing", body = ApiErrorBody),
        (status = 404, description = "No such import job", body = ApiErrorBody),
        (status = 409, description = "Changed retry or failed job", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn materialize_import(
    State(state): State<AppState>,
    Path(id): Path<ImportJobId>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let actor = subject()?;
        let claim = Claim::from_headers(
            &headers,
            "okf.import.materialize",
            &actor,
            &json!({"route": "/v1/okf/imports/{id}/materialize", "id": id}),
        )?;
        {
            let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
            load_job(&state, &mut tx, tenant, id, Action::KnowledgeWrite).await?;
            commit(tx).await?;
        }
        let replayed = match crate::idempotency::dispatch(&state.pool, tenant, &claim).await? {
            Dispatch::Create => false,
            Dispatch::Replay(resource_id) if resource_id == id.as_uuid() => true,
            Dispatch::Replay(resource_id) => {
                return Err(Error::Internal {
                    message: format!(
                        "OKF materialisation key resolved to {resource_id}, expected {id}"
                    ),
                });
            }
        };
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let (job, allowed, resource) =
            load_job(&state, &mut tx, tenant, id, Action::KnowledgeWrite).await?;
        let configuration =
            synveda_store::configuration::effective_at_scope(&mut tx, tenant, job.scope_id).await?;
        let result = store::materialize(&mut tx, tenant, id, &configuration).await?;
        if !replayed {
            claim.remember(&mut tx, tenant, id.as_uuid()).await?;
        }
        if result.created {
            audit::record(
                &mut tx,
                tenant,
                AuditAction::OkfImportMaterialized,
                resource.to_string(),
                Outcome::Success,
                json!({
                    "import_job_id": id,
                    "project_id": result.job.project_id,
                    "bundle_digest": result.job.bundle_digest,
                    "capture_batch_id": result.batch.id,
                    "candidate_count": result.candidates.len(),
                    "configuration_version_id": result.batch.configuration_version_id,
                    "configuration_hash": result.batch.configuration_hash,
                    "authz": audit::decision_context(Action::KnowledgeWrite, &allowed),
                }),
            )
            .await?;
        }
        let status = if result.created && !replayed {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        };
        let view = OkfMaterializationView {
            job: result.job.into(),
            batch: result.batch.into(),
            candidates: result.candidates.into_iter().map(Into::into).collect(),
        };
        commit(tx).await?;
        Ok((status, Json(view)))
    }
    .await;
    respond(&state, "okf.import.materialize", result).await
}

async fn selected_export_snapshots(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    project_id: ProjectId,
    item_ids: &[KnowledgeItemId],
) -> Result<Vec<KnowledgeSnapshot>> {
    if item_ids.is_empty() {
        return visible_project_knowledge(state, tx, tenant, project_id).await;
    }
    if item_ids.len() as i64 > MAX_EXPORT_ITEMS {
        return Err(Error::Invalid {
            message: format!("an OKF export contains at most {MAX_EXPORT_ITEMS} Knowledge items"),
        });
    }
    let mut ids = item_ids.to_vec();
    ids.sort_unstable();
    if ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(Error::Invalid {
            message: "OKF export item_ids must be unique".to_owned(),
        });
    }
    let mut snapshots = Vec::with_capacity(ids.len());
    for id in ids {
        let snapshot = knowledge_store::current(&mut *tx, tenant, id)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity: format!("Knowledge item {id}"),
            })?;
        if snapshot.item.project_id != Some(project_id)
            || !matches!(
                snapshot.item.lifecycle_state,
                KnowledgeLifecycleState::Active | KnowledgeLifecycleState::Stale
            )
        {
            return Err(Error::NotFound {
                entity: format!("current project Knowledge item {id}"),
            });
        }
        crate::knowledge_api::authorize_snapshot(state, tx, tenant, &snapshot).await?;
        snapshots.push(snapshot);
    }
    Ok(snapshots)
}

async fn export_sources(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    snapshot: &KnowledgeSnapshot,
) -> Result<Vec<ExportSource>> {
    let source_scopes = search::source_scopes(&mut *tx, tenant, snapshot.revision.id).await?;
    let mut visible_scopes = Vec::new();
    for scope_id in source_scopes {
        let Some(scope) = scopes::get(&mut *tx, tenant, scope_id).await? else {
            continue;
        };
        let input =
            authz::gather(state, tx, Some(&scope), AnchorSelection::none(), Vec::new()).await?;
        match authz::decide_knowledge_read(
            state,
            &input,
            Resource::Scope(scope_id),
            snapshot.revision.content.sensitivity,
        ) {
            Ok(_) => visible_scopes.push(scope_id),
            Err(Error::PolicyDenied { .. }) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(
        knowledge_store::visible_sources(&mut *tx, tenant, snapshot.revision.id, &visible_scopes)
            .await?
            .into_iter()
            .map(|source| ExportSource {
                id: source.id,
                source_type: source.source_type,
                locator: source.locator,
                source_revision: source.source_revision,
                content_hash: source.content_hash,
                metadata: source.metadata,
            })
            .collect(),
    )
}

async fn export_knowledge(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    snapshots: Vec<KnowledgeSnapshot>,
) -> Result<Vec<ExportKnowledge>> {
    let selected: BTreeMap<KnowledgeItemId, synveda_types::KnowledgeRevisionId> = snapshots
        .iter()
        .map(|snapshot| (snapshot.item.id, snapshot.revision.id))
        .collect();
    let mut output = Vec::with_capacity(snapshots.len());
    for snapshot in snapshots {
        let sources = export_sources(state, tx, tenant, &snapshot).await?;
        let relations = knowledge_store::relations(&mut *tx, tenant, snapshot.item.id)
            .await?
            .into_iter()
            .filter(|relation| {
                relation.source_item_id == snapshot.item.id
                    && relation.asserting_revision_id == snapshot.revision.id
                    && selected.contains_key(&relation.target_item_id)
            })
            .map(|relation| ExportRelation {
                target_item_id: relation.target_item_id,
                relation: relation.relation_type.as_str().to_owned(),
            })
            .collect();
        output.push(ExportKnowledge {
            item_id: snapshot.item.id,
            revision_id: snapshot.revision.id,
            knowledge_type: snapshot.item.knowledge_type,
            origin: snapshot.item.origin,
            lifecycle: snapshot.item.lifecycle_state,
            content: snapshot.revision.content,
            sources,
            relations,
        });
    }
    Ok(output)
}

/// Export freshly authorised current Knowledge deterministically as OKF v0.2.
#[utoipa::path(
    post,
    path = "/v1/projects/{project_id}/okf/exports",
    operation_id = "export_okf",
    tag = "okf",
    params(("project_id" = String, Path, description = "Source project")),
    request_body = ExportOkfBody,
    responses(
        (status = 200, description = "Deterministic OKF v0.2 bundle", body = OkfExportView),
        (status = 400, description = "Invalid selection", body = ApiErrorBody),
        (status = 403, description = "The PDP denied project, item or source reading", body = ApiErrorBody),
        (status = 404, description = "No such project or current selected item", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "okf.export", skip_all, fields(project.id = %project_id))]
pub(crate) async fn export(
    State(state): State<AppState>,
    Path(project_id): Path<ProjectId>,
    payload: std::result::Result<Json<ExportOkfBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let (_, allowed, resource) =
            authorize_project(&state, &mut tx, tenant, project_id, Action::KnowledgeRead).await?;
        let snapshots =
            selected_export_snapshots(&state, &mut tx, tenant, project_id, &body.item_ids).await?;
        let revision_ids: Vec<_> = snapshots
            .iter()
            .map(|snapshot| snapshot.revision.id)
            .collect();
        let items = export_knowledge(&state, &mut tx, tenant, snapshots).await?;
        let bundle = OkfAdapter.export(&items)?;
        audit::record(
            &mut tx,
            tenant,
            AuditAction::OkfExported,
            resource.to_string(),
            Outcome::Success,
            json!({
                "project_id": project_id,
                "format_version": bundle.format_version,
                "specification_commit": bundle.specification_commit,
                "bundle_digest": bundle.bundle_digest,
                "revision_ids": revision_ids,
                "file_count": bundle.files.len(),
                "authz": audit::decision_context(Action::KnowledgeRead, &allowed),
            }),
        )
        .await?;
        let view = OkfExportView {
            format_version: bundle.format_version,
            specification_commit: bundle.specification_commit,
            files: bundle
                .files
                .into_iter()
                .map(|file| OkfExportFileView {
                    logical_path: file.logical_path,
                    content: file.content,
                    content_hash: file.content_hash,
                })
                .collect(),
            bundle_digest: bundle.bundle_digest,
        };
        commit(tx).await?;
        Ok(Json(view))
    }
    .await;
    respond(&state, "okf.export", result).await
}
