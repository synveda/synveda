//! CTX-6 exact evidence for bounded, declared working state (ADR-0150).

use serde::{Deserialize, Serialize};

use crate::{
    ContextRunId, ImportArtifactId, KnowledgeItemId, KnowledgeRevisionId, RepositoryId,
    SessionEventId, SessionId,
};

/// Maximum statements in one replacement snapshot.
pub const MAX_TASK_STATEMENTS: usize = 32;
/// Maximum UTF-8 bytes copied for one statement.
pub const MAX_TASK_STATEMENT_BYTES: usize = 1024;

/// A caller's classification of evidenced text, never a policy instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatementKind {
    /// Desired result.
    Objective,
    /// Declared constraint; current policy and newer instructions still govern.
    Constraint,
    /// Earlier decision.
    Decision,
    /// An unapproved proposal.
    Proposal,
    /// Reported observation.
    Observation,
    /// Work with a separately declared outcome.
    Work,
    /// Unresolved question.
    Question,
    /// Suggested next action, granting no permission to execute it.
    NextAction,
    /// Artifact identity or revision.
    Artifact,
    /// A reported check; ordinary client reports are unverified.
    Verification,
}

/// Declared work status. A successful report is not authenticated verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    /// Intent only.
    Planned,
    /// An attempt is reported.
    Attempted,
    /// Completion is reported by the labelled source.
    Succeeded,
    /// Failure is reported by the labelled source.
    Failed,
    /// Outcome is unknown; this never authorizes repeating an external effect.
    Unknown,
}

/// Closed string selectors in already admitted event payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskTextField {
    /// Top-level `text`.
    Text,
    /// One tool's `text` within a bounded `tools` array.
    ToolText,
}

/// Exact UTF-8 byte boundaries in the redacted source, checked before copying.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskTextSpan {
    /// Inclusive first byte.
    pub start: u32,
    /// Exclusive last byte.
    pub end: u32,
}

/// Immutable source pointer; no caller supplies copied text or provenance class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source_kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskEvidenceRef {
    /// An event in the TaskState's own Session.
    SessionEvent {
        /// Exact server event.
        event_id: SessionEventId,
        /// Hash of the admitted, redacted payload.
        payload_hash: String,
        /// Closed selector.
        field: TaskTextField,
        /// Required only for `tool_text`, at most 31.
        tool_index: Option<u8>,
        /// Exact source byte range.
        span: TaskTextSpan,
    },
    /// Exact current reviewed Knowledge body.
    KnowledgeRevision {
        /// Stable item.
        item_id: KnowledgeItemId,
        /// Exact current revision.
        revision_id: KnowledgeRevisionId,
        /// Immutable revision hash.
        content_hash: String,
        /// Body byte range.
        span: TaskTextSpan,
    },
    /// Actual admitted ImportArtifact body.
    ImportArtifact {
        /// Exact artifact, never a caller-invented upload identity.
        artifact_id: ImportArtifactId,
        /// Immutable source hash.
        content_hash: String,
        /// Body byte range.
        span: TaskTextSpan,
    },
}

/// The source's epistemic class, assigned by the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskEvidenceClass {
    /// Admitted user statement.
    UserDeclared,
    /// Admitted assistant/tool/client report, without host attestation.
    ClientReported,
    /// Current governed Knowledge; its verification status is not invented.
    PublishedKnowledge,
    /// Admitted source bytes, not instructions or verified outcomes.
    ImportedSource,
}

/// One server-copied statement with explicit declared semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskStateStatement {
    /// Declared semantic category.
    pub kind: TaskStatementKind,
    /// Declared outcome, if relevant.
    pub declared_outcome: Option<TaskOutcome>,
    /// Exact source address/hash/span.
    pub evidence: TaskEvidenceRef,
    /// Exact copied bytes, never independent caller prose.
    pub text: String,
    /// Server-derived provenance.
    pub evidence_class: TaskEvidenceClass,
}

/// Content-free address of a prior context snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskContextSnapshot {
    /// Exact saved run in the same Session.
    pub context_run_id: ContextRunId,
    /// Immutable rendered block hash.
    pub block_hash: String,
}

/// Source address for a labelled checkout observation in an admitted event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskCheckoutRef {
    /// Same-Session source event containing `checkout`.
    pub event_id: SessionEventId,
    /// Exact source payload hash.
    pub payload_hash: String,
}

/// Client-observed checkout identity. Missing fields stay unknown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskCheckout {
    /// Exact source address; observation is not authenticated execution proof.
    pub source: TaskCheckoutRef,
    /// Installation-local opaque worktree identity.
    pub worktree_ref: String,
    /// Observed branch.
    pub branch: Option<String>,
    /// Observed immutable commit.
    pub commit: Option<String>,
    /// Observed immutable tree, when exposed.
    pub tree: Option<String>,
    /// Reported dirty state; omission is unknown.
    pub dirty: Option<bool>,
    /// Exact dirty-byte digest when the client supplies one.
    pub dirty_digest: Option<String>,
}

/// Server-derived replacement working state in the immutable Session ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskState {
    /// Closed producer version.
    pub schema_version: u8,
    /// Exact deterministic method.
    pub method: String,
    /// Request identity, used to refuse changed event retries.
    pub request_hash: String,
    /// Actual repository attachment of the source Session, when present.
    pub repository_id: Option<RepositoryId>,
    /// Latest replacement, not an accumulation of superseded objectives.
    pub statements: Vec<TaskStateStatement>,
    /// Independently admitted prior context snapshot.
    pub context_snapshot: Option<TaskContextSnapshot>,
    /// Labelled checkout observation.
    pub checkout: Option<TaskCheckout>,
}

/// Separate cross-Session handoff; receiver-local restart fields stay local.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionHandoff {
    /// Independently authorized source Session.
    pub source_session_id: SessionId,
    /// Server TaskState or verified Checkpoint.
    pub snapshot_event_id: SessionEventId,
    /// Exact admitted snapshot hash.
    pub payload_hash: String,
}

/// Exact TaskState incorporated by checkpoint method v2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointTaskState {
    /// Server TaskState in this same Session.
    pub event_id: SessionEventId,
    /// Immutable TaskState payload hash.
    pub payload_hash: String,
}
