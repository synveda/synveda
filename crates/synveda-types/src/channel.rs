//! The VedaFlow channel vocabulary (tech plan §2.2; FLOW-2, ADR-0031).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Error;

/// One of the three standing channels every scope has per asset type.
///
/// A channel is a `vedaflow_refs` row named `{asset-kind}/{channel}`
/// (ADR-0031 decision 1); this enum is the second half of that name. The
/// refs materialise on first write — a scope with nothing published has no
/// `memory/published` ref, and that is the true answer rather than a gap
/// (decision 2).
///
/// There is no `Default`: which channel content is on is the whole question
/// this feature exists to answer, and a default would answer it silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Channel {
    /// Auto-committed by the ingestion pipeline: extracted memories land
    /// here continuously, readable per policy and always rendered
    /// unreviewed. Its commits are a log of what each batch added — the
    /// channel's membership is the complement of `published`, never an
    /// enumeration (ADR-0031 decisions 3 and 4).
    Derived,
    /// Proposals under review (FLOW-3). Nothing writes it yet; it composes
    /// into nothing, because material under review is not material anyone
    /// stands behind.
    Staged,
    /// The trust boundary. `inject` composes from here always, and from
    /// `derived` only where the scope's pack allows it
    /// ([`crate::InjectChannels`]). Each commit's tree is the channel's
    /// entire membership, keyed by record id and bound to the exact
    /// content that was reviewed (ADR-0031 decisions 3 and 5).
    Published,
}

impl Channel {
    /// All channels, in lifecycle order (tech plan §2.3).
    pub const ALL: [Channel; 3] = [Channel::Derived, Channel::Staged, Channel::Published];

    /// Stable wire name, identical to the serde form and to the half of a
    /// ref name this enum owns. Renaming one would orphan every ref
    /// carrying the old spelling.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Channel::Derived => "derived",
            Channel::Staged => "staged",
            Channel::Published => "published",
        }
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Channel {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Channel::ALL
            .into_iter()
            .find(|channel| channel.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown channel: {s:?}"),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_names_round_trip_through_display_and_parse() {
        for channel in Channel::ALL {
            assert_eq!(channel.to_string().parse::<Channel>().unwrap(), channel);
            assert_eq!(
                serde_json::to_string(&channel).unwrap(),
                format!("\"{}\"", channel.as_str())
            );
        }
    }

    #[test]
    fn unknown_names_are_invalid_not_defaulted() {
        assert!(matches!(
            "review".parse::<Channel>(),
            Err(Error::Invalid { .. })
        ));
    }
}
