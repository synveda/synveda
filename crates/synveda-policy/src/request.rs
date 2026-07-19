//! The domain-typed halves of the `authorize(subject, action, resource,
//! context)` facade (seed §6, ADR-0012 decision 1). Cedar types never
//! appear here: engines are an implementation detail of [`crate::pdp`].

use std::fmt;

use synveda_types::{Error, HierarchyNode, PolicyAssignment, Result, ScopeId, TenantId};

/// Who is asking: a verified token subject resolved to a tenant (TEN-1)
/// with its provisioning status (AUTH-2, ADR-0013 decision 6). The caller
/// resolves `quarantined` at the enforcement seam — identity placement for
/// provisioned subjects, fail-closed `true` for IdP subjects that never
/// provisioned, `false` for out-of-band (dev) subjects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    /// The tenant the caller resolved to.
    pub tenant_id: TenantId,
    /// The verified token's `sub` claim.
    pub subject: String,
    /// Whether the subject is quarantined; the base layer forbids every
    /// action when set (ADR-0013 decision 5, ADR-0014 decision 2).
    pub quarantined: bool,
    /// The identity's placement — its personal scope node (AUTH-2). The
    /// principal entity is parented to it, so pack membership rules
    /// (`principal in resource`) walk the real hierarchy. `None` for
    /// subjects provisioning never placed (dev HS256): such a principal
    /// is a member of nothing and `MemoryRead` denies everywhere
    /// (ADR-0014 decision 5).
    pub scope_id: Option<ScopeId>,
}

/// The typed action vocabulary. Free-form action strings would let a typo
/// become an always-deny (or worse, an unaudited name); the vocabulary
/// grows in lockstep with the Cedar schema, and a mismatch fails at
/// compile-check time, not at decision time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    /// Create a hierarchy node under the resource (the parent scope, or
    /// the tenant when creating the org root).
    HierarchyCreate,
    /// Read a hierarchy node or listing anchored at the resource.
    HierarchyRead,
    /// Rename or move the resource node.
    HierarchyUpdate,
    /// Delete the resource node.
    HierarchyDelete,
    /// Include memories attached to the resource scope in the caller's
    /// composition — the seam inject/recall stand on (AUTHZ-2, ADR-0014
    /// decision 5).
    MemoryRead,
    /// Read packs and effective assignments (`/v1/policy/*`).
    PolicyRead,
    /// Assign a pack to the resource node, or set the tenant default
    /// (the tenant resource).
    PolicyAssign,
}

impl Action {
    /// Stable machine-readable name: audit events, metrics labels, and
    /// [`Error::PolicyDenied`] all carry this string.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Action::HierarchyCreate => "hierarchy.create",
            Action::HierarchyRead => "hierarchy.read",
            Action::HierarchyUpdate => "hierarchy.update",
            Action::HierarchyDelete => "hierarchy.delete",
            Action::MemoryRead => "memory.read",
            Action::PolicyRead => "policy.read",
            Action::PolicyAssign => "policy.assign",
        }
    }

    /// The action's entity id inside the Cedar schema's `Synveda` namespace.
    pub(crate) const fn cedar_id(&self) -> &'static str {
        match self {
            Action::HierarchyCreate => "HierarchyCreate",
            Action::HierarchyRead => "HierarchyRead",
            Action::HierarchyUpdate => "HierarchyUpdate",
            Action::HierarchyDelete => "HierarchyDelete",
            Action::MemoryRead => "MemoryRead",
            Action::PolicyRead => "PolicyRead",
            Action::PolicyAssign => "PolicyAssign",
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What is being acted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resource {
    /// The tenant itself — the resource of tenant-level actions such as
    /// creating the org root.
    Tenant(TenantId),
    /// A node of the tenancy hierarchy.
    Scope(ScopeId),
}

impl fmt::Display for Resource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Resource::Tenant(id) => write!(f, "tenant {id}"),
            Resource::Scope(id) => write!(f, "scope {id}"),
        }
    }
}

/// What the engine cannot fetch itself (seed §2.4: policy knows nothing of
/// storage): the caller supplies the data entities are materialised from
/// and the effective pack is resolved from (ADR-0014 decision 3).
/// AUTHZ-5 adds ABAC attributes (sensitivity, residency, channel, ...) here.
#[derive(Debug, Clone, Copy, Default)]
pub struct AuthzContext<'a> {
    /// The resource's scope chain — the node and its ancestors, in any
    /// order — from the caller's own transaction. A [`Resource::Scope`]
    /// whose chain is absent has no ancestors in the entity graph, so
    /// tenant-membership rules fail closed. Empty is correct for
    /// [`Resource::Tenant`]. HIER-3 replaces this with a synced entity
    /// store (ADR-0012 decision 4).
    pub scopes: &'a [HierarchyNode],
    /// The principal's placement chain — its personal scope node and that
    /// node's ancestors, in any order — when the principal is a
    /// provisioned identity. Empty for unplaced principals: membership
    /// rules then fail closed (ADR-0014 decision 5).
    pub principal_scopes: &'a [HierarchyNode],
    /// Pack assignments for the nodes of the resource's chain (missing
    /// rows mean "inherit"). The PDP walks the chain nearest-first; the
    /// first assigned node decides the effective pack (ADR-0014
    /// decision 3).
    pub assignments: &'a [PolicyAssignment],
    /// The tenant's default pack name, when one is stored — the fallback
    /// when no node on the chain carries an assignment. `None` falls
    /// back to `regulated-strict` (seed §2.1).
    pub default_pack: Option<&'a str>,
}

/// The verdict, plus everything the decision log and audit event need to
/// reproduce it (the AUTHZ-1 AC: decision + policy version, every call).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthzDecision {
    /// Allow or deny.
    pub allowed: bool,
    /// The pack that decided (a stored per-tenant pack, or `bootstrap`).
    pub pack_name: String,
    /// The pack's version at decision time.
    pub pack_version: i64,
    /// Ids of the policies that determined the outcome (empty for a
    /// default-deny where no policy matched at all).
    pub determining: Vec<String>,
}

impl AuthzDecision {
    /// Collapses the decision into the taxonomy: `Ok(())` on allow,
    /// [`Error::PolicyDenied`] naming the pack and version on deny.
    /// Denial reasons are policy names, never content (synveda-types).
    pub fn require(self, action: Action, resource: Resource) -> Result<()> {
        if self.allowed {
            return Ok(());
        }
        let determining = if self.determining.is_empty() {
            "no policy permitted it".to_owned()
        } else {
            format!("determining policies: {}", self.determining.join(", "))
        };
        Err(Error::PolicyDenied {
            action: action.as_str().to_owned(),
            resource: resource.to_string(),
            reason: format!(
                "pack {}@{} denied ({determining})",
                self.pack_name, self.pack_version
            ),
        })
    }
}
