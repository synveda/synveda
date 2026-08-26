//! Governed runtime-configuration API (CPR-30, ADR-0089).
//!
//! Templates are inert source documents. Every effective runtime setting is
//! selected by a revisioned scope binding to an immutable version, and every
//! mutation is a typed VedaFlow `Configuration/apply` change.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Acquire, PgConnection};
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{Action, Resource};
use synveda_store::anchors::AnchorSelection;
use synveda_store::{configuration as store, rls, scopes};
use synveda_types::configuration::{
    AdvertisementConfiguration, CaptureConfiguration, ConfigurationArtifact, ConfigurationBinding,
    ConfigurationCommand, ConfigurationContextChannel, ConfigurationDocument,
    ConfigurationMutationOutcome, ConfigurationMutationResult, ConfigurationTemplate,
    ConfigurationVersion, EffectiveConfiguration, ExternalProvider, FreshnessConfiguration,
    GraphRetrievalConfiguration, RelaxationConfiguration,
};
use synveda_types::json::canonicalise;
use synveda_types::relaxation::RelaxationAction;
use synveda_types::scope::Scope;
use synveda_types::{
    ArtifactFamily, ArtifactReference, AssetKind, ConfigurationArtifactId, ConfigurationBindingId,
    ConfigurationVersionId, Error, IdentityId, ProposalEffect, ProposalId, ProposalState, Result,
    ScopeId, Sensitivity, TenantId, TraceRetentionMode,
};
use synveda_vedaflow::{self as vedaflow, PolicySnapshot, Signer};
use utoipa::ToSchema;

use crate::app::AppState;
use crate::approvals::{self, Requested};
use crate::audit;
use crate::authz::{self, Authorized, DecisionInput};
use crate::error::ApiError;
use crate::idempotency::{Claim, Dispatch};
use crate::request::{body, commit, found, tenant_id};
use crate::workspaces::{ApiErrorBody, subject};

/// Configuration API outcomes by operation and `ok|rejected|error`.
pub const CONFIGURATION_OPERATIONS_TOTAL: &str = "synveda_configuration_operations_total";

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

fn mutation_outcome_schema() -> utoipa::openapi::schema::Object {
    crate::workspaces::string_enum(["applied", "pending_review", "rejected"].into_iter())
}

fn template_schema() -> utoipa::openapi::schema::Object {
    crate::workspaces::string_enum(
        ConfigurationTemplate::ALL
            .iter()
            .map(|value| value.as_str()),
    )
}

fn trace_schema() -> utoipa::openapi::schema::Object {
    crate::workspaces::string_enum(TraceRetentionMode::ALL.iter().map(|value| value.as_str()))
}

/// Capture and extraction settings in one immutable document.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CaptureConfigurationBody {
    pub enabled: bool,
    pub on_session_end: bool,
    pub explicit_request: bool,
    pub minimum_confidence_permille: u16,
    pub maximum_candidates_per_batch: u32,
}

/// Budgeted context delivery settings.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContextConfigurationBody {
    pub token_budget: u32,
    /// `current_knowledge`, optionally followed by `unreviewed_candidates`.
    pub channels: Vec<String>,
    #[schema(schema_with = trace_schema)]
    pub trace_retention: String,
    pub graph: GraphRetrievalConfigurationBody,
}

/// Bounded anchor-first Knowledge relationship expansion.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GraphRetrievalConfigurationBody {
    pub enabled: bool,
    pub max_hops: u8,
    pub fan_out_per_node: u32,
    pub max_expanded_candidates: u32,
    pub time_budget_ms: u32,
    pub token_budget: u32,
}

/// Type-aware implicit staleness intervals in days; zero disables the
/// implicit date for that type.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FreshnessConfigurationBody {
    pub fact_days: u32,
    pub decision_days: u32,
    pub preference_days: u32,
    pub procedure_days: u32,
    pub entity_days: u32,
    pub episode_days: u32,
    pub convention_days: u32,
    pub warning_days: u32,
    pub reference_days: u32,
}

/// Distribution switches. They narrow already-authorised bindings and grant
/// no Skill or Tool authority.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdvertisementConfigurationBody {
    pub skills: bool,
    pub tools: bool,
}

/// Time-boxed relaxation bounds. This document can narrow the closed product
/// vocabulary but never grants authority by itself.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RelaxationConfigurationBody {
    pub enabled: bool,
    pub maximum_duration_secs: u32,
    #[schema(value_type = Vec<String>)]
    pub allowed_actions: Vec<String>,
}

/// A complete immutable governed runtime document.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConfigurationDocumentBody {
    pub policy_pack: String,
    pub capture: CaptureConfigurationBody,
    pub context: ContextConfigurationBody,
    pub freshness: FreshnessConfigurationBody,
    pub advertisement: AdvertisementConfigurationBody,
    pub relaxations: RelaxationConfigurationBody,
    /// `anthropic`, `vllm`, `tei`, or `remote_mcp`, sorted and unique.
    pub allowed_external_providers: Vec<String>,
}

impl TryFrom<ConfigurationDocumentBody> for ConfigurationDocument {
    type Error = Error;

    fn try_from(value: ConfigurationDocumentBody) -> Result<Self> {
        let document = Self {
            policy_pack: value.policy_pack,
            capture: CaptureConfiguration {
                enabled: value.capture.enabled,
                on_session_end: value.capture.on_session_end,
                explicit_request: value.capture.explicit_request,
                minimum_confidence_permille: value.capture.minimum_confidence_permille,
                maximum_candidates_per_batch: value.capture.maximum_candidates_per_batch,
            },
            context: synveda_types::configuration::ContextConfiguration {
                token_budget: value.context.token_budget,
                channels: value
                    .context
                    .channels
                    .into_iter()
                    .map(|name| name.parse())
                    .collect::<Result<Vec<ConfigurationContextChannel>>>()?,
                trace_retention: value.context.trace_retention.parse()?,
                graph: GraphRetrievalConfiguration {
                    enabled: value.context.graph.enabled,
                    max_hops: value.context.graph.max_hops,
                    fan_out_per_node: value.context.graph.fan_out_per_node,
                    max_expanded_candidates: value.context.graph.max_expanded_candidates,
                    time_budget_ms: value.context.graph.time_budget_ms,
                    token_budget: value.context.graph.token_budget,
                },
            },
            freshness: FreshnessConfiguration {
                fact_days: value.freshness.fact_days,
                decision_days: value.freshness.decision_days,
                preference_days: value.freshness.preference_days,
                procedure_days: value.freshness.procedure_days,
                entity_days: value.freshness.entity_days,
                episode_days: value.freshness.episode_days,
                convention_days: value.freshness.convention_days,
                warning_days: value.freshness.warning_days,
                reference_days: value.freshness.reference_days,
            },
            advertisement: AdvertisementConfiguration {
                skills: value.advertisement.skills,
                tools: value.advertisement.tools,
            },
            relaxations: RelaxationConfiguration {
                enabled: value.relaxations.enabled,
                maximum_duration_secs: value.relaxations.maximum_duration_secs,
                allowed_actions: value
                    .relaxations
                    .allowed_actions
                    .into_iter()
                    .map(|name| name.parse())
                    .collect::<Result<Vec<RelaxationAction>>>()?,
            },
            allowed_external_providers: value
                .allowed_external_providers
                .into_iter()
                .map(|name| name.parse())
                .collect::<Result<Vec<ExternalProvider>>>()?,
        };
        document.validate()?;
        Ok(document)
    }
}

