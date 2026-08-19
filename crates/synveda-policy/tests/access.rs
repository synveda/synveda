//! CPR-5: the four access-plane actions, across all three shipped packs
//! (ADR-0072 decision 7).
//!
//! The plane's routes decide at the **tenant** for the reason CPR-4's do — the
//! Cedar entity model still materialises `Scope` from the old hierarchy — so
//! this suite decides where the routes decide, and asserts the properties the
//! packs actually differ on rather than re-running the whole role×action matrix
//! one plane over.
//!
//! Four questions:
//!
//! 1. Who may **grant** and who may **curate groups** — the admin roles, under
//!    every pack, because handing out access is administration.
//! 2. Who may **read** membership — and this is the one the packs grade
//!    differently, deliberately: `regulated-strict` keeps it to the admin
//!    roles, `standard` adds the reviewing role, `open-collaboration` adds
//!    every content role. A collaboration space whose members cannot see who
//!    else is in it is not one; a regulated one where a contractor can
//!    enumerate the staff is a different problem.
//! 3. Whether **redeeming an invitation** needs a role. It must not: the token
//!    is the authority, and a person this product invited must not be turned
//!    away for want of a binding.
//! 4. Whether the **invariant floor** still runs over all four. It must: a
//!    quarantined principal does nothing, invitation or no invitation.

use chrono::Utc;
use synveda_policy::{
    Action, AuthzContext, OPEN_COLLABORATION, Pdp, Principal, REGULATED_STRICT, Resource, STANDARD,
};
use synveda_types::{
    HierarchyNode, PolicyAssignment, Role, RoleBinding, ScopeId, ScopeKind, TenantId,
};

/// The access plane's four actions.
const ACTIONS: [Action; 4] = [
    Action::MembershipRead,
    Action::MembershipGrant,
    Action::GroupManage,
    Action::InviteAccept,
];

struct Fixture {
    tenant: TenantId,
    org: HierarchyNode,
}

fn fixture() -> Fixture {
    let tenant = TenantId::new();
    Fixture {
        tenant,
        org: HierarchyNode {
            id: ScopeId::new(),
            tenant_id: tenant,
            parent_id: None,
            kind: ScopeKind::Org,
            slug: "org".to_owned(),
            name: "org".to_owned(),
            depth: 0,
            path: "org".to_owned(),
            sealed: false,
            created_at: Utc::now(),
        },
    }
}

fn principal(fx: &Fixture, subject: &str, quarantined: bool) -> Principal {
    Principal {
        tenant_id: fx.tenant,
        subject: subject.to_owned(),
        quarantined,
        scope_id: Some(fx.org.id),
        token_scope: None,
    }
}

fn binding(fx: &Fixture, subject: &str, role: Role) -> RoleBinding {
    RoleBinding {
        tenant_id: fx.tenant,
        subject: subject.to_owned(),
        scope_id: None,
        role,
        updated_at: Utc::now(),
    }
}

fn assignment(fx: &Fixture, pack: &str) -> PolicyAssignment {
    PolicyAssignment {
        tenant_id: fx.tenant,
        scope_id: fx.org.id,
        pack_name: pack.to_owned(),
        updated_at: Utc::now(),
    }
}

/// One decision at the tenant, under `pack`, with `roles` bound tenant-wide —
/// exactly the shape `crate::access`'s `require` produces.
fn allows(pdp: &Pdp, fx: &Fixture, pack: &str, roles: &[Role], action: Action) -> bool {
    decide(pdp, fx, pack, roles, action, false)
}

fn decide(
    pdp: &Pdp,
    fx: &Fixture,
    pack: &str,
    roles: &[Role],
    action: Action,
    quarantined: bool,
) -> bool {
    let subject = "sam";
    let bindings: Vec<RoleBinding> = roles
        .iter()
        .map(|role| binding(fx, subject, *role))
        .collect();
    let chain = [fx.org.clone()];
    pdp.authorize(
        &principal(fx, subject, quarantined),
        action,
        Resource::Tenant(fx.tenant),
        &AuthzContext {
            scopes: &[],
            principal_scopes: &chain,
            assignments: &[assignment(fx, pack)],
            default_pack: Some(pack),
            role_bindings: &bindings,
            ..Default::default()
        },
    )
    .expect("authorize")
    .allowed
}

fn pdp() -> Pdp {
    Pdp::new().expect("build the embedded PDP")
}

/// Handing out access is administration, priced with the rest of it — under
/// every pack, so a tenant that switched packs does not silently move who may
/// add somebody to a workspace.
#[test]
fn granting_and_group_curation_are_the_admin_roles_under_every_pack() {
    let pdp = pdp();
    let fx = fixture();
    for pack in [REGULATED_STRICT, STANDARD, OPEN_COLLABORATION] {
        for action in [Action::MembershipGrant, Action::GroupManage] {
            for role in [Role::Steward, Role::OrgAdmin] {
                assert!(
                    allows(&pdp, &fx, pack, &[role], action),
                    "{pack}: {role} must hold {action}"
                );
            }
            for role in [
                Role::Viewer,
                Role::Contributor,
                Role::Curator,
                Role::Auditor,
                Role::SecurityReviewer,
                Role::Compliance,
            ] {
                assert!(
                    !allows(&pdp, &fx, pack, &[role], action),
                    "{pack}: {role} must not hold {action}"
                );
            }
            assert!(
                !allows(&pdp, &fx, pack, &[], action),
                "{pack}: {action} is not role-free"
            );
        }
    }
}

