//! The VedaFlow proposal API (FLOW-3, ADR-0032): `/v1/proposals` behind
//! tenant resolution, uniform-404 ownership, and the PDP.
//!
//! A proposal is the governed request to run one reviewed effect. Most move
//! content onto a scope's published channel; Knowledge proposals apply a
//! typed aggregate command instead (CPR-16, ADR-0081). In both cases its
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

use std::collections::BTreeMap;

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
use synveda_store::{records, rls, scopes};
use synveda_types::scope::Scope;
use synveda_types::{
    ApprovalRequirement, AssetKind, CastApproval, Channel, Checklist, ChecklistItem,
    ChecklistVerdict, DocumentPath, Error, IdentityId, PromotionEvidence, PromptName,
    ProposalEffect, ProposalId, ProposalState, ProposalView, QualityShortfall, RecordId, Result,
    ScopeId, Sensitivity, SkillFile, SkillFilePath, SkillName, SkillPath, SkillQualityConfig,
    SkillScanConfig, TenantId, Verdict,
};
use synveda_vedaflow::{self as vedaflow, MemoryAsset, PolicySnapshot, Signer};

use crate::app::AppState;
use crate::approvals::{self, RequirementView};
use crate::audit;
use crate::authz::{self, DecisionInput};
use crate::error::ApiError;
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
    /// What running this proposal would do (AUTHZ-4, ADR-0037
    /// decision 16). `published` for every FLOW-3 proposal; `lapse` for a
    /// grant. Named for the effect rather than the channel because a lapse
    /// has no channel, and a field that said `published` on a proposal that
    /// publishes nothing would be the paper-over this feature refused at
    /// the schema.
    effect: ProposalEffect,
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

/// What publishing this proposal would do to the target's published
/// channel, for one member (FLOW-6, ADR-0035 decision 5). Membership in
/// the target's tree is the predicate — the same sense of "this scope
/// holds it" ADR-0034 decision 3 used one scope over.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
enum MemberEffect {
    /// The channel names no version of this record; publication admits it.
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
/// decision 8): a reviewer holding no `MemoryRead` sees what a
/// publication would overwrite. Bounded by the proposal's own member set,
/// the target's own channel, and the target scope the reviewer already
/// holds `ProposalRead` on — and admitted because a review of a change
/// that hides one side of the change is not a review.
#[derive(Serialize)]
struct BaselineView {
    /// The address the target's tree names for this record today.
    object_hash: String,
    /// That object's canonical bytes as text (ADR-0030 decision 4's
    /// human-readable form, which FLOW-1 chose for exactly this).
    text: String,
}

/// One member of a proposal — the id and the address that was proposed,
/// plus what a reviewer needs to review it: the bytes under review, the
/// bytes they would replace, and the record's current content.
#[derive(Serialize)]
struct MemberView {
    /// The tree entry name: a record id for a memory, a path for a prompt
    /// (PRMT-1, ADR-0049 decision 3). The one field both asset kinds carry,
    /// and the one a review surface displays.
    member: String,
    /// The record id, for a memory proposal. Absent for an authored asset,
    /// whose members are named rather than identified.
    #[serde(skip_serializing_if = "Option::is_none")]
    record_id: Option<RecordId>,
    /// What kind of asset this proposal carries — one word, so a reviewer's
    /// first line says what they are looking at.
    asset: String,
    /// The address the proposal named.
    object_hash: String,
    /// Whether the member still hashes to that address. `false` means the
    /// content moved after the proposal opened, and publishing will
    /// refuse (ADR-0032 decision 6).
    unchanged: bool,
    /// A memory's class. Absent for a prompt, which has none — `asset`
    /// says what it is instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    class: Option<String>,
    sensitivity: Sensitivity,
    /// The member's text **as it stands now**: a record's content, or a
    /// prompt draft's template. Beside `unchanged` this is what makes drift
    /// legible; it is not what the approvals bind.
    content: String,
    /// What publication would do to the target's channel for this member.
    effect: MemberEffect,
    /// The canonical bytes at the proposed address — what the approvals
    /// bind, read from the object store rather than re-derived from the
    /// record, because an edited record is no longer what anyone approved
    /// (ADR-0035 decision 6). Empty only if the object is missing, which
    /// the append-only store makes impossible.
    proposed: String,
    /// The version being replaced, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    baseline: Option<BaselineView>,
}

/// One proposal, in full.
#[derive(Serialize)]
struct ProposalDetail {
    #[serde(flatten)]
    summary: ProposalSummary,
    members: Vec<MemberView>,
    approvals: Vec<ApprovalView>,
    /// The security scan of the bytes this proposal would publish
    /// (SKIL-2, ADR-0052 decision 7).
    ///
    /// Present for `skill` proposals and absent for every other asset
    /// kind, because it is a statement about executable content and a
    /// field that was always there would be a field that means nothing
    /// three times out of four.
    ///
    /// **Recomputed on this read, not stored** (decision 6): it is a pure
    /// function of the members' bytes and the rule table, and both are
    /// already here — the objects were read to draw the diff, and the
    /// table is compiled in. What is durable is the audit record, because
    /// a row is mutable and a chained event is not.
    #[serde(skip_serializing_if = "Option::is_none")]
    scan: Option<crate::skills::ScanReport>,
    /// The quality of the bytes this proposal would publish (SKIL-3,
    /// ADR-0053 decision 11).
    ///
    /// Present for `skill` proposals and absent for every other kind, on
    /// `scan`'s reasoning. **Two halves, never averaged**: the automated
    /// rubric is recomputed here from the same member bytes the diff
    /// renders, and the reviewer checklist is looked up by a digest of
    /// exactly those members — so a checklist answered against an earlier
    /// draft is simply not found, rather than laundering answers about
    /// content nobody reviewed (decision 4).
    ///
    /// It is rendered against the pack that will decide the *publication*,
    /// so `needs_override` means "this will need somebody holding
    /// `SkillQualityOverride`" rather than "some pack somewhere would ask
    /// for one".
    #[serde(skip_serializing_if = "Option::is_none")]
    quality: Option<crate::skills::QualityReport>,
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
        let members_of = vedaflow::proposals::members(&mut tx, tenant_id, proposal.commit).await?;
        // The report a reviewer reads, over the same bytes the diff above
        // renders — recomputed here rather than looked up (ADR-0052
        // decision 6), and reported against the pack that would decide the
        // publication, so `blocked` means "this will be refused at publish"
        // rather than "some pack somewhere would refuse it".
        let effective = state
            .pdp
            .effective(tenant_id, Resource::Scope(node.id), &input.context());
        let (scan, quality) = if proposal.asset == AssetKind::Skill {
            let config = effective.scan;
            // Labelled by the member name — `<skill>/<file>` — rather than
            // the bare filename, because a proposal may carry more than one
            // bundle and "SKILL.md" alone would not say whose.
            let files: Vec<SkillFile> = members
                .iter()
                .filter_map(|member| {
                    member
                        .member
                        .parse::<SkillFilePath>()
                        .ok()
                        .map(|path| SkillFile {
                            path,
                            content: member.proposed.clone(),
                        })
                })
                .collect();
            let scanned = crate::skills::scan_security(&files).await?;
            // The rubric over the same bytes, and the checklist bound to
            // exactly these members. The digest is computed from
            // `(member name, object address)` pairs, which is what a
            // publication will recompute — so what a reviewer is shown
            // here and what the gate reads there are the same lookup.
            // **From the objects, not from the member names.** A member
            // is named `<skill>/<path>`, so reconstructing a `SkillFile`
            // from that name gives a path of `code-review/SKILL.md` —
            // and the rubric, which has to know which file is the
            // manifest, then sees a bundle with no manifest at all and
            // scores it as garbage. That is harmless for the scanner
            // above, which reads every file's content regardless of its
            // name, and fatal here.
            //
            // The acceptance test caught it by asserting that the review
            // and the registry agree about the same bytes, which is the
            // property a recomputed score has to have to be worth
            // rendering beside a cached one.
            let bundle = crate::skills::files_of_members(&mut tx, tenant_id, &members_of).await?;
            let scored = crate::skills::score_quality(&bundle).await?;
            let digest = crate::skills::digest_of_members(
                &members
                    .iter()
                    .map(|member| (member.member.clone(), member.object_hash.clone()))
                    .collect::<Vec<_>>(),
            )?;
            let review = match crate::skills::skill_of(
                &members.iter().map(|m| m.member.clone()).collect::<Vec<_>>(),
            ) {
                Some(name) => {
                    synveda_store::skill_reviews::for_bundle(
                        &mut *tx,
                        tenant_id,
                        proposal.source_scope_id,
                        &name,
                        &digest,
                    )
                    .await?
                }
                None => None,
            };
            (
                Some(crate::skills::ScanReport::new(&scanned, &config)),
                Some(crate::skills::QualityReport::new(
                    &scored,
                    &effective.quality,
                    &digest,
                    review.as_ref(),
                )),
            )
        } else {
            (None, None)
        };
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
            scan,
            quality,
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
    #[serde(default)]
    record_ids: Vec<RecordId>,
    /// The prompts to propose, by name (PRMT-1, ADR-0049 decision 6).
    ///
    /// Exactly one of `record_ids` and `prompt_names` may be present: a
    /// proposal has one asset kind, because the approval matrix resolves
    /// from it and `regulated-strict` prices a prompt at two distinct
    /// people where it prices a team's memory at one. A mixed set would
    /// have to be priced at the maximum, which is a rule nobody wrote and
    /// a review nobody asked for.
    ///
    /// The same two senses of "the source holds it" apply: the draft lives
    /// there, or the source's published channel names it at that address —
    /// which is what lets a department propose onward what a team climbed
    /// into it, with no draft row at the department at all.
    #[serde(default)]
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
    document_paths: Vec<DocumentPath>,
    /// The skills to promote, by **name** (SKIL-1, ADR-0051 decision 1).
    /// Never by file: a client loads a bundle whole, so proposing three of
    /// its four files would propose a version nobody can run. Every file
    /// the source holds becomes a member.
    ///
    /// Exactly one of the four lists may be present, for `prompt_names`'
    /// reason.
    #[serde(default)]
    skill_names: Vec<SkillName>,
    /// What this proposes, in one line. A reviewer reads it in a list.
    title: String,
    /// What running this proposal would *do*. Absent means `published` —
    /// the effect a proposal had before AUTHZ-4 and the one it has almost
    /// always. `lapse` is refused here: a lapse's terms are a different
    /// body and have their own route (ADR-0037).
    #[serde(default)]
    effect: Option<ProposalEffect>,
    /// The tier a `classify` proposal would install (AUTHZ-5, ADR-0038
    /// decision 9). Required for that effect and refused for any other:
    /// a publication does not move a tier, and a body that named one would
    /// be describing something the effect will not do.
    #[serde(default)]
    sensitivity: Option<Sensitivity>,
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
    let effect = body.effect.unwrap_or(ProposalEffect::Published);
    // Present exactly for the effect that installs one, absent for every
    // other: `validate_open` refuses the two mismatches by name.
    let proposed_tier = body.sensitivity.unwrap_or(Sensitivity::WORKING);
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

