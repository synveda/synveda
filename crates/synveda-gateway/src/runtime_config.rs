//! Process-neutral runtime configuration shared by the gateway and core worker.
//!
//! These functions interpret application settings only. Compose, direct
//! binaries and later Kubernetes supply values differently, but they must not
//! assign them different meanings (CPR-45, ADR-0102).

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Read as _;
use std::path::Path;
use std::time::Duration;

use synveda_identity::directory::DirectoryConnector;
use synveda_types::TenantId;

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
    let file = std::fs::File::open(path).map_err(|_| format!("{name}_FILE cannot be read"))?;
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
}
