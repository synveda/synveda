//! The Cedar entity store (HIER-3, ADR-0017): prebuilt entity fragments
//! per scope chain, so warm decisions stop reconstructing the entity
//! graph node by node.
//!
//! A fragment is valid exactly for the chain it was built from, checked
//! by *shape* — the ordered entity-relevant rows `(id, parent_id,
//! tenant_id, kind)` — never by a second invalidation protocol. Chains
//! arrive from the caller already carrying ADR-0016's transactional
//! freshness (the scope-chain cache is flushed post-commit at every
//! hierarchy-mutating seam), so a committed move changes the supplied
//! chain's shape and a stale fragment can never be served; a racing
//! request that reinserts pre-move data merely loses the next shape
//! comparison and is rebuilt. Display fields (`name`, `slug`, `path`,
//! `depth`) never reach Cedar entities (ADR-0011), so renames keep every
//! fragment valid.
//!
//! [`EntityStore::flush`] is hygiene, not correctness: it drops a
//! tenant's fragments at the gateway's unified hierarchy-invalidation
//! seam so deleted scopes do not linger as dead entries.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use cedar_policy::Entity;
use synveda_types::{HierarchyNode, Result, ScopeId, ScopeKind, TenantId};

use crate::{CEDAR_ENTITY_FLUSHES_TOTAL, CEDAR_ENTITY_FRAGMENTS_TOTAL};

/// The fields of a chain that Cedar entities are built from — a fragment
/// serves only a chain whose shape matches, in order.
type ShapeRow = (ScopeId, Option<ScopeId>, TenantId, ScopeKind);

fn shape_of(chain: &[HierarchyNode]) -> Vec<ShapeRow> {
    chain
        .iter()
        .map(|node| (node.id, node.parent_id, node.tenant_id, node.kind))
        .collect()
}

/// One chain's built entities: a `Scope` entity per node plus the
/// chain's distinct `Tenant` entities, ready to merge into a decision's
/// entity set.
pub(crate) struct Fragment {
    shape: Vec<ShapeRow>,
    entities: Vec<Entity>,
}

impl Fragment {
    pub(crate) fn entities(&self) -> &[Entity] {
        &self.entities
    }
}

/// The store: `tenant → (chain-head scope → fragment)`. Entry count per
/// tenant is bounded by scopes actually resolved — O(nodes), the
/// ADR-0016 bound. One per [`crate::Pdp`], shared by every decision.
#[derive(Default)]
pub(crate) struct EntityStore {
    tenants: RwLock<HashMap<TenantId, HashMap<ScopeId, Arc<Fragment>>>>,
}

impl EntityStore {
    /// The fragment for `chain` — served when the stored shape matches
    /// the supplied chain, rebuilt from it via `build` otherwise
    /// (ADR-0017 decision 2). `chain` must be non-empty; fragments key
    /// by the chain's own tenant and head node, so the gateway's
    /// tenant-wide flush reaches exactly the chains a mutation could
    /// have reshaped.
    pub(crate) fn resolve(
        &self,
        chain: &[HierarchyNode],
        build: impl FnOnce() -> Result<Vec<Entity>>,
    ) -> Result<Arc<Fragment>> {
        let head = &chain[0];
        let shape = shape_of(chain);
        let cached = {
            let tenants = read(&self.tenants);
            tenants
                .get(&head.tenant_id)
                .and_then(|fragments| fragments.get(&head.id))
                .cloned()
        };
        if let Some(fragment) = cached
            && fragment.shape == shape
        {
            metrics::counter!(CEDAR_ENTITY_FRAGMENTS_TOTAL, "outcome" => "hit").increment(1);
            return Ok(fragment);
        }
        let fragment = Arc::new(Fragment {
            shape,
            entities: build()?,
        });
        write(&self.tenants)
            .entry(head.tenant_id)
            .or_default()
            .insert(head.id, Arc::clone(&fragment));
        metrics::counter!(CEDAR_ENTITY_FRAGMENTS_TOTAL, "outcome" => "rebuild").increment(1);
        Ok(fragment)
    }

