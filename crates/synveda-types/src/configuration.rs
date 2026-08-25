//! Governed runtime configuration artifacts (CPR-30, ADR-0089).
//!
//! A document is complete, immutable content. `personal`, `team` and
//! `enterprise` are canonical source documents, not runtime branches: once a
//! template is chosen its fields are copied into an ordinary governed version
//! and every consumer reads those fields.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::knowledge::KnowledgeType;
use crate::relaxation::{PRODUCT_MAX_RELAXATION_SECS, RelaxationAction};
use crate::{
    ConfigurationArtifactId, ConfigurationBindingId, ConfigurationVersionId, Error, ProposalId,
    Result, ScopeId, TenantId, TraceRetentionMode,
};

/// Maximum configuration-artifact name length.
pub const MAX_CONFIGURATION_NAME_CHARS: usize = 100;
/// Maximum context budget accepted by one configuration version.
pub const MAX_CONTEXT_BUDGET_TOKENS: u32 = 100_000;
/// Maximum capture candidates one batch may retain after extraction.
pub const MAX_CONFIGURED_CAPTURE_CANDIDATES: u32 = 256;
/// Maximum default freshness interval. Zero means no implicit staleness date.
pub const MAX_FRESHNESS_DAYS: u32 = 36_500;
/// Product ceiling for one Knowledge graph expansion.
pub const MAX_GRAPH_EXPANDED_CANDIDATES: u32 = 256;
/// Product ceiling for adjacency rows considered from one frontier item.
pub const MAX_GRAPH_FAN_OUT: u32 = 32;
/// Product ceiling for a graph-expansion wall-clock budget.
pub const MAX_GRAPH_TIME_BUDGET_MS: u32 = 5_000;
/// Product ceiling for Knowledge content considered during graph expansion.
pub const MAX_GRAPH_TOKEN_BUDGET: u32 = 20_000;

macro_rules! closed_vocabulary {
    ($name:ident, [$($variant:ident => $wire:literal),+ $(,)?], $what:literal) => {
        impl $name {
            /// Every supported value in canonical order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// Stable wire and storage spelling.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $wire),+ }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(value: &str) -> Result<Self> {
                Self::ALL.iter().copied().find(|candidate| candidate.as_str() == value)
                    .ok_or_else(|| Error::Invalid {
                        message: format!("unknown {}: {value:?}", $what),
                    })
            }
        }
    };
}

/// Canonical source document a caller may copy into a governed version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationTemplate {
    /// Individual use with collaborative defaults and explicit external
    /// providers enabled.
    Personal,
    /// Small-team use with project sharing and conservative freshness.
    Team,
    /// Strict fail-safe document with no external provider enabled.
    Enterprise,
}

closed_vocabulary!(
    ConfigurationTemplate,
    [Personal => "personal", Team => "team", Enterprise => "enterprise"],
    "configuration template"
);

/// A context content channel selected by a configuration document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationContextChannel {
    /// Current, active, reviewed Knowledge revisions.
    CurrentKnowledge,
    /// Pending capture candidates, always rendered and traced as unreviewed.
    UnreviewedCandidates,
}

closed_vocabulary!(
    ConfigurationContextChannel,
    [
        CurrentKnowledge => "current_knowledge",
        UnreviewedCandidates => "unreviewed_candidates"
    ],
    "configuration context channel"
);

/// External dependency families a governed document may allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalProvider {
    /// Anthropic Messages API capture extraction.
    Anthropic,
    /// OpenAI-compatible vLLM capture extraction.
    Vllm,
    /// Text Embeddings Inference semantic embedding.
    Tei,
    /// Remote Streamable-HTTP MCP servers.
    RemoteMcp,
}

closed_vocabulary!(
    ExternalProvider,
    [
        Anthropic => "anthropic",
        Vllm => "vllm",
        Tei => "tei",
        RemoteMcp => "remote_mcp"
    ],
    "external provider family"
);

/// Rules deciding whether and how session evidence is extracted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureConfiguration {
    /// Master narrowing switch for capture.
    pub enabled: bool,
    /// Freeze a batch on a terminal session close.
    pub on_session_end: bool,
    /// Admit explicit capture-batch requests.
    pub explicit_request: bool,
    /// Minimum proposed confidence retained, in `0..=1000`.
    pub minimum_confidence_permille: u16,
    /// Maximum candidates retained for one batch after validation.
    pub maximum_candidates_per_batch: u32,
}

