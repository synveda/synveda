//! The VedaFlow proposal API (FLOW-3, ADR-0032): `/v1/proposals` behind
//! tenant resolution, uniform-404 ownership, and the PDP.
//!
//! A proposal is the governed request to move reviewed content onto a
//! scope's published channel. Its content is a commit — a tree naming
//! every member at the object address of exactly the version proposed —
//! and its workflow is a row. Approvals are append-only, each naming the
//! commit it approved and the effective roles its caster held at the
//! target when they cast it.
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
//! A proposal's target may be a strict **ancestor** of its source, which
//! is how tribal knowledge reaches the department and then the org. It is
//! not a second kind of proposal: same table, same matrix resolved at the
//! target and only there, same lifecycle, same audit actions. Two things
//! are added and nothing else. Opening a climb takes a second Cedar
//! decision — `MemoryRead` at the *source*, the proposer's warrant for
//! showing the material to the target's reviewers — and a climb's members
//! must be material the source scope holds, meaning records that live
//! there or records its published channel names at their current address.
//! The second sense is what lets a department propose onward what a team
//! climbed into it, with nothing stored to make the hop possible.
//!
//! # Publishing is a separate act
//!
//! The deciding approval does not publish. `POST /v1/proposals/{id}/publish`
//! takes `ChannelPublish` and `MemoryRead` at the target exactly as the
//! direct route does, and additionally requires the proposal open, the
//! requirement satisfied, and the bytes unchanged since the review.
//! Auto-publishing would have to run under system authority precisely
//! when a `compliance` reviewer casts the deciding vote — a role that
//! holds no publish grant in any pack — and that is a PDP bypass however
//! it is spelled (ADR-0032 decision 9).

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{Action, Resource};
use synveda_store::records::RecordState;
use synveda_store::{hierarchy, records, rls};
use synveda_types::{
    ApprovalRequirement, AssetKind, CastApproval, Channel, Error, HierarchyNode, IdentityId,
    PromotionEvidence, ProposalId, ProposalState, ProposalView, RecordId, Result, Role, ScopeId,
    Sensitivity, TenantId, Verdict,
};
use synveda_vedaflow::{self as vedaflow, MemoryAsset, PolicySnapshot, Signer};

use crate::app::AppState;
use crate::approvals::{self, RequirementView};
use crate::audit;
use crate::authz::{self, DecisionInput};
use crate::error::ApiError;
use crate::hierarchy::{body, commit, found, tenant_id};
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
    let outcome = match &result {
        Ok(_) => "ok",
        Err(
            Error::Unauthenticated { .. }
            | Error::PolicyDenied { .. }
            | Error::NotFound { .. }
            | Error::Invalid { .. }
            | Error::Conflict { .. }
            | Error::RateLimited { .. },
        ) => "rejected",
        Err(_) => "error",
    };
    metrics::counter!(PROPOSAL_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    match result {
        Ok(response) => response.into_response(),
        Err(error) => {
            audit::record_rejection(state, op, &error).await;
            ApiError(error).into_response()
        }
    }
}

// ── Views ──────────────────────────────────────────────────────────────

/// One proposal in a listing.
#[derive(Serialize)]
struct ProposalSummary {
    id: ProposalId,
    target_scope_id: ScopeId,
    source_scope_id: ScopeId,
    asset: String,
    channel: Channel,
    /// The five-state vocabulary tech plan §2.3 describes: the stored
    /// state, with `approved` rendered from `open` plus a satisfied
    /// requirement (ADR-0032 decision 11).
    state: ProposalView,
    sensitivity: Sensitivity,
    title: String,
    /// The commit holding exactly what is proposed.
    commit: String,
    proposer_id: IdentityId,
    proposer_subject: String,
    created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    closed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    close_reason: Option<String>,
    /// What the matrix asks for here, resolved now.
    required: RequirementView,
    /// What it still lacks, in one line a reviewer reads.
    outstanding: String,
    /// Why a rule opened this, when one did (FLOW-4, ADR-0033 decision
    /// 12): the counts, the actions counted, and the audit range they
    /// were folded from — so a reviewer can check the claim against the
    /// chain rather than trust it. Absent on a human's proposal.
    #[serde(skip_serializing_if = "Option::is_none")]
    promotion: Option<PromotionEvidence>,
}

