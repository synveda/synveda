//! The operator's half of the key plane (TEN-4, ADR-0064).
//!
//! The operator-facing key and secret custody boundary:
//!
//!   * mint a KEK (`synveda kms keygen`), because the alternative to a
//!     generated key is somebody typing one;
//!   * provision and rotate a tenant's data key;
//!   * store stable scope-bound directory, Tool and provider references;
//!   * re-encrypt active secret envelopes after tenant-key rotation;
//!   * export a tenant, sealed, and open that export again — which is where
//!     "tenant export is unreadable without that tenant's key" stops being a
//!     sentence and becomes a command that fails without the key.
//!
//! These run against the database rather than the gateway, deliberately.
//! `synveda tenant export` is an operator act on a deployment's own data at a
//! moment when the gateway may be exactly what is unavailable, and the
//! break-glass precedent (`db migrate`, `tenant create`, `audit verify`) is
//! the one this follows.

use std::io::{Read as _, Write as _};

use synveda_crypto::{
    DataKey, KeyManagement, KeyScope, KeyVersion, Kms, LocalKms, Purpose, RowKey, SealingKey,
};
use synveda_store::keys::KeyRing;
use synveda_types::secret::{TenantSecretKind, TenantSecretState, tenant_secret_reference};
use synveda_types::{ScopeId, TenantId, TenantSecretId, TenantSecretReencryptionJobId};
use zeroize::Zeroizing;

/// The archive's first bytes. Versioned from the first byte because an
/// export is the one artefact here that outlives the build that wrote it.
const ARCHIVE_MAGIC: &[u8; 8] = b"SVCTXEX2";

/// Builds the KMS from the environment, the gateway's rules exactly.
///
/// # Errors
/// A key that is present and malformed. Absent is [`Kms::Disabled`], and the
/// commands below then fail with a message naming the variable — which is
/// better than this function guessing that an operator running
/// `tenant key rotate` did not mean it.
pub fn kms_from_env() -> Result<Kms, String> {
    let Some(key) =
        crate::init::sensitive_setting("SYNVEDA_KMS_KEY")?.filter(|value| !value.trim().is_empty())
    else {
        return Ok(Kms::Disabled);
    };
    let key_ref = crate::init::sensitive_setting("SYNVEDA_KMS_KEY_REF")?
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "local:default".to_string());
    LocalKms::from_hex(&key, key_ref)
        .map(Kms::Local)
        .map_err(|err| format!("SYNVEDA_KMS_KEY is not usable: {err}"))
}

fn ring() -> Result<KeyRing, String> {
    Ok(KeyRing::new(kms_from_env()?))
}

/// Prints a fresh KEK as hex, and nothing else.
///
/// Nothing else on purpose: this is meant to be captured
/// (`SYNVEDA_KMS_KEY=$(synveda kms keygen)`), and a helpful banner on stdout
/// is a banner in somebody's environment variable. The guidance goes to
/// stderr, where a pipe does not collect it.
///
/// # Errors
/// If the system CSPRNG is unavailable.
pub fn keygen() -> Result<(), String> {
    let key = DataKey::generate().map_err(|err| err.to_string())?;
    let hex = key.to_hex();
    println!("{}", &*hex);
    eprintln!(
        "This is a key-encryption key. Store it where the deployment reads \
         SYNVEDA_KMS_KEY from, and back it up: every tenant key in this \
         database is wrapped by it, and losing it is losing them."
    );
    Ok(())
}

/// Mints a tenant's first data key and returns the generation, printing
/// nothing.
///
/// What `tenant create` calls, so admission and the key are one command
/// without the key's JSON landing in the middle of the tenant's.
///
/// # Errors
/// If no KEK is configured, or the tenant does not exist.
pub async fn provision_quietly(pool: &sqlx::PgPool, tenant: TenantId) -> Result<u32, String> {
    let ring = ring()?;
    provision_quietly_with_ring(pool, tenant, &ring).await
}

pub(crate) async fn provision_quietly_with_ring(
    pool: &sqlx::PgPool,
    tenant: TenantId,
    ring: &KeyRing,
) -> Result<u32, String> {
    let version = ring
        .provision(pool, KeyScope::Tenant(tenant))
        .await
        .map_err(|err| err.to_string())?;
    // An existing wrapped row is not custody. Prove that this process can
    // unwrap the current generation before converging success evidence; this
    // catches disabled, wrong and externally denied KMS authority on reruns.
    ring.sealing_key(pool, KeyScope::Tenant(tenant))
        .await
        .map_err(|err| err.to_string())?;
    converge_provision_audit(pool, tenant).await?;
    Ok(version.get())
}

