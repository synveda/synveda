//! AUTHZ-3 at the product surface (ADR-0015 decision 7): tenant-wide and
//! per-node role bindings over `/v1/roles/bindings` and
//! `/v1/hierarchy/nodes/{id}/roles`, and the headline behaviour — a
//! binding (or its revocation) is in force on the very next request,
//! inherited over the bound subtree; delegation works; the escalation
//! guard holds end to end; cross-tenant probes see uniform 404. The full
//! role×action matrix lives in `crates/synveda-policy/tests/roles.rs`.
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
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_policy::Pdp;
use synveda_store::{rls, role_bindings};
use synveda_types::{Role, TenantId};
use tower::ServiceExt;

const SECRET: &[u8] = b"authz-3-test-secret";

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

fn issue(subject: &str, tenant_id: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue(subject, tenant_id, Duration::from_secs(300))
}

/// Admits a tenant and seeds its bootstrap admin: a tenant-wide org-admin
/// binding for `role-admin` through the store — the CLI's path (ADR-0015
/// decision 6).
async fn admitted_tenant(pool: &PgPool, label: &str) -> TenantId {
    let id = TenantId::new();
    let slug = format!("{label}-{}", id.as_uuid().simple());
    synveda_store::tenants::create(
        pool,
        id,
        &slug,
        "AUTHZ-3 roles routes test",
        synveda_types::TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    let mut tx = rls::begin_tenant_tx(pool, id)
        .await
        .expect("begin tenant tx");
    role_bindings::bind(&mut *tx, id, "role-admin", None, Role::OrgAdmin)
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

fn node_id(body: &Value) -> String {
    body["id"].as_str().expect("node id").to_owned()
}

async fn create_node(
    app: &Router,
    token: &str,
    parent: Option<&str>,
    kind: &str,
    slug: &str,
) -> Value {
    let mut body = json!({"kind": kind, "slug": slug, "name": slug});
    if let Some(parent) = parent {
        body["parent_id"] = json!(parent);
    }
    let (status, node) = api(app, "POST", "/v1/hierarchy/nodes", token, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "create {slug}: {node}");
    node
}

/// The headline flow: bootstrap admin → node-scoped steward → subtree
/// inheritance and delegation on the very next request → the escalation
/// guard and the tenant-plane boundary → revocation in force on the next
/// request.
#[tokio::test]
async fn bindings_govern_the_subtree_from_the_next_request() {
    let _serial = serial().await;
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping roles routes test: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
            return;
        }
    };
    let state = state(&url);
    let pool = state.pool.clone();
    synveda_store::migrate(&pool).await.expect("migrate");
    let tenant_id = admitted_tenant(&pool, "authz3").await;
    let app = router(state);
    let admin = issue("role-admin", tenant_id);
    let steward = issue("stew", tenant_id);

    // The bootstrap admin builds the tree: org → {eng → core, ops}.
    let org = create_node(&app, &admin, None, "org", "acme").await;
    let eng = create_node(&app, &admin, Some(&node_id(&org)), "department", "eng").await;
    let ops = create_node(&app, &admin, Some(&node_id(&org)), "department", "ops").await;
    let core = create_node(&app, &admin, Some(&node_id(&eng)), "team", "core").await;

    // The tenant listing shows the bootstrap binding.
    let (status, listing) = api(&app, "GET", "/v1/roles/bindings", &admin, None).await;
    assert_eq!(status, StatusCode::OK, "{listing}");
    let bindings = listing["bindings"].as_array().expect("bindings");
    assert!(
        bindings.iter().any(|b| b["subject"] == "role-admin"
            && b["role"] == "org-admin"
            && b["scope_id"].is_null()),
        "the tenant-wide admin binding must be listed: {listing}"
    );

    // An unbound subject holds nothing: the listing itself is denied.
    let nobody = issue("nobody", tenant_id);
    let (status, denied) = api(&app, "GET", "/v1/roles/bindings", &nobody, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    assert_eq!(denied["kind"], "policy_denied", "{denied}");

    // Before the binding: the steward-to-be cannot touch anything.
    let rename = json!({"name": "Core Renamed"});
    let path_core = format!("/v1/hierarchy/nodes/{}", node_id(&core));
    let (status, _) = api(&app, "PATCH", &path_core, &steward, Some(rename.clone())).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "unbound stew must be denied");

    // The admin binds stew as steward at eng.
    let path_eng_roles = format!("/v1/hierarchy/nodes/{}/roles", node_id(&eng));
    let (status, bound) = api(
        &app,
        "PUT",
        &path_eng_roles,
        &admin,
        Some(json!({"subject": "stew", "role": "steward"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{bound}");
    assert_eq!(bound["role"], "steward", "{bound}");

    // In force on the very next request, inherited over the subtree.
    let (status, renamed) = api(&app, "PATCH", &path_core, &steward, Some(rename)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the eng steward governs core on the next request: {renamed}"
    );

    // ...but not outside the bound subtree.
    let path_ops = format!("/v1/hierarchy/nodes/{}", node_id(&ops));
    let (status, outside) = api(
        &app,
        "PATCH",
        &path_ops,
        &steward,
        Some(json!({"name": "Ops"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the eng binding must not reach ops: {outside}"
    );

    // Delegation downward: the steward binds a viewer in their subtree.
    let (status, delegated) = api(
        &app,
        "PUT",
        &path_eng_roles,
        &steward,
        Some(json!({"subject": "viewer-1", "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{delegated}");

    // The escalation guard, end to end: a steward cannot mint org-admin.
    let (status, escalation) = api(
        &app,
        "PUT",
        &path_eng_roles,
        &steward,
        Some(json!({"subject": "mallory", "role": "org-admin"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{escalation}");
    assert_eq!(escalation["kind"], "policy_denied", "{escalation}");

    // The tenant plane needs a tenant-wide role: the node steward is out.
    let (status, plane) = api(
        &app,
        "PUT",
        "/v1/roles/bindings",
        &steward,
        Some(json!({"subject": "x", "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{plane}");

    // The steward reads their subtree's bindings.
    let (status, at_eng) = api(&app, "GET", &path_eng_roles, &steward, None).await;
    assert_eq!(status, StatusCode::OK, "{at_eng}");
    let subjects: Vec<&str> = at_eng["bindings"]
        .as_array()
        .expect("bindings")
        .iter()
        .map(|b| b["subject"].as_str().expect("subject"))
        .collect();
    assert_eq!(subjects, ["stew", "viewer-1"], "{at_eng}");

    // An unknown role never reaches the store or the PDP.
    let (status, unknown) = api(
        &app,
        "PUT",
        &path_eng_roles,
        &admin,
        Some(json!({"subject": "x", "role": "superuser"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{unknown}");

    // Revocation is in force on the next request too.
    let (status, revoked) = api(
        &app,
        "DELETE",
        &format!("{path_eng_roles}?subject=stew&role=steward"),
        &admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{revoked}");
    let (status, after) = api(
        &app,
        "PATCH",
        &path_core,
        &steward,
        Some(json!({"name": "Core Again"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the revoked steward is out on the next request: {after}"
    );

    // Deleting it again: 404, not a silent no-op.
    let (status, gone) = api(
        &app,
        "DELETE",
        &format!("{path_eng_roles}?subject=stew&role=steward"),
        &admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{gone}");

    // The metric contract.
    let exposition = metrics_handle().render();
    for op in ["list", "bind_node", "unbind_node"] {
        assert!(
            exposition
                .lines()
                .any(|line| line.starts_with("synveda_role_operations_total")
                    && line.contains(&format!("op=\"{op}\""))),
            "role op {op} missing from exposition:\n{exposition}"
        );
    }
}

/// Cross-tenant probes see uniform 404 on the node-roles routes — never a
/// policy-denial oracle (ADR-0012 decision 7).
#[tokio::test]
async fn cross_tenant_role_probes_see_uniform_404() {
    let _serial = serial().await;
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("skipping roles routes test: DATABASE_URL is not set");
            return;
        }
    };
    let state = state(&url);
    let pool = state.pool.clone();
    synveda_store::migrate(&pool).await.expect("migrate");
    let victim = admitted_tenant(&pool, "authz3v").await;
    let intruder = admitted_tenant(&pool, "authz3i").await;
    let app = router(state);

    let victim_admin = issue("role-admin", victim);
    let org = create_node(&app, &victim_admin, None, "org", "acme-v").await;
    let path = format!("/v1/hierarchy/nodes/{}/roles", node_id(&org));

    let intruder_admin = issue("role-admin", intruder);
    for (method, body) in [
        ("GET", None),
        ("PUT", Some(json!({"subject": "x", "role": "viewer"}))),
    ] {
        let (status, response) = api(&app, method, &path, &intruder_admin, body).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{method} {path} must be a uniform 404 cross-tenant: {response}"
        );
    }
    let (status, response) = api(
        &app,
        "DELETE",
        &format!("{path}?subject=role-admin&role=org-admin"),
        &intruder_admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{response}");
}
