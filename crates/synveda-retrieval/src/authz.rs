//! PDP-derived planning for authored context (CPR-43).
//!
//! Knowledge candidates are authorised individually by the gateway. This
//! module performs the distinct bounded walk for context packs and Skill
//! advertisements: one tier-aware PDP sweep per candidate scope, using that
//! resource's own chain and effective policy pack. The resulting plan only
//! narrows; the composition module has no authority to add a scope or tier.

use synveda_policy::{AuthzContext, EntityBatch, Pdp, Principal, Resource, ScopeNode};
use synveda_types::anchor::ScopeAnchor;
use synveda_types::{
    CompositionConfig, GroupId, PolicyAssignment, Result, ScopeId, Sensitivity, TraceRetentionMode,
};

use crate::compose::ComposeScope;

/// Inputs for one authored-context policy walk.
#[derive(Debug, Clone, Copy)]
pub struct AuthoredReadInputs<'a> {
    /// Asking principal.
    pub principal: &'a Principal,
    /// Principal's own scope chain, nearest first.
    pub chain: &'a [ScopeNode],
    /// Effective scope anchors and role keys.
    pub anchors: &'a [ScopeAnchor],
    /// Groups the principal belongs to.
    pub groups: &'a [GroupId],
    /// Policy assignments visible on the chain.
    pub assignments: &'a [PolicyAssignment],
    /// Tenant default policy-pack name.
    pub default_pack: Option<&'a str>,
    /// Additional session/project scopes outside the principal chain.
    pub candidates: &'a [CandidateScope<'a>],
}

/// One additional scope with its own chain and policy assignments.
#[derive(Debug, Clone, Copy)]
pub struct CandidateScope<'a> {
    /// Scope to decide.
    pub scope_id: ScopeId,
    /// Scope's own nearest-first chain.
    pub chain: &'a [ScopeNode],
    /// Assignments applying on that chain.
    pub assignments: &'a [PolicyAssignment],
}

/// PDP-derived authored-context plan.
#[derive(Debug, Clone)]
pub struct CompositionPlan {
    /// Authorised scopes in gradient order.
    pub scopes: Vec<ComposeScope>,
    /// Effective home-scope token budget.
    pub budget_tokens: u32,
    /// Effective trace-retention mode.
    pub trace_retention: TraceRetentionMode,
    /// Every considered scope's outcome, including denied scopes.
    pub decisions: Vec<ScopeDecision>,
}

/// One scope's tier-aware authored-context outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeDecision {
    /// Scope decided.
    pub scope_id: ScopeId,
    /// Whether any authored family was permitted.
    pub allowed: bool,
    /// Context-pack tiers permitted.
    pub context_pack_sensitivities: Vec<Sensitivity>,
    /// Skill tiers permitted.
    pub skill_sensitivities: Vec<Sensitivity>,
    /// Policy pack that decided.
    pub pack_name: String,
    /// Policy-pack version that decided.
    pub pack_version: i64,
}

#[derive(Debug, Clone, Copy)]
struct PlannedScope<'a> {
    scope_id: ScopeId,
    chain: &'a [ScopeNode],
    assignments: &'a [PolicyAssignment],
}

