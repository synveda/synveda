//! The gateway's side of the PDP (AUTHZ-1 ADR-0012; AUTHZ-2 ADR-0014):
//! the enforcement helper handlers call before acting, and the policy pack
//! refresher that hot-swaps stored per-tenant packs into the embedded
//! engine.
//!
//! Layering (seed §2.4): `synveda-policy` never touches storage, so this
//! module carries the data between them — hierarchy rows and pack
//! assignments into [`synveda_policy::AuthzContext`], stored pack sources
//! into [`Pdp::install_source`].

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgConnection;
use synveda_identity::Claims;
use synveda_policy::{Action, AuthzContext, AuthzDecision, Pdp, Principal, Resource};
use synveda_store::{
    identities, lapses, policy_assignments, policy_packs, rls, role_bindings, tenants,
};
use synveda_types::{
    Error, HierarchyNode, Identity, IdentityKind, Lapse, LapseAction, Result, Role, RoleBinding,
    ScopeId, Sensitivity, TenantId,
};

use crate::app::AppState;
use crate::telemetry::{POLICY_PACK_RELOADS_TOTAL, SERVICE_TOKEN_REJECTIONS_TOTAL};

/// Everything [`require`] assembles for one decision: the principal and
/// the caller-supplied data the PDP resolves and materialises from.
/// Handlers that need more than a verdict (the policy routes display the
/// effective pack) build this too, through [`gather`].
pub(crate) struct DecisionInput {
    pub(crate) principal: Principal,
    pub(crate) chain: Arc<[HierarchyNode]>,
    pub(crate) principal_scopes: Arc<[HierarchyNode]>,
    pub(crate) assignments: Vec<synveda_types::PolicyAssignment>,
    pub(crate) default_pack: Option<String>,
    pub(crate) role_bindings: Vec<RoleBinding>,
    /// The lapses standing over this caller, as of this request's own read
    /// (AUTHZ-4, ADR-0037 decision 4): grants whose grantee scope is on the
    /// caller's placement chain, neither revoked nor expired.
    ///
    /// **This is where expiry happens.** Nothing runs to end a lapse; the
    /// predicate on this one query does, so a sweep that is down cannot
    /// leave a grant standing.
    pub(crate) lapses: Vec<Lapse>,
    /// The caller's identity row, already read for the principal — so
    /// handlers whose resource *is* the placement (observe, MEM-1) never
    /// read it twice. `None` for verified subjects with no identity.
    pub(crate) identity: Option<Identity>,
}

impl DecisionInput {
    pub(crate) fn context(&self) -> AuthzContext<'_> {
        self.context_from(0)
    }

    /// The decision context for a resource whose own chain is the
    /// gathered chain from `position` onwards — an *ancestor* of the node
    /// this input was gathered at.
    ///
    /// A cross-scope promotion decides at two scopes (FLOW-5, ADR-0034
    /// decision 12): `MemoryRead` at the source and `ProposalOpen` at an
    /// ancestor of it. Gathering at the deeper node reads assignments and
    /// role bindings for a chain that already contains the ancestor's, so
    /// the second decision needs a slice rather than a second gather —
    /// the same trick `permitted_chain_scopes` uses to decide a whole
    /// chain from one set of rows. A position past the end yields the
    /// empty chain, which fails closed.
    pub(crate) fn context_from(&self, position: usize) -> AuthzContext<'_> {
        AuthzContext {
            scopes: self.chain.get(position..).unwrap_or(&[]),
            principal_scopes: &self.principal_scopes,
            assignments: &self.assignments,
            default_pack: self.default_pack.as_deref(),
            role_bindings: &self.role_bindings,
            grant: None,
            lapses: &self.lapses,
            // Named per decision by [`decide_read`], which is the only way
            // to ask `MemoryRead` here: a read decided without a tier is
            // refused by the PDP rather than defaulted (AUTHZ-5, ADR-0038
            // decision 2).
            sensitivity: None,
        }
    }

    /// The position of `scope_id` on the gathered chain, if it is on it.
    /// `Some(0)` is the node itself; a strict ancestor is `Some(n > 0)`.
    pub(crate) fn position_of(&self, scope_id: ScopeId) -> Option<usize> {
        self.chain.iter().position(|node| node.id == scope_id)
    }
}

/// [`DecisionInput::context`] carrying the role being granted, for the one
/// action that fails closed without it (ADR-0015 decision 5).
///
/// Exists for CNSL-2's probe, which asks `RoleAssign` once per role
/// because "may I bind a role here" is not a question with one answer —
/// the base layer's escalation guard reads `context.grant`, so a probe
/// that supplied no role would be asking something the PDP is right to
/// refuse (ADR-0058 decision 1).
pub(crate) fn context_granting<'a>(input: &'a DecisionInput, grant: Role) -> AuthzContext<'a> {
    AuthzContext {
        grant: Some(grant),
        ..input.context()
    }
}

