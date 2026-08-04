//! MEM-6 acceptance (ADR-0040): retention, disposal and staleness.
//!
//! The AC has two halves and both are here, over the real product path —
//! `POST /v1/observe`, the extraction worker, `POST /v1/inject`, the sweep
//! the gateway spawns — never a hand-written record:
//!
//! * **a retention policy change re-evaluates existing records**: material
//!   that composed a moment ago stops composing on the very next inject
//!   because a pack was applied, with nobody acting, nothing restarted and
//!   no sweep having run — nothing was ever stamped on a record, so there
//!   is nothing to re-evaluate;
//! * **audit trail of expiries**: the sweep then removes it from the live
//!   corpus and chains `memory.expired` under `actor_kind=system`, naming
//!   the horizon, the class, the ids and the ages, with the chain
//!   verifying and no record content anywhere in the payload.
//!
//! Around them: pinned material of the same age survives both layers
//! (seed §4.2 as a rule, not a setting); the destruction horizon takes the
//! history the expiry deliberately left behind, and the as-of query that
//! answered before it stops answering after — which is the difference
//! between the two horizons, asserted rather than described; the observe
//! staging plane is disposed of on its own horizon, discharging the
//! obligation ADR-0020 and ADR-0021 both parked here; and a pack that
//! turns the feature off gets the pre-MEM-6 product back exactly.
//!
//! Tests need a live Postgres: they read `DATABASE_URL` and skip when it is
//! unset (CI has no database) — the tests/dedup.rs harness.

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
use synveda_identity::{OidcVerifier, parse_issuers, personal_slug};
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder};
use synveda_ingest::extraction::{AnyExtractor, DeterministicExtractor};
use synveda_ingest::retention as sweep;
use synveda_ingest::worker::{self, WorkerConfig, WorkerDeps};
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::{hierarchy, identities, policy_packs, rls, tenants};
use synveda_types::{
    ClassTtl, HierarchyNode, Identity, IdentityId, IdentityKind, PackConfig, RecordClass, RecordId,
    RecordKind, RetentionConfig, RetentionMode, ScopeId, ScopeKind, Sensitivity, TenantId,
    TenantStatus,
};
use tower::ServiceExt;

const KEY_PEM: &str = include_str!("fixtures/idp_key_a.pem");
const KEY_JWK: &str = include_str!("fixtures/idp_key_a.jwk.json");
const CLIENT_ID: &str = "synveda-test";

/// The two statements the suite ages differently. Both are `episode`
/// material, which is what a retention schedule shortens first and what
/// `--retention` in the demo shortens here.
const OLD_EPISODE: &str = "Session summary: we walked the staging cluster runbook end to end.";
const RECENT_EPISODE: &str = "Session summary: we rotated the payments sandbox credentials.";

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

// ── Gateway, worker and sweep harness ───────────────────────────────────────

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

/// The sweep wired exactly as the gateway wires it (ADR-0040 decision 14),
/// sharing the gateway's PDP and chain cache so a pack installed a moment
/// ago is the one it resolves.
fn sweep_deps(state: &AppState) -> sweep::SweepDeps {
    sweep::SweepDeps {
        pool: state.pool.clone(),
        pdp: Arc::clone(&state.pdp),
        chains: Arc::clone(&state.scope_chains),
    }
}

async fn run_sweep(state: &AppState, tenant: TenantId) -> sweep::TenantPass {
    sweep::run_tenant(&sweep_deps(state), &sweep::SweepConfig::default(), tenant)
        .await
        .expect("retention pass")
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
                // `tool_result` is what the deterministic extractor routes
                // to `episode` (the class a real schedule shortens first).
                "kind": "tool_result",
                "payload": {"text": text},
                "occurred_at": at.to_rfc3339(),
            }],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "observe: {body}");
    assert_eq!(body["accepted"], json!(1), "observe: {body}");
}

/// The composed block a cold session start receives.
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
                "skipping MEM-6 retention test: DATABASE_URL is not set \
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
    let slug = format!("mem6-{}", id.as_uuid().simple());
    tenants::create(&pool, id, &slug, "MEM-6 test tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((pool, id, url))
}

/// acme-org → platform (team) plus the reserved quarantine team.
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

