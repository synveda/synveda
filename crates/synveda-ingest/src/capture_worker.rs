//! Restart-safe capture extraction worker (CPR-18, ADR-0083).
//!
//! A database lease, not a PGMQ event signal, is the work address. The worker
//! reads one frozen batch under tenant RLS, re-decides `SessionWrite` as the
//! principal that opened the run, calls the configured extractor outside any
//! transaction, then re-decides each exact current Knowledge neighbour before
//! persisting a match. Its only domain output is reviewable candidates.

use std::collections::BTreeSet;
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
use synveda_store::{anchors, identities, knowledge, policy_assignments, rls, tenants};
use synveda_types::capture::{CaptureBatch, CaptureMatch, CaptureMatchKind, CaptureSourceKind};
use synveda_types::knowledge::{
    KnowledgeLifecycleState, KnowledgeRevisionContent, normalise_knowledge_tags,
    validate_knowledge_revision_content,
};
use synveda_types::{
    Error, IdentityKind, IdentityStatus, Result, ScopeId, Sensitivity, SessionId, TenantId,
};

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
const DEFAULT_LEASE_OWNER: &str = "capture-worker";

/// Background pacing and lease bound.
#[derive(Debug, Clone)]
pub struct Config {
    /// Delay between complete tenant sweeps.
    pub poll_interval: Duration,
    /// Lease retained while one model pass runs.
    pub lease_duration: Duration,
    /// Claimed batches per tenant per sweep.
    pub batches_per_tenant: usize,
    /// Stable worker identity recorded on the lease.
    pub lease_owner: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(1),
            lease_duration: Duration::from_secs(60),
            batches_per_tenant: 8,
            lease_owner: DEFAULT_LEASE_OWNER.to_owned(),
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
}

/// Starts one immediate pass and then one per configured interval.
#[must_use]
pub fn spawn(deps: Deps, config: Config) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(config.poll_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(error) = sweep_once(&deps, &config).await {
                tracing::warn!(%error, "capture sweep failed; retrying");
            }
        }
    })
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
}

