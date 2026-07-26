//! The lapse plane (AUTHZ-4, ADR-0037): `/v1/lapses` behind tenant
//! resolution, uniform-404 ownership, and the PDP.
//!
//! A lapse is seed §6's "scoped, reasoned, time-boxed override" — the
//! mechanism the seed names as the thing that lets one product serve both
//! an SMB and a bank. It is **an ordinary FLOW-3 proposal whose asset is
//! `policy` and whose effect is a grant row**: no new asset kind, no new
//! approval rule, no new proposal action, and FLOW-6's `synveda proposal`
//! reviews one the day it exists.
//!
//! # There is no direct route
//!
//! `POST /v1/lapses` opens a **proposal**; it grants nothing. Under
//! `regulated-strict` the `policy` cell of the matrix asks for two distinct
//! stewards, and even under a pack that asks for one, running the effect is
//! a second call under `LapseGrant`.
//!
//! That is deliberately unlike publishing, which kept its direct route
//! (ADR-0032 decision 8). Publishing is a routine curatorial act whose
//! matrix legitimately says "one curator"; a lapse is by construction an
//! exception with a mandatory reason, and one call that both writes the
//! reason and enacts it is the shape the audit trail exists to make
//! impossible (ADR-0037 option 10).
//!
//! # The disclosing side opens it
//!
//! The target is the scope whose material is disclosed. Requirements
//! resolve there, `ProposalOpen` is decided there, and the packs floor that
//! on membership plus contributor-and-above — so a steward of the team that
//! *wants* the access cannot open the proposal that grants it unless they
//! are also bound above the target. Asking is a conversation; disclosing is
//! an act, and it happens on the disclosing side.
//!
//! # What the grant then does
//!
//! Nothing here decides a read. The row this writes is picked up by
//! `authz::gather` on every subsequent request, gated by the target pack's
//! own ceiling, and turned into `context.lapsed` for the base layer's
//! permit. **Expiry is that query's predicate**, so nothing has to run for
//! the window to close.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{Actor, AuditAction, Outcome};
use synveda_policy::{Action, Resource};
use synveda_store::{hierarchy, lapses, rls};
use synveda_types::{
    AssetKind, Error, HierarchyNode, IdentityId, Lapse, LapseAction, LapseId, LapseOutcome,
    LapseTerms, ProposalEffect, ProposalId, ProposalState, Result, ScopeId, ScopeKind, Sensitivity,
    TenantId,
};
use synveda_vedaflow::{self as vedaflow, LapseAsset, PolicySnapshot, Signer};

use crate::app::AppState;
use crate::approvals;
use crate::audit;
use crate::authz::{self, DecisionInput};
use crate::error::ApiError;
use crate::hierarchy::{body, commit, found, tenant_id};
use crate::telemetry::{LAPSE_EXPIRIES_TOTAL, LAPSE_OPERATIONS_TOTAL};

/// The sensitivity a lapse resolves the approval matrix at (ADR-0037
/// decision 14).
///
/// `internal` is the tier `inject` composes: the read path clamps below
/// `restricted` unconditionally (ADR-0024 decision 2) and requests default
/// to `internal`, so this is the most sensitive material a grant can
/// actually disclose. The invariant floor therefore does not engage and the
/// pack rules decide — `regulated-strict` asks its two stewards, which is
/// tech plan §2.4's lapse row landing where it was written to land.
///
/// When AUTHZ-5 lets a lapse declare a higher ceiling, the matrix resolves
/// at *that* ceiling and the floor engages by itself, with no
/// lapse-specific rule anywhere.
const LAPSE_SENSITIVITY: Sensitivity = Sensitivity::Internal;

/// Counts the operation and renders the result — the outcome taxonomy every
/// governed plane uses.
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
    metrics::counter!(LAPSE_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    match result {
        Ok(response) => response.into_response(),
        Err(error) => {
            audit::record_rejection(state, op, &error).await;
            ApiError(error).into_response()
        }
    }
}

// ── Views ──────────────────────────────────────────────────────────────