/// This tenant's current records, oldest valid-time first (superuser test
/// connection — RLS-exempt on purpose; the RLS suite owns isolation).
async fn current_records(pool: &PgPool, tenant: TenantId) -> Vec<(RecordId, String)> {
    sqlx::query!(
        r#"select id, content from records where tenant_id = $1 order by valid_from, id"#,
        tenant.as_uuid(),
    )
    .fetch_all(pool)
    .await
    .expect("read records")
    .into_iter()
    .map(|row| (RecordId::from_uuid(row.id), row.content))
    .collect()
}

/// Every version the database has ever held for this tenant, current and
/// archived — the as-of surface's whole universe, and what destruction
/// takes away.
async fn all_versions(pool: &PgPool, tenant: TenantId) -> Vec<String> {
    sqlx::query_scalar!(
        r#"select content as "content!" from records_versions where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_all(pool)
    .await
    .expect("read versions")
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

/// The permits this suite's own material needs — write to your own home,
/// read your own chain — and nothing else, so the only thing a pack built
/// on it changes is the configuration under test.
const MEMBER_PACK: &str = r#"
    permit (principal, action == Synveda::Action::"MemoryRead", resource)
    when { principal in resource };
    permit (principal, action == Synveda::Action::"MemoryWrite", resource)
    when { principal has home && resource == principal.home };
"#;

const PACK_NAME: &str = "mem6-pack";

/// Applies a retention schedule as a tenant pack and makes it the default —
/// stored *and* installed into the PDP, so the very next request and the
/// very next sweep both see it, which is what the gateway's refresher does
/// a few seconds later in production.
async fn apply_retention(
    pool: &PgPool,
    state: &AppState,
    tenant: TenantId,
    retention: RetentionConfig,
) {
    let mut tx = rls::begin_tenant_tx(pool, tenant).await.expect("tenant tx");
    let pack = policy_packs::apply(
        &mut *tx,
        tenant,
        PACK_NAME,
        MEMBER_PACK,
        &PackConfig {
            retention: Some(retention),
            ..PackConfig::default()
        },
    )
    .await
    .expect("apply pack");
    synveda_store::policy_assignments::set_default(&mut *tx, tenant, PACK_NAME)
        .await
        .expect("make it the tenant default");
    tx.commit().await.expect("commit pack");
    state
        .pdp
        .install_source(
            tenant,
            PACK_NAME,
            pack.version,
            MEMBER_PACK,
            PackConfig {
                retention: Some(retention),
                ..PackConfig::default()
            },
        )
        .expect("install stored pack into the PDP");
}

/// A schedule that keeps episodes for `days` and everything else forever.
fn episodes_for(days: u32) -> RetentionConfig {
    RetentionConfig {
        ttl: ClassTtl {
            episode: days,
            ..ClassTtl::KEEP
        },
        ..RetentionConfig::DEFAULT
    }
}

// ── The AC ──────────────────────────────────────────────────────────────────

/// **AC: a retention policy change re-evaluates existing records, and
/// expiries are audited.**
///
/// One identity, two session summaries of different ages. The schedule
/// arrives *after* both of them exist, which is the whole point: nothing
/// was stamped on either record when it was written, so the pack that
/// governs them is the one in force at the moment somebody asks.
#[tokio::test]
async fn a_retention_policy_change_governs_the_very_next_inject_and_the_expiry_is_audited() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let platform = seed_hierarchy(&pool, tenant).await;
    let alice = seed_user(&pool, tenant, "alice", platform.id).await;
    let idp = MockIdp::spawn().await;
    let state = state(&db_url, &idp.issuer, tenant);
    let app = router(state.clone());
    let deps = worker_deps(&state);
    let config = worker_config();
    let token = idp.user_token("alice");

    // Ninety days ago, and yesterday. Explicit instants, because valid
    // time is the retention clock (ADR-0040 decision 3).
    let long_ago = Utc::now() - ChronoDuration::days(90);
    let yesterday = Utc::now() - ChronoDuration::days(1);
    observe(&app, &token, "sess-old", long_ago, OLD_EPISODE).await;
    observe(&app, &token, "sess-new", yesterday, RECENT_EPISODE).await;
    drain(&deps, &config).await;

    let stored = current_records(&pool, tenant).await;
    assert_eq!(stored.len(), 2, "both were extracted: {stored:?}");

    // Under the product default — the machinery on, no record horizon —
    // both compose. This is the pre-change state and it must be the
    // *product's* behaviour, not a test fixture's.
    let before = inject_text(&app, &token, "cold-1").await;
    assert!(
        before.contains("runbook"),
        "the old summary is there: {before}"
    );
    assert!(
        before.contains("credentials"),
        "and so is the recent one: {before}"
    );

    // ── The AC's first half ─────────────────────────────────────────────
    // A steward shortens the schedule. Nobody touches a record, no sweep
    // runs, nothing restarts.
    apply_retention(&pool, &state, tenant, episodes_for(30)).await;

    let after = inject_text(&app, &token, "cold-2").await;
    assert!(
        !after.contains("runbook"),
        "the very next inject stops serving material past the new horizon: {after}"
    );
    assert!(
        after.contains("credentials"),
        "and keeps what is still inside it: {after}"
    );
    assert_eq!(
        current_records(&pool, tenant).await.len(),
        2,
        "the read path refused it; nothing has been deleted yet — enforcement \
         and disposal are two different acts (ADR-0040 decision 2)"
    );

    // ── The AC's second half ────────────────────────────────────────────
    let pass = run_sweep(&state, tenant).await;
    assert_eq!(pass.expired, 1, "the sweep expired exactly the due record");

    let remaining = current_records(&pool, tenant).await;
    assert_eq!(remaining.len(), 1, "one record left: {remaining:?}");
    assert!(remaining[0].1.contains("credentials"));

    // The temporal delete: the record left the corpus, its version did
    // not leave the database. This is the difference between MEM-6's two
    // horizons, and the reason the regulated-industry demo still works
    // for everything that has expired but not yet been destroyed.
    let versions = all_versions(&pool, tenant).await;
    assert!(
        versions.iter().any(|content| content.contains("runbook")),
        "the expired record's version is archived, not destroyed: {versions:?}"
    );

    let events = audit_rows(&pool, tenant, "memory.expired").await;
    assert_eq!(events.len(), 1, "one event for the batch: {events:?}");
    let (actor_kind, outcome, payload) = &events[0];
    assert_eq!(actor_kind, "system", "no human expired this");
    assert_eq!(outcome, "success");
    assert_eq!(payload["count"], json!(1));
    assert_eq!(payload["scope_id"], json!(alice.scope_id));
    let horizon = payload["horizons"]
        .as_array()
        .expect("horizons")
        .iter()
        .find(|entry| entry["class"] == json!("episode"))
        .expect("the class that expired is named");
    assert_eq!(horizon["ttl_days"], json!(30), "the schedule that decided");
    let record = &payload["records"][0];
    assert_eq!(record["class"], json!("episode"));
    assert!(
        record["age_days"].as_i64().expect("age") >= 90,
        "how old it was, in whole days: {payload}"
    );
    assert!(
        !payload.to_string().contains("runbook"),
        "no audit payload carries record content — the point of the act is that \
         the content stops being available: {payload}"
    );
    assert_chain_verifies(&pool, tenant).await;

    // A second pass is a no-op: the horizon has nothing left to catch.
    assert_eq!(run_sweep(&state, tenant).await.expired, 0);

    let rendered = state.metrics.render();
    assert!(
        rendered.contains(r#"synveda_records_expired_total{class="episode"}"#),
        "the expiry is counted"
    );
}

/// Pinned material of the same age survives the read cut and the sweep.
/// Seed §4.2 — "cannot be shadowed or decayed" — as a rule rather than a
/// setting: there is no pack field that could re-admit it (ADR-0040
/// decision 8).
#[tokio::test]
async fn pinned_material_is_exempt_from_every_horizon() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let platform = seed_hierarchy(&pool, tenant).await;
    let alice = seed_user(&pool, tenant, "alice", platform.id).await;
    let idp = MockIdp::spawn().await;
    let state = state(&db_url, &idp.issuer, tenant);
    let app = router(state.clone());
    let token = idp.user_token("alice");

    // Pinned records have no authoring surface yet (PRMT-2's context packs
    // are where one arrives), so this one is written through the store —
    // the only seeded row in the suite, and it is seeded precisely because
    // the exemption must hold for material this feature cannot create.
    let ancient = Utc::now() - ChronoDuration::days(400);
    let pinned_id = RecordId::new();
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("tenant tx");
    records::insert(
        &mut *tx,
        pinned_id,
        tenant,
        &RecordState {
            scope_id: alice.scope_id,
            owner_id: alice.id,
            kind: RecordKind::Pinned,
            class: RecordClass::Episode,
            content: "Canonical: the incident review of the 2025 outage.".to_owned(),
            sensitivity: Sensitivity::WORKING,
            provenance: json!({"source": "test"}),
            valid_from: ancient,
            valid_to: None,
        },
        &RecordEmbedding {
            model: "hash@1".to_owned(),
            vector: vec![0.1; 16],
        },
    )
    .await
    .expect("insert pinned record");
    tx.commit().await.expect("commit");

    // A schedule that would catch it twice over if it governed pinned
    // material at all.
    apply_retention(&pool, &state, tenant, episodes_for(30)).await;

    let block = inject_text(&app, &token, "cold-1").await;
    assert!(
        block.contains("incident review"),
        "pinned material is not subject to the read cut: {block}"
    );
    assert_eq!(
        run_sweep(&state, tenant).await.expired,
        0,
        "and the sweep does not touch it either"
    );
    assert_eq!(current_records(&pool, tenant).await.len(), 1);
}

/// The second horizon: what expiry deliberately left behind is destroyed,
/// and the as-of question that had an answer stops having one. That
/// transition *is* the difference between the two horizons (ADR-0040
/// decision 5), so it is asserted rather than described.
#[tokio::test]
async fn the_destruction_horizon_takes_the_history_the_expiry_left() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let platform = seed_hierarchy(&pool, tenant).await;
    seed_user(&pool, tenant, "alice", platform.id).await;
    let idp = MockIdp::spawn().await;
    let state = state(&db_url, &idp.issuer, tenant);
    let app = router(state.clone());
    let deps = worker_deps(&state);
    let config = worker_config();
    let token = idp.user_token("alice");

    let long_ago = Utc::now() - ChronoDuration::days(90);
    observe(&app, &token, "sess-old", long_ago, OLD_EPISODE).await;
    drain(&deps, &config).await;

    apply_retention(&pool, &state, tenant, episodes_for(30)).await;
    assert_eq!(run_sweep(&state, tenant).await.expired, 1);
    assert!(
        all_versions(&pool, tenant)
            .await
            .iter()
            .any(|content| content.contains("runbook")),
        "after expiry the history still answers"
    );

    // A destruction horizon is measured in days from the instant a version
    // closed, and a test cannot wait a day. The archived row is backdated
    // through the superuser test connection — the same trigger suppression
    // the AUD-1 tamper suite performs, here to age a fixture rather than to
    // attack anything.
    sqlx::query!("alter table records_history disable trigger records_history_append_only")
        .execute(&pool)
        .await
        .expect("suspend the append-only trigger");
    // Both ends move: a transaction period is half-open and checked
    // (`tx_from < tx_to`), so ageing only the close would forge a version
    // that ended before it began.
    sqlx::query!(
        "update records_history \
         set tx_from = tx_from - interval '20 days', tx_to = tx_to - interval '10 days' \
         where tenant_id = $1",
        tenant.as_uuid(),
    )
    .execute(&pool)
    .await
    .expect("age the archived version");
    sqlx::query!("alter table records_history enable trigger records_history_append_only")
        .execute(&pool)
        .await
        .expect("restore the append-only trigger");

    let mut destroying = episodes_for(30);
    destroying.destroy_after_days = 7;
    apply_retention(&pool, &state, tenant, destroying).await;

    let pass = run_sweep(&state, tenant).await;
    assert_eq!(pass.destroyed, 1, "the closed version was destroyed");
    let versions = all_versions(&pool, tenant).await;
    assert!(
        !versions.iter().any(|content| content.contains("runbook")),
        "and the content is gone from every version the database holds — this is \
         the half of 'retention enforced' the product did not have: {versions:?}"
    );

    let events = audit_rows(&pool, tenant, "memory.disposed").await;
    let history = events
        .iter()
        .find(|(_, _, payload)| payload["plane"] == json!("records_history"))
        .expect("the destruction is on the chain");
    assert_eq!(history.0, "system");
    assert_eq!(history.2["versions"], json!(1));
    assert_eq!(history.2["destroy_after_days"], json!(7));
    assert!(
        !history.2.to_string().contains("runbook"),
        "what remains is the proof it happened, never what was destroyed: {}",
        history.2
    );
    assert_chain_verifies(&pool, tenant).await;

    let rendered = state.metrics.render();
    assert!(
        rendered.contains(r#"synveda_retention_destroyed_total{plane="history"}"#),
        "the destruction is counted"
    );
}

/// The observe staging plane is disposed of on its own horizon — the
/// obligation ADR-0020 and ADR-0021 both parked here, and the one plane
/// every pack disposes of by default (ADR-0040 decisions 7 and 13).
#[tokio::test]
async fn the_observe_staging_plane_is_disposed_of_on_its_own_horizon() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let platform = seed_hierarchy(&pool, tenant).await;
    seed_user(&pool, tenant, "alice", platform.id).await;
    let idp = MockIdp::spawn().await;
    let state = state(&db_url, &idp.issuer, tenant);
    let app = router(state.clone());
    let deps = worker_deps(&state);
    let config = worker_config();
    let token = idp.user_token("alice");

    observe(&app, &token, "sess", Utc::now(), RECENT_EPISODE).await;
    drain(&deps, &config).await;
    let staged: i64 = sqlx::query_scalar!(
        r#"select count(*) as "count!" from observe_events where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&pool)
    .await
    .expect("count staging");
    assert_eq!(
        staged, 1,
        "the payload is staged, as it has been since MEM-1"
    );

    // Fresh material is not disposed of: the default horizon is 30 days.
    assert_eq!(run_sweep(&state, tenant).await.staging_disposed, 0);

    // Age the staging row past the strict pack's week.
    sqlx::query!(
        "update observe_events set received_at = received_at - interval '40 days' \
         where tenant_id = $1",
        tenant.as_uuid(),
    )
    .execute(&pool)
    .await
    .expect("age the staging row");

    let pass = run_sweep(&state, tenant).await;
    assert_eq!(pass.staging_disposed, 1, "the payload was destroyed");
    let left: i64 = sqlx::query_scalar!(
        r#"select count(*) as "count!" from observe_events where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&pool)
    .await
    .expect("count staging");
    assert_eq!(left, 0);
    assert_eq!(
        current_records(&pool, tenant).await.len(),
        1,
        "disposal of the staging row is not disposal of what was extracted from it"
    );

    let events = audit_rows(&pool, tenant, "memory.disposed").await;
    let staging = events
        .iter()
        .find(|(_, _, payload)| payload["plane"] == json!("observe_staging"))
        .expect("the disposal is on the chain");
    assert_eq!(staging.0, "system");
    assert_eq!(staging.2["events"], json!(1));
    assert_eq!(staging.2["quarantine_pending"], json!(0));
    assert!(
        staging.2["staging_days"].as_u64().expect("horizon") > 0,
        "the horizon that authorised it is named: {}",
        staging.2
    );
    assert_chain_verifies(&pool, tenant).await;
}

/// A pack that turns the feature off gets the pre-MEM-6 product back
/// exactly: the read path serves everything, and the sweep — including the
/// staging plane every other pack disposes of — does nothing at all.
#[tokio::test]
async fn a_pack_can_turn_the_feature_off() {
    let _serial = serial().await;
    let Some((pool, tenant, db_url)) = admitted_tenant().await else {
        return;
    };
    let platform = seed_hierarchy(&pool, tenant).await;
    seed_user(&pool, tenant, "alice", platform.id).await;
    let idp = MockIdp::spawn().await;
    let state = state(&db_url, &idp.issuer, tenant);
    let app = router(state.clone());
    let deps = worker_deps(&state);
    let config = worker_config();
    let token = idp.user_token("alice");

    let long_ago = Utc::now() - ChronoDuration::days(90);
    observe(&app, &token, "sess-old", long_ago, OLD_EPISODE).await;
    drain(&deps, &config).await;
    sqlx::query!(
        "update observe_events set received_at = received_at - interval '400 days' \
         where tenant_id = $1",
        tenant.as_uuid(),
    )
    .execute(&pool)
    .await
    .expect("age the staging row");

    // Horizons set, and the mode off: the numbers must be ignored rather
    // than merely absent, or "off" would be a different feature.
    apply_retention(
        &pool,
        &state,
        tenant,
        RetentionConfig {
            mode: RetentionMode::Off,
            ttl: ClassTtl {
                episode: 1,
                ..ClassTtl::KEEP
            },
            destroy_after_days: 1,
            staging_days: 1,
            staleness_half_life_days: 1,
        },
    )
    .await;

    let block = inject_text(&app, &token, "cold-1").await;
    assert!(
        block.contains("runbook"),
        "a 90-day-old episode under a 1-day horizon still composes when the \
         feature is off: {block}"
    );
    let pass = run_sweep(&state, tenant).await;
    assert_eq!(pass.expired, 0);
    assert_eq!(pass.destroyed, 0);
    assert_eq!(
        pass.staging_disposed, 0,
        "off means off, including the plane every other pack disposes of"
    );
    assert!(
        audit_rows(&pool, tenant, "memory.expired").await.is_empty(),
        "and nothing is chained about an act nobody took"
    );
}
