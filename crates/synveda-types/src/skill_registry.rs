//! Stable, immutable Agent Skill catalogue vocabulary (CPR-23, ADR-0085).
//!
//! The external bundle grammar remains in [`crate::skill`]. This module is
//! the product model around those bytes: a stable skill id, immutable version
//! ids, revisioned scope bindings, exact usage evidence and controlled test
//! runs. Tool declarations deliberately do not appear in any authority type;
//! they remain manifest metadata.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Error, IdentityId, ProposalId, Result, ScopeId, Sensitivity, SkillBindingId, SkillFilePath,
    SkillId, SkillName, SkillVersionId,
};

/// The official Agent Skills specification snapshot exercised by this build.
///
/// The specification publishes no numbered release. Pinning the source commit
/// is therefore more precise than inventing a protocol version string.
pub const AGENT_SKILLS_SPEC_COMMIT: &str = "69ef37e9424c0a7ea9dd2293b559e43ec8176379";

/// Date on which [`AGENT_SKILLS_SPEC_COMMIT`] was verified against the public
/// specification.
pub const AGENT_SKILLS_SPEC_VERIFIED_AT: &str = "2026-08-24";

/// Maximum provenance reference length.
pub const MAX_SKILL_SOURCE_REFERENCE_CHARS: usize = 2_048;

/// Maximum client event id length for idempotent usage recording.
pub const MAX_SKILL_USAGE_CLIENT_EVENT_ID_CHARS: usize = 200;

/// Where a bundle entered the catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSourceKind {
    /// Authored through the public API.
    Authored,
    /// Imported from a directory.
    Directory,
    /// Imported from an archive.
    Archive,
    /// Imported from a checked-out Git tree or revision.
    Git,
    /// Imported from an external skill registry.
    Registry,
}

impl SkillSourceKind {
    /// Every stored value, in schema order.
    pub const ALL: [Self; 5] = [
        Self::Authored,
        Self::Directory,
        Self::Archive,
        Self::Git,
        Self::Registry,
    ];

    /// Stable wire/storage spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authored => "authored",
            Self::Directory => "directory",
            Self::Archive => "archive",
            Self::Git => "git",
            Self::Registry => "registry",
        }
    }
}

impl fmt::Display for SkillSourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SkillSourceKind {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == value)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown skill source kind: {value:?}"),
            })
    }
}

/// Provenance retained on one immutable version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillProvenance {
    /// Source class.
    pub kind: SkillSourceKind,
    /// Human-inspectable source reference, without credentials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// Exact source revision when the source provides one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// Forward-compatible, non-secret source metadata.
    #[serde(default)]
    pub metadata: Value,
}

impl Default for SkillProvenance {
    fn default() -> Self {
        Self {
            kind: SkillSourceKind::Authored,
            reference: None,
            revision: None,
            metadata: Value::Object(Default::default()),
        }
    }
}

impl SkillProvenance {
    /// Validate bounded, object-shaped provenance.
    pub fn validate(&self) -> Result<()> {
        for (field, value) in [
            ("reference", self.reference.as_deref()),
            ("revision", self.revision.as_deref()),
        ] {
            if let Some(value) = value
                && (value.trim().is_empty()
                    || value.chars().count() > MAX_SKILL_SOURCE_REFERENCE_CHARS)
            {
                return Err(Error::Invalid {
                    message: format!(
                        "skill provenance {field} must contain 1..={MAX_SKILL_SOURCE_REFERENCE_CHARS} characters"
                    ),
                });
            }
        }
        if !self.metadata.is_object() {
            return Err(Error::Invalid {
                message: "skill provenance metadata must be a JSON object".to_owned(),
            });
        }
        let bytes = serde_json::to_vec(&self.metadata).map_err(|err| Error::Invalid {
            message: format!("encode skill provenance metadata: {err}"),
        })?;
        if bytes.len() > 16 * 1024 {
            return Err(Error::Invalid {
                message: "skill provenance metadata exceeds 16384 bytes".to_owned(),
            });
        }
        Ok(())
    }
}

