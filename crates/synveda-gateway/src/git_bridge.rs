//! FLOW-8 application service: fresh disclosure authority, immutable source
//! evidence and recoverable content-free VedaFlow cursors (ADR-0138).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Duration;

use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    response::Response,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgConnection;
use synveda_audit::{AuditAction, Outcome};
use synveda_git::{
    CommitEvidence, Destination, EntryEvidence, GitHubCredential, ObjectEvidence, PinEvidence,
    Projection, Snapshot, TreeEvidence,
};
use synveda_policy::{Action, Resource};
use synveda_store::{configuration, git_exports as evidence, rls, scopes};
use synveda_types::configuration::{EffectiveConfiguration, ExternalProvider};
use synveda_types::secret::{TenantSecretKind, TenantSecretState};
use synveda_types::{
    AssetKind, Channel, Error, IdentityId, Result, ScopeId, TenantId, TenantSecretId,
};
use synveda_vedaflow::{self as flow, ChannelRef, CommitHash, PolicySnapshot, Signer};

use crate::{
    app::AppState,
    audit, authz,
    request::{body, commit, found, tenant_id},
};

pub(crate) const OPERATIONS_TOTAL: &str = "synveda_git_export_operations_total";
pub(crate) const OPERATION_SECONDS: &str = "synveda_git_export_operation_seconds";
const DEADLINE: Duration = Duration::from_secs(60);

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GitExportBody {
    /// Governed destination ID. The deployment fixes local or private GitHub custody.
    target: String,
    /// `prompt` or `context-pack`.
    asset: String,
    /// `published` or `staged`.
    channel: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub(crate) struct GitExportResponse {
    target: String,
    /// `local` or `github`.
    provider: String,
    /// Content-free binding to exact destination and credential custody.
    destination_digest: String,
    /// Pinned GitHub repository ID, absent for local export.
    repository_id: Option<u64>,
    source_head: String,
    source_pin: Option<String>,
    git_ref: String,
    git_head: String,
    mapping_digest: String,
    commits: usize,
    objects: usize,
    /// `completed`, `resumed`, or `no_op`. No content or filesystem path.
    outcome: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RefState {
    head: String,
    updated_at: DateTime<Utc>,
    updated_by: IdentityId,
    pin: Option<PinEvidence>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Prepared,
    Completed,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    format: String,
    target: String,
    destination_digest: String,
    asset: AssetKind,
    channel: Channel,
    source: RefState,
    source_digest: String,
    previous_git_head: Option<String>,
    git_head: String,
    mapping_digest: String,
    export_states: usize,
    manifest_bytes: usize,
    objects: BTreeMap<String, String>,
    trees: BTreeMap<String, String>,
    commits: BTreeMap<String, String>,
    phase: Phase,
}

struct DestinationEvidence {
    provider: &'static str,
    digest: String,
    repository_id: Option<u64>,
    secret_id: Option<TenantSecretId>,
    secret_value_revision: Option<u64>,
}

struct Custody {
    credential: Option<GitHubCredential>,
    revision: Option<u64>,
}

fn invalid_evidence() -> Error {
    Error::Internal {
        message: "Git export source or cursor evidence failed integrity checks".into(),
    }
}

fn conflict() -> Error {
    Error::Conflict {
        message: "Git export cursor changed; retry the export".into(),
    }
}

/// Export the complete bounded authored-channel graph to a governed local or private GitHub
/// destination. Replays repeat current Cedar and per-artifact read decisions.
/// A prepared receipt survives cancellation and is resumed before new heads.
#[utoipa::path(
    post, path = "/v1/channels/{scope_id}/git-export",
    operation_id = "export_channel_to_git", tag = "channels",
    params(("scope_id" = String, Path, format = "uuid")),
    request_body = GitExportBody,
    responses(
        (status = 200, description = "Verified local or private GitHub advancement or no-op replay", body = GitExportResponse),
        (status = 400, description = "Invalid selector, disabled destination or source bound", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Disclosure or an exact artifact read is denied", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "Scope or channel is absent or foreign", body = crate::workspaces::ApiErrorBody),
        (status = 409, description = "Git destination diverged or cursor changed", body = crate::workspaces::ApiErrorBody),
        (status = 500, description = "Git transport, integrity or deadline failure; prepared receipt retained", body = crate::workspaces::ApiErrorBody),
    ), security(("bearer" = [])),
)]
#[tracing::instrument(name = "git.export", skip_all)]
pub(crate) async fn export(
    State(state): State<AppState>,
    Path(scope): Path<ScopeId>,
    payload: std::result::Result<Json<GitExportBody>, JsonRejection>,
) -> Response {
    let started = std::time::Instant::now();
    let result = match tokio::time::timeout(DEADLINE, run(&state, scope, payload)).await {
        Ok(result) => result,
        Err(_) => Err(Error::Internal {
            message: "Git export deadline exceeded; retry to recover any prepared receipt".into(),
        }),
    };
    let outcome = crate::response::outcome(&result);
    metrics::counter!(OPERATIONS_TOTAL, "outcome" => outcome).increment(1);
    metrics::histogram!(OPERATION_SECONDS, "outcome" => outcome)
        .record(started.elapsed().as_secs_f64());
    crate::response::finish(&state, "git_export", result).await
}

async fn run(
    state: &AppState,
    scope: ScopeId,
    payload: std::result::Result<Json<GitExportBody>, JsonRejection>,
) -> Result<Json<GitExportResponse>> {
    let body = body(payload)?;
    synveda_git::validate_target(&body.target)?;
    let asset: AssetKind = body.asset.parse()?;
    if !asset.has_channels() {
        return Err(Error::Invalid {
            message: "Git export admits only Prompt and ContextPack channels".into(),
        });
    }
    let channel: Channel = body.channel.parse()?;
    let channel_ref = ChannelRef::new(asset, channel);
    let tenant = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
    let (authorized, actor, config) =
        authorize_channel(state, &mut tx, tenant, scope, &body.target).await?;
    let transport = state.git_exports.as_ref().ok_or_else(|| Error::Invalid {
        message: "Git export transport is disabled".into(),
    })?;
    let destination = transport.destination(tenant, scope, &body.target)?;
    require_provider(&config, destination)?;
    let mut destination_evidence = DestinationEvidence {
        provider: destination.provider(),
        digest: destination.digest(tenant, scope, &body.target)?,
        repository_id: destination.repository_id(),
        secret_id: destination.secret_id()?,
        secret_value_revision: None,
    };
    let name = format!("git-export/{}/{channel_ref}", body.target);
    let previous_cursor = read_cursor(&mut tx, tenant, scope, &name).await?;
    if previous_cursor
        .as_ref()
        .is_some_and(|(_, c)| c.target != body.target || c.asset != asset || c.channel != channel)
    {
        return Err(invalid_evidence());
    }
    if previous_cursor
        .as_ref()
        .is_some_and(|(_, c)| c.destination_digest != destination_evidence.digest)
    {
        return Err(Error::Conflict { message: "Git export destination custody changed; retain the old destination and enable a governed new target identifier".into() });
    }
    let resuming = previous_cursor
        .as_ref()
        .is_some_and(|(_, c)| c.phase == Phase::Prepared);
    let source = match &previous_cursor {
        Some((_, cursor)) if resuming => cursor.source.clone(),
        _ => source_ref(&mut tx, tenant, scope, channel_ref).await?,
    };
    let snapshot = capture(
        &mut tx,
        tenant,
        scope,
        &body.target,
        channel_ref,
        source.clone(),
    )
    .await?;
    authorize_objects(state, &mut tx, tenant, &snapshot).await?;
    let custody = open_custody(state, &mut tx, tenant, scope, destination).await?;
    destination_evidence.secret_value_revision = custody.revision;
    let digest = snapshot.digest()?;
    let no_op = previous_cursor
        .as_ref()
        .is_some_and(|(_, c)| c.phase == Phase::Completed && c.source_digest == digest);
    let prior = previous_cursor.as_ref().and_then(|(_, c)| {
        if resuming || no_op {
            c.previous_git_head.clone()
        } else {
            Some(c.git_head.clone())
        }
    });
    let projection = Projection::render(snapshot, prior.clone())?;
    let manifest_size = serde_json::to_vec(&projection.manifest)
        .map_err(|_| invalid_evidence())?
        .len();
    let (export_states, manifest_bytes) =
        previous_cursor
            .as_ref()
            .map_or((1, manifest_size), |(_, cursor)| {
                if resuming || no_op {
                    (cursor.export_states, cursor.manifest_bytes)
                } else {
                    (
                        cursor.export_states.saturating_add(1),
                        cursor.manifest_bytes.saturating_add(manifest_size),
                    )
                }
            });
    if export_states == 0
        || export_states > synveda_git::MAX_EXPORT_STATES
        || manifest_bytes < manifest_size
        || manifest_bytes > synveda_git::MAX_VERIFICATION_BYTES
    {
        return Err(Error::Invalid { message: "Git export target exceeds its verifiable history bound; retain it and enable a governed new target identifier".into() });
    }
    if (resuming || no_op)
        && previous_cursor.as_ref().is_some_and(|(_, c)| {
            c.git_head != projection.head
                || c.mapping_digest != projection.manifest.mapping_digest
                || c.source_digest != digest
                || c.objects != projection.manifest.objects
                || c.trees != projection.manifest.trees
                || c.commits != projection.manifest.commits
        })
    {
        return Err(invalid_evidence());
    }
    if no_op {
        destination
            .advance(
                &projection,
                prior.as_deref(),
                true,
                custody.credential.as_ref(),
            )
            .await?;
        record(
            &mut tx,
            tenant,
            scope,
            &projection,
            &authorized,
            &config.content_hash,
            &destination_evidence,
            "no_op",
            Outcome::Success,
        )
        .await?;
        commit(tx).await?;
        return Ok(Json(response(&projection, &destination_evidence, "no_op")));
    }
    let mut cursor = Cursor {
        format: "synveda-git-cursor-v2".into(),
        target: body.target,
        destination_digest: destination_evidence.digest.clone(),
        asset,
        channel,
        source,
        source_digest: digest,
        previous_git_head: prior,
        git_head: projection.head.clone(),
        mapping_digest: projection.manifest.mapping_digest.clone(),
        export_states,
        manifest_bytes,
        objects: projection.manifest.objects.clone(),
        trees: projection.manifest.trees.clone(),
        commits: projection.manifest.commits.clone(),
        phase: Phase::Prepared,
    };
    let prepared_hash = if resuming {
        previous_cursor
            .as_ref()
            .map(|(hash, _)| *hash)
            .ok_or_else(invalid_evidence)?
    } else {
        save_cursor(
            &mut tx,
            tenant,
            scope,
            &name,
            previous_cursor.as_ref().map(|(hash, _)| *hash),
            &cursor,
            actor,
            &authorized,
        )
        .await?
    };
    record(
        &mut tx,
        tenant,
        scope,
        &projection,
        &authorized,
        &config.content_hash,
        &destination_evidence,
        if resuming {
            "resume_prepared"
        } else {
            "prepared"
        },
        Outcome::Success,
    )
    .await?;
    // Durable intent precedes every destination content write. A lost HTTP
    // response or later transaction can never make an unknown disclosure.
    commit(tx).await?;
    drop(custody);

    let mut effect = rls::begin_tenant_tx(&state.pool, tenant).await?;
    let (authorized, actor, config) =
        authorize_channel(state, &mut effect, tenant, scope, &cursor.target).await?;
    require_provider(&config, destination)?;
    authorize_objects(state, &mut effect, tenant, &projection.manifest.snapshot).await?;
    let current = read_cursor(&mut effect, tenant, scope, &name)
        .await?
        .ok_or_else(conflict)?;
    if current.0 != prepared_hash {
        return Err(conflict());
    }
    let custody = open_custody(state, &mut effect, tenant, scope, destination).await?;
    destination_evidence.secret_value_revision = custody.revision;
    let advanced = destination
        .advance(
            &projection,
            cursor.previous_git_head.as_deref(),
            false,
            custody.credential.as_ref(),
        )
        .await;
    if let Err(error) = advanced {
        record(
            &mut effect,
            tenant,
            scope,
            &projection,
            &authorized,
            &config.content_hash,
            &destination_evidence,
            "transport_failed",
            Outcome::Failure,
        )
        .await?;
        commit(effect).await?;
        return Err(error);
    }
    cursor.phase = Phase::Completed;
    save_cursor(
        &mut effect,
        tenant,
        scope,
        &name,
        Some(prepared_hash),
        &cursor,
        actor,
        &authorized,
    )
    .await?;
    let outcome = if resuming { "resumed" } else { "completed" };
    record(
        &mut effect,
        tenant,
        scope,
        &projection,
        &authorized,
        &config.content_hash,
        &destination_evidence,
        outcome,
        Outcome::Success,
    )
    .await?;
    commit(effect).await?;
    Ok(Json(response(&projection, &destination_evidence, outcome)))
}

async fn authorize_channel(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    scope: ScopeId,
    target: &str,
) -> Result<(authz::Authorized, IdentityId, EffectiveConfiguration)> {
    let node = found(scopes::get(&mut *tx, tenant, scope).await?, tenant, scope)?;
    let input = authz::gather(
        state,
        tx,
        Some(&node),
        synveda_store::anchors::AnchorSelection::none(),
        Vec::new(),
    )
    .await?;
    let authorized = authz::decide(state, &input, Action::ChannelExport, Resource::Scope(scope))?;
    authz::decide(state, &input, Action::ChannelRead, Resource::Scope(scope))?;
    let configuration = configuration::effective_at_scope(tx, tenant, scope).await?;
    if !configuration
        .document
        .git_export_targets
        .iter()
        .any(|id| id == target)
    {
        return Err(Error::Invalid {
            message: "Git export destination is disabled by effective governed Configuration"
                .into(),
        });
    }
    let actor = input
        .identity
        .as_ref()
        .map(|i| i.id)
        .ok_or_else(|| Error::Invalid {
            message: "Git export requires a provisioned identity".into(),
        })?;
    Ok((authorized, actor, configuration))
}

fn require_provider(config: &EffectiveConfiguration, destination: Destination<'_>) -> Result<()> {
    if destination.provider() == "github"
        && !config.document.permits_provider(ExternalProvider::Github)
    {
        return Err(Error::Invalid {
            message: "GitHub export is disabled by effective governed Configuration".into(),
        });
    }
    Ok(())
}

async fn open_custody(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    scope: ScopeId,
    destination: Destination<'_>,
) -> Result<Custody> {
    let Some(id) = destination.secret_id()? else {
        return Ok(Custody {
            credential: None,
            revision: None,
        });
    };
    let unavailable = || Error::Invalid {
        message: "GitHub export destination or credential is unavailable".into(),
    };
    let stored = synveda_store::tenant_secrets::get(tx, tenant, id)
        .await?
        .ok_or_else(unavailable)?;
    if stored.tenant_id != tenant
        || stored.scope_id != scope
        || stored.kind != TenantSecretKind::ImportExport
        || stored.provider.as_deref() != Some("github")
        || stored.state != TenantSecretState::Active
    {
        return Err(unavailable());
    }
    let sealed = stored
        .sealed
        .as_deref()
        .filter(|bytes| bytes.len() <= 8192)
        .ok_or_else(unavailable)?;
    let opened = state
        .keys
        .opening_key(
            &state.pool,
            synveda_crypto::KeyScope::Tenant(tenant),
            sealed,
        )
        .await
        .map_err(|_| unavailable())?
        .open(
            synveda_crypto::Purpose::TenantSecret,
            synveda_crypto::RowKey::Uuid(id.as_uuid()),
            sealed,
        )
        .inspect_err(|_| {
            metrics::counter!(synveda_store::keys::KEY_OPEN_FAILURES_TOTAL,
            "scope" => "tenant", "purpose" => synveda_crypto::Purpose::TenantSecret.as_str())
            .increment(1);
        })
        .map_err(|_| unavailable())?;
    let credential = GitHubCredential::parse(
        &opened,
        destination.repository_id().ok_or_else(unavailable)?,
    )?;
    Ok(Custody {
        credential: Some(credential),
        revision: Some(stored.value_revision),
    })
}

async fn authorize_objects(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    snapshot: &Snapshot,
) -> Result<()> {
    let mut inputs = BTreeMap::new();
    for object in &snapshot.objects {
        let (scope, sensitivity) = match object.kind {
            AssetKind::Prompt => {
                let asset = flow::PromptAsset::from_bytes(object.content.as_bytes())
                    .map_err(|_| invalid_evidence())?;
                (asset.scope_id, asset.sensitivity)
            }
            AssetKind::ContextPack => {
                let asset = flow::ContextPackAsset::from_bytes(object.content.as_bytes())
                    .map_err(|_| invalid_evidence())?;
                (asset.scope_id, asset.sensitivity)
            }
            _ => return Err(invalid_evidence()),
        };
        if let std::collections::btree_map::Entry::Vacant(entry) = inputs.entry(scope) {
            let node = found(scopes::get(&mut *tx, tenant, scope).await?, tenant, scope)?;
            let input = authz::gather(
                state,
                tx,
                Some(&node),
                synveda_store::anchors::AnchorSelection::none(),
                Vec::new(),
            )
            .await?;
            entry.insert(input);
        }
        let input = inputs.get(&scope).ok_or_else(invalid_evidence)?;
        match object.kind {
            AssetKind::Prompt => {
                authz::decide_prompt_read(state, input, Resource::Scope(scope), sensitivity)?
            }
            AssetKind::ContextPack => {
                authz::decide_context_pack_read(state, input, Resource::Scope(scope), sensitivity)?
            }
            _ => return Err(invalid_evidence()),
        };
    }
    Ok(())
}

async fn source_ref(
    tx: &mut PgConnection,
    tenant: TenantId,
    scope: ScopeId,
    channel: ChannelRef,
) -> Result<RefState> {
    let head = evidence::read_ref(tx, tenant, scope, &channel.name())
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: "authored channel".into(),
        })?;
    Ok(RefState {
        head: CommitHash::from_slice(&head.commit_hash)?.to_hex(),
        updated_at: head.updated_at,
        updated_by: head.updated_by,
        pin: head
            .pin
            .map(|p| {
                Ok(PinEvidence {
                    commit: CommitHash::from_slice(&p.commit_hash)?.to_hex(),
                    pinned_at: p.pinned_at,
                    pinned_by: p.pinned_by,
                })
            })
            .transpose()?,
    })
}

