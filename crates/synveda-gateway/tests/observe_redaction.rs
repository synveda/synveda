//! MEM-2 acceptance criteria (ADR-0021): seeded secrets never reach
//! storage in any mode — deny, redact, or quarantine — and the
//! quarantine review queue works end to end.
//!
//! The storage sweep is adversarial: after driving seeded secrets and
//! PII through `/v1/observe` under each mode, every surface that could
//! have persisted them — staging payloads and redaction summaries, the
//! quarantine queue, both PGMQ tables, and the audit chain — is searched
//! for the literals. The review E2E exercises `QuarantineRead`/
//! `QuarantineReview` through the PDP proper: steward and
//! security-reviewer (its first live action) adjudicate; the owner holds
//! no self-release; release sends the standard work signal; review is
//! one-shot; redelivery of a quarantined event neither re-quarantines
//! nor signals.
//!
//! Tests need a live Postgres: they read `DATABASE_URL` and skip when it
//! is unset (CI has no database), same convention as tests/observe.rs
//! (whose harness this copies).

use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::response::Response;
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_audit::ChainVerification;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::{OidcVerifier, parse_issuers, personal_slug};
use synveda_store::{
    hierarchy, identities, policy_assignments, policy_packs, rls, role_bindings, tenants,
};
use synveda_types::{
    HierarchyNode, Identity, IdentityId, IdentityKind, PackConfig, RedactionConfig, RedactionMode,
    Role, ScopeId, ScopeKind, TenantId, TenantStatus,
};
use tower::ServiceExt;

const KEY_PEM: &str = include_str!("fixtures/idp_key_a.pem");
const KEY_JWK: &str = include_str!("fixtures/idp_key_a.jwk.json");
const CLIENT_ID: &str = "synveda-test";

// The seeded findings — well-known documentation examples, never real
// credentials. The AWS pair and the Luhn-valid test PAN are the exact
// values the vendor docs publish for this purpose.
const SEEDED_AWS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
const SEEDED_GITHUB_TOKEN: &str = "ghp_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789";
const SEEDED_CARD: &str = "4111 1111 1111 1111";
const SEEDED_EMAIL: &str = "leaky.human@example.com";

/// Serialises tests: the Prometheus recorder and tracing's
/// callsite-interest cache are process-global.
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

// ── Mock IdP (user tokens only — no service flow needed here) ───────────────

struct MockIdp {
    issuer: String,
}

impl MockIdp {
    async fn spawn() -> Self {
        use axum::extract::State;
        use axum::response::Json;
        use axum::routing::get;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock idp");
        let addr = listener.local_addr().expect("mock idp addr");
        let issuer = format!("http://{addr}/mock-idp");

        async fn discovery(State(issuer): State<String>) -> Json<Value> {
            Json(json!({
                "issuer": issuer,
                "authorization_endpoint": format!("{issuer}/authorize"),
                "token_endpoint": format!("{issuer}/token"),
                "jwks_uri": format!("{issuer}/jwks"),
            }))
        }
        async fn jwks() -> Json<Value> {
            Json(json!({ "keys": [serde_json::from_str::<Value>(KEY_JWK).expect("jwk fixture")] }))
        }
        let app = Router::new()
            .route("/mock-idp/.well-known/openid-configuration", get(discovery))
            .route("/mock-idp/jwks", get(jwks))
            .with_state(issuer.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("mock idp serve");
        });
        Self { issuer }
    }

    fn user_token(&self, subject: &str) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("key-a".to_owned());
        let key = EncodingKey::from_rsa_pem(KEY_PEM.as_bytes()).expect("test key");
        jsonwebtoken::encode(
            &header,
            &json!({
                "iss": self.issuer,
                "sub": subject,
                "aud": CLIENT_ID,
                "iat": now_secs(),
                "exp": now_secs() + 600,
            }),
            &key,
        )
        .expect("sign token")
    }
}

