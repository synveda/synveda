//! The context read boundary. Knowledge lexical, semantic and graph
//! retrieval lives on immutable Knowledge revisions in the store/gateway;
//! this crate plans and composes the separately governed authored context
//! families (context packs and Skill advertisements) under a token budget.
//!
//! The crate also carries the read path's readiness probe — the "core"
//! leg of the gateway→core→store trace (FND-5, ADR-0007).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod authz;
pub mod compose;

pub use authz::{
    AuthoredReadInputs, CandidateScope, CompositionPlan, ScopeDecision, composition_plan,
};
pub use compose::{
    AUTHORED_SUMMARY_TOKENS, AdvertisedSkill, COMPOSED_ENTRIES_TOTAL, ChannelWatermark,
    ComposeRequest, ComposeScope, ComposedBlock, ComposedEntry, MAX_ADVERTISED_SKILLS,
    SKILL_INDEX_TOKENS, compose_authored, estimated_tokens,
};

use sqlx::PgPool;
use synveda_types::Result;

/// Histogram: estimated tokens per authored-context block.
pub const TOKENS_PER_CONTEXT_RUN: &str = "synveda_tokens_per_context_run";

/// Verifies the read path can reach its storage backend. Ops-plane only: no
/// domain rows are read, so nothing here needs (or may bypass) the PDP.
#[tracing::instrument(name = "retrieval.readiness", skip_all, err(Display))]
pub async fn readiness(pool: &PgPool) -> Result<()> {
    synveda_store::ping(pool).await
}