/// [`DecisionInput::context`] naming a tier, for the tier-bearing reads
/// (AUTHZ-5, ADR-0038 decision 2).
///
/// Also CNSL-2's: a capability answer for a tiered read is the *set* of
/// tiers permitted, which takes one ask per tier.
pub(crate) fn context_at_tier<'a>(
    input: &'a DecisionInput,
    sensitivity: Sensitivity,
) -> AuthzContext<'a> {
    AuthzContext {
        sensitivity: Some(sensitivity),
        ..input.context()
    }
}

/// Assembles the decision input for the request's ambient principal (the
/// resolved tenant + token subject) inside the caller's transaction.
/// `anchor` is the already-fetched, ownership-checked node the resource
/// refers to — `None` for tenant-level resources.
///
/// Quarantine resolves here (AUTH-2, ADR-0013 decision 6): a provisioned
/// identity's placement decides; an IdP subject with no identity is
/// quarantined (fail closed — skipping `/auth/login` must not
/// out-privilege completing it, and an unregistered service client is
/// exactly this case); an out-of-band (dev HS256) subject is not, but
/// carries no placement either, so composition rules read nothing for it
/// (ADR-0014 decision 5). Service identities additionally resolve here
/// (AUTH-3, ADR-0018): the token-lifetime cap is enforced fail-closed
/// (decision 5), and `token_scope` — the anchor above the personal leaf —
/// arms the base layer's confinement forbid (decision 4). The identity's
/// placement chain and the resource chain's pack assignments are read
/// here too — pack switches are in force on the very next request
/// (ADR-0014 decision 3).
///
/// Since HIER-2 (ADR-0016) both chains come from the scope-chain cache:
/// warm requests read no hierarchy rows; assignments and bindings stay
/// per-request reads — that is what keeps the next-request freshness
/// promises true (ADR-0016 decision 6). Must run before any hierarchy
/// mutation staged in `conn`'s transaction (decision 4).
pub(crate) async fn gather(
    state: &AppState,
    conn: &mut PgConnection,
    anchor: Option<&HierarchyNode>,
) -> Result<DecisionInput> {
    gather_inner(state, conn, ResourceChain::Anchor(anchor)).await
}

/// [`gather`] for the observe shape (MEM-1, ADR-0020 decision 4): the
/// resource is the caller's own placement leaf, so the resource chain IS
/// the placement chain — one identity read, no separate anchor fetch. The
/// handler takes the home scope and owner from
/// [`DecisionInput::identity`].
pub(crate) async fn gather_at_home(
    state: &AppState,
    conn: &mut PgConnection,
) -> Result<DecisionInput> {
    gather_inner(state, conn, ResourceChain::PrincipalHome).await
}

/// How [`gather_inner`] obtains the resource's scope chain.
enum ResourceChain<'a> {
    /// The already-fetched, ownership-checked node the resource refers to
    /// (`None` for tenant-level resources).
    Anchor(Option<&'a HierarchyNode>),
    /// The principal's own placement chain — the resource of a write that
    /// lands at home.
    PrincipalHome,
}

