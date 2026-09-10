//! Restart-safe capture extraction worker (CPR-18, ADR-0083).
//!
//! A tenant-bound database lease is the work address. The worker
//! reads one frozen batch under tenant RLS, re-decides `SessionWrite` as the
//! principal that opened the run, calls the configured extractor outside any
//! transaction, then re-decides each exact current Knowledge neighbour before
//! persisting a match. Its only domain output is reviewable candidates.

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde_json::{Value, json};
use sqlx::PgPool;
use synveda_audit::{Actor, AuditAction, AuditEvent, Outcome};
use synveda_policy::{
    Action, AuthzContext, AuthzDecision, Pdp, Principal, Resource, ResourceEntity, ScopeNode,
};
use synveda_store::capture::{self, NewCaptureCandidate};
use synveda_store::knowledge::KnowledgeSnapshot;
use synveda_store::knowledge_search::{self, Filters};
use synveda_store::{
    anchors, configuration, identities, knowledge, knowledge_conflicts, policy_assignments, rls,
    tenants,
};
use synveda_types::capture::{CaptureBatch, CaptureMatch, CaptureMatchKind, CaptureSourceKind};
use synveda_types::configuration::{ConfigurationDocument, ExternalProvider};
use synveda_types::knowledge::{
    ConflictClassification, KnowledgeLifecycleState, KnowledgeRevisionContent,
    classify_knowledge_match, normalise_knowledge_tags, validate_knowledge_revision_content,
};
use synveda_types::{
    CaptureBatchId, Error, IdentityKind, IdentityStatus, Result, ScopeId, Sensitivity, SessionId,
    TenantId,
};
use tokio::sync::{oneshot, watch};

use crate::chain::scope_chain;
use crate::extraction::{AnyExtractor, ExtractionInput, Extractor};

/// Extractor calls, labelled by implementation and outcome.
pub const CAPTURE_EXTRACTOR_REQUESTS_TOTAL: &str = "synveda_capture_extractor_requests_total";
/// Extractor latency by implementation.
pub const CAPTURE_EXTRACTOR_SECONDS: &str = "synveda_capture_extractor_seconds";
/// Batch processing outcomes.
pub const CAPTURE_BATCHES_TOTAL: &str = "synveda_capture_batches_total";
/// Candidate proposals materialised by Knowledge type.
pub const CAPTURE_CANDIDATES_TOTAL: &str = "synveda_capture_candidates_total";

const ACTOR_COMPONENT: &str = "capture-extraction";
const DEFAULT_LEASE_OWNER_PREFIX: &str = "capture-worker";
const MIN_RENEWAL_INTERVAL: Duration = Duration::from_millis(250);
const MAX_RENEWAL_INTERVAL: Duration = Duration::from_secs(30);
const RENEWAL_STOP_TIMEOUT: Duration = Duration::from_secs(2);

/// Background pacing and lease bound.
#[derive(Debug, Clone)]
pub struct Config {
    /// Delay between complete tenant sweeps.
    pub poll_interval: Duration,
    /// Lease retained while one model pass runs.
    pub lease_duration: Duration,
    /// Claimed batches per tenant per sweep.
    pub batches_per_tenant: usize,
    /// Process-unique worker identity recorded on the lease.
    pub lease_owner: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(1),
            lease_duration: Duration::from_secs(60),
            batches_per_tenant: 8,
            lease_owner: format!("{DEFAULT_LEASE_OWNER_PREFIX}-{}", CaptureBatchId::new()),
        }
    }
}

/// Everything the worker holds for its lifetime.
#[derive(Clone)]
pub struct Deps {
    /// Shared database pool.
    pub pool: PgPool,
    /// Embedded policy decision point.
    pub pdp: Arc<Pdp>,
    /// Configured extraction implementation.
    pub extractor: Arc<AnyExtractor>,
}

/// One complete sweep summary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepSummary {
    /// Active tenants visited.
    pub tenants: usize,
    /// Batches completed.
    pub completed: usize,
    /// Attempts released for retry or terminally failed.
    pub failed_attempts: usize,
    /// Attempts abandoned because their lease could no longer be proved.
    pub abandoned_attempts: usize,
}

struct LeaseGuard {
    stop: Option<oneshot::Sender<()>>,
    lost: watch::Receiver<bool>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl LeaseGuard {
    async fn start(deps: &Deps, config: &Config, batch: &CaptureBatch) -> Result<Self> {
        let pool = deps.pool.clone();
        let batch = batch.clone();
        let lease_owner = config.lease_owner.clone();
        let lease_seconds = lease_seconds(config.lease_duration);
        let renewal_interval = renewal_interval(config.lease_duration);
        // The claim's clock starts inside the preflight transaction. Prove it
        // is still live after that transaction commits and before disclosing
        // any frozen event to an external extractor.
        renew_claim(&pool, &batch, &lease_owner, lease_seconds).await?;
        Ok(Self::spawn_with_renewal(
            renewal_interval,
            renewal_interval,
            move || {
                let pool = pool.clone();
                let batch = batch.clone();
                let lease_owner = lease_owner.clone();
                async move { renew_claim(&pool, &batch, &lease_owner, lease_seconds).await }
            },
        ))
    }

    fn spawn_with_renewal<F, Fut>(
        renewal_interval: Duration,
        renewal_timeout: Duration,
        mut renew: F,
    ) -> Self
    where
        F: FnMut() -> Fut + Send + 'static,
        Fut: Future<Output = Result<()>> + Send,
    {
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let (lost_tx, lost_rx) = watch::channel(false);
        let task = tokio::spawn(async move {
            let first_renewal = tokio::time::Instant::now() + renewal_interval;
            let mut ticker = tokio::time::interval_at(first_renewal, renewal_interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = &mut stop_rx => return,
                    _ = ticker.tick() => {}
                }
                // Dropping a SQLx future cancels the in-flight acquisition or
                // query and drops its transaction. Stop therefore remains
                // effective even when PostgreSQL or the pool is stalled.
                let renewal = tokio::select! {
                    _ = &mut stop_rx => return,
                    renewal = tokio::time::timeout(renewal_timeout, renew()) => renewal,
                };
                if !matches!(renewal, Ok(Ok(()))) {
                    let _ = lost_tx.send(true);
                    return;
                }
            }
        });
        Self {
            stop: Some(stop_tx),
            lost: lost_rx,
            task: Some(task),
        }
    }

