//! MEM-5 acceptance (ADR-0039): the store stops being ADD-only.
//!
//! The AC has two halves and both are here, over the real product path —
//! `POST /v1/observe`, the extraction worker, `POST /v1/inject` — never a
//! seeded row:
//!
//! * **superseded facts are excluded from current inject**: a session states
//!   a fact, a later session states its replacement, and the very next
//!   inject carries the replacement and not the fact it replaced;
//! * **superseded facts stay retrievable via as-of**: the same record is
//!   still there at the instant it held, through the bitemporal read the
//!   composition engine itself uses.
//!
//! Around them: a restatement merges rather than duplicating, a
//! contradiction against *published* material is refused and counted
//! because reviewed content leaves the trust boundary through review, a
//! late-arriving older fact lands already closed rather than being dropped,
//! and a pack can turn the whole thing off.
//!
//! Tests need a live Postgres: they read `DATABASE_URL` and skip when it is
//! unset (CI has no database) — the tests/extraction.rs harness.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::response::{Json, Response};
use axum::routing::get;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_audit::ChainVerification;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::{OidcVerifier, parse_issuers};
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder};
use synveda_ingest::extraction::{AnyExtractor, DeterministicExtractor};
use synveda_ingest::worker::{self, WorkerConfig, WorkerDeps};
use synveda_store::records;
use synveda_store::{access, identities, policy_packs, rls, scopes, tenants};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{
    Channel, DedupConfig, DedupMode, GrantId, Identity, IdentityId, IdentityKind, PackConfig,
    RecordId, ScopeId, Sensitivity, TenantId, TenantStatus,
};
use tower::ServiceExt;

const KEY_PEM: &str = include_str!("fixtures/idp_key_a.pem");
const KEY_JWK: &str = include_str!("fixtures/idp_key_a.jwk.json");
const CLIENT_ID: &str = "synveda-test";

/// The pair the whole feature is about: one statement, then the same
/// statement with its value changed. Same class (`decision` routes by kind,
/// so the classifier cannot drift), same subject, one differing word.
const BEFORE: &str =
    "We decided the payment reconciliation job runs against the ledger-archive replica.";
const AFTER: &str =
    "We decided the payment reconciliation job runs against the ledger-live replica.";

/// Serialises tests: the Prometheus recorder, tracing's callsite cache, and
/// the shared PGMQ queue (each test purges it) are process-global.
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
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs()
}

// ── The mock IdP (user tokens only) ─────────────────────────────────────────

#[derive(Clone)]
struct MockIdp {
    issuer: String,
}

impl MockIdp {
    async fn spawn() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock idp");
        let addr = listener.local_addr().expect("mock idp addr");
        let idp = Self {
            issuer: format!("http://{addr}/mock-idp"),
        };
        let issuer = idp.issuer.clone();
        let app = Router::new()
            .route(
                "/mock-idp/.well-known/openid-configuration",
                get(move || {
                    let issuer = issuer.clone();
                    async move {
                        Json(json!({
                            "issuer": issuer,
                            "authorization_endpoint": format!("{issuer}/authorize"),
                            "token_endpoint": format!("{issuer}/token"),
                            "jwks_uri": format!("{issuer}/jwks"),
                        }))
                    }
                }),
            )
            .route(
                "/mock-idp/jwks",
                get(|| async {
                    Json(json!({
                        "keys": [serde_json::from_str::<Value>(KEY_JWK).expect("jwk fixture")]
                    }))
                }),
            );
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("mock idp serve");
        });
        idp
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

// ── Gateway + worker harness ────────────────────────────────────────────────

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
        service_token_max_ttl: Duration::from_secs(3600),
        search_index: Arc::new(
            synveda_retrieval::SearchIndex::open(
                std::env::temp_dir()
                    .join("synveda-gateway-tests")
                    .join(TenantId::new().to_string()),
            )
            .expect("open search index"),
        ),
        embedder: Arc::new(AnyEmbedder::Deterministic(DeterministicEmbedder::new())),
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

