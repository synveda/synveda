//! The scheduled pull sync (AUTH-5, ADR-0060): one pass, and what it is
//! entitled to conclude.
//!
//! The loop is the pack refresher's and the lapse sweep's shape — one pass
//! immediately, then one per interval, failures logged and never fatal
//! (ADR-0060 decision 1). It lives in this crate because [`reconcile`] does:
//! a pull writes the mirror and then calls the same projection the SCIM plane
//! calls, so joiner, mover and leaver exist once (decision 2).
//!
//! ## The order a pass decides in
//!
//! 1. **Yield to the push plane.** A tenant with a live SCIM credential is
//!    skipped entirely (decision 5). Live means issued and not revoked, and
//!    deliberately not "unexpired": an unrotated expiry is an operational
//!    lapse, and the answer to "the push plane broke" must not be a silent
//!    3am handover to a plane that has never enumerated this directory.
//! 2. **Enumerate.** Full, never a delta feed (decision 6).
//! 3. **Record presence, always.** Everybody the pass saw is upserted into
//!    the mirror and reconciled, whether or not the pass completed — seeing
//!    somebody is not conditional on seeing everybody.
//! 4. **Stop here if the pass did not complete.** No absence is counted, no
//!    completeness is recorded, nobody is sealed. An incomplete pass is not
//!    evidence about who is gone (decision 3.1), and this is the one step
//!    where that rule is enforced rather than described.
//! 5. **Count absence, then decide.** Everyone unseen has their count
//!    advanced; everyone at or past the threshold is *proposed* as a leaver;
//!    the breaker sizes the proposal; and only then does anybody seal.
//!
//! ## What this module does not do
//!
//! It does not seal anybody itself. Sealing is `active: false` on the mirror
//! followed by [`reconcile`], which is AUTH-4's own leaver path — the same
//! seal, the same events, the same three layers. A second sealing mechanism
//! is exactly what ADR-0059 decision 3's single reconciler exists to prevent.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use serde_json::json;
use synveda_audit::{Actor, AuditAction, Outcome};
use synveda_identity::directory::{DirectoryConnector, Enumeration};
use synveda_store::directory::UserAttributes;
use synveda_store::{directory, directory_sync, rls, tenants};
use synveda_types::{DirectoryGroupId, DirectoryUserId, Result, Tenant, TenantId};

use crate::app::AppState;
use crate::audit;
use crate::scim::reconcile::{self, DirectorySource};

/// How a deployment tunes the pass.
#[derive(Debug, Clone, Copy)]
pub struct SyncConfig {
    /// How often a pass runs.
    pub interval: Duration,
    /// `N` — consecutive complete passes an absence must survive before it
    /// is offered as a leaver (ADR-0060 decision 3.2).
    pub absence_passes: i32,
    /// The share of a tenant's live users a single pass may seal before the
    /// breaker refuses (decision 3.3).
    pub breaker_fraction: f64,
    /// The floor beneath which the fraction does not apply, so a six-person
    /// tenant does not trip on one leaver.
    pub breaker_floor: i64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(3600),
            absence_passes: 2,
            breaker_fraction: 0.10,
            breaker_floor: 5,
        }
    }
}

/// What one pass did, for the log, the metric and the test.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PassReport {
    /// Users the enumeration listed.
    pub seen: usize,
    /// Whether every page answered.
    pub complete: bool,
    /// Mirror rows whose absence count advanced.
    pub absent: u64,
    /// People sealed by this pass.
    pub sealed: usize,
    /// People the breaker declined to seal, if it refused.
    pub refused: Option<i32>,
    /// Whether a standing authorisation was spent to get past the breaker.
    pub authorised: bool,
    /// Whether the tenant was skipped because the push plane owns it.
    pub yielded: bool,
}

