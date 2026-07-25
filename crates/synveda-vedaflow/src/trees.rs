//! Trees: named groupings of objects and subtrees (FLOW-1, ADR-0030).
//!
//! Entries live in their own table with foreign keys to what they point at
//! (ADR-0030 decision 5), so a tree entry addressing an object that does not
//! exist is unrepresentable rather than merely unwritten. Referential closure
//! is the half of "history immutable" an array column cannot give.

use sqlx::PgConnection;
use synveda_types::{Error, Result, TenantId};

use crate::hash::{
    HashedEntry, ObjectHash, TARGET_TAG_OBJECT, TARGET_TAG_TREE, TreeHash, tree_hash_from,
};
use crate::{Written, storage_error};

/// Counts tree writes, labelled by whether the tree was already present.
pub const TREES_WRITTEN_TOTAL: &str = "synveda_vedaflow_trees_written_total";

/// The largest entry name migration 0018 accepts.
const MAX_ENTRY_NAME: usize = 255;

/// What a tree entry points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TreeTarget {
    /// A blob.
    Object(ObjectHash),
    /// Another tree, so scopes can nest.
    Tree(TreeHash),
}

impl TreeTarget {
    /// The tag that distinguishes the two targets inside a tree's address —
    /// without it, an object and a subtree at the same hash would produce
    /// the same tree.
    const fn tag(&self) -> u8 {
        match self {
            TreeTarget::Object(_) => TARGET_TAG_OBJECT,
            TreeTarget::Tree(_) => TARGET_TAG_TREE,
        }
    }

    const fn bytes(&self) -> &[u8; 32] {
        match self {
            TreeTarget::Object(hash) => hash.as_bytes(),
            TreeTarget::Tree(hash) => hash.as_bytes(),
        }
    }
}

/// One named entry in a tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    /// Unique within its tree; sorted bytewise in the canonical encoding.
    pub name: String,
    /// What it points at.
    pub target: TreeTarget,
}

impl TreeEntry {
    /// Builds an entry pointing at an object.
    #[must_use]
    pub fn object(name: impl Into<String>, hash: ObjectHash) -> Self {
        TreeEntry {
            name: name.into(),
            target: TreeTarget::Object(hash),
        }
    }

    /// Builds an entry pointing at a subtree.
    #[must_use]
    pub fn subtree(name: impl Into<String>, hash: TreeHash) -> Self {
        TreeEntry {
            name: name.into(),
            target: TreeTarget::Tree(hash),
        }
    }
}

/// Writes a tree, returning its address.
///
/// Entries are sorted by name before hashing, so the caller's ordering never
/// reaches the address: the same set of entries in any order is the same
/// tree. Duplicate names are rejected — a tree with two entries called `x`
/// has no meaning, and the primary key would reject it anyway.
///
/// Like [`crate::objects::put_object`], a second write of an identical tree
/// is reported as deduplicated and touches nothing: the existing entries are
/// already correct, and the immutability triggers would refuse to change them
/// even if they were not.
#[tracing::instrument(
    name = "vedaflow.put_tree",
    skip_all,
    fields(
        tenant.id = %tenant,
        vedaflow.entries = entries.len(),
        vedaflow.hash = tracing::field::Empty,
        vedaflow.deduplicated = tracing::field::Empty,
    ),
    err(Display)
)]
pub async fn put_tree(
    conn: &mut PgConnection,
    tenant: TenantId,
    entries: &[TreeEntry],
) -> Result<Written<TreeHash>> {
    let sorted = canonicalise(entries)?;
    let hash = tree_hash_from(
        &sorted
            .iter()
            .map(|entry| HashedEntry {
                name: &entry.name,
                tag: entry.target.tag(),
                target: entry.target.bytes(),
            })
            .collect::<Vec<_>>(),
    );

    let inserted = sqlx::query!(
        "insert into vedaflow_trees (tenant_id, hash) values ($1, $2)
         on conflict (tenant_id, hash) do nothing",
        tenant.as_uuid(),
        hash.as_slice(),
    )
    .execute(&mut *conn)
    .await
    .map_err(|err| storage_error("insert tree", &err))?
    .rows_affected();

    let written = Written {
        hash,
        deduplicated: inserted == 0,
    };
    if !written.deduplicated && !sorted.is_empty() {
        // Parallel arrays with a subtree flag rather than nullable arrays:
        // the CASE keeps both foreign keys real while binding one target
        // column per entry.
        let names: Vec<&str> = sorted.iter().map(|entry| entry.name.as_str()).collect();
        let targets: Vec<&[u8]> = sorted
            .iter()
            .map(|entry| &entry.target.bytes()[..])
            .collect();
        let subtree_flags: Vec<bool> = sorted
            .iter()
            .map(|entry| matches!(entry.target, TreeTarget::Tree(_)))
            .collect();
        sqlx::query!(
            "insert into vedaflow_tree_entries
                 (tenant_id, tree_hash, name, object_hash, subtree_hash)
             select $1, $2, name,
                    case when is_subtree then null else target end,
                    case when is_subtree then target else null end
             from unnest($3::text[], $4::bytea[], $5::bool[])
                  as entry(name, target, is_subtree)",
            tenant.as_uuid(),
            hash.as_slice(),
            &names as &[&str],
            &targets as &[&[u8]],
            &subtree_flags,
        )
        .execute(&mut *conn)
        .await
        .map_err(|err| storage_error("insert tree entries", &err))?;
    }

    let span = tracing::Span::current();
    span.record("vedaflow.hash", hash.to_hex());
    span.record("vedaflow.deduplicated", written.deduplicated);
    metrics::counter!(
        TREES_WRITTEN_TOTAL,
        "result" => if written.deduplicated { "deduplicated" } else { "stored" },
    )
    .increment(1);
    Ok(written)
}

