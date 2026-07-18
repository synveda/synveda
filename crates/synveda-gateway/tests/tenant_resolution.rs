//! TEN-1 acceptance criteria: a request without a resolvable tenant → 401,
//! and traces carry the tenant id. Plus the surrounding contract: uniform
//! 401 for unknown/suspended tenants, task-local propagation to the handler
//! (`/v1/whoami`), fail-closed storage errors, and the resolution counter.
//!
//! Tests that need a live Postgres read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database); run them locally with
//! `make db-test` or via `demos/ten-1-tenant-resolution.sh`.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_types::{TenantId, TenantStatus};
use tower::ServiceExt;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

const SECRET: &[u8] = b"ten-1-test-secret";

/// A URL that parses but connects nowhere, for tests that must not touch a
/// database.
const UNREACHABLE_URL: &str = "postgres://nobody:nothing@127.0.0.1:1/void";

/// Serialises tests: the Prometheus recorder and tracing's callsite-interest
/// cache are process-global (same rationale as tests/observability.rs).
async fn serial() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn pool(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        // Fail fast when a test points at the unreachable URL.
        .acquire_timeout(Duration::from_secs(2))
        .connect_lazy(url)
        .expect("parse database url")
}

fn state(url: &str) -> AppState {
    AppState {
        pool: pool(url),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        pdp: Arc::new(synveda_policy::Pdp::new().expect("build the embedded PDP")),
    }
}

fn issue(tenant_id: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue("test-subject", tenant_id, Duration::from_secs(300))
}

fn whoami_request(authorization: Option<&str>) -> Request<Body> {
    let builder = Request::get("/v1/whoami");
    let builder = match authorization {
        Some(value) => builder.header("authorization", value),
        None => builder,
    };
    builder.body(Body::empty()).unwrap()
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read response body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("json body")
}

/// Asserts the response is the uniform 401: status, `WWW-Authenticate`, and
/// a taxonomy body with kind `unauthenticated`.
async fn assert_unauthenticated(response: axum::response::Response) {
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response
            .headers()
            .get("www-authenticate")
            .map(|v| v.to_str().unwrap()),
        Some("Bearer"),
    );
    let body = body_json(response).await;
    assert_eq!(body["kind"], "unauthenticated", "body: {body}");
}

// ── No database needed ──────────────────────────────────────────────────────

#[tokio::test]
async fn request_without_a_token_is_401() {
    let _serial = serial().await;
    let response = router(state(UNREACHABLE_URL))
        .oneshot(whoami_request(None))
        .await
        .unwrap();
    assert_unauthenticated(response).await;
}

#[tokio::test]
async fn garbage_and_wrong_scheme_tokens_are_401() {
    let _serial = serial().await;
    for authorization in [
        "Bearer not-a-jwt",
        "Bearer ",
        "Basic dXNlcjpwYXNz",
        "no-scheme-at-all",
    ] {
        let response = router(state(UNREACHABLE_URL))
            .oneshot(whoami_request(Some(authorization)))
            .await
            .unwrap();
        assert_unauthenticated(response).await;
    }
}

#[tokio::test]
async fn valid_token_with_unreachable_storage_is_503_not_401() {
    let _serial = serial().await;
    // The token is fine; resolution fails on the operator side. The caller
    // must not be told their credentials are bad.
    let response = router(state(UNREACHABLE_URL))
        .oneshot(whoami_request(Some(&format!(
            "Bearer {}",
            issue(TenantId::new())
        ))))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = body_json(response).await;
    assert_eq!(body["kind"], "storage", "body: {body}");
}

#[tokio::test]
async fn rejections_increment_the_resolution_counter() {
    let _serial = serial().await;
    let app = router(state(UNREACHABLE_URL));
    let response = app.oneshot(whoami_request(None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let exposition = metrics_handle().render();
    assert!(
        exposition
            .lines()
            .any(|line| line.starts_with("synveda_tenant_resolutions_total")
                && line.contains("outcome=\"rejected\"")),
        "rejected outcome missing from exposition:\n{exposition}"
    );
}

#[tokio::test]
async fn ops_plane_needs_no_tenant() {
    let _serial = serial().await;
    let response = router(state(UNREACHABLE_URL))
        .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// ── Database-backed (skip without DATABASE_URL) ─────────────────────────────

/// Connects to `DATABASE_URL`, applies migrations, and admits one tenant per
/// (slug, status). `None` = no database configured; the test skips quietly.
async fn admitted_tenant(status: TenantStatus) -> Option<(String, TenantId)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping tenant-resolution DB test: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
            return None;
        }
    };
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");
    let id = TenantId::new();
    let slug = format!("ten1-{}", id.as_uuid().simple());
    synveda_store::tenants::create(&pool, id, &slug, "TEN-1 test tenant", status)
        .await
        .expect("admit tenant");
    Some((url, id))
}

