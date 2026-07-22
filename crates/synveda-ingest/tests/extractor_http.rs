//! The LLM extractor implementations against in-process mock servers
//! (the MockIdp discipline: never a network dependency in tests). Covers
//! the request contract both impls promise — forced strict tool call for
//! Claude, JSON response format for vLLM, the opacity instruction in the
//! shared prompt — and the failure taxonomy: malformed output and error
//! statuses are `Error::Dependency`, which the worker maps to a
//! visibility-timeout retry (ADR-0022 decisions 3 and 6).

use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::response::Json;
use axum::routing::post;
use axum::{Extension, Router};
use chrono::Utc;
use serde_json::{Value, json};
use synveda_ingest::extraction::{ClaudeExtractor, ExtractionInput, Extractor, VllmExtractor};
use synveda_types::{
    Error, IdentityId, ObserveEventId, ObserveKind, RecordClass, ScopeId, Sensitivity, TenantId,
};

const API_KEY: &str = "test-key-never-real";

/// The last request body the mock saw, for post-assertions.
type Captured = Arc<Mutex<Option<(axum::http::HeaderMap, Value)>>>;

async fn spawn_mock(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock");
    let addr = listener.local_addr().expect("mock addr");
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("mock serve");
    });
    format!("http://{addr}")
}

fn sample_input() -> ExtractionInput {
    ExtractionInput {
        event_id: ObserveEventId::new(),
        tenant_id: TenantId::new(),
        scope_id: ScopeId::new(),
        owner_id: IdentityId::new(),
        session_id: "sess-http".to_owned(),
        kind: ObserveKind::TranscriptDelta,
        payload: json!({"text": "We decided the value [REDACTED:github-pat] stays redacted."}),
        occurred_at: Utc::now(),
        redactions: None,
    }
}

async fn claude_mock(
    State(response): State<Value>,
    Extension(captured): Extension<Captured>,
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    *captured.lock().expect("capture lock") = Some((headers, body));
    Json(response)
}

async fn spawn_claude_mock(response: Value) -> (String, Captured) {
    let captured: Captured = Arc::new(Mutex::new(None));
    let router = Router::new()
        .route("/v1/messages", post(claude_mock))
        .layer(Extension(Arc::clone(&captured)))
        .with_state(response);
    (spawn_mock(router).await, captured)
}

/// The Claude impl sends the documented request shape — key header, API
/// version, forced strict tool choice, the opacity rule in the system
/// prompt — and parses the forced tool call into candidates with the
/// response's model string as provenance.
#[tokio::test]
async fn claude_request_and_response_contract() {
    let (base_url, captured) = spawn_claude_mock(json!({
        "model": "claude-opus-4-8",
        "stop_reason": "tool_use",
        "content": [{
            "type": "tool_use",
            "id": "tu-1",
            "name": "emit_extraction",
            "input": {
                "candidates": [{
                    "class": "decision",
                    "content": "The token [REDACTED:github-pat] stays redacted.",
                    "confidence": 1.4,
                    "sensitivity": "confidential",
                    "entities": ["github"]
                }]
            }
        }]
    }))
    .await;
    let extractor =
        ClaudeExtractor::new(API_KEY.to_owned(), "claude-opus-4-8".to_owned(), base_url);
    let outcome = extractor.extract(&sample_input()).await.expect("extract");

    let (headers, body) = captured
        .lock()
        .expect("capture lock")
        .take()
        .expect("captured");
    assert_eq!(
        headers.get("x-api-key").and_then(|v| v.to_str().ok()),
        Some(API_KEY)
    );
    assert_eq!(
        headers
            .get("anthropic-version")
            .and_then(|v| v.to_str().ok()),
        Some("2023-06-01")
    );
    assert_eq!(body["model"], "claude-opus-4-8");
    assert_eq!(body["tool_choice"]["type"], "tool");
    assert_eq!(body["tool_choice"]["name"], "emit_extraction");
    assert_eq!(body["tools"][0]["strict"], true);
    assert_eq!(
        body["tools"][0]["input_schema"]["additionalProperties"],
        false
    );
    let system = body["system"].as_str().expect("system prompt");
    assert!(
        system.contains("[REDACTED:rule-id]"),
        "the opacity rule must ride the prompt"
    );

    assert_eq!(outcome.method, "claude-api");
    assert_eq!(outcome.model_version, "claude-opus-4-8");
    assert_eq!(outcome.candidates.len(), 1);
    let candidate = &outcome.candidates[0];
    assert_eq!(candidate.class, RecordClass::Decision);
    assert!(candidate.content.contains("[REDACTED:github-pat]"));
    assert_eq!(candidate.confidence, 1.0, "confidence clamps into [0,1]");
    assert_eq!(candidate.sensitivity, Some(Sensitivity::Confidential));
    assert_eq!(candidate.entities, vec!["github".to_owned()]);
}

