//! The TEI embedder: `POST /embed` against a text-embeddings-inference
//! base URL (tech plan §1.3 — BGE-M3 in the dev compose). Plain HTTP is
//! fine here: the endpoint is self-hosted by definition. The model
//! identity is config-declared, never probed from `/info` — gateway
//! boot must not couple to TEI availability (ADR-0023 decision 6).

use serde::Deserialize;
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
    #[must_use]
    pub fn new(model: String, base_url: String) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .unwrap_or_default(),
            model,
            base_url: base_url.trim_end_matches('/').to_owned(),
        }
    }
}

/// TEI's error body, when it sends one.
#[derive(Deserialize)]
struct TeiError {
    error: String,
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
            .map_err(|err| dependency(format!("request failed: {err}")))?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            let detail = serde_json::from_str::<TeiError>(&detail)
                .map(|body| body.error)
                .unwrap_or(detail);
            let cut = detail
                .char_indices()
                .nth(200)
                .map_or(detail.len(), |(index, _)| index);
            return Err(dependency(format!("status {status}: {}", &detail[..cut])));
        }
        let vectors: Vec<Vec<f32>> = response
            .json()
            .await
            .map_err(|err| dependency(format!("unreadable response: {err}")))?;
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

fn dependency(message: String) -> Error {
    Error::Dependency {
        service: SERVICE.to_owned(),
        message,
    }
}
