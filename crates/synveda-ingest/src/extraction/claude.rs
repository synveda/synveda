//! The Claude API extractor: the Anthropic Messages API over plain
//! `reqwest` (Rust has no official SDK), with a forced strict tool call
//! so the response is schema-validated candidates, never prose (ADR-0022
//! decision 3).

use serde::Deserialize;
use synveda_types::{Error, Result};

use super::{
    ExtractionError, ExtractionInput, ExtractionOutcome, Extractor, ProviderFailure, prompt,
};

/// The dependency name in errors and metrics labels.
const SERVICE: &str = "claude-api";

/// The wire-stable API version header the Messages API requires.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Output cap: candidates JSON for a 64 KiB payload fits comfortably.
const MAX_TOKENS: u32 = 2048;

/// Per-request timeout, under the worker's default 60s visibility
/// timeout so a slow call fails here, not as a redelivery surprise.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

/// The forced tool's name.
const TOOL_NAME: &str = "emit_extraction";

/// The Anthropic Messages API extractor.
#[derive(Clone)]
pub struct ClaudeExtractor {
    client: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl std::fmt::Debug for ClaudeExtractor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClaudeExtractor")
            .field("api_key", &"[REDACTED]")
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl ClaudeExtractor {
    /// The default model when `SYNVEDA_EXTRACTOR_MODEL` is unset.
    pub const DEFAULT_MODEL: &'static str = "claude-opus-4-8";

    /// The default API endpoint when `SYNVEDA_ANTHROPIC_BASE_URL` is
    /// unset. Overridable so tests and demos point at a local mock.
    pub const DEFAULT_BASE_URL: &'static str = "https://api.anthropic.com";

    /// Builds the extractor. The key is held, sent in the `x-api-key`
    /// header, and never logged (the `SYNVEDA_DEV_JWT_SECRET`
    /// discipline).
    pub fn new(api_key: String, model: String, base_url: String) -> Result<Self> {
        let base_url = crate::provider_url::normalise(&base_url)
            .ok_or_else(|| dependency("client_configuration_failed".to_owned()))?;
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            // Never forward `x-api-key` to an operator-controlled redirect.
            // Provider endpoint changes are configuration, not wire behavior.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| dependency("client_configuration_failed".to_owned()))?;
        Ok(Self {
            client,
            api_key,
            model,
            base_url,
        })
    }

    /// The normalised endpoint that identifies this provider's capacity.
    pub(crate) fn endpoint(&self) -> &str {
        &self.base_url
    }

    /// Replaces the per-request deadline, which must stay below the Capture
    /// lease so a stalled call fails as a retryable timeout.
    pub fn with_request_timeout(mut self, timeout: std::time::Duration) -> Result<Self> {
        self.client = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| dependency("client_configuration_failed".to_owned()))?;
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_discloses_the_api_key() {
        let secret = "SYNVEDA_CLAUDE_DEBUG_SECRET";
        let extractor = ClaudeExtractor::new(
            secret.to_owned(),
            ClaudeExtractor::DEFAULT_MODEL.to_owned(),
            ClaudeExtractor::DEFAULT_BASE_URL.to_owned(),
        )
        .expect("configure extractor");
        let rendered = format!("{extractor:?}");
        assert!(!rendered.contains(secret));
        assert!(rendered.contains("[REDACTED]"));
    }
}

/// The slice of a Messages API response the extractor reads.
#[derive(Deserialize)]
struct MessagesResponse {
    model: String,
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    input: Option<serde_json::Value>,
}

impl Extractor for ClaudeExtractor {
    fn method(&self) -> &'static str {
        SERVICE
    }

    async fn extract(
        &self,
        input: &ExtractionInput,
    ) -> std::result::Result<ExtractionOutcome, ExtractionError> {
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "system": prompt::SYSTEM_PROMPT,
            "messages": [{ "role": "user", "content": prompt::user_message(input) }],
            "tools": [{
                "name": TOOL_NAME,
                "description": "Emit the extracted memory candidates for this event.",
                "strict": true,
                "input_schema": prompt::candidates_schema(),
            }],
            "tool_choice": { "type": "tool", "name": TOOL_NAME },
        });
        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(|error| failed(ProviderFailure::from_transport(&error)))?;
        let status = response.status();
        if !status.is_success() {
            return Err(ExtractionError::status(
                SERVICE,
                status.as_u16(),
                response.headers().get(reqwest::header::RETRY_AFTER),
            ));
        }
        let message: MessagesResponse = response.json().await.map_err(|error| {
            if error.is_decode() {
                failed(ProviderFailure::ResponseInvalid)
            } else {
                failed(ProviderFailure::from_transport(&error))
            }
        })?;
        let tool_input = message
            .content
            .into_iter()
            .find(|block| block.kind == "tool_use" && block.name.as_deref() == Some(TOOL_NAME))
            .and_then(|block| block.input)
            .ok_or_else(|| failed(ProviderFailure::ResponseInvalid))?;
        Ok(ExtractionOutcome {
            candidates: prompt::parse_candidates(SERVICE, tool_input, input.event_type)
                .map_err(|_| failed(ProviderFailure::ResponseInvalid))?,
            method: SERVICE.to_owned(),
            // The response's model string: honest provenance even when
            // the configured id is an alias.
            model_version: message.model,
        })
    }
}

fn dependency(message: String) -> Error {
    Error::Dependency {
        service: SERVICE.to_owned(),
        message,
    }
}

const fn failed(failure: ProviderFailure) -> ExtractionError {
    ExtractionError::new(SERVICE, failure)
}
