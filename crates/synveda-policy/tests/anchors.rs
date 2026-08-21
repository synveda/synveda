//! CPR-6: authorisation over governed scope anchors (ADR-0073).
//!
//! Nine properties, decided by the real PDP against the real shipped packs.
//! They are here rather than in the gateway because every one of them is a
//! statement about the *decision*, not about a route: what a grant reaches,
//! what it does not, and what no pack can widen.
//!
//! The tree every test uses:
//!
//! ```text
//! tenant root  ────┬── org unit ──┬── workspace "payments" ── project "ledger"
//!                  │              └── workspace "risk"     ── project "models"
//!                  ├── principal scope: alice
//!                  └── principal scope: bob
//! ```
//!
//! No hierarchy node appears anywhere in this file, and no role binding: this
//! is the governed model deciding on its own.

use std::collections::{BTreeMap, BTreeSet};

use synveda_policy::{
    Action, AuthzContext, OPEN_COLLABORATION, PackOrigin, Pdp, Principal, REGULATED_STRICT,
    Resource, ResourceEntity, STANDARD, ScopeNode, effective_role_keys_at,
};
use synveda_types::access::{GrantSource, RoleKey};
use synveda_types::anchor::{AnchorSource, ScopeAnchor};
use synveda_types::scope::ScopeKind;
use synveda_types::{
    GrantId, GroupId, PolicyAssignment, ProjectId, ScopeId, Sensitivity, TenantId, WorkspaceId,
};

// ── The tree ─────────────────────────────────────────────────────────────────

struct Tree {
    tenant: TenantId,
    root: ScopeNode,
    unit: ScopeNode,
    payments: ScopeNode,
    risk: ScopeNode,
    ledger: ScopeNode,
    models: ScopeNode,
    alice_scope: ScopeNode,
    bob_scope: ScopeNode,
    /// The workspace and project subtypes that own four of those scopes.
    payments_ws: WorkspaceId,
    risk_ws: WorkspaceId,
    ledger_project: ProjectId,
}

fn node(tenant: TenantId, parent: Option<&ScopeNode>, kind: ScopeKind) -> ScopeNode {
    let id = ScopeId::new();
    ScopeNode {
        id,
        tenant_id: tenant,
        parent_id: parent.map(|node| node.id),
        kind,
        slug: format!("s-{}", id.as_uuid().simple()),
        sealed: false,
    }
}

fn tree() -> Tree {
    let tenant = TenantId::new();
    let root = node(tenant, None, ScopeKind::Tenant);
    let unit = node(tenant, Some(&root), ScopeKind::OrgUnit);
    let payments = node(tenant, Some(&unit), ScopeKind::Workspace);
    let risk = node(tenant, Some(&unit), ScopeKind::Workspace);
    let ledger = node(tenant, Some(&payments), ScopeKind::Project);
    let models = node(tenant, Some(&risk), ScopeKind::Project);
    let alice_scope = node(tenant, Some(&root), ScopeKind::Principal);
    let bob_scope = node(tenant, Some(&root), ScopeKind::Principal);
    Tree {
        tenant,
        root,
        unit,
        payments,
        risk,
        ledger,
        models,
        alice_scope,
        bob_scope,
        payments_ws: WorkspaceId::new(),
        risk_ws: WorkspaceId::new(),
        ledger_project: ProjectId::new(),
    }
}

impl Tree {
    /// Every node of the tree, so a chain can be walked from any of them.
    fn all(&self) -> Vec<ScopeNode> {
        vec![
            self.root.clone(),
            self.unit.clone(),
            self.payments.clone(),
            self.risk.clone(),
            self.ledger.clone(),
            self.models.clone(),
            self.alice_scope.clone(),
            self.bob_scope.clone(),
        ]
    }

    /// A scope's chain, nearest first — what a caller supplies as
    /// `AuthzContext::scopes`.
    fn chain(&self, scope: &ScopeNode) -> Vec<ScopeNode> {
        let all = self.all();
        let mut chain = vec![scope.clone()];
        let mut current = scope.clone();
        while let Some(parent_id) = current.parent_id {
            let Some(parent) = all.iter().find(|node| node.id == parent_id) else {
                break;
            };
            chain.push(parent.clone());
            current = parent.clone();
        }
        chain
    }
}

// ── Anchors and principals ───────────────────────────────────────────────────

/// An anchor this caller **holds**: a grant written at `scope`.
fn held(_tree: &Tree, scope: &ScopeNode, source: AnchorSource, roles: &[RoleKey]) -> ScopeAnchor {
    ScopeAnchor {
        scope_id: scope.id,
        kind: scope.kind,
        parent_scope_id: scope.parent_id,
        depth: 0,
        source,
        roles: roles.to_vec(),
        granted_at: vec![scope.id],
        via_groups: Vec::new(),
    }
}