    fn is_lost(&self) -> bool {
        *self.lost.borrow() || self.lost.has_changed().is_err()
    }

    async fn wait_for_loss(&mut self) {
        if self.is_lost() {
            return;
        }
        let _ = self.lost.changed().await;
    }

    async fn stop(mut self) {
        self.request_stop();
        let Some(task) = self.task.as_mut() else {
            return;
        };
        match tokio::time::timeout(RENEWAL_STOP_TIMEOUT, task).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(%error, "capture lease renewal task did not stop cleanly");
            }
            Err(_) => {
                tracing::warn!(
                    timeout_ms = RENEWAL_STOP_TIMEOUT.as_millis() as u64,
                    "capture lease renewal task exceeded its stop bound; aborting"
                );
                if let Some(task) = self.task.as_mut() {
                    task.abort();
                    let _ = tokio::time::timeout(RENEWAL_STOP_TIMEOUT, task).await;
                }
            }
        }
        self.task.take();
    }

    fn request_stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

async fn renew_claim(
    pool: &PgPool,
    batch: &CaptureBatch,
    lease_owner: &str,
    lease_seconds: i64,
) -> Result<()> {
    let mut tx = rls::begin_tenant_tx(pool, batch.tenant_id).await?;
    capture::renew_batch(&mut tx, batch, lease_owner, lease_seconds).await?;
    tx.commit().await.map_err(commit_error)
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        self.request_stop();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

fn lease_seconds(duration: Duration) -> i64 {
    i64::try_from(duration.as_secs())
        .unwrap_or(i64::MAX)
        .clamp(1, 3_600)
}

fn renewal_interval(duration: Duration) -> Duration {
    (duration / 3).clamp(MIN_RENEWAL_INTERVAL, MAX_RENEWAL_INTERVAL)
}

/// Runs one immediate pass and then one per configured interval until
/// shutdown.
///
/// Shutdown is observed before tenant discovery and each claim. If it arrives
/// during an attempt, dropping the attempt cancels the provider request,
/// rolls back any open transaction and drops the renewal guard; the fenced
/// row becomes reclaimable only after its database-time lease expires.
pub async fn run(deps: Deps, config: Config, mut shutdown: watch::Receiver<bool>) {
    let mut ticker = tokio::time::interval(config.poll_interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
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
        match sweep_until_shutdown(&deps, &config, &mut shutdown).await {
            Ok(Some(_)) => {}
            Ok(None) => return,
            Err(error) => tracing::warn!(%error, "capture sweep failed; retrying"),
        }
    }
}

async fn sweep_until_shutdown(
    deps: &Deps,
    config: &Config,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<Option<SweepSummary>> {
    let active = tokio::select! {
        biased;
        () = shutdown_requested(shutdown) => return Ok(None),
        result = tenants::active(&deps.pool) => result?,
    };
    let mut summary = SweepSummary::default();
    for tenant in active {
        if *shutdown.borrow() {
            return Ok(None);
        }
        summary.tenants += 1;
        for _ in 0..config.batches_per_tenant.max(1) {
            let outcome = tokio::select! {
                biased;
                () = shutdown_requested(shutdown) => {
                    tracing::info!(
                        tenant.id = %tenant.id,
                        "capture attempt cancelled for worker shutdown; any fenced claim waits for lease expiry"
                    );
                    return Ok(None);
                }
                result = process_one(deps, config, tenant.id) => result,
            };
            match outcome {
                Ok(ProcessOutcome::Empty) => break,
                Ok(ProcessOutcome::Completed) => summary.completed += 1,
                Ok(ProcessOutcome::FailedAttempt) => summary.failed_attempts += 1,
                Ok(ProcessOutcome::Abandoned) => {
                    summary.abandoned_attempts += 1;
                    break;
                }
                Err(error) => {
                    summary.failed_attempts += 1;
                    tracing::warn!(tenant.id = %tenant.id, %error, "capture tenant pass failed");
                    break;
                }
            }
        }
    }
    Ok(Some(summary))
}

async fn shutdown_requested(shutdown: &mut watch::Receiver<bool>) {
    loop {
        if *shutdown.borrow() || shutdown.changed().await.is_err() {
            return;
        }
    }
}

/// Sweeps active tenants without allowing one tenant failure to starve the
/// rest. Public for deterministic acceptance tests.
#[tracing::instrument(name = "capture.worker.sweep", skip_all, err(Display))]
pub async fn sweep_once(deps: &Deps, config: &Config) -> Result<SweepSummary> {
    let mut summary = SweepSummary::default();
    for tenant in tenants::active(&deps.pool).await? {
        summary.tenants += 1;
        for _ in 0..config.batches_per_tenant.max(1) {
            match process_one(deps, config, tenant.id).await {
                Ok(ProcessOutcome::Empty) => break,
                Ok(ProcessOutcome::Completed) => summary.completed += 1,
                Ok(ProcessOutcome::FailedAttempt) => summary.failed_attempts += 1,
                Ok(ProcessOutcome::Abandoned) => {
                    summary.abandoned_attempts += 1;
                    break;
                }
                Err(error) => {
                    summary.failed_attempts += 1;
                    tracing::warn!(tenant.id = %tenant.id, %error, "capture tenant pass failed");
                    break;
                }
            }
        }
    }
    Ok(summary)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessOutcome {
    Empty,
    Completed,
    FailedAttempt,
    Abandoned,
}

/// Processes at most one batch for a tenant. Public behavior is exposed by
/// [`sweep_once`]; keeping this function private prevents callers from
/// inventing a second claim discipline.
#[tracing::instrument(
    name = "capture.worker.tenant_attempt",
    skip_all,
    fields(tenant.id = %tenant_id, worker = %config.lease_owner),
    err(Display)
)]
async fn process_one(deps: &Deps, config: &Config, tenant_id: TenantId) -> Result<ProcessOutcome> {
    let lease_seconds = lease_seconds(config.lease_duration);
    let mut claim_tx = rls::begin_tenant_tx(&deps.pool, tenant_id).await?;
    if let Some(expired) = capture::fail_expired_exhausted_batch(&mut claim_tx, tenant_id).await? {
        append_system_failure(&mut claim_tx, &expired, "lease_expired").await?;
        claim_tx.commit().await.map_err(commit_error)?;
        metrics::counter!(CAPTURE_BATCHES_TOTAL, "outcome" => "lease_expired").increment(1);
        return Ok(ProcessOutcome::FailedAttempt);
    }
    let Some(batch) =
        capture::claim_batch(&mut claim_tx, tenant_id, &config.lease_owner, lease_seconds).await?
    else {
        claim_tx.rollback().await.map_err(commit_error)?;
        return Ok(ProcessOutcome::Empty);
    };
    let session_id = capture_session_id(&batch)?;
    let events = capture::frozen_events(&mut *claim_tx, tenant_id, batch.id).await?;
    let runtime_configuration = exact_configuration(&mut claim_tx, &batch).await;
    let runtime_configuration = match runtime_configuration {
        Ok(document) => document,
        Err(error) => {
            claim_tx.commit().await.map_err(commit_error)?;
            return fail_attempt(deps, config, &batch, "configuration_invalid", error).await;
        }
    };
    if !runtime_configuration.capture.enabled {
        claim_tx.commit().await.map_err(commit_error)?;
        return fail_attempt(
            deps,
            config,
            &batch,
            "capture_disabled",
            Error::PolicyDenied {
                action: "capture.extract".to_owned(),
                resource: Resource::Session(session_id).to_string(),
                reason: "the frozen configuration disables capture".to_owned(),
            },
        )
        .await;
    }
    if let Some(provider) = extractor_provider(deps.extractor.method())
        && !runtime_configuration.permits_provider(provider)
    {
        claim_tx.commit().await.map_err(commit_error)?;
        return fail_attempt(
            deps,
            config,
            &batch,
            "provider_not_allowed",
            Error::PolicyDenied {
                action: "capture.extract".to_owned(),
                resource: Resource::Session(session_id).to_string(),
                reason: format!("the frozen configuration does not allow provider {provider}"),
            },
        )
        .await;
    }
    let session_decision = decide_exact(
        deps,
        &mut claim_tx,
        tenant_id,
        &batch.principal_id,
        batch.scope_id,
        batch.project_id,
        Action::SessionWrite,
        Resource::Session(session_id),
        vec![ResourceEntity::Session {
            id: session_id,
            scope_id: batch.scope_id,
        }],
        None,
    )
    .await?;
    if !session_decision.allowed {
        let failed = capture::fail_batch(
            &mut claim_tx,
            &batch,
            &config.lease_owner,
            "session_write_denied",
        )
        .await?;
        append_batch_event(&mut claim_tx, &failed, &session_decision, "failed", 0, 0).await?;
        claim_tx.commit().await.map_err(commit_error)?;
        metrics::counter!(CAPTURE_BATCHES_TOTAL, "outcome" => "denied").increment(1);
        return Ok(ProcessOutcome::FailedAttempt);
    }
    claim_tx.commit().await.map_err(commit_error)?;
    let mut lease = match LeaseGuard::start(deps, config, &batch).await {
        Ok(lease) => lease,
        Err(Error::Conflict { .. }) => return Ok(record_abandoned_attempt(&batch)),
        Err(error) => return Err(error),
    };

    let mut proposed = Vec::new();
    let mut method = deps.extractor.method().to_owned();
    let mut model_versions = BTreeSet::new();
    for event in &events {
        let input = ExtractionInput {
            event_id: event.id,
            tenant_id,
            scope_id: batch.scope_id,
            session_id,
            principal_id: batch.principal_id.clone(),
            event_type: event.event_type,
            payload: event.payload.clone(),
            occurred_at: event.occurred_at,
            redactions: event.redactions.clone(),
        };
        let started = std::time::Instant::now();
        let outcome = tokio::select! {
            biased;
            _ = lease.wait_for_loss() => None,
            outcome = deps.extractor.extract(&input) => Some(outcome),
        };
        metrics::histogram!(CAPTURE_EXTRACTOR_SECONDS, "method" => method.clone())
            .record(started.elapsed().as_secs_f64());
        let Some(outcome) = outcome else {
            return abandon_attempt(lease, &batch).await;
        };
        let outcome = match outcome {
            Ok(outcome) => {
                metrics::counter!(
                    CAPTURE_EXTRACTOR_REQUESTS_TOTAL,
                    "method" => method.clone(),
                    "outcome" => "ok"
                )
                .increment(1);
                outcome
            }
            Err(error) => {
                metrics::counter!(
                    CAPTURE_EXTRACTOR_REQUESTS_TOTAL,
                    "method" => method.clone(),
                    "outcome" => "error"
                )
                .increment(1);
                return fail_attempt_with_lease(
                    deps,
                    config,
                    &batch,
                    "extractor_failed",
                    error,
                    lease,
                )
                .await;
            }
        };
        method = outcome.method.clone();
        model_versions.insert(outcome.model_version.clone());
        if outcome.candidates.len() > synveda_types::capture::MAX_CANDIDATES_PER_EVENT {
            return fail_attempt_with_lease(
                deps,
                config,
                &batch,
                "too_many_candidates",
                Error::Invalid {
                    message: "extractor returned too many candidates".to_owned(),
                },
                lease,
            )
            .await;
        }
        for candidate in outcome.candidates {
            if proposed.len()
                >= usize::try_from(runtime_configuration.capture.maximum_candidates_per_batch)
                    .unwrap_or(usize::MAX)
            {
                break;
            }
            let ordinal = match i32::try_from(proposed.len() + 1) {
                Ok(ordinal) => ordinal,
                Err(_) => {
                    return fail_attempt_with_lease(
                        deps,
                        config,
                        &batch,
                        "candidate_limit",
                        Error::Internal {
                            message: "capture candidate ordinal overflow".to_owned(),
                        },
                        lease,
                    )
                    .await;
                }
            };
            let prepared = match prepare_candidate(
                &batch,
                event,
                ordinal,
                &outcome.method,
                &outcome.model_version,
                candidate,
            ) {
                Ok(prepared) => prepared,
                Err(error) => {
                    return fail_attempt_with_lease(
                        deps,
                        config,
                        &batch,
                        "candidate_invalid",
                        error,
                        lease,
                    )
                    .await;
                }
            };
            if prepared.content.confidence_permille
                >= i32::from(runtime_configuration.capture.minimum_confidence_permille)
            {
                proposed.push(prepared);
            }
        }
    }
    if events.is_empty() {
        method = "no-eligible-events".to_owned();
        model_versions.insert("none@0".to_owned());
    }
    let model_version = if model_versions.len() == 1 {
        model_versions
            .into_iter()
            .next()
            .unwrap_or_else(|| "none@0".to_owned())
    } else {
        let digest = blake3::hash(
            model_versions
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join("\0")
                .as_bytes(),
        );
        format!("mixed:{}", digest.to_hex())
    };

    if lease.is_lost() {
        return abandon_attempt(lease, &batch).await;
    }

    let mut write = rls::begin_tenant_tx(&deps.pool, tenant_id).await?;
    let fresh_session_decision = decide_exact(
        deps,
        &mut write,
        tenant_id,
        &batch.principal_id,
        batch.scope_id,
        batch.project_id,
        Action::SessionWrite,
        Resource::Session(session_id),
        vec![ResourceEntity::Session {
            id: session_id,
            scope_id: batch.scope_id,
        }],
        None,
    )
    .await?;
    if !fresh_session_decision.allowed {
        let failed = capture::fail_batch(
            &mut write,
            &batch,
            &config.lease_owner,
            "session_write_revoked",
        )
        .await?;
        append_batch_event(&mut write, &failed, &fresh_session_decision, "failed", 0, 0).await?;
        write.commit().await.map_err(commit_error)?;
        lease.stop().await;
        return Ok(ProcessOutcome::FailedAttempt);
    }
    // Preferences are personal by default. Their evidence remains governed by
    // the session's scope when it becomes a KnowledgeSource, but the proposed
    // Knowledge itself belongs to the principal scope. This is placement as
    // identity (ADR-0074), not a UI convention that a later caller may forget.
    let identity = identities::by_subject(&mut *write, tenant_id, &batch.principal_id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("session principal {:?}", batch.principal_id),
        })?;
    for candidate in &mut proposed {
        if candidate.knowledge_type == synveda_types::knowledge::KnowledgeType::Preference {
            candidate.proposed_scope_id = identity.scope_id;
            candidate.proposed_project_id = None;
            candidate.proposed_owner_principal_id = Some(batch.principal_id.clone());
        }
    }
    let mut visible_match_count = 0usize;
    for candidate in &mut proposed {
        candidate.matches = visible_matches(deps, &mut write, &batch, candidate).await?;
        visible_match_count += candidate.matches.len();
    }
    let completed = capture::complete_batch(
        &mut write,
        &batch,
        &config.lease_owner,
        &method,
        &model_version,
        &proposed,
    )
    .await?;
    let mut conflict_sets = Vec::new();
    for candidate in &proposed {
        let Some(classification) = dominant_classification(&candidate.matches) else {
            continue;
        };
        let matches = candidate
            .matches
            .iter()
            .map(|matched| knowledge_conflicts::MatchedRevision {
                item_id: matched.knowledge_item_id,
                revision_id: matched.knowledge_revision_id,
                classification: capture_classification(matched.kind),
                similarity_permille: matched.similarity_permille,
                reason_code: matched.reason_code.clone(),
            })
            .collect::<Vec<_>>();
        let set = knowledge_conflicts::create(
            &mut write,
            &knowledge_conflicts::NewConflictSet {
                id: synveda_types::ConflictSetId::new(),
                tenant_id,
                scope_id: candidate.proposed_scope_id,
                project_id: candidate.proposed_project_id,
                classification,
                challenger_item_id: None,
                challenger_revision_id: None,
                capture_candidate_id: Some(candidate.id),
                matches: &matches,
                created_by: &batch.principal_id,
            },
        )
        .await?;
        conflict_sets.push(set.id);
    }
    append_batch_event(
        &mut write,
        &completed,
        &fresh_session_decision,
        "completed",
        proposed.len(),
        visible_match_count,
    )
    .await?;
    for conflict_set_id in &conflict_sets {
        synveda_audit::append(
            &mut write,
            tenant_id,
            &AuditEvent {
                occurred_at: Utc::now(),
                actor: Actor::system(ACTOR_COMPONENT),
                action: AuditAction::KnowledgeConflictOpened,
                resource: format!("knowledge-conflict:{conflict_set_id}"),
                outcome: Outcome::Success,
                payload: json!({
                    "conflict_set_id": conflict_set_id,
                    "capture_batch_id": batch.id,
                    "source": "capture",
                }),
                trace_id: None,
            },
        )
        .await?;
    }
    write.commit().await.map_err(commit_error)?;
    lease.stop().await;
    for candidate in &proposed {
        metrics::counter!(
            CAPTURE_CANDIDATES_TOTAL,
            "knowledge_type" => candidate.knowledge_type.as_str()
        )
        .increment(1);
    }
    metrics::counter!(CAPTURE_BATCHES_TOTAL, "outcome" => "completed").increment(1);
    Ok(ProcessOutcome::Completed)
}