impl From<ConfigurationDocument> for ConfigurationDocumentBody {
    fn from(value: ConfigurationDocument) -> Self {
        Self {
            policy_pack: value.policy_pack,
            capture: CaptureConfigurationBody {
                enabled: value.capture.enabled,
                on_session_end: value.capture.on_session_end,
                explicit_request: value.capture.explicit_request,
                minimum_confidence_permille: value.capture.minimum_confidence_permille,
                maximum_candidates_per_batch: value.capture.maximum_candidates_per_batch,
            },
            context: ContextConfigurationBody {
                token_budget: value.context.token_budget,
                channels: value
                    .context
                    .channels
                    .into_iter()
                    .map(|channel| channel.as_str().to_owned())
                    .collect(),
                trace_retention: value.context.trace_retention.as_str().to_owned(),
                graph: GraphRetrievalConfigurationBody {
                    enabled: value.context.graph.enabled,
                    max_hops: value.context.graph.max_hops,
                    fan_out_per_node: value.context.graph.fan_out_per_node,
                    max_expanded_candidates: value.context.graph.max_expanded_candidates,
                    time_budget_ms: value.context.graph.time_budget_ms,
                    token_budget: value.context.graph.token_budget,
                },
            },
            freshness: FreshnessConfigurationBody {
                fact_days: value.freshness.fact_days,
                decision_days: value.freshness.decision_days,
                preference_days: value.freshness.preference_days,
                procedure_days: value.freshness.procedure_days,
                entity_days: value.freshness.entity_days,
                episode_days: value.freshness.episode_days,
                convention_days: value.freshness.convention_days,
                warning_days: value.freshness.warning_days,
                reference_days: value.freshness.reference_days,
            },
            advertisement: AdvertisementConfigurationBody {
                skills: value.advertisement.skills,
                tools: value.advertisement.tools,
            },
            relaxations: RelaxationConfigurationBody {
                enabled: value.relaxations.enabled,
                maximum_duration_secs: value.relaxations.maximum_duration_secs,
                allowed_actions: value
                    .relaxations
                    .allowed_actions
                    .into_iter()
                    .map(|action| action.as_str().to_owned())
                    .collect(),
            },
            allowed_external_providers: value
                .allowed_external_providers
                .into_iter()
                .map(|provider| provider.as_str().to_owned())
                .collect(),
        }
    }
}

/// Create an aggregate and its first immutable version.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateConfigurationBody {
    #[schema(value_type = String, format = "uuid")]
    pub governing_scope_id: ScopeId,
    pub name: String,
    pub document: ConfigurationDocumentBody,
    #[serde(default)]
    #[schema(schema_with = template_schema)]
    pub source_template: Option<String>,
}

/// Publish another complete immutable version.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PublishConfigurationBody {
    #[schema(value_type = String, format = "uuid")]
    pub expected_current_version_id: ConfigurationVersionId,
    pub document: ConfigurationDocumentBody,
    #[serde(default)]
    #[schema(schema_with = template_schema)]
    pub source_template: Option<String>,
}

/// Create a scope selector.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateConfigurationBindingBody {
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    #[schema(value_type = String, format = "uuid")]
    pub artifact_id: ConfigurationArtifactId,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub pinned_version_id: Option<ConfigurationVersionId>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Change a selector under an exact revision precondition.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateConfigurationBindingBody {
    pub expected_revision: u64,
    #[schema(value_type = String, format = "uuid")]
    pub artifact_id: ConfigurationArtifactId,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub pinned_version_id: Option<ConfigurationVersionId>,
    pub enabled: bool,
    pub reason: String,
}

/// Pin an older version without mutating history.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RollbackConfigurationBindingBody {
    pub expected_revision: u64,
    #[schema(value_type = String, format = "uuid")]
    pub version_id: ConfigurationVersionId,
}

fn default_true() -> bool {
    true
}

/// Stable result envelope for every governed mutation.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct ConfigurationMutationView {
    #[schema(value_type = String, format = "uuid")]
    pub change_id: ProposalId,
    #[schema(schema_with = mutation_outcome_schema)]
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub artifact_id: Option<ConfigurationArtifactId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub version_id: Option<ConfigurationVersionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub binding_id: Option<ConfigurationBindingId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding_revision: Option<u64>,
}

impl From<ConfigurationMutationResult> for ConfigurationMutationView {
    fn from(value: ConfigurationMutationResult) -> Self {
        Self {
            change_id: value.change_id,
            outcome: match value.outcome {
                ConfigurationMutationOutcome::Applied => "applied",
                ConfigurationMutationOutcome::PendingReview => "pending_review",
                ConfigurationMutationOutcome::Rejected => "rejected",
            }
            .to_owned(),
            artifact_id: value.artifact_id,
            version_id: value.version_id,
            binding_id: value.binding_id,
            binding_revision: value.binding_revision,
        }
    }
}

/// Canonical template source. Selecting one creates ordinary version data.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct ConfigurationTemplateView {
    #[schema(schema_with = template_schema)]
    pub name: String,
    pub content_hash: String,
    pub document: ConfigurationDocumentBody,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct ConfigurationTemplateListView {
    pub templates: Vec<ConfigurationTemplateView>,
}

/// Stable aggregate metadata.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct ConfigurationArtifactView {
    #[schema(value_type = String, format = "uuid")]
    pub id: ConfigurationArtifactId,
    #[schema(value_type = String, format = "uuid")]
    pub governing_scope_id: ScopeId,
    pub name: String,
    #[schema(value_type = String, format = "uuid")]
    pub current_version_id: ConfigurationVersionId,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    pub updated_at: DateTime<Utc>,
    pub updated_by: String,
}

