//! AUTHZ-3 AC: the full role×action matrix, golden-tested (ADR-0015
//! decision 8). Nine principals (the eight product roles plus role-free)
//! × the decision columns (the action vocabulary, `RoleAssign` split by
//! ordinary vs org-admin grant; `MemoryWrite` joined with MEM-1,
//! ADR-0020 — the contributor-writes marker discharged) × four targets
//! (in-subtree, out-of-subtree, above the binding, the tenant plane) ×
//! all three product packs — plus the cross-cutting invariants: subtree
//! boundaries, the privacy floor, the base-layer escalation guard under
//! custom packs, quarantine trumping roles, foreign rows ignored, and the
//! grant-less fail-closed path. Bindings flow through the same resolution
//! path production uses — never a PDP bypass (CLAUDE.md, seed §2.2).
//!
//! The fixture (as in packs.rs), with the matrix binding at `eng`:
//!
//! ```text
//! org
//! ├── eng (department)          ← the binding node
//! │   ├── team-a ── alice-user
//! │   └── team-b ── carol-user
//! └── sales (department)
//!     └── team-c ── dave-user
//! ```

use chrono::Utc;
use synveda_policy::{
    Action, AuthzContext, OPEN_COLLABORATION, Pdp, Principal, REGULATED_STRICT, Resource, STANDARD,
};
use synveda_types::{
    HierarchyNode, PolicyAssignment, Role, RoleBinding, ScopeId, ScopeKind, TenantId,
};

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

