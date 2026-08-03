//! The gateway's HTTP surface: the unauthenticated ops plane (liveness,
//! readiness, Prometheus) and the authenticated `/v1` plane behind tenant
//! resolution (TEN-1). The three primitives land on `/v1` behind the full
//! AuthN → tenant → PDP → audit chain (CTX-3, MEM-1).

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::service_identities;

use axum::Router;
use axum::extract::{DefaultBodyLimit, MatchedPath, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post, put};
use metrics_exporter_prometheus::PrometheusHandle;
use serde::Serialize;
use sqlx::PgPool;
use synveda_identity::{LoginFlow, TokenVerifier};
use synveda_policy::Pdp;
use synveda_types::{Error, Tenant};
use tower_http::trace::TraceLayer;

use crate::audit_query;
use crate::auth;
use crate::channels;
use crate::curators;
use crate::error::ApiError;
use crate::hierarchy;
use crate::inject;
use crate::observe;
use crate::packs;
use crate::policy;
use crate::prompts;
use crate::proposals;
use crate::quarantine;
use crate::recall;
use crate::roles;
use crate::skills;
use crate::telemetry::{HTTP_REQUEST_DURATION_SECONDS, HTTP_REQUESTS_TOTAL};
use crate::tenant;

/// Shared state for all routes.
#[derive(Clone)]
pub struct AppState {
    /// Lazily-connecting Postgres pool; `/readyz` surfaces connection
    /// failures rather than the process refusing to boot.
    pub pool: PgPool,
    /// Renders the Prometheus exposition for `GET /metrics`.
    pub metrics: PrometheusHandle,
    /// The AuthN seam (ADR-0008): the OIDC/JWKS verifier (ADR-0010), the
    /// HS256 dev verifier, or the fail-closed
    /// [`synveda_identity::DisabledVerifier`] when neither is configured.
    pub verifier: Arc<dyn TokenVerifier>,
    /// The code+PKCE login flow when OIDC is configured (AUTH-1); `None`
    /// otherwise, in which case `/auth/*` answers 404.
    pub login: Option<Arc<LoginFlow>>,
    /// The embedded PDP (AUTHZ-1, ADR-0012): handlers authorize through it
    /// before acting; the pack refresher hot-swaps stored packs into it.
    pub pdp: Arc<Pdp>,
    /// The scope-chain resolver (HIER-2, ADR-0016): read-through cache
    /// over the closure table; the hierarchy-mutating handlers invalidate
    /// it post-commit through [`AppState::invalidate_hierarchy`].
    pub scope_chains: Arc<synveda_store::ScopeChainCache>,
    /// The service-token lifetime cap (AUTH-3, ADR-0018 decision 5):
    /// the enforcement seam refuses a service identity's token whose
    /// lifetime (`exp − iat`) is unknown or exceeds this.
    /// `SYNVEDA_SERVICE_TOKEN_MAX_TTL_SECS`, default 3600.
    pub service_token_max_ttl: Duration,
    /// The search-index sidecar (CTX-1, ADR-0024): the inject route's
    /// lexical leg (CTX-3, ADR-0026); the indexer task converges it.
    pub search_index: Arc<synveda_retrieval::SearchIndex>,
    /// The MEM-4 embedder seam (ADR-0023): the inject route's
    /// query-embedding call and the pipeline worker's record vectors
    /// share one config-declared model identity (ADR-0026 decision 3).
    pub embedder: Arc<synveda_ingest::embedding::AnyEmbedder>,
    /// The inject route's embed deadline (ADR-0026 decision 3):
    /// `SYNVEDA_INJECT_EMBED_TIMEOUT_MS`, default 100. Expiry drops the
    /// dense leg (sparse-only, marked degraded), never the request.
    pub inject_embed_timeout: Duration,
}

impl AppState {
    /// The one post-commit seam for every hierarchy mutation (ADR-0016
    /// decision 5, ADR-0017 decision 5): flushes the tenant's cached
    /// scope chains and its Cedar entity fragments, so the very next
    /// request re-reads committed truth. Any future hierarchy writer
    /// (AUTH-4 SCIM, AUTH-5 directory sync) calls this — never the two
    /// caches individually.
    pub fn invalidate_hierarchy(&self, tenant_id: synveda_types::TenantId) {
        self.scope_chains.invalidate(tenant_id);
        self.pdp.flush_entities(tenant_id);
    }
}

