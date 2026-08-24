//! The authz-derived predicate (CTX-1, ADR-0024 decision 1): the
//! caller's placement chain, each scope decided through the PDP's
//! `MemoryRead` seam — "CTX-1/2/3 ask exactly this question per
//! candidate scope" (the Cedar schema's recorded contract, AUTHZ-2
//! ADR-0014 decision 5).
//!
//! The candidate universe is the chain (seed §4.4's composition
//! contract: own scope outward to the tenant root) for `inject`, and that
//! plus
//! the scopes a request could actually draw from for `recall` (CTX-5,
//! ADR-0042 decision 2) — which is where the bound subtrees and
//! `standard`'s shared subtree finally become reachable, seven ADRs
//! after ADR-0024 option 2 parked them.
//!
//! Three sources of candidate, one decision per `(scope, tier)` for all
//! of them: the caller's chain, the scopes a lapse reaches (AUTHZ-4), and
//! recall's widened set. They differ only in where the scope came from and
//! what its channel rule is — never in how it is decided, because a second
//! way to decide would be a second answer to "may this caller see this
//! record" (seed §2.2).

use synveda_policy::{
    AuthzContext, EntityBatch, Pdp, PermittedTiers, Principal, Resource, ScopeNode,
};
use synveda_types::anchor::ScopeAnchor;
use synveda_types::{
    CompositionConfig, GroupId, Lapse, LapseId, PolicyAssignment, Result, ScopeId, ScopeTier,
    Sensitivity, TraceRetentionMode,
};

use crate::compose::ComposeScope;

/// What one chain sweep needs: the caller's principal and own chain
/// (node-first, governed scopes), plus the per-request rows the PDP
/// resolves packs and roles from — exactly what the gateway's decision
/// gathering already reads for the full chain. The PDP consults only rows
/// whose node is on the resource chain, so the full-chain rows serve every
/// suffix decision.
#[derive(Debug, Clone, Copy)]
pub struct MemoryReadInputs<'a> {
    /// Who is asking.
    pub principal: &'a Principal,
    /// The caller's own chain, nearest-first (own scope → tenant root).
    pub chain: &'a [ScopeNode],
    /// The caller's ordered anchors with the role keys they carry (CPR-6,
    /// ADR-0073) — the grants, direct and group-derived, that reach this
    /// caller. The composition sweep decides with them exactly as the
    /// admin plane does; before the hierarchy cutover this arrived empty
    /// because an anchor's scope could never be a node of the old
    /// hierarchy's chains, which is no longer true (CPR-7, ADR-0074).
    pub anchors: &'a [ScopeAnchor],
    /// The groups this caller is in, so a pack may name one.
    pub groups: &'a [GroupId],
    /// Pack assignments for the chain's nodes (missing rows inherit).
    pub assignments: &'a [PolicyAssignment],
    /// The tenant's stored default pack, if any.
    pub default_pack: Option<&'a str>,
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
    /// A lapse is a row that names its target (ADR-0037 decision 13),
    /// which is what made these enumerable before `candidates` existed.
    pub lapsed: &'a [LapsedScope<'a>],
    /// Recall's widened candidate set (CTX-5, ADR-0042 decision 2): the
    /// scopes that could contribute a record to *this* request, which is
    /// how a pack's own permits beyond the chain — granted subtrees,
    /// `standard`'s `principal.ambit` — finally get asked.
    ///
    /// **Empty for every `inject`**, which is what keeps ADR-0024
    /// decision 1's universe exactly where it was on the hot path. A scope
    /// already on `chain` or already reached by a lapse is skipped rather
    /// than decided twice: the nearer source wins, because it is the one
    /// that carries the gradient position and, for a lapse, the grant id
    /// the audit event names.
    pub candidates: &'a [CandidateScope<'a>],
}

