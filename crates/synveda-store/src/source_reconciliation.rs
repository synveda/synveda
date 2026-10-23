//! Durable inbound requests and immutable complete observations.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use synveda_types::{
    ConfigurationVersionId, DurableOperationId, Error, IdentityId, ImportArtifactId, ImportJobId,
    ProjectId, RepositoryId, Result, ScopeId, TenantId,
};
use uuid::Uuid;

/// Private cursor address. No source plaintext or filesystem path is retained.
#[derive(Debug)]
pub struct SourceHead {
    /// Actual project scope.
    pub scope_id: Uuid,
    /// Original/current attachment of the same immutable repository identity.
    pub repository_id: Uuid,
    /// Server-derived source identity surviving reattachment.
    pub repository_identity_digest: String,
    /// Last completely committed observation.
    pub observation_id: Option<Uuid>,
    /// Current runtime work, when present.
    pub active_operation_id: Option<Uuid>,
    /// Most recent refresh receipt, including failures.
    pub last_operation_id: Option<Uuid>,
}

/// Frozen operational request and safe receipt fields.
#[derive(Debug)]
pub struct SourceRequest {
    /// Durable address.
    pub operation_id: Uuid,
    /// Exact project.
    pub project_id: Uuid,
    /// Stable configured source.
    pub source_id: String,
    /// Actual attachment admitted for this request.
    pub repository_id: Uuid,
    /// Frozen governed Configuration version.
    pub configuration_version_id: Uuid,
    /// Canonical descriptor digest.
    pub descriptor_hash: String,
    /// Deployment custody digest.
    pub custody_digest: String,
    /// Current sealed value revision for remote reads.
    pub credential_revision: Option<i64>,
    /// Successful predecessor; completion is a compare-and-swap.
    pub previous_observation_id: Option<Uuid>,
    /// Exact expected complete path coverage.
    pub path_count: i32,
    /// Commit pinned once before blob reads.
    pub commit_oid: Option<String>,
    /// Corresponding verified tree.
    pub tree_oid: Option<String>,
    /// Persisted user, independently reauthorized by the worker.
    pub requested_by: Uuid,
    /// Database request time; stable OKF valid-time defaults across retry.
    pub created_at: DateTime<Utc>,
    /// Closed durable state.
    pub state: String,
    /// Stable bounded failure code.
    pub error_code: Option<String>,
    /// Monotonic attempt fence.
    pub attempts: i32,
    /// Authorized safe result ids.
    pub result: serde_json::Value,
}

/// Claim of one immutable request, or terminal exhaustion after a crashed lease.
pub struct ReconcileClaim {
    /// Exact frozen target plus requester.
    pub request: SourceRequest,
    /// No further transport may run after exhaustion.
    pub exhausted: bool,
}

/// One already-verified path result to commit, with no second plaintext copy.
pub struct ObservedPath {
    /// Exact configured path; persisted only as SHA-256.
    pub path: String,
    /// `present`, `missing`, or `suppressed`.
    pub state: &'static str,
    /// Git identity for present or suppressed bytes.
    pub blob_oid: Option<String>,
    /// Exact OKF BLAKE3 content hash.
    pub content_hash: Option<String>,
    /// Actual retained artifact, if this path has been materialized.
    pub artifact_id: Option<ImportArtifactId>,
}

/// Prior complete path evidence, before fetching or deciding new work.
pub struct PriorPath {
    /// Closed observation state.
    pub state: String,
    /// Exact Git identity, absent for proven deletion.
    pub blob_oid: Option<String>,
    /// Original content digest.
    pub content_hash: Option<String>,
    /// Actual immutable artifact.
    pub artifact_id: Option<Uuid>,
}

/// Load a private source head; callers decide SourceRead independently.
pub async fn head(
    conn: &mut PgConnection,
    tenant: TenantId,
    project: ProjectId,
    source: &str,
) -> Result<Option<SourceHead>> {
    sqlx::query_as!(SourceHead,r#"select scope_id,repository_id,repository_identity_digest,observation_id,active_operation_id,last_operation_id
      from source_reconcile_heads where tenant_id=$1 and project_id=$2 and source_id=$3"#,
      tenant.as_uuid(),project.as_uuid(),source).fetch_optional(conn).await.map_err(storage)
}

