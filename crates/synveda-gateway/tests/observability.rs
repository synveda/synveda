//! FND-5 tests: the ops routes respond, the Prometheus contract (including
//! `synveda_tokens_per_inject`) renders from boot, and one readiness request
//! produces the gateway→core→store span chain the Jaeger AC relies on.
//!
//! The span-chain test needs a live Postgres: it reads `DATABASE_URL` and
//! skips with a message when unset (CI has no database); run it locally with
//! `make db-test` or via `demos/fnd-5-observability.sh`.

use std::sync::{Arc, Mutex, OnceLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use tower::ServiceExt;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

/// The Prometheus recorder and tracing's callsite-interest cache are both
/// process-global: a test running with no subscriber can race another test's
/// collector registration and leave `#[instrument]` callsites cached as
/// disabled. Tests serialise on this lock instead of sharing global state.
async fn serial() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

/// The Prometheus recorder is process-global; install it once for the whole
/// test binary.
fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn state(url: &str) -> AppState {
    AppState {
        pool: PgPoolOptions::new()
            .max_connections(2)
            // Default is 30s; the unreachable-storage test should fail fast,
            // and localhost acquires are far under this.
            .acquire_timeout(std::time::Duration::from_secs(2))
            .connect_lazy(url)
            .expect("parse database url"),
        metrics: metrics_handle(),
    }
}

/// A URL that parses but connects nowhere, for tests that must not touch a
/// database.
const UNREACHABLE_URL: &str = "postgres://nobody:nothing@127.0.0.1:1/void";

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read response body")
        .to_bytes();
    String::from_utf8(bytes.to_vec()).expect("utf-8 body")
}

#[tokio::test]
async fn healthz_is_alive_without_a_database() {
    let _serial = serial().await;
    let response = router(state(UNREACHABLE_URL))
        .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn metrics_exposes_the_tokens_per_inject_contract() {
    let _serial = serial().await;
    // The middleware records after a response completes; serve one request
    // first so the labelled HTTP series exist in the exposition.
    let warmup = router(state(UNREACHABLE_URL))
        .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(warmup.status(), StatusCode::OK);

    let response = router(state(UNREACHABLE_URL))
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    // Registered at startup, before any inject exists (ADR-0007): the SLO
    // metric must be scrapeable from boot, not from first use.
    assert!(
        body.contains("# TYPE synveda_tokens_per_inject histogram"),
        "tokens_per_inject histogram missing from exposition:\n{body}"
    );
    assert!(
        body.contains("synveda_http_requests_total"),
        "http request counter missing from exposition:\n{body}"
    );
}

#[tokio::test]
async fn readyz_degrades_to_503_when_storage_is_unreachable() {
    let _serial = serial().await;
    let response = router(state(UNREACHABLE_URL))
        .oneshot(Request::get("/readyz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    // The body stays generic; failure detail belongs to the trace and log.
    assert_eq!(body_text(response).await, "not ready");
}

// ── Span-chain test (needs a live Postgres) ─────────────────────────────────

/// `(span name, contextual parent name)` for every span opened.
type SpanLog = Arc<Mutex<Vec<(String, Option<String>)>>>;

/// Records the [`SpanLog`] via a thread-local default subscriber.
#[derive(Clone)]
struct SpanCollector {
    spans: SpanLog,
}

impl<S> tracing_subscriber::Layer<S> for SpanCollector
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        _id: &tracing::span::Id,
        ctx: Context<'_, S>,
    ) {
        let parent = if attrs.is_contextual() {
            ctx.lookup_current().map(|span| span.name().to_owned())
        } else {
            attrs
                .parent()
                .and_then(|id| ctx.span(id))
                .map(|span| span.name().to_owned())
        };
        self.spans
            .lock()
            .unwrap()
            .push((attrs.metadata().name().to_owned(), parent));
    }
}

#[tokio::test]
async fn readyz_produces_the_gateway_core_store_span_chain() {
    let _serial = serial().await;
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping span-chain test: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
            return;
        }
    };

    let spans = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(SpanCollector {
        spans: Arc::clone(&spans),
    });
    // Thread-local default: #[tokio::test] runs single-threaded, so every
    // span this request opens lands in the collector.
    let _guard = tracing::subscriber::set_default(subscriber);

    let response = router(state(&url))
        .oneshot(Request::get("/readyz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let spans = spans.lock().unwrap();
    let parent_of = |name: &str| {
        spans
            .iter()
            .find(|(span, _)| span == name)
            .unwrap_or_else(|| panic!("span '{name}' was never opened; saw {spans:?}"))
            .1
            .clone()
    };
    // The AC's trace shape: one request span rooting core and store legs.
    assert_eq!(
        parent_of("http.request"),
        None,
        "request span must be a root"
    );
    assert_eq!(
        parent_of("retrieval.readiness").as_deref(),
        Some("http.request")
    );
    assert_eq!(
        parent_of("store.ping").as_deref(),
        Some("retrieval.readiness")
    );
}