/// One scope of recall's widened universe, with what deciding it costs
/// (CTX-5, ADR-0042 decisions 2 and 3).
///
/// Structurally a [`LapsedScope`] without the grant, and decided by the
/// same code — the difference is the channel rule. A lapse admits only
/// what its target stands behind (ADR-0037 decision 11); a widened
/// candidate is an ordinary pack grant, so it composes under that scope's
/// own channel rule exactly as a chain scope does.
#[derive(Debug, Clone, Copy)]
pub struct CandidateScope<'a> {
    /// The scope to decide.
    pub scope_id: ScopeId,
    /// That scope's own chain, node-first, from the scope closure.
    pub chain: &'a [ScopeNode],
    /// Pack assignments for that chain's nodes.
    pub assignments: &'a [PolicyAssignment],
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
    /// The **target's** own chain, node-first, from the scope closure.
    pub chain: &'a [ScopeNode],
    /// Pack assignments for that chain's nodes.
    pub assignments: &'a [PolicyAssignment],
}

/// The display path of every scope on a node-first chain, parallel to it:
/// the slugs from the root down to that scope, `/`-joined. Derived, never
/// stored — the closure is ground truth and a derived path cannot be stale
/// (CPR-3's doctrine, kept when the chain stopped carrying one).
fn chain_paths(chain: &[ScopeNode]) -> Vec<String> {
    let mut paths = Vec::with_capacity(chain.len());
    for index in 0..chain.len() {
        paths.push(
            chain[index..]
                .iter()
                .rev()
                .map(|node| node.slug.as_str())
                .collect::<Vec<_>>()
                .join("/"),
        );
    }
    paths
}

