//! The Claude API judge: the Anthropic Messages API over plain `reqwest`
//! (Rust has no official SDK), with a forced strict tool call so the
//! response is a schema-validated verdict, never prose.
//!
//! `crates/synveda-ingest/src/extraction/claude.rs` is the shape this
//! copies, down to the `model_version`-from-the-response honesty ADR-0061
//! decision 6 requires — the published score is keyed to the model the
//! API *served*, and an alias recorded as if it were a version is a
//! benchmark figure nobody, us included, can reproduce.

use serde::Deserialize;

use super::{Judge, JudgeInput, Verdict, prompt};

/// The method name in the report, the tally and errors.
const SERVICE: &str = "claude-api";

/// The wire-stable API version header the Messages API requires.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Output cap, deliberately far above what a two-field verdict needs.
/// The ceiling bounds thinking *plus* response, so one sized to the
/// verdict alone truncates mid-thought and returns no tool call at all —
/// a paid call that graded nothing. An unused ceiling costs nothing;
/// billing is on the tokens actually produced.
const MAX_TOKENS: u32 = 8192;

/// Grading is intelligence-sensitive and its agreement rate is published
/// (decision 4), so the effort level is stated rather than inherited —
/// a default that moved would move a published number with it. Lower
/// levels are the cost lever a sweep would test; that sweep belongs with
/// the measurement, not with the seam.
const EFFORT: &str = "high";

/// Per-request timeout. A judged run is deliberate and rare (decision 5's
/// consequence), so this is meant to end a hung call rather than to bound
/// the measurement — with thinking on, a slow verdict is a real verdict.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// The forced tool's name.
const TOOL_NAME: &str = "emit_verdict";

/// The Anthropic Messages API judge.
#[derive(Clone)]
pub struct ClaudeJudge {
    client: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
}

/// Hand-written rather than derived, because a derived one would print
/// the key. `AnyJudge` is `Debug`, `AnyJudge` ends up in error paths, and
/// "never logged" is only true if it cannot be.
impl std::fmt::Debug for ClaudeJudge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClaudeJudge")
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("api_key", &"[redacted]")
            .finish()
    }
}

impl ClaudeJudge {
    /// The default model when `SYNVEDA_JUDGE_MODEL` is unset. Its own
    /// default rather than the extractor's: they are different jobs, and
    /// decision 6 keys the baseline to each model separately anyway.
    pub const DEFAULT_MODEL: &'static str = "claude-opus-5";

    /// The default API endpoint when `SYNVEDA_ANTHROPIC_BASE_URL` is
    /// unset. Overridable so tests point at an in-process mock.
    pub const DEFAULT_BASE_URL: &'static str = "https://api.anthropic.com";

    /// Builds the judge. The key is held, sent in the `x-api-key` header,
    /// and never logged.
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

/// The slice of a Messages API response the judge reads.
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

impl Judge for ClaudeJudge {
    fn method(&self) -> &'static str {
        SERVICE
    }