impl CaptureConfiguration {
    fn validate(&self) -> Result<()> {
        if self.minimum_confidence_permille > 1_000 {
            return Err(Error::Invalid {
                message: "capture minimum_confidence_permille must be in 0..=1000".to_owned(),
            });
        }
        if !(1..=MAX_CONFIGURED_CAPTURE_CANDIDATES).contains(&self.maximum_candidates_per_batch) {
            return Err(Error::Invalid {
                message: format!(
                    "capture maximum_candidates_per_batch must be in 1..={MAX_CONFIGURED_CAPTURE_CANDIDATES}"
                ),
            });
        }
        if !self.enabled && (self.on_session_end || self.explicit_request) {
            return Err(Error::Invalid {
                message: "disabled capture cannot enable session-end or explicit extraction"
                    .to_owned(),
            });
        }
        Ok(())
    }
}

/// Context-delivery and trace settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextConfiguration {
    /// Maximum estimated-token budget. A request may narrow, never widen it.
    pub token_budget: u32,
    /// Ordered, duplicate-free content channels.
    pub channels: Vec<ConfigurationContextChannel>,
    /// Diagnostic trace detail retained for a run.
    pub trace_retention: TraceRetentionMode,
    /// Bounded relationship expansion after ordinary retrieval has found
    /// authorised anchors.
    pub graph: GraphRetrievalConfiguration,
}

impl ContextConfiguration {
    fn validate(&self) -> Result<()> {
        if !(1..=MAX_CONTEXT_BUDGET_TOKENS).contains(&self.token_budget) {
            return Err(Error::Invalid {
                message: format!("context token_budget must be in 1..={MAX_CONTEXT_BUDGET_TOKENS}"),
            });
        }
        if self.channels.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(Error::Invalid {
                message: "context channels must be sorted and unique".to_owned(),
            });
        }
        self.graph.validate()?;
        Ok(())
    }

    /// Whether the document permits one context channel.
    #[must_use]
    pub fn permits(&self, channel: ConfigurationContextChannel) -> bool {
        self.channels.binary_search(&channel).is_ok()
    }
}

/// Governed bounds for anchor-first `KnowledgeRelation` expansion.
///
/// These values constrain candidate generation only. They grant no read
/// authority: every anchor, frontier and endpoint still receives an exact PDP
/// decision before it can influence rank or trace detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphRetrievalConfiguration {
    /// Master switch for relationship expansion.
    pub enabled: bool,
    /// Maximum relationship hops. The product ceiling is two.
    pub max_hops: u8,
    /// Maximum adjacency claims considered for one frontier item.
    pub fan_out_per_node: u32,
    /// Maximum distinct non-anchor candidates admitted from the graph.
    pub max_expanded_candidates: u32,
    /// Wall-clock budget for the isolated expansion transaction.
    pub time_budget_ms: u32,
    /// Maximum estimated content tokens admitted from expansion.
    pub token_budget: u32,
}

impl GraphRetrievalConfiguration {
    fn validate(&self) -> Result<()> {
        if !self.enabled {
            if self.max_hops != 0
                || self.fan_out_per_node != 0
                || self.max_expanded_candidates != 0
                || self.time_budget_ms != 0
                || self.token_budget != 0
            {
                return Err(Error::Invalid {
                    message: "disabled graph retrieval must have zero bounds".to_owned(),
                });
            }
            return Ok(());
        }
        if !(1..=2).contains(&self.max_hops) {
            return Err(Error::Invalid {
                message: "graph max_hops must be in 1..=2".to_owned(),
            });
        }
        if !(1..=MAX_GRAPH_FAN_OUT).contains(&self.fan_out_per_node) {
            return Err(Error::Invalid {
                message: format!("graph fan_out_per_node must be in 1..={MAX_GRAPH_FAN_OUT}"),
            });
        }
        if !(1..=MAX_GRAPH_EXPANDED_CANDIDATES).contains(&self.max_expanded_candidates) {
            return Err(Error::Invalid {
                message: format!(
                    "graph max_expanded_candidates must be in 1..={MAX_GRAPH_EXPANDED_CANDIDATES}"
                ),
            });
        }
        if !(1..=MAX_GRAPH_TIME_BUDGET_MS).contains(&self.time_budget_ms) {
            return Err(Error::Invalid {
                message: format!("graph time_budget_ms must be in 1..={MAX_GRAPH_TIME_BUDGET_MS}"),
            });
        }
        if !(1..=MAX_GRAPH_TOKEN_BUDGET).contains(&self.token_budget) {
            return Err(Error::Invalid {
                message: format!("graph token_budget must be in 1..={MAX_GRAPH_TOKEN_BUDGET}"),
            });
        }
        Ok(())
    }
}

