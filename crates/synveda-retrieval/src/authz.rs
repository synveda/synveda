//! The authz-derived predicate (CTX-1, ADR-0024 decision 1): the
//! caller's placement chain, each scope decided through the PDP's
//! `MemoryRead` seam — "CTX-1/2/3 ask exactly this question per
//! candidate scope" (the Cedar schema's recorded contract, AUTHZ-2
//! ADR-0014 decision 5).
//!
//! The candidate universe is the chain (seed §4.4's composition
//! contract: user > team > department > org). Scopes packs permit
//! *beyond* the chain — bound subtrees, `standard`'s department subtree
//! — are recall's deep-query surface; CTX-5 owns enumerating a broader
//! universe (ADR-0024 option 2).

use synveda_policy::{Action, AuthzContext, Pdp, Principal, Resource};
use synveda_types::{
    CompositionConfig, HierarchyNode, PolicyAssignment, Result, RoleBinding, ScopeId,
};

use crate::compose::ComposeScope;

/// What one chain sweep needs: the caller's principal and placement
/// chain (node-first, from the HIER-2 cache), plus the per-request rows
/// the PDP resolves packs and roles from — exactly what the gateway's
/// decision gathering already reads for the full chain. The PDP
/// consults only rows whose node is on the resource chain, so the
/// full-chain rows serve every suffix decision.
#[derive(Debug, Clone, Copy)]
pub struct MemoryReadInputs<'a> {
    /// Who is asking.
    pub principal: &'a Principal,
    /// The caller's placement chain, nearest-first (personal scope →
    /// org root).
    pub chain: &'a [HierarchyNode],
    /// Pack assignments for the chain's nodes (missing rows inherit).
    pub assignments: &'a [PolicyAssignment],
    /// The tenant's stored default pack, if any.
    pub default_pack: Option<&'a str>,
    /// The subject's role bindings on the chain plus tenant-wide rows.
    pub role_bindings: &'a [RoleBinding],
}

/// The chain scopes the caller may compose memories from: one
/// `MemoryRead` decision per chain node under that node's effective
/// pack, its own chain suffix as the resource chain. Returns allowed
/// scopes in chain order (nearest-first — the gradient order CTX-2
/// composes in). ≤ hierarchy-depth decisions at µs each, prebuilt
/// entity fragments (HIER-3) inherited through the facade.
#[tracing::instrument(
    name = "retrieval.permitted_scopes",
    skip_all,
    fields(
        principal.subject = %inputs.principal.subject,
        chain.len = inputs.chain.len(),
        permitted = tracing::field::Empty,
    ),
    err(Display)
)]
pub fn permitted_chain_scopes(pdp: &Pdp, inputs: &MemoryReadInputs<'_>) -> Result<Vec<ScopeId>> {
    let mut permitted = Vec::with_capacity(inputs.chain.len());
    for (position, node) in inputs.chain.iter().enumerate() {
        let context = AuthzContext {
            scopes: &inputs.chain[position..],
            principal_scopes: inputs.chain,
            assignments: inputs.assignments,
            default_pack: inputs.default_pack,
            role_bindings: inputs.role_bindings,
            grant: None,
        };
        let decision = pdp.authorize(
            inputs.principal,
            Action::MemoryRead,
            Resource::Scope(node.id),
            &context,
        )?;
        if decision.allowed {
            permitted.push(node.id);
        }
    }
    tracing::Span::current().record("permitted", permitted.len());
    Ok(permitted)
}

/// A composition plan (CTX-2, ADR-0025 decision 1): the PDP-allowed
/// chain scopes in gradient order, each carrying its effective pack's
/// channel rule, plus the budget in force for the inject.
#[derive(Debug, Clone)]
pub struct CompositionPlan {
    /// Allowed scopes, nearest-first, with per-scope channel rules.
    pub scopes: Vec<ComposeScope>,
    /// The budget from the caller's home scope's effective pack —
    /// "per-scope configurable" resolves at the caller's placement
    /// (ADR-0025 decision 3).
    pub budget_tokens: u32,
}

/// The [`permitted_chain_scopes`] sweep extended with composition
/// config (ADR-0025 decision 1): one walk deciding `MemoryRead` per
/// chain scope and reading each allowed scope's effective pack for its
/// channel rule; the budget resolves at the chain head (the caller's
/// home scope) whether or not that scope itself composes. An empty
/// chain plans nothing under the default budget.
#[tracing::instrument(
    name = "retrieval.composition_plan",
    skip_all,
    fields(
        principal.subject = %inputs.principal.subject,
        chain.len = inputs.chain.len(),
        permitted = tracing::field::Empty,
        budget = tracing::field::Empty,
    ),
    err(Display)
)]
pub fn composition_plan(pdp: &Pdp, inputs: &MemoryReadInputs<'_>) -> Result<CompositionPlan> {
    let tenant_id = inputs.principal.tenant_id;
    let context_at = |position: usize| AuthzContext {
        scopes: &inputs.chain[position..],
        principal_scopes: inputs.chain,
        assignments: inputs.assignments,
        default_pack: inputs.default_pack,
        role_bindings: inputs.role_bindings,
        grant: None,
    };
    let budget_tokens = match inputs.chain.first() {
        Some(home) => {
            pdp.effective(tenant_id, Resource::Scope(home.id), &context_at(0))
                .composition
                .budget_tokens
        }
        None => CompositionConfig::DEFAULT.budget_tokens,
    };
    let mut scopes = Vec::with_capacity(inputs.chain.len());
    for (position, node) in inputs.chain.iter().enumerate() {
        let context = context_at(position);
        let decision = pdp.authorize(
            inputs.principal,
            Action::MemoryRead,
            Resource::Scope(node.id),
            &context,
        )?;
        if !decision.allowed {
            continue;
        }
        // The channel rule comes from the same effective-pack resolution
        // that just decided the scope (ADR-0025 decision 2).
        let effective = pdp.effective(tenant_id, Resource::Scope(node.id), &context);
        scopes.push(ComposeScope {
            scope_id: node.id,
            kind: node.kind,
            path: node.path.clone(),
            include_derived: effective.composition.channels.includes_derived(),
        });
    }
    let span = tracing::Span::current();
    span.record("permitted", scopes.len());
    span.record("budget", budget_tokens);
    Ok(CompositionPlan {
        scopes,
        budget_tokens,
    })
}
