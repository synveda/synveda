//! Reusable durable operation vocabulary (CPR-16, ADR-0081).
//!
//! Operations are the retry-safe address for work that may outlive the
//! request which governed it. They carry hashes, identifiers and bounded
//! content-free metadata; domain payloads stay in their owning aggregate.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{DurableOperationId, Error, KnowledgeItemId, ProposalId, Result, TenantId};

/// First-class kinds of durable work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    /// Governed removal of one Knowledge aggregate's plaintext and indexes.
    KnowledgeErasure,
}

impl OperationKind {
    /// Every operation kind in stable storage order.
    pub const ALL: &'static [Self] = &[Self::KnowledgeErasure];

    /// Stable wire/storage name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::KnowledgeErasure => "knowledge_erasure",
        }
    }
}

impl fmt::Display for OperationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for OperationKind {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|kind| kind.as_str() == value)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown durable operation kind: {value:?}"),
            })
    }
}

/// Durable operation lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    /// Ready for a worker to claim.
    Pending,
    /// Leased to one worker.
    Running,
    /// Completed successfully; terminal.
    Succeeded,
    /// A retryable or terminal execution failure, as recorded by attempts.
    Failed,
    /// Policy or legal hold deliberately prevents execution.
    Blocked,
}

impl OperationState {
    /// Every stored state.
    pub const ALL: &'static [Self] = &[
        Self::Pending,
        Self::Running,
        Self::Succeeded,
        Self::Failed,
        Self::Blocked,
    ];

    /// Stable wire/storage name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
        }
    }

    /// Whether no worker may claim this row again.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Blocked)
    }
}

impl fmt::Display for OperationState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for OperationState {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|state| state.as_str() == value)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown durable operation state: {value:?}"),
            })
    }
}

/// One durable operation as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableOperation {
    /// Stable operation id.
    pub id: DurableOperationId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// VedaFlow change that authorised this work.
    pub change_id: ProposalId,
    /// Domain target, when this operation acts on one Knowledge aggregate.
    pub knowledge_item_id: Option<KnowledgeItemId>,
    /// Work family.
    pub kind: OperationKind,
    /// Canonical input digest, never the input plaintext.
    pub input_hash: String,
    /// Current durable state.
    pub state: OperationState,
    /// Number of worker claims.
    pub attempts: i32,
    /// Current lease holder, if running.
    pub lease_owner: Option<String>,
    /// Lease expiry, if running.
    pub lease_expires_at: Option<DateTime<Utc>>,
    /// Content-free result/error metadata.
    pub result: Value,
    /// Stable machine error code from the last failed attempt.
    pub last_error_code: Option<String>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Last transition time.
    pub updated_at: DateTime<Utc>,
    /// First start time, when the operation has been claimed.
    pub started_at: Option<DateTime<Utc>>,
    /// Terminal time.
    pub completed_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocabularies_round_trip_and_terminal_states_are_explicit() {
        for kind in OperationKind::ALL {
            assert_eq!(kind.as_str().parse::<OperationKind>().unwrap(), *kind);
        }
        for state in OperationState::ALL {
            assert_eq!(state.as_str().parse::<OperationState>().unwrap(), *state);
        }
        assert!(OperationState::Succeeded.is_terminal());
        assert!(OperationState::Blocked.is_terminal());
        assert!(!OperationState::Failed.is_terminal());
        assert!("erase".parse::<OperationKind>().is_err());
    }
}
