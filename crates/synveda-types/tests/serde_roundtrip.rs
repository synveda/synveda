//! FND-3 acceptance criteria: serde round-trip tests for every public type.

use std::fmt::Debug;
use std::str::FromStr;

use serde::Serialize;
use serde::de::DeserializeOwned;
use synveda_types::session::SessionEventType;
use synveda_types::{
    CompositionConfig, Error, IdentityId, RedactionConfig, RedactionMode, ScopeId, Sensitivity,
    Tenant, TenantId, TenantStatus,
};

fn json_roundtrip<T>(value: &T) -> T
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
{
    let json = serde_json::to_string(value).expect("serialize");
    let back: T = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(&back, value, "round-trip changed the value (json: {json})");
    back
}

// ── Identifiers ──────────────────────────────────────────────────────────────

macro_rules! id_tests {
    ($mod_name:ident, $ty:ident) => {
        mod $mod_name {
            use super::*;

            #[test]
            fn json_roundtrip_preserves_value() {
                json_roundtrip(&$ty::new());
            }

            #[test]
            fn serializes_transparently_as_uuid_string() {
                let id = $ty::new();
                let json = serde_json::to_string(&id).expect("serialize");
                assert_eq!(json, format!("\"{}\"", id.as_uuid().as_hyphenated()));
            }

            #[test]
            fn display_fromstr_roundtrip() {
                let id = $ty::new();
                let parsed = $ty::from_str(&id.to_string()).expect("parse own display output");
                assert_eq!(parsed, id);
            }

            #[test]
            fn new_ids_are_v7_and_unique() {
                let a = $ty::new();
                let b = $ty::new();
                assert_ne!(a, b);
                assert_eq!(a.as_uuid().get_version_num(), 7);
            }

            #[test]
            fn rejects_garbage() {
                assert!($ty::from_str("not-a-uuid").is_err());
                assert!(serde_json::from_str::<$ty>("\"not-a-uuid\"").is_err());
            }
        }
    };
}

id_tests!(tenant_id, TenantId);
id_tests!(scope_id, ScopeId);
id_tests!(identity_id, IdentityId);

// ── Sensitivity ──────────────────────────────────────────────────────────────

#[test]
fn sensitivity_all_levels_roundtrip() {
    for level in Sensitivity::ALL {
        json_roundtrip(&level);
    }
}

#[test]
fn sensitivity_wire_form_is_lowercase_and_matches_as_str() {
    for level in Sensitivity::ALL {
        let json = serde_json::to_string(&level).expect("serialize");
        assert_eq!(json, format!("\"{}\"", level.as_str()));
        assert_eq!(Sensitivity::from_str(level.as_str()).unwrap(), level);
        assert_eq!(level.to_string(), level.as_str());
    }
}

#[test]
fn sensitivity_ordering_is_least_to_most_sensitive() {
    assert!(Sensitivity::Public < Sensitivity::Internal);
    assert!(Sensitivity::Internal < Sensitivity::Confidential);
    assert!(Sensitivity::Confidential < Sensitivity::Restricted);
}

#[test]
fn sensitivity_rejects_unknown_levels() {
    assert!(serde_json::from_str::<Sensitivity>("\"secret\"").is_err());
    assert!(
        Sensitivity::from_str("Restricted").is_err(),
        "wire form is lowercase only"
    );
}

// ── Redaction (MEM-2, ADR-0021) ──────────────────────────────────────────────

#[test]
fn redaction_mode_all_roundtrip_and_match_as_str() {
    for mode in RedactionMode::ALL {
        json_roundtrip(&mode);
        let json = serde_json::to_string(&mode).expect("serialize");
        assert_eq!(json, format!("\"{}\"", mode.as_str()));
        assert_eq!(RedactionMode::from_str(mode.as_str()).unwrap(), mode);
        assert_eq!(mode.to_string(), mode.as_str());
    }
}

#[test]
fn redaction_mode_ordering_is_least_to_most_strict() {
    // The disposition rule (ADR-0021 decision 4): max() over triggered
    // categories' modes must pick deny over quarantine over redact.
    assert!(RedactionMode::Redact < RedactionMode::Quarantine);
    assert!(RedactionMode::Quarantine < RedactionMode::Deny);
}

#[test]
fn redaction_mode_rejects_unknown_modes() {
    assert!(serde_json::from_str::<RedactionMode>("\"drop\"").is_err());
    assert!(
        RedactionMode::from_str("Deny").is_err(),
        "wire form is lowercase only"
    );
}