async fn converge_provision_audit(pool: &sqlx::PgPool, tenant: TenantId) -> Result<(), String> {
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .map_err(|err| err.to_string())?;
    let first = synveda_store::keys::tenant_at(&mut *tx, tenant, KeyVersion::FIRST)
        .await
        .map_err(|err| err.to_string())?
        .ok_or_else(|| "tenant key provisioning committed without generation 1".to_owned())?;
    let witness = synveda_audit::TenantKeyProvisionedWitness {
        occurred_at: chrono::Utc::now(),
        break_glass_subject: crate::break_glass().subject,
        kek_ref: first.kek_ref,
        trace_id: None,
    };
    synveda_audit::append_tenant_key_provisioned_once(&mut tx, tenant, &witness)
        .await
        .map_err(|err| err.to_string())?;
    tx.commit().await.map_err(|err| err.to_string())
}

/// Mints a tenant's first data key, or reports the one already there.
///
/// # Errors
/// If no KEK is configured, or the tenant does not exist.
pub async fn provision(pool: &sqlx::PgPool, tenant: TenantId) -> Result<(), String> {
    let version = provision_quietly(pool, tenant).await?;
    println!(
        "{}",
        serde_json::json!({
            "tenant": tenant.to_string(),
            "version": version,
            "kek_ref": ring()?.kms().key_ref(),
        })
    );
    Ok(())
}

/// Retires a tenant's current data key and mints the next generation.
///
/// Active database-owned tenant-secret envelopes are advanced by a durable
/// re-encryption job. Retired generations remain available for external
/// archives and other payloads the job does not own (ADR-0094 decision 6).
///
/// # Errors
/// If no KEK is configured, or the tenant has no key to rotate.
pub async fn rotate(pool: &sqlx::PgPool, tenant: TenantId) -> Result<(), String> {
    let ring = ring()?;
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .map_err(|err| err.to_string())?;
    let previous = synveda_store::keys::tenant_current(&mut *tx, tenant)
        .await
        .map_err(|err| err.to_string())?
        .ok_or_else(|| format!("current key for tenant {tenant} was not found"))?
        .version;
    tx.commit().await.map_err(|err| err.to_string())?;
    let version = ring
        .rotate(pool, KeyScope::Tenant(tenant))
        .await
        .map_err(|err| err.to_string())?;
    let job_id = TenantSecretReencryptionJobId::new();
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .map_err(|err| err.to_string())?;
    let job = synveda_store::tenant_secrets::create_reencryption_job(
        &mut tx, job_id, tenant, previous, version,
    )
    .await
    .map_err(|err| err.to_string())?;
    crate::record_break_glass(
        &mut tx,
        tenant,
        synveda_audit::AuditAction::TenantKeyRotated,
        format!("tenant {tenant} key"),
        serde_json::json!({
            "from_version": previous.get(),
            "version": version.get(),
            "kek_ref": ring.kms().key_ref(),
            "reencryption_job_id": job.id,
        }),
    )
    .await?;
    tx.commit().await.map_err(|err| err.to_string())?;
    let job = reencrypt_tenant_secrets(pool, &ring, tenant, job).await?;
    println!(
        "{}",
        serde_json::json!({
            "tenant": tenant.to_string(),
            "version": version.get(),
            "kek_ref": ring.kms().key_ref(),
            "reencryption_job": {
                "id": job.id,
                "state": job.state,
                "secrets_total": job.secrets_total,
                "secrets_reencrypted": job.secrets_reencrypted,
            },
            "note": "active tenant secrets were re-encrypted; retired generations remain for old external archives",
        })
    );
    Ok(())
}

