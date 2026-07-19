//! Observe ingestion vocabulary (seed §3, MEM-1, ADR-0020).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Error;

/// What an observe event reports (seed §3: "transcript deltas, tool
/// results, decisions"). Drives extraction routing in MEM-3; stored as the
/// staging row's `kind` and CHECK-constrained to this vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObserveKind {
    /// A slice of session transcript.
    TranscriptDelta,
    /// The outcome of a tool invocation.
    ToolResult,
    /// A decision the agent or user made, with its context.
    Decision,
}

impl ObserveKind {
    /// All kinds.
    pub const ALL: [ObserveKind; 3] = [
        ObserveKind::TranscriptDelta,
        ObserveKind::ToolResult,
        ObserveKind::Decision,
    ];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            ObserveKind::TranscriptDelta => "transcript_delta",
            ObserveKind::ToolResult => "tool_result",
            ObserveKind::Decision => "decision",
        }
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
        match s {
            "transcript_delta" => Ok(ObserveKind::TranscriptDelta),
            "tool_result" => Ok(ObserveKind::ToolResult),
            "decision" => Ok(ObserveKind::Decision),
            other => Err(Error::Invalid {
                message: format!("unknown observe kind: {other:?}"),
            }),
        }
    }
}
