//! The operator's half of the key plane (TEN-4, ADR-0064).
//!
//! Four things an operator does with keys, and one of them is the AC:
//!
//!   * mint a KEK (`synveda kms keygen`), because the alternative to a
//!     generated key is somebody typing one;
//!   * provision and rotate a tenant's data key;
//!   * store a tenant's directory credential sealed under it (decision 9);
//!   * export a tenant, sealed, and open that export again — which is where
//!     "tenant export is unreadable without that tenant's key" stops being a
//!     sentence and becomes a command that fails without the key.
//!
//! These run against the database rather than the gateway, deliberately.
//! `synveda tenant export` is an operator act on a deployment's own data at a
//! moment when the gateway may be exactly what is unavailable, and the
//! break-glass precedent (`db migrate`, `tenant create`, `audit verify`) is
//! the one this follows.

use std::io::Write as _;

use synveda_crypto::{
    DataKey, KeyManagement, KeyScope, KeyVersion, Kms, LocalKms, Purpose, RowKey, SealingKey,
};
use synveda_store::keys::KeyRing;
use synveda_types::TenantId;

/// The archive's first bytes. Versioned from the first byte because an
/// export is the one artefact here that outlives the build that wrote it.
const ARCHIVE_MAGIC: &[u8; 8] = b"SVEXPRT1";

/// Builds the KMS from the environment, the gateway's rules exactly.
///
/// # Errors
/// A key that is present and malformed. Absent is [`Kms::Disabled`], and the
/// commands below then fail with a message naming the variable — which is
/// better than this function guessing that an operator running
/// `tenant key rotate` did not mean it.
pub fn kms_from_env() -> Result<Kms, String> {
    let Some(key) = std::env::var("SYNVEDA_KMS_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(Kms::Disabled);
    };
    let key_ref = std::env::var("SYNVEDA_KMS_KEY_REF")
        .ok()
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
    let version = ring
        .provision(pool, KeyScope::Tenant(tenant))
        .await
        .map_err(|err| err.to_string())?;
    chain(
        pool,
        tenant,
        synveda_audit::AuditAction::TenantKeyProvisioned,
        format!("tenant {tenant} key"),
        serde_json::json!({ "version": version.get(), "kek_ref": ring.kms().key_ref() }),
    )
    .await?;
    Ok(version.get())
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
/// Nothing is re-sealed. Payloads under the retired key keep opening under
/// it, and move forward when their rows are next written (ADR-0064
/// decision 6) — so this command is fast, and finishing does not mean every
/// ciphertext is on the new key.
///
/// # Errors
/// If no KEK is configured, or the tenant has no key to rotate.
pub async fn rotate(pool: &sqlx::PgPool, tenant: TenantId) -> Result<(), String> {
    let ring = ring()?;
    let version = ring
        .rotate(pool, KeyScope::Tenant(tenant))
        .await
        .map_err(|err| err.to_string())?;
    chain(
        pool,
        tenant,
        synveda_audit::AuditAction::TenantKeyRotated,
        format!("tenant {tenant} key"),
        serde_json::json!({ "version": version.get(), "kek_ref": ring.kms().key_ref() }),
    )
    .await?;
    println!(
        "{}",
        serde_json::json!({
            "tenant": tenant.to_string(),
            "version": version.get(),
            "kek_ref": ring.kms().key_ref(),
            "note": "existing payloads still open under the retired generation; \
                     they move forward when their rows are next written",
        })
    );
    Ok(())
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
    let names = synveda_store::tenant_secrets::names(&mut *tx, tenant)
        .await
        .map_err(|err| err.to_string())?;
    tx.commit().await.map_err(|err| err.to_string())?;

    println!(
        "{}",
        serde_json::json!({
            "tenant": tenant.to_string(),
            "current_version": current.as_ref().map(|key| key.version.get()),
            "kek_ref": current.as_ref().map(|key| key.kek_ref.clone()),
            "sealed_secrets": names,
        })
    );
    Ok(())
}

// ── The export ──────────────────────────────────────────────────────────────

/// Writes a sealed archive of a tenant's records and audit chain.
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
    let records = synveda_store::records::export_current(&mut *tx, tenant)
        .await
        .map_err(|err| err.to_string())?;
    tx.commit().await.map_err(|err| err.to_string())?;

    let mut conn = pool.acquire().await.map_err(|err| err.to_string())?;
    // `tail` newest-first over the whole chain, reversed: `since` needs an
    // explicit action filter, and an export that silently omitted an action
    // nobody had added to a list would be an export that looks complete.
    let mut events = synveda_audit::tail(&mut conn, tenant, i64::MAX)
        .await
        .map_err(|err| err.to_string())?;
    drop(conn);
    events.reverse();

    // Mapped field by field rather than derived from the internal structs.
    // An archive format outlives the build that wrote it, so what goes in it
    // is a decision each time rather than whatever a struct happens to hold
    // after the next refactor.
    let body = serde_json::json!({
        "format": "synveda-export-1",
        "tenant": tenant.to_string(),
        "records": records
            .iter()
            .map(|record| serde_json::json!({
                "id": record.id.to_string(),
                "scope_id": record.state.scope_id.to_string(),
                "owner_id": record.state.owner_id.to_string(),
                "kind": record.state.kind.as_str(),
                "class": record.state.class.as_str(),
                "content": record.state.content,
                "sensitivity": record.state.sensitivity.as_str(),
                "provenance": record.state.provenance,
                "valid_from": record.state.valid_from,
                "valid_to": record.state.valid_to,
                "tx_from": record.tx_from,
                "tx_to": record.tx_to,
            }))
            .collect::<Vec<_>>(),
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
        "format": "synveda-export-1",
        "tenant": tenant.to_string(),
        "tenant_key_version": tenant_key.version().get(),
        "records": records.len(),
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
            "records": records.len() as i64,
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
            "records": records.len(),
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

// ── Sealed per-tenant secrets ───────────────────────────────────────────────

/// Seals a tenant's directory configuration into `tenant_secrets`
/// (ADR-0064 decision 9).
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
            err.classify_for_operator()
        )
    })?;

    let ring = ring()?;
    let name = synveda_identity::directory::CREDENTIAL_SECRET_NAME;
    let sealed = ring
        .sealing_key(pool, KeyScope::Tenant(tenant))
        .await
        .map_err(|err| err.to_string())?
        .seal(
            Purpose::DirectoryCredential,
            RowKey::Name(name),
            config_json.as_bytes(),
        )
        .map_err(|err| err.to_string())?;

    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .map_err(|err| err.to_string())?;
    synveda_store::tenant_secrets::put(&mut *tx, tenant, name, &sealed)
        .await
        .map_err(|err| err.to_string())?;
    // In the same transaction as the write it describes, the house rule for
    // a break-glass act: a stored credential and the record of storing it
    // commit together or not at all.
    crate::record_break_glass(
        &mut tx,
        tenant,
        synveda_audit::AuditAction::TenantSecretStored,
        format!("tenant {tenant} secret {name}"),
        serde_json::json!({ "name": name, "connector": connector_name(&config) }),
    )
    .await?;
    tx.commit().await.map_err(|err| err.to_string())?;

    println!(
        "{}",
        serde_json::json!({
            "tenant": tenant.to_string(),
            "secret": name,
            "connector": connector_name(&config),
            "sealed_bytes": sealed.len(),
        })
    );
    Ok(())
}

