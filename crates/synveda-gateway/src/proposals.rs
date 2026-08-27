//! The VedaFlow proposal API (FLOW-3, ADR-0032): `/v1/proposals` behind
//! tenant resolution, uniform-404 ownership, and the PDP.
//!
//! A proposal is the governed request to run one reviewed effect. Authored
//! artifacts move onto a scope's published channel; Knowledge proposals
//! apply a typed aggregate command instead (CPR-16, ADR-0081). In both cases its
//! content is a commit — a tree naming every member at the exact object
//! address reviewed — and its workflow is one row. Approvals are append-only,
//! each naming the commit it approved and the effective roles its caster held
//! at the target when they cast it.
//!
//! # Two layers, kept apart
//!
//! Cedar decides *who may act*: `ProposalOpen` to open one, `ProposalRead`
//! to see one, `ProposalReview` to cast a verdict. The approval matrix
//! decides *how many acts are needed*, counting recorded approvals
//! against what the target scope's pack, the invariant floor, and the
//! nearest curator file require. Neither can do the other's job: the PDP
//! cannot see stored approvals, and a counting rule must never be
//! authority.
//!
//! # Climbing (FLOW-5, ADR-0034)
//!
//! An authored-artifact proposal's target may be a strict **ancestor** of
//! its source. It is not a second kind of proposal: same table, matrix,
//! lifecycle and audit actions. Opening a climb takes a second Cedar
//! decision using that artifact family's read action at the source, which
//! is the proposer's warrant for showing the material to target reviewers.
//!
//! # Publishing is a separate act
//!
//! The deciding approval does not publish. `POST /v1/proposals/{id}/publish`
//! takes `ChannelPublish` and the artifact family's read action at the
//! target, and additionally requires the proposal open, the
//! requirement satisfied, and the bytes unchanged since the review.
//! Auto-publishing would have to run under system authority precisely
//! when a `compliance` reviewer casts the deciding vote — a role that
//! holds no publish grant in any pack — and that is a PDP bypass however
//! it is spelled (ADR-0032 decision 9).
//!
//! # What a reviewer is shown (FLOW-6, ADR-0035)
//!
//! `GET /v1/proposals/{id}` renders each member as the *effect* publishing
//! it would have on the target's published channel — `add`, `update`, or
//! `none` — with the bytes on both sides of that effect: the object at the
//! proposed address, and (for an `update`) the object the target's tree
//! names today. That is what makes a terminal review possible without a
//! console, and it is the same disclosure ADR-0034 decision 1 already
//! makes, one version back: the old side is shown to whoever holds
//! `ProposalRead` at the target, because a review of a change that hides
//! one side of the change is not a review. The CLI does the rendering;
//! this route ships bytes.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{Action, Resource};
use synveda_store::{rls, scopes};
use synveda_types::scope::Scope;
use synveda_types::{
    ApprovalRequirement, ArtifactFamily, ArtifactReference, AssetKind, CastApproval, Channel,
    DocumentPath, Error, IdentityId, PromptName, ProposalEffect, ProposalId, ProposalState,
    ProposalView, Result, ScopeId, Sensitivity, TenantId, Verdict,
};
use synveda_vedaflow::{self as vedaflow, PolicySnapshot, Signer};

use crate::app::AppState;
use crate::approvals::{self, RequirementView};
use crate::audit;
use crate::authz::{self, DecisionInput};
use crate::request::{body, commit, found, tenant_id};
use crate::telemetry::{
    PROPOSAL_CLIMBS_TOTAL, PROPOSAL_OPERATIONS_TOTAL, PUBLISH_REVIEW_REQUIRED_TOTAL,
};

/// The listing page cap; `limit` above it is a 400, not a silent trim.
const MAX_LIMIT: i64 = 500;
const DEFAULT_LIMIT: i64 = 100;

/// The title cap; mirrors `vedaflow_proposals`' CHECK.
const MAX_TITLE_CHARS: usize = 500;

/// The comment and reason cap; mirrors the tables' CHECKs.
const MAX_TEXT_CHARS: usize = 1000;

/// Counts the operation and renders the result — the outcome taxonomy
/// every governed plane uses. Error-path audit events chain at this seam
/// (AUD-1, ADR-0019 decision 5).
async fn respond<T: IntoResponse>(
    state: &AppState,
    op: &'static str,
    result: Result<T>,
) -> Response {
    let outcome = crate::response::outcome(&result);
    metrics::counter!(PROPOSAL_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    crate::response::finish(state, op, result).await
}

// ── Views ──────────────────────────────────────────────────────────────

/// Content-free typed address shared by every governed artifact family.
#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = ProposalArtifactReference)]
pub(crate) struct ArtifactReferenceView {
    /// Closed common-review family vocabulary.
    family: String,
    /// Stable aggregate, binding, import job, or authored member id.
    artifact_id: String,
    /// Domain mutation carried by the reviewed effect.
    operation: String,
    /// Exact immutable revision, binding-state digest, or content digest.
    version: String,
    /// Head inspected by a revision-aware mutation.
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_revision: Option<String>,
}

impl From<&ArtifactReference> for ArtifactReferenceView {
    fn from(reference: &ArtifactReference) -> Self {
        Self {
            family: reference.family.as_str().to_owned(),
            artifact_id: reference.artifact_id.clone(),
            operation: reference.operation.clone(),
            version: reference.version.clone(),
            expected_revision: reference.expected_revision.clone(),
        }
    }
}

/// One proposal in a listing.
#[derive(Serialize, utoipa::ToSchema)]
pub(crate) struct ProposalSummary {
    #[schema(value_type = String, format = "uuid")]
    id: ProposalId,
    #[schema(value_type = String, format = "uuid")]
    target_scope_id: ScopeId,
    #[schema(value_type = String, format = "uuid")]
    source_scope_id: ScopeId,
    /// The target's hierarchy path. A review surface that renders two
    /// UUIDs is not one a person can use, and for a climb the *source*
    /// is half of what is being judged (FLOW-6, ADR-0035 decision 9).
    /// Absent only inside TEN-5's disposal window, when the scope the
    /// proposal targets has already gone.
    #[serde(skip_serializing_if = "Option::is_none")]
    target_scope_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_scope_path: Option<String>,
    asset: String,
    /// What running this proposal would do. `published` writes a channel,
    /// `classify` changes sensitivity, and `apply` executes a typed governed
    /// artifact command (including a policy relaxation).
    #[schema(value_type = String)]
    effect: ProposalEffect,
    /// The five-state vocabulary tech plan §2.3 describes: the stored
    /// state, with `approved` rendered from `open` plus a satisfied
    /// requirement (ADR-0032 decision 11).
    #[schema(value_type = String)]
    state: ProposalView,
    #[schema(value_type = String)]
    sensitivity: Sensitivity,
    title: String,
    /// The commit holding exactly what is proposed.
    commit: String,
    #[schema(value_type = String, format = "uuid")]
    proposer_id: IdentityId,
    proposer_subject: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    closed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    close_reason: Option<String>,
    /// Stable, content-free artifacts and exact versions bound by the commit.
    artifact_references: Vec<ArtifactReferenceView>,
    /// What the matrix asks for here, resolved now.
    required: RequirementView,
    /// What it still lacks, in one line a reviewer reads.
    outstanding: String,
}

/// One review act as the API renders it.
#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = ProposalApprovalView)]
pub(crate) struct ApprovalView {
    #[schema(value_type = String, format = "uuid")]
    approver_id: IdentityId,
    approver_subject: String,
    #[schema(value_type = String)]
    verdict: Verdict,
    /// The effective roles the approver held at the target when they cast
    /// it — recorded then, never re-derived now (ADR-0032 decision 5).
    roles: Vec<String>,
    /// The commit reviewed. An approval of another commit is evidence
    /// about other content and never carries over.
    commit: String,
    /// Whether this act still counts: `false` once the proposal's commit
    /// has moved past it.
    counts: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    comment: Option<String>,
    created_at: DateTime<Utc>,
}

/// One common proposal lifecycle event, oldest first.
#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = ProposalTimelineEvent)]
pub(crate) struct TimelineEventView {
    /// `opened`, `approved`, `rejected`, `withdrawn`, `applied`, or `published`.
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    actor_id: Option<IdentityId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actor_subject: Option<String>,
    at: DateTime<Utc>,
    /// Exact proposal commit the act bound.
    commit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

/// What publishing this proposal would do to the target's published
/// channel, for one member (FLOW-6, ADR-0035 decision 5). Membership in
/// the target's tree is the predicate — the same sense of "this scope
/// holds it" ADR-0034 decision 3 used one scope over.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug, utoipa::ToSchema)]
#[schema(as = ProposalMemberEffect)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemberEffect {
    /// The channel names no version of this member; publication admits it.
    Add,
    /// The channel names it at a different address; publication replaces
    /// that version with this one.
    Update,
    /// A non-channel Knowledge command will execute against its aggregate.
    Apply,
    /// The channel already names it at exactly this address; publication
    /// changes nothing about this member.
    None,
}

/// The version the target's published channel holds for a member now —
/// the old side of the diff, present only for [`MemberEffect::Update`].
///
/// This is the one content-visibility widening in FLOW-6 (ADR-0035
/// decision 8): a reviewer sees what a publication would overwrite.
/// Bounded by the proposal's own member set,
/// the target's own channel, and the target scope the reviewer already
/// holds `ProposalRead` on — and admitted because a review of a change
/// that hides one side of the change is not a review.
#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = ProposalBaselineView)]
pub(crate) struct BaselineView {
    /// The address the target's tree names for this member today.
    object_hash: String,
    /// That object's canonical bytes as text (ADR-0030 decision 4's
    /// human-readable form, which FLOW-1 chose for exactly this).
    text: String,
}

/// One member of a proposal — the id and the address that was proposed,
/// plus what a reviewer needs to review it: the bytes under review, the
/// bytes they would replace, and the artifact's current content.
#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = ProposalMemberView)]
pub(crate) struct MemberView {
    /// The tree entry name: a path for an authored asset or `command` for a
    /// typed aggregate effect. The one field every artifact family carries.
    member: String,
    /// What kind of asset this proposal carries — one word, so a reviewer's
    /// first line says what they are looking at.
    asset: String,
    /// The address the proposal named.
    object_hash: String,
    /// Whether the member still hashes to that address. `false` means the
    /// content moved after the proposal opened, and publishing will
    /// refuse (ADR-0032 decision 6).
    unchanged: bool,
    #[schema(value_type = String)]
    sensitivity: Sensitivity,
    /// The member's text **as it stands now**. Beside `unchanged` this is what makes drift
    /// legible; it is not what the approvals bind.
    content: String,
    /// What publication would do to the target's channel for this member.
    effect: MemberEffect,
    /// The canonical bytes at the proposed address — what the approvals
    /// bind, read from the object store rather than re-derived from the
    /// source row, because an edited artifact is no longer what anyone approved
    /// (ADR-0035 decision 6). Empty only if the object is missing, which
    /// the append-only store makes impossible.
    proposed: String,
    /// The version being replaced, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    baseline: Option<BaselineView>,
}

/// One proposal, in full.
#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = ProposalDetail)]
pub(crate) struct ProposalDetail {
    #[serde(flatten)]
    summary: ProposalSummary,
    members: Vec<MemberView>,
    approvals: Vec<ApprovalView>,
    timeline: Vec<TimelineEventView>,
}

// ── List ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct ListParams {
    /// Restrict to proposals targeting this scope; absent lists
    /// tenant-wide (and takes a tenant-resource decision, which the packs
    /// grant to review and admin roles only).
    scope_id: Option<ScopeId>,
    /// Restrict to one stored state. `approved` is not a stored state —
    /// it is computed — so filter on `open` and read `state` per row.
    state: Option<ProposalState>,
    /// Restrict to proposals whose typed artifact index contains this family.
    artifact_family: Option<ArtifactFamily>,
    limit: Option<i64>,
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = ProposalListResponse)]
pub(crate) struct ListResponse {
    proposals: Vec<ProposalSummary>,
}

/// `GET /v1/proposals` — proposals, newest first.
#[utoipa::path(
    get,
    path = "/v1/proposals",
    operation_id = "list_proposals",
    tag = "proposals",
    params(
        ("scope_id" = Option<String>, Query, format = "uuid"),
        ("state" = Option<String>, Query),
        ("artifact_family" = Option<String>, Query),
        ("limit" = Option<i64>, Query)
    ),
    responses(
        (status = 200, description = "Visible proposals", body = ListResponse),
        (status = 400, description = "The filter or limit is invalid", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Proposal read is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "The requested scope is absent", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "proposals.list", skip_all)]
