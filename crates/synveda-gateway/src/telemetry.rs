//! Tracing and metrics wiring (FND-5, ADR-0007). This is the only place in
//! the workspace that touches the OpenTelemetry SDK or a metrics recorder;
//! every other crate instruments through the `tracing`/`metrics` facades.

use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::SdkTracerProvider;
use synveda_types::{Error, Result};
use tracing_subscriber::EnvFilter;
// `with_filter` is the per-layer form; without this import both layers
// would share one registry-level filter — see `init`'s doc comment for why
// that is not merely a style choice.
use tracing_subscriber::Layer as _;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Tokens included in each composed context run. The constant lives in the
/// emitting crate and is re-exported for recorder wiring.
/// A tracked SLO metric (research digest A1).
pub use synveda_retrieval::TOKENS_PER_CONTEXT_RUN;

/// Requests served, labelled by method/route/status.
pub const HTTP_REQUESTS_TOTAL: &str = "synveda_http_requests_total";

/// Tenant resolutions, labelled by outcome: `resolved`, `rejected`
/// (unauthenticated — the uniform 401), or `error` (storage/internal
/// failure). TEN-1; an AUD-1 emission point once the audit log lands.
pub const TENANT_RESOLUTIONS_TOTAL: &str = "synveda_tenant_resolutions_total";

/// Request latency in seconds, labelled by method/route/status.
pub const HTTP_REQUEST_DURATION_SECONDS: &str = "synveda_http_request_duration_seconds";

/// Core-worker readiness: 1 only while the supervisor is running and its
/// most recent dependency probe accepted the schema and runtime role.
pub const WORKER_READY: &str = "synveda_worker_ready";

/// Age in seconds of the core worker supervisor's scheduler heartbeat. This
/// is process-loop liveness, not progress of every owned task.
pub const WORKER_HEARTBEAT_AGE_SECONDS: &str = "synveda_worker_heartbeat_age_seconds";

/// Scope admin operations (CPR-7, ADR-0074), labelled by `op`
/// (`list`/`create`/`get`/`update`/`ancestors`/`descendants`) and
/// `outcome` (`ok`, `rejected` — the caller's fault, `error` — ours or an
/// operator's).
pub const SCOPE_OPERATIONS_TOTAL: &str = "synveda_scope_operations_total";

/// The workspace/project/repository plane's operations (CPR-4, ADR-0071),
/// labelled by `op` (`me`, `workspace.list`, `workspace.create`,
/// `workspace.get`, `workspace.update`, `project.list`, `project.create`,
/// `project.get`, `project.update`, `repository.list`, `repository.attach`,
/// `repository.detach`) and `outcome` (`ok`, `rejected`, `error`) — the same
/// three-outcome taxonomy every other admin plane uses.
pub const WORKSPACE_OPERATIONS_TOTAL: &str = "synveda_workspace_operations_total";

/// The access plane's operations (CPR-5, ADR-0072), labelled by `op`
/// (`members.list`, `member.add`, `member.remove`, `invite.create`,
/// `invite.list`, `invite.revoke`, `invite.accept`, `group.list`,
/// `group.create`, `group.update`, `grant.list`, `grant.create`,
/// `grant.revoke`) and `outcome` (`ok`, `rejected`, `error`) — the same
/// three-outcome taxonomy every other admin plane uses.
pub const ACCESS_OPERATIONS_TOTAL: &str = "synveda_access_operations_total";

/// Counter: session-plane operations, labelled `op` (`session.open`,
/// `session.list`, `session.get`, `session.events.append`, `session.end`,
/// `session.timeline`, `session.context_run`) and `outcome`
/// (`ok`/`rejected`/`error`). CPR-10, ADR-0076.
pub const SESSION_OPERATIONS_TOTAL: &str = "synveda_session_operations_total";

/// Policy pack reload sweeps' per-pack outcomes: `installed`, `removed`,
/// `unchanged`, or `error` (a stored pack that fails to compile keeps the
/// last-good compile in force — ADR-0012 decision 5). AUTHZ-1/AUTHZ-2.
pub const POLICY_PACK_RELOADS_TOTAL: &str = "synveda_policy_pack_reloads_total";

/// Policy-source catalogue operations (AUTHZ-2, CPR-30), labelled by `op`
/// (`packs`) and `outcome` (`ok`, `rejected`, `error`). Runtime selection is
/// measured separately by the Configuration plane.
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