// ── Gateway harness ──────────────────────────────────────────────────────────

fn state(url: &str, issuer: &str, tenant: TenantId) -> AppState {
    let config = format!(
        r#"[{{"issuer":"{issuer}","client_id":"{CLIENT_ID}",
             "tenant":{{"static":{{"tenant_id":"{tenant}"}}}}}}]"#
    );
    let verifier = Arc::new(
        OidcVerifier::new(parse_issuers(&config).expect("issuer config")).expect("build verifier"),
    );
    AppState {
        pool: PgPoolOptions::new()
            .max_connections(4)
            .connect_lazy(url)
            .expect("parse database url"),
        metrics: metrics_handle(),
        verifier,
        login: None,
        public_origin: "http://127.0.0.1:8120".to_owned(),
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

async fn admitted_tenant() -> Option<(PgPool, TenantId, String)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping MEM-2 redaction test: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
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
    let id = TenantId::new();
    let slug = format!("mem2-{}", id.as_uuid().simple());
    tenants::create(&pool, id, &slug, "MEM-2 test tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((pool, id, url))
}

/// Seeds acme-org → eng (dept) → platform (team). Returns (org, eng,
/// platform).
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

/// Provisions a user identity at the store level (the JIT shape).
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

async fn bind_role(pool: &PgPool, tenant: TenantId, subject: &str, role: Role) {
    let mut tx = rls::begin_tenant_tx(pool, tenant).await.expect("tx");
    role_bindings::bind(&mut *tx, tenant, subject, None, role)
        .await
        .expect("bind role");
    tx.commit().await.expect("commit binding");
}

fn event(key: &str, text: &str) -> Value {
    json!({
        "idempotency_key": key,
        "kind": "transcript_delta",
        "payload": {"text": text},
        "occurred_at": chrono::Utc::now().to_rfc3339(),
    })
}

fn batch(session: &str, events: Vec<Value>) -> Value {
    json!({ "session_id": session, "events": events })
}

/// Queue signals for `tenant` (live queue plus archive — a consumed
/// signal must count too).
async fn queued(pool: &PgPool, tenant: TenantId) -> i64 {
    sqlx::query_scalar!(
        r#"
        select (select count(*) from pgmq.q_observe
                where message ->> 'tenant_id' = $1)
             + (select count(*) from pgmq.a_observe
                where message ->> 'tenant_id' = $1) as "count!"
        "#,
        tenant.to_string(),
    )
    .fetch_one(pool)
    .await
    .expect("count queue signals")
}

/// The AC's adversarial sweep: every storage surface the tenant's data
/// touched, searched for a seeded literal (spaces stripped too, so a
/// reformatted card cannot hide). Runs on the RLS-exempt test
/// connection — if the secret is anywhere, this finds it.
async fn storage_contains(pool: &PgPool, tenant: TenantId, seed: &str) -> bool {
    let compact = seed.replace(' ', "");
    sqlx::query_scalar!(
        r#"
        select exists (
            select 1 from observe_events
            where tenant_id = $1
              and (payload::text like '%' || $2 || '%'
                   or payload::text like '%' || $3 || '%'
                   or coalesce(redactions::text, '') like '%' || $2 || '%')
            union all
            select 1 from observe_quarantine
            where tenant_id = $1
              and (findings::text like '%' || $2 || '%'
                   or coalesce(review_reason, '') like '%' || $2 || '%')
            union all
            select 1 from audit_log
            where tenant_id = $1
              and (payload::text like '%' || $2 || '%'
                   or resource like '%' || $2 || '%')
            union all
            select 1 from pgmq.q_observe
            where message ->> 'tenant_id' = $4
              and message::text like '%' || $2 || '%'
            union all
            select 1 from pgmq.a_observe
            where message ->> 'tenant_id' = $4
              and message::text like '%' || $2 || '%'
        ) as "found!"
        "#,
        tenant.as_uuid(),
        seed,
        compact,
        tenant.to_string(),
    )
    .fetch_one(pool)
    .await
    .expect("sweep storage")
}

async fn assert_storage_clean(pool: &PgPool, tenant: TenantId) {
    for seed in [
        SEEDED_AWS_KEY,
        SEEDED_GITHUB_TOKEN,
        SEEDED_CARD,
        SEEDED_EMAIL,
    ] {
        assert!(
            !storage_contains(pool, tenant, seed).await,
            "seeded literal {seed:?} reached storage"
        );
    }
}

// ── The AC: seeded secrets never reach storage, in any mode ─────────────────

/// Quarantine mode (regulated-strict, the zero-config default): a secret
/// stages redacted and signal-less behind a pending review; PII redacts
/// and flows. Redact mode (standard): the secret redacts and flows.
/// Deny mode (a stored custom pack): the event is refused per event and
/// nothing persists. After all three, the storage sweep finds no seeded
/// literal anywhere.
#[tokio::test]
async fn seeded_secrets_never_reach_storage_in_any_mode() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let (org, _, platform) = seed_hierarchy(&pool, tenant).await;
    let idp = MockIdp::spawn().await;
    let app_state = state(&db_url, &idp.issuer, tenant);
    let pdp = Arc::clone(&app_state.pdp);
    let app = router(app_state);
    seed_user(&pool, tenant, "alice", platform.id).await;
    let alice = idp.user_token("alice");

    // ── Quarantine mode: the embedded default, no assignment needed. ──
    let (status, body) = send(
        &app,
        request(
            Method::POST,
            "/v1/observe",
            &alice,
            Some(batch(
                "quarantine-session",
                vec![
                    event("q-secret", &format!("creds: {SEEDED_AWS_KEY} ok")),
                    event(
                        "q-pii",
                        &format!("mail {SEEDED_EMAIL} and card {SEEDED_CARD}"),
                    ),
                    event("q-clean", "refactored the resolver; tests green"),
                ],
            )),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["accepted"], 2, "{body}");
    assert_eq!(body["quarantined"], 1, "{body}");
    assert_eq!(body["denied"], 0, "{body}");
    assert_eq!(body["events"][0]["status"], "quarantined", "{body}");
    assert_eq!(body["events"][1]["status"], "accepted", "{body}");
    assert_eq!(body["events"][2]["status"], "accepted", "{body}");
    assert!(
        body["events"][1]["redactions"].is_array(),
        "the PII event reports its finding summary: {body}"
    );
    assert!(
        body["events"][2].get("redactions").is_none(),
        "a clean event carries no redactions field: {body}"
    );
    let quarantined_id = body["events"][0]["event_id"]
        .as_str()
        .expect("quarantined event id")
        .to_owned();
    // The quarantined event sent no signal: only the two accepted did.
    assert_eq!(queued(&pool, tenant).await, 2, "quarantine must not signal");
    // The staged PII payload holds placeholders, not the finding.
    let pii_payload = sqlx::query_scalar!(
        r#"select payload::text as "payload!" from observe_events
           where tenant_id = $1 and idempotency_key = 'q-pii'"#,
        tenant.as_uuid(),
    )
    .fetch_one(&pool)
    .await
    .expect("read staged payload");
    assert!(
        pii_payload.contains("[REDACTED:email]") && pii_payload.contains("[REDACTED:payment-card]"),
        "staged payload must carry placeholders: {pii_payload}"
    );

    // ── Redact mode: standard assigned at the org root. ──
    let mut tx = rls::begin_tenant_tx(&pool, tenant).await.expect("tx");
    policy_assignments::assign(&mut *tx, tenant, org.id, "standard")
        .await
        .expect("assign standard");
    tx.commit().await.expect("commit assignment");
    let (status, body) = send(
        &app,
        request(
            Method::POST,
            "/v1/observe",
            &alice,
            Some(batch(
                "redact-session",
                vec![event(
                    "r-secret",
                    &format!("token {SEEDED_GITHUB_TOKEN} pasted"),
                )],
            )),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["accepted"], 1, "{body}");
    assert_eq!(
        body["quarantined"], 0,
        "under standard a secret redacts and flows: {body}"
    );
    assert_eq!(queued(&pool, tenant).await, 3, "the redacted event signals");

    // ── Deny mode: a stored custom pack, hot-installed like the
    //    refresher would, assigned at the org root. ──
    const MEMBER_PACK: &str = r#"
        permit (principal, action == Synveda::Action::"MemoryRead", resource)
        when { principal in resource };
        permit (principal, action == Synveda::Action::"MemoryWrite", resource)
        when { principal has home && resource == principal.home };
    "#;
    let deny_config = RedactionConfig {
        secrets: RedactionMode::Deny,
        pii: RedactionMode::Redact,
    };
    let mut tx = rls::begin_tenant_tx(&pool, tenant).await.expect("tx");
    policy_packs::apply(
        &mut *tx,
        tenant,
        "acme-deny",
        MEMBER_PACK,
        &PackConfig {
            redaction: Some(deny_config),
            ..Default::default()
        },
    )
    .await
    .expect("store deny pack");
    policy_assignments::assign(&mut *tx, tenant, org.id, "acme-deny")
        .await
        .expect("assign deny pack");
    tx.commit().await.expect("commit deny pack");
    synveda_gateway::authz::refresh_tenant_packs(&pool, &pdp, tenant).await;

    let staged_before = sqlx::query_scalar!(
        r#"select count(*) as "count!" from observe_events where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&pool)
    .await
    .expect("count staged");
    let deny_batch = batch(
        "deny-session",
        vec![
            event("d-secret", &format!("here is {SEEDED_AWS_KEY} again")),
            event("d-clean", "sibling events still admit"),
        ],
    );
    let (status, body) = send(
        &app,
        request(
            Method::POST,
            "/v1/observe",
            &alice,
            Some(deny_batch.clone()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["denied"], 1, "{body}");
    assert_eq!(body["accepted"], 1, "{body}");
    assert_eq!(body["events"][0]["status"], "denied", "{body}");
    assert!(
        body["events"][0].get("event_id").is_none(),
        "a denied event has no id — nothing was persisted: {body}"
    );
    assert_eq!(body["events"][1]["status"], "accepted", "{body}");
    let staged_after = sqlx::query_scalar!(
        r#"select count(*) as "count!" from observe_events where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&pool)
    .await
    .expect("count staged");
    assert_eq!(
        staged_after,
        staged_before + 1,
        "only the clean sibling staged"
    );
    // A retry of the denied batch re-scans deterministically: the denied
    // event denies again, the sibling reports duplicate.
    let (status, retry) = send(
        &app,
        request(Method::POST, "/v1/observe", &alice, Some(deny_batch)),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{retry}");
    assert_eq!(retry["events"][0]["status"], "denied", "{retry}");
    assert_eq!(retry["events"][1]["status"], "duplicate", "{retry}");

    // ── The sweep: no seeded literal anywhere, after all three modes. ──
    assert_storage_clean(&pool, tenant).await;

    // The audit trail recorded the outcomes — counts and rule ids only.
    let mut tx = rls::begin_tenant_tx(&pool, tenant).await.expect("tx");
    let events = synveda_audit::tail(&mut tx, tenant, 100)
        .await
        .expect("read chain");
    let observed: Vec<_> = events
        .iter()
        .filter(|event| event.action == "memory.observed")
        .collect();
    assert_eq!(observed.len(), 4, "one chained event per batch");
    let quarantine_batch = observed
        .iter()
        .find(|event| event.payload["session_id"] == "quarantine-session")
        .expect("quarantine batch event");
    assert_eq!(quarantine_batch.payload["quarantined"], 1);
    assert_eq!(
        quarantine_batch.payload["redactions"]["aws-access-key-id"], 1,
        "the rule summary rides the chain: {}",
        quarantine_batch.payload
    );
    let deny_batch_event = observed
        .iter()
        .find(|event| event.payload["session_id"] == "deny-session")
        .expect("deny batch event");
    assert_eq!(deny_batch_event.payload["denied"], 1);
    let verification = synveda_audit::verify(&mut tx, tenant)
        .await
        .expect("verify chain");
    assert!(
        matches!(verification, ChainVerification::Valid { .. }),
        "the chain must verify: {verification:?}"
    );
    drop(tx);

    // ── The review queue on the quarantined event from mode one. ──
    // (Continues in this test so the sweep above covered a pending row.)
    // The deny pack was a minimal member pack without the quarantine
    // plane; unassign it so review decides under the embedded default.
    let mut tx = rls::begin_tenant_tx(&pool, tenant).await.expect("tx");
    policy_assignments::unassign(&mut *tx, tenant, org.id)
        .await
        .expect("unassign deny pack");
    tx.commit().await.expect("commit unassign");
    seed_user(&pool, tenant, "stew", platform.id).await;
    bind_role(&pool, tenant, "stew", Role::Steward).await;
    let steward = idp.user_token("stew");

    // The owner holds no review right: the queue and both verdicts are
    // denied (self-release would defeat review, ADR-0021 decision 6).
    let (status, body) = send(&app, request(Method::GET, "/v1/quarantine", &alice, None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "owner must not list: {body}");
    let (status, body) = send(
        &app,
        request(
            Method::POST,
            &format!("/v1/quarantine/{quarantined_id}/release"),
            &alice,
            Some(json!({})),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "owner must not release: {body}"
    );

    // The steward sees the pending event — redacted payload, findings.
    let (status, queue) = send(&app, request(Method::GET, "/v1/quarantine", &steward, None)).await;
    assert_eq!(status, StatusCode::OK, "{queue}");
    let pending = queue["pending"].as_array().expect("pending list");
    assert_eq!(pending.len(), 1, "{queue}");
    assert_eq!(pending[0]["event_id"], quarantined_id.as_str(), "{queue}");
    let shown = pending[0]["payload"]["text"]
        .as_str()
        .expect("payload text");
    assert!(
        shown.contains("[REDACTED:aws-access-key-id]") && !shown.contains(SEEDED_AWS_KEY),
        "the reviewer sees redacted content only: {shown}"
    );
    assert_eq!(
        pending[0]["findings"][0]["rule"], "aws-access-key-id",
        "{queue}"
    );

    // Subtree filter: the platform team's queue holds it; a subtree
    // listing is authorized at that node.
    let (status, scoped) = send(
        &app,
        request(
            Method::GET,
            &format!("/v1/quarantine?scope_id={}", platform.id),
            &steward,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{scoped}");
    assert_eq!(scoped["pending"].as_array().expect("list").len(), 1);

    // Release: the signal goes out, the state flips, the chain records it.
    let signals_before = queued(&pool, tenant).await;
    let (status, released) = send(
        &app,
        request(
            Method::POST,
            &format!("/v1/quarantine/{quarantined_id}/release"),
            &steward,
            Some(json!({"reason": "reviewed: key is the AWS docs example"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{released}");
    assert_eq!(released["state"], "released", "{released}");
    assert_eq!(released["reviewer_subject"], "stew", "{released}");
    assert_eq!(
        queued(&pool, tenant).await,
        signals_before + 1,
        "release sends the standard work signal"
    );

    // One-shot: a second verdict conflicts.
    let (status, body) = send(
        &app,
        request(
            Method::POST,
            &format!("/v1/quarantine/{quarantined_id}/reject"),
            &steward,
            Some(json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "review is one-shot: {body}");

    // The released event's review rides the chain.
    let mut tx = rls::begin_tenant_tx(&pool, tenant).await.expect("tx");
    let events = synveda_audit::tail(&mut tx, tenant, 100)
        .await
        .expect("read chain");
    assert!(
        events
            .iter()
            .any(|event| event.action == "memory.quarantine.released"
                && event.payload["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains("AWS docs example"))),
        "the release must chain"
    );
    let verification = synveda_audit::verify(&mut tx, tenant)
        .await
        .expect("verify chain");
    assert!(matches!(verification, ChainVerification::Valid { .. }));
}

/// The rest of the review contract: security-reviewer (its first live
/// action) rejects without a signal; redelivery of a quarantined event
/// reports duplicate and neither re-quarantines nor signals; unknown ids
/// answer the uniform 404.
#[tokio::test]
async fn quarantine_review_contract_holds() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let (_, _, platform) = seed_hierarchy(&pool, tenant).await;
    let idp = MockIdp::spawn().await;
    let app = router(state(&db_url, &idp.issuer, tenant));
    seed_user(&pool, tenant, "alice", platform.id).await;
    seed_user(&pool, tenant, "sec", platform.id).await;
    bind_role(&pool, tenant, "sec", Role::SecurityReviewer).await;
    let alice = idp.user_token("alice");
    let reviewer = idp.user_token("sec");

    // Quarantine one event under the strict default.
    let secret_batch = batch(
        "contract-session",
        vec![event("c-secret", &format!("psst {SEEDED_AWS_KEY}"))],
    );
    let (status, body) = send(
        &app,
        request(
            Method::POST,
            "/v1/observe",
            &alice,
            Some(secret_batch.clone()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["quarantined"], 1, "{body}");
    let event_id = body["events"][0]["event_id"]
        .as_str()
        .expect("event id")
        .to_owned();
    assert_eq!(queued(&pool, tenant).await, 0, "no signal while pending");

    // Redelivery: duplicate under the original id, still one quarantine
    // row, still no signal — the winning delivery's disposition stands.
    let (status, retry) = send(
        &app,
        request(Method::POST, "/v1/observe", &alice, Some(secret_batch)),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{retry}");
    assert_eq!(retry["events"][0]["status"], "duplicate", "{retry}");
    assert_eq!(retry["events"][0]["event_id"], event_id.as_str(), "{retry}");
    let quarantine_rows = sqlx::query_scalar!(
        r#"select count(*) as "count!" from observe_quarantine where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&pool)
    .await
    .expect("count quarantine rows");
    assert_eq!(quarantine_rows, 1, "redelivery must not re-quarantine");
    assert_eq!(queued(&pool, tenant).await, 0, "redelivery must not signal");

    // The security reviewer rejects: state flips, no signal, chained.
    let (status, rejected) = send(
        &app,
        request(
            Method::POST,
            &format!("/v1/quarantine/{event_id}/reject"),
            &reviewer,
            Some(json!({"reason": "credential paste; do not extract"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rejected}");
    assert_eq!(rejected["state"], "rejected", "{rejected}");
    assert_eq!(queued(&pool, tenant).await, 0, "reject never signals");
    let mut tx = rls::begin_tenant_tx(&pool, tenant).await.expect("tx");
    let events = synveda_audit::tail(&mut tx, tenant, 50)
        .await
        .expect("read chain");
    assert!(
        events
            .iter()
            .any(|event| event.action == "memory.quarantine.rejected"),
        "the rejection must chain"
    );
    drop(tx);

    // Unknown id: the uniform 404, for reviewer and owner alike.
    let ghost = synveda_types::ObserveEventId::new();
    for bearer in [&reviewer, &alice] {
        let (status, _) = send(
            &app,
            request(
                Method::POST,
                &format!("/v1/quarantine/{ghost}/release"),
                bearer,
                Some(json!({})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "unknown ids answer 404");
    }

    // The seeded literal is nowhere, pending-through-rejected included.
    assert!(
        !storage_contains(&pool, tenant, SEEDED_AWS_KEY).await,
        "the seeded key must not survive the quarantine lifecycle"
    );
}
