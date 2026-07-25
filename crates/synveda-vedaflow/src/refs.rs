//! Refs: the only mutable thing in the store (FLOW-1, ADR-0030).
//!
//! A ref is a named pointer from a scope to a commit. FLOW-2 gives the names
//! meaning — `derived`/`staged`/`published` per scope per asset type — and
//! FLOW-1 leaves the vocabulary open, because a CHECK constraint written now
//! would have to be guessed and migrated then.
//!
//! Two rules make concurrent history safe (ADR-0030 decisions 10 and 11):
//!
//! - **Compare-and-swap.** Every move states the commit it expects to
//!   replace. A move that finds something else affects zero rows and returns
//!   [`RefUpdate::Raced`] — a result the caller retries, never an error to
//!   log and continue past. Last-writer-wins is exactly the lost update the
//!   FLOW-1 acceptance criteria forbid.
//! - **Fast-forward by default.** A ref moves only to a commit that has the
//!   current one as an ancestor. Rolling back is [`force_update_ref`], a
//!   different function with a different name, so no rollback is ever a typo.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use synveda_types::{Error, IdentityId, Result, ScopeId, TenantId};

use crate::commits::is_ancestor;
use crate::hash::CommitHash;
use crate::storage_error;

/// Counts ref updates, labelled by outcome.
pub const REF_UPDATES_TOTAL: &str = "synveda_vedaflow_ref_updates_total";

/// The longest ref name migration 0018 accepts.
const MAX_REF_NAME: usize = 200;

/// A ref as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRef {
    /// The scope this ref belongs to.
    pub scope_id: ScopeId,
    /// The channel name.
    pub name: String,
    /// Where it points.
    pub commit_hash: CommitHash,
    /// When it last moved.
    pub updated_at: DateTime<Utc>,
    /// Who last moved it.
    pub updated_by: IdentityId,
}

/// What a ref update did.
///
/// Racing and refusing are results rather than errors on purpose: both are
/// ordinary outcomes of concurrent writers, and both leave the ref exactly as
/// it was. An error type would tempt callers to log and continue, which is
/// how a commit becomes unreachable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefUpdate {
    /// The ref now points at the requested commit.
    Updated,
    /// The ref did not hold the expected commit — another writer moved it
    /// first. Re-read it, re-parent, and try again.
    Raced,
    /// The requested commit does not descend from the current one. Nothing
    /// was written; a deliberate rewind is [`force_update_ref`].
    NotFastForward,
}

impl RefUpdate {
    /// Whether the ref moved.
    #[must_use]
    pub const fn moved(&self) -> bool {
        matches!(self, RefUpdate::Updated)
    }

    const fn as_str(&self) -> &'static str {
        match self {
            RefUpdate::Updated => "updated",
            RefUpdate::Raced => "raced",
            RefUpdate::NotFastForward => "not_fast_forward",
        }
    }
}

