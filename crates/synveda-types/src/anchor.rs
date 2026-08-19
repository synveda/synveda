//! Scope anchors (CPR-6, ADR-0073): the ordered set of scopes a request is
//! decided against.
//!
//! ## What replaces "the chain"
//!
//! The old model had one answer to "where does this caller stand": their
//! placement node and its ancestors, a single path from a `user` node up to
//! the `org` root. Every decision walked it, every pack rule read it, and the
//! shape of an organisation was baked into the walk — a person had a
//! department, a department had a division, and the ladder was the model.
//!
//! There is no such path here, and the reason is not tidiness. In the governed
//! scope model a caller can stand in several places at once and none of them
//! contains the others: their own `principal` scope, a workspace somebody
//! shared with them, one project inside a *different* workspace they were
//! given directly, and the tenant root if a grant reaches that far. Those are
//! four applicable anchors, and collapsing them into one chain would have to
//! discard three.
//!
//! So an [`AnchorSet`] is **ordered, not nested**. It is a list of the scopes
//! this request may be decided against, most specific first, each carrying the
//! [`RoleKey`]s that actually reach it and where those came from.
//!
//! ## Ordering is specificity, never rank
//!
//! The order is a *tie-break for readers*, not an authorisation input: nothing
//! in this crate or in the PDP grants more because an anchor sorted earlier.
//! It exists so that a listing, a capability block and an audit payload all
//! present the same anchors in the same order, and so that "the nearest place
//! I hold something" is answerable without a second query.
//!
//! The key is [`AnchorSource`] precedence, then **depth in the scope tree,
//! deepest first**, then the scope id. Depth is a structural fact about the
//! tree (how many edges from the tenant root) and not the old rank vocabulary:
//! it says nothing about what kind of thing a scope is, an `org_unit` nested
//! four deep sorts ahead of one nested twice, and no comparison anywhere asks
//! whether one *kind* outranks another.
//!
//! ## Principal privacy is resolved here, not asserted downstream
//!
//! An anchor at somebody's own `principal` scope is produced only by a grant
//! written **at** it — [`crate::access::inherits_into`] is applied while the
//! set is built, so an ancestor grant never lands here as an anchor that
//! reaches into a private scope. The PDP's base layer restates the same floor
//! over the entity graph, which is deliberate belt and braces: this crate
//! decides what a resolver may produce, the base layer decides what any pack
//! may permit.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::access::RoleKey;
use crate::scope::ScopeKind;
use crate::{Error, GroupId, Result, ScopeId};

/// Why a scope is applicable to this request.
///
/// Ordered by precedence, most specific first — the discriminant order *is*
/// the precedence, so adding a variant is a deliberate placement rather than a
/// silent append.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorSource {
    /// The authenticated principal's own scope. Nobody else's grant reaches
    /// it, so it sorts first and always.
    PrincipalScope,
    /// The project the request selected.
    SelectedProject,
    /// The workspace the request selected.
    SelectedWorkspace,
    /// A scope this caller holds a grant at, directly or through a group,
    /// which the selection did not name.
    Grant,
    /// An organisation-unit scope on the ancestry of something above. Present
    /// because policy is assigned to it and a grant may be written at it —
    /// never because a person is expected to have one.
    OrgUnit,
    /// The tenant's root scope. Always last: it is the widest thing the model
    /// can express, so anything else is more specific than it.
    TenantRoot,
}

impl AnchorSource {
    /// Every source, in precedence order.
    pub const ALL: &'static [AnchorSource] = &[
        AnchorSource::PrincipalScope,
        AnchorSource::SelectedProject,
        AnchorSource::SelectedWorkspace,
        AnchorSource::Grant,
        AnchorSource::OrgUnit,
        AnchorSource::TenantRoot,
    ];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            AnchorSource::PrincipalScope => "principal_scope",
            AnchorSource::SelectedProject => "selected_project",
            AnchorSource::SelectedWorkspace => "selected_workspace",
            AnchorSource::Grant => "grant",
            AnchorSource::OrgUnit => "org_unit",
            AnchorSource::TenantRoot => "tenant_root",
        }
    }
}

impl fmt::Display for AnchorSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AnchorSource {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        AnchorSource::ALL
            .iter()
            .copied()
            .find(|source| source.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown anchor source: {s:?}"),
            })
    }
}

