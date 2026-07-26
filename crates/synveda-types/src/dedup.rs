//! Dedup & conflict-detection vocabulary (MEM-5, ADR-0039).
//!
//! Thresholds are integers in per-mille rather than floats, for the reason
//! ADR-0039 decision 13 gives about the audit payload and migration 0024
//! gives about the edge row: a similarity that jsonb, a client, or a
//! serde round-trip may reshape is a similarity that cannot be compared
//! later. It also keeps every pack config `Eq` and `Hash`, which
//! [`crate::PackConfig`] and the PDP's effective-pack type both need.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Error;

/// How much of the feature runs at scopes a pack governs (ADR-0039
/// decision 12).
///
/// Ordered least → most active, so a comparison reads as "does this mode
/// do at least this much".
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "lowercase")]
pub enum DedupMode {
    /// Nothing runs: every candidate inserts, as the pipeline behaved
    /// before MEM-5.
    Off,
    /// Near-duplicates merge into the record they restate; contradictions
    /// insert beside what they contradict.
    Merge,
    /// Merging, plus contradiction detection: a newer statement closes the
    /// valid window of the record it replaces and an edge records it. The
    /// product default — seed §4.4 already orders conflicts by "newer
    /// valid-time beats older", and a pack that hoarded contradictions
    /// would be the surprising one.
    #[default]
    Supersede,
}

impl DedupMode {
    /// All modes, least to most active.
    pub const ALL: [DedupMode; 3] = [DedupMode::Off, DedupMode::Merge, DedupMode::Supersede];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            DedupMode::Off => "off",
            DedupMode::Merge => "merge",
            DedupMode::Supersede => "supersede",
        }
    }

    /// Whether near-duplicates merge under this mode.
    #[must_use]
    pub const fn merges(&self) -> bool {
        matches!(self, DedupMode::Merge | DedupMode::Supersede)
    }

    /// Whether contradictions close windows under this mode.
    #[must_use]
    pub const fn supersedes(&self) -> bool {
        matches!(self, DedupMode::Supersede)
    }
}

impl fmt::Display for DedupMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DedupMode {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        DedupMode::ALL
            .into_iter()
            .find(|mode| mode.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown dedup mode: {s:?}"),
            })
    }
}

/// The largest nomination depth a pack may ask for. The nominator runs
/// inside the write transaction against the pipeline-lag SLO (seed §10),
/// so the depth is bounded by the product rather than by a tenant's
/// optimism.
pub const MAX_DEDUP_NEIGHBOURS: u16 = 64;

/// A pack's dedup configuration (ADR-0039 decision 12). Rides the loaded
/// pack beside [`crate::RedactionConfig`] and [`crate::CompositionConfig`]
/// and resolves the same way: the worker asks for the effective pack at
/// the owner's home scope and gets the configuration that governed the
/// write it is about to make.
///
/// This config never grants access and never widens anything — it decides
/// only whether the pipeline collapses restatements and closes the windows
/// of contradicted records. That is why [`Default`] (the product config)
/// is also the fail-safe for a stored pack configuring nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DedupConfig {
    /// How much of the feature runs.
    pub mode: DedupMode,
    /// Jaccard, per mille, at or above which two contents the judge did
    /// not call a contradiction are the same statement restated.
    pub near_dup_jaccard_permille: u16,
    /// Cosine similarity, per mille, at or above which the same holds on
    /// the semantic leg. Only as meaningful as the embedder in use: the
    /// deterministic hash embedder reaches it exactly when the texts are
    /// identical (ADR-0023 decision 6).
    pub near_dup_cosine_permille: u16,
    /// The overlap coefficient, per mille, the two contents' frames must
    /// reach before the judge will call a pair a contradiction (ADR-0039
    /// decision 5).
    pub conflict_frame_overlap_permille: u16,
    /// How many neighbours the semantic leg nominates per candidate, and
    /// the cap on what either leg contributes.
    pub neighbours: u16,
}

impl DedupConfig {
    /// The product config: supersession on, tuned to refuse.
    ///
    /// The numbers are ADR-0039 decisions 4 and 5. `0.90` Jaccard and
    /// `0.97` cosine are "this is the same statement, restated"; `0.70`
    /// frame overlap is "these two sentences are about the same thing",
    /// and it is the one that decides whether a fact stops composing, so
    /// it sits high enough that "deploys go through make deploy" and
    /// "tests go through make test" (overlap 0.50) fall the safe side of
    /// it.
    pub const DEFAULT: DedupConfig = DedupConfig {
        mode: DedupMode::Supersede,
        near_dup_jaccard_permille: 900,
        near_dup_cosine_permille: 970,
        conflict_frame_overlap_permille: 700,
        neighbours: 16,
    };

