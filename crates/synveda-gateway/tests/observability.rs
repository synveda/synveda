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
        pdp: std::sync::Arc::new(synveda_policy::Pdp::new().expect("build the embedded PDP")),
        scope_chains: std::sync::Arc::new(synveda_store::ScopeChainCache::new()),
        service_token_max_ttl: std::time::Duration::from_secs(3600),
        // These tests exercise the ops plane only; fail-closed default.
        verifier: std::sync::Arc::new(synveda_identity::DisabledVerifier),
        login: None,
        public_origin: "http://127.0.0.1:8120".to_owned(),
        search_index: Arc::new(
            synveda_retrieval::SearchIndex::open(
                std::env::temp_dir()
                    .join("synveda-gateway-tests")
                    .join(synveda_types::TenantId::new().to_string()),
            )
            .expect("open search index"),
        ),
        embedder: Arc::new(synveda_ingest::embedding::AnyEmbedder::Deterministic(
            synveda_ingest::embedding::DeterministicEmbedder::new(),
        )),
        inject_embed_timeout: std::time::Duration::from_millis(100),
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

// ── W3C trace context (ADR-0007's deferred clause) ─────────────────────
//
// ADR-0007 deferred `traceparent` extraction "to Phase 1 (ADPT-1/CTX-3),
// when external callers exist; the baseline emits new root traces per
// request". Those callers arrived and the extraction did not, so every
// trace still began at this process and the header ADPT-1 had been sending
// since it shipped was decorative.
//
// These tests read the spans the gateway would have *exported*, through a
// real `SdkTracerProvider` and an in-memory exporter, rather than asserting
// on a header the code parsed. The property is "an operator sees one trace
// across the boundary", and only the exported ids can say that.

/// A caller's trace id and span id, in the shape a `traceparent` carries.
const CALLER_TRACE: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
const CALLER_SPAN: &str = "00f067aa0ba902b7";

/// Serves one request through the real router with a real OTel pipeline
/// behind it, and returns the request span as it reached the exporter.
async fn exported_request_span(request: Request<Body>) -> opentelemetry_sdk::trace::SpanData {
    use opentelemetry::trace::TracerProvider as _;

    // The propagator is what turns the header into a context; the gateway
    // installs it in `telemetry::init`, which a test must not call (it
    // builds an OTLP exporter and installs a global subscriber).
    telemetry::install_propagator();

    let exporter = opentelemetry_sdk::trace::InMemorySpanExporter::default();
    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")));
    // Thread-local rather than global: the other tests in this binary set
    // their own, and #[tokio::test] is single-threaded.
    let _guard = tracing::subscriber::set_default(subscriber);

    let response = router(state(UNREACHABLE_URL))
        .oneshot(request)
        .await
        .unwrap();
    // `/healthz` touches no database, so this runs without one — the trace
    // shape is the subject here, not the readiness leg.
    assert_eq!(response.status(), StatusCode::OK);
    // The response body keeps `TraceLayer`'s span open, and an OTel span is
    // exported when its `tracing` span *closes*. Draining the body before
    // flushing is the difference between reading the request span and
    // reading an empty exporter — found by getting it wrong.
    assert_eq!(body_text(response).await, "ok");

    provider.force_flush().expect("flush spans");
    // The guard outlives the flush on purpose: a span that closed after the
    // subscriber went away would be recorded by nothing.
    drop(_guard);
    // `make_request_span` sets `otel.name` to `VERB /route`, and
    // tracing-opentelemetry *renames the exported span to it* — which is how
    // Jaeger shows an operation rather than a literal `http.request`. So the
    // span is found by the name an operator would see, not by the macro's.
    exporter
        .get_finished_spans()
        .expect("exported spans")
        .into_iter()
        .find(|span| span.name == "GET /healthz")
        .expect("the request span reached the exporter")
}

