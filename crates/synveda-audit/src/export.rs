//! Offline verification for deterministic audit-chain exports (CPR-33,
//! ADR-0092).
//!
//! The public gateway emits a JSON envelope containing the tenant-bound
//! genesis, one frozen head and every canonical input for the contiguous
//! prefix between them. This module deliberately accepts that inert JSON
//! rather than a database connection: the verifier is useful precisely when
//! the service and its storage are not trusted or reachable.

use chrono::{DateTime, Utc};
use serde_json::Value;
use synveda_types::{Error, Result, TenantId};

use crate::canonical::canonical_event;
use crate::chain::{compute_hash, genesis_hash};

/// Stable top-level export format identifier.
pub const EXPORT_FORMAT: &str = "synveda.audit-chain.v1";
/// Hash algorithm and domain rule identifier.
pub const EXPORT_HASH_ALGORITHM: &str = "blake3:synveda-audit-event-v1";
/// Canonical event envelope identifier.
pub const EXPORT_CANONICALIZATION: &str = "synveda.audit-event.v1";

/// Successful offline verification summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineVerification {
    /// Tenant bound into genesis and every canonical event.
    pub tenant_id: TenantId,
    /// Number of contiguous events checked.
    pub events: i64,
    /// Frozen head sequence.
    pub head_seq: i64,
    /// Frozen head hash, lowercase hex.
    pub head_hash: String,
}

/// Verify a complete deterministic export JSON value.
///
/// The export must begin at tenant-bound genesis and end exactly at its frozen
/// head. A page by itself is intentionally not accepted: callers assemble all
/// cursor pages before claiming offline verification.
pub fn verify_export(value: &Value) -> Result<OfflineVerification> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("export must be a JSON object"))?;
    exact_string(object.get("format"), "format", EXPORT_FORMAT)?;
    exact_string(
        object.get("hash_algorithm"),
        "hash_algorithm",
        EXPORT_HASH_ALGORITHM,
    )?;
    exact_string(
        object.get("canonicalization"),
        "canonicalization",
        EXPORT_CANONICALIZATION,
    )?;

    let tenant_text = string(object.get("tenant_id"), "tenant_id")?;
    let tenant_id: TenantId = tenant_text
        .parse()
        .map_err(|_| invalid("tenant_id is invalid"))?;
    let supplied_genesis = hash(object.get("genesis_hash"), "genesis_hash")?;
    let expected_genesis = genesis_hash(tenant_id);
    if supplied_genesis.as_slice() != expected_genesis {
        return Err(invalid("genesis_hash does not bind this tenant"));
    }

    let head_seq = integer(object.get("snapshot_seq"), "snapshot_seq")?;
    if head_seq < 0 {
        return Err(invalid("snapshot_seq must be non-negative"));
    }
    let head_hash = hash(object.get("snapshot_hash"), "snapshot_hash")?;
    let events = object
        .get("events")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("events must be an array"))?;

    let mut previous = expected_genesis.to_vec();
    let mut expected_seq = 1_i64;
    for event in events {
        let event = event
            .as_object()
            .ok_or_else(|| invalid("every export event must be an object"))?;
        let seq = integer(event.get("seq"), "event.seq")?;
        if seq != expected_seq {
            return Err(invalid(&format!(
                "export sequence gap: expected {expected_seq}, got {seq}"
            )));
        }
        let previous_hash = hash(event.get("prev_hash"), "event.prev_hash")?;
        if previous_hash != previous {
            return Err(invalid(&format!(
                "event {seq} previous hash does not match the verified prefix"
            )));
        }
        let occurred_at: DateTime<Utc> = string(event.get("occurred_at"), "event.occurred_at")?
            .parse()
            .map_err(|_| invalid("event.occurred_at is not RFC 3339"))?;
        let trace_id = event.get("trace_id").map(|value| {
            value
                .as_str()
                .ok_or_else(|| invalid("event.trace_id must be a string when present"))
        });
        let trace_id = trace_id.transpose()?;
        let payload = event
            .get("payload")
            .ok_or_else(|| invalid("event.payload is required"))?;
        let canonical = canonical_event(
            tenant_id.as_uuid(),
            seq,
            occurred_at,
            string(event.get("actor_kind"), "event.actor_kind")?,
            string(event.get("actor_subject"), "event.actor_subject")?,
            string(event.get("action"), "event.action")?,
            string(event.get("resource"), "event.resource")?,
            string(event.get("outcome"), "event.outcome")?,
            payload,
            trace_id,
        )?;
        let recomputed = compute_hash(&previous, &canonical);
        let stored = hash(event.get("hash"), "event.hash")?;
        if stored.as_slice() != recomputed {
            return Err(invalid(&format!(
                "event {seq} content hash does not verify"
            )));
        }
        previous = stored;
        expected_seq += 1;
    }

    let checked = expected_seq - 1;
    if checked != head_seq {
        return Err(invalid(&format!(
            "export is incomplete: contains {checked} events for snapshot head {head_seq}"
        )));
    }
    if previous != head_hash {
        return Err(invalid(
            "snapshot_hash does not match the verified final event",
        ));
    }

    Ok(OfflineVerification {
        tenant_id,
        events: checked,
        head_seq,
        head_hash: hex(&head_hash),
    })
}