/// Type-aware implicit staleness intervals. Zero leaves the revision without
/// an implicit date; an explicit revision `stale_after` always wins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreshnessConfiguration {
    /// Fact interval.
    pub fact_days: u32,
    /// Decision interval; commonly zero because decisions change explicitly.
    pub decision_days: u32,
    /// Preference interval.
    pub preference_days: u32,
    /// Procedure interval.
    pub procedure_days: u32,
    /// Entity interval.
    pub entity_days: u32,
    /// Episode interval; commonly zero because episodes become history.
    pub episode_days: u32,
    /// Convention interval.
    pub convention_days: u32,
    /// Warning interval.
    pub warning_days: u32,
    /// Reference interval.
    pub reference_days: u32,
}

impl FreshnessConfiguration {
    fn validate(&self) -> Result<()> {
        let values = [
            self.fact_days,
            self.decision_days,
            self.preference_days,
            self.procedure_days,
            self.entity_days,
            self.episode_days,
            self.convention_days,
            self.warning_days,
            self.reference_days,
        ];
        if values.into_iter().any(|days| days > MAX_FRESHNESS_DAYS) {
            return Err(Error::Invalid {
                message: format!("freshness defaults may not exceed {MAX_FRESHNESS_DAYS} days"),
            });
        }
        Ok(())
    }

    /// Implicit staleness interval for a Knowledge type.
    #[must_use]
    pub const fn days_for(&self, kind: KnowledgeType) -> u32 {
        match kind {
            KnowledgeType::Fact => self.fact_days,
            KnowledgeType::Decision => self.decision_days,
            KnowledgeType::Preference => self.preference_days,
            KnowledgeType::Procedure => self.procedure_days,
            KnowledgeType::Entity => self.entity_days,
            KnowledgeType::Episode => self.episode_days,
            KnowledgeType::Convention => self.convention_days,
            KnowledgeType::Warning => self.warning_days,
            KnowledgeType::Reference => self.reference_days,
        }
    }
}

/// Distribution controls. They narrow already-authorised bindings and grant
/// no Skill or Tool authority themselves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdvertisementConfiguration {
    /// Advertise policy-visible Skill bindings.
    pub skills: bool,
    /// Emit policy-visible Tool bindings in client configuration.
    pub tools: bool,
}

/// Policy-relaxation bounds. These settings can only narrow the closed
/// product vocabulary; Cedar remains the authority that turns a matched row
/// into a decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelaxationConfiguration {
    /// Master narrowing switch. Disabling it ends every standing relaxation
    /// under this effective configuration on the next request.
    pub enabled: bool,
    /// Maximum authority window calculated at effect time.
    pub maximum_duration_secs: u32,
    /// Sorted, duplicate-free subset of the product vocabulary.
    pub allowed_actions: Vec<RelaxationAction>,
}

impl RelaxationConfiguration {
    /// Validate the canonical narrowing shape.
    pub fn validate(&self) -> Result<()> {
        if self.maximum_duration_secs > PRODUCT_MAX_RELAXATION_SECS {
            return Err(Error::Invalid {
                message: format!(
                    "relaxation maximum_duration_secs must be at most {PRODUCT_MAX_RELAXATION_SECS}"
                ),
            });
        }
        if self
            .allowed_actions
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            return Err(Error::Invalid {
                message: "relaxation allowed_actions must be sorted and unique".to_owned(),
            });
        }
        if self.enabled {
            if self.maximum_duration_secs == 0 || self.allowed_actions.is_empty() {
                return Err(Error::Invalid {
                    message: "enabled relaxations need a positive duration and at least one action"
                        .to_owned(),
                });
            }
        } else if self.maximum_duration_secs != 0 || !self.allowed_actions.is_empty() {
            return Err(Error::Invalid {
                message: "disabled relaxations must use zero duration and no actions".to_owned(),
            });
        }
        Ok(())
    }

    /// Whether a standing row may participate under this current document.
    #[must_use]
    pub fn permits(&self, action: RelaxationAction) -> bool {
        self.enabled && self.allowed_actions.binary_search(&action).is_ok()
    }
}

