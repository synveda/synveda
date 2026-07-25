//! Channels: branches with meaning (tech plan §2.2; FLOW-2, ADR-0031).
//!
//! FLOW-1 left the ref vocabulary open because a CHECK constraint written
//! then would have been a guess. This module fixes it: a channel is a
//! [`crate::refs`] row named `{asset-kind}/{channel}` — `memory/derived`,
//! `memory/published`, `memory/staged`, and the same three per asset type
//! as those types arrive. Nothing is created up front; a ref materialises
//! on its first write, so a scope with nothing published has no published
//! ref and reading it returns the empty set (ADR-0031 decision 2).
//!
//! # Two shapes, on purpose
//!
//! - **Set channels** (`published`, `staged`): each commit's tree is the
//!   channel's *entire* membership, so "what is published here" is one
//!   read of one tree. Written by [`publish`].
//! - **The log channel** (`derived`): each commit's tree holds only what
//!   that commit added, and the history is the parent chain. Written by
//!   [`append`].
//!
//! The asymmetry is cost, and it is safe because derived membership is
//! never enumerated: a record is derived material unless it is published
//! (ADR-0031 decisions 3 and 4). A full-membership tree per derived commit
//! would cost one `vedaflow_tree_entries` row per record in the corpus on
//! every extraction batch.
//!
//! # Publication binds bytes
//!
//! A set channel's tree entry is `<name> → the object address of exactly
//! the content that was reviewed`. A reader recomputes that address from
//! the content it is about to serve and requires a match, so editing
//! published content demotes it to unreviewed rather than laundering the
//! edit through a published id (ADR-0031 decision 5).

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::PgConnection;
use synveda_types::{
    AssetKind, Channel, Error, IdentityId, RecordClass, RecordId, RecordKind, Result, ScopeId,
    Sensitivity, TenantId,
};
use uuid::Uuid;

use crate::commits::{NewCommit, commit};
use crate::hash::{CommitHash, ObjectHash, canonical_timestamp, object_hash};
use crate::objects::put_object;
use crate::policy::{PolicySnapshot, canonical_json};
use crate::refs::{RefUpdate, create_ref, read_ref, update_ref};
use crate::signer::CommitSigner;
use crate::trees::{TreeEntry, put_tree};
use crate::{Written, storage_error};

/// Counts channel commits, labelled by asset kind, channel, and outcome.
pub const CHANNEL_COMMITS_TOTAL: &str = "synveda_vedaflow_channel_commits_total";

/// The largest membership a set channel accepts at one scope.
///
/// A reviewed constant on the [`crate::MAX_OBJECT_BYTES`] precedent, not a
/// tuning knob: a published set is something a curator stands behind, and
/// ten thousand of them is already far past what anyone reviews. Subtree
/// sharding is the recorded upgrade (ADR-0031 reversal trigger a).
pub const MAX_CHANNEL_MEMBERS: usize = 10_000;

/// Compare-and-swap attempts before a channel write gives up.
///
/// Racing is a result, not an error (ADR-0030 decision 10), and the retry
/// re-reads the head and re-parents. Three is bounded rather than
/// generous on purpose: same-tenant write transactions already serialise
/// at the audit chain head (ADR-0019 decision 1), so sustained contention
/// on one channel means something is wrong, not busy.
const MAX_ATTEMPTS: u32 = 3;

/// One scope's channel of one asset type — the two halves of a ref name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChannelRef {
    /// Which asset type's channel.
    pub asset: AssetKind,
    /// Which channel.
    pub channel: Channel,
}

impl ChannelRef {
    /// The channel of `asset` named `channel`.
    #[must_use]
    pub const fn new(asset: AssetKind, channel: Channel) -> Self {
        ChannelRef { asset, channel }
    }

    /// The `memory/{channel}` ref — the only asset type with a writer
    /// today (PRMT-1/2 and SKIL-1 bring the rest).
    #[must_use]
    pub const fn memory(channel: Channel) -> Self {
        ChannelRef::new(AssetKind::Memory, channel)
    }