/// The decision columns: the action vocabulary, with `RoleAssign` split
/// by what is being granted — the base layer decides those differently
/// (ADR-0015 decision 5).
const COLUMNS: [(Action, Option<Role>); 13] = [
    (Action::HierarchyCreate, None),
    (Action::HierarchyRead, None),
    (Action::HierarchyUpdate, None),
    (Action::HierarchyDelete, None),
    (Action::MemoryRead, None),
    (Action::MemoryWrite, None),
    (Action::PolicyRead, None),
    (Action::PolicyAssign, None),
    (Action::RoleRead, None),
    (Action::RoleAssign, Some(Role::Viewer)),
    (Action::RoleAssign, Some(Role::OrgAdmin)),
    (Action::ServiceIdentityRead, None),
    (Action::ServiceIdentityManage, None),
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

    fn binding(&self, subject: &str, slug: Option<&str>, role: Role) -> RoleBinding {
        RoleBinding {
            tenant_id: self.tenant,
            subject: subject.to_owned(),
            scope_id: slug.map(|slug| self.node(slug).id),
            role,
            updated_at: Utc::now(),
        }
    }

    /// An unplaced principal — the shape that isolates role effects: the
    /// membership floor grants it nothing, so every allow it sees is
    /// role-driven.
    fn unplaced(&self, subject: &str) -> Principal {
        Principal {
            tenant_id: self.tenant,
            subject: subject.to_owned(),
            quarantined: false,
            scope_id: None,
            token_scope: None,
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

/// One decision through the facade, with the resource chain, placement
/// chain, assignments, and bindings supplied exactly as the gateway does.
#[allow(clippy::too_many_arguments)]
fn decide(
    pdp: &Pdp,
    fx: &Fixture,
    principal: &Principal,
    placement: Option<&str>,
    action: Action,
    target: Option<&str>,
    bindings: &[RoleBinding],
    assignments: &[PolicyAssignment],
    grant: Option<Role>,
) -> synveda_policy::AuthzDecision {
    let scopes = target.map(|slug| fx.chain(slug)).unwrap_or_default();
    let principal_scopes = placement.map(|slug| fx.chain(slug)).unwrap_or_default();
    let resource = match target {
        Some(slug) => Resource::Scope(fx.node(slug).id),
        None => Resource::Tenant(fx.tenant),
    };
    pdp.authorize(
        principal,
        action,
        resource,
        &AuthzContext {
            scopes: &scopes,
            principal_scopes: &principal_scopes,
            assignments,
            role_bindings: bindings,
            grant,
            ..Default::default()
        },
    )
    .expect("authorize")
}

/// The columns a role is expected to be allowed for a target *inside* the
/// bound subtree (ADR-0015 decision 4). Everything else in the matrix is
/// a deny.
fn allowed_in_subtree(role: Role) -> Vec<(Action, Option<Role>)> {
    match role {
        // Viewer: composition read only — read-only by name (ADR-0020
        // decision 3).
        Role::Viewer => vec![(Action::MemoryRead, None)],
        // Contributing content roles: read plus the shared-scope write
        // grant (MEM-1, ADR-0020 decision 3 — ADR-0015's
        // contributor-writes marker discharged).
        Role::Contributor | Role::Curator => {
            vec![(Action::MemoryRead, None), (Action::MemoryWrite, None)]
        }
        // Steward: the full admin plane, but never org-admin grants and
        // never content.
        Role::Steward => vec![
            (Action::HierarchyCreate, None),
            (Action::HierarchyRead, None),
            (Action::HierarchyUpdate, None),
            (Action::HierarchyDelete, None),
            (Action::PolicyRead, None),
            (Action::PolicyAssign, None),
            (Action::RoleRead, None),
            (Action::RoleAssign, Some(Role::Viewer)),
            (Action::ServiceIdentityRead, None),
            (Action::ServiceIdentityManage, None),
        ],
        // Org-admin: steward plus org-admin grants; still no content.
        Role::OrgAdmin => vec![
            (Action::HierarchyCreate, None),
            (Action::HierarchyRead, None),
            (Action::HierarchyUpdate, None),
            (Action::HierarchyDelete, None),
            (Action::PolicyRead, None),
            (Action::PolicyAssign, None),
            (Action::RoleRead, None),
            (Action::RoleAssign, Some(Role::Viewer)),
            (Action::RoleAssign, Some(Role::OrgAdmin)),
            (Action::ServiceIdentityRead, None),
            (Action::ServiceIdentityManage, None),
        ],
        // Auditor: the read-only admin surfaces; never content, never
        // mutations (seed §5).
        Role::Auditor => vec![
            (Action::HierarchyRead, None),
            (Action::PolicyRead, None),
            (Action::RoleRead, None),
            (Action::ServiceIdentityRead, None),
        ],
        // Marker roles until their approval actions land (SKIL-2, FLOW-3,
        // AUTHZ-5): no standing grants.
        Role::SecurityReviewer | Role::Compliance => Vec::new(),
    }
}

/// The full matrix under one pack: every role (and role-free), every
/// column, against an in-subtree target, an out-of-subtree target, the
/// node *above* the binding, and the tenant plane. A wrong allow and a
/// wrong deny both fail, and the failure names pack, role, action, and
/// target.
fn assert_matrix(pack: &str, version: i64) {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let assignments = [fx.assignment("org", pack)];

    let mut principals: Vec<(Principal, Option<Role>, Vec<RoleBinding>)> =
        vec![(fx.unplaced("nobody"), None, Vec::new())];
    for role in Role::ALL {
        let subject = format!("holder-{role}");
        let bindings = vec![fx.binding(&subject, Some("eng"), role)];
        principals.push((fx.unplaced(&subject), Some(role), bindings));
    }

    for (principal, role, bindings) in &principals {
        let expected = role.map(allowed_in_subtree).unwrap_or_default();
        for (action, grant) in COLUMNS {
            // In-subtree target: the binding node's subtree is where the
            // role is in force.
            let decision = decide(
                &pdp,
                &fx,
                principal,
                None,
                action,
                Some("team-b"),
                bindings,
                &assignments,
                grant,
            );
            let want = expected.contains(&(action, grant));
            assert_eq!(
                decision.allowed, want,
                "{pack}: role {role:?}, {action} (grant {grant:?}) on in-subtree team-b \
                 decided {}, matrix says {}",
                decision.allowed, want,
            );
            assert_eq!(decision.pack_name, pack, "{pack}: wrong pack decided");
            assert_eq!(decision.pack_version, version);

            // Out-of-subtree, above the binding, and the tenant plane:
            // a node binding reaches none of them (ADR-0015 decision 3).
            for target in [Some("team-c"), Some("org"), None] {
                if matches!(
                    action,
                    Action::MemoryRead | Action::MemoryWrite | Action::ServiceIdentityManage
                ) && target.is_none()
                {
                    // The schema scopes the memory plane and
                    // ServiceIdentityManage to Scope resources; a
                    // tenant-resource request is unrepresentable
                    // (ADR-0018 decision 3, ADR-0020 decision 3).
                    continue;
                }
                let decision = decide(
                    &pdp,
                    &fx,
                    principal,
                    None,
                    action,
                    target,
                    bindings,
                    &assignments,
                    grant,
                );
                assert!(
                    !decision.allowed,
                    "{pack}: role {role:?}, {action} (grant {grant:?}) on {} must be \
                     denied — the eng binding does not reach it",
                    target.unwrap_or("the tenant"),
                );
            }
        }
    }
}

/// regulated-strict: the golden matrix (the AC).
#[test]
fn matrix_regulated_strict() {
    assert_matrix(REGULATED_STRICT, 4);
}

/// standard: identical role matrix — packs differ on composition
/// membership, never on who administers (ADR-0015 decision 4).
#[test]
fn matrix_standard() {
    assert_matrix(STANDARD, 4);
}

/// open-collaboration: identical role matrix.
#[test]
fn matrix_open_collaboration() {
    assert_matrix(OPEN_COLLABORATION, 4);
}

/// A tenant-wide binding is in force everywhere, the tenant plane
/// included — the bootstrap property that makes a fresh tenant governable
/// (ADR-0015 decisions 2 and 6).
#[test]
fn tenant_wide_bindings_reach_every_scope_and_the_tenant_plane() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let admin = fx.unplaced("root-admin");
    let bindings = vec![fx.binding("root-admin", None, Role::OrgAdmin)];

    for target in [Some("org"), Some("team-c"), Some("carol-user"), None] {
        let decision = decide(
            &pdp,
            &fx,
            &admin,
            None,
            Action::PolicyAssign,
            target,
            &bindings,
            &[],
            None,
        );
        assert!(
            decision.allowed,
            "tenant-wide org-admin must reach {}",
            target.unwrap_or("the tenant plane"),
        );
    }
    // Including org-admin grants, node-scoped and tenant-wide.
    for target in [Some("team-b"), None] {
        let decision = decide(
            &pdp,
            &fx,
            &admin,
            None,
            Action::RoleAssign,
            target,
            &bindings,
            &[],
            Some(Role::OrgAdmin),
        );
        assert!(decision.allowed, "tenant-wide org-admin grants org-admin");
    }
}

/// A node-bound steward manages its subtree but never the tenant plane —
/// and a *tenant-wide* steward manages the tenant plane but still cannot
/// grant org-admin (the base guard, ADR-0015 decision 5).
#[test]
fn steward_boundaries_node_and_tenant_wide() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let steward = fx.unplaced("steward");

    // Node-bound at eng: subtree yes, tenant plane no (asserted in the
    // matrix too; pinned here as the headline).
    let at_eng = vec![fx.binding("steward", Some("eng"), Role::Steward)];
    assert!(
        decide(
            &pdp,
            &fx,
            &steward,
            None,
            Action::RoleAssign,
            Some("team-b"),
            &at_eng,
            &[],
            Some(Role::Steward),
        )
        .allowed,
        "a steward delegates stewardship downward"
    );

    // Tenant-wide steward: the tenant plane opens, org-admin grants stay
    // shut.
    let tenant_wide = vec![fx.binding("steward", None, Role::Steward)];
    assert!(
        decide(
            &pdp,
            &fx,
            &steward,
            None,
            Action::PolicyAssign,
            None,
            &tenant_wide,
            &[],
            None,
        )
        .allowed,
        "a tenant-wide steward may set the tenant default pack"
    );
    let escalation = decide(
        &pdp,
        &fx,
        &steward,
        None,
        Action::RoleAssign,
        None,
        &tenant_wide,
        &[],
        Some(Role::OrgAdmin),
    );
    assert!(
        !escalation.allowed,
        "granting org-admin without holding it must be forbidden"
    );
    assert!(
        !escalation.determining.is_empty(),
        "the escalation denial must name the base forbid"
    );
}

/// The escalation guard lives in the compiled-in base layer: a custom
/// pack that blanket-permits everything still cannot let a non-org-admin
/// grant (or revoke) org-admin (ADR-0015 decision 5).
#[test]
fn the_escalation_guard_survives_custom_packs() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    pdp.install_source(
        fx.tenant,
        "authz3-blanket",
        1,
        "permit (principal, action, resource) when { resource in principal.tenant };",
    )
    .expect("install blanket pack");
    let assignments = [fx.assignment("org", "authz3-blanket")];
    let nobody = fx.unplaced("nobody");

    // The blanket pack permits an ordinary grant to a role-free subject...
    assert!(
        decide(
            &pdp,
            &fx,
            &nobody,
            None,
            Action::RoleAssign,
            Some("team-b"),
            &[],
            &assignments,
            Some(Role::Viewer),
        )
        .allowed,
        "the blanket pack permits ordinary grants"
    );
    // ...but the base guard still forbids the org-admin grant.
    assert!(
        !decide(
            &pdp,
            &fx,
            &nobody,
            None,
            Action::RoleAssign,
            Some("team-b"),
            &[],
            &assignments,
            Some(Role::OrgAdmin),
        )
        .allowed,
        "the base guard must survive any custom pack"
    );
}