/// Reads a tree's entries back, in canonical (name) order. `None` = no such
/// tree in this tenant.
#[tracing::instrument(name = "vedaflow.read_tree", skip_all, fields(tenant.id = %tenant), err(Display))]
pub async fn read_tree(
    conn: &mut PgConnection,
    tenant: TenantId,
    hash: TreeHash,
) -> Result<Option<Vec<TreeEntry>>> {
    let exists = sqlx::query_scalar!(
        r#"select true as "exists!" from vedaflow_trees
           where tenant_id = $1 and hash = $2"#,
        tenant.as_uuid(),
        hash.as_slice(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(|err| storage_error("read tree", &err))?;
    if exists.is_none() {
        return Ok(None);
    }
    read_entries(conn, tenant, hash).await.map(Some)
}

/// A tree's entries in canonical order. The empty tree is legitimate (git
/// has one), so this returns an empty vector rather than an error.
pub(crate) async fn read_entries(
    conn: &mut PgConnection,
    tenant: TenantId,
    hash: TreeHash,
) -> Result<Vec<TreeEntry>> {
    // `order by name` is the canonical order — the same order the hash was
    // computed in, so verification re-derives the address without re-sorting.
    let rows = sqlx::query!(
        "select name, object_hash, subtree_hash from vedaflow_tree_entries
         where tenant_id = $1 and tree_hash = $2
         order by name",
        tenant.as_uuid(),
        hash.as_slice(),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| storage_error("read tree entries", &err))?;

    rows.into_iter()
        .map(|row| {
            let target = match (row.object_hash, row.subtree_hash) {
                (Some(object), None) => TreeTarget::Object(ObjectHash::from_slice(&object)?),
                (None, Some(subtree)) => TreeTarget::Tree(TreeHash::from_slice(&subtree)?),
                // The CHECK constraint forbids both cases; reaching here
                // means schema and code have drifted.
                _ => {
                    return Err(Error::Internal {
                        message: format!(
                            "tree entry {:?} names neither exactly one target",
                            row.name
                        ),
                    });
                }
            };
            Ok(TreeEntry {
                name: row.name,
                target,
            })
        })
        .collect()
}

/// Validates and sorts entries into canonical order.
fn canonicalise(entries: &[TreeEntry]) -> Result<Vec<TreeEntry>> {
    let mut sorted = entries.to_vec();
    sorted.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    for entry in &sorted {
        if entry.name.is_empty() || entry.name.chars().count() > MAX_ENTRY_NAME {
            return Err(Error::Invalid {
                message: format!(
                    "tree entry names must be 1..={MAX_ENTRY_NAME} characters, got {}",
                    entry.name.chars().count()
                ),
            });
        }
    }
    if let Some(duplicate) = sorted.windows(2).find(|pair| pair[0].name == pair[1].name) {
        return Err(Error::Invalid {
            message: format!("duplicate tree entry name: {:?}", duplicate[0].name),
        });
    }
    Ok(sorted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use synveda_types::AssetKind;

    fn object(byte: u8) -> ObjectHash {
        crate::hash::object_hash(AssetKind::Memory, &[byte])
    }

    #[test]
    fn canonicalisation_sorts_bytewise_and_is_order_independent() {
        let unsorted = vec![
            TreeEntry::object("zeta", object(1)),
            TreeEntry::object("Alpha", object(2)),
            TreeEntry::object("alpha", object(3)),
        ];
        let sorted = canonicalise(&unsorted).unwrap();
        let names: Vec<&str> = sorted.iter().map(|entry| entry.name.as_str()).collect();
        // Bytewise, not locale-aware: 'A' (0x41) sorts before 'a' (0x61).
        assert_eq!(names, ["Alpha", "alpha", "zeta"]);
    }

    #[test]
    fn duplicate_names_are_rejected_before_the_primary_key_sees_them() {
        let entries = vec![
            TreeEntry::object("x", object(1)),
            TreeEntry::object("x", object(2)),
        ];
        assert!(matches!(canonicalise(&entries), Err(Error::Invalid { .. })));
    }

    #[test]
    fn entry_names_are_bounded_on_both_ends() {
        assert!(canonicalise(&[TreeEntry::object("", object(1))]).is_err());
        let long = "x".repeat(MAX_ENTRY_NAME + 1);
        assert!(canonicalise(&[TreeEntry::object(long, object(1))]).is_err());
        let at_limit = "x".repeat(MAX_ENTRY_NAME);
        assert!(canonicalise(&[TreeEntry::object(at_limit, object(1))]).is_ok());
    }
}
