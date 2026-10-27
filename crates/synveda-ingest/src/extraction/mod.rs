//! The Extractor seam (ADR-0083): classification into proposed
//! Knowledge plus summarise-at-write, behind one trait with
//! three implementations — Claude API, any OpenAI-compatible endpoint
//! (vLLM, the air-gapped path), and a deterministic rule-based extractor
//! that keeps dev, demos, and tests network-free (seed §2.1).
//!
//! Every type here is serde-serializable on purpose: durable execution can
//! persist and replay bounded stage inputs and outputs without coupling this
//! domain boundary to a particular scheduler or queue implementation.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use synveda_types::capture::CaptureFailureCode;
use synveda_types::knowledge::{KnowledgeOrigin, KnowledgeType};
use synveda_types::session::SessionEventType;
use synveda_types::{Error, ScopeId, Sensitivity, SessionEventId, SessionId, TenantId};

mod claude;
mod deterministic;
mod prompt;
mod vllm;

pub use claude::ClaudeExtractor;
pub use deterministic::DeterministicExtractor;
pub use vllm::VllmExtractor;

/// Everything one extraction sees: the event's redacted content plus the run's
/// own provenance (`synveda_store::sessions::StagedEvent`, re-shaped as a
/// serializable stage input).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractionInput {
    /// The exact frozen session event being classified.
    pub event_id: SessionEventId,
    /// The owning tenant.
    pub tenant_id: TenantId,
    /// The governed scope the run was **decided at** — the default candidate
    /// placement (ADR-0078 decision 3).
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
    /// the extraction suite measures it (ADR-0022 decision 3); the pipeline clamps.
    pub confidence: f64,
    /// The extractor's sensitivity proposal. The pipeline floors it at
    /// `internal` — auto-derived content is never `public` (ADR-0022
    /// decision 7). `None` means "no opinion".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitivity: Option<Sensitivity>,
    /// Entity mentions, a forward seam for dedup and graph linking
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

/// Why one provider call failed, as a closed content-free classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderFailure {
    /// The request exceeded its client deadline.
    Timeout,
    /// No connection to the configured endpoint could be made.
    Unreachable,
    /// The transport failed after the request began.
    Interrupted,
    /// HTTP 429.
    RateLimited,
    /// HTTP 503 or the Anthropic overload status 529.
    Unavailable,
    /// HTTP 408, 500, 502 or 504.
    Transient,
    /// Any other non-success status, including an unfollowed redirect.
    Rejected,
    /// HTTP 401 or 403.
    Unauthorized,
    /// A success response outside the extraction contract.
    ResponseInvalid,
    /// The client could not be configured.
    Misconfigured,
}

impl ProviderFailure {
    /// Classifies a non-success HTTP status.
    #[must_use]
    pub const fn from_status(status: u16) -> Self {
        match status {
            429 => Self::RateLimited,
            503 | 529 => Self::Unavailable,
            408 | 500 | 502 | 504 => Self::Transient,
            401 | 403 => Self::Unauthorized,
            _ => Self::Rejected,
        }
    }

    /// Classifies a transport error without retaining its message.
    #[must_use]
    pub fn from_transport(error: &reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::Timeout
        } else if error.is_connect() {
            Self::Unreachable
        } else {
            Self::Interrupted
        }
    }

    /// The persisted Capture failure code.
    #[must_use]
    pub const fn code(self) -> CaptureFailureCode {
        match self {
            Self::Timeout => CaptureFailureCode::ProviderTimeout,
            Self::Unreachable => CaptureFailureCode::ProviderUnreachable,
            Self::Interrupted => CaptureFailureCode::ProviderInterrupted,
            Self::RateLimited => CaptureFailureCode::ProviderRateLimited,
            Self::Unavailable => CaptureFailureCode::ProviderUnavailable,
            Self::Transient => CaptureFailureCode::ProviderTransient,
            Self::Rejected => CaptureFailureCode::ProviderRejected,
            Self::Unauthorized => CaptureFailureCode::ProviderUnauthorized,
            Self::ResponseInvalid => CaptureFailureCode::ProviderResponseInvalid,
            Self::Misconfigured => CaptureFailureCode::ProviderMisconfigured,
        }
    }

    /// Whether a provider `Retry-After` hint may be honoured for this class.
    #[must_use]
    pub const fn admits_retry_after(self) -> bool {
        matches!(self, Self::RateLimited | Self::Unavailable)
    }
}

/// One failed extraction: a closed classification and an optional, already
/// capped provider retry hint. It carries no provider body, header value or
/// credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractionError {
    service: &'static str,
    failure: ProviderFailure,
    retry_after: Option<Duration>,
}

impl ExtractionError {
    /// A failure without a provider hint.
    #[must_use]
    pub const fn new(service: &'static str, failure: ProviderFailure) -> Self {
        Self {
            service,
            failure,
            retry_after: None,
        }
    }

    /// A non-success HTTP status, keeping a `Retry-After` hint only where the
    /// status admits one.
    #[must_use]
    pub fn status(
        service: &'static str,
        status: u16,
        retry_after: Option<&reqwest::header::HeaderValue>,
    ) -> Self {
        let failure = ProviderFailure::from_status(status);
        let retry_after = retry_after
            .filter(|_| failure.admits_retry_after())
            .and_then(|value| value.to_str().ok())
            .and_then(|value| crate::retry::parse_retry_after(value, std::time::SystemTime::now()));
        Self {
            service,
            failure,
            retry_after,
        }
    }

    /// The closed classification.
    #[must_use]
    pub const fn failure(&self) -> ProviderFailure {
        self.failure
    }

