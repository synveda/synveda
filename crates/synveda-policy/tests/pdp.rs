//! Facade mechanics (AUTHZ-1 ADR-0012; AUTHZ-2 ADR-0014): the embedded
//! default, per-node effective-pack resolution (nearest assignment →
//! tenant default → `regulated-strict`), stored pack install/remove,
//! reserved names, compile rejection with last-good semantics, and the
//! decision metadata (pack name + version + determining policies) every
//! call carries. Restrictive behaviour is exercised through *test policy
//! packs* installed via the same path the reloader uses — never a PDP
//! bypass (CLAUDE.md, seed §2.2).

use chrono::Utc;
use synveda_policy::{
    Action, AuthzContext, Pdp, Principal, REGULATED_STRICT, Resource, ScopeNode, is_reserved,
};
use synveda_types::access::RoleKey;
use synveda_types::anchor::{AnchorSource, ScopeAnchor};
use synveda_types::scope::ScopeKind;
use synveda_types::{Error, PackConfig, PolicyAssignment, ScopeId, Sensitivity, TenantId};

const ADMIN_ACTIONS: [Action; 6] = [
    Action::ScopeCreate,
    Action::ScopeRead,
    Action::ScopeUpdate,
    Action::ScopeUpdate,
    Action::PolicyRead,
    Action::PolicyAssign,
];

/// A pack that only permits scope reads; everything else falls to
/// Cedar's default-deny.
const READ_ONLY_PACK: &str = r#"
permit (
    principal,
    action == Synveda::Action::"ScopeRead",
    resource
) when { resource in principal.tenant };
"#;

fn node(
    tenant_id: TenantId,
    id: ScopeId,
    parent_id: Option<ScopeId>,
    kind: ScopeKind,
    slug: &str,
    _depth: i32,
    _path: &str,
) -> ScopeNode {
    ScopeNode {
        id,
        tenant_id,
        parent_id,
        kind,
        slug: slug.to_owned(),
        sealed: false,
    }
}

/// An org → department → team chain for `tenant`, deepest scope last.
/// The chain the PDP takes: the old hierarchy's rows projected onto the
/// shape vocabulary at the caller's edge (CPR-6, ADR-0073 decision 1).
fn chain(tenant: TenantId) -> Vec<ScopeNode> {
    let org = ScopeId::new();
    let dept = ScopeId::new();
    let team = ScopeId::new();
    vec![
        node(tenant, org, None, ScopeKind::Tenant, "acme", 0, "acme"),
        node(
            tenant,
            dept,
            Some(org),
            ScopeKind::OrgUnit,
            "payments",
            1,
            "acme/payments",
        ),
        node(
            tenant,
            team,
            Some(dept),
            ScopeKind::OrgUnit,
            "core",
            2,
            "acme/payments/core",
        ),
    ]
}

fn team_of(chain: &[ScopeNode]) -> ScopeId {
    chain.last().expect("chain is non-empty").id
}

fn org_of(chain: &[ScopeNode]) -> ScopeId {
    chain.first().expect("chain is non-empty").id
}

fn assignment(tenant_id: TenantId, scope_id: ScopeId, pack_name: &str) -> PolicyAssignment {
    PolicyAssignment {
        tenant_id,
        scope_id,
        pack_name: pack_name.to_owned(),
        updated_at: Utc::now(),
    }
}

fn principal(tenant_id: TenantId) -> Principal {
    Principal {
        tenant_id,
        subject: "alice".to_owned(),
        quarantined: false,
        scope_id: None,
        token_scope: None,
    }
}

/// A tenant-wide org-admin binding for [`principal`] — since AUTHZ-3 the
/// product packs' admin planes require a role (ADR-0015 decision 4), so
/// facade-mechanics tests that expect admin allows bind their principal
/// the same way production would.
fn admin_anchor(kind: synveda_types::scope::ScopeKind, scope_id: ScopeId) -> ScopeAnchor {
    ScopeAnchor {
        scope_id,
        kind,
        parent_scope_id: None,
        depth: 0,
        source: AnchorSource::Grant,
        roles: vec![RoleKey::Administrator],
        granted_at: vec![scope_id],
        via_groups: Vec::new(),
    }
}

