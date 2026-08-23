//! The gateway's side of the PDP (AUTHZ-1 ADR-0012; AUTHZ-2 ADR-0014):
//! the enforcement helper handlers call before acting, and the policy pack
//! refresher that hot-swaps stored per-tenant packs into the embedded
//! engine.
//!
//! Layering (seed §2.4): `synveda-policy` never touches storage, so this
//! module carries the data between them — governed scope rows and pack
//! assignments into [`synveda_policy::AuthzContext`], stored pack sources
//! into [`Pdp::install_source`].
//!
//! Since the hierarchy cutover there is **one gather** (CPR-7, ADR-0074
//! decision 2): every route's resource chain comes from `scope_closure`,
//! every caller's own chain starts at their principal scope, and the roles
//! a decision weighs are the grant keys its anchors carry. The two-gather
//! shape CPR-6 left behind — old hierarchy chains projected at the
//! caller's edge, governed chains read from the closure — is deleted with
//! the tree it read.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgConnection;
use synveda_identity::Claims;
use synveda_policy::{
    Action, AuthzContext, AuthzDecision, Pdp, Principal, Resource, ResourceEntity, ScopeNode,
};
use synveda_store::anchors::AnchorSelection;
use synveda_store::{
    anchors, identities, lapses, policy_assignments, policy_packs, rls, scopes, tenants,
};
use synveda_types::anchor::AnchorSet;
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{
    Error, GroupId, Identity, IdentityKind, Lapse, LapseAction, Result, ScopeId, Sensitivity,
    TenantId,
};

use crate::app::AppState;
use crate::telemetry::{POLICY_PACK_RELOADS_TOTAL, SERVICE_TOKEN_REJECTIONS_TOTAL};

