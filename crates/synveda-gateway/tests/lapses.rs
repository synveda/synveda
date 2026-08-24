//! AUTHZ-4 regressions that remain meaningful until governed relaxations
//! replace the lapse plane.
//!
//! CPR-17 removed raw-record publication and composition. The surviving
//! contract is fail-closed validation plus durable, queryable expiry evidence;
//! Knowledge deliberately does not inherit the retired `memory.read` lapse.

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
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder};
use synveda_policy::Pdp;
use synveda_retrieval::index::SearchIndex;
use synveda_store::{access, identities, scopes, tenants};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{GrantId, Identity, IdentityId, IdentityKind, ScopeId, TenantId, TenantStatus};
use tower::ServiceExt;

const SECRET: &[u8] = b"authz-4-test-secret";
const WINDOW_SECS: u32 = 4;

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
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp: Arc::new(Pdp::new().expect("build the embedded PDP")),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index: Arc::new(
            SearchIndex::open(
                std::env::temp_dir()
                    .join("synveda-authz4-tests")
                    .join(TenantId::new().to_string()),
            )
            .expect("open sidecar"),
        ),
        embedder: Arc::new(AnyEmbedder::Deterministic(DeterministicEmbedder::new())),
        inject_embed_timeout: Duration::from_millis(100),
        keys: Arc::new(synveda_store::keys::KeyRing::new(
            synveda_crypto::Kms::Local(
                synveda_crypto::LocalKms::from_hex(&"11".repeat(32), "local:test")
                    .expect("test kek"),
            ),
        )),
    }
}

fn issue(subject: &str, tenant: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue(subject, tenant, Duration::from_secs(300))
}

async fn admitted_tenant() -> Option<(PgPool, TenantId)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("skipping AUTHZ-4 lapse test: DATABASE_URL is not set");
            return None;
        }
    };
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");
    let tenant = TenantId::new();
    tenants::create(
        &pool,
        tenant,
        &format!("authz4-{}", tenant.as_uuid().simple()),
        "AUTHZ-4 test tenant",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    Some((pool, tenant))
}

fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("checked by admitted_tenant")
}

struct Org {
    eng: Scope,
    platform: Scope,
    payments: Scope,
}

async fn child(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    parent: ScopeId,
    slug: &str,
) -> Scope {
    scopes::create(
        tx,
        &scopes::NewScope {
            id: ScopeId::new(),
            tenant_id: tenant,
            kind: ScopeKind::OrgUnit,
            parent_scope_id: Some(parent),
            slug: slug.to_owned(),
            display_name: slug.to_owned(),
            attributes: json!({}),
            principal_id: None,
            created_by: None,
        },
    )
    .await
    .expect("create scope")
}

async fn seed_hierarchy(pool: &PgPool, tenant: TenantId) -> Org {
    let mut tx = pool.begin().await.expect("begin");
    let root = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("mint root");
    let eng = child(&mut tx, tenant, root.id, "eng").await;
    let platform = child(&mut tx, tenant, eng.id, "platform").await;
    let payments = child(&mut tx, tenant, eng.id, "payments").await;
    tx.commit().await.expect("commit scopes");
    Org {
        eng,
        platform,
        payments,
    }
}

async fn seed_user(pool: &PgPool, tenant: TenantId, subject: &str) -> Identity {
    let mut tx = pool.begin().await.expect("begin");
    let own = scopes::ensure_principal_scope(&mut tx, tenant, subject, subject)
        .await
        .expect("mint principal scope");
    let identity = identities::create(
        &mut tx,
        IdentityId::new(),
        tenant,
        Some(subject),
        IdentityKind::User,
        None,
        None,
        own.id,
    )
    .await
    .expect("create identity");
    tx.commit().await.expect("commit user");
    identity
}

async fn bind(pool: &PgPool, tenant: TenantId, subject: &str, scope: ScopeId) {
    let mut tx = pool.begin().await.expect("begin");
    access::create_grant(
        &mut *tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id: tenant,
            scope_id: scope,
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
    .expect("create grant");
    tx.commit().await.expect("commit grant");
}

async fn call(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(request).await.expect("route responds");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, body)
}

