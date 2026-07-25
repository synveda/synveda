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

/// Whether `ancestor` lies on `descendant`'s **first-parent line** — the
/// rewind test (FLOW-7, ADR-0036 decision 1).
///
/// Not the same question as [`is_ancestor`], and the difference is the whole
/// of why a rollback is safe. A channel commit's first parent is the state it
/// replaced ([`crate::channels`] puts the head first in every parent list), so
/// walking ordinal 0 from a head enumerates exactly the states that ref has
/// held. Every *other* reachable commit is something else: since FLOW-3 a
/// publication through a proposal is a merge commit whose second parent is the
/// proposal commit, whose tree is a proposed member set that may never have
/// been approved.
///
/// `union all` rather than `union`: the first-parent line is a path — each
/// commit has at most one ordinal-0 parent — so there are no diamonds to
/// deduplicate, and `depth` bounds the walk regardless.
///
/// A commit is on its own first-parent line, matching [`is_ancestor`]'s
/// convention; callers that mean a *strict* ancestor compare first.
///
/// # The one place ordinal 0 is not enough
///
/// A channel's **first** publication has no head to be its first parent, so
/// when that publication came through review the proposal commit lands at
/// ordinal 0 — a shape ADR-0032 decision 10 chose deliberately and FLOW-3's
/// acceptance test pins ("head first (there is none — this is the channel's
/// first commit), then the proposal"). Walking ordinal 0 alone would then
/// offer a proposal commit as a rewind target on exactly the channels that
/// have published least.
///
/// So the walk stops at any commit a proposal names. That is a fact this
/// schema already stores, it needs no marker column, and it says the rule
/// out loud: a proposal commit is not a state a ref has held, whichever
/// ordinal it happens to sit at.
#[tracing::instrument(
    name = "vedaflow.is_first_parent_ancestor",
    skip_all,
    fields(tenant.id = %tenant),
    err(Display)
)]
pub async fn is_first_parent_ancestor(
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
        with recursive line (hash, depth) as (
            select $3::bytea, 0
            union all
            select parent.parent_hash, line.depth + 1
            from vedaflow_commit_parents parent
            join line on line.hash = parent.commit_hash
            where parent.tenant_id = $1 and parent.ordinal = 0
              and line.depth < $4
              and not exists (
                  select from vedaflow_proposals proposal
                  where proposal.tenant_id = parent.tenant_id
                    and proposal.commit_hash = parent.parent_hash
              )
        )
        select exists (select from line where hash = $2) as "found!"
        "#,
        tenant.as_uuid(),
        ancestor.as_slice(),
        descendant.as_slice(),
        MAX_FIRST_PARENT_WALK,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(|err| storage_error("walk first-parent ancestry", &err))
}

/// How far back the first-parent walk goes.
///
/// A bound rather than a full walk: this runs inside a request, and a channel
/// with a hundred thousand publications behind it must not be able to turn a
/// rollback into a table scan. A rewind past this depth is refused as
/// unreachable rather than served slowly — the honest answer, since the
/// history route stops there too and an operator cannot roll back to a commit
/// the product will not show them (ADR-0036 decision 11).
pub const MAX_FIRST_PARENT_WALK: i32 = 10_000;

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
