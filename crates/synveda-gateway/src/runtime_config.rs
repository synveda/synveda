//! Process-neutral runtime configuration shared by the gateway and core worker.
//!
//! These functions interpret application settings only. Compose, direct
//! binaries and later Kubernetes supply values differently, but they must not
//! assign them different meanings (CPR-45, ADR-0102).

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Read as _;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use synveda_identity::directory::DirectoryConnector;
use synveda_types::TenantId;
use tokio::sync::watch;

const MAX_SETTING_FILE_BYTES: u64 = 1_048_576;

/// Reads `NAME` or `NAME_FILE`, rejecting ambiguity and oversized/non-text
/// files without ever including the setting value in an error.
pub fn setting(name: &str) -> Result<Option<String>, String> {
    let file_name = format!("{name}_FILE");
    resolve_setting(name, std::env::var_os(name), std::env::var_os(&file_name))
}

/// Reads a required direct/file setting.
pub fn required_setting(name: &str) -> Result<String, String> {
    setting(name)?.ok_or_else(|| format!("{name} or {name}_FILE must be set"))
}

/// Reads the provider-neutral database role contract shared by gateway,
/// worker, migrator and deployment preflight.
pub fn database_roles() -> Result<synveda_store::runtime_role::DatabaseRoles, String> {
    let value = required_setting("SYNVEDA_DATABASE_ROLES")?;
    if value.len() > 4_096 {
        return Err("SYNVEDA_DATABASE_ROLES exceeds the 4096 byte startup bound".to_owned());
    }
    let roles = synveda_store::runtime_role::DatabaseRoles::parse_json(&value)
        .map_err(|error| error.to_string())?;
    let required_peer = setting("SYNVEDA_DATABASE_REQUIRED_PEER")?;
    validate_required_database_peer(&roles, required_peer.as_deref())?;
    Ok(roles)
}