/// With nothing stored and nothing assigned, the embedded default decides:
/// `regulated-strict@2`, strict by default (seed §2.1, ADR-0014
/// decision 1) — and since AUTHZ-3, its admin plane admits *bound*
/// admins: a tenant-wide org-admin administers its own tenant
/// (ADR-0015 decision 4; ADR-0012 decision 3's semantics, role-gated).
#[test]
fn the_default_pack_is_regulated_strict_and_admits_bound_admins() {
    let pdp = Pdp::new().expect("build pdp");
    let tenant = TenantId::new();
    let scopes = chain(tenant);
    let team = team_of(&scopes);
    let alice = principal(tenant);
    let anchors = [admin_anchor(scopes[0].kind, scopes[0].id)];
    let context = AuthzContext {
        sensitivity: Some(Sensitivity::Internal),
        scopes: &scopes,
        anchors: &anchors,
        ..Default::default()
    };

    for action in ADMIN_ACTIONS {
        let decision = pdp
            .authorize(&alice, action, Resource::Scope(team), &context)
            .expect("authorize");
        assert!(decision.allowed, "{action} must be allowed on own scope");
        assert_eq!(decision.pack_name, REGULATED_STRICT);
        assert_eq!(decision.pack_version, 19);
        assert!(
            !decision.determining.is_empty(),
            "an allow must name its permitting policies"
        );
    }

    // Tenant-level resources (creating the org root, reading the root)
    // need no chain at all: the tenant-root grant carries them.
    for action in [Action::ScopeCreate, Action::PolicyAssign] {
        let decision = pdp
            .authorize(&alice, action, Resource::Tenant(tenant), &context)
            .expect("authorize");
        assert!(decision.allowed, "{action} must be allowed on own tenant");
    }
}

/// The base layer travels with every pack (ADR-0014 decision 2): a stored
/// pack that never mentions quarantine still forbids a quarantined
/// principal everything, because the forbid is compiled in ahead of it.
#[test]
fn the_base_quarantine_forbid_is_compiled_into_stored_packs() {
    let pdp = Pdp::new().expect("build pdp");
    let tenant = TenantId::new();
    let scopes = chain(tenant);
    let team = team_of(&scopes);
    pdp.install_source(
        tenant,
        "authz2-blanket",
        1,
        "permit (principal, action, resource) when { resource in principal.tenant };",
        PackConfig::default(),
    )
    .expect("install test pack");
    let assignments = [assignment(tenant, org_of(&scopes), "authz2-blanket")];
    let context = AuthzContext {
        sensitivity: Some(Sensitivity::Internal),
        scopes: &scopes,
        assignments: &assignments,
        ..Default::default()
    };

    let allowed = pdp
        .authorize(
            &principal(tenant),
            Action::ScopeRead,
            Resource::Scope(team),
            &context,
        )
        .expect("authorize");
    assert!(
        allowed.allowed,
        "a clean principal keeps the blanket permit"
    );
    assert_eq!(allowed.pack_name, "authz2-blanket");

    let denied = pdp
        .authorize(
            &Principal {
                quarantined: true,
                ..principal(tenant)
            },
            Action::ScopeRead,
            Resource::Scope(team),
            &context,
        )
        .expect("authorize");
    assert!(
        !denied.allowed,
        "the compiled-in base forbid overrides the pack's blanket permit"
    );
    assert!(
        !denied.determining.is_empty(),
        "the quarantine forbid must be the determining policy"
    );
}

#[test]
fn the_default_pack_denies_a_foreign_principal_everything() {
    let pdp = Pdp::new().expect("build pdp");
    let victim = TenantId::new();
    let scopes = chain(victim);
    let team = team_of(&scopes);
    let intruder = principal(TenantId::new());
    let context = AuthzContext {
        sensitivity: Some(Sensitivity::Internal),
        scopes: &scopes,
        ..Default::default()
    };

    for action in ADMIN_ACTIONS {
        let decision = pdp
            .authorize(&intruder, action, Resource::Scope(team), &context)
            .expect("authorize");
        assert!(!decision.allowed, "{action} must be denied cross-tenant");
    }

    // require() renders the denial into the taxonomy with the pack version.
    let denial = pdp
        .require(
            &intruder,
            Action::ScopeRead,
            Resource::Tenant(victim),
            &AuthzContext::default(),
        )
        .expect_err("cross-tenant require must deny");
    match denial {
        Error::PolicyDenied {
            action,
            resource,
            reason,
        } => {
            assert_eq!(action, "scope.read");
            assert_eq!(resource, format!("tenant {victim}"));
            assert!(
                reason.contains(&format!("{REGULATED_STRICT}@19")),
                "denial must name pack@version, got: {reason}"
            );
        }
        other => panic!("expected PolicyDenied, got {other:?}"),
    }
}