    let asset = if !body.prompt_names.is_empty() {
        AssetKind::Prompt
    } else if !body.document_paths.is_empty() {
        AssetKind::ContextPack
    } else if !body.skill_names.is_empty() {
        AssetKind::Skill
    } else {
        AssetKind::Memory
    };
    // The members, as the asset kind's own reader sees them: the two senses
    // of "the source holds it" (ADR-0034 decision 3) are the same two for
    // both kinds — it lives there, or the source's published channel names
    // it at its current address — read from different tables.
    let members_now: Vec<Proposed> = match asset {
        AssetKind::Memory => held_versions(&mut tx, tenant_id, source_scope_id, &body.record_ids)
            .await?
            .into_iter()
            .map(Proposed::Memory)
            .collect(),
        AssetKind::ContextPack => {
            held_documents(&mut tx, tenant_id, source_scope_id, &body.document_paths)
                .await?
                .into_iter()
                .map(Proposed::ContextPack)
                .collect()
        }
        AssetKind::Skill => held_skills(&mut tx, tenant_id, source_scope_id, &body.skill_names)
            .await?
            .into_iter()
            .map(Proposed::Skill)
            .collect(),
        _ => held_prompts(&mut tx, tenant_id, source_scope_id, &body.prompt_names)
            .await?
            .into_iter()
            .map(Proposed::Prompt)
            .collect(),
    };
    let held = max_sensitivity(&members_now);
    // A classification's requirement resolves at the **maximum of the
    // current and proposed tiers** (ADR-0038 decision 9). Taking only the
    // proposed side would price a declassification at the tier it is
    // leaving *for* — so removing `restricted` would cost what `internal`
    // costs, and the one direction that actually removes a control would be
    // the cheap one.
    let sensitivity = match effect {
        ProposalEffect::Classify => held.max(proposed_tier),
        _ => held,
    };
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
    // deciding it as a memory read would ask a question about a different
    // corpus — in a pack that shares prompts more widely than memory, or
    // less, the two answers differ.
    let disclosed = decide_asset_read(state, &input, asset, source_scope_id)?;
    // Objects first: each member's address, computed from the version
    // being proposed. This is what binds the review to bytes — approvals
    // name this commit, and publishing recomputes these addresses from the
    // records as they stand then (ADR-0032 decision 6).
    let mut members: Vec<(String, vedaflow::hash::ObjectHash)> =
        Vec::with_capacity(members_now.len());
    for proposed in &members_now {
        let (entry, hash) = match proposed {
            Proposed::Memory(version) => {
                let mut asset = memory_asset(version.id, &version.state);
                if effect == ProposalEffect::Classify {
                    // The proposed version differs from the live record in
                    // exactly one field, and it lives in the object store
                    // rather than in the row: writing the row first would put
                    // the change live before anyone reviewed it (ADR-0038
                    // decision 9). The tier is inside the memory object's
                    // address, so the approvals bind it the way they bind
                    // bytes, with no recheck.
                    asset.sensitivity = proposed_tier;
                }
                let object = vedaflow::put_memory(&mut tx, tenant_id, &asset).await?;
                (asset.entry_name(), object.hash)
            }
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
            // Same story again, and for the same reason: the file's object
            // was stored at authoring — the draft row's foreign key required
            // it — so this write dedups, and it runs anyway because a member
            // reached through the *published* sense has no draft row here.
            Proposed::Skill(asset) => {
                let object = vedaflow::put_skill(&mut tx, tenant_id, asset).await?;
                (asset.entry_name(), object.hash)
            }
        };
        members.push((entry, hash));
    }
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
            asset,
            effect,
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
            // Names and addresses, never content. `record_id` is kept for
            // a memory proposal because AUD-2's disclosure query reads it;
            // an authored asset is named rather than identified, and the
            // `member` key is the one both kinds carry.
            "records": members.iter().map(|(name, hash)| json!({
                "member": name,
                "record_id": (asset == AssetKind::Memory).then(|| name.clone()),
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

// ── Checklist: the reviewer's half of the quality score ────────────────

/// What a reviewer submits (SKIL-3, ADR-0053 decision 6).
#[derive(Deserialize)]
pub(crate) struct ChecklistBody {
    /// Item → verdict, by the wire names: `instructions-correct`,
    /// `scope-appropriate`, `not-duplicate`, `dependencies-available`,
    /// `tested`, each `yes`, `no` or `n/a`.
    answers: BTreeMap<ChecklistItem, ChecklistVerdict>,
    /// Anything the reviewer wants to say, in prose. Scanned before it is
    /// stored, like every other author-supplied text in the product.
    #[serde(default)]
    note: Option<String>,
}

#[derive(Serialize)]
struct ChecklistResponse {
    proposal_id: ProposalId,
    skill: String,
    /// The digest the answers are bound to — what the publish gate will
    /// look them up by, echoed so a caller can see the binding is to
    /// bytes rather than to this proposal.
    bundle_digest: String,
    complete: bool,
    concerns: Vec<&'static str>,
    /// The quality of the bundle these answers are about, recomputed.
    quality: crate::skills::QualityReport,
}

/// `POST /v1/proposals/{id}/checklist` — record the reviewer's half of a
/// skill's quality score.
///
/// **Its own act, not part of casting a verdict.** An approval says "ship
/// it"; a checklist says "here is what I checked", and a reviewer may
/// legitimately do the second without the first — indeed the whole point
/// of a mandatory checklist under `regulated-strict` is that somebody
/// worked through it *before* anyone decided.
///
/// It takes `ProposalReview` at the target, which is the same decision
/// approving takes: the people who may weigh a proposal are the people who
/// may record what they weighed. It does not take `SkillRead` — a proposal
/// carries the bytes to its reviewer without granting them the registry
/// (ADR-0032 decision 16), and a checklist is a statement about what the
/// proposal already showed them.
#[tracing::instrument(name = "proposals.checklist", skip_all)]
pub(crate) async fn checklist(
    State(state): State<AppState>,
    Path(id): Path<ProposalId>,
    payload: std::result::Result<Json<ChecklistBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        checklist_inner(&state, id, body).await
    }
    .await;
    respond(&state, "checklist", result).await
}

async fn checklist_inner(
    state: &AppState,
    id: ProposalId,
    body: ChecklistBody,
) -> Result<Json<ChecklistResponse>> {
    let submitted = Checklist {
        answers: body.answers,
        note: body.note,
    };
    submitted.validate()?;

    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let proposal = load(&mut tx, tenant_id, id).await?;
    if proposal.asset != AssetKind::Skill {
        return Err(Error::Invalid {
            message: format!(
                "proposal {id} carries {}, and a quality checklist is a statement about a \
                 skill bundle; there is nothing here for it to be about",
                proposal.asset.as_str()
            ),
        });
    }
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
    let reviewer = identity_of(&input)?;

    // The note is the first author-supplied prose this plane stores that
    // is not a bundled file, so it goes through MEM-2's scanner before it
    // is written. A reason carrying a credential is **refused rather than
    // scrubbed**: unlike a bundled file there is nothing a placeholder
    // would preserve, and the person who wrote it is on the other end of
    // this request.
    if let Some(note) = &submitted.note {
        crate::skills::refuse_if_secret("checklist note", note).await?;
    }

    let members = vedaflow::proposals::members(&mut tx, tenant_id, proposal.commit).await?;
    let names: Vec<String> = members.iter().map(|member| member.name.clone()).collect();
    let skill = crate::skills::skill_of(&names).ok_or_else(|| Error::Invalid {
        message: format!("proposal {id} does not carry exactly one skill bundle"),
    })?;
    let pairs: Vec<(&str, vedaflow::hash::ObjectHash)> = members
        .iter()
        .map(|member| (member.name.as_str(), member.object))
        .collect();
    let digest = vedaflow::bundle_digest(&pairs);

    // The rubric over the same bytes, so the response tells the reviewer
    // what their answers were recorded *against*.
    let files = crate::skills::files_of_members(&mut tx, tenant_id, &members).await?;
    let scored = crate::skills::score_quality(&files).await?;

    let stored = synveda_store::skill_reviews::record(
        &mut *tx,
        tenant_id,
        &synveda_store::skill_reviews::NewReview {
            // The **source** scope, which is where the bundle is drafted
            // and where the publish seam will look the answers up. A
            // climb's target is a different node, and keying by it would
            // hide the checklist from the very gate it exists for.
            scope_id: proposal.source_scope_id,
            skill_name: &skill,
            bundle_digest: digest,
            checklist: &submitted,
            rubric_version: scored.rubric_version,
            reviewer,
        },
    )
    .await?;

    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::SkillChecklistRecorded,
        Resource::Scope(node.id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::ProposalReview, &authorized),
            "asset": AssetKind::Skill.as_str(),
            "proposal_id": id,
            "skill": skill.as_str(),
            // The binding. An auditor reading this event can tell exactly
            // which bytes were judged, and a later publication of
            // different bytes will not find these answers.
            "bundle_digest": crate::skills::hex(&digest),
            "rubric_version": scored.rubric_version,
            "score": scored.score,
            "answers": submitted
                .answers
                .iter()
                .map(|(item, verdict)| json!({
                    "item": item.as_str(),
                    "verdict": verdict.as_str(),
                }))
                .collect::<Vec<_>>(),
            "complete": submitted.is_complete(),
            "concerns": submitted
                .concerns()
                .iter()
                .map(|item| item.as_str())
                .collect::<Vec<_>>(),
            // The note rides the chain because a reviewer wrote it to be
            // read, and it has already passed the scanner.
            "note": submitted.note,
        }),
    )
    .await?;
    commit(tx).await?;

