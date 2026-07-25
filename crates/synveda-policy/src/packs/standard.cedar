// standard@7 — team-shares-by-default within a department (seed §6,
// ADR-0014, ADR-0015, ADR-0018, ADR-0020, ADR-0021).
//
// Own-chain reads plus the department subtree: sibling teams under the
// principal's department share by default. Other people's personal
// (user-kind) scopes stay excluded — the privacy floor until AUTHZ-5
// classification refines it. A hierarchy that skips the department level
// leaves the principal without a `department` attribute, and this pack
// collapses to regulated-strict's read surface.

// The mutating admin planes: steward/org-admin over the bound subtree
// (context.roles is resolved from the resource chain's bindings,
// ADR-0015 decision 3).
permit (
    principal,
    action in [
        Synveda::Action::"HierarchyCreate",
        Synveda::Action::"HierarchyUpdate",
        Synveda::Action::"HierarchyDelete",
        Synveda::Action::"PolicyAssign",
        Synveda::Action::"RoleAssign",
        Synveda::Action::"ServiceIdentityManage"
    ],
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["steward", "org-admin"])
};

// The read-only admin surfaces: auditor joins.
permit (
    principal,
    action in [
        Synveda::Action::"HierarchyRead",
        Synveda::Action::"PolicyRead",
        Synveda::Action::"RoleRead",
        Synveda::Action::"ServiceIdentityRead"
    ],
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["steward", "org-admin", "auditor"])
};

// The membership floor: own chain, as under regulated-strict.
permit (
    principal,
    action == Synveda::Action::"MemoryRead",
    resource
) when { principal in resource };

// The department subtree, personal scopes excluded.
permit (
    principal,
    action == Synveda::Action::"MemoryRead",
    resource
) when {
    principal has department
        && resource in principal.department
        && resource.kind != "user"
};

// Content roles read the bound subtree, personal scopes excluded
// (ADR-0015 decision 4).
permit (
    principal,
    action == Synveda::Action::"MemoryRead",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["viewer", "contributor", "curator"])
        && resource.kind != "user"
};

// The write floor, role-free: own personal scope only. Write governance
// is pack-uniform — sharing defaults differ on the read side; writes
// beyond home always take an explicit grant (MEM-1, ADR-0020
// decision 3).
permit (
    principal,
    action == Synveda::Action::"MemoryWrite",
    resource
) when {
    principal has home
        && resource == principal.home
};

// Content roles write the bound subtree, personal scopes excluded
// (ADR-0015's contributor-writes marker, ADR-0020 decision 3).
permit (
    principal,
    action == Synveda::Action::"MemoryWrite",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["contributor", "curator"])
        && resource.kind != "user"
};

// The quarantine review plane (MEM-2, ADR-0021 decision 6):
// pack-uniform — see regulated-strict for the full rationale.
permit (
    principal,
    action in [
        Synveda::Action::"QuarantineRead",
        Synveda::Action::"QuarantineReview"
    ],
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["steward", "org-admin", "security-reviewer"])
};

// The VedaFlow channel plane (FLOW-2, ADR-0031 decision 12): pack-uniform,
// like the quarantine plane and for the same reason — how content crosses
// the trust boundary does not loosen per pack. Reading a scope's channels
// joins the read-only admin surfaces, plus curator, who has to see a
// channel before publishing to it.
permit (
    principal,
    action == Synveda::Action::"ChannelRead",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["steward", "org-admin", "auditor", "curator"])
};

// Publishing is the curator's act by name (seed §5: curator "can
// pin/approve"), with steward and org-admin above it. Personal scopes are
// deliberately NOT excluded here, unlike the content-role grants: the
// route additionally requires MemoryRead on every record it admits
// (ADR-0031 decision 12), and the privacy floor already denies that at
// someone else's personal scope. So a user who holds a curator binding may
// publish their own memories to their own channel, and no curator can
// publish material they cannot read — which is the property a scope-kind
// clause here would only approximate. FLOW-3 puts required approvals in
// front of this action; it does not replace it.
permit (
    principal,
    action == Synveda::Action::"ChannelPublish",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["curator", "steward", "org-admin"])
};

// A rewind and a pin (FLOW-7, ADR-0036 decision 3): pack-uniform, and the
// same three roles as publishing, because a rewind can only reinstate a
// state this same set already approved. What the separate actions buy is
// the ability for a *stored* pack to say "publish widely, rewind narrowly"
// — the embedded packs choose not to, and could not express the choice at
// all if the two shared one action.
//
// Rolling back also requires the asset kind's read action at the same scope
// (ADR-0036 decision 3), which is what keeps a curator out of a teammate's
// personal published channel here, exactly as it does for publishing.
permit (
    principal,
    action in [
        Synveda::Action::"ChannelRollback",
        Synveda::Action::"ChannelPin"
    ],
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["curator", "steward", "org-admin"])
};

// The VedaFlow proposal plane (FLOW-3, ADR-0032 decision 16): pack-uniform,
// like the channel and quarantine planes and for the same reason — how
// content crosses the trust boundary does not loosen per pack. What varies
// per pack is the *approval matrix*, which is the thing FLOW-3's text says
// varies (asset × sensitivity × scope × pack).

// The tenant-wide listing: review and admin roles only. Being placed
// somewhere is not a reason to see every proposal in the tenant, so the
// membership floor below is deliberately absent here.
permit (
    principal,
    action == Synveda::Action::"ProposalRead",
    resource
) when {
    resource is Synveda::Tenant
        && resource in principal.tenant
        && context.roles.containsAny([
            "curator", "steward", "org-admin", "auditor", "compliance",
            "security-reviewer"
        ])
};

// At a scope, reading proposals mirrors this pack's MemoryRead shape — the
// membership floor, then content roles with personal scopes excluded — plus
// the review and audit roles, who have to see a proposal before acting.
permit (
    principal,
    action == Synveda::Action::"ProposalRead",
    resource
) when { resource is Synveda::Scope && principal in resource };

permit (
    principal,
    action == Synveda::Action::"ProposalRead",
    resource
) when {
    resource is Synveda::Scope
        && resource in principal.tenant
        && context.roles.containsAny([
            "viewer", "contributor", "curator", "steward", "org-admin",
            "auditor", "compliance", "security-reviewer"
        ])
        && resource.kind != "user"
};

// Opening a proposal grants nothing — it asks. So the floor is membership:
// a placed principal may propose at a scope it belongs to, which is how
// tribal knowledge climbs the gradient without needing a grant first (tech
// plan §2.3). Content roles reach the rest of the bound subtree; viewer
// stays read-only by name.
permit (
    principal,
    action == Synveda::Action::"ProposalOpen",
    resource
) when { principal in resource };

permit (
    principal,
    action == Synveda::Action::"ProposalOpen",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["contributor", "curator", "steward", "org-admin"])
        && resource.kind != "user"
};

// Reviewing is a role act with no membership floor: casting a verdict is
// authority, and this is where compliance and security-reviewer stop being
// markers. Auditor is excluded — read-only by name. Whether a given approval
// is *enough* is the matrix's arithmetic, never Cedar's (ADR-0032
// decision 5).
permit (
    principal,
    action == Synveda::Action::"ProposalReview",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny([
            "curator", "steward", "org-admin", "compliance", "security-reviewer"
        ])
};
