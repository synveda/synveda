//! CTX-3 latency AC (ADR-0026 decision 9): 1,000 concurrent sessions
//! inject against one tenant, arriving open-loop at 50/s — a 3×
//! thundering herd over "every session reconnects within a minute" —
//! with warm caches, asserted as MEDIAN under the 150ms budget with
//! the tails and the per-stage split (plan/embed/search/compose/audit)
//! reported: the HIER-1/MEM-1/CTX-1 discipline (MEM-1's shape exactly:
//! a stated sustained rate, the median asserted). Docker Desktop's
//! virtualised fsync owns the upper percentiles (the audit append
//! commits inside the request), and EVAL-6 owns percentile SLO
//! enforcement on production-shaped IO.
//!
//! A second, REPORTED-ONLY window then saturates the route closed-loop
//! (32 in flight) to print the per-tenant ceiling: appends serialize on
//! the tenant's chain-head lock, so saturated session-starts on ONE
//! tenant bound throughput and inflate the audit stage. That number is
//! the ADR-0019 option 2 trigger's standing evidence — measured every
//! run, asserted never (the SLO's load shape is the herd, not a
//! closed-loop hammer; the buffered read-path appender is the recorded
//! upgrade if real deployments approach the ceiling).
//!
//! The embedder is the deterministic one (in-process, µs) — the TEI
//! round-trip is exercised by demos/ctx-3-inject.sh against the live
//! stack and bounded by the embed deadline either way.
//!
//! `#[ignore]`d: seeding plus 1,250 requests (50 warm, 1,000 asserted,
//! 200 probe) take ~30s — run it alone, against the dev stack:
//!
//! ```text
//! DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda \
//!   cargo test -p synveda-gateway --test inject_latency -- --ignored --nocapture
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::json;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::{Hs256Verifier, personal_slug};
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder, Embedder as _};
use synveda_policy::Pdp;
use synveda_retrieval::index::SearchIndex;
use synveda_retrieval::indexer::{self, IndexerConfig};
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::{hierarchy, identities, tenants};
use synveda_types::{
    IdentityId, IdentityKind, RecordClass, RecordId, RecordKind, ScopeId, ScopeKind, Sensitivity,
    TenantId, TenantStatus,
};
use tower::ServiceExt;

const SECRET: &[u8] = b"ctx-3-latency-secret";
const USERS: usize = 50;
const SESSIONS: usize = 1_000;
/// The asserted window's arrival process: one session-start every 20ms
/// (50/s) — all 1,000 concurrent sessions inject within 20s, 3× the
/// all-at-once-within-a-minute reconnect herd.
const ARRIVAL_INTERVAL: Duration = Duration::from_millis(20);
/// The reported-only saturation probe's in-flight depth and size.
const PROBE_CONCURRENCY: usize = 32;
const PROBE_SESSIONS: usize = 200;
const BUDGET: Duration = Duration::from_millis(150);
/// Corpus: shared material at org/dept/team plus per-user records —
/// every caller composes over four scopes with full candidate lists.
const ORG_RECORDS: usize = 25;
const DEPT_RECORDS: usize = 25;
const TEAM_RECORDS: usize = 35;
const USER_RECORDS: usize = 10;

const TASKS: [&str; 4] = [
    "kubernetes rollout upgrade procedure",
    "postgres vacuum maintenance window",
    "audit chain verification runbook",
    "tenant policy pack review",
];

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let index = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[index]
}

async fn seed_scope_records(
    pool: &PgPool,
    tenant: TenantId,
    scope: ScopeId,
    owner: IdentityId,
    count: usize,
    label: &str,
) {
    let embedder = DeterministicEmbedder::new();
    for index in 0..count {
        let kind = if index % 8 == 0 {
            RecordKind::Pinned
        } else {
            RecordKind::Derived
        };
        let content = format!(
            "{label} record {index}: {} notes for the load corpus.",
            TASKS[index % TASKS.len()]
        );
        let vector = embedder
            .embed(std::slice::from_ref(&content))
            .await
            .expect("embed")
            .remove(0);
        records::insert(
            pool,
            RecordId::new(),
            tenant,
            &RecordState {
                scope_id: scope,
                owner_id: owner,
                kind,
                class: RecordClass::Fact,
                content,
                sensitivity: Sensitivity::Internal,
                provenance: json!({"source": "ctx-3 latency corpus"}),
                valid_from: chrono::Utc::now(),
                valid_to: None,
            },
            &RecordEmbedding {
                model: embedder.model().to_owned(),
                vector,
            },
        )
        .await
        .expect("insert record");
    }
}

