// bootstrap@2 — the embedded default pack (AUTHZ-1, ADR-0012 decision 3;
// quarantine forbid added by AUTH-2, ADR-0013 decision 5).
//
// Two rules: quarantined principals may do nothing (forbid overrides any
// permit, current or future), and any other principal may administer its
// own tenant's hierarchy — ADR-0011 decision 8's semantics as enforced,
// versioned policy. Everything this pack does not name is denied by
// Cedar's default-deny. AUTHZ-2 replaces this with real policy packs;
// AUTHZ-3 adds roles.
forbid (
    principal,
    action,
    resource
) when { principal.quarantined };

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
