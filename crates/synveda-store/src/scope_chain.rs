//! The scope chain resolver (HIER-2, ADR-0016): given a scope, the
//! ordered chain for composition — the node itself first, then ancestors
//! nearest-first, org root last. Identity → chain is the composition
//! callers already have: `identities::by_subject(..).scope_id` →
//! [`ScopeChainCache::resolve`].
//!
//! Chains are cached read-through per `(tenant, scope)` and invalidated
//! tenant-wide after any committed hierarchy mutation — the gateway's
//! mutating handlers call [`ScopeChainCache::invalidate`] post-commit. A
//! per-tenant generation counter closes the read/invalidate race: the
//! populating read snapshots the generation before touching the
//! database and its insert is discarded if an invalidation intervened
//! (ADR-0016 decision 4). Callers must therefore not resolve after
//! staging hierarchy writes in the same transaction — an uncommitted (or
//! rolled-back) chain must never enter the cache.
//!
//! The cache key carries the tenant and the miss query filters on it in
//! SQL, so tenant correctness never rides on the RLS backstop, which the
//! dev-compose superuser bypasses (ADR-0009).

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use sqlx::PgExecutor;
use synveda_types::{HierarchyNode, Result, ScopeId, TenantId};

use crate::hierarchy;

/// Counter: chain resolutions, labelled `outcome` = `hit` | `miss`.
/// Emitted here, described by the gateway where the recorder lives
/// (ADR-0007).
pub const SCOPE_CHAIN_RESOLUTIONS_TOTAL: &str = "synveda_scope_chain_resolutions_total";

/// Counter: tenant-wide flushes after committed hierarchy mutations.
pub const SCOPE_CHAIN_INVALIDATIONS_TOTAL: &str = "synveda_scope_chain_invalidations_total";

/// One tenant's cached chains plus the generation that guards them.
#[derive(Default)]
struct TenantChains {
    /// Bumped by every invalidation; a populating read that snapshotted
    /// an older generation discards its chain instead of inserting it.
    generation: u64,
    chains: HashMap<ScopeId, Arc<[HierarchyNode]>>,
}

/// The read-through scope-chain cache (ADR-0016). One per process,
/// shared by every request; a warm resolve is a read lock and an `Arc`
/// clone.
#[derive(Default)]
pub struct ScopeChainCache {
    tenants: RwLock<HashMap<TenantId, TenantChains>>,
}

impl ScopeChainCache {
    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The chain for `scope_id`, node-first to the org root — cached, or
    /// read through `executor` (one closure scan, ADR-0016 decision 2).
    /// `None` means the scope does not exist for this tenant; negative
    /// results are never cached, so a subsequently created node is
    /// visible immediately.
    #[tracing::instrument(
        name = "store.scope_chain.resolve",
        skip_all,
        fields(tenant.id = %tenant_id, scope.id = %scope_id, cache.outcome = tracing::field::Empty),
        err(Display)
    )]
    pub async fn resolve(
        &self,
        executor: impl PgExecutor<'_>,
        tenant_id: TenantId,
        scope_id: ScopeId,
    ) -> Result<Option<Arc<[HierarchyNode]>>> {
        let span = tracing::Span::current();
        // Snapshot the generation before the database read (decision 4).
        let (cached, generation) = self.lookup(tenant_id, scope_id);
        if let Some(chain) = cached {
            span.record("cache.outcome", "hit");
            metrics::counter!(SCOPE_CHAIN_RESOLUTIONS_TOTAL, "outcome" => "hit").increment(1);
            return Ok(Some(chain));
        }
        span.record("cache.outcome", "miss");
        metrics::counter!(SCOPE_CHAIN_RESOLUTIONS_TOTAL, "outcome" => "miss").increment(1);
        let chain = hierarchy::chain(executor, tenant_id, scope_id).await?;
        if chain.is_empty() {
            return Ok(None);
        }
        let chain: Arc<[HierarchyNode]> = chain.into();
        self.insert_if_current(tenant_id, generation, scope_id, Arc::clone(&chain));
        Ok(Some(chain))
    }

    /// Drops every cached chain for the tenant and bumps its generation,
    /// so in-flight populating reads that predate the call discard their
    /// results. The gateway calls this after committing any hierarchy
    /// mutation (ADR-0016 decision 5).
    pub fn invalidate(&self, tenant_id: TenantId) {
        let mut tenants = write(&self.tenants);
        let entry = tenants.entry(tenant_id).or_default();
        entry.generation += 1;
        entry.chains.clear();
        metrics::counter!(SCOPE_CHAIN_INVALIDATIONS_TOTAL).increment(1);
    }

    /// The cached chain (if any) and the generation to validate a
    /// populating insert against. An unknown tenant is generation 0 —
    /// consistent with the entry `or_default` inserts later.
    fn lookup(
        &self,
        tenant_id: TenantId,
        scope_id: ScopeId,
    ) -> (Option<Arc<[HierarchyNode]>>, u64) {
        let tenants = read(&self.tenants);
        match tenants.get(&tenant_id) {
            Some(entry) => (entry.chains.get(&scope_id).cloned(), entry.generation),
            None => (None, 0),
        }
    }

    /// Stores a freshly read chain unless an invalidation intervened
    /// since the caller snapshotted `generation`.
    fn insert_if_current(
        &self,
        tenant_id: TenantId,
        generation: u64,
        scope_id: ScopeId,
        chain: Arc<[HierarchyNode]>,
    ) {
        let mut tenants = write(&self.tenants);
        let entry = tenants.entry(tenant_id).or_default();
        if entry.generation == generation {
            entry.chains.insert(scope_id, chain);
        }
    }
}