impl From<ConfigurationArtifact> for ConfigurationArtifactView {
    fn from(value: ConfigurationArtifact) -> Self {
        Self {
            id: value.id,
            governing_scope_id: value.governing_scope_id,
            name: value.name,
            current_version_id: value.current_version_id,
            created_at: value.created_at,
            created_by: value.created_by,
            updated_at: value.updated_at,
            updated_by: value.updated_by,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct ConfigurationArtifactListView {
    pub artifacts: Vec<ConfigurationArtifactView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// One immutable version with its complete runtime document.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct ConfigurationVersionView {
    #[schema(value_type = String, format = "uuid")]
    pub id: ConfigurationVersionId,
    #[schema(value_type = String, format = "uuid")]
    pub artifact_id: ConfigurationArtifactId,
    pub ordinal: i64,
    pub document: ConfigurationDocumentBody,
    pub content_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(schema_with = template_schema)]
    pub source_template: Option<String>,
    #[schema(value_type = String, format = "uuid")]
    pub change_id: ProposalId,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
}

impl From<ConfigurationVersion> for ConfigurationVersionView {
    fn from(value: ConfigurationVersion) -> Self {
        Self {
            id: value.id,
            artifact_id: value.artifact_id,
            ordinal: value.ordinal,
            document: value.document.into(),
            content_hash: value.content_hash,
            source_template: value
                .source_template
                .map(|template| template.as_str().to_owned()),
            change_id: value.proposal_id,
            created_at: value.created_at,
            created_by: value.created_by,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct ConfigurationVersionListView {
    pub versions: Vec<ConfigurationVersionView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<i64>,
}

/// Revisioned scope selector.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct ConfigurationBindingView {
    #[schema(value_type = String, format = "uuid")]
    pub id: ConfigurationBindingId,
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    #[schema(value_type = String, format = "uuid")]
    pub artifact_id: ConfigurationArtifactId,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub pinned_version_id: Option<ConfigurationVersionId>,
    pub enabled: bool,
    pub revision: u64,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    pub updated_at: DateTime<Utc>,
    pub updated_by: String,
}

impl From<ConfigurationBinding> for ConfigurationBindingView {
    fn from(value: ConfigurationBinding) -> Self {
        Self {
            id: value.id,
            scope_id: value.scope_id,
            artifact_id: value.artifact_id,
            pinned_version_id: value.pinned_version_id,
            enabled: value.enabled,
            revision: value.revision,
            created_at: value.created_at,
            created_by: value.created_by,
            updated_at: value.updated_at,
            updated_by: value.updated_by,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct ConfigurationBindingListView {
    pub bindings: Vec<ConfigurationBindingView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub next_cursor: Option<ConfigurationBindingId>,
}

/// Exact current document and selector evidence at a governed scope.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct EffectiveConfigurationView {
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub binding_id: Option<ConfigurationBindingId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub binding_scope_id: Option<ScopeId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub artifact_id: Option<ConfigurationArtifactId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub version_id: Option<ConfigurationVersionId>,
    pub content_hash: String,
    pub document: ConfigurationDocumentBody,
    pub fail_safe: bool,
}

impl From<EffectiveConfiguration> for EffectiveConfigurationView {
    fn from(value: EffectiveConfiguration) -> Self {
        Self {
            scope_id: value.scope_id,
            binding_id: value.binding_id,
            binding_scope_id: value.binding_scope_id,
            artifact_id: value.artifact_id,
            version_id: value.version_id,
            content_hash: value.content_hash,
            document: value.document.into(),
            fail_safe: value.binding_id.is_none(),
        }
    }
}

/// Deterministic field-level comparison of two immutable versions.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct ConfigurationComparisonView {
    #[schema(value_type = String, format = "uuid")]
    pub from_version_id: ConfigurationVersionId,
    #[schema(value_type = String, format = "uuid")]
    pub to_version_id: ConfigurationVersionId,
    pub from_hash: String,
    pub to_hash: String,
    pub changed_fields: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListQuery {
    #[serde(default)]
    pub governing_scope_id: Option<ScopeId>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct VersionListQuery {
    #[serde(default)]
    pub cursor: Option<i64>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BindingListQuery {
    pub scope_id: ScopeId,
    #[serde(default)]
    pub cursor: Option<ConfigurationBindingId>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EffectiveQuery {
    pub scope_id: ScopeId,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CompareQuery {
    pub from: ConfigurationVersionId,
    pub to: ConfigurationVersionId,
}

fn default_limit() -> i64 {
    DEFAULT_LIMIT
}

fn checked_limit(limit: i64) -> Result<i64> {
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(Error::Invalid {
            message: format!("limit must be in 1..={MAX_LIMIT}"),
        });
    }
    Ok(limit)
}

async fn respond<T: IntoResponse>(
    state: &AppState,
    operation: &'static str,
    result: Result<T>,
) -> Response {
    let outcome = match &result {
        Ok(_) => "ok",
        Err(
            Error::Unauthenticated { .. }
            | Error::PolicyDenied { .. }
            | Error::NotFound { .. }
            | Error::Invalid { .. }
            | Error::Conflict { .. }
            | Error::RateLimited { .. },
        ) => "rejected",
        Err(_) => "error",
    };
    metrics::counter!(
        CONFIGURATION_OPERATIONS_TOTAL,
        "operation" => operation,
        "outcome" => outcome
    )
    .increment(1);
    match result {
        Ok(value) => value.into_response(),
        Err(error) => {
            audit::record_rejection(state, operation, &error).await;
            ApiError(error).into_response()
        }
    }
}

fn identity_of(input: &DecisionInput) -> Result<IdentityId> {
    input
        .identity
        .as_ref()
        .map(|identity| identity.id)
        .ok_or_else(|| Error::Invalid {
            message: "changing runtime configuration requires a provisioned identity".to_owned(),
        })
}

async fn scope_for(tx: &mut PgConnection, tenant: TenantId, id: ScopeId) -> Result<Scope> {
    found(scopes::get(&mut *tx, tenant, id).await?, tenant, id)
}

async fn read_authorized(
    state: &AppState,
    tx: &mut PgConnection,
    scope: &Scope,
) -> Result<Authorized> {
    let input = authz::gather(state, tx, Some(scope), AnchorSelection::none(), Vec::new()).await?;
    authz::decide(
        state,
        &input,
        Action::ConfigurationRead,
        Resource::Scope(scope.id),
    )
}

async fn tenant_read_authorized(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
) -> Result<Authorized> {
    authz::require(
        state,
        tx,
        Action::ConfigurationRead,
        Resource::Tenant(tenant),
        None,
    )
    .await
}

async fn read_event(
    tx: &mut PgConnection,
    tenant: TenantId,
    operation: &'static str,
    resource: Resource,
    allowed: &Authorized,
    details: Value,
) -> Result<()> {
    audit::record(
        tx,
        tenant,
        AuditAction::AuthzDecision,
        resource.to_string(),
        Outcome::Allow,
        json!({
            "operation": operation,
            "authz": audit::decision_context(Action::ConfigurationRead, allowed),
            "details": details,
        }),
    )
    .await
    .map(|_| ())
}

fn validate_template_provenance(
    document: &ConfigurationDocument,
    source_template: Option<ConfigurationTemplate>,
) -> Result<()> {
    if let Some(template) = source_template
        && *document != ConfigurationDocument::template(template)
    {
        return Err(Error::Invalid {
            message: format!(
                "source_template {template} may be retained only when the document is its exact canonical content"
            ),
        });
    }
    Ok(())
}

struct CommandAuthorization {
    target: Scope,
    input: DecisionInput,
    write_allowed: Authorized,
    proposal_allowed: Authorized,
}

async fn artifact_and_current(
    tx: &mut PgConnection,
    tenant: TenantId,
    id: ConfigurationArtifactId,
) -> Result<(ConfigurationArtifact, ConfigurationVersion)> {
    let artifact = store::artifact(&mut *tx, tenant, id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("configuration artifact {id}"),
        })?;
    let version = store::version(&mut *tx, tenant, artifact.current_version_id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!("configuration artifact {id} has no current version"),
        })?;
    Ok((artifact, version))
}

async fn ensure_binding_reachable(
    tx: &mut PgConnection,
    tenant: TenantId,
    target: ScopeId,
    governing: ScopeId,
) -> Result<()> {
    if target == governing {
        return Ok(());
    }
    let ancestors = scopes::ancestors(&mut *tx, tenant, target).await?;
    if ancestors.iter().any(|scope| scope.id == governing) {
        return Ok(());
    }
    Err(Error::Invalid {
        message: "a configuration may bind only at or below its governing scope".to_owned(),
    })
}

async fn authorize_command(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    command: &ConfigurationCommand,
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
    let write_allowed = authz::decide(
        state,
        &input,
        Action::ConfigurationWrite,
        Resource::Scope(target.id),
    )?;
    let proposal_allowed = authz::decide(
        state,
        &input,
        Action::ProposalOpen,
        Resource::Scope(target.id),
    )?;

    match command {
        ConfigurationCommand::Create {
            document,
            source_template,
            ..
        } => {
            document.validate()?;
            validate_template_provenance(document, *source_template)?;
            crate::policy::known_pack(tx, tenant, &document.policy_pack).await?;
        }
        ConfigurationCommand::Publish {
            artifact_id,
            expected_current_version_id,
            governing_scope_id,
            document,
            source_template,
            ..
        } => {
            let (artifact, current) = artifact_and_current(tx, tenant, *artifact_id).await?;
            if artifact.governing_scope_id != *governing_scope_id
                || current.id != *expected_current_version_id
            {
                return Err(Error::Conflict {
                    message: format!(
                        "configuration {artifact_id} no longer has expected version {expected_current_version_id}"
                    ),
                });
            }
            let governing = scope_for(tx, tenant, artifact.governing_scope_id).await?;
            read_authorized(state, tx, &governing).await?;
            document.validate()?;
            validate_template_provenance(document, *source_template)?;
            crate::policy::known_pack(tx, tenant, &document.policy_pack).await?;
        }
        ConfigurationCommand::Bind {
            artifact_id,
            pinned_version_id,
            ..
        } => {
            let (artifact, _) = artifact_and_current(tx, tenant, *artifact_id).await?;
            ensure_binding_reachable(tx, tenant, target.id, artifact.governing_scope_id).await?;
            if let Some(version_id) = pinned_version_id {
                store::version(&mut *tx, tenant, *version_id)
                    .await?
                    .filter(|version| version.artifact_id == *artifact_id)
                    .ok_or_else(|| Error::NotFound {
                        entity: format!("configuration version {version_id}"),
                    })?;
            }
        }
        ConfigurationCommand::SetBinding {
            binding_id,
            scope_id,
            expected_revision,
            artifact_id,
            pinned_version_id,
            ..
        } => {
            store::binding(&mut *tx, tenant, *binding_id)
                .await?
                .filter(|binding| {
                    binding.scope_id == *scope_id && binding.revision == *expected_revision
                })
                .ok_or_else(|| Error::Conflict {
                    message: format!(
                        "configuration binding {binding_id} is absent or not at revision {expected_revision}"
                    ),
                })?;
            let (artifact, _) = artifact_and_current(tx, tenant, *artifact_id).await?;
            ensure_binding_reachable(tx, tenant, target.id, artifact.governing_scope_id).await?;
            if let Some(version_id) = pinned_version_id {
                store::version(&mut *tx, tenant, *version_id)
                    .await?
                    .filter(|version| version.artifact_id == *artifact_id)
                    .ok_or_else(|| Error::NotFound {
                        entity: format!("configuration version {version_id}"),
                    })?;
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

fn command_payload_hash(command: &ConfigurationCommand) -> Result<String> {
    let value = canonicalise(
        &serde_json::to_value(command).map_err(|error| Error::Invalid {
            message: format!("encode Configuration command: {error}"),
        })?,
    );
    Ok(blake3::hash(value.to_string().as_bytes())
        .to_hex()
        .to_string())
}

fn configuration_artifact_reference(
    command: &ConfigurationCommand,
    payload_hash: &str,
) -> Result<ArtifactReference> {
    match command {
        ConfigurationCommand::Create {
            artifact_id,
            version_id,
            ..
        } => ArtifactReference::new(
            ArtifactFamily::Configuration,
            artifact_id.to_string(),
            command.kind(),
            version_id.to_string(),
            None,
        ),
        ConfigurationCommand::Publish {
            artifact_id,
            expected_current_version_id,
            version_id,
            ..
        } => ArtifactReference::new(
            ArtifactFamily::Configuration,
            artifact_id.to_string(),
            command.kind(),
            version_id.to_string(),
            Some(expected_current_version_id.to_string()),
        ),
        ConfigurationCommand::Bind {
            binding_id,
            pinned_version_id,
            ..
        } => ArtifactReference::new(
            ArtifactFamily::Configuration,
            binding_id.to_string(),
            command.kind(),
            pinned_version_id.map_or_else(|| payload_hash.to_owned(), |id| id.to_string()),
            None,
        ),
        ConfigurationCommand::SetBinding {
            binding_id,
            expected_revision,
            pinned_version_id,
            ..
        } => ArtifactReference::new(
            ArtifactFamily::Configuration,
            binding_id.to_string(),
            command.kind(),
            pinned_version_id.map_or_else(|| payload_hash.to_owned(), |id| id.to_string()),
            Some(expected_revision.to_string()),
        ),
    }
}

async fn open_command(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    command: &ConfigurationCommand,
    authorization: &CommandAuthorization,
    claim: &Claim,
) -> Result<ConfigurationMutationResult> {
    let actor = identity_of(&authorization.input)?;
    let payload_hash = command_payload_hash(command)?;
    let artifact_reference = configuration_artifact_reference(command, &payload_hash)?;
    let manifest = canonicalise(&json!({
        "command": command.kind(),
        "payload_hash": payload_hash,
        "artifact_id": command.artifact_id(),
        "version_id": command.version_id(),
        "binding_id": command.binding_id(),
    }));
    let bytes = serde_json::to_vec(&manifest).map_err(|error| Error::Internal {
        message: format!("encode Configuration change manifest: {error}"),
    })?;
    let object = vedaflow::put_object(tx, tenant, AssetKind::Configuration, &bytes).await?;
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
            asset: AssetKind::Configuration,
            effect: ProposalEffect::Apply,
            members: &members,
            artifact_references: &[artifact_reference],
            sensitivity: Sensitivity::Internal,
            title: &format!("{} runtime configuration", command.kind()),
            proposer: actor,
            proposer_subject: &authorization.input.principal.subject,
            committed_at: Utc::now(),
            policy_snapshot: &snapshot,
        },
        &Signer::Unsigned,
    )
    .await?;
    store::insert_change(tx, tenant, proposal.id, command, &payload_hash).await?;
    let requirement = approvals::resolve(
        state,
        tx,
        tenant,
        &authorization.input,
        &Requested {
            target: &authorization.target,
            asset: AssetKind::Configuration,
            sensitivity: Sensitivity::Internal,
            entries: &["command".to_owned()],
        },
    )
    .await?;
    // A fresh deployment otherwise deadlocks on its conservative fail-safe:
    // `regulated-strict` asks for two administrators before the first profile
    // can be selected, while first login can have created only one. The one
    // narrow bootstrap is still a typed, hashed, audited VedaFlow change: a
    // root administrator may auto-apply the tenant's first exact canonical
    // profile artifact and its first binding. Any edited document, second
    // artifact/binding or non-administrator uses the ordinary live matrix.
    let initial_profile_adoption =
        initial_profile_adoption(tx, tenant, command, authorization).await?;
    let outstanding = if initial_profile_adoption {
        synveda_types::Outstanding::default()
    } else {
        requirement.outstanding(&[])
    };
    audit::record(
        tx,
        tenant,
        AuditAction::ConfigurationChangeOpened,
        Resource::Scope(authorization.target.id).to_string(),
        Outcome::Success,
        json!({
            "change_id": proposal.id,
            "command": command.kind(),
            "payload_hash": payload_hash,
            "manifest_hash": object.hash.to_hex(),
            "artifact_references": &proposal.artifact_references,
            "artifact_id": command.artifact_id(),
            "version_id": command.version_id(),
            "binding_id": command.binding_id(),
            "authz": audit::decision_context(Action::ProposalOpen, &authorization.proposal_allowed),
            "approvals": approvals::audit_context(&requirement, &outstanding),
            "initial_profile_adoption": initial_profile_adoption,
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
        ConfigurationMutationResult {
            change_id: proposal.id,
            outcome: ConfigurationMutationOutcome::PendingReview,
            artifact_id: command.artifact_id(),
            version_id: command.version_id(),
            binding_id: command.binding_id(),
            binding_revision: None,
        }
    };
    claim.remember(tx, tenant, proposal.id.as_uuid()).await?;
    Ok(result)
}

/// Whether this is the one canonical profile adoption that makes a fresh
/// tenant operable without inventing a second bootstrap authority path.
async fn initial_profile_adoption(
    tx: &mut PgConnection,
    tenant: TenantId,
    command: &ConfigurationCommand,
    authorization: &CommandAuthorization,
) -> Result<bool> {
    if !authorization
        .write_allowed
        .roles
        .iter()
        .any(|role| role == "administrator")
    {
        return Ok(false);
    }

    // The absence checks below are one tenant-wide compare-and-apply. Two
    // workspaces can otherwise race through them and each become "first".
    // The root is immutable and already exists before an administrator can
    // reach this path, so it is the natural transaction-scoped mutex.
    let root = scopes::tenant_root(&mut *tx, tenant)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!("tenant {tenant} has no root scope"),
        })?;
    let root_administrator = authorization
        .input
        .anchors
        .get(root.id)
        .is_some_and(|anchor| {
            anchor
                .roles
                .iter()
                .any(|role| role.as_str() == "administrator")
                && anchor.granted_at.contains(&root.id)
        });
    if !root_administrator {
        return Ok(false);
    }
    scopes::lock_for_update(&mut *tx, tenant, root.id).await?;
    if !store::bindings(tx, tenant, None, None, 1).await?.is_empty() {
        return Ok(false);
    }

    match command {
        ConfigurationCommand::Create {
            document,
            source_template: Some(template),
            ..
        } => Ok(store::list_artifacts(tx, tenant, None, None, 1)
            .await?
            .is_empty()
            && *document == ConfigurationDocument::template(*template)),
        ConfigurationCommand::Bind {
            scope_id,
            artifact_id,
            ..
        } => {
            let artifacts = store::list_artifacts(tx, tenant, None, None, 2).await?;
            let [artifact] = artifacts.as_slice() else {
                return Ok(false);
            };
            if artifact.id != *artifact_id || artifact.governing_scope_id != *scope_id {
                return Ok(false);
            }
            let Some(version) = store::version(tx, tenant, artifact.current_version_id).await?
            else {
                return Ok(false);
            };
            Ok(version.source_template.is_some_and(|template| {
                version.document == ConfigurationDocument::template(template)
            }))
        }
        _ => Ok(false),
    }
}

async fn apply_loaded(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    change_id: ProposalId,
    command: &ConfigurationCommand,
    payload_hash: &str,
    actor: IdentityId,
) -> Result<ConfigurationMutationResult> {
    let authorization = authorize_command(state, tx, tenant, command).await?;
    let mut effect_tx = tx.begin().await.map_err(|error| Error::Storage {
        message: format!("begin Configuration effect savepoint: {error}"),
    })?;
    let applied = store::apply(
        &mut effect_tx,
        tenant,
        change_id,
        &actor.to_string(),
        command,
    )
    .await;
    let applied = match applied {
        Ok(value) => {
            effect_tx.commit().await.map_err(|error| Error::Storage {
                message: format!("commit Configuration effect savepoint: {error}"),
            })?;
            value
        }
        Err(error @ (Error::Conflict { .. } | Error::NotFound { .. } | Error::Invalid { .. })) => {
            effect_tx
                .rollback()
                .await
                .map_err(|rollback| Error::Storage {
                    message: format!("roll back rejected Configuration effect: {rollback}"),
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
                    message: format!(
                        "Configuration change {change_id} closed before rejection was recorded"
                    ),
                });
            }
            audit::record(
                tx,
                tenant,
                AuditAction::ConfigurationChangeRejected,
                Resource::Scope(authorization.target.id).to_string(),
                Outcome::Deny,
                json!({
                    "change_id": change_id,
                    "command": command.kind(),
                    "payload_hash": payload_hash,
                    "artifact_references": [configuration_artifact_reference(command, payload_hash)?],
                    "reason_code": reason,
                }),
            )
            .await?;
            return Ok(ConfigurationMutationResult {
                change_id,
                outcome: ConfigurationMutationOutcome::Rejected,
                artifact_id: command.artifact_id(),
                version_id: command.version_id(),
                binding_id: command.binding_id(),
                binding_revision: None,
            });
        }
        Err(error) => {
            effect_tx
                .rollback()
                .await
                .map_err(|rollback| Error::Storage {
                    message: format!("roll back failed Configuration effect: {rollback}"),
                })?;
            return Err(error);
        }
    };
    store::complete_change(tx, tenant, change_id, applied).await?;
    if !vedaflow::proposals::close(tx, tenant, change_id, ProposalState::Applied, actor, None)
        .await?
    {
        return Err(Error::Conflict {
            message: format!("Configuration change {change_id} closed before effect completion"),
        });
    }
    audit::record(
        tx,
        tenant,
        AuditAction::ConfigurationChangeApplied,
        Resource::Scope(authorization.target.id).to_string(),
        Outcome::Success,
        json!({
            "change_id": change_id,
            "command": command.kind(),
            "payload_hash": payload_hash,
            "artifact_references": [configuration_artifact_reference(command, payload_hash)?],
            "artifact_id": applied.artifact_id,
            "version_id": applied.version_id,
            "binding_id": applied.binding_id,
            "binding_revision": applied.binding_revision,
            "authz": audit::decision_context(Action::ConfigurationWrite, &authorization.write_allowed),
        }),
    )
    .await?;
    Ok(ConfigurationMutationResult {
        change_id,
        outcome: ConfigurationMutationOutcome::Applied,
        artifact_id: applied.artifact_id,
        version_id: applied.version_id,
        binding_id: applied.binding_id,
        binding_revision: applied.binding_revision,
    })
}

fn invalid_change(id: ProposalId) -> Error {
    Error::Internal {
        message: format!("Configuration change {id} failed its VedaFlow payload-integrity check"),
    }
}

async fn verify_change_binding(
    tx: &mut PgConnection,
    tenant: TenantId,
    proposal: &vedaflow::StoredProposal,
    change: &store::ConfigurationChange,
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
        && object.kind == AssetKind::Configuration
        && command_payload_hash(&change.command)? == change.payload_hash
        && manifest.get("command").and_then(Value::as_str) == Some(change.command.kind())
        && manifest.get("payload_hash").and_then(Value::as_str)
            == Some(change.payload_hash.as_str());
    if !valid {
        return Err(invalid_change(proposal.id));
    }
    Ok(())
}

fn proposal_outcome(state: ProposalState) -> Result<ConfigurationMutationOutcome> {
    match state {
        ProposalState::Open => Ok(ConfigurationMutationOutcome::PendingReview),
        ProposalState::Applied => Ok(ConfigurationMutationOutcome::Applied),
        ProposalState::Rejected | ProposalState::Withdrawn => {
            Ok(ConfigurationMutationOutcome::Rejected)
        }
        ProposalState::Published => Err(Error::Internal {
            message: "a Configuration/apply proposal was published as a channel".to_owned(),
        }),
    }
}

fn change_result(
    proposal: &vedaflow::StoredProposal,
    change: &store::ConfigurationChange,
) -> Result<ConfigurationMutationResult> {
    Ok(ConfigurationMutationResult {
        change_id: proposal.id,
        outcome: proposal_outcome(proposal.state)?,
        artifact_id: change
            .resulting_artifact_id
            .or_else(|| change.command.artifact_id()),
        version_id: change
            .resulting_version_id
            .or_else(|| change.command.version_id()),
        binding_id: change
            .resulting_binding_id
            .or_else(|| change.command.binding_id()),
        binding_revision: change.resulting_binding_revision,
    })
}

pub(crate) async fn result(
    state: &AppState,
    id: ProposalId,
) -> Result<ConfigurationMutationResult> {
    let tenant = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
    let proposal = vedaflow::proposals::read(&mut tx, tenant, id)
        .await?
        .filter(|proposal| {
            proposal.asset == AssetKind::Configuration && proposal.effect == ProposalEffect::Apply
        })
        .ok_or_else(|| Error::NotFound {
            entity: format!("Configuration change {id}"),
        })?;
    let target = scope_for(&mut tx, tenant, proposal.target_scope_id).await?;
    let allowed = read_authorized(state, &mut tx, &target).await?;
    let change = store::change(&mut tx, tenant, id)
        .await?
        .ok_or_else(|| invalid_change(id))?;
    let rendered = change_result(&proposal, &change)?;
    read_event(
        &mut tx,
        tenant,
        "configuration.change.result",
        Resource::Scope(target.id),
        &allowed,
        json!({"change_id": id}),
    )
    .await?;
    commit(tx).await?;
    Ok(rendered)
}

/// Apply an approved Configuration change from the generic proposal route.
pub(crate) async fn apply_reviewed(
    state: &AppState,
    id: ProposalId,
) -> Result<ConfigurationMutationResult> {
    let tenant = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
    let proposal = vedaflow::proposals::read(&mut tx, tenant, id)
        .await?
        .filter(|proposal| {
            proposal.asset == AssetKind::Configuration && proposal.effect == ProposalEffect::Apply
        })
        .ok_or_else(|| Error::NotFound {
            entity: format!("Configuration change {id}"),
        })?;
    if proposal.state != ProposalState::Open {
        let change = store::change(&mut tx, tenant, id)
            .await?
            .ok_or_else(|| invalid_change(id))?;
        return change_result(&proposal, &change);
    }
    let change = store::change(&mut tx, tenant, id)
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
            asset: AssetKind::Configuration,
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
            message: format!(
                "Configuration change {id} still needs {}",
                outstanding.describe()
            ),
        });
    }
    let actor = identity_of(&authorization.input)?;
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
    canonical: Value,
    command: ConfigurationCommand,
) -> Result<(StatusCode, Json<ConfigurationMutationView>)> {
    let tenant = tenant_id()?;
    let claim = Claim::from_headers(headers, operation, &subject()?, &canonical)?;
    if let Dispatch::Replay(id) = crate::idempotency::dispatch(&state.pool, tenant, &claim).await? {
        return Ok((
            StatusCode::OK,
            Json(result(state, ProposalId::from_uuid(id)).await?.into()),
        ));
    }
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
    let authorization = authorize_command(state, &mut tx, tenant, &command).await?;
    let opened = open_command(state, &mut tx, tenant, &command, &authorization, &claim).await;
    match opened {
        Ok(value) => {
            commit(tx).await?;
            Ok((StatusCode::CREATED, Json(value.into())))
        }
        Err(conflict @ Error::Conflict { .. }) => {
            drop(tx);
            let id =
                crate::idempotency::resolve_conflict(&state.pool, tenant, &claim, conflict).await?;
            Ok((
                StatusCode::OK,
                Json(result(state, ProposalId::from_uuid(id)).await?.into()),
            ))
        }
        Err(error) => Err(error),
    }
}

/// Create an immutable governed configuration aggregate.
#[utoipa::path(
    post,
    path = "/v1/configurations",
    operation_id = "create_configuration",
    tag = "configuration",
    params(("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry.")),
    request_body = CreateConfigurationBody,
    responses(
        (status = 201, description = "Change opened", body = ConfigurationMutationView),
        (status = 400, description = "Invalid document", body = ApiErrorBody),
        (status = 403, description = "Denied", body = ApiErrorBody),
        (status = 409, description = "Conflict", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "configuration.create", skip_all)]
pub(crate) async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: std::result::Result<Json<CreateConfigurationBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let canonical = serde_json::to_value(&body).map_err(|error| Error::Invalid {
            message: format!("encode configuration request: {error}"),
        })?;
        let document: ConfigurationDocument = body.document.try_into()?;
        let source_template = body
            .source_template
            .map(|value| value.parse())
            .transpose()?;
        let command = ConfigurationCommand::Create {
            artifact_id: ConfigurationArtifactId::new(),
            version_id: ConfigurationVersionId::new(),
            governing_scope_id: body.governing_scope_id,
            name: body.name,
            content_hash: document.content_hash()?,
            document,
            source_template,
        };
        submit_command(&state, &headers, "configuration.create", canonical, command).await
    }
    .await;
    respond(&state, "create", result).await
}

/// Publish and select another immutable version.
#[utoipa::path(
    post,
    path = "/v1/configurations/{id}/versions",
    operation_id = "publish_configuration_version",
    tag = "configuration",
    params(
        ("id" = String, Path, format = "uuid"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry.")
    ),
    request_body = PublishConfigurationBody,
    responses(
        (status = 201, description = "Change opened", body = ConfigurationMutationView),
        (status = 400, description = "Invalid document", body = ApiErrorBody),
        (status = 404, description = "Artifact absent", body = ApiErrorBody),
        (status = 409, description = "Stale current-version precondition", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "configuration.publish", skip_all)]
pub(crate) async fn publish(
    State(state): State<AppState>,
    Path(id): Path<ConfigurationArtifactId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<PublishConfigurationBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let canonical = json!({"artifact_id": id, "body": &body});
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let artifact =
            store::artifact(&mut tx, tenant, id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("configuration artifact {id}"),
                })?;
        drop(tx);
        let document: ConfigurationDocument = body.document.try_into()?;
        let source_template = body
            .source_template
            .map(|value| value.parse())
            .transpose()?;
        let command = ConfigurationCommand::Publish {
            artifact_id: id,
            expected_current_version_id: body.expected_current_version_id,
            version_id: ConfigurationVersionId::new(),
            governing_scope_id: artifact.governing_scope_id,
            content_hash: document.content_hash()?,
            document,
            source_template,
        };
        submit_command(
            &state,
            &headers,
            "configuration.publish",
            canonical,
            command,
        )
        .await
    }
    .await;
    respond(&state, "publish", result).await
}

/// Create one selector at a governed scope.
#[utoipa::path(
    post,
    path = "/v1/configuration-bindings",
    operation_id = "create_configuration_binding",
    tag = "configuration",
    params(("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry.")),
    request_body = CreateConfigurationBindingBody,
    responses(
        (status = 201, description = "Change opened", body = ConfigurationMutationView),
        (status = 400, description = "Invalid selector", body = ApiErrorBody),
        (status = 403, description = "Denied", body = ApiErrorBody),
        (status = 409, description = "Scope already has a selector", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "configuration.binding.create", skip_all)]
pub(crate) async fn create_binding(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: std::result::Result<Json<CreateConfigurationBindingBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let canonical = serde_json::to_value(&body).map_err(|error| Error::Invalid {
            message: format!("encode configuration-binding request: {error}"),
        })?;
        let command = ConfigurationCommand::Bind {
            binding_id: ConfigurationBindingId::new(),
            scope_id: body.scope_id,
            artifact_id: body.artifact_id,
            pinned_version_id: body.pinned_version_id,
            enabled: body.enabled,
        };
        submit_command(
            &state,
            &headers,
            "configuration.binding.create",
            canonical,
            command,
        )
        .await
    }
    .await;
    respond(&state, "binding.create", result).await
}

async fn set_binding_command(
    state: &AppState,
    headers: &HeaderMap,
    id: ConfigurationBindingId,
    body: UpdateConfigurationBindingBody,
    operation: &'static str,
) -> Result<(StatusCode, Json<ConfigurationMutationView>)> {
    let tenant = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
    let binding = store::binding(&mut tx, tenant, id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("configuration binding {id}"),
        })?;
    drop(tx);
    let canonical = json!({"binding_id": id, "body": &body});
    let command = ConfigurationCommand::SetBinding {
        binding_id: id,
        scope_id: binding.scope_id,
        expected_revision: body.expected_revision,
        artifact_id: body.artifact_id,
        pinned_version_id: body.pinned_version_id,
        enabled: body.enabled,
        reason: body.reason,
    };
    submit_command(state, headers, operation, canonical, command).await
}