    /// The ref name this channel is stored under.
    #[must_use]
    pub fn name(&self) -> String {
        format!("{}/{}", self.asset.as_str(), self.channel.as_str())
    }

    /// Whether the channel's tree is its whole membership (`published`,
    /// `staged`) rather than one commit's additions (`derived`).
    #[must_use]
    pub const fn is_set(&self) -> bool {
        matches!(self.channel, Channel::Published | Channel::Staged)
    }
}

impl fmt::Display for ChannelRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name())
    }
}

impl FromStr for ChannelRef {
    type Err = Error;

    /// Parses a stored ref name. A name that is not a channel — FLOW-3's
    /// proposal refs, FLOW-7's pins — is [`Error::Invalid`], not a
    /// channel with odd halves.
    fn from_str(name: &str) -> Result<Self> {
        let (asset, channel) = name.split_once('/').ok_or_else(|| Error::Invalid {
            message: format!("not a channel ref name: {name:?}"),
        })?;
        Ok(ChannelRef::new(asset.parse()?, channel.parse()?))
    }
}

/// One entry of a channel's tree: a name and the content address it was
/// admitted at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelMember {
    /// The entry name — a record id for memories, a path for the authored
    /// asset types.
    pub name: String,
    /// The address of exactly the content that was admitted.
    pub object: ObjectHash,
}

/// One scope's channel as of now: where the ref points, and what the
/// commit it points at holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSnapshot {
    /// The scope whose channel this is.
    pub scope_id: ScopeId,
    /// The commit the channel points at — what the block cites and the
    /// audit event records (ADR-0031 decision 11).
    pub commit: CommitHash,
    /// The commit's tree entries, in name order. For a set channel that
    /// is the whole membership; for the derived log it is the last
    /// commit's additions.
    pub members: Vec<ChannelMember>,
}

/// One scope's memory channel, keyed the way composition reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryChannel {
    /// The scope whose channel this is.
    pub scope_id: ScopeId,
    /// The commit the channel points at.
    pub commit: CommitHash,
    /// Record id → the address that scope admitted it at.
    pub members: HashMap<RecordId, ObjectHash>,
}

/// One scope's channel as it stands: what it points at, and how big the
/// commit it points at is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelStatus {
    /// Which channel.
    pub channel: ChannelRef,
    /// Where it points.
    pub commit: CommitHash,
    /// When it last moved.
    pub updated_at: DateTime<Utc>,
    /// Who last moved it.
    pub updated_by: IdentityId,
    /// Entries in the head commit's tree. For a set channel that is the
    /// membership; for the derived log it is what the last commit added.
    pub entries: usize,
}

/// A channel write, as the caller describes it.
#[derive(Debug, Clone)]
pub struct ChannelWrite<'a> {
    /// The scope whose channel moves.
    pub scope: ScopeId,
    /// Which channel.
    pub channel: ChannelRef,
    /// What this write contributes: the members to add.
    pub members: &'a [(String, ObjectHash)],
    /// Additional parents beyond the channel head — the proposal commit a
    /// publication is the effect of (FLOW-3, ADR-0032 decision 10). The
    /// head stays the first parent, so the channel's own line is
    /// unbroken and the fast-forward check is unaffected; the extras
    /// make lineage a fact about the commit graph rather than a join.
    /// Empty for a publication nobody proposed.
    pub merge_parents: &'a [CommitHash],
    /// Who is writing.
    pub author: IdentityId,
    /// Why — an auditor reads this.
    pub message: &'a str,
    /// When, in valid-time terms.
    pub committed_at: DateTime<Utc>,
    /// The pack in force, as the caller resolved it. Vedaflow cannot ask
    /// the PDP anything (ADR-0030 decision 1); the caller passes what it
    /// already decided with.
    pub policy_snapshot: &'a PolicySnapshot,
}

/// What a channel write did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelCommit {
    /// The commit the channel now points at.
    pub commit: CommitHash,
    /// What it pointed at before — `None` when the ref did not exist.
    pub parent: Option<CommitHash>,
    /// Entries in the new commit's tree.
    pub entries: usize,
    /// Members this write added that the channel did not already hold at
    /// that address. Zero means the act happened and changed nothing —
    /// which is a fact an auditor should see, not one to hide by
    /// refusing to commit.
    pub added: usize,
    /// Compare-and-swaps it took. More than one means a writer raced.
    pub attempts: u32,
}