    async fn grade(&self, input: &JudgeInput<'_>) -> Result<Verdict, String> {
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            // Stated rather than left to the default, and never disabled.
            // A verdict here arrives as a forced tool call, and with
            // thinking off the current models sometimes write the call
            // into the visible text instead: the turn succeeds, the tool
            // never runs, and nothing raises. That failure would read as
            // "the judge is unreachable" for a reason no operator could
            // find from the error.
            "thinking": { "type": "adaptive" },
            "output_config": { "effort": EFFORT },
            "system": prompt::SYSTEM_PROMPT,
            "messages": [{ "role": "user", "content": prompt::user_message(input) }],
            "tools": [{
                "name": TOOL_NAME,
                "description": "Emit the verdict for this candidate answer.",
                "strict": true,
                "input_schema": prompt::verdict_schema(),
            }],
            "tool_choice": { "type": "tool", "name": TOOL_NAME },
            // No server-side `fallbacks`, deliberately. A fallback would
            // let a second model answer mid-run, and decision 6 keys the
            // published baseline to the judge model — a score whose judge
            // changed partway is not the score the baseline bounds. A
            // refusal is surfaced below instead, where whoever is running
            // the benchmark can see it and decide.
        });
        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(|err| format!("{SERVICE}: request failed: {err}"))?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            return Err(format!(
                "{SERVICE}: status {status}: {}",
                truncate_detail(&detail)
            ));
        }
        let message: MessagesResponse = response
            .json()
            .await
            .map_err(|err| format!("{SERVICE}: unreadable response: {err}"))?;

        // A refusal is an HTTP 200 with no tool call, so it would
        // otherwise reach the report as "the judge emitted no verdict" —
        // true, unhelpful, and indistinguishable from a malformed
        // response. Named here so a corpus that trips a safety classifier
        // is a finding about the corpus rather than a mystery.
        let stop_reason = message.stop_reason.as_deref().unwrap_or("unknown");
        if stop_reason == "refusal" {
            return Err(format!(
                "{SERVICE}: the model declined to grade this pair (stop_reason: refusal); the \
                 pair is unjudged rather than incorrect"
            ));
        }

        let tool_input = message
            .content
            .into_iter()
            .find(|block| block.kind == "tool_use" && block.name.as_deref() == Some(TOOL_NAME))
            .and_then(|block| block.input)
            .ok_or_else(|| {
                format!(
                    "{SERVICE}: no {TOOL_NAME} tool call in response (stop_reason: {stop_reason})"
                )
            })?;
        let (correct, rationale) = prompt::parse_verdict(SERVICE, tool_input)?;
        Ok(Verdict {
            correct,
            rationale,
            method: SERVICE.to_owned(),
            // The response's model string: honest provenance even when the
            // configured id is an alias (decision 6).
            model_version: message.model,
        })
    }
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
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::extract::State;
    use axum::response::Json;
    use axum::routing::post;
    use axum::{Extension, Router};
    use serde_json::{Value, json};

    use super::*;

    const API_KEY: &str = "test-key-never-real";

    /// The last request body the mock saw, for post-assertions.
    type Captured = Arc<Mutex<Option<(axum::http::HeaderMap, Value)>>>;

    async fn handler(
        State(response): State<Value>,
        Extension(captured): Extension<Captured>,
        headers: axum::http::HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        *captured.lock().expect("capture lock") = Some((headers, body));
        Json(response)
    }

    /// An in-process mock, the `extractor_http.rs` discipline: a test that
    /// reached the real API would be a test that failed when someone
    /// else's network did.
    async fn spawn(response: Value) -> (ClaudeJudge, Captured) {
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
        let judge = ClaudeJudge::new(
            API_KEY.to_owned(),
            ClaudeJudge::DEFAULT_MODEL.to_owned(),
            format!("http://{addr}"),
        );
        (judge, captured)
    }

    fn sample<'a>() -> JudgeInput<'a> {
        JudgeInput {
            question: "when did the lease end",
            reference: "March",
            candidate: "It took about 21 days.",
        }
    }

    #[tokio::test]
    async fn the_request_contract_and_the_served_model_are_both_kept() {
        let (judge, captured) = spawn(json!({
            // Deliberately not the configured id: decision 6 records what
            // the API served, and a test that used the same string for
            // both could not tell the two apart.
            "model": "claude-opus-5-served-build",
            "stop_reason": "tool_use",
            "content": [{
                "type": "tool_use",
                "id": "tu-1",
                "name": "emit_verdict",
                "input": {"correct": true, "rationale": "  21 days is three weeks.  "}
            }]
        }))
        .await;

        let verdict = judge.grade(&sample()).await.expect("graded");
        assert!(verdict.correct);
        assert_eq!(verdict.rationale, "21 days is three weeks.");
        assert_eq!(verdict.method, "claude-api");
        assert_eq!(
            verdict.model_version, "claude-opus-5-served-build",
            "the verdict must carry the model the API served, not the alias requested"
        );

        let (headers, body) = captured
            .lock()
            .expect("capture lock")
            .take()
            .expect("captured");
        assert_eq!(
            headers
                .get("x-api-key")
                .and_then(|value| value.to_str().ok()),
            Some(API_KEY)
        );
        assert_eq!(
            headers
                .get("anthropic-version")
                .and_then(|value| value.to_str().ok()),
            Some(ANTHROPIC_VERSION)
        );
        assert_eq!(body["model"], ClaudeJudge::DEFAULT_MODEL);
        assert_eq!(body["tool_choice"]["type"], "tool");
        assert_eq!(body["tool_choice"]["name"], "emit_verdict");
        assert_eq!(body["tools"][0]["strict"], true);
        assert_eq!(
            body["tools"][0]["input_schema"]["additionalProperties"],
            false
        );
        assert_eq!(body["output_config"]["effort"], EFFORT);
        // The three texts go as data in one user turn, never as separate
        // turns a corpus string could impersonate.
        let sent: Value = serde_json::from_str(
            body["messages"][0]["content"]
                .as_str()
                .expect("text content"),
        )
        .expect("the user message is JSON");
        assert_eq!(sent["reference_answer"], "March");
        assert_eq!(sent["candidate_answer"], "It took about 21 days.");
    }

    /// Thinking stays on. With it disabled the current models sometimes
    /// write the forced tool call into the visible text instead — the
    /// turn succeeds, the call never runs, and the judge silently grades
    /// nothing. Asserted rather than commented, because a future edit
    /// chasing cost would otherwise make it quietly.
    #[tokio::test]
    async fn thinking_is_never_disabled_on_the_forced_tool_call() {
        let (judge, captured) = spawn(json!({
            "model": "m",
            "stop_reason": "tool_use",
            "content": [{"type": "tool_use", "name": "emit_verdict",
                         "input": {"correct": false, "rationale": "no"}}]
        }))
        .await;
        judge.grade(&sample()).await.expect("graded");
        let (_, body) = captured
            .lock()
            .expect("capture lock")
            .take()
            .expect("captured");
        assert_eq!(body["thinking"]["type"], "adaptive");
    }

    /// A refusal is an HTTP 200 with no tool call. Unnamed, it reads as a
    /// malformed response; named, it says the pair went ungraded rather
    /// than graded wrong — which is the difference between a corpus
    /// finding and a moved score.
    #[tokio::test]
    async fn a_refusal_says_the_pair_is_unjudged_rather_than_incorrect() {
        let (judge, _) = spawn(json!({
            "model": "m",
            "stop_reason": "refusal",
            "stop_details": {"type": "refusal", "category": "cyber"},
            "content": []
        }))
        .await;
        let err = judge
            .grade(&sample())
            .await
            .expect_err("a refusal is not a verdict");
        assert!(err.contains("declined to grade"), "unhelpful error: {err}");
        assert!(
            err.contains("rather than incorrect"),
            "unhelpful error: {err}"
        );
    }

    #[tokio::test]
    async fn a_response_without_the_forced_call_names_the_stop_reason() {
        let (judge, _) = spawn(json!({
            "model": "m",
            "stop_reason": "max_tokens",
            "content": [{"type": "text", "text": "I think the answer is..."}]
        }))
        .await;
        let err = judge
            .grade(&sample())
            .await
            .expect_err("prose is not a verdict");
        assert!(err.contains("max_tokens"), "unhelpful error: {err}");
    }
}
