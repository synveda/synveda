//! The prompt registry API (PRMT-1, ADR-0049): `/v1/prompts` behind
//! tenant resolution, uniform-404 ownership, and the PDP (`PromptWrite` to
//! author a draft, `PromptRead` to be served one).
//!
//! Three surfaces, and only one of them is on anybody's hot path:
//!
//! - **author** (`POST /v1/prompts`) — writes the draft row and its
//!   content-addressed object. It moves nothing a consumer reads, which is
//!   the whole of "prompt change behind review": the published channel is
//!   somewhere else, and only the approval matrix moves it.
//! - **resolve** (`GET /v1/prompts/{name}`) — the consumer's call. By
//!   default it walks the caller's own placement chain nearest-first and
//!   serves the first scope that publishes the name *and* permits the read,
//!   which is seed §4.4's specificity gradient applied to a fetch: a team's
//!   version overrides the org's, and a nearer copy nobody may read does
//!   not shadow the further one that is readable.
//! - **list** (`GET /v1/prompts?scope_id=…`) — the registry view at one
//!   scope: what is drafted, what is published, and whether they are the
//!   same bytes.
//!
//! # The consumer's pin
//!
//! `?commit=` serves the version a caller was built against while the
//! channel moves on. It is **not** ADR-0036 decision 12's reader-side pin,
//! which was refused: this one is stored nowhere, governs nobody else, and
//! expires with the request.
//!
//! What it cannot do is outlive a withdrawal. The commit must still be a
//! state that channel has held — FLOW-7's own first-parent rule — so a
//! rewind makes the pinned read a `Conflict` naming both commits. Serving
//! the pinned bytes anyway would make "<60s to fleet-wide effect" false;
//! serving the head instead would make the pin false. The refusal is the
//! only answer that leaves both meaning what they say, and it reaches the
//! consumer on its next call rather than its next session.
//!
//! And a pin freezes bytes, never authority (decision 11): the decision is
//! taken at request time, against the live pack, at the tier the pinned
//! version carries. CTX-4's rule for handles, restated — a commit hash is a
//! name, not a capability.
//!
//! # Absent, unpublished and denied answer alike
//!
//! A resolve that cannot serve is `NotFound`, whichever of the three it is
//! (CTX-4, ADR-0041: a recall must not become an oracle for "does this
//! exist"). The one exception is the rewound pin above, which is a
//! `Conflict` — and it is taken *after* a working-tier `PromptRead` at the
//! named scope, so it tells nothing to a caller who could not have read
//! prompts there anyway.

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
use synveda_store::{prompts, rls, scopes};
use synveda_types::scope::Scope;
use synveda_types::{
    Channel, Error, IdentityId, PromptChannel, PromptName, PromptTemplate, PromptVariable, Result,
    ScopeId, Sensitivity,
};
use synveda_vedaflow::{self as vedaflow, ChannelRef, PromptAsset};

use crate::app::AppState;
use crate::audit;
use crate::authz::{self, DecisionInput};
use crate::request::{body, commit, found, tenant_id};
use crate::telemetry::PROMPT_OPERATIONS_TOTAL;

