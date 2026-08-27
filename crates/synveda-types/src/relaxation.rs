//! Governed policy-relaxation vocabulary (CPR-31, ADR-0090).
//!
//! The permission vocabulary is deliberately closed over one current product
//! action. A relaxation is reviewable data, never caller-supplied Cedar and
//! never a post-decision override.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    ConfigurationVersionId, Error, IdentityId, ProposalId, RelaxationId, RelaxationVersionId,
    Result, ScopeId, Sensitivity, TenantId,
};

/// Longest requested window the product accepts before governed
/// Configuration narrows it further.
pub const PRODUCT_MAX_RELAXATION_SECS: u32 = 90 * 24 * 60 * 60;
/// Longest reason retained in the governed effect projection.
pub const MAX_RELAXATION_REASON_CHARS: usize = 512;

/// Actions a typed relaxation may temporarily widen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RelaxationAction {
    /// Read policy-visible current Knowledge at the named governed scope.
    #[serde(rename = "knowledge.read")]
    KnowledgeRead,
}

impl RelaxationAction {
    /// Complete supported vocabulary.
    pub const ALL: [Self; 1] = [Self::KnowledgeRead];

    /// Stable wire and storage spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::KnowledgeRead => "knowledge.read",
        }
    }
}

impl fmt::Display for RelaxationAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for RelaxationAction {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == value)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown relaxation action {value:?}; supported: knowledge.read"),
            })
    }
}

/// Complete immutable terms reviewed for one relaxation version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelaxationTerms {
    /// Provisioned identity receiving the temporary permission.
    pub subject_identity_id: IdentityId,
    /// Exact target scope whose current Knowledge may be read.
    pub target_scope_id: ScopeId,
    /// Closed permission vocabulary.
    pub action: RelaxationAction,
    /// Highest sensitivity this version may admit.
    pub max_sensitivity: Sensitivity,
    /// Requested absolute beginning of the window.
    pub requested_start_at: DateTime<Utc>,
    /// Requested absolute end. Application may only shorten this into the
    /// stored hard expiry.
    pub requested_end_at: DateTime<Utc>,
    /// Mandatory reviewable justification.
    pub reason: String,
}

impl RelaxationTerms {
    /// Validate syntax and the product-wide absolute bounds. Governed
    /// Configuration applies its narrower ceiling at effect time.
    pub fn validate(&self) -> Result<()> {
        if self.reason.trim() != self.reason
            || self.reason.is_empty()
            || self.reason.chars().count() > MAX_RELAXATION_REASON_CHARS
            || self.reason.chars().any(char::is_control)
        {
            return Err(Error::Invalid {
                message: format!(
                    "relaxation reason must contain 1..={MAX_RELAXATION_REASON_CHARS} non-control characters without surrounding whitespace"
                ),
            });
        }
        if self.requested_start_at >= self.requested_end_at {
            return Err(Error::Invalid {
                message: "relaxation requested_end_at must be after requested_start_at".to_owned(),
            });
        }
        let maximum = TimeDelta::seconds(i64::from(PRODUCT_MAX_RELAXATION_SECS));
        if self.requested_end_at - self.requested_start_at > maximum {
            return Err(Error::Invalid {
                message: format!(
                    "relaxation requested window exceeds the product ceiling of {PRODUCT_MAX_RELAXATION_SECS} seconds"
                ),
            });
        }
        Ok(())
    }

    /// Canonical content hash of the reviewed terms.
    pub fn content_hash(&self) -> Result<String> {
        self.validate()?;
        let value = crate::json::canonicalise(&serde_json::to_value(self).map_err(|error| {
            Error::Invalid {
                message: format!("encode relaxation terms: {error}"),
            }
        })?);
        Ok(blake3::hash(value.to_string().as_bytes())
            .to_hex()
            .to_string())
    }
}