/// Everything [`require`] assembles for one decision: the principal and
/// the caller-supplied data the PDP resolves and materialises from.
/// Handlers that need more than a verdict (the policy routes display the
/// effective pack) build this too, through [`gather`].
pub(crate) struct DecisionInput {
    pub(crate) principal: Principal,
    /// The resource's chain as the PDP takes it — the governed scope chain
    /// itself since the cutover (CPR-7, ADR-0074), read from
    /// `scope_closure`. Kept beside `chain_nodes` for the callers that
    /// read ids off it; the two are one chain.
    pub(crate) chain: Arc<[ScopeNode]>,
    /// The caller's own chain, nearest-first from their own scope.
    pub(crate) principal_scopes: Arc<[ScopeNode]>,
    /// The ordered anchors this request resolved to (CPR-6, ADR-0073).
    /// Empty when the tenant has no governed scopes yet, which denies
    /// exactly what a grant would have permitted.
    pub(crate) anchors: AnchorSet,
    /// The groups this caller is in, so a pack may name one.
    pub(crate) groups: Vec<GroupId>,
    /// The subtype and access-plane entities this decision names.
    pub(crate) resources: Vec<ResourceEntity>,
    pub(crate) assignments: Vec<synveda_types::PolicyAssignment>,
    pub(crate) default_pack: Option<String>,
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
            anchors: self.anchors.as_slice(),
            groups: &self.groups,
            resources: &self.resources,
            assignments: &self.assignments,
            default_pack: self.default_pack.as_deref(),
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
/// `anchor` is the already-fetched, ownership-checked governed scope the
/// resource refers to — `None` for tenant-level resources.
///
/// Quarantine resolves here (AUTH-2, ADR-0013 decision 6), and since the
/// cutover it has exactly one meaning: *not provisioned*. An IdP subject
/// with no identity is quarantined — fail closed, because skipping
/// `/auth/login` must not out-privilege completing it, and an unregistered
/// service client is exactly this case. A provisioned identity never is:
/// its scope is its own principal scope (CPR-7, ADR-0074 decision 3), and
/// an ungranted one reaches nothing because the anchor model says so
/// rather than because a placement-derived flag does. Service identities
/// additionally resolve here (AUTH-3, ADR-0018): the token-lifetime cap is
/// enforced fail-closed (decision 5), and `token_scope` — the scope above
/// their own — arms the base layer's confinement forbid (decision 4). The
/// caller's own chain and the resource chain's pack assignments are read
/// here too — pack switches are in force on the very next request
/// (ADR-0014 decision 3).
///
/// `selection` names a workspace or a project the request is about, so
/// the anchors are resolved against it; `resources` are the Cedar
/// entities the decision names — the workspace, project, group or grant
/// it is actually about. A decision whose entity is missing evaluates
/// with no attributes and no parents, which denies.
pub(crate) async fn gather(
    state: &AppState,
    conn: &mut PgConnection,
    anchor: Option<&Scope>,
    selection: AnchorSelection,
    resources: Vec<ResourceEntity>,
) -> Result<DecisionInput> {
    gather_inner(
        state,
        conn,
        ResourceChain::Anchor(anchor),
        selection,
        resources,
    )
    .await
}

/// [`gather`] for the observe shape (MEM-1, ADR-0020 decision 4): the
/// resource is the caller's own scope, so the resource chain IS the
/// caller's chain — one identity read, no separate anchor fetch. The
/// handler takes the home scope and owner from
/// [`DecisionInput::identity`].
pub(crate) async fn gather_at_home(
    state: &AppState,
    conn: &mut PgConnection,
) -> Result<DecisionInput> {
    gather_inner(
        state,
        conn,
        ResourceChain::PrincipalHome,
        AnchorSelection::none(),
        Vec::new(),
    )
    .await
}

/// How [`gather_inner`] obtains the resource's scope chain.
enum ResourceChain<'a> {
    /// The already-fetched, ownership-checked governed scope the resource
    /// refers to (`None` for tenant-level resources).
    Anchor(Option<&'a Scope>),
    /// The principal's own chain — the resource of a write that lands at
    /// home.
    PrincipalHome,
}

async fn gather_inner(
    state: &AppState,
    conn: &mut PgConnection,
    resource_chain: ResourceChain<'_>,
    selection: AnchorSelection,
    resources: Vec<ResourceEntity>,
) -> Result<DecisionInput> {
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
    // Quarantine is only ever "not provisioned" now (CPR-7, ADR-0074
    // decision 3): an identity row — user or service — is never
    // quarantined, and a subject with none is, fail closed.
    let mut quarantined = identity.is_none() && context.claims.provisioning.is_some();
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
    // The caller's own chain: the identity row is the binding between a
    // token subject and the scope that is theirs (CPR-7, ADR-0074 decision
    // 3) — a directory-created identity's scope is keyed by its directory
    // anchor, so the row is read before the slug-keyed fallback and the
    // two can never mint one person two scopes.
    let own_scope = match &identity {
        Some(identity) => Some(identity.scope_id),
        None => scopes::principal_scope(&mut *conn, tenant_id, &context.claims.subject)
            .await?
            .map(|scope| scope.id),
    };
    let principal_scopes: Arc<[ScopeNode]> = match own_scope {
        Some(scope_id) => {
            // The identity pins its scope, so this resolves; a missing row
            // (mid-transaction archive) just leaves the principal
            // unanchored — composition rules then read nothing (fail
            // closed).
            let Some(scope) = scopes::get(&mut *conn, tenant_id, scope_id).await? else {
                unreachable!("an identity's scope is pinned by its foreign key")
            };
            let sealed = identity.as_ref().is_some_and(Identity::sealed);
            let mut chain = vec![ScopeNode::from_scope(&scope, sealed)];
            for ancestor in scopes::ancestors(&mut *conn, tenant_id, scope_id).await? {
                chain.push(ScopeNode::from_scope(&ancestor, false));
            }
            chain.into()
        }
        None => empty_chain(),
    };
    let chain: Arc<[ScopeNode]> = match resource_chain {
        ResourceChain::Anchor(Some(scope)) => {
            // The anchor was fetched and ownership-checked moments ago; a
            // chain that somehow fails to resolve falls back to the scope
            // alone, no ancestry — the shape a concurrent delete leaves.
            scope_chain_of(&mut *conn, tenant_id, scope).await?
        }
        ResourceChain::Anchor(None) => empty_chain(),
        ResourceChain::PrincipalHome => Arc::clone(&principal_scopes),
    };
    // The confinement scope (ADR-0018 decision 4): the scope above the
    // service's own — the anchor it was registered at — read off the
    // already-resolved chain at zero extra cost. A service identity whose
    // anchor cannot be resolved is quarantined, never unconfined (fail
    // closed).
    let token_scope = if service {
        let anchor_node = principal_scopes.get(1);
        if anchor_node.is_none() {
            quarantined = true;
        }
        anchor_node.map(|node| node.id)
    } else {
        None
    };
    // The anchors, resolved for every request (ADR-0073 decision 7).
    //
    // Not only on the governed plane, and the reason is the tenant plane: a
    // grant written at the tenant root is the tenant-wide grant, and an audit,
    // directory or proposal decision that skipped anchor resolution would be a
    // decision the grant model could not reach. The cost is a handful of
    // indexed reads inside a transaction the request already opened.
    let anchors =
        anchors::resolve(&mut *conn, tenant_id, &context.claims.subject, selection).await?;
    let groups = anchors::groups_of(&mut *conn, tenant_id, &context.claims.subject).await?;

    let principal = Principal {
        tenant_id,
        subject: context.claims.subject,
        quarantined,
        // Where this caller stands: their own scope's id, which is what
        // `principal in resource` walks up from.
        scope_id: principal_scopes.first().map(|node| node.id),
        token_scope,
    };
    let chain_ids: Vec<_> = chain.iter().map(|node| node.id).collect();
    let assignments = if chain_ids.is_empty() {
        Vec::new()
    } else {
        policy_assignments::for_scopes(&mut *conn, tenant_id, &chain_ids).await?
    };
    let default_pack = policy_assignments::default_pack(&mut *conn, tenant_id).await?;
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
        anchors,
        groups,
        resources,
        assignments,
        default_pack,
        lapses,
        identity,
    })
}