/// A `RoleAssign` decision without the grant in context fails closed as
/// an internal error — never an allow, never a skipped forbid (ADR-0015
/// decision 5).
#[test]
fn role_assign_without_a_grant_fails_closed() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let admin = fx.unplaced("root-admin");
    let bindings = vec![fx.binding("root-admin", None, Role::OrgAdmin)];
    let scopes = fx.chain("team-b");
    let result = pdp.authorize(
        &admin,
        Action::RoleAssign,
        Resource::Scope(fx.node("team-b").id),
        &AuthzContext {
            scopes: &scopes,
            role_bindings: &bindings,
            grant: None,
            ..Default::default()
        },
    );
    assert!(
        matches!(result, Err(synveda_types::Error::Internal { .. })),
        "a grant-less RoleAssign must fail closed, got {result:?}"
    );
}

/// Content roles read the bound subtree, personal scopes excluded — and
/// the grant composes with the membership floor: under regulated-strict
/// this is the seed's explicit cross-team grant (ADR-0015 decision 4).
#[test]
fn content_roles_read_the_bound_subtree_with_the_privacy_floor() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();

    // An unplaced viewer at eng: exactly the eng subtree's non-user
    // scopes, under every pack's identical rule.
    let viewer = fx.unplaced("viewer");
    let bindings = vec![fx.binding("viewer", Some("eng"), Role::Viewer)];
    let readable: Vec<&str> = ALL_SCOPES
        .into_iter()
        .filter(|target| {
            decide(
                &pdp,
                &fx,
                &viewer,
                None,
                Action::MemoryRead,
                Some(target),
                &bindings,
                &[],
                None,
            )
            .allowed
        })
        .collect();
    assert_eq!(
        readable,
        vec!["eng", "team-a", "team-b"],
        "viewer@eng reads the eng subtree, personal scopes excluded"
    );

    // Placed alice (team-a) granted viewer on sales: her own chain floor
    // plus the sales subtree — the explicit cross-team grant under
    // regulated-strict.
    let alice = fx.placed("alice", "alice-user");
    let granted = vec![fx.binding("alice", Some("sales"), Role::Viewer)];
    let readable: Vec<&str> = ALL_SCOPES
        .into_iter()
        .filter(|target| {
            decide(
                &pdp,
                &fx,
                &alice,
                Some("alice-user"),
                Action::MemoryRead,
                Some(target),
                &granted,
                &[],
                None,
            )
            .allowed
        })
        .collect();
    assert_eq!(
        readable,
        vec!["org", "eng", "sales", "team-a", "team-c", "alice-user"],
        "the sales grant adds exactly the sales subtree to alice's floor"
    );
}