/// Capability probes (CNSL-2, ADR-0058), labelled by `op` (`at_node`,
/// `batch`, and `pairs` for the fan-out size) and `outcome` (`ok`,
/// `rejected`, `error`, `decided`).
///
/// The `pairs` series is the one worth watching: it counts (node, action)
/// decisions, which is exactly the number that would have been audit rows
/// under a per-pair chaining rule (ADR-0058 decision 4). If it grows
/// faster than probe requests, a client has stopped bounding what it
/// renders.
pub const CAPABILITY_PROBES_TOTAL: &str = "synveda_capability_probes_total";

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

/// Redaction findings on the session-event intake seam,
/// labelled by `rule` and `category` (`secret`/`pii`). Counts findings
/// only — matched text appears nowhere, metrics included.
pub const REDACTION_FINDINGS_TOTAL: &str = "synveda_redaction_findings_total";

/// Session-event quarantine review operations, labelled
/// by `op` (`list`/`release`/`reject`) and `outcome` (`ok`, `rejected`,
/// `error`). Release and reject chain `session.quarantine.*` events;
/// list chains its allowed decision (ADR-0019 decision 4).
pub const QUARANTINE_OPERATIONS_TOTAL: &str = "synveda_quarantine_operations_total";

/// Audit query API operations (AUD-2, ADR-0045), labelled by `op`
/// (`events`/`disclosures`/`knowledge`/`verify`) and `outcome` (`ok`,
/// `rejected`, `error`).
///
/// Every allowed op also chains its own `authz.decision`, so reading the
/// trail is itself on the trail (ADR-0019 decision 4) — this counter and
/// that chain should agree, and a divergence means appends are failing.
pub const AUDIT_QUERY_OPERATIONS_TOTAL: &str = "synveda_audit_query_operations_total";

/// Channel API operations (FLOW-2, ADR-0031 decision 12; FLOW-7,
/// ADR-0036), labelled by `op` (`list`/`publish`/`history`/`rollback`/
/// `pin`/`unpin`) and `outcome` (`ok`, `rejected`, `error`). Publish,
/// rollback, pin, and unpin each chain their own `vedaflow.channel.*`
/// event; the two reads chain their allowed decision (ADR-0019
/// decision 4).
pub const CHANNEL_OPERATIONS_TOTAL: &str = "synveda_channel_operations_total";

/// Proposal API operations (FLOW-3, ADR-0032), labelled by `op`
/// (`list`/`get`/`open`/`approve`/`reject`/`withdraw`/`publish`) and
/// `outcome` (`ok`, `rejected`, `error`). Every op but the reads chains
/// its own `vedaflow.proposal.*` event; `publish` chains
/// `vedaflow.channel.published` with the proposal id, since it is the
/// same governed act as a direct publish (ADR-0032 decision 18).
pub const PROPOSAL_OPERATIONS_TOTAL: &str = "synveda_proposal_operations_total";

/// Prompt registry operations (PRMT-1, ADR-0049), labelled by `op`
/// (`author`/`resolve`/`list`) and `outcome` (`ok`, `rejected`, `error`).
///
/// `author` chains `prompt.authored` and `resolve` chains
/// `prompt.resolved`; the listing chains its allowed decision like every
/// other read (ADR-0019 decision 4). A publication is counted by
/// `CHANNEL_OPERATIONS_TOTAL` and chains `vedaflow.channel.published`,
/// because it is the same governed publication act.
pub const PROMPT_OPERATIONS_TOTAL: &str = "synveda_prompt_operations_total";

/// Context-pack registry operations (PRMT-2, ADR-0050), labelled by `op`
/// (`author`/`list`) and `outcome` (`ok`, `rejected`, `error`).
///
/// Two ops rather than three, and the missing one is the point: a pack is
/// not *fetched*. `author` chains `context_pack.authored` (or
/// `context_pack.quarantined` when the scanner stops a document), the
/// listing chains its allowed decision like every other read, and a pack's
/// content reaches a session through a context run where it is counted and
/// watermarked like every other authored entry.
pub const CONTEXT_PACK_OPERATIONS_TOTAL: &str = "synveda_context_pack_operations_total";

