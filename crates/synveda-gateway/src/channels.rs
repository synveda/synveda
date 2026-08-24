//! The VedaFlow channel API (FLOW-2, ADR-0031 decision 12):
//! `/v1/channels/{scope_id}` behind tenant resolution, uniform-404
//! ownership, and the PDP (`ChannelRead` to see a scope's authored-artifact
//! channels, `ChannelPublish` to move an immutable artifact version onto one).
//!
//! Publishing is the act that crosses the trust boundary. It is therefore a curator's action by name
//! (seed §5), same-scope (climbing is FLOW-5's, with the higher scope's
//! approvers), and additive — retraction is a rewind, and rewinds are
//! FLOW-7's by name.
//!
//! What lands is bound to *bytes*, not a mutable name: each artifact's
//! content address is stored in the channel tree (ADR-0031 decision 5).
//!
//! Since FLOW-7 (ADR-0036) the same plane holds the two acts that move a
//! channel the other way: `ChannelRollback` rewinds it to a state it has
//! already held, and `ChannelPin` holds what it *serves* at a commit
//! without moving where it points. Neither resolves the approval matrix —
//! a rewind can install nothing the matrix has not already cleared — and
//! both take the asset kind's read action alongside their own, on the same
//! rule publishing follows: nobody governs material they cannot read.
//!
//! Since FLOW-3 (ADR-0032 decision 8) this route resolves the **same**
//! approval matrix a proposal does, with the acting principal counting as
//! the only approver. A multi-person requirement refuses and names the
//! proposal route. That is what keeps one matrix rather than two paths:
//! the direct route did not become a hole to close, it became the
//! degenerate case where one approval is enough.

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
use synveda_types::{
    AssetKind, CastApproval, Channel, Error, IdentityId, Result, ScopeId, Sensitivity,
};
use synveda_vedaflow::{self as vedaflow, ChannelRef, PolicySnapshot, Signer};

use crate::app::AppState;
use crate::approvals;
use crate::audit;
use crate::authz;
use crate::error::ApiError;
use crate::request::{body, commit, found, tenant_id};
use crate::telemetry::CHANNEL_OPERATIONS_TOTAL;

/// Members per publish. Well below `MAX_CHANNEL_MEMBERS` on purpose: a
/// publish is a reviewed act, and a thousand members in one call is a
/// migration wearing a curator's hat.
const MAX_PUBLISH_MEMBERS: usize = 200;

/// The commit-message cap; mirrors `vedaflow_commits`' CHECK.
const MAX_MESSAGE_CHARS: usize = 4096;

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
    metrics::counter!(CHANNEL_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    match result {
        Ok(response) => response.into_response(),
        Err(error) => {
            audit::record_rejection(state, op, &error).await;
            ApiError(error).into_response()
        }
    }
}

/// One standing channel as the API renders it.
#[derive(Serialize)]
struct ChannelView {
    /// The ref name, e.g. `skill/published`.
    name: String,
    asset: String,
    channel: Channel,
    /// Where the channel points — what an authorised reader cites.
    commit: String,
    /// Entries in that commit's tree: the membership for `published` and
    /// `staged`, the last commit's additions for `derived` (which is a
    /// log, not a set — ADR-0031 decision 3).
    entries: usize,
    updated_at: DateTime<Utc>,
    updated_by: IdentityId,
    /// The standing pin, when there is one: readers compose this commit
    /// rather than the one above until it is released (FLOW-7, ADR-0036
    /// decision 6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pin: Option<PinView>,
}

/// A pin as the API renders it.
#[derive(Serialize)]
struct PinView {
    /// The commit readers are held at.
    commit: String,
    pinned_at: DateTime<Utc>,
    pinned_by: IdentityId,
}

impl PinView {
    fn of(pin: &vedaflow::ChannelPin) -> Self {
        PinView {
            commit: pin.commit.to_hex(),
            pinned_at: pin.pinned_at,
            pinned_by: pin.pinned_by,
        }
    }
}

#[derive(Serialize)]
struct ChannelsResponse {
    scope_id: ScopeId,
    channels: Vec<ChannelView>,
}