fn validate_required_database_peer(
    roles: &synveda_store::runtime_role::DatabaseRoles,
    required_peer: Option<&str>,
) -> Result<(), String> {
    let Some(required_peer) = required_peer else {
        return Ok(());
    };
    let peer_is_forbidden = roles
        .forbidden_databases()
        .iter()
        .any(|database| database == required_peer);
    let peer_is_isolated = roles
        .isolated_peer_roles()
        .iter()
        .any(|role| role == required_peer);
    if !peer_is_forbidden || !peer_is_isolated {
        return Err(
            "SYNVEDA_DATABASE_REQUIRED_PEER is absent from the configured bidirectional isolation contract"
                .to_owned(),
        );
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PinnedDatabaseTarget {
    database: String,
    cluster_system_identifier: String,
    database_oid: i64,
}

impl From<synveda_store::runtime_role::DatabaseIdentity> for PinnedDatabaseTarget {
    fn from(identity: synveda_store::runtime_role::DatabaseIdentity) -> Self {
        Self {
            database: identity.database,
            cluster_system_identifier: identity.cluster_system_identifier,
            database_oid: identity.database_oid,
        }
    }
}

/// Content-free stage at which a strict pool hook conclusively refused a
/// physical database connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PoolRefusalStage {
    /// Effective session settings could not satisfy the product contract.
    Session,
    /// The selected login was not the configured runtime principal.
    Role,
    /// The connection selected a different writable database identity.
    Identity,
}

/// Absorbing refusal signal paired with one runtime pool.
///
/// SQLx retries failed connection callbacks until the acquire deadline and
/// consequently reports only a pool timeout. This side channel retains the
/// first conclusive, content-free refusal so the authority sentinel can close
/// the application plane terminally instead of treating it as an outage.
#[derive(Clone)]
pub struct PoolRefusal {
    state: watch::Receiver<Option<PoolRefusalStage>>,
    // Keep the channel live even before SQLx has cloned either callback.
    _keepalive: watch::Sender<Option<PoolRefusalStage>>,
}

impl PoolRefusal {
    pub(crate) fn current(&self) -> Option<PoolRefusalStage> {
        *self.state.borrow()
    }

    pub(crate) async fn wait_until_refused(&mut self) -> PoolRefusalStage {
        loop {
            if let Some(stage) = *self.state.borrow_and_update() {
                return stage;
            }
            if self.state.changed().await.is_err() {
                // `_keepalive` makes sender loss unreachable through the
                // public constructor. Fail closed if that invariant changes.
                return PoolRefusalStage::Identity;
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn refuse_for_test(&self, stage: PoolRefusalStage) {
        self._keepalive.send_if_modified(|current| {
            if current.is_some() {
                false
            } else {
                *current = Some(stage);
                true
            }
        });
    }
}

#[derive(Clone)]
struct PoolRefusalWriter {
    state: watch::Sender<Option<PoolRefusalStage>>,
}

impl PoolRefusalWriter {
    fn refuse(&self, stage: PoolRefusalStage) {
        self.state.send_if_modified(|current| {
            if current.is_some() {
                false
            } else {
                *current = Some(stage);
                true
            }
        });
    }
}

fn pool_refusal_pair() -> (PoolRefusalWriter, PoolRefusal) {
    let (state, receiver) = watch::channel(None);
    (
        PoolRefusalWriter {
            state: state.clone(),
        },
        PoolRefusal {
            state: receiver,
            _keepalive: state,
        },
    )
}

fn refused_pool_connection() -> sqlx::Error {
    sqlx::Error::Configuration(Box::new(std::io::Error::other(
        "runtime database connection was refused",
    )))
}

fn classify_pool_connection_error(
    error: synveda_types::Error,
    stage: PoolRefusalStage,
    refusal: &PoolRefusalWriter,
) -> sqlx::Error {
    if !matches!(error, synveda_types::Error::Storage { .. }) {
        refusal.refuse(stage);
    }
    refused_pool_connection()
}

/// Builds the shared gateway/worker pool contract. Every physical connection
/// is session-initialized and pinned to one writable database identity; every
/// checkout clears tenant/maintenance state before application SQL can run.
/// The options and refusal handle are an inseparable pair: the pool built from
/// these options must be passed to the authority monitor with this handle.
pub fn runtime_pool_options(
    max_connections: u32,
    acquire_timeout: Duration,
    expected_principal: String,
) -> (PgPoolOptions, PoolRefusal) {
    let expected_principal: Arc<str> = expected_principal.into();
    let pinned_target = Arc::new(Mutex::new(None::<PinnedDatabaseTarget>));
    let (refusal_writer, refusal) = pool_refusal_pair();
    let options = PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(acquire_timeout)
        .after_connect({
            let expected_principal = Arc::clone(&expected_principal);
            let pinned_target = Arc::clone(&pinned_target);
            let refusal_writer = refusal_writer.clone();
            move |connection, _metadata| {
                let expected_principal = Arc::clone(&expected_principal);
                let pinned_target = Arc::clone(&pinned_target);
                let refusal_writer = refusal_writer.clone();
                Box::pin(async move {
                    synveda_store::runtime_role::initialize_product_session_connection(connection)
                        .await
                        .map_err(|error| {
                            classify_pool_connection_error(
                                error,
                                PoolRefusalStage::Session,
                                &refusal_writer,
                            )
                        })?;
                    synveda_store::runtime_role::verify_selected_principal_connection(
                        connection,
                        &expected_principal,
                    )
                    .await
                    .map_err(|error| {
                        classify_pool_connection_error(
                            error,
                            PoolRefusalStage::Role,
                            &refusal_writer,
                        )
                    })?;
                    let candidate =
                        synveda_store::runtime_role::database_identity_connection(connection)
                            .await
                            .map(PinnedDatabaseTarget::from)
                            .map_err(|error| {
                                classify_pool_connection_error(
                                    error,
                                    PoolRefusalStage::Identity,
                                    &refusal_writer,
                                )
                            })?;
                    let mut pinned = pinned_target.lock().map_err(|_| {
                        refusal_writer.refuse(PoolRefusalStage::Identity);
                        refused_pool_connection()
                    })?;
                    match pinned.as_ref() {
                        Some(expected) if expected != &candidate => {
                            refusal_writer.refuse(PoolRefusalStage::Identity);
                            return Err(refused_pool_connection());
                        }
                        Some(_) => {}
                        None => *pinned = Some(candidate),
                    }
                    Ok(())
                })
            }
        })
        .before_acquire({
            let expected_principal = Arc::clone(&expected_principal);
            move |connection, _metadata| {
                let expected_principal = Arc::clone(&expected_principal);
                let refusal_writer = refusal_writer.clone();
                Box::pin(async move {
                    synveda_store::runtime_role::initialize_product_session_connection(connection)
                        .await
                        .map_err(|error| {
                            classify_pool_connection_error(
                                error,
                                PoolRefusalStage::Session,
                                &refusal_writer,
                            )
                        })?;
                    synveda_store::runtime_role::verify_selected_principal_connection(
                        connection,
                        &expected_principal,
                    )
                    .await
                    .map_err(|error| {
                        classify_pool_connection_error(
                            error,
                            PoolRefusalStage::Role,
                            &refusal_writer,
                        )
                    })?;
                    Ok(true)
                })
            }
        });
    (options, refusal)
}

fn resolve_setting(
    name: &str,
    direct: Option<OsString>,
    file: Option<OsString>,
) -> Result<Option<String>, String> {
    match (direct, file) {
        (Some(_), Some(_)) => Err(format!(
            "{name} and {name}_FILE are mutually exclusive; configure exactly one"
        )),
        (Some(value), None) => value
            .into_string()
            .map(Some)
            .map_err(|_| format!("{name} must be valid UTF-8")),
        (None, Some(path)) => read_setting_file(name, Path::new(&path)).map(Some),
        (None, None) => Ok(None),
    }
}

fn read_setting_file(name: &str, path: &Path) -> Result<String, String> {
    if path.as_os_str().is_empty() {
        return Err(format!("{name}_FILE must name a file"));
    }
    let file = open_nonblocking_read(path).map_err(|_| format!("{name}_FILE cannot be read"))?;
    let metadata = file
        .metadata()
        .map_err(|_| format!("{name}_FILE cannot be read"))?;
    if !metadata.is_file() {
        return Err(format!("{name}_FILE must name a regular file"));
    }
    if metadata.len() > MAX_SETTING_FILE_BYTES {
        return Err(format!(
            "{name}_FILE exceeds the {MAX_SETTING_FILE_BYTES} byte startup bound"
        ));
    }
    let capacity = usize::try_from(metadata.len().min(MAX_SETTING_FILE_BYTES + 1))
        .map_err(|_| format!("{name}_FILE exceeds the startup bound"))?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(MAX_SETTING_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| format!("{name}_FILE cannot be read"))?;
    if bytes.len() as u64 > MAX_SETTING_FILE_BYTES {
        return Err(format!(
            "{name}_FILE exceeds the {MAX_SETTING_FILE_BYTES} byte startup bound"
        ));
    }
    let mut value =
        String::from_utf8(bytes).map_err(|_| format!("{name}_FILE must contain valid UTF-8"))?;
    if let Some(stripped) = value.strip_suffix("\r\n") {
        value.truncate(stripped.len());
    } else if value.ends_with('\n') {
        value.pop();
    }
    if value.contains('\0') {
        return Err(format!("{name}_FILE must not contain NUL bytes"));
    }
    Ok(value)
}

fn open_nonblocking_read(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        // Refuse a FIFO/device from its opened descriptor without ever
        // waiting for a producer. Projected-secret symlinks remain valid
        // because their target descriptor is a regular file.
        options.custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    options.open(path)
}

/// Builds the local key-management provider from the current environment.
///
/// `SYNVEDA_KMS_KEY` is a 64-character hexadecimal key. Absence means the
/// fail-closed disabled provider; malformed configured material is refused
/// without echoing it.
pub fn kms_from_env() -> Result<synveda_crypto::Kms, String> {
    let Some(key) = setting("SYNVEDA_KMS_KEY")?.filter(|value| !value.trim().is_empty()) else {
        return Ok(synveda_crypto::Kms::Disabled);
    };
    let key_ref = setting("SYNVEDA_KMS_KEY_REF")?
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "local:default".to_string());
    synveda_crypto::LocalKms::from_hex(&key, key_ref)
        .map(synveda_crypto::Kms::Local)
        .map_err(|err| format!("SYNVEDA_KMS_KEY is not usable: {err}"))
}

/// Builds the configured Capture extractor.
///
/// There is deliberately no `off`: a terminal session has durably requested
/// candidate extraction, so a deployment must select an implementation.
pub(crate) fn extractor_from_env() -> Result<synveda_ingest::extraction::AnyExtractor, String> {
    use synveda_ingest::extraction::{
        AnyExtractor, ClaudeExtractor, DeterministicExtractor, VllmExtractor,
    };
    let selected = std::env::var("SYNVEDA_EXTRACTOR")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "deterministic".to_owned());
    let model = std::env::var("SYNVEDA_EXTRACTOR_MODEL")
        .ok()
        .filter(|value| !value.is_empty());
    match selected.as_str() {
        "deterministic" => Ok(AnyExtractor::Deterministic(DeterministicExtractor::new())),
        "claude" => {
            let api_key = setting("ANTHROPIC_API_KEY")?
                .filter(|value| !value.is_empty())
                .ok_or("SYNVEDA_EXTRACTOR=claude requires ANTHROPIC_API_KEY")?;
            let base_url = std::env::var("SYNVEDA_ANTHROPIC_BASE_URL")
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| ClaudeExtractor::DEFAULT_BASE_URL.to_owned());
            let model = model.unwrap_or_else(|| ClaudeExtractor::DEFAULT_MODEL.to_owned());
            ClaudeExtractor::new(api_key, model, base_url)
                .map(AnyExtractor::Claude)
                .map_err(|_| "could not configure the Claude extractor HTTP client".to_owned())
        }
        "vllm" => {
            let base_url = std::env::var("SYNVEDA_VLLM_BASE_URL")
                .ok()
                .filter(|value| !value.is_empty())
                .ok_or("SYNVEDA_EXTRACTOR=vllm requires SYNVEDA_VLLM_BASE_URL")?;
            let model = model.ok_or("SYNVEDA_EXTRACTOR=vllm requires SYNVEDA_EXTRACTOR_MODEL")?;
            VllmExtractor::new(model, base_url)
                .map(AnyExtractor::Vllm)
                .map_err(|_| "could not configure the vLLM extractor HTTP client".to_owned())
        }
        other => Err(format!(
            "SYNVEDA_EXTRACTOR must be deterministic|claude|vllm, got {other:?}"
        )),
    }
}

/// Builds Capture polling and lease tuning.
///
/// Invalid operational tuning falls back to conservative defaults. Every
/// invocation appends a fresh identifier to the optional operator prefix so
/// two processes can never share lease ownership accidentally.
pub(crate) fn capture_config_from_env() -> Result<synveda_ingest::capture_worker::Config, String> {
    let defaults = synveda_ingest::capture_worker::Config::default();
    let lease_owner_prefix = std::env::var("SYNVEDA_CAPTURE_LEASE_OWNER").ok();
    let lease_owner = capture_lease_owner(lease_owner_prefix.as_deref(), &defaults.lease_owner);
    let poll_interval = bounded_u64_setting("SYNVEDA_CAPTURE_POLL_MS", 1, 60_000)?
        .map(Duration::from_millis)
        .unwrap_or(defaults.poll_interval);
    let lease_duration = bounded_u64_setting("SYNVEDA_CAPTURE_LEASE_SECS", 1, 3_600)?
        .map(Duration::from_secs)
        .unwrap_or(defaults.lease_duration);
    let batches_per_tenant = bounded_u64_setting("SYNVEDA_CAPTURE_BATCHES_PER_TENANT", 1, 64)?
        .map(|value| {
            usize::try_from(value)
                .map_err(|_| "SYNVEDA_CAPTURE_BATCHES_PER_TENANT exceeds this platform".to_owned())
        })
        .transpose()?
        .unwrap_or(defaults.batches_per_tenant);
    Ok(synveda_ingest::capture_worker::Config {
        poll_interval,
        lease_duration,
        batches_per_tenant,
        lease_owner,
    })
}

fn capture_lease_owner(prefix: Option<&str>, default: &str) -> String {
    prefix
        .filter(|value| !value.trim().is_empty())
        .map(|prefix| {
            let suffix = synveda_types::CaptureBatchId::new();
            let prefix = prefix.trim().chars().take(218).collect::<String>();
            format!("{prefix}-{suffix}")
        })
        .unwrap_or_else(|| default.to_owned())
}

/// Builds the configured Knowledge embedder.
///
/// Context composition and the worker's immutable Knowledge index use the
/// same explicit implementation and model identity.
pub fn embedder_from_env() -> Result<synveda_ingest::embedding::AnyEmbedder, String> {
    use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder, TeiEmbedder};
    let selected = std::env::var("SYNVEDA_EMBEDDER")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "deterministic".to_owned());
    match selected.as_str() {
        "deterministic" => Ok(AnyEmbedder::Deterministic(DeterministicEmbedder::new())),
        "tei" => {
            let base_url = std::env::var("SYNVEDA_TEI_URL")
                .ok()
                .filter(|value| !value.is_empty())
                .ok_or("SYNVEDA_EMBEDDER=tei requires SYNVEDA_TEI_URL")?;
            let model = std::env::var("SYNVEDA_EMBEDDER_MODEL")
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| TeiEmbedder::DEFAULT_MODEL.to_owned());
            TeiEmbedder::new(model, base_url)
                .map(AnyEmbedder::Tei)
                .map_err(|_| "could not configure the TEI embedder HTTP client".to_owned())
        }
        other => Err(format!(
            "SYNVEDA_EMBEDDER must be deterministic|tei, got {other:?}"
        )),
    }
}