/// One governed scope's chain, nearest first, as the PDP takes it.
///
/// The seal rides the chain head only, and only on the one shape that is
/// ever somebody's own: a `principal`-shaped scope is sealed exactly when
/// the identity that owns it has departed (ADR-0059 decisions 7 and 9) —
/// one indexed read, no second column, the same single source of truth the
/// old model derived from placement.
async fn scope_chain_of(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    scope: &Scope,
) -> Result<Arc<[ScopeNode]>> {
    let sealed = if scope.kind == ScopeKind::Principal {
        identities::by_scope(&mut *conn, tenant_id, scope.id)
            .await?
            .is_some_and(|identity| identity.sealed())
    } else {
        false
    };
    let mut nodes = vec![ScopeNode::from_scope(scope, sealed)];
    for ancestor in scopes::ancestors(&mut *conn, tenant_id, scope.id).await? {
        nodes.push(ScopeNode::from_scope(&ancestor, false));
    }
    Ok(nodes.into())
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
    pub(crate) chain: Arc<[ScopeNode]>,
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
    _state: &AppState,
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
        let Some(target) = scopes::get(&mut *conn, tenant_id, lapse.target_scope_id).await? else {
            continue;
        };
        let chain = scope_chain_of(&mut *conn, tenant_id, &target).await?;
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

/// An allowed decision plus what the audit event embeds: the verdict
/// context and the distinct role names the PDP weighed (AUD-1, ADR-0019
/// decision 4).
pub(crate) struct Authorized {
    /// The PDP's verdict context (pack@version, determining policies).
    pub(crate) decision: AuthzDecision,
    /// The distinct grant keys the decision weighed — every key a direct
    /// or group grant reaches this resource with (CPR-7, ADR-0074
    /// decision 6 over ADR-0015 decision 3).
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
    anchor: Option<&Scope>,
) -> Result<Authorized> {
    let input = gather(state, conn, anchor, AnchorSelection::none(), Vec::new()).await?;
    decide(state, &input, action, resource)
}

/// The one decision seam over a gathered [`DecisionInput`]: evaluates,
/// collapses a deny into the taxonomy, and keeps the allow's context for
/// the caller's audit event.
///
/// Not for [`Action::MemoryRead`]: that one names a tier, through
/// [`decide_read`].
pub(crate) fn decide(
    state: &AppState,
    input: &DecisionInput,
    action: Action,
    resource: Resource,
) -> Result<Authorized> {
    decide_from(state, input, 0, action, resource)
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
) -> Result<Authorized> {
    decide_inner(state, input, position, action, resource, None)
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
    sensitivity: Option<Sensitivity>,
) -> Result<Authorized> {
    let mut context = input.context_from(position);
    context.sensitivity = sensitivity;
    let decision = state
        .pdp
        .authorize(&input.principal, action, resource, &context)?;
    decision.clone().require(action, resource)?;
    // The grant keys that reached this resource (CPR-6, ADR-0073 decision
    // 5; since the cutover, the only roles there are). The audit event's
    // role list is what the decision *actually* weighed.
    let mut roles: Vec<String> = Vec::new();
    roles.extend(
        synveda_policy::effective_role_keys_at(resource, &context)
            .into_iter()
            .map(|key| key.as_str().to_owned()),
    );
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

fn empty_chain() -> Arc<[ScopeNode]> {
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