/// Adds `members` to a set channel: the new tree is the union of the
/// current membership and the write's members, with the write's address
/// winning where a name appears in both (re-publishing is how edited
/// content is re-admitted).
///
/// Additive by construction — retraction is a rewind, and rewinds are
/// [`crate::force_update_ref`] by name (FLOW-7).
pub async fn publish(
    conn: &mut PgConnection,
    tenant: TenantId,
    write: &ChannelWrite<'_>,
    signer: &impl CommitSigner,
) -> Result<ChannelCommit> {
    if !write.channel.is_set() {
        return Err(Error::Invalid {
            message: format!(
                "{} is a log channel; use append (ADR-0031 decision 3)",
                write.channel
            ),
        });
    }
    write_channel(conn, tenant, write, signer, true).await
}

/// Appends `members` to a log channel: the new tree holds exactly this
/// write's members, parented on the channel's history.
pub async fn append(
    conn: &mut PgConnection,
    tenant: TenantId,
    write: &ChannelWrite<'_>,
    signer: &impl CommitSigner,
) -> Result<ChannelCommit> {
    if write.channel.is_set() {
        return Err(Error::Invalid {
            message: format!(
                "{} is a set channel; use publish (ADR-0031 decision 3)",
                write.channel
            ),
        });
    }
    if write.members.is_empty() {
        return Err(Error::Invalid {
            message: "a log-channel commit with no members records nothing".to_owned(),
        });
    }
    write_channel(conn, tenant, write, signer, false).await
}

/// The compare-and-swap loop both shapes share.
///
/// Read the head, build the tree, mint the commit parented on that head,
/// swap. A writer that slipped in makes the swap affect zero rows; the
/// retry re-reads and re-parents, and re-mints only the commit — the
/// objects and the tree are content-addressed and parent-independent, so
/// they survive untouched (and dedup, so the retry writes no new rows for
/// them).
#[tracing::instrument(
    name = "vedaflow.channel_write",
    skip_all,
    fields(
        tenant.id = %tenant,
        scope.id = %write.scope,
        vedaflow.channel = %write.channel,
        vedaflow.members = write.members.len(),
        vedaflow.attempts = tracing::field::Empty,
        vedaflow.commit = tracing::field::Empty,
    ),
    err(Display)
)]
async fn write_channel(
    conn: &mut PgConnection,
    tenant: TenantId,
    write: &ChannelWrite<'_>,
    signer: &impl CommitSigner,
    union_with_head: bool,
) -> Result<ChannelCommit> {
    let name = write.channel.name();
    for attempt in 1..=MAX_ATTEMPTS {
        let head = read_ref(conn, tenant, write.scope, &name).await?;
        let existing = match (&head, union_with_head) {
            (Some(head), true) => tree_of(conn, tenant, head.commit_hash).await?,
            _ => Vec::new(),
        };

        let mut entries: HashMap<&str, ObjectHash> = existing
            .iter()
            .map(|(entry_name, hash)| (entry_name.as_str(), *hash))
            .collect();
        let mut added = 0_usize;
        for (member, hash) in write.members {
            if entries.insert(member.as_str(), *hash) != Some(*hash) {
                added += 1;
            }
        }
        if entries.len() > MAX_CHANNEL_MEMBERS {
            return Err(Error::Invalid {
                message: format!(
                    "{} at this scope would hold {} members, over the \
                     {MAX_CHANNEL_MEMBERS} limit (ADR-0031 decision 3)",
                    write.channel,
                    entries.len()
                ),
            });
        }

        let tree_entries: Vec<TreeEntry> = entries
            .into_iter()
            .map(|(entry_name, hash)| TreeEntry::object(entry_name, hash))
            .collect();
        let entry_count = tree_entries.len();
        let tree = put_tree(conn, tenant, &tree_entries).await?;
        // Head first (the mainline), then the merge parents, with any
        // that duplicate the head or each other dropped: a commit listing
        // one parent twice is meaningless and the table rejects it.
        let mut parents: Vec<CommitHash> = head.iter().map(|head| head.commit_hash).collect();
        for parent in write.merge_parents {
            if !parents.contains(parent) {
                parents.push(*parent);
            }
        }
        let minted = commit(
            conn,
            tenant,
            &NewCommit {
                tree: tree.hash,
                parents,
                author: write.author,
                message: write.message.to_owned(),
                committed_at: write.committed_at,
                policy_snapshot: write.policy_snapshot.clone(),
            },
            signer,
        )
        .await?;

        let outcome = match &head {
            None => create_ref(conn, tenant, write.scope, &name, minted.hash, write.author).await?,
            Some(head) => {
                update_ref(
                    conn,
                    tenant,
                    write.scope,
                    &name,
                    head.commit_hash,
                    minted.hash,
                    write.author,
                )
                .await?
            }
        };
        if outcome == RefUpdate::Updated {
            let span = tracing::Span::current();
            span.record("vedaflow.attempts", attempt);
            span.record("vedaflow.commit", minted.hash.to_hex());
            record(write.channel, "committed");
            return Ok(ChannelCommit {
                commit: minted.hash,
                parent: head.map(|head| head.commit_hash),
                entries: entry_count,
                added,
                attempts: attempt,
            });
        }
        // Raced (or, for a first write, someone created the ref first).
        // NotFastForward cannot happen: the commit was parented on the
        // head this iteration read.
        tracing::debug!(
            channel = %write.channel,
            attempt,
            "channel ref moved under us; re-reading and re-parenting"
        );
    }

    record(write.channel, "contended");
    Err(Error::Conflict {
        message: format!(
            "{} at this scope moved under {MAX_ATTEMPTS} attempts; retry the operation",
            write.channel
        ),
    })
}

