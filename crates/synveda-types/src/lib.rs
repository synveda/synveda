//! Domain types, identifiers, and the error taxonomy shared by every Synveda crate.
//!
//! This crate is the root of the workspace dependency graph and must never depend
//! on another `synveda-*` crate (seed §8; enforced by `scripts/check-crate-deps.mjs`).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

// Membership and access assignment over governed scopes (CPR-5, ADR-0072).
// Public as a module for `scope`'s and `workspace`'s reason: `RoleKey` here
// is the one role vocabulary (CPR-7, ADR-0074 decision 6), and the module
// path keeps grant keys, approval roles and Cedar role lists reading as the
// same closed set.
pub mod access;
// Public as a module for the same reason: an "anchor" is a concept of the
// governed scope model (CPR-6, ADR-0073).
pub mod anchor;
mod approval;
mod asset;
mod channel;
mod composition;
mod dedup;
mod directory;
mod error;
mod graph;
mod id;
mod identity;
// Stable Knowledge aggregates and immutable revisions (CPR-15, ADR-0080).
// Public because these are the product's primary content vocabulary and the
// store, command, gateway and generated API layers must share one copy.
pub mod knowledge;
// Canonical JSON (CPR-10). Public as a module because both callers hash a
// **caller-supplied** object and neither is in this crate: the gateway's
// idempotency digest and the session ledger's payload hash.
pub mod json;
mod lapse;
mod mover;
// Reusable durable operation vocabulary (CPR-16, ADR-0081).
pub mod operation;
mod pack;
mod policy;
mod promotion;
mod prompt;
mod proposal;
mod quarantine;
mod record;
mod redaction;
// Canonical repository identity (CPR-4, ADR-0071 decision 4). Public as a
// module so `repository::identify` reads as what it is at every call site —
// the one place two clients agree on what "the same repository" means.
pub mod repository;
mod retention;
// The governed scope model (CPR-3, ADR-0070) — public as a module because
// `ScopeKind` and `Scope` are the tree every other type hangs off. Until the
// hierarchy cutover (CPR-7, ADR-0074) a second `ScopeKind` lived at the
// root; there is one tree and one shape vocabulary now, and the module path
// still says which model a caller is written against.
pub mod scope;
mod sensitivity;
// The session ledger (CPR-10, ADR-0076). Public as a module for
// `workspace`'s reason: a "session" is an ordinary word, and the module path
// is what says this one is an agent run recorded by Synveda rather than a
// login session, a console session or an HTTP one.
pub mod session;
mod skill;
mod skillquality;
mod skillscan;
mod tenant;
// The two product-level subtypes of a governed scope (CPR-4, ADR-0071).
// Public as a module for `repository`'s reason and one of its own: `Project`
// and `Workspace` are ordinary nouns, and the module path is what says these
// are Synveda's rather than somebody else's.
pub mod workspace;

pub use approval::{
    ApprovalMatrix, ApprovalRequirement, ApprovalRule, CastApproval, Outstanding, RequiredAudit,
    RequirementAudit, RequirementOrigin, RoleAudit, RoleRequirement,
};
pub use asset::AssetKind;
pub use channel::Channel;
pub use composition::{
    CompositionConfig, DEFAULT_INDEX_ENTRY_CHARS, EntryTier, IndexTier, InjectChannels, SkillIndex,
};
pub use dedup::{DedupConfig, DedupMode, MAX_DEDUP_NEIGHBOURS, permille};
pub use directory::{DirectoryGroup, DirectoryUser, ScimCredential};
pub use error::{Error, Result};
pub use graph::{Depth, Graph};
pub use id::{
    ContextRunId, DirectoryGroupId, DirectoryUserId, DurableOperationId, GrantId, GraphEdgeId,
    GraphVertexId, GroupId, IdentityId, InviteId, KnowledgeItemId, KnowledgeRelationId,
    KnowledgeRevisionId, KnowledgeSourceId, LapseId, ProjectId, ProposalId, RecordId, RepositoryId,
    ScimCredentialId, ScopeId, SessionEventId, SessionId, TenantId, WorkspaceId,
};
pub use identity::{Identity, IdentityKind, IdentityStatus};
pub use lapse::{
    Lapse, LapseAction, LapseConfig, LapseOutcome, LapseTerms, MAX_LAPSE_REASON,
    PRODUCT_MAX_DURATION_SECS, STRICT_MAX_DURATION_SECS,
};
pub use mover::{MoverConfig, PersonalMemory};
pub use pack::{
    CHUNK_CHARS, ContextPackChannel, ContextPackName, DocumentChunk, DocumentName, DocumentPath,
    MAX_DOCUMENT_CHARS, MAX_DOCUMENT_CHUNKS, MAX_DOCUMENT_NAME_CHARS, MAX_DOCUMENT_NAME_SEGMENTS,
    MAX_DOCUMENT_TITLE_CHARS, MAX_PACK_DESCRIPTION_CHARS, MAX_PACK_DOCUMENTS, MAX_PACK_NAME_CHARS,
    MAX_PACK_SEGMENT_CHARS, PackDocument, chunk,
};
pub use policy::{PackConfig, PolicyAssignment};
pub use promotion::{
    MAX_PROMOTION_RULES, MAX_RULE_NAME, MemberEvidence, PromotionConfig, PromotionEvidence,
    PromotionRule, UsageFacts,
};
pub use prompt::{
    MAX_DEFAULT_CHARS, MAX_DESCRIPTION_CHARS, MAX_NAME_CHARS, MAX_NAME_SEGMENTS, MAX_SEGMENT_CHARS,
    MAX_TEMPLATE_CHARS, MAX_VARIABLES, PromptChannel, PromptName, PromptTemplate, PromptVariable,
};
pub use proposal::{ProposalEffect, ProposalState, ProposalView, Verdict};
pub use quarantine::QuarantineState;
pub use record::{RecordClass, RecordKind};
pub use redaction::{RedactionConfig, RedactionMode};
pub use retention::{
    ClassTtl, MAX_RETENTION_DAYS, MIN_STAGING_DAYS, RetentionConfig, RetentionMode,
};
pub use sensitivity::{ScopeTier, Sensitivity};
pub use skill::{
    Frontmatter, MAX_FRONTMATTER_ENTRIES, MAX_FRONTMATTER_VALUE_CHARS, MAX_SKILL_BUNDLE_CHARS,
    MAX_SKILL_DESCRIPTION_CHARS, MAX_SKILL_FILE_CHARS, MAX_SKILL_FILES, MAX_SKILL_NAME_CHARS,
    MAX_SKILL_PATH_CHARS, MAX_SKILL_PATH_SEGMENT_CHARS, MAX_SKILL_PATH_SEGMENTS, SKILL_MANIFEST,
    SkillBundle, SkillChannel, SkillFile, SkillFilePath, SkillName, SkillPath,
};
pub use skillquality::{
    Checklist, ChecklistItem, ChecklistVerdict, MAX_CHECKLIST_NOTE_CHARS, QualityShortfall,
    SkillQualityConfig,
};
pub use skillscan::{ScanSeverity, SkillScanConfig};
pub use tenant::{Tenant, TenantStatus};
