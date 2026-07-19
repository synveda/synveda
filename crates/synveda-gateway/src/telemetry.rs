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

/// Hierarchy admin operations, labelled by `op` (create/get/root/children/
/// ancestors/descendants/update/delete) and `outcome` (`ok`, `rejected` —
/// the caller's fault, `error` — ours or an operator's). HIER-1; an AUD-1
/// emission point once the audit log lands.
pub const HIERARCHY_OPERATIONS_TOTAL: &str = "synveda_hierarchy_operations_total";

/// Policy pack reload sweeps' per-pack outcomes: `installed`, `removed`,
/// `unchanged`, or `error` (a stored pack that fails to compile keeps the
/// last-good compile in force — ADR-0012 decision 5). AUTHZ-1/AUTHZ-2.
pub const POLICY_PACK_RELOADS_TOTAL: &str = "synveda_policy_pack_reloads_total";

/// Policy admin operations (AUTHZ-2, ADR-0014 decision 8), labelled by
/// `op` (packs/get_default/set_default/clear_default/get_node_policy/
/// assign_node_policy/unassign_node_policy) and `outcome` (`ok`,
/// `rejected`, `error`). Mutations are an AUD-1 emission point once the
/// audit log lands.
pub const POLICY_OPERATIONS_TOTAL: &str = "synveda_policy_operations_total";

/// JIT provisioning outcomes at login (AUTH-2, ADR-0013): `mapped`,
/// `quarantined`, `existing` (repeat login), or `error`. An AUD-1 emission
/// point (`identity.provisioned`) once the audit log lands.
pub const JIT_PROVISIONS_TOTAL: &str = "synveda_jit_provisions_total";

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
    metrics::describe_counter!(
        HIERARCHY_OPERATIONS_TOTAL,
        "Hierarchy admin operations by op and outcome (ok/rejected/error)"
    );
    // AUTHZ-1 counters (ADR-0012): the decision counter is emitted in
    // synveda-policy through the facade, the reload counter in the
    // gateway's refresher; both described here where the recorder lives.
    metrics::describe_counter!(
        synveda_policy::AUTHZ_DECISIONS_TOTAL,
        "Authorization decisions by action, decision (allow/deny), and pack"
    );
    metrics::describe_counter!(
        POLICY_PACK_RELOADS_TOTAL,
        "Policy pack reloads by outcome (installed/removed/unchanged/error)"
    );
    // AUTHZ-2 counters (ADR-0014): operations in the gateway's policy
    // routes; fallbacks in synveda-policy's effective-pack resolution.
    metrics::describe_counter!(
        POLICY_OPERATIONS_TOTAL,
        "Policy admin operations by op and outcome (ok/rejected/error)"
    );
    metrics::describe_counter!(
        synveda_policy::POLICY_PACK_FALLBACKS_TOTAL,
        "Assigned pack names that resolved to no compiled pack (fell back to regulated-strict)"
    );
    // AUTH-2 counter (ADR-0013): emitted in the gateway's provisioning
    // module at login completion.
    metrics::describe_counter!(
        JIT_PROVISIONS_TOTAL,
        "JIT identity provisioning by outcome (mapped/quarantined/existing/error)"
    );
    // AUTH-1 counters (ADR-0010): emitted in synveda-identity through the
    // facade, described here where the recorder lives (ADR-0007).
    metrics::describe_counter!(
        synveda_identity::TOKEN_VERIFICATIONS_TOTAL,
        "Bearer-token verifications by issuer and outcome (ok/rejected/error)"
    );
    metrics::describe_counter!(
        synveda_identity::JWKS_REFRESHES_TOTAL,
        "JWKS refreshes by issuer and outcome (ok/error)"
    );
    metrics::describe_counter!(
        synveda_identity::OIDC_LOGINS_TOTAL,
        "OIDC logins by issuer and outcome (started/completed/rejected/error)"
    );
    // Touch the label-less histogram so it renders (count 0) before the first
    // inject exists — the FND-5 contract is visible in /metrics from boot.
    let _ = metrics::histogram!(TOKENS_PER_INJECT);

    Ok(handle)
}
