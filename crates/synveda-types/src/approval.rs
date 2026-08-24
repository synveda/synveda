//! The approval matrix (tech plan §2.4, FLOW-3, ADR-0032).
//!
//! Required approvals resolve from **asset type × maximum sensitivity ×
//! target scope kind × effective pack**. The matrix is pack configuration
//! — it rides the loaded pack beside [`crate::RedactionConfig`] and
//! [`crate::CompositionConfig`] — with one difference that matters: an
//! invariant product floor is merged into *every* resolution, embedded
//! pack or stored, so no configuration can author away compliance review
//! of `restricted` material or security review of executable skills.
//! That is the `base.cedar` pattern (ADR-0014 decision 2) applied to
//! configuration.
//!
//! # What this module decides and what it does not
//!
//! It counts. [`ApprovalRequirement::outstanding`] says what a proposal
//! still needs; it never says who may approve. Whether a principal may
//! cast an approval at all is a Cedar decision (`ProposalReview`) taken
//! before anything here runs (ADR-0032 decision 5). Two layers, kept
//! apart on purpose: the PDP cannot see stored approvals, and this
//! module must never be given authority.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::access::RoleKey;
use crate::scope::ScopeKind;
use crate::{AssetKind, IdentityId, ScopeId, Sensitivity};

/// How many distinct approvers holding one role a rule asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleRequirement {
    /// The role an approver must hold at the target scope.
    pub role: RoleKey,
    /// How many distinct approvers must hold it. Zero is meaningless and
    /// is rejected when a matrix is validated.
    pub count: u8,
}

impl RoleRequirement {
    /// `count` approvers holding `role`.
    #[must_use]
    pub const fn new(role: RoleKey, count: u8) -> Self {
        RoleRequirement { role, count }
    }
}

/// One rule of an approval matrix.
///
/// A rule matches when its asset kind (or any), its minimum sensitivity,
/// and its scope kinds (or any) all match the request. Matching rules
/// combine by taking the **maximum** count per role and the maximum
/// distinct-approver count — never a sum, so the same requirement stated
/// twice in two forms does not silently double (ADR-0032 decision 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalRule {
    /// Which asset kind this rule governs; `None` (absent) is any kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<AssetKind>,
    /// The rule applies at this sensitivity **and above**, so a rule
    /// written for `public` governs everything.
    #[serde(default = "lowest_sensitivity")]
    pub min_sensitivity: Sensitivity,
    /// Which target scope kinds this rule governs; `None` (absent) is any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_kinds: Option<Vec<ScopeKind>>,
    /// The roles approvers must hold, and how many of each.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<RoleRequirement>,
    /// How many distinct identities must have approved. This is what
    /// "dual approval" means, and it is the only thing that forbids
    /// unilateral action (ADR-0032 decision 7).
    #[serde(default)]
    pub distinct_approvers: u8,
}

fn lowest_sensitivity() -> Sensitivity {
    Sensitivity::Public
}

impl ApprovalRule {
    /// Whether this rule governs `(asset, sensitivity, scope_kind)`.
    #[must_use]
    pub fn matches(
        &self,
        asset: AssetKind,
        sensitivity: Sensitivity,
        scope_kind: ScopeKind,
    ) -> bool {
        self.asset.is_none_or(|kind| kind == asset)
            && sensitivity >= self.min_sensitivity
            && self
                .scope_kinds
                .as_ref()
                .is_none_or(|kinds| kinds.contains(&scope_kind))
    }
}

/// Where a contribution to a resolved requirement came from — recorded
/// on every audit event so a trail explains why a proposal needed what it
/// needed, without reading a pack that has since changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "origin")]
pub enum RequirementOrigin {
    /// The invariant product floor — merged into every matrix, not
    /// authorable away (ADR-0032 decision 4).
    Floor,
    /// The effective pack's own rules.
    Pack,
    /// The nearest-ancestor curator file, at this scope (ADR-0032
    /// decisions 13 and 14).
    Curators {
        /// The scope whose curator file contributed.
        scope_id: ScopeId,
    },
}

impl RequirementOrigin {
    /// The name an audit payload and an API response both use.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            RequirementOrigin::Floor => "floor".to_owned(),
            RequirementOrigin::Pack => "pack".to_owned(),
            RequirementOrigin::Curators { scope_id } => format!("curators:{scope_id}"),
        }
    }
}