/// Typed mutations carried by `Policy/apply` VedaFlow changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum RelaxationCommand {
    /// Create a stable aggregate and first immutable version.
    Create {
        /// Preallocated stable id, also used for idempotent replay.
        relaxation_id: RelaxationId,
        /// First immutable version id.
        version_id: RelaxationVersionId,
        /// Reviewed terms.
        terms: RelaxationTerms,
    },
    /// Publish a replacement immutable version.
    Revise {
        /// Stable aggregate.
        relaxation_id: RelaxationId,
        /// Exact head inspected by the caller.
        expected_current_version_id: RelaxationVersionId,
        /// New immutable version id.
        version_id: RelaxationVersionId,
        /// Stable governing scope, repeated so authorization never has to
        /// trust a payload-derived destination.
        governing_scope_id: ScopeId,
        /// Complete replacement terms.
        terms: RelaxationTerms,
    },
    /// End a relaxation before its hard expiry.
    Revoke {
        /// Stable aggregate.
        relaxation_id: RelaxationId,
        /// Exact head inspected by the caller.
        expected_current_version_id: RelaxationVersionId,
        /// Stable governing scope.
        governing_scope_id: ScopeId,
        /// Mandatory reason for ending it.
        reason: String,
    },
}

impl RelaxationCommand {
    /// Stable command name for persistence, metrics and audit.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Create { .. } => "create",
            Self::Revise { .. } => "revise",
            Self::Revoke { .. } => "revoke",
        }
    }

    /// Stable aggregate named by every command.
    #[must_use]
    pub const fn relaxation_id(&self) -> RelaxationId {
        match self {
            Self::Create { relaxation_id, .. }
            | Self::Revise { relaxation_id, .. }
            | Self::Revoke { relaxation_id, .. } => *relaxation_id,
        }
    }

    /// Resulting version when one is preallocated.
    #[must_use]
    pub const fn version_id(&self) -> Option<RelaxationVersionId> {
        match self {
            Self::Create { version_id, .. } | Self::Revise { version_id, .. } => Some(*version_id),
            Self::Revoke { .. } => None,
        }
    }

    /// Scope at which the effect is governed.
    #[must_use]
    pub const fn governing_scope_id(&self) -> ScopeId {
        match self {
            Self::Create { terms, .. } | Self::Revise { terms, .. } => terms.target_scope_id,
            Self::Revoke {
                governing_scope_id, ..
            } => *governing_scope_id,
        }
    }

    /// Structural validation independent of persisted state.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Create { terms, .. } => terms.validate(),
            Self::Revise {
                governing_scope_id,
                terms,
                ..
            } => {
                terms.validate()?;
                if terms.target_scope_id != *governing_scope_id {
                    return Err(Error::Invalid {
                        message: "a relaxation revision cannot move its governing scope".to_owned(),
                    });
                }
                Ok(())
            }
            Self::Revoke { reason, .. } => validate_revoke_reason(reason),
        }
    }
}

fn validate_revoke_reason(reason: &str) -> Result<()> {
    if reason.trim() != reason
        || reason.is_empty()
        || reason.chars().count() > MAX_RELAXATION_REASON_CHARS
        || reason.chars().any(char::is_control)
    {
        return Err(Error::Invalid {
            message: format!(
                "relaxation revocation reason must contain 1..={MAX_RELAXATION_REASON_CHARS} non-control characters without surrounding whitespace"
            ),
        });
    }
    Ok(())
}

/// Stable relaxation aggregate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relaxation {
    /// Stable id.
    pub id: RelaxationId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Scope whose policy governs every version.
    pub governing_scope_id: ScopeId,
    /// Current immutable terms.
    pub current_version_id: RelaxationVersionId,
    /// Aggregate revision, incremented by head movement or revocation.
    pub revision: u64,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Proposal author.
    pub created_by: IdentityId,
    /// Last state transition.
    pub updated_at: DateTime<Utc>,
    /// Last acting identity.
    pub updated_by: IdentityId,
    /// Terminal early revocation time.
    pub revoked_at: Option<DateTime<Utc>>,
    /// Actor applying the governed revocation.
    pub revoked_by: Option<IdentityId>,
    /// Governed revocation change.
    pub revocation_proposal_id: Option<ProposalId>,
    /// Human reason, retained separately from ordinary audit payloads.
    pub revocation_reason: Option<String>,
    /// Bookkeeping stamp after the content-free expiry event is chained.
    pub expiry_recorded_at: Option<DateTime<Utc>>,
}

