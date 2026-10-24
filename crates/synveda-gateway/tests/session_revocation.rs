//! Self console-session inventory and issuer-profile revocation (ADR-0130),
//! against the real database with ordinary tenant transactions.
//!
//! The in-process issuer below is shaped like the bundled Keycloak realm:
//! RS256 access tokens with a fresh `jti`, a `sid` shared by every login in
//! one browser SSO session and stable through refresh, five-minute access
//! lifetimes, and rotating refresh tokens whose reuse is refused. It is a
//! signed fixture, not live Keycloak evidence.
//!
//! Database cases read `DATABASE_URL` and skip with a message when it is
//! unset, the same convention as tests/oidc_login.rs.

#[path = "../../synveda-store/tests/support/tenant_fixture.rs"]
mod tenant_fixture;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Body;
use axum::extract::{Form, Query, State};
use axum::http::{Method, Request, StatusCode, header};
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
use synveda_gateway::app::{AppState, ConfiguredLogin, behavior_test_router as router};
use synveda_gateway::telemetry;
use synveda_identity::console::LOGIN_COOKIE;
use synveda_identity::{LoginFlow, LoginLedger, OidcVerifier, parse_issuers};
use synveda_types::{TenantId, TenantStatus};
use tower::ServiceExt;

const KEY_A_PEM: &str = include_str!("fixtures/idp_key_a.pem");
const KEY_A_JWK: &str = include_str!("fixtures/idp_key_a.jwk.json");

const CLIENT_ID: &str = "synveda-test";
const API_AUDIENCE: &str = "synveda-test-api";
const REDIRECT_URI: &str = "http://gateway.test/auth/callback";
const PUBLIC_ORIGIN: &str = "http://127.0.0.1:8120";
const CLI_REDIRECT: &str = "http://127.0.0.1:9/callback";
const CLI_STATE: &str = "cli-state-000000000000000000000000000000000";

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

// ── A Keycloak-shaped issuer ─────────────────────────────────────────────────

struct PendingCode {
    nonce: String,
    challenge: String,
    redirect_uri: String,
    subject: String,
    tenant: String,
    sid: String,
}

struct RefreshGrant {
    subject: String,
    tenant: String,
    sid: String,
    used: bool,
}

#[derive(Default)]
struct IssuerState {
    codes: HashMap<String, PendingCode>,
    refresh: HashMap<String, RefreshGrant>,
    sso_sid: Option<String>,
    subject: String,
    tenant: String,
}

#[derive(Clone)]
struct Keycloak {
    issuer: String,
    key: Arc<EncodingKey>,
    state: Arc<Mutex<IssuerState>>,
    next: Arc<AtomicU64>,
}

impl Keycloak {
    async fn spawn(subject: &str, tenant: TenantId) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind issuer");
        let addr = listener.local_addr().expect("issuer addr");
        let idp = Self {
            issuer: format!("http://{addr}/realms/synveda"),
            key: Arc::new(EncodingKey::from_rsa_pem(KEY_A_PEM.as_bytes()).expect("key a")),
            state: Arc::new(Mutex::new(IssuerState {
                subject: subject.to_owned(),
                tenant: tenant.to_string(),
                ..IssuerState::default()
            })),
            next: Arc::new(AtomicU64::new(1)),
        };
        let app = Router::new()
            .route(
                "/realms/synveda/.well-known/openid-configuration",
                get(discovery),
            )
            .route("/realms/synveda/jwks", get(jwks))
            .route("/realms/synveda/authorize", get(authorize))
            .route("/realms/synveda/token", post(token))
            .with_state(idp.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("issuer serve");
        });
        idp
    }

    fn serial(&self, prefix: &str) -> String {
        format!("{prefix}-{}", self.next.fetch_add(1, Ordering::SeqCst))
    }

    /// The next browser login authenticates as this subject and tenant.
    fn sign_in_as(&self, subject: &str, tenant: TenantId) {
        let mut state = self.state.lock().unwrap();
        state.subject = subject.to_owned();
        state.tenant = tenant.to_string();
    }

    /// Ends the browser's SSO session at the issuer; the next login gets a
    /// new `sid`.
    fn end_sso(&self) {
        self.state.lock().unwrap().sso_sid = None;
    }

    fn sign(&self, claims: &Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("key-a".to_owned());
        jsonwebtoken::encode(&header, claims, &self.key).expect("sign")
    }

    fn access_claims(&self, subject: &str, tenant: &str, sid: &str) -> Value {
        let issued = now_secs();
        json!({
            "iss": self.issuer, "sub": subject, "tid": tenant, "aud": API_AUDIENCE,
            "azp": CLIENT_ID, "typ": "Bearer", "iat": issued, "exp": issued + 300,
            "jti": self.serial("at"), "sid": sid,
        })
    }

    /// A bearer for an existing session family, as a second client of the
    /// same issuer session would receive it.
    fn bearer(&self, subject: &str, tenant: TenantId, sid: &str) -> String {
        self.sign(&self.access_claims(subject, &tenant.to_string(), sid))
    }

    fn tokens(&self, subject: &str, tenant: &str, sid: &str, nonce: Option<&str>) -> Value {
        let refresh = self.serial("rt");
        self.state.lock().unwrap().refresh.insert(
            refresh.clone(),
            RefreshGrant {
                subject: subject.to_owned(),
                tenant: tenant.to_owned(),
                sid: sid.to_owned(),
                used: false,
            },
        );
        let mut body = json!({
            "access_token": self.sign(&self.access_claims(subject, tenant, sid)),
            "token_type": "Bearer",
            "expires_in": 300,
            "refresh_token": refresh,
        });
        if let Some(nonce) = nonce {
            let issued = now_secs();
            body["id_token"] = json!(self.sign(&json!({
                "iss": self.issuer, "sub": subject, "tid": tenant, "aud": CLIENT_ID,
                "azp": CLIENT_ID, "typ": "ID", "iat": issued, "exp": issued + 300,
                "nonce": nonce, "sid": sid, "jti": self.serial("id"),
            })));
        }
        body
    }
}

