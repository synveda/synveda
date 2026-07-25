//! AUTHZ-2 golden tests per pack (the AC): for a fixed two-department
//! fixture, the full `MemoryRead` decision matrix of each embedded product
//! pack, the `MemoryWrite` floor (MEM-1, ADR-0020 decision 3 — pack
//! uniform: own home only, role-free; the content-role write grant lives
//! in tests/roles.rs), the shared admin-plane semantics, and the
//! cross-cutting invariants (quarantine, unplaced principals, foreign
//! tenants). Packs are applied through the same assignment-resolution
//! path production uses — never a PDP bypass (CLAUDE.md, seed §2.2).
//!
//! The fixture:
//!
//! ```text
//! org
//! ├── eng (department)
//! │   ├── team-a ── alice-user   ← alice's placement
//! │   └── team-b ── carol-user
//! └── sales (department)
//!     └── team-c ── dave-user
//! ```

use chrono::Utc;
use synveda_policy::{
    Action, AuthzContext, OPEN_COLLABORATION, Pdp, Principal, REGULATED_STRICT, Resource, STANDARD,
};
use synveda_types::{HierarchyNode, PolicyAssignment, Role, ScopeId, ScopeKind, TenantId};

/// Every scope of the fixture — the candidate set a composition sweep
/// would consider.
const ALL_SCOPES: [&str; 8] = [
    "org",
    "eng",
    "sales",
    "team-a",
    "team-b",
    "team-c",
    "alice-user",
    "carol-user",
];

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

    /// The node and its ancestors — what a caller reads for the resource
    /// (and what the gateway reads for a principal's placement).
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

    fn assignment(&self, slug: &str, pack: &str) -> PolicyAssignment {
        PolicyAssignment {
            tenant_id: self.tenant,
            scope_id: self.node(slug).id,
            pack_name: pack.to_owned(),
            updated_at: Utc::now(),
        }
    }

    /// A principal placed at `slug`'s personal scope.
    fn placed(&self, subject: &str, slug: &str) -> Principal {
        Principal {
            tenant_id: self.tenant,
            subject: subject.to_owned(),
            quarantined: false,
            scope_id: Some(self.node(slug).id),
            token_scope: None,
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
    let team_c = add(Some(sales), ScopeKind::Team, "team-c", 2);
    add(Some(team_a), ScopeKind::User, "alice-user", 3);
    add(Some(team_b), ScopeKind::User, "carol-user", 3);
    add(Some(team_c), ScopeKind::User, "dave-user", 3);
    Fixture { tenant, nodes }
}

/// One memory-plane decision for `target`, with the principal's placement
/// chain materialised from `placement` and packs applied via
/// `assignments` — the exact shape the gateway feeds the facade.
fn memory(
    pdp: &Pdp,
    fx: &Fixture,
    principal: &Principal,
    action: Action,
    placement: Option<&str>,
    target: &str,
    assignments: &[PolicyAssignment],
) -> synveda_policy::AuthzDecision {
    let scopes = fx.chain(target);
    let principal_scopes = placement.map(|slug| fx.chain(slug)).unwrap_or_default();
    pdp.authorize(
        principal,
        action,
        Resource::Scope(fx.node(target).id),
        &AuthzContext {
            scopes: &scopes,
            principal_scopes: &principal_scopes,
            assignments,
            ..Default::default()
        },
    )
    .expect("authorize")
}

fn read(
    pdp: &Pdp,
    fx: &Fixture,
    principal: &Principal,
    placement: Option<&str>,
    target: &str,
    assignments: &[PolicyAssignment],
) -> synveda_policy::AuthzDecision {
    memory(
        pdp,
        fx,
        principal,
        Action::MemoryRead,
        placement,
        target,
        assignments,
    )
}

/// The composition sweep: which of the fixture's scopes may this
/// principal's inject composition include? (What CTX-2 will ask, ADR-0014
/// decision 5.)
fn composition(
    pdp: &Pdp,
    fx: &Fixture,
    principal: &Principal,
    placement: Option<&str>,
    assignments: &[PolicyAssignment],
) -> Vec<&'static str> {
    ALL_SCOPES
        .into_iter()
        .filter(|target| read(pdp, fx, principal, placement, target, assignments).allowed)
        .collect()
}

