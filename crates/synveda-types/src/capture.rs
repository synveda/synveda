//! Reviewable Knowledge proposals extracted from immutable session evidence
//! (CPR-18, ADR-0083).
//!
//! A [`CaptureBatch`] freezes the exact eligible event set it was extracted
//! from. A [`CaptureCandidate`] is model output, not published Knowledge: it
//! becomes current only after one of its decisions enters the ordinary
//! Knowledge/VedaFlow command layer. This distinction is represented in the
//! types so a caller cannot confuse a successful extraction with a publish.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::knowledge::{
    KnowledgeMutationOutcome, KnowledgeOrigin, KnowledgeRevisionContent, KnowledgeType,
};
use crate::{
    CaptureBatchId, CaptureCandidateDecisionId, CaptureCandidateId, Error, KnowledgeItemId,
    KnowledgeRevisionId, ProjectId, ProposalId, Result, ScopeId, SessionEventId, SessionId,
    TenantId, WorkspaceId,
};

/// Maximum extraction attempts before a batch becomes failed.
pub const MAX_CAPTURE_ATTEMPTS: i32 = 5;
/// Maximum candidates one event may propose.
pub const MAX_CANDIDATES_PER_EVENT: usize = 16;
/// Maximum visible Knowledge neighbours persisted for one candidate.
pub const MAX_CAPTURE_MATCHES: usize = 20;

macro_rules! string_enum {
    ($name:ident, [$($variant:ident => $value:literal),+ $(,)?], $label:literal) => {
        #[doc = $label]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $(
                #[doc = concat!("`", $value, "`.")]
                $variant,
            )+
        }

        impl $name {
            /// Every value in stable storage order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// Stable wire and storage name.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $value),+ }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(value: &str) -> Result<Self> {
                Self::ALL.iter().copied().find(|item| item.as_str() == value)
                    .ok_or_else(|| Error::Invalid {
                        message: format!(concat!("unknown ", $label, ": {:?}"), value),
                    })
            }
        }
    };
}

string_enum!(
    CaptureBatchState,
    [
        Pending => "pending",
        Running => "running",
        Completed => "completed",
        Failed => "failed"
    ],
    "capture batch state"
);

string_enum!(
    CaptureCandidateState,
    [
        Pending => "pending",
        Accepted => "accepted",
        EditedAndAccepted => "edited_and_accepted",
        Merged => "merged",
        Replaced => "replaced",
        Dismissed => "dismissed",
        Failed => "failed"
    ],
    "capture candidate state"
);

string_enum!(
    CaptureMatchKind,
    [
        Duplicate => "duplicate",
        Conflict => "conflict",
        PossibleSupersession => "possible_supersession"
    ],
    "capture match kind"
);

string_enum!(
    CaptureDecisionAction,
    [
        Accept => "accept",
        EditAndAccept => "edit_and_accept",
        Merge => "merge",
        Replace => "replace",
        Dismiss => "dismiss"
    ],
    "capture decision action"
);

string_enum!(
    CaptureDecisionState,
    [
        Running => "running",
        Succeeded => "succeeded",
        Failed => "failed"
    ],
    "capture decision state"
);

/// One extraction job over an exact, ordered session-event snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureBatch {
    /// Stable job id.
    pub id: CaptureBatchId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Session whose evidence was frozen.
    pub session_id: SessionId,
    /// Session's governed scope.
    pub scope_id: ScopeId,
    /// Session's workspace.
    pub workspace_id: WorkspaceId,
    /// Session's project, when present.
    pub project_id: Option<ProjectId>,
    /// Principal that opened the session and on whose authority extraction
    /// resolves visible neighbours.
    pub principal_id: String,
    /// BLAKE3-256 of the ordered eligible evidence tuple set.
    pub input_hash: String,
    /// Number of frozen events.
    pub event_count: i32,
    /// Durable job state.
    pub state: CaptureBatchState,
    /// Extraction implementation, after processing begins.
    pub extractor_method: Option<String>,
    /// Model or ruleset version, after processing begins.
    pub model_version: Option<String>,
    /// Attempts claimed so far.
    pub attempts: i32,
    /// Number of candidates materialised.
    pub candidate_count: i32,
    /// Content-free stable failure code, when failed.
    pub error_code: Option<String>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// First processing instant.
    pub started_at: Option<DateTime<Utc>>,
    /// Terminal instant.
    pub completed_at: Option<DateTime<Utc>>,
    /// Last state transition.
    pub updated_at: DateTime<Utc>,
}