async fn capture(
    tx: &mut PgConnection,
    tenant: TenantId,
    scope: ScopeId,
    target: &str,
    channel: ChannelRef,
    source: RefState,
) -> Result<Snapshot> {
    let mut commits = BTreeMap::new();
    let mut trees = BTreeMap::new();
    let mut objects = BTreeMap::new();
    let mut bytes = 0usize;
    let mut todo = VecDeque::from([source.head.clone()]);
    if let Some(pin) = &source.pin {
        todo.push_back(pin.commit.clone());
    }
    let mut queued: BTreeSet<_> = todo.iter().cloned().collect();
    while let Some(hash) = todo.pop_front() {
        if commits.contains_key(&hash) {
            continue;
        }
        if commits.len() >= synveda_git::MAX_COMMITS {
            return Err(bound());
        }
        let commit_hash: CommitHash = hash.parse().map_err(|_| invalid_evidence())?;
        let stored = evidence::read_commit(
            tx,
            tenant,
            commit_hash.as_slice(),
            synveda_git::MAX_COMMITS as i64,
        )
        .await?
        .ok_or_else(invalid_evidence)?;
        let tree = flow::TreeHash::from_slice(&stored.tree_hash)?.to_hex();
        let parents: Vec<_> = stored
            .parents
            .iter()
            .map(|p| CommitHash::from_slice(p).map(|h| h.to_hex()))
            .collect::<Result<_>>()?;
        for parent in &parents {
            if queued.insert(parent.clone()) {
                if queued.len() > synveda_git::MAX_COMMITS {
                    return Err(bound());
                }
                todo.push_back(parent.clone());
            }
        }
        if !trees.contains_key(&tree) {
            let entries = evidence::read_tree(
                tx,
                tenant,
                &stored.tree_hash,
                synveda_git::MAX_ENTRIES as i64,
            )
            .await?;
            let mut members = Vec::new();
            for entry in entries {
                if entry.subtree_hash.is_some() {
                    return Err(bound());
                }
                let object_hash =
                    flow::ObjectHash::from_slice(&entry.object_hash.ok_or_else(invalid_evidence)?)?;
                let object_id = object_hash.to_hex();
                if !objects.contains_key(&object_id) {
                    if objects.len() >= synveda_git::MAX_OBJECTS {
                        return Err(bound());
                    }
                    let (kind, content) = evidence::read_object(
                        tx,
                        tenant,
                        object_hash.as_slice(),
                        synveda_git::MAX_BYTES.saturating_sub(bytes) as i64,
                    )
                    .await?
                    .ok_or_else(invalid_evidence)?;
                    bytes += content.len();
                    objects.insert(
                        object_id.clone(),
                        ObjectEvidence {
                            hash: object_id.clone(),
                            kind: kind.parse().map_err(|_| invalid_evidence())?,
                            content: String::from_utf8(content).map_err(|_| invalid_evidence())?,
                        },
                    );
                }
                members.push(EntryEvidence {
                    name: entry.name,
                    object: object_id,
                });
            }
            trees.insert(
                tree.clone(),
                TreeEvidence {
                    hash: tree.clone(),
                    entries: members,
                },
            );
        }
        commits.insert(
            hash.clone(),
            CommitEvidence {
                hash,
                tree,
                parents,
                author: stored.author_id,
                message: stored.message,
                committed_at: stored.committed_at,
                policy_snapshot_hash: flow::PolicySnapshotHash::from_slice(
                    &stored.policy_snapshot_hash,
                )?
                .to_hex(),
                signature: stored.signature,
                signer_key_id: stored.signer_key_id,
            },
        );
    }
    let snapshot = Snapshot {
        tenant,
        scope,
        target: target.into(),
        asset: channel.asset,
        channel: channel.channel,
        head: source.head,
        updated_at: source.updated_at,
        updated_by: source.updated_by,
        pin: source.pin,
        commits: commits.into_values().collect(),
        trees: trees.into_values().collect(),
        objects: objects.into_values().collect(),
    };
    snapshot.validate().map_err(|_| bound())?;
    Ok(snapshot)
}

