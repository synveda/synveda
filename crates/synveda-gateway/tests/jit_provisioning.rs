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
use synveda_store::{identities, scopes, tenants};
use synveda_types::scope::ScopeKind;
use synveda_types::{GrantId, TenantId, TenantStatus};
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
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp: Arc::new(synveda_policy::Pdp::new().expect("build the embedded PDP")),
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

async fn status_and_kind(response: Response) -> (StatusCode, String) {
    let status = response.status();
    let body = body_json(response).await;
    (status, body["kind"].as_str().unwrap_or_default().to_owned())
}

// ── AC 1: the first login ────────────────────────────────────────────────────

#[tokio::test]
async fn first_login_mints_the_identity_and_its_own_scope_with_zero_admin_action() {
    let _serial = serial().await;
    let Some((pool, tenant_id, db_url)) = admitted_tenant().await else {
        return;
    };
    let idp = MockIdp::spawn(tenant_id).await;
    let app = router(state(&db_url, &idp.issuer));

    idp.login_as(
        "alice-sub",
        &["everyone", "synveda-eng-platform"],
        Some("alice@example.test"),
    );
    let session = login(&app).await;

    // The session reports her own scope — the placement identity itself
    // carries (CPR-7, ADR-0074 decision 3).
    let scope_path = session["identity"]["scope_path"].as_str().expect("path");
    // A *path*, rooted at the tenant, whose last segment is her own
    // principal slug — the login response promises a chain, not a slug.
    let (tenant_slug, own) = scope_path.split_once('/').expect("a rooted path");
    assert!(
        !tenant_slug.is_empty(),
        "rooted at the tenant: {scope_path}"
    );
    assert!(
        own.starts_with("p-"),
        "alice's scope is her own principal scope, got {scope_path}"
    );

    // The store agrees: a principal-shaped scope hanging at the tenant
    // root, with her identity bound to it.
    let identity = identities::by_subject(&pool, tenant_id, "alice-sub")
        .await
        .expect("read identity")
        .expect("alice is provisioned");
    assert_eq!(identity.email.as_deref(), Some("alice@example.test"));
    let mut tx = pool.begin().await.expect("begin");
    let personal = scopes::get(&mut *tx, tenant_id, identity.scope_id)
        .await
        .expect("read personal scope")
        .expect("personal scope exists");
    let root = scopes::ensure_tenant_root(&mut tx, tenant_id)
        .await
        .expect("mint root");
    tx.commit().await.expect("commit");
    assert_eq!(personal.parent_scope_id, Some(root.id));
    assert_eq!(personal.kind, ScopeKind::Principal);

    // Her session bearer resolves and reaches the PDP — but an ungranted
    // member holds no admin-plane read: a policy denial, not an
    // authentication failure.
    let token = session["access_token"].as_str().expect("access_token");
    let (status, kind) = status_and_kind(
        app.clone()
            .oneshot(get_request("/v1/admin/scopes", Some(token)))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        (status, kind.as_str()),
        (StatusCode::FORBIDDEN, "policy_denied"),
        "an ungranted member holds no admin-plane read"
    );

    // Granted administrator through the store (the access API has its own
    // suite), the same bearer reads on the very next request: the grant,
    // not the placement, carries the admin plane.
    let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant_id)
        .await
        .expect("begin tenant tx");
    synveda_store::access::create_grant(
        &mut *tx,
        &synveda_store::access::NewGrant {
            id: GrantId::new(),
            tenant_id,
            scope_id: root.id,
            subject: synveda_types::access::GrantSubject::Principal {
                principal_id: "alice-sub".to_owned(),
            },
            role_key: synveda_types::access::RoleKey::Administrator,
            source: synveda_types::access::GrantSource::Automation,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .expect("grant administrator at the root");
    tx.commit().await.expect("commit grant");
    let response = app
        .clone()
        .oneshot(get_request("/v1/admin/scopes", Some(token)))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the administrator grant carries the read on the next request"
    );

    // Repeat login: same identity, same scope — "unmapped" never meant a
    // second node, and adoption is keyed by subject.
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
    assert_eq!(
        second["identity"]["scope_id"], session["identity"]["scope_id"],
        "a repeat login must adopt the existing scope"
    );

    // The metric contract: a first login and an adopted one.
    let exposition = metrics_handle().render();
    for outcome in ["own-scope", "bound"] {
        assert!(
            exposition
                .lines()
                .any(|line| line.starts_with("synveda_jit_provisions_total")
                    && line.contains(&format!("outcome=\"{outcome}\""))),
            "outcome {outcome} missing from exposition:\n{exposition}"
        );
    }
}

