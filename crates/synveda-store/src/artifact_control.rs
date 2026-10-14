//! Exact imported source roots, permanent aliases and checked cleanup.

use sqlx::PgConnection;
use synveda_types::knowledge::{KnowledgeSourceDraft, SourceControlMode};
use synveda_types::{DurableOperationId, Error, ImportArtifactId, ProposalId, Result, TenantId};
use uuid::Uuid;

/// Immutable content-free address retained after source cleanup.
pub struct ArtifactAddress {
    /// Inspected artifact, rather than a guessed metadata identifier.
    pub artifact_id: Uuid,
    /// Original import plan.
    pub job_id: Uuid,
    /// Actual project and scope anchors.
    pub project_id: Uuid,
    /// Actual governed scope.
    pub scope_id: Uuid,
    /// Actual parent workspace.
    pub workspace_id: Uuid,
    /// Original exact admitted-byte hash.
    pub content_hash: String,
    /// Stable source/path/hash identity shared by duplicate jobs.
    pub source_key_digest: String,
}

/// Resolve a retained address; the caller independently decides its project.
pub async fn address(
    conn: &mut PgConnection,
    tenant: TenantId,
    artifact: ImportArtifactId,
) -> Result<Option<ArtifactAddress>> {
    sqlx::query_as!(
        ArtifactAddress,
        r#"select artifact_id,job_id,project_id,scope_id,workspace_id,content_hash,source_key_digest
        from source_artifact_addresses where tenant_id=$1 and artifact_id=$2"#,
        tenant.as_uuid(),
        artifact.as_uuid()
    )
    .fetch_optional(conn)
    .await
    .map_err(storage)
}

/// Resolve typed provenance, or the complete retained server tuple of an older
/// pending command. Metadata alone never grants a source address.
pub async fn source_artifact(
    conn: &mut PgConnection,
    tenant: TenantId,
    source: &KnowledgeSourceDraft,
) -> Result<Option<ImportArtifactId>> {
    let value = serde_json::to_value(source).map_err(|_| Error::Invalid {
        message: "invalid artifact provenance".to_owned(),
    })?;
    sqlx::query_scalar!(
        r#"select synveda_import_source_artifact($1,$2) as "artifact_id?""#,
        tenant.as_uuid(),
        value
    )
    .fetch_one(conn)
    .await
    .map(|id| id.map(ImportArtifactId::from_uuid))
    .map_err(storage)
}

/// Check every admitted alias before a duplicate job can be returned.
pub async fn admit_plan(
    conn: &mut PgConnection,
    plan: &crate::imports::NewImportPlan,
) -> Result<()> {
    for artifact in &plan.artifacts {
        let suppressed = sqlx::query_scalar!(
            r#"select exists(select 1 from artifact_suppressions
          where tenant_id=$1 and source_key_digest=synveda_artifact_source_key($2,
            synveda_repository_source_identity($1,$3,$4),$5,$6,
             encode(sha256(convert_to($7,'UTF8')),'hex'),$8)) as "suppressed!""#,
            plan.tenant_id.as_uuid(),
            plan.scope_id.as_uuid(),
            plan.project_id.as_uuid(),
            plan.repository_id.map(|id| id.as_uuid()) as Option<Uuid>,
            plan.source_kind,
            plan.source_locator,
            artifact.logical_path,
            artifact.content_hash
        )
        .fetch_one(&mut *conn)
        .await
        .map_err(storage)?;
        if suppressed {
            return Err(crate::source_control::unavailable());
        }
    }
    Ok(())
}

/// A filtered immutable plan cannot be replayed or materialized as a subset.
pub async fn admit_job(
    conn: &mut PgConnection,
    tenant: TenantId,
    job: &synveda_types::import::ImportJob,
) -> Result<()> {
    let intact=sqlx::query_scalar!(r#"select
       (select count(*) from import_artifacts where tenant_id=$1 and job_id=$2 and not content_erased)=$3
       and (select count(*) from import_mappings where tenant_id=$1 and job_id=$2)=$4 as "intact!""#,
       tenant.as_uuid(),job.id.as_uuid(),i64::from(job.artifact_count),i64::from(job.mapping_count))
       .fetch_one(conn).await.map_err(storage)?;
    if !intact {
        return Err(crate::source_control::unavailable());
    }
    Ok(())
}