/// Counts the operation and renders the result — the outcome taxonomy
/// every governed plane uses. Error-path audit events chain at this seam
/// (AUD-1, ADR-0019 decision 5).
async fn respond<T: IntoResponse>(
    state: &AppState,
    op: &'static str,
    result: Result<T>,
) -> Response {
    let outcome = crate::response::outcome(&result);
    metrics::counter!(PROMPT_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    crate::response::finish(state, op, result).await
}

// ── Author ─────────────────────────────────────────────────────────────

#[derive(utoipa::ToSchema)]
#[allow(dead_code)] // Contract-only projection for an upstream wire type.
pub(crate) struct PromptVariableSchema {
    name: String,
    description: Option<String>,
    default: Option<String>,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[schema(as = PromptAuthorBody)]
pub(crate) struct AuthorBody {
    /// Where the prompt is authored — the scope that will stand behind it,
    /// and the scope whose published channel a proposal would move.
    #[schema(value_type = String, format = "uuid")]
    scope_id: ScopeId,
    /// Its name: path-shaped, lower-case, and the identifier a consumer
    /// writes in its source (ADR-0049 decision 3).
    #[schema(value_type = String)]
    name: PromptName,
    /// One line, read in a listing and at review.
    #[serde(default)]
    description: String,
    /// The text, with `{{ name }}` placeholders.
    template: String,
    /// Every placeholder the template uses, declared. A schema that
    /// disagrees with the template is refused here rather than discovered
    /// by a consumer (decision 12).
    #[serde(default)]
    #[schema(value_type = Vec<PromptVariableSchema>)]
    variables: Vec<PromptVariable>,
    /// Its classification. Absent means `internal`, the working tier
    /// everything else in the product defaults to. `restricted` is refused
    /// by name: nothing in the product mints that tier for an authored
    /// asset, so a prompt carrying it could never be read back
    /// (decision 5).
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    sensitivity: Option<Sensitivity>,
}

/// What a scope's published channel holds for a name right now — the
/// answer to "is my edit live?", which an author who just saved has to be
/// told rather than left to infer.
#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = PromptPublishedView)]
pub(crate) struct PublishedView {
    /// The commit the channel serves.
    commit: String,
    /// The address it names for this prompt.
    object_hash: String,
    /// Whether that is the draft's own address. `false` after an edit: the
    /// draft has moved and the reviewed version has not, which is what
    /// "behind review" looks like from the writing side.
    current: bool,
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = PromptView)]
pub(crate) struct PromptView {
    name: String,
    #[schema(value_type = String, format = "uuid")]
    scope_id: ScopeId,
    scope_path: String,
    description: String,
    #[schema(value_type = String)]
    sensitivity: Sensitivity,
    template: String,
    #[schema(value_type = Vec<PromptVariableSchema>)]
    variables: Vec<PromptVariable>,
    /// The draft's content address — what a proposal would bind.
    object_hash: String,
    created_at: DateTime<Utc>,
    #[schema(value_type = String, format = "uuid")]
    created_by: IdentityId,
    updated_at: DateTime<Utc>,
    #[schema(value_type = String, format = "uuid")]
    updated_by: IdentityId,
    /// The published version at this scope, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    published: Option<PublishedView>,
}

/// `POST /v1/prompts` — author a draft: create it, or replace the content
/// of the one that is there.
///
/// An overwrite is the authoring act rather than a conflict; what cannot
/// change is the prompt's identity, which migration 0029's trigger
/// enforces below this handler.
#[utoipa::path(
    post,
    path = "/v1/prompts",
    operation_id = "author_prompt",
    tag = "prompts",
    request_body = AuthorBody,
    responses(
        (status = 200, description = "The authored prompt draft", body = PromptView),
        (status = 400, description = "The prompt name, template, variables, or sensitivity is invalid", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Prompt authoring is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "The governing scope is absent", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "prompts.author", skip_all)]
pub(crate) async fn author(
    State(state): State<AppState>,
    payload: std::result::Result<Json<AuthorBody>, JsonRejection>,
) -> Response {
    let result = author_inner(&state, payload).await;
    respond(&state, "author", result).await
}

async fn author_inner(
    state: &AppState,
    payload: std::result::Result<Json<AuthorBody>, JsonRejection>,
) -> Result<Json<PromptView>> {
    let body = body(payload)?;
    let sensitivity = body.sensitivity.unwrap_or(Sensitivity::WORKING);
    if sensitivity == Sensitivity::Restricted {
        return Err(Error::Invalid {
            message: "a prompt cannot be `restricted`: the only path to that tier is a \
                      classification proposal over records, priced at compliance plus two \
                      distinct approvers (ADR-0038 decision 8), and no such path exists for \
                      an authored asset — so nothing could read the prompt back \
                      (ADR-0049 decision 5)"
                .to_owned(),
        });
    }
    // Sorted by name before anything is hashed or stored, so the order a
    // reader is served and the order the address covers are one list.
    let mut variables = body.variables.clone();
    variables.sort_by(|left, right| left.name.cmp(&right.name));
    let template = PromptTemplate {
        name: body.name.clone(),
        description: body.description.clone(),
        template: body.template.clone(),
        variables,
    };
    template.validate()?;

    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let node = found(
        scopes::get(&mut *tx, tenant_id, body.scope_id).await?,
        tenant_id,
        body.scope_id,
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
        Action::PromptWrite,
        Resource::Scope(body.scope_id),
    )?;
    let author = identity_of(&input)?;

    // The object first: the draft row's foreign key requires it, which is
    // what makes "the bytes a proposal will bind are already stored" a
    // property of the schema rather than of this handler.
    let asset = PromptAsset {
        scope_id: body.scope_id,
        sensitivity,
        template,
    };
    let object = vedaflow::put_prompt(&mut tx, tenant_id, &asset).await?;
    let stored = prompts::upsert(
        &mut *tx,
        tenant_id,
        &prompts::NewPrompt {
            scope_id: body.scope_id,
            template: &asset.template,
            sensitivity,
            object_hash: *object.hash.as_bytes(),
            author,
        },
    )
    .await?;
    let published = published_at(&mut tx, tenant_id, body.scope_id, &body.name).await?;

    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::PromptAuthored,
        Resource::Scope(body.scope_id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::PromptWrite, &authorized),
            "asset": synveda_types::AssetKind::Prompt.as_str(),
            "name": body.name.as_str(),
            "sensitivity": sensitivity.as_str(),
            // The address, never the template. An auditor rechecks the
            // former; the latter is content, and no payload has carried
            // content since AUD-1.
            "object_hash": object.hash.to_hex(),
            "deduplicated": object.deduplicated,
            "variables": asset.template.variables.len(),
            // What a consumer is being served *now*, which is the point of
            // the whole feature: authoring moved nothing.
            "published": published.as_ref().map(|(commit, hash)| json!({
                "commit": commit.to_hex(),
                "object_hash": hash.to_hex(),
                "current": *hash == object.hash,
            })),
        }),
    )
    .await?;
    commit(tx).await?;

    Ok(Json(view(&node, stored, published)))
}

