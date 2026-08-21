//! CNSL-1's gateway half (ADR-0056 decisions 2, 3 and 4): a browser
//! authenticates at the `/v1` seam with a cookie that *names* a bearer
//! rather than being one.
//!
//! What is asserted here is the security shape of that transport, not the
//! OIDC exchange in front of it — that is AUTH-1's suite, and the console
//! reuses it unchanged. Specifically:
//!
//!   * a cookie resolves to exactly the claims its stored token carries,
//!     through the same verifier a bearer goes through;
//!   * a presented bearer wins over a cookie, so "which credential did this
//!     act under" never depends on header order;
//!   * ambient authority costs an `Origin` on every mutation, and a missing
//!     one is refused rather than waved through;
//!   * a session past its hard cap is gone, and so is one that was signed
//!     out — both as the same uniform 401 a bad bearer gets;
//!   * the cookie the gateway sets carries every attribute the design
//!     depends on, because a cookie missing `HttpOnly` is the whole ADR
//!     silently undone.
//!
//! Tests that need a live Postgres read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database), same convention as
//! tests/tenant_resolution.rs.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use chrono::Utc;
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_identity::console::{CONSOLE_COOKIE, mint};
use synveda_types::{TenantId, TenantStatus};
use tower::ServiceExt;

const SECRET: &[u8] = b"cnsl-1-test-secret";
const ORIGIN: &str = "http://console.test";

/// Serialises tests: the Prometheus recorder is process-global (same
/// rationale as tests/tenant_resolution.rs).
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

fn db_url() -> Option<String> {
    match std::env::var("DATABASE_URL") {
        Ok(url) if !url.is_empty() => Some(url),
        _ => {
            eprintln!("skipping: DATABASE_URL unset (run `make db-test`)");
            None
        }
    }
}

fn pool(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(2))
        .connect_lazy(url)
        .expect("parse database url")
}

