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

use crate::{
    DurableOperationId, Error, IdentityId, KnowledgeItemId, ProposalId, Result, SkillVersionId,
    TenantId,
};

/// The only currently supported durable Skill-validation contract version.
pub const SKILL_VALIDATION_OPERATION_VERSION: u16 = 1;

/// First-class kinds of durable work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    /// Governed removal of one Knowledge aggregate's plaintext and indexes.
    KnowledgeErasure,
    /// Non-executing validation of one immutable Skill version.
    SkillValidation,
}

impl OperationKind {
    /// Every operation kind in stable storage order.
    pub const ALL: &'static [Self] = &[Self::KnowledgeErasure, Self::SkillValidation];

    /// Stable wire/storage name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::KnowledgeErasure => "knowledge_erasure",
            Self::SkillValidation => "skill_validation",
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
    /// Waiting for a bounded retry after a failed execution attempt.
    Failed,
    /// Policy or legal hold deliberately prevents execution.
    Blocked,
    /// A caller cancelled the operation before its effect committed.
    Cancelled,
    /// The bounded retry budget was exhausted; terminal.
    DeadLettered,
}

impl OperationState {
    /// Every stored state.
    pub const ALL: &'static [Self] = &[
        Self::Pending,
        Self::Running,
        Self::Succeeded,
        Self::Failed,
        Self::Blocked,
        Self::Cancelled,
        Self::DeadLettered,
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
            Self::Cancelled => "cancelled",
            Self::DeadLettered => "dead_lettered",
        }
    }

    /// Whether no worker may claim this row again.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Blocked | Self::Cancelled | Self::DeadLettered
        )
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
    /// Version of the closed operation-kind contract.
    pub operation_version: u16,
    /// VedaFlow change that authorised this work, for governed Knowledge work.
    pub change_id: Option<ProposalId>,
    /// Domain target, when this operation acts on one Knowledge aggregate.
    pub knowledge_item_id: Option<KnowledgeItemId>,
    /// Immutable Skill target, for Skill validation.
    pub skill_version_id: Option<SkillVersionId>,
    /// Identity whose current authority the worker must re-evaluate.
    pub requested_by: Option<IdentityId>,
    /// Time at which the request transaction authorised the operation.
    pub authorized_at: DateTime<Utc>,
    /// Work family.
    pub kind: OperationKind,
    /// Canonical input digest, never the input plaintext.
    pub input_hash: String,
    /// Current durable state.
    pub state: OperationState,
    /// Bounded customer-safe progress percentage.
    pub progress_percent: u8,
    /// Number of worker claims.
    pub attempts: i32,
    /// Earliest database time at which another attempt may be claimed.
    pub next_attempt_at: Option<DateTime<Utc>>,
    /// Caller cancellation request, when present.
    pub cancel_requested_at: Option<DateTime<Utc>>,
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
        assert!(OperationState::Cancelled.is_terminal());
        assert!(OperationState::DeadLettered.is_terminal());
        assert!(!OperationState::Failed.is_terminal());
        assert!("erase".parse::<OperationKind>().is_err());
    }
}