// ── Resolve ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct ResolveParams {
    /// Which version. Absent means `published` — a consumer that asks for
    /// nothing in particular gets the reviewed one.
    #[serde(default)]
    channel: Option<PromptChannel>,
    /// Which scope. Absent walks the caller's placement chain nearest-first
    /// (published only). Required for `draft`, and required beside
    /// `commit`.
    #[serde(default)]
    scope_id: Option<ScopeId>,
    /// The commit to pin to — a state that scope's channel has held. The
    /// resolve response carries the pair to pin with, which is why naming a
    /// commit without its scope is refused rather than searched for.
    #[serde(default)]
    commit: Option<String>,
}

/// Where the served bytes came from — one field with four honest answers,
/// because a response that cites a frozen commit without saying so
/// overstates its own freshness (ADR-0036 decision 10, applied here).
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug, utoipa::ToSchema)]
#[schema(as = PromptOrigin)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Origin {
    /// The channel's head: the current reviewed version.
    Head,
    /// The commit this request named.
    PinnedCommit,
    /// A standing FLOW-7 pin on that scope's channel — the caller asked for
    /// the head and the scope is holding its readers elsewhere.
    ChannelPin,
    /// The authoring row. Unreviewed by construction.
    Draft,
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = PromptResolveResponse)]
pub(crate) struct ResolveResponse {
    name: String,
    /// The scope the version came from — for a walked resolve, the nearest
    /// one on the caller's chain that publishes it and permits the read.
    #[schema(value_type = String, format = "uuid")]
    scope_id: ScopeId,
    scope_path: String,
    #[schema(value_type = String)]
    channel: PromptChannel,
    /// What produced these bytes.
    origin: Origin,
    /// The commit whose tree named this version — what a consumer pins next
    /// time. Absent for a draft, which is on no channel.
    #[serde(skip_serializing_if = "Option::is_none")]
    commit: Option<String>,
    /// The version's content address.
    object_hash: String,
    #[schema(value_type = String)]
    sensitivity: Sensitivity,
    description: String,
    template: String,
    #[schema(value_type = Vec<PromptVariableSchema>)]
    variables: Vec<PromptVariable>,
}