/// One standing or historical grant, as the listing and the detail render
/// it.
#[derive(Serialize)]
pub(crate) struct LapseView {
    id: LapseId,
    proposal_id: ProposalId,
    grantee_scope_id: ScopeId,
    target_scope_id: ScopeId,
    #[serde(skip_serializing_if = "Option::is_none")]
    grantee_scope_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_scope_path: Option<String>,
    action: LapseAction,
    reason: String,
    granted_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    granted_by: IdentityId,
    /// Standing, expired, or revoked — rendered from the row rather than
    /// stored on it, the [`synveda_types::ProposalView`] discipline: a
    /// stored state would need something to run to stay true.
    outcome: LapseOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    revoked_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revoke_reason: Option<String>,
}

fn render(lapse: &Lapse, paths: (Option<String>, Option<String>), now: DateTime<Utc>) -> LapseView {
    LapseView {
        id: lapse.id,
        proposal_id: lapse.proposal_id,
        grantee_scope_id: lapse.grantee_scope_id,
        target_scope_id: lapse.target_scope_id,
        grantee_scope_path: paths.0,
        target_scope_path: paths.1,
        action: lapse.action,
        reason: lapse.reason.clone(),
        granted_at: lapse.granted_at,
        expires_at: lapse.expires_at,
        granted_by: lapse.granted_by,
        outcome: lapse.outcome_at(now),
        revoked_at: lapse.revoked_at,
        revoke_reason: lapse.revoke_reason.clone(),
    }
}

// ── Propose ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct ProposeBody {
    /// The scope whose material would be disclosed. Requirements resolve
    /// here, `ProposalOpen` is decided here, and this is the only scope the
    /// permit will cover.
    scope_id: ScopeId,
    /// Who would get the access: every principal placed at or under this
    /// scope. A single person is their own personal scope, so this one
    /// shape covers "team X" and "just Dana".
    grantee_scope_id: ScopeId,
    /// What to relax. A closed vocabulary; anything outside it is refused
    /// by name (ADR-0037 decision 2).
    action: LapseAction,
    /// How long the grant runs **once its effect executes** — never from
    /// now, because a proposal that sits in a queue for a week must not
    /// spend the window it was approved for.
    duration_secs: u32,
    /// Why. Mandatory: it is what two approvers weigh and what an auditor
    /// reads afterwards.
    reason: String,
}

/// `POST /v1/lapses` — open a lapse proposal. Grants nothing.
#[tracing::instrument(name = "lapses.propose", skip_all)]
pub(crate) async fn propose(
    State(state): State<AppState>,
    payload: std::result::Result<Json<ProposeBody>, JsonRejection>,
) -> Response {
    let result = propose_inner(&state, payload).await;
    respond(&state, "propose", result).await
}