    let effective = state
        .pdp
        .effective(tenant_id, Resource::Scope(node.id), &input.context());
    Ok(Json(ChecklistResponse {
        proposal_id: id,
        skill: skill.to_string(),
        bundle_digest: crate::skills::hex(&digest),
        complete: stored.checklist.is_complete(),
        concerns: stored
            .checklist
            .concerns()
            .iter()
            .map(ChecklistItem::as_str)
            .collect(),
        quality: crate::skills::QualityReport::new(
            &scored,
            &effective.quality,
            &digest,
            Some(&stored),
        ),
    }))
}

// ── The quality override ───────────────────────────────────────────────

/// What an override-holder sends (SKIL-3, ADR-0053 decision 8).
#[derive(Deserialize)]
pub(crate) struct OverrideBody {
    /// Why this bundle should ship despite being below the bar.
    ///
    /// Mandatory, and it is the whole value of the act: it is what an
    /// auditor reads in a year to find out why the product shipped
    /// something it had itself marked down.
    reason: String,
}

#[derive(Serialize)]
struct OverrideResponse {
    proposal_id: ProposalId,
    skill: String,
    /// The digest the override is granted over. An edit produces different
    /// bytes and this override does not follow them.
    bundle_digest: String,
    score: u8,
    /// The bars it steps over, so the response says what was excused.
    shortfalls: Vec<QualityShortfall>,
    granted_by: IdentityId,
    granted_at: DateTime<Utc>,
}

/// `POST /v1/proposals/{id}/quality-override` — record a decision to
/// publish a bundle the quality gate refuses.
///
/// **A separate act from the publication, and that is the design rather
/// than a convenience.** Under the product packs `curator` holds the
/// `SkillRead` and `ChannelPublish` that publishing a skill takes, and
/// `steward` holds this action and no content read at all — so putting the
/// override on the publish request would have meant nobody could publish a
/// below-bar bundle under any pack. Splitting it is ADR-0032 decision 9's
/// own shape, the one that already separates the approval that decides
/// from the act that runs the effect.
///
/// It is refused when the bundle needs no override, which keeps the audit
/// trail honest: an override standing over a bundle that never needed one
/// is a record of a decision nobody had to make.
#[tracing::instrument(name = "proposals.quality_override", skip_all)]
pub(crate) async fn quality_override(
    State(state): State<AppState>,
    Path(id): Path<ProposalId>,
    payload: std::result::Result<Json<OverrideBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        quality_override_inner(&state, id, &body.reason).await
    }
    .await;
    respond(&state, "quality_override", result).await
}