/// `GET /v1/channels/{scope_id}` — the channels standing at one scope.
///
/// A scope with no channels answers 200 with an empty list: refs
/// materialise on first write (ADR-0031 decision 2), so "nothing has been
/// committed here" is the answer, not a 404.
#[tracing::instrument(name = "channels.list", skip_all)]
pub(crate) async fn list(State(state): State<AppState>, Path(scope_id): Path<ScopeId>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let node = found(
            scopes::get(&mut *tx, tenant_id, scope_id).await?,
            tenant_id,
            scope_id,
        )?;
        let authorized = authz::require(
            &state,
            &mut tx,
            Action::ChannelRead,
            Resource::Scope(scope_id),
            Some(&node),
        )
        .await?;
        let statuses: Vec<_> = vedaflow::channels::status(&mut tx, tenant_id, scope_id)
            .await?
            .into_iter()
            .filter(|status| {
                matches!(
                    status.channel.asset,
                    AssetKind::Prompt | AssetKind::ContextPack | AssetKind::Skill
                )
            })
            .collect();
        // An allowed admin-plane read chains its decision (ADR-0019
        // decision 4).
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::AuthzDecision,
            Resource::Scope(scope_id).to_string(),
            Outcome::Allow,
            json!({
                "op": "list",
                "authz": audit::decision_context(Action::ChannelRead, &authorized),
                "channels": statuses.len(),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(ChannelsResponse {
            scope_id,
            channels: statuses
                .into_iter()
                .map(|status| ChannelView {
                    name: status.channel.name(),
                    asset: status.channel.asset.as_str().to_owned(),
                    channel: status.channel.channel,
                    commit: status.commit.to_hex(),
                    entries: status.entries,
                    updated_at: status.updated_at,
                    updated_by: status.updated_by,
                    pin: status.pin.as_ref().map(PinView::of),
                })
                .collect(),
        }))
    }
    .await;
    respond(&state, "list", result).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PublishBody {
    /// The prompts to admit, by name (PRMT-1, ADR-0049 decision 7). Must be
    /// drafts of **this** scope: the direct route stays same-scope.
    ///
    /// Exactly one member list may be present. Under the default pack
    /// a prompt publication refuses here on its own arithmetic — the matrix
    /// asks for a steward *and* a curator, two distinct people — and names
    /// the proposal route; under `standard` a single curator may publish,
    /// which is that pack saying what that pack exists to say. That is
    /// ADR-0032 decision 8's invariant kept rather than a second rule for
    /// authored assets.
    #[serde(default)]
    prompt_names: Vec<synveda_types::PromptName>,
    /// The context-pack documents to admit, by path (PRMT-2, ADR-0050
    /// decision 1). Must be documents of **this** scope, for the reason
    /// the other lists must be its material: the direct route stays
    /// same-scope.
    ///
    /// Exactly one of the three lists may be present. Under the default
    /// pack a pack publication above a team now refuses here on its own
    /// arithmetic — since ADR-0050 decision 15 the matrix asks for a
    /// curator *and* a steward, two distinct people — and names the
    /// proposal route; at a team or a leaf one curator still publishes
    /// directly, which is the governed `SHARED`/`LOCAL` split.
    #[serde(default)]
    document_paths: Vec<synveda_types::DocumentPath>,
    /// The skills to admit, by name (SKIL-1, ADR-0051 decision 1). Must be
    /// drafts of **this** scope, for the reason the other lists must
    /// be its material: the direct route stays same-scope.
    ///
    /// A skill names the *bundle*, never a file: a client loads a skill
    /// whole, so publishing three of its four files would publish a version
    /// nobody can run. Every file the draft holds becomes a member.
    ///
    /// Exactly one list may be present. Under **every** pack a
    /// skill publication refuses here on its own arithmetic — the invariant
    /// floor asks for a security reviewer and, since ADR-0051 decision 18,
    /// two distinct approvers — and names the proposal route. That
    /// uniformity is the difference from the other three: a skill is
    /// executable, and no pack makes shipping code a one-signature act.
    #[serde(default)]
    skill_names: Vec<synveda_types::SkillName>,
    /// Why — an auditor and a reviewer both read this. Required: a
    /// publication with nothing to say is one nobody can review after
    /// the fact.
    message: String,
}

#[derive(Serialize)]
struct PublishResponse {
    scope_id: ScopeId,
    /// The channel that moved.
    channel: String,
    /// The commit it now points at.
    commit: String,
    /// What it pointed at before — absent on the channel's first commit.
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<String>,
    /// The published set's size after this call.
    members: usize,
    /// Members this call admitted that the channel did not already hold
    /// at that address. Zero means everything named was already
    /// published, unchanged — the act still commits and still audits.
    added: usize,
    /// Each member's content address, in request order.
    published: Vec<PublishedMember>,
    /// What the approval matrix asked for here, and which of the acting
    /// principal's roles supplied it (FLOW-3, ADR-0032 decision 8).
    /// A publication that needed nothing renders an empty requirement,
    /// which is the honest answer: this pack asks for no review at this
    /// cell.
    required: crate::approvals::RequirementView,
    /// The standing pin, when this scope has one. A publication onto a
    /// pinned channel lands and the ref advances — what does not change is
    /// what readers compose, and a curator who published and saw no effect
    /// has to be told why rather than left to discover it (ADR-0036
    /// decision 6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pinned: Option<PinView>,
}

#[derive(Serialize)]
struct PublishedMember {
    /// The tree entry name or authored path.
    member: String,
    object_hash: String,
}

/// `POST /v1/channels/{scope_id}/publish` — admit authored artifact versions.
#[tracing::instrument(name = "channels.publish", skip_all)]
pub(crate) async fn publish(
    State(state): State<AppState>,
    Path(scope_id): Path<ScopeId>,
    payload: std::result::Result<Json<PublishBody>, JsonRejection>,
) -> Response {
    let result = publish_inner(&state, scope_id, payload).await;
    respond(&state, "publish", result).await
}

async fn publish_inner(
    state: &AppState,
    scope_id: ScopeId,
    payload: std::result::Result<Json<PublishBody>, JsonRejection>,
) -> Result<Json<PublishResponse>> {
    let body = body(payload)?;
    validate(&body)?;
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let node = found(
        scopes::get(&mut *tx, tenant_id, scope_id).await?,
        tenant_id,
        scope_id,
    )?;
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
        Action::ChannelPublish,
        Resource::Scope(scope_id),
    )?;
    // The commit's author is the curator who published, which is what
    // makes blame and lineage run through the history (tech plan §2.5).
    // A verified subject with no identity row cannot reach here — the
    // packs require a role binding, and bindings are resolved per
    // subject — but the check is explicit rather than an unwrap.
    let author = input
        .identity
        .as_ref()
        .map(|identity| identity.id)
        .ok_or_else(|| Error::Invalid {
            message: "publishing requires a provisioned identity".to_owned(),
        })?;

    // What is being published, per asset kind (PRMT-1, ADR-0049
    // decision 7). Both readers answer the same question — the current
    // versions this scope holds, refusing the whole request rather than a
    // subset — and both stop there: everything below this point is shared,
    // because one matrix governs every path across the trust boundary
    // (ADR-0032 decision 8).
    let asset_kind = if !body.prompt_names.is_empty() {
        AssetKind::Prompt
    } else if !body.skill_names.is_empty() {
        AssetKind::Skill
    } else {
        AssetKind::ContextPack
    };
    let mut paths = body.document_paths.clone();
    paths.sort();
    paths.dedup();
    let documents: Vec<synveda_store::packs::StoredDocument> = if paths.is_empty() {
        Vec::new()
    } else {
        let held: Vec<synveda_store::packs::StoredDocument> =
            synveda_store::packs::list_all_documents(&mut *tx, tenant_id, scope_id)
                .await?
                .into_iter()
                .filter(|document| {
                    paths.iter().any(|path| {
                        path.pack == document.pack_name && path.document == document.document_name
                    })
                })
                .collect();
        if held.len() != paths.len() {
            let missing: Vec<String> = paths
                .iter()
                .filter(|path| {
                    !held.iter().any(|document| {
                        path.pack == document.pack_name && path.document == document.document_name
                    })
                })
                .map(ToString::to_string)
                .collect();
            return Err(Error::Invalid {
                message: format!(
                    "not documents of this scope: {} — promote from a child scope with \
                     POST /v1/proposals and a source_scope_id (FLOW-5)",
                    missing.join(", ")
                ),
            });
        }
        held
    };
    let mut names = body.prompt_names.clone();
    names.sort();
    names.dedup();
    let drafts = synveda_store::prompts::read_many(&mut *tx, tenant_id, scope_id, &names).await?;
    if drafts.len() != names.len() {
        let missing: Vec<String> = names
            .iter()
            .filter(|name| !drafts.iter().any(|draft| &&draft.template.name == name))
            .map(ToString::to_string)
            .collect();
        return Err(Error::Invalid {
            message: format!(
                "not drafts of this scope: {} — promote from a child scope with \
                 POST /v1/proposals and a source_scope_id (FLOW-5)",
                missing.join(", ")
            ),
        });
    }
    // A skill names the bundle, so this reads every file of it: publishing
    // a subset would publish a version no client can run (ADR-0051
    // decision 17's rule, one surface over).
    let mut skill_names = body.skill_names.clone();
    skill_names.sort();
    skill_names.dedup();
    let mut skill_drafts: Vec<(
        synveda_store::skills::StoredSkill,
        Vec<synveda_store::skills::StoredFile>,
    )> = Vec::with_capacity(skill_names.len());
    for name in &skill_names {
        let Some(skill) = synveda_store::skills::skill(&mut *tx, tenant_id, scope_id, name).await?
        else {
            return Err(Error::Invalid {
                message: format!(
                    "not a skill draft of this scope: {name} — promote from a child scope \
                     with POST /v1/proposals and a source_scope_id (FLOW-5)"
                ),
            });
        };
        let files = synveda_store::skills::files_of(&mut *tx, tenant_id, scope_id, name).await?;
        if files.is_empty() {
            return Err(Error::Invalid {
                message: format!("skill {name} holds no files; there is nothing to publish"),
            });
        }
        skill_drafts.push((skill, files));
    }

    // The approval matrix, resolved at this scope from this pack, this
    // asset kind, the *maximum* sensitivity over the set (a set is
    // reviewed as a set), and the nearest curator file on the chain —
    // then satisfied by the acting principal alone or refused with the
    // proposal route named (ADR-0032 decision 8). Same resolution
    // function a proposal uses; there is one matrix, not two.
    let sensitivity = drafts
        .iter()
        .map(|draft| draft.sensitivity)
        .chain(documents.iter().map(|document| document.sensitivity))
        .chain(skill_drafts.iter().map(|(skill, _)| skill.sensitivity))
        .max()
        .unwrap_or(Sensitivity::Public);
    // The second decision (ADR-0031 decision 12): may this principal *read*
    // what it is about to declare reviewed. Publication is same-scope, so it
    // is the same scope. Nobody publishes material they cannot read.
    //
    // At the working tier, deliberately (AUTHZ-5, ADR-0038 decision 10):
    // this guard asks *whose* material it is, not how sensitive it is.
    // Publishing discloses nothing to the publisher that they did not
    // already hold, and what prices the tier is the approval matrix
    // immediately below — resolved at the set's maximum, where `restricted`
    // means compliance and two distinct approvers. Asking it at the set's
    // tier instead would make `restricted` material unpublishable by
    // anyone, which would leave the invariant floor's own cell unreachable
    // and a restricted lapse with nothing to disclose.
    decide_asset_read(state, &input, asset_kind, scope_id)?;
    let entries: Vec<String> = drafts
        .iter()
        .map(|draft| draft.template.name.to_string())
        .chain(documents.iter().map(|document| {
            synveda_types::DocumentPath::new(
                document.pack_name.clone(),
                document.document_name.clone(),
            )
            .to_string()
        }))
        .chain(skill_drafts.iter().map(|(skill, _)| skill.name.to_string()))
        .collect();
    let requirement = approvals::resolve(
        state,
        &mut tx,
        tenant_id,
        &input,
        &approvals::Requested {
            target: &node,
            asset: asset_kind,
            sensitivity,
            entries: &entries,
        },
    )
    .await?;
    let actor = CastApproval {
        identity: author,
        subject: input.principal.subject.clone(),
        roles: approvals::roles_at(&input, &node),
    };
    approvals::require_single_actor(&requirement, &actor, "channel")?;

    // Objects first: each member's content address, computed from the
    // version being published, then stored. Content-addressed, so
    // re-publishing unchanged content stores nothing new — and a prompt's
    // object was already written at authoring time, so that write dedups.
    let mut members: Vec<(String, vedaflow::hash::ObjectHash)> =
        Vec::with_capacity(drafts.len() + documents.len() + skill_drafts.len());
    let mut published: Vec<PublishedMember> = Vec::with_capacity(members.capacity());
    for draft in &drafts {
        let asset = vedaflow::PromptAsset {
            scope_id,
            sensitivity: draft.sensitivity,
            template: draft.template.clone(),
        };
        let object = vedaflow::put_prompt(&mut tx, tenant_id, &asset).await?;
        members.push((asset.entry_name(), object.hash));
        published.push(PublishedMember {
            member: asset.entry_name(),
            object_hash: object.hash.to_hex(),
        });
    }
    // A pack document's object is already stored — the draft row's foreign
    // key required it at authoring — so this read is the *object*, not a
    // rebuild from the row. Rebuilding would re-derive the bytes a
    // reviewer would have read, and the address is what the channel names
    // (ADR-0050 decision 3).
    for document in &documents {
        let address = vedaflow::hash::ObjectHash::from_bytes(document.object_hash);
        let path = synveda_types::DocumentPath::new(
            document.pack_name.clone(),
            document.document_name.clone(),
        );
        members.push((path.to_string(), address));
        published.push(PublishedMember {
            member: path.to_string(),
            object_hash: address.to_hex(),
        });
    }
    // A skill file's object is already stored, for the pack document's
    // reason — and every file of the bundle goes, because a client loads it
    // whole.
    for (skill, files) in &skill_drafts {
        for file in files {
            let address = vedaflow::hash::ObjectHash::from_bytes(file.object_hash);
            let path = synveda_types::SkillPath::new(skill.name.clone(), file.path.clone());
            members.push((path.to_string(), address));
            published.push(PublishedMember {
                member: path.to_string(),
                object_hash: address.to_hex(),
            });
        }
    }

    let channel = ChannelRef::new(asset_kind, Channel::Published);
    let snapshot = PolicySnapshot::new(
        authorized.decision.pack_name.clone(),
        authorized.decision.pack_version,
    );
    let committed = vedaflow::publish(
        &mut tx,
        tenant_id,
        &vedaflow::ChannelWrite {
            scope: scope_id,
            channel,
            members: &members,
            merge_parents: &[],
            author,
            message: &body.message,
            committed_at: Utc::now(),
            policy_snapshot: &snapshot,
        },
        &Signer::Unsigned,
    )
    .await?;

    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::ChannelPublished,
        Resource::Scope(scope_id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::ChannelPublish, &authorized),
            "channel": channel.name(),
            "asset": channel.asset.as_str(),
            "message": body.message,
            // Names and addresses, never content — the text stays in its own
            // table, and the address is what an auditor rechecks.
            "published": published.iter().map(|member| json!({
                "member": member.member,
                "object_hash": member.object_hash,
            })).collect::<Vec<_>>(),
            "commit": committed.commit.to_hex(),
            "parent": committed.parent.map(|parent| parent.to_hex()),
            "members": committed.entries,
            "added": committed.added,
            "sensitivity": sensitivity.as_str(),
            // The requirement *as resolved at this moment*, and the roles
            // the acting principal supplied it with — so an auditor can
            // reconstruct why one signature was enough without reading a
            // pack that has since changed (ADR-0032 decision 18).
            "approvals": approvals::audit_context(
                &requirement,
                &requirement.outstanding(std::slice::from_ref(&actor)),
            ),
            "approved_by": [{
                "identity_id": actor.identity,
                "subject": actor.subject,
                "roles": actor.roles.iter().map(|role| role.as_str()).collect::<Vec<_>>(),
            }],
        }),
    )
    .await?;
    let pinned = vedaflow::read_pin(&mut tx, tenant_id, scope_id, channel).await?;
    commit(tx).await?;

    Ok(Json(PublishResponse {
        scope_id,
        channel: channel.name(),
        commit: committed.commit.to_hex(),
        parent: committed.parent.map(|parent| parent.to_hex()),
        members: committed.entries,
        added: committed.added,
        published,
        required: crate::approvals::RequirementView::of(&requirement),
        pinned: pinned.as_ref().map(PinView::of),
    }))
}