/// One visible current-Knowledge match retained as a review hint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureMatch {
    /// Existing stable aggregate.
    pub knowledge_item_id: KnowledgeItemId,
    /// Exact current revision compared.
    pub knowledge_revision_id: KnowledgeRevisionId,
    /// Classifier outcome.
    pub kind: CaptureMatchKind,
    /// Deterministic integer similarity in `0..=1000`.
    pub similarity_permille: i32,
    /// Stable, content-free explanation.
    pub reason_code: String,
}

/// One reviewable proposal emitted by a batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureCandidate {
    /// Stable candidate id.
    pub id: CaptureCandidateId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Batch that produced it.
    pub batch_id: CaptureBatchId,
    /// Source session.
    pub session_id: SessionId,
    /// Stable position inside the batch.
    pub ordinal: i32,
    /// Proposed governing scope.
    pub proposed_scope_id: ScopeId,
    /// Proposed project association.
    pub proposed_project_id: Option<ProjectId>,
    /// Proposed personal owner.
    pub proposed_owner_principal_id: Option<String>,
    /// Proposed Knowledge type.
    pub knowledge_type: KnowledgeType,
    /// Proposed Knowledge origin.
    pub origin: KnowledgeOrigin,
    /// Complete proposed immutable revision content.
    pub content: KnowledgeRevisionContent,
    /// Canonical proposed-content hash.
    pub content_hash: String,
    /// Current review state.
    pub state: CaptureCandidateState,
    /// Exact source events, ordered by session sequence.
    pub source_event_ids: Vec<SessionEventId>,
    /// Only independently policy-visible current Knowledge neighbours.
    pub matches: Vec<CaptureMatch>,
    /// VedaFlow change opened by a terminal publish action.
    pub resulting_change_id: Option<ProposalId>,
    /// Its governance outcome.
    pub resulting_outcome: Option<KnowledgeMutationOutcome>,
    /// Result stable item when applicable.
    pub resulting_knowledge_item_id: Option<KnowledgeItemId>,
    /// Result immutable revision when applied.
    pub resulting_revision_id: Option<KnowledgeRevisionId>,
    /// Actor that made the terminal decision.
    pub decided_by: Option<String>,
    /// Human reason for dismissal when supplied.
    pub decision_reason: Option<String>,
    /// Decision time.
    pub decided_at: Option<DateTime<Utc>>,
    /// Whether governed erasure scrubbed the candidate's plaintext.
    pub content_erased: bool,
    /// Creation time.
    pub created_at: DateTime<Utc>,
}

/// Durable, idempotent decision intent and its eventual result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureCandidateDecision {
    /// Stable intent id.
    pub id: CaptureCandidateDecisionId,
    /// Candidate being decided.
    pub candidate_id: CaptureCandidateId,
    /// Requested action.
    pub action: CaptureDecisionAction,
    /// Execution state.
    pub state: CaptureDecisionState,
    /// Actor that opened the intent.
    pub actor_subject: String,
    /// Caller idempotency key.
    pub idempotency_key: String,
    /// Canonical request digest.
    pub request_hash: String,
    /// Mutable request data until a successful forget scrubs it.
    pub payload: Option<Value>,
    /// Hash retained if payload is scrubbed.
    pub payload_hash: String,
    /// VedaFlow change result.
    pub resulting_change_id: Option<ProposalId>,
    /// Governance outcome.
    pub resulting_outcome: Option<KnowledgeMutationOutcome>,
    /// Result aggregate.
    pub resulting_knowledge_item_id: Option<KnowledgeItemId>,
    /// Result revision.
    pub resulting_revision_id: Option<KnowledgeRevisionId>,
    /// Content-free failure code.
    pub error_code: Option<String>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Completion time.
    pub completed_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_vocabularies_round_trip_and_do_not_call_pending_published() {
        for state in CaptureBatchState::ALL {
            assert_eq!(state.as_str().parse::<CaptureBatchState>().unwrap(), *state);
        }
        for state in CaptureCandidateState::ALL {
            assert_eq!(
                state.as_str().parse::<CaptureCandidateState>().unwrap(),
                *state
            );
        }
        for kind in CaptureMatchKind::ALL {
            assert_eq!(kind.as_str().parse::<CaptureMatchKind>().unwrap(), *kind);
        }
        assert!("published".parse::<CaptureCandidateState>().is_err());
        assert!("superseded".parse::<CaptureMatchKind>().is_err());
    }
}
