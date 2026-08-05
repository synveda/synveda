//! The tenancy hierarchy (seed §4.1, HIER-1, ADR-0011).
//!
//! Every node is a scope — an attachment point for memories, context packs,
//! skills, prompts, and policies. The level vocabulary is fixed; depth is
//! "configurable" through the rank rule alone: a child's kind must outrank
//! its parent's, so optional levels (division, department) are simply
//! skipped, never configured.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{Error, ScopeId, TenantId};

/// Which level of the tenancy hierarchy a scope sits at.
///
/// No `Default`: a node's level is always an explicit decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScopeKind {
    /// The root — one per tenant; the tenant *is* the organisation.
    Org,
    /// Optional grouping level below the org (a division or a region).
    Division,
    /// Department.
    Department,
    /// Team.
    Team,
    /// A person's (or service identity's) personal scope; always a leaf.
    User,
}

impl ScopeKind {
    /// Position in the hierarchy, root first. A child's rank must be
    /// strictly greater than its parent's (ADR-0011): skipping levels is
    /// legal, inverting or repeating them is not, and nothing can sit
    /// below a user.
    #[must_use]
    pub const fn rank(&self) -> u8 {
        match self {
            ScopeKind::Org => 0,
            ScopeKind::Division => 1,
            ScopeKind::Department => 2,
            ScopeKind::Team => 3,
            ScopeKind::User => 4,
        }
    }

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            ScopeKind::Org => "org",
            ScopeKind::Division => "division",
            ScopeKind::Department => "department",
            ScopeKind::Team => "team",
            ScopeKind::User => "user",
        }
    }
}

impl fmt::Display for ScopeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ScopeKind {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "org" => Ok(ScopeKind::Org),
            "division" => Ok(ScopeKind::Division),
            "department" => Ok(ScopeKind::Department),
            "team" => Ok(ScopeKind::Team),
            "user" => Ok(ScopeKind::User),
            other => Err(Error::Invalid {
                message: format!("unknown scope kind: {other:?}"),
            }),
        }
    }
}

/// A node in the tenancy hierarchy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HierarchyNode {
    /// The node's identity — the `ScopeId` that assets attach to.
    pub id: ScopeId,
    /// Owning tenant; immutable for the life of the node.
    pub tenant_id: TenantId,
    /// Parent node; `None` only for the org root.
    pub parent_id: Option<ScopeId>,
    /// Hierarchy level.
    pub kind: ScopeKind,
    /// Human-stable handle, unique among siblings, immutable (ADR-0011);
    /// same grammar as tenant slugs.
    pub slug: String,
    /// Display name; renameable.
    pub name: String,
    /// Edges from the root: the org is 0, its children 1, and so on.
    /// Distinct from [`ScopeKind::rank`] when optional levels are skipped.
    pub depth: i32,
    /// Slug chain from the root (`acme/emea/payments`). Display and
    /// ordering only — never an authorisation input (ADR-0011).
    pub path: String,
    /// Whether this scope is sealed (AUTH-4, ADR-0059 decisions 7 and 9).
    ///
    /// Derived in the query, never stored on the node: a user-kind scope
    /// is sealed exactly when the identity that owns it is departed, and
    /// the one-personal-scope-per-identity constraint makes that a 1:1.
    /// The same shape [`crate::Identity::quarantined`] has had since
    /// ADR-0013 decision 4, and for the same reason — one source of
    /// truth, nothing to drift.
    ///
    /// It reaches Cedar as a `Scope` entity attribute, which is why it
    /// lives on the node rather than travelling beside it: a fact about a
    /// node that arrives by a second road is a fact that can disagree
    /// with the node. Non-user scopes are never sealed.
    pub sealed: bool,
    /// When the node was created.
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_order_the_vocabulary_root_first() {
        let ordered = [
            ScopeKind::Org,
            ScopeKind::Division,
            ScopeKind::Department,
            ScopeKind::Team,
            ScopeKind::User,
        ];
        for pair in ordered.windows(2) {
            assert!(pair[0].rank() < pair[1].rank(), "{pair:?} out of order");
        }
    }

    #[test]
    fn kind_round_trips_through_the_wire_name() {
        for kind in [
            ScopeKind::Org,
            ScopeKind::Division,
            ScopeKind::Department,
            ScopeKind::Team,
            ScopeKind::User,
        ] {
            assert_eq!(kind.as_str().parse::<ScopeKind>().unwrap(), kind);
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{}\"", kind.as_str()));
        }
        assert!("organisation".parse::<ScopeKind>().is_err());
    }
}