/// The write sweep: at which of the fixture's scopes may this principal
/// land memory content? (What observe asks for home, and FLOW will ask
/// beyond it — MEM-1, ADR-0020 decision 3.)
fn write_targets(
    pdp: &Pdp,
    fx: &Fixture,
    principal: &Principal,
    placement: Option<&str>,
    assignments: &[PolicyAssignment],
) -> Vec<&'static str> {
    ALL_SCOPES
        .into_iter()
        .filter(|target| {
            memory(
                pdp,
                fx,
                principal,
                Action::MemoryWrite,
                placement,
                target,
                assignments,
            )
            .allowed
        })
        .collect()
}

/// Asserts one pack's golden `MemoryRead` matrix for alice, plus the
/// invariants every pack shares: version stamping, admin-plane semantics,
/// unplaced/quarantined/foreign denial.
fn assert_pack_golden(pack: &str, version: i64, expected_for_alice: &[&str]) {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let alice = fx.placed("alice", "alice-user");
    // Applied per node: the org root carries the assignment, the whole
    // tenant inherits it (ADR-0014 decision 4).
    let assignments = [fx.assignment("org", pack)];

    // The golden matrix, target by target — a wrong allow and a wrong
    // deny both fail, and the failure names the pack and target.
    for target in ALL_SCOPES {
        let decision = read(&pdp, &fx, &alice, Some("alice-user"), target, &assignments);
        assert_eq!(
            decision.allowed,
            expected_for_alice.contains(&target),
            "{pack}: alice reading {target} decided {}",
            decision.allowed,
        );
        assert_eq!(decision.pack_name, pack, "{pack}: wrong pack for {target}");
        assert_eq!(decision.pack_version, version);
    }

    // Since AUTHZ-3 the admin planes require roles (ADR-0015 decision 4):
    // an unbound principal — placed or not — holds no administrative
    // power under any pack. The role×action matrix lives in
    // tests/roles.rs; here we pin the unbound baseline.
    let unplaced = Principal {
        tenant_id: fx.tenant,
        subject: "dev-token".to_owned(),
        quarantined: false,
        scope_id: None,
        token_scope: None,
    };
    for action in [
        Action::HierarchyCreate,
        Action::HierarchyRead,
        Action::HierarchyUpdate,
        Action::HierarchyDelete,
        Action::PolicyRead,
        Action::PolicyAssign,
        Action::RoleRead,
        Action::RoleAssign,
    ] {
        for principal in [&alice, &unplaced] {
            let scopes = fx.chain("team-b");
            let decision = pdp
                .authorize(
                    principal,
                    action,
                    Resource::Scope(fx.node("team-b").id),
                    &AuthzContext {
                        scopes: &scopes,
                        assignments: &assignments,
                        // RoleAssign requires the grant in context; the
                        // decision must still be a deny.
                        grant: (action == Action::RoleAssign).then_some(Role::Viewer),
                        ..Default::default()
                    },
                )
                .expect("authorize");
            assert!(
                !decision.allowed,
                "{pack}: {action} must be denied to the unbound {}",
                principal.subject
            );
        }
    }

    // The write floor is pack-uniform (MEM-1, ADR-0020 decision 3): a
    // role-free principal writes exactly its own personal scope, under
    // every pack — openness is a read-side property. The content-role
    // write grant is golden-tested in tests/roles.rs.
    assert_eq!(
        write_targets(&pdp, &fx, &alice, Some("alice-user"), &assignments),
        vec!["alice-user"],
        "{pack}: the role-free write floor is home, and home only"
    );

    // An unplaced principal composes nothing, under every pack — and has
    // no home, so it writes nothing either.
    assert_eq!(
        composition(&pdp, &fx, &unplaced, None, &assignments),
        Vec::<&str>::new(),
        "{pack}: an unplaced principal must read nothing"
    );
    assert_eq!(
        write_targets(&pdp, &fx, &unplaced, None, &assignments),
        Vec::<&str>::new(),
        "{pack}: an unplaced principal must write nothing"
    );

    // A quarantined principal is forbidden everything, admin included.
    let quarantined = Principal {
        quarantined: true,
        ..alice.clone()
    };
    assert_eq!(
        composition(&pdp, &fx, &quarantined, Some("alice-user"), &assignments),
        Vec::<&str>::new(),
        "{pack}: a quarantined principal must read nothing"
    );
    assert_eq!(
        write_targets(&pdp, &fx, &quarantined, Some("alice-user"), &assignments),
        Vec::<&str>::new(),
        "{pack}: a quarantined principal must write nothing"
    );

    // A foreign principal is denied everything, placed or not.
    let intruder = Principal {
        tenant_id: TenantId::new(),
        subject: "intruder".to_owned(),
        quarantined: false,
        scope_id: None,
        token_scope: None,
    };
    assert_eq!(
        composition(&pdp, &fx, &intruder, None, &assignments),
        Vec::<&str>::new(),
        "{pack}: a foreign principal must read nothing"
    );
    assert_eq!(
        write_targets(&pdp, &fx, &intruder, None, &assignments),
        Vec::<&str>::new(),
        "{pack}: a foreign principal must write nothing"
    );
}

