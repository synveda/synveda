//! AUTHZ-4: the lapse permit (ADR-0037).
//!
//! What a standing grant does to a `MemoryRead` decision, and — mattering
//! more — what it does not. Every case runs through the ordinary
//! `authorize` facade with caller-supplied rows, the shape the gateway
//! feeds it; there is no bypass here and no direct policy evaluation
//! (CLAUDE.md, seed §2.2).
//!
//! The fixture is `packs.rs`'s, because the question this feature answers
//! is precisely the one that pack's golden says is impossible:
//!
//! ```text
//! org
//! ├── eng (department)
//! │   ├── team-a ── alice-user   ← alice's placement
//! │   └── team-b ── carol-user
//! └── sales (department)
//!     └── team-c ── dave-user
//! ```
//!
//! Under `regulated-strict` alice reads her own chain and nothing else.
//! A lapse from team-b to team-a is the sanctioned, time-boxed exception,
//! and every test below is about the edge of it.

use chrono::{TimeDelta, Utc};
use synveda_policy::{
    Action, AuthzContext, OPEN_COLLABORATION, Pdp, Principal, REGULATED_STRICT, Resource, STANDARD,
    ScopeNode, lapsable, lapsed_scopes,
};
use synveda_types::scope::ScopeKind;
use synveda_types::{
    ApprovalMatrix, IdentityId, Lapse, LapseAction, LapseConfig, LapseId, PackConfig,
    PolicyAssignment, ProposalId, ScopeId, Sensitivity, TenantId,
};

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
    /// decision 1). The fixture still holds `HierarchyNode`s because the
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
        chain.clone()
    }

    fn assignment(&self, slug: &str, pack: &str) -> PolicyAssignment {
        PolicyAssignment {
            tenant_id: self.tenant,
            scope_id: self.node(slug).id,
            pack_name: pack.to_owned(),
            updated_at: Utc::now(),
        }
    }

    fn placed(&self, subject: &str, slug: &str) -> Principal {
        Principal {
            tenant_id: self.tenant,
            subject: subject.to_owned(),
            quarantined: false,
            scope_id: Some(self.node(slug).id),
            token_scope: None,
        }
    }

    /// A grant from `grantee` to `target`, standing for an hour at the
    /// working tier — what every grant meant before AUTHZ-5 gave a lapse a
    /// declared ceiling (ADR-0038 decision 6).
    fn lapse(&self, grantee: &str, target: &str) -> Lapse {
        self.lapse_at(grantee, target, Sensitivity::Internal)
    }

    /// A grant that declares how sensitive the material it discloses may be.
    fn lapse_at(&self, grantee: &str, target: &str, max_sensitivity: Sensitivity) -> Lapse {
        let now = Utc::now();
        Lapse {
            id: LapseId::new(),
            tenant_id: self.tenant,
            proposal_id: ProposalId::new(),
            grantee_scope_id: self.node(grantee).id,
            target_scope_id: self.node(target).id,
            action: LapseAction::MemoryRead,
            max_sensitivity,
            reason: "joint incident review".to_owned(),
            granted_at: now,
            expires_at: now + TimeDelta::hours(1),
            granted_by: IdentityId::new(),
            revoked_at: None,
            revoked_by: None,
            revoke_reason: None,
            expiry_recorded_at: None,
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
    let team_c = add(Some(sales), ScopeKind::OrgUnit, "team-c", 2);
    add(Some(team_a), ScopeKind::Principal, "alice-user", 3);
    add(Some(team_b), ScopeKind::Principal, "carol-user", 3);
    add(Some(team_c), ScopeKind::Principal, "dave-user", 3);
    Fixture { tenant, nodes }
}

/// One `MemoryRead` decision, the gateway's shape: the resource's own
/// chain, the principal's placement chain, and the grants standing over
/// that principal.
fn read(
    pdp: &Pdp,
    fx: &Fixture,
    principal: &Principal,
    placement: &str,
    target: &str,
    assignments: &[PolicyAssignment],
    lapses: &[Lapse],
) -> bool {
    let scopes = fx.chain(target);
    let principal_scopes = fx.chain(placement);
    pdp.authorize(
        principal,
        Action::MemoryRead,
        Resource::Scope(fx.node(target).id),
        &AuthzContext {
            sensitivity: Some(Sensitivity::Internal),
            scopes: &scopes,
            principal_scopes: &principal_scopes,
            assignments,
            lapses,
            ..Default::default()
        },
    )
    .expect("authorize")
    .allowed
}

/// The AC's authorization half, and the fact that makes it a feature: the
/// same decision, same pack, same principal, flips on the presence of one
/// row — and flips back when it is gone.
#[test]
fn a_grant_opens_a_cross_team_read_that_the_pack_forbids() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let alice = fx.placed("alice", "alice-user");
    let assignments = [fx.assignment("org", REGULATED_STRICT)];

    assert!(
        !read(&pdp, &fx, &alice, "alice-user", "team-b", &assignments, &[]),
        "regulated-strict has no cross-team read: that is what the lapse is for"
    );

    let grant = [fx.lapse("team-a", "team-b")];
    assert!(
        read(
            &pdp,
            &fx,
            &alice,
            "alice-user",
            "team-b",
            &assignments,
            &grant
        ),
        "a standing grant to alice's team opens team-b"
    );

    // "Expiry restores denial" is this, from the decision's side: the row
    // stops being supplied because the store's predicate dropped it, and
    // nothing else changes.
    assert!(
        !read(&pdp, &fx, &alice, "alice-user", "team-b", &assignments, &[]),
        "with the grant gone the decision is what it always was"
    );
}

