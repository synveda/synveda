//! Deployment KEK replacement.
//!
//! The local KMS holds one KEK and the ring unwraps every stored generation
//! with it, so a replacement rewraps every deployment and tenant generation,
//! retired ones included. Each row moves by compare-and-set from the previous
//! KEK reference to the replacement: an interrupted run resumes, and a row
//! wrapped by any third KEK stops the ceremony. Backups taken before the
//! ceremony still need the previous KEK, so its custody must outlive them.

use synveda_crypto::{KeyManagement, KeyScope, LocalKms};
use synveda_store::keys::StoredKey;
use synveda_types::TenantId;
use zeroize::Zeroizing;

const TARGET_KEY: &str = "SYNVEDA_KMS_KEY";
const TARGET_KEY_REF: &str = "SYNVEDA_KMS_KEY_REF";
const PREVIOUS_KEY: &str = "SYNVEDA_KMS_PREVIOUS_KEY";
const PREVIOUS_KEY_REF: &str = "SYNVEDA_KMS_PREVIOUS_KEY_REF";
const MAX_TENANTS: usize = 10_000;

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Current,
    Rewrap,
}

fn step(row_ref: &str, previous: &str, target: &str) -> Result<Step, String> {
    if row_ref == target {
        Ok(Step::Current)
    } else if row_ref == previous {
        Ok(Step::Rewrap)
    } else {
        Err("a stored key generation is wrapped by a KEK reference that is neither the previous nor the replacement".to_owned())
    }
}

struct Keks {
    previous: LocalKms,
    target: LocalKms,
}

fn required(name: &str) -> Result<Zeroizing<String>, String> {
    crate::settings::sensitive_setting(name)?
        .map(|value| Zeroizing::new(value.trim().to_owned()))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{name} or {name}_FILE is required for KEK replacement"))
}

fn keks_from_env() -> Result<Keks, String> {
    let target = required(TARGET_KEY)?;
    let target_ref = required(TARGET_KEY_REF)?;
    let previous = required(PREVIOUS_KEY)?;
    let previous_ref = required(PREVIOUS_KEY_REF)?;
    if previous_ref.as_str() == target_ref.as_str() {
        return Err("the replacement KEK needs a reference distinct from the previous one".into());
    }
    if previous.eq_ignore_ascii_case(&target) {
        return Err("the replacement KEK must differ from the previous KEK".to_owned());
    }
    let local = |hex: &str, key_ref: &str, name: &str| {
        LocalKms::from_hex(hex, key_ref).map_err(|_| format!("{name} is not a usable KEK"))
    };
    Ok(Keks {
        previous: local(&previous, &previous_ref, PREVIOUS_KEY)?,
        target: local(&target, &target_ref, TARGET_KEY)?,
    })
}

/// Rewrapped bytes for every row still on the previous KEK, after proving
/// that rows already on the replacement open under it.
async fn plan(
    keks: &Keks,
    scope: KeyScope,
    rows: &[StoredKey],
) -> Result<Vec<(usize, Vec<u8>)>, String> {
    let mut updates = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let version = row.version.get();
        match step(&row.kek_ref, keks.previous.key_ref(), keks.target.key_ref())? {
            Step::Current => {
                keks.target
                    .unwrap_key(scope, &row.wrapped_dek)
                    .await
                    .map_err(|_| {
                        format!("{scope} generation {version} names the replacement KEK but does not open under it")
                    })?;
            }
            Step::Rewrap => {
                let key = keks
                    .previous
                    .unwrap_key(scope, &row.wrapped_dek)
                    .await
                    .map_err(|_| {
                        format!("{scope} generation {version} did not open under the previous KEK")
                    })?;
                let wrapped =
                    keks.target.wrap_key(scope, &key).await.map_err(|_| {
                        format!("{scope} generation {version} could not be rewrapped")
                    })?;
                updates.push((index, wrapped));
            }
        }
    }
    Ok(updates)
}