async fn gather_inner(
    state: &AppState,
    conn: &mut PgConnection,
    resource_chain: ResourceChain<'_>,
) -> Result<DecisionInput> {
    let chains = &state.scope_chains;
    let context = synveda_identity::current_tenant().ok_or_else(|| Error::Internal {
        message: "authorization ran outside a tenant scope".to_owned(),
    })?;
    let tenant_id = context.tenant.id;
    let identity = identities::by_subject(&mut *conn, tenant_id, &context.claims.subject).await?;
    let service = identity
        .as_ref()
        .is_some_and(|identity| identity.kind == IdentityKind::Service);
    if service {
        enforce_service_token_lifetime(&context.claims, state.service_token_max_ttl)?;
    }
    let mut quarantined = match &identity {
        Some(identity) => identity.quarantined,
        None => context.claims.provisioning.is_some(),
    };
    // A departed identity may do nothing (AUTH-4, ADR-0059 decision 8,
    // first layer). Refused through the quarantine attribute rather than
    // through a rule of its own: the base layer's forbid is already
    // invariant, already golden-tested against every pack, and already
    // proof against a pack that forgets it — a second everything-denied
    // mechanism beside it would be a second thing to keep true.
    //
    // This is what makes a seal outlive the IdP: an access token minted
    // before somebody's last day stops working on the next request,
    // whatever the issuer still thinks, and without waiting for AUTH-6's
    // revocation list.
    if identity.as_ref().is_some_and(Identity::sealed) {
        quarantined = true;
        tracing::debug!(
            tenant.id = %tenant_id,
            "a departed identity presented a token; refusing every action"
        );
    }
    let principal_scopes = match &identity {
        // The FK pins the placement node, so it resolves; a missing chain
        // (mid-transaction delete) just leaves the principal unplaced —
        // composition rules then read nothing (fail closed).
        Some(identity) => chains
            .resolve(&mut *conn, tenant_id, identity.scope_id)
            .await?
            .unwrap_or_else(empty_chain),
        None => empty_chain(),
    };
    let chain = match resource_chain {
        ResourceChain::Anchor(Some(node)) => {
            match chains.resolve(&mut *conn, tenant_id, node.id).await? {
                Some(chain) => chain,
                // The anchor vanished between the handler's fetch and this
                // statement (a concurrent committed delete): the node alone,
                // no ancestry — the same shape the per-request reads produced.
                None => vec![node.clone()].into(),
            }
        }
        ResourceChain::Anchor(None) => empty_chain(),
        ResourceChain::PrincipalHome => Arc::clone(&principal_scopes),
    };
    // The confinement scope (ADR-0018 decision 4): the personal leaf's
    // parent — the anchor the agent was registered at — read off the
    // already-resolved placement chain at zero extra cost. A service
    // identity whose anchor cannot be resolved is quarantined, never
    // unconfined (fail closed).
    let token_scope = if service {
        let anchor_node = principal_scopes.get(1);
        if anchor_node.is_none() {
            quarantined = true;
        }
        anchor_node.map(|node| node.id)
    } else {
        None
    };
    let principal = Principal {
        tenant_id,
        subject: context.claims.subject,
        quarantined,
        scope_id: identity.as_ref().map(|identity| identity.scope_id),
        token_scope,
    };
    let chain_ids: Vec<_> = chain.iter().map(|node| node.id).collect();
    let assignments = if chain_ids.is_empty() {
        Vec::new()
    } else {
        policy_assignments::for_scopes(&mut *conn, tenant_id, &chain_ids).await?
    };
    let default_pack = policy_assignments::default_pack(&mut *conn, tenant_id).await?;
    // The subject's bindings for the resource's chain plus its
    // tenant-wide rows (AUTHZ-3, ADR-0015 decision 3) — read here so a
    // new binding is in force on the very next request.
    let role_bindings =
        role_bindings::for_subject_on_scopes(&mut *conn, tenant_id, &principal.subject, &chain_ids)
            .await?;
    // The grants standing over this caller (AUTHZ-4, ADR-0037 decision 4).
    // Keyed on the *placement* chain, not the resource's: a lapse grants to
    // everyone at or under a grantee scope, and which scope is being
    // *decided* is the target, resolved per decision inside the PDP.
    //
    // Read here, per request, alongside bindings and assignments — and for
    // a stronger reason than theirs. Their per-request read keeps a
    // next-request freshness promise; this one *is* the expiry mechanism.
    // Caching it would make a window end late by the length of the cache.
    let lapse_scopes: Vec<ScopeId> = principal_scopes.iter().map(|node| node.id).collect();
    let lapses = lapses::active_for_scopes(&mut *conn, tenant_id, &lapse_scopes).await?;
    Ok(DecisionInput {
        principal,
        chain,
        principal_scopes,
        assignments,
        default_pack,
        role_bindings,
        lapses,
        identity,
    })
}

/// One off-chain scope a standing lapse reaches, with the rows its own
/// `MemoryRead` decision needs (AUTHZ-4, ADR-0037 decision 10).
///
/// Owned, because the read path borrows from it after the gathering
/// transaction has been dropped.
pub(crate) struct LapsedChain {
    /// The grant that reaches the scope.
    pub(crate) lapse: Lapse,
    /// The target's own chain, node-first.
    pub(crate) chain: Arc<[HierarchyNode]>,
    /// Pack assignments for that chain.
    pub(crate) assignments: Vec<synveda_types::PolicyAssignment>,
}

