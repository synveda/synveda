//! MEM-8 source suppression and domain-specific durable cleanup.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use synveda_types::knowledge::SourceControlMode;
use synveda_types::{
    DurableOperationId, Error, ProposalId, Result, SessionEventId, SessionId, TenantId,
};
use uuid::Uuid;

/// Content-free source address survives event retention and erasure.
pub struct SourceAddress {
    /// Original immutable event.
    pub event_id: Uuid,
    /// Original Session.
    pub session_id: Uuid,
    /// Actual governing scope.
    pub scope_id: Uuid,
    /// Original workspace anchor, retained without a Session foreign key.
    pub workspace_id: Uuid,
    /// Original project anchor.
    pub project_id: Option<Uuid>,
    /// Original immutable sequence.
    pub sequence: i64,
    /// Hash of the raw client id, never the id text.
    pub client_event_id_digest: String,
    /// Original unredacted input hash.
    pub source_payload_hash: String,
    /// Public redacted payload hash inspected by the caller.
    pub payload_hash: String,
}

/// Read the source address; the application independently authorizes its Session.
pub async fn address(
    conn: &mut PgConnection,
    tenant: TenantId,
    session: SessionId,
    event: SessionEventId,
) -> Result<Option<SourceAddress>> {
    sqlx::query_as!(SourceAddress, r#"select event_id,session_id,scope_id,workspace_id,project_id,sequence,client_event_id_digest,source_payload_hash,payload_hash
        from source_event_addresses where tenant_id=$1 and session_id=$2 and event_id=$3"#,
        tenant.as_uuid(),session.as_uuid(),event.as_uuid()).fetch_optional(conn).await.map_err(storage)
}

/// Resolve an exact source address for independently authorized provenance.
pub async fn address_by_event(
    conn: &mut PgConnection,
    tenant: TenantId,
    event: SessionEventId,
) -> Result<Option<SourceAddress>> {
    sqlx::query_as!(SourceAddress,r#"select event_id,session_id,scope_id,workspace_id,project_id,sequence,client_event_id_digest,source_payload_hash,payload_hash
      from source_event_addresses where tenant_id=$1 and event_id=$2"#,tenant.as_uuid(),event.as_uuid())
      .fetch_optional(conn).await.map_err(storage)
}

/// Check before duplicate append disclosure as well as new insertion. Aliases
/// stay scoped to the original Session; a new materialization cannot restore
/// the original source's receipts or reviewed derivatives.
///
/// One statement for the whole batch, refused whole when any event is
/// suppressed: a round trip per event put a hundred link crossings back on
/// the MEM-1 ack path that the batched insert had removed.
pub async fn append_available(
    conn: &mut PgConnection,
    tenant: TenantId,
    session: SessionId,
    client_ids: &[String],
    source_hashes: &[String],
) -> Result<()> {
    let suppressed = sqlx::query_scalar!(r#"select exists(select 1 from source_suppressions
        where tenant_id=$1 and session_id=$2 and
        (client_event_id_digest=any(array(select encode(sha256(convert_to(client_id,'UTF8')),'hex')
            from unnest($3::text[]) client_id)) or source_payload_hash=any($4::text[]))) as "suppressed!""#,
        tenant.as_uuid(),session.as_uuid(),client_ids,source_hashes).fetch_one(conn).await.map_err(storage)?;
    if suppressed {
        return Err(unavailable());
    }
    Ok(())
}

/// Source eligibility is independent of permission to inspect its descriptor.
pub async fn available(
    conn: &mut PgConnection,
    tenant: TenantId,
    kind: &str,
    id: Uuid,
) -> Result<bool> {
    if !matches!(
        kind,
        "import_artifact"
            | "import_mapping"
            | "session_event"
            | "knowledge_revision"
            | "knowledge_source"
            | "capture_candidate"
            | "knowledge_change"
            | "context_run"
    ) {
        return Err(Error::Invalid {
            message: "unknown source dependency family".to_owned(),
        });
    }
    sqlx::query_scalar!(
        r#"select synveda_source_available($1,$2,$3) as "available!""#,
        tenant.as_uuid(),
        kind,
        id
    )
    .fetch_one(conn)
    .await
    .map_err(storage)
}

