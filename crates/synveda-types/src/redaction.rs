//! Redaction & secret-scanning vocabulary (seed §6, MEM-2, ADR-0021).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Error;

/// What happens to an observe event when scanning finds something
/// (seed §6): the modes a policy pack configures per finding category.
///
/// Variants are ordered least → most strict, so `Ord` comparisons pick
/// the disposition when one event triggers categories with different
/// modes (ADR-0021 decision 4: the strictest triggered mode wins).
/// Redaction of the matched spans is unconditional in every mode — the
/// mode decides flow, never whether the finding's text survives.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "lowercase")]
pub enum RedactionMode {
    /// The event is admitted with matched spans redacted.
    #[default]
    Redact,
    /// The event stages redacted but sends no work signal until a
    /// reviewer releases it (the review queue, ADR-0021 decision 5).
    Quarantine,
    /// The event is refused per event; nothing persists for it.
    Deny,
}

impl RedactionMode {
    /// All modes, least to most strict.
    pub const ALL: [RedactionMode; 3] = [
        RedactionMode::Redact,
        RedactionMode::Quarantine,
        RedactionMode::Deny,
    ];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            RedactionMode::Redact => "redact",
            RedactionMode::Quarantine => "quarantine",
            RedactionMode::Deny => "deny",
        }
    }
}

impl fmt::Display for RedactionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RedactionMode {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RedactionMode::ALL
            .into_iter()
            .find(|mode| mode.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown redaction mode: {s:?}"),
            })
    }
}

/// A pack's redaction configuration (ADR-0021 decision 3): one mode per
/// finding category. Rides the loaded pack inside the PDP and resolves
/// with it, so the mode always comes from exactly the pack that decided
/// the write.
///
/// `Default` is the strict product config — secrets quarantine, PII
/// redacts — which is also what a stored pack without an explicit
/// config gets (fail safe).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedactionConfig {
    /// Mode for secret findings (credentials, keys, tokens).
    pub secrets: RedactionMode,
    /// Mode for PII findings (emails, card numbers, identifiers).
    pub pii: RedactionMode,
}

impl RedactionConfig {
    /// The strict product config (`regulated-strict`, ADR-0021
    /// decision 3): secrets quarantine for review, PII redacts on
    /// ingest — the seed §6 wording. Also the fail-safe default for
    /// stored packs that configure nothing.
    pub const STRICT: RedactionConfig = RedactionConfig {
        secrets: RedactionMode::Quarantine,
        pii: RedactionMode::Redact,
    };

    /// The relaxed product config (`standard`, `open-collaboration`):
    /// everything redacts; nothing is held or refused.
    pub const REDACT_ALL: RedactionConfig = RedactionConfig {
        secrets: RedactionMode::Redact,
        pii: RedactionMode::Redact,
    };
}

impl Default for RedactionConfig {
    fn default() -> Self {
        RedactionConfig::STRICT
    }
}
