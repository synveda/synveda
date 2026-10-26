//! The LLM extractor implementations against in-process mock servers
//! (the MockIdp discipline: never a network dependency in tests). Covers
//! the request contract both impls promise — forced strict tool call for
//! Claude, JSON response format for vLLM, the opacity instruction in the
//! shared prompt — and the closed failure taxonomy the worker schedules or
//! closes on: malformed output and permanent statuses close, transient
//! statuses and transport loss retry, and a capped `Retry-After` hint is kept
//! only where it applies (ADR-0158).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::response::Json;
use axum::routing::post;
use axum::{Extension, Router};
use chrono::Utc;
use serde_json::{Value, json};
use std::time::Duration;

use synveda_ingest::extraction::{
    ClaudeExtractor, ExtractionInput, Extractor, ProviderFailure, VllmExtractor,
};
use synveda_types::capture::CaptureFailureCode;
use synveda_types::knowledge::{KnowledgeOrigin, KnowledgeType};
use synveda_types::session::SessionEventType;
use synveda_types::{Error, ScopeId, Sensitivity, SessionEventId, SessionId, TenantId};

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
        event_id: SessionEventId::new(),
        tenant_id: TenantId::new(),
        scope_id: ScopeId::new(),
        session_id: SessionId::new(),
        principal_id: "sess-http".to_owned(),
        event_type: SessionEventType::MessageUser,
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
                    "knowledge_type": "decision",
                    "title": "Redacted token decision",
                    "body_markdown": "The token [REDACTED:github-pat] stays redacted.",
                    "summary": "Keep the redacted token redacted.",
                    "tags": ["security"],
                    "confidence": 1.4,
                    "sensitivity": "confidential",
                    "entities": ["github"]
                }]
            }
        }]
    }))
    .await;
    let extractor =
        ClaudeExtractor::new(API_KEY.to_owned(), "claude-opus-4-8".to_owned(), base_url)
            .expect("configure Claude client");
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
    assert_eq!(candidate.knowledge_type, KnowledgeType::Decision);
    assert_eq!(candidate.origin, KnowledgeOrigin::Observed);
    assert!(candidate.body_markdown.contains("[REDACTED:github-pat]"));
    assert_eq!(candidate.confidence, 1.0, "confidence clamps into [0,1]");
    assert_eq!(candidate.sensitivity, Some(Sensitivity::Confidential));
    assert_eq!(candidate.entities, vec!["github".to_owned()]);
}

/// Output outside the candidates contract is a permanent invalid response,
/// never a partial persist or a reason to spend more provider calls.
#[tokio::test]
async fn claude_malformed_output_is_a_permanent_invalid_response() {
    let (base_url, _) = spawn_claude_mock(json!({
        "model": "claude-opus-4-8",
        "stop_reason": "tool_use",
        "content": [{
            "type": "tool_use",
            "id": "tu-1",
            "name": "emit_extraction",
            "input": { "candidates": "SYNVEDA_SECRET_SENTINEL_NOT_A_LIST" }
        }]
    }))
    .await;
    let extractor =
        ClaudeExtractor::new(API_KEY.to_owned(), "claude-opus-4-8".to_owned(), base_url)
            .expect("configure Claude client");
    let error = extractor
        .extract(&sample_input())
        .await
        .expect_err("must fail");
    assert_eq!(error.failure(), ProviderFailure::ResponseInvalid);
    assert!(!error.failure().code().retryable());
    let generic = error.into_error();
    assert!(
        matches!(&generic, Error::Dependency { service, message }
            if service == "claude-api" && message == "provider_response_invalid"),
        "wrong error: {generic:?}"
    );
    assert!(!error.to_string().contains("SYNVEDA_SECRET_SENTINEL"));
}