/// Change, enable, disable, pin or unpin a binding.
#[utoipa::path(
    patch,
    path = "/v1/configuration-bindings/{id}",
    operation_id = "update_configuration_binding",
    tag = "configuration",
    params(
        ("id" = String, Path, format = "uuid"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry.")
    ),
    request_body = UpdateConfigurationBindingBody,
    responses(
        (status = 201, description = "Change opened", body = ConfigurationMutationView),
        (status = 404, description = "Binding absent", body = ApiErrorBody),
        (status = 409, description = "Stale revision", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "configuration.binding.update", skip_all)]
pub(crate) async fn update_binding(
    State(state): State<AppState>,
    Path(id): Path<ConfigurationBindingId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<UpdateConfigurationBindingBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        set_binding_command(&state, &headers, id, body, "configuration.binding.update").await
    }
    .await;
    respond(&state, "binding.update", result).await
}

/// Roll back by pinning one earlier immutable version.
#[utoipa::path(
    post,
    path = "/v1/configuration-bindings/{id}/rollback",
    operation_id = "rollback_configuration_binding",
    tag = "configuration",
    params(
        ("id" = String, Path, format = "uuid"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry.")
    ),
    request_body = RollbackConfigurationBindingBody,
    responses(
        (status = 201, description = "Rollback change opened", body = ConfigurationMutationView),
        (status = 404, description = "Binding or version absent", body = ApiErrorBody),
        (status = 409, description = "Stale revision", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "configuration.binding.rollback", skip_all)]
pub(crate) async fn rollback_binding(
    State(state): State<AppState>,
    Path(id): Path<ConfigurationBindingId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<RollbackConfigurationBindingBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let binding =
            store::binding(&mut tx, tenant, id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("configuration binding {id}"),
                })?;
        drop(tx);
        set_binding_command(
            &state,
            &headers,
            id,
            UpdateConfigurationBindingBody {
                expected_revision: body.expected_revision,
                artifact_id: binding.artifact_id,
                pinned_version_id: Some(body.version_id),
                enabled: true,
                reason: "rollback".to_owned(),
            },
            "configuration.binding.rollback",
        )
        .await
    }
    .await;
    respond(&state, "binding.rollback", result).await
}