/// An anchor whose roles arrived through a group.
fn via_group(tree: &Tree, scope: &ScopeNode, roles: &[RoleKey], group: GroupId) -> ScopeAnchor {
    ScopeAnchor {
        via_groups: vec![group],
        ..held(tree, scope, AnchorSource::Grant, roles)
    }
}

/// An anchor the caller holds **nothing** at — applicable, but conferring
/// nothing. The tenant root is one of these for almost everybody.
fn unheld(tree: &Tree, scope: &ScopeNode, source: AnchorSource) -> ScopeAnchor {
    ScopeAnchor {
        roles: Vec::new(),
        granted_at: Vec::new(),
        ..held(tree, scope, source, &[])
    }
}

/// An inherited anchor: the roles reach `scope` from `from`, so the grant is
/// written elsewhere and [`ScopeAnchor::is_direct`] is false.
fn inherited(tree: &Tree, scope: &ScopeNode, from: &ScopeNode, roles: &[RoleKey]) -> ScopeAnchor {
    ScopeAnchor {
        granted_at: vec![from.id],
        ..held(tree, scope, AnchorSource::Grant, roles)
    }
}

fn principal(tree: &Tree, subject: &str, own: Option<&ScopeNode>) -> Principal {
    Principal {
        tenant_id: tree.tenant,
        subject: subject.to_owned(),
        quarantined: false,
        scope_id: own.map(|scope| scope.id),
        token_scope: None,
    }
}

/// The decision context, with everything a test does not care about empty.
struct Ask<'a> {
    scopes: &'a [ScopeNode],
    principal_scopes: &'a [ScopeNode],
    anchors: &'a [ScopeAnchor],
    groups: &'a [GroupId],
    resources: &'a [ResourceEntity],
    assignments: &'a [PolicyAssignment],
    pack: &'a str,
}

impl<'a> Ask<'a> {
    fn context(&self) -> AuthzContext<'a> {
        AuthzContext {
            scopes: self.scopes,
            principal_scopes: self.principal_scopes,
            anchors: self.anchors,
            groups: self.groups,
            resources: self.resources,
            assignments: self.assignments,
            default_pack: Some(self.pack),
            lapses: &[],
            sensitivity: None,
        }
    }
}

fn ask<'a>(
    scopes: &'a [ScopeNode],
    principal_scopes: &'a [ScopeNode],
    anchors: &'a [ScopeAnchor],
) -> Ask<'a> {
    Ask {
        scopes,
        principal_scopes,
        anchors,
        groups: &[],
        resources: &[],
        assignments: &[],
        pack: REGULATED_STRICT,
    }
}

fn allows(
    pdp: &Pdp,
    principal: &Principal,
    action: Action,
    resource: Resource,
    ask: &Ask<'_>,
) -> bool {
    pdp.authorize(principal, action, resource, &ask.context())
        .expect("the decision must evaluate")
        .allowed
}

fn reads(
    pdp: &Pdp,
    principal: &Principal,
    resource: Resource,
    ask: &Ask<'_>,
    tier: Sensitivity,
) -> bool {
    let context = AuthzContext {
        sensitivity: Some(tier),
        ..ask.context()
    };
    pdp.authorize(principal, Action::MemoryRead, resource, &context)
        .expect("the decision must evaluate")
        .allowed
}

// ── 1. Personal principal-scope privacy ──────────────────────────────────────

/// Nobody reaches into somebody else's own scope — not the tenant root's
/// owner, not an administrator, under no pack.
///
/// The widest thing the model can express is a grant at the tenant root, so
/// that is what this asks with. It is a **base-layer** property, so it is
/// asserted against all three shipped packs rather than one.
#[test]
fn nobody_reaches_into_somebody_elses_own_scope() {
    let pdp = Pdp::new().expect("pdp");
    let tree = tree();
    let alice = principal(&tree, "alice", Some(&tree.alice_scope));
    let alice_chain = tree.chain(&tree.alice_scope);
    let bob_chain = tree.chain(&tree.bob_scope);
    // Alice owns the whole tenant, and Bob's scope is applicable to her request
    // only as the thing she is asking about.
    let anchors = vec![
        held(
            &tree,
            &tree.root,
            AnchorSource::TenantRoot,
            &[RoleKey::Owner],
        ),
        held(
            &tree,
            &tree.alice_scope,
            AnchorSource::PrincipalScope,
            &[RoleKey::Owner],
        ),
    ];

    for pack in [REGULATED_STRICT, STANDARD, OPEN_COLLABORATION] {
        let mut probe = ask(&bob_chain, &alice_chain, &anchors);
        probe.pack = pack;
        for tier in Sensitivity::ALL {
            assert!(
                !reads(
                    &pdp,
                    &alice,
                    Resource::Scope(tree.bob_scope.id),
                    &probe,
                    tier
                ),
                "{pack}: a tenant-root owner must not read {tier} in somebody else's own scope"
            );
        }
        for action in [
            Action::MemoryWrite,
            Action::MembershipRead,
            Action::MembershipGrant,
            Action::PolicyAssign,
            Action::ChannelPublish,
        ] {
            assert!(
                !allows(
                    &pdp,
                    &alice,
                    action,
                    Resource::Scope(tree.bob_scope.id),
                    &probe
                ),
                "{pack}: {action} must be refused in somebody else's own scope"
            );
        }
    }
}