/// Reads a ref. `None` = no such ref in this tenant and scope.
#[tracing::instrument(name = "vedaflow.read_ref", skip_all, fields(tenant.id = %tenant, scope.id = %scope), err(Display))]
pub async fn read_ref(
    conn: &mut PgConnection,
    tenant: TenantId,
    scope: ScopeId,
    name: &str,
) -> Result<Option<StoredRef>> {
    let row = sqlx::query!(
        "select commit_hash, updated_at, updated_by from vedaflow_refs
         where tenant_id = $1 and scope_id = $2 and name = $3",
        tenant.as_uuid(),
        scope.as_uuid(),
        name,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(|err| storage_error("read ref", &err))?;

    row.map(|row| {
        Ok(StoredRef {
            scope_id: scope,
            name: name.to_string(),
            commit_hash: CommitHash::from_slice(&row.commit_hash)?,
            updated_at: row.updated_at,
            updated_by: IdentityId::from_uuid(row.updated_by),
        })
    })
    .transpose()
}

/// Every ref at a scope, by name — FLOW-2's channel listing.
#[tracing::instrument(name = "vedaflow.list_refs", skip_all, fields(tenant.id = %tenant, scope.id = %scope), err(Display))]
pub async fn list_refs(
    conn: &mut PgConnection,
    tenant: TenantId,
    scope: ScopeId,
) -> Result<Vec<StoredRef>> {
    sqlx::query!(
        "select name, commit_hash, updated_at, updated_by from vedaflow_refs
         where tenant_id = $1 and scope_id = $2
         order by name",
        tenant.as_uuid(),
        scope.as_uuid(),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| storage_error("list refs", &err))?
    .into_iter()
    .map(|row| {
        Ok(StoredRef {
            scope_id: scope,
            name: row.name,
            commit_hash: CommitHash::from_slice(&row.commit_hash)?,
            updated_at: row.updated_at,
            updated_by: IdentityId::from_uuid(row.updated_by),
        })
    })
    .collect()
}

/// Creates a ref that does not exist yet.
///
/// Returns [`RefUpdate::Raced`] if one already does — the same "someone got
/// here first" outcome as a lost compare-and-swap, and the same response:
/// re-read and decide.
#[tracing::instrument(
    name = "vedaflow.create_ref",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope, vedaflow.ref = name, vedaflow.outcome = tracing::field::Empty),
    err(Display)
)]
pub async fn create_ref(
    conn: &mut PgConnection,
    tenant: TenantId,
    scope: ScopeId,
    name: &str,
    commit: CommitHash,
    by: IdentityId,
) -> Result<RefUpdate> {
    validate_name(name)?;
    let inserted = sqlx::query!(
        "insert into vedaflow_refs (tenant_id, scope_id, name, commit_hash, updated_by)
         values ($1, $2, $3, $4, $5)
         on conflict (tenant_id, scope_id, name) do nothing",
        tenant.as_uuid(),
        scope.as_uuid(),
        name,
        commit.as_slice(),
        by.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(|err| storage_error("create ref", &err))?
    .rows_affected();

    Ok(record(if inserted == 1 {
        RefUpdate::Updated
    } else {
        RefUpdate::Raced
    }))
}

/// Moves a ref from `expected` to `commit`, if it still holds `expected` and
/// the move is a fast-forward.
///
/// The ancestry check runs before the swap and the swap is conditional on
/// `expected`, so the pair is safe without an explicit lock: a writer that
/// slipped in between makes the `where` clause miss, and the caller retries
/// against the new head.
#[tracing::instrument(
    name = "vedaflow.update_ref",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope, vedaflow.ref = name, vedaflow.outcome = tracing::field::Empty),
    err(Display)
)]
pub async fn update_ref(
    conn: &mut PgConnection,
    tenant: TenantId,
    scope: ScopeId,
    name: &str,
    expected: CommitHash,
    commit: CommitHash,
    by: IdentityId,
) -> Result<RefUpdate> {
    validate_name(name)?;
    if !is_ancestor(conn, tenant, expected, commit).await? {
        return Ok(record(RefUpdate::NotFastForward));
    }
    swap(conn, tenant, scope, name, expected, commit, by).await
}

/// Moves a ref to `commit` regardless of ancestry, still compare-and-swapped
/// against `expected`.
///
/// This is FLOW-7's rollback: separate from [`update_ref`] so that rewinding
/// published history is always something a caller asked for by name. It is
/// still a compare-and-swap — forcing a move is a decision about *which*
/// commit to abandon, and that decision is stale if someone else moved the
/// ref meanwhile.
#[tracing::instrument(
    name = "vedaflow.force_update_ref",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope, vedaflow.ref = name, vedaflow.outcome = tracing::field::Empty),
    err(Display)
)]
pub async fn force_update_ref(
    conn: &mut PgConnection,
    tenant: TenantId,
    scope: ScopeId,
    name: &str,
    expected: CommitHash,
    commit: CommitHash,
    by: IdentityId,
) -> Result<RefUpdate> {
    validate_name(name)?;
    swap(conn, tenant, scope, name, expected, commit, by).await
}

/// The compare-and-swap itself.
async fn swap(
    conn: &mut PgConnection,
    tenant: TenantId,
    scope: ScopeId,
    name: &str,
    expected: CommitHash,
    commit: CommitHash,
    by: IdentityId,
) -> Result<RefUpdate> {
    let updated = sqlx::query!(
        "update vedaflow_refs
         set commit_hash = $5, updated_at = now(), updated_by = $6
         where tenant_id = $1 and scope_id = $2 and name = $3 and commit_hash = $4",
        tenant.as_uuid(),
        scope.as_uuid(),
        name,
        expected.as_slice(),
        commit.as_slice(),
        by.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(|err| storage_error("update ref", &err))?
    .rows_affected();

    Ok(record(if updated == 1 {
        RefUpdate::Updated
    } else {
        RefUpdate::Raced
    }))
}

/// Records the outcome on the current span and in the metric.
fn record(outcome: RefUpdate) -> RefUpdate {
    tracing::Span::current().record("vedaflow.outcome", outcome.as_str());
    metrics::counter!(REF_UPDATES_TOTAL, "outcome" => outcome.as_str()).increment(1);
    outcome
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.chars().count() > MAX_REF_NAME {
        return Err(Error::Invalid {
            message: format!(
                "ref names must be 1..={MAX_REF_NAME} characters, got {}",
                name.chars().count()
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_update_counts_as_moved() {
        assert!(RefUpdate::Updated.moved());
        assert!(!RefUpdate::Raced.moved());
        assert!(!RefUpdate::NotFastForward.moved());
    }

    #[test]
    fn ref_names_are_bounded_on_both_ends() {
        assert!(validate_name("").is_err());
        assert!(validate_name(&"x".repeat(MAX_REF_NAME + 1)).is_err());
        assert!(validate_name(&"x".repeat(MAX_REF_NAME)).is_ok());
        assert!(validate_name("published").is_ok());
    }
}
