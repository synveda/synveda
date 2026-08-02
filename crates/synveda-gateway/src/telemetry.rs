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

/// Tokens included in each composed inject block. The name was registered
/// here from day one (FND-5); the constant now lives in the emitting crate
/// (CTX-2, ADR-0025 decision 8) and is re-exported for the recorder wiring.
/// A tracked SLO metric (research digest A1).
pub use synveda_retrieval::TOKENS_PER_INJECT;

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
/// `outcome`: `accepted` (staged and enqueued), `duplicate` (idempotency
/// key already admitted — reported, never re-enqueued), `quarantined`
/// (staged signal-less behind a pending review, MEM-2 ADR-0021), or
/// `denied` (refused per event; nothing persisted).
pub const OBSERVE_EVENTS_TOTAL: &str = "synveda_observe_events_total";

/// Redaction findings on the observe scan seam (MEM-2, ADR-0021),
/// labelled by `rule` and `category` (`secret`/`pii`). Counts findings
/// only — matched text appears nowhere, metrics included.
pub const REDACTION_FINDINGS_TOTAL: &str = "synveda_redaction_findings_total";

/// Quarantine review operations (MEM-2, ADR-0021 decision 6), labelled
/// by `op` (`list`/`release`/`reject`) and `outcome` (`ok`, `rejected`,
/// `error`). Release and reject chain `memory.quarantine.*` events;
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
/// because it is the same governed act a memory publication is.
pub const PROMPT_OPERATIONS_TOTAL: &str = "synveda_prompt_operations_total";

/// Lapse API operations (AUTHZ-4, ADR-0037), labelled by `op`
/// (`propose`/`grant`/`revoke`/`list`) and `outcome` (`ok`, `rejected`,
/// `error`).
///
/// `propose` chains `vedaflow.proposal.opened` like any other proposal —
/// a lapse is one — and `grant`/`revoke` chain `policy.lapse.granted` and
/// `policy.lapse.revoked`. The expiry sweep chains
/// `policy.lapse.expired` under `actor_kind=system` and is counted
/// separately: it is bookkeeping, not an operation anyone requested.
pub const LAPSE_OPERATIONS_TOTAL: &str = "synveda_lapse_operations_total";

/// Standing lapse grants that reached the end of their window and had
/// their expiry chained by the sweep (AUTHZ-4, ADR-0037 decision 4).
///
/// Deliberately *not* a gauge of standing grants: nothing needs to know
/// how many stand, because the read path asks the question per request.
/// A flat counter with a growing backlog in `policy_lapses` would mean
/// the sweep is wedged — which costs an audit line, never access.
pub const LAPSE_EXPIRIES_TOTAL: &str = "synveda_lapse_expiries_total";

/// Publications refused because the approval matrix could not be met by
/// the acting principal alone (FLOW-3, ADR-0032 decision 8), labelled by
/// `surface` (`channel` for the direct route, `proposal` for a
/// proposal's effect). Nonzero on `channel` is the product working: it
/// counts the publications that were pushed onto the review path.
pub const PUBLISH_REVIEW_REQUIRED_TOTAL: &str = "synveda_publish_review_required_total";

/// Proposals opened against an **ancestor** of the material's scope
/// (FLOW-5, ADR-0034), labelled by `levels` (how far up the chain the
/// target sits — `1` is the parent) and by the source and target scope
/// kinds. Same-scope proposals are not counted here: this measures the
/// org gradient being climbed, which is the thing the feature exists to
/// make observable, and a counter that also ticked for FLOW-3's case
/// would answer a different question.
pub const PROPOSAL_CLIMBS_TOTAL: &str = "synveda_proposal_climbs_total";

/// Curator-file edits (FLOW-3, ADR-0032 decision 15), labelled by `op`
/// (`get`/`put`) and `outcome`.
pub const CURATOR_OPERATIONS_TOTAL: &str = "synveda_curator_operations_total";

/// Inject requests (CTX-3, ADR-0026 decision 8), labelled by `outcome`,
/// funnel-collapsed worst-first: `error`, `rejected`, `degraded` (a
/// block was served under a degradation), `empty` (nothing composed),
/// `ok`. Each served inject chains one `context.injected` audit event.
pub const CONTEXT_INJECTS_TOTAL: &str = "synveda_context_injects_total";

