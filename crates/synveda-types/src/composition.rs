//! Composition vocabulary (seed §4.4, CTX-2, ADR-0025).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Error;

/// Which channels compose into an inject block under a pack (CTX-2,
/// ADR-0025 decision 2). Pre-FLOW-2, [`crate::RecordKind`] is the
/// channel stand-in: `pinned` is the published channel (authored,
/// canonical), `derived` the derived channel (unreviewed pipeline
/// output). FLOW-2's channel refs replace the stand-in; this switch —
/// the "bank mode" — keeps its meaning unchanged.
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

/// A pack's composition configuration (ADR-0025 decisions 2–3). Rides
/// the loaded pack beside [`crate::RedactionConfig`] and resolves with
/// it: the channel rule applies per candidate scope under that scope's
/// effective pack; the budget applies per inject under the caller's
/// home-scope pack.
///
/// This config never grants access — `MemoryRead` is decided per scope
/// before composition — it only narrows which channel of readable
/// material composes and how much. That is why `Default` (the product
/// config) is also the fail-safe for stored packs configuring nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionConfig {
    /// Estimated-token budget per composed block (seed §4.4: default
    /// 1,500).
    pub budget_tokens: u32,
    /// Which channels compose at scopes this pack governs.
    pub channels: InjectChannels,
}

impl CompositionConfig {
    /// The product config: the seed §4.4 default budget, both channels.
    pub const DEFAULT: CompositionConfig = CompositionConfig {
        budget_tokens: 1_500,
        channels: InjectChannels::PublishedAndDerived,
    };
}

impl Default for CompositionConfig {
    fn default() -> Self {
        CompositionConfig::DEFAULT
    }
}
