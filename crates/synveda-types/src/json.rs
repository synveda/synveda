//! Canonical JSON, for the two places this product hashes a caller's object.
//!
//! # Why this is not a no-op, and why that was a surprise
//!
//! `serde_json::Map` is a `BTreeMap` — **unless** something in the dependency
//! graph turns on `serde_json/preserve_order`, in which case it becomes an
//! `IndexMap` and iterates in *insertion* order. Cargo unifies features across
//! a workspace, so that is a global property of a build rather than of the
//! crate that asked for it.
//!
//! In this repository it **is** on: `cedar-policy-core` enables it. So in any
//! build that includes the policy crate — which is every `cargo build`,
//! `cargo test --workspace` and every binary this product ships —
//! `Value::to_string()` reflects the order a client happened to write its keys
//! in, and two byte-different encodings of one object hash differently.
//!
//! It also means the behaviour **changes with the build's scope**: `cargo test
//! -p synveda-types` has no Cedar in its graph and no `preserve_order`, so the
//! same two objects encode identically there. That is the strongest argument
//! for canonicalising unconditionally rather than where it looks needed — a
//! property that depends on which crates were compiled is not one anybody
//! should be reasoning about at a call site.
//!
//! That was found by a unit test rather than by reading — CPR-4 had written
//! this same recursion inside the gateway's idempotency seam with a comment
//! saying it was a no-op "today", kept only against the day somebody turned
//! the flag on. The flag was already on. The mechanism was right and the
//! reasoning behind it was wrong, which is the failure mode a comment is worst
//! at catching, so it is stated here where the code is.
//!
//! # What "canonical" means here
//!
//! Object keys sorted, recursively. Nothing else: arrays keep their order
//! (an array's order is content), numbers keep their encoding, and no
//! normalisation of strings or floats is attempted. This is exactly enough to
//! make "the same object, written differently" hash the same, and deliberately
//! not a general canonical-JSON implementation — the two callers hash requests
//! and event payloads, not signatures.

use serde_json::Value;

/// Returns `value` with every object's keys sorted, recursively.
///
/// Hash the result, never the input:
///
/// ```
/// use synveda_types::json::canonicalise;
///
/// let a = serde_json::json!({"b": 1, "a": 2});
/// let b = serde_json::json!({"a": 2, "b": 1});
/// assert_eq!(canonicalise(&a).to_string(), canonicalise(&b).to_string());
/// ```
#[must_use]
pub fn canonicalise(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), canonicalise(&map[key])))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalise).collect()),
        scalar => scalar.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property, unconditionally.
    ///
    /// It deliberately does **not** assert that the two raw `to_string()`s
    /// differ, and the reason is the same subtlety this module exists for:
    /// whether they differ depends on **which crates are in the build**.
    /// `cargo test -p synveda-types` compiles this crate's own subtree, which
    /// has no Cedar in it and therefore no `preserve_order`, and the raw
    /// strings match; `cargo test --workspace` unifies the feature in and they
    /// do not. An assertion about that would pass under one command and fail
    /// under the other — which is exactly why the canonicalisation is
    /// unconditional rather than a thing switched on when it looks needed.
    #[test]
    fn two_encodings_of_one_object_canonicalise_to_one_string() {
        let a = serde_json::json!({"b": {"z": 1, "a": 2}, "a": [3, 4]});
        let b = serde_json::json!({"a": [3, 4], "b": {"a": 2, "z": 1}});
        assert_eq!(canonicalise(&a).to_string(), canonicalise(&b).to_string());
        assert_eq!(
            canonicalise(&a).to_string(),
            r#"{"a":[3,4],"b":{"a":2,"z":1}}"#,
            "keys ascending at every depth"
        );
    }

    #[test]
    fn an_array_keeps_its_order_because_an_array_s_order_is_content() {
        let a = serde_json::json!([1, 2, 3]);
        let b = serde_json::json!([3, 2, 1]);
        assert_ne!(canonicalise(&a).to_string(), canonicalise(&b).to_string());
        // And objects nested inside arrays are still canonicalised.
        let c = serde_json::json!([{"b": 1, "a": 2}]);
        let d = serde_json::json!([{"a": 2, "b": 1}]);
        assert_eq!(canonicalise(&c).to_string(), canonicalise(&d).to_string());
    }

    #[test]
    fn scalars_and_empties_pass_through_unchanged() {
        for value in [
            serde_json::json!(null),
            serde_json::json!(true),
            serde_json::json!(7),
            serde_json::json!("text"),
            serde_json::json!({}),
            serde_json::json!([]),
        ] {
            assert_eq!(canonicalise(&value), value);
        }
    }
}
