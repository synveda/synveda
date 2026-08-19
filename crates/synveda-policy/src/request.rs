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
    /// Read a workspace, or list the tenant's workspaces (CPR-4, ADR-0071
    /// decision 3).
    ///
    /// Structure rather than content, which is why the shipped packs grant it
    /// to the content roles as well as the admin ones: a workspace's *name*
    /// discloses nothing about what is in it, and everything that is in it
    /// stays behind the tiered reads.
    WorkspaceRead,
    /// Create a workspace — and, with it, the governed scope the workspace
    /// owns.
    WorkspaceCreate,
    /// Rename, re-describe or retire a workspace.
    ///
    /// Retirement is a status transition under this action rather than a
    /// delete action of its own: a workspace is what sessions, versions and
    /// audit events name, so nothing removes one, and a delete authority
    /// nobody can exercise would be a lie in the vocabulary.
    WorkspaceUpdate,
    /// Read a project, list a workspace's projects, or list the repositories
    /// a project is about.
    ProjectRead,
    /// Create a project inside a workspace.
    ProjectCreate,
    /// Rename, re-describe or retire a project — and attach or detach the
    /// repositories it is about.
    ///
    /// Repository attachment takes this action rather than one of its own:
    /// what a project is *about* is part of what the project is, and a
    /// separate authority over it would be one nobody could describe without
    /// describing this one.
    ProjectUpdate,
    /// Read who holds what on the access plane: a scope's effective members,
    /// the tenant's groups, its grants and its outstanding invitations
    /// (CPR-5, ADR-0072 decision 7).
    ///
    /// One read authority over the whole plane rather than one per noun, on
    /// [`Action::DirectoryManage`]'s argument: a list of who may act and a
    /// list of outstanding invitations to act are the same disclosure seen
    /// from two ends, and splitting them would create a role whose only power
    /// is reconnaissance over the other half.
    ///
    /// It is separate from [`Action::WorkspaceRead`] because a workspace's
    /// *name* discloses nothing and its *membership* discloses who works on
    /// what — a pack must be able to let a contributor see the workspace they
    /// contribute to without handing them the org chart.
    MembershipRead,
    /// Assign or revoke access at the resource scope: create and revoke
    /// grants, add and remove members, issue and withdraw invitations
    /// (ADR-0072 decision 7).
    ///
    /// Issuing an invitation takes this action rather than one of its own,
    /// and that is the whole of what an invitation is: deferred granting. An
    /// authority to invite that was weaker than the authority to grant would
    /// be a way around the second, and one that was stronger would be an
    /// action nobody could describe.
    MembershipGrant,
    /// Create, rename, retire and re-populate a group (ADR-0072 decision 7).
    ///
    /// Separate from [`Action::MembershipGrant`] on [`Action::ChannelRollback`]'s
    /// separability rule, and the distinction is real: a group says who exists
    /// together, a grant says what they may do, and a deployment must be able
    /// to let somebody curate the first without conferring the second. A group
    /// with no grant naming it confers nothing at all.
    GroupManage,
    /// Redeem an invitation (ADR-0072 decision 8).
    ///
    /// Every shipped pack permits this to any principal the base layer has
    /// not already forbidden, which is the point: the *token* is the
    /// authority, and a person holding a valid invitation who is refused for
    /// want of a role is a person the product invited and then turned away.
    ///
    /// It is an action rather than an exemption so that the invariant floor
    /// still runs — a quarantined principal and a sealed scope refuse it like
    /// everything else — and so that a deployment which wants to switch the
    /// mechanism off can say so in a pack instead of in a deployment flag.
    ///
    /// Tenant-only: a service identity's token confinement therefore forbids
    /// it outright (ADR-0018 decision 4), which is correct — an agent must not
    /// redeem a person's invitation.
    InviteAccept,
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
    /// Be served a prompt attached to the resource scope — the seam the
    /// registry's resolve stands on (PRMT-1, ADR-0049 decision 4).
    ///
    /// Names the tier it is asking about, exactly as [`Action::MemoryRead`]
    /// does and for the same reason: with four values the seam can be asked
    /// about a tier before any content is fetched. It carries no `lapsed`
    /// attribute — the lapse vocabulary is closed over `memory.read`
    /// (ADR-0037 decision 2) — and no pack names `restricted`, because
    /// nothing in the product mints that tier for an authored asset.
    ///
    /// It is also the read action that makes a rewind or a pin of
    /// `prompt/published` decidable, which is the deferral ADR-0036
    /// decision 3 parked on this feature by name.
    PromptRead,
    /// Author a prompt draft at the resource scope (ADR-0049 decision 4).
    ///
    /// Its own action rather than [`Action::MemoryWrite`], on
    /// [`Action::ChannelRollback`]'s separability rule: every placed
    /// principal holds the memory write floor at its own home, and a pack
    /// must be able to say "observe here" without saying "author governed
    /// assets here". Whether a draft may then cross the trust boundary is
    /// the approval matrix's arithmetic, never this decision's.
    PromptWrite,
    /// Be served a context pack attached to the resource scope — the seam
    /// composition stands on for pack material (PRMT-2, ADR-0050
    /// decision 7).
    ///
    /// Taken per scope inside the plan walk composition already runs, never
    /// as a second authorization path (decision 8), and it is what *admits*
    /// pack chunks: [`Action::MemoryRead`] never does, and this action never
    /// admits a memory. That separation is the case packs exist for — a
    /// scope may distribute conventions and glossaries to readers who hold
    /// no readable memory there at all.
    ///
    /// Carries the tier for [`Action::PromptRead`]'s reason and carries no
    /// `lapsed` for the same one. It is also the read action that makes a
    /// rewind or a pin of `context-pack/published` decidable, which
    /// discharges ADR-0036 decision 3 for the second of the three kinds it
    /// refused by name, leaving `skill`.
    ContextPackRead,
    /// Author a context pack draft at the resource scope (ADR-0050
    /// decision 7).
    ///
    /// Its own action rather than [`Action::PromptWrite`] on
    /// [`Action::ChannelRollback`]'s separability rule: bulk external
    /// documents are not hand-written templates, and a pack must be able to
    /// price the two differently.
    ContextPackWrite,
    /// Be served a skill bundle attached to the resource scope (SKIL-1,
    /// ADR-0051 decision 10).
    ///
    /// [`Action::PromptRead`]'s shape and none of
    /// [`Action::ContextPackRead`]'s: a skill is fetched by name and
    /// materialised into a client, never ranked into a composed block
    /// (decision 9), so this action is taken on its own route and appears
    /// nowhere in the composition plan walk.
    ///
    /// Carries the tier for [`Action::PromptRead`]'s reason and carries no
    /// `lapsed` for the same one. It is also the read action that makes a
    /// rewind or a pin of `skill/published` decidable, which discharges
    /// ADR-0036 decision 3 for the **last** of the three kinds it refused by
    /// name — after this, every asset kind with a channel has a read action.
    SkillRead,
    /// Author a skill draft at the resource scope (ADR-0051 decision 10).
    ///
    /// Its own action rather than [`Action::ContextPackWrite`] on
    /// [`Action::ChannelRollback`]'s separability rule, and here the reason
    /// is sharpest: a skill is executable, so a pack must be able to say
    /// "curate conventions here" without saying "author code here".
    SkillWrite,
    /// Publish a skill bundle the quality gate would otherwise refuse
    /// (SKIL-3, ADR-0053 decision 8).
    ///
    /// A *second* decision at the publish seam, distinct from the
    /// [`Action::ChannelPublish`] the publication already takes, and that
    /// separation is the entire content of the action: a publisher who
    /// may ship a good skill cannot necessarily ship one below the bar,
    /// and must go and find somebody who can. ADR-0051 decision 18's
    /// argument in its own idiom — the content of separating two
    /// authorities is that they can be two people.
    ///
    /// It exists at all because a quality bar with no way past it is a bar
    /// that gets routed around (ADR-0053 force 3). That is the asymmetry
    /// with SKIL-2's security gate and it is deliberate: `critical` is a
    /// band defined by having no legitimate reading, so refusing an
    /// exception costs an author only a wait for a rule fix; a *low score*
    /// always has a legitimate reading, so refusing an exception costs the
    /// product its registry. There is deliberately **no equivalent action
    /// for the scan** and there must not be one — an override that could
    /// reach the security floor would make ADR-0052 decision 3's guarantee
    /// negotiable by whoever holds this.
    ///
    /// A scope action carrying no `context.sensitivity`: it is a statement
    /// about a process rather than about content, so a tier would be a
    /// field with nothing to say.
    SkillQualityOverride,
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
    /// Read the tenant's audit chain: the search, the two questions AUD-2
    /// exists for, and the chain check — `/v1/audit/*` (ADR-0045
    /// decision 1).
    ///
    /// **The only action in the vocabulary that applies to the tenant
    /// resource and nothing else.** A chain is per tenant, and an audit
    /// answer that covered part of one would be an answer that quietly
    /// omitted rows; the schema refuses a scope resource so the refusal
    /// cannot be forgotten at a route (ADR-0045 decision 2). A
    /// subtree-bound auditor therefore holds nothing here — deliberately,
    /// and the denial names what it would take.
    ///
    /// Grants no content: this action reads record ids, object addresses,
    /// channels and tiers, and resolving any of them to a body is
    /// [`Action::MemoryRead`] through a different route (ADR-0045
    /// decision 6).
    AuditRead,
    /// Issue, list or revoke this tenant's provisioning credentials —
    /// `/v1/scim/credentials` (AUTH-4, ADR-0059 decision 13).
    ///
    /// **Tenant resource only**, like [`Action::AuditRead`] and for a
    /// related reason: a provisioning credential is anchored nowhere — it
    /// can place a joiner anywhere the mapping rules reach and seal
    /// anybody the directory says has left — so a subtree-scoped authority
    /// over one would be a fiction the schema refuses to let a route tell.
    ///
    /// One action for the inventory *and* its mutation, unlike the
    /// service-identity plane's pair: the inventory is a list of live keys
    /// to the directory plane, and a role that could see which credentials
    /// exist without being able to rotate them would hold nothing but
    /// reconnaissance.
    DirectoryManage,
    /// See what a pull sync's circuit breaker refused, and authorise it to
    /// seal past it — `GET /v1/directory/sync` and
    /// `POST /v1/directory/seal-authorisations` (AUTH-5, ADR-0060 decision
    /// 10).
    ///
    /// **Its own action rather than `DirectoryManage`'s**, and the split is
    /// the decision rather than a tidying. One hands out a provisioning
    /// token; this one authorises irreversible bulk sealing of personal
    /// scopes that do not unseal. A customer who wants their IT team to run
    /// provisioning while somebody else signs off on mass deprovisioning
    /// has no way to say so if the two share an action — SKIL-1 decision
    /// 18's finding in its general form, that separating two authorities
    /// has no content beyond their being two people.
    ///
    /// The read and the signature are **one** action, unlike the
    /// service-identity plane's pair and for the opposite reason to
    /// `DirectoryManage`'s: a signer who cannot see the number they are
    /// bounding is being asked to sign blind, which is precisely what
    /// decision 10's ceiling exists to prevent.
    ///
    /// Tenant-scoped, because a breaker trip is about a whole directory and
    /// a subtree-bounded authority over one would be a fiction.
    DirectorySealAuthorise,
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
    /// Every action in the vocabulary, for exhaustive iteration.
    ///
    /// Kept beside [`Action::PROBED_AT_SCOPE`] and its siblings so that
    /// adding a variant forces a classification: the test below asserts
    /// every action is in exactly one of the four groups, so a new action
    /// that nobody classified fails the build rather than silently going
    /// unanswerable at CNSL-2's probe.
    pub const ALL: [Action; 44] = [
        Action::HierarchyCreate,
        Action::HierarchyRead,
        Action::HierarchyUpdate,
        Action::HierarchyDelete,
        Action::WorkspaceRead,
        Action::WorkspaceCreate,
        Action::WorkspaceUpdate,
        Action::ProjectRead,
        Action::ProjectCreate,
        Action::ProjectUpdate,
        Action::MembershipRead,
        Action::MembershipGrant,
        Action::GroupManage,
        Action::InviteAccept,
        Action::MemoryRead,
        Action::MemoryWrite,
        Action::MemoryClassify,
        Action::PromptRead,
        Action::PromptWrite,
        Action::ContextPackRead,
        Action::ContextPackWrite,
        Action::SkillRead,
        Action::SkillWrite,
        Action::SkillQualityOverride,
        Action::QuarantineRead,
        Action::QuarantineReview,
        Action::PolicyRead,
        Action::PolicyAssign,
        Action::RoleRead,
        Action::RoleAssign,
        Action::ServiceIdentityRead,
        Action::ServiceIdentityManage,
        Action::AuditRead,
        Action::DirectoryManage,
        Action::DirectorySealAuthorise,
        Action::ChannelRead,
        Action::ChannelPublish,
        Action::ChannelRollback,
        Action::ChannelPin,
        Action::ProposalRead,
        Action::ProposalOpen,
        Action::ProposalReview,
        Action::LapseGrant,
        Action::LapseRevoke,
    ];

    /// The actions a capability probe answers as a plain yes/no about a
    /// **scope** (CNSL-2, ADR-0058 decision 1).
    ///
    /// The membership rule is mechanical rather than editorial, which is
    /// what keeps this list from becoming a place where a surface quietly
    /// decides what a reader is allowed to ask about: an action is here
    /// when it applies to `Scope` in the Cedar schema **and** needs no
    /// operand beyond (principal, action, resource). The two exclusions
    /// are their own groups — [`Action::TIERED_READS`], which answer with
    /// a tier set because a bare boolean would have to pick a tier and
    /// then hide which one, and [`Action::RoleAssign`], which fails closed
    /// without [`AuthzContext::grant`] and is therefore probed once per
    /// role. [`Action::AuditRead`] is absent because the schema refuses it
    /// a scope resource at all (ADR-0045 decision 2); it appears in
    /// [`Action::PROBED_AT_TENANT`], where the chain it reads actually
    /// lives.
    pub const PROBED_AT_SCOPE: [Action; 34] = [
        Action::HierarchyCreate,
        Action::HierarchyRead,
        Action::HierarchyUpdate,
        Action::HierarchyDelete,
        Action::WorkspaceRead,
        Action::WorkspaceCreate,
        Action::WorkspaceUpdate,
        Action::ProjectRead,
        Action::ProjectCreate,
        Action::ProjectUpdate,
        Action::MembershipRead,
        Action::MembershipGrant,
        Action::MemoryWrite,
        Action::MemoryClassify,
        Action::PromptWrite,
        Action::ContextPackWrite,
        Action::SkillWrite,
        Action::SkillQualityOverride,
        Action::QuarantineRead,
        Action::QuarantineReview,
        Action::PolicyRead,
        Action::PolicyAssign,
        Action::RoleRead,
        Action::ServiceIdentityRead,
        Action::ServiceIdentityManage,
        Action::ChannelRead,
        Action::ChannelPublish,
        Action::ChannelRollback,
        Action::ChannelPin,
        Action::ProposalRead,
        Action::ProposalOpen,
        Action::ProposalReview,
        Action::LapseGrant,
        Action::LapseRevoke,
    ];

    /// The same, for the **tenant** plane — what `whoami` answers.
    ///
    /// Strictly the schema's `Tenant`-applicable, operand-free set. It is
    /// much shorter than the scope set and that is the honest shape: most
    /// of this vocabulary is about a node, and an action that is only ever
    /// taken at a node has no tenant-level answer to give.
    pub const PROBED_AT_TENANT: [Action; 22] = [
        Action::HierarchyCreate,
        Action::HierarchyRead,
        Action::HierarchyUpdate,
        Action::HierarchyDelete,
        Action::WorkspaceRead,
        Action::WorkspaceCreate,
        Action::WorkspaceUpdate,
        Action::ProjectRead,
        Action::ProjectCreate,
        Action::ProjectUpdate,
        Action::MembershipRead,
        Action::MembershipGrant,
        Action::GroupManage,
        Action::InviteAccept,
        Action::QuarantineRead,
        Action::PolicyRead,
        Action::PolicyAssign,
        Action::RoleRead,
        Action::ServiceIdentityRead,
        Action::AuditRead,
        Action::DirectoryManage,
        Action::ProposalRead,
    ];

    /// The four actions that name the tier they ask about (AUTHZ-5,
    /// ADR-0038 decision 2), so a probe answers them with the set of tiers
    /// permitted rather than with a boolean.
    ///
    /// A boolean here would have to choose a tier to ask at, and then the
    /// answer would be about that tier while looking like it was about the
    /// action — the failure ADR-0038 decision 2 refuses a default for.
    pub const TIERED_READS: [Action; 4] = [
        Action::MemoryRead,
        Action::PromptRead,
        Action::ContextPackRead,
        Action::SkillRead,
    ];

    /// Stable machine-readable name: audit events, metrics labels, and
    /// [`Error::PolicyDenied`] all carry this string.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Action::HierarchyCreate => "hierarchy.create",
            Action::HierarchyRead => "hierarchy.read",
            Action::HierarchyUpdate => "hierarchy.update",
            Action::HierarchyDelete => "hierarchy.delete",
            Action::WorkspaceRead => "workspace.read",
            Action::WorkspaceCreate => "workspace.create",
            Action::WorkspaceUpdate => "workspace.update",
            Action::ProjectRead => "project.read",
            Action::ProjectCreate => "project.create",
            Action::ProjectUpdate => "project.update",
            Action::MembershipRead => "membership.read",
            Action::MembershipGrant => "membership.grant",
            Action::GroupManage => "group.manage",
            Action::InviteAccept => "invite.accept",
            Action::MemoryRead => "memory.read",
            Action::MemoryWrite => "memory.write",
            Action::MemoryClassify => "memory.classify",
            Action::PromptRead => "prompt.read",
            Action::PromptWrite => "prompt.write",
            Action::ContextPackRead => "context_pack.read",
            Action::ContextPackWrite => "context_pack.write",
            Action::SkillRead => "skill.read",
            Action::SkillWrite => "skill.write",
            Action::SkillQualityOverride => "skill.quality.override",
            Action::QuarantineRead => "quarantine.read",
            Action::QuarantineReview => "quarantine.review",
            Action::PolicyRead => "policy.read",
            Action::PolicyAssign => "policy.assign",
            Action::RoleRead => "role.read",
            Action::RoleAssign => "role.assign",
            Action::ServiceIdentityRead => "service_identity.read",
            Action::ServiceIdentityManage => "service_identity.manage",
            Action::AuditRead => "audit.read",
            Action::DirectoryManage => "directory.manage",
            Action::DirectorySealAuthorise => "directory.seal.authorise",
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
            Action::WorkspaceRead => "WorkspaceRead",
            Action::WorkspaceCreate => "WorkspaceCreate",
            Action::WorkspaceUpdate => "WorkspaceUpdate",
            Action::ProjectRead => "ProjectRead",
            Action::ProjectCreate => "ProjectCreate",
            Action::ProjectUpdate => "ProjectUpdate",
            Action::MembershipRead => "MembershipRead",
            Action::MembershipGrant => "MembershipGrant",
            Action::GroupManage => "GroupManage",
            Action::InviteAccept => "InviteAccept",
            Action::MemoryRead => "MemoryRead",
            Action::MemoryWrite => "MemoryWrite",
            Action::MemoryClassify => "MemoryClassify",
            Action::PromptRead => "PromptRead",
            Action::PromptWrite => "PromptWrite",
            Action::ContextPackRead => "ContextPackRead",
            Action::ContextPackWrite => "ContextPackWrite",
            Action::SkillRead => "SkillRead",
            Action::SkillWrite => "SkillWrite",
            Action::SkillQualityOverride => "SkillQualityOverride",
            Action::QuarantineRead => "QuarantineRead",
            Action::QuarantineReview => "QuarantineReview",
            Action::PolicyRead => "PolicyRead",
            Action::PolicyAssign => "PolicyAssign",
            Action::RoleRead => "RoleRead",
            Action::RoleAssign => "RoleAssign",
            Action::ServiceIdentityRead => "ServiceIdentityRead",
            Action::ServiceIdentityManage => "ServiceIdentityManage",
            Action::AuditRead => "AuditRead",
            Action::DirectoryManage => "DirectoryManage",
            Action::DirectorySealAuthorise => "DirectorySealAuthorise",
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