/// One scope this request may be decided against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeAnchor {
    /// The scope.
    pub scope_id: ScopeId,
    /// Its shape — what may parent it, and (for `principal`) whether anything
    /// above it reaches in.
    pub kind: ScopeKind,
    /// Its parent, `None` exactly for the tenant root.
    pub parent_scope_id: Option<ScopeId>,
    /// Edges from the tenant root. A structural fact, used only to order the
    /// set; never compared against another scope's *kind*.
    pub depth: i32,
    /// Why it is applicable.
    pub source: AnchorSource,
    /// The role keys effective here, ascending and deduplicated. Empty is a
    /// real answer: a scope can be applicable — the tenant root always is —
    /// without this caller holding anything at it.
    pub roles: Vec<RoleKey>,
    /// The scopes the roles were actually written at. `scope_id` itself when
    /// the grant is written here; an ancestor when it was inherited.
    pub granted_at: Vec<ScopeId>,
    /// The groups that reached this caller here, if any. A grant reaching them
    /// directly leaves this empty.
    pub via_groups: Vec<GroupId>,
}

impl ScopeAnchor {
    /// Whether any role reaches this caller here.
    #[must_use]
    pub fn is_held(&self) -> bool {
        !self.roles.is_empty()
    }

    /// Whether a role was written at this very scope rather than inherited.
    #[must_use]
    pub fn is_direct(&self) -> bool {
        self.granted_at.contains(&self.scope_id)
    }

    /// Whether this anchor is somebody's own scope.
    #[must_use]
    pub fn is_private(&self) -> bool {
        self.kind == ScopeKind::Principal
    }

    /// The sort key: source precedence, then deepest first, then the id.
    fn order_key(&self) -> (AnchorSource, i32, ScopeId) {
        (self.source, -self.depth, self.scope_id)
    }
}

/// The ordered set of anchors one request resolved to.
///
/// Constructed through [`AnchorSet::new`], which sorts and deduplicates, so a
/// set built twice from the same rows is the same set — the determinism a
/// capability block and an audit payload both need.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorSet {
    anchors: Vec<ScopeAnchor>,
}

impl AnchorSet {
    /// Sorts and deduplicates `anchors` into a set.
    ///
    /// Two entries for one scope are merged rather than dropped: the same
    /// scope can be both the selected workspace and a scope a grant reaches,
    /// and the answer is one anchor carrying the union of the roles under the
    /// **more specific** source.
    #[must_use]
    pub fn new(anchors: Vec<ScopeAnchor>) -> Self {
        let mut merged: Vec<ScopeAnchor> = Vec::with_capacity(anchors.len());
        for anchor in anchors {
            match merged
                .iter_mut()
                .find(|existing| existing.scope_id == anchor.scope_id)
            {
                Some(existing) => {
                    if anchor.source < existing.source {
                        existing.source = anchor.source;
                    }
                    existing.roles.extend(anchor.roles);
                    existing.granted_at.extend(anchor.granted_at);
                    existing.via_groups.extend(anchor.via_groups);
                }
                None => merged.push(anchor),
            }
        }
        for anchor in &mut merged {
            anchor.roles.sort_unstable();
            anchor.roles.dedup();
            anchor.granted_at.sort_unstable();
            anchor.granted_at.dedup();
            anchor.via_groups.sort_unstable();
            anchor.via_groups.dedup();
        }
        merged.sort_by_key(ScopeAnchor::order_key);
        AnchorSet { anchors: merged }
    }

    /// The anchors, most specific first.
    #[must_use]
    pub fn as_slice(&self) -> &[ScopeAnchor] {
        &self.anchors
    }

    /// The anchors, most specific first, consuming the set.
    #[must_use]
    pub fn into_vec(self) -> Vec<ScopeAnchor> {
        self.anchors
    }

    /// How many anchors there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.anchors.len()
    }

    /// Whether there are none at all — a caller with no principal scope, no
    /// selection and no grant, in a tenant with no root yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.anchors.is_empty()
    }

    /// Iterates the anchors, most specific first.
    pub fn iter(&self) -> std::slice::Iter<'_, ScopeAnchor> {
        self.anchors.iter()
    }

    /// The anchor for `scope_id`, if the set has one.
    #[must_use]
    pub fn get(&self, scope_id: ScopeId) -> Option<&ScopeAnchor> {
        self.anchors
            .iter()
            .find(|anchor| anchor.scope_id == scope_id)
    }

    /// The caller's own scope, when they have one.
    #[must_use]
    pub fn principal_scope(&self) -> Option<&ScopeAnchor> {
        self.anchors
            .iter()
            .find(|anchor| anchor.source == AnchorSource::PrincipalScope)
    }

    /// Every anchor this caller actually holds a role at, most specific first.
    pub fn held(&self) -> impl Iterator<Item = &ScopeAnchor> {
        self.anchors.iter().filter(|anchor| anchor.is_held())
    }
}