async fn propose_inner(
    state: &AppState,
    payload: std::result::Result<Json<ProposeBody>, JsonRejection>,
) -> Result<Json<serde_json::Value>> {
    let request = body(payload)?;
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let target = found(
        hierarchy::node(&mut *tx, request.scope_id).await?,
        tenant_id,
        request.scope_id,
    )?;
    let grantee = found(
        hierarchy::node(&mut *tx, request.grantee_scope_id).await?,
        tenant_id,
        request.grantee_scope_id,
    )?;
    let input = authz::gather(state, &mut tx, Some(&target)).await?;
    let authorized = authz::decide(
        state,
        &input,
        Action::ProposalOpen,
        Resource::Scope(target.id),
        None,
    )?;
    let proposer = identity_of(&input)?;

    let terms = LapseTerms {
        grantee_scope_id: request.grantee_scope_id,
        target_scope_id: request.scope_id,
        action: request.action,
        duration_secs: request.duration_secs,
        reason: request.reason.trim().to_owned(),
    };
    // The ceiling is the **target's** pack, resolved exactly as every other
    // decision about the target resolves it.
    let effective = state
        .pdp
        .effective(tenant_id, Resource::Scope(target.id), &input.context());
    terms.validate(&effective.lapse)?;
    validate_shape(&terms, &target, &grantee)?;

    // The reviewed object: one member, the terms in canonical form. This is
    // what the approvals bind, and because it is the *only* copy of the
    // terms, "approve, edit, run" is structurally impossible rather than
    // caught by a recheck (ADR-0037 decision 1).
    let asset = LapseAsset::new(terms.clone());
    let object = vedaflow::put_lapse(&mut tx, tenant_id, &asset).await?;
    let members = [(asset.entry_name(), object.hash)];

    let open_here = vedaflow::proposals::count_open(&mut tx, tenant_id, target.id).await?;
    if open_here >= vedaflow::MAX_OPEN_PROPOSALS {
        return Err(Error::Conflict {
            message: format!(
                "{open_here} proposals already stand open at this scope, at the \
                 {} limit; review some before opening more",
                vedaflow::MAX_OPEN_PROPOSALS
            ),
        });
    }

    let title = terms.summary();
    let snapshot = PolicySnapshot::new(
        authorized.decision.pack_name.clone(),
        authorized.decision.pack_version,
    );
    let proposal = vedaflow::proposals::open(
        &mut tx,
        tenant_id,
        &vedaflow::NewProposal {
            target_scope: target.id,
            // Nothing moves, so FLOW-5's direction rule keeps its meaning:
            // source equals target, a climb of zero levels (ADR-0037
            // decision 16).
            source_scope: target.id,
            asset: AssetKind::Policy,
            effect: ProposalEffect::Lapse,
            members: &members,
            sensitivity: LAPSE_SENSITIVITY,
            title: &title,
            proposer,
            proposer_subject: &input.principal.subject,
            committed_at: Utc::now(),
            policy_snapshot: &snapshot,
            evidence: None,
        },
        &Signer::Unsigned,
    )
    .await?;

    let entries = [asset.entry_name()];
    let requirement = approvals::resolve(
        state,
        &mut tx,
        tenant_id,
        &input,
        &approvals::Requested {
            target: &target,
            asset: AssetKind::Policy,
            sensitivity: LAPSE_SENSITIVITY,
            entries: &entries,
        },
    )
    .await?;
    let outstanding = requirement.outstanding(&[]);
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::ProposalOpened,
        Resource::Scope(target.id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::ProposalOpen, &authorized),
            "proposal_id": proposal.id,
            "asset": AssetKind::Policy.as_str(),
            "effect": ProposalEffect::Lapse.as_str(),
            "title": title,
            "sensitivity": LAPSE_SENSITIVITY.as_str(),
            "commit": proposal.commit.to_hex(),
            "source_scope_id": target.id,
            "target_scope_id": target.id,
            // The terms, which are admin text rather than memory content:
            // a reviewer and an auditor both need to know what was asked
            // for, and none of this is anybody's memory.
            "lapse": lapse_terms_context(&terms, object.hash.to_hex()),
            "approvals": approvals::audit_context(&requirement, &outstanding),
        }),
    )
    .await?;
    commit(tx).await?;

    Ok(Json(json!({
        "proposal_id": proposal.id,
        "commit": proposal.commit.to_hex(),
        "state": ProposalState::Open.as_str(),
        "effect": ProposalEffect::Lapse.as_str(),
        "target_scope_id": target.id,
        "target_scope_path": target.path,
        "grantee_scope_id": grantee.id,
        "grantee_scope_path": grantee.path,
        "action": terms.action.as_str(),
        "duration_secs": terms.duration_secs,
        "reason": terms.reason,
        "required": requirement.describe(),
        "outstanding": outstanding.describe(),
    })))
}

