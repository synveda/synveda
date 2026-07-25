//! The VedaFlow channel API (FLOW-2, ADR-0031 decision 12):
//! `/v1/channels/{scope_id}` behind tenant resolution, uniform-404
//! ownership, and the PDP (`ChannelRead` to see a scope's channels,
//! `ChannelPublish` to move records onto its published one).
//!
//! Publishing is the act that crosses the trust boundary: after it,
//! `inject` composes those records as reviewed material rather than as
//! unreviewed derived output. It is therefore a curator's action by name
//! (seed §5), same-scope (climbing is FLOW-5's, with the higher scope's
//! approvers), and additive — retraction is a rewind, and rewinds are
//! FLOW-7's by name.
//!
//! What lands is bound to *bytes*, not ids: each record's content address
//! is computed from the version being published and stored in the
//! channel's tree, so a later edit demotes the record to unreviewed
//! rather than riding a published id (ADR-0031 decision 5).
//!
//! Since FLOW-3 (ADR-0032 decision 8) this route resolves the **same**
//! approval matrix a proposal does, with the acting principal counting as
//! the only approver. A curator publishing internal memory under
//! `regulated-strict` still works — the matrix asks for one curator and
//! one curator acted — and a `restricted` record refuses and names the
//! proposal route. That is what keeps one matrix rather than two paths:
//! the direct route did not become a hole to close, it became the
//! degenerate case where one approval is enough.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{Action, Resource};
use synveda_store::records::RecordState;
use synveda_store::{hierarchy, records, rls};
use synveda_types::{
    AssetKind, CastApproval, Channel, Error, IdentityId, RecordId, Result, ScopeId, Sensitivity,
};
use synveda_vedaflow::{self as vedaflow, ChannelRef, MemoryAsset, PolicySnapshot, Signer};

use crate::app::AppState;
use crate::approvals;
use crate::audit;
use crate::authz;
use crate::error::ApiError;
use crate::hierarchy::{body, commit, found, tenant_id};
use crate::telemetry::CHANNEL_OPERATIONS_TOTAL;

/// Records per publish. Well below `MAX_CHANNEL_MEMBERS` on purpose: a
/// publish is a reviewed act, and a thousand records in one call is a
/// migration wearing a curator's hat.
const MAX_PUBLISH_RECORDS: usize = 200;

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
    /// The ref name, e.g. `memory/published`.
    name: String,
    asset: String,
    channel: Channel,
    /// Where the channel points — what an inject block cites.
    commit: String,
    /// Entries in that commit's tree: the membership for `published` and
    /// `staged`, the last commit's additions for `derived` (which is a
    /// log, not a set — ADR-0031 decision 3).
    entries: usize,
    updated_at: DateTime<Utc>,
    updated_by: IdentityId,
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
            hierarchy::node(&mut *tx, scope_id).await?,
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
        let statuses = vedaflow::channels::status(&mut tx, tenant_id, scope_id).await?;
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
                })
                .collect(),
        }))
    }
    .await;
    respond(&state, "list", result).await
}

#[derive(Deserialize)]
pub(crate) struct PublishBody {
    /// The records to admit. Must be current records of this scope —
    /// climbing from a child scope is FLOW-5's, under that scope's
    /// approvers.
    record_ids: Vec<RecordId>,
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
    /// Records this call admitted that the channel did not already hold
    /// at that address. Zero means everything named was already
    /// published, unchanged — the act still commits and still audits.
    added: usize,
    /// Each record's content address, in request order: what the tree now
    /// names, and what composition recomputes to decide the record is
    /// still the version that was reviewed.
    published: Vec<PublishedRecord>,
    /// What the approval matrix asked for here, and which of the acting
    /// principal's roles supplied it (FLOW-3, ADR-0032 decision 8).
    /// A publication that needed nothing renders an empty requirement,
    /// which is the honest answer: this pack asks for no review at this
    /// cell.
    required: crate::approvals::RequirementView,
}

#[derive(Serialize)]
struct PublishedRecord {
    record_id: RecordId,
    object_hash: String,
}

