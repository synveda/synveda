//! Explainable context-planning vocabulary (CPR-20, ADR-0084).
//!
//! A context run is a budgeted delivery record. Candidates and selections
//! cite immutable Knowledge revisions; feedback cites one exact selection.
//! These types deliberately contain no Knowledge body text: content remains
//! in its immutable revision and every disclosure re-runs the PDP.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::knowledge::KnowledgeLifecycleState;
use crate::{
    ContextCandidateId, ContextFeedbackId, ContextRunId, ContextSelectionId, Error,
    KnowledgeItemId, KnowledgeRevisionId, Result, ScopeId, TenantId,
};

fn joined(values: impl Iterator<Item = &'static str>) -> String {
    values.collect::<Vec<_>>().join(", ")
}

/// How much visible planner detail is retained and exposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TraceRetentionMode {
    /// Exact visible candidate and selection references plus score detail.
    #[default]
    Full,
    /// Exact visible references and reasons, with sensitive diagnostic fields
    /// omitted from trace reads.
    Redacted,
    /// Per-candidate and per-selection hashes without Knowledge addresses.
    HashesOnly,
    /// Only the run's minimal delivery/version/budget envelope.
    Disabled,
}

impl TraceRetentionMode {
    /// Every supported mode, least detail last.
    pub const ALL: [Self; 4] = [Self::Full, Self::Redacted, Self::HashesOnly, Self::Disabled];

    /// Stable stored and wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Redacted => "redacted",
            Self::HashesOnly => "hashes_only",
            Self::Disabled => "disabled",
        }
    }

    /// Used by serde to omit the product default from stored pack JSON. This
    /// preserves existing content-addressed pack bytes when the field is not
    /// configured explicitly.
    #[must_use]
    pub const fn is_full(&self) -> bool {
        matches!(self, Self::Full)
    }
}

impl fmt::Display for TraceRetentionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TraceRetentionMode {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|mode| mode.as_str() == value)
            .ok_or_else(|| Error::Invalid {
                message: format!(
                    "unknown context trace-retention mode: {value:?} (one of {})",
                    joined(Self::ALL.iter().copied().map(Self::as_str))
                ),
            })
    }
}

/// Terminal state of one immutable context-planning attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextCompletionStatus {
    /// Planning has a durable address but has not finished.
    Pending,
    /// Planning and delivery completed.
    Completed,
    /// Planning ended without a delivered block.
    Failed,
}

impl ContextCompletionStatus {
    /// Every supported state.
    pub const ALL: [Self; 3] = [Self::Pending, Self::Completed, Self::Failed];

    /// Stable stored and wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

impl fmt::Display for ContextCompletionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ContextCompletionStatus {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|state| state.as_str() == value)
            .ok_or_else(|| Error::Invalid {
                message: format!(
                    "unknown context completion status: {value:?} (one of {})",
                    joined(Self::ALL.iter().copied().map(Self::as_str))
                ),
            })
    }
}

/// Why a visible candidate was considered, selected or excluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextReasonCode {
    /// Dense-vector similarity contributed.
    SemanticMatch,
    /// Lexical search contributed.
    KeywordMatch,
    /// A project-scoped convention received its deterministic boost.
    ProjectConvention,
    /// A caller-owned preference received its deterministic boost.
    PersonalPreference,
    /// Current freshness improved rank.
    FreshnessBoost,
    /// Governed metadata explicitly pinned the revision.
    ExplicitPin,
    /// The revision belongs to a superseded item and is not current truth.
    Superseded,
    /// The revision is stale or past its verification time.
    Stale,
    /// The visible item is outside this session's task scope.
    OutsideTaskScope,
    /// It survived retrieval but did not fit the context budget.
    TokenBudget,
    /// Another visible candidate had the same canonical content hash.
    Duplicate,
}

impl ContextReasonCode {
    /// Complete initial reason vocabulary.
    pub const ALL: [Self; 11] = [
        Self::SemanticMatch,
        Self::KeywordMatch,
        Self::ProjectConvention,
        Self::PersonalPreference,
        Self::FreshnessBoost,
        Self::ExplicitPin,
        Self::Superseded,
        Self::Stale,
        Self::OutsideTaskScope,
        Self::TokenBudget,
        Self::Duplicate,
    ];

    /// Stable stored and wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SemanticMatch => "semantic_match",
            Self::KeywordMatch => "keyword_match",
            Self::ProjectConvention => "project_convention",
            Self::PersonalPreference => "personal_preference",
            Self::FreshnessBoost => "freshness_boost",
            Self::ExplicitPin => "explicit_pin",
            Self::Superseded => "superseded",
            Self::Stale => "stale",
            Self::OutsideTaskScope => "outside_task_scope",
            Self::TokenBudget => "token_budget",
            Self::Duplicate => "duplicate",
        }
    }
}

impl fmt::Display for ContextReasonCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ContextReasonCode {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|reason| reason.as_str() == value)
            .ok_or_else(|| Error::Invalid {
                message: format!(
                    "unknown context reason code: {value:?} (one of {})",
                    joined(Self::ALL.iter().copied().map(Self::as_str))
                ),
            })
    }
}

