//! Governed Knowledge mutation commands (CPR-16, ADR-0081).
//!
//! Every command takes two independent controls before it can change data:
//! Cedar decides whether this principal may write/forget the exact scope or
//! aggregate, and VedaFlow opens a typed `Knowledge/apply` change whose live
//! approval matrix decides whether it applies now or waits. Even an auto-
//! applied personal change therefore has an immutable object, commit,
//! proposal row and hash-chained audit evidence.
//!
//! This is an application service, not yet an HTTP surface. CPR-17 maps the
//! public Knowledge API onto it; capture and import call the same functions.

use std::collections::HashSet;

use chrono::Utc;
use serde_json::{Value, json};
use sqlx::{Acquire, PgConnection};
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{Action, Resource, ResourceEntity};
use synveda_store::knowledge::{
    self as store, KnowledgeSnapshot, NewKnowledgeItem, NewKnowledgeRelation, NewKnowledgeRevision,
    NewKnowledgeSource,
};
use synveda_store::{
    anchors::AnchorSelection, capture as capture_store, configuration, knowledge_conflicts,
    knowledge_lifecycle, knowledge_search, projects, rls, scopes,
};
use synveda_types::json::canonicalise;
use synveda_types::knowledge::{
    ConflictClassification, ConflictMemberRole, ConflictResolutionKind, ConflictSetStatus,
    FreshnessPolicy, KnowledgeCommand, KnowledgeLifecycleState, KnowledgeMutationOutcome,
    KnowledgeMutationResult, KnowledgeRelationType, KnowledgeSourceDraft, classify_knowledge_match,
    validate_knowledge_revision_content, validate_knowledge_source,
};
use synveda_types::{
    ArtifactFamily, ArtifactReference, AssetKind, ConflictSetId, DurableOperationId, Error,
    IdentityId, KnowledgeItemId, KnowledgeRelationId, KnowledgeRevisionId, KnowledgeSourceId,
    ProposalEffect, ProposalId, ProposalState, Result, ScopeId, Sensitivity, TenantId,
};
use synveda_vedaflow::{self as vedaflow, PolicySnapshot, Signer};

use crate::app::AppState;
use crate::approvals::{self, Requested};
use crate::audit;
use crate::authz::{self, Authorized, DecisionInput};
use crate::idempotency::Claim;

/// Maximum length of an archive/restore/forget reason.
pub const MAX_KNOWLEDGE_REASON_CHARS: usize = 1_000;
/// Durable conflict-set transitions by source/outcome.
pub const KNOWLEDGE_CONFLICTS_TOTAL: &str = "synveda_knowledge_conflicts_total";

/// Open a governed Knowledge change and auto-apply it only when the active
/// approval matrix has no outstanding requirement.
///
/// # Errors
///
/// Validation, ownership, PDP, stale-revision, VedaFlow, persistence and audit
/// failures are returned without committing any partial state.
#[tracing::instrument(
    name = "knowledge.command",
    skip_all,
    fields(knowledge.command = %command.kind()),
    err(Display)
)]
pub async fn command(
    state: &AppState,
    command: KnowledgeCommand,
) -> Result<KnowledgeMutationResult> {
    command_inner(state, command, None).await
}

/// [`command`] with a creation idempotency claim recorded atomically beside
/// the VedaFlow proposal and its applied or pending effect.
///
/// The stored resource id is the change id, not the proposed item id: a replay
/// must return the same governance outcome even when the change is still
/// pending and the item therefore does not exist yet.
pub async fn command_idempotent(
    state: &AppState,
    command: KnowledgeCommand,
    claim: &Claim,
) -> Result<KnowledgeMutationResult> {
    command_inner(state, command, Some(claim)).await
}

async fn command_inner(
    state: &AppState,
    mut command: KnowledgeCommand,
    idempotency: Option<&Claim>,
) -> Result<KnowledgeMutationResult> {
    let tenant_id = ambient_tenant()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    apply_freshness_policy(&mut tx, tenant_id, &mut command).await?;
    validate_command(&command)?;
    let authorization = authorize_command(state, &mut tx, tenant_id, &command).await?;
    let actor = identity_of(&authorization.proposal_input)?;
    let actor_subject = authorization.proposal_input.principal.subject.clone();
    let payload_hash = command_payload_hash(&command)?;
    let artifact_references = artifact_references(&command, &payload_hash)?;
    let target_ids = command.target_item_ids();
    let manifest = canonicalise(&json!({
        "command": command.kind().as_str(),
        "payload_hash": payload_hash,
        "target_item_ids": target_ids,
    }));
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(|err| Error::Internal {
        message: format!("encode Knowledge change manifest: {err}"),
    })?;
    let object =
        vedaflow::put_object(&mut tx, tenant_id, AssetKind::Knowledge, &manifest_bytes).await?;
    let members = vec![("command".to_owned(), object.hash)];
    let snapshot = PolicySnapshot::new(
        authorization.proposal_allowed.decision.pack_name.clone(),
        authorization.proposal_allowed.decision.pack_version,
    );
    let title = format!("{} Knowledge", command.kind());
    let proposal = vedaflow::proposals::open(
        &mut tx,
        tenant_id,
        &vedaflow::NewProposal {
            target_scope: authorization.target.id,
            source_scope: authorization.target.id,
            asset: AssetKind::Knowledge,
            effect: ProposalEffect::Apply,
            members: &members,
            artifact_references: &artifact_references,
            sensitivity: authorization.sensitivity,
            title: &title,
            proposer: actor,
            proposer_subject: &actor_subject,
            committed_at: Utc::now(),
            policy_snapshot: &snapshot,
        },
        &Signer::Unsigned,
    )
    .await?;
    knowledge_lifecycle::insert_change(&mut tx, tenant_id, proposal.id, &command, &payload_hash)
        .await?;

    let entries = vec!["command".to_owned()];
    let requirement = approvals::resolve(
        state,
        &mut tx,
        tenant_id,
        &authorization.proposal_input,
        &Requested {
            target: &authorization.target,
            asset: AssetKind::Knowledge,
            sensitivity: authorization.sensitivity,
            entries: &entries,
        },
    )
    .await?;
    let outstanding = requirement.outstanding(&[]);
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::KnowledgeChangeOpened,
        Resource::Scope(authorization.target.id).to_string(),
        Outcome::Success,
        json!({
            "change_id": proposal.id,
            "command": command.kind().as_str(),
            "payload_hash": payload_hash,
            "manifest_hash": object.hash.to_hex(),
            "commit": proposal.commit.to_hex(),
            "artifact_references": &proposal.artifact_references,
            "target_item_ids": target_ids,
            "target_scope_id": authorization.target.id,
            "sensitivity": authorization.sensitivity.as_str(),
            "authz": audit::decision_context(Action::ProposalOpen, &authorization.proposal_allowed),
            "approvals": approvals::audit_context(&requirement, &outstanding),
        }),
    )
    .await?;

    let result = if outstanding.is_empty() {
        apply_loaded(
            &mut tx,
            &ApplyRequest {
                state,
                tenant_id,
                change_id: proposal.id,
                command: &command,
                payload_hash: &payload_hash,
                actor,
                actor_subject: &actor_subject,
                authorization: &authorization,
            },
        )
        .await?
    } else {
        if let KnowledgeCommand::ResolveConflict {
            conflict_set_id,
            expected_conflict_revision,
            resolution,
            ..
        } = &command
        {
            knowledge_conflicts::mark_pending(
                &mut tx,
                tenant_id,
                *conflict_set_id,
                *expected_conflict_revision,
                proposal.id,
                *resolution,
                &actor_subject,
            )
            .await?;
        }
        metrics::counter!(
            knowledge_lifecycle::KNOWLEDGE_LIFECYCLE_ACTS_TOTAL,
            "act" => "pending_review",
            "command" => command.kind().as_str()
        )
        .increment(1);
        KnowledgeMutationResult {
            change_id: proposal.id,
            outcome: KnowledgeMutationOutcome::PendingReview,
            knowledge_item_id: proposed_result_item(&command),
            revision_id: None,
            operation_id: None,
        }
    };
    if let Some(claim) = idempotency {
        claim
            .remember(&mut tx, tenant_id, proposal.id.as_uuid())
            .await?;
    }
    tx.commit().await.map_err(|err| Error::Storage {
        message: format!("commit Knowledge command: {err}"),
    })?;
    Ok(result)
}

/// Freeze the exact effective freshness evidence into every content-bearing
/// command before its payload is hashed and opened in VedaFlow. Explicit
/// `stale_after` remains authoritative; an absent date receives the governed
/// type interval when one exists.
async fn apply_freshness_policy(
    tx: &mut PgConnection,
    tenant_id: TenantId,
    command: &mut KnowledgeCommand,
) -> Result<()> {
    let target = match command {
        KnowledgeCommand::Create {
            scope_id,
            knowledge_type,
            ..
        }
        | KnowledgeCommand::Supersede {
            scope_id,
            knowledge_type,
            ..
        }
        | KnowledgeCommand::Merge {
            scope_id,
            knowledge_type,
            ..
        } => Some((*scope_id, *knowledge_type)),
        KnowledgeCommand::Edit { item_id, .. } => {
            let snapshot = store::current(&mut *tx, tenant_id, *item_id)
                .await?
                .ok_or_else(|| missing_item(*item_id))?;
            Some((snapshot.item.scope_id, snapshot.item.knowledge_type))
        }
        KnowledgeCommand::Verify { .. }
        | KnowledgeCommand::Archive { .. }
        | KnowledgeCommand::Restore { .. }
        | KnowledgeCommand::Forget { .. }
        | KnowledgeCommand::ResolveConflict { .. } => None,
    };
    let Some((scope_id, knowledge_type)) = target else {
        return Ok(());
    };
    let effective = configuration::effective_at_scope(tx, tenant_id, scope_id).await?;
    let policy = FreshnessPolicy::from_effective(&effective, knowledge_type);
    let content = match command {
        KnowledgeCommand::Create { content, .. }
        | KnowledgeCommand::Edit { content, .. }
        | KnowledgeCommand::Supersede { content, .. }
        | KnowledgeCommand::Merge { content, .. } => content,
        _ => return Ok(()),
    };
    apply_policy_to_content(content, &policy, Utc::now())
}