/// One complete immutable runtime document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationDocument {
    /// Existing Cedar pack selected for authorisation and approval semantics.
    pub policy_pack: String,
    /// Capture/extraction rules.
    pub capture: CaptureConfiguration,
    /// Context budget, channels and trace rules.
    pub context: ContextConfiguration,
    /// Type-aware default freshness intervals.
    pub freshness: FreshnessConfiguration,
    /// Skill and Tool advertisement switches.
    pub advertisement: AdvertisementConfiguration,
    /// Time-boxed policy-relaxation bounds.
    pub relaxations: RelaxationConfiguration,
    /// Sorted, duplicate-free provider-family allowlist.
    pub allowed_external_providers: Vec<ExternalProvider>,
}

impl ConfigurationDocument {
    /// Return the canonical source document for a built-in template.
    #[must_use]
    pub fn template(template: ConfigurationTemplate) -> Self {
        let (policy_pack, capture, freshness, providers) = match template {
            ConfigurationTemplate::Personal => (
                "open-collaboration",
                CaptureConfiguration {
                    enabled: true,
                    on_session_end: true,
                    explicit_request: true,
                    minimum_confidence_permille: 350,
                    maximum_candidates_per_batch: 96,
                },
                FreshnessConfiguration {
                    fact_days: 60,
                    decision_days: 0,
                    preference_days: 0,
                    procedure_days: 180,
                    entity_days: 365,
                    episode_days: 0,
                    convention_days: 180,
                    warning_days: 30,
                    reference_days: 180,
                },
                vec![
                    ExternalProvider::Anthropic,
                    ExternalProvider::Vllm,
                    ExternalProvider::Tei,
                    ExternalProvider::RemoteMcp,
                ],
            ),
            ConfigurationTemplate::Team => (
                "standard",
                CaptureConfiguration {
                    enabled: true,
                    on_session_end: true,
                    explicit_request: true,
                    minimum_confidence_permille: 450,
                    maximum_candidates_per_batch: 64,
                },
                FreshnessConfiguration {
                    fact_days: 45,
                    decision_days: 0,
                    preference_days: 0,
                    procedure_days: 120,
                    entity_days: 180,
                    episode_days: 0,
                    convention_days: 90,
                    warning_days: 21,
                    reference_days: 90,
                },
                vec![
                    ExternalProvider::Anthropic,
                    ExternalProvider::Vllm,
                    ExternalProvider::Tei,
                    ExternalProvider::RemoteMcp,
                ],
            ),
            ConfigurationTemplate::Enterprise => (
                "regulated-strict",
                CaptureConfiguration {
                    enabled: true,
                    on_session_end: true,
                    explicit_request: true,
                    minimum_confidence_permille: 600,
                    maximum_candidates_per_batch: 48,
                },
                FreshnessConfiguration {
                    fact_days: 30,
                    decision_days: 0,
                    preference_days: 0,
                    procedure_days: 90,
                    entity_days: 90,
                    episode_days: 0,
                    convention_days: 60,
                    warning_days: 14,
                    reference_days: 60,
                },
                Vec::new(),
            ),
        };
        Self {
            policy_pack: policy_pack.to_owned(),
            capture,
            context: ContextConfiguration {
                token_budget: 1_500,
                channels: vec![ConfigurationContextChannel::CurrentKnowledge],
                trace_retention: match template {
                    ConfigurationTemplate::Personal => TraceRetentionMode::Full,
                    ConfigurationTemplate::Team => TraceRetentionMode::Redacted,
                    ConfigurationTemplate::Enterprise => TraceRetentionMode::HashesOnly,
                },
                graph: match template {
                    ConfigurationTemplate::Personal => GraphRetrievalConfiguration {
                        enabled: true,
                        max_hops: 2,
                        fan_out_per_node: 8,
                        max_expanded_candidates: 32,
                        time_budget_ms: 500,
                        token_budget: 512,
                    },
                    ConfigurationTemplate::Team => GraphRetrievalConfiguration {
                        enabled: true,
                        max_hops: 2,
                        fan_out_per_node: 6,
                        max_expanded_candidates: 24,
                        time_budget_ms: 350,
                        token_budget: 384,
                    },
                    ConfigurationTemplate::Enterprise => GraphRetrievalConfiguration {
                        enabled: true,
                        max_hops: 2,
                        fan_out_per_node: 4,
                        max_expanded_candidates: 16,
                        time_budget_ms: 250,
                        token_budget: 256,
                    },
                },
            },
            freshness,
            advertisement: AdvertisementConfiguration {
                skills: true,
                tools: true,
            },
            relaxations: RelaxationConfiguration {
                enabled: true,
                maximum_duration_secs: match template {
                    ConfigurationTemplate::Personal => 7 * 24 * 60 * 60,
                    ConfigurationTemplate::Team | ConfigurationTemplate::Enterprise => {
                        30 * 24 * 60 * 60
                    }
                },
                allowed_actions: vec![RelaxationAction::KnowledgeRead],
            },
            allowed_external_providers: providers,
        }
    }