pub(crate) async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Response {
    let result = async {
        let limit = params.limit.unwrap_or(DEFAULT_LIMIT);
        if !(1..=MAX_LIMIT).contains(&limit) {
            return Err(Error::Invalid {
                message: format!("limit must be 1..={MAX_LIMIT}"),
            });
        }
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let (input, resource) = match params.scope_id {
            None => (
                authz::gather(
                    &state,
                    &mut tx,
                    None,
                    synveda_store::anchors::AnchorSelection::none(),
                    Vec::new(),
                )
                .await?,
                Resource::Tenant(tenant_id),
            ),
            Some(scope_id) => {
                let node = found(
                    scopes::get(&mut *tx, tenant_id, scope_id).await?,
                    tenant_id,
                    scope_id,
                )?;
                (
                    authz::gather(
                        &state,
                        &mut tx,
                        Some(&node),
                        synveda_store::anchors::AnchorSelection::none(),
                        Vec::new(),
                    )
                    .await?,
                    Resource::Scope(scope_id),
                )
            }
        };
        let authorized = authz::decide(&state, &input, Action::ProposalRead, resource)?;
        let stored = vedaflow::proposals::list(
            &mut tx,
            tenant_id,
            vedaflow::ProposalFilter {
                target_scope: params.scope_id,
                state: params.state,
                artifact_family: params.artifact_family,
                limit,
            },
        )
        .await?;
        let mut proposals = Vec::with_capacity(stored.len());
        for proposal in stored {
            proposals.push(summarise(&state, &mut tx, tenant_id, &input, &proposal).await?);
        }
        // An allowed admin-plane read chains its decision (ADR-0019
        // decision 4).
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::AuthzDecision,
            resource.to_string(),
            Outcome::Allow,
            json!({
                "op": "list",
                "authz": audit::decision_context(Action::ProposalRead, &authorized),
                "proposals": proposals.len(),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(ListResponse { proposals }))
    }
    .await;
    respond(&state, "list", result).await
}

/// `GET /v1/proposals/{id}` — one proposal, with its members' content and
/// its review log.
///
/// The members' content is the disclosure a proposal makes: a reviewer
/// who cannot read the source scope reviews what the proposal shows them.
/// Since FLOW-5 that is a real difference — a climb's members live below
/// the target — and `ProposalRead` at the target is deliberately the only
/// gate (ADR-0034 decision 1). Requiring the reviewer to hold the artifact
/// read action at the *source* instead would break the product: `compliance`
/// holds no content read in any pack, so the invariant floor's own role
/// could never review a `restricted` climb. The read that guards a climb is the
/// proposer’s, taken once at open time and recorded under their name.
#[utoipa::path(
    get,
    path = "/v1/proposals/{id}",
    operation_id = "get_proposal",
    tag = "proposals",
    params(("id" = String, Path, format = "uuid")),
    responses(
        (status = 200, description = "The proposal, members, and review log", body = ProposalDetail),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Proposal read is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "The proposal is absent or outside the tenant", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "proposals.get", skip_all)]
pub(crate) async fn get(State(state): State<AppState>, Path(id): Path<ProposalId>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let proposal = load(&mut tx, tenant_id, id).await?;
        let node = target_node(&mut tx, tenant_id, &proposal).await?;
        let input = authz::gather(
            &state,
            &mut tx,
            Some(&node),
            synveda_store::anchors::AnchorSelection::none(),
            Vec::new(),
        )
        .await?;
        let authorized = authz::decide(
            &state,
            &input,
            Action::ProposalRead,
            Resource::Scope(node.id),
        )?;
        let summary = summarise(&state, &mut tx, tenant_id, &input, &proposal).await?;
        let members = member_views(&mut tx, tenant_id, &proposal).await?;
        let recorded = vedaflow::proposals::approvals(&mut tx, tenant_id, id).await?;
        let timeline = timeline(&proposal, &recorded);
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::AuthzDecision,
            Resource::Scope(node.id).to_string(),
            Outcome::Allow,
            json!({
                "op": "get",
                "authz": audit::decision_context(Action::ProposalRead, &authorized),
                "proposal_id": id,
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(ProposalDetail {
            members,
            approvals: recorded
                .into_iter()
                .map(|approval| ApprovalView {
                    counts: approval.commit == proposal.commit,
                    approver_id: approval.approver_id,
                    approver_subject: approval.approver_subject,
                    verdict: approval.verdict,
                    roles: approval
                        .roles
                        .iter()
                        .map(|role| role.as_str().to_owned())
                        .collect(),
                    commit: approval.commit.to_hex(),
                    comment: approval.comment,
                    created_at: approval.created_at,
                })
                .collect(),
            timeline,
            summary,
        }))
    }
    .await;
    respond(&state, "get", result).await
}

fn timeline(
    proposal: &vedaflow::StoredProposal,
    approvals: &[vedaflow::StoredApproval],
) -> Vec<TimelineEventView> {
    let commit = proposal.commit.to_hex();
    let mut events = vec![TimelineEventView {
        kind: "opened".to_owned(),
        actor_id: Some(proposal.proposer_id),
        actor_subject: Some(proposal.proposer_subject.clone()),
        at: proposal.created_at,
        commit: commit.clone(),
        reason: None,
    }];
    events.extend(approvals.iter().map(|approval| {
        TimelineEventView {
            kind: match approval.verdict {
                Verdict::Approve => "approved",
                Verdict::Reject => "rejected",
            }
            .to_owned(),
            actor_id: Some(approval.approver_id),
            actor_subject: Some(approval.approver_subject.clone()),
            at: approval.created_at,
            commit: approval.commit.to_hex(),
            reason: approval.comment.clone(),
        }
    }));
    let rejection_is_recorded = approvals
        .iter()
        .any(|approval| approval.verdict == Verdict::Reject);
    if proposal.state.is_terminal()
        && !(proposal.state == ProposalState::Rejected && rejection_is_recorded)
    {
        let actor_subject = proposal.closed_by.and_then(|actor| {
            approvals
                .iter()
                .find(|approval| approval.approver_id == actor)
                .map(|approval| approval.approver_subject.clone())
                .or_else(|| {
                    (actor == proposal.proposer_id).then(|| proposal.proposer_subject.clone())
                })
        });
        events.push(TimelineEventView {
            kind: proposal.state.as_str().to_owned(),
            actor_id: proposal.closed_by,
            actor_subject,
            at: proposal.closed_at.unwrap_or(proposal.updated_at),
            commit,
            reason: proposal.close_reason.clone(),
        });
    }
    events.sort_by_key(|event| event.at);
    events
}

// ── Open ───────────────────────────────────────────────────────────────

#[derive(Deserialize, utoipa::ToSchema)]
#[schema(as = ProposalOpenBody)]
#[serde(deny_unknown_fields)]
pub(crate) struct OpenBody {
    /// The scope whose published channel would move. Requirements resolve
    /// here, and only here — "each level's approvers" is true because
    /// each level's proposal resolves at that level (ADR-0034
    /// decision 4).
    #[schema(value_type = String, format = "uuid")]
    scope_id: ScopeId,
    /// Where the material is now. Absent means the target — the
    /// same-scope case, a climb of zero levels. Present, it must be the
    /// target or a **descendant** of it: a climb goes up the chain that
    /// composition walks down (ADR-0034 decision 2).
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    source_scope_id: Option<ScopeId>,
    /// The prompts to propose, by name (PRMT-1, ADR-0049 decision 6).
    ///
    /// Exactly one authored-artifact member list may be present: a proposal
    /// has one asset kind because the approval matrix resolves from it.
    ///
    /// The same two senses of "the source holds it" apply: the draft lives
    /// there, or the source's published channel names it at that address —
    /// which is what lets a department propose onward what a team climbed
    /// into it, with no draft row at the department at all.
    #[serde(default)]
    #[schema(value_type = Vec<String>)]
    prompt_names: Vec<PromptName>,
    /// The context-pack documents to propose, by path (PRMT-2, ADR-0050
    /// decision 1).
    ///
    /// One entry per **document**, named `pack/document`: the pack channel
    /// names documents rather than bundles (decision 3), so a proposal that
    /// publishes half a pack is a thing the vocabulary can express and a
    /// curator can decide on. Exactly one of the three member lists may be
    /// present, for `prompt_names`' reason — a proposal has one asset kind,
    /// because the approval matrix resolves from it and, since decision 15,
    /// `regulated-strict` prices a pack at a department at two distinct
    /// people where it prices a team's memory at one.
    #[serde(default)]
    #[schema(value_type = Vec<String>)]
    document_paths: Vec<DocumentPath>,
    /// What this proposes, in one line. A reviewer reads it in a list.
    title: String,
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = ProposalOpenResponse)]
pub(crate) struct OpenResponse {
    #[serde(flatten)]
    summary: ProposalSummary,
}

/// `POST /v1/proposals` — open a proposal against a scope's published
/// channel.
#[utoipa::path(
    post,
    path = "/v1/proposals",
    operation_id = "open_proposal",
    tag = "proposals",
    request_body = OpenBody,
    responses(
        (status = 200, description = "The opened proposal", body = OpenResponse),
        (status = 400, description = "The proposal shape or direction is invalid", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Opening this proposal is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "A scope or member is absent", body = crate::workspaces::ApiErrorBody),
        (status = 409, description = "The scope has reached its open-proposal limit", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "proposals.open", skip_all)]
pub(crate) async fn open(
    State(state): State<AppState>,
    payload: std::result::Result<Json<OpenBody>, JsonRejection>,
) -> Response {
    let result = open_inner(&state, payload).await;
    respond(&state, "open", result).await
}

