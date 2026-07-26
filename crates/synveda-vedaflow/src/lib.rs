//! VedaFlow: git-style governance for knowledge assets — BLAKE3
//! content-addressed objects, trees, commits, and refs, implemented natively
//! in Postgres (tech plan §2, ADR-0003; object model in ADR-0030).
//!
//! # What this crate is
//!
//! The substrate, the channel vocabulary built on it, and the review that
//! stands in front of it. FLOW-1 shipped the object store: content
//! addressing, immutable history, and compare-and-swap ref updates.
//! FLOW-2 ([`channels`], ADR-0031) gives ref names meaning —
//! `{asset-kind}/{channel}` per scope, published and staged carrying their
//! whole membership and derived carrying a log of what each commit added.
//! FLOW-3 ([`proposals`] and [`curators`], ADR-0032) adds the governed
//! request that moves content onto a published channel: a commit holding
//! exactly what is reviewed, an append-only log of who approved it under
//! which roles, and a per-scope CODEOWNERS file that adds required
//! approvers. FLOW-7 ([`channels::rollback`], [`channels::pin`], ADR-0036)
//! rewinds a channel to a state it has already held, and holds what a
//! channel serves without moving where it points. Nothing here decides who
//! may write what, and nothing here counts as authority; that is the PDP's
//! job, at the seam the caller crossed to get here.
//!
//! # How it is used
//!
//! Every function takes the caller's `&mut PgConnection`, never a pool. The
//! transaction was opened by `synveda_store::rls::begin_tenant_tx` for the
//! same tenant, which is what makes ADR-0003's central claim true: a commit,
//! the records it describes, and the audit event attesting to it either all
//! land or none do. A caller who skipped that step writes zero rows — forced
//! RLS with an unset GUC matches nothing (ADR-0009).
//!
//! ```no_run
//! # use synveda_types::{AssetKind, IdentityId, ScopeId, TenantId};
//! # use synveda_vedaflow::{NewCommit, PolicySnapshot, Signer, TreeEntry};
//! # async fn example(conn: &mut sqlx::PgConnection, tenant: TenantId, scope: ScopeId,
//! #                  author: IdentityId) -> synveda_types::Result<()> {
//! let blob = synveda_vedaflow::put_object(conn, tenant, AssetKind::Prompt, b"be terse").await?;
//! let tree = synveda_vedaflow::put_tree(
//!     conn, tenant, &[TreeEntry::object("house-style.md", blob.hash)]
//! ).await?;
//! let head = synveda_vedaflow::commit(
//!     conn,
//!     tenant,
//!     &NewCommit {
//!         tree: tree.hash,
//!         parents: vec![],
//!         author,
//!         message: "house style".into(),
//!         committed_at: chrono::Utc::now(),
//!         policy_snapshot: PolicySnapshot::new("regulated-strict", 5),
//!     },
//!     &Signer::Unsigned,
//! )
//! .await?;
//! synveda_vedaflow::create_ref(conn, tenant, scope, "published", head.hash, author).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # The two invariants
//!
//! - **Identical content dedups.** The address *is* the primary key, so a
//!   second write of identical content conflicts with the first and is
//!   reported as [`Written::deduplicated`] rather than stored again. Dedup is
//!   per tenant, never across one (ADR-0030 decision 3).
//! - **History is immutable.** The five history tables grant the application
//!   role SELECT and INSERT only and raise on every UPDATE, DELETE, and
//!   TRUNCATE. Concurrent ref advances are compare-and-swap, so no writer's
//!   commit is ever silently abandoned. What a trigger cannot stop — a
//!   principal who disables triggers — [`verify()`] detects, by recomputing
//!   every address from the row it is stored under.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod channels;
pub mod commits;
pub mod curators;
pub mod hash;
pub mod lapses;
pub mod objects;
pub mod policy;
pub mod proposals;
pub mod refs;
pub mod signer;
pub mod trees;
pub mod verify;

pub use channels::{
    ChannelCommit, ChannelHistoryEntry, ChannelMember, ChannelPin, ChannelRef, ChannelRewind,
    ChannelRolledBack, ChannelSnapshot, ChannelStatus, ChannelWrite, MAX_CHANNEL_MEMBERS,
    MemoryAsset, MemoryChannel, PIN_PREFIX, append, history, pin, publish, put_memory,
    read_members, read_memory_members, read_pin, rollback, unpin,
};
pub use commits::{
    MAX_FIRST_PARENT_WALK, NewCommit, StoredCommit, commit, is_ancestor, is_first_parent_ancestor,
    read_commit,
};
pub use curators::{
    Approver, CURATORS_REF, CuratorCommit, CuratorFile, CuratorRule, CuratorWrite,
    MAX_CURATOR_FILE_BYTES, StoredCuratorFile, nearest_curators, read_curators, write_curators,
};
pub use hash::{CommitHash, ObjectHash, PolicySnapshotHash, TreeHash};
pub use lapses::{LapseAsset, put_lapse, read_lapse};
pub use objects::{MAX_OBJECT_BYTES, StoredObject, put_object, read_object, read_objects};
pub use policy::PolicySnapshot;
pub use proposals::{
    MAX_OPEN_PROPOSALS, MAX_PROPOSAL_MEMBERS, NewApproval, NewProposal, ProposalFilter,
    StoredApproval, StoredProposal,
};
pub use refs::{
    RefUpdate, StoredRef, create_ref, force_update_ref, list_refs, read_ref, update_ref,
};
pub use signer::{CommitSignature, CommitSigner, Ed25519Signer, Signer, verify_ed25519};
pub use trees::{TreeEntry, TreeTarget, put_tree, read_tree};
pub use verify::{ObjectClass, StoreVerification, verify};

use synveda_types::Error;

/// What a content-addressed write produced.
///
/// `deduplicated` is the FLOW-1 acceptance criterion made observable: the
/// caller learns whether its bytes were new without a second query, and the
/// property test asserts on it directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Written<H> {
    /// The content address, whether or not this call stored it.
    pub hash: H,
    /// True when the content was already present and this write changed
    /// nothing.
    pub deduplicated: bool,
}

/// Maps a sqlx failure into the taxonomy, marking RLS-backstop trips the way
/// `synveda_store::rls::backstop_error` does.
///
/// The marker prefix is duplicated rather than imported: `synveda-store` is a
/// sibling, not a dependency (seed §8), and the gateway's audit seam
/// classifies by this exact string (`rls::is_backstop_trip`, ADR-0019
/// decision 5). The RLS suite pins the two spellings together.
pub(crate) fn storage_error(context: &str, err: &sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = err
        && db.code().as_deref() == Some(INSUFFICIENT_PRIVILEGE)
    {
        return Error::Internal {
            message: format!("{BACKSTOP_PREFIX}: {context}: {err}"),
        };
    }
    Error::Storage {
        message: format!("{context}: {err}"),
    }
}

/// SQLSTATE 42501 — what forced RLS and a withheld grant both raise.
const INSUFFICIENT_PRIVILEGE: &str = "42501";

/// The marker `synveda_store::rls::is_backstop_trip` looks for.
const BACKSTOP_PREFIX: &str = "row-level security or privilege violation";
