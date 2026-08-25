//! The retention sweep (MEM-6, ADR-0040): the third background loop.
//!
//! Three stages, per tenant, in order, each bounded by a batch:
//!
//! 1. **Expire** — derived records past the horizon the pack at their own
//!    scope sets leave the live corpus through the FND-4 temporal delete.
//!    `as_of` keeps answering, and the CTX-1 sidecar drops their documents
//!    on its next pass through the change feed it already tails.
//! 2. **Destroy** — closed versions past the *second* horizon are deleted
//!    outright, behind migration 0025's named flag. This is the only stage
//!    in the product that removes recorded content from the database, and
//!    no embedded pack configures it (ADR-0040 decision 13), so in the
//!    default configuration it never issues a query.
//! 3. **Dispose** — observe staging rows and their quarantine markers past
//!    the staging horizon, which is the obligation ADR-0020 and ADR-0021
//!    both parked here and migration 0012 wrote into the schema.
//!
//! Nothing here enforces retention. The read path already refused this
//! material in the query that asked (ADR-0040 decision 2, the composition
//! plan's per-scope horizons); this loop is disposal, which is why its
//! cadence is minutes and why a slow pass costs storage rather than
//! exposure.
//!
//! Nothing here is stamped on a record either: every horizon is read from
//! the effective pack at the moment the pass runs, so a policy change
//! governs the very next sweep and the very next inject alike, with no
//! backfill and nothing to reconcile.
//!
//! Where each horizon resolves (ADR-0040 decision 10): a record's own
//! scope decides what that scope keeps, so stages 1 and 2 resolve per
//! scope. The staging plane is a tenant-level buffer whose rows
//! deliberately carry no scope foreign key (ADR-0020), so stage 3 resolves
//! once, at the org root.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::PgPool;
use synveda_audit::{Actor, AuditAction, AuditEvent, Outcome};
use synveda_policy::{AuthzContext, Pdp, Resource};
use synveda_store::retention::{self, DueRecord};
use synveda_store::{policy_assignments, rls, scopes, tenants};

use crate::chain::scope_chain as resolve_scope_chain;
use synveda_types::{
    Error, RecordClass, RecordId, Result, RetentionConfig, ScopeId, Sensitivity, TenantId,
};

/// Counter: records expired out of the live corpus, labelled `class`.
pub const RECORDS_EXPIRED_TOTAL: &str = "synveda_records_expired_total";

/// Counter: rows destroyed, labelled `plane = history | staging |
/// quarantine`. The one counter in the product that measures data being
/// gone rather than hidden.
pub const RETENTION_DESTROYED_TOTAL: &str = "synveda_retention_destroyed_total";

/// The audit actor component for this loop (migration 0014's system actor
/// kind, which ADR-0022 introduced for exactly this class of writer).
const ACTOR_COMPONENT: &str = "retention";

/// The sweep's tuning knobs, parsed from `SYNVEDA_RETENTION_*` by the
/// gateway.
#[derive(Debug, Clone)]
pub struct SweepConfig {
    /// How often a pass runs. Minutes, like the promotion engine and for
    /// the same reason: the read path has already stopped serving this
    /// material, so what a slow pass costs is storage, not exposure.
    pub interval: Duration,
    /// Rows touched per stage, per scope, per pass. A bounded pass that
    /// runs again in five minutes beats an unbounded one that holds a
    /// transaction open across a tenant's whole history.
    pub batch: i64,
}

impl Default for SweepConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(300),
            batch: 512,
        }
    }
}

/// What the sweep holds for its lifetime.
#[derive(Clone)]
pub struct SweepDeps {
    /// The shared connection pool.
    pub pool: PgPool,
    /// The embedded PDP — for pack *configuration*, never for a decision:
    /// retention answers "does this still exist", which is not an access
    /// question (ADR-0040 compliance notes).
    pub pdp: Arc<Pdp>,
}

/// What one pass did, across every tenant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// Tenants examined.
    pub tenants: usize,
    /// Records expired out of the live corpus.
    pub expired: u64,
    /// Closed versions destroyed.
    pub destroyed: u64,
    /// Staging rows disposed of.
    pub staging_disposed: u64,
}

/// What one pass did for one tenant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TenantPass {
    /// Records expired.
    pub expired: u64,
    /// Closed versions destroyed.
    pub destroyed: u64,
    /// Staging rows disposed of.
    pub staging_disposed: u64,
}