/// Resolve an exact runtime target without filtering its required evidence.
pub async fn request(
    conn: &mut PgConnection,
    tenant: TenantId,
    operation: DurableOperationId,
) -> Result<Option<SourceRequest>> {
    sqlx::query_as!(SourceRequest,r#"select r.operation_id,r.project_id,r.source_id,r.repository_id,r.configuration_version_id,
      r.descriptor_hash,r.custody_digest,r.credential_revision,r.previous_observation_id,r.path_count,r.commit_oid,r.tree_oid,
      op.requested_by_identity_id as "requested_by!",op.created_at,op.state,op.last_error_code as error_code,op.attempts,op.result
      from source_reconcile_requests r join durable_operations op on op.tenant_id=r.tenant_id and op.id=r.operation_id
      where r.tenant_id=$1 and r.operation_id=$2 and op.kind='source_reconcile'"#,
      tenant.as_uuid(),operation.as_uuid()).fetch_optional(conn).await.map_err(storage)
}

/// Queue operational work after current Cedar and effective Configuration checks.
#[allow(clippy::too_many_arguments)]
pub async fn enqueue(
    conn: &mut PgConnection,
    tenant: TenantId,
    project: ProjectId,
    scope: ScopeId,
    source: &str,
    repository: RepositoryId,
    configuration: ConfigurationVersionId,
    descriptor_hash: &str,
    custody_digest: &str,
    credential_revision: Option<i64>,
    path_count: i32,
    requester: IdentityId,
    input_hash: &str,
) -> Result<DurableOperationId> {
    sqlx::query!(r#"insert into source_reconcile_heads(tenant_id,project_id,scope_id,source_id,repository_id,repository_identity_digest)
      values($1,$2,$3,$4,$5,repeat('0',64)) on conflict(tenant_id,project_id,source_id) do nothing"#,
      tenant.as_uuid(),project.as_uuid(),scope.as_uuid(),source,repository.as_uuid()).execute(&mut *conn).await.map_err(storage)?;
    let head = sqlx::query!(
        r#"select observation_id,active_operation_id from source_reconcile_heads
      where tenant_id=$1 and project_id=$2 and source_id=$3 for update"#,
        tenant.as_uuid(),
        project.as_uuid(),
        source
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage)?;
    if head.active_operation_id.is_some() {
        return Err(conflict());
    }
    let id = DurableOperationId::new();
    sqlx::query!(r#"insert into durable_operations(tenant_id,id,kind,input_hash,requested_by_identity_id,state,next_attempt_at)
      values($1,$2,'source_reconcile',$3,$4,'pending',clock_timestamp())"#,
      tenant.as_uuid(),id.as_uuid(),input_hash,requester.as_uuid()).execute(&mut *conn).await.map_err(storage)?;
    sqlx::query!(r#"insert into source_reconcile_requests(tenant_id,operation_id,project_id,source_id,repository_id,
      configuration_version_id,descriptor_hash,custody_digest,credential_revision,previous_observation_id,path_count)
      values($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
      tenant.as_uuid(),id.as_uuid(),project.as_uuid(),source,repository.as_uuid(),configuration.as_uuid(),descriptor_hash,custody_digest,
      credential_revision,head.observation_id,path_count).execute(&mut *conn).await.map_err(storage)?;
    sqlx::query!(r#"update source_reconcile_heads set repository_id=$4,active_operation_id=$5,last_operation_id=$5
      where tenant_id=$1 and project_id=$2 and source_id=$3"#,
      tenant.as_uuid(),project.as_uuid(),source,repository.as_uuid(),id.as_uuid()).execute(&mut *conn).await.map_err(storage)?;
    sqlx::query!(r#"insert into operation_outbox(tenant_id,operation_id,operation_version,state,next_dispatch_at)
      values($1,$2,1,'pending',clock_timestamp())"#,tenant.as_uuid(),id.as_uuid()).execute(conn).await.map_err(storage)?;
    Ok(id)
}

/// Read one prior path within the frozen coverage, without provider IO.
pub async fn prior_path(
    conn: &mut PgConnection,
    tenant: TenantId,
    observation: Option<Uuid>,
    path: &str,
) -> Result<Option<PriorPath>> {
    sqlx::query_as!(PriorPath,r#"select state,blob_oid,content_hash,artifact_id from source_observation_paths
      where tenant_id=$1 and observation_id=$2 and path_digest=encode(sha256(convert_to($3,'UTF8')),'hex')"#,
      tenant.as_uuid(),observation,path).fetch_optional(conn).await.map_err(storage)
}

/// Exact current item and the admitted bytes behind its source dependency.
pub struct CurrentSourceItem {
    /// Actual current item, independently authorized by the caller.
    pub id: Uuid,
    /// Immutable original source bytes, independent of semantic valid time.
    pub content_hash: String,
}

/// Adopt the configured provider ID without rewriting any retained artifact root.
pub async fn bind_github_identity(
    conn: &mut PgConnection,
    tenant: TenantId,
    project: ProjectId,
    scope: ScopeId,
    repository: RepositoryId,
    provider_id: i64,
) -> Result<()> {
    sqlx::query_scalar!(
        r#"select synveda_bind_github_source_identity($1,$2,$3,$4,$5) as "root_digest?""#,
        tenant.as_uuid(),
        project.as_uuid(),
        scope.as_uuid(),
        repository.as_uuid(),
        provider_id
    )
    .fetch_one(conn)
    .await
    .map_err(|error| {
        match error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref()
        {
            Some("23514" | "23505") => conflict(),
            _ => storage(error),
        }
    })?
    .ok_or_else(conflict)?;
    Ok(())
}

/// Validate the immutable configured provider assertion under final admission.
pub async fn github_identity_bound(
    conn: &mut PgConnection,
    tenant: TenantId,
    project: ProjectId,
    repository: RepositoryId,
    provider_id: i64,
) -> Result<bool> {
    sqlx::query_scalar!(r#"select exists(select 1 from source_repository_attachments
      where tenant_id=$1 and project_id=$2 and repository_id=$3 and github_repository_id=$4) as "bound!""#,
      tenant.as_uuid(),project.as_uuid(),repository.as_uuid(),provider_id).fetch_one(conn).await.map_err(storage)
}

