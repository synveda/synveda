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
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, behavior_test_router as router};
use synveda_gateway::telemetry;
use synveda_identity::{LoginFlow, OidcVerifier, parse_issuers};
use synveda_store::{access, anchors, directory, identities, scopes};
use synveda_types::scope::ScopeKind;
use synveda_types::{DirectoryUserId, GrantId, IdentityId, IdentityKind, TenantId, TenantStatus};
use tower::ServiceExt;

const KEY_PEM: &str = include_str!("fixtures/idp_key_a.pem");
const KEY_JWK: &str = include_str!("fixtures/idp_key_a.jwk.json");
const CLIENT_ID: &str = "synveda-test";
const API_AUDIENCE: &str = "synveda-test-api";
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
    external_id: String,
    groups: Vec<String>,
    email: Option<String>,
    email_verified: bool,
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
                external_id: "nobody".to_owned(),
                groups: Vec::new(),
                email: None,
                email_verified: false,
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
        self.login_as_with_email_verification(subject, groups, email, email.is_some());
    }

    fn login_as_with_email_verification(
        &self,
        subject: &str,
        groups: &[&str],
        email: Option<&str>,
        email_verified: bool,
    ) {
        self.login_as_with_external_id(subject, subject, groups, email, email_verified);
    }

    fn login_as_with_external_id(
        &self,
        subject: &str,
        external_id: &str,
        groups: &[&str],
        email: Option<&str>,
        email_verified: bool,
    ) {
        *self.user.lock().unwrap() = CurrentUser {
            subject: subject.to_owned(),
            external_id: external_id.to_owned(),
            groups: groups.iter().map(|g| (*g).to_owned()).collect(),
            email: email.map(str::to_owned),
            email_verified,
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
            "oid": user.external_id,
            "aud": aud,
            "exp": now_secs() + 600,
            "iat": now_secs(),
            "tid": self.tid,
            "groups": user.groups,
            "name": user.subject,
        });
        if let Some(email) = &user.email {
            claims["email"] = json!(email);
            claims["email_verified"] = json!(user.email_verified);
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
        let claims = self.claims(&user, API_AUDIENCE, None);
        self.sign(&claims)
    }
}

