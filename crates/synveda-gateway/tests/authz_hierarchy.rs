//! AUTHZ-1/AUTHZ-2: the PDP gates `/v1/hierarchy/*` (ADR-0012 decision 7)
//! under the resource's *effective* pack (ADR-0014): stored custom packs
//! hot-swap through the reload path, and what governs a request is the
//! tenant default (or an assignment) — request-time data, in force on the
//! next request. Restrictive behaviour comes from a *test policy pack*
//! applied through the same store + reload + assignment paths the product
//! uses — never a PDP bypass (CLAUDE.md, seed §2.2).
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
use synveda_gateway::{authz, telemetry};
use synveda_identity::Hs256Verifier;
use synveda_policy::{Pdp, REGULATED_STRICT};
use synveda_store::{policy_assignments, policy_packs, rls, role_bindings};
use synveda_types::{Role, TenantId};
use tower::ServiceExt;

const SECRET: &[u8] = b"authz-1-test-secret";

/// Only permits hierarchy reads: mutations — and the policy admin plane —
/// fall to Cedar's default-deny.
const READ_ONLY_PACK: &str = r#"
permit (
    principal,
    action == Synveda::Action::"HierarchyRead",
    resource
) when { resource in principal.tenant };
"#;

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
    Hs256Verifier::new(SECRET).issue("authz-admin", tenant_id, Duration::from_secs(300))
}

/// Connects, migrates, admits one tenant. `None` = no database configured;
/// the test skips quietly.
async fn admitted_tenant() -> Option<(String, TenantId)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping authz hierarchy test: DATABASE_URL is not set \
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
    let slug = format!("authz-{}", id.as_uuid().simple());
    synveda_store::tenants::create(
        &pool,
        id,
        &slug,
        "AUTHZ gateway test",
        synveda_types::TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    Some((url, id))
}

/// One API call: returns (status, parsed JSON body — `Value::Null` when
/// empty).
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

async fn store_pack(pool: &PgPool, tenant: TenantId, name: &str, source: &str) {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant tx");
    policy_packs::apply(&mut *tx, tenant, name, source, None, None)
        .await
        .expect("store pack");
    tx.commit().await.expect("commit pack");
}

/// Seeds the tenant's admin: a tenant-wide org-admin binding for the dev
/// test subject, at the store level — the CLI's bootstrap/break-glass
/// path (ADR-0015 decision 6). Since AUTHZ-3 an unbound dev subject holds
/// no administrative power; enforcement still runs through the PDP with
/// this row as data — never a bypass.
async fn bind_admin(pool: &PgPool, tenant: TenantId, subject: &str) {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant tx");
    role_bindings::bind(&mut *tx, tenant, subject, None, Role::OrgAdmin)
        .await
        .expect("bind admin");
    tx.commit().await.expect("commit binding");
}

async fn clear_pack(pool: &PgPool, tenant: TenantId, name: &str) {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant tx");
    policy_packs::clear(&mut tx, tenant, name)
        .await
        .expect("clear pack");
    tx.commit().await.expect("commit clear");
}

/// The store-level break-glass (ADR-0014): a tenant default naming a pack
/// that permits no policy admin locks the `/v1/policy` plane; the CLI's
/// tenant-tx store path clears it.
async fn break_glass_clear_default(pool: &PgPool, tenant: TenantId) {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant tx");
    policy_assignments::clear_default(&mut *tx, tenant)
        .await
        .expect("clear default");
    tx.commit().await.expect("commit clear default");
}

fn node_id(body: &Value) -> String {
    body["id"].as_str().expect("node id").to_owned()
}

