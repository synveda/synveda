//! HIER-1: CRUD via the admin API (`/v1/hierarchy/*`), behind tenant
//! resolution like every `/v1` route. The store-level contract (closure
//! consistency, the 10k-node AC) lives in
//! `crates/synveda-store/tests/hierarchy.rs`; this suite proves the HTTP
//! surface: status codes, taxonomy bodies, tenant scoping, and the
//! operations counter.
//!
//! Tests that need a live Postgres read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database); run them locally with
//! `make db-test`.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_types::TenantId;
use tower::ServiceExt;

const SECRET: &[u8] = b"hier-1-test-secret";

/// Serialises tests: the Prometheus recorder is process-global (same
/// rationale as tests/tenant_resolution.rs).
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

fn state(url: &str) -> AppState {
    AppState {
        pool: PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(2))
            .connect_lazy(url)
            .expect("parse database url"),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        pdp: Arc::new(synveda_policy::Pdp::new().expect("build the embedded PDP")),
    }
}

fn issue(tenant_id: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue("hier-admin", tenant_id, Duration::from_secs(300))
}

/// Connects, migrates, admits one tenant. `None` = no database configured;
/// the test skips quietly.
async fn admitted_tenant() -> Option<(String, TenantId)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping hierarchy admin test: DATABASE_URL is not set \
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
    let slug = format!("hieradm-{}", id.as_uuid().simple());
    synveda_store::tenants::create(
        &pool,
        id,
        &slug,
        "HIER-1 admin test",
        synveda_types::TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    Some((url, id))
}

/// One API call: returns (status, parsed JSON body — `Value::Null` when
/// empty, e.g. the delete 204).
async fn api(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let request = match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())),
        None => builder.body(Body::empty()),
    }
    .expect("build request");
    let response = app.clone().oneshot(request).await.expect("send request");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read response body")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("json body")
    };
    (status, value)
}

fn node_id(body: &Value) -> String {
    body["id"].as_str().expect("node id").to_owned()
}

fn ids(body: &Value) -> Vec<String> {
    body.as_array()
        .expect("array body")
        .iter()
        .map(node_id)
        .collect()
}

// ── No database needed ──────────────────────────────────────────────────────

#[tokio::test]
async fn hierarchy_routes_require_a_resolvable_tenant() {
    let _serial = serial().await;
    let app = router(state("postgres://nobody:nothing@127.0.0.1:1/void"));
    for (method, path) in [
        ("POST", "/v1/hierarchy/nodes"),
        ("GET", "/v1/hierarchy/root"),
        (
            "GET",
            "/v1/hierarchy/nodes/00000000-0000-7000-8000-000000000000",
        ),
        (
            "DELETE",
            "/v1/hierarchy/nodes/00000000-0000-7000-8000-000000000000",
        ),
    ] {
        let (status, body) = api(&app, method, path, None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}: {body}");
        assert_eq!(body["kind"], "unauthenticated", "{method} {path}: {body}");
    }
}

// ── Database-backed (skip without DATABASE_URL) ─────────────────────────────