#[tokio::test]
async fn a_callers_traceparent_becomes_this_requests_parent() {
    let _serial = serial().await;
    let request_span = exported_request_span(
        Request::get("/healthz")
            .header("traceparent", format!("00-{CALLER_TRACE}-{CALLER_SPAN}-01"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    // Same trace as the caller, and a child of the caller's span. Both
    // halves matter: matching the trace id alone would pass on a span that
    // joined the trace as a second root, which is not one trace in Jaeger.
    assert_eq!(
        request_span.span_context.trace_id().to_string(),
        CALLER_TRACE,
        "the request did not join the caller's trace",
    );
    assert_eq!(
        request_span.parent_span_id.to_string(),
        CALLER_SPAN,
        "the request span is not a child of the caller's span",
    );
}

#[tokio::test]
async fn without_a_traceparent_the_request_still_roots_its_own_trace() {
    let _serial = serial().await;
    let request_span =
        exported_request_span(Request::get("/healthz").body(Body::empty()).unwrap()).await;
    assert!(
        request_span.span_context.trace_id() != opentelemetry::trace::TraceId::INVALID,
        "a request with no caller context must still get a trace of its own",
    );
    assert_eq!(
        request_span.parent_span_id,
        opentelemetry::trace::SpanId::INVALID,
        "with nothing to join, the request span is a root — ADR-0007's baseline behaviour",
    );
}

/// A trace is plumbing, and refusing a request over its telemetry would
/// turn an observability feature into an availability one. Every one of
/// these is a header a real proxy or a buggy client can send.
#[tokio::test]
async fn a_malformed_traceparent_is_ignored_rather_than_refused() {
    for header in [
        "",
        "garbage",
        "00-not-hex-at-all-01",
        // Well-formed but all-zero ids: the spec's own "invalid" values.
        "00-00000000000000000000000000000000-0000000000000000-01",
    ] {
        let _serial = serial().await;
        let request_span = exported_request_span(
            Request::get("/healthz")
                .header("traceparent", header)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(
            request_span.parent_span_id,
            opentelemetry::trace::SpanId::INVALID,
            "traceparent {header:?} was treated as a usable parent",
        );
    }
}

/// The one malformed shape the SDK does **not** ignore, pinned so that a
/// tightened propagator shows up here rather than silently.
///
/// W3C requires a version-`00` trace-id to be exactly 32 hex digits.
/// `TraceContextPropagator` checks only that the field parses as hex, so a
/// short id is accepted and zero-padded. This is left as the SDK has it —
/// a length check in the gateway would be the first line of a second
/// implementation of a protocol we took a library for — and the cost is
/// bounded: a confusing trace, never an authorisation or an audit fact
/// (ADR-0007's compliance note).
#[tokio::test]
async fn a_short_trace_id_is_accepted_and_padded_by_the_sdk() {
    let _serial = serial().await;
    let request_span = exported_request_span(
        Request::get("/healthz")
            .header(
                "traceparent",
                format!("00-4bf92f3577b34da6-{CALLER_SPAN}-01"),
            )
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        request_span.span_context.trace_id().to_string(),
        "00000000000000004bf92f3577b34da6",
        "the SDK's padding behaviour changed; re-read `app::parent_context`",
    );
}

/// A `traceparent` from a future revision still joins the trace, which is
/// what W3C asks for: "if a higher version is detected, the implementation
/// SHOULD try to parse it by parsing the first 55 characters as version
/// 00". Pinned because the opposite — a newer client silently starting a
/// fresh trace at this hop — is the failure this whole clause exists to
/// remove, and it would look exactly like nothing being wrong.
#[tokio::test]
async fn a_traceparent_from_a_future_revision_is_parsed_forward() {
    let _serial = serial().await;
    let request_span = exported_request_span(
        Request::get("/healthz")
            .header("traceparent", format!("99-{CALLER_TRACE}-{CALLER_SPAN}-01"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        request_span.span_context.trace_id().to_string(),
        CALLER_TRACE
    );
    assert_eq!(request_span.parent_span_id.to_string(), CALLER_SPAN);
}

/// The trap that hid inside a single registry-level `EnvFilter`: it applied
/// to the span exporter too, so `RUST_LOG=warn` — a thing operators do to
/// production — silently stopped every trace being recorded, and FND-5's
/// acceptance criterion quietly stopped holding.
///
/// This asserts the arrangement rather than the symptom, because the
/// symptom needs a live collector: the OTel layer must carry its own floor
/// so that an `info` span is still *recorded* when the console filter is
/// `error`. If this fails, check `telemetry::init` for a filter that moved
/// back onto the registry.
#[tokio::test]
async fn a_quiet_console_still_records_spans_for_export() {
    let _serial = serial().await;
    use opentelemetry::trace::TracerProvider as _;
    use tracing_subscriber::Layer as _;

    let exporter = opentelemetry_sdk::trace::InMemorySpanExporter::default();
    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    // The shape `telemetry::init` builds: the console filter belongs to the
    // fmt layer, and the exporter keeps its own floor.
    let subscriber = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::sink)
                .with_filter(tracing_subscriber::EnvFilter::new("error")),
        )
        .with(
            tracing_opentelemetry::layer()
                .with_tracer(provider.tracer("test"))
                .with_filter(tracing_subscriber::filter::LevelFilter::INFO),
        );
    let guard = tracing::subscriber::set_default(subscriber);

    let response = router(state(UNREACHABLE_URL))
        .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_text(response).await, "ok");

    provider.force_flush().expect("flush spans");
    drop(guard);
    let spans = exporter.get_finished_spans().expect("exported spans");
    assert!(
        spans.iter().any(|span| span.name == "GET /healthz"),
        "a console filter of `error` suppressed the exported span; traces must not \
         depend on log verbosity — see telemetry::init",
    );
}
