//! AUTH-1 acceptance criteria: OIDC login (code+PKCE) yields a Synveda
//! session. The mock-Entra half of the AC runs here CI-clean — an
//! in-process IdP serving discovery/JWKS/authorize/token, signing RS256
//! with checked-in test keys, Entra-shaped issuer path and `tid` claim —
//! plus the surrounding contract: PKCE S256 challenge on the redirect,
//! single-use callback state, JWKS rotation handling (refetch on unknown
//! kid, rate-limited), and uniform 401s on every doubt. The live-Rauthy
//! half of the AC runs in `demos/auth-1-oidc-login.sh` (ADR-0010 §9).
//!
//! Tests that need a live Postgres read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database), same convention as
//! tests/tenant_resolution.rs.

#[path = "../../synveda-store/tests/support/tenant_fixture.rs"]
mod tenant_fixture;

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
use synveda_gateway::app::{AppState, behavior_test_router as router};
use synveda_gateway::telemetry;
use synveda_identity::{LoginFlow, OidcVerifier, parse_issuers};
use synveda_types::{TenantId, TenantStatus};
use tower::ServiceExt;

const KEY_A_PEM: &str = include_str!("fixtures/idp_key_a.pem");
const KEY_A_JWK: &str = include_str!("fixtures/idp_key_a.jwk.json");
const KEY_B_PEM: &str = include_str!("fixtures/idp_key_b.pem");
const KEY_B_JWK: &str = include_str!("fixtures/idp_key_b.jwk.json");

const CLIENT_ID: &str = "synveda-test";
const SUBJECT: &str = "alice@example.test";
const REDIRECT_URI: &str = "http://gateway.test/auth/callback";

/// A URL that parses but connects nowhere, for tests that must not touch a
/// database.
const UNREACHABLE_URL: &str = "postgres://nobody:nothing@127.0.0.1:1/void";

/// Serialises tests: the Prometheus recorder and tracing's callsite-interest
/// cache are process-global (same rationale as tests/tenant_resolution.rs).
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

struct Signer {
    kid: String,
    key: EncodingKey,
}

struct AuthCode {
    nonce: String,
    code_challenge: String,
    client_id: String,
    redirect_uri: String,
}

/// An in-process OIDC provider with an Entra-shaped issuer path. The test
/// swaps its signing key to exercise rotation and counts JWKS fetches to
/// prove the rate limit.
#[derive(Clone)]
struct MockIdp {
    issuer: String,
    jwks: Arc<Mutex<Value>>,
    signing: Arc<Mutex<Signer>>,
    codes: Arc<Mutex<HashMap<String, AuthCode>>>,
    next_code: Arc<AtomicUsize>,
    jwks_hits: Arc<AtomicUsize>,
    /// The `tid` claim to embed in tokens, when this IdP models the
    /// claim-binding (Entra) shape.
    tid: Option<String>,
}