    /// Drops every fragment of the tenant — the gateway's unified
    /// hierarchy-invalidation seam calls this beside the scope-chain
    /// flush (ADR-0017 decision 5).
    pub(crate) fn flush(&self, tenant_id: TenantId) {
        write(&self.tenants).remove(&tenant_id);
        metrics::counter!(CEDAR_ENTITY_FLUSHES_TOTAL).increment(1);
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
    use std::cell::Cell;

    use chrono::Utc;

    use super::*;

    fn node(
        tenant_id: TenantId,
        parent_id: Option<ScopeId>,
        kind: ScopeKind,
        slug: &str,
    ) -> HierarchyNode {
        HierarchyNode {
            id: ScopeId::new(),
            tenant_id,
            parent_id,
            kind,
            slug: slug.to_owned(),
            name: slug.to_owned(),
            depth: 0,
            path: slug.to_owned(),
            created_at: Utc::now(),
        }
    }

    /// A build function that records whether it ran; the entities
    /// themselves are irrelevant to the store's mechanics.
    fn counted<'a>(built: &'a Cell<u32>) -> impl FnOnce() -> Result<Vec<Entity>> + 'a {
        move || {
            built.set(built.get() + 1);
            Ok(Vec::new())
        }
    }

    #[test]
    fn matching_shape_is_served_without_rebuilding() {
        let store = EntityStore::default();
        let tenant = TenantId::new();
        let org = node(tenant, None, ScopeKind::Org, "acme");
        let team = node(tenant, Some(org.id), ScopeKind::Team, "payments");
        let chain = vec![team, org];
        let built = Cell::new(0);

        store.resolve(&chain, counted(&built)).expect("first build");
        store.resolve(&chain, counted(&built)).expect("warm hit");
        assert_eq!(built.get(), 1, "an unchanged chain must not rebuild");
    }

    /// The HIER-3 crux: a chain reshaped by a committed move (new
    /// parent) must never be answered by the old fragment.
    #[test]
    fn a_moved_chain_rebuilds_its_fragment() {
        let store = EntityStore::default();
        let tenant = TenantId::new();
        let org = node(tenant, None, ScopeKind::Org, "acme");
        let dept_x = node(tenant, Some(org.id), ScopeKind::Department, "x");
        let dept_y = node(tenant, Some(org.id), ScopeKind::Department, "y");
        let mut team = node(tenant, Some(dept_x.id), ScopeKind::Team, "payments");
        let built = Cell::new(0);

        let before = vec![team.clone(), dept_x, org.clone()];
        store.resolve(&before, counted(&built)).expect("build");

        team.parent_id = Some(dept_y.id);
        let after = vec![team, dept_y, org];
        store.resolve(&after, counted(&built)).expect("rebuild");
        assert_eq!(built.get(), 2, "a reshaped chain must rebuild");

        // And a racing reinsert of pre-move data loses the next
        // comparison: serving `after` again stays warm on the new shape.
        store.resolve(&before, counted(&built)).expect("stale race");
        store.resolve(&after, counted(&built)).expect("fresh again");
        assert_eq!(built.get(), 4, "stale reinserts must not stick");
    }

    /// Renames flush the chain cache (ADR-0016 invalidates uniformly)
    /// but reach no Cedar entity: the re-read chain differs only in
    /// display fields, so the fragment stays valid (ADR-0017 decision 3).
    #[test]
    fn a_rename_keeps_the_fragment() {
        let store = EntityStore::default();
        let tenant = TenantId::new();
        let org = node(tenant, None, ScopeKind::Org, "acme");
        let mut team = node(tenant, Some(org.id), ScopeKind::Team, "payments");
        let built = Cell::new(0);

        store
            .resolve(&[team.clone(), org.clone()], counted(&built))
            .expect("build");
        team.name = "Payments Platform".to_owned();
        store
            .resolve(&[team, org], counted(&built))
            .expect("renamed");
        assert_eq!(built.get(), 1, "display-only changes must not rebuild");
    }

    #[test]
    fn flush_drops_only_its_tenant() {
        let store = EntityStore::default();
        let (this, other) = (TenantId::new(), TenantId::new());
        let mine = node(this, None, ScopeKind::Org, "mine");
        let theirs = node(other, None, ScopeKind::Org, "theirs");
        let built = Cell::new(0);
        store
            .resolve(std::slice::from_ref(&mine), counted(&built))
            .expect("build mine");
        store
            .resolve(std::slice::from_ref(&theirs), counted(&built))
            .expect("build theirs");

        store.flush(this);

        store
            .resolve(std::slice::from_ref(&theirs), counted(&built))
            .expect("theirs still warm");
        assert_eq!(built.get(), 2, "another tenant's fragments must survive");
        store
            .resolve(std::slice::from_ref(&mine), counted(&built))
            .expect("mine rebuilt");
        assert_eq!(built.get(), 3, "the flushed tenant re-misses");
    }
}