/// Builds the statically tenant-bound directory connectors declared by OIDC
/// issuer configuration.
///
/// A timer has no request token from which to read a tenant claim, so an
/// issuer that enables pull sync must bind its tenant statically. Two issuers
/// may not declare pull authority for the same tenant.
pub(crate) fn build_directory_connectors(
    issuers: &[synveda_identity::IssuerConfig],
) -> Result<HashMap<TenantId, Box<dyn DirectoryConnector>>, Box<dyn std::error::Error>> {
    let mut connectors = HashMap::new();
    for issuer in issuers {
        let Some(config) = &issuer.directory_sync else {
            continue;
        };
        let synveda_identity::TenantBinding::Static { tenant_id } = &issuer.tenant else {
            return Err(format!(
                "issuer {} configures `directory_sync` with a claim-bound tenant: a \
                 pull sync runs on a timer with no request to read a claim from, so \
                 it needs `tenant: {{\"static\": ...}}` (AUTH-5, ADR-0060)",
                issuer.issuer
            )
            .into());
        };
        let connector = synveda_identity::directory::connector(config)?;
        tracing::info!(
            issuer = issuer.issuer,
            tenant.id = %tenant_id,
            connector = connector.name(),
            "directory pull sync configured"
        );
        if connectors.insert(*tenant_id, connector).is_some() {
            return Err(format!(
                "tenant {tenant_id} is pull-synced by two issuers: one directory is \
                 the authority for one tenant (ADR-0060 decision 5)"
            )
            .into());
        }
    }
    Ok(connectors)
}

