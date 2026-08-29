//! CPR-18 acceptance evidence for the public capture plane.
//!
//! The tests exercise the real HTTP surface, embedded PDP, VedaFlow Knowledge
//! commands, database worker lease, forced-RLS store and hash-chain audit. A
//! candidate is deliberately inspected before and after every transition: the
//! extraction half may propose, but only the decision half may publish.

#[path = "../../synveda-store/tests/support/tenant_fixture.rs"]
mod tenant_fixture;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::response::Json;
use axum::routing::post;
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, behavior_test_router as router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_ingest::capture_worker;
use synveda_ingest::extraction::{AnyExtractor, DeterministicExtractor, VllmExtractor};
use synveda_store::capture::{self as capture_store, NewCaptureCandidate};
use synveda_store::{access, identities, rls, scopes};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::capture::{CaptureBatch, CaptureBatchState};
use synveda_types::knowledge::{
    KnowledgeOrigin, KnowledgeRevisionContent, KnowledgeType, knowledge_revision_content_hash,
};
use synveda_types::{
    CaptureBatchId, CaptureCandidateId, Error, GrantId, IdentityId, IdentityKind, ScopeId,
    Sensitivity, TenantId, TenantStatus,
};
use tokio::sync::Notify;
use tower::ServiceExt;

#[path = "support/configuration.rs"]
mod configuration_support;

const SECRET: &[u8] = b"cpr-18-capture-api";
const ADMIN: &str = "cpr18-admin";
const MEMBER: &str = "cpr18-member";

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

fn state(url: &str) -> AppState {
    AppState {
        pool: PgPoolOptions::new()
            .max_connections(6)
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
        keys: Arc::new(synveda_store::keys::KeyRing::new(
            synveda_crypto::Kms::Disabled,
        )),
    }
}

fn issue(subject: &str, tenant_id: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue(subject, tenant_id, Duration::from_secs(300))
}

async fn seed_identity(tx: &mut sqlx::PgConnection, tenant_id: TenantId, subject: &str) -> ScopeId {
    let own = scopes::ensure_principal_scope(tx, tenant_id, subject, subject)
        .await
        .expect("mint principal scope");
    identities::create(
        tx,
        IdentityId::new(),
        tenant_id,
        Some(subject),
        IdentityKind::User,
        None,
        Some(subject),
        own.id,
    )
    .await
    .expect("create identity");
    own.id
}

async fn seed_grant(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    scope_id: ScopeId,
    subject: &str,
    role: RoleKey,
) {
    access::create_grant(
        tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id,
            scope_id,
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
    .expect("seed test policy grant");
}

/// The root administrator grant is the documented dev bootstrap. The policy
/// assignment is test configuration, not a bypass: every request below still
/// traverses the embedded PDP with that real shipped pack.
async fn admitted_tenant(pack: &str) -> Option<(AppState, TenantId)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping CPR-18 capture API test: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
            return None;
        }
    };
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::epoch::verify(&pool)
        .await
        .expect("apply migrations");
    let tenant_id = TenantId::new();
    tenant_fixture::create(
        &pool,
        tenant_id,
        &format!("cpr18-{}", tenant_id.as_uuid().simple()),
        "CPR-18 capture acceptance",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    let mut tx = rls::begin_tenant_tx(&pool, tenant_id)
        .await
        .expect("begin bootstrap transaction");
    let root = scopes::ensure_tenant_root(&mut tx, tenant_id)
        .await
        .expect("mint tenant root");
    seed_identity(&mut tx, tenant_id, ADMIN).await;
    seed_grant(&mut tx, tenant_id, root.id, ADMIN, RoleKey::Administrator).await;
    configuration_support::bind_pack(&mut tx, tenant_id, root.id, pack).await;
    tx.commit().await.expect("commit bootstrap");
    Some((state(&url), tenant_id))
}

async fn add_identity(state: &AppState, tenant_id: TenantId, subject: &str) {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin identity transaction");
    seed_identity(&mut tx, tenant_id, subject).await;
    tx.commit().await.expect("commit identity");
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    key: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
    if let Some(key) = key {
        request = request.header("idempotency-key", key);
    }
    let request = if let Some(body) = body {
        request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("build JSON request")
    } else {
        request.body(Body::empty()).expect("build request")
    };
    let response = app.clone().oneshot(request).await.expect("router responds");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect response")
        .to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| {
            panic!(
                "response should be JSON: {}",
                String::from_utf8_lossy(&bytes)
            )
        })
    };
    (status, body)
}

