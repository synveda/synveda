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
/// `admin` (admin-group subject with no team mapping, placed under the
/// org root — AUTHZ-3, ADR-0015 decision 6), `quarantined`, `existing`
/// (repeat login), or `error`. An AUD-1 emission point
/// (`identity.provisioned`) once the audit log lands.
pub const JIT_PROVISIONS_TOTAL: &str = "synveda_jit_provisions_total";

/// Role admin operations (AUTHZ-3, ADR-0015 decision 7), labelled by `op`
/// (list/bind/unbind/list_node/bind_node/unbind_node) and `outcome`
/// (`ok`, `rejected`, `error`). Mutations — the JIT admin-group binding
/// included — are an AUD-1 emission point once the audit log lands.
pub const ROLE_OPERATIONS_TOTAL: &str = "synveda_role_operations_total";

/// Service-identity admin operations (AUTH-3, ADR-0018 decision 3),
/// labelled by `op` (register/list/get/remove) and `outcome` (`ok`,
/// `rejected`, `error`). Register and remove are AUD-1 emission points
/// once the audit log lands.
pub const SERVICE_IDENTITY_OPERATIONS_TOTAL: &str = "synveda_service_identity_operations_total";

/// Service tokens refused at the enforcement seam (AUTH-3, ADR-0018
/// decision 5), labelled by `reason` (`lifetime_exceeded`,
/// `lifetime_unknown` — no `iat`). An AUD-1 emission point once the audit
/// log lands.
pub const SERVICE_TOKEN_REJECTIONS_TOTAL: &str = "synveda_service_token_rejections_total";

/// Observe ingestion batches (MEM-1, ADR-0020), labelled by `outcome`
/// (`ok`, `rejected`, `error`). Each `ok` batch chains one
/// `memory.observed` audit event.
pub const OBSERVE_BATCHES_TOTAL: &str = "synveda_observe_batches_total";

/// Observe events admitted to the buffer (MEM-1, ADR-0020), labelled by
/// `outcome`: `accepted` (staged and enqueued) or `duplicate`
/// (idempotency key already admitted — reported, never re-enqueued).
pub const OBSERVE_EVENTS_TOTAL: &str = "synveda_observe_events_total";

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
        "JIT identity provisioning by outcome (mapped/admin/quarantined/existing/error)"
    );
    // AUTHZ-3 counter (ADR-0015): operations in the gateway's roles routes.
    metrics::describe_counter!(
        ROLE_OPERATIONS_TOTAL,
        "Role admin operations by op and outcome (ok/rejected/error)"
    );
    // AUTH-3 counters (ADR-0018): operations in the gateway's
    // service-identity routes; rejections at the enforcement seam.
    metrics::describe_counter!(
        SERVICE_IDENTITY_OPERATIONS_TOTAL,
        "Service-identity admin operations by op and outcome (ok/rejected/error)"
    );
    metrics::describe_counter!(
        SERVICE_TOKEN_REJECTIONS_TOTAL,
        "Service tokens refused at the enforcement seam by reason \
         (lifetime_exceeded/lifetime_unknown)"
    );
    // MEM-1 counters (ADR-0020): emitted in the gateway's observe route.
    metrics::describe_counter!(
        OBSERVE_BATCHES_TOTAL,
        "Observe ingestion batches by outcome (ok/rejected/error)"
    );
    metrics::describe_counter!(
        OBSERVE_EVENTS_TOTAL,
        "Observe events admitted to the buffer by outcome (accepted/duplicate)"
    );
    // AUD-1 counters (ADR-0019): appends and verifications in
    // synveda-audit through the facade; best-effort append failures at
    // the gateway's error-path emission seams.
    metrics::describe_counter!(
        synveda_audit::AUDIT_EVENTS_TOTAL,
        "Audit events appended to tenant chains by action and outcome"
    );
    metrics::describe_counter!(
        synveda_audit::AUDIT_APPEND_FAILURES_TOTAL,
        "Audit appends that failed on a best-effort path (the event is lost, never the response)"
    );
    metrics::describe_counter!(
        synveda_audit::AUDIT_VERIFICATIONS_TOTAL,
        "Audit chain verifications by outcome (valid/broken)"
    );
    // HIER-2 counters (ADR-0016): emitted in synveda-store's scope-chain
    // resolver; invalidations by the hierarchy-mutating handlers.
    metrics::describe_counter!(
        synveda_store::scope_chain::SCOPE_CHAIN_RESOLUTIONS_TOTAL,
        "Scope chain resolutions by outcome (hit/miss)"
    );
    metrics::describe_counter!(
        synveda_store::scope_chain::SCOPE_CHAIN_INVALIDATIONS_TOTAL,
        "Tenant-wide scope-chain cache flushes after committed hierarchy mutations"
    );
    // HIER-3 counters (ADR-0017): fragments in synveda-policy's entity
    // store; flushes at the gateway's unified hierarchy-invalidation seam.
    metrics::describe_counter!(
        synveda_policy::CEDAR_ENTITY_FRAGMENTS_TOTAL,
        "Cedar entity fragment resolutions by outcome (hit/rebuild)"
    );
    metrics::describe_counter!(
        synveda_policy::CEDAR_ENTITY_FLUSHES_TOTAL,
        "Tenant-wide Cedar entity fragment flushes at the hierarchy-invalidation seam"
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
