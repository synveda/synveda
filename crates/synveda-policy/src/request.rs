//! The domain-typed halves of the `authorize(subject, action, resource,
//! context)` facade (seed §6, ADR-0012 decision 1). Cedar types never
//! appear here: engines are an implementation detail of [`crate::pdp`].

use std::fmt;

use synveda_types::{
    Error, HierarchyNode, Lapse, PolicyAssignment, Result, Role, RoleBinding, ScopeId, Sensitivity,
    TenantId,
};

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
    /// The token's confinement scope — the anchor node whose subtree
    /// bounds every decision for a service identity (AUTH-3, ADR-0018
    /// decision 4). The base layer forbids everything outside it, roles
    /// notwithstanding, except own-chain `MemoryRead`. `None` for users
    /// and dev subjects: no confinement.
    pub token_scope: Option<ScopeId>,
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
    /// Land memory content at the resource scope — the write seam observe
    /// stands on (MEM-1, ADR-0020 decision 3). The packs permit the
    /// principal's own personal scope role-free (zero-config) and bound
    /// content roles beyond it.
    MemoryWrite,
    /// Run a classification proposal's effect at the resource scope: move
    /// records to the sensitivity their proposed versions carry (AUTHZ-5,
    /// ADR-0038 decision 9).
    ///
    /// Its own action rather than [`Action::MemoryWrite`], on
    /// [`Action::ChannelRollback`]'s separability rule: the write floor
    /// grants every principal `MemoryWrite` at its own home, and a pack must
    /// be able to say "you may write here" without saying "you may classify
    /// here".
    ///
    /// Like publishing, the route resolves the approval matrix on top of
    /// this decision — at the **maximum** of the current and proposed tiers,
    /// so a declassification is priced at the tier it is leaving — and
    /// additionally requires [`Action::MemoryRead`] at the working tier,
    /// which is the whose-material question every governance act asks
    /// (ADR-0038 decision 10).
    MemoryClassify,
    /// List/read quarantined observe events: the tenant's pending queue
    /// (the tenant resource) or a subtree's (a scope) — `/v1/quarantine`
    /// (MEM-2, ADR-0021 decision 6).
    QuarantineRead,
    /// Release or reject one quarantined event at its scope. Never a
    /// tenant-level action: a quarantined event lives at a node.
    QuarantineReview,
    /// Read packs and effective assignments (`/v1/policy/*`).
    PolicyRead,
    /// Assign a pack to the resource node, or set the tenant default
    /// (the tenant resource).
    PolicyAssign,
    /// Read role bindings at the resource node, or the tenant's bindings
    /// (the tenant resource) — `/v1/roles/*` (AUTHZ-3, ADR-0015).
    RoleRead,
    /// Bind or unbind a role at the resource node, or tenant-wide (the
    /// tenant resource). Decisions require [`AuthzContext::grant`] — the
    /// role being granted or revoked (ADR-0015 decision 5).
    RoleAssign,
    /// Read service-identity registrations: one at its anchor node, or
    /// the tenant's list (the tenant resource) — `/v1/service-identities`
    /// (AUTH-3, ADR-0018 decision 3).
    ServiceIdentityRead,
    /// Register or revoke a service identity at the resource node (its
    /// anchor). Never a tenant-level action: an agent is always anchored
    /// at a node (ADR-0018 decision 3).
    ServiceIdentityManage,
    /// Read the VedaFlow channels standing at the resource scope —
    /// `GET /v1/channels/{scope}` (FLOW-2, ADR-0031 decision 12).
    ChannelRead,
    /// Publish records onto the resource scope's published channel: the
    /// act that moves content across the trust boundary, so `inject`
    /// composes it as reviewed material. Never a tenant-level action —
    /// a channel belongs to a node — and never cross-scope: climbing to
    /// a higher scope's channel needs that scope's approvers (FLOW-5).
    ///
    /// Since FLOW-3 this decision is necessary but no longer sufficient:
    /// the approval matrix resolved at the same scope decides whether the
    /// acting principal's authority is enough on its own (ADR-0032
    /// decision 8).
    ChannelPublish,
    /// Rewind the resource scope's published channel to a state it has
    /// already held (FLOW-7, ADR-0036 decision 3).
    ///
    /// Its own action rather than [`Action::ChannelPublish`] so that a pack
    /// can grant publication broadly and rewinds narrowly — the two are the
    /// same grant forever if they share one. It deliberately resolves *no*
    /// approval matrix: a rewind can only install a state that cleared the
    /// matrix when it was installed (ADR-0036 decisions 1 and 2), and an
    /// incident response that needs a quorum is not a rollback. Like
    /// publishing, the route additionally requires the asset kind's read
    /// action at the same scope.
    ChannelRollback,
    /// Hold what the resource scope's channel *serves* at a commit, or
    /// release that hold (FLOW-7, ADR-0036 decisions 6 and 8).
    ///
    /// One action for setting, moving, and releasing a pin: they are the
    /// same decision by the same principal about the same channel, and
    /// releasing is never the more dangerous half — it can only return
    /// readers to material that was approved more recently.
    ChannelPin,
    /// Read proposals targeting the resource scope — `GET /v1/proposals`
    /// and `GET /v1/proposals/{id}` (FLOW-3, ADR-0032 decision 16).
    ProposalRead,
    /// Open a proposal against the resource scope's published channel.
    /// Grants nothing — a proposal asks — so the packs floor it on
    /// membership: a placed principal may propose at a scope it belongs
    /// to (tech plan §2.3's climb).
    ProposalOpen,
    /// Cast a review verdict on a proposal at the resource scope. This is
    /// where `compliance` and `security-reviewer` stop being markers.
    /// Whether the verdicts recorded so far are *enough* is the approval
    /// matrix's arithmetic, never this decision's (ADR-0032 decision 5).
    ProposalReview,
    /// Run an approved lapse proposal's effect at the resource scope: open
    /// a time-boxed grant of one action over this scope's material to
    /// another scope's members (AUTHZ-4, ADR-0037 decision 15).
    ///
    /// The resource is the scope whose material is *disclosed*, never the
    /// one receiving it: authority over a disclosure belongs where the
    /// material is. Like publishing, the route resolves the approval matrix
    /// on top of this decision — a lapse is the `policy` cell, which every
    /// pack has carried since FLOW-3.
    LapseGrant,
    /// End a standing lapse early, with a mandatory reason (ADR-0037
    /// decision 15).
    ///
    /// Its own action rather than a mode of [`Action::LapseGrant`], on
    /// [`Action::ChannelRollback`]'s precedent: a pack must be able to grant
    /// one broadly and the other narrowly. It deliberately resolves *no*
    /// approval matrix — a revocation installs nothing and can only narrow,
    /// and a product whose answer to "that grant was a mistake" is "convene
    /// the two stewards again" has not shipped revocation.
    LapseRevoke,
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
            Action::MemoryWrite => "memory.write",
            Action::MemoryClassify => "memory.classify",
            Action::QuarantineRead => "quarantine.read",
            Action::QuarantineReview => "quarantine.review",
            Action::PolicyRead => "policy.read",
            Action::PolicyAssign => "policy.assign",
            Action::RoleRead => "role.read",
            Action::RoleAssign => "role.assign",
            Action::ServiceIdentityRead => "service_identity.read",
            Action::ServiceIdentityManage => "service_identity.manage",
            Action::ChannelRead => "channel.read",
            Action::ChannelPublish => "channel.publish",
            Action::ChannelRollback => "channel.rollback",
            Action::ChannelPin => "channel.pin",
            Action::ProposalRead => "proposal.read",
            Action::ProposalOpen => "proposal.open",
            Action::ProposalReview => "proposal.review",
            Action::LapseGrant => "lapse.grant",
            Action::LapseRevoke => "lapse.revoke",
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
            Action::MemoryWrite => "MemoryWrite",
            Action::MemoryClassify => "MemoryClassify",
            Action::QuarantineRead => "QuarantineRead",
            Action::QuarantineReview => "QuarantineReview",
            Action::PolicyRead => "PolicyRead",
            Action::PolicyAssign => "PolicyAssign",
            Action::RoleRead => "RoleRead",
            Action::RoleAssign => "RoleAssign",
            Action::ServiceIdentityRead => "ServiceIdentityRead",
            Action::ServiceIdentityManage => "ServiceIdentityManage",
            Action::ChannelRead => "ChannelRead",
            Action::ChannelPublish => "ChannelPublish",
            Action::ChannelRollback => "ChannelRollback",
            Action::ChannelPin => "ChannelPin",
            Action::ProposalRead => "ProposalRead",
            Action::ProposalOpen => "ProposalOpen",
            Action::ProposalReview => "ProposalReview",
            Action::LapseGrant => "LapseGrant",
            Action::LapseRevoke => "LapseRevoke",
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
///
/// ABAC arrived with AUTHZ-5 as exactly one attribute —
/// [`AuthzContext::sensitivity`] — because a closed, ordered vocabulary is
/// the only kind a per-scope seam can be asked about without holding a
/// record (ADR-0038 decision 1). Channel, residency, time-of-day and
/// purpose-of-use are refused or deferred there, each for its own reason.
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
    /// The principal's role bindings relevant to this resource: rows
    /// bound at nodes of the resource's chain, plus its tenant-wide rows
    /// (AUTHZ-3, ADR-0015 decision 3). The PDP resolves the effective
    /// set — tenant-wide always; node rows when the bound node is on the
    /// chain — and passes it to policies as `context.roles`. Empty means
    /// no roles: strict by default.
    pub role_bindings: &'a [RoleBinding],
    /// For [`Action::RoleAssign`] only: the role being granted or
    /// revoked, passed to policies as `context.grant` so the base layer
    /// can guard org-admin escalation (ADR-0015 decision 5). A
    /// `RoleAssign` decision without it fails closed; other actions
    /// ignore it.
    pub grant: Option<Role>,
    /// For [`Action::MemoryRead`] only: the tier being asked about, passed
    /// to policies as `context.sensitivity` (AUTHZ-5, ADR-0038 decision 2).
    /// A `MemoryRead` decision without it fails closed — the `grant`
    /// discipline, applied to the attribute the base layer's `restricted`
    /// forbid stands on; other actions ignore it.
    ///
    /// One tier per decision, never a ceiling: the read path asks per tier
    /// and keeps the answers as a set, so a pack that says something
    /// non-contiguous gets exactly what it said (decision 3).
    pub sensitivity: Option<Sensitivity>,
    /// The lapses standing over the caller **as the caller's own read
    /// found them** (AUTHZ-4, ADR-0037 decision 9): grants whose grantee
    /// scope is on [`AuthzContext::principal_scopes`], neither revoked nor
    /// expired at the instant that query ran.
    ///
    /// Caller-supplied for the reason [`AuthzContext::role_bindings`] is —
    /// policy knows nothing of storage (seed §2.4) — and *pre-filtered* for
    /// the reason that matters more: expiry is a property of the decision
    /// rather than of a job, so the one query that loads these rows is the
    /// one place a window ends (decision 4). Empty means no grant stands,
    /// which is the zero-config answer everywhere.
    ///
    /// The PDP still gates them on the resource's own pack: a pack whose
    /// lapse ceiling is zero admits none of these, standing or not
    /// (decision 5).
    pub lapses: &'a [Lapse],
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