/// Apply only inside an authorized, exclusive VedaFlow effect transaction.
/// The suppression and its operation are committed with the applied result.
#[allow(clippy::too_many_arguments)]
pub async fn apply(
    conn: &mut PgConnection,
    tenant: TenantId,
    source: &SourceAddress,
    change: ProposalId,
    input_hash: &str,
    mode: SourceControlMode,
    actor_hash: &str,
    reason_hash: &str,
) -> Result<DurableOperationId> {
    if let Some(previous) = status(
        conn,
        tenant,
        SessionId::from_uuid(source.session_id),
        SessionEventId::from_uuid(source.event_id),
    )
    .await?
        && (mode == SourceControlMode::Withdraw || previous.forgetting)
    {
        return Err(unavailable());
    }
    let forgetting = mode == SourceControlMode::Forget;
    sqlx::query!(r#"insert into source_suppressions
        (tenant_id,event_id,session_id,scope_id,client_event_id_digest,source_payload_hash,payload_hash,
         change_id,forget_change_id,actor_hash,reason_hash)
        values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
        on conflict (tenant_id,event_id) do update set forget_change_id=coalesce(source_suppressions.forget_change_id,excluded.forget_change_id)"#,
        tenant.as_uuid(),source.event_id,source.session_id,source.scope_id,source.client_event_id_digest,
        source.source_payload_hash,source.payload_hash,change.as_uuid(),
        forgetting.then(||change.as_uuid()),actor_hash,reason_hash).execute(&mut *conn).await.map_err(storage)?;
    let id = DurableOperationId::new();
    sqlx::query!(r#"insert into durable_operations
        (tenant_id,id,kind,proposal_id,input_hash,state,next_attempt_at,completed_at,last_error_code)
        values ($1,$2,'source_cleanup',$3,$4,
        case when $5 then 'pending' else 'blocked' end,
        case when $5 then now() else null end,
        case when $5 then null else now() end,
        case when $5 then null else 'source_retained' end)"#,
        tenant.as_uuid(),id.as_uuid(),change.as_uuid(),input_hash,forgetting).execute(&mut *conn).await.map_err(storage)?;
    sqlx::query!(
        r#"insert into source_cleanup_targets (tenant_id,operation_id,event_id,change_id,input_hash)
        values ($1,$2,$3,$4,$5)"#,
        tenant.as_uuid(),
        id.as_uuid(),
        source.event_id,
        change.as_uuid(),
        input_hash
    )
    .execute(&mut *conn)
    .await
    .map_err(storage)?;
    sqlx::query!(r#"insert into operation_outbox (tenant_id,operation_id,operation_version,state,next_dispatch_at,completed_at)
        values ($1,$2,1,case when $3 then 'pending' else 'completed' end,
        case when $3 then now() else null end,case when $3 then null else now() end)"#,
        tenant.as_uuid(),id.as_uuid(),forgetting).execute(&mut *conn).await.map_err(storage)?;
    Ok(id)
}

/// Customer-safe state, without affected derivative counts or addresses.
pub struct SourceStatus {
    /// Applied logical removal boundary.
    pub removed_at: DateTime<Utc>,
    /// Governed source effect.
    pub change_id: Uuid,
    /// Durable cleanup address.
    pub operation_id: Uuid,
    /// Closed operation state, validated by the public view.
    pub state: String,
    /// Fixed cleanup stage, never a dynamic resource name.
    pub phase: i16,
    /// Stable content-free failure/hold code.
    pub last_error_code: Option<String>,
    /// Whether physical cleanup was requested.
    pub forgetting: bool,
}

