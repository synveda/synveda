//! The Anthropic Messages API transport both model-backed seams share
//! (EVAL-3, ADR-0061): one client, one forced-tool-call cycle, one
//! failure taxonomy.
//!
//! It exists for `extraction::prompt`'s reason, one level down. That
//! module keeps two extractors extracting the same things; this keeps the
//! reader and the judge *reaching the API* the same way — same version
//! header, same refusal handling, same "the model the API served" rule
//! that decision 6 turns into baseline provenance. Two copies of this
//! cycle would drift, and the half that drifted would be whichever one
//! nobody was looking at when a model changed.
//!
//! Rust has no official SDK, so this is plain `reqwest` over the
//! documented wire shape.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The wire-stable API version header the Messages API requires.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Per-request timeout. A model-backed eval run is deliberate and rare
/// (ADR-0061 decision 5's consequence), so this is meant to end a hung
/// call rather than to bound the measurement — with thinking on, a slow
/// answer is a real answer.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// The default endpoint when `SYNVEDA_ANTHROPIC_BASE_URL` is unset.
/// Overridable so tests point at an in-process mock.
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// The default model when a seam's own `_MODEL` variable is unset.
pub const DEFAULT_MODEL: &str = "claude-opus-5";

/// One forced-tool-call request. Everything that differs between the
/// reader and the judge is a field here; everything that must not differ
/// is in [`Client::call`].
pub struct ToolCall {
    /// The seam's method name, for error messages.
    pub service: &'static str,
    pub tool_name: &'static str,
    pub tool_description: &'static str,
    pub schema: serde_json::Value,
    pub system: &'static str,
    pub user: String,
    /// Deliberately far above what the payload needs: the ceiling bounds
    /// thinking *plus* response, so one sized to the answer alone
    /// truncates mid-thought and returns no tool call at all — a paid
    /// call that produced nothing. An unused ceiling costs nothing.
    pub max_tokens: u32,
    /// Stated by each seam rather than inherited, because a default that
    /// moved would move a published number with it.
    pub effort: &'static str,
}

/// What one call produced.
#[derive(Debug)]
pub struct ToolResult {
    /// The forced tool call's input, for the caller to parse.
    pub input: serde_json::Value,
    /// The model the API *served*, never the alias requested — decision
    /// 6's rule, and the reason a benchmark figure is reproducible.
    pub model_version: String,
    /// What the call cost, in tokens.
    pub usage: Usage,
}

/// One call's token usage, as the API reports it.
///
/// Recorded because these are the two paths in this repo that bill per
/// item, and a run whose cost nobody counted is one nobody can plan. The
/// concrete question it answers: decision 7 splits a declared slice from
/// the full 500 instances, and choosing that slice without knowing what
/// an instance costs is guessing.
///
/// **Tokens, not money, and deliberately.** A cost in dollars is a fact
/// about a price list that changes; a token count is a fact about the
/// run. A hardcoded price table would go stale silently inside a
/// published artefact, which is the failure mode ADR-0061 spent its first
/// decision on. Multiply at the point of asking.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
pub struct Usage {
    /// The prompt tokens billed at full rate. **Not** the whole prompt —
    /// the cached halves are the two fields below, and a total that used
    /// this alone would under-report every cached call.
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
}

impl Usage {
    /// Accumulates another call's usage into this one.
    pub fn add(&mut self, other: Usage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_creation_input_tokens += other.cache_creation_input_tokens;
        self.cache_read_input_tokens += other.cache_read_input_tokens;
    }

    /// Every prompt token the call carried, cached or not.
    #[must_use]
    pub fn prompt_tokens(&self) -> u64 {
        self.input_tokens + self.cache_creation_input_tokens + self.cache_read_input_tokens
    }
}

/// A configured endpoint and model.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
}

/// Hand-written rather than derived, because a derived one would print
/// the key. These clients end up inside seams that are `Debug`, and
/// "never logged" is only true if it cannot be.
impl std::fmt::Debug for Client {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("anthropic::Client")
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("api_key", &"[redacted]")
            .finish()
    }
}