/// The other half of the same rule: your own scope is yours.
///
/// A privacy floor that also locked the owner out would not be privacy, it
/// would be a broken table.
#[test]
fn your_own_scope_is_yours() {
    let pdp = Pdp::new().expect("pdp");
    let tree = tree();
    let bob = principal(&tree, "bob", Some(&tree.bob_scope));
    let bob_chain = tree.chain(&tree.bob_scope);
    let anchors = vec![held(
        &tree,
        &tree.bob_scope,
        AnchorSource::PrincipalScope,
        &[RoleKey::Owner],
    )];
    let probe = ask(&bob_chain, &bob_chain, &anchors);

    assert!(
        reads(
            &pdp,
            &bob,
            Resource::Scope(tree.bob_scope.id),
            &probe,
            Sensitivity::Internal
        ),
        "a person reads their own scope"
    );
    assert!(
        allows(
            &pdp,
            &bob,
            Action::MemoryWrite,
            Resource::Scope(tree.bob_scope.id),
            &probe
        ),
        "a person writes to their own scope"
    );
}

/// A grant written **directly at** somebody's own scope reaches it; an
/// inherited one never does.
///
/// This is the one door in the floor, and it is the difference between "I
/// shared my notes with you" and "an administrator can read everybody".
#[test]
fn only_a_grant_written_at_a_private_scope_reaches_it() {
    let pdp = Pdp::new().expect("pdp");
    let tree = tree();
    let alice = principal(&tree, "alice", Some(&tree.alice_scope));
    let alice_chain = tree.chain(&tree.alice_scope);
    let bob_chain = tree.chain(&tree.bob_scope);

    // Inherited from the root: refused, however senior the role.
    let inherited_only = vec![
        held(
            &tree,
            &tree.root,
            AnchorSource::TenantRoot,
            &[RoleKey::Owner],
        ),
        inherited(&tree, &tree.bob_scope, &tree.root, &[RoleKey::Owner]),
    ];
    let probe = ask(&bob_chain, &alice_chain, &inherited_only);
    assert!(
        !reads(
            &pdp,
            &alice,
            Resource::Scope(tree.bob_scope.id),
            &probe,
            Sensitivity::Internal
        ),
        "an inherited grant does not reach a private scope"
    );

    // Written at Bob's own scope by Bob: it does.
    let direct = vec![held(
        &tree,
        &tree.bob_scope,
        AnchorSource::Grant,
        &[RoleKey::Viewer],
    )];
    let probe = ask(&bob_chain, &alice_chain, &direct);
    assert!(
        reads(
            &pdp,
            &alice,
            Resource::Scope(tree.bob_scope.id),
            &probe,
            Sensitivity::Internal
        ),
        "a grant written at the private scope reaches it"
    );
}

// ── 2 and 3. Project sharing, and workspace inheritance ──────────────────────

/// A workspace grant is in force at that workspace's projects, with **no row
/// written there**.
#[test]
fn a_workspace_grant_reaches_its_projects() {
    let pdp = Pdp::new().expect("pdp");
    let tree = tree();
    let alice = principal(&tree, "alice", Some(&tree.alice_scope));
    let alice_chain = tree.chain(&tree.alice_scope);
    let ledger_chain = tree.chain(&tree.ledger);
    // One anchor at the workspace, and the project anchor the resolver derives
    // from it — granted_at names the workspace, not the project.
    let anchors = vec![
        held(
            &tree,
            &tree.payments,
            AnchorSource::SelectedWorkspace,
            &[RoleKey::Owner],
        ),
        inherited(&tree, &tree.ledger, &tree.payments, &[RoleKey::Owner]),
    ];
    let resources = [ResourceEntity::Project {
        id: tree.ledger_project,
        scope_id: tree.ledger.id,
        workspace_id: tree.payments_ws,
    }];
    let mut probe = ask(&ledger_chain, &alice_chain, &anchors);
    probe.resources = &resources;

    assert!(
        allows(
            &pdp,
            &alice,
            Action::ProjectRead,
            Resource::Project(tree.ledger_project),
            &probe
        ),
        "a workspace owner reads the workspace's project"
    );
    assert!(
        allows(
            &pdp,
            &alice,
            Action::ProjectUpdate,
            Resource::Project(tree.ledger_project),
            &probe
        ),
        "a workspace owner administers the workspace's project"
    );
    assert!(
        !anchors[1].is_direct(),
        "the project anchor is inherited: no grant row was written there"
    );
}

