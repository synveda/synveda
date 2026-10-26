//! Operator tenant suspension, resumption and dry-run inventory (ADR-0157).
//!
//! Suspension and resumption run under the exact migrator principal and hold
//! the tenant's exclusive admission lock for the transition. Resumption first
//! proves that the tenant's stored Cedar packs compile with this build and
//! that the configured KMS unwraps every stored tenant key generation; the
//! KMS calls happen before the transition transaction, which then re-reads
//! the same key rows.
//!
//! The migrator transaction's snapshot starts with its authority proof,
//! before the lock wait, so a custody write committed during that wait is
//! invisible to it. Resumption therefore compares its view with a snapshot
//! taken after the lock is held and refuses on any difference.
//!
//! Erasure runs each bounded step in its own migrator transaction under the
//! same exclusive lock, so an interrupted run commits whole steps only and the
//! next run continues from the recorded cursor.

use std::time::{Duration, Instant};

use serde_json::json;
use synveda_audit::AuditAction;
use synveda_crypto::{KeyManagement as _, KeyScope};
use synveda_store::tenant_erasure::{self, Start, StepLimits};
use synveda_store::tenant_inventory::{self, DEPLOYMENT_TABLES, EXTERNAL_OBJECTS, TableCount};
use synveda_store::tenant_lifecycle::{self, Plan, Transition};
use synveda_types::{TenantId, TenantStatus};

/// Why an operator changed a tenant's lifecycle state. Closed so the audit
/// chain never carries free text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ReasonCode {
    /// Security or operational incident response.
    Incident,
    /// Planned maintenance or recovery rehearsal.
    Maintenance,
    /// Customer offboarding.
    Offboarding,
    /// Restore or disaster recovery.
    Recovery,
    /// Any other deployment administration.
    Administrative,
}

impl ReasonCode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Incident => "incident",
            Self::Maintenance => "maintenance",
            Self::Offboarding => "offboarding",
            Self::Recovery => "recovery",
            Self::Administrative => "administrative",
        }
    }
}

/// Default per-table cap for the dry-run inventory.
pub const DEFAULT_INVENTORY_CAP: u64 = 1_000_000;

/// Default rows one erasure statement deletes.
pub const DEFAULT_ERASURE_CHUNK: u32 = 2_000;
/// Default bound on one erasure run.
pub const DEFAULT_ERASURE_TIMEOUT_SECONDS: u64 = 3_600;
/// Longest erasure run one invocation may request.
pub const MAX_ERASURE_TIMEOUT_SECONDS: u64 = 86_400;
/// The erasure lease each step renews.
const ERASURE_LEASE: Duration = Duration::from_secs(30);
/// The statement bound inside each erasure step.
const ERASURE_STATEMENT_TIMEOUT: Duration = Duration::from_secs(120);
/// Steps one run may take; each deletes a chunk or finishes a unit.
const MAX_ERASURE_STEPS: u32 = 1_000_000;

/// Suspends a tenant, or reports that it already is.
pub async fn suspend(
    pool: &sqlx::PgPool,
    roles: &synveda_store::runtime_role::DatabaseRoles,
    tenant: TenantId,
    reason: ReasonCode,
) -> Result<(), String> {
    let changed = transition(pool, roles, tenant, Transition::Suspend, reason, None).await?;
    print_transition(tenant, Transition::Suspend, changed);
    Ok(())
}

/// What an operator acknowledged before erasure may start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EraseOptions {
    /// Erasure is irreversible.
    pub force: bool,
    /// Retention follows the interim fixture rules of ADR-0157.
    pub fixture_rules: bool,
    /// Rows one statement deletes.
    pub chunk_rows: u32,
    /// Bound on this run; a later run continues.
    pub timeout_seconds: u64,
}