/// `GET /v1/prompts/{name}` — resolve a prompt for this caller.
#[utoipa::path(
    get,
    path = "/v1/prompts/{name}",
    operation_id = "resolve_prompt",
    tag = "prompts",
    params(
        ("name" = String, Path),
        ("channel" = Option<String>, Query),
        ("scope_id" = Option<String>, Query, format = "uuid"),
        ("commit" = Option<String>, Query)
    ),
    responses(
        (status = 200, description = "The resolved prompt version", body = ResolveResponse),
        (status = 400, description = "The prompt selector is invalid", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Prompt read is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "No visible prompt matches", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "prompts.resolve", skip_all)]
pub(crate) async fn resolve(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(params): Query<ResolveParams>,
) -> Response {
    let result = resolve_inner(&state, &name, &params).await;
    respond(&state, "resolve", result).await
}

async fn resolve_inner(
    state: &AppState,
    raw_name: &str,
    params: &ResolveParams,
) -> Result<Json<ResolveResponse>> {
    let name: PromptName = raw_name.parse()?;
    let channel = params.channel.unwrap_or(PromptChannel::Published);
    let pinned = params
        .commit
        .as_deref()
        .map(str::parse::<vedaflow::CommitHash>)
        .transpose()?;
    if channel == PromptChannel::Draft && pinned.is_some() {
        return Err(Error::Invalid {
            message: "a draft is on no channel, so there is no commit to pin it to".to_owned(),
        });
    }
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;

    let resolved = match (params.scope_id, channel) {
        (None, PromptChannel::Draft) => {
            return Err(Error::Invalid {
                message: "reading a draft names its scope: unreviewed content reaches a \
                          caller who asked for that scope's unreviewed content, and nobody \
                          else (ADR-0049 decision 15)"
                    .to_owned(),
            });
        }
        (None, PromptChannel::Published) => {
            if pinned.is_some() {
                return Err(Error::Invalid {
                    message: "pinning a commit names its scope too — a commit belongs to \
                              one scope's channel, and the resolve response carries the \
                              pair to pin with"
                        .to_owned(),
                });
            }
            walk_chain(state, &mut tx, tenant_id, &name).await?
        }
        (Some(scope_id), channel) => {
            at_scope(state, &mut tx, tenant_id, scope_id, &name, channel, pinned).await?
        }
    };

    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::PromptResolved,
        Resource::Scope(resolved.node.id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::PromptRead, &resolved.authorized),
            "asset": synveda_types::AssetKind::Prompt.as_str(),
            "name": name.as_str(),
            "channel": channel.as_str(),
            "origin": resolved.origin,
            "sensitivity": resolved.asset.sensitivity.as_str(),
            // The address and the commit — the citation a consumer can pin
            // and an auditor can recompute. Never the template.
            "object_hash": resolved.object_hash.to_hex(),
            "commit": resolved.commit.map(|commit| commit.to_hex()),
            // How far up the caller's own chain the name was found. Zero is
            // the caller's home; a larger number is the gradient working.
            "chain_position": resolved.position,
        }),
    )
    .await?;
    commit(tx).await?;

    Ok(Json(ResolveResponse {
        name: name.to_string(),
        scope_id: resolved.node.id,
        scope_path: resolved.node.slug.clone(),
        channel,
        origin: resolved.origin,
        commit: resolved.commit.map(|commit| commit.to_hex()),
        object_hash: resolved.object_hash.to_hex(),
        sensitivity: resolved.asset.sensitivity,
        description: resolved.asset.template.description.clone(),
        template: resolved.asset.template.template.clone(),
        variables: resolved.asset.template.variables.clone(),
    }))
}

