// regulated-strict@2 — the default product pack (seed §6, ADR-0014,
// ADR-0015).
//
// Deny-first: composition reads the principal's own chain, plus whatever
// a content-role binding explicitly grants — under this pack a viewer/
// contributor/curator binding *is* the seed's "explicit grant" for
// cross-team read; AUTHZ-4 lapses add the time-boxed variant. Everything
// this pack does not name is denied by Cedar's default-deny, and the
// compiled-in base layer forbids quarantined principals everything and
// guards org-admin escalation.

// The mutating admin planes: steward/org-admin, over the bound subtree —
// which roles are effective here is resolved from the bindings on the
// resource's chain (context.roles, ADR-0015 decision 3), so a binding at
// a node governs exactly that node's subtree.
permit (
    principal,
    action in [
        Synveda::Action::"HierarchyCreate",
        Synveda::Action::"HierarchyUpdate",
        Synveda::Action::"HierarchyDelete",
        Synveda::Action::"PolicyAssign",
        Synveda::Action::"RoleAssign"
    ],
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["steward", "org-admin"])
};

// The read-only admin surfaces: auditor joins (seed §5 — read-only,
// including audit logs when AUD-2 lands, never content).
permit (
    principal,
    action in [
        Synveda::Action::"HierarchyRead",
        Synveda::Action::"PolicyRead",
        Synveda::Action::"RoleRead"
    ],
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["steward", "org-admin", "auditor"])
};

// The membership floor, role-free (zero-config): the principal (parented
// to its placement scope) is a member of the resource scope. An unplaced
// principal has no scope parent and reads nothing.
permit (
    principal,
    action == Synveda::Action::"MemoryRead",
    resource
) when { principal in resource };

// Content roles read the bound subtree — the explicit-grant mechanism.
// Personal (user-kind) scopes stay excluded: the privacy floor until
// AUTHZ-5 classification (ADR-0015 decision 4). Admin and audit roles
// deliberately grant no content read (least privilege).
permit (
    principal,
    action == Synveda::Action::"MemoryRead",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["viewer", "contributor", "curator"])
        && resource.kind != "user"
};