/// Skills registry operations (SKIL-1, ADR-0051), labelled by `op`
/// (`author`/`resolve`/`list`) and `outcome` (`ok`, `rejected`, `error`).
///
/// Three ops, like the prompt registry and unlike the pack one: a skill IS
/// fetched by name. What it is not is *composed* — nothing here appears in
/// `synveda_composed_entries_total`, because a skill's content becomes no
/// Knowledge revisions and enters no block (ADR-0051 decision 9). `author` chains
/// `skill.authored` (or `skill.quarantined` when the scanner stops a file),
/// `resolve` chains `skill.resolved`, and a publication is counted by
/// `CHANNEL_OPERATIONS_TOTAL` like every other one.
pub const SKILL_OPERATIONS_TOTAL: &str = "synveda_skill_operations_total";

/// Skills published over the quality gate's objection (SKIL-3, ADR-0053),
/// labelled by the `pack` that set the bar.
///
/// **The one number that says whether this gate is working.** ADR-0053
/// Publications refused because the approval matrix could not be met by
/// the acting principal alone (FLOW-3, ADR-0032 decision 8), labelled by
/// `surface` (`channel` for the direct route, `proposal` for a
/// proposal's effect). Nonzero on `channel` is the product working: it
/// counts the publications that were pushed onto the review path.
pub const PUBLISH_REVIEW_REQUIRED_TOTAL: &str = "synveda_publish_review_required_total";

/// Proposals opened against an ancestor scope, labelled by a bounded distance
/// bucket and the source and target scope kinds.
pub const PROPOSAL_CLIMBS_TOTAL: &str = "synveda_proposal_climbs_total";

/// Curator-file edits (FLOW-3, ADR-0032 decision 15), labelled by `op`
/// (`get`/`put`) and `outcome`.
pub const CURATOR_OPERATIONS_TOTAL: &str = "synveda_curator_operations_total";

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
///
/// # `RUST_LOG` quietens the console and nothing else
///
/// The two filters are per-layer on purpose, and it is worth saying why,
/// because the obvious arrangement — one `EnvFilter` on the registry — is
/// what was here and it had a trap in it. A registry-level filter applies
/// to *every* layer, so `RUST_LOG=warn` did not merely quieten the log: it
/// stopped `info`-level spans being recorded at all, and with them every
/// exported trace. FND-5's acceptance criterion ("a single trace visible in
/// Jaeger") silently stopped holding for anyone who turned their logs down,
/// which is a thing operators do to *production*. Measured before the fix:
/// at `RUST_LOG=warn` a request carrying a `traceparent` reached Jaeger not
/// at all; at `info`, it arrived.
///
/// So the span exporter takes a fixed `INFO` floor of its own and the
/// console keeps `RUST_LOG`. The trade-off, stated rather than discovered:
/// `RUST_LOG=debug` no longer deepens what is *traced*, only what is
/// printed. Traces are an operational contract with an SLO attached; log
/// verbosity is a knob. Widening the contract should be a decision, not a
/// side effect of an environment variable.
pub fn init(service_name: &'static str) -> Result<Telemetry> {
    install_propagator();
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

    let console = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(console))
        .with(
            tracing_opentelemetry::layer()
                .with_tracer(tracer)
                .with_filter(tracing_subscriber::filter::LevelFilter::INFO),
        )
        .try_init()
        .map_err(|err| Error::Internal {
            message: format!("tracing subscriber already installed: {err}"),
        })?;

    Ok(Telemetry { provider })
}

