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

use synveda_policy::{Action, AuthzContext, AuthzDecision, Pdp, Principal, Resource};
use synveda_types::{
    CompositionConfig, Error, HierarchyNode, Lapse, LapseId, PolicyAssignment, Result, RoleBinding,
    ScopeId, ScopeTier, Sensitivity,
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

/// The `(scope, tier)` pairs the caller may compose memories from: one
/// `MemoryRead` decision per chain node **per tier**, under that node's
/// effective pack, with its own chain suffix as the resource chain.
///
/// Four decisions per scope rather than one, because that is what makes a
/// per-record attribute decidable at a seam that holds no record: the tier
/// vocabulary is closed, so it can be enumerated before anything is fetched
/// (AUTHZ-5, ADR-0038 decision 1). The four asks at one scope differ in a
/// single context attribute and share their entity graph, so HIER-3's
/// cached fragments absorb most of the cost — but the decision count is
/// real, and the latency AC measures it.
///
/// Pairs come back in chain order (nearest-first — the gradient order CTX-2
/// composes in), ascending by tier within a scope. A scope that permits
/// nothing contributes no pairs, which is the fail-empty shape the whole
/// read path is built on.
#[tracing::instrument(
    name = "retrieval.permitted_scopes",
    skip_all,
    fields(
        principal.subject = %inputs.principal.subject,
        chain.len = inputs.chain.len(),
        permitted = tracing::field::Empty,
        pairs = tracing::field::Empty,
    ),
    err(Display)
)]
pub fn permitted_chain_scopes(pdp: &Pdp, inputs: &MemoryReadInputs<'_>) -> Result<Vec<ScopeTier>> {
    let mut permitted: Vec<ScopeTier> = Vec::with_capacity(inputs.chain.len());
    let mut scopes = 0usize;
    for (position, node) in inputs.chain.iter().enumerate() {
        let context = |sensitivity: Sensitivity| AuthzContext {
            scopes: &inputs.chain[position..],
            principal_scopes: inputs.chain,
            assignments: inputs.assignments,
            default_pack: inputs.default_pack,
            role_bindings: inputs.role_bindings,
            grant: None,
            lapses: inputs.lapses,
            sensitivity: Some(sensitivity),
        };
        let tiers = permitted_tiers(pdp, inputs.principal, node.id, context)?.0;
        if !tiers.is_empty() {
            scopes += 1;
        }
        permitted.extend(ScopeTier::expand(node.id, &tiers));
    }
    let span = tracing::Span::current();
    span.record("permitted", scopes);
    span.record("pairs", permitted.len());
    Ok(permitted)
}