#[cfg(test)]
mod probe_vocabulary_tests {
    use super::Action;
    use std::collections::HashSet;

    /// The guard that makes the probe lists maintainable: every action is
    /// classified exactly once, so adding one to the vocabulary without
    /// deciding how a capability probe answers it fails here rather than
    /// leaving a hole in a governed read surface.
    #[test]
    fn every_action_is_classified_exactly_once() {
        let mut seen: HashSet<&'static str> = HashSet::new();
        let mut twice: Vec<&'static str> = Vec::new();
        for action in Action::PROBED_AT_SCOPE
            .iter()
            .chain(Action::TIERED_READS.iter())
            .chain(std::iter::once(&Action::RoleAssign))
            // The two tenant-only actions: the schema gives neither a
            // scope resource, so a per-node probe would be asking a
            // question the model refuses to represent.
            .chain(std::iter::once(&Action::AuditRead))
            .chain(std::iter::once(&Action::DirectoryManage))
            .chain(std::iter::once(&Action::DirectorySealAuthorise))
            // CPR-5's two tenant-plane actions: a group is tenant-wide and an
            // invitation is redeemed at the tenant, so neither has a per-scope
            // question to answer (ADR-0072 decision 7).
            .chain(std::iter::once(&Action::GroupManage))
            .chain(std::iter::once(&Action::InviteAccept))
        {
            if !seen.insert(action.as_str()) {
                twice.push(action.as_str());
            }
        }
        assert!(twice.is_empty(), "classified more than once: {twice:?}");

        let unclassified: Vec<&'static str> = Action::ALL
            .iter()
            .map(Action::as_str)
            .filter(|name| !seen.contains(name))
            .collect();
        assert!(
            unclassified.is_empty(),
            "unclassified actions — add each to PROBED_AT_SCOPE, TIERED_READS, \
             or the operand/tenant-only exceptions: {unclassified:?}"
        );
        assert_eq!(seen.len(), Action::ALL.len(), "ALL is missing a variant");
    }