fn encode_artifact_cursor(cursor: store::ArtifactCursor) -> String {
    URL_SAFE_NO_PAD.encode(format!(
        "cfg1|{}|{}",
        cursor.created_at.to_rfc3339(),
        cursor.id
    ))
}

fn decode_artifact_cursor(raw: &str) -> Result<store::ArtifactCursor> {
    let invalid = || Error::Invalid {
        message: "invalid configuration cursor".to_owned(),
    };
    let decoded = URL_SAFE_NO_PAD.decode(raw).map_err(|_| invalid())?;
    let decoded = std::str::from_utf8(&decoded).map_err(|_| invalid())?;
    let mut parts = decoded.split('|');
    if parts.next() != Some("cfg1") {
        return Err(invalid());
    }
    let created_at = parts
        .next()
        .ok_or_else(invalid)?
        .parse::<DateTime<Utc>>()
        .map_err(|_| invalid())?;
    let id = parts
        .next()
        .ok_or_else(invalid)?
        .parse::<ConfigurationArtifactId>()
        .map_err(|_| invalid())?;
    if parts.next().is_some() {
        return Err(invalid());
    }
    Ok(store::ArtifactCursor { created_at, id })
}

/// List canonical source templates. Templates are never effective by name.
#[utoipa::path(
    get,
    path = "/v1/configuration-templates",
    operation_id = "list_configuration_templates",
    tag = "configuration",
    responses(
        (status = 200, description = "Canonical source documents", body = ConfigurationTemplateListView),
        (status = 403, description = "Denied", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "configuration.templates", skip_all)]
