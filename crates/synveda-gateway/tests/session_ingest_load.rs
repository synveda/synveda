//! Acceptance criteria (ADR-0020): duplicate delivery does not
//! duplicate memories — discharged structurally at the buffer — and the
//! observe ack sustains 1k events/s on dev hardware. Plus the surrounding
//! contract: the role-free write floor (zero-config — a JIT-placed user
//! with no bindings observes), fail-closed denial for unprovisioned and
//! quarantined subjects, a service identity observing at its own leaf
//! through the real client-credentials grant, whole-batch validation, and
//! one chained `memory.observed` audit event per batch.
//!
//! Tests need a live Postgres: they read `DATABASE_URL` and skip with a
//! message when it is unset (an intentional local unit-only run), same convention as
//! tests/service_identities.rs (whose harness this copies).

#[path = "../../synveda-store/tests/support/tenant_fixture.rs"]
mod tenant_fixture;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Body;
use axum::extract::{Form, State};
use axum::http::{Method, Request, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, behavior_test_router as router};
use synveda_gateway::telemetry;
use synveda_identity::{OidcVerifier, parse_issuers};
use synveda_store::{identities, scopes};
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{Identity, IdentityId, IdentityKind, ScopeId, TenantId, TenantStatus};
use tower::ServiceExt;

const KEY_PEM: &str = include_str!("fixtures/idp_key_a.pem");
const KEY_JWK: &str = include_str!("fixtures/idp_key_a.jwk.json");
const CLIENT_ID: &str = "synveda-test";
const API_AUDIENCE: &str = "synveda-test-api";
const SERVICE_AUDIENCE: &str = "synveda-agents";
const AGENT_CLIENT: &str = "obs-agent";
const AGENT_SECRET: &str = "obs-agent-secret";

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

// ── The mock IdP (user + client-credentials shaped) ──────────────────────────

#[derive(Clone)]
struct MockIdp {
    issuer: String,
    clients: Arc<HashMap<String, String>>,
}

impl MockIdp {
    async fn spawn() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock idp");
        let addr = listener.local_addr().expect("mock idp addr");
        let idp = Self {
            issuer: format!("http://{addr}/mock-idp"),
            clients: Arc::new(HashMap::from([(
                AGENT_CLIENT.to_owned(),
                AGENT_SECRET.to_owned(),
            )])),
        };
        let app = Router::new()
            .route("/mock-idp/.well-known/openid-configuration", get(discovery))
            .route("/mock-idp/jwks", get(jwks_endpoint))
            .route("/mock-idp/token", post(token_endpoint))
            .with_state(idp.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("mock idp serve");
        });
        idp
    }

    fn sign(&self, claims: &Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("key-a".to_owned());
        let key = EncodingKey::from_rsa_pem(KEY_PEM.as_bytes()).expect("test key");
        jsonwebtoken::encode(&header, claims, &key).expect("sign token")
    }

    fn user_token(&self, subject: &str) -> String {
        self.sign(&json!({
            "iss": self.issuer,
            "sub": subject,
            "aud": API_AUDIENCE,
            "iat": now_secs(),
            "exp": now_secs() + 600,
        }))
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

async fn token_endpoint(
    State(idp): State<MockIdp>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let authenticated = form.get("grant_type").map(String::as_str) == Some("client_credentials")
        && form
            .get("client_id")
            .zip(form.get("client_secret"))
            .is_some_and(|(id, secret)| idp.clients.get(id) == Some(secret));
    if !authenticated {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "invalid_client" })),
        )
            .into_response();
    }
    let token = idp.sign(&json!({
        "iss": idp.issuer,
        "sub": Value::Null,
        "azp": form["client_id"].clone(),
        "aud": SERVICE_AUDIENCE,
        "iat": now_secs(),
        "exp": now_secs() + 600,
    }));
    Json(json!({
        "access_token": token,
        "token_type": "Bearer",
        "expires_in": 600,
    }))
    .into_response()
}

// ── Gateway harness ──────────────────────────────────────────────────────────

