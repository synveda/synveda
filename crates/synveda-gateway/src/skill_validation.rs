//! Native durable Skill-validation executor and the narrow leaf-adapter seam
//! (CPR-45, ADR-0102).
//!
//! The default worker sweeps active tenants with concurrency one. Optional
//! transports may call [`execute_operation`] after delivering untrusted tenant
//! routing, operation id and version fields; PostgreSQL still verifies the
//! association and fences every effect and retry.

use std::sync::Arc;
use std::time::{Duration, Instant};

use synveda_audit::{Actor, AuditAction, Outcome};
use synveda_policy::{Action, Pdp, Resource};
use synveda_store::operations::{
    self as store, ExecutionClaim, ExecutionSelection, TerminalExecution,
};
use synveda_store::{identities, rls, tenants};
use synveda_types::operation::{OperationState, SKILL_VALIDATION_OPERATION_VERSION};
use synveda_types::{DurableOperationId, Error, IdentityId, Result, TenantId, TenantStatus};
use tokio::sync::watch;

use crate::audit;
use crate::skills;

const COMPONENT: &str = "skill-validation";
const POLL_INTERVAL: Duration = Duration::from_secs(1);
const QUEUE_AGE_INTERVAL: Duration = Duration::from_secs(15);
const LEASE_DURATION: Duration = Duration::from_secs(30);
const LEASE_RENEWAL: Duration = Duration::from_secs(10);
const LEASE_STOP_TIMEOUT: Duration = Duration::from_secs(2);
const EVALUATION_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_ATTEMPTS: i32 = 3;

/// Worker dependencies shared by native and optional leaf execution.
#[derive(Clone)]
pub struct Runtime {
    pool: sqlx::PgPool,
    pdp: Arc<Pdp>,
    worker_id: String,
}

impl Runtime {
    /// Creates a bounded executor with a process-unique lease identity.
    #[must_use]
    pub fn new(pool: sqlx::PgPool, pdp: Arc<Pdp>) -> Self {
        Self {
            pool,
            pdp,
            worker_id: format!("{COMPONENT}-{}", DurableOperationId::new()),
        }
    }

    /// Overrides only the process lease identity for deterministic tests and
    /// separately supervised adapter workers.
    #[must_use]
    pub fn with_worker_id(mut self, worker_id: impl Into<String>) -> Self {
        self.worker_id = worker_id.into();
        self
    }
}

/// Result of one provider-neutral execution request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecuteOutcome {
    /// No due operation matched, or another worker owns its live lease.
    NotClaimed,
    /// The unique test effect committed.
    Succeeded,
    /// A bounded retry was scheduled.
    RetryScheduled,
    /// The retry budget or a permanent refusal ended the operation.
    DeadLettered,
    /// The lease was lost; a future dispatcher may reclaim it after expiry.
    LeaseLost,
}

/// Appends the content-free terminal audit for a dispatcher retry budget
/// exhaustion. The caller owns the tenant transaction so the outbox terminal
/// transition and audit evidence commit together.
pub async fn record_dispatch_dead_letter(
    conn: &mut sqlx::PgConnection,
    tenant: TenantId,
    terminal: TerminalExecution,
) -> Result<()> {
    audit::record_as(
        conn,
        tenant,
        Actor::system(COMPONENT),
        AuditAction::OperationFinished,
        Resource::Scope(terminal.scope_id).to_string(),
        Outcome::Failure,
        serde_json::json!({
            "operation_id": terminal.operation_id,
            "kind": "skill_validation",
            "operation_version": SKILL_VALIDATION_OPERATION_VERSION,
            "state": "dead_lettered",
            "attempts": terminal.attempts,
            "error_code": "dispatch_retry_exhausted",
        }),
    )
    .await?;
    Ok(())
}

