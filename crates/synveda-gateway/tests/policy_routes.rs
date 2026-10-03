//! AUTHZ-2/CPR-30 regression: policy source remains inspectable, while the
//! runtime selector is the immutable Configuration aggregate rather than the
//! deleted default/assignment plane.

#[path = "../../synveda-store/tests/support/tenant_fixture.rs"]
mod tenant_fixture;

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

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
use synveda_policy::Pdp;
use synveda_store::{access, policy_packs, rls, scopes};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{GrantId, PackConfig, ScopeId, TenantId};
use tower::ServiceExt;

#[path = "support/configuration.rs"]
mod configuration_support;

const SECRET: &[u8] = b"authz-2-test-secret";
const FROZEN_PACK: &str = r#"
permit (
    principal,
    action == Synveda::Action::"ScopeRead",
    resource
) when { resource in principal.tenant };

permit (
    principal,
    action == Synveda::Action::"ConfigurationRead",
    resource
) when { resource in principal.tenant };
"#;

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
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(2))
            .connect_lazy(url)
            .expect("parse database url"),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp: Arc::new(Pdp::new().expect("build embedded PDP")),
        service_token_max_ttl: Duration::from_secs(3_600),
        embedder: Arc::new(synveda_ingest::embedding::AnyEmbedder::Deterministic(
            synveda_ingest::embedding::DeterministicEmbedder::new(),
        )),
        context_embed_timeout: Duration::from_millis(100),
        keys: Arc::new(synveda_store::keys::KeyRing::new(
            synveda_crypto::Kms::Disabled,
        )),
    }
}

fn issue(tenant: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue("authz2-admin", tenant, Duration::from_secs(300))
}

async fn node(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: TenantId,
    parent: ScopeId,
    kind: ScopeKind,
    slug: &str,
) -> Scope {
    scopes::create(
        tx,
        &scopes::NewScope {
            id: ScopeId::new(),
            tenant_id: tenant,
            kind,
            parent_scope_id: Some(parent),
            slug: format!("{slug}-{}", ScopeId::new().as_uuid().simple()),
            display_name: slug.to_owned(),
            attributes: json!({}),
            principal_id: None,
            created_by: None,
        },
    )
    .await
    .expect("create governed scope")
}