async fn inject_once(app: &Router, token: &str, session: usize) -> Duration {
    let body = json!({
        "task": TASKS[session % TASKS.len()],
        "session_id": format!("load-{session}"),
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/inject")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build request");
    let start = Instant::now();
    let response = app.clone().oneshot(request).await.expect("send request");
    let elapsed = start.elapsed();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "every load inject must succeed"
    );
    elapsed
}

/// Prints avg seconds per inject stage from the Prometheus exposition —
/// the ADR-0019 option 2 trigger's evidence.
fn report_stage_split(rendered: &str) {
    for stage in ["plan", "embed", "search", "compose", "audit"] {
        let mut sum = None;
        let mut count = None;
        for line in rendered.lines() {
            if line.contains("synveda_inject_stage_duration_seconds_sum")
                && line.contains(&format!("stage=\"{stage}\""))
            {
                sum = line
                    .split_whitespace()
                    .last()
                    .and_then(|v| v.parse::<f64>().ok());
            }
            if line.contains("synveda_inject_stage_duration_seconds_count")
                && line.contains(&format!("stage=\"{stage}\""))
            {
                count = line
                    .split_whitespace()
                    .last()
                    .and_then(|v| v.parse::<f64>().ok());
            }
        }
        if let (Some(sum), Some(count)) = (sum, count)
            && count > 0.0
        {
            eprintln!("  stage {stage:>8}: avg {:.2?} over {count} calls", {
                Duration::from_secs_f64(sum / count)
            });
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "seeds a load corpus and times 2,050 requests; run alone against the dev stack"]
async fn inject_median_under_budget_at_1k_concurrent_sessions() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping CTX-3 latency AC: DATABASE_URL is not set");
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect");
    synveda_store::migrate(&pool).await.expect("migrations");

    // ── Seed: one tenant, org → eng → platform, 50 placed users ─────────
    let tenant = TenantId::new();
    let slug = format!("ctx3l-{}", tenant.as_uuid().simple());
    tenants::create(
        &pool,
        tenant,
        &slug,
        "CTX-3 latency tenant",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
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
    .expect("org");
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
    .expect("dept");
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
    .expect("team");
    tx.commit().await.expect("commit hierarchy");

    let mut tokens: Vec<String> = Vec::with_capacity(USERS);
    let mut leaves: Vec<(ScopeId, IdentityId)> = Vec::with_capacity(USERS);
    for user in 0..USERS {
        let subject = format!("load-user-{user}");
        let mut tx = pool.begin().await.expect("begin");
        let id = IdentityId::new();
        let leaf = hierarchy::create(
            &mut tx,
            ScopeId::new(),
            tenant,
            Some(platform.id),
            ScopeKind::User,
            &personal_slug(None, &subject, id),
            &subject,
        )
        .await
        .expect("personal scope");
        identities::create(
            &mut tx,
            id,
            tenant,
            Some(&subject),
            IdentityKind::User,
            None,
            None,
            leaf.id,
        )
        .await
        .expect("identity");
        tx.commit().await.expect("commit user");
        tokens.push(Hs256Verifier::new(SECRET).issue(&subject, tenant, Duration::from_secs(3600)));
        leaves.push((leaf.id, id));
    }

    // ── Corpus ───────────────────────────────────────────────────────────
    let owner = leaves[0].1;
    seed_scope_records(&pool, tenant, org.id, owner, ORG_RECORDS, "org").await;
    seed_scope_records(&pool, tenant, eng.id, owner, DEPT_RECORDS, "dept").await;
    seed_scope_records(&pool, tenant, platform.id, owner, TEAM_RECORDS, "team").await;
    for (leaf, identity) in &leaves {
        seed_scope_records(&pool, tenant, *leaf, *identity, USER_RECORDS, "user").await;
    }

    // ── The gateway, production-shaped state ─────────────────────────────
    let index_root = std::env::temp_dir()
        .join("synveda-ctx3-latency")
        .join(tenant.to_string());
    let search_index = Arc::new(SearchIndex::open(&index_root).expect("open sidecar"));
    let swept = indexer::sweep_tenant(
        &pool,
        &search_index,
        tenant,
        &IndexerConfig {
            overlap: Duration::ZERO,
            ..IndexerConfig::default()
        },
    )
    .await
    .expect("sweep");
    assert_eq!(
        swept.upserts as usize,
        ORG_RECORDS + DEPT_RECORDS + TEAM_RECORDS + USERS * USER_RECORDS,
        "sidecar converged"
    );
    let state = AppState {
        pool: PgPoolOptions::new()
            .max_connections(16)
            .connect_lazy(&url)
            .expect("parse url"),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp: Arc::new(Pdp::new().expect("build the embedded PDP")),
        scope_chains: Arc::new(synveda_store::ScopeChainCache::new()),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index,
        embedder: Arc::new(AnyEmbedder::Deterministic(DeterministicEmbedder::new())),
        inject_embed_timeout: Duration::from_millis(100),
    };
    let app = router(state);

    // ── The Docker-link baseline (delta-over-baseline discipline) ────────
    let mut baseline = Vec::with_capacity(20);
    for _ in 0..20 {
        let start = Instant::now();
        sqlx::query_scalar!("select 1 as one")
            .fetch_one(&pool)
            .await
            .expect("baseline round-trip");
        baseline.push(start.elapsed());
    }
    baseline.sort_unstable();
    let baseline_median = baseline[baseline.len() / 2];

    // ── Warm pass: every session's caches, excluded from the SLO ────────
    // (seed §10: "excluding first-call cold cache"): scope chains, Cedar
    // fragments, the sidecar reader slot, statement caches.
    for (user, token) in tokens.iter().enumerate() {
        inject_once(&app, token, user).await;
    }

    // ── The asserted window: 1,000 sessions, open-loop at 50/s ──────────
    let tokens = Arc::new(tokens);
    let started = Instant::now();
    let mut sessions = Vec::with_capacity(SESSIONS);
    for session in 0..SESSIONS {
        let app = app.clone();
        let tokens = Arc::clone(&tokens);
        let arrive_at = started + ARRIVAL_INTERVAL * session as u32;
        sessions.push(tokio::spawn(async move {
            tokio::time::sleep_until(arrive_at.into()).await;
            let token = &tokens[session % tokens.len()];
            inject_once(&app, token, session).await
        }));
    }
    let mut latencies: Vec<Duration> = Vec::with_capacity(SESSIONS);
    for session in sessions {
        latencies.push(session.await.expect("session"));
    }
    let window = started.elapsed();
    assert_eq!(latencies.len(), SESSIONS);
    latencies.sort_unstable();
    let p50 = percentile(&latencies, 0.50);
    let p95 = percentile(&latencies, 0.95);
    let p99 = percentile(&latencies, 0.99);

    eprintln!(
        "CTX-3 latency AC: {SESSIONS} sessions ({USERS} users, arriving at {:.0}/s) in \
         {window:.2?} — p50 {p50:.2?}, p95 {p95:.2?}, p99 {p99:.2?} \
         (select-1 baseline {baseline_median:.2?}; tails reported, not asserted — EVAL-6)",
        1.0 / ARRIVAL_INTERVAL.as_secs_f64(),
    );
    report_stage_split(&metrics_handle().render());

    // ── The saturation probe (reported, never asserted) ─────────────────
    // Closed-loop 32-deep on one tenant: appends serialize on the
    // chain-head lock, so this prints the per-tenant ceiling — the
    // ADR-0019 option 2 trigger's standing evidence.
    let next = Arc::new(AtomicUsize::new(0));
    let probe_started = Instant::now();
    let mut workers = Vec::with_capacity(PROBE_CONCURRENCY);
    for _ in 0..PROBE_CONCURRENCY {
        let app = app.clone();
        let tokens = Arc::clone(&tokens);
        let next = Arc::clone(&next);
        workers.push(tokio::spawn(async move {
            let mut latencies = Vec::new();
            loop {
                let session = next.fetch_add(1, Ordering::Relaxed);
                if session >= PROBE_SESSIONS {
                    return latencies;
                }
                let token = &tokens[session % tokens.len()];
                latencies.push(inject_once(&app, token, SESSIONS + session).await);
            }
        }));
    }
    let mut probe: Vec<Duration> = Vec::with_capacity(PROBE_SESSIONS);
    for worker in workers {
        probe.extend(worker.await.expect("probe worker"));
    }
    let probe_window = probe_started.elapsed();
    probe.sort_unstable();
    eprintln!(
        "  saturation probe ({PROBE_SESSIONS} sessions, {PROBE_CONCURRENCY} in flight, one \
         tenant): {:.0}/s ceiling, p50 {:.2?} — the chain-head lock serializes appends; \
         ADR-0019 option 2 is the recorded upgrade if deployments approach this rate",
        PROBE_SESSIONS as f64 / probe_window.as_secs_f64(),
        percentile(&probe, 0.50),
    );

    assert!(
        p50 < BUDGET,
        "inject median {p50:.2?} exceeds the {BUDGET:.0?} budget at {SESSIONS} concurrent sessions"
    );
}
