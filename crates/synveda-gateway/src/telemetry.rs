//! Tracing and metrics wiring (FND-5, ADR-0007). This is the only place in
//! the workspace that touches the OpenTelemetry SDK or a metrics recorder;
//! every other crate instruments through the `tracing`/`metrics` facades.

use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::SdkTracerProvider;
use synveda_types::{Error, Result};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Tokens included in each composed inject block. Registered here so the
/// contract exists from day one; the composition engine (CTX-2) records into
/// it through the `metrics` facade. A tracked SLO metric (research digest A1).
pub const TOKENS_PER_INJECT: &str = "synveda_tokens_per_inject";

/// Requests served, labelled by method/route/status.
pub const HTTP_REQUESTS_TOTAL: &str = "synveda_http_requests_total";

/// Tenant resolutions, labelled by outcome: `resolved`, `rejected`
/// (unauthenticated — the uniform 401), or `error` (storage/internal
/// failure). TEN-1; an AUD-1 emission point once the audit log lands.
pub const TENANT_RESOLUTIONS_TOTAL: &str = "synveda_tenant_resolutions_total";

/// Request latency in seconds, labelled by method/route/status.
pub const HTTP_REQUEST_DURATION_SECONDS: &str = "synveda_http_request_duration_seconds";

/// Handle to the installed tracer provider. Call [`Telemetry::shutdown`] on
/// exit to flush batched spans; dropping without it can lose the tail of the
/// trace.
pub struct Telemetry {
    provider: SdkTracerProvider,
}

/// Installs the global tracing subscriber: fmt logs filtered by `RUST_LOG`
/// (default `info`) plus an OTLP/gRPC span exporter reading the standard
/// `OTEL_EXPORTER_OTLP_ENDPOINT` (default `http://localhost:4317` — Jaeger in
/// the dev compose). Call once, from `main`, inside the Tokio runtime.
pub fn init(service_name: &'static str) -> Result<Telemetry> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .build()
        .map_err(|err| Error::Dependency {
            service: "otlp-exporter".to_owned(),
            message: err.to_string(),
        })?;
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(Resource::builder().with_service_name(service_name).build())
        .build();
    let tracer = provider.tracer(service_name);

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .try_init()
        .map_err(|err| Error::Internal {
            message: format!("tracing subscriber already installed: {err}"),
        })?;

    Ok(Telemetry { provider })
}

impl Telemetry {
    /// Flushes pending spans and shuts the exporter down.
    pub fn shutdown(self) {
        if let Err(err) = self.provider.shutdown() {
            // The subscriber is being torn down; stderr is the honest channel.
            eprintln!("telemetry shutdown failed: {err}");
        }
    }
}

/// Installs the process-global Prometheus recorder and pre-registers the
/// metric contract. Fails if a recorder is already installed (call once).
pub fn init_metrics() -> Result<PrometheusHandle> {
    let internal = |message: String| Error::Internal { message };
    let handle = PrometheusBuilder::new()
        // Budget-shaped buckets: the default inject budget is 1,500 tokens
        // (seed §4.4); the tail catches misconfigured budgets.
        .set_buckets_for_metric(
            Matcher::Full(TOKENS_PER_INJECT.to_owned()),
            &[64.0, 128.0, 256.0, 512.0, 1024.0, 1536.0, 2048.0, 4096.0],
        )
        .map_err(|err| internal(format!("metric buckets: {err}")))?
        .set_buckets_for_metric(
            Matcher::Full(HTTP_REQUEST_DURATION_SECONDS.to_owned()),
            // The inject SLO is p99 < 150ms (seed §10); buckets bracket it.
            &[0.005, 0.01, 0.025, 0.05, 0.1, 0.15, 0.25, 0.5, 1.0, 2.5],
        )
        .map_err(|err| internal(format!("metric buckets: {err}")))?
        .install_recorder()
        .map_err(|err| internal(format!("prometheus recorder: {err}")))?;

    metrics::describe_histogram!(
        TOKENS_PER_INJECT,
        metrics::Unit::Count,
        "Tokens included in each composed inject context block"
    );
    metrics::describe_counter!(HTTP_REQUESTS_TOTAL, "HTTP requests served by the gateway");
    metrics::describe_counter!(
        TENANT_RESOLUTIONS_TOTAL,
        "Tenant resolutions by outcome (resolved/rejected/error)"
    );
    metrics::describe_histogram!(
        HTTP_REQUEST_DURATION_SECONDS,
        metrics::Unit::Seconds,
        "Gateway HTTP request latency"
    );
    // Touch the label-less histogram so it renders (count 0) before the first
    // inject exists — the FND-5 contract is visible in /metrics from boot.
    let _ = metrics::histogram!(TOKENS_PER_INJECT);

    Ok(handle)
}
