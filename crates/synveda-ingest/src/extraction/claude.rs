//! The Claude API extractor: the Anthropic Messages API over plain
//! `reqwest` (Rust has no official SDK), with a forced strict tool call
//! so the response is schema-validated candidates, never prose (ADR-0022
//! decision 3).

use serde::Deserialize;
use synveda_types::{Error, Result};

use super::{ExtractionInput, ExtractionOutcome, Extractor, prompt};

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
#[derive(Debug, Clone)]
pub struct ClaudeExtractor {
    client: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
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
    #[must_use]
    pub fn new(api_key: String, model: String, base_url: String) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .unwrap_or_default(),
            api_key,
            model,
            base_url: base_url.trim_end_matches('/').to_owned(),
        }
    }
}

/// The slice of a Messages API response the extractor reads.
#[derive(Deserialize)]
struct MessagesResponse {
    model: String,
    stop_reason: Option<String>,
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

    async fn extract(&self, input: &ExtractionInput) -> Result<ExtractionOutcome> {
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
            .map_err(|err| dependency(format!("request failed: {err}")))?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            return Err(dependency(format!(
                "status {status}: {}",
                truncate_detail(&detail)
            )));
        }
        let message: MessagesResponse = response
            .json()
            .await
            .map_err(|err| dependency(format!("unreadable response: {err}")))?;
        let tool_input = message
            .content
            .into_iter()
            .find(|block| block.kind == "tool_use" && block.name.as_deref() == Some(TOOL_NAME))
            .and_then(|block| block.input)
            .ok_or_else(|| {
                dependency(format!(
                    "no {TOOL_NAME} tool call in response (stop_reason: {})",
                    message.stop_reason.as_deref().unwrap_or("unknown")
                ))
            })?;
        Ok(ExtractionOutcome {
            candidates: prompt::parse_candidates(SERVICE, tool_input)?,
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

/// Error bodies ride into `Error::Dependency` messages, which reach logs
/// and (as denial reasons never, but as messages sometimes) operators —
/// keep them short and content-light.
fn truncate_detail(detail: &str) -> &str {
    let cut = detail
        .char_indices()
        .nth(200)
        .map_or(detail.len(), |(index, _)| index);
    &detail[..cut]
}