/// The same grant does **not** reach the workspace beside it.
#[test]
fn a_workspace_grant_does_not_reach_a_sibling_workspace() {
    let pdp = Pdp::new().expect("pdp");
    let tree = tree();
    let alice = principal(&tree, "alice", Some(&tree.alice_scope));
    let alice_chain = tree.chain(&tree.alice_scope);
    let risk_chain = tree.chain(&tree.risk);
    let anchors = vec![held(
        &tree,
        &tree.payments,
        AnchorSource::SelectedWorkspace,
        &[RoleKey::Owner],
    )];
    let resources = [ResourceEntity::Workspace {
        id: tree.risk_ws,
        scope_id: tree.risk.id,
    }];
    let mut probe = ask(&risk_chain, &alice_chain, &anchors);
    probe.resources = &resources;

    assert!(
        !allows(
            &pdp,
            &alice,
            Action::WorkspaceUpdate,
            Resource::Workspace(tree.risk_ws),
            &probe
        ),
        "an owner of one workspace does not administer its sibling"
    );
    assert!(
        !allows(
            &pdp,
            &alice,
            Action::MembershipRead,
            Resource::Workspace(tree.risk_ws),
            &probe
        ),
        "nor read its membership"
    );
}

// ── 4. Project-only access ───────────────────────────────────────────────────

/// Somebody granted one project reaches that project and **not** the
/// workspace above it.
///
/// This is the direction the old model could not express at all: a placement
/// chain runs upward, so being "in" a project made you a member of everything
/// containing it. A grant runs downward from where it was written.
#[test]
fn a_project_grant_does_not_reach_its_workspace() {
    let pdp = Pdp::new().expect("pdp");
    let tree = tree();
    let bob = principal(&tree, "bob", Some(&tree.bob_scope));
    let bob_chain = tree.chain(&tree.bob_scope);
    let anchors = vec![held(
        &tree,
        &tree.ledger,
        AnchorSource::SelectedProject,
        &[RoleKey::Member],
    )];

    // At the project: they may work.
    let ledger_chain = tree.chain(&tree.ledger);
    let project_entity = [ResourceEntity::Project {
        id: tree.ledger_project,
        scope_id: tree.ledger.id,
        workspace_id: tree.payments_ws,
    }];
    let mut at_project = ask(&ledger_chain, &bob_chain, &anchors);
    at_project.resources = &project_entity;
    assert!(
        allows(
            &pdp,
            &bob,
            Action::ProjectRead,
            Resource::Project(tree.ledger_project),
            &at_project
        ),
        "a project member reads the project"
    );
    assert!(
        reads(
            &pdp,
            &bob,
            Resource::Scope(tree.ledger.id),
            &at_project,
            Sensitivity::Internal
        ),
        "and composes from its scope"
    );

    // At the workspace above it: nothing.
    let payments_chain = tree.chain(&tree.payments);
    let workspace_entity = [ResourceEntity::Workspace {
        id: tree.payments_ws,
        scope_id: tree.payments.id,
    }];
    let mut at_workspace = ask(&payments_chain, &bob_chain, &anchors);
    at_workspace.resources = &workspace_entity;
    for action in [
        Action::WorkspaceRead,
        Action::WorkspaceUpdate,
        Action::MembershipRead,
        Action::MembershipGrant,
    ] {
        assert!(
            !allows(
                &pdp,
                &bob,
                action,
                Resource::Workspace(tree.payments_ws),
                &at_workspace
            ),
            "{action} must not follow a project grant upward"
        );
    }
    assert!(
        !reads(
            &pdp,
            &bob,
            Resource::Scope(tree.payments.id),
            &at_workspace,
            Sensitivity::Internal
        ),
        "nor does composition"
    );
}

// ── 5. Group-derived access ──────────────────────────────────────────────────

