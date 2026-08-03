// open-collaboration@10 — org-wide read (seed §6, ADR-0014, ADR-0015,
// ADR-0018, ADR-0020, ADR-0021, ADR-0031, ADR-0032, ADR-0036,
// ADR-0037, ADR-0038).
//
// Own-chain reads plus every scope of the tenant, personal (user-kind)
// scopes excluded — the privacy floor. The seed's "org-wide read for
// **non-restricted** content" is now the pack rather than a comment
// pointing at a future feature: the org-wide permit names every tier below
// `restricted`, and `restricted` is the base layer's forbid, liftable only
// by a lapse that declared the tier and therefore cleared the invariant
// floor's compliance requirement (ADR-0038 decisions 4 and 5).

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

// The read-only admin surfaces: auditor joins — including the audit chain
// since AUD-2, which reaches only the tenant resource (ADR-0045
// decision 2), and never content.
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

// The membership floor: own chain — covers the principal's personal
// scope, which the org-wide permit below deliberately does not.
permit (
    principal,
    action == Synveda::Action::"MemoryRead",
    resource
) when {
    principal in resource
        && ["public", "internal"].contains(context.sensitivity)
};

// Your own home, up to `confidential` — pack-uniform (ADR-0038 decision 4).
permit (
    principal,
    action == Synveda::Action::"MemoryRead",
    resource
) when {
    principal has home
        && resource == principal.home
        && ["public", "internal", "confidential"].contains(context.sensitivity)
};

// Org-wide, personal scopes excluded — and only for *placed* principals:
// an identity provisioned into the hierarchy. An unplaced subject (a dev
// token) composes nothing under any pack (ADR-0014 decision 5).
//
// **This is where seed §6's "org-wide read for non-restricted content"
// finally reads as itself.** The qualifier has been a comment deferring to
// AUTHZ-5 since ADR-0014; classification arrived, and non-restricted is
// exactly what this permit now names. `restricted` is left to the base
// layer's forbid, which is what makes the sentence true under this pack
// rather than merely stated in it.
permit (
    principal,
    action == Synveda::Action::"MemoryRead",
    resource
) when {
    principal has home
        && resource in principal.tenant
        && resource.kind != "user"
        && ["public", "internal", "confidential"].contains(context.sensitivity)
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
        && ["public", "internal", "confidential"].contains(context.sensitivity)
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

// The prompt registry (PRMT-1, ADR-0049 decision 4). Reading mirrors this
// pack's MemoryRead shape, the org-wide permit included: a registry whose
// prompts stopped at a team boundary under the pack whose whole point is
// org-wide read would be this pack disagreeing with itself. Personal scopes
// stay excluded from it, one's own home reaches `confidential`, and
// `restricted` is named by nothing — no path in the product mints it for an
// authored asset (decision 5).
permit (
    principal,
    action == Synveda::Action::"PromptRead",
    resource
) when {
    principal in resource
        && ["public", "internal"].contains(context.sensitivity)
};

permit (
    principal,
    action == Synveda::Action::"PromptRead",
    resource
) when {
    principal has home
        && resource == principal.home
        && ["public", "internal", "confidential"].contains(context.sensitivity)
};

permit (
    principal,
    action == Synveda::Action::"PromptRead",
    resource
) when {
    principal has home
        && resource in principal.tenant
        && resource.kind != "user"
        && ["public", "internal", "confidential"].contains(context.sensitivity)
};

permit (
    principal,
    action == Synveda::Action::"PromptRead",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["viewer", "contributor", "curator"])
        && resource.kind != "user"
        && ["public", "internal", "confidential"].contains(context.sensitivity)
};

// Authoring is pack-uniform, like writing: own home role-free, the bound
// subtree with a content role. Openness is a read-side property — a
// governed asset authored beyond home takes an explicit grant under every
// pack.
permit (
    principal,
    action == Synveda::Action::"PromptWrite",
    resource
) when {
    principal has home
        && resource == principal.home
};

permit (
    principal,
    action == Synveda::Action::"PromptWrite",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["contributor", "curator"])
        && resource.kind != "user"
};

// The context-pack registry (PRMT-2, ADR-0050 decision 7). The prompt
// plane's shape above, mirrored tier for tier onto this pack's own
// MemoryRead — the org-wide permit included, for the reason the prompt
// plane gives and more so: a pack whose whole content is org-wide read,
// withholding the org's own conventions from a session, would be this pack
// disagreeing with itself.
permit (
    principal,
    action == Synveda::Action::"ContextPackRead",
    resource
) when {
    principal in resource
        && ["public", "internal"].contains(context.sensitivity)
};

permit (
    principal,
    action == Synveda::Action::"ContextPackRead",
    resource
) when {
    principal has home
        && resource == principal.home
        && ["public", "internal", "confidential"].contains(context.sensitivity)
};

permit (
    principal,
    action == Synveda::Action::"ContextPackRead",
    resource
) when {
    principal has home
        && resource in principal.tenant
        && resource.kind != "user"
        && ["public", "internal", "confidential"].contains(context.sensitivity)
};

permit (
    principal,
    action == Synveda::Action::"ContextPackRead",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["viewer", "contributor", "curator"])
        && resource.kind != "user"
        && ["public", "internal", "confidential"].contains(context.sensitivity)
};

// Authoring is pack-uniform, like writing and like PromptWrite above:
// openness is a read-side property.
permit (
    principal,
    action == Synveda::Action::"ContextPackWrite",
    resource
) when {
    principal has home
        && resource == principal.home
};

permit (
    principal,
    action == Synveda::Action::"ContextPackWrite",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["contributor", "curator"])
        && resource.kind != "user"
};

// The skills registry (SKIL-1, ADR-0051 decision 10). The context-pack
// plane's shape above, mirrored tier for tier onto this pack's own
// MemoryRead — org-wide, personal scopes excluded, which is this pack's
// whole content.
//
// A skill composes into no block (decision 9); this decides whether an
// install may write the bundle onto this caller's disk. Authoring is
// separate from ContextPackWrite because a skill is executable, and the
// approval matrix still asks for a steward and a security reviewer, two
// distinct people, under this pack as under every other (ADR-0051
// decision 17).
permit (
    principal,
    action == Synveda::Action::"SkillRead",
    resource
) when {
    principal in resource
        && ["public", "internal"].contains(context.sensitivity)
};

permit (
    principal,
    action == Synveda::Action::"SkillRead",
    resource
) when {
    principal has home
        && resource == principal.home
        && ["public", "internal", "confidential"].contains(context.sensitivity)
};

permit (
    principal,
    action == Synveda::Action::"SkillRead",
    resource
) when {
    principal has home
        && resource in principal.tenant
        && resource.kind != "user"
        && ["public", "internal", "confidential"].contains(context.sensitivity)
};

permit (
    principal,
    action == Synveda::Action::"SkillRead",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["viewer", "contributor", "curator"])
        && resource.kind != "user"
        && ["public", "internal", "confidential"].contains(context.sensitivity)
};

// Authoring is pack-uniform, like writing and like PromptWrite above:
// openness is a read-side property.
permit (
    principal,
    action == Synveda::Action::"SkillWrite",
    resource
) when {
    principal has home
        && resource == principal.home
};

permit (
    principal,
    action == Synveda::Action::"SkillWrite",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["contributor", "curator"])
        && resource.kind != "user"
};

// The classification plane (AUTHZ-5, ADR-0038 decision 9): pack-uniform —
// see regulated-strict for the full rationale. Openness is a read-side
// property; what a tier *means* is uniform, or `restricted` would mean
// something different in an open tenant, which is the one thing it must
// never do.
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
