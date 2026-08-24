//! Trusted MCP server catalogue vocabulary (CPR-25, ADR-0086).
//!
//! This is registry metadata, not an execution API. A declared tool or an
//! imported description grants no authority, credentials are represented only
//! by opaque secret references, and project bindings pin an exact approved
//! immutable version.

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    CapabilitySnapshotId, Error, ProjectId, ProposalId, Result, ScopeId, ToolBindingId,
    ToolServerId, ToolServerVersionId,
};

/// MCP protocol version implemented and tested by this catalogue boundary.
pub const MCP_PROTOCOL_VERSION: &str = "2026-07-28";

/// Official modelcontextprotocol specification commit for the pinned release.
pub const MCP_SPEC_COMMIT: &str = "5f5440bb26a62e2cf3440b92da5a667efa03b267";

/// Largest accepted raw discovery result.
pub const MAX_CAPABILITY_SNAPSHOT_BYTES: usize = 512 * 1024;
/// Largest number of entries in any one capability family.
pub const MAX_CAPABILITIES_PER_FAMILY: usize = 2_000;
/// Largest number of requested permission labels.
pub const MAX_REQUESTED_PERMISSIONS: usize = 100;

macro_rules! closed_enum {
    ($name:ident, [$($variant:ident => $wire:literal),+ $(,)?]) => {
        impl $name {
            /// Every stored value, in schema order.
            pub const ALL: [Self; closed_enum!(@count $($variant),+)] = [$(Self::$variant),+];

            /// Stable wire and storage spelling.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $wire),+ }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(value: &str) -> Result<Self> {
                Self::ALL.into_iter().find(|candidate| candidate.as_str() == value)
                    .ok_or_else(|| Error::Invalid {
                        message: format!("unknown {}: {value:?}", stringify!($name)),
                    })
            }
        }
    };
    (@count $head:ident $(,$tail:ident)*) => { 1usize $(+ closed_enum!(@one $tail))* };
    (@one $value:ident) => { 1usize };
}

/// How catalogue metadata entered Synveda.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolServerSourceKind {
    /// A standalone MCP server manifest.
    Manifest,
    /// One entry imported from supported client configuration.
    ClientConfig,
    /// Remote Streamable HTTP metadata.
    RemoteHttp,
    /// Local stdio metadata admitted by a trusted local adapter.
    TrustedLocalAdapter,
}
closed_enum!(ToolServerSourceKind, [
    Manifest => "manifest",
    ClientConfig => "client_config",
    RemoteHttp => "remote_http",
    TrustedLocalAdapter => "trusted_local_adapter",
]);

/// Supported MCP transport architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolTransport {
    /// Standard-input/output transport, launched only by a trusted local adapter.
    Stdio,
    /// Stateless Streamable HTTP transport.
    StreamableHttp,
}
closed_enum!(ToolTransport, [Stdio => "stdio", StreamableHttp => "streamable_http"]);

/// Authentication metadata. This never contains credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolAuthenticationKind {
    /// No authentication.
    None,
    /// OAuth metadata.
    OAuth,
    /// API-key metadata backed by a secret reference.
    ApiKey,
    /// Another named mechanism backed by a secret reference.
    Custom,
}
closed_enum!(ToolAuthenticationKind, [
    None => "none",
    OAuth => "oauth",
    ApiKey => "api_key",
    Custom => "custom",
]);

/// State of a revisioned project binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolBindingState {
    /// Advertised to authorised project clients.
    Enabled,
    /// Retained but not advertised.
    Disabled,
    /// Logically removed; history and audit references remain.
    Removed,
}
closed_enum!(ToolBindingState, [Enabled => "enabled", Disabled => "disabled", Removed => "removed"]);

/// Trust state derived from the version's single VedaFlow proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolVersionState {
    /// Proposal remains open; the version cannot be bound or advertised.
    Quarantined,
    /// Proposal applied; the version may be bound explicitly.
    Approved,
    /// Proposal was rejected or withdrawn.
    Rejected,
}
closed_enum!(ToolVersionState, [Quarantined => "quarantined", Approved => "approved", Rejected => "rejected"]);

/// Trusted reporter that performed a read-only connection test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolTestHarness {
    /// Trusted local process boundary for stdio metadata.
    TrustedLocalAdapter,
    /// Trusted remote client for Streamable HTTP metadata.
    RemoteHttpAdapter,
}
closed_enum!(ToolTestHarness, [
    TrustedLocalAdapter => "trusted_local_adapter",
    RemoteHttpAdapter => "remote_http_adapter",
]);

