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
    ScopeId, TenantId,
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
        }
    }

    /// The position of `scope_id` on the gathered chain, if it is on it.
    /// `Some(0)` is the node itself; a strict ancestor is `Some(n > 0)`.
    pub(crate) fn position_of(&self, scope_id: ScopeId) -> Option<usize> {
        self.chain.iter().position(|node| node.id == scope_id)
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
    let mut context = input.context_from(position);
    context.grant = grant;
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