async fn quality_override_inner(
    state: &AppState,
    id: ProposalId,
    reason: &str,
) -> Result<Json<OverrideResponse>> {
    check_text("reason", Some(reason))?;
    if reason.trim().is_empty() {
        return Err(Error::Invalid {
            message: "an override must say why: it is the whole of what the audit trail will \
                      carry about a publication the product itself scored below the bar"
                .to_owned(),
        });
    }

    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let proposal = load(&mut tx, tenant_id, id).await?;
    if proposal.asset != AssetKind::Skill {
        return Err(Error::Invalid {
            message: format!(
                "proposal {id} carries {}, and the quality gate is a statement about a skill \
                 bundle; there is nothing here to override",
                proposal.asset.as_str()
            ),
        });
    }
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
        Action::SkillQualityOverride,
        Resource::Scope(node.id),
    )?;
    require_open(&proposal)?;
    let granter = identity_of(&input)?;
    crate::skills::refuse_if_secret("override reason", reason).await?;

    let members = vedaflow::proposals::members(&mut tx, tenant_id, proposal.commit).await?;
    let names: Vec<String> = members.iter().map(|member| member.name.clone()).collect();
    let skill = crate::skills::skill_of(&names).ok_or_else(|| Error::Invalid {
        message: format!("proposal {id} does not carry exactly one skill bundle"),
    })?;
    let pairs: Vec<(&str, vedaflow::hash::ObjectHash)> = members
        .iter()
        .map(|member| (member.name.as_str(), member.object))
        .collect();
    let digest = vedaflow::bundle_digest(&pairs);

    let files = crate::skills::files_of_members(&mut tx, tenant_id, &members).await?;
    let scored = crate::skills::score_quality(&files).await?;
    let review = synveda_store::skill_reviews::for_bundle(
        &mut *tx,
        tenant_id,
        proposal.source_scope_id,
        &skill,
        &digest,
    )
    .await?;
    let effective = state
        .pdp
        .effective(tenant_id, Resource::Scope(node.id), &input.context());
    let shortfalls = effective
        .quality
        .shortfalls(scored.score, review.as_ref().map(|r| &r.checklist));
    if shortfalls.is_empty() {
        return Err(Error::Conflict {
            message: format!(
                "skill {skill} already clears this pack's quality bar (rubric v{} scored it \
                 {}/100); an override standing over a bundle that never needed one is a \
                 record of a decision nobody had to make",
                scored.rubric_version, scored.score,
            ),
        });
    }

    let stored = synveda_store::skill_reviews::grant_override(
        &mut *tx,
        tenant_id,
        &synveda_store::skill_reviews::NewOverride {
            // The source scope, where the bundle is drafted and where the
            // publish seam will look this up.
            scope_id: proposal.source_scope_id,
            skill_name: &skill,
            bundle_digest: digest,
            reason,
            score: scored.score,
            rubric_version: scored.rubric_version,
            granter,
        },
    )
    .await?;

    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::SkillQualityOverridden,
        Resource::Scope(node.id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::SkillQualityOverride, &authorized),
            "asset": AssetKind::Skill.as_str(),
            "proposal_id": id,
            "skill": skill.as_str(),
            "rubric_version": scored.rubric_version,
            "score": scored.score,
            "min_score": effective.quality.min_score,
            // Which bars were stepped over, so an auditor filtering for
            // "what did we ship below the bar" can tell a low-scoring
            // bundle from one a reviewer objected to.
            "shortfalls": shortfalls
                .iter()
                .map(|shortfall| json!({
                    "kind": serde_json::to_value(shortfall)
                        .ok()
                        .and_then(|value| value.get("kind").cloned())
                        .unwrap_or(serde_json::Value::Null),
                    "detail": shortfall.describe(),
                }))
                .collect::<Vec<_>>(),
            "bundle_digest": crate::skills::hex(&digest),
            "reason": reason,
        }),
    )
    .await?;
    commit(tx).await?;
    metrics::counter!(
        crate::telemetry::SKILL_QUALITY_OVERRIDES_TOTAL,
        "pack" => authorized.decision.pack_name.clone(),
    )
    .increment(1);

    Ok(Json(OverrideResponse {
        proposal_id: id,
        skill: skill.to_string(),
        bundle_digest: crate::skills::hex(&digest),
        score: stored.score,
        shortfalls,
        granted_by: stored.granted_by,
        granted_at: stored.granted_at,
    }))
}

// ── Publish: the proposal's effect ─────────────────────────────────────

