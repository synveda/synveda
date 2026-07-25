//! Commits: the governed history (FLOW-1, ADR-0030).
//!
//! A commit records what was published (its tree), where it came from (its
//! parents), who published it (the author), when, why (the message), and
//! **which policy pack was in force** — ADR-0003's compliance claim, made
//! concrete by `policy_snapshot_hash`. When a key is configured, a signature
//! over the commit hash covers all of it at once (ADR-0030 decision 9).

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use synveda_types::{Error, IdentityId, Result, TenantId};

use crate::hash::{CommitHash, PolicySnapshotHash, TreeHash, commit_hash_from, truncate_to_micros};
use crate::policy::PolicySnapshot;
use crate::signer::{CommitSignature, CommitSigner};
use crate::{Written, storage_error};

/// Counts commit writes, labelled by signing method and by whether the commit
/// was already present.
pub const COMMITS_WRITTEN_TOTAL: &str = "synveda_vedaflow_commits_written_total";

/// The longest commit message migration 0018 accepts.
const MAX_MESSAGE: usize = 4096;

/// A commit to be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCommit {
    /// What this commit publishes.
    pub tree: TreeHash,
    /// Ordered; empty for a root commit. The first parent is the mainline,
    /// as in git, and the order is part of the address.
    pub parents: Vec<CommitHash>,
    /// Who authored it.
    pub author: IdentityId,
    /// Why. Never empty — a commit with nothing to say is a commit an
    /// auditor cannot read.
    pub message: String,
    /// When, in valid-time terms. Truncated to microseconds before hashing so
    /// the hashed value is the stored value.
    pub committed_at: DateTime<Utc>,
    /// The policy pack in force, as the caller resolved it.
    pub policy_snapshot: PolicySnapshot,
}

/// A commit as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCommit {
    /// Its address.
    pub hash: CommitHash,
    /// What it publishes.
    pub tree: TreeHash,
    /// Its parents, in order.
    pub parents: Vec<CommitHash>,
    /// Who authored it.
    pub author: IdentityId,
    /// Why.
    pub message: String,
    /// When.
    pub committed_at: DateTime<Utc>,
    /// The fingerprint of the pack that governed it.
    pub policy_snapshot_hash: PolicySnapshotHash,
    /// The signature, when one was made.
    pub signature: Option<CommitSignature>,
}

/// Writes a commit, returning its address.
///
/// The tree and every parent must already exist in this tenant; migration
/// 0018's foreign keys enforce that, so a commit claiming a parent that does
/// not exist cannot be written even by a caller who tries.
///
/// Idempotent like the other writes: two callers producing the identical
/// commit — same tree, parents, author, message, instant, and pack — produce
/// one row, and the second is reported as deduplicated.
#[tracing::instrument(
    name = "vedaflow.commit",
    skip_all,
    fields(
        tenant.id = %tenant,
        vedaflow.parents = new.parents.len(),
        vedaflow.signer = signer.method(),
        vedaflow.hash = tracing::field::Empty,
        vedaflow.deduplicated = tracing::field::Empty,
    ),
    err(Display)
)]
pub async fn commit(
    conn: &mut PgConnection,
    tenant: TenantId,
    new: &NewCommit,
    signer: &impl CommitSigner,
) -> Result<Written<CommitHash>> {
    if new.message.is_empty() || new.message.chars().count() > MAX_MESSAGE {
        return Err(Error::Invalid {
            message: format!(
                "commit messages must be 1..={MAX_MESSAGE} characters, got {}",
                new.message.chars().count()
            ),
        });
    }
    if let Some(duplicate) = first_duplicate(&new.parents) {
        return Err(Error::Invalid {
            message: format!("commit lists parent {duplicate} twice"),
        });
    }

    let committed_at = truncate_to_micros(new.committed_at);
    let policy_snapshot_hash = new.policy_snapshot.hash()?;
    let hash = commit_hash_from(
        new.tree,
        &new.parents,
        new.author,
        committed_at,
        &new.message,
        policy_snapshot_hash,
    );
    let signature = signer.sign(hash);

    let inserted = sqlx::query!(
        "insert into vedaflow_commits
             (tenant_id, hash, tree_hash, author_id, message, committed_at,
              policy_snapshot_hash, signature, signer_key_id)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         on conflict (tenant_id, hash) do nothing",
        tenant.as_uuid(),
        hash.as_slice(),
        new.tree.as_slice(),
        new.author.as_uuid(),
        &new.message,
        committed_at,
        policy_snapshot_hash.as_slice(),
        signature.as_ref().map(|signed| &signed.signature[..]),
        signature.as_ref().map(|signed| signed.key_id.as_str()),
    )
    .execute(&mut *conn)
    .await
    .map_err(|err| storage_error("insert commit", &err))?
    .rows_affected();

    let written = Written {
        hash,
        deduplicated: inserted == 0,
    };
    if !written.deduplicated && !new.parents.is_empty() {
        let ordinals: Vec<i32> = (0..new.parents.len() as i32).collect();
        let parents: Vec<&[u8]> = new.parents.iter().map(CommitHash::as_slice).collect();
        sqlx::query!(
            "insert into vedaflow_commit_parents (tenant_id, commit_hash, ordinal, parent_hash)
             select $1, $2, ordinal, parent
             from unnest($3::int[], $4::bytea[]) as parent_row(ordinal, parent)",
            tenant.as_uuid(),
            hash.as_slice(),
            &ordinals,
            &parents as &[&[u8]],
        )
        .execute(&mut *conn)
        .await
        .map_err(|err| storage_error("insert commit parents", &err))?;
    }

    let span = tracing::Span::current();
    span.record("vedaflow.hash", hash.to_hex());
    span.record("vedaflow.deduplicated", written.deduplicated);
    metrics::counter!(
        COMMITS_WRITTEN_TOTAL,
        "signer" => signer.method(),
        "result" => if written.deduplicated { "deduplicated" } else { "stored" },
    )
    .increment(1);
    Ok(written)
}

