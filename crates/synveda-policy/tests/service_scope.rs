//! AUTH-3 (ADR-0018 decision 4): the base layer's token-scope confinement.
//! A service identity's decisions are bounded by its anchor subtree
//! *regardless of bound roles* — a tenant-wide org-admin binding on an
//! agent subject cannot escape — with exactly one carve-out: the
//! role-free membership floor (own-chain `MemoryRead`), so a team agent
//! still composes team → department → org. Tenant-plane resources are
//! unreachable outright. Users carry no `token_scope` and are untouched.
//!
//! The fixture (as in tests/roles.rs), with the agent anchored at team-a:
//!
//! ```text
//! org
//! ├── eng (department)
//! │   ├── team-a ── agent-user   ← token_scope = team-a
//! │   └── team-b ── carol-user
//! └── sales (department)
//!     └── team-c
//! ```

use chrono::Utc;
use synveda_policy::{Action, AuthzContext, Pdp, Principal, Resource};
use synveda_types::{HierarchyNode, Role, RoleBinding, ScopeId, ScopeKind, Sensitivity, TenantId};

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

    fn binding(&self, subject: &str, slug: Option<&str>, role: Role) -> RoleBinding {
        RoleBinding {
            tenant_id: self.tenant,
            subject: subject.to_owned(),
            scope_id: slug.map(|slug| self.node(slug).id),
            role,
            updated_at: Utc::now(),
        }
    }

    /// The agent: placed at its personal leaf under team-a, confined to
    /// team-a — exactly what the gateway seam builds for a service
    /// identity (ADR-0018 decision 4).
    fn agent(&self) -> Principal {
        Principal {
            tenant_id: self.tenant,
            subject: "ci-agent".to_owned(),
            quarantined: false,
            scope_id: Some(self.node("agent-user").id),
            token_scope: Some(self.node("team-a").id),
        }
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
    let sales = add(Some(org), ScopeKind::Department, "sales", 1);
    let team_a = add(Some(eng), ScopeKind::Team, "team-a", 2);
    let team_b = add(Some(eng), ScopeKind::Team, "team-b", 2);
    add(Some(sales), ScopeKind::Team, "team-c", 2);
    add(Some(team_a), ScopeKind::User, "agent-user", 3);
    add(Some(team_b), ScopeKind::User, "carol-user", 3);
    Fixture { tenant, nodes }
}

/// One decision through the facade, chains supplied as the gateway would.
fn decide(
    pdp: &Pdp,
    fx: &Fixture,
    principal: &Principal,
    action: Action,
    target: Option<&str>,
    bindings: &[RoleBinding],
    grant: Option<Role>,
) -> bool {
    let scopes = target.map(|slug| fx.chain(slug)).unwrap_or_default();
    let principal_scopes = if principal.scope_id.is_some() {
        fx.chain("agent-user")
    } else {
        Vec::new()
    };
    let resource = match target {
        Some(slug) => Resource::Scope(fx.node(slug).id),
        None => Resource::Tenant(fx.tenant),
    };
    pdp.authorize(
        principal,
        action,
        resource,
        &AuthzContext {
            sensitivity: Some(Sensitivity::Internal),
            scopes: &scopes,
            principal_scopes: &principal_scopes,
            role_bindings: bindings,
            grant,
            ..Default::default()
        },
    )
    .expect("authorize")
    .allowed
}

/// The AC's policy half: a tenant-wide org-admin binding on the agent's
/// subject grants the full admin plane *inside* team-a and nothing
/// anywhere else — the forbid overrides every pack permit.
#[test]
fn token_scope_confines_the_admin_plane_regardless_of_roles() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let agent = fx.agent();
    let bindings = [fx.binding("ci-agent", None, Role::OrgAdmin)];

    for action in [
        Action::HierarchyCreate,
        Action::HierarchyRead,
        Action::HierarchyUpdate,
        Action::HierarchyDelete,
        Action::PolicyRead,
        Action::PolicyAssign,
        Action::RoleRead,
        Action::ServiceIdentityRead,
        Action::ServiceIdentityManage,
    ] {
        // Inside the anchor subtree the binding works: team-a itself and
        // the agent's own leaf.
        for target in ["team-a", "agent-user"] {
            assert!(
                decide(&pdp, &fx, &agent, action, Some(target), &bindings, None),
                "{action} on in-subtree {target} must be allowed"
            );
        }
        // Outside it — ancestors included — the base forbid decides.
        for target in ["eng", "org", "sales", "team-b", "team-c", "carol-user"] {
            assert!(
                !decide(&pdp, &fx, &agent, action, Some(target), &bindings, None),
                "{action} on out-of-scope {target} must be denied to the confined agent"
            );
        }
    }

    // The tenant plane is never inside a scope subtree: unreachable, even
    // for a tenant-wide org-admin agent (ADR-0018 decision 4).
    for (action, grant) in [
        (Action::HierarchyCreate, None),
        (Action::HierarchyRead, None),
        (Action::PolicyRead, None),
        (Action::PolicyAssign, None),
        (Action::RoleRead, None),
        (Action::RoleAssign, Some(Role::Viewer)),
        (Action::ServiceIdentityRead, None),
        // The audit chain included: `AuditRead` reaches only the tenant
        // resource (ADR-0045 decision 2), and the tenant plane is never
        // inside a scope subtree — so no service identity can read the
        // trail however it is bound, and the confinement forbid gets that
        // for free rather than by naming a new action.
        (Action::AuditRead, None),
    ] {
        assert!(
            !decide(&pdp, &fx, &agent, action, None, &bindings, grant),
            "{action} on the tenant plane must be denied to a service token"
        );
    }

    // RoleAssign inside the subtree still works (delegation stays
    // possible where the token reaches) and still cannot mint org-admin
    // without holding it — both guards compose.
    assert!(decide(
        &pdp,
        &fx,
        &agent,
        Action::RoleAssign,
        Some("team-a"),
        &bindings,
        Some(Role::Viewer),
    ));
    assert!(!decide(
        &pdp,
        &fx,
        &agent,
        Action::RoleAssign,
        Some("eng"),
        &bindings,
        Some(Role::Viewer),
    ));
}

