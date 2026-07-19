// regulated-strict@1 — the default product pack (seed §6, ADR-0014).
//
// Deny-first: composition reads only the principal's own chain — its
// personal scope and that scope's ancestors, via entity-hierarchy
// membership, never string or path comparison. No cross-team read exists
// here at all; AUTHZ-4 lapses are the sanctioned relaxation. Everything
// this pack does not name is denied by Cedar's default-deny, and the
// compiled-in base layer forbids quarantined principals everything.

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

// Own chain only: the principal (parented to its placement scope) is a
// member of the resource scope. An unplaced principal has no scope
// parent and reads nothing.
permit (
    principal,
    action == Synveda::Action::"MemoryRead",
    resource
) when { principal in resource };
