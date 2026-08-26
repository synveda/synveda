//! Channels: branches with meaning (tech plan §2.2; FLOW-2, ADR-0031).
//!
//! FLOW-1 left the ref vocabulary open because a CHECK constraint written
//! then would have been a guess. This module fixes it: a channel is a
//! [`crate::refs`] row named `{asset-kind}/{channel}` for authored prompts
//! and context packs. Nothing is created up front; a ref materialises
//! on its first write, so a scope with nothing published has no published
//! ref and reading it returns the empty set (ADR-0031 decision 2).
//!
//! Every channel is a set: each commit's tree is the entire membership, so
//! "what is published here" is one read of one immutable tree.
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
use sqlx::PgConnection;
use synveda_types::{AssetKind, Channel, Error, IdentityId, Result, ScopeId, TenantId};
use uuid::Uuid;

use crate::commits::{
    MAX_FIRST_PARENT_WALK, NewCommit, commit, is_ancestor, is_first_parent_ancestor,
};
use crate::hash::{CommitHash, ObjectHash};
use crate::policy::PolicySnapshot;
use crate::refs::{RefUpdate, StoredRef, create_ref, force_update_ref, read_ref, update_ref};
use crate::signer::CommitSigner;
use crate::storage_error;
use crate::trees::{TreeEntry, put_tree};

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
        assert!(
            asset.has_channels(),
            "a VedaFlow channel requires a channelled asset kind"
        );
        ChannelRef { asset, channel }
    }

    /// The `prompt/{channel}` ref (PRMT-1, ADR-0049) — the second asset
    /// type with a writer, and the first whose entries are paths.
    #[must_use]
    pub const fn prompt(channel: Channel) -> Self {
        ChannelRef::new(AssetKind::Prompt, channel)
    }

    /// The `context-pack/{channel}` ref (PRMT-2, ADR-0050 decision 3) —
    /// the third asset type with a writer, and the first whose entries are
    /// *bundles*: one entry per document, named `pack/document`.
    #[must_use]
    pub const fn context_pack(channel: Channel) -> Self {
        ChannelRef::new(AssetKind::ContextPack, channel)
    }

    /// The ref name this channel is stored under.
    #[must_use]
    pub fn name(&self) -> String {
        format!("{}/{}", self.asset.as_str(), self.channel.as_str())
    }

    /// The ref name a pin on this channel takes.
    /// (FLOW-7, ADR-0036 decision 5).
    ///
    /// It cannot collide with a channel: [`FromStr`] splits on the first
    /// `/` and parses the halves, and `pin` is not an asset kind, so the
    /// same refusal that keeps FLOW-3's proposal refs out of the channel
    /// listing keeps pins out too.
    #[must_use]
    pub fn pin_name(&self) -> String {
        format!("{PIN_PREFIX}{}", self.name())
    }
}

/// The ref-name prefix that marks a pin. Migration 0021's delete policy and
/// trigger both key on it: a ref named `pin/…` may be released, and every
/// other ref is a channel pointer that never disappears.
pub const PIN_PREFIX: &str = "pin/";

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
        let asset = asset.parse::<AssetKind>()?;
        if !asset.has_channels() {
            return Err(Error::Invalid {
                message: format!("{} has no VedaFlow channels", asset.as_str()),
            });
        }
        Ok(ChannelRef::new(asset, channel.parse()?))
    }
}

/// One entry of a channel's tree: a name and the content address it was
/// admitted at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelMember {
    /// The authored member path.
    pub name: String,
    /// The address of exactly the content that was admitted.
    pub object: ObjectHash,
}

/// One scope's channel as of now: the commit it serves, and what that
/// commit holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSnapshot {
    /// The scope whose channel this is.
    pub scope_id: ScopeId,
    /// The commit the channel **serves** — what the block cites and the
    /// audit event records (ADR-0031 decision 11). Where the ref points,
    /// unless a pin holds it at an earlier state (ADR-0036 decision 6).
    pub commit: CommitHash,
    /// Whether that commit came from a pin rather than the ref. Rides the
    /// watermark, because a block that cites a frozen commit without
    /// saying so overstates its own freshness (ADR-0036 decision 10).
    pub pinned: bool,
    /// The commit's complete membership, in name order.
    pub members: Vec<ChannelMember>,
}