/// Destroys a tenant's stored directory credential. The sweep then falls back
/// to the deployment's configuration, if it has one for this tenant.
///
/// # Errors
/// If the row cannot be deleted.
pub async fn clear_directory_credential(
    pool: &sqlx::PgPool,
    tenant: TenantId,
) -> Result<(), String> {
    let name = synveda_identity::directory::CREDENTIAL_SECRET_NAME;
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .map_err(|err| err.to_string())?;
    let removed = synveda_store::tenant_secrets::delete(&mut *tx, tenant, name)
        .await
        .map_err(|err| err.to_string())?;
    if removed {
        crate::record_break_glass(
            &mut tx,
            tenant,
            synveda_audit::AuditAction::TenantSecretCleared,
            format!("tenant {tenant} secret {name}"),
            serde_json::json!({ "name": name }),
        )
        .await?;
    }
    tx.commit().await.map_err(|err| err.to_string())?;
    println!(
        "{}",
        serde_json::json!({ "tenant": tenant.to_string(), "secret": name, "removed": removed })
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
trait ClassifyForOperator {
    fn classify_for_operator(&self) -> String;
}

impl ClassifyForOperator for serde_json::Error {
    fn classify_for_operator(&self) -> String {
        format!(
            "{:?} at line {} column {}",
            self.classify(),
            self.line(),
            self.column()
        )
    }
}