/// Parses directory-sync pacing and breaker tuning.
///
/// Values that would make absence evidence eager are startup errors rather
/// than silently clamped operator intent.
pub(crate) fn directory_sync_config_from_env()
-> Result<crate::directory_sync::SyncConfig, Box<dyn std::error::Error>> {
    let defaults = crate::directory_sync::SyncConfig::default();
    let absence_passes_raw = std::env::var("SYNVEDA_DIRECTORY_ABSENCE_PASSES").ok();
    let absence_passes =
        directory_absence_passes(absence_passes_raw.as_deref(), defaults.absence_passes)?;
    let breaker_fraction = match std::env::var("SYNVEDA_DIRECTORY_BREAKER_FRACTION") {
        Ok(value) => {
            let parsed: f64 = value
                .parse()
                .map_err(|_| "SYNVEDA_DIRECTORY_BREAKER_FRACTION must be a number")?;
            if !(0.0..=1.0).contains(&parsed) {
                return Err("SYNVEDA_DIRECTORY_BREAKER_FRACTION must be between 0 and 1".into());
            }
            parsed
        }
        Err(_) => defaults.breaker_fraction,
    };
    Ok(crate::directory_sync::SyncConfig {
        interval: std::env::var("SYNVEDA_DIRECTORY_SYNC_INTERVAL_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|secs| *secs > 0)
            .map_or(defaults.interval, Duration::from_secs),
        absence_passes,
        breaker_fraction,
        breaker_floor: std::env::var("SYNVEDA_DIRECTORY_BREAKER_FLOOR")
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|floor| *floor >= 0)
            .unwrap_or(defaults.breaker_floor),
    })
}

