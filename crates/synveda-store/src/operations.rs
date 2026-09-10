//! Provider-neutral durable operation, attempt and outbox storage (CPR-45).
//!
//! PostgreSQL owns operation identity, tenant association, retries and the
//! business-effect fence. Queue adapters receive only an untrusted tenant
//! routing identifier, operation identifier and version; no adapter state is
//! authoritative here.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgExecutor;
use sqlx::postgres::PgConnection;
use synveda_types::operation::{
    DurableOperation, OperationKind, OperationState, SKILL_VALIDATION_OPERATION_VERSION,
};
use synveda_types::{
    DurableOperationId, Error, IdentityId, KnowledgeItemId, ProposalId, Result, ScopeId, SkillId,
    SkillTestRunId, SkillVersionId, TenantId,
};
use uuid::Uuid;

/// Closed-label operation transition counter.
pub const OPERATIONS_TOTAL: &str = "synveda_operations_total";
/// Closed-label execution-attempt counter.
pub const OPERATION_ATTEMPTS_TOTAL: &str = "synveda_operation_attempts_total";
/// Execution latency by the closed operation kind and outcome.
pub const OPERATION_ATTEMPT_SECONDS: &str = "synveda_operation_attempt_seconds";
/// Bounded Skill-validation dispatcher/sweep outcomes.
pub const OPERATION_SWEEPS_TOTAL: &str = "synveda_operation_sweeps_total";
/// Age of the oldest non-terminal Skill-validation operation.
pub const OPERATION_OLDEST_PENDING_AGE_SECONDS: &str =
    "synveda_operation_oldest_pending_age_seconds";
/// Age of the oldest non-terminal provider-neutral outbox row.
pub const OPERATION_OUTBOX_OLDEST_PENDING_AGE_SECONDS: &str =
    "synveda_operation_outbox_oldest_pending_age_seconds";

const MAX_SUBMISSION_FAILURES: i32 = 3;

/// Content-free queue ages for one tenant, combined without tenant labels by
/// the supervising worker.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QueueAges {
    /// Seconds since the oldest non-terminal operation was requested.
    pub operation_seconds: f64,
    /// Seconds since the oldest non-terminal outbox row was created.
    pub outbox_seconds: f64,
}

/// A Skill-validation operation plus the safe target/result fields needed by
/// the public API.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredSkillValidationOperation {
    /// Provider-neutral operation ledger row.
    pub operation: DurableOperation,
    /// Stable Skill aggregate containing the immutable target version.
    pub skill_id: SkillId,
    /// Governing scope derived from the immutable Skill aggregate.
    pub scope_id: ScopeId,
    /// Immutable result, present only after the effect commits.
    pub test_run_id: Option<SkillTestRunId>,
}

/// Result of an idempotent cancellation request, decided while holding the
/// authoritative operation row lock.
#[derive(Debug, Clone, PartialEq)]
pub struct CancellationResult {
    /// Current operation state.
    pub operation: StoredSkillValidationOperation,
    /// Whether this call performed the terminal transition.
    pub transitioned: bool,
}

/// One fenced outbox claim for an external dispatcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchClaim {
    /// Tenant selected by the dispatcher's ordinary active-tenant sweep.
    pub tenant_id: TenantId,
    /// Opaque Synveda operation identifier.
    pub operation_id: DurableOperationId,
    /// Closed operation payload version.
    pub operation_version: u16,
    /// Monotonic fence for this dispatch attempt.
    pub dispatch_attempt: i32,
    /// Process-unique lease owner.
    pub lease_owner: String,
}

/// Result of claiming one provider-neutral outbox delivery.
#[derive(Debug, Clone, PartialEq)]
pub enum DispatchSelection {
    /// The dispatcher owns a fresh fenced submission attempt.
    Claimed(DispatchClaim),
    /// Repeated delivery failure exhausted the bounded transport budget.
    DeadLettered(TerminalExecution),
}

/// Result of acknowledging a queue submission after its outbox claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchAcknowledgement {
    /// This caller advanced its exact claim to dispatched.
    Acknowledged,
    /// A fast consumer already completed the authoritative operation.
    AlreadyCompleted,
    /// The claim fence was lost while the operation remains non-terminal.
    LostFence,
}

/// Result of deferring a failed queue submission.
#[derive(Debug, Clone, PartialEq)]
pub enum DispatchDeferral {
    /// Another bounded submission attempt is scheduled.
    RetryScheduled,
    /// Queue submission was uncertain, but an execution lease proves that a
    /// consumer already received the task.
    ConsumerRunning,
    /// The transport retry budget ended the operation.
    DeadLettered(TerminalExecution),
    /// A fast consumer already completed the authoritative operation.
    AlreadyCompleted,
    /// The claim fence was lost while the operation remains non-terminal.
    LostFence,
}

/// One execution lease and its monotonic attempt fence.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionClaim {
    /// Claimed operation.
    pub operation: DurableOperation,
    /// Stable Skill aggregate containing the immutable target version.
    pub skill_id: SkillId,
    /// Governing scope derived from the immutable Skill aggregate.
    pub scope_id: ScopeId,
    /// Exact attempt number; equal to `operation.attempts`.
    pub attempt_number: i32,
}

/// Result of one bounded execution claim pass.
#[derive(Debug, Clone, PartialEq)]
pub enum ExecutionSelection {
    /// A worker owns a fresh fenced attempt.
    Claimed(Box<ExecutionClaim>),
    /// An expired operation had already consumed its retry budget and was
    /// terminalized under the row lock without running its business effect.
    DeadLettered(TerminalExecution),
}

/// Content-free terminalization evidence returned for audit in the caller's
/// still-open tenant transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalExecution {
    /// Opaque public operation id.
    pub operation_id: DurableOperationId,
    /// Governing scope derived from the immutable Skill target.
    pub scope_id: ScopeId,
    /// Number of actual execution claims consumed.
    pub attempts: i32,
}

/// Internal row shape shared with the existing Knowledge erasure seam.
pub(crate) struct OperationRow {
    pub(crate) tenant_id: Uuid,
    pub(crate) id: Uuid,
    pub(crate) operation_version: i16,
    pub(crate) proposal_id: Option<Uuid>,
    pub(crate) knowledge_item_id: Option<Uuid>,
    pub(crate) skill_version_id: Option<Uuid>,
    pub(crate) requested_by_identity_id: Option<Uuid>,
    pub(crate) authorized_at: DateTime<Utc>,
    pub(crate) kind: String,
    pub(crate) state: String,
    pub(crate) input_hash: String,
    pub(crate) progress_percent: i16,
    pub(crate) attempts: i32,
    pub(crate) next_attempt_at: Option<DateTime<Utc>>,
    pub(crate) cancel_requested_at: Option<DateTime<Utc>>,
    pub(crate) lease_owner: Option<String>,
    pub(crate) lease_expires_at: Option<DateTime<Utc>>,
    pub(crate) last_error_code: Option<String>,
    pub(crate) result: Value,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) started_at: Option<DateTime<Utc>>,
    pub(crate) completed_at: Option<DateTime<Utc>>,
}

impl TryFrom<OperationRow> for DurableOperation {
    type Error = Error;