/// A refusal (or any response without the forced tool call) uses a closed
/// diagnostic code; provider response fields are not copied into logs.
#[tokio::test]
async fn claude_missing_tool_call_uses_a_content_free_code() {
    let (base_url, _) = spawn_claude_mock(json!({
        "model": "claude-opus-4-8",
        "stop_reason": "SYNVEDA_SECRET_SENTINEL_REFUSAL",
        "content": []
    }))
    .await;
    let extractor =
        ClaudeExtractor::new(API_KEY.to_owned(), "claude-opus-4-8".to_owned(), base_url)
            .expect("configure Claude client");
    let error = extractor
        .extract(&sample_input())
        .await
        .expect_err("must fail");
    assert_eq!(error.failure(), ProviderFailure::ResponseInvalid);
    assert_eq!(error.to_string(), "claude-api: provider_response_invalid");
}

/// Provider redirects are configuration failures, not an instruction to
/// forward the API credential to another origin.
#[tokio::test]
async fn claude_does_not_follow_redirects() {
    let redirected_requests = Arc::new(AtomicUsize::new(0));
    let redirected_counter = Arc::clone(&redirected_requests);
    let router = Router::new()
        .route(
            "/v1/messages",
            post(|| async {
                (
                    axum::http::StatusCode::TEMPORARY_REDIRECT,
                    [(axum::http::header::LOCATION, "/redirect-target")],
                )
            }),
        )
        .route(
            "/redirect-target",
            post(move || {
                let redirected_counter = Arc::clone(&redirected_counter);
                async move {
                    redirected_counter.fetch_add(1, Ordering::SeqCst);
                    Json(json!({}))
                }
            }),
        );
    let base_url = spawn_mock(router).await;
    let extractor =
        ClaudeExtractor::new(API_KEY.to_owned(), "claude-opus-4-8".to_owned(), base_url)
            .expect("configure Claude client");

    let error = extractor
        .extract(&sample_input())
        .await
        .expect_err("redirect must fail closed");

    assert_eq!(error.failure(), ProviderFailure::Rejected);
    assert!(!error.failure().code().retryable());
    assert_eq!(redirected_requests.load(Ordering::SeqCst), 0);
    assert!(!error.to_string().contains(API_KEY));
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
                                "content": "```json\n{\"candidates\":[{\"knowledge_type\":\"fact\",\
                                 \"title\":\"Fenced JSON\",\"body_markdown\":\"Fenced JSON parses.\",\
                                 \"summary\":\"Fenced JSON parses.\",\"confidence\":0.7}]}\n```"
                            }
                        }]
                    }))
                }
            },
        ),
    );
    let base_url = spawn_mock(router).await;
    let extractor = VllmExtractor::new("qwen-72b-instruct".to_owned(), base_url)
        .expect("configure vLLM client");
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
    assert_eq!(outcome.candidates[0].knowledge_type, KnowledgeType::Fact);
}

/// An error status keeps only its closed class. The upstream body may contain
/// prompts or credentials and is never surfaced.
#[tokio::test]
async fn error_statuses_keep_only_their_class() {
    let router = Router::new().route(
        "/v1/chat/completions",
        post(|| async {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "SYNVEDA_SECRET_SENTINEL_PROVIDER_BODY",
            )
        }),
    );
    let base_url = spawn_mock(router).await;
    let extractor = VllmExtractor::new("m".to_owned(), base_url).expect("configure vLLM client");
    let error = extractor
        .extract(&sample_input())
        .await
        .expect_err("must fail");
    assert_eq!(error.failure(), ProviderFailure::Transient);
    assert_eq!(
        error.failure().code(),
        CaptureFailureCode::ProviderTransient
    );
    assert!(error.failure().code().retryable());
    assert!(!error.to_string().contains("SYNVEDA_SECRET_SENTINEL"));
}

async fn status_mock(status: u16, retry_after: Option<String>) -> String {
    let router = Router::new().route(
        "/v1/chat/completions",
        post(move || {
            let retry_after = retry_after.clone();
            async move {
                let mut response = axum::response::Response::new(axum::body::Body::from(
                    "SYNVEDA_SECRET_SENTINEL_PROVIDER_BODY",
                ));
                *response.status_mut() =
                    axum::http::StatusCode::from_u16(status).expect("valid status");
                if let Some(value) = retry_after {
                    response.headers_mut().insert(
                        axum::http::header::RETRY_AFTER,
                        value.parse().expect("header value"),
                    );
                }
                response
            }
        }),
    );
    spawn_mock(router).await
}