/// One channel across `scopes`, in `(scope, name)` order — where each
/// scope's ref points and what that commit holds.
///
/// One indexed query for the whole scope chain: refs by their primary-key
/// prefix, the commit by its primary key, the tree's entries by theirs.
/// This is what the inject path runs, so it stays one round trip
/// (ADR-0031 decision 3). Scopes with no such ref are simply absent —
/// nothing is published there, which is the answer, not a gap.
#[tracing::instrument(
    name = "vedaflow.read_members",
    skip_all,
    fields(tenant.id = %tenant, scopes.count = scopes.len(), vedaflow.channel = %channel),
    err(Display)
)]
pub async fn read_members(
    conn: &mut PgConnection,
    tenant: TenantId,
    scopes: &[ScopeId],
    channel: ChannelRef,
) -> Result<Vec<ChannelSnapshot>> {
    if scopes.is_empty() {
        return Ok(Vec::new());
    }
    let scope_ids: Vec<Uuid> = scopes.iter().map(ScopeId::as_uuid).collect();
    // Left join on the entries: a ref pointing at an empty tree is a
    // channel that exists and holds nothing, which is not the same fact
    // as a channel that was never written.
    let rows = sqlx::query!(
        // The `?` overrides are the left join: sqlx reads nullability off
        // the column declaration, which says NOT NULL on the entry side.
        r#"select r.scope_id, r.commit_hash,
                  e.name as "name?", e.object_hash as "object_hash?"
           from vedaflow_refs r
           join vedaflow_commits c
               on c.tenant_id = r.tenant_id and c.hash = r.commit_hash
           left join vedaflow_tree_entries e
               on e.tenant_id = c.tenant_id and e.tree_hash = c.tree_hash
           where r.tenant_id = $1 and r.scope_id = any($2) and r.name = $3
           order by r.scope_id, e.name"#,
        tenant.as_uuid(),
        &scope_ids,
        channel.name(),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| storage_error("read channel members", &err))?;

    let mut snapshots: Vec<ChannelSnapshot> = Vec::new();
    for row in rows {
        let scope_id = ScopeId::from_uuid(row.scope_id);
        let commit = CommitHash::from_slice(&row.commit_hash)?;
        if snapshots
            .last()
            .is_none_or(|last| last.scope_id != scope_id)
        {
            snapshots.push(ChannelSnapshot {
                scope_id,
                commit,
                members: Vec::new(),
            });
        }
        let Some(name) = row.name else { continue };
        // Set channels hold only object entries today. A subtree here
        // means the sharding upgrade landed without this reader — schema
        // and code drift, which is our bug, not a member to skip quietly.
        let Some(object) = row.object_hash else {
            return Err(Error::Internal {
                message: format!(
                    "channel {channel} entry {name:?} points at a subtree; \
                     this reader predates sharding"
                ),
            });
        };
        snapshots
            .last_mut()
            .expect("a snapshot was pushed for this scope")
            .members
            .push(ChannelMember {
                name,
                object: ObjectHash::from_slice(&object)?,
            });
    }
    Ok(snapshots)
}

