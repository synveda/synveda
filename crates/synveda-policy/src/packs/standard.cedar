// standard@4 — team-shares-by-default within a department (seed §6,
// ADR-0014, ADR-0015, ADR-0018, ADR-0020).
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
