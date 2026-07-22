//! The TEI embedder against in-process mock servers (the MockIdp
//! discipline: never a network dependency in tests). Covers the request
//! contract — `POST /embed` with the inputs array — and the failure
//! taxonomy the worker's retry flow depends on: error statuses, count
//! mismatches, empty vectors, and an unreachable endpoint are all
//! `Error::Dependency` (ADR-0023 decision 6), never a partial result.

use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::post;
use axum::{Extension, Router};
use serde_json::{Value, json};
use synveda_ingest::embedding::{Embedder, TeiEmbedder};
use synveda_types::Error;

/// The last request body the mock saw, for post-assertions.
type Captured = Arc<Mutex<Option<Value>>>;

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

async fn tei_mock(
    State(response): State<(StatusCode, Value)>,
    Extension(captured): Extension<Captured>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    *captured.lock().expect("capture lock") = Some(body);
    (response.0, Json(response.1))
}

async fn spawn_tei_mock(status: StatusCode, response: Value) -> (String, Captured) {
    let captured: Captured = Arc::new(Mutex::new(None));
    let router = Router::new()
        .route("/embed", post(tei_mock))
        .layer(Extension(Arc::clone(&captured)))
        .with_state((status, response));
    (spawn_mock(router).await, captured)
}

fn embedder(base_url: String) -> TeiEmbedder {
    TeiEmbedder::new("mock-model".to_owned(), base_url)
}

/// The success contract: one `POST /embed` with the inputs in order,
/// vectors returned in the same order, model identity as configured.
#[tokio::test]
async fn embed_round_trips_inputs_in_order() {
    let (base_url, captured) =
        spawn_tei_mock(StatusCode::OK, json!([[0.1, 0.2, 0.3], [0.4, 0.5, 0.6]])).await;
    let embedder = embedder(base_url);
    assert_eq!(embedder.method(), "tei");
    assert_eq!(embedder.model(), "mock-model");

    let inputs = vec!["first text".to_owned(), "second text".to_owned()];
    let vectors = embedder.embed(&inputs).await.expect("embed");
    assert_eq!(vectors, vec![vec![0.1, 0.2, 0.3], vec![0.4, 0.5, 0.6]]);

    let body = captured
        .lock()
        .expect("capture lock")
        .clone()
        .expect("the mock saw a request");
    assert_eq!(body["inputs"], json!(["first text", "second text"]));
}

/// No inputs, no network call: an empty batch resolves locally.
#[tokio::test]
async fn empty_batch_never_calls_the_endpoint() {
    let (base_url, captured) = spawn_tei_mock(StatusCode::OK, json!([])).await;
    let vectors = embedder(base_url).embed(&[]).await.expect("embed");
    assert!(vectors.is_empty());
    assert!(
        captured.lock().expect("capture lock").is_none(),
        "an empty batch must not reach the endpoint"
    );
}

/// TEI's error statuses surface as `Dependency` with the body's error
/// detail — the worker's signal-redelivery trigger.
#[tokio::test]
async fn error_status_is_a_dependency_error() {
    let (base_url, _) = spawn_tei_mock(
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({"error": "input exceeds the model context", "error_type": "validation"}),
    )
    .await;
    let err = embedder(base_url)
        .embed(&["too long".to_owned()])
        .await
        .expect_err("error status must fail");
    let Error::Dependency { service, message } = &err else {
        panic!("expected Dependency, got {err:?}");
    };
    assert_eq!(service, "tei");
    assert!(
        message.contains("input exceeds the model context"),
        "the TEI error detail must surface: {message}"
    );
}

/// A vector count that deviates from the input count is a dependency
/// failure, never a partial success — a zip would silently misattribute
/// vectors to contents.
#[tokio::test]
async fn count_mismatch_is_a_dependency_error() {
    let (base_url, _) = spawn_tei_mock(StatusCode::OK, json!([[0.1, 0.2]])).await;
    let err = embedder(base_url)
        .embed(&["one".to_owned(), "two".to_owned()])
        .await
        .expect_err("count mismatch must fail");
    assert!(
        matches!(&err, Error::Dependency { service, message }
            if service == "tei" && message.contains("expected 2 vectors, got 1")),
        "got {err:?}"
    );
}

/// An empty vector in an otherwise well-shaped response is refused: it
/// would become an unindexable row that still satisfies the count.
#[tokio::test]
async fn empty_vector_is_a_dependency_error() {
    let (base_url, _) = spawn_tei_mock(StatusCode::OK, json!([[0.1], []])).await;
    let err = embedder(base_url)
        .embed(&["one".to_owned(), "two".to_owned()])
        .await
        .expect_err("empty vector must fail");
    assert!(
        matches!(&err, Error::Dependency { service, .. } if service == "tei"),
        "got {err:?}"
    );
}

/// An unreachable endpoint (TEI killed) is a dependency error too — the
/// chaos path's building block.
#[tokio::test]
async fn unreachable_endpoint_is_a_dependency_error() {
    // Bind then drop: the port is real but nothing is listening.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    drop(listener);
    let err = embedder(format!("http://{addr}"))
        .embed(&["text".to_owned()])
        .await
        .expect_err("a dead endpoint must fail");
    assert!(
        matches!(&err, Error::Dependency { service, .. } if service == "tei"),
        "got {err:?}"
    );
}
