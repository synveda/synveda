//! AUTH-3 acceptance criteria (ADR-0018): an agent token with team scope
//! cannot call org-scope endpoints — even when its subject holds a
//! tenant-wide org-admin binding, because the base layer confines every
//! decision to the anchor subtree. Plus the surrounding contract: the
//! client-credentials grant end to end against a mock IdP, the
//! PDP-gated registration surface, the fail-closed containment of
//! unregistered clients, the service-token lifetime cap, and revocation
//! taking effect on the very next request.
//!
//! Tests need a live Postgres: they read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database), same convention as
//! tests/jit_provisioning.rs.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Body;
use axum::extract::{Form, State};
use axum::http::{Method, Request, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::{OidcVerifier, parse_issuers, personal_slug};
use synveda_store::{hierarchy, identities, role_bindings, tenants};
use synveda_types::{
    HierarchyNode, Identity, IdentityId, IdentityKind, Role, ScopeId, ScopeKind, TenantId,
    TenantStatus,
};
use tower::ServiceExt;

const KEY_PEM: &str = include_str!("fixtures/idp_key_a.pem");
const KEY_JWK: &str = include_str!("fixtures/idp_key_a.jwk.json");
const CLIENT_ID: &str = "synveda-test";
const SERVICE_AUDIENCE: &str = "synveda-agents";
const AGENT_CLIENT: &str = "ci-agent";
const AGENT_SECRET: &str = "ci-agent-secret";

/// Serialises tests: the Prometheus recorder and tracing's
/// callsite-interest cache are process-global (same rationale as
/// tests/oidc_login.rs).
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

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs()
}

// ── The mock IdP (client-credentials shaped) ─────────────────────────────────

/// An in-process OIDC provider serving discovery, JWKS, and a token
/// endpoint speaking the OAuth2 client-credentials grant (ADR-0018
/// decision 1): the registered client authenticates with its secret and
/// receives an RS256 access token carrying its own audience — the shape
/// Rauthy and the enterprise IdPs mint for headless agents.
#[derive(Clone)]
struct MockIdp {
    issuer: String,
    clients: Arc<HashMap<String, String>>,
}

impl MockIdp {
    async fn spawn() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock idp");
        let addr = listener.local_addr().expect("mock idp addr");
        let idp = Self {
            issuer: format!("http://{addr}/mock-idp"),
            clients: Arc::new(HashMap::from([(
                AGENT_CLIENT.to_owned(),
                AGENT_SECRET.to_owned(),
            )])),
        };
        let app = Router::new()
            .route("/mock-idp/.well-known/openid-configuration", get(discovery))
            .route("/mock-idp/jwks", get(jwks_endpoint))
            .route("/mock-idp/token", post(token_endpoint))
            .with_state(idp.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("mock idp serve");
        });
        idp
    }

    fn sign(&self, claims: &Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("key-a".to_owned());
        let key = EncodingKey::from_rsa_pem(KEY_PEM.as_bytes()).expect("test key");
        jsonwebtoken::encode(&header, claims, &key).expect("sign token")
    }

    /// A user bearer token, as the IdP mints for interactive callers.
    fn user_token(&self, subject: &str) -> String {
        self.sign(&json!({
            "iss": self.issuer,
            "sub": subject,
            "aud": CLIENT_ID,
            "iat": now_secs(),
            "exp": now_secs() + 600,
        }))
    }

    /// A service access token minted directly — the edge cases the seam
    /// must refuse even though a well-behaved IdP would not mint them.
    fn service_token(&self, subject: &str, ttl_secs: u64, include_iat: bool) -> String {
        let mut claims = json!({
            "iss": self.issuer,
            "sub": subject,
            "aud": SERVICE_AUDIENCE,
            "exp": now_secs() + ttl_secs,
        });
        if include_iat {
            claims["iat"] = json!(now_secs());
        }
        self.sign(&claims)
    }

    /// The real grant: POST the token endpoint with client credentials,
    /// as the headless agent does.
    async fn client_credentials(&self, client_id: &str, client_secret: &str) -> String {
        let response = reqwest::Client::new()
            .post(format!("{}/token", self.issuer))
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", client_id),
                ("client_secret", client_secret),
            ])
            .send()
            .await
            .expect("token endpoint");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let body: Value = response.json().await.expect("token body");
        body["access_token"]
            .as_str()
            .expect("access token")
            .to_owned()
    }
}

