//! Proposal vocabulary (tech plan §2.3, FLOW-3, ADR-0032).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{Channel, Error};

/// A proposal's stored lifecycle — only what *happened* (ADR-0032
/// decision 11).
///
/// `approved` is deliberately absent: whether an open proposal has
/// enough approvals is computed live from its recorded approvals against
/// the live requirement, because requirements resolve live (a pack switch
/// governs the very next request, ADR-0014 decision 3). A stored
/// `approved` would be a second answer that a lowered requirement could
/// contradict, and keeping it true would need a background re-evaluator.
/// [`ProposalView`] is what the API renders, and it has the fifth state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProposalState {
    /// Under review. The only state approvals may be cast in.
    Open,
    /// Closed by a reviewer, with a reason. Terminal.
    Rejected,
    /// Closed by its proposer. Terminal.
    Withdrawn,
    /// Its effect ran: the target channel moved. Terminal.
    Published,
}

impl ProposalState {
    /// Every stored state.
    pub const ALL: [ProposalState; 4] = [
        ProposalState::Open,
        ProposalState::Rejected,
        ProposalState::Withdrawn,
        ProposalState::Published,
    ];

    /// Stable wire name, identical to the serde form and the stored
    /// column (whose CHECK constraint mirrors this list).
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            ProposalState::Open => "open",
            ProposalState::Rejected => "rejected",
            ProposalState::Withdrawn => "withdrawn",
            ProposalState::Published => "published",
        }
    }

    /// Whether the proposal is closed — nothing further may act on it.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        !matches!(self, ProposalState::Open)
    }
}

impl fmt::Display for ProposalState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ProposalState {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ProposalState::ALL
            .into_iter()
            .find(|state| state.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown proposal state: {s:?}"),
            })
    }
}

/// The five-state vocabulary tech plan §2.3 describes, as the API renders
/// it: the stored state, with `approved` rendered from `open` plus a
/// satisfied requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProposalView {
    /// Open, and still short of its requirement.
    Open,
    /// Open, and the requirement is met — ready for its effect to run
    /// under `ChannelPublish` (ADR-0032 decision 9).
    Approved,
    /// Closed by a reviewer.
    Rejected,
    /// Closed by its proposer.
    Withdrawn,
    /// Its effect ran.
    Published,
}

impl ProposalView {
    /// How `state` renders given whether its requirement is satisfied.
    #[must_use]
    pub const fn of(state: ProposalState, satisfied: bool) -> Self {
        match state {
            ProposalState::Open if satisfied => ProposalView::Approved,
            ProposalState::Open => ProposalView::Open,
            ProposalState::Rejected => ProposalView::Rejected,
            ProposalState::Withdrawn => ProposalView::Withdrawn,
            ProposalState::Published => ProposalView::Published,
        }
    }

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            ProposalView::Open => "open",
            ProposalView::Approved => "approved",
            ProposalView::Rejected => "rejected",
            ProposalView::Withdrawn => "withdrawn",
            ProposalView::Published => "published",
        }
    }
}

impl fmt::Display for ProposalView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What running a proposal's effect would do.
///
/// Until AUTHZ-4 a proposal had exactly one possible effect — publish its
/// members onto the target scope's published channel — and the column
/// holding this was called `target_channel` with a `= 'published'` check.
/// A lapse has no target channel at all: its effect is a grant row
/// (ADR-0037 decision 16). So the column names the *effect*, and this is
/// its vocabulary.
///
/// [`ProposalEffect::Lapse`] is deliberately **not** a [`Channel`] variant.
/// No scope has a `policy/lapse` ref, nothing writes one, and a channel
/// that cannot express withdrawal cannot express expiry either — the same
/// fact that kept `staged` unwritten (ADR-0032 decision 2).
///
/// There is no `Default`: what a proposal would *do* is the first thing a
/// reviewer needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProposalEffect {
    /// Publish the members onto the target scope's published channel
    /// (FLOW-3). The only effect there was until AUTHZ-4.
    Published,
    /// Open a time-boxed grant over the target scope's material
    /// (AUTHZ-4, ADR-0037). Always an [`crate::AssetKind::Policy`]
    /// proposal whose one member is the lapse's reviewed terms.
    Lapse,
}