/// Installs the W3C trace-context propagator, which is what lets an
/// incoming `traceparent` become the parent of this request's span
/// (ADR-0007's deferred clause; see [`crate::app::parent_context`]).
///
/// Global rather than per-request because that is the only shape the OTel
/// API offers: `global::get_text_map_propagator` is how both the extractor
/// on the way in and any future injector on the way out find it. Installing
/// it twice is harmless — the second call replaces the first with an
/// identical value — so this is safe under the test binaries that call
/// [`init`] more than once.
pub fn install_propagator() {
    opentelemetry::global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );
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
            Matcher::Full(TOKENS_PER_CONTEXT_RUN.to_owned()),
            &[64.0, 128.0, 256.0, 512.0, 1024.0, 1536.0, 2048.0, 4096.0],
        )
        .map_err(|err| internal(format!("metric buckets: {err}")))?
        .set_buckets_for_metric(
            Matcher::Full(HTTP_REQUEST_DURATION_SECONDS.to_owned()),
            // Buckets bracket the 150ms context engineering budget (seed §10).
            &[0.005, 0.01, 0.025, 0.05, 0.1, 0.15, 0.25, 0.5, 1.0, 2.5],
        )
        .map_err(|err| internal(format!("metric buckets: {err}")))?
        .install_recorder()
        .map_err(|err| internal(format!("prometheus recorder: {err}")))?;

    metrics::describe_histogram!(
        TOKENS_PER_CONTEXT_RUN,
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
    metrics::describe_gauge!(
        WORKER_READY,
        "Core-worker supervisor readiness after lifecycle, database, schema and runtime-role checks"
    );
    metrics::describe_gauge!(
        WORKER_HEARTBEAT_AGE_SECONDS,
        metrics::Unit::Seconds,
        "Age of the core worker supervisor scheduler heartbeat; not per-task progress"
    );
    metrics::describe_counter!(
        SCOPE_OPERATIONS_TOTAL,
        "Hierarchy admin operations by op and outcome (ok/rejected/error)"
    );
    // CPR-4 counters (ADR-0071): the plane's own operations here, and the
    // store-side mutation counters beside the scope one below.
    metrics::describe_counter!(
        WORKSPACE_OPERATIONS_TOTAL,
        "Workspace, project and repository operations by op and outcome (ok/rejected/error)"
    );
    // CPR-5 counters (ADR-0072): the access plane's own operations here, its
    // store-side mutation counter beside the scope ones below.
    metrics::describe_counter!(
        ACCESS_OPERATIONS_TOTAL,
        "Group, grant, member and invitation operations by op and outcome (ok/rejected/error)"
    );
    // CPR-10 counters (ADR-0076): the session plane's own operations here, its
    // store-side ledger counter beside the other store counters below.
    metrics::describe_counter!(
        SESSION_OPERATIONS_TOTAL,
        "Session, event, timeline and context-run operations by op and outcome (ok/rejected/error)"
    );
    metrics::describe_counter!(
        synveda_store::sessions::SESSION_MUTATIONS_TOTAL,
        "Session-ledger row mutations by table (session/event/context_run) and operation"
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
    // AUTHZ-2 counters (ADR-0014): policy-source catalogue reads;
    // fail-safe resolution in synveda-policy.
    metrics::describe_counter!(
        POLICY_OPERATIONS_TOTAL,
        "Policy-source catalogue operations by op and outcome (ok/rejected/error)"
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
    // CNSL-2 counter (ADR-0058): probes in the gateway's capabilities
    // routes, plus the fan-out size the single audit event summarises.
    metrics::describe_counter!(
        CAPABILITY_PROBES_TOTAL,
        "Capability probes by op and outcome, and (node, action) pairs decided"
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
    // CPR-18 metrics (ADR-0083): extraction freezes session evidence and
    // produces reviewable candidates. Nothing here calls a candidate a
    // published record or retains the retired per-event queue vocabulary.
    metrics::describe_counter!(
        synveda_ingest::capture_worker::CAPTURE_BATCHES_TOTAL,
        "Capture batches processed by outcome"
    );
    metrics::describe_counter!(
        synveda_ingest::capture_worker::CAPTURE_CANDIDATES_TOTAL,
        "Reviewable capture candidates persisted by Knowledge type"
    );
    metrics::describe_counter!(
        synveda_ingest::capture_worker::CAPTURE_EXTRACTOR_REQUESTS_TOTAL,
        "Extractor calls by method and outcome (ok/error)"
    );
    metrics::describe_histogram!(
        synveda_ingest::capture_worker::CAPTURE_EXTRACTOR_SECONDS,
        "Extractor call duration in seconds by method"
    );
    metrics::describe_counter!(
        synveda_store::capture::CAPTURE_MUTATIONS_TOTAL,
        "Capture mutation statements accepted inside caller-owned transactions; transaction outcome is reported by the enclosing API or worker metric"
    );
    metrics::describe_counter!(
        crate::capture::CAPTURE_API_OPERATIONS_TOTAL,
        "Capture API operations by op and outcome (ok/rejected/error)"
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
    // (The HIER-2 chain-cache counters left with the cache and the tree it
    // read — CPR-7, ADR-0074. The description block is kept rather than
    // silently dropped so a scrape comparing releases sees the series end
    // rather than a gap.)
    // CPR-3 (ADR-0070): the generic scope substrate. Emitted in
    // synveda-store's scope services; described here where the recorder lives
    // (ADR-0007). No route reaches those services yet — the governed entry
    // points land with the later prompts of the context-platform programme —
    // so this series is expected to be absent rather than zero until then.
    metrics::describe_counter!(
        synveda_store::scopes::SCOPE_MUTATIONS_TOTAL,
        "Scope tree mutations by operation (create/rename/move/status)"
    );
    // CPR-4 (ADR-0071): the product-level subtypes above those scopes.
    // Unlike the scope counter above, these series appear as soon as anybody
    // uses the product — /v1/workspaces is the route the scope services
    // finally have.
    metrics::describe_counter!(
        synveda_store::workspaces::SUBTYPE_MUTATIONS_TOTAL,
        "Workspace and project mutations by subtype and operation (create/update)"
    );
    metrics::describe_counter!(
        synveda_store::repositories::REPOSITORY_MUTATIONS_TOTAL,
        "Repository attachments by operation (attach/detach)"
    );
    // CPR-5 (ADR-0072): who holds what, and where it came from.
    metrics::describe_counter!(
        synveda_store::access::ACCESS_MUTATIONS_TOTAL,
        "Access-plane mutations by object (group/membership/grant/invite) and \
         operation (create/update/revoke/accept)"
    );
    // TEN-4 key plane (ADR-0064). Emitted in synveda-store and the gateway,
    // described here where the recorder lives (ADR-0007).
    metrics::describe_counter!(
        synveda_store::keys::KEY_UNWRAPS_TOTAL,
        "Data keys unwrapped through the KMS, by scope. A rate that tracks \
         request rate rather than sitting near zero means the key cache is \
         not working"
    );
    metrics::describe_counter!(
        synveda_store::keys::KEY_CACHE_LOOKUPS_TOTAL,
        "Key-ring lookups by scope and outcome (hit/miss)"
    );
    metrics::describe_counter!(
        synveda_store::keys::KEYS_MINTED_TOTAL,
        "Data keys minted, by scope and reason (provision/rotate)"
    );
    metrics::describe_counter!(
        synveda_store::keys::KEY_OPEN_FAILURES_TOTAL,
        "Sealed payloads that did not open, by scope and purpose. Nothing in \
         normal operation fails to open: anything but zero is corruption, a \
         missing key, or the cross-tenant transplant ADR-0064 decision 4 \
         turns into a failure"
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
    // Authored-context summary cost, emitted by composition.
    metrics::describe_histogram!(
        synveda_retrieval::AUTHORED_SUMMARY_TOKENS,
        "Estimated tokens spent abbreviating authored context"
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
    // FLOW-1's counters (ADR-0030 decision 14 deferred describing them to
    // whichever feature made the binary call that crate — FLOW-2 did),
    // plus FLOW-2's own.
    metrics::describe_counter!(
        synveda_vedaflow::objects::OBJECTS_WRITTEN_TOTAL,
        "VedaFlow object writes by asset kind and result (stored/deduplicated)"
    );
    metrics::describe_counter!(
        synveda_vedaflow::trees::TREES_WRITTEN_TOTAL,
        "VedaFlow tree writes by result (stored/deduplicated)"
    );
    metrics::describe_counter!(
        synveda_vedaflow::commits::COMMITS_WRITTEN_TOTAL,
        "VedaFlow commit writes by signer and result (stored/deduplicated)"
    );
    metrics::describe_counter!(
        synveda_vedaflow::refs::REF_UPDATES_TOTAL,
        "VedaFlow ref updates by outcome (updated/raced/not_fast_forward)"
    );
    metrics::describe_counter!(
        synveda_vedaflow::verify::VERIFICATIONS_TOTAL,
        "VedaFlow store verifications by outcome"
    );
    metrics::describe_counter!(
        synveda_vedaflow::channels::CHANNEL_COMMITS_TOTAL,
        "VedaFlow channel commits by asset, channel, and outcome (committed/contended)"
    );
    metrics::describe_counter!(
        CHANNEL_OPERATIONS_TOTAL,
        "Channel API operations by op (list/publish/history/rollback/pin/unpin) and \
         outcome (ok/rejected/error)"
    );
    metrics::describe_counter!(
        PROPOSAL_OPERATIONS_TOTAL,
        "Proposal API operations by op and outcome (ok/rejected/error)"
    );
    metrics::describe_counter!(
        crate::relaxations::RELAXATION_OPERATIONS_TOTAL,
        "Governed relaxation API operations by operation and outcome (ok/rejected/error)"
    );
    metrics::describe_counter!(
        crate::relaxations::RELAXATION_EXPIRIES_TOTAL,
        "Immutable relaxation versions whose hard expiry was chained by the bookkeeping sweep"
    );
    metrics::describe_counter!(
        PROMPT_OPERATIONS_TOTAL,
        "Prompt registry operations by op (author/resolve/list) and \
         outcome (ok/rejected/error)"
    );
    metrics::describe_counter!(
        CONTEXT_PACK_OPERATIONS_TOTAL,
        "Context pack registry operations by op (author/list) and \
         outcome (ok/rejected/error)"
    );
    metrics::describe_counter!(
        SKILL_OPERATIONS_TOTAL,
        "Skills registry operations by op (author/resolve/list) and \
         outcome (ok/rejected/error)"
    );
    metrics::describe_counter!(
        AUDIT_QUERY_OPERATIONS_TOTAL,
        "Audit query API operations by op (events/disclosures/knowledge/verify) and \
         outcome (ok/rejected/error)"
    );
    metrics::describe_counter!(
        PUBLISH_REVIEW_REQUIRED_TOTAL,
        "Publications refused for want of approvals, by surface (channel/proposal)"
    );
    metrics::describe_counter!(
        PROPOSAL_CLIMBS_TOTAL,
        "Proposals opened against an ancestor of the material's scope, \
         by bounded distance and source/target scope kind"
    );
    metrics::describe_counter!(
        CURATOR_OPERATIONS_TOTAL,
        "Curator-file operations by op (get/put) and outcome (ok/rejected/error)"
    );
    metrics::describe_counter!(
        synveda_vedaflow::proposals::PROPOSAL_ACTS_TOTAL,
        "VedaFlow proposal lifecycle acts by act and asset kind"
    );
    metrics::describe_counter!(
        synveda_retrieval::COMPOSED_ENTRIES_TOTAL,
        "Published authored context chunks selected by rendered tier"
    );
    // AUTH-4 (ADR-0059): the directory plane. Reads there are metered and
    // traced rather than chained (decision 14), so these three counters and
    // the `scim.*` spans are the whole record of a provisioning agent's
    // polling — which makes describing them load-bearing rather than
    // decorative.
    metrics::describe_counter!(
        SCIM_REQUESTS_TOTAL,
        "SCIM plane requests by authentication outcome (authenticated/rejected)"
    );
    metrics::describe_counter!(
        SCIM_RECONCILES_TOTAL,
        "Directory reconciliations by outcome \
         (provisioned/adopted/moved/moved_and_sealed/sealed/unchanged)"
    );
    metrics::describe_counter!(
        SCIM_CREDENTIAL_OPERATIONS_TOTAL,
        "Provisioning-credential admin operations by op (issue/list/revoke) and outcome"
    );
    // Touch the label-less histogram so it renders (count 0) before the first
    // inject exists — the FND-5 contract is visible in /metrics from boot.
    let _ = metrics::histogram!(TOKENS_PER_CONTEXT_RUN);

    Ok(handle)
}

/// SCIM plane requests by authentication outcome (`authenticated`,
/// `rejected`) — AUTH-4, ADR-0059 decision 14. Reads on that plane are
/// metered and traced rather than chained, so this counter and the
/// `scim.*` spans are the whole record of a provisioning agent's polling.
pub const SCIM_REQUESTS_TOTAL: &str = "synveda_scim_requests_total";

/// Reconciliations by outcome (`provisioned`, `moved`, `sealed`,
/// `adopted`, `unchanged`, `quarantined`) — what the directory actually
/// changed, as opposed to what it sent.
pub const SCIM_RECONCILES_TOTAL: &str = "synveda_scim_reconciles_total";

/// Provisioning-credential admin operations by op and outcome — the
/// `/v1/scim/credentials` plane's counter (AUTH-4, ADR-0059 decision 13).
pub const SCIM_CREDENTIAL_OPERATIONS_TOTAL: &str = "synveda_scim_credential_operations_total";