async fn discovery(State(idp): State<MockIdp>) -> Json<Value> {
    Json(json!({
        "issuer": idp.issuer,
        "authorization_endpoint": format!("{}/authorize", idp.issuer),
        "token_endpoint": format!("{}/token", idp.issuer),
        "jwks_uri": format!("{}/jwks", idp.issuer),
    }))
}

async fn jwks_endpoint() -> Json<Value> {
    Json(json!({ "keys": [serde_json::from_str::<Value>(KEY_JWK).expect("jwk fixture")] }))
}

async fn token_endpoint(
    State(idp): State<MockIdp>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let authenticated = form.get("grant_type").map(String::as_str) == Some("client_credentials")
        && form
            .get("client_id")
            .zip(form.get("client_secret"))
            .is_some_and(|(id, secret)| idp.clients.get(id) == Some(secret));
    if !authenticated {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "invalid_client" })),
        )
            .into_response();
    }
    // The Rauthy shape (ADR-0018 decision 1): client-credentials access
    // tokens carry `sub: null` and name the client in `azp` — the
    // verifier's azp fallback is what makes them usable. The direct
    // `service_token` mints cover the sub-bearing (Entra) shape.
    let token = idp.sign(&json!({
        "iss": idp.issuer,
        "sub": Value::Null,
        "azp": form["client_id"].clone(),
        "aud": SERVICE_AUDIENCE,
        "iat": now_secs(),
        "exp": now_secs() + 600,
    }));
    Json(json!({
        "access_token": token,
        "token_type": "Bearer",
        "expires_in": 600,
    }))
    .into_response()
}

// ── Gateway harness ──────────────────────────────────────────────────────────

/// The gateway in OIDC mode with a static tenant binding (the dev-Rauthy
/// shape, ADR-0010 decision 4) and the agents' audience accepted on
/// bearer tokens (ADR-0018 decision 1).
fn state(url: &str, issuer: &str, tenant: TenantId) -> AppState {
    let config = format!(
        r#"[{{"issuer":"{issuer}","client_id":"{CLIENT_ID}",
             "tenant":{{"static":{{"tenant_id":"{tenant}"}}}},
             "service_audiences":["{SERVICE_AUDIENCE}"]}}]"#
    );
    let verifier = Arc::new(
        OidcVerifier::new(parse_issuers(&config).expect("issuer config"))
            .expect("build verifier")
            .with_refresh_min_interval(Duration::ZERO),
    );
    AppState {
        pool: PgPoolOptions::new()
            .max_connections(2)
            .connect_lazy(url)
            .expect("parse database url"),
        metrics: metrics_handle(),
        verifier,
        login: None,
        pdp: Arc::new(synveda_policy::Pdp::new().expect("build the embedded PDP")),
        scope_chains: Arc::new(synveda_store::ScopeChainCache::new()),
        service_token_max_ttl: Duration::from_secs(3600),
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

async fn body_json(response: Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read response body")
        .to_bytes();
    if bytes.is_empty() {
        return Value::Null;
    }
    serde_json::from_slice(&bytes).expect("json body")
}

fn request(method: Method, uri: &str, bearer: &str, body: Option<Value>) -> Request<Body> {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {bearer}"));
    match body {
        Some(json) => builder
            .header("content-type", "application/json")
            .body(Body::from(json.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(req).await.unwrap();
    let status = response.status();
    (status, body_json(response).await)
}

/// Connects to `DATABASE_URL`, applies migrations, and admits one active
/// tenant. `None` = no database configured; the test skips quietly.
async fn admitted_tenant() -> Option<(PgPool, TenantId, String)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping service-identity test: DATABASE_URL is not set \
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
    let slug = format!("auth3-{}", id.as_uuid().simple());
    tenants::create(&pool, id, &slug, "AUTH-3 test tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((pool, id, url))
}

/// Seeds acme-org → eng (dept) → platform (team), plus the reserved
/// quarantine team. Returns (org, eng, platform, quarantine).
async fn seed_hierarchy(
    pool: &PgPool,
    tenant: TenantId,
) -> (HierarchyNode, HierarchyNode, HierarchyNode, HierarchyNode) {
    let mut tx = pool.begin().await.expect("begin");
    let org = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        None,
        ScopeKind::Org,
        "acme",
        "ACME",
    )
    .await
    .expect("create org");
    let eng = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(org.id),
        ScopeKind::Department,
        "eng",
        "Engineering",
    )
    .await
    .expect("create dept");
    let platform = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(eng.id),
        ScopeKind::Team,
        "platform",
        "Platform",
    )
    .await
    .expect("create team");
    let quarantine = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(org.id),
        ScopeKind::Team,
        identities::QUARANTINE_SLUG,
        "Quarantine",
    )
    .await
    .expect("create quarantine");
    tx.commit().await.expect("commit hierarchy");
    (org, eng, platform, quarantine)
}