/// Erases a suspended tenant, or continues or reports an earlier erasure.
pub async fn erase(
    pool: &sqlx::PgPool,
    roles: &synveda_store::runtime_role::DatabaseRoles,
    tenant: TenantId,
    reason: ReasonCode,
    options: EraseOptions,
) -> Result<(), String> {
    if !options.force || !options.fixture_rules {
        return Err(
            "erasure_unacknowledged: erasure is irreversible and follows the interim fixture rules of ADR-0157; pass --force and --fixture-rules"
                .to_owned(),
        );
    }
    if options.chunk_rows == 0 || options.chunk_rows > tenant_erasure::MAX_CHUNK_ROWS {
        return Err(format!(
            "--chunk-rows must be between 1 and {}",
            tenant_erasure::MAX_CHUNK_ROWS
        ));
    }
    if options.timeout_seconds == 0 || options.timeout_seconds > MAX_ERASURE_TIMEOUT_SECONDS {
        return Err(format!(
            "--timeout-seconds must be between 1 and {MAX_ERASURE_TIMEOUT_SECONDS}"
        ));
    }
    let deadline = Instant::now() + Duration::from_secs(options.timeout_seconds);
    let limits = StepLimits {
        chunk: options.chunk_rows,
        lease: ERASURE_LEASE,
        statement_timeout: ERASURE_STATEMENT_TIMEOUT,
    };
    let worker = format!("synveda-cli:{}", uuid::Uuid::now_v7());
    let Some((plan, lease)) = start_erasure(pool, roles, tenant, reason, &worker).await? else {
        return Ok(());
    };
    let mut finished = false;
    for _ in 0..MAX_ERASURE_STEPS {
        if Instant::now() >= deadline {
            release_erasure(pool, roles, tenant, &lease).await?;
            return Err(
                "erasure_timeout: --timeout-seconds elapsed; the committed steps stand, rerun to continue"
                    .to_owned(),
            );
        }
        let mut tx = locked_migrator_tx(pool, roles, tenant).await?;
        let progress = tenant_erasure::step(&mut tx, tenant, &plan, &lease, limits)
            .await
            .map_err(|error| error.to_string())?;
        tx.commit().await.map_err(|error| error.to_string())?;
        if let Some(label) = &progress.label {
            eprintln!(
                "erasing {}/{} {label}: {} rows",
                progress.unit, progress.units, progress.deleted
            );
        }
        if progress.finished {
            finished = true;
            break;
        }
    }
    if !finished {
        release_erasure(pool, roles, tenant, &lease).await?;
        return Err(
            "erasure_incomplete: the step bound elapsed; the committed steps stand, rerun to continue"
                .to_owned(),
        );
    }
    let mut tx = locked_migrator_tx(pool, roles, tenant).await?;
    let receipt = tenant_erasure::finish(&mut tx, tenant, &plan, &lease, limits)
        .await
        .map_err(|error| error.to_string())?;
    crate::record_break_glass(
        &mut tx,
        tenant,
        AuditAction::TenantErased,
        format!("tenant {tenant}"),
        json!({
            "operation": lease.operation,
            "reason": reason.as_str(),
            "receipt_digest": receipt["digest"],
            "deleted_rows": receipt["deleted_rows"],
            "attempts": lease.attempt,
        }),
    )
    .await?;
    tx.commit().await.map_err(|error| error.to_string())?;
    print_erasure(tenant, true, &receipt);
    Ok(())
}

