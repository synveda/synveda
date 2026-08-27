//! The embedded packs' approval matrices (tech plan §2.4; FLOW-3,
//! ADR-0032 decisions 3 and 4).
//!
//! Cedar decides *who may act*; these decide *how many acts are needed*.
//! They are compiled in beside the redaction and composition configs, for
//! the same reason those are: the answer has to come from exactly the
//! pack that governs the target scope, and a product pack means the same
//! thing in every tenant (ADR-0014 decision 6).
//!
//! Every resolution merges [`ApprovalMatrix::floor`] first, so the rules
//! that make `restricted`, `skill` and `tool` non-negotiable hold whatever
//! a pack — embedded or stored — says. The matrices here sit *above* the
//! floor; none of them can lower it.
//!
//! The shape follows tech plan §2.4's table directly:
//!
//! | | `regulated-strict` | `standard` | `open-collaboration` |
//! |---|---|---|---|
//! | knowledge → workspace/project/own | 1 × curator | — (auto) | — (auto) |
//! | knowledge → tenant root / org unit | curator + administrator, 2 distinct | 1 × curator | 1 × curator at the tenant root |
//! | prompt | administrator + curator, 2 distinct | 1 × curator | 1 × curator |
//! | context pack → workspace/project/own | 1 × curator | 1 × curator | 1 × curator |
//! | context pack → tenant root / org unit | curator + administrator, 2 distinct | 1 × curator | 1 × curator |
//! | skill | administrator, 2 distinct (+ floor's reviewer) | 1 × administrator (+ floor) | 1 × administrator (+ floor) |
//! | tool | administrator, 2 distinct (+ floor's reviewer) | 1 × administrator (+ floor) | 1 × administrator (+ floor) |
//! | policy | 2 × administrator | 1 × administrator | — (auto) |
//! | anything `restricted` | the floor: administrator, 2 distinct | same | same |
//!
//! The role names are grant keys since CPR-7 (ADR-0074 decision 6):
//! `steward` became `administrator` and the floor's `compliance` and
//! `security-reviewer` became `administrator` and `reviewer`. The
//! scope-kind columns are the five shapes: §2.4's "team/user" cell is a
//! person's own scope, a workspace or a project, and its "dept/org" cell is
//! an org unit **or the tenant root**. The root carries the old `org` row
//! rather than a row of its own, because it is the widest audience this
//! product has — every member's own chain runs through it — and a shape no
//! rule named would fall through to auto-approve, which would make the
//! widest publication in the tenant the cheapest one.
//!
//! The context-pack rows are **not** in tech plan §2.4's table — it has no
//! row for them at all. FLOW-3 filled the cell at one curator everywhere,
//! nothing could open a `context-pack` proposal until PRMT-2, and reading
//! the matrix this feature makes resolvable turned up what that had done:
//! under `regulated-strict` a Knowledge item published at an org unit took two
//! distinct people while a whole *bundle* published at the org took one, so
//! the cheapest thing to publish into every session in the company was the
//! largest one. ADR-0050 decision 15 re-prices it to match memory's own
//! shared-scope rule, before any tenant has published under it, and
//! deliberately leaves `standard` and `open-collaboration` alone — the
//! whole content of those packs is that the same publication is cheaper.

use synveda_types::access::RoleKey;
use synveda_types::scope::ScopeKind;
use synveda_types::{ApprovalMatrix, ApprovalRule, AssetKind, RoleRequirement, Sensitivity};

/// Scope shapes where a scope's own people work — a person's own scope, a
/// workspace, a project. Publishing here reaches people who were in the
/// room (CPR-7, ADR-0074: the old team/user rank pair, as shapes).
const LOCAL: [ScopeKind; 3] = [
    ScopeKind::Principal,
    ScopeKind::Workspace,
    ScopeKind::Project,
];

/// Scope shapes where publishing reaches people who were not in the room:
/// an org unit, and the tenant root above all of them (CPR-7 — the root is
/// what the old `org` rank became, and the partition test below is what
/// keeps it from falling through to auto-approve).
const SHARED: [ScopeKind; 2] = [ScopeKind::Tenant, ScopeKind::OrgUnit];

