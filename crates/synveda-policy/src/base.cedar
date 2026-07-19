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
