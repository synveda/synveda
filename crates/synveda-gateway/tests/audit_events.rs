//! AUD-1 at the gateway seams (ADR-0019 decision 5): an admin mutation
//! chains its semantic event with the deciding pack in the payload, an
//! allowed read chains its decision, a denial chains at the `respond`
//! seam, a suspended tenant's attempted resolution chains on that
//! tenant's log, and a service token over the TTL cap chains its seam
//! rejection — and the resulting chain verifies.
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
use synveda_audit::{ChainVerification, StoredEvent};
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_policy::Pdp;
use synveda_store::{hierarchy, identities, rls, role_bindings};
use synveda_types::{IdentityId, IdentityKind, Role, ScopeId, ScopeKind, TenantId, TenantStatus};
use tower::ServiceExt;

const SECRET: &[u8] = b"aud-1-test-secret";
const ADMIN: &str = "aud1-admin";

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
        service_token_max_ttl: Duration::from_secs(3600),
    }
}

fn issue(subject: &str, tenant_id: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue(subject, tenant_id, Duration::from_secs(300))
}

async fn admitted_tenant(pool: &PgPool, label: &str, status: TenantStatus) -> TenantId {
    let id = TenantId::new();
    let slug = format!("{label}-{}", id.as_uuid().simple());
    synveda_store::tenants::create(pool, id, &slug, "AUD-1 events test", status)
        .await
        .expect("admit tenant");
    id
}