/// `POST /v1/channels/{scope_id}/publish` — admit records onto the
/// scope's `memory/published` channel.
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
        hierarchy::node(&mut *tx, scope_id).await?,
        tenant_id,
        scope_id,
    )?;
    let input = authz::gather(state, &mut tx, Some(&node)).await?;
    let authorized = authz::decide(
        state,
        &input,
        Action::ChannelPublish,
        Resource::Scope(scope_id),
        None,
    )?;
    // Two decisions, not one (ADR-0031 decision 12): may this principal
    // publish here, *and* may it read what it is about to declare
    // reviewed. Publication is same-scope, so the second decision is the
    // same scope — but it is the one that keeps a team's curator out of
    // a teammate's personal channel, because the privacy floor
    // (ADR-0015 decision 4) denies `MemoryRead` there. Nobody publishes
    // material they cannot read.
    authz::decide(
        state,
        &input,
        Action::MemoryRead,
        Resource::Scope(scope_id),
        None,
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

    let mut requested = body.record_ids.clone();
    requested.sort_unstable();
    requested.dedup();
    let versions = records::current_at_scope(&mut *tx, tenant_id, scope_id, &requested).await?;
    if versions.len() != requested.len() {
        let found: Vec<RecordId> = versions.iter().map(|version| version.id).collect();
        let missing: Vec<String> = requested
            .iter()
            .filter(|id| !found.contains(id))
            .map(ToString::to_string)
            .collect();
        // Named rather than silently dropped: publishing a subset of what
        // a curator asked for is the one outcome a review surface must
        // never produce quietly.
        return Err(Error::Invalid {
            message: format!(
                "not current records of this scope: {} (cross-scope promotion is FLOW-5)",
                missing.join(", ")
            ),
        });
    }

    // The approval matrix, resolved at this scope from this pack, this
    // asset kind, the *maximum* sensitivity over the set (a set is
    // reviewed as a set), and the nearest curator file on the chain —
    // then satisfied by the acting principal alone or refused with the
    // proposal route named (ADR-0032 decision 8). Same resolution
    // function a proposal uses; there is one matrix, not two.
    let sensitivity = versions
        .iter()
        .map(|version| version.state.sensitivity)
        .max()
        .unwrap_or(Sensitivity::Public);
    let entries: Vec<String> = versions
        .iter()
        .map(|version| version.id.as_uuid().to_string())
        .collect();
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
    let actor = CastApproval {
        identity: author,
        subject: input.principal.subject.clone(),
        roles: approvals::roles_at(&input, &node),
    };
    approvals::require_single_actor(&requirement, &actor, "channel")?;

    // Objects first: each record's content address, computed from the
    // version being published, then stored. Content-addressed, so
    // re-publishing unchanged content stores nothing new.
    let mut members: Vec<(String, vedaflow::hash::ObjectHash)> = Vec::with_capacity(versions.len());
    let mut published: Vec<PublishedRecord> = Vec::with_capacity(versions.len());
    for version in &versions {
        let asset = memory_asset(version.id, &version.state);
        let object = vedaflow::put_memory(&mut tx, tenant_id, &asset).await?;
        members.push((asset.entry_name(), object.hash));
        published.push(PublishedRecord {
            record_id: version.id,
            object_hash: object.hash.to_hex(),
        });
    }

    let channel = ChannelRef::memory(Channel::Published);
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
            // Ids and addresses, never content — the record text stays in
            // `records`, and the address is what an auditor rechecks.
            "records": published.iter().map(|record| json!({
                "record_id": record.record_id,
                "object_hash": record.object_hash,
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
    }))
}

fn validate(body: &PublishBody) -> Result<()> {
    let invalid = |message: String| Err(Error::Invalid { message });
    if body.record_ids.is_empty() {
        return invalid("record_ids must name at least one record".to_owned());
    }
    if body.record_ids.len() > MAX_PUBLISH_RECORDS {
        return invalid(format!(
            "record_ids must name at most {MAX_PUBLISH_RECORDS} records"
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

/// The VedaFlow view of a stored record version (ADR-0031 decision 6).
/// The same field copy the pipeline and the composition engine make, for
/// the reason recorded there: `synveda-store` and `synveda-vedaflow` are
/// siblings, so neither can host a conversion between their types.
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
