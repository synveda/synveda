//! FLOW-3's first acceptance criterion: **full matrix golden tests**
//! (ADR-0032 decisions 3 and 4).
//!
//! The matrix resolves from asset × sensitivity × target scope kind ×
//! pack, so the golden table is every cell of that product — 3 packs × 6
//! asset kinds × 4 sensitivities × 5 scope kinds = 360 — rendered
//! canonically and compared against one checked-in literal. A wrong
//! requirement and a wrong *absence* of one both fail, and the diff names
//! the exact cell that moved.
//!
//! Beside it, the property no table can express: the invariant floor
//! (`restricted` → compliance + dual approval; `skill` →
//! security-reviewer) survives **every** pack, including a hostile stored
//! one written specifically to author it away.

use std::fmt::Write as _;

use synveda_policy::{OPEN_COLLABORATION, Pdp, REGULATED_STRICT, STANDARD, approvals};
use synveda_types::access::RoleKey;
use synveda_types::scope::ScopeKind;
use synveda_types::{
    ApprovalMatrix, ApprovalRule, AssetKind, CastApproval, IdentityId, PackConfig,
    RequirementOrigin, RoleRequirement, Sensitivity, TenantId,
};

const PACKS: [&str; 3] = [REGULATED_STRICT, STANDARD, OPEN_COLLABORATION];

const SCOPE_KINDS: [ScopeKind; 5] = [
    ScopeKind::Tenant,
    ScopeKind::OrgUnit,
    ScopeKind::Workspace,
    ScopeKind::Project,
    ScopeKind::Principal,
];

/// One cell, rendered: `pack asset sensitivity scope → requirement`.
fn render(pack: &str, matrix: &ApprovalMatrix) -> String {
    let mut out = String::new();
    for asset in AssetKind::ALL {
        for sensitivity in Sensitivity::ALL {
            for kind in SCOPE_KINDS {
                let requirement = matrix.resolve(asset, sensitivity, kind);
                let mut parts: Vec<String> = requirement
                    .roles
                    .iter()
                    .map(|required| format!("{}×{}", required.role, required.count))
                    .collect();
                if requirement.distinct_approvers > 0 {
                    parts.push(format!("{}distinct", requirement.distinct_approvers));
                }
                writeln!(
                    out,
                    "{pack:<18} {:<12} {:<12} {:<10} {}",
                    asset,
                    sensitivity,
                    kind.as_str(),
                    if parts.is_empty() {
                        "auto".to_owned()
                    } else {
                        parts.join(" + ")
                    }
                )
                .expect("write to a String cannot fail");
            }
        }
    }
    out
}

/// The full matrix, exactly as the three embedded packs resolve it.
///
/// This is the AC's golden table. Regenerate deliberately, never to make a
/// failure go away: every line is a statement about what it takes to move
/// content across the trust boundary.
const GOLDEN: &str = include_str!("golden/approval-matrix.txt");

#[test]
fn the_full_matrix_matches_its_golden_table() {
    let mut rendered = String::new();
    for pack in PACKS {
        rendered.push_str(&render(pack, &approvals::embedded(pack)));
    }
    // The regeneration hatch is deliberate and deliberately loud: a golden
    // table you cannot regenerate gets edited by hand until it agrees with
    // whatever the code does, which is the failure mode goldens exist to
    // prevent. `SYNVEDA_UPDATE_GOLDEN=1 cargo test -p synveda-policy
    // --test approvals` rewrites it, and the diff is the review.
    if std::env::var_os("SYNVEDA_UPDATE_GOLDEN").is_some() {
        std::fs::write(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/golden/approval-matrix.txt"
            ),
            &rendered,
        )
        .expect("rewrite the golden table");
        panic!("golden table rewritten; re-run without SYNVEDA_UPDATE_GOLDEN and review the diff");
    }
    assert_eq!(
        rendered, GOLDEN,
        "the approval matrix moved; if that is intended, regenerate with \
         SYNVEDA_UPDATE_GOLDEN=1 and review the diff"
    );
}