async fn discovery(State(idp): State<Keycloak>) -> Json<Value> {
    Json(json!({
        "issuer": idp.issuer,
        "authorization_endpoint": format!("{}/authorize", idp.issuer),
        "token_endpoint": format!("{}/token", idp.issuer),
        "jwks_uri": format!("{}/jwks", idp.issuer),
        "code_challenge_methods_supported": ["S256"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
    }))
}

async fn jwks() -> Json<Value> {
    Json(json!({ "keys": [serde_json::from_str::<Value>(KEY_A_JWK).expect("jwk")] }))
}

async fn authorize(
    State(idp): State<Keycloak>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let code = idp.serial("code");
    let mut state = idp.state.lock().unwrap();
    let sid = match &state.sso_sid {
        Some(sid) => sid.clone(),
        None => {
            let sid = idp.serial("sid");
            state.sso_sid = Some(sid.clone());
            sid
        }
    };
    let pending = PendingCode {
        nonce: query["nonce"].clone(),
        challenge: query["code_challenge"].clone(),
        redirect_uri: query["redirect_uri"].clone(),
        subject: state.subject.clone(),
        tenant: state.tenant.clone(),
        sid,
    };
    state.codes.insert(code.clone(), pending);
    Redirect::temporary(&format!(
        "{}?code={code}&state={}",
        query["redirect_uri"], query["state"]
    ))
    .into_response()
}

async fn token(State(idp): State<Keycloak>, Form(form): Form<HashMap<String, String>>) -> Response {
    let refuse = || {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_grant"})),
        )
            .into_response()
    };
    match form.get("grant_type").map(String::as_str) {
        Some("authorization_code") => {
            let Some(pending) = form
                .get("code")
                .and_then(|code| idp.state.lock().unwrap().codes.remove(code))
            else {
                return refuse();
            };
            let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(
                form.get("code_verifier")
                    .map_or("", String::as_str)
                    .as_bytes(),
            ));
            if challenge != pending.challenge
                || form.get("redirect_uri") != Some(&pending.redirect_uri)
            {
                return refuse();
            }
            Json(idp.tokens(
                &pending.subject,
                &pending.tenant,
                &pending.sid,
                Some(&pending.nonce),
            ))
            .into_response()
        }
        Some("refresh_token") => {
            let grant = {
                let mut state = idp.state.lock().unwrap();
                let Some(grant) = form
                    .get("refresh_token")
                    .and_then(|token| state.refresh.get_mut(token))
                else {
                    return refuse();
                };
                // Rotation with zero reuse: a used refresh token is refused.
                if grant.used {
                    return refuse();
                }
                grant.used = true;
                (
                    grant.subject.clone(),
                    grant.tenant.clone(),
                    grant.sid.clone(),
                )
            };
            Json(idp.tokens(&grant.0, &grant.1, &grant.2, None)).into_response()
        }
        _ => refuse(),
    }
}

// ── Gateway harness ──────────────────────────────────────────────────────────

fn pool(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        .connect_lazy(url)
        .expect("parse database url")
}