// ── Rollback & pinning (FLOW-7, ADR-0036) ────────────────────────────────
//
// Three surfaces and one rule between them: the history is the set a
// rewind may name, a rewind installs a state the channel has already held,
// and a pin holds what the channel *serves* without moving where it
// points. None of the three resolves the approval matrix, because a rewind
// cannot install content the matrix has not already cleared (ADR-0036
// decisions 1–3) and a pin can only hold membership at an earlier such
// state.

/// The most states `GET /history` returns in one call.
const MAX_HISTORY: u32 = 200;

/// Its default, when the caller does not ask.
const DEFAULT_HISTORY: u32 = 20;

/// Which authored-artifact channel a FLOW-7 call is about. The asset is
/// required; `published` remains the channel default.
///
/// The routes are asset-kind generic on purpose: PRMT-1's prompts and
/// SKIL-1's skill bundles land on channels of the same shape, and a
/// rewind of one is this same act. Every admitted family has its own read
/// action; all other asset kinds are refused by name.
///
/// Spelled out on each request type rather than flattened into them: a
/// flattened struct deserialises differently from a query string than
/// from a body, and two fields are cheaper than that surprise.
fn channel_of(asset: AssetKind, channel: Option<Channel>) -> Result<ChannelRef> {
    if !matches!(
        asset,
        AssetKind::Prompt | AssetKind::ContextPack | AssetKind::Skill
    ) {
        return Err(Error::Invalid {
            message: format!(
                "{} is not a public authored-artifact channel",
                asset.as_str()
            ),
        });
    }
    Ok(ChannelRef::new(
        asset,
        channel.unwrap_or(Channel::Published),
    ))
}