/// One pass over every active tenant. A tenant that fails is logged and
/// skipped: retention is per tenant, and one tenant's broken hierarchy
/// must not stop another's disposal.
#[tracing::instrument(name = "ingest.retention.run_once", skip_all, err(Display))]
pub async fn run_once(deps: &SweepDeps, config: &SweepConfig) -> Result<SweepReport> {
    let tenants = tenants::active(&deps.pool).await?;
    let mut report = SweepReport {
        tenants: tenants.len(),
        ..SweepReport::default()
    };
    for tenant in tenants {
        match run_tenant(deps, config, tenant.id).await {
            Ok(pass) => {
                report.expired += pass.expired;
                report.destroyed += pass.destroyed;
                report.staging_disposed += pass.staging_disposed;
            }
            Err(error) => tracing::error!(
                tenant.id = %tenant.id,
                error = %error,
                "retention pass failed for this tenant; others continue"
            ),
        }
    }
    Ok(report)
}

/// One pass for one tenant: expire, destroy, dispose.
#[tracing::instrument(
    name = "ingest.retention.run_tenant",
    skip_all,
    fields(tenant.id = %tenant_id, expired, destroyed, disposed),
    err(Display)
)]
pub async fn run_tenant(
    deps: &SweepDeps,
    config: &SweepConfig,
    tenant_id: TenantId,
) -> Result<TenantPass> {
    let now = Utc::now();
    let mut pass = TenantPass::default();

    // One look first, so a tenant with nothing to dispose of pays a
    // single transaction and three indexed reads rather than a pack
    // resolution per scope — the FLOW-4 lesson AUTHZ-4's sweep records in
    // its own words, learned on a shared dev database where a pass visits
    // thousands of leftover test tenants.
    let (scopes, root, holds_history, holds_staging) = {
        let mut tx = rls::begin_tenant_tx(&deps.pool, tenant_id).await?;
        // Stage 1's work list: one pack resolution per *populated* scope,
        // not per hierarchy node — a tenant with 10,000 scopes and records
        // at three of them resolves three packs.
        let scopes = retention::populated_scopes(&mut *tx, tenant_id).await?;
        let root = scopes::tenant_root(&mut *tx, tenant_id).await?;
        let holds_history = retention::holds_closed_versions(&mut *tx, tenant_id).await?;
        let holds_staging = retention::holds_staging(&mut *tx, tenant_id).await?;
        tx.rollback().await.map_err(|err| Error::Storage {
            message: format!("close retention work-list transaction: {err}"),
        })?;
        (scopes, root, holds_history, holds_staging)
    };
    if scopes.is_empty() && !holds_history && !holds_staging {
        return Ok(pass);
    }

    // Kept for stage 2's shortest-horizon test, so a tenant whose packs
    // destroy nothing never scans history at all.
    let mut resolved: BTreeMap<ScopeId, RetentionConfig> = BTreeMap::new();
    for scope_id in scopes {
        match expire_scope(deps, config, tenant_id, scope_id, now).await {
            Ok((retention, expired)) => {
                resolved.insert(scope_id, retention);
                pass.expired += expired;
            }
            Err(error) => tracing::error!(
                tenant.id = %tenant_id,
                scope.id = %scope_id,
                error = %error,
                "retention expiry failed at this scope; others continue"
            ),
        }
    }

    // The root's pack governs the staging plane and stands in for scopes
    // whose live records have all gone (ADR-0040 decision 10).
    let Some(root) = root else {
        // A tenant with no root scope has nothing placed and nothing to
        // resolve a pack against. Nothing to do, and nothing wrong.
        return Ok(pass);
    };
    let root_config = {
        let mut tx = rls::begin_tenant_tx(&deps.pool, tenant_id).await?;
        let config = effective_retention(deps, &mut tx, tenant_id, root.id).await?;
        tx.rollback().await.map_err(|err| Error::Storage {
            message: format!("close retention root resolution: {err}"),
        })?;
        config
    };
    resolved.insert(root.id, root_config);

    // Stage 2. The shortest destruction horizon any resolved pack sets
    // decides whether history is scanned at all; each scope then applies
    // its own. In the product default nothing destroys, so this is one
    // `min` over a map and no query.
    let shortest = resolved
        .values()
        .filter_map(|config| config.destroy_cutoff(now))
        .max();
    if let Some(cutoff) = shortest {
        match destroy_stage(deps, config, tenant_id, cutoff, &resolved, root_config, now).await {
            Ok(destroyed) => pass.destroyed += destroyed,
            Err(error) => tracing::error!(
                tenant.id = %tenant_id,
                error = %error,
                "retention destruction stage failed; expiry and disposal stand"
            ),
        }
    }

    // Stage 3. The staging plane, on the root's horizon.
    match dispose_stage(deps, config, tenant_id, root.id, root_config, now).await {
        Ok(disposed) => pass.staging_disposed += disposed,
        Err(error) => tracing::error!(
            tenant.id = %tenant_id,
            error = %error,
            "staging disposal failed; the records stages stand"
        ),
    }

    let span = tracing::Span::current();
    span.record("expired", pass.expired);
    span.record("destroyed", pass.destroyed);
    span.record("disposed", pass.staging_disposed);
    Ok(pass)
}