/// One requirement as every audit payload and API response renders it
/// ([`ApprovalRequirement::audit_view`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequirementAudit {
    /// What was asked for.
    pub required: RequiredAudit,
    /// A one-line rendering of what is still missing.
    pub outstanding: String,
    /// Whether nothing is missing.
    pub satisfied: bool,
}

/// The asked-for half of [`RequirementAudit`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequiredAudit {
    /// Roles required, with counts.
    pub roles: Vec<RoleAudit>,
    /// Distinct identities required.
    pub distinct_approvers: u8,
    /// Named subjects a curator file requires.
    pub subjects: Vec<String>,
    /// Where each part came from: `floor`, `pack`, or `curators:{scope}`.
    pub origins: Vec<String>,
}

/// One role line of a [`RequiredAudit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RoleAudit {
    /// The role's wire name.
    pub role: &'static str,
    /// How many approvers holding it are required.
    pub count: u8,
}

/// A pack's approval matrix: an ordered list of rules.
///
/// Stored in `policy_packs.approvals` for custom packs and compiled in
/// for the embedded ones, exactly like the redaction and composition
/// configs. `Default` is the empty matrix, which still resolves to the
/// floor — a pack that configures nothing gets the product's
/// non-negotiables and nothing more.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalMatrix {
    /// The pack's own rules, merged on top of the floor.
    #[serde(default)]
    pub rules: Vec<ApprovalRule>,
}

/// The invariant product floor (ADR-0032 decision 4).
///
/// Three rules, prepended to every matrix — embedded pack, stored pack, or
/// no configuration at all:
///
/// - anything `restricted` needs an `administrator` and two distinct
///   approvers (tech plan §2.4; seed §4.2's own definition of the tier);
/// - any `skill` or trusted-MCP `tool` version needs a `reviewer` **and two
///   distinct approvers**, because executable instructions and capability
///   declarations both cross a trust boundary.
///
/// The two named roles are the grant-key re-vocabulary of `compliance` and
/// `security-reviewer` (CPR-7, ADR-0074 decision 6): the floors are
/// unchanged in substance — restricted material needs the administrator's
/// sign-off, executable content a reviewer's — and Prompt 27 re-cuts the
/// matrix over artifact versions.
///
/// The second rule's approver count is ADR-0051 decision 18, and it is a
/// correction rather than an addition. FLOW-3 wrote it at one, so under
/// `standard` and `open-collaboration` — whose own skill rule also asks for
/// one — the resolved requirement was a single signature, and one person
/// holding both `steward` and `security-reviewer` published executable code
/// alone. Separating those two roles has no other content than that they
/// are two people. It is on the floor rather than in a pack because the
/// floor is where "not a pack's to opt out of" lives, and it was taken in
/// SKIL-1 because that is the feature that made the cell reachable at all —
/// nothing could open a `skill` proposal before it.
static FLOOR: LazyLock<Vec<ApprovalRule>> = LazyLock::new(|| {
    vec![
        ApprovalRule {
            asset: None,
            min_sensitivity: Sensitivity::Restricted,
            scope_kinds: None,
            roles: vec![RoleRequirement::new(RoleKey::Administrator, 1)],
            distinct_approvers: 2,
        },
        ApprovalRule {
            asset: Some(AssetKind::Skill),
            min_sensitivity: Sensitivity::Public,
            scope_kinds: None,
            roles: vec![RoleRequirement::new(RoleKey::Reviewer, 1)],
            distinct_approvers: 2,
        },
        ApprovalRule {
            asset: Some(AssetKind::Tool),
            min_sensitivity: Sensitivity::Public,
            scope_kinds: None,
            roles: vec![RoleRequirement::new(RoleKey::Reviewer, 1)],
            distinct_approvers: 2,
        },
    ]
});

impl ApprovalMatrix {
    /// The empty matrix — the floor and nothing else.
    #[must_use]
    pub fn empty() -> Self {
        ApprovalMatrix::default()
    }