/// One scope's channel as it stands: what it points at, how big the
/// commit it points at is, and whether a pin holds its readers elsewhere.
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
    /// Entries in the head commit's complete membership tree.
    pub entries: usize,
    /// The standing pin, when there is one: publications keep landing on
    /// the head above, and readers keep composing this commit until it is
    /// released (ADR-0036 decision 6).
    pub pin: Option<ChannelPin>,
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
    write_channel(conn, tenant, write, signer, true).await
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

/// One channel across `scopes`, in `(scope, name)` order — the commit each
/// scope's channel serves and what that commit holds.
///
/// One indexed query for the whole scope chain: refs by their primary-key
/// prefix, the commit by its primary key, the tree's entries by theirs.
/// This is what the inject path runs, so it stays one round trip
/// (ADR-0031 decision 3). Scopes with no such ref are simply absent —
/// nothing is published there, which is the answer, not a gap.
///
/// Since FLOW-7 the same query left-joins the pin ref and coalesces, so a
/// pinned scope serves the commit it was held at and the read path gains
/// no round trip (ADR-0036 decision 6). A pin can only hold membership at
/// an earlier approved state, so it never widens what composes.
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
        // The `?` overrides are the left joins: sqlx reads nullability off
        // the column declaration, which says NOT NULL on the entry side —
        // and on the pin's, which is the same table joined to itself.
        r#"select r.scope_id,
                  coalesce(p.commit_hash, r.commit_hash) as "commit_hash!",
                  (p.commit_hash is not null) as "pinned!",
                  e.name as "name?", e.object_hash as "object_hash?"
           from vedaflow_refs r
           left join vedaflow_refs p
               on p.tenant_id = r.tenant_id and p.scope_id = r.scope_id
                  and p.name = $4
           join vedaflow_commits c
               on c.tenant_id = r.tenant_id
                  and c.hash = coalesce(p.commit_hash, r.commit_hash)
           left join vedaflow_tree_entries e
               on e.tenant_id = c.tenant_id and e.tree_hash = c.tree_hash
           where r.tenant_id = $1 and r.scope_id = any($2) and r.name = $3
           order by r.scope_id, e.name"#,
        tenant.as_uuid(),
        &scope_ids,
        channel.name(),
        channel.pin_name(),
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
                pinned: row.pinned,
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

/// Every scope whose `channel` ref exists at all — the published half of
/// a recall's candidate universe (CTX-5, ADR-0042 decision 2).
///
/// Deliberately an over-approximation: a ref pointing at an empty tree is
/// returned, because the question this answers is "could this scope
/// contribute", and the cost of a wrong *yes* is one PDP decision whose
/// verdict changes no result. A wrong *no* would silently drop material,
/// which is why nothing here filters on membership.
///
/// A ref materialises on its first write (this module's header), so a
/// tenant that has published nothing returns nothing rather than every
/// scope it has.
#[tracing::instrument(
    name = "vedaflow.scopes_with_channel",
    skip_all,
    fields(tenant.id = %tenant, channel = %channel, scopes = tracing::field::Empty),
    err(Display)
)]
pub async fn scopes_with_channel(
    conn: &mut PgConnection,
    tenant: TenantId,
    channel: ChannelRef,
) -> Result<Vec<ScopeId>> {
    let rows = sqlx::query_scalar!(
        r#"select scope_id as "scope_id!"
           from vedaflow_refs
           where tenant_id = $1 and name = $2
           order by scope_id"#,
        tenant.as_uuid(),
        channel.name(),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| storage_error("read channel scopes", &err))?;
    tracing::Span::current().record("scopes", rows.len());
    Ok(rows.into_iter().map(ScopeId::from_uuid).collect())
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
        // The pin is the same table joined to itself by the name its
        // channel derives; the `?` overrides are that left join.
        r#"select r.name, r.commit_hash, r.updated_at, r.updated_by,
                  p.commit_hash as "pinned_commit?",
                  p.updated_at as "pinned_at?",
                  p.updated_by as "pinned_by?",
                  (select count(*) from vedaflow_tree_entries e
                   where e.tenant_id = r.tenant_id and e.tree_hash = c.tree_hash)
                      as "entries!"
           from vedaflow_refs r
           join vedaflow_commits c
               on c.tenant_id = r.tenant_id and c.hash = r.commit_hash
           left join vedaflow_refs p
               on p.tenant_id = r.tenant_id and p.scope_id = r.scope_id
                  and p.name = $3 || r.name
           where r.tenant_id = $1 and r.scope_id = $2
           order by r.name"#,
        tenant.as_uuid(),
        scope.as_uuid(),
        PIN_PREFIX,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| storage_error("read channel status", &err))?;

    let mut statuses = Vec::with_capacity(rows.len());
    for row in rows {
        // Ref names that are not channels are skipped, which is also what
        // keeps the pins themselves off this listing: they ride their
        // channel's row instead.
        let Ok(channel) = row.name.parse::<ChannelRef>() else {
            continue;
        };
        // The three pin columns come from one left-joined row, so they are
        // present together or absent together.
        let pin = match (row.pinned_commit, row.pinned_at, row.pinned_by) {
            (Some(commit), Some(pinned_at), Some(pinned_by)) => Some(ChannelPin {
                scope_id: scope,
                channel,
                commit: CommitHash::from_slice(&commit)?,
                pinned_at,
                pinned_by: IdentityId::from_uuid(pinned_by),
            }),
            _ => None,
        };
        statuses.push(ChannelStatus {
            channel,
            commit: CommitHash::from_slice(&row.commit_hash)?,
            updated_at: row.updated_at,
            updated_by: IdentityId::from_uuid(row.updated_by),
            entries: usize::try_from(row.entries).unwrap_or(usize::MAX),
            pin,
        });
    }
    Ok(statuses)
}