/// The grant reaches its target and nothing beside it. A lapse names one
/// scope; the material below it reaches a reader through what that scope
/// published, which is the set the approvers could inspect.
#[test]
fn a_grant_reaches_the_target_and_not_its_neighbours_or_its_subtree() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let alice = fx.placed("alice", "alice-user");
    let assignments = [fx.assignment("org", REGULATED_STRICT)];
    let grant = [fx.lapse("team-a", "team-b")];

    for off_target in ["team-c", "sales", "carol-user"] {
        assert!(
            !read(
                &pdp,
                &fx,
                &alice,
                "alice-user",
                off_target,
                &assignments,
                &grant
            ),
            "a grant naming team-b must not reach {off_target}"
        );
    }
}

/// The privacy floor: no lapse opens somebody's personal scope, and the
/// rule lives in the permit rather than in the grant surface — so it holds
/// even for a row that reached the table by some other road.
#[test]
fn no_grant_opens_a_personal_scope() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let alice = fx.placed("alice", "alice-user");
    // A grant naming carol's own scope directly — what the grant surface
    // refuses by name, evaluated here as though it had been written anyway.
    let grant = [fx.lapse("team-a", "carol-user")];

    for pack in [REGULATED_STRICT, STANDARD, OPEN_COLLABORATION] {
        let assignments = [fx.assignment("org", pack)];
        assert!(
            !read(
                &pdp,
                &fx,
                &alice,
                "alice-user",
                "carol-user",
                &assignments,
                &grant
            ),
            "{pack}: a lapse must never disclose a personal scope"
        );
    }
}

/// A grant is to a *scope*, so it reaches everyone placed at or under it
/// and nobody else — which is what makes "team X may read team Y" one row
/// that tracks membership rather than N rows that do not.
#[test]
fn a_grant_reaches_the_grantees_subtree_and_stops_there() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let assignments = [fx.assignment("org", REGULATED_STRICT)];
    let grant = [fx.lapse("team-a", "team-b")];

    // Alice is under team-a: she has it.
    let alice = fx.placed("alice", "alice-user");
    assert!(read(
        &pdp,
        &fx,
        &alice,
        "alice-user",
        "team-b",
        &assignments,
        &grant
    ));

    // Dave is under sales/team-c, holding the same rows: he does not.
    let dave = fx.placed("dave", "dave-user");
    assert!(
        !read(
            &pdp,
            &fx,
            &dave,
            "dave-user",
            "team-b",
            &assignments,
            &grant
        ),
        "a grant to team-a is not a grant to everyone who can see the row"
    );

    // A grant at the department reaches everyone under it. Carol is the
    // case that proves it: she is under eng, and team-a is *not* on her own
    // chain, so nothing but the grant can be permitting this. (Alice would
    // prove nothing here — team-a is her own team, and the membership floor
    // would allow it with no lapse in sight.)
    let wide = [fx.lapse("eng", "team-a")];
    let carol = fx.placed("carol", "carol-user");
    assert!(
        !read(&pdp, &fx, &carol, "carol-user", "team-a", &assignments, &[]),
        "team-a is off carol's chain: the pack alone denies it"
    );
    assert!(
        read(
            &pdp,
            &fx,
            &carol,
            "carol-user",
            "team-a",
            &assignments,
            &wide
        ),
        "a department-wide grantee reaches every team under it"
    );
    assert!(
        !read(&pdp, &fx, &dave, "dave-user", "team-a", &assignments, &wide),
        "sales is not under eng, so the same row reaches dave with nothing"
    );
}

/// A pack whose ceiling is zero admits no lapse **at decision time**, not
/// merely at grant time: flipping the pack ends every standing grant at
/// scopes it governs on the very next request (ADR-0014 decision 3's
/// doctrine, ADR-0037 decision 5).
#[test]
fn a_pack_that_admits_no_lapses_ends_the_standing_ones_too() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let alice = fx.placed("alice", "alice-user");
    let grant = [fx.lapse("team-a", "team-b")];

    // A stored pack identical to the product default but for the ceiling.
    let source = include_str!("../src/packs/regulated-strict.cedar");
    pdp.install_source(
        fx.tenant,
        "acme-no-lapses",
        1,
        source,
        PackConfig {
            approvals: Some(ApprovalMatrix::empty()),
            lapse: Some(LapseConfig::NONE),
            ..Default::default()
        },
    )
    .expect("install");

    let assignments = [fx.assignment("org", REGULATED_STRICT)];
    assert!(read(
        &pdp,
        &fx,
        &alice,
        "alice-user",
        "team-b",
        &assignments,
        &grant
    ));

    // The pack is resolved from the **target's** chain, so assigning it at
    // team-b is what governs a read of team-b.
    let refused = [fx.assignment("team-b", "acme-no-lapses")];
    assert!(
        !read(&pdp, &fx, &alice, "alice-user", "team-b", &refused, &grant),
        "the target's pack decides whether a grant over it stands at all"
    );
}

