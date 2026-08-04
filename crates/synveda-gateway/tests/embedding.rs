//! MEM-4 acceptance (ADR-0023): transactional embed-or-fail. The chaos
//! test kills TEI mid-batch and proves zero lost and zero embedding-less
//! records — the events whose embeddings returned commit atomically with
//! their vectors, the rest redeliver and commit after recovery, and the
//! documented Mem0 failure mode (a record persisted, its embedding
//! silently dropped) is unrepresentable. The schema backstop test proves
//! even raw SQL cannot commit an embedding-less record.
//!
//! Tests need a live Postgres: they read `DATABASE_URL` and skip when it
//! is unset (CI has no database) — the tests/extraction.rs harness. TEI
//! itself is always an in-process mock (the MockIdp discipline).

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Method, Request, StatusCode};
use axum::response::{Json, Response};
use axum::routing::{get, post};
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
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder, TeiEmbedder};
use synveda_ingest::extraction::{AnyExtractor, DeterministicExtractor};
use synveda_ingest::worker::{self, WorkerConfig, WorkerDeps};
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::{hierarchy, identities, rls, tenants};
use synveda_types::{
    HierarchyNode, Identity, IdentityId, IdentityKind, RecordClass, RecordId, RecordKind, ScopeId,
    ScopeKind, Sensitivity, TenantId, TenantStatus,
};
use tower::ServiceExt;

const KEY_PEM: &str = include_str!("fixtures/idp_key_a.pem");
const KEY_JWK: &str = include_str!("fixtures/idp_key_a.jwk.json");
const CLIENT_ID: &str = "synveda-test";

/// Serialises tests: the Prometheus recorder, tracing's callsite cache,
/// and the shared PGMQ queue (each test purges it) are process-global.
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

// ── The chaos TEI mock ──────────────────────────────────────────────────────

/// A TEI mock with a health budget: each `/embed` request spends one
/// unit; a spent budget answers 500 — TEI is "dead" — until the test
/// revives it. Killing after the first request is the AC's mid-batch
/// kill: some events embedded, the rest mid-air.
#[derive(Clone)]
struct ChaosTei {
    budget: Arc<AtomicI64>,
}

/// The mock's fixed output dimension.
const MOCK_DIM: usize = 4;

async fn chaos_embed(
    State(chaos): State<ChaosTei>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if chaos.budget.fetch_sub(1, Ordering::SeqCst) <= 0 {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "tei was killed mid-batch"})),
        ));
    }
    let inputs = body["inputs"].as_array().expect("inputs array").len();
    let vectors: Vec<Vec<f32>> = (0..inputs).map(|_| vec![0.5; MOCK_DIM]).collect();
    Ok(Json(json!(vectors)))
}

/// Spawns the mock and returns its base URL plus the budget handle.
async fn spawn_chaos_tei(initial_budget: i64) -> (String, Arc<AtomicI64>) {
    let budget = Arc::new(AtomicI64::new(initial_budget));
    let chaos = ChaosTei {
        budget: Arc::clone(&budget),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind chaos tei");
    let addr = listener.local_addr().expect("chaos tei addr");
    let app = Router::new()
        .route("/embed", post(chaos_embed))
        .with_state(chaos);
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("chaos tei serve");
    });
    (format!("http://{addr}"), budget)
}

// ── Gateway + worker harness (the tests/extraction.rs shape) ────────────────

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

fn worker_deps(state: &AppState, embedder: AnyEmbedder) -> WorkerDeps {
    WorkerDeps {
        pool: state.pool.clone(),
        pdp: Arc::clone(&state.pdp),
        chains: Arc::clone(&state.scope_chains),
        extractor: AnyExtractor::Deterministic(DeterministicExtractor::new()),
        embedder,
    }
}