/// Samples queue age and, when selected, runs the default PostgreSQL-backed
/// executor until shutdown. The core worker retains observability when an
/// optional transport owns execution.
pub async fn run(runtime: Runtime, execute_operations: bool, mut shutdown: watch::Receiver<bool>) {
    let mut ticker = tokio::time::interval(POLL_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut next_queue_age_sample = Instant::now();
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
            _ = ticker.tick() => {}
        }
        if *shutdown.borrow() {
            return;
        }
        let active = match tenants::active(&runtime.pool).await {
            Ok(active) => active,
            Err(error) => {
                tracing::warn!(%error, "Skill-validation tenant sweep failed");
                metrics::counter!(store::OPERATION_SWEEPS_TOTAL, "outcome" => "tenant_inventory_error")
                    .increment(1);
                continue;
            }
        };
        let sample_queue_ages = Instant::now() >= next_queue_age_sample;
        if sample_queue_ages {
            next_queue_age_sample = Instant::now() + QUEUE_AGE_INTERVAL;
        }
        let tenant_ids = active.iter().map(|tenant| tenant.id).collect::<Vec<_>>();
        let mut complete = true;
        if execute_operations {
            for tenant in active {
                if *shutdown.borrow() {
                    return;
                }
                let outcome = execute_operation(
                    &runtime,
                    tenant.id,
                    None,
                    SKILL_VALIDATION_OPERATION_VERSION,
                )
                .await;
                if let Err(error) = outcome {
                    complete = false;
                    tracing::warn!(tenant.id = %tenant.id, %error, "Skill-validation sweep failed");
                }
            }
        }
        if sample_queue_ages && let Err(error) = record_queue_ages(&runtime.pool, &tenant_ids).await
        {
            tracing::warn!(%error, "Skill-validation queue-age sample failed");
        }
        metrics::counter!(
            store::OPERATION_SWEEPS_TOTAL,
            "outcome" => if !execute_operations {
                "observed"
            } else if complete {
                "completed"
            } else {
                "partial"
            },
        )
        .increment(1);
    }
}

/// Samples customer-safe operation and outbox ages without tenant labels.
///
/// The native executor and optional transport leaf share this one bounded
/// implementation so changing transport does not create an observability gap.
pub async fn record_queue_ages(pool: &sqlx::PgPool, tenants: &[TenantId]) -> Result<()> {
    let mut operation_age = 0.0_f64;
    let mut outbox_age = 0.0_f64;
    for tenant in tenants {
        let ages = queue_ages(pool, *tenant).await?;
        operation_age = operation_age.max(ages.operation_seconds);
        outbox_age = outbox_age.max(ages.outbox_seconds);
    }
    metrics::gauge!(store::OPERATION_OLDEST_PENDING_AGE_SECONDS).set(operation_age);
    metrics::gauge!(store::OPERATION_OUTBOX_OLDEST_PENDING_AGE_SECONDS).set(outbox_age);
    Ok(())
}

async fn queue_ages(pool: &sqlx::PgPool, tenant: TenantId) -> Result<store::QueueAges> {
    let mut tx = rls::begin_tenant_tx(pool, tenant).await?;
    let ages = store::skill_validation_queue_ages(&mut *tx, tenant).await?;
    tx.rollback().await.map_err(commit_error)?;
    Ok(ages)
}

