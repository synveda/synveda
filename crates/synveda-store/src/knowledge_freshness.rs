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
            ) as "source_stale!""#,
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
    })
}