/// Runs one pass for one tenant.
///
/// # Errors
/// Storage failures. A connector failure is not an error — it is an
/// incomplete pass, which is a result this function returns rather than
/// something it raises.
#[tracing::instrument(
    name = "directory_sync.pass",
    skip_all,
    fields(tenant.id = %tenant.id, sync.connector = connector.name()),
    err(Display)
)]
pub async fn run_once(
    state: &AppState,
    tenant: &Tenant,
    connector: &dyn DirectoryConnector,
    config: &SyncConfig,
) -> Result<PassReport> {
    let mut report = PassReport::default();

    if push_plane_owns(state, tenant.id).await? {
        // Said once per pass at debug, and counted, because "this tenant is
        // not being pulled" must be visible without being noise.
        tracing::debug!(tenant.id = %tenant.id, "directory pull yields to a live SCIM credential");
        report.yielded = true;
        return Ok(report);
    }

    // A connector change invalidates every absence beneath it: the new one
    // has never listed anybody, so nobody it fails to list is missing yet.
    let previous = {
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant.id).await?;
        let previous = directory_sync::state(&mut *tx, tenant.id).await?;
        tx.commit().await.map_err(storage)?;
        previous
    };
    if let Some(previous) = &previous
        && previous.connector != connector.name()
    {
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant.id).await?;
        let cleared = directory_sync::reset_absences(&mut *tx, tenant.id).await?;
        tx.commit().await.map_err(storage)?;
        tracing::info!(
            tenant.id = %tenant.id,
            from = previous.connector,
            to = connector.name(),
            cleared,
            "directory connector changed; absence counts forgotten"
        );
    }

    {
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant.id).await?;
        directory_sync::begin_pass(&mut *tx, tenant.id, connector.name()).await?;
        tx.commit().await.map_err(storage)?;
    }

    let enumeration = connector.enumerate().await;
    report.seen = enumeration.snapshot().users.len();
    report.complete = enumeration.is_complete();

    // Presence, unconditionally. This runs for a partial pass too, because
    // the people it listed were listed.
    let seen = record_presence(state, tenant, connector.name(), &enumeration).await?;

    let Enumeration::Complete(_) = &enumeration else {
        let failure = match &enumeration {
            Enumeration::Partial { failure, .. } => failure.clone(),
            Enumeration::Complete(_) => unreachable!(),
        };
        // No `complete_pass`, so `passes_completed` does not move and this
        // pass cannot contribute to anybody's absence count. That omission
        // is the enforcement of decision 3.1.
        tracing::warn!(
            tenant.id = %tenant.id,
            connector = connector.name(),
            seen = report.seen,
            failure = %failure,
            "directory pass incomplete; presence recorded, absence not counted"
        );
        return Ok(report);
    };

    let mut tx = rls::begin_tenant_tx(&state.pool, tenant.id).await?;
    directory_sync::mark_present(&mut *tx, tenant.id, &seen).await?;
    report.absent = directory_sync::mark_absent(&mut *tx, tenant.id, &seen).await?;
    let proposed =
        directory_sync::absent_at_least(&mut *tx, tenant.id, config.absence_passes).await?;
    let live = directory::count_users(&mut *tx, tenant.id).await?;
    tx.commit().await.map_err(storage)?;

    let proposed_count = i32::try_from(proposed.len()).unwrap_or(i32::MAX);
    let mut may_seal = !breaker_trips(proposed.len(), live, config);

    if !may_seal && proposed_count > 0 {
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant.id).await?;
        // Read inside the spending transaction and immediately before the
        // spend, never from the snapshot taken at the top of the pass. An
        // authorisation granted while this pass was enumerating is precisely
        // the one an operator signed *because* of the trip they had just
        // seen, and reading the stale snapshot would spend it while chaining
        // nulls for who signed it and why — losing the only record decision
        // 10 exists to produce.
        let granted = directory_sync::state(&mut *tx, tenant.id)
            .await?
            .and_then(|state| state.authorisation);
        let spent =
            directory_sync::spend_seal_authorisation(&mut *tx, tenant.id, proposed_count).await?;
        if spent {
            audit::record_as(
                &mut tx,
                tenant.id,
                Actor::system(ACTOR),
                AuditAction::DirectorySealAuthorisationUsed,
                format!("tenant {}", tenant.id),
                Outcome::Success,
                json!({
                    "connector": connector.name(),
                    "sealing": proposed_count,
                    "ceiling": granted.as_ref().map(|a| a.ceiling),
                    "granted_by": granted.as_ref().map(|a| a.granted_by.clone()),
                    "reason": granted.as_ref().map(|a| a.reason.clone()),
                }),
            )
            .await?;
            report.authorised = true;
            may_seal = true;
        }
        tx.commit().await.map_err(storage)?;
    }

    if may_seal {
        for user in &proposed {
            seal(state, tenant, connector.name(), user).await?;
            report.sealed += 1;
        }
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant.id).await?;
        directory_sync::complete_pass(&mut *tx, tenant.id, None).await?;
        tx.commit().await.map_err(storage)?;
    } else {
        report.refused = Some(proposed_count);
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant.id).await?;
        directory_sync::complete_pass(&mut *tx, tenant.id, Some(proposed_count)).await?;
        // The one thing on this plane an auditor must not have to notice
        // (ADR-0060 decision 9): a pass that refused to act.
        audit::record_as(
            &mut tx,
            tenant.id,
            Actor::system(ACTOR),
            AuditAction::DirectorySyncBreakerTripped,
            format!("tenant {}", tenant.id),
            // `Failure` and deliberately not `Deny`: `Deny` means the PDP
            // refused (ADR-0019), and a chain query filtering for policy
            // denials must not start returning breaker trips. This is a
            // backstop declining an operation that was otherwise going to
            // proceed — the RLS trip's shape, which `Failure` already names.
            Outcome::Failure,
            json!({
                "connector": connector.name(),
                "would_have_sealed": proposed_count,
                "live_users": live,
                // Basis points, because an audit payload may hold no
                // non-integer number: jsonb re-renders floats and the
                // chain's hash is over the rendered bytes (ADR-0019,
                // `synveda_audit::canonical`). A `0.1` here would have
                // failed the first time a breaker tripped in production,
                // which is the worst possible moment to discover it.
                "fraction_bps": (config.breaker_fraction * 10_000.0).round() as i64,
                "floor": config.breaker_floor,
                "note": "no seal authorisation in force covered this many",
            }),
        )
        .await?;
        tx.commit().await.map_err(storage)?;
        tracing::warn!(
            tenant.id = %tenant.id,
            would_have_sealed = proposed_count,
            live,
            "directory sync breaker refused a bulk seal"
        );
    }

    // A pass that changed nothing chains nothing (decision 9): a quiet
    // tenant's chain must not become a record of the product reading a
    // directory that had not changed.
    if report.sealed > 0 || report.absent > 0 {
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant.id).await?;
        audit::record_as(
            &mut tx,
            tenant.id,
            Actor::system(ACTOR),
            AuditAction::DirectorySyncCompleted,
            format!("tenant {}", tenant.id),
            Outcome::Success,
            json!({
                "connector": connector.name(),
                "seen": report.seen,
                "absent": report.absent,
                "sealed": report.sealed,
                "authorised": report.authorised,
            }),
        )
        .await?;
        tx.commit().await.map_err(storage)?;
    }

    Ok(report)
}