async fn reencrypt_tenant_secrets(
    pool: &sqlx::PgPool,
    ring: &KeyRing,
    tenant: TenantId,
    job: synveda_store::tenant_secrets::SecretReencryptionJob,
) -> Result<synveda_store::tenant_secrets::SecretReencryptionJob, String> {
    if job.state == "completed" {
        return Ok(job);
    }
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .map_err(|err| err.to_string())?;
    let secrets = synveda_store::tenant_secrets::active_for_key_generation(
        &mut *tx,
        tenant,
        job.from_key_version,
    )
    .await
    .map_err(|err| err.to_string())?;
    let running = synveda_store::tenant_secrets::start_reencryption_job(
        &mut tx,
        tenant,
        job.id,
        secrets.len() as u64,
    )
    .await
    .map_err(|err| err.to_string())?
    .ok_or_else(|| {
        format!(
            "tenant-secret re-encryption job {} is already complete",
            job.id
        )
    })?;
    tx.commit().await.map_err(|err| err.to_string())?;

    let mut progress = 0_u64;
    let work: Result<(), (&'static str, String)> = async {
        let target = ring
            .sealing_key(pool, KeyScope::Tenant(tenant))
            .await
            .map_err(|err| ("target_key", err.to_string()))?;
        if target.version() != running.to_key_version {
            return Err((
                "stale_target_key",
                "the current tenant key changed while re-encryption started".to_owned(),
            ));
        }
        for secret in secrets {
            let sealed = secret.sealed.as_deref().ok_or_else(|| {
                (
                    "missing_envelope",
                    "an active secret has no envelope".to_owned(),
                )
            })?;
            let opened = ring
                .opening_key(pool, KeyScope::Tenant(tenant), sealed)
                .await
                .map_err(|err| ("source_key", err.to_string()))?
                .open(
                    Purpose::TenantSecret,
                    RowKey::Uuid(secret.id.as_uuid()),
                    sealed,
                )
                .map_err(|err| ("envelope_open", err.to_string()))?;
            let resealed = target
                .seal(
                    Purpose::TenantSecret,
                    RowKey::Uuid(secret.id.as_uuid()),
                    &opened,
                )
                .map_err(|err| ("envelope_seal", err.to_string()))?;
            let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
                .await
                .map_err(|err| ("storage", err.to_string()))?;
            let replaced = synveda_store::tenant_secrets::replace_envelope(
                &mut tx,
                tenant,
                secret.id,
                secret.value_revision,
                running.from_key_version,
                running.to_key_version,
                &resealed,
            )
            .await
            .map_err(|err| ("storage", err.to_string()))?;
            if !replaced {
                return Err((
                    "stale_secret",
                    format!("tenant-secret {} changed during re-encryption", secret.id),
                ));
            }
            tx.commit()
                .await
                .map_err(|err| ("storage", err.to_string()))?;
            progress += 1;
        }
        Ok(())
    }
    .await;

    match work {
        Ok(()) => {
            let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let completed_job = synveda_store::tenant_secrets::complete_reencryption_job(
                &mut tx, tenant, running.id, progress,
            )
            .await
            .map_err(|err| err.to_string())?
            .ok_or_else(|| {
                format!(
                    "tenant-secret re-encryption job {} changed before completion",
                    running.id
                )
            })?;
            crate::record_break_glass(
                &mut tx,
                tenant,
                synveda_audit::AuditAction::TenantSecretsReencrypted,
                format!("tenant {tenant} secret re-encryption {}", running.id),
                serde_json::json!({
                    "job_id": running.id,
                    "from_key_version": running.from_key_version.get(),
                    "to_key_version": running.to_key_version.get(),
                    "secrets_reencrypted": progress,
                    "attempt": completed_job.attempt,
                }),
            )
            .await?;
            tx.commit().await.map_err(|err| err.to_string())?;
            Ok(completed_job)
        }
        Err((code, detail)) => {
            let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            synveda_store::tenant_secrets::fail_reencryption_job(
                &mut tx, tenant, running.id, progress, code,
            )
            .await
            .map_err(|err| err.to_string())?;
            tx.commit().await.map_err(|err| err.to_string())?;
            Err(format!(
                "tenant key rotated, but secret re-encryption job {} failed ({code}): {detail}",
                running.id
            ))
        }
    }
}

fn secret_metadata(
    secret: &synveda_store::tenant_secrets::StoredTenantSecret,
) -> serde_json::Value {
    serde_json::json!({
        "id": secret.id,
        "reference": tenant_secret_reference(secret.id),
        "scope_id": secret.scope_id,
        "kind": secret.kind.as_str(),
        "label": secret.label,
        "provider": secret.provider,
        "state": secret.state.as_str(),
        "value_revision": secret.value_revision,
        "key_version": secret.key_version.map(|version| version.get()),
        "created_at": secret.created_at,
        "rotated_at": secret.rotated_at,
        "updated_at": secret.updated_at,
        "revoked_at": secret.revoked_at,
    })
}