/// Terminal read-only connection-test outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolTestOutcome {
    /// Every declared read-only check passed.
    Passed,
    /// One or more checks returned a protocol failure.
    Failed,
    /// The harness could not complete the checks.
    Error,
}
closed_enum!(ToolTestOutcome, [Passed => "passed", Failed => "failed", Error => "error"]);

/// Credential-free transport and source metadata for one immutable version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolServerDescriptor {
    /// Import source class.
    pub source_kind: ToolServerSourceKind,
    /// Human-inspectable source reference with no credentials.
    pub source_reference: String,
    /// Supported transport.
    pub transport: ToolTransport,
    /// HTTPS Streamable HTTP endpoint, when remote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// Executable identity for trusted-local-adapter stdio, never a shell line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Literal argument vector for stdio metadata.
    #[serde(default)]
    pub args: Vec<String>,
    /// Authentication mechanism, not authentication material.
    pub authentication: ToolAuthenticationKind,
    /// Opaque reference resolved by the trusted adapter at use time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_reference: Option<String>,
    /// Requested capability/permission labels; declarations grant nothing.
    #[serde(default)]
    pub requested_permissions: Vec<String>,
    /// Forward-compatible, credential-free metadata.
    #[serde(default)]
    pub metadata: Value,
}

impl ToolServerDescriptor {
    /// Validate transport shape, bounds and the secret-reference boundary.
    pub fn validate(&self) -> Result<()> {
        bounded_text("tool source_reference", &self.source_reference, 1, 2_048)?;
        match self.transport {
            ToolTransport::StreamableHttp => {
                let endpoint = self.endpoint.as_deref().ok_or_else(|| Error::Invalid {
                    message: "streamable_http tool metadata requires an HTTPS endpoint".to_owned(),
                })?;
                if !endpoint.starts_with("https://") || endpoint.contains('@') {
                    return Err(Error::Invalid {
                        message: "streamable_http endpoint must be credential-free HTTPS"
                            .to_owned(),
                    });
                }
                if self.command.is_some() || !self.args.is_empty() {
                    return Err(Error::Invalid {
                        message: "streamable_http metadata cannot name a local command".to_owned(),
                    });
                }
            }
            ToolTransport::Stdio => {
                if self.source_kind != ToolServerSourceKind::TrustedLocalAdapter
                    && self.source_kind != ToolServerSourceKind::ClientConfig
                    && self.source_kind != ToolServerSourceKind::Manifest
                {
                    return Err(Error::Invalid {
                        message: "stdio metadata must come through a trusted local source"
                            .to_owned(),
                    });
                }
                let command = self.command.as_deref().ok_or_else(|| Error::Invalid {
                    message: "stdio tool metadata requires an executable name".to_owned(),
                })?;
                bounded_text("stdio command", command, 1, 512)?;
                if self.endpoint.is_some() || command.contains(char::is_whitespace) {
                    return Err(Error::Invalid {
                        message: "stdio command is one executable token and has no endpoint"
                            .to_owned(),
                    });
                }
                if self.args.len() > 100 {
                    return Err(Error::Invalid {
                        message: "stdio metadata exceeds 100 literal arguments".to_owned(),
                    });
                }
                for arg in &self.args {
                    bounded_text("stdio argument", arg, 0, 2_048)?;
                }
            }
        }
        match (self.authentication, self.secret_reference.as_deref()) {
            (ToolAuthenticationKind::None, None) => {}
            (ToolAuthenticationKind::None, Some(_)) => {
                return Err(Error::Invalid {
                    message: "authentication none cannot carry a secret reference".to_owned(),
                });
            }
            (_, Some(reference)) => bounded_secret_ref(reference)?,
            (_, None) => {
                return Err(Error::Invalid {
                    message: "authenticated tool metadata requires a secret_reference".to_owned(),
                });
            }
        }
        if self.requested_permissions.len() > MAX_REQUESTED_PERMISSIONS {
            return Err(Error::Invalid {
                message: format!(
                    "tool metadata exceeds {MAX_REQUESTED_PERMISSIONS} requested permissions"
                ),
            });
        }
        let mut permissions = BTreeSet::new();
        for permission in &self.requested_permissions {
            bounded_text("requested permission", permission, 1, 100)?;
            if !permissions.insert(permission) {
                return Err(Error::Invalid {
                    message: format!("duplicate requested permission {permission:?}"),
                });
            }
        }
        validate_safe_object("tool metadata", &self.metadata, 64 * 1024)
    }
}

fn bounded_text(field: &str, value: &str, min: usize, max: usize) -> Result<()> {
    let count = value.chars().count();
    if count < min || count > max || value.chars().any(char::is_control) {
        return Err(Error::Invalid {
            message: format!("{field} must contain {min}..={max} non-control characters"),
        });
    }
    Ok(())
}

