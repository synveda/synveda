// standard@10 — team-shares-by-default within a department (seed §6,
// ADR-0014, ADR-0015, ADR-0018, ADR-0020, ADR-0021, ADR-0031, ADR-0032, ADR-0036,
// ADR-0037, ADR-0038).
//
// Own-chain reads plus the department subtree: sibling teams under the
// principal's department share by default. Other people's personal
// (user-kind) scopes stay excluded — the privacy floor, which AUTHZ-5
// resolved by leaving it exactly where it was: a tier says how sensitive
// material is, not whose it is (ADR-0038 decision 4). A hierarchy that skips
// the department level leaves the principal without a `department`
// attribute, and this pack collapses to regulated-strict's read surface.
//
// What sharing by default means here is the *working* tiers: the department
// subtree is a default, not an explicit grant, so `confidential` still takes
// a content-role binding or one's own home, exactly as under
// regulated-strict. `restricted` takes a lapse under every pack.

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

// The membership floor: own chain at the working tiers, as under
// regulated-strict.
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

// The department subtree, personal scopes excluded, at the working tiers:
// this pack's sharing default is a default, and `confidential` is the tier
// held to explicitly granted scopes under every pack.
permit (
    principal,
    action == Synveda::Action::"MemoryRead",
    resource
) when {
    principal has department
        && resource in principal.department
        && resource.kind != "user"
        && ["public", "internal"].contains(context.sensitivity)
};

// Content roles read the bound subtree, personal scopes excluded
// (ADR-0015 decision 4) — the explicit grant, so `confidential` joins.
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

// The prompt registry (PRMT-1, ADR-0049 decision 4). Reading mirrors this
// pack's MemoryRead shape, department sharing included: under `standard` a
// department's prompts are its members' prompts, which is the same default
// this pack applies to memory and for the same reason. `confidential` stays
// with the explicit grant; `restricted` is named by nothing, because no path
// in the product mints it for an authored asset (decision 5).
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
    principal has department
        && resource in principal.department
        && resource.kind != "user"
        && ["public", "internal"].contains(context.sensitivity)
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
// subtree with a content role. Sharing defaults differ on the read side; a
// governed asset authored beyond home always takes an explicit grant.
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
// MemoryRead — department sharing included, because under `standard` a
// department's conventions are its members' conventions, which is this
// pack's default for memory and holds at least as strongly for a glossary.
//
// This is the action that admits pack chunks into a block; `MemoryRead`
// never does (ADR-0050 decision 8).
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
    principal has department
        && resource in principal.department
        && resource.kind != "user"
        && ["public", "internal"].contains(context.sensitivity)
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

// Authoring is pack-uniform, like writing and like PromptWrite above.
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
// MemoryRead — department sharing included, because under `standard` a
// department's skills are its members' skills.
//
// A skill composes into no block (decision 9); this decides whether an
// install may write the bundle onto this caller's disk. Authoring is
// separate from ContextPackWrite because a skill is executable.
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
    principal has department
        && resource in principal.department
        && resource.kind != "user"
        && ["public", "internal"].contains(context.sensitivity)
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

// Authoring is pack-uniform, like writing and like PromptWrite above.
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

// The quality override (SKIL-3, ADR-0053 decision 8): the strict pack's
// two roles, unchanged.
//
// This pack is cheaper about the *bar* — `min_score` 50 and no mandatory
// checklist — rather than about who may step over it, which is the
// division of labour the config and the grant are for. A ten-person shop
// that hits this at all has hit a rubric failure or a written-down
// concern, and neither is a thing the person who wrote the bundle should
// clear on their own.
permit (
    principal,
    action == Synveda::Action::"SkillQualityOverride",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["steward", "org-admin"])
};

// The classification plane (AUTHZ-5, ADR-0038 decision 9): pack-uniform —
// see regulated-strict for the full rationale.
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