async fn abandon_attempt(lease: LeaseGuard, batch: &CaptureBatch) -> Result<ProcessOutcome> {
    lease.stop().await;
    Ok(record_abandoned_attempt(batch))
}

fn record_abandoned_attempt(batch: &CaptureBatch) -> ProcessOutcome {
    tracing::warn!(
        batch.id = %batch.id,
        batch.attempt = batch.attempts,
        "capture attempt abandoned after lease ownership was lost"
    );
    metrics::counter!(CAPTURE_BATCHES_TOTAL, "outcome" => "lease_lost").increment(1);
    ProcessOutcome::Abandoned
}

async fn exact_configuration(
    connection: &mut sqlx::PgConnection,
    batch: &CaptureBatch,
) -> Result<ConfigurationDocument> {
    let document = if let Some(version_id) = batch.configuration_version_id {
        let version = configuration::version(connection, batch.tenant_id, version_id)
            .await?
            .ok_or_else(|| Error::Internal {
                message: format!(
                    "capture batch {} references missing configuration version {version_id}",
                    batch.id
                ),
            })?;
        version.document
    } else {
        ConfigurationDocument::fail_safe()
    };
    let actual_hash = document.content_hash()?;
    if actual_hash != batch.configuration_hash {
        return Err(Error::Internal {
            message: format!(
                "capture batch {} configuration hash does not match its frozen document",
                batch.id
            ),
        });
    }
    Ok(document)
}

