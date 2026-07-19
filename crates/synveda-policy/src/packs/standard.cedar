// standard@1 — team-shares-by-default within a department (seed §6,
// ADR-0014).
//
// Own-chain reads plus the department subtree: sibling teams under the
// principal's department share by default. Other people's personal
// (user-kind) scopes stay excluded — the privacy floor until AUTHZ-5
// classification refines it. A hierarchy that skips the department level
// leaves the principal without a `department` attribute, and this pack
// collapses to regulated-strict's read surface.

// The admin planes stay tenant-wide — ADR-0012 decision 3's semantics,
// carried by every product pack until AUTHZ-3 narrows them with roles.
permit (
    principal,
    action in [
        Synveda::Action::"HierarchyCreate",
        Synveda::Action::"HierarchyRead",
        Synveda::Action::"HierarchyUpdate",
        Synveda::Action::"HierarchyDelete",
        Synveda::Action::"PolicyRead",
        Synveda::Action::"PolicyAssign"
    ],
    resource
) when { resource in principal.tenant };

// Own chain, as under regulated-strict.
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