fn bound() -> Error {
    Error::Invalid {
        message: "Git export source is invalid, unsupported or exceeds its history/size bounds"
            .into(),
    }
}

async fn read_cursor(
    tx: &mut PgConnection,
    tenant: TenantId,
    scope: ScopeId,
    name: &str,
) -> Result<Option<(CommitHash, Cursor)>> {
    let Some(reference) = flow::read_ref(tx, tenant, scope, name).await? else {
        return Ok(None);
    };
    let stored = flow::read_commit(tx, tenant, reference.commit_hash)
        .await?
        .ok_or_else(invalid_evidence)?;
    let entries = flow::read_tree(tx, tenant, stored.tree)
        .await?
        .ok_or_else(invalid_evidence)?;
    if stored.hash != flow::verify::recompute_commit(&stored)
        || entries.len() != 1
        || entries[0].name != "export-cursor.json"
        || flow::verify::recompute_tree(&entries) != stored.tree
    {
        return Err(invalid_evidence());
    }
    let flow::TreeTarget::Object(hash) = entries[0].target else {
        return Err(invalid_evidence());
    };
    let object = flow::read_object(tx, tenant, hash)
        .await?
        .ok_or_else(invalid_evidence)?;
    if object.kind != AssetKind::Configuration
        || flow::hash::object_hash(object.kind, &object.content) != hash
    {
        return Err(invalid_evidence());
    }
    let cursor: Cursor = serde_json::from_slice(&object.content).map_err(|_| invalid_evidence())?;
    if cursor.format != "synveda-git-cursor-v2" || cursor.destination_digest.len() != 64 {
        return Err(invalid_evidence());
    }
    Ok(Some((stored.hash, cursor)))
}