fn exact_string(value: Option<&Value>, field: &str, expected: &str) -> Result<()> {
    let actual = string(value, field)?;
    if actual == expected {
        Ok(())
    } else {
        Err(invalid(&format!("unsupported {field}: {actual:?}")))
    }
}

fn string<'a>(value: Option<&'a Value>, field: &str) -> Result<&'a str> {
    value
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(&format!("{field} must be a string")))
}

fn integer(value: Option<&Value>, field: &str) -> Result<i64> {
    value
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid(&format!("{field} must be an integer")))
}

fn hash(value: Option<&Value>, field: &str) -> Result<Vec<u8>> {
    let text = string(value, field)?;
    if text.len() != 64 {
        return Err(invalid(&format!(
            "{field} must be 64 lowercase hex characters"
        )));
    }
    let mut bytes = Vec::with_capacity(32);
    for pair in text.as_bytes().chunks_exact(2) {
        let pair = std::str::from_utf8(pair).map_err(|_| invalid(field))?;
        if !pair
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(invalid(&format!("{field} must be lowercase hex")));
        }
        bytes.push(u8::from_str_radix(pair, 16).map_err(|_| invalid(field))?);
    }
    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut output, byte| {
        write!(output, "{byte:02x}").expect("writing into a String cannot fail");
        output
    })
}

fn invalid(message: &str) -> Error {
    Error::Invalid {
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;

    fn valid_export() -> Value {
        let tenant = TenantId::from_uuid(uuid::Uuid::nil());
        let genesis = genesis_hash(tenant);
        let occurred_at = Utc.with_ymd_and_hms(2026, 8, 25, 12, 0, 0).unwrap();
        let payload = json!({"count": 1});
        let canonical = canonical_event(
            tenant.as_uuid(),
            1,
            occurred_at,
            "subject",
            "alice",
            "knowledge.change.applied",
            "scope 00000000-0000-0000-0000-000000000001",
            "success",
            &payload,
            None,
        )
        .unwrap();
        let event_hash = compute_hash(&genesis, &canonical);
        json!({
            "format": EXPORT_FORMAT,
            "hash_algorithm": EXPORT_HASH_ALGORITHM,
            "canonicalization": EXPORT_CANONICALIZATION,
            "tenant_id": tenant,
            "genesis_hash": hex(&genesis),
            "snapshot_seq": 1,
            "snapshot_hash": hex(&event_hash),
            "events": [{
                "seq": 1,
                "occurred_at": occurred_at,
                "actor_kind": "subject",
                "actor_subject": "alice",
                "action": "knowledge.change.applied",
                "resource": "scope 00000000-0000-0000-0000-000000000001",
                "outcome": "success",
                "payload": payload,
                "prev_hash": hex(&genesis),
                "hash": hex(&event_hash),
            }]
        })
    }

    #[test]
    fn a_complete_export_verifies_without_a_database() {
        let verified = verify_export(&valid_export()).unwrap();
        assert_eq!(verified.events, 1);
        assert_eq!(verified.head_seq, 1);
    }

    #[test]
    fn mutation_incompleteness_and_cross_tenant_genesis_are_refused() {
        let mut changed = valid_export();
        changed["events"][0]["payload"]["count"] = json!(2);
        assert!(
            verify_export(&changed)
                .unwrap_err()
                .to_string()
                .contains("content hash")
        );

        let mut incomplete = valid_export();
        incomplete["events"] = json!([]);
        assert!(
            verify_export(&incomplete)
                .unwrap_err()
                .to_string()
                .contains("incomplete")
        );

        let mut transplanted = valid_export();
        transplanted["tenant_id"] = json!(TenantId::new());
        assert!(
            verify_export(&transplanted)
                .unwrap_err()
                .to_string()
                .contains("genesis")
        );
    }
}