/// What a resolution found, before it is rendered or audited.
struct Resolved {
    node: Scope,
    asset: PromptAsset,
    object_hash: vedaflow::hash::ObjectHash,
    commit: Option<vedaflow::CommitHash>,
    origin: Origin,
    /// Distance up the caller's chain — 0 at home. Always 0 for a resolve
    /// that named its scope.
    position: usize,
    authorized: crate::authz::Authorized,
}

/// The gradient walk (ADR-0049 decision 8): the caller's own placement
/// chain, nearest first, serving the first scope that publishes the name
/// **and** permits the read.
///
/// A denied scope is skipped rather than fatal, which is what keeps a
/// nearer copy nobody may read from shadowing the further one that is
/// readable — and the whole walk answers `NotFound` when nothing matches,
/// so a name is never an existence oracle.
///
/// One channel read for the whole chain (the composition engine's own
/// query shape), then at most one object read.
async fn walk_chain(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant_id: synveda_types::TenantId,
    name: &PromptName,
) -> Result<Resolved> {
    let input = authz::gather_at_home(state, tx).await?;
    let chain: Vec<synveda_policy::ScopeNode> = input.chain.to_vec();
    let scope_ids: Vec<ScopeId> = chain.iter().map(|node| node.id).collect();
    let published =
        vedaflow::read_prompt_members(tx, tenant_id, &scope_ids, Channel::Published).await?;

    for (position, node) in chain.iter().enumerate() {
        let Some(state_at) = published.iter().find(|state| state.scope_id == node.id) else {
            continue;
        };
        let Some(address) = state_at.members.get(name).copied() else {
            continue;
        };
        let (asset, authorized) =
            match admit(state, tx, tenant_id, &input, position, node, address).await? {
                Some(admitted) => admitted,
                None => continue,
            };
        let node = scopes::get(tx, tenant_id, node.id)
            .await?
            .expect("the chain's own node resolves");
        return Ok(Resolved {
            node: node.clone(),
            asset,
            object_hash: address,
            commit: Some(state_at.commit),
            origin: if state_at.pinned {
                Origin::ChannelPin
            } else {
                Origin::Head
            },
            position,
            authorized,
        });
    }
    Err(not_found(name))
}

