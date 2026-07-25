// regulated-strict@6 — the default product pack (seed §6, ADR-0014,
// ADR-0015, ADR-0018, ADR-0020, ADR-0021).
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
        Synveda::Action::"RoleAssign",
        Synveda::Action::"ServiceIdentityManage"
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
        Synveda::Action::"RoleRead",
        Synveda::Action::"ServiceIdentityRead"
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

// The write floor, role-free (zero-config): every placed principal may
// land observe events at its own personal scope — and nowhere else.
// Deliberately narrower than the read floor: reads compose up the
// chain, writes land at home (MEM-1, ADR-0020 decision 3).
permit (
    principal,
    action == Synveda::Action::"MemoryWrite",
    resource
) when {
    principal has home
        && resource == principal.home
};

// Content roles write the bound subtree, personal scopes excluded — the
// explicit-grant mechanism for shared-scope writes (ADR-0015's
// contributor-writes marker, discharged by ADR-0020 decision 3).
// Viewer stays read-only by name; admin and audit roles grant no
// content write (least privilege, as with reads).
permit (
    principal,
    action == Synveda::Action::"MemoryWrite",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["contributor", "curator"])
        && resource.kind != "user"
};

// The quarantine review plane (MEM-2, ADR-0021 decision 6): stewards,
// org-admins, and security reviewers adjudicate quarantined observe
// events over the bound subtree — security-reviewer's first live
// action. Pack-uniform, like the write grant: how a security control is
// reviewed does not loosen per pack. Auditor is excluded (quarantine
// rows are content, redacted or not). Quarantined events live at
// user-kind home scopes, so this permit is the deliberate, recorded
// oversight carve-out of the personal-scope privacy floor — bounded by
// redaction, and the owner holds no self-release right.
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