/// A grant naming a group reaches every member of that group, and the
/// principal is `in` the group as an entity rather than by a string claim.
#[test]
fn a_group_grant_reaches_its_members() {
    let pdp = Pdp::new().expect("pdp");
    let tree = tree();
    let carol = principal(&tree, "carol", None);
    let group = GroupId::new();
    let anchors = vec![via_group(&tree, &tree.payments, &[RoleKey::Owner], group)];
    let groups = [group];
    let payments_chain = tree.chain(&tree.payments);
    let entity = [ResourceEntity::Workspace {
        id: tree.payments_ws,
        scope_id: tree.payments.id,
    }];
    let mut probe = ask(&payments_chain, &[], &anchors);
    probe.groups = &groups;
    probe.resources = &entity;

    assert!(
        allows(
            &pdp,
            &carol,
            Action::WorkspaceUpdate,
            Resource::Workspace(tree.payments_ws),
            &probe
        ),
        "a group's grant administers the workspace for its members"
    );
    assert_eq!(
        effective_role_keys_at(Resource::Workspace(tree.payments_ws), &probe.context()),
        vec![RoleKey::Owner],
        "and it arrives as a role key like any other"
    );
    assert_eq!(
        anchors[0].via_groups,
        vec![group],
        "the anchor records which group reached them"
    );
}

/// A group with no grant naming it confers nothing — the property ADR-0072
/// decision 2 refused a permission table to keep.
#[test]
fn membership_of_a_group_alone_confers_nothing() {
    let pdp = Pdp::new().expect("pdp");
    let tree = tree();
    let carol = principal(&tree, "carol", None);
    let group = GroupId::new();
    let groups = [group];
    let payments_chain = tree.chain(&tree.payments);
    let entity = [ResourceEntity::Workspace {
        id: tree.payments_ws,
        scope_id: tree.payments.id,
    }];
    let mut probe = ask(&payments_chain, &[], &[]);
    probe.groups = &groups;
    probe.resources = &entity;

    assert!(
        !allows(
            &pdp,
            &carol,
            Action::WorkspaceRead,
            Resource::Workspace(tree.payments_ws),
            &probe
        ),
        "being in a group is not being granted anything"
    );
}

// ── 6. Grant revocation ──────────────────────────────────────────────────────

/// The decision follows the rows. A revoked grant is an anchor that no longer
/// carries the role, and the very next decision refuses.
///
/// Revocation is not a separate mechanism and there is nothing to invalidate:
/// the anchors are resolved per request, so this is the same property
/// ADR-0037 decision 4 gave lapses — the read *is* the expiry.
#[test]
fn revoking_a_grant_refuses_the_next_decision() {
    let pdp = Pdp::new().expect("pdp");
    let tree = tree();
    let alice = principal(&tree, "alice", Some(&tree.alice_scope));
    let alice_chain = tree.chain(&tree.alice_scope);
    let payments_chain = tree.chain(&tree.payments);
    let entity = [ResourceEntity::Workspace {
        id: tree.payments_ws,
        scope_id: tree.payments.id,
    }];

    let granted = vec![held(
        &tree,
        &tree.payments,
        AnchorSource::SelectedWorkspace,
        &[RoleKey::Owner],
    )];
    let mut before = ask(&payments_chain, &alice_chain, &granted);
    before.resources = &entity;
    assert!(
        allows(
            &pdp,
            &alice,
            Action::WorkspaceUpdate,
            Resource::Workspace(tree.payments_ws),
            &before
        ),
        "with the grant, the update is permitted"
    );

    // The revocation: the scope is still applicable — it is still the selected
    // workspace — and it now carries no role.
    let revoked = vec![unheld(
        &tree,
        &tree.payments,
        AnchorSource::SelectedWorkspace,
    )];
    let mut after = ask(&payments_chain, &alice_chain, &revoked);
    after.resources = &entity;
    assert!(
        !allows(
            &pdp,
            &alice,
            Action::WorkspaceUpdate,
            Resource::Workspace(tree.payments_ws),
            &after
        ),
        "without it, the very next decision refuses"
    );
    assert!(
        effective_role_keys_at(Resource::Workspace(tree.payments_ws), &after.context()).is_empty(),
        "and no role key reaches the resource"
    );
}

/// Revoking a grant on a **grant resource** is itself decidable, which is what
/// makes "who may take this away" a question a pack can answer.
#[test]
fn a_grant_is_a_resource_a_decision_can_name() {
    let pdp = Pdp::new().expect("pdp");
    let tree = tree();
    let alice = principal(&tree, "alice", Some(&tree.alice_scope));
    let alice_chain = tree.chain(&tree.alice_scope);
    let payments_chain = tree.chain(&tree.payments);
    let grant_id = GrantId::new();
    let entity = [ResourceEntity::Grant {
        id: grant_id,
        scope_id: tree.payments.id,
        role: RoleKey::Member,
        source: GrantSource::Direct,
    }];

    let anchors = vec![held(
        &tree,
        &tree.payments,
        AnchorSource::SelectedWorkspace,
        &[RoleKey::Administrator],
    )];
    let mut probe = ask(&payments_chain, &alice_chain, &anchors);
    probe.resources = &entity;
    assert!(
        allows(
            &pdp,
            &alice,
            Action::MembershipGrant,
            Resource::Grant(grant_id),
            &probe
        ),
        "an administrator of the scope the grant is written at may revoke it"
    );

    let outsider = principal(&tree, "mallory", None);
    let mut nothing = ask(&payments_chain, &[], &[]);
    nothing.resources = &entity;
    assert!(
        !allows(
            &pdp,
            &outsider,
            Action::MembershipGrant,
            Resource::Grant(grant_id),
            &nothing
        ),
        "somebody holding nothing may not"
    );
}

