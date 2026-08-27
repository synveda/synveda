//! Trusted MCP server catalogue and project bindings (CPR-25, ADR-0086).
//!
//! This module registers and compares credential-free metadata. It does not
//! proxy execution, spawn stdio processes or treat tool descriptions as
//! authority. A trusted adapter may report stateless discovery and read-only
//! connection-test evidence; every approval and binding mutation is a typed
//! VedaFlow `Tool/apply` change.

use std::collections::{BTreeMap, BTreeSet};

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::{Acquire, PgConnection};
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{Action, Resource};
use synveda_store::anchors::AnchorSelection;
use synveda_store::tool_registry::{
    self as store, StoredToolBinding, StoredToolChange, StoredToolServer, StoredToolTestRun,
    StoredToolVersion,
};
use synveda_store::{configuration as runtime_configuration, projects, rls, scopes};
use synveda_types::configuration::ExternalProvider;
use synveda_types::json::canonicalise;
use synveda_types::{
    ArtifactFamily, ArtifactReference, AssetKind, CapabilitySnapshotId, Error, IdentityId,
    NormalizedCapabilities, ProjectId, ProposalEffect, ProposalId, ProposalState, Result, ScopeId,
    Sensitivity, TenantId, ToolAuthenticationKind, ToolBindingId, ToolBindingState, ToolCommand,
    ToolMutationOutcome, ToolMutationResult, ToolServerDescriptor, ToolServerId,
    ToolServerSourceKind, ToolServerVersionId, ToolTestHarness, ToolTestOutcome, ToolTestRunId,
    ToolTransport, ToolVersionState, normalize_capabilities,
};
use synveda_vedaflow::{self as vedaflow, PolicySnapshot, Signer};
use utoipa::ToSchema;

use crate::app::AppState;
use crate::approvals::{self, Requested};
use crate::audit;
use crate::authz::{self, Authorized, DecisionInput};
use crate::idempotency::{Claim, Dispatch};
use crate::request::{body, commit, found, tenant_id};
use crate::workspaces::{ApiErrorBody, string_enum, subject};

/// Tool-registry API outcomes by operation and result class.
pub const TOOL_OPERATIONS_TOTAL: &str = "synveda_tool_operations_total";

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;
const READ_ONLY_METHODS: [&str; 4] = [
    "server/discover",
    "tools/list",
    "resources/list",
    "prompts/list",
];

fn source_kind_schema() -> utoipa::openapi::schema::Object {
    string_enum(ToolServerSourceKind::ALL.iter().map(|value| value.as_str()))
}

fn transport_schema() -> utoipa::openapi::schema::Object {
    string_enum(ToolTransport::ALL.iter().map(|value| value.as_str()))
}

fn authentication_schema() -> utoipa::openapi::schema::Object {
    string_enum(
        ToolAuthenticationKind::ALL
            .iter()
            .map(|value| value.as_str()),
    )
}

fn version_state_schema() -> utoipa::openapi::schema::Object {
    string_enum(ToolVersionState::ALL.iter().map(|value| value.as_str()))
}

fn binding_state_schema() -> utoipa::openapi::schema::Object {
    string_enum(ToolBindingState::ALL.iter().map(|value| value.as_str()))
}

fn mutation_outcome_schema() -> utoipa::openapi::schema::Object {
    string_enum(["applied", "pending_review", "rejected"].into_iter())
}

fn test_harness_schema() -> utoipa::openapi::schema::Object {
    string_enum(ToolTestHarness::ALL.iter().map(|value| value.as_str()))
}

fn test_outcome_schema() -> utoipa::openapi::schema::Object {
    string_enum(ToolTestOutcome::ALL.iter().map(|value| value.as_str()))
}

/// Credential-free immutable server descriptor.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolServerDescriptorBody {
    /// Import source class.
    #[schema(schema_with = source_kind_schema)]
    pub source_kind: String,
    /// Human-inspectable credential-free source reference.
    pub source_reference: String,
    /// `stdio` or `streamable_http`; legacy HTTP+SSE is not accepted.
    #[schema(schema_with = transport_schema)]
    pub transport: String,
    /// HTTPS endpoint for Streamable HTTP.
    #[serde(default)]
    pub endpoint: Option<String>,
    /// One executable token for trusted-local stdio metadata.
    #[serde(default)]
    pub command: Option<String>,
    /// Literal argument vector; never a shell line.
    #[serde(default)]
    pub args: Vec<String>,
    /// Credential-free authentication class.
    #[schema(schema_with = authentication_schema)]
    pub authentication: String,
    /// Opaque reference resolved outside the gateway. Never a secret value.
    #[serde(default)]
    pub secret_reference: Option<String>,
    /// Requested permissions are review metadata, not authorisation.
    #[serde(default)]
    pub requested_permissions: Vec<String>,
    /// Forward-compatible credential-free metadata.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub metadata: Value,
}

impl TryFrom<ToolServerDescriptorBody> for ToolServerDescriptor {
    type Error = Error;

    fn try_from(value: ToolServerDescriptorBody) -> Result<Self> {
        let descriptor = Self {
            source_kind: value.source_kind.parse()?,
            source_reference: value.source_reference,
            transport: value.transport.parse()?,
            endpoint: value.endpoint,
            command: value.command,
            args: value.args,
            authentication: value.authentication.parse()?,
            secret_reference: value.secret_reference,
            requested_permissions: value.requested_permissions,
            metadata: value.metadata,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }
}

impl From<ToolServerDescriptor> for ToolServerDescriptorBody {
    fn from(value: ToolServerDescriptor) -> Self {
        Self {
            source_kind: value.source_kind.as_str().to_owned(),
            source_reference: value.source_reference,
            transport: value.transport.as_str().to_owned(),
            endpoint: value.endpoint,
            command: value.command,
            args: value.args,
            authentication: value.authentication.as_str().to_owned(),
            secret_reference: value.secret_reference,
            requested_permissions: value.requested_permissions,
            metadata: value.metadata,
        }
    }
}

/// Register a server and its first immutable discovery snapshot.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RegisterToolServerBody {
    /// Scope governing the catalogue entry.
    #[schema(value_type = String, format = "uuid")]
    pub governing_scope_id: ScopeId,
    /// Tenant-unique display name.
    pub name: String,
    /// Immutable source/transport/authentication metadata.
    pub descriptor: ToolServerDescriptorBody,
    /// Raw stateless MCP discovery result.
    #[schema(value_type = Object)]
    pub capabilities: Value,
}

/// Import one supported client configuration entry.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportToolClientConfigBody {
    /// Governing scope.
    #[schema(value_type = String, format = "uuid")]
    pub governing_scope_id: ScopeId,
    /// `claude_code`, `cursor` or `vscode` configuration grammar.
    pub client: String,
    /// Server key in the client configuration.
    pub name: String,
    /// One client server object. Embedded env/header values are refused.
    #[schema(value_type = Object)]
    pub server: Value,
    /// Opaque secret reference replacing any client credential material.
    #[serde(default)]
    pub secret_reference: Option<String>,
    /// Raw stateless discovery result captured by the trusted adapter.
    #[schema(value_type = Object)]
    pub capabilities: Value,
}

/// Stage changed source, transport, auth or capability metadata.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StageToolVersionBody {
    /// Exact current approved version precondition.
    #[schema(value_type = String, format = "uuid")]
    pub expected_current_version_id: ToolServerVersionId,
    /// Complete replacement descriptor.
    pub descriptor: ToolServerDescriptorBody,
    /// Complete raw discovery result.
    #[schema(value_type = Object)]
    pub capabilities: Value,
}

/// Report a fresh discovery using the current descriptor.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoverToolServerBody {
    /// Exact current approved version precondition.
    #[schema(value_type = String, format = "uuid")]
    pub expected_current_version_id: ToolServerVersionId,
    /// Raw stateless discovery result.
    #[schema(value_type = Object)]
    pub capabilities: Value,
}

/// Bind one exact approved version to a project.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateToolBindingBody {
    /// Target project.
    #[schema(value_type = String, format = "uuid")]
    pub project_id: ProjectId,
    /// Stable server.
    #[schema(value_type = String, format = "uuid")]
    pub server_id: ToolServerId,
    /// Exact approved version. There is no follow-current mode.
    #[schema(value_type = String, format = "uuid")]
    pub version_id: ToolServerVersionId,
    /// Initial activation state (`removed` is invalid for creation).
    #[schema(schema_with = binding_state_schema)]
    pub state: String,
}

/// Change a binding using optimistic concurrency.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateToolBindingBody {
    /// Exact current binding revision.
    pub expected_revision: u64,
    /// Complete resulting exact version.
    #[schema(value_type = String, format = "uuid")]
    pub version_id: ToolServerVersionId,
    /// Complete resulting state.
    #[schema(schema_with = binding_state_schema)]
    pub state: String,
    /// Bounded reason code (`disable`, `enable`, `repin`, `remove`).
    pub reason: String,
}

/// Trusted-adapter report for a read-only connection test.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RunToolTestBody {
    /// Trusted harness class.
    #[schema(schema_with = test_harness_schema)]
    pub harness: String,
    /// Exact adapter implementation/version.
    pub harness_version: String,
    /// Terminal outcome.
    #[schema(schema_with = test_outcome_schema)]
    pub outcome: String,
    /// Methods attempted. `tools/call` and every execution method are refused.
    pub methods: Vec<String>,
    /// End-to-end elapsed milliseconds.
    #[serde(default)]
    pub latency_ms: Option<u64>,
    /// Bounded credential-free report.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub evidence: Value,
}

/// Governed mutation response.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ToolMutationView {
    /// VedaFlow change id.
    #[schema(value_type = String, format = "uuid")]
    pub change_id: ProposalId,
    /// `applied`, `pending_review` or `rejected`.
    #[schema(schema_with = mutation_outcome_schema)]
    pub outcome: String,
    /// Stable server when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub server_id: Option<ToolServerId>,
    /// Exact immutable version when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub version_id: Option<ToolServerVersionId>,
    /// Binding when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub binding_id: Option<ToolBindingId>,
    /// Resulting binding revision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding_revision: Option<u64>,
}

impl From<ToolMutationResult> for ToolMutationView {
    fn from(value: ToolMutationResult) -> Self {
        let outcome = match value.outcome {
            ToolMutationOutcome::Applied => "applied",
            ToolMutationOutcome::PendingReview => "pending_review",
            ToolMutationOutcome::Rejected => "rejected",
        };
        Self {
            change_id: value.change_id,
            outcome: outcome.to_owned(),
            server_id: value.server_id,
            version_id: value.version_id,
            binding_id: value.binding_id,
            binding_revision: value.binding_revision,
        }
    }
}