#[allow(clippy::too_many_arguments)]
async fn save_cursor(
    tx: &mut PgConnection,
    tenant: TenantId,
    scope: ScopeId,
    name: &str,
    previous: Option<CommitHash>,
    cursor: &Cursor,
    actor: IdentityId,
    authorized: &authz::Authorized,
) -> Result<CommitHash> {
    let bytes = serde_json::to_vec(cursor).map_err(|_| invalid_evidence())?;
    let object = flow::put_object(tx, tenant, AssetKind::Configuration, &bytes).await?;
    let tree = flow::put_tree(
        tx,
        tenant,
        &[flow::TreeEntry::object("export-cursor.json", object.hash)],
    )
    .await?;
    let receipt = flow::commit(
        tx,
        tenant,
        &flow::NewCommit {
            tree: tree.hash,
            parents: previous.into_iter().collect(),
            author: actor,
            message: "Record governed Git export receipt".into(),
            committed_at: Utc::now(),
            policy_snapshot: PolicySnapshot::new(
                authorized.decision.pack_name.clone(),
                authorized.decision.pack_version,
            ),
        },
        &Signer::Unsigned,
    )
    .await?;
    let updated = match previous {
        Some(previous) => {
            flow::update_ref(tx, tenant, scope, name, previous, receipt.hash, actor).await?
        }
        None => flow::create_ref(tx, tenant, scope, name, receipt.hash, actor).await?,
    };
    if updated != flow::RefUpdate::Updated {
        return Err(conflict());
    }
    Ok(receipt.hash)
}