/// A resolve that named its scope: the draft row, the channel head, or the
/// commit the caller pinned.
async fn at_scope(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant_id: synveda_types::TenantId,
    scope_id: ScopeId,
    name: &PromptName,
    channel: PromptChannel,
    pinned: Option<vedaflow::CommitHash>,
) -> Result<Resolved> {
    let node = found(
        scopes::get(&mut *tx, tenant_id, scope_id).await?,
        tenant_id,
        scope_id,
    )?;
    let input = authz::gather(
        state,
        tx,
        Some(&node),
        synveda_store::anchors::AnchorSelection::none(),
        Vec::new(),
    )
    .await?;

    if channel == PromptChannel::Draft {
        let Some(draft) = prompts::read(&mut *tx, tenant_id, scope_id, name).await? else {
            return Err(not_found(name));
        };
        let asset = PromptAsset {
            scope_id,
            sensitivity: draft.sensitivity,
            template: draft.template,
        };
        let Some(authorized) = permit(state, &input, scope_id, asset.sensitivity)? else {
            return Err(not_found(name));
        };
        return Ok(Resolved {
            node,
            object_hash: vedaflow::hash::ObjectHash::from_slice(&draft.object_hash)?,
            asset,
            commit: None,
            origin: Origin::Draft,
            position: 0,
            authorized,
        });
    }

    // Published, at a named scope. The pin's refusal is a `Conflict` and
    // every other outcome is the uniform `NotFound`, so this working-tier
    // decision comes first: it tells nothing about the channel to a caller
    // who could not read prompts here anyway. Same question the publish
    // route asks at the same tier — whose material is this — rather than
    // how sensitive it is (ADR-0038 decision 10).
    if permit(state, &input, scope_id, Sensitivity::WORKING)?.is_none() {
        return Err(not_found(name));
    }
    let channel_ref = ChannelRef::prompt(Channel::Published);
    let head = vedaflow::read_ref(&mut *tx, tenant_id, scope_id, &channel_ref.name())
        .await?
        .ok_or_else(|| not_found(name))?;
    // What this scope actually serves: its head, unless a standing FLOW-7
    // pin is holding its readers at an earlier state (ADR-0036 decision 6).
    let standing = vedaflow::read_pin(&mut *tx, tenant_id, scope_id, channel_ref).await?;
    let served = standing.as_ref().map_or(head.commit_hash, |pin| pin.commit);

    let (commit_hash, origin) = match pinned {
        None => (
            served,
            standing
                .as_ref()
                .map_or(Origin::Head, |_| Origin::ChannelPin),
        ),
        Some(wanted) => {
            // The pin may only name a state the channel has held — FLOW-7's
            // own first-parent rule (ADR-0036 decision 1), which is what
            // makes a rewind reach a pinned consumer.
            //
            // Measured against what the scope **serves**, not against its
            // head: under a standing FLOW-7 pin those differ, and taking the
            // head would let a request parameter hand a consumer a version
            // the scope is deliberately holding its readers back from.
            // "Exactly one thing decides what readers see" (ADR-0036
            // decision 7) survives a consumer pin only if the scope's hold is
            // the ceiling — a consumer may pin at or below what the scope
            // serves, never above it.
            if !vedaflow::is_first_parent_ancestor(&mut *tx, tenant_id, wanted, served).await? {
                return Err(Error::Conflict {
                    message: format!(
                        "{} is not a state {} at this scope has held; it now serves {}. \
                         A rewind withdrew that version, and serving it anyway would make \
                         a rollback partial (FLOW-7); re-resolve to take the current one \
                         deliberately",
                        wanted.to_hex(),
                        channel_ref,
                        served.to_hex(),
                    ),
                });
            }
            (wanted, Origin::PinnedCommit)
        }
    };

    let Some(address) = member_at(&mut *tx, tenant_id, commit_hash, name).await? else {
        return Err(not_found(name));
    };
    // The tier decision is taken now, against the live pack, at the tier the
    // *served* version carries: a pin freezes bytes and never authority
    // (decision 11).
    let Some(scope_node) = input.chain.first() else {
        return Err(not_found(name));
    };
    let Some((asset, authorized)) =
        admit(state, tx, tenant_id, &input, 0, scope_node, address).await?
    else {
        return Err(not_found(name));
    };
    Ok(Resolved {
        node,
        asset,
        object_hash: address,
        commit: Some(commit_hash),
        origin,
        position: 0,
        authorized,
    })
}

/// Reads the object at `address`, and decides `PromptRead` at the tier it
/// carries. `None` means the decision denied — the caller skips or answers
/// `NotFound`, never a policy error, so the two are indistinguishable.
async fn admit(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant_id: synveda_types::TenantId,
    input: &DecisionInput,
    position: usize,
    node: &synveda_policy::ScopeNode,
    address: vedaflow::hash::ObjectHash,
) -> Result<Option<(PromptAsset, crate::authz::Authorized)>> {
    let object = vedaflow::read_object(&mut *tx, tenant_id, address)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!(
                "{} names object {} which the append-only store does not hold",
                node.slug,
                address.to_hex()
            ),
        })?;
    let asset = PromptAsset::from_bytes(&object.content)?;
    let authorized = authz::decide_prompt_read_from(
        state,
        input,
        position,
        Resource::Scope(node.id),
        asset.sensitivity,
    );
    match authorized {
        Ok(authorized) => Ok(Some((asset, authorized))),
        Err(Error::PolicyDenied { .. }) => Ok(None),
        Err(other) => Err(other),
    }
}

