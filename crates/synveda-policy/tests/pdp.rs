//! AUTHZ-1 golden tests for the facade: bootstrap allow/deny, default-deny,
//! pack install/hot-swap/remove, compile rejection, and the decision
//! metadata (pack name + version + determining policies) every call
//! carries. Restrictive behaviour is exercised through *test policy packs*
//! installed via the same path the reloader uses — never a PDP bypass
//! (CLAUDE.md, seed §2.2).

use chrono::Utc;
use synveda_policy::{
    Action, AuthzContext, BOOTSTRAP_PACK, BOOTSTRAP_VERSION, Pdp, Principal, Resource,
};
use synveda_types::{Error, HierarchyNode, ScopeId, ScopeKind, TenantId};

const ALL_ACTIONS: [Action; 4] = [
    Action::HierarchyCreate,
    Action::HierarchyRead,
    Action::HierarchyUpdate,
    Action::HierarchyDelete,
];

/// A pack that only permits reads — the shape AUTHZ-2's `regulated-strict`
/// takes for non-curators.
const READ_ONLY_PACK: &str = r#"
permit (
    principal,
    action == Synveda::Action::"HierarchyRead",
    resource
) when { resource in principal.tenant };
"#;

fn node(
    tenant_id: TenantId,
    id: ScopeId,
    parent_id: Option<ScopeId>,
    kind: ScopeKind,
    slug: &str,
    depth: i32,
    path: &str,
) -> HierarchyNode {
    HierarchyNode {
        id,
        tenant_id,
        parent_id,
        kind,
        slug: slug.to_owned(),
        name: slug.to_owned(),
        depth,
        path: path.to_owned(),
        created_at: Utc::now(),
    }
}

/// An org → department → team chain for `tenant`, deepest scope last.
fn chain(tenant: TenantId) -> Vec<HierarchyNode> {
    let org = ScopeId::new();
    let dept = ScopeId::new();
    let team = ScopeId::new();
    vec![
        node(tenant, org, None, ScopeKind::Org, "acme", 0, "acme"),
        node(
            tenant,
            dept,
            Some(org),
            ScopeKind::Department,
            "payments",
            1,
            "acme/payments",
        ),
        node(
            tenant,
            team,
            Some(dept),
            ScopeKind::Team,
            "core",
            2,
            "acme/payments/core",
        ),
    ]
}

fn team_of(chain: &[HierarchyNode]) -> ScopeId {
    chain.last().expect("chain is non-empty").id
}

fn principal(tenant_id: TenantId) -> Principal {
    Principal {
        tenant_id,
        subject: "alice".to_owned(),
        quarantined: false,
    }
}

/// AUTH-2 (ADR-0013 decision 5): a quarantined principal is forbidden
/// everything under `bootstrap@2`, even inside its own tenant, and the
/// denial names the forbidding policy.
#[test]
fn bootstrap_forbids_a_quarantined_principal_everything() {
    let pdp = Pdp::new().expect("build pdp");
    let tenant = TenantId::new();
    let scopes = chain(tenant);
    let team = team_of(&scopes);
    let quarantined = Principal {
        quarantined: true,
        ..principal(tenant)
    };

    for action in ALL_ACTIONS {
        let decision = pdp
            .authorize(
                &quarantined,
                action,
                Resource::Scope(team),
                &AuthzContext { scopes: &scopes },
            )
            .expect("authorize");
        assert!(
            !decision.allowed,
            "{action} must be denied when quarantined"
        );
        assert_eq!(decision.pack_name, BOOTSTRAP_PACK);
        assert_eq!(decision.pack_version, BOOTSTRAP_VERSION);
        assert!(
            !decision.determining.is_empty(),
            "the quarantine forbid must be the determining policy"
        );
    }

    // Tenant-level resources too: quarantine has no carve-outs.
    let decision = pdp
        .authorize(
            &quarantined,
            Action::HierarchyRead,
            Resource::Tenant(tenant),
            &AuthzContext::default(),
        )
        .expect("authorize");
    assert!(!decision.allowed, "tenant-level reads are forbidden too");
}

