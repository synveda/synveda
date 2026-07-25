//! Recomputing the store's content addresses from the stored columns
//! (FLOW-1, ADR-0030 decision 6).
//!
//! Migration 0018's triggers refuse every UPDATE and DELETE on the history
//! tables, but a principal who can disable triggers can still rewrite a row —
//! that is the AUD-1 tamper test's attacker, and no schema defends against
//! it. What content addressing gives is *detection*: every stored hash is a
//! claim about the row's own bytes, so recomputing it either agrees or names
//! the row that lies.
//!
//! Deterministic and side-effect-free. One snapshot — run it inside a tenant
//! transaction.

use sqlx::PgConnection;
use synveda_types::{AssetKind, IdentityId, Result, TenantId};

use crate::commits::read_parents;
use crate::hash::{
    CommitHash, HashedEntry, ObjectHash, PolicySnapshotHash, TreeHash, commit_hash_from,
    object_hash, tree_hash_from, truncate_to_micros,
};
use crate::storage_error;
use crate::trees::{TreeEntry, TreeTarget, read_entries};

/// Counts verifications, labelled by outcome (`valid` / `broken`).
pub const VERIFICATIONS_TOTAL: &str = "synveda_vedaflow_verifications_total";

/// Rows fetched per round — bounds memory on large stores.
const PAGE: i64 = 512;

/// Which table a divergence was found in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectClass {
    /// `vedaflow_objects`.
    Object,
    /// `vedaflow_trees` (with its entries).
    Tree,
    /// `vedaflow_commits` (with its parents).
    Commit,
}

impl std::fmt::Display for ObjectClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ObjectClass::Object => "object",
            ObjectClass::Tree => "tree",
            ObjectClass::Commit => "commit",
        })
    }
}

/// The result of recomputing every address in a tenant's store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreVerification {
    /// Every stored row re-hashed to its stored address.
    Valid {
        /// How many blobs were checked.
        objects: i64,
        /// How many trees.
        trees: i64,
        /// How many commits.
        commits: i64,
    },
    /// A row's content does not hash to the address it is stored under.
    Broken {
        /// Which table.
        class: ObjectClass,
        /// The address it is stored under, as hex.
        stored: String,
        /// What its content actually hashes to, as hex.
        recomputed: String,
    },
}

impl std::fmt::Display for StoreVerification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreVerification::Valid {
                objects,
                trees,
                commits,
            } => write!(
                f,
                "store valid ({objects} objects, {trees} trees, {commits} commits)"
            ),
            StoreVerification::Broken {
                class,
                stored,
                recomputed,
            } => write!(
                f,
                "store BROKEN: {class} stored as {stored} hashes to {recomputed}"
            ),
        }
    }
}

/// Recomputes every content address in `tenant`'s store and reports the first
/// divergence.
///
/// Objects first, then trees, then commits — cheapest and most local first,
/// so the report names the innermost lie rather than the outermost symptom.
#[tracing::instrument(name = "vedaflow.verify", skip_all, fields(tenant.id = %tenant), err(Display))]
pub async fn verify(conn: &mut PgConnection, tenant: TenantId) -> Result<StoreVerification> {
    let verification = walk(conn, tenant).await?;
    metrics::counter!(
        VERIFICATIONS_TOTAL,
        "outcome" => match verification {
            StoreVerification::Valid { .. } => "valid",
            StoreVerification::Broken { .. } => "broken",
        },
    )
    .increment(1);
    Ok(verification)
}

async fn walk(conn: &mut PgConnection, tenant: TenantId) -> Result<StoreVerification> {
    let mut objects = 0i64;
    let mut after: Vec<u8> = Vec::new();
    loop {
        let rows = sqlx::query!(
            "select hash, kind, content from vedaflow_objects
             where tenant_id = $1 and hash > $2
             order by hash
             limit $3",
            tenant.as_uuid(),
            &after[..],
            PAGE,
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(|err| storage_error("read objects page", &err))?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            let stored = ObjectHash::from_slice(&row.hash)?;
            // An unparseable kind is itself a rewritten row: no writer of
            // ours could have produced it, so it cannot hash to `stored`.
            let recomputed = row
                .kind
                .parse::<AssetKind>()
                .map(|kind| object_hash(kind, &row.content))
                .unwrap_or_else(|_| ObjectHash::from_bytes([0u8; 32]));
            if recomputed != stored {
                return Ok(broken(
                    ObjectClass::Object,
                    &stored.to_hex(),
                    &recomputed.to_hex(),
                ));
            }
            after = row.hash;
            objects += 1;
        }
    }

    let mut trees = 0i64;
    let mut after: Vec<u8> = Vec::new();
    loop {
        let hashes = sqlx::query_scalar!(
            "select hash from vedaflow_trees
             where tenant_id = $1 and hash > $2
             order by hash
             limit $3",
            tenant.as_uuid(),
            &after[..],
            PAGE,
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(|err| storage_error("read trees page", &err))?;
        if hashes.is_empty() {
            break;
        }
        for hash in hashes {
            let stored = TreeHash::from_slice(&hash)?;
            let entries = read_entries(conn, tenant, stored).await?;
            let recomputed = recompute_tree(&entries);
            if recomputed != stored {
                return Ok(broken(
                    ObjectClass::Tree,
                    &stored.to_hex(),
                    &recomputed.to_hex(),
                ));
            }
            after = hash;
            trees += 1;
        }
    }

    let mut commits = 0i64;
    let mut after: Vec<u8> = Vec::new();
    loop {
        let rows = sqlx::query!(
            "select hash, tree_hash, author_id, message, committed_at, policy_snapshot_hash
             from vedaflow_commits
             where tenant_id = $1 and hash > $2
             order by hash
             limit $3",
            tenant.as_uuid(),
            &after[..],
            PAGE,
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(|err| storage_error("read commits page", &err))?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            let stored = CommitHash::from_slice(&row.hash)?;
            let recomputed = commit_hash_from(
                TreeHash::from_slice(&row.tree_hash)?,
                &read_parents(conn, tenant, stored).await?,
                IdentityId::from_uuid(row.author_id),
                truncate_to_micros(row.committed_at),
                &row.message,
                PolicySnapshotHash::from_slice(&row.policy_snapshot_hash)?,
            );
            if recomputed != stored {
                return Ok(broken(
                    ObjectClass::Commit,
                    &stored.to_hex(),
                    &recomputed.to_hex(),
                ));
            }
            after = row.hash;
            commits += 1;
        }
    }

    Ok(StoreVerification::Valid {
        objects,
        trees,
        commits,
    })
}

/// Re-derives a tree's address from the entries as read back. The read is
/// already in canonical (name) order, so no re-sorting is needed — and if the
/// stored order were wrong, that too would show up as a different address.
fn recompute_tree(entries: &[TreeEntry]) -> TreeHash {
    tree_hash_from(
        &entries
            .iter()
            .map(|entry| HashedEntry {
                name: &entry.name,
                tag: match entry.target {
                    TreeTarget::Object(_) => crate::hash::TARGET_TAG_OBJECT,
                    TreeTarget::Tree(_) => crate::hash::TARGET_TAG_TREE,
                },
                target: match &entry.target {
                    TreeTarget::Object(hash) => hash.as_bytes(),
                    TreeTarget::Tree(hash) => hash.as_bytes(),
                },
            })
            .collect::<Vec<_>>(),
    )
}

fn broken(class: ObjectClass, stored: &str, recomputed: &str) -> StoreVerification {
    StoreVerification::Broken {
        class,
        stored: stored.to_string(),
        recomputed: recomputed.to_string(),
    }
}
