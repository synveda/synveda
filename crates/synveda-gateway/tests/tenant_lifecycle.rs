//! Tenant suspension through the public gateway (ADR-0157): a bearer issued
//! before suspension, a fresh bearer and a stored console session all fail
//! with the uniform 401, another tenant keeps serving, each refusal is chained
//! on the suspended tenant, and resumption restores the same credentials.
//!
//! The router uses the ordinary `DATABASE_URL` role; suspension and
//! resumption use the exact test migrator, as the operator CLI does.

#[path = "../../synveda-store/tests/support/tenant_fixture.rs"]
mod tenant_fixture;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use chrono::Utc;
use metrics_exporter_prometheus::PrometheusHandle;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, behavior_test_router as router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_identity::console::{CONSOLE_COOKIE, mint};
use synveda_store::tenant_lifecycle::{self, Transition};
use synveda_types::{TenantId, TenantStatus};
use tower::ServiceExt;

const SECRET: &[u8] = b"tenant-lifecycle-test-secret";

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
            .acquire_timeout(Duration::from_secs(5))
            .connect_lazy(url)
            .expect("parse database url"),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: "http://console.test".to_owned(),
        pdp: Arc::new(synveda_policy::Pdp::new().expect("build the embedded PDP")),
        service_token_max_ttl: Duration::from_secs(3600),
        embedder: Arc::new(synveda_ingest::embedding::AnyEmbedder::Deterministic(
            synveda_ingest::embedding::DeterministicEmbedder::new(),
        )),
        context_embed_timeout: Duration::from_millis(100),
        git_sources: None,
        git_exports: None,
        keys: Arc::new(synveda_store::keys::KeyRing::new(
            synveda_crypto::Kms::Local(
                synveda_crypto::LocalKms::from_hex(&"11".repeat(32), "local:test")
                    .expect("test kek"),
            ),
        )),
    }
}

fn bearer(subject: &str, tenant: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue(subject, tenant, Duration::from_secs(300))
}

async fn admit(pool: &PgPool, prefix: &str) -> TenantId {
    let tenant = TenantId::new();
    let slug = format!("{prefix}-{}", uuid::Uuid::now_v7().simple());
    tenant_fixture::create(pool, tenant, &slug, "lifecycle", TenantStatus::Active)
        .await
        .expect("admit tenant");
    tenant
}

async fn console_session(state: &AppState, subject: &str, tenant: TenantId) -> String {
    state
        .keys
        .provision(&state.pool, synveda_crypto::KeyScope::Deployment)
        .await
        .expect("provision the deployment key");
    let secret = mint().expect("mint a session secret");
    let sealed = state
        .seal_console_token(
            &secret.hash,
            synveda_crypto::Purpose::ConsoleAccessToken,
            &bearer(subject, tenant),
        )
        .await
        .expect("seal the access token");
    synveda_store::console_sessions::create(
        &state.pool,
        &secret.hash,
        "http://idp.test",
        &sealed,
        Some(Utc::now() + chrono::Duration::minutes(5)),
        None,
        Utc::now() + chrono::Duration::hours(12),
    )
    .await
    .expect("open a console session");
    secret.secret
}

enum Credential<'a> {
    Bearer(&'a str),
    Cookie(&'a str),
}

async fn whoami(app: &axum::Router, credential: Credential<'_>) -> StatusCode {
    let request = Request::get("/v1/whoami");
    let request = match credential {
        Credential::Bearer(token) => {
            request.header(header::AUTHORIZATION, format!("Bearer {token}"))
        }
        Credential::Cookie(secret) => {
            request.header(header::COOKIE, format!("{CONSOLE_COOKIE}={secret}"))
        }
    };
    app.clone()
        .oneshot(request.body(Body::empty()).expect("build request"))
        .await
        .expect("send request")
        .status()
}

async fn transition(migrator: &PgPool, tenant: TenantId, transition: Transition) {
    let path = std::env::var_os("SYNVEDA_DATABASE_ROLES_FILE").expect("role contract file");
    let roles = synveda_store::runtime_role::DatabaseRoles::parse_json(
        std::fs::read_to_string(path)
            .expect("read role contract")
            .trim_end(),
    )
    .expect("parse role contract");
    let mut tx = synveda_store::rls::begin_migrator_tenant_tx(migrator, tenant, &roles)
        .await
        .expect("migrator transaction");
    tenant_lifecycle::lock_exclusive(&mut tx, tenant)
        .await
        .expect("exclusive admission");
    assert!(
        tenant_lifecycle::apply(&mut tx, tenant, transition)
            .await
            .expect("transition")
    );
    tx.commit().await.expect("commit transition");
}

async fn denials(pool: &PgPool, tenant: TenantId) -> usize {
    let mut tx = synveda_store::rls::begin_tenant_snapshot_tx(pool, tenant)
        .await
        .expect("snapshot");
    let events = synveda_audit::tail(&mut tx, tenant, 100)
        .await
        .expect("read chain");
    tx.rollback().await.expect("end snapshot");
    events
        .iter()
        .filter(|event| event.action == "tenant.resolution.denied")
        .count()
}

#[tokio::test]
async fn suspension_refuses_existing_and_fresh_credentials_until_resumed() {
    let _serial = serial().await;
    let Some(url) = tenant_fixture::database_url("tenant lifecycle gateway test") else {
        return;
    };
    let state = state(&url);
    synveda_store::epoch::verify(&state.pool)
        .await
        .expect("current schema");
    let migrator = tenant_fixture::migrator_pool(&state.pool).await;
    let tenant = admit(&state.pool, "suspended").await;
    let canary = admit(&state.pool, "canary").await;
    let app = router(state.clone());

    let existing = bearer("ada@example.test", tenant);
    let cookie = console_session(&state, "ada@example.test", tenant).await;
    let canary_bearer = bearer("grace@example.test", canary);
    assert_eq!(
        whoami(&app, Credential::Bearer(&existing)).await,
        StatusCode::OK
    );
    assert_eq!(
        whoami(&app, Credential::Cookie(&cookie)).await,
        StatusCode::OK
    );

    transition(&migrator, tenant, Transition::Suspend).await;
    let fresh = bearer("ada@example.test", tenant);
    for credential in [
        Credential::Bearer(&existing),
        Credential::Bearer(&fresh),
        Credential::Cookie(&cookie),
    ] {
        assert_eq!(whoami(&app, credential).await, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(
        whoami(&app, Credential::Bearer(&canary_bearer)).await,
        StatusCode::OK,
        "another tenant keeps serving"
    );
    assert_eq!(
        denials(&state.pool, tenant).await,
        3,
        "each refusal is chained on the suspended tenant"
    );
    assert_eq!(denials(&state.pool, canary).await, 0);

    transition(&migrator, tenant, Transition::Resume).await;
    assert_eq!(
        whoami(&app, Credential::Bearer(&existing)).await,
        StatusCode::OK
    );
    assert_eq!(
        whoami(&app, Credential::Bearer(&fresh)).await,
        StatusCode::OK
    );
    assert_eq!(
        whoami(&app, Credential::Cookie(&cookie)).await,
        StatusCode::OK
    );
}
