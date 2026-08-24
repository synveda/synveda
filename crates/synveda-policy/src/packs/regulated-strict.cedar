// regulated-strict@21 — the default product pack (seed §6, ADR-0014,
// ADR-0015, ADR-0018, ADR-0020, ADR-0021, ADR-0031, ADR-0032, ADR-0036,
// ADR-0037, ADR-0038).
//
// Deny-first: composition reads the principal's own chain, plus whatever
// a content-role grant explicitly reaches — under this pack a viewer,
// member or curator grant *is* the seed's "explicit grant" for
// cross-scope read; AUTHZ-4 lapses add the time-boxed variant. Everything
// this pack does not name is denied by Cedar's default-deny, and the
// compiled-in base layer forbids sealed principals everything and holds
// the personal-scope privacy floor (ADR-0074 deleted the binding
// escalation guard with the action it guarded).
//
// Since AUTHZ-5 every MemoryRead permit also names the tiers it covers
// (ADR-0038 decision 4), and the three mechanisms are graded by how
// deliberate they are: membership reads the working tiers, an explicit
// content-role grant (or one's own scope) reaches `confidential`, and
// `restricted` is reachable by nothing this pack can say — the base layer
// forbids it outright unless a lapse that declared the tier stands, and such
// a lapse cleared the invariant floor's administrator requirement to exist.

// **The privacy floor moved.** Until CPR-6 every permit here restated
// `resource.kind != "user"`; it is now one base-layer forbid over
// `principal`-shaped scopes that no pack can drop and that a grant written
// *at* somebody's own scope can lift (ADR-0073 decision 6). So "personal
// scopes excluded" is still true of every permit below — it is just no longer
// each permit's job to remember, and sharing your own notes deliberately is
// finally sayable.

