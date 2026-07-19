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