/// [`read_members`] for the memory channel, keyed the way composition
/// reads it: record id → the address that scope admitted.
///
/// Entry names that are not record ids cannot occur — only this crate
/// writes memory channels, and it names entries by id — so one is an
/// internal error rather than a member to drop.
pub async fn read_memory_members(
    conn: &mut PgConnection,
    tenant: TenantId,
    scopes: &[ScopeId],
    channel: Channel,
) -> Result<Vec<MemoryChannel>> {
    read_members(conn, tenant, scopes, ChannelRef::memory(channel))
        .await?
        .into_iter()
        .map(|snapshot| {
            let members = snapshot
                .members
                .into_iter()
                .map(|member| {
                    let id = member.name.parse::<Uuid>().map_err(|err| Error::Internal {
                        message: format!(
                            "memory channel entry {:?} is not a record id: {err}",
                            member.name
                        ),
                    })?;
                    Ok((RecordId::from_uuid(id), member.object))
                })
                .collect::<Result<HashMap<RecordId, ObjectHash>>>()?;
            Ok(MemoryChannel {
                scope_id: snapshot.scope_id,
                commit: snapshot.commit,
                members,
            })
        })
        .collect()
}

/// Every channel that exists at one scope, in ref-name order, with the
/// size of the commit each points at.
///
/// Ref names that are not channels are skipped: `vedaflow_refs` is
/// deliberately generic (ADR-0031 decision 1), and FLOW-3's proposal refs
/// will share the table.
#[tracing::instrument(
    name = "vedaflow.channel_status",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope),
    err(Display)
)]
pub async fn status(
    conn: &mut PgConnection,
    tenant: TenantId,
    scope: ScopeId,
) -> Result<Vec<ChannelStatus>> {
    let rows = sqlx::query!(
        r#"select r.name, r.commit_hash, r.updated_at, r.updated_by,
                  (select count(*) from vedaflow_tree_entries e
                   where e.tenant_id = r.tenant_id and e.tree_hash = c.tree_hash)
                      as "entries!"
           from vedaflow_refs r
           join vedaflow_commits c
               on c.tenant_id = r.tenant_id and c.hash = r.commit_hash
           where r.tenant_id = $1 and r.scope_id = $2
           order by r.name"#,
        tenant.as_uuid(),
        scope.as_uuid(),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| storage_error("read channel status", &err))?;

    let mut statuses = Vec::with_capacity(rows.len());
    for row in rows {
        let Ok(channel) = row.name.parse::<ChannelRef>() else {
            continue;
        };
        statuses.push(ChannelStatus {
            channel,
            commit: CommitHash::from_slice(&row.commit_hash)?,
            updated_at: row.updated_at,
            updated_by: IdentityId::from_uuid(row.updated_by),
            entries: usize::try_from(row.entries).unwrap_or(usize::MAX),
        });
    }
    Ok(statuses)
}