/// regulated-strict: own chain only — no cross-team read exists at all
/// (seed §6; lapses, AUTHZ-4, are the sanctioned relaxation).
#[test]
fn golden_regulated_strict() {
    assert_pack_golden(REGULATED_STRICT, 6, &["org", "eng", "team-a", "alice-user"]);
}

/// standard: own chain plus the department subtree — sibling team-b joins;
/// sales, team-c, and carol's personal scope stay out.
#[test]
fn golden_standard() {
    assert_pack_golden(
        STANDARD,
        6,
        &["org", "eng", "team-a", "team-b", "alice-user"],
    );
}

/// open-collaboration: org-wide — only other people's personal scopes
/// stay out (the privacy floor until AUTHZ-5 classification).
#[test]
fn golden_open_collaboration() {
    assert_pack_golden(
        OPEN_COLLABORATION,
        6,
        &[
            "org",
            "eng",
            "sales",
            "team-a",
            "team-b",
            "team-c",
            "alice-user",
        ],
    );
}

/// standard, from the other side of the org: dave (sales/team-c) gains
/// nothing in eng — department sharing is symmetric and bounded.
#[test]
fn standard_shares_within_the_department_only() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let assignments = [fx.assignment("org", STANDARD)];
    let dave = fx.placed("dave", "dave-user");
    assert_eq!(
        composition(&pdp, &fx, &dave, Some("dave-user"), &assignments),
        vec!["org", "sales", "team-c"],
        "dave's department sharing must stop at sales"
    );
    let carol = fx.placed("carol", "carol-user");
    assert_eq!(
        composition(&pdp, &fx, &carol, Some("carol-user"), &assignments),
        vec!["org", "eng", "team-a", "team-b", "carol-user"],
        "carol shares alice's department, both teams visible"
    );
}