/// One gateway instance with its own pool, verifier and login ledger.
async fn gateway(url: &str, issuer: &str, mode: &str) -> Router {
    gateway_with(
        url,
        issuer,
        &format!(r#","revocation":{{"profile":"bundled_keycloak","mode":"{mode}"}}"#),
    )
    .await
}

/// The same issuer with no revocation profile: today's authentication
/// contract and no ledger read.
async fn gateway_without_profile(url: &str, issuer: &str) -> Router {
    gateway_with(url, issuer, "").await
}

async fn gateway_with(url: &str, issuer: &str, profile: &str) -> Router {
    let config = format!(
        r#"[{{"issuer":"{issuer}","client_id":"{CLIENT_ID}","audience":"{API_AUDIENCE}"{profile}}}]"#
    );
    let verifier = Arc::new(
        OidcVerifier::new(parse_issuers(&config).expect("issuer config")).expect("verifier"),
    );
    let keys = Arc::new(synveda_store::keys::KeyRing::new(
        synveda_crypto::Kms::Local(
            synveda_crypto::LocalKms::from_hex(&"11".repeat(32), "local:test").expect("test kek"),
        ),
    ));
    let ledger: Arc<dyn LoginLedger> = Arc::new(synveda_identity::MemoryLoginLedger::new());
    let pool = pool(url);
    keys.provision(&pool, synveda_crypto::KeyScope::Deployment)
        .await
        .expect("provision the deployment key for sealed console tokens");
    router(AppState {
        pool,
        metrics: metrics_handle(),
        verifier: verifier.clone(),
        login: Some(Arc::new(ConfiguredLogin::for_behavior_test(
            LoginFlow::new(verifier, REDIRECT_URI.to_owned(), ledger),
            false,
        ))),
        public_origin: PUBLIC_ORIGIN.to_owned(),
        pdp: Arc::new(synveda_policy::Pdp::new().expect("PDP")),
        service_token_max_ttl: Duration::from_secs(3600),
        embedder: Arc::new(synveda_ingest::embedding::AnyEmbedder::Deterministic(
            synveda_ingest::embedding::DeterministicEmbedder::new(),
        )),
        context_embed_timeout: Duration::from_millis(100),
        git_sources: None,
        git_exports: None,
        keys,
    })
}

async fn admitted_tenant(label: &str) -> Option<(String, TenantId)> {
    let url = tenant_fixture::database_url("session revocation DB test")?;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect");
    synveda_store::epoch::verify(&pool)
        .await
        .expect("current schema");
    let id = TenantId::new();
    let slug = format!("{label}-{}", id.as_uuid().simple());
    tenant_fixture::create(&pool, id, &slug, "revocation test", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((url, id))
}

async fn second_tenant(url: &str) -> TenantId {
    let pool = pool(url);
    let id = TenantId::new();
    let slug = format!("auth6b-{}", id.as_uuid().simple());
    tenant_fixture::create(&pool, id, &slug, "other tenant", TenantStatus::Active)
        .await
        .expect("admit second tenant");
    id
}

enum Auth<'a> {
    Bearer(&'a str),
    Cookie(&'a str),
}

async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    auth: Auth<'_>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    builder = match auth {
        Auth::Bearer(token) => builder.header(header::AUTHORIZATION, format!("Bearer {token}")),
        Auth::Cookie(cookie) => builder
            .header(header::COOKIE, cookie)
            .header(header::ORIGIN, PUBLIC_ORIGIN),
    };
    let request = match body {
        Some(body) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string())),
        None => builder.body(Body::empty()),
    }
    .expect("request");
    let response = app.clone().oneshot(request).await.expect("route");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn location(response: &Response) -> String {
    response.headers()[header::LOCATION]
        .to_str()
        .expect("location")
        .to_owned()
}

async fn issuer_hop(location: &str) -> String {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("client");
    let response = client.get(location).send().await.expect("authorize hop");
    response.headers()[reqwest::header::LOCATION]
        .to_str()
        .expect("callback location")
        .to_owned()
}

