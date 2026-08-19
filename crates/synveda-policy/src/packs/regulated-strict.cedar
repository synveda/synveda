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
        Synveda::Action::"ServiceIdentityManage",
        // Making and retiring workspaces and projects is structural
        // administration, priced with the rest of it (CPR-4, ADR-0071
        // decision 3). There is no delete action to name: retiring a
        // workspace is a status transition under WorkspaceUpdate, because
        // a workspace is what sessions, versions and audit events name.
        Synveda::Action::"WorkspaceCreate",
        Synveda::Action::"WorkspaceUpdate",
        Synveda::Action::"ProjectCreate",
        Synveda::Action::"ProjectUpdate",
        // Handing out and taking back access is administration of the same
        // kind, priced with it (CPR-5, ADR-0072 decision 7). `GroupManage` is
        // beside it rather than inside it because a group confers nothing on
        // its own — a deployment that wanted somebody to curate groups without
        // conferring access can say so by naming one and not the other.
        Synveda::Action::"MembershipGrant",
        Synveda::Action::"GroupManage"
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
        Synveda::Action::"AuditRead",
        Synveda::Action::"WorkspaceRead",
        Synveda::Action::"ProjectRead",
        // Who holds what is an administrative read, not a content one
        // (CPR-5, ADR-0072). It is deliberately not folded into
        // `WorkspaceRead`: a workspace's name discloses nothing and its
        // membership discloses who works on what.
        Synveda::Action::"MembershipRead"
    ],
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["steward", "org-admin", "auditor"])
};

// Workspaces and projects are **structure, not content** (CPR-4, ADR-0071
// decision 3), so the content roles read them too. This is the one place
// this feature widens a pack, and CNSL-2's finding is the argument: under
// every shipped pack `HierarchyRead` is steward/org-admin/auditor only, so
// a curator — the role the review inbox exists for — could not see the
// tree at all. A workspace's *name* discloses nothing about what is in it,
// and a contributor who cannot see the workspace they contribute to cannot
// use the product. What is inside stays behind MemoryRead, PromptRead,
// ContextPackRead and SkillRead, tier by tier, exactly as before.
permit (
    principal,
    action in [
        Synveda::Action::"WorkspaceRead",
        Synveda::Action::"ProjectRead"
    ],
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["viewer", "contributor", "curator"])
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

// The prompt registry (PRMT-1, ADR-0049 decision 4). Reading mirrors this
// pack's MemoryRead shape exactly — membership at the working tiers, one's
// own home and an explicit content-role binding up to `confidential`,
// personal scopes excluded from the binding — because a prompt is content
// the same people are trusted with, classified on the same four-tier scale.
// `restricted` is named by nothing: no path in the product mints it for an
// authored asset, so a permit for it would grant access to a row that
// cannot exist (decision 5).
//
// The membership floor is what makes zero-config true for consumers: a team
// member — and, through the base layer's carve-out, a team-anchored agent —
// resolves the org's prompts on their own chain with no binding at all.
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
    resource in principal.tenant
        && context.roles.containsAny(["viewer", "contributor", "curator"])
        && resource.kind != "user"
        && ["public", "internal", "confidential"].contains(context.sensitivity)
};

// Authoring mirrors the write floor: your own home role-free, the bound
// subtree with a content role, personal scopes excluded. A draft is not
// published material — the approval matrix decides that — so authoring is
// priced like writing rather than like publishing.
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
// MemoryRead — a pack's documents are content the same people are trusted
// with, classified on the same four-tier scale, and `restricted` is named
// by nothing for decision 12's reason.
//
// The membership floor matters more here than it did for prompts, and that
// is the case the feature exists for (decision 8): a reader who holds no
// readable *memory* at a scope still receives that scope's conventions,
// because this action admits pack chunks and `MemoryRead` never does.
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
    resource in principal.tenant
        && context.roles.containsAny(["viewer", "contributor", "curator"])
        && resource.kind != "user"
        && ["public", "internal", "confidential"].contains(context.sensitivity)
};