/// Chaos pacing: vt 0 keeps failed signals immediately re-readable (no
/// 30 s waits in a test), max_reads far above the retry count so nothing
/// dead-letters by accident.
fn chaos_config() -> WorkerConfig {
    WorkerConfig {
        batch: 32,
        vt_secs: 0,
        max_reads: 50,
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

async fn post_observe(app: &Router, bearer: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/observe")
        .header("authorization", format!("Bearer {bearer}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    (status, body_json(response).await)
}

async fn admitted_tenant() -> Option<(PgPool, TenantId, String)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping MEM-4 embedding test: DATABASE_URL is not set \
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
    let slug = format!("mem4-{}", id.as_uuid().simple());
    tenants::create(&pool, id, &slug, "MEM-4 test tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((pool, id, url))
}

async fn seed_hierarchy(pool: &PgPool, tenant: TenantId) -> HierarchyNode {
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
    let platform = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(org.id),
        ScopeKind::Team,
        "platform",
        "Platform",
    )
    .await
    .expect("create team");
    hierarchy::create(
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
    platform
}

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

/// This tenant's queued signal count (body-filtered — the queue is shared).
async fn queued(pool: &PgPool, tenant: TenantId) -> i64 {
    sqlx::query_scalar!(
        r#"select count(*) as "count!" from pgmq.q_observe
           where message ->> 'tenant_id' = $1"#,
        tenant.to_string(),
    )
    .fetch_one(pool)
    .await
    .expect("count queue signals")
}

/// This tenant's committed records as (class, embedding model, dim),
/// with a NULL model marking the Mem0 failure mode: a record that exists
/// without its embedding.
async fn records_with_embeddings(
    pool: &PgPool,
    tenant: TenantId,
) -> Vec<(String, Option<String>, Option<i32>)> {
    sqlx::query!(
        r#"select r.class, e.model as "model?", e.dim as "dim?"
           from records r
           left join record_embeddings e on e.record_id = r.id
           where r.tenant_id = $1
           order by r.class"#,
        tenant.as_uuid(),
    )
    .fetch_all(pool)
    .await
    .expect("read records with embeddings")
    .into_iter()
    .map(|row| (row.class, row.model, row.dim))
    .collect()
}

fn event(key: &str, kind: &str, occurred_at: &str, payload: Value) -> Value {
    json!({
        "idempotency_key": key,
        "kind": kind,
        "payload": payload,
        "occurred_at": occurred_at,
    })
}

// ── The AC: kill TEI mid-batch; zero lost, zero embedding-less ──────────────

/// Three observed events; TEI dies after embedding the first. The
/// embedded event commits atomically with its vector, the other two
/// redeliver — nothing is lost, nothing commits embedding-less. TEI
/// comes back; the stragglers commit. Every record carries a
/// model-tagged vector, both commit groups audit their embedder, and
/// the chain verifies.
#[tokio::test]
async fn chaos_killing_tei_mid_batch_loses_no_records_and_embeds_all() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let platform = seed_hierarchy(&pool, tenant).await;
    let idp = MockIdp::spawn().await;
    let state = state(&db_url, &idp.issuer, tenant);
    let app = router(state.clone());
    seed_user(&pool, tenant, "alice", platform.id).await;
    let token = idp.user_token("alice");

    // TEI serves exactly one embed call, then dies mid-batch.
    let (tei_url, budget) = spawn_chaos_tei(1).await;
    let deps = worker_deps(
        &state,
        AnyEmbedder::Tei(TeiEmbedder::new("mock-bge".to_owned(), tei_url)),
    );

    let occurred = "2026-07-22T09:00:00Z";
    let (status, _) = post_observe(
        &app,
        &token,
        json!({
            "session_id": "sess-chaos",
            "events": [
                event("c-1", "decision", occurred,
                      json!({"text": "Chose embed-or-fail over async embedding."})),
                event("c-2", "tool_result", occurred,
                      json!({"output": "cargo test: 14 passed, 0 failed."})),
                event("c-3", "transcript_delta", occurred,
                      json!({"text": "We always use transactional vectors."})),
            ],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    // One pass while TEI dies under it.
    let read = worker::run_once(&deps, &chaos_config())
        .await
        .expect("chaos pass");
    assert_eq!(read, 3, "the pass saw all three signals");

    // Partial batch: the embedded event committed with its vector; the
    // two that lost TEI redeliver; NOTHING is embedding-less.
    let after_kill = records_with_embeddings(&pool, tenant).await;
    assert_eq!(after_kill.len(), 1, "one event committed before the kill");
    assert!(
        after_kill.iter().all(|(_, model, _)| model.is_some()),
        "no record may exist without its embedding: {after_kill:?}"
    );
    assert_eq!(
        queued(&pool, tenant).await,
        2,
        "the failed events redeliver"
    );

    // TEI comes back; the stragglers drain.
    budget.store(1_000, Ordering::SeqCst);
    drain(&deps, &chaos_config()).await;

    assert_eq!(queued(&pool, tenant).await, 0, "the queue must drain");
    let recovered = records_with_embeddings(&pool, tenant).await;
    let classes: Vec<&str> = recovered
        .iter()
        .map(|(class, _, _)| class.as_str())
        .collect();
    assert_eq!(
        classes,
        vec!["decision", "episode", "preference"],
        "zero lost records"
    );
    for (class, model, dim) in &recovered {
        assert_eq!(
            model.as_deref(),
            Some("mock-bge"),
            "{class} must carry the configured model"
        );
        assert_eq!(*dim, Some(MOCK_DIM as i32), "{class} must carry the dim");
    }

    // Both commit groups chained with their embedder identity.
    let extracted = sqlx::query!(
        r#"select outcome, payload from audit_log
           where tenant_id = $1 and action = 'memory.extracted' order by seq"#,
        tenant.as_uuid(),
    )
    .fetch_all(&pool)
    .await
    .expect("read audit rows");
    assert_eq!(extracted.len(), 2, "one aggregated event per commit group");
    for row in &extracted {
        assert_eq!(row.outcome, "success");
        assert_eq!(row.payload["embedder"], "tei");
        assert_eq!(row.payload["embedding_model"], "mock-bge");
    }
    let committed_events: usize = extracted
        .iter()
        .map(|row| row.payload["events"].as_array().expect("events").len())
        .sum();
    assert_eq!(committed_events, 3, "every event is accounted for");

    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("tenant tx");
    let verification = synveda_audit::verify(&mut tx, tenant)
        .await
        .expect("verify chain");
    assert!(
        matches!(verification, ChainVerification::Valid { .. }),
        "the chain must verify: {verification:?}"
    );
    drop(tx);

    assert!(
        state
            .metrics
            .render()
            .contains("synveda_embedder_requests_total"),
        "embedder metrics must reach the recorder"
    );
}

/// The zero-config path: with the deterministic embedder every committed
/// record carries a `hash@1` vector — embed-or-fail holds without TEI.
#[tokio::test]
async fn deterministic_embedder_commits_hash_vectors() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let platform = seed_hierarchy(&pool, tenant).await;
    let idp = MockIdp::spawn().await;
    let state = state(&db_url, &idp.issuer, tenant);
    let app = router(state.clone());
    seed_user(&pool, tenant, "alice", platform.id).await;
    let token = idp.user_token("alice");

    let (status, _) = post_observe(
        &app,
        &token,
        json!({
            "session_id": "sess-det",
            "events": [event("d-1", "decision", "2026-07-22T10:00:00Z",
                json!({"text": "The deterministic embedder is the zero-config default."}))],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let deps = worker_deps(
        &state,
        AnyEmbedder::Deterministic(DeterministicEmbedder::new()),
    );
    drain(&deps, &chaos_config()).await;

    let committed = records_with_embeddings(&pool, tenant).await;
    assert_eq!(committed.len(), 1);
    assert_eq!(
        committed[0].1.as_deref(),
        Some(DeterministicEmbedder::MODEL)
    );
    assert_eq!(committed[0].2, Some(16));
}

/// The schema backstop (migration 0015, ADR-0023 decision 4): raw SQL
/// that inserts a record without its embedding cannot COMMIT — the
/// deferred constraint trigger refuses at the boundary where the
/// archive-lock would otherwise consume the signal. The store API's
/// single-statement write commits fine on the same connection.
#[tokio::test]
async fn raw_sql_cannot_commit_an_embedding_less_record() {
    let _serial = serial().await;
    let Some((pool, tenant, _)) = admitted_tenant().await else {
        return;
    };

    let bare = RecordId::new();
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query!(
        r#"
        insert into records
            (id, tenant_id, scope_id, owner_id, kind, class, content,
             sensitivity, provenance, valid_from, valid_to, tx_from)
        values ($1, $2, $3, $4, 'derived', 'fact', 'embedding-less',
                'internal', '{}'::jsonb, now(), null, now())
        "#,
        bare.as_uuid(),
        tenant.as_uuid(),
        ScopeId::new().as_uuid(),
        IdentityId::new().as_uuid(),
    )
    .execute(&mut *tx)
    .await
    .expect("the insert itself is deferred-legal");
    let err = tx.commit().await.expect_err("the commit must be refused");
    assert!(
        err.to_string().contains("no embedding"),
        "the backstop names the violation: {err}"
    );
    assert!(
        records::current(&pool, bare)
            .await
            .expect("re-read")
            .is_none(),
        "nothing persisted"
    );

    // Control: the sanctioned single-statement write commits.
    let embedded = RecordId::new();
    let mut tx = pool.begin().await.expect("begin");
    records::insert(
        &mut *tx,
        embedded,
        tenant,
        &RecordState {
            scope_id: ScopeId::new(),
            owner_id: IdentityId::new(),
            kind: RecordKind::Derived,
            class: RecordClass::Fact,
            content: "embedded".to_owned(),
            sensitivity: Sensitivity::Internal,
            provenance: json!({"source": "mem-4 backstop test"}),
            valid_from: chrono::Utc::now(),
            valid_to: None,
        },
        &RecordEmbedding {
            model: "test@1".to_owned(),
            vector: vec![1.0, 0.0],
        },
    )
    .await
    .expect("insert with embedding");
    tx.commit().await.expect("commit embedded record");
    let meta = records::embedding_meta(&pool, embedded)
        .await
        .expect("read meta")
        .expect("the embedding row exists");
    assert_eq!(meta.model, "test@1");
    assert_eq!(meta.dim, 2);
}