/// Resolves what an inject needs to decide the caller's lapsed scopes: for
/// each grant reaching a scope off the caller's own chain, that scope's
/// chain and assignments.
///
/// The cost the feature is honest about (ADR-0037 decision 10): the
/// effective pack is a property of the resource, so a scope the caller's
/// chain does not cover needs its own rows. Chains come from the HIER-2
/// cache, so the usual case is warm; the assignments are one indexed read
/// per lapsed scope, paid only by callers who actually hold a grant.
///
/// A target whose chain no longer resolves is dropped here rather than
/// planned and denied later — a deleted scope grants nothing, which is the
/// same fail-closed reading an unplaced principal gets.
pub(crate) async fn gather_lapsed(
    state: &AppState,
    conn: &mut PgConnection,
    input: &DecisionInput,
) -> Result<Vec<LapsedChain>> {
    let tenant_id = input.principal.tenant_id;
    // One shared containment rule with the PDP's own permit, so the plan
    // and the decision can never disagree about who a grant reaches.
    let granting = synveda_policy::lapsed_scopes(
        &input.principal_scopes,
        &input.lapses,
        LapseAction::MemoryRead,
    );
    let mut resolved = Vec::with_capacity(granting.len());
    for lapse in granting {
        let Some(chain) = state
            .scope_chains
            .resolve(&mut *conn, tenant_id, lapse.target_scope_id)
            .await?
        else {
            continue;
        };
        let chain_ids: Vec<ScopeId> = chain.iter().map(|node| node.id).collect();
        let assignments = policy_assignments::for_scopes(&mut *conn, tenant_id, &chain_ids).await?;
        resolved.push(LapsedChain {
            lapse: lapse.clone(),
            chain,
            assignments,
        });
    }
    Ok(resolved)
}

/// One scope of recall's widened universe, with the rows its own
/// `MemoryRead` decision needs (CTX-5, ADR-0042 decisions 2 and 3).
///
/// The same shape as [`LapsedChain`] minus the grant, and owned for the
/// same reason: the read path borrows from it after the gathering
/// transaction has been dropped.
pub(crate) struct CandidateChain {
    /// The scope to decide.
    pub(crate) scope_id: ScopeId,
    /// That scope's own chain, node-first.
    pub(crate) chain: Arc<[HierarchyNode]>,
    /// Pack assignments for that chain.
    pub(crate) assignments: Vec<synveda_types::PolicyAssignment>,
}

/// How many scopes one recall may decide beyond the caller's own chain
/// (ADR-0042 decision 5).
///
/// Not a round number for comfort — a measured one. ADR-0029 allotted the
/// plan stage **15ms** of a 300ms recall, and
/// `tests/recall.rs::the_plan_stage_fits_the_budget_adr_0029_derived`
/// measures a decided scope at roughly **230µs**: four `MemoryRead`
/// evaluations, dominated by Cedar request construction rather than by
/// anything this feature could hoist out (the batch materialisation of
/// ADR-0042 decision 6 already took the plan stage from 378ms to 120ms at
/// 512 scopes; what is left is per-request, not per-sweep).
///
/// The stage also carries a fixed cost the sweep does not — the identity
/// read, the chain, the occupancy reads, the assignment and binding reads
/// — measured at roughly 7.7ms on the dev stack, so the arithmetic is
/// `(15ms − fixed) / per-scope`, which lands near 50 and takes 32 for
/// headroom rather than sitting on the line.
///
/// **That fixed cost is virtualised dev IO**, and it is most of the
/// budget: six or so round trips at Docker Desktop's fsync. The same
/// sweep on production-shaped storage has far more room, which is why
/// this is a *default* and not a constant — see [`max_recall_scopes`] —
/// and why EVAL-6 is where the number gets re-derived on hardware that
/// resembles a deployment (the HIER-1/MEM-1/CTX-1 discipline).
///
/// Prefer the other lever before raising it: the honest fix is making a
/// decision cheaper, not making the budget bigger. When the cap binds the
/// caller is *told* (`truncated`), because a bounded answer presented as
/// a complete one is the one failure this surface cannot afford — and 32
/// scopes *will* bind in a large tenant, which is exactly why that
/// reporting is not decoration.
const DEFAULT_MAX_RECALL_SCOPES: usize = 32;

/// [`DEFAULT_MAX_RECALL_SCOPES`], overridable by
/// `SYNVEDA_RECALL_MAX_SCOPES`.
///
/// An operator who has measured their own plan stage — the
/// `synveda_recall_stage_duration_seconds{stage="plan"}` histogram is
/// there for this — can widen the universe on hardware that affords it.
/// A garbage or zero value takes the default rather than failing a read:
/// this bounds cost, and the surface stays correct at any value.
pub(crate) fn max_recall_scopes() -> usize {
    static CAP: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("SYNVEDA_RECALL_MAX_SCOPES")
            .ok()
            .and_then(|raw| raw.parse::<usize>().ok())
            .filter(|cap| *cap > 0)
            .unwrap_or(DEFAULT_MAX_RECALL_SCOPES)
    })
}

/// What one recall decides beyond the caller's chain, and what it had to
/// leave out.
pub(crate) struct Universe {
    /// The scopes to decide, nearest-first.
    pub(crate) candidates: Vec<CandidateChain>,
    /// Every binding the caller holds anywhere in the tenant, replacing
    /// the chain-scoped set [`gather`] read.
    ///
    /// A binding is the grant an administrator issues to widen someone's
    /// reach, and it is one of the two grants ADR-0024 left unreachable.
    /// Deciding a candidate scope without the binding that permits it
    /// would ask the question and get the wrong answer — so the widened
    /// universe needs the widened binding set, or it silently does not
    /// work. `effective_roles_at` still admits each binding only where its
    /// scope is on the resource's own chain, so this widens the *read*,
    /// never a decision.
    pub(crate) role_bindings: Vec<RoleBinding>,
    /// How many contributing scopes existed before the cap.
    pub(crate) considered: usize,
    /// Whether the cap ([`max_recall_scopes`]) dropped any of them.
    pub(crate) truncated: bool,
}

