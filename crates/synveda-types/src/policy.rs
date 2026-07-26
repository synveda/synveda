//! Policy pack application (seed §6, AUTHZ-2, ADR-0014).
//!
//! Packs themselves live where they are authored (embedded in the policy
//! crate, or as tenant rows in the store); what is shared across crates is
//! the *assignment* — which pack a hierarchy node runs. Assignments are
//! request-time data: the store reads them, the gateway carries them, and
//! the PDP resolves the effective pack nearest-ancestor-first from them.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    ApprovalMatrix, CompositionConfig, DedupConfig, LapseConfig, PromotionConfig, RedactionConfig,
    RetentionConfig, ScopeId, TenantId,
};

/// A per-node policy pack assignment: the node (and its subtree, until a
/// deeper assignment) runs `pack_name`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyAssignment {
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// The node the pack is assigned at.
    pub scope_id: ScopeId,
    /// The assigned pack — an embedded product pack or a stored custom
    /// pack of the same tenant.
    pub pack_name: String,
    /// When the assignment was last changed.
    pub updated_at: DateTime<Utc>,
}

/// A policy pack's non-Cedar configuration — everything an engine carries
/// beside its policies.
///
/// One struct rather than a growing parameter list, and one place to look
/// for "what does a pack configure": the redaction modes the observe scan
/// applies (MEM-2, ADR-0021 decision 3), the budget and channel rule the
/// read path composes under (CTX-2, ADR-0025 decisions 2–3), the
/// approvals a publication needs (FLOW-3, ADR-0032 decision 3), the
/// rules that open a promotion proposal without a human deciding to
/// (FLOW-4, ADR-0033 decision 6), the longest window a lapse may run
/// for (AUTHZ-4, ADR-0037 decision 5), what the ingestion pipeline
/// does with a restatement or a contradiction (MEM-5, ADR-0039
/// decision 12), and how long material is served, kept and destroyed
/// (MEM-6, ADR-0040).
///
/// Every field is optional because a stored pack may configure none of
/// them, and each has its own fail-safe default resolved downstream:
/// strict redaction, the product composition config (which only ever
/// narrows), the empty approval matrix — which still resolves to the
/// invariant floor, never to "no review needed" — no promotion rules
/// at all, because an absent trigger must not fire, the strict lapse
/// window, which narrows and never grants, the product dedup config,
/// which removes nothing a reader could otherwise have seen except the
/// facts a newer statement replaced, and the product retention config,
/// whose record horizons are all unset — a pack that configures nothing
/// must not start destroying memory (MEM-6, ADR-0040 decision 13).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackConfig {
    /// The pack's redaction configuration.
    pub redaction: Option<RedactionConfig>,
    /// The pack's composition configuration.
    pub composition: Option<CompositionConfig>,
    /// The pack's approval matrix.
    pub approvals: Option<ApprovalMatrix>,
    /// The pack's auto-promotion rules.
    pub promotion: Option<PromotionConfig>,
    /// The pack's lapse ceiling.
    pub lapse: Option<LapseConfig>,
    /// The pack's dedup and conflict-detection configuration.
    pub dedup: Option<DedupConfig>,
    /// The pack's retention, disposal and staleness configuration.
    pub retention: Option<RetentionConfig>,
}
