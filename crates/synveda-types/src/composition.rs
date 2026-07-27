//! Composition vocabulary (seed §4.4, CTX-2, ADR-0025).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Error;

/// Which channels compose into an inject block under a pack (CTX-2,
/// ADR-0025 decision 2). Since FLOW-2 the channels are real: a record
/// composes as published when its scope's `memory/published` tree names
/// it at exactly the content it holds, and as derived otherwise
/// ([`crate::Channel`], ADR-0031 decisions 4 and 5). This switch — the
/// "bank mode" — kept its meaning across that change; what changed is
/// that `published` now requires a publication rather than standing in
/// for one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum InjectChannels {
    /// Published material plus policy-permitted derived — the product
    /// default in every embedded pack (derived is readable per policy,
    /// clearly marked unreviewed).
    #[default]
    PublishedAndDerived,
    /// Published material only; derived never composes at scopes this
    /// pack governs. The bank-mode switch (tech plan §2.2).
    PublishedOnly,
}

impl InjectChannels {
    /// All channel rules.
    pub const ALL: [InjectChannels; 2] = [
        InjectChannels::PublishedAndDerived,
        InjectChannels::PublishedOnly,
    ];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            InjectChannels::PublishedAndDerived => "published-and-derived",
            InjectChannels::PublishedOnly => "published-only",
        }
    }

    /// Whether derived-channel material composes under this rule.
    #[must_use]
    pub const fn includes_derived(&self) -> bool {
        matches!(self, InjectChannels::PublishedAndDerived)
    }
}

impl fmt::Display for InjectChannels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for InjectChannels {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        InjectChannels::ALL
            .into_iter()
            .find(|channels| channels.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown inject channels: {s:?}"),
            })
    }
}

/// Whether a candidate that does not fit the remaining budget is named
/// instead of dropped (CTX-4, ADR-0041 decision 11).
///
/// Two-valued for the reason ADR-0040 decision 13 gives about retention
/// modes: a knob whose settings nobody can enumerate is a knob nobody can
/// audit. Unlike a retention horizon, neither setting hides or destroys
/// anything — `demote` only ever converts an omission the previous product
/// made in silence into one the block names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum IndexTier {
    /// A candidate that does not fit is dropped, counted in
    /// `skipped_over_budget`, and never mentioned — composition exactly as
    /// it behaved before CTX-4, byte for byte.
    Off,
    /// A candidate that does not fit is offered its index line, and takes
    /// it when that line is strictly cheaper than its body (ADR-0041
    /// decision 2). The product default in every embedded pack.
    #[default]
    Demote,
}

impl IndexTier {
    /// All index-tier modes.
    pub const ALL: [IndexTier; 2] = [IndexTier::Off, IndexTier::Demote];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            IndexTier::Off => "off",
            IndexTier::Demote => "demote",
        }
    }

    /// Whether a candidate may be named rather than dropped.
    #[must_use]
    pub const fn demotes(&self) -> bool {
        matches!(self, IndexTier::Demote)
    }
}

impl fmt::Display for IndexTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for IndexTier {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        IndexTier::ALL
            .into_iter()
            .find(|tier| tier.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown index tier: {s:?}"),
            })
    }
}

/// How much of a record a composed entry carried (CTX-4, ADR-0041
/// decision 9).
///
/// On the entry, in the inject response, and in the `context.injected`
/// payload: "was that agent given the payments runbook, or only told it
/// exists" is a question an auditor asks, and it is answered by reading
/// the chain rather than by re-deriving rendered widths from a corpus that
/// has since moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum EntryTier {
    /// The record's full content composed.
    #[default]
    Body,
    /// Only its elided head and its handle composed: the block disclosed
    /// that the record exists, and the reader fetches the rest through
    /// `POST /v1/recall` (ADR-0041 decision 5).
    Index,
}

impl EntryTier {
    /// Both tiers.
    pub const ALL: [EntryTier; 2] = [EntryTier::Body, EntryTier::Index];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            EntryTier::Body => "body",
            EntryTier::Index => "index",
        }
    }
}

impl fmt::Display for EntryTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The index line's content width in characters, before the class marker,
/// the trust markers and the handle (ADR-0041 decision 3).
///
/// 320 is the feature's "skills-style ~80 tokens each" through the
/// ADR-0025 decision 4 estimator, which is `ceil(chars / 4)`.
pub const DEFAULT_INDEX_ENTRY_CHARS: u32 = 320;

const fn default_index_entry_chars() -> u32 {
    DEFAULT_INDEX_ENTRY_CHARS
}

/// A pack's composition configuration (ADR-0025 decisions 2–3; CTX-4's
/// index tier, ADR-0041 decision 11). Rides the loaded pack beside
/// [`crate::RedactionConfig`] and resolves with it: the channel rule and
/// the index tier apply per candidate scope under that scope's effective
/// pack; the budget applies per inject under the caller's home-scope pack.
///
/// This config never grants access — `MemoryRead` is decided per scope
/// before composition — it only narrows which channel of readable
/// material composes and how much. The index tier does not even do that:
/// it changes how much of an already-admitted record is rendered, never
/// which records are admissible. That is why `Default` (the product
/// config) is also the fail-safe for stored packs configuring nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionConfig {
    /// Estimated-token budget per composed block (seed §4.4: default
    /// 1,500).
    pub budget_tokens: u32,
    /// Which channels compose at scopes this pack governs.
    pub channels: InjectChannels,
    /// Whether material that does not fit is named rather than dropped.
    ///
    /// `#[serde(default)]` because stored packs configured before CTX-4
    /// carry a `composition` object without this key, and a pack that
    /// predates a feature must keep resolving to the product config
    /// rather than failing to load (the ADR-0021 fail-safe).
    #[serde(default)]
    pub index_tier: IndexTier,
    /// The index line's content width in characters
    /// ([`DEFAULT_INDEX_ENTRY_CHARS`]).
    #[serde(default = "default_index_entry_chars")]
    pub index_entry_chars: u32,
}

impl CompositionConfig {
    /// The product config: the seed §4.4 default budget, both channels,
    /// and the index tier on.
    pub const DEFAULT: CompositionConfig = CompositionConfig {
        budget_tokens: 1_500,
        channels: InjectChannels::PublishedAndDerived,
        index_tier: IndexTier::Demote,
        index_entry_chars: DEFAULT_INDEX_ENTRY_CHARS,
    };
}

impl Default for CompositionConfig {
    fn default() -> Self {
        CompositionConfig::DEFAULT
    }
}
