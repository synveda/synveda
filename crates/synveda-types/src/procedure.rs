//! SKIL-6 bounded outcome evidence and frozen Procedure qualification.
use crate::knowledge::KnowledgeRevisionContent;
use crate::{
    ContextRunId, IdentityId, KnowledgeItemId, KnowledgeRevisionId, RepositoryId, SessionEventId,
    SkillVersionId,
};
use serde::{Deserialize, Serialize};

/// Closed task report, preserving unknown external effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservedTaskOutcome {
    /// Completion is reported by the labelled source.
    Succeeded,
    /// Failure is reported by the labelled source.
    Failed,
    /// No established terminal result; never permission to repeat an effect.
    Unknown,
}

/// Server-derived provenance. Authentication alone does not establish causality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeEvidenceClass {
    /// Ordinary caller assertion, without authenticated execution custody.
    ClientReported,
    /// Observation from an explicitly allowed registered service.
    AuthenticatedRunner,
}

/// Closed controlled-case observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseOutcome {
    /// The runner reports the fixed case passing.
    Passed,
    /// The runner reports the fixed case failing.
    Failed,
    /// The fixed case has no known terminal result.
    Unknown,
}

/// Exact already admitted event supporting an observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcedureEventRef {
    /// Exact server event address.
    pub event_id: SessionEventId,
    /// Exact admitted redacted event digest.
    pub payload_hash: String,
}

/// Exact Procedure version involved, without attributing success to inclusion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcedureRevisionRef {
    /// Stable Procedure aggregate.
    pub item_id: KnowledgeItemId,
    /// Exact immutable Procedure revision.
    pub revision_id: KnowledgeRevisionId,
    /// Canonical content digest.
    pub content_hash: String,
}

/// Exact delivered context involved in a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcedureContextRef {
    /// Exact saved delivery involved.
    pub context_run_id: ContextRunId,
    /// Exact saved rendered-block digest.
    pub block_hash: String,
}

/// Bounded reported execution identity; unknown fields stay absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcedureEnvironment {
    /// Bounded caller-reported task class.
    pub task_class: String,
    /// Bounded runner/environment label.
    pub environment_id: String,
    /// Exact reported environment digest.
    pub environment_digest: String,

    /// Reported immutable checkout commit, when known.
    pub commit: Option<String>,
    /// Reported checkout tree, when known.
    pub tree: Option<String>,
    /// Reported exact dirty-byte digest, when known.
    pub dirty_digest: Option<String>,
}

/// Reserved immutable Session outcome, authored only by the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcedureOutcome {
    /// Closed producer schema version.
    pub schema_version: u8,
    /// Canonical bounded request digest.
    pub request_hash: String,

    /// Exact Procedure involved; inclusion does not establish causality.
    pub procedure: ProcedureRevisionRef,
    /// Exact delivered context, when supplied.
    pub context: Option<ProcedureContextRef>,

    /// Exact Skill versions involved; this grants no activation authority.
    pub skill_versions: Vec<SkillVersionId>,
    /// Session-derived repository attachment, when present.
    pub repository_id: Option<RepositoryId>,

    /// Reported execution environment.
    pub environment: ProcedureEnvironment,
    /// Closed observed task status.
    pub outcome: ObservedTaskOutcome,

    /// Server-derived report provenance.
    pub evidence_class: OutcomeEvidenceClass,
    /// Authenticated registered identity responsible for the report.
    pub reporter_id: IdentityId,

    /// At most eight exact same-Session supporting events.
    pub supporting_events: Vec<ProcedureEventRef>,
    /// Digest of retained external runner result evidence.
    pub result_digest: String,
}