    /// The capped provider hint, if one was sent and admitted.
    #[must_use]
    pub const fn retry_after(&self) -> Option<Duration> {
        self.retry_after
    }

    /// The generic content-free dependency error for callers outside Capture.
    #[must_use]
    pub fn into_error(self) -> Error {
        Error::Dependency {
            service: self.service.to_owned(),
            message: self.failure.code().as_str().to_owned(),
        }
    }
}

impl std::fmt::Display for ExtractionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.service, self.failure.code())
    }
}

impl std::error::Error for ExtractionError {}

/// The extraction seam: classify one frozen event into Knowledge candidates
/// and summarise at capture time. Implementations must treat `[REDACTED:*]`
/// placeholders as opaque tokens and report failures through the closed
/// [`ExtractionError`] so the worker can schedule or close the durable batch
/// (ADR-0158).
///
/// `async fn` in a public trait is deliberate here: dispatch is static
/// through [`AnyExtractor`], never `dyn`, so the auto-trait caveat the
/// lint warns about cannot bite.
#[allow(async_fn_in_trait)]
pub trait Extractor {
    /// The stable method name recorded in provenance and metrics labels.
    fn method(&self) -> &'static str;

    /// Extracts proposed Knowledge from one frozen event.
    async fn extract(
        &self,
        input: &ExtractionInput,
    ) -> std::result::Result<ExtractionOutcome, ExtractionError>;
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

    async fn extract(
        &self,
        input: &ExtractionInput,
    ) -> std::result::Result<ExtractionOutcome, ExtractionError> {
        match self {
            AnyExtractor::Deterministic(inner) => inner.extract(input).await,
            AnyExtractor::Claude(inner) => inner.extract(input).await,
            AnyExtractor::Vllm(inner) => inner.extract(input).await,
        }
    }
}

impl AnyExtractor {
    /// Content-free digest of the external provider identity whose capacity
    /// Capture reserves, or `None` for the network-free extractor.
    ///
    /// The identity is the method and normalised endpoint. Models and
    /// credentials at one endpoint deliberately share one capacity pool.
    #[must_use]
    pub fn provider_key(&self) -> Option<[u8; 32]> {
        let (method, endpoint) = match self {
            AnyExtractor::Deterministic(_) => return None,
            AnyExtractor::Claude(inner) => (inner.method(), inner.endpoint()),
            AnyExtractor::Vllm(inner) => (inner.method(), inner.endpoint()),
        };
        let mut hash = blake3::Hasher::new();
        hash.update(b"synveda.capture.provider.v1\0");
        hash.update(&(method.len() as u64).to_be_bytes());
        hash.update(method.as_bytes());
        hash.update(&(endpoint.len() as u64).to_be_bytes());
        hash.update(endpoint.as_bytes());
        Some(*hash.finalize().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_status_has_one_closed_class_and_only_transient_classes_retry() {
        for status in 300_u16..=599 {
            let failure = ProviderFailure::from_status(status);
            let expected_retry = matches!(status, 408 | 429 | 500 | 502 | 503 | 504 | 529);
            assert_eq!(failure.code().retryable(), expected_retry, "{status}");
            assert!(!failure.code().invalidates_source());
        }
        assert_eq!(
            ProviderFailure::from_status(401),
            ProviderFailure::Unauthorized
        );
        assert_eq!(
            ProviderFailure::from_status(403),
            ProviderFailure::Unauthorized
        );
        assert_eq!(ProviderFailure::from_status(307), ProviderFailure::Rejected);
        assert_eq!(ProviderFailure::from_status(501), ProviderFailure::Rejected);
        for failure in [
            ProviderFailure::Timeout,
            ProviderFailure::Unreachable,
            ProviderFailure::Interrupted,
        ] {
            assert!(failure.code().retryable());
        }
        for failure in [
            ProviderFailure::ResponseInvalid,
            ProviderFailure::Misconfigured,
        ] {
            assert!(!failure.code().retryable());
        }
    }

    #[test]
    fn retry_after_is_kept_only_for_rate_limits_and_unavailability() {
        let hint = reqwest::header::HeaderValue::from_static("40");
        let limited = ExtractionError::status("vllm", 429, Some(&hint));
        assert_eq!(limited.retry_after(), Some(Duration::from_secs(40)));
        let unavailable = ExtractionError::status("vllm", 503, Some(&hint));
        assert_eq!(unavailable.retry_after(), Some(Duration::from_secs(40)));
        for status in [500, 502, 401, 400] {
            assert_eq!(
                ExtractionError::status("vllm", status, Some(&hint)).retry_after(),
                None
            );
        }
        let huge = reqwest::header::HeaderValue::from_static("86400000");
        assert_eq!(
            ExtractionError::status("vllm", 429, Some(&huge)).retry_after(),
            Some(crate::retry::MAX_RETRY_AFTER)
        );
    }

    #[test]
    fn provider_keys_separate_endpoints_and_omit_the_credential() {
        let claude = |key: &str, url: &str| {
            AnyExtractor::Claude(
                ClaudeExtractor::new(key.to_owned(), "m".to_owned(), url.to_owned()).unwrap(),
            )
        };
        let first = claude("one", "https://models.example").provider_key();
        assert_eq!(
            first,
            claude("two", "https://models.example/").provider_key()
        );
        assert_ne!(first, claude("one", "https://other.example").provider_key());
        let vllm = AnyExtractor::Vllm(
            VllmExtractor::new("m".to_owned(), "https://models.example".to_owned()).unwrap(),
        );
        assert_ne!(first, vllm.provider_key());
        assert_eq!(
            AnyExtractor::Deterministic(DeterministicExtractor::new()).provider_key(),
            None
        );
    }
}