/// Stable catalogue entry.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ToolServerView {
    /// Stable id.
    #[schema(value_type = String, format = "uuid")]
    pub id: ToolServerId,
    /// Governing scope.
    #[schema(value_type = String, format = "uuid")]
    pub governing_scope_id: ScopeId,
    /// Display name.
    pub name: String,
    /// Current approved version, absent while first registration is quarantined.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub current_version_id: Option<ToolServerVersionId>,
    /// Creation instant.
    pub created_at: DateTime<Utc>,
    /// Last approved-pointer transition.
    pub updated_at: DateTime<Utc>,
}

impl From<StoredToolServer> for ToolServerView {
    fn from(value: StoredToolServer) -> Self {
        Self {
            id: value.id,
            governing_scope_id: value.governing_scope_id,
            name: value.name,
            current_version_id: value.current_version_id,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

/// Cursor-paginated server catalogue.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ToolServerListView {
    /// Policy-visible entries.
    pub servers: Vec<ToolServerView>,
    /// Cursor after the last candidate considered.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub next_cursor: Option<ToolServerId>,
}

/// One immutable MCP server version and discovery snapshot.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ToolServerVersionView {
    /// Immutable version id.
    #[schema(value_type = String, format = "uuid")]
    pub id: ToolServerVersionId,
    /// Stable server id.
    #[schema(value_type = String, format = "uuid")]
    pub server_id: ToolServerId,
    /// VedaFlow proposal defining the trust state.
    #[schema(value_type = String, format = "uuid")]
    pub change_id: ProposalId,
    /// Monotonic version ordinal.
    pub ordinal: u64,
    /// Stable digest.
    pub digest: String,
    /// Pinned official MCP protocol version.
    pub protocol_version: String,
    /// `quarantined`, `approved` or `rejected`, derived from VedaFlow.
    #[schema(schema_with = version_state_schema)]
    pub state: String,
    /// Credential-free descriptor. Secret references are opaque identifiers.
    pub descriptor: ToolServerDescriptorBody,
    /// Whether an opaque secret reference is configured.
    pub secret_reference_present: bool,
    /// Immutable raw discovery evidence.
    #[schema(value_type = Object)]
    pub raw_capabilities: Value,
    /// Canonical comparison snapshot.
    #[schema(value_type = Object)]
    pub normalized_capabilities: Value,
    /// Snapshot digest.
    pub capability_digest: String,
    /// Capability names and descriptions grant no authority.
    pub declared_capabilities_are_authorization: bool,
    /// Discovery instant.
    pub discovered_at: DateTime<Utc>,
    /// Creation instant.
    pub created_at: DateTime<Utc>,
}

impl From<StoredToolVersion> for ToolServerVersionView {
    fn from(value: StoredToolVersion) -> Self {
        let secret_reference_present = value.descriptor.secret_reference.is_some();
        Self {
            id: value.id,
            server_id: value.server_id,
            change_id: value.proposal_id,
            ordinal: value.ordinal,
            digest: store::hex_32(&value.digest),
            protocol_version: value.protocol_version,
            state: value.state.as_str().to_owned(),
            descriptor: value.descriptor.into(),
            secret_reference_present,
            raw_capabilities: value.raw_capabilities,
            normalized_capabilities: serde_json::to_value(value.normalized_capabilities)
                .expect("normalised capabilities serialise"),
            capability_digest: store::hex_32(&value.capability_digest),
            declared_capabilities_are_authorization: false,
            discovered_at: value.discovered_at,
            created_at: value.created_at,
        }
    }
}

/// Version collection.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ToolServerVersionListView {
    /// Policy-visible versions.
    pub versions: Vec<ToolServerVersionView>,
    /// Cursor after the last returned version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<u64>,
}

/// One exact-version project binding.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ToolBindingView {
    /// Stable binding id.
    #[schema(value_type = String, format = "uuid")]
    pub id: ToolBindingId,
    /// Target project.
    #[schema(value_type = String, format = "uuid")]
    pub project_id: ProjectId,
    /// Target project scope.
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    /// Stable server.
    #[schema(value_type = String, format = "uuid")]
    pub server_id: ToolServerId,
    /// Exact immutable approved version.
    #[schema(value_type = String, format = "uuid")]
    pub version_id: ToolServerVersionId,
    /// Activation/removal state.
    #[schema(schema_with = binding_state_schema)]
    pub state: String,
    /// Optimistic-concurrency revision.
    pub revision: u64,
    /// Creation instant.
    pub created_at: DateTime<Utc>,
    /// Last transition instant.
    pub updated_at: DateTime<Utc>,
}

impl From<StoredToolBinding> for ToolBindingView {
    fn from(value: StoredToolBinding) -> Self {
        Self {
            id: value.id,
            project_id: value.project_id,
            scope_id: value.scope_id,
            server_id: value.server_id,
            version_id: value.version_id,
            state: value.state.as_str().to_owned(),
            revision: value.revision,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

/// Binding collection.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ToolBindingListView {
    /// Policy-visible bindings.
    pub bindings: Vec<ToolBindingView>,
    /// Cursor after the last returned binding.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub next_cursor: Option<ToolBindingId>,
}

/// Visible comparison between two immutable versions.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ToolVersionDiffView {
    /// Baseline version.
    #[schema(value_type = String, format = "uuid")]
    pub from_version_id: ToolServerVersionId,
    /// Candidate version.
    #[schema(value_type = String, format = "uuid")]
    pub to_version_id: ToolServerVersionId,
    /// Descriptor fields whose canonical values differ.
    pub descriptor_changed: Vec<String>,
    /// Added tool names.
    pub tools_added: Vec<String>,
    /// Removed tool names.
    pub tools_removed: Vec<String>,
    /// Tool names whose description or input schema changed.
    pub tools_changed: Vec<String>,
    /// Added resource URIs.
    pub resources_added: Vec<String>,
    /// Removed resource URIs.
    pub resources_removed: Vec<String>,
    /// Resource URIs whose schema/metadata changed.
    pub resources_changed: Vec<String>,
    /// Added prompt names.
    pub prompts_added: Vec<String>,
    /// Removed prompt names.
    pub prompts_removed: Vec<String>,
    /// Prompt names whose schema/metadata changed.
    pub prompts_changed: Vec<String>,
}

/// Secret-free generated client configuration.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ToolClientConfigurationView {
    /// Target project.
    #[schema(value_type = String, format = "uuid")]
    pub project_id: ProjectId,
    /// Configuration contains exact approved bindings only.
    #[schema(value_type = Object)]
    pub configuration: Value,
    /// Binding/version pairs included.
    pub bindings: Vec<ToolConfigurationBindingView>,
}

/// Evidence for one generated configuration entry.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ToolConfigurationBindingView {
    /// Stable server id.
    #[schema(value_type = String, format = "uuid")]
    pub server_id: ToolServerId,
    /// Binding id.
    #[schema(value_type = String, format = "uuid")]
    pub binding_id: ToolBindingId,
    /// Exact version id.
    #[schema(value_type = String, format = "uuid")]
    pub version_id: ToolServerVersionId,
    /// Immutable version digest.
    pub digest: String,
}

/// One immutable read-only connection-test report.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ToolTestRunView {
    /// Run id.
    #[schema(value_type = String, format = "uuid")]
    pub id: ToolTestRunId,
    /// Exact version tested.
    #[schema(value_type = String, format = "uuid")]
    pub version_id: ToolServerVersionId,
    /// Trusted reporter class.
    #[schema(schema_with = test_harness_schema)]
    pub harness: String,
    /// Exact reporter version.
    pub harness_version: String,
    /// Terminal outcome.
    #[schema(schema_with = test_outcome_schema)]
    pub outcome: String,
    /// Read-only methods attempted.
    pub methods: Vec<String>,
    /// Elapsed milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    /// Credential-free evidence.
    #[schema(value_type = Object)]
    pub evidence: Value,
    /// Server receipt instant.
    pub created_at: DateTime<Utc>,
}

impl From<StoredToolTestRun> for ToolTestRunView {
    fn from(value: StoredToolTestRun) -> Self {
        Self {
            id: value.id,
            version_id: value.version_id,
            harness: value.harness.as_str().to_owned(),
            harness_version: value.harness_version,
            outcome: value.outcome.as_str().to_owned(),
            methods: value.methods,
            latency_ms: value.latency_ms,
            evidence: value.evidence,
            created_at: value.created_at,
        }
    }
}

/// Test-run collection.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ToolTestRunListView {
    /// Immutable test reports.
    pub runs: Vec<ToolTestRunView>,
    /// Cursor after the last returned report.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub next_cursor: Option<ToolTestRunId>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
/// Shared bounded collection parameters for Tool catalogue resources.
pub struct ListParams {
    /// Resume after this stable catalogue identifier.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "uuid")]
    pub cursor: Option<ToolServerId>,
    /// Maximum rows, 1..=200.
    pub limit: Option<i64>,
}