/// The retention config in force at `scope_id`, resolved exactly as the
/// read path resolves it (ADR-0014 decision 3): the scope's own chain,
/// its assignments, the tenant default, the embedded default.
///
/// No principal and no roles: which horizons apply here is a property of
/// the node, not of whoever's material happens to sit on it — the same
/// reading FLOW-4's sweep takes of promotion rules.
///
/// Runs inside the caller's transaction rather than opening one: a pass
/// over a busy tenant resolves a pack per populated scope, and a
/// transaction per resolution is the difference between a sweep that keeps
/// up and one that does not.
async fn effective_retention(
    deps: &SweepDeps,
    conn: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    scope_id: ScopeId,
) -> Result<RetentionConfig> {
    let chain = resolve_scope_chain(&mut *conn, tenant_id, scope_id).await?;
    if chain.is_empty() {
        // A scope deleted since the work list was taken takes its
        // material's schedule with it; the next pass will not see it.
        return Ok(RetentionConfig::OFF);
    }
    let chain_ids: Vec<ScopeId> = chain.iter().map(|node| node.id).collect();
    let assignments = policy_assignments::for_scopes(&mut *conn, tenant_id, &chain_ids).await?;
    let default_pack = policy_assignments::default_pack(&mut *conn, tenant_id).await?;
    let context = AuthzContext {
        scopes: &chain,
        principal_scopes: &[],
        anchors: &[],
        groups: &[],
        resources: &[],
        assignments: &assignments,
        default_pack: default_pack.as_deref(),
        sensitivity: Some(Sensitivity::WORKING),
        relaxations: &[],
    };
    let pack = deps
        .pdp
        .effective(tenant_id, Resource::Scope(scope_id), &context);
    Ok(pack.retention)
}

/// Stage 1 at one scope: expire what the scope's own pack no longer keeps,
/// and chain one event describing exactly what left.
async fn expire_scope(
    deps: &SweepDeps,
    config: &SweepConfig,
    tenant_id: TenantId,
    scope_id: ScopeId,
    now: DateTime<Utc>,
) -> Result<(RetentionConfig, u64)> {
    let mut tx = rls::begin_tenant_tx(&deps.pool, tenant_id).await?;
    let retention_config = effective_retention(deps, &mut tx, tenant_id, scope_id).await?;
    let cutoffs: Vec<(RecordClass, DateTime<Utc>)> = RecordClass::ALL
        .into_iter()
        .filter_map(|class| {
            retention_config
                .cutoff(class, now)
                .map(|cutoff| (class, cutoff))
        })
        .collect();
    if cutoffs.is_empty() {
        tx.rollback().await.map_err(|err| Error::Storage {
            message: format!("close retention pack resolution: {err}"),
        })?;
        return Ok((retention_config, 0));
    }
    let due = retention::due_at_scope(&mut tx, tenant_id, scope_id, &cutoffs, config.batch).await?;
    if due.is_empty() {
        tx.rollback().await.map_err(|err| Error::Storage {
            message: format!("close retention expiry read: {err}"),
        })?;
        return Ok((retention_config, 0));
    }
    let ids: Vec<RecordId> = due.iter().map(|record| record.id).collect();
    // What actually left, not what was selected: a record re-pinned or
    // already gone between the two statements is simply absent, and the
    // event describes the delete rather than the intention.
    let expired = retention::expire(&mut tx, tenant_id, &ids).await?;
    if expired.is_empty() {
        tx.rollback().await.map_err(|err| Error::Storage {
            message: format!("close retention expiry: {err}"),
        })?;
        return Ok((retention_config, 0));
    }
    synveda_audit::append(
        &mut tx,
        tenant_id,
        &AuditEvent {
            occurred_at: now,
            actor: Actor::system(ACTOR_COMPONENT),
            action: AuditAction::MemoryExpired,
            resource: Resource::Scope(scope_id).to_string(),
            outcome: Outcome::Success,
            payload: expiry_payload(scope_id, &retention_config, &cutoffs, &expired, now),
            // A sweep has no request span to inherit; the chain is the
            // trail here (the promotion engine's shape).
            trace_id: None,
        },
    )
    .await?;
    tx.commit().await.map_err(|err| Error::Storage {
        message: format!("commit retention expiry: {err}"),
    })?;
    for record in &expired {
        metrics::counter!(RECORDS_EXPIRED_TOTAL, "class" => record.class.as_str()).increment(1);
    }
    tracing::info!(
        tenant.id = %tenant_id,
        scope.id = %scope_id,
        expired = expired.len(),
        "records expired by retention"
    );
    Ok((retention_config, expired.len() as u64))
}