/// What keys a tenant has, and which is current.
///
/// # Errors
/// If the tenant's rows cannot be read.
pub async fn status(pool: &sqlx::PgPool, tenant: TenantId) -> Result<(), String> {
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .map_err(|err| err.to_string())?;
    let current = synveda_store::keys::tenant_current(&mut *tx, tenant)
        .await
        .map_err(|err| err.to_string())?;
    let secrets = synveda_store::tenant_secrets::list(&mut *tx, tenant)
        .await
        .map_err(|err| err.to_string())?;
    let jobs = synveda_store::tenant_secrets::reencryption_jobs(&mut *tx, tenant)
        .await
        .map_err(|err| err.to_string())?;
    tx.commit().await.map_err(|err| err.to_string())?;

    println!(
        "{}",
        serde_json::json!({
            "tenant": tenant.to_string(),
            "current_version": current.as_ref().map(|key| key.version.get()),
            "kek_ref": current.as_ref().map(|key| key.kek_ref.clone()),
            "secrets": secrets.iter().map(secret_metadata).collect::<Vec<_>>(),
            "reencryption_jobs": jobs.iter().map(|job| serde_json::json!({
                "id": job.id,
                "from_key_version": job.from_key_version.get(),
                "to_key_version": job.to_key_version.get(),
                "state": job.state,
                "secrets_total": job.secrets_total,
                "secrets_reencrypted": job.secrets_reencrypted,
                "attempt": job.attempt,
                "failure_code": job.failure_code,
                "created_at": job.created_at,
                "started_at": job.started_at,
                "completed_at": job.completed_at,
            })).collect::<Vec<_>>(),
        })
    );
    Ok(())
}

// ── The export ──────────────────────────────────────────────────────────────

