//! HIER-2 at the product surface (ADR-0016): governed requests ride the
//! scope-chain cache — the resolution counter shows warm hits — and a
//! committed hierarchy move invalidates it, so the moved subtree's
//! effective policy is correct on the very next request. If invalidation
//! were missing, the cached pre-move chain would keep serving the old
//! division's pack assignment.
//!
//! Tests that need a live Postgres read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database); run them locally with
//! `make dev-up` then `make db-test`.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_policy::{Pdp, REGULATED_STRICT};
use synveda_store::{rls, role_bindings};
use synveda_types::{Role, TenantId};
use tower::ServiceExt;

const SECRET: &[u8] = b"hier-2-test-secret";
const SUBJECT: &str = "hier2-admin";

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
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp: Arc::new(Pdp::new().expect("build the embedded PDP")),
        scope_chains: Arc::new(synveda_store::ScopeChainCache::new()),
        service_token_max_ttl: std::time::Duration::from_secs(3600),
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

fn issue(tenant_id: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue(SUBJECT, tenant_id, Duration::from_secs(300))
}

async fn admitted_tenant(pool: &PgPool) -> TenantId {
    let id = TenantId::new();
    let slug = format!("hier2-{}", id.as_uuid().simple());
    synveda_store::tenants::create(
        pool,
        id,
        &slug,
        "HIER-2 scope chain test",
        synveda_types::TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    // The policy/hierarchy admin planes require a role (ADR-0015): seed a
    // tenant-wide org-admin binding for the dev test subject through the
    // store — the CLI's bootstrap path. Enforcement still runs through
    // the PDP with this row as data.
    let mut tx = rls::begin_tenant_tx(pool, id)
        .await
        .expect("begin tenant tx");
    role_bindings::bind(&mut *tx, id, SUBJECT, None, Role::OrgAdmin)
        .await
        .expect("bind admin");
    tx.commit().await.expect("commit binding");
    id
}

async fn api(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
    let request = match body {
        Some(body) => {
            builder = builder.header("content-type", "application/json");
            builder.body(Body::from(body.to_string()))
        }
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

async fn create_node(
    app: &Router,
    token: &str,
    parent: Option<&str>,
    kind: &str,
    slug: &str,
) -> String {
    let mut body = json!({"kind": kind, "slug": slug, "name": slug});
    if let Some(parent) = parent {
        body["parent_id"] = json!(parent);
    }
    let (status, node) = api(app, "POST", "/v1/hierarchy/nodes", token, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "create {slug}: {node}");
    node["id"].as_str().expect("node id").to_owned()
}

async fn effective_policy(app: &Router, token: &str, node: &str) -> Value {
    let (status, shown) = api(
        app,
        "GET",
        &format!("/v1/hierarchy/nodes/{node}/policy"),
        token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{shown}");
    shown
}

/// The HIER-2 AC end to end: warm requests hit the cache, and a node move
/// flips the moved team's effective pack on the very next request.
#[tokio::test]
async fn a_move_governs_the_very_next_request_through_the_cache() {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping scope chain routes test: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
            return;
        }
    };
    let state = state(&url);
    let pool = state.pool.clone();
    let handle = state.metrics.clone();
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");
    let tenant_id = admitted_tenant(&pool).await;
    let app = router(state);
    let token = issue(tenant_id);

    let org = create_node(&app, &token, None, "org", "acme").await;
    let emea = create_node(&app, &token, Some(&org), "division", "emea").await;
    let apac = create_node(&app, &token, Some(&org), "division", "apac").await;
    let team = create_node(&app, &token, Some(&emea), "team", "payments").await;

    // Assign `standard` at EMEA: the team inherits it through its chain.
    let (status, assigned) = api(
        &app,
        "PUT",
        &format!("/v1/hierarchy/nodes/{emea}/policy"),
        &token,
        Some(json!({"name": "standard"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{assigned}");

    let shown = effective_policy(&app, &token, &team).await;
    assert_eq!(shown["name"], "standard");
    assert_eq!(shown["origin"]["kind"], "assigned");
    assert_eq!(shown["origin"]["scope_id"], json!(emea));

    // Ask again: the chain is warm now — the hit counter proves these
    // requests resolve from memory, not from hierarchy rows.
    let shown = effective_policy(&app, &token, &team).await;
    assert_eq!(shown["name"], "standard");
    assert!(
        handle
            .render()
            .contains(r#"synveda_scope_chain_resolutions_total{outcome="hit"}"#),
        "warm requests must hit the scope-chain cache"
    );

    // Move the team to APAC. The mutation commits, the handler
    // invalidates, and the very next request sees the new chain: no
    // assignment anywhere on it, so the embedded default governs.
    let (status, moved) = api(
        &app,
        "PATCH",
        &format!("/v1/hierarchy/nodes/{team}"),
        &token,
        Some(json!({"parent_id": apac})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved}");

    let shown = effective_policy(&app, &token, &team).await;
    assert_eq!(
        shown["name"], REGULATED_STRICT,
        "the moved team must leave EMEA's pack behind on the very next request: {shown}"
    );
    assert_eq!(shown["origin"]["kind"], "default");
    assert!(
        handle
            .render()
            .contains("synveda_scope_chain_invalidations_total"),
        "the mutating handlers must have flushed the tenant's chains"
    );
}