async fn discovery(State(idp): State<MockIdp>) -> Json<Value> {
    Json(json!({
        "issuer": idp.issuer,
        "authorization_endpoint": format!("{}/authorize", idp.issuer),
        "token_endpoint": format!("{}/token", idp.issuer),
        "jwks_uri": format!("{}/jwks", idp.issuer),
        "code_challenge_methods_supported": ["S256"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code"],
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
    let access_claims = idp.claims(&auth.user, API_AUDIENCE, None);
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
    let config = format!(
        r#"[{{"issuer":"{issuer}","client_id":"{CLIENT_ID}","audience":"{API_AUDIENCE}","external_id_claim":"oid"}}]"#
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
/// `login_as` named) and returns the callback response.
async fn login_response(app: &Router) -> Response {
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
    app.clone()
        .oneshot(get_request(&callback, None))
        .await
        .unwrap()
}

async fn login(app: &Router) -> Value {
    let response = login_response(app).await;
    assert_eq!(response.status(), StatusCode::OK, "callback must succeed");
    body_json(response).await
}

async fn create_directory_mirror(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: TenantId,
    source: &str,
    external_id: &str,
    user_name: &str,
    identity_id: Option<IdentityId>,
) -> DirectoryUserId {
    let id = DirectoryUserId::new();
    directory::create_user(
        tx,
        id,
        tenant_id,
        &directory_user_attributes(source, external_id, user_name, true),
    )
    .await
    .expect("create directory mirror");
    if let Some(identity_id) = identity_id {
        directory::link_identity(tx, tenant_id, source, id, identity_id)
            .await
            .expect("link directory mirror");
    }
    id
}

fn directory_user_attributes(
    source: &str,
    external_id: &str,
    user_name: &str,
    active: bool,
) -> directory::UserAttributes {
    directory::UserAttributes {
        directory_source: source.to_owned(),
        external_id: Some(external_id.to_owned()),
        user_name: user_name.to_owned(),
        active,
        display_name: None,
        given_name: None,
        family_name: None,
        work_email: Some(user_name.to_owned()),
    }
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
    synveda_store::epoch::verify(&pool)
        .await
        .expect("apply migrations");
    let id = TenantId::new();
    let slug = format!("auth2-{}", id.as_uuid().simple());
    tenant_fixture::create(&pool, id, &slug, "AUTH-2 test tenant", TenantStatus::Active)
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
    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    let identity = identities::by_subject(&mut *tx, tenant_id, "alice-sub")
        .await
        .expect("read identity")
        .expect("alice is provisioned");
    assert_eq!(identity.email.as_deref(), Some("alice@example.test"));
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
        &mut tx,
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

#[tokio::test]
async fn unverified_email_cannot_adopt_a_waiting_directory_identity() {
    let _serial = serial().await;
    let Some((pool, tenant_id, db_url)) = admitted_tenant().await else {
        return;
    };
    let idp = MockIdp::spawn(tenant_id).await;
    let app = router(state(&db_url, &idp.issuer));

    let waiting_email = "waiting@example.test";
    let waiting_id = synveda_types::IdentityId::new();
    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    let waiting_scope = scopes::ensure_principal_scope(
        &mut tx,
        tenant_id,
        "directory-anchor-waiting",
        "Waiting directory identity",
    )
    .await
    .expect("create waiting principal scope");
    identities::create(
        &mut tx,
        waiting_id,
        tenant_id,
        None,
        synveda_types::IdentityKind::User,
        Some(waiting_email),
        Some("Waiting directory identity"),
        waiting_scope.id,
    )
    .await
    .expect("create waiting directory identity");
    create_directory_mirror(
        &mut tx,
        tenant_id,
        "scim",
        "verified-anchor-only",
        waiting_email,
        Some(waiting_id),
    )
    .await;
    tx.commit()
        .await
        .expect("commit waiting directory identity");

    idp.login_as_with_email_verification(
        "unverified-email-sub",
        &["everyone"],
        Some(waiting_email),
        false,
    );
    let session = login(&app).await;

    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    let waiting = identities::by_id(&mut *tx, tenant_id, waiting_id)
        .await
        .expect("read waiting identity")
        .expect("waiting identity remains");
    let provisioned = identities::by_subject(&mut *tx, tenant_id, "unverified-email-sub")
        .await
        .expect("read newly provisioned identity")
        .expect("unverified-email subject is provisioned independently");
    tx.commit().await.expect("commit identity reads");

    assert_eq!(
        waiting.subject.as_deref(),
        None,
        "unverified email must not bind the waiting row"
    );
    assert_ne!(
        provisioned.id, waiting.id,
        "unverified email must not adopt by address"
    );
    assert_eq!(
        provisioned.email, None,
        "an unverified claim must not be persisted as an identity address"
    );
    assert_eq!(
        session["identity"]["id"],
        json!(provisioned.id),
        "the login response must name the independently provisioned identity"
    );
}

#[tokio::test]
async fn pending_directory_projection_blocks_jit_until_the_mirror_is_linked() {
    let _serial = serial().await;
    let Some((pool, tenant_id, db_url)) = admitted_tenant().await else {
        return;
    };
    let idp = MockIdp::spawn(tenant_id).await;
    let app = router(state(&db_url, &idp.issuer));
    let subject = "pending-directory-sub";
    let external_id = "pending-directory-object-id";

    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    let mirror_id = create_directory_mirror(
        &mut tx,
        tenant_id,
        "entra",
        external_id,
        "pending-directory@example.test",
        None,
    )
    .await;
    tx.commit().await.expect("commit pending mirror");

    idp.login_as_with_external_id(subject, external_id, &["everyone"], None, false);
    let response = login_response(&app).await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let error = body_json(response).await;
    assert_eq!(error["kind"], "dependency");
    assert_eq!(error["service"], "directory-projection");

    let successor_id = IdentityId::new();
    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    assert!(
        identities::by_subject(&mut *tx, tenant_id, subject)
            .await
            .expect("read refused subject")
            .is_none(),
        "projection-in-progress must not mint a JIT identity"
    );
    let mirror = directory::user(&mut *tx, tenant_id, "entra", mirror_id)
        .await
        .expect("read pending mirror")
        .expect("pending mirror remains");
    assert_eq!(mirror.identity_id, None);
    let successor_scope = scopes::ensure_principal_scope(
        &mut tx,
        tenant_id,
        external_id,
        "Pending directory successor",
    )
    .await
    .expect("create successor scope");
    identities::create(
        &mut tx,
        successor_id,
        tenant_id,
        None,
        IdentityKind::User,
        Some("pending-directory@example.test"),
        Some("Pending directory successor"),
        successor_scope.id,
    )
    .await
    .expect("create successor identity");
    directory::link_identity(&mut tx, tenant_id, "entra", mirror_id, successor_id)
        .await
        .expect("link projected successor");
    tx.commit().await.expect("commit projected successor");

    let session = login(&app).await;
    assert_eq!(session["identity"]["id"], json!(successor_id));
    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    assert_eq!(
        identities::by_subject(&mut *tx, tenant_id, subject)
            .await
            .expect("read bound successor")
            .expect("successor is bound")
            .id,
        successor_id
    );
    let resolved = anchors::resolve(
        &mut tx,
        tenant_id,
        subject,
        Some(successor_id),
        anchors::AnchorSelection::none(),
    )
    .await
    .expect("resolve adopted principal authority");
    let home = resolved
        .as_slice()
        .iter()
        .find(|anchor| anchor.scope_id == successor_scope.id)
        .expect("adopted principal scope is an authority anchor");
    assert_eq!(home.roles, [synveda_types::access::RoleKey::Owner]);
    let owner_grants = access::list_grants(
        &mut *tx,
        tenant_id,
        &access::GrantFilter {
            scope_id: Some(successor_scope.id),
            principal_id: None,
        },
    )
    .await
    .expect("read transferred structural owner");
    assert!(owner_grants.iter().any(|grant| {
        grant.role_key == synveda_types::access::RoleKey::Owner
            && grant.source == synveda_types::access::GrantSource::Owner
            && grant.principal_id.as_deref() == Some(subject)
    }));
    assert!(
        !owner_grants
            .iter()
            .any(|grant| grant.principal_id.as_deref() == Some(external_id))
    );
    tx.commit().await.expect("commit successor read");
}

#[tokio::test]
async fn repeat_login_repairs_a_preexisting_directory_owner_anchor() {
    let _serial = serial().await;
    let Some((pool, tenant_id, db_url)) = admitted_tenant().await else {
        return;
    };
    let idp = MockIdp::spawn(tenant_id).await;
    let app = router(state(&db_url, &idp.issuer));
    let subject = "already-bound-pairwise-sub";
    let external_id = "already-bound-directory-object-id";
    let identity_id = IdentityId::new();

    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    let scope = scopes::ensure_principal_scope(
        &mut tx,
        tenant_id,
        external_id,
        "Already-bound directory identity",
    )
    .await
    .expect("create anchored principal scope");
    identities::create(
        &mut tx,
        identity_id,
        tenant_id,
        Some(subject),
        IdentityKind::User,
        Some("already-bound-pairwise@example.test"),
        Some("Already-bound directory identity"),
        scope.id,
    )
    .await
    .expect("seed identity bound by an earlier binary");
    create_directory_mirror(
        &mut tx,
        tenant_id,
        "entra",
        external_id,
        "already-bound-pairwise@example.test",
        Some(identity_id),
    )
    .await;
    tx.commit().await.expect("commit earlier binding shape");

    idp.login_as_with_external_id(subject, external_id, &["everyone"], None, false);
    let session = login(&app).await;
    assert_eq!(session["identity"]["id"], json!(identity_id));

    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    let grants = access::structural_owner_grants(&mut *tx, tenant_id, scope.id)
        .await
        .expect("read repaired owner");
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].principal_id.as_deref(), Some(subject));
    tx.commit().await.expect("commit repaired owner read");
}

#[tokio::test]
async fn owner_anchor_repair_normalizes_a_redundant_direct_owner_grant() {
    let _serial = serial().await;
    let Some((pool, tenant_id, db_url)) = admitted_tenant().await else {
        return;
    };
    let idp = MockIdp::spawn(tenant_id).await;
    let app = router(state(&db_url, &idp.issuer));
    let subject = "direct-owner-pairwise-sub";
    let external_id = "direct-owner-directory-object-id";
    let identity_id = IdentityId::new();

    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    let scope = scopes::ensure_principal_scope(
        &mut tx,
        tenant_id,
        external_id,
        "Direct-owner directory identity",
    )
    .await
    .expect("create anchored principal scope");
    identities::create(
        &mut tx,
        identity_id,
        tenant_id,
        Some(subject),
        IdentityKind::User,
        Some("direct-owner-pairwise@example.test"),
        Some("Direct-owner directory identity"),
        scope.id,
    )
    .await
    .expect("seed bound identity");
    create_directory_mirror(
        &mut tx,
        tenant_id,
        "entra",
        external_id,
        "direct-owner-pairwise@example.test",
        Some(identity_id),
    )
    .await;
    let direct_owner_id = GrantId::new();
    access::create_grant(
        &mut tx,
        &access::NewGrant {
            id: direct_owner_id,
            tenant_id,
            scope_id: scope.id,
            subject: synveda_types::access::GrantSubject::Principal {
                principal_id: subject.to_owned(),
            },
            role_key: synveda_types::access::RoleKey::Owner,
            source: synveda_types::access::GrantSource::Direct,
            invite_id: None,
            granted_by: Some("fixture-admin".to_owned()),
        },
    )
    .await
    .expect("seed redundant direct owner");
    tx.commit().await.expect("commit pre-cutover shape");

    idp.login_as_with_external_id(subject, external_id, &["everyone"], None, false);
    let session = login(&app).await;
    assert_eq!(session["identity"]["id"], json!(identity_id));

    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    let owner_grants = access::list_grants(
        &mut *tx,
        tenant_id,
        &access::GrantFilter {
            scope_id: Some(scope.id),
            principal_id: Some(subject.to_owned()),
        },
    )
    .await
    .expect("read normalized owner grants")
    .into_iter()
    .filter(|grant| grant.role_key == synveda_types::access::RoleKey::Owner)
    .collect::<Vec<_>>();
    assert_eq!(owner_grants.len(), 1, "owner authority is canonicalized");
    assert_eq!(
        owner_grants[0].source,
        synveda_types::access::GrantSource::Owner
    );
    assert_eq!(owner_grants[0].principal_id.as_deref(), Some(subject));
    assert!(
        access::get_grant(&mut *tx, tenant_id, direct_owner_id)
            .await
            .expect("read retired direct owner")
            .is_none(),
        "the redundant direct row is retired rather than hidden"
    );
    let structural = access::structural_owner_grants(&mut *tx, tenant_id, scope.id)
        .await
        .expect("read canonical structural owner");
    assert_eq!(structural.len(), 1);
    assert_eq!(
        structural[0].source,
        synveda_types::access::GrantSource::Owner
    );
    assert_eq!(structural[0].principal_id.as_deref(), Some(subject));
    tx.commit().await.expect("commit normalized owner read");
}

#[tokio::test]
async fn ambiguous_directory_anchor_is_not_treated_as_absent() {
    let _serial = serial().await;
    let Some((pool, tenant_id, db_url)) = admitted_tenant().await else {
        return;
    };
    let idp = MockIdp::spawn(tenant_id).await;
    let app = router(state(&db_url, &idp.issuer));
    let subject = "ambiguous-directory-sub";

    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    create_directory_mirror(
        &mut tx,
        tenant_id,
        "entra",
        subject,
        "ambiguous-entra@example.test",
        None,
    )
    .await;
    create_directory_mirror(
        &mut tx,
        tenant_id,
        "okta",
        subject,
        "ambiguous-okta@example.test",
        None,
    )
    .await;
    tx.commit().await.expect("commit ambiguous mirrors");

    idp.login_as(subject, &["everyone"], None);
    let response = login_response(&app).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(body_json(response).await["kind"], "unauthenticated");
    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    assert!(
        identities::by_subject(&mut *tx, tenant_id, subject)
            .await
            .expect("read ambiguous subject")
            .is_none(),
        "an ambiguous strong anchor must not fall back to JIT"
    );
    tx.commit().await.expect("commit ambiguous read");
}

#[tokio::test]
async fn inactive_directory_correspondence_never_falls_through_to_jit() {
    let _serial = serial().await;
    let Some((pool, tenant_id, db_url)) = admitted_tenant().await else {
        return;
    };
    let idp = MockIdp::spawn(tenant_id).await;
    let app = router(state(&db_url, &idp.issuer));

    let unprojected_subject = "inactive-unprojected-sub";
    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    directory::create_user(
        &mut tx,
        DirectoryUserId::new(),
        tenant_id,
        &directory_user_attributes(
            "entra",
            unprojected_subject,
            "inactive-unprojected@example.test",
            false,
        ),
    )
    .await
    .expect("create retained inactive mirror");
    tx.commit().await.expect("commit inactive mirror");

    idp.login_as(unprojected_subject, &["synveda-admins"], None);
    let response = login_response(&app).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    assert!(
        identities::by_subject(&mut *tx, tenant_id, unprojected_subject)
            .await
            .expect("read inactive subject")
            .is_none(),
        "an inactive pre-login mirror must block JIT"
    );
    assert!(
        access::list_grants(
            &mut *tx,
            tenant_id,
            &access::GrantFilter {
                scope_id: None,
                principal_id: Some(unprojected_subject.to_owned()),
            },
        )
        .await
        .expect("read refused admin grants")
        .is_empty(),
        "the refused admin-group claim must not commit a root grant"
    );
    tx.commit().await.expect("commit inactive read");

    let transition_subject = "inactive-transition-sub";
    let linked_id = IdentityId::new();
    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    let linked_scope = scopes::ensure_principal_scope(
        &mut tx,
        tenant_id,
        "inactive-transition-anchor",
        "Inactive transition identity",
    )
    .await
    .expect("create transition scope");
    identities::create(
        &mut tx,
        linked_id,
        tenant_id,
        None,
        IdentityKind::User,
        Some("inactive-transition@example.test"),
        Some("Inactive transition identity"),
        linked_scope.id,
    )
    .await
    .expect("create transition identity");
    let mirror_id = create_directory_mirror(
        &mut tx,
        tenant_id,
        "okta",
        transition_subject,
        "inactive-transition@example.test",
        Some(linked_id),
    )
    .await;
    directory::replace_user(
        &mut tx,
        tenant_id,
        mirror_id,
        &directory_user_attributes(
            "okta",
            transition_subject,
            "inactive-transition@example.test",
            false,
        ),
    )
    .await
    .expect("deactivate mirror")
    .expect("mirror remains");
    tx.commit()
        .await
        .expect("commit deactivation before reconciliation");

    idp.login_as(transition_subject, &["synveda-admins"], None);
    let response = login_response(&app).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    assert!(
        identities::by_subject(&mut *tx, tenant_id, transition_subject)
            .await
            .expect("read transition subject")
            .is_none(),
        "a committed deactivation must block JIT before reconciliation"
    );
    assert_eq!(
        identities::by_id(&mut *tx, tenant_id, linked_id)
            .await
            .expect("read linked transition identity")
            .expect("linked identity remains")
            .subject,
        None,
        "the failed login must not bind the inactive mirror"
    );
    assert!(
        access::list_grants(
            &mut *tx,
            tenant_id,
            &access::GrantFilter {
                scope_id: None,
                principal_id: Some(transition_subject.to_owned()),
            },
        )
        .await
        .expect("read transition admin grants")
        .is_empty(),
        "the deactivation window must not commit an admin grant"
    );
    tx.commit().await.expect("commit transition reads");
}

#[tokio::test]
async fn bound_directory_identity_is_refused_immediately_after_mirror_deactivation() {
    let _serial = serial().await;
    let Some((pool, tenant_id, db_url)) = admitted_tenant().await else {
        return;
    };
    let idp = MockIdp::spawn(tenant_id).await;
    let app = router(state(&db_url, &idp.issuer));
    let subject = "bound-inactive-sub";
    let identity_id = IdentityId::new();

    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    let scope =
        scopes::ensure_principal_scope(&mut tx, tenant_id, subject, "Bound inactive identity")
            .await
            .expect("create bound scope");
    identities::create(
        &mut tx,
        identity_id,
        tenant_id,
        Some(subject),
        IdentityKind::User,
        Some("bound-inactive@example.test"),
        Some("Bound inactive identity"),
        scope.id,
    )
    .await
    .expect("create bound identity");
    let mirror_id = create_directory_mirror(
        &mut tx,
        tenant_id,
        "entra",
        subject,
        "bound-inactive@example.test",
        Some(identity_id),
    )
    .await;
    directory::replace_user(
        &mut tx,
        tenant_id,
        mirror_id,
        &directory_user_attributes("entra", subject, "bound-inactive@example.test", false),
    )
    .await
    .expect("deactivate bound mirror")
    .expect("bound mirror remains");
    tx.commit()
        .await
        .expect("commit bound deactivation before reconciliation");

    idp.login_as(subject, &["synveda-admins"], None);
    let response = login_response(&app).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    let identity = identities::by_id(&mut *tx, tenant_id, identity_id)
        .await
        .expect("read bound identity")
        .expect("bound identity remains");
    assert!(
        !identity.sealed(),
        "reconciliation has deliberately not run"
    );
    assert_eq!(identity.subject.as_deref(), Some(subject));
    assert!(
        access::list_grants(
            &mut *tx,
            tenant_id,
            &access::GrantFilter {
                scope_id: None,
                principal_id: Some(subject.to_owned()),
            },
        )
        .await
        .expect("read bound subject grants")
        .iter()
        .all(|grant| grant.role_key != synveda_types::access::RoleKey::Administrator),
        "the deactivation window must not establish admin-group authority"
    );
    tx.commit().await.expect("commit bound refusal read");
}

#[tokio::test]
async fn directory_anchor_bound_to_another_subject_is_refused_without_jit() {
    let _serial = serial().await;
    let Some((pool, tenant_id, db_url)) = admitted_tenant().await else {
        return;
    };
    let idp = MockIdp::spawn(tenant_id).await;
    let app = router(state(&db_url, &idp.issuer));
    let subject = "claiming-directory-sub";
    let bound_id = IdentityId::new();

    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    let bound_scope = scopes::ensure_principal_scope(
        &mut tx,
        tenant_id,
        "already-bound-directory-anchor",
        "Already-bound directory identity",
    )
    .await
    .expect("create bound scope");
    identities::create(
        &mut tx,
        bound_id,
        tenant_id,
        Some("different-directory-sub"),
        IdentityKind::User,
        Some("already-bound@example.test"),
        Some("Already-bound directory identity"),
        bound_scope.id,
    )
    .await
    .expect("create bound identity");
    create_directory_mirror(
        &mut tx,
        tenant_id,
        "entra",
        subject,
        "already-bound@example.test",
        Some(bound_id),
    )
    .await;
    tx.commit().await.expect("commit bound mirror");

    idp.login_as(subject, &["everyone"], None);
    let response = login_response(&app).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    assert!(
        identities::by_subject(&mut *tx, tenant_id, subject)
            .await
            .expect("read claiming subject")
            .is_none(),
        "a mirror already bound elsewhere must not mint a second identity"
    );
    assert_eq!(
        identities::by_id(&mut *tx, tenant_id, bound_id)
            .await
            .expect("read bound identity")
            .expect("bound identity remains")
            .subject
            .as_deref(),
        Some("different-directory-sub")
    );
    tx.commit().await.expect("commit bound reads");
}

#[tokio::test]
async fn departed_subject_can_bind_only_a_projected_directory_successor() {
    let _serial = serial().await;
    let Some((pool, tenant_id, db_url)) = admitted_tenant().await else {
        return;
    };
    let idp = MockIdp::spawn(tenant_id).await;
    let app = router(state(&db_url, &idp.issuer));
    let subject = "rehired-directory-sub";
    let departed_id = IdentityId::new();

    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    let departed_scope = scopes::ensure_principal_scope(
        &mut tx,
        tenant_id,
        "departed-directory-oid",
        "Departed directory identity",
    )
    .await
    .expect("create departed scope");
    identities::create(
        &mut tx,
        departed_id,
        tenant_id,
        Some(subject),
        IdentityKind::User,
        Some("rehired@example.test"),
        Some("Departed directory identity"),
        departed_scope.id,
    )
    .await
    .expect("create departed identity");
    let old_structural_owner =
        access::structural_owner_grants(&mut *tx, tenant_id, departed_scope.id)
            .await
            .expect("read departed structural owner")
            .into_iter()
            .next()
            .expect("departed structural owner");
    access::revoke_grant(&mut tx, tenant_id, old_structural_owner.id)
        .await
        .expect("retire provider-anchor owner");
    access::create_grant(
        &mut tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id,
            scope_id: departed_scope.id,
            subject: synveda_types::access::GrantSubject::Principal {
                principal_id: subject.to_owned(),
            },
            role_key: synveda_types::access::RoleKey::Owner,
            source: synveda_types::access::GrantSource::Owner,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .expect("bind departed structural owner to token subject");
    let root = scopes::ensure_tenant_root(&mut tx, tenant_id)
        .await
        .expect("read tenant root");
    access::create_grant(
        &mut tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id,
            scope_id: root.id,
            subject: synveda_types::access::GrantSubject::Principal {
                principal_id: subject.to_owned(),
            },
            role_key: synveda_types::access::RoleKey::Curator,
            source: synveda_types::access::GrantSource::Direct,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .expect("grant former direct authority");
    identities::depart(&mut tx, tenant_id, departed_id)
        .await
        .expect("depart identity")
        .expect("active identity departed");
    let mirror_id = create_directory_mirror(
        &mut tx,
        tenant_id,
        "entra",
        subject,
        "rehired@example.test",
        Some(departed_id),
    )
    .await;
    tx.commit().await.expect("commit departed mirror");

    idp.login_as(subject, &["everyone"], None);
    let response = login_response(&app).await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

    let successor_id = IdentityId::new();
    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    let successor_scope =
        scopes::ensure_principal_scope(&mut tx, tenant_id, subject, "Rehired directory successor")
            .await
            .expect("create rehire successor scope");
    identities::create(
        &mut tx,
        successor_id,
        tenant_id,
        None,
        IdentityKind::User,
        Some("rehired@example.test"),
        Some("Rehired directory successor"),
        successor_scope.id,
    )
    .await
    .expect("create rehire successor");
    directory::link_identity(&mut tx, tenant_id, "entra", mirror_id, successor_id)
        .await
        .expect("link rehire successor");
    tx.commit().await.expect("commit rehire successor");

    let session = login(&app).await;
    assert_eq!(session["identity"]["id"], json!(successor_id));
    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    let departed = identities::by_id(&mut *tx, tenant_id, departed_id)
        .await
        .expect("read departed identity")
        .expect("departed identity remains");
    let successor = identities::by_id(&mut *tx, tenant_id, successor_id)
        .await
        .expect("read successor")
        .expect("successor remains");
    assert!(departed.sealed());
    assert_eq!(
        departed.subject, None,
        "the successor released the old subject"
    );
    assert_eq!(successor.subject.as_deref(), Some(subject));
    let successor_grants = access::list_grants(
        &mut *tx,
        tenant_id,
        &access::GrantFilter {
            scope_id: None,
            principal_id: Some(subject.to_owned()),
        },
    )
    .await
    .expect("read successor authority");
    assert_eq!(
        successor_grants.len(),
        1,
        "rehire must not revive the old home-owner or arbitrary direct grant"
    );
    assert_eq!(successor_grants[0].scope_id, successor_scope.id);
    assert_eq!(
        successor_grants[0].role_key,
        synveda_types::access::RoleKey::Owner
    );
    assert_eq!(
        successor_grants[0].source,
        synveda_types::access::GrantSource::Owner
    );
    tx.commit().await.expect("commit rehire reads");
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
    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
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

    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
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
    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
    let skipped = identities::by_subject(&mut *tx, tenant_id, "dave-sub")
        .await
        .expect("read identity");
    tx.commit()
        .await
        .expect("commit skipped-login identity read");
    assert_eq!(
        skipped, None,
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

    let mut tx = tenant_fixture::begin(&pool, tenant_id).await;
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
/// The one-time convention can first be claimed on a later login: a
/// directory-created identity whose subject is already bound, or the first
/// tenant member added to `synveda-admins` after joining, reaches the door down
/// the `bound` branch. That branch looks read-only and is not. This test proves
/// that a winning claim and its `access.granted` event are committed there.
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

    // A third login observes the closed bootstrap, not a second grant.
    let _ = login(&app).await;
    assert_eq!(
        admin_door_grants_of(&pool, tenant_id, "late-admin").await,
        1,
        "the one-time convention is idempotent"
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

/// An IdP group is only an initial-administrator signal. Once consumed, later
/// `synveda-admins` members need a governed Synveda grant, and revoking the
/// original grant must not hand authority back to the provider.
#[tokio::test]
async fn later_admin_group_members_cannot_reopen_consumed_bootstrap() {
    let _serial = serial().await;
    let Some((pool, tenant_id, db_url)) = admitted_tenant().await else {
        return;
    };
    let idp = MockIdp::spawn(tenant_id).await;
    let app = router(state(&db_url, &idp.issuer));

    idp.login_as(
        "initial-admin",
        &["everyone", "synveda-admins"],
        Some("initial-admin@example.test"),
    );
    let _ = login(&app).await;
    let initial_grants = admin_door_grant_ids_of(&pool, tenant_id, "initial-admin").await;
    assert_eq!(
        initial_grants.len(),
        1,
        "the first qualifying login claims initial administration"
    );

    idp.login_as(
        "later-admin",
        &["everyone", "synveda-admins"],
        Some("later-admin@example.test"),
    );
    let later_session = login(&app).await;
    assert_eq!(
        admin_door_grants_of(&pool, tenant_id, "later-admin").await,
        0,
        "a later provider group member cannot mint Synveda authority"
    );
    let later_token = later_session["access_token"]
        .as_str()
        .expect("access_token");
    let response = app
        .clone()
        .oneshot(get_request("/v1/admin/scopes", Some(later_token)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant_id)
        .await
        .expect("begin tenant tx");
    access::revoke_grant(&mut tx, tenant_id, initial_grants[0])
        .await
        .expect("revoke initial administrator grant");
    tx.commit().await.expect("commit revocation");
    assert_eq!(
        admin_door_grants_of(&pool, tenant_id, "initial-admin").await,
        0,
        "the original administrator grant is revoked"
    );

    let later_session = login(&app).await;
    assert_eq!(
        admin_door_grants_of(&pool, tenant_id, "later-admin").await,
        0,
        "revocation does not reopen provider-controlled bootstrap"
    );
    let later_token = later_session["access_token"]
        .as_str()
        .expect("access_token");
    let response = app
        .clone()
        .oneshot(get_request("/v1/admin/scopes", Some(later_token)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let exposition = metrics_handle().render();
    for outcome in ["claimed", "closed"] {
        assert!(
            exposition.lines().any(|line| {
                line.starts_with("synveda_jit_admin_bootstraps_total")
                    && line.contains(&format!("outcome=\"{outcome}\""))
            }),
            "administrator bootstrap outcome {outcome} missing:\n{exposition}"
        );
    }
}

/// How many grants `principal` holds **at the tenant root** — the admin
/// door, deliberately excluding the `owner` grant every principal scope
/// carries at itself (ADR-0074 decision 8), which is not what this test
/// is about.
async fn admin_door_grants_of(pool: &PgPool, tenant_id: TenantId, principal: &str) -> usize {
    admin_door_grant_ids_of(pool, tenant_id, principal)
        .await
        .len()
}

async fn admin_door_grant_ids_of(
    pool: &PgPool,
    tenant_id: TenantId,
    principal: &str,
) -> Vec<GrantId> {
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
    grants.into_iter().map(|grant| grant.id).collect()
}