    /// The conservative document used only while no binding exists.
    #[must_use]
    pub fn fail_safe() -> Self {
        Self::template(ConfigurationTemplate::Enterprise)
    }

    /// Validate every closed vocabulary, bound and canonical collection.
    pub fn validate(&self) -> Result<()> {
        if self.policy_pack.trim() != self.policy_pack
            || self.policy_pack.is_empty()
            || self.policy_pack.chars().count() > 100
            || self.policy_pack.chars().any(char::is_control)
        {
            return Err(Error::Invalid {
                message: "configuration policy_pack must contain 1..=100 non-control characters without surrounding whitespace".to_owned(),
            });
        }
        self.capture.validate()?;
        self.context.validate()?;
        self.freshness.validate()?;
        self.relaxations.validate()?;
        if self
            .allowed_external_providers
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            return Err(Error::Invalid {
                message: "allowed_external_providers must be sorted and unique".to_owned(),
            });
        }
        Ok(())
    }

    /// Whether a provider family is enabled.
    #[must_use]
    pub fn permits_provider(&self, provider: ExternalProvider) -> bool {
        self.allowed_external_providers
            .binary_search(&provider)
            .is_ok()
    }

    /// Canonical lowercase BLAKE3-256 content hash.
    pub fn content_hash(&self) -> Result<String> {
        self.validate()?;
        let value = crate::json::canonicalise(&serde_json::to_value(self).map_err(|error| {
            Error::Invalid {
                message: format!("encode configuration document: {error}"),
            }
        })?);
        Ok(blake3::hash(value.to_string().as_bytes())
            .to_hex()
            .to_string())
    }
}

/// Stable configuration aggregate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigurationArtifact {
    /// Stable id.
    pub id: ConfigurationArtifactId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Scope governing updates to the aggregate.
    pub governing_scope_id: ScopeId,
    /// Tenant-unique display name.
    pub name: String,
    /// Current published immutable version.
    pub current_version_id: ConfigurationVersionId,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Creating principal.
    pub created_by: String,
    /// Last head movement.
    pub updated_at: DateTime<Utc>,
    /// Principal that last moved the head.
    pub updated_by: String,
}

/// One immutable published document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigurationVersion {
    /// Immutable id.
    pub id: ConfigurationVersionId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Stable aggregate.
    pub artifact_id: ConfigurationArtifactId,
    /// Monotonic aggregate-local ordinal.
    pub ordinal: i64,
    /// Complete validated document.
    pub document: ConfigurationDocument,
    /// Canonical document hash.
    pub content_hash: String,
    /// Template copied to create this version, when applicable.
    pub source_template: Option<ConfigurationTemplate>,
    /// Typed VedaFlow change that applied it.
    pub proposal_id: ProposalId,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Creating principal.
    pub created_by: String,
}

/// Revisioned scope selector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigurationBinding {
    /// Stable binding id.
    pub id: ConfigurationBindingId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Scope whose subtree inherits this selection.
    pub scope_id: ScopeId,
    /// Selected aggregate.
    pub artifact_id: ConfigurationArtifactId,
    /// Exact pin; absent follows the aggregate current pointer.
    pub pinned_version_id: Option<ConfigurationVersionId>,
    /// Disabled bindings do not participate in resolution.
    pub enabled: bool,
    /// Optimistic concurrency revision.
    pub revision: u64,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Creating principal.
    pub created_by: String,
    /// Last transition.
    pub updated_at: DateTime<Utc>,
    /// Last actor.
    pub updated_by: String,
}

