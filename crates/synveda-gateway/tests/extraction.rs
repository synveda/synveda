//! MEM-3 acceptance (ADR-0022): observe → worker → bitemporal records,
//! end to end. Every record carries the provenance quadruple (session,
//! method, model version, confidence — the AC), commits are exactly-once
//! under the archive-lock, a released quarantined event is
//! indistinguishable from an admitted one, the pipeline's own write
//! re-decides at current facts (a since-quarantined owner is denied), a
//! poisoned signal dead-letters with an audited failure, and an
//! extractor that echoes a live-format secret writes the placeholder,
//! never the text.
//!
//! Tests need a live Postgres: they read `DATABASE_URL` and skip when it
//! is unset (CI has no database) — the tests/observe.rs harness, pruned
//! to the user-token shape.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::response::{Json, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Utc};
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
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder};
use synveda_ingest::extraction::{AnyExtractor, ClaudeExtractor, DeterministicExtractor};
use synveda_ingest::worker::{self, WorkerConfig, WorkerDeps};
use synveda_store::{hierarchy, identities, quarantine, rls, tenants};
use synveda_types::{
    HierarchyNode, Identity, IdentityId, IdentityKind, ObserveEventId, RecordId, ScopeId,
    ScopeKind, TenantId, TenantStatus,
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

/// The worker wired exactly as the gateway wires it: the same pool, PDP,
/// and scope-chain cache instance (ADR-0022 decision 1). This suite is
/// about extraction; the embedder is the network-free deterministic one
/// (the MEM-4 suite owns embedding behaviour).
fn worker_deps(state: &AppState, extractor: AnyExtractor) -> WorkerDeps {
    WorkerDeps {
        pool: state.pool.clone(),
        pdp: Arc::clone(&state.pdp),
        chains: Arc::clone(&state.scope_chains),
        extractor,
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
                "skipping MEM-3 extraction test: DATABASE_URL is not set \
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
    // The shared queue may hold other suites' leftovers (nothing else
    // consumes); purge so drain loops and assertions see only this test.
    sqlx::query_scalar!(r#"select pgmq.purge_queue('observe') as "purged!""#)
        .fetch_one(&pool)
        .await
        .expect("purge observe queue");
    let id = TenantId::new();
    let slug = format!("mem3-{}", id.as_uuid().simple());
    tenants::create(&pool, id, &slug, "MEM-3 test tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((pool, id, url))
}

/// Seeds acme-org → platform (team) plus the reserved quarantine team;
/// returns (platform, quarantine).
async fn seed_hierarchy(pool: &PgPool, tenant: TenantId) -> (HierarchyNode, HierarchyNode) {
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
    let quarantine = hierarchy::create(
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
    (platform, quarantine)
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

struct StoredRecord {
    id: RecordId,
    class: String,
    content: String,
    sensitivity: String,
    kind: String,
    provenance: Value,
    valid_from: DateTime<Utc>,
}

/// This tenant's current records (superuser test connection — RLS-exempt
/// on purpose; the RLS suite owns isolation).
async fn stored_records(pool: &PgPool, tenant: TenantId) -> Vec<StoredRecord> {
    sqlx::query!(
        r#"select id, class, content, sensitivity, kind, provenance, valid_from
           from records where tenant_id = $1 order by class"#,
        tenant.as_uuid(),
    )
    .fetch_all(pool)
    .await
    .expect("read records")
    .into_iter()
    .map(|row| StoredRecord {
        id: RecordId::from_uuid(row.id),
        class: row.class,
        content: row.content,
        sensitivity: row.sensitivity,
        kind: row.kind,
        provenance: row.provenance,
        valid_from: row.valid_from,
    })
    .collect()
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

async fn staged_events(pool: &PgPool, tenant: TenantId) -> Vec<(ObserveEventId, DateTime<Utc>)> {
    sqlx::query!(
        "select id, occurred_at from observe_events where tenant_id = $1",
        tenant.as_uuid(),
    )
    .fetch_all(pool)
    .await
    .expect("read staged events")
    .into_iter()
    .map(|row| (ObserveEventId::from_uuid(row.id), row.occurred_at))
    .collect()
}

/// Audit rows for one action, as (actor_kind, outcome, payload).
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

fn event(key: &str, kind: &str, occurred_at: &str, payload: Value) -> Value {
    json!({
        "idempotency_key": key,
        "kind": kind,
        "payload": payload,
        "occurred_at": occurred_at,
    })
}

// ── The AC: observe → worker → records with full provenance ─────────────────

/// Three observed events become three derived records at the owner's home
/// with the right classes; every record carries the provenance quadruple
/// (session, method, model version, confidence); valid time is the
/// event's occurred-at; the queue drains; one aggregated
/// `memory.extracted` event chains with the `system` actor — and the
/// chain verifies.
#[tokio::test]
async fn observed_events_become_derived_records_with_provenance() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let (platform, _) = seed_hierarchy(&pool, tenant).await;
    let idp = MockIdp::spawn().await;
    let state = state(&db_url, &idp.issuer, tenant);
    let app = router(state.clone());
    let alice = seed_user(&pool, tenant, "alice", platform.id).await;
    let token = idp.user_token("alice");

    let occurred = "2026-07-21T09:00:00Z";
    let (status, _) = post_observe(
        &app,
        &token,
        json!({
            "session_id": "sess-a",
            "events": [
                event("a-1", "decision", occurred,
                      json!({"text": "Chose PGMQ over Kafka for the observe buffer."})),
                event("a-2", "tool_result", occurred,
                      json!({"output": "cargo test: 12 passed, 0 failed."})),
                event("a-3", "transcript_delta", occurred,
                      json!({"text": "We always use small single-purpose commits."})),
            ],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let deps = worker_deps(
        &state,
        AnyExtractor::Deterministic(DeterministicExtractor::new()),
    );
    drain(&deps, &worker_config()).await;

    assert_eq!(queued(&pool, tenant).await, 0, "the queue must drain");
    let staged: Vec<_> = staged_events(&pool, tenant).await;
    let records = stored_records(&pool, tenant).await;
    assert_eq!(records.len(), 3);
    let classes: Vec<&str> = records.iter().map(|r| r.class.as_str()).collect();
    assert_eq!(classes, vec!["decision", "episode", "preference"]);
    let expected_valid_from: DateTime<Utc> = occurred.parse().expect("occurred_at parses");
    for record in &records {
        assert_eq!(record.kind, "derived");
        assert_eq!(record.sensitivity, "internal");
        assert_eq!(record.valid_from, expected_valid_from);
        // The AC quadruple, on every record.
        assert_eq!(record.provenance["session_id"], "sess-a");
        assert_eq!(record.provenance["method"], "deterministic");
        assert_eq!(record.provenance["model_version"], "builtin@1");
        let confidence = record.provenance["confidence"]
            .as_f64()
            .expect("confidence is a number");
        assert!((0.0..=1.0).contains(&confidence));
        let event_id: ObserveEventId = record.provenance["event_id"]
            .as_str()
            .expect("provenance names its event")
            .parse()
            .expect("event id parses");
        assert!(staged.iter().any(|(id, _)| *id == event_id));
        assert!(record.provenance["extracted_at"].as_str().is_some());
    }
    // Records land at the owner's home scope.
    let scopes = sqlx::query_scalar!(
        "select scope_id from records where tenant_id = $1",
        tenant.as_uuid()
    )
    .fetch_all(&pool)
    .await
    .expect("record scopes");
    assert!(
        scopes
            .iter()
            .all(|scope| *scope == alice.scope_id.as_uuid())
    );

    let extracted = audit_rows(&pool, tenant, "memory.extracted").await;
    assert_eq!(extracted.len(), 1, "one aggregated event per commit group");
    let (actor_kind, outcome, payload) = &extracted[0];
    assert_eq!(actor_kind, "system");
    assert_eq!(outcome, "success");
    assert_eq!(payload["events"].as_array().expect("events array").len(), 3);
    assert_eq!(payload["method"], "deterministic");
    assert_chain_verifies(&pool, tenant).await;

    // FLOW-2 (ADR-0031 decision 13), discharging ADR-0022's recorded
    // forward obligation: the records and their derived-channel commit
    // land in one transaction. The channel rides the group's existing
    // event rather than chaining a second one (decision 14).
    let channels = payload["channels"].as_array().expect("channels array");
    assert_eq!(channels.len(), 1, "one commit per owner's home scope");
    assert_eq!(channels[0]["ref"], "memory/derived");
    assert_eq!(channels[0]["scope_id"], json!(alice.scope_id));
    assert_eq!(channels[0]["records"], json!(3), "this batch's records");
    assert!(
        channels[0]["parent"].is_null(),
        "the channel's first commit"
    );
    let commit = channels[0]["commit"].as_str().expect("commit hash");
    assert_eq!(commit.len(), 64);

    // And the ref really points there, with an object per record whose
    // address the composition engine recomputes from the record itself.
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("tenant tx");
    let derived = synveda_vedaflow::read_memory_members(
        &mut tx,
        tenant,
        &[alice.scope_id],
        synveda_types::Channel::Derived,
    )
    .await
    .expect("read the derived channel");
    let head = derived.first().expect("the derived channel exists");
    assert_eq!(head.commit.to_hex(), commit);
    assert_eq!(head.members.len(), 3);
    for record in &records {
        assert!(
            head.members.contains_key(&record.id),
            "record {} is on the derived channel",
            record.id
        );
    }
    drop(tx);

    assert!(
        state
            .metrics
            .render()
            .contains("synveda_extraction_events_total"),
        "worker metrics must reach the recorder"
    );
}

/// A released quarantined event flows through extraction exactly like an
/// admitted one (ADR-0021 decision 7 → ADR-0022): the record's content is
/// the redacted staging content, and the finding summary rides provenance.
#[tokio::test]
async fn released_quarantined_events_extract_identically() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let (platform, _) = seed_hierarchy(&pool, tenant).await;
    let idp = MockIdp::spawn().await;
    let state = state(&db_url, &idp.issuer, tenant);
    let app = router(state.clone());
    seed_user(&pool, tenant, "alice", platform.id).await;
    let token = idp.user_token("alice");

    // The documentation-only AWS example key: quarantined under the
    // zero-config strict pack (secrets → quarantine), redacted at
    // admission.
    let (status, _) = post_observe(
        &app,
        &token,
        json!({
            "session_id": "sess-q",
            "events": [event("q-1", "transcript_delta", "2026-07-21T10:00:00Z",
                json!({"text": "The old deploy key AKIAIOSFODNN7EXAMPLE was rotated today."}))],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(queued(&pool, tenant).await, 0, "quarantine sends no signal");
    let (event_id, _) = staged_events(&pool, tenant).await[0];

    // Release re-joins the pipeline with the standard signal.
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("tenant tx");
    quarantine::review(
        &mut tx,
        tenant,
        event_id,
        quarantine::ReviewDecision::Release,
        "sec-reviewer",
        None,
    )
    .await
    .expect("release")
    .expect("review row exists");
    tx.commit().await.expect("commit release");
    assert_eq!(queued(&pool, tenant).await, 1);

    let deps = worker_deps(
        &state,
        AnyExtractor::Deterministic(DeterministicExtractor::new()),
    );
    drain(&deps, &worker_config()).await;

    let records = stored_records(&pool, tenant).await;
    assert_eq!(records.len(), 1);
    assert!(
        records[0].content.contains("[REDACTED:aws-access-key-id]"),
        "content is the redacted staging text: {:?}",
        records[0].content
    );
    assert!(!records[0].content.contains("AKIAIOSFODNN7EXAMPLE"));
    assert!(
        !records[0].provenance["redactions"].is_null(),
        "the finding summary rides provenance"
    );
}

/// The pipeline's write re-decides at current facts (seed §2.2, ADR-0022
/// decision 4): an owner quarantined after admission is denied — the
/// signal archives, nothing persists, the staging row remains, and a
/// standalone deny decision chains under the `system` actor.
#[tokio::test]
async fn since_quarantined_owner_is_denied_at_commit() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let (platform, quarantine_node) = seed_hierarchy(&pool, tenant).await;
    let idp = MockIdp::spawn().await;
    let state = state(&db_url, &idp.issuer, tenant);
    let app = router(state.clone());
    let mallory = seed_user(&pool, tenant, "mallory", platform.id).await;
    let token = idp.user_token("mallory");

    let (status, _) = post_observe(
        &app,
        &token,
        json!({
            "session_id": "sess-d",
            "events": [event("d-1", "decision", "2026-07-21T11:00:00Z",
                json!({"text": "A decision made moments before the quarantine."}))],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    // Quarantined between admission and extraction: placement under the
    // reserved node (ADR-0013 decision 4), with the cache invalidation
    // any out-of-band hierarchy writer owes (ADR-0016).
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("tenant tx");
    hierarchy::move_node(&mut tx, mallory.scope_id, quarantine_node.id)
        .await
        .expect("move mallory into quarantine");
    tx.commit().await.expect("commit quarantine move");
    state.scope_chains.invalidate(tenant);

    let deps = worker_deps(
        &state,
        AnyExtractor::Deterministic(DeterministicExtractor::new()),
    );
    drain(&deps, &worker_config()).await;

    assert!(stored_records(&pool, tenant).await.is_empty());
    assert_eq!(queued(&pool, tenant).await, 0, "the denied signal archives");
    assert_eq!(
        staged_events(&pool, tenant).await.len(),
        1,
        "staging remains"
    );
    let denials = audit_rows(&pool, tenant, "authz.decision").await;
    let denial = denials
        .iter()
        .find(|(_, outcome, payload)| outcome == "deny" && payload["op"] == "extraction")
        .expect("the extraction denial chains");
    assert_eq!(denial.0, "system");
    assert_eq!(denial.2["action"], "memory.write");
    assert_chain_verifies(&pool, tenant).await;
}

/// Retries exhausted (ADR-0022 decision 6): a signal past the read
/// threshold archives without an extraction attempt and chains
/// `memory.extracted` with outcome `failure`; the staging row stays
/// re-drivable provenance.
#[tokio::test]
async fn exhausted_signals_dead_letter_with_an_audited_failure() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let (platform, _) = seed_hierarchy(&pool, tenant).await;
    let idp = MockIdp::spawn().await;
    let state = state(&db_url, &idp.issuer, tenant);
    let app = router(state.clone());
    seed_user(&pool, tenant, "alice", platform.id).await;
    let token = idp.user_token("alice");

    let (status, _) = post_observe(
        &app,
        &token,
        json!({
            "session_id": "sess-p",
            "events": [event("p-1", "transcript_delta", "2026-07-21T12:00:00Z",
                json!({"text": "A poisoned message's content."}))],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let deps = worker_deps(
        &state,
        AnyExtractor::Deterministic(DeterministicExtractor::new()),
    );
    // max_reads = 0: the very first read is already past the threshold.
    drain(
        &deps,
        &WorkerConfig {
            max_reads: 0,
            ..worker_config()
        },
    )
    .await;

    assert!(stored_records(&pool, tenant).await.is_empty());
    assert_eq!(queued(&pool, tenant).await, 0);
    assert_eq!(
        staged_events(&pool, tenant).await.len(),
        1,
        "staging remains"
    );
    let extracted = audit_rows(&pool, tenant, "memory.extracted").await;
    assert_eq!(extracted.len(), 1);
    let (actor_kind, outcome, payload) = &extracted[0];
    assert_eq!(actor_kind, "system");
    assert_eq!(outcome, "failure");
    assert_eq!(payload["reason"], "retries exhausted");
    assert_eq!(payload["events"][0]["read_count"], 1);
    assert_chain_verifies(&pool, tenant).await;
}

/// Redelivery cannot duplicate memories (ADR-0022 decision 2): a signal
/// read once and left to expire is processed exactly once by the worker —
/// the archive-lock makes the commit idempotent against at-least-once
/// delivery.
#[tokio::test]
async fn redelivered_signals_extract_exactly_once() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let (platform, _) = seed_hierarchy(&pool, tenant).await;
    let idp = MockIdp::spawn().await;
    let state = state(&db_url, &idp.issuer, tenant);
    let app = router(state.clone());
    seed_user(&pool, tenant, "alice", platform.id).await;
    let token = idp.user_token("alice");

    let (status, _) = post_observe(
        &app,
        &token,
        json!({
            "session_id": "sess-r",
            "events": [event("r-1", "decision", "2026-07-21T13:00:00Z",
                json!({"text": "One decision, delivered more than once."}))],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    // A competing consumer read the signal and died: vt 0 leaves it
    // immediately visible again with its read count advanced.
    let mut conn = pool.acquire().await.expect("acquire");
    let seen = synveda_store::observe::read_signals(&mut conn, 0, 32)
        .await
        .expect("competing read");
    assert!(!seen.is_empty());
    drop(conn);

    let deps = worker_deps(
        &state,
        AnyExtractor::Deterministic(DeterministicExtractor::new()),
    );
    drain(&deps, &worker_config()).await;
    drain(&deps, &worker_config()).await;

    assert_eq!(
        stored_records(&pool, tenant).await.len(),
        1,
        "exactly one record"
    );
    assert_eq!(queued(&pool, tenant).await, 0);
}

/// The LLM-echo hole is closed (ADR-0022 decision 7): an extractor whose
/// output carries a live-format secret — here a mock Claude API echoing
/// an AWS key — persists the placeholder, never the text, and the audit
/// payload counts the catch.
#[tokio::test]
async fn extractor_output_secrets_never_persist() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let (platform, _) = seed_hierarchy(&pool, tenant).await;
    let idp = MockIdp::spawn().await;
    let state = state(&db_url, &idp.issuer, tenant);
    let app = router(state.clone());
    seed_user(&pool, tenant, "alice", platform.id).await;
    let token = idp.user_token("alice");

    // A mock Claude endpoint that "extracts" a memorised credential.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock claude");
    let base_url = format!("http://{}", listener.local_addr().expect("addr"));
    tokio::spawn(async move {
        let app = Router::new().route(
            "/v1/messages",
            post(|| async {
                Json(json!({
                    "model": "claude-opus-4-8",
                    "stop_reason": "tool_use",
                    "content": [{
                        "type": "tool_use", "id": "tu-1", "name": "emit_extraction",
                        "input": { "candidates": [{
                            "class": "fact",
                            "content": "The deploy key is AKIAIOSFODNN7EXAMPLE and lives in CI.",
                            "confidence": 0.9
                        }]}
                    }]
                }))
            }),
        );
        axum::serve(listener, app).await.expect("mock claude serve");
    });

    let (status, _) = post_observe(
        &app,
        &token,
        json!({
            "session_id": "sess-e",
            "events": [event("e-1", "transcript_delta", "2026-07-21T14:00:00Z",
                json!({"text": "Rotate the CI deploy credentials."}))],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let deps = worker_deps(
        &state,
        AnyExtractor::Claude(ClaudeExtractor::new(
            "test-key-never-real".to_owned(),
            "claude-opus-4-8".to_owned(),
            base_url,
        )),
    );
    drain(&deps, &worker_config()).await;

    let records = stored_records(&pool, tenant).await;
    assert_eq!(records.len(), 1);
    assert!(
        records[0].content.contains("[REDACTED:aws-access-key-id]"),
        "the echoed secret must persist as its placeholder: {:?}",
        records[0].content
    );
    assert!(!records[0].content.contains("AKIAIOSFODNN7EXAMPLE"));
    assert_eq!(records[0].provenance["method"], "claude-api");
    let extracted = audit_rows(&pool, tenant, "memory.extracted").await;
    assert_eq!(extracted.len(), 1);
    assert!(
        extracted[0].2["rescan_findings"].as_u64().unwrap_or(0) >= 1,
        "the catch is audited: {:?}",
        extracted[0].2
    );
}
