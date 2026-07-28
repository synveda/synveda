// regulated-strict@10 — the default product pack (seed §6, ADR-0014,
// ADR-0015, ADR-0018, ADR-0020, ADR-0021, ADR-0031, ADR-0032, ADR-0036,
// ADR-0037, ADR-0038).
//
// Deny-first: composition reads the principal's own chain, plus whatever
// a content-role binding explicitly grants — under this pack a viewer/
// contributor/curator binding *is* the seed's "explicit grant" for
// cross-team read; AUTHZ-4 lapses add the time-boxed variant. Everything
// this pack does not name is denied by Cedar's default-deny, and the
// compiled-in base layer forbids quarantined principals everything and
// guards org-admin escalation.
//
// Since AUTHZ-5 every MemoryRead permit also names the tiers it covers
// (ADR-0038 decision 4), and the three mechanisms are graded by how
// deliberate they are: membership reads the working tiers, an explicit
// content-role binding (or one's own home) reaches `confidential`, and
// `restricted` is reachable by nothing this pack can say — the base layer
// forbids it outright unless a lapse that declared the tier stands, and such
// a lapse cleared the invariant floor's compliance requirement to exist.

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
// including the audit chain since AUD-2, never content).
//
// `AuditRead` is the one action here that reaches only the tenant resource
// (ADR-0045 decision 2), so this permit grants it exactly where the chain
// lives and a subtree binding does not reach it.
permit (
    principal,
    action in [
        Synveda::Action::"HierarchyRead",
        Synveda::Action::"PolicyRead",
        Synveda::Action::"RoleRead",
        Synveda::Action::"ServiceIdentityRead",
        Synveda::Action::"AuditRead"
    ],
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["steward", "org-admin", "auditor"])
};

// The membership floor, role-free (zero-config): the principal (parented
// to its placement scope) is a member of the resource scope. An unplaced
// principal has no scope parent and reads nothing.
//
// Membership reads the working tiers and no further: belonging to a scope
// is not a grant, and `confidential` is defined as the tier held to
// explicitly granted scopes (ADR-0038 decision 4).
permit (
    principal,
    action == Synveda::Action::"MemoryRead",
    resource
) when {
    principal in resource
        && ["public", "internal"].contains(context.sensitivity)
};

// Your own home, up to `confidential`: material extracted from your own
// sessions, at your own personal scope. Without this a model's tier
// proposal could make an author's own memory invisible to them, which is
// not a control but an accident (ADR-0038 decision 4).
permit (
    principal,
    action == Synveda::Action::"MemoryRead",
    resource
) when {
    principal has home
        && resource == principal.home
        && ["public", "internal", "confidential"].contains(context.sensitivity)
};

// Content roles read the bound subtree — the explicit-grant mechanism, and
// therefore the mechanism that reaches `confidential` (ADR-0038 decision 4).
// Admin and audit roles deliberately grant no content read (least
// privilege).
//
// Personal (user-kind) scopes stay excluded, and AUTHZ-5 resolved that
// deferral rather than lifting it: classification says how sensitive
// material is, and the privacy floor is about *whose* it is (ADR-0015
// decision 4). A tier cannot answer a question about ownership, so the
// exclusion stays exactly where it was — and a lapse cannot target a
// user-kind scope either.
permit (
    principal,
    action == Synveda::Action::"MemoryRead",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["viewer", "contributor", "curator"])
        && resource.kind != "user"
        && ["public", "internal", "confidential"].contains(context.sensitivity)
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

// The classification plane (AUTHZ-5, ADR-0038 decision 9): pack-uniform,
// like the channel, quarantine and proposal planes and for the same reason
// — what a tier *means* cannot loosen per pack, or `restricted` stops
// meaning one thing. The same three roles as publishing, plus the owner at
// their own home, because classifying your own memory is not an act of
// authority over anyone else.
//
// This decides who may run a classification effect, never how much
// approval it takes: that is the matrix, resolved at the maximum of the
// current and proposed tiers. The route additionally requires MemoryRead at
// the *working* tier — decision 10's whose-material question, the same guard
// publishing takes — so you may not reclassify material at a scope you are a
// stranger to, and the top tier stays maintainable by someone who could
// never compose it.
permit (
    principal,
    action == Synveda::Action::"MemoryClassify",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["curator", "steward", "org-admin"])
};

permit (
    principal,
    action == Synveda::Action::"MemoryClassify",
    resource
) when {
    principal has home
        && resource == principal.home
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

// The lapse plane (AUTHZ-4, ADR-0037 decision 15): pack-uniform, like the
// channel, quarantine and proposal planes and for the same reason — how
// content crosses the trust boundary does not loosen per pack. What varies
// per pack is the *window* (the LapseConfig ceiling) and the *approvals*
// (the `policy` cell of the matrix, which has carried tech plan §2.4's lapse
// row since FLOW-3).
//
// Granting is the steward's act by name (seed §6: "a steward may apply a
// scoped, reasoned, time-boxed override"), with org-admin above it. Curator
// is deliberately absent: a lapse is policy, and ADR-0015 put the policy
// plane at steward and above. The resource is the scope whose material is
// disclosed, so a steward bound at a department may open its teams' material
// and a team's steward may not open a sibling's.
permit (
    principal,
    action == Synveda::Action::"LapseGrant",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["steward", "org-admin"])
};

// Revoking is wider than granting, which is the whole reason the two are
// separate actions: security-reviewer joins, because ending a disclosure
// early is incident response — the same role that adjudicates quarantined
// events (MEM-2) and reviews executable skills. Revocation installs nothing
// and can only narrow, so no matrix stands in front of it.
permit (
    principal,
    action == Synveda::Action::"LapseRevoke",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["steward", "org-admin", "security-reviewer"])
};