/// A memory record as VedaFlow addresses it (ADR-0031 decision 6).
///
/// The governed fields and nothing else: `provenance` is outside because
/// it carries a float (`confidence`) and a float has no canonical form
/// worth hashing, and `tx_from` is outside because this is a *content*
/// address — the same content at the same scope is the same object
/// however many times the bitemporal pair rewrites around it. `valid_to`
/// is inside, so closing a record's window changes its address and drops
/// it off any published set that admitted the open version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryAsset {
    /// The record.
    pub id: RecordId,
    /// The scope it attaches to.
    pub scope_id: ScopeId,
    /// Who owns it.
    pub owner_id: IdentityId,
    /// Authored/canonical or pipeline-derived (seed §4.2). Authorship,
    /// not channel — that is what FLOW-2 separated.
    pub kind: RecordKind,
    /// What it asserts.
    pub class: RecordClass,
    /// The persisted (post-redaction) text.
    pub content: String,
    /// Its classification.
    pub sensitivity: Sensitivity,
    /// Valid-time window start.
    pub valid_from: DateTime<Utc>,
    /// Valid-time window end, when closed.
    pub valid_to: Option<DateTime<Utc>>,
}

impl MemoryAsset {
    /// The object's bytes: canonical JSON, keys sorted bytewise,
    /// timestamps in the one rendering this crate uses everywhere.
    ///
    /// Human-readable on purpose — FLOW-6 renders diffs of it and FLOW-8
    /// exports it into a real git repository, where a length-prefixed
    /// binary blob would be worthless.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let value = json!({
            "class": self.class.as_str(),
            "content": self.content,
            "id": self.id.as_uuid().to_string(),
            "kind": self.kind.as_str(),
            "owner": self.owner_id.as_uuid().to_string(),
            "scope": self.scope_id.as_uuid().to_string(),
            "sensitivity": self.sensitivity.as_str(),
            "valid_from": canonical_timestamp(self.valid_from),
            "valid_to": self.valid_to.map(canonical_timestamp),
        });
        let mut out = String::with_capacity(self.content.len() + 256);
        // Every value here is a string, a null, or an object of those:
        // the float rejection cannot fire, and the expect says so.
        canonical_json(&value, &mut out).expect("a memory asset contains no numbers");
        out.into_bytes()
    }

    /// The content address — computable without touching the database,
    /// which is what lets composition check a published entry against the
    /// version it is about to serve for the cost of a hash.
    #[must_use]
    pub fn address(&self) -> ObjectHash {
        object_hash(AssetKind::Memory, &self.canonical_bytes())
    }

    /// The name this asset takes in a channel tree.
    #[must_use]
    pub fn entry_name(&self) -> String {
        self.id.as_uuid().to_string()
    }
}

/// Writes a memory's object, returning its address.
///
/// Dedups like every other object write: re-committing unchanged content
/// stores nothing and returns the same address.
pub async fn put_memory(
    conn: &mut PgConnection,
    tenant: TenantId,
    asset: &MemoryAsset,
) -> Result<Written<ObjectHash>> {
    put_object(conn, tenant, AssetKind::Memory, &asset.canonical_bytes()).await
}

/// A commit's tree as `(name, object address)` pairs.
async fn tree_of(
    conn: &mut PgConnection,
    tenant: TenantId,
    commit: CommitHash,
) -> Result<Vec<(String, ObjectHash)>> {
    let rows = sqlx::query!(
        "select e.name, e.object_hash
         from vedaflow_commits c
         join vedaflow_tree_entries e
             on e.tenant_id = c.tenant_id and e.tree_hash = c.tree_hash
         where c.tenant_id = $1 and c.hash = $2
         order by e.name",
        tenant.as_uuid(),
        commit.as_slice(),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| storage_error("read channel head tree", &err))?;

    rows.into_iter()
        .map(|row| {
            let Some(object) = row.object_hash else {
                return Err(Error::Internal {
                    message: format!(
                        "channel head entry {:?} points at a subtree; \
                         this writer predates sharding",
                        row.name
                    ),
                });
            };
            Ok((row.name, ObjectHash::from_slice(&object)?))
        })
        .collect()
}