/// Decides the asset kind's own read action at the scope, at the working
/// tier.
///
/// A rewind and a pin both take it in addition to their own action, on
/// ADR-0031 decision 12's rule: nobody governs material they cannot read.
/// That is what keeps a curator out of a teammate's personal published
/// channel, through the privacy floor, with no clause about personal
/// scopes anywhere here.
///
/// The working tier, deliberately: moving a ref discloses no content to the
/// actor (the response carries ids and addresses, never text), so this is a
/// *whose-material* question, which the privacy floor answers identically at
/// every tier (ADR-0038 decision 10).
///
/// PRMT-1 (ADR-0049 decision 4) supplied the second answer here, PRMT-2 the
/// third and SKIL-1 (ADR-0051 decision 10) the fourth — which **closes**
/// ADR-0036 decision 3's deferral: every asset kind that has a channel now
/// has a read action, and the refusal below survives only for `policy`,
/// which has no channel at all (a lapse writes a row, ADR-0037
/// decision 16).
fn decide_asset_read(
    state: &AppState,
    input: &crate::authz::DecisionInput,
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
        AssetKind::Skill => authz::decide_skill_read(state, input, resource, Sensitivity::WORKING),
        // `policy` is the one that remains, and it has no channel at all —
        // a lapse writes a row (ADR-0037 decision 16). ADR-0036 decision 3's
        // refusal-by-name now reaches no asset kind that has one.
        other => Err(Error::Invalid {
            message: format!(
                "{} has no channel, so there is nothing here to rewind or pin",
                other.as_str()
            ),
        }),
    }
}