/// Starts or reclaims the erasure operation. `None` means there is nothing
/// left to run: the tenant was already erased and its receipt was printed.
async fn start_erasure(
    pool: &sqlx::PgPool,
    roles: &synveda_store::runtime_role::DatabaseRoles,
    tenant: TenantId,
    reason: ReasonCode,
    worker: &str,
) -> Result<Option<(tenant_erasure::Plan, tenant_erasure::Lease)>, String> {
    let mut tx = locked_migrator_tx(pool, roles, tenant).await?;
    let plan = tenant_erasure::plan(&mut tx)
        .await
        .map_err(|error| error.to_string())?;
    let start = tenant_erasure::start(&mut tx, tenant, &plan, worker, ERASURE_LEASE)
        .await
        .map_err(|error| error.to_string())?;
    match start {
        Start::Erased(receipt) => {
            tx.rollback().await.map_err(|error| error.to_string())?;
            print_erasure(tenant, false, &receipt);
            Ok(None)
        }
        Start::Held {
            operation,
            revisions,
        } => {
            crate::record_break_glass(
                &mut tx,
                tenant,
                AuditAction::TenantErasureHeld,
                format!("tenant {tenant}"),
                json!({
                    "operation": operation,
                    "reason": reason.as_str(),
                    "held_revisions": revisions,
                }),
            )
            .await?;
            tx.commit().await.map_err(|error| error.to_string())?;
            Err(format!(
                "{}: {revisions} Knowledge revisions are under a legal or retention hold; nothing was deleted",
                tenant_erasure::ERASURE_HELD
            ))
        }
        Start::Started(lease) => {
            crate::record_break_glass(
                &mut tx,
                tenant,
                AuditAction::TenantErasureStarted,
                format!("tenant {tenant}"),
                json!({
                    "operation": lease.operation,
                    "reason": reason.as_str(),
                    "plan_digest": plan.digest(),
                    "units": plan.units().len(),
                }),
            )
            .await?;
            tx.commit().await.map_err(|error| error.to_string())?;
            Ok(Some((plan, lease)))
        }
        Start::Resumed(lease) => {
            tx.commit().await.map_err(|error| error.to_string())?;
            Ok(Some((plan, lease)))
        }
    }
}

async fn release_erasure(
    pool: &sqlx::PgPool,
    roles: &synveda_store::runtime_role::DatabaseRoles,
    tenant: TenantId,
    lease: &tenant_erasure::Lease,
) -> Result<(), String> {
    let mut tx = locked_migrator_tx(pool, roles, tenant).await?;
    tenant_erasure::release(&mut tx, tenant, lease)
        .await
        .map_err(|error| error.to_string())?;
    tx.commit().await.map_err(|error| error.to_string())
}

async fn locked_migrator_tx(
    pool: &sqlx::PgPool,
    roles: &synveda_store::runtime_role::DatabaseRoles,
    tenant: TenantId,
) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, String> {
    let mut tx = synveda_store::rls::begin_migrator_tenant_tx(pool, tenant, roles)
        .await
        .map_err(|error| {
            format!(
                "tenant lifecycle requires the exact configured migrator principal, database target and schema epoch: {error}"
            )
        })?;
    tenant_lifecycle::lock_exclusive(&mut tx, tenant)
        .await
        .map_err(|error| error.to_string())?;
    Ok(tx)
}

fn print_erasure(tenant: TenantId, changed: bool, receipt: &serde_json::Value) {
    println!(
        "{}",
        json!({
            "tenant": tenant,
            "status": TenantStatus::Erased,
            "changed": changed,
            "receipt": receipt,
        })
    );
}

/// Resumes a suspended tenant after revalidating its policy and keys.
pub async fn resume(
    pool: &sqlx::PgPool,
    roles: &synveda_store::runtime_role::DatabaseRoles,
    tenant: TenantId,
    reason: ReasonCode,
) -> Result<(), String> {
    refuse_after_erasure(pool, tenant).await?;
    let keys = verified_key_generations(pool, tenant).await?;
    let changed = transition(pool, roles, tenant, Transition::Resume, reason, Some(&keys)).await?;
    print_transition(tenant, Transition::Resume, changed);
    Ok(())
}