fn extractor_provider(method: &str) -> Option<ExternalProvider> {
    match method {
        "claude-api" => Some(ExternalProvider::Anthropic),
        "vllm" => Some(ExternalProvider::Vllm),
        _ => None,
    }
}

fn prepare_candidate(
    batch: &CaptureBatch,
    event: &capture::FrozenEvent,
    ordinal: i32,
    method: &str,
    model_version: &str,
    candidate: crate::extraction::CandidateKnowledge,
) -> Result<NewCaptureCandidate> {
    let scan = crate::scan(json!({
        "title": candidate.title,
        "body_markdown": candidate.body_markdown,
        "summary": candidate.summary,
    }));
    let object = scan.payload.as_object().ok_or_else(|| Error::Internal {
        message: "candidate rescan changed an object into another JSON shape".to_owned(),
    })?;
    let field = |name: &str| {
        object
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| Error::Invalid {
                message: format!("extractor candidate has no string {name}"),
            })
    };
    let sensitivity = candidate
        .sensitivity
        .unwrap_or(Sensitivity::WORKING)
        .clamp(Sensitivity::WORKING, Sensitivity::MAX_DERIVED);
    let content = KnowledgeRevisionContent {
        title: field("title")?,
        body_markdown: field("body_markdown")?,
        summary: field("summary")?,
        tags: normalise_knowledge_tags(&candidate.tags)?,
        sensitivity,
        confidence_permille: (candidate.confidence.clamp(0.0, 1.0) * 1_000.0).round() as i32,
        valid_from: event.occurred_at,
        valid_to: None,
        stale_after: None,
        verification_metadata: json!({}),
        metadata: json!({
            "capture": {
                "method": method,
                "model_version": model_version,
                "source_event_id": event.id,
                "redaction_findings": scan.findings.iter().map(|finding| json!({
                    "rule": finding.rule,
                    "category": finding.category.as_str(),
                    "count": finding.count,
                })).collect::<Vec<_>>(),
            },
            "entities": candidate.entities,
        }),
    };
    validate_knowledge_revision_content(&content)?;
    let content_hash = knowledge::revision_content_hash(&content);
    Ok(NewCaptureCandidate {
        id: synveda_types::CaptureCandidateId::new(),
        ordinal,
        proposed_scope_id: batch.scope_id,
        proposed_project_id: batch.project_id,
        proposed_owner_principal_id: if candidate.knowledge_type
            == synveda_types::knowledge::KnowledgeType::Preference
        {
            Some(batch.principal_id.clone())
        } else {
            None
        },
        knowledge_type: candidate.knowledge_type,
        origin: candidate.origin,
        content,
        content_hash,
        source_event_ids: vec![event.id],
        matches: Vec::new(),
    })
}

