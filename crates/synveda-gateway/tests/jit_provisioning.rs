//! AUTH-2 acceptance criteria (ADR-0013): a new user's first OIDC login
//! JIT-provisions them into the correct team scope from their groups with
//! zero admin action; unmapped users land in the quarantine scope with no
//! read rights. Plus the surrounding contract: repeat logins are
//! idempotent, the override table beats the convention, quarantine
//! enforcement is a PDP decision (403, not 401), and an IdP subject that
//! skips `/auth/login` is quarantined fail-closed.
//!
//! Runs against an in-process mock IdP (RS256, checked-in test keys) that
//! embeds a `groups` claim, per-login subject. Tests need a live Postgres:
//! they read `DATABASE_URL` and skip with a message when it is unset (CI
//! has no database), same convention as tests/oidc_login.rs.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Body;
use axum::extract::{Form, Query, State};
use axum::http::{Request, StatusCode, header};
use axum::response::{IntoResponse, Json, Redirect, Response};
use axum::routing::{get, post};
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::{LoginFlow, OidcVerifier, parse_issuers};
use synveda_store::{group_mappings, hierarchy, identities, tenants};
use synveda_types::{HierarchyNode, ScopeId, ScopeKind, TenantId, TenantStatus};
use tower::ServiceExt;

const KEY_PEM: &str = include_str!("fixtures/idp_key_a.pem");
const KEY_JWK: &str = include_str!("fixtures/idp_key_a.jwk.json");
const CLIENT_ID: &str = "synveda-test";
const REDIRECT_URI: &str = "http://gateway.test/auth/callback";

/// Serialises tests: the Prometheus recorder and tracing's callsite-interest
/// cache are process-global (same rationale as tests/oidc_login.rs).
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

// ── The mock IdP (groups-claim shaped) ───────────────────────────────────────

/// Who the next login authenticates as.
#[derive(Clone)]
struct CurrentUser {
    subject: String,
    groups: Vec<String>,
    email: Option<String>,
}

struct AuthCode {
    nonce: String,
    user: CurrentUser,
}

/// An in-process OIDC provider that signs RS256 ID tokens carrying a
/// `groups` array and a `tid` tenant claim. The PKCE/state/nonce contract
/// itself is AUTH-1's mock-Entra suite; this one focuses on claims.
#[derive(Clone)]
struct MockIdp {
    issuer: String,
    tid: String,
    user: Arc<Mutex<CurrentUser>>,
    codes: Arc<Mutex<HashMap<String, AuthCode>>>,
    next_code: Arc<AtomicUsize>,
}