/// Reading membership is an administrative read everywhere, and the packs then
/// grade **who else** sees it. This is the one place this feature makes the
/// three packs differ, and the gradient is the point.
#[test]
fn the_packs_grade_who_may_read_membership() {
    let pdp = pdp();
    let fx = fixture();

    // Everywhere: the admin and audit roles.
    for pack in [REGULATED_STRICT, STANDARD, OPEN_COLLABORATION] {
        for role in [Role::Steward, Role::OrgAdmin, Role::Auditor] {
            assert!(
                allows(&pdp, &fx, pack, &[role], Action::MembershipRead),
                "{pack}: {role} must be able to read membership"
            );
        }
    }

    // regulated-strict: nobody else. A contractor with a viewer binding must
    // not be able to enumerate the staff.
    for role in [Role::Viewer, Role::Contributor, Role::Curator] {
        assert!(
            !allows(&pdp, &fx, REGULATED_STRICT, &[role], Action::MembershipRead),
            "regulated-strict must keep membership to the admin roles: {role}"
        );
    }

    // standard: the reviewing role joins. A curator casts verdicts on other
    // people's work and cannot do that without seeing whose work it is.
    assert!(allows(
        &pdp,
        &fx,
        STANDARD,
        &[Role::Curator],
        Action::MembershipRead
    ));
    for role in [Role::Viewer, Role::Contributor] {
        assert!(
            !allows(&pdp, &fx, STANDARD, &[role], Action::MembershipRead),
            "standard does not extend membership reads to {role}"
        );
    }

    // open-collaboration: every content role. A collaboration space whose
    // members cannot see who else is in it is not one.
    for role in [Role::Viewer, Role::Contributor, Role::Curator] {
        assert!(
            allows(
                &pdp,
                &fx,
                OPEN_COLLABORATION,
                &[role],
                Action::MembershipRead
            ),
            "open-collaboration extends membership reads to {role}"
        );
    }
}

/// Reading membership is **not** reading a workspace. The two are separate
/// actions precisely so a pack can grant one without the other, and this is the
/// assertion that keeps somebody from folding them together later.
#[test]
fn seeing_a_workspace_is_not_seeing_who_is_in_it() {
    let pdp = pdp();
    let fx = fixture();
    // Under regulated-strict a contributor reads the workspace (CPR-4 widened
    // the packs for exactly that) and must not read its membership.
    assert!(allows(
        &pdp,
        &fx,
        REGULATED_STRICT,
        &[Role::Contributor],
        Action::WorkspaceRead
    ));
    assert!(!allows(
        &pdp,
        &fx,
        REGULATED_STRICT,
        &[Role::Contributor],
        Action::MembershipRead
    ));
}

/// Redeeming an invitation needs the **token**, not a role — under every pack.
/// A person this product invited and then turned away for want of a binding is
/// the failure this permit exists to prevent.
#[test]
fn redeeming_an_invitation_needs_no_role_under_any_pack() {
    let pdp = pdp();
    let fx = fixture();
    for pack in [REGULATED_STRICT, STANDARD, OPEN_COLLABORATION] {
        assert!(
            allows(&pdp, &fx, pack, &[], Action::InviteAccept),
            "{pack}: a role-free principal must be able to redeem an invitation"
        );
        assert!(
            allows(&pdp, &fx, pack, &[Role::Viewer], Action::InviteAccept),
            "{pack}: and so must one with any binding at all"
        );
    }
}

/// The invariant floor still runs. A quarantined principal does nothing on this
/// plane — invitation or no invitation — which is why redeeming is an action
/// rather than an exemption.
#[test]
fn the_base_layer_forbids_a_quarantined_principal_everything_here() {
    let pdp = pdp();
    let fx = fixture();
    for pack in [REGULATED_STRICT, STANDARD, OPEN_COLLABORATION] {
        for action in ACTIONS {
            assert!(
                !decide(&pdp, &fx, pack, &[Role::OrgAdmin], action, true),
                "{pack}: a quarantined org-admin must not hold {action}"
            );
        }
    }
}

/// A service identity cannot reach this plane at all: its token confinement
/// forbids every tenant-plane action, and all four of these are decided at the
/// tenant. An agent must not redeem a person's invitation or hand itself a
/// grant.
#[test]
fn a_service_identity_cannot_reach_the_access_plane() {
    let pdp = pdp();
    let fx = fixture();
    let anchored = Principal {
        tenant_id: fx.tenant,
        subject: "agent".to_owned(),
        quarantined: false,
        scope_id: Some(fx.org.id),
        // Confined to the org node: the tenant plane is never inside a scope
        // subtree, so the base layer's forbid covers everything here.
        token_scope: Some(fx.org.id),
    };
    let chain = [fx.org.clone()];
    for pack in [REGULATED_STRICT, STANDARD, OPEN_COLLABORATION] {
        for action in ACTIONS {
            let decision = pdp
                .authorize(
                    &anchored,
                    action,
                    Resource::Tenant(fx.tenant),
                    &AuthzContext {
                        scopes: &[],
                        principal_scopes: &chain,
                        assignments: &[assignment(&fx, pack)],
                        default_pack: Some(pack),
                        role_bindings: &[binding(&fx, "agent", Role::OrgAdmin)],
                        ..Default::default()
                    },
                )
                .expect("authorize");
            assert!(
                !decision.allowed,
                "{pack}: a confined agent holding org-admin must still be \
                 refused {action}"
            );
        }
    }
}

/// Every action this feature adds is decidable under every pack — no typo in a
/// Cedar id, which would read as a denial rather than as the mistake it is.
#[test]
fn every_new_action_is_decidable_under_every_pack() {
    let pdp = pdp();
    let fx = fixture();
    for pack in [REGULATED_STRICT, STANDARD, OPEN_COLLABORATION] {
        for action in ACTIONS {
            // The call itself is the assertion: an unknown action id fails
            // schema validation and errors rather than returning a verdict.
            let _ = allows(&pdp, &fx, pack, &[Role::OrgAdmin], action);
        }
    }
}