fn state(url: &str) -> AppState {
    AppState {
        pool: pool(url),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: ORIGIN.to_owned(),
        pdp: Arc::new(synveda_policy::Pdp::new().expect("build the embedded PDP")),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index: Arc::new(
            synveda_retrieval::SearchIndex::open(
                std::env::temp_dir()
                    .join("synveda-gateway-tests")
                    .join(TenantId::new().to_string()),
            )
            .expect("open search index"),
        ),
        embedder: Arc::new(synveda_ingest::embedding::AnyEmbedder::Deterministic(
            synveda_ingest::embedding::DeterministicEmbedder::new(),
        )),
        inject_embed_timeout: Duration::from_millis(100),
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

fn issue(subject: &str, tenant_id: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue(subject, tenant_id, Duration::from_secs(300))
}

/// Admits a tenant and opens a console session naming a freshly issued
/// token for `subject`. Returns the cookie secret the browser would hold.
///
/// Takes the whole state rather than the pool since TEN-4: the stored token
/// is **sealed** under the deployment key (ADR-0064 decision 5), so a fixture
/// that wrote plaintext would be writing a row the gateway cannot read.
async fn open_session(state: &AppState, subject: &str) -> (TenantId, String) {
    let tenant_id = TenantId::new();
    let slug = format!("cnsl1-{}", &tenant_id.to_string()[24..]);
    synveda_store::tenants::create(
        &state.pool,
        tenant_id,
        &slug,
        "CNSL-1",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
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
            &issue(subject, tenant_id),
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
    (tenant_id, secret.secret)
}

fn cookie(secret: &str) -> String {
    format!("{CONSOLE_COOKIE}={secret}")
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read response body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("json body")
}

// ── The seam ─────────────────────────────────────────────────────────────────

/// ADR-0056 decision 2, the whole of it: the cookie resolves to the claims
/// its stored token carries, and to nothing else. `whoami` is the right
/// probe precisely because it reports what tenant resolution concluded.
#[tokio::test]
async fn a_cookie_resolves_to_exactly_what_its_stored_token_says() {
    let _guard = serial().await;
    let Some(url) = db_url() else { return };
    let state = state(&url);
    let (tenant_id, secret) = open_session(&state, "reviewer@example.test").await;

    let response = router(state)
        .oneshot(
            Request::get("/v1/whoami")
                .header(header::COOKIE, cookie(&secret))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("route");

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["subject"], "reviewer@example.test");
    assert_eq!(body["tenant"]["id"], tenant_id.to_string());
}

/// A header is an explicit act and a cookie is an ambient one. A client
/// that sent a bearer meant that bearer; if the cookie could win, "which
/// credential authorised this" would be a question whose answer depends on
/// which header the gateway happened to read first.
#[tokio::test]
async fn a_presented_bearer_wins_over_a_cookie() {
    let _guard = serial().await;
    let Some(url) = db_url() else { return };
    let state = state(&url);
    let (_, secret) = open_session(&state, "cookie-subject@example.test").await;

    // A second tenant, named only by the bearer.
    let bearer_tenant = TenantId::new();
    let slug = format!("cnsl1b-{}", &bearer_tenant.to_string()[24..]);
    synveda_store::tenants::create(
        &state.pool,
        bearer_tenant,
        &slug,
        "CNSL-1 bearer",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");

    let response = router(state)
        .oneshot(
            Request::get("/v1/whoami")
                .header(header::COOKIE, cookie(&secret))
                .header(
                    header::AUTHORIZATION,
                    format!(
                        "Bearer {}",
                        issue("bearer-subject@example.test", bearer_tenant)
                    ),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("route");

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["subject"], "bearer-subject@example.test");
    assert_eq!(body["tenant"]["id"], bearer_tenant.to_string());
}

/// An unknown cookie is the uniform 401 (ADR-0008): the gateway is not an
/// existence oracle for session ids either.
#[tokio::test]
async fn an_unknown_cookie_is_the_uniform_401() {
    let _guard = serial().await;
    let Some(url) = db_url() else { return };
    let secret = mint().expect("mint").secret;

    let response = router(state(&url))
        .oneshot(
            Request::get("/v1/whoami")
                .header(header::COOKIE, cookie(&secret))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("route");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// The hard cap is enforced by the query, not by a caller remembering to
/// check it — migration 0034's `absolute_expires_at > now()`.
#[tokio::test]
async fn a_session_past_its_hard_cap_is_gone() {
    let _guard = serial().await;
    let Some(url) = db_url() else { return };
    let state = state(&url);
    let tenant_id = TenantId::new();
    let slug = format!("cnsl1x-{}", &tenant_id.to_string()[24..]);
    synveda_store::tenants::create(
        &state.pool,
        tenant_id,
        &slug,
        "CNSL-1",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    state
        .keys
        .provision(&state.pool, synveda_crypto::KeyScope::Deployment)
        .await
        .expect("provision the deployment key");
    let secret = mint().expect("mint");
    let sealed = state
        .seal_console_token(
            &secret.hash,
            synveda_crypto::Purpose::ConsoleAccessToken,
            &issue("late@example.test", tenant_id),
        )
        .await
        .expect("seal the access token");
    // Created in the past so the cap can be in the past too: the CHECK
    // refuses a cap that precedes creation, which is itself the point.
    sqlx::query(
        "insert into console_sessions (token_hash, issuer, access_token_sealed, \
         created_at, absolute_expires_at) values ($1, $2, $3, now() - interval '2 hours', \
         now() - interval '1 hour')",
    )
    .bind(&secret.hash[..])
    .bind("http://idp.test")
    .bind(sealed)
    .execute(&state.pool)
    .await
    .expect("insert an expired session");

    let response = router(state)
        .oneshot(
            Request::get("/v1/whoami")
                .header(header::COOKIE, cookie(&secret.secret))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("route");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ── Ambient authority (decision 4) ───────────────────────────────────────────

/// The CSRF defence. A cross-site form can make a browser send the cookie;
/// it cannot make the browser lie about `Origin`.
#[tokio::test]
async fn a_cookie_mutation_from_another_origin_is_refused() {
    let _guard = serial().await;
    let Some(url) = db_url() else { return };
    let state = state(&url);
    let (_, secret) = open_session(&state, "reviewer@example.test").await;

    let response = router(state)
        .oneshot(
            Request::post("/v1/admin/scopes")
                .header(header::COOKIE, cookie(&secret))
                .header(header::ORIGIN, "http://evil.test")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"parent_id":"11111111-1111-1111-1111-111111111111","kind":"org_unit","slug":"x","display_name":"x"}"#,
                ))
                .unwrap(),
        )
        .await
        .expect("route");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// A missing `Origin` is refused rather than allowed. Browsers have sent it
/// on cross-origin requests for years and on same-origin non-GET requests
/// since 2020; a caller that omits it is not a browser, and a caller that
/// is not a browser has a bearer available.
#[tokio::test]
async fn a_cookie_mutation_without_an_origin_is_refused() {
    let _guard = serial().await;
    let Some(url) = db_url() else { return };
    let state = state(&url);
    let (_, secret) = open_session(&state, "reviewer@example.test").await;

    let response = router(state)
        .oneshot(
            Request::post("/v1/admin/scopes")
                .header(header::COOKIE, cookie(&secret))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"parent_id":"11111111-1111-1111-1111-111111111111","kind":"org_unit","slug":"x","display_name":"x"}"#,
                ))
                .unwrap(),
        )
        .await
        .expect("route");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// The check is on the *transport*, not on the route: a bearer-authenticated
/// mutation carries no `Origin` and must be untouched, or CNSL-1 would have
/// broken the CLI, both adapters and every service identity at once.
#[tokio::test]
async fn a_bearer_mutation_needs_no_origin() {
    let _guard = serial().await;
    let Some(url) = db_url() else { return };
    let state = state(&url);
    let tenant_id = TenantId::new();
    let slug = format!("cnsl1c-{}", &tenant_id.to_string()[24..]);
    synveda_store::tenants::create(
        &state.pool,
        tenant_id,
        &slug,
        "CNSL-1",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");

    let response = router(state)
        .oneshot(
            Request::post("/v1/admin/scopes")
                .header(
                    header::AUTHORIZATION,
                    format!("Bearer {}", issue("agent@example.test", tenant_id)),
                )
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"parent_id":"11111111-1111-1111-1111-111111111111","kind":"org_unit","slug":"x","display_name":"x"}"#,
                ))
                .unwrap(),
        )
        .await
        .expect("route");

    // Whatever the PDP decides, it is not the transport check: reaching a
    // policy decision at all is the assertion.
    assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
}

/// A safe method is exempt, or ordinary navigation to the console would
/// need a header a browser does not send on a top-level GET.
#[tokio::test]
async fn a_cookie_read_needs_no_origin() {
    let _guard = serial().await;
    let Some(url) = db_url() else { return };
    let state = state(&url);
    let (_, secret) = open_session(&state, "reviewer@example.test").await;

    let response = router(state)
        .oneshot(
            Request::get("/v1/whoami")
                .header(header::COOKIE, cookie(&secret))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("route");

    assert_eq!(response.status(), StatusCode::OK);
}

// ── Sign-out ─────────────────────────────────────────────────────────────────

/// Sign-out destroys the row, and the next request is the uniform 401.
/// This is what migration 0034's DELETE grant is for, and the reason the
/// contrast with `skill_reviews` is drawn there: a credential that cannot
/// be destroyed cannot be revoked.
#[tokio::test]
async fn signing_out_destroys_the_session_for_the_next_request() {
    let _guard = serial().await;
    let Some(url) = db_url() else { return };
    let state = state(&url);
    let (_, secret) = open_session(&state, "reviewer@example.test").await;
    let app = router(state);

    let logout = app
        .clone()
        .oneshot(
            Request::post("/auth/console/logout")
                .header(header::COOKIE, cookie(&secret))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("route");
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);

    let cleared = logout
        .headers()
        .get(header::SET_COOKIE)
        .expect("sign-out clears the cookie")
        .to_str()
        .expect("ascii cookie");
    assert!(cleared.contains("Max-Age=0"), "got {cleared}");

    let after = app
        .oneshot(
            Request::get("/v1/whoami")
                .header(header::COOKIE, cookie(&secret))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("route");
    assert_eq!(after.status(), StatusCode::UNAUTHORIZED);
}

/// Signing out twice is signing out. A second click, a replayed request and
/// a session the gateway already reaped all end in the same place, because
/// a sign-out that errors leaves the operator unsure whether they are out.
#[tokio::test]
async fn signing_out_is_idempotent() {
    let _guard = serial().await;
    let Some(url) = db_url() else { return };
    let secret = mint().expect("mint").secret;

    let response = router(state(&url))
        .oneshot(
            Request::post("/auth/console/logout")
                .header(header::COOKIE, cookie(&secret))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("route");

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

/// Sign-out with no cookie at all still clears and still succeeds.
#[tokio::test]
async fn signing_out_without_a_cookie_is_not_an_error() {
    let _guard = serial().await;
    let Some(url) = db_url() else { return };

    let response = router(state(&url))
        .oneshot(
            Request::post("/auth/console/logout")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("route");

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

// ── The cookie itself ────────────────────────────────────────────────────────

/// Every attribute the design leans on, asserted on the bytes that reach the
/// browser. `HttpOnly` is what keeps an XSS from reading the session,
/// `Secure` and the `__Host-` prefix are what keep a sibling host from
/// setting one, and `SameSite=Strict` is the first line the `Origin` check
/// is the second of. A cookie missing any of them is this ADR silently
/// undone, and nothing else in the suite would notice.
#[tokio::test]
async fn the_cleared_cookie_carries_every_attribute_the_design_needs() {
    let _guard = serial().await;
    let Some(url) = db_url() else { return };

    let response = router(state(&url))
        .oneshot(
            Request::post("/auth/console/logout")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("route");

    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .expect("a Set-Cookie header")
        .to_str()
        .expect("ascii cookie");

    assert!(cookie.starts_with("__Host-"), "got {cookie}");
    assert!(cookie.contains("HttpOnly"), "got {cookie}");
    assert!(cookie.contains("Secure"), "got {cookie}");
    assert!(cookie.contains("SameSite=Strict"), "got {cookie}");
    assert!(cookie.contains("Path=/"), "got {cookie}");
    // `__Host-` forbids it, and a browser would reject the whole cookie.
    assert!(!cookie.contains("Domain="), "got {cookie}");
}
