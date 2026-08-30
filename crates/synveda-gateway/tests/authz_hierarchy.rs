//! AUTHZ-1/AUTHZ-2: the PDP gates `/v1/admin/scopes` (ADR-0012 decision 7)
//! under the resource's *effective* pack (ADR-0014): stored custom packs
//! hot-swap through the reload path, and what governs a request is the
//! tenant default (or an assignment) — request-time data, in force on the
//! next request. Restrictive behaviour comes from a *test policy pack*
//! applied through the same store + reload + assignment paths the product
//! uses — never a PDP bypass.
//!
//! Tests that need a live Postgres read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database); run them locally with
//! `make db-test`.

#[path = "../../synveda-store/tests/support/tenant_fixture.rs"]
mod tenant_fixture;

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
use synveda_gateway::app::{AppState, behavior_test_router as router};
use synveda_gateway::{authz, telemetry};
use synveda_identity::Hs256Verifier;
use synveda_policy::{Pdp, REGULATED_STRICT};
use synveda_store::{access, policy_packs, rls, scopes};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::{GrantId, PackConfig, TenantId};
use tower::ServiceExt;

#[path = "support/configuration.rs"]
mod configuration_support;

const SECRET: &[u8] = b"authz-1-test-secret";

/// Only permits scope reads: mutations — and the policy admin plane —
/// fall to Cedar's default-deny.
const READ_ONLY_PACK: &str = r#"
permit (
    principal,
    action == Synveda::Action::"ScopeRead",
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
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp: Arc::new(Pdp::new().expect("build the embedded PDP")),
        service_token_max_ttl: std::time::Duration::from_secs(3600),
        embedder: Arc::new(synveda_ingest::embedding::AnyEmbedder::Deterministic(
            synveda_ingest::embedding::DeterministicEmbedder::new(),
        )),
        context_embed_timeout: std::time::Duration::from_millis(100),
        // TEN-4 (ADR-0064): a fixed test KEK, so a suite that touches a
        // sealed column seals rather than skipping. `Kms::Disabled` is the
        // production default when no key is configured.
        keys: std::sync::Arc::new(synveda_store::keys::KeyRing::new(
            synveda_crypto::Kms::Local(
                synveda_crypto::LocalKms::from_hex(&"11".repeat(32), "local:test")
                    .expect("test kek"),
            ),
        )),
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
    synveda_store::epoch::verify(&pool)
        .await
        .expect("apply migrations");
    let id = TenantId::new();
    let slug = format!("authz-{}", id.as_uuid().simple());
    tenant_fixture::create(
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
    key: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
    if let Some(key) = key {
        builder = builder.header("idempotency-key", key);
    }
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
    policy_packs::apply(&mut *tx, tenant, name, source, &PackConfig::default())
        .await
        .expect("store pack");
    tx.commit().await.expect("commit pack");
}

/// Seeds the tenant's admin: an `administrator` grant at the tenant root
/// for the dev test subject, at the store level — the CLI's
/// bootstrap/break-glass path (ADR-0015 decision 6). Since AUTHZ-3 an
/// ungranted dev subject holds no administrative power; enforcement still
/// runs through the PDP with this row as data — never a bypass.
async fn bind_admin(pool: &PgPool, tenant: TenantId, subject: &str) {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant tx");
    let root = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("mint root");
    access::create_grant(
        &mut tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id: tenant,
            scope_id: root.id,
            subject: GrantSubject::Principal {
                principal_id: subject.to_owned(),
            },
            role_key: RoleKey::Administrator,
            source: GrantSource::Automation,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .expect("grant administrator");
    tx.commit().await.expect("commit grant");
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

/// Test-only stand-in for the documented local operator break-glass: disable
/// the revisioned tenant-root Configuration binding. The public application
/// plane has no mutation path around its own effective PDP.
async fn break_glass_disable_configuration(pool: &PgPool, tenant: TenantId) {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant tx");
    assert!(configuration_support::disable_tenant(&mut tx, tenant).await);
    tx.commit().await.expect("commit Configuration disable");
}

fn node_id(body: &Value) -> String {
    body["id"].as_str().expect("scope id").to_owned()
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

    // Under the embedded default: read the tenant root, then create an
    // org unit and a workspace under it (the admin semantics ADR-0014
    // carried, now held by the administrator grant — ADR-0015 decision 4).
    let (status, level) = api(&app, "GET", "/v1/admin/scopes", &token, None, None).await;
    assert_eq!(status, StatusCode::OK, "{level}");
    let org = level["parent"]["id"].as_str().expect("root id").to_owned();
    let (status, dept) = api(
        &app,
        "POST",
        "/v1/admin/scopes",
        &token,
        Some("authz1-create-eng"),
        Some(json!({
            "parent_id": org, "kind": "org_unit",
            "slug": "eng", "display_name": "Engineering"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{dept}");
    let (status, ws) = api(
        &app,
        "POST",
        "/v1/admin/scopes",
        &token,
        Some("authz1-create-payments"),
        Some(json!({
            "parent_id": node_id(&dept), "kind": "workspace",
            "slug": "payments", "display_name": "Payments"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{ws}");

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
        "/v1/admin/scopes",
        &token,
        Some("authz1-create-probe"),
        Some(json!({
            "parent_id": org, "kind": "org_unit",
            "slug": "probe", "display_name": "Probe"
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "an unassigned pack must not govern: {probe}"
    );

    // The old mutable default route is gone. The governed fixture creates a
    // typed VedaFlow change, immutable Configuration version and root
    // binding; that selection is in force on the next request.
    let (status, old_route) = api(
        &app,
        "PUT",
        "/v1/policy/default",
        &token,
        None,
        Some(json!({"name": "authz1-readonly"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{old_route}");
    let mut tx = rls::begin_tenant_tx(&pool, tenant_id)
        .await
        .expect("begin governed selection");
    configuration_support::bind_tenant_pack(&mut tx, tenant_id, "authz1-readonly").await;
    tx.commit().await.expect("commit governed selection");

    // Mutations are denied 403 with the pack version in the denial
    // reason: a create, a rename and an archive — there is no scope
    // delete to name, retiring is a status transition under ScopeUpdate.
    let dept = node_id(&dept);
    for (method, path, body) in [
        (
            "POST",
            "/v1/admin/scopes".to_owned(),
            Some(json!({
                "parent_id": org, "kind": "org_unit",
                "slug": "core", "display_name": "Core"
            })),
        ),
        (
            "PATCH",
            format!("/v1/admin/scopes/{dept}"),
            Some(json!({"display_name": "Renamed"})),
        ),
        (
            "PATCH",
            format!("/v1/admin/scopes/{dept}"),
            Some(json!({"status": "archived"})),
        ),
    ] {
        let (status, response) =
            api(&app, method, &path, &token, Some("authz1-deny-probe"), body).await;
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
        "/v1/admin/scopes".to_owned(),
        format!("/v1/admin/scopes/{dept}"),
        format!("/v1/admin/scopes/{dept}/ancestors"),
    ] {
        let (status, response) = api(&app, "GET", &path, &token, None, None).await;
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

    // The deleted route stays absent even while the selected pack permits no
    // ConfigurationWrite. No compatibility mutation path reappears.
    let (status, locked) = api(&app, "DELETE", "/v1/policy/default", &token, None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{locked}");

    // Local operator break-glass disables the binding; the immutable version
    // and its policy-pack reference remain. Mutations work again under the
    // strict fail-safe.
    break_glass_disable_configuration(&pool, tenant_id).await;
    let (status, team) = api(
        &app,
        "POST",
        "/v1/admin/scopes",
        &token,
        Some("authz1-create-core"),
        Some(json!({
            "parent_id": org, "kind": "org_unit",
            "slug": "core", "display_name": "Core"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{team}");

    let mut tx = rls::begin_tenant_tx(&pool, tenant_id)
        .await
        .expect("begin immutable-reference check");
    let retained = policy_packs::clear(&mut tx, tenant_id, "authz1-readonly").await;
    assert!(
        matches!(retained, Err(synveda_types::Error::Conflict { .. })),
        "immutable Configuration history must retain the pack: {retained:?}"
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
    let (status, level) = api(&app, "GET", "/v1/admin/scopes", &token, None, None).await;
    assert_eq!(status, StatusCode::OK, "{level}");
    let root = level["parent"]["id"].as_str().expect("root id").to_owned();
    let (status, org) = api(
        &app,
        "POST",
        "/v1/admin/scopes",
        &token,
        Some("authz1-create-acme"),
        Some(json!({
            "parent_id": root, "kind": "org_unit",
            "slug": "acme", "display_name": "ACME"
        })),
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