#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
/// Cursor controls for immutable server versions, newest first.
pub struct ListVersionsParams {
    /// Resume below this version ordinal.
    #[serde(default)]
    pub before_ordinal: Option<u64>,
    /// Maximum rows, 1..=200.
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
/// Bounded project-binding collection parameters.
pub struct ListBindingsParams {
    /// Restrict to one project.
    #[param(value_type = Option<String>, format = "uuid")]
    pub project_id: Option<ProjectId>,
    /// Include logically removed bindings.
    #[serde(default)]
    pub include_removed: bool,
    /// Resume below this stable binding identifier.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "uuid")]
    pub cursor: Option<ToolBindingId>,
    /// Maximum rows, 1..=200.
    pub limit: Option<i64>,
}

#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
/// Cursor controls for immutable read-only test evidence.
pub struct ListTestRunsParams {
    /// Resume below this stable test-run identifier.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "uuid")]
    pub cursor: Option<ToolTestRunId>,
    /// Maximum rows, 1..=200.
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
/// Selects the exact approved or quarantined baseline for a version comparison.
pub struct DiffParams {
    /// Baseline exact version.
    #[param(value_type = String, format = "uuid")]
    pub against: ToolServerVersionId,
}

fn limit(value: Option<i64>) -> Result<i64> {
    let value = value.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&value) {
        return Err(Error::Invalid {
            message: format!("limit must be between 1 and {MAX_LIMIT}"),
        });
    }
    Ok(value)
}

fn validate_name(name: &str) -> Result<()> {
    if name.trim().is_empty() || name.chars().count() > 200 || name.chars().any(char::is_control) {
        return Err(Error::Invalid {
            message: "tool server name must contain 1..=200 non-control characters".to_owned(),
        });
    }
    Ok(())
}

fn validate_reason(reason: &str) -> Result<()> {
    if reason.trim().is_empty()
        || reason.chars().count() > 100
        || reason.chars().any(char::is_control)
    {
        return Err(Error::Invalid {
            message: "binding reason must contain 1..=100 non-control characters".to_owned(),
        });
    }
    Ok(())
}

fn validate_safe_report(value: &Value) -> Result<()> {
    if !value.is_object() {
        return Err(Error::Invalid {
            message: "tool test evidence must be a JSON object".to_owned(),
        });
    }
    if serde_json::to_vec(value)
        .map_err(|err| Error::Invalid {
            message: format!("encode tool test evidence: {err}"),
        })?
        .len()
        > 64 * 1024
    {
        return Err(Error::Invalid {
            message: "tool test evidence exceeds 65536 bytes".to_owned(),
        });
    }
    fn walk(value: &Value, path: &str) -> Result<()> {
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
                                "tool test evidence {path}.{key} looks like credential material"
                            ),
                        });
                    }
                    walk(child, &format!("{path}.{key}"))?;
                }
            }
            Value::Array(values) => {
                for (index, child) in values.iter().enumerate() {
                    walk(child, &format!("{path}[{index}]"))?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    walk(value, "evidence")
}

fn validate_methods(methods: &[String]) -> Result<()> {
    if methods.is_empty() || methods.len() > READ_ONLY_METHODS.len() {
        return Err(Error::Invalid {
            message: "tool test methods must name 1..=4 read-only discovery methods".to_owned(),
        });
    }
    let allowed = READ_ONLY_METHODS.into_iter().collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for method in methods {
        if !allowed.contains(method.as_str()) {
            return Err(Error::Invalid {
                message: format!(
                    "tool test method {method:?} is not read-only; tools/call is never accepted"
                ),
            });
        }
        if !seen.insert(method) {
            return Err(Error::Invalid {
                message: format!("duplicate tool test method {method:?}"),
            });
        }
    }
    Ok(())
}

fn prepare_version(
    descriptor: ToolServerDescriptor,
    capabilities: Value,
) -> Result<(ToolServerDescriptor, NormalizedCapabilities, String)> {
    descriptor.validate()?;
    let normalized = normalize_capabilities(&capabilities)?;
    let canonical = canonicalise(&json!({
        "descriptor": descriptor,
        "capabilities": normalized,
    }));
    let digest = blake3::hash(canonical.to_string().as_bytes())
        .to_hex()
        .to_string();
    let descriptor: ToolServerDescriptor = serde_json::from_value(
        canonical
            .get("descriptor")
            .cloned()
            .expect("canonical descriptor"),
    )
    .map_err(|err| Error::Internal {
        message: format!("rebuild canonical tool descriptor: {err}"),
    })?;
    let normalized: NormalizedCapabilities = serde_json::from_value(
        canonical
            .get("capabilities")
            .cloned()
            .expect("canonical capabilities"),
    )
    .map_err(|err| Error::Internal {
        message: format!("rebuild canonical tool capabilities: {err}"),
    })?;
    Ok((descriptor, normalized, digest))
}

fn descriptor_from_client_config(
    body: &ImportToolClientConfigBody,
) -> Result<ToolServerDescriptor> {
    if !["claude_code", "cursor", "vscode"].contains(&body.client.as_str()) {
        return Err(Error::Invalid {
            message: "supported MCP client config is claude_code, cursor or vscode".to_owned(),
        });
    }
    let object = body.server.as_object().ok_or_else(|| Error::Invalid {
        message: "client server configuration must be a JSON object".to_owned(),
    })?;
    if object.contains_key("env") || object.contains_key("headers") {
        return Err(Error::Invalid {
            message: "client configuration cannot embed env or header values; use secret_reference"
                .to_owned(),
        });
    }
    let (transport, endpoint, command, args) = match (
        object.get("url").and_then(Value::as_str),
        object.get("command").and_then(Value::as_str),
    ) {
        (Some(url), None) => (
            ToolTransport::StreamableHttp,
            Some(url.to_owned()),
            None,
            Vec::new(),
        ),
        (None, Some(command)) => {
            let args = object
                .get("args")
                .map(|value| {
                    value
                        .as_array()
                        .ok_or_else(|| Error::Invalid {
                            message: "client stdio args must be an array".to_owned(),
                        })?
                        .iter()
                        .map(|arg| {
                            arg.as_str()
                                .map(str::to_owned)
                                .ok_or_else(|| Error::Invalid {
                                    message: "client stdio args must be strings".to_owned(),
                                })
                        })
                        .collect::<Result<Vec<_>>>()
                })
                .transpose()?
                .unwrap_or_default();
            (ToolTransport::Stdio, None, Some(command.to_owned()), args)
        }
        _ => {
            return Err(Error::Invalid {
                message: "client server configuration names exactly one of url or command"
                    .to_owned(),
            });
        }
    };
    let descriptor = ToolServerDescriptor {
        source_kind: ToolServerSourceKind::ClientConfig,
        source_reference: format!("client-config:{}:{}", body.client, body.name),
        transport,
        endpoint,
        command,
        args,
        authentication: if body.secret_reference.is_some() {
            ToolAuthenticationKind::Custom
        } else {
            ToolAuthenticationKind::None
        },
        secret_reference: body.secret_reference.clone(),
        requested_permissions: Vec::new(),
        metadata: json!({"client": body.client}),
    };
    descriptor.validate()?;
    Ok(descriptor)
}

struct CommandAuthorization {
    target: synveda_types::scope::Scope,
    input: DecisionInput,
    write_allowed: Authorized,
    proposal_allowed: Authorized,
}

fn identity_of(input: &DecisionInput, act: &str) -> Result<IdentityId> {
    input
        .identity
        .as_ref()
        .map(|identity| identity.id)
        .ok_or_else(|| Error::Invalid {
            message: format!("{act} requires a provisioned identity"),
        })
}

async fn scope_for(
    tx: &mut PgConnection,
    tenant: TenantId,
    scope_id: ScopeId,
) -> Result<synveda_types::scope::Scope> {
    found(
        scopes::get(&mut *tx, tenant, scope_id).await?,
        tenant,
        scope_id,
    )
}

async fn project_scope(
    tx: &mut PgConnection,
    tenant: TenantId,
    project_id: ProjectId,
) -> Result<synveda_types::scope::Scope> {
    let project = projects::get(&mut *tx, tenant, project_id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("project {project_id}"),
        })?;
    scope_for(tx, tenant, project.scope_id).await
}

async fn authorize_read_at(
    state: &AppState,
    tx: &mut PgConnection,
    scope: &synveda_types::scope::Scope,
) -> Result<Authorized> {
    let input = authz::gather(state, tx, Some(scope), AnchorSelection::none(), Vec::new()).await?;
    authz::decide(state, &input, Action::ToolRead, Resource::Scope(scope.id))
}

async fn authorize_server_read(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    server: &StoredToolServer,
) -> Result<Authorized> {
    let scope = scope_for(tx, tenant, server.governing_scope_id).await?;
    authorize_read_at(state, tx, &scope).await
}

async fn require_active_internal_secret(
    tx: &mut PgConnection,
    tenant: TenantId,
    scope_id: ScopeId,
    descriptor: &ToolServerDescriptor,
) -> Result<()> {
    let Some(reference) = descriptor.secret_reference.as_deref() else {
        return Ok(());
    };
    let Some(id) = synveda_types::secret::parse_tenant_secret_reference(reference)? else {
        return Ok(());
    };
    if synveda_store::tenant_secrets::reference_is_active(
        &mut *tx,
        tenant,
        id,
        synveda_types::secret::TenantSecretKind::ToolServer,
        scope_id,
    )
    .await?
    {
        Ok(())
    } else {
        Err(Error::Invalid {
            message: "tool server credential reference is unavailable".to_owned(),
        })
    }
}

async fn authorize_command(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    command: &ToolCommand,
) -> Result<CommandAuthorization> {
    let target = scope_for(tx, tenant, command.scope_id()).await?;
    let input = authz::gather(
        state,
        tx,
        Some(&target),
        AnchorSelection::none(),
        Vec::new(),
    )
    .await?;
    let write_allowed =
        authz::decide(state, &input, Action::ToolWrite, Resource::Scope(target.id))?;
    let proposal_allowed = authz::decide(
        state,
        &input,
        Action::ProposalOpen,
        Resource::Scope(target.id),
    )?;

    match command {
        ToolCommand::Register {
            governing_scope_id,
            descriptor,
            ..
        } => {
            require_active_internal_secret(tx, tenant, *governing_scope_id, descriptor).await?;
        }
        ToolCommand::StageVersion {
            server_id,
            expected_current_version_id,
            governing_scope_id,
            descriptor,
            ..
        } => {
            let server = store::server(&mut *tx, tenant, *server_id)
                .await?
                .filter(|server| {
                    server.governing_scope_id == *governing_scope_id
                        && server.current_version_id == Some(*expected_current_version_id)
                })
                .ok_or_else(|| Error::Conflict {
                    message: format!(
                        "tool server {server_id} no longer has expected version {expected_current_version_id}"
                    ),
            })?;
            authorize_server_read(state, tx, tenant, &server).await?;
            require_active_internal_secret(tx, tenant, *governing_scope_id, descriptor).await?;
        }
        ToolCommand::Bind {
            project_id,
            scope_id,
            server_id,
            version_id,
            state: binding_state,
            ..
        } => {
            let project = project_scope(tx, tenant, *project_id).await?;
            if project.id != *scope_id || *binding_state == ToolBindingState::Removed {
                return Err(Error::Invalid {
                    message:
                        "a new tool binding targets its project's scope and cannot start removed"
                            .to_owned(),
                });
            }
            let server = store::server(&mut *tx, tenant, *server_id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("tool server {server_id}"),
                })?;
            let version = store::version(&mut *tx, tenant, *version_id)
                .await?
                .filter(|version| {
                    version.server_id == *server_id && version.state == ToolVersionState::Approved
                })
                .ok_or_else(|| Error::NotFound {
                    entity: format!("approved tool version {version_id}"),
                })?;
            authorize_server_read(state, tx, tenant, &server).await?;
            require_active_internal_secret(
                tx,
                tenant,
                server.governing_scope_id,
                &version.descriptor,
            )
            .await?;
        }
        ToolCommand::SetBinding {
            binding_id,
            project_id,
            scope_id,
            expected_revision,
            version_id,
            state: binding_state,
            ..
        } => {
            let binding = store::binding(&mut *tx, tenant, *binding_id)
                .await?
                .filter(|binding| {
                    binding.project_id == *project_id
                        && binding.scope_id == *scope_id
                        && binding.revision == *expected_revision
                })
                .ok_or_else(|| Error::Conflict {
                    message: format!(
                        "tool binding {binding_id} no longer has expected revision {expected_revision}"
                    ),
                })?;
            let server = store::server(&mut *tx, tenant, binding.server_id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("tool server {}", binding.server_id),
                })?;
            let version = store::version(&mut *tx, tenant, *version_id)
                .await?
                .filter(|version| {
                    version.server_id == binding.server_id
                        && version.state == ToolVersionState::Approved
                })
                .ok_or_else(|| Error::NotFound {
                    entity: format!("approved tool version {version_id}"),
                })?;
            authorize_server_read(state, tx, tenant, &server).await?;
            // A revoked credential must fail closed for activation, but it
            // must never become a trap that prevents an operator removing
            // the binding which cites it.
            if *binding_state != ToolBindingState::Removed {
                require_active_internal_secret(
                    tx,
                    tenant,
                    server.governing_scope_id,
                    &version.descriptor,
                )
                .await?;
            }
        }
    }
    Ok(CommandAuthorization {
        target,
        input,
        write_allowed,
        proposal_allowed,
    })
}