/// One `PromptRead` decision at `scope_id`, as an option rather than an
/// error: the resolve surface turns every denial into the uniform
/// `NotFound`.
fn permit(
    state: &AppState,
    input: &DecisionInput,
    scope_id: ScopeId,
    sensitivity: Sensitivity,
) -> Result<Option<crate::authz::Authorized>> {
    match authz::decide_prompt_read(state, input, Resource::Scope(scope_id), sensitivity) {
        Ok(authorized) => Ok(Some(authorized)),
        Err(Error::PolicyDenied { .. }) => Ok(None),
        Err(other) => Err(other),
    }
}

/// The one answer a resolve gives for absent, unpublished, and denied
/// alike (ADR-0041's rule for handles, applied to names).
fn not_found(name: &PromptName) -> Error {
    Error::NotFound {
        entity: format!("prompt {name}"),
    }
}

/// The address a commit's tree names for `name`, if it names one.
async fn member_at(
    tx: &mut sqlx::PgConnection,
    tenant_id: synveda_types::TenantId,
    commit_hash: vedaflow::CommitHash,
    name: &PromptName,
) -> Result<Option<vedaflow::hash::ObjectHash>> {
    let Some(stored) = vedaflow::read_commit(&mut *tx, tenant_id, commit_hash).await? else {
        return Ok(None);
    };
    // The store is append-only, so a commit's tree always resolves; a miss
    // would be corruption rather than an absent member.
    let tree = vedaflow::read_tree(&mut *tx, tenant_id, stored.tree)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!(
                "commit {} names tree {} which the append-only store does not hold",
                commit_hash.to_hex(),
                stored.tree.to_hex()
            ),
        })?;
    Ok(tree.into_iter().find_map(|entry| match entry.target {
        vedaflow::TreeTarget::Object(hash) if entry.name == name.as_str() => Some(hash),
        _ => None,
    }))
}

// ── List ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct ListParams {
    /// The scope whose registry to list. Required: a listing is a scope's
    /// own shelf, and a tenant-wide one would be a different question with
    /// a different resource.
    scope_id: ScopeId,
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = PromptListEntry)]
pub(crate) struct ListEntry {
    name: String,
    description: String,
    #[schema(value_type = String)]
    sensitivity: Sensitivity,
    /// The draft's address, and when it last moved.
    object_hash: String,
    updated_at: DateTime<Utc>,
    #[schema(value_type = String, format = "uuid")]
    updated_by: IdentityId,
    #[schema(value_type = Vec<PromptVariableSchema>)]
    variables: Vec<PromptVariable>,
    #[serde(skip_serializing_if = "Option::is_none")]
    published: Option<PublishedView>,
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = PromptListResponse)]
pub(crate) struct ListResponse {
    #[schema(value_type = String, format = "uuid")]
    scope_id: ScopeId,
    scope_path: String,
    prompts: Vec<ListEntry>,
}

/// `GET /v1/prompts?scope_id=…` — the registry at one scope: every draft,
/// with what the published channel holds for it.
///
/// Entries the caller may not read at their tier are omitted rather than
/// refused, for the reason the walk skips them: a listing that refused
/// wholesale would make one `confidential` prompt hide the rest.
#[utoipa::path(
    get,
    path = "/v1/prompts",
    operation_id = "list_prompts",
    tag = "prompts",
    params(("scope_id" = String, Query, format = "uuid")),
    responses(
        (status = 200, description = "Visible prompt drafts at the scope", body = ListResponse),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Prompt read is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "The governing scope is absent", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "prompts.list", skip_all)]
pub(crate) async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Response {
    let result = list_inner(&state, params.scope_id).await;
    respond(&state, "list", result).await
}

