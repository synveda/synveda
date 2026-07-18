//! Provisioned identities (seed §5, AUTH-2, ADR-0013).
//!
//! An identity is a token subject that JIT provisioning has placed in the
//! tenancy hierarchy: every user (and later every service identity, AUTH-3)
//! owns a personal user-kind scope node. Subjects the IdP has verified but
//! provisioning has never seen are *not* identities — the PDP treats them
//! as quarantined (ADR-0013 decision 6).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{IdentityId, ScopeId, TenantId};

/// A provisioned identity: a subject bound to its personal scope node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// The identity's own id.
    pub id: IdentityId,
    /// Owning tenant; immutable for the life of the identity.
    pub tenant_id: TenantId,
    /// The verified token subject (`sub`), unique per tenant.
    pub subject: String,
    /// The IdP's `email` claim at provisioning time, if any.
    pub email: Option<String>,
    /// The IdP's `name` claim at provisioning time, if any.
    pub display_name: Option<String>,
    /// The identity's personal scope node (`ScopeKind::User`).
    pub scope_id: ScopeId,
    /// Derived from placement, never stored (ADR-0013 decision 4): the
    /// personal scope sits under the tenant's quarantine scope.
    pub quarantined: bool,
    /// When the identity was provisioned.
    pub created_at: DateTime<Utc>,
}