/// One immutable version file as a governed change names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillVersionFileRef {
    /// Relative bundle path.
    pub path: SkillFilePath,
    /// VedaFlow object address, lowercase hexadecimal.
    pub object_hash: String,
    /// Character count of the external UTF-8 file.
    pub chars: u32,
}

/// A typed skill effect carried by a VedaFlow `apply` proposal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum SkillCommand {
    /// Create a stable aggregate and its first immutable version.
    Install {
        /// Pre-minted stable aggregate id.
        skill_id: SkillId,
        /// Pre-minted immutable version id.
        version_id: SkillVersionId,
        /// Scope governing the catalogue entry.
        governing_scope_id: ScopeId,
        /// Tenant-unique external bundle name.
        name: SkillName,
        /// Bundle sensitivity.
        sensitivity: Sensitivity,
        /// Stable digest over path/object-address pairs.
        bundle_digest: String,
        /// Parsed manifest, including extension metadata.
        manifest: Value,
        /// Exact immutable file references.
        files: Vec<SkillVersionFileRef>,
        /// Bundle provenance.
        provenance: SkillProvenance,
        /// Retained content-free scan evidence.
        scan: Value,
        /// Scanner ruleset version.
        scan_ruleset_version: u32,
        /// Automated quality score.
        quality_score: u8,
        /// Rubric version producing the score.
        rubric_version: u32,
    },
    /// Add a new immutable version and advance the aggregate current pointer.
    Update {
        /// Stable aggregate.
        skill_id: SkillId,
        /// Exact current version required when the effect runs.
        expected_current_version_id: SkillVersionId,
        /// Pre-minted new immutable version.
        version_id: SkillVersionId,
        /// Governing scope, repeated in the payload-integrity boundary.
        governing_scope_id: ScopeId,
        /// External bundle name.
        name: SkillName,
        /// Bundle sensitivity.
        sensitivity: Sensitivity,
        /// Stable digest over path/object-address pairs.
        bundle_digest: String,
        /// Parsed manifest, including extension metadata.
        manifest: Value,
        /// Exact immutable file references.
        files: Vec<SkillVersionFileRef>,
        /// Bundle provenance.
        provenance: SkillProvenance,
        /// Retained content-free scan evidence.
        scan: Value,
        /// Scanner ruleset version.
        scan_ruleset_version: u32,
        /// Automated quality score.
        quality_score: u8,
        /// Rubric version producing the score.
        rubric_version: u32,
    },
    /// Create a project- or principal-scope binding.
    Bind {
        /// Pre-minted binding id.
        binding_id: SkillBindingId,
        /// Bound skill.
        skill_id: SkillId,
        /// Project- or principal-shaped target scope.
        scope_id: ScopeId,
        /// Exact version pin; absent means follow current.
        pinned_version_id: Option<SkillVersionId>,
        /// Whether the binding starts enabled.
        enabled: bool,
    },
    /// Change a binding, including disable, pin, unpin and rollback.
    SetBinding {
        /// Binding being changed.
        binding_id: SkillBindingId,
        /// Target scope, repeated for authorization and integrity.
        scope_id: ScopeId,
        /// Exact revision required when the effect runs.
        expected_revision: u64,
        /// Complete resulting enabled state.
        enabled: bool,
        /// Complete resulting pin state.
        pinned_version_id: Option<SkillVersionId>,
        /// Stable reason code such as `disable`, `pin`, `unpin` or `rollback`.
        reason: String,
    },
}