/// Executes one due operation for `tenant`, optionally restricted to the
/// opaque id delivered by a leaf adapter. The operation version is verified
/// from authoritative state after the claim.
#[tracing::instrument(
    name = "skill_validation.execute",
    skip_all,
    fields(operation.kind = "skill_validation", operation.version = SKILL_VALIDATION_OPERATION_VERSION)
)]
pub async fn execute_operation(
    runtime: &Runtime,
    tenant: TenantId,
    operation_id: Option<DurableOperationId>,
    expected_version: u16,
) -> Result<ExecuteOutcome> {
    if expected_version != SKILL_VALIDATION_OPERATION_VERSION {
        return Err(Error::Invalid {
            message: "unsupported Skill-validation operation version".to_owned(),
        });
    }
    if !matches!(
        tenants::by_id(&runtime.pool, tenant).await?,
        Some(current) if current.status == TenantStatus::Active
    ) {
        return Ok(ExecuteOutcome::NotClaimed);
    }
    let lease_seconds = i64::try_from(LEASE_DURATION.as_secs()).unwrap_or(30);
    let mut claim_tx = rls::begin_tenant_tx(&runtime.pool, tenant).await?;
    let claim = store::claim_skill_validation_execution(
        &mut claim_tx,
        tenant,
        &runtime.worker_id,
        lease_seconds,
        operation_id,
        MAX_ATTEMPTS,
    )
    .await?;
    let Some(selection) = claim else {
        claim_tx.rollback().await.map_err(commit_error)?;
        return Ok(ExecuteOutcome::NotClaimed);
    };
    let claim = match selection {
        ExecutionSelection::Claimed(claim) => claim,
        ExecutionSelection::DeadLettered(terminal) => {
            audit::record_as(
                &mut claim_tx,
                tenant,
                Actor::system(COMPONENT),
                AuditAction::OperationFinished,
                Resource::Scope(terminal.scope_id).to_string(),
                Outcome::Failure,
                serde_json::json!({
                    "operation_id": terminal.operation_id,
                    "kind": "skill_validation",
                    "operation_version": SKILL_VALIDATION_OPERATION_VERSION,
                    "state": "dead_lettered",
                    "attempts": terminal.attempts,
                    "error_code": "lease_expired_retry_exhausted",
                }),
            )
            .await?;
            claim_tx.commit().await.map_err(commit_error)?;
            return Ok(ExecuteOutcome::DeadLettered);
        }
    };
    if claim.operation.operation_version != expected_version {
        claim_tx.rollback().await.map_err(commit_error)?;
        return Err(Error::Invalid {
            message: "unsupported Skill-validation operation version".to_owned(),
        });
    }
    claim_tx.commit().await.map_err(commit_error)?;

    let started = Instant::now();
    let lease = LeaseGuard::start(runtime, &claim);
    let evaluation = tokio::time::timeout(EVALUATION_TIMEOUT, evaluate(runtime, &claim));
    tokio::pin!(evaluation);
    let evaluated = tokio::select! {
        biased;
        () = lease.lost() => {
            metrics::counter!(
                store::OPERATION_ATTEMPTS_TOTAL,
                "kind" => "skill_validation",
                "outcome" => "lease_lost",
            ).increment(1);
            return Ok(ExecuteOutcome::LeaseLost);
        }
        result = &mut evaluation => result,
    };
    lease.stop().await;
    let outcome = match evaluated {
        Ok(Ok(validation)) => match finish_success(runtime, &claim, &validation).await {
            Ok(outcome) => outcome,
            Err(error) => finish_failure(runtime, &claim, &error).await?,
        },
        Ok(Err(error)) => finish_failure(runtime, &claim, &error).await?,
        Err(_) => {
            let error = Error::Dependency {
                service: "skill_validation".to_owned(),
                message: "the bounded validation evaluation timed out".to_owned(),
            };
            finish_failure(runtime, &claim, &error).await?
        }
    };
    metrics::histogram!(
        store::OPERATION_ATTEMPT_SECONDS,
        "kind" => "skill_validation",
        "outcome" => outcome.as_str(),
    )
    .record(started.elapsed().as_secs_f64());
    Ok(outcome)
}

impl ExecuteOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::NotClaimed => "not_claimed",
            Self::Succeeded => "succeeded",
            Self::RetryScheduled => "retry_scheduled",
            Self::DeadLettered => "dead_lettered",
            Self::LeaseLost => "lease_lost",
        }
    }
}

async fn evaluate(runtime: &Runtime, claim: &ExecutionClaim) -> Result<skills::ValidationEvidence> {
    let requester_id = requester_id(claim)?;
    let version_id = claim
        .operation
        .skill_version_id
        .ok_or_else(invalid_operation_target)?;
    let mut tx = rls::begin_tenant_tx(&runtime.pool, claim.operation.tenant_id).await?;
    let requester = identities::by_id(&mut *tx, claim.operation.tenant_id, requester_id)
        .await?
        .ok_or_else(requester_unavailable)?;
    let (skill, version, _) = skills::authorize_durable_validation(
        &runtime.pdp,
        &mut tx,
        claim.operation.tenant_id,
        requester,
        claim.skill_id,
        version_id,
        claim.scope_id,
    )
    .await?;
    let validation =
        skills::evaluate_validation(&mut tx, claim.operation.tenant_id, &skill, &version).await?;
    tx.commit().await.map_err(commit_error)?;
    Ok(validation)
}

