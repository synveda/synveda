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
mod error;
mod hierarchy;
mod id;
mod identity;
mod observe;
mod policy;
mod promotion;
mod proposal;
mod record;
mod redaction;
mod role;
mod sensitivity;
mod tenant;

pub use approval::{
    ApprovalMatrix, ApprovalRequirement, ApprovalRule, CastApproval, Outstanding, RequiredAudit,
    RequirementAudit, RequirementOrigin, RoleAudit, RoleRequirement,
};
pub use asset::AssetKind;
pub use channel::Channel;
pub use composition::{CompositionConfig, InjectChannels};
pub use error::{Error, Result};
pub use hierarchy::{HierarchyNode, ScopeKind};
pub use id::{IdentityId, ObserveEventId, ProposalId, RecordId, ScopeId, TenantId};
pub use identity::{Identity, IdentityKind};
pub use observe::{ObserveKind, QuarantineState};
pub use policy::{PackConfig, PolicyAssignment};
pub use promotion::{
    MAX_PROMOTION_RULES, MAX_RULE_NAME, MemberEvidence, PromotionConfig, PromotionEvidence,
    PromotionRule, UsageFacts,
};
pub use proposal::{ProposalState, ProposalView, Verdict};
pub use record::{RecordClass, RecordKind};
pub use redaction::{RedactionConfig, RedactionMode};
pub use role::{Role, RoleBinding};
pub use sensitivity::Sensitivity;
pub use tenant::{Tenant, TenantStatus};
