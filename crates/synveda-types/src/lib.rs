//! Domain types, identifiers, and the error taxonomy shared by every Synveda crate.
//!
//! This crate is the root of the workspace dependency graph and must never depend
//! on another `synveda-*` crate (seed §8; enforced by `scripts/check-crate-deps.mjs`).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod approval;
mod asset;
mod channel;
mod composition;
mod dedup;
mod error;
mod graph;
mod hierarchy;
mod id;
mod identity;
mod lapse;
mod observe;
mod pack;
mod policy;
mod promotion;
mod prompt;
mod proposal;
mod record;
mod redaction;
mod retention;
mod role;
mod sensitivity;
mod skill;
mod skillscan;
mod tenant;

pub use approval::{
    ApprovalMatrix, ApprovalRequirement, ApprovalRule, CastApproval, Outstanding, RequiredAudit,
    RequirementAudit, RequirementOrigin, RoleAudit, RoleRequirement,
};
pub use asset::AssetKind;
pub use channel::Channel;
pub use composition::{
    CompositionConfig, DEFAULT_INDEX_ENTRY_CHARS, EntryTier, IndexTier, InjectChannels,
};
pub use dedup::{DedupConfig, DedupMode, MAX_DEDUP_NEIGHBOURS, permille};
pub use error::{Error, Result};
pub use graph::{Depth, Graph};
pub use hierarchy::{HierarchyNode, ScopeKind};
pub use id::{
    GraphEdgeId, GraphVertexId, IdentityId, LapseId, ObserveEventId, ProposalId, RecordId, ScopeId,
    TenantId,
};
pub use identity::{Identity, IdentityKind};
pub use lapse::{
    Lapse, LapseAction, LapseConfig, LapseOutcome, LapseTerms, MAX_LAPSE_REASON,
    PRODUCT_MAX_DURATION_SECS, STRICT_MAX_DURATION_SECS,
};
pub use observe::{ObserveKind, QuarantineState};
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
pub use record::{RecordClass, RecordKind};
pub use redaction::{RedactionConfig, RedactionMode};
pub use retention::{
    ClassTtl, MAX_RETENTION_DAYS, MIN_STAGING_DAYS, RetentionConfig, RetentionMode,
};
pub use role::{Role, RoleBinding};
pub use sensitivity::{ScopeTier, Sensitivity};
pub use skill::{
    Frontmatter, MAX_FRONTMATTER_ENTRIES, MAX_FRONTMATTER_VALUE_CHARS, MAX_SKILL_BUNDLE_CHARS,
    MAX_SKILL_DESCRIPTION_CHARS, MAX_SKILL_FILE_CHARS, MAX_SKILL_FILES, MAX_SKILL_NAME_CHARS,
    MAX_SKILL_PATH_CHARS, MAX_SKILL_PATH_SEGMENT_CHARS, MAX_SKILL_PATH_SEGMENTS, SKILL_MANIFEST,
    SkillBundle, SkillChannel, SkillFile, SkillFilePath, SkillName, SkillPath,
};
pub use skillscan::{ScanSeverity, SkillScanConfig};
pub use tenant::{Tenant, TenantStatus};