/// One immutable applied relaxation version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelaxationVersion {
    /// Immutable id.
    pub id: RelaxationVersionId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Stable aggregate.
    pub relaxation_id: RelaxationId,
    /// Monotonic aggregate-local ordinal.
    pub ordinal: i64,
    /// Typed VedaFlow change that applied this version.
    pub proposal_id: ProposalId,
    /// Reviewed terms.
    pub terms: RelaxationTerms,
    /// Authenticated subject spelling frozen from the identity row.
    pub subject_principal_id: String,
    /// Actual start, never before effect application.
    pub effective_start_at: DateTime<Utc>,
    /// Absolute authority boundary calculated at apply time.
    pub hard_expires_at: DateTime<Utc>,
    /// Exact governed Configuration version used, absent only for the
    /// built-in fail-safe document.
    pub configuration_version_id: Option<ConfigurationVersionId>,
    /// Digest of the exact document, including the fail-safe.
    pub configuration_hash: String,
    /// Canonical hash of [`Self::terms`].
    pub content_hash: String,
    /// Proposal author.
    pub creator_id: IdentityId,
    /// Explicit approval identities whose commit-bound verdicts satisfied
    /// the live matrix.
    pub approver_ids: Vec<IdentityId>,
    /// True when the live matrix required no explicit approval.
    pub auto_applied: bool,
    /// Application time / transaction time.
    pub created_at: DateTime<Utc>,
}

/// Current status derived from immutable time and terminal revocation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelaxationStatus {
    /// The effective start is still in the future.
    Scheduled,
    /// Effective now.
    Active,
    /// Ended by its immutable hard expiry.
    Expired,
    /// Ended early by a governed revoke change.
    Revoked,
}

impl RelaxationStatus {
    /// Stable names.
    pub const ALL: [Self; 4] = [Self::Scheduled, Self::Active, Self::Expired, Self::Revoked];

    /// Wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Active => "active",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
        }
    }
}

impl Relaxation {
    /// Status of the supplied current version at `now`.
    #[must_use]
    pub fn status_at(&self, version: &RelaxationVersion, now: DateTime<Utc>) -> RelaxationStatus {
        if self.revoked_at.is_some() {
            RelaxationStatus::Revoked
        } else if now < version.effective_start_at {
            RelaxationStatus::Scheduled
        } else if now < version.hard_expires_at {
            RelaxationStatus::Active
        } else {
            RelaxationStatus::Expired
        }
    }
}

/// Combined current projection used by policy and API readers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentRelaxation {
    /// Stable aggregate.
    pub relaxation: Relaxation,
    /// Current immutable version.
    pub version: RelaxationVersion,
}

impl CurrentRelaxation {
    /// Match the immutable subject, action, target, and sensitivity terms.
    /// Request gathering is the authority for database-time window filtering;
    /// the PDP repeats this structural match independently.
    #[must_use]
    pub fn matches(
        &self,
        subject: &str,
        action: RelaxationAction,
        target_scope_id: ScopeId,
        sensitivity: Sensitivity,
    ) -> bool {
        self.relaxation.revoked_at.is_none()
            && self.version.subject_principal_id == subject
            && self.version.terms.action == action
            && self.version.terms.target_scope_id == target_scope_id
            && sensitivity <= self.version.terms.max_sensitivity
    }

    /// Whether this row grants exactly the supplied decision at `now`.
    #[must_use]
    pub fn grants(
        &self,
        subject: &str,
        action: RelaxationAction,
        target_scope_id: ScopeId,
        sensitivity: Sensitivity,
        now: DateTime<Utc>,
    ) -> bool {
        self.relaxation.status_at(&self.version, now) == RelaxationStatus::Active
            && self.matches(subject, action, target_scope_id, sensitivity)
    }
}

/// Governance outcome shared with other typed artifact commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelaxationMutationOutcome {
    /// Effect completed.
    Applied,
    /// Exact bytes remain on the VedaFlow review queue.
    PendingReview,
    /// Change closed without an effect.
    Rejected,
}

