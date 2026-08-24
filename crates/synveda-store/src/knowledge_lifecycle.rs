//! Durable VedaFlow effect projection and operation ledger for Knowledge
//! lifecycle commands (CPR-16, ADR-0081).
//!
//! This module stores no second approval or workflow state. The proposal row
//! in `synveda-vedaflow` is the change; [`StoredKnowledgeChange`] is the typed
//! payload its `apply` effect will execute after the approval matrix permits
//! it. Payload hashes remain after governed erasure, while the plaintext JSON
//! is cleared by the database's sole security-definer erasure primitive.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgConnection, PgExecutor};
use synveda_types::knowledge::{KnowledgeCommand, KnowledgeCommandKind};
use synveda_types::operation::DurableOperation;
use synveda_types::{
    DurableOperationId, Error, KnowledgeItemId, KnowledgeRevisionId, ProposalId, Result, TenantId,
};
use uuid::Uuid;

/// Counts typed Knowledge change and operation transitions.
pub const KNOWLEDGE_LIFECYCLE_ACTS_TOTAL: &str = "synveda_knowledge_lifecycle_acts_total";

/// One typed effect projection bound to a VedaFlow proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredKnowledgeChange {
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// The VedaFlow proposal/change id.
    pub proposal_id: ProposalId,
    /// Mutation family.
    pub command_kind: KnowledgeCommandKind,
    /// Existing aggregate ids this command may change.
    pub target_item_ids: Vec<KnowledgeItemId>,
    /// Erasable command payload; absent after authorised forget.
    pub payload: Option<KnowledgeCommand>,
    /// Canonical payload digest retained permanently.
    pub payload_hash: String,
    /// Stable result item, when the effect produced one.
    pub resulting_item_id: Option<KnowledgeItemId>,
    /// Exact result revision, when the effect produced one.
    pub resulting_revision_id: Option<KnowledgeRevisionId>,
    /// Durable operation, when the effect scheduled one.
    pub operation_id: Option<DurableOperationId>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Effect completion time.
    pub applied_at: Option<DateTime<Utc>>,
}

/// An open Knowledge change that names an aggregate about to be erased.
///
/// Forget closes these changes before their erasable payloads are cleared,
/// so the review queue never retains an open proposal whose effect can no
/// longer be inspected or executed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenKnowledgeChange {
    /// The VedaFlow proposal/change id.
    pub proposal_id: ProposalId,
    /// Mutation family, retained without content.
    pub command_kind: KnowledgeCommandKind,
    /// Digest of the reviewed payload.
    pub payload_hash: String,
}

struct OpenChangeRow {
    proposal_id: Uuid,
    command_kind: String,
    payload_hash: String,
}

struct ChangeRow {
    tenant_id: Uuid,
    proposal_id: Uuid,
    command_kind: String,
    target_item_ids: Vec<Uuid>,
    payload: Option<Value>,
    payload_hash: String,
    resulting_item_id: Option<Uuid>,
    resulting_revision_id: Option<Uuid>,
    operation_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    applied_at: Option<DateTime<Utc>>,
}

impl TryFrom<ChangeRow> for StoredKnowledgeChange {
    type Error = Error;

    fn try_from(row: ChangeRow) -> Result<Self> {
        Ok(Self {
            tenant_id: TenantId::from_uuid(row.tenant_id),
            proposal_id: ProposalId::from_uuid(row.proposal_id),
            command_kind: row.command_kind.parse().map_err(vocabulary)?,
            target_item_ids: row
                .target_item_ids
                .into_iter()
                .map(KnowledgeItemId::from_uuid)
                .collect(),
            payload: row
                .payload
                .map(serde_json::from_value)
                .transpose()
                .map_err(|err| Error::Storage {
                    message: format!("stored Knowledge change payload is invalid: {err}"),
                })?,
            payload_hash: row.payload_hash,
            resulting_item_id: row.resulting_item_id.map(KnowledgeItemId::from_uuid),
            resulting_revision_id: row
                .resulting_revision_id
                .map(KnowledgeRevisionId::from_uuid),
            operation_id: row.operation_id.map(DurableOperationId::from_uuid),
            created_at: row.created_at,
            applied_at: row.applied_at,
        })
    }
}