    fn try_from(row: OperationRow) -> Result<Self> {
        Ok(Self {
            id: DurableOperationId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            operation_version: u16::try_from(row.operation_version).map_err(|_| {
                Error::Storage {
                    message: "stored operation version is outside the supported range".to_owned(),
                }
            })?,
            change_id: row.proposal_id.map(ProposalId::from_uuid),
            knowledge_item_id: row.knowledge_item_id.map(KnowledgeItemId::from_uuid),
            skill_version_id: row.skill_version_id.map(SkillVersionId::from_uuid),
            requested_by: row.requested_by_identity_id.map(IdentityId::from_uuid),
            authorized_at: row.authorized_at,
            kind: row.kind.parse().map_err(vocabulary)?,
            input_hash: row.input_hash,
            state: row.state.parse().map_err(vocabulary)?,
            progress_percent: u8::try_from(row.progress_percent).map_err(|_| Error::Storage {
                message: "stored operation progress is outside the supported range".to_owned(),
            })?,
            attempts: row.attempts,
            next_attempt_at: row.next_attempt_at,
            cancel_requested_at: row.cancel_requested_at,
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

struct SkillOperationRow {
    tenant_id: Uuid,
    id: Uuid,
    operation_version: i16,
    proposal_id: Option<Uuid>,
    knowledge_item_id: Option<Uuid>,
    skill_version_id: Option<Uuid>,
    requested_by_identity_id: Option<Uuid>,
    authorized_at: DateTime<Utc>,
    kind: String,
    state: String,
    input_hash: String,
    progress_percent: i16,
    attempts: i32,
    next_attempt_at: Option<DateTime<Utc>>,
    cancel_requested_at: Option<DateTime<Utc>>,
    lease_owner: Option<String>,
    lease_expires_at: Option<DateTime<Utc>>,
    last_error_code: Option<String>,
    result: Value,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    skill_id: Uuid,
    scope_id: Uuid,
    test_run_id: Option<Uuid>,
}

impl TryFrom<SkillOperationRow> for StoredSkillValidationOperation {
    type Error = Error;

    fn try_from(row: SkillOperationRow) -> Result<Self> {
        let operation: DurableOperation = OperationRow {
            tenant_id: row.tenant_id,
            id: row.id,
            operation_version: row.operation_version,
            proposal_id: row.proposal_id,
            knowledge_item_id: row.knowledge_item_id,
            skill_version_id: row.skill_version_id,
            requested_by_identity_id: row.requested_by_identity_id,
            authorized_at: row.authorized_at,
            kind: row.kind,
            state: row.state,
            input_hash: row.input_hash,
            progress_percent: row.progress_percent,
            attempts: row.attempts,
            next_attempt_at: row.next_attempt_at,
            cancel_requested_at: row.cancel_requested_at,
            lease_owner: row.lease_owner,
            lease_expires_at: row.lease_expires_at,
            last_error_code: row.last_error_code,
            result: row.result,
            created_at: row.created_at,
            updated_at: row.updated_at,
            started_at: row.started_at,
            completed_at: row.completed_at,
        }
        .try_into()?;
        if operation.kind != OperationKind::SkillValidation {
            return Err(Error::Storage {
                message: "a non-Skill operation reached the Skill operation reader".to_owned(),
            });
        }
        Ok(Self {
            operation,
            skill_id: SkillId::from_uuid(row.skill_id),
            scope_id: ScopeId::from_uuid(row.scope_id),
            test_run_id: row.test_run_id.map(SkillTestRunId::from_uuid),
        })
    }
}

/// Creates one pending Skill-validation operation and its payload-free outbox
/// row in the caller's transaction.
#[allow(clippy::too_many_arguments)]
pub async fn create_skill_validation_operation(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    skill_version_id: SkillVersionId,
    requested_by: IdentityId,
    input_hash: &str,
) -> Result<DurableOperation> {
    let id = DurableOperationId::new();
    let version =
        i16::try_from(SKILL_VALIDATION_OPERATION_VERSION).map_err(|_| Error::Invalid {
            message: "Skill validation operation version is outside the database range".to_owned(),
        })?;
    let row = sqlx::query_as!(
        OperationRow,
        r#"
        insert into durable_operations
            (tenant_id, id, kind, operation_version, skill_version_id,
             requested_by_identity_id, input_hash)
        values ($1, $2, 'skill_validation', $3, $4, $5, $6)
        returning tenant_id, id, operation_version, proposal_id, knowledge_item_id,
                  skill_version_id, requested_by_identity_id, authorized_at,
                  kind, state, input_hash, progress_percent, attempts, next_attempt_at,
                  cancel_requested_at, lease_owner, lease_expires_at, last_error_code,
                  result, created_at, updated_at, started_at, completed_at
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
        version,
        skill_version_id.as_uuid(),
        requested_by.as_uuid(),
        input_hash,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    sqlx::query!(
        r#"insert into operation_outbox
               (tenant_id, operation_id, operation_version)
           values ($1, $2, $3)"#,
        tenant_id.as_uuid(),
        id.as_uuid(),
        version,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;
    metrics::counter!(OPERATIONS_TOTAL, "kind" => "skill_validation", "state" => "pending")
        .increment(1);
    row.try_into()
}

/// Reads content-free queue ages inside one ordinary tenant transaction.
pub async fn skill_validation_queue_ages(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
) -> Result<QueueAges> {
    let row = sqlx::query!(
        r#"
        select
          coalesce(extract(epoch from statement_timestamp() - (
            select min(created_at)
            from durable_operations
            where tenant_id = $1 and kind = 'skill_validation'
              and state in ('pending', 'running', 'failed')
          )), 0)::float8 as "operation_seconds!",
          coalesce(extract(epoch from statement_timestamp() - (
            select min(created_at)
            from operation_outbox
            where tenant_id = $1 and state <> 'completed'
          )), 0)::float8 as "outbox_seconds!"
        "#,
        tenant_id.as_uuid(),
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    Ok(QueueAges {
        operation_seconds: row.operation_seconds.max(0.0),
        outbox_seconds: row.outbox_seconds.max(0.0),
    })
}

/// Reads one Skill-validation operation and its safe public target/result.
pub async fn read_skill_validation_operation(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    operation_id: DurableOperationId,
) -> Result<Option<StoredSkillValidationOperation>> {
    sqlx::query_as!(
        SkillOperationRow,
        r#"
        select operation.tenant_id, operation.id, operation.operation_version,
               operation.proposal_id, operation.knowledge_item_id,
               operation.skill_version_id, operation.requested_by_identity_id,
               operation.authorized_at, operation.kind, operation.state,
               operation.input_hash, operation.progress_percent, operation.attempts,
               operation.next_attempt_at, operation.cancel_requested_at,
               operation.lease_owner, operation.lease_expires_at,
               operation.last_error_code, operation.result, operation.created_at,
               operation.updated_at, operation.started_at, operation.completed_at,
               version.skill_id as "skill_id!",
               skill.governing_scope_id as "scope_id!", run.id as "test_run_id?"
        from durable_operations operation
        join skill_versions version
          on version.tenant_id = operation.tenant_id
         and version.id = operation.skill_version_id
        join skills skill
          on skill.tenant_id = version.tenant_id
         and skill.id = version.skill_id
        left join skill_test_runs run
          on run.tenant_id = operation.tenant_id
         and run.operation_id = operation.id
        where operation.tenant_id = $1 and operation.id = $2
          and operation.kind = 'skill_validation'
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

/// Pages Skill-validation operations at one exact governing scope, newest
/// UUIDv7 first.
pub async fn list_skill_validation_operations(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    scope_id: ScopeId,
    before: Option<DurableOperationId>,
    limit: i64,
) -> Result<Vec<StoredSkillValidationOperation>> {
    let rows = sqlx::query_as!(
        SkillOperationRow,
        r#"
        select operation.tenant_id, operation.id, operation.operation_version,
               operation.proposal_id, operation.knowledge_item_id,
               operation.skill_version_id, operation.requested_by_identity_id,
               operation.authorized_at, operation.kind, operation.state,
               operation.input_hash, operation.progress_percent, operation.attempts,
               operation.next_attempt_at, operation.cancel_requested_at,
               operation.lease_owner, operation.lease_expires_at,
               operation.last_error_code, operation.result, operation.created_at,
               operation.updated_at, operation.started_at, operation.completed_at,
               version.skill_id as "skill_id!",
               skill.governing_scope_id as "scope_id!", run.id as "test_run_id?"
        from durable_operations operation
        join skill_versions version
          on version.tenant_id = operation.tenant_id
         and version.id = operation.skill_version_id
        join skills skill
          on skill.tenant_id = version.tenant_id
         and skill.id = version.skill_id
        left join skill_test_runs run
          on run.tenant_id = operation.tenant_id
         and run.operation_id = operation.id
        where operation.tenant_id = $1 and skill.governing_scope_id = $2
          and operation.kind = 'skill_validation'
          and ($3::uuid is null or operation.id < $3)
        order by operation.id desc
        limit $4
        "#,
        tenant_id.as_uuid(),
        scope_id.as_uuid(),
        before.map(|id| id.as_uuid()),
        limit,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Records an idempotent cancellation request. Pending/retry-wait operations
/// become terminal immediately. A running attempt is closed under the same
/// row lock, so either cancellation or its effect wins the transaction fence.
pub async fn request_cancellation(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    operation_id: DurableOperationId,
) -> Result<Option<CancellationResult>> {
    let current = sqlx::query!(
        r#"
        select state, attempts, lease_owner
        from durable_operations
        where tenant_id = $1 and id = $2 and kind = 'skill_validation'
        for update
        "#,
        tenant_id.as_uuid(),
        operation_id.as_uuid(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    let Some(current) = current else {
        return Ok(None);
    };
    let state: OperationState = current.state.parse().map_err(vocabulary)?;
    if state.is_terminal() {
        return Ok(
            read_skill_validation_operation(&mut *conn, tenant_id, operation_id)
                .await?
                .map(|operation| CancellationResult {
                    operation,
                    transitioned: false,
                }),
        );
    }
    if state == OperationState::Running {
        let worker_id = current.lease_owner.ok_or_else(|| Error::Storage {
            message: "a running Skill-validation operation has no lease owner".to_owned(),
        })?;
        let attempt = sqlx::query!(
            r#"
            update operation_attempts
            set state = 'cancelled', heartbeat_at = now(), completed_at = now()
            where tenant_id = $1 and operation_id = $2 and attempt_number = $3
              and worker_id = $4 and state = 'running'
            "#,
            tenant_id.as_uuid(),
            operation_id.as_uuid(),
            current.attempts,
            worker_id,
        )
        .execute(&mut *conn)
        .await
        .map_err(storage_error)?
        .rows_affected();
        if attempt != 1 {
            return Err(Error::Storage {
                message: "a running Skill-validation operation has no live fenced attempt"
                    .to_owned(),
            });
        }
    }
    let changed = sqlx::query!(
        r#"
        update durable_operations
        set cancel_requested_at = coalesce(cancel_requested_at, now()),
            state = 'cancelled', completed_at = now(), next_attempt_at = null,
            lease_owner = null, lease_expires_at = null, last_error_code = null,
            result = '{}'::jsonb, updated_at = now()
        where tenant_id = $1 and id = $2 and kind = 'skill_validation'
          and state in ('pending', 'running', 'failed')
        "#,
        tenant_id.as_uuid(),
        operation_id.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?
    .rows_affected();
    if changed != 1 {
        return Err(Error::Storage {
            message: "the locked Skill-validation operation changed unexpectedly".to_owned(),
        });
    }
    complete_outbox(conn, tenant_id, operation_id).await?;
    metrics::counter!(OPERATIONS_TOTAL, "kind" => "skill_validation", "state" => "cancelled")
        .increment(1);
    Ok(
        read_skill_validation_operation(&mut *conn, tenant_id, operation_id)
            .await?
            .map(|operation| CancellationResult {
                operation,
                transitioned: true,
            }),
    )
}

/// Claims one due outbox row for an external queue adapter.
pub async fn claim_dispatch(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    lease_owner: &str,
    lease_seconds: i64,
    delivery_timeout_seconds: i64,
) -> Result<Option<DispatchSelection>> {
    validate_worker_lease(lease_owner, lease_seconds)?;
    if !(1..=86_400).contains(&delivery_timeout_seconds) {
        return Err(Error::Invalid {
            message: "operation delivery timeout must be between 1 and 86400 seconds".to_owned(),
        });
    }
    let max_submission_failures = MAX_SUBMISSION_FAILURES;
    if let Some(terminal) =
        terminalize_exhausted_dispatch(conn, tenant_id, max_submission_failures).await?
    {
        return Ok(Some(DispatchSelection::DeadLettered(terminal)));
    }
    let row = sqlx::query!(
        r#"
        with candidate as (
            select outbox.tenant_id, outbox.operation_id, outbox.operation_version
            from durable_operations operation
            join operation_outbox outbox
              on outbox.tenant_id = operation.tenant_id
             and outbox.operation_id = operation.id
             and outbox.operation_version = operation.operation_version
            where operation.tenant_id = $1
              and operation.kind = 'skill_validation'
              and (
                  (operation.state in ('pending', 'failed')
                      and operation.next_attempt_at <= statement_timestamp())
                  or (operation.state = 'running'
                      and operation.lease_expires_at <= statement_timestamp())
              )
              and (
                  (outbox.state = 'pending'
                      and outbox.next_dispatch_at <= statement_timestamp())
                  or (outbox.state = 'claimed'
                      and outbox.lease_expires_at <= statement_timestamp())
                  or (outbox.state = 'dispatched'
                      and outbox.dispatched_at <= statement_timestamp()
                          - make_interval(secs => $4::double precision))
              )
            order by coalesce(outbox.next_dispatch_at, outbox.lease_expires_at,
                              outbox.dispatched_at), outbox.operation_id
            for update of operation, outbox skip locked
            limit 1
        )
        update operation_outbox outbox
        set state = 'claimed', dispatch_attempts = outbox.dispatch_attempts + 1,
            lease_owner = $2,
            lease_expires_at = statement_timestamp()
                + make_interval(secs => $3::double precision),
            next_dispatch_at = null, last_error_code = null, updated_at = now()
        from candidate
        where outbox.tenant_id = candidate.tenant_id
          and outbox.operation_id = candidate.operation_id
          and outbox.operation_version = candidate.operation_version
          and (
              (outbox.state = 'pending'
                  and outbox.next_dispatch_at <= statement_timestamp())
              or (outbox.state = 'claimed'
                  and outbox.lease_expires_at <= statement_timestamp())
              or (outbox.state = 'dispatched'
                  and outbox.dispatched_at <= statement_timestamp()
                      - make_interval(secs => $4::double precision))
          )
        returning outbox.operation_id, outbox.operation_version,
                  outbox.dispatch_attempts, outbox.lease_owner as "lease_owner!"
        "#,
        tenant_id.as_uuid(),
        lease_owner,
        lease_seconds as f64,
        delivery_timeout_seconds as f64,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    row.map(|row| {
        let operation_version =
            u16::try_from(row.operation_version).map_err(|_| Error::Storage {
                message: "stored outbox version is outside the supported range".to_owned(),
            })?;
        Ok(DispatchSelection::Claimed(DispatchClaim {
            tenant_id,
            operation_id: DurableOperationId::from_uuid(row.operation_id),
            operation_version,
            dispatch_attempt: row.dispatch_attempts,
            lease_owner: row.lease_owner,
        }))
    })
    .transpose()
}

async fn terminalize_exhausted_dispatch(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    max_submission_failures: i32,
) -> Result<Option<TerminalExecution>> {
    let row = sqlx::query!(
        r#"
        select operation.id, operation.operation_version, operation.state, operation.attempts,
               operation.lease_owner, skill.governing_scope_id
        from durable_operations operation
        join operation_outbox outbox
          on outbox.tenant_id = operation.tenant_id
         and outbox.operation_id = operation.id
         and outbox.operation_version = operation.operation_version
        join skill_versions version
          on version.tenant_id = operation.tenant_id
         and version.id = operation.skill_version_id
        join skills skill
          on skill.tenant_id = version.tenant_id and skill.id = version.skill_id
        where operation.tenant_id = $1 and operation.kind = 'skill_validation'
          and operation.cancel_requested_at is null
          and outbox.submission_failures >= $2
          and (
              (operation.state in ('pending', 'failed')
                  and operation.next_attempt_at <= statement_timestamp())
              or (operation.state = 'running'
                  and operation.lease_expires_at <= statement_timestamp())
          )
          and (
              (outbox.state = 'pending'
                  and outbox.next_dispatch_at <= statement_timestamp())
              or (outbox.state = 'claimed'
                  and outbox.lease_expires_at <= statement_timestamp())
          )
        order by outbox.next_dispatch_at nulls first, outbox.operation_id
        for update of operation, outbox skip locked
        limit 1
        "#,
        tenant_id.as_uuid(),
        max_submission_failures,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    close_running_attempt_for_terminal(
        conn,
        tenant_id,
        DurableOperationId::from_uuid(row.id),
        &row.state,
        row.attempts,
        row.lease_owner.as_deref(),
        "dispatch_retry_exhausted",
    )
    .await?;
    let changed = sqlx::query!(
        r#"
        update durable_operations
        set state = 'dead_lettered', completed_at = now(), lease_owner = null,
            lease_expires_at = null, next_attempt_at = null,
            last_error_code = 'dispatch_retry_exhausted', result = '{}'::jsonb,
            updated_at = now()
        where tenant_id = $1 and id = $2 and kind = 'skill_validation'
          and state = $3 and attempts = $4
          and lease_owner is not distinct from $5
        "#,
        tenant_id.as_uuid(),
        row.id,
        row.state,
        row.attempts,
        row.lease_owner,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?
    .rows_affected();
    if changed != 1 {
        return Err(lost_execution());
    }
    let operation_id = DurableOperationId::from_uuid(row.id);
    complete_outbox(conn, tenant_id, operation_id).await?;
    metrics::counter!(OPERATIONS_TOTAL, "kind" => "skill_validation", "state" => "dead_lettered")
        .increment(1);
    Ok(Some(TerminalExecution {
        operation_id,
        scope_id: ScopeId::from_uuid(row.governing_scope_id),
        attempts: row.attempts,
    }))
}

/// Acknowledges that the durable queue owns the submitted task.
pub async fn acknowledge_dispatch(
    conn: &mut PgConnection,
    claim: &DispatchClaim,
) -> Result<DispatchAcknowledgement> {
    let operation = sqlx::query!(
        r#"
        select state, next_attempt_at, last_error_code
        from durable_operations
        where tenant_id = $1 and id = $2 and kind = 'skill_validation'
          and operation_version = $3
        for update
        "#,
        claim.tenant_id.as_uuid(),
        claim.operation_id.as_uuid(),
        i16::try_from(claim.operation_version).map_err(|_| Error::Invalid {
            message: "dispatch operation version is outside the database range".to_owned(),
        })?,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    let Some(operation) = operation else {
        return Ok(DispatchAcknowledgement::LostFence);
    };
    let operation_state: OperationState = operation.state.parse().map_err(vocabulary)?;
    if operation_state.is_terminal() {
        return Ok(DispatchAcknowledgement::AlreadyCompleted);
    }
    let changed = sqlx::query!(
        r#"
        update operation_outbox
        set state = 'dispatched', lease_owner = null, lease_expires_at = null,
            next_dispatch_at = null,
            dispatched_at = now(), updated_at = now(), last_error_code = null,
            submission_failures = 0
        where tenant_id = $1 and operation_id = $2 and state = 'claimed'
          and operation_version = $3 and lease_owner = $4 and dispatch_attempts = $5
        "#,
        claim.tenant_id.as_uuid(),
        claim.operation_id.as_uuid(),
        i16::try_from(claim.operation_version).map_err(|_| Error::Invalid {
            message: "dispatch operation version is outside the database range".to_owned(),
        })?,
        &claim.lease_owner,
        claim.dispatch_attempt,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?
    .rows_affected();
    if changed != 1 {
        return Ok(DispatchAcknowledgement::LostFence);
    }
    if operation_state == OperationState::Failed {
        let changed = sqlx::query!(
            r#"
            update operation_outbox
            set state = 'pending', next_dispatch_at = $3,
                last_error_code = $4, updated_at = now()
            where tenant_id = $1 and operation_id = $2 and state = 'dispatched'
            "#,
            claim.tenant_id.as_uuid(),
            claim.operation_id.as_uuid(),
            operation.next_attempt_at,
            operation.last_error_code,
        )
        .execute(&mut *conn)
        .await
        .map_err(storage_error)?
        .rows_affected();
        if changed != 1 {
            return Err(Error::Storage {
                message: "a failed fast consumer could not reschedule its outbox row".to_owned(),
            });
        }
    }
    Ok(DispatchAcknowledgement::Acknowledged)
}

/// Restores an acknowledged delivery to the provider-neutral outbox after a
/// consumer cannot conclusively finish it.
///
/// A live product lease is not stolen: its outbox becomes due only when that
/// lease expires. Failed work retains its authoritative retry deadline, while
/// pending work can be handed off again immediately. Terminal operations close
/// the outbox instead.
pub async fn recover_interrupted_delivery(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    operation_id: DurableOperationId,
    safe_error_code: &str,
) -> Result<Option<OperationState>> {
    validate_error_code(safe_error_code)?;
    let current = sqlx::query_scalar!(
        r#"select state from durable_operations
           where tenant_id = $1 and id = $2 and kind = 'skill_validation'
           for update"#,
        tenant_id.as_uuid(),
        operation_id.as_uuid(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    let Some(current) = current else {
        return Ok(None);
    };
    let state: OperationState = current.parse().map_err(vocabulary)?;
    if state.is_terminal() {
        let outbox_state = sqlx::query_scalar!(
            r#"select state from operation_outbox
               where tenant_id = $1 and operation_id = $2
               for update"#,
            tenant_id.as_uuid(),
            operation_id.as_uuid(),
        )
        .fetch_optional(&mut *conn)
        .await
        .map_err(storage_error)?;
        match outbox_state.as_deref() {
            Some("completed") => {}
            Some(_) => complete_outbox(conn, tenant_id, operation_id).await?,
            None => {
                return Err(Error::Storage {
                    message: "a terminal Skill-validation operation has no outbox row".to_owned(),
                });
            }
        }
        return Ok(Some(state));
    }
    // A fast consumer can observe the task before the dispatcher records its
    // acknowledgement. Converge that exact in-flight submission first so the
    // ordinary dispatched -> pending recovery below also fences a delayed
    // acknowledgement. Both transitions commit in this tenant transaction.
    sqlx::query!(
        r#"
        update operation_outbox
        set state = 'dispatched', lease_owner = null, lease_expires_at = null,
            next_dispatch_at = null, dispatched_at = coalesce(dispatched_at, now()),
            last_error_code = null, submission_failures = 0, updated_at = now()
        where tenant_id = $1 and operation_id = $2 and state = 'claimed'
        "#,
        tenant_id.as_uuid(),
        operation_id.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;
    let changed = sqlx::query!(
        r#"
        update operation_outbox outbox
        set state = 'pending', lease_owner = null, lease_expires_at = null,
            next_dispatch_at = case operation.state
                when 'pending' then statement_timestamp()
                when 'failed' then greatest(operation.next_attempt_at, statement_timestamp())
                when 'running' then greatest(operation.lease_expires_at, statement_timestamp())
            end,
            last_error_code = coalesce(operation.last_error_code, $3),
            updated_at = now()
        from durable_operations operation
        where outbox.tenant_id = $1 and outbox.operation_id = $2
          and outbox.state = 'dispatched'
          and operation.tenant_id = outbox.tenant_id
          and operation.id = outbox.operation_id
          and operation.kind = 'skill_validation'
          and operation.state in ('pending', 'failed', 'running')
        "#,
        tenant_id.as_uuid(),
        operation_id.as_uuid(),
        safe_error_code,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?
    .rows_affected();
    if changed == 0 {
        let outbox_state = sqlx::query_scalar!(
            r#"select state from operation_outbox
               where tenant_id = $1 and operation_id = $2
               for update"#,
            tenant_id.as_uuid(),
            operation_id.as_uuid(),
        )
        .fetch_optional(&mut *conn)
        .await
        .map_err(storage_error)?;
        if !matches!(
            outbox_state.as_deref(),
            Some("pending" | "claimed" | "completed")
        ) {
            return Err(Error::Storage {
                message: "an interrupted Skill-validation delivery could not be recovered"
                    .to_owned(),
            });
        }
    }
    metrics::counter!(OPERATION_SWEEPS_TOTAL, "outcome" => "delivery_recovered").increment(1);
    Ok(Some(state))
}

/// Releases a failed external submission for a bounded retry.
pub async fn defer_dispatch(
    conn: &mut PgConnection,
    claim: &DispatchClaim,
    retry_seconds: i64,
    safe_error_code: &str,
) -> Result<DispatchDeferral> {
    validate_retry(retry_seconds, safe_error_code)?;
    let current = sqlx::query!(
        r#"
        select operation.state, operation.attempts, operation.lease_owner,
               coalesce(operation.lease_expires_at > statement_timestamp(), false) as "lease_live!",
               outbox.submission_failures, skill.governing_scope_id
        from durable_operations operation
        join operation_outbox outbox
          on outbox.tenant_id = operation.tenant_id
         and outbox.operation_id = operation.id
         and outbox.operation_version = operation.operation_version
        join skill_versions version
          on version.tenant_id = operation.tenant_id
         and version.id = operation.skill_version_id
        join skills skill
          on skill.tenant_id = version.tenant_id and skill.id = version.skill_id
        where operation.tenant_id = $1 and operation.id = $2
          and operation.kind = 'skill_validation'
          and outbox.operation_version = $3 and outbox.state = 'claimed'
          and outbox.lease_owner = $4 and outbox.dispatch_attempts = $5
        for update of operation, outbox
        "#,
        claim.tenant_id.as_uuid(),
        claim.operation_id.as_uuid(),
        i16::try_from(claim.operation_version).map_err(|_| Error::Invalid {
            message: "dispatch operation version is outside the database range".to_owned(),
        })?,
        &claim.lease_owner,
        claim.dispatch_attempt,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    let Some(current) = current else {
        return if dispatch_is_completed(conn, claim.tenant_id, claim.operation_id).await? {
            Ok(DispatchDeferral::AlreadyCompleted)
        } else {
            Ok(DispatchDeferral::LostFence)
        };
    };
    let operation_state: OperationState = current.state.parse().map_err(vocabulary)?;
    if operation_state.is_terminal() {
        return Err(Error::Storage {
            message: "a terminal Skill-validation operation retained a live outbox claim"
                .to_owned(),
        });
    }
    if current.submission_failures + 1 >= MAX_SUBMISSION_FAILURES
        && !(operation_state == OperationState::Running && current.lease_live)
    {
        close_running_attempt_for_terminal(
            conn,
            claim.tenant_id,
            claim.operation_id,
            &current.state,
            current.attempts,
            current.lease_owner.as_deref(),
            "dispatch_retry_exhausted",
        )
        .await?;
        let operation = sqlx::query!(
            r#"
            update durable_operations
            set state = 'dead_lettered', completed_at = now(), lease_owner = null,
                lease_expires_at = null, next_attempt_at = null,
                last_error_code = 'dispatch_retry_exhausted', result = '{}'::jsonb,
                updated_at = now()
            where tenant_id = $1 and id = $2 and kind = 'skill_validation'
              and state = $3 and attempts = $4
              and lease_owner is not distinct from $5
            "#,
            claim.tenant_id.as_uuid(),
            claim.operation_id.as_uuid(),
            current.state,
            current.attempts,
            current.lease_owner,
        )
        .execute(&mut *conn)
        .await
        .map_err(storage_error)?
        .rows_affected();
        if operation != 1 {
            return Err(lost_execution());
        }
        complete_outbox(conn, claim.tenant_id, claim.operation_id).await?;
        metrics::counter!(OPERATIONS_TOTAL, "kind" => "skill_validation", "state" => "dead_lettered")
            .increment(1);
        return Ok(DispatchDeferral::DeadLettered(TerminalExecution {
            operation_id: claim.operation_id,
            scope_id: ScopeId::from_uuid(current.governing_scope_id),
            attempts: current.attempts,
        }));
    }
    let live_operation_running = operation_state == OperationState::Running && current.lease_live;
    let changed = sqlx::query!(
        r#"
        update operation_outbox
        set state = case when $8 then 'dispatched' else 'pending' end,
            lease_owner = null, lease_expires_at = null,
            next_dispatch_at = case when $8 then null else
                statement_timestamp() + make_interval(secs => $6::double precision)
            end,
            last_error_code = case when $8 then null else $7 end,
            submission_failures = case when $8 then 0 else submission_failures + 1 end,
            dispatched_at = case when $8 then coalesce(dispatched_at, now())
                                 else dispatched_at end,
            updated_at = now()
        where tenant_id = $1 and operation_id = $2 and operation_version = $3
          and state = 'claimed' and lease_owner = $4 and dispatch_attempts = $5
        "#,
        claim.tenant_id.as_uuid(),
        claim.operation_id.as_uuid(),
        i16::try_from(claim.operation_version).map_err(|_| Error::Invalid {
            message: "dispatch operation version is outside the database range".to_owned(),
        })?,
        &claim.lease_owner,
        claim.dispatch_attempt,
        retry_seconds as f64,
        safe_error_code,
        live_operation_running,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?
    .rows_affected();
    if changed != 1 {
        return if dispatch_is_completed(conn, claim.tenant_id, claim.operation_id).await? {
            Ok(DispatchDeferral::AlreadyCompleted)
        } else {
            Ok(DispatchDeferral::LostFence)
        };
    }
    if live_operation_running {
        Ok(DispatchDeferral::ConsumerRunning)
    } else {
        Ok(DispatchDeferral::RetryScheduled)
    }
}

/// Claims one due Skill validation, closing an orphaned prior attempt in the
/// same transaction before inserting the next fence.
pub async fn claim_skill_validation_execution(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    worker_id: &str,
    lease_seconds: i64,
    operation_id: Option<DurableOperationId>,
    max_attempts: i32,
) -> Result<Option<ExecutionSelection>> {
    validate_worker_lease(worker_id, lease_seconds)?;
    if !(1..=100).contains(&max_attempts) {
        return Err(Error::Invalid {
            message: "operation max attempts must be between 1 and 100".to_owned(),
        });
    }
    if let Some(terminal) =
        terminalize_exhausted_execution(conn, tenant_id, operation_id, max_attempts).await?
    {
        return Ok(Some(ExecutionSelection::DeadLettered(terminal)));
    }
    let row = sqlx::query!(
        r#"
        with candidate as (
            select candidate.id, candidate.state as previous_state
            from durable_operations candidate
            where candidate.tenant_id = $1 and candidate.kind = 'skill_validation'
              and candidate.cancel_requested_at is null
              and ($4::uuid is null or candidate.id = $4)
              and candidate.attempts < $5
              and (
                  (candidate.state in ('pending', 'failed')
                      and candidate.next_attempt_at <= statement_timestamp())
                  or (candidate.state = 'running'
                      and candidate.lease_expires_at <= statement_timestamp())
              )
            order by candidate.next_attempt_at nulls first, candidate.id
            for update skip locked
            limit 1
        )
        update durable_operations operation
        set state = 'running', attempts = operation.attempts + 1,
            lease_owner = $2,
            lease_expires_at = statement_timestamp()
                + make_interval(secs => $3::double precision),
            started_at = coalesce(operation.started_at, now()), completed_at = null,
            next_attempt_at = null, last_error_code = null,
            progress_percent = operation.progress_percent, updated_at = now()
        from candidate
        where operation.tenant_id = $1 and operation.id = candidate.id
        returning operation.tenant_id, operation.id, operation.operation_version,
                  operation.proposal_id, operation.knowledge_item_id,
                  operation.skill_version_id, operation.requested_by_identity_id,
                  operation.authorized_at, operation.kind, operation.state,
                  operation.input_hash, operation.progress_percent, operation.attempts,
                  operation.next_attempt_at, operation.cancel_requested_at,
                  operation.lease_owner, operation.lease_expires_at,
                  operation.last_error_code, operation.result, operation.created_at,
                  operation.updated_at, operation.started_at, operation.completed_at,
                  operation.attempts as attempt_number,
                  candidate.previous_state as "previous_state!"
        "#,
        tenant_id.as_uuid(),
        worker_id,
        lease_seconds as f64,
        operation_id.map(|id| id.as_uuid()),
        max_attempts,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let attempt_number = row.attempt_number;
    if row.previous_state == "running" {
        let closed = sqlx::query!(
            r#"
            update operation_attempts
            set state = 'retry_scheduled', completed_at = now(),
                heartbeat_at = now(), safe_error_code = 'lease_expired'
            where tenant_id = $1 and operation_id = $2
              and attempt_number = $3 and state = 'running'
            "#,
            tenant_id.as_uuid(),
            row.id,
            attempt_number - 1,
        )
        .execute(&mut *conn)
        .await
        .map_err(storage_error)?
        .rows_affected();
        if closed != 1 {
            return Err(Error::Storage {
                message: "an expired Skill-validation operation has no live fenced attempt"
                    .to_owned(),
            });
        }
    }
    sqlx::query!(
        r#"
        insert into operation_attempts
            (tenant_id, operation_id, attempt_number, worker_id)
        values ($1, $2, $3, $4)
        "#,
        tenant_id.as_uuid(),
        row.id,
        attempt_number,
        worker_id,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;
    let target = sqlx::query!(
        r#"
        select version.skill_id, skill.governing_scope_id
        from skill_versions version
        join skills skill
          on skill.tenant_id = version.tenant_id and skill.id = version.skill_id
        where version.tenant_id = $1 and version.id = $2
        "#,
        tenant_id.as_uuid(),
        row.skill_version_id,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    let operation = OperationRow {
        tenant_id: row.tenant_id,
        id: row.id,
        operation_version: row.operation_version,
        proposal_id: row.proposal_id,
        knowledge_item_id: row.knowledge_item_id,
        skill_version_id: row.skill_version_id,
        requested_by_identity_id: row.requested_by_identity_id,
        authorized_at: row.authorized_at,
        kind: row.kind,
        state: row.state,
        input_hash: row.input_hash,
        progress_percent: row.progress_percent,
        attempts: row.attempts,
        next_attempt_at: row.next_attempt_at,
        cancel_requested_at: row.cancel_requested_at,
        lease_owner: row.lease_owner,
        lease_expires_at: row.lease_expires_at,
        last_error_code: row.last_error_code,
        result: row.result,
        created_at: row.created_at,
        updated_at: row.updated_at,
        started_at: row.started_at,
        completed_at: row.completed_at,
    }
    .try_into()?;
    Ok(Some(ExecutionSelection::Claimed(Box::new(
        ExecutionClaim {
            operation,
            skill_id: SkillId::from_uuid(target.skill_id),
            scope_id: ScopeId::from_uuid(target.governing_scope_id),
            attempt_number,
        },
    ))))
}

async fn terminalize_exhausted_execution(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    operation_id: Option<DurableOperationId>,
    max_attempts: i32,
) -> Result<Option<TerminalExecution>> {
    let row = sqlx::query!(
        r#"
        select operation.id, operation.attempts, operation.lease_owner,
               skill.governing_scope_id
        from durable_operations operation
        join skill_versions version
          on version.tenant_id = operation.tenant_id
         and version.id = operation.skill_version_id
        join skills skill
          on skill.tenant_id = version.tenant_id and skill.id = version.skill_id
        where operation.tenant_id = $1 and operation.kind = 'skill_validation'
          and operation.state = 'running' and operation.cancel_requested_at is null
          and operation.lease_expires_at <= statement_timestamp()
          and operation.attempts >= $2
          and ($3::uuid is null or operation.id = $3)
        order by operation.lease_expires_at, operation.id
        for update of operation skip locked
        limit 1
        "#,
        tenant_id.as_uuid(),
        max_attempts,
        operation_id.map(|id| id.as_uuid()),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let worker_id = row.lease_owner.ok_or_else(|| Error::Storage {
        message: "an expired Skill-validation operation has no lease owner".to_owned(),
    })?;
    let attempt = sqlx::query!(
        r#"
        update operation_attempts
        set state = 'dead_lettered', heartbeat_at = now(), completed_at = now(),
            safe_error_code = 'lease_expired_retry_exhausted'
        where tenant_id = $1 and operation_id = $2 and attempt_number = $3
          and worker_id = $4 and state = 'running'
        "#,
        tenant_id.as_uuid(),
        row.id,
        row.attempts,
        worker_id,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?
    .rows_affected();
    if attempt != 1 {
        return Err(Error::Storage {
            message: "an exhausted Skill-validation operation has no live fenced attempt"
                .to_owned(),
        });
    }
    let operation = sqlx::query!(
        r#"
        update durable_operations
        set state = 'dead_lettered', completed_at = now(), lease_owner = null,
            lease_expires_at = null, next_attempt_at = null,
            last_error_code = 'lease_expired_retry_exhausted', result = '{}'::jsonb,
            updated_at = now()
        where tenant_id = $1 and id = $2 and state = 'running'
          and attempts = $3 and lease_owner = $4
        "#,
        tenant_id.as_uuid(),
        row.id,
        row.attempts,
        worker_id,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?
    .rows_affected();
    if operation != 1 {
        return Err(lost_execution());
    }
    let operation_id = DurableOperationId::from_uuid(row.id);
    complete_outbox(conn, tenant_id, operation_id).await?;
    metrics::counter!(OPERATIONS_TOTAL, "kind" => "skill_validation", "state" => "dead_lettered")
        .increment(1);
    metrics::counter!(OPERATION_ATTEMPTS_TOTAL, "kind" => "skill_validation", "outcome" => "dead_lettered")
        .increment(1);
    Ok(Some(TerminalExecution {
        operation_id,
        scope_id: ScopeId::from_uuid(row.governing_scope_id),
        attempts: row.attempts,
    }))
}

/// Renews an execution lease and its attempt heartbeat under the same fence.
pub async fn renew_execution_lease(
    conn: &mut PgConnection,
    claim: &ExecutionClaim,
    worker_id: &str,
    lease_seconds: i64,
) -> Result<bool> {
    validate_worker_lease(worker_id, lease_seconds)?;
    let changed = sqlx::query!(
        r#"
        update durable_operations
        set lease_expires_at = statement_timestamp()
                + make_interval(secs => $5::double precision),
            updated_at = now()
        where tenant_id = $1 and id = $2 and state = 'running'
          and attempts = $3 and lease_owner = $4 and cancel_requested_at is null
          and lease_expires_at > statement_timestamp()
        "#,
        claim.operation.tenant_id.as_uuid(),
        claim.operation.id.as_uuid(),
        claim.attempt_number,
        worker_id,
        lease_seconds as f64,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?
    .rows_affected();
    if changed == 1 {
        let heartbeat = sqlx::query!(
            r#"update operation_attempts set heartbeat_at = now()
               where tenant_id = $1 and operation_id = $2 and attempt_number = $3
                 and worker_id = $4 and state = 'running'"#,
            claim.operation.tenant_id.as_uuid(),
            claim.operation.id.as_uuid(),
            claim.attempt_number,
            worker_id,
        )
        .execute(&mut *conn)
        .await
        .map_err(storage_error)?
        .rows_affected();
        if heartbeat != 1 {
            return Err(Error::Storage {
                message: "a renewed Skill-validation operation has no live fenced attempt"
                    .to_owned(),
            });
        }
    }
    Ok(changed == 1)
}

/// Locks and returns the current execution row before the caller persists any
/// business effect. A cancellation request remains visible on the result.
pub async fn lock_execution(
    conn: &mut PgConnection,
    claim: &ExecutionClaim,
    worker_id: &str,
) -> Result<Option<DurableOperation>> {
    sqlx::query_as!(
        OperationRow,
        r#"
        select tenant_id, id, operation_version, proposal_id, knowledge_item_id,
               skill_version_id, requested_by_identity_id, authorized_at,
               kind, state, input_hash, progress_percent, attempts, next_attempt_at,
               cancel_requested_at, lease_owner, lease_expires_at, last_error_code,
               result, created_at, updated_at, started_at, completed_at
        from durable_operations
        where tenant_id = $1 and id = $2 and state = 'running'
          and attempts = $3 and lease_owner = $4
          and lease_expires_at > statement_timestamp()
        for update
        "#,
        claim.operation.tenant_id.as_uuid(),
        claim.operation.id.as_uuid(),
        claim.attempt_number,
        worker_id,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?
    .map(TryInto::try_into)
    .transpose()
}

/// Completes the already-locked execution after its unique Skill result was
/// inserted. The caller appends audit last in the same transaction.
pub async fn complete_execution_success(
    conn: &mut PgConnection,
    claim: &ExecutionClaim,
    worker_id: &str,
) -> Result<()> {
    let attempt = sqlx::query!(
        r#"
        update operation_attempts
        set state = 'succeeded', heartbeat_at = now(), completed_at = now()
        where tenant_id = $1 and operation_id = $2 and attempt_number = $3
          and worker_id = $4 and state = 'running'
        "#,
        claim.operation.tenant_id.as_uuid(),
        claim.operation.id.as_uuid(),
        claim.attempt_number,
        worker_id,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?
    .rows_affected();
    let operation = sqlx::query!(
        r#"
        update durable_operations
        set state = 'succeeded', progress_percent = 100, completed_at = now(),
            lease_owner = null, lease_expires_at = null, next_attempt_at = null,
            last_error_code = null, result = '{}'::jsonb, updated_at = now()
        where tenant_id = $1 and id = $2 and state = 'running'
          and attempts = $3 and lease_owner = $4 and cancel_requested_at is null
          and exists (
              select 1 from skill_test_runs run
              where run.tenant_id = durable_operations.tenant_id
                and run.operation_id = durable_operations.id
                and run.version_id = durable_operations.skill_version_id
          )
        "#,
        claim.operation.tenant_id.as_uuid(),
        claim.operation.id.as_uuid(),
        claim.attempt_number,
        worker_id,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?
    .rows_affected();
    if attempt != 1 || operation != 1 {
        return Err(lost_execution());
    }
    complete_outbox(conn, claim.operation.tenant_id, claim.operation.id).await?;
    metrics::counter!(OPERATIONS_TOTAL, "kind" => "skill_validation", "state" => "succeeded")
        .increment(1);
    metrics::counter!(OPERATION_ATTEMPTS_TOTAL, "kind" => "skill_validation", "outcome" => "succeeded")
        .increment(1);
    Ok(())
}

/// Completes the already-locked attempt as a retry wait or dead letter.
pub async fn fail_execution(
    conn: &mut PgConnection,
    claim: &ExecutionClaim,
    worker_id: &str,
    safe_error_code: &str,
    retry_seconds: Option<i64>,
    max_attempts: i32,
) -> Result<OperationState> {
    if !(1..=100).contains(&max_attempts) {
        return Err(Error::Invalid {
            message: "operation max attempts must be between 1 and 100".to_owned(),
        });
    }
    let retry_seconds = retry_seconds.filter(|_| claim.attempt_number < max_attempts);
    if let Some(seconds) = retry_seconds {
        validate_retry(seconds, safe_error_code)?;
    } else {
        validate_error_code(safe_error_code)?;
    }
    let (attempt, operation, operation_state, attempt_state) = if let Some(seconds) = retry_seconds
    {
        let attempt = sqlx::query!(
            r#"
                update operation_attempts
                set state = 'retry_scheduled', heartbeat_at = now(),
                    completed_at = now(), safe_error_code = $5
                where tenant_id = $1 and operation_id = $2 and attempt_number = $3
                  and worker_id = $4 and state = 'running'
                "#,
            claim.operation.tenant_id.as_uuid(),
            claim.operation.id.as_uuid(),
            claim.attempt_number,
            worker_id,
            safe_error_code,
        )
        .execute(&mut *conn)
        .await
        .map_err(storage_error)?
        .rows_affected();
        let operation = sqlx::query!(
            r#"
                update durable_operations
                set state = 'failed', completed_at = null, lease_owner = null,
                    lease_expires_at = null,
                    next_attempt_at = statement_timestamp()
                        + make_interval(secs => $5::double precision),
                    last_error_code = $6, result = '{}'::jsonb, updated_at = now()
                where tenant_id = $1 and id = $2 and state = 'running'
                  and attempts = $3 and lease_owner = $4
                "#,
            claim.operation.tenant_id.as_uuid(),
            claim.operation.id.as_uuid(),
            claim.attempt_number,
            worker_id,
            seconds as f64,
            safe_error_code,
        )
        .execute(&mut *conn)
        .await
        .map_err(storage_error)?
        .rows_affected();
        (attempt, operation, "failed", "retry_scheduled")
    } else {
        let attempt = sqlx::query!(
            r#"
                update operation_attempts
                set state = 'dead_lettered', heartbeat_at = now(),
                    completed_at = now(), safe_error_code = $5
                where tenant_id = $1 and operation_id = $2 and attempt_number = $3
                  and worker_id = $4 and state = 'running'
                "#,
            claim.operation.tenant_id.as_uuid(),
            claim.operation.id.as_uuid(),
            claim.attempt_number,
            worker_id,
            safe_error_code,
        )
        .execute(&mut *conn)
        .await
        .map_err(storage_error)?
        .rows_affected();
        let operation = sqlx::query!(
            r#"
                update durable_operations
                set state = 'dead_lettered', completed_at = now(), lease_owner = null,
                    lease_expires_at = null, next_attempt_at = null,
                    last_error_code = $5, result = '{}'::jsonb, updated_at = now()
                where tenant_id = $1 and id = $2 and state = 'running'
                  and attempts = $3 and lease_owner = $4
                "#,
            claim.operation.tenant_id.as_uuid(),
            claim.operation.id.as_uuid(),
            claim.attempt_number,
            worker_id,
            safe_error_code,
        )
        .execute(&mut *conn)
        .await
        .map_err(storage_error)?
        .rows_affected();
        (attempt, operation, "dead_lettered", "dead_lettered")
    };
    if attempt != 1 || operation != 1 {
        return Err(lost_execution());
    }
    if let Some(seconds) = retry_seconds {
        let outbox = sqlx::query!(
            r#"
            update operation_outbox
            set state = 'pending', lease_owner = null, lease_expires_at = null,
                next_dispatch_at = statement_timestamp()
                    + make_interval(secs => $3::double precision),
                last_error_code = $4, completed_at = null, updated_at = now()
            where tenant_id = $1 and operation_id = $2
              and state in ('pending', 'dispatched')
            "#,
            claim.operation.tenant_id.as_uuid(),
            claim.operation.id.as_uuid(),
            seconds as f64,
            safe_error_code,
        )
        .execute(&mut *conn)
        .await
        .map_err(storage_error)?
        .rows_affected();
        if outbox == 0 {
            let state = sqlx::query_scalar!(
                r#"select state from operation_outbox
                   where tenant_id = $1 and operation_id = $2
                   for update"#,
                claim.operation.tenant_id.as_uuid(),
                claim.operation.id.as_uuid(),
            )
            .fetch_optional(&mut *conn)
            .await
            .map_err(storage_error)?;
            if state.as_deref() != Some("claimed") {
                return Err(Error::Storage {
                    message: "a retryable Skill-validation operation has no active outbox row"
                        .to_owned(),
                });
            }
        } else if outbox != 1 {
            return Err(Error::Storage {
                message: "a retryable Skill-validation operation has duplicate outbox rows"
                    .to_owned(),
            });
        }
    } else {
        complete_outbox(conn, claim.operation.tenant_id, claim.operation.id).await?;
    }
    let state = operation_state.parse().map_err(vocabulary)?;
    metrics::counter!(OPERATIONS_TOTAL, "kind" => "skill_validation", "state" => operation_state.to_owned())
        .increment(1);
    metrics::counter!(OPERATION_ATTEMPTS_TOTAL, "kind" => "skill_validation", "outcome" => attempt_state.to_owned())
        .increment(1);
    Ok(state)
}

async fn complete_outbox(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    operation_id: DurableOperationId,
) -> Result<()> {
    let changed = sqlx::query!(
        r#"
        update operation_outbox
        set state = 'completed', lease_owner = null, lease_expires_at = null,
            next_dispatch_at = null, completed_at = coalesce(completed_at, now()),
            updated_at = now()
        where tenant_id = $1 and operation_id = $2 and state <> 'completed'
        "#,
        tenant_id.as_uuid(),
        operation_id.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?
    .rows_affected();
    if changed != 1 {
        return Err(Error::Storage {
            message: "a terminal Skill-validation operation has no active outbox row".to_owned(),
        });
    }
    Ok(())
}

async fn dispatch_is_completed(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    operation_id: DurableOperationId,
) -> Result<bool> {
    let row = sqlx::query!(
        r#"
        select operation.state as operation_state, outbox.state as outbox_state
        from durable_operations operation
        join operation_outbox outbox
          on outbox.tenant_id = operation.tenant_id
         and outbox.operation_id = operation.id
         and outbox.operation_version = operation.operation_version
        where operation.tenant_id = $1 and operation.id = $2
          and operation.kind = 'skill_validation'
        "#,
        tenant_id.as_uuid(),
        operation_id.as_uuid(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?
    .ok_or_else(|| Error::Storage {
        message: "a claimed Skill-validation dispatch lost its operation or outbox row".to_owned(),
    })?;
    let operation_state: OperationState = row.operation_state.parse().map_err(vocabulary)?;
    let operation_terminal = operation_state.is_terminal();
    let outbox_completed = row.outbox_state == "completed";
    if operation_terminal != outbox_completed {
        return Err(Error::Storage {
            message: "a Skill-validation operation and its outbox disagree on completion"
                .to_owned(),
        });
    }
    Ok(operation_terminal)
}

async fn close_running_attempt_for_terminal(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    operation_id: DurableOperationId,
    operation_state: &str,
    attempts: i32,
    lease_owner: Option<&str>,
    safe_error_code: &str,
) -> Result<()> {
    if operation_state != OperationState::Running.as_str() {
        return Ok(());
    }
    validate_error_code(safe_error_code)?;
    let worker_id = lease_owner.ok_or_else(|| Error::Storage {
        message: "a running Skill-validation operation has no lease owner".to_owned(),
    })?;
    let changed = sqlx::query!(
        r#"
        update operation_attempts
        set state = 'dead_lettered', heartbeat_at = now(), completed_at = now(),
            safe_error_code = $5
        where tenant_id = $1 and operation_id = $2 and attempt_number = $3
          and worker_id = $4 and state = 'running'
        "#,
        tenant_id.as_uuid(),
        operation_id.as_uuid(),
        attempts,
        worker_id,
        safe_error_code,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?
    .rows_affected();
    if changed != 1 {
        return Err(Error::Storage {
            message: "a running Skill-validation operation has no live fenced attempt".to_owned(),
        });
    }
    Ok(())
}

fn validate_worker_lease(worker: &str, lease_seconds: i64) -> Result<()> {
    if worker.trim().is_empty()
        || worker.chars().count() > 255
        || !(1..=3600).contains(&lease_seconds)
    {
        return Err(Error::Invalid {
            message: "an operation worker is non-blank and its lease is 1..=3600 seconds"
                .to_owned(),
        });
    }
    Ok(())
}

fn validate_retry(retry_seconds: i64, safe_error_code: &str) -> Result<()> {
    if !(1..=3600).contains(&retry_seconds) {
        return Err(Error::Invalid {
            message: "an operation retry is 1..=3600 seconds".to_owned(),
        });
    }
    validate_error_code(safe_error_code)
}

fn validate_error_code(code: &str) -> Result<()> {
    let bytes = code.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 64
        || !bytes[0].is_ascii_lowercase()
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
    {
        return Err(Error::Invalid {
            message:
                "an operation error code is 1..=64 lowercase ASCII letters, digits or underscores"
                    .to_owned(),
        });
    }
    Ok(())
}

fn lost_execution() -> Error {
    Error::Conflict {
        message: "the durable operation execution fence is no longer held".to_owned(),
    }
}

fn vocabulary(error: Error) -> Error {
    Error::Storage {
        message: format!("stored operation vocabulary is invalid: {error}"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_and_retry_bounds_are_closed() {
        assert!(validate_worker_lease("worker-1", 30).is_ok());
        assert!(validate_worker_lease("", 30).is_err());
        assert!(validate_worker_lease("worker-1", 0).is_err());
        assert!(validate_retry(2, "dependency_unavailable").is_ok());
        assert!(validate_retry(0, "dependency_unavailable").is_err());
        assert!(validate_error_code("").is_err());
        assert!(validate_error_code("a").is_ok());
        assert!(validate_error_code("a1_retry").is_ok());
        assert!(validate_error_code(&format!("a{}", "0".repeat(63))).is_ok());
        for invalid in [
            "1starts_with_a_digit",
            "Uppercase",
            "has-hyphen",
            "has space",
            "non_ascii_é",
        ] {
            assert!(validate_error_code(invalid).is_err(), "accepted {invalid}");
        }
        assert!(validate_error_code(&format!("a{}", "0".repeat(64))).is_err());
    }
}