async fn admitted(pool: &PgPool, label: &str) -> (TenantId, ScopeId, ScopeId, ScopeId) {
    let tenant = TenantId::new();
    tenant_fixture::create(
        pool,
        tenant,
        &format!("{label}-{}", tenant.as_uuid().simple()),
        "AUTHZ-2 Configuration selector test",
        synveda_types::TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant bootstrap");
    let root = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("create root");
    let workspace = node(&mut tx, tenant, root.id, ScopeKind::Workspace, "workspace").await;
    let project_a = node(
        &mut tx,
        tenant,
        workspace.id,
        ScopeKind::Project,
        "project-a",
    )
    .await;
    let project_b = node(
        &mut tx,
        tenant,
        workspace.id,
        ScopeKind::Project,
        "project-b",
    )
    .await;
    access::create_grant(
        &mut tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id: tenant,
            scope_id: root.id,
            subject: GrantSubject::Principal {
                principal_id: "authz2-admin".to_owned(),
            },
            role_key: RoleKey::Administrator,
            source: GrantSource::Automation,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .expect("grant administrator");
    configuration_support::bind_pack(&mut tx, tenant, workspace.id, synveda_policy::STANDARD).await;
    tx.commit().await.expect("commit tenant bootstrap");
    (tenant, workspace.id, project_a.id, project_b.id)
}

async fn api(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
    let request = if let Some(body) = body {
        request = request.header("content-type", "application/json");
        request.body(Body::from(body.to_string()))
    } else {
        request.body(Body::empty())
    }
    .expect("build request");
    let response = app.clone().oneshot(request).await.expect("call API");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect response")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("response is JSON")
    };
    (status, value)
}

#[tokio::test]
async fn immutable_configuration_selects_policy_per_scope() {
    let _guard = serial().await;
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping policy selector test: DATABASE_URL is not set");
        return;
    };
    let state = state(&url);
    synveda_store::epoch::verify(&state.pool)
        .await
        .expect("apply migrations");
    let (tenant, workspace, project_a, project_b) = admitted(&state.pool, "authz2").await;

    let mut tx = rls::begin_tenant_tx(&state.pool, tenant)
        .await
        .expect("begin custom-pack transaction");
    policy_packs::apply(
        &mut *tx,
        tenant,
        "authz2-frozen",
        FROZEN_PACK,
        &PackConfig::default(),
    )
    .await
    .expect("store custom policy pack");
    configuration_support::bind_pack(&mut tx, tenant, project_b, "authz2-frozen").await;
    tx.commit().await.expect("commit custom selection");
    assert_eq!(
        authz::refresh_tenant_packs(&state.pool, &state.pdp, tenant).await,
        "installed"
    );

    let app = router(state);
    let token = issue(tenant);
    let (status, packs) = api(&app, "GET", "/v1/policy/packs", &token, None).await;
    assert_eq!(status, StatusCode::OK, "{packs}");
    assert!(
        packs["packs"]
            .as_array()
            .expect("pack array")
            .iter()
            .any(|pack| pack["name"] == "authz2-frozen" && pack["kind"] == "stored")
    );

    let (status, inherited) = api(
        &app,
        "GET",
        &format!("/v1/configurations/effective?scope_id={project_a}"),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{inherited}");
    assert_eq!(
        inherited["document"]["policy_pack"],
        synveda_policy::STANDARD
    );
    assert_eq!(inherited["binding_scope_id"], workspace.to_string());

    let (status, exact) = api(
        &app,
        "GET",
        &format!("/v1/configurations/effective?scope_id={project_b}"),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{exact}");
    assert_eq!(exact["document"]["policy_pack"], "authz2-frozen");
    assert_eq!(exact["binding_scope_id"], project_b.to_string());

    let (status, denied) = api(
        &app,
        "PATCH",
        &format!("/v1/admin/scopes/{project_b}"),
        &token,
        Some(json!({"display_name": "Frozen project"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    assert!(
        denied["reason"]
            .as_str()
            .expect("denial reason")
            .contains("authz2-frozen@1")
    );
    let (status, updated) = api(
        &app,
        "PATCH",
        &format!("/v1/admin/scopes/{project_a}"),
        &token,
        Some(json!({"display_name": "Available project"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");

    for (method, path) in [
        ("GET", "/v1/policy/default".to_owned()),
        ("PUT", "/v1/policy/default".to_owned()),
        ("DELETE", "/v1/policy/default".to_owned()),
        ("GET", format!("/v1/admin/scopes/{project_a}/policy")),
    ] {
        let (status, _) = api(&app, method, &path, &token, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "old route survived: {path}");
    }
}

#[tokio::test]
async fn separate_gateway_compiles_converge_and_a_bad_pack_does_not_renew_freshness() {
    let _guard = serial().await;
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping cross-gateway policy refresh test: DATABASE_URL is not set");
        return;
    };
    let first = state(&url);
    let second = state(&url);
    synveda_store::epoch::verify(&first.pool)
        .await
        .expect("apply migrations");
    let (tenant, _, _, _) = admitted(&first.pool, "authz2-refresh").await;

    let mut tx = rls::begin_tenant_tx(&first.pool, tenant)
        .await
        .expect("begin policy transaction");
    policy_packs::apply(
        &mut *tx,
        tenant,
        "authz2-reload",
        FROZEN_PACK,
        &PackConfig::default(),
    )
    .await
    .expect("store first policy revision");
    tx.commit().await.expect("commit first revision");
    authz::refresh_packs_once(&first.pool, &first.pdp)
        .await
        .expect("first gateway refresh");
    authz::refresh_packs_once(&second.pool, &second.pdp)
        .await
        .expect("second gateway refresh");
    let metrics = metrics_handle().render();
    assert!(
        metrics
            .lines()
            .any(|line| line.starts_with("synveda_policy_pack_active_tenants ")),
        "active-tenant sweep count is absent:\n{metrics}"
    );
    assert!(
        metrics
            .lines()
            .any(|line| line.starts_with("synveda_policy_pack_sources_per_tenant_count ")),
        "per-tenant pack count is absent:\n{metrics}"
    );
    assert!(
        first
            .pdp
            .installed_versions(tenant)
            .contains(&("authz2-reload".to_owned(), 1))
    );
    assert!(
        second
            .pdp
            .installed_versions(tenant)
            .contains(&("authz2-reload".to_owned(), 1))
    );

    let mut tx = rls::begin_tenant_tx(&first.pool, tenant)
        .await
        .expect("begin policy update");
    policy_packs::apply(
        &mut *tx,
        tenant,
        "authz2-reload",
        FROZEN_PACK,
        &PackConfig::default(),
    )
    .await
    .expect("store second policy revision");
    tx.commit().await.expect("commit second revision");
    authz::refresh_packs_once(&first.pool, &first.pdp)
        .await
        .expect("first gateway sees update");
    assert!(
        first
            .pdp
            .installed_versions(tenant)
            .contains(&("authz2-reload".to_owned(), 2))
    );
    assert!(
        second
            .pdp
            .installed_versions(tenant)
            .contains(&("authz2-reload".to_owned(), 1)),
        "another gateway stays on its own compile until its sweep"
    );
    authz::refresh_packs_once(&second.pool, &second.pdp)
        .await
        .expect("second gateway sees update");
    assert!(
        second
            .pdp
            .installed_versions(tenant)
            .contains(&("authz2-reload".to_owned(), 2))
    );

    let mut tx = rls::begin_tenant_tx(&first.pool, tenant)
        .await
        .expect("begin bad policy update");
    policy_packs::apply(
        &mut *tx,
        tenant,
        "authz2-reload",
        "permit (",
        &PackConfig::default(),
    )
    .await
    .expect("store a deliberately invalid test pack");
    tx.commit().await.expect("commit bad revision");
    assert!(
        authz::refresh_packs_once(&second.pool, &second.pdp)
            .await
            .is_err()
    );
    assert!(
        second
            .pdp
            .installed_versions(tenant)
            .contains(&("authz2-reload".to_owned(), 2)),
        "a failed compile keeps last-good but cannot renew the lease"
    );

    let mut tx = rls::begin_tenant_tx(&first.pool, tenant)
        .await
        .expect("begin bad-pack cleanup");
    policy_packs::clear(&mut tx, tenant, "authz2-reload")
        .await
        .expect("remove invalid test pack");
    tx.commit().await.expect("commit cleanup");
}

/// Manual capacity evidence only: the roadmap has not selected a supported
/// tenant/pack ceiling, and a disposable local database is not production load.
#[tokio::test]
#[ignore = "manual OPS-7 policy-sweep capacity probe"]
async fn ops7_provisional_policy_sweep_capacity_probe() {
    const TENANTS: usize = 32;
    const PACKS_PER_TENANT: usize = 4;
    const DEADLINE: Duration = Duration::from_secs(5);

    let _guard = serial().await;
    let url = std::env::var("DATABASE_URL").expect("run with the isolated db-test fixture");
    let state = state(&url);
    synveda_store::epoch::verify(&state.pool)
        .await
        .expect("current schema");
    let mut ids = Vec::with_capacity(TENANTS);
    for _ in 0..TENANTS {
        let id = TenantId::new();
        tenant_fixture::create(
            &state.pool,
            id,
            &format!("ops7-probe-{}", id.as_uuid().simple()),
            "OPS-7 disposable sweep probe",
            synveda_types::TenantStatus::Active,
        )
        .await
        .expect("admit test tenant");
        let mut tx = rls::begin_tenant_tx(&state.pool, id)
            .await
            .expect("begin ordinary tenant transaction");
        for pack in 0..PACKS_PER_TENANT {
            policy_packs::apply(
                &mut *tx,
                id,
                &format!("ops7-probe-{pack}"),
                FROZEN_PACK,
                &PackConfig::default(),
            )
            .await
            .expect("store test pack");
        }
        tx.commit().await.expect("commit test packs");
        ids.push(id);
    }

    let cold_started = Instant::now();
    tokio::time::timeout(
        DEADLINE,
        authz::converge_packs_once(&state.pool, &state.pdp),
    )
    .await
    .expect("cold convergence exceeded the process deadline")
    .expect("cold convergence failed");
    let cold = cold_started.elapsed();
    for id in &ids {
        assert_eq!(state.pdp.installed_versions(*id).len(), PACKS_PER_TENANT);
    }

    let mut warm_max = Duration::ZERO;
    for _ in 0..5 {
        let started = Instant::now();
        tokio::time::timeout(DEADLINE, authz::refresh_packs_once(&state.pool, &state.pdp))
            .await
            .expect("unchanged sweep exceeded the process deadline")
            .expect("unchanged sweep failed");
        warm_max = warm_max.max(started.elapsed());
    }

    for id in &ids {
        let mut tx = rls::begin_tenant_tx(&state.pool, *id)
            .await
            .expect("begin test revision transaction");
        for pack in 0..PACKS_PER_TENANT {
            policy_packs::apply(
                &mut *tx,
                *id,
                &format!("ops7-probe-{pack}"),
                FROZEN_PACK,
                &PackConfig::default(),
            )
            .await
            .expect("revise test pack");
        }
        tx.commit().await.expect("commit test revisions");
    }
    let revised_started = Instant::now();
    tokio::time::timeout(DEADLINE, authz::refresh_packs_once(&state.pool, &state.pdp))
        .await
        .expect("revised sweep exceeded the process deadline")
        .expect("revised sweep failed");
    let revised = revised_started.elapsed();
    for id in &ids {
        assert!(
            state
                .pdp
                .installed_versions(*id)
                .iter()
                .all(|(_, version)| *version == 2),
            "every test pack revision must compile"
        );
    }
    eprintln!(
        "OPS-7 provisional {TENANTS}x{PACKS_PER_TENANT}: cold={:.3}s warm_max={:.3}s revised={:.3}s (5s deadline, local exact-role fixture)",
        cold.as_secs_f64(),
        warm_max.as_secs_f64(),
        revised.as_secs_f64()
    );
}

#[tokio::test]
async fn cross_tenant_configuration_probes_are_not_found() {
    let _guard = serial().await;
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping Configuration isolation test: DATABASE_URL is not set");
        return;
    };
    let state = state(&url);
    synveda_store::epoch::verify(&state.pool)
        .await
        .expect("apply migrations");
    let (victim, _, victim_project, _) = admitted(&state.pool, "authz2-victim").await;
    let (intruder, _, _, _) = admitted(&state.pool, "authz2-intruder").await;
    let victim_selection = {
        let mut tx = rls::begin_tenant_tx(&state.pool, victim)
            .await
            .expect("begin victim selection read");
        let selection = configuration_support::bind_pack(
            &mut tx,
            victim,
            victim_project,
            synveda_policy::OPEN_COLLABORATION,
        )
        .await;
        tx.commit().await.expect("commit victim selection");
        selection
    };
    let app = router(state);
    let token = issue(intruder);
    for path in [
        format!("/v1/configurations/{}", victim_selection.artifact_id),
        format!("/v1/configurations/effective?scope_id={victim_project}"),
    ] {
        let (status, body) = api(&app, "GET", &path, &token, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
    }
}