/// The `memory.expired` payload: the schedule that decided, the horizon
/// per class, and what left with how old it was. Never content — the
/// point of the operation is that the content stops being available, and
/// an audit trail that quoted it would defeat the act it records.
fn expiry_payload(
    scope_id: ScopeId,
    config: &RetentionConfig,
    cutoffs: &[(RecordClass, DateTime<Utc>)],
    expired: &[DueRecord],
    now: DateTime<Utc>,
) -> serde_json::Value {
    json!({
        "scope_id": scope_id,
        "horizons": cutoffs
            .iter()
            .map(|(class, cutoff)| json!({
                "class": class.as_str(),
                "ttl_days": config.ttl.days(*class),
                "cutoff": cutoff,
            }))
            .collect::<Vec<_>>(),
        "records": expired
            .iter()
            .map(|record| json!({
                "record_id": record.id,
                "class": record.class.as_str(),
                "valid_from": record.valid_from,
                // Whole days, so the number is comparable across events
                // and carries no float (ADR-0019 decision 2).
                "age_days": (now - record.valid_from).num_days(),
            }))
            .collect::<Vec<_>>(),
        "count": expired.len(),
        // Said plainly: an auditor reading this should not need to know
        // the implementation to know what survived it.
        "note": "temporal delete: the records left the live corpus, their history \
                 remains queryable as-of until the destruction horizon",
    })
}

/// [`effective_retention`] with a transaction of its own — for the one
/// caller that has none open.
async fn resolve_scope(
    deps: &SweepDeps,
    tenant_id: TenantId,
    scope_id: ScopeId,
) -> Result<RetentionConfig> {
    let mut tx = rls::begin_tenant_tx(&deps.pool, tenant_id).await?;
    let config = effective_retention(deps, &mut tx, tenant_id, scope_id).await?;
    tx.rollback().await.map_err(|err| Error::Storage {
        message: format!("close retention pack resolution: {err}"),
    })?;
    Ok(config)
}

/// Stage 2: destroy closed versions, per scope, under each scope's own
/// horizon. `shortest` bounds the work list; scopes without their own
/// resolved config fall back to the root's.
async fn destroy_stage(
    deps: &SweepDeps,
    sweep: &SweepConfig,
    tenant_id: TenantId,
    shortest: DateTime<Utc>,
    resolved: &BTreeMap<ScopeId, RetentionConfig>,
    root_config: RetentionConfig,
    now: DateTime<Utc>,
) -> Result<u64> {
    let scopes = {
        let mut tx = rls::begin_tenant_tx(&deps.pool, tenant_id).await?;
        let scopes = retention::scopes_with_closed_versions(&mut *tx, tenant_id, shortest).await?;
        tx.rollback().await.map_err(|err| Error::Storage {
            message: format!("close retention destruction work list: {err}"),
        })?;
        scopes
    };
    let mut destroyed = 0;
    for scope_id in scopes {
        let config = match resolved.get(&scope_id) {
            Some(config) => *config,
            // A scope whose live records have all gone: it was not in
            // stage 1's work list, so resolve it now rather than skipping
            // it — its history is exactly what this stage is for.
            None => match resolve_scope(deps, tenant_id, scope_id).await {
                Ok(config) => config,
                Err(error) => {
                    tracing::error!(
                        tenant.id = %tenant_id,
                        scope.id = %scope_id,
                        error = %error,
                        "retention pack resolution failed at this scope; \
                         falling back to the org root's schedule"
                    );
                    root_config
                }
            },
        };
        let Some(cutoff) = config.destroy_cutoff(now) else {
            continue;
        };
        let mut tx = rls::begin_tenant_tx(&deps.pool, tenant_id).await?;
        let count =
            retention::destroy_history(&mut tx, tenant_id, scope_id, cutoff, sweep.batch).await?;
        if count == 0 {
            tx.rollback().await.map_err(|err| Error::Storage {
                message: format!("close retention destruction: {err}"),
            })?;
            continue;
        }
        synveda_audit::append(
            &mut tx,
            tenant_id,
            &AuditEvent {
                occurred_at: now,
                actor: Actor::system(ACTOR_COMPONENT),
                action: AuditAction::MemoryDisposed,
                resource: Resource::Scope(scope_id).to_string(),
                outcome: Outcome::Success,
                payload: json!({
                    "plane": "records_history",
                    "scope_id": scope_id,
                    "versions": count,
                    "destroy_after_days": config.destroy_after_days,
                    "cutoff": cutoff,
                    "note": "closed versions destroyed; as-of queries can no longer \
                             answer for the instants they covered",
                }),
                trace_id: None,
            },
        )
        .await?;
        tx.commit().await.map_err(|err| Error::Storage {
            message: format!("commit retention destruction: {err}"),
        })?;
        metrics::counter!(RETENTION_DESTROYED_TOTAL, "plane" => "history").increment(count);
        tracing::info!(
            tenant.id = %tenant_id,
            scope.id = %scope_id,
            versions = count,
            "record versions destroyed by retention"
        );
        destroyed += count;
    }
    Ok(destroyed)
}