/// One review act as the API renders it.
#[derive(Serialize)]
struct ApprovalView {
    approver_id: IdentityId,
    approver_subject: String,
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

/// One member of a proposal — the id and the address that was proposed,
/// plus the record's current content so a reviewer can review it.
#[derive(Serialize)]
struct MemberView {
    record_id: RecordId,
    /// The address the proposal named.
    object_hash: String,
    /// Whether the record still hashes to that address. `false` means the
    /// content moved after the proposal opened, and publishing will
    /// refuse (ADR-0032 decision 6).
    unchanged: bool,
    class: String,
    sensitivity: Sensitivity,
    content: String,
}

/// One proposal, in full.
#[derive(Serialize)]
struct ProposalDetail {
    #[serde(flatten)]
    summary: ProposalSummary,
    members: Vec<MemberView>,
    approvals: Vec<ApprovalView>,
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
    limit: Option<i64>,
}

#[derive(Serialize)]
struct ListResponse {
    proposals: Vec<ProposalSummary>,
}

/// `GET /v1/proposals` — proposals, newest first.
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
                authz::gather(&state, &mut tx, None).await?,
                Resource::Tenant(tenant_id),
            ),
            Some(scope_id) => {
                let node = found(
                    hierarchy::node(&mut *tx, scope_id).await?,
                    tenant_id,
                    scope_id,
                )?;
                (
                    authz::gather(&state, &mut tx, Some(&node)).await?,
                    Resource::Scope(scope_id),
                )
            }
        };
        let authorized = authz::decide(&state, &input, Action::ProposalRead, resource, None)?;
        let stored = vedaflow::proposals::list(
            &mut tx,
            tenant_id,
            vedaflow::ProposalFilter {
                target_scope: params.scope_id,
                state: params.state,
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
/// gate (ADR-0034 decision 1). Requiring the reviewer to hold
/// `MemoryRead` at the *source* instead would break the product twice
/// over: `compliance` holds no content read in any pack, so the invariant
/// floor's own role could never review a `restricted` climb, and nobody
/// but the owner reads a personal scope, so a user's own memory could
/// never climb to their team. The read that guards a climb is the
/// proposer's, taken once at open time and recorded under their name.
#[tracing::instrument(name = "proposals.get", skip_all)]
pub(crate) async fn get(State(state): State<AppState>, Path(id): Path<ProposalId>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let proposal = load(&mut tx, tenant_id, id).await?;
        let node = target_node(&mut tx, tenant_id, &proposal).await?;
        let input = authz::gather(&state, &mut tx, Some(&node)).await?;
        let authorized = authz::decide(
            &state,
            &input,
            Action::ProposalRead,
            Resource::Scope(node.id),
            None,
        )?;
        let summary = summarise(&state, &mut tx, tenant_id, &input, &proposal).await?;
        let members = member_views(&mut tx, tenant_id, &proposal).await?;
        let recorded = vedaflow::proposals::approvals(&mut tx, tenant_id, id).await?;
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
            summary,
        }))
    }
    .await;
    respond(&state, "get", result).await
}

// ── Open ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct OpenBody {
    /// The scope whose published channel would move. Requirements resolve
    /// here, and only here — "each level's approvers" is true because
    /// each level's proposal resolves at that level (ADR-0034
    /// decision 4).
    scope_id: ScopeId,
    /// Where the material is now. Absent means the target — the
    /// same-scope case, a climb of zero levels. Present, it must be the
    /// target or a **descendant** of it: a climb goes up the chain that
    /// composition walks down (ADR-0034 decision 2).
    #[serde(default)]
    source_scope_id: Option<ScopeId>,
    /// The records to propose. Must be material the source scope holds —
    /// records living there, or records its published channel names at
    /// their current address (ADR-0034 decision 3).
    record_ids: Vec<RecordId>,
    /// What this proposes, in one line. A reviewer reads it in a list.
    title: String,
}

#[derive(Serialize)]
struct OpenResponse {
    #[serde(flatten)]
    summary: ProposalSummary,
}

