//! Operator tenant suspension, resumption and dry-run inventory (ADR-0157).
//!
//! Suspension and resumption run under the exact migrator principal and hold
//! the tenant's exclusive admission lock for the transition. Resumption first
//! proves that the tenant's stored Cedar packs compile with this build and
//! that the configured KMS unwraps every stored tenant key generation; the
//! KMS calls happen before the transition transaction, which then re-reads
//! the same key rows.

use serde_json::json;
use synveda_audit::AuditAction;
use synveda_crypto::{KeyManagement as _, KeyScope};
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

/// Resumes a suspended tenant after revalidating its policy and keys.
pub async fn resume(
    pool: &sqlx::PgPool,
    roles: &synveda_store::runtime_role::DatabaseRoles,
    tenant: TenantId,
    reason: ReasonCode,
) -> Result<(), String> {
    let keys = verified_key_generations(pool, tenant).await?;
    let changed = transition(pool, roles, tenant, Transition::Resume, reason, Some(&keys)).await?;
    print_transition(tenant, Transition::Resume, changed);
    Ok(())
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
#[derive(Debug, PartialEq, Eq)]
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
    let current = synveda_store::tenants::by_id(&mut *tx, tenant)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "tenant_not_found: no tenant has that id".to_owned())?;
    if tenant_lifecycle::plan(current.status, transition) == Plan::AlreadyThere {
        tx.rollback().await.map_err(|error| error.to_string())?;
        return Ok(false);
    }
    let evidence = match verified_keys {
        Some(keys) => Some(revalidate(&mut tx, tenant, keys).await?),
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
) -> Result<(i64, i64), String> {
    let packs = synveda_store::policy_packs::stored(&mut *tx, tenant)
        .await
        .map_err(|error| error.to_string())?;
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
    let current: Vec<KeyGeneration> = synveda_store::keys::tenant_all(&mut *tx, tenant)
        .await
        .map_err(|error| error.to_string())?
        .iter()
        .map(key_generation)
        .collect();
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