/// The carve-out is exactly the membership floor: own-chain `MemoryRead`
/// survives confinement (a team agent composes team → dept → org on
/// inject like any placed member), while roles that would widen reads
/// beyond the subtree are clamped.
#[test]
fn memory_read_keeps_the_own_chain_floor_and_nothing_more() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let agent = fx.agent();

    // Role-free: the floor, upward only (regulated-strict is the
    // zero-config default pack).
    for target in ["agent-user", "team-a", "eng", "org"] {
        assert!(
            decide(
                &pdp,
                &fx,
                &agent,
                Action::MemoryRead,
                Some(target),
                &[],
                None
            ),
            "own-chain MemoryRead on {target} must survive confinement"
        );
    }
    for target in ["team-b", "team-c", "sales", "carol-user"] {
        assert!(
            !decide(
                &pdp,
                &fx,
                &agent,
                Action::MemoryRead,
                Some(target),
                &[],
                None
            ),
            "off-chain MemoryRead on {target} must be denied"
        );
    }

    // A viewer binding at eng would grant a *user* the eng subtree
    // (team-b included); the agent's token scope clamps it to team-a's,
    // while the own-chain floor is untouched.
    let bindings = [fx.binding("ci-agent", Some("eng"), Role::Viewer)];
    assert!(decide(
        &pdp,
        &fx,
        &agent,
        Action::MemoryRead,
        Some("team-a"),
        &bindings,
        None,
    ));
    assert!(
        !decide(
            &pdp,
            &fx,
            &agent,
            Action::MemoryRead,
            Some("team-b"),
            &bindings,
            None,
        ),
        "a role must not widen MemoryRead beyond the token scope"
    );

    // Contrast: the same principal shape without a token scope (a user)
    // does get team-b through the eng viewer binding — the clamp is the
    // token scope, not the role machinery.
    let user = Principal {
        token_scope: None,
        subject: "carol".to_owned(),
        ..fx.agent()
    };
    let bindings = [fx.binding("carol", Some("eng"), Role::Viewer)];
    assert!(decide(
        &pdp,
        &fx,
        &user,
        Action::MemoryRead,
        Some("team-b"),
        &bindings,
        None,
    ));
}

/// The write seam under confinement (MEM-1, ADR-0020 decision 3): the
/// agent's own personal leaf sits *inside* its anchor subtree, so the
/// role-free write floor needs no new base-layer carve-out — and a
/// content-role binding grants writes only where the token reaches.
#[test]
fn memory_write_lands_at_home_and_confinement_clamps_the_grant() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let agent = fx.agent();

    // Role-free: home, and home only — observe's exact question.
    assert!(
        decide(
            &pdp,
            &fx,
            &agent,
            Action::MemoryWrite,
            Some("agent-user"),
            &[],
            None
        ),
        "the agent's observe writes must land at its personal leaf"
    );
    for target in ["team-a", "eng", "org", "team-b", "team-c", "carol-user"] {
        assert!(
            !decide(
                &pdp,
                &fx,
                &agent,
                Action::MemoryWrite,
                Some(target),
                &[],
                None
            ),
            "role-free MemoryWrite on {target} must be denied"
        );
    }

    // A contributor binding at eng: for a user it grants the eng subtree;
    // the agent's token scope clamps it to team-a's subtree.
    let bindings = [fx.binding("ci-agent", Some("eng"), Role::Contributor)];
    assert!(
        decide(
            &pdp,
            &fx,
            &agent,
            Action::MemoryWrite,
            Some("team-a"),
            &bindings,
            None,
        ),
        "the contributor grant works where the token reaches"
    );
    for target in ["team-b", "eng"] {
        assert!(
            !decide(
                &pdp,
                &fx,
                &agent,
                Action::MemoryWrite,
                Some(target),
                &bindings,
                None,
            ),
            "a role must not widen MemoryWrite beyond the token scope ({target})"
        );
    }

    // The same binding on a user (no token scope) does reach team-b.
    let user = Principal {
        token_scope: None,
        subject: "carol".to_owned(),
        ..fx.agent()
    };
    let bindings = [fx.binding("carol", Some("eng"), Role::Contributor)];
    assert!(decide(
        &pdp,
        &fx,
        &user,
        Action::MemoryWrite,
        Some("team-b"),
        &bindings,
        None,
    ));
}