async fn open_inner(
    state: &AppState,
    payload: std::result::Result<Json<OpenBody>, JsonRejection>,
) -> Result<Json<OpenResponse>> {
    let body = body(payload)?;
    validate_open(&body)?;
    let effect = ProposalEffect::Published;
    let tenant_id = tenant_id()?;
    let source_scope_id = body.source_scope_id.unwrap_or(body.scope_id);
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let node = found(
        scopes::get(&mut *tx, tenant_id, body.scope_id).await?,
        tenant_id,
        body.scope_id,
    )?;
    let source = found(
        scopes::get(&mut *tx, tenant_id, source_scope_id).await?,
        tenant_id,
        source_scope_id,
    )?;
    // Gathered at the *source* — the deeper node, whose chain contains
    // the target's as a suffix — so two scopes are decided from one set of
    // pack assignments and role bindings (ADR-0034 decision 12).
    let input = authz::gather(
        state,
        &mut tx,
        Some(&source),
        synveda_store::anchors::AnchorSelection::none(),
        Vec::new(),
    )
    .await?;
    // The climb's direction, checked before anything is decided about it:
    // the target has to be on the source's own chain. A peer scope is not,
    // has no authority over the source, and admitting one would turn the
    // approval matrix into a cross-team transfer with the target's curator
    // as the only party (ADR-0034 decision 2).
    let Some(target_position) = input.position_of(body.scope_id) else {
        return Err(Error::Invalid {
            message: format!(
                "scope {} is not an ancestor of {source_scope_id}; a promotion climbs \
                 the hierarchy, it does not cross it",
                body.scope_id
            ),
        });
    };
    let authorized = authz::decide_from(
        state,
        &input,
        target_position,
        Action::ProposalOpen,
        Resource::Scope(body.scope_id),
    )?;
    let proposer = identity_of(&input)?;

    // A review queue nobody can drain is a denial of service against the
    // reviewers — and FLOW-4 will open proposals without a human deciding
    // to (ADR-0032 reversal trigger a).
    let open_here = vedaflow::proposals::count_open(&mut tx, tenant_id, body.scope_id).await?;
    if open_here >= vedaflow::MAX_OPEN_PROPOSALS {
        return Err(Error::Conflict {
            message: format!(
                "{open_here} proposals already stand open at this scope, at the \
                 {} limit; review some before opening more",
                vedaflow::MAX_OPEN_PROPOSALS
            ),
        });
    }

    let asset = if body.prompt_names.is_empty() {
        AssetKind::ContextPack
    } else {
        AssetKind::Prompt
    };
    // The members, as the asset kind's own reader sees them: the two senses
    // of "the source holds it" (ADR-0034 decision 3) are the same two for
    // both kinds — it lives there, or the source's published channel names
    // it at its current address — read from different tables.
    let members_now: Vec<Proposed> = match asset {
        AssetKind::ContextPack => {
            held_documents(&mut tx, tenant_id, source_scope_id, &body.document_paths)
                .await?
                .into_iter()
                .map(Proposed::ContextPack)
                .collect()
        }
        AssetKind::Prompt => held_prompts(&mut tx, tenant_id, source_scope_id, &body.prompt_names)
            .await?
            .into_iter()
            .map(Proposed::Prompt)
            .collect(),
        other => {
            return Err(Error::Invalid {
                message: format!(
                    "{} is not accepted by the authored-artifact proposal route",
                    other.as_str()
                ),
            });
        }
    };
    let held = max_sensitivity(&members_now);
    let sensitivity = held;
    // The disclosure decision (ADR-0034 decision 1): may this principal
    // read what it is about to show the target's reviewers. It is the
    // whole warrant for the climb — the privacy floor then makes "nobody
    // climbs another principal's personal material" true with no clause
    // about personal scopes anywhere.
    //
    // At the working tier (AUTHZ-5, ADR-0038 decision 10): the question is
    // whose material this is, which the privacy floor answers identically at
    // every tier. How sensitive it is prices the *review* — the matrix
    // resolves at the set's maximum, and `restricted` there means compliance
    // and two distinct approvers before anything moves.
    //
    // Taken with the *asset kind's* own read action (PRMT-1, ADR-0049
    // decision 4): a prompt proposal's disclosure is a `PromptRead`, and
    // deciding it as another artifact family's read would ask a question
    // about a different corpus.
    let disclosed = decide_asset_read(state, &input, asset, source_scope_id)?;
    let disclosed_action = asset_read_action(asset)?;
    // Objects first: each member's address, computed from the version
    // being proposed. This is what binds the review to bytes — approvals
    // name this commit, and publishing recomputes these addresses from the
    // source rows as they stand then (ADR-0032 decision 6).
    let mut members: Vec<(String, vedaflow::hash::ObjectHash)> =
        Vec::with_capacity(members_now.len());
    for proposed in &members_now {
        let (entry, hash) = match proposed {
            // A prompt's object is already stored — the draft row's foreign
            // key required it at authoring time — so this write dedups and
            // stores nothing. It runs anyway, because a member reached
            // through the *published* sense of "the source holds it" has no
            // draft row here and this is the one line that does not care.
            Proposed::Prompt(asset) => {
                let object = vedaflow::put_prompt(&mut tx, tenant_id, asset).await?;
                (asset.entry_name(), object.hash)
            }
            // Same story as a prompt's, and for the same reason: the
            // document's object was stored at authoring — the draft row's
            // foreign key required it — so this write dedups. It runs anyway
            // because a member reached through the *published* sense of "the
            // source holds it" has no draft row here.
            //
            // What it deliberately does **not** touch is the chunks. They
            // were cut and embedded at authoring (ADR-0050 decision 4), and
            // a proposal is a ref move waiting to happen: no approval in
            // this product has ever made a network call, and this is the
            // line where that stays true.
            Proposed::ContextPack(asset) => {
                let object = vedaflow::put_context_pack(&mut tx, tenant_id, asset).await?;
                (asset.entry_name(), object.hash)
            }
        };
        members.push((entry, hash));
    }
    let snapshot = PolicySnapshot::new(
        authorized.decision.pack_name.clone(),
        authorized.decision.pack_version,
    );
    let artifact_family = match asset {
        AssetKind::Prompt => ArtifactFamily::Prompt,
        AssetKind::ContextPack => ArtifactFamily::ContextPack,
        _ => {
            return Err(Error::Invalid {
                message: "authored proposals support prompt and context-pack assets only"
                    .to_owned(),
            });
        }
    };
    let artifact_references = members
        .iter()
        .map(|(entry, hash)| {
            ArtifactReference::new(artifact_family, entry, "publish", hash.to_hex(), None)
        })
        .collect::<Result<Vec<_>>>()?;
    let proposal = vedaflow::proposals::open(
        &mut tx,
        tenant_id,
        &vedaflow::NewProposal {
            target_scope: body.scope_id,
            // Equal for a same-scope proposal, a strict descendant for a
            // climb. Both were stored from FLOW-3 onward and migration
            // 0019's transition trigger already makes both immutable, so
            // the climb needed no schema (ADR-0034 decision 8).
            source_scope: source_scope_id,
            asset,
            effect,
            members: &members,
            artifact_references: &artifact_references,
            sensitivity,
            title: &body.title,
            proposer,
            proposer_subject: &input.principal.subject,
            committed_at: Utc::now(),
            policy_snapshot: &snapshot,
        },
        &Signer::Unsigned,
    )
    .await?;

    if target_position > 0 {
        metrics::counter!(
            PROPOSAL_CLIMBS_TOTAL,
            "levels" => climb_level_bucket(target_position),
            "from" => source.kind.as_str(),
            "to" => node.kind.as_str(),
        )
        .increment(1);
        tracing::info!(
            proposal.id = %proposal.id,
            scope.source = %source_scope_id,
            scope.target = %body.scope_id,
            climb.levels = target_position,
            "proposal climbs {target_position} level(s) to {}", node.slug
        );
    }

    let entries: Vec<String> = members.iter().map(|(name, _)| name.clone()).collect();
    let requirement = approvals::resolve(
        state,
        &mut tx,
        tenant_id,
        &input,
        &approvals::Requested {
            target: &node,
            asset,
            sensitivity,
            entries: &entries,
        },
    )
    .await?;
    let outstanding = requirement.outstanding(&[]);
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::ProposalOpened,
        Resource::Scope(body.scope_id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::ProposalOpen, &authorized),
            "proposal_id": proposal.id,
            "asset": asset.as_str(),
            "channel": Channel::Published.as_str(),
            "title": body.title,
            "sensitivity": sensitivity.as_str(),
            "commit": proposal.commit.to_hex(),
            "artifact_references": artifact_reference_audit(&proposal),
            // Where it came from, and — when that is not the target — the
            // second governed decision the climb took: the proposer's read
            // at the source, which is the disclosure this proposal makes
            // (ADR-0034 decisions 1 and 9).
            "source_scope_id": source_scope_id,
            "target_scope_id": body.scope_id,
            "climb": (source_scope_id != body.scope_id).then(|| json!({
                "levels": target_position,
                "source_read": audit::decision_context(disclosed_action, &disclosed),
            })),
            // Names and addresses, never content.
            "members": members.iter().map(|(name, hash)| json!({
                "member": name,
                "object_hash": hash.to_hex(),
            })).collect::<Vec<_>>(),
            "approvals": approvals::audit_context(&requirement, &outstanding),
        }),
    )
    .await?;
    commit(tx).await?;

    Ok(Json(OpenResponse {
        summary: render(
            &proposal,
            &ScopePaths {
                target: Some(node.slug.clone()),
                source: Some(source.slug.clone()),
            },
            &requirement,
            &outstanding,
        ),
    }))
}

// ── Review ─────────────────────────────────────────────────────────────

#[derive(Deserialize, utoipa::ToSchema)]
#[schema(as = ProposalReviewBody)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReviewBody {
    /// Exact proposal commit the reviewer inspected.
    expected_commit: String,
    /// What the reviewer wants to say. Optional on an approval; a
    /// rejection carries its reason in `reason` instead.
    #[serde(default)]
    comment: Option<String>,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[schema(as = ProposalRejectBody)]
#[serde(deny_unknown_fields)]
pub(crate) struct RejectBody {
    /// Exact proposal commit the reviewer inspected.
    expected_commit: String,
    /// Why. Mandatory — a rejection an auditor cannot read the reason for
    /// is not a review, and FLOW-5 inherits this reason for its
    /// per-level denials.
    reason: String,
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = ProposalReviewResponse)]
pub(crate) struct ReviewResponse {
    #[serde(flatten)]
    summary: ProposalSummary,
    /// What this act contributed: the roles it counted under.
    counted_roles: Vec<String>,
}

/// `POST /v1/proposals/{id}/approve` — cast an approval.
#[utoipa::path(
    post,
    path = "/v1/proposals/{id}/approve",
    operation_id = "approve_proposal",
    tag = "proposals",
    params(("id" = String, Path, format = "uuid")),
    request_body(content = ReviewBody, description = "Commit-bound review verdict"),
    responses(
        (status = 200, description = "The proposal after this approval", body = ReviewResponse),
        (status = 400, description = "The review comment is invalid", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Proposal review is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "The proposal is absent", body = crate::workspaces::ApiErrorBody),
        (status = 409, description = "The proposal is closed or this approval advances nothing", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "proposals.approve", skip_all)]
pub(crate) async fn approve(
    State(state): State<AppState>,
    Path(id): Path<ProposalId>,
    payload: std::result::Result<Json<ReviewBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        approve_inner(&state, id, &body.expected_commit, body.comment.as_deref()).await
    }
    .await;
    respond(&state, "approve", result).await
}

async fn approve_inner(
    state: &AppState,
    id: ProposalId,
    expected_commit: &str,
    comment: Option<&str>,
) -> Result<Json<ReviewResponse>> {
    check_text("comment", comment)?;
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let proposal = load(&mut tx, tenant_id, id).await?;
    let node = target_node(&mut tx, tenant_id, &proposal).await?;
    let input = authz::gather(
        state,
        &mut tx,
        Some(&node),
        synveda_store::anchors::AnchorSelection::none(),
        Vec::new(),
    )
    .await?;
    let authorized = authz::decide(
        state,
        &input,
        Action::ProposalReview,
        Resource::Scope(node.id),
    )?;
    require_open(&proposal)?;
    require_expected_commit(&proposal, expected_commit)?;
    let approver = identity_of(&input)?;

    let requirement = requirement_for(state, &mut tx, tenant_id, &input, &node, &proposal).await?;
    approvals::require_review_actor(&requirement, id, proposal.proposer_id, approver)?;
    let recorded = vedaflow::proposals::approvals(&mut tx, tenant_id, id).await?;
    let cast = vedaflow::proposals::cast_for(&recorded, proposal.commit);
    let outstanding = requirement.outstanding(&cast);
    let candidate = CastApproval {
        identity: approver,
        subject: input.principal.subject.clone(),
        roles: approvals::roles_at(&input, &node),
    };
    // An approval that advances nothing is refused rather than recorded:
    // a vote that governs nothing is noise in a log a reviewer and an
    // auditor both read (ADR-0032 decision 5).
    if !outstanding.advanced_by(&candidate) {
        return Err(Error::Conflict {
            message: if outstanding.is_empty() {
                format!(
                    "proposal {id} already has the approvals it needs; publish it \
                     with POST /v1/proposals/{id}/publish"
                )
            } else {
                format!(
                    "this principal's roles ({}) satisfy nothing this proposal still \
                     needs: {}",
                    role_list(&candidate.roles),
                    outstanding.describe()
                )
            },
        });
    }

    vedaflow::proposals::record_approval(
        &mut tx,
        tenant_id,
        &vedaflow::NewApproval {
            proposal: id,
            commit: proposal.commit,
            approver,
            approver_subject: &input.principal.subject,
            verdict: Verdict::Approve,
            roles: &candidate.roles,
            comment,
        },
    )
    .await?;
    vedaflow::proposals::act("approved", proposal.asset);

    let mut now = cast;
    now.push(candidate.clone());
    let after = requirement.outstanding(&now);
    let paths = ScopePaths::resolve(&mut tx, tenant_id, &proposal, &node).await?;
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::ProposalApproved,
        Resource::Scope(node.id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::ProposalReview, &authorized),
            "proposal_id": id,
            "commit": proposal.commit.to_hex(),
            "source_scope_id": proposal.source_scope_id,
            "target_scope_id": proposal.target_scope_id,
            "artifact_references": artifact_reference_audit(&proposal),
            "roles": candidate.roles.iter().map(|role| role.as_str()).collect::<Vec<_>>(),
            "comment": comment,
            "approvals": approvals::audit_context(&requirement, &after),
        }),
    )
    .await?;
    commit(tx).await?;

    Ok(Json(ReviewResponse {
        summary: render(&proposal, &paths, &requirement, &after),
        counted_roles: candidate
            .roles
            .iter()
            .map(|role| role.as_str().to_owned())
            .collect(),
    }))
}

/// `POST /v1/proposals/{id}/reject` — close a proposal with a reason.
///
/// Terminal. A revision is a new proposal (ADR-0032 decision 12): a
/// proposal whose content changes under its approvals is a review nobody
/// consented to, and "withdraw and open a new one" says that plainly in
/// the trail.
#[utoipa::path(
    post,
    path = "/v1/proposals/{id}/reject",
    operation_id = "reject_proposal",
    tag = "proposals",
    params(("id" = String, Path, format = "uuid")),
    request_body = RejectBody,
    responses(
        (status = 200, description = "The rejected proposal", body = ProposalSummary),
        (status = 400, description = "The rejection reason is invalid", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Proposal review is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "The proposal is absent", body = crate::workspaces::ApiErrorBody),
        (status = 409, description = "The proposal is already closed", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "proposals.reject", skip_all)]
pub(crate) async fn reject(
    State(state): State<AppState>,
    Path(id): Path<ProposalId>,
    payload: std::result::Result<Json<RejectBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        reject_inner(&state, id, &body.expected_commit, &body.reason).await
    }
    .await;
    respond(&state, "reject", result).await
}