/// The actor every event on this plane carries — ADR-0022's kind for
/// "sweeps and AUTH-4/5 sync jobs", used by the job it named.
const ACTOR: &str = "directory-sync";

fn storage(err: sqlx::Error) -> synveda_types::Error {
    synveda_types::Error::Storage {
        message: format!("directory sync: {err}"),
    }
}

/// Whether the SCIM plane owns this tenant (ADR-0060 decision 5).
///
/// "Live" is issued and not revoked. An **expired** credential still counts,
/// which is the decision's sharp end: expiry is an operational lapse rather
/// than a handover, and a tenant that stops syncing loudly is a better
/// failure than one that silently changes which plane decides who has left.
async fn push_plane_owns(state: &AppState, tenant_id: TenantId) -> Result<bool> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let credentials = directory::credentials(&mut *tx, tenant_id).await?;
    tx.commit().await.map_err(storage)?;
    Ok(credentials
        .iter()
        .any(|credential| credential.revoked_at.is_none()))
}

/// Whether a proposal of this size is too big to act on unaided.
fn breaker_trips(proposed: usize, live_users: i64, config: &SyncConfig) -> bool {
    let proposed = i64::try_from(proposed).unwrap_or(i64::MAX);
    if proposed == 0 {
        return false;
    }
    if proposed <= config.breaker_floor {
        return false;
    }
    #[allow(clippy::cast_precision_loss)]
    let share = proposed as f64 / live_users.max(1) as f64;
    share > config.breaker_fraction
}