/// The quarantine forbid overrides permits in *stored* packs as well —
/// but only while the pack's own rules keep the attribute in play; the
/// forbid itself lives in each pack, so a stored pack that omits it
/// relies on its own permits' conditions. This pins the bootstrap
/// behaviour stored packs inherit when AUTHZ-2 templates them.
#[test]
fn a_stored_pack_with_the_quarantine_forbid_behaves_like_bootstrap() {
    let pdp = Pdp::new().expect("build pdp");
    let tenant = TenantId::new();
    let scopes = chain(tenant);
    let team = team_of(&scopes);
    pdp.install_source(
        tenant,
        "auth2-strict",
        1,
        r#"
        forbid (principal, action, resource) when { principal.quarantined };
        permit (principal, action, resource) when { resource in principal.tenant };
        "#,
    )
    .expect("install test pack");

    let allowed = pdp
        .authorize(
            &principal(tenant),
            Action::HierarchyRead,
            Resource::Scope(team),
            &AuthzContext { scopes: &scopes },
        )
        .expect("authorize");
    assert!(allowed.allowed, "a placed principal keeps its rights");

    let denied = pdp
        .authorize(
            &Principal {
                quarantined: true,
                ..principal(tenant)
            },
            Action::HierarchyRead,
            Resource::Scope(team),
            &AuthzContext { scopes: &scopes },
        )
        .expect("authorize");
    assert!(!denied.allowed, "the forbid overrides the blanket permit");
}

#[test]
fn bootstrap_allows_a_tenant_principal_to_administer_its_own_hierarchy() {
    let pdp = Pdp::new().expect("build pdp");
    let tenant = TenantId::new();
    let scopes = chain(tenant);
    let team = team_of(&scopes);
    let alice = principal(tenant);

    for action in ALL_ACTIONS {
        let decision = pdp
            .authorize(
                &alice,
                action,
                Resource::Scope(team),
                &AuthzContext { scopes: &scopes },
            )
            .expect("authorize");
        assert!(decision.allowed, "{action} must be allowed on own scope");
        assert_eq!(decision.pack_name, BOOTSTRAP_PACK);
        assert_eq!(decision.pack_version, BOOTSTRAP_VERSION);
        assert!(
            !decision.determining.is_empty(),
            "an allow must name its permitting policies"
        );
    }

    // Tenant-level resources (creating the org root, reading the root)
    // need no chain at all.
    for action in [Action::HierarchyCreate, Action::HierarchyRead] {
        let decision = pdp
            .authorize(
                &alice,
                action,
                Resource::Tenant(tenant),
                &AuthzContext::default(),
            )
            .expect("authorize");
        assert!(decision.allowed, "{action} must be allowed on own tenant");
    }
}