/// Names the one-way state before resumption reads keys through custody,
/// which an erasing or erased tenant no longer admits.
async fn refuse_after_erasure(pool: &sqlx::PgPool, tenant: TenantId) -> Result<(), String> {
    let mut tx = synveda_store::rls::begin_tenant_snapshot_tx(pool, tenant)
        .await
        .map_err(|error| error.to_string())?;
    let status = synveda_store::tenants::by_id(&mut *tx, tenant)
        .await
        .map_err(|error| error.to_string())?
        .map(|row| row.status);
    tx.rollback().await.map_err(|error| error.to_string())?;
    match status.map(|status| tenant_lifecycle::plan(status, Transition::Resume)) {
        Some(Plan::Refused(reason)) => Err(format!(
            "{reason}: erasure is one-way; the tenant cannot be suspended or resumed"
        )),
        _ => Ok(()),
    }
}

fn print_transition(tenant: TenantId, transition: Transition, changed: bool) {
    println!(
        "{}",
        json!({
            "tenant": tenant,
            "status": transition.to(),
            "changed": changed,
        })
    );
}

/// One stored tenant key generation as the KMS proved it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct KeyGeneration {
    version: u32,
    wrapped: Vec<u8>,
}

async fn verified_key_generations(
    pool: &sqlx::PgPool,
    tenant: TenantId,
) -> Result<Vec<KeyGeneration>, String> {
    let stored = stored_key_generations(pool, tenant).await?;
    if stored.is_empty() {
        return Ok(Vec::new());
    }
    let kms = crate::keys::kms_from_env()?;
    let mut verified = Vec::with_capacity(stored.len());
    for key in &stored {
        if key.kek_ref != kms.key_ref() {
            return Err(format!(
                "key_unavailable: tenant key generation {} names KEK {} and the configured KEK is {}",
                key.version.get(),
                key.kek_ref,
                kms.key_ref()
            ));
        }
        kms.unwrap_key(KeyScope::Tenant(tenant), &key.wrapped_dek)
            .await
            .map_err(|_| {
                format!(
                    "key_unavailable: the configured KMS does not open tenant key generation {}",
                    key.version.get()
                )
            })?;
        verified.push(key_generation(key));
    }
    Ok(verified)
}

async fn stored_key_generations(
    pool: &sqlx::PgPool,
    tenant: TenantId,
) -> Result<Vec<synveda_store::keys::StoredKey>, String> {
    let mut tx = synveda_store::rls::begin_tenant_custody_tx(pool, tenant)
        .await
        .map_err(|error| format!("tenant_not_found: {error}"))?;
    let keys = synveda_store::keys::tenant_all(&mut *tx, tenant)
        .await
        .map_err(|error| error.to_string())?;
    tx.rollback().await.map_err(|error| error.to_string())?;
    Ok(keys)
}

fn key_generation(key: &synveda_store::keys::StoredKey) -> KeyGeneration {
    KeyGeneration {
        version: key.version.get(),
        wrapped: key.wrapped_dek.clone(),
    }
}

async fn transition(
    pool: &sqlx::PgPool,
    roles: &synveda_store::runtime_role::DatabaseRoles,
    tenant: TenantId,
    transition: Transition,
    reason: ReasonCode,
    verified_keys: Option<&[KeyGeneration]>,
) -> Result<bool, String> {
    let mut tx = locked_migrator_tx(pool, roles, tenant).await?;
    let current = synveda_store::tenants::by_id(&mut *tx, tenant)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "tenant_not_found: no tenant has that id".to_owned())?;
    match tenant_lifecycle::plan(current.status, transition) {
        Plan::AlreadyThere => {
            tx.rollback().await.map_err(|error| error.to_string())?;
            return Ok(false);
        }
        Plan::Refused(reason) => {
            return Err(format!(
                "{reason}: erasure is one-way; the tenant cannot be suspended or resumed"
            ));
        }
        Plan::Apply => {}
    }
    let evidence = match verified_keys {
        Some(keys) => {
            let fresh = committed_view(pool, tenant).await?;
            Some(revalidate(&mut tx, tenant, keys, &fresh).await?)
        }
        None => None,
    };
    if !tenant_lifecycle::apply(&mut tx, tenant, transition)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err(format!(
            "{}: the tenant left the required status",
            tenant_lifecycle::TENANT_STATE_CHANGED
        ));
    }
    let mut payload = json!({
        "from": current.status,
        "to": transition.to(),
        "reason": reason.as_str(),
    });
    if let (Some((packs, keys)), Some(object)) = (evidence, payload.as_object_mut()) {
        object.insert("policy_packs".to_owned(), packs.into());
        object.insert("key_generations".to_owned(), keys.into());
    }
    crate::record_break_glass(
        &mut tx,
        tenant,
        audit_action(transition),
        format!("tenant {tenant}"),
        payload,
    )
    .await?;
    tx.commit().await.map_err(|error| error.to_string())?;
    Ok(true)
}