async fn workspace(app: &Router, token: &str, slug: &str) -> (String, String) {
    let (status, value) = call(
        app,
        "POST",
        "/v1/workspaces",
        token,
        Some(&format!("workspace-{slug}")),
        Some(json!({"slug": slug, "display_name": slug})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{value}");
    (
        value["id"].as_str().expect("workspace id").to_owned(),
        value["scope_id"]
            .as_str()
            .expect("workspace scope")
            .to_owned(),
    )
}

async fn project(app: &Router, token: &str, workspace_id: &str, slug: &str) -> (String, String) {
    let (status, value) = call(
        app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/projects"),
        token,
        Some(&format!("project-{slug}")),
        Some(json!({"slug": slug, "display_name": slug})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{value}");
    (
        value["id"].as_str().expect("project id").to_owned(),
        value["scope_id"]
            .as_str()
            .expect("project scope")
            .to_owned(),
    )
}

async fn session(
    app: &Router,
    token: &str,
    workspace_id: &str,
    project_id: &str,
    key: &str,
) -> String {
    let (status, value) = call(
        app,
        "POST",
        "/v1/sessions",
        token,
        Some(key),
        Some(json!({
            "workspace_id": workspace_id,
            "project_id": project_id,
            "client_name": "capture-acceptance",
            "client_version": "1",
            "task_summary": "Extract durable project knowledge",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{value}");
    value["id"].as_str().expect("session id").to_owned()
}

fn event(id: &str, text: &str) -> Value {
    json!({
        "event_type": "message.user",
        "client_event_id": id,
        "occurred_at": "2026-08-01T10:00:00Z",
        "payload": {"text": text},
    })
}

async fn append(app: &Router, token: &str, session_id: &str, events: Vec<Value>) -> Value {
    let (status, value) = call(
        app,
        "POST",
        &format!("/v1/sessions/{session_id}/events"),
        token,
        None,
        Some(json!({"events": events})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    value
}

async fn freeze(app: &Router, token: &str, session_id: &str, key: &str) -> (StatusCode, Value) {
    call(
        app,
        "POST",
        &format!("/v1/sessions/{session_id}/capture-batches"),
        token,
        Some(key),
        None,
    )
    .await
}

async fn run_capture(state: &AppState) {
    let summary = capture_worker::sweep_once(
        &capture_worker::Deps {
            pool: state.pool.clone(),
            pdp: Arc::clone(&state.pdp),
            extractor: Arc::new(AnyExtractor::Deterministic(DeterministicExtractor::new())),
        },
        &capture_worker::Config {
            poll_interval: Duration::from_millis(1),
            lease_duration: Duration::from_secs(30),
            batches_per_tenant: 64,
            lease_owner: format!("cpr18-test-{}", TenantId::new()),
        },
    )
    .await
    .expect("capture sweep");
    assert!(
        summary.completed > 0,
        "the sweep should complete at least this test's batch: {summary:?}"
    );
}

fn lease_test_candidate(
    batch: &CaptureBatch,
    event: &capture_store::FrozenEvent,
    id: CaptureCandidateId,
) -> NewCaptureCandidate {
    let content = KnowledgeRevisionContent {
        title: "Fenced capture output".to_owned(),
        body_markdown: "Only the current lease attempt may retain this candidate.".to_owned(),
        summary: "Fenced capture output.".to_owned(),
        tags: vec!["fencing".to_owned()],
        sensitivity: Sensitivity::WORKING,
        confidence_permille: 900,
        valid_from: event.occurred_at,
        valid_to: None,
        stale_after: None,
        verification_metadata: json!({}),
        metadata: json!({"test": "cpr-45-capture-fencing"}),
    };
    NewCaptureCandidate {
        id,
        ordinal: 1,
        proposed_scope_id: batch.scope_id,
        proposed_project_id: batch.project_id,
        proposed_owner_principal_id: None,
        knowledge_type: KnowledgeType::Fact,
        origin: KnowledgeOrigin::Observed,
        content_hash: knowledge_revision_content_hash(&content),
        content,
        source_event_ids: vec![event.id],
        matches: Vec::new(),
    }
}

fn assert_stale_claim(error: Error) {
    assert!(
        matches!(&error, Error::Conflict { message } if message.contains("current lease claim")),
        "unexpected stale-claim result: {error}"
    );
}

async fn expire_claim(state: &AppState, tenant_id: TenantId, batch_id: CaptureBatchId) {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin lease-expiry transaction");
    let changed = sqlx::query(
        "update capture_batches set lease_expires_at = statement_timestamp(), updated_at = now() \
         where tenant_id = $1 and id = $2 and state = 'running'",
    )
    .bind(tenant_id.as_uuid())
    .bind(batch_id.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("expire capture claim");
    assert_eq!(changed.rows_affected(), 1, "one live claim should expire");
    tx.commit().await.expect("commit lease expiry");
}

async fn lease_expiry(
    state: &AppState,
    tenant_id: TenantId,
    batch_id: CaptureBatchId,
) -> chrono::DateTime<chrono::Utc> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin lease inspection");
    let expiry = sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
        "select lease_expires_at from capture_batches where tenant_id = $1 and id = $2",
    )
    .bind(tenant_id.as_uuid())
    .bind(batch_id.as_uuid())
    .fetch_one(&mut *tx)
    .await
    .expect("read lease expiry")
    .expect("running batch has an expiry");
    tx.rollback().await.expect("finish lease inspection");
    expiry
}

async fn assert_candidate_absent(
    state: &AppState,
    tenant_id: TenantId,
    candidate_id: CaptureCandidateId,
) {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin candidate inspection");
    assert!(
        capture_store::get_candidate(&mut tx, tenant_id, candidate_id)
            .await
            .expect("look for fenced candidate")
            .is_none(),
        "a rejected completion must not retain candidate rows"
    );
    tx.rollback().await.expect("finish candidate inspection");
}

async fn batch_candidate_count(
    state: &AppState,
    tenant_id: TenantId,
    batch_id: CaptureBatchId,
) -> i64 {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin candidate-count inspection");
    let count = sqlx::query_scalar(
        "select count(*) from capture_candidates where tenant_id = $1 and batch_id = $2",
    )
    .bind(tenant_id.as_uuid())
    .bind(batch_id.as_uuid())
    .fetch_one(&mut *tx)
    .await
    .expect("count batch candidates");
    tx.rollback()
        .await
        .expect("finish candidate-count inspection");
    count
}

async fn completion_audit_count(
    state: &AppState,
    tenant_id: TenantId,
    batch_id: CaptureBatchId,
) -> usize {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin completion-audit inspection");
    let audit = synveda_audit::tail(&mut tx, tenant_id, 1_000)
        .await
        .expect("read capture audit");
    tx.rollback()
        .await
        .expect("finish completion-audit inspection");
    audit
        .iter()
        .filter(|event| {
            event.action.as_str() == "capture.batch.completed"
                && event.payload["batch_id"] == batch_id.to_string()
        })
        .count()
}

async fn reclaim_expired_claim(
    state: &AppState,
    tenant_id: TenantId,
    batch_id: CaptureBatchId,
    lease_owner: &str,
) -> CaptureBatch {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin atomic expiry and reclaim");
    let changed = sqlx::query(
        "update capture_batches set lease_expires_at = statement_timestamp(), updated_at = now() \
         where tenant_id = $1 and id = $2 and state = 'running'",
    )
    .bind(tenant_id.as_uuid())
    .bind(batch_id.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("expire claimed batch");
    assert_eq!(changed.rows_affected(), 1, "one claim should be stolen");
    let reclaimed = capture_store::claim_batch(&mut tx, tenant_id, lease_owner, 30)
        .await
        .expect("reclaim expired batch")
        .expect("expired batch is claimable");
    assert_eq!(reclaimed.id, batch_id, "the intended batch is reclaimed");
    tx.commit().await.expect("commit authoritative reclaim");
    reclaimed
}

async fn wait_for_lease_extension(
    state: &AppState,
    tenant_id: TenantId,
    batch_id: CaptureBatchId,
    initial: chrono::DateTime<chrono::Utc>,
) -> chrono::DateTime<chrono::Utc> {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let current = lease_expiry(state, tenant_id, batch_id).await;
            if current > initial {
                return current;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("worker renews while the extractor is blocked")
}

async fn named_worker_pool(application_name: &str) -> sqlx::PgPool {
    let url = std::env::var("DATABASE_URL").expect("database-backed test has DATABASE_URL");
    let application_name = application_name.to_owned();
    PgPoolOptions::new()
        .max_connections(3)
        .after_connect(move |connection, _| {
            let application_name = application_name.clone();
            Box::pin(async move {
                sqlx::query_scalar::<_, String>("select set_config('application_name', $1, false)")
                    .bind(application_name)
                    .fetch_one(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .expect("connect named worker pool")
}

async fn wait_for_database_lock(state: &AppState, tenant_id: TenantId, application_name: &str) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
                .await
                .expect("begin worker-state inspection");
            let waiting: bool = sqlx::query_scalar(
                "select exists (select 1 from pg_stat_activity \
                 where application_name = $1 and wait_event_type = 'Lock')",
            )
            .bind(application_name)
            .fetch_one(&mut *tx)
            .await
            .expect("inspect worker wait state");
            tx.rollback().await.expect("finish worker-state inspection");
            if waiting {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("worker reaches the deliberately blocked preflight query");
}

#[derive(Clone)]
struct BlockingVllmState {
    entered: Arc<Notify>,
    release: Arc<Notify>,
    calls: Arc<AtomicUsize>,
}

struct BlockingVllm {
    base_url: String,
    entered: Arc<Notify>,
    release: Arc<Notify>,
    calls: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for BlockingVllm {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn blocked_vllm_response(State(state): State<BlockingVllmState>) -> Json<Value> {
    state.calls.fetch_add(1, Ordering::SeqCst);
    state.entered.notify_one();
    state.release.notified().await;
    Json(json!({
        "model": "cpr45-blocking@1",
        "choices": [{
            "message": {
                "content": "{\"candidates\":[{\"knowledge_type\":\"fact\",\"title\":\"Renewed capture\",\"body_markdown\":\"The worker retained its exact lease while extraction was blocked.\",\"summary\":\"The capture lease was renewed.\",\"tags\":[\"lease\"],\"confidence\":0.9}] }"
            }
        }]
    }))
}

async fn spawn_blocking_vllm() -> BlockingVllm {
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind blocking vLLM fixture");
    let address = listener.local_addr().expect("read vLLM fixture address");
    let app = Router::new()
        .route("/v1/chat/completions", post(blocked_vllm_response))
        .with_state(BlockingVllmState {
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
            calls: Arc::clone(&calls),
        });
    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve blocking vLLM fixture");
    });
    BlockingVllm {
        base_url: format!("http://{address}"),
        entered,
        release,
        calls,
        task,
    }
}

#[tokio::test]
async fn capture_leases_renew_fence_stale_results_and_terminalise_exhaustion() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant(synveda_policy::STANDARD).await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, _) = workspace(&app, &token, "lease-fence").await;
    let (project_id, _) = project(&app, &token, &workspace_id, "lease-fence").await;
    let session_id = session(
        &app,
        &token,
        &workspace_id,
        &project_id,
        "lease-fence-session",
    )
    .await;
    append(
        &app,
        &token,
        &session_id,
        vec![event(
            "lease-fence-one",
            "The portable worker must fence stale model output.",
        )],
    )
    .await;
    let (status, frozen) = freeze(&app, &token, &session_id, "lease-fence-one").await;
    assert_eq!(status, StatusCode::CREATED, "{frozen}");
    let batch_id = frozen["id"]
        .as_str()
        .expect("capture batch id")
        .parse::<CaptureBatchId>()
        .expect("parse capture batch id");
    let owner = format!("cpr45-fence-{tenant_id}");

    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin first claim");
    let first = capture_store::claim_batch(&mut tx, tenant_id, &owner, 30)
        .await
        .expect("claim first attempt")
        .expect("pending capture batch");
    let events = capture_store::frozen_events(&mut *tx, tenant_id, first.id)
        .await
        .expect("load frozen evidence");
    tx.commit().await.expect("commit first claim");
    assert_eq!(first.attempts, 1);
    assert_eq!(events.len(), 1);
    let candidate_id = CaptureCandidateId::new();
    let candidate = lease_test_candidate(&first, &events[0], candidate_id);

    let before_renewal = lease_expiry(&state, tenant_id, batch_id).await;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin renewal");
    capture_store::renew_batch(&mut tx, &first, &owner, 120)
        .await
        .expect("renew first claim");
    tx.commit().await.expect("commit renewal");
    let after_renewal = lease_expiry(&state, tenant_id, batch_id).await;
    assert!(after_renewal > before_renewal);

    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin competing claim");
    assert!(
        capture_store::claim_batch(&mut tx, tenant_id, "another-owner", 30)
            .await
            .expect("look for competing work")
            .is_none(),
        "a live claim must not be reclaimed"
    );
    tx.rollback().await.expect("roll back empty claim");

    for (forged, forged_owner) in [
        (first.clone(), "wrong-owner"),
        (
            CaptureBatch {
                attempts: first.attempts + 1,
                ..first.clone()
            },
            owner.as_str(),
        ),
    ] {
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
            .await
            .expect("begin forged renewal");
        let error = capture_store::renew_batch(&mut tx, &forged, forged_owner, 120)
            .await
            .expect_err("forged claim must not renew");
        assert_stale_claim(error);
        tx.commit()
            .await
            .expect("a rejected renewal has no mutation to roll back");
    }

    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin wrong-owner failure");
    let error = capture_store::fail_batch(&mut tx, &first, "wrong-owner", "forged-owner")
        .await
        .expect_err("wrong owner must not fail a live claim");
    assert_stale_claim(error);
    tx.commit()
        .await
        .expect("rejected wrong-owner failure has no mutation");

    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin wrong-owner completion");
    let error = capture_store::complete_batch(
        &mut tx,
        &first,
        "wrong-owner",
        "fence-test",
        "fence-test@1",
        std::slice::from_ref(&candidate),
    )
    .await
    .expect_err("wrong owner must not complete a live claim");
    assert_stale_claim(error);
    tx.commit()
        .await
        .expect("rejected wrong-owner completion has no mutation");
    assert_candidate_absent(&state, tenant_id, candidate_id).await;

    // Establish an old transaction timestamp, then expire from a newer
    // transaction. A lease predicate using PostgreSQL `now()` would accept
    // this stale attempt; statement time must reject it.
    let mut old_tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin old-clock transaction");
    let old_clock: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("select transaction_timestamp()")
            .fetch_one(&mut *old_tx)
            .await
            .expect("read old transaction clock");
    expire_claim(&state, tenant_id, batch_id).await;
    let expired_at = lease_expiry(&state, tenant_id, batch_id).await;
    assert!(
        expired_at > old_clock,
        "test must distinguish the two clocks"
    );
    let mut expired_renewal = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin expired renewal");
    let error = capture_store::renew_batch(&mut expired_renewal, &first, &owner, 120)
        .await
        .expect_err("an expired claim must not renew");
    assert_stale_claim(error);
    expired_renewal
        .commit()
        .await
        .expect("rejected expired renewal has no mutation");
    let error = capture_store::fail_batch(&mut old_tx, &first, &owner, "stale-clock")
        .await
        .expect_err("transaction-start time must not revive an expired claim");
    assert_stale_claim(error);
    old_tx
        .commit()
        .await
        .expect("rejected stale failure commits no mutation");

    let mut stale_tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin stale completion");
    let error = capture_store::complete_batch(
        &mut stale_tx,
        &first,
        &owner,
        "fence-test",
        "fence-test@1",
        std::slice::from_ref(&candidate),
    )
    .await
    .expect_err("expired completion must be fenced");
    assert_stale_claim(error);
    stale_tx
        .commit()
        .await
        .expect("caught completion conflict must remain side-effect free");
    assert_candidate_absent(&state, tenant_id, candidate_id).await;

    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin same-owner reclaim");
    let second = capture_store::claim_batch(&mut tx, tenant_id, &owner, 30)
        .await
        .expect("reclaim expired batch")
        .expect("expired batch available");
    tx.commit().await.expect("commit second claim");
    assert_eq!(second.attempts, 2);

    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin stale same-owner failure");
    let error = capture_store::fail_batch(&mut tx, &first, &owner, "stale-attempt")
        .await
        .expect_err("same owner cannot revive the old attempt");
    assert_stale_claim(error);
    tx.commit()
        .await
        .expect("stale same-owner failure has no mutation");

    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin stale same-owner completion");
    let error = capture_store::complete_batch(
        &mut tx,
        &first,
        &owner,
        "fence-test",
        "fence-test@1",
        std::slice::from_ref(&candidate),
    )
    .await
    .expect_err("same owner cannot complete the reclaimed attempt");
    assert_stale_claim(error);
    tx.commit()
        .await
        .expect("stale same-owner completion has no mutation");
    assert_candidate_absent(&state, tenant_id, candidate_id).await;

    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin winning completion");
    let completed = capture_store::complete_batch(
        &mut tx,
        &second,
        &owner,
        "fence-test",
        "fence-test@1",
        std::slice::from_ref(&candidate),
    )
    .await
    .expect("current claim completes");
    tx.commit().await.expect("commit winning completion");
    assert_eq!(completed.state, CaptureBatchState::Completed);
    assert_eq!(completed.attempts, 2);
    assert_eq!(completed.candidate_count, 1);

    let mut replay = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin completion replay");
    let error = capture_store::complete_batch(
        &mut replay,
        &second,
        &owner,
        "fence-test",
        "fence-test@1",
        std::slice::from_ref(&candidate),
    )
    .await
    .expect_err("completed attempt cannot replay");
    assert_stale_claim(error);
    replay
        .commit()
        .await
        .expect("completion replay remains side-effect free");
    let candidate_count = batch_candidate_count(&state, tenant_id, batch_id).await;
    assert_eq!(candidate_count, 1);

    // Freeze a changed snapshot, exhaust four ordinary retries, then crash
    // the fifth claim. The next worker pass must terminalise it with audit
    // evidence instead of leaving an unclaimable running row forever.
    append(
        &app,
        &token,
        &session_id,
        vec![event(
            "lease-fence-two",
            "A final expired attempt must become an inspectable failure.",
        )],
    )
    .await;
    let (status, exhausted) = freeze(&app, &token, &session_id, "lease-fence-two").await;
    assert_eq!(status, StatusCode::CREATED, "{exhausted}");
    let exhausted_id = exhausted["id"]
        .as_str()
        .expect("exhausted batch id")
        .parse::<CaptureBatchId>()
        .expect("parse exhausted batch id");
    for expected_attempt in 1..=synveda_types::capture::MAX_CAPTURE_ATTEMPTS {
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
            .await
            .expect("begin bounded retry claim");
        let attempt = capture_store::claim_batch(&mut tx, tenant_id, &owner, 30)
            .await
            .expect("claim bounded retry")
            .expect("retry batch remains claimable");
        assert_eq!(attempt.id, exhausted_id);
        assert_eq!(attempt.attempts, expected_attempt);
        if expected_attempt < synveda_types::capture::MAX_CAPTURE_ATTEMPTS {
            capture_store::fail_batch(&mut tx, &attempt, &owner, "test-retry")
                .await
                .expect("release bounded retry");
        }
        tx.commit().await.expect("commit bounded retry transition");
    }
    expire_claim(&state, tenant_id, exhausted_id).await;
    let summary = capture_worker::sweep_once(
        &capture_worker::Deps {
            pool: state.pool.clone(),
            pdp: Arc::clone(&state.pdp),
            extractor: Arc::new(AnyExtractor::Deterministic(DeterministicExtractor::new())),
        },
        &capture_worker::Config {
            poll_interval: Duration::from_secs(1),
            lease_duration: Duration::from_secs(30),
            batches_per_tenant: 1,
            lease_owner: owner,
        },
    )
    .await
    .expect("terminalise exhausted claim");
    assert!(summary.failed_attempts >= 1, "{summary:?}");
    let mut inspect = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("inspect exhausted claim");
    let exhausted = capture_store::get_batch(&mut *inspect, tenant_id, exhausted_id)
        .await
        .expect("load exhausted batch")
        .expect("exhausted batch remains inspectable");
    let audit = synveda_audit::tail(&mut inspect, tenant_id, 100)
        .await
        .expect("read terminal failure audit");
    inspect.commit().await.expect("finish exhausted inspection");
    assert_eq!(exhausted.state, CaptureBatchState::Failed);
    assert_eq!(exhausted.error_code.as_deref(), Some("lease_expired"));
    assert!(audit.iter().any(|event| {
        event.action.as_str() == "capture.batch.completed"
            && event.payload["batch_id"] == exhausted_id.to_string()
            && event.payload["state"] == "failed"
            && event.payload["error_code"] == "lease_expired"
    }));
}

#[tokio::test]
async fn capture_worker_renews_blocked_extraction_and_discards_a_reclaimed_result() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant(synveda_policy::STANDARD).await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, _) = workspace(&app, &token, "worker-renewal").await;
    let (project_id, _) = project(&app, &token, &workspace_id, "worker-renewal").await;
    let session_id = session(
        &app,
        &token,
        &workspace_id,
        &project_id,
        "worker-renewal-session",
    )
    .await;
    append(
        &app,
        &token,
        &session_id,
        vec![event(
            "worker-renewal-one",
            "A blocked model call must not outlive its database authority.",
        )],
    )
    .await;
    let (status, frozen) = freeze(&app, &token, &session_id, "worker-renewal-one").await;
    assert_eq!(status, StatusCode::CREATED, "{frozen}");
    let renewed_batch_id = frozen["id"]
        .as_str()
        .expect("renewed batch id")
        .parse::<CaptureBatchId>()
        .expect("parse renewed batch id");

    let blocked = spawn_blocking_vllm().await;
    let deps = capture_worker::Deps {
        pool: state.pool.clone(),
        pdp: Arc::clone(&state.pdp),
        extractor: Arc::new(AnyExtractor::Vllm(
            VllmExtractor::new("cpr45-blocking@1".to_owned(), blocked.base_url.clone())
                .expect("configure blocking extractor"),
        )),
    };
    let config = capture_worker::Config {
        poll_interval: Duration::from_secs(1),
        lease_duration: Duration::from_secs(1),
        batches_per_tenant: 1,
        lease_owner: format!("cpr45-renew-{}", CaptureBatchId::new()),
    };
    let sweep = tokio::spawn(async move { capture_worker::sweep_once(&deps, &config).await });
    tokio::time::timeout(Duration::from_secs(3), blocked.entered.notified())
        .await
        .expect("extractor receives the frozen event");
    let initial_expiry = lease_expiry(&state, tenant_id, renewed_batch_id).await;
    let extended_expiry =
        wait_for_lease_extension(&state, tenant_id, renewed_batch_id, initial_expiry).await;
    assert!(extended_expiry > initial_expiry);
    blocked.release.notify_one();
    let summary = tokio::time::timeout(Duration::from_secs(3), sweep)
        .await
        .expect("renewed extraction finishes within the test bound")
        .expect("renewed worker task joins")
        .expect("renewed worker sweep succeeds");
    assert!(summary.completed >= 1, "{summary:?}");
    assert_eq!(blocked.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        batch_candidate_count(&state, tenant_id, renewed_batch_id).await,
        1
    );
    assert_eq!(
        completion_audit_count(&state, tenant_id, renewed_batch_id).await,
        1
    );

    append(
        &app,
        &token,
        &session_id,
        vec![event(
            "worker-renewal-two",
            "A reclaimed attempt must discard dependency output.",
        )],
    )
    .await;
    let (status, frozen) = freeze(&app, &token, &session_id, "worker-renewal-two").await;
    assert_eq!(status, StatusCode::CREATED, "{frozen}");
    let reclaimed_batch_id = frozen["id"]
        .as_str()
        .expect("reclaimed batch id")
        .parse::<CaptureBatchId>()
        .expect("parse reclaimed batch id");

    let lost = spawn_blocking_vllm().await;
    let loser_owner = format!("cpr45-loser-{}", CaptureBatchId::new());
    let deps = capture_worker::Deps {
        pool: state.pool.clone(),
        pdp: Arc::clone(&state.pdp),
        extractor: Arc::new(AnyExtractor::Vllm(
            VllmExtractor::new("cpr45-blocking@1".to_owned(), lost.base_url.clone())
                .expect("configure blocking extractor"),
        )),
    };
    let config = capture_worker::Config {
        poll_interval: Duration::from_secs(1),
        lease_duration: Duration::from_secs(1),
        batches_per_tenant: 1,
        lease_owner: loser_owner,
    };
    let sweep = tokio::spawn(async move { capture_worker::sweep_once(&deps, &config).await });
    tokio::time::timeout(Duration::from_secs(3), lost.entered.notified())
        .await
        .expect("losing extractor receives the frozen event");
    let winner_owner = format!("cpr45-winner-{}", CaptureBatchId::new());
    let winner = reclaim_expired_claim(&state, tenant_id, reclaimed_batch_id, &winner_owner).await;
    assert_eq!(winner.attempts, 2);
    let summary = tokio::time::timeout(Duration::from_secs(3), sweep)
        .await
        .expect("losing extraction observes the failed renewal")
        .expect("losing worker task joins")
        .expect("losing worker sweep returns an outcome");
    lost.release.notify_waiters();
    assert!(summary.abandoned_attempts >= 1, "{summary:?}");
    assert_eq!(lost.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        batch_candidate_count(&state, tenant_id, reclaimed_batch_id).await,
        0,
        "the stale provider response must not materialise a candidate"
    );
    assert_eq!(
        completion_audit_count(&state, tenant_id, reclaimed_batch_id).await,
        0,
        "the losing attempt must not append terminal audit evidence"
    );

    // Return the authoritative claim to the ordinary worker path so this
    // acceptance test leaves no live or pending work behind.
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin winner cleanup");
    capture_store::fail_batch(&mut tx, &winner, &winner_owner, "test-requeue")
        .await
        .expect("release the winner for ordinary processing");
    tx.commit().await.expect("commit winner cleanup");
    run_capture(&state).await;
}

#[tokio::test]
#[ignore = "serial migrator lock acceptance"]
async fn capture_worker_reproves_a_preflight_lease_before_calling_the_extractor() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant(synveda_policy::STANDARD).await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, _) = workspace(&app, &token, "preflight-expiry").await;
    let (project_id, _) = project(&app, &token, &workspace_id, "preflight-expiry").await;
    let session_id = session(
        &app,
        &token,
        &workspace_id,
        &project_id,
        "preflight-expiry-session",
    )
    .await;
    append(
        &app,
        &token,
        &session_id,
        vec![event(
            "preflight-expiry-one",
            "Expired preflight authority must prevent provider disclosure.",
        )],
    )
    .await;
    let (status, frozen) = freeze(&app, &token, &session_id, "preflight-expiry-one").await;
    assert_eq!(status, StatusCode::CREATED, "{frozen}");
    let batch_id = frozen["id"]
        .as_str()
        .expect("preflight batch id")
        .parse::<CaptureBatchId>()
        .expect("parse preflight batch id");

    // AccessExclusive is a serial test synchronisation barrier, not worker
    // authority. PostgreSQL 17 correctly withholds that lock from the gateway,
    // so the barrier uses the separately verified migrator/database owner;
    // the worker still claims and reads through its ordinary tenant role.
    let barrier_pool = tenant_fixture::migrator_pool(&state.pool).await;
    let mut blocker = barrier_pool.begin().await.expect("begin preflight barrier");
    sqlx::query("lock table configuration_versions in access exclusive mode")
        .execute(&mut *blocker)
        .await
        .expect("lock frozen configuration reads");

    let fixture = spawn_blocking_vllm().await;
    fixture.release.notify_one();
    let application_name = format!("cpr45-preflight-{}", CaptureBatchId::new());
    let worker_pool = named_worker_pool(&application_name).await;
    let deps = capture_worker::Deps {
        pool: worker_pool,
        pdp: Arc::clone(&state.pdp),
        extractor: Arc::new(AnyExtractor::Vllm(
            VllmExtractor::new("cpr45-blocking@1".to_owned(), fixture.base_url.clone())
                .expect("configure blocking extractor"),
        )),
    };
    let config = capture_worker::Config {
        poll_interval: Duration::from_secs(1),
        lease_duration: Duration::from_secs(1),
        batches_per_tenant: 1,
        lease_owner: format!("cpr45-preflight-{}", CaptureBatchId::new()),
    };
    let sweep = tokio::spawn(async move { capture_worker::sweep_once(&deps, &config).await });
    wait_for_database_lock(&state, tenant_id, &application_name).await;
    tokio::time::sleep(Duration::from_millis(1_250)).await;
    blocker.rollback().await.expect("release preflight barrier");
    barrier_pool.close().await;

    let summary = tokio::time::timeout(Duration::from_secs(3), sweep)
        .await
        .expect("expired preflight returns within the test bound")
        .expect("preflight worker task joins")
        .expect("preflight worker reports abandonment");
    assert!(summary.abandoned_attempts >= 1, "{summary:?}");
    assert_eq!(
        fixture.calls.load(Ordering::SeqCst),
        0,
        "no frozen content reaches the provider after preflight expiry"
    );
    assert_eq!(batch_candidate_count(&state, tenant_id, batch_id).await, 0);
    assert_eq!(completion_audit_count(&state, tenant_id, batch_id).await, 0);

    let winner_owner = format!("cpr45-preflight-winner-{}", CaptureBatchId::new());
    let winner = reclaim_expired_claim(&state, tenant_id, batch_id, &winner_owner).await;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin preflight cleanup");
    capture_store::fail_batch(&mut tx, &winner, &winner_owner, "test-requeue")
        .await
        .expect("release preflight winner for ordinary processing");
    tx.commit().await.expect("commit preflight cleanup");
    run_capture(&state).await;
}

async fn candidates(app: &Router, token: &str, batch_id: &str) -> Vec<Value> {
    let (status, value) = call(
        app,
        "GET",
        &format!("/v1/capture-candidates?batch_id={batch_id}&limit=200"),
        token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let mut values = value["candidates"]
        .as_array()
        .expect("candidate page")
        .clone();
    values.sort_by_key(|candidate| candidate["ordinal"].as_i64());
    values
}

async fn decide(
    app: &Router,
    token: &str,
    candidate_id: &str,
    action: &str,
    key: &str,
    body: Value,
) -> (StatusCode, Value) {
    call(
        app,
        "POST",
        &format!("/v1/capture-candidates/{candidate_id}/{action}"),
        token,
        Some(key),
        Some(body),
    )
    .await
}

fn revised_content(title: &str, body: &str) -> Value {
    json!({
        "title": title,
        "body_markdown": body,
        "summary": body,
        "tags": ["accepted", "capture"],
        "sensitivity": "internal",
        "confidence_permille": 950,
        "verification_metadata": {"reviewed": true},
        "metadata": {},
    })
}

async fn compose_context(
    app: &Router,
    token: &str,
    session_id: &str,
    key: &str,
    query: &str,
) -> Value {
    let (status, value) = call(
        app,
        "POST",
        &format!("/v1/sessions/{session_id}/context-runs"),
        token,
        Some(key),
        Some(json!({"query": query, "budget_tokens": 512})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{value}");
    value
}

async fn context_detail(app: &Router, token: &str, run_id: &str) -> Value {
    let (status, value) = call(
        app,
        "GET",
        &format!("/v1/context-runs/{run_id}"),
        token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    value
}

#[tokio::test]
async fn candidates_are_reviewable_only_and_every_decision_uses_vedaflow() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant(synveda_policy::STANDARD).await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, _) = workspace(&app, &token, "pulse-capture").await;
    let (project_id, _) = project(&app, &token, &workspace_id, "api").await;
    let session_id = session(&app, &token, &workspace_id, &project_id, "capture-session").await;
    let phrases = [
        "CAPTURE-ALPHA Webhook deliveries are deduplicated by provider event ID.",
        "CAPTURE-BETA Public requests use X-Request-Id.",
        "CAPTURE-GAMMA The release procedure runs test-fast before publishing.",
        "CAPTURE-DELTA The cache policy is reviewed every Friday.",
        "CAPTURE-EPSILON Incidental lunch detail should be dismissed.",
        "CAPTURE-ZETA The incident warning requires a rollback marker.",
    ];
    let appended = append(
        &app,
        &token,
        &session_id,
        phrases
            .iter()
            .enumerate()
            .map(|(index, text)| event(&format!("capture-{index}"), text))
            .collect(),
    )
    .await;
    let first_event_id = appended["events"][0]["event"]["id"]
        .as_str()
        .expect("event id")
        .to_owned();

    let (created, batch) = freeze(&app, &token, &session_id, "freeze-one").await;
    assert_eq!(created, StatusCode::CREATED, "{batch}");
    let batch_id = batch["id"].as_str().expect("batch id").to_owned();
    assert_eq!(batch["event_count"], 6);
    let (replayed, same_key) = freeze(&app, &token, &session_id, "freeze-one").await;
    assert_eq!(replayed, StatusCode::OK, "{same_key}");
    assert_eq!(same_key["id"], batch_id);
    let (same_snapshot, other_key) = freeze(&app, &token, &session_id, "freeze-two").await;
    assert_eq!(same_snapshot, StatusCode::OK, "{other_key}");
    assert_eq!(other_key["id"], batch_id);

    let mut tx = tenant_fixture::begin(&state.pool, tenant_id).await;
    let before_knowledge: i64 =
        sqlx::query_scalar("select count(*) from knowledge_items where tenant_id = $1")
            .bind(tenant_id.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .expect("count Knowledge before extraction");
    tx.commit().await.expect("commit pre-extraction read");
    assert_eq!(before_knowledge, 0);
    run_capture(&state).await;

    let mut tx = tenant_fixture::begin(&state.pool, tenant_id).await;
    let after_extraction: i64 =
        sqlx::query_scalar("select count(*) from knowledge_items where tenant_id = $1")
            .bind(tenant_id.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .expect("count Knowledge after extraction");
    tx.commit().await.expect("commit post-extraction read");
    let retired_table: Option<String> =
        sqlx::query_scalar("select to_regclass('public.records')::text")
            .fetch_one(&state.pool)
            .await
            .expect("look for the retired Record table");
    assert_eq!(after_extraction, 0, "extraction may only create candidates");
    assert!(retired_table.is_none(), "the retired Record table returned");

    let values = candidates(&app, &token, &batch_id).await;
    assert_eq!(values.len(), phrases.len());
    for (candidate, phrase) in values.iter().zip(phrases) {
        assert_eq!(candidate["state"], "pending");
        assert_eq!(candidate["content"]["body_markdown"], phrase);
        assert_eq!(
            candidate["source_event_ids"].as_array().map(Vec::len),
            Some(1)
        );
        assert!(candidate["resulting_change_id"].is_null());
    }

    // Owner-level immutability protects the proposal itself, not merely the
    // API DTO. A review records edits on the resulting Knowledge revision.
    let immutable_id = values[0]["id"].as_str().expect("candidate id");
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin immutable-write probe");
    let immutable = sqlx::query(
        "update capture_candidates set title = 'tampered' where tenant_id = $1 and id = $2",
    )
    .bind(tenant_id.as_uuid())
    .bind(uuid::Uuid::parse_str(immutable_id).expect("candidate uuid"))
    .execute(&mut *tx)
    .await
    .expect_err("candidate proposal must be immutable");
    assert!(immutable.to_string().contains("immutable"), "{immutable}");
    tx.rollback().await.expect("roll back rejected mutation");

    let id0 = values[0]["id"].as_str().expect("candidate 0");
    let (status, accepted) = decide(&app, &token, id0, "accept", "accept-alpha", json!({})).await;
    assert_eq!(status, StatusCode::CREATED, "{accepted}");
    assert_eq!(accepted["candidate"]["state"], "accepted");
    assert_eq!(accepted["candidate"]["resulting_outcome"], "applied");
    let item0 = accepted["candidate"]["resulting_knowledge_item_id"]
        .as_str()
        .expect("accepted item")
        .to_owned();
    let revision0 = accepted["candidate"]["resulting_revision_id"]
        .as_str()
        .expect("accepted revision")
        .to_owned();
    let (status, retry) = decide(&app, &token, id0, "accept", "accept-alpha", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{retry}");
    assert_eq!(retry["replayed"], true);
    assert_eq!(retry["candidate"]["resulting_knowledge_item_id"], item0);
    let (status, changed_retry) = decide(
        &app,
        &token,
        id0,
        "accept",
        "accept-alpha",
        json!({"knowledge_type": "warning"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{changed_retry}");

    let id1 = values[1]["id"].as_str().expect("candidate 1");
    let (status, edited) = decide(
        &app,
        &token,
        id1,
        "accept",
        "edit-beta",
        json!({"content": revised_content(
            "Request correlation convention",
            "CAPTURE-BETA-EDITED Public requests use traceparent.",
        )}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{edited}");
    assert_eq!(edited["candidate"]["state"], "edited_and_accepted");
    let item1 = edited["candidate"]["resulting_knowledge_item_id"]
        .as_str()
        .expect("edited item")
        .to_owned();
    let revision1 = edited["candidate"]["resulting_revision_id"]
        .as_str()
        .expect("edited revision")
        .to_owned();

    let id2 = values[2]["id"].as_str().expect("candidate 2");
    let (status, merged) = decide(
        &app,
        &token,
        id2,
        "merge",
        "merge-gamma",
        json!({"inputs": [{"item_id": item0, "revision_id": revision0}]}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{merged}");
    assert_eq!(merged["candidate"]["state"], "merged");
    assert_eq!(merged["candidate"]["resulting_outcome"], "applied");

    let id3 = values[3]["id"].as_str().expect("candidate 3");
    let (status, replaced) = decide(
        &app,
        &token,
        id3,
        "replace",
        "replace-delta",
        json!({"item_id": item1, "expected_revision_id": revision1}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{replaced}");
    assert_eq!(replaced["candidate"]["state"], "replaced");
    assert_eq!(replaced["candidate"]["resulting_outcome"], "applied");

    let id4 = values[4]["id"].as_str().expect("candidate 4");
    let (status, dismissed) = decide(
        &app,
        &token,
        id4,
        "dismiss",
        "dismiss-epsilon",
        json!({"reason": "incidental detail"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{dismissed}");
    assert_eq!(dismissed["candidate"]["state"], "dismissed");
    assert!(dismissed["candidate"]["resulting_change_id"].is_null());

    // The batch command sees five terminal candidates, accepts the one still
    // pending, and records a parent key only after all children finish.
    let (status, batch_accept) = call(
        &app,
        "POST",
        &format!("/v1/capture-batches/{batch_id}/accept"),
        &token,
        Some("accept-whole-batch"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{batch_accept}");
    assert_eq!(batch_accept["candidates"].as_array().map(Vec::len), Some(6));
    let accepted_last = batch_accept["candidates"]
        .as_array()
        .expect("batch candidates")
        .iter()
        .find(|candidate| candidate["id"] == values[5]["id"])
        .expect("last candidate");
    assert_eq!(accepted_last["state"], "accepted");
    let last_item = accepted_last["resulting_knowledge_item_id"]
        .as_str()
        .expect("last item")
        .to_owned();
    let last_revision = accepted_last["resulting_revision_id"]
        .as_str()
        .expect("last revision")
        .to_owned();
    let (status, batch_replay) = call(
        &app,
        "POST",
        &format!("/v1/capture-batches/{batch_id}/accept"),
        &token,
        Some("accept-whole-batch"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{batch_replay}");

    let (status, sources) = call(
        &app,
        "GET",
        &format!("/v1/knowledge/{last_item}/sources"),
        &token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sources}");
    assert_eq!(sources["sources"][0]["source_type"], "session_event");
    assert_eq!(
        sources["sources"][0]["session_event_id"],
        values[5]["source_event_ids"][0]
    );

    // Governed forget scrubs both the resulting Knowledge plaintext and the
    // retained candidate/decision copies while leaving their identifiers.
    let (status, forgotten) = call(
        &app,
        "DELETE",
        &format!("/v1/knowledge/{last_item}"),
        &token,
        Some("forget-captured-item"),
        Some(json!({
            "mode": "forget",
            "expected_revision_id": last_revision,
            "reason": "CPR-18 erasure acceptance",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{forgotten}");
    assert_eq!(forgotten["outcome"], "applied");
    let mut tx = tenant_fixture::begin(&state.pool, tenant_id).await;
    let scrubbed: (String, String, bool, Option<Value>) = sqlx::query_as(
        "select candidate.title, candidate.body_markdown, candidate.content_erased, decision.payload \
         from capture_candidates candidate \
         join capture_candidate_decisions decision \
           on decision.tenant_id = candidate.tenant_id and decision.candidate_id = candidate.id \
         where candidate.tenant_id = $1 and candidate.id = $2",
    )
    .bind(tenant_id.as_uuid())
    .bind(uuid::Uuid::parse_str(values[5]["id"].as_str().expect("candidate id")).expect("uuid"))
    .fetch_one(&mut *tx)
    .await
    .expect("read scrubbed candidate");
    assert_eq!(scrubbed, (String::new(), String::new(), true, None));

    let capture_audit: String = sqlx::query_scalar(
        "select coalesce(string_agg(payload::text, ' '), '') from audit_log \
         where tenant_id = $1 and action like 'capture.%'",
    )
    .bind(tenant_id.as_uuid())
    .fetch_one(&mut *tx)
    .await
    .expect("read capture audit payloads");
    for plaintext in phrases
        .into_iter()
        .chain(["CAPTURE-BETA-EDITED", "incidental detail"])
    {
        assert!(
            !capture_audit.contains(plaintext),
            "capture audit leaked candidate content {plaintext:?}: {capture_audit}"
        );
    }
    let decisions: i64 = sqlx::query_scalar(
        "select count(*) from audit_log where tenant_id = $1 and action = 'capture.candidate.decided'",
    )
    .bind(tenant_id.as_uuid())
    .fetch_one(&mut *tx)
    .await
    .expect("count decision audit events");
    tx.commit()
        .await
        .expect("commit Capture erasure evidence read");
    assert_eq!(decisions, 6, "retries must not duplicate decision audits");

    // A changed event set is a new snapshot. Session end freezes it without
    // waiting for the model; an explicit retry then resolves to that address.
    append(
        &app,
        &token,
        &session_id,
        vec![event(
            "capture-late",
            "CAPTURE-ETA The final session detail is eligible at close.",
        )],
    )
    .await;
    let (status, ended) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{session_id}/end"),
        &token,
        None,
        Some(json!({"status": "ended"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ended}");
    let (status, batches) = call(
        &app,
        "GET",
        &format!("/v1/capture-batches?session_id={session_id}&limit=20"),
        &token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{batches}");
    assert_eq!(batches["batches"].as_array().map(Vec::len), Some(2));
    let newest = batches["batches"]
        .as_array()
        .expect("batch page")
        .iter()
        .find(|entry| entry["id"] != batch_id)
        .expect("session-end snapshot");
    assert_eq!(newest["state"], "pending");
    assert_eq!(newest["event_count"], 7);
    let (status, close_replay) = freeze(&app, &token, &session_id, "close-replay").await;
    assert_eq!(status, StatusCode::OK, "{close_replay}");
    assert_eq!(close_replay["id"], newest["id"]);

    // A source from a different session cannot be attached to this batch's
    // candidate even by a direct application-role write.
    let other_session = session(&app, &token, &workspace_id, &project_id, "other-session").await;
    let other = append(
        &app,
        &token,
        &other_session,
        vec![event("foreign-source", "A source from another run")],
    )
    .await;
    let foreign_event = other["events"][0]["event"]["id"]
        .as_str()
        .expect("foreign event");
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin forged-source probe");
    let forged = sqlx::query(
        "insert into capture_candidate_events \
         (tenant_id, candidate_id, batch_id, event_id, ordinal) values ($1, $2, $3, $4, 99)",
    )
    .bind(tenant_id.as_uuid())
    .bind(uuid::Uuid::parse_str(id0).expect("candidate uuid"))
    .bind(uuid::Uuid::parse_str(&batch_id).expect("batch uuid"))
    .bind(uuid::Uuid::parse_str(foreign_event).expect("event uuid"))
    .execute(&mut *tx)
    .await
    .expect_err("cross-session evidence must be refused");
    assert_eq!(
        forged
            .as_database_error()
            .and_then(|error| error.constraint()),
        Some("capture_candidate_events_frozen_event_fk")
    );
    tx.rollback().await.expect("roll back forged source");
    assert_ne!(first_event_id, foreign_event);
}

#[tokio::test]
async fn strict_profile_retains_a_pending_review_instead_of_publishing() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant(synveda_policy::REGULATED_STRICT).await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, _) = workspace(&app, &token, "strict-capture").await;
    let (project_id, _) = project(&app, &token, &workspace_id, "reviewed").await;
    let session_id = session(&app, &token, &workspace_id, &project_id, "strict-session").await;
    append(
        &app,
        &token,
        &session_id,
        vec![event(
            "strict-one",
            "STRICT-CANDIDATE The governed release decision is retained for review.",
        )],
    )
    .await;
    let (status, batch) = freeze(&app, &token, &session_id, "strict-freeze").await;
    assert_eq!(status, StatusCode::CREATED, "{batch}");
    run_capture(&state).await;
    let values = candidates(&app, &token, batch["id"].as_str().expect("batch id")).await;
    assert_eq!(values.len(), 1);
    let id = values[0]["id"].as_str().expect("candidate id");
    let (status, pending) = decide(&app, &token, id, "accept", "strict-accept", json!({})).await;
    assert_eq!(status, StatusCode::CREATED, "{pending}");
    assert_eq!(pending["candidate"]["state"], "accepted");
    assert_eq!(pending["candidate"]["resulting_outcome"], "pending_review");
    assert!(pending["candidate"]["resulting_change_id"].is_string());
    assert!(pending["candidate"]["resulting_revision_id"].is_null());
    let mut tx = tenant_fixture::begin(&state.pool, tenant_id).await;
    let active: i64 =
        sqlx::query_scalar("select count(*) from knowledge_items where tenant_id = $1")
            .bind(tenant_id.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .expect("count unpublished Knowledge");
    tx.commit()
        .await
        .expect("commit unpublished Knowledge read");
    assert_eq!(active, 0, "pending review is not active Knowledge");
    let (status, replay) = decide(&app, &token, id, "accept", "strict-accept", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["replayed"], true);
    assert_eq!(
        replay["candidate"]["resulting_change_id"],
        pending["candidate"]["resulting_change_id"]
    );
}

#[tokio::test]
async fn candidate_matches_are_reauthorised_and_foreign_tenants_see_404() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant(synveda_policy::STANDARD).await else {
        return;
    };
    let app = router(state.clone());
    let admin = issue(ADMIN, tenant_id);
    add_identity(&state, tenant_id, MEMBER).await;
    let (workspace_id, _) = workspace(&app, &admin, "match-visibility").await;
    let (project_a, scope_a) = project(&app, &admin, &workspace_id, "private-match").await;
    let (project_b, _) = project(&app, &admin, &workspace_id, "member-anchor").await;
    let (status, grant) = call(
        &app,
        "POST",
        &format!("/v1/projects/{project_b}/members"),
        &admin,
        Some("grant-member-sibling"),
        Some(json!({"principal_id": MEMBER, "role": "member"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{grant}");

    let shared_text = "MATCH-SENTINEL Public requests use X-Request-Id for correlation.";
    let (status, existing) = call(
        &app,
        "POST",
        "/v1/knowledge",
        &admin,
        Some("confidential-existing"),
        Some(json!({
            "scope_id": scope_a,
            "project_id": project_a,
            "knowledge_type": "fact",
            "origin": "authored",
            "content": {
                "title": "Request correlation",
                "body_markdown": shared_text,
                "summary": shared_text,
                "tags": ["correlation"],
                "sensitivity": "confidential",
                "confidence_permille": 1000,
                "verification_metadata": {},
                "metadata": {},
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{existing}");
    assert_eq!(existing["outcome"], "applied");
    let existing_id = existing["knowledge_item_id"]
        .as_str()
        .expect("existing Knowledge id")
        .to_owned();

    let session_id = session(&app, &admin, &workspace_id, &project_a, "match-session").await;
    append(
        &app,
        &admin,
        &session_id,
        vec![event("match-event", shared_text)],
    )
    .await;
    let (status, batch) = freeze(&app, &admin, &session_id, "match-freeze").await;
    assert_eq!(status, StatusCode::CREATED, "{batch}");
    let batch_id = batch["id"].as_str().expect("batch id").to_owned();
    run_capture(&state).await;
    let admin_candidates = candidates(&app, &admin, &batch_id).await;
    assert_eq!(admin_candidates.len(), 1);
    assert_eq!(
        admin_candidates[0]["matches"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        admin_candidates[0]["matches"][0]["knowledge_item_id"],
        existing_id
    );
    let (status, conflict_page) = call(
        &app,
        "GET",
        "/v1/knowledge-conflicts?status=open&limit=200",
        &admin,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{conflict_page}");
    let conflict = conflict_page["conflicts"]
        .as_array()
        .expect("conflict page")
        .iter()
        .find(|set| {
            set["members"].as_array().is_some_and(|members| {
                members
                    .iter()
                    .any(|member| member["capture_candidate_id"] == admin_candidates[0]["id"])
            })
        })
        .expect("capture-backed conflict evidence");
    let conflict_id = conflict["id"].as_str().expect("conflict id").to_owned();
    let conflict_revision = conflict["revision"].as_i64().expect("conflict revision");

    // A sibling-project member can read this standard-profile session and
    // its internal candidate through the governed ambit, but confidential
    // Knowledge requires an explicit grant at the item's scope. The match is
    // omitted whole—there is no id, edge or count side channel.
    let member = issue(MEMBER, tenant_id);
    let (status, visible_batch) = call(
        &app,
        "GET",
        &format!("/v1/capture-batches/{batch_id}"),
        &member,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{visible_batch}");
    let member_candidates = candidates(&app, &member, &batch_id).await;
    assert_eq!(member_candidates.len(), 1);
    assert_eq!(member_candidates[0]["matches"], json!([]));
    let (status, denied_item) = call(
        &app,
        "GET",
        &format!("/v1/knowledge/{existing_id}"),
        &member,
        None,
        None,
    )
    .await;
    assert!(
        matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
        "confidential match unexpectedly readable: {status} {denied_item}"
    );
    let (status, hidden_conflicts) = call(
        &app,
        "GET",
        "/v1/knowledge-conflicts?status=open&limit=200",
        &member,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{hidden_conflicts}");
    assert_eq!(hidden_conflicts["conflicts"], json!([]));
    assert_eq!(hidden_conflicts["policy_exclusions"], true);
    let (status, denied_resolution) = call(
        &app,
        "POST",
        &format!("/v1/knowledge-conflicts/{conflict_id}/resolve"),
        &member,
        Some("hidden-capture-conflict"),
        Some(json!({
            "expected_revision": conflict_revision,
            "resolution": "duplicate",
            "reason": "a guessed id must not reveal the candidate or comparison",
        })),
    )
    .await;
    assert!(
        matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
        "capture conflict became an existence oracle: {status} {denied_resolution}"
    );
    let (status, new_learnings_only) = call(
        &app,
        "POST",
        &format!("/v1/knowledge-conflicts/{conflict_id}/resolve"),
        &admin,
        Some("capture-conflict-owned-by-review"),
        Some(json!({
            "expected_revision": conflict_revision,
            "resolution": "duplicate",
            "reason": "capture challengers retain New Learnings as publication authority",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{new_learnings_only}");

    let candidate_id = admin_candidates[0]["id"]
        .as_str()
        .expect("candidate id")
        .to_owned();
    let Some((foreign_state, foreign_tenant)) = admitted_tenant(synveda_policy::STANDARD).await
    else {
        return;
    };
    let foreign_app = router(foreign_state);
    let foreign = issue(ADMIN, foreign_tenant);
    let (status, hidden_batch) = call(
        &foreign_app,
        "GET",
        &format!("/v1/capture-batches/{batch_id}"),
        &foreign,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{hidden_batch}");
    let (status, hidden_candidate) = decide(
        &foreign_app,
        &foreign,
        &candidate_id,
        "dismiss",
        "foreign-dismiss",
        json!({"reason": "must not cross tenants"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{hidden_candidate}");
}

/// CPR-22's product checkpoint deliberately crosses every seam in one test.
/// The only direct writes are the documented test-policy bootstrap (identity
/// and root grant); sessions, events, membership, candidate decisions,
/// Knowledge, supersession and context all use their public application APIs.
#[tokio::test]
async fn pulseboard_cross_session_team_knowledge_loop_is_governed_end_to_end() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant(synveda_policy::STANDARD).await else {
        return;
    };
    let app = router(state.clone());
    let alice = issue(ADMIN, tenant_id);
    add_identity(&state, tenant_id, MEMBER).await;
    let bob = issue(MEMBER, tenant_id);

    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin identity lookup");
    let alice_scope = identities::by_subject(&mut *tx, tenant_id, ADMIN)
        .await
        .expect("read Alice identity")
        .expect("Alice identity")
        .scope_id;
    tx.commit().await.expect("commit identity lookup");

    let (workspace_id, _) = workspace(&app, &alice, "pulseboard-mvp").await;
    let (project_id, project_scope) = project(&app, &alice, &workspace_id, "delivery-api").await;
    let project_scope: ScopeId = project_scope.parse().expect("parse project scope");
    let (grant_status, grant) = call(
        &app,
        "POST",
        &format!("/v1/projects/{project_id}/members"),
        &alice,
        Some("cpr22-grant-bob"),
        Some(json!({"principal_id": MEMBER, "role": "member"})),
    )
    .await;
    assert_eq!(grant_status, StatusCode::CREATED, "{grant}");

    // Alice's first supported run records three durable facts and one detail
    // that the reviewer will deliberately decline to publish.
    let alice_session = session(
        &app,
        &alice,
        &workspace_id,
        &project_id,
        "cpr22-alice-session",
    )
    .await;
    let statements = [
        "Webhook deliveries are deduplicated by provider event ID.",
        "Public requests currently use X-Request-Id.",
        "I prefer my local quick-test command to be just test-fast.",
        "The cafe beside the office closes at four.",
    ];
    let appended = append(
        &app,
        &alice,
        &alice_session,
        statements
            .iter()
            .enumerate()
            .map(|(ordinal, text)| event(&format!("cpr22-alice-{ordinal}"), text))
            .collect(),
    )
    .await;
    let source_event_ids = appended["events"]
        .as_array()
        .expect("appended events")
        .iter()
        .map(|entry| entry["event"]["id"].as_str().expect("event id").to_owned())
        .collect::<Vec<_>>();
    let (batch_status, batch) = freeze(&app, &alice, &alice_session, "cpr22-alice-capture").await;
    assert_eq!(batch_status, StatusCode::CREATED, "{batch}");
    let alice_batch = batch["id"].as_str().expect("Alice batch id");

    let mut tx = tenant_fixture::begin(&state.pool, tenant_id).await;
    let unpublished: i64 =
        sqlx::query_scalar("select count(*) from knowledge_items where tenant_id = $1")
            .bind(tenant_id.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .expect("count pre-review Knowledge");
    tx.commit().await.expect("commit pre-review Knowledge read");
    assert_eq!(
        unpublished, 0,
        "extraction intent must not publish Knowledge"
    );
    run_capture(&state).await;
    let proposed = candidates(&app, &alice, alice_batch).await;
    assert_eq!(proposed.len(), statements.len());
    assert!(proposed.iter().all(|value| value["state"] == "pending"));
    for (candidate, source_id) in proposed.iter().zip(&source_event_ids) {
        assert_eq!(candidate["source_event_ids"], json!([source_id]));
        assert!(candidate["resulting_change_id"].is_null());
    }
    assert_eq!(proposed[0]["proposed_scope_id"], project_scope.to_string());
    assert_eq!(proposed[1]["proposed_scope_id"], project_scope.to_string());
    assert_eq!(proposed[2]["knowledge_type"], "preference");
    assert_eq!(proposed[2]["proposed_scope_id"], alice_scope.to_string());
    assert_eq!(proposed[2]["proposed_project_id"], Value::Null);
    assert_eq!(proposed[2]["proposed_owner_principal_id"], ADMIN);

    let (first_status, first) = decide(
        &app,
        &alice,
        proposed[0]["id"].as_str().expect("first candidate"),
        "accept",
        "cpr22-share-webhooks",
        json!({}),
    )
    .await;
    assert_eq!(first_status, StatusCode::CREATED, "{first}");
    assert_eq!(first["candidate"]["resulting_outcome"], "applied");
    let webhook_item = first["candidate"]["resulting_knowledge_item_id"]
        .as_str()
        .expect("webhook Knowledge")
        .to_owned();
    let webhook_revision = first["candidate"]["resulting_revision_id"]
        .as_str()
        .expect("webhook revision")
        .to_owned();

    let (second_status, second) = decide(
        &app,
        &alice,
        proposed[1]["id"].as_str().expect("second candidate"),
        "accept",
        "cpr22-share-request-id",
        json!({}),
    )
    .await;
    assert_eq!(second_status, StatusCode::CREATED, "{second}");
    assert_eq!(second["candidate"]["resulting_outcome"], "applied");
    let request_id_item = second["candidate"]["resulting_knowledge_item_id"]
        .as_str()
        .expect("request-id Knowledge")
        .to_owned();
    let request_id_revision = second["candidate"]["resulting_revision_id"]
        .as_str()
        .expect("request-id revision")
        .to_owned();

    let (private_status, private) = decide(
        &app,
        &alice,
        proposed[2]["id"].as_str().expect("private candidate"),
        "accept",
        "cpr22-keep-private",
        json!({}),
    )
    .await;
    assert_eq!(private_status, StatusCode::CREATED, "{private}");
    assert_eq!(private["candidate"]["resulting_outcome"], "applied");
    let private_item = private["candidate"]["resulting_knowledge_item_id"]
        .as_str()
        .expect("private Knowledge")
        .to_owned();

    let (dismiss_status, dismissed) = decide(
        &app,
        &alice,
        proposed[3]["id"].as_str().expect("incidental candidate"),
        "dismiss",
        "cpr22-dismiss-incidental",
        json!({"reason": "not durable project knowledge"}),
    )
    .await;
    assert_eq!(dismiss_status, StatusCode::CREATED, "{dismissed}");
    assert_eq!(dismissed["candidate"]["state"], "dismissed");
    assert!(dismissed["candidate"]["resulting_change_id"].is_null());

    // A genuinely fresh Bob session uses the same project runtime. Both
    // shared revisions arrive with source evidence; Alice's principal-owned
    // preference is absent from the rendered block and every trace address.
    let bob_session = session(&app, &bob, &workspace_id, &project_id, "cpr22-bob-session").await;
    let reused_started = std::time::Instant::now();
    let reused = compose_context(
        &app,
        &bob,
        &bob_session,
        "cpr22-bob-reuse",
        "provider event ID OR X-Request-Id OR quick-test",
    )
    .await;
    let reused_latency_ms = reused_started.elapsed().as_secs_f64() * 1_000.0;
    let reused_text = reused["rendered"].as_str().expect("Bob rendered context");
    assert!(reused_text.contains("provider event ID"), "{reused_text}");
    assert!(reused_text.contains("X-Request-Id"), "{reused_text}");
    assert!(!reused_text.contains("test-fast"), "{reused_text}");
    let reused_detail =
        context_detail(&app, &bob, reused["id"].as_str().expect("reuse run id")).await;
    let reused_run_id = reused["id"].as_str().expect("reuse run id");
    let reused_selections = reused_detail["selections"]
        .as_array()
        .expect("reuse selections");
    for (item, revision) in [
        (&webhook_item, &webhook_revision),
        (&request_id_item, &request_id_revision),
    ] {
        let selected = reused_selections
            .iter()
            .find(|selection| {
                selection["knowledge_item_id"] == item.as_str()
                    && selection["knowledge_revision_id"] == revision.as_str()
            })
            .unwrap_or_else(|| {
                panic!("shared revision absent from context trace: {reused_detail}")
            });
        assert!(selected["rank"].as_i64().is_some());
        assert!(selected["token_count"].as_i64().is_some());
        assert!(
            !selected["reason_codes"]
                .as_array()
                .expect("reason codes")
                .is_empty()
        );
        assert_eq!(
            selected["sources"][0]["source_type"], "session_event",
            "selected Knowledge must retain its conversation evidence: {selected}"
        );
    }
    let webhook_selection_id = reused_selections
        .iter()
        .find(|selection| selection["knowledge_revision_id"] == webhook_revision)
        .and_then(|selection| selection["id"].as_str())
        .expect("webhook selection id");
    let request_id_selection_id = reused_selections
        .iter()
        .find(|selection| selection["knowledge_revision_id"] == request_id_revision)
        .and_then(|selection| selection["id"].as_str())
        .expect("request-id selection id");
    assert!(!reused_detail.to_string().contains(&private_item));
    assert!(!reused_detail.to_string().contains("test-fast"));
    let (private_read_status, private_read) = call(
        &app,
        "GET",
        &format!("/v1/knowledge/{private_item}"),
        &bob,
        None,
        None,
    )
    .await;
    assert!(
        matches!(
            private_read_status,
            StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
        ),
        "Bob read Alice's private Knowledge: {private_read_status} {private_read}"
    );
    assert!(!private_read.to_string().contains("test-fast"));
    let (private_query_status, private_query) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{bob_session}/knowledge-query"),
        &bob,
        None,
        Some(json!({"query": "test-fast", "limit": 20})),
    )
    .await;
    assert_eq!(private_query_status, StatusCode::OK, "{private_query}");
    assert_eq!(private_query["items"], json!([]));

    // Bob's correction is another captured proposal. Replace names the exact
    // old revision, and its reviewed content stops repeating the obsolete
    // convention while retaining the original session event as provenance.
    let correction = append(
        &app,
        &bob,
        &bob_session,
        vec![event(
            "cpr22-bob-correction",
            "We decided traceparent replaces X-Request-Id for public requests.",
        )],
    )
    .await;
    let correction_event = correction["events"][0]["event"]["id"]
        .as_str()
        .expect("correction event")
        .to_owned();
    let (correction_batch_status, correction_batch) =
        freeze(&app, &bob, &bob_session, "cpr22-bob-capture").await;
    assert_eq!(
        correction_batch_status,
        StatusCode::CREATED,
        "{correction_batch}"
    );
    run_capture(&state).await;
    let correction_candidates = candidates(
        &app,
        &bob,
        correction_batch["id"]
            .as_str()
            .expect("correction batch id"),
    )
    .await;
    assert_eq!(correction_candidates.len(), 1);
    assert_eq!(
        correction_candidates[0]["source_event_ids"],
        json!([correction_event])
    );
    let (replace_status, replaced) = decide(
        &app,
        &bob,
        correction_candidates[0]["id"]
            .as_str()
            .expect("correction candidate"),
        "replace",
        "cpr22-replace-request-id",
        json!({
            "item_id": request_id_item,
            "expected_revision_id": request_id_revision,
            "replacement": {
                "knowledge_type": "convention",
                "content": revised_content(
                    "Current request correlation convention",
                    "PulseBoard public requests use the W3C traceparent header."
                )
            }
        }),
    )
    .await;
    assert_eq!(replace_status, StatusCode::CREATED, "{replaced}");
    assert_eq!(replaced["candidate"]["state"], "replaced");
    assert_eq!(replaced["candidate"]["resulting_outcome"], "applied");
    let replacement_item = replaced["candidate"]["resulting_knowledge_item_id"]
        .as_str()
        .expect("replacement Knowledge")
        .to_owned();
    let replacement_revision = replaced["candidate"]["resulting_revision_id"]
        .as_str()
        .expect("replacement revision")
        .to_owned();

    let (old_status, old_detail) = call(
        &app,
        "GET",
        &format!("/v1/knowledge/{request_id_item}"),
        &alice,
        None,
        None,
    )
    .await;
    assert_eq!(old_status, StatusCode::OK, "{old_detail}");
    assert_eq!(old_detail["lifecycle_state"], "superseded");
    let mut tx = tenant_fixture::begin(&state.pool, tenant_id).await;
    let relation_count: i64 = sqlx::query_scalar(
        "select count(*) from knowledge_relations where tenant_id = $1 \
         and source_item_id = $2 and target_item_id = $3 and relation_type = 'supersedes'",
    )
    .bind(tenant_id.as_uuid())
    .bind(uuid::Uuid::parse_str(&replacement_item).expect("replacement item UUID"))
    .bind(uuid::Uuid::parse_str(&request_id_item).expect("old item UUID"))
    .fetch_one(&mut *tx)
    .await
    .expect("count explicit supersession");
    tx.commit().await.expect("commit supersession read");
    assert_eq!(relation_count, 1);

    // A third clean run receives the correction, never the superseded head.
    // The returned detail is the exact generated contract the Context
    // Inspector renders, so these are also its user-visible assertions.
    let third_session = session(
        &app,
        &bob,
        &workspace_id,
        &project_id,
        "cpr22-third-session",
    )
    .await;
    let current_started = std::time::Instant::now();
    let current = compose_context(
        &app,
        &bob,
        &third_session,
        "cpr22-third-context",
        "traceparent OR X-Request-Id",
    )
    .await;
    let current_latency_ms = current_started.elapsed().as_secs_f64() * 1_000.0;
    let current_text = current["rendered"].as_str().expect("current context");
    assert!(current_text.contains("traceparent"), "{current_text}");
    assert!(!current_text.contains("X-Request-Id"), "{current_text}");
    assert!(!current_text.contains("test-fast"), "{current_text}");
    let current_run_id = current["id"].as_str().expect("current run id");
    let inspector = context_detail(&app, &bob, current_run_id).await;
    let selected = inspector["selections"]
        .as_array()
        .expect("inspector selections");
    let replacement_selection = selected
        .iter()
        .find(|selection| {
            selection["knowledge_item_id"] == replacement_item
                && selection["knowledge_revision_id"] == replacement_revision
        })
        .unwrap_or_else(|| panic!("replacement missing from inspector: {inspector}"));
    assert_eq!(
        replacement_selection["sources"][0]["session_event_id"],
        correction_event
    );
    assert!(
        selected
            .iter()
            .all(|selection| selection["knowledge_item_id"] != request_id_item),
        "superseded Knowledge was selected: {inspector}"
    );
    let obsolete = inspector["candidates"]
        .as_array()
        .expect("inspector candidates")
        .iter()
        .find(|candidate| candidate["knowledge_item_id"] == request_id_item)
        .unwrap_or_else(|| panic!("obsolete candidate absent from explanation: {inspector}"));
    assert_eq!(obsolete["lifecycle_state"], "superseded");
    assert_eq!(obsolete["exclusion_reason"], "superseded");
    assert_eq!(
        inspector["run"]["retrieval_version"],
        "knowledge-planner-v2"
    );
    assert!(inspector["run"]["block_hash"].as_str().is_some());

    // CPR-40 measures outcomes as separate facts. Retrieval and injection are
    // not proxies for use: the useful webhook convention and the request-id
    // convention that later caused a correction retain distinct observations
    // against the exact immutable revisions and ContextRun that supplied them.
    for (feedback_type, run_id, selection_id, revision_id) in [
        (
            "referenced_by_agent",
            reused_run_id,
            webhook_selection_id,
            webhook_revision.as_str(),
        ),
        (
            "helpful",
            reused_run_id,
            webhook_selection_id,
            webhook_revision.as_str(),
        ),
        (
            "accepted_by_user",
            reused_run_id,
            request_id_selection_id,
            request_id_revision.as_str(),
        ),
        (
            "unhelpful",
            reused_run_id,
            request_id_selection_id,
            request_id_revision.as_str(),
        ),
        (
            "caused_correction",
            reused_run_id,
            request_id_selection_id,
            request_id_revision.as_str(),
        ),
    ] {
        let (feedback_status, feedback) = call(
            &app,
            "POST",
            &format!("/v1/context-runs/{run_id}/feedback"),
            &bob,
            Some(&format!("cpr40-{feedback_type}")),
            Some(json!({
                "context_selection_id": selection_id,
                "knowledge_revision_id": revision_id,
                "feedback_type": feedback_type,
            })),
        )
        .await;
        assert_eq!(feedback_status, StatusCode::CREATED, "{feedback}");
        assert_eq!(feedback["feedback_type"], feedback_type);
    }

    let (timeline_status, timeline) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{third_session}/timeline"),
        &bob,
        None,
        None,
    )
    .await;
    assert_eq!(timeline_status, StatusCode::OK, "{timeline}");
    let timeline_context = timeline["entries"]
        .as_array()
        .expect("timeline entries")
        .iter()
        .find(|entry| entry["kind"] == "context_run" && entry["id"] == current_run_id)
        .expect("timeline Context Inspector link target");
    assert!(
        timeline_context["summary"]
            .as_str()
            .expect("context summary")
            .starts_with("Synveda supplied ")
    );

    // The product path remains one model: every publication has a VedaFlow
    // change, the retired aggregate is absent, and the deleted global runtime
    // endpoints are still hard 404s.
    let mut tx = tenant_fixture::begin(&state.pool, tenant_id).await;
    let counts: (i64, i64, i64, i64, i64, i64, i64, i64, i64, bool) = sqlx::query_as(
        "select \
           (select count(*) from sessions where tenant_id = $1), \
           (select count(*) from session_events where tenant_id = $1), \
           (select count(*) from capture_batches where tenant_id = $1), \
           (select count(*) from capture_candidates where tenant_id = $1), \
           (select count(*) from capture_candidate_decisions where tenant_id = $1), \
           (select count(*) from knowledge_items where tenant_id = $1), \
           (select count(*) from knowledge_revisions where tenant_id = $1), \
           (select count(*) from knowledge_changes where tenant_id = $1), \
           (select count(*) from session_context_runs where tenant_id = $1), \
           (select to_regclass('public.records') is not null)",
    )
    .bind(tenant_id.as_uuid())
    .fetch_one(&mut *tx)
    .await
    .expect("read MVP database state");
    assert_eq!(counts, (3, 5, 2, 5, 5, 4, 4, 4, 2, false));
    let active: i64 = sqlx::query_scalar(
        "select count(*) from knowledge_items where tenant_id = $1 and lifecycle_state = 'active'",
    )
    .bind(tenant_id.as_uuid())
    .fetch_one(&mut *tx)
    .await
    .expect("count current Knowledge");
    let superseded: i64 = sqlx::query_scalar(
        "select count(*) from knowledge_items where tenant_id = $1 and lifecycle_state = 'superseded'",
    )
    .bind(tenant_id.as_uuid())
    .fetch_one(&mut *tx)
    .await
    .expect("count superseded Knowledge");
    assert_eq!((active, superseded), (3, 1));
    tx.commit().await.expect("commit MVP state read");

    for path in ["/v1/observe", "/v1/inject", "/v1/recall"] {
        let (status, body) = call(&app, "POST", path, &alice, None, Some(json!({}))).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "dead route {path}: {body}");
    }

    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin audit verification");
    let audit = synveda_audit::tail(&mut tx, tenant_id, 1_000)
        .await
        .expect("read MVP audit chain");
    let verification = synveda_audit::verify(&mut tx, tenant_id)
        .await
        .expect("verify MVP audit chain");
    tx.commit().await.expect("commit audit verification");
    assert!(
        matches!(
            verification,
            synveda_audit::ChainVerification::Valid { events } if events == audit.len() as i64
        ),
        "invalid audit chain: {verification:?}"
    );
    let actions = audit
        .iter()
        .map(|entry| entry.action.as_str())
        .collect::<Vec<_>>();
    for action in [
        "authz.decision",
        "session.opened",
        "session.events.appended",
        "capture.batch.created",
        "capture.batch.completed",
        "capture.candidate.decided",
        "knowledge.change.opened",
        "knowledge.change.applied",
        "context.candidates.retrieved",
        "context.selections.made",
        "session.context.composed",
    ] {
        assert!(
            actions.contains(&action),
            "missing audit action {action}: {actions:?}"
        );
    }
    assert_eq!(
        actions
            .iter()
            .filter(|action| **action == "knowledge.change.opened")
            .count(),
        4,
        "every active mutation must open exactly one VedaFlow change"
    );
    let audit_text = audit
        .iter()
        .map(|entry| entry.payload.to_string())
        .collect::<String>();
    for sensitive in ["test-fast", "X-Request-Id", "traceparent"] {
        assert!(
            !audit_text.contains(sensitive),
            "ordinary audit metadata leaked Knowledge or session content: {sensitive}"
        );
    }

    // The deterministic product runner requests this machine-readable
    // evidence file. Ordinary `cargo test` runs do not write anything. The
    // values below are measurements of persisted product state; the runner
    // joins them with independently executed Skill, Tool, OKF, graph and
    // isolation scenarios rather than manufacturing counters from test names.
    if let Ok(path) = std::env::var("SYNVEDA_PRODUCT_EVAL_EVIDENCE") {
        let mut tx = tenant_fixture::begin(&state.pool, tenant_id).await;
        let funnel: (i64, i64, i64, i64) = sqlx::query_as(
            "select \
               (select coalesce(sum(candidate_count), 0) from session_context_runs where tenant_id = $1), \
               (select count(*) from context_selections where tenant_id = $1), \
               (select coalesce(sum(selection_count), 0) from session_context_runs \
                  where tenant_id = $1 and completion_status = 'completed'), \
               (select coalesce(sum(token_count), 0) from context_selections where tenant_id = $1)",
        )
        .bind(tenant_id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .expect("read CPR-40 funnel measurements");
        let feedback: Vec<(String, i64)> = sqlx::query_as(
            "select feedback_type, count(*) from context_feedback \
             where tenant_id = $1 group by feedback_type order by feedback_type",
        )
        .bind(tenant_id.as_uuid())
        .fetch_all(&mut *tx)
        .await
        .expect("read CPR-40 feedback measurements");
        let feedback = feedback
            .into_iter()
            .collect::<std::collections::BTreeMap<_, _>>();
        let source_gaps: i64 = sqlx::query_scalar(
            "select count(*) from context_selections selected \
             where selected.tenant_id = $1 and selected.knowledge_revision_id is not null \
               and not exists (select 1 from knowledge_sources source \
                 join knowledge_revision_sources link \
                   on link.tenant_id = source.tenant_id and link.knowledge_source_id = source.id \
                 where link.tenant_id = selected.tenant_id \
                   and link.knowledge_revision_id = selected.knowledge_revision_id)",
        )
        .bind(tenant_id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .expect("measure selected Knowledge provenance gaps");
        let model_versions: Vec<String> = sqlx::query_scalar(
            "select distinct model_version from capture_batches \
             where tenant_id = $1 and model_version is not null order by model_version",
        )
        .bind(tenant_id.as_uuid())
        .fetch_all(&mut *tx)
        .await
        .expect("read CPR-40 extraction model versions");
        tx.commit().await.expect("commit product measurements read");
        assert_eq!(
            model_versions.len(),
            1,
            "one deterministic ruleset version per product run"
        );
        let evidence = json!({
            "schema_version": 1,
            "code_revision": std::env::var("SYNVEDA_PRODUCT_EVAL_CODE_REVISION")
                .unwrap_or_else(|_| "unrecorded".to_owned()),
            "retrieval_version": inspector["run"]["retrieval_version"],
            "model_version": model_versions[0],
            "embedding_model": inspector["run"]["embedding_model"],
            "index_version": inspector["run"]["index_version"],
            "measurements": {
                "retrieved": funnel.0,
                "selected": funnel.1,
                "injected": funnel.2,
                "referenced_by_agent": feedback.get("referenced_by_agent").copied().unwrap_or(0),
                "accepted_by_user": feedback.get("accepted_by_user").copied().unwrap_or(0),
                "helpful": feedback.get("helpful").copied().unwrap_or(0),
                "unhelpful": feedback.get("unhelpful").copied().unwrap_or(0),
                "caused_correction": feedback.get("caused_correction").copied().unwrap_or(0),
                "selected_tokens": funnel.3,
                "context_latency_ms": [reused_latency_ms, current_latency_ms],
                "capture_candidates": counts.3,
                "accepted_candidates": counts.4 - 1,
                "dismissed_candidates": 1,
            },
            "hard_gate_observations": {
                "private_scope_leakage": 0,
                "superseded_current_injection": 0,
                "selected_without_provenance": source_gaps,
                "plaintext_sensitive_audit_leakage": 0,
            },
        });
        std::fs::write(
            &path,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&evidence).expect("serialise CPR-40 evidence")
            ),
        )
        .unwrap_or_else(|error| panic!("write CPR-40 evidence {path}: {error}"));
    }
}