/// A scope resource whose chain was not supplied has no ancestors in the
/// entity graph: membership cannot be proven, so the decision fails closed.
#[test]
fn a_scope_without_its_chain_fails_closed() {
    let pdp = Pdp::new().expect("build pdp");
    let tenant = TenantId::new();
    let decision = pdp
        .authorize(
            &principal(tenant),
            Action::ScopeRead,
            Resource::Scope(ScopeId::new()),
            &AuthzContext::default(),
        )
        .expect("authorize");
    assert!(!decision.allowed, "an unmaterialised scope must deny");
}

/// Per-node application (ADR-0014 decisions 3–4): the nearest assignment
/// on the resource's chain decides; deeper assignments override shallower
/// ones; unassigned chains fall to the tenant default, then the embedded
/// default.
#[test]
fn effective_pack_resolution_walks_nearest_assignment_first() {
    let pdp = Pdp::new().expect("build pdp");
    let tenant = TenantId::new();
    let scopes = chain(tenant);
    let (org, dept, team) = (scopes[0].id, scopes[1].id, scopes[2].id);
    let alice = principal(tenant);
    pdp.install_source(
        tenant,
        "authz2-readonly",
        4,
        READ_ONLY_PACK,
        PackConfig::default(),
    )
    .expect("install test pack");

    // Assigned at the department: the team inherits it.
    let at_dept = [assignment(tenant, dept, "authz2-readonly")];
    let denied = pdp
        .authorize(
            &alice,
            Action::ScopeUpdate,
            Resource::Scope(team),
            &AuthzContext {
                sensitivity: Some(Sensitivity::Internal),
                scopes: &scopes,
                assignments: &at_dept,
                ..Default::default()
            },
        )
        .expect("authorize");
    assert!(!denied.allowed, "the inherited read-only pack must deny");
    assert_eq!(denied.pack_name, "authz2-readonly");
    assert_eq!(denied.pack_version, 4);

    // The org above the assignment is out of its reach: default applies.
    let anchors = [admin_anchor(scopes[0].kind, scopes[0].id)];
    let org_decision = pdp
        .authorize(
            &alice,
            Action::ScopeUpdate,
            Resource::Scope(org),
            &AuthzContext {
                sensitivity: Some(Sensitivity::Internal),
                scopes: &scopes[..1],
                assignments: &at_dept,
                anchors: &anchors,
                ..Default::default()
            },
        )
        .expect("authorize");
    assert!(org_decision.allowed);
    assert_eq!(org_decision.pack_name, REGULATED_STRICT);

    // A deeper assignment overrides the inherited one: nearest wins.
    let overridden = [
        assignment(tenant, dept, "authz2-readonly"),
        assignment(tenant, team, REGULATED_STRICT),
    ];
    let nearest = pdp
        .authorize(
            &alice,
            Action::ScopeUpdate,
            Resource::Scope(team),
            &AuthzContext {
                sensitivity: Some(Sensitivity::Internal),
                scopes: &scopes,
                assignments: &overridden,
                anchors: &anchors,
                ..Default::default()
            },
        )
        .expect("authorize");
    assert!(nearest.allowed, "the node's own assignment must win");
    assert_eq!(nearest.pack_name, REGULATED_STRICT);

    // No assignment on the chain: the tenant default decides.
    let by_default = pdp
        .authorize(
            &alice,
            Action::ScopeUpdate,
            Resource::Scope(team),
            &AuthzContext {
                sensitivity: Some(Sensitivity::Internal),
                scopes: &scopes,
                default_pack: Some("authz2-readonly"),
                anchors: &anchors,
                ..Default::default()
            },
        )
        .expect("authorize");
    assert!(!by_default.allowed);
    assert_eq!(by_default.pack_name, "authz2-readonly");
}