/// Inspect only after current SessionRead authorization.
pub async fn status(
    conn: &mut PgConnection,
    tenant: TenantId,
    session: SessionId,
    event: SessionEventId,
) -> Result<Option<SourceStatus>> {
    sqlx::query_as!(SourceStatus,r#"select s.removed_at,t.change_id,t.operation_id,o.state,t.phase,o.last_error_code,
        (s.forget_change_id is not null) as "forgetting!"
        from source_suppressions s join source_cleanup_targets t on t.tenant_id=s.tenant_id and t.event_id=s.event_id
        join durable_operations o on o.tenant_id=t.tenant_id and o.id=t.operation_id
        where s.tenant_id=$1 and s.session_id=$2 and s.event_id=$3
        order by (t.change_id=coalesce(s.forget_change_id,s.change_id)) desc,o.created_at desc limit 1"#,
        tenant.as_uuid(),session.as_uuid(),event.as_uuid()).fetch_optional(conn).await.map_err(storage)
}

/// Exact retained cleanup address, without Skill-specific joins or source text.
pub async fn operation_source(
    conn: &mut PgConnection,
    tenant: TenantId,
    operation: synveda_types::DurableOperationId,
) -> Result<Option<SourceAddress>> {
    sqlx::query_as!(SourceAddress,r#"select address.event_id,address.session_id,address.scope_id,
        address.workspace_id,address.project_id,address.sequence,address.client_event_id_digest,address.source_payload_hash,address.payload_hash
        from source_cleanup_targets target join source_event_addresses address
        on address.tenant_id=target.tenant_id and address.event_id=target.event_id
        where target.tenant_id=$1 and target.operation_id=$2"#,tenant.as_uuid(),operation.as_uuid())
        .fetch_optional(conn).await.map_err(storage)
}

/// Inspect the addressed cleanup operation, including an earlier withdrawal
/// receipt after a later Forget upgrade, rather than substituting the latest job.
pub async fn operation_status(
    conn: &mut PgConnection,
    tenant: TenantId,
    operation: synveda_types::DurableOperationId,
) -> Result<Option<SourceStatus>> {
    sqlx::query_as!(SourceStatus,r#"select s.removed_at,t.change_id,t.operation_id,o.state,t.phase,o.last_error_code,
        coalesce(s.forget_change_id=t.change_id,false) as "forgetting!"
        from source_cleanup_targets t join source_suppressions s on s.tenant_id=t.tenant_id and s.event_id=t.event_id
        join durable_operations o on o.tenant_id=t.tenant_id and o.id=t.operation_id
        where t.tenant_id=$1 and t.operation_id=$2"#,tenant.as_uuid(),operation.as_uuid())
        .fetch_optional(conn).await.map_err(storage)
}

/// One live domain-specific lease. It authorizes no arbitrary target id.
pub struct CleanupClaim {
    /// Exact operation.
    pub operation_id: Uuid,
    /// Monotonic attempt.
    pub attempts: i32,
    /// The final expired attempt was terminalized; no page may execute.
    pub exhausted: bool,
    /// Closed source family chosen from the exact durable target.
    pub artifact: bool,
}

/// Claim or reclaim a page executor. Provider work is never performed here.
pub async fn claim(
    conn: &mut PgConnection,
    tenant: TenantId,
    owner: &str,
) -> Result<Option<CleanupClaim>> {
    if owner.is_empty() || owner.len() > 255 {
        return Err(Error::Invalid {
            message: "invalid source cleanup owner".to_owned(),
        });
    }
    let exhausted=sqlx::query!(r#"select id,attempts from durable_operations where tenant_id=$1
        and kind='source_cleanup' and state='running' and attempts>=5 and lease_expires_at<=clock_timestamp()
        order by lease_expires_at,id for update skip locked limit 1"#,tenant.as_uuid())
        .fetch_optional(&mut *conn).await.map_err(storage)?;
    if let Some(exhausted) = exhausted {
        sqlx::query!(r#"update durable_operations set state='dead_lettered',completed_at=clock_timestamp(),updated_at=clock_timestamp(),
            lease_owner=null,lease_expires_at=null,next_attempt_at=null,last_error_code='lease_expired_retry_exhausted'
            where tenant_id=$1 and id=$2"#,tenant.as_uuid(),exhausted.id).execute(&mut *conn).await.map_err(storage)?;
        sqlx::query!(r#"update operation_attempts set state='dead_lettered',completed_at=clock_timestamp(),safe_error_code='lease_expired_retry_exhausted'
            where tenant_id=$1 and operation_id=$2 and attempt_number=$3 and state='running'"#,
            tenant.as_uuid(),exhausted.id,exhausted.attempts).execute(&mut *conn).await.map_err(storage)?;
        sqlx::query!(r#"update operation_outbox set state='completed',completed_at=clock_timestamp(),updated_at=clock_timestamp(),
            lease_owner=null,lease_expires_at=null,next_dispatch_at=null where tenant_id=$1 and operation_id=$2"#,
            tenant.as_uuid(),exhausted.id).execute(&mut *conn).await.map_err(storage)?;
        return Ok(Some(CleanupClaim {
            operation_id: exhausted.id,
            attempts: exhausted.attempts,
            exhausted: true,
            artifact: false,
        }));
    }
    let row = sqlx::query!(
        r#"select o.id,o.attempts,o.lease_owner,exists(select 1 from artifact_cleanup_targets target where target.tenant_id=o.tenant_id and target.operation_id=o.id) as "artifact!",(o.state='running' and o.lease_owner=$2 and o.lease_expires_at>clock_timestamp()) as "continuing!" from durable_operations o
        where o.tenant_id=$1 and o.kind='source_cleanup'
        and ((exists(select 1 from source_cleanup_targets target where target.tenant_id=o.tenant_id and target.operation_id=o.id))
            <> (exists(select 1 from artifact_cleanup_targets target where target.tenant_id=o.tenant_id and target.operation_id=o.id)))
        and ((o.state='running' and o.lease_owner=$2 and o.lease_expires_at>clock_timestamp())
          or (o.attempts<5 and ((o.state in ('pending','failed') and o.next_attempt_at<=statement_timestamp())
          or (o.state='running' and o.lease_expires_at<=clock_timestamp()))))
        order by o.created_at,o.id for update of o skip locked limit 1"#,
        tenant.as_uuid(),owner
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage)?;
    let Some(row) = row else { return Ok(None) };
    if row.continuing {
        return Ok(Some(CleanupClaim {
            operation_id: row.id,
            attempts: row.attempts,
            exhausted: false,
            artifact: row.artifact,
        }));
    }
    sqlx::query!(r#"update operation_attempts set state='retry_scheduled',safe_error_code='lease_expired',completed_at=clock_timestamp()
        where tenant_id=$1 and operation_id=$2 and state='running'"#,tenant.as_uuid(),row.id)
        .execute(&mut *conn).await.map_err(storage)?;
    let attempts = row.attempts + 1;
    sqlx::query!(r#"update durable_operations set state='running',attempts=$3,lease_owner=$4,
        lease_expires_at=clock_timestamp()+interval '30 seconds',started_at=coalesce(started_at,clock_timestamp()),
        updated_at=clock_timestamp(),next_attempt_at=null,last_error_code=null
        where tenant_id=$1 and id=$2"#,tenant.as_uuid(),row.id,attempts,owner)
        .execute(&mut *conn).await.map_err(storage)?;
    sqlx::query!(
        r#"insert into operation_attempts (tenant_id,operation_id,attempt_number,worker_id)
        values ($1,$2,$3,$4)"#,
        tenant.as_uuid(),
        row.id,
        attempts,
        owner
    )
    .execute(&mut *conn)
    .await
    .map_err(storage)?;
    Ok(Some(CleanupClaim {
        operation_id: row.id,
        attempts,
        exhausted: false,
        artifact: row.artifact,
    }))
}

/// Record bounded retry after a rolled-back page. A stale attempt cannot fail
/// a newer owner. Suppression remains permanent through every outcome.
pub async fn fail(
    conn: &mut PgConnection,
    tenant: TenantId,
    claim: &CleanupClaim,
    owner: &str,
) -> Result<bool> {
    let terminal = claim.attempts >= 5;
    let changed=sqlx::query!(r#"update durable_operations set state=case when $5 then 'dead_lettered' else 'failed' end,
        completed_at=case when $5 then clock_timestamp() else null end,updated_at=clock_timestamp(),
        lease_owner=null,lease_expires_at=null,last_error_code='cleanup_failed',
        next_attempt_at=case when $5 then null else clock_timestamp()+make_interval(secs=>least(60,power(2,$3))::double precision) end
        where tenant_id=$1 and id=$2 and kind='source_cleanup' and state='running' and attempts=$3 and lease_owner=$4"#,
        tenant.as_uuid(),claim.operation_id,claim.attempts,owner,terminal).execute(&mut *conn).await.map_err(storage)?.rows_affected();
    if changed == 0 {
        return Ok(false);
    }
    sqlx::query!(r#"update operation_attempts set state=case when $4 then 'dead_lettered' else 'retry_scheduled' end,
        safe_error_code='cleanup_failed',completed_at=clock_timestamp() where tenant_id=$1 and operation_id=$2
        and attempt_number=$3 and state='running'"#,tenant.as_uuid(),claim.operation_id,claim.attempts,terminal)
        .execute(&mut *conn).await.map_err(storage)?;
    if terminal {
        sqlx::query!(r#"update operation_outbox set state='completed',completed_at=clock_timestamp(),updated_at=clock_timestamp(),
            lease_owner=null,lease_expires_at=null,next_dispatch_at=null where tenant_id=$1 and operation_id=$2"#,
            tenant.as_uuid(),claim.operation_id).execute(&mut *conn).await.map_err(storage)?;
    }
    Ok(true)
}

/// A page result commits with the worker's content-free audit record.
pub struct CleanupPage {
    /// Fixed percentage over stages; never an estimate of bytes erased.
    pub progress: i16,
    /// Terminal completion or a retention hold.
    pub done: bool,
    /// Bounded visited targets, for content-free worker telemetry only.
    pub erased: i64,
}

/// Execute at most 100 indexed targets under the exact live attempt.
pub async fn chunk(
    conn: &mut PgConnection,
    tenant: TenantId,
    claim: &CleanupClaim,
    owner: &str,
) -> Result<CleanupPage> {
    if claim.artifact {
        return crate::artifact_control::chunk(conn, tenant, claim, owner).await;
    }
    sqlx::query_as!(
        CleanupPage,
        r#"select progress as "progress!",done as "done!",erased as "erased!"
        from synveda_source_cleanup_chunk($1,$2,$3,$4,100)"#,
        tenant.as_uuid(),
        claim.operation_id,
        owner,
        claim.attempts
    )
    .fetch_one(conn)
    .await
    .map_err(storage)
}

/// Uniform logical source unavailability, with no body or derivative identities.
pub fn unavailable() -> Error {
    Error::Conflict {
        message: "Source evidence is unavailable under current source state".to_owned(),
    }
}

fn storage(error: sqlx::Error) -> Error {
    if let sqlx::Error::Database(database) = &error {
        if database.code().as_deref() == Some("42501") {
            return crate::rls::backstop_error(error);
        }
        if database.code().as_deref() == Some("23514") {
            return unavailable();
        }
    }
    Error::Storage {
        message: format!("source control: {error}"),
    }
}
