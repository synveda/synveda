//! The OpenAI-compatible extractor: `/v1/chat/completions` against a
//! configured base URL — vLLM is the air-gapped deployment the tech plan
//! names (§1.3), but any endpoint speaking the same wire shape works.
//! Plain HTTP is fine here: the endpoint is self-hosted by definition.

use serde::Deserialize;
use synveda_types::{Error, Result};

use super::{
    ExtractionError, ExtractionInput, ExtractionOutcome, Extractor, ProviderFailure, prompt,
};

/// The dependency name in errors and metrics labels.
const SERVICE: &str = "vllm";

/// Output cap; mirrors the Claude impl.
const MAX_TOKENS: u32 = 2048;

/// Per-request timeout, under the worker's default visibility timeout.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

/// The OpenAI-compatible chat-completions extractor.
#[derive(Debug, Clone)]
pub struct VllmExtractor {
    client: reqwest::Client,
    model: String,
    base_url: String,
}

impl VllmExtractor {
    /// Builds the extractor against an OpenAI-compatible base URL
    /// (e.g. `http://vllm.internal:8000`).
    pub fn new(model: String, base_url: String) -> Result<Self> {
        let base_url = crate::provider_url::normalise(&base_url)
            .ok_or_else(|| dependency("client_configuration_failed".to_owned()))?;
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| dependency("client_configuration_failed".to_owned()))?;
        Ok(Self {
            client,
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

/// The slice of a chat-completions response the extractor reads.
#[derive(Deserialize)]
struct ChatResponse {
    #[serde(default)]
    model: Option<String>,
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    #[serde(default)]
    content: Option<String>,
}

impl Extractor for VllmExtractor {
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
            "temperature": 0,
            "response_format": { "type": "json_object" },
            "messages": [
                { "role": "system", "content": prompt::SYSTEM_PROMPT },
                { "role": "user", "content": prompt::user_message(input) },
            ],
        });
        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
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
        let completion: ChatResponse = response.json().await.map_err(|error| {
            if error.is_decode() {
                failed(ProviderFailure::ResponseInvalid)
            } else {
                failed(ProviderFailure::from_transport(&error))
            }
        })?;
        let text = completion
            .choices
            .first()
            .and_then(|choice| choice.message.content.as_deref())
            .ok_or_else(|| failed(ProviderFailure::ResponseInvalid))?;
        let value: serde_json::Value = serde_json::from_str(prompt::strip_fence(text))
            .map_err(|_| failed(ProviderFailure::ResponseInvalid))?;
        Ok(ExtractionOutcome {
            candidates: prompt::parse_candidates(SERVICE, value, input.event_type)
                .map_err(|_| failed(ProviderFailure::ResponseInvalid))?,
            method: SERVICE.to_owned(),
            model_version: completion.model.unwrap_or_else(|| self.model.clone()),
        })
    }
}

const fn failed(failure: ProviderFailure) -> ExtractionError {
    ExtractionError::new(SERVICE, failure)
}

fn dependency(message: String) -> Error {
    Error::Dependency {
        service: SERVICE.to_owned(),
        message,
    }
}