/// `regulated-strict`: every publication is reviewed, and reaching past
/// a team takes two people.
#[must_use]
pub fn regulated_strict() -> ApprovalMatrix {
    let mut matrix = ApprovalMatrix {
        rules: vec![
            rule(
                Some(AssetKind::Knowledge),
                Some(LOCAL.to_vec()),
                &[(RoleKey::Curator, 1)],
                1,
            ),
            rule(
                Some(AssetKind::Knowledge),
                Some(SHARED.to_vec()),
                &[(RoleKey::Curator, 1), (RoleKey::Administrator, 1)],
                2,
            ),
            // Tech plan §2.4: "1 × dept steward + 1 × any curator (peer
            // review)". Peer review is the point, so two distinct.
            rule(
                Some(AssetKind::Prompt),
                None,
                &[(RoleKey::Administrator, 1), (RoleKey::Curator, 1)],
                2,
            ),
            // The Knowledge `SHARED`/`LOCAL` split, given
            // to context packs by ADR-0050 decision 15. Its blast radius is
            // strictly wider than the Knowledge row above it: a published pack
            // composes into *every* session at and below the publishing
            // scope, so pricing it below a single Knowledge item at the same
            // scope was an inversion, not a discount.
            rule(
                Some(AssetKind::ContextPack),
                Some(LOCAL.to_vec()),
                &[(RoleKey::Curator, 1)],
                1,
            ),
            rule(
                Some(AssetKind::ContextPack),
                Some(SHARED.to_vec()),
                &[(RoleKey::Curator, 1), (RoleKey::Administrator, 1)],
                2,
            ),
            // The floor already requires a security-reviewer; this adds
            // the steward tech plan §2.4 names and forces them to be two
            // different people.
            rule(
                Some(AssetKind::Skill),
                None,
                &[(RoleKey::Administrator, 1)],
                2,
            ),
            rule(
                Some(AssetKind::Tool),
                None,
                &[(RoleKey::Administrator, 1)],
                2,
            ),
            // A Policy relaxation under regulated-strict needs two
            // administrators at the target scope. Policy governs everything
            // else, so it is the one
            // asset whose review needs two holders of the same role.
            rule(
                Some(AssetKind::Policy),
                None,
                &[(RoleKey::Administrator, 2)],
                2,
            ),
            rule(
                Some(AssetKind::Configuration),
                None,
                &[(RoleKey::Administrator, 2)],
                2,
            ),
        ],
    };
    for rule in &mut matrix.rules {
        rule.forbid_author_approval = true;
        rule.separate_effect_actor = true;
    }
    matrix
}

/// `standard`: the SMB collapse tech plan §2.4 names — "most of the above
/// collapses to single-approver or auto-approve".
///
/// Auto-approve is a real answer, not a hole: `ChannelPublish` still
/// requires a curator or above in every pack, so "nothing required" means
/// a curator may publish without a second look — never that anyone may.
#[must_use]
pub fn standard() -> ApprovalMatrix {
    let mut matrix = ApprovalMatrix {
        rules: vec![
            rule(
                Some(AssetKind::Knowledge),
                Some(SHARED.to_vec()),
                &[(RoleKey::Curator, 1)],
                1,
            ),
            rule(Some(AssetKind::Prompt), None, &[(RoleKey::Curator, 1)], 1),
            rule(
                Some(AssetKind::ContextPack),
                None,
                &[(RoleKey::Curator, 1)],
                1,
            ),
            rule(
                Some(AssetKind::Skill),
                None,
                &[(RoleKey::Administrator, 1)],
                1,
            ),
            rule(
                Some(AssetKind::Tool),
                None,
                &[(RoleKey::Administrator, 1)],
                1,
            ),
            rule(
                Some(AssetKind::Policy),
                None,
                &[(RoleKey::Administrator, 1)],
                1,
            ),
            rule(
                Some(AssetKind::Configuration),
                None,
                &[(RoleKey::Administrator, 1)],
                1,
            ),
        ],
    };
    for rule in &mut matrix.rules {
        rule.forbid_author_approval = true;
    }
    matrix
}