impl MockIdp {
    async fn spawn(tenant_id: TenantId) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock idp");
        let addr = listener.local_addr().expect("mock idp addr");
        let idp = Self {
            issuer: format!("http://{addr}/mock-idp"),
            tid: tenant_id.to_string(),
            user: Arc::new(Mutex::new(CurrentUser {
                subject: "nobody".to_owned(),
                groups: Vec::new(),
                email: None,
            })),
            codes: Arc::new(Mutex::new(HashMap::new())),
            next_code: Arc::new(AtomicUsize::new(0)),
        };
        let app = Router::new()
            .route("/mock-idp/.well-known/openid-configuration", get(discovery))
            .route("/mock-idp/jwks", get(jwks_endpoint))
            .route("/mock-idp/authorize", get(authorize))
            .route("/mock-idp/token", post(token_endpoint))
            .with_state(idp.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("mock idp serve");
        });
        idp
    }

    /// Sets who the next login (or minted bearer) authenticates as.
    fn login_as(&self, subject: &str, groups: &[&str], email: Option<&str>) {
        *self.user.lock().unwrap() = CurrentUser {
            subject: subject.to_owned(),
            groups: groups.iter().map(|g| (*g).to_owned()).collect(),
            email: email.map(str::to_owned),
        };
    }

    fn sign(&self, claims: &Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("key-a".to_owned());
        let key = EncodingKey::from_rsa_pem(KEY_PEM.as_bytes()).expect("test key");
        jsonwebtoken::encode(&header, claims, &key).expect("sign token")
    }

    fn claims(&self, user: &CurrentUser, aud: &str, nonce: Option<&str>) -> Value {
        let mut claims = json!({
            "iss": self.issuer,
            "sub": user.subject,
            "aud": aud,
            "exp": now_secs() + 600,
            "iat": now_secs(),
            "tid": self.tid,
            "groups": user.groups,
            "name": user.subject,
        });
        if let Some(email) = &user.email {
            claims["email"] = json!(email);
        }
        if let Some(nonce) = nonce {
            claims["nonce"] = json!(nonce);
        }
        claims
    }

    /// A bearer token for the current user, as the IdP would mint — the
    /// path an API caller takes when it never touches `/auth/login`.
    fn access_token(&self) -> String {
        let user = self.user.lock().unwrap().clone();
        let claims = self.claims(&user, CLIENT_ID, None);
        self.sign(&claims)
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

async fn authorize(
    State(idp): State<MockIdp>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let code = format!("code-{}", idp.next_code.fetch_add(1, Ordering::SeqCst));
    idp.codes.lock().unwrap().insert(
        code.clone(),
        AuthCode {
            nonce: query["nonce"].clone(),
            user: idp.user.lock().unwrap().clone(),
        },
    );
    Redirect::temporary(&format!(
        "{}?code={code}&state={}",
        query["redirect_uri"], query["state"]
    ))
    .into_response()
}

async fn token_endpoint(
    State(idp): State<MockIdp>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let Some(auth) = form
        .get("code")
        .and_then(|code| idp.codes.lock().unwrap().remove(code))
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid_grant" })),
        )
            .into_response();
    };
    let id_claims = idp.claims(&auth.user, CLIENT_ID, Some(&auth.nonce));
    let access_claims = idp.claims(&auth.user, CLIENT_ID, None);
    Json(json!({
        "access_token": idp.sign(&access_claims),
        "id_token": idp.sign(&id_claims),
        "token_type": "Bearer",
        "expires_in": 600,
    }))
    .into_response()
}

// ── Gateway harness ──────────────────────────────────────────────────────────

fn state(url: &str, issuer: &str) -> AppState {
    let config = format!(r#"[{{"issuer":"{issuer}","client_id":"{CLIENT_ID}"}}]"#);
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
        verifier: verifier.clone(),
        login: Some(Arc::new(LoginFlow::new(verifier, REDIRECT_URI.to_owned()))),
        pdp: Arc::new(synveda_policy::Pdp::new().expect("build the embedded PDP")),
    }
}

async fn body_json(response: Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read response body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("json body")
}

fn get_request(uri: &str, bearer: Option<&str>) -> Request<Body> {
    let builder = Request::get(uri);
    let builder = match bearer {
        Some(token) => builder.header("authorization", format!("Bearer {token}")),
        None => builder,
    };
    builder.body(Body::empty()).unwrap()
}