fn bounded_secret_ref(value: &str) -> Result<()> {
    bounded_text("secret_reference", value, 1, 512)?;
    if value.contains(char::is_whitespace) || value.contains('=') {
        return Err(Error::Invalid {
            message: "secret_reference must be an opaque identifier, not credential material"
                .to_owned(),
        });
    }
    Ok(())
}

fn validate_safe_object(field: &str, value: &Value, max_bytes: usize) -> Result<()> {
    if !value.is_object() {
        return Err(Error::Invalid {
            message: format!("{field} must be a JSON object"),
        });
    }
    let bytes = serde_json::to_vec(value).map_err(|err| Error::Invalid {
        message: format!("encode {field}: {err}"),
    })?;
    if bytes.len() > max_bytes {
        return Err(Error::Invalid {
            message: format!("{field} exceeds {max_bytes} bytes"),
        });
    }
    reject_secret_fields(value, field)
}

fn reject_secret_fields(value: &Value, path: &str) -> Result<()> {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let lower = key.to_ascii_lowercase();
                if [
                    "secret",
                    "password",
                    "token",
                    "authorization",
                    "credential",
                    "api_key",
                ]
                .iter()
                .any(|needle| lower.contains(needle))
                {
                    return Err(Error::Invalid {
                        message: format!(
                            "{path}.{key} looks like credential material; use secret_reference"
                        ),
                    });
                }
                reject_secret_fields(child, &format!("{path}.{key}"))?;
            }
        }
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                reject_secret_fields(child, &format!("{path}[{index}]"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// One normalised family of discovery entries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityCollection {
    /// Complete entries, sorted by stable identity.
    pub entries: Vec<Value>,
}

/// Canonical discovery projection for comparison and public inspection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedCapabilities {
    /// Exact MCP protocol version.
    pub protocol_version: String,
    /// Server implementation identity, including forward-compatible fields.
    pub server_info: Value,
    /// Tool schemas sorted by name.
    pub tools: CapabilityCollection,
    /// Resources sorted by URI then name.
    pub resources: CapabilityCollection,
    /// Prompt schemas sorted by name.
    pub prompts: CapabilityCollection,
    /// Forward-compatible discovery metadata.
    pub metadata: Value,
}

/// Validate and normalise a raw stateless MCP discovery result.
pub fn normalize_capabilities(raw: &Value) -> Result<NormalizedCapabilities> {
    validate_safe_object("capability snapshot", raw, MAX_CAPABILITY_SNAPSHOT_BYTES)?;
    let object = raw.as_object().expect("validated object");
    let protocol_version = object
        .get("protocol_version")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Invalid {
            message: "capability snapshot requires protocol_version".to_owned(),
        })?;
    if protocol_version != MCP_PROTOCOL_VERSION {
        return Err(Error::Invalid {
            message: format!(
                "MCP protocol {protocol_version:?} is unsupported; expected {MCP_PROTOCOL_VERSION}"
            ),
        });
    }
    let server_info = object
        .get("server_info")
        .cloned()
        .unwrap_or_else(|| Value::Object(Default::default()));
    if !server_info.is_object() {
        return Err(Error::Invalid {
            message: "server_info must be an object".to_owned(),
        });
    }
    let tools = sorted_entries(object.get("tools"), "tools", "name")?;
    let resources = sorted_entries(object.get("resources"), "resources", "uri")?;
    let prompts = sorted_entries(object.get("prompts"), "prompts", "name")?;
    let mut metadata = object.clone();
    for key in [
        "protocol_version",
        "server_info",
        "tools",
        "resources",
        "prompts",
    ] {
        metadata.remove(key);
    }
    Ok(NormalizedCapabilities {
        protocol_version: protocol_version.to_owned(),
        server_info,
        tools: CapabilityCollection { entries: tools },
        resources: CapabilityCollection { entries: resources },
        prompts: CapabilityCollection { entries: prompts },
        metadata: Value::Object(metadata),
    })
}