fn record(channel: ChannelRef, outcome: &'static str) {
    metrics::counter!(
        CHANNEL_COMMITS_TOTAL,
        "asset" => channel.asset.as_str(),
        "channel" => channel.channel.as_str(),
        "outcome" => outcome,
    )
    .increment(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn asset(content: &str) -> MemoryAsset {
        MemoryAsset {
            id: RecordId::from_uuid(Uuid::from_bytes([1; 16])),
            scope_id: ScopeId::from_uuid(Uuid::from_bytes([2; 16])),
            owner_id: IdentityId::from_uuid(Uuid::from_bytes([3; 16])),
            kind: RecordKind::Derived,
            class: RecordClass::Fact,
            content: content.to_owned(),
            sensitivity: Sensitivity::Internal,
            valid_from: Utc.with_ymd_and_hms(2026, 7, 25, 9, 0, 0).unwrap(),
            valid_to: None,
        }
    }

    #[test]
    fn ref_names_round_trip_through_the_vocabulary() {
        for asset in AssetKind::ALL {
            for channel in Channel::ALL {
                let reference = ChannelRef::new(asset, channel);
                assert_eq!(reference.name().parse::<ChannelRef>().unwrap(), reference);
            }
        }
        assert_eq!(
            ChannelRef::memory(Channel::Published).name(),
            "memory/published"
        );
    }

    /// `vedaflow_refs` is generic on purpose: FLOW-3's proposal refs and
    /// FLOW-7's pins share the table, and neither is a channel.
    #[test]
    fn names_that_are_not_channels_do_not_parse_into_one() {
        for name in [
            "published",
            "memory",
            "memory/review",
            "notes/published",
            "",
        ] {
            assert!(
                name.parse::<ChannelRef>().is_err(),
                "{name:?} must not parse as a channel"
            );
        }
    }

    #[test]
    fn set_and_log_channels_are_distinguished_by_shape_not_by_name() {
        assert!(ChannelRef::memory(Channel::Published).is_set());
        assert!(ChannelRef::memory(Channel::Staged).is_set());
        assert!(!ChannelRef::memory(Channel::Derived).is_set());
    }

    #[test]
    fn a_memory_object_is_canonical_json_with_sorted_keys() {
        let bytes = asset("be terse").canonical_bytes();
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(
            text,
            "{\"class\":\"fact\",\"content\":\"be terse\",\
             \"id\":\"01010101-0101-0101-0101-010101010101\",\"kind\":\"derived\",\
             \"owner\":\"03030303-0303-0303-0303-030303030303\",\
             \"scope\":\"02020202-0202-0202-0202-020202020202\",\
             \"sensitivity\":\"internal\",\"valid_from\":\"2026-07-25T09:00:00.000000Z\",\
             \"valid_to\":null}"
        );
    }

    /// The property decision 5 rests on: the address moves with the
    /// content, so an edited record no longer matches what was published.
    #[test]
    fn the_address_moves_with_every_governed_field() {
        let base = asset("be terse");
        assert_eq!(base.address(), asset("be terse").address(), "recomputable");
        assert_ne!(base.address(), asset("be verbose").address(), "content");

        let mut moved = base.clone();
        moved.scope_id = ScopeId::from_uuid(Uuid::from_bytes([9; 16]));
        assert_ne!(base.address(), moved.address(), "scope");

        let mut pinned = base.clone();
        pinned.kind = RecordKind::Pinned;
        assert_ne!(base.address(), pinned.address(), "kind");

        let mut reclassified = base.clone();
        reclassified.sensitivity = Sensitivity::Confidential;
        assert_ne!(base.address(), reclassified.address(), "sensitivity");

        let mut closed = base.clone();
        closed.valid_to = Some(Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap());
        assert_ne!(base.address(), closed.address(), "valid_to");
    }

    /// The object is a *content* address: two versions of the same text
    /// at the same scope share it however the bitemporal pair rewrites
    /// around them. Nothing outside the governed fields can move it.
    #[test]
    fn the_address_is_content_not_version() {
        let a = asset("same");
        let b = asset("same");
        assert_eq!(a.address(), b.address());
        // A memory's object never carries a number, so the canonical-JSON
        // float rejection cannot fire on this path.
        assert!(
            !String::from_utf8(a.canonical_bytes())
                .unwrap()
                .contains("confidence")
        );
    }
}