/// The carve-out PRMT-1 widened and PRMT-2 widened again, and the four
/// things neither widened (ADR-0049 decision 4; ADR-0050 decision 8).
///
/// A headless agent is the consumer prompts exist for, and the org's
/// `house-style` sits two levels above the anchor — so without `PromptRead`
/// beside `MemoryRead` in the base layer's confinement forbid, the registry
/// would be unreadable by exactly the callers it is for. `ContextPackRead`
/// is there for a stronger version of the same reason: pack material
/// composes through `inject`, off the same plan walk that already reaches
/// the agent's own chain for memory, so leaving it out would mean an agent
/// composed the org's memory and not the org's conventions in one block.
///
/// What the carve-out stays is the *membership* floor: own chain, reads
/// only. This test is the evidence for that sentence, because a carve-out
/// is the kind of thing that is widened once and then assumed to be narrow
/// — and it now runs over both authored asset types, so widening a third
/// cannot quietly skip it.
#[test]
fn an_agent_reads_authored_assets_up_its_own_chain_and_nothing_else() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let agent = fx.agent();

    for (read, write, what) in [
        (Action::PromptRead, Action::PromptWrite, "prompts"),
        (
            Action::ContextPackRead,
            Action::ContextPackWrite,
            "context packs",
        ),
    ] {
        // 1. Up the chain, role-free: the anchor, its department, the org,
        //    and the agent's own leaf. This is the zero-config resolution.
        for target in ["agent-user", "team-a", "eng", "org"] {
            assert!(
                decide(&pdp, &fx, &agent, read, Some(target), &[], None),
                "a team-anchored agent must resolve {what} at {target}"
            );
        }

        // 2. Off the chain: outside the anchor and not an ancestor of it.
        //    The carve-out is `principal in resource`, so a sibling team and
        //    another department are refused exactly as they are for memory.
        for target in ["team-b", "team-c", "sales", "carol-user"] {
            assert!(
                !decide(&pdp, &fx, &agent, read, Some(target), &[], None),
                "{target} is off the agent's chain and its {what} must stay \
                 unreadable"
            );
        }

        // 3. A role cannot widen it. The whole point of the confinement
        //    forbid is that it beats every permit a pack adds, and a viewer
        //    binding at the department is the widest content grant a pack
        //    offers.
        let bindings = [fx.binding("ci-agent", Some("eng"), Role::Viewer)];
        for target in ["team-b", "carol-user"] {
            assert!(
                !decide(&pdp, &fx, &agent, read, Some(target), &bindings, None),
                "a role must not widen {read} past the token scope ({target})"
            );
        }

        // 4. Writes are not in the carve-out. The agent authors at its own
        //    leaf — inside the anchor, so no carve-out is needed — and
        //    nowhere above it, however it is bound.
        assert!(
            decide(&pdp, &fx, &agent, write, Some("agent-user"), &[], None),
            "the agent's own leaf is inside its anchor"
        );
        let bindings = [fx.binding("ci-agent", Some("eng"), Role::Contributor)];
        for target in ["eng", "org", "team-b"] {
            assert!(
                !decide(&pdp, &fx, &agent, write, Some(target), &bindings, None),
                "{write} at {target} is outside the anchor and is not a read"
            );
        }
        // Inside the anchor a content role still works, exactly as it does
        // for memory: confinement clamps the grant rather than removing it.
        assert!(
            decide(&pdp, &fx, &agent, write, Some("team-a"), &bindings, None),
            "the contributor grant works where the token reaches"
        );

        // 5. And a user with the same bindings is untouched by any of it —
        //    the `has token_scope` guard makes the forbid a no-op for people.
        let user = Principal {
            token_scope: None,
            subject: "carol".to_owned(),
            ..fx.agent()
        };
        let bindings = [fx.binding("carol", Some("eng"), Role::Viewer)];
        assert!(decide(
            &pdp,
            &fx,
            &user,
            read,
            Some("team-b"),
            &bindings,
            None,
        ));
    }
}
