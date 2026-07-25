//! The `/v1` client (EVAL-1, ADR-0028 decision 1).
//!
//! Two endpoints, an actor's own bearer, and no other way in. The wire
//! structs are declared here rather than imported because this crate
//! depends on no Synveda crate at all — the same price the TypeScript
//! adapter pays, for the same reason: what an outside caller can see is
//! exactly what an eval should measure.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Generous next to inject's 150ms SLO: a deadline here is meant to end a
/// hung run, not to be the thing under measurement. The latency axis
/// measures what the call took, not what it was allowed to take.
const TIMEOUT: Duration = Duration::from_secs(30);

pub struct Client {
    gateway_url: String,
    http: reqwest::Client,
}

/// `POST /v1/inject` (CTX-3, ADR-0026).
#[derive(Debug, Serialize)]
pub struct InjectRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<&'a str>,
    pub session_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<u32>,
}

/// Only what a measurement needs. The body also carries its own
/// `degraded` list; the header is the one this reads, because the header
/// is what ADR-0026 decision 4 makes the warning.
#[derive(Debug, Deserialize)]
pub struct InjectResponse {
    pub text: String,
    /// The watermark (ADR-0025 decision 7). It rides into the report so a
    /// measurement can be traced back to exactly the block that produced
    /// it, months later, from the audit chain.
    pub block_hash: String,
    pub record_ids: Vec<String>,
    pub tokens: u32,
    pub budget_tokens: u32,
}

/// `POST /v1/observe` (MEM-1, ADR-0020).
#[derive(Debug, Serialize)]
pub struct ObserveRequest<'a> {
    pub session_id: &'a str,
    pub events: Vec<ObserveEvent<'a>>,
}

#[derive(Debug, Serialize)]
pub struct ObserveEvent<'a> {
    pub idempotency_key: String,
    pub kind: &'a str,
    pub payload: serde_json::Value,
    pub occurred_at: String,
}

#[derive(Debug, Deserialize)]
pub struct ObserveResponse {
    pub accepted: usize,
    pub duplicates: usize,
    pub quarantined: usize,
    pub denied: usize,
}

/// A call's result and what it cost, because the cost is a measurement.
pub struct Timed<T> {
    pub value: T,
    pub elapsed_ms: f64,
    /// The degradation ladder's header (ADR-0026 decision 4). A degraded
    /// measurement is still a measurement, and the report says so rather
    /// than quietly averaging it in.
    pub degraded: Vec<String>,
}

impl Client {
    pub fn new(gateway_url: &str) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(TIMEOUT)
            .build()
            .map_err(|err| format!("build the HTTP client: {err}"))?;
        Ok(Self {
            gateway_url: gateway_url.trim_end_matches('/').to_owned(),
            http,
        })
    }

    pub async fn inject(
        &self,
        bearer: &str,
        request: &InjectRequest<'_>,
    ) -> Result<Timed<InjectResponse>, String> {
        self.post("/v1/inject", bearer, request).await
    }

    pub async fn observe(
        &self,
        bearer: &str,
        request: &ObserveRequest<'_>,
    ) -> Result<Timed<ObserveResponse>, String> {
        self.post("/v1/observe", bearer, request).await
    }

    async fn post<B: Serialize, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        bearer: &str,
        body: &B,
    ) -> Result<Timed<T>, String> {
        let started = Instant::now();
        let response = self
            .http
            .post(format!("{}{path}", self.gateway_url))
            .bearer_auth(bearer)
            .header(
                "x-synveda-client",
                concat!("synveda-eval/", env!("CARGO_PKG_VERSION")),
            )
            .json(body)
            .send()
            .await
            .map_err(|err| format!("POST {path}: {err}"))?;
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

        let status = response.status();
        let degraded = response
            .headers()
            .get("x-synveda-degraded")
            .and_then(|value| value.to_str().ok())
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let raw = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!("POST {path} returned {status}: {}", detail(&raw)));
        }
        let value = serde_json::from_str(&raw)
            .map_err(|err| format!("POST {path} returned an unreadable body: {err}"))?;
        Ok(Timed {
            value,
            elapsed_ms,
            degraded,
        })
    }
}

/// The gateway's error taxonomy carries a `message`; anything else is
/// printed as it arrived, truncated.
fn detail(raw: &str) -> String {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| raw.chars().take(200).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_inject_request_omits_what_it_does_not_set() {
        // A `null` task is not the same request as no task: the taskless
        // branch is chosen by absence (ADR-0026 decision 3).
        let request = InjectRequest {
            task: None,
            session_id: "s1",
            budget_tokens: None,
        };
        let json = serde_json::to_string(&request).expect("serialises");
        assert_eq!(json, r#"{"session_id":"s1"}"#);
    }

    #[test]
    fn an_error_body_reads_as_its_message() {
        assert_eq!(
            detail(r#"{"message":"unauthenticated"}"#),
            "unauthenticated"
        );
        assert_eq!(detail("not json at all"), "not json at all");
    }
}