fn apply_policy_to_content(
    content: &mut synveda_types::knowledge::KnowledgeRevisionContent,
    policy: &FreshnessPolicy,
    basis: chrono::DateTime<Utc>,
) -> Result<()> {
    let explicit = content.stale_after.is_some();
    if !explicit && let Some(mut due) = policy.interval_due_at(basis) {
        if let Some(valid_to) = content.valid_to {
            due = due.min(valid_to);
        }
        if due > content.valid_from {
            content.stale_after = Some(due);
        }
    }
    let metadata = content
        .metadata
        .as_object_mut()
        .ok_or_else(|| Error::Invalid {
            message: "Knowledge metadata must be a JSON object".to_owned(),
        })?;
    metadata.insert(
        "synveda_freshness_policy".to_owned(),
        json!({
            "scope_id": policy.scope_id,
            "knowledge_type": policy.knowledge_type.as_str(),
            "default_days": policy.default_days,
            "triggers": policy.triggers.iter().map(|trigger| trigger.as_str()).collect::<Vec<_>>(),
            "configuration_artifact_id": policy.configuration_artifact_id,
            "configuration_binding_id": policy.configuration_binding_id,
            "configuration_version_id": policy.configuration_version_id,
            "configuration_hash": policy.configuration_hash,
            "due_source": if explicit { "explicit_revision" } else { "governed_default" },
        }),
    );
    validate_knowledge_revision_content(content)
}

#[allow(clippy::too_many_arguments)]
async fn classify_write(
    state: &AppState,
    tx: &mut PgConnection,
    tenant_id: TenantId,
    scope_id: ScopeId,
    project_id: Option<synveda_types::ProjectId>,
    owner_principal_id: Option<String>,
    knowledge_type: synveda_types::knowledge::KnowledgeType,
    content: &synveda_types::knowledge::KnowledgeRevisionContent,
    exclude_item_id: Option<KnowledgeItemId>,
) -> Result<Vec<knowledge_conflicts::MatchedRevision>> {
    let at = Utc::now();
    let filters = knowledge_search::Filters {
        scope_ids: Vec::new(),
        workspace_id: None,
        project_id,
        scope_id: Some(scope_id),
        owner_principal_id,
        knowledge_type: Some(knowledge_type),
        origin: None,
        lifecycle: Some(KnowledgeLifecycleState::Active),
        tag: None,
        source_type: None,
        updated_from: None,
        updated_before: None,
        stale: None,
        at,
        as_known_at: at,
        include_history: false,
        include_transitional: false,
    };
    let query = format!("{} {}", content.title, content.summary);
    let depth = knowledge_conflicts::MAX_CONFLICT_MEMBERS as i64 * 2;
    let mut candidates =
        knowledge_search::lexical_candidates(tx, tenant_id, &filters, &query, depth).await?;
    // A negation or renamed term can make the web-search expression narrower
    // than the actual shared subject. Complete the bounded pool with recent
    // same-type/scope heads, then run the one deterministic classifier over
    // both sources. This is still bounded and every row is independently
    // authorised below.
    let mut seen = candidates
        .iter()
        .map(|candidate| candidate.item_id)
        .collect::<HashSet<_>>();
    for candidate in knowledge_search::list_candidates(tx, tenant_id, &filters, None, depth).await?
    {
        if seen.insert(candidate.item_id) {
            candidates.push(candidate);
        }
    }
    let proposed_hash = store::revision_content_hash(content);
    let mut matches = Vec::new();
    for candidate in candidates {
        if Some(candidate.item_id) == exclude_item_id {
            continue;
        }
        let Some(existing) = store::current(&mut *tx, tenant_id, candidate.item_id).await? else {
            continue;
        };
        match crate::knowledge_api::authorize_snapshot(state, tx, tenant_id, &existing).await {
            Ok(_) => {}
            Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => continue,
            Err(error) => return Err(error),
        }
        let Some(matched) = classify_knowledge_match(
            knowledge_type,
            content,
            &proposed_hash,
            existing.item.knowledge_type,
            &existing.revision.content,
            &existing.revision.content_hash,
            at,
        ) else {
            continue;
        };
        matches.push(knowledge_conflicts::MatchedRevision {
            item_id: existing.item.id,
            revision_id: existing.revision.id,
            classification: matched.classification,
            similarity_permille: matched.similarity_permille,
            reason_code: matched.reason_code.to_owned(),
        });
    }
    matches.sort_by(|left, right| {
        right
            .similarity_permille
            .cmp(&left.similarity_permille)
            .then_with(|| left.item_id.cmp(&right.item_id))
    });
    matches.truncate(knowledge_conflicts::MAX_CONFLICT_MEMBERS - 1);
    Ok(matches)
}

fn dominant_classification(
    matches: &[knowledge_conflicts::MatchedRevision],
) -> ConflictClassification {
    matches
        .iter()
        .map(|matched| matched.classification)
        .max_by_key(|classification| match classification {
            ConflictClassification::Contradiction => 5,
            ConflictClassification::Transition => 4,
            ConflictClassification::Supersession => 3,
            ConflictClassification::Duplicate => 2,
            ConflictClassification::Support => 1,
        })
        .expect("a retained conflict has at least one match")
}

async fn retain_write_conflict(
    tx: &mut PgConnection,
    tenant_id: TenantId,
    change_id: ProposalId,
    mut challenger: KnowledgeSnapshot,
    matches: &[knowledge_conflicts::MatchedRevision],
    actor_subject: &str,
) -> Result<KnowledgeSnapshot> {
    let classification = dominant_classification(matches);
    if matches
        .iter()
        .any(|matched| matched.classification.requires_resolution())
    {
        challenger = store::set_lifecycle(
            tx,
            tenant_id,
            challenger.item.id,
            challenger.revision.id,
            KnowledgeLifecycleState::Transitional,
            Some(actor_subject),
        )
        .await?
        .ok_or_else(|| missing_item(challenger.item.id))?;
    }
    let set = knowledge_conflicts::create(
        tx,
        &knowledge_conflicts::NewConflictSet {
            id: ConflictSetId::new(),
            tenant_id,
            scope_id: challenger.item.scope_id,
            project_id: challenger.item.project_id,
            classification,
            challenger_item_id: Some(challenger.item.id),
            challenger_revision_id: Some(challenger.revision.id),
            capture_candidate_id: None,
            matches,
            created_by: actor_subject,
        },
    )
    .await?;
    let needs_resolution = matches
        .iter()
        .any(|matched| matched.classification.requires_resolution());
    audit::record(
        tx,
        tenant_id,
        AuditAction::KnowledgeConflictOpened,
        Resource::KnowledgeItem(challenger.item.id).to_string(),
        Outcome::Success,
        json!({
            "change_id": change_id,
            "conflict_set_id": set.id,
            "classification": classification.as_str(),
            "challenger_item_id": challenger.item.id,
            "challenger_revision_id": challenger.revision.id,
            "visible_member_count": matches.len() + 1,
            "transitional": challenger.item.lifecycle_state == KnowledgeLifecycleState::Transitional,
            "requires_resolution": needs_resolution,
        }),
    )
    .await?;
    if !needs_resolution {
        for matched in matches {
            store::add_relation(
                tx,
                &NewKnowledgeRelation {
                    id: KnowledgeRelationId::new(),
                    tenant_id,
                    source_item_id: challenger.item.id,
                    target_item_id: matched.item_id,
                    asserting_revision_id: challenger.revision.id,
                    relation_type: KnowledgeRelationType::Supports,
                    metadata: json!({
                        "change_id": change_id,
                        "conflict_set_id": set.id,
                        "classification": matched.classification.as_str(),
                    }),
                    created_by: Some(actor_subject.to_owned()),
                },
            )
            .await?;
        }
        knowledge_conflicts::mark_resolved(
            tx,
            tenant_id,
            set.id,
            set.revision,
            change_id,
            ConflictResolutionKind::Support,
            actor_subject,
        )
        .await?;
        audit::record(
            tx,
            tenant_id,
            AuditAction::KnowledgeConflictResolved,
            Resource::KnowledgeItem(challenger.item.id).to_string(),
            Outcome::Success,
            json!({
                "change_id": change_id,
                "conflict_set_id": set.id,
                "resolution": "support",
                "automatic": true,
                "challenger_item_id": challenger.item.id,
                "challenger_revision_id": challenger.revision.id,
            }),
        )
        .await?;
    }
    metrics::counter!(
        KNOWLEDGE_CONFLICTS_TOTAL,
        "source" => "knowledge_write",
        "classification" => classification.as_str()
    )
    .increment(1);
    Ok(challenger)
}