/// Builds the gateway router: ops-plane routes plus the authenticated `/v1`
/// plane, wrapped in the per-request trace span and HTTP metrics middleware.
pub fn router(state: AppState) -> Router {
    // Every /v1 route sits behind tenant resolution; ops routes do not.
    // The hierarchy admin plane (HIER-1) additionally authorizes every
    // operation through the PDP inside its handlers (AUTHZ-1, ADR-0012).
    let authenticated = Router::new()
        .route("/v1/whoami", get(whoami))
        .route("/v1/hierarchy/nodes", post(hierarchy::create))
        .route("/v1/hierarchy/root", get(hierarchy::root))
        .route(
            "/v1/hierarchy/nodes/{id}",
            get(hierarchy::get)
                .patch(hierarchy::update)
                .delete(hierarchy::delete),
        )
        .route(
            "/v1/hierarchy/nodes/{id}/children",
            get(hierarchy::children),
        )
        .route(
            "/v1/hierarchy/nodes/{id}/ancestors",
            get(hierarchy::ancestors),
        )
        .route(
            "/v1/hierarchy/nodes/{id}/descendants",
            get(hierarchy::descendants),
        )
        // The policy admin plane (AUTHZ-2, ADR-0014 decision 8).
        .route("/v1/policy/packs", get(policy::packs))
        .route(
            "/v1/policy/default",
            get(policy::get_default)
                .put(policy::set_default)
                .delete(policy::clear_default),
        )
        .route(
            "/v1/hierarchy/nodes/{id}/policy",
            put(policy::assign_node_policy)
                .get(policy::get_node_policy)
                .delete(policy::unassign_node_policy),
        )
        // The role admin plane (AUTHZ-3, ADR-0015 decision 7).
        .route(
            "/v1/roles/bindings",
            get(roles::list)
                .put(roles::bind_tenant_wide)
                .delete(roles::unbind_tenant_wide),
        )
        .route(
            "/v1/hierarchy/nodes/{id}/roles",
            get(roles::list_node)
                .put(roles::bind_node)
                .delete(roles::unbind_node),
        )
        // The observe primitive (MEM-1, ADR-0020): the data plane's write
        // seam. Its body limit covers the worst-case batch; every other
        // route keeps axum's default. The redaction scan runs inside it
        // (MEM-2, ADR-0021).
        .route(
            "/v1/observe",
            post(observe::create).layer(DefaultBodyLimit::max(observe::BODY_LIMIT_BYTES)),
        )
        // The inject primitive (CTX-3, ADR-0026): the read path's
        // session-start seam — plan, retrieve, compose, one chained
        // audit event.
        .route("/v1/inject", post(inject::create))
        // The recall primitive (CTX-4, ADR-0041): the bodies behind the
        // handles an inject block's index tier handed out. The plan is
        // re-decided per call — a handle is a name, not a capability —
        // and the same `admit` the block composed under answers here.
        .route("/v1/recall", post(recall::create))
        // The quarantine review plane (MEM-2, ADR-0021 decisions 5–7).
        .route("/v1/quarantine", get(quarantine::list))
        .route(
            "/v1/quarantine/{event_id}/release",
            post(quarantine::release),
        )
        .route("/v1/quarantine/{event_id}/reject", post(quarantine::reject))
        // The audit query plane (AUD-2, ADR-0045): one action, `AuditRead`,
        // decided at the tenant — there is no scope-resource variant, so
        // an audit answer covers the whole chain or is refused. The two
        // AC questions get one call each; `verify` is the chain check the
        // CLI has had since AUD-1, now reachable without DATABASE_URL.
        .route("/v1/audit/events", get(audit_query::events))
        .route("/v1/audit/disclosures", get(audit_query::disclosures))
        .route("/v1/audit/knowledge", get(audit_query::knowledge))
        .route("/v1/audit/verify", get(audit_query::verify))
        // The VedaFlow channel plane (FLOW-2, ADR-0031 decision 12):
        // reading a scope's standing channels, and publishing records
        // across the trust boundary onto its published one. Since FLOW-3
        // the publish resolves the same approval matrix a proposal does,
        // satisfied by the acting principal alone (ADR-0032 decision 8).
        .route("/v1/channels/{scope_id}", get(channels::list))
        .route("/v1/channels/{scope_id}/publish", post(channels::publish))
        // Rollback and pinning (FLOW-7, ADR-0036). `history` is the
        // listing a rewind is chosen from and renders exactly the set the
        // rewind accepts; `pin` holds what the channel serves without
        // moving where it points, and is released by deleting it.
        .route("/v1/channels/{scope_id}/history", get(channels::history))
        .route("/v1/channels/{scope_id}/rollback", post(channels::rollback))
        .route("/v1/channels/{scope_id}/pin", post(channels::pin))
        .route("/v1/channels/{scope_id}/unpin", post(channels::unpin))
        // The VedaFlow proposal plane (FLOW-3, ADR-0032): the review in
        // front of a publication. Opening asks, approving counts, and
        // publishing runs the effect under `ChannelPublish` — approvals
        // go in front of that decision, they do not replace it.
        .route("/v1/proposals", get(proposals::list).post(proposals::open))
        .route("/v1/proposals/{id}", get(proposals::get))
        .route("/v1/proposals/{id}/approve", post(proposals::approve))
        .route("/v1/proposals/{id}/reject", post(proposals::reject))
        .route("/v1/proposals/{id}/withdraw", post(proposals::withdraw))
        .route("/v1/proposals/{id}/publish", post(proposals::publish))
        .route("/v1/proposals/{id}/classify", post(proposals::classify))
        // The prompt registry (PRMT-1, ADR-0049). Authoring writes a draft
        // and moves nothing a consumer reads; resolution walks the caller's
        // own placement chain nearest-first, or serves a named scope's
        // draft or a commit the caller pins. The wildcard is the path
        // shape of a prompt name (decision 3), and it sits *after* the
        // collection route so `GET /v1/prompts?scope_id=…` still lists.
        .route("/v1/prompts", get(prompts::list).post(prompts::author))
        .route("/v1/prompts/{*name}", get(prompts::resolve))
        // The context-pack registry (PRMT-2, ADR-0050). There is no
        // `GET /v1/context-packs/{name}` resolve route, and that is the
        // difference between the two authored asset types rather than an
        // omission: a prompt is fetched by name, and a pack's content
        // arrives through `/v1/inject` as ranked pinned material.
        .route("/v1/context-packs", get(packs::list).post(packs::author))
        // The skills registry (SKIL-1, ADR-0051). Shaped like the prompt
        // registry and not the pack one, because a skill IS fetched by name
        // — and unlike a prompt, what comes back is the whole bundle from
        // one commit, since a client loads a skill whole. There is no
        // materialisation route: writing files into a client's own skills
        // directory is `synveda skill install`, because the harness is a
        // guest (seed §2.6, ADR-0051 decision 12).
        .route("/v1/skills", get(skills::list).post(skills::author))
        .route("/v1/skills/{name}", get(skills::resolve))
        // The lapse plane (AUTHZ-4, ADR-0037). `POST /v1/lapses` opens a
        // *proposal* and grants nothing; the grant is that proposal's
        // effect, beside `/publish` and taking the same shape.
        .route("/v1/proposals/{id}/lapse", post(crate::lapses::grant))
        .route(
            "/v1/lapses",
            get(crate::lapses::list).post(crate::lapses::propose),
        )
        .route("/v1/lapses/{id}/revoke", post(crate::lapses::revoke))
        // CODEOWNERS-style curator files (FLOW-3, ADR-0032 decisions
        // 13–15), under the policy plane's own actions: they add required
        // approvers and grant nothing.
        .route(
            "/v1/hierarchy/nodes/{id}/curators",
            get(curators::get).put(curators::put),
        )
        // The service-identity plane (AUTH-3, ADR-0018 decision 3).
        .route(
            "/v1/service-identities",
            get(service_identities::list).post(service_identities::register),
        )
        .route(
            "/v1/service-identities/{id}",
            get(service_identities::get).delete(service_identities::remove),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            tenant::resolve_tenant,
        ));
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(render_metrics))
        // The auth plane is unauthenticated by nature: it is how a caller
        // becomes authenticated (AUTH-1, ADR-0010). The two CLI routes
        // serve `synveda login` over the same flow (ADPT-1, ADR-0027
        // decisions 5 and 6) — they exist so the client needs no OAuth
        // configuration and no client credentials of its own.
        .route("/auth/login", get(auth::login))
        .route("/auth/callback", get(auth::callback))
        .route("/auth/cli/exchange", post(auth::cli_exchange))
        .route("/auth/refresh", post(auth::refresh))
        .merge(authenticated)
        .layer(middleware::from_fn(track_http_metrics))
        // Added last so the request span is outermost and every inner span —
        // middleware included — nests under it.
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(make_request_span)
                .on_response(record_response),
        )
        .with_state(state)
}

