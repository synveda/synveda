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

use synveda_policy::ScopeNode;
use synveda_policy::{Action, AuthzContext, Pdp, Principal, Resource};
use synveda_types::access::RoleKey;
use synveda_types::anchor::{AnchorSource, ScopeAnchor};
use synveda_types::scope::ScopeKind;
use synveda_types::{ScopeId, Sensitivity, TenantId};

struct Fixture {
    tenant: TenantId,
    nodes: Vec<ScopeNode>,
}

impl Fixture {
    fn node(&self, slug: &str) -> &ScopeNode {
        self.nodes
            .iter()
            .find(|node| node.slug == slug)
            .unwrap_or_else(|| panic!("fixture has no node {slug}"))
    }

    /// The chain the PDP takes: the old hierarchy's rows projected onto
    /// the shape vocabulary at the caller's edge (CPR-6, ADR-0073
    /// decision 1). The fixture still holds `ScopeNode`s because the
    /// hierarchy plane still exists; nothing below this line does.
    fn chain(&self, slug: &str) -> Vec<ScopeNode> {
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

    fn anchor(&self, slug: Option<&str>, roles: &[RoleKey]) -> ScopeAnchor {
        let node = slug.map_or(self.nodes.first().expect("fixture has a root"), |slug| {
            self.node(slug)
        });
        ScopeAnchor {
            scope_id: node.id,
            kind: node.kind,
            parent_scope_id: node.parent_id,
            depth: 0,
            source: AnchorSource::Grant,
            roles: roles.to_vec(),
            granted_at: vec![node.id],
            via_groups: Vec::new(),
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
    let mut add = |parent: Option<ScopeId>, kind: ScopeKind, slug: &str, _depth: i32| -> ScopeId {
        let id = ScopeId::new();
        nodes.push(ScopeNode {
            id,
            tenant_id: tenant,
            parent_id: parent,
            kind,
            slug: slug.to_owned(),
            sealed: false,
        });
        id
    };
    let org = add(None, ScopeKind::Tenant, "org", 0);
    let eng = add(Some(org), ScopeKind::OrgUnit, "eng", 1);
    let sales = add(Some(org), ScopeKind::OrgUnit, "sales", 1);
    let team_a = add(Some(eng), ScopeKind::OrgUnit, "team-a", 2);
    let team_b = add(Some(eng), ScopeKind::OrgUnit, "team-b", 2);
    add(Some(sales), ScopeKind::OrgUnit, "team-c", 2);
    add(Some(team_a), ScopeKind::Principal, "agent-user", 3);
    add(Some(team_b), ScopeKind::Principal, "carol-user", 3);
    Fixture { tenant, nodes }
}

/// One decision through the facade, chains supplied as the gateway would.
fn decide(
    pdp: &Pdp,
    fx: &Fixture,
    principal: &Principal,
    action: Action,
    target: Option<&str>,
    anchors: &[ScopeAnchor],
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
    let decision = pdp
        .authorize(
            principal,
            action,
            resource,
            &AuthzContext {
                sensitivity: Some(Sensitivity::Internal),
                scopes: &scopes,
                principal_scopes: &principal_scopes,
                anchors,
                ..Default::default()
            },
        )
        .expect("authorize");
    if !decision.allowed && std::env::var("SCOPE_DEBUG").is_ok() {
        eprintln!(
            "DEBUG {action} target={:?} determining={:?}",
            target, decision.determining
        );
    }
    decision.allowed
}

/// The AC's policy half: a tenant-wide org-admin binding on the agent's
/// subject grants the full admin plane *inside* team-a and nothing
/// anywhere else — the forbid overrides every pack permit.
#[test]
fn token_scope_confines_the_admin_plane_regardless_of_roles() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let agent = fx.agent();
    let anchors = [fx.anchor(None, &[RoleKey::Administrator])];

    for action in [
        Action::ScopeCreate,
        Action::ScopeRead,
        Action::ScopeUpdate,
        Action::ScopeUpdate,
        Action::PolicyRead,
        Action::PolicyAssign,
        Action::ServiceIdentityRead,
        Action::ServiceIdentityManage,
    ] {
        // Inside the anchor subtree the grant works: team-a itself. The
        // agent's own scope is a principal-shaped scope — the privacy
        // floor owns it, asserted below.
        assert!(
            decide(&pdp, &fx, &agent, action, Some("team-a"), &anchors),
            "{action} on in-subtree team-a must be allowed"
        );
        // Outside it — ancestors included — the base forbid decides.
        for target in ["eng", "org", "sales", "team-b", "team-c", "carol-user"] {
            assert!(
                !decide(&pdp, &fx, &agent, action, Some(target), &anchors),
                "{action} on out-of-scope {target} must be denied to the confined agent"
            );
        }
    }

    // The tenant plane is never inside a scope subtree: unreachable, even
    // for a tenant-wide org-admin agent (ADR-0018 decision 4).
    for action in [
        Action::ScopeCreate,
        Action::ScopeRead,
        Action::PolicyRead,
        Action::PolicyAssign,
        Action::ServiceIdentityRead,
        // The audit chain included: `AuditRead` reaches only the tenant
        // resource (ADR-0045 decision 2), and the tenant plane is never
        // inside a scope subtree — so no service identity can read the
        // trail however it is bound, and the confinement forbid gets that
        // for free rather than by naming a new action.
        Action::AuditRead,
    ] {
        assert!(
            !decide(&pdp, &fx, &agent, action, None, &anchors),
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
        Action::ScopeUpdate,
        Some("team-a"),
        &anchors,
    ));
    assert!(!decide(
        &pdp,
        &fx,
        &agent,
        Action::ScopeUpdate,
        Some("eng"),
        &anchors,
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
            decide(&pdp, &fx, &agent, Action::MemoryRead, Some(target), &[]),
            "own-chain MemoryRead on {target} must survive confinement"
        );
    }
    for target in ["team-b", "team-c", "sales", "carol-user"] {
        assert!(
            !decide(&pdp, &fx, &agent, Action::MemoryRead, Some(target), &[]),
            "off-chain MemoryRead on {target} must be denied"
        );
    }

    // A viewer binding at eng would grant a *user* the eng subtree
    // (team-b included); the agent's token scope clamps it to team-a's,
    // while the own-chain floor is untouched.
    let anchors = [fx.anchor(Some("eng"), &[RoleKey::Viewer])];
    assert!(decide(
        &pdp,
        &fx,
        &agent,
        Action::MemoryRead,
        Some("team-a"),
        &anchors,
    ));
    assert!(
        !decide(
            &pdp,
            &fx,
            &agent,
            Action::MemoryRead,
            Some("team-b"),
            &anchors,
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
    let anchors = [fx.anchor(Some("eng"), &[RoleKey::Viewer])];
    assert!(decide(
        &pdp,
        &fx,
        &user,
        Action::MemoryRead,
        Some("team-b"),
        &anchors
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
            &[]
        ),
        "the agent's observe writes must land at its personal leaf"
    );
    for target in ["team-a", "eng", "org", "team-b", "team-c", "carol-user"] {
        assert!(
            !decide(&pdp, &fx, &agent, Action::MemoryWrite, Some(target), &[]),
            "role-free MemoryWrite on {target} must be denied"
        );
    }

    // A contributor binding at eng: for a user it grants the eng subtree;
    // the agent's token scope clamps it to team-a's subtree.
    let anchors = [fx.anchor(Some("eng"), &[RoleKey::Member])];
    assert!(
        decide(
            &pdp,
            &fx,
            &agent,
            Action::MemoryWrite,
            Some("team-a"),
            &anchors
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
                &anchors
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
    let anchors = [fx.anchor(Some("eng"), &[RoleKey::Member])];
    assert!(decide(
        &pdp,
        &fx,
        &user,
        Action::MemoryWrite,
        Some("team-b"),
        &anchors
    ));
}

/// The carve-out PRMT-1 widened, PRMT-2 widened again and SKIL-1 widened a
/// third time, and the four things none of them widened (ADR-0049
/// decision 4; ADR-0050 decision 8; ADR-0051 decision 10).
///
/// A headless agent is the consumer prompts exist for, and the org's
/// `house-style` sits two levels above the anchor — so without `PromptRead`
/// beside `MemoryRead` in the base layer's confinement forbid, the registry
/// would be unreadable by exactly the callers it is for. `ContextPackRead`
/// is there for a stronger version of the same reason: pack material
/// composes through `inject`, off the same plan walk that already reaches
/// the agent's own chain for memory, so leaving it out would mean an agent
/// composed the org's memory and not the org's conventions in one block.
/// `SkillRead` is there on the plainest version of it: an agent that cannot
/// resolve the org's skills cannot do the work they were published for.
///
/// What the carve-out stays is the *membership* floor: own chain, reads
/// only. This test is the evidence for that sentence, because a carve-out
/// is the kind of thing that is widened once and then assumed to be narrow
/// — and it runs over every authored asset type, so widening a fourth
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
        (Action::SkillRead, Action::SkillWrite, "skills"),
    ] {
        // 1. Up the chain, role-free: the anchor, its department, the org,
        //    and the agent's own leaf. This is the zero-config resolution.
        for target in ["agent-user", "team-a", "eng", "org"] {
            assert!(
                decide(&pdp, &fx, &agent, read, Some(target), &[]),
                "a team-anchored agent must resolve {what} at {target}"
            );
        }

        // 2. Off the chain: outside the anchor and not an ancestor of it.
        //    The carve-out is `principal in resource`, so a sibling team and
        //    another department are refused exactly as they are for memory.
        for target in ["team-b", "team-c", "sales", "carol-user"] {
            assert!(
                !decide(&pdp, &fx, &agent, read, Some(target), &[]),
                "{target} is off the agent's chain and its {what} must stay \
                 unreadable"
            );
        }

        // 3. A role cannot widen it. The whole point of the confinement
        //    forbid is that it beats every permit a pack adds, and a viewer
        //    binding at the department is the widest content grant a pack
        //    offers.
        let anchors = [fx.anchor(Some("eng"), &[RoleKey::Viewer])];
        for target in ["team-b", "carol-user"] {
            assert!(
                !decide(&pdp, &fx, &agent, read, Some(target), &anchors),
                "a role must not widen {read} past the token scope ({target})"
            );
        }

        // 4. Writes are not in the carve-out. The agent authors at its own
        //    leaf — inside the anchor, so no carve-out is needed — and
        //    nowhere above it, however it is bound.
        assert!(
            decide(&pdp, &fx, &agent, write, Some("agent-user"), &[]),
            "the agent's own leaf is inside its anchor"
        );
        let anchors = [fx.anchor(Some("eng"), &[RoleKey::Member])];
        for target in ["eng", "org", "team-b"] {
            assert!(
                !decide(&pdp, &fx, &agent, write, Some(target), &anchors),
                "{write} at {target} is outside the anchor and is not a read"
            );
        }
        // Inside the anchor a content role still works, exactly as it does
        // for memory: confinement clamps the grant rather than removing it.
        assert!(
            decide(&pdp, &fx, &agent, write, Some("team-a"), &anchors),
            "the contributor grant works where the token reaches"
        );

        // 5. And a user with the same bindings is untouched by any of it —
        //    the `has token_scope` guard makes the forbid a no-op for people.
        let user = Principal {
            token_scope: None,
            subject: "carol".to_owned(),
            ..fx.agent()
        };
        let anchors = [fx.anchor(Some("eng"), &[RoleKey::Viewer])];
        assert!(decide(&pdp, &fx, &user, read, Some("team-b"), &anchors));
    }
}