/// `open-collaboration`: share by default, review at the org boundary.
///
/// The only Knowledge publication that takes a review is one onto the tenant's
/// own channel — the one place a publication reaches everybody.
#[must_use]
pub fn open_collaboration() -> ApprovalMatrix {
    ApprovalMatrix {
        rules: vec![
            rule(
                Some(AssetKind::Knowledge),
                Some(vec![ScopeKind::Tenant]),
                &[(RoleKey::Curator, 1)],
                1,
            ),
            rule(Some(AssetKind::Prompt), None, &[(RoleKey::Curator, 1)], 1),
            rule(
                Some(AssetKind::ContextPack),
                None,
                &[(RoleKey::Curator, 1)],
                1,
            ),
            rule(
                Some(AssetKind::Skill),
                None,
                &[(RoleKey::Administrator, 1)],
                1,
            ),
            rule(
                Some(AssetKind::Tool),
                None,
                &[(RoleKey::Administrator, 1)],
                1,
            ),
            rule(
                Some(AssetKind::Configuration),
                Some(SHARED.to_vec()),
                &[(RoleKey::Administrator, 1)],
                1,
            ),
        ],
    }
}

/// The matrix compiled into `name`, or the empty matrix (floor only) for
/// a name that is not an embedded pack.
#[must_use]
pub fn embedded(name: &str) -> ApprovalMatrix {
    match name {
        crate::pdp::REGULATED_STRICT => regulated_strict(),
        crate::pdp::STANDARD => standard(),
        crate::pdp::OPEN_COLLABORATION => open_collaboration(),
        _ => ApprovalMatrix::empty(),
    }
}

fn rule(
    asset: Option<AssetKind>,
    scope_kinds: Option<Vec<ScopeKind>>,
    roles: &[(RoleKey, u8)],
    distinct_approvers: u8,
) -> ApprovalRule {
    ApprovalRule {
        asset,
        min_sensitivity: Sensitivity::Public,
        scope_kinds,
        roles: roles
            .iter()
            .map(|(role, count)| RoleRequirement::new(*role, *count))
            .collect(),
        distinct_approvers,
        forbid_author_approval: false,
        separate_effect_actor: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property that would break silently: a matrix asking for more
    /// of a role than it asks of people is unsatisfiable at every cell it
    /// governs, and it would fail with no error anywhere.
    #[test]
    fn every_embedded_matrix_is_satisfiable() {
        for (name, matrix) in [
            (crate::pdp::REGULATED_STRICT, regulated_strict()),
            (crate::pdp::STANDARD, standard()),
            (crate::pdp::OPEN_COLLABORATION, open_collaboration()),
        ] {
            matrix
                .validate()
                .unwrap_or_else(|err| panic!("{name} matrix is unsatisfiable: {err}"));
        }
    }

    /// Every scope kind is governed by exactly one Knowledge rule in the
    /// strict pack,
    /// so no cell falls through to "auto-approve" by accident rather than
    /// by decision.
    #[test]
    fn knowledge_rules_partition_the_scope_kinds() {
        for kind in [
            ScopeKind::Tenant,
            ScopeKind::OrgUnit,
            ScopeKind::Workspace,
            ScopeKind::Project,
            ScopeKind::Principal,
        ] {
            let matching = regulated_strict()
                .rules
                .iter()
                .filter(|rule| rule.matches(AssetKind::Knowledge, Sensitivity::Internal, kind))
                .count();
            assert_eq!(matching, 1, "regulated-strict leaves {kind:?} to chance");
        }
    }

    #[test]
    fn an_unknown_pack_name_still_gets_the_floor() {
        let matrix = embedded("someones-custom-pack");
        assert!(matrix.rules.is_empty());
        assert!(
            !matrix
                .resolve(
                    AssetKind::Knowledge,
                    Sensitivity::Restricted,
                    ScopeKind::OrgUnit
                )
                .is_empty(),
            "the floor is not a pack's to opt out of"
        );
    }
}
