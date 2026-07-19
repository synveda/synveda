//! The gateway's HTTP surface: the unauthenticated ops plane (liveness,
//! readiness, Prometheus) and the authenticated `/v1` plane behind tenant
//! resolution (TEN-1). The three primitives land on `/v1` behind the full
//! AuthN → tenant → PDP → audit chain (CTX-3, MEM-1).

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{MatchedPath, Request, State};
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

use crate::auth;
use crate::error::ApiError;
use crate::hierarchy;
use crate::policy;
use crate::roles;
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
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            tenant::resolve_tenant,
        ));
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(render_metrics))
        // The auth plane is unauthenticated by nature: it is how a caller
        // becomes authenticated (AUTH-1, ADR-0010).
        .route("/auth/login", get(auth::login))
        .route("/auth/callback", get(auth::callback))
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