pub(crate) async fn templates(State(state): State<AppState>) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let allowed = tenant_read_authorized(&state, &mut tx, tenant).await?;
        let templates = ConfigurationTemplate::ALL
            .iter()
            .copied()
            .map(|template| {
                let document = ConfigurationDocument::template(template);
                Ok(ConfigurationTemplateView {
                    name: template.as_str().to_owned(),
                    content_hash: document.content_hash()?,
                    document: document.into(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        read_event(
            &mut tx,
            tenant,
            "configuration.templates",
            Resource::Tenant(tenant),
            &allowed,
            json!({"count": templates.len()}),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(ConfigurationTemplateListView { templates }))
    }
    .await;
    respond(&state, "templates", result).await
}

/// List policy-visible stable aggregates with an opaque keyset cursor.
#[utoipa::path(
    get,
    path = "/v1/configurations",
    operation_id = "list_configurations",
    tag = "configuration",
    params(
        ("governing_scope_id" = Option<String>, Query, format = "uuid"),
        ("cursor" = Option<String>, Query),
        ("limit" = Option<i64>, Query, minimum = 1, maximum = 200)
    ),
    responses(
        (status = 200, description = "Visible aggregate page", body = ConfigurationArtifactListView),
        (status = 400, description = "Invalid cursor", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "configuration.list", skip_all)]
pub(crate) async fn list(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let limit = checked_limit(query.limit)?;
        let cursor = query
            .cursor
            .as_deref()
            .map(decode_artifact_cursor)
            .transpose()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let at_root = tenant_read_authorized(&state, &mut tx, tenant).await;
        let candidates =
            store::list_artifacts(&mut tx, tenant, query.governing_scope_id, cursor, limit + 1)
                .await?;
        let more = candidates.len() > usize::try_from(limit).unwrap_or(usize::MAX);
        let considered = candidates
            .into_iter()
            .take(usize::try_from(limit).unwrap_or(usize::MAX))
            .collect::<Vec<_>>();
        let last = considered.last().map(|artifact| store::ArtifactCursor {
            created_at: artifact.created_at,
            id: artifact.id,
        });
        let mut visible = Vec::new();
        let mut at_row = None;
        for artifact in considered {
            let scope = scope_for(&mut tx, tenant, artifact.governing_scope_id).await?;
            if let Ok(allowed) = read_authorized(&state, &mut tx, &scope).await {
                at_row.get_or_insert((allowed, scope.id));
                visible.push(artifact.into());
            }
        }
        let next_cursor = more.then(|| last.map(encode_artifact_cursor)).flatten();
        let (allowed, resource) = match (at_root, at_row) {
            (Ok(allowed), _) => (allowed, Resource::Tenant(tenant)),
            (Err(_), Some((allowed, scope_id))) => (allowed, Resource::Scope(scope_id)),
            (Err(denial), None) => return Err(denial),
        };
        read_event(
            &mut tx,
            tenant,
            "configuration.list",
            resource,
            &allowed,
            json!({"served": visible.len(), "more": next_cursor.is_some()}),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(ConfigurationArtifactListView {
            artifacts: visible,
            next_cursor,
        }))
    }
    .await;
    respond(&state, "list", result).await
}

/// Read one stable aggregate.
#[utoipa::path(
    get,
    path = "/v1/configurations/{id}",
    operation_id = "get_configuration",
    tag = "configuration",
    params(("id" = String, Path, format = "uuid")),
    responses(
        (status = 200, description = "Stable aggregate", body = ConfigurationArtifactView),
        (status = 404, description = "Absent or not visible", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "configuration.get", skip_all)]
pub(crate) async fn get(
    State(state): State<AppState>,
    Path(id): Path<ConfigurationArtifactId>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let artifact =
            store::artifact(&mut tx, tenant, id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("configuration artifact {id}"),
                })?;
        let scope = scope_for(&mut tx, tenant, artifact.governing_scope_id).await?;
        let allowed = read_authorized(&state, &mut tx, &scope).await?;
        read_event(
            &mut tx,
            tenant,
            "configuration.get",
            Resource::Scope(scope.id),
            &allowed,
            json!({"artifact_id": id}),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(ConfigurationArtifactView::from(artifact)))
    }
    .await;
    respond(&state, "get", result).await
}

