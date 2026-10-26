//! Manual Capture claim-path latency probe.
//!
//! It drives the public worker sweep for one tenant through the ordinary
//! runtime role and prints p50/p95/p99 for an idle poll over completed
//! history and for a deterministic claim-to-completion. The deterministic
//! extractor makes no provider call, so the figures isolate database work.
//! Run it with
//! `bash scripts/db-test.sh -p synveda-gateway --test capture_claim_probe -- --ignored --nocapture --test-threads=1`.

#[path = "../../synveda-store/tests/support/tenant_fixture.rs"]
mod tenant_fixture;

#[path = "support/configuration.rs"]
mod configuration_support;

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, behavior_test_router as router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_ingest::capture_worker;
use synveda_ingest::extraction::{AnyExtractor, DeterministicExtractor};
use synveda_store::{access, identities, rls, scopes};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::{GrantId, IdentityId, IdentityKind, TenantId, TenantStatus};
use tower::ServiceExt;

const SECRET: &[u8] = b"capture-claim-probe";
const ADMIN: &str = "capture-claim-probe-admin";
const HISTORY: usize = 300;
const IDLE_SAMPLES: usize = 300;
const CLAIM_SAMPLES: usize = 150;

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn state(url: &str) -> AppState {
    AppState {
        pool: PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(5))
            .connect_lazy(url)
            .expect("parse database url"),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp: Arc::new(synveda_policy::Pdp::new().expect("build embedded PDP")),
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
    }
}

async fn call(app: &Router, path: &str, token: &str, key: &str, body: Option<Value>) -> Value {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("idempotency-key", key);
    let request = match body {
        Some(body) => request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())),
        None => request.body(Body::empty()),
    }
    .expect("build request");
    let response = app.clone().oneshot(request).await.expect("router responds");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect response")
        .to_bytes();
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "{path}: {status} {value}"
    );
    value
}

fn percentile(samples: &mut [Duration], quantile: f64) -> f64 {
    samples.sort();
    let index = ((samples.len() as f64 - 1.0) * quantile).round() as usize;
    samples[index].as_secs_f64() * 1_000.0
}

fn report(label: &str, mut samples: Vec<Duration>) -> Value {
    json!({
        "workload": label,
        "samples": samples.len(),
        "p50_ms": percentile(&mut samples, 0.50),
        "p95_ms": percentile(&mut samples, 0.95),
        "p99_ms": percentile(&mut samples, 0.99),
    })
}

#[tokio::test]
#[ignore = "manual Capture claim-path latency probe"]
async fn capture_claim_path_latency_probe() {
    let Some(url) = tenant_fixture::database_url("capture claim probe") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect");
    let tenant_id = TenantId::new();
    tenant_fixture::create(
        &pool,
        tenant_id,
        &format!("probe-{}", tenant_id.as_uuid().simple()),
        "capture claim probe",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    let mut tx = rls::begin_tenant_tx(&pool, tenant_id)
        .await
        .expect("begin bootstrap");
    let root = scopes::ensure_tenant_root(&mut tx, tenant_id)
        .await
        .expect("root");
    let own = scopes::ensure_principal_scope(&mut tx, tenant_id, ADMIN, ADMIN)
        .await
        .expect("principal scope");
    identities::create(
        &mut tx,
        IdentityId::new(),
        tenant_id,
        Some(ADMIN),
        IdentityKind::User,
        None,
        Some(ADMIN),
        own.id,
    )
    .await
    .expect("identity");
    access::create_grant(
        &mut tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id,
            scope_id: root.id,
            subject: GrantSubject::Principal {
                principal_id: ADMIN.to_owned(),
            },
            role_key: RoleKey::Administrator,
            source: GrantSource::Automation,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .expect("grant");
    configuration_support::bind_pack(&mut tx, tenant_id, root.id, synveda_policy::STANDARD).await;
    tx.commit().await.expect("commit bootstrap");
    pool.close().await;

    let state = state(&url);
    let app = router(state.clone());
    let token = Hs256Verifier::new(SECRET).issue(ADMIN, tenant_id, Duration::from_secs(3_600));
    let workspace = call(
        &app,
        "/v1/workspaces",
        &token,
        "probe-workspace",
        Some(json!({"slug": "probe", "display_name": "probe"})),
    )
    .await;
    let project = call(
        &app,
        &format!(
            "/v1/workspaces/{}/projects",
            workspace["id"].as_str().unwrap()
        ),
        &token,
        "probe-project",
        Some(json!({"slug": "probe", "display_name": "probe"})),
    )
    .await;
    let deps = capture_worker::Deps {
        pool: state.pool.clone(),
        pdp: Arc::clone(&state.pdp),
        extractor: Arc::new(AnyExtractor::Deterministic(DeterministicExtractor::new())),
    };
    let config = capture_worker::Config {
        batches_per_tenant: 1,
        lease_owner: format!("capture-claim-probe-{tenant_id}"),
        ..capture_worker::Config::default()
    };
    let freeze = |index: usize| {
        let app = app.clone();
        let token = token.clone();
        let workspace_id = workspace["id"].clone();
        let project_id = project["id"].clone();
        async move {
            let session = call(
                &app,
                "/v1/sessions",
                &token,
                &format!("probe-session-{index}"),
                Some(json!({
                    "workspace_id": workspace_id,
                    "project_id": project_id,
                    "client_name": "capture-claim-probe",
                    "client_version": "1",
                    "task_summary": "probe",
                })),
            )
            .await;
            let session_id = session["id"].as_str().unwrap().to_owned();
            call(
                &app,
                &format!("/v1/sessions/{session_id}/events"),
                &token,
                &format!("probe-events-{index}"),
                Some(json!({"events": [{
                    "event_type": "message.user",
                    "client_event_id": format!("probe-{index}"),
                    "occurred_at": "2026-10-26T10:00:00Z",
                    "payload": {"text": format!("Remember probe convention {index}.")},
                }]})),
            )
            .await;
            call(
                &app,
                &format!("/v1/sessions/{session_id}/capture-batches"),
                &token,
                &format!("probe-freeze-{index}"),
                None,
            )
            .await;
        }
    };

    for index in 0..HISTORY {
        freeze(index).await;
        let summary = capture_worker::sweep_tenant_once(&deps, &config, tenant_id)
            .await
            .expect("seed sweep");
        assert_eq!(summary.completed, 1);
    }
    let mut idle = Vec::with_capacity(IDLE_SAMPLES);
    for _ in 0..IDLE_SAMPLES {
        let started = Instant::now();
        let summary = capture_worker::sweep_tenant_once(&deps, &config, tenant_id)
            .await
            .expect("idle sweep");
        idle.push(started.elapsed());
        assert_eq!(summary.completed, 0);
    }
    let mut claimed = Vec::with_capacity(CLAIM_SAMPLES);
    for index in HISTORY..HISTORY + CLAIM_SAMPLES {
        freeze(index).await;
        let started = Instant::now();
        let summary = capture_worker::sweep_tenant_once(&deps, &config, tenant_id)
            .await
            .expect("claim sweep");
        claimed.push(started.elapsed());
        assert_eq!(summary.completed, 1);
    }
    println!(
        "{}",
        json!({
            "probe": "capture-claim-path",
            "completed_history": HISTORY,
            "results": [
                report("idle_poll_over_history", idle),
                report("deterministic_claim_to_completion", claimed),
            ],
        })
    );
}
