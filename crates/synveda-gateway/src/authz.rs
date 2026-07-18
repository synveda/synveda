//! The gateway's side of the PDP (AUTHZ-1, ADR-0012): the enforcement
//! helper handlers call before acting, and the policy pack refresher that
//! hot-swaps stored per-tenant packs into the embedded engine.
//!
//! Layering (seed §2.4): `synveda-policy` never touches storage, so this
//! module carries the data between them — hierarchy rows into
//! [`synveda_policy::AuthzContext`], stored pack sources into
//! [`Pdp::install_source`].

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgConnection;
use synveda_policy::{Action, AuthzContext, Pdp, Principal, Resource};
use synveda_store::{hierarchy, identities, policy_packs, rls, tenants};
use synveda_types::{Error, HierarchyNode, Result, TenantId};

use crate::telemetry::POLICY_PACK_RELOADS_TOTAL;

/// Authorizes `action` on `resource` for the request's ambient principal
/// (the resolved tenant + token subject), materialising entities from the
/// anchor node's chain. `anchor` is the already-fetched, ownership-checked
/// node the resource refers to — `None` for tenant-level resources. Runs
/// after the uniform-404 ownership check, so cross-tenant probes never see
/// a policy denial oracle (ADR-0012 decision 7).
///
/// Quarantine resolves here, inside the caller's transaction (AUTH-2,
/// ADR-0013 decision 6): a provisioned identity's placement decides; an
/// IdP subject with no identity is quarantined (fail closed — skipping
/// `/auth/login` must not out-privilege completing it); an out-of-band
/// (dev HS256) subject is not, preserving ADR-0012's bootstrap semantics
/// until AUTHZ-2/3 land roles.
pub(crate) async fn require(
    pdp: &Pdp,
    conn: &mut PgConnection,
    action: Action,
    resource: Resource,
    anchor: Option<&HierarchyNode>,
) -> Result<()> {
    let context = synveda_identity::current_tenant().ok_or_else(|| Error::Internal {
        message: "authorization ran outside a tenant scope".to_owned(),
    })?;
    let identity =
        identities::by_subject(&mut *conn, context.tenant.id, &context.claims.subject).await?;
    let quarantined = match &identity {
        Some(identity) => identity.quarantined,
        None => context.claims.provisioning.is_some(),
    };
    let principal = Principal {
        tenant_id: context.tenant.id,
        subject: context.claims.subject,
        quarantined,
    };
    let chain = match anchor {
        Some(node) => {
            let mut chain = hierarchy::ancestors(&mut *conn, node.id).await?;
            chain.insert(0, node.clone());
            chain
        }
        None => Vec::new(),
    };
    pdp.require(
        &principal,
        action,
        resource,
        &AuthzContext { scopes: &chain },
    )
}

/// One reload sweep: for every active tenant, read its stored pack (in a
/// tenant transaction — `policy_packs` is RLS-scoped) and reconcile the
/// PDP: install changed packs, drop removed ones back to `bootstrap`, skip
/// unchanged versions. A pack that fails to compile keeps the tenant's
/// last-good pack (ADR-0012 decision 5); per-tenant failures never abort
/// the sweep.
#[tracing::instrument(name = "authz.refresh_packs", skip_all, err(Display))]
pub async fn refresh_packs_once(pool: &PgPool, pdp: &Pdp) -> Result<()> {
    for tenant in tenants::active(pool).await? {
        refresh_tenant_packs(pool, pdp, tenant.id).await;
    }
    Ok(())
}

/// Reconciles one tenant's pack (see [`refresh_packs_once`]) and returns
/// the recorded outcome: `installed`, `removed`, `unchanged`, or `error`.
pub async fn refresh_tenant_packs(pool: &PgPool, pdp: &Pdp, tenant_id: TenantId) -> &'static str {
    let outcome = match refresh_tenant(pool, pdp, tenant_id).await {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::warn!(
                tenant.id = %tenant_id,
                error = %error,
                "policy pack reload failed; last-good pack stays in force"
            );
            "error"
        }
    };
    metrics::counter!(POLICY_PACK_RELOADS_TOTAL, "outcome" => outcome).increment(1);
    outcome
}

async fn refresh_tenant(pool: &PgPool, pdp: &Pdp, tenant_id: TenantId) -> Result<&'static str> {
    let mut tx = rls::begin_tenant_tx(pool, tenant_id).await?;
    let stored = policy_packs::active(&mut *tx, tenant_id).await?;
    // Read-only transaction; dropping it rolls back, GUC included.
    drop(tx);
    match stored {
        None => {
            if pdp.remove_pack(tenant_id) {
                Ok("removed")
            } else {
                Ok("unchanged")
            }
        }
        Some(pack) => {
            if pdp.installed_version(tenant_id) == Some((pack.name.clone(), pack.version)) {
                return Ok("unchanged");
            }
            pdp.install_source(tenant_id, &pack.name, pack.version, &pack.source)?;
            Ok("installed")
        }
    }
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