/// The three packs really are different at the cells the tech plan's §2.4
/// table calls out — a golden that happened to be identical everywhere
/// would pass while proving nothing about "× pack".
#[test]
fn the_packs_differ_where_the_tech_plan_says_they_do() {
    let strict = approvals::embedded(REGULATED_STRICT);
    let standard = approvals::embedded(STANDARD);
    let open = approvals::embedded(OPEN_COLLABORATION);

    // A working neighbourhood's own memory (a workspace): one curator
    // under regulated-strict, auto under the two sharing packs (tech plan
    // §2.4's SMB collapse; an org_unit is *shared* territory since CPR-7's
    // shape vocabulary, priced one notch higher).
    let local = |matrix: &ApprovalMatrix| {
        matrix.resolve(
            AssetKind::Knowledge,
            Sensitivity::Internal,
            ScopeKind::Workspace,
        )
    };
    assert_eq!(
        local(&strict).roles,
        vec![RoleRequirement::new(RoleKey::Curator, 1)]
    );
    assert!(
        local(&standard).is_empty(),
        "standard auto-approves at a team"
    );
    assert!(local(&open).is_empty(), "open-collaboration too");

    // Reaching past a team: two people under regulated-strict, one curator
    // under standard, and under open-collaboration only at the org.
    let dept = |matrix: &ApprovalMatrix| {
        matrix.resolve(
            AssetKind::Knowledge,
            Sensitivity::Internal,
            ScopeKind::OrgUnit,
        )
    };
    assert_eq!(dept(&strict).distinct_approvers, 2);
    assert_eq!(dept(&standard).distinct_approvers, 1);
    assert!(dept(&open).is_empty());
    assert_eq!(
        open.resolve(
            AssetKind::Knowledge,
            Sensitivity::Internal,
            ScopeKind::Tenant
        )
        .roles,
        vec![RoleRequirement::new(RoleKey::Curator, 1)],
        "open-collaboration reviews at the one boundary that reaches everybody"
    );
}

/// The invariant floor, stated as the AC states it: **restricted asset
/// requires compliance + dual approval** — under every pack, at every
/// scope kind, for every asset type.
#[test]
fn restricted_always_requires_compliance_and_dual_approval() {
    let mut matrices: Vec<(String, ApprovalMatrix)> = PACKS
        .iter()
        .map(|pack| ((*pack).to_owned(), approvals::embedded(pack)))
        .collect();
    matrices.push(("empty".to_owned(), ApprovalMatrix::empty()));
    // A pack written specifically to author the floor away: it names the
    // restricted cell and asks for nothing at all.
    matrices.push((
        "hostile".to_owned(),
        ApprovalMatrix {
            rules: vec![ApprovalRule {
                asset: None,
                min_sensitivity: Sensitivity::Public,
                scope_kinds: None,
                roles: Vec::new(),
                distinct_approvers: 0,
                forbid_author_approval: false,
                separate_effect_actor: false,
            }],
        },
    ));

    for (name, matrix) in &matrices {
        for asset in AssetKind::ALL {
            for kind in SCOPE_KINDS {
                let requirement = matrix.resolve(asset, Sensitivity::Restricted, kind);
                // Since CPR-7 merged the vocabularies, a pack cell may ask
                // for *more* administrators than the floor does; the merged
                // count is a maximum, so "at least one administrator" is
                // the floor's claim and the assertion's.
                assert!(
                    requirement
                        .roles
                        .iter()
                        .any(|required| required.role == RoleKey::Administrator
                            && required.count >= 1),
                    "{name}: restricted {asset} at {kind:?} escaped compliance review"
                );
                assert!(
                    requirement.distinct_approvers >= 2,
                    "{name}: restricted {asset} at {kind:?} needs only \
                     {} approver(s) — dual approval is the floor",
                    requirement.distinct_approvers
                );
                assert!(
                    requirement.origins.contains(&RequirementOrigin::Floor),
                    "{name}: the floor did not contribute to restricted {asset}"
                );

                // And the arithmetic that makes "dual" mean two people:
                // one person holding both roles satisfies both role lines
                // and still leaves the requirement unmet.
                let both = [CastApproval {
                    identity: IdentityId::new(),
                    subject: "one-person".to_owned(),
                    roles: RoleKey::ALL.to_vec(),
                }];
                assert!(
                    !requirement.satisfied_by(&both),
                    "{name}: one principal holding every role published a \
                     restricted {asset} alone"
                );
            }
        }
    }
}

