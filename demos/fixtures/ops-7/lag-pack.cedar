permit (
    principal,
    action == Synveda::Action::"ScopeRead",
    resource
) when { resource in principal.tenant };