/// Quarantine and token confinement are forbids, and a forbid beats the
/// base layer's permit. Neither needed a clause in this feature.
#[test]
fn a_forbid_still_beats_the_permit() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let assignments = [fx.assignment("org", REGULATED_STRICT)];
    let grant = [fx.lapse("team-a", "team-b")];

    let quarantined = Principal {
        quarantined: true,
        ..fx.placed("alice", "alice-user")
    };
    assert!(
        !read(
            &pdp,
            &fx,
            &quarantined,
            "alice-user",
            "team-b",
            &assignments,
            &grant
        ),
        "a quarantined principal is forbidden everything, grant or no grant"
    );

    // A service identity anchored at team-a: the confinement forbid carves
    // out own-chain MemoryRead only, so a lapse cannot widen its token.
    let agent = Principal {
        token_scope: Some(fx.node("team-a").id),
        ..fx.placed("agent", "alice-user")
    };
    assert!(
        !read(
            &pdp,
            &fx,
            &agent,
            "alice-user",
            "team-b",
            &assignments,
            &grant
        ),
        "an agent credential must not be widened past its anchor by a lapse"
    );
    // The same agent still composes its own chain, so the forbid has not
    // simply denied everything.
    assert!(read(
        &pdp,
        &fx,
        &agent,
        "alice-user",
        "team-a",
        &assignments,
        &grant
    ));
}

/// The lapse vocabulary is closed at the PDP too: `context.lapsed` is only
/// ever set for an action a lapse may relax, so a grant row cannot widen a
/// write or an admin plane even if one somehow named it.
#[test]
fn only_the_lapsable_action_carries_a_grant() {
    assert_eq!(lapsable(Action::MemoryRead), Some(LapseAction::MemoryRead));
    for action in [
        Action::MemoryWrite,
        Action::PolicyAssign,
        Action::ChannelPublish,
        Action::ProposalReview,
        Action::LapseGrant,
        Action::LapseRevoke,
    ] {
        assert_eq!(lapsable(action), None, "{action} must not be lapsable");
    }

    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let alice = fx.placed("alice", "alice-user");
    let assignments = [fx.assignment("org", REGULATED_STRICT)];
    let grant = [fx.lapse("team-a", "team-b")];
    let scopes = fx.chain("team-b");
    let principal_scopes = fx.chain("alice-user");
    let decision = pdp
        .authorize(
            &alice,
            Action::MemoryWrite,
            Resource::Scope(fx.node("team-b").id),
            &AuthzContext {
                sensitivity: Some(Sensitivity::Internal),
                scopes: &scopes,
                principal_scopes: &principal_scopes,
                assignments: &assignments,
                lapses: &grant,
                ..Default::default()
            },
        )
        .expect("authorize");
    assert!(
        !decision.allowed,
        "a read grant must not become a write grant"
    );
}

/// The plan and the permit share one containment rule, and the plan's half
/// drops what the caller already reaches: a target on the caller's own
/// chain is not reached "only by lapse", and two grants naming one target
/// are one plan entry.
#[test]
fn the_plan_offers_each_off_chain_target_once_and_in_a_stable_order() {
    let fx = fixture();
    let chain = fx.chain("alice-user");

    let older = fx.lapse("team-a", "team-b");
    let newer = Lapse {
        granted_at: older.granted_at + TimeDelta::seconds(1),
        ..fx.lapse("alice-user", "team-c")
    };
    // A second grant naming a target the first already named.
    let duplicate = Lapse {
        granted_at: older.granted_at + TimeDelta::seconds(2),
        ..fx.lapse("eng", "team-b")
    };
    // And one naming a scope alice is already under.
    let redundant = Lapse {
        granted_at: older.granted_at + TimeDelta::seconds(3),
        ..fx.lapse("team-a", "eng")
    };

    let rows = [
        duplicate.clone(),
        redundant.clone(),
        newer.clone(),
        older.clone(),
    ];
    let offered = lapsed_scopes(&chain, &rows, LapseAction::MemoryRead);
    let targets: Vec<ScopeId> = offered.iter().map(|lapse| lapse.target_scope_id).collect();
    assert_eq!(
        targets,
        vec![older.target_scope_id, newer.target_scope_id],
        "oldest grant first, one entry per target, and nothing already on the chain"
    );
    assert_eq!(
        offered[0].id, older.id,
        "the earlier grant is the one that carries the target"
    );

    // A grant to somebody else's team reaches nothing here.
    let elsewhere = [fx.lapse("sales", "team-b")];
    assert!(
        lapsed_scopes(&chain, &elsewhere, LapseAction::MemoryRead).is_empty(),
        "a grant alice is not under offers her nothing"
    );
}