/// List immutable versions newest first.
#[utoipa::path(
    get,
    path = "/v1/configurations/{id}/versions",
    operation_id = "list_configuration_versions",
    tag = "configuration",
    params(
        ("id" = String, Path, format = "uuid"),
        ("cursor" = Option<i64>, Query),
        ("limit" = Option<i64>, Query, minimum = 1, maximum = 200)
    ),
    responses(
        (status = 200, description = "Immutable version page", body = ConfigurationVersionListView),
        (status = 404, description = "Absent or not visible", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "configuration.versions", skip_all)]
pub(crate) async fn versions(
    State(state): State<AppState>,
    Path(id): Path<ConfigurationArtifactId>,
    Query(query): Query<VersionListQuery>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let limit = checked_limit(query.limit)?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let artifact =
            store::artifact(&mut tx, tenant, id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("configuration artifact {id}"),
                })?;
        let scope = scope_for(&mut tx, tenant, artifact.governing_scope_id).await?;
        let allowed = read_authorized(&state, &mut tx, &scope).await?;
        let rows = store::versions(&mut tx, tenant, id, query.cursor, limit + 1).await?;
        let more = rows.len() > usize::try_from(limit).unwrap_or(usize::MAX);
        let rows = rows
            .into_iter()
            .take(usize::try_from(limit).unwrap_or(usize::MAX))
            .collect::<Vec<_>>();
        let next_cursor = more
            .then(|| rows.last().map(|version| version.ordinal))
            .flatten();
        read_event(
            &mut tx,
            tenant,
            "configuration.versions",
            Resource::Scope(scope.id),
            &allowed,
            json!({"artifact_id": id, "served": rows.len()}),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(ConfigurationVersionListView {
            versions: rows.into_iter().map(Into::into).collect(),
            next_cursor,
        }))
    }
    .await;
    respond(&state, "versions", result).await
}

