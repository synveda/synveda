//! Static exact reads for server-derived Session working state.

use sqlx::PgConnection;
use synveda_types::{
    Error, ImportArtifactId, Result, Sensitivity, SessionEventId, SessionId, TenantId,
};

/// Content-free event head, including unavailable heads for concurrency checks.
pub struct TaskStateHeader {
    /// Exact immutable event.
    pub id: uuid::Uuid,
    /// Admitted payload hash.
    pub payload_hash: String,
    /// Original derivation request hash, retained without plaintext after erasure.
    pub request_hash: Option<String>,
    /// Immutable sequence, including unavailable snapshot heads.
    pub sequence: i64,
}

/// Serialize replacement against both appenders and other TaskState commands.
pub async fn lock_session(
    tx: &mut PgConnection,
    tenant: TenantId,
    session: SessionId,
) -> Result<()> {
    sqlx::query!(
        "select id from sessions where tenant_id=$1 and id=$2 for update",
        tenant.as_uuid(),
        session.as_uuid()
    )
    .fetch_optional(tx)
    .await
    .map_err(storage)?
    .ok_or_else(unavailable)?;
    Ok(())
}

/// Find an exact event retry without disclosing its payload.
pub async fn by_client(
    tx: &mut PgConnection,
    tenant: TenantId,
    session: SessionId,
    client: &str,
) -> Result<Option<TaskStateHeader>> {
    sqlx::query_as!(TaskStateHeader,r#"select id,payload_hash,request_hash,sequence from session_snapshot_headers
       where tenant_id=$1 and session_id=$2 and client_event_id_digest=encode(sha256(convert_to($3,'UTF8')),'hex') and event_type='session.task_state'"#,
       tenant.as_uuid(),session.as_uuid(),client).fetch_optional(tx).await.map_err(storage)
}

/// Latest raw state head; source admission is an independent check.
pub async fn latest(
    tx: &mut PgConnection,
    tenant: TenantId,
    session: SessionId,
    before: Option<i64>,
) -> Result<Option<TaskStateHeader>> {
    latest_snapshot(tx, tenant, session, before, "session.task_state").await
}

/// Exact checkpoint head, without fallback past withdrawn newer content.
pub async fn latest_checkpoint(
    tx: &mut PgConnection,
    tenant: TenantId,
    session: SessionId,
    before: Option<i64>,
) -> Result<Option<TaskStateHeader>> {
    latest_snapshot(tx, tenant, session, before, "session.checkpoint").await
}

async fn latest_snapshot(
    tx: &mut PgConnection,
    tenant: TenantId,
    session: SessionId,
    before: Option<i64>,
    kind: &str,
) -> Result<Option<TaskStateHeader>> {
    sqlx::query_as!(TaskStateHeader,r#"select id,payload_hash,request_hash,sequence from session_snapshot_headers
       where tenant_id=$1 and session_id=$2 and event_type=$4 and ($3::bigint is null or sequence<$3)
       order by sequence desc limit 1"#,tenant.as_uuid(),session.as_uuid(),before,kind)
       .fetch_optional(tx).await.map_err(storage)
}

/// One admitted artifact body, without loading its whole import plan.
pub struct ArtifactText {
    /// Immutable artifact hash.
    pub content_hash: String,
    /// Already admitted source bytes.
    pub body_markdown: String,
}

/// Read exact artifact text only after independent project/source authorization.
pub async fn artifact_text(
    tx: &mut PgConnection,
    tenant: TenantId,
    artifact: ImportArtifactId,
) -> Result<Option<ArtifactText>> {
    sqlx::query_as!(ArtifactText,r#"select content_hash,body_markdown from import_artifacts
       where tenant_id=$1 and id=$2 and not content_erased and synveda_source_admissible(tenant_id,'import_artifact',id)"#,
       tenant.as_uuid(),artifact.as_uuid()).fetch_optional(tx).await.map_err(storage)
}

fn unavailable() -> Error {
    Error::NotFound {
        entity: "Task evidence".to_owned(),
    }
}

/// Narrow copied Knowledge using immutable classifications, without returning
/// a denied count or the identity of the narrower source.
pub async fn within_ceiling(
    tx: &mut PgConnection,
    tenant: TenantId,
    event: SessionEventId,
    ceiling: Option<Sensitivity>,
) -> Result<bool> {
    let Some(ceiling) = ceiling else {
        return Ok(true);
    };
    let rank = match ceiling {
        Sensitivity::Public => 0i32,
        Sensitivity::Internal => 1,
        Sensitivity::Confidential => 2,
        Sensitivity::Restricted => 3,
    };
    sqlx::query_scalar!(r#"select not exists(select 1 from (select knowledge_item_id,revision_id from knowledge_copy_dependencies where tenant_id=$1 and dependent_kind='session_event' and dependent_id=$2
       union select knowledge_item_id,revision_id from historical_knowledge_dependencies where tenant_id=$1 and dependent_kind='session_event' and dependent_id=$2) copy
       join knowledge_revision_headers header on header.tenant_id=$1 and header.revision_id=copy.revision_id
       where case header.sensitivity when 'public' then 0 when 'internal' then 1 when 'confidential' then 2 when 'restricted' then 3 else 4 end>$3)
       as "allowed!""#,tenant.as_uuid(),event.as_uuid(),rank).fetch_one(tx).await.map_err(storage)
}
fn storage(error: sqlx::Error) -> Error {
    Error::Storage {
        message: format!("task-state storage: {error}"),
    }
}