/// Provisions a *user* identity at the store level (the JIT shape) so an
/// IdP-verified subject is not quarantined at the seam.
async fn seed_user(pool: &PgPool, tenant: TenantId, subject: &str, parent: ScopeId) -> Identity {
    let mut tx = pool.begin().await.expect("begin");
    let id = IdentityId::new();
    let leaf = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(parent),
        ScopeKind::User,
        &personal_slug(None, subject, id),
        subject,
    )
    .await
    .expect("create personal scope");
    let identity = identities::create(
        &mut tx,
        id,
        tenant,
        subject,
        IdentityKind::User,
        None,
        None,
        leaf.id,
    )
    .await
    .expect("create identity");
    tx.commit().await.expect("commit user");
    identity
}

async fn bind(pool: &PgPool, tenant: TenantId, subject: &str, scope: Option<ScopeId>, role: Role) {
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("tenant tx");
    role_bindings::bind(&mut *tx, tenant, subject, scope, role)
        .await
        .expect("bind role");
    tx.commit().await.expect("commit binding");
}

/// Registers the agent through the API as `admin_bearer` and returns the
/// created identity body.
async fn register_agent(app: &Router, admin_bearer: &str, anchor: ScopeId) -> Value {
    let (status, body) = send(
        app,
        request(
            Method::POST,
            "/v1/service-identities",
            admin_bearer,
            Some(json!({ "subject": AGENT_CLIENT, "scope_id": anchor })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "register agent: {body}");
    assert_eq!(body["kind"], "service", "{body}");
    body
}

// ── The AC ───────────────────────────────────────────────────────────────────

/// An agent registered at the platform team, holding a *tenant-wide
/// org-admin binding*, still cannot call org-scope endpoints: the token's
/// scope confines it to the team subtree. The same binding on a user
/// subject reaches the org — the clamp is the token scope, not the role
/// machinery.
#[tokio::test]
async fn agent_token_with_team_scope_cannot_call_org_scope_endpoints() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let (org, eng, platform, _) = seed_hierarchy(&pool, tenant).await;
    let idp = MockIdp::spawn().await;
    let app = router(state(&db_url, &idp.issuer, tenant));

    seed_user(&pool, tenant, "admin", org.id).await;
    bind(&pool, tenant, "admin", None, Role::OrgAdmin).await;
    let admin = idp.user_token("admin");
    register_agent(&app, &admin, platform.id).await;

    // The agent obtains its token through the real client-credentials
    // grant and — bound steward at its team — works its own subtree.
    let agent = idp.client_credentials(AGENT_CLIENT, AGENT_SECRET).await;
    bind(
        &pool,
        tenant,
        AGENT_CLIENT,
        Some(platform.id),
        Role::Steward,
    )
    .await;
    let (status, body) = send(
        &app,
        request(
            Method::GET,
            &format!("/v1/hierarchy/nodes/{}", platform.id),
            &agent,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "team-scope read: {body}");

    // Escalate the agent's *subject* to tenant-wide org-admin: a user
    // with this binding administers the whole tenant...
    bind(&pool, tenant, AGENT_CLIENT, None, Role::OrgAdmin).await;
    let (status, _) = send(
        &app,
        request(
            Method::GET,
            &format!("/v1/hierarchy/nodes/{}", org.id),
            &admin,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the admin user reaches the org");

    // ...but the agent's token stays confined: org-scope endpoints deny.
    for (method, uri, body) in [
        (Method::GET, format!("/v1/hierarchy/nodes/{}", org.id), None),
        (Method::GET, format!("/v1/hierarchy/nodes/{}", eng.id), None),
        (Method::GET, "/v1/hierarchy/root".to_owned(), None),
        (Method::GET, "/v1/roles/bindings".to_owned(), None),
        (
            Method::PUT,
            "/v1/policy/default".to_owned(),
            Some(json!({ "name": "standard" })),
        ),
        (
            Method::POST,
            "/v1/hierarchy/nodes".to_owned(),
            Some(json!({
                "parent_id": org.id,
                "kind": "department",
                "slug": "rogue",
                "name": "Rogue",
            })),
        ),
    ] {
        let (status, response) = send(&app, request(method.clone(), &uri, &agent, body)).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{method} {uri} must be denied to the team-scoped agent: {response}"
        );
    }

    // Within the subtree the org-admin binding still works — confinement
    // bounds authority, it does not subtract in-scope grants.
    let (status, body) = send(
        &app,
        request(
            Method::GET,
            &format!("/v1/hierarchy/nodes/{}", platform.id),
            &agent,
            None,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "in-subtree read still works: {body}"
    );
}

/// A client-credentials token whose subject was never registered is
/// quarantined at the seam (ADR-0013 decision 6): denied everything, even
/// with roles bound to the subject.
#[tokio::test]
async fn an_unregistered_client_token_is_quarantined_fail_closed() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let (_, _, platform, _) = seed_hierarchy(&pool, tenant).await;
    let idp = MockIdp::spawn().await;
    let app = router(state(&db_url, &idp.issuer, tenant));

    bind(&pool, tenant, "rogue", Some(platform.id), Role::Steward).await;
    let rogue = idp.service_token("rogue", 600, true);
    let (status, body) = send(
        &app,
        request(
            Method::GET,
            &format!("/v1/hierarchy/nodes/{}", platform.id),
            &rogue,
            None,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "an unregistered client must be quarantined: {body}"
    );
}

/// The lifetime cap (ADR-0018 decision 5): a service token that lives
/// longer than the configured maximum — or carries no `iat` at all — is
/// refused as the uniform 401. `/v1/whoami` (introspection, no PDP)
/// stays reachable, pinning the cap's documented boundary.
#[tokio::test]
async fn service_tokens_exceeding_the_lifetime_cap_are_refused() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let (org, _, platform, _) = seed_hierarchy(&pool, tenant).await;
    let idp = MockIdp::spawn().await;
    let app = router(state(&db_url, &idp.issuer, tenant));

    seed_user(&pool, tenant, "admin", org.id).await;
    bind(&pool, tenant, "admin", None, Role::OrgAdmin).await;
    register_agent(&app, &idp.user_token("admin"), platform.id).await;
    bind(
        &pool,
        tenant,
        AGENT_CLIENT,
        Some(platform.id),
        Role::Steward,
    )
    .await;

    let team_uri = format!("/v1/hierarchy/nodes/{}", platform.id);
    for (token, label) in [
        (idp.service_token(AGENT_CLIENT, 7200, true), "over-long"),
        (idp.service_token(AGENT_CLIENT, 600, false), "iat-less"),
    ] {
        let (status, body) = send(&app, request(Method::GET, &team_uri, &token, None)).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "the {label} service token must be refused: {body}"
        );
        let (status, _) = send(&app, request(Method::GET, "/v1/whoami", &token, None)).await;
        assert_eq!(status, StatusCode::OK, "whoami is introspection-only");
    }

    // The compliant token works — the cap refuses lifetimes, not agents.
    let ok = idp.service_token(AGENT_CLIENT, 600, true);
    let (status, body) = send(&app, request(Method::GET, &team_uri, &ok, None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// Registration is PDP-gated on the anchor (`ServiceIdentityManage`): an
/// unbound user cannot register anywhere; a team steward registers at
/// their team but not at the org; the quarantine scope is refused as an
/// anchor outright.
#[tokio::test]
async fn registration_is_pdp_gated_on_the_anchor() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let (org, _, platform, quarantine) = seed_hierarchy(&pool, tenant).await;
    let idp = MockIdp::spawn().await;
    let app = router(state(&db_url, &idp.issuer, tenant));

    seed_user(&pool, tenant, "nobody", platform.id).await;
    seed_user(&pool, tenant, "team-steward", platform.id).await;
    bind(
        &pool,
        tenant,
        "team-steward",
        Some(platform.id),
        Role::Steward,
    )
    .await;

    let register = |bearer: String, anchor: ScopeId, subject: &str| {
        let body = json!({ "subject": subject, "scope_id": anchor });
        let req = request(Method::POST, "/v1/service-identities", &bearer, Some(body));
        let app = app.clone();
        async move { app.oneshot(req).await.unwrap() }
    };

    let response = register(idp.user_token("nobody"), platform.id, "agent-a").await;
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "an unbound user must not register agents"
    );

    let response = register(idp.user_token("team-steward"), org.id, "agent-a").await;
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a team steward must not register agents at the org"
    );

    let response = register(idp.user_token("team-steward"), quarantine.id, "agent-a").await;
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "the quarantine scope is not an anchor"
    );

    let response = register(idp.user_token("team-steward"), platform.id, "agent-a").await;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "a team steward registers agents in their subtree"
    );

    // Subject collision: one identity per (tenant, subject).
    let response = register(idp.user_token("team-steward"), platform.id, "agent-a").await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

/// Revocation deletes the registration and its personal leaf; the agent's
/// very next request is quarantined fail-closed, and the listing/get
/// surfaces agree it is gone.
#[tokio::test]
async fn revocation_takes_effect_on_the_next_request() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let (org, _, platform, _) = seed_hierarchy(&pool, tenant).await;
    let idp = MockIdp::spawn().await;
    let app = router(state(&db_url, &idp.issuer, tenant));

    seed_user(&pool, tenant, "admin", org.id).await;
    bind(&pool, tenant, "admin", None, Role::OrgAdmin).await;
    let admin = idp.user_token("admin");
    let registered = register_agent(&app, &admin, platform.id).await;
    let id = registered["id"].as_str().expect("identity id").to_owned();
    let leaf = registered["scope_id"].as_str().expect("leaf id").to_owned();
    bind(
        &pool,
        tenant,
        AGENT_CLIENT,
        Some(platform.id),
        Role::Steward,
    )
    .await;

    let agent = idp.client_credentials(AGENT_CLIENT, AGENT_SECRET).await;
    let team_uri = format!("/v1/hierarchy/nodes/{}", platform.id);
    let (status, _) = send(&app, request(Method::GET, &team_uri, &agent, None)).await;
    assert_eq!(status, StatusCode::OK, "the agent works before revocation");

    // The tenant's list shows the registration.
    let (status, body) = send(
        &app,
        request(Method::GET, "/v1/service-identities", &admin, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["identities"][0]["subject"], AGENT_CLIENT, "{body}");

    let (status, _) = send(
        &app,
        request(
            Method::DELETE,
            &format!("/v1/service-identities/{id}"),
            &admin,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The very next agent request is quarantined (ADR-0013 decision 6):
    // an IdP-verified subject with no identity row.
    let (status, body) = send(&app, request(Method::GET, &team_uri, &agent, None)).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the revoked agent must be contained: {body}"
    );

    // Gone from the surfaces, leaf included.
    let (status, _) = send(
        &app,
        request(
            Method::GET,
            &format!("/v1/service-identities/{id}"),
            &admin,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(
        &app,
        request(
            Method::GET,
            &format!("/v1/hierarchy/nodes/{leaf}"),
            &admin,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "the personal leaf is gone");
}