/// Reads a commit back, parents included. `None` = no such commit in this
/// tenant.
#[tracing::instrument(name = "vedaflow.read_commit", skip_all, fields(tenant.id = %tenant), err(Display))]
pub async fn read_commit(
    conn: &mut PgConnection,
    tenant: TenantId,
    hash: CommitHash,
) -> Result<Option<StoredCommit>> {
    let row = sqlx::query!(
        "select tree_hash, author_id, message, committed_at, policy_snapshot_hash,
                signature, signer_key_id
         from vedaflow_commits
         where tenant_id = $1 and hash = $2",
        tenant.as_uuid(),
        hash.as_slice(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(|err| storage_error("read commit", &err))?;
    let Some(row) = row else { return Ok(None) };

    Ok(Some(StoredCommit {
        hash,
        tree: TreeHash::from_slice(&row.tree_hash)?,
        parents: read_parents(conn, tenant, hash).await?,
        author: IdentityId::from_uuid(row.author_id),
        message: row.message,
        committed_at: row.committed_at,
        policy_snapshot_hash: PolicySnapshotHash::from_slice(&row.policy_snapshot_hash)?,
        // The CHECK constraint pairs the two columns; either both are
        // present or neither is.
        signature: match (row.signature, row.signer_key_id) {
            (Some(signature), Some(key_id)) => Some(CommitSignature { signature, key_id }),
            _ => None,
        },
    }))
}

/// A commit's parents in mainline-first order.
pub(crate) async fn read_parents(
    conn: &mut PgConnection,
    tenant: TenantId,
    hash: CommitHash,
) -> Result<Vec<CommitHash>> {
    sqlx::query_scalar!(
        "select parent_hash from vedaflow_commit_parents
         where tenant_id = $1 and commit_hash = $2
         order by ordinal",
        tenant.as_uuid(),
        hash.as_slice(),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| storage_error("read commit parents", &err))?
    .iter()
    .map(|bytes| CommitHash::from_slice(bytes))
    .collect()
}

/// Whether `ancestor` is reachable from `descendant` by walking parents —
/// the fast-forward test (ADR-0030 decision 11).
///
/// A commit is its own ancestor, matching git's `merge-base --is-ancestor`,
/// so a ref "moving" to where it already points is trivially a fast-forward.
///
/// `union` rather than `union all`: the DAG is acyclic by construction
/// (ADR-0030 decision 5), and deduplicating bounds a diamond-shaped history
/// to its distinct commits rather than its distinct paths.
#[tracing::instrument(name = "vedaflow.is_ancestor", skip_all, fields(tenant.id = %tenant), err(Display))]
pub async fn is_ancestor(
    conn: &mut PgConnection,
    tenant: TenantId,
    ancestor: CommitHash,
    descendant: CommitHash,
) -> Result<bool> {
    if ancestor == descendant {
        return Ok(true);
    }
    sqlx::query_scalar!(
        r#"
        with recursive reachable (hash) as (
            select $3::bytea
            union
            select parent.parent_hash
            from vedaflow_commit_parents parent
            join reachable on reachable.hash = parent.commit_hash
            where parent.tenant_id = $1
        )
        select exists (select from reachable where hash = $2) as "found!"
        "#,
        tenant.as_uuid(),
        ancestor.as_slice(),
        descendant.as_slice(),
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(|err| storage_error("walk commit ancestry", &err))
}

/// The first hash that appears twice, if any. A commit listing the same
/// parent twice is meaningless and the table's unique constraint rejects it;
/// catching it here makes it the caller's error rather than a storage one.
fn first_duplicate(parents: &[CommitHash]) -> Option<CommitHash> {
    parents
        .iter()
        .enumerate()
        .find(|(index, parent)| parents[..*index].contains(parent))
        .map(|(_, parent)| *parent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_parents_are_caught_before_the_database_sees_them() {
        let a = CommitHash::from_bytes([1u8; 32]);
        let b = CommitHash::from_bytes([2u8; 32]);
        assert_eq!(first_duplicate(&[]), None);
        assert_eq!(first_duplicate(&[a, b]), None);
        assert_eq!(first_duplicate(&[a, b, a]), Some(a));
    }
}