fn artifact_references(
    command: &KnowledgeCommand,
    payload_hash: &str,
) -> Result<Vec<ArtifactReference>> {
    let mut references = match command {
        KnowledgeCommand::Create {
            item_id,
            revision_id,
            ..
        } => vec![ArtifactReference::new(
            ArtifactFamily::Knowledge,
            item_id.to_string(),
            "create",
            revision_id.to_string(),
            None,
        )?],
        KnowledgeCommand::Edit {
            item_id,
            expected_revision_id,
            revision_id,
            ..
        }
        | KnowledgeCommand::Verify {
            item_id,
            expected_revision_id,
            revision_id,
            ..
        } => vec![ArtifactReference::new(
            ArtifactFamily::Knowledge,
            item_id.to_string(),
            command.kind().as_str(),
            revision_id.to_string(),
            Some(expected_revision_id.to_string()),
        )?],
        KnowledgeCommand::Supersede {
            item_id,
            expected_revision_id,
            replacement_item_id,
            replacement_revision_id,
            ..
        } => vec![
            ArtifactReference::new(
                ArtifactFamily::Knowledge,
                item_id.to_string(),
                "supersede",
                payload_hash,
                Some(expected_revision_id.to_string()),
            )?,
            ArtifactReference::new(
                ArtifactFamily::Knowledge,
                replacement_item_id.to_string(),
                "create_replacement",
                replacement_revision_id.to_string(),
                None,
            )?,
        ],
        KnowledgeCommand::Merge {
            inputs,
            result_item_id,
            result_revision_id,
            ..
        } => {
            let mut references = Vec::with_capacity(inputs.len() + 1);
            for input in inputs {
                references.push(ArtifactReference::new(
                    ArtifactFamily::Knowledge,
                    input.item_id.to_string(),
                    "merge_source",
                    payload_hash,
                    Some(input.revision_id.to_string()),
                )?);
            }
            references.push(ArtifactReference::new(
                ArtifactFamily::Knowledge,
                result_item_id.to_string(),
                "merge",
                result_revision_id.to_string(),
                None,
            )?);
            references
        }
        KnowledgeCommand::Archive {
            item_id,
            expected_revision_id,
            ..
        }
        | KnowledgeCommand::Restore {
            item_id,
            expected_revision_id,
            ..
        }
        | KnowledgeCommand::Forget {
            item_id,
            expected_revision_id,
            ..
        } => vec![ArtifactReference::new(
            ArtifactFamily::Knowledge,
            item_id.to_string(),
            command.kind().as_str(),
            payload_hash,
            Some(expected_revision_id.to_string()),
        )?],
        KnowledgeCommand::ResolveConflict {
            conflict_set_id,
            expected_conflict_revision,
            ..
        } => vec![ArtifactReference::new(
            ArtifactFamily::Knowledge,
            conflict_set_id.to_string(),
            "resolve_conflict",
            payload_hash,
            Some(expected_conflict_revision.to_string()),
        )?],
    };

    let sources = match command {
        KnowledgeCommand::Create { sources, .. }
        | KnowledgeCommand::Edit { sources, .. }
        | KnowledgeCommand::Supersede { sources, .. }
        | KnowledgeCommand::Merge { sources, .. } => sources.as_slice(),
        KnowledgeCommand::Verify { .. }
        | KnowledgeCommand::Archive { .. }
        | KnowledgeCommand::Restore { .. }
        | KnowledgeCommand::Forget { .. } => &[],
        KnowledgeCommand::ResolveConflict { .. } => &[],
    };
    for source in sources
        .iter()
        .filter(|source| source.source_type.as_str() == "okf")
    {
        let artifact_id = source
            .metadata
            .get("artifact_id")
            .and_then(Value::as_str)
            .or(source.locator.as_deref())
            .map_or_else(|| source.id.to_string(), str::to_owned);
        let version = source
            .content_hash
            .as_deref()
            .or(source.source_revision.as_deref())
            .map_or_else(|| source.id.to_string(), str::to_owned);
        references.push(ArtifactReference::new(
            ArtifactFamily::OkfImport,
            artifact_id,
            "publish_candidate",
            version,
            None,
        )?);
    }
    references.sort();
    references.dedup();
    Ok(references)
}

/// Replays one idempotent Knowledge command after taking the original
/// command's current ownership and PDP decisions again.
///
/// A replay is not cached authority. If the caller lost write or proposal
/// authority after the first request, the same key is refused now. An erased
/// result has no payload to re-authorise and is returned as vanished rather
/// than creating a replacement item.
pub async fn replay_command(
    state: &AppState,
    change_id: ProposalId,
) -> Result<KnowledgeMutationResult> {
    let tenant_id = ambient_tenant()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let proposal = vedaflow::proposals::read(&mut tx, tenant_id, change_id)
        .await?
        .filter(|proposal| {
            proposal.asset == AssetKind::Knowledge && proposal.effect == ProposalEffect::Apply
        })
        .ok_or_else(|| Error::NotFound {
            entity: format!("Knowledge change {change_id}"),
        })?;
    let change = knowledge_lifecycle::read_change(&mut *tx, tenant_id, change_id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("Knowledge change {change_id}"),
        })?;
    let command = change.payload.as_ref().ok_or_else(|| Error::NotFound {
        entity: format!("Knowledge affected by change {change_id}"),
    })?;
    authorize_command(state, &mut tx, tenant_id, command).await?;
    change_result(&mut tx, tenant_id, &proposal).await
}

/// Apply an open Knowledge change after its recorded approvals satisfy the
/// live matrix. Preconditions and PDP decisions are repeated at execution;
/// an approval never turns stale bytes or lost authority into a write.
pub async fn apply_reviewed(
    state: &AppState,
    change_id: ProposalId,
) -> Result<KnowledgeMutationResult> {
    let tenant_id = ambient_tenant()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let proposal = vedaflow::proposals::read(&mut tx, tenant_id, change_id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("Knowledge change {change_id}"),
        })?;
    if proposal.asset != AssetKind::Knowledge || proposal.effect != ProposalEffect::Apply {
        return Err(Error::NotFound {
            entity: format!("Knowledge change {change_id}"),
        });
    }
    authorize_change_read(state, &mut tx, tenant_id, &proposal).await?;
    if proposal.state != ProposalState::Open {
        return change_result(&mut tx, tenant_id, &proposal).await;
    }
    let change = knowledge_lifecycle::read_change(&mut *tx, tenant_id, change_id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!("Knowledge proposal {change_id} has no typed effect projection"),
        })?;
    let command = change.payload.as_ref().ok_or_else(|| Error::Conflict {
        message: format!("Knowledge change {change_id} payload was erased and cannot apply"),
    })?;
    verify_change_binding(&mut tx, tenant_id, &proposal, &change, command).await?;
    let authorization = authorize_command(state, &mut tx, tenant_id, command).await?;
    if authorization.target.id != proposal.target_scope_id {
        return Err(Error::Internal {
            message: format!(
                "Knowledge change {change_id} target moved from {} to {}",
                proposal.target_scope_id, authorization.target.id
            ),
        });
    }
    let requirement = approvals::resolve(
        state,
        &mut tx,
        tenant_id,
        &authorization.proposal_input,
        &Requested {
            target: &authorization.target,
            asset: AssetKind::Knowledge,
            sensitivity: authorization.sensitivity,
            entries: &["command".to_owned()],
        },
    )
    .await?;
    let approvals = vedaflow::proposals::approvals(&mut tx, tenant_id, change_id).await?;
    let cast = vedaflow::proposals::cast_for(&approvals, proposal.commit);
    let outstanding = requirement.outstanding(&cast);
    if !outstanding.is_empty() {
        return Err(Error::Conflict {
            message: format!(
                "Knowledge change {change_id} still needs {}",
                outstanding.describe()
            ),
        });
    }
    let actor = identity_of(&authorization.proposal_input)?;
    approvals::require_effect_actor(&requirement, change_id, proposal.proposer_id, &cast, actor)?;
    let actor_subject = authorization.proposal_input.principal.subject.clone();
    let result = apply_loaded(
        &mut tx,
        &ApplyRequest {
            state,
            tenant_id,
            change_id,
            command,
            payload_hash: &change.payload_hash,
            actor,
            actor_subject: &actor_subject,
            authorization: &authorization,
        },
    )
    .await?;
    tx.commit().await.map_err(|err| Error::Storage {
        message: format!("commit reviewed Knowledge change: {err}"),
    })?;
    Ok(result)
}

/// Render the current result of a Knowledge change without exposing its
/// erasable payload.
pub async fn result(state: &AppState, change_id: ProposalId) -> Result<KnowledgeMutationResult> {
    let tenant_id = ambient_tenant()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let proposal = vedaflow::proposals::read(&mut tx, tenant_id, change_id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("Knowledge change {change_id}"),
        })?;
    if proposal.asset != AssetKind::Knowledge || proposal.effect != ProposalEffect::Apply {
        return Err(Error::NotFound {
            entity: format!("Knowledge change {change_id}"),
        });
    }
    let authorized = authorize_change_read(state, &mut tx, tenant_id, &proposal).await?;
    let rendered = change_result(&mut tx, tenant_id, &proposal).await?;
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::AuthzDecision,
        Resource::Scope(proposal.target_scope_id).to_string(),
        Outcome::Allow,
        json!({
            "op": "knowledge_change_result",
            "change_id": change_id,
            "authz": audit::decision_context(Action::ProposalRead, &authorized),
        }),
    )
    .await?;
    tx.commit().await.map_err(|err| Error::Storage {
        message: format!("commit Knowledge change read: {err}"),
    })?;
    Ok(rendered)
}

fn command_payload_hash(command: &KnowledgeCommand) -> Result<String> {
    let encoded = canonicalise(
        &serde_json::to_value(command).map_err(|err| Error::Invalid {
            message: format!("encode Knowledge command: {err}"),
        })?,
    );
    let encoded_bytes = serde_json::to_vec(&encoded).map_err(|err| Error::Invalid {
        message: format!("encode canonical Knowledge command: {err}"),
    })?;
    Ok(blake3::hash(&encoded_bytes).to_hex().to_string())
}

/// Prove at the execution boundary that the typed effect is still the exact
/// command whose digest the proposal commit names. The database makes all
/// three rows immutable, but this independent check also detects storage
/// corruption or privileged tampering before any Knowledge write occurs.
async fn verify_change_binding(
    tx: &mut PgConnection,
    tenant_id: TenantId,
    proposal: &vedaflow::StoredProposal,
    change: &knowledge_lifecycle::StoredKnowledgeChange,
    command: &KnowledgeCommand,
) -> Result<()> {
    let members = vedaflow::proposals::members(tx, tenant_id, proposal.commit).await?;
    let [member] = members.as_slice() else {
        return Err(invalid_change_binding(proposal.id));
    };
    if member.name != "command" {
        return Err(invalid_change_binding(proposal.id));
    }
    let object = vedaflow::read_object(tx, tenant_id, member.object)
        .await?
        .ok_or_else(|| invalid_change_binding(proposal.id))?;
    if object.kind != AssetKind::Knowledge {
        return Err(invalid_change_binding(proposal.id));
    }
    let manifest: Value =
        serde_json::from_slice(&object.content).map_err(|_| invalid_change_binding(proposal.id))?;
    let targets =
        serde_json::to_value(command.target_item_ids()).map_err(|err| Error::Internal {
            message: format!("encode Knowledge change {} targets: {err}", proposal.id),
        })?;
    let valid = command_payload_hash(command)? == change.payload_hash
        && manifest.get("command").and_then(Value::as_str) == Some(change.command_kind.as_str())
        && manifest.get("payload_hash").and_then(Value::as_str)
            == Some(change.payload_hash.as_str())
        && manifest.get("target_item_ids") == Some(&targets);
    if !valid {
        return Err(invalid_change_binding(proposal.id));
    }
    Ok(())
}

fn invalid_change_binding(change_id: ProposalId) -> Error {
    Error::Internal {
        message: format!(
            "Knowledge change {change_id} failed its VedaFlow payload-integrity check"
        ),
    }
}

struct CommandAuthorization {
    target: synveda_types::scope::Scope,
    sensitivity: Sensitivity,
    proposal_input: DecisionInput,
    proposal_allowed: Authorized,
}

