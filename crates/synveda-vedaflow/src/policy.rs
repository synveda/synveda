//! The policy snapshot every commit records (FLOW-1, ADR-0030 decision 8).
//!
//! ADR-0003's compliance claim is that "an auditor can prove which policy pack
//! governed any published asset at creation time". This module owns the rule
//! for turning a resolved pack into 32 bytes; it does not own the resolution.
//!
//! It cannot: `synveda-vedaflow` is a middle-tier crate and may not import
//! `synveda-policy` (seed §8). The gateway and the ingestion pipeline already
//! resolve the effective pack for every governed operation, so the caller
//! passes what it has. What lives here is the *encoding*, so two callers with
//! the same pack always produce the same hash.
//!
//! The canonical-JSON rule is `synveda-audit`'s, restated locally rather than
//! shared — sorting keys bytewise at every depth and rejecting non-integer
//! numbers, because a float has no canonical form worth hashing (ADR-0019
//! decision 2). Restated rather than shared because the two crates are
//! siblings; the duplication is eleven lines and is tested on both sides.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use synveda_types::{Error, Result};

use crate::hash::{PolicySnapshotHash, policy_snapshot_writer};

/// The policy pack in force when a commit was made.
///
/// `config` is the pack's non-Cedar configuration — the
/// `CompositionConfig`/`RedactionConfig` shapes packs carry today (ADR-0021
/// decision 3, ADR-0025 decision 2) — as JSON. An unconfigured pack passes
/// `Value::Null`, which hashes distinctly from an empty object: "configured
/// with nothing" and "not configured" are different facts about a pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicySnapshot {
    /// Pack name, e.g. `regulated-strict`.
    pub pack: String,
    /// Pack version, as the pack itself reports it.
    pub version: i64,
    /// The pack's configuration at that version.
    pub config: Value,
}

impl PolicySnapshot {
    /// Builds a snapshot with no configuration recorded.
    #[must_use]
    pub fn new(pack: impl Into<String>, version: i64) -> Self {
        PolicySnapshot {
            pack: pack.into(),
            version,
            config: Value::Null,
        }
    }

    /// Attaches the pack's configuration.
    #[must_use]
    pub fn with_config(mut self, config: Value) -> Self {
        self.config = config;
        self
    }

    /// The snapshot's 32-byte fingerprint, stored on every commit.
    ///
    /// Fails only if `config` contains a non-integer number: jsonb re-renders
    /// floats, so a float simply cannot be part of a hashed value.
    pub fn hash(&self) -> Result<PolicySnapshotHash> {
        let mut config = String::with_capacity(64);
        canonical_json(&self.config, &mut config)?;
        let mut writer = policy_snapshot_writer();
        writer
            .field(self.pack.as_bytes())
            .field(self.version.to_string().as_bytes())
            .field(config.as_bytes());
        Ok(PolicySnapshotHash::from_bytes(writer.finish()))
    }
}

/// Serialises a JSON value canonically: keys sorted bytewise at every depth,
/// non-integer numbers rejected. String escaping is delegated to
/// `serde_json`, whose output for a bare string is deterministic.
fn canonical_json(value: &Value, out: &mut String) -> Result<()> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => {
            if number.is_f64() {
                return Err(Error::Invalid {
                    message: format!(
                        "policy snapshots must not contain non-integer numbers (got {number})"
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
            // Sorted here, explicitly: correctness must not depend on which
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

/// Appends a JSON string literal (quotes included) with serde_json's escaping.
fn push_string(value: &str, out: &mut String) {
    out.push_str(&serde_json::to_string(value).expect("serialising a string cannot fail"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_same_pack_hashes_the_same_however_the_config_was_built() {
        let a = PolicySnapshot::new("regulated-strict", 5)
            .with_config(json!({"budget_tokens": 1500, "channels": "published-only"}));
        // Same content, keys inserted in the other order.
        let b = PolicySnapshot::new("regulated-strict", 5)
            .with_config(json!({"channels": "published-only", "budget_tokens": 1500}));
        assert_eq!(a.hash().unwrap(), b.hash().unwrap());
    }

    #[test]
    fn pack_name_version_and_config_all_move_the_hash() {
        let base = PolicySnapshot::new("regulated-strict", 5).with_config(json!({"a": 1}));
        let renamed = PolicySnapshot::new("standard", 5).with_config(json!({"a": 1}));
        let bumped = PolicySnapshot::new("regulated-strict", 6).with_config(json!({"a": 1}));
        let reconfigured = PolicySnapshot::new("regulated-strict", 5).with_config(json!({"a": 2}));
        let hashes = [
            base.hash().unwrap(),
            renamed.hash().unwrap(),
            bumped.hash().unwrap(),
            reconfigured.hash().unwrap(),
        ];
        for (i, left) in hashes.iter().enumerate() {
            for right in &hashes[i + 1..] {
                assert_ne!(left, right);
            }
        }
    }

    #[test]
    fn unconfigured_and_configured_with_nothing_are_different_facts() {
        let unconfigured = PolicySnapshot::new("standard", 2);
        let empty = PolicySnapshot::new("standard", 2).with_config(json!({}));
        assert_ne!(unconfigured.hash().unwrap(), empty.hash().unwrap());
    }

    #[test]
    fn floats_are_rejected_wherever_they_hide() {
        let snapshot =
            PolicySnapshot::new("standard", 1).with_config(json!({"deep": [{"n": 1.5}]}));
        assert!(matches!(snapshot.hash(), Err(Error::Invalid { .. })));
    }

    #[test]
    fn version_digits_cannot_run_into_the_config() {
        // Length-prefixed fields: "1" + "23…" must not collide with
        // "12" + "3…". The prefix is what stops the boundary sliding.
        let a = PolicySnapshot::new("p", 1).with_config(json!("23"));
        let b = PolicySnapshot::new("p", 12).with_config(json!("3"));
        assert_ne!(a.hash().unwrap(), b.hash().unwrap());
    }
}
