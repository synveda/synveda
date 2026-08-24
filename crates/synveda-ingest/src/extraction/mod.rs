//! The Extractor seam (CPR-18, ADR-0083): classification into proposed
//! Knowledge plus summarise-at-write, behind one trait with
//! three implementations — Claude API, any OpenAI-compatible endpoint
//! (vLLM, the air-gapped path), and a deterministic rule-based extractor
//! that keeps dev, demos, and tests network-free (seed §2.1).
//!
//! Every type here is serde-serializable on purpose: the pipeline stages
//! are Temporal-shaped activities (ADR-0022 decision 1), and serializable
//! inputs/outputs are what lets the enterprise profile host the same
//! stages under a workflow engine without redesign.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use synveda_types::knowledge::{KnowledgeOrigin, KnowledgeType};
use synveda_types::session::SessionEventType;
use synveda_types::{Result, ScopeId, Sensitivity, SessionEventId, SessionId, TenantId};

mod claude;
mod deterministic;
mod prompt;
mod vllm;

pub use claude::ClaudeExtractor;
pub use deterministic::DeterministicExtractor;
pub use vllm::VllmExtractor;

/// Everything one extraction sees: the event's redacted content plus the run's
/// own provenance (`synveda_store::sessions::StagedEvent`, re-shaped as a
/// serializable activity input).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractionInput {
    /// The exact frozen session event being classified.
    pub event_id: SessionEventId,
    /// The owning tenant.
    pub tenant_id: TenantId,
    /// The governed scope the run was **decided at** — the default candidate
    /// placement (CPR-12, ADR-0078 decision 3).
    ///
    /// This was the submitter's own home scope until the observe cutover,
    /// because a correlation string could not say where a run happened. A
    /// session can: its scope is derived from its workspace and project by the
    /// schema and is unforgeable by a client. So a run against a shared
    /// project now proposes project Knowledge rather than one person's private
    /// pile of notes about shared work.
    pub scope_id: ScopeId,
    /// The run that produced the event.
    pub session_id: SessionId,
    /// The token subject that opened that run.
    pub principal_id: String,
    /// What happened; drives extraction routing.
    pub event_type: SessionEventType,
    /// The redacted event body. `[REDACTED:*]` placeholders are opaque
    /// tokens (ADR-0021): extractors preserve them verbatim, never guess
    /// at what they hid.
    pub payload: serde_json::Value,
    /// Client-asserted event time — becomes the proposed revision's valid-from.
    pub occurred_at: DateTime<Utc>,
    /// The admission scan's finding summary, carried into provenance.
    pub redactions: Option<serde_json::Value>,
}

/// One extracted, reviewable Knowledge proposal.
///
/// This is deliberately not a [`synveda_types::knowledge::KnowledgeRevision`]:
/// it has no stable item/revision ids and cannot become current until a
/// capture decision enters VedaFlow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateKnowledge {
    /// What the proposal says.
    pub knowledge_type: KnowledgeType,
    /// How the source event entered the session ledger. Set from the event
    /// type rather than chosen by a model.
    pub origin: KnowledgeOrigin,
    /// Human-readable proposed title.
    pub title: String,
    /// Proposed canonical Markdown body.
    pub body_markdown: String,
    /// Short proposed listing/retrieval summary.
    pub summary: String,
    /// Proposed canonical tags. The capture pipeline normalises and validates
    /// them before persistence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Self-reported confidence in `0.0..=1.0`. Uncalibrated until
    /// EVAL-2 measures it (ADR-0022 decision 3); the pipeline clamps.
    pub confidence: f64,
    /// The extractor's sensitivity proposal. The pipeline floors it at
    /// `internal` — auto-derived content is never `public` (ADR-0022
    /// decision 7). `None` means "no opinion".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitivity: Option<Sensitivity>,
    /// Entity mentions, a forward seam for MEM-5 dedup and GRPH-2
    /// graph-linking. Empty is normal today.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<String>,
}

/// One extraction's result: zero or more candidates plus the method and
/// model-version halves of the provenance quadruple.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractionOutcome {
    /// The candidates, in the extractor's order. Empty is a legal
    /// outcome: not every observation holds a memory.
    pub candidates: Vec<CandidateKnowledge>,
    /// The extraction method (`deterministic`, `claude-api`, `vllm`).
    pub method: String,
    /// The model (or ruleset) version that produced the candidates,
    /// e.g. `claude-opus-4-8` or `builtin@1`.
    pub model_version: String,
}

/// The extraction seam: classify one frozen event into Knowledge candidates
/// and summarise at capture time. Implementations must treat `[REDACTED:*]`
/// placeholders as opaque tokens and report failures as
/// [`synveda_types::Error::Dependency`] so the database-leased batch can be
/// retried (ADR-0083 decision 2).
///
/// `async fn` in a public trait is deliberate here: dispatch is static
/// through [`AnyExtractor`], never `dyn`, so the auto-trait caveat the
/// lint warns about cannot bite.
#[allow(async_fn_in_trait)]
pub trait Extractor {
    /// The stable method name recorded in provenance and metrics labels.
    fn method(&self) -> &'static str;

    /// Extracts proposed Knowledge from one frozen event.
    async fn extract(&self, input: &ExtractionInput) -> Result<ExtractionOutcome>;
}

/// The configured extractor, dispatched statically (no `dyn`, no boxed
/// futures — the worker holds exactly one of these for its lifetime).
#[derive(Debug, Clone)]
pub enum AnyExtractor {
    /// The rule-based, network-free default (seed §2.1: zero-config).
    Deterministic(DeterministicExtractor),
    /// The Anthropic Messages API (ADR-0022 decision 3).
    Claude(ClaudeExtractor),
    /// Any OpenAI-compatible `/v1/chat/completions` endpoint — vLLM is
    /// the air-gapped deployment named by the tech plan (§1.3).
    Vllm(VllmExtractor),
}

impl Extractor for AnyExtractor {
    fn method(&self) -> &'static str {
        match self {
            AnyExtractor::Deterministic(inner) => inner.method(),
            AnyExtractor::Claude(inner) => inner.method(),
            AnyExtractor::Vllm(inner) => inner.method(),
        }
    }

    async fn extract(&self, input: &ExtractionInput) -> Result<ExtractionOutcome> {
        match self {
            AnyExtractor::Deterministic(inner) => inner.extract(input).await,
            AnyExtractor::Claude(inner) => inner.extract(input).await,
            AnyExtractor::Vllm(inner) => inner.extract(input).await,
        }
    }
}
