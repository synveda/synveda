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
use synveda_policy::{Action, AuthzContext, Pdp, Principal, Resource};
use synveda_store::{hierarchy, identities, policy_assignments, policy_packs, rls, tenants};
use synveda_types::{Error, HierarchyNode, Result, TenantId};

use crate::telemetry::POLICY_PACK_RELOADS_TOTAL;

/// Everything [`require`] assembles for one decision: the principal and
/// the caller-supplied data the PDP resolves and materialises from.
/// Handlers that need more than a verdict (the policy routes display the
/// effective pack) build this too, through [`gather`].
pub(crate) struct DecisionInput {
    pub(crate) principal: Principal,
    pub(crate) chain: Vec<HierarchyNode>,
    pub(crate) principal_scopes: Vec<HierarchyNode>,
    pub(crate) assignments: Vec<synveda_types::PolicyAssignment>,
    pub(crate) default_pack: Option<String>,
}

impl DecisionInput {
    pub(crate) fn context(&self) -> AuthzContext<'_> {
        AuthzContext {
            scopes: &self.chain,
            principal_scopes: &self.principal_scopes,
            assignments: &self.assignments,
            default_pack: self.default_pack.as_deref(),
        }
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
/// out-privilege completing it); an out-of-band (dev HS256) subject is
/// not, but carries no placement either, so composition rules read
/// nothing for it (ADR-0014 decision 5). The identity's placement chain
/// and the resource chain's pack assignments are read here too — pack
/// switches are in force on the very next request (ADR-0014 decision 3).
pub(crate) async fn gather(
    conn: &mut PgConnection,
    anchor: Option<&HierarchyNode>,
) -> Result<DecisionInput> {
    let context = synveda_identity::current_tenant().ok_or_else(|| Error::Internal {
        message: "authorization ran outside a tenant scope".to_owned(),
    })?;
    let tenant_id = context.tenant.id;
    let identity = identities::by_subject(&mut *conn, tenant_id, &context.claims.subject).await?;
    let quarantined = match &identity {
        Some(identity) => identity.quarantined,
        None => context.claims.provisioning.is_some(),
    };
    let principal = Principal {
        tenant_id,
        subject: context.claims.subject,
        quarantined,
        scope_id: identity.as_ref().map(|identity| identity.scope_id),
    };
    let chain = match anchor {
        Some(node) => {
            let mut chain = hierarchy::ancestors(&mut *conn, node.id).await?;
            chain.insert(0, node.clone());
            chain
        }
        None => Vec::new(),
    };
    let principal_scopes = match &identity {
        // The FK pins the placement node, so it exists; a missing row
        // (mid-transaction delete) just leaves the principal unplaced —
        // composition rules then read nothing (fail closed).
        Some(identity) => match hierarchy::node(&mut *conn, identity.scope_id).await? {
            Some(node) => {
                let mut placement = hierarchy::ancestors(&mut *conn, node.id).await?;
                placement.insert(0, node);
                placement
            }
            None => Vec::new(),
        },
        None => Vec::new(),
    };
    let assignments = if chain.is_empty() {
        Vec::new()
    } else {
        let ids: Vec<_> = chain.iter().map(|node| node.id).collect();
        policy_assignments::for_scopes(&mut *conn, tenant_id, &ids).await?
    };
    let default_pack = policy_assignments::default_pack(&mut *conn, tenant_id).await?;
    Ok(DecisionInput {
        principal,
        chain,
        principal_scopes,
        assignments,
        default_pack,
    })
}

/// Authorizes `action` on `resource` for the request's ambient principal,
/// under the resource's effective pack. Runs after the uniform-404
/// ownership check, so cross-tenant probes never see a policy denial
/// oracle (ADR-0012 decision 7).
pub(crate) async fn require(
    pdp: &Pdp,
    conn: &mut PgConnection,
    action: Action,
    resource: Resource,
    anchor: Option<&HierarchyNode>,
) -> Result<()> {
    let input = gather(conn, anchor).await?;
    pdp.require(&input.principal, action, resource, &input.context())
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
        match pdp.install_source(tenant_id, &pack.name, pack.version, &pack.source) {
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
