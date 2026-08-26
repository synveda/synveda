//! Durable, tenant-isolated external-format plans (CPR-27, ADR-0087).
//!
//! A plan freezes admitted artifacts and proposed mappings. Materialising it
//! creates an ordinary capture batch and reviewable candidates; it never
//! creates Knowledge and therefore cannot bypass the Knowledge/VedaFlow seam.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgConnection, PgExecutor};
use synveda_types::capture::{CaptureBatch, CaptureCandidate, CaptureMatch, CaptureMatchKind};
use synveda_types::configuration::EffectiveConfiguration;
use synveda_types::import::{
    ImportArtifact, ImportArtifactKind, ImportJob, ImportJobState, ImportMapping,
    ImportMappingClassification,
};
use synveda_types::knowledge::{
    KnowledgeOrigin, KnowledgeRevisionContent, KnowledgeType, normalise_knowledge_tags,
    validate_content_hash, validate_knowledge_revision_content,
};
use synveda_types::{
    CaptureBatchId, CaptureCandidateId, Error, ImportArtifactId, ImportJobId, ImportMappingId,
    KnowledgeItemId, KnowledgeRevisionId, ProjectId, Result, ScopeId, TenantId, WorkspaceId,
};
use uuid::Uuid;

/// Maximum jobs scanned before the application applies its per-row PDP.
pub const IMPORT_SCAN_LIMIT: i64 = 500;
/// Low-level import-plan changes.
pub const IMPORT_MUTATIONS_TOTAL: &str = "synveda_import_mutations_total";

/// One admitted artifact ready for immutable persistence.
#[derive(Debug, Clone)]
pub struct NewImportArtifact {
    /// Stable artifact id.
    pub id: ImportArtifactId,
    /// Stable path order.
    pub ordinal: i32,
    /// Safe bundle-relative path.
    pub logical_path: String,
    /// Concept, index or log.
    pub kind: ImportArtifactKind,
    /// Exact admitted-byte digest.
    pub content_hash: String,
    /// Parsed frontmatter.
    pub frontmatter: Value,
    /// Markdown body after frontmatter.
    pub body_markdown: String,
}

/// One proposed concept mapping ready for immutable persistence.
#[derive(Debug, Clone)]
pub struct NewImportMapping {
    /// Stable mapping id.
    pub id: ImportMappingId,
    /// Source artifact id from this plan.
    pub artifact_id: ImportArtifactId,
    /// Stable concept order.
    pub ordinal: i32,
    /// Producer-defined OKF type.
    pub okf_type: String,
    /// Proposed Synveda type.
    pub knowledge_type: KnowledgeType,
    /// Complete proposed revision.
    pub content: KnowledgeRevisionContent,
    /// Canonical semantic digest.
    pub content_hash: String,
    /// Addition, update, duplicate or conflict.
    pub classification: ImportMappingClassification,
    /// Visible current item compared, if any.
    pub matched_item_id: Option<KnowledgeItemId>,
    /// Exact visible current revision compared, if any.
    pub matched_revision_id: Option<KnowledgeRevisionId>,
    /// Proposed internal links.
    pub proposed_relations: Value,
    /// Whether external lifecycle permits a candidate.
    pub materializable: bool,
}

/// One complete, pre-authorised immutable import plan.
#[derive(Debug, Clone)]
pub struct NewImportPlan {
    /// Stable job id.
    pub id: ImportJobId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Project being imported into.
    pub project_id: ProjectId,
    /// Project scope.
    pub scope_id: ScopeId,
    /// Parent workspace.
    pub workspace_id: WorkspaceId,
    /// Authenticated actor.
    pub principal_id: String,
    /// Exact versioned adapter identifier.
    pub format: String,
    /// Exact external format version.
    pub format_version: String,
    /// Exact official specification revision.
    pub specification_commit: String,
    /// Directory, zip, tar or Git.
    pub source_kind: String,
    /// Credential-free source label.
    pub source_locator: String,
    /// Explicit upstream revision when present.
    pub source_revision: Option<String>,
    /// Canonical admitted-bundle digest.
    pub bundle_digest: String,
    /// Content-free validation notices.
    pub notices: Vec<String>,
    /// Immutable artifacts.
    pub artifacts: Vec<NewImportArtifact>,
    /// Immutable concept mappings.
    pub mappings: Vec<NewImportMapping>,
}

