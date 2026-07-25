//! The embedded packs' approval matrices (tech plan §2.4; FLOW-3,
//! ADR-0032 decisions 3 and 4).
//!
//! Cedar decides *who may act*; these decide *how many acts are needed*.
//! They are compiled in beside the redaction and composition configs, for
//! the same reason those are: the answer has to come from exactly the
//! pack that governs the target scope, and a product pack means the same
//! thing in every tenant (ADR-0014 decision 6).
//!
//! Every resolution merges [`ApprovalMatrix::floor`] first, so the two
//! rules that make `restricted` and `skill` non-negotiable hold whatever
//! a pack — embedded or stored — says. The matrices here sit *above* the
//! floor; none of them can lower it.
//!
//! The shape follows tech plan §2.4's table directly:
//!
//! | | `regulated-strict` | `standard` | `open-collaboration` |
//! |---|---|---|---|
//! | memory → team/user | 1 × curator | — (auto) | — (auto) |
//! | memory → dept/org | curator + steward, 2 distinct | 1 × curator | 1 × curator at org |
//! | prompt | steward + curator, 2 distinct | 1 × curator | 1 × curator |
//! | skill | steward, 2 distinct (+ floor's reviewer) | 1 × steward (+ floor) | 1 × steward (+ floor) |
//! | policy | 2 × steward | 1 × steward | 1 × steward |
//! | anything `restricted` | the floor: compliance, 2 distinct | same | same |

use synveda_types::{
    ApprovalMatrix, ApprovalRule, AssetKind, Role, RoleRequirement, ScopeKind, Sensitivity,
};

/// Scope kinds at or below a team — where a scope's own people work.
const LOCAL: [ScopeKind; 2] = [ScopeKind::Team, ScopeKind::User];

/// Scope kinds above a team — where publishing reaches people who were
/// not in the room.
const SHARED: [ScopeKind; 3] = [ScopeKind::Org, ScopeKind::Division, ScopeKind::Department];

/// `regulated-strict`: every publication is reviewed, and reaching past
/// a team takes two people.
#[must_use]
pub fn regulated_strict() -> ApprovalMatrix {
    ApprovalMatrix {
        rules: vec![
            rule(
                Some(AssetKind::Memory),
                Some(LOCAL.to_vec()),
                &[(Role::Curator, 1)],
                1,
            ),
            rule(
                Some(AssetKind::Memory),
                Some(SHARED.to_vec()),
                &[(Role::Curator, 1), (Role::Steward, 1)],
                2,
            ),
            // Tech plan §2.4: "1 × dept steward + 1 × any curator (peer
            // review)". Peer review is the point, so two distinct.
            rule(
                Some(AssetKind::Prompt),
                None,
                &[(Role::Steward, 1), (Role::Curator, 1)],
                2,
            ),
            rule(Some(AssetKind::ContextPack), None, &[(Role::Curator, 1)], 1),
            // The floor already requires a security-reviewer; this adds
            // the steward tech plan §2.4 names and forces them to be two
            // different people.
            rule(Some(AssetKind::Skill), None, &[(Role::Steward, 1)], 2),
            // "Policy lapse under regulated-strict: 2 × steward at target
            // scope". Policy governs everything else, so it is the one
            // asset whose review needs two holders of the same role.
            rule(Some(AssetKind::Policy), None, &[(Role::Steward, 2)], 2),
        ],
    }
}

/// `standard`: the SMB collapse tech plan §2.4 names — "most of the above
/// collapses to single-approver or auto-approve".
///
/// Auto-approve is a real answer, not a hole: `ChannelPublish` still
/// requires a curator or above in every pack, so "nothing required" means
/// a curator may publish without a second look — never that anyone may.
#[must_use]
pub fn standard() -> ApprovalMatrix {
    ApprovalMatrix {
        rules: vec![
            rule(
                Some(AssetKind::Memory),
                Some(SHARED.to_vec()),
                &[(Role::Curator, 1)],
                1,
            ),
            rule(Some(AssetKind::Prompt), None, &[(Role::Curator, 1)], 1),
            rule(Some(AssetKind::ContextPack), None, &[(Role::Curator, 1)], 1),
            rule(Some(AssetKind::Skill), None, &[(Role::Steward, 1)], 1),
            rule(Some(AssetKind::Policy), None, &[(Role::Steward, 1)], 1),
        ],
    }
}

/// `open-collaboration`: share by default, review at the org boundary.
///
/// The only memory publication that takes a review is one onto the org's
/// own channel — the one place a publication reaches everybody.
#[must_use]
pub fn open_collaboration() -> ApprovalMatrix {
    ApprovalMatrix {
        rules: vec![
            rule(
                Some(AssetKind::Memory),
                Some(vec![ScopeKind::Org]),
                &[(Role::Curator, 1)],
                1,
            ),
            rule(Some(AssetKind::Prompt), None, &[(Role::Curator, 1)], 1),
            rule(Some(AssetKind::ContextPack), None, &[(Role::Curator, 1)], 1),
            rule(Some(AssetKind::Skill), None, &[(Role::Steward, 1)], 1),
            rule(Some(AssetKind::Policy), None, &[(Role::Steward, 1)], 1),
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
    roles: &[(Role, u8)],
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

    /// Every scope kind is governed by exactly one memory rule per pack,
    /// so no cell falls through to "auto-approve" by accident rather than
    /// by decision.
    #[test]
    fn memory_rules_partition_the_scope_kinds() {
        for kind in [
            ScopeKind::Org,
            ScopeKind::Division,
            ScopeKind::Department,
            ScopeKind::Team,
            ScopeKind::User,
        ] {
            let matching = regulated_strict()
                .rules
                .iter()
                .filter(|rule| rule.matches(AssetKind::Memory, Sensitivity::Internal, kind))
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
                .resolve(AssetKind::Memory, Sensitivity::Restricted, ScopeKind::Team)
                .is_empty(),
            "the floor is not a pack's to opt out of"
        );
    }
}