fn state(url: &str, issuer: &str, tenant: TenantId) -> AppState {
    let config = format!(
        r#"[{{"issuer":"{issuer}","client_id":"{CLIENT_ID}",
             "audience":"{API_AUDIENCE}",
             "tenant":{{"static":{{"tenant_id":"{tenant}"}}}},
             "service_audiences":["{SERVICE_AUDIENCE}"]}}]"#
    );
    // No `with_refresh_min_interval(ZERO)` here: that setting exists for
    // key-rotation tests and refetches JWKS per request. This suite times
    // the ack path, so the verifier caches keys like production does.
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
        // A fixed test KEK (ADR-0064), so a suite that touches a
        // sealed column seals rather than skipping. `Kms::Disabled` is the
        // production default when no key is configured.
        keys: std::sync::Arc::new(synveda_store::keys::KeyRing::new(
            synveda_crypto::Kms::Local(
                synveda_crypto::LocalKms::from_hex(&"11".repeat(32), "local:test")
                    .expect("test kek"),
            ),
        )),
        embedder: Arc::new(synveda_ingest::embedding::AnyEmbedder::Deterministic(
            synveda_ingest::embedding::DeterministicEmbedder::new(),
        )),
        context_embed_timeout: std::time::Duration::from_millis(100),
        git_sources: None,
        git_exports: None,
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
    let url = tenant_fixture::database_url("observe test")?;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::epoch::verify(&pool)
        .await
        .expect("apply migrations");
    let id = TenantId::new();
    let slug = format!("mem1-{}", id.as_uuid().simple());
    tenant_fixture::create(&pool, id, &slug, "test tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((pool, id, url))
}