async fn reject_inner(
    state: &AppState,
    id: ProposalId,
    expected_commit: &str,
    reason: &str,
) -> Result<Json<ProposalSummary>> {
    check_text("reason", Some(reason))?;
    if reason.trim().is_empty() {
        return Err(Error::Invalid {
            message: "reason must say why".to_owned(),
        });
    }
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let proposal = load(&mut tx, tenant_id, id).await?;
    let node = target_node(&mut tx, tenant_id, &proposal).await?;
    let input = authz::gather(
        state,
        &mut tx,
        Some(&node),
        synveda_store::anchors::AnchorSelection::none(),
        Vec::new(),
    )
    .await?;
    let authorized = authz::decide(
        state,
        &input,
        Action::ProposalReview,
        Resource::Scope(node.id),
    )?;
    require_open(&proposal)?;
    require_expected_commit(&proposal, expected_commit)?;
    let reviewer = identity_of(&input)?;
    let roles = approvals::roles_at(&input, &node);
    let requirement = requirement_for(state, &mut tx, tenant_id, &input, &node, &proposal).await?;
    approvals::require_review_actor(&requirement, id, proposal.proposer_id, reviewer)?;

    vedaflow::proposals::record_approval(
        &mut tx,
        tenant_id,
        &vedaflow::NewApproval {
            proposal: id,
            commit: proposal.commit,
            approver: reviewer,
            approver_subject: &input.principal.subject,
            verdict: Verdict::Reject,
            roles: &roles,
            comment: Some(reason),
        },
    )
    .await?;
    close(
        &mut tx,
        tenant_id,
        id,
        ProposalState::Rejected,
        reviewer,
        Some(reason),
    )
    .await?;
    vedaflow::proposals::act("rejected", proposal.asset);

    let paths = ScopePaths::resolve(&mut tx, tenant_id, &proposal, &node).await?;
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::ProposalRejected,
        Resource::Scope(node.id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::ProposalReview, &authorized),
            "proposal_id": id,
            "commit": proposal.commit.to_hex(),
            // The level a denial happened at, and what it refused to take:
            // the AC's "denial at any level audited with reason" is this
            // event, with the reason ADR-0032 decision 12 already made
            // mandatory (ADR-0034 decision 9).
            "source_scope_id": proposal.source_scope_id,
            "target_scope_id": proposal.target_scope_id,
            "artifact_references": artifact_reference_audit(&proposal),
            "roles": roles.iter().map(|role| role.as_str()).collect::<Vec<_>>(),
            "reason": reason,
        }),
    )
    .await?;
    commit(tx).await?;

    let mut closed = proposal;
    closed.state = ProposalState::Rejected;
    closed.close_reason = Some(reason.to_owned());
    let outstanding = requirement.outstanding(&[]);
    Ok(Json(render(&closed, &paths, &requirement, &outstanding)))
}

/// `POST /v1/proposals/{id}/withdraw` — the proposer closes their own.
///
/// Authorized by `ProposalOpen` at the target *and* by being the
/// proposer: withdrawing is the proposer's act, and a reviewer who wants
/// it gone rejects it with a reason instead.
#[utoipa::path(
    post,
    path = "/v1/proposals/{id}/withdraw",
    operation_id = "withdraw_proposal",
    tag = "proposals",
    params(("id" = String, Path, format = "uuid")),
    responses(
        (status = 200, description = "The withdrawn proposal", body = ProposalSummary),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Only the proposer may withdraw this proposal", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "The proposal is absent", body = crate::workspaces::ApiErrorBody),
        (status = 409, description = "The proposal is already closed", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "proposals.withdraw", skip_all)]
pub(crate) async fn withdraw(
    State(state): State<AppState>,
    Path(id): Path<ProposalId>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let proposal = load(&mut tx, tenant_id, id).await?;
        let node = target_node(&mut tx, tenant_id, &proposal).await?;
        let input = authz::gather(
            &state,
            &mut tx,
            Some(&node),
            synveda_store::anchors::AnchorSelection::none(),
            Vec::new(),
        )
        .await?;
        let authorized = authz::decide(
            &state,
            &input,
            Action::ProposalOpen,
            Resource::Scope(node.id),
        )?;
        require_open(&proposal)?;
        let actor = identity_of(&input)?;
        if actor != proposal.proposer_id {
            return Err(Error::PolicyDenied {
                action: "proposal.withdraw".to_owned(),
                resource: format!("proposal {id}"),
                reason: "only the proposer withdraws a proposal; a reviewer rejects it \
                         with a reason"
                    .to_owned(),
            });
        }
        close(
            &mut tx,
            tenant_id,
            id,
            ProposalState::Withdrawn,
            actor,
            None,
        )
        .await?;
        vedaflow::proposals::act("withdrawn", proposal.asset);
        let requirement =
            requirement_for(&state, &mut tx, tenant_id, &input, &node, &proposal).await?;
        let paths = ScopePaths::resolve(&mut tx, tenant_id, &proposal, &node).await?;
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::ProposalWithdrawn,
            Resource::Scope(node.id).to_string(),
            Outcome::Success,
            json!({
                "authz": audit::decision_context(Action::ProposalOpen, &authorized),
                "proposal_id": id,
                "commit": proposal.commit.to_hex(),
                "source_scope_id": proposal.source_scope_id,
                "target_scope_id": proposal.target_scope_id,
                "artifact_references": artifact_reference_audit(&proposal),
            }),
        )
        .await?;
        commit(tx).await?;
        let mut closed = proposal;
        closed.state = ProposalState::Withdrawn;
        let outstanding = requirement.outstanding(&[]);
        Ok(Json(render(&closed, &paths, &requirement, &outstanding)))
    }
    .await;
    respond(&state, "withdraw", result).await
}

// ── Publish: the proposal's effect ─────────────────────────────────────

/// `POST /v1/proposals/{id}/apply` — run an approved typed aggregate effect.
/// The artifact command layer repeats ownership, PDP and revision checks at
/// this boundary; approvals never become write authority by themselves.
#[utoipa::path(
    post,
    path = "/v1/proposals/{id}/apply",
    operation_id = "apply_proposal",
    tag = "proposals",
    params(("id" = String, Path, format = "uuid")),
    responses(
        (status = 200, description = "The typed aggregate mutation result", body = serde_json::Value),
        (status = 400, description = "The proposal does not carry a typed apply effect", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Applying the governed mutation is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "The proposal or aggregate is absent", body = crate::workspaces::ApiErrorBody),
        (status = 409, description = "The proposal is stale, closed, or incomplete", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "proposals.apply", skip_all)]
pub(crate) async fn apply(State(state): State<AppState>, Path(id): Path<ProposalId>) -> Response {
    let result = apply_inner(&state, id).await;
    respond(&state, "apply", result).await
}

async fn apply_inner(state: &AppState, id: ProposalId) -> Result<Json<serde_json::Value>> {
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let proposal = load(&mut tx, tenant_id, id).await?;
    drop(tx);
    let value = match (proposal.asset, proposal.effect) {
        (AssetKind::Knowledge, ProposalEffect::Apply) => {
            serde_json::to_value(crate::knowledge::apply_reviewed(state, id).await?)
        }
        (AssetKind::Skill, ProposalEffect::Apply) => {
            serde_json::to_value(crate::skills::apply_reviewed(state, id).await?)
        }
        (AssetKind::Tool, ProposalEffect::Apply) => {
            serde_json::to_value(crate::tool_registry::apply_reviewed(state, id).await?)
        }
        (AssetKind::Configuration, ProposalEffect::Apply) => {
            serde_json::to_value(crate::configuration::apply_reviewed(state, id).await?)
        }
        (AssetKind::Policy, ProposalEffect::Apply) => {
            serde_json::to_value(crate::relaxations::apply_reviewed(state, id).await?)
        }
        _ => {
            return Err(Error::Invalid {
                message: format!(
                    "proposal {id} carries {}/{}; only typed apply effects use this route",
                    proposal.asset.as_str(),
                    proposal.effect.as_str()
                ),
            });
        }
    }
    .map_err(|err| Error::Internal {
        message: format!("encode result for proposal {id}: {err}"),
    })?;
    Ok(Json(value))
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = ProposalPublishResponse)]
pub(crate) struct PublishResponse {
    #[schema(value_type = String, format = "uuid")]
    proposal_id: ProposalId,
    #[schema(value_type = String, format = "uuid")]
    scope_id: ScopeId,
    channel: String,
    /// The commit the channel now points at. Its parents are
    /// `[previous head, proposal commit]` — first-parent mainline as in
    /// git, so lineage is a fact about the graph (ADR-0032 decision 10).
    commit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<String>,
    /// The proposal commit, this publication's second parent.
    proposal_commit: String,
    members: usize,
    added: usize,
}

/// `POST /v1/proposals/{id}/publish` — run an approved proposal's effect.
#[utoipa::path(
    post,
    path = "/v1/proposals/{id}/publish",
    operation_id = "publish_proposal",
    tag = "proposals",
    params(("id" = String, Path, format = "uuid")),
    responses(
        (status = 200, description = "The published channel state", body = PublishResponse),
        (status = 400, description = "The proposal does not carry a publish effect", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Publishing the proposal is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "The proposal or target scope is absent", body = crate::workspaces::ApiErrorBody),
        (status = 409, description = "The proposal is stale, closed, or lacks approval", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "proposals.publish", skip_all)]
pub(crate) async fn publish(State(state): State<AppState>, Path(id): Path<ProposalId>) -> Response {
    let result = publish_inner(&state, id).await;
    respond(&state, "publish", result).await
}

async fn publish_inner(state: &AppState, id: ProposalId) -> Result<Json<PublishResponse>> {
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let proposal = load(&mut tx, tenant_id, id).await?;
    let node = target_node(&mut tx, tenant_id, &proposal).await?;
    let input = authz::gather(
        state,
        &mut tx,
        Some(&node),
        synveda_store::anchors::AnchorSelection::none(),
        Vec::new(),
    )
    .await?;
    // The same two decisions the direct route takes (ADR-0031
    // decision 12): may this principal publish here, and may it read what
    // it is about to declare reviewed. The approvals go *in front of*
    // these; they do not replace them.
    let authorized = authz::decide(
        state,
        &input,
        Action::ChannelPublish,
        Resource::Scope(node.id),
    )?;
    // At the working tier, like the direct route (ADR-0038 decision 10):
    // running an approved effect governs material, it does not compose it,
    // and the tier was priced by the matrix these approvals satisfied. With
    // the asset kind's own read action since PRMT-1 (ADR-0049 decision 4) —
    // which is what keeps a steward, who reads no content in any pack, from
    // running a prompt publication's effect.
    decide_asset_read(state, &input, proposal.asset, node.id)?;
    require_open(&proposal)?;
    require_effect(&proposal, ProposalEffect::Published, "publish")?;
    let publisher = identity_of(&input)?;

    let requirement = requirement_for(state, &mut tx, tenant_id, &input, &node, &proposal).await?;
    let recorded = vedaflow::proposals::approvals(&mut tx, tenant_id, id).await?;
    let cast = vedaflow::proposals::cast_for(&recorded, proposal.commit);
    let outstanding = requirement.outstanding(&cast);
    if !outstanding.is_empty() {
        metrics::counter!(PUBLISH_REVIEW_REQUIRED_TOTAL, "surface" => "proposal").increment(1);
        return Err(Error::Conflict {
            message: format!(
                "proposal {id} still needs {}; it cannot publish yet",
                outstanding.describe()
            ),
        });
    }
    approvals::require_effect_actor(&requirement, id, proposal.proposer_id, &cast, publisher)?;

    // Approvals bind bytes. Recompute every member's address from the
    // artifact as it stands *now* and require it to equal what the approved
    // commit named — otherwise the content moved after the review, and
    // publishing it would launder unreviewed text through a completed
    // approval (ADR-0032 decision 6). Then re-ask whether the source still
    // holds the material, which is the same check one scope over
    // (ADR-0034 decision 7).
    let proposed = vedaflow::proposals::members(&mut tx, tenant_id, proposal.commit).await?;
    if proposal.asset == AssetKind::ContextPack {
        return publish_documents(
            tx,
            tenant_id,
            id,
            &proposal,
            publisher,
            &authorized,
            &requirement,
            &cast,
            &outstanding,
            &proposed,
        )
        .await;
    }
    if proposal.asset == AssetKind::Prompt {
        return publish_prompts(
            tx,
            tenant_id,
            id,
            &proposal,
            publisher,
            &authorized,
            &requirement,
            &cast,
            &outstanding,
            &proposed,
        )
        .await;
    }
    Err(Error::Invalid {
        message: format!(
            "proposal {id} carries removed asset kind {}; raw records cannot be published",
            proposal.asset.as_str()
        ),
    })
}