/// Writes everybody the pass saw into the mirror and reconciles them.
///
/// Returns the mirror ids of everyone seen, which is what the absence
/// statements take. Runs for a partial pass as well as a complete one.
async fn record_presence(
    state: &AppState,
    tenant: &Tenant,
    connector: &'static str,
    enumeration: &Enumeration,
) -> Result<Vec<DirectoryUserId>> {
    let source = DirectorySource::Pull { connector };
    let mut seen = Vec::with_capacity(enumeration.snapshot().users.len());
    let mut memberships: HashMap<String, Vec<DirectoryUserId>> = HashMap::new();

    for record in &enumeration.snapshot().users {
        let attributes = UserAttributes {
            external_id: Some(record.external_id.clone()),
            user_name: record.user_name.clone(),
            active: record.active,
            display_name: record.display_name.clone(),
            given_name: record.given_name.clone(),
            family_name: record.family_name.clone(),
            work_email: record.work_email.clone(),
        };
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant.id).await?;
        let existing =
            directory::user_by_external_id(&mut *tx, tenant.id, &record.external_id).await?;
        let user = match existing {
            Some(existing) => {
                directory::replace_user(&mut *tx, tenant.id, existing.id, &attributes)
                    .await?
                    .unwrap_or(existing)
            }
            None => {
                directory::create_user(&mut *tx, DirectoryUserId::new(), tenant.id, &attributes)
                    .await?
            }
        };
        tx.commit().await.map_err(storage)?;
        for group in &record.groups {
            memberships.entry(group.clone()).or_default().push(user.id);
        }
        seen.push(user.id);
    }

    sync_groups(state, tenant, &memberships).await?;

    // Reconciled after membership is in place, because the placement
    // resolver reads the mirror's groups rather than anything passed to it
    // (ADR-0059 decision 6). Reconciling first would place every joiner in
    // quarantine and move them out on the following pass.
    for id in &seen {
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant.id).await?;
        let user = directory::user(&mut *tx, tenant.id, *id).await?;
        tx.commit().await.map_err(storage)?;
        if let Some(user) = user {
            reconcile::reconcile(state, tenant, source, &user).await?;
        }
    }
    Ok(seen)
}

/// Makes the mirror's groups and memberships match what the pass read.
async fn sync_groups(
    state: &AppState,
    tenant: &Tenant,
    memberships: &HashMap<String, Vec<DirectoryUserId>>,
) -> Result<()> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant.id).await?;
    for (name, members) in memberships {
        let group = match directory::group_by_display_name(&mut *tx, tenant.id, name).await? {
            Some(group) => group,
            None => {
                directory::create_group(&mut *tx, DirectoryGroupId::new(), tenant.id, None, name)
                    .await?
            }
        };
        let unique: Vec<DirectoryUserId> = members
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        directory::replace_members(&mut tx, tenant.id, group.id, &unique).await?;
    }
    tx.commit().await.map_err(storage)?;
    Ok(())
}

/// Seals one person the way the push plane seals them.
///
/// `active: false` on the mirror, then [`reconcile`] — AUTH-4's own leaver
/// path, which is why this feature adds no second seal. The mirror write is a
/// real change to the resource and takes an ETag bump with it: a SCIM client
/// reading this row afterwards is being told what this product now believes,
/// which is that the directory stopped listing them.
async fn seal(
    state: &AppState,
    tenant: &Tenant,
    connector: &'static str,
    user: &synveda_types::DirectoryUser,
) -> Result<()> {
    let attributes = UserAttributes {
        external_id: user.external_id.clone(),
        user_name: user.user_name.clone(),
        active: false,
        display_name: user.display_name.clone(),
        given_name: user.given_name.clone(),
        family_name: user.family_name.clone(),
        work_email: user.work_email.clone(),
    };
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant.id).await?;
    let deactivated = directory::replace_user(&mut *tx, tenant.id, user.id, &attributes).await?;
    tx.commit().await.map_err(storage)?;
    if let Some(deactivated) = deactivated {
        reconcile::reconcile(
            state,
            tenant,
            DirectorySource::Pull { connector },
            &deactivated,
        )
        .await?;
    }
    Ok(())
}

/// The tenant's own sealed connector, if it has one (TEN-4, ADR-0064
/// decision 9).
///
/// The whole `DirectorySyncConfig` is sealed, not just its secret. Sealing
/// only the credential and reading the endpoints from somewhere else would
/// mean a tenant's credential and the host it is presented to could disagree,
/// which is the one disagreement a directory integration must not have.
///
/// Built per pass rather than cached: a pass is hourly, an unwrap is one
/// cached-key lookup, and a credential an operator rotated should be in use
/// on the next pass rather than after a restart.
#[tracing::instrument(
    name = "directory_sync.stored_connector",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
