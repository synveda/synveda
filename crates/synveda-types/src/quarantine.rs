//! The quarantine review vocabulary (MEM-2, ADR-0021 decision 5).
//!
//! `ObserveKind` lived here until CPR-12 (ADR-0078 decision 1). It said what
//! an observe event reported — `transcript_delta`, `tool_result`, `decision`,
//! `assertion` — and extraction routed on it. The session ledger's
//! [`crate::session::SessionEventType`] answers the same question with twelve
//! names instead of four, so the vocabulary left with the plane it described
//! and this module kept the one type that outlived it.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Error;

/// A quarantined session event's review state (MEM-2, ADR-0021
/// decision 5). Review is one-shot: `pending → released | rejected`,
/// schema-enforced by the transition trigger in migration 0046.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuarantineState {
    /// Awaiting review; the event row exists but no work signal was
    /// sent — the pipeline cannot see it.
    Pending,
    /// A reviewer released it: the work signal went out and the
    /// pipeline treats it like any admitted event.
    Released,
    /// A reviewer rejected it: the event row remains immutable
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
