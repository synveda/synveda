// bootstrap@1 — the embedded default pack (AUTHZ-1, ADR-0012 decision 3).
//
// One rule: a principal may administer its own tenant's hierarchy. This
// preserves ADR-0011 decision 8's semantics as enforced, versioned policy.
// Everything this pack does not name is denied by Cedar's default-deny.
// AUTHZ-2 replaces this with real policy packs; AUTHZ-3 adds roles.
permit (
    principal,
    action in [
        Synveda::Action::"HierarchyCreate",
        Synveda::Action::"HierarchyRead",
        Synveda::Action::"HierarchyUpdate",
        Synveda::Action::"HierarchyDelete"
    ],
    resource
) when { resource in principal.tenant };