/// Result of create, revise or revoke.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelaxationMutationResult {
    /// Typed VedaFlow change id.
    pub change_id: ProposalId,
    /// Governance outcome.
    pub outcome: RelaxationMutationOutcome,
    /// Stable aggregate named by the command.
    pub relaxation_id: RelaxationId,
    /// New version when the command proposed one.
    pub version_id: Option<RelaxationVersionId>,
    /// Aggregate revision after a completed effect.
    pub revision: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms() -> RelaxationTerms {
        let start = Utc::now() + TimeDelta::minutes(1);
        RelaxationTerms {
            subject_identity_id: IdentityId::new(),
            target_scope_id: ScopeId::new(),
            action: RelaxationAction::KnowledgeRead,
            max_sensitivity: Sensitivity::Internal,
            requested_start_at: start,
            requested_end_at: start + TimeDelta::hours(1),
            reason: "joint incident review".to_owned(),
        }
    }

    #[test]
    fn action_is_closed_and_round_trips() {
        let action: RelaxationAction = "knowledge.read".parse().expect("known action");
        assert_eq!(action, RelaxationAction::KnowledgeRead);
        let encoded = serde_json::to_string(&action).expect("encode action");
        assert_eq!(encoded, r#""knowledge.read""#);
        assert_eq!(
            serde_json::from_str::<RelaxationAction>(&encoded).expect("decode action"),
            action
        );
        assert!(serde_json::from_str::<RelaxationAction>(r#""knowledge_read""#).is_err());
        assert!("memory.read".parse::<RelaxationAction>().is_err());
        assert!("policy.assign".parse::<RelaxationAction>().is_err());
    }

    #[test]
    fn terms_are_bounded_and_hash_every_field() {
        let valid = terms();
        valid.validate().expect("valid terms");
        let first = valid.content_hash().expect("hash");
        let changed = RelaxationTerms {
            reason: "another exact reason".to_owned(),
            ..valid
        };
        assert_ne!(first, changed.content_hash().expect("changed hash"));

        let mut too_long = changed;
        too_long.requested_end_at = too_long.requested_start_at
            + TimeDelta::seconds(i64::from(PRODUCT_MAX_RELAXATION_SECS) + 1);
        assert!(too_long.validate().is_err());
    }

    #[test]
    fn status_and_grant_follow_the_immutable_window() {
        let now = Utc::now();
        let terms = RelaxationTerms {
            requested_start_at: now - TimeDelta::minutes(1),
            requested_end_at: now + TimeDelta::minutes(1),
            ..terms()
        };
        let version = RelaxationVersion {
            id: RelaxationVersionId::new(),
            tenant_id: TenantId::new(),
            relaxation_id: RelaxationId::new(),
            ordinal: 1,
            proposal_id: ProposalId::new(),
            subject_principal_id: "alice".to_owned(),
            effective_start_at: terms.requested_start_at,
            hard_expires_at: terms.requested_end_at,
            configuration_version_id: None,
            configuration_hash: "0".repeat(64),
            content_hash: terms.content_hash().expect("hash"),
            creator_id: IdentityId::new(),
            approver_ids: Vec::new(),
            auto_applied: true,
            created_at: now,
            terms,
        };
        let aggregate = Relaxation {
            id: version.relaxation_id,
            tenant_id: version.tenant_id,
            governing_scope_id: version.terms.target_scope_id,
            current_version_id: version.id,
            revision: 1,
            created_at: now,
            created_by: version.creator_id,
            updated_at: now,
            updated_by: version.creator_id,
            revoked_at: None,
            revoked_by: None,
            revocation_proposal_id: None,
            revocation_reason: None,
            expiry_recorded_at: None,
        };
        let current = CurrentRelaxation {
            relaxation: aggregate,
            version,
        };
        assert!(current.grants(
            "alice",
            RelaxationAction::KnowledgeRead,
            current.version.terms.target_scope_id,
            Sensitivity::Internal,
            now,
        ));
        assert!(!current.grants(
            "bob",
            RelaxationAction::KnowledgeRead,
            current.version.terms.target_scope_id,
            Sensitivity::Internal,
            now,
        ));
        assert!(!current.grants(
            "alice",
            RelaxationAction::KnowledgeRead,
            current.version.terms.target_scope_id,
            Sensitivity::Confidential,
            now,
        ));
        assert_eq!(
            current
                .relaxation
                .status_at(&current.version, current.version.hard_expires_at),
            RelaxationStatus::Expired
        );
    }
}