/// The headline flow: the embedded default (`regulated-strict`) admits the
/// tenant's own admin; a stored read-only pack hot-reloads in and — once
/// made the tenant default through the product route — denies mutations
/// (naming pack@version in the denial) on the next request, while reads
/// keep working; clearing the default restores the embedded default.
#[tokio::test]
async fn stored_packs_gate_the_admin_plane_and_hot_reload() {
    let _serial = serial().await;
    let Some((url, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let state = state(&url);
    let pdp = Arc::clone(&state.pdp);
    let pool = state.pool.clone();
    let app = router(state);
    let token = issue(tenant_id);
    bind_admin(&pool, tenant_id, "authz-admin").await;

    // Under the embedded default: create the org and a department (the
    // admin semantics ADR-0014 carried, now held by the org-admin role —
    // ADR-0015 decision 4).
    let (status, org) = api(
        &app,
        "POST",
        "/v1/hierarchy/nodes",
        &token,
        Some(json!({"kind": "org", "slug": "acme", "name": "ACME"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{org}");
    let (status, dept) = api(
        &app,
        "POST",
        "/v1/hierarchy/nodes",
        &token,
        Some(json!({
            "parent_id": node_id(&org), "kind": "department",
            "slug": "payments", "name": "Payments"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{dept}");

    // Store the read-only pack and reload — the source distribution path.
    store_pack(&pool, tenant_id, "authz1-readonly", READ_ONLY_PACK).await;
    assert_eq!(
        authz::refresh_tenant_packs(&pool, &pdp, tenant_id).await,
        "installed"
    );
    assert_eq!(
        authz::refresh_tenant_packs(&pool, &pdp, tenant_id).await,
        "unchanged",
        "an unchanged version must be skipped"
    );

    // A compiled-but-unassigned pack governs nothing: mutations still work.
    let (status, probe) = api(
        &app,
        "POST",
        "/v1/hierarchy/nodes",
        &token,
        Some(json!({
            "parent_id": node_id(&org), "kind": "team",
            "slug": "probe", "name": "Probe"
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "an unassigned pack must not govern: {probe}"
    );

    // Make it the tenant default through the product route: in force on
    // the very next request, no reload involved (ADR-0014 decision 3).
    let (status, set) = api(
        &app,
        "PUT",
        "/v1/policy/default",
        &token,
        Some(json!({"name": "authz1-readonly"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{set}");

    // Mutations are denied 403 with the pack version in the denial reason.
    for (method, path, body) in [
        (
            "POST",
            "/v1/hierarchy/nodes".to_owned(),
            Some(json!({
                "parent_id": node_id(&org), "kind": "team",
                "slug": "core", "name": "Core"
            })),
        ),
        (
            "PATCH",
            format!("/v1/hierarchy/nodes/{}", node_id(&dept)),
            Some(json!({"name": "Renamed"})),
        ),
        (
            "DELETE",
            format!("/v1/hierarchy/nodes/{}", node_id(&dept)),
            None,
        ),
    ] {
        let (status, response) = api(&app, method, &path, &token, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {response}");
        assert_eq!(response["kind"], "policy_denied", "{response}");
        let reason = response["reason"].as_str().expect("reason");
        assert!(
            reason.contains("authz1-readonly@1"),
            "the denial must name pack@version, got: {reason}"
        );
    }

    // Reads keep working under the same pack.
    for path in [
        "/v1/hierarchy/root".to_owned(),
        format!("/v1/hierarchy/nodes/{}", node_id(&dept)),
        format!("/v1/hierarchy/nodes/{}/ancestors", node_id(&dept)),
    ] {
        let (status, response) = api(&app, "GET", &path, &token, None).await;
        assert_eq!(status, StatusCode::OK, "GET {path}: {response}");
    }

    // Decisions are visible in the metric contract, allow and deny alike.
    let exposition = metrics_handle().render();
    for (decision, pack) in [
        ("allow", REGULATED_STRICT),
        ("deny", "authz1-readonly"),
        ("allow", "authz1-readonly"),
    ] {
        assert!(
            exposition
                .lines()
                .any(|line| line.starts_with("synveda_authz_decisions_total")
                    && line.contains(&format!("decision=\"{decision}\""))
                    && line.contains(&format!("pack=\"{pack}\""))),
            "decision {decision}/{pack} missing from exposition:\n{exposition}"
        );
    }

    // The read-only pack permits no PolicyAssign, so the product route to
    // undo the default is itself denied — the lockout ADR-0014 documents.
    let (status, locked) = api(&app, "DELETE", "/v1/policy/default", &token, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{locked}");

    // Break-glass at the store level, then mutations work again on the
    // next request — back on the embedded default.
    break_glass_clear_default(&pool, tenant_id).await;
    let (status, team) = api(
        &app,
        "POST",
        "/v1/hierarchy/nodes",
        &token,
        Some(json!({
            "parent_id": node_id(&org), "kind": "team",
            "slug": "core", "name": "Core"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{team}");

    // With no references left, the stored pack clears and the reloader
    // drops the compiled copy.
    clear_pack(&pool, tenant_id, "authz1-readonly").await;
    assert_eq!(
        authz::refresh_tenant_packs(&pool, &pdp, tenant_id).await,
        "removed"
    );
}

/// A stored pack that does not compile must not change enforcement: the
/// reload records an error and enforcement stays on the last-good state
/// (ADR-0012 decision 5).
#[tokio::test]
async fn an_invalid_stored_pack_keeps_the_last_good_state() {
    let _serial = serial().await;
    let Some((url, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let state = state(&url);
    let pdp = Arc::clone(&state.pdp);
    let pool = state.pool.clone();
    let app = router(state);
    let token = issue(tenant_id);
    bind_admin(&pool, tenant_id, "authz-admin").await;

    // The store accepts what the CLI's compile check would refuse — the
    // reloader is the enforcement boundary for out-of-band writes.
    store_pack(&pool, tenant_id, "authz1-broken", "permit (principal").await;
    assert_eq!(
        authz::refresh_tenant_packs(&pool, &pdp, tenant_id).await,
        "error"
    );

    // The embedded default (the last-good state) still decides: admin
    // works.
    let (status, org) = api(
        &app,
        "POST",
        "/v1/hierarchy/nodes",
        &token,
        Some(json!({"kind": "org", "slug": "acme", "name": "ACME"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{org}");

    let exposition = metrics_handle().render();
    assert!(
        exposition
            .lines()
            .any(|line| line.starts_with("synveda_policy_pack_reloads_total")
                && line.contains("outcome=\"error\"")),
        "reload error missing from exposition:\n{exposition}"
    );
    clear_pack(&pool, tenant_id, "authz1-broken").await;
}