/// One state a channel has held, as the API renders it.
#[derive(Serialize)]
struct HistoryEntryView {
    commit: String,
    /// The state it replaced — its first parent, absent on the channel's
    /// first commit.
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<String>,
    /// Parents beyond the first: the proposal this publication was the
    /// effect of, when it had one. Present so a reviewer can trace the
    /// decision — and deliberately *not* a rewind target, because a
    /// proposal's tree is a member set that may never have been approved
    /// (ADR-0036 decision 1).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    merge_parents: Vec<String>,
    author: IdentityId,
    message: String,
    committed_at: DateTime<Utc>,
    /// The membership this state served.
    members: usize,
    /// True for the commit the channel points at now — where it already
    /// is, and so the one entry a rewind cannot name.
    head: bool,
    /// True for the commit a pin holds readers at.
    served: bool,
}

#[derive(Serialize)]
struct HistoryResponse {
    scope_id: ScopeId,
    channel: String,
    /// The commit the ref points at.
    head: String,
    /// The standing pin, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pin: Option<PinView>,
    /// Newest first. Every entry but `head` is a legal rewind target, and
    /// nothing outside this listing is (ADR-0036 decision 11).
    history: Vec<HistoryEntryView>,
}

/// `GET /v1/channels/{scope_id}/history` — the states this channel has
/// held, newest first.
#[tracing::instrument(name = "channels.history", skip_all)]
pub(crate) async fn history(
    State(state): State<AppState>,
    Path(scope_id): Path<ScopeId>,
    Query(query): Query<HistoryQuery>,
) -> Response {
    let result = history_inner(&state, scope_id, query).await;
    respond(&state, "history", result).await
}