fn directory_absence_passes(
    raw: Option<&str>,
    default: i32,
) -> Result<i32, Box<dyn std::error::Error>> {
    match raw {
        Some(value) => {
            let parsed: i32 = value
                .parse()
                .map_err(|_| "SYNVEDA_DIRECTORY_ABSENCE_PASSES must be an integer")?;
            if parsed < 1 {
                return Err("SYNVEDA_DIRECTORY_ABSENCE_PASSES must be at least 1: a \
                            threshold of zero seals somebody the first time one page \
                            of a directory read is throttled (ADR-0060 decision 3.2)"
                    .into());
            }
            Ok(parsed)
        }
        None => Ok(default),
    }
}

/// Reads a positive process pool bound from `name`, or returns `default`.
pub fn positive_connection_limit(name: &str, default: u32) -> Result<u32, String> {
    let raw = std::env::var(name).ok();
    positive_connection_limit_value(name, raw.as_deref(), default)
}

fn positive_connection_limit_value(
    name: &str,
    raw: Option<&str>,
    default: u32,
) -> Result<u32, String> {
    raw.map(|raw| {
        raw.parse::<u32>()
            .map_err(|_| format!("{name} must be a positive integer, got `{raw}`"))
            .and_then(|value| match value {
                1..=64 => Ok(value),
                0 => Err(format!("{name} must be at least 1")),
                _ => Err(format!("{name} must be at most 64")),
            })
    })
    .transpose()
    .map(|value| value.unwrap_or(default))
}