    /// Refuses a config that could never do anything sensible: a
    /// similarity outside `0..=1`, or a nomination depth of zero or one
    /// past the product bound. Called at apply time, and again when a
    /// stored row is read (an out-of-band write is the only way to reach
    /// the second).
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] naming the field that is out of range.
    pub fn validate(&self) -> Result<(), Error> {
        let bounded = |label: &str, value: u16| -> Result<(), Error> {
            if value > 1000 {
                return Err(Error::Invalid {
                    message: format!("{label} is {value} per mille; the maximum is 1000"),
                });
            }
            Ok(())
        };
        bounded("near_dup_jaccard_permille", self.near_dup_jaccard_permille)?;
        bounded("near_dup_cosine_permille", self.near_dup_cosine_permille)?;
        bounded(
            "conflict_frame_overlap_permille",
            self.conflict_frame_overlap_permille,
        )?;
        if self.neighbours == 0 || self.neighbours > MAX_DEDUP_NEIGHBOURS {
            return Err(Error::Invalid {
                message: format!(
                    "neighbours is {}; it must be between 1 and {MAX_DEDUP_NEIGHBOURS}",
                    self.neighbours
                ),
            });
        }
        Ok(())
    }

    /// The near-duplicate Jaccard threshold as a ratio.
    #[must_use]
    pub fn near_dup_jaccard(&self) -> f64 {
        f64::from(self.near_dup_jaccard_permille) / 1000.0
    }

    /// The near-duplicate cosine threshold as a ratio.
    #[must_use]
    pub fn near_dup_cosine(&self) -> f64 {
        f64::from(self.near_dup_cosine_permille) / 1000.0
    }

    /// The conflict frame-overlap threshold as a ratio.
    #[must_use]
    pub fn conflict_frame_overlap(&self) -> f64 {
        f64::from(self.conflict_frame_overlap_permille) / 1000.0
    }
}

impl Default for DedupConfig {
    fn default() -> Self {
        DedupConfig::DEFAULT
    }
}

/// A similarity as it is stored and audited: per mille, rounded, clamped
/// to the range the column's CHECK constraint allows. Cosine can be
/// negative, which is why the floor is `-1000` rather than zero.
#[must_use]
pub fn permille(value: f64) -> i32 {
    if value.is_nan() {
        return 0;
    }
    (value * 1000.0).round().clamp(-1000.0, 1000.0) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_product_config_is_valid_and_supersedes() {
        let config = DedupConfig::DEFAULT;
        config.validate().expect("the product config is valid");
        assert_eq!(config, DedupConfig::default());
        assert!(config.mode.supersedes());
        assert!(config.mode.merges(), "supersede implies merge");
        assert!(!DedupMode::Merge.supersedes());
        assert!(!DedupMode::Off.merges());
        assert!(DedupMode::Off < DedupMode::Merge && DedupMode::Merge < DedupMode::Supersede);
    }

    #[test]
    fn modes_round_trip_through_their_wire_names() {
        for mode in DedupMode::ALL {
            assert_eq!(mode.as_str().parse::<DedupMode>().expect("parses"), mode);
        }
        assert!("collapse".parse::<DedupMode>().is_err());
    }

    #[test]
    fn a_config_outside_its_ranges_is_refused_by_field_name() {
        let over = DedupConfig {
            near_dup_jaccard_permille: 1001,
            ..DedupConfig::DEFAULT
        };
        let err = over.validate().expect_err("1001 per mille is not a ratio");
        assert!(err.to_string().contains("near_dup_jaccard_permille"));

        let none = DedupConfig {
            neighbours: 0,
            ..DedupConfig::DEFAULT
        };
        let err = none
            .validate()
            .expect_err("a nominator that fetches nothing nominates nothing");
        assert!(err.to_string().contains("neighbours"));

        let greedy = DedupConfig {
            neighbours: MAX_DEDUP_NEIGHBOURS + 1,
            ..DedupConfig::DEFAULT
        };
        assert!(greedy.validate().is_err(), "the depth is product-bounded");
    }

    /// An unknown field is a typo'd threshold, and a silently-ignored
    /// threshold is a pack that does not do what its author wrote.
    #[test]
    fn an_unknown_field_is_refused_rather_than_ignored() {
        let json = r#"{"mode":"supersede","near_dup_jaccard_permille":900,
            "near_dup_cosine_permille":970,"conflict_frame_overlap_permille":700,
            "neighbours":16,"near_dup_jacard_permille":500}"#;
        assert!(serde_json::from_str::<DedupConfig>(json).is_err());
    }

    #[test]
    fn permille_rounds_clamps_and_survives_nan() {
        assert_eq!(permille(0.6154), 615);
        assert_eq!(permille(1.0), 1000);
        assert_eq!(permille(-1.0), -1000);
        // Float error around the ends must not produce a value the CHECK
        // constraint refuses.
        assert_eq!(permille(1.000_000_2), 1000);
        assert_eq!(permille(-1.000_000_2), -1000);
        assert_eq!(permille(f64::NAN), 0);
    }
}
