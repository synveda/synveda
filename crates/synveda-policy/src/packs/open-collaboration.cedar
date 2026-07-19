// open-collaboration@1 — org-wide read (seed §6, ADR-0014).
//
// Own-chain reads plus every scope of the tenant, personal (user-kind)
// scopes excluded — the privacy floor. The seed's "non-restricted
// content" qualifier is AUTHZ-5's classification context; until it
// lands, openness stops at other people's personal scopes.

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

// Own chain — covers the principal's personal scope, which the org-wide
// permit below deliberately does not.
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
