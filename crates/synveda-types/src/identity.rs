//! Provisioned identities (seed §5, AUTH-2, ADR-0013; AUTH-3, ADR-0018).
//!
//! An identity is a token subject placed in the tenancy hierarchy: every
//! user and every service identity owns a personal user-kind scope node.
//! Users arrive through JIT provisioning at first login; service
//! identities are registered explicitly (headless agents never log in).
//! Subjects the IdP has verified but neither path has seen are *not*
//! identities — the PDP treats them as quarantined (ADR-0013 decision 6).

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{Error, IdentityId, ScopeId, TenantId};

/// What stands behind an identity's subject (AUTH-3, ADR-0018 decision 2).
/// The kind decides the enforcement seam's semantics: service identities
/// carry a token-scope confinement and a token-lifetime cap; users do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IdentityKind {
    /// A person, JIT-provisioned at first login (AUTH-2).
    User,
    /// A headless agent authenticating via client credentials (AUTH-3).
    Service,
}

impl IdentityKind {
    /// Stable machine-readable name — the `identities.kind` column value.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            IdentityKind::User => "user",
            IdentityKind::Service => "service",
        }
    }
}

impl fmt::Display for IdentityKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for IdentityKind {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "user" => Ok(IdentityKind::User),
            "service" => Ok(IdentityKind::Service),
            other => Err(Error::Invalid {
                message: format!("unknown identity kind {other:?}"),
            }),
        }
    }
}

/// Where an identity is in its lifecycle (AUTH-4, ADR-0059 decision 7).
///
/// The one piece of lifecycle state this product stores rather than
/// derives. ADR-0013 decision 4 refused a `quarantined` column because
/// placement already answered that question and a second copy would
/// drift; departure is answered by nothing else in the schema, so it is
/// stored — once, here — and a *scope's* sealed-ness derives from it
/// through the one-personal-scope-per-identity constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IdentityStatus {
    /// The identity may authenticate and its personal scope is live.
    Active,
    /// The directory said this person is gone. The token stops working at
    /// the enforcement seam, the personal scope is unreadable by everyone
    /// under a base-layer forbid, and the retention sweep will not
    /// dispose of what it holds (ADR-0059 decision 8). Not reversible:
    /// a rehire is a new identity and a new scope (decision 12).
    Departed,
}

impl IdentityStatus {
    /// Stable machine-readable name — the `identities.status` column value.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            IdentityStatus::Active => "active",
            IdentityStatus::Departed => "departed",
        }
    }
}

impl fmt::Display for IdentityStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for IdentityStatus {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "active" => Ok(IdentityStatus::Active),
            "departed" => Ok(IdentityStatus::Departed),
            other => Err(Error::Invalid {
                message: format!("unknown identity status {other:?}"),
            }),
        }
    }
}

/// A provisioned identity: a subject bound to its personal scope node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// The identity's own id.
    pub id: IdentityId,
    /// Owning tenant; immutable for the life of the identity.
    pub tenant_id: TenantId,
    /// The verified token subject (`sub`), unique per tenant — `None` for
    /// an identity a directory created before its person ever logged in
    /// (AUTH-4, ADR-0059 decision 5). An identity with no subject
    /// authenticates nothing and can take no decision: every caller seam
    /// resolves by subject, so an unbound row is unreachable rather than
    /// specially handled.
    pub subject: Option<String>,
    /// User or service (AUTH-3, ADR-0018 decision 2).
    pub kind: IdentityKind,
    /// The IdP's `email` claim at provisioning time, if any.
    pub email: Option<String>,
    /// The IdP's `name` claim at provisioning time, if any.
    pub display_name: Option<String>,
    /// The identity's personal scope node (`ScopeKind::User`).
    pub scope_id: ScopeId,
    /// Derived from placement, never stored (ADR-0013 decision 4): the
    /// personal scope sits under the tenant's quarantine scope.
    pub quarantined: bool,
    /// Active, or departed and therefore sealed (ADR-0059 decision 7).
    pub status: IdentityStatus,
    /// When the directory said this person left; `Some` exactly when
    /// `status` is [`IdentityStatus::Departed`] (the schema refuses any
    /// other pairing).
    pub departed_at: Option<DateTime<Utc>>,
    /// When the identity was provisioned.
    pub created_at: DateTime<Utc>,
}

impl Identity {
    /// Whether this identity's personal scope is sealed — the derivation
    /// ADR-0059 decision 7 keeps in one place so that no caller invents a
    /// second one.
    #[must_use]
    pub const fn sealed(&self) -> bool {
        matches!(self.status, IdentityStatus::Departed)
    }
}
