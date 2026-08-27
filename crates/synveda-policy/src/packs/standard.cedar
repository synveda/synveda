// standard@23 — shares by default inside what you belong to (seed §6,
// ADR-0014, ADR-0015, ADR-0018, ADR-0020, ADR-0021, ADR-0031, ADR-0032,
// ADR-0036, ADR-0037, ADR-0038, ADR-0072, ADR-0073).
//
// Own-chain reads plus **the neighbourhood of every scope this caller holds**:
// somebody given one project reads the workspace it sits in and the other
// projects in it, and a workspace they hold nothing at stays closed. A grant
// reaches its own subtree under every pack — that is what a grant is — so what
// this pack adds is exactly one step *outward* from what somebody was given. Other people's
// `principal` scopes are excluded by the base layer itself, which is stronger
// than the pack clause that used to say it — a tier says how sensitive
// material is, not whose it is (ADR-0038 decision 4).
//
// Until CPR-6 this sentence was "the department subtree", read off a
// `principal.department` attribute that named the nearest department-kind
// ancestor of somebody's placement. That attribute is gone with the rank
// vocabulary it belonged to (ADR-0073 decision 4), and what replaces it is
// not a renamed rung: `principal.ambit` is the **parent** of every scope a
// grant reaches this caller at, so the sharing default now follows what
// somebody was actually given rather than where an org chart put them, and
// nothing anywhere asks what kind of thing that parent is. The tenant root is
// excluded, which is what keeps this pack from meaning `open-collaboration`.
// A caller who holds nothing gets exactly regulated-strict's read surface,
// which is the same collapse the missing-department case used to produce.
//
// What sharing by default means here is still the *working* tiers: an
// anchor's subtree is a default, not an explicit grant, so `confidential`
// still takes a content role or one's own scope, exactly as under
// regulated-strict. `restricted` Knowledge takes a governed relaxation under
// every pack.

// **The privacy floor moved.** Until CPR-6 every permit here restated
// `resource.kind != "user"`; it is now one base-layer forbid over
// `principal`-shaped scopes that no pack can drop and that a grant written
// *at* somebody's own scope can lift (ADR-0073 decision 6). So "personal
// scopes excluded" is still true of every permit below — it is just no longer
// each permit's job to remember, and sharing your own notes deliberately is
// finally sayable.