/// A standing pin: the commit a scope's channel *serves*, whatever its ref
/// now points at (FLOW-7, ADR-0036 decisions 5 and 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelPin {
    /// The scope whose channel is held.
    pub scope_id: ScopeId,
    /// Which channel.
    pub channel: ChannelRef,
    /// The commit readers are held at.
    pub commit: CommitHash,
    /// When the pin was set or last moved.
    pub pinned_at: DateTime<Utc>,
    /// Who set it.
    pub pinned_by: IdentityId,
}

/// Reads a channel's pin. `None` = the channel serves its head.
#[tracing::instrument(
    name = "vedaflow.read_pin",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope, vedaflow.channel = %channel),
    err(Display)
)]
pub async fn read_pin(
    conn: &mut PgConnection,
    tenant: TenantId,
    scope: ScopeId,
    channel: ChannelRef,
) -> Result<Option<ChannelPin>> {
    let row = sqlx::query!(
        "select commit_hash, updated_at, updated_by from vedaflow_refs
         where tenant_id = $1 and scope_id = $2 and name = $3",
        tenant.as_uuid(),
        scope.as_uuid(),
        channel.pin_name(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(|err| storage_error("read channel pin", &err))?;

    row.map(|row| {
        Ok(ChannelPin {
            scope_id: scope,
            channel,
            commit: CommitHash::from_slice(&row.commit_hash)?,
            pinned_at: row.updated_at,
            pinned_by: IdentityId::from_uuid(row.updated_by),
        })
    })
    .transpose()
}

/// Holds a scope's channel at `commit`: readers compose that commit's
/// membership until the pin is released, while publications keep landing
/// and the channel's own ref keeps advancing (ADR-0036 decision 6).
///
/// Moving a standing pin is this same call — a re-pin is a decision, not a
/// race — and the previous commit comes back so the caller can audit what
/// changed.
///
/// The commit must be one the channel has actually held: the same
/// first-parent rule a rewind obeys (ADR-0036 decision 1), for the same
/// reason. A pin at a proposal commit would serve a member set nobody
/// approved, indefinitely, which is a worse version of the thing decision 1
/// exists to prevent.
#[tracing::instrument(
    name = "vedaflow.pin_channel",
    skip_all,
    fields(
        tenant.id = %tenant,
        scope.id = %scope,
        vedaflow.channel = %channel,
        vedaflow.commit = %commit.to_hex(),
    ),
    err(Display)
)]
pub async fn pin(
    conn: &mut PgConnection,
    tenant: TenantId,
    scope: ScopeId,
    channel: ChannelRef,
    commit: CommitHash,
    by: IdentityId,
) -> Result<Option<ChannelPin>> {
    let head = require_head(conn, tenant, scope, channel).await?;
    require_channel_state(conn, tenant, channel, head.commit_hash, commit, "pin").await?;

    let previous = read_pin(conn, tenant, scope, channel).await?;
    sqlx::query!(
        "insert into vedaflow_refs (tenant_id, scope_id, name, commit_hash, updated_by)
         values ($1, $2, $3, $4, $5)
         on conflict (tenant_id, scope_id, name)
         do update set commit_hash = excluded.commit_hash,
                       updated_at = now(),
                       updated_by = excluded.updated_by",
        tenant.as_uuid(),
        scope.as_uuid(),
        channel.pin_name(),
        commit.as_slice(),
        by.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(|err| storage_error("pin channel", &err))?;

    record(channel, "pinned");
    Ok(previous)
}

/// Releases a pin, returning what it held. `None` = there was none, which
/// is the answer rather than an error: unpinning an unpinned channel leaves
/// it serving its head either way.
///
/// This is the one ref deletion the schema permits, narrowed to names
/// beginning `pin/` by migration 0021's restrictive policy and trigger.
#[tracing::instrument(
    name = "vedaflow.unpin_channel",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope, vedaflow.channel = %channel),
    err(Display)
)]
pub async fn unpin(
    conn: &mut PgConnection,
    tenant: TenantId,
    scope: ScopeId,
    channel: ChannelRef,
) -> Result<Option<ChannelPin>> {
    let row = sqlx::query!(
        "delete from vedaflow_refs
         where tenant_id = $1 and scope_id = $2 and name = $3
         returning commit_hash, updated_at, updated_by",
        tenant.as_uuid(),
        scope.as_uuid(),
        channel.pin_name(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(|err| storage_error("release channel pin", &err))?;

    let released = row
        .map(|row| {
            Ok::<_, Error>(ChannelPin {
                scope_id: scope,
                channel,
                commit: CommitHash::from_slice(&row.commit_hash)?,
                pinned_at: row.updated_at,
                pinned_by: IdentityId::from_uuid(row.updated_by),
            })
        })
        .transpose()?;
    if released.is_some() {
        record(channel, "unpinned");
    }
    Ok(released)
}

/// A rewind, as the caller describes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelRewind {
    /// The scope whose channel moves back.
    pub scope: ScopeId,
    /// Which channel.
    pub channel: ChannelRef,
    /// The commit being abandoned — what the caller read before deciding.
    /// The move is compare-and-swapped against it, because a rewind is a
    /// decision about *which* state to leave and that decision is stale if
    /// someone else moved the ref meanwhile.
    pub from: CommitHash,
    /// The state to install: a strict first-parent ancestor of `from`.
    pub to: CommitHash,
    /// Who is rewinding.
    pub by: IdentityId,
}

/// What a rewind did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelRolledBack {
    /// The commit abandoned.
    pub from: CommitHash,
    /// The commit installed.
    pub to: CommitHash,
    /// Membership after the rewind.
    pub entries: usize,
    /// Entry names the channel held and no longer holds — for a memory
    /// channel, the record ids that stopped being published material.
    pub removed: Vec<String>,
    /// Entries the installed state names that the abandoned one did not
    /// name *at that address*: a record whose published version is now an
    /// earlier one.
    pub restored: Vec<ChannelMember>,
}

