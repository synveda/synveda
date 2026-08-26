//! ADPT-1's login half (ADR-0027 decisions 5 and 6): `synveda login` is a
//! gateway-mediated loopback flow, so the gateway grows exactly three
//! small surfaces — a `cli_redirect_uri` on `/auth/login`, a one-time
//! handoff code on the callback, and `POST /auth/cli/exchange` /
//! `POST /auth/refresh`.
//!
//! What is asserted here is the security shape of those three, over the
//! same in-process mock IdP the AUTH-1 suite uses: the redirect allowlist
//! is absolute, the handoff code is single-use, state-bound, and carries
//! no token in a URL, the refresh token reaches the CLI and nothing else,
//! and a login that fails still lands back in the terminal rather than
//! hanging it. The AC's own timed run is the demo script.
//!
//! Tests that need a live Postgres read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database), same convention as
//! tests/oidc_login.rs.

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
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::{LoginFlow, OidcVerifier, parse_issuers};
use synveda_types::{TenantId, TenantStatus};
use tower::ServiceExt;

const KEY_PEM: &str = include_str!("fixtures/idp_key_a.pem");
const KEY_JWK: &str = include_str!("fixtures/idp_key_a.jwk.json");

const CLIENT_ID: &str = "synveda-test";
const SUBJECT: &str = "alice@example.test";
const REDIRECT_URI: &str = "http://gateway.test/auth/callback";
const LOOPBACK: &str = "http://127.0.0.1:54321/callback";
const CLI_STATE: &str = "cli-state-0123456789";

const UNREACHABLE_URL: &str = "postgres://nobody:nothing@127.0.0.1:1/void";

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

// ── The mock IdP ─────────────────────────────────────────────────────────────

struct AuthCode {
    nonce: String,
    code_challenge: String,
    client_id: String,
    redirect_uri: String,
    scope: String,
}

/// The AUTH-1 mock IdP, plus what ADR-0027 decision 6 needs of it:
/// `scopes_supported` in discovery, a refresh token when `offline_access`
/// was requested, and a refresh grant that rotates.
#[derive(Clone)]
struct MockIdp {
    issuer: String,
    codes: Arc<Mutex<HashMap<String, AuthCode>>>,
    /// Live refresh tokens → the number of times each has been redeemed.
    refresh_tokens: Arc<Mutex<HashMap<String, usize>>>,
    next_code: Arc<AtomicUsize>,
    next_refresh: Arc<AtomicUsize>,
    /// Whether discovery advertises `offline_access` at all.
    offline_access: bool,
    tid: String,
}

impl MockIdp {
    async fn spawn(tid: TenantId, offline_access: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock idp");
        let addr = listener.local_addr().expect("mock idp addr");
        let idp = Self {
            issuer: format!("http://{addr}/mock-entra/v2.0"),
            codes: Arc::new(Mutex::new(HashMap::new())),
            refresh_tokens: Arc::new(Mutex::new(HashMap::new())),
            next_code: Arc::new(AtomicUsize::new(0)),
            next_refresh: Arc::new(AtomicUsize::new(0)),
            offline_access,
            tid: tid.to_string(),
        };
        let app = Router::new()
            .route(
                "/mock-entra/v2.0/.well-known/openid-configuration",
                get(discovery),
            )
            .route("/mock-entra/v2.0/jwks", get(jwks_endpoint))
            .route("/mock-entra/v2.0/authorize", get(authorize))
            .route("/mock-entra/v2.0/token", post(token_endpoint))
            .with_state(idp.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("mock idp serve");
        });
        idp
    }

    fn sign(&self, claims: &Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("key-a".to_owned());
        jsonwebtoken::encode(
            &header,
            claims,
            &EncodingKey::from_rsa_pem(KEY_PEM.as_bytes()).expect("key"),
        )
        .expect("sign token")
    }

    fn access_token(&self, exp_in: i64) -> String {
        self.sign(&json!({
            "iss": self.issuer,
            "sub": SUBJECT,
            "aud": CLIENT_ID,
            "tid": self.tid,
            "exp": now_secs() as i64 + exp_in,
            "iat": now_secs(),
        }))
    }

    fn mint_refresh(&self) -> String {
        let token = format!("rt-{}", self.next_refresh.fetch_add(1, Ordering::SeqCst));
        self.refresh_tokens.lock().unwrap().insert(token.clone(), 0);
        token
    }
}

