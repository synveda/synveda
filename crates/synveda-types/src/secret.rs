//! Stable, content-free tenant-secret vocabulary (CPR-35, ADR-0094).
//!
//! This module deliberately has no secret value type. The shared application
//! vocabulary needs to recognise a stable reference and its purpose; only the
//! operator custody boundary and the envelope crate ever handle plaintext.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{Error, Result, TenantSecretId};

/// Canonical scheme for a Synveda-custodied tenant-secret reference.
pub const TENANT_SECRET_REFERENCE_PREFIX: &str = "synveda-secret://";

/// Closed purpose vocabulary for locally custodied tenant secrets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantSecretKind {
    /// A complete directory connector configuration.
    Directory,
    /// Authentication material referenced by one immutable Tool descriptor.
    ToolServer,
    /// Credential for a model or embedding provider when a scoped consumer exists.
    ModelProvider,
    /// Credential used by an explicitly credentialed import/export provider.
    ImportExport,
}

impl TenantSecretKind {
    /// Stable SQL/wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Directory => "directory",
            Self::ToolServer => "tool_server",
            Self::ModelProvider => "model_provider",
            Self::ImportExport => "import_export",
        }
    }
}

impl fmt::Display for TenantSecretKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TenantSecretKind {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "directory" => Ok(Self::Directory),
            "tool_server" => Ok(Self::ToolServer),
            "model_provider" => Ok(Self::ModelProvider),
            "import_export" => Ok(Self::ImportExport),
            _ => Err(Error::Invalid {
                message: format!("unknown tenant-secret kind: {value:?}"),
            }),
        }
    }
}

/// Whether a stable local reference currently resolves to an envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantSecretState {
    /// The reference has a current sealed value.
    Active,
    /// The value was destroyed; content-free identity remains.
    Revoked,
}

impl TenantSecretState {
    /// Stable SQL/wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Revoked => "revoked",
        }
    }
}

impl fmt::Display for TenantSecretState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TenantSecretState {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "active" => Ok(Self::Active),
            "revoked" => Ok(Self::Revoked),
            _ => Err(Error::Invalid {
                message: format!("unknown tenant-secret state: {value:?}"),
            }),
        }
    }
}

/// Render the stable, non-secret reference stored by immutable artifacts.
#[must_use]
pub fn tenant_secret_reference(id: TenantSecretId) -> String {
    format!("{TENANT_SECRET_REFERENCE_PREFIX}{id}")
}

/// Parse a Synveda-local reference, leaving external opaque references alone.
///
/// A value using our scheme must be canonical. That makes misspellings fail at
/// admission rather than surviving as an external reference which no trusted
/// adapter could resolve.
pub fn parse_tenant_secret_reference(value: &str) -> Result<Option<TenantSecretId>> {
    let Some(raw) = value.strip_prefix(TENANT_SECRET_REFERENCE_PREFIX) else {
        return Ok(None);
    };
    let id: TenantSecretId = raw.parse().map_err(|_| Error::Invalid {
        message: "synveda-secret reference must end in a canonical UUID".to_owned(),
    })?;
    if tenant_secret_reference(id) != value {
        return Err(Error::Invalid {
            message: "synveda-secret reference must use its canonical spelling".to_owned(),
        });
    }
    Ok(Some(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_references_round_trip_and_external_references_pass_through() {
        let id = TenantSecretId::new();
        let reference = tenant_secret_reference(id);
        assert_eq!(parse_tenant_secret_reference(&reference), Ok(Some(id)));
        assert_eq!(
            parse_tenant_secret_reference("vault:projects/pulseboard/mcp"),
            Ok(None)
        );
        assert!(parse_tenant_secret_reference("synveda-secret://not-a-uuid").is_err());
        assert!(
            parse_tenant_secret_reference(&format!(
                "synveda-secret://{}",
                id.to_string().to_ascii_uppercase()
            ))
            .is_err()
        );
    }
}