async fn vllm_failure(base_url: String) -> synveda_ingest::extraction::ExtractionError {
    VllmExtractor::new("m".to_owned(), base_url)
        .expect("configure vLLM client")
        .extract(&sample_input())
        .await
        .expect_err("provider failure")
}

/// 429 and 503 retry and keep a capped hint; 401 and 400 close without one,
/// even when the provider sends a hint.
#[tokio::test]
async fn rate_limits_and_unavailability_keep_bounded_retry_hints() {
    let limited = vllm_failure(status_mock(429, Some("17".to_owned())).await).await;
    assert_eq!(limited.failure(), ProviderFailure::RateLimited);
    assert!(limited.failure().code().retryable());
    assert_eq!(limited.retry_after(), Some(Duration::from_secs(17)));

    let date = httpdate::fmt_http_date(std::time::SystemTime::now() + Duration::from_secs(600));
    let unavailable = vllm_failure(status_mock(503, Some(date)).await).await;
    assert_eq!(unavailable.failure(), ProviderFailure::Unavailable);
    let hinted = unavailable.retry_after().expect("date hint");
    assert!(hinted > Duration::from_secs(590) && hinted <= Duration::from_secs(600));

    let excessive = vllm_failure(status_mock(429, Some("999999999".to_owned())).await).await;
    assert_eq!(
        excessive.retry_after(),
        Some(synveda_ingest::retry::MAX_RETRY_AFTER)
    );
    let garbage = vllm_failure(status_mock(503, Some("tomorrow-ish".to_owned())).await).await;
    assert_eq!(garbage.retry_after(), None);

    for (status, failure) in [
        (401, ProviderFailure::Unauthorized),
        (400, ProviderFailure::Rejected),
    ] {
        let error = vllm_failure(status_mock(status, Some("5".to_owned())).await).await;
        assert_eq!(error.failure(), failure);
        assert!(!error.failure().code().retryable());
        assert_eq!(error.retry_after(), None);
        assert!(!error.to_string().contains("SYNVEDA_SECRET_SENTINEL"));
    }
}

/// A refused connection is a retryable unreachable provider; a connection
/// that closes mid-response is a retryable interruption.
#[tokio::test]
async fn transport_failures_are_retryable_and_content_free() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve port");
    let refused = format!("http://{}", listener.local_addr().expect("address"));
    drop(listener);
    let unreachable = vllm_failure(refused).await;
    assert_eq!(unreachable.failure(), ProviderFailure::Unreachable);
    assert!(unreachable.failure().code().retryable());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind closing fixture");
    let closing = format!("http://{}", listener.local_addr().expect("address"));
    tokio::spawn(async move {
        if let Ok((socket, _)) = listener.accept().await {
            drop(socket);
        }
    });
    let interrupted = vllm_failure(closing).await;
    assert_eq!(interrupted.failure(), ProviderFailure::Interrupted);
    assert!(interrupted.failure().code().retryable());
}

/// A provider that accepts the request and never answers fails as a
/// retryable timeout at the configured client deadline.
#[tokio::test]
async fn a_stalled_provider_is_a_retryable_timeout_at_the_configured_deadline() {
    let router = Router::new().route(
        "/v1/chat/completions",
        post(|| async {
            std::future::pending::<()>().await;
            Json(json!({}))
        }),
    );
    let base_url = spawn_mock(router).await;
    let extractor = VllmExtractor::new("m".to_owned(), base_url)
        .expect("configure vLLM client")
        .with_request_timeout(Duration::from_millis(200))
        .expect("bound the request");
    let started = std::time::Instant::now();
    let error = extractor
        .extract(&sample_input())
        .await
        .expect_err("stalled provider");
    assert_eq!(error.failure(), ProviderFailure::Timeout);
    assert!(error.failure().code().retryable());
    assert!(started.elapsed() < Duration::from_secs(5));
}
