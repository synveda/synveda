//! HIER-3 at the facade (ADR-0017): the entity store serves prebuilt
//! fragments to warm decisions, and a chain reshaped by a hierarchy move
//! is never answered from stale entities — the decision flips with the
//! supplied chain, both directions. The pack is applied through the same
//! assignment-resolution path production uses — never a PDP bypass
//! (CLAUDE.md, seed §2.2).
//!
//! The fixture mirrors the AC: two departments, a principal in a team of
//! one, a sibling team that moves between them.
//!
//! ```text
//! org
//! ├── eng (department)
//! │   ├── team-a ── alice-user   ← alice's placement
//! │   └── team-b                 ← moves to sales and back
//! └── sales (department)
//! ```

use chrono::Utc;
use synveda_policy::{Action, AuthzContext, Pdp, Principal, Resource, STANDARD};
use synveda_types::{HierarchyNode, PolicyAssignment, ScopeId, ScopeKind, TenantId};

struct Fixture {
    tenant: TenantId,
    nodes: Vec<HierarchyNode>,
}

impl Fixture {
    fn node(&self, slug: &str) -> &HierarchyNode {
        self.nodes
            .iter()
            .find(|node| node.slug == slug)
            .unwrap_or_else(|| panic!("fixture has no node {slug}"))
    }

    /// Re-parents `slug` — the post-commit truth the scope-chain cache
    /// would serve after a committed move (ADR-0016).
    fn move_node(&mut self, slug: &str, new_parent: &str) {
        let parent_id = self.node(new_parent).id;
        let node = self
            .nodes
            .iter_mut()
            .find(|node| node.slug == slug)
            .unwrap_or_else(|| panic!("fixture has no node {slug}"));
        node.parent_id = Some(parent_id);
    }

    /// The node and its ancestors — what the gateway resolves from the
    /// scope-chain cache for a resource or a placement.
    fn chain(&self, slug: &str) -> Vec<HierarchyNode> {
        let mut chain = vec![self.node(slug).clone()];
        let mut current = chain[0].parent_id;
        while let Some(id) = current {
            let parent = self
                .nodes
                .iter()
                .find(|node| node.id == id)
                .expect("parent exists");
            current = parent.parent_id;
            chain.push(parent.clone());
        }
        chain
    }
}

fn fixture() -> Fixture {
    let tenant = TenantId::new();
    let mut nodes = Vec::new();
    let mut add = |parent: Option<ScopeId>, kind: ScopeKind, slug: &str, depth: i32| -> ScopeId {
        let id = ScopeId::new();
        nodes.push(HierarchyNode {
            id,
            tenant_id: tenant,
            parent_id: parent,
            kind,
            slug: slug.to_owned(),
            name: slug.to_owned(),
            depth,
            path: slug.to_owned(),
            created_at: Utc::now(),
        });
        id
    };
    let org = add(None, ScopeKind::Org, "org", 0);
    let eng = add(Some(org), ScopeKind::Department, "eng", 1);
    add(Some(org), ScopeKind::Department, "sales", 1);
    let team_a = add(Some(eng), ScopeKind::Team, "team-a", 2);
    add(Some(eng), ScopeKind::Team, "team-b", 2);
    add(Some(team_a), ScopeKind::User, "alice-user", 3);
    Fixture { tenant, nodes }
}

/// Alice's `MemoryRead` on `target` under `standard`, chains supplied
/// exactly as the gateway would after resolving them from the
/// scope-chain cache.
fn alice_reads(pdp: &Pdp, fx: &Fixture, target: &str, assignments: &[PolicyAssignment]) -> bool {
    let alice = Principal {
        tenant_id: fx.tenant,
        subject: "alice".to_owned(),
        quarantined: false,
        scope_id: Some(fx.node("alice-user").id),
        token_scope: None,
    };
    let scopes = fx.chain(target);
    let principal_scopes = fx.chain("alice-user");
    pdp.authorize(
        &alice,
        Action::MemoryRead,
        Resource::Scope(fx.node(target).id),
        &AuthzContext {
            scopes: &scopes,
            principal_scopes: &principal_scopes,
            assignments,
            ..Default::default()
        },
    )
    .expect("authorize")
    .allowed
}

/// The AC's decision flip at the facade: move team-b between
/// departments and the very next decision — served through the entity
/// store — reflects the new chain; move it back and the read returns.
/// If the store answered by chain head instead of chain shape, the warm
/// pre-move fragment would keep team-b readable after the move.
#[test]
fn a_moved_team_is_never_decided_from_stale_fragments() {
    let pdp = Pdp::new().expect("build pdp");
    let mut fx = fixture();
    let assignments = [PolicyAssignment {
        tenant_id: fx.tenant,
        scope_id: fx.node("org").id,
        pack_name: STANDARD.to_owned(),
        updated_at: Utc::now(),
    }];

    // Warm the store: repeat decisions serve team-b's fragment from
    // memory (`standard`'s department rule: team-b is in alice's
    // department).
    assert!(alice_reads(&pdp, &fx, "team-b", &assignments));
    assert!(alice_reads(&pdp, &fx, "team-b", &assignments));

    // team-b moves to sales: alice's department no longer contains it.
    fx.move_node("team-b", "sales");
    assert!(
        !alice_reads(&pdp, &fx, "team-b", &assignments),
        "the warm fragment must not survive the move"
    );

    // And back: the fragment rebuilds again, no flush in between.
    fx.move_node("team-b", "eng");
    assert!(
        alice_reads(&pdp, &fx, "team-b", &assignments),
        "moving back must restore the department read"
    );
}

/// Flushing a tenant's fragments (the gateway seam) changes no decision
/// — the store is a cache, never an authority — and other tenants'
/// decisions are untouched.
#[test]
fn flush_changes_no_decision() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let other = fixture();
    let assignments = [PolicyAssignment {
        tenant_id: fx.tenant,
        scope_id: fx.node("org").id,
        pack_name: STANDARD.to_owned(),
        updated_at: Utc::now(),
    }];
    let other_assignments = [PolicyAssignment {
        tenant_id: other.tenant,
        scope_id: other.node("org").id,
        pack_name: STANDARD.to_owned(),
        updated_at: Utc::now(),
    }];

    assert!(alice_reads(&pdp, &fx, "team-b", &assignments));
    assert!(alice_reads(&pdp, &other, "team-b", &other_assignments));

    pdp.flush_entities(fx.tenant);

    assert!(alice_reads(&pdp, &fx, "team-b", &assignments));
    assert!(alice_reads(&pdp, &other, "team-b", &other_assignments));
}
