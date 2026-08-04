//! GRPH-2 acceptance (ADR-0044), the half that needs a database: the
//! pipeline links records→entities→episodes over the real product path —
//! `POST /v1/observe`, the extraction worker — and never a seeded row.
//!
//! The fixture-set precision measurement the AC names is
//! `crates/synveda-ingest/tests/entity_resolution.rs`, where the resolver
//! is a pure function. What is asserted *here* is the other half of that
//! claim: that the key it computes really does converge on one vertex.
//! Two sessions, a month apart, naming the same company two different ways
//! meet on one node — "entity resolution against existing nodes" is a
//! property of the schema's unique constraint, and this is where that gets
//! proved rather than assumed.
//!
//! Also here: the orphan rate the AC asks to be tracked, per graph and on
//! the metric a dashboard would read; the 2-hop traversal that makes the
//! whole thing worth building (record → name → the other record, which is
//! the seed path GRPH-3 will take); the idempotency of re-assertion; and
//! the compliance property that a redaction placeholder never becomes a
//! graph identity.
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
use chrono::{DateTime, Utc};
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::{OidcVerifier, parse_issuers, personal_slug};
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder};
use synveda_ingest::extraction::{AnyExtractor, DeterministicExtractor};
use synveda_ingest::worker::{self, WorkerConfig, WorkerDeps};
use synveda_store::graph::{self, ProvenanceEdge};
use synveda_store::{hierarchy, identities, quarantine, rls, tenants};
use synveda_types::{
    Depth, Graph, GraphVertexId, HierarchyNode, Identity, IdentityId, IdentityKind, ObserveEventId,
    RecordId, ScopeId, ScopeKind, TenantId, TenantStatus,
};
use tower::ServiceExt;

const KEY_PEM: &str = include_str!("fixtures/idp_key_a.pem");
const KEY_JWK: &str = include_str!("fixtures/idp_key_a.jwk.json");
const CLIENT_ID: &str = "synveda-test";

/// The documentation-only key the redaction suites use. Never a real
/// credential — the fixture discipline MEM-2 set.
const SEEDED_AWS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";

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
        scope_chains: Arc::new(synveda_store::ScopeChainCache::new()),
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
    }
}

