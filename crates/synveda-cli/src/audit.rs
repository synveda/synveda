//! Public-API client for audit query, frozen export and verification
//! (CPR-29/33, ADR-0088/0092).
//!
//! The command sees only what `AuditRead` permits and every read is itself
//! chained by the gateway. It never opens Postgres or recomputes a tenant's
//! chain with operator authority behind the PDP.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::Path;

use serde_json::{Value, json};

use crate::api::{Api, Origin};

/// Content-free public event-search options.
pub struct EventQuery {
    pub actor_subject: Option<String>,
    pub action: Option<String>,
    pub outcome: Option<String>,
    pub resource: Option<String>,
    pub from: Option<String>,
    pub until: Option<String>,
    pub artifact_family: Option<String>,
    pub artifact_id: Option<String>,
    pub artifact_version: Option<String>,
    pub session_id: Option<String>,
    pub context_run_id: Option<String>,
    pub after: i64,
    pub limit: i64,
}

/// Bitemporal Knowledge audit options.
pub struct KnowledgeQuery {
    pub subject: String,
    pub valid_at: Option<String>,
    pub as_known_at: Option<String>,
    pub before: Option<i64>,
    pub limit: i64,
}

/// `synveda audit verify`.
pub async fn verify(profile: &str, json: bool) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin, "verifying");
    let response = api.get("/v1/audit/verify").await?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&response).map_err(|err| err.to_string())?
        );
    } else {
        println!("{}", render_verification(&response));
    }
    if response["valid"].as_bool() == Some(true) {
        Ok(())
    } else {
        Err("audit chain verification failed".to_owned())
    }
}

/// `synveda audit tail`.
///
/// The public collection is forward-keyset paginated. To retain `tail`'s
/// meaning without a storage shortcut, first read the authorised chain head,
/// then start the page immediately before the requested window.
pub async fn tail(profile: &str, limit: i64, json: bool) -> Result<(), String> {
    if !(1..=1000).contains(&limit) {
        return Err("audit tail --limit must be 1..=1000".to_owned());
    }
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin, "reading");
    let verification = api.get("/v1/audit/verify").await?;
    let head = verification["head_seq"]
        .as_i64()
        .ok_or_else(|| "audit verify response has no head_seq".to_owned())?;
    let after = head.saturating_sub(limit).max(0);
    let response = api
        .get(&format!("/v1/audit/events?after={after}&limit={limit}"))
        .await?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&response).map_err(|err| err.to_string())?
        );
        return Ok(());
    }
    let events = response["events"]
        .as_array()
        .ok_or_else(|| "audit events response has no events array".to_owned())?;
    for event in events {
        println!("{event}");
    }
    Ok(())
}

/// `synveda audit events`.
pub async fn events(profile: &str, query: EventQuery, json_output: bool) -> Result<(), String> {
    validate_limit(query.limit)?;
    if query.after < 0 {
        return Err("audit events --after must be non-negative".to_owned());
    }
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin, "querying");
    let mut parameters = vec![
        ("after", query.after.to_string()),
        ("limit", query.limit.to_string()),
    ];
    optional(&mut parameters, "action", query.action);
    optional(&mut parameters, "actor", query.actor_subject);
    optional(&mut parameters, "outcome", query.outcome);
    optional(&mut parameters, "resource", query.resource);
    optional(&mut parameters, "from", query.from);
    optional(&mut parameters, "until", query.until);
    optional(&mut parameters, "artifact_family", query.artifact_family);
    optional(&mut parameters, "artifact_id", query.artifact_id);
    optional(&mut parameters, "artifact_version", query.artifact_version);
    optional(&mut parameters, "session_id", query.session_id);
    optional(&mut parameters, "context_run_id", query.context_run_id);
    let response = api
        .get(&query_path("/v1/audit/events", &parameters))
        .await?;
    render_collection(&response, "events", json_output)
}