// ── 7. Organisation-unit policy inheritance ──────────────────────────────────

/// A profile assigned at an organisation unit governs everything beneath it,
/// however deep, and a grant written there reaches the same subtree.
///
/// Both halves matter: the first is where policy comes from, the second is
/// where authority comes from, and the old model conflated them into one
/// ladder.
#[test]
fn an_org_unit_governs_and_grants_into_its_subtree() {
    let pdp = Pdp::new().expect("pdp");
    let tree = tree();
    let alice = principal(&tree, "alice", Some(&tree.alice_scope));
    let alice_chain = tree.chain(&tree.alice_scope);
    let ledger_chain = tree.chain(&tree.ledger);
    let assignments = [PolicyAssignment {
        tenant_id: tree.tenant,
        scope_id: tree.unit.id,
        pack_name: OPEN_COLLABORATION.to_owned(),
        updated_at: chrono::Utc::now(),
    }];
    let anchors = vec![
        held(
            &tree,
            &tree.unit,
            AnchorSource::OrgUnit,
            &[RoleKey::Curator],
        ),
        inherited(&tree, &tree.ledger, &tree.unit, &[RoleKey::Curator]),
    ];
    let mut probe = ask(&ledger_chain, &alice_chain, &anchors);
    probe.assignments = &assignments;

    let effective = pdp.effective(
        tree.tenant,
        Resource::Scope(tree.ledger.id),
        &probe.context(),
    );
    assert_eq!(effective.name, OPEN_COLLABORATION);
    assert_eq!(
        effective.origin,
        PackOrigin::Assigned(tree.unit.id),
        "the profile in force two levels down is the org unit's"
    );

    assert_eq!(
        effective_role_keys_at(Resource::Scope(tree.ledger.id), &probe.context()),
        vec![RoleKey::Curator],
        "and the grant written at the org unit reaches the project"
    );
    assert!(
        allows(
            &pdp,
            &alice,
            Action::ChannelPublish,
            Resource::Scope(tree.ledger.id),
            &probe
        ),
        "which is what lets a curator bound at an org unit publish inside it"
    );
}

/// **No rank.** Nesting the same tree deeper changes no verdict.
///
/// The old model gave every scope a rank and made a chain a strictly
/// increasing ladder; a pack could — and `standard` did — read "the nearest
/// department". There is nothing here to read: an org unit nests inside itself
/// arbitrarily and the decision is the same at every depth.
#[test]
fn depth_is_not_authority() {
    let pdp = Pdp::new().expect("pdp");
    let tree = tree();
    let alice = principal(&tree, "alice", Some(&tree.alice_scope));
    let alice_chain = tree.chain(&tree.alice_scope);

    // A second, third and fourth org unit between the first and the workspace.
    let deep_a = node(tree.tenant, Some(&tree.unit), ScopeKind::OrgUnit);
    let deep_b = node(tree.tenant, Some(&deep_a), ScopeKind::OrgUnit);
    let deep_c = node(tree.tenant, Some(&deep_b), ScopeKind::OrgUnit);
    let deep_ws = node(tree.tenant, Some(&deep_c), ScopeKind::Workspace);
    let deep_chain = vec![
        deep_ws.clone(),
        deep_c,
        deep_b,
        deep_a,
        tree.unit.clone(),
        tree.root.clone(),
    ];

    let shallow_anchors = vec![held(
        &tree,
        &tree.payments,
        AnchorSource::SelectedWorkspace,
        &[RoleKey::Member],
    )];
    let deep_anchors = vec![ScopeAnchor {
        scope_id: deep_ws.id,
        kind: deep_ws.kind,
        parent_scope_id: deep_ws.parent_id,
        depth: 0,
        source: AnchorSource::SelectedWorkspace,
        roles: vec![RoleKey::Member],
        granted_at: vec![deep_ws.id],
        via_groups: Vec::new(),
    }];

    let shallow_chain = tree.chain(&tree.payments);
    let shallow = ask(&shallow_chain, &alice_chain, &shallow_anchors);
    let deep = ask(&deep_chain, &alice_chain, &deep_anchors);

    for action in Action::PROBED_AT_SCOPE {
        assert_eq!(
            allows(
                &pdp,
                &alice,
                action,
                Resource::Scope(tree.payments.id),
                &shallow
            ),
            allows(&pdp, &alice, action, Resource::Scope(deep_ws.id), &deep),
            "{action} must not depend on how deep the scope is"
        );
    }
}

