//! Bounded authoritative freshness signals for exact Knowledge revisions
//! (CPR-37, ADR-0096).
//!
//! This is candidate evidence below authorisation. Callers must decide the
//! exact revision first; these booleans never disclose a source, repository,
//! feedback row or count.

use sqlx::PgConnection;
use synveda_types::knowledge::FreshnessEvidence;
use synveda_types::{Error, KnowledgeRevisionId, ProjectId, Result, TenantId};

fn storage_error(error: sqlx::Error) -> Error {
    if let sqlx::Error::Database(database) = &error
        && database.code().as_deref() == Some("42501")
    {
        return crate::rls::backstop_error(database);
    }
    Error::Storage {
        message: error.to_string(),
    }
}

/// Resolve non-content signals for one already-authorised exact revision.
///
/// A repository change is meaningful only when the repository adapter has
/// supplied `metadata.content_revision`; attachment timestamp churn alone is
/// ignored. Reference source freshness is an immutable descriptor state, not
/// a live network fetch. Failed use is user feedback attached to this exact
/// revision.
pub async fn evidence(
    connection: &mut PgConnection,
    tenant: TenantId,
    revision_id: KnowledgeRevisionId,
    project_id: Option<ProjectId>,
) -> Result<FreshnessEvidence> {
    let row = sqlx::query!(
        r#"select
            exists (
                select 1
                  from project_repositories repository
                 where repository.tenant_id = $1
                   and repository.project_id = $3
                   and repository.metadata ? 'content_revision'
                   and nullif(repository.metadata->>'content_revision', '') is not null
                   and not exists (
                       select 1
                         from knowledge_revision_sources link
                         join knowledge_sources source
                           on source.tenant_id = link.tenant_id
                          and source.id = link.knowledge_source_id
                        where link.tenant_id = $1
                          and link.knowledge_revision_id = $2
                          and source.source_type = 'repository'
                          and source.source_revision =
                              repository.metadata->>'content_revision'
                   )
            ) as "repository_changed!",
            exists (
                select 1
                  from context_feedback feedback
                 where feedback.tenant_id = $1
                   and feedback.knowledge_revision_id = $2
                   and feedback.feedback_type in ('unhelpful', 'caused_correction')
            ) as "failed_use!",
            exists (
                select 1
                  from knowledge_revision_sources link
                  join knowledge_sources source
                    on source.tenant_id = link.tenant_id
                   and source.id = link.knowledge_source_id
                 where link.tenant_id = $1
                   and link.knowledge_revision_id = $2
                   and source.metadata->>'synveda_freshness_state' = 'stale'
            ) as "source_stale!",
            exists (
                select 1 from knowledge_revision_source_headers link
                join knowledge_sources source on source.tenant_id=link.tenant_id and source.id=link.knowledge_source_id
                join source_artifact_addresses address on address.tenant_id=source.tenant_id and address.artifact_id=source.import_artifact_id
                join import_jobs job on job.tenant_id=address.tenant_id and job.id=address.job_id
                join source_reconcile_heads head on head.tenant_id=job.tenant_id and head.project_id=job.project_id and head.source_id=job.managed_source_id
                join source_observation_paths current on current.tenant_id=head.tenant_id and current.observation_id=head.observation_id and current.path_digest=address.path_digest
                where link.tenant_id=$1 and link.knowledge_revision_id=$2
                 and current.state in ('present','missing') and current.content_hash is distinct from source.content_hash
            ) as "managed_source_changed!""#,
        tenant.as_uuid(),
        revision_id.as_uuid(),
        project_id.map(|id| id.as_uuid()),
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(storage_error)?;
    Ok(FreshnessEvidence {
        repository_changed: row.repository_changed,
        failed_use: row.failed_use,
        source_stale: row.source_stale,
        managed_source_changed: row.managed_source_changed,
    })
}

/// Exact managed-source observation below independent SourceRead authorization.
pub struct SourceObservationSignal {
    /// Project whose source workflow owns this observation.
    pub project_id: uuid::Uuid,
    /// Exact configured source, never a inferred inventory.
    pub source_id: String,
    /// Last complete observation clock, separate from refresh request time.
    pub observed_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Exact descriptor that was completely observed.
    pub descriptor_hash: Option<String>,
    /// Exact current configured-path state.
    pub path_state: Option<String>,
    /// Whether the current bytes differ from this approved source descriptor.
    pub changed: bool,
    /// Most recent independently inspectable refresh receipt.
    pub operation_id: Option<uuid::Uuid>,
    /// Refresh lifecycle; it does not establish complete synchronization.
    pub operation_state: Option<String>,
}
/// Bounded exact-revision source joins; no tenant-wide source-health scan.
pub async fn source_observations(
    connection: &mut PgConnection,
    tenant: TenantId,
    revision: KnowledgeRevisionId,
) -> Result<Vec<SourceObservationSignal>> {
    sqlx::query_as!(SourceObservationSignal,r#"select distinct job.project_id,job.managed_source_id as "source_id!",observation.observed_at as "observed_at?",observation.descriptor_hash as "descriptor_hash?",
    path.state as "path_state?",coalesce(path.state in ('present','missing') and path.content_hash is distinct from source.content_hash,false) as "changed!",
    head.last_operation_id as operation_id,operation.state as "operation_state?"
   from knowledge_revision_source_headers link
   join knowledge_sources source on source.tenant_id=link.tenant_id and source.id=link.knowledge_source_id
   join source_artifact_addresses address on address.tenant_id=source.tenant_id and address.artifact_id=source.import_artifact_id
   join import_jobs job on job.tenant_id=address.tenant_id and job.id=address.job_id
   join source_reconcile_heads head on head.tenant_id=job.tenant_id and head.project_id=job.project_id and head.source_id=job.managed_source_id
   left join source_observations observation on observation.tenant_id=head.tenant_id and observation.id=head.observation_id
   left join source_observation_paths path on path.tenant_id=head.tenant_id and path.observation_id=head.observation_id and path.path_digest=address.path_digest
   left join durable_operations operation on operation.tenant_id=head.tenant_id and operation.id=head.last_operation_id
   where link.tenant_id=$1 and link.knowledge_revision_id=$2 and job.managed_source_id is not null
   order by job.project_id,job.managed_source_id limit 33"#,tenant.as_uuid(),revision.as_uuid()).fetch_all(connection).await.map_err(storage_error)
}