/// The publish effect for a context-pack proposal (PRMT-2, ADR-0050
/// decision 1).
///
/// [`publish_prompts`] one table over, and the same three properties:
/// approvals bind bytes, so every member's address is recomputed from the
/// document as it stands *now* and required to equal what the approved
/// commit named; the source must still hold it; and every refusal is a
/// `Conflict`, because the request was well formed when it was approved and
/// what moved is the world.
///
/// It writes `context-pack/published` and chains the
/// `vedaflow.channel.published` event with `asset` reading
/// `context-pack` — the same governed act with the same consequence
/// (ADR-0019 decision 4).
///
/// **What it does not do is touch a chunk.** Publication is a ref move; the
/// chunk rows were written with their embeddings at authoring, and this
/// commit names addresses that only exist because that transaction
/// committed. That is why there is no window in which half a pack is live,
/// and why a FLOW-7 rewind of this channel restores a previous version with
/// no re-embedding at all (decisions 5 and 6).
#[allow(clippy::too_many_arguments)]
async fn publish_documents(
    mut tx: sqlx::Transaction<'static, sqlx::Postgres>,
    tenant_id: TenantId,
    id: ProposalId,
    proposal: &vedaflow::StoredProposal,
    publisher: IdentityId,
    authorized: &crate::authz::Authorized,
    requirement: &ApprovalRequirement,
    cast: &[CastApproval],
    outstanding: &synveda_types::Outstanding,
    proposed: &[vedaflow::ChannelMember],
) -> Result<Json<PublishResponse>> {
    let source = proposal.source_scope_id;
    let paths: Vec<DocumentPath> = proposed
        .iter()
        .map(|member| {
            member
                .name
                .parse::<DocumentPath>()
                .map_err(|err| Error::Internal {
                    message: format!(
                        "proposal member {:?} is not a document path: {err}",
                        member.name
                    ),
                })
        })
        .collect::<Result<_>>()?;
    let drafts: std::collections::HashMap<DocumentPath, [u8; 32]> =
        synveda_store::packs::list_all_documents(&mut *tx, tenant_id, source)
            .await?
            .into_iter()
            .map(|document| {
                (
                    DocumentPath::new(document.pack_name.clone(), document.document_name.clone()),
                    document.object_hash,
                )
            })
            .collect();
    let published_at_source = published_documents_at(&mut tx, tenant_id, source).await?;

    let moved = |what: &str, path: &DocumentPath| Error::Conflict {
        message: format!(
            "context pack document {path} {what} after this proposal was approved; \
             withdraw it and open a new one so the change is reviewed"
        ),
    };
    let mut members: Vec<(String, vedaflow::hash::ObjectHash)> = Vec::with_capacity(paths.len());
    for (member, path) in proposed.iter().zip(&paths) {
        match drafts.get(path) {
            // The draft lives at the source: its current address must be the
            // one the approvals bound. An edit since the review moved it —
            // and moved its chunks off the published set with it, which is
            // the same fact seen from the read side (ADR-0050 decision 3).
            Some(address) if member.object.as_bytes() == address => {}
            Some(_) => return Err(moved("changed", path)),
            None => {
                if published_at_source.get(path) != Some(&member.object) {
                    return Err(Error::Conflict {
                        message: format!(
                            "scope {source} no longer holds context pack document {path}; \
                             the climb was approved against material its source has since \
                             given up"
                        ),
                    });
                }
            }
        }
        members.push((member.name.clone(), member.object));
    }

    let channel = vedaflow::ChannelRef::context_pack(Channel::Published);
    let snapshot = PolicySnapshot::new(
        authorized.decision.pack_name.clone(),
        authorized.decision.pack_version,
    );
    let committed = vedaflow::publish(
        &mut tx,
        tenant_id,
        &vedaflow::ChannelWrite {
            scope: proposal.target_scope_id,
            channel,
            members: &members,
            merge_parents: &[proposal.commit],
            author: publisher,
            message: &proposal.title,
            committed_at: Utc::now(),
            policy_snapshot: &snapshot,
        },
        &Signer::Unsigned,
    )
    .await?;
    close(
        &mut tx,
        tenant_id,
        id,
        ProposalState::Published,
        publisher,
        None,
    )
    .await?;
    vedaflow::proposals::act("published", proposal.asset);

    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::ChannelPublished,
        Resource::Scope(proposal.target_scope_id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::ChannelPublish, authorized),
            "channel": channel.name(),
            "asset": channel.asset.as_str(),
            "message": proposal.title,
            "proposal_id": id,
            "proposal_commit": proposal.commit.to_hex(),
            "sensitivity": proposal.sensitivity.as_str(),
            "source_scope_id": source,
            "target_scope_id": proposal.target_scope_id,
            "artifact_references": artifact_reference_audit(proposal),
            // Paths and addresses, never document text.
            "records": members.iter().map(|(name, hash)| json!({
                "member": name,
                "object_hash": hash.to_hex(),
            })).collect::<Vec<_>>(),
            "commit": committed.commit.to_hex(),
            "parent": committed.parent.map(|parent| parent.to_hex()),
            "members": committed.entries,
            "added": committed.added,
            "approvals": approvals::audit_context(requirement, outstanding),
            "approvers": cast.len(),
        }),
    )
    .await?;
    commit(tx).await?;

    Ok(Json(PublishResponse {
        proposal_id: id,
        scope_id: proposal.target_scope_id,
        channel: channel.name(),
        commit: committed.commit.to_hex(),
        parent: committed.parent.map(|parent| parent.to_hex()),
        proposal_commit: proposal.commit.to_hex(),
        members: committed.entries,
        added: committed.added,
    }))
}

/// The publish effect for a prompt proposal (PRMT-1, ADR-0049 decision 6).
///
/// The same act as its memory sibling, line for line — approvals bind
/// bytes, so every member's address is recomputed from the version as it
/// stands *now* and required to equal what the approved commit named, and
/// the source must still hold it. What differs is only where "as it stands
/// now" is read from: a draft row at the source scope, or, for a climb of
/// something already published there, the source's own tree.
///
/// It writes `prompt/published` rather than `memory/published`, and it
/// chains the same `vedaflow.channel.published` event with `asset` reading
/// `prompt` — the same governed act with the same consequence, so a second
/// action asserting it would be a fact an auditor has to reconcile
/// (ADR-0019 decision 4).
#[allow(clippy::too_many_arguments)]
async fn publish_prompts(
    mut tx: sqlx::Transaction<'static, sqlx::Postgres>,
    tenant_id: TenantId,
    id: ProposalId,
    proposal: &vedaflow::StoredProposal,
    publisher: IdentityId,
    authorized: &crate::authz::Authorized,
    requirement: &ApprovalRequirement,
    cast: &[CastApproval],
    outstanding: &synveda_types::Outstanding,
    proposed: &[vedaflow::ChannelMember],
) -> Result<Json<PublishResponse>> {
    let source = proposal.source_scope_id;
    let names: Vec<PromptName> = proposed
        .iter()
        .map(|member| {
            member
                .name
                .parse::<PromptName>()
                .map_err(|err| Error::Internal {
                    message: format!(
                        "proposal member {:?} is not a prompt name: {err}",
                        member.name
                    ),
                })
        })
        .collect::<Result<_>>()?;
    let drafts: std::collections::HashMap<PromptName, [u8; 32]> =
        synveda_store::prompts::read_many(&mut *tx, tenant_id, source, &names)
            .await?
            .into_iter()
            .map(|draft| (draft.template.name.clone(), draft.object_hash))
            .collect();
    let published_at_source = published_prompts_at(&mut tx, tenant_id, source).await?;

    // Every refusal here is a `Conflict`, never an `Invalid`: the request
    // was well formed when it was approved — what moved is the world,
    // between the review and its effect (ADR-0034 decision 7).
    let moved = |what: &str, name: &PromptName| Error::Conflict {
        message: format!(
            "prompt {name} {what} after this proposal was approved; withdraw it and \
             open a new one so the change is reviewed"
        ),
    };
    let mut members: Vec<(String, vedaflow::hash::ObjectHash)> = Vec::with_capacity(names.len());
    for (member, name) in proposed.iter().zip(&names) {
        match drafts.get(name) {
            // The draft lives at the source: its current address must be the
            // one the approvals bound.
            Some(address) if member.object.as_bytes() == address => {}
            Some(_) => return Err(moved("changed", name)),
            // No draft: the source must still publish exactly these bytes.
            // A rewind at the source (FLOW-7) or a republication at a
            // different address takes the member out of both senses at once.
            None => {
                if published_at_source.get(name) != Some(&member.object) {
                    return Err(Error::Conflict {
                        message: format!(
                            "scope {source} no longer holds prompt {name}; the climb was \
                             approved against material its source has since given up"
                        ),
                    });
                }
            }
        }
        members.push((member.name.clone(), member.object));
    }

    let channel = vedaflow::ChannelRef::prompt(Channel::Published);
    let snapshot = PolicySnapshot::new(
        authorized.decision.pack_name.clone(),
        authorized.decision.pack_version,
    );
    let committed = vedaflow::publish(
        &mut tx,
        tenant_id,
        &vedaflow::ChannelWrite {
            scope: proposal.target_scope_id,
            channel,
            members: &members,
            merge_parents: &[proposal.commit],
            author: publisher,
            message: &proposal.title,
            committed_at: Utc::now(),
            policy_snapshot: &snapshot,
        },
        &Signer::Unsigned,
    )
    .await?;
    close(
        &mut tx,
        tenant_id,
        id,
        ProposalState::Published,
        publisher,
        None,
    )
    .await?;
    vedaflow::proposals::act("published", proposal.asset);

    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::ChannelPublished,
        Resource::Scope(proposal.target_scope_id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::ChannelPublish, authorized),
            "channel": channel.name(),
            "asset": channel.asset.as_str(),
            "message": proposal.title,
            "proposal_id": id,
            "proposal_commit": proposal.commit.to_hex(),
            "sensitivity": proposal.sensitivity.as_str(),
            "source_scope_id": source,
            "target_scope_id": proposal.target_scope_id,
            "artifact_references": artifact_reference_audit(proposal),
            // Names and addresses, never template text.
            "records": members.iter().map(|(name, hash)| json!({
                "member": name,
                "object_hash": hash.to_hex(),
            })).collect::<Vec<_>>(),
            "commit": committed.commit.to_hex(),
            "parent": committed.parent.map(|parent| parent.to_hex()),
            "members": committed.entries,
            "added": committed.added,
            "approvals": approvals::audit_context(requirement, outstanding),
            "approved_by": cast.iter().map(|approval| json!({
                "identity_id": approval.identity,
                "subject": approval.subject,
                "roles": approval.roles.iter().map(|role| role.as_str()).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        }),
    )
    .await?;
    commit(tx).await?;

    Ok(Json(PublishResponse {
        proposal_id: id,
        scope_id: proposal.target_scope_id,
        channel: channel.name(),
        commit: committed.commit.to_hex(),
        parent: committed.parent.map(|parent| parent.to_hex()),
        proposal_commit: proposal.commit.to_hex(),
        members: committed.entries,
        added: committed.added,
    }))
}

/// Refuses a proposal whose effect is not the one this route runs.
///
/// A route per effect, and each one checks that an authored publication
/// cannot execute a typed Knowledge change (or vice versa).
fn require_effect(
    proposal: &vedaflow::StoredProposal,
    expected: ProposalEffect,
    route: &str,
) -> Result<()> {
    if proposal.effect == expected {
        return Ok(());
    }
    Err(Error::Invalid {
        message: format!(
            "proposal {} has effect {}; {route} runs {expected} proposals",
            proposal.id, proposal.effect
        ),
    })
}

// ── Shared plumbing ────────────────────────────────────────────────────

/// Loads a proposal or answers the uniform 404 — the same shape every
/// governed plane uses, so a cross-tenant probe never sees a policy
/// denial oracle (ADR-0012 decision 7).
async fn load(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    id: ProposalId,
) -> Result<vedaflow::StoredProposal> {
    vedaflow::proposals::read(tx, tenant_id, id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("proposal {id}"),
        })
}

/// The target node, or the uniform 404 when it was deleted under the
/// proposal (a revoked agent's leaf): the proposal's rows await disposal
/// (TEN-5), and there is nothing to decide against meanwhile.
async fn target_node(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    proposal: &vedaflow::StoredProposal,
) -> Result<Scope> {
    found(
        scopes::get(&mut *tx, tenant_id, proposal.target_scope_id).await?,
        tenant_id,
        proposal.target_scope_id,
    )
}