/// Resolved runtime answer, including exact immutable evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveConfiguration {
    /// Resource scope resolution was requested for.
    pub scope_id: ScopeId,
    /// Binding selected nearest-first, absent for the built-in fail-safe.
    pub binding_id: Option<ConfigurationBindingId>,
    /// Scope carrying that binding.
    pub binding_scope_id: Option<ScopeId>,
    /// Stable aggregate, absent for the built-in fail-safe.
    pub artifact_id: Option<ConfigurationArtifactId>,
    /// Exact immutable version, absent for the built-in fail-safe.
    pub version_id: Option<ConfigurationVersionId>,
    /// Canonical document digest, including for the fail-safe.
    pub content_hash: String,
    /// Complete runtime document.
    pub document: ConfigurationDocument,
}

/// Typed effect stored beside a VedaFlow `Configuration/apply` proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConfigurationCommand {
    /// Create an aggregate and first version.
    Create {
        /// Pre-minted aggregate id.
        artifact_id: ConfigurationArtifactId,
        /// Pre-minted version id.
        version_id: ConfigurationVersionId,
        /// Governing scope.
        governing_scope_id: ScopeId,
        /// Tenant-unique name.
        name: String,
        /// Complete document.
        document: ConfigurationDocument,
        /// Canonical document hash.
        content_hash: String,
        /// Template provenance.
        source_template: Option<ConfigurationTemplate>,
    },
    /// Publish another immutable version and advance current.
    Publish {
        /// Stable aggregate.
        artifact_id: ConfigurationArtifactId,
        /// Exact current head required at apply time.
        expected_current_version_id: ConfigurationVersionId,
        /// Pre-minted new version.
        version_id: ConfigurationVersionId,
        /// Governing scope repeated for integrity and authorisation.
        governing_scope_id: ScopeId,
        /// Complete document.
        document: ConfigurationDocument,
        /// Canonical document hash.
        content_hash: String,
        /// Template provenance.
        source_template: Option<ConfigurationTemplate>,
    },
    /// Create the sole selector at a scope.
    Bind {
        /// Pre-minted binding id.
        binding_id: ConfigurationBindingId,
        /// Target scope.
        scope_id: ScopeId,
        /// Selected aggregate.
        artifact_id: ConfigurationArtifactId,
        /// Exact pin, or follow current.
        pinned_version_id: Option<ConfigurationVersionId>,
        /// Initial enabled state.
        enabled: bool,
    },
    /// Change selection, enabled state or pin, including rollback.
    SetBinding {
        /// Stable binding.
        binding_id: ConfigurationBindingId,
        /// Target scope repeated for integrity.
        scope_id: ScopeId,
        /// Exact binding revision.
        expected_revision: u64,
        /// Complete resulting aggregate selection.
        artifact_id: ConfigurationArtifactId,
        /// Complete resulting pin state.
        pinned_version_id: Option<ConfigurationVersionId>,
        /// Complete resulting enabled state.
        enabled: bool,
        /// Bounded semantic reason (`bind`, `pin`, `unpin`, `rollback`,
        /// `enable`, `disable`).
        reason: String,
    },
}

impl ConfigurationCommand {
    /// Stable command kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Create { .. } => "create",
            Self::Publish { .. } => "publish",
            Self::Bind { .. } => "bind",
            Self::SetBinding { .. } => "set_binding",
        }
    }

    /// Scope at which the effect is governed.
    #[must_use]
    pub const fn scope_id(&self) -> ScopeId {
        match self {
            Self::Create {
                governing_scope_id, ..
            }
            | Self::Publish {
                governing_scope_id, ..
            } => *governing_scope_id,
            Self::Bind { scope_id, .. } | Self::SetBinding { scope_id, .. } => *scope_id,
        }
    }

    /// Stable aggregate named directly, when present.
    #[must_use]
    pub const fn artifact_id(&self) -> Option<ConfigurationArtifactId> {
        match self {
            Self::Create { artifact_id, .. }
            | Self::Publish { artifact_id, .. }
            | Self::Bind { artifact_id, .. }
            | Self::SetBinding { artifact_id, .. } => Some(*artifact_id),
        }
    }

    /// Immutable version minted or pinned, when present.
    #[must_use]
    pub const fn version_id(&self) -> Option<ConfigurationVersionId> {
        match self {
            Self::Create { version_id, .. } | Self::Publish { version_id, .. } => Some(*version_id),
            Self::Bind {
                pinned_version_id, ..
            }
            | Self::SetBinding {
                pinned_version_id, ..
            } => *pinned_version_id,
        }
    }

    /// Binding changed by the command, when present.
    #[must_use]
    pub const fn binding_id(&self) -> Option<ConfigurationBindingId> {
        match self {
            Self::Bind { binding_id, .. } | Self::SetBinding { binding_id, .. } => {
                Some(*binding_id)
            }
            Self::Create { .. } | Self::Publish { .. } => None,
        }
    }
}