/// Exact small append, captured through the deterministic worker path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcedureImprovement {
    /// Closed producer schema version.
    pub schema_version: u8,
    /// Canonical bounded request digest.
    pub request_hash: String,
    /// Exact outcome/target/patch deduplication identity.
    pub identity_hash: String,

    /// Exact Procedure involved; inclusion does not establish causality.
    pub procedure: ProcedureRevisionRef,
    /// At most eight exact supporting outcome receipts.
    pub outcomes: Vec<ProcedureEventRef>,

    /// At most 1024 UTF-8 bytes appended to the current Procedure.
    pub proposed_append: String,
    /// Bounded explicit uncertainty for reviewers.
    pub uncertainty: String,
    /// Development case label, distinct from held-out qualification.
    pub development_case: String,

    /// Frozen complete proposed content.
    pub content: KnowledgeRevisionContent,
    /// Canonical content digest.
    pub content_hash: String,
}

/// Predeclared controlled case identity; answers remain with the runner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcedureEvaluationCase {
    /// Stable predeclared case label.
    pub case_id: String,
    /// Exact predeclared case digest.
    pub digest: String,
    /// Whether passing this case is mandatory.
    pub required: bool,
}

/// Approved independently executed suite. This configuration only narrows Cedar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcedureEvaluationSuite {
    /// Stable governed suite label.
    pub suite_id: String,
    /// Registered service selected for independent qualification.
    pub evaluator_id: IdentityId,
    /// Approved controlled runner label.
    pub harness: String,

    /// Approved controlled runner version.
    pub harness_version: String,
    /// Exact fixed evaluation configuration digest.
    pub configuration_digest: String,

    /// At most 32 sorted unique predeclared cases.
    pub cases: Vec<ProcedureEvaluationCase>,
}

/// Opt-in governed custody for authenticated reporters and fixed quality suites.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcedureLearningConfiguration {
    /// At most eight sorted unique registered reporter identities.
    pub reporters: Vec<IdentityId>,
    /// At most eight sorted unique approved suites.
    pub evaluation_suites: Vec<ProcedureEvaluationSuite>,
}

/// One case result bound to its frozen identity, including the baseline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcedureCaseResult {
    /// Stable predeclared case label.
    pub case_id: String,
    /// Exact predeclared case digest.
    pub digest: String,
    /// Observed result for the unchanged baseline.
    pub baseline: CaseOutcome,

    /// Observed result for the exact frozen candidate.
    pub candidate: CaseOutcome,
    /// Digest of retained external runner result evidence.
    pub result_digest: String,
}

/// Fixed digest grammar shared by bounded evidence/configuration validation.
pub fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
}

impl ProcedureLearningConfiguration {
    /// Reject ambiguous or unbounded reporter/suite custody.
    pub fn validate(&self) -> crate::Result<()> {
        let label = |s: &str| {
            !s.is_empty()
                && s.len() <= 128
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        };
        if self.reporters.is_empty()
            || self.reporters.len() > 8
            || self.reporters.windows(2).any(|p| p[0] >= p[1])
            || self.evaluation_suites.is_empty()
            || self.evaluation_suites.len() > 8
            || self
                .evaluation_suites
                .windows(2)
                .any(|p| p[0].suite_id >= p[1].suite_id)
        {
            return Err(crate::Error::Invalid {
                message: "Procedure learning requires bounded sorted unique reporters and suites"
                    .to_owned(),
            });
        }
        for suite in &self.evaluation_suites {
            if !self.reporters.contains(&suite.evaluator_id)
                || !label(&suite.suite_id)
                || !label(&suite.harness)
                || !label(&suite.harness_version)
                || !valid_digest(&suite.configuration_digest)
                || suite.cases.is_empty()
                || suite.cases.len() > 32
                || !suite.cases.iter().any(|c| c.required)
                || suite.cases.windows(2).any(|p| p[0].case_id >= p[1].case_id)
                || suite
                    .cases
                    .iter()
                    .any(|c| !label(&c.case_id) || !valid_digest(&c.digest))
            {
                return Err(crate::Error::Invalid {
                    message: "Invalid fixed Procedure evaluation suite".to_owned(),
                });
            }
        }
        Ok(())
    }
}