/// `POST /v1/proposals/{id}/apply` — run an approved typed Knowledge
/// effect. The command layer repeats ownership, PDP and revision checks at
/// this boundary; approvals never become write authority by themselves.
#[tracing::instrument(name = "proposals.apply", skip_all)]
pub(crate) async fn apply(State(state): State<AppState>, Path(id): Path<ProposalId>) -> Response {
    let result = crate::knowledge::apply_reviewed(&state, id).await.map(Json);
    respond(&state, "apply", result).await
}

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
    // running a prompt publication's effect exactly as it keeps them from
    // running a memory one's.
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

    // Approvals bind bytes. Recompute every member's address from the
    // record as it stands *now* and require it to equal what the approved
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
    if proposal.asset == AssetKind::Skill {
        // The pack in force at the *target*, resolved here because this is
        // where the decision input lives. SKIL-2's gate runs inside the
        // publish seam, so a bundle approved under one threshold and
        // published under another is decided by the one standing when the
        // bytes go fleet-wide (ADR-0052 decision 5).
        let effective = state
            .pdp
            .effective(tenant_id, Resource::Scope(node.id), &input.context());
        return publish_skills(
            state,
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
            effective.scan,
            effective.quality,
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
/// It writes `context-pack/published` rather than `memory/published`, and it
/// chains the same `vedaflow.channel.published` event with `asset` reading
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

/// The publish effect for a skill proposal (SKIL-1, ADR-0051 decision 1).
///
/// [`publish_documents`] line for line, one table over: approvals bind
/// bytes, so every member's address is required to equal what the approved
/// commit named, and the source must still hold it. Every refusal is a
/// `Conflict`, because the request was well formed when it was approved and
/// what moved is the world.
///
/// It writes `skill/published` and chains the same
/// `vedaflow.channel.published` event with `asset` reading `skill` — the
/// same governed act with the same consequence (ADR-0019 decision 4).
///
/// **What it does not do is write a file.** Publication moves a ref; the
/// bytes reach a client when somebody runs `synveda skill install`, and the
/// receipt of that lives in the CLI's own state rather than in the bundle
/// (ADR-0051 decision 12).
#[allow(clippy::too_many_arguments)]
async fn publish_skills(
    state: &AppState,
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
    scan_config: SkillScanConfig,
    quality_config: SkillQualityConfig,
) -> Result<Json<PublishResponse>> {
    let source = proposal.source_scope_id;
    let paths: Vec<SkillPath> = proposed
        .iter()
        .map(|member| {
            member
                .name
                .parse::<SkillPath>()
                .map_err(|err| Error::Internal {
                    message: format!(
                        "proposal member {:?} is not a skill path: {err}",
                        member.name
                    ),
                })
        })
        .collect::<Result<_>>()?;
    let drafts: std::collections::HashMap<SkillPath, [u8; 32]> =
        synveda_store::skills::list_all_files(&mut *tx, tenant_id, source)
            .await?
            .into_iter()
            .map(|file| {
                (
                    SkillPath::new(file.skill_name.clone(), file.path.clone()),
                    file.object_hash,
                )
            })
            .collect();
    let published_at_source = published_skills_at(&mut tx, tenant_id, source).await?;

    let moved = |what: &str, path: &SkillPath| Error::Conflict {
        message: format!(
            "skill file {path} {what} after this proposal was approved; withdraw it and \
             open a new one so the change is reviewed"
        ),
    };
    let mut members: Vec<(String, vedaflow::hash::ObjectHash)> = Vec::with_capacity(paths.len());
    for (member, path) in proposed.iter().zip(&paths) {
        match drafts.get(path) {
            Some(address) if member.object.as_bytes() == address => {}
            Some(_) => return Err(moved("changed", path)),
            None => {
                // No draft here — but a file *removed* from the bundle since
                // the review lands here too, which is exactly the case
                // ADR-0051 decision 17's DELETE grant creates and exactly
                // the one this check exists to catch: a publication that
                // silently dropped it would ship a bundle a reviewer never
                // saw.
                if published_at_source.get(path) != Some(&member.object) {
                    return Err(Error::Conflict {
                        message: format!(
                            "scope {source} no longer holds skill file {path}; the bundle \
                             was approved against files its source has since changed or \
                             removed"
                        ),
                    });
                }
            }
        }
        members.push((member.name.clone(), member.object));
    }

    // ── The security gate, on exactly the approved bytes ───────────────
    //
    // SKIL-2, ADR-0052 decision 5. The loop above re-verified every
    // address because approvals bind bytes; this re-verifies what those
    // bytes *do*, because the rule table is what says whether they are
    // publishable and it moves independently of them. A rule that landed
    // between authoring and approval must not be one a proposal outruns.
    //
    // Its refusal is a `Conflict` for the same reason every other refusal
    // in this function is: the request was well formed and was well
    // formed when it was approved — what changed is the world.
    let addresses: Vec<vedaflow::hash::ObjectHash> =
        members.iter().map(|(_, hash)| *hash).collect();
    let objects = vedaflow::read_objects(&mut tx, tenant_id, &addresses).await?;
    let files: Vec<SkillFile> = members
        .iter()
        .filter_map(|(_, hash)| objects.get(hash))
        .filter_map(|object| vedaflow::SkillAsset::from_bytes(&object.content).ok())
        .map(|asset| asset.file)
        .collect();
    let security = crate::skills::scan_security(&files).await?;
    if security.blocked_by(&scan_config) {
        // The gate's own transaction: this one is about to be dropped, and
        // the refusal has to chain even though the publication does not.
        drop(tx);
        let pack = format!(
            "{}@{}",
            authorized.decision.pack_name, authorized.decision.pack_version
        );
        let named = paths
            .first()
            .map_or_else(|| proposal.title.clone(), |path| path.skill.to_string());
        return crate::skills::refuse_scan(
            state,
            tenant_id,
            proposal.target_scope_id,
            &named,
            &security,
            &scan_config,
            &pack,
            "publication",
        )
        .await;
    }

    // ── The quality gate ───────────────────────────────────────────────
    //
    // SKIL-3, ADR-0053 decision 7. Three bars, each named separately in
    // the refusal because the remedy differs: an edit for a low score, a
    // reviewer for a missing checklist, a conversation for a concern.
    //
    // **Recomputed here, never read from the registry cache** (decision
    // 3). The cache on the `skills` row exists so a listing can draw a
    // column without reading forty bundles; a gate that read it would be
    // deciding on a number that was true when somebody last authored,
    // which is exactly the staleness the recompute exists to avoid.
    //
    // The checklist is looked up at the **source** scope by a digest of
    // exactly these members, which is the same lookup the review surface
    // did — so answers given about these bytes are found, and answers
    // given about an earlier draft are not (decision 4).
    let scored = crate::skills::score_quality(&files).await?;
    let digest = {
        let pairs: Vec<(&str, vedaflow::hash::ObjectHash)> = members
            .iter()
            .map(|(name, hash)| (name.as_str(), *hash))
            .collect();
        vedaflow::bundle_digest(&pairs)
    };
    let named = paths
        .first()
        .map_or_else(|| proposal.title.clone(), |path| path.skill.to_string());
    let review = match crate::skills::skill_of(
        &members
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>(),
    ) {
        Some(skill) => {
            synveda_store::skill_reviews::for_bundle(&mut *tx, tenant_id, source, &skill, &digest)
                .await?
        }
        None => None,
    };
    let mut overridden: Option<synveda_store::skill_reviews::StoredOverride> = None;
    let shortfalls = quality_config.shortfalls(scored.score, review.as_ref().map(|r| &r.checklist));
    if !shortfalls.is_empty() {
        // **The override is looked up, not sent.** It is a separate
        // governed act by a separate authority (ADR-0053 decision 8), and
        // it has to be: under the product packs `curator` holds the
        // `SkillRead` and `ChannelPublish` that publishing a skill takes,
        // while `steward` holds the override and no content read at all —
        // so requiring one principal to hold both would mean nobody could
        // publish a below-bar bundle under any pack. That is ADR-0032
        // decision 9's own separation, arriving one seam later: the
        // authority records the override, the publisher spends it.
        //
        // It is looked up by the **same digest** as the checklist, so an
        // override granted over one bundle does not carry to whatever the
        // author edits it into. Nobody agreed to ship that.
        let standing = match crate::skills::skill_of(
            &members
                .iter()
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>(),
        ) {
            Some(skill) => {
                synveda_store::skill_reviews::override_for(
                    &mut *tx, tenant_id, source, &skill, &digest,
                )
                .await?
            }
            None => None,
        };
        let Some(standing) = standing else {
            // A `Conflict` rather than an `Invalid`, on this function's
            // own rule: the request is well formed, and what stands in
            // its way is a state — the bytes scored what they scored, and
            // nobody has recorded an override over them.
            return Err(Error::Conflict {
                message: format!(
                    "skill {named} is below this pack's quality bar: {}. \
Somebody holding `SkillQualityOverride` at this scope — a steward or an org-admin under \
the product packs, deliberately not the role that publishes — can record an override with \
`POST /v1/proposals/{id}/quality-override`, after which this publication proceeds. \
Or fix the bundle and open a new proposal",
                    shortfalls
                        .iter()
                        .map(QualityShortfall::describe)
                        .collect::<Vec<_>>()
                        .join("; "),
                ),
            });
        };
        // The publication rides an override somebody else already
        // recorded and the chain already carries. What is added here is
        // the join: this commit went out under that decision.
        overridden = Some(standing);
    }

    let channel = vedaflow::ChannelRef::skill(Channel::Published);
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
            // What the rubric made of the bytes that just went fleet-wide,
            // and — when they were below the bar — whose override this
            // publication spent (SKIL-3, ADR-0053).
            //
            // The override chained an event of its own when it was
            // granted, so this is deliberately not a second copy of it: it
            // is the *join*, the thing that says this commit went out
            // under that decision. Without it an auditor holding a
            // published commit would have to go looking for an override by
            // digest to find out whether one was involved at all.
            "quality": {
                "rubric_version": scored.rubric_version,
                "score": scored.score,
                "min_score": quality_config.min_score,
                "overridden_by": overridden.as_ref().map(|granted| json!({
                    "granted_by": granted.granted_by,
                    "granted_at": granted.granted_at,
                    "reason": granted.reason,
                })),
            },
            "target_scope_id": proposal.target_scope_id,
            // Paths and addresses, never SKILL.md text and never file
            // content.
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

// ── Classify: the other effect ─────────────────────────────────────────

#[derive(Serialize)]
struct ClassifyResponse {
    proposal_id: ProposalId,
    scope_id: ScopeId,
    /// The tier every named record now carries.
    sensitivity: Sensitivity,
    /// The records that moved, with the tier each left.
    records: Vec<ClassifiedRecord>,
    proposal_commit: String,
    state: &'static str,
}

#[derive(Serialize)]
struct ClassifiedRecord {
    record_id: RecordId,
    /// What the record carried before this effect ran — the half of the
    /// change an auditor needs to price it (ADR-0038 decision 9).
    was: Sensitivity,
}

/// `POST /v1/proposals/{id}/classify` — run an approved classification
/// proposal's effect (AUTHZ-5, ADR-0038 decision 9).
#[tracing::instrument(name = "proposals.classify", skip_all)]
pub(crate) async fn classify(
    State(state): State<AppState>,
    Path(id): Path<ProposalId>,
) -> Response {
    let result = classify_inner(&state, id).await;
    respond(&state, "classify", result).await
}

async fn classify_inner(state: &AppState, id: ProposalId) -> Result<Json<ClassifyResponse>> {
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
    // Two decisions, the shape every governed act over material takes: may
    // this principal classify here, and is it a stranger to the material.
    // The read is at the working tier (ADR-0038 decision 10) — a
    // reclassification discloses nothing to the actor, and how much
    // authority the tier takes is the matrix's arithmetic, resolved below at
    // the maximum of both tiers.
    let authorized = authz::decide(
        state,
        &input,
        Action::MemoryClassify,
        Resource::Scope(node.id),
    )?;
    authz::decide_read(
        state,
        &input,
        Resource::Scope(node.id),
        Sensitivity::WORKING,
    )?;
    require_open(&proposal)?;
    require_effect(&proposal, ProposalEffect::Classify, "classify")?;
    let classifier = identity_of(&input)?;

    let requirement = requirement_for(state, &mut tx, tenant_id, &input, &node, &proposal).await?;
    let recorded = vedaflow::proposals::approvals(&mut tx, tenant_id, id).await?;
    let cast = vedaflow::proposals::cast_for(&recorded, proposal.commit);
    let outstanding = requirement.outstanding(&cast);
    if !outstanding.is_empty() {
        metrics::counter!(PUBLISH_REVIEW_REQUIRED_TOTAL, "surface" => "classify").increment(1);
        return Err(Error::Conflict {
            message: format!(
                "proposal {id} still needs {}; it cannot reclassify yet",
                outstanding.describe()
            ),
        });
    }

    // Approvals bind bytes here exactly as they do for a publication, with
    // one substitution: the member objects were written at the *proposed*
    // tier, so the address to compare against is the live record's with that
    // tier substituted. Everything else — content, class, scope, validity —
    // must be untouched, or the record moved under its own review and this
    // effect would install a tier decided about different bytes.
    let proposed = vedaflow::proposals::members(&mut tx, tenant_id, proposal.commit).await?;
    let ids = member_ids(&proposed)?;
    let versions = records::current_many(&mut *tx, tenant_id, &ids).await?;
    let moved = |what: &str, record: RecordId| Error::Conflict {
        message: format!(
            "record {record} {what} after this proposal was approved; withdraw it and \
             open a new one so the change is reviewed"
        ),
    };
    let mut classified: Vec<ClassifiedRecord> = Vec::with_capacity(ids.len());
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
        let mut asset = memory_asset(version.id, &version.state);
        asset.sensitivity = proposal.sensitivity;
        if asset.address() != member.object {
            return Err(moved("changed", record));
        }
        // The record must still live where the proposal was decided: a
        // reclassification is authorized at one scope, and material that
        // moved out of it since is governed somewhere else now.
        if version.state.scope_id != proposal.target_scope_id {
            return Err(Error::Conflict {
                message: format!(
                    "record {record} no longer lives at scope {}; its classification is \
                     decided where it lives",
                    proposal.target_scope_id
                ),
            });
        }
        let was = version.state.sensitivity;
        records::reclassify(&mut *tx, tenant_id, record, proposal.sensitivity)
            .await?
            .ok_or_else(|| moved("no longer exists", record))?;
        classified.push(ClassifiedRecord {
            record_id: record,
            was,
        });
    }

    close(
        &mut tx,
        tenant_id,
        id,
        ProposalState::Published,
        classifier,
        None,
    )
    .await?;

    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::MemoryClassified,
        Resource::Scope(proposal.target_scope_id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::MemoryClassify, &authorized),
            "proposal_id": id,
            "proposal_commit": proposal.commit.to_hex(),
            "scope_id": proposal.target_scope_id,
            // Both tiers per record, because the requirement was resolved at
            // their maximum and an auditor reading this event has to be able
            // to see why it cost what it cost.
            "sensitivity": proposal.sensitivity.as_str(),
            "records": classified.iter().map(|record| json!({
                "record_id": record.record_id,
                "was": record.was.as_str(),
                "now": proposal.sensitivity.as_str(),
            })).collect::<Vec<_>>(),
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

    Ok(Json(ClassifyResponse {
        proposal_id: id,
        scope_id: proposal.target_scope_id,
        sensitivity: proposal.sensitivity,
        records: classified,
        proposal_commit: proposal.commit.to_hex(),
        state: ProposalState::Published.as_str(),
    }))
}

/// Refuses a proposal whose effect is not the one this route runs.
///
/// A route per effect, and each one checks: running a classification
/// through the publish route would move a channel to member objects
/// carrying a tier no row has, and running a publication through this one
/// would rewrite tiers nobody proposed.
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
        closed_at: proposal.closed_at,
        close_reason: proposal.close_reason.clone(),
        required: RequirementView::of(requirement),
        outstanding: outstanding.describe(),
        promotion: proposal.evidence.clone(),
    }
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
        return skill_member_views(tx, tenant_id, proposal, &proposed).await;
    }
    if proposal.asset == AssetKind::Prompt {
        return prompt_member_views(tx, tenant_id, proposal, &proposed).await;
    }
    let ids = member_ids(&proposed)?;
    // Read wherever the records live, not at the target: a climb's members
    // live below it, and rendering them as "changed, no content" would
    // make every climb unreviewable (ADR-0034 decision 3). The address
    // comparison below is what says whether they still match the review.
    let versions = records::current_many(&mut *tx, tenant_id, &ids).await?;
    // The baseline is the **target's** channel, which for a climb is the
    // ancestor's: what the proposal would move is the target's published
    // set, so that is what the diff is against.
    let published = published_at(tx, tenant_id, proposal.target_scope_id).await?;
    // Both sides of every member's diff, in one statement rather than two
    // per member (ADR-0035 decision 10).
    let mut wanted: Vec<vedaflow::hash::ObjectHash> =
        proposed.iter().map(|member| member.object).collect();
    wanted.extend(ids.iter().filter_map(|id| published.get(id).copied()));
    let objects = vedaflow::read_objects(tx, tenant_id, &wanted).await?;
    // The store is append-only, so an address a tree or a commit names
    // always resolves; a miss would be corruption, and rendering it as an
    // empty side is honest about having nothing to show rather than
    // failing a review that is otherwise fine.
    let text_at = |hash: &vedaflow::hash::ObjectHash| -> String {
        objects
            .get(hash)
            .map(|object| String::from_utf8_lossy(&object.content).into_owned())
            .unwrap_or_default()
    };

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
            let (effect, baseline) = match published.get(&record_id) {
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
                member: record_id.to_string(),
                record_id: Some(record_id),
                asset: AssetKind::Memory.as_str().to_owned(),
                object_hash: member.object.to_hex(),
                unchanged,
                class: Some(class),
                sensitivity,
                content,
                effect,
                proposed: text_at(&member.object),
                baseline,
            }
        })
        .collect())
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
        record_id: None,
        asset: AssetKind::Knowledge.as_str().to_owned(),
        object_hash: member.object.to_hex(),
        unchanged: payload_matches && manifest_hash == Some(change.payload_hash.as_str()),
        class: None,
        sensitivity: proposal.sensitivity,
        content: content.clone(),
        effect: MemberEffect::Apply,
        proposed: content,
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
                record_id: None,
                asset: AssetKind::ContextPack.as_str().to_owned(),
                object_hash: member.object.to_hex(),
                unchanged,
                class: None,
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
/// The same three questions FLOW-6 asks of a memory member — what the
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
                record_id: None,
                asset: AssetKind::Prompt.as_str().to_owned(),
                object_hash: member.object.to_hex(),
                unchanged,
                class: None,
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