/// Governance result for a configuration mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationMutationOutcome {
    /// Effect applied in the opening request.
    Applied,
    /// Change is open and awaiting review.
    PendingReview,
    /// Effect was terminally rejected.
    Rejected,
}

/// Common mutation response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigurationMutationResult {
    /// VedaFlow change id.
    pub change_id: ProposalId,
    /// Governance outcome.
    pub outcome: ConfigurationMutationOutcome,
    /// Stable artifact when known.
    pub artifact_id: Option<ConfigurationArtifactId>,
    /// Immutable version when known.
    pub version_id: Option<ConfigurationVersionId>,
    /// Stable binding when known.
    pub binding_id: Option<ConfigurationBindingId>,
    /// Resulting binding revision after application.
    pub binding_revision: Option<u64>,
}

/// Validate an aggregate display name.
pub fn validate_configuration_name(name: &str) -> Result<()> {
    if name.trim() != name
        || name.is_empty()
        || name.chars().count() > MAX_CONFIGURATION_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(Error::Invalid {
            message: format!(
                "configuration name must contain 1..={MAX_CONFIGURATION_NAME_CHARS} non-control characters without surrounding whitespace"
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_are_complete_valid_data_not_runtime_labels() {
        for template in ConfigurationTemplate::ALL {
            let document = ConfigurationDocument::template(*template);
            document.validate().unwrap();
            assert_eq!(document.content_hash().unwrap().len(), 64);
            assert_eq!(
                template.as_str().parse::<ConfigurationTemplate>().unwrap(),
                *template
            );
        }
        assert_eq!(
            ConfigurationDocument::fail_safe(),
            ConfigurationDocument::template(ConfigurationTemplate::Enterprise)
        );
    }

    #[test]
    fn canonical_hash_ignores_no_semantic_field() {
        let personal = ConfigurationDocument::template(ConfigurationTemplate::Personal);
        let mut changed = personal.clone();
        changed.context.token_budget += 1;
        assert_ne!(
            personal.content_hash().unwrap(),
            changed.content_hash().unwrap()
        );
    }

    #[test]
    fn collections_and_bounds_are_canonical() {
        let mut document = ConfigurationDocument::fail_safe();
        document.allowed_external_providers = vec![ExternalProvider::Tei, ExternalProvider::Tei];
        assert!(document.validate().is_err());
        document.allowed_external_providers = Vec::new();
        document.context.channels = vec![
            ConfigurationContextChannel::UnreviewedCandidates,
            ConfigurationContextChannel::CurrentKnowledge,
        ];
        assert!(document.validate().is_err());
        document.context.channels.clear();
        document.capture.maximum_candidates_per_batch = 0;
        assert!(document.validate().is_err());
    }

    #[test]
    fn freshness_is_type_aware_and_zero_means_no_default() {
        let document = ConfigurationDocument::template(ConfigurationTemplate::Team);
        assert_eq!(document.freshness.days_for(KnowledgeType::Fact), 45);
        assert_eq!(document.freshness.days_for(KnowledgeType::Decision), 0);
        assert_eq!(document.freshness.days_for(KnowledgeType::Warning), 21);
    }

    #[test]
    fn graph_bounds_are_closed_and_disabled_is_unambiguous() {
        let mut document = ConfigurationDocument::template(ConfigurationTemplate::Personal);
        document.context.graph.max_hops = 3;
        assert!(document.validate().is_err());

        document = ConfigurationDocument::template(ConfigurationTemplate::Personal);
        document.context.graph.enabled = false;
        assert!(
            document.validate().is_err(),
            "disabled with live bounds is ambiguous"
        );
        document.context.graph = GraphRetrievalConfiguration {
            enabled: false,
            max_hops: 0,
            fan_out_per_node: 0,
            max_expanded_candidates: 0,
            time_budget_ms: 0,
            token_budget: 0,
        };
        document.validate().unwrap();
    }
}