// The mutating admin planes: owner/administrator, over the granted
// subtree — which keys are effective here is resolved from the grants
// reaching the resource's chain (context.roles, ADR-0074 decision 6 over
// ADR-0015 decision 3), so a grant at a scope governs exactly its subtree.
permit (
    principal,
    action in [
        Synveda::Action::"ScopeCreate",
        Synveda::Action::"ScopeUpdate",
        Synveda::Action::"PolicyAssign",
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

// The read-only admin surfaces (seed §5 — read-only,
// including the audit chain since AUD-2, never content).
//
// `AuditRead` is the one action here that reaches only the tenant resource
// (ADR-0045 decision 2), so this permit grants it exactly where the chain
// lives and a subtree binding does not reach it.
permit (
    principal,
    action in [
        Synveda::Action::"ScopeRead",
        Synveda::Action::"PolicyRead",
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
        && context.roles.containsAny(["viewer", "curator", "member", "reviewer", "owner", "administrator"])
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
    principal has own_scope
        && resource == principal.own_scope
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
        && context.roles.containsAny(["viewer", "curator", "member", "reviewer", "owner", "administrator"])
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
    principal has own_scope
        && resource == principal.own_scope
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
        && context.roles.containsAny(["curator", "member", "owner", "administrator"])
};

// Stable Knowledge uses its own action and aggregate entity (CPR-16,
// ADR-0081). Scope decisions govern create/list; item decisions govern exact
// reads and immutable-revision mutations. The reach mirrors memory without
// inheriting its lapse override.
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
// Reading a run is **content**, not structure, so it is priced with the
// content reads and never with `ProjectRead`: a project's name discloses
// nothing, and a session's timeline is a transcript of what somebody and their
// agent did, said, read and changed.
//
// Under this pack that means the membership floor — your own chain, which is
// where your own `principal` scope's runs live — plus an explicit content
// grant, and nothing else. There is no `ambit` clause here, exactly as there
// is none for `MemoryRead`: this is the deny-first pack, and a neighbouring
// project's transcripts are not a default.
//
// It carries no tier. A session has no sensitivity of its own; what a run
// *produced* is classified when it becomes a candidate, on the other side of
// the trust boundary (ADR-0068 decision 6).
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

permit (
    principal,
    action == Synveda::Action::"SessionRead",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["viewer", "curator", "member", "reviewer", "owner", "administrator"])
};

// Opening, appending, composing and closing — one authority, because an agent
// that may open a run must be able to close it. Pack-uniform, like every other
// write floor: your own scope role-free — a person's own agent runs without
// anybody granting them anything — and a content key beyond it. `viewer` and
// `reviewer` are absent by name: reading somebody's runs is not running one.
//
// Composing context for a session additionally takes `MemoryRead` at every
// scope that contributes, decided by the same walk `inject` runs, so this
// permit admits no material of its own.
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
// Beyond your own chain it takes a governance key: `reviewer`, `owner` or
// `administrator`. A `member` may follow their project's runs and may not
// read everybody's prompts, which is the whole of what this action is for.

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
    principal has own_scope
        && resource == principal.own_scope
        && ["public", "internal", "confidential"].contains(context.sensitivity)
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

// Authoring mirrors the write floor: your own home role-free, the bound
// subtree with a content role, personal scopes excluded. A draft is not
// published material — the approval matrix decides that — so authoring is
// priced like writing rather than like publishing.
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
    principal has own_scope
        && resource == principal.own_scope
        && ["public", "internal", "confidential"].contains(context.sensitivity)
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

// Authoring mirrors PromptWrite, which mirrors the write floor. A draft is
// not published material — the approval matrix decides that, and for a pack
// under this pack it now decides it at two distinct people above a leaf
// (ADR-0050 decision 15).
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
    principal has own_scope
        && resource == principal.own_scope
        && ["public", "internal", "confidential"].contains(context.sensitivity)
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

// Authoring mirrors PromptWrite, which mirrors the write floor. A draft is
// not published material — the approval matrix decides that, and for a pack
// under this pack it now decides it at two distinct people above a leaf
// (ADR-0050 decision 15).
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
// declaration grants no invocation authority.
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
        && context.roles.containsAny(["curator", "owner", "administrator"])
};

permit (
    principal,
    action == Synveda::Action::"MemoryClassify",
    resource
) when {
    principal has own_scope
        && resource == principal.own_scope
};

// The quarantine review plane (MEM-2, ADR-0021 decision 6): owners,
// administrators and reviewers adjudicate quarantined observe events over
// the granted subtree — `reviewer`'s first live action. Pack-uniform,
// like the write grant: how a security control is reviewed does not
// loosen per pack. A viewer is excluded (quarantine rows are content,
// redacted or not). Since CPR-7 the plane is **tenant-anchored**
// (ADR-0074 decision 7): quarantined events land at `principal`-shaped
// scopes, which inherit nothing, so a verdict decided at the event's own
// scope would be reachable by nobody. The resource is the tenant, and
// the disclosure stays bounded by redaction with no self-release right
// for the owner.
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

// The lapse plane (AUTHZ-4, ADR-0037 decision 15): pack-uniform, like the
// channel, quarantine and proposal planes and for the same reason — how
// content crosses the trust boundary does not loosen per pack. What varies
// per pack is the *window* (the LapseConfig ceiling) and the *approvals*
// (the `policy` cell of the matrix, which has carried tech plan §2.4's lapse
// row since FLOW-3).
//
// Granting is the administrator's act (seed §6: "a steward may apply a
// scoped, reasoned, time-boxed override" — `steward` is `administrator`
// since ADR-0074 decision 6), with owner above it. Curator is deliberately
// absent: a lapse is policy, and ADR-0015 put the policy plane at that
// level and above. The resource is the scope whose material is disclosed,
// so an administrator granted at an org unit may open its subtree's
// material and may not open a sibling's.
permit (
    principal,
    action == Synveda::Action::"LapseGrant",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["owner", "administrator"])
};

// Revoking is wider than granting, which is the whole reason the two are
// separate actions: `reviewer` joins, because ending a disclosure early is
// incident response — the same key that adjudicates quarantined events
// (MEM-2) and reviews executable skills. Revocation installs nothing
// and can only narrow, so no matrix stands in front of it.
permit (
    principal,
    action == Synveda::Action::"LapseRevoke",
    resource
) when {
    resource in principal.tenant
        && context.roles.containsAny(["reviewer", "owner", "administrator"])
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
