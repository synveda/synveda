//! Latency of ordinary tenant-transaction admission (ADR-0157 hot path).
//!
//! Reports p50/p95/p99 for `GET /v1/me`, which resolves the tenant and opens
//! one ordinary tenant transaction, and for bare admission plus one indexed
//! read and commit. It asserts only success; the numbers are evidence for a
//! same-machine before/after comparison, not a service objective.

#[path = "../../synveda-store/tests/support/tenant_fixture.rs"]
mod tenant_fixture;

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, header};
use metrics_exporter_prometheus::PrometheusHandle;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, behavior_test_router as router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_types::{TenantId, TenantStatus};
use tower::ServiceExt;

const SECRET: &[u8] = b"tenant-admission-measure-secret";

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn setting(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn report(label: &str, mut values: Vec<Duration>) {
    values.sort_unstable();
    let at = |quantile: f64| {
        let index = ((values.len() as f64) * quantile).ceil() as usize;
        values[index.saturating_sub(1).min(values.len() - 1)]
    };
    eprintln!(
        "tenant-admission-measure {label}: n={} p50={:?} p95={:?} p99={:?} max={:?}",
        values.len(),
        at(0.50),
        at(0.95),
        at(0.99),
        values[values.len() - 1]
    );
}

#[tokio::test]
async fn ordinary_tenant_admission_latency() {
    let Some(url) = tenant_fixture::database_url("tenant admission measurement") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::epoch::verify(&pool)
        .await
        .expect("current schema");
    let tenant = TenantId::new();
    let slug = format!("measure-{}", tenant.as_uuid().simple());
    tenant_fixture::create(&pool, tenant, &slug, "measure", TenantStatus::Active)
        .await
        .expect("admit tenant");
    let mut tx = tenant_fixture::begin(&pool, tenant).await;
    let root = synveda_store::scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("root");
    tx.commit().await.expect("commit root");

    let state = AppState {
        pool: pool.clone(),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: "http://measure.test".to_owned(),
        pdp: Arc::new(synveda_policy::Pdp::new().expect("PDP")),
        service_token_max_ttl: Duration::from_secs(3600),
        embedder: Arc::new(synveda_ingest::embedding::AnyEmbedder::Deterministic(
            synveda_ingest::embedding::DeterministicEmbedder::new(),
        )),
        context_embed_timeout: Duration::from_millis(100),
        git_sources: None,
        git_exports: None,
        keys: Arc::new(synveda_store::keys::KeyRing::new(
            synveda_crypto::Kms::Disabled,
        )),
    };
    let app = router(state);
    let token =
        Hs256Verifier::new(SECRET).issue("measure@example.test", tenant, Duration::from_secs(3600));
    let me = || {
        Request::get("/v1/me")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .expect("request")
    };
    let warmup = setting("SYNVEDA_ADMISSION_MEASURE_WARMUP", 200);
    let samples = setting("SYNVEDA_ADMISSION_MEASURE_SAMPLES", 2_000);
    let expected = app.clone().oneshot(me()).await.expect("send").status();
    assert!(expected.is_success(), "GET /v1/me returned {expected}");
    for _ in 0..warmup {
        assert_eq!(
            app.clone().oneshot(me()).await.expect("send").status(),
            expected
        );
    }

    let mut request = Vec::with_capacity(samples);
    for _ in 0..samples {
        let started = Instant::now();
        let status = app.clone().oneshot(me()).await.expect("send").status();
        request.push(started.elapsed());
        assert_eq!(status, expected);
    }
    report("GET /v1/me", request);

    let mut admission = Vec::with_capacity(samples);
    for _ in 0..samples {
        let started = Instant::now();
        let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
            .await
            .expect("admit");
        synveda_store::scopes::get(&mut *tx, tenant, root.id)
            .await
            .expect("read root")
            .expect("root exists");
        tx.commit().await.expect("commit");
        admission.push(started.elapsed());
    }
    report("admit+read+commit", admission);
}