async fn visible_matches(
    deps: &Deps,
    tx: &mut sqlx::PgConnection,
    batch: &CaptureBatch,
    proposed: &NewCaptureCandidate,
) -> Result<Vec<CaptureMatch>> {
    let personal = proposed.proposed_owner_principal_id.is_some();
    let filters = Filters {
        scope_ids: Vec::new(),
        workspace_id: (!personal).then_some(batch.workspace_id),
        project_id: proposed.proposed_project_id,
        scope_id: Some(proposed.proposed_scope_id),
        owner_principal_id: proposed.proposed_owner_principal_id.clone(),
        knowledge_type: None,
        origin: None,
        lifecycle: Some(KnowledgeLifecycleState::Active),
        tag: None,
        source_type: None,
        updated_from: None,
        updated_before: None,
        stale: None,
        at: Utc::now(),
        as_known_at: Utc::now(),
        include_history: false,
        include_transitional: false,
    };
    let query = format!("{} {}", proposed.content.title, proposed.content.summary);
    let candidates = knowledge_search::lexical_candidates(
        tx,
        batch.tenant_id,
        &filters,
        &query,
        synveda_types::capture::MAX_CAPTURE_MATCHES as i64,
    )
    .await?;
    let mut visible = Vec::new();
    for candidate in candidates {
        let Some(existing) =
            knowledge::current(&mut *tx, batch.tenant_id, candidate.item_id).await?
        else {
            continue;
        };
        let decision = decide_exact(
            deps,
            tx,
            batch.tenant_id,
            &batch.principal_id,
            existing.item.scope_id,
            existing.item.project_id,
            Action::KnowledgeRead,
            Resource::KnowledgeItem(existing.item.id),
            vec![ResourceEntity::KnowledgeItem {
                id: existing.item.id,
                scope_id: existing.item.scope_id,
            }],
            Some(existing.revision.content.sensitivity),
        )
        .await?;
        if !decision.allowed {
            continue;
        }
        if let Some(matched) = classify_match(proposed, &existing) {
            visible.push(matched);
        }
    }
    visible.sort_by_key(|matched| std::cmp::Reverse(matched.similarity_permille));
    visible.truncate(synveda_types::capture::MAX_CAPTURE_MATCHES);
    Ok(visible)
}