async fn finish_success(
    runtime: &Runtime,
    claim: &ExecutionClaim,
    validation: &skills::ValidationEvidence,
) -> Result<ExecuteOutcome> {
    let requester_id = requester_id(claim)?;
    let version_id = claim
        .operation
        .skill_version_id
        .ok_or_else(invalid_operation_target)?;
    let scope_id = claim.scope_id;
    let mut tx = rls::begin_tenant_tx(&runtime.pool, claim.operation.tenant_id).await?;
    let Some(_) = store::lock_execution(&mut tx, claim, &runtime.worker_id).await? else {
        tx.rollback().await.map_err(commit_error)?;
        return Ok(ExecuteOutcome::LeaseLost);
    };
    if !matches!(
        tenants::by_id(&mut *tx, claim.operation.tenant_id).await?,
        Some(current) if current.status == TenantStatus::Active
    ) {
        return Err(tenant_unavailable());
    }
    let requester = identities::by_id(&mut *tx, claim.operation.tenant_id, requester_id)
        .await?
        .ok_or_else(requester_unavailable)?;
    let (_, _, authorized) = skills::authorize_durable_validation(
        &runtime.pdp,
        &mut tx,
        claim.operation.tenant_id,
        requester,
        claim.skill_id,
        version_id,
        scope_id,
    )
    .await?;
    let run = skills::record_validation_result(
        &mut tx,
        claim.operation.tenant_id,
        version_id,
        requester_id,
        Some(claim.operation.id),
        validation,
    )
    .await?;
    store::complete_execution_success(&mut tx, claim, &runtime.worker_id).await?;
    audit::record_as(
        &mut tx,
        claim.operation.tenant_id,
        Actor::system(COMPONENT),
        AuditAction::SkillTestRecorded,
        Resource::Scope(scope_id).to_string(),
        Outcome::Success,
        serde_json::json!({
            "operation_id": claim.operation.id,
            "test_run_id": run.id,
            "skill_id": claim.skill_id,
            "version_id": version_id,
            "requested_by_identity_id": requester_id,
            "harness": "validation_sandbox",
            "harness_version": skills::VALIDATION_HARNESS_VERSION,
            "outcome": "passed",
            "executes_bundle_code": false,
            "authz": audit::decision_context(Action::SkillWrite, &authorized),
        }),
    )
    .await?;
    audit::record_as(
        &mut tx,
        claim.operation.tenant_id,
        Actor::system(COMPONENT),
        AuditAction::OperationFinished,
        Resource::Scope(scope_id).to_string(),
        Outcome::Success,
        terminal_payload(claim, "succeeded", None),
    )
    .await?;
    tx.commit().await.map_err(commit_error)?;
    Ok(ExecuteOutcome::Succeeded)
}

async fn finish_failure(
    runtime: &Runtime,
    claim: &ExecutionClaim,
    error: &Error,
) -> Result<ExecuteOutcome> {
    let scope_id = claim.scope_id;
    let mut tx = rls::begin_tenant_tx(&runtime.pool, claim.operation.tenant_id).await?;
    let Some(_) = store::lock_execution(&mut tx, claim, &runtime.worker_id).await? else {
        tx.rollback().await.map_err(commit_error)?;
        return Ok(ExecuteOutcome::LeaseLost);
    };
    let failure = classify_failure(error, claim.attempt_number);
    let state = store::fail_execution(
        &mut tx,
        claim,
        &runtime.worker_id,
        failure.code,
        failure
            .retry_after
            .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)),
        MAX_ATTEMPTS,
    )
    .await?;
    if let Error::PolicyDenied {
        action,
        resource,
        reason,
    } = error
    {
        audit::record_as(
            &mut tx,
            claim.operation.tenant_id,
            Actor::system(COMPONENT),
            AuditAction::AuthzDecision,
            Resource::Scope(scope_id).to_string(),
            Outcome::Deny,
            serde_json::json!({
                "op": "skill_validation.execute",
                "operation_id": claim.operation.id,
                "requested_by_identity_id": claim.operation.requested_by,
                "action": action,
                "resource": resource,
                "reason": reason,
            }),
        )
        .await?;
    }
    if state == OperationState::DeadLettered {
        audit::record_as(
            &mut tx,
            claim.operation.tenant_id,
            Actor::system(COMPONENT),
            AuditAction::OperationFinished,
            Resource::Scope(scope_id).to_string(),
            Outcome::Failure,
            terminal_payload(claim, "dead_lettered", Some(failure.code)),
        )
        .await?;
    }
    tx.commit().await.map_err(commit_error)?;
    Ok(if state == OperationState::DeadLettered {
        ExecuteOutcome::DeadLettered
    } else {
        ExecuteOutcome::RetryScheduled
    })
}

fn terminal_payload(
    claim: &ExecutionClaim,
    state: &'static str,
    error_code: Option<&'static str>,
) -> serde_json::Value {
    serde_json::json!({
        "operation_id": claim.operation.id,
        "kind": "skill_validation",
        "operation_version": claim.operation.operation_version,
        "state": state,
        "attempts": claim.attempt_number,
        "error_code": error_code,
    })
}

struct Failure {
    code: &'static str,
    retry_after: Option<Duration>,
}