#[tokio::test]
async fn resolvable_tenant_reaches_the_handler_through_the_task_local() {
    let _serial = serial().await;
    let Some((url, tenant_id)) = admitted_tenant(TenantStatus::Active).await else {
        return;
    };
    let response = router(state(&url))
        .oneshot(whoami_request(Some(&format!(
            "Bearer {}",
            issue(tenant_id)
        ))))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    // /v1/whoami reads the task-local, not a request extension: a correct
    // body proves middleware → task-local → handler propagation end to end.
    let body = body_json(response).await;
    assert_eq!(body["subject"], "test-subject", "body: {body}");
    assert_eq!(body["tenant"]["id"], tenant_id.to_string(), "body: {body}");
    assert_eq!(body["tenant"]["status"], "active", "body: {body}");
}

#[tokio::test]
async fn token_for_an_unknown_tenant_is_401() {
    let _serial = serial().await;
    // Migrations must exist for the lookup to run at all; admit a throwaway
    // tenant to get the harness, then present a token for a fresh id.
    let Some((url, _)) = admitted_tenant(TenantStatus::Active).await else {
        return;
    };
    let response = router(state(&url))
        .oneshot(whoami_request(Some(&format!(
            "Bearer {}",
            issue(TenantId::new())
        ))))
        .await
        .unwrap();
    assert_unauthenticated(response).await;
}

#[tokio::test]
async fn token_for_a_suspended_tenant_is_the_same_401() {
    let _serial = serial().await;
    let Some((url, tenant_id)) = admitted_tenant(TenantStatus::Suspended).await else {
        return;
    };
    let response = router(state(&url))
        .oneshot(whoami_request(Some(&format!(
            "Bearer {}",
            issue(tenant_id)
        ))))
        .await
        .unwrap();
    // Identical to the unknown-tenant rejection: no existence oracle.
    assert_unauthenticated(response).await;
}

// ── Traces carry tenant_id (the second half of the AC) ──────────────────────

/// Captured `(field, value)` pairs per span name, from creation and later
/// `Span::record` calls.
type FieldLog = Arc<Mutex<Vec<(String, String, String)>>>;

#[derive(Clone)]
struct FieldCollector {
    fields: FieldLog,
}

struct Visitor<'a> {
    span: &'a str,
    fields: &'a FieldLog,
}

impl tracing::field::Visit for Visitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.fields.lock().unwrap().push((
            self.span.to_owned(),
            field.name().to_owned(),
            format!("{value:?}"),
        ));
    }
}

impl<S> tracing_subscriber::Layer<S> for FieldCollector
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        _id: &tracing::span::Id,
        _ctx: Context<'_, S>,
    ) {
        attrs.record(&mut Visitor {
            span: attrs.metadata().name(),
            fields: &self.fields,
        });
    }

    fn on_record(
        &self,
        id: &tracing::span::Id,
        values: &tracing::span::Record<'_>,
        ctx: Context<'_, S>,
    ) {
        let span = ctx.span(id).expect("span exists");
        values.record(&mut Visitor {
            span: span.name(),
            fields: &self.fields,
        });
    }
}

#[tokio::test]
async fn the_request_span_carries_the_tenant_id() {
    let _serial = serial().await;
    let Some((url, tenant_id)) = admitted_tenant(TenantStatus::Active).await else {
        return;
    };

    let fields: FieldLog = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(FieldCollector {
        fields: Arc::clone(&fields),
    });
    // Thread-local default: #[tokio::test] runs single-threaded, so every
    // span this request opens lands in the collector.
    let _guard = tracing::subscriber::set_default(subscriber);

    let response = router(state(&url))
        .oneshot(whoami_request(Some(&format!(
            "Bearer {}",
            issue(tenant_id)
        ))))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let fields = fields.lock().unwrap();
    let recorded = fields
        .iter()
        .find(|(span, field, _)| span == "http.request" && field == "tenant.id")
        .unwrap_or_else(|| panic!("no tenant.id on the http.request span; saw {fields:?}"));
    assert_eq!(recorded.2, tenant_id.to_string());
}