fn command_payload_hash(command: &ToolCommand) -> Result<String> {
    let value = canonicalise(
        &serde_json::to_value(command).map_err(|err| Error::Invalid {
            message: format!("encode Tool command: {err}"),
        })?,
    );
    Ok(blake3::hash(value.to_string().as_bytes())
        .to_hex()
        .to_string())
}

fn tool_artifact_reference(command: &ToolCommand) -> Result<ArtifactReference> {
    match command {
        ToolCommand::Register {
            server_id,
            version_id,
            ..
        } => ArtifactReference::new(
            ArtifactFamily::ToolServer,
            server_id.to_string(),
            command.kind(),
            version_id.to_string(),
            None,
        ),
        ToolCommand::StageVersion {
            server_id,
            expected_current_version_id,
            version_id,
            ..
        } => ArtifactReference::new(
            ArtifactFamily::ToolServer,
            server_id.to_string(),
            command.kind(),
            version_id.to_string(),
            Some(expected_current_version_id.to_string()),
        ),
        ToolCommand::Bind {
            binding_id,
            version_id,
            ..
        } => ArtifactReference::new(
            ArtifactFamily::ToolBinding,
            binding_id.to_string(),
            command.kind(),
            version_id.to_string(),
            None,
        ),
        ToolCommand::SetBinding {
            binding_id,
            expected_revision,
            version_id,
            ..
        } => ArtifactReference::new(
            ArtifactFamily::ToolBinding,
            binding_id.to_string(),
            command.kind(),
            version_id.to_string(),
            Some(expected_revision.to_string()),
        ),
    }
}

async fn open_command(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    command: &ToolCommand,
    authorization: &CommandAuthorization,
    claim: &Claim,
) -> Result<ToolMutationResult> {
    let actor = identity_of(&authorization.input, "changing trusted MCP metadata")?;
    let payload_hash = command_payload_hash(command)?;
    let artifact_reference = tool_artifact_reference(command)?;
    let manifest = canonicalise(&json!({
        "command": command.kind(),
        "payload_hash": payload_hash,
        "server_id": command.server_id(),
        "version_id": command.version_id(),
        "binding_id": command.binding_id(),
    }));
    let bytes = serde_json::to_vec(&manifest).map_err(|err| Error::Internal {
        message: format!("encode Tool change manifest: {err}"),
    })?;
    let object = vedaflow::put_object(tx, tenant, AssetKind::Tool, &bytes).await?;
    let snapshot = PolicySnapshot::new(
        authorization.proposal_allowed.decision.pack_name.clone(),
        authorization.proposal_allowed.decision.pack_version,
    );
    let members = vec![("command".to_owned(), object.hash)];
    let proposal = vedaflow::proposals::open(
        tx,
        tenant,
        &vedaflow::NewProposal {
            target_scope: authorization.target.id,
            source_scope: authorization.target.id,
            asset: AssetKind::Tool,
            effect: ProposalEffect::Apply,
            members: &members,
            artifact_references: &[artifact_reference],
            sensitivity: Sensitivity::Internal,
            title: &format!("{} MCP tool registry", command.kind()),
            proposer: actor,
            proposer_subject: &authorization.input.principal.subject,
            committed_at: Utc::now(),
            policy_snapshot: &snapshot,
        },
        &Signer::Unsigned,
    )
    .await?;
    store::insert_change(&mut *tx, tenant, proposal.id, command, &payload_hash).await?;
    store::stage_version(&mut *tx, tenant, proposal.id, command, actor).await?;
    let requirement = approvals::resolve(
        state,
        tx,
        tenant,
        &authorization.input,
        &Requested {
            target: &authorization.target,
            asset: AssetKind::Tool,
            sensitivity: Sensitivity::Internal,
            entries: &["command".to_owned()],
        },
    )
    .await?;
    let outstanding = requirement.outstanding(&[]);
    audit::record(
        tx,
        tenant,
        AuditAction::ToolChangeOpened,
        Resource::Scope(authorization.target.id).to_string(),
        Outcome::Success,
        json!({
            "change_id": proposal.id,
            "command": command.kind(),
            "payload_hash": payload_hash,
            "manifest_hash": object.hash.to_hex(),
            "artifact_references": &proposal.artifact_references,
            "server_id": command.server_id(),
            "version_id": command.version_id(),
            "binding_id": command.binding_id(),
            "authz": audit::decision_context(Action::ProposalOpen, &authorization.proposal_allowed),
            "approvals": approvals::audit_context(&requirement, &outstanding),
        }),
    )
    .await?;
    let result = if outstanding.is_empty() {
        apply_loaded(
            state,
            tx,
            tenant,
            proposal.id,
            command,
            &payload_hash,
            actor,
        )
        .await?
    } else {
        ToolMutationResult {
            change_id: proposal.id,
            outcome: ToolMutationOutcome::PendingReview,
            server_id: command.server_id(),
            version_id: Some(command.version_id()),
            binding_id: command.binding_id(),
            binding_revision: None,
        }
    };
    claim.remember(tx, tenant, proposal.id.as_uuid()).await?;
    Ok(result)
}

async fn verify_change_binding(
    tx: &mut PgConnection,
    tenant: TenantId,
    proposal: &vedaflow::StoredProposal,
    change: &StoredToolChange,
) -> Result<()> {
    let members = vedaflow::proposals::members(tx, tenant, proposal.commit).await?;
    let [member] = members.as_slice() else {
        return Err(invalid_change(proposal.id));
    };
    let object = vedaflow::read_object(tx, tenant, member.object)
        .await?
        .ok_or_else(|| invalid_change(proposal.id))?;
    let manifest: Value =
        serde_json::from_slice(&object.content).map_err(|_| invalid_change(proposal.id))?;
    let valid = member.name == "command"
        && object.kind == AssetKind::Tool
        && command_payload_hash(&change.command)? == change.payload_hash
        && manifest.get("command").and_then(Value::as_str) == Some(change.command.kind())
        && manifest.get("payload_hash").and_then(Value::as_str)
            == Some(change.payload_hash.as_str());
    if !valid {
        return Err(invalid_change(proposal.id));
    }
    if matches!(
        change.command,
        ToolCommand::Register { .. } | ToolCommand::StageVersion { .. }
    ) {
        let version = store::version(&mut *tx, tenant, change.command.version_id())
            .await?
            .filter(|version| version.proposal_id == proposal.id)
            .ok_or_else(|| invalid_change(proposal.id))?;
        let (descriptor, capabilities, digest) =
            prepare_version(version.descriptor.clone(), version.raw_capabilities.clone())?;
        let expected = match &change.command {
            ToolCommand::Register {
                descriptor,
                normalized_capabilities,
                digest,
                ..
            }
            | ToolCommand::StageVersion {
                descriptor,
                normalized_capabilities,
                digest,
                ..
            } => (descriptor, normalized_capabilities, digest),
            _ => unreachable!(),
        };
        if &descriptor != expected.0 || &capabilities != expected.1 || &digest != expected.2 {
            return Err(invalid_change(proposal.id));
        }
    }
    Ok(())
}

fn invalid_change(id: ProposalId) -> Error {
    Error::Internal {
        message: format!("Tool change {id} failed its VedaFlow payload-integrity check"),
    }
}

struct AppliedEffect {
    server_id: Option<ToolServerId>,
    version_id: Option<ToolServerVersionId>,
    binding_id: Option<ToolBindingId>,
    binding_revision: Option<u64>,
}

async fn apply_loaded(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    change_id: ProposalId,
    command: &ToolCommand,
    payload_hash: &str,
    actor: IdentityId,
) -> Result<ToolMutationResult> {
    let authorization = authorize_command(state, tx, tenant, command).await?;
    let mut effect_tx = tx.begin().await.map_err(|err| Error::Storage {
        message: format!("begin Tool effect savepoint: {err}"),
    })?;
    let effect: Result<AppliedEffect> = async {
        Ok(match command {
            ToolCommand::Register {
                server_id,
                version_id,
                ..
            }
            | ToolCommand::StageVersion {
                server_id,
                version_id,
                ..
            } => {
                if !store::approve_version(&mut *effect_tx, tenant, command, actor).await? {
                    return Err(Error::Conflict {
                        message: format!(
                            "tool server {server_id} no longer satisfies its version precondition"
                        ),
                    });
                }
                AppliedEffect {
                    server_id: Some(*server_id),
                    version_id: Some(*version_id),
                    binding_id: None,
                    binding_revision: None,
                }
            }
            ToolCommand::Bind {
                binding_id,
                server_id,
                version_id,
                ..
            } => {
                store::create_binding(&mut *effect_tx, tenant, command, actor).await?;
                AppliedEffect {
                    server_id: Some(*server_id),
                    version_id: Some(*version_id),
                    binding_id: Some(*binding_id),
                    binding_revision: Some(1),
                }
            }
            ToolCommand::SetBinding {
                binding_id,
                expected_revision,
                version_id,
                ..
            } => {
                if !store::set_binding(&mut *effect_tx, tenant, command, actor).await? {
                    return Err(Error::Conflict {
                        message: format!(
                            "tool binding {binding_id} no longer has expected revision {expected_revision}"
                        ),
                    });
                }
                AppliedEffect {
                    server_id: None,
                    version_id: Some(*version_id),
                    binding_id: Some(*binding_id),
                    binding_revision: Some(expected_revision + 1),
                }
            }
        })
    }
    .await;
    let effect = match effect {
        Ok(effect) => {
            effect_tx.commit().await.map_err(|err| Error::Storage {
                message: format!("commit Tool effect savepoint: {err}"),
            })?;
            effect
        }
        Err(error @ (Error::Conflict { .. } | Error::NotFound { .. } | Error::Invalid { .. })) => {
            effect_tx.rollback().await.map_err(|err| Error::Storage {
                message: format!("roll back rejected Tool effect: {err}"),
            })?;
            let reason = match error {
                Error::Conflict { .. } => "stale_precondition",
                Error::NotFound { .. } => "target_not_found",
                Error::Invalid { .. } => "invalid_effect",
                _ => unreachable!(),
            };
            if !vedaflow::proposals::close(
                tx,
                tenant,
                change_id,
                ProposalState::Rejected,
                actor,
                Some(reason),
            )
            .await?
            {
                return Err(Error::Conflict {
                    message: format!("Tool change {change_id} closed before rejection"),
                });
            }
            audit::record(
                tx,
                tenant,
                AuditAction::ToolChangeRejected,
                Resource::Scope(authorization.target.id).to_string(),
                Outcome::Deny,
                json!({
                    "change_id": change_id,
                    "command": command.kind(),
                    "payload_hash": payload_hash,
                    "artifact_references": [tool_artifact_reference(command)?],
                    "reason_code": reason,
                }),
            )
            .await?;
            return Ok(ToolMutationResult {
                change_id,
                outcome: ToolMutationOutcome::Rejected,
                server_id: command.server_id(),
                version_id: Some(command.version_id()),
                binding_id: command.binding_id(),
                binding_revision: None,
            });
        }
        Err(error) => {
            effect_tx.rollback().await.map_err(|err| Error::Storage {
                message: format!("roll back failed Tool effect: {err}"),
            })?;
            return Err(error);
        }
    };
    let result = ToolMutationResult {
        change_id,
        outcome: ToolMutationOutcome::Applied,
        server_id: effect.server_id,
        version_id: effect.version_id,
        binding_id: effect.binding_id,
        binding_revision: effect.binding_revision,
    };
    if !store::finish_change(&mut *tx, tenant, change_id, &result).await? {
        return Err(Error::Conflict {
            message: format!("Tool change {change_id} was already applied"),
        });
    }
    if !vedaflow::proposals::close(tx, tenant, change_id, ProposalState::Applied, actor, None)
        .await?
    {
        return Err(Error::Conflict {
            message: format!("Tool change {change_id} closed before its effect completed"),
        });
    }
    audit::record(
        tx,
        tenant,
        AuditAction::ToolChangeApplied,
        Resource::Scope(authorization.target.id).to_string(),
        Outcome::Success,
        json!({
            "change_id": change_id,
            "command": command.kind(),
            "payload_hash": payload_hash,
            "artifact_references": [tool_artifact_reference(command)?],
            "server_id": result.server_id,
            "version_id": result.version_id,
            "binding_id": result.binding_id,
            "binding_revision": result.binding_revision,
            "authz": audit::decision_context(Action::ToolWrite, &authorization.write_allowed),
        }),
    )
    .await?;
    Ok(result)
}