async fn discovery(State(idp): State<MockIdp>) -> Json<Value> {
    let mut scopes = vec!["openid", "profile", "email"];
    if idp.offline_access {
        scopes.push("offline_access");
    }
    Json(json!({
        "issuer": idp.issuer,
        "authorization_endpoint": format!("{}/authorize", idp.issuer),
        "token_endpoint": format!("{}/token", idp.issuer),
        "jwks_uri": format!("{}/jwks", idp.issuer),
        "scopes_supported": scopes,
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
            code_challenge: query["code_challenge"].clone(),
            client_id: query["client_id"].clone(),
            redirect_uri: query["redirect_uri"].clone(),
            scope: query["scope"].clone(),
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
    let refuse = || {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid_grant" })),
        )
            .into_response()
    };
    match form.get("grant_type").map(String::as_str) {
        Some("refresh_token") => {
            let Some(presented) = form.get("refresh_token") else {
                return refuse();
            };
            // A rotating issuer: the presented token dies, a new one is
            // issued. Redeeming a dead one is a hard refusal.
            let mut live = idp.refresh_tokens.lock().unwrap();
            if live.remove(presented).is_none() {
                return refuse();
            }
            drop(live);
            Json(json!({
                "access_token": idp.access_token(600),
                "refresh_token": idp.mint_refresh(),
                "token_type": "Bearer",
                "expires_in": 600,
            }))
            .into_response()
        }
        Some("authorization_code") => {
            let Some(code) = form.get("code") else {
                return refuse();
            };
            let Some(auth) = idp.codes.lock().unwrap().remove(code) else {
                return refuse();
            };
            let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(
                form.get("code_verifier")
                    .map(String::as_str)
                    .unwrap_or("")
                    .as_bytes(),
            ));
            if form.get("client_id") != Some(&auth.client_id)
                || form.get("redirect_uri") != Some(&auth.redirect_uri)
                || challenge != auth.code_challenge
            {
                return refuse();
            }
            let id_token = idp.sign(&json!({
                "iss": idp.issuer,
                "sub": SUBJECT,
                "aud": auth.client_id,
                "tid": idp.tid,
                "exp": now_secs() + 600,
                "iat": now_secs(),
                "nonce": auth.nonce,
            }));
            let mut body = json!({
                "access_token": idp.access_token(600),
                "id_token": id_token,
                "token_type": "Bearer",
                "expires_in": 600,
            });
            // Only a login that asked for it gets one — which is what
            // makes "requested where advertised" observable from here.
            if auth.scope.split(' ').any(|scope| scope == "offline_access") {
                body["refresh_token"] = json!(idp.mint_refresh());
            }
            Json(body).into_response()
        }
        _ => refuse(),
    }
}

// ── Gateway harness ──────────────────────────────────────────────────────────

fn pool(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(2))
        .connect_lazy(url)
        .expect("parse database url")
}

