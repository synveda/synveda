//! AUTHZ-1/AUTHZ-2: the PDP gates `/v1/admin/scopes` (ADR-0012 decision 7)
//! under the resource's *effective* pack (ADR-0014). Each request compiles the
//! tenant's current stored packs (ADR-0145/0146), so a committed pack or
//! selection governs the next request; the process refresher only maintains
//! a cache and readiness. Restrictive behaviour comes from a *test policy
//! pack* stored and selected through the product paths — never a PDP bypass.
//!
//! These cases need the exact-role database fixture: run `make db-test`.

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
use synveda_audit::{Actor, AuditAction, AuditEvent, ChainVerification, Outcome, StoredEvent};
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

/// Accepted by the store, refused by every compile.
const MALFORMED_PACK: &str = "permit (principal";

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
        git_sources: None,
        git_exports: None,
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

/// Connects, migrates, admits one tenant. `None` is an intentional local
/// unit-only run; required database acceptance fails instead.
async fn admitted_tenant() -> Option<(String, TenantId)> {
    let url = tenant_fixture::database_url("authz hierarchy test")?;
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

/// An out-of-band stored-pack write: no compile check and no audit, so it can
/// commit what the operator path would refuse. Returns the stored version.
async fn store_pack(pool: &PgPool, tenant: TenantId, name: &str, source: &str) -> i64 {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant tx");
    let pack = policy_packs::apply(&mut *tx, tenant, name, source, &PackConfig::default())
        .await
        .expect("store pack");
    tx.commit().await.expect("commit pack");
    pack.version
}

/// The writes `synveda policy apply` makes after its compile check: a
/// version-bumping replacement and a break-glass audit event, under the
/// exclusive admission gate.
async fn operator_apply(pool: &PgPool, tenant: TenantId, name: &str, source: &str) -> i64 {
    Pdp::new()
        .expect("build the embedded PDP")
        .compile_check(name, source)
        .expect("the operator path accepts only a compiling pack");
    let mut tx = rls::begin_tenant_write_tx(pool, tenant)
        .await
        .expect("begin operator tx");
    let pack = policy_packs::apply(&mut *tx, tenant, name, source, &PackConfig::default())
        .await
        .expect("store repaired pack");
    let payload = json!({"pack": name, "version": pack.version});
    operator_audit(&mut tx, tenant, AuditAction::PolicyPackApplied, payload).await;
    tx.commit().await.expect("commit repair");
    pack.version
}

/// The writes `synveda policy clear` makes for an unreferenced pack.
async fn operator_clear(pool: &PgPool, tenant: TenantId, name: &str) {
    let mut tx = rls::begin_tenant_write_tx(pool, tenant)
        .await
        .expect("begin operator tx");
    assert!(
        policy_packs::clear(&mut tx, tenant, name)
            .await
            .expect("clear pack"),
        "the pack to clear exists"
    );
    let payload = json!({"pack": name});
    operator_audit(&mut tx, tenant, AuditAction::PolicyPackCleared, payload).await;
    tx.commit().await.expect("commit clear");
}

async fn operator_audit(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    action: AuditAction,
    payload: Value,
) {
    synveda_audit::append(
        tx,
        tenant,
        &AuditEvent {
            occurred_at: chrono::Utc::now(),
            actor: Actor::break_glass("authz-test-operator"),
            action,
            resource: format!("tenant {tenant}"),
            outcome: Outcome::Success,
            payload,
            trace_id: None,
        },
    )
    .await
    .expect("append break-glass audit");
}

/// The tenant's chain, oldest first, and its verification.
async fn audit_chain(pool: &PgPool, tenant: TenantId) -> (Vec<StoredEvent>, ChainVerification) {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant tx");
    let mut events = synveda_audit::tail(&mut tx, tenant, 500)
        .await
        .expect("read chain");
    events.reverse();
    let verification = synveda_audit::verify(&mut tx, tenant)
        .await
        .expect("verify chain");
    tx.rollback().await.expect("end read transaction");
    (events, verification)
}

/// The public refusal for absent current authority: stable kind/status, no
/// policy source or name, and no replay remedy.
fn assert_policy_unavailable(status: StatusCode, body: &Value, packs: &[&str]) {
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["kind"], "conflict", "{body}");
    let rendered = body.to_string();
    for disclosed in ["permit", "idempotency", "Saved", "compose"]
        .iter()
        .chain(packs)
    {
        assert!(!rendered.contains(disclosed), "{disclosed}: {rendered}");
    }
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

/// A malformed replacement cannot overwrite the process's valid compile
/// (ADR-0012 decision 5). That compile is cache state, not request authority.
#[tokio::test]
async fn a_failed_refresh_keeps_the_valid_compile_in_the_process_cache() {
    let _serial = serial().await;
    let Some((url, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let state = state(&url);
    let pdp = Arc::clone(&state.pdp);
    let pool = state.pool.clone();

    store_pack(&pool, tenant_id, "authz1-cached", READ_ONLY_PACK).await;
    assert_eq!(
        authz::refresh_tenant_packs(&pool, &pdp, tenant_id).await,
        "installed"
    );
    assert_eq!(
        store_pack(&pool, tenant_id, "authz1-cached", MALFORMED_PACK).await,
        2
    );
    assert_eq!(
        authz::refresh_tenant_packs(&pool, &pdp, tenant_id).await,
        "error"
    );
    assert_eq!(
        pdp.installed_versions(tenant_id),
        vec![("authz1-cached".to_owned(), 1)],
        "a failed compile must not replace or drop the cached pack"
    );
    let exposition = metrics_handle().render();
    assert!(
        exposition
            .lines()
            .any(|line| line.starts_with("synveda_policy_pack_reloads_total")
                && line.contains("outcome=\"error\"")),
        "reload error missing from exposition:\n{exposition}"
    );

    operator_clear(&pool, tenant_id, "authz1-cached").await;
    assert_eq!(
        authz::refresh_tenant_packs(&pool, &pdp, tenant_id).await,
        "removed"
    );
    assert!(pdp.installed_versions(tenant_id).is_empty());
}

/// Admission compiles the tenant's current stored packs, so malformed current
/// policy refuses every governed request, even while the process cache holds
/// a valid compile under the same name and version. Neither a retry nor a new
/// idempotency key helps; an operator repair does.
#[tokio::test]
async fn malformed_current_policy_refuses_requests_until_an_operator_repairs_it() {
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
    let (status, level) = api(&app, "GET", "/v1/admin/scopes", &token, None, None).await;
    assert_eq!(status, StatusCode::OK, "{level}");
    let root = level["parent"]["id"].as_str().expect("root id").to_owned();
    let create = |slug: &str| {
        json!({
            "parent_id": root, "kind": "org_unit",
            "slug": slug, "display_name": slug
        })
    };

    // An invalid new pack governs no scope, yet the tenant has no valid
    // current authority while it is stored.
    store_pack(&pool, tenant_id, "authz1-broken", MALFORMED_PACK).await;
    let (status, body) = api(&app, "GET", "/v1/admin/scopes", &token, None, None).await;
    assert_policy_unavailable(status, &body, &["authz1-broken"]);
    let key = Some("authz1-create-acme");
    let (status, body) = api(
        &app,
        "POST",
        "/v1/admin/scopes",
        &token,
        key,
        Some(create("acme")),
    )
    .await;
    assert_policy_unavailable(status, &body, &["authz1-broken"]);
    let (status, body) = api(
        &app,
        "POST",
        "/v1/admin/scopes",
        &token,
        Some("authz1-create-acme-again"),
        Some(create("acme")),
    )
    .await;
    assert_policy_unavailable(status, &body, &["authz1-broken"]);

    // The refused request recorded nothing, so the repair lets the same key
    // create exactly once.
    operator_clear(&pool, tenant_id, "authz1-broken").await;
    let (status, created) = api(
        &app,
        "POST",
        "/v1/admin/scopes",
        &token,
        key,
        Some(create("acme")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let (status, replayed) = api(
        &app,
        "POST",
        "/v1/admin/scopes",
        &token,
        key,
        Some(create("acme")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replayed}");
    assert_eq!(replayed["id"], created["id"]);

    // A cleared and recreated pack returns to version 1 with different bytes.
    // The process compile under that name/version must not be reused.
    assert_eq!(
        store_pack(&pool, tenant_id, "authz1-recreated", READ_ONLY_PACK).await,
        1
    );
    assert_eq!(
        authz::refresh_tenant_packs(&pool, &pdp, tenant_id).await,
        "installed"
    );
    operator_clear(&pool, tenant_id, "authz1-recreated").await;
    assert_eq!(
        store_pack(&pool, tenant_id, "authz1-recreated", MALFORMED_PACK).await,
        1
    );
    assert_eq!(
        pdp.installed_versions(tenant_id),
        vec![("authz1-recreated".to_owned(), 1)],
        "the process cache still holds the earlier valid version 1"
    );
    let (status, body) = api(&app, "GET", "/v1/admin/scopes", &token, None, None).await;
    assert_policy_unavailable(status, &body, &["authz1-recreated"]);

    operator_clear(&pool, tenant_id, "authz1-recreated").await;
    let (status, level) = api(&app, "GET", "/v1/admin/scopes", &token, None, None).await;
    assert_eq!(status, StatusCode::OK, "{level}");
    assert_eq!(
        authz::refresh_tenant_packs(&pool, &pdp, tenant_id).await,
        "removed"
    );
}

/// A committed restrictive selection governs the next request before any
/// process refresh. A malformed replacement of that selected pack refuses
/// instead of falling back to its last valid compile, and the operator's
/// repair restores the stored policy with verifiable audit evidence.
#[tokio::test]
async fn committed_policy_governs_the_next_request_and_repair_is_audited() {
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
    let (status, level) = api(&app, "GET", "/v1/admin/scopes", &token, None, None).await;
    assert_eq!(status, StatusCode::OK, "{level}");
    let root = level["parent"]["id"].as_str().expect("root id").to_owned();
    let create = json!({
        "parent_id": root, "kind": "org_unit",
        "slug": "denied", "display_name": "Denied"
    });

    store_pack(&pool, tenant_id, "authz1-bound", READ_ONLY_PACK).await;
    let mut tx = rls::begin_tenant_tx(&pool, tenant_id)
        .await
        .expect("begin governed selection");
    configuration_support::bind_tenant_pack(&mut tx, tenant_id, "authz1-bound").await;
    tx.commit().await.expect("commit governed selection");
    assert!(
        pdp.installed_versions(tenant_id).is_empty(),
        "no process refresh has run"
    );
    let key = Some("authz1-create-denied");
    let (status, denied) = api(
        &app,
        "POST",
        "/v1/admin/scopes",
        &token,
        key,
        Some(create.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    assert!(
        denied["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("authz1-bound@1")),
        "{denied}"
    );
    let (status, level) = api(&app, "GET", "/v1/admin/scopes", &token, None, None).await;
    assert_eq!(status, StatusCode::OK, "{level}");

    // The process cache holds version 1; the malformed version 2 is current.
    assert_eq!(
        authz::refresh_tenant_packs(&pool, &pdp, tenant_id).await,
        "installed"
    );
    assert_eq!(
        store_pack(&pool, tenant_id, "authz1-bound", MALFORMED_PACK).await,
        2
    );
    assert_eq!(
        authz::refresh_tenant_packs(&pool, &pdp, tenant_id).await,
        "error"
    );
    assert_eq!(
        pdp.installed_versions(tenant_id),
        vec![("authz1-bound".to_owned(), 1)]
    );
    let (status, body) = api(&app, "GET", "/v1/admin/scopes", &token, None, None).await;
    assert_policy_unavailable(status, &body, &["authz1-bound"]);
    let (status, body) = api(
        &app,
        "POST",
        "/v1/admin/scopes",
        &token,
        key,
        Some(create.clone()),
    )
    .await;
    assert_policy_unavailable(status, &body, &["authz1-bound"]);
    let (before, verification) = audit_chain(&pool, tenant_id).await;
    assert!(
        matches!(verification, ChainVerification::Valid { .. }),
        "{verification:?}"
    );

    // A selected pack cannot be cleared; the repair replaces its source.
    assert_eq!(
        operator_apply(&pool, tenant_id, "authz1-bound", READ_ONLY_PACK).await,
        3
    );
    let (status, level) = api(&app, "GET", "/v1/admin/scopes", &token, None, None).await;
    assert_eq!(status, StatusCode::OK, "{level}");
    let (status, denied) = api(&app, "POST", "/v1/admin/scopes", &token, key, Some(create)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    assert!(
        denied["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("authz1-bound@3")),
        "the repaired stored version decides: {denied}"
    );

    let (after, verification) = audit_chain(&pool, tenant_id).await;
    assert_eq!(
        verification,
        ChainVerification::Valid {
            events: i64::try_from(after.len()).expect("bounded chain")
        }
    );
    assert!(after.len() > before.len());
    for (earlier, retained) in before.iter().zip(&after) {
        assert_eq!(
            (earlier.seq, &earlier.hash),
            (retained.seq, &retained.hash),
            "the repair must not rewrite earlier evidence"
        );
    }
    let repair = after[before.len()..]
        .iter()
        .find(|event| event.action == "policy.pack.applied")
        .expect("the repair is audited");
    assert_eq!(repair.actor_kind, "break_glass");
    assert_eq!(
        repair.payload,
        json!({"pack": "authz1-bound", "version": 3})
    );
}