    /// The tenant list is a subset of the vocabulary and never invents a
    /// name — a typo here would probe an action the schema has no permit
    /// for and read as a denial rather than as the mistake it is.
    #[test]
    fn the_tenant_probe_set_is_drawn_from_the_vocabulary() {
        let all: HashSet<&'static str> = Action::ALL.iter().map(Action::as_str).collect();
        for action in Action::PROBED_AT_TENANT {
            assert!(all.contains(action.as_str()), "not in ALL: {action:?}");
        }
    }

    /// A tiered read must never appear in a boolean list: it would be
    /// decided with no tier in context, which the PDP refuses rather than
    /// defaults (ADR-0038 decision 2).
    #[test]
    fn no_tiered_read_is_probed_as_a_boolean() {
        for tiered in Action::TIERED_READS {
            assert!(
                !Action::PROBED_AT_SCOPE.contains(&tiered),
                "{tiered:?} is tier-bearing and cannot be a boolean probe"
            );
            assert!(
                !Action::PROBED_AT_TENANT.contains(&tiered),
                "{tiered:?} is tier-bearing and cannot be a boolean probe"
            );
        }
    }

    /// `RoleAssign` fails closed without `context.grant`, so it must not
    /// be in a list the probe loops over without one.
    #[test]
    fn role_assign_is_not_probed_without_a_grant() {
        assert!(!Action::PROBED_AT_SCOPE.contains(&Action::RoleAssign));
        assert!(!Action::PROBED_AT_TENANT.contains(&Action::RoleAssign));
    }

    /// `AuditRead` applies to the tenant and nothing else (ADR-0045
    /// decision 2), so probing it at a scope would ask the schema a
    /// question it refuses.
    #[test]
    fn audit_read_is_probed_only_at_the_tenant() {
        assert!(!Action::PROBED_AT_SCOPE.contains(&Action::AuditRead));
        assert!(Action::PROBED_AT_TENANT.contains(&Action::AuditRead));
    }
}