const fn audit_action(transition: Transition) -> AuditAction {
    match transition {
        Transition::Suspend => AuditAction::TenantSuspended,
        Transition::Resume => AuditAction::TenantResumed,
    }
}

/// Proves under the exclusive lock that the tenant's stored packs compile
/// and that its key rows are the ones the KMS opened. Returns the pack and
/// key-generation counts for the audit payload.
async fn revalidate(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    verified_keys: &[KeyGeneration],
    committed: &AuthorityView,
) -> Result<(i64, i64), String> {
    let packs = synveda_store::policy_packs::stored(&mut *tx, tenant)
        .await
        .map_err(|error| error.to_string())?;
    let current: Vec<KeyGeneration> = synveda_store::keys::tenant_all(&mut *tx, tenant)
        .await
        .map_err(|error| error.to_string())?
        .iter()
        .map(key_generation)
        .collect();
    if &authority_view(&packs, &current) != committed {
        return Err(format!(
            "{}: the tenant's keys or policy packs changed while it was resumed; retry",
            tenant_lifecycle::TENANT_STATE_CHANGED
        ));
    }
    let pdp = synveda_policy::Pdp::new().map_err(|error| error.to_string())?;
    for pack in &packs {
        pdp.install_source(
            tenant,
            &pack.name,
            pack.version,
            &pack.source,
            pack.config.clone(),
        )
        .map_err(|_| {
            format!(
                "policy_invalid: stored policy pack {} version {} does not compile with this build; repair it with `synveda policy apply` and retry",
                pack.name, pack.version
            )
        })?;
    }
    if current != verified_keys {
        return Err(
            "key_unavailable: the tenant's key generations changed while they were verified; retry"
                .to_owned(),
        );
    }
    Ok((
        i64::try_from(packs.len()).unwrap_or(i64::MAX),
        i64::try_from(current.len()).unwrap_or(i64::MAX),
    ))
}

/// The stored pack versions and key generations resumption depends on.
#[derive(Debug, PartialEq, Eq)]
struct AuthorityView {
    packs: Vec<(String, i64)>,
    keys: Vec<KeyGeneration>,
}

fn authority_view(
    packs: &[synveda_store::policy_packs::PolicyPack],
    keys: &[KeyGeneration],
) -> AuthorityView {
    AuthorityView {
        packs: packs
            .iter()
            .map(|pack| (pack.name.clone(), pack.version))
            .collect(),
        keys: keys.to_vec(),
    }
}

/// Reads the committed view through a snapshot taken while the caller holds
/// the exclusive admission lock, so no tenant write is in flight.
async fn committed_view(pool: &sqlx::PgPool, tenant: TenantId) -> Result<AuthorityView, String> {
    let mut tx = synveda_store::rls::begin_tenant_snapshot_tx(pool, tenant)
        .await
        .map_err(|error| error.to_string())?;
    let packs = synveda_store::policy_packs::stored(&mut *tx, tenant)
        .await
        .map_err(|error| error.to_string())?;
    let keys: Vec<KeyGeneration> = synveda_store::keys::tenant_all(&mut *tx, tenant)
        .await
        .map_err(|error| error.to_string())?
        .iter()
        .map(key_generation)
        .collect();
    tx.rollback().await.map_err(|error| error.to_string())?;
    Ok(authority_view(&packs, &keys))
}