/// Writes a sealed archive of a tenant's Knowledge history and audit chain.
///
/// **Sealed under a fresh per-archive key, itself sealed under the tenant's**
/// (ADR-0064 decision 8). Two reasons, and the second is the one that matters:
/// the archive is one ciphertext however large it is, and handing somebody an
/// export does not hand them the key that opens the tenant's live secrets.
///
/// # Errors
/// If no KEK is configured, the tenant has no key, or the archive cannot be
/// written.
pub async fn export(
    pool: &sqlx::PgPool,
    tenant: TenantId,
    out: &std::path::Path,
) -> Result<(), String> {
    let ring = ring()?;
    let scope = KeyScope::Tenant(tenant);
    let tenant_key = ring
        .sealing_key(pool, scope)
        .await
        .map_err(|err| err.to_string())?;

    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .map_err(|err| err.to_string())?;
    let knowledge = synveda_store::knowledge::export_tenant(&mut tx, tenant)
        .await
        .map_err(|err| err.to_string())?;
    // `tail` newest-first over the whole chain, reversed: `since` needs an
    // explicit action filter, and an export that silently omitted an action
    // nobody had added to a list would be an export that looks complete. It
    // shares the Knowledge transaction so the archive is one database
    // snapshot rather than two individually valid views separated by a race.
    let mut events = synveda_audit::tail(&mut tx, tenant, i64::MAX)
        .await
        .map_err(|err| err.to_string())?;
    events.reverse();
    tx.commit().await.map_err(|err| err.to_string())?;

    // Mapped field by field rather than derived from the internal structs.
    // An archive format outlives the build that wrote it, so what goes in it
    // is a decision each time rather than whatever a struct happens to hold
    // after the next refactor.
    let body = serde_json::json!({
        "format": "synveda-context-export-2",
        "tenant": tenant.to_string(),
        "knowledge": {
            "head_history": knowledge.head_history,
            "revisions": knowledge.revisions,
            "sources": knowledge.sources,
            "revision_sources": knowledge.revision_sources,
            "relations": knowledge.relations,
        },
        "audit": events
            .iter()
            .map(|event| serde_json::json!({
                "seq": event.seq,
                "occurred_at": event.occurred_at,
                "actor_kind": event.actor_kind,
                "actor_subject": event.actor_subject,
                "action": event.action,
                "resource": event.resource,
                "outcome": event.outcome,
                "payload": event.payload,
                "prev_hash": hex(&event.prev_hash),
                "hash": hex(&event.hash),
            }))
            .collect::<Vec<_>>(),
    });
    let body = serde_json::to_vec(&body).map_err(|err| err.to_string())?;

    // The per-archive key. `KeyVersion::FIRST` because this key has no
    // generations — its identity is the archive, and the tenant key that
    // wraps it is the one that rotates.
    let archive_key = DataKey::generate().map_err(|err| err.to_string())?;
    let wrapped = tenant_key
        .seal_data_key(
            Purpose::ExportKey,
            RowKey::Uuid(tenant.as_uuid()),
            &archive_key,
        )
        .map_err(|err| err.to_string())?;
    let sealed = SealingKey::new(scope, KeyVersion::FIRST, archive_key)
        .seal(Purpose::TenantExport, RowKey::Uuid(tenant.as_uuid()), &body)
        .map_err(|err| err.to_string())?;

    let header = serde_json::to_vec(&serde_json::json!({
        "format": "synveda-context-export-2",
        "tenant": tenant.to_string(),
        "tenant_key_version": tenant_key.version().get(),
        "knowledge_items": knowledge.item_count,
        "knowledge_revisions": knowledge.revision_count,
        "knowledge_sources": knowledge.source_count,
        "knowledge_relations": knowledge.relation_count,
        "audit_events": events.len(),
    }))
    .map_err(|err| err.to_string())?;

    let mut archive =
        Vec::with_capacity(ARCHIVE_MAGIC.len() + 8 + header.len() + wrapped.len() + sealed.len());
    archive.extend_from_slice(ARCHIVE_MAGIC);
    archive.extend_from_slice(&(header.len() as u32).to_be_bytes());
    archive.extend_from_slice(&(wrapped.len() as u32).to_be_bytes());
    archive.extend_from_slice(&header);
    archive.extend_from_slice(&wrapped);
    archive.extend_from_slice(&sealed);

    // The header is cleartext and says so by being outside the seal: an
    // operator holding an archive has to be able to tell whose it is and
    // which key generation opens it *without* the key, or a backup vault
    // full of these is a vault full of anonymous blobs.
    std::fs::write(out, &archive).map_err(|err| format!("write {}: {err}", out.display()))?;
    // Chained *after* the archive exists, so the chain never claims an
    // export that a full disk stopped. The payload carries integers and
    // strings only — an audit payload may hold no non-integer number, the
    // defect AUTH-5 found at its first breaker trip.
    chain(
        pool,
        tenant,
        synveda_audit::AuditAction::TenantExported,
        format!("tenant {tenant} export"),
        serde_json::json!({
            "knowledge_items": knowledge.item_count as i64,
            "knowledge_revisions": knowledge.revision_count as i64,
            "knowledge_sources": knowledge.source_count as i64,
            "knowledge_relations": knowledge.relation_count as i64,
            "audit_events": events.len() as i64,
            "bytes": archive.len() as i64,
            "tenant_key_version": tenant_key.version().get(),
        }),
    )
    .await?;
    println!(
        "{}",
        serde_json::json!({
            "path": out.display().to_string(),
            "bytes": archive.len(),
            "knowledge_items": knowledge.item_count,
            "knowledge_revisions": knowledge.revision_count,
            "knowledge_sources": knowledge.source_count,
            "knowledge_relations": knowledge.relation_count,
            "audit_events": events.len(),
            "tenant_key_version": tenant_key.version().get(),
        })
    );
    Ok(())
}