async fn stored_connector(
    state: &AppState,
    tenant_id: TenantId,
) -> Result<Option<Box<dyn DirectoryConnector>>> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let stored = synveda_store::tenant_secrets::get(
        &mut *tx,
        tenant_id,
        synveda_identity::directory::CREDENTIAL_SECRET_NAME,
    )
    .await?;
    tx.commit()
        .await
        .map_err(|err| synveda_types::Error::Storage {
            message: format!("read a tenant's directory credential: {err}"),
        })?;
    let Some(stored) = stored else {
        return Ok(None);
    };

    let opened = state
        .keys
        .opening_key(
            &state.pool,
            synveda_crypto::KeyScope::Tenant(tenant_id),
            &stored.sealed,
        )
        .await?
        .open(
            synveda_crypto::Purpose::DirectoryCredential,
            synveda_crypto::RowKey::Name(synveda_identity::directory::CREDENTIAL_SECRET_NAME),
            &stored.sealed,
        )
        .inspect_err(|_| {
            metrics::counter!(
                synveda_store::keys::KEY_OPEN_FAILURES_TOTAL,
                "scope" => "tenant",
                "purpose" => synveda_crypto::Purpose::DirectoryCredential.as_str(),
            )
            .increment(1);
        })?;
    // The error deliberately carries no serde detail: a parse failure's
    // message can quote the input, and the input is a credential.
    let config: synveda_identity::directory::DirectorySyncConfig = serde_json::from_slice(&opened)
        .map_err(|_| synveda_types::Error::Invalid {
            message: "a tenant's stored directory credential is not a directory \
                      configuration this build understands"
                .to_string(),
        })?;
    synveda_identity::directory::connector(&config).map(Some)
}

/// One pass over every active tenant a connector serves.
///
/// # Errors
/// Only if the tenant list cannot be read. A tenant whose own pass fails is
/// logged and the sweep carries on, the retention sweep's rule: one
/// customer's directory outage is not every customer's.
pub async fn sweep(
    state: &AppState,
    connectors: &HashMap<TenantId, Box<dyn DirectoryConnector>>,
    config: &SyncConfig,
) -> Result<()> {
    let tenants = tenants::active(&state.pool).await?;
    for tenant in tenants {
        // Precedence, stated in one place (ADR-0064 decision 9): the tenant's
        // own sealed credential, then the deployment's configuration.
        //
        // **A stored credential that fails to open skips the tenant rather
        // than falling back.** Falling back would quietly re-point a customer
        // at whatever directory the deployment happens to configure, on the
        // day their key or their row broke — a different directory's answer
        // to "who still works here", which is the input this feature turns
        // into departures.
        let stored = match stored_connector(state, tenant.id).await {
            Ok(stored) => stored,
            Err(error) => {
                tracing::warn!(
                    tenant.id = %tenant.id,
                    %error,
                    "a tenant's stored directory credential could not be used; \
                     skipping rather than falling back to the deployment's"
                );
                continue;
            }
        };
        let connector = match stored.as_deref() {
            Some(connector) => connector,
            None => match connectors.get(&tenant.id) {
                Some(connector) => connector.as_ref(),
                None => continue,
            },
        };
        if let Err(error) = run_once(state, &tenant, connector, config).await {
            tracing::warn!(tenant.id = %tenant.id, error = %error, "directory sync pass failed");
        }
    }
    Ok(())
}

/// Spawns the sync loop — the pack refresher's shape. Abort the handle on
/// shutdown.
#[must_use]
pub fn spawn(
    state: AppState,
    connectors: HashMap<TenantId, Box<dyn DirectoryConnector>>,
    config: SyncConfig,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(config.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(error) = sweep(&state, &connectors, &config).await {
                tracing::warn!(error = %error, "directory sync sweep failed");
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> SyncConfig {
        SyncConfig {
            breaker_fraction: 0.10,
            breaker_floor: 5,
            ..SyncConfig::default()
        }
    }

    #[test]
    fn the_breaker_sizes_a_proposal_against_the_tenant() {
        let config = config();
        // Nobody proposed is not a refusal; it is a quiet week.
        assert!(!breaker_trips(0, 1000, &config));
        // Under the floor, the fraction does not apply at all — otherwise a
        // six-person company trips on its first leaver.
        assert!(!breaker_trips(5, 6, &config));
        // Over the floor and inside the share: an ordinary month.
        assert!(!breaker_trips(50, 1000, &config));
        // Over the floor and over the share: the case a person decides.
        assert!(breaker_trips(300, 1000, &config));
        // The pathological one the breaker exists for: a directory that
        // completed a pass and listed nobody.
        assert!(breaker_trips(1000, 1000, &config));
    }

    #[test]
    fn a_tenant_with_no_users_does_not_divide_by_zero() {
        // `live_users` is a count taken in the same transaction, and a
        // tenant can legitimately be empty. The guard is `max(1)`, asserted
        // rather than trusted because a panic here would take the whole
        // sweep down for every other tenant.
        assert!(breaker_trips(6, 0, &config()));
    }
}