#[allow(clippy::too_many_arguments)]
async fn record(
    tx: &mut PgConnection,
    tenant: TenantId,
    scope: ScopeId,
    projection: &Projection,
    authorized: &authz::Authorized,
    config_hash: &str,
    destination: &DestinationEvidence,
    phase: &str,
    outcome: Outcome,
) -> Result<()> {
    let source = &projection.manifest.snapshot;
    audit::record(
        tx,
        tenant,
        AuditAction::GitExported,
        Resource::Scope(scope).to_string(),
        outcome,
        json!({
            "authz": audit::decision_context(Action::ChannelExport, authorized),
            "configuration_hash": config_hash, "target_id": source.target,
            "provider": destination.provider, "destination_digest": destination.digest,
            "repository_id": destination.repository_id, "secret_id": destination.secret_id,
            "secret_value_revision": destination.secret_value_revision,
            "asset": source.asset.as_str(), "channel": source.channel.as_str(),
            "source_head": source.head, "source_pin": source.pin.as_ref().map(|p| &p.commit),
            "git_head": projection.head, "mapping_digest": projection.manifest.mapping_digest,
            "phase": phase,
        }),
    )
    .await?;
    Ok(())
}

fn response(
    projection: &Projection,
    destination: &DestinationEvidence,
    outcome: &str,
) -> GitExportResponse {
    let source = &projection.manifest.snapshot;
    GitExportResponse {
        target: source.target.clone(),
        provider: destination.provider.into(),
        destination_digest: destination.digest.clone(),
        repository_id: destination.repository_id,
        source_head: source.head.clone(),
        source_pin: source.pin.as_ref().map(|p| p.commit.clone()),
        git_ref: source.git_ref(),
        git_head: projection.head.clone(),
        mapping_digest: projection.manifest.mapping_digest.clone(),
        commits: source.commits.len(),
        objects: source.objects.len(),
        outcome: outcome.into(),
    }
}