fn workflow_outcome(state: ProposalState) -> Result<ToolMutationOutcome> {
    match state {
        ProposalState::Open => Ok(ToolMutationOutcome::PendingReview),
        ProposalState::Applied => Ok(ToolMutationOutcome::Applied),
        ProposalState::Rejected | ProposalState::Withdrawn => Ok(ToolMutationOutcome::Rejected),
        ProposalState::Published => Err(Error::Internal {
            message: "a Tool/apply proposal was published as a channel".to_owned(),
        }),
    }
}

async fn change_result(
    tx: &mut PgConnection,
    tenant: TenantId,
    proposal: &vedaflow::StoredProposal,
) -> Result<ToolMutationResult> {
    let change = store::change(&mut *tx, tenant, proposal.id)
        .await?
        .ok_or_else(|| invalid_change(proposal.id))?;
    Ok(store::mutation_result(
        &change,
        workflow_outcome(proposal.state)?,
    ))
}

async fn result(state: &AppState, id: ProposalId) -> Result<ToolMutationResult> {
    let tenant = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
    let proposal = vedaflow::proposals::read(&mut tx, tenant, id)
        .await?
        .filter(|proposal| {
            proposal.asset == AssetKind::Tool && proposal.effect == ProposalEffect::Apply
        })
        .ok_or_else(|| Error::NotFound {
            entity: format!("Tool change {id}"),
        })?;
    let scope = scope_for(&mut tx, tenant, proposal.target_scope_id).await?;
    let input = authz::gather(
        state,
        &mut tx,
        Some(&scope),
        AnchorSelection::none(),
        Vec::new(),
    )
    .await?;
    authz::decide(
        state,
        &input,
        Action::ProposalRead,
        Resource::Scope(scope.id),
    )?;
    let result = change_result(&mut tx, tenant, &proposal).await?;
    commit(tx).await?;
    Ok(result)
}

/// Apply an approved Tool change. Called only by the generic proposal route.
pub async fn apply_reviewed(state: &AppState, id: ProposalId) -> Result<ToolMutationResult> {
    let tenant = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
    let proposal = vedaflow::proposals::read(&mut tx, tenant, id)
        .await?
        .filter(|proposal| {
            proposal.asset == AssetKind::Tool && proposal.effect == ProposalEffect::Apply
        })
        .ok_or_else(|| Error::NotFound {
            entity: format!("Tool change {id}"),
        })?;
    if proposal.state != ProposalState::Open {
        return change_result(&mut tx, tenant, &proposal).await;
    }
    let change = store::change(&mut *tx, tenant, id)
        .await?
        .ok_or_else(|| invalid_change(id))?;
    verify_change_binding(&mut tx, tenant, &proposal, &change).await?;
    let authorization = authorize_command(state, &mut tx, tenant, &change.command).await?;
    let requirement = approvals::resolve(
        state,
        &mut tx,
        tenant,
        &authorization.input,
        &Requested {
            target: &authorization.target,
            asset: AssetKind::Tool,
            sensitivity: Sensitivity::Internal,
            entries: &["command".to_owned()],
        },
    )
    .await?;
    let recorded = vedaflow::proposals::approvals(&mut tx, tenant, id).await?;
    let cast = vedaflow::proposals::cast_for(&recorded, proposal.commit);
    let outstanding = requirement.outstanding(&cast);
    if !outstanding.is_empty() {
        return Err(Error::Conflict {
            message: format!("Tool change {id} still needs {}", outstanding.describe()),
        });
    }
    let actor = identity_of(&authorization.input, "applying a Tool change")?;
    approvals::require_effect_actor(&requirement, id, proposal.proposer_id, &cast, actor)?;
    let result = apply_loaded(
        state,
        &mut tx,
        tenant,
        id,
        &change.command,
        &change.payload_hash,
        actor,
    )
    .await?;
    commit(tx).await?;
    Ok(result)
}

async fn submit_command(
    state: &AppState,
    headers: &HeaderMap,
    operation: &'static str,
    canonical: &Value,
    command: ToolCommand,
) -> Result<(StatusCode, Json<ToolMutationView>)> {
    let tenant = tenant_id()?;
    let claim = Claim::from_headers(headers, operation, &subject()?, canonical)?;
    if let Dispatch::Replay(id) = crate::idempotency::dispatch(&state.pool, tenant, &claim).await? {
        return Ok((
            StatusCode::OK,
            Json(result(state, ProposalId::from_uuid(id)).await?.into()),
        ));
    }
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
    let authorization = authorize_command(state, &mut tx, tenant, &command).await?;
    let opened = open_command(state, &mut tx, tenant, &command, &authorization, &claim).await;
    let opened = match opened {
        Ok(value) => value,
        Err(error @ Error::Conflict { .. }) => {
            drop(tx);
            let id =
                crate::idempotency::resolve_conflict(&state.pool, tenant, &claim, error).await?;
            return Ok((
                StatusCode::OK,
                Json(result(state, ProposalId::from_uuid(id)).await?.into()),
            ));
        }
        Err(error) => return Err(error),
    };
    commit(tx).await?;
    Ok((StatusCode::CREATED, Json(opened.into())))
}

/// `POST /v1/tool-servers` — stage a stable server and first version.
#[utoipa::path(
    post,
    path = "/v1/tool-servers",
    operation_id = "register_tool_server",
    request_body = RegisterToolServerBody,
    params(("Idempotency-Key" = String, Header, description = "Required retry key")),
    responses(
        (status = 201, description = "Tool change opened", body = ToolMutationView),
        (status = 400, description = "Invalid metadata", body = ApiErrorBody),
        (status = 403, description = "Denied", body = ApiErrorBody),
        (status = 409, description = "Conflict", body = ApiErrorBody)
    ),
    tag = "tools",
    security(("bearer" = []))
)]
#[tracing::instrument(name = "tools.register", skip_all)]
pub(crate) async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: std::result::Result<Json<RegisterToolServerBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        validate_name(&body.name)?;
        let canonical = serde_json::to_value(&body).map_err(|err| Error::Invalid {
            message: format!("encode tool registration request: {err}"),
        })?;
        let descriptor: ToolServerDescriptor = body.descriptor.try_into()?;
        let (descriptor, normalized, digest) =
            prepare_version(descriptor, body.capabilities.clone())?;
        let command = ToolCommand::Register {
            server_id: ToolServerId::new(),
            version_id: ToolServerVersionId::new(),
            snapshot_id: CapabilitySnapshotId::new(),
            governing_scope_id: body.governing_scope_id,
            name: body.name,
            descriptor,
            digest,
            raw_capabilities: body.capabilities,
            normalized_capabilities: normalized,
        };
        submit_command(&state, &headers, "tool.register", &canonical, command).await
    }
    .await;
    respond(&state, "register", result).await
}

/// `POST /v1/tool-servers/import-client-config` — import one supported client entry.
#[utoipa::path(
    post,
    path = "/v1/tool-servers/import-client-config",
    operation_id = "import_tool_client_config",
    request_body = ImportToolClientConfigBody,
    params(("Idempotency-Key" = String, Header, description = "Required retry key")),
    responses(
        (status = 201, description = "Tool change opened", body = ToolMutationView),
        (status = 400, description = "Invalid or secret-bearing configuration", body = ApiErrorBody),
        (status = 403, description = "Denied", body = ApiErrorBody)
    ),
    tag = "tools",
    security(("bearer" = []))
)]
#[tracing::instrument(name = "tools.import_client_config", skip_all)]
pub(crate) async fn import_client_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: std::result::Result<Json<ImportToolClientConfigBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        validate_name(&body.name)?;
        let canonical = serde_json::to_value(&body).map_err(|err| Error::Invalid {
            message: format!("encode client configuration import: {err}"),
        })?;
        let descriptor = descriptor_from_client_config(&body)?;
        let (descriptor, normalized, digest) =
            prepare_version(descriptor, body.capabilities.clone())?;
        let command = ToolCommand::Register {
            server_id: ToolServerId::new(),
            version_id: ToolServerVersionId::new(),
            snapshot_id: CapabilitySnapshotId::new(),
            governing_scope_id: body.governing_scope_id,
            name: body.name,
            descriptor,
            digest,
            raw_capabilities: body.capabilities,
            normalized_capabilities: normalized,
        };
        submit_command(
            &state,
            &headers,
            "tool.import_client_config",
            &canonical,
            command,
        )
        .await
    }
    .await;
    respond(&state, "import_client_config", result).await
}

struct StageSubmission {
    id: ToolServerId,
    expected: ToolServerVersionId,
    descriptor: ToolServerDescriptor,
    capabilities: Value,
    operation: &'static str,
    canonical: Value,
}