// The mutating admin planes: owner/administrator over the granted
// subtree (context.roles is the grant keys resolved from the resource
// chain, CPR-7 ADR-0074 decision 6 over ADR-0015 decision 3).
permit (
    principal,
    action in [
        Synveda::Action::"ScopeCreate",
        Synveda::Action::"ScopeUpdate",
        Synveda::Action::"PolicyAssign",
        Synveda::Action::"ConfigurationWrite",
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
        && context.roles.containsAny(["owner", "administrator"])
};

// The read-only admin surfaces — including the audit chain since AUD-2,
// which reaches only the tenant resource (ADR-0045 decision 2), and never
// content. The `auditor` key that used to join here left with the binding
// vocabulary (ADR-0074 decision 6); Prompt 27 is where the specialist
// names come back.
permit (
    principal,
    action in [
        Synveda::Action::"ScopeRead",
        Synveda::Action::"PolicyRead",
        Synveda::Action::"ConfigurationRead",
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
        && context.roles.containsAny(["owner", "administrator"])
};

// Workspaces and projects are **structure, not content** (CPR-4, ADR-0071
// decision 3), so the content roles read them too. This is the one place
// this feature widens a pack, and CNSL-2's finding is the argument: under
// every shipped pack `ScopeRead` is owner/administrator only, so
// a curator — the role the review inbox exists for — could not see the
// tree at all. A workspace's *name* discloses nothing about what is in it,
// and a member who cannot see the workspace they contribute to cannot
// use the product. What is inside stays behind KnowledgeRead, PromptRead,
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
        && context.roles.containsAny(["viewer", "curator", "member", "reviewer", "owner", "administrator"])
};

// Stable Knowledge (CPR-16, ADR-0081): the same content reach as memory,
// expressed over both a scope (create/list) and an exact aggregate (read or
// mutate). A relaxation is matched exactly for subject, scope, action, tier
// and database-time window before Cedar sees it.
permit (principal, action == Synveda::Action::"KnowledgeRead", resource)
when {
    resource is Synveda::Scope
        && principal in resource
        && ["public", "internal"].contains(context.sensitivity)
};
permit (principal, action == Synveda::Action::"KnowledgeRead", resource)
when {
    resource is Synveda::KnowledgeItem
        && principal in resource.scope
        && ["public", "internal"].contains(context.sensitivity)
};
permit (principal, action == Synveda::Action::"KnowledgeRead", resource)
when {
    principal has own_scope
        && ((resource is Synveda::Scope && resource == principal.own_scope)
            || (resource is Synveda::KnowledgeItem && resource.scope == principal.own_scope))
        && ["public", "internal", "confidential"].contains(context.sensitivity)
};
permit (principal, action == Synveda::Action::"KnowledgeRead", resource)
when {
    ((resource is Synveda::Scope && resource in principal.ambit)
        || (resource is Synveda::KnowledgeItem && resource.scope in principal.ambit))
        && ["public", "internal"].contains(context.sensitivity)
};
permit (principal, action == Synveda::Action::"KnowledgeRead", resource)
when {
    resource in principal.tenant
        && context.roles.containsAny(["viewer", "curator", "member", "reviewer", "owner", "administrator"])
        && ["public", "internal", "confidential"].contains(context.sensitivity)
};
permit (principal, action == Synveda::Action::"KnowledgeWrite", resource)
when {
    principal has own_scope
        && ((resource is Synveda::Scope && resource == principal.own_scope)
            || (resource is Synveda::KnowledgeItem && resource.scope == principal.own_scope))
};
permit (principal, action == Synveda::Action::"KnowledgeWrite", resource)
when {
    resource in principal.tenant
        && context.roles.containsAny(["curator", "member", "owner", "administrator"])
};
permit (principal, action == Synveda::Action::"KnowledgeForget", resource)
when {
    principal has own_scope
        && ((resource is Synveda::Scope && resource == principal.own_scope)
            || (resource is Synveda::KnowledgeItem && resource.scope == principal.own_scope))
};
permit (principal, action == Synveda::Action::"KnowledgeForget", resource)
when {
    resource in principal.tenant
        && context.roles.containsAny(["owner", "administrator"])
};

// The session ledger (CPR-10, ADR-0076 decision 6).
//
// regulated-strict's shape plus this pack's one step outward: the
// neighbourhood of everything you hold. Under `standard` a workspace's runs
// are its members' runs, which is the same default this pack applies to memory
// and to conventions — and the argument is a little stronger here, because the
// thing a teammate most often needs is *what the last agent already tried*.
//
// Other people's `principal` scopes are excluded by the base layer, so
// somebody's own runs stay their own under this pack exactly as under the
// strict one.
// The membership floor, in two clauses because this is the one rule that
// cannot be uniform over the resource union: `principal in resource` walks
// *up* from the caller's own scope, which reaches a `Scope` resource and can
// never reach a `Session` — a session hangs *below* a scope. So the session
// case asks the same question about the scope it hangs below.
permit (
    principal,
    action == Synveda::Action::"SessionRead",
    resource
) when { resource is Synveda::Scope && principal in resource };

permit (
    principal,
    action == Synveda::Action::"SessionRead",
    resource
) when { resource is Synveda::Session && principal in resource.scope };

// The neighbourhood, uniform over the union: a session is parented to the
// scope it runs at, so `resource in principal.ambit` is true for a run inside
// something one hop out from anything this caller holds.
permit (
    principal,
    action == Synveda::Action::"SessionRead",
    resource
) when { resource in principal.ambit };

permit (
    principal,
    action == Synveda::Action::"SessionRead",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["viewer", "curator", "member", "reviewer", "owner", "administrator"])
};

// Writing is pack-uniform, like every other write floor: sharing defaults
// differ on the read side, and opening a run beyond your own scope always
// takes an explicit grant.
// The write floor, role-free: your own scope, and the runs that hang below it.
// `resource in principal.own_scope` is uniform over the union — Cedar's `in`
// is reflexive, so it is "this scope is mine" for a `Scope` and "this run is
// at a scope of mine" for a `Session`.
permit (
    principal,
    action == Synveda::Action::"SessionWrite",
    resource
) when {
    principal has own_scope
        && resource in principal.own_scope
};

permit (
    principal,
    action == Synveda::Action::"SessionWrite",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["curator", "member", "owner", "administrator"])
};


// The diagnostic expansion (CPR-11, ADR-0077 decision 3).
//
// `SessionRead` serves a **timeline**: this event was a message, that one ran
// a tool, here is a line about each. `SessionDiagnostics` serves the raw
// payload — what the person and the agent actually said, byte for byte, the
// prompt, the tool arguments, the diff. The two are not one authority, and
// this pack's split is what that decision buys.
//
// It is strictly narrower than this pack's `SessionRead` and must stay so:
// an authority to read a payload without the authority to know the event
// exists is not expressible here, because the route reads the timeline's own
// rows.
// Note what is **not** here: this pack's `ambit` clause. Sharing one step
// outward is a decision about the shape of a team's reading surface; a
// neighbouring project's raw prompts are not a default under any pack.