async fn authorize_change_read(
    state: &AppState,
    tx: &mut PgConnection,
    tenant_id: TenantId,
    proposal: &vedaflow::StoredProposal,
) -> Result<Authorized> {
    let target = scope_for(tx, tenant_id, proposal.target_scope_id).await?;
    let input = authz::gather(
        state,
        tx,
        Some(&target),
        AnchorSelection::none(),
        Vec::new(),
    )
    .await?;
    authz::decide(
        state,
        &input,
        Action::ProposalRead,
        Resource::Scope(proposal.target_scope_id),
    )
}

async fn authorize_command(
    state: &AppState,
    tx: &mut PgConnection,
    tenant_id: TenantId,
    command: &KnowledgeCommand,
) -> Result<CommandAuthorization> {
    // Capture challengers remain governed by New Learnings. Authorise both
    // their source and proposed destination before revealing that this command
    // is intentionally unavailable; otherwise a guessed conflict-set id would
    // become a same-tenant candidate-existence oracle.
    let mut capture_backed_conflict = false;
    if let KnowledgeCommand::ResolveConflict {
        conflict_set_id, ..
    } = command
    {
        let set = knowledge_conflicts::get(&mut *tx, tenant_id, *conflict_set_id)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity: format!("conflict set {conflict_set_id}"),
            })?;
        if let Some(candidate_id) = set.capture_candidate_id {
            capture_backed_conflict = true;
            let candidate = capture_store::get_candidate(&mut *tx, tenant_id, candidate_id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("conflict set {conflict_set_id}"),
                })?;
            crate::capture::authorize_context_candidate(state, tx, tenant_id, &candidate).await?;
        }
    }
    let targets = load_targets(tx, tenant_id, command).await?;
    for snapshot in &targets {
        let scope = scope_for(tx, tenant_id, snapshot.item.scope_id).await?;
        let input = authz::gather(
            state,
            tx,
            Some(&scope),
            AnchorSelection::none(),
            vec![ResourceEntity::KnowledgeItem {
                id: snapshot.item.id,
                scope_id: snapshot.item.scope_id,
            }],
        )
        .await?;
        let action = if matches!(command, KnowledgeCommand::Forget { .. }) {
            Action::KnowledgeForget
        } else {
            Action::KnowledgeWrite
        };
        authz::decide(
            state,
            &input,
            action,
            Resource::KnowledgeItem(snapshot.item.id),
        )?;
        if command_reads_existing(command) {
            authz::decide_knowledge_read(
                state,
                &input,
                Resource::KnowledgeItem(snapshot.item.id),
                snapshot.revision.content.sensitivity,
            )?;
        }
    }
    if capture_backed_conflict {
        return Err(Error::Invalid {
            message: "capture-backed conflicts are resolved through New Learnings".to_owned(),
        });
    }

    let target_scope_id = command_scope(command, &targets)?;
    let target = scope_for(tx, tenant_id, target_scope_id).await?;
    validate_project_scope(tx, tenant_id, command, target_scope_id).await?;
    let proposal_input = authz::gather(
        state,
        tx,
        Some(&target),
        AnchorSelection::none(),
        Vec::new(),
    )
    .await?;
    if command_creates_item(command) {
        authz::decide(
            state,
            &proposal_input,
            Action::KnowledgeWrite,
            Resource::Scope(target_scope_id),
        )?;
    }
    for source_scope_id in source_scopes(command) {
        let source_scope = scope_for(tx, tenant_id, source_scope_id).await?;
        let input = authz::gather(
            state,
            tx,
            Some(&source_scope),
            AnchorSelection::none(),
            Vec::new(),
        )
        .await?;
        authz::decide(
            state,
            &input,
            Action::KnowledgeWrite,
            Resource::Scope(source_scope_id),
        )?;
    }
    let proposal_allowed = authz::decide(
        state,
        &proposal_input,
        Action::ProposalOpen,
        Resource::Scope(target_scope_id),
    )?;
    let sensitivity = command_sensitivity(command, &targets)?;
    Ok(CommandAuthorization {
        target,
        sensitivity,
        proposal_input,
        proposal_allowed,
    })
}

struct ApplyRequest<'a> {
    state: &'a AppState,
    tenant_id: TenantId,
    change_id: ProposalId,
    command: &'a KnowledgeCommand,
    payload_hash: &'a str,
    actor: IdentityId,
    actor_subject: &'a str,
    authorization: &'a CommandAuthorization,
}

struct RejectedEffect {
    code: &'static str,
    item_id: Option<KnowledgeItemId>,
    operation_id: Option<DurableOperationId>,
}