impl Client {
    /// Builds the client. The key is held, sent in the `x-api-key`
    /// header, and never printed.
    #[must_use]
    pub fn new(api_key: String, model: String, base_url: String) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .unwrap_or_default(),
            api_key,
            model,
            base_url: base_url.trim_end_matches('/').to_owned(),
        }
    }

    /// The configured model id, which is an alias as often as not — the
    /// served version comes back on [`ToolResult`].
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// One forced tool call, start to finish.
    pub async fn call(&self, call: &ToolCall) -> Result<ToolResult, String> {
        let service = call.service;
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": call.max_tokens,
            // Stated rather than left to the default, and never disabled.
            // Every payload here arrives as a forced tool call, and with
            // thinking off the current models sometimes write the call
            // into the visible text instead: the turn succeeds, the tool
            // never runs, and nothing raises. That failure would read as
            // "the model is unreachable" for a reason no operator could
            // find from the error.
            "thinking": { "type": "adaptive" },
            "output_config": { "effort": call.effort },
            "system": call.system,
            "messages": [{ "role": "user", "content": call.user }],
            "tools": [{
                "name": call.tool_name,
                "description": call.tool_description,
                "strict": true,
                "input_schema": call.schema,
            }],
            "tool_choice": { "type": "tool", "name": call.tool_name },
            // No server-side `fallbacks`, deliberately. A fallback would
            // let a second model answer mid-run, and decision 6 keys the
            // published baseline to the reader and judge models — a score
            // whose model changed partway is not the score the baseline
            // bounds. A refusal is surfaced below instead, where whoever
            // is running the benchmark can see it and decide.
        });

        let response = self
            .http
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(|err| format!("{service}: request failed: {err}"))?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            return Err(format!(
                "{service}: status {status}: {}",
                truncate_detail(&detail)
            ));
        }
        let message: MessagesResponse = response
            .json()
            .await
            .map_err(|err| format!("{service}: unreadable response: {err}"))?;

        // A refusal is an HTTP 200 with no tool call, so it would
        // otherwise reach the report as "the model emitted nothing" —
        // true, unhelpful, and indistinguishable from a malformed
        // response. Named here so a corpus that trips a safety classifier
        // is a finding about the corpus rather than a mystery.
        // Read before the early returns below: a declined or malformed
        // call is still a billed call on the pre-output half, and a
        // budget that only counted successes would under-report exactly
        // the runs that went wrong.
        let usage = message.usage;
        let stop_reason = message.stop_reason.as_deref().unwrap_or("unknown");
        if stop_reason == "refusal" {
            return Err(format!(
                "{service}: the model declined this call (stop_reason: refusal); the item is \
                 unmeasured rather than wrong"
            ));
        }

        let tool_name = call.tool_name;
        let input = message
            .content
            .into_iter()
            .find(|block| block.kind == "tool_use" && block.name.as_deref() == Some(tool_name))
            .and_then(|block| block.input)
            .ok_or_else(|| {
                format!(
                    "{service}: no {tool_name} tool call in response (stop_reason: {stop_reason})"
                )
            })?;
        Ok(ToolResult {
            input,
            model_version: message.model,
            usage,
        })
    }
}

/// The slice of a Messages API response these seams read.
#[derive(Deserialize)]
struct MessagesResponse {
    model: String,
    stop_reason: Option<String>,
    content: Vec<ContentBlock>,
    /// Defaulted so a mock that omits it still parses; a real response
    /// always carries one.
    #[serde(default)]
    usage: Usage,
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

/// Error bodies ride into report rows and operator eyes — keep them short
/// and content-light.
fn truncate_detail(detail: &str) -> &str {
    let cut = detail
        .char_indices()
        .nth(200)
        .map_or(detail.len(), |(index, _)| index);
    &detail[..cut]
}

#[cfg(test)]
pub(crate) mod mock {
    //! An in-process Messages API, the `extractor_http.rs` discipline: a
    //! test that reached the real API would be a test that failed when
    //! someone else's network did.

    use std::sync::{Arc, Mutex};