fn sorted_entries(value: Option<&Value>, family: &str, identity: &str) -> Result<Vec<Value>> {
    let values = match value {
        None => Vec::new(),
        Some(Value::Array(values)) => values.clone(),
        Some(_) => {
            return Err(Error::Invalid {
                message: format!("{family} must be an array"),
            });
        }
    };
    if values.len() > MAX_CAPABILITIES_PER_FAMILY {
        return Err(Error::Invalid {
            message: format!("{family} exceeds {MAX_CAPABILITIES_PER_FAMILY} entries"),
        });
    }
    let mut identities = BTreeSet::new();
    for entry in &values {
        let object = entry.as_object().ok_or_else(|| Error::Invalid {
            message: format!("every {family} entry must be an object"),
        })?;
        let key = object
            .get(identity)
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Invalid {
                message: format!("every {family} entry requires {identity}"),
            })?;
        bounded_text(&format!("{family} {identity}"), key, 1, 2_048)?;
        if !identities.insert(key.to_owned()) {
            return Err(Error::Invalid {
                message: format!("duplicate {family} {identity} {key:?}"),
            });
        }
    }
    let mut values = values;
    values.sort_by(|left, right| {
        left.get(identity)
            .and_then(Value::as_str)
            .cmp(&right.get(identity).and_then(Value::as_str))
            .then_with(|| left.to_string().cmp(&right.to_string()))
    });
    Ok(values)
}

/// Typed Tool/apply effect carried by a VedaFlow proposal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToolCommand {
    /// Register a stable server and stage its first immutable version.
    Register {
        /// Pre-minted aggregate id.
        server_id: ToolServerId,
        /// Pre-minted immutable version id.
        version_id: ToolServerVersionId,
        /// Pre-minted immutable snapshot id.
        snapshot_id: CapabilitySnapshotId,
        /// Scope governing the catalogue entry.
        governing_scope_id: ScopeId,
        /// Tenant-unique display name.
        name: String,
        /// Credential-free immutable descriptor.
        descriptor: ToolServerDescriptor,
        /// Canonical descriptor and capability digest.
        digest: String,
        /// Raw discovery evidence.
        raw_capabilities: Value,
        /// Canonical comparison projection.
        normalized_capabilities: NormalizedCapabilities,
    },
    /// Stage a changed immutable version; the current approved pointer does not move yet.
    StageVersion {
        /// Stable catalogue aggregate.
        server_id: ToolServerId,
        /// Exact current approved version precondition.
        expected_current_version_id: ToolServerVersionId,
        /// Pre-minted new version id.
        version_id: ToolServerVersionId,
        /// Pre-minted snapshot id.
        snapshot_id: CapabilitySnapshotId,
        /// Governing scope repeated at the integrity boundary.
        governing_scope_id: ScopeId,
        /// Credential-free immutable descriptor.
        descriptor: ToolServerDescriptor,
        /// Canonical descriptor and capability digest.
        digest: String,
        /// Raw discovery evidence.
        raw_capabilities: Value,
        /// Canonical comparison projection.
        normalized_capabilities: NormalizedCapabilities,
    },
    /// Bind an exact approved server version to a project.
    Bind {
        /// Pre-minted binding id.
        binding_id: ToolBindingId,
        /// Target project.
        project_id: ProjectId,
        /// Project scope repeated for authorization/integrity.
        scope_id: ScopeId,
        /// Stable server.
        server_id: ToolServerId,
        /// Exact immutable version; never follow-current.
        version_id: ToolServerVersionId,
        /// Initial state.
        state: ToolBindingState,
    },
    /// Change a binding's exact version or activation state.
    SetBinding {
        /// Stable binding.
        binding_id: ToolBindingId,
        /// Project repeated for integrity.
        project_id: ProjectId,
        /// Project scope repeated for authorization/integrity.
        scope_id: ScopeId,
        /// Exact optimistic concurrency precondition.
        expected_revision: u64,
        /// Complete resulting exact version.
        version_id: ToolServerVersionId,
        /// Complete resulting state.
        state: ToolBindingState,
        /// Bounded reason code.
        reason: String,
    },
}

impl ToolCommand {
    /// Stable command name.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Register { .. } => "register",
            Self::StageVersion { .. } => "stage_version",
            Self::Bind { .. } => "bind",
            Self::SetBinding { .. } => "set_binding",
        }
    }

    /// Scope at which this effect is decided.
    #[must_use]
    pub const fn scope_id(&self) -> ScopeId {
        match self {
            Self::Register {
                governing_scope_id, ..
            }
            | Self::StageVersion {
                governing_scope_id, ..
            } => *governing_scope_id,
            Self::Bind { scope_id, .. } | Self::SetBinding { scope_id, .. } => *scope_id,
        }
    }

    /// Stable server id when known.
    #[must_use]
    pub const fn server_id(&self) -> Option<ToolServerId> {
        match self {
            Self::Register { server_id, .. }
            | Self::StageVersion { server_id, .. }
            | Self::Bind { server_id, .. } => Some(*server_id),
            Self::SetBinding { .. } => None,
        }
    }

    /// Exact immutable version named by the command.
    #[must_use]
    pub const fn version_id(&self) -> ToolServerVersionId {
        match self {
            Self::Register { version_id, .. }
            | Self::StageVersion { version_id, .. }
            | Self::Bind { version_id, .. }
            | Self::SetBinding { version_id, .. } => *version_id,
        }
    }

    /// Binding id when the command is a binding change.
    #[must_use]
    pub const fn binding_id(&self) -> Option<ToolBindingId> {
        match self {
            Self::Bind { binding_id, .. } | Self::SetBinding { binding_id, .. } => {
                Some(*binding_id)
            }
            Self::Register { .. } | Self::StageVersion { .. } => None,
        }
    }
}