/// Opens a sealed archive, writing its contents to stdout.
///
/// This is the AC in a command: it needs the tenant's key, and a deployment
/// that does not have it gets an error rather than a body.
///
/// # Errors
/// If the archive is malformed, or this deployment cannot open it.
pub async fn export_open(pool: &sqlx::PgPool, archive: &std::path::Path) -> Result<(), String> {
    let bytes =
        std::fs::read(archive).map_err(|err| format!("read {}: {err}", archive.display()))?;
    let (header, wrapped, sealed) = split_archive(&bytes)?;

    let header_json: serde_json::Value =
        serde_json::from_slice(header).map_err(|err| format!("archive header: {err}"))?;
    let tenant: TenantId = header_json
        .get("tenant")
        .and_then(serde_json::Value::as_str)
        .ok_or("archive header names no tenant")?
        .parse()
        .map_err(|_| "archive header's tenant is not a uuid".to_string())?;

    let ring = ring()?;
    let scope = KeyScope::Tenant(tenant);
    // The tenant key generation comes from the wrapped key's own header, not
    // from the cleartext one — an archive whose cleartext claims a different
    // generation must not redirect the lookup.
    // Every refusal from here down reads the same way, and that is the point:
    // an operator holding an archive that will not open needs to be told they
    // cannot open it, not which internal purpose string the cipher was
    // checking. The causes — wrong KEK, wrong tenant, no key at all — are
    // deliberately not distinguished either, because telling somebody *which*
    // of their keys was wrong tells them something about an archive they
    // cannot otherwise read.
    let refused =
        |err: synveda_types::Error| format!("this deployment cannot open that export ({err})");
    let tenant_key = ring
        .opening_key(pool, scope, wrapped)
        .await
        .map_err(refused)?;
    let archive_key = tenant_key
        .open_data_key(Purpose::ExportKey, RowKey::Uuid(tenant.as_uuid()), wrapped)
        .map_err(refused)?;
    let body = SealingKey::new(scope, KeyVersion::FIRST, archive_key)
        .open(
            Purpose::TenantExport,
            RowKey::Uuid(tenant.as_uuid()),
            sealed,
        )
        .map_err(refused)?;

    std::io::stdout()
        .write_all(&body)
        .map_err(|err| err.to_string())?;
    println!();
    Ok(())
}

/// Prints an archive's cleartext header without opening it — whose it is,
/// how big, which key generation. What a backup vault's index needs.
///
/// # Errors
/// If the archive is malformed.
pub fn export_describe(archive: &std::path::Path) -> Result<(), String> {
    let bytes =
        std::fs::read(archive).map_err(|err| format!("read {}: {err}", archive.display()))?;
    let (header, _, sealed) = split_archive(&bytes)?;
    let mut header_json: serde_json::Value =
        serde_json::from_slice(header).map_err(|err| format!("archive header: {err}"))?;
    if let Some(object) = header_json.as_object_mut() {
        object.insert("sealed_bytes".to_string(), sealed.len().into());
    }
    println!("{header_json}");
    Ok(())
}

/// Chains a break-glass event of its own, in its own transaction.
///
/// The acts above that touch a key are not in the same transaction as the
/// key write, and deliberately: minting a key is a KMS call, and holding a
/// transaction open across somebody else's network is how one slow vendor
/// becomes a lock nobody can explain. The secret commands *are* transactional
/// with their write, because there is no network in the middle of those.
async fn chain(
    pool: &sqlx::PgPool,
    tenant: TenantId,
    action: synveda_audit::AuditAction,
    resource: String,
    payload: serde_json::Value,
) -> Result<(), String> {
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .map_err(|err| err.to_string())?;
    crate::record_break_glass(&mut tx, tenant, action, resource, payload).await?;
    tx.commit().await.map_err(|err| err.to_string())
}

/// Chain hashes as hex, so the archive stays readable text rather than
/// base64 an auditor has to decode to compare against `synveda audit verify`.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// An archive's three parts: the cleartext header, the wrapped per-archive
/// key, and the sealed body.
type ArchiveParts<'a> = (&'a [u8], &'a [u8], &'a [u8]);

fn split_archive(bytes: &[u8]) -> Result<ArchiveParts<'_>, String> {
    const PREFIX: usize = 8 + 4 + 4;
    if bytes.len() < PREFIX || &bytes[..8] != ARCHIVE_MAGIC {
        return Err("not a synveda export archive".to_string());
    }
    let header_len = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    let wrapped_len = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
    let header_end = PREFIX
        .checked_add(header_len)
        .ok_or("archive header length overflows")?;
    let wrapped_end = header_end
        .checked_add(wrapped_len)
        .ok_or("archive key length overflows")?;
    if bytes.len() < wrapped_end {
        return Err("archive is truncated".to_string());
    }
    Ok((
        &bytes[PREFIX..header_end],
        &bytes[header_end..wrapped_end],
        &bytes[wrapped_end..],
    ))
}

// ── Stable sealed per-tenant secrets ───────────────────────────────────────