/// The floor's executable-boundary rule: neither a Skill nor a Tool version
/// reaches application without a reviewer — at any sensitivity or pack.
#[test]
fn every_executable_boundary_needs_a_security_reviewer_under_every_pack() {
    for pack in PACKS.iter().copied().chain(["someones-custom-pack"]) {
        let matrix = approvals::embedded(pack);
        for asset in [AssetKind::Skill, AssetKind::Tool] {
            for sensitivity in Sensitivity::ALL {
                for kind in SCOPE_KINDS {
                    let requirement = matrix.resolve(asset, sensitivity, kind);
                    assert!(
                        requirement
                            .roles
                            .contains(&RoleRequirement::new(RoleKey::Reviewer, 1)),
                        "{pack}: a {sensitivity} {asset} at {kind:?} reached application \
                         without a security reviewer"
                    );
                }
            }
        }
    }
}

/// The wiring: a stored pack's matrix rides its `EffectivePack`, so what
/// the publish path resolves comes from exactly the pack that governs the
/// target scope — and a stored pack that configures nothing still carries
/// the floor.
#[test]
fn a_stored_packs_matrix_rides_its_effective_pack() {
    let pdp = Pdp::new().expect("build pdp");
    let tenant = TenantId::new();
    let matrix = ApprovalMatrix {
        rules: vec![ApprovalRule {
            asset: Some(AssetKind::Knowledge),
            min_sensitivity: Sensitivity::Public,
            scope_kinds: None,
            roles: vec![RoleRequirement::new(RoleKey::Administrator, 2)],
            distinct_approvers: 2,
            forbid_author_approval: false,
            separate_effect_actor: false,
        }],
    };
    pdp.install_source(
        tenant,
        "acme-two-stewards",
        1,
        "permit (principal, action, resource) when { resource in principal.tenant };",
        PackConfig {
            approvals: Some(matrix.clone()),
            ..Default::default()
        },
    )
    .expect("install a pack carrying a matrix");

    let effective = pdp.effective(
        tenant,
        synveda_policy::Resource::Tenant(tenant),
        &synveda_policy::AuthzContext {
            sensitivity: Some(Sensitivity::Internal),
            default_pack: Some("acme-two-stewards"),
            ..Default::default()
        },
    );
    assert_eq!(effective.name, "acme-two-stewards");
    assert_eq!(*effective.approvals, matrix, "the pack carries its matrix");
    assert_eq!(
        effective
            .approvals
            .resolve(
                AssetKind::Knowledge,
                Sensitivity::Internal,
                ScopeKind::OrgUnit
            )
            .roles,
        vec![RoleRequirement::new(RoleKey::Administrator, 2)]
    );

    // The unconfigured case: the floor, and nothing else.
    pdp.install_source(
        tenant,
        "acme-plain",
        1,
        "permit (principal, action, resource) when { resource in principal.tenant };",
        PackConfig::default(),
    )
    .expect("install an unconfigured pack");
    let plain = pdp.effective(
        tenant,
        synveda_policy::Resource::Tenant(tenant),
        &synveda_policy::AuthzContext {
            sensitivity: Some(Sensitivity::Internal),
            default_pack: Some("acme-plain"),
            ..Default::default()
        },
    );
    assert!(plain.approvals.rules.is_empty());
    assert_eq!(
        plain
            .approvals
            .resolve(
                AssetKind::Knowledge,
                Sensitivity::Restricted,
                ScopeKind::OrgUnit
            )
            .distinct_approvers,
        2,
        "an unconfigured pack still carries the floor, never 'no review'"
    );
}

/// A matrix that cannot be satisfied is refused at install time, not
/// discovered at review time: `curator × 2` with one distinct approver
/// would deny every proposal at the cells it governs, silently.
#[test]
fn an_unsatisfiable_matrix_is_refused_at_install_time() {
    let pdp = Pdp::new().expect("build pdp");
    let tenant = TenantId::new();
    let refused = pdp.install_source(
        tenant,
        "acme-impossible",
        1,
        "permit (principal, action, resource) when { resource in principal.tenant };",
        PackConfig {
            approvals: Some(ApprovalMatrix {
                rules: vec![ApprovalRule {
                    asset: None,
                    min_sensitivity: Sensitivity::Public,
                    scope_kinds: None,
                    roles: vec![RoleRequirement::new(RoleKey::Curator, 2)],
                    distinct_approvers: 1,
                    forbid_author_approval: false,
                    separate_effect_actor: false,
                }],
            }),
            ..Default::default()
        },
    );
    assert!(
        matches!(refused, Err(synveda_types::Error::Invalid { .. })),
        "an unsatisfiable matrix must be Invalid at install, got {refused:?}"
    );
}