/// Per-stage inject latency in seconds, labelled by `stage`
/// (`plan`/`embed`/`search`/`compose`/`audit` — audit includes the
/// commit). This is the measurement behind ADR-0019 option 2's trigger:
/// whether the chain append binds the read path is read here, not
/// guessed (ADR-0026 decision 9).
pub const INJECT_STAGE_SECONDS: &str = "synveda_inject_stage_duration_seconds";

/// Recall requests (CTX-4, ADR-0041), labelled by `outcome`, on the
/// inject counter's funnel: `error`, `rejected`, `empty` (nothing the
/// current plan admits), `degraded`, `ok` — and by `mode` (`ids` |
/// `query`) since CTX-5 gave the route two ways in (ADR-0042 decision 1).
/// Each served recall chains one `context.recalled` audit event.
pub const CONTEXT_RECALLS_TOTAL: &str = "synveda_context_recalls_total";

/// Per-stage recall latency in seconds, labelled by `stage`
/// (`plan`/`embed`/`search`/`admit`).
///
/// `plan` is the one this feature is most exposed on: ADR-0029 allotted
/// the plan stage **15ms** of a 300ms recall, and CTX-5 spends it on a
/// universe wider than the chain (ADR-0042 decisions 2 and 17). Whether
/// the widened sweep fits is read here, not guessed.
pub const RECALL_STAGE_SECONDS: &str = "synveda_recall_stage_duration_seconds";