/// Read and custody one arbitrary tenant-secret value from a file or stdin.
/// The value never appears in argv, stdout, audit or a store DTO.
pub async fn put_tenant_secret_from_path(
    pool: &sqlx::PgPool,
    tenant: TenantId,
    scope: ScopeId,
    kind: TenantSecretKind,
    label: &str,
    provider: Option<&str>,
    from: &std::path::Path,
) -> Result<(), String> {
    let bytes = if from == std::path::Path::new("-") {
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(65_537)
            .read_to_end(&mut bytes)
            .map_err(|err| format!("read secret from stdin: {err}"))?;
        bytes
    } else {
        let metadata =
            std::fs::metadata(from).map_err(|err| format!("inspect {}: {err}", from.display()))?;
        if metadata.len() > 65_536 {
            return Err("tenant-secret value exceeds 65536 bytes".to_owned());
        }
        std::fs::read(from).map_err(|err| format!("read {}: {err}", from.display()))?
    };
    let bytes = Zeroizing::new(bytes);
    let secret = store_secret_value(pool, tenant, scope, kind, label, provider, &bytes).await?;
    println!("{}", secret_metadata(&secret));
    Ok(())
}

async fn store_secret_value(
    pool: &sqlx::PgPool,
    tenant: TenantId,
    scope: ScopeId,
    kind: TenantSecretKind,
    label: &str,
    provider: Option<&str>,
    plaintext: &[u8],
) -> Result<synveda_store::tenant_secrets::StoredTenantSecret, String> {
    if plaintext.is_empty() || plaintext.len() > 65_536 {
        return Err("tenant-secret value must contain 1..=65536 bytes".to_owned());
    }
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .map_err(|err| err.to_string())?;
    let existing = synveda_store::tenant_secrets::by_label(&mut *tx, tenant, kind, label)
        .await
        .map_err(|err| err.to_string())?;
    tx.commit().await.map_err(|err| err.to_string())?;
    let id = existing
        .as_ref()
        .map_or_else(TenantSecretId::new, |secret| secret.id);
    let ring = ring()?;
    let key = ring
        .sealing_key(pool, KeyScope::Tenant(tenant))
        .await
        .map_err(|err| err.to_string())?;
    let sealed = key
        .seal(Purpose::TenantSecret, RowKey::Uuid(id.as_uuid()), plaintext)
        .map_err(|err| err.to_string())?;

    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .map_err(|err| err.to_string())?;
    let stored = synveda_store::tenant_secrets::put(
        &mut tx,
        id,
        tenant,
        scope,
        kind,
        label,
        provider,
        key.version(),
        &sealed,
    )
    .await
    .map_err(|err| err.to_string())?;
    crate::record_break_glass(
        &mut tx,
        tenant,
        synveda_audit::AuditAction::TenantSecretStored,
        format!("tenant-secret:{}", stored.id),
        serde_json::json!({
            "secret_id": stored.id,
            "reference": tenant_secret_reference(stored.id),
            "scope_id": stored.scope_id,
            "kind": stored.kind.as_str(),
            "label": stored.label,
            "provider": stored.provider,
            "value_revision": stored.value_revision,
            "key_version": stored.key_version.map(|version| version.get()),
        }),
    )
    .await?;
    tx.commit().await.map_err(|err| err.to_string())?;
    Ok(stored)
}

/// Revoke one stable tenant-secret reference and destroy its envelope.
pub async fn revoke_tenant_secret(
    pool: &sqlx::PgPool,
    tenant: TenantId,
    id: TenantSecretId,
) -> Result<(), String> {
    let revoked = revoke_secret_value(pool, tenant, id).await?;
    println!(
        "{}",
        serde_json::json!({
            "tenant": tenant,
            "secret_id": id,
            "reference": tenant_secret_reference(id),
            "revoked": revoked.is_some(),
            "value_revision": revoked.as_ref().map(|secret| secret.value_revision),
        })
    );
    Ok(())
}

async fn revoke_secret_value(
    pool: &sqlx::PgPool,
    tenant: TenantId,
    id: TenantSecretId,
) -> Result<Option<synveda_store::tenant_secrets::StoredTenantSecret>, String> {
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .map_err(|err| err.to_string())?;
    let revoked = synveda_store::tenant_secrets::revoke(&mut tx, tenant, id)
        .await
        .map_err(|err| err.to_string())?;
    if let Some(secret) = &revoked {
        crate::record_break_glass(
            &mut tx,
            tenant,
            synveda_audit::AuditAction::TenantSecretCleared,
            format!("tenant-secret:{}", secret.id),
            serde_json::json!({
                "secret_id": secret.id,
                "reference": tenant_secret_reference(secret.id),
                "scope_id": secret.scope_id,
                "kind": secret.kind.as_str(),
                "label": secret.label,
                "provider": secret.provider,
                "value_revision": secret.value_revision,
            }),
        )
        .await?;
    }
    tx.commit().await.map_err(|err| err.to_string())?;
    Ok(revoked)
}

