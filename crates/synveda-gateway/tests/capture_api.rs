//! CPR-18 acceptance evidence for the public capture plane.
//!
//! The tests exercise the real HTTP surface, embedded PDP, VedaFlow Knowledge
//! commands, database worker lease, forced-RLS store and hash-chain audit. A
//! candidate is deliberately inspected before and after every transition: the
//! extraction half may propose, but only the decision half may publish.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_ingest::capture_worker;
use synveda_ingest::extraction::{AnyExtractor, DeterministicExtractor};
use synveda_store::{access, identities, policy_assignments, rls, scopes, tenants};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::{GrantId, IdentityId, IdentityKind, ScopeId, TenantId, TenantStatus};
use tower::ServiceExt;

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
        search_index: Arc::new(
            synveda_retrieval::SearchIndex::open(
                std::env::temp_dir()
                    .join("synveda-cpr18-tests")
                    .join(TenantId::new().to_string()),
            )
            .expect("open search sidecar"),
        ),
        embedder: Arc::new(synveda_ingest::embedding::AnyEmbedder::Deterministic(
            synveda_ingest::embedding::DeterministicEmbedder::new(),
        )),
        inject_embed_timeout: Duration::from_millis(100),
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
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");
    let tenant_id = TenantId::new();
    tenants::create(
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
    policy_assignments::set_default(&mut *tx, tenant_id, pack)
        .await
        .expect("select shipped policy pack");
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

    let before_knowledge: i64 =
        sqlx::query_scalar("select count(*) from knowledge_items where tenant_id = $1")
            .bind(tenant_id.as_uuid())
            .fetch_one(&state.pool)
            .await
            .expect("count Knowledge before extraction");
    assert_eq!(before_knowledge, 0);
    run_capture(&state).await;

    let after_extraction: i64 =
        sqlx::query_scalar("select count(*) from knowledge_items where tenant_id = $1")
            .bind(tenant_id.as_uuid())
            .fetch_one(&state.pool)
            .await
            .expect("count Knowledge after extraction");
    let old_records: i64 = sqlx::query_scalar("select count(*) from records where tenant_id = $1")
        .bind(tenant_id.as_uuid())
        .fetch_one(&state.pool)
        .await
        .expect("count retired records");
    assert_eq!(after_extraction, 0, "extraction may only create candidates");
    assert_eq!(
        old_records, 0,
        "the deleted writer may not dual-write records"
    );

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
    let scrubbed: (String, String, bool, Option<Value>) = sqlx::query_as(
        "select candidate.title, candidate.body_markdown, candidate.content_erased, decision.payload \
         from capture_candidates candidate \
         join capture_candidate_decisions decision \
           on decision.tenant_id = candidate.tenant_id and decision.candidate_id = candidate.id \
         where candidate.tenant_id = $1 and candidate.id = $2",
    )
    .bind(tenant_id.as_uuid())
    .bind(uuid::Uuid::parse_str(values[5]["id"].as_str().expect("candidate id")).expect("uuid"))
    .fetch_one(&state.pool)
    .await
    .expect("read scrubbed candidate");
    assert_eq!(scrubbed, (String::new(), String::new(), true, None));

    let capture_audit: String = sqlx::query_scalar(
        "select coalesce(string_agg(payload::text, ' '), '') from audit_log \
         where tenant_id = $1 and action like 'capture.%'",
    )
    .bind(tenant_id.as_uuid())
    .fetch_one(&state.pool)
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
    .fetch_one(&state.pool)
    .await
    .expect("count decision audit events");
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
    let active: i64 =
        sqlx::query_scalar("select count(*) from knowledge_items where tenant_id = $1")
            .bind(tenant_id.as_uuid())
            .fetch_one(&state.pool)
            .await
            .expect("count unpublished Knowledge");
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