/// Stage 3: the staging plane, on the root's horizon — one cutoff for a
/// buffer whose rows deliberately carry no scope FK (ADR-0020).
async fn dispose_stage(
    deps: &SweepDeps,
    sweep: &SweepConfig,
    tenant_id: TenantId,
    root_id: ScopeId,
    config: RetentionConfig,
    now: DateTime<Utc>,
) -> Result<u64> {
    let Some(cutoff) = config.staging_cutoff(now) else {
        return Ok(0);
    };
    let mut tx = rls::begin_tenant_tx(&deps.pool, tenant_id).await?;
    let disposed = retention::dispose_staging(&mut tx, tenant_id, cutoff, sweep.batch).await?;
    if disposed.events == 0 {
        tx.rollback().await.map_err(|err| Error::Storage {
            message: format!("close staging disposal: {err}"),
        })?;
        return Ok(0);
    }
    synveda_audit::append(
        &mut tx,
        tenant_id,
        &AuditEvent {
            occurred_at: now,
            actor: Actor::system(ACTOR_COMPONENT),
            action: AuditAction::MemoryDisposed,
            resource: Resource::Scope(root_id).to_string(),
            outcome: Outcome::Success,
            payload: json!({
                "plane": "observe_staging",
                "events": disposed.events,
                "quarantine_markers": disposed.quarantined,
                // Counted on its own because "reviews that aged out" is a
                // fact an auditor should be told rather than notice
                // (ADR-0040 decision 7).
                "quarantine_pending": disposed.quarantined_pending,
                "staging_days": config.staging_days,
                "cutoff": cutoff,
                "note": "observe payloads destroyed; their idempotency keys are free \
                         again, which is what MEM-1's admission guarantee is worth",
            }),
            trace_id: None,
        },
    )
    .await?;
    tx.commit().await.map_err(|err| Error::Storage {
        message: format!("commit staging disposal: {err}"),
    })?;
    metrics::counter!(RETENTION_DESTROYED_TOTAL, "plane" => "staging").increment(disposed.events);
    metrics::counter!(RETENTION_DESTROYED_TOTAL, "plane" => "quarantine")
        .increment(disposed.quarantined);
    tracing::info!(
        tenant.id = %tenant_id,
        events = disposed.events,
        quarantined = disposed.quarantined,
        "observe staging disposed by retention"
    );
    Ok(disposed.events)
}

/// Spawns the sweep loop — the promotion engine's shape. Abort the handle
/// on shutdown.
#[must_use]
pub fn spawn(deps: SweepDeps, config: SweepConfig) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(config.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match run_once(&deps, &config).await {
                Ok(report) if report.expired + report.destroyed + report.staging_disposed > 0 => {
                    tracing::info!(
                        tenants = report.tenants,
                        expired = report.expired,
                        destroyed = report.destroyed,
                        disposed = report.staging_disposed,
                        "retention pass complete"
                    );
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(error = %error, "retention sweep failed"),
            }
        }
    })
}