fn classify_failure(error: &Error, attempt: i32) -> Failure {
    let permanent = matches!(
        error,
        Error::Unauthenticated { .. }
            | Error::PolicyDenied { .. }
            | Error::NotFound { .. }
            | Error::Invalid { .. }
    );
    if permanent || attempt >= MAX_ATTEMPTS {
        return Failure {
            code: if matches!(
                error,
                Error::Unauthenticated { .. } | Error::PolicyDenied { .. } | Error::NotFound { .. }
            ) {
                "authorization_unavailable"
            } else if permanent {
                "validation_invalid"
            } else {
                "retry_exhausted"
            },
            retry_after: None,
        };
    }
    let seconds = match attempt {
        i32::MIN..=1 => 2,
        2 => 10,
        _ => 30,
    };
    Failure {
        code: "dependency_unavailable",
        retry_after: Some(Duration::from_secs(seconds)),
    }
}

fn requester_id(claim: &ExecutionClaim) -> Result<IdentityId> {
    claim
        .operation
        .requested_by
        .ok_or_else(invalid_operation_target)
}

fn requester_unavailable() -> Error {
    Error::PolicyDenied {
        action: "skill_validation.execute".to_owned(),
        resource: "durable operation".to_owned(),
        reason: "the requesting identity is unavailable".to_owned(),
    }
}

fn tenant_unavailable() -> Error {
    Error::PolicyDenied {
        action: "skill_validation.execute".to_owned(),
        resource: "durable operation".to_owned(),
        reason: "the owning tenant is unavailable".to_owned(),
    }
}

fn invalid_operation_target() -> Error {
    Error::Internal {
        message: "Skill-validation operation target is incomplete".to_owned(),
    }
}

fn commit_error(error: sqlx::Error) -> Error {
    Error::Storage {
        message: format!("finish Skill-validation transaction: {error}"),
    }
}

struct LeaseGuard {
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    lost: watch::Receiver<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl LeaseGuard {
    fn start(runtime: &Runtime, claim: &ExecutionClaim) -> Self {
        let runtime = runtime.clone();
        let claim = claim.clone();
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel();
        let (lost_tx, lost_rx) = watch::channel(false);
        let task = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(LEASE_RENEWAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            ticker.tick().await;
            loop {
                tokio::select! {
                    _ = &mut stop_rx => return,
                    _ = ticker.tick() => {}
                }
                let mut tx =
                    match rls::begin_tenant_tx(&runtime.pool, claim.operation.tenant_id).await {
                        Ok(tx) => tx,
                        Err(_) => {
                            let _ = lost_tx.send(true);
                            return;
                        }
                    };
                let renewed = store::renew_execution_lease(
                    &mut tx,
                    &claim,
                    &runtime.worker_id,
                    i64::try_from(LEASE_DURATION.as_secs()).unwrap_or(30),
                )
                .await;
                if !matches!(renewed, Ok(true)) || tx.commit().await.is_err() {
                    let _ = lost_tx.send(true);
                    return;
                }
            }
        });
        Self {
            stop: Some(stop_tx),
            lost: lost_rx,
            task,
        }
    }

    async fn lost(&self) {
        let mut lost = self.lost.clone();
        if *lost.borrow() {
            return;
        }
        let _ = lost.changed().await;
    }

    async fn stop(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if tokio::time::timeout(LEASE_STOP_TIMEOUT, &mut self.task)
            .await
            .is_err()
        {
            self.task.abort();
            let _ = (&mut self.task).await;
        }
    }
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_policy_is_bounded_and_closed() {
        let denied = Error::PolicyDenied {
            action: "skill.write".to_owned(),
            resource: "scope".to_owned(),
            reason: "denied".to_owned(),
        };
        let failure = classify_failure(&denied, 1);
        assert_eq!(failure.code, "authorization_unavailable");
        assert_eq!(failure.retry_after, None);

        let transient = Error::Dependency {
            service: "postgres".to_owned(),
            message: "unavailable".to_owned(),
        };
        assert_eq!(
            classify_failure(&transient, 1).retry_after,
            Some(Duration::from_secs(2))
        );
        assert_eq!(classify_failure(&transient, MAX_ATTEMPTS).retry_after, None);
    }

    #[test]
    fn execution_outcomes_have_closed_metric_labels() {
        assert_eq!(ExecuteOutcome::NotClaimed.as_str(), "not_claimed");
        assert_eq!(ExecuteOutcome::Succeeded.as_str(), "succeeded");
        assert_eq!(ExecuteOutcome::RetryScheduled.as_str(), "retry_scheduled");
        assert_eq!(ExecuteOutcome::DeadLettered.as_str(), "dead_lettered");
        assert_eq!(ExecuteOutcome::LeaseLost.as_str(), "lease_lost");
    }
}
