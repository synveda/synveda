//! Authored-context composition vocabulary (seed §4.4, CTX-2, ADR-0025).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{Error, TraceRetentionMode};

/// Whether an inject block names the skills this identity may install
/// (SKIL-4, ADR-0054 decision 11).
///
/// An advertisement is new content: a skill has no body in a block, so the
/// switch that turns it off remains useful for a client already carrying its
/// own permitted-skill catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SkillIndex {
    /// No skills section. A block is byte-identical to what it was before
    /// SKIL-4 — the MEM-5/MEM-6/CTX-4 discipline, and a test rather than a
    /// claim.
    Off,
    /// The block names every skill the caller may install, nearest scope
    /// first, at the pack's own [`CompositionConfig::summary_chars`].
    /// The product default in every embedded pack.
    #[default]
    Names,
}

impl SkillIndex {
    /// All skill-index modes.
    pub const ALL: [SkillIndex; 2] = [SkillIndex::Off, SkillIndex::Names];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            SkillIndex::Off => "off",
            SkillIndex::Names => "names",
        }
    }

    /// Whether a scope's published skills are named in the block.
    #[must_use]
    pub const fn advertises(&self) -> bool {
        matches!(self, SkillIndex::Names)
    }
}

impl fmt::Display for SkillIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SkillIndex {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        SkillIndex::ALL
            .into_iter()
            .find(|index| index.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown skill index: {s:?}"),
            })
    }
}

/// How much of an authored context chunk a composed entry carried.
///
/// On the entry, in the inject response, and in the `context.injected`
/// payload: "was that agent given the payments runbook, or only told it
/// exists" is a question an auditor asks, and it is answered by reading
/// the chain rather than by re-deriving rendered widths from a changed pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum EntryTier {
    /// The authored chunk's full content composed.
    #[default]
    Body,
    /// A compact summary composed because the full chunk did not fit.
    Summary,
}

impl EntryTier {
    /// Both tiers.
    pub const ALL: [EntryTier; 2] = [EntryTier::Body, EntryTier::Summary];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            EntryTier::Body => "body",
            EntryTier::Summary => "summary",
        }
    }
}

impl fmt::Display for EntryTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The compact summary width in characters.
///
/// 320 is the feature's "skills-style ~80 tokens each" through the
/// ADR-0025 decision 4 estimator, which is `ceil(chars / 4)`.
pub const DEFAULT_SUMMARY_CHARS: u32 = 320;

const fn default_summary_chars() -> u32 {
    DEFAULT_SUMMARY_CHARS
}

/// A pack's composition configuration (ADR-0025 decisions 2–3; CTX-4's
/// index tier, ADR-0041 decision 11). Rides the loaded pack beside
/// [`crate::RedactionConfig`] and resolves with it. The budget applies per
/// context composition under the caller's home-scope pack.
///
/// This config never grants access. The PDP admits authored chunks and
/// skills before composition; this config only controls rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionConfig {
    /// Estimated-token budget per composed block (seed §4.4: default
    /// 1,500).
    pub budget_tokens: u32,
    /// The compact-summary width in characters
    /// ([`DEFAULT_SUMMARY_CHARS`]) — the width a skill's advertised
    /// description is elided at too, so a pack that narrowed one narrowed
    /// both (SKIL-4, ADR-0054 decision 11).
    #[serde(default = "default_summary_chars")]
    pub summary_chars: u32,
    /// Whether this scope's published skills are named in a block
    /// (SKIL-4, ADR-0054 decision 11).
    ///
    #[serde(default)]
    pub skill_index: SkillIndex,
    /// How much planner detail is retained for context inspection
    /// (CPR-20, ADR-0084). This never grants a read; it only removes detail
    /// from an already-authorised trace.
    #[serde(default, skip_serializing_if = "TraceRetentionMode::is_full")]
    pub trace_retention: TraceRetentionMode,
}

impl CompositionConfig {
    /// The product config: seed §4.4's budget, compact summaries, and
    /// permitted skills named.
    pub const DEFAULT: CompositionConfig = CompositionConfig {
        budget_tokens: 1_500,
        summary_chars: DEFAULT_SUMMARY_CHARS,
        skill_index: SkillIndex::Names,
        trace_retention: TraceRetentionMode::Full,
    };
}

impl Default for CompositionConfig {
    fn default() -> Self {
        CompositionConfig::DEFAULT
    }
}