#[derive(Deserialize)]
pub(crate) struct HistoryQuery {
    asset: AssetKind,
    channel: Option<Channel>,
    limit: Option<u32>,
}

async fn history_inner(
    state: &AppState,
    scope_id: ScopeId,
    query: HistoryQuery,
) -> Result<Json<HistoryResponse>> {
    let channel = channel_of(query.asset, query.channel)?;
    let limit = query.limit.unwrap_or(DEFAULT_HISTORY).clamp(1, MAX_HISTORY);
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let node = found(
        scopes::get(&mut *tx, tenant_id, scope_id).await?,
        tenant_id,
        scope_id,
    )?;
    // Reading a channel's history is reading the channel: same action, and
    // the entries carry no artifact content — names, addresses, and the
    // curators' own commit messages.
    let authorized = authz::require(
        &state.clone(),
        &mut tx,
        Action::ChannelRead,
        Resource::Scope(scope_id),
        Some(&node),
    )
    .await?;

    let head = vedaflow::read_ref(&mut tx, tenant_id, scope_id, &channel.name())
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("{channel} channel at scope {scope_id}"),
        })?;
    let pin = vedaflow::read_pin(&mut tx, tenant_id, scope_id, channel).await?;
    let served = pin.as_ref().map_or(head.commit_hash, |pin| pin.commit);
    let entries = vedaflow::history(&mut tx, tenant_id, scope_id, channel, limit).await?;

    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::AuthzDecision,
        Resource::Scope(scope_id).to_string(),
        Outcome::Allow,
        json!({
            "op": "history",
            "authz": audit::decision_context(Action::ChannelRead, &authorized),
            "channel": channel.name(),
            "entries": entries.len(),
        }),
    )
    .await?;
    commit(tx).await?;

    Ok(Json(HistoryResponse {
        scope_id,
        channel: channel.name(),
        head: head.commit_hash.to_hex(),
        pin: pin.as_ref().map(PinView::of),
        history: entries
            .into_iter()
            .map(|entry| HistoryEntryView {
                head: entry.commit == head.commit_hash,
                served: entry.commit == served,
                commit: entry.commit.to_hex(),
                parent: entry.parent.map(|parent| parent.to_hex()),
                merge_parents: entry
                    .merge_parents
                    .iter()
                    .map(|parent| parent.to_hex())
                    .collect(),
                author: entry.author,
                message: entry.message,
                committed_at: entry.committed_at,
                members: entry.members,
            })
            .collect(),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RollbackBody {
    asset: AssetKind,
    channel: Option<Channel>,
    /// The commit being abandoned — what the caller read before deciding.
    /// Required rather than inferred: a rewind is a decision about *which*
    /// state to leave, and that decision is stale if someone else moved
    /// the ref meanwhile (ADR-0030 decision 10's rule, applied to the one
    /// call that can move a ref backwards).
    from_commit: String,
    /// The state to install: one of the entries `GET /history` lists.
    to_commit: String,
    /// Why. An auditor reads this, and so does whoever asks next week why
    /// an artifact stopped being published.
    message: String,
}

#[derive(Serialize)]
struct RollbackResponse {
    scope_id: ScopeId,
    channel: String,
    /// The commit abandoned.
    from: String,
    /// The commit installed — what the next authorised reader sees.
    to: String,
    /// The membership after the rewind.
    members: usize,
    /// Member names that stopped being published.
    removed: Vec<String>,
    /// Members whose published version went back to an earlier one, with
    /// the address now bound.
    restored: Vec<PublishedMember>,
}

/// `POST /v1/channels/{scope_id}/rollback` — rewind the channel to a state
/// it has already held.
#[tracing::instrument(name = "channels.rollback", skip_all)]
pub(crate) async fn rollback(
    State(state): State<AppState>,
    Path(scope_id): Path<ScopeId>,
    payload: std::result::Result<Json<RollbackBody>, JsonRejection>,
) -> Response {
    let result = rollback_inner(&state, scope_id, payload).await;
    respond(&state, "rollback", result).await
}

async fn rollback_inner(
    state: &AppState,
    scope_id: ScopeId,
    payload: std::result::Result<Json<RollbackBody>, JsonRejection>,
) -> Result<Json<RollbackResponse>> {
    let body = body(payload)?;
    validate_message(&body.message)?;
    let channel = channel_of(body.asset, body.channel)?;
    let from: vedaflow::CommitHash = body.from_commit.parse()?;
    let to: vedaflow::CommitHash = body.to_commit.parse()?;

    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let node = found(
        scopes::get(&mut *tx, tenant_id, scope_id).await?,
        tenant_id,
        scope_id,
    )?;
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
        Action::ChannelRollback,
        Resource::Scope(scope_id),
    )?;
    // The second decision, as for publishing: nobody governs material they
    // cannot read (ADR-0031 decision 12, ADR-0036 decision 3).
    // Decided with the *asset kind's* own read action (ADR-0049
    // decision 4), at the working tier: moving a ref discloses no content
    // to the actor, so this guard is a *whose-material* question, which the
    // privacy floor answers identically at every tier (ADR-0038
    // decision 10). Asset kinds with no read action are refused by name.
    decide_asset_read(state, &input, channel.asset, scope_id)?;
    let author = acting_identity(&input, "rewinding")?;

    let rolled_back = vedaflow::rollback(
        &mut tx,
        tenant_id,
        &vedaflow::ChannelRewind {
            scope: scope_id,
            channel,
            from,
            to,
            by: author,
        },
    )
    .await?;

    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::ChannelRolledBack,
        Resource::Scope(scope_id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::ChannelRollback, &authorized),
            "channel": channel.name(),
            "asset": channel.asset.as_str(),
            "message": body.message,
            "from": rolled_back.from.to_hex(),
            "to": rolled_back.to.to_hex(),
            "members": rolled_back.entries,
            // Ids and addresses, never content — the same rule the
            // publication event follows.
            "removed": rolled_back.removed,
            "restored": rolled_back.restored.iter().map(|member| json!({
                "name": member.name,
                "object_hash": member.object.to_hex(),
            })).collect::<Vec<_>>(),
        }),
    )
    .await?;
    commit(tx).await?;

    Ok(Json(RollbackResponse {
        scope_id,
        channel: channel.name(),
        from: rolled_back.from.to_hex(),
        to: rolled_back.to.to_hex(),
        members: rolled_back.entries,
        removed: rolled_back.removed,
        restored: rolled_back
            .restored
            .into_iter()
            .map(|member| PublishedMember {
                member: member.name,
                object_hash: member.object.to_hex(),
            })
            .collect(),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PinBody {
    asset: AssetKind,
    channel: Option<Channel>,
    /// The commit to hold readers at: one of the entries `GET /history`
    /// lists, the head included.
    commit: String,
    /// Why this scope is holding its readers. The pin's only record — the
    /// ref carries who and when and nothing else (ADR-0036 decision 9).
    reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UnpinBody {
    asset: AssetKind,
    channel: Option<Channel>,
    /// Why the hold is being released.
    reason: String,
}

#[derive(Serialize)]
struct PinResponse {
    scope_id: ScopeId,
    channel: String,
    /// The commit readers now compose.
    commit: String,
    /// What the pin held before, when this call moved a standing one.
    #[serde(skip_serializing_if = "Option::is_none")]
    previous: Option<String>,
    /// Where the channel's ref points. Publications keep landing here
    /// while the pin stands (ADR-0036 decision 6).
    head: String,
}

#[derive(Serialize)]
struct UnpinResponse {
    scope_id: ScopeId,
    channel: String,
    /// The commit that was held, when there was a pin. Absent means there
    /// was none, which is the answer rather than an error: the channel
    /// serves its head either way.
    #[serde(skip_serializing_if = "Option::is_none")]
    released: Option<String>,
    /// What readers compose from the next session on.
    head: String,
}

/// `POST /v1/channels/{scope_id}/pin` — hold what this channel serves at a
/// commit.
#[tracing::instrument(name = "channels.pin", skip_all)]
pub(crate) async fn pin(
    State(state): State<AppState>,
    Path(scope_id): Path<ScopeId>,
    payload: std::result::Result<Json<PinBody>, JsonRejection>,
) -> Response {
    let result = pin_inner(&state, scope_id, payload).await;
    respond(&state, "pin", result).await
}

async fn pin_inner(
    state: &AppState,
    scope_id: ScopeId,
    payload: std::result::Result<Json<PinBody>, JsonRejection>,
) -> Result<Json<PinResponse>> {
    let body = body(payload)?;
    validate_message(&body.reason)?;
    let channel = channel_of(body.asset, body.channel)?;
    let commit_hash: vedaflow::CommitHash = body.commit.parse()?;

    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let node = found(
        scopes::get(&mut *tx, tenant_id, scope_id).await?,
        tenant_id,
        scope_id,
    )?;
    let input = authz::gather(
        state,
        &mut tx,
        Some(&node),
        synveda_store::anchors::AnchorSelection::none(),
        Vec::new(),
    )
    .await?;
    let authorized = authz::decide(state, &input, Action::ChannelPin, Resource::Scope(scope_id))?;
    // Decided with the *asset kind's* own read action (ADR-0049
    // decision 4), at the working tier: moving a ref discloses no content
    // to the actor, so this guard is a *whose-material* question, which the
    // privacy floor answers identically at every tier (ADR-0038
    // decision 10). Asset kinds with no read action are refused by name.
    decide_asset_read(state, &input, channel.asset, scope_id)?;
    let author = acting_identity(&input, "pinning")?;

    let previous =
        vedaflow::pin(&mut tx, tenant_id, scope_id, channel, commit_hash, author).await?;
    let head = vedaflow::read_ref(&mut tx, tenant_id, scope_id, &channel.name())
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("{channel} channel at scope {scope_id}"),
        })?;

    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::ChannelPinned,
        Resource::Scope(scope_id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::ChannelPin, &authorized),
            "channel": channel.name(),
            "asset": channel.asset.as_str(),
            "reason": body.reason,
            "commit": commit_hash.to_hex(),
            "previous": previous.as_ref().map(|pin| pin.commit.to_hex()),
            "head": head.commit_hash.to_hex(),
        }),
    )
    .await?;
    commit(tx).await?;

    Ok(Json(PinResponse {
        scope_id,
        channel: channel.name(),
        commit: commit_hash.to_hex(),
        previous: previous.map(|pin| pin.commit.to_hex()),
        head: head.commit_hash.to_hex(),
    }))
}