/// The two refusals that need the hierarchy, which `LapseTerms::validate`
/// cannot make because `synveda-types` knows no nodes.
fn validate_shape(
    terms: &LapseTerms,
    target: &HierarchyNode,
    grantee: &HierarchyNode,
) -> Result<()> {
    // The privacy floor, refused loudly here as well as excluded silently
    // in the permit (ADR-0037 decision 8). Nobody's personal memory is
    // disclosed by a lapse, and an investigation that genuinely needs one
    // person's corpus is a different feature with a different name.
    if target.kind == ScopeKind::User {
        return Err(Error::Invalid {
            message: format!(
                "scope {} is a personal scope; no lapse discloses one, under any pack. \
                 Lapse the team it sits under, or ask its owner to publish",
                target.id
            ),
        });
    }
    // A target the grantee already reaches through its own chain grants
    // nothing: `lapsed_scopes` would drop it from every plan, and the
    // proposal would consume two stewards' review to change nothing.
    if grantee.path == target.path || grantee.path.starts_with(&format!("{}/", target.path)) {
        return Err(Error::Invalid {
            message: format!(
                "scope {} already composes {} through its own chain; a lapse there \
                 would grant nothing",
                grantee.id, target.id
            ),
        });
    }
    let _ = terms;
    Ok(())
}

// ── Grant: the proposal's effect ───────────────────────────────────────

/// `POST /v1/proposals/{id}/lapse` — run an approved lapse proposal's
/// effect.
///
/// The parallel of `/publish`, and it takes the same shape: the proposal
/// open, the requirement satisfied, and one Cedar decision — here
/// `LapseGrant` at the target, the scope whose material is disclosed.
#[tracing::instrument(name = "lapses.grant", skip_all)]
pub(crate) async fn grant(State(state): State<AppState>, Path(id): Path<ProposalId>) -> Response {
    let result = grant_inner(&state, id).await;
    respond(&state, "grant", result).await
}

async fn grant_inner(state: &AppState, id: ProposalId) -> Result<Json<LapseView>> {
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let proposal = vedaflow::proposals::read(&mut tx, tenant_id, id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("proposal {id}"),
        })?;
    if proposal.effect != ProposalEffect::Lapse {
        return Err(Error::Invalid {
            message: format!(
                "proposal {id} publishes onto a channel; run its effect with \
                 POST /v1/proposals/{id}/publish"
            ),
        });
    }
    let target = found(
        hierarchy::node(&mut *tx, proposal.target_scope_id).await?,
        tenant_id,
        proposal.target_scope_id,
    )?;
    let input = authz::gather(state, &mut tx, Some(&target)).await?;
    let authorized = authz::decide(
        state,
        &input,
        Action::LapseGrant,
        Resource::Scope(target.id),
        None,
    )?;
    if proposal.state.is_terminal() {
        return Err(Error::Conflict {
            message: format!("proposal {id} is already {}", proposal.state),
        });
    }
    let granter = identity_of(&input)?;

    let entries = member_names(&mut tx, tenant_id, &proposal).await?;
    let requirement = approvals::resolve(
        state,
        &mut tx,
        tenant_id,
        &input,
        &approvals::Requested {
            target: &target,
            asset: AssetKind::Policy,
            sensitivity: proposal.sensitivity,
            entries: &entries,
        },
    )
    .await?;
    let recorded = vedaflow::proposals::approvals(&mut tx, tenant_id, id).await?;
    let cast = vedaflow::proposals::cast_for(&recorded, proposal.commit);
    let outstanding = requirement.outstanding(&cast);
    if !outstanding.is_empty() {
        return Err(Error::Conflict {
            message: format!(
                "proposal {id} still needs {}; the lapse cannot be granted yet",
                outstanding.describe()
            ),
        });
    }

    // The terms come from the reviewed object, never from a request body:
    // what runs is exactly what was approved.
    let members = vedaflow::proposals::members(&mut tx, tenant_id, proposal.commit).await?;
    let [member] = members.as_slice() else {
        return Err(Error::Internal {
            message: format!(
                "lapse proposal {id} names {} members; a lapse has exactly one",
                members.len()
            ),
        });
    };
    let terms = vedaflow::read_lapse(&mut tx, tenant_id, member.object).await?;

    // The ceiling is re-resolved here, not trusted from open time: a pack
    // that stopped admitting lapses between the review and its effect must
    // refuse, and a shortened window must bind (ADR-0014 decision 3's
    // live-resolution doctrine).
    let effective = state
        .pdp
        .effective(tenant_id, Resource::Scope(target.id), &input.context());
    terms
        .validate(&effective.lapse)
        .map_err(|err| Error::Conflict {
            message: format!(
                "the pack in force at {} no longer admits these terms: {err}",
                target.id
            ),
        })?;

    let granted = lapses::grant(
        &mut *tx,
        tenant_id,
        id,
        terms.grantee_scope_id,
        terms.target_scope_id,
        terms.action,
        &terms.reason,
        terms.duration_secs,
        granter,
    )
    .await?;
    close(&mut tx, tenant_id, id, granter).await?;

    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::LapseGranted,
        Resource::Scope(target.id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::LapseGrant, &authorized),
            "lapse_id": granted.id,
            "proposal_id": id,
            "proposal_commit": proposal.commit.to_hex(),
            "lapse": lapse_terms_context(&terms, member.object.to_hex()),
            // The window, recorded here so the trail stays complete even if
            // the expiry sweep never runs: when this grant stopped deciding
            // anything is arithmetic over these two instants.
            "granted_at": granted.granted_at,
            "expires_at": granted.expires_at,
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

    tracing::info!(
        lapse.id = %granted.id,
        scope.grantee = %granted.grantee_scope_id,
        scope.target = %granted.target_scope_id,
        lapse.expires_at = %granted.expires_at,
        "lapse granted"
    );
    Ok(Json(render(
        &granted,
        (None, Some(target.path)),
        Utc::now(),
    )))
}

