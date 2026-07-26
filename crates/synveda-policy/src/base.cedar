// The invariant base layer (ADR-0014 decision 2): prepended to every
// pack at compile time — embedded and stored custom packs alike — so no
// pack, however authored, can drop these rules.
//
// Until AUTHZ-4 this file held only forbids, and the sentence for it was
// "these are the things no pack can escape". It now also holds one permit,
// so the sentence is weaker: **these are the things no pack can change**
// (ADR-0037 decision 7). The permit is last, and every forbid above it
// still wins.
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
// principal holds: MemoryRead up the principal's own chain, so a team
// agent still composes team → department → org on inject. Users carry
// no token_scope; the `has` guard makes this a no-op for them.
forbid (
    principal,
    action,
    resource
) when {
    principal has token_scope &&
    !(resource is Synveda::Scope && resource in principal.token_scope) &&
    !(action == Synveda::Action::"MemoryRead" && principal in resource)
};

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