/// Prints a dry-run inventory of one tenant. Changes nothing.
pub async fn inventory(
    pool: &sqlx::PgPool,
    tenant: TenantId,
    cap: u64,
    json_output: bool,
) -> Result<(), String> {
    let mut tx = synveda_store::rls::begin_tenant_snapshot_tx(pool, tenant)
        .await
        .map_err(|error| error.to_string())?;
    let status = synveda_store::tenants::by_id(&mut *tx, tenant)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "tenant_not_found: no tenant has that id".to_owned())?
        .status;
    let counts = tenant_inventory::count_tenant_rows(&mut tx, tenant, cap)
        .await
        .map_err(|error| error.to_string())?;
    tx.rollback().await.map_err(|error| error.to_string())?;
    let report = inventory_report(tenant, status, cap, &counts);
    if json_output {
        println!("{report}");
    } else {
        print_inventory(&report, &counts);
    }
    Ok(())
}

fn inventory_report(
    tenant: TenantId,
    status: TenantStatus,
    cap: u64,
    counts: &[TableCount],
) -> serde_json::Value {
    let total: u64 = counts.iter().map(|count| count.rows).sum();
    let occupied = counts.iter().filter(|count| count.rows > 0).count();
    json!({
        "format": "synveda.tenant-inventory.v1",
        "tenant": tenant,
        "status": status,
        "cap_per_table": cap,
        "tables": counts,
        "tenant_tables": counts.len(),
        "occupied_tables": occupied,
        "rows": total,
        "capped": counts.iter().any(|count| count.capped),
        "deployment_tables": DEPLOYMENT_TABLES,
        "external_objects": EXTERNAL_OBJECTS,
    })
}

fn print_inventory(report: &serde_json::Value, counts: &[TableCount]) {
    println!(
        "tenant {} is {}: {} rows across {} of {} tenant tables",
        report["tenant"].as_str().unwrap_or_default(),
        report["status"].as_str().unwrap_or_default(),
        report["rows"],
        report["occupied_tables"],
        report["tenant_tables"],
    );
    for count in counts.iter().filter(|count| count.rows > 0) {
        let marker = if count.capped { "+" } else { "" };
        println!(
            "  {:<40} {:>10}{marker}  archive={:?} erasure={:?}",
            count.table, count.rows, count.archive, count.erasure
        );
    }
    println!("external copies outside this database:");
    for object in &EXTERNAL_OBJECTS {
        println!(
            "  {:<28} {:?}: {}",
            object.name, object.owner, object.treatment
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reason_codes_are_closed_lowercase_words() {
        for reason in [
            ReasonCode::Incident,
            ReasonCode::Maintenance,
            ReasonCode::Offboarding,
            ReasonCode::Recovery,
            ReasonCode::Administrative,
        ] {
            assert!(
                reason
                    .as_str()
                    .chars()
                    .all(|character| character.is_ascii_lowercase())
            );
        }
    }

    #[test]
    fn the_report_names_every_external_copy_and_counts_only_rows() {
        let tenant = TenantId::new();
        let counts = [TableCount {
            table: "sessions",
            family: synveda_store::tenant_inventory::Family::Sessions,
            archive: synveda_store::tenant_inventory::Archive::Portable,
            erasure: synveda_store::tenant_inventory::Erasure::Erase,
            rows: 3,
            capped: false,
        }];
        let report = inventory_report(tenant, TenantStatus::Suspended, 10, &counts);
        assert_eq!(report["rows"], 3);
        assert_eq!(report["status"], "suspended");
        assert_eq!(
            report["external_objects"].as_array().map(Vec::len),
            Some(EXTERNAL_OBJECTS.len())
        );
    }
}
