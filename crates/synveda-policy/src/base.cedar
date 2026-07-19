// The invariant base layer (ADR-0014 decision 2): prepended to every
// pack at compile time — embedded and stored custom packs alike — so no
// pack, however authored, can drop these rules.
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
