//! Proposal vocabulary (tech plan §2.3, FLOW-3, ADR-0032).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{Channel, Error, Result};

/// The stable artifact family a common VedaFlow proposal refers to
/// (CPR-32, ADR-0091).
///
/// This is deliberately more precise than [`crate::AssetKind`]: one Tool
/// asset may change a server version or an exact project binding, and an OKF
/// import publishes through Knowledge without becoming a second Knowledge
/// aggregate. The family says what stable domain address a reviewer is
/// looking at; it never grants authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactFamily {
    /// Stable Knowledge aggregate.
    Knowledge,
    /// Stable Agent Skill aggregate or binding.
    Skill,
    /// Stable trusted MCP server catalogue entry.
    ToolServer,
    /// Exact project-to-Tool-version binding.
    ToolBinding,
    /// Stable governed runtime Configuration aggregate or binding.
    Configuration,
    /// Stable time-boxed policy-relaxation aggregate.
    PolicyRelaxation,
    /// Immutable OKF import job or source artifact cited by a Knowledge change.
    OkfImport,
    /// Authored prompt template.
    Prompt,
    /// Authored context-pack document.
    ContextPack,
    /// Pre-cut authored Memory proposal retained only until the final hard cut.
    Memory,
}

impl ArtifactFamily {
    /// Every supported common-review family.
    pub const ALL: [Self; 10] = [
        Self::Knowledge,
        Self::Skill,
        Self::ToolServer,
        Self::ToolBinding,
        Self::Configuration,
        Self::PolicyRelaxation,
        Self::OkfImport,
        Self::Prompt,
        Self::ContextPack,
        Self::Memory,
    ];

    /// Stable wire/storage name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Knowledge => "knowledge",
            Self::Skill => "skill",
            Self::ToolServer => "tool_server",
            Self::ToolBinding => "tool_binding",
            Self::Configuration => "configuration",
            Self::PolicyRelaxation => "policy_relaxation",
            Self::OkfImport => "okf_import",
            Self::Prompt => "prompt",
            Self::ContextPack => "context_pack",
            Self::Memory => "memory",
        }
    }
}

impl fmt::Display for ArtifactFamily {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ArtifactFamily {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|family| family.as_str() == value)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown governed artifact family: {value:?}"),
            })
    }
}

/// One immutable typed address carried by the common proposal row.
///
/// `version` is the exact revision, content digest or binding revision the
/// proposal commit binds. `expected_revision` is the head the command author
/// inspected, when the domain command has a stale-write precondition. Both
/// are identifiers only: review lists and audit events never need artifact
/// content or secret-bearing configuration here.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactReference {
    /// Closed artifact-family vocabulary.
    pub family: ArtifactFamily,
    /// Stable aggregate, binding, import job or authored member id.
    pub artifact_id: String,
    /// Typed domain operation (`edit`, `bind`, `approve_version`, ...).
    pub operation: String,
    /// Exact immutable revision/digest proposed.
    pub version: String,
    /// Existing head inspected by a mutable command, if one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<String>,
}

impl ArtifactReference {
    /// Constructs and validates one content-free typed address.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when any component is empty, contains control
    /// characters, or exceeds the bounded common-review contract.
    pub fn new(
        family: ArtifactFamily,
        artifact_id: impl Into<String>,
        operation: impl Into<String>,
        version: impl Into<String>,
        expected_revision: Option<String>,
    ) -> Result<Self> {
        let reference = Self {
            family,
            artifact_id: artifact_id.into(),
            operation: operation.into(),
            version: version.into(),
            expected_revision,
        };
        reference.validate()?;
        Ok(reference)
    }

    /// Validates an address loaded from or about to enter persistence.
    pub fn validate(&self) -> Result<()> {
        bounded_component("artifact_id", &self.artifact_id, 1_024)?;
        bounded_component("operation", &self.operation, 64)?;
        bounded_component("version", &self.version, 512)?;
        if let Some(expected) = &self.expected_revision {
            bounded_component("expected_revision", expected, 512)?;
        }
        Ok(())
    }
}