async fn apply_loaded(
    tx: &mut PgConnection,
    request: &ApplyRequest<'_>,
) -> Result<KnowledgeMutationResult> {
    let state = request.state;
    let tenant_id = request.tenant_id;
    let change_id = request.change_id;
    let command = request.command;
    let payload_hash = request.payload_hash;
    let actor = request.actor;
    let actor_subject = request.actor_subject;
    let authorization = request.authorization;
    // Re-run PDP decisions at the effect boundary. Revision and lifecycle
    // preconditions are evaluated inside a savepoint below: a stale command
    // must close this real change as rejected without retaining any partial
    // source, revision or relation writes.
    let refreshed = authorize_command(state, tx, tenant_id, command).await?;
    if refreshed.target.id != authorization.target.id {
        return Err(Error::Conflict {
            message: "the Knowledge command's governing scope changed before apply".to_owned(),
        });
    }

    let mut effect_tx = tx.begin().await.map_err(|err| Error::Storage {
        message: format!("begin Knowledge effect savepoint: {err}"),
    })?;
    let execution: Result<EffectExecution> = {
        let tx: &mut PgConnection = &mut effect_tx;
        async {
            Ok(EffectExecution::Applied(match command {
        KnowledgeCommand::Create {
            item_id,
            scope_id,
            project_id,
            owner_principal_id,
            knowledge_type,
            origin,
            revision_id,
            content,
            sources,
        } => {
            let matches = classify_write(
                state,
                tx,
                tenant_id,
                *scope_id,
                *project_id,
                owner_principal_id.clone(),
                *knowledge_type,
                content,
                None,
            )
            .await?;
            let source_ids = create_sources(tx, tenant_id, sources, actor_subject).await?;
            let mut snapshot = store::create_item(
                tx,
                &NewKnowledgeItem {
                    id: *item_id,
                    tenant_id,
                    scope_id: *scope_id,
                    project_id: *project_id,
                    owner_principal_id: owner_principal_id.clone(),
                    knowledge_type: *knowledge_type,
                    origin: *origin,
                    created_by: Some(actor_subject.to_owned()),
                },
                &NewKnowledgeRevision {
                    id: *revision_id,
                    content: content.clone(),
                    created_by: Some(actor_subject.to_owned()),
                },
                &source_ids,
            )
            .await?;
            if !matches.is_empty() {
                snapshot = retain_write_conflict(
                    tx,
                    tenant_id,
                    change_id,
                    snapshot,
                    &matches,
                    actor_subject,
                )
                .await?;
            }
            AppliedEffect::item(snapshot.item.id, snapshot.revision.id)
        }
        KnowledgeCommand::Edit {
            item_id,
            expected_revision_id,
            revision_id,
            content,
            sources,
        } => {
            let before = current(tx, tenant_id, *item_id, *expected_revision_id).await?;
            let matches = classify_write(
                state,
                tx,
                tenant_id,
                before.item.scope_id,
                before.item.project_id,
                before.item.owner_principal_id.clone(),
                before.item.knowledge_type,
                content,
                Some(*item_id),
            )
            .await?;
            let source_ids = create_sources(tx, tenant_id, sources, actor_subject).await?;
            let mut snapshot = store::append_revision(
                tx,
                tenant_id,
                *item_id,
                *expected_revision_id,
                &NewKnowledgeRevision {
                    id: *revision_id,
                    content: content.clone(),
                    created_by: Some(actor_subject.to_owned()),
                },
                &source_ids,
            )
            .await?
            .ok_or_else(|| missing_item(*item_id))?;
            if !matches.is_empty() {
                snapshot = retain_write_conflict(
                    tx,
                    tenant_id,
                    change_id,
                    snapshot,
                    &matches,
                    actor_subject,
                )
                .await?;
            }
            AppliedEffect::item(snapshot.item.id, snapshot.revision.id)
        }
        KnowledgeCommand::Verify {
            item_id,
            expected_revision_id,
            revision_id,
            verification_metadata,
        } => {
            let current = current(tx, tenant_id, *item_id, *expected_revision_id).await?;
            let mut content = current.revision.content.clone();
            content.verification_metadata = canonicalise(verification_metadata);
            content.stale_after = None;
            let effective = configuration::effective_at_scope(
                tx,
                tenant_id,
                current.item.scope_id,
            )
            .await?;
            let policy = FreshnessPolicy::from_effective(&effective, current.item.knowledge_type);
            apply_policy_to_content(&mut content, &policy, Utc::now())?;
            validate_knowledge_revision_content(&content)?;
            let source_ids =
                store::revision_source_ids(&mut *tx, tenant_id, current.revision.id).await?;
            let snapshot = store::append_revision(
                tx,
                tenant_id,
                *item_id,
                *expected_revision_id,
                &NewKnowledgeRevision {
                    id: *revision_id,
                    content,
                    created_by: Some(actor_subject.to_owned()),
                },
                &source_ids,
            )
            .await?
            .ok_or_else(|| missing_item(*item_id))?;
            AppliedEffect::item(snapshot.item.id, snapshot.revision.id)
        }
        KnowledgeCommand::Supersede {
            item_id,
            expected_revision_id,
            replacement_item_id,
            replacement_revision_id,
            scope_id,
            project_id,
            owner_principal_id,
            knowledge_type,
            origin,
            content,
            sources,
        } => {
            let old = current(tx, tenant_id, *item_id, *expected_revision_id).await?;
            ensure_current_for_replacement(&old)?;
            let source_ids = create_sources(tx, tenant_id, sources, actor_subject).await?;
            let replacement = store::create_item(
                tx,
                &NewKnowledgeItem {
                    id: *replacement_item_id,
                    tenant_id,
                    scope_id: *scope_id,
                    project_id: *project_id,
                    owner_principal_id: owner_principal_id.clone(),
                    knowledge_type: *knowledge_type,
                    origin: *origin,
                    created_by: Some(actor_subject.to_owned()),
                },
                &NewKnowledgeRevision {
                    id: *replacement_revision_id,
                    content: content.clone(),
                    created_by: Some(actor_subject.to_owned()),
                },
                &source_ids,
            )
            .await?;
            store::add_relation(
                tx,
                &NewKnowledgeRelation {
                    id: KnowledgeRelationId::new(),
                    tenant_id,
                    source_item_id: *replacement_item_id,
                    target_item_id: *item_id,
                    asserting_revision_id: *replacement_revision_id,
                    relation_type: KnowledgeRelationType::Supersedes,
                    metadata: json!({"change_id": change_id}),
                    created_by: Some(actor_subject.to_owned()),
                },
            )
            .await?;
            store::set_lifecycle(
                tx,
                tenant_id,
                *item_id,
                *expected_revision_id,
                KnowledgeLifecycleState::Superseded,
                Some(actor_subject),
            )
            .await?
            .ok_or_else(|| missing_item(*item_id))?;
            AppliedEffect::item(replacement.item.id, replacement.revision.id)
        }
        KnowledgeCommand::Merge {
            inputs,
            result_item_id,
            result_revision_id,
            scope_id,
            project_id,
            owner_principal_id,
            knowledge_type,
            origin,
            content,
            sources,
        } => {
            let mut source_ids = Vec::new();
            let mut seen = HashSet::new();
            let mut snapshots = Vec::with_capacity(inputs.len());
            for input in inputs {
                let snapshot = current(tx, tenant_id, input.item_id, input.revision_id).await?;
                ensure_current_for_replacement(&snapshot)?;
                for source_id in
                    store::revision_source_ids(&mut *tx, tenant_id, snapshot.revision.id).await?
                {
                    if seen.insert(source_id) {
                        source_ids.push(source_id);
                    }
                }
                snapshots.push(snapshot);
            }
            for source_id in create_sources(tx, tenant_id, sources, actor_subject).await? {
                if seen.insert(source_id) {
                    source_ids.push(source_id);
                }
            }
            let result = store::create_item(
                tx,
                &NewKnowledgeItem {
                    id: *result_item_id,
                    tenant_id,
                    scope_id: *scope_id,
                    project_id: *project_id,
                    owner_principal_id: owner_principal_id.clone(),
                    knowledge_type: *knowledge_type,
                    origin: *origin,
                    created_by: Some(actor_subject.to_owned()),
                },
                &NewKnowledgeRevision {
                    id: *result_revision_id,
                    content: content.clone(),
                    created_by: Some(actor_subject.to_owned()),
                },
                &source_ids,
            )
            .await?;
            for snapshot in snapshots {
                store::add_relation(
                    tx,
                    &NewKnowledgeRelation {
                        id: KnowledgeRelationId::new(),
                        tenant_id,
                        source_item_id: *result_item_id,
                        target_item_id: snapshot.item.id,
                        asserting_revision_id: *result_revision_id,
                        relation_type: KnowledgeRelationType::DerivedFrom,
                        metadata: json!({"change_id": change_id}),
                        created_by: Some(actor_subject.to_owned()),
                    },
                )
                .await?;
                store::set_lifecycle(
                    tx,
                    tenant_id,
                    snapshot.item.id,
                    snapshot.revision.id,
                    KnowledgeLifecycleState::Superseded,
                    Some(actor_subject),
                )
                .await?
                .ok_or_else(|| missing_item(snapshot.item.id))?;
            }
            AppliedEffect::item(result.item.id, result.revision.id)
        }
        KnowledgeCommand::ResolveConflict {
            conflict_set_id,
            expected_conflict_revision,
            resolution,
            transition_at,
            ..
        } => {
            let set = knowledge_conflicts::get(tx, tenant_id, *conflict_set_id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("conflict set {conflict_set_id}"),
                })?;
            if set.capture_candidate_id.is_some() {
                return Err(Error::Invalid {
                    message: "a capture-backed conflict is resolved through New Learnings so the candidate remains the publication authority".to_owned(),
                });
            }
            let members = knowledge_conflicts::members(tx, tenant_id, *conflict_set_id).await?;
            let challenger_member = members
                .iter()
                .find(|member| member.role == ConflictMemberRole::Challenger)
                .ok_or_else(|| Error::Internal {
                    message: format!("conflict set {conflict_set_id} has no challenger"),
                })?;
            let challenger_item_id = challenger_member.knowledge_item_id.ok_or_else(|| {
                Error::Internal {
                    message: format!("conflict set {conflict_set_id} challenger is not Knowledge"),
                }
            })?;
            let challenger_revision_id = challenger_member.knowledge_revision_id.ok_or_else(|| {
                Error::Internal {
                    message: format!("conflict set {conflict_set_id} challenger has no revision"),
                }
            })?;
            let mut challenger = current(
                tx,
                tenant_id,
                challenger_item_id,
                challenger_revision_id,
            )
            .await?;
            if *resolution == ConflictResolutionKind::Transition {
                let transition_at = transition_at.ok_or_else(|| Error::Invalid {
                    message: "a transition resolution requires transition_at".to_owned(),
                })?;
                if transition_at <= Utc::now()
                    || challenger.revision.content.valid_from != transition_at
                {
                    return Err(Error::Invalid {
                        message: "transition_at must equal the challenger's future valid_from"
                            .to_owned(),
                    });
                }
            } else if transition_at.is_some() {
                return Err(Error::Invalid {
                    message: "transition_at is valid only for a transition resolution".to_owned(),
                });
            }

            let mut currents = Vec::new();
            for member in members
                .iter()
                .filter(|member| member.role == ConflictMemberRole::Current)
            {
                let item_id = member.knowledge_item_id.ok_or_else(|| Error::Internal {
                    message: format!("conflict set {conflict_set_id} has a non-Knowledge current member"),
                })?;
                let revision_id = member.knowledge_revision_id.ok_or_else(|| Error::Internal {
                    message: format!("conflict set {conflict_set_id} current member has no revision"),
                })?;
                currents.push(current(tx, tenant_id, item_id, revision_id).await?);
            }
            if currents.is_empty() {
                return Err(Error::Conflict {
                    message: format!("conflict set {conflict_set_id} has no current member"),
                });
            }

            if *resolution == ConflictResolutionKind::Supersede {
                for current in &currents {
                    ensure_current_for_replacement(current)?;
                    store::set_lifecycle(
                        tx,
                        tenant_id,
                        current.item.id,
                        current.revision.id,
                        KnowledgeLifecycleState::Superseded,
                        Some(actor_subject),
                    )
                    .await?
                    .ok_or_else(|| missing_item(current.item.id))?;
                }
            }
            let challenger_state = match resolution {
                ConflictResolutionKind::Archive | ConflictResolutionKind::Duplicate => {
                    KnowledgeLifecycleState::Archived
                }
                ConflictResolutionKind::KeepSeparate
                | ConflictResolutionKind::Support
                | ConflictResolutionKind::Supersede
                | ConflictResolutionKind::Transition => KnowledgeLifecycleState::Active,
            };
            if challenger.item.lifecycle_state != challenger_state {
                challenger = store::set_lifecycle(
                    tx,
                    tenant_id,
                    challenger.item.id,
                    challenger.revision.id,
                    challenger_state,
                    Some(actor_subject),
                )
                .await?
                .ok_or_else(|| missing_item(challenger.item.id))?;
            }
            if *resolution != ConflictResolutionKind::Archive {
                for current in &currents {
                    let relation_type = match resolution {
                        ConflictResolutionKind::KeepSeparate => {
                            ConflictClassification::Contradiction.relation_type()
                        }
                        ConflictResolutionKind::Support => KnowledgeRelationType::Supports,
                        ConflictResolutionKind::Duplicate => KnowledgeRelationType::Duplicates,
                        ConflictResolutionKind::Supersede => KnowledgeRelationType::Supersedes,
                        ConflictResolutionKind::Transition => KnowledgeRelationType::TransitionsTo,
                        ConflictResolutionKind::Archive => unreachable!("checked above"),
                    };
                    store::add_relation(
                        tx,
                        &NewKnowledgeRelation {
                            id: KnowledgeRelationId::new(),
                            tenant_id,
                            source_item_id: challenger.item.id,
                            target_item_id: current.item.id,
                            asserting_revision_id: challenger.revision.id,
                            relation_type,
                            metadata: json!({
                                "change_id": change_id,
                                "conflict_set_id": conflict_set_id,
                                "resolution": resolution.as_str(),
                            }),
                            created_by: Some(actor_subject.to_owned()),
                        },
                    )
                    .await?;
                }
            }
            knowledge_conflicts::mark_resolved(
                tx,
                tenant_id,
                *conflict_set_id,
                *expected_conflict_revision,
                change_id,
                *resolution,
                actor_subject,
            )
            .await?;
            audit::record(
                tx,
                tenant_id,
                AuditAction::KnowledgeConflictResolved,
                Resource::KnowledgeItem(challenger.item.id).to_string(),
                Outcome::Success,
                json!({
                    "change_id": change_id,
                    "conflict_set_id": conflict_set_id,
                    "resolution": resolution.as_str(),
                    "challenger_item_id": challenger.item.id,
                    "challenger_revision_id": challenger.revision.id,
                    "current_member_count": currents.len(),
                    "transition_at": transition_at,
                }),
            )
            .await?;
            metrics::counter!(
                KNOWLEDGE_CONFLICTS_TOTAL,
                "source" => "resolution",
                "classification" => set.classification.as_str()
            )
            .increment(1);
            AppliedEffect::item(challenger.item.id, challenger.revision.id)
        }
        KnowledgeCommand::Archive {
            item_id,
            expected_revision_id,
            ..
        } => {
            let current = current(tx, tenant_id, *item_id, *expected_revision_id).await?;
            if !matches!(
                current.item.lifecycle_state,
                KnowledgeLifecycleState::Active
                    | KnowledgeLifecycleState::Stale
                    | KnowledgeLifecycleState::Transitional
                    | KnowledgeLifecycleState::Superseded
            ) {
                return Err(Error::Conflict {
                    message: format!(
                        "Knowledge item {item_id} is {}; only current or superseded Knowledge can be archived",
                        current.item.lifecycle_state
                    ),
                });
            }
            let changed = store::set_lifecycle(
                tx,
                tenant_id,
                *item_id,
                *expected_revision_id,
                KnowledgeLifecycleState::Archived,
                Some(actor_subject),
            )
            .await?
            .ok_or_else(|| missing_item(*item_id))?;
            AppliedEffect::item(changed.item.id, changed.revision.id)
        }
        KnowledgeCommand::Restore {
            item_id,
            expected_revision_id,
            ..
        } => {
            let current = current(tx, tenant_id, *item_id, *expected_revision_id).await?;
            if current.item.lifecycle_state != KnowledgeLifecycleState::Archived {
                return Err(Error::Conflict {
                    message: format!(
                        "Knowledge item {item_id} is {}; only archived Knowledge can be restored",
                        current.item.lifecycle_state
                    ),
                });
            }
            let changed = store::set_lifecycle(
                tx,
                tenant_id,
                *item_id,
                *expected_revision_id,
                KnowledgeLifecycleState::Active,
                Some(actor_subject),
            )
            .await?
            .ok_or_else(|| missing_item(*item_id))?;
            AppliedEffect::item(changed.item.id, changed.revision.id)
        }
        KnowledgeCommand::Forget {
            item_id,
            expected_revision_id,
            reason,
        } => {
            let current = current(tx, tenant_id, *item_id, *expected_revision_id).await?;
            let operation = knowledge_lifecycle::create_erasure_operation(
                tx,
                tenant_id,
                change_id,
                *item_id,
                payload_hash,
            )
            .await?;
            if let Some(code) = erasure_hold(&current) {
                knowledge_lifecycle::block_operation(tx, tenant_id, operation.id, code).await?;
                return Ok(EffectExecution::Rejected {
                    item_id: Some(*item_id),
                    operation_id: Some(operation.id),
                    code,
                });
            }
            store::set_lifecycle(
                tx,
                tenant_id,
                *item_id,
                *expected_revision_id,
                KnowledgeLifecycleState::ErasurePending,
                Some(actor_subject),
            )
            .await?
            .ok_or_else(|| missing_item(*item_id))?;
            let running = knowledge_lifecycle::start_operation(
                tx,
                tenant_id,
                operation.id,
                "gateway-inline",
                300,
            )
            .await?
            .ok_or_else(|| Error::Conflict {
                message: format!(
                    "Knowledge erasure operation {} was already claimed",
                    operation.id
                ),
            })?;
            let actor_hash = blake3::hash(actor_subject.as_bytes()).to_hex().to_string();
            let reason_hash = blake3::hash(reason.as_bytes()).to_hex().to_string();
            reject_open_changes_for_erasure(
                tx,
                tenant_id,
                *item_id,
                change_id,
                actor,
                running.id,
            )
            .await?;
            knowledge_lifecycle::erase_knowledge(
                tx,
                tenant_id,
                *item_id,
                change_id,
                running.id,
                &actor_hash,
                &reason_hash,
            )
            .await?;
            audit::record(
                tx,
                tenant_id,
                AuditAction::KnowledgeErased,
                Resource::KnowledgeItem(*item_id).to_string(),
                Outcome::Success,
                json!({
                    "change_id": change_id,
                    "operation_id": running.id,
                    "knowledge_item_id": item_id,
                    "revision_id": expected_revision_id,
                    "actor_hash": actor_hash,
                    "reason_hash": reason_hash,
                    "payload_hash": payload_hash,
                    "artifact_references": artifact_references(command, payload_hash)?,
                }),
            )
            .await?;
            AppliedEffect {
                // The stable identifier is safe and useful after erasure; no
                // aggregate row or content remains behind it.
                item_id: Some(*item_id),
                revision_id: None,
                operation_id: Some(running.id),
            }
            }
            }))
        }
        .await
    };
    let execution = match execution {
        Ok(execution) => {
            effect_tx.commit().await.map_err(|err| Error::Storage {
                message: format!("commit Knowledge effect savepoint: {err}"),
            })?;
            execution
        }
        Err(error @ (Error::Conflict { .. } | Error::NotFound { .. })) => {
            effect_tx.rollback().await.map_err(|err| Error::Storage {
                message: format!("roll back rejected Knowledge effect: {err}"),
            })?;
            return reject_change(
                tx,
                request,
                &refreshed,
                RejectedEffect {
                    code: rejection_code(&error),
                    item_id: proposed_result_item(command),
                    operation_id: None,
                },
            )
            .await;
        }
        Err(error) => {
            effect_tx.rollback().await.map_err(|err| Error::Storage {
                message: format!("roll back failed Knowledge effect: {err}"),
            })?;
            return Err(error);
        }
    };
    let applied = match execution {
        EffectExecution::Applied(applied) => applied,
        EffectExecution::Rejected {
            item_id,
            operation_id,
            code,
        } => {
            audit::record(
                tx,
                tenant_id,
                AuditAction::KnowledgeErasureBlocked,
                item_id
                    .map(Resource::KnowledgeItem)
                    .unwrap_or(Resource::Scope(refreshed.target.id))
                    .to_string(),
                Outcome::Deny,
                json!({
                    "change_id": change_id,
                    "operation_id": operation_id,
                    "knowledge_item_id": item_id,
                    "hook": code,
                    "payload_hash": payload_hash,
                    "artifact_references": artifact_references(command, payload_hash)?,
                }),
            )
            .await?;
            return reject_change(
                tx,
                request,
                &refreshed,
                RejectedEffect {
                    code,
                    item_id,
                    operation_id,
                },
            )
            .await;
        }
    };

    if !knowledge_lifecycle::finish_change(
        tx,
        tenant_id,
        change_id,
        applied.item_id,
        applied.revision_id,
        applied.operation_id,
    )
    .await?
    {
        return Err(Error::Conflict {
            message: format!("Knowledge change {change_id} was already applied"),
        });
    }
    if !vedaflow::proposals::close(
        tx,
        tenant_id,
        change_id,
        ProposalState::Applied,
        actor,
        None,
    )
    .await?
    {
        return Err(Error::Conflict {
            message: format!("Knowledge change {change_id} closed before its effect completed"),
        });
    }
    audit::record(
        tx,
        tenant_id,
        AuditAction::KnowledgeChangeApplied,
        Resource::Scope(refreshed.target.id).to_string(),
        Outcome::Success,
        json!({
            "change_id": change_id,
            "command": command.kind().as_str(),
            "payload_hash": payload_hash,
            "artifact_references": artifact_references(command, payload_hash)?,
            "target_item_ids": command.target_item_ids(),
            "resulting_item_id": applied.item_id,
            "resulting_revision_id": applied.revision_id,
            "operation_id": applied.operation_id,
            "authz": audit::decision_context(Action::ProposalOpen, &refreshed.proposal_allowed),
        }),
    )
    .await?;
    Ok(KnowledgeMutationResult {
        change_id,
        outcome: KnowledgeMutationOutcome::Applied,
        knowledge_item_id: applied.item_id,
        revision_id: applied.revision_id,
        operation_id: applied.operation_id,
    })
}