/// The worker wired exactly as the gateway wires it (ADR-0022 decision 1),
/// on the network-free deterministic extractor and embedder. The hash
/// embedder's geometry carries no meaning (ADR-0023 decision 6) — so
/// everything this suite proves is proved by the *lexical* leg alone, which
/// is the honest floor and the reason ADR-0039 decision 2 keeps two.
fn worker_deps(state: &AppState) -> WorkerDeps {
    WorkerDeps {
        pool: state.pool.clone(),
        pdp: Arc::clone(&state.pdp),
        extractor: AnyExtractor::Deterministic(DeterministicExtractor::new()),
        embedder: AnyEmbedder::Deterministic(DeterministicEmbedder::new()),
    }
}

fn worker_config() -> WorkerConfig {
    WorkerConfig {
        batch: 32,
        vt_secs: 30,
        ..WorkerConfig::default()
    }
}

/// Drains the queue: `run_once` until a pass reads nothing.
async fn drain(deps: &WorkerDeps, config: &WorkerConfig) {
    for _ in 0..200 {
        if worker::run_once(deps, config).await.expect("worker pass") == 0 {
            return;
        }
    }
    panic!("the observe queue did not drain");
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

async fn post(app: &Router, uri: &str, bearer: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("authorization", format!("Bearer {bearer}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build request");
    let response = app.clone().oneshot(request).await.expect("send request");
    let status = response.status();
    (status, body_json(response).await)
}

/// One observed statement, through the product's own write path.
async fn observe(app: &Router, bearer: &str, session: &str, at: DateTime<Utc>, text: &str) {
    let (status, body) = post(
        app,
        "/v1/observe",
        bearer,
        json!({
            "session_id": session,
            "events": [{
                "idempotency_key": format!("{session}:{}", at.timestamp_micros()),
                "kind": "decision",
                "payload": {"text": text},
                "occurred_at": at.to_rfc3339(),
            }],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "observe: {body}");
    assert_eq!(body["accepted"], json!(1), "observe: {body}");
}

/// The composed block a cold session start receives — the taskless,
/// recency-ordered branch, which is what "current inject" means for a
/// scenario with no query.
async fn inject_text(app: &Router, bearer: &str, session: &str) -> String {
    let (status, body) = post(app, "/v1/inject", bearer, json!({"session_id": session})).await;
    assert_eq!(status, StatusCode::OK, "inject: {body}");
    body["text"].as_str().expect("block text").to_owned()
}

async fn admitted_tenant() -> Option<(PgPool, TenantId, String)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping MEM-5 dedup test: DATABASE_URL is not set \
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
    sqlx::query_scalar!(r#"select pgmq.purge_queue('observe') as "purged!""#)
        .fetch_one(&pool)
        .await
        .expect("purge observe queue");
    let id = TenantId::new();
    let slug = format!("mem5-{}", id.as_uuid().simple());
    tenants::create(&pool, id, &slug, "MEM-5 test tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((pool, id, url))
}

/// acme-org → platform (team) plus the reserved quarantine team.
async fn seed_hierarchy(pool: &PgPool, tenant: TenantId) -> Scope {
    let mut tx = pool.begin().await.expect("begin");
    let org = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("mint root");
    let platform = scopes::create(
        &mut tx,
        &scopes::NewScope {
            id: ScopeId::new(),
            tenant_id: tenant,
            kind: ScopeKind::OrgUnit,
            parent_scope_id: Some(org.id),
            slug: "platform".to_owned(),
            display_name: "Platform".to_owned(),
            attributes: serde_json::json!({}),
            principal_id: None,
            created_by: None,
        },
    )
    .await
    .expect("create scope");
    tx.commit().await.expect("commit hierarchy");
    platform
}

async fn seed_user(pool: &PgPool, tenant: TenantId, subject: &str) -> Identity {
    let mut tx = pool.begin().await.expect("begin");
    let own = scopes::ensure_principal_scope(&mut tx, tenant, subject, subject)
        .await
        .expect("mint principal scope");
    let identity = identities::create(
        &mut tx,
        IdentityId::new(),
        tenant,
        Some(subject),
        IdentityKind::User,
        None,
        None,
        own.id,
    )
    .await
    .expect("create identity");
    tx.commit().await.expect("commit user");
    identity
}

async fn bind(pool: &PgPool, tenant: TenantId, subject: &str, scope: ScopeId, role: RoleKey) {
    let mut tx = rls::begin_tenant_tx(pool, tenant).await.expect("begin");
    access::create_grant(
        &mut *tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id: tenant,
            scope_id: scope,
            subject: GrantSubject::Principal {
                principal_id: subject.to_owned(),
            },
            role_key: role,
            source: GrantSource::Automation,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .expect("create grant");
    tx.commit().await.expect("commit grant");
}

/// One stored record, as this suite needs to see it.
#[derive(Debug)]
struct Stored {
    id: RecordId,
    content: String,
    valid_from: DateTime<Utc>,
    valid_to: Option<DateTime<Utc>>,
    merged: Option<i64>,
}

/// This tenant's current records, oldest valid-time first (superuser test
/// connection — RLS-exempt on purpose; the RLS suite owns isolation).
async fn stored_records(pool: &PgPool, tenant: TenantId) -> Vec<Stored> {
    sqlx::query!(
        r#"select id, content, valid_from, valid_to,
                  (provenance -> 'merged' ->> 'count')::bigint as merged
           from records where tenant_id = $1 order by valid_from, id"#,
        tenant.as_uuid(),
    )
    .fetch_all(pool)
    .await
    .expect("read records")
    .into_iter()
    .map(|row| Stored {
        id: RecordId::from_uuid(row.id),
        content: row.content,
        valid_from: row.valid_from,
        valid_to: row.valid_to,
        merged: row.merged,
    })
    .collect()
}

/// The supersession edges this tenant holds.
async fn edges(pool: &PgPool, tenant: TenantId) -> Vec<(RecordId, RecordId, String, String)> {
    sqlx::query!(
        r#"select superseded_id, superseding_id, method, reason
           from record_supersessions where tenant_id = $1 order by decided_at, superseded_id"#,
        tenant.as_uuid(),
    )
    .fetch_all(pool)
    .await
    .expect("read edges")
    .into_iter()
    .map(|row| {
        (
            RecordId::from_uuid(row.superseded_id),
            RecordId::from_uuid(row.superseding_id),
            row.method,
            row.reason,
        )
    })
    .collect()
}

async fn audit_rows(pool: &PgPool, tenant: TenantId, action: &str) -> Vec<(String, String, Value)> {
    sqlx::query!(
        r#"select actor_kind, outcome, payload from audit_log
           where tenant_id = $1 and action = $2 order by seq"#,
        tenant.as_uuid(),
        action,
    )
    .fetch_all(pool)
    .await
    .expect("read audit rows")
    .into_iter()
    .map(|row| (row.actor_kind, row.outcome, row.payload))
    .collect()
}

async fn assert_chain_verifies(pool: &PgPool, tenant: TenantId) {
    let mut tx = rls::begin_tenant_tx(pool, tenant).await.expect("tenant tx");
    let verification = synveda_audit::verify(&mut tx, tenant)
        .await
        .expect("verify chain");
    assert!(
        matches!(verification, ChainVerification::Valid { .. }),
        "the chain must verify: {verification:?}"
    );
}

/// Everything a scope holds that is valid at `at` — the composition
/// engine's own candidate read (`compose_candidates`), which is what makes
/// "excluded from current inject" and "retrievable via as-of" the same
/// question asked at two instants.
async fn valid_at(
    pool: &PgPool,
    tenant: TenantId,
    scope: ScopeId,
    at: DateTime<Utc>,
) -> Vec<String> {
    let mut tx = rls::begin_tenant_tx(pool, tenant).await.expect("tenant tx");
    let pairs = synveda_types::ScopeTier::expand(scope, &Sensitivity::ALL);
    // No horizons: this helper asks what the composition engine would
    // see, and MEM-6's product default expires nothing (ADR-0040
    // decision 13).
    // `None` names nothing: the sweep every inject issues, rather than
    // CTX-4's named-id read (ADR-0041 decision 5).
    let found =
        synveda_store::search::compose_candidates(&mut tx, tenant, &pairs, &[], at, 64, None)
            .await
            .expect("compose candidates");
    found
        .into_iter()
        .map(|version| version.state.content)
        .collect()
}

/// The permits this suite's own material needs — write to your own home,
/// read your own chain — and nothing else, so the only thing a pack built
/// on it changes is the configuration under test.
const MEMBER_PACK: &str = r#"
    permit (principal, action == Synveda::Action::"MemoryRead", resource)
    when { principal in resource };
    permit (principal, action == Synveda::Action::"MemoryWrite", resource)
    when { principal has own_scope && resource == principal.own_scope };
"#;

/// A tenant pack carrying a dedup configuration, made the default.
async fn install_dedup_pack(pool: &PgPool, tenant: TenantId, dedup: DedupConfig) {
    let mut tx = rls::begin_tenant_tx(pool, tenant).await.expect("tenant tx");
    policy_packs::apply(
        &mut *tx,
        tenant,
        "mem5-pack",
        MEMBER_PACK,
        &PackConfig {
            dedup: Some(dedup),
            ..PackConfig::default()
        },
    )
    .await
    .expect("apply pack");
    synveda_store::policy_assignments::set_default(&mut *tx, tenant, "mem5-pack")
        .await
        .expect("make it the tenant default");
    tx.commit().await.expect("commit pack");
}

/// The gateway compiles stored packs through its refresher; a test drives
/// the same install directly so the very next write sees it.
fn install_into_pdp(state: &AppState, tenant: TenantId, dedup: DedupConfig) {
    state
        .pdp
        .install_source(
            tenant,
            "mem5-pack",
            1,
            MEMBER_PACK,
            PackConfig {
                dedup: Some(dedup),
                ..PackConfig::default()
            },
        )
        .expect("install stored pack into the PDP");
}

// ── The AC ──────────────────────────────────────────────────────────────────

/// **AC: a superseded fact is excluded from current inject and retrievable
/// as of when it held.**
///
/// Two sessions of one identity, the second stating what replaced the
/// first. Everything below is asserted from the reader's side or from the
/// bitemporal store — nothing seeds a row, and nothing asks the judge
/// directly.
#[tokio::test]
async fn an_updated_fact_supersedes_the_one_it_replaces_and_stays_readable_as_of() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let _platform = seed_hierarchy(&pool, tenant).await;
    let alice = seed_user(&pool, tenant, "alice").await;
    let idp = MockIdp::spawn().await;
    let state = state(&db_url, &idp.issuer, tenant);
    let app = router(state.clone());
    let deps = worker_deps(&state);
    let config = worker_config();
    let token = idp.user_token("alice");

    // Monday: the fact. Tuesday: the fact that replaced it. Explicit
    // instants because valid time is what supersession is *about*.
    let monday = Utc::now() - ChronoDuration::days(2);
    let tuesday = Utc::now() - ChronoDuration::days(1);

    observe(&app, &token, "sess-before", monday, BEFORE).await;
    drain(&deps, &config).await;

    // Before the update, the fact is what the session start receives.
    let block = inject_text(&app, &token, "cold-1").await;
    assert!(
        block.contains("ledger-archive"),
        "the fact is known: {block}"
    );

    observe(&app, &token, "sess-after", tuesday, AFTER).await;
    drain(&deps, &config).await;

    // ── The first half of the AC, from the reader's side ────────────────
    let block = inject_text(&app, &token, "cold-2").await;
    assert!(
        block.contains("ledger-live"),
        "the replacement composes: {block}"
    );
    assert!(
        !block.contains("ledger-archive"),
        "and the fact it replaced does not — this is the whole feature: {block}"
    );

    // ── The store agrees, and says why ──────────────────────────────────
    let stored = stored_records(&pool, tenant).await;
    assert_eq!(
        stored.len(),
        2,
        "nothing was deleted: superseding is closing a window, not removing a row: {stored:?}"
    );
    let (old, new) = (&stored[0], &stored[1]);
    assert!(old.content.contains("ledger-archive"), "{stored:?}");
    assert!(new.content.contains("ledger-live"), "{stored:?}");
    assert_eq!(
        old.valid_to,
        Some(new.valid_from),
        "the old fact's window closes exactly where the new one's begins"
    );
    assert_eq!(new.valid_to, None, "and the new one is open-ended");

    let edges = edges(&pool, tenant).await;
    assert_eq!(edges.len(), 1, "one explicit edge: {edges:?}");
    assert_eq!(edges[0].0, old.id, "superseded");
    assert_eq!(edges[0].1, new.id, "superseding");
    assert_eq!(
        edges[0].2, "deterministic",
        "the judge is named on the edge"
    );
    assert_eq!(edges[0].3, "contradiction");

    // ── The second half of the AC: as-of ────────────────────────────────
    // The same read the composition engine makes, at the instant the old
    // fact held: it is there. At now: it is not. One query, two instants —
    // which is what "excluded from current inject but retrievable via
    // as-of" means when it is a property of the schema rather than of
    // application discipline.
    let then = valid_at(
        &pool,
        tenant,
        alice.scope_id,
        monday + ChronoDuration::hours(1),
    )
    .await;
    assert!(
        then.iter()
            .any(|content| content.contains("ledger-archive")),
        "as of when it held, the superseded fact is still the answer: {then:?}"
    );
    assert!(
        !then.iter().any(|content| content.contains("ledger-live")),
        "and the fact that replaced it had not happened yet: {then:?}"
    );
    let now = valid_at(&pool, tenant, alice.scope_id, Utc::now()).await;
    assert_eq!(now.len(), 1, "one current assertion: {now:?}");
    assert!(now[0].contains("ledger-live"));

    // Every version the database ever held is still addressable, including
    // the open-ended one the record carried before the close.
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("tenant tx");
    let versions = records::versions(&mut *tx, old.id).await.expect("versions");
    tx.commit().await.expect("commit");
    assert_eq!(
        versions.len(),
        2,
        "the pre-close version is archived, not lost"
    );
    assert_eq!(versions[0].state.valid_to, None);
    assert_eq!(versions[1].state.valid_to, Some(new.valid_from));

    // ── The trail ───────────────────────────────────────────────────────
    let events = audit_rows(&pool, tenant, "memory.superseded").await;
    assert_eq!(events.len(), 1, "one event for the group: {events:?}");
    let (actor_kind, outcome, payload) = &events[0];
    assert_eq!(actor_kind, "system");
    assert_eq!(outcome, "success");
    let entry = &payload["superseded"][0];
    assert_eq!(entry["record"], json!(old.id));
    assert_eq!(entry["by"], json!(new.id));
    assert_eq!(entry["method"], json!("deterministic"));
    assert_eq!(entry["on_arrival"], json!(false));
    assert!(
        entry["jaccard_permille"].is_i64(),
        "similarities ride as integers — canonicalisation rejects floats: {payload}"
    );
    assert!(
        !payload.to_string().contains("ledger"),
        "no audit payload carries record content: {payload}"
    );
    assert_chain_verifies(&pool, tenant).await;

    // The derived channel names the closed record at its *new* address:
    // closing a window changes the content address (ADR-0031 decision 6),
    // and re-committing is the obligation that ADR left to this feature.
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("tenant tx");
    let channels =
        synveda_vedaflow::read_memory_members(&mut tx, tenant, &[alice.scope_id], Channel::Derived)
            .await
            .expect("read derived channel");
    tx.commit().await.expect("commit");
    let members = &channels.first().expect("a derived channel exists").members;
    assert!(
        members.contains_key(&old.id) && members.contains_key(&new.id),
        "the commit that superseded names both the new record and the closed one: {members:?}"
    );

    let rendered = state.metrics.render();
    assert!(
        rendered.contains(r#"synveda_dedup_decisions_total{outcome="supersede"}"#),
        "the decision is counted"
    );
}

/// A restatement is absorbed, not duplicated: one record, its provenance
/// carrying the merge, and the merge reported on the extraction event
/// rather than as a second fact of its own (ADR-0039 decisions 10 and 13).
#[tokio::test]
async fn a_restated_fact_merges_into_the_record_it_restates() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let _platform = seed_hierarchy(&pool, tenant).await;
    seed_user(&pool, tenant, "alice").await;
    let idp = MockIdp::spawn().await;
    let state = state(&db_url, &idp.issuer, tenant);
    let app = router(state.clone());
    let deps = worker_deps(&state);
    let config = worker_config();
    let token = idp.user_token("alice");

    let monday = Utc::now() - ChronoDuration::days(2);
    observe(&app, &token, "sess-1", monday, BEFORE).await;
    drain(&deps, &config).await;
    // The same thing said again, in a later session — the ordinary case,
    // and the one an ADD-only store turns into two records.
    observe(
        &app,
        &token,
        "sess-2",
        monday + ChronoDuration::hours(6),
        BEFORE,
    )
    .await;
    drain(&deps, &config).await;

    let stored = stored_records(&pool, tenant).await;
    assert_eq!(stored.len(), 1, "one fact, said twice: {stored:?}");
    assert_eq!(
        stored[0].merged,
        Some(1),
        "and the survivor records that it was observed again: {stored:?}"
    );
    assert_eq!(stored[0].valid_to, None, "a merge closes no windows");
    assert!(
        edges(&pool, tenant).await.is_empty(),
        "a merge is not a supersession"
    );
    assert!(
        audit_rows(&pool, tenant, "memory.superseded")
            .await
            .is_empty(),
        "and it chains no supersession event"
    );

    let extracted = audit_rows(&pool, tenant, "memory.extracted").await;
    let merged = extracted
        .iter()
        .flat_map(|(_, _, payload)| payload["events"].as_array().cloned().unwrap_or_default())
        .filter_map(|event| event["merged"].as_array().cloned())
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(
        merged.len(),
        1,
        "the merge rides the extraction event: {extracted:?}"
    );
    assert_eq!(merged[0]["reason"], json!("identical"));
    assert_eq!(merged[0]["into"], json!(stored[0].id));
    assert_chain_verifies(&pool, tenant).await;
}

/// **The governance boundary.** A contradiction against material a scope
/// has *published* is found, counted, audited — and not acted on. Reviewed
/// content leaves the trust boundary through a proposal or a rollback, never
/// as a side effect of somebody's session (ADR-0039 decision 9).
#[tokio::test]
async fn a_contradiction_against_published_material_is_refused_and_recorded() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let platform = seed_hierarchy(&pool, tenant).await;
    let alice = seed_user(&pool, tenant, "alice").await;
    // A curator **at her own personal scope**, which is where the material
    // she is about to publish lives. A grant at `platform` would not reach
    // it: a principal scope inherits nothing, and the one door into it is a
    // grant written directly there (CPR-6, ADR-0073 decision 4). She can
    // therefore read her own material, which is what publishing it requires
    // (ADR-0031 decision 12).
    let _ = platform;
    bind(&pool, tenant, "alice", alice.scope_id, RoleKey::Curator).await;
    let idp = MockIdp::spawn().await;
    let state = state(&db_url, &idp.issuer, tenant);
    let app = router(state.clone());
    let deps = worker_deps(&state);
    let config = worker_config();
    let token = idp.user_token("alice");

    let monday = Utc::now() - ChronoDuration::days(2);
    observe(&app, &token, "sess-before", monday, BEFORE).await;
    drain(&deps, &config).await;
    let published_id = stored_records(&pool, tenant).await[0].id;

    let (status, body) = post(
        &app,
        &format!("/v1/channels/{}/publish", alice.scope_id),
        &token,
        json!({"record_ids": [published_id], "message": "reviewed"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "publish: {body}");

    // The very same update that supersedes an unreviewed record.
    observe(
        &app,
        &token,
        "sess-after",
        monday + ChronoDuration::days(1),
        AFTER,
    )
    .await;
    drain(&deps, &config).await;

    let stored = stored_records(&pool, tenant).await;
    assert_eq!(stored.len(), 2, "{stored:?}");
    assert_eq!(
        stored[0].valid_to, None,
        "the published fact's window is untouched: only review closes reviewed material"
    );
    assert!(
        edges(&pool, tenant).await.is_empty(),
        "and no edge claims otherwise"
    );

    // Both compose — which is the honest state: the estate holds a reviewed
    // fact its own members have contradicted, and somebody has to decide.
    let block = inject_text(&app, &token, "cold").await;
    assert!(
        block.contains("ledger-archive") && block.contains("ledger-live"),
        "{block}"
    );

    // The refusal is data, not silence: this is how a tenant learns a
    // proposal is owed.
    let events = audit_rows(&pool, tenant, "memory.superseded").await;
    assert_eq!(events.len(), 1, "the refusal is chained: {events:?}");
    let payload = &events[0].2;
    assert_eq!(payload["superseded"], json!([]), "nothing was superseded");
    assert_eq!(
        payload["refused_published"][0]["record"],
        json!(published_id)
    );
    assert_eq!(
        payload["refused_published"][0]["reason"],
        json!("contradiction")
    );
    assert_chain_verifies(&pool, tenant).await;
}

/// **Never ADD-only cuts both ways** (ADR-0039 decision 8): an observation
/// that reaches the pipeline after the fact that replaced it is *recorded*,
/// with its window already shut — never dropped, and never resurrecting a
/// fact that has moved on.
#[tokio::test]
async fn a_late_arriving_older_fact_lands_already_closed() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let _platform = seed_hierarchy(&pool, tenant).await;
    let alice = seed_user(&pool, tenant, "alice").await;
    let idp = MockIdp::spawn().await;
    let state = state(&db_url, &idp.issuer, tenant);
    let app = router(state.clone());
    let deps = worker_deps(&state);
    let config = worker_config();
    let token = idp.user_token("alice");

    let monday = Utc::now() - ChronoDuration::days(2);
    let tuesday = Utc::now() - ChronoDuration::days(1);

    // The newer statement is observed first — an offline session syncing
    // late, a replayed spool, a clock that was behind.
    observe(&app, &token, "sess-current", tuesday, AFTER).await;
    drain(&deps, &config).await;
    observe(&app, &token, "sess-late", monday, BEFORE).await;
    drain(&deps, &config).await;

    let stored = stored_records(&pool, tenant).await;
    assert_eq!(stored.len(), 2, "the late fact is recorded: {stored:?}");
    let late = &stored[0];
    assert!(late.content.contains("ledger-archive"));
    assert_eq!(
        late.valid_to,
        Some(stored[1].valid_from),
        "and it lands already closed, where the newer assertion begins"
    );

    let block = inject_text(&app, &token, "cold").await;
    assert!(block.contains("ledger-live"), "{block}");
    assert!(
        !block.contains("ledger-archive"),
        "a late arrival never resurrects a fact that has moved on: {block}"
    );

    // It is still history, addressable at the instant it held.
    let then = valid_at(
        &pool,
        tenant,
        alice.scope_id,
        monday + ChronoDuration::hours(1),
    )
    .await;
    assert!(
        then.iter()
            .any(|content| content.contains("ledger-archive")),
        "{then:?}"
    );

    let events = audit_rows(&pool, tenant, "memory.superseded").await;
    assert_eq!(events[0].2["superseded"][0]["on_arrival"], json!(true));
    assert_chain_verifies(&pool, tenant).await;
}

/// The feature is configuration, not a constant: a pack that turns dedup
/// off gets the pre-MEM-5 behaviour exactly — both facts current, both
/// composing — which is also what makes the AC a measurement rather than an
/// assertion about code that could not do otherwise.
#[tokio::test]
async fn a_pack_can_turn_the_feature_off() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let _platform = seed_hierarchy(&pool, tenant).await;
    seed_user(&pool, tenant, "alice").await;
    let idp = MockIdp::spawn().await;
    let state = state(&db_url, &idp.issuer, tenant);
    let off = DedupConfig {
        mode: DedupMode::Off,
        ..DedupConfig::DEFAULT
    };
    install_dedup_pack(&pool, tenant, off).await;
    install_into_pdp(&state, tenant, off);
    let app = router(state.clone());
    let deps = worker_deps(&state);
    let config = worker_config();
    let token = idp.user_token("alice");

    let monday = Utc::now() - ChronoDuration::days(2);
    observe(&app, &token, "sess-before", monday, BEFORE).await;
    drain(&deps, &config).await;
    observe(
        &app,
        &token,
        "sess-after",
        monday + ChronoDuration::days(1),
        AFTER,
    )
    .await;
    drain(&deps, &config).await;

    let stored = stored_records(&pool, tenant).await;
    assert_eq!(stored.len(), 2, "{stored:?}");
    assert!(
        stored.iter().all(|record| record.valid_to.is_none()),
        "with dedup off nothing closes: {stored:?}"
    );
    let block = inject_text(&app, &token, "cold").await;
    assert!(
        block.contains("ledger-archive") && block.contains("ledger-live"),
        "the ADD-only store, on request: {block}"
    );
    assert!(edges(&pool, tenant).await.is_empty());
}