/// [`member_views`] for a skill proposal (SKIL-1, ADR-0051 decision 1).
///
/// [`document_member_views`]'s shape, one table over — and the renderer
/// ADR-0035 predicted for "SKIL-1's skill bundles", arriving as its third
/// kind. A reviewer sees a file at a time, which is what makes reviewing a
/// skill *like reviewing code* rather than like reviewing a blob.
async fn skill_member_views(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    proposal: &vedaflow::StoredProposal,
    proposed: &[vedaflow::ChannelMember],
) -> Result<Vec<MemberView>> {
    let paths: Vec<SkillPath> = proposed
        .iter()
        .map(|member| {
            member
                .name
                .parse::<SkillPath>()
                .map_err(|err| Error::Internal {
                    message: format!(
                        "proposal member {:?} is not a skill path: {err}",
                        member.name
                    ),
                })
        })
        .collect::<Result<_>>()?;
    let drafts: std::collections::HashMap<SkillPath, [u8; 32]> =
        synveda_store::skills::list_all_files(&mut *tx, tenant_id, proposal.source_scope_id)
            .await?
            .into_iter()
            .map(|file| {
                (
                    SkillPath::new(file.skill_name.clone(), file.path.clone()),
                    file.object_hash,
                )
            })
            .collect();
    let published = published_skills_at(tx, tenant_id, proposal.target_scope_id).await?;

    let mut wanted: Vec<vedaflow::hash::ObjectHash> =
        proposed.iter().map(|member| member.object).collect();
    wanted.extend(paths.iter().filter_map(|path| published.get(path).copied()));
    let objects = vedaflow::read_objects(tx, tenant_id, &wanted).await?;
    // The *file's* bytes rather than the envelope's: a reviewer reads the
    // script, not the JSON around it. That is the same content a client
    // will load, which is the whole point of reviewing it here.
    let text_at = |hash: &vedaflow::hash::ObjectHash| -> String {
        objects
            .get(hash)
            .and_then(|object| vedaflow::SkillAsset::from_bytes(&object.content).ok())
            .map(|asset| asset.file.content)
            .unwrap_or_default()
    };

    Ok(proposed
        .iter()
        .zip(&paths)
        .map(|(member, path)| {
            let reviewed = objects
                .get(&member.object)
                .and_then(|object| vedaflow::SkillAsset::from_bytes(&object.content).ok());
            let (unchanged, content) = match drafts.get(path) {
                Some(address) => (member.object.as_bytes() == address, text_at(&member.object)),
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
                record_id: None,
                asset: AssetKind::Skill.as_str().to_owned(),
                object_hash: member.object.to_hex(),
                unchanged,
                class: None,
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
        AssetKind::Memory => authz::decide_read(state, input, resource, Sensitivity::WORKING),
        AssetKind::Prompt => {
            authz::decide_prompt_read(state, input, resource, Sensitivity::WORKING)
        }
        AssetKind::ContextPack => {
            authz::decide_context_pack_read(state, input, resource, Sensitivity::WORKING)
        }
        AssetKind::Skill => authz::decide_skill_read(state, input, resource, Sensitivity::WORKING),
        // `policy` is the one that remains, and it is not proposable here —
        // a lapse is its own route (ADR-0037 decision 16).
        other => Err(Error::Invalid {
            message: format!(
                "{} is not an asset a proposal carries; a policy lapse is POST /v1/lapses",
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
    /// A memory record's current version.
    Memory(synveda_store::records::RecordVersion),
    /// A prompt, as it will be addressed. Either the draft that lives at
    /// the source scope, or — for the second sense of "the source holds
    /// it" — the object the source's published tree already names.
    Prompt(vedaflow::PromptAsset),
    /// One **document** of a context pack, as it will be addressed
    /// (PRMT-2, ADR-0050 decision 3). The same two senses as a prompt, one
    /// table over — and a bundle is several of these rather than one
    /// member, because the channel names documents.
    ContextPack(vedaflow::ContextPackAsset),
    /// One **file** of a skill bundle, as it will be addressed (SKIL-1,
    /// ADR-0051 decision 2). The same two senses again — and unlike the
    /// other three, a caller never names one of these: it names the skill,
    /// and every file of it becomes a member, because a client loads a
    /// bundle whole.
    Skill(vedaflow::SkillAsset),
}

impl Proposed {
    /// The tier the approval matrix prices this member at.
    fn sensitivity(&self) -> Sensitivity {
        match self {
            Proposed::Memory(version) => version.state.sensitivity,
            Proposed::Prompt(asset) => asset.sensitivity,
            // Per document, never per pack (ADR-0050 decision 12) — so a
            // bundle mixing a public glossary and a confidential runbook is
            // priced at the runbook, which is `max_sensitivity`'s existing
            // rule doing exactly what it was written for.
            Proposed::ContextPack(asset) => asset.sensitivity,
            // Per skill rather than per file (ADR-0051 decision 11), so
            // every member of one bundle reports the same tier and the
            // maximum over the set is that tier.
            Proposed::Skill(asset) => asset.sensitivity,
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

/// What `scope`'s published skill channel names, by skill path.
async fn published_skills_at(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    scope_id: ScopeId,
) -> Result<std::collections::HashMap<SkillPath, vedaflow::hash::ObjectHash>> {
    Ok(
        vedaflow::read_skill_members(tx, tenant_id, &[scope_id], Channel::Published)
            .await?
            .into_iter()
            .next()
            .map(|state| state.members)
            .unwrap_or_default(),
    )
}

/// The skills `scope` **holds**, refusing the whole request if any is not
/// held there — [`held_documents`]'s two senses, one table over, with one
/// difference that is the whole of ADR-0051 decision 17.
///
/// A caller names the **bundle** and every file of it becomes a member,
/// because a client loads a skill whole: proposing three files of four
/// would put a version nobody can run in front of a reviewer, and approving
/// it would publish one.
async fn held_skills(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    scope_id: ScopeId,
    names: &[SkillName],
) -> Result<Vec<vedaflow::SkillAsset>> {
    let mut requested: Vec<SkillName> = names.to_vec();
    requested.sort();
    requested.dedup();
    let published = published_skills_at(tx, tenant_id, scope_id).await?;

    let mut held = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for name in &requested {
        // Sense one: the draft lives here, and its files are the bundle.
        let drafts = synveda_store::skills::files_of(&mut *tx, tenant_id, scope_id, name).await?;
        let addresses: Vec<vedaflow::hash::ObjectHash> = if drafts.is_empty() {
            // Sense two: no draft here, but this scope published it — which
            // is how a second hop starts from where the first one landed.
            let names_here: Vec<vedaflow::hash::ObjectHash> = published
                .iter()
                .filter(|(path, _)| &path.skill == name)
                .map(|(_, address)| *address)
                .collect();
            if names_here.is_empty() {
                missing.push(name.to_string());
                continue;
            }
            names_here
        } else {
            drafts
                .into_iter()
                .map(|file| vedaflow::hash::ObjectHash::from_bytes(file.object_hash))
                .collect()
        };
        for address in addresses {
            let object = vedaflow::read_object(&mut *tx, tenant_id, address)
                .await?
                .ok_or_else(|| Error::Internal {
                    message: format!(
                        "skill {name} names object {} which the append-only store does \
                         not hold",
                        address.to_hex()
                    ),
                })?;
            held.push(vedaflow::SkillAsset::from_bytes(&object.content)?);
        }
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

fn validate_open(body: &OpenBody) -> Result<()> {
    let invalid = |message: String| Err(Error::Invalid { message });
    // One asset kind per proposal (ADR-0049 decision 6): the approval
    // matrix resolves from it, and a mixed set would have to be priced at
    // the maximum by a rule nobody wrote.
    let named = usize::from(!body.record_ids.is_empty())
        + usize::from(!body.prompt_names.is_empty())
        + usize::from(!body.document_paths.is_empty())
        + usize::from(!body.skill_names.is_empty());
    match named {
        0 => {
            return invalid(
                "name at least one member: record_ids for memories, prompt_names for \
                 prompts, document_paths for context pack documents, skill_names for \
                 skills"
                    .to_owned(),
            );
        }
        1 => {}
        _ => {
            return invalid(
                "a proposal carries one asset kind: name record_ids, prompt_names, \
                 document_paths or skill_names, never more than one — the approval matrix resolves \
                 from the asset, and regulated-strict prices a prompt at two distinct \
                 people where it prices a team's memory at one"
                    .to_owned(),
            );
        }
    }
    let members = body
        .record_ids
        .len()
        .max(body.prompt_names.len())
        .max(body.document_paths.len())
        .max(body.skill_names.len());
    if members > vedaflow::MAX_PROPOSAL_MEMBERS {
        return invalid(format!(
            "a proposal may name at most {} members",
            vedaflow::MAX_PROPOSAL_MEMBERS
        ));
    }
    if (!body.prompt_names.is_empty() || !body.skill_names.is_empty())
        && body.effect.unwrap_or(ProposalEffect::Published) != ProposalEffect::Published
    {
        return invalid(
            "an authored-asset proposal publishes: reclassification is a records effect \
             (ADR-0038 decision 9), and an authored asset's tier is a field of the \
             version under review"
                .to_owned(),
        );
    }
    let chars = body.title.chars().count();
    if chars == 0 || chars > MAX_TITLE_CHARS {
        return invalid(format!("title must be 1..={MAX_TITLE_CHARS} characters"));
    }
    // The effect and the tier travel together or not at all (AUTHZ-5,
    // ADR-0038 decision 9). A body that says `classify` without a tier has
    // not said what it would do; one that names a tier for a publication has
    // described something the effect will not perform, and storing it would
    // make the proposal read as a reclassification that never happens.
    match body.effect.unwrap_or(ProposalEffect::Published) {
        ProposalEffect::Classify if body.sensitivity.is_none() => {
            return invalid(
                "a classify proposal must name the sensitivity it would install".to_owned(),
            );
        }
        ProposalEffect::Classify => {}
        other if body.sensitivity.is_some() => {
            return invalid(format!(
                "sensitivity applies to the classify effect only; this proposal's \
                 effect is {other} and would not move a tier"
            ));
        }
        // A lapse's terms are a different body entirely, and refusing it
        // here by name beats opening a policy proposal with no terms in it
        // (ADR-0037 decision 1: POST /v1/lapses is its surface).
        ProposalEffect::Lapse => {
            return invalid(
                "a lapse is proposed at POST /v1/lapses, which is where its terms live".to_owned(),
            );
        }
        ProposalEffect::Apply => {
            return invalid(
                "a Knowledge change is opened by the governed Knowledge command layer; \
                 the generic proposal route cannot supply its typed effect payload"
                    .to_owned(),
            );
        }
        ProposalEffect::Published => {}
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