/// Roles are strictly additive: binding auditor to a placed person never
/// strips their own membership floor (ADR-0015 decision 4).
#[test]
fn roles_never_subtract_the_membership_floor() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let alice = fx.placed("alice", "alice-user");
    let auditor = vec![fx.binding("alice", None, Role::Auditor)];
    let decision = decide(
        &pdp,
        &fx,
        &alice,
        Some("alice-user"),
        Action::MemoryRead,
        Some("alice-user"),
        &auditor,
        &[],
        None,
    );
    assert!(
        decision.allowed,
        "an auditor keeps reading their own memories"
    );
}

/// Quarantine trumps every role (the base forbid, ADR-0013/ADR-0014):
/// even a tenant-wide org-admin does nothing while quarantined.
#[test]
fn quarantine_trumps_roles() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let quarantined = Principal {
        quarantined: true,
        ..fx.unplaced("root-admin")
    };
    let bindings = vec![fx.binding("root-admin", None, Role::OrgAdmin)];
    for (action, grant) in COLUMNS {
        let decision = decide(
            &pdp,
            &fx,
            &quarantined,
            None,
            action,
            Some("team-b"),
            &bindings,
            &[],
            grant,
        );
        assert!(
            !decision.allowed,
            "{action} must be forbidden to a quarantined org-admin"
        );
    }
}

/// Binding rows from a foreign tenant contribute nothing, even if a
/// confused caller supplies them (defense in depth — the store's RLS
/// already makes this unrepresentable).
#[test]
fn foreign_tenant_binding_rows_are_ignored() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let subject = fx.unplaced("mallory");
    let forged = vec![RoleBinding {
        tenant_id: TenantId::new(),
        subject: "mallory".to_owned(),
        scope_id: None,
        role: Role::OrgAdmin,
        updated_at: Utc::now(),
    }];
    let decision = decide(
        &pdp,
        &fx,
        &subject,
        None,
        Action::HierarchyRead,
        Some("team-b"),
        &forged,
        &[],
        None,
    );
    assert!(
        !decision.allowed,
        "a foreign tenant's binding row must grant nothing"
    );
}