/// New plan or unchanged-content replay.
#[derive(Debug, Clone)]
pub struct PlannedImport {
    /// Durable job.
    pub job: ImportJob,
    /// Whether this transaction inserted the plan.
    pub created: bool,
}

/// Candidate-only materialisation result.
#[derive(Debug, Clone)]
pub struct MaterializedImport {
    /// Terminal job.
    pub job: ImportJob,
    /// Completed import-sourced capture batch.
    pub batch: CaptureBatch,
    /// Reviewable candidates; never active Knowledge.
    pub candidates: Vec<CaptureCandidate>,
    /// Whether this call performed the transition.
    pub created: bool,
}

/// Keyset for job listings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportCursor {
    /// Creation instant.
    pub created_at: DateTime<Utc>,
    /// Stable tie-breaker.
    pub id: ImportJobId,
}

struct JobRow {
    id: Uuid,
    tenant_id: Uuid,
    project_id: Uuid,
    scope_id: Uuid,
    workspace_id: Uuid,
    principal_id: String,
    format: String,
    format_version: String,
    specification_commit: String,
    source_kind: String,
    source_locator: String,
    source_revision: Option<String>,
    bundle_digest: String,
    state: String,
    artifact_count: i32,
    mapping_count: i32,
    candidate_count: i32,
    capture_batch_id: Option<Uuid>,
    error_code: Option<String>,
    notices: Value,
    created_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<JobRow> for ImportJob {
    type Error = Error;

    fn try_from(row: JobRow) -> Result<Self> {
        Ok(Self {
            id: ImportJobId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            project_id: ProjectId::from_uuid(row.project_id),
            scope_id: ScopeId::from_uuid(row.scope_id),
            workspace_id: WorkspaceId::from_uuid(row.workspace_id),
            principal_id: row.principal_id,
            format: row.format,
            format_version: row.format_version,
            specification_commit: row.specification_commit,
            source_kind: row.source_kind,
            source_locator: row.source_locator,
            source_revision: row.source_revision,
            bundle_digest: row.bundle_digest,
            state: stored(&row.state)?,
            artifact_count: row.artifact_count,
            mapping_count: row.mapping_count,
            candidate_count: row.candidate_count,
            capture_batch_id: row.capture_batch_id.map(CaptureBatchId::from_uuid),
            error_code: row.error_code,
            notices: serde_json::from_value(row.notices).map_err(|error| Error::Internal {
                message: format!("stored import notices are invalid: {error}"),
            })?,
            created_at: row.created_at,
            completed_at: row.completed_at,
            updated_at: row.updated_at,
        })
    }
}

struct ArtifactRow {
    id: Uuid,
    job_id: Uuid,
    ordinal: i32,
    logical_path: String,
    artifact_kind: String,
    content_hash: String,
    frontmatter: Value,
    body_markdown: String,
    created_at: DateTime<Utc>,
}

impl TryFrom<ArtifactRow> for ImportArtifact {
    type Error = Error;

    fn try_from(row: ArtifactRow) -> Result<Self> {
        Ok(Self {
            id: ImportArtifactId::from_uuid(row.id),
            job_id: ImportJobId::from_uuid(row.job_id),
            ordinal: row.ordinal,
            logical_path: row.logical_path,
            kind: stored(&row.artifact_kind)?,
            content_hash: row.content_hash,
            frontmatter: row.frontmatter,
            body_markdown: row.body_markdown,
            created_at: row.created_at,
        })
    }
}

struct MappingRow {
    id: Uuid,
    job_id: Uuid,
    artifact_id: Uuid,
    ordinal: i32,
    okf_type: String,
    knowledge_type: String,
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
    classification: String,
    matched_item_id: Option<Uuid>,
    matched_revision_id: Option<Uuid>,
    proposed_relations: Value,
    materializable: bool,
    content_erased: bool,
    candidate_id: Option<Uuid>,
    created_at: DateTime<Utc>,
}

impl TryFrom<MappingRow> for ImportMapping {
    type Error = Error;