/// Records served per recall, against records asked for — labelled
/// `side` (`requested`/`served`).
///
/// The gap is the measurement that matters on this surface: a handle is a
/// name rather than a capability (ADR-0041 decision 5), so a caller
/// naming ids it may no longer read is a *normal* outcome, and one whose
/// rate an operator should be able to see without reading the chain.
pub const RECALL_RECORDS_TOTAL: &str = "synveda_recall_records_total";

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
        .set_buckets_for_metric(
            Matcher::Full(INJECT_STAGE_SECONDS.to_owned()),
            // Stages subdivide the 150ms SLO (ADR-0026 decision 9);
            // finer low end than the HTTP histogram so the split is
            // readable when every stage is fast.
            &[
                0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.15, 0.25, 0.5,
            ],
        )
        .map_err(|err| internal(format!("metric buckets: {err}")))?
        .set_buckets_for_metric(
            Matcher::Full(RECALL_STAGE_SECONDS.to_owned()),
            // Stages subdivide ADR-0029's derived 300ms recall budget; the
            // 0.015 boundary is the plan-stage allowance CTX-5 spends on
            // the widened universe, so "did it fit" is one bucket edge.
            &[
                0.001, 0.0025, 0.005, 0.01, 0.015, 0.03, 0.08, 0.15, 0.3, 0.6,
            ],
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
    // MEM-3 metrics (ADR-0022): emitted in synveda-ingest's extraction
    // worker through the facade, described here where the recorder lives.
    metrics::describe_counter!(
        synveda_ingest::worker::EXTRACTION_EVENTS_TOTAL,
        "Staged events resolved by the extraction worker by outcome \
         (ok/empty/denied/dead_letter/error/skipped)"
    );
    metrics::describe_counter!(
        synveda_ingest::worker::EXTRACTION_RECORDS_TOTAL,
        "Derived records committed by the extraction pipeline by class"
    );
    metrics::describe_histogram!(
        synveda_ingest::worker::EXTRACTION_LAG_SECONDS,
        "Seconds from observe admission to extraction commit (seed §10: <60s SLO)"
    );
    metrics::describe_counter!(
        synveda_ingest::worker::EXTRACTOR_REQUESTS_TOTAL,
        "Extractor calls by method and outcome (ok/error)"
    );
    metrics::describe_histogram!(
        synveda_ingest::worker::EXTRACTOR_REQUEST_SECONDS,
        "Extractor call duration in seconds by method"
    );
    metrics::describe_counter!(
        synveda_ingest::worker::EXTRACTION_RESCAN_FINDINGS_TOTAL,
        "Redaction findings in extractor output (redacted before persistence, ADR-0022)"
    );
    // MEM-4 metrics (ADR-0023): emitted in the worker's embed stage.
    metrics::describe_counter!(
        synveda_ingest::worker::EMBEDDER_REQUESTS_TOTAL,
        "Embedder calls by method and outcome (ok/error)"
    );
    metrics::describe_histogram!(
        synveda_ingest::worker::EMBEDDER_REQUEST_SECONDS,
        "Embedder call duration in seconds by method"
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
    // CTX-1 metrics (ADR-0024): the search legs in synveda-retrieval's
    // hybrid engine; sweeps and document ops in its sidecar indexer.
    metrics::describe_counter!(
        synveda_retrieval::RETRIEVAL_SEARCHES_TOTAL,
        "Hybrid searches by mode (hybrid/sparse_only/dense_only/empty_filter)"
    );
    metrics::describe_histogram!(
        synveda_retrieval::RETRIEVAL_LEG_SECONDS,
        metrics::Unit::Seconds,
        "Retrieval leg latency by leg (dense/sparse/hydrate)"
    );
    metrics::describe_counter!(
        synveda_retrieval::SEARCH_INDEX_SWEEPS_TOTAL,
        "Search index sweeps per tenant by outcome (updated/empty/error)"
    );
    metrics::describe_counter!(
        synveda_retrieval::SEARCH_INDEX_DOCS_TOTAL,
        "Search index document operations by op (upsert/delete)"
    );
    // CTX-3 metrics (ADR-0026): emitted in the gateway's inject route.
    metrics::describe_counter!(
        CONTEXT_INJECTS_TOTAL,
        "Inject requests by outcome (ok/degraded/empty/rejected/error)"
    );
    metrics::describe_histogram!(
        INJECT_STAGE_SECONDS,
        metrics::Unit::Seconds,
        "Inject stage latency by stage (plan/embed/search/compose/audit)"
    );
    // CTX-4 metrics (ADR-0041): the index tier's cost, emitted in
    // composition, and the recall surface's, emitted in its route.
    metrics::describe_histogram!(
        synveda_retrieval::INDEX_TIER_TOKENS,
        "Estimated tokens spent on the index tier per composed block"
    );
    metrics::describe_counter!(
        CONTEXT_RECALLS_TOTAL,
        "Recall requests by outcome (ok/degraded/empty/rejected/error) and mode (ids/query)"
    );
    metrics::describe_counter!(
        RECALL_RECORDS_TOTAL,
        "Records asked for and served by recall, by side (requested/served)"
    );
    // CTX-5 (ADR-0042): the widened universe's cost, against ADR-0029's
    // 15ms plan-stage allowance.
    metrics::describe_histogram!(
        RECALL_STAGE_SECONDS,
        metrics::Unit::Seconds,
        "Recall stage latency by stage (plan/embed/search/admit)"
    );
    // GRPH-1 metrics (ADR-0043): emitted in synveda-store's graph module —
    // the traversal's own clock, against the slice ADR-0029's 300ms recall
    // decomposition reserves for graph expansion.
    metrics::describe_histogram!(
        synveda_store::graph::GRAPH_EXPANSION_SECONDS,
        metrics::Unit::Seconds,
        "Graph expansion latency by graph (entity/episode/provenance) and depth (one/two)"
    );
    metrics::describe_counter!(
        synveda_store::graph::GRAPH_EDGES_TOTAL,
        "Graph edges asserted by graph (entity/episode/provenance)"
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
        PROMPT_OPERATIONS_TOTAL,
        "Prompt registry operations by op (author/resolve/list) and \
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
         by levels climbed and source/target scope kind"
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
        "Composed context entries by the channel they composed from (published/derived) \
         and the tier they composed at (body/index)"
    );
    // MEM-6 counters (ADR-0040): emitted by the retention sweep. The
    // second one is the only counter in the product that measures data
    // being gone rather than hidden.
    metrics::describe_counter!(
        synveda_ingest::retention::RECORDS_EXPIRED_TOTAL,
        "Records expired out of the live corpus by class (the temporal delete)"
    );
    metrics::describe_counter!(
        synveda_ingest::retention::RETENTION_DESTROYED_TOTAL,
        "Rows destroyed by retention, by plane (history/staging/quarantine)"
    );
    // Touch the label-less histogram so it renders (count 0) before the first
    // inject exists — the FND-5 contract is visible in /metrics from boot.
    let _ = metrics::histogram!(TOKENS_PER_INJECT);

    Ok(handle)
}