// ── Revoke ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct RevokeBody {
    /// Why. Mandatory, like the grant's own reason: an ending an auditor
    /// cannot read the reason for is not a governed act.
    reason: String,
}

/// `POST /v1/lapses/{id}/revoke` — end a standing grant early.
///
/// Resolves no approval matrix. A revocation installs nothing and can only
/// narrow, and a product whose answer to "that grant was a mistake" is
/// "convene the two stewards again" has not shipped revocation (ADR-0037
/// decision 15).
#[tracing::instrument(name = "lapses.revoke", skip_all)]
pub(crate) async fn revoke(
    State(state): State<AppState>,
    Path(id): Path<LapseId>,
    payload: std::result::Result<Json<RevokeBody>, JsonRejection>,
) -> Response {
    let result = revoke_inner(&state, id, payload).await;
    respond(&state, "revoke", result).await
}

async fn revoke_inner(
    state: &AppState,
    id: LapseId,
    payload: std::result::Result<Json<RevokeBody>, JsonRejection>,
) -> Result<Json<LapseView>> {
    let request = body(payload)?;
    let reason = request.reason.trim();
    if reason.is_empty() || reason.len() > synveda_types::MAX_LAPSE_REASON {
        return Err(Error::Invalid {
            message: format!(
                "a revocation's reason is mandatory and at most {} characters",
                synveda_types::MAX_LAPSE_REASON
            ),
        });
    }
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let existing = lapses::by_id(&mut *tx, tenant_id, id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("lapse {id}"),
        })?;
    let target = found(
        hierarchy::node(&mut *tx, existing.target_scope_id).await?,
        tenant_id,
        existing.target_scope_id,
    )?;
    let input = authz::gather(state, &mut tx, Some(&target)).await?;
    let authorized = authz::decide(
        state,
        &input,
        Action::LapseRevoke,
        Resource::Scope(target.id),
        None,
    )?;
    let revoker = identity_of(&input)?;
    let revoked = lapses::revoke(&mut *tx, tenant_id, id, revoker, reason).await?;

    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::LapseRevoked,
        Resource::Scope(target.id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::LapseRevoke, &authorized),
            "lapse_id": revoked.id,
            "proposal_id": revoked.proposal_id,
            "grantee_scope_id": revoked.grantee_scope_id,
            "target_scope_id": revoked.target_scope_id,
            "action": revoked.action.as_str(),
            "reason": reason,
            "granted_at": revoked.granted_at,
            // What the revocation cut short — the difference between the
            // window granted and the window served.
            "would_have_expired_at": revoked.expires_at,
            "revoked_at": revoked.revoked_at,
        }),
    )
    .await?;
    commit(tx).await?;

    tracing::info!(lapse.id = %revoked.id, "lapse revoked early");
    Ok(Json(render(
        &revoked,
        (None, Some(target.path)),
        Utc::now(),
    )))
}