/// Liveness: the process is up. No dependencies touched.
async fn healthz() -> &'static str {
    "ok"
}

/// Readiness: walks the real crate layering (gateway→retrieval→store) down to
/// a Postgres round-trip. This is the FND-5 end-to-end traced request; it
/// reads no application data (ADR-0007).
async fn readyz(State(state): State<AppState>) -> Response {
    match synveda_retrieval::readiness(&state.pool).await {
        Ok(()) => (StatusCode::OK, "ready").into_response(),
        Err(err) => {
            tracing::error!(error = %err, "readiness check failed");
            // The detail is in the trace and the log; the body stays generic.
            (StatusCode::SERVICE_UNAVAILABLE, "not ready").into_response()
        }
    }
}

/// Prometheus exposition endpoint.
async fn render_metrics(State(state): State<AppState>) -> String {
    state.metrics.render()
}

#[derive(Serialize)]
struct WhoamiResponse {
    subject: String,
    tenant: Tenant,
}

/// Introspection: who does the gateway think is calling? Returns only the
/// caller's own resolution result — no governed assets, so no PDP
/// involvement (ADR-0008). Reads the task-local rather than a request
/// extension: this endpoint exists to prove the propagation path end to end.
async fn whoami() -> Response {
    match synveda_identity::current_tenant() {
        Some(context) => Json(WhoamiResponse {
            subject: context.claims.subject,
            tenant: context.tenant,
        })
        .into_response(),
        None => ApiError(Error::Internal {
            message: "authenticated route ran outside a tenant scope".to_owned(),
        })
        .into_response(),
    }
}