    /// The invariant floor, for the test that pins its content and for
    /// the admin surface that displays it.
    #[must_use]
    pub fn floor() -> &'static [ApprovalRule] {
        &FLOOR
    }

    /// What it takes to publish `asset` at `sensitivity` onto a
    /// `scope_kind` scope's channel under this pack.
    ///
    /// **The floor is always merged.** There is deliberately no way to
    /// resolve a matrix without it: an API that could be called the wrong
    /// way eventually is (ADR-0032 decision 4).
    #[must_use]
    pub fn resolve(
        &self,
        asset: AssetKind,
        sensitivity: Sensitivity,
        scope_kind: ScopeKind,
    ) -> ApprovalRequirement {
        let mut requirement = ApprovalRequirement::default();
        for (origin, rules) in [
            (RequirementOrigin::Floor, ApprovalMatrix::floor()),
            (RequirementOrigin::Pack, self.rules.as_slice()),
        ] {
            for rule in rules {
                if rule.matches(asset, sensitivity, scope_kind) {
                    requirement.absorb(origin, rule);
                }
            }
        }
        requirement
    }

    /// Rejects a matrix that cannot mean anything: a role requirement of
    /// zero, or a rule asking for more of one role than it asks for
    /// distinct approvers in total.
    ///
    /// The second is not pedantry — `[curator × 2, distinct 1]` is
    /// unsatisfiable, and a stored pack that says it would make every
    /// proposal at that cell impossible with no error anywhere.
    ///
    /// # Errors
    ///
    /// [`crate::Error::Invalid`] naming the offending rule.
    pub fn validate(&self) -> crate::Result<()> {
        for (index, rule) in self.rules.iter().enumerate() {
            let mut seen = BTreeSet::new();
            for requirement in &rule.roles {
                if requirement.count == 0 {
                    return Err(crate::Error::Invalid {
                        message: format!(
                            "approval rule {index}: {} requires 0 approvers, which asks nothing",
                            requirement.role
                        ),
                    });
                }
                if !seen.insert(requirement.role) {
                    return Err(crate::Error::Invalid {
                        message: format!(
                            "approval rule {index}: {} appears twice; state one count",
                            requirement.role
                        ),
                    });
                }
                if u16::from(requirement.count) > u16::from(rule.distinct_approvers) {
                    return Err(crate::Error::Invalid {
                        message: format!(
                            "approval rule {index}: {} × {} needs at least that many distinct \
                             approvers, but the rule asks for {}",
                            requirement.role, requirement.count, rule.distinct_approvers
                        ),
                    });
                }
            }
        }
        Ok(())
    }
}

/// What a specific `(asset, sensitivity, scope kind)` needs, resolved.
///
/// Empty means auto-approve: the pack asks for nothing here and the floor
/// does not apply, so a principal who passes `ChannelPublish` may publish
/// without a second look. That is the tech plan §2.4 SMB collapse, and it
/// is a decision a pack makes explicitly rather than a hole.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ApprovalRequirement {
    /// Roles required, sorted, with the maximum count asked by any
    /// matching rule.
    pub roles: Vec<RoleRequirement>,
    /// Distinct identities required.
    pub distinct_approvers: u8,
    /// Named token subjects that must each have approved — the curator
    /// file's contribution (ADR-0032 decision 13). Sorted.
    pub subjects: Vec<String>,
    /// Which sources contributed, for the audit event.
    pub origins: Vec<RequirementOrigin>,
}