// The floor, in the same two clauses and for the same reason: your own runs'
// payloads are yours, and `principal in resource` cannot reach a `Session`.
permit (
    principal,
    action == Synveda::Action::"SessionDiagnostics",
    resource
) when { resource is Synveda::Scope && principal in resource };

permit (
    principal,
    action == Synveda::Action::"SessionDiagnostics",
    resource
) when { resource is Synveda::Session && principal in resource.scope };

permit (
    principal,
    action == Synveda::Action::"SessionDiagnostics",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["reviewer", "owner", "administrator"])
};

// The prompt registry (PRMT-1, ADR-0049 decision 4). Reading mirrors this
// pack's KnowledgeRead shape, anchor sharing included: under `standard` a
// workspace's prompts are its members' prompts, which is the same default
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
    principal has own_scope
        && resource == principal.own_scope
        && ["public", "internal", "confidential"].contains(context.sensitivity)
};

permit (
    principal,
    action == Synveda::Action::"PromptRead",
    resource
) when {
    resource in principal.ambit
        && ["public", "internal"].contains(context.sensitivity)
};

permit (
    principal,
    action == Synveda::Action::"PromptRead",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["viewer", "curator", "member", "reviewer", "owner", "administrator"])
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
    principal has own_scope
        && resource == principal.own_scope
};

permit (
    principal,
    action == Synveda::Action::"PromptWrite",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["curator", "member", "owner", "administrator"])
};

// The context-pack registry (PRMT-2, ADR-0050 decision 7). The prompt
// plane's shape above, mirrored tier for tier onto this pack's own
// KnowledgeRead — anchor sharing included, because under `standard` a
// workspace's conventions are its members' conventions, which is this
// pack's default for memory and holds at least as strongly for a glossary.
//
// This is the action that admits pack chunks into a block; `KnowledgeRead`
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
    principal has own_scope
        && resource == principal.own_scope
        && ["public", "internal", "confidential"].contains(context.sensitivity)
};

permit (
    principal,
    action == Synveda::Action::"ContextPackRead",
    resource
) when {
    resource in principal.ambit
        && ["public", "internal"].contains(context.sensitivity)
};

permit (
    principal,
    action == Synveda::Action::"ContextPackRead",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["viewer", "curator", "member", "reviewer", "owner", "administrator"])
        && ["public", "internal", "confidential"].contains(context.sensitivity)
};

// Authoring is pack-uniform, like writing and like PromptWrite above.
permit (
    principal,
    action == Synveda::Action::"ContextPackWrite",
    resource
) when {
    principal has own_scope
        && resource == principal.own_scope
};

permit (
    principal,
    action == Synveda::Action::"ContextPackWrite",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["curator", "member", "owner", "administrator"])
};

// The skills registry (SKIL-1, ADR-0051 decision 10). The context-pack
// plane's shape above, mirrored tier for tier onto this pack's own
// KnowledgeRead — anchor sharing included, because under `standard` a
// workspace's skills are its members' skills.
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
    principal has own_scope
        && resource == principal.own_scope
        && ["public", "internal", "confidential"].contains(context.sensitivity)
};

permit (
    principal,
    action == Synveda::Action::"SkillRead",
    resource
) when {
    resource in principal.ambit
        && ["public", "internal"].contains(context.sensitivity)
};

permit (
    principal,
    action == Synveda::Action::"SkillRead",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["viewer", "curator", "member", "reviewer", "owner", "administrator"])
        && ["public", "internal", "confidential"].contains(context.sensitivity)
};

// Authoring is pack-uniform, like writing and like PromptWrite above.
permit (
    principal,
    action == Synveda::Action::"SkillWrite",
    resource
) when {
    principal has own_scope
        && resource == principal.own_scope
};

permit (
    principal,
    action == Synveda::Action::"SkillWrite",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["curator", "member", "owner", "administrator"])
};

// Trusted MCP catalogue metadata and governed binding intent (CPR-25). A
// declaration is never invocation authority; project execution remains at
// the client/provider boundary.
permit (
    principal,
    action == Synveda::Action::"ToolRead",
    resource
) when {
    resource in principal.tenant
        && principal in resource
};

permit (
    principal,
    action == Synveda::Action::"ToolRead",
    resource
) when {
    resource in principal.tenant
        && resource in principal.ambit
};

