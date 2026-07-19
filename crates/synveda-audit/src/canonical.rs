//! The canonical event serialisation the hash covers (ADR-0019 decision 2).
//!
//! Both append and verify compute this form independently; the stored jsonb
//! is never trusted as a byte source. Properties that make it reproducible:
//!
//! - object keys sorted bytewise at every depth, by this module — never by
//!   relying on `serde_json`'s map ordering, which flips to insertion order
//!   if any crate in the workspace enables `preserve_order` (feature
//!   unification would change our byte stream silently);
//! - no non-integer numbers anywhere: jsonb re-renders floats, so a float
//!   simply cannot be part of a hashed event — [`canonical_json`] rejects
//!   them and append surfaces the rejection as `Error::Invalid`;
//! - timestamps as RFC 3339 UTC with exactly six fractional digits,
//!   matching the microsecond precision timestamptz preserves.

use chrono::{DateTime, Utc};
use serde_json::Value;
use synveda_types::{Error, Result};

/// The canonical event envelope, serialised. `seq`, the tenant, and every
/// column that participates in the hash are inside the serialisation, so a
/// mutated column — not just a mutated payload — changes the hash.
///
/// `trace_id` is omitted entirely when absent (a stored NULL and an absent
/// field must canonicalise identically).
#[allow(clippy::too_many_arguments)] // Mirrors the audit_log columns 1:1.
pub(crate) fn canonical_event(
    tenant: uuid::Uuid,
    seq: i64,
    occurred_at: DateTime<Utc>,
    actor_kind: &str,
    actor_subject: &str,
    action: &str,
    resource: &str,
    outcome: &str,
    payload: &Value,
    trace_id: Option<&str>,
) -> Result<String> {
    let mut out = String::with_capacity(256);
    out.push_str(r#"{"action":"#);
    push_string(action, &mut out);
    out.push_str(r#","actor":{"kind":"#);
    push_string(actor_kind, &mut out);
    out.push_str(r#","subject":"#);
    push_string(actor_subject, &mut out);
    out.push_str(r#"},"occurred_at":"#);
    push_string(&canonical_timestamp(occurred_at), &mut out);
    out.push_str(r#","outcome":"#);
    push_string(outcome, &mut out);
    out.push_str(r#","payload":"#);
    canonical_json(payload, &mut out)?;
    out.push_str(r#","resource":"#);
    push_string(resource, &mut out);
    out.push_str(r#","seq":"#);
    out.push_str(&seq.to_string());
    out.push_str(r#","tenant":"#);
    push_string(&tenant.as_hyphenated().to_string(), &mut out);
    if let Some(trace_id) = trace_id {
        out.push_str(r#","trace_id":"#);
        push_string(trace_id, &mut out);
    }
    out.push_str(r#","v":1}"#);
    Ok(out)
}

/// RFC 3339 UTC with exactly six fractional digits — the one timestamp
/// rendering append and verify agree on. Input must already be truncated
/// to whole microseconds ([`truncate_to_micros`]).
fn canonical_timestamp(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

/// Truncates to whole microseconds so the value hashed is the value the
/// timestamptz column stores — no rounding on insert, no drift on read.
pub(crate) fn truncate_to_micros(at: DateTime<Utc>) -> DateTime<Utc> {
    DateTime::from_timestamp_micros(at.timestamp_micros())
        .expect("a valid DateTime survives microsecond truncation")
}

/// Serialises a JSON value canonically: keys sorted bytewise, floats
/// rejected. String escaping is delegated to `serde_json`, whose output for
/// a bare string is deterministic (minimal escapes, stable `\u` forms).
pub(crate) fn canonical_json(value: &Value, out: &mut String) -> Result<()> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => {
            // i64/u64 render identically everywhere; anything else is a
            // float and has no canonical form worth trusting.
            if number.is_f64() {
                return Err(Error::Invalid {
                    message: format!(
                        "audit payloads must not contain non-integer numbers (got {number})"
                    ),
                });
            }
            out.push_str(&number.to_string());
        }
        Value::String(string) => push_string(string, out),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                canonical_json(item, out)?;
            }
            out.push(']');
        }
        Value::Object(map) => {
            // Sort here, explicitly: correctness must not depend on which
            // map implementation serde_json was compiled with.
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable();
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                push_string(key, out);
                out.push(':');
                canonical_json(&map[key], out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

/// Appends a JSON string literal (quotes included) with serde_json's
/// escaping.
fn push_string(value: &str, out: &mut String) {
    out.push_str(&serde_json::to_string(value).expect("serialising a string cannot fail"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    fn canon(value: &Value) -> Result<String> {
        let mut out = String::new();
        canonical_json(value, &mut out).map(|()| out)
    }

    #[test]
    fn object_keys_sort_bytewise_at_every_depth() {
        // Constructed out of order on purpose; the serialiser must not
        // depend on serde_json's map ordering.
        let value = json!({
            "zeta": {"b": 1, "a": [{"y": true, "x": null}]},
            "alpha": "first",
        });
        assert_eq!(
            canon(&value).unwrap(),
            r#"{"alpha":"first","zeta":{"a":[{"x":null,"y":true}],"b":1}}"#
        );
    }

    #[test]
    fn floats_are_rejected_wherever_they_hide() {
        for value in [json!(1.5), json!({"deep": [{"n": 2.0}]})] {
            assert!(matches!(canon(&value), Err(Error::Invalid { .. })));
        }
    }

    #[test]
    fn integer_extremes_round_trip() {
        let value = json!({"max_u64": u64::MAX, "min_i64": i64::MIN});
        assert_eq!(
            canon(&value).unwrap(),
            r#"{"max_u64":18446744073709551615,"min_i64":-9223372036854775808}"#
        );
    }

    #[test]
    fn strings_use_serde_json_escaping() {
        let value = json!({"key": "quote \" backslash \\ control \u{1f} unicode é"});
        // Round-tripping the canonical form through serde_json proves the
        // escapes are valid JSON meaning the same string.
        let reparsed: Value = serde_json::from_str(&canon(&value).unwrap()).unwrap();
        assert_eq!(reparsed, value);
    }

    #[test]
    fn timestamps_render_with_exactly_six_fraction_digits() {
        let at = Utc.with_ymd_and_hms(2026, 7, 19, 8, 30, 5).unwrap();
        assert_eq!(canonical_timestamp(at), "2026-07-19T08:30:05.000000Z");
        let with_micros = at + chrono::Duration::microseconds(42);
        assert_eq!(
            canonical_timestamp(with_micros),
            "2026-07-19T08:30:05.000042Z"
        );
    }

    #[test]
    fn truncation_drops_sub_microsecond_precision() {
        let at = Utc.with_ymd_and_hms(2026, 7, 19, 8, 30, 5).unwrap()
            + chrono::Duration::nanoseconds(1_999);
        let truncated = truncate_to_micros(at);
        assert_eq!(truncated.timestamp_subsec_nanos(), 1_000);
        // Idempotent: a value already at microsecond precision is unchanged.
        assert_eq!(truncate_to_micros(truncated), truncated);
    }

    #[test]
    fn canonical_event_is_stable_and_omits_absent_trace_id() {
        let tenant = uuid::Uuid::nil();
        let at = Utc.with_ymd_and_hms(2026, 7, 19, 8, 30, 5).unwrap();
        let payload = json!({"pack": "regulated-strict@3"});
        let with = canonical_event(
            tenant,
            1,
            at,
            "subject",
            "alice",
            "authz.decision",
            "scope:x",
            "deny",
            &payload,
            Some("trace"),
        )
        .unwrap();
        let without = canonical_event(
            tenant,
            1,
            at,
            "subject",
            "alice",
            "authz.decision",
            "scope:x",
            "deny",
            &payload,
            None,
        )
        .unwrap();
        assert_eq!(
            with,
            r#"{"action":"authz.decision","actor":{"kind":"subject","subject":"alice"},"occurred_at":"2026-07-19T08:30:05.000000Z","outcome":"deny","payload":{"pack":"regulated-strict@3"},"resource":"scope:x","seq":1,"tenant":"00000000-0000-0000-0000-000000000000","trace_id":"trace","v":1}"#
        );
        assert!(!without.contains("trace_id"));
        assert_ne!(with, without);
    }
}