// ── Listing ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct ListParams {
    /// The target scope whose grants to list — the scope whose material
    /// they disclose.
    scope_id: ScopeId,
}

/// `GET /v1/lapses?scope_id=` — every grant ever made over a scope.
///
/// Under `PolicyRead` rather than an action of its own: a lapse is policy,
/// and "how is this node governed" should have one place to look (the
/// ADR-0032 decision 15 argument for curator files).
///
/// Expired and revoked rows are included deliberately — "who could read
/// this scope's material in March" is the question the surface exists for,
/// and a listing of only standing grants cannot answer it.
#[tracing::instrument(name = "lapses.list", skip_all)]
pub(crate) async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Response {
    let result = list_inner(&state, params).await;
    respond(&state, "list", result).await
}

async fn list_inner(state: &AppState, params: ListParams) -> Result<Json<serde_json::Value>> {
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let target = found(
        hierarchy::node(&mut *tx, params.scope_id).await?,
        tenant_id,
        params.scope_id,
    )?;
    let input = authz::gather(state, &mut tx, Some(&target)).await?;
    authz::decide(
        state,
        &input,
        Action::PolicyRead,
        Resource::Scope(target.id),
        None,
    )?;
    let rows = lapses::at_target(&mut *tx, tenant_id, target.id).await?;
    let now = Utc::now();
    let views: Vec<LapseView> = rows
        .iter()
        .map(|lapse| render(lapse, (None, Some(target.path.clone())), now))
        .collect();
    Ok(Json(json!({
        "scope_id": target.id,
        "scope_path": target.path,
        "lapses": views,
    })))
}

// ── Shared plumbing ────────────────────────────────────────────────────

/// The terms as every audit payload renders them. Admin text and ids,
/// never any of the material a grant discloses.
fn lapse_terms_context(terms: &LapseTerms, object_hash: String) -> serde_json::Value {
    json!({
        "grantee_scope_id": terms.grantee_scope_id,
        "target_scope_id": terms.target_scope_id,
        "action": terms.action.as_str(),
        "duration_secs": terms.duration_secs,
        "reason": terms.reason,
        "object_hash": object_hash,
    })
}

async fn member_names(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    proposal: &vedaflow::StoredProposal,
) -> Result<Vec<String>> {
    let members = vedaflow::proposals::members(tx, tenant_id, proposal.commit).await?;
    Ok(members.into_iter().map(|member| member.name).collect())
}

/// Closes the proposal as `published` — the stored vocabulary's word for
/// "its effect ran", which for a lapse is a grant rather than a channel
/// move (ADR-0037 decision 16). `ProposalEffect` is what says which.
async fn close(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    id: ProposalId,
    by: IdentityId,
) -> Result<()> {
    vedaflow::proposals::close(tx, tenant_id, id, ProposalState::Published, by, None).await?;
    Ok(())
}

fn identity_of(input: &DecisionInput) -> Result<IdentityId> {
    input
        .identity
        .as_ref()
        .map(|identity| identity.id)
        .ok_or_else(|| Error::PolicyDenied {
            action: "lapse".to_owned(),
            resource: "the lapse plane".to_owned(),
            reason: "a lapse is an act by a provisioned identity; this subject has none".to_owned(),
        })
}

// ── The expiry sweep ───────────────────────────────────────────────────

/// The component name the sweep's audit events are attributed to.
const EXPIRY_COMPONENT: &str = "lapse-expiry";

/// The most grants one pass closes out per tenant, so a tenant with a
/// backlog does not hold a transaction open across all of it. The rest
/// arrive next tick; nothing is waiting on them.
const EXPIRY_BATCH: i64 = 256;