fn classify_match(
    proposed: &NewCaptureCandidate,
    existing: &KnowledgeSnapshot,
) -> Option<CaptureMatch> {
    let matched = classify_knowledge_match(
        proposed.knowledge_type,
        &proposed.content,
        &proposed.content_hash,
        existing.item.knowledge_type,
        &existing.revision.content,
        &existing.revision.content_hash,
        Utc::now(),
    )?;
    Some(CaptureMatch {
        knowledge_item_id: existing.item.id,
        knowledge_revision_id: existing.revision.id,
        kind: match matched.classification {
            ConflictClassification::Duplicate => CaptureMatchKind::Duplicate,
            ConflictClassification::Support => CaptureMatchKind::Support,
            ConflictClassification::Contradiction => CaptureMatchKind::Contradiction,
            ConflictClassification::Supersession => CaptureMatchKind::Supersession,
            ConflictClassification::Transition => CaptureMatchKind::Transition,
        },
        similarity_permille: matched.similarity_permille,
        reason_code: matched.reason_code.to_owned(),
    })
}

const fn capture_classification(kind: CaptureMatchKind) -> ConflictClassification {
    match kind {
        CaptureMatchKind::Duplicate => ConflictClassification::Duplicate,
        CaptureMatchKind::Support => ConflictClassification::Support,
        CaptureMatchKind::Contradiction => ConflictClassification::Contradiction,
        CaptureMatchKind::Supersession => ConflictClassification::Supersession,
        CaptureMatchKind::Transition => ConflictClassification::Transition,
    }
}

fn dominant_classification(matches: &[CaptureMatch]) -> Option<ConflictClassification> {
    matches
        .iter()
        .map(|matched| capture_classification(matched.kind))
        .max_by_key(|classification| match classification {
            ConflictClassification::Contradiction => 5,
            ConflictClassification::Transition => 4,
            ConflictClassification::Supersession => 3,
            ConflictClassification::Duplicate => 2,
            ConflictClassification::Support => 1,
        })
}

#[allow(clippy::too_many_arguments)]
async fn decide_exact(
    deps: &Deps,
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    subject: &str,
    scope_id: ScopeId,
    project_id: Option<synveda_types::ProjectId>,
    action: Action,
    resource: Resource,
    resources: Vec<ResourceEntity>,
    sensitivity: Option<Sensitivity>,
) -> Result<AuthzDecision> {
    let identity = identities::by_subject(&mut *tx, tenant_id, subject)
        .await?
        .ok_or_else(|| Error::PolicyDenied {
            action: action.as_str().to_owned(),
            resource: resource.to_string(),
            reason: "the session principal has no current identity".to_owned(),
        })?;
    if identity.status != IdentityStatus::Active {
        return Err(Error::PolicyDenied {
            action: action.as_str().to_owned(),
            resource: resource.to_string(),
            reason: format!("the session principal is {}", identity.status),
        });
    }
    let principal_chain: Vec<ScopeNode> = scope_chain(tx, tenant_id, identity.scope_id).await?;
    let chain: Vec<ScopeNode> = scope_chain(tx, tenant_id, scope_id).await?;
    let token_scope = if identity.kind == IdentityKind::Service {
        Some(
            principal_chain
                .get(1)
                .ok_or_else(|| Error::PolicyDenied {
                    action: action.as_str().to_owned(),
                    resource: resource.to_string(),
                    reason: "the service identity has no anchor".to_owned(),
                })?
                .id,
        )
    } else {
        None
    };
    let principal = Principal {
        tenant_id,
        subject: subject.to_owned(),
        quarantined: false,
        scope_id: Some(identity.scope_id),
        token_scope,
    };
    let chain_ids: Vec<ScopeId> = chain.iter().map(|node| node.id).collect();
    let assignments = policy_assignments::for_scopes(tx, tenant_id, &chain_ids).await?;
    let default_pack = policy_assignments::default_pack(tx, tenant_id).await?;
    let selection = project_id.map_or_else(
        anchors::AnchorSelection::none,
        anchors::AnchorSelection::project,
    );
    let anchor_set =
        anchors::resolve(&mut *tx, tenant_id, subject, Some(identity.id), selection).await?;
    let groups = anchors::groups_of(&mut *tx, tenant_id, Some(identity.id)).await?;
    let context = AuthzContext {
        scopes: &chain,
        principal_scopes: &principal_chain,
        anchors: anchor_set.as_slice(),
        groups: &groups,
        resources: &resources,
        assignments: &assignments,
        default_pack: default_pack.as_deref(),
        sensitivity,
        relaxations: &[],
    };
    deps.pdp.authorize(&principal, action, resource, &context)
}