/// `synveda audit knowledge`.
pub async fn knowledge(
    profile: &str,
    query: KnowledgeQuery,
    json_output: bool,
) -> Result<(), String> {
    validate_limit(query.limit)?;
    if query.before.is_some_and(|before| before <= 0) {
        return Err("audit knowledge --before must be positive".to_owned());
    }
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin, "reconstructing");
    let mut parameters = vec![
        ("subject", query.subject),
        ("limit", query.limit.to_string()),
    ];
    optional(&mut parameters, "valid_at", query.valid_at);
    optional(&mut parameters, "as_known_at", query.as_known_at);
    if let Some(before) = query.before {
        parameters.push(("before", before.to_string()));
    }
    let response = api
        .get(&query_path("/v1/audit/knowledge", &parameters))
        .await?;
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&response).map_err(|error| error.to_string())?
        );
    } else {
        for field in ["known", "outside_time", "unresolved"] {
            let values = response[field]
                .as_array()
                .ok_or_else(|| format!("audit Knowledge response has no {field} array"))?;
            for value in values {
                println!("{field}: {value}");
            }
        }
    }
    Ok(())
}

/// `synveda audit export` — assemble every page against the first frozen
/// head, verify it locally, then create the requested file without overwrite.
pub async fn export(profile: &str, output: &Path, page_size: i64) -> Result<(), String> {
    validate_limit(page_size)?;
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin, "exporting");
    let mut after = 0_i64;
    let mut frozen: Option<Value> = None;
    let mut events = Vec::new();
    loop {
        let mut parameters = vec![
            ("after", after.to_string()),
            ("limit", page_size.to_string()),
        ];
        if let Some(head) = frozen
            .as_ref()
            .and_then(|value| value["snapshot_seq"].as_i64())
        {
            parameters.push(("through", head.to_string()));
        }
        let page = api
            .get(&query_path("/v1/audit/export", &parameters))
            .await?;
        if let Some(first) = &frozen {
            for field in [
                "format",
                "hash_algorithm",
                "canonicalization",
                "tenant_id",
                "genesis_hash",
                "snapshot_seq",
                "snapshot_hash",
            ] {
                if page[field] != first[field] {
                    return Err(format!(
                        "audit export page changed frozen field {field}; refusing mixed snapshots"
                    ));
                }
            }
        } else {
            frozen = Some(page.clone());
        }
        let page_events = page["events"]
            .as_array()
            .ok_or_else(|| "audit export response has no events array".to_owned())?;
        events.extend(page_events.iter().cloned());
        match page["next_cursor"].as_i64() {
            Some(next) if next > after => after = next,
            Some(_) => return Err("audit export returned a non-advancing cursor".to_owned()),
            None => break,
        }
    }
    let frozen = frozen.ok_or_else(|| "audit export returned no snapshot".to_owned())?;
    let assembled = json!({
        "format": frozen["format"],
        "hash_algorithm": frozen["hash_algorithm"],
        "canonicalization": frozen["canonicalization"],
        "tenant_id": frozen["tenant_id"],
        "genesis_hash": frozen["genesis_hash"],
        "snapshot_seq": frozen["snapshot_seq"],
        "snapshot_hash": frozen["snapshot_hash"],
        "events": events,
    });
    let verified = synveda_audit::verify_export(&assembled).map_err(|error| error.to_string())?;
    let mut encoded = serde_json::to_vec_pretty(&assembled).map_err(|error| error.to_string())?;
    encoded.push(b'\n');
    write_atomic_new(output, &encoded)?;
    println!(
        "exported {} verified events for tenant {} at head {} {} to {}",
        verified.events,
        verified.tenant_id,
        verified.head_seq,
        verified.head_hash,
        output.display()
    );
    Ok(())
}

/// `synveda audit verify-export` — intentionally requires no profile or
/// network access.
pub fn verify_export_file(path: &Path, json_output: bool) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse {} as JSON: {error}", path.display()))?;
    let verified = synveda_audit::verify_export(&value).map_err(|error| error.to_string())?;
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "valid": true,
                "tenant_id": verified.tenant_id,
                "events": verified.events,
                "head_seq": verified.head_seq,
                "head_hash": verified.head_hash,
            }))
            .map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "valid: {} events for tenant {}, head {} {}",
            verified.events, verified.tenant_id, verified.head_seq, verified.head_hash
        );
    }
    Ok(())
}

fn validate_limit(limit: i64) -> Result<(), String> {
    if (1..=1000).contains(&limit) {
        Ok(())
    } else {
        Err("audit page size must be 1..=1000".to_owned())
    }
}