/// `POST /v1/channels/{scope_id}/unpin` — release the hold.
#[tracing::instrument(name = "channels.unpin", skip_all)]
pub(crate) async fn unpin(
    State(state): State<AppState>,
    Path(scope_id): Path<ScopeId>,
    payload: std::result::Result<Json<UnpinBody>, JsonRejection>,
) -> Response {
    let result = unpin_inner(&state, scope_id, payload).await;
    respond(&state, "unpin", result).await
}

async fn unpin_inner(
    state: &AppState,
    scope_id: ScopeId,
    payload: std::result::Result<Json<UnpinBody>, JsonRejection>,
) -> Result<Json<UnpinResponse>> {
    let body = body(payload)?;
    validate_message(&body.reason)?;
    let channel = channel_of(body.asset, body.channel)?;

    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let node = found(
        scopes::get(&mut *tx, tenant_id, scope_id).await?,
        tenant_id,
        scope_id,
    )?;
    let input = authz::gather(
        state,
        &mut tx,
        Some(&node),
        synveda_store::anchors::AnchorSelection::none(),
        Vec::new(),
    )
    .await?;
    let authorized = authz::decide(state, &input, Action::ChannelPin, Resource::Scope(scope_id))?;
    // Decided with the *asset kind's* own read action (ADR-0049
    // decision 4), at the working tier: moving a ref discloses no content
    // to the actor, so this guard is a *whose-material* question, which the
    // privacy floor answers identically at every tier (ADR-0038
    // decision 10). Asset kinds with no read action are refused by name.
    decide_asset_read(state, &input, channel.asset, scope_id)?;

    let released = vedaflow::unpin(&mut tx, tenant_id, scope_id, channel).await?;
    let head = vedaflow::read_ref(&mut tx, tenant_id, scope_id, &channel.name())
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("{channel} channel at scope {scope_id}"),
        })?;

    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::ChannelUnpinned,
        Resource::Scope(scope_id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::ChannelPin, &authorized),
            "channel": channel.name(),
            "asset": channel.asset.as_str(),
            "reason": body.reason,
            // Absent means there was no pin. The act still audits: an
            // operator asserting "nothing holds this channel" is a fact an
            // auditor should be able to see someone established.
            "released": released.as_ref().map(|pin| pin.commit.to_hex()),
            "head": head.commit_hash.to_hex(),
        }),
    )
    .await?;
    commit(tx).await?;

    Ok(Json(UnpinResponse {
        scope_id,
        channel: channel.name(),
        released: released.map(|pin| pin.commit.to_hex()),
        head: head.commit_hash.to_hex(),
    }))
}