/// `POST /v1/proposals` — open a proposal against a scope's published
/// channel.
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
    let tenant_id = tenant_id()?;
    let source_scope_id = body.source_scope_id.unwrap_or(body.scope_id);
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let node = found(
        hierarchy::node(&mut *tx, body.scope_id).await?,
        tenant_id,
        body.scope_id,
    )?;
    let source = found(
        hierarchy::node(&mut *tx, source_scope_id).await?,
        tenant_id,
        source_scope_id,
    )?;
    // Gathered at the *source* — the deeper node, whose chain contains
    // the target's as a suffix — so two scopes are decided from one set of
    // pack assignments and role bindings (ADR-0034 decision 12).
    let input = authz::gather(state, &mut tx, Some(&source)).await?;
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
    // The disclosure decision (ADR-0034 decision 1): may this principal
    // read what it is about to show the target's reviewers. It is the
    // whole warrant for the climb, and it is asked once, here — the
    // privacy floor then makes "nobody climbs another principal's personal
    // material" true with no clause about personal scopes anywhere.
    let disclosed = authz::decide(
        state,
        &input,
        Action::MemoryRead,
        Resource::Scope(source_scope_id),
        None,
    )?;
    let authorized = authz::decide_from(
        state,
        &input,
        target_position,
        Action::ProposalOpen,
        Resource::Scope(body.scope_id),
        None,
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

    let versions = held_versions(&mut tx, tenant_id, source_scope_id, &body.record_ids).await?;
    // Objects first: each member's address, computed from the version
    // being proposed. This is what binds the review to bytes — approvals
    // name this commit, and publishing recomputes these addresses from the
    // records as they stand then (ADR-0032 decision 6).
    let mut members: Vec<(String, vedaflow::hash::ObjectHash)> = Vec::with_capacity(versions.len());
    for version in &versions {
        let asset = memory_asset(version.id, &version.state);
        let object = vedaflow::put_memory(&mut tx, tenant_id, &asset).await?;
        members.push((asset.entry_name(), object.hash));
    }
    let sensitivity = max_sensitivity(&versions);
    let snapshot = PolicySnapshot::new(
        authorized.decision.pack_name.clone(),
        authorized.decision.pack_version,
    );
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
            asset: AssetKind::Memory,
            channel: Channel::Published,
            members: &members,
            sensitivity,
            title: &body.title,
            proposer,
            proposer_subject: &input.principal.subject,
            committed_at: Utc::now(),
            policy_snapshot: &snapshot,
            // A human opened this one (FLOW-4, ADR-0033 decision 12).
            evidence: None,
        },
        &Signer::Unsigned,
    )
    .await?;

    if target_position > 0 {
        metrics::counter!(
            PROPOSAL_CLIMBS_TOTAL,
            "levels" => target_position.to_string(),
            "from" => source.kind.as_str(),
            "to" => node.kind.as_str(),
        )
        .increment(1);
        tracing::info!(
            proposal.id = %proposal.id,
            scope.source = %source_scope_id,
            scope.target = %body.scope_id,
            climb.levels = target_position,
            "proposal climbs {target_position} level(s) to {}", node.path
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
            asset: AssetKind::Memory,
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
            "asset": AssetKind::Memory.as_str(),
            "channel": Channel::Published.as_str(),
            "title": body.title,
            "sensitivity": sensitivity.as_str(),
            "commit": proposal.commit.to_hex(),
            // Where it came from, and — when that is not the target — the
            // second governed decision the climb took: the proposer's read
            // at the source, which is the disclosure this proposal makes
            // (ADR-0034 decisions 1 and 9).
            "source_scope_id": source_scope_id,
            "target_scope_id": body.scope_id,
            "climb": (source_scope_id != body.scope_id).then(|| json!({
                "levels": target_position,
                "source_read": audit::decision_context(Action::MemoryRead, &disclosed),
            })),
            // Ids and addresses, never content.
            "records": members.iter().map(|(name, hash)| json!({
                "record_id": name,
                "object_hash": hash.to_hex(),
            })).collect::<Vec<_>>(),
            "approvals": approvals::audit_context(&requirement, &outstanding),
        }),
    )
    .await?;
    commit(tx).await?;

    Ok(Json(OpenResponse {
        summary: render(&proposal, &requirement, &outstanding),
    }))
}