/// Inserts the typed payload for an already-opened Knowledge/apply proposal.
#[tracing::instrument(
    name = "store.knowledge_change.insert",
    skip_all,
    fields(tenant.id = %tenant_id, vedaflow.proposal = %proposal_id, knowledge.command = %command.kind()),
    err(Display)
)]
pub async fn insert_change(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    proposal_id: ProposalId,
    command: &KnowledgeCommand,
    payload_hash: &str,
) -> Result<StoredKnowledgeChange> {
    let payload = serde_json::to_value(command).map_err(|err| Error::Invalid {
        message: format!("encode Knowledge command: {err}"),
    })?;
    let targets: Vec<Uuid> = command
        .target_item_ids()
        .into_iter()
        .map(|id| id.as_uuid())
        .collect();
    let row = sqlx::query_as!(
        ChangeRow,
        r#"
        insert into knowledge_changes
            (tenant_id, proposal_id, command_kind, target_item_ids,
             payload, payload_hash)
        values ($1, $2, $3, $4, $5, $6)
        returning tenant_id, proposal_id, command_kind,
                  target_item_ids as "target_item_ids!: Vec<Uuid>",
                  payload, payload_hash, resulting_item_id,
                  resulting_revision_id, operation_id, created_at, applied_at
        "#,
        tenant_id.as_uuid(),
        proposal_id.as_uuid(),
        command.kind().as_str(),
        &targets,
        payload,
        payload_hash,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    act("opened", command.kind());
    row.try_into()
}

/// Reads one typed change. `None` is an unknown proposal in this tenant.
pub async fn read_change(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    proposal_id: ProposalId,
) -> Result<Option<StoredKnowledgeChange>> {
    sqlx::query_as!(
        ChangeRow,
        r#"
        select tenant_id, proposal_id, command_kind,
               target_item_ids as "target_item_ids!: Vec<Uuid>",
               payload, payload_hash, resulting_item_id,
               resulting_revision_id, operation_id, created_at, applied_at
        from knowledge_changes
        where tenant_id = $1 and proposal_id = $2
        "#,
        tenant_id.as_uuid(),
        proposal_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?
    .map(TryInto::try_into)
    .transpose()
}

/// Lists other open Knowledge changes that name `item_id`.
///
/// The current forget change is excluded explicitly. The target-id array is
/// content-free and survives erasure, making it the durable invalidation
/// index for pending edits, merges and supersessions.
pub async fn open_changes_for_item(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    item_id: KnowledgeItemId,
    except: ProposalId,
) -> Result<Vec<OpenKnowledgeChange>> {
    let rows = sqlx::query_as!(
        OpenChangeRow,
        r#"
        select change.proposal_id, change.command_kind, change.payload_hash
        from knowledge_changes change
        join vedaflow_proposals proposal
          on proposal.tenant_id = change.tenant_id
         and proposal.id = change.proposal_id
        where change.tenant_id = $1
          and change.target_item_ids @> array[$2]::uuid[]
          and change.proposal_id <> $3
          and proposal.state = 'open'
        order by change.created_at, change.proposal_id
        "#,
        tenant_id.as_uuid(),
        item_id.as_uuid(),
        except.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter()
        .map(|row| {
            Ok(OpenKnowledgeChange {
                proposal_id: ProposalId::from_uuid(row.proposal_id),
                command_kind: row.command_kind.parse().map_err(vocabulary)?,
                payload_hash: row.payload_hash,
            })
        })
        .collect()
}

/// Records an applied effect exactly once.
pub async fn finish_change(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    proposal_id: ProposalId,
    item_id: Option<KnowledgeItemId>,
    revision_id: Option<KnowledgeRevisionId>,
    operation_id: Option<DurableOperationId>,
) -> Result<bool> {
    let updated = sqlx::query!(
        r#"
        update knowledge_changes
        set resulting_item_id = $3,
            resulting_revision_id = $4,
            operation_id = $5,
            applied_at = now()
        where tenant_id = $1 and proposal_id = $2 and applied_at is null
        "#,
        tenant_id.as_uuid(),
        proposal_id.as_uuid(),
        item_id.map(|id| id.as_uuid()) as Option<Uuid>,
        revision_id.map(|id| id.as_uuid()) as Option<Uuid>,
        operation_id.map(|id| id.as_uuid()) as Option<Uuid>,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?
    .rows_affected();
    if updated == 1 {
        metrics::counter!(KNOWLEDGE_LIFECYCLE_ACTS_TOTAL, "act" => "applied").increment(1);
    }
    Ok(updated == 1)
}

struct OperationRow {
    tenant_id: Uuid,
    id: Uuid,
    proposal_id: Uuid,
    knowledge_item_id: Option<Uuid>,
    kind: String,
    state: String,
    input_hash: String,
    attempts: i32,
    lease_owner: Option<String>,
    lease_expires_at: Option<DateTime<Utc>>,
    last_error_code: Option<String>,
    result: Value,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
}

impl TryFrom<OperationRow> for DurableOperation {
    type Error = Error;

    fn try_from(row: OperationRow) -> Result<Self> {
        Ok(Self {
            id: DurableOperationId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            change_id: ProposalId::from_uuid(row.proposal_id),
            knowledge_item_id: row.knowledge_item_id.map(KnowledgeItemId::from_uuid),
            kind: row.kind.parse().map_err(vocabulary)?,
            input_hash: row.input_hash,
            state: row.state.parse().map_err(vocabulary)?,
            attempts: row.attempts,
            lease_owner: row.lease_owner,
            lease_expires_at: row.lease_expires_at,
            result: row.result,
            last_error_code: row.last_error_code,
            created_at: row.created_at,
            updated_at: row.updated_at,
            started_at: row.started_at,
            completed_at: row.completed_at,
        })
    }
}

/// Creates a pending erasure operation.
pub async fn create_erasure_operation(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    proposal_id: ProposalId,
    item_id: KnowledgeItemId,
    input_hash: &str,
) -> Result<DurableOperation> {
    let id = DurableOperationId::new();
    let row = sqlx::query_as!(
        OperationRow,
        r#"
        insert into durable_operations
            (tenant_id, id, kind, proposal_id, knowledge_item_id, input_hash)
        values ($1, $2, 'knowledge_erasure', $3, $4, $5)
        returning tenant_id, id, proposal_id, knowledge_item_id, kind, state,
                  input_hash, attempts, lease_owner, lease_expires_at,
                  last_error_code, result, created_at, updated_at, started_at,
                  completed_at
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
        proposal_id.as_uuid(),
        item_id.as_uuid(),
        input_hash,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    metrics::counter!(KNOWLEDGE_LIFECYCLE_ACTS_TOTAL, "act" => "operation_created").increment(1);
    row.try_into()
}

/// Claims a pending/failed operation for bounded execution.
pub async fn start_operation(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    operation_id: DurableOperationId,
    worker: &str,
    lease_seconds: i64,
) -> Result<Option<DurableOperation>> {
    if worker.trim().is_empty()
        || worker.chars().count() > 255
        || !(1..=3600).contains(&lease_seconds)
    {
        return Err(Error::Invalid {
            message: "an operation worker is non-blank and its lease is 1..=3600 seconds"
                .to_owned(),
        });
    }
    let row = sqlx::query_as!(
        OperationRow,
        r#"
        update durable_operations
        set state = 'running', attempts = attempts + 1,
            lease_owner = $3,
            lease_expires_at = now() + make_interval(secs => $4::double precision),
            started_at = coalesce(started_at, now()),
            completed_at = null, updated_at = now(), last_error_code = null
        where tenant_id = $1 and id = $2 and state in ('pending', 'failed')
        returning tenant_id, id, proposal_id, knowledge_item_id, kind, state,
                  input_hash, attempts, lease_owner, lease_expires_at,
                  last_error_code, result, created_at, updated_at, started_at,
                  completed_at
        "#,
        tenant_id.as_uuid(),
        operation_id.as_uuid(),
        worker,
        lease_seconds as f64,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Records that a retention/legal-hold hook refused erasure.
pub async fn block_operation(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    operation_id: DurableOperationId,
    code: &str,
) -> Result<bool> {
    let updated = sqlx::query!(
        r#"
        update durable_operations
        set state = 'blocked', completed_at = now(), updated_at = now(),
            lease_owner = null, lease_expires_at = null,
            last_error_code = $3, result = jsonb_build_object('blocked', true)
        where tenant_id = $1 and id = $2 and state in ('pending', 'running', 'failed')
        "#,
        tenant_id.as_uuid(),
        operation_id.as_uuid(),
        code,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?
    .rows_affected();
    Ok(updated == 1)
}

/// Reads one operation.
pub async fn read_operation(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    operation_id: DurableOperationId,
) -> Result<Option<DurableOperation>> {
    sqlx::query_as!(
        OperationRow,
        r#"
        select tenant_id, id, proposal_id, knowledge_item_id, kind, state,
               input_hash, attempts, lease_owner, lease_expires_at,
               last_error_code, result, created_at, updated_at, started_at,
               completed_at
        from durable_operations
        where tenant_id = $1 and id = $2
        "#,
        tenant_id.as_uuid(),
        operation_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?
    .map(TryInto::try_into)
    .transpose()
}

/// Reads the operation owned by one change, when that change scheduled one.
///
/// The operation row is authoritative while a retention hook is deciding the
/// outcome. In particular, a blocked erasure closes its proposal without
/// marking the effect projection applied, so deriving this only from
/// `knowledge_changes.operation_id` would lose the durable job identifier.
pub async fn operation_for_change(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    proposal_id: ProposalId,
) -> Result<Option<DurableOperation>> {
    sqlx::query_as!(
        OperationRow,
        r#"
        select tenant_id, id, proposal_id, knowledge_item_id, kind, state,
               input_hash, attempts, lease_owner, lease_expires_at,
               last_error_code, result, created_at, updated_at, started_at,
               completed_at
        from durable_operations
        where tenant_id = $1 and proposal_id = $2
        "#,
        tenant_id.as_uuid(),
        proposal_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?
    .map(TryInto::try_into)
    .transpose()
}

/// Executes the database's sole authorised plaintext-erasure primitive.
pub async fn erase_knowledge(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    item_id: KnowledgeItemId,
    proposal_id: ProposalId,
    operation_id: DurableOperationId,
    actor_hash: &str,
    reason_hash: &str,
) -> Result<()> {
    sqlx::query_scalar!(
        "select synveda_erase_knowledge($1, $2, $3, $4, $5, $6)",
        tenant_id.as_uuid(),
        item_id.as_uuid(),
        proposal_id.as_uuid(),
        operation_id.as_uuid(),
        actor_hash,
        reason_hash,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    metrics::counter!(KNOWLEDGE_LIFECYCLE_ACTS_TOTAL, "act" => "erased").increment(1);
    Ok(())
}

fn act(name: &'static str, command: KnowledgeCommandKind) {
    metrics::counter!(
        KNOWLEDGE_LIFECYCLE_ACTS_TOTAL,
        "act" => name,
        "command" => command.as_str()
    )
    .increment(1);
}

fn vocabulary(error: Error) -> Error {
    Error::Storage {
        message: format!("stored lifecycle vocabulary is invalid: {error}"),
    }
}

fn storage_error(error: sqlx::Error) -> Error {
    match error.as_database_error().and_then(|db| db.code()) {
        Some(code) if code == "42501" => crate::rls::backstop_error(error),
        _ => Error::Storage {
            message: error.to_string(),
        },
    }
}