/// Governed tool mutation outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolMutationOutcome {
    /// Effect applied immediately.
    Applied,
    /// Immutable version/binding intent is quarantined pending review.
    PendingReview,
    /// Governed effect reached a terminal refusal.
    Rejected,
}

/// Stable response shared by all Tool/apply mutations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolMutationResult {
    /// VedaFlow change id.
    pub change_id: ProposalId,
    /// Governance result.
    pub outcome: ToolMutationOutcome,
    /// Stable server when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_id: Option<ToolServerId>,
    /// Exact immutable version when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_id: Option<ToolServerVersionId>,
    /// Binding when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding_id: Option<ToolBindingId>,
    /// Resulting binding revision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding_revision: Option<u64>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn descriptor() -> ToolServerDescriptor {
        ToolServerDescriptor {
            source_kind: ToolServerSourceKind::RemoteHttp,
            source_reference: "https://tools.example.test/manifest.json".to_owned(),
            transport: ToolTransport::StreamableHttp,
            endpoint: Some("https://tools.example.test/mcp".to_owned()),
            command: None,
            args: Vec::new(),
            authentication: ToolAuthenticationKind::ApiKey,
            secret_reference: Some("vault:projects/pulseboard/mcp".to_owned()),
            requested_permissions: vec!["repository.read".to_owned()],
            metadata: json!({"vendor_extension": true}),
        }
    }

    #[test]
    fn pinned_protocol_is_the_verified_stable_release() {
        assert_eq!(MCP_PROTOCOL_VERSION, "2026-07-28");
        assert_eq!(MCP_SPEC_COMMIT, "5f5440bb26a62e2cf3440b92da5a667efa03b267");
    }

    #[test]
    fn transport_and_secret_boundaries_are_explicit() {
        descriptor().validate().unwrap();
        let mut bad = descriptor();
        bad.endpoint = Some("http://user:pass@tools.example.test/mcp".to_owned());
        assert!(bad.validate().is_err());
        let mut embedded = descriptor();
        embedded.metadata = json!({"api_token": "plaintext"});
        assert!(embedded.validate().is_err());
    }

    #[test]
    fn stdio_metadata_names_a_literal_command_at_a_trusted_boundary() {
        let mut local = descriptor();
        local.source_kind = ToolServerSourceKind::TrustedLocalAdapter;
        local.source_reference = "trusted-adapter:claude-code".to_owned();
        local.transport = ToolTransport::Stdio;
        local.endpoint = None;
        local.command = Some("pulseboard-mcp".to_owned());
        local.args = vec!["serve".to_owned(), "--read-only".to_owned()];
        local.authentication = ToolAuthenticationKind::None;
        local.secret_reference = None;
        local.validate().unwrap();

        local.command = Some("sh -c".to_owned());
        assert!(
            local.validate().is_err(),
            "a shell line is not an executable token"
        );
    }

    #[test]
    fn discovery_is_bounded_sorted_and_preserves_extensions() {
        let raw = json!({
            "protocol_version": MCP_PROTOCOL_VERSION,
            "server_info": {"name": "repo", "version": "1.2.3", "x-vendor": 7},
            "tools": [
                {"name": "zeta", "inputSchema": {"type": "object"}},
                {"name": "alpha", "description": "untrusted text", "inputSchema": {}}
            ],
            "resources": [{"uri": "repo://README.md", "name": "README"}],
            "prompts": [{"name": "review"}],
            "instructions": "forward compatible"
        });
        let normalized = normalize_capabilities(&raw).unwrap();
        assert_eq!(normalized.tools.entries[0]["name"], "alpha");
        assert_eq!(normalized.metadata["instructions"], "forward compatible");
    }

    #[test]
    fn retired_and_future_protocols_are_not_invented() {
        for version in ["2024-11-05", "2025-03-26", "2025-06-18", "2027-01-01"] {
            assert!(normalize_capabilities(&json!({"protocol_version": version})).is_err());
        }
    }
}