/// The acting principal's identity row, or an [`Error::Invalid`] naming
/// the act.
///
/// A verified subject with no identity row cannot reach here — the packs
/// require a role binding, and bindings are resolved per subject — but the
/// check is explicit rather than an unwrap, as it is on the publish path.
fn acting_identity(input: &authz::DecisionInput, act: &str) -> Result<IdentityId> {
    input
        .identity
        .as_ref()
        .map(|identity| identity.id)
        .ok_or_else(|| Error::Invalid {
            message: format!("{act} requires a provisioned identity"),
        })
}

/// The message/reason bound every FLOW-7 act shares: present and within
/// the commit-message cap, so a governed act always says why.
fn validate_message(message: &str) -> Result<()> {
    let chars = message.chars().count();
    if chars == 0 || chars > MAX_MESSAGE_CHARS {
        return Err(Error::Invalid {
            message: format!("message must be 1..={MAX_MESSAGE_CHARS} characters"),
        });
    }
    Ok(())
}

fn validate(body: &PublishBody) -> Result<()> {
    let invalid = |message: String| Err(Error::Invalid { message });
    // One asset kind per publication, for the reason a proposal carries one
    // (ADR-0049 decision 6): the approval matrix resolves from it.
    let named = usize::from(!body.prompt_names.is_empty())
        + usize::from(!body.document_paths.is_empty())
        + usize::from(!body.skill_names.is_empty());
    match named {
        0 => {
            return invalid(
                "name at least one member: prompt_names for prompts, document_paths for \
                 context pack documents, or skill_names for skills"
                    .to_owned(),
            );
        }
        1 => {}
        _ => {
            return invalid(
                "a publication carries one asset kind: name prompt_names, document_paths \
                 or skill_names, never more than one"
                    .to_owned(),
            );
        }
    }
    if body
        .prompt_names
        .len()
        .max(body.document_paths.len())
        .max(body.skill_names.len())
        > MAX_PUBLISH_MEMBERS
    {
        return invalid(format!(
            "a publication may name at most {MAX_PUBLISH_MEMBERS} members"
        ));
    }
    let chars = body.message.chars().count();
    if chars == 0 || chars > MAX_MESSAGE_CHARS {
        return invalid(format!(
            "message must be 1..={MAX_MESSAGE_CHARS} characters"
        ));
    }
    Ok(())
}
