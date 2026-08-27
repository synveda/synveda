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
    ApprovalMatrix, CompositionConfig, RedactionConfig, ScopeId, SkillQualityConfig,
    SkillScanConfig, TenantId,
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
/// for "what does a pack configure": redaction at external-text admission,
/// the authored-context budget, VedaFlow approvals, and Skill scan/quality
/// thresholds. Runtime capture, freshness and context behaviour live in
/// governed Configuration artifacts rather than in this policy-pack row.
///
/// Every field is optional because a stored pack may configure none of
/// them, and each has its own fail-safe default resolved downstream:
/// strict redaction, the product authored-context config, the empty approval
/// matrix (which still resolves to the invariant floor), and the Skill gates.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackConfig {
    /// The pack's redaction configuration.
    pub redaction: Option<RedactionConfig>,
    /// The pack's composition configuration.
    pub composition: Option<CompositionConfig>,
    /// The pack's approval matrix.
    pub approvals: Option<ApprovalMatrix>,
    /// The severity at which a skill bundle's security scan refuses.
    ///
    /// Its fail-safe is the invariant floor (`critical` blocks and
    /// nothing else does) rather than the strict pack's threshold: a
    /// pack that says nothing must not start refusing bundles nobody
    /// asked it to, and the band that matters holds either way
    /// (ADR-0052 decision 9).
    pub scan: Option<SkillScanConfig>,
    /// The bar a skill bundle's quality must clear to publish without an
    /// override.
    ///
    /// Its fail-safe is the **opposite** of `scan`'s, and the difference
    /// is the whole distinction between the two gates: an unconfigured
    /// pack gates nothing here, because quality is not an invariant and
    /// there is no floor to hold. A pack that has said nothing about
    /// quality has not asked for a quality gate, and a product that began
    /// refusing publications on a rubric nobody opted into would break
    /// every tenant on an upgrade (ADR-0053 decision 9).
    pub quality: Option<SkillQualityConfig>,
}