impl ApprovalRequirement {
    /// Whether nothing at all is required.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.roles.is_empty() && self.subjects.is_empty() && self.distinct_approvers == 0
    }

    /// The shape every act against this requirement records: what was
    /// required, where each part came from, and what is still
    /// outstanding afterwards.
    ///
    /// A `Serialize` view rather than rendered JSON, because this crate
    /// is the root of the graph and stays format-agnostic — each caller
    /// serialises with its own `serde_json`. The *shape* lives here
    /// because ADR-0032's compliance note promises the requirement is
    /// recorded as resolved at every act, and since FLOW-4 more than one
    /// kind of actor writes those events. Two renderings of one
    /// requirement would eventually differ, and an auditor would have to
    /// know which surface wrote which.
    #[must_use]
    pub fn audit_view(&self, outstanding: &Outstanding) -> RequirementAudit {
        RequirementAudit {
            required: RequiredAudit {
                roles: self
                    .roles
                    .iter()
                    .map(|required| RoleAudit {
                        role: required.role.as_str(),
                        count: required.count,
                    })
                    .collect(),
                distinct_approvers: self.distinct_approvers,
                subjects: self.subjects.clone(),
                origins: self.origins.iter().map(RequirementOrigin::label).collect(),
            },
            outstanding: outstanding.describe(),
            satisfied: outstanding.is_empty(),
        }
    }

    /// A one-line rendering of what this requirement asks for, for an
    /// error or a summary a reviewer reads.
    #[must_use]
    pub fn describe(&self) -> String {
        if self.is_empty() {
            return "no approvals".to_owned();
        }
        let mut parts: Vec<String> = self
            .roles
            .iter()
            .map(|role| format!("{} × {}", role.role, role.count))
            .collect();
        parts.extend(self.subjects.iter().map(|subject| format!("@{subject}")));
        if self.distinct_approvers > 1 {
            parts.push(format!("{} distinct approvers", self.distinct_approvers));
        }
        parts.join(", ")
    }

    /// Merges one matching rule in, taking maxima.
    fn absorb(&mut self, origin: RequirementOrigin, rule: &ApprovalRule) {
        if rule.roles.is_empty() && rule.distinct_approvers == 0 {
            return;
        }
        for required in &rule.roles {
            match self
                .roles
                .iter_mut()
                .find(|existing| existing.role == required.role)
            {
                Some(existing) => existing.count = existing.count.max(required.count),
                None => self.roles.push(*required),
            }
        }
        self.distinct_approvers = self.distinct_approvers.max(rule.distinct_approvers);
        self.roles.sort_unstable();
        self.note(origin);
    }

    /// Adds a named subject requirement — the curator file's seam. Also
    /// raises the distinct-approver floor to the number of named
    /// subjects, since each must approve and each is one identity.
    pub fn require_subject(&mut self, origin: RequirementOrigin, subject: &str) {
        if !self.subjects.iter().any(|existing| existing == subject) {
            self.subjects.push(subject.to_owned());
            self.subjects.sort();
        }
        let named = u8::try_from(self.subjects.len()).unwrap_or(u8::MAX);
        self.distinct_approvers = self.distinct_approvers.max(named);
        self.note(origin);
    }

    /// Adds a role requirement from a source other than a matrix rule —
    /// the curator file's `role:` form.
    pub fn require_role(&mut self, origin: RequirementOrigin, role: RoleKey) {
        match self.roles.iter_mut().find(|existing| existing.role == role) {
            Some(existing) => existing.count = existing.count.max(1),
            None => self.roles.push(RoleRequirement::new(role, 1)),
        }
        self.distinct_approvers = self.distinct_approvers.max(1);
        self.roles.sort_unstable();
        self.note(origin);
    }

    fn note(&mut self, origin: RequirementOrigin) {
        if !self.origins.contains(&origin) {
            self.origins.push(origin);
        }
    }

    /// What `cast` still leaves outstanding.
    ///
    /// RoleKey counts are satisfied by distinct approvers *holding* the
    /// role, so one person holding two required roles satisfies both role
    /// lines and still counts as one identity — which is exactly why
    /// `distinct_approvers` is a separate number (ADR-0032 decision 7).
    #[must_use]
    pub fn outstanding(&self, cast: &[CastApproval]) -> Outstanding {
        let distinct: BTreeSet<IdentityId> =
            cast.iter().map(|approval| approval.identity).collect();
        let roles = self
            .roles
            .iter()
            .filter_map(|required| {
                let held = cast
                    .iter()
                    .filter(|approval| approval.roles.contains(&required.role))
                    .map(|approval| approval.identity)
                    .collect::<BTreeSet<_>>()
                    .len();
                let short = u16::from(required.count)
                    .saturating_sub(u16::try_from(held).unwrap_or(u16::MAX));
                (short > 0).then(|| {
                    RoleRequirement::new(required.role, u8::try_from(short).unwrap_or(u8::MAX))
                })
            })
            .collect();
        let subjects = self
            .subjects
            .iter()
            .filter(|subject| {
                !cast
                    .iter()
                    .any(|approval| approval.subject.as_str() == subject.as_str())
            })
            .cloned()
            .collect();
        let short_of_distinct = u16::from(self.distinct_approvers)
            .saturating_sub(u16::try_from(distinct.len()).unwrap_or(u16::MAX));
        Outstanding {
            roles,
            distinct_approvers: u8::try_from(short_of_distinct).unwrap_or(u8::MAX),
            subjects,
        }
    }

    /// Whether `cast` satisfies this requirement outright.
    #[must_use]
    pub fn satisfied_by(&self, cast: &[CastApproval]) -> bool {
        self.outstanding(cast).is_empty()
    }
}