/// Processes at most one batch for a tenant. Public behavior is exposed by
/// [`sweep_once`]; keeping this function private prevents callers from
/// inventing a second claim discipline.
async fn process_one(deps: &Deps, config: &Config, tenant_id: TenantId) -> Result<ProcessOutcome> {
    let lease_seconds = i64::try_from(config.lease_duration.as_secs())
        .unwrap_or(i64::MAX)
        .clamp(1, 3_600);
    let mut claim_tx = rls::begin_tenant_tx(&deps.pool, tenant_id).await?;
    let Some(batch) =
        capture::claim_batch(&mut claim_tx, tenant_id, &config.lease_owner, lease_seconds).await?
    else {
        claim_tx.rollback().await.map_err(commit_error)?;
        return Ok(ProcessOutcome::Empty);
    };
    let session_id = capture_session_id(&batch)?;
    let events = capture::frozen_events(&mut *claim_tx, tenant_id, batch.id).await?;
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
        let outcome = deps.extractor.extract(&input).await;
        metrics::histogram!(CAPTURE_EXTRACTOR_SECONDS, "method" => method.clone())
            .record(started.elapsed().as_secs_f64());
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
                return fail_attempt(deps, config, &batch, "extractor_failed", error).await;
            }
        };
        method = outcome.method.clone();
        model_versions.insert(outcome.model_version.clone());
        if outcome.candidates.len() > synveda_types::capture::MAX_CANDIDATES_PER_EVENT {
            return fail_attempt(
                deps,
                config,
                &batch,
                "too_many_candidates",
                Error::Invalid {
                    message: "extractor returned too many candidates".to_owned(),
                },
            )
            .await;
        }
        for candidate in outcome.candidates {
            let ordinal = match i32::try_from(proposed.len() + 1) {
                Ok(ordinal) => ordinal,
                Err(_) => {
                    return fail_attempt(
                        deps,
                        config,
                        &batch,
                        "candidate_limit",
                        Error::Internal {
                            message: "capture candidate ordinal overflow".to_owned(),
                        },
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
                    return fail_attempt(deps, config, &batch, "candidate_invalid", error).await;
                }
            };
            proposed.push(prepared);
        }
    }
    if events.is_empty() {
        method = "no-eligible-events".to_owned();
        model_versions.insert("none@0".to_owned());
    }
    let model_version = if model_versions.len() == 1 {
        model_versions.into_iter().next().expect("one model")
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
    append_batch_event(
        &mut write,
        &completed,
        &fresh_session_decision,
        "completed",
        proposed.len(),
        visible_match_count,
    )
    .await?;
    write.commit().await.map_err(commit_error)?;
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
    if proposed.content_hash == existing.revision.content_hash {
        return Some(CaptureMatch {
            knowledge_item_id: existing.item.id,
            knowledge_revision_id: existing.revision.id,
            kind: CaptureMatchKind::Duplicate,
            similarity_permille: 1_000,
            reason_code: "same_content_hash".to_owned(),
        });
    }
    let proposed_tokens = tokens(&format!(
        "{} {} {}",
        proposed.content.title, proposed.content.summary, proposed.content.body_markdown
    ));
    let existing_tokens = tokens(&format!(
        "{} {} {}",
        existing.revision.content.title,
        existing.revision.content.summary,
        existing.revision.content.body_markdown
    ));
    if proposed_tokens.is_empty() || existing_tokens.is_empty() {
        return None;
    }
    let intersection = proposed_tokens.intersection(&existing_tokens).count();
    let union = proposed_tokens.union(&existing_tokens).count();
    let similarity = i32::try_from(intersection * 1_000 / union).unwrap_or(1_000);
    let proposed_negated = has_negation(&proposed.content.body_markdown);
    let existing_negated = has_negation(&existing.revision.content.body_markdown);
    let (kind, reason) = if similarity >= 850 {
        (CaptureMatchKind::Duplicate, "lexical_near_duplicate")
    } else if similarity >= 450
        && proposed.knowledge_type == existing.item.knowledge_type
        && proposed_negated != existing_negated
    {
        (
            CaptureMatchKind::Conflict,
            "shared_subject_opposite_polarity",
        )
    } else if similarity >= 550 && proposed.knowledge_type == existing.item.knowledge_type {
        (
            CaptureMatchKind::PossibleSupersession,
            "shared_subject_new_statement",
        )
    } else {
        return None;
    };
    Some(CaptureMatch {
        knowledge_item_id: existing.item.id,
        knowledge_revision_id: existing.revision.id,
        kind,
        similarity_permille: similarity,
        reason_code: reason.to_owned(),
    })
}

fn tokens(value: &str) -> BTreeSet<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| token.chars().count() >= 2)
        .map(str::to_lowercase)
        .collect()
}

fn has_negation(value: &str) -> bool {
    tokens(value)
        .iter()
        .any(|token| matches!(token.as_str(), "not" | "never" | "no" | "without"))
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
    let assignments = policy_assignments::for_scopes(&mut *tx, tenant_id, &chain_ids).await?;
    let default_pack = policy_assignments::default_pack(&mut *tx, tenant_id).await?;
    let selection = project_id.map_or_else(
        anchors::AnchorSelection::none,
        anchors::AnchorSelection::project,
    );
    let anchor_set = anchors::resolve(&mut *tx, tenant_id, subject, selection).await?;
    let groups = anchors::groups_of(&mut *tx, tenant_id, subject).await?;
    let context = AuthzContext {
        scopes: &chain,
        principal_scopes: &principal_chain,
        anchors: anchor_set.as_slice(),
        groups: &groups,
        resources: &resources,
        assignments: &assignments,
        default_pack: default_pack.as_deref(),
        sensitivity,
        lapses: &[],
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
        let exact_tokens = tokens("Public requests never use X-Request-Id.");
        assert!(exact_tokens.contains("public"));
        assert!(has_negation(&proposed.content.body_markdown));
        assert!(!has_negation("Public requests use X-Request-Id."));
        assert!(
            tokens("unrelated release procedure")
                .intersection(&exact_tokens)
                .count()
                == 0
        );
    }

    #[test]
    fn defaults_bound_work_and_lease_time() {
        let config = Config::default();
        assert!(config.batches_per_tenant > 0);
        assert!(config.lease_duration <= Duration::from_secs(3_600));
    }
}