#[tokio::test]
async fn crud_lifecycle_via_the_admin_api() {
    let _serial = serial().await;
    let Some((url, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state(&url));
    let token = issue(tenant_id);
    let token = Some(token.as_str());

    // Create: root, then a department, then a team under it.
    let (status, org) = api(
        &app,
        "POST",
        "/v1/hierarchy/nodes",
        token,
        Some(json!({"kind": "org", "slug": "acme", "name": "ACME"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{org}");
    assert_eq!(org["kind"], "org");
    assert_eq!(org["depth"], 0);
    assert_eq!(org["path"], "acme");
    assert_eq!(org["parent_id"], Value::Null);

    let (status, dept) = api(
        &app,
        "POST",
        "/v1/hierarchy/nodes",
        token,
        Some(json!({
            "parent_id": node_id(&org), "kind": "department",
            "slug": "payments", "name": "Payments"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{dept}");
    assert_eq!(dept["path"], "acme/payments");

    let (status, team) = api(
        &app,
        "POST",
        "/v1/hierarchy/nodes",
        token,
        Some(json!({
            "parent_id": node_id(&dept), "kind": "team",
            "slug": "core", "name": "Core"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{team}");

    // Read: node, root, and the three listings.
    let (status, body) = api(
        &app,
        "GET",
        &format!("/v1/hierarchy/nodes/{}", node_id(&team)),
        token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, team);

    let (status, body) = api(&app, "GET", "/v1/hierarchy/root", token, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(node_id(&body), node_id(&org));

    let (_, body) = api(
        &app,
        "GET",
        &format!("/v1/hierarchy/nodes/{}/children", node_id(&org)),
        token,
        None,
    )
    .await;
    assert_eq!(ids(&body), vec![node_id(&dept)]);

    let (_, body) = api(
        &app,
        "GET",
        &format!("/v1/hierarchy/nodes/{}/ancestors", node_id(&team)),
        token,
        None,
    )
    .await;
    assert_eq!(ids(&body), vec![node_id(&dept), node_id(&org)]);

    let (_, body) = api(
        &app,
        "GET",
        &format!("/v1/hierarchy/nodes/{}/descendants", node_id(&org)),
        token,
        None,
    )
    .await;
    assert_eq!(ids(&body), vec![node_id(&dept), node_id(&team)]);

    // Update: rename, then move under a second department.
    let (status, renamed) = api(
        &app,
        "PATCH",
        &format!("/v1/hierarchy/nodes/{}", node_id(&team)),
        token,
        Some(json!({"name": "Core Banking"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["name"], "Core Banking");
    assert_eq!(renamed["slug"], "core");
    assert_eq!(renamed["path"], "acme/payments/core");

    let (_, second) = api(
        &app,
        "POST",
        "/v1/hierarchy/nodes",
        token,
        Some(json!({
            "parent_id": node_id(&org), "kind": "department",
            "slug": "lending", "name": "Lending"
        })),
    )
    .await;
    let (status, moved) = api(
        &app,
        "PATCH",
        &format!("/v1/hierarchy/nodes/{}", node_id(&team)),
        token,
        Some(json!({"parent_id": node_id(&second)})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["parent_id"], second["id"]);
    assert_eq!(moved["path"], "acme/lending/core");

    // Delete: the team is a leaf; afterwards it is gone.
    let (status, _) = api(
        &app,
        "DELETE",
        &format!("/v1/hierarchy/nodes/{}", node_id(&team)),
        token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = api(
        &app,
        "GET",
        &format!("/v1/hierarchy/nodes/{}", node_id(&team)),
        token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["kind"], "not_found", "{body}");

    // The operations counter saw all of it.
    let exposition = metrics_handle().render();
    for op in [
        "create",
        "get",
        "root",
        "children",
        "ancestors",
        "descendants",
        "update",
        "delete",
    ] {
        assert!(
            exposition.lines().any(
                |line| line.starts_with("synveda_hierarchy_operations_total")
                    && line.contains(&format!("op=\"{op}\""))
                    && line.contains("outcome=\"ok\"")
            ),
            "op {op} missing from exposition:\n{exposition}"
        );
    }
}

#[tokio::test]
async fn invalid_requests_map_onto_the_taxonomy() {
    let _serial = serial().await;
    let Some((url, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state(&url));
    let token = issue(tenant_id);
    let token = Some(token.as_str());

    let (_, org) = api(
        &app,
        "POST",
        "/v1/hierarchy/nodes",
        token,
        Some(json!({"kind": "org", "slug": "acme", "name": "ACME"})),
    )
    .await;
    let (_, dept) = api(
        &app,
        "POST",
        "/v1/hierarchy/nodes",
        token,
        Some(json!({
            "parent_id": node_id(&org), "kind": "department",
            "slug": "payments", "name": "Payments"
        })),
    )
    .await;

    // (body, expected status, expected kind)
    let invalid_creates = [
        // Unknown kind never reaches the store.
        (
            json!({"kind": "organisation", "slug": "x", "name": "X"}),
            StatusCode::BAD_REQUEST,
            "invalid",
        ),
        // A non-org root.
        (
            json!({"kind": "team", "slug": "loose", "name": "Loose"}),
            StatusCode::BAD_REQUEST,
            "invalid",
        ),
        // Malformed slug (CHECK constraint).
        (
            json!({
                "parent_id": node_id(&org), "kind": "team",
                "slug": "Not A Slug!", "name": "X"
            }),
            StatusCode::BAD_REQUEST,
            "invalid",
        ),
        // Duplicate sibling slug.
        (
            json!({
                "parent_id": node_id(&org), "kind": "department",
                "slug": "payments", "name": "Payments again"
            }),
            StatusCode::CONFLICT,
            "conflict",
        ),
        // Second root.
        (
            json!({"kind": "org", "slug": "acme-two", "name": "ACME 2"}),
            StatusCode::CONFLICT,
            "conflict",
        ),
    ];
    for (body, expected_status, expected_kind) in invalid_creates {
        let (status, response) = api(&app, "POST", "/v1/hierarchy/nodes", token, Some(body)).await;
        assert_eq!(status, expected_status, "{response}");
        assert_eq!(response["kind"], expected_kind, "{response}");
    }

    // Empty patch, and deleting a non-leaf.
    let (status, response) = api(
        &app,
        "PATCH",
        &format!("/v1/hierarchy/nodes/{}", node_id(&dept)),
        token,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
    assert_eq!(response["kind"], "invalid", "{response}");

    let (status, response) = api(
        &app,
        "DELETE",
        &format!("/v1/hierarchy/nodes/{}", node_id(&org)),
        token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{response}");
    assert_eq!(response["kind"], "conflict", "{response}");
}

/// Another tenant's nodes are indistinguishable from missing ones — reads
/// and mutations alike — even on connections where the RLS backstop does
/// not bite (the handlers' own tenant check).
#[tokio::test]
async fn foreign_nodes_are_uniformly_not_found() {
    let _serial = serial().await;
    let Some((url, tenant_a)) = admitted_tenant().await else {
        return;
    };
    let Some((_, tenant_b)) = admitted_tenant().await else {
        return;
    };
    let app = router(state(&url));
    let token_a = issue(tenant_a);
    let token_b = issue(tenant_b);

    let (status, org_b) = api(
        &app,
        "POST",
        "/v1/hierarchy/nodes",
        Some(token_b.as_str()),
        Some(json!({"kind": "org", "slug": "acme-b", "name": "ACME B"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{org_b}");

    let path = format!("/v1/hierarchy/nodes/{}", node_id(&org_b));
    for (method, body) in [
        ("GET", None),
        ("PATCH", Some(json!({"name": "Hijacked"}))),
        ("DELETE", None),
    ] {
        let (status, response) = api(&app, method, &path, Some(token_a.as_str()), body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method}: {response}");
        assert_eq!(response["kind"], "not_found", "{method}: {response}");
    }

    // And tenant B still sees its node untouched.
    let (status, body) = api(&app, "GET", &path, Some(token_b.as_str()), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "ACME B", "{body}");
}