/// Rewinds a channel to a state it has already held (FLOW-7, ADR-0036).
///
/// Four rules, each of which refuses rather than surprises:
///
/// - **A set channel only.** A log channel's tree is one commit's additions
///   rather than its membership, and nothing composes it.
/// - **A strict first-parent ancestor only** — the states this ref has held
///   (decision 1), never an arbitrary reachable commit, and never a
///   descendant (decision 2). Recovery from a mistaken rewind is
///   publishing, which resolves the approval matrix again.
/// - **Not while pinned** (decision 7). A rewind's whole contract is that
///   readers heal on their next session; under a pin that is false, so it
///   is a [`Error::Conflict`] naming the pin rather than a 200 that did
///   nothing anyone can see.
/// - **Compare-and-swapped against `from`**, like every other ref move.
#[tracing::instrument(
    name = "vedaflow.rollback_channel",
    skip_all,
    fields(
        tenant.id = %tenant,
        scope.id = %rewind.scope,
        vedaflow.channel = %rewind.channel,
        vedaflow.from = %rewind.from.to_hex(),
        vedaflow.to = %rewind.to.to_hex(),
    ),
    err(Display)
)]
pub async fn rollback(
    conn: &mut PgConnection,
    tenant: TenantId,
    rewind: &ChannelRewind,
) -> Result<ChannelRolledBack> {
    let channel = rewind.channel;
    if rewind.from == rewind.to {
        return Err(Error::Invalid {
            message: format!("{channel} already points at {}", rewind.to.to_hex()),
        });
    }

    let head = require_head(conn, tenant, rewind.scope, channel).await?;
    if head.commit_hash != rewind.from {
        return Err(Error::Conflict {
            message: format!(
                "{channel} at this scope points at {}, not the {} this rewind abandons; \
                 re-read the history and decide again",
                head.commit_hash.to_hex(),
                rewind.from.to_hex()
            ),
        });
    }
    if let Some(pin) = read_pin(conn, tenant, rewind.scope, channel).await? {
        return Err(Error::Conflict {
            message: format!(
                "{channel} at this scope is pinned at {}, so readers would not heal; \
                 release the pin first (ADR-0036 decision 7)",
                pin.commit.to_hex()
            ),
        });
    }
    require_channel_state(conn, tenant, channel, rewind.from, rewind.to, "rewind").await?;

    let before = tree_of(conn, tenant, rewind.from).await?;
    let after = tree_of(conn, tenant, rewind.to).await?;
    let after_by_name: HashMap<&str, ObjectHash> = after
        .iter()
        .map(|(name, hash)| (name.as_str(), *hash))
        .collect();
    let removed: Vec<String> = before
        .iter()
        .filter(|(name, _)| !after_by_name.contains_key(name.as_str()))
        .map(|(name, _)| name.clone())
        .collect();
    let before_by_name: HashMap<&str, ObjectHash> = before
        .iter()
        .map(|(name, hash)| (name.as_str(), *hash))
        .collect();
    let restored: Vec<ChannelMember> = after
        .iter()
        .filter(|(name, hash)| before_by_name.get(name.as_str()) != Some(hash))
        .map(|(name, hash)| ChannelMember {
            name: name.clone(),
            object: *hash,
        })
        .collect();

    let outcome = force_update_ref(
        conn,
        tenant,
        rewind.scope,
        &channel.name(),
        rewind.from,
        rewind.to,
        rewind.by,
    )
    .await?;
    if !outcome.moved() {
        // The head was re-read inside this transaction, so losing the swap
        // means a concurrent writer committed between the two statements.
        return Err(Error::Conflict {
            message: format!("{channel} at this scope moved under the rewind; retry"),
        });
    }

    record(channel, "rolled_back");
    Ok(ChannelRolledBack {
        from: rewind.from,
        to: rewind.to,
        entries: after.len(),
        removed,
        restored,
    })
}