// ── Review ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct ReviewBody {
    /// What the reviewer wants to say. Optional on an approval; a
    /// rejection carries its reason in `reason` instead.
    #[serde(default)]
    comment: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct RejectBody {
    /// Why. Mandatory — a rejection an auditor cannot read the reason for
    /// is not a review, and FLOW-5 inherits this reason for its
    /// per-level denials.
    reason: String,
}

#[derive(Serialize)]
struct ReviewResponse {
    #[serde(flatten)]
    summary: ProposalSummary,
    /// What this act contributed: the roles it counted under.
    counted_roles: Vec<String>,
}

/// `POST /v1/proposals/{id}/approve` — cast an approval.
#[tracing::instrument(name = "proposals.approve", skip_all)]
pub(crate) async fn approve(
    State(state): State<AppState>,
    Path(id): Path<ProposalId>,
    payload: std::result::Result<Json<ReviewBody>, JsonRejection>,
) -> Response {
    let result = async {
        // An approval with no body at all is the common case: `comment`
        // is the only field and it is optional, so a bodiless POST is a
        // bare "I approve".
        let comment = match payload {
            Ok(Json(body)) => body.comment,
            Err(_) => None,
        };
        approve_inner(&state, id, comment.as_deref()).await
    }
    .await;
    respond(&state, "approve", result).await
}

async fn approve_inner(
    state: &AppState,
    id: ProposalId,
    comment: Option<&str>,
) -> Result<Json<ReviewResponse>> {
    check_text("comment", comment)?;
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let proposal = load(&mut tx, tenant_id, id).await?;
    let node = target_node(&mut tx, tenant_id, &proposal).await?;
    let input = authz::gather(state, &mut tx, Some(&node)).await?;
    let authorized = authz::decide(
        state,
        &input,
        Action::ProposalReview,
        Resource::Scope(node.id),
        None,
    )?;
    require_open(&proposal)?;
    let approver = identity_of(&input)?;

    let requirement = requirement_for(state, &mut tx, tenant_id, &input, &node, &proposal).await?;
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
            "roles": candidate.roles.iter().map(|role| role.as_str()).collect::<Vec<_>>(),
            "comment": comment,
            "approvals": approvals::audit_context(&requirement, &after),
        }),
    )
    .await?;
    commit(tx).await?;

    Ok(Json(ReviewResponse {
        summary: render(&proposal, &requirement, &after),
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
#[tracing::instrument(name = "proposals.reject", skip_all)]
pub(crate) async fn reject(
    State(state): State<AppState>,
    Path(id): Path<ProposalId>,
    payload: std::result::Result<Json<RejectBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        reject_inner(&state, id, &body.reason).await
    }
    .await;
    respond(&state, "reject", result).await
}

async fn reject_inner(
    state: &AppState,
    id: ProposalId,
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
    let input = authz::gather(state, &mut tx, Some(&node)).await?;
    let authorized = authz::decide(
        state,
        &input,
        Action::ProposalReview,
        Resource::Scope(node.id),
        None,
    )?;
    require_open(&proposal)?;
    let reviewer = identity_of(&input)?;
    let roles = approvals::roles_at(&input, &node);

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

    let requirement = requirement_for(state, &mut tx, tenant_id, &input, &node, &proposal).await?;
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
    Ok(Json(render(&closed, &requirement, &outstanding)))
}

/// `POST /v1/proposals/{id}/withdraw` — the proposer closes their own.
///
/// Authorized by `ProposalOpen` at the target *and* by being the
/// proposer: withdrawing is the proposer's act, and a reviewer who wants
/// it gone rejects it with a reason instead.
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
        let input = authz::gather(&state, &mut tx, Some(&node)).await?;
        let authorized = authz::decide(
            &state,
            &input,
            Action::ProposalOpen,
            Resource::Scope(node.id),
            None,
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
            }),
        )
        .await?;
        commit(tx).await?;
        let mut closed = proposal;
        closed.state = ProposalState::Withdrawn;
        let outstanding = requirement.outstanding(&[]);
        Ok(Json(render(&closed, &requirement, &outstanding)))
    }
    .await;
    respond(&state, "withdraw", result).await
}

// ── Publish: the proposal's effect ─────────────────────────────────────