    use axum::extract::State;
    use axum::response::Json;
    use axum::routing::post;
    use axum::{Extension, Router};
    use serde_json::Value;

    pub const API_KEY: &str = "test-key-never-real";

    /// The last request body the mock saw, for post-assertions.
    pub type Captured = Arc<Mutex<Option<(axum::http::HeaderMap, Value)>>>;

    async fn handler(
        State(response): State<Value>,
        Extension(captured): Extension<Captured>,
        headers: axum::http::HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        *captured.lock().expect("capture lock") = Some((headers, body));
        Json(response)
    }

    /// Spawns a mock that answers every call with `response`, and returns
    /// its base url plus the capture slot.
    pub async fn spawn(response: Value) -> (String, Captured) {
        let captured: Captured = Arc::new(Mutex::new(None));
        let router = Router::new()
            .route("/v1/messages", post(handler))
            .layer(Extension(Arc::clone(&captured)))
            .with_state(response);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock");
        let addr = listener.local_addr().expect("mock addr");
        tokio::spawn(async move {
            axum::serve(listener, router).await.expect("mock serve");
        });
        (format!("http://{addr}"), captured)
    }

    /// The body the mock last received.
    pub fn body(captured: &Captured) -> Value {
        captured
            .lock()
            .expect("capture lock")
            .clone()
            .expect("captured")
            .1
    }

