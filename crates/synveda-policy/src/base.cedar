// The invariant base layer (ADR-0014 decision 2): prepended to every
// pack at compile time — embedded and stored custom packs alike — so no
// pack, however authored, can drop these rules.
//
// Until AUTHZ-4 this file held only forbids, and the sentence for it was
// "these are the things no pack can escape". It now also holds one permit,
// so the sentence is weaker: **these are the things no pack can change**
// (ADR-0037 decision 7). The permit is last, and every forbid above it
// still wins — including AUTHZ-5's `restricted` forbid, whose one `unless`
// is the same grant the permit turns on, so the two read as one rule about
// lapses seen from either end (ADR-0038 decision 5).
//
// Quarantined principals may do nothing (AUTH-2, ADR-0013 decision 5):
// a forbid overrides any permit a pack adds, current or future.
forbid (
    principal,
    action,
    resource
) when { principal.quarantined };

// A sealed scope is nobody's to act on (AUTH-4, ADR-0059 decision 8).
//
// The mirror of the quarantine forbid above, and the pairing is the whole
// design: quarantine says *this caller* may do nothing, a seal says nothing
// may be done to *this material*. A person's personal scope is sealed when
// the directory says they have left, and from that moment no pack, no role
// and no lapse reaches it — the lapse permit at the bottom of this file
// could not anyway, because its `resource.kind != "principal"` privacy floor
// (ADR-0015 decision 4) already excludes every personal scope, sealed or
// live.
//
// It forbids **every** action rather than reads alone, and the write half
// is the one that would be missed: a departed person's scope that still
// accepted observations would keep growing under a retention hold, and an
// extraction worker draining a queue after somebody's last day would be
// writing memory for a person the directory has already said is gone.
//
// There is deliberately no carve-out for compliance, legal or an
// administrator. Adding a reader here is adding this product's first "read
// somebody else's private material" surface, and that belongs to the
// export plane with its own ADR and its own approval matrix — not to the
// feature that handles somebody resigning (ADR-0059 decision 8).
//
// `sealed` is required by the schema, so this forbid can never be skipped
// for want of the attribute — the ADR-0015 decision 5 shape, applied to
// the rule whose silent absence would be least visible: everything would
// simply keep working.
forbid (
    principal,
    action,
    resource
) when { resource is Synveda::Scope && resource.sealed };

// (The role-binding escalation guard ADR-0015 decision 5 put here —
// `RoleAssign` of `org-admin` requiring `org-admin` — is deleted with the
// action it guarded: role bindings left with the old hierarchy, and the
// grant plane's own revocation rule names the grant it is about, which is
// the stronger form of the same resistance. CPR-7, ADR-0074.)

// Token-scope confinement (AUTH-3, ADR-0018 decision 4): a service
// identity's token is confined to its anchor subtree — everything
// outside it is forbidden *regardless of bound roles*, so a leaked or
// over-privileged agent credential cannot escape, and tenant-plane
// resources (never inside any scope subtree) are unreachable outright.
// The one carve-out is the role-free membership floor every placed
// principal holds: reading up the principal's own chain, so a team
// agent still composes team → department → org on inject. Users carry
// no token_scope; the `has` guard makes this a no-op for them.
//
// PRMT-1 (ADR-0049 decision 4) puts `PromptRead` in that carve-out, and it
// is the same decision rather than a second one: a headless agent is the
// consumer prompts exist for, and the org's `house-style` is exactly the
// material an anchored token must still resolve. PRMT-2 (ADR-0050
// decision 8) puts `ContextPackRead` beside it for a stronger version of
// the same reason: pack material composes through `inject`, which a
// team-anchored agent already reaches its own chain with — leaving it out
// would mean an agent composed the org's memory and not the org's
// conventions, in the same block, off the same walk. SKIL-1 (ADR-0051
// decision 10) puts `SkillRead` beside them on the plainest version of it:
// a headless agent that cannot resolve the org's skills is an agent that
// cannot do the work the skills were published for. CPR-25/ADR-0086 applies
// the same confinement to `ToolRead`: an adapter resolves only catalogue
// metadata inside its anchor, and a declaration still grants no invocation.
//
// The carve-out stays what it was — the *membership* floor, `principal in
// resource`, reads only — so nothing outside the agent's own chain is
// reachable, no role widens it, and every write, every admin plane and
// every tenant-plane action remains forbidden outside the anchor.
//
// CPR-6 drops the `resource is Synveda::Scope` guard the containment test used
// to carry, and that is a widening of the *forbid* rather than of anything an
// agent may do: a workspace, a project and a grant are now entities parented
// to the scope they belong to, so `resource in principal.token_scope` is true
// for exactly the ones inside the anchor and false for everything else —
// including the Tenant and Group entities, which sit in no subtree at all and
// are therefore forbidden outright, which is what the old guard achieved by
// naming the type.
forbid (
    principal,
    action,
    resource
) when {
    principal has token_scope &&
    !(resource in principal.token_scope) &&
    !([
        Synveda::Action::"MemoryRead",
        Synveda::Action::"KnowledgeRead",
        Synveda::Action::"PromptRead",
        Synveda::Action::"ContextPackRead",
        Synveda::Action::"SkillRead",
        Synveda::Action::"ToolRead"
    ].contains(action) && principal in resource)
};