/// `PolicyAssign` is decided under the pack the node *inherits* — the
/// resolution skips the node's own assignment (ADR-0014 decision 4): a
/// restrictive pack cannot seal its own node against reassignment.
#[test]
fn policy_assign_is_decided_under_the_inherited_pack() {
    let pdp = Pdp::new().expect("build pdp");
    let tenant = TenantId::new();
    let scopes = chain(tenant);
    let team = team_of(&scopes);
    let alice = principal(tenant);
    pdp.install_source(
        tenant,
        "authz2-frozen",
        1,
        READ_ONLY_PACK,
        PackConfig::default(),
    )
    .expect("install test pack");
    let assignments = [assignment(tenant, team, "authz2-frozen")];
    let anchors = [admin_anchor(scopes[0].kind, scopes[0].id)];
    let context = AuthzContext {
        sensitivity: Some(Sensitivity::Internal),
        scopes: &scopes,
        assignments: &assignments,
        anchors: &anchors,
        ..Default::default()
    };

    // The frozen pack governs the node's ordinary actions...
    let frozen = pdp
        .authorize(&alice, Action::ScopeUpdate, Resource::Scope(team), &context)
        .expect("authorize");
    assert!(!frozen.allowed);
    assert_eq!(frozen.pack_name, "authz2-frozen");

    // ...but changing the node's governance is decided by the pack it
    // inherits (here the embedded default), which permits it.
    let rescue = pdp
        .authorize(
            &alice,
            Action::PolicyAssign,
            Resource::Scope(team),
            &context,
        )
        .expect("authorize");
    assert!(
        rescue.allowed,
        "reassignment must not be sealed by the node's own pack"
    );
    assert_eq!(rescue.pack_name, REGULATED_STRICT);

    // The display resolution keeps answering "what governs this node":
    // the node's own assignment.
    let shown = pdp.effective(tenant, Resource::Scope(team), &context);
    assert_eq!(shown.name, "authz2-frozen");
}

/// An assigned name with no compiled pack falls back to the embedded
/// default — strict, never dark (ADR-0014 decision 7).
#[test]
fn a_dangling_assignment_falls_back_to_regulated_strict() {
    let pdp = Pdp::new().expect("build pdp");
    let tenant = TenantId::new();
    let scopes = chain(tenant);
    let team = team_of(&scopes);
    let assignments = [assignment(tenant, team, "never-stored")];
    let anchors = [admin_anchor(scopes[0].kind, scopes[0].id)];
    let decision = pdp
        .authorize(
            &principal(tenant),
            Action::ScopeRead,
            Resource::Scope(team),
            &AuthzContext {
                sensitivity: Some(Sensitivity::Internal),
                scopes: &scopes,
                assignments: &assignments,
                anchors: &anchors,
                ..Default::default()
            },
        )
        .expect("authorize");
    assert!(decision.allowed, "the fallback pack still admits the admin");
    assert_eq!(decision.pack_name, REGULATED_STRICT);
}

/// Stored packs are tenant-isolated and hot-swappable by name; removal
/// leaves other tenants and other packs untouched.
#[test]
fn stored_packs_install_and_remove_by_name_per_tenant() {
    let pdp = Pdp::new().expect("build pdp");
    let tenant = TenantId::new();
    let other_tenant = TenantId::new();
    let scopes = chain(tenant);
    let team = team_of(&scopes);
    let alice = principal(tenant);

    pdp.install_source(
        tenant,
        "authz2-readonly",
        7,
        READ_ONLY_PACK,
        PackConfig::default(),
    )
    .expect("install test pack");
    assert_eq!(
        pdp.installed_versions(tenant),
        vec![("authz2-readonly".to_owned(), 7)]
    );
    assert!(
        pdp.installed_versions(other_tenant).is_empty(),
        "other tenants must not see the pack"
    );

    // The other tenant's identically-named assignment resolves to nothing
    // stored and falls back — never to the first tenant's pack.
    let other_scopes = chain(other_tenant);
    let other_assignments = [assignment(
        other_tenant,
        team_of(&other_scopes),
        "authz2-readonly",
    )];
    let other_anchors = [admin_anchor(other_scopes[0].kind, other_scopes[0].id)];
    let other_decision = pdp
        .authorize(
            &principal(other_tenant),
            Action::ScopeUpdate,
            Resource::Scope(team_of(&other_scopes)),
            &AuthzContext {
                sensitivity: Some(Sensitivity::Internal),
                scopes: &other_scopes,
                assignments: &other_assignments,
                anchors: &other_anchors,
                ..Default::default()
            },
        )
        .expect("authorize other tenant");
    assert!(other_decision.allowed);
    assert_eq!(other_decision.pack_name, REGULATED_STRICT);

    // Removal: assigned scopes fall back to the default at decision time.
    let assignments = [assignment(tenant, team, "authz2-readonly")];
    let anchors = [admin_anchor(scopes[0].kind, scopes[0].id)];
    let context = AuthzContext {
        sensitivity: Some(Sensitivity::Internal),
        scopes: &scopes,
        assignments: &assignments,
        anchors: &anchors,
        ..Default::default()
    };
    assert!(pdp.remove_pack(tenant, "authz2-readonly"));
    assert!(
        !pdp.remove_pack(tenant, "authz2-readonly"),
        "second removal is a no-op"
    );
    assert!(pdp.installed_versions(tenant).is_empty());
    let restored = pdp
        .authorize(&alice, Action::ScopeUpdate, Resource::Scope(team), &context)
        .expect("authorize after removal");
    assert!(restored.allowed);
    assert_eq!(restored.pack_name, REGULATED_STRICT);
}