impl SkillCommand {
    /// Stable command name.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Install { .. } => "install",
            Self::Update { .. } => "update",
            Self::Bind { .. } => "bind",
            Self::SetBinding { .. } => "set_binding",
        }
    }

    /// Scope at which the effect is governed.
    #[must_use]
    pub const fn scope_id(&self) -> ScopeId {
        match self {
            Self::Install {
                governing_scope_id, ..
            }
            | Self::Update {
                governing_scope_id, ..
            } => *governing_scope_id,
            Self::Bind { scope_id, .. } | Self::SetBinding { scope_id, .. } => *scope_id,
        }
    }

    /// Sensitivity used by the approval matrix.
    #[must_use]
    pub const fn sensitivity(&self) -> Sensitivity {
        match self {
            Self::Install { sensitivity, .. } | Self::Update { sensitivity, .. } => *sensitivity,
            Self::Bind { .. } | Self::SetBinding { .. } => Sensitivity::Internal,
        }
    }

    /// Stable aggregate id when this command names one directly.
    #[must_use]
    pub const fn skill_id(&self) -> Option<SkillId> {
        match self {
            Self::Install { skill_id, .. }
            | Self::Update { skill_id, .. }
            | Self::Bind { skill_id, .. } => Some(*skill_id),
            Self::SetBinding { .. } => None,
        }
    }

    /// Version id proposed by an install/update or pinned by a bind.
    #[must_use]
    pub const fn version_id(&self) -> Option<SkillVersionId> {
        match self {
            Self::Install { version_id, .. } | Self::Update { version_id, .. } => Some(*version_id),
            Self::Bind {
                pinned_version_id, ..
            }
            | Self::SetBinding {
                pinned_version_id, ..
            } => *pinned_version_id,
        }
    }

    /// Binding id proposed or changed by this command.
    #[must_use]
    pub const fn binding_id(&self) -> Option<SkillBindingId> {
        match self {
            Self::Bind { binding_id, .. } | Self::SetBinding { binding_id, .. } => {
                Some(*binding_id)
            }
            Self::Install { .. } | Self::Update { .. } => None,
        }
    }
}

/// Outcome returned by a governed skill mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillMutationOutcome {
    /// Effect executed in the opening request.
    Applied,
    /// Change exists and awaits approval.
    PendingReview,
    /// Effect reached a terminal governed refusal.
    Rejected,
}

/// Stable result envelope for all skill mutation commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillMutationResult {
    /// VedaFlow change id.
    pub change_id: ProposalId,
    /// Governance outcome.
    pub outcome: SkillMutationOutcome,
    /// Stable skill id, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_id: Option<SkillId>,
    /// Immutable version id, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_id: Option<SkillVersionId>,
    /// Binding id, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding_id: Option<SkillBindingId>,
    /// Resulting binding revision after application.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding_revision: Option<u64>,
}

/// A stage in a skill's observable lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillUsageStage {
    /// Named to the client/model as available.
    Advertised,
    /// Found through client discovery.
    Discovered,
    /// Selected for use.
    Activated,
    /// `SKILL.md` instructions were loaded.
    InstructionsLoaded,
    /// A bundled resource was loaded.
    ResourceLoaded,
    /// A bundled script was requested.
    ScriptRequested,
    /// A bundled script or procedure executed.
    Executed,
    /// An outcome was reported.
    OutcomeReported,
}

impl SkillUsageStage {
    /// Every stored value, in lifecycle order.
    pub const ALL: [Self; 8] = [
        Self::Advertised,
        Self::Discovered,
        Self::Activated,
        Self::InstructionsLoaded,
        Self::ResourceLoaded,
        Self::ScriptRequested,
        Self::Executed,
        Self::OutcomeReported,
    ];

    /// Stable wire/storage spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Advertised => "advertised",
            Self::Discovered => "discovered",
            Self::Activated => "activated",
            Self::InstructionsLoaded => "instructions_loaded",
            Self::ResourceLoaded => "resource_loaded",
            Self::ScriptRequested => "script_requested",
            Self::Executed => "executed",
            Self::OutcomeReported => "outcome_reported",
        }
    }
}

impl FromStr for SkillUsageStage {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == value)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown skill usage stage: {value:?}"),
            })
    }
}

/// Authority of one usage assertion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillUsageEvidence {
    /// Directly observed by a supported host/client adapter.
    HostObserved,
    /// Asserted by the model or agent without host confirmation.
    ModelReported,
}

impl SkillUsageEvidence {
    /// Every stored value.
    pub const ALL: [Self; 2] = [Self::HostObserved, Self::ModelReported];

    /// Stable wire/storage spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HostObserved => "host_observed",
            Self::ModelReported => "model_reported",
        }
    }
}

impl FromStr for SkillUsageEvidence {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == value)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown skill usage evidence: {value:?}"),
            })
    }
}