/// One state a channel has held: a commit on its first-parent line, with
/// what it recorded (FLOW-7, ADR-0036 decision 11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelHistoryEntry {
    /// Its address — what a rewind names and what a block cites.
    pub commit: CommitHash,
    /// The state it replaced: its first parent, absent on the channel's
    /// first commit.
    pub parent: Option<CommitHash>,
    /// Parents beyond the first — the proposal this publication was the
    /// effect of, when it had one (ADR-0032 decision 10). Reachable, and
    /// deliberately *not* rewind targets.
    pub merge_parents: Vec<CommitHash>,
    /// Who published it.
    pub author: IdentityId,
    /// Why.
    pub message: String,
    /// When, in valid-time terms.
    pub committed_at: DateTime<Utc>,
    /// Entries in its tree — the membership this state served.
    pub members: usize,
}

/// The states a channel has held, newest first: the first-parent walk from
/// its head, bounded by `limit`.
///
/// This renders exactly the set [`rollback`] accepts, so the surface an
/// operator reads and the set the route admits cannot drift apart — if it
/// is on this listing it can be rewound to (the head excepted, which is
/// where the channel already is), and if it is not, it cannot.
#[tracing::instrument(
    name = "vedaflow.channel_history",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope, vedaflow.channel = %channel),
    err(Display)
)]
pub async fn history(
    conn: &mut PgConnection,
    tenant: TenantId,
    scope: ScopeId,
    channel: ChannelRef,
    limit: u32,
) -> Result<Vec<ChannelHistoryEntry>> {
    let depth = i32::try_from(limit.max(1)).unwrap_or(MAX_FIRST_PARENT_WALK);
    let rows = sqlx::query!(
        r#"
        with recursive line (hash, depth) as (
            select r.commit_hash, 0
            from vedaflow_refs r
            where r.tenant_id = $1 and r.scope_id = $2 and r.name = $3
            union all
            select parent.parent_hash, line.depth + 1
            from vedaflow_commit_parents parent
            join line on line.hash = parent.commit_hash
            where parent.tenant_id = $1 and parent.ordinal = 0
              and line.depth + 1 < $4
              -- The same stop `is_first_parent_ancestor` makes, so the
              -- listing and the set a rewind accepts cannot drift apart:
              -- a channel's first publication through review has the
              -- proposal commit at ordinal 0, and a proposal commit was
              -- never a state the ref held.
              and not exists (
                  select from vedaflow_proposals proposal
                  where proposal.tenant_id = parent.tenant_id
                    and proposal.commit_hash = parent.parent_hash
              )
        )
        select line.depth as "depth!", c.hash, c.author_id, c.message, c.committed_at,
               (select count(*) from vedaflow_tree_entries e
                where e.tenant_id = c.tenant_id and e.tree_hash = c.tree_hash)
                   as "members!",
               (select array_agg(p.parent_hash order by p.ordinal)
                from vedaflow_commit_parents p
                where p.tenant_id = c.tenant_id and p.commit_hash = c.hash)
                   as "parents?",
               -- Which of them a proposal names. Split out rather than
               -- inferred from the ordinal, because the channel's first
               -- publication has its proposal *at* ordinal 0 and calling
               -- that "the state it replaced" would be a lie in the one
               -- place the distinction matters.
               (select array_agg(p.parent_hash order by p.ordinal)
                from vedaflow_commit_parents p
                where p.tenant_id = c.tenant_id and p.commit_hash = c.hash
                  and exists (
                      select from vedaflow_proposals proposal
                      where proposal.tenant_id = c.tenant_id
                        and proposal.commit_hash = p.parent_hash
                  ))
                   as "proposal_parents?"
        from line
        join vedaflow_commits c on c.tenant_id = $1 and c.hash = line.hash
        order by line.depth
        "#,
        tenant.as_uuid(),
        scope.as_uuid(),
        channel.name(),
        depth.min(MAX_FIRST_PARENT_WALK),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| storage_error("read channel history", &err))?;

    rows.into_iter()
        .map(|row| {
            let parents = row
                .parents
                .unwrap_or_default()
                .iter()
                .map(|bytes| CommitHash::from_slice(bytes))
                .collect::<Result<Vec<_>>>()?;
            let from_proposals = row
                .proposal_parents
                .unwrap_or_default()
                .iter()
                .map(|bytes| CommitHash::from_slice(bytes))
                .collect::<Result<Vec<_>>>()?;
            // The state this one replaced is its first parent that is not
            // a proposal commit — which for every publication but a
            // channel's first is simply its first parent.
            let parent = parents
                .iter()
                .find(|parent| !from_proposals.contains(parent))
                .copied();
            Ok(ChannelHistoryEntry {
                commit: CommitHash::from_slice(&row.hash)?,
                parent,
                merge_parents: parents
                    .into_iter()
                    .filter(|candidate| Some(*candidate) != parent)
                    .collect(),
                author: IdentityId::from_uuid(row.author_id),
                message: row.message,
                committed_at: row.committed_at,
                members: usize::try_from(row.members).unwrap_or(usize::MAX),
            })
        })
        .collect()
}