/// The AC, at the seam: switching one team's pack changes what a
/// principal's composition may include — immediately, because assignment
/// is request-time data (ADR-0014 decision 3), and only for that team's
/// subtree.
#[test]
fn switching_a_teams_pack_changes_the_composition_set() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let alice = fx.placed("alice", "alice-user");

    // Nothing assigned: the tenant runs regulated-strict; alice composes
    // her own chain only.
    assert_eq!(
        composition(&pdp, &fx, &alice, Some("alice-user"), &[]),
        vec!["org", "eng", "team-a", "alice-user"]
    );

    // team-b switches to open-collaboration. The effective pack is the
    // resource's: team-b now admits org-wide readers, while every other
    // scope keeps deciding under regulated-strict.
    let switched = [fx.assignment("team-b", OPEN_COLLABORATION)];
    assert_eq!(
        composition(&pdp, &fx, &alice, Some("alice-user"), &switched),
        vec!["org", "eng", "team-a", "team-b", "alice-user"],
        "alice's next composition includes team-b and nothing else new"
    );

    // carol's personal scope under team-b keeps its privacy floor even
    // under the team's open pack.
    let carol_scope = read(
        &pdp,
        &fx,
        &alice,
        Some("alice-user"),
        "carol-user",
        &switched,
    );
    assert!(
        !carol_scope.allowed,
        "personal scopes stay excluded under open-collaboration"
    );

    // Switching back restores the strict composition.
    assert_eq!(
        composition(&pdp, &fx, &alice, Some("alice-user"), &[]),
        vec!["org", "eng", "team-a", "alice-user"]
    );
}

/// MEM-2 (ADR-0021 decision 3): the redaction config rides the effective
/// pack — embedded defaults per product pack, a stored pack's explicit
/// config, and the fail-safe strict fallback for an unconfigured stored
/// pack — through the same resolution every decision uses.
#[test]
fn redaction_config_rides_the_effective_pack() {
    use synveda_types::{RedactionConfig, RedactionMode};

    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let team = fx.node("team-a").id;
    let scopes = fx.chain("team-a");

    for (pack, expected) in [
        (REGULATED_STRICT, RedactionConfig::STRICT),
        (STANDARD, RedactionConfig::REDACT_ALL),
        (OPEN_COLLABORATION, RedactionConfig::REDACT_ALL),
    ] {
        let assignments = [fx.assignment("org", pack)];
        let effective = pdp.effective(
            fx.tenant,
            Resource::Scope(team),
            &AuthzContext {
                scopes: &scopes,
                assignments: &assignments,
                ..Default::default()
            },
        );
        assert_eq!(effective.name, pack);
        assert_eq!(effective.redaction, expected, "{pack}");
    }

    // Nothing assigned anywhere: the embedded default is strict.
    let unassigned = pdp.effective(
        fx.tenant,
        Resource::Scope(team),
        &AuthzContext {
            scopes: &scopes,
            ..Default::default()
        },
    );
    assert_eq!(unassigned.name, REGULATED_STRICT);
    assert_eq!(unassigned.redaction, RedactionConfig::STRICT);

    // A stored pack carries its explicit config; one stored without a
    // config falls back to strict (fail safe).
    const MEMBER_READ: &str = r#"permit (principal, action == Synveda::Action::"MemoryRead", resource)
           when { principal in resource };"#;
    let deny_secrets = RedactionConfig {
        secrets: RedactionMode::Deny,
        pii: RedactionMode::Redact,
    };
    pdp.install_source(
        fx.tenant,
        "acme-deny",
        1,
        MEMBER_READ,
        Some(deny_secrets),
        None,
    )
    .expect("install configured pack");
    pdp.install_source(fx.tenant, "acme-unconfigured", 1, MEMBER_READ, None, None)
        .expect("install unconfigured pack");
    for (pack, expected) in [
        ("acme-deny", deny_secrets),
        ("acme-unconfigured", RedactionConfig::STRICT),
    ] {
        let assignments = [fx.assignment("org", pack)];
        let effective = pdp.effective(
            fx.tenant,
            Resource::Scope(team),
            &AuthzContext {
                scopes: &scopes,
                assignments: &assignments,
                ..Default::default()
            },
        );
        assert_eq!(effective.name, pack);
        assert_eq!(effective.redaction, expected, "{pack}");
    }
}