fn require_open(proposal: &vedaflow::StoredProposal) -> Result<()> {
    if proposal.state.is_terminal() {
        return Err(Error::Conflict {
            message: format!(
                "proposal {} is {}; closed proposals are history",
                proposal.id, proposal.state
            ),
        });
    }
    Ok(())
}

fn require_expected_commit(
    proposal: &vedaflow::StoredProposal,
    expected_commit: &str,
) -> Result<()> {
    let expected: vedaflow::CommitHash = expected_commit.parse()?;
    if expected == proposal.commit {
        return Ok(());
    }
    Err(Error::Conflict {
        message: format!(
            "proposal {} is at commit {}; the reviewed commit {} is stale",
            proposal.id,
            proposal.commit.to_hex(),
            expected.to_hex()
        ),
    })
}

/// The proposing/reviewing identity. A verified subject with no identity
/// row cannot reach here — every pack requires either a binding or
/// placement — but the check is explicit rather than an unwrap.
fn identity_of(input: &DecisionInput) -> Result<IdentityId> {
    input
        .identity
        .as_ref()
        .map(|identity| identity.id)
        .ok_or_else(|| Error::Invalid {
            message: "proposals require a provisioned identity".to_owned(),
        })
}

async fn close(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    id: ProposalId,
    state: ProposalState,
    by: IdentityId,
    reason: Option<&str>,
) -> Result<()> {
    if vedaflow::proposals::close(tx, tenant_id, id, state, by, reason).await? {
        return Ok(());
    }
    // Someone else closed it between the read and the write. Reported,
    // never papered over: the caller's verdict was about a proposal that
    // is no longer open.
    Err(Error::Conflict {
        message: format!("proposal {id} was closed by another reviewer; re-read it"),
    })
}

/// The proposal's requirement, resolved live against its own member set.
async fn requirement_for(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    input: &DecisionInput,
    node: &Scope,
    proposal: &vedaflow::StoredProposal,
) -> Result<ApprovalRequirement> {
    let members = vedaflow::proposals::members(tx, tenant_id, proposal.commit).await?;
    let entries: Vec<String> = members.into_iter().map(|member| member.name).collect();
    approvals::resolve(
        state,
        tx,
        tenant_id,
        input,
        &approvals::Requested {
            target: node,
            asset: proposal.asset,
            sensitivity: proposal.sensitivity,
            entries: &entries,
        },
    )
    .await
}

/// Resolves the requirement and counts the recorded approvals for one
/// proposal, then renders it.
async fn summarise(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    input: &DecisionInput,
    proposal: &vedaflow::StoredProposal,
) -> Result<ProposalSummary> {
    // The listing decides against the *requested* resource; a proposal at
    // another scope is still rendered with its own target's requirement,
    // which is the honest answer to "what does this need".
    let node = match scopes::get(&mut *tx, tenant_id, proposal.target_scope_id).await? {
        Some(node) => node,
        // The target vanished (TEN-5's disposal window): render what the
        // pack asks for at the tenant default rather than dropping the row.
        None => {
            let requirement = ApprovalRequirement::default();
            return Ok(render(
                proposal,
                &ScopePaths::default(),
                &requirement,
                &requirement.outstanding(&[]),
            ));
        }
    };
    let paths = ScopePaths::resolve(tx, tenant_id, proposal, &node).await?;
    let requirement = requirement_for(state, tx, tenant_id, input, &node, proposal).await?;
    let recorded = vedaflow::proposals::approvals(tx, tenant_id, proposal.id).await?;
    let cast = vedaflow::proposals::cast_for(&recorded, proposal.commit);
    let outstanding = requirement.outstanding(&cast);
    Ok(render(proposal, &paths, &requirement, &outstanding))
}

/// The two scopes a proposal names, as a person reads them (FLOW-6,
/// ADR-0035 decision 9). `None` on either side is TEN-5's disposal
/// window: the scope is gone and the proposal's rows await disposal.
#[derive(Default)]
struct ScopePaths {
    target: Option<String>,
    source: Option<String>,
}

impl ScopePaths {
    /// Resolves both from the target node the caller already holds. The
    /// source costs a read only when it differs — that is, only for a
    /// climb, which is the only case where the two paths say different
    /// things.
    async fn resolve(
        tx: &mut sqlx::PgConnection,
        tenant_id: synveda_types::TenantId,
        proposal: &vedaflow::StoredProposal,
        target: &Scope,
    ) -> Result<Self> {
        let source = if proposal.source_scope_id == proposal.target_scope_id {
            Some(target.slug.clone())
        } else {
            scopes::get(&mut *tx, tenant_id, proposal.source_scope_id)
                .await?
                .map(|node| node.slug)
        };
        Ok(Self {
            target: Some(target.slug.clone()),
            source,
        })
    }
}

fn render(
    proposal: &vedaflow::StoredProposal,
    paths: &ScopePaths,
    requirement: &ApprovalRequirement,
    outstanding: &synveda_types::Outstanding,
) -> ProposalSummary {
    ProposalSummary {
        id: proposal.id,
        target_scope_id: proposal.target_scope_id,
        source_scope_id: proposal.source_scope_id,
        target_scope_path: paths.target.clone(),
        source_scope_path: paths.source.clone(),
        asset: proposal.asset.as_str().to_owned(),
        effect: proposal.effect,
        state: ProposalView::of(proposal.state, outstanding.is_empty()),
        sensitivity: proposal.sensitivity,
        title: proposal.title.clone(),
        commit: proposal.commit.to_hex(),
        proposer_id: proposal.proposer_id,
        proposer_subject: proposal.proposer_subject.clone(),
        created_at: proposal.created_at,
        updated_at: proposal.updated_at,
        closed_at: proposal.closed_at,
        close_reason: proposal.close_reason.clone(),
        artifact_references: proposal
            .artifact_references
            .iter()
            .map(ArtifactReferenceView::from)
            .collect(),
        required: RequirementView::of(requirement),
        outstanding: outstanding.describe(),
    }
}

fn artifact_reference_audit(proposal: &vedaflow::StoredProposal) -> serde_json::Value {
    serde_json::to_value(&proposal.artifact_references)
        .unwrap_or_else(|_| serde_json::Value::Array(Vec::new()))
}

/// A proposal's members with their current content, a drift flag, and the
/// two sides of the diff a reviewer needs (FLOW-6, ADR-0035 decisions 5
/// and 6).
async fn member_views(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    proposal: &vedaflow::StoredProposal,
) -> Result<Vec<MemberView>> {
    let proposed = vedaflow::proposals::members(tx, tenant_id, proposal.commit).await?;
    if proposal.asset == AssetKind::Knowledge {
        return knowledge_member_views(tx, tenant_id, proposal, &proposed).await;
    }
    if proposal.asset == AssetKind::ContextPack {
        return document_member_views(tx, tenant_id, proposal, &proposed).await;
    }
    if proposal.asset == AssetKind::Skill {
        return skill_change_member_views(tx, tenant_id, proposal, &proposed).await;
    }
    if proposal.asset == AssetKind::Tool {
        return tool_change_member_views(tx, tenant_id, proposal, &proposed).await;
    }
    if proposal.asset == AssetKind::Configuration {
        return configuration_change_member_views(tx, tenant_id, proposal, &proposed).await;
    }
    if proposal.asset == AssetKind::Policy && proposal.effect == ProposalEffect::Apply {
        return relaxation_change_member_views(tx, tenant_id, proposal, &proposed).await;
    }
    if proposal.asset == AssetKind::Prompt {
        return prompt_member_views(tx, tenant_id, proposal, &proposed).await;
    }
    Err(Error::Invalid {
        message: format!(
            "proposal {} carries removed asset kind {}; the fresh context-platform epoch \
             does not admit raw-record proposals",
            proposal.id,
            proposal.asset.as_str()
        ),
    })
}

/// [`member_views`] for a governed Knowledge command (CPR-16, ADR-0081).
///
/// The VedaFlow object is deliberately content-free so authorised erasure can
/// remove plaintext without rewriting immutable governance history. The
/// erasable typed effect projection supplies the review text while it exists;
/// its canonical digest must still equal the digest in the reviewed manifest.
async fn knowledge_member_views(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    proposal: &vedaflow::StoredProposal,
    proposed: &[vedaflow::ChannelMember],
) -> Result<Vec<MemberView>> {
    let [member] = proposed else {
        return Err(Error::Internal {
            message: format!(
                "Knowledge change {} has {} members rather than one command",
                proposal.id,
                proposed.len()
            ),
        });
    };
    if member.name != "command" {
        return Err(Error::Internal {
            message: format!(
                "Knowledge change {} names member {:?}, expected command",
                proposal.id, member.name
            ),
        });
    }
    let change = synveda_store::knowledge_lifecycle::read_change(&mut *tx, tenant_id, proposal.id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!(
                "Knowledge change {} has no typed effect projection",
                proposal.id
            ),
        })?;
    let object = vedaflow::read_object(&mut *tx, tenant_id, member.object)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!(
                "Knowledge change {} names missing manifest object {}",
                proposal.id,
                member.object.to_hex()
            ),
        })?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&object.content).map_err(|err| Error::Internal {
            message: format!(
                "Knowledge change {} manifest is invalid: {err}",
                proposal.id
            ),
        })?;
    let manifest_hash = manifest
        .get("payload_hash")
        .and_then(serde_json::Value::as_str);
    let (content, payload_matches) = match change.payload {
        Some(command) => {
            let value = synveda_types::json::canonicalise(&serde_json::to_value(command).map_err(
                |err| Error::Internal {
                    message: format!("encode Knowledge change {} for review: {err}", proposal.id),
                },
            )?);
            let bytes = serde_json::to_vec(&value).map_err(|err| Error::Internal {
                message: format!(
                    "encode Knowledge change {} canonical bytes: {err}",
                    proposal.id
                ),
            })?;
            let hash = blake3::hash(&bytes).to_hex().to_string();
            let rendered = serde_json::to_string_pretty(&value).map_err(|err| Error::Internal {
                message: format!("render Knowledge change {}: {err}", proposal.id),
            })?;
            (rendered, hash == change.payload_hash)
        }
        None => (
            format!(
                "{{\n  \"payload\": \"erased\",\n  \"payload_hash\": \"{}\"\n}}",
                change.payload_hash
            ),
            true,
        ),
    };
    Ok(vec![MemberView {
        member: member.name.clone(),
        asset: AssetKind::Knowledge.as_str().to_owned(),
        object_hash: member.object.to_hex(),
        unchanged: payload_matches && manifest_hash == Some(change.payload_hash.as_str()),
        sensitivity: proposal.sensitivity,
        content: content.clone(),
        effect: MemberEffect::Apply,
        proposed: content,
        baseline: None,
    }])
}

/// [`member_views`] for an immutable, typed Skill command (CPR-23,
/// ADR-0085). Review binds the canonical command digest; bundle files remain
/// content-addressed objects referenced by the command rather than mutable
/// channel drafts.
async fn skill_change_member_views(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    proposal: &vedaflow::StoredProposal,
    proposed: &[vedaflow::ChannelMember],
) -> Result<Vec<MemberView>> {
    let [member] = proposed else {
        return Err(Error::Internal {
            message: format!(
                "Skill change {} has {} members rather than one command",
                proposal.id,
                proposed.len()
            ),
        });
    };
    if member.name != "command" {
        return Err(Error::Internal {
            message: format!(
                "Skill change {} names member {:?}, expected command",
                proposal.id, member.name
            ),
        });
    }
    let change = synveda_store::skills::change(&mut *tx, tenant_id, proposal.id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!("Skill change {} has no typed effect", proposal.id),
        })?;
    let object = vedaflow::read_object(&mut *tx, tenant_id, member.object)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!(
                "Skill change {} names missing manifest object {}",
                proposal.id,
                member.object.to_hex()
            ),
        })?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&object.content).map_err(|err| Error::Internal {
            message: format!("Skill change {} manifest is invalid: {err}", proposal.id),
        })?;
    let manifest_hash = manifest
        .get("payload_hash")
        .and_then(serde_json::Value::as_str);
    let value = synveda_types::json::canonicalise(&serde_json::to_value(&change.command).map_err(
        |err| Error::Internal {
            message: format!("encode Skill change {} for review: {err}", proposal.id),
        },
    )?);
    let rendered = serde_json::to_string_pretty(&value).map_err(|err| Error::Internal {
        message: format!("render Skill change {}: {err}", proposal.id),
    })?;
    let payload_hash = blake3::hash(value.to_string().as_bytes())
        .to_hex()
        .to_string();
    Ok(vec![MemberView {
        member: member.name.clone(),
        asset: AssetKind::Skill.as_str().to_owned(),
        object_hash: member.object.to_hex(),
        unchanged: payload_hash == change.payload_hash
            && manifest_hash == Some(change.payload_hash.as_str()),
        sensitivity: change.command.sensitivity(),
        content: rendered.clone(),
        effect: MemberEffect::Apply,
        proposed: rendered,
        baseline: None,
    }])
}