/// Bounded exact current items carrying this configured source/path lineage.
/// The caller still independently decides each item's scope and sensitivity.
pub async fn current_items(
    conn: &mut PgConnection,
    tenant: TenantId,
    project: ProjectId,
    source: &str,
    path: &str,
) -> Result<Vec<CurrentSourceItem>> {
    let ids=sqlx::query_as!(CurrentSourceItem,r#"select distinct item.id,address.content_hash from knowledge_items item
        join knowledge_revision_source_headers link on link.tenant_id=item.tenant_id and link.knowledge_revision_id=item.current_revision_id
        join knowledge_sources source on source.tenant_id=link.tenant_id and source.id=link.knowledge_source_id
        join source_artifact_addresses address on address.tenant_id=source.tenant_id and address.artifact_id=source.import_artifact_id
        join import_jobs job on job.tenant_id=address.tenant_id and job.id=address.job_id
        where item.tenant_id=$1 and item.project_id=$2 and item.lifecycle_state in ('active','stale')
         and job.project_id=$2 and job.managed_source_id=$3
         and address.path_digest=encode(sha256(convert_to($4,'UTF8')),'hex') order by item.id,address.content_hash limit 97"#,
       tenant.as_uuid(),project.as_uuid(),source,path).fetch_all(conn).await.map_err(storage)?;
    if ids.len() > 96 {
        return Err(Error::Invalid {
            message: "Source path exceeds the bounded current Knowledge comparison set".to_owned(),
        });
    }
    Ok(ids)
}

