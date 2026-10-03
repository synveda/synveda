permit (
    principal,
    action == Synveda::Action::"ScopeRead",
    resource
) when { resource in principal.tenant };

permit (
    principal,
    action == Synveda::Action::"ScopeUpdate",
    resource
) when { resource in principal.tenant };

permit (
    principal,
    action == Synveda::Action::"ConfigurationRead",
    resource
) when { resource in principal.tenant };

permit (
    principal,
    action == Synveda::Action::"ConfigurationWrite",
    resource
) when { resource in principal.tenant };

permit (
    principal,
    action == Synveda::Action::"ProposalOpen",
    resource
) when { resource in principal.tenant };