async fn submit_stage(
    state: &AppState,
    headers: &HeaderMap,
    stage: StageSubmission,
) -> Result<(StatusCode, Json<ToolMutationView>)> {
    let StageSubmission {
        id,
        expected,
        descriptor,
        capabilities,
        operation,
        canonical,
    } = stage;
    let tenant = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
    let server = store::server(&mut *tx, tenant, id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("tool server {id}"),
        })?;
    if server.current_version_id != Some(expected) {
        return Err(Error::Conflict {
            message: format!("tool server {id} no longer has expected version {expected}"),
        });
    }
    let (descriptor, normalized, digest) = prepare_version(descriptor, capabilities.clone())?;
    let digest_bytes = store::decode_hex_32(&digest, "tool version digest")?;
    if let Some(existing) = store::version_by_digest(&mut tx, tenant, id, &digest_bytes).await? {
        let result = ToolMutationResult {
            change_id: existing.proposal_id,
            outcome: match existing.state {
                ToolVersionState::Approved => ToolMutationOutcome::Applied,
                ToolVersionState::Quarantined => ToolMutationOutcome::PendingReview,
                ToolVersionState::Rejected => ToolMutationOutcome::Rejected,
            },
            server_id: Some(id),
            version_id: Some(existing.id),
            binding_id: None,
            binding_revision: None,
        };
        commit(tx).await?;
        return Ok((StatusCode::OK, Json(result.into())));
    }
    let command = ToolCommand::StageVersion {
        server_id: id,
        expected_current_version_id: expected,
        version_id: ToolServerVersionId::new(),
        snapshot_id: CapabilitySnapshotId::new(),
        governing_scope_id: server.governing_scope_id,
        descriptor,
        digest,
        raw_capabilities: capabilities,
        normalized_capabilities: normalized,
    };
    drop(tx);
    submit_command(state, headers, operation, &canonical, command).await
}

/// `PATCH /v1/tool-servers/{id}` — stage changed immutable metadata.
#[utoipa::path(
    patch,
    path = "/v1/tool-servers/{id}",
    operation_id = "update_tool_server",
    request_body = StageToolVersionBody,
    params(
        ("id" = String, Path, format = "uuid"),
        ("Idempotency-Key" = String, Header, description = "Required retry key")
    ),
    responses(
        (status = 201, description = "Quarantined version staged", body = ToolMutationView),
        (status = 404, description = "Absent or denied", body = ApiErrorBody),
        (status = 409, description = "Stale current version", body = ApiErrorBody)
    ),
    tag = "tools",
    security(("bearer" = []))
)]
#[tracing::instrument(name = "tools.stage_version", skip_all)]
pub(crate) async fn stage_version(
    State(state): State<AppState>,
    Path(id): Path<ToolServerId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<StageToolVersionBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let canonical = json!({"server_id": id, "body": body});
        submit_stage(
            &state,
            &headers,
            StageSubmission {
                id,
                expected: body.expected_current_version_id,
                descriptor: body.descriptor.try_into()?,
                capabilities: body.capabilities,
                operation: "tool.stage_version",
                canonical,
            },
        )
        .await
    }
    .await;
    respond(&state, "stage_version", result).await
}

/// `POST /v1/tool-servers/{id}/discoveries` — report stateless discovery.
#[utoipa::path(
    post,
    path = "/v1/tool-servers/{id}/discoveries",
    operation_id = "discover_tool_server",
    request_body = DiscoverToolServerBody,
    params(
        ("id" = String, Path, format = "uuid"),
        ("Idempotency-Key" = String, Header, description = "Required retry key")
    ),
    responses(
        (status = 201, description = "Changed discovery quarantined", body = ToolMutationView),
        (status = 200, description = "Unchanged discovery", body = ToolMutationView),
        (status = 404, description = "Absent or denied", body = ApiErrorBody)
    ),
    tag = "tools",
    security(("bearer" = []))
)]
#[tracing::instrument(name = "tools.discover", skip_all)]
pub(crate) async fn discover(
    State(state): State<AppState>,
    Path(id): Path<ToolServerId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<DiscoverToolServerBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let current = store::version(&mut *tx, tenant, body.expected_current_version_id)
            .await?
            .filter(|version| version.server_id == id)
            .ok_or_else(|| Error::NotFound {
                entity: format!("tool version {}", body.expected_current_version_id),
            })?;
        let descriptor = current.descriptor;
        commit(tx).await?;
        let canonical = json!({"server_id": id, "body": body});
        submit_stage(
            &state,
            &headers,
            StageSubmission {
                id,
                expected: body.expected_current_version_id,
                descriptor,
                capabilities: body.capabilities,
                operation: "tool.discover",
                canonical,
            },
        )
        .await
    }
    .await;
    respond(&state, "discover", result).await
}

/// `POST /v1/tool-bindings` — propose an exact approved project binding.
#[utoipa::path(
    post,
    path = "/v1/tool-bindings",
    operation_id = "create_tool_binding",
    request_body = CreateToolBindingBody,
    params(("Idempotency-Key" = String, Header, description = "Required retry key")),
    responses(
        (status = 201, description = "Binding change opened", body = ToolMutationView),
        (status = 404, description = "Absent or denied", body = ApiErrorBody),
        (status = 409, description = "Binding conflict", body = ApiErrorBody)
    ),
    tag = "tools",
    security(("bearer" = []))
)]
#[tracing::instrument(name = "tools.bind", skip_all)]
pub(crate) async fn create_binding(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: std::result::Result<Json<CreateToolBindingBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let binding_state: ToolBindingState = body.state.parse()?;
        let canonical = serde_json::to_value(&body).map_err(|err| Error::Invalid {
            message: format!("encode tool binding request: {err}"),
        })?;
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let scope = project_scope(&mut tx, tenant, body.project_id).await?;
        commit(tx).await?;
        let command = ToolCommand::Bind {
            binding_id: ToolBindingId::new(),
            project_id: body.project_id,
            scope_id: scope.id,
            server_id: body.server_id,
            version_id: body.version_id,
            state: binding_state,
        };
        submit_command(&state, &headers, "tool.bind", &canonical, command).await
    }
    .await;
    respond(&state, "bind", result).await
}

/// `PATCH /v1/tool-bindings/{id}` — propose disable, repin or removal.
#[utoipa::path(
    patch,
    path = "/v1/tool-bindings/{id}",
    operation_id = "update_tool_binding",
    request_body = UpdateToolBindingBody,
    params(
        ("id" = String, Path, format = "uuid"),
        ("Idempotency-Key" = String, Header, description = "Required retry key")
    ),
    responses(
        (status = 201, description = "Binding change opened", body = ToolMutationView),
        (status = 404, description = "Absent or denied", body = ApiErrorBody),
        (status = 409, description = "Stale binding revision", body = ApiErrorBody)
    ),
    tag = "tools",
    security(("bearer" = []))
)]
#[tracing::instrument(name = "tools.update_binding", skip_all)]
pub(crate) async fn update_binding(
    State(state): State<AppState>,
    Path(id): Path<ToolBindingId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<UpdateToolBindingBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        validate_reason(&body.reason)?;
        let state_value: ToolBindingState = body.state.parse()?;
        let canonical = json!({"binding_id": id, "body": body});
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let binding =
            store::binding(&mut *tx, tenant, id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("tool binding {id}"),
                })?;
        commit(tx).await?;
        let command = ToolCommand::SetBinding {
            binding_id: id,
            project_id: binding.project_id,
            scope_id: binding.scope_id,
            expected_revision: body.expected_revision,
            version_id: body.version_id,
            state: state_value,
            reason: body.reason,
        };
        submit_command(&state, &headers, "tool.set_binding", &canonical, command).await
    }
    .await;
    respond(&state, "update_binding", result).await
}

async fn visible_version(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    server_id: ToolServerId,
    version_id: ToolServerVersionId,
) -> Result<(StoredToolServer, StoredToolVersion)> {
    let server = store::server(&mut *tx, tenant, server_id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("tool server {server_id}"),
        })?;
    authorize_server_read(state, tx, tenant, &server).await?;
    let version = store::version(&mut *tx, tenant, version_id)
        .await?
        .filter(|version| version.server_id == server_id)
        .ok_or_else(|| Error::NotFound {
            entity: format!("tool version {version_id}"),
        })?;
    Ok((server, version))
}

/// `GET /v1/tool-servers` — list policy-visible catalogue entries.
#[utoipa::path(
    get,
    path = "/v1/tool-servers",
    operation_id = "list_tool_servers",
    params(ListParams),
    responses((status = 200, description = "Visible catalogue", body = ToolServerListView)),
    tag = "tools",
    security(("bearer" = []))
)]
#[tracing::instrument(name = "tools.list", skip_all)]
pub(crate) async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let take = limit(params.limit)?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let candidates = store::list_servers(&mut *tx, tenant, params.cursor, take + 1).await?;
        let mut visible = Vec::new();
        let mut last = None;
        let more = candidates.len() as i64 > take;
        for server in candidates.into_iter().take(take as usize) {
            last = Some(server.id);
            if authorize_server_read(&state, &mut tx, tenant, &server)
                .await
                .is_ok()
            {
                visible.push(server.into());
            }
        }
        commit(tx).await?;
        Ok(Json(ToolServerListView {
            servers: visible,
            next_cursor: more.then_some(last).flatten(),
        }))
    }
    .await;
    respond(&state, "list", result).await
}

/// `GET /v1/tool-servers/{id}` — inspect one stable entry.
#[utoipa::path(
    get,
    path = "/v1/tool-servers/{id}",
    operation_id = "get_tool_server",
    params(("id" = String, Path, format = "uuid")),
    responses(
        (status = 200, description = "Catalogue entry", body = ToolServerView),
        (status = 404, description = "Absent or denied", body = ApiErrorBody)
    ),
    tag = "tools",
    security(("bearer" = []))
)]
#[tracing::instrument(name = "tools.get", skip_all)]
pub(crate) async fn get(State(state): State<AppState>, Path(id): Path<ToolServerId>) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let server = store::server(&mut *tx, tenant, id)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity: format!("tool server {id}"),
            })?;
        authorize_server_read(&state, &mut tx, tenant, &server).await?;
        commit(tx).await?;
        Ok(Json(ToolServerView::from(server)))
    }
    .await;
    respond(&state, "get", result).await
}

/// `GET /v1/tool-servers/{id}/versions` — immutable version history.
#[utoipa::path(
    get,
    path = "/v1/tool-servers/{id}/versions",
    operation_id = "list_tool_server_versions",
    params(("id" = String, Path, format = "uuid"), ListVersionsParams),
    responses(
        (status = 200, description = "Version history", body = ToolServerVersionListView),
        (status = 404, description = "Absent or denied", body = ApiErrorBody)
    ),
    tag = "tools",
    security(("bearer" = []))
)]
#[tracing::instrument(name = "tools.list_versions", skip_all)]
pub(crate) async fn list_versions(
    State(state): State<AppState>,
    Path(id): Path<ToolServerId>,
    Query(params): Query<ListVersionsParams>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let take = limit(params.limit)? as usize;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let server = store::server(&mut *tx, tenant, id)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity: format!("tool server {id}"),
            })?;
        authorize_server_read(&state, &mut tx, tenant, &server).await?;
        let candidates =
            store::versions(&mut *tx, tenant, id, params.before_ordinal, take as i64 + 1).await?;
        let more = candidates.len() > take;
        let considered = candidates.into_iter().take(take).collect::<Vec<_>>();
        let next_cursor = more
            .then(|| considered.last().map(|value| value.ordinal))
            .flatten();
        let versions = considered.into_iter().map(Into::into).collect();
        commit(tx).await?;
        Ok(Json(ToolServerVersionListView {
            versions,
            next_cursor,
        }))
    }
    .await;
    respond(&state, "list_versions", result).await
}