/// The channel's ref, or a `NotFound` naming it. A channel with no ref was
/// never written (ADR-0031 decision 2), so there is nothing to hold or
/// rewind.
async fn require_head(
    conn: &mut PgConnection,
    tenant: TenantId,
    scope: ScopeId,
    channel: ChannelRef,
) -> Result<StoredRef> {
    read_ref(conn, tenant, scope, &channel.name())
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("{channel} channel at this scope"),
        })
}

/// The rule both a rewind and a pin obey: `commit` must be a state this
/// channel has held — a strict first-parent ancestor of `head`, or `head`
/// itself for a pin.
async fn require_channel_state(
    conn: &mut PgConnection,
    tenant: TenantId,
    channel: ChannelRef,
    head: CommitHash,
    commit: CommitHash,
    verb: &str,
) -> Result<()> {
    if is_first_parent_ancestor(conn, tenant, commit, head).await? {
        return Ok(());
    }
    // Named separately because the two failures mean different things to
    // whoever is holding the terminal at 2am: a commit this channel will
    // reach later, versus one it was never in.
    let reachable = is_ancestor(conn, tenant, commit, head).await?;
    Err(Error::Invalid {
        message: if reachable {
            format!(
                "{} is reachable from {channel} but was never a state it held — it is a \
                 proposal or a side commit, whose tree is a member set that may never have \
                 been approved; {verb} to a commit from the channel's history (ADR-0036 \
                 decision 1)",
                commit.to_hex()
            )
        } else {
            format!(
                "{} is not in {channel}'s history at this scope; a {verb} only installs a \
                 state the channel has already held, and re-admitting content is a \
                 publication (ADR-0036 decisions 1 and 2)",
                commit.to_hex()
            )
        },
    })
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

    #[test]
    fn ref_names_round_trip_through_the_vocabulary() {
        for asset in AssetKind::CHANNELLED {
            for channel in Channel::ALL {
                let reference = ChannelRef::new(asset, channel);
                assert_eq!(reference.name().parse::<ChannelRef>().unwrap(), reference);
            }
        }
        assert_eq!(
            ChannelRef::prompt(Channel::Published).name(),
            "prompt/published"
        );
    }

    /// `vedaflow_refs` is generic on purpose: FLOW-3's proposal refs and
    /// FLOW-7's pins share the table, and neither is a channel.
    #[test]
    fn names_that_are_not_channels_do_not_parse_into_one() {
        for name in [
            "published",
            "knowledge",
            "knowledge/review",
            "notes/published",
            "",
        ] {
            assert!(
                name.parse::<ChannelRef>().is_err(),
                "{name:?} must not parse as a channel"
            );
        }
    }

    /// A pin's name must never round-trip into the channel it pins —
    /// otherwise the channel listing would show pins as channels, and
    /// `read_members` would compose one (ADR-0036 decision 5).
    #[test]
    fn a_pin_name_is_not_a_channel_name() {
        for asset in AssetKind::CHANNELLED {
            for channel in Channel::ALL {
                let reference = ChannelRef::new(asset, channel);
                let pin = reference.pin_name();
                assert!(pin.starts_with(PIN_PREFIX), "{pin} must be marked as a pin");
                assert_ne!(pin, reference.name());
                assert!(
                    pin.parse::<ChannelRef>().is_err(),
                    "{pin} must not parse as a channel"
                );
            }
        }
        assert_eq!(
            ChannelRef::prompt(Channel::Published).pin_name(),
            "pin/prompt/published"
        );
    }

    /// Migration 0021's delete policy and trigger both key on the prefix,
    /// so a name the code produces and a name the schema admits have to be
    /// the same shape. The `like 'pin/%'` pattern is spelled once in SQL
    /// and once here.
    #[test]
    fn every_pin_name_matches_what_the_schema_lets_go() {
        for asset in AssetKind::CHANNELLED {
            for channel in Channel::ALL {
                let pin = ChannelRef::new(asset, channel).pin_name();
                assert!(pin.starts_with("pin/"), "migration 0021 would refuse {pin}");
                assert!(
                    pin.chars().count() <= 200,
                    "{pin} is longer than vedaflow_refs.name accepts"
                );
            }
        }
    }

    #[test]
    fn aggregate_and_row_effect_assets_are_not_channels() {
        for name in ["knowledge/published", "policy/published"] {
            assert!(
                name.parse::<ChannelRef>().is_err(),
                "{name} must not create a second current-state pointer"
            );
        }
    }
}