/// Resolves recall's widened candidate set: the scopes that could
/// contribute to this request, ordered nearest-first, capped, each with
/// the rows its own decision needs (CTX-5, ADR-0042 decision 2).
///
/// `occupied` is the raw set — every scope holding or publishing material
/// the request could draw on — and it is deliberately an
/// over-approximation. Narrowing it further would mean inferring a verdict
/// from a pack's shape, which is a second source of truth about policy;
/// every scope that survives here is still an individual per-`(scope,
/// tier)` PDP decision, and the PDP is what says no.
///
/// The ordering is hierarchy distance from the caller: scopes sharing the
/// longest prefix of the caller's own chain come first, so a cap drops the
/// farthest material rather than an arbitrary slice. Scopes already on the
/// caller's chain are omitted — the chain walk decides those, and it
/// carries their gradient position.
#[tracing::instrument(
    name = "gateway.recall_universe",
    skip_all,
    fields(
        occupied = occupied.len(),
        candidates = tracing::field::Empty,
        truncated = tracing::field::Empty,
    ),
    err(Display)
)]
pub(crate) async fn gather_universe(
    state: &AppState,
    conn: &mut PgConnection,
    input: &DecisionInput,
    occupied: &[ScopeId],
) -> Result<Universe> {
    let tenant_id = input.principal.tenant_id;
    let own: HashSet<ScopeId> = input.principal_scopes.iter().map(|node| node.id).collect();
    let own_path: Vec<ScopeId> = input
        .principal_scopes
        .iter()
        .rev()
        .map(|node| node.id)
        .collect();

    // Resolve first, then order by distance: the chain is what distance is
    // measured on, so it has to exist before the cap can be applied
    // fairly. Chains come from the HIER-2 cache, so this is warm for any
    // tenant a recall has touched before.
    let mut resolved: Vec<(usize, CandidateChain)> = Vec::new();
    for scope_id in occupied.iter().copied().filter(|id| !own.contains(id)) {
        let Some(chain) = state
            .scope_chains
            .resolve(&mut *conn, tenant_id, scope_id)
            .await?
        else {
            // A scope that vanished between the occupancy read and now.
            // Deciding it would deny anyway; dropping it is the same
            // fail-closed reading a deleted lapse target gets.
            continue;
        };
        // Root-first, so a shared prefix with the caller's own root-first
        // path is a common ancestry depth.
        let path: Vec<ScopeId> = chain.iter().rev().map(|node| node.id).collect();
        let shared = own_path
            .iter()
            .zip(path.iter())
            .take_while(|(a, b)| a == b)
            .count();
        // Nearer means: more ancestry in common, then shallower. Both are
        // "closer to this reader" in the seed §4.4 sense.
        let distance = (own_path.len() - shared) + (path.len() - shared);
        resolved.push((
            distance,
            CandidateChain {
                scope_id,
                chain,
                assignments: Vec::new(),
            },
        ));
    }
    let considered = resolved.len();
    resolved
        .sort_by(|(a, left), (b, right)| a.cmp(b).then_with(|| left.scope_id.cmp(&right.scope_id)));
    let cap = max_recall_scopes();
    let truncated = considered > cap;
    resolved.truncate(cap);

    // Assignments for everything that survived the cap, in **one** read.
    //
    // A read per candidate is the obvious shape and it is the wrong one:
    // at the cap that is 64 round trips, which measured larger than the
    // entire PDP sweep they exist to feed. The union is a single indexed
    // read; partitioning it per chain afterwards is a memory operation.
    let mut wanted: Vec<ScopeId> = resolved
        .iter()
        .flat_map(|(_, candidate)| candidate.chain.iter().map(|node| node.id))
        .collect();
    wanted.sort_unstable();
    wanted.dedup();
    let all = policy_assignments::for_scopes(&mut *conn, tenant_id, &wanted).await?;

    let mut candidates = Vec::with_capacity(resolved.len());
    for (_, mut candidate) in resolved {
        // Each candidate still gets exactly its own chain's rows: the PDP
        // resolves the effective pack by walking *this* chain, and handing
        // it a neighbour's assignment would put a pack in force at a scope
        // nobody assigned it to (ADR-0014 decision 3).
        candidate.assignments = all
            .iter()
            .filter(|assignment| {
                candidate
                    .chain
                    .iter()
                    .any(|node| node.id == assignment.scope_id)
            })
            .cloned()
            .collect();
        candidates.push(candidate);
    }

    let role_bindings =
        role_bindings::for_subject(&mut *conn, tenant_id, &input.principal.subject).await?;

    let span = tracing::Span::current();
    span.record("candidates", candidates.len());
    span.record("truncated", truncated);
    Ok(Universe {
        candidates,
        role_bindings,
        considered,
        truncated,
    })
}