#[derive(Serialize)]
struct PublishResponse {
    proposal_id: ProposalId,
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
    let input = authz::gather(state, &mut tx, Some(&node)).await?;
    // The same two decisions the direct route takes (ADR-0031
    // decision 12): may this principal publish here, and may it read what
    // it is about to declare reviewed. The approvals go *in front of*
    // these; they do not replace them.
    let authorized = authz::decide(
        state,
        &input,
        Action::ChannelPublish,
        Resource::Scope(node.id),
        None,
    )?;
    authz::decide(
        state,
        &input,
        Action::MemoryRead,
        Resource::Scope(node.id),
        None,
    )?;
    require_open(&proposal)?;
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

    // Approvals bind bytes. Recompute every member's address from the
    // record as it stands *now* and require it to equal what the approved
    // commit named — otherwise the content moved after the review, and
    // publishing it would launder unreviewed text through a completed
    // approval (ADR-0032 decision 6). Then re-ask whether the source still
    // holds the material, which is the same check one scope over
    // (ADR-0034 decision 7).
    let proposed = vedaflow::proposals::members(&mut tx, tenant_id, proposal.commit).await?;
    let ids = member_ids(&proposed)?;
    let versions = records::current_many(&mut *tx, tenant_id, &ids).await?;
    let published = published_at(&mut tx, tenant_id, proposal.source_scope_id).await?;
    // Every refusal here is a `Conflict`, never an `Invalid`: the request
    // is well formed and was well formed when it was approved — what moved
    // is the world, between the review and its effect.
    let moved = |what: &str, record: RecordId| Error::Conflict {
        message: format!(
            "record {record} {what} after this proposal was approved; withdraw it and \
             open a new one so the change is reviewed"
        ),
    };
    let mut members: Vec<(String, vedaflow::hash::ObjectHash)> = Vec::with_capacity(ids.len());
    for member in &proposed {
        let record: RecordId = member.name.parse().map_err(|err| Error::Internal {
            message: format!(
                "proposal member {:?} is not a record id: {err}",
                member.name
            ),
        })?;
        let Some(version) = versions.iter().find(|version| version.id == record) else {
            return Err(moved("no longer exists", record));
        };
        let asset = memory_asset(version.id, &version.state);
        let address = asset.address();
        if address != member.object {
            return Err(moved("changed", record));
        }
        // And the source must still hold it. A record rewound off the
        // source's channel (FLOW-7) or moved out of the scope between
        // approval and publication is refused rather than carried up on a
        // review of material the source no longer stands behind
        // (ADR-0034 decision 7).
        if version.state.scope_id != proposal.source_scope_id
            && published.get(&record) != Some(&address)
        {
            return Err(Error::Conflict {
                message: format!(
                    "scope {} no longer holds record {record}; the climb was approved \
                     against material its source has since given up",
                    proposal.source_scope_id
                ),
            });
        }
        // Content-addressed: the object is already stored from the open,
        // so this re-write dedups and stores nothing.
        vedaflow::put_memory(&mut tx, tenant_id, &asset).await?;
        members.push((asset.entry_name(), address));
    }

    let channel = vedaflow::ChannelRef::memory(Channel::Published);
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
            // The publication is a merge: head first (the mainline), then
            // the proposal it is the effect of. Lineage becomes a fact
            // about the commit graph rather than a join between tables.
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

