//! Policy pack application (seed §6, AUTHZ-2, ADR-0014).
//!
//! Packs themselves live where they are authored (embedded in the policy
//! crate, or as tenant rows in the store); what is shared across crates is
//! the *assignment* — which pack a hierarchy node runs. Assignments are
//! request-time data: the store reads them, the gateway carries them, and
//! the PDP resolves the effective pack nearest-ancestor-first from them.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{ScopeId, TenantId};

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