/// An allowed decision plus what the audit event embeds: the verdict
/// context and the distinct role names the PDP weighed (AUD-1, ADR-0019
/// decision 4).
pub(crate) struct Authorized {
    /// The PDP's verdict context (pack@version, determining policies).
    pub(crate) decision: AuthzDecision,
    /// Distinct role names from the bindings relevant to this decision
    /// (the resource's chain plus tenant-wide — ADR-0015 decision 3).
    pub(crate) roles: Vec<String>,
}

/// Authorizes `action` on `resource` for the request's ambient principal,
/// under the resource's effective pack. Runs after the uniform-404
/// ownership check, so cross-tenant probes never see a policy denial
/// oracle (ADR-0012 decision 7). Returns the decision for the caller's
/// audit event; denials surface as [`Error::PolicyDenied`] and chain at
/// the `respond` seam (ADR-0019 decision 5).
pub(crate) async fn require(
    state: &AppState,
    conn: &mut PgConnection,
    action: Action,
    resource: Resource,
    anchor: Option<&HierarchyNode>,
) -> Result<Authorized> {
    let input = gather(state, conn, anchor).await?;
    decide(state, &input, action, resource, None)
}

/// The one decision seam over a gathered [`DecisionInput`]: evaluates,
/// collapses a deny into the taxonomy, and keeps the allow's context for
/// the caller's audit event. `grant` is the role being granted or revoked
/// for [`Action::RoleAssign`] (ADR-0015 decision 5).
///
/// Not for [`Action::MemoryRead`]: that one names a tier, through
/// [`decide_read`].
pub(crate) fn decide(
    state: &AppState,
    input: &DecisionInput,
    action: Action,
    resource: Resource,
    grant: Option<Role>,
) -> Result<Authorized> {
    decide_from(state, input, 0, action, resource, grant)
}

/// [`decide`] for a resource whose chain starts at `position` of the
/// gathered chain (see [`DecisionInput::context_from`]). The audit
/// event's role list is narrowed to that chain too — a binding at a scope
/// *below* the resource did not bear on this decision and must not be
/// reported as though it had.
pub(crate) fn decide_from(
    state: &AppState,
    input: &DecisionInput,
    position: usize,
    action: Action,
    resource: Resource,
    grant: Option<Role>,
) -> Result<Authorized> {
    decide_inner(state, input, position, action, resource, grant, None)
}

/// [`decide`] for `MemoryRead`, which is the one action that names the tier
/// it is asking about (AUTHZ-5, ADR-0038 decision 2).
///
/// A separate function rather than a seventh parameter on [`decide`],
/// because "which tier" is a question only this action answers, and a
/// parameter every other call site passes `None` to is an invitation to
/// pass `None` here.
pub(crate) fn decide_read(
    state: &AppState,
    input: &DecisionInput,
    resource: Resource,
    sensitivity: Sensitivity,
) -> Result<Authorized> {
    decide_read_from(state, input, 0, resource, sensitivity)
}

/// [`decide_read`] for `PromptRead` (PRMT-1, ADR-0049 decision 4).
///
/// The second action that names a tier, and the reason [`decide_inner`]
/// takes one rather than [`Action::MemoryRead`] assuming it. It is a
/// separate wrapper rather than a parameter on [`decide_read`] for that
/// function's own stated reason: which seam is being asked is not something
/// a call site should be able to get wrong by passing the wrong constant.
pub(crate) fn decide_prompt_read(
    state: &AppState,
    input: &DecisionInput,
    resource: Resource,
    sensitivity: Sensitivity,
) -> Result<Authorized> {
    decide_prompt_read_from(state, input, 0, resource, sensitivity)
}

/// [`decide_prompt_read`] for a resource whose chain starts at `position` —
/// what the registry's gradient walk asks once per scope on the caller's
/// own chain.
pub(crate) fn decide_prompt_read_from(
    state: &AppState,
    input: &DecisionInput,
    position: usize,
    resource: Resource,
    sensitivity: Sensitivity,
) -> Result<Authorized> {
    decide_inner(
        state,
        input,
        position,
        Action::PromptRead,
        resource,
        None,
        Some(sensitivity),
    )
}