async fn reject_change(
    tx: &mut PgConnection,
    request: &ApplyRequest<'_>,
    authorization: &CommandAuthorization,
    rejected: RejectedEffect,
) -> Result<KnowledgeMutationResult> {
    let tenant_id = request.tenant_id;
    let change_id = request.change_id;
    let command = request.command;
    let payload_hash = request.payload_hash;
    let actor = request.actor;
    let RejectedEffect {
        code,
        item_id,
        operation_id,
    } = rejected;
    if let KnowledgeCommand::ResolveConflict {
        conflict_set_id, ..
    } = command
    {
        let set = knowledge_conflicts::get(&mut *tx, tenant_id, *conflict_set_id)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity: format!("conflict set {conflict_set_id}"),
            })?;
        if set.status == ConflictSetStatus::PendingReview
            && set.resolution_change_id == Some(change_id)
        {
            knowledge_conflicts::reopen_after_rejection(tx, tenant_id, *conflict_set_id, change_id)
                .await?;
        }
    }
    if !vedaflow::proposals::close(
        tx,
        tenant_id,
        change_id,
        ProposalState::Rejected,
        actor,
        Some(code),
    )
    .await?
    {
        return Err(Error::Conflict {
            message: format!("Knowledge change {change_id} closed before rejection was recorded"),
        });
    }
    audit::record(
        tx,
        tenant_id,
        AuditAction::KnowledgeChangeRejected,
        Resource::Scope(authorization.target.id).to_string(),
        Outcome::Deny,
        json!({
            "change_id": change_id,
            "command": command.kind().as_str(),
            "payload_hash": payload_hash,
            "artifact_references": artifact_references(command, payload_hash)?,
            "target_item_ids": command.target_item_ids(),
            "knowledge_item_id": item_id,
            "operation_id": operation_id,
            "reason_code": code,
            "authz": audit::decision_context(Action::ProposalOpen, &authorization.proposal_allowed),
        }),
    )
    .await?;
    metrics::counter!(
        knowledge_lifecycle::KNOWLEDGE_LIFECYCLE_ACTS_TOTAL,
        "act" => "rejected",
        "command" => command.kind().as_str()
    )
    .increment(1);
    Ok(KnowledgeMutationResult {
        change_id,
        outcome: KnowledgeMutationOutcome::Rejected,
        knowledge_item_id: item_id,
        revision_id: None,
        operation_id,
    })
}

/// Close every other open effect that names an aggregate before its payload
/// is erased. Leaving one open would put a content hash with no reviewable
/// bytes in the inbox and make every later apply fail without reaching a
/// terminal state.
async fn reject_open_changes_for_erasure(
    tx: &mut PgConnection,
    tenant_id: TenantId,
    item_id: KnowledgeItemId,
    forget_change_id: ProposalId,
    actor: IdentityId,
    operation_id: DurableOperationId,
) -> Result<()> {
    let affected =
        knowledge_lifecycle::open_changes_for_item(&mut *tx, tenant_id, item_id, forget_change_id)
            .await?;
    for change in affected {
        let artifact_references =
            vedaflow::proposals::read(&mut *tx, tenant_id, change.proposal_id)
                .await?
                .ok_or_else(|| Error::Internal {
                    message: format!(
                        "Knowledge change {} disappeared while erasure invalidated it",
                        change.proposal_id
                    ),
                })?
                .artifact_references;
        if !vedaflow::proposals::close(
            &mut *tx,
            tenant_id,
            change.proposal_id,
            ProposalState::Rejected,
            actor,
            Some("target_erased"),
        )
        .await?
        {
            return Err(Error::Conflict {
                message: format!(
                    "Knowledge change {} closed while erasure invalidated it",
                    change.proposal_id
                ),
            });
        }
        audit::record(
            &mut *tx,
            tenant_id,
            AuditAction::KnowledgeChangeRejected,
            Resource::KnowledgeItem(item_id).to_string(),
            Outcome::Deny,
            json!({
                "change_id": change.proposal_id,
                "command": change.command_kind.as_str(),
                "payload_hash": change.payload_hash,
                "artifact_references": artifact_references,
                "knowledge_item_id": item_id,
                "invalidated_by_change_id": forget_change_id,
                "operation_id": operation_id,
                "reason_code": "target_erased",
            }),
        )
        .await?;
        metrics::counter!(
            knowledge_lifecycle::KNOWLEDGE_LIFECYCLE_ACTS_TOTAL,
            "act" => "rejected",
            "command" => change.command_kind.as_str()
        )
        .increment(1);
    }
    Ok(())
}