/// A console login as the browser drives it. `Ok(cookie)` on success, or the
/// console's error classification.
async fn console_login(app: &Router) -> Result<String, String> {
    let start = app
        .clone()
        .oneshot(
            Request::get("/auth/login?console=true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("login route");
    let correlation = start.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    assert!(correlation.starts_with(&format!("{LOGIN_COOKIE}=")));
    let callback = url::Url::parse(&issuer_hop(&location(&start)).await).expect("callback");
    let callback = format!("/auth/callback?{}", callback.query().expect("query"));
    let landed = app
        .clone()
        .oneshot(
            Request::get(callback)
                .header(header::COOKIE, correlation)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("callback route");
    let landing = location(&landed);
    if let Some(error) = landing.strip_prefix("/console/?error=") {
        return Err(error.to_owned());
    }
    assert_eq!(landing, "/console/");
    let cookie = landed
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| {
            value
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .to_owned()
        })
        .find(|pair| pair.starts_with("__Host-synveda_console="))
        .expect("console session cookie");
    Ok(cookie)
}

struct CliSession {
    access_token: String,
    refresh_token: String,
}

async fn cli_login(app: &Router) -> CliSession {
    let start = app
        .clone()
        .oneshot(
            Request::get(format!(
                "/auth/login?cli_redirect_uri={}&cli_state={CLI_STATE}",
                urlencode(CLI_REDIRECT)
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .expect("cli login route");
    let callback = url::Url::parse(&issuer_hop(&location(&start)).await).expect("callback");
    let handed = app
        .clone()
        .oneshot(
            Request::get(format!("/auth/callback?{}", callback.query().unwrap()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("callback");
    let handoff = url::Url::parse(&location(&handed)).expect("handoff");
    let code = handoff
        .query_pairs()
        .find_map(|(name, value)| (name == "code").then(|| value.into_owned()))
        .expect("handoff code");
    let request = Request::post("/auth/cli/exchange")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"code": code, "state": CLI_STATE}).to_string(),
        ))
        .unwrap();
    let response = app.clone().oneshot(request).await.expect("exchange");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let session: Value = serde_json::from_slice(&bytes).unwrap();
    CliSession {
        access_token: session["access_token"].as_str().unwrap().to_owned(),
        refresh_token: session["refresh_token"].as_str().unwrap().to_owned(),
    }
}

async fn refresh(app: &Router, refresh_token: &str) -> (StatusCode, Value) {
    let request = Request::post("/auth/refresh")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"refresh_token": refresh_token}).to_string(),
        ))
        .unwrap();
    let response = app.clone().oneshot(request).await.expect("refresh route");
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn urlencode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

async fn whoami(app: &Router, auth: Auth<'_>) -> StatusCode {
    call(app, Method::GET, "/v1/whoami", auth, None).await.0
}

async fn listed(app: &Router, auth: Auth<'_>) -> Vec<Value> {
    let (status, body) = call(app, Method::GET, "/v1/me/console-sessions", auth, None).await;
    assert_eq!(status, StatusCode::OK, "list: {body}");
    body["sessions"].as_array().cloned().unwrap_or_default()
}

/// Custody and index rows written straight through the store, as a long
/// history of logins would leave them. The ciphertext is never opened.
async fn seed_sessions(url: &str, tenant: TenantId, subject: &str, count: usize) -> Vec<[u8; 32]> {
    let pool = pool(url);
    let mut hashes = Vec::with_capacity(count);
    let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
        .await
        .unwrap();
    for index in 0..count {
        let hash: [u8; 32] = Sha256::digest(format!("{tenant}-{subject}-{index}")).into();
        let family: [u8; 32] = Sha256::digest(format!("family-{tenant}-{subject}-{index}")).into();
        let expires = chrono::Utc::now() + chrono::Duration::hours(12);
        synveda_store::console_sessions::create(
            &mut *tx,
            &hash,
            "https://seeded.test",
            &[0u8; 64],
            None,
            None,
            expires,
        )
        .await
        .unwrap();
        synveda_store::session_index::insert(
            &mut *tx,
            &synveda_store::session_index::NewEntry {
                tenant_id: tenant,
                id: uuid::Uuid::from_u128(u128::from_be_bytes(hash[..16].try_into().unwrap())),
                subject,
                issuer: "https://seeded.test",
                custody_hash: &hash,
                family_digest: Some(&family),
                expires_at: expires,
            },
        )
        .await
        .unwrap();
        hashes.push(hash);
    }
    tx.commit().await.unwrap();
    hashes
}

async fn custody_rows(url: &str, hashes: &[[u8; 32]]) -> i64 {
    let hashes: Vec<Vec<u8>> = hashes.iter().map(|hash| hash.to_vec()).collect();
    sqlx::query_scalar("select count(*) from console_sessions where token_hash = any($1)")
        .bind(&hashes)
        .fetch_one(&pool(url))
        .await
        .unwrap()
}

async fn live_ledger_rows(url: &str, tenant: TenantId) -> i64 {
    let pool = pool(url);
    let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
        .await
        .unwrap();
    let count = sqlx::query_scalar(
        "select count(*) from token_revocations where tenant_id = $1 and expires_at > now()",
    )
    .bind(tenant.as_uuid())
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    count
}

// ── Acceptance ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn console_login_indexes_its_custody_and_lists_only_safe_fields() {
    let _serial = serial().await;
    let Some((url, tenant)) = admitted_tenant("auth6-list").await else {
        return;
    };
    let idp = Keycloak::spawn("alice@example.test", tenant).await;
    let app = gateway(&url, &idp.issuer, "enforce").await;
    let cookie = console_login(&app).await.expect("console login");
    let secret = cookie.split_once('=').unwrap().1.to_owned();

    let (status, page) = call(
        &app,
        Method::GET,
        "/v1/me/console-sessions",
        Auth::Cookie(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let sessions = page["sessions"].as_array().expect("sessions");
    assert_eq!(sessions.len(), 1, "{page}");
    let mut keys: Vec<&str> = sessions[0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["created_at", "expires_at", "id", "issuer", "last_seen_at"]
    );
    assert_eq!(sessions[0]["issuer"], idp.issuer);
    let rendered = page.to_string();
    let hash = URL_SAFE_NO_PAD.encode(synveda_identity::console::hash(&secret));
    for forbidden in [secret.as_str(), hash.as_str(), "token", "hash", "digest"] {
        assert!(
            !rendered.contains(forbidden),
            "inventory leaked {forbidden}"
        );
    }

    // Custody and index committed together: the index row names live custody.
    let pool = pool(&url);
    let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
        .await
        .unwrap();
    let joined: i64 = sqlx::query_scalar(
        "select count(*) from console_session_index entry \
         join console_sessions custody on custody.token_hash = entry.custody_hash \
         where entry.tenant_id = $1 and entry.family_digest is not null",
    )
    .bind(tenant.as_uuid())
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(joined, 1);
}

#[tokio::test]
async fn revoking_a_console_session_ends_its_custody_and_every_bearer_of_its_family() {
    let _serial = serial().await;
    let Some((url, tenant)) = admitted_tenant("auth6-revoke").await else {
        return;
    };
    let idp = Keycloak::spawn("bob@example.test", tenant).await;
    let app = gateway(&url, &idp.issuer, "enforce").await;
    let cookie = console_login(&app).await.expect("console login");
    // A CLI login in the same browser joins the same issuer session.
    let cli = cli_login(&app).await;
    assert_eq!(whoami(&app, Auth::Cookie(&cookie)).await, StatusCode::OK);
    assert_eq!(
        whoami(&app, Auth::Bearer(&cli.access_token)).await,
        StatusCode::OK
    );

    let sessions = listed(&app, Auth::Bearer(&cli.access_token)).await;
    let id = sessions[0]["id"].as_str().unwrap().to_owned();
    let revoke = format!("/v1/me/console-sessions/{id}");
    let guessed = format!("/v1/me/console-sessions/{}", uuid::Uuid::from_u128(7));
    // A guessed id answers exactly like a real one.
    let (status, _) = call(
        &app,
        Method::DELETE,
        &guessed,
        Auth::Bearer(&cli.access_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(whoami(&app, Auth::Cookie(&cookie)).await, StatusCode::OK);
    let (status, _) = call(&app, Method::DELETE, &revoke, Auth::Cookie(&cookie), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The cookie, the CLI bearer of the same family and a refresh of it all
    // fail; the duplicate request has no other outcome to report.
    assert_eq!(
        whoami(&app, Auth::Cookie(&cookie)).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        whoami(&app, Auth::Bearer(&cli.access_token)).await,
        StatusCode::UNAUTHORIZED
    );
    let (status, _) = refresh(&app, &cli.refresh_token).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = call(&app, Method::DELETE, &revoke, Auth::Cookie(&cookie), None).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the revoked cookie cannot act at all"
    );

    // Signing in again inside the same issuer session is a revoked family;
    // a fresh issuer session is a fresh family.
    assert_eq!(console_login(&app).await, Err("session_revoked".to_owned()));
    idp.end_sso();
    let fresh = console_login(&app).await.expect("fresh issuer session");
    assert_eq!(whoami(&app, Auth::Cookie(&fresh)).await, StatusCode::OK);
    assert_eq!(listed(&app, Auth::Cookie(&fresh)).await.len(), 1);

    // Content-free audit: counts and closed labels only.
    let pool = pool(&url);
    let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
        .await
        .unwrap();
    let payloads: Vec<Value> = sqlx::query_scalar(
        "select payload from audit_log where tenant_id = $1 and action = 'auth.login_session.revoked' order by seq",
    )
    .bind(tenant.as_uuid())
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(payloads.len(), 2, "{payloads:?}");
    assert_eq!(payloads[0]["sessions_revoked"], 0);
    assert_eq!(payloads[1]["sessions_revoked"], 1);
    assert_eq!(payloads[1]["family"], "recorded");
    let rendered = json!(payloads).to_string();
    assert!(!rendered.contains(&id) && !rendered.contains(cookie.split_once('=').unwrap().1));
}

#[tokio::test]
async fn another_principal_or_tenant_can_neither_see_nor_revoke_a_session() {
    let _serial = serial().await;
    let Some((url, tenant)) = admitted_tenant("auth6-foreign").await else {
        return;
    };
    let other_tenant = second_tenant(&url).await;
    let idp = Keycloak::spawn("carol@example.test", tenant).await;
    let app = gateway(&url, &idp.issuer, "enforce").await;
    let carol = console_login(&app).await.expect("carol");
    let carol_id = listed(&app, Auth::Cookie(&carol)).await[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    idp.end_sso();
    idp.sign_in_as("dave@example.test", tenant);
    let dave = console_login(&app).await.expect("dave");
    idp.end_sso();
    idp.sign_in_as("erin@example.test", other_tenant);
    let erin = console_login(&app).await.expect("erin in another tenant");

    for intruder in [&dave, &erin] {
        let sessions = listed(&app, Auth::Cookie(intruder)).await;
        assert_eq!(sessions.len(), 1);
        assert_ne!(sessions[0]["id"], carol_id.as_str());
        let (status, _) = call(
            &app,
            Method::DELETE,
            &format!("/v1/me/console-sessions/{carol_id}"),
            Auth::Cookie(intruder),
            None,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NO_CONTENT,
            "the same answer as any absent id"
        );
    }
    assert_eq!(whoami(&app, Auth::Cookie(&carol)).await, StatusCode::OK);
    assert_eq!(listed(&app, Auth::Cookie(&carol)).await.len(), 1);
}

#[tokio::test]
async fn revoke_all_commits_bounded_batches_and_reports_truthful_progress() {
    let _serial = serial().await;
    let Some((url, tenant)) = admitted_tenant("auth6-all").await else {
        return;
    };
    let subject = "frank@example.test";
    let idp = Keycloak::spawn(subject, tenant).await;
    let app = gateway(&url, &idp.issuer, "enforce").await;
    let cli = cli_login(&app).await;
    let hashes = seed_sessions(&url, tenant, subject, 300).await;
    assert_eq!(custody_rows(&url, &hashes).await, 300);

    let all = "/v1/me/console-sessions/revoke-all";
    let (status, first) = call(
        &app,
        Method::POST,
        all,
        Auth::Bearer(&cli.access_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first, json!({"revoked": 256, "complete": false}));
    assert_eq!(custody_rows(&url, &hashes).await, 44);
    let (_, second) = call(
        &app,
        Method::POST,
        all,
        Auth::Bearer(&cli.access_token),
        None,
    )
    .await;
    assert_eq!(second, json!({"revoked": 44, "complete": true}));
    let (_, third) = call(
        &app,
        Method::POST,
        all,
        Auth::Bearer(&cli.access_token),
        None,
    )
    .await;
    assert_eq!(third, json!({"revoked": 0, "complete": true}));
    assert_eq!(custody_rows(&url, &hashes).await, 0);
    assert_eq!(live_ledger_rows(&url, tenant).await, 300);
    // The CLI login is not an inventoried console session, so it survives.
    assert_eq!(
        whoami(&app, Auth::Bearer(&cli.access_token)).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn an_expired_session_is_not_listed_and_revoking_it_is_idempotent() {
    let _serial = serial().await;
    let Some((url, tenant)) = admitted_tenant("auth6-expired").await else {
        return;
    };
    let subject = "grace@example.test";
    let idp = Keycloak::spawn(subject, tenant).await;
    let app = gateway(&url, &idp.issuer, "enforce").await;
    let cli = cli_login(&app).await;
    let hashes = seed_sessions(&url, tenant, subject, 1).await;
    sqlx::query(
        "update console_sessions set created_at = now() - interval '13 hours', \
         absolute_expires_at = now() - interval '1 hour' where token_hash = $1",
    )
    .bind(hashes[0].to_vec())
    .execute(&pool(&url))
    .await
    .unwrap();
    assert!(
        listed(&app, Auth::Bearer(&cli.access_token))
            .await
            .is_empty()
    );
    let id = uuid::Uuid::from_u128(u128::from_be_bytes(hashes[0][..16].try_into().unwrap()));
    for _ in 0..2 {
        let (status, _) = call(
            &app,
            Method::DELETE,
            &format!("/v1/me/console-sessions/{id}"),
            Auth::Bearer(&cli.access_token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
    assert_eq!(custody_rows(&url, &hashes).await, 0);
}

#[tokio::test]
async fn a_full_ledger_refuses_the_revocation_and_keeps_the_session() {
    let _serial = serial().await;
    let Some((url, tenant)) = admitted_tenant("auth6-full").await else {
        return;
    };
    let idp = Keycloak::spawn("heidi@example.test", tenant).await;
    let app = gateway(&url, &idp.issuer, "enforce").await;
    let cookie = console_login(&app).await.expect("console login");
    let pool = pool(&url);
    let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
        .await
        .unwrap();
    sqlx::query(
        "insert into token_revocations (tenant_id, digest, kind, expires_at) \
         select $1, sha256(convert_to('fill-' || i, 'UTF8')), 'family', now() + interval '1 hour' \
         from generate_series(1, $2::int) as i",
    )
    .bind(tenant.as_uuid())
    .bind(i32::try_from(synveda_store::token_revocations::MAX_LIVE_ROWS_PER_TENANT).unwrap())
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let id = listed(&app, Auth::Cookie(&cookie)).await[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, body) = call(
        &app,
        Method::DELETE,
        &format!("/v1/me/console-sessions/{id}"),
        Auth::Cookie(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    let (status, _) = call(
        &app,
        Method::POST,
        "/v1/me/credential/revoke",
        Auth::Cookie(&cookie),
        Some(json!({"identifier": "family"})),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    // Nothing was acknowledged, so nothing changed.
    assert_eq!(whoami(&app, Auth::Cookie(&cookie)).await, StatusCode::OK);
    assert_eq!(listed(&app, Auth::Cookie(&cookie)).await.len(), 1);
    assert_eq!(
        live_ledger_rows(&url, tenant).await,
        synveda_store::token_revocations::MAX_LIVE_ROWS_PER_TENANT
    );
}

#[tokio::test]
async fn the_enforced_profile_refuses_tokens_outside_it_and_observe_admits_them() {
    let _serial = serial().await;
    let Some((url, tenant)) = admitted_tenant("auth6-shape").await else {
        return;
    };
    let subject = "ivan@example.test";
    let idp = Keycloak::spawn(subject, tenant).await;
    let enforce = gateway(&url, &idp.issuer, "enforce").await;
    let observe = gateway(&url, &idp.issuer, "observe").await;
    let good = idp.access_claims(subject, &tenant.to_string(), "sid-shape");
    let mut cases = vec![("in profile", good.clone(), StatusCode::OK)];
    for (label, claim) in [("no jti", "jti"), ("no sid", "sid"), ("no iat", "iat")] {
        let mut claims = good.clone();
        claims.as_object_mut().unwrap().remove(claim);
        cases.push((label, claims, StatusCode::UNAUTHORIZED));
    }
    let mut long = good.clone();
    long["exp"] = json!(good["iat"].as_u64().unwrap() + 301);
    cases.push(("lifetime 301 s", long, StatusCode::UNAUTHORIZED));
    let mut oversized = good.clone();
    oversized["jti"] = json!("j".repeat(257));
    cases.push(("oversized jti", oversized, StatusCode::UNAUTHORIZED));

    for (label, claims, expected) in cases {
        let token = idp.sign(&claims);
        assert_eq!(
            whoami(&enforce, Auth::Bearer(&token)).await,
            expected,
            "{label}"
        );
        assert_eq!(
            whoami(&observe, Auth::Bearer(&token)).await,
            StatusCode::OK,
            "{label}"
        );
    }
    let rendered = metrics_handle().render();
    assert!(
        rendered.contains(
            r#"synveda_token_revocation_checks_total{mode="observe",outcome="missing_token_id"}"#
        ),
        "observe reports what enforce refuses"
    );
}

#[tokio::test]
async fn gateway_instances_and_a_restart_enforce_one_committed_revocation() {
    let _serial = serial().await;
    let Some((url, tenant)) = admitted_tenant("auth6-replicas").await else {
        return;
    };
    let idp = Keycloak::spawn("judy@example.test", tenant).await;
    let first = gateway(&url, &idp.issuer, "enforce").await;
    let second = gateway(&url, &idp.issuer, "enforce").await;
    let cli = cli_login(&first).await;
    let sibling = idp.bearer("judy@example.test", tenant, &sid_of(&cli.access_token));
    for app in [&first, &second] {
        assert_eq!(
            whoami(app, Auth::Bearer(&cli.access_token)).await,
            StatusCode::OK
        );
    }

    let (status, body) = call(
        &second,
        Method::POST,
        "/v1/me/credential/revoke",
        Auth::Bearer(&cli.access_token),
        Some(json!({"identifier": "token"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"identifier": "token", "enforced": true}));
    let committed = Instant::now();
    assert_eq!(
        whoami(&first, Auth::Bearer(&cli.access_token)).await,
        StatusCode::UNAUTHORIZED
    );
    eprintln!(
        "fixture-only: first request after commit refused on the other instance in {:?}",
        committed.elapsed()
    );
    // One bearer, not its family: another token of the session still works.
    assert_eq!(whoami(&first, Auth::Bearer(&sibling)).await, StatusCode::OK);

    let (status, _) = call(
        &first,
        Method::POST,
        "/v1/me/credential/revoke",
        Auth::Bearer(&sibling),
        Some(json!({"identifier": "family"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let restarted = gateway(&url, &idp.issuer, "enforce").await;
    for app in [&first, &second, &restarted] {
        assert_eq!(
            whoami(app, Auth::Bearer(&sibling)).await,
            StatusCode::UNAUTHORIZED
        );
    }
}

fn sid_of(token: &str) -> String {
    let payload = token.split('.').nth(1).expect("payload");
    let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
    claims["sid"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn concurrent_refresh_replay_and_revoke_keep_the_committed_result() {
    let _serial = serial().await;
    let Some((url, tenant)) = admitted_tenant("auth6-race").await else {
        return;
    };
    let idp = Keycloak::spawn("kim@example.test", tenant).await;
    let app = gateway(&url, &idp.issuer, "enforce").await;
    let cli = cli_login(&app).await;

    // Replaying one refresh token concurrently mints at most one successor.
    let attempts = (0..4).map(|_| {
        let app = app.clone();
        let token = cli.refresh_token.clone();
        tokio::spawn(async move { refresh(&app, &token).await })
    });
    let results: Vec<(StatusCode, Value)> = futures_join(attempts).await;
    let winners: Vec<&Value> = results
        .iter()
        .filter(|(status, _)| *status == StatusCode::OK)
        .map(|(_, body)| body)
        .collect();
    assert_eq!(winners.len(), 1, "{results:?}");
    let rotated = winners[0]["refresh_token"].as_str().unwrap().to_owned();
    let access = winners[0]["access_token"].as_str().unwrap().to_owned();

    // Revoke the family while a refresh of it races.
    let revoke = {
        let app = app.clone();
        let access = access.clone();
        tokio::spawn(async move {
            call(
                &app,
                Method::POST,
                "/v1/me/credential/revoke",
                Auth::Bearer(&access),
                Some(json!({"identifier": "family"})),
            )
            .await
        })
    };
    let racing = {
        let app = app.clone();
        tokio::spawn(async move { refresh(&app, &rotated).await })
    };
    let (revoked, raced) = (revoke.await.unwrap(), racing.await.unwrap());
    assert_eq!(revoked.0, StatusCode::OK);
    // Whatever the race returned, the committed revocation governs it now.
    if raced.0 == StatusCode::OK {
        let token = raced.1["access_token"].as_str().unwrap();
        assert_eq!(
            whoami(&app, Auth::Bearer(token)).await,
            StatusCode::UNAUTHORIZED
        );
        let (status, _) = refresh(&app, raced.1["refresh_token"].as_str().unwrap()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(
        whoami(&app, Auth::Bearer(&access)).await,
        StatusCode::UNAUTHORIZED
    );
    let restarted = gateway(&url, &idp.issuer, "enforce").await;
    assert_eq!(
        whoami(&restarted, Auth::Bearer(&access)).await,
        StatusCode::UNAUTHORIZED
    );
}

async fn futures_join(
    handles: impl Iterator<Item = tokio::task::JoinHandle<(StatusCode, Value)>>,
) -> Vec<(StatusCode, Value)> {
    let mut results = Vec::new();
    for handle in handles.collect::<Vec<_>>() {
        results.push(handle.await.expect("join"));
    }
    results
}

#[tokio::test]
async fn a_ledger_outage_refuses_enforced_requests_without_looking_like_a_revocation() {
    let _serial = serial().await;
    let Some((url, tenant)) = admitted_tenant("auth6-outage").await else {
        return;
    };
    let subject = "liam@example.test";
    let idp = Keycloak::spawn(subject, tenant).await;
    let enforce = gateway(&url, &idp.issuer, "enforce").await;
    let observe = gateway(&url, &idp.issuer, "observe").await;
    let token = idp.bearer(subject, tenant, "sid-outage");
    assert_eq!(whoami(&enforce, Auth::Bearer(&token)).await, StatusCode::OK);

    let holder = pool(&url);
    let mut lock = holder.begin().await.unwrap();
    sqlx::query("lock table token_revocations in access exclusive mode")
        .execute(&mut *lock)
        .await
        .unwrap();
    let (status, body) = call(
        &enforce,
        Method::GET,
        "/v1/whoami",
        Auth::Bearer(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_ne!(body["kind"], "unauthenticated");
    assert_eq!(whoami(&observe, Auth::Bearer(&token)).await, StatusCode::OK);
    lock.rollback().await.unwrap();
    assert_eq!(whoami(&enforce, Auth::Bearer(&token)).await, StatusCode::OK);
}

/// Paired request-path latency with and without the revocation lookup.
/// Ignored by default: it prints fixture-local percentiles and asserts only
/// that every request was admitted. Run it alone with `--ignored --nocapture`.
#[tokio::test]
#[ignore = "measurement; run explicitly"]
async fn measure_the_revocation_lookup_on_the_request_path() {
    let _serial = serial().await;
    let Some((url, tenant)) = admitted_tenant("auth6-measure").await else {
        return;
    };
    let subject = "measure@example.test";
    let idp = Keycloak::spawn(subject, tenant).await;
    let plain = gateway_without_profile(&url, &idp.issuer).await;
    let enforce = gateway(&url, &idp.issuer, "enforce").await;
    let token = idp.bearer(subject, tenant, "sid-measure");
    let samples: usize = std::env::var("SYNVEDA_REVOCATION_MEASURE_SAMPLES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2_000);
    let mut timings: HashMap<&str, Vec<Duration>> = HashMap::new();
    let warmup: usize = std::env::var("SYNVEDA_REVOCATION_MEASURE_WARMUP")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(200);
    for _ in 0..warmup {
        for app in [&plain, &enforce] {
            assert_eq!(whoami(app, Auth::Bearer(&token)).await, StatusCode::OK);
        }
    }
    for round in 0..2 {
        let order: [(&str, &Router); 2] = if round == 0 {
            [("baseline", &plain), ("enforce", &enforce)]
        } else {
            [("enforce", &enforce), ("baseline", &plain)]
        };
        for (label, app) in order {
            for _ in 0..samples / 2 {
                let started = Instant::now();
                assert_eq!(whoami(app, Auth::Bearer(&token)).await, StatusCode::OK);
                timings.entry(label).or_default().push(started.elapsed());
            }
        }
    }
    for label in ["baseline", "enforce"] {
        let mut values = timings.remove(label).unwrap();
        values.sort_unstable();
        let at = |quantile: f64| {
            let index = ((values.len() as f64) * quantile).ceil() as usize;
            values[index.saturating_sub(1).min(values.len() - 1)]
        };
        eprintln!(
            "revocation-measure {label}: n={} p50={:?} p95={:?} p99={:?} max={:?}",
            values.len(),
            at(0.50),
            at(0.95),
            at(0.99),
            values[values.len() - 1]
        );
    }
}

#[tokio::test]
async fn a_full_page_reports_the_next_one() {
    let _serial = serial().await;
    let Some((url, tenant)) = admitted_tenant("auth6-page").await else {
        return;
    };
    let subject = "olga@example.test";
    let idp = Keycloak::spawn(subject, tenant).await;
    let app = gateway(&url, &idp.issuer, "enforce").await;
    let cli = cli_login(&app).await;
    seed_sessions(&url, tenant, subject, 101).await;
    let (status, first) = call(
        &app,
        Method::GET,
        "/v1/me/console-sessions?limit=100",
        Auth::Bearer(&cli.access_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["sessions"].as_array().unwrap().len(), 100);
    let after = first["next_after"]
        .as_str()
        .expect("a full page names the next");
    let (_, second) = call(
        &app,
        Method::GET,
        &format!("/v1/me/console-sessions?limit=100&after={after}"),
        Auth::Bearer(&cli.access_token),
        None,
    )
    .await;
    assert_eq!(second["sessions"].as_array().unwrap().len(), 1);
    assert!(second.get("next_after").is_none(), "{second}");
    let (status, _) = call(
        &app,
        Method::GET,
        "/v1/me/console-sessions?limit=101",
        Auth::Bearer(&cli.access_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_ledger_sweep_removes_only_expired_rows_and_reports_lag() {
    let _serial = serial().await;
    let Some((url, tenant)) = admitted_tenant("auth6-sweep").await else {
        return;
    };
    let subject = "pia@example.test";
    let idp = Keycloak::spawn(subject, tenant).await;
    let app = gateway(&url, &idp.issuer, "enforce").await;
    let cli = cli_login(&app).await;
    let (status, _) = call(
        &app,
        Method::POST,
        "/v1/me/credential/revoke",
        Auth::Bearer(&cli.access_token),
        Some(json!({"identifier": "token"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let pool = pool(&url);
    let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
        .await
        .unwrap();
    sqlx::query(
        "insert into token_revocations (tenant_id, digest, kind, revoked_at, expires_at) \
         select $1, sha256(convert_to('expired-' || i, 'UTF8')), 'family', \
                now() - interval '13 hours', now() - interval '1 hour' \
         from generate_series(1, 300) as i",
    )
    .bind(tenant.as_uuid())
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let (oldest, fullest) = synveda_gateway::worker::sweep_revocation_ledger_once(&pool)
        .await
        .expect("bounded sweep");
    assert!(oldest == 0.0, "every expired row fit in one pass: {oldest}");
    assert!(fullest >= 1);
    let pool = self::pool(&url);
    let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
        .await
        .unwrap();
    let (expired, live): (i64, i64) = sqlx::query_as(
        "select count(*) filter (where expires_at <= now()), count(*) filter (where expires_at > now()) \
         from token_revocations where tenant_id = $1",
    )
    .bind(tenant.as_uuid())
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!((expired, live), (0, 1));
    // The sweep never touches a live row, so the revoked bearer stays refused.
    assert_eq!(
        whoami(&app, Auth::Bearer(&cli.access_token)).await,
        StatusCode::UNAUTHORIZED
    );
}