/// [`decide_read`] for `ContextPackRead` (PRMT-2, ADR-0050 decision 7).
///
/// The third action that names a tier, and a separate wrapper for
/// [`decide_prompt_read`]'s reason. What makes it different from the other
/// two is what it *admits*: this is the only decision that lets a context
/// pack's chunks into a composed block, and it never lets a memory in
/// (decision 8).
pub(crate) fn decide_context_pack_read(
    state: &AppState,
    input: &DecisionInput,
    resource: Resource,
    sensitivity: Sensitivity,
) -> Result<Authorized> {
    decide_context_pack_read_from(state, input, 0, resource, sensitivity)
}

/// [`decide_context_pack_read`] for a resource whose chain starts at
/// `position`.
pub(crate) fn decide_context_pack_read_from(
    state: &AppState,
    input: &DecisionInput,
    position: usize,
    resource: Resource,
    sensitivity: Sensitivity,
) -> Result<Authorized> {
    decide_inner(
        state,
        input,
        position,
        Action::ContextPackRead,
        resource,
        None,
        Some(sensitivity),
    )
}

/// [`decide_read`] for `SkillRead` (SKIL-1, ADR-0051 decision 10).
///
/// The fourth action that names a tier, and a separate wrapper for
/// [`decide_prompt_read`]'s reason. What makes it different from
/// [`decide_context_pack_read`] is what it does *not* do: it admits nothing
/// into a composed block, because a skill's content becomes no records and
/// enters no block at all (decision 9). It gates a fetch whose bytes are
/// about to become files on somebody's machine.
pub(crate) fn decide_skill_read(
    state: &AppState,
    input: &DecisionInput,
    resource: Resource,
    sensitivity: Sensitivity,
) -> Result<Authorized> {
    decide_skill_read_from(state, input, 0, resource, sensitivity)
}

/// [`decide_skill_read`] for a resource whose chain starts at `position` —
/// what the registry's gradient walk asks once per scope on the caller's own
/// chain.
pub(crate) fn decide_skill_read_from(
    state: &AppState,
    input: &DecisionInput,
    position: usize,
    resource: Resource,
    sensitivity: Sensitivity,
) -> Result<Authorized> {
    decide_inner(
        state,
        input,
        position,
        Action::SkillRead,
        resource,
        None,
        Some(sensitivity),
    )
}

/// [`decide_read`] for a resource whose chain starts at `position` — the
/// [`decide_from`] shape, for the cross-scope decisions FLOW-5 takes.
pub(crate) fn decide_read_from(
    state: &AppState,
    input: &DecisionInput,
    position: usize,
    resource: Resource,
    sensitivity: Sensitivity,
) -> Result<Authorized> {
    decide_inner(
        state,
        input,
        position,
        Action::MemoryRead,
        resource,
        None,
        Some(sensitivity),
    )
}

#[allow(clippy::too_many_arguments)]
fn decide_inner(
    state: &AppState,
    input: &DecisionInput,
    position: usize,
    action: Action,
    resource: Resource,
    grant: Option<Role>,
    sensitivity: Option<Sensitivity>,
) -> Result<Authorized> {
    let mut context = input.context_from(position);
    context.grant = grant;
    context.sensitivity = sensitivity;
    let decision = state
        .pdp
        .authorize(&input.principal, action, resource, &context)?;
    decision.clone().require(action, resource)?;
    let on_chain: HashSet<_> = context.scopes.iter().map(|node| node.id).collect();
    let mut roles: Vec<String> = input
        .role_bindings
        .iter()
        .filter(|binding| {
            binding
                .scope_id
                .is_none_or(|scope| on_chain.contains(&scope))
        })
        .map(|binding| binding.role.as_str().to_owned())
        .collect();
    roles.sort_unstable();
    roles.dedup();
    Ok(Authorized { decision, roles })
}

/// The one message shape a service-token seam rejection carries;
/// [`is_service_token_rejection`] is its one interpreter, so the audit
/// seam never parses ad-hoc strings (same doctrine as the store's
/// backstop marker).
const SERVICE_TOKEN_REJECTION_PREFIX: &str = "service tokens must carry iat and live at most";

/// Whether `error` is a service-token seam rejection — the audit seam
/// (`auth.token.rejected`, ADR-0019 decision 5) classifies with this.
pub(crate) fn is_service_token_rejection(error: &Error) -> bool {
    matches!(error, Error::Unauthenticated { message }
        if message.starts_with(SERVICE_TOKEN_REJECTION_PREFIX))
}