fn bounded_component(name: &str, value: &str, max_chars: usize) -> Result<()> {
    let chars = value.chars().count();
    if chars == 0 || chars > max_chars || value.chars().any(char::is_control) {
        return Err(Error::Invalid {
            message: format!(
                "artifact reference {name} must contain 1..={max_chars} non-control characters"
            ),
        });
    }
    Ok(())
}

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
    /// Its non-channel effect ran (CPR-16, ADR-0081).
    Applied,
}

impl ProposalState {
    /// Every stored state.
    pub const ALL: [ProposalState; 5] = [
        ProposalState::Open,
        ProposalState::Rejected,
        ProposalState::Withdrawn,
        ProposalState::Published,
        ProposalState::Applied,
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
            ProposalState::Applied => "applied",
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

/// The API rendering of the stored vocabulary, with `approved` derived from
/// `open` plus a satisfied requirement.
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
    /// A governed non-channel effect ran.
    Applied,
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
            ProposalState::Applied => ProposalView::Applied,
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
            ProposalView::Applied => "applied",
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
/// The column retains its historical `target_channel` name, but this is the
/// effect vocabulary: publication writes a channel, classification changes
/// record state, and apply executes a typed governed command.
///
/// There is no `Default`: what a proposal would *do* is the first thing a
/// reviewer needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProposalEffect {
    /// Publish the members onto the target scope's published channel
    /// (FLOW-3). The only effect there was until AUTHZ-4.
    Published,
    /// Move the members to the sensitivity their proposed versions carry
    /// (AUTHZ-5, ADR-0038 decision 9) — the only path to `restricted`, and
    /// the only path back down from it.
    ///
    /// It writes no channel either: a reclassification changes what a record
    /// *is*, not where it is published, and a record can be reclassified
    /// without ever having crossed the trust boundary.
    ///
    /// Its requirement resolves at the **maximum of the current and proposed
    /// tiers**, which is the whole reason it is its own effect: taking only
    /// the proposed side would price a declassification at the tier it is
    /// leaving for, and the one direction that removes a control would be
    /// the cheap one.
    Classify,
    /// Apply a typed Knowledge aggregate mutation (CPR-16, ADR-0081).
    ///
    /// The reviewed VedaFlow object binds a content-free command manifest and
    /// payload hash; the effect projection holds erasable plaintext.
    Apply,
}

impl ProposalEffect {
    /// Every effect.
    pub const ALL: [ProposalEffect; 3] = [
        ProposalEffect::Published,
        ProposalEffect::Classify,
        ProposalEffect::Apply,
    ];

    /// Stable wire name, identical to the serde form and to the stored
    /// column (whose CHECK constraint mirrors this list).
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            ProposalEffect::Published => "published",
            ProposalEffect::Classify => "classify",
            ProposalEffect::Apply => "apply",
        }
    }

    /// The channel this effect writes, when it writes one.
    ///
    /// `None` for reclassification and typed application, which is the honest
    /// answer rather than a stand-in: their effects are state changes, and a caller
    /// that needs a channel here has taken a wrong turn.
    #[must_use]
    pub const fn channel(&self) -> Option<Channel> {
        match self {
            ProposalEffect::Published => Some(Channel::Published),
            ProposalEffect::Classify | ProposalEffect::Apply => None,
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

    /// `approved` is the only state computed rather than persisted.
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
            ProposalState::Applied,
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
        assert!(ProposalState::Applied.is_terminal());
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
        assert_eq!(ProposalEffect::Apply.channel(), None);
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

    #[test]
    fn typed_artifact_references_round_trip_and_reject_loose_addresses() {
        for family in ArtifactFamily::ALL {
            assert_eq!(
                family.to_string().parse::<ArtifactFamily>().unwrap(),
                family
            );
        }
        let reference = ArtifactReference::new(
            ArtifactFamily::Knowledge,
            "item-1",
            "edit",
            "revision-2",
            Some("revision-1".to_owned()),
        )
        .unwrap();
        assert_eq!(
            serde_json::from_value::<ArtifactReference>(serde_json::to_value(&reference).unwrap())
                .unwrap(),
            reference
        );
        assert!(
            ArtifactReference::new(
                ArtifactFamily::ToolBinding,
                "binding-1",
                "bind",
                "digest-1",
                Some("\n".to_owned())
            )
            .is_err()
        );
        assert!("tool-server".parse::<ArtifactFamily>().is_err());
    }
}