async fn list_inner(state: &AppState, scope_id: ScopeId) -> Result<Json<ListResponse>> {
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
    // The gate first: may this principal see this scope's shelf at all. At
    // the working tier, which is the question a listing asks — the allowed
    // decision is also what puts the read on the chain (ADR-0019
    // decision 4).
    let authorized = authz::decide_prompt_read(
        state,
        &input,
        Resource::Scope(scope_id),
        Sensitivity::WORKING,
    )?;
    // Then one decision per tier the shelf actually carries — at most three,
    // and usually one. The `retrieval::plan` shape (ADR-0038 decision 3):
    // ask per tier, keep the answers as a set.
    let drafts = prompts::list(&mut *tx, tenant_id, scope_id).await?;
    let mut permitted: BTreeMap<Sensitivity, bool> = BTreeMap::new();
    for draft in &drafts {
        if let std::collections::btree_map::Entry::Vacant(slot) = permitted.entry(draft.sensitivity)
        {
            slot.insert(permit(state, &input, scope_id, draft.sensitivity)?.is_some());
        }
    }

    let published =
        vedaflow::read_prompt_members(&mut tx, tenant_id, &[scope_id], Channel::Published)
            .await?
            .into_iter()
            .next();
    let mut entries = Vec::with_capacity(drafts.len());
    for draft in drafts {
        if !permitted.get(&draft.sensitivity).copied().unwrap_or(false) {
            continue;
        }
        let address = vedaflow::hash::ObjectHash::from_slice(&draft.object_hash)?;
        let published = published.as_ref().and_then(|state| {
            state
                .members
                .get(&draft.template.name)
                .map(|hash| PublishedView {
                    commit: state.commit.to_hex(),
                    object_hash: hash.to_hex(),
                    current: *hash == address,
                })
        });
        entries.push(ListEntry {
            name: draft.template.name.to_string(),
            description: draft.template.description,
            sensitivity: draft.sensitivity,
            object_hash: address.to_hex(),
            updated_at: draft.updated_at,
            updated_by: draft.updated_by,
            variables: draft.template.variables,
            published,
        });
    }

    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::AuthzDecision,
        Resource::Scope(scope_id).to_string(),
        Outcome::Allow,
        json!({
            "authz": audit::decision_context(Action::PromptRead, &authorized),
            "op": "prompts.list",
            "prompts": entries.len(),
        }),
    )
    .await?;
    commit(tx).await?;

    Ok(Json(ListResponse {
        scope_id,
        scope_path: node.slug.clone(),
        prompts: entries,
    }))
}

// ── Shared ─────────────────────────────────────────────────────────────

/// What `scope`'s published prompt channel holds for `name`: the commit it
/// serves and the address it names.
async fn published_at(
    tx: &mut sqlx::PgConnection,
    tenant_id: synveda_types::TenantId,
    scope_id: ScopeId,
    name: &PromptName,
) -> Result<Option<(vedaflow::CommitHash, vedaflow::hash::ObjectHash)>> {
    Ok(
        vedaflow::read_prompt_members(tx, tenant_id, &[scope_id], Channel::Published)
            .await?
            .into_iter()
            .next()
            .and_then(|state| state.members.get(name).map(|hash| (state.commit, *hash))),
    )
}

fn view(
    node: &Scope,
    stored: prompts::StoredPrompt,
    published: Option<(vedaflow::CommitHash, vedaflow::hash::ObjectHash)>,
) -> PromptView {
    let address = stored.object_hash;
    PromptView {
        name: stored.template.name.to_string(),
        scope_id: stored.scope_id,
        scope_path: node.slug.clone(),
        description: stored.template.description,
        sensitivity: stored.sensitivity,
        template: stored.template.template,
        variables: stored.template.variables,
        object_hash: vedaflow::hash::ObjectHash::from_bytes(address).to_hex(),
        created_at: stored.created_at,
        created_by: stored.created_by,
        updated_at: stored.updated_at,
        updated_by: stored.updated_by,
        published: published.map(|(commit, hash)| PublishedView {
            commit: commit.to_hex(),
            object_hash: hash.to_hex(),
            current: hash.as_bytes() == &address,
        }),
    }
}

/// The authoring identity. A verified subject with no identity row cannot
/// reach here — every pack requires either a binding or placement — but the
/// check is explicit rather than an unwrap.
fn identity_of(input: &DecisionInput) -> Result<IdentityId> {
    input
        .identity
        .as_ref()
        .map(|identity| identity.id)
        .ok_or_else(|| Error::Invalid {
            message: "authoring a prompt requires a provisioned identity".to_owned(),
        })
}