// **Personal principal-scope privacy** (CPR-6, ADR-0073 decision 6).
//
// A `principal`-shaped scope is somebody's own. Nothing above it reaches in:
// not a tenant-root grant, not an org unit, not a workspace owner, and no
// role — which is what makes "my own notes" a thing this product can hold at
// all for a person who is also a member of an organisation.
//
// It is a base-layer forbid rather than a rule each pack restates, because the
// property it protects is the one a custom pack would be most tempted to
// widen: an administrator who could read every private scope is one line of
// Cedar away in a product that leaves this to the packs. `synveda-store`'s
// resolver applies the same floor when it builds an anchor set
// (`synveda_types::access::inherits_into`), so a grant at an ancestor never
// even produces roles here. Both, deliberately: the resolver decides what may
// be *offered*, this decides what may be *permitted*.
//
// `principal.private` is the whole of the escape hatch and it is the PDP's own
// resolution, never a caller's claim: the caller's own scope, plus any
// principal scope somebody wrote a grant **directly at**. An inherited grant
// is not in it, because an inherited grant did not reach here.
//
// There is deliberately no carve-out for compliance, legal or an
// administrator **on the material**. Adding a reader here is adding this
// product's first "read somebody else's private material" surface, and that
// belongs to an export plane with its own ADR and its own approval matrix
// (ADR-0059 decision 8's argument, one model later).
//
// What *is* carved out is the **governance plane**, and the list is
// deliberately short and deliberately closed: reading and assigning a policy
// profile, and the structural and service-identity planes. None of them
// discloses a byte of the material and none of them confers access to it —
// which is exactly why `MembershipGrant` is **not** here. The quarantine
// review plane is **not** here either, and did not need to be: since CPR-7
// it decides at the tenant resource rather than at the private scope its
// events land on (ADR-0074 decision 7). A grant written at
// a private scope puts that scope in the grantee's `private` set, so an
// administrator who could write one could grant themselves everybody's
// notes, and this floor would be a formality. The person whose scope it is
// holds it in their own `private` set, so they can share it; nobody else
// can. (The role-binding read the list once carried left with the
// role-binding plane. CPR-7, ADR-0074.)
//
// The shape is "forbid everything, then name the exceptions" rather than
// "forbid a list of content actions", because the failure modes are not
// symmetric: a content action added to the vocabulary and forgotten here
// would silently become readable, and a governance action forgotten here is
// merely refused until somebody notices.
forbid (
    principal,
    action,
    resource
) when { resource is Synveda::Scope && resource.kind == "principal" }
unless { resource in principal.private }
unless {
    [
        Synveda::Action::"PolicyRead",
        Synveda::Action::"PolicyAssign",
        Synveda::Action::"ScopeRead",
        Synveda::Action::"ScopeUpdate",
        Synveda::Action::"ServiceIdentityRead",
        Synveda::Action::"ServiceIdentityManage"
    ].contains(action)
};

// The read side of the `restricted` floor (AUTHZ-5, ADR-0038 decision 5).
//
// The product already has one non-negotiable rule about this tier, and it
// is on the write side: nothing reaches a published channel at `restricted`
// without an administrator and two distinct approvers, under every pack,
// unauthorable away (ADR-0032 decision 4's invariant approval floor, on the
// grant-key vocabulary since CPR-7). This is its mirror. Nothing reaches a
// *reader* at `restricted` without a standing lapse — and because a lapse's
// approval matrix resolves at the tier it declares, the grant that lifts
// this forbid is one an administrator signed. One tier, one meaning, both
// directions, the same role.
//
// `context.sensitivity` is required by the schema, so this forbid can never
// be skipped for want of the attribute — the ADR-0015 decision 5 shape,
// applied to the rule that would be most dangerous to lose silently.
//
// There is deliberately no carve-out for the material's own author at their
// own personal scope. Under ADR-0038 decision 8 the only way a record
// carries this tier is a reclassification two people approved, one of them
// an administrator, so what an owner loses here is access a governance
// decision deliberately removed — and an owner exception would make the acceptance
// criterion false as written.
forbid (
    principal,
    action == Synveda::Action::"MemoryRead",
    resource
) when { context.sensitivity == "restricted" }
unless { context.lapsed };

// Knowledge does not inherit the retired lapse override. Until the governed
// relaxation successor explicitly extends the new vocabulary, restricted
// Knowledge is not readable under any pack.
forbid (
    principal,
    action == Synveda::Action::"KnowledgeRead",
    resource
) when { context.sensitivity == "restricted" };

// Lapses (AUTHZ-4, ADR-0037 decision 7): the product's one base-layer
// *permit*, and it is here rather than in each pack because a lapse is an
// override. A pack that could neutralise it by not mentioning it would make
// "lapse" mean different things in different tenants, which is the
// trustworthiness rule that reserved the product pack names (ADR-0014
// decision 6).
//
// `context.lapsed` is the PDP's own resolution and is never a caller's
// claim: it is true only when a standing grant row covers exactly this
// principal (placed at or under the grantee scope), this action, and this
// resource — and only when the pack in force at the resource still admits
// lapses at all. So a pack refuses the mechanism outright by setting its
// lapse ceiling to zero (decision 5), visibly, in configuration.
//
// A pack that wants to refuse it for a *subtree* writes its own forbid;
// Cedar honours that over any permit, which is exactly the escape hatch a
// base-layer permit needs to be safe.
//
// What still forbids over this permit is the reason it is safe to put here:
// a quarantined principal is unaffected, and a service identity cannot be
// widened past its anchor, because ADR-0018's confinement forbid above
// carves out only own-chain MemoryRead. `resource.kind != "principal"` is
// ADR-0015 decision 4's privacy floor restated where it cannot be
// forgotten — nobody's personal memory is disclosed by a lapse, and the
// grant surface refuses such a target by name as well. Since CPR-6 the
// forbid above says the same thing for every action, so this clause is the
// narrower of two statements of one rule rather than the only one.
permit (
    principal,
    action == Synveda::Action::"MemoryRead",
    resource
) when {
    context.lapsed && resource.kind != "principal"
};