// ── 8. No cross-tenant entity injection ──────────────────────────────────────

/// A caller-supplied chain, anchor or entity belonging to another tenant
/// grants nothing.
///
/// Every one of those is caller-supplied by design — the PDP holds no storage
/// (seed §2.4) — so the guard cannot be "the caller would not do that". It is
/// that a foreign scope chains up to a **different** `Tenant` entity, so
/// `resource in principal.tenant` is false and every permit in every pack is
/// built on it.
#[test]
fn a_foreign_tenants_scope_grants_nothing() {
    let pdp = Pdp::new().expect("pdp");
    let ours = tree();
    let theirs = tree();
    let alice = principal(&ours, "alice", Some(&ours.alice_scope));
    let alice_chain = ours.chain(&ours.alice_scope);

    // The whole injection: their chain, an anchor claiming `owner` at their
    // workspace, their workspace entity, and a group we say we are in.
    let their_chain = theirs.chain(&theirs.payments);
    let injected = vec![ScopeAnchor {
        scope_id: theirs.payments.id,
        kind: ScopeKind::Workspace,
        parent_scope_id: theirs.payments.parent_id,
        depth: 0,
        source: AnchorSource::SelectedWorkspace,
        roles: vec![RoleKey::Owner, RoleKey::Administrator],
        granted_at: vec![theirs.payments.id],
        via_groups: Vec::new(),
    }];
    let entity = [ResourceEntity::Workspace {
        id: theirs.payments_ws,
        scope_id: theirs.payments.id,
    }];
    let mut probe = ask(&their_chain, &alice_chain, &injected);
    probe.resources = &entity;

    for action in Action::PROBED_AT_SCOPE {
        assert!(
            !allows(
                &pdp,
                &alice,
                action,
                Resource::Scope(theirs.payments.id),
                &probe
            ),
            "{action} must be refused on another tenant's scope"
        );
    }
    for action in [Action::WorkspaceRead, Action::WorkspaceUpdate] {
        assert!(
            !allows(
                &pdp,
                &alice,
                action,
                Resource::Workspace(theirs.payments_ws),
                &probe
            ),
            "{action} must be refused on another tenant's workspace"
        );
    }
    for tier in Sensitivity::ALL {
        assert!(
            !reads(
                &pdp,
                &alice,
                Resource::Scope(theirs.payments.id),
                &probe,
                tier
            ),
            "no tier composes from another tenant's scope"
        );
    }
}

/// A chain that *mixes* tenants — our root spliced above their workspace — is
/// refused too. The tenant is carried per node rather than per chain exactly
/// so that this cannot be laundered.
#[test]
fn a_spliced_chain_cannot_launder_a_foreign_scope() {
    let pdp = Pdp::new().expect("pdp");
    let ours = tree();
    let theirs = tree();
    let alice = principal(&ours, "alice", Some(&ours.alice_scope));
    let alice_chain = ours.chain(&ours.alice_scope);

    let spliced = vec![
        theirs.payments.clone(),
        ours.unit.clone(),
        ours.root.clone(),
    ];
    let anchors = vec![
        held(
            &ours,
            &ours.root,
            AnchorSource::TenantRoot,
            &[RoleKey::Owner],
        ),
        ScopeAnchor {
            scope_id: theirs.payments.id,
            kind: ScopeKind::Workspace,
            parent_scope_id: Some(ours.unit.id),
            depth: 0,
            source: AnchorSource::Grant,
            roles: vec![RoleKey::Owner],
            granted_at: vec![ours.root.id],
            via_groups: Vec::new(),
        },
    ];
    let probe = ask(&spliced, &alice_chain, &anchors);

    assert!(
        !reads(
            &pdp,
            &alice,
            Resource::Scope(theirs.payments.id),
            &probe,
            Sensitivity::Public
        ),
        "a foreign node keeps its own tenant however it is spliced"
    );
    assert!(
        !allows(
            &pdp,
            &alice,
            Action::MemoryWrite,
            Resource::Scope(theirs.payments.id),
            &probe
        ),
        "and nothing may be written to it"
    );
}

// ── 9. Effective capabilities match real authorised actions ──────────────────