/// One scope's allowed tier set, ascending, plus the decision the pack
/// identity is read from.
///
/// Every ask is a real PDP call: there is no short-circuit on the first
/// allow and no monotonicity assumption, so a pack that permits
/// `confidential` while denying `internal` gets exactly what it said rather
/// than what it probably meant (ADR-0038 decision 3, option 6 records the
/// upgrade if the decision count ever binds).
fn permitted_tiers<'a>(
    pdp: &Pdp,
    principal: &Principal,
    scope_id: ScopeId,
    context: impl Fn(Sensitivity) -> AuthzContext<'a>,
) -> Result<(Vec<Sensitivity>, AuthzDecision)> {
    let mut tiers = Vec::with_capacity(Sensitivity::ALL.len());
    let mut last: Option<AuthzDecision> = None;
    for tier in Sensitivity::ALL {
        let decision = pdp.authorize(
            principal,
            Action::MemoryRead,
            Resource::Scope(scope_id),
            &context(tier),
        )?;
        if decision.allowed {
            tiers.push(tier);
        }
        last = Some(decision);
    }
    // Four tiers are always asked, so this is always set; the pack that
    // decided is the same for all four (one resource, one resolution).
    let decision = last.ok_or_else(|| Error::Internal {
        message: "the sensitivity vocabulary is empty".to_owned(),
    })?;
    Ok((tiers, decision))
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
    /// Whether any tier at all composes here.
    pub allowed: bool,
    /// The tiers the walk permitted, ascending (AUTHZ-5, ADR-0038
    /// decision 13).
    ///
    /// The audit event carries this rather than a bare allow, because
    /// "what could this reader see at that scope in March" is a question
    /// about tiers, and it is the question a regulator actually asks about
    /// a `restricted` record.
    pub sensitivities: Vec<Sensitivity>,
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
    let context_at = |position: usize, sensitivity: Sensitivity| AuthzContext {
        scopes: &inputs.chain[position..],
        principal_scopes: inputs.chain,
        assignments: inputs.assignments,
        default_pack: inputs.default_pack,
        role_bindings: inputs.role_bindings,
        grant: None,
        lapses: inputs.lapses,
        sensitivity: Some(sensitivity),
    };
    let budget_tokens = match inputs.chain.first() {
        Some(home) => {
            // The pack resolution is tier-blind — an effective pack is a
            // property of the resource (ADR-0014 decision 3) — so any tier
            // reads the same config.
            pdp.effective(
                tenant_id,
                Resource::Scope(home.id),
                &context_at(0, Sensitivity::WORKING),
            )
            .composition
            .budget_tokens
        }
        None => CompositionConfig::DEFAULT.budget_tokens,
    };
    let mut scopes = Vec::with_capacity(inputs.chain.len());
    let mut decisions = Vec::with_capacity(inputs.chain.len());
    for (position, node) in inputs.chain.iter().enumerate() {
        let (sensitivities, decision) = permitted_tiers(pdp, inputs.principal, node.id, |tier| {
            context_at(position, tier)
        })?;
        decisions.push(ScopeDecision {
            scope_id: node.id,
            allowed: !sensitivities.is_empty(),
            sensitivities: sensitivities.clone(),
            pack_name: decision.pack_name,
            pack_version: decision.pack_version,
            lapse: None,
        });
        if sensitivities.is_empty() {
            continue;
        }
        // The channel rule comes from the same effective-pack resolution
        // that just decided the scope (ADR-0025 decision 2).
        let effective = pdp.effective(
            tenant_id,
            Resource::Scope(node.id),
            &context_at(position, Sensitivity::WORKING),
        );
        scopes.push(ComposeScope {
            scope_id: node.id,
            kind: node.kind,
            path: node.path.clone(),
            include_derived: effective.composition.channels.includes_derived(),
            sensitivities,
            // What this scope does with material that does not fit, from
            // the same resolution (CTX-4, ADR-0041 decision 11).
            index_tier: effective.composition.index_tier,
            index_entry_chars: effective.composition.index_entry_chars,
            lapse: None,
            // The horizons this scope *serves* under, from the same
            // resolution (MEM-6, ADR-0040 decision 10). Nothing is stamped
            // on a record, so a pack applied a second ago governs the
            // block this walk is planning.
            retention: effective.retention,
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
        // Per tier here too, and this is where the grant's declared ceiling
        // shows up as a *smaller set*: the PDP sets `context.lapsed` only
        // at tiers at or below what the grant declared (ADR-0038
        // decision 6), so a working-tier grant plans the working tiers and
        // a restricted one plans all four.
        let (sensitivities, decision) =
            permitted_tiers(pdp, inputs.principal, target, |tier| AuthzContext {
                scopes: lapsed.chain,
                principal_scopes: inputs.chain,
                assignments: lapsed.assignments,
                default_pack: inputs.default_pack,
                role_bindings: inputs.role_bindings,
                grant: None,
                lapses: inputs.lapses,
                sensitivity: Some(tier),
            })?;
        decisions.push(ScopeDecision {
            scope_id: target,
            allowed: !sensitivities.is_empty(),
            sensitivities: sensitivities.clone(),
            pack_name: decision.pack_name,
            pack_version: decision.pack_version,
            lapse: Some(lapsed.lapse.id),
        });
        if sensitivities.is_empty() {
            continue;
        }
        // The *target's* effective pack, not the reader's: a lapse
        // discloses what that scope stands behind, under that scope's
        // schedule (ADR-0040 decision 10) and rendered by that scope's
        // rules (ADR-0041 decision 11).
        let effective = pdp.effective(
            tenant_id,
            Resource::Scope(target),
            &AuthzContext {
                scopes: lapsed.chain,
                principal_scopes: inputs.chain,
                assignments: lapsed.assignments,
                default_pack: inputs.default_pack,
                role_bindings: inputs.role_bindings,
                grant: None,
                lapses: inputs.lapses,
                sensitivity: Some(Sensitivity::WORKING),
            },
        );
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
            sensitivities,
            index_tier: effective.composition.index_tier,
            index_entry_chars: effective.composition.index_entry_chars,
            lapse: Some(lapsed.lapse.id),
            retention: effective.retention,
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