    /// The headers the mock last received.
    pub fn headers(captured: &Captured) -> axum::http::HeaderMap {
        captured
            .lock()
            .expect("capture lock")
            .clone()
            .expect("captured")
            .0
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn call() -> ToolCall {
        ToolCall {
            service: "test-seam",
            tool_name: "emit_thing",
            tool_description: "Emit the thing.",
            schema: json!({"type": "object", "additionalProperties": false}),
            system: "You do the thing.",
            user: "the thing".to_owned(),
            max_tokens: 8192,
            effort: "high",
        }
    }

    async fn client(response: serde_json::Value) -> (Client, mock::Captured) {
        let (base_url, captured) = mock::spawn(response).await;
        (
            Client::new(
                mock::API_KEY.to_owned(),
                DEFAULT_MODEL.to_owned(),
                format!("{base_url}/"),
            ),
            captured,
        )
    }

    #[tokio::test]
    async fn the_request_contract_and_the_served_model_are_both_kept() {
        let (client, captured) = client(json!({
            // Deliberately not the configured id: decision 6 records what
            // the API served, and a test using the same string for both
            // could not tell them apart.
            "model": "claude-opus-5-served-build",
            "stop_reason": "tool_use",
            "content": [{"type": "tool_use", "name": "emit_thing", "input": {"ok": true}}]
        }))
        .await;

        let result = client.call(&call()).await.expect("called");
        assert_eq!(result.model_version, "claude-opus-5-served-build");
        assert_eq!(result.input["ok"], true);

        let headers = mock::headers(&captured);
        assert_eq!(
            headers.get("x-api-key").and_then(|v| v.to_str().ok()),
            Some(mock::API_KEY)
        );
        assert_eq!(
            headers
                .get("anthropic-version")
                .and_then(|v| v.to_str().ok()),
            Some(ANTHROPIC_VERSION)
        );
        let body = mock::body(&captured);
        assert_eq!(body["model"], DEFAULT_MODEL);
        assert_eq!(body["tool_choice"]["type"], "tool");
        assert_eq!(body["tool_choice"]["name"], "emit_thing");
        assert_eq!(body["tools"][0]["strict"], true);
        assert_eq!(body["output_config"]["effort"], "high");
        assert_eq!(body["max_tokens"], 8192);
    }

    /// Thinking stays on. With it disabled the current models sometimes
    /// write the forced tool call into visible text — the turn succeeds,
    /// the call never runs, and the seam silently produces nothing.
    /// Asserted rather than commented, because a future edit chasing cost
    /// would otherwise make it quietly.
    #[tokio::test]
    async fn thinking_is_never_disabled_on_the_forced_tool_call() {
        let (client, captured) = client(json!({
            "model": "m", "stop_reason": "tool_use",
            "content": [{"type": "tool_use", "name": "emit_thing", "input": {}}]
        }))
        .await;
        client.call(&call()).await.expect("called");
        assert_eq!(mock::body(&captured)["thinking"]["type"], "adaptive");
    }

    #[tokio::test]
    async fn a_refusal_says_the_item_is_unmeasured_rather_than_wrong() {
        let (client, _) = client(json!({
            "model": "m",
            "stop_reason": "refusal",
            "stop_details": {"type": "refusal", "category": "cyber"},
            "content": []
        }))
        .await;
        let err = client
            .call(&call())
            .await
            .expect_err("a refusal is not a result");
        assert!(err.contains("declined this call"), "unhelpful error: {err}");
        assert!(err.contains("rather than wrong"), "unhelpful error: {err}");
    }

    #[tokio::test]
    async fn a_response_without_the_forced_call_names_the_stop_reason() {
        let (client, _) = client(json!({
            "model": "m",
            "stop_reason": "max_tokens",
            "content": [{"type": "text", "text": "I think the answer is..."}]
        }))
        .await;
        let err = client
            .call(&call())
            .await
            .expect_err("prose is not a result");
        assert!(err.contains("max_tokens"), "unhelpful error: {err}");
        assert!(err.contains("test-seam"), "the error names the seam: {err}");
    }

    #[tokio::test]
    async fn an_error_status_is_a_named_failure_with_a_bounded_body() {
        let (base_url, _) = mock::spawn(json!({})).await;
        let client = Client::new(
            mock::API_KEY.to_owned(),
            DEFAULT_MODEL.to_owned(),
            // Nothing serves this path, so the mock 404s.
            format!("{base_url}/nowhere"),
        );
        let err = client
            .call(&call())
            .await
            .expect_err("a 404 is not a result");
        assert!(err.contains("status 404"), "unhelpful error: {err}");
    }

    /// The accounting these two paths bill against. `input_tokens` is the
    /// uncached remainder, not the whole prompt, and a total built from it
    /// alone would under-report every cached call — so the total is
    /// asserted rather than assumed.
    #[tokio::test]
    async fn usage_is_read_from_the_response_and_totals_the_whole_prompt() {
        let (client, _) = client(json!({
            "model": "m",
            "stop_reason": "tool_use",
            "content": [{"type": "tool_use", "name": "emit_thing", "input": {}}],
            "usage": {
                "input_tokens": 1200,
                "output_tokens": 340,
                "cache_read_input_tokens": 800,
                "cache_creation_input_tokens": 64
            }
        }))
        .await;
        let result = client.call(&call()).await.expect("called");
        assert_eq!(result.usage.output_tokens, 340);
        assert_eq!(result.usage.prompt_tokens(), 2064);

        let mut running = Usage::default();
        running.add(result.usage);
        running.add(result.usage);
        assert_eq!(running.prompt_tokens(), 4128);
        assert_eq!(running.output_tokens, 680);
    }

    /// A response without a `usage` block still parses. Mocks omit it,
    /// and a harness that panicked on one would be a harness whose tests
    /// could not run offline.
    #[tokio::test]
    async fn a_response_without_usage_reports_zero_rather_than_failing() {
        let (client, _) = client(json!({
            "model": "m",
            "stop_reason": "tool_use",
            "content": [{"type": "tool_use", "name": "emit_thing", "input": {}}]
        }))
        .await;
        let result = client.call(&call()).await.expect("called");
        assert_eq!(result.usage.prompt_tokens(), 0);
    }

    #[test]
    fn a_debug_print_never_carries_the_key() {
        let client = Client::new(
            "sk-ant-secret".to_owned(),
            DEFAULT_MODEL.to_owned(),
            "https://example.test/".to_owned(),
        );
        let shown = format!("{client:?}");
        assert!(!shown.contains("sk-ant-secret"), "{shown}");
        assert!(shown.contains("[redacted]"), "{shown}");
        // The trailing slash is trimmed, so the request path cannot end up
        // with a double slash the endpoint would 404.
        assert!(shown.contains("https://example.test\""), "{shown}");
    }
}