fn worker_deps(state: &AppState) -> WorkerDeps {
    WorkerDeps {
        pool: state.pool.clone(),
        pdp: Arc::clone(&state.pdp),
        chains: Arc::clone(&state.scope_chains),
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

async fn post_observe(app: &Router, bearer: &str, body: Value) -> StatusCode {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/observe")
        .header("authorization", format!("Bearer {bearer}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let _ = body_json(response).await;
    status
}

async fn admitted_tenant() -> Option<(PgPool, TenantId, String)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping GRPH-2 linking test: DATABASE_URL is not set \
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
    let slug = format!("grph2-{}", id.as_uuid().simple());
    tenants::create(&pool, id, &slug, "GRPH-2 test tenant", TenantStatus::Active)
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

fn event(key: &str, kind: &str, occurred_at: &str, text: &str) -> Value {
    json!({
        "idempotency_key": key,
        "kind": kind,
        "payload": { "text": text },
        "occurred_at": occurred_at,
    })
}

// ── Reading the graph as built (superuser connection; the RLS suite owns
//    isolation) ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct StoredVertex {
    id: GraphVertexId,
    graph: String,
    kind: String,
    key: String,
    label: String,
    record_id: Option<RecordId>,
}

async fn vertices(pool: &PgPool, tenant: TenantId) -> Vec<StoredVertex> {
    sqlx::query!(
        "select id, graph, kind, key, label, record_id from graph_vertices
         where tenant_id = $1 order by graph, kind, key",
        tenant.as_uuid(),
    )
    .fetch_all(pool)
    .await
    .expect("read vertices")
    .into_iter()
    .map(|row| StoredVertex {
        id: GraphVertexId::from_uuid(row.id),
        graph: row.graph,
        kind: row.kind,
        key: row.key,
        label: row.label,
        record_id: row.record_id.map(RecordId::from_uuid),
    })
    .collect()
}

#[derive(Debug, Clone)]
struct StoredEdge {
    graph: String,
    kind: String,
    src_id: GraphVertexId,
    dst_id: GraphVertexId,
    method: String,
    confidence_permille: i32,
    source_record_id: Option<RecordId>,
    valid_from: DateTime<Utc>,
    valid_to: Option<DateTime<Utc>>,
}

async fn edges(pool: &PgPool, tenant: TenantId) -> Vec<StoredEdge> {
    sqlx::query!(
        "select graph, kind, src_id, dst_id, method, confidence_permille,
                source_record_id, valid_from, valid_to
         from graph_edges where tenant_id = $1 order by graph, kind",
        tenant.as_uuid(),
    )
    .fetch_all(pool)
    .await
    .expect("read edges")
    .into_iter()
    .map(|row| StoredEdge {
        graph: row.graph,
        kind: row.kind,
        src_id: GraphVertexId::from_uuid(row.src_id),
        dst_id: GraphVertexId::from_uuid(row.dst_id),
        method: row.method,
        confidence_permille: row.confidence_permille,
        source_record_id: row.source_record_id.map(RecordId::from_uuid),
        valid_from: row.valid_from,
        valid_to: row.valid_to,
    })
    .collect()
}

async fn record_ids(pool: &PgPool, tenant: TenantId) -> Vec<(RecordId, String)> {
    sqlx::query!(
        "select id, content from records where tenant_id = $1 order by content",
        tenant.as_uuid(),
    )
    .fetch_all(pool)
    .await
    .expect("read records")
    .into_iter()
    .map(|row| (RecordId::from_uuid(row.id), row.content))
    .collect()
}

async fn audit_payloads(pool: &PgPool, tenant: TenantId, action: &str) -> Vec<Value> {
    sqlx::query_scalar!(
        "select payload from audit_log where tenant_id = $1 and action = $2 order by seq",
        tenant.as_uuid(),
        action,
    )
    .fetch_all(pool)
    .await
    .expect("read audit rows")
}

// ── The AC ──────────────────────────────────────────────────────────────────

/// The linking stage, end to end. Two sessions a month apart name the same
/// company two different ways; the graph holds **one** name vertex, each
/// record hangs off it, each record hangs off its own session, and a 2-hop
/// traversal from one record reaches the other — records → entities →
/// records, which is the path GRPH-3 exists to walk.
#[tokio::test]
async fn two_sessions_naming_one_company_converge_on_one_vertex() {
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
    let deps = worker_deps(&state);

    // Session one, in June: the decision names the company one way.
    let status = post_observe(
        &app,
        &token,
        json!({
            "session_id": "sess-june",
            "events": [event(
                "g-1", "decision", "2026-06-02T09:00:00Z",
                "We decided ACME Corp will host the ledger service.",
            )],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    drain(&deps, &worker_config()).await;

    // Session two, in July: a different session, a different owner-facing
    // wording, the same company. Drained separately so the second commit
    // resolves against a vertex that already exists rather than against one
    // it is creating — which is what "against existing nodes" means.
    let status = post_observe(
        &app,
        &token,
        json!({
            "session_id": "sess-july",
            "events": [event(
                "g-2", "transcript_delta", "2026-07-14T09:00:00Z",
                "Acme Corporation renewed the platform contract.",
            )],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    drain(&deps, &worker_config()).await;

    let records = record_ids(&pool, tenant).await;
    assert_eq!(records.len(), 2, "two events, two records");
    let vertices = vertices(&pool, tenant).await;
    let edges = edges(&pool, tenant).await;

    // One name, however many ways it was written.
    let names: Vec<&StoredVertex> = vertices
        .iter()
        .filter(|vertex| vertex.graph == "entity" && vertex.kind == "name")
        .collect();
    assert_eq!(
        names.len(),
        1,
        "two spellings of one company are one vertex: {names:?}"
    );
    assert_eq!(names[0].key, "acme");
    assert_eq!(
        names[0].label, "Acme Corporation",
        "the newest observation names the thing"
    );
    assert_eq!(
        names[0].record_id, None,
        "a name is identity, not a record's content"
    );

    // Every record is a vertex of both graphs, and its key and label are
    // its id — never its content (ADR-0044 decision 8).
    for (record_id, content) in &records {
        for graph in ["entity", "episode"] {
            let vertex = vertices
                .iter()
                .find(|vertex| {
                    vertex.graph == graph
                        && vertex.kind == "record"
                        && vertex.record_id == Some(*record_id)
                })
                .unwrap_or_else(|| panic!("record {record_id} has a {graph} vertex"));
            assert_eq!(vertex.key, record_id.to_string());
            assert_eq!(vertex.label, record_id.to_string());
            assert!(
                !content.contains(&vertex.label),
                "a vertex label must never carry record content"
            );
        }
    }

    // Two sessions, two episode vertices.
    let mut sessions: Vec<&str> = vertices
        .iter()
        .filter(|vertex| vertex.kind == "session")
        .map(|vertex| vertex.key.as_str())
        .collect();
    sessions.sort_unstable();
    assert_eq!(
        sessions,
        vec!["sess-july", "sess-june"],
        "sorted, not chronological"
    );

    // Four claims: two mentions, two occurrences. Each names the record it
    // came from, each is open-ended, and the confidence tier reports what
    // normalisation did — "ACME Corp" needed a suffix removed, so it is the
    // lossy tier; "Acme Corporation" needed the same, and the occurrence
    // claims needed nothing at all.
    assert_eq!(edges.len(), 4, "{edges:?}");
    let mentions: Vec<&StoredEdge> = edges
        .iter()
        .filter(|edge| edge.kind == "mentions")
        .collect();
    assert_eq!(mentions.len(), 2);
    for edge in &mentions {
        assert_eq!(edge.graph, "entity");
        assert_eq!(edge.method, "deterministic");
        assert_eq!(edge.dst_id, names[0].id);
        assert_eq!(edge.confidence_permille, 900);
        // The claim starts at the record that made it, and says so twice:
        // the source vertex *is* that record, and `source_record_id` names
        // it — which is what keeps every name in the graph traceable to the
        // scope that governs it (ADR-0044 decision 7).
        let source = vertices
            .iter()
            .find(|vertex| vertex.id == edge.src_id)
            .expect("the claim's source is a vertex");
        assert_eq!(source.kind, "record");
        assert_eq!(source.record_id, edge.source_record_id);
        assert!(
            edge.source_record_id.is_some(),
            "an edge names its evidence"
        );
        assert_eq!(
            edge.valid_to, None,
            "a mention does not expire (ADR-0044 decision 13)"
        );
    }
    let occurrences: Vec<&StoredEdge> = edges
        .iter()
        .filter(|edge| edge.kind == "occurred_during")
        .collect();
    assert_eq!(occurrences.len(), 2);
    for edge in &occurrences {
        assert_eq!(edge.graph, "episode");
        assert_eq!(
            edge.confidence_permille, 1000,
            "a session id is observed, not inferred"
        );
    }
    // Valid time is the record's, which is the event's occurred-at.
    let june: DateTime<Utc> = "2026-06-02T09:00:00Z".parse().expect("parses");
    assert!(edges.iter().any(|edge| edge.valid_from == june));

    // The payoff: from one record, two hops through the shared name reach
    // the other record — undirected, so it does not matter which end the
    // seed sits on. This is the seed path GRPH-3 takes.
    let seed = vertices
        .iter()
        .find(|vertex| {
            vertex.graph == "entity"
                && vertex.kind == "record"
                && vertex.record_id == Some(records[0].0)
        })
        .expect("seed vertex");
    let other = vertices
        .iter()
        .find(|vertex| {
            vertex.graph == "entity"
                && vertex.kind == "record"
                && vertex.record_id == Some(records[1].0)
        })
        .expect("the other record's vertex");
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("tenant tx");
    let expansion = graph::expand(
        &mut tx,
        tenant,
        Graph::Entity,
        &[seed.id],
        Depth::Two,
        Utc::now(),
        None,
    )
    .await
    .expect("expand");
    tx.commit().await.expect("commit read");
    let reached: Vec<GraphVertexId> = expansion
        .reached
        .iter()
        .map(|reached| reached.vertex_id)
        .collect();
    assert!(
        reached.contains(&names[0].id),
        "the name is one hop from the record"
    );
    assert!(
        reached.contains(&other.id),
        "the other record is two hops away, through the name"
    );
    assert_eq!(expansion.edges.len(), 2, "both mention claims were walked");

    // The graph's work rides the group's existing extraction event: no new
    // action type, and an auditor holding it can see what was learned.
    let payloads = audit_payloads(&pool, tenant, "memory.extracted").await;
    assert_eq!(payloads.len(), 2, "one aggregated event per commit group");
    for payload in &payloads {
        assert_eq!(payload["graph"]["entity"]["linked"], 1);
        assert_eq!(payload["graph"]["entity"]["orphans"], 0);
        assert_eq!(payload["graph"]["episode"]["linked"], 1);
        assert_eq!(payload["graph"]["names"], 1);
        assert_eq!(payload["graph"]["edges"], 2);
        assert_eq!(payload["graph"]["mentions_resolved"], 1);
        assert_eq!(payload["graph"]["held"], 0);
    }
    assert!(
        audit_payloads(&pool, tenant, "graph.linked")
            .await
            .is_empty(),
        "GRPH-2 adds no action type (ADR-0043's compliance note)"
    );
}

/// The orphan rate the AC asks to be tracked: a record that names nothing
/// is a normal outcome, counted per graph, and visible on the metric a
/// dashboard would read. The episode graph has no orphan here — a session
/// id is always present — which is exactly why the counter is labelled by
/// graph rather than summed.
#[tokio::test]
async fn records_that_name_nothing_are_counted_as_orphans() {
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

    let status = post_observe(
        &app,
        &token,
        json!({
            "session_id": "sess-orphan",
            "events": [
                event("o-1", "transcript_delta", "2026-07-20T09:00:00Z",
                      "the nightly reconciliation finished without errors"),
                event("o-2", "decision", "2026-07-20T09:05:00Z",
                      "We decided Grafana stays on the internal network."),
            ],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    drain(&worker_deps(&state), &worker_config()).await;

    let records = record_ids(&pool, tenant).await;
    assert_eq!(records.len(), 2);
    let edges = edges(&pool, tenant).await;
    let mentions: Vec<&StoredEdge> = edges
        .iter()
        .filter(|edge| edge.kind == "mentions")
        .collect();
    assert_eq!(
        mentions.len(),
        1,
        "one record named something, the other named nothing"
    );
    assert_eq!(
        edges
            .iter()
            .filter(|edge| edge.kind == "occurred_during")
            .count(),
        2,
        "both records belong to the session they came from"
    );

    let payload = audit_payloads(&pool, tenant, "memory.extracted")
        .await
        .pop()
        .expect("the extraction event");
    assert_eq!(payload["graph"]["entity"]["linked"], 1);
    assert_eq!(payload["graph"]["entity"]["orphans"], 1);
    assert_eq!(payload["graph"]["episode"]["orphans"], 0);

    let rendered = state.metrics.render();
    for series in [
        r#"synveda_graph_link_records_total{graph="entity",outcome="orphan"}"#,
        r#"synveda_graph_link_records_total{graph="entity",outcome="linked"}"#,
        r#"synveda_graph_link_records_total{graph="episode",outcome="linked"}"#,
        r#"synveda_graph_link_mentions_total{outcome="resolved"}"#,
    ] {
        assert!(rendered.contains(series), "{series} is not being tracked");
    }
    println!(
        "orphan rate, this run: entity {}/{} records unlinked",
        payload["graph"]["entity"]["orphans"],
        records.len()
    );
}

/// A redaction placeholder never becomes a graph identity (ADR-0044
/// decision 9). The admission scanner replaces the key before anything is
/// stored, and neither the placeholder nor the text it hid reaches a
/// vertex — which matters because `graph_vertices` carries no scope, so a
/// key there is readable tenant-wide.
#[tokio::test]
async fn a_redacted_secret_never_becomes_a_vertex() {
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

    let status = post_observe(
        &app,
        &token,
        json!({
            "session_id": "sess-secret",
            "events": [event(
                "s-1", "transcript_delta", "2026-07-20T09:00:00Z",
                &format!("The old deploy key {SEEDED_AWS_KEY} was rotated by Ada Lovelace."),
            )],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    // The strict pack quarantines a secret rather than queueing it; a
    // reviewer releases it and the redacted text rejoins the pipeline
    // (MEM-2, ADR-0021). Going through that door rather than around it is
    // the point: the linker sees exactly what a released event carries.
    let event_id: ObserveEventId = sqlx::query_scalar!(
        "select id from observe_events where tenant_id = $1",
        tenant.as_uuid()
    )
    .fetch_one(&pool)
    .await
    .map(ObserveEventId::from_uuid)
    .expect("the staged event");
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
    drain(&worker_deps(&state), &worker_config()).await;

    let vertices = vertices(&pool, tenant).await;
    assert!(
        !vertices.is_empty(),
        "the record still links — this is not a test that linking failed"
    );
    for vertex in &vertices {
        for value in [&vertex.key, &vertex.label] {
            assert!(
                !value.contains(SEEDED_AWS_KEY),
                "a secret must never reach an unscoped vertex: {value:?}"
            );
            assert!(
                !value.to_uppercase().contains("REDACTED"),
                "a placeholder is not an entity: {value:?}"
            );
        }
    }
    // The name beside the secret still resolves: the refusal is targeted,
    // not a blanket refusal of any record that was scanned.
    assert!(
        vertices
            .iter()
            .any(|vertex| vertex.kind == "name" && vertex.key == "ada lovelace"),
        "{vertices:?}"
    );
}

/// The `provenance` graph is **projected, not written** (ADR-0044
/// decision 14, discharging ADR-0039's trigger (d) and ADR-0043's
/// decision 11).
///
/// A statement and its replacement go through the pipeline; MEM-5 closes
/// the first record's window and records why in `record_supersessions`.
/// The graph then answers the same fact in the edge model — without a row
/// of `graph_edges` existing for it, because two systems of record for one
/// claim is the failure this projection was chosen to avoid.
#[tokio::test]
async fn supersessions_are_projected_as_provenance_edges_and_never_mirrored() {
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
    let deps = worker_deps(&state);

    for (key, session, occurred, text) in [
        (
            "p-1",
            "sess-before",
            "2026-06-01T09:00:00Z",
            "We decided the reconciliation job runs against the ledger-archive replica.",
        ),
        (
            "p-2",
            "sess-after",
            "2026-07-01T09:00:00Z",
            "We decided the reconciliation job runs against the ledger-live replica.",
        ),
    ] {
        let status = post_observe(
            &app,
            &token,
            json!({
                "session_id": session,
                "events": [event(key, "decision", occurred, text)],
            }),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        drain(&deps, &worker_config()).await;
    }

    let records = record_ids(&pool, tenant).await;
    assert_eq!(records.len(), 2, "{records:?}");
    let ids: Vec<RecordId> = records.iter().map(|(id, _)| *id).collect();

    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("tenant tx");
    let projected = graph::supersession_edges(&mut *tx, tenant, &ids)
        .await
        .expect("project supersessions");
    tx.commit().await.expect("commit read");
    assert_eq!(
        projected.len(),
        1,
        "one statement replaced the other: {projected:?}"
    );
    let edge = &projected[0];
    assert_eq!(ProvenanceEdge::GRAPH, Graph::Provenance);
    assert_eq!(ProvenanceEdge::KIND, "supersedes");
    assert_eq!(edge.method, "deterministic");
    assert!(ids.contains(&edge.superseding_id) && ids.contains(&edge.superseded_id));
    assert_ne!(edge.superseding_id, edge.superseded_id);
    assert!(
        edge.jaccard_permille.is_some(),
        "the evidence is carried through as the judge recorded it, not reshaped"
    );

    // The projection is not a mirror: nothing was written to the edge
    // table, and no vertex was minted to hold a record that already exists.
    assert!(
        edges(&pool, tenant)
            .await
            .iter()
            .all(|edge| edge.kind != "supersedes"),
        "record_supersessions stays the only system of record for this claim"
    );
    assert!(
        vertices(&pool, tenant)
            .await
            .iter()
            .all(|vertex| vertex.graph != "provenance"),
        "the provenance graph is answered, not materialised"
    );
}