const fn rejection_code(error: &Error) -> &'static str {
    match error {
        Error::Conflict { .. } => "precondition_conflict",
        Error::NotFound { .. } => "target_disappeared",
        _ => "effect_rejected",
    }
}

enum EffectExecution {
    Applied(AppliedEffect),
    Rejected {
        item_id: Option<KnowledgeItemId>,
        operation_id: Option<DurableOperationId>,
        code: &'static str,
    },
}

#[derive(Debug, Clone, Copy)]
struct AppliedEffect {
    item_id: Option<KnowledgeItemId>,
    revision_id: Option<KnowledgeRevisionId>,
    operation_id: Option<DurableOperationId>,
}

impl AppliedEffect {
    const fn item(item_id: KnowledgeItemId, revision_id: KnowledgeRevisionId) -> Self {
        Self {
            item_id: Some(item_id),
            revision_id: Some(revision_id),
            operation_id: None,
        }
    }
}

async fn load_targets(
    tx: &mut PgConnection,
    tenant_id: TenantId,
    command: &KnowledgeCommand,
) -> Result<Vec<KnowledgeSnapshot>> {
    if let KnowledgeCommand::ResolveConflict {
        conflict_set_id, ..
    } = command
    {
        let set = knowledge_conflicts::get(&mut *tx, tenant_id, *conflict_set_id)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity: format!("conflict set {conflict_set_id}"),
            })?;
        let members = knowledge_conflicts::members(&mut *tx, tenant_id, *conflict_set_id).await?;
        let mut snapshots = Vec::with_capacity(members.len());
        for member in members {
            let (Some(item_id), Some(revision_id)) =
                (member.knowledge_item_id, member.knowledge_revision_id)
            else {
                continue;
            };
            let current = store::current(&mut *tx, tenant_id, item_id)
                .await?
                .ok_or_else(|| missing_item(item_id))?;
            let revision = store::revision(&mut *tx, tenant_id, item_id, revision_id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("Knowledge revision {revision_id}"),
                })?;
            snapshots.push(KnowledgeSnapshot {
                item: current.item,
                revision,
                transaction_to: current.transaction_to,
            });
        }
        if snapshots.is_empty() && set.capture_candidate_id.is_none() {
            return Err(Error::Internal {
                message: format!("conflict set {conflict_set_id} has no Knowledge evidence"),
            });
        }
        return Ok(snapshots);
    }

    let item_ids: Vec<KnowledgeItemId> = match command {
        KnowledgeCommand::Create { .. } => Vec::new(),
        KnowledgeCommand::Edit { item_id, .. }
        | KnowledgeCommand::Verify { item_id, .. }
        | KnowledgeCommand::Supersede { item_id, .. }
        | KnowledgeCommand::Archive { item_id, .. }
        | KnowledgeCommand::Restore { item_id, .. }
        | KnowledgeCommand::Forget { item_id, .. } => vec![*item_id],
        KnowledgeCommand::Merge { inputs, .. } => {
            inputs.iter().map(|input| input.item_id).collect()
        }
        KnowledgeCommand::ResolveConflict { .. } => unreachable!("handled above"),
    };
    let mut snapshots = Vec::with_capacity(item_ids.len());
    for item_id in item_ids {
        snapshots.push(
            store::current(&mut *tx, tenant_id, item_id)
                .await?
                .ok_or_else(|| missing_item(item_id))?,
        );
    }
    Ok(snapshots)
}

async fn current(
    tx: &mut PgConnection,
    tenant_id: TenantId,
    item_id: KnowledgeItemId,
    expected_revision_id: KnowledgeRevisionId,
) -> Result<KnowledgeSnapshot> {
    let snapshot = store::current(&mut *tx, tenant_id, item_id)
        .await?
        .ok_or_else(|| missing_item(item_id))?;
    if snapshot.revision.id != expected_revision_id {
        return Err(Error::Conflict {
            message: format!(
                "Knowledge item {item_id} is at revision {}, expected {expected_revision_id}",
                snapshot.revision.id
            ),
        });
    }
    Ok(snapshot)
}

async fn scope_for(
    tx: &mut PgConnection,
    tenant_id: TenantId,
    scope_id: ScopeId,
) -> Result<synveda_types::scope::Scope> {
    scopes::get(&mut *tx, tenant_id, scope_id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("scope {scope_id}"),
        })
}

async fn validate_project_scope(
    tx: &mut PgConnection,
    tenant_id: TenantId,
    command: &KnowledgeCommand,
    scope_id: ScopeId,
) -> Result<()> {
    let project_id = match command {
        KnowledgeCommand::Create { project_id, .. }
        | KnowledgeCommand::Supersede { project_id, .. }
        | KnowledgeCommand::Merge { project_id, .. } => *project_id,
        _ => None,
    };
    let Some(project_id) = project_id else {
        return Ok(());
    };
    let project = projects::get(&mut *tx, tenant_id, project_id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("project {project_id}"),
        })?;
    if project.scope_id != scope_id {
        return Err(Error::Invalid {
            message: format!(
                "project {project_id} owns scope {}, not Knowledge scope {scope_id}",
                project.scope_id
            ),
        });
    }
    Ok(())
}

async fn create_sources(
    tx: &mut PgConnection,
    tenant_id: TenantId,
    sources: &[KnowledgeSourceDraft],
    actor_subject: &str,
) -> Result<Vec<KnowledgeSourceId>> {
    let mut ids = Vec::with_capacity(sources.len());
    for source in sources {
        store::create_source(
            tx,
            &NewKnowledgeSource {
                id: source.id,
                tenant_id,
                scope_id: source.scope_id,
                source_type: source.source_type,
                session_event_id: source.session_event_id,
                locator: source.locator.clone(),
                source_revision: source.source_revision.clone(),
                content_hash: source.content_hash.clone(),
                metadata: canonicalise(&source.metadata),
                created_by: Some(actor_subject.to_owned()),
            },
        )
        .await?;
        ids.push(source.id);
    }
    Ok(ids)
}

async fn change_result(
    tx: &mut PgConnection,
    tenant_id: TenantId,
    proposal: &vedaflow::StoredProposal,
) -> Result<KnowledgeMutationResult> {
    let change = knowledge_lifecycle::read_change(&mut *tx, tenant_id, proposal.id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!("Knowledge proposal {} has no effect row", proposal.id),
        })?;
    let outcome = match proposal.state {
        ProposalState::Open => KnowledgeMutationOutcome::PendingReview,
        ProposalState::Rejected | ProposalState::Withdrawn => KnowledgeMutationOutcome::Rejected,
        ProposalState::Applied => KnowledgeMutationOutcome::Applied,
        ProposalState::Published => {
            return Err(Error::Internal {
                message: format!("Knowledge/apply proposal {} was published", proposal.id),
            });
        }
    };
    let operation_id = match change.operation_id {
        Some(id) => Some(id),
        None => knowledge_lifecycle::operation_for_change(&mut *tx, tenant_id, proposal.id)
            .await?
            .map(|operation| operation.id),
    };
    Ok(KnowledgeMutationResult {
        change_id: proposal.id,
        outcome,
        knowledge_item_id: change
            .resulting_item_id
            .or_else(|| proposed_result_item_from_change(&change)),
        revision_id: change.resulting_revision_id,
        operation_id,
    })
}

fn proposed_result_item(command: &KnowledgeCommand) -> Option<KnowledgeItemId> {
    match command {
        KnowledgeCommand::Create { item_id, .. } => Some(*item_id),
        KnowledgeCommand::Supersede {
            replacement_item_id,
            ..
        } => Some(*replacement_item_id),
        KnowledgeCommand::Merge { result_item_id, .. } => Some(*result_item_id),
        KnowledgeCommand::Edit { item_id, .. }
        | KnowledgeCommand::Verify { item_id, .. }
        | KnowledgeCommand::Archive { item_id, .. }
        | KnowledgeCommand::Restore { item_id, .. }
        | KnowledgeCommand::Forget { item_id, .. } => Some(*item_id),
        KnowledgeCommand::ResolveConflict { .. } => None,
    }
}

fn proposed_result_item_from_change(
    change: &knowledge_lifecycle::StoredKnowledgeChange,
) -> Option<KnowledgeItemId> {
    change.payload.as_ref().and_then(proposed_result_item)
}

fn command_scope(command: &KnowledgeCommand, targets: &[KnowledgeSnapshot]) -> Result<ScopeId> {
    match command {
        KnowledgeCommand::Create { scope_id, .. }
        | KnowledgeCommand::Supersede { scope_id, .. }
        | KnowledgeCommand::Merge { scope_id, .. } => Ok(*scope_id),
        _ => targets
            .first()
            .map(|snapshot| snapshot.item.scope_id)
            .ok_or_else(|| Error::Internal {
                message: "Knowledge command has neither an output scope nor a target".to_owned(),
            }),
    }
}

fn command_sensitivity(
    command: &KnowledgeCommand,
    targets: &[KnowledgeSnapshot],
) -> Result<Sensitivity> {
    let proposed = match command {
        KnowledgeCommand::Create { content, .. }
        | KnowledgeCommand::Edit { content, .. }
        | KnowledgeCommand::Supersede { content, .. }
        | KnowledgeCommand::Merge { content, .. } => Some(content.sensitivity),
        _ => None,
    };
    targets
        .iter()
        .map(|snapshot| snapshot.revision.content.sensitivity)
        .chain(proposed)
        .max()
        .ok_or_else(|| Error::Invalid {
            message: "a Knowledge command has no sensitivity to govern".to_owned(),
        })
}