/// Resolve permanent alias state before fetching unchanged blobs or persisting.
pub async fn artifact_state(
    conn: &mut PgConnection,
    tenant: TenantId,
    project: ProjectId,
    source: &str,
    path: &str,
    hash: &str,
) -> Result<(Option<ImportArtifactId>, bool)> {
    let row=sqlx::query!(r#"select (min(address.artifact_id::text) filter (where job.managed_source_id=head.source_id
       and job.repository_identity_digest=head.repository_identity_digest))::uuid as artifact_id,
      coalesce(bool_or(suppression.source_key_digest is not null),false) as "suppressed!"
      from source_reconcile_heads head join source_artifact_addresses address
        on address.tenant_id=head.tenant_id and address.project_id=head.project_id and address.scope_id=head.scope_id
       and address.source_identity_digest=encode(sha256(convert_to(jsonb_build_array('repository',head.repository_identity_digest)::text,'UTF8')),'hex')
      join import_jobs job on job.tenant_id=address.tenant_id and job.id=address.job_id
      left join artifact_suppressions suppression on suppression.tenant_id=address.tenant_id and suppression.source_key_digest=address.source_key_digest
      where head.tenant_id=$1 and head.project_id=$2 and head.source_id=$3
       and address.path_digest=encode(sha256(convert_to($4,'UTF8')),'hex') and address.content_hash=$5"#,
      tenant.as_uuid(),project.as_uuid(),source,path,hash).fetch_one(conn).await.map_err(storage)?;
    Ok((
        row.artifact_id.map(ImportArtifactId::from_uuid),
        row.suppressed,
    ))
}