/// Builds a plan without reading any authored content.
#[tracing::instrument(
    name = "retrieval.authored_context_plan",
    skip_all,
    fields(principal.subject = %inputs.principal.subject, chain.len = inputs.chain.len()),
    err(Display)
)]
pub fn composition_plan(pdp: &Pdp, inputs: &AuthoredReadInputs<'_>) -> Result<CompositionPlan> {
    let batch = materialise(pdp, inputs)?;
    let tenant_id = inputs.principal.tenant_id;
    let home_context = context(inputs, inputs.chain, inputs.assignments, 0);
    let home = inputs
        .chain
        .first()
        .map_or(CompositionConfig::DEFAULT, |scope| {
            pdp.effective(tenant_id, Resource::Scope(scope.id), &home_context)
                .composition
        });
    let mut scopes = Vec::new();
    let mut decisions = Vec::new();
    let mut seen = Vec::new();

    for (position, node) in inputs.chain.iter().enumerate() {
        plan_scope(
            pdp,
            &batch,
            inputs,
            PlannedScope {
                scope_id: node.id,
                chain: &inputs.chain[position..],
                assignments: inputs.assignments,
            },
            &mut scopes,
            &mut decisions,
        )?;
        seen.push(node.id);
    }
    for candidate in inputs.candidates {
        if seen.contains(&candidate.scope_id) {
            continue;
        }
        plan_scope(
            pdp,
            &batch,
            inputs,
            PlannedScope {
                scope_id: candidate.scope_id,
                chain: candidate.chain,
                assignments: candidate.assignments,
            },
            &mut scopes,
            &mut decisions,
        )?;
        seen.push(candidate.scope_id);
    }

    tracing::Span::current().record("permitted", scopes.len());
    Ok(CompositionPlan {
        scopes,
        budget_tokens: home.budget_tokens,
        trace_retention: home.trace_retention,
        decisions,
    })
}

fn plan_scope(
    pdp: &Pdp,
    batch: &EntityBatch,
    inputs: &AuthoredReadInputs<'_>,
    planned: PlannedScope<'_>,
    scopes: &mut Vec<ComposeScope>,
    decisions: &mut Vec<ScopeDecision>,
) -> Result<()> {
    let Some(node) = planned
        .chain
        .iter()
        .find(|node| node.id == planned.scope_id)
    else {
        return Ok(());
    };
    let request_context = context(inputs, planned.chain, planned.assignments, 0);
    let permitted =
        pdp.permitted_read_tiers(batch, inputs.principal, planned.scope_id, &request_context)?;
    let allowed = !permitted.context_pack.is_empty() || !permitted.skill.is_empty();
    decisions.push(ScopeDecision {
        scope_id: planned.scope_id,
        allowed,
        context_pack_sensitivities: permitted.context_pack.clone(),
        skill_sensitivities: permitted.skill.clone(),
        pack_name: permitted.effective.name.clone(),
        pack_version: permitted.effective.version,
    });
    if !allowed {
        return Ok(());
    }
    scopes.push(ComposeScope {
        scope_id: planned.scope_id,
        kind: node.kind,
        path: chain_path(planned.chain, planned.scope_id),
        pack_sensitivities: permitted.context_pack,
        skill_sensitivities: permitted.skill,
        summary_chars: permitted.effective.composition.summary_chars,
        skill_index: permitted.effective.composition.skill_index,
    });
    Ok(())
}

fn context<'a>(
    inputs: &'a AuthoredReadInputs<'a>,
    scopes: &'a [ScopeNode],
    assignments: &'a [PolicyAssignment],
    position: usize,
) -> AuthzContext<'a> {
    AuthzContext {
        scopes: &scopes[position..],
        principal_scopes: inputs.chain,
        anchors: inputs.anchors,
        groups: inputs.groups,
        resources: &[],
        assignments,
        default_pack: inputs.default_pack,
        relaxations: &[],
        sensitivity: None,
    }
}

fn materialise(pdp: &Pdp, inputs: &AuthoredReadInputs<'_>) -> Result<EntityBatch> {
    let mut owned = Vec::with_capacity(1 + inputs.candidates.len());
    owned.push(inputs.chain.to_vec());
    owned.extend(
        inputs
            .candidates
            .iter()
            .map(|candidate| candidate.chain.to_vec()),
    );
    let chains: Vec<&[ScopeNode]> = owned.iter().map(Vec::as_slice).collect();
    let materialise_context = AuthzContext {
        principal_scopes: inputs.chain,
        anchors: inputs.anchors,
        groups: inputs.groups,
        ..AuthzContext::default()
    };
    pdp.materialise(inputs.principal, &chains, &materialise_context)
}

fn chain_path(chain: &[ScopeNode], scope_id: ScopeId) -> String {
    let Some(position) = chain.iter().position(|node| node.id == scope_id) else {
        return scope_id.to_string();
    };
    chain[position..]
        .iter()
        .rev()
        .map(|node| node.slug.as_str())
        .collect::<Vec<_>>()
        .join("/")
}
