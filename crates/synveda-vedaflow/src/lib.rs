//! VedaFlow: git-style governance for knowledge assets — BLAKE3
//! content-addressed objects, trees, commits, and refs, implemented natively
//! in Postgres (tech plan §2, ADR-0003; object model in ADR-0030).
//!
//! # What this crate is
//!
//! The substrate, and only the substrate. FLOW-1 ships the object store:
//! content addressing, immutable history, and compare-and-swap ref updates.
//! The meanings come later — FLOW-2 turns ref names into the
//! `derived`/`staged`/`published` channels the inject path reads, FLOW-3
//! hangs proposals and the approval matrix off two refs, FLOW-7 rolls a ref
//! back. Nothing here decides who may write what; that is the PDP's job, at
//! the seam the caller crossed to get here.
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

pub mod commits;
pub mod hash;
pub mod objects;
pub mod policy;
pub mod refs;
pub mod signer;
pub mod trees;
pub mod verify;

pub use commits::{NewCommit, StoredCommit, commit, is_ancestor, read_commit};
pub use hash::{CommitHash, ObjectHash, PolicySnapshotHash, TreeHash};
pub use objects::{MAX_OBJECT_BYTES, StoredObject, put_object, read_object};
pub use policy::PolicySnapshot;
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
