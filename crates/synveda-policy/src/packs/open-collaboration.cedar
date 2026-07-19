// open-collaboration@5 — org-wide read (seed §6, ADR-0014, ADR-0015,
// ADR-0018, ADR-0020, ADR-0021).
//
// Own-chain reads plus every scope of the tenant, personal (user-kind)
// scopes excluded — the privacy floor. The seed's "non-restricted
// content" qualifier is AUTHZ-5's classification context; until it
// lands, openness stops at other people's personal scopes.

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

// The membership floor: own chain — covers the principal's personal
// scope, which the org-wide permit below deliberately does not.
permit (
    principal,
    action == Synveda::Action::"MemoryRead",
    resource
) when { principal in resource };

// Org-wide, personal scopes excluded — and only for *placed* principals:
// an identity provisioned into the hierarchy. An unplaced subject (a dev
// token) composes nothing under any pack (ADR-0014 decision 5).
permit (
    principal,
    action == Synveda::Action::"MemoryRead",
    resource
) when {
    principal has home
        && resource in principal.tenant
        && resource.kind != "user"
};

// Content roles read the bound subtree, personal scopes excluded
// (ADR-0015 decision 4) — under this pack the grant only adds reach for
// *unplaced* role holders, but keeping the rule identical across packs
// keeps the matrix identical too.
permit (
    principal,
    action == Synveda::Action::"MemoryRead",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["viewer", "contributor", "curator"])
        && resource.kind != "user"
};

// The write floor, role-free: own personal scope only. Openness is a
// read-side property — writes beyond home take an explicit grant under
// every pack (MEM-1, ADR-0020 decision 3).
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