#[test]
fn bootstrap_denies_a_foreign_principal_everything() {
    let pdp = Pdp::new().expect("build pdp");
    let victim = TenantId::new();
    let scopes = chain(victim);
    let team = team_of(&scopes);
    let intruder = principal(TenantId::new());

    for action in ALL_ACTIONS {
        let decision = pdp
            .authorize(
                &intruder,
                action,
                Resource::Scope(team),
                &AuthzContext { scopes: &scopes },
            )
            .expect("authorize");
        assert!(!decision.allowed, "{action} must be denied cross-tenant");
    }

    // require() renders the denial into the taxonomy with the pack version.
    let denial = pdp
        .require(
            &intruder,
            Action::HierarchyRead,
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
            assert_eq!(action, "hierarchy.read");
            assert_eq!(resource, format!("tenant {victim}"));
            assert!(
                reason.contains(&format!("{BOOTSTRAP_PACK}@{BOOTSTRAP_VERSION}")),
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
            Action::HierarchyRead,
            Resource::Scope(ScopeId::new()),
            &AuthzContext::default(),
        )
        .expect("authorize");
    assert!(!decision.allowed, "an unmaterialised scope must deny");
}

#[test]
fn installed_packs_swap_decisions_and_report_their_version() {
    let pdp = Pdp::new().expect("build pdp");
    let tenant = TenantId::new();
    let other_tenant = TenantId::new();
    let scopes = chain(tenant);
    let team = team_of(&scopes);
    let alice = principal(tenant);
    let context = AuthzContext { scopes: &scopes };

    pdp.install_source(tenant, "authz1-readonly", 7, READ_ONLY_PACK)
        .expect("install test pack");
    assert_eq!(
        pdp.installed_version(tenant),
        Some(("authz1-readonly".to_owned(), 7))
    );

    let read = pdp
        .authorize(
            &alice,
            Action::HierarchyRead,
            Resource::Scope(team),
            &context,
        )
        .expect("authorize read");
    assert!(read.allowed, "the test pack permits reads");
    assert_eq!(read.pack_name, "authz1-readonly");
    assert_eq!(read.pack_version, 7);

    let write = pdp
        .authorize(
            &alice,
            Action::HierarchyDelete,
            Resource::Scope(team),
            &context,
        )
        .expect("authorize delete");
    assert!(!write.allowed, "the test pack does not permit mutations");
    assert_eq!(write.pack_name, "authz1-readonly");
    assert_eq!(write.pack_version, 7);

    // Other tenants keep running bootstrap, unaffected.
    let other_scopes = chain(other_tenant);
    let other_decision = pdp
        .authorize(
            &principal(other_tenant),
            Action::HierarchyDelete,
            Resource::Scope(team_of(&other_scopes)),
            &AuthzContext {
                scopes: &other_scopes,
            },
        )
        .expect("authorize other tenant");
    assert!(other_decision.allowed);
    assert_eq!(other_decision.pack_name, BOOTSTRAP_PACK);

    // Removal falls back to bootstrap — hot reload in both directions.
    assert!(pdp.remove_pack(tenant));
    assert!(!pdp.remove_pack(tenant), "second removal is a no-op");
    assert_eq!(pdp.installed_version(tenant), None);
    let restored = pdp
        .authorize(
            &alice,
            Action::HierarchyDelete,
            Resource::Scope(team),
            &context,
        )
        .expect("authorize after removal");
    assert!(restored.allowed);
    assert_eq!(restored.pack_name, BOOTSTRAP_PACK);
}

#[test]
fn an_explicit_forbid_reports_its_determining_policy() {
    let pdp = Pdp::new().expect("build pdp");
    let tenant = TenantId::new();
    let scopes = chain(tenant);
    let team = team_of(&scopes);
    let context = AuthzContext { scopes: &scopes };
    pdp.install_source(
        tenant,
        "authz1-no-delete",
        1,
        r#"
        permit (principal, action, resource) when { resource in principal.tenant };
        forbid (principal, action == Synveda::Action::"HierarchyDelete", resource);
        "#,
    )
    .expect("install test pack");

    let decision = pdp
        .authorize(
            &principal(tenant),
            Action::HierarchyDelete,
            Resource::Scope(team),
            &context,
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
    let context = AuthzContext { scopes: &scopes };

    pdp.install_source(tenant, "authz1-readonly", 1, READ_ONLY_PACK)
        .expect("install good pack");

    // Syntax error: does not parse.
    let syntax = pdp.install_source(tenant, "authz1-broken", 2, "permit (principal");
    assert!(
        matches!(syntax, Err(Error::Invalid { .. })),
        "a syntax error must be Invalid, got {syntax:?}"
    );

    // Well-formed but outside the schema: fails validation.
    let unknown_action = pdp.install_source(
        tenant,
        "authz1-unknown",
        3,
        r#"permit (principal, action == Synveda::Action::"LaunchMissiles", resource);"#,
    );
    assert!(
        matches!(unknown_action, Err(Error::Invalid { .. })),
        "an out-of-schema pack must be Invalid, got {unknown_action:?}"
    );

    // The last-good pack still decides (ADR-0012 decision 5).
    assert_eq!(
        pdp.installed_version(tenant),
        Some(("authz1-readonly".to_owned(), 1))
    );
    let decision = pdp
        .authorize(
            &alice,
            Action::HierarchyDelete,
            Resource::Scope(team),
            &context,
        )
        .expect("authorize");
    assert!(
        !decision.allowed,
        "the read-only pack must still be in force"
    );
    assert_eq!(decision.pack_name, "authz1-readonly");

    // compile_check applies the same gate without installing.
    assert!(pdp.compile_check("ok", READ_ONLY_PACK).is_ok());
    assert!(pdp.compile_check("bad", "permit (principal").is_err());
}