// ── AC 2: the ungranted first login ──────────────────────────────────────────

#[tokio::test]
async fn an_ungranted_login_reaches_nothing_beyond_its_own_scope() {
    let _serial = serial().await;
    let Some((pool, tenant_id, db_url)) = admitted_tenant().await else {
        return;
    };
    let idp = MockIdp::spawn(tenant_id).await;
    let app = router(state(&db_url, &idp.issuer));

    idp.login_as("bob-sub", &["everyone", "not-a-synveda-group"], None);
    let session = login(&app).await;

    let scope_path = session["identity"]["scope_path"].as_str().expect("path");
    let (tenant_slug, own) = scope_path.split_once('/').expect("a rooted path");
    assert!(
        !tenant_slug.is_empty(),
        "rooted at the tenant: {scope_path}"
    );
    assert!(
        own.starts_with("p-"),
        "bob's scope is his own principal scope, got {scope_path}"
    );

    // "No read rights": the PDP forbids reads — a 403 policy denial, not
    // a 401; he is authenticated, just ungranted.
    let token = session["access_token"].as_str().expect("access_token");
    let (status, kind) = status_and_kind(
        app.clone()
            .oneshot(get_request("/v1/admin/scopes", Some(token)))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        (status, kind.as_str()),
        (StatusCode::FORBIDDEN, "policy_denied"),
        "an ungranted user must be policy-denied reads"
    );

    // Writes too — being a principal holds no carve-outs. The parent is
    // the real tenant root, so the ownership check passes and the PDP is
    // what says no (a made-up parent would be a 404 before the decision).
    let mut tx = pool.begin().await.expect("begin");
    let root = scopes::ensure_tenant_root(&mut tx, tenant_id)
        .await
        .expect("mint root");
    tx.commit().await.expect("commit");
    let create = Request::post("/v1/admin/scopes")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        // The admin create route takes the key every governed create does
        // (CPR-4's shape); an ungranted caller never gets past the PDP, but
        // the request has to be well-formed enough to reach it.
        .header("idempotency-key", "auth2-ungranted-create")
        .body(Body::from(
            json!({
                "parent_id": root.id, "kind": "org_unit",
                "slug": "rogue", "display_name": "Rogue"
            })
            .to_string(),
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

// ── Zero-config root, fail-closed bearer ─────────────────────────────────────

#[tokio::test]
async fn a_fresh_tenant_needs_no_admin_before_the_first_login() {
    let _serial = serial().await;
    let Some((pool, tenant_id, db_url)) = admitted_tenant().await else {
        return;
    };
    // No scopes at all: the tenant root and the login's own principal
    // scope are created by provisioning itself (seed §2.1 zero-config,
    // ADR-0013 decision 4).
    let idp = MockIdp::spawn(tenant_id).await;
    let app = router(state(&db_url, &idp.issuer));

    idp.login_as("eve-sub", &[], None);
    let session = login(&app).await;

    let mut tx = pool.begin().await.expect("begin");
    let root = scopes::ensure_tenant_root(&mut tx, tenant_id)
        .await
        .expect("read root");
    let identity = identities::by_subject(&mut *tx, tenant_id, "eve-sub")
        .await
        .expect("read identity")
        .expect("eve is provisioned");
    let personal = scopes::get(&mut *tx, tenant_id, identity.scope_id)
        .await
        .expect("read personal scope")
        .expect("personal scope exists");
    tx.commit().await.expect("commit");
    assert_eq!(
        root.slug,
        session["tenant"]["slug"].as_str().unwrap(),
        "the root carries the tenant's slug"
    );
    assert_eq!(personal.kind, ScopeKind::Principal);
    assert_eq!(personal.parent_scope_id, Some(root.id));
}

#[tokio::test]
async fn an_idp_bearer_that_skipped_login_is_refused_fail_closed() {
    let _serial = serial().await;
    let Some((pool, tenant_id, db_url)) = admitted_tenant().await else {
        return;
    };
    let idp = MockIdp::spawn(tenant_id).await;
    let app = router(state(&db_url, &idp.issuer));

    // Dave never completes /auth/login — he takes his IdP-minted access
    // token straight to the API. Skipping provisioning must not
    // out-privilege completing it (ADR-0013 decision 6).
    idp.login_as("dave-sub", &["synveda-eng-platform"], None);
    let token = idp.access_token();
    let (status, kind) = status_and_kind(
        app.clone()
            .oneshot(get_request("/v1/admin/scopes", Some(&token)))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        (status, kind.as_str()),
        (StatusCode::FORBIDDEN, "policy_denied"),
        "an unprovisioned IdP subject holds nothing"
    );
    assert_eq!(
        identities::by_subject(&pool, tenant_id, "dave-sub")
            .await
            .expect("read identity"),
        None,
        "the bearer path never provisions (ADR-0013 decision 2)"
    );
}

// ── AUTHZ-3: the admin door (ADR-0074 decision 4) ────────────────────────────

/// Zero-config bootstrap: on a fresh tenant with no scopes and no admin
/// action, the first login of a `synveda-admins` member (matched
/// case-insensitively) gets an `administrator` grant at the tenant root
/// and governs the tenant on the very same bearer.
#[tokio::test]
async fn admin_group_login_bootstraps_a_governable_tenant() {
    let _serial = serial().await;
    let Some((pool, tenant_id, db_url)) = admitted_tenant().await else {
        return;
    };
    let idp = MockIdp::spawn(tenant_id).await;
    let app = router(state(&db_url, &idp.issuer));

    idp.login_as(
        "it-admin",
        &["everyone", "Synveda-Admins"],
        Some("admin@example.test"),
    );
    let session = login(&app).await;

    let mut tx = pool.begin().await.expect("begin");
    let root = scopes::ensure_tenant_root(&mut tx, tenant_id)
        .await
        .expect("read root");
    let identity = identities::by_subject(&mut *tx, tenant_id, "it-admin")
        .await
        .expect("read identity")
        .expect("admin is provisioned");
    let personal = scopes::get(&mut *tx, tenant_id, identity.scope_id)
        .await
        .expect("read personal scope")
        .expect("personal scope exists");
    tx.commit().await.expect("commit");
    assert_eq!(
        personal.parent_scope_id,
        Some(root.id),
        "the admin's own scope hangs at the root like anybody else's"
    );

    // The door is one grant: `administrator` at the tenant root.
    let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant_id)
        .await
        .expect("begin tenant tx");
    let grants = synveda_store::access::list_grants(
        &mut *tx,
        tenant_id,
        &synveda_store::access::GrantFilter {
            scope_id: None,
            principal_id: Some("it-admin".to_owned()),
        },
    )
    .await
    .expect("read grants");
    drop(tx);
    // Exactly two: the admin door at the tenant root, and the `owner`
    // grant every principal scope carries at itself since CPR-7 (ADR-0074
    // decision 8) — the door this test is about, plus the one every
    // principal scope mints regardless of who logged in.
    assert_eq!(
        grants.len(),
        2,
        "the admin door plus the own-scope owner grant: {grants:?}"
    );
    let door = grants
        .iter()
        .find(|grant| grant.scope_id == root.id)
        .expect("the admin door grant is at the tenant root");
    assert_eq!(door.role_key, synveda_types::access::RoleKey::Administrator);

    // The same bearer governs immediately: creating a scope is an
    // administrator action, decided through the PDP.
    let token = session["access_token"].as_str().expect("access_token");
    let request = Request::builder()
        .method("POST")
        .uri("/v1/admin/scopes")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        // The governed-create contract (CPR-4): the key is required, and a
        // replay of it with the same body is the same create.
        .header("Idempotency-Key", "authz3-first-governed-create")
        .body(Body::from(
            json!({
                "parent_id": root.id, "kind": "org_unit",
                "slug": "eng", "display_name": "Engineering"
            })
            .to_string(),
        ))
        .expect("build request");
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "SSO login to governing admin with zero admin action"
    );

    // The metric contract: the first-login outcome.
    let exposition = metrics_handle().render();
    assert!(
        exposition
            .lines()
            .any(|line| line.starts_with("synveda_jit_provisions_total")
                && line.contains("outcome=\"own-scope\"")),
        "own-scope provision outcome missing from exposition:\n{exposition}"
    );
}

/// The door opens for somebody who was **already provisioned** — and the
/// grant survives the transaction.
///
/// The convention is upserted at *every* login completion, so the login
/// that first establishes the grant is very often not a first login: a
/// directory-created identity whose subject is already bound, or anybody
/// added to `synveda-admins` after they joined, reaches the door down the
/// `bound` branch. That branch looks read-only and is not, and a version
/// of it that returned without committing dropped the grant and its
/// `access.granted` event silently, on every such login — leaving the
/// operator door of ADR-0074 decision 4 broken for exactly the population
/// it exists for. This is the test that says so.
#[tokio::test]
async fn the_admin_door_opens_on_a_later_login_and_the_grant_is_committed() {
    let _serial = serial().await;
    let Some((pool, tenant_id, db_url)) = admitted_tenant().await else {
        return;
    };
    let idp = MockIdp::spawn(tenant_id).await;
    let app = router(state(&db_url, &idp.issuer));

    // First login: an ordinary member, no admin group, no admin-door
    // grant. (Their own `owner` grant at their own scope — the one every
    // principal scope carries at itself, ADR-0074 decision 8 — is not the
    // door this test is about, so `admin_door_grants_of` counts only the
    // tenant root.)
    idp.login_as("late-admin", &["everyone"], Some("late@example.test"));
    let session = login(&app).await;
    assert!(session["identity"]["id"].is_string(), "{session}");
    assert_eq!(
        admin_door_grants_of(&pool, tenant_id, "late-admin").await,
        0
    );

    // They are added to `synveda-admins`, and log in again. The identity
    // exists, so this goes down the `bound` branch.
    idp.login_as(
        "late-admin",
        &["everyone", "synveda-admins"],
        Some("late@example.test"),
    );
    let session = login(&app).await;
    assert_eq!(
        admin_door_grants_of(&pool, tenant_id, "late-admin").await,
        1,
        "the admin grant must survive the login that established it"
    );

    // And it decides on the very next request, which is the whole point of
    // the door: the same bearer administers the tenant.
    let token = session["access_token"].as_str().expect("access_token");
    let response = app
        .clone()
        .oneshot(get_request("/v1/admin/scopes", Some(token)))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the committed grant carries the admin plane on the next request"
    );

    // A third login is a no-op upsert, not a second grant.
    let _ = login(&app).await;
    assert_eq!(
        admin_door_grants_of(&pool, tenant_id, "late-admin").await,
        1,
        "the convention is additive and idempotent"
    );

    // The metric names the branch this test is about.
    let exposition = metrics_handle().render();
    assert!(
        exposition
            .lines()
            .any(|line| line.starts_with("synveda_jit_provisions_total")
                && line.contains("outcome=\"bound\"")),
        "bound provision outcome missing from exposition:\n{exposition}"
    );
}

/// How many grants `principal` holds **at the tenant root** — the admin
/// door, deliberately excluding the `owner` grant every principal scope
/// carries at itself (ADR-0074 decision 8), which is not what this test
/// is about.
async fn admin_door_grants_of(pool: &PgPool, tenant_id: TenantId, principal: &str) -> usize {
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant_id)
        .await
        .expect("begin tenant tx");
    let root = synveda_store::scopes::ensure_tenant_root(&mut tx, tenant_id)
        .await
        .expect("read root");
    let grants = synveda_store::access::list_grants(
        &mut *tx,
        tenant_id,
        &synveda_store::access::GrantFilter {
            scope_id: Some(root.id),
            principal_id: Some(principal.to_owned()),
        },
    )
    .await
    .expect("read grants");
    grants.len()
}