/// Claim one due refresh with three attempts, bounded lease and SKIP LOCKED.
pub async fn claim(
    conn: &mut PgConnection,
    tenant: TenantId,
    owner: &str,
) -> Result<Option<ReconcileClaim>> {
    let row=sqlx::query!(r#"select id,attempts from durable_operations where tenant_id=$1 and kind='source_reconcile'
      and cancel_requested_at is null and ((state in ('pending','failed') and next_attempt_at<=clock_timestamp())
        or (state='running' and lease_expires_at<=clock_timestamp()))
      order by created_at,id for update skip locked limit 1"#,tenant.as_uuid()).fetch_optional(&mut *conn).await.map_err(storage)?;
    let Some(row) = row else { return Ok(None) };
    let id = DurableOperationId::from_uuid(row.id);
    if row.attempts >= 3 {
        finish_terminal(
            conn,
            tenant,
            id,
            "dead_lettered",
            Some("lease_retry_exhausted"),
            &serde_json::json!({}),
        )
        .await?;
        return Ok(Some(ReconcileClaim {
            request: request(conn, tenant, id).await?.ok_or_else(conflict)?,
            exhausted: true,
        }));
    }
    sqlx::query!(r#"update operation_attempts set state='retry_scheduled',safe_error_code='lease_expired',completed_at=clock_timestamp()
      where tenant_id=$1 and operation_id=$2 and state='running'"#,tenant.as_uuid(),row.id).execute(&mut *conn).await.map_err(storage)?;
    let attempt = row.attempts + 1;
    sqlx::query!(r#"update durable_operations set state='running',attempts=$3,lease_owner=$4,lease_expires_at=clock_timestamp()+interval '90 seconds',
      started_at=coalesce(started_at,clock_timestamp()),updated_at=clock_timestamp(),next_attempt_at=null,last_error_code=null
      where tenant_id=$1 and id=$2"#,tenant.as_uuid(),row.id,attempt,owner).execute(&mut *conn).await.map_err(storage)?;
    sqlx::query!(
        r#"insert into operation_attempts(tenant_id,operation_id,attempt_number,worker_id)
      values($1,$2,$3,$4)"#,
        tenant.as_uuid(),
        row.id,
        attempt,
        owner
    )
    .execute(&mut *conn)
    .await
    .map_err(storage)?;
    Ok(Some(ReconcileClaim {
        request: request(conn, tenant, id).await?.ok_or_else(conflict)?,
        exhausted: false,
    }))
}

/// Recheck the exact execution fence and hold its operation lock for commit.
pub async fn fence(
    conn: &mut PgConnection,
    tenant: TenantId,
    id: DurableOperationId,
    attempt: i32,
    owner: &str,
) -> Result<()> {
    let valid=sqlx::query_scalar!(r#"select true as "valid!" from durable_operations op
      where op.tenant_id=$1 and op.id=$2 and op.kind='source_reconcile' and op.state='running'
       and op.attempts=$3 and op.lease_owner=$4 and op.lease_expires_at>clock_timestamp() and op.cancel_requested_at is null
       and exists(select 1 from operation_attempts a where a.tenant_id=op.tenant_id and a.operation_id=op.id
        and a.attempt_number=op.attempts and a.worker_id=op.lease_owner and a.state='running') for update of op"#,
      tenant.as_uuid(),id.as_uuid(),attempt,owner).fetch_optional(&mut *conn).await.map_err(storage)?;
    if valid != Some(true) {
        return Err(conflict());
    }
    sqlx::query!(r#"select set_config('synveda.source_reconcile_operation',$1,true) as operation_config,
      set_config('synveda.source_reconcile_attempt',$2,true) as attempt_config,set_config('synveda.source_reconcile_owner',$3,true) as owner_config"#,
      id.to_string(),attempt.to_string(),owner).fetch_one(conn).await.map_err(storage)?;
    Ok(())
}

/// Store the once-only commit/tree pin under the live attempt.
pub async fn pin(
    conn: &mut PgConnection,
    tenant: TenantId,
    id: DurableOperationId,
    commit: &str,
    tree: &str,
) -> Result<()> {
    let result = sqlx::query!(
        r#"update source_reconcile_requests set commit_oid=$3,tree_oid=$4
      where tenant_id=$1 and operation_id=$2 and commit_oid is null"#,
        tenant.as_uuid(),
        id.as_uuid(),
        commit,
        tree
    )
    .execute(conn)
    .await
    .map_err(storage)?;
    if result.rows_affected() != 1 {
        return Err(conflict());
    }
    Ok(())
}

/// Atomically commit complete coverage, source work and the successful cursor.
pub async fn complete(
    conn: &mut PgConnection,
    tenant: TenantId,
    request: &SourceRequest,
    job: Option<ImportJobId>,
    paths: &[ObservedPath],
) -> Result<Uuid> {
    if paths.len() != usize::try_from(request.path_count).map_err(|_| conflict())? {
        return Err(conflict());
    }
    let id = Uuid::now_v7();
    sqlx::query!(r#"insert into source_observations(tenant_id,id,project_id,source_id,operation_id,previous_id,descriptor_hash,commit_oid,tree_oid,import_job_id)
      values($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#,tenant.as_uuid(),id,request.project_id,request.source_id,request.operation_id,
      request.previous_observation_id,request.descriptor_hash,request.commit_oid,request.tree_oid,job.map(|i|i.as_uuid())).execute(&mut *conn).await.map_err(storage)?;
    for path in paths {
        sqlx::query!(r#"insert into source_observation_paths(tenant_id,observation_id,path_digest,state,blob_oid,content_hash,artifact_id)
          values($1,$2,encode(sha256(convert_to($3,'UTF8')),'hex'),$4,$5,$6,$7)"#,
          tenant.as_uuid(),id,path.path,path.state,path.blob_oid,path.content_hash,path.artifact_id.map(|i|i.as_uuid())).execute(&mut *conn).await.map_err(storage)?;
    }
    sqlx::query!(
        r#"update source_reconcile_heads set observation_id=$4,active_operation_id=null
      where tenant_id=$1 and project_id=$2 and source_id=$3"#,
        tenant.as_uuid(),
        request.project_id,
        request.source_id,
        id
    )
    .execute(&mut *conn)
    .await
    .map_err(storage)?;
    finish_terminal(
        conn,
        tenant,
        DurableOperationId::from_uuid(request.operation_id),
        "succeeded",
        None,
        &serde_json::json!({"observation_id":id,"import_job_id":job}),
    )
    .await?;
    Ok(id)
}

/// Commit a content-free failed attempt. Only a live owner may report it.
pub async fn fail(
    conn: &mut PgConnection,
    tenant: TenantId,
    request: &SourceRequest,
    owner: &str,
    code: &'static str,
    permanent: bool,
) -> Result<()> {
    let id = DurableOperationId::from_uuid(request.operation_id);
    fence(conn, tenant, id, request.attempts, owner).await?;
    if permanent || request.attempts >= 3 {
        return finish_terminal(
            conn,
            tenant,
            id,
            if permanent {
                "blocked"
            } else {
                "dead_lettered"
            },
            Some(code),
            &serde_json::json!({}),
        )
        .await;
    }
    sqlx::query!(r#"update durable_operations set state='failed',last_error_code=$3,lease_owner=null,lease_expires_at=null,
      next_attempt_at=clock_timestamp()+interval '15 seconds',updated_at=clock_timestamp() where tenant_id=$1 and id=$2"#,
      tenant.as_uuid(),id.as_uuid(),code).execute(&mut *conn).await.map_err(storage)?;
    sqlx::query!(r#"update operation_attempts set state='retry_scheduled',safe_error_code=$3,completed_at=clock_timestamp()
      where tenant_id=$1 and operation_id=$2 and state='running'"#,tenant.as_uuid(),id.as_uuid(),code).execute(conn).await.map_err(storage)?;
    Ok(())
}

/// Cancel before completion; fetched bytes cannot pass a cancelled fence.
pub async fn cancel(
    conn: &mut PgConnection,
    tenant: TenantId,
    id: DurableOperationId,
) -> Result<bool> {
    sqlx::query!(r#"select id from durable_operations where tenant_id=$1 and id=$2 and kind='source_reconcile' for update"#,
      tenant.as_uuid(),id.as_uuid()).fetch_optional(&mut *conn).await.map_err(storage)?.ok_or_else(conflict)?;
    let current = request(conn, tenant, id).await?.ok_or_else(conflict)?;
    if matches!(current.state.as_str(), "pending" | "running" | "failed") {
        finish_terminal(
            conn,
            tenant,
            id,
            "cancelled",
            Some("cancelled"),
            &serde_json::json!({}),
        )
        .await?;
        return Ok(true);
    }
    Ok(false)
}

async fn finish_terminal(
    conn: &mut PgConnection,
    tenant: TenantId,
    id: DurableOperationId,
    state: &str,
    code: Option<&str>,
    result: &serde_json::Value,
) -> Result<()> {
    let changed = sqlx::query!(r#"update durable_operations set state=$3,last_error_code=$4,result=$5,progress_percent=case when $3='succeeded' then 100 else progress_percent end,
      completed_at=clock_timestamp(),updated_at=clock_timestamp(),lease_owner=null,lease_expires_at=null,next_attempt_at=null,
      cancel_requested_at=case when $3='cancelled' then coalesce(cancel_requested_at,clock_timestamp()) else cancel_requested_at end
      where tenant_id=$1 and id=$2 and ($3<>'succeeded' or (state='running' and lease_expires_at>clock_timestamp()))"#,tenant.as_uuid(),id.as_uuid(),state,code,result).execute(&mut *conn).await.map_err(storage)?;
    if changed.rows_affected() != 1 {
        return Err(conflict());
    }
    sqlx::query!(r#"update operation_attempts set state=case when $3='succeeded' then 'succeeded' when $3='cancelled' then 'cancelled' else 'dead_lettered' end,
      safe_error_code=$4,completed_at=clock_timestamp() where tenant_id=$1 and operation_id=$2 and state='running'"#,
      tenant.as_uuid(),id.as_uuid(),state,code).execute(&mut *conn).await.map_err(storage)?;
    sqlx::query!(r#"update operation_outbox set state='completed',completed_at=clock_timestamp(),updated_at=clock_timestamp(),
      lease_owner=null,lease_expires_at=null,next_dispatch_at=null where tenant_id=$1 and operation_id=$2"#,
      tenant.as_uuid(),id.as_uuid()).execute(&mut *conn).await.map_err(storage)?;
    sqlx::query!(r#"update source_reconcile_heads set active_operation_id=null where tenant_id=$1 and active_operation_id=$2"#,
      tenant.as_uuid(),id.as_uuid()).execute(conn).await.map_err(storage)?;
    Ok(())
}

/// One bounded page of ordinary active-tenant routing ids for the supervised worker.
pub async fn active_tenants(pool: &sqlx::PgPool, after: Option<TenantId>) -> Result<Vec<TenantId>> {
    let ids=sqlx::query_scalar!("select id from tenants where status='active' and ($1::uuid is null or id>$1) order by id limit 64",
        after.map(|t|t.as_uuid()) as Option<Uuid>).fetch_all(pool).await.map_err(storage)?;
    Ok(ids.into_iter().map(TenantId::from_uuid).collect())
}

fn conflict() -> Error {
    Error::Conflict {
        message: "Source refresh is unavailable under its current cursor or execution fence"
            .to_owned(),
    }
}
fn storage(error: sqlx::Error) -> Error {
    Error::Storage {
        message: error.to_string(),
    }
}
