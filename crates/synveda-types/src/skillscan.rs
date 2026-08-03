//! Skill security-scan vocabulary (SKIL-2, ADR-0052).
//!
//! The severities a static-analysis finding carries and the one thing a
//! policy pack gets to say about them. The rules themselves live in
//! `synveda-ingest` beside MEM-2's; what is here is what a pack
//! configures and what an audit payload names, which is why it sits in
//! the crate everything depends on.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Error;

/// How bad a skill-scan finding is (ADR-0052 decision 3).
///
/// Variants are ordered least → most severe, so `Ord` picks a bundle's
/// worst finding and compares it against the pack's blocking threshold.
///
/// The bands are not degrees of the same thing — they are three
/// different statements about whether a human could disagree:
///
/// - [`Notice`](ScanSeverity::Notice): ordinary, and named only so a
///   reviewer's eye lands on it.
/// - [`High`](ScanSeverity::High): dangerous and occasionally
///   legitimate, so a pack decides.
/// - [`Critical`](ScanSeverity::Critical): no legitimate reading exists,
///   so nobody decides. This is the band on the invariant floor.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "lowercase")]
pub enum ScanSeverity {
    /// Plain network egress, ordinary subprocess use, an environment
    /// read, a package install. Always reported, never blocking.
    #[default]
    Notice,
    /// Dynamic execution, shell-true invocation, a destructive command,
    /// a privilege change, a write outside the bundle. Blocks under
    /// `regulated-strict`; reported elsewhere.
    High,
    /// Fetch-and-execute, decode-and-execute, a credential location in a
    /// file that also reaches the network, a reverse shell. Blocks under
    /// every pack — [`SkillScanConfig::block_at`] cannot be set here.
    Critical,
}

impl ScanSeverity {
    /// All severities, least to most severe.
    pub const ALL: [ScanSeverity; 3] = [
        ScanSeverity::Notice,
        ScanSeverity::High,
        ScanSeverity::Critical,
    ];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            ScanSeverity::Notice => "notice",
            ScanSeverity::High => "high",
            ScanSeverity::Critical => "critical",
        }
    }
}

impl fmt::Display for ScanSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ScanSeverity {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ScanSeverity::ALL
            .into_iter()
            .find(|severity| severity.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown scan severity: {s:?}"),
            })
    }
}

/// A pack's skill-scan configuration (ADR-0052 decision 9): the one
/// threshold a pack gets to move.
///
/// Rides the loaded pack inside the PDP beside
/// [`RedactionConfig`](crate::RedactionConfig) and resolves with it, so
/// the threshold always comes from exactly the pack that decided the
/// write.
///
/// `Default` is `critical` — the floor and nothing more — which is also
/// what a stored pack that configures nothing gets. That is the fail-safe
/// reading available here: the floor still holds, and a pack that says
/// nothing must not inherit the strict pack's stricter threshold by
/// accident and start refusing bundles its author never asked it to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillScanConfig {
    /// The lowest severity that refuses a bundle. Clamped to
    /// [`ScanSeverity::Critical`] on read by
    /// [`SkillScanConfig::threshold`] — a configuration cannot permit
    /// what the floor refuses.
    pub block_at: ScanSeverity,
}

impl SkillScanConfig {
    /// The strict product config (`regulated-strict`): `high` blocks
    /// too, so a skill that shells out or writes outside its bundle
    /// needs a rule change rather than a reviewer's judgement.
    ///
    /// A bank refusing that skill is a bank behaving as intended; the
    /// same default for a ten-person shop would be a product nobody can
    /// use, which is why the relaxed packs stop at the floor.
    pub const STRICT: SkillScanConfig = SkillScanConfig {
        block_at: ScanSeverity::High,
    };

    /// The relaxed product config (`standard`, `open-collaboration`) and
    /// the fail-safe for an unconfigured stored pack: only the invariant
    /// band refuses, everything else is a reviewer's to weigh.
    pub const FLOOR: SkillScanConfig = SkillScanConfig {
        block_at: ScanSeverity::Critical,
    };

    /// The threshold this config actually enforces.
    ///
    /// **The clamp is the point.** `block_at` is deserialised from a
    /// stored pack's JSON, so nothing stops a tenant writing a value
    /// above `critical` if the type ever grows one — and an API that can
    /// be called the wrong way eventually is (ADR-0032 decision 4's
    /// reasoning, restated for a threshold rather than an approval
    /// matrix). Every read goes through here, so there is no path that
    /// permits what the floor refuses.
    #[must_use]
    pub fn threshold(&self) -> ScanSeverity {
        self.block_at.min(ScanSeverity::Critical)
    }

    /// Whether a bundle whose worst finding is `worst` may be stored or
    /// published under this config. `None` means the bundle was clean.
    #[must_use]
    pub fn blocks(&self, worst: Option<ScanSeverity>) -> bool {
        worst.is_some_and(|worst| worst >= self.threshold())
    }
}

impl Default for SkillScanConfig {
    fn default() -> Self {
        SkillScanConfig::FLOOR
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severities_order_least_to_most_severe() {
        assert!(ScanSeverity::Notice < ScanSeverity::High);
        assert!(ScanSeverity::High < ScanSeverity::Critical);
        // The property every caller relies on: the worst finding in a
        // bundle is the max of its findings.
        let found = [
            ScanSeverity::Notice,
            ScanSeverity::Critical,
            ScanSeverity::High,
        ];
        assert_eq!(found.into_iter().max(), Some(ScanSeverity::Critical));
    }

    #[test]
    fn wire_names_round_trip() {
        for severity in ScanSeverity::ALL {
            assert_eq!(severity.as_str().parse::<ScanSeverity>().unwrap(), severity);
            assert_eq!(
                serde_json::to_value(severity).unwrap(),
                serde_json::json!(severity.as_str())
            );
        }
        assert!("catastrophic".parse::<ScanSeverity>().is_err());
    }

    #[test]
    fn the_floor_is_the_default_and_critical_always_blocks() {
        assert_eq!(SkillScanConfig::default(), SkillScanConfig::FLOOR);
        for config in [SkillScanConfig::STRICT, SkillScanConfig::FLOOR] {
            assert!(config.blocks(Some(ScanSeverity::Critical)));
            assert!(!config.blocks(None));
        }
    }

    #[test]
    fn strict_blocks_high_and_the_floor_reports_it() {
        assert!(SkillScanConfig::STRICT.blocks(Some(ScanSeverity::High)));
        assert!(!SkillScanConfig::FLOOR.blocks(Some(ScanSeverity::High)));
        // Notice never blocks, under either.
        assert!(!SkillScanConfig::STRICT.blocks(Some(ScanSeverity::Notice)));
        assert!(!SkillScanConfig::FLOOR.blocks(Some(ScanSeverity::Notice)));
    }

    #[test]
    fn a_config_cannot_permit_what_the_floor_refuses() {
        // The clamp, exercised through the only door: a pack asking for
        // the loosest threshold the type can express still blocks
        // `critical`.
        let loosest = SkillScanConfig {
            block_at: ScanSeverity::Critical,
        };
        assert_eq!(loosest.threshold(), ScanSeverity::Critical);
        assert!(loosest.blocks(Some(ScanSeverity::Critical)));
    }

    #[test]
    fn unknown_fields_are_refused() {
        assert!(serde_json::from_str::<SkillScanConfig>(r#"{"block_at":"high"}"#).is_ok());
        assert!(
            serde_json::from_str::<SkillScanConfig>(r#"{"block_at":"high","allow":["x"]}"#)
                .is_err()
        );
    }
}