async fn fail_attempt(
    deps: &Deps,
    config: &Config,
    batch: &CaptureBatch,
    code: &'static str,
    error: Error,
) -> Result<ProcessOutcome> {
    tracing::warn!(batch.id = %batch.id, error.code = code, %error, "capture attempt failed");
    let mut tx = rls::begin_tenant_tx(&deps.pool, batch.tenant_id).await?;
    let failed = capture::fail_batch(&mut tx, batch, &config.lease_owner, code).await?;
    // The original allowed decision is not cached here; a content-free
    // synthetic denial-free marker records only the processing outcome. The
    // next attempt reauthorises before doing work.
    append_system_failure(&mut tx, &failed, code).await?;
    tx.commit().await.map_err(commit_error)?;
    metrics::counter!(CAPTURE_BATCHES_TOTAL, "outcome" => "failed_attempt").increment(1);
    Ok(ProcessOutcome::FailedAttempt)
}

async fn fail_attempt_with_lease(
    deps: &Deps,
    config: &Config,
    batch: &CaptureBatch,
    code: &'static str,
    error: Error,
    lease: LeaseGuard,
) -> Result<ProcessOutcome> {
    let result = fail_attempt(deps, config, batch, code, error).await;
    lease.stop().await;
    result
}

fn capture_session_id(batch: &CaptureBatch) -> Result<SessionId> {
    if batch.source_kind != CaptureSourceKind::Session || batch.import_job_id.is_some() {
        return Err(Error::Internal {
            message: format!(
                "session extraction worker claimed non-session batch {}",
                batch.id
            ),
        });
    }
    batch.session_id.ok_or_else(|| Error::Internal {
        message: format!("session capture batch {} has no session", batch.id),
    })
}

async fn append_batch_event(
    tx: &mut sqlx::PgConnection,
    batch: &CaptureBatch,
    decision: &AuthzDecision,
    state: &str,
    candidate_count: usize,
    visible_match_count: usize,
) -> Result<()> {
    let session_id = capture_session_id(batch)?;
    synveda_audit::append(
        tx,
        batch.tenant_id,
        &AuditEvent {
            occurred_at: Utc::now(),
            actor: Actor::system(ACTOR_COMPONENT),
            action: AuditAction::CaptureBatchCompleted,
            resource: Resource::Session(session_id).to_string(),
            outcome: if state == "completed" {
                Outcome::Success
            } else {
                Outcome::Failure
            },
            payload: json!({
                "batch_id": batch.id,
                "session_id": session_id,
                "input_hash": batch.input_hash,
                "configuration_version_id": batch.configuration_version_id,
                "configuration_hash": batch.configuration_hash,
                "event_count": batch.event_count,
                "state": state,
                "attempts": batch.attempts,
                "candidate_count": candidate_count,
                "visible_match_count": visible_match_count,
                "extractor_method": batch.extractor_method,
                "model_version": batch.model_version,
                "error_code": batch.error_code,
                "authz": {
                    "action": Action::SessionWrite.as_str(),
                    "pack": decision.pack_name,
                    "pack_version": decision.pack_version,
                    "allowed": decision.allowed,
                    "determining": decision.determining,
                },
            }),
            trace_id: None,
        },
    )
    .await
    .map(|_| ())
}

async fn append_system_failure(
    tx: &mut sqlx::PgConnection,
    batch: &CaptureBatch,
    code: &str,
) -> Result<()> {
    let session_id = capture_session_id(batch)?;
    synveda_audit::append(
        tx,
        batch.tenant_id,
        &AuditEvent {
            occurred_at: Utc::now(),
            actor: Actor::system(ACTOR_COMPONENT),
            action: AuditAction::CaptureBatchCompleted,
            resource: Resource::Session(session_id).to_string(),
            outcome: Outcome::Failure,
            payload: json!({
                "batch_id": batch.id,
                "session_id": session_id,
                "input_hash": batch.input_hash,
                "configuration_version_id": batch.configuration_version_id,
                "configuration_hash": batch.configuration_hash,
                "event_count": batch.event_count,
                "state": batch.state.as_str(),
                "attempts": batch.attempts,
                "candidate_count": 0,
                "visible_match_count": 0,
                "error_code": code,
            }),
            trace_id: None,
        },
    )
    .await
    .map(|_| ())
}