/// [`member_views`] for an immutable typed Tool/apply command (CPR-25,
/// ADR-0086). The command carries only credential-free descriptors, schemas,
/// hashes and exact binding intent; secret material cannot enter this review.
async fn tool_change_member_views(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    proposal: &vedaflow::StoredProposal,
    proposed: &[vedaflow::ChannelMember],
) -> Result<Vec<MemberView>> {
    let [member] = proposed else {
        return Err(Error::Internal {
            message: format!(
                "Tool change {} has {} members rather than one command",
                proposal.id,
                proposed.len()
            ),
        });
    };
    if member.name != "command" {
        return Err(Error::Internal {
            message: format!(
                "Tool change {} names member {:?}, expected command",
                proposal.id, member.name
            ),
        });
    }
    let change = synveda_store::tool_registry::change(&mut *tx, tenant_id, proposal.id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!("Tool change {} has no typed effect", proposal.id),
        })?;
    let object = vedaflow::read_object(&mut *tx, tenant_id, member.object)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!(
                "Tool change {} names missing manifest object {}",
                proposal.id,
                member.object.to_hex()
            ),
        })?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&object.content).map_err(|err| Error::Internal {
            message: format!("Tool change {} manifest is invalid: {err}", proposal.id),
        })?;
    let manifest_hash = manifest
        .get("payload_hash")
        .and_then(serde_json::Value::as_str);
    let value = synveda_types::json::canonicalise(&serde_json::to_value(&change.command).map_err(
        |err| Error::Internal {
            message: format!("encode Tool change {} for review: {err}", proposal.id),
        },
    )?);
    let rendered = serde_json::to_string_pretty(&value).map_err(|err| Error::Internal {
        message: format!("render Tool change {}: {err}", proposal.id),
    })?;
    let payload_hash = blake3::hash(value.to_string().as_bytes())
        .to_hex()
        .to_string();
    Ok(vec![MemberView {
        member: member.name.clone(),
        asset: AssetKind::Tool.as_str().to_owned(),
        object_hash: member.object.to_hex(),
        unchanged: payload_hash == change.payload_hash
            && manifest_hash == Some(change.payload_hash.as_str()),
        sensitivity: Sensitivity::Internal,
        content: rendered.clone(),
        effect: MemberEffect::Apply,
        proposed: rendered,
        baseline: None,
    }])
}

/// [`member_views`] for an immutable typed Configuration/apply command
/// (CPR-30, ADR-0089). Complete documents are reviewable here; they carry no
/// credentials, and their canonical hash is bound into the manifest.
async fn configuration_change_member_views(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    proposal: &vedaflow::StoredProposal,
    proposed: &[vedaflow::ChannelMember],
) -> Result<Vec<MemberView>> {
    let [member] = proposed else {
        return Err(Error::Internal {
            message: format!(
                "Configuration change {} has {} members rather than one command",
                proposal.id,
                proposed.len()
            ),
        });
    };
    if member.name != "command" {
        return Err(Error::Internal {
            message: format!(
                "Configuration change {} names member {:?}, expected command",
                proposal.id, member.name
            ),
        });
    }
    let change = synveda_store::configuration::change(&mut *tx, tenant_id, proposal.id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!("Configuration change {} has no typed effect", proposal.id),
        })?;
    let object = vedaflow::read_object(&mut *tx, tenant_id, member.object)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!(
                "Configuration change {} names missing manifest object {}",
                proposal.id,
                member.object.to_hex()
            ),
        })?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&object.content).map_err(|error| Error::Internal {
            message: format!(
                "Configuration change {} manifest is invalid: {error}",
                proposal.id
            ),
        })?;
    let manifest_hash = manifest
        .get("payload_hash")
        .and_then(serde_json::Value::as_str);
    let value = synveda_types::json::canonicalise(&serde_json::to_value(&change.command).map_err(
        |error| Error::Internal {
            message: format!(
                "encode Configuration change {} for review: {error}",
                proposal.id
            ),
        },
    )?);
    let rendered = serde_json::to_string_pretty(&value).map_err(|error| Error::Internal {
        message: format!("render Configuration change {}: {error}", proposal.id),
    })?;
    let payload_hash = blake3::hash(value.to_string().as_bytes())
        .to_hex()
        .to_string();
    Ok(vec![MemberView {
        member: member.name.clone(),
        asset: AssetKind::Configuration.as_str().to_owned(),
        object_hash: member.object.to_hex(),
        unchanged: payload_hash == change.payload_hash
            && manifest_hash == Some(change.payload_hash.as_str()),
        sensitivity: Sensitivity::Internal,
        content: rendered.clone(),
        effect: MemberEffect::Apply,
        proposed: rendered,
        baseline: None,
    }])
}

/// [`member_views`] for an immutable typed Policy/apply relaxation command
/// (CPR-31, ADR-0090). The complete bounded terms are reviewable here and
/// their canonical digest must match the content-addressed manifest.
async fn relaxation_change_member_views(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    proposal: &vedaflow::StoredProposal,
    proposed: &[vedaflow::ChannelMember],
) -> Result<Vec<MemberView>> {
    let [member] = proposed else {
        return Err(Error::Internal {
            message: format!(
                "relaxation change {} has {} members rather than one command",
                proposal.id,
                proposed.len()
            ),
        });
    };
    if member.name != "command" {
        return Err(Error::Internal {
            message: format!(
                "relaxation change {} names member {:?}, expected command",
                proposal.id, member.name
            ),
        });
    }
    let change = synveda_store::relaxations::change(&mut *tx, tenant_id, proposal.id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!("relaxation change {} has no typed effect", proposal.id),
        })?;
    let object = vedaflow::read_object(&mut *tx, tenant_id, member.object)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!(
                "relaxation change {} names missing manifest object {}",
                proposal.id,
                member.object.to_hex()
            ),
        })?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&object.content).map_err(|error| Error::Internal {
            message: format!(
                "relaxation change {} manifest is invalid: {error}",
                proposal.id
            ),
        })?;
    let manifest_hash = manifest
        .get("payload_hash")
        .and_then(serde_json::Value::as_str);
    let value = synveda_types::json::canonicalise(&serde_json::to_value(&change.command).map_err(
        |error| Error::Internal {
            message: format!(
                "encode relaxation change {} for review: {error}",
                proposal.id
            ),
        },
    )?);
    let rendered = serde_json::to_string_pretty(&value).map_err(|error| Error::Internal {
        message: format!("render relaxation change {}: {error}", proposal.id),
    })?;
    let payload_hash = blake3::hash(value.to_string().as_bytes())
        .to_hex()
        .to_string();
    Ok(vec![MemberView {
        member: member.name.clone(),
        asset: AssetKind::Policy.as_str().to_owned(),
        object_hash: member.object.to_hex(),
        unchanged: payload_hash == change.payload_hash
            && manifest_hash == Some(change.payload_hash.as_str()),
        sensitivity: proposal.sensitivity,
        content: rendered.clone(),
        effect: MemberEffect::Apply,
        proposed: rendered,
        baseline: None,
    }])
}

/// [`member_views`] for a context-pack proposal (PRMT-2, ADR-0050).
///
/// [`prompt_member_views`] one table over, with one difference that is
/// worth its own sentence: the diff a reviewer reads is the **document**,
/// not its chunks. A pack is reviewed as the prose somebody wrote, and the
/// chunk boundaries are a deterministic function of that prose — showing
/// them would be showing the reviewer an implementation detail of the read
/// path (ADR-0050 reversal trigger (c) is where that changes, and it is
/// ADR-0035's seam doing its job when it does).
async fn document_member_views(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    proposal: &vedaflow::StoredProposal,
    proposed: &[vedaflow::ChannelMember],
) -> Result<Vec<MemberView>> {
    let paths: Vec<DocumentPath> = proposed
        .iter()
        .map(|member| {
            member
                .name
                .parse::<DocumentPath>()
                .map_err(|err| Error::Internal {
                    message: format!(
                        "proposal member {:?} is not a document path: {err}",
                        member.name
                    ),
                })
        })
        .collect::<Result<_>>()?;
    let drafts: std::collections::HashMap<DocumentPath, synveda_store::packs::StoredDocument> =
        synveda_store::packs::list_all_documents(&mut *tx, tenant_id, proposal.source_scope_id)
            .await?
            .into_iter()
            .map(|document| {
                (
                    DocumentPath::new(document.pack_name.clone(), document.document_name.clone()),
                    document,
                )
            })
            .collect();
    let published = published_documents_at(tx, tenant_id, proposal.target_scope_id).await?;

    let mut wanted: Vec<vedaflow::hash::ObjectHash> =
        proposed.iter().map(|member| member.object).collect();
    wanted.extend(paths.iter().filter_map(|path| published.get(path).copied()));
    let objects = vedaflow::read_objects(tx, tenant_id, &wanted).await?;
    let text_at = |hash: &vedaflow::hash::ObjectHash| -> String {
        objects
            .get(hash)
            .and_then(|object| vedaflow::ContextPackAsset::from_bytes(&object.content).ok())
            .map(|asset| asset.document.content)
            .unwrap_or_default()
    };

    Ok(proposed
        .iter()
        .zip(&paths)
        .map(|(member, path)| {
            // The tier under review is the *proposed version's*, read from
            // the object the approvals bind rather than from a draft that
            // may have moved since.
            let reviewed = objects
                .get(&member.object)
                .and_then(|object| vedaflow::ContextPackAsset::from_bytes(&object.content).ok());
            let (unchanged, content) = match drafts.get(path) {
                Some(draft) => (
                    member.object.as_bytes() == &draft.object_hash,
                    text_at(&member.object),
                ),
                // No draft at the source: the member is one the source holds
                // by publishing it, so what it "stands at now" is that
                // publication — unchanged exactly while the source still
                // names these bytes, which is what publishing re-checks.
                None => (true, String::new()),
            };
            let (effect, baseline) = match published.get(path) {
                None => (MemberEffect::Add, None),
                Some(held) if *held == member.object => (MemberEffect::None, None),
                Some(held) => (
                    MemberEffect::Update,
                    Some(BaselineView {
                        object_hash: held.to_hex(),
                        text: text_at(held),
                    }),
                ),
            };
            MemberView {
                member: path.to_string(),
                asset: AssetKind::ContextPack.as_str().to_owned(),
                object_hash: member.object.to_hex(),
                unchanged,
                sensitivity: reviewed
                    .as_ref()
                    .map_or(proposal.sensitivity, |asset| asset.sensitivity),
                content,
                effect,
                proposed: text_at(&member.object),
                baseline,
            }
        })
        .collect())
}

/// [`member_views`] for a prompt proposal (PRMT-1, ADR-0049 decision 6).
///
/// The same three questions FLOW-6 asks of every governed member — what the
/// approvals bind, what publication would replace, and whether the source
/// has moved since — read one table over. The baseline is still the
/// **target's** tree, which for a climb is the ancestor's, and the "as it
/// stands now" side is the draft at the source scope, absent when the
/// member is one the source holds by publishing it.
async fn prompt_member_views(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    proposal: &vedaflow::StoredProposal,
    proposed: &[vedaflow::ChannelMember],
) -> Result<Vec<MemberView>> {
    let names: Vec<PromptName> = proposed
        .iter()
        .map(|member| {
            member
                .name
                .parse::<PromptName>()
                .map_err(|err| Error::Internal {
                    message: format!(
                        "proposal member {:?} is not a prompt name: {err}",
                        member.name
                    ),
                })
        })
        .collect::<Result<_>>()?;
    let drafts: std::collections::HashMap<PromptName, synveda_store::prompts::StoredPrompt> =
        synveda_store::prompts::read_many(&mut *tx, tenant_id, proposal.source_scope_id, &names)
            .await?
            .into_iter()
            .map(|draft| (draft.template.name.clone(), draft))
            .collect();
    let published = published_prompts_at(tx, tenant_id, proposal.target_scope_id).await?;

    let mut wanted: Vec<vedaflow::hash::ObjectHash> =
        proposed.iter().map(|member| member.object).collect();
    wanted.extend(names.iter().filter_map(|name| published.get(name).copied()));
    let objects = vedaflow::read_objects(tx, tenant_id, &wanted).await?;
    let text_at = |hash: &vedaflow::hash::ObjectHash| -> String {
        objects
            .get(hash)
            .map(|object| String::from_utf8_lossy(&object.content).into_owned())
            .unwrap_or_default()
    };

    Ok(proposed
        .iter()
        .zip(&names)
        .map(|(member, name)| {
            // The tier under review is the *proposed version's*, read from
            // the object the approvals bind rather than from a draft that
            // may have moved since.
            let reviewed = objects
                .get(&member.object)
                .and_then(|object| vedaflow::PromptAsset::from_bytes(&object.content).ok());
            let (unchanged, content) = match drafts.get(name) {
                Some(draft) => (
                    member.object.as_bytes() == &draft.object_hash,
                    draft.template.template.clone(),
                ),
                // No draft at the source: the member is one the source holds
                // by publishing it, so what it "stands at now" is that
                // publication — unchanged exactly while the source still
                // names these bytes, which is what publishing re-checks.
                None => (true, String::new()),
            };
            let (effect, baseline) = match published.get(name) {
                None => (MemberEffect::Add, None),
                Some(held) if *held == member.object => (MemberEffect::None, None),
                Some(held) => (
                    MemberEffect::Update,
                    Some(BaselineView {
                        object_hash: held.to_hex(),
                        text: text_at(held),
                    }),
                ),
            };
            MemberView {
                member: name.to_string(),
                asset: AssetKind::Prompt.as_str().to_owned(),
                object_hash: member.object.to_hex(),
                unchanged,
                sensitivity: reviewed
                    .as_ref()
                    .map_or(proposal.sensitivity, |asset| asset.sensitivity),
                content,
                effect,
                proposed: text_at(&member.object),
                baseline,
            }
        })
        .collect())
}