async fn post(app: &Router, token: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    call(
        app,
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("build request"),
    )
    .await
}

async fn get(app: &Router, token: &str, uri: &str) -> (StatusCode, Value) {
    call(
        app,
        Request::builder()
            .method("GET")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("build request"),
    )
    .await
}

async fn propose(
    app: &Router,
    token: &str,
    target: ScopeId,
    grantee: ScopeId,
    duration_secs: u32,
    reason: &str,
) -> (StatusCode, Value) {
    post(
        app,
        token,
        "/v1/lapses",
        json!({
            "scope_id": target,
            "grantee_scope_id": grantee,
            "action": "memory.read",
            "duration_secs": duration_secs,
            "reason": reason,
        }),
    )
    .await
}

async fn proposed(app: &Router, token: &str, target: ScopeId, grantee: ScopeId) -> String {
    let (status, body) = propose(
        app,
        token,
        target,
        grantee,
        WINDOW_SECS,
        "joint incident review",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "opening lapse: {body}");
    body["proposal_id"]
        .as_str()
        .expect("proposal id")
        .to_owned()
}

async fn approve(app: &Router, token: &str, proposal: &str) {
    let (status, body) = post(
        app,
        token,
        &format!("/v1/proposals/{proposal}/approve"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "approval: {body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn meaningless_lapses_fail_closed_at_the_surface() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let app = router(state(&database_url()));
    let org = seed_hierarchy(&pool, tenant).await;
    let sam = seed_user(&pool, tenant, "sam").await;
    seed_user(&pool, tenant, "nadia").await;
    bind(&pool, tenant, "nadia", org.eng.id).await;
    let nadia = issue("nadia", tenant);

    let (status, body) = propose(
        &app,
        &nadia,
        sam.scope_id,
        org.payments.id,
        600,
        "read personal notes",
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    let (status, body) = propose(
        &app,
        &issue("sam", tenant),
        sam.scope_id,
        org.payments.id,
        600,
        "share personal notes",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("personal scope"),
        "{body}"
    );

    let (status, body) = propose(
        &app,
        &nadia,
        org.eng.id,
        org.payments.id,
        600,
        "already inherited",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("already composes"),
        "{body}"
    );

    let (status, body) = post(
        &app,
        &nadia,
        "/v1/lapses",
        json!({
            "scope_id": org.platform.id,
            "grantee_scope_id": org.payments.id,
            "action": "policy.assign",
            "duration_secs": 600,
            "reason": "unsupported action",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, body) = propose(
        &app,
        &nadia,
        org.platform.id,
        org.payments.id,
        60 * 60 * 24 * 45,
        "past the ceiling",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("ceiling"),
        "{body}"
    );

    let (status, _) = propose(&app, &nadia, org.platform.id, org.payments.id, 600, "  ").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_listing_keeps_expired_grants() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let app = router(state(&database_url()));
    let org = seed_hierarchy(&pool, tenant).await;
    for subject in ["nadia", "omar"] {
        seed_user(&pool, tenant, subject).await;
        bind(&pool, tenant, subject, org.eng.id).await;
    }
    let nadia = issue("nadia", tenant);
    let omar = issue("omar", tenant);
    let proposal = proposed(&app, &nadia, org.platform.id, org.payments.id).await;
    approve(&app, &nadia, &proposal).await;
    approve(&app, &omar, &proposal).await;
    let (status, body) = post(
        &app,
        &omar,
        &format!("/v1/proposals/{proposal}/lapse"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "apply lapse: {body}");

    let path = format!("/v1/lapses?scope_id={}", org.platform.id);
    let (status, body) = get(&app, &nadia, &path).await;
    assert_eq!(status, StatusCode::OK, "listing: {body}");
    assert_eq!(body["lapses"].as_array().expect("lapses").len(), 1);
    assert_eq!(body["lapses"][0]["outcome"], json!("active"));

    tokio::time::sleep(Duration::from_secs(u64::from(WINDOW_SECS) + 1)).await;
    let (_, body) = get(&app, &nadia, &path).await;
    assert_eq!(body["lapses"].as_array().expect("lapses").len(), 1);
    assert_eq!(body["lapses"][0]["outcome"], json!("expired"));
}