fn command_creates_item(command: &KnowledgeCommand) -> bool {
    matches!(
        command,
        KnowledgeCommand::Create { .. }
            | KnowledgeCommand::Supersede { .. }
            | KnowledgeCommand::Merge { .. }
    )
}

fn command_reads_existing(command: &KnowledgeCommand) -> bool {
    !matches!(command, KnowledgeCommand::Create { .. })
}

fn source_scopes(command: &KnowledgeCommand) -> Vec<ScopeId> {
    let sources = match command {
        KnowledgeCommand::Create { sources, .. }
        | KnowledgeCommand::Edit { sources, .. }
        | KnowledgeCommand::Supersede { sources, .. } => sources.as_slice(),
        _ => &[],
    };
    let mut scopes: Vec<ScopeId> = sources.iter().map(|source| source.scope_id).collect();
    scopes.sort_unstable();
    scopes.dedup();
    scopes
}

fn validate_command(command: &KnowledgeCommand) -> Result<()> {
    match command {
        KnowledgeCommand::Create {
            item_id,
            revision_id,
            content,
            sources,
            ..
        } => {
            validate_knowledge_revision_content(content)?;
            validate_sources(sources)?;
            if item_id.as_uuid() == revision_id.as_uuid() {
                return Err(Error::Invalid {
                    message: "Knowledge item and revision ids must be distinct".to_owned(),
                });
            }
        }
        KnowledgeCommand::Edit {
            item_id,
            expected_revision_id,
            revision_id,
            content,
            sources,
        } => {
            validate_knowledge_revision_content(content)?;
            validate_sources(sources)?;
            if revision_id == expected_revision_id || item_id.as_uuid() == revision_id.as_uuid() {
                return Err(Error::Invalid {
                    message: "an edit requires a fresh revision id".to_owned(),
                });
            }
        }
        KnowledgeCommand::Verify {
            expected_revision_id,
            revision_id,
            verification_metadata,
            ..
        } => {
            if revision_id == expected_revision_id || !verification_metadata.is_object() {
                return Err(Error::Invalid {
                    message: "verification requires a fresh revision id and object metadata"
                        .to_owned(),
                });
            }
        }
        KnowledgeCommand::Supersede {
            item_id,
            expected_revision_id,
            replacement_item_id,
            replacement_revision_id,
            content,
            sources,
            ..
        } => {
            validate_knowledge_revision_content(content)?;
            validate_sources(sources)?;
            if replacement_item_id == item_id
                || replacement_revision_id == expected_revision_id
                || replacement_item_id.as_uuid() == replacement_revision_id.as_uuid()
            {
                return Err(Error::Invalid {
                    message: "supersession requires fresh replacement item and revision ids"
                        .to_owned(),
                });
            }
        }
        KnowledgeCommand::Merge {
            inputs,
            result_item_id,
            result_revision_id,
            content,
            sources,
            ..
        } => {
            validate_knowledge_revision_content(content)?;
            validate_sources_allow_empty(sources)?;
            let minimum = if sources.is_empty() { 2 } else { 1 };
            if inputs.len() < minimum || inputs.len() > vedaflow::MAX_PROPOSAL_MEMBERS {
                return Err(Error::Invalid {
                    message: format!(
                        "merge requires {minimum}..={} input items",
                        vedaflow::MAX_PROPOSAL_MEMBERS
                    ),
                });
            }
            let mut seen = HashSet::new();
            if inputs.iter().any(|input| !seen.insert(input.item_id))
                || inputs.iter().any(|input| input.item_id == *result_item_id)
                || result_item_id.as_uuid() == result_revision_id.as_uuid()
            {
                return Err(Error::Invalid {
                    message: "merge inputs and result ids must be distinct".to_owned(),
                });
            }
        }
        KnowledgeCommand::Archive { reason, .. }
        | KnowledgeCommand::Restore { reason, .. }
        | KnowledgeCommand::Forget { reason, .. } => validate_reason(reason)?,
        KnowledgeCommand::ResolveConflict {
            expected_conflict_revision,
            resolution,
            transition_at,
            reason,
            ..
        } => {
            if *expected_conflict_revision <= 0 {
                return Err(Error::Invalid {
                    message: "a conflict resolution requires a positive expected revision"
                        .to_owned(),
                });
            }
            validate_reason(reason)?;
            if (*resolution == ConflictResolutionKind::Transition) != transition_at.is_some() {
                return Err(Error::Invalid {
                    message: "transition_at is required only for transition resolution".to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn validate_sources(sources: &[KnowledgeSourceDraft]) -> Result<()> {
    if sources.is_empty() || sources.len() > 200 {
        return Err(Error::Invalid {
            message: "a Knowledge revision requires 1..=200 provenance sources".to_owned(),
        });
    }
    let mut ids = HashSet::new();
    for source in sources {
        if !ids.insert(source.id) {
            return Err(Error::Invalid {
                message: format!("Knowledge source {} is repeated", source.id),
            });
        }
        validate_knowledge_source(
            source.source_type,
            source.session_event_id,
            source.locator.as_deref(),
            source.source_revision.as_deref(),
            source.content_hash.as_deref(),
            &source.metadata,
        )?;
    }
    Ok(())
}

fn validate_sources_allow_empty(sources: &[KnowledgeSourceDraft]) -> Result<()> {
    if sources.is_empty() {
        Ok(())
    } else {
        validate_sources(sources)
    }
}

fn validate_reason(reason: &str) -> Result<()> {
    let chars = reason.chars().count();
    if reason.trim().is_empty() || chars > MAX_KNOWLEDGE_REASON_CHARS {
        return Err(Error::Invalid {
            message: format!(
                "a Knowledge lifecycle reason is 1..={MAX_KNOWLEDGE_REASON_CHARS} characters"
            ),
        });
    }
    Ok(())
}

fn ensure_current_for_replacement(snapshot: &KnowledgeSnapshot) -> Result<()> {
    if !matches!(
        snapshot.item.lifecycle_state,
        KnowledgeLifecycleState::Active | KnowledgeLifecycleState::Stale
    ) {
        return Err(Error::Conflict {
            message: format!(
                "Knowledge item {} is {}; only active or stale Knowledge can be replaced",
                snapshot.item.id, snapshot.item.lifecycle_state
            ),
        });
    }
    Ok(())
}

fn erasure_hold(snapshot: &KnowledgeSnapshot) -> Option<&'static str> {
    let metadata = &snapshot.revision.content.metadata;
    if metadata
        .get("legal_hold")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        Some("legal_hold")
    } else if metadata
        .get("retention_hold")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        Some("retention_hold")
    } else {
        None
    }
}

fn ambient_tenant() -> Result<TenantId> {
    synveda_identity::current_tenant()
        .map(|context| context.tenant.id)
        .ok_or_else(|| Error::Internal {
            message: "Knowledge command ran outside a tenant scope".to_owned(),
        })
}

fn identity_of(input: &DecisionInput) -> Result<IdentityId> {
    input
        .identity
        .as_ref()
        .map(|identity| identity.id)
        .ok_or_else(|| Error::Invalid {
            message: "Knowledge commands require a provisioned identity".to_owned(),
        })
}

fn missing_item(item_id: KnowledgeItemId) -> Error {
    Error::NotFound {
        entity: format!("Knowledge item {item_id}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use synveda_types::knowledge::{
        KnowledgeOrigin, KnowledgeRevisionContent, KnowledgeSourceType, KnowledgeType,
    };

    fn content() -> KnowledgeRevisionContent {
        KnowledgeRevisionContent {
            title: "Request correlation".to_owned(),
            body_markdown: "Use `traceparent`.".to_owned(),
            summary: "Public requests use traceparent.".to_owned(),
            tags: vec!["http".to_owned()],
            sensitivity: Sensitivity::Internal,
            confidence_permille: 900,
            valid_from: Utc::now(),
            valid_to: None,
            stale_after: Some(Utc::now() + Duration::days(30)),
            verification_metadata: json!({}),
            metadata: json!({}),
        }
    }

    fn source(scope_id: ScopeId) -> KnowledgeSourceDraft {
        KnowledgeSourceDraft {
            id: KnowledgeSourceId::new(),
            scope_id,
            source_type: KnowledgeSourceType::Manual,
            session_event_id: None,
            locator: None,
            source_revision: None,
            content_hash: None,
            metadata: json!({}),
        }
    }

    #[test]
    fn command_validation_refuses_missing_provenance_and_duplicate_merge_inputs() {
        let scope = ScopeId::new();
        let create = KnowledgeCommand::Create {
            item_id: KnowledgeItemId::new(),
            scope_id: scope,
            project_id: None,
            owner_principal_id: None,
            knowledge_type: KnowledgeType::Convention,
            origin: KnowledgeOrigin::Authored,
            revision_id: KnowledgeRevisionId::new(),
            content: content(),
            sources: Vec::new(),
        };
        assert!(validate_command(&create).is_err());

        let item = KnowledgeItemId::new();
        let merge = KnowledgeCommand::Merge {
            inputs: vec![
                synveda_types::knowledge::KnowledgeExpectedRevision {
                    item_id: item,
                    revision_id: KnowledgeRevisionId::new(),
                },
                synveda_types::knowledge::KnowledgeExpectedRevision {
                    item_id: item,
                    revision_id: KnowledgeRevisionId::new(),
                },
            ],
            result_item_id: KnowledgeItemId::new(),
            result_revision_id: KnowledgeRevisionId::new(),
            scope_id: scope,
            project_id: None,
            owner_principal_id: None,
            knowledge_type: KnowledgeType::Convention,
            origin: KnowledgeOrigin::Authored,
            content: content(),
            sources: Vec::new(),
        };
        assert!(validate_command(&merge).is_err());
        assert!(validate_sources(&[source(scope)]).is_ok());
    }

    #[test]
    fn erasure_hold_is_fail_closed_and_names_no_content() {
        let mut content = content();
        content.metadata = json!({"legal_hold": true});
        let snapshot = KnowledgeSnapshot {
            item: synveda_types::knowledge::KnowledgeItem {
                id: KnowledgeItemId::new(),
                tenant_id: TenantId::new(),
                scope_id: ScopeId::new(),
                project_id: None,
                owner_principal_id: None,
                knowledge_type: KnowledgeType::Fact,
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
                content,
                content_hash: "0".repeat(64),
                created_by: None,
                transaction_time: Utc::now(),
            },
            transaction_to: None,
        };
        assert_eq!(erasure_hold(&snapshot), Some("legal_hold"));
    }
}
