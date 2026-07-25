//! Roles and role bindings (seed §5, AUTHZ-3, ADR-0015).
//!
//! Roles are a closed product vocabulary — `steward` means the same thing
//! in every tenant, like pack names (ADR-0014 decision 6). Bindings attach
//! a role to a subject at a hierarchy node, inherited downward; a binding
//! with no node binds at the tenant itself and applies everywhere. The
//! store reads bindings, the gateway carries them, and the PDP resolves
//! the effective role set per decision (ADR-0015 decision 3).

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{Error, ScopeId, TenantId};

/// The product role vocabulary (seed §5; ADR-0015 decision 1). Closed:
/// new roles are a product decision, not tenant data, so packs and the
/// golden matrix can name them portably.
///
/// `Ord` is declaration order — the seed §5 ordering — so approval
/// requirements sort into a stable, readable sequence (FLOW-3,
/// ADR-0032). It carries no privilege meaning: `curator < steward` here
/// is alphabetical-by-seed, not a lattice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    /// Read content in the bound subtree.
    Viewer,
    /// Viewer, plus content writes when the write surface lands (MEM-1).
    Contributor,
    /// Contributor, plus the review authority seed §5 names: publishing
    /// onto a channel (FLOW-2) and casting proposal verdicts (FLOW-3).
    Curator,
    /// Policy + membership administration for the bound subtree.
    Steward,
    /// Steward everywhere, plus the tenant plane and org-admin grants.
    OrgAdmin,
    /// Read-only administrative surfaces, including audit logs (AUD-2);
    /// never content.
    Auditor,
    /// Adjudicates quarantined observe events (MEM-2) and reviews
    /// proposals — the approval floor requires one on every `skill`,
    /// because a skill is executable (FLOW-3, ADR-0032 decision 4).
    /// SKIL-2 brings the skills themselves.
    SecurityReviewer,
    /// Reviews proposals, and the invariant approval floor requires the
    /// role on everything `restricted` (FLOW-3, ADR-0032 decision 4).
    /// Grants no content access: a compliance reviewer sees a proposal,
    /// not the corpus. AUTHZ-5 adds classification-time duties.
    Compliance,
}

impl Role {
    /// Every role, in seed §5 order.
    pub const ALL: [Role; 8] = [
        Role::Viewer,
        Role::Contributor,
        Role::Curator,
        Role::Steward,
        Role::OrgAdmin,
        Role::Auditor,
        Role::SecurityReviewer,
        Role::Compliance,
    ];

    /// Stable wire name, identical to the serde form; also the value the
    /// PDP passes to policies as `context.roles` and the store persists.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Contributor => "contributor",
            Role::Curator => "curator",
            Role::Steward => "steward",
            Role::OrgAdmin => "org-admin",
            Role::Auditor => "auditor",
            Role::SecurityReviewer => "security-reviewer",
            Role::Compliance => "compliance",
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Role {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Role::ALL
            .into_iter()
            .find(|role| role.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown role: {s:?}"),
            })
    }
}

/// A role bound to a subject at a node — the node's whole subtree holds
/// it, until nothing: bindings are strictly additive (ADR-0015
/// decision 4). `scope_id: None` binds at the tenant itself: the top of
/// the inheritance chain, in force everywhere including the tenant plane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleBinding {
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// The bound token subject. Subject-keyed, not identity-keyed
    /// (ADR-0015 decision 2): a binding may precede first login.
    pub subject: String,
    /// The node the role is bound at; `None` is tenant-wide.
    pub scope_id: Option<ScopeId>,
    /// The bound role.
    pub role: Role,
    /// When the binding was last changed.
    pub updated_at: DateTime<Utc>,
}