/// Product names are reserved (ADR-0014 decision 6): `regulated-strict`
/// must mean the same thing in every tenant, forever.
#[test]
fn reserved_pack_names_cannot_be_stored() {
    let pdp = Pdp::new().expect("build pdp");
    let tenant = TenantId::new();
    for name in [
        "regulated-strict",
        "standard",
        "open-collaboration",
        "bootstrap",
    ] {
        assert!(is_reserved(name), "{name} must be reserved");
        let refused = pdp.install_source(tenant, name, 1, READ_ONLY_PACK, PackConfig::default());
        assert!(
            matches!(refused, Err(Error::Invalid { .. })),
            "storing {name} must be refused, got {refused:?}"
        );
    }
    assert!(!is_reserved("acme-strict"));
}

#[test]
fn an_explicit_forbid_reports_its_determining_policy() {
    let pdp = Pdp::new().expect("build pdp");
    let tenant = TenantId::new();
    let scopes = chain(tenant);
    let team = team_of(&scopes);
    pdp.install_source(
        tenant,
        "authz2-no-delete",
        1,
        r#"
        permit (principal, action, resource) when { resource in principal.tenant };
        forbid (principal, action == Synveda::Action::"ScopeUpdate", resource);
        "#,
        PackConfig::default(),
    )
    .expect("install test pack");
    let assignments = [assignment(tenant, team, "authz2-no-delete")];

    let decision = pdp
        .authorize(
            &principal(tenant),
            Action::ScopeUpdate,
            Resource::Scope(team),
            &AuthzContext {
                sensitivity: Some(Sensitivity::Internal),
                scopes: &scopes,
                assignments: &assignments,
                ..Default::default()
            },
        )
        .expect("authorize");
    assert!(!decision.allowed);
    assert!(
        !decision.determining.is_empty(),
        "a forbid-driven denial must name the forbidding policy"
    );
}

#[test]
fn invalid_packs_are_rejected_and_leave_the_previous_pack_in_force() {
    let pdp = Pdp::new().expect("build pdp");
    let tenant = TenantId::new();
    let scopes = chain(tenant);
    let team = team_of(&scopes);
    let alice = principal(tenant);

    pdp.install_source(
        tenant,
        "authz2-readonly",
        1,
        READ_ONLY_PACK,
        PackConfig::default(),
    )
    .expect("install good pack");

    // Syntax error: does not parse.
    let syntax = pdp.install_source(
        tenant,
        "authz2-readonly",
        2,
        "permit (principal",
        PackConfig::default(),
    );
    assert!(
        matches!(syntax, Err(Error::Invalid { .. })),
        "a syntax error must be Invalid, got {syntax:?}"
    );

    // Well-formed but outside the schema: fails validation.
    let unknown_action = pdp.install_source(
        tenant,
        "authz2-readonly",
        3,
        r#"permit (principal, action == Synveda::Action::"LaunchMissiles", resource);"#,
        PackConfig::default(),
    );
    assert!(
        matches!(unknown_action, Err(Error::Invalid { .. })),
        "an out-of-schema pack must be Invalid, got {unknown_action:?}"
    );

    // The last-good pack still decides (ADR-0012 decision 5).
    assert_eq!(
        pdp.installed_versions(tenant),
        vec![("authz2-readonly".to_owned(), 1)]
    );
    let assignments = [assignment(tenant, team, "authz2-readonly")];
    let decision = pdp
        .authorize(
            &alice,
            Action::ScopeUpdate,
            Resource::Scope(team),
            &AuthzContext {
                sensitivity: Some(Sensitivity::Internal),
                scopes: &scopes,
                assignments: &assignments,
                ..Default::default()
            },
        )
        .expect("authorize");
    assert!(
        !decision.allowed,
        "the read-only pack must still be in force"
    );
    assert_eq!(decision.pack_name, "authz2-readonly");

    // compile_check applies the same gate without installing.
    assert!(pdp.compile_check("ok", READ_ONLY_PACK).is_ok());
    assert!(pdp.compile_check("bad", "permit (principal").is_err());
}