/// Drives login as the browser would (the mock IdP authenticates whoever
/// `login_as` named) and returns the session body.
async fn login(app: &Router) -> Value {
    let response = app
        .clone()
        .oneshot(get_request("/auth/login", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = response.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .to_owned();
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let idp_response = client.get(&location).send().await.expect("authorize hop");
    let callback = idp_response.headers()[reqwest::header::LOCATION]
        .to_str()
        .unwrap();
    let callback = url::Url::parse(callback).expect("callback url");
    let callback = format!("/auth/callback?{}", callback.query().expect("query"));
    let response = app
        .clone()
        .oneshot(get_request(&callback, None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "callback must succeed");
    body_json(response).await
}

/// Connects to `DATABASE_URL`, applies migrations, and admits one active
/// tenant. `None` = no database configured; the test skips quietly.
async fn admitted_tenant() -> Option<(PgPool, TenantId, String)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping JIT provisioning test: DATABASE_URL is not set \
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
    let slug = format!("auth2-{}", id.as_uuid().simple());
    tenants::create(&pool, id, &slug, "AUTH-2 test tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((pool, id, url))
}

/// Seeds acme-org → eng (dept) → platform (team) on the RLS-exempt test
/// connection. Returns (org, eng, platform).
async fn seed_hierarchy(
    pool: &PgPool,
    tenant: TenantId,
) -> (HierarchyNode, HierarchyNode, HierarchyNode) {
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
    tx.commit().await.expect("commit hierarchy");
    (org, eng, platform)
}

async fn status_and_kind(response: Response) -> (StatusCode, String) {
    let status = response.status();
    let body = body_json(response).await;
    (status, body["kind"].as_str().unwrap_or_default().to_owned())
}

// ── AC 1: the mapped first login ─────────────────────────────────────────────

#[tokio::test]
async fn first_login_lands_in_the_correct_team_scope_with_zero_admin_action() {
    let _serial = serial().await;
    let Some((pool, tenant_id, db_url)) = admitted_tenant().await else {
        return;
    };
    let (_, _, platform) = seed_hierarchy(&pool, tenant_id).await;
    let idp = MockIdp::spawn(tenant_id).await;
    let app = router(state(&db_url, &idp.issuer));

    idp.login_as(
        "alice-sub",
        &["everyone", "synveda-eng-platform"],
        Some("alice@example.test"),
    );
    let session = login(&app).await;

    // The session says where provisioning placed her.
    assert_eq!(session["identity"]["quarantined"], false, "{session}");
    let scope_path = session["identity"]["scope_path"].as_str().expect("path");
    assert!(
        scope_path.starts_with("acme/eng/platform/alice-"),
        "alice must land under the platform team, got {scope_path}"
    );

    // The store agrees: her personal user node hangs off the team, and her
    // identity binds to it.
    let identity = identities::by_subject(&pool, tenant_id, "alice-sub")
        .await
        .expect("read identity")
        .expect("alice is provisioned");
    assert!(!identity.quarantined);
    assert_eq!(identity.email.as_deref(), Some("alice@example.test"));
    let personal = hierarchy::node(&pool, identity.scope_id)
        .await
        .expect("read personal scope")
        .expect("personal scope exists");
    assert_eq!(personal.parent_id, Some(platform.id));
    assert_eq!(personal.kind, ScopeKind::User);

    // Her session bearer keeps its read rights (not quarantined).
    let token = session["access_token"].as_str().expect("access_token");
    let response = app
        .clone()
        .oneshot(get_request("/v1/hierarchy/root", Some(token)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "a placed user can read");

    // Repeat login: same identity, no second personal scope.
    idp.login_as(
        "alice-sub",
        &["everyone", "synveda-eng-platform"],
        Some("alice@example.test"),
    );
    let second = login(&app).await;
    assert_eq!(
        second["identity"]["id"], session["identity"]["id"],
        "a repeat login must adopt the existing identity"
    );
    let children = hierarchy::children(&pool, platform.id)
        .await
        .expect("list team children");
    assert_eq!(
        children.len(),
        1,
        "exactly one personal scope after two logins: {children:?}"
    );

    // The metric contract: a mapped provision and an existing hit.
    let exposition = metrics_handle().render();
    for outcome in ["mapped", "existing"] {
        assert!(
            exposition
                .lines()
                .any(|line| line.starts_with("synveda_jit_provisions_total")
                    && line.contains(&format!("outcome=\"{outcome}\""))),
            "outcome {outcome} missing from exposition:\n{exposition}"
        );
    }
}

// ── AC 2: the unmapped first login ───────────────────────────────────────────

#[tokio::test]
async fn unmapped_login_lands_in_quarantine_with_no_read_rights() {
    let _serial = serial().await;
    let Some((pool, tenant_id, db_url)) = admitted_tenant().await else {
        return;
    };
    seed_hierarchy(&pool, tenant_id).await;
    let idp = MockIdp::spawn(tenant_id).await;
    let app = router(state(&db_url, &idp.issuer));

    idp.login_as("bob-sub", &["everyone", "not-a-synveda-group"], None);
    let session = login(&app).await;

    assert_eq!(session["identity"]["quarantined"], true, "{session}");
    let scope_path = session["identity"]["scope_path"].as_str().expect("path");
    assert!(
        scope_path.starts_with("acme/quarantine/"),
        "bob must land under the reserved quarantine scope, got {scope_path}"
    );

    // "No read rights": the PDP forbids reads (and everything else) — a
    // 403 policy denial, not a 401; he is authenticated, just contained.
    let token = session["access_token"].as_str().expect("access_token");
    let (status, kind) = status_and_kind(
        app.clone()
            .oneshot(get_request("/v1/hierarchy/root", Some(token)))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        (status, kind.as_str()),
        (StatusCode::FORBIDDEN, "policy_denied"),
        "a quarantined user must be policy-denied reads"
    );

    // Writes too — quarantine has no carve-outs.
    let create = Request::post("/v1/hierarchy/nodes")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"parent_id":null,"kind":"org","slug":"rogue","name":"Rogue"}"#,
        ))
        .unwrap();
    let (status, kind) = status_and_kind(app.clone().oneshot(create).await.unwrap()).await;
    assert_eq!(
        (status, kind.as_str()),
        (StatusCode::FORBIDDEN, "policy_denied")
    );

    // whoami still works: introspection of his own resolution, no governed
    // assets involved (ADR-0008).
    let response = app
        .oneshot(get_request("/v1/whoami", Some(token)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// ── Override table, zero-config root, fail-closed bearer ────────────────────

#[tokio::test]
async fn an_override_mapping_beats_the_convention() {
    let _serial = serial().await;
    let Some((pool, tenant_id, db_url)) = admitted_tenant().await else {
        return;
    };
    let (_, eng, _) = seed_hierarchy(&pool, tenant_id).await;
    group_mappings::upsert(&pool, tenant_id, "vendors", eng.id)
        .await
        .expect("create override");
    let idp = MockIdp::spawn(tenant_id).await;
    let app = router(state(&db_url, &idp.issuer));

    // Carol matches both an override (vendors → eng) and the convention
    // (synveda-eng-platform → platform team): the override wins.
    idp.login_as("carol-sub", &["vendors", "synveda-eng-platform"], None);
    let session = login(&app).await;
    assert_eq!(session["identity"]["quarantined"], false);
    let identity = identities::by_subject(&pool, tenant_id, "carol-sub")
        .await
        .expect("read identity")
        .expect("carol is provisioned");
    let personal = hierarchy::node(&pool, identity.scope_id)
        .await
        .expect("read personal scope")
        .expect("personal scope exists");
    assert_eq!(
        personal.parent_id,
        Some(eng.id),
        "the override target, not the convention team"
    );
}

#[tokio::test]
async fn a_fresh_tenant_needs_no_admin_before_the_first_login() {
    let _serial = serial().await;
    let Some((pool, tenant_id, db_url)) = admitted_tenant().await else {
        return;
    };
    // No hierarchy at all: the org root and quarantine scope are created
    // by provisioning itself (seed §2.1 zero-config, ADR-0013 decision 4).
    let idp = MockIdp::spawn(tenant_id).await;
    let app = router(state(&db_url, &idp.issuer));

    idp.login_as("eve-sub", &[], None);
    let session = login(&app).await;
    assert_eq!(session["identity"]["quarantined"], true);

    let root = hierarchy::root(&pool, tenant_id)
        .await
        .expect("read root")
        .expect("provisioning must have created the org root");
    assert_eq!(root.slug, session["tenant"]["slug"].as_str().unwrap());
    let quarantine = hierarchy::child_by_slug(&pool, root.id, identities::QUARANTINE_SLUG)
        .await
        .expect("read quarantine")
        .expect("provisioning must have created the quarantine scope");
    assert_eq!(quarantine.kind, ScopeKind::Team);
}

#[tokio::test]
async fn an_idp_bearer_that_skipped_login_is_quarantined_fail_closed() {
    let _serial = serial().await;
    let Some((pool, tenant_id, db_url)) = admitted_tenant().await else {
        return;
    };
    seed_hierarchy(&pool, tenant_id).await;
    let idp = MockIdp::spawn(tenant_id).await;
    let app = router(state(&db_url, &idp.issuer));

    // Dave never completes /auth/login — he takes his IdP-minted access
    // token straight to the API. Skipping provisioning must not
    // out-privilege completing it (ADR-0013 decision 6).
    idp.login_as("dave-sub", &["synveda-eng-platform"], None);
    let token = idp.access_token();
    let (status, kind) = status_and_kind(
        app.clone()
            .oneshot(get_request("/v1/hierarchy/root", Some(&token)))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        (status, kind.as_str()),
        (StatusCode::FORBIDDEN, "policy_denied"),
        "an unprovisioned IdP subject must be treated as quarantined"
    );
    assert_eq!(
        identities::by_subject(&pool, tenant_id, "dave-sub")
            .await
            .expect("read identity"),
        None,
        "the bearer path never provisions (ADR-0013 decision 2)"
    );
}