fn commit_error(error: sqlx::Error) -> Error {
    Error::Storage {
        message: format!("commit capture transaction: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synveda_types::knowledge::{KnowledgeOrigin, KnowledgeType};
    use synveda_types::{KnowledgeItemId, KnowledgeRevisionId};
    use tokio::sync::Notify;

    struct PendingRenewalDrop(Option<oneshot::Sender<()>>);

    impl Drop for PendingRenewalDrop {
        fn drop(&mut self) {
            if let Some(dropped) = self.0.take() {
                let _ = dropped.send(());
            }
        }
    }

    fn candidate(body: &str) -> NewCaptureCandidate {
        let content = KnowledgeRevisionContent {
            title: "Request headers".to_owned(),
            body_markdown: body.to_owned(),
            summary: body.to_owned(),
            tags: Vec::new(),
            sensitivity: Sensitivity::Internal,
            confidence_permille: 900,
            valid_from: Utc::now(),
            valid_to: None,
            stale_after: None,
            verification_metadata: json!({}),
            metadata: json!({}),
        };
        NewCaptureCandidate {
            id: synveda_types::CaptureCandidateId::new(),
            ordinal: 1,
            proposed_scope_id: ScopeId::new(),
            proposed_project_id: None,
            proposed_owner_principal_id: None,
            knowledge_type: KnowledgeType::Convention,
            origin: KnowledgeOrigin::Observed,
            content_hash: knowledge::revision_content_hash(&content),
            content,
            source_event_ids: vec![synveda_types::SessionEventId::new()],
            matches: Vec::new(),
        }
    }

    #[test]
    fn deterministic_matcher_distinguishes_duplicate_conflict_and_weak_overlap() {
        let proposed = candidate("Public requests never use X-Request-Id.");
        let mut existing = candidate("Public requests use X-Request-Id.");
        let snapshot = KnowledgeSnapshot {
            item: synveda_types::knowledge::KnowledgeItem {
                id: KnowledgeItemId::new(),
                tenant_id: TenantId::new(),
                scope_id: ScopeId::new(),
                project_id: None,
                owner_principal_id: None,
                knowledge_type: KnowledgeType::Convention,
                origin: KnowledgeOrigin::Authored,
                lifecycle_state: KnowledgeLifecycleState::Active,
                current_revision_id: KnowledgeRevisionId::new(),
                created_by: None,
                updated_by: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                transaction_from: Utc::now(),
            },
            revision: synveda_types::knowledge::KnowledgeRevision {
                id: KnowledgeRevisionId::new(),
                tenant_id: TenantId::new(),
                knowledge_item_id: KnowledgeItemId::new(),
                revision_number: 1,
                content: existing.content.clone(),
                content_hash: existing.content_hash.clone(),
                created_by: None,
                transaction_time: Utc::now(),
            },
            transaction_to: None,
        };
        assert_eq!(
            classify_match(&proposed, &snapshot).map(|matched| matched.kind),
            Some(CaptureMatchKind::Contradiction)
        );
        existing.content = proposed.content.clone();
        existing.content_hash = proposed.content_hash.clone();
        let mut exact = snapshot;
        exact.revision.content = existing.content;
        exact.revision.content_hash = existing.content_hash;
        assert_eq!(
            classify_match(&proposed, &exact).map(|matched| matched.kind),
            Some(CaptureMatchKind::Duplicate)
        );
    }

    #[test]
    fn defaults_bound_work_and_lease_time() {
        let config = Config::default();
        assert!(config.batches_per_tenant > 0);
        assert!(config.lease_duration <= Duration::from_secs(3_600));
    }

    #[tokio::test]
    async fn stopping_a_guard_cancels_a_blocked_renewal() {
        let entered = Arc::new(Notify::new());
        let (dropped_tx, dropped_rx) = oneshot::channel();
        let mut dropped_tx = Some(dropped_tx);
        let guard =
            LeaseGuard::spawn_with_renewal(Duration::from_millis(1), Duration::from_secs(60), {
                let entered = Arc::clone(&entered);
                move || {
                    let entered = Arc::clone(&entered);
                    let dropped = PendingRenewalDrop(dropped_tx.take());
                    async move {
                        entered.notify_one();
                        let _dropped = dropped;
                        std::future::pending::<Result<()>>().await
                    }
                }
            });

        tokio::time::timeout(Duration::from_secs(1), entered.notified())
            .await
            .expect("renewal entered its blocked future");
        tokio::time::timeout(Duration::from_secs(1), guard.stop())
            .await
            .expect("guard stop remains bounded");
        tokio::time::timeout(Duration::from_secs(1), dropped_rx)
            .await
            .expect("blocked renewal future is cancelled")
            .expect("drop signal is delivered");
    }

    #[tokio::test]
    async fn a_stalled_renewal_marks_the_claim_lost_within_its_bound() {
        let entered = Arc::new(Notify::new());
        let (dropped_tx, dropped_rx) = oneshot::channel();
        let mut dropped_tx = Some(dropped_tx);
        let mut guard =
            LeaseGuard::spawn_with_renewal(Duration::from_millis(1), Duration::from_millis(25), {
                let entered = Arc::clone(&entered);
                move || {
                    let entered = Arc::clone(&entered);
                    let dropped = PendingRenewalDrop(dropped_tx.take());
                    async move {
                        entered.notify_one();
                        let _dropped = dropped;
                        std::future::pending::<Result<()>>().await
                    }
                }
            });

        tokio::time::timeout(Duration::from_secs(1), entered.notified())
            .await
            .expect("renewal entered its stalled future");
        tokio::time::timeout(Duration::from_secs(1), guard.wait_for_loss())
            .await
            .expect("stalled renewal marks the lease lost");
        assert!(guard.is_lost());
        tokio::time::timeout(Duration::from_secs(1), dropped_rx)
            .await
            .expect("timed-out renewal future is cancelled")
            .expect("drop signal is delivered");
        guard.stop().await;
    }
}