/// Resolve the nearest enabled selector and return exact version evidence.
#[utoipa::path(
    get,
    path = "/v1/configurations/effective",
    operation_id = "get_effective_configuration",
    tag = "configuration",
    params(("scope_id" = String, Query, format = "uuid")),
    responses(
        (status = 200, description = "Effective immutable document", body = EffectiveConfigurationView),
        (status = 404, description = "Scope absent", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "configuration.effective", skip_all)]
pub(crate) async fn effective(
    State(state): State<AppState>,
    Query(query): Query<EffectiveQuery>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let scope = scope_for(&mut tx, tenant, query.scope_id).await?;
        let allowed = read_authorized(&state, &mut tx, &scope).await?;
        let mut chain = vec![scope.id];
        chain.extend(
            scopes::ancestors(&mut *tx, tenant, scope.id)
                .await?
                .into_iter()
                .map(|ancestor| ancestor.id),
        );
        let effective = store::effective_for_chain(&mut tx, tenant, scope.id, &chain).await?;
        read_event(
            &mut tx,
            tenant,
            "configuration.effective",
            Resource::Scope(scope.id),
            &allowed,
            json!({
                "binding_id": effective.binding_id,
                "version_id": effective.version_id,
                "content_hash": effective.content_hash,
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(EffectiveConfigurationView::from(effective)))
    }
    .await;
    respond(&state, "effective", result).await
}

/// List revisioned bindings at one exact scope.
#[utoipa::path(
    get,
    path = "/v1/configuration-bindings",
    operation_id = "list_configuration_bindings",
    tag = "configuration",
    params(
        ("scope_id" = String, Query, format = "uuid"),
        ("cursor" = Option<String>, Query, format = "uuid"),
        ("limit" = Option<i64>, Query, minimum = 1, maximum = 200)
    ),
    responses(
        (status = 200, description = "Binding page", body = ConfigurationBindingListView),
        (status = 404, description = "Scope absent", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "configuration.bindings", skip_all)]
pub(crate) async fn bindings(
    State(state): State<AppState>,
    Query(query): Query<BindingListQuery>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let limit = checked_limit(query.limit)?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let scope = scope_for(&mut tx, tenant, query.scope_id).await?;
        let allowed = read_authorized(&state, &mut tx, &scope).await?;
        let rows =
            store::bindings(&mut tx, tenant, Some(scope.id), query.cursor, limit + 1).await?;
        let more = rows.len() > usize::try_from(limit).unwrap_or(usize::MAX);
        let rows = rows
            .into_iter()
            .take(usize::try_from(limit).unwrap_or(usize::MAX))
            .collect::<Vec<_>>();
        let next_cursor = more
            .then(|| rows.last().map(|binding| binding.id))
            .flatten();
        read_event(
            &mut tx,
            tenant,
            "configuration.bindings",
            Resource::Scope(scope.id),
            &allowed,
            json!({"served": rows.len()}),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(ConfigurationBindingListView {
            bindings: rows.into_iter().map(Into::into).collect(),
            next_cursor,
        }))
    }
    .await;
    respond(&state, "bindings", result).await
}

fn diff_value(prefix: &str, left: &Value, right: &Value, changed: &mut Vec<String>) {
    if left == right {
        return;
    }
    match (left, right) {
        (Value::Object(left), Value::Object(right)) => {
            let mut keys = left.keys().chain(right.keys()).collect::<Vec<_>>();
            keys.sort_unstable();
            keys.dedup();
            for key in keys {
                let child = if prefix.is_empty() {
                    key.to_owned()
                } else {
                    format!("{prefix}.{key}")
                };
                match (left.get(key), right.get(key)) {
                    (Some(left), Some(right)) => diff_value(&child, left, right, changed),
                    _ => changed.push(child),
                }
            }
        }
        _ => changed.push(prefix.to_owned()),
    }
}

/// Compare two versions of one stable aggregate.
#[utoipa::path(
    get,
    path = "/v1/configurations/{id}/compare",
    operation_id = "compare_configuration_versions",
    tag = "configuration",
    params(
        ("id" = String, Path, format = "uuid"),
        ("from" = String, Query, format = "uuid"),
        ("to" = String, Query, format = "uuid")
    ),
    responses(
        (status = 200, description = "Deterministic changed-field set", body = ConfigurationComparisonView),
        (status = 404, description = "Artifact or version absent", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "configuration.compare", skip_all)]
pub(crate) async fn compare(
    State(state): State<AppState>,
    Path(id): Path<ConfigurationArtifactId>,
    Query(query): Query<CompareQuery>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let artifact =
            store::artifact(&mut tx, tenant, id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("configuration artifact {id}"),
                })?;
        let scope = scope_for(&mut tx, tenant, artifact.governing_scope_id).await?;
        let allowed = read_authorized(&state, &mut tx, &scope).await?;
        let from = store::version(&mut tx, tenant, query.from)
            .await?
            .filter(|version| version.artifact_id == id)
            .ok_or_else(|| Error::NotFound {
                entity: format!("configuration version {}", query.from),
            })?;
        let to = store::version(&mut tx, tenant, query.to)
            .await?
            .filter(|version| version.artifact_id == id)
            .ok_or_else(|| Error::NotFound {
                entity: format!("configuration version {}", query.to),
            })?;
        let left = serde_json::to_value(&from.document).map_err(|error| Error::Internal {
            message: format!("encode configuration comparison: {error}"),
        })?;
        let right = serde_json::to_value(&to.document).map_err(|error| Error::Internal {
            message: format!("encode configuration comparison: {error}"),
        })?;
        let mut changed_fields = Vec::new();
        diff_value("", &left, &right, &mut changed_fields);
        read_event(
            &mut tx,
            tenant,
            "configuration.compare",
            Resource::Scope(scope.id),
            &allowed,
            json!({
                "artifact_id": id,
                "from_version_id": from.id,
                "to_version_id": to.id,
                "changed_count": changed_fields.len(),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(ConfigurationComparisonView {
            from_version_id: from.id,
            to_version_id: to.id,
            from_hash: from.content_hash,
            to_hash: to.content_hash,
            changed_fields,
        }))
    }
    .await;
    respond(&state, "compare", result).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_provenance_cannot_be_attached_to_changed_content() {
        let mut document = ConfigurationDocument::template(ConfigurationTemplate::Personal);
        document.context.token_budget += 1;
        assert!(
            validate_template_provenance(&document, Some(ConfigurationTemplate::Personal)).is_err()
        );
    }

    #[test]
    fn comparison_is_stable_and_field_granular() {
        let left = json!({"context": {"budget": 10}, "capture": true});
        let right = json!({"context": {"budget": 20}, "capture": true});
        let mut changed = Vec::new();
        diff_value("", &left, &right, &mut changed);
        assert_eq!(changed, vec!["context.budget"]);
    }
}
