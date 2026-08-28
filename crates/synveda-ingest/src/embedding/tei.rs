//! The TEI embedder: `POST /embed` against a text-embeddings-inference
//! base URL (tech plan §1.3 — BGE-M3 in the dev compose). Plain HTTP is
//! fine here: the endpoint is self-hosted by definition. The model
//! identity is config-declared, never probed from `/info` — gateway
//! boot must not couple to TEI availability (ADR-0023 decision 6).

use synveda_types::{Error, Result};

use super::Embedder;

/// The dependency name in errors and metrics labels.
const SERVICE: &str = "tei";

/// Per-request timeout, under the worker's default 60 s visibility
/// timeout so a slow call fails here, not as a redelivery surprise.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The text-embeddings-inference embedder.
#[derive(Debug, Clone)]
pub struct TeiEmbedder {
    client: reqwest::Client,
    model: String,
    base_url: String,
}

impl TeiEmbedder {
    /// The default model identity when `SYNVEDA_EMBEDDER_MODEL` is
    /// unset — what the dev compose serves.
    pub const DEFAULT_MODEL: &'static str = "BAAI/bge-m3";

    /// Builds the embedder against a TEI base URL
    /// (e.g. `http://localhost:8110`, the dev compose port).
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
}

impl Embedder for TeiEmbedder {
    fn method(&self) -> &'static str {
        SERVICE
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let response = self
            .client
            .post(format!("{}/embed", self.base_url))
            .json(&serde_json::json!({ "inputs": inputs }))
            .send()
            .await
            .map_err(|error| dependency(transport_code(&error).to_owned()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(dependency(format!("upstream_http_{}", status.as_u16())));
        }
        let vectors: Vec<Vec<f32>> = response
            .json()
            .await
            .map_err(|_| dependency("response_invalid".to_owned()))?;
        // One vector per input, in order, or the whole call failed —
        // a deviation here silently misattributes vectors to contents.
        if vectors.len() != inputs.len() {
            return Err(dependency(format!(
                "expected {} vectors, got {}",
                inputs.len(),
                vectors.len()
            )));
        }
        if let Some(empty) = vectors.iter().position(Vec::is_empty) {
            return Err(dependency(format!("empty vector at index {empty}")));
        }
        Ok(vectors)
    }
}

fn transport_code(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "request_timeout"
    } else if error.is_connect() {
        "request_connect_failed"
    } else {
        "request_failed"
    }
}

fn dependency(message: String) -> Error {
    Error::Dependency {
        service: SERVICE.to_owned(),
        message,
    }
}