/// Seeds root → eng (org unit) → platform (org unit). There is no reserved
/// quarantine scope any more: quarantine is a departure-derived status
/// (ADR-0074 decision 3).
async fn seed_hierarchy(pool: &PgPool, tenant: TenantId) -> (Scope, Scope) {
    let mut tx = tenant_fixture::begin(pool, tenant).await;
    let org = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("mint root");
    let eng = scopes::create(
        &mut tx,
        &scopes::NewScope {
            id: ScopeId::new(),
            tenant_id: tenant,
            kind: ScopeKind::OrgUnit,
            parent_scope_id: Some(org.id),
            slug: "eng".to_owned(),
            display_name: "Engineering".to_owned(),
            attributes: serde_json::json!({}),
            principal_id: None,
            created_by: None,
        },
    )
    .await
    .expect("create scope");
    let platform = scopes::create(
        &mut tx,
        &scopes::NewScope {
            id: ScopeId::new(),
            tenant_id: tenant,
            kind: ScopeKind::OrgUnit,
            parent_scope_id: Some(eng.id),
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
    (org, platform)
}

/// Provisions a user identity at the store level (the JIT shape).
#[path = "session_seed.rs"]
mod session_seed;

/// The append route for one run.
fn events_uri(run: synveda_types::SessionId) -> String {
    format!("/v1/sessions/{run}/events")
}

async fn seed_user(pool: &PgPool, tenant: TenantId, subject: &str) -> Identity {
    let mut tx = tenant_fixture::begin(pool, tenant).await;
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

fn event(key: &str) -> Value {
    json!({
        "client_event_id": key,
        "event_type": "message.user",
        "payload": {"text": format!("delta for {key}")},
        "occurred_at": chrono::Utc::now().to_rfc3339(),
    })
}

fn batch(_session: &str, keys: &[&str]) -> Value {
    json!({
        "events": keys.iter().map(|key| event(key)).collect::<Vec<_>>(),
    })
}

/// Staged events for `tenant`, observed through the ordinary request-plane
/// role and a tenant-scoped transaction.
async fn staged(pool: &PgPool, tenant: TenantId) -> i64 {
    let mut tx = tenant_fixture::begin(pool, tenant).await;
    let count = sqlx::query_scalar!(
        r#"select count(*) as "count!" from session_events where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut *tx)
    .await
    .expect("count observe_events");
    tx.commit().await.expect("commit staged-event count");
    count
}

/// One table's cumulative scan counters, for the load report.
#[derive(Clone, Copy, Default)]
struct TableReads {
    seq_scans: i64,
    seq_rows: i64,
    index_scans: i64,
    index_rows: i64,
    live: i64,
    dead: i64,
}

/// Every user table's scan counters, or `None` when the statistics views
/// are unreadable: the report is a diagnostic and never fails the AC.
/// Backends publish their counters once idle and at most once a second,
/// so callers wait two seconds after the work they want counted.
async fn table_reads(pool: &PgPool) -> Option<HashMap<String, TableReads>> {
    let rows = sqlx::query_as::<_, (String, i64, i64, i64, i64, i64, i64)>(
        "select t.schemaname || '.' || t.relname, coalesce(t.seq_scan, 0), \
         coalesce(t.seq_tup_read, 0), coalesce(t.idx_scan, 0), \
         coalesce((select sum(i.idx_tup_read) from pg_stat_user_indexes i \
         where i.relid = t.relid), 0)::bigint, t.n_live_tup, t.n_dead_tup \
         from pg_stat_user_tables t",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| eprintln!("session append load: table statistics unavailable: {error}"))
    .ok()?;
    Some(
        rows.into_iter()
            .map(
                |(table, seq_scans, seq_rows, index_scans, index_rows, live, dead)| {
                    let reads = TableReads {
                        seq_scans,
                        seq_rows,
                        index_scans,
                        index_rows,
                        live,
                        dead,
                    };
                    (table, reads)
                },
            )
            .collect(),
    )
}

/// The tables the load read most, per batch: a request path that scans a
/// table grown by earlier work shows here as rows read per ack.
fn heaviest_reads(
    before: &HashMap<String, TableReads>,
    after: &HashMap<String, TableReads>,
    batches: usize,
) -> String {
    let batches = batches as f64;
    let mut deltas: Vec<(String, TableReads)> = after
        .iter()
        .map(|(table, now)| {
            let was = before.get(table).copied().unwrap_or_default();
            let delta = TableReads {
                seq_scans: now.seq_scans - was.seq_scans,
                seq_rows: now.seq_rows - was.seq_rows,
                index_scans: now.index_scans - was.index_scans,
                index_rows: now.index_rows - was.index_rows,
                live: now.live,
                dead: now.dead,
            };
            (table.clone(), delta)
        })
        .filter(|(_, delta)| delta.seq_rows + delta.index_rows > 0)
        .collect();
    deltas.sort_by_key(|(table, delta)| {
        (
            std::cmp::Reverse(delta.seq_rows + delta.index_rows),
            table.clone(),
        )
    });
    if deltas.is_empty() {
        return "none recorded".to_owned();
    }
    deltas
        .iter()
        .take(8)
        .map(|(table, delta)| {
            format!(
                "{table} seq {:.1}x/{:.0} rows, index {:.1}x/{:.0} rows (live {}, dead {})",
                delta.seq_scans as f64 / batches,
                delta.seq_rows as f64 / batches,
                delta.index_scans as f64 / batches,
                delta.index_rows as f64 / batches,
                delta.live,
                delta.dead,
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// This database's cumulative session-time and block counters (PostgreSQL
/// 14+), for the load report.
#[derive(Clone, Copy, Default)]
struct DatabaseTime {
    active_ms: f64,
    idle_in_transaction_ms: f64,
    commits: i64,
    blocks_read: i64,
    blocks_hit: i64,
    temp_bytes: i64,
}

/// `None` when the statistics view is unreadable: diagnostic only. The same
/// two-second publication wait as [`table_reads`] applies.
async fn database_time(pool: &PgPool) -> Option<DatabaseTime> {
    let (active_ms, idle_in_transaction_ms, commits, blocks_read, blocks_hit, temp_bytes) =
        sqlx::query_as::<_, (f64, f64, i64, i64, i64, i64)>(
            "select coalesce(active_time, 0), coalesce(idle_in_transaction_time, 0), \
             xact_commit, blks_read, blks_hit, temp_bytes \
             from pg_stat_database where datname = current_database()",
        )
        .fetch_one(pool)
        .await
        .map_err(|error| eprintln!("session append load: database time unavailable: {error}"))
        .ok()?;
    Some(DatabaseTime {
        active_ms,
        idle_in_transaction_ms,
        commits,
        blocks_read,
        blocks_hit,
        temp_bytes,
    })
}

/// Where each batch's database time went: executing statements (`active`)
/// or holding a transaction open while the gateway works between them.
fn database_time_per_batch(before: DatabaseTime, after: DatabaseTime, batches: usize) -> String {
    let batches = batches as f64;
    format!(
        "active {:.1}ms, idle in transaction {:.1}ms, {:.1} commits, \
         {:.0} blocks read, {:.0} hit, {:.0} temp bytes",
        (after.active_ms - before.active_ms) / batches,
        (after.idle_in_transaction_ms - before.idle_in_transaction_ms) / batches,
        (after.commits - before.commits) as f64 / batches,
        (after.blocks_read - before.blocks_read) as f64 / batches,
        (after.blocks_hit - before.blocks_hit) as f64 / batches,
        (after.temp_bytes - before.temp_bytes) as f64 / batches,
    )
}

/// Samples this role's non-idle backends every 25ms until `stop`, on one
/// dedicated connection, bucketed by state, wait event and the statement's
/// first 60 characters: the report then says whether a batch's time is
/// Postgres CPU, an IO or lock wait, or the gateway between two statements.
/// Statement text is parameterized. `None` when the view is unreadable.
async fn sample_backends(
    pool: PgPool,
    stop: Arc<AtomicBool>,
) -> Option<(usize, usize, Vec<(usize, String)>)> {
    let mut conn = pool
        .acquire()
        .await
        .map_err(|error| eprintln!("session append load: backend samples unavailable: {error}"))
        .ok()?;
    let mut buckets: HashMap<String, usize> = HashMap::new();
    let mut backends = std::collections::HashSet::new();
    let mut samples = 0;
    while !stop.load(Ordering::Relaxed) {
        let rows = sqlx::query_as::<_, (i32, String)>(
            "select pid, coalesce(state, '?') || ' ' || coalesce(wait_event_type, '-') || ':' \
             || coalesce(wait_event, '-') || ' [' \
             || left(regexp_replace(query, '\\s+', ' ', 'g'), 60) || ']' \
             from pg_stat_activity where backend_type = 'client backend' \
             and usename = session_user and pid <> pg_backend_pid() and state <> 'idle'",
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(|error| eprintln!("session append load: backend samples unavailable: {error}"))
        .ok()?;
        samples += 1;
        for (pid, bucket) in rows {
            backends.insert(pid);
            *buckets.entry(bucket).or_default() += 1;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let mut buckets: Vec<(usize, String)> = buckets
        .into_iter()
        .map(|(bucket, count)| (count, bucket))
        .collect();
    buckets.sort_unstable_by(|left, right| right.cmp(left));
    buckets.truncate(10);
    Some((samples, backends.len(), buckets))
}

// ── The throughput AC ────────────────────────────────────────────────────────

/// The load AC: 1k events/s sustained on dev hardware — a hundred
/// 100-event batches paced every 99ms, every batch fully admitted, and the
/// ack median inside the 20ms budget plus 250 measured `select 1` round
/// trips (the delta-over-baseline discipline: the ack path crosses the
/// database link 13 times, and the owner sized the multiple from hosted
/// runs of migration 0012, whose per-row source projection costs Postgres
/// CPU that a `select 1` does not measure). The bound binds in
/// optimized code; a debug build measures the compiler, not the ack path.
/// The database gate therefore leaves it out of the debug workspace run and
/// runs it alone under the optimized `ingest-load` profile, first, on the
/// fresh volume this benchmark assumes (`scripts/db-test.sh`, `Cargo.toml`).
/// Every execution asserts the same bound, and the report adds the
/// first-ten and last-ten batch medians, the tables each batch read, where
/// its database time went and what the gateway's backends were doing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "serial observe throughput AC; the database gate runs it alone under the optimized ingest-load profile"]
async fn append_ack_sustains_1k_events_per_second() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    const BATCHES: usize = 100;
    let (_, _platform) = seed_hierarchy(&pool, tenant).await;
    let idp = MockIdp::spawn().await;
    let app = router(state(&db_url, &idp.issuer, tenant));
    seed_user(&pool, tenant, "loader").await;
    let bearer = idp.user_token("loader");

    // **One run per batch, because that is the shape a fleet produces.**
    //
    // An append takes its session's row lock before reading `max(sequence)`
    // (ADR-0076), so two writers to one run serialise. That is a
    // deliberate trade — the optimistic alternative is only faster when two
    // clients append to *one* run at once, which is not what a fleet does.
    //
    // Worth recording what porting this AC actually found, because the obvious
    // hypothesis was wrong. Spreading the load across runs changed the ack
    // median by ~0.3ms: the lock was **not** the cost. The cost was that the
    // append inserted one row per round trip where `/v1/observe` had used a
    // single `unnest`, so a hundred-event batch paid a hundred link crossings
    // — p50 35.6ms against a 22ms budget. Batching the insert
    // (`synveda_store::sessions::append_events`) took it to 19.0ms. The lock
    // is still real and still serialises one run; it is simply not what this
    // measurement was catching.
    let first = session_seed::seed_run_for(&pool, tenant, "mem1-load", "loader").await;
    let mut runs = vec![first.session_id];
    for index in 1..BATCHES {
        runs.push(
            session_seed::open_run(
                &pool,
                tenant,
                first.workspace_id,
                &format!("mem1-load-{index}"),
                "loader",
            )
            .await,
        );
    }
    let run = runs[0];

    // `make db-test` supplies a fresh isolated volume. Table maintenance is
    // intentionally not part of this request-plane benchmark: PostgreSQL 17
    // reserves VACUUM for MAINTAIN/table-owner authority, which the gateway
    // credential must never hold.

    // The Docker-link baseline, for the failure message.
    let mut baseline = Vec::with_capacity(20);
    for _ in 0..20 {
        let start = Instant::now();
        sqlx::query_scalar!("select 1")
            .fetch_one(&pool)
            .await
            .expect("baseline round-trip");
        baseline.push(start.elapsed());
    }
    baseline.sort_unstable();
    let baseline_median = baseline[baseline.len() / 2];

    // Warmup: the first requests pay one-time costs (JWKS fetch, cache
    // fills, per-connection prepared statements) that steady-state acks
    // do not.
    const WARMUPS: usize = 3;
    for warmup in 0..WARMUPS {
        let key = format!("warmup-{warmup}");
        let (status, body) = send(
            &app,
            request(
                Method::POST,
                &events_uri(run),
                &bearer,
                Some(batch("warmup", &[key.as_str()])),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "warmup: {body}");
    }

    // Outside `elapsed`: lets the seeding and warmup counters publish, so the
    // read report covers the paced load alone.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let reads_before = table_reads(&pool).await;
    let time_before = database_time(&pool).await;
    let stop_sampling = Arc::new(AtomicBool::new(false));
    let sampler = tokio::spawn(sample_backends(pool.clone(), stop_sampling.clone()));

    const EVENTS_PER_BATCH: usize = 100;
    const INTERVAL: Duration = Duration::from_millis(99);
    let mut ticker = tokio::time::interval(INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
    let mut tasks = tokio::task::JoinSet::new();
    let started = Instant::now();
    for (index, run) in runs.iter().copied().enumerate().take(BATCHES) {
        ticker.tick().await;
        let app = app.clone();
        let bearer = bearer.clone();
        tasks.spawn(async move {
            let keys: Vec<String> = (0..EVENTS_PER_BATCH)
                .map(|event| format!("load-{index}-{event}"))
                .collect();
            let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
            let payload = batch(&format!("load-session-{index}"), &refs);
            let start = Instant::now();
            let (status, body) = send(
                &app,
                request(Method::POST, &events_uri(run), &bearer, Some(payload)),
            )
            .await;
            let ack = start.elapsed();
            assert_eq!(status, StatusCode::OK, "batch {index}: {body}");
            assert_eq!(body["appended"], EVENTS_PER_BATCH, "batch {index}: {body}");
            (index, ack, start.duration_since(started))
        });
    }
    let mut paced: Vec<(usize, Duration)> = Vec::with_capacity(BATCHES);
    let mut sends: Vec<Duration> = Vec::with_capacity(BATCHES);
    while let Some(joined) = tasks.join_next().await {
        let (index, ack, sent) = joined.expect("load task");
        paced.push((index, ack));
        sends.push(sent);
    }
    let elapsed = started.elapsed();
    stop_sampling.store(true, Ordering::Relaxed);
    let samples = match sampler.await.expect("backend sampler") {
        Some((samples, backends, buckets)) => format!(
            "{samples} samples, {backends} backends: {}",
            buckets
                .iter()
                .map(|(count, bucket)| format!("{count} {bucket}"))
                .collect::<Vec<_>>()
                .join("; ")
        ),
        None => "unavailable".to_owned(),
    };
    tokio::time::sleep(Duration::from_secs(2)).await;
    let reads = match (reads_before, table_reads(&pool).await) {
        (Some(before), Some(after)) => heaviest_reads(&before, &after, BATCHES),
        _ => "unavailable".to_owned(),
    };
    let time = match (time_before, database_time(&pool).await) {
        (Some(before), Some(after)) => database_time_per_batch(before, after, BATCHES),
        _ => "unavailable".to_owned(),
    };

    assert_eq!(paced.len(), BATCHES);
    // Batch order, not completion order: a cost that grows with the data
    // this load writes shows as a later median above the earlier one.
    paced.sort_unstable();
    let median_of = |batches: &[(usize, Duration)]| {
        let mut acks: Vec<Duration> = batches.iter().map(|(_, ack)| *ack).collect();
        acks.sort_unstable();
        acks[acks.len() / 2]
    };
    let first_ten = median_of(&paced[..10]);
    let last_ten = median_of(&paced[BATCHES - 10..]);
    let mut acks: Vec<Duration> = paced.iter().map(|(_, ack)| *ack).collect();
    assert_eq!(
        staged(&pool, tenant).await as usize,
        BATCHES * EVENTS_PER_BATCH + WARMUPS,
        "every load event admitted exactly once (plus the warmups)"
    );

    // The database gate owns a disposable volume. Keeping the immutable
    // evidence intact exercises the same retention boundary as production;
    // privileged deletion and VACUUM belong to operator acceptance, not this
    // request-plane throughput test.
    acks.sort_unstable();
    let percentile = |p: f64| {
        let rank = ((acks.len() as f64) * p).ceil() as usize;
        acks[rank.clamp(1, acks.len()) - 1]
    };
    let p50 = percentile(0.50);
    let p95 = percentile(0.95);
    let p99 = percentile(0.99);

    // The sustained-rate half of the AC: the paced generator finished its
    // schedule with every batch fully admitted. The rate is taken over the
    // send schedule, first send to last send plus one interval, so the last
    // batch's ack is judged by the ack half like every other (owner,
    // 2026-10-21; counting it here failed runs whose last ack exceeded one
    // interval while the median met its budget). The generator paces at
    // 99ms, about 1,010 events/s offered: paced at exactly 100ms, the
    // schedule is exactly the AC's 1,000/s, and timer rounding alone
    // failed a run at 999.5/s (owner, 2026-10-21).
    let first_send = sends.iter().min().copied().unwrap_or_default();
    let last_send = sends.iter().max().copied().unwrap_or_default();
    let schedule = last_send - first_send + INTERVAL;
    let rate = (BATCHES * EVENTS_PER_BATCH) as f64 / schedule.as_secs_f64();

    // The ack half: the <20ms local budget (seed §10) plus a multiple of the
    // measured select-1 round trip. The ack path makes 13 round trips:
    // tenant resolution, BEGIN, set_config, identity, assignments, default
    // pack, bindings, the batch insert, send_batch, the three audit-append
    // statements, and COMMIT (the scope-chain reads are cache hits). The
    // multiple is larger because the batch insert also spends Postgres CPU
    // in the per-row source projection, which select-1 does not measure.
    // With migration 0012, two hosted runs measured p50 50.50ms at a
    // 217µs select-1 and 39.40ms at 155µs. The owner set the multiple to
    // 250 (2026-10-20): it covers the slower p50 at the faster link, and
    // the routines before 0012 (hosted p50 155ms or more at 306–380µs)
    // still fail it. A slow local link (Docker Desktop, about 2ms) makes
    // the budget loose. Asserted at the MEDIAN, the hierarchy store precedent for
    // IO-crossing perf ACs on dev hardware: every commit here fsyncs WAL
    // through Docker Desktop's virtual disk, whose periodic 30–100ms stalls own the upper
    // percentiles — a tail assertion would measure the hypervisor, not
    // the ack path. p95/p99 are reported; production objectives and
    // production-shaped IO are the load suite's charter.
    const ACK_ROUND_TRIP_MULTIPLE: u32 = 250;
    let budget = Duration::from_millis(20) + ACK_ROUND_TRIP_MULTIPLE * baseline_median;
    eprintln!(
        "session append load: {} events sent over {schedule:.2?} \
         ({rate:.0} events/s sustained), last ack {elapsed:.2?} after start, \
         ack p50 {p50:.2?} p95 {p95:.2?} p99 {p99:.2?}, \
         select-1 baseline {baseline_median:.2?}, budget {budget:.2?}, \
         first-ten median {first_ten:.2?}, last-ten median {last_ten:.2?}",
        BATCHES * EVENTS_PER_BATCH,
    );
    eprintln!("session append load reads per batch: {reads}");
    eprintln!(
        "session append load database time per batch (with the sampler's own queries): {time}"
    );
    eprintln!("session append load backend samples, every 25ms: {samples}");
    assert!(
        rate >= 1000.0,
        "1k events/s must be sustained, got {rate:.0}/s sent over {schedule:.2?}"
    );
    assert!(
        p50 <= budget,
        "the ack budget is <20ms (seed §10) plus \
         {ACK_ROUND_TRIP_MULTIPLE} select-1 round trips: \
         median {p50:.2?} > {budget:.2?} \
         (select-1 baseline {baseline_median:.2?}; run with \
         --profile ingest-load)"
    );
}