fn oidc_state(url: &str, issuer: &str) -> AppState {
    let config = format!(r#"[{{"issuer":"{issuer}","client_id":"{CLIENT_ID}"}}]"#);
    let verifier = Arc::new(
        OidcVerifier::new(parse_issuers(&config).expect("issuer config"))
            .expect("build verifier")
            .with_refresh_min_interval(Duration::ZERO),
    );
    AppState {
        pool: pool(url),
        metrics: metrics_handle(),
        verifier: verifier.clone(),
        login: Some(Arc::new(LoginFlow::new(verifier, REDIRECT_URI.to_owned()))),
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp: Arc::new(synveda_policy::Pdp::new().expect("build the embedded PDP")),
        service_token_max_ttl: Duration::from_secs(3600),
        embedder: Arc::new(synveda_ingest::embedding::AnyEmbedder::Deterministic(
            synveda_ingest::embedding::DeterministicEmbedder::new(),
        )),
        context_embed_timeout: Duration::from_millis(100),
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

fn post_json(uri: &str, body: &Value) -> Request<Body> {
    Request::post(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn location(response: &Response) -> String {
    response.headers()[header::LOCATION]
        .to_str()
        .expect("location header")
        .to_owned()
}

fn query_of(url: &str) -> HashMap<String, String> {
    url::Url::parse(url)
        .expect("url")
        .query_pairs()
        .into_owned()
        .collect()
}

/// Drives a CLI login as `synveda login` would: the gateway's `/auth/login`
/// with a loopback return address, the browser hop to the IdP, and the
/// gateway callback. Returns the loopback URL the gateway redirected to.
async fn drive_cli_login(app: &Router, redirect_uri: &str, cli_state: &str) -> String {
    let start = format!(
        "/auth/login?cli_redirect_uri={}&cli_state={cli_state}",
        urlencoding(redirect_uri)
    );
    let response = app
        .clone()
        .oneshot(get_request(&start, None))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::TEMPORARY_REDIRECT,
        "CLI login must start like any other"
    );
    let authorize = location(&response);

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let idp_response = client.get(&authorize).send().await.expect("authorize hop");
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
    assert_eq!(
        response.status(),
        StatusCode::TEMPORARY_REDIRECT,
        "a CLI login must 302 back to the loopback, not answer with JSON"
    );
    location(&response)
}

fn urlencoding(value: &str) -> String {
    value
        .replace(':', "%3A")
        .replace('/', "%2F")
        .replace('?', "%3F")
        .replace('&', "%26")
}

async fn admitted_tenant() -> Option<(String, TenantId)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping ADPT-1 CLI login DB test: DATABASE_URL is not set \
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
    let slug = format!("adpt1-{}", id.as_uuid().simple());
    synveda_store::tenants::create(&pool, id, &slug, "ADPT-1 test tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((url, id))
}

// ── The flow, end to end (needs a database) ─────────────────────────────────

#[tokio::test]
async fn a_cli_login_hands_back_a_code_that_redeems_a_usable_session() {
    let _serial = serial().await;
    let Some((db_url, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let idp = MockIdp::spawn(tenant_id, true).await;
    let app = router(oidc_state(&db_url, &idp.issuer));

    let loopback = drive_cli_login(&app, LOOPBACK, CLI_STATE).await;
    assert!(
        loopback.starts_with(LOOPBACK),
        "the redirect must go to the CLI's own listener: {loopback}"
    );
    let handed = query_of(&loopback);
    assert_eq!(
        handed["state"], CLI_STATE,
        "the CLI's state must round-trip"
    );
    let code = handed["code"].clone();

    // Decision 5's whole point: what reaches the loopback — and therefore
    // the browser history — is a code, never a credential.
    assert!(!loopback.contains("access_token"), "{loopback}");
    assert!(!loopback.contains("refresh_token"), "{loopback}");
    assert!(
        !loopback.contains("eyJ"),
        "no JWT may appear in a URL: {loopback}"
    );

    let response = app
        .clone()
        .oneshot(post_json(
            "/auth/cli/exchange",
            &json!({ "code": code, "state": CLI_STATE }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let session = body_json(response).await;
    assert_eq!(session["subject"], SUBJECT, "session: {session}");
    assert_eq!(session["tenant"]["id"], tenant_id.to_string());
    assert_eq!(session["issuer"], idp.issuer, "the CLI needs it to refresh");
    assert!(
        session["refresh_token"].is_string(),
        "an advertising issuer must yield a refresh token: {session}"
    );

    // "A usable session": the exchanged token IS the /v1 bearer.
    let token = session["access_token"].as_str().expect("access_token");
    let response = app
        .clone()
        .oneshot(get_request("/v1/whoami", Some(token)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["subject"], SUBJECT);

    // And the refresh keeps it usable without a second browser round-trip.
    let refresh_token = session["refresh_token"].as_str().expect("refresh_token");
    let response = app
        .clone()
        .oneshot(post_json(
            "/auth/refresh",
            &json!({ "refresh_token": refresh_token, "issuer": idp.issuer }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let refreshed = body_json(response).await;
    let renewed = refreshed["access_token"].as_str().expect("access_token");
    assert!(
        refreshed["refresh_token"].is_string(),
        "a rotating issuer's new refresh token must reach the CLI: {refreshed}"
    );
    let response = app
        .oneshot(get_request("/v1/whoami", Some(renewed)))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the renewed bearer works"
    );
}

#[tokio::test]
async fn a_browser_login_never_receives_a_refresh_token() {
    let _serial = serial().await;
    let Some((db_url, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let idp = MockIdp::spawn(tenant_id, true).await;
    let app = router(oidc_state(&db_url, &idp.issuer));

    // No cli_redirect_uri: AUTH-1's original path, unchanged.
    let response = app
        .clone()
        .oneshot(get_request("/auth/login", None))
        .await
        .unwrap();
    let authorize = location(&response);
    let scopes = query_of(&authorize)["scope"].clone();
    assert!(
        !scopes.contains("offline_access"),
        "a browser login must not ask for a refresh token it cannot use: {scopes}"
    );

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let hop = client.get(&authorize).send().await.expect("authorize hop");
    let callback = url::Url::parse(hop.headers()[reqwest::header::LOCATION].to_str().unwrap())
        .expect("callback url");
    let response = app
        .oneshot(get_request(
            &format!("/auth/callback?{}", callback.query().expect("query")),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "browser logins read JSON"
    );
    let session = body_json(response).await;
    assert!(session["access_token"].is_string());
    assert!(
        session.get("refresh_token").is_none(),
        "the browser-facing response is structurally incapable of carrying one: {session}"
    );
}

#[tokio::test]
async fn offline_access_is_requested_only_where_the_issuer_advertises_it() {
    let _serial = serial().await;
    let tenant = TenantId::new();

    // Advertised: the CLI login asks for it.
    let idp = MockIdp::spawn(tenant, true).await;
    let app = router(oidc_state(UNREACHABLE_URL, &idp.issuer));
    let response = app
        .oneshot(get_request(
            &format!(
                "/auth/login?cli_redirect_uri={}&cli_state={CLI_STATE}",
                urlencoding(LOOPBACK)
            ),
            None,
        ))
        .await
        .unwrap();
    let scopes = query_of(&location(&response))["scope"].clone();
    assert!(scopes.contains("offline_access"), "scopes: {scopes}");

    // Not advertised: asking anyway is how logins break at IdPs that
    // reject unknown scopes, so the flow does not.
    let quiet = MockIdp::spawn(tenant, false).await;
    let app = router(oidc_state(UNREACHABLE_URL, &quiet.issuer));
    let response = app
        .oneshot(get_request(
            &format!(
                "/auth/login?cli_redirect_uri={}&cli_state={CLI_STATE}",
                urlencoding(LOOPBACK)
            ),
            None,
        ))
        .await
        .unwrap();
    let scopes = query_of(&location(&response))["scope"].clone();
    assert!(!scopes.contains("offline_access"), "scopes: {scopes}");
}

// ── The allowlist and the code's contract (no database) ─────────────────────

#[tokio::test]
async fn only_literal_loopback_callback_uris_are_accepted() {
    let _serial = serial().await;
    let idp = MockIdp::spawn(TenantId::new(), true).await;
    let app = router(oidc_state(UNREACHABLE_URL, &idp.issuer));

    for target in [
        "http://evil.test/callback",
        // The name resolves; the literal does not. That is the whole
        // reason the allowlist names addresses.
        "http://localhost:8080/callback",
        "https://127.0.0.1:8080/callback",
        "http://127.0.0.2:8080/callback",
        "http://127.0.0.1:8080/other",
        "http://127.0.0.1:8080/callback?next=http://evil.test",
        "http://user:pass@127.0.0.1:8080/callback",
        "file:///callback",
        "not-a-url",
    ] {
        let response = app
            .clone()
            .oneshot(get_request(
                &format!(
                    "/auth/login?cli_redirect_uri={}&cli_state={CLI_STATE}",
                    urlencoding(target)
                ),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{target} must never be an accepted handoff target"
        );
    }

    // The two halves are given together or not at all: a redirect with no
    // state would be a login the CLI cannot tell from anyone else's.
    for partial in [
        format!("/auth/login?cli_redirect_uri={}", urlencoding(LOOPBACK)),
        format!("/auth/login?cli_state={CLI_STATE}"),
    ] {
        let response = app
            .clone()
            .oneshot(get_request(&partial, None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{partial}");
    }
}

#[tokio::test]
async fn a_handoff_code_is_single_use_and_bound_to_the_cli_state() {
    let _serial = serial().await;
    let Some((db_url, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let idp = MockIdp::spawn(tenant_id, true).await;
    let app = router(oidc_state(&db_url, &idp.issuer));

    // A code redeemed with the wrong state buys nothing — and burns.
    let loopback = drive_cli_login(&app, LOOPBACK, CLI_STATE).await;
    let code = query_of(&loopback)["code"].clone();
    let response = app
        .clone()
        .oneshot(post_json(
            "/auth/cli/exchange",
            &json!({ "code": code, "state": "not-the-cli-state" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = app
        .clone()
        .oneshot(post_json(
            "/auth/cli/exchange",
            &json!({ "code": code, "state": CLI_STATE }),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "a failed redemption consumes the code; it is not a free retry"
    );

    // A fresh code redeems exactly once.
    let loopback = drive_cli_login(&app, LOOPBACK, CLI_STATE).await;
    let code = query_of(&loopback)["code"].clone();
    let exchange = json!({ "code": code, "state": CLI_STATE });
    let response = app
        .clone()
        .oneshot(post_json("/auth/cli/exchange", &exchange))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = app
        .clone()
        .oneshot(post_json("/auth/cli/exchange", &exchange))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "single use");

    // A code the gateway never issued is the same uniform 401.
    let response = app
        .oneshot(post_json(
            "/auth/cli/exchange",
            &json!({ "code": "never-issued", "state": CLI_STATE }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_failed_login_lands_back_in_the_terminal_rather_than_hanging_it() {
    let _serial = serial().await;
    let idp = MockIdp::spawn(TenantId::new(), true).await;
    // Unreachable storage: the tenant lookup fails after the IdP said yes,
    // which is exactly the shape of "something went wrong late".
    let app = router(oidc_state(UNREACHABLE_URL, &idp.issuer));

    let loopback = drive_cli_login(&app, LOOPBACK, CLI_STATE).await;
    let reported = query_of(&loopback);
    assert!(loopback.starts_with(LOOPBACK), "{loopback}");
    assert_eq!(reported["state"], CLI_STATE);
    assert_eq!(reported["error"], "login_failed");
    assert!(
        reported["error_description"].contains("storage"),
        "the CLI must be told what went wrong: {loopback}"
    );
    assert!(!reported.contains_key("code"), "{loopback}");

    // A user who declines at the IdP is the same story: the terminal
    // hears about it.
    let response = app
        .clone()
        .oneshot(get_request(
            &format!(
                "/auth/login?cli_redirect_uri={}&cli_state={CLI_STATE}",
                urlencoding(LOOPBACK)
            ),
            None,
        ))
        .await
        .unwrap();
    let login_state = query_of(&location(&response))["state"].clone();
    let response = app
        .oneshot(get_request(
            &format!("/auth/callback?error=access_denied&state={login_state}"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    let denied = query_of(&location(&response));
    assert_eq!(denied["error"], "login_failed");
    assert_eq!(denied["state"], CLI_STATE);
}

#[tokio::test]
async fn a_revoked_refresh_token_is_the_uniform_401() {
    let _serial = serial().await;
    let idp = MockIdp::spawn(TenantId::new(), true).await;
    let app = router(oidc_state(UNREACHABLE_URL, &idp.issuer));

    for body in [
        json!({ "refresh_token": "never-issued", "issuer": idp.issuer }),
        // No issuer named: with exactly one configured, that is allowed,
        // and the token is still refused.
        json!({ "refresh_token": "never-issued" }),
    ] {
        let response = app
            .clone()
            .oneshot(post_json("/auth/refresh", &body))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "a refresh the IdP refuses is a 401, not a 500"
        );
    }

    // An issuer this gateway does not trust is a caller error, and the
    // refusal names no endpoint.
    let response = app
        .oneshot(post_json(
            "/auth/refresh",
            &json!({ "refresh_token": "x", "issuer": "http://evil.test" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_cli_auth_plane_is_404_without_oidc() {
    let _serial = serial().await;
    let state = AppState {
        verifier: Arc::new(synveda_identity::DisabledVerifier),
        login: None,
        public_origin: "http://127.0.0.1:8120".to_owned(),
        ..oidc_state(UNREACHABLE_URL, "http://unused.test")
    };
    for (uri, body) in [
        ("/auth/cli/exchange", json!({ "code": "c", "state": "s" })),
        ("/auth/refresh", json!({ "refresh_token": "r" })),
    ] {
        let response = router(state.clone())
            .oneshot(post_json(uri, &body))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "{uri} must 404 without OIDC, like the rest of the auth plane"
        );
    }
}