/// One span per request. `otel.name` gives Jaeger the `VERB /route` operation
/// name; the status code is recorded on response, and `tenant.id` by the
/// tenant-resolution middleware once resolution succeeds (TEN-1 AC).
fn make_request_span(request: &Request) -> tracing::Span {
    let route = matched_route(request);
    tracing::info_span!(
        "http.request",
        otel.name = %format!("{} {}", request.method(), route),
        otel.kind = "server",
        http.request.method = %request.method(),
        http.route = %route,
        url.path = %request.uri().path(),
        http.response.status_code = tracing::field::Empty,
        tenant.id = tracing::field::Empty,
    )
}

fn record_response(response: &Response, _latency: Duration, span: &tracing::Span) {
    span.record("http.response.status_code", response.status().as_u16());
}

/// Counts and times every request, labelled by method/route/status. Routes
/// are matched patterns, not raw paths, to keep label cardinality bounded.
async fn track_http_metrics(request: Request, next: Next) -> Response {
    let method = request.method().as_str().to_owned();
    let route = matched_route(&request);
    let start = Instant::now();
    let response = next.run(request).await;
    let labels = [
        ("method", method),
        ("route", route),
        ("status", response.status().as_u16().to_string()),
    ];
    metrics::counter!(HTTP_REQUESTS_TOTAL, &labels).increment(1);
    metrics::histogram!(HTTP_REQUEST_DURATION_SECONDS, &labels)
        .record(start.elapsed().as_secs_f64());
    response
}

fn matched_route(request: &Request) -> String {
    request.extensions().get::<MatchedPath>().map_or_else(
        || request.uri().path().to_owned(),
        |path| path.as_str().to_owned(),
    )
}