/// One recorded approval, as the matrix counts it: who cast it and which
/// roles they effectively held **at the target scope** when they did
/// (ADR-0032 decision 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CastApproval {
    /// The approving identity — the unit `distinct_approvers` counts.
    pub identity: IdentityId,
    /// The approver's token subject — what a curator file names.
    pub subject: String,
    /// The effective roles held at the target scope when cast.
    pub roles: Vec<RoleKey>,
}

/// What a requirement still lacks.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Outstanding {
    /// Roles still short, carrying how many more approvers are needed.
    pub roles: Vec<RoleRequirement>,
    /// How many more distinct identities must approve.
    pub distinct_approvers: u8,
    /// Named subjects that have not approved.
    pub subjects: Vec<String>,
}

impl Outstanding {
    /// Whether the requirement is met.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.roles.is_empty() && self.subjects.is_empty() && self.distinct_approvers == 0
    }

    /// Whether `candidate` would advance any outstanding line.
    ///
    /// An approval that advances nothing is refused rather than recorded
    /// (ADR-0032 decision 5): a vote that governs nothing is noise in a
    /// log a reviewer and an auditor both read. A candidate always has
    /// *some* review role — `ProposalReview` gated their entry — so the
    /// distinct-approver line is a real contribution, not a loophole.
    #[must_use]
    pub fn advanced_by(&self, candidate: &CastApproval) -> bool {
        if self.is_empty() {
            return false;
        }
        self.distinct_approvers > 0
            || self
                .roles
                .iter()
                .any(|required| candidate.roles.contains(&required.role))
            || self
                .subjects
                .iter()
                .any(|subject| subject.as_str() == candidate.subject.as_str())
    }

    /// A one-line rendering for an error message a reviewer reads.
    #[must_use]
    pub fn describe(&self) -> String {
        if self.is_empty() {
            return "nothing".to_owned();
        }
        let mut parts: Vec<String> = self
            .roles
            .iter()
            .map(|required| format!("{} × {}", required.role, required.count))
            .collect();
        parts.extend(self.subjects.iter().map(|subject| format!("@{subject}")));
        if self.distinct_approvers > 0 {
            parts.push(format!("{} distinct approver(s)", self.distinct_approvers));
        }
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn identity(byte: u8) -> IdentityId {
        IdentityId::from_uuid(Uuid::from_bytes([byte; 16]))
    }

    fn approval(byte: u8, roles: &[RoleKey]) -> CastApproval {
        CastApproval {
            identity: identity(byte),
            subject: format!("subject-{byte}"),
            roles: roles.to_vec(),
        }
    }

    fn memory_rule() -> ApprovalRule {
        ApprovalRule {
            asset: Some(AssetKind::Memory),
            min_sensitivity: Sensitivity::Public,
            scope_kinds: None,
            roles: vec![RoleRequirement::new(RoleKey::Curator, 1)],
            distinct_approvers: 1,
        }
    }

    /// The decision-4 property: no matrix, however authored, publishes
    /// restricted material without compliance and two distinct approvers.
    #[test]
    fn the_floor_survives_an_empty_matrix() {
        let requirement = ApprovalMatrix::empty().resolve(
            AssetKind::Memory,
            Sensitivity::Restricted,
            ScopeKind::OrgUnit,
        );
        assert_eq!(
            requirement.roles,
            vec![RoleRequirement::new(RoleKey::Administrator, 1)]
        );
        assert_eq!(requirement.distinct_approvers, 2);
        assert!(requirement.origins.contains(&RequirementOrigin::Floor));
    }

    /// A pack cannot lower the floor, only add above it: a pack rule
    /// asking for one approver leaves the floor's two in force.
    #[test]
    fn a_pack_rule_cannot_lower_the_floor() {
        let matrix = ApprovalMatrix {
            rules: vec![memory_rule()],
        };
        let requirement = matrix.resolve(
            AssetKind::Memory,
            Sensitivity::Restricted,
            ScopeKind::OrgUnit,
        );
        assert_eq!(
            requirement.distinct_approvers, 2,
            "the floor's, not the pack's"
        );
        assert_eq!(
            requirement.roles,
            vec![
                RoleRequirement::new(RoleKey::Curator, 1),
                RoleRequirement::new(RoleKey::Administrator, 1),
            ]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn below_restricted_the_floor_asks_nothing_of_a_memory() {
        let requirement = ApprovalMatrix::empty().resolve(
            AssetKind::Memory,
            Sensitivity::Confidential,
            ScopeKind::OrgUnit,
        );
        assert!(requirement.is_empty(), "auto-approve is a real answer");
    }

    /// Skills and Tool versions cross executable trust boundaries, so the
    /// floor asks for a reviewer at every sensitivity — including `public`.
    #[test]
    fn every_executable_boundary_needs_a_security_reviewer() {
        for asset in [AssetKind::Skill, AssetKind::Tool] {
            for sensitivity in Sensitivity::ALL {
                let requirement =
                    ApprovalMatrix::empty().resolve(asset, sensitivity, ScopeKind::Tenant);
                assert!(
                    requirement
                        .roles
                        .contains(&RoleRequirement::new(RoleKey::Reviewer, 1)),
                    "{sensitivity} {asset} escaped security review"
                );
            }
        }
    }

    #[test]
    fn rules_combine_by_maximum_never_by_sum() {
        let matrix = ApprovalMatrix {
            rules: vec![
                memory_rule(),
                ApprovalRule {
                    roles: vec![RoleRequirement::new(RoleKey::Curator, 2)],
                    distinct_approvers: 2,
                    ..memory_rule()
                },
            ],
        };
        let requirement =
            matrix.resolve(AssetKind::Memory, Sensitivity::Internal, ScopeKind::OrgUnit);
        assert_eq!(
            requirement.roles,
            vec![RoleRequirement::new(RoleKey::Curator, 2)],
            "two rules asking 1 and 2 ask for 2, not 3"
        );
        assert_eq!(requirement.distinct_approvers, 2);
    }

    #[test]
    fn scope_kinds_and_min_sensitivity_both_gate_a_rule() {
        let rule = ApprovalRule {
            asset: Some(AssetKind::Memory),
            min_sensitivity: Sensitivity::Confidential,
            scope_kinds: Some(vec![ScopeKind::OrgUnit, ScopeKind::Tenant]),
            roles: vec![RoleRequirement::new(RoleKey::Administrator, 1)],
            distinct_approvers: 1,
        };
        assert!(rule.matches(
            AssetKind::Memory,
            Sensitivity::Restricted,
            ScopeKind::Tenant
        ));
        assert!(!rule.matches(AssetKind::Memory, Sensitivity::Internal, ScopeKind::Tenant));
        assert!(!rule.matches(
            AssetKind::Memory,
            Sensitivity::Restricted,
            ScopeKind::Principal
        ));
        assert!(!rule.matches(
            AssetKind::Prompt,
            Sensitivity::Restricted,
            ScopeKind::Tenant
        ));
    }

    /// One person holding both required roles satisfies both role lines
    /// and still counts as one identity — which is what makes
    /// `distinct_approvers` the thing that forbids unilateral action.
    #[test]
    fn two_roles_in_one_person_do_not_make_two_approvers() {
        let requirement = ApprovalMatrix {
            rules: vec![memory_rule()],
        }
        .resolve(
            AssetKind::Memory,
            Sensitivity::Restricted,
            ScopeKind::OrgUnit,
        );
        let both = [approval(1, &[RoleKey::Curator, RoleKey::Administrator])];
        let outstanding = requirement.outstanding(&both);
        assert!(outstanding.roles.is_empty(), "both role lines are met");
        assert_eq!(outstanding.distinct_approvers, 1, "still one person");
        assert!(!requirement.satisfied_by(&both));

        let two = [
            approval(1, &[RoleKey::Curator, RoleKey::Administrator]),
            approval(2, &[RoleKey::Viewer]),
        ];
        assert!(requirement.satisfied_by(&two));
    }

    /// The proposer is not special-cased: what forbids acting alone is
    /// the distinct count, and it forbids it the same way everywhere.
    #[test]
    fn one_curator_satisfies_a_single_approver_requirement() {
        let requirement = ApprovalMatrix {
            rules: vec![memory_rule()],
        }
        .resolve(AssetKind::Memory, Sensitivity::Internal, ScopeKind::OrgUnit);
        assert!(requirement.satisfied_by(&[approval(1, &[RoleKey::Curator])]));
        assert!(!requirement.satisfied_by(&[approval(1, &[RoleKey::Viewer])]));
    }

    #[test]
    fn a_named_subject_must_approve_and_raises_the_distinct_floor() {
        let mut requirement = ApprovalMatrix::empty().resolve(
            AssetKind::Memory,
            Sensitivity::Internal,
            ScopeKind::OrgUnit,
        );
        let scope = ScopeId::from_uuid(Uuid::from_bytes([7; 16]));
        requirement.require_subject(RequirementOrigin::Curators { scope_id: scope }, "subject-3");
        requirement.require_subject(RequirementOrigin::Curators { scope_id: scope }, "subject-4");
        assert_eq!(requirement.distinct_approvers, 2);

        let one = [approval(3, &[RoleKey::Curator])];
        assert_eq!(requirement.outstanding(&one).subjects, vec!["subject-4"]);
        let both = [
            approval(3, &[RoleKey::Curator]),
            approval(4, &[RoleKey::Curator]),
        ];
        assert!(requirement.satisfied_by(&both));
    }

    #[test]
    fn an_approval_that_advances_nothing_is_not_a_contribution() {
        let requirement = ApprovalMatrix {
            rules: vec![memory_rule()],
        }
        .resolve(AssetKind::Memory, Sensitivity::Internal, ScopeKind::OrgUnit);
        let outstanding = requirement.outstanding(&[]);
        assert!(
            outstanding.advanced_by(&approval(1, &[RoleKey::Viewer])),
            "distinct line"
        );

        let satisfied = requirement.outstanding(&[approval(1, &[RoleKey::Curator])]);
        assert!(satisfied.is_empty());
        assert!(!satisfied.advanced_by(&approval(2, &[RoleKey::Curator])));
    }

    #[test]
    fn a_rule_asking_more_of_a_role_than_it_asks_of_people_is_rejected() {
        let matrix = ApprovalMatrix {
            rules: vec![ApprovalRule {
                roles: vec![RoleRequirement::new(RoleKey::Curator, 2)],
                distinct_approvers: 1,
                ..memory_rule()
            }],
        };
        assert!(matrix.validate().is_err(), "unsatisfiable at every cell");

        let zero = ApprovalMatrix {
            rules: vec![ApprovalRule {
                roles: vec![RoleRequirement::new(RoleKey::Curator, 0)],
                ..memory_rule()
            }],
        };
        assert!(zero.validate().is_err());

        let twice = ApprovalMatrix {
            rules: vec![ApprovalRule {
                roles: vec![
                    RoleRequirement::new(RoleKey::Curator, 1),
                    RoleRequirement::new(RoleKey::Curator, 2),
                ],
                distinct_approvers: 2,
                ..memory_rule()
            }],
        };
        assert!(twice.validate().is_err());
        assert!(
            ApprovalMatrix {
                rules: vec![memory_rule()]
            }
            .validate()
            .is_ok()
        );
    }

    /// The floor is itself a valid matrix — the property that would
    /// otherwise break silently if someone edited it.
    #[test]
    fn the_floor_validates() {
        assert!(
            ApprovalMatrix {
                rules: ApprovalMatrix::floor().to_vec()
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn a_matrix_round_trips_through_its_stored_form() {
        let matrix = ApprovalMatrix {
            rules: vec![memory_rule()],
        };
        let json = serde_json::to_string(&matrix).unwrap();
        assert_eq!(
            serde_json::from_str::<ApprovalMatrix>(&json).unwrap(),
            matrix
        );
        // Terse rules parse: everything but the counts is optional.
        let terse: ApprovalMatrix = serde_json::from_str(
            r#"{"rules":[{"roles":[{"role":"curator","count":1}],"distinct_approvers":1}]}"#,
        )
        .unwrap();
        assert_eq!(terse.rules[0].asset, None);
        assert_eq!(terse.rules[0].min_sensitivity, Sensitivity::Public);
        assert!(serde_json::from_str::<ApprovalMatrix>(r#"{"rulez":[]}"#).is_err());
    }

    #[test]
    fn outstanding_describes_itself_for_a_reviewer() {
        let requirement = ApprovalMatrix::empty().resolve(
            AssetKind::Memory,
            Sensitivity::Restricted,
            ScopeKind::OrgUnit,
        );
        assert_eq!(
            requirement.outstanding(&[]).describe(),
            "administrator × 1, 2 distinct approver(s)"
        );
        assert_eq!(
            requirement
                .outstanding(&[
                    approval(1, &[RoleKey::Administrator]),
                    approval(2, &[RoleKey::Curator])
                ])
                .describe(),
            "nothing"
        );
    }
}