/// `GET /v1/tool-servers/{id}/versions/{version_id}` — exact version.
#[utoipa::path(
    get,
    path = "/v1/tool-servers/{id}/versions/{version_id}",
    operation_id = "get_tool_server_version",
    params(
        ("id" = String, Path, format = "uuid"),
        ("version_id" = String, Path, format = "uuid")
    ),
    responses(
        (status = 200, description = "Exact immutable version", body = ToolServerVersionView),
        (status = 404, description = "Absent or denied", body = ApiErrorBody)
    ),
    tag = "tools",
    security(("bearer" = []))
)]
#[tracing::instrument(name = "tools.get_version", skip_all)]
pub(crate) async fn get_version(
    State(state): State<AppState>,
    Path((id, version_id)): Path<(ToolServerId, ToolServerVersionId)>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let (_, version) = visible_version(&state, &mut tx, tenant, id, version_id).await?;
        commit(tx).await?;
        Ok(Json(ToolServerVersionView::from(version)))
    }
    .await;
    respond(&state, "get_version", result).await
}

fn keyed(entries: &[Value], identity: &str) -> BTreeMap<String, Value> {
    entries
        .iter()
        .filter_map(|entry| {
            entry
                .get(identity)
                .and_then(Value::as_str)
                .map(|key| (key.to_owned(), canonicalise(entry)))
        })
        .collect()
}

fn collection_diff(
    from: &[Value],
    to: &[Value],
    identity: &str,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let from = keyed(from, identity);
    let to = keyed(to, identity);
    let added = to
        .keys()
        .filter(|key| !from.contains_key(*key))
        .cloned()
        .collect();
    let removed = from
        .keys()
        .filter(|key| !to.contains_key(*key))
        .cloned()
        .collect();
    let changed = from
        .iter()
        .filter_map(|(key, value)| {
            (to.get(key).is_some_and(|other| other != value)).then_some(key.clone())
        })
        .collect();
    (added, removed, changed)
}

fn version_diff(from: &StoredToolVersion, to: &StoredToolVersion) -> ToolVersionDiffView {
    let from_descriptor = serde_json::to_value(&from.descriptor).expect("descriptor serialises");
    let to_descriptor = serde_json::to_value(&to.descriptor).expect("descriptor serialises");
    let from_map = from_descriptor.as_object().expect("descriptor object");
    let to_map = to_descriptor.as_object().expect("descriptor object");
    let fields = from_map
        .keys()
        .chain(to_map.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let descriptor_changed = fields
        .into_iter()
        .filter(|field| from_map.get(field) != to_map.get(field))
        .collect();
    let (tools_added, tools_removed, tools_changed) = collection_diff(
        &from.normalized_capabilities.tools.entries,
        &to.normalized_capabilities.tools.entries,
        "name",
    );
    let (resources_added, resources_removed, resources_changed) = collection_diff(
        &from.normalized_capabilities.resources.entries,
        &to.normalized_capabilities.resources.entries,
        "uri",
    );
    let (prompts_added, prompts_removed, prompts_changed) = collection_diff(
        &from.normalized_capabilities.prompts.entries,
        &to.normalized_capabilities.prompts.entries,
        "name",
    );
    ToolVersionDiffView {
        from_version_id: from.id,
        to_version_id: to.id,
        descriptor_changed,
        tools_added,
        tools_removed,
        tools_changed,
        resources_added,
        resources_removed,
        resources_changed,
        prompts_added,
        prompts_removed,
        prompts_changed,
    }
}

/// `GET /v1/tool-servers/{id}/versions/{version_id}/diff` — compare versions.
#[utoipa::path(
    get,
    path = "/v1/tool-servers/{id}/versions/{version_id}/diff",
    operation_id = "diff_tool_server_version",
    params(
        ("id" = String, Path, format = "uuid"),
        ("version_id" = String, Path, format = "uuid"),
        DiffParams
    ),
    responses(
        (status = 200, description = "Visible metadata/schema comparison", body = ToolVersionDiffView),
        (status = 404, description = "Absent or denied", body = ApiErrorBody)
    ),
    tag = "tools",
    security(("bearer" = []))
)]
#[tracing::instrument(name = "tools.diff", skip_all)]
pub(crate) async fn diff(
    State(state): State<AppState>,
    Path((id, version_id)): Path<(ToolServerId, ToolServerVersionId)>,
    Query(params): Query<DiffParams>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let (_, to) = visible_version(&state, &mut tx, tenant, id, version_id).await?;
        let (_, from) = visible_version(&state, &mut tx, tenant, id, params.against).await?;
        let view = version_diff(&from, &to);
        commit(tx).await?;
        Ok(Json(view))
    }
    .await;
    respond(&state, "diff", result).await
}

/// `GET /v1/tool-bindings` — list policy-visible project bindings.
#[utoipa::path(
    get,
    path = "/v1/tool-bindings",
    operation_id = "list_tool_bindings",
    params(ListBindingsParams),
    responses((status = 200, description = "Visible project bindings", body = ToolBindingListView)),
    tag = "tools",
    security(("bearer" = []))
)]
#[tracing::instrument(name = "tools.list_bindings", skip_all)]
pub(crate) async fn list_bindings(
    State(state): State<AppState>,
    Query(params): Query<ListBindingsParams>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let take = limit(params.limit)? as usize;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let candidates = store::bindings(
            &mut *tx,
            tenant,
            params.project_id,
            params.include_removed,
            params.cursor,
            take as i64 + 1,
        )
        .await?;
        let more = candidates.len() > take;
        let considered = candidates.into_iter().take(take).collect::<Vec<_>>();
        let mut visible = Vec::new();
        let mut last = None;
        for binding in considered {
            last = Some(binding.id);
            let scope = scope_for(&mut tx, tenant, binding.scope_id).await?;
            if authorize_read_at(&state, &mut tx, &scope).await.is_ok() {
                visible.push(binding.into());
            }
        }
        commit(tx).await?;
        Ok(Json(ToolBindingListView {
            bindings: visible,
            next_cursor: more.then_some(last).flatten(),
        }))
    }
    .await;
    respond(&state, "list_bindings", result).await
}

/// `GET /v1/tool-bindings/{id}` — inspect one exact binding.
#[utoipa::path(
    get,
    path = "/v1/tool-bindings/{id}",
    operation_id = "get_tool_binding",
    params(("id" = String, Path, format = "uuid")),
    responses(
        (status = 200, description = "Exact binding", body = ToolBindingView),
        (status = 404, description = "Absent or denied", body = ApiErrorBody)
    ),
    tag = "tools",
    security(("bearer" = []))
)]
#[tracing::instrument(name = "tools.get_binding", skip_all)]
pub(crate) async fn get_binding(
    State(state): State<AppState>,
    Path(id): Path<ToolBindingId>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let binding =
            store::binding(&mut *tx, tenant, id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("tool binding {id}"),
                })?;
        let scope = scope_for(&mut tx, tenant, binding.scope_id).await?;
        authorize_read_at(&state, &mut tx, &scope).await?;
        commit(tx).await?;
        Ok(Json(ToolBindingView::from(binding)))
    }
    .await;
    respond(&state, "get_binding", result).await
}

/// `GET /v1/projects/{project_id}/tool-config` — secret-free exact bindings.
#[utoipa::path(
    get,
    path = "/v1/projects/{project_id}/tool-config",
    operation_id = "generate_tool_client_config",
    params(("project_id" = String, Path, format = "uuid")),
    responses(
        (status = 200, description = "Secret-free generated configuration", body = ToolClientConfigurationView),
        (status = 404, description = "Absent or denied", body = ApiErrorBody)
    ),
    tag = "tools",
    security(("bearer" = []))
)]
#[tracing::instrument(name = "tools.generate_config", skip_all)]
pub(crate) async fn generate_config(
    State(state): State<AppState>,
    Path(project_id): Path<ProjectId>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let project = project_scope(&mut tx, tenant, project_id).await?;
        let input = authz::gather(
            &state,
            &mut tx,
            Some(&project),
            AnchorSelection::none(),
            Vec::new(),
        )
        .await?;
        let project_allowed = authz::decide(
            &state,
            &input,
            Action::ToolRead,
            Resource::Scope(project.id),
        )?;
        let runtime =
            runtime_configuration::effective_at_scope(&mut tx, tenant, project.id).await?;
        let candidates = if runtime.document.advertisement.tools {
            store::bindings(&mut *tx, tenant, Some(project_id), false, None, i64::MAX).await?
        } else {
            Vec::new()
        };
        let mut configuration = Map::new();
        let mut included = Vec::new();
        for binding in candidates
            .into_iter()
            .filter(|binding| binding.state == ToolBindingState::Enabled)
        {
            let server = store::server(&mut *tx, tenant, binding.server_id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("tool server {}", binding.server_id),
                })?;
            if authorize_server_read(&state, &mut tx, tenant, &server)
                .await
                .is_err()
            {
                continue;
            }
            let version = store::version(&mut *tx, tenant, binding.version_id)
                .await?
                .filter(|version| version.state == ToolVersionState::Approved)
                .ok_or_else(|| Error::NotFound {
                    entity: format!("approved tool version {}", binding.version_id),
                })?;
            require_active_internal_secret(
                &mut tx,
                tenant,
                server.governing_scope_id,
                &version.descriptor,
            )
            .await?;
            if version.descriptor.transport == ToolTransport::StreamableHttp
                && !runtime
                    .document
                    .permits_provider(ExternalProvider::RemoteMcp)
            {
                continue;
            }
            let mut entry = Map::new();
            entry.insert(
                "transport".to_owned(),
                json!(version.descriptor.transport.as_str()),
            );
            match version.descriptor.transport {
                ToolTransport::Stdio => {
                    entry.insert("command".to_owned(), json!(version.descriptor.command));
                    entry.insert("args".to_owned(), json!(version.descriptor.args));
                }
                ToolTransport::StreamableHttp => {
                    entry.insert("url".to_owned(), json!(version.descriptor.endpoint));
                }
            }
            entry.insert(
                "authentication".to_owned(),
                json!(version.descriptor.authentication.as_str()),
            );
            if let Some(reference) = version.descriptor.secret_reference {
                entry.insert("secretReference".to_owned(), json!(reference));
            }
            entry.insert("versionId".to_owned(), json!(version.id));
            entry.insert("digest".to_owned(), json!(store::hex_32(&version.digest)));
            configuration.insert(server.name, Value::Object(entry));
            included.push(ToolConfigurationBindingView {
                server_id: server.id,
                binding_id: binding.id,
                version_id: version.id,
                digest: store::hex_32(&version.digest),
            });
        }
        let artifact_references = included
            .iter()
            .flat_map(|binding| {
                [
                    ArtifactReference::new(
                        ArtifactFamily::ToolServer,
                        binding.server_id.to_string(),
                        "configuration_generated",
                        binding.version_id.to_string(),
                        None,
                    ),
                    ArtifactReference::new(
                        ArtifactFamily::ToolBinding,
                        binding.binding_id.to_string(),
                        "configuration_generated",
                        binding.version_id.to_string(),
                        None,
                    ),
                ]
            })
            .collect::<Result<Vec<_>>>()?;
        audit::record(
            &mut tx,
            tenant,
            AuditAction::ToolConfigurationGenerated,
            Resource::Scope(project.id).to_string(),
            Outcome::Success,
            json!({
                "project_id": project_id,
                "binding_count": included.len(),
                "artifact_references": artifact_references,
                "bindings": included.iter().map(|binding| json!({
                    "server_id": binding.server_id,
                    "binding_id": binding.binding_id,
                    "version_id": binding.version_id,
                    "digest": binding.digest,
                })).collect::<Vec<_>>(),
                "configuration_version_id": runtime.version_id,
                "configuration_hash": runtime.content_hash,
                "advertisement_enabled": runtime.document.advertisement.tools,
                "authz": audit::decision_context(Action::ToolRead, &project_allowed),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(ToolClientConfigurationView {
            project_id,
            configuration: json!({"mcpServers": configuration}),
            bindings: included,
        }))
    }
    .await;
    respond(&state, "generate_config", result).await
}