async fn rewrap_deployment(pool: &sqlx::PgPool, keks: &Keks) -> Result<(usize, usize), String> {
    let mut tx = pool.begin().await.map_err(|err| err.to_string())?;
    let rows = synveda_store::keys::deployment_all(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
    let updates = plan(keks, KeyScope::Deployment, &rows).await?;
    for (index, wrapped) in &updates {
        let changed = synveda_store::keys::deployment_rewrap(
            &mut *tx,
            &rows[*index],
            wrapped,
            keks.target.key_ref(),
        )
        .await
        .map_err(|err| err.to_string())?;
        if !changed {
            return Err("a deployment key generation changed during KEK replacement".to_owned());
        }
    }
    tx.commit().await.map_err(|err| err.to_string())?;
    Ok((rows.len(), updates.len()))
}

async fn rewrap_tenant(
    pool: &sqlx::PgPool,
    keks: &Keks,
    tenant: TenantId,
) -> Result<(usize, usize), String> {
    let mut tx = synveda_store::rls::begin_tenant_custody_tx(pool, tenant)
        .await
        .map_err(|err| err.to_string())?;
    let rows = synveda_store::keys::tenant_all(&mut *tx, tenant)
        .await
        .map_err(|err| err.to_string())?;
    let updates = plan(keks, KeyScope::Tenant(tenant), &rows).await?;
    for (index, wrapped) in &updates {
        let changed = synveda_store::keys::tenant_rewrap(
            &mut *tx,
            tenant,
            &rows[*index],
            wrapped,
            keks.target.key_ref(),
        )
        .await
        .map_err(|err| err.to_string())?;
        if !changed {
            return Err(format!(
                "tenant {tenant} key generation changed during KEK replacement"
            ));
        }
    }
    if !updates.is_empty() {
        crate::record_break_glass(
            &mut tx,
            tenant,
            synveda_audit::AuditAction::TenantKeyRewrapped,
            format!("tenant {tenant} key"),
            serde_json::json!({
                "from_kek_ref": keks.previous.key_ref(),
                "kek_ref": keks.target.key_ref(),
                "generations_rewrapped": updates.len(),
                "generations_total": rows.len(),
            }),
        )
        .await?;
    }
    tx.commit().await.map_err(|err| err.to_string())?;
    Ok((rows.len(), updates.len()))
}

/// Proves that every generation of `rows` names and opens under `kms`.
pub(crate) async fn require_generations(
    kms: &impl KeyManagement,
    scope: KeyScope,
    rows: &[StoredKey],
) -> Result<(), String> {
    for row in rows {
        if row.kek_ref != kms.key_ref() {
            return Err(format!(
                "{scope} generation {} is wrapped by a KEK reference this custody set does not hold",
                row.version.get()
            ));
        }
        kms.unwrap_key(scope, &row.wrapped_dek).await.map_err(|_| {
            format!(
                "{scope} generation {} did not open under the configured KEK",
                row.version.get()
            )
        })?;
    }
    Ok(())
}

async fn verify_replacement(
    pool: &sqlx::PgPool,
    keks: &Keks,
    tenants: &[TenantId],
) -> Result<(), String> {
    let rows = synveda_store::keys::deployment_all(pool)
        .await
        .map_err(|err| err.to_string())?;
    require_generations(&keks.target, KeyScope::Deployment, &rows).await?;
    for tenant in tenants {
        let mut tx = synveda_store::rls::begin_tenant_custody_tx(pool, *tenant)
            .await
            .map_err(|err| err.to_string())?;
        let rows = synveda_store::keys::tenant_all(&mut *tx, *tenant)
            .await
            .map_err(|err| err.to_string())?;
        tx.commit().await.map_err(|err| err.to_string())?;
        require_generations(&keks.target, KeyScope::Tenant(*tenant), &rows).await?;
    }
    Ok(())
}

/// Rewraps every stored data-key generation from the previous KEK to the
/// configured replacement, then proves that all of them open under it.
///
/// # Errors
/// Missing or identical KEKs, a generation wrapped by an unknown KEK, a row
/// that changed concurrently, or a storage failure. Completed tenants stay
/// rewrapped and a rerun resumes from the remainder.
pub async fn rewrap(pool: &sqlx::PgPool) -> Result<(), String> {
    let keks = keks_from_env()?;
    let (deployment_total, deployment_rewrapped) = rewrap_deployment(pool, &keks).await?;
    let tenants = synveda_store::tenants::all_ids(pool, MAX_TENANTS)
        .await
        .map_err(|err| err.to_string())?;
    let (mut tenant_total, mut tenant_rewrapped) = (0_usize, 0_usize);
    for tenant in &tenants {
        let (total, rewrapped) = rewrap_tenant(pool, &keks, *tenant).await?;
        tenant_total += total;
        tenant_rewrapped += rewrapped;
    }
    verify_replacement(pool, &keks, &tenants).await?;
    println!(
        "{}",
        serde_json::json!({
            "previous_kek_ref": keks.previous.key_ref(),
            "kek_ref": keks.target.key_ref(),
            "deployment_generations": deployment_total,
            "deployment_rewrapped": deployment_rewrapped,
            "tenants": tenants.len(),
            "tenant_generations": tenant_total,
            "tenant_rewrapped": tenant_rewrapped,
            "note": "retain the previous KEK with every backup taken before this replacement",
        })
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use synveda_crypto::{DataKey, KeyVersion};

    use super::*;

    fn kms(key_ref: &str) -> LocalKms {
        LocalKms::new(DataKey::generate().expect("kek"), key_ref).expect("kms")
    }

    async fn stored(kms: &LocalKms, scope: KeyScope, version: u32) -> StoredKey {
        let dek = DataKey::generate().expect("dek");
        StoredKey {
            version: KeyVersion::new(version).expect("version"),
            wrapped_dek: kms.wrap_key(scope, &dek).await.expect("wrap"),
            kek_ref: kms.key_ref().to_owned(),
            retired: version == 1,
        }
    }

    #[test]
    fn a_row_moves_only_from_the_previous_reference() {
        assert_eq!(
            step("local:new", "local:old", "local:new"),
            Ok(Step::Current)
        );
        assert_eq!(
            step("local:old", "local:old", "local:new"),
            Ok(Step::Rewrap)
        );
        assert!(step("local:third", "local:old", "local:new").is_err());
    }

    #[tokio::test]
    async fn the_plan_rewraps_retired_and_current_generations_under_the_replacement() {
        let keks = Keks {
            previous: kms("local:old"),
            target: kms("local:new"),
        };
        let scope = KeyScope::Tenant(TenantId::new());
        let rows = vec![
            stored(&keks.previous, scope, 1).await,
            stored(&keks.target, scope, 2).await,
            stored(&keks.previous, scope, 3).await,
        ];
        let updates = plan(&keks, scope, &rows).await.expect("plan");
        assert_eq!(
            updates.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
            [0, 2]
        );
        for (index, wrapped) in &updates {
            let rewrapped = StoredKey {
                wrapped_dek: wrapped.clone(),
                kek_ref: keks.target.key_ref().to_owned(),
                ..rows[*index].clone()
            };
            require_generations(&keks.target, scope, &[rewrapped])
                .await
                .expect("opens");
        }
    }

    #[tokio::test]
    async fn a_row_claiming_the_replacement_reference_must_open_under_it() {
        let keks = Keks {
            previous: kms("local:old"),
            target: kms("local:new"),
        };
        let mut forged = stored(&keks.previous, KeyScope::Deployment, 1).await;
        forged.kek_ref = "local:new".to_owned();
        assert!(plan(&keks, KeyScope::Deployment, &[forged]).await.is_err());
    }

    #[tokio::test]
    async fn custody_verification_refuses_a_generation_on_another_reference() {
        let held = kms("local:held");
        let other = kms("local:other");
        let scope = KeyScope::Deployment;
        let rows = [
            stored(&held, scope, 1).await,
            stored(&other, scope, 2).await,
        ];
        let error = require_generations(&held, scope, &rows)
            .await
            .expect_err("refused");
        assert!(error.contains("does not hold"), "{error}");
        assert!(require_generations(&held, scope, &rows[..1]).await.is_ok());
    }
}