/// Controlled harness identity for a skill test run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillTestHarness {
    /// Built-in parse/scan/rubric sandbox. Executes no bundle code.
    ValidationSandbox,
    /// An identified external supported client harness.
    ControlledClient,
}

impl SkillTestHarness {
    /// Every stored value.
    pub const ALL: [Self; 2] = [Self::ValidationSandbox, Self::ControlledClient];

    /// Stable wire/storage spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ValidationSandbox => "validation_sandbox",
            Self::ControlledClient => "controlled_client",
        }
    }
}

impl FromStr for SkillTestHarness {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == value)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown skill test harness: {value:?}"),
            })
    }
}

/// Terminal result of one controlled test run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillTestOutcome {
    /// Every controlled assertion passed.
    Passed,
    /// A controlled assertion failed.
    Failed,
    /// The harness could not complete.
    Error,
}

impl SkillTestOutcome {
    /// Every stored value.
    pub const ALL: [Self; 3] = [Self::Passed, Self::Failed, Self::Error];

    /// Stable wire/storage spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Error => "error",
        }
    }
}

impl FromStr for SkillTestOutcome {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == value)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown skill test outcome: {value:?}"),
            })
    }
}

/// Validate the shared bounded client-event vocabulary.
pub fn validate_skill_usage_client_event_id(value: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.chars().count() > MAX_SKILL_USAGE_CLIENT_EVENT_ID_CHARS
        || value.chars().any(char::is_control)
    {
        return Err(Error::Invalid {
            message: format!(
                "skill usage client_event_id must contain 1..={MAX_SKILL_USAGE_CLIENT_EVENT_ID_CHARS} non-control characters"
            ),
        });
    }
    Ok(())
}

/// The identity fields an append-only usage event must bind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillUsageIdentity {
    /// Binding active when the evidence was observed.
    pub binding_id: SkillBindingId,
    /// Exact immutable version involved.
    pub version_id: SkillVersionId,
    /// Principal reporting or observed by the host.
    pub principal_id: IdentityId,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_vocabularies_round_trip() {
        for value in SkillSourceKind::ALL {
            assert_eq!(value.as_str().parse::<SkillSourceKind>().unwrap(), value);
        }
        for value in SkillUsageStage::ALL {
            assert_eq!(value.as_str().parse::<SkillUsageStage>().unwrap(), value);
        }
        for value in SkillUsageEvidence::ALL {
            assert_eq!(value.as_str().parse::<SkillUsageEvidence>().unwrap(), value);
        }
        for value in SkillTestHarness::ALL {
            assert_eq!(value.as_str().parse::<SkillTestHarness>().unwrap(), value);
        }
        for value in SkillTestOutcome::ALL {
            assert_eq!(value.as_str().parse::<SkillTestOutcome>().unwrap(), value);
        }
    }

    #[test]
    fn the_agent_skills_contract_is_pinned_to_a_real_upstream_snapshot() {
        assert_eq!(
            AGENT_SKILLS_SPEC_COMMIT,
            "69ef37e9424c0a7ea9dd2293b559e43ec8176379"
        );
        assert_eq!(AGENT_SKILLS_SPEC_VERIFIED_AT, "2026-08-24");
        assert_eq!(AGENT_SKILLS_SPEC_COMMIT.len(), 40);
        assert!(
            AGENT_SKILLS_SPEC_COMMIT
                .chars()
                .all(|value| value.is_ascii_hexdigit())
        );
    }

    #[test]
    fn provenance_is_bounded_and_object_shaped() {
        SkillProvenance::default().validate().unwrap();
        let invalid = SkillProvenance {
            metadata: Value::Array(Vec::new()),
            ..SkillProvenance::default()
        };
        assert!(invalid.validate().is_err());
        let invalid = SkillProvenance {
            reference: Some("x".repeat(MAX_SKILL_SOURCE_REFERENCE_CHARS + 1)),
            ..SkillProvenance::default()
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn usage_client_ids_are_bounded() {
        validate_skill_usage_client_event_id("host:session:42").unwrap();
        assert!(validate_skill_usage_client_event_id("").is_err());
        assert!(validate_skill_usage_client_event_id("bad\nvalue").is_err());
    }
}