/// The service-token lifetime cap (AUTH-3, ADR-0018 decision 5): a
/// service identity's token must carry a known lifetime (`exp − iat`)
/// within the configured maximum. Fail closed — an unknown lifetime (no
/// `iat`) is refused like an excessive one, as the uniform 401. The
/// rejection chains as `auth.token.rejected` at the `respond` seam
/// (AUD-1, ADR-0019 decision 5 — the transaction here rolls back).
fn enforce_service_token_lifetime(claims: &Claims, max: Duration) -> Result<()> {
    let reason = match claims.lifetime {
        Some(lifetime) if lifetime <= max => return Ok(()),
        Some(_) => "lifetime_exceeded",
        None => "lifetime_unknown",
    };
    metrics::counter!(SERVICE_TOKEN_REJECTIONS_TOTAL, "reason" => reason).increment(1);
    tracing::warn!(
        principal.subject = %claims.subject,
        token.lifetime_secs = claims.lifetime.map(|lifetime| lifetime.as_secs()),
        token.max_ttl_secs = max.as_secs(),
        "service token refused: {reason}"
    );
    Err(Error::Unauthenticated {
        message: format!("{SERVICE_TOKEN_REJECTION_PREFIX} {} seconds", max.as_secs()),
    })
}

fn empty_chain() -> Arc<[HierarchyNode]> {
    Vec::new().into()
}

/// One reload sweep: for every active tenant, read its stored packs (in a
/// tenant transaction — `policy_packs` is RLS-scoped) and reconcile the
/// PDP: install changed packs, drop removed ones, skip unchanged
/// versions. A pack that fails to compile keeps that pack's last-good
/// compile (ADR-0012 decision 5); per-tenant and per-pack failures never
/// abort the sweep.
#[tracing::instrument(name = "authz.refresh_packs", skip_all, err(Display))]
pub async fn refresh_packs_once(pool: &PgPool, pdp: &Pdp) -> Result<()> {
    for tenant in tenants::active(pool).await? {
        refresh_tenant_packs(pool, pdp, tenant.id).await;
    }
    Ok(())
}

/// Reconciles one tenant's stored packs (see [`refresh_packs_once`]),
/// counts each pack-level outcome (`installed`, `removed`, `unchanged`,
/// `error`) into the reload metric, and returns the tenant's collapsed
/// outcome — `error` over `installed` over `removed` over `unchanged`.
pub async fn refresh_tenant_packs(pool: &PgPool, pdp: &Pdp, tenant_id: TenantId) -> &'static str {
    let outcomes = match refresh_tenant(pool, pdp, tenant_id).await {
        Ok(outcomes) => outcomes,
        Err(error) => {
            tracing::warn!(
                tenant.id = %tenant_id,
                error = %error,
                "policy pack reload failed; last-good packs stay in force"
            );
            vec!["error"]
        }
    };
    for outcome in &outcomes {
        metrics::counter!(POLICY_PACK_RELOADS_TOTAL, "outcome" => *outcome).increment(1);
    }
    for candidate in ["error", "installed", "removed"] {
        if outcomes.contains(&candidate) {
            return candidate;
        }
    }
    "unchanged"
}

async fn refresh_tenant(
    pool: &PgPool,
    pdp: &Pdp,
    tenant_id: TenantId,
) -> Result<Vec<&'static str>> {
    let mut tx = rls::begin_tenant_tx(pool, tenant_id).await?;
    let stored = policy_packs::stored(&mut *tx, tenant_id).await?;
    // Read-only transaction; dropping it rolls back, GUC included.
    drop(tx);
    let installed = pdp.installed_versions(tenant_id);
    let mut outcomes = Vec::new();
    for pack in &stored {
        if installed
            .iter()
            .any(|(name, version)| *name == pack.name && *version == pack.version)
        {
            continue;
        }
        match pdp.install_source(
            tenant_id,
            &pack.name,
            pack.version,
            &pack.source,
            pack.config.clone(),
        ) {
            Ok(()) => outcomes.push("installed"),
            Err(error) => {
                tracing::warn!(
                    tenant.id = %tenant_id,
                    policy.pack = %pack.name,
                    error = %error,
                    "stored pack failed to compile; its last-good compile stays in force"
                );
                outcomes.push("error");
            }
        }
    }
    let stored_names: HashSet<&str> = stored.iter().map(|pack| pack.name.as_str()).collect();
    for (name, _) in &installed {
        if !stored_names.contains(name.as_str()) && pdp.remove_pack(tenant_id, name) {
            outcomes.push("removed");
        }
    }
    if outcomes.is_empty() {
        outcomes.push("unchanged");
    }
    Ok(outcomes)
}

/// Spawns the reload loop: one sweep immediately, then one per `interval`.
/// Sweep-level failures (e.g. the database down) are logged and retried
/// next tick — policy distribution degrades to the last-good state, it
/// never takes the gateway down.
pub fn spawn_pack_refresher(
    pool: PgPool,
    pdp: Arc<Pdp>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(error) = refresh_packs_once(&pool, &pdp).await {
                tracing::warn!(error = %error, "policy pack refresh sweep failed");
            }
        }
    })
}
