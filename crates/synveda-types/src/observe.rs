//! Observe ingestion vocabulary (seed §3, MEM-1, ADR-0020).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Error;

/// What an observe event reports (seed §3: "transcript deltas, tool
/// results, decisions"). Drives extraction routing in MEM-3; stored as the
/// staging row's `kind` and CHECK-constrained to this vocabulary.
///
/// The vocabulary answers two questions at once, and they are not the same
/// question. The first three variants say *what was seen*, all of them by a
/// host watching a session. [`Assertion`](ObserveKind::Assertion) says the
/// content was *composed and volunteered by a model* rather than observed,
/// which is a provenance claim rather than a content one — see its own
/// documentation, and ADR-0057 decision 8 for why this axis is `kind` and
/// not a parallel field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObserveKind {
    /// A slice of session transcript.
    TranscriptDelta,
    /// The outcome of a tool invocation.
    ToolResult,
    /// A decision the agent or user made, with its context — one the host
    /// *observed being made*. A model reporting its own decision through a
    /// tool call is an [`Assertion`](ObserveKind::Assertion).
    Decision,
    /// A fact a model composed and chose to store, arriving because the
    /// model called a write tool (ADPT-2's `remember`, ADR-0057 decisions 7
    /// and 8) rather than because a hook observed a session.
    ///
    /// The distinction is epistemic and cannot be recovered later: a hook
    /// records what happened whether or not the model thinks to call it,
    /// while an assertion is the model's own claim, shaped by the model,
    /// for the recorder. Everything downstream is unchanged — the same
    /// route, the same `MemoryWrite` decision, the same redaction scan, the
    /// same home-scope placement — so this variant buys provenance, not
    /// privilege.
    Assertion,
}

impl ObserveKind {
    /// All kinds.
    pub const ALL: [ObserveKind; 4] = [
        ObserveKind::TranscriptDelta,
        ObserveKind::ToolResult,
        ObserveKind::Decision,
        ObserveKind::Assertion,
    ];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            ObserveKind::TranscriptDelta => "transcript_delta",
            ObserveKind::ToolResult => "tool_result",
            ObserveKind::Decision => "decision",
            ObserveKind::Assertion => "assertion",
        }
    }

    /// Whether the content was composed by a model rather than observed by
    /// a host. The one question the [`Assertion`](ObserveKind::Assertion)
    /// variant exists to answer, named so that callers ask it by meaning
    /// rather than by matching on a variant they have to remember the
    /// significance of.
    #[must_use]
    pub const fn is_model_asserted(&self) -> bool {
        matches!(self, ObserveKind::Assertion)
    }
}

impl fmt::Display for ObserveKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ObserveKind {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ObserveKind::ALL
            .into_iter()
            .find(|kind| kind.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown observe kind: {s:?}"),
            })
    }
}

/// A quarantined observe event's review state (MEM-2, ADR-0021
/// decision 5). Review is one-shot: `pending → released | rejected`,
/// schema-enforced by the transition trigger in migration 0013.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuarantineState {
    /// Awaiting review; the staging row exists but no work signal was
    /// sent — the pipeline cannot see it.
    Pending,
    /// A reviewer released it: the work signal went out and the
    /// pipeline treats it like any admitted event.
    Released,
    /// A reviewer rejected it: the staging row remains immutable
    /// provenance that never enters the pipeline.
    Rejected,
}

impl QuarantineState {
    /// All states.
    pub const ALL: [QuarantineState; 3] = [
        QuarantineState::Pending,
        QuarantineState::Released,
        QuarantineState::Rejected,
    ];

    /// Stable wire name, identical to the serde form and the stored
    /// column value.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            QuarantineState::Pending => "pending",
            QuarantineState::Released => "released",
            QuarantineState::Rejected => "rejected",
        }
    }
}

impl fmt::Display for QuarantineState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for QuarantineState {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        QuarantineState::ALL
            .into_iter()
            .find(|state| state.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown quarantine state: {s:?}"),
            })
    }
}
