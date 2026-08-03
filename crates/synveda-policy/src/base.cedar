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

// Escalation resistance (AUTHZ-3, ADR-0015 decision 5): granting or
// revoking org-admin requires holding org-admin, under every pack — a
// custom pack that forgot the guard must not let its permitted
// principals mint org-admins. `context.grant` is required by the schema,
// so this forbid can never be skipped for want of the attribute.
forbid (
    principal,
    action == Synveda::Action::"RoleAssign",
    resource
) when { context.grant == "org-admin" }
unless { context.roles.contains("org-admin") };

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
// conventions, in the same block, off the same walk.
//
// The carve-out stays what it was — the *membership* floor, `principal in
// resource`, reads only — so nothing outside the agent's own chain is
// reachable, no role widens it, and every write, every admin plane and
// every tenant-plane action remains forbidden outside the anchor.
forbid (
    principal,
    action,
    resource
) when {
    principal has token_scope &&
    !(resource is Synveda::Scope && resource in principal.token_scope) &&
    !([
        Synveda::Action::"MemoryRead",
        Synveda::Action::"PromptRead",
        Synveda::Action::"ContextPackRead"
    ].contains(action) && principal in resource)
};

// The read side of the `restricted` floor (AUTHZ-5, ADR-0038 decision 5).
//
// The product already has one non-negotiable rule about this tier, and it
// is on the write side: nothing reaches a published channel at `restricted`
// without the `compliance` role and two distinct approvers, under every
// pack, unauthorable away (ADR-0032 decision 4's invariant approval floor).
// This is its mirror. Nothing reaches a *reader* at `restricted` without a
// standing lapse — and because a lapse's approval matrix resolves at the
// tier it declares, the grant that lifts this forbid is one a compliance
// approver signed. One tier, one meaning, both directions, the same role.
//
// `context.sensitivity` is required by the schema, so this forbid can never
// be skipped for want of the attribute — the ADR-0015 decision 5 shape,
// applied to the rule that would be most dangerous to lose silently.
//
// There is deliberately no carve-out for the material's own author at their
// own personal scope. Under ADR-0038 decision 8 the only way a record
// carries this tier is a reclassification two people approved, one of them
// compliance, so what an owner loses here is access a compliance decision
// deliberately removed — and an owner exception would make the acceptance
// criterion false as written.
forbid (
    principal,
    action == Synveda::Action::"MemoryRead",
    resource
) when { context.sensitivity == "restricted" }
unless { context.lapsed };

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
// carves out only own-chain MemoryRead. `resource.kind != "user"` is
// ADR-0015 decision 4's privacy floor restated where it cannot be
// forgotten — nobody's personal memory is disclosed by a lapse, and the
// grant surface refuses such a target by name as well.
permit (
    principal,
    action == Synveda::Action::"MemoryRead",
    resource
) when {
    context.lapsed && resource.kind != "user"
};