/// Seeds the admin binding through the store — the CLI bootstrap path,
/// deliberately silent in the chain so assertions start from seq 1.
async fn bind_admin(pool: &PgPool, tenant_id: TenantId) {
    let mut tx = rls::begin_tenant_tx(pool, tenant_id)
        .await
        .expect("begin tenant tx");
    role_bindings::bind(&mut *tx, tenant_id, ADMIN, None, Role::OrgAdmin)
        .await
        .expect("bind admin");
    tx.commit().await.expect("commit binding");
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

/// The tenant's whole chain, oldest first, plus its verification result.
async fn chain(pool: &PgPool, tenant_id: TenantId) -> (Vec<StoredEvent>, ChainVerification) {
    let mut tx = rls::begin_tenant_tx(pool, tenant_id)
        .await
        .expect("begin tenant tx");
    let mut events = synveda_audit::tail(&mut tx, tenant_id, 100)
        .await
        .expect("read chain");
    events.reverse();
    let verification = synveda_audit::verify(&mut tx, tenant_id)
        .await
        .expect("verify chain");
    (events, verification)
}

fn database_url() -> Option<String> {
    let url = std::env::var("DATABASE_URL").ok();
    if url.is_none() {
        eprintln!(
            "skipping AUD-1 event tests: DATABASE_URL is not set \
             (run `make dev-up` then `make db-test`)"
        );
    }
    url
}

/// A mutation chains its semantic event, a read its allowed decision, a
/// denial its deny — one event per operation, and the chain verifies.
#[tokio::test]
async fn admin_plane_operations_chain_one_event_each() {
    let Some(url) = database_url() else { return };
    let state = state(&url);
    let pool = state.pool.clone();
    let app = router(state);

    let tenant_id = admitted_tenant(&pool, "aud1", TenantStatus::Active).await;
    bind_admin(&pool, tenant_id).await;
    let admin = issue(ADMIN, tenant_id);

    // Mutation: create the org root.
    let (status, body) = api(
        &app,
        "POST",
        "/v1/hierarchy/nodes",
        &admin,
        Some(json!({"kind": "org", "slug": "acme", "name": "ACME"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create root: {body}");
    let root = body["id"].as_str().expect("root id").to_owned();

    // Read: fetch it back.
    let (status, body) = api(
        &app,
        "GET",
        &format!("/v1/hierarchy/nodes/{root}"),
        &admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "read root: {body}");

    // Denial: a roleless subject tries to create under the root.
    let nobody = issue("aud1-nobody", tenant_id);
    let (status, body) = api(
        &app,
        "POST",
        "/v1/hierarchy/nodes",
        &nobody,
        Some(json!({"parent_id": root, "kind": "team", "slug": "rogue", "name": "Rogue"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "denied create: {body}");

    let (events, verification) = chain(&pool, tenant_id).await;
    assert_eq!(verification, ChainVerification::Valid { events: 3 });
    let [created, read, denied] = events.as_slice() else {
        panic!("expected exactly 3 chain events, got {}", events.len());
    };

    assert_eq!(created.action, "hierarchy.node.created");
    assert_eq!(created.outcome, "success");
    assert_eq!(created.actor_kind, "subject");
    assert_eq!(created.actor_subject, ADMIN);
    assert_eq!(created.resource, format!("scope {root}"));
    assert_eq!(created.payload["node"]["slug"], "acme");
    let pack = created.payload["authz"]["pack"].as_str().expect("pack");
    assert!(
        pack.starts_with("regulated-strict@"),
        "the deciding pack rides in the payload, got {pack:?}"
    );

    assert_eq!(read.action, "authz.decision");
    assert_eq!(read.outcome, "allow");
    assert_eq!(read.payload["op"], "get");
    assert_eq!(read.payload["authz"]["action"], "hierarchy.read");

    assert_eq!(denied.action, "authz.decision");
    assert_eq!(denied.outcome, "deny");
    assert_eq!(denied.actor_subject, "aud1-nobody");
    assert_eq!(denied.payload["action"], "hierarchy.create");
    assert!(
        denied.payload["reason"]
            .as_str()
            .expect("denial reason")
            .contains("regulated-strict@"),
        "the denial names the pack, got {:?}",
        denied.payload["reason"]
    );
}

/// A verified token naming a suspended tenant chains
/// `tenant.resolution.denied` on that tenant's log (ADR-0019 decision 6).
#[tokio::test]
async fn suspended_tenant_resolution_chains_the_denial() {
    let Some(url) = database_url() else { return };
    let state = state(&url);
    let pool = state.pool.clone();
    let app = router(state);

    let tenant_id = admitted_tenant(&pool, "aud1s", TenantStatus::Suspended).await;
    let token = issue("aud1-suspended-user", tenant_id);
    let (status, _) = api(&app, "GET", "/v1/whoami", &token, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (events, verification) = chain(&pool, tenant_id).await;
    assert_eq!(verification, ChainVerification::Valid { events: 1 });
    assert_eq!(events[0].action, "tenant.resolution.denied");
    assert_eq!(events[0].outcome, "deny");
    assert_eq!(events[0].actor_subject, "aud1-suspended-user");
    assert_eq!(events[0].payload["status"], "suspended");
}

/// A registered service identity presenting a token over the TTL cap
/// chains `auth.token.rejected` at the enforcement seam (ADR-0018
/// decision 5, ADR-0019 decision 5).
#[tokio::test]
async fn over_ttl_service_token_chains_the_rejection() {
    let Some(url) = database_url() else { return };
    let state = state(&url);
    let pool = state.pool.clone();
    let app = router(state);

    let tenant_id = admitted_tenant(&pool, "aud1t", TenantStatus::Active).await;
    // Register an agent through the store (the CLI bootstrap path): an
    // anchor team and a personal leaf carrying the identity row.
    let mut tx = rls::begin_tenant_tx(&pool, tenant_id)
        .await
        .expect("begin tenant tx");
    let org = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant_id,
        None,
        ScopeKind::Org,
        "acme",
        "ACME",
    )
    .await
    .expect("create org");
    let team = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant_id,
        Some(org.id),
        ScopeKind::Team,
        "agents",
        "Agents",
    )
    .await
    .expect("create team");
    let leaf = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant_id,
        Some(team.id),
        ScopeKind::User,
        "agent-1-leaf",
        "Agent 1",
    )
    .await
    .expect("create leaf");
    identities::create(
        &mut tx,
        IdentityId::new(),
        tenant_id,
        "agent-1",
        IdentityKind::Service,
        None,
        None,
        leaf.id,
    )
    .await
    .expect("register agent");
    tx.commit().await.expect("commit registration");

    // A token living twice the cap: refused at the seam with a uniform
    // 401, chained as auth.token.rejected.
    let long_lived =
        Hs256Verifier::new(SECRET).issue("agent-1", tenant_id, Duration::from_secs(7200));
    let (status, _) = api(&app, "GET", "/v1/hierarchy/root", &long_lived, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (events, verification) = chain(&pool, tenant_id).await;
    assert_eq!(verification, ChainVerification::Valid { events: 1 });
    assert_eq!(events[0].action, "auth.token.rejected");
    assert_eq!(events[0].outcome, "deny");
    assert_eq!(events[0].actor_subject, "agent-1");
    assert_eq!(events[0].payload["op"], "root");
}