/// One chain sweep's path lookup: the path of `scope_id` on `chain`, or its
/// bare id when the chain did not name it (which the caller treats as
/// absent anyway).
fn path_of(chain: &[ScopeNode], paths: &[String], scope_id: ScopeId) -> String {
    chain
        .iter()
        .position(|node| node.id == scope_id)
        .and_then(|position| paths.get(position).cloned())
        .unwrap_or_else(|| scope_id.to_string())
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
    let batch = materialise(pdp, inputs)?;
    let chain_nodes: Vec<ScopeNode> = inputs.chain.to_vec();
    let mut permitted: Vec<ScopeTier> = Vec::with_capacity(inputs.chain.len());
    let mut scopes = 0usize;
    for (position, node) in inputs.chain.iter().enumerate() {
        let context = AuthzContext {
            scopes: &chain_nodes[position..],
            principal_scopes: &chain_nodes,
            anchors: inputs.anchors,
            groups: inputs.groups,
            resources: &[],
            assignments: inputs.assignments,
            default_pack: inputs.default_pack,
            lapses: inputs.lapses,
            // Named per tier inside the sweep (ADR-0038 decision 2).
            sensitivity: None,
        };
        // `permitted_chain_scopes` answers the memory question only: it is
        // the sweep the search legs and the recall gate stand on, and a
        // pack chunk reaches neither of those by this route.
        let tiers = permitted_tiers(pdp, &batch, inputs.principal, node.id, &context)?.memory;
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

/// One entity store for every chain this walk will decide (CTX-5,
/// ADR-0042 decision 6).
///
/// The cost of a decision is dominated by materialising Cedar's entity
/// store, not by evaluating against it: measured at 516 candidate scopes,
/// re-materialising per call put the plan stage at 378ms against
/// ADR-0029's 15ms allowance. Building it once per walk is what makes a
/// universe wider than the chain affordable, and it changes no verdict —
/// a superset store answers a request identically, because Cedar resolves
/// only the entities the request names.
///
/// Every chain the walk will touch has to be in here. A scope whose chain
/// is missing denies rather than mis-allows (the entities its permits test
/// membership against are simply absent), so the failure mode of getting
/// this wrong is a caller reading less than they should — never more.
fn materialise(pdp: &Pdp, inputs: &MemoryReadInputs<'_>) -> Result<EntityBatch> {
    let mut owned: Vec<Vec<ScopeNode>> =
        Vec::with_capacity(1 + inputs.lapsed.len() + inputs.candidates.len());
    owned.push(inputs.chain.to_vec());
    owned.extend(inputs.lapsed.iter().map(|lapsed| lapsed.chain.to_vec()));
    owned.extend(
        inputs
            .candidates
            .iter()
            .map(|candidate| candidate.chain.to_vec()),
    );
    let chains: Vec<&[ScopeNode]> = owned.iter().map(Vec::as_slice).collect();
    // `anchors` and `groups` matter here, not only on the later per-scope
    // decision: the Principal entity — with its `ambit`, `anchors` and
    // `private` attributes — is built once, at materialise time, and every
    // decision through this batch reuses it (`entities_over`,
    // `synveda-policy`). A default (empty) context here bakes an
    // ambit-less, group-less principal into the batch permanently — no
    // later per-scope `AuthzContext` can repair it, because those contexts
    // supply Cedar's request `context` map, not the entity itself. Found by
    // recall's own widened universe (CTX-5, ADR-0042 decision 2): a
    // `standard`-pack sharing permit that reads `principal.ambit` denied
    // every off-chain candidate, unconditionally, because the entity this
    // batch built never had one.
    let context = AuthzContext {
        principal_scopes: &owned[0],
        anchors: inputs.anchors,
        groups: inputs.groups,
        ..AuthzContext::default()
    };
    pdp.materialise(inputs.principal, &chains, &context)
}

/// One off-chain scope to decide, whatever put it on the walk.
#[derive(Debug, Clone, Copy)]
struct OffChain<'a> {
    scope_id: ScopeId,
    chain: &'a [ScopeNode],
    assignments: &'a [PolicyAssignment],
    /// The grant that reached it, when one did. `None` is recall's widened
    /// universe: an ordinary pack permit, which is the whole difference
    /// between the two off-chain sources.
    lapse: Option<LapseId>,
}

/// Decides one off-chain scope and appends what it produced.
///
/// The lapse loop and recall's widened sweep share this because they must:
/// two bodies would be two answers to "may this caller see this record",
/// and the second one would be the one nobody's leak suite covers
/// (ADR-0042 decision 3).
///
/// Each is decided under **its own** chain and assignments — the effective
/// pack is a property of the resource (ADR-0014 decision 3), and deciding a
/// scope with somebody else's chain would fall back to the tenant default
/// and materialise an entity graph with no ancestry.
fn plan_off_chain(
    pdp: &Pdp,
    batch: &EntityBatch,
    inputs: &MemoryReadInputs<'_>,
    scope: OffChain<'_>,
    decisions: &mut Vec<ScopeDecision>,
    scopes: &mut Vec<ComposeScope>,
) -> Result<()> {
    let target = scope.scope_id;
    let Some(node) = scope.chain.iter().find(|node| node.id == target) else {
        // A scope the caller could not resolve: deleted, or the chain
        // arrived malformed. Plan nothing — the same fail-closed reading an
        // unplaced principal gets.
        return Ok(());
    };
    let scope_nodes: Vec<ScopeNode> = scope.chain.to_vec();
    let principal_nodes: Vec<ScopeNode> = inputs.chain.to_vec();
    let context = AuthzContext {
        scopes: &scope_nodes,
        principal_scopes: &principal_nodes,
        anchors: inputs.anchors,
        groups: inputs.groups,
        resources: &[],
        assignments: scope.assignments,
        default_pack: inputs.default_pack,
        lapses: inputs.lapses,
        sensitivity: None,
    };
    // Per tier here too, and for a lapse this is where the grant's declared
    // ceiling shows up as a *smaller set*: the PDP sets `context.lapsed`
    // only at tiers at or below what the grant declared (ADR-0038
    // decision 6), so a working-tier grant plans the working tiers and a
    // restricted one plans all four.
    let permitted = permitted_tiers(pdp, batch, inputs.principal, target, &context)?;
    decisions.push(ScopeDecision {
        scope_id: target,
        allowed: !permitted.memory.is_empty(),
        sensitivities: permitted.memory.clone(),
        pack_name: permitted.decision.pack_name,
        pack_version: permitted.decision.pack_version,
        lapse: scope.lapse,
    });
    if permitted.memory.is_empty() {
        return Ok(());
    }
    let effective = permitted.effective;
    // The *target's* effective pack, not the reader's: what that scope
    // stands behind, under that scope's schedule (ADR-0040 decision 10) and
    // rendered by that scope's rules (ADR-0041 decision 11). It comes back
    // from the same resolution that just decided the scope, so planning one
    // scope walks its chain once rather than twice.
    scopes.push(ComposeScope {
        scope_id: target,
        kind: node.kind,
        path: path_of(scope.chain, &chain_paths(scope.chain), target),
        // A lapse admits what the target scope stands behind and nothing
        // else. Not the pack's channel rule: derived material is unreviewed
        // extraction output nobody at the target has looked at, and it is
        // the one thing the approvers could not inspect before consenting
        // (ADR-0037 decision 11).
        //
        // A widened candidate is not a grant anybody approved — it is the
        // pack's own permit, asked at last — so it composes under that
        // scope's channel rule exactly as a chain scope does.
        include_derived: scope.lapse.is_none() && effective.composition.channels.includes_derived(),
        sensitivities: permitted.memory,
        // A **lapse** admits what its target published as memory and
        // nothing else: ADR-0037 decision 11 bounded a grant to what its
        // approvers could inspect, and a lapse names a scope rather than a
        // bundle. A reader who should have another scope's conventions gets
        // them by being placed or bound, which is the decision
        // `ContextPackRead` already takes on the chain.
        //
        // A **widened candidate** is not a grant anybody approved — it is
        // the pack's own permit, asked at last — so its pack material
        // composes exactly as a chain scope's does. Same distinction
        // `include_derived` makes one line up, for the same reason.
        pack_sensitivities: if scope.lapse.is_some() {
            Vec::new()
        } else {
            permitted.context_pack
        },
        // The same distinction one line down, and for skills it is not even
        // reachable: a lapse admits what its target published as memory
        // (ADR-0037 decision 11), and skills are advertised on `inject`
        // only, which is the one path with no widened candidates at all
        // (ADR-0054 decision 13). Written out rather than left to that
        // coincidence, because the coincidence is CTX-5's to change.
        skill_sensitivities: if scope.lapse.is_some() {
            Vec::new()
        } else {
            permitted.skill
        },
        index_tier: effective.composition.index_tier,
        index_entry_chars: effective.composition.index_entry_chars,
        skill_index: effective.composition.skill_index,
        lapse: scope.lapse,
        retention: effective.retention,
    });
    Ok(())
}

/// One scope's allowed tier sets, ascending, the decision the pack
/// identity is read from, and that pack's configuration.
///
/// Every ask is a real PDP call: there is no short-circuit on the first
/// allow and no monotonicity assumption, so a pack that permits
/// `confidential` while denying `internal` gets exactly what it said rather
/// than what it probably meant (ADR-0038 decision 3).
///
/// All of them go through `permitted_read_tiers`, which resolves the pack
/// and the roles once and evaluates against a shared entity store (CTX-5,
/// ADR-0042 decision 6). Same verdicts, a fraction of the work — which is
/// what a universe wider than the chain costs when it is not done this way.
/// Since PRMT-2 that one resolution answers `ContextPackRead` too, which is
/// what keeps ADR-0050 decision 8's "the same walk, never a second
/// authorization path" true rather than aspirational.
fn permitted_tiers(
    pdp: &Pdp,
    batch: &EntityBatch,
    principal: &Principal,
    scope_id: ScopeId,
    context: &AuthzContext<'_>,
) -> Result<PermittedTiers> {
    pdp.permitted_read_tiers(batch, principal, scope_id, context)
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
    /// Governed amount of explainability retained for the whole run, from
    /// the same home-scope pack as the budget (CPR-20, ADR-0084).
    pub trace_retention: TraceRetentionMode,
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
    let batch = materialise(pdp, inputs)?;
    let chain_nodes: Vec<ScopeNode> = inputs.chain.to_vec();
    let paths = chain_paths(&chain_nodes);
    let tenant_id = inputs.principal.tenant_id;
    let context_at = |position: usize| AuthzContext {
        scopes: &chain_nodes[position..],
        principal_scopes: &chain_nodes,
        anchors: inputs.anchors,
        groups: inputs.groups,
        resources: &[],
        assignments: inputs.assignments,
        default_pack: inputs.default_pack,
        lapses: inputs.lapses,
        // Named per tier inside the sweep (ADR-0038 decision 2).
        sensitivity: None,
    };
    let home_composition = match inputs.chain.first() {
        Some(home) => {
            // The pack resolution is tier-blind — an effective pack is a
            // property of the resource (ADR-0014 decision 3) — so any tier
            // reads the same config.
            pdp.effective(tenant_id, Resource::Scope(home.id), &context_at(0))
                .composition
        }
        None => CompositionConfig::DEFAULT,
    };
    let budget_tokens = home_composition.budget_tokens;
    let trace_retention = home_composition.trace_retention;
    let mut scopes = Vec::with_capacity(inputs.chain.len());
    let mut decisions = Vec::with_capacity(inputs.chain.len());
    for (position, node) in inputs.chain.iter().enumerate() {
        let permitted = permitted_tiers(
            pdp,
            &batch,
            inputs.principal,
            node.id,
            &context_at(position),
        )?;
        decisions.push(ScopeDecision {
            scope_id: node.id,
            allowed: !permitted.memory.is_empty(),
            sensitivities: permitted.memory.clone(),
            pack_name: permitted.decision.pack_name,
            pack_version: permitted.decision.pack_version,
            lapse: None,
        });
        // A scope is planned when it admits *any* kind of material.
        // `ContextPackRead` non-empty while `MemoryRead` is empty is the
        // case packs exist for (ADR-0050 decision 8): a reader who holds no
        // readable memory at a scope still receives that scope's
        // conventions, and skipping the scope here would be the one place
        // that could quietly stop being true. Since SKIL-4 the same holds
        // of `SkillRead`, and more sharply — an org that publishes skills
        // and nothing else is an ordinary shape, and a reader who could not
        // see them there would be told about a capability they hold by
        // nothing at all (ADR-0054 decision 10).
        if permitted.memory.is_empty()
            && permitted.context_pack.is_empty()
            && permitted.skill.is_empty()
        {
            continue;
        }
        let effective = permitted.effective;
        // The channel rule comes from the same effective-pack resolution
        // that just decided the scope (ADR-0025 decision 2) — literally the
        // same one now, returned by the sweep rather than walked again.
        scopes.push(ComposeScope {
            scope_id: node.id,
            kind: node.kind,
            path: paths[position].clone(),
            include_derived: effective.composition.channels.includes_derived(),
            sensitivities: permitted.memory,
            // The tiers `ContextPackRead` permitted here — what admits this
            // scope's pack chunks, and nothing else does (ADR-0050
            // decision 8).
            pack_sensitivities: permitted.context_pack,
            // The tiers `SkillRead` permitted here — what admits this
            // scope's published skills into the block's advertisement, and
            // exactly what the resolve route decides when the same caller
            // asks for one of them by name (SKIL-4, ADR-0054 decision 2).
            skill_sensitivities: permitted.skill,
            // What this scope does with material that does not fit, from
            // the same resolution (CTX-4, ADR-0041 decision 11).
            index_tier: effective.composition.index_tier,
            index_entry_chars: effective.composition.index_entry_chars,
            // Whether this scope's skills are named at all (SKIL-4,
            // ADR-0054 decision 11).
            skill_index: effective.composition.skill_index,
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
        plan_off_chain(
            pdp,
            &batch,
            inputs,
            OffChain {
                scope_id: lapsed.lapse.target_scope_id,
                chain: lapsed.chain,
                assignments: lapsed.assignments,
                lapse: Some(lapsed.lapse.id),
            },
            &mut decisions,
            &mut scopes,
        )?;
    }

    // Recall's widened universe, last (CTX-5, ADR-0042 decision 2) and
    // empty for every inject. Scopes the chain or a lapse already planned
    // are skipped: deciding one twice would double-count it in the audit
    // event and could drop a lapse's grant id from the entry that names
    // why the reader could see it.
    let planned: Vec<ScopeId> = decisions.iter().map(|decision| decision.scope_id).collect();
    for candidate in inputs.candidates {
        if planned.contains(&candidate.scope_id) {
            continue;
        }
        plan_off_chain(
            pdp,
            &batch,
            inputs,
            OffChain {
                scope_id: candidate.scope_id,
                chain: candidate.chain,
                assignments: candidate.assignments,
                lapse: None,
            },
            &mut decisions,
            &mut scopes,
        )?;
    }

    let span = tracing::Span::current();
    span.record("permitted", scopes.len());
    span.record("budget", budget_tokens);
    Ok(CompositionPlan {
        scopes,
        budget_tokens,
        trace_retention,
        decisions,
    })
}
