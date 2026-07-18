//! The tenant — the root isolation boundary (seed §4.1, TEN-1).

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{Error, TenantId};

/// Whether a tenant may authenticate. Suspension is fail-closed: a suspended
/// tenant's tokens stop resolving (uniform 401, ADR-0008); its data is
/// retained untouched. Full lifecycle (export, deletion, destruction
/// certificates) is TEN-5.
///
/// No `Default`: admitting a tenant is always an explicit decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TenantStatus {
    /// Tokens for this tenant resolve normally.
    Active,
    /// Tokens for this tenant are rejected as unresolvable.
    Suspended,
}

impl TenantStatus {
    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            TenantStatus::Active => "active",
            TenantStatus::Suspended => "suspended",
        }
    }
}

impl fmt::Display for TenantStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TenantStatus {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(TenantStatus::Active),
            "suspended" => Ok(TenantStatus::Suspended),
            other => Err(Error::Invalid {
                message: format!("unknown tenant status: {other:?}"),
            }),
        }
    }
}

/// An organisation admitted to this Synveda deployment. Every request the
/// gateway serves runs on behalf of exactly one tenant, resolved from token
/// claims before anything else (TEN-1, ADR-0008).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tenant {
    /// The isolation key every tenant-scoped table carries.
    pub id: TenantId,
    /// Human-stable handle (lowercase, hyphenated), unique per deployment;
    /// used by operators and the CLI, never as an isolation key.
    pub slug: String,
    /// Display name.
    pub name: String,
    /// Whether the tenant may authenticate.
    pub status: TenantStatus,
    /// When the tenant was admitted.
    pub created_at: DateTime<Utc>,
}