    fn try_from(row: MappingRow) -> Result<Self> {
        Ok(Self {
            id: ImportMappingId::from_uuid(row.id),
            job_id: ImportJobId::from_uuid(row.job_id),
            artifact_id: ImportArtifactId::from_uuid(row.artifact_id),
            ordinal: row.ordinal,
            okf_type: row.okf_type,
            knowledge_type: stored(&row.knowledge_type)?,
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
            classification: stored(&row.classification)?,
            matched_item_id: row.matched_item_id.map(KnowledgeItemId::from_uuid),
            matched_revision_id: row.matched_revision_id.map(KnowledgeRevisionId::from_uuid),
            proposed_relations: row.proposed_relations,
            materializable: row.materializable,
            content_erased: row.content_erased,
            candidate_id: row.candidate_id.map(CaptureCandidateId::from_uuid),
            created_at: row.created_at,
        })
    }
}

fn stored<T: std::str::FromStr<Err = Error>>(value: &str) -> Result<T> {
    value.parse().map_err(|_| Error::Internal {
        message: format!("stored value outside vocabulary: {value:?}"),
    })
}

fn storage_error(error: sqlx::Error) -> Error {
    if let sqlx::Error::Database(database) = &error {
        match database.code().as_deref() {
            Some("23505" | "40001") => {
                return Error::Conflict {
                    message: database.to_string(),
                };
            }
            Some("23503" | "23514" | "22001" | "P0001") => {
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

fn validate_plan(plan: &NewImportPlan) -> Result<()> {
    if plan.artifacts.is_empty() || plan.artifacts.len() > 2_000 {
        return Err(Error::Invalid {
            message: "an import plan requires between 1 and 2000 artifacts".to_owned(),
        });
    }
    if plan.mappings.is_empty() || plan.mappings.len() > plan.artifacts.len() {
        return Err(Error::Invalid {
            message: "an import plan requires at least one concept mapping".to_owned(),
        });
    }
    validate_content_hash(&plan.bundle_digest)?;
    for mapping in &plan.mappings {
        validate_knowledge_revision_content(&mapping.content)?;
        validate_content_hash(&mapping.content_hash)?;
        if mapping.content_hash
            != synveda_types::knowledge::knowledge_revision_content_hash(&mapping.content)
        {
            return Err(Error::Invalid {
                message: "import mapping content hash does not match semantic content".to_owned(),
            });
        }
        if normalise_knowledge_tags(&mapping.content.tags)? != mapping.content.tags {
            return Err(Error::Invalid {
                message: "import mapping tags must be lower-case, sorted and unique".to_owned(),
            });
        }
        let has_match = mapping.matched_item_id.is_some() && mapping.matched_revision_id.is_some();
        if (mapping.classification == ImportMappingClassification::Addition) == has_match
            || mapping.matched_item_id.is_some() != mapping.matched_revision_id.is_some()
        {
            return Err(Error::Invalid {
                message: "import mapping classification and visible match disagree".to_owned(),
            });
        }
    }
    Ok(())
}

/// Creates an immutable dry-run plan or returns the unchanged-content plan.
#[tracing::instrument(name = "store.imports.plan", skip_all, fields(tenant.id = %plan.tenant_id, import.job.id = %plan.id), err(Display))]
pub async fn create_plan(conn: &mut PgConnection, plan: &NewImportPlan) -> Result<PlannedImport> {
    validate_plan(plan)?;
    let inserted = sqlx::query_scalar!(
        r#"
        insert into import_jobs
            (id, tenant_id, project_id, scope_id, workspace_id, principal_id,
             format, format_version, specification_commit, source_kind,
             source_locator, source_revision, bundle_digest, artifact_count,
             mapping_count, notices)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                $13, $14, $15, $16)
        on conflict on constraint import_jobs_source_digest_unique do nothing
        returning id
        "#,
        plan.id.as_uuid(),
        plan.tenant_id.as_uuid(),
        plan.project_id.as_uuid(),
        plan.scope_id.as_uuid(),
        plan.workspace_id.as_uuid(),
        plan.principal_id,
        plan.format,
        plan.format_version,
        plan.specification_commit,
        plan.source_kind,
        plan.source_locator,
        plan.source_revision.as_deref() as Option<&str>,
        plan.bundle_digest,
        plan.artifacts.len() as i32,
        plan.mappings.len() as i32,
        serde_json::json!(plan.notices),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;

    if inserted.is_some() {
        for artifact in &plan.artifacts {
            sqlx::query!(
                r#"
                insert into import_artifacts
                    (id, tenant_id, job_id, ordinal, logical_path, artifact_kind,
                     content_hash, frontmatter, body_markdown)
                values ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                "#,
                artifact.id.as_uuid(),
                plan.tenant_id.as_uuid(),
                plan.id.as_uuid(),
                artifact.ordinal,
                artifact.logical_path,
                artifact.kind.as_str(),
                artifact.content_hash,
                artifact.frontmatter,
                artifact.body_markdown,
            )
            .execute(&mut *conn)
            .await
            .map_err(storage_error)?;
        }
        for mapping in &plan.mappings {
            let content = &mapping.content;
            sqlx::query!(
                r#"
                insert into import_mappings
                    (id, tenant_id, job_id, artifact_id, ordinal, okf_type,
                     knowledge_type, title, body_markdown, summary, tags,
                     sensitivity, confidence_permille, valid_from, valid_to,
                     stale_after, verification_metadata, metadata, content_hash,
                     classification, matched_item_id, matched_revision_id,
                     proposed_relations, materializable)
                values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                        $13, $14, $15, $16, $17, $18, $19, $20, $21, $22,
                        $23, $24)
                "#,
                mapping.id.as_uuid(),
                plan.tenant_id.as_uuid(),
                plan.id.as_uuid(),
                mapping.artifact_id.as_uuid(),
                mapping.ordinal,
                mapping.okf_type,
                mapping.knowledge_type.as_str(),
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
                mapping.content_hash,
                mapping.classification.as_str(),
                mapping.matched_item_id.map(|id| id.as_uuid()) as Option<Uuid>,
                mapping.matched_revision_id.map(|id| id.as_uuid()) as Option<Uuid>,
                mapping.proposed_relations,
                mapping.materializable,
            )
            .execute(&mut *conn)
            .await
            .map_err(storage_error)?;
        }
        metrics::counter!(IMPORT_MUTATIONS_TOTAL, "operation" => "plan_created").increment(1);
    }
    let job = if inserted.is_some() {
        get_job(&mut *conn, plan.tenant_id, plan.id).await?
    } else {
        get_job_by_digest(
            &mut *conn,
            plan.tenant_id,
            plan.project_id,
            &plan.source_kind,
            &plan.source_locator,
            &plan.bundle_digest,
        )
        .await?
    }
    .ok_or_else(|| Error::Internal {
        message: "import plan insert produced no readable job".to_owned(),
    })?;
    Ok(PlannedImport {
        job,
        created: inserted.is_some(),
    })
}

/// Reads one import job.
pub async fn get_job(
    executor: impl PgExecutor<'_>,
    tenant: TenantId,
    id: ImportJobId,
) -> Result<Option<ImportJob>> {
    let row = sqlx::query_as!(
        JobRow,
        r#"
        select id, tenant_id, project_id, scope_id, workspace_id, principal_id,
               format, format_version, specification_commit, source_kind,
               source_locator, source_revision, bundle_digest, state,
               artifact_count, mapping_count, candidate_count, capture_batch_id,
               error_code, notices, created_at, completed_at, updated_at
        from import_jobs where tenant_id = $1 and id = $2
        "#,
        tenant.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

async fn get_job_by_digest(
    executor: impl PgExecutor<'_>,
    tenant: TenantId,
    project: ProjectId,
    source_kind: &str,
    locator: &str,
    digest: &str,
) -> Result<Option<ImportJob>> {
    let row = sqlx::query_as!(
        JobRow,
        r#"
        select id, tenant_id, project_id, scope_id, workspace_id, principal_id,
               format, format_version, specification_commit, source_kind,
               source_locator, source_revision, bundle_digest, state,
               artifact_count, mapping_count, candidate_count, capture_batch_id,
               error_code, notices, created_at, completed_at, updated_at
        from import_jobs
        where tenant_id = $1 and project_id = $2 and source_kind = $3
          and source_locator = $4 and bundle_digest = $5
        "#,
        tenant.as_uuid(),
        project.as_uuid(),
        source_kind,
        locator,
        digest,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Lists bounded import jobs newest first for per-row PDP decisions.
pub async fn list_jobs(
    executor: impl PgExecutor<'_>,
    tenant: TenantId,
    project: Option<ProjectId>,
    state: Option<ImportJobState>,
    after: Option<ImportCursor>,
) -> Result<Vec<ImportJob>> {
    let rows = sqlx::query_as!(
        JobRow,
        r#"
        select id, tenant_id, project_id, scope_id, workspace_id, principal_id,
               format, format_version, specification_commit, source_kind,
               source_locator, source_revision, bundle_digest, state,
               artifact_count, mapping_count, candidate_count, capture_batch_id,
               error_code, notices, created_at, completed_at, updated_at
        from import_jobs
        where tenant_id = $1
          and ($2::uuid is null or project_id = $2)
          and ($3::text is null or state = $3)
          and ($4::timestamptz is null or created_at < $4
               or (created_at = $4 and id < $5))
        order by created_at desc, id desc
        limit $6
        "#,
        tenant.as_uuid(),
        project.map(|id| id.as_uuid()) as Option<Uuid>,
        state.map(ImportJobState::as_str) as Option<&str>,
        after.map(|cursor| cursor.created_at) as Option<DateTime<Utc>>,
        after.map(|cursor| cursor.id.as_uuid()) as Option<Uuid>,
        IMPORT_SCAN_LIMIT,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Reads immutable admitted artifacts in canonical order.
pub async fn artifacts(
    executor: impl PgExecutor<'_>,
    tenant: TenantId,
    job: ImportJobId,
) -> Result<Vec<ImportArtifact>> {
    let rows = sqlx::query_as!(
        ArtifactRow,
        r#"
        select id, job_id, ordinal, logical_path, artifact_kind, content_hash,
               frontmatter, body_markdown, created_at
        from import_artifacts
        where tenant_id = $1 and job_id = $2
        order by ordinal
        "#,
        tenant.as_uuid(),
        job.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Reads immutable proposed mappings in canonical order.
pub async fn mappings(
    executor: impl PgExecutor<'_>,
    tenant: TenantId,
    job: ImportJobId,
) -> Result<Vec<ImportMapping>> {
    let rows = sqlx::query_as!(
        MappingRow,
        r#"
        select id, job_id, artifact_id, ordinal, okf_type, knowledge_type,
               title, body_markdown, summary, tags as "tags!: Vec<String>",
               sensitivity, confidence_permille, valid_from, valid_to,
               stale_after, verification_metadata, metadata, content_hash,
               classification, matched_item_id, matched_revision_id,
               proposed_relations, materializable, content_erased, candidate_id,
               created_at
        from import_mappings
        where tenant_id = $1 and job_id = $2
        order by ordinal
        "#,
        tenant.as_uuid(),
        job.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

fn candidate_match(mapping: &ImportMapping) -> Option<CaptureMatch> {
    let (item, revision) = (mapping.matched_item_id?, mapping.matched_revision_id?);
    let (kind, similarity, reason) = match mapping.classification {
        ImportMappingClassification::Duplicate => {
            (CaptureMatchKind::Duplicate, 1_000, "okf_content_hash")
        }
        ImportMappingClassification::Update => (
            CaptureMatchKind::Supersession,
            950,
            "okf_source_path_changed",
        ),
        ImportMappingClassification::Conflict => {
            (CaptureMatchKind::Contradiction, 700, "okf_title_conflict")
        }
        ImportMappingClassification::Addition => return None,
    };
    Some(CaptureMatch {
        knowledge_item_id: item,
        knowledge_revision_id: revision,
        kind,
        similarity_permille: similarity,
        reason_code: reason.to_owned(),
    })
}

/// Materialises one planned import into candidates, or replays its terminal result.
#[tracing::instrument(name = "store.imports.materialize", skip_all, fields(tenant.id = %tenant, import.job.id = %id), err(Display))]
pub async fn materialize(
    conn: &mut PgConnection,
    tenant: TenantId,
    id: ImportJobId,
    configuration: &EffectiveConfiguration,
) -> Result<MaterializedImport> {
    let job = get_job(&mut *conn, tenant, id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("import job {id}"),
        })?;
    if job.state == ImportJobState::Failed {
        return Err(Error::Conflict {
            message: format!("import job {id} is failed"),
        });
    }
    if configuration.scope_id != job.scope_id
        || configuration.document.content_hash()? != configuration.content_hash
    {
        return Err(Error::Invalid {
            message: "OKF materialisation configuration evidence is invalid".to_owned(),
        });
    }
    if job.state == ImportJobState::Materialized {
        let batch_id = job.capture_batch_id.ok_or_else(|| Error::Internal {
            message: "materialized import job has no capture batch".to_owned(),
        })?;
        let batch = crate::capture::get_batch(&mut *conn, tenant, batch_id)
            .await?
            .ok_or_else(|| Error::Internal {
                message: "materialized import job points at no capture batch".to_owned(),
            })?;
        let candidates = crate::capture::list_candidates(
            conn,
            tenant,
            &crate::capture::CandidateFilter {
                batch_id: Some(batch_id),
                ..Default::default()
            },
        )
        .await?;
        return Ok(MaterializedImport {
            job,
            batch,
            candidates,
            created: false,
        });
    }

    let mappings = mappings(&mut *conn, tenant, id).await?;
    let admitted: Vec<&ImportMapping> = mappings
        .iter()
        .filter(|mapping| {
            mapping.materializable
                && mapping.classification != ImportMappingClassification::Duplicate
        })
        .collect();
    let batch_id = CaptureBatchId::new();
    sqlx::query!(
        r#"
        insert into capture_batches
            (id, tenant_id, source_kind, session_id, import_job_id, scope_id,
             workspace_id, project_id, principal_id,
             configuration_version_id, configuration_hash, input_hash, event_count,
             state, extractor_method, model_version, attempts, candidate_count,
             created_at, started_at, completed_at, updated_at)
        values ($1, $2, 'okf_import', null, $3, $4, $5, $6, $7, $8, $9,
                $10, 0, 'completed', 'okf-v0.2', $11, 1, $12,
                transaction_timestamp(), transaction_timestamp(),
                transaction_timestamp(), transaction_timestamp())
        "#,
        batch_id.as_uuid(),
        tenant.as_uuid(),
        id.as_uuid(),
        job.scope_id.as_uuid(),
        job.workspace_id.as_uuid(),
        job.project_id.as_uuid(),
        job.principal_id,
        configuration.version_id.map(|value| value.as_uuid()) as Option<Uuid>,
        configuration.content_hash,
        job.bundle_digest,
        job.specification_commit,
        admitted.len() as i32,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;

    for (position, mapping) in admitted.iter().enumerate() {
        let candidate_id = CaptureCandidateId::new();
        let content = &mapping.content;
        sqlx::query!(
            r#"
            insert into capture_candidates
                (id, tenant_id, batch_id, source_kind, session_id, import_job_id,
                 ordinal, proposed_scope_id, proposed_project_id,
                 proposed_owner_principal_id, knowledge_type, origin, title,
                 body_markdown, summary, tags, sensitivity, confidence_permille,
                 valid_from, valid_to, stale_after, verification_metadata,
                 metadata, content_hash)
            values ($1, $2, $3, 'okf_import', null, $4, $5, $6, $7, null,
                    $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18,
                    $19, $20, $21)
            "#,
            candidate_id.as_uuid(),
            tenant.as_uuid(),
            batch_id.as_uuid(),
            id.as_uuid(),
            (position + 1) as i32,
            job.scope_id.as_uuid(),
            job.project_id.as_uuid(),
            mapping.knowledge_type.as_str(),
            KnowledgeOrigin::Imported.as_str(),
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
            mapping.content_hash,
        )
        .execute(&mut *conn)
        .await
        .map_err(storage_error)?;
        sqlx::query!(
            r#"
            insert into capture_candidate_import_artifacts
                (tenant_id, candidate_id, import_job_id, artifact_id, ordinal)
            values ($1, $2, $3, $4, 1)
            "#,
            tenant.as_uuid(),
            candidate_id.as_uuid(),
            id.as_uuid(),
            mapping.artifact_id.as_uuid(),
        )
        .execute(&mut *conn)
        .await
        .map_err(storage_error)?;
        if let Some(matched) = candidate_match(mapping) {
            sqlx::query!(
                r#"
                insert into capture_candidate_matches
                    (tenant_id, candidate_id, knowledge_item_id,
                     knowledge_revision_id, match_kind, similarity_permille,
                     reason_code)
                values ($1, $2, $3, $4, $5, $6, $7)
                "#,
                tenant.as_uuid(),
                candidate_id.as_uuid(),
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
        sqlx::query!(
            r#"
            update import_mappings set candidate_id = $3
            where tenant_id = $1 and id = $2 and candidate_id is null
            "#,
            tenant.as_uuid(),
            mapping.id.as_uuid(),
            candidate_id.as_uuid(),
        )
        .execute(&mut *conn)
        .await
        .map_err(storage_error)?;
    }
    let now = Utc::now();
    sqlx::query!(
        r#"
        update import_jobs
           set state = 'materialized', candidate_count = $3,
               capture_batch_id = $4, completed_at = $5, updated_at = $5
         where tenant_id = $1 and id = $2 and state = 'planned'
        "#,
        tenant.as_uuid(),
        id.as_uuid(),
        admitted.len() as i32,
        batch_id.as_uuid(),
        now,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;
    metrics::counter!(IMPORT_MUTATIONS_TOTAL, "operation" => "materialized").increment(1);
    let job = get_job(&mut *conn, tenant, id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: "materialized import job disappeared".to_owned(),
        })?;
    let batch = crate::capture::get_batch(&mut *conn, tenant, batch_id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: "materialized import capture batch disappeared".to_owned(),
        })?;
    let candidates = crate::capture::list_candidates(
        conn,
        tenant,
        &crate::capture::CandidateFilter {
            batch_id: Some(batch_id),
            ..Default::default()
        },
    )
    .await?;
    Ok(MaterializedImport {
        job,
        batch,
        candidates,
        created: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn additions_do_not_create_match_hints() {
        let mapping = ImportMapping {
            id: ImportMappingId::new(),
            job_id: ImportJobId::new(),
            artifact_id: ImportArtifactId::new(),
            ordinal: 1,
            okf_type: "concept".to_owned(),
            knowledge_type: KnowledgeType::Fact,
            content: KnowledgeRevisionContent {
                title: "T".to_owned(),
                body_markdown: "B".to_owned(),
                summary: "S".to_owned(),
                tags: Vec::new(),
                sensitivity: synveda_types::Sensitivity::Internal,
                confidence_permille: 900,
                valid_from: Utc::now(),
                valid_to: None,
                stale_after: None,
                verification_metadata: serde_json::json!({}),
                metadata: serde_json::json!({}),
            },
            content_hash: "0".repeat(64),
            classification: ImportMappingClassification::Addition,
            matched_item_id: None,
            matched_revision_id: None,
            proposed_relations: serde_json::json!([]),
            materializable: true,
            content_erased: false,
            candidate_id: None,
            created_at: Utc::now(),
        };
        assert!(candidate_match(&mapping).is_none());
    }
}