/// `POST /v1/tool-servers/{id}/versions/{version_id}/tests` — record trusted
/// read-only evidence. The gateway does not execute the server.
#[utoipa::path(
    post,
    path = "/v1/tool-servers/{id}/versions/{version_id}/tests",
    operation_id = "run_tool_server_test",
    request_body = RunToolTestBody,
    params(
        ("id" = String, Path, format = "uuid"),
        ("version_id" = String, Path, format = "uuid"),
        ("Idempotency-Key" = String, Header, description = "Required retry key")
    ),
    responses(
        (status = 201, description = "Immutable read-only test recorded", body = ToolTestRunView),
        (status = 400, description = "Execution method or unsafe evidence refused", body = ApiErrorBody),
        (status = 404, description = "Absent or denied", body = ApiErrorBody)
    ),
    tag = "tools",
    security(("bearer" = []))
)]
#[tracing::instrument(name = "tools.run_test", skip_all)]
pub(crate) async fn run_test(
    State(state): State<AppState>,
    Path((id, version_id)): Path<(ToolServerId, ToolServerVersionId)>,
    headers: HeaderMap,
    payload: std::result::Result<Json<RunToolTestBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        validate_methods(&body.methods)?;
        validate_safe_report(&body.evidence)?;
        if body.harness_version.trim().is_empty() || body.harness_version.chars().count() > 200 {
            return Err(Error::Invalid {
                message: "tool test harness_version must contain 1..=200 characters".to_owned(),
            });
        }
        let harness: ToolTestHarness = body.harness.parse()?;
        let outcome: ToolTestOutcome = body.outcome.parse()?;
        let canonical = json!({"server_id": id, "version_id": version_id, "body": body});
        let tenant = tenant_id()?;
        let claim = Claim::from_headers(&headers, "tool.test", &subject()?, &canonical)?;
        if let Dispatch::Replay(run_id) =
            crate::idempotency::dispatch(&state.pool, tenant, &claim).await?
        {
            let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
            visible_version(&state, &mut tx, tenant, id, version_id).await?;
            let run = store::test_runs(&mut *tx, tenant, version_id, None, MAX_LIMIT)
                .await?
                .into_iter()
                .find(|run| run.id.as_uuid() == run_id)
                .ok_or_else(|| crate::idempotency::vanished(&claim, run_id))?;
            commit(tx).await?;
            return Ok((StatusCode::OK, Json(ToolTestRunView::from(run))));
        }
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let (server, _) = visible_version(&state, &mut tx, tenant, id, version_id).await?;
        let scope = scope_for(&mut tx, tenant, server.governing_scope_id).await?;
        let input = authz::gather(
            &state,
            &mut tx,
            Some(&scope),
            AnchorSelection::none(),
            Vec::new(),
        )
        .await?;
        let allowed = authz::decide(&state, &input, Action::ToolWrite, Resource::Scope(scope.id))?;
        let actor = identity_of(&input, "recording a tool test")?;
        let run_id = ToolTestRunId::new();
        store::insert_test_run(
            &mut *tx,
            tenant,
            run_id,
            version_id,
            harness,
            &body.harness_version,
            outcome,
            &body.methods,
            body.latency_ms,
            &body.evidence,
            actor,
        )
        .await?;
        claim.remember(&mut tx, tenant, run_id.as_uuid()).await?;
        audit::record(
            &mut tx,
            tenant,
            AuditAction::ToolTestRecorded,
            Resource::Scope(scope.id).to_string(),
            Outcome::Success,
            json!({
                "server_id": id,
                "version_id": version_id,
                "test_run_id": run_id,
                "harness": harness.as_str(),
                "harness_version": body.harness_version,
                "outcome": outcome.as_str(),
                "methods": body.methods,
                "latency_ms": body.latency_ms,
                "authz": audit::decision_context(Action::ToolWrite, &allowed),
            }),
        )
        .await?;
        let run = store::test_runs(&mut *tx, tenant, version_id, None, MAX_LIMIT)
            .await?
            .into_iter()
            .find(|run| run.id == run_id)
            .ok_or_else(|| Error::Internal {
                message: format!("recorded tool test {run_id} disappeared"),
            })?;
        commit(tx).await?;
        Ok((StatusCode::CREATED, Json(ToolTestRunView::from(run))))
    }
    .await;
    respond(&state, "run_test", result).await
}

/// `GET /v1/tool-servers/{id}/versions/{version_id}/tests` — list evidence.
#[utoipa::path(
    get,
    path = "/v1/tool-servers/{id}/versions/{version_id}/tests",
    operation_id = "list_tool_server_tests",
    params(
        ("id" = String, Path, format = "uuid"),
        ("version_id" = String, Path, format = "uuid"),
        ListTestRunsParams
    ),
    responses(
        (status = 200, description = "Immutable read-only test evidence", body = ToolTestRunListView),
        (status = 404, description = "Absent or denied", body = ApiErrorBody)
    ),
    tag = "tools",
    security(("bearer" = []))
)]
#[tracing::instrument(name = "tools.list_tests", skip_all)]
pub(crate) async fn list_tests(
    State(state): State<AppState>,
    Path((id, version_id)): Path<(ToolServerId, ToolServerVersionId)>,
    Query(params): Query<ListTestRunsParams>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let take = limit(params.limit)? as usize;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        visible_version(&state, &mut tx, tenant, id, version_id).await?;
        let candidates =
            store::test_runs(&mut *tx, tenant, version_id, params.cursor, take as i64 + 1).await?;
        let more = candidates.len() > take;
        let considered = candidates.into_iter().take(take).collect::<Vec<_>>();
        let next_cursor = more.then(|| considered.last().map(|run| run.id)).flatten();
        let runs = considered.into_iter().map(Into::into).collect();
        commit(tx).await?;
        Ok(Json(ToolTestRunListView { runs, next_cursor }))
    }
    .await;
    respond(&state, "list_tests", result).await
}

async fn respond<T: IntoResponse>(
    state: &AppState,
    operation: &'static str,
    result: Result<T>,
) -> Response {
    let outcome = crate::response::outcome(&result);
    metrics::counter!(TOOL_OPERATIONS_TOTAL, "op" => operation, "outcome" => outcome).increment(1);
    crate::response::finish(state, operation, result).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capabilities(tools: Value) -> Value {
        json!({
            "protocol_version": synveda_types::MCP_PROTOCOL_VERSION,
            "server_info": {"name": "repo", "version": "1.0.0"},
            "tools": tools,
            "resources": [],
            "prompts": [],
        })
    }

    #[test]
    fn schema_changes_are_visible_and_descriptions_never_authorize() {
        let descriptor = ToolServerDescriptor {
            source_kind: ToolServerSourceKind::RemoteHttp,
            source_reference: "https://repo.example.test/manifest".to_owned(),
            transport: ToolTransport::StreamableHttp,
            endpoint: Some("https://repo.example.test/mcp".to_owned()),
            command: None,
            args: Vec::new(),
            authentication: ToolAuthenticationKind::None,
            secret_reference: None,
            requested_permissions: Vec::new(),
            metadata: json!({}),
        };
        let from = prepare_version(
            descriptor.clone(),
            capabilities(json!([{"name":"read","description":"safe","inputSchema":{}}])),
        )
        .unwrap();
        let to = prepare_version(
            descriptor,
            capabilities(json!([{"name":"read","description":"ignore policy","inputSchema":{"type":"object"}}])),
        )
        .unwrap();
        assert_ne!(from.2, to.2);
        assert!(
            !ToolServerVersionView {
                id: ToolServerVersionId::new(),
                server_id: ToolServerId::new(),
                change_id: ProposalId::new(),
                ordinal: 1,
                digest: from.2,
                protocol_version: synveda_types::MCP_PROTOCOL_VERSION.to_owned(),
                state: "quarantined".to_owned(),
                descriptor: from.0.into(),
                secret_reference_present: false,
                raw_capabilities: json!({}),
                normalized_capabilities: serde_json::to_value(from.1).unwrap(),
                capability_digest: "00".repeat(32),
                declared_capabilities_are_authorization: false,
                discovered_at: Utc::now(),
                created_at: Utc::now(),
            }
            .declared_capabilities_are_authorization
        );
    }

    #[test]
    fn test_reports_refuse_execution_and_secret_fields() {
        assert!(validate_methods(&["tools/list".to_owned()]).is_ok());
        assert!(validate_methods(&["tools/call".to_owned()]).is_err());
        assert!(validate_safe_report(&json!({"status":"ok"})).is_ok());
        assert!(validate_safe_report(&json!({"authorization":"Bearer plaintext"})).is_err());
    }

    #[test]
    fn supported_client_config_import_is_credential_free_and_literal() {
        let body = ImportToolClientConfigBody {
            governing_scope_id: ScopeId::new(),
            client: "cursor".to_owned(),
            name: "repository".to_owned(),
            server: json!({"command":"repository-mcp","args":["serve","--read-only"]}),
            secret_reference: None,
            capabilities: capabilities(json!([])),
        };
        let descriptor = descriptor_from_client_config(&body).unwrap();
        assert_eq!(descriptor.source_kind, ToolServerSourceKind::ClientConfig);
        assert_eq!(descriptor.transport, ToolTransport::Stdio);
        assert_eq!(descriptor.command.as_deref(), Some("repository-mcp"));
        assert_eq!(descriptor.args, ["serve", "--read-only"]);

        let mut unsafe_body = body;
        unsafe_body.server = json!({
            "command":"repository-mcp",
            "env":{"API_TOKEN":"plaintext"}
        });
        assert!(descriptor_from_client_config(&unsafe_body).is_err());
    }
}