permit (
    principal,
    action == Synveda::Action::"ToolRead",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["viewer", "curator", "member", "reviewer", "owner", "administrator"])
};

permit (
    principal,
    action == Synveda::Action::"ToolWrite",
    resource
) when {
    principal has own_scope
        && resource == principal.own_scope
};

permit (
    principal,
    action == Synveda::Action::"ToolWrite",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["curator", "member", "owner", "administrator"])
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
        && context.roles.containsAny(["reviewer", "owner", "administrator"])
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
        && context.roles.containsAny(["curator", "owner", "administrator"])
};

// Publishing is the curator's act by name (seed §5: curator "can
// pin/approve"), with owner and administrator above it. Personal scopes are
// deliberately NOT excluded here, unlike the content-role grants: the
// route additionally requires KnowledgeRead on every record it admits
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
        && context.roles.containsAny(["curator", "owner", "administrator"])
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
        && context.roles.containsAny(["curator", "owner", "administrator"])
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
        && context.roles.containsAny(["curator", "reviewer", "owner", "administrator"])
};

// At a scope, reading proposals mirrors this pack's KnowledgeRead shape — the
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
        && context.roles.containsAny(["viewer", "curator", "member", "reviewer", "owner", "administrator"])
};

// Opening a proposal grants nothing — it asks. So the floor is
// **neighbourhood**: a principal may propose at their own chain, and one
// step out from anything they hold — which is how tribal knowledge climbs
// the gradient without needing a grant at the target first (tech plan
// §2.3). Content keys reach the rest of the granted subtree below; viewer
// stays read-only by name.
//
// `principal.ambit` is what carries the climb since CPR-7 (ADR-0074
// decision 8). Until CPR-6 the floor read `principal in resource` alone
// and that was enough, because placement made every ancestor of your home
// an entity parent. Anchors are deliberately **not** parents now
// (ADR-0073) — authority flows down from a grant and never up — so
// `principal in resource` narrowed to your own chain and the climb this
// floor exists for stopped being sayable. The ambit is the honest
// replacement: the parent of every scope you hold, which is exactly one
// hop of the gradient and no more.
permit (
    principal,
    action == Synveda::Action::"ProposalOpen",
    resource
) when {
    principal in resource
        || (resource in principal.tenant && resource in principal.ambit)
};

permit (
    principal,
    action == Synveda::Action::"ProposalOpen",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["curator", "member", "owner", "administrator"])
};

// Reviewing is a role act with no membership floor: casting a verdict is
// authority, and `reviewer` is the key that carries it (ADR-0074 decision
// 6's re-vocabulary of security-reviewer). A viewer is excluded —
// read-only by name. Whether a given approval
// is *enough* is the matrix's arithmetic, never Cedar's (ADR-0032
// decision 5).
permit (
    principal,
    action == Synveda::Action::"ProposalReview",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["curator", "reviewer", "owner", "administrator"])
};

// Governed relaxations (CPR-31, ADR-0090): create, revision and revocation
// use one Policy/apply path and the same owner/administrator authority at the
// exact scope whose Knowledge may be disclosed.
permit (
    principal,
    action == Synveda::Action::"RelaxationWrite",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["owner", "administrator"])
};

// The directory plane (AUTH-4, ADR-0059 decision 13): who may wire this
// tenant up to an identity provider, and rotate the credential that does
// it. Pack-uniform, like the channel, quarantine and proposal planes and
// for their reason — how a directory is connected does not loosen per pack.
//
// Tenant-wide keys only — `owner` and `administrator`, which is what the
// old `org-admin` became (ADR-0074 decision 6). The point is the
// *exclusion* of a subtree key: a curator's authority is a subtree, and
// this credential is not anchored in one. It can place a joiner anywhere the mapping rules
// reach and seal anybody the directory says has left, so an authority over
// it that stopped at an org-unit boundary would be a fiction.
permit (
    principal,
    action == Synveda::Action::"DirectoryManage",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["owner", "administrator"])
};

// The breaker's release (AUTH-5, ADR-0060 decision 10). The same two keys
// here as well, and the separation is about what a *stored* pack can now express
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
        && context.roles.containsAny(["owner", "administrator"])
};

// Membership, to the role that reviews (CPR-5, ADR-0072). A curator casts
// verdicts on other people's work, and cannot do that without seeing whose
// work it is. `viewer` and `member` are deliberately left out under this
// pack: reading a scope's material does not require knowing who else can.
permit (
    principal,
    action == Synveda::Action::"MembershipRead",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["curator", "reviewer"])
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