/// Decides the asset kind's own read action at `scope_id`, at the working
/// tier — the *whose-material* question every governance act asks
/// (ADR-0031 decision 12, ADR-0038 decision 10, ADR-0049 decision 4).
///
/// The twin of `channels::decide_asset_read`, spelled here because the two
/// routes are in different modules and neither is the other's helper; the
/// suites pin them to the same behaviour.
fn decide_asset_read(
    state: &AppState,
    input: &DecisionInput,
    asset: AssetKind,
    scope_id: ScopeId,
) -> Result<crate::authz::Authorized> {
    let resource = Resource::Scope(scope_id);
    match asset {
        AssetKind::Prompt => {
            authz::decide_prompt_read(state, input, resource, Sensitivity::WORKING)
        }
        AssetKind::ContextPack => {
            authz::decide_context_pack_read(state, input, resource, Sensitivity::WORKING)
        }
        // Typed governed artifacts are opened by their own command routes;
        // this generic authoring route carries only prompt/pack documents.
        other => Err(Error::Invalid {
            message: format!(
                "{} is not an authored artifact this proposal route carries",
                other.as_str()
            ),
        }),
    }
}

fn asset_read_action(asset: AssetKind) -> Result<Action> {
    match asset {
        AssetKind::Prompt => Ok(Action::PromptRead),
        AssetKind::ContextPack => Ok(Action::ContextPackRead),
        other => Err(Error::Invalid {
            message: format!(
                "{} is not an authored artifact a proposal carries",
                other.as_str()
            ),
        }),
    }
}

/// One member of a proposal-to-be, as the asset kind's own reader sees it.
///
/// The per-asset-kind seam ADR-0035 predicted, arriving on the write side
/// first: everything a proposal does with a member — price it, address it,
/// name it in a tree — is one of these three questions, and the two kinds
/// answer them from different tables.
enum Proposed {
    /// A prompt, as it will be addressed. Either the draft that lives at
    /// the source scope, or — for the second sense of "the source holds
    /// it" — the object the source's published tree already names.
    Prompt(vedaflow::PromptAsset),
    /// One **document** of a context pack, as it will be addressed
    /// (PRMT-2, ADR-0050 decision 3). The same two senses as a prompt, one
    /// table over — and a bundle is several of these rather than one
    /// member, because the channel names documents.
    ContextPack(vedaflow::ContextPackAsset),
}

impl Proposed {
    /// The tier the approval matrix prices this member at.
    fn sensitivity(&self) -> Sensitivity {
        match self {
            Proposed::Prompt(asset) => asset.sensitivity,
            // Per document, never per pack (ADR-0050 decision 12) — so a
            // bundle mixing a public glossary and a confidential runbook is
            // priced at the runbook, which is `max_sensitivity`'s existing
            // rule doing exactly what it was written for.
            Proposed::ContextPack(asset) => asset.sensitivity,
        }
    }
}

/// The prompts `scope` **holds**, refusing the whole request if any is not
/// held there — [`held_versions`]'s two senses, one table over.
///
/// - the draft **lives** there (`prompts.scope_id`), which is every
///   same-scope proposal and the first hop of any climb; or
/// - the scope **published** it — its `prompt/published` tree names the
///   name, and the object at that address is what climbs onward. There is
///   deliberately no draft row involved in that case: a department that
///   admitted a team's prompt holds the bytes through its own channel, not
///   through an authoring copy nobody edited.
async fn held_prompts(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    scope_id: ScopeId,
    names: &[PromptName],
) -> Result<Vec<vedaflow::PromptAsset>> {
    let mut requested: Vec<PromptName> = names.to_vec();
    requested.sort();
    requested.dedup();
    let drafts = synveda_store::prompts::read_many(&mut *tx, tenant_id, scope_id, &requested)
        .await?
        .into_iter()
        .map(|draft| {
            (
                draft.template.name.clone(),
                vedaflow::PromptAsset {
                    scope_id,
                    sensitivity: draft.sensitivity,
                    template: draft.template,
                },
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
    let published = published_prompts_at(tx, tenant_id, scope_id).await?;

    let mut held = Vec::with_capacity(requested.len());
    let mut missing: Vec<String> = Vec::new();
    for name in &requested {
        if let Some(asset) = drafts.get(name) {
            held.push(asset.clone());
            continue;
        }
        // No draft here: the source may still hold it through its own
        // published channel, which is how a second hop starts from where
        // the first one landed.
        let Some(address) = published.get(name).copied() else {
            missing.push(name.to_string());
            continue;
        };
        let object = vedaflow::read_object(&mut *tx, tenant_id, address)
            .await?
            .ok_or_else(|| Error::Internal {
                message: format!(
                    "published prompt {name} names object {} which the append-only \
                     store does not hold",
                    address.to_hex()
                ),
            })?;
        held.push(vedaflow::PromptAsset::from_bytes(&object.content)?);
    }
    if !missing.is_empty() {
        // Named rather than silently dropped: proposing a subset of what an
        // author asked for is the one outcome a review surface must never
        // produce quietly.
        return Err(Error::Invalid {
            message: format!(
                "scope {scope_id} neither drafts nor publishes: {} — name the scope \
                 that does with source_scope_id, which must be {scope_id} or a scope \
                 beneath it (FLOW-5 climbs the hierarchy, it does not cross it)",
                missing.join(", ")
            ),
        });
    }
    Ok(held)
}

/// What `scope`'s published prompt channel names, prompt name → address.
async fn published_prompts_at(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    scope_id: ScopeId,
) -> Result<std::collections::HashMap<PromptName, vedaflow::hash::ObjectHash>> {
    Ok(
        vedaflow::read_prompt_members(tx, tenant_id, &[scope_id], Channel::Published)
            .await?
            .into_iter()
            .next()
            .map(|state| state.members)
            .unwrap_or_default(),
    )
}

/// What `scope`'s published context-pack channel names, by document path.
async fn published_documents_at(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    scope_id: ScopeId,
) -> Result<std::collections::HashMap<DocumentPath, vedaflow::hash::ObjectHash>> {
    Ok(
        vedaflow::read_context_pack_members(tx, tenant_id, &[scope_id], Channel::Published)
            .await?
            .into_iter()
            .next()
            .map(|state| state.members)
            .unwrap_or_default(),
    )
}

/// The context-pack documents `scope` **holds**, refusing the whole
/// request if any is not held there — [`held_prompts`]'s two senses, one
/// table over.
///
/// - the document **lives** there (`context_pack_documents.scope_id`),
///   which is every same-scope proposal and the first hop of any climb; or
/// - the scope **published** it — its `context-pack/published` tree names
///   the path, and the object at that address is what climbs onward.
async fn held_documents(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    scope_id: ScopeId,
    paths: &[DocumentPath],
) -> Result<Vec<vedaflow::ContextPackAsset>> {
    let mut requested: Vec<DocumentPath> = paths.to_vec();
    requested.sort();
    requested.dedup();
    let drafts: std::collections::HashMap<DocumentPath, synveda_store::packs::StoredDocument> =
        synveda_store::packs::list_all_documents(&mut *tx, tenant_id, scope_id)
            .await?
            .into_iter()
            .map(|document| {
                (
                    DocumentPath::new(document.pack_name.clone(), document.document_name.clone()),
                    document,
                )
            })
            .collect();
    let published = published_documents_at(tx, tenant_id, scope_id).await?;

    let mut held = Vec::with_capacity(requested.len());
    let mut missing: Vec<String> = Vec::new();
    for path in &requested {
        // The draft's *object* rather than its row: the row holds a title
        // and a tier, and the asset a proposal binds is the bytes at the
        // address the draft names — which is the same object either sense
        // reaches.
        let address = match drafts.get(path) {
            Some(document) => vedaflow::hash::ObjectHash::from_bytes(document.object_hash),
            None => match published.get(path).copied() {
                Some(address) => address,
                None => {
                    missing.push(path.to_string());
                    continue;
                }
            },
        };
        let object = vedaflow::read_object(&mut *tx, tenant_id, address)
            .await?
            .ok_or_else(|| Error::Internal {
                message: format!(
                    "context pack document {path} names object {} which the append-only \
                     store does not hold",
                    address.to_hex()
                ),
            })?;
        held.push(vedaflow::ContextPackAsset::from_bytes(&object.content)?);
    }
    if !missing.is_empty() {
        return Err(Error::Invalid {
            message: format!(
                "scope {scope_id} neither drafts nor publishes: {} — name the scope \
                 that does with source_scope_id, which must be {scope_id} or a scope \
                 beneath it (FLOW-5 climbs the hierarchy, it does not cross it)",
                missing.join(", ")
            ),
        });
    }
    Ok(held)
}

/// A set is reviewed as a set and is governed by its most sensitive
/// element (ADR-0032 decision 3), whichever kind of asset it holds.
fn max_sensitivity(members: &[Proposed]) -> Sensitivity {
    members
        .iter()
        .map(Proposed::sensitivity)
        .max()
        .unwrap_or(Sensitivity::Public)
}

fn role_list(roles: &[synveda_types::access::RoleKey]) -> String {
    if roles.is_empty() {
        return "none".to_owned();
    }
    roles
        .iter()
        .map(|role| role.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn climb_level_bucket(levels: usize) -> &'static str {
    match levels {
        0 => "0",
        1 => "1",
        2 => "2",
        _ => "3_plus",
    }
}

fn validate_open(body: &OpenBody) -> Result<()> {
    let invalid = |message: String| Err(Error::Invalid { message });
    // One asset kind per proposal (ADR-0049 decision 6): the approval
    // matrix resolves from it, and a mixed set would have to be priced at
    // the maximum by a rule nobody wrote.
    let named =
        usize::from(!body.prompt_names.is_empty()) + usize::from(!body.document_paths.is_empty());
    match named {
        0 => {
            return invalid(
                "name at least one member: prompt_names for prompts or document_paths for \
                 context pack documents"
                    .to_owned(),
            );
        }
        1 => {}
        _ => {
            return invalid(
                "a proposal carries one asset kind: name prompt_names or document_paths, \
                 never both — the approval matrix resolves from the asset"
                    .to_owned(),
            );
        }
    }
    let members = body.prompt_names.len().max(body.document_paths.len());
    if members > vedaflow::MAX_PROPOSAL_MEMBERS {
        return invalid(format!(
            "a proposal may name at most {} members",
            vedaflow::MAX_PROPOSAL_MEMBERS
        ));
    }
    let chars = body.title.chars().count();
    if chars == 0 || chars > MAX_TITLE_CHARS {
        return invalid(format!("title must be 1..={MAX_TITLE_CHARS} characters"));
    }
    Ok(())
}

fn check_text(label: &str, value: Option<&str>) -> Result<()> {
    let Some(text) = value else { return Ok(()) };
    if text.is_empty() {
        return Err(Error::Invalid {
            message: format!("{label} must not be empty"),
        });
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(Error::Invalid {
            message: format!("{label} must be at most {MAX_TEXT_CHARS} characters"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::climb_level_bucket;

    #[test]
    fn proposal_climb_metric_uses_bounded_labels() {
        assert_eq!(climb_level_bucket(0), "0");
        assert_eq!(climb_level_bucket(1), "1");
        assert_eq!(climb_level_bucket(2), "2");
        assert_eq!(climb_level_bucket(3), "3_plus");
        assert_eq!(climb_level_bucket(usize::MAX), "3_plus");
    }
}