#[test]
fn redaction_config_roundtrips_and_defaults_strict() {
    json_roundtrip(&RedactionConfig::STRICT);
    json_roundtrip(&RedactionConfig::REDACT_ALL);
    assert_eq!(
        RedactionConfig::default(),
        RedactionConfig::STRICT,
        "an unconfigured pack must fail safe (ADR-0021 decision 3)"
    );
    assert_eq!(RedactionConfig::STRICT.secrets, RedactionMode::Quarantine);
    assert_eq!(RedactionConfig::STRICT.pii, RedactionMode::Redact);
}

#[test]
fn redaction_config_rejects_unknown_fields() {
    // deny_unknown_fields: a stored config with a typo'd category must
    // fail loudly, never silently fall back for that category.
    assert!(
        serde_json::from_str::<RedactionConfig>(
            r#"{"secrets":"deny","pii":"redact","secrests":"deny"}"#
        )
        .is_err()
    );
}

// ── Composition config ───────────────────────────────────────────────────────

#[test]
fn composition_config_roundtrips_and_defaults_to_the_product_config() {
    json_roundtrip(&CompositionConfig::DEFAULT);
    assert_eq!(
        CompositionConfig::default(),
        CompositionConfig::DEFAULT,
        "an unconfigured pack composes under the product config \
         (ADR-0025 decision 3 — the config only ever narrows)"
    );
    assert_eq!(CompositionConfig::DEFAULT.budget_tokens, 1_500, "seed §4.4");
}

#[test]
fn composition_config_rejects_unknown_fields() {
    // deny_unknown_fields: a stored config with a typo'd key must fail
    // loudly, never silently compose under a default it didn't choose.
    assert!(
        serde_json::from_str::<CompositionConfig>(
            r#"{"budget_tokens":900,"summary_chars":240,"channels":"published-only"}"#
        )
        .is_err()
    );
}

// ── Session event type (CPR-10, ADR-0076; CPR-12, ADR-0078) ─────────────────
//
// `ObserveKind` lived here until the observe cutover. It named four things an
// observe event could report; `SessionEventType` names thirteen things that
// can happen in a run, and it is the vocabulary extraction routes on now.

#[test]
fn session_event_type_all_roundtrip_and_match_as_str() {
    for event_type in SessionEventType::ALL {
        json_roundtrip(event_type);
        let json = serde_json::to_string(event_type).expect("serialize");
        assert_eq!(json, format!("\"{}\"", event_type.as_str()));
        assert_eq!(
            SessionEventType::from_str(event_type.as_str()).unwrap(),
            *event_type
        );
    }
}

#[test]
fn session_event_type_rejects_unknown_names() {
    assert!(serde_json::from_str::<SessionEventType>("\"summary\"").is_err());
    assert!(
        SessionEventType::from_str("message_user").is_err(),
        "dotted, not snake_cased"
    );
    assert!(
        SessionEventType::from_str("MessageUser").is_err(),
        "dotted, not the Rust spelling"
    );
    // The vocabulary that left with the observe plane must not creep back in.
    for gone in ["transcript_delta", "tool_result", "decision", "assertion"] {
        assert!(
            SessionEventType::from_str(gone).is_err(),
            "{gone} was an ObserveKind and is not an event type"
        );
    }
}

/// The wire names are stored values under a CHECK constraint (migration 0044,
/// widened by 0046), so renaming one silently orphans every row already
/// written with the old spelling. Pinned literally, on purpose: this test is
/// meant to fail when someone edits `as_str`.
#[test]
fn session_event_type_wire_names_are_pinned_to_the_stored_vocabulary() {
    assert_eq!(
        SessionEventType::ALL
            .iter()
            .map(|event_type| event_type.as_str())
            .collect::<Vec<_>>(),
        vec![
            "session.started",
            "session.ended",
            "message.user",
            "message.assistant",
            "tool.invoked",
            "tool.result",
            "file.read",
            "file.changed",
            "command.executed",
            "skill.loaded",
            "context.requested",
            "adapter.warning",
            "memory.asserted",
        ]
    );
}