impl ProposalEffect {
    /// Every effect.
    pub const ALL: [ProposalEffect; 2] = [ProposalEffect::Published, ProposalEffect::Lapse];

    /// Stable wire name, identical to the serde form and to the stored
    /// column (whose CHECK constraint mirrors this list).
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            ProposalEffect::Published => "published",
            ProposalEffect::Lapse => "lapse",
        }
    }

    /// The channel this effect writes, when it writes one.
    ///
    /// `None` for a lapse, which is the honest answer rather than a
    /// stand-in: its effect is a row, and a caller that needs a channel
    /// here has taken a wrong turn.
    #[must_use]
    pub const fn channel(&self) -> Option<Channel> {
        match self {
            ProposalEffect::Published => Some(Channel::Published),
            ProposalEffect::Lapse => None,
        }
    }
}

impl fmt::Display for ProposalEffect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ProposalEffect {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ProposalEffect::ALL
            .into_iter()
            .find(|effect| effect.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown proposal effect: {s:?}"),
            })
    }
}

/// A reviewer's verdict on a proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// Counts toward the requirement.
    Approve,
    /// Closes the proposal, with a reason.
    Reject,
}

impl Verdict {
    /// Both verdicts.
    pub const ALL: [Verdict; 2] = [Verdict::Approve, Verdict::Reject];

    /// Stable wire name, identical to the serde form and the stored
    /// column.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Verdict::Approve => "approve",
            Verdict::Reject => "reject",
        }
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Verdict {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Verdict::ALL
            .into_iter()
            .find(|verdict| verdict.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown verdict: {s:?}"),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_names_round_trip() {
        for state in ProposalState::ALL {
            assert_eq!(state.to_string().parse::<ProposalState>().unwrap(), state);
            assert_eq!(
                serde_json::to_string(&state).unwrap(),
                format!("\"{}\"", state.as_str())
            );
        }
        for verdict in Verdict::ALL {
            assert_eq!(verdict.to_string().parse::<Verdict>().unwrap(), verdict);
        }
    }

    /// The stored vocabulary has four states; the rendered one has five,
    /// and `approved` is the only one that is computed.
    #[test]
    fn approved_is_a_rendering_of_open_plus_a_satisfied_requirement() {
        assert_eq!(
            ProposalView::of(ProposalState::Open, true),
            ProposalView::Approved
        );
        assert_eq!(
            ProposalView::of(ProposalState::Open, false),
            ProposalView::Open
        );
        // A closed proposal renders as itself whatever the requirement
        // now says: satisfaction cannot reopen a withdrawal.
        for state in [
            ProposalState::Rejected,
            ProposalState::Withdrawn,
            ProposalState::Published,
        ] {
            for satisfied in [true, false] {
                assert_eq!(ProposalView::of(state, satisfied).as_str(), state.as_str());
            }
        }
    }

    #[test]
    fn only_open_admits_further_acts() {
        assert!(!ProposalState::Open.is_terminal());
        assert!(ProposalState::Rejected.is_terminal());
        assert!(ProposalState::Withdrawn.is_terminal());
        assert!(ProposalState::Published.is_terminal());
    }

    #[test]
    fn an_effect_round_trips_and_only_publication_names_a_channel() {
        for effect in ProposalEffect::ALL {
            assert_eq!(
                effect.to_string().parse::<ProposalEffect>().unwrap(),
                effect
            );
            assert_eq!(
                serde_json::to_string(&effect).unwrap(),
                format!("\"{}\"", effect.as_str())
            );
        }
        assert_eq!(
            ProposalEffect::Published.channel(),
            Some(Channel::Published)
        );
        assert_eq!(
            ProposalEffect::Lapse.channel(),
            None,
            "a lapse writes a row, not a channel; standing in for one would be a lie"
        );
        assert!("derived".parse::<ProposalEffect>().is_err());
        assert!("staged".parse::<ProposalEffect>().is_err());
    }

    #[test]
    fn unknown_states_are_invalid_not_defaulted() {
        assert!(matches!(
            "approved".parse::<ProposalState>(),
            Err(Error::Invalid { .. })
        ));
    }
}