impl<'a> IntoIterator for &'a AnchorSet {
    type Item = &'a ScopeAnchor;
    type IntoIter = std::slice::Iter<'a, ScopeAnchor>;

    fn into_iter(self) -> Self::IntoIter {
        self.anchors.iter()
    }
}

impl IntoIterator for AnchorSet {
    type Item = ScopeAnchor;
    type IntoIter = std::vec::IntoIter<ScopeAnchor>;

    fn into_iter(self) -> Self::IntoIter {
        self.anchors.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor(kind: ScopeKind, depth: i32, source: AnchorSource) -> ScopeAnchor {
        ScopeAnchor {
            scope_id: ScopeId::new(),
            kind,
            parent_scope_id: None,
            depth,
            source,
            roles: Vec::new(),
            granted_at: Vec::new(),
            via_groups: Vec::new(),
        }
    }

    #[test]
    fn sources_sort_most_specific_first() {
        let mut sources = vec![
            AnchorSource::TenantRoot,
            AnchorSource::PrincipalScope,
            AnchorSource::OrgUnit,
            AnchorSource::SelectedWorkspace,
            AnchorSource::Grant,
            AnchorSource::SelectedProject,
        ];
        sources.sort_unstable();
        assert_eq!(sources, AnchorSource::ALL.to_vec());
    }

    #[test]
    fn source_names_round_trip() {
        for source in AnchorSource::ALL {
            assert_eq!(
                source.as_str().parse::<AnchorSource>().expect("parses"),
                *source
            );
            assert_eq!(
                serde_json::to_string(source).expect("serialises"),
                format!("\"{}\"", source.as_str())
            );
        }
    }

    #[test]
    fn the_set_orders_by_source_then_depth() {
        let root = anchor(ScopeKind::Tenant, 0, AnchorSource::TenantRoot);
        let shallow = anchor(ScopeKind::OrgUnit, 1, AnchorSource::OrgUnit);
        let deep = anchor(ScopeKind::OrgUnit, 4, AnchorSource::OrgUnit);
        let mine = anchor(ScopeKind::Principal, 1, AnchorSource::PrincipalScope);
        let set = AnchorSet::new(vec![
            root.clone(),
            shallow.clone(),
            deep.clone(),
            mine.clone(),
        ]);
        let order: Vec<ScopeId> = set.iter().map(|anchor| anchor.scope_id).collect();
        assert_eq!(
            order,
            vec![
                mine.scope_id,
                deep.scope_id,
                shallow.scope_id,
                root.scope_id
            ],
            "own scope first, then the deeper org unit, then the shallower, then the root"
        );
    }

    #[test]
    fn two_entries_for_one_scope_merge_under_the_more_specific_source() {
        let scope = ScopeId::new();
        let granted = ScopeId::new();
        let group = GroupId::new();
        let mut selected = anchor(ScopeKind::Workspace, 2, AnchorSource::SelectedWorkspace);
        selected.scope_id = scope;
        selected.roles = vec![RoleKey::Viewer];
        let mut by_grant = anchor(ScopeKind::Workspace, 2, AnchorSource::Grant);
        by_grant.scope_id = scope;
        by_grant.roles = vec![RoleKey::Owner, RoleKey::Viewer];
        by_grant.granted_at = vec![granted];
        by_grant.via_groups = vec![group];

        let set = AnchorSet::new(vec![by_grant, selected]);
        assert_eq!(set.len(), 1, "one scope is one anchor");
        let merged = set.get(scope).expect("present");
        assert_eq!(merged.source, AnchorSource::SelectedWorkspace);
        assert_eq!(merged.roles, vec![RoleKey::Owner, RoleKey::Viewer]);
        assert_eq!(merged.granted_at, vec![granted]);
        assert_eq!(merged.via_groups, vec![group]);
    }

    #[test]
    fn a_direct_grant_is_distinguishable_from_an_inherited_one() {
        let mut here = anchor(ScopeKind::Project, 3, AnchorSource::Grant);
        here.granted_at = vec![here.scope_id];
        here.roles = vec![RoleKey::Member];
        assert!(here.is_direct() && here.is_held());

        let mut above = anchor(ScopeKind::Project, 3, AnchorSource::Grant);
        above.granted_at = vec![ScopeId::new()];
        above.roles = vec![RoleKey::Member];
        assert!(!above.is_direct(), "an ancestor's grant is not direct");
    }

    #[test]
    fn an_empty_set_answers_rather_than_panics() {
        let set = AnchorSet::default();
        assert!(set.is_empty());
        assert_eq!(set.principal_scope(), None);
        assert_eq!(set.held().count(), 0);
    }
}