/// ADR-0057 decision 8's whole point, carried across the cutover: exactly one
/// name reports that a model composed the content rather than that a host
/// observed it. If a later variant joins the model-driven side it has to come
/// here deliberately rather than by inheriting a default.
#[test]
fn only_memory_asserted_is_model_composed() {
    assert_eq!(
        SessionEventType::MemoryAsserted.as_str(),
        "memory.asserted",
        "the one name that is a provenance claim rather than an observation"
    );
    assert_eq!(SessionEventType::MemoryAsserted.family(), "memory");
    // Nothing else shares its family, which is what keeps the claim separable.
    let family: Vec<_> = SessionEventType::ALL
        .iter()
        .filter(|event_type| event_type.family() == "memory")
        .collect();
    assert_eq!(family, vec![&SessionEventType::MemoryAsserted]);
}

// ── Roles ────────────────────────────────────────────────────────────────────
// The grant keys are the one vocabulary since CPR-7 (ADR-0074 decision 6);
// the old binding vocabulary's roundtrips left with it.

#[test]
fn role_keys_roundtrip_and_match_as_str() {
    use std::str::FromStr;
    use synveda_types::access::RoleKey;
    for role in RoleKey::ALL {
        json_roundtrip(role);
        let json = serde_json::to_string(role).expect("serialize");
        assert_eq!(json, format!("\"{}\"", role.as_str()));
        assert_eq!(RoleKey::from_str(role.as_str()).unwrap(), *role);
        assert_eq!(role.to_string(), role.as_str());
    }
}

#[test]
fn role_keys_reject_the_old_vocabulary_by_name() {
    use std::str::FromStr;
    use synveda_types::access::RoleKey;
    for old in [
        "steward",
        "org-admin",
        "compliance",
        "security-reviewer",
        "contributor",
    ] {
        assert!(
            RoleKey::from_str(old).is_err(),
            "the binding vocabulary's {old:?} must fail by name"
        );
    }
}

// ── Tenant ───────────────────────────────────────────────────────────────────

#[test]
fn tenant_status_roundtrips_and_matches_as_str() {
    for status in [TenantStatus::Active, TenantStatus::Suspended] {
        json_roundtrip(&status);
        let json = serde_json::to_string(&status).expect("serialize");
        assert_eq!(json, format!("\"{}\"", status.as_str()));
        assert_eq!(TenantStatus::from_str(status.as_str()).unwrap(), status);
        assert_eq!(status.to_string(), status.as_str());
    }
}

#[test]
fn tenant_status_rejects_unknown_values() {
    assert!(serde_json::from_str::<TenantStatus>("\"deleted\"").is_err());
    assert!(TenantStatus::from_str("Active").is_err(), "lowercase only");
}

#[test]
fn tenant_roundtrips() {
    json_roundtrip(&Tenant {
        id: TenantId::new(),
        slug: "acme-bank".into(),
        name: "ACME Bank".into(),
        status: TenantStatus::Active,
        created_at: "2026-07-18T12:00:00Z".parse().expect("timestamp"),
    });
}

// ── Error taxonomy ───────────────────────────────────────────────────────────

fn every_error_variant() -> Vec<Error> {
    vec![
        Error::Unauthenticated {
            message: "token expired".into(),
        },
        Error::PolicyDenied {
            action: "inject".into(),
            resource: "record 0198".into(),
            reason: "regulated-strict/no-cross-team-read".into(),
        },
        Error::NotFound {
            entity: "scope team-billing".into(),
        },
        Error::Invalid {
            message: "token budget must be positive".into(),
        },
        Error::Conflict {
            message: "ref moved since proposal was opened".into(),
        },
        Error::RateLimited {
            message: "inject qps".into(),
        },
        Error::Storage {
            message: "connection pool exhausted".into(),
        },
        Error::Dependency {
            service: "tei".into(),
            message: "embedding timeout".into(),
        },
        Error::Internal {
            message: "scope chain resolved empty".into(),
        },
    ]
}

#[test]
fn error_every_variant_roundtrips() {
    for err in every_error_variant() {
        json_roundtrip(&err);
    }
}

#[test]
fn error_serde_tag_equals_stable_code() {
    for err in every_error_variant() {
        let value = serde_json::to_value(&err).expect("serialize");
        assert_eq!(
            value["kind"],
            err.code(),
            "kind tag and code() diverged for {err:?}"
        );
    }
}

#[test]
fn error_display_is_informative() {
    let err = Error::PolicyDenied {
        action: "recall".into(),
        resource: "record 0198".into(),
        reason: "sensitivity".into(),
    };
    assert_eq!(
        err.to_string(),
        "policy denied recall on record 0198: sensitivity"
    );
}
