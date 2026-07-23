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
use synveda_types::{HierarchyNode, PolicyAssignment, Result, RoleBinding, ScopeId};

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