/// Explicit feedback about one selected immutable revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextFeedbackType {
    /// The agent cited or otherwise referenced the revision.
    ReferencedByAgent,
    /// A user accepted an output that used the revision.
    AcceptedByUser,
    /// A user explicitly marked it helpful.
    Helpful,
    /// A user explicitly marked it unhelpful.
    Unhelpful,
    /// The selected revision caused a later correction.
    CausedCorrection,
}

impl ContextFeedbackType {
    /// Complete feedback vocabulary.
    pub const ALL: [Self; 5] = [
        Self::ReferencedByAgent,
        Self::AcceptedByUser,
        Self::Helpful,
        Self::Unhelpful,
        Self::CausedCorrection,
    ];

    /// Stable stored and wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReferencedByAgent => "referenced_by_agent",
            Self::AcceptedByUser => "accepted_by_user",
            Self::Helpful => "helpful",
            Self::Unhelpful => "unhelpful",
            Self::CausedCorrection => "caused_correction",
        }
    }
}

impl fmt::Display for ContextFeedbackType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ContextFeedbackType {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|feedback| feedback.as_str() == value)
            .ok_or_else(|| Error::Invalid {
                message: format!(
                    "unknown context feedback type: {value:?} (one of {})",
                    joined(Self::ALL.iter().copied().map(Self::as_str))
                ),
            })
    }
}

/// One independently visible candidate retained for a context run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextCandidate {
    /// Candidate row id.
    pub id: ContextCandidateId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Context run that considered it.
    pub context_run_id: ContextRunId,
    /// Stable deterministic position in the considered pool.
    pub ordinal: i32,
    /// Stable item, absent in hashes-only mode.
    pub knowledge_item_id: Option<KnowledgeItemId>,
    /// Immutable revision, absent in hashes-only mode.
    pub knowledge_revision_id: Option<KnowledgeRevisionId>,
    /// Canonical revision content hash, retained without plaintext.
    pub content_hash: String,
    /// Governing scope, absent when addresses are not retained.
    pub scope_id: Option<ScopeId>,
    /// Lifecycle observed during planning, absent in hashes-only mode.
    pub lifecycle_state: Option<KnowledgeLifecycleState>,
    /// Integer score components, per million.
    pub keyword_score_micros: i32,
    /// Integer semantic contribution, per million.
    pub semantic_score_micros: i32,
    /// Integer freshness contribution, per million.
    pub freshness_score_micros: i32,
    /// Integer explicit-pin contribution, per million.
    pub pin_score_micros: i32,
    /// Integer current-state contribution, per million.
    pub current_state_score_micros: i32,
    /// Final deterministic score, per million.
    pub final_score_micros: i32,
    /// Why it was considered.
    pub reason_codes: Vec<ContextReasonCode>,
    /// Why a visible candidate was not selected.
    pub exclusion_reason: Option<ContextReasonCode>,
    /// Persistence time.
    pub created_at: DateTime<Utc>,
}

/// One exact revision selected for delivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSelection {
    /// Selection row id.
    pub id: ContextSelectionId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Context run that selected it.
    pub context_run_id: ContextRunId,
    /// One-based delivery rank.
    pub rank: i32,
    /// Stable item, absent in hashes-only mode.
    pub knowledge_item_id: Option<KnowledgeItemId>,
    /// Immutable revision, absent in hashes-only mode.
    pub knowledge_revision_id: Option<KnowledgeRevisionId>,
    /// Canonical revision content hash.
    pub content_hash: String,
    /// Estimated tokens charged to this selection.
    pub token_count: i32,
    /// Visible selection reasons.
    pub reason_codes: Vec<ContextReasonCode>,
    /// Selection time.
    pub created_at: DateTime<Utc>,
}

/// One immutable feedback assertion about a selected revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextFeedback {
    /// Feedback row id.
    pub id: ContextFeedbackId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Exact context run.
    pub context_run_id: ContextRunId,
    /// Exact selection.
    pub context_selection_id: ContextSelectionId,
    /// Exact immutable revision judged.
    pub knowledge_revision_id: KnowledgeRevisionId,
    /// Feedback vocabulary.
    pub feedback_type: ContextFeedbackType,
    /// Authenticated subject that supplied it.
    pub principal_id: String,
    /// Creation time.
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocabularies_round_trip() {
        for mode in TraceRetentionMode::ALL {
            assert_eq!(mode.as_str().parse::<TraceRetentionMode>().unwrap(), mode);
        }
        for status in ContextCompletionStatus::ALL {
            assert_eq!(
                status.as_str().parse::<ContextCompletionStatus>().unwrap(),
                status
            );
        }
        for reason in ContextReasonCode::ALL {
            assert_eq!(
                reason.as_str().parse::<ContextReasonCode>().unwrap(),
                reason
            );
        }
        for feedback in ContextFeedbackType::ALL {
            assert_eq!(
                feedback.as_str().parse::<ContextFeedbackType>().unwrap(),
                feedback
            );
        }
    }

    #[test]
    fn full_is_the_pack_default_and_omittable() {
        assert_eq!(TraceRetentionMode::default(), TraceRetentionMode::Full);
        assert!(TraceRetentionMode::Full.is_full());
        assert!(!TraceRetentionMode::Disabled.is_full());
    }
}