    // The same action a direct publish emits, with the proposal named:
    // it is the same governed act with the same consequence, and a second
    // action asserting it would be a fact an auditor has to reconcile
    // (ADR-0019 decision 4; ADR-0032 decision 18).
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::ChannelPublished,
        Resource::Scope(proposal.target_scope_id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::ChannelPublish, &authorized),
            "channel": channel.name(),
            "asset": channel.asset.as_str(),
            "message": proposal.title,
            "proposal_id": id,
            "proposal_commit": proposal.commit.to_hex(),
            "sensitivity": proposal.sensitivity.as_str(),
            // What climbed, and from where. The scope pair on the
            // publication event is what lets an auditor read a climb off
            // the chain without joining the proposal row (ADR-0034
            // decision 9).
            "source_scope_id": proposal.source_scope_id,
            "target_scope_id": proposal.target_scope_id,
            "records": members.iter().map(|(name, hash)| json!({
                "record_id": name,
                "object_hash": hash.to_hex(),
            })).collect::<Vec<_>>(),
            "commit": committed.commit.to_hex(),
            "parent": committed.parent.map(|parent| parent.to_hex()),
            "members": committed.entries,
            "added": committed.added,
            "approvals": approvals::audit_context(&requirement, &outstanding),
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
) -> Result<HierarchyNode> {
    found(
        hierarchy::node(&mut *tx, proposal.target_scope_id).await?,
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
    node: &HierarchyNode,
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
    let node = match hierarchy::node(&mut *tx, proposal.target_scope_id).await? {
        Some(node) => node,
        // The target vanished (TEN-5's disposal window): render what the
        // pack asks for at the tenant default rather than dropping the row.
        None => {
            let requirement = ApprovalRequirement::default();
            return Ok(render(
                proposal,
                &requirement,
                &requirement.outstanding(&[]),
            ));
        }
    };
    let requirement = requirement_for(state, tx, tenant_id, input, &node, proposal).await?;
    let recorded = vedaflow::proposals::approvals(tx, tenant_id, proposal.id).await?;
    let cast = vedaflow::proposals::cast_for(&recorded, proposal.commit);
    let outstanding = requirement.outstanding(&cast);
    Ok(render(proposal, &requirement, &outstanding))
}

fn render(
    proposal: &vedaflow::StoredProposal,
    requirement: &ApprovalRequirement,
    outstanding: &synveda_types::Outstanding,
) -> ProposalSummary {
    ProposalSummary {
        id: proposal.id,
        target_scope_id: proposal.target_scope_id,
        source_scope_id: proposal.source_scope_id,
        asset: proposal.asset.as_str().to_owned(),
        channel: proposal.channel,
        state: ProposalView::of(proposal.state, outstanding.is_empty()),
        sensitivity: proposal.sensitivity,
        title: proposal.title.clone(),
        commit: proposal.commit.to_hex(),
        proposer_id: proposal.proposer_id,
        proposer_subject: proposal.proposer_subject.clone(),
        created_at: proposal.created_at,
        closed_at: proposal.closed_at,
        close_reason: proposal.close_reason.clone(),
        required: RequirementView::of(requirement),
        outstanding: outstanding.describe(),
        promotion: proposal.evidence.clone(),
    }
}

/// A proposal's members with their current content and a drift flag.
async fn member_views(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    proposal: &vedaflow::StoredProposal,
) -> Result<Vec<MemberView>> {
    let proposed = vedaflow::proposals::members(tx, tenant_id, proposal.commit).await?;
    let ids = member_ids(&proposed)?;
    // Read wherever the records live, not at the target: a climb's members
    // live below it, and rendering them as "changed, no content" would
    // make every climb unreviewable (ADR-0034 decision 3). The address
    // comparison below is what says whether they still match the review.
    let versions = records::current_many(&mut *tx, tenant_id, &ids).await?;
    Ok(proposed
        .into_iter()
        .zip(ids)
        .map(|(member, record_id)| {
            let current = versions.iter().find(|version| version.id == record_id);
            let (unchanged, class, sensitivity, content) = match current {
                Some(version) => {
                    let asset = memory_asset(version.id, &version.state);
                    (
                        asset.address() == member.object,
                        version.state.class.as_str().to_owned(),
                        version.state.sensitivity,
                        version.state.content.clone(),
                    )
                }
                // The record was deleted or re-scoped under the proposal.
                // Rendered as changed, with no content to show — which is
                // exactly what publishing will refuse on.
                None => (false, String::new(), proposal.sensitivity, String::new()),
            };
            MemberView {
                record_id,
                object_hash: member.object.to_hex(),
                unchanged,
                class,
                sensitivity,
                content,
            }
        })
        .collect())
}

/// A proposal commit's entry names as record ids, in tree order.
///
/// Only this crate writes memory-asset entries and it names them by id,
/// so an unparseable name means schema and code have drifted — a bug to
/// name, not a member to drop silently from a review.
fn member_ids(members: &[vedaflow::ChannelMember]) -> Result<Vec<RecordId>> {
    members
        .iter()
        .map(|member| {
            member
                .name
                .parse::<RecordId>()
                .map_err(|err| Error::Internal {
                    message: format!(
                        "proposal member {:?} is not a record id: {err}",
                        member.name
                    ),
                })
        })
        .collect()
}

/// The current versions of `ids` that `scope` **holds**, refusing the
/// whole request if any is not held there.
///
/// A scope holds material in one of two senses, and FLOW-5 needs both
/// (ADR-0034 decision 3):
///
/// - the record **lives** there (`records.scope_id`), which is every
///   same-scope proposal and the first hop of any climb; or
/// - the scope **published** it — its `memory/published` tree names the
///   record at exactly the address its current content produces — which
///   is how a second hop starts from where the first one landed, with
///   nothing new stored to make it possible.
///
/// The address is what makes the second sense safe: an edited record
/// falls out of it by arithmetic, so a climb can never carry content the
/// source scope did not stand behind (ADR-0031 decision 5).
///
/// Named rather than silently dropped: proposing (or publishing) a subset
/// of what a curator asked for is the one outcome a review surface must
/// never produce quietly.
async fn held_versions(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    scope_id: ScopeId,
    ids: &[RecordId],
) -> Result<Vec<synveda_store::records::RecordVersion>> {
    let mut requested = ids.to_vec();
    requested.sort_unstable();
    requested.dedup();
    // Scope-blind: where each record lives is one of the two answers, not
    // the predicate.
    let versions = records::current_many(&mut *tx, tenant_id, &requested).await?;
    let published = published_at(tx, tenant_id, scope_id).await?;
    let held: Vec<synveda_store::records::RecordVersion> = versions
        .into_iter()
        .filter(|version| {
            version.state.scope_id == scope_id
                || published.get(&version.id)
                    == Some(&memory_asset(version.id, &version.state).address())
        })
        .collect();
    if held.len() != requested.len() {
        let found: Vec<RecordId> = held.iter().map(|version| version.id).collect();
        let missing: Vec<String> = requested
            .iter()
            .filter(|id| !found.contains(id))
            .map(ToString::to_string)
            .collect();
        return Err(Error::Invalid {
            message: format!(
                "scope {scope_id} neither holds nor publishes: {} — name the scope \
                 that does with source_scope_id, which must be {scope_id} or a scope \
                 beneath it (FLOW-5 climbs the hierarchy, it does not cross it)",
                missing.join(", ")
            ),
        });
    }
    Ok(held)
}

/// What `scope`'s published channel names, record id → admitted address.
async fn published_at(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    scope_id: ScopeId,
) -> Result<std::collections::HashMap<RecordId, vedaflow::hash::ObjectHash>> {
    Ok(
        vedaflow::read_memory_members(tx, tenant_id, &[scope_id], Channel::Published)
            .await?
            .into_iter()
            .next()
            .map(|channel| channel.members)
            .unwrap_or_default(),
    )
}

fn max_sensitivity(versions: &[synveda_store::records::RecordVersion]) -> Sensitivity {
    versions
        .iter()
        .map(|version| version.state.sensitivity)
        .max()
        .unwrap_or(Sensitivity::Public)
}

fn role_list(roles: &[Role]) -> String {
    if roles.is_empty() {
        return "none".to_owned();
    }
    roles
        .iter()
        .map(|role| role.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn validate_open(body: &OpenBody) -> Result<()> {
    let invalid = |message: String| Err(Error::Invalid { message });
    if body.record_ids.is_empty() {
        return invalid("record_ids must name at least one record".to_owned());
    }
    if body.record_ids.len() > vedaflow::MAX_PROPOSAL_MEMBERS {
        return invalid(format!(
            "record_ids must name at most {} records",
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

/// The VedaFlow view of a stored record version (ADR-0031 decision 6).
/// The same field copy the pipeline, the composition engine, and the
/// channel route make, for the reason recorded there: `synveda-store` and
/// `synveda-vedaflow` are siblings, so neither can host a conversion
/// between their types.
fn memory_asset(id: RecordId, state: &RecordState) -> MemoryAsset {
    MemoryAsset {
        id,
        scope_id: state.scope_id,
        owner_id: state.owner_id,
        kind: state.kind,
        class: state.class,
        content: state.content.clone(),
        sensitivity: state.sensitivity,
        valid_from: state.valid_from,
        valid_to: state.valid_to,
    }
}