fn write_atomic_new(output: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{} has no usable file name", output.display()))?;
    let mut entropy = [0_u8; 8];
    getrandom::fill(&mut entropy).map_err(|error| format!("system CSPRNG unavailable: {error}"))?;
    let suffix = entropy.iter().fold(String::new(), |mut value, byte| {
        use std::fmt::Write as _;
        let _ = write!(value, "{byte:02x}");
        value
    });
    let temporary = parent.join(format!(".{name}.{suffix}.tmp"));
    let write_result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| format!("create {}: {error}", temporary.display()))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("write {}: {error}", temporary.display()))?;
        std::fs::hard_link(&temporary, output)
            .map_err(|error| format!("create {} without overwrite: {error}", output.display()))
    })();
    let cleanup = std::fs::remove_file(&temporary);
    write_result?;
    cleanup.map_err(|error| format!("remove temporary {}: {error}", temporary.display()))
}

fn optional(
    parameters: &mut Vec<(&'static str, String)>,
    name: &'static str,
    value: Option<String>,
) {
    if let Some(value) = value {
        parameters.push((name, value));
    }
}

fn query_path(base: &str, parameters: &[(&str, String)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer.extend_pairs(
        parameters
            .iter()
            .map(|(name, value)| (*name, value.as_str())),
    );
    format!("{base}?{}", serializer.finish())
}

fn render_collection(response: &Value, field: &str, json_output: bool) -> Result<(), String> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(response).map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    let values = response[field]
        .as_array()
        .ok_or_else(|| format!("audit response has no {field} array"))?;
    for value in values {
        println!("{value}");
    }
    Ok(())
}

fn render_verification(response: &Value) -> String {
    let valid = response["valid"].as_bool().unwrap_or(false);
    let events = response["events"].as_i64().unwrap_or(0);
    let head = response["head_seq"].as_i64().unwrap_or(0);
    let hash = response["head_hash"].as_str().unwrap_or("?");
    if valid {
        format!("valid: {events} events, head {head} {hash}")
    } else {
        format!(
            "BROKEN at {}: {} (head {head} {hash})",
            response["broken_at"]
                .as_i64()
                .map_or_else(|| "unknown".to_owned(), |seq| seq.to_string()),
            response["reason"].as_str().unwrap_or("no reason returned")
        )
    }
}

fn announce(api: &Api, origin: &Origin, verb: &str) {
    match origin {
        Origin::Profile(name) => eprintln!("{verb} as {} (profile {name})", api.subject),
        Origin::Environment => eprintln!("{verb} as {} (SYNVEDA_TOKEN)", api.subject),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_renderer_distinguishes_a_broken_chain() {
        let valid = serde_json::json!({
            "valid": true,
            "events": 12,
            "head_seq": 12,
            "head_hash": "abcd"
        });
        assert_eq!(
            render_verification(&valid),
            "valid: 12 events, head 12 abcd"
        );

        let broken = serde_json::json!({
            "valid": false,
            "events": 12,
            "head_seq": 12,
            "head_hash": "abcd",
            "broken_at": 7,
            "reason": "hash mismatch"
        });
        assert!(render_verification(&broken).starts_with("BROKEN at 7: hash mismatch"));
    }

    #[test]
    fn audit_client_has_no_storage_authority() {
        let source = include_str!("audit.rs");
        for forbidden in [
            concat!("synveda_", "store"),
            concat!("sql", "x"),
            concat!("DATABASE", "_URL"),
            concat!("synveda_audit", "::verify("),
        ] {
            assert!(
                !source.contains(forbidden),
                "audit client contains {forbidden}"
            );
        }
    }

    #[test]
    fn structured_query_values_are_percent_encoded() {
        let path = query_path(
            "/v1/audit/events",
            &[
                ("resource", "scope one/two".to_owned()),
                ("action", "a&b".to_owned()),
            ],
        );
        assert_eq!(
            path,
            "/v1/audit/events?resource=scope+one%2Ftwo&action=a%26b"
        );
    }

    #[test]
    fn export_write_is_atomic_and_never_overwrites() {
        let directory = std::env::temp_dir().join(format!(
            "synveda-audit-export-test-{}",
            synveda_types::TenantId::new()
        ));
        std::fs::create_dir(&directory).expect("create isolated export test directory");
        let output = directory.join("chain.json");
        write_atomic_new(&output, b"first\n").expect("first atomic write");
        assert_eq!(std::fs::read(&output).unwrap(), b"first\n");
        assert!(
            write_atomic_new(&output, b"second\n")
                .unwrap_err()
                .contains("without overwrite")
        );
        assert_eq!(std::fs::read(&output).unwrap(), b"first\n");
        std::fs::remove_file(output).expect("remove owned export fixture");
        std::fs::remove_dir(directory).expect("remove owned export directory");
    }
}