impl MockIdp {
    async fn spawn(tid: Option<String>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock idp");
        let addr = listener.local_addr().expect("mock idp addr");
        let issuer = format!("http://{addr}/mock-entra/v2.0");
        let idp = Self {
            issuer,
            jwks: Arc::new(Mutex::new(jwks_document(KEY_A_JWK))),
            signing: Arc::new(Mutex::new(Signer {
                kid: "key-a".to_owned(),
                key: EncodingKey::from_rsa_pem(KEY_A_PEM.as_bytes()).expect("key a"),
            })),
            codes: Arc::new(Mutex::new(HashMap::new())),
            next_code: Arc::new(AtomicUsize::new(0)),
            jwks_hits: Arc::new(AtomicUsize::new(0)),
            tid,
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
        let signer = self.signing.lock().unwrap();
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(signer.kid.clone());
        jsonwebtoken::encode(&header, claims, &signer.key).expect("sign token")
    }

    /// A bearer token as this IdP would mint for `/v1` calls.
    fn access_token(&self, aud: &str, exp_in: i64) -> String {
        let mut claims = json!({
            "iss": self.issuer,
            "sub": SUBJECT,
            "aud": aud,
            "exp": now_secs() as i64 + exp_in,
            "iat": now_secs(),
        });
        if let Some(tid) = &self.tid {
            claims["tid"] = json!(tid);
        }
        self.sign(&claims)
    }

    /// Rotates the signing key and the published JWKS to key B.
    fn rotate_to_key_b(&self) {
        *self.signing.lock().unwrap() = Signer {
            kid: "key-b".to_owned(),
            key: EncodingKey::from_rsa_pem(KEY_B_PEM.as_bytes()).expect("key b"),
        };
        *self.jwks.lock().unwrap() = jwks_document(KEY_B_JWK);
    }

    fn jwks_fetches(&self) -> usize {
        self.jwks_hits.load(Ordering::SeqCst)
    }
}

fn jwks_document(jwk: &str) -> Value {
    json!({ "keys": [serde_json::from_str::<Value>(jwk).expect("jwk fixture")] })
}

async fn discovery(State(idp): State<MockIdp>) -> Json<Value> {
    Json(json!({
        "issuer": idp.issuer,
        "authorization_endpoint": format!("{}/authorize", idp.issuer),
        "token_endpoint": format!("{}/token", idp.issuer),
        "jwks_uri": format!("{}/jwks", idp.issuer),
    }))
}

async fn jwks_endpoint(State(idp): State<MockIdp>) -> Json<Value> {
    idp.jwks_hits.fetch_add(1, Ordering::SeqCst);
    Json(idp.jwks.lock().unwrap().clone())
}

async fn authorize(
    State(idp): State<MockIdp>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if query.get("response_type").map(String::as_str) != Some("code")
        || query.get("code_challenge_method").map(String::as_str) != Some("S256")
    {
        return (StatusCode::BAD_REQUEST, "unsupported authorize request").into_response();
    }
    let code = format!("code-{}", idp.next_code.fetch_add(1, Ordering::SeqCst));
    idp.codes.lock().unwrap().insert(
        code.clone(),
        AuthCode {
            nonce: query["nonce"].clone(),
            code_challenge: query["code_challenge"].clone(),
            client_id: query["client_id"].clone(),
            redirect_uri: query["redirect_uri"].clone(),
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
    let Some(code) = form.get("code") else {
        return refuse();
    };
    let Some(auth) = idp.codes.lock().unwrap().remove(code) else {
        return refuse();
    };
    // The mock enforces real PKCE: a gateway that mangles the verifier or
    // the redirect fails here, not silently downstream.
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(
        form.get("code_verifier")
            .map(String::as_str)
            .unwrap_or("")
            .as_bytes(),
    ));
    if form.get("grant_type").map(String::as_str) != Some("authorization_code")
        || form.get("client_id") != Some(&auth.client_id)
        || form.get("redirect_uri") != Some(&auth.redirect_uri)
        || challenge != auth.code_challenge
    {
        return refuse();
    }
    let mut id_claims = json!({
        "iss": idp.issuer,
        "sub": SUBJECT,
        "aud": auth.client_id,
        "exp": now_secs() + 600,
        "iat": now_secs(),
        "nonce": auth.nonce,
    });
    if let Some(tid) = &idp.tid {
        id_claims["tid"] = json!(tid);
    }
    let access_token = idp.access_token(&auth.client_id, 600);
    Json(json!({
        "access_token": access_token,
        "id_token": idp.sign(&id_claims),
        "token_type": "Bearer",
        "expires_in": 600,
    }))
    .into_response()
}

// ── Gateway harness ──────────────────────────────────────────────────────────

enum Binding {
    /// Entra shape: the `tid` claim carries the tenant UUID.
    Claim,
    /// Rauthy/dev shape: every login from this issuer is one tenant.
    Static(TenantId),
}

fn pool(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(2))
        .connect_lazy(url)
        .expect("parse database url")
}

/// Builds the gateway with one configured OIDC issuer. `refresh_interval`
/// tunes the JWKS rate limit (zero = always refetch, huge = never twice).
fn oidc_state(url: &str, issuer: &str, binding: &Binding, refresh_interval: Duration) -> AppState {
    let config = match binding {
        Binding::Claim => {
            format!(r#"[{{"issuer":"{issuer}","client_id":"{CLIENT_ID}"}}]"#)
        }
        Binding::Static(tenant_id) => format!(
            r#"[{{"issuer":"{issuer}","client_id":"{CLIENT_ID}",
                 "tenant":{{"static":{{"tenant_id":"{tenant_id}"}}}}}}]"#
        ),
    };
    let verifier = Arc::new(
        OidcVerifier::new(parse_issuers(&config).expect("issuer config"))
            .expect("build verifier")
            .with_refresh_min_interval(refresh_interval),
    );
    AppState {
        pool: pool(url),
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

async fn body_json(response: axum::response::Response) -> Value {
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

async fn status_and_kind(response: axum::response::Response) -> (StatusCode, String) {
    let status = response.status();
    let body = body_json(response).await;
    (status, body["kind"].as_str().unwrap_or_default().to_owned())
}

/// Drives login as the browser would: gateway redirect → IdP authorize →
/// callback query. Returns the `/auth/callback` path+query to hit.
async fn drive_to_callback(app: &Router, expect_challenge: bool) -> String {
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
    if expect_challenge {
        let url = url::Url::parse(&location).expect("authorize url");
        let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!(query["response_type"], "code");
        assert_eq!(query["client_id"], CLIENT_ID);
        assert_eq!(query["redirect_uri"], REDIRECT_URI);
        assert_eq!(query["code_challenge_method"], "S256");
        assert!(!query["code_challenge"].is_empty());
        assert!(!query["state"].is_empty());
        assert!(!query["nonce"].is_empty());
        assert!(query["scope"].contains("openid"));
    }

    // The browser hop to the IdP; no redirect-following so the callback
    // Location comes back to us.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let idp_response = client.get(&location).send().await.expect("authorize hop");
    assert_eq!(
        idp_response.status(),
        reqwest::StatusCode::TEMPORARY_REDIRECT
    );
    let callback = idp_response.headers()[reqwest::header::LOCATION]
        .to_str()
        .unwrap();
    let callback = url::Url::parse(callback).expect("callback url");
    assert_eq!(callback.path(), "/auth/callback");
    format!("/auth/callback?{}", callback.query().expect("query"))
}

/// Connects to `DATABASE_URL`, applies migrations, and admits one active
/// tenant. `None` = no database configured; the test skips quietly.
async fn admitted_tenant() -> Option<(String, TenantId)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping OIDC login DB test: DATABASE_URL is not set \
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
    let slug = format!("auth1-{}", id.as_uuid().simple());
    tenant_fixture::create(&pool, id, &slug, "AUTH-1 test tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((url, id))
}

// ── The AC, mock-Entra half (needs a database) ──────────────────────────────

#[tokio::test]
async fn entra_shaped_login_yields_a_synveda_session() {
    let _serial = serial().await;
    let Some((db_url, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let idp = MockIdp::spawn(Some(tenant_id.to_string())).await;
    let app = router(oidc_state(
        &db_url,
        &idp.issuer,
        &Binding::Claim,
        Duration::ZERO,
    ));

    let callback = drive_to_callback(&app, true).await;
    let response = app
        .clone()
        .oneshot(get_request(&callback, None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let session = body_json(response).await;
    assert_eq!(session["subject"], SUBJECT, "session: {session}");
    assert_eq!(
        session["tenant"]["id"],
        tenant_id.to_string(),
        "session: {session}"
    );
    assert_eq!(session["token_type"], "Bearer", "session: {session}");

    // "Yields a Synveda session": the returned access token IS the /v1
    // bearer credential, and whoami proves the full chain.
    let token = session["access_token"].as_str().expect("access_token");
    let response = app
        .oneshot(get_request("/v1/whoami", Some(token)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let whoami = body_json(response).await;
    assert_eq!(whoami["subject"], SUBJECT, "whoami: {whoami}");
    assert_eq!(
        whoami["tenant"]["id"],
        tenant_id.to_string(),
        "whoami: {whoami}"
    );
}

#[tokio::test]
async fn static_tenant_binding_login_yields_a_session_too() {
    // The Rauthy/dev config shape (ADR-0010 §4): no tenant claim in the
    // token; the issuer itself is bound to one tenant. The live-Rauthy run
    // of this same shape is demos/auth-1-oidc-login.sh.
    let _serial = serial().await;
    let Some((db_url, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let idp = MockIdp::spawn(None).await;
    let app = router(oidc_state(
        &db_url,
        &idp.issuer,
        &Binding::Static(tenant_id),
        Duration::ZERO,
    ));

    let callback = drive_to_callback(&app, false).await;
    let response = app
        .clone()
        .oneshot(get_request(&callback, None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let session = body_json(response).await;
    assert_eq!(session["tenant"]["id"], tenant_id.to_string());

    let token = session["access_token"].as_str().expect("access_token");
    let response = app
        .oneshot(get_request("/v1/whoami", Some(token)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// ── Verification contract (no database) ─────────────────────────────────────

#[tokio::test]
async fn oidc_bearer_is_verified_via_jwks_before_storage() {
    let _serial = serial().await;
    let tenant = TenantId::new();
    let idp = MockIdp::spawn(Some(tenant.to_string())).await;
    let app = router(oidc_state(
        UNREACHABLE_URL,
        &idp.issuer,
        &Binding::Claim,
        Duration::ZERO,
    ));

    // Valid token, unreachable storage: 503, not 401 — verification
    // succeeded (the TEN-1 doctrine, now through JWKS).
    let good = idp.access_token(CLIENT_ID, 600);
    let (status, kind) = status_and_kind(
        app.clone()
            .oneshot(get_request("/v1/whoami", Some(&good)))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        (status, kind.as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, "storage")
    );

    // Every doubt is the uniform 401.
    for (label, token) in [
        ("expired", idp.access_token(CLIENT_ID, -120)),
        ("wrong audience", idp.access_token("someone-else", 600)),
        ("tampered", format!("{good}x")),
    ] {
        let (status, kind) = status_and_kind(
            app.clone()
                .oneshot(get_request("/v1/whoami", Some(&token)))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(
            (status, kind.as_str()),
            (StatusCode::UNAUTHORIZED, "unauthenticated"),
            "{label} token must be the uniform 401"
        );
    }

    // The rejection is visible in the AUTH-1 metric contract.
    let exposition = metrics_handle().render();
    assert!(
        exposition
            .lines()
            .any(|line| line.starts_with("synveda_token_verifications_total")
                && line.contains("outcome=\"rejected\"")),
        "rejected outcome missing from exposition:\n{exposition}"
    );
}

#[tokio::test]
async fn key_rotation_heals_on_the_next_request() {
    let _serial = serial().await;
    let tenant = TenantId::new();
    let idp = MockIdp::spawn(Some(tenant.to_string())).await;
    let app = router(oidc_state(
        UNREACHABLE_URL,
        &idp.issuer,
        &Binding::Claim,
        Duration::ZERO,
    ));

    let (status, _) = status_and_kind(
        app.clone()
            .oneshot(get_request(
                "/v1/whoami",
                Some(&idp.access_token(CLIENT_ID, 600)),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "key A verifies");
    let fetches_before = idp.jwks_fetches();

    // The IdP rotates; the very next request signed with the new key must
    // trigger a refetch and verify (the AUTH-1 rotation-handling AC).
    idp.rotate_to_key_b();
    let (status, _) = status_and_kind(
        app.clone()
            .oneshot(get_request(
                "/v1/whoami",
                Some(&idp.access_token(CLIENT_ID, 600)),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "key B verifies after rotation"
    );
    assert!(
        idp.jwks_fetches() > fetches_before,
        "rotation must refetch the JWKS"
    );
}

#[tokio::test]
async fn unknown_kid_is_rejected_when_the_refresh_rate_limit_holds() {
    let _serial = serial().await;
    let tenant = TenantId::new();
    let idp = MockIdp::spawn(Some(tenant.to_string())).await;
    // An hour-long rate limit: the initial fetch is the only one allowed.
    let app = router(oidc_state(
        UNREACHABLE_URL,
        &idp.issuer,
        &Binding::Claim,
        Duration::from_secs(3600),
    ));

    let (status, _) = status_and_kind(
        app.clone()
            .oneshot(get_request(
                "/v1/whoami",
                Some(&idp.access_token(CLIENT_ID, 600)),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(idp.jwks_fetches(), 1);

    idp.rotate_to_key_b();
    let (status, kind) = status_and_kind(
        app.clone()
            .oneshot(get_request(
                "/v1/whoami",
                Some(&idp.access_token(CLIENT_ID, 600)),
            ))
            .await
            .unwrap(),
    )
    .await;
    // Fail closed, and no fetch: an unknown kid cannot drive load.
    assert_eq!(
        (status, kind.as_str()),
        (StatusCode::UNAUTHORIZED, "unauthenticated")
    );
    assert_eq!(idp.jwks_fetches(), 1, "rate-limited refresh must not fetch");
}

// ── Login flow contract (no database) ───────────────────────────────────────

#[tokio::test]
async fn callback_state_is_single_use() {
    let _serial = serial().await;
    let tenant = TenantId::new();
    let idp = MockIdp::spawn(Some(tenant.to_string())).await;
    let app = router(oidc_state(
        UNREACHABLE_URL,
        &idp.issuer,
        &Binding::Claim,
        Duration::ZERO,
    ));

    let callback = drive_to_callback(&app, true).await;
    // First use: the whole flow succeeds up to the tenant lookup, which
    // hits unreachable storage — 503 proves exchange + ID-token + nonce
    // all passed.
    let (status, kind) = status_and_kind(
        app.clone()
            .oneshot(get_request(&callback, None))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        (status, kind.as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, "storage")
    );

    // Replay: the state was consumed; the identical callback is a 401.
    let (status, kind) = status_and_kind(
        app.clone()
            .oneshot(get_request(&callback, None))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        (status, kind.as_str()),
        (StatusCode::UNAUTHORIZED, "unauthenticated")
    );
}

#[tokio::test]
async fn callback_rejects_errors_forgeries_and_garbage() {
    let _serial = serial().await;
    let tenant = TenantId::new();
    let idp = MockIdp::spawn(Some(tenant.to_string())).await;
    let app = router(oidc_state(
        UNREACHABLE_URL,
        &idp.issuer,
        &Binding::Claim,
        Duration::ZERO,
    ));

    // IdP-reported denial.
    let (status, kind) = status_and_kind(
        app.clone()
            .oneshot(get_request("/auth/callback?error=access_denied", None))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        (status, kind.as_str()),
        (StatusCode::UNAUTHORIZED, "unauthenticated")
    );

    // A state the gateway never issued.
    let (status, kind) = status_and_kind(
        app.clone()
            .oneshot(get_request(
                "/auth/callback?code=x&state=never-issued",
                None,
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        (status, kind.as_str()),
        (StatusCode::UNAUTHORIZED, "unauthenticated")
    );

    // Missing parameters.
    let (status, kind) = status_and_kind(
        app.clone()
            .oneshot(get_request("/auth/callback", None))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        (status, kind.as_str()),
        (StatusCode::BAD_REQUEST, "invalid")
    );
}

#[tokio::test]
async fn auth_plane_is_404_when_oidc_is_not_configured() {
    let _serial = serial().await;
    // HS256/dev mode: no login flow. The routes exist but answer 404.
    let state = AppState {
        pool: pool(UNREACHABLE_URL),
        metrics: metrics_handle(),
        verifier: Arc::new(synveda_identity::DisabledVerifier),
        login: None,
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
    };
    for uri in ["/auth/login", "/auth/callback?code=x&state=y"] {
        let (status, kind) = status_and_kind(
            router(state.clone())
                .oneshot(get_request(uri, None))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(
            (status, kind.as_str()),
            (StatusCode::NOT_FOUND, "not_found"),
            "{uri} must 404 without OIDC"
        );
    }
}