/// Seals a tenant's directory configuration into its stable root-scope
/// tenant-secret aggregate (ADR-0094 decision 5).
///
/// The whole configuration is sealed, not only its secret, so a tenant's
/// credential and the host it is presented to cannot disagree. It is
/// validated by parsing before it is sealed — a credential that is stored and
/// then found to be unparseable at 3am on a sweep is a credential that has
/// already cost somebody a night.
///
/// # Errors
/// If no KEK is configured, the tenant has no key, or the configuration does
/// not parse.
pub async fn set_directory_credential(
    pool: &sqlx::PgPool,
    tenant: TenantId,
    config_json: &str,
) -> Result<(), String> {
    // The parse error is rendered without the input: the input is a
    // credential, and serde quotes what it could not read.
    let parsed: Result<synveda_identity::directory::DirectorySyncConfig, _> =
        serde_json::from_str(config_json);
    let config = parsed.map_err(|err| {
        format!(
            "that is not a directory configuration this build understands \
             ({}): expected {{\"connector\": \"entra\"|\"okta\", ...}}",
            classify_json_error(&err)
        )
    })?;

    let name = synveda_identity::directory::CREDENTIAL_SECRET_NAME;
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .map_err(|err| err.to_string())?;
    let root = synveda_store::scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .map_err(|err| err.to_string())?;
    tx.commit().await.map_err(|err| err.to_string())?;
    let stored = store_secret_value(
        pool,
        tenant,
        root.id,
        TenantSecretKind::Directory,
        name,
        Some(connector_name(&config)),
        config_json.as_bytes(),
    )
    .await?;

    println!(
        "{}",
        serde_json::json!({
            "tenant": tenant.to_string(),
            "secret_id": stored.id,
            "reference": tenant_secret_reference(stored.id),
            "secret": stored.label,
            "connector": connector_name(&config),
            "value_revision": stored.value_revision,
            "key_version": stored.key_version.map(|version| version.get()),
        })
    );
    Ok(())
}

/// Revokes a tenant's stored directory credential. Its stable row prevents a
/// stale reference from falling back to deployment configuration.
///
/// # Errors
/// If the row cannot be read or revoked.
pub async fn clear_directory_credential(
    pool: &sqlx::PgPool,
    tenant: TenantId,
) -> Result<(), String> {
    let name = synveda_identity::directory::CREDENTIAL_SECRET_NAME;
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .map_err(|err| err.to_string())?;
    let stored = synveda_store::tenant_secrets::by_label(
        &mut *tx,
        tenant,
        TenantSecretKind::Directory,
        name,
    )
    .await
    .map_err(|err| err.to_string())?;
    tx.commit().await.map_err(|err| err.to_string())?;
    let revoked = match stored {
        Some(secret) if secret.state == TenantSecretState::Active => {
            revoke_secret_value(pool, tenant, secret.id).await?
        }
        _ => None,
    };
    println!(
        "{}",
        serde_json::json!({
            "tenant": tenant.to_string(),
            "secret": name,
            "secret_id": revoked.as_ref().map(|secret| secret.id),
            "revoked": revoked.is_some(),
        })
    );
    Ok(())
}

fn connector_name(config: &synveda_identity::directory::DirectorySyncConfig) -> &'static str {
    match config {
        synveda_identity::directory::DirectorySyncConfig::Entra { .. } => "entra",
        synveda_identity::directory::DirectorySyncConfig::Okta { .. } => "okta",
    }
}

/// serde's error text quotes the input it failed on, and here the input is a
/// credential. This keeps the shape of the complaint (which field, which
/// line) and drops the value.
fn classify_json_error(error: &serde_json::Error) -> String {
    format!(
        "{:?} at line {} column {}",
        error.classify(),
        error.line(),
        error.column()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_record_era_archive_magic_is_not_a_compatibility_reader() {
        let mut old = Vec::from(&b"SVTENEX1"[..]);
        old.extend_from_slice(&0_u32.to_be_bytes());
        old.extend_from_slice(&0_u32.to_be_bytes());
        assert_eq!(
            split_archive(&old).expect_err("old archive must be refused"),
            "not a synveda export archive"
        );
    }
}