// Authoring mirrors PromptWrite, which mirrors the write floor. A draft is
// not published material — the approval matrix decides that, and for a pack
// under this pack it now decides it at two distinct people above a leaf
// (ADR-0050 decision 15).
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
// MemoryRead — a skill is content the same people are trusted with,
// classified on the same four-tier scale, and `restricted` is named by
// nothing for decision 11's reason.
//
// It is a *read* seam only in the sense of being served the bundle: a skill
// composes into no block (decision 9). What it does is decide whether an
// install may write these bytes onto this caller's disk, which is why the
// membership floor is the whole of zero-config here — a team member, and
// through the base layer's carve-out a team-anchored agent, installs the
// org's skills with no binding at all.
//
// Authoring is separate from ContextPackWrite because a skill is
// executable: this pack must be able to say "curate conventions here"
// without saying "author code here". Whether a draft then crosses the trust
// boundary is the approval matrix, which under every pack asks for a
// security reviewer and — since ADR-0051 decision 17 — a second signature.
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
    resource in principal.tenant
        && context.roles.containsAny(["viewer", "contributor", "curator"])
        && resource.kind != "user"
        && ["public", "internal", "confidential"].contains(context.sensitivity)
};

// Authoring mirrors PromptWrite, which mirrors the write floor. A draft is
// not published material — the approval matrix decides that, and for a pack
// under this pack it now decides it at two distinct people above a leaf
// (ADR-0050 decision 15).
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

// The quality override (SKIL-3, ADR-0053 decision 8): who may publish a
// skill the rubric or the checklist says is below this pack's bar.
//
// `steward` and `org-admin` only, and the exclusion of `curator` is the
// point rather than an omission. A curator is exactly the role that
// approves and publishes skills here, so granting it the override would
// make the gate self-service: the same person who decided the bundle was
// good enough would be the person recording that it was not. Under this
// pack the two authorities are two people, which is ADR-0051 decision 18's
// finding applied one seam later.
//
// This grants nothing about the *security* scan, which has no override at
// any tier and must not acquire one (ADR-0052 decision 3).
permit (
    principal,
    action == Synveda::Action::"SkillQualityOverride",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["steward", "org-admin"])
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

// The directory plane (AUTH-4, ADR-0059 decision 13): who may wire this
// tenant up to an identity provider, and rotate the credential that does
// it. Pack-uniform, like the channel, quarantine and proposal planes and
// for their reason — how a directory is connected does not loosen per pack.
//
// `org-admin` alone, and the exclusion of `steward` is the point rather
// than an omission: a steward's authority is a subtree, and this credential
// is not anchored in one. It can place a joiner anywhere the mapping rules
// reach and seal anybody the directory says has left, so an authority over
// it that stopped at a department boundary would be a fiction.
permit (
    principal,
    action == Synveda::Action::"DirectoryManage",
    resource
) when {
    resource in principal.tenant
        && context.roles.contains("org-admin")
};

// The breaker's release (AUTH-5, ADR-0060 decision 10). `org-admin` here as
// well, and the separation is about what a *stored* pack can now express
// rather than about these three saying anything different: a tenant that
// wants their IT team to run provisioning while somebody else signs off on
// mass deprovisioning can say so, which was unsayable while the two shared
// an action.
permit (
    principal,
    action == Synveda::Action::"DirectorySealAuthorise",
    resource
) when {
    resource in principal.tenant
        && context.roles.contains("org-admin")
};

// Redeeming an invitation (CPR-5, ADR-0072 decision 8).
//
// Permitted to every principal the base layer has not already forbidden, and
// that is the design rather than a hole: the **token** is the authority. An
// invitation is 256 bits of entropy, one-time, expiring and revocable, and the
// only person who can present one is somebody an authorised person handed it
// to. A person holding a valid invitation who is refused for want of a role is
// a person this product invited and then turned away.
//
// What still refuses: the base layer's quarantine forbid, its seal forbid and
// AUTH-3's token-scope confinement — which, because this action reaches only
// the tenant resource, forbids it to every service identity outright. An agent
// must not redeem a person's invitation.
//
// A deployment that wants the mechanism off deletes this permit in its own
// pack, visibly, in configuration.
permit (
    principal,
    action == Synveda::Action::"InviteAccept",
    resource
) when {
    resource == principal.tenant
};