/// Output outside the candidates contract is a `Dependency` error — the
/// worker's leave-and-retry signal, never a partial persist.
#[tokio::test]
async fn claude_malformed_output_is_a_dependency_error() {
    let (base_url, _) = spawn_claude_mock(json!({
        "model": "claude-opus-4-8",
        "stop_reason": "tool_use",
        "content": [{
            "type": "tool_use",
            "id": "tu-1",
            "name": "emit_extraction",
            "input": { "candidates": "not-a-list" }
        }]
    }))
    .await;
    let extractor =
        ClaudeExtractor::new(API_KEY.to_owned(), "claude-opus-4-8".to_owned(), base_url);
    let error = extractor
        .extract(&sample_input())
        .await
        .expect_err("must fail");
    assert!(
        matches!(&error, Error::Dependency { service, .. } if service == "claude-api"),
        "wrong error: {error:?}"
    );
}

/// A refusal (or any response without the forced tool call) fails with
/// the stop reason named — diagnosable from the log line alone.
#[tokio::test]
async fn claude_missing_tool_call_names_the_stop_reason() {
    let (base_url, _) = spawn_claude_mock(json!({
        "model": "claude-opus-4-8",
        "stop_reason": "refusal",
        "content": []
    }))
    .await;
    let extractor =
        ClaudeExtractor::new(API_KEY.to_owned(), "claude-opus-4-8".to_owned(), base_url);
    let error = extractor
        .extract(&sample_input())
        .await
        .expect_err("must fail");
    assert!(
        matches!(&error, Error::Dependency { message, .. } if message.contains("refusal")),
        "wrong error: {error:?}"
    );
}

/// The vLLM impl speaks OpenAI-compatible chat completions, tolerates a
/// Markdown fence, and reports the endpoint's model string.
#[tokio::test]
async fn vllm_parses_fenced_json_completions() {
    let captured: Captured = Arc::new(Mutex::new(None));
    let inner = Arc::clone(&captured);
    let router = Router::new().route(
        "/v1/chat/completions",
        post(
            move |headers: axum::http::HeaderMap, Json(body): Json<Value>| {
                let captured = Arc::clone(&inner);
                async move {
                    *captured.lock().expect("capture lock") = Some((headers, body));
                    Json(json!({
                        "model": "qwen-72b-instruct",
                        "choices": [{
                            "message": {
                                "content": "```json\n{\"candidates\":[{\"class\":\"fact\",\
                                 \"content\":\"Fenced JSON parses.\",\"confidence\":0.7}]}\n```"
                            }
                        }]
                    }))
                }
            },
        ),
    );
    let base_url = spawn_mock(router).await;
    let extractor = VllmExtractor::new("qwen-72b-instruct".to_owned(), base_url);
    let outcome = extractor.extract(&sample_input()).await.expect("extract");

    let (_, body) = captured
        .lock()
        .expect("capture lock")
        .take()
        .expect("captured");
    assert_eq!(body["model"], "qwen-72b-instruct");
    assert_eq!(body["response_format"]["type"], "json_object");
    assert_eq!(body["messages"][0]["role"], "system");

    assert_eq!(outcome.method, "vllm");
    assert_eq!(outcome.model_version, "qwen-72b-instruct");
    assert_eq!(outcome.candidates.len(), 1);
    assert_eq!(outcome.candidates[0].class, RecordClass::Fact);
}

/// An error status is a `Dependency` error carrying the status, with the
/// body truncated — never a panic, never a silent empty outcome.
#[tokio::test]
async fn error_statuses_are_dependency_errors() {
    let router = Router::new().route(
        "/v1/chat/completions",
        post(|| async {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "vllm exploded",
            )
        }),
    );
    let base_url = spawn_mock(router).await;
    let extractor = VllmExtractor::new("m".to_owned(), base_url);
    let error = extractor
        .extract(&sample_input())
        .await
        .expect_err("must fail");
    assert!(
        matches!(&error, Error::Dependency { message, .. } if message.contains("500")),
        "wrong error: {error:?}"
    );
}
