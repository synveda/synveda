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
    CompositionConfig, HierarchyNode, Lapse, LapseId, PolicyAssignment, Result, RoleBinding,
    ScopeId, Sensitivity,
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
    /// The lapses standing over the caller (AUTHZ-4, ADR-0037): grants
    /// whose grantee scope is on `chain`, neither revoked nor expired as of
    /// the read that produced them.
    ///
    /// These reach *every* decision the walk takes, including the chain's:
    /// a grant naming a scope the caller is already under is redundant with
    /// the membership floor, not wrong.
    pub lapses: &'a [Lapse],
    /// The off-chain scopes those grants reach, each with the rows its own
    /// decision needs — `synveda_policy::lapsed_scopes` over `lapses`, with
    /// every target's chain and assignments resolved by the caller.
    ///
    /// These are the **only** source of off-chain candidates. A pack's own
    /// permits beyond the chain — bound subtrees, `standard`'s department —
    /// stay where ADR-0024 put them, on recall's deep-query surface: a
    /// permit cannot be enumerated, and a lapse is a row that names its
    /// target (ADR-0037 decision 13).
    pub lapsed: &'a [LapsedScope<'a>],
}

/// One off-chain scope a lapse reaches, with what deciding it costs
/// (ADR-0037 decision 10).
///
/// The chain and assignments are the caller's to resolve — this crate
/// touches no storage — and they are the honest price of the feature: a
/// lapsed scope is not on the caller's chain, so the rows the chain walk
/// already had do not cover it.
#[derive(Debug, Clone, Copy)]
pub struct LapsedScope<'a> {
    /// The grant that reaches this scope.
    pub lapse: &'a Lapse,
    /// The **target's** own chain, node-first, from the HIER-2 cache.
    pub chain: &'a [HierarchyNode],
    /// Pack assignments for that chain's nodes.
    pub assignments: &'a [PolicyAssignment],
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
            lapses: inputs.lapses,
            // The working tier, which is every tier the read path composed
            // before AUTHZ-5 (`inject` and `ComposeRequest::new` both asked
            // for `internal`). The per-tier walk — ask four times, keep the
            // answers as a set — lands with the read path's own change
            // (ADR-0038 decisions 1 and 3); until then this decides exactly
            // what it decided before, which is what keeps this a refactor.
            sensitivity: Some(Sensitivity::WORKING),
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
    /// Every chain scope's `MemoryRead` outcome, chain order (allowed
    /// and denied alike): what the walk decided, kept so the inject
    /// audit event aggregates decisions without re-deriving them
    /// (ADR-0026 decision 5; ADR-0019 decision 4). The per-call
    /// decision log remains the full-fidelity record.
    pub decisions: Vec<ScopeDecision>,
}

/// One scope's `MemoryRead` verdict as the plan walk decided it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeDecision {
    /// The scope decided.
    pub scope_id: ScopeId,
    /// Allow or deny.
    pub allowed: bool,
    /// The pack that decided.
    pub pack_name: String,
    /// The pack's version at decision time.
    pub pack_version: i64,
    /// The lapse that put this scope on the walk, when it was not the
    /// caller's own chain that did (AUTHZ-4, ADR-0037 decision 12).
    ///
    /// This is the audit chain's half of "why was that in the block": the
    /// `context.injected` event carries these decisions, so a grant that
    /// reached a reader is named in the same event as the material it
    /// reached them with.
    pub lapse: Option<LapseId>,
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
        lapses: inputs.lapses,
        sensitivity: Some(Sensitivity::WORKING),
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
    let mut decisions = Vec::with_capacity(inputs.chain.len());
    for (position, node) in inputs.chain.iter().enumerate() {
        let context = context_at(position);
        let decision = pdp.authorize(
            inputs.principal,
            Action::MemoryRead,
            Resource::Scope(node.id),
            &context,
        )?;
        decisions.push(ScopeDecision {
            scope_id: node.id,
            allowed: decision.allowed,
            pack_name: decision.pack_name,
            pack_version: decision.pack_version,
            lapse: None,
        });
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
            lapse: None,
        });
    }

    // The scopes a lapse reaches, **after** the chain (ADR-0037
    // decision 10): last in gradient order, so a lapsed record loses every
    // conflict against the reader's own material rather than winning one.
    //
    // Each is decided under its own chain and its own assignments — the
    // effective pack is a property of the resource (ADR-0014 decision 3),
    // and deciding a scope with somebody else's chain would fall back to
    // the tenant default and materialise an entity graph with no ancestry.
    for lapsed in inputs.lapsed {
        let target = lapsed.lapse.target_scope_id;
        let Some(node) = lapsed.chain.iter().find(|node| node.id == target) else {
            // A grant whose target the caller could not resolve: the scope
            // was deleted, or the chain arrived malformed. Plan nothing —
            // the same fail-closed reading an unplaced principal gets.
            continue;
        };
        let context = AuthzContext {
            scopes: lapsed.chain,
            principal_scopes: inputs.chain,
            assignments: lapsed.assignments,
            default_pack: inputs.default_pack,
            role_bindings: inputs.role_bindings,
            grant: None,
            lapses: inputs.lapses,
            // The working tier, which is every tier the read path composed
            // before AUTHZ-5 (`inject` and `ComposeRequest::new` both asked
            // for `internal`). The per-tier walk — ask four times, keep the
            // answers as a set — lands with the read path's own change
            // (ADR-0038 decisions 1 and 3); until then this decides exactly
            // what it decided before, which is what keeps this a refactor.
            sensitivity: Some(Sensitivity::WORKING),
        };
        let decision = pdp.authorize(
            inputs.principal,
            Action::MemoryRead,
            Resource::Scope(target),
            &context,
        )?;
        decisions.push(ScopeDecision {
            scope_id: target,
            allowed: decision.allowed,
            pack_name: decision.pack_name,
            pack_version: decision.pack_version,
            lapse: Some(lapsed.lapse.id),
        });
        if !decision.allowed {
            continue;
        }
        scopes.push(ComposeScope {
            scope_id: target,
            kind: node.kind,
            path: node.path.clone(),
            // A lapse admits what the target scope stands behind and
            // nothing else. Not the pack's channel rule: derived material
            // is unreviewed extraction output nobody at the target has
            // looked at, and it is the one thing the approvers could not
            // inspect before consenting (ADR-0037 decision 11).
            include_derived: false,
            lapse: Some(lapsed.lapse.id),
        });
    }

    let span = tracing::Span::current();
    span.record("permitted", scopes.len());
    span.record("budget", budget_tokens);
    Ok(CompositionPlan {
        scopes,
        budget_tokens,
        decisions,
    })
}