/// Apply with the exclusive tenant gate and reviewed Knowledge effect.
#[allow(clippy::too_many_arguments)]
pub async fn apply(
    conn: &mut PgConnection,
    tenant: TenantId,
    source: &ArtifactAddress,
    change: ProposalId,
    input_hash: &str,
    mode: SourceControlMode,
    actor_hash: &str,
    reason_hash: &str,
) -> Result<DurableOperationId> {
    if let Some(previous) = status(
        conn,
        tenant,
        ImportArtifactId::from_uuid(source.artifact_id),
    )
    .await?
        && (mode == SourceControlMode::Withdraw || previous.forgetting)
    {
        return Err(crate::source_control::unavailable());
    }
    let forgetting = mode == SourceControlMode::Forget;
    sqlx::query!(r#"insert into artifact_suppressions (tenant_id,source_key_digest,artifact_id,change_id,forget_change_id,actor_hash,reason_hash)
      values ($1,$2,$3,$4,$5,$6,$7) on conflict (tenant_id,source_key_digest) do update
      set forget_change_id=coalesce(artifact_suppressions.forget_change_id,excluded.forget_change_id)"#,
      tenant.as_uuid(),source.source_key_digest,source.artifact_id,change.as_uuid(),forgetting.then(||change.as_uuid()),actor_hash,reason_hash)
      .execute(&mut *conn).await.map_err(storage)?;
    let operation = DurableOperationId::new();
    sqlx::query!(r#"insert into durable_operations (tenant_id,id,kind,proposal_id,input_hash,state,next_attempt_at,completed_at,last_error_code)
      values ($1,$2,'source_cleanup',$3,$4,case when $5 then 'pending' else 'blocked' end,
       case when $5 then now() else null end,case when $5 then null else now() end,case when $5 then null else 'source_retained' end)"#,
       tenant.as_uuid(),operation.as_uuid(),change.as_uuid(),input_hash,forgetting).execute(&mut *conn).await.map_err(storage)?;
    sqlx::query!(r#"insert into artifact_cleanup_targets (tenant_id,operation_id,artifact_id,source_key_digest,change_id,input_hash) values ($1,$2,$3,$4,$5,$6)"#,
      tenant.as_uuid(),operation.as_uuid(),source.artifact_id,source.source_key_digest,change.as_uuid(),input_hash).execute(&mut *conn).await.map_err(storage)?;
    sqlx::query!(r#"insert into operation_outbox (tenant_id,operation_id,operation_version,state,next_dispatch_at,completed_at)
      values ($1,$2,1,case when $3 then 'pending' else 'completed' end,case when $3 then now() else null end,case when $3 then null else now() end)"#,
      tenant.as_uuid(),operation.as_uuid(),forgetting).execute(conn).await.map_err(storage)?;
    Ok(operation)
}

/// Inspect only after current source-reading authorization, including aliases.
pub async fn status(
    conn: &mut PgConnection,
    tenant: TenantId,
    artifact: ImportArtifactId,
) -> Result<Option<crate::source_control::SourceStatus>> {
    sqlx::query_as!(crate::source_control::SourceStatus,r#"select s.removed_at,target.change_id,target.operation_id,op.state,target.phase,op.last_error_code,
      (s.forget_change_id is not null) as "forgetting!" from source_artifact_addresses address
      join artifact_suppressions s on s.tenant_id=address.tenant_id and s.source_key_digest=address.source_key_digest
      join artifact_cleanup_targets target on target.tenant_id=s.tenant_id and target.source_key_digest=s.source_key_digest
      join durable_operations op on op.tenant_id=target.tenant_id and op.id=target.operation_id
      where address.tenant_id=$1 and address.artifact_id=$2
      order by (target.change_id=coalesce(s.forget_change_id,s.change_id)) desc,op.created_at desc limit 1"#,
      tenant.as_uuid(),artifact.as_uuid()).fetch_optional(conn).await.map_err(storage)
}

/// Retained address for exactly the requested operation.
pub async fn operation_source(
    conn: &mut PgConnection,
    tenant: TenantId,
    operation: DurableOperationId,
) -> Result<Option<ArtifactAddress>> {
    sqlx::query_as!(ArtifactAddress,r#"select address.artifact_id,address.job_id,address.project_id,address.scope_id,address.workspace_id,address.content_hash,address.source_key_digest
      from artifact_cleanup_targets target join source_artifact_addresses address on address.tenant_id=target.tenant_id and address.artifact_id=target.artifact_id
      where target.tenant_id=$1 and target.operation_id=$2"#,tenant.as_uuid(),operation.as_uuid()).fetch_optional(conn).await.map_err(storage)
}

/// Exact withdrawal receipts survive a later Forget upgrade.
pub async fn operation_status(
    conn: &mut PgConnection,
    tenant: TenantId,
    operation: DurableOperationId,
) -> Result<Option<crate::source_control::SourceStatus>> {
    sqlx::query_as!(crate::source_control::SourceStatus,r#"select s.removed_at,target.change_id,target.operation_id,op.state,target.phase,op.last_error_code,
      coalesce(s.forget_change_id=target.change_id,false) as "forgetting!" from artifact_cleanup_targets target
      join artifact_suppressions s on s.tenant_id=target.tenant_id and s.source_key_digest=target.source_key_digest
      join durable_operations op on op.tenant_id=target.tenant_id and op.id=target.operation_id
      where target.tenant_id=$1 and target.operation_id=$2"#,tenant.as_uuid(),operation.as_uuid()).fetch_optional(conn).await.map_err(storage)
}

/// Same bounded attempt contract, with an artifact-specific SQL primitive.
pub async fn chunk(
    conn: &mut PgConnection,
    tenant: TenantId,
    claim: &crate::source_control::CleanupClaim,
    owner: &str,
) -> Result<crate::source_control::CleanupPage> {
    sqlx::query_as!(
        crate::source_control::CleanupPage,
        r#"select progress as "progress!",done as "done!",erased as "erased!"
      from synveda_artifact_cleanup_chunk($1,$2,$3,$4,100)"#,
        tenant.as_uuid(),
        claim.operation_id,
        owner,
        claim.attempts
    )
    .fetch_one(conn)
    .await
    .map_err(storage)
}

fn storage(error: sqlx::Error) -> Error {
    Error::Storage {
        message: format!("artifact source control: {error}"),
    }
}