/// Parses one bounded integer setting for either product process.
pub(crate) fn bounded_u64_setting(
    name: &str,
    minimum: u64,
    maximum: u64,
) -> Result<Option<u64>, String> {
    let raw = std::env::var(name).ok();
    bounded_u64_value(name, raw.as_deref(), minimum, maximum)
}

/// Parses one bounded duration setting with identical gateway/worker
/// semantics. Sharing this function prevents one product image from accepting
/// a value in one command and refusing it in the other.
pub fn bounded_duration_setting(
    name: &str,
    default: u64,
    minimum: u64,
    maximum: u64,
) -> Result<Duration, String> {
    Ok(Duration::from_secs(
        bounded_u64_setting(name, minimum, maximum)?.unwrap_or(default),
    ))
}

fn bounded_u64_value(
    name: &str,
    raw: Option<&str>,
    minimum: u64,
    maximum: u64,
) -> Result<Option<u64>, String> {
    raw.map(|raw| {
        let value = raw
            .parse::<u64>()
            .map_err(|_| format!("{name} must be an integer, got `{raw}`"))?;
        if !(minimum..=maximum).contains(&value) {
            return Err(format!(
                "{name} must be between {minimum} and {maximum}, got `{value}`"
            ));
        }
        Ok(value)
    })
    .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use synveda_identity::directory::{DirectorySyncConfig, Secret};

    fn issuer(tenant: synveda_identity::TenantBinding) -> synveda_identity::IssuerConfig {
        let json = r#"[{"issuer":"https://idp.example","client_id":"c"}]"#;
        let mut parsed = synveda_identity::parse_issuers(json).expect("parse");
        let mut config = parsed.remove(0);
        config.tenant = tenant;
        config
    }

    fn okta() -> DirectorySyncConfig {
        DirectorySyncConfig::Okta {
            org_url: "https://example.okta.com".to_owned(),
            api_token: Secret::new("token"),
        }
    }

    fn database_roles_with_topology(
        forbidden_databases: &str,
        isolated_peer_roles: &str,
    ) -> synveda_store::runtime_role::DatabaseRoles {
        let json = format!(
            r#"{{"migrator":"migrator","gateway":"gateway","worker":"worker","administrators":["administrator"],"administrative_memberships":[],"forbidden_databases":{forbidden_databases},"isolated_peer_roles":{isolated_peer_roles}}}"#
        );
        synveda_store::runtime_role::DatabaseRoles::parse_json(&json)
            .expect("parse database role fixture")
    }

    #[tokio::test]
    async fn pool_refusal_retains_the_first_terminal_stage() {
        let (writer, mut refusal) = pool_refusal_pair();
        assert_eq!(refusal.current(), None);

        writer.refuse(PoolRefusalStage::Session);
        assert_eq!(
            refusal.wait_until_refused().await,
            PoolRefusalStage::Session
        );

        writer.refuse(PoolRefusalStage::Identity);
        drop(writer);
        assert_eq!(refusal.current(), Some(PoolRefusalStage::Session));
    }

    #[test]
    fn only_non_storage_hook_errors_publish_a_terminal_refusal() {
        let (writer, refusal) = pool_refusal_pair();
        let _ = classify_pool_connection_error(
            synveda_types::Error::Storage {
                message: "transient fixture".to_owned(),
            },
            PoolRefusalStage::Session,
            &writer,
        );
        assert_eq!(refusal.current(), None);

        let _ = classify_pool_connection_error(
            synveda_types::Error::Invalid {
                message: "terminal fixture".to_owned(),
            },
            PoolRefusalStage::Role,
            &writer,
        );
        assert_eq!(refusal.current(), Some(PoolRefusalStage::Role));
    }

    #[test]
    fn required_database_peer_must_be_bidirectional() {
        let exact = database_roles_with_topology(
            r#"["keycloak","postgres","template1"]"#,
            r#"["keycloak"]"#,
        );
        validate_required_database_peer(&exact, Some("keycloak"))
            .expect("accept exact bidirectional peer");
        validate_required_database_peer(&exact, None).expect("peer is optional");

        for one_sided in [
            database_roles_with_topology(r#"["postgres","template1"]"#, r#"["keycloak"]"#),
            database_roles_with_topology(r#"["keycloak","postgres","template1"]"#, r#"[]"#),
        ] {
            assert_eq!(
                validate_required_database_peer(&one_sided, Some("keycloak"))
                    .expect_err("one-sided peer must be refused"),
                "SYNVEDA_DATABASE_REQUIRED_PEER is absent from the configured bidirectional isolation contract"
            );
        }
    }

    #[test]
    fn a_pull_sync_needs_a_statically_bound_issuer() {
        let mut claim_bound = issuer(synveda_identity::TenantBinding::Claim {
            name: "tid".to_owned(),
        });
        claim_bound.directory_sync = Some(okta());
        let message = build_directory_connectors(std::slice::from_ref(&claim_bound))
            .err()
            .expect("a claim-bound pull sync is refused")
            .to_string();
        assert!(message.contains("static"));

        let tenant_id = TenantId::new();
        let mut bound = issuer(synveda_identity::TenantBinding::Static { tenant_id });
        bound.directory_sync = Some(okta());
        let built = build_directory_connectors(std::slice::from_ref(&bound)).expect("built");
        assert_eq!(built.len(), 1);
        assert_eq!(built[&tenant_id].name(), "okta");
    }

    #[test]
    fn an_issuer_with_no_directory_sync_contributes_no_connector() {
        let plain = issuer(synveda_identity::TenantBinding::Static {
            tenant_id: TenantId::new(),
        });
        assert!(
            build_directory_connectors(std::slice::from_ref(&plain))
                .expect("built")
                .is_empty()
        );
    }

    #[test]
    fn two_issuers_cannot_pull_one_tenant() {
        let tenant_id = TenantId::new();
        let mut first = issuer(synveda_identity::TenantBinding::Static { tenant_id });
        first.directory_sync = Some(okta());
        let mut second = issuer(synveda_identity::TenantBinding::Static { tenant_id });
        second.issuer = "https://other.example".to_owned();
        second.directory_sync = Some(okta());
        assert!(build_directory_connectors(&[first, second]).is_err());
    }

    #[test]
    fn an_absence_threshold_of_zero_is_refused_rather_than_clamped() {
        let refused = directory_absence_passes(Some("0"), 2);
        assert!(refused.is_err());
    }

    #[test]
    fn configured_capture_owner_is_a_bounded_prefix_not_an_instance_id() {
        let prefix = "x".repeat(400);
        let first = capture_lease_owner(Some(&prefix), "default");
        let second = capture_lease_owner(Some(&prefix), "default");

        assert_ne!(first, second);
        assert_eq!(first.chars().count(), 255);
        assert_eq!(second.chars().count(), 255);
        assert!(first.starts_with(&"x".repeat(218)));
        assert!(second.starts_with(&"x".repeat(218)));
    }

    #[test]
    fn connection_limits_are_positive() {
        let refused =
            positive_connection_limit_value("SYNVEDA_TEST_CONNECTION_LIMIT", Some("0"), 8);
        assert!(refused.is_err());
    }

    #[test]
    fn shared_process_durations_are_bounded_without_clamping() {
        assert_eq!(
            bounded_u64_value("SYNVEDA_POLICY_REFRESH_SECS", None, 1, 3_600).unwrap(),
            None
        );
        for refused in ["0", "3601", "not-an-integer"] {
            assert!(
                bounded_u64_value("SYNVEDA_POLICY_REFRESH_SECS", Some(refused), 1, 3_600).is_err(),
                "{refused} must be refused by both product commands"
            );
        }
    }

    #[test]
    fn direct_and_file_settings_are_mutually_exclusive_without_echoing_values() {
        let refused = resolve_setting(
            "SYNVEDA_TEST_SECRET",
            Some(OsString::from("SYNVEDA_SECRET_SENTINEL_DIRECT")),
            Some(OsString::from("/run/secrets/test")),
        )
        .expect_err("ambiguous secret input is refused");
        assert!(refused.contains("mutually exclusive"));
        assert!(!refused.contains("SYNVEDA_SECRET_SENTINEL"));
    }

    #[test]
    fn file_settings_drop_one_secret_file_newline() {
        let path = std::env::temp_dir().join(format!(
            "synveda-setting-{}-{}",
            std::process::id(),
            synveda_types::CaptureBatchId::new()
        ));
        std::fs::write(&path, b"SYNVEDA_SECRET_SENTINEL\n").expect("write setting fixture");
        let resolved = resolve_setting(
            "SYNVEDA_TEST_SECRET",
            None,
            Some(path.clone().into_os_string()),
        )
        .expect("read setting")
        .expect("setting present");
        let _ = std::fs::remove_file(path);
        assert_eq!(resolved, "SYNVEDA_SECRET_SENTINEL");
    }

    #[test]
    fn file_settings_refuse_bytes_beyond_the_startup_allocation_bound() {
        let path = std::env::temp_dir().join(format!(
            "synveda-setting-{}-{}",
            std::process::id(),
            synveda_types::CaptureBatchId::new()
        ));
        let file = std::fs::File::create(&path).expect("create oversized setting fixture");
        file.set_len(MAX_SETTING_FILE_BYTES + 1)
            .expect("size oversized setting fixture");
        let refused = read_setting_file("SYNVEDA_TEST_SECRET", &path)
            .expect_err("oversized setting is refused");
        let _ = std::fs::remove_file(path);
        assert!(refused.contains("startup bound"));
    }

    #[cfg(unix)]
    #[test]
    fn file_settings_refuse_a_fifo_without_waiting_for_a_writer() {
        let path = std::env::temp_dir().join(format!(
            "synveda-setting-fifo-{}-{}",
            std::process::id(),
            synveda_types::CaptureBatchId::new()
        ));
        let status = std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .expect("create FIFO fixture");
        assert!(status.success(), "mkfifo failed");

        let (sender, receiver) = std::sync::mpsc::channel();
        let thread_path = path.clone();
        std::thread::spawn(move || {
            let _ = sender.send(read_setting_file("SYNVEDA_TEST_SECRET", &thread_path));
        });
        let result = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("FIFO path blocked the runtime configuration boundary");
        let error = result.expect_err("FIFO must not be accepted as a setting file");
        assert!(error.contains("regular file"), "{error}");
        let _ = std::fs::remove_file(path);
    }

    #[cfg(unix)]
    #[test]
    fn file_settings_accept_a_projected_secret_symlink() {
        use std::os::unix::fs::symlink;

        let stem = format!(
            "synveda-setting-projected-{}-{}",
            std::process::id(),
            synveda_types::CaptureBatchId::new()
        );
        let directory = std::env::temp_dir().join(stem);
        std::fs::create_dir_all(&directory).expect("create projected-secret fixture");
        let target = directory.join("..data-secret");
        let path = directory.join("secret");
        std::fs::write(&target, b"SYNVEDA_SECRET_SENTINEL\n")
            .expect("write projected-secret target");
        symlink(&target, &path).expect("link projected-secret path");
        assert_eq!(
            read_setting_file("SYNVEDA_TEST_SECRET", &path).expect("read projected secret"),
            "SYNVEDA_SECRET_SENTINEL"
        );
        let _ = std::fs::remove_dir_all(directory);
    }
}
