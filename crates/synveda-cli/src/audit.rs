//! Public-API client for audit query and verification (CPR-29, ADR-0088).
//!
//! The command sees only what `AuditRead` permits and every read is itself
//! chained by the gateway. It never opens Postgres or recomputes a tenant's
//! chain with operator authority behind the PDP.

use serde_json::Value;

use crate::api::{Api, Origin};

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
            concat!("synveda_audit", "::verify"),
        ] {
            assert!(
                !source.contains(forbidden),
                "audit client contains {forbidden}"
            );
        }
    }
}