/// The capability answer for a scope is exactly the set of actions the PDP
/// would allow there — because it is the same call.
///
/// The property worth testing is not that two identical calls agree, it is
/// that the answer **moves with the grant** rather than with a plan: two
/// principals differing only in the role key they hold get different sets, and
/// the difference is the one the packs describe.
#[test]
fn capabilities_are_the_decisions_they_forecast() {
    let pdp = Pdp::new().expect("pdp");
    let tree = tree();
    let payments_chain = tree.chain(&tree.payments);
    let entity = [ResourceEntity::Workspace {
        id: tree.payments_ws,
        scope_id: tree.payments.id,
    }];

    let capabilities = |roles: &[RoleKey]| -> BTreeMap<&'static str, bool> {
        let who = principal(&tree, "someone", Some(&tree.alice_scope));
        let own = tree.chain(&tree.alice_scope);
        let anchors = vec![held(
            &tree,
            &tree.payments,
            AnchorSource::SelectedWorkspace,
            roles,
        )];
        let mut probe = ask(&payments_chain, &own, &anchors);
        probe.resources = &entity;
        Action::PROBED_AT_SCOPE
            .iter()
            .map(|action| {
                (
                    action.as_str(),
                    allows(
                        &pdp,
                        &who,
                        *action,
                        Resource::Scope(tree.payments.id),
                        &probe,
                    ),
                )
            })
            .collect()
    };

    let owner = capabilities(&[RoleKey::Owner]);
    let viewer = capabilities(&[RoleKey::Viewer]);
    let nobody = capabilities(&[]);

    // Every capability a viewer has, an owner has.
    for (action, allowed) in &viewer {
        if *allowed {
            assert!(
                owner[action],
                "an owner must hold everything a viewer does: {action}"
            );
        }
    }
    // And the owner holds strictly more.
    let owner_yes: BTreeSet<&str> = owner
        .iter()
        .filter(|(_, allowed)| **allowed)
        .map(|(action, _)| *action)
        .collect();
    let viewer_yes: BTreeSet<&str> = viewer
        .iter()
        .filter(|(_, allowed)| **allowed)
        .map(|(action, _)| *action)
        .collect();
    assert!(
        owner_yes.len() > viewer_yes.len(),
        "an owner and a viewer must not forecast the same thing"
    );
    assert!(
        owner_yes.contains(Action::MembershipGrant.as_str()),
        "an owner may hand out access"
    );
    assert!(
        !viewer_yes.contains(Action::MembershipGrant.as_str()),
        "a viewer may not"
    );

    // Somebody holding nothing forecasts nothing at a scope they hold nothing
    // at — no floor, no edition, no default.
    assert!(
        nobody.values().all(|allowed| !*allowed),
        "holding nothing must forecast nothing: {:?}",
        nobody
            .iter()
            .filter(|(_, allowed)| **allowed)
            .map(|(action, _)| *action)
            .collect::<Vec<_>>()
    );
}

/// A capability forecast is never a shape read off a deployment size.
///
/// The same principal, the same grant, the same scope: assigning a *different
/// profile* changes the answer, and nothing else does. That is what "policy
/// profiles, not editions" means where it is checkable.
#[test]
fn only_the_profile_and_the_grant_move_a_capability() {
    let pdp = Pdp::new().expect("pdp");
    let tree = tree();
    let bob = principal(&tree, "bob", Some(&tree.bob_scope));
    let bob_chain = tree.chain(&tree.bob_scope);
    let payments_chain = tree.chain(&tree.payments);
    let anchors = vec![held(
        &tree,
        &tree.payments,
        AnchorSource::SelectedWorkspace,
        &[RoleKey::Member],
    )];

    let under = |pack: &'static str| {
        let mut probe = ask(&payments_chain, &bob_chain, &anchors);
        probe.pack = pack;
        allows(
            &pdp,
            &bob,
            Action::MembershipRead,
            Resource::Scope(tree.payments.id),
            &probe,
        )
    };

    assert!(
        !under(REGULATED_STRICT),
        "a regulated deployment keeps membership to the admin roles"
    );
    assert!(
        under(OPEN_COLLABORATION),
        "a collaborative one lets its members see who else is in it"
    );
}

// ── The vocabulary that is gone ──────────────────────────────────────────────

/// The five shapes are what a scope entity carries, and nothing orders them.
///
/// A guard rather than a behaviour test: if somebody reintroduces a rank by
/// adding a kind that implies one, this is where the vocabulary is written
/// down beside the claim that it has no order.
#[test]
fn the_scope_vocabulary_is_five_shapes_with_no_order() {
    assert_eq!(
        ScopeKind::ALL
            .iter()
            .map(|kind| kind.as_str())
            .collect::<Vec<_>>(),
        vec!["tenant", "org_unit", "workspace", "project", "principal"],
    );
    // An org unit nests inside itself — the property a rank ladder forbids.
    assert!(ScopeKind::OrgUnit.permits_parent(ScopeKind::OrgUnit));
}