/// Lock helpers in the PDP's pattern (ADR-0012): a poisoned lock means a
/// panic mid-`HashMap` operation; the map is still structurally sound,
/// so recover the guard rather than propagate unavailability.
fn read<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(PoisonError::into_inner)
}

fn write<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use synveda_types::ScopeKind;

    fn node(tenant_id: TenantId, kind: ScopeKind, slug: &str) -> HierarchyNode {
        HierarchyNode {
            id: ScopeId::new(),
            tenant_id,
            parent_id: None,
            kind,
            slug: slug.to_owned(),
            name: slug.to_owned(),
            depth: 0,
            path: slug.to_owned(),
            created_at: chrono::Utc::now(),
        }
    }

    fn chain_of(nodes: &[HierarchyNode]) -> Arc<[HierarchyNode]> {
        nodes.to_vec().into()
    }

    /// The race the generation closes (ADR-0016 decision 4): a read that
    /// snapshotted its generation before an invalidation must not
    /// repopulate the cache with what it read.
    #[test]
    fn stale_populating_read_is_discarded_after_invalidation() {
        let cache = ScopeChainCache::new();
        let tenant = TenantId::new();
        let org = node(tenant, ScopeKind::Org, "acme");
        let chain = chain_of(std::slice::from_ref(&org));

        let (cached, generation) = cache.lookup(tenant, org.id);
        assert!(cached.is_none(), "fresh cache starts empty");

        // A hierarchy mutation commits between the snapshot and the insert.
        cache.invalidate(tenant);
        cache.insert_if_current(tenant, generation, org.id, Arc::clone(&chain));
        let (cached, bumped) = cache.lookup(tenant, org.id);
        assert!(cached.is_none(), "stale insert must be discarded");

        // The re-read (post-bump generation) stores fine.
        cache.insert_if_current(tenant, bumped, org.id, chain);
        let (cached, _) = cache.lookup(tenant, org.id);
        assert!(cached.is_some(), "current-generation insert is kept");
    }

    #[test]
    fn invalidation_flushes_only_its_tenant() {
        let cache = ScopeChainCache::new();
        let (this, other) = (TenantId::new(), TenantId::new());
        let mine = node(this, ScopeKind::Org, "mine");
        let theirs = node(other, ScopeKind::Org, "theirs");
        cache.insert_if_current(this, 0, mine.id, chain_of(std::slice::from_ref(&mine)));
        cache.insert_if_current(other, 0, theirs.id, chain_of(std::slice::from_ref(&theirs)));

        cache.invalidate(this);

        assert!(cache.lookup(this, mine.id).0.is_none());
        assert!(
            cache.lookup(other, theirs.id).0.is_some(),
            "another tenant's chains must survive"
        );
    }

    /// The key carries the tenant: a foreign tenant probing a cached
    /// scope id must miss (the read-through query then reads nothing —
    /// its tenant filter is in SQL).
    #[test]
    fn cache_keys_are_tenant_scoped() {
        let cache = ScopeChainCache::new();
        let (owner, prober) = (TenantId::new(), TenantId::new());
        let org = node(owner, ScopeKind::Org, "owner-org");
        cache.insert_if_current(owner, 0, org.id, chain_of(std::slice::from_ref(&org)));

        assert!(cache.lookup(owner, org.id).0.is_some());
        assert!(
            cache.lookup(prober, org.id).0.is_none(),
            "a cached chain must never answer for another tenant"
        );
    }
}