/// One pass over every active tenant, chaining `policy.lapse.expired` for
/// grants whose window has closed.
///
/// **This is bookkeeping and nothing more.** Every grant it touches
/// stopped deciding anything the moment `expires_at` passed — the read
/// path's predicate saw to that — so a pass that never runs costs an audit
/// line, never access. That asymmetry is the whole reason the feature
/// does not schedule its expiries (ADR-0037 decision 4).
///
/// Revoked grants are excluded by the query: their ending is already on the
/// chain as `policy.lapse.revoked`, and a second event asserting the same
/// fact is something an auditor would have to reconcile (ADR-0019
/// decision 4).
#[tracing::instrument(name = "lapses.expiry_sweep", skip_all, err(Display))]
pub async fn expire_once(pool: &sqlx::PgPool) -> Result<usize> {
    let mut chained = 0;
    for tenant in synveda_store::tenants::active(pool).await? {
        match expire_tenant(pool, tenant.id).await {
            Ok(count) => chained += count,
            // One tenant's failure must not strand the rest: its grants
            // are already inert, and the next pass finds them again
            // because nothing stamped them.
            Err(error) => tracing::error!(
                tenant.id = %tenant.id,
                error = %error,
                "lapse expiry pass failed for this tenant; the grants are already \
                 inert and the next pass will chain them"
            ),
        }
    }
    Ok(chained)
}

async fn expire_tenant(pool: &sqlx::PgPool, tenant_id: TenantId) -> Result<usize> {
    // An unlocked look first, so an idle tenant pays one indexed read
    // rather than a transaction and a write — the FLOW-4 lesson, learned
    // on a shared dev database where a pass visits thousands of leftover
    // test tenants.
    let mut tx = rls::begin_tenant_tx(pool, tenant_id).await?;
    let due = lapses::due_for_expiry_event(&mut *tx, tenant_id, EXPIRY_BATCH).await?;
    if due.is_empty() {
        return Ok(0);
    }
    let mut chained = 0;
    for lapse in &due {
        // The stamp is the idempotency key: two overlapping sweeps cannot
        // chain one expiry twice, and the loser simply finds nothing to
        // update rather than writing a duplicate event.
        if !lapses::record_expiry(&mut *tx, tenant_id, lapse.id).await? {
            continue;
        }
        audit::record_as(
            &mut tx,
            tenant_id,
            Actor::system(EXPIRY_COMPONENT),
            AuditAction::LapseExpired,
            Resource::Scope(lapse.target_scope_id).to_string(),
            Outcome::Success,
            json!({
                "lapse_id": lapse.id,
                "proposal_id": lapse.proposal_id,
                "grantee_scope_id": lapse.grantee_scope_id,
                "target_scope_id": lapse.target_scope_id,
                "action": lapse.action.as_str(),
                "reason": lapse.reason,
                "granted_at": lapse.granted_at,
                "expires_at": lapse.expires_at,
                // Said plainly, because an auditor reading this event should
                // not have to know the implementation to know what it means.
                "note": "the window closed; the grant stopped deciding reads at \
                         expires_at, whether or not this event was written",
            }),
        )
        .await?;
        chained += 1;
    }
    tx.commit().await.map_err(|err| Error::Storage {
        message: format!("commit lapse expiry pass: {err}"),
    })?;
    metrics::counter!(LAPSE_EXPIRIES_TOTAL).increment(chained as u64);
    if chained > 0 {
        tracing::info!(tenant.id = %tenant_id, lapses = chained, "lapse expiries chained");
    }
    Ok(chained)
}

/// Spawns the expiry sweep loop — the pack-refresher shape. Abort the
/// handle on shutdown.
#[must_use]
pub fn spawn_expiry_sweep(
    pool: sqlx::PgPool,
    interval: std::time::Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(error) = expire_once(&pool).await {
                tracing::warn!(error = %error, "lapse expiry sweep failed");
            }
        }
    })
}
