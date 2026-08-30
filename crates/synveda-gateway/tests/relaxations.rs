//! CPR-31 acceptance: one VedaFlow path for auto-applied and reviewed
//! policy relaxations (ADR-0090).
//!
//! These tests use only public product APIs after typed Configuration
//! bootstrap. They prove that the stable/immutable relaxation projection
//! widens one exact subject's Knowledge read, then closes it by revocation;
//! and that a stricter profile retains pending and rejected outcomes.

#[path = "../../synveda-store/tests/support/tenant_fixture.rs"]
mod tenant_fixture;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{TimeDelta, Utc};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, behavior_test_router as router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder};
use synveda_policy::Pdp;
use synveda_store::{access, identities, rls, scopes};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::{GrantId, IdentityId, IdentityKind, ScopeId, TenantId, TenantStatus};
use tower::ServiceExt;

#[path = "support/configuration.rs"]
mod configuration_support;

const SECRET: &[u8] = b"cpr-31-governed-relaxations";
const AUTHOR: &str = "cpr31-author";
const REVIEWER: &str = "cpr31-reviewer";
const REVIEWER_TWO: &str = "cpr32-reviewer-two";
const EXECUTOR: &str = "cpr32-executor";
const ALICE: &str = "cpr31-alice";
const BOB: &str = "cpr31-bob";

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
            .max_connections(8)
            .connect_lazy(url)
            .expect("parse DATABASE_URL"),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp: Arc::new(Pdp::new().expect("build embedded PDP")),
        service_token_max_ttl: Duration::from_secs(3_600),
        embedder: Arc::new(AnyEmbedder::Deterministic(DeterministicEmbedder::new())),
        context_embed_timeout: Duration::from_millis(100),
        keys: Arc::new(synveda_store::keys::KeyRing::new(
            synveda_crypto::Kms::Local(
                synveda_crypto::LocalKms::from_hex(&"31".repeat(32), "local:cpr31")
                    .expect("test KEK"),
            ),
        )),
    }
}

fn token(subject: &str, tenant: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue(subject, tenant, Duration::from_secs(300))
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    key: Option<&str>,
    payload: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
    if let Some(key) = key {
        request = request.header("idempotency-key", key);
    }
    let request = if let Some(payload) = payload {
        request
            .header("content-type", "application/json")
            .body(Body::from(payload.to_string()))
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
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("response is JSON")
    };
    (status, value)
}

async fn provision(tx: &mut sqlx::PgConnection, tenant: TenantId, subject: &str) -> IdentityId {
    let principal = scopes::ensure_principal_scope(tx, tenant, subject, subject)
        .await
        .expect("create principal scope");
    let id = IdentityId::new();
    identities::create(
        tx,
        id,
        tenant,
        Some(subject),
        IdentityKind::User,
        None,
        Some(subject),
        principal.id,
    )
    .await
    .expect("create identity");
    id
}

async fn administrator(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    root: ScopeId,
    subject: &str,
) -> IdentityId {
    let id = provision(tx, tenant, subject).await;
    access::create_grant(
        tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id: tenant,
            scope_id: root,
            subject: GrantSubject::Principal {
                principal_id: subject.to_owned(),
            },
            role_key: RoleKey::Administrator,
            source: GrantSource::Automation,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .expect("grant administrator");
    id
}

struct World {
    state: AppState,
    app: Router,
    tenant: TenantId,
    root: ScopeId,
    alice_id: IdentityId,
    author: String,
    reviewer: String,
    reviewer_two: String,
    executor: String,
    alice: String,
    bob: String,
}

async fn admitted(pack: &str) -> Option<World> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!(
            "skipping CPR-31 relaxation API test: DATABASE_URL is not set \
             (run `make dev-up` then `make db-test`)"
        );
        return None;
    };
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::epoch::verify(&pool)
        .await
        .expect("apply migrations");
    let tenant = TenantId::new();
    tenant_fixture::create(
        &pool,
        tenant,
        &format!("cpr31-{}", tenant.as_uuid().simple()),
        "CPR-31 relaxation acceptance",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("begin bootstrap");
    let root = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("create tenant root");
    administrator(&mut tx, tenant, root.id, AUTHOR).await;
    administrator(&mut tx, tenant, root.id, REVIEWER).await;
    administrator(&mut tx, tenant, root.id, REVIEWER_TWO).await;
    administrator(&mut tx, tenant, root.id, EXECUTOR).await;
    let alice_id = provision(&mut tx, tenant, ALICE).await;
    provision(&mut tx, tenant, BOB).await;
    configuration_support::bind_tenant_pack(&mut tx, tenant, pack).await;
    tx.commit().await.expect("commit bootstrap");
    let state = state(&url);
    Some(World {
        app: router(state.clone()),
        state,
        tenant,
        root: root.id,
        alice_id,
        author: token(AUTHOR, tenant),
        reviewer: token(REVIEWER, tenant),
        reviewer_two: token(REVIEWER_TWO, tenant),
        executor: token(EXECUTOR, tenant),
        alice: token(ALICE, tenant),
        bob: token(BOB, tenant),
    })
}

fn terms(subject_identity_id: IdentityId, reason: &str) -> Value {
    let now = Utc::now();
    json!({
        "subject_identity_id": subject_identity_id,
        "action": "knowledge.read",
        "max_sensitivity": "internal",
        "requested_start_at": now - TimeDelta::minutes(1),
        "requested_end_at": now + TimeDelta::minutes(30),
        "reason": reason,
    })
}

async fn workspace(world: &World, key: &str) -> Value {
    let (status, body) = call(
        &world.app,
        "POST",
        "/v1/workspaces",
        &world.author,
        Some(key),
        Some(json!({"slug": key, "display_name": "Relaxation target"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}

async fn create_internal_knowledge(world: &World, scope: &str, key: &str) -> String {
    let (status, knowledge) = call(
        &world.app,
        "POST",
        "/v1/knowledge",
        &world.author,
        Some(key),
        Some(json!({
            "scope_id": scope,
            "knowledge_type": "warning",
            "origin": "authored",
            "content": {
                "title": "Provider retry boundary",
                "body_markdown": "Retry provider delivery only by event ID.",
                "summary": "Provider event IDs are the retry boundary.",
                "tags": ["delivery"],
                "sensitivity": "internal",
                "confidence_permille": 950,
                "verification_metadata": {},
                "metadata": {"fixture": "CPR-31"}
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{knowledge}");
    assert_eq!(knowledge["outcome"], "applied", "{knowledge}");
    knowledge["knowledge_item_id"]
        .as_str()
        .expect("Knowledge item")
        .to_owned()
}

#[tokio::test]
async fn personal_auto_apply_uses_vedaflow_and_immutable_versions() {
    let _guard = serial().await;
    let Some(world) = admitted(synveda_policy::OPEN_COLLABORATION).await else {
        return;
    };
    let workspace = workspace(&world, "cpr31-personal").await;
    let scope = workspace["scope_id"].as_str().expect("workspace scope");

    let mut create = terms(world.alice_id, "investigate the delivery incident");
    create["target_scope_id"] = json!(scope);
    let (status, created) = call(
        &world.app,
        "POST",
        "/v1/relaxations",
        &world.author,
        Some("cpr31-create"),
        Some(create.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["outcome"], "applied", "{created}");
    let relaxation = created["relaxation_id"].as_str().expect("relaxation id");
    let first_version = created["version_id"].as_str().expect("version id");

    let (replay_status, replay) = call(
        &world.app,
        "POST",
        "/v1/relaxations",
        &world.author,
        Some("cpr31-create"),
        Some(create),
    )
    .await;
    assert_eq!(replay_status, StatusCode::OK, "{replay}");
    assert_eq!(replay["change_id"], created["change_id"]);

    let (status, current) = call(
        &world.app,
        "GET",
        &format!("/v1/relaxations/{relaxation}"),
        &world.author,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{current}");
    assert_eq!(current["status"], "active");
    assert_eq!(current["current"]["auto_applied"], true);
    assert!(current["current"]["configuration_version_id"].is_string());
    assert_eq!(
        current["current"]["configuration_hash"]
            .as_str()
            .map(str::len),
        Some(64)
    );

    let mut stale = terms(world.alice_id, "stale revision must not land");
    stale["expected_current_version_id"] = json!(IdentityId::new());
    let (status, body) = call(
        &world.app,
        "PATCH",
        &format!("/v1/relaxations/{relaxation}"),
        &world.author,
        Some("cpr31-stale"),
        Some(stale),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let mut revision = terms(world.alice_id, "continue the bounded investigation");
    revision["expected_current_version_id"] = json!(first_version);
    let (status, revised) = call(
        &world.app,
        "PATCH",
        &format!("/v1/relaxations/{relaxation}"),
        &world.author,
        Some("cpr31-revise"),
        Some(revision),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{revised}");
    assert_eq!(revised["outcome"], "applied");
    assert_eq!(revised["revision"], 2);
    let second_version = revised["version_id"].as_str().expect("second version");
    assert_ne!(first_version, second_version);

    let (status, versions) = call(
        &world.app,
        "GET",
        &format!("/v1/relaxations/{relaxation}/versions"),
        &world.author,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{versions}");
    assert_eq!(versions["versions"].as_array().expect("versions").len(), 2);
    assert_ne!(
        versions["versions"][0]["content_hash"],
        versions["versions"][1]["content_hash"]
    );

    let (status, revoked) = call(
        &world.app,
        "POST",
        &format!("/v1/relaxations/{relaxation}/revoke"),
        &world.author,
        Some("cpr31-revoke"),
        Some(json!({
            "expected_current_version_id": second_version,
            "reason": "incident review complete"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{revoked}");
    assert_eq!(revoked["outcome"], "applied");
    assert_eq!(revoked["revision"], 3);
    for (method, path) in [
        ("GET", "/v1/lapses".to_owned()),
        (
            "POST",
            format!("/v1/proposals/{}/lapse", created["change_id"]),
        ),
    ] {
        let (status, _) = call(&world.app, method, &path, &world.author, None, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "old route survived: {path}");
    }

    let mut tx = tenant_fixture::begin(&world.state.pool, world.tenant).await;
    let opened = sqlx::query_scalar!(
        r#"select count(*) as "count!" from audit_log
           where tenant_id = $1 and action = 'policy.relaxation.change.opened'"#,
        world.tenant.as_uuid(),
    )
    .fetch_one(&mut *tx)
    .await
    .expect("count opened audit events");
    let applied = sqlx::query_scalar!(
        r#"select count(*) as "count!" from audit_log
           where tenant_id = $1 and action = 'policy.relaxation.change.applied'"#,
        world.tenant.as_uuid(),
    )
    .fetch_one(&mut *tx)
    .await
    .expect("count applied audit events");
    assert_eq!((opened, applied), (3, 3));
    let untyped_terminal = sqlx::query_scalar!(
        r#"select count(*) as "count!" from audit_log
           where tenant_id = $1
             and action in ('policy.relaxation.change.applied',
                            'policy.relaxation.change.rejected',
                            'policy.relaxation.expired')
             and not (payload @> '{"artifact_references":[{"family":"policy_relaxation"}]}'::jsonb)"#,
        world.tenant.as_uuid(),
    )
    .fetch_one(&mut *tx)
    .await
    .expect("check terminal relaxation artifact references");
    tx.commit().await.expect("commit Relaxation audit read");
    assert_eq!(
        untyped_terminal, 0,
        "terminal relaxation evidence lost its typed address"
    );
    let retired_table_exists = sqlx::query_scalar!(
        r#"
        select exists (
            select 1
            from information_schema.tables
            where table_schema = 'public' and table_name = $1
        ) as "exists!"
        "#,
        "policy_lapses",
    )
    .fetch_one(&world.state.pool)
    .await
    .expect("inspect retired table");
    assert!(
        !retired_table_exists,
        "retired relaxation table survived the hard cut"
    );
}

#[tokio::test]
async fn standard_profile_returns_pending_and_rejected_without_a_fast_path() {
    let _guard = serial().await;
    let Some(world) = admitted(synveda_policy::STANDARD).await else {
        return;
    };
    let workspace = workspace(&world, "cpr31-standard").await;
    let scope = workspace["scope_id"].as_str().expect("workspace scope");
    let item = create_internal_knowledge(&world, scope, "cpr31-standard-knowledge").await;
    for credential in [&world.alice, &world.bob] {
        let (status, _) = call(
            &world.app,
            "GET",
            &format!("/v1/knowledge/{item}"),
            credential,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    let mut body = terms(world.alice_id, "reviewed support investigation");
    body["target_scope_id"] = json!(scope);
    let (status, pending) = call(
        &world.app,
        "POST",
        "/v1/relaxations",
        &world.author,
        Some("cpr31-pending"),
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{pending}");
    assert_eq!(pending["outcome"], "pending_review", "{pending}");
    let change = pending["change_id"].as_str().expect("change id");
    let relaxation = pending["relaxation_id"].as_str().expect("relaxation id");
    let (status, proposal) = call(
        &world.app,
        "GET",
        &format!("/v1/proposals/{change}"),
        &world.reviewer,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{proposal}");
    assert_eq!(
        proposal["artifact_references"][0]["family"],
        "policy_relaxation"
    );
    assert_eq!(proposal["artifact_references"][0]["operation"], "create");
    assert_eq!(proposal["timeline"][0]["kind"], "opened");
    assert_eq!(proposal["required"]["forbid_author_approval"], true);

    let (status, filtered) = call(
        &world.app,
        "GET",
        "/v1/proposals?state=open&artifact_family=policy_relaxation",
        &world.reviewer,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{filtered}");
    assert!(
        filtered["proposals"]
            .as_array()
            .expect("filtered proposals")
            .iter()
            .any(|listed| listed["id"] == change),
        "typed family filter lost the pending relaxation: {filtered}"
    );

    let (status, stale) = call(
        &world.app,
        "POST",
        &format!("/v1/proposals/{change}/approve"),
        &world.reviewer,
        None,
        Some(json!({"expected_commit": "00".repeat(32)})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{stale}");
    let (status, self_review) = call(
        &world.app,
        "POST",
        &format!("/v1/proposals/{change}/approve"),
        &world.author,
        None,
        Some(json!({"expected_commit": proposal["commit"]})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{self_review}");

    let (status, approved) = call(
        &world.app,
        "POST",
        &format!("/v1/proposals/{change}/approve"),
        &world.reviewer,
        None,
        Some(json!({"expected_commit": proposal["commit"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    let (status, applied) = call(
        &world.app,
        "POST",
        &format!("/v1/proposals/{change}/apply"),
        &world.author,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{applied}");
    assert_eq!(applied["outcome"], "applied");
    let (status, visible) = call(
        &world.app,
        "GET",
        &format!("/v1/knowledge/{item}"),
        &world.alice,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{visible}");
    let (status, _) = call(
        &world.app,
        "GET",
        &format!("/v1/knowledge/{item}"),
        &world.bob,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "subject authority leaked");
    let (status, current) = call(
        &world.app,
        "GET",
        &format!("/v1/relaxations/{relaxation}"),
        &world.author,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{current}");
    assert_eq!(current["current"]["auto_applied"], false);
    assert_eq!(
        current["current"]["approver_ids"]
            .as_array()
            .expect("approvers")
            .len(),
        1
    );

    let mut rejected_body = terms(world.alice_id, "request should be rejected");
    rejected_body["target_scope_id"] = json!(scope);
    let (status, rejected_pending) = call(
        &world.app,
        "POST",
        "/v1/relaxations",
        &world.author,
        Some("cpr31-rejected"),
        Some(rejected_body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{rejected_pending}");
    assert_eq!(rejected_pending["outcome"], "pending_review");
    let rejected_change = rejected_pending["change_id"]
        .as_str()
        .expect("rejected change");
    let rejected_id = rejected_pending["relaxation_id"]
        .as_str()
        .expect("rejected relaxation");
    let (status, proposal) = call(
        &world.app,
        "GET",
        &format!("/v1/proposals/{rejected_change}"),
        &world.reviewer,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{proposal}");
    let (status, rejected) = call(
        &world.app,
        "POST",
        &format!("/v1/proposals/{rejected_change}/reject"),
        &world.reviewer,
        None,
        Some(json!({
            "expected_commit": proposal["commit"],
            "reason": "scope is broader than the incident requires"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rejected}");
    let (status, replay) = call(
        &world.app,
        "POST",
        "/v1/relaxations",
        &world.author,
        Some("cpr31-rejected"),
        Some(rejected_body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["outcome"], "rejected");
    let (status, _) = call(
        &world.app,
        "GET",
        &format!("/v1/relaxations/{rejected_id}"),
        &world.author,
        None,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "rejected change created an aggregate"
    );

    let mut cancelled_body = terms(world.alice_id, "author cancels a mistaken request");
    cancelled_body["target_scope_id"] = json!(scope);
    let (status, cancelled_pending) = call(
        &world.app,
        "POST",
        "/v1/relaxations",
        &world.author,
        Some("cpr32-cancelled"),
        Some(cancelled_body),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{cancelled_pending}");
    let cancelled_change = cancelled_pending["change_id"]
        .as_str()
        .expect("cancelled change");
    let (status, cancelled) = call(
        &world.app,
        "POST",
        &format!("/v1/proposals/{cancelled_change}/withdraw"),
        &world.author,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cancelled}");
    assert_eq!(cancelled["state"], "withdrawn");
    let (status, cancelled_detail) = call(
        &world.app,
        "GET",
        &format!("/v1/proposals/{cancelled_change}"),
        &world.author,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cancelled_detail}");
    assert_eq!(cancelled_detail["timeline"][1]["kind"], "withdrawn");

    let mut tx = tenant_fixture::begin(&world.state.pool, world.tenant).await;
    let changes = sqlx::query_scalar!(
        r#"select count(*) as "count!" from policy_relaxation_changes
           where tenant_id = $1"#,
        world.tenant.as_uuid(),
    )
    .fetch_one(&mut *tx)
    .await
    .expect("count typed changes");
    assert_eq!(
        changes, 3,
        "applied, rejected and cancelled outcomes retained typed VedaFlow commands"
    );
    let versions = sqlx::query_scalar!(
        r#"select count(*) as "count!" from policy_relaxation_versions
           where tenant_id = $1"#,
        world.tenant.as_uuid(),
    )
    .fetch_one(&mut *tx)
    .await
    .expect("count applied versions");
    tx.commit().await.expect("commit Relaxation evidence read");
    assert_eq!(versions, 1, "a rejected command published no version");
    assert_ne!(world.root.to_string(), scope);
}

#[tokio::test]
async fn regulated_profile_separates_author_reviewers_and_effect_actor() {
    let _guard = serial().await;
    let Some(world) = admitted(synveda_policy::REGULATED_STRICT).await else {
        return;
    };
    let workspace = workspace(&world, "cpr32-separated").await;
    let scope = workspace["scope_id"].as_str().expect("workspace scope");
    let mut body = terms(world.alice_id, "regulated separation acceptance");
    body["target_scope_id"] = json!(scope);
    let (status, pending) = call(
        &world.app,
        "POST",
        "/v1/relaxations",
        &world.author,
        Some("cpr32-separated"),
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{pending}");
    assert_eq!(pending["outcome"], "pending_review");
    let change = pending["change_id"].as_str().expect("change id");
    let (status, proposal) = call(
        &world.app,
        "GET",
        &format!("/v1/proposals/{change}"),
        &world.reviewer,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{proposal}");
    assert_eq!(proposal["required"]["forbid_author_approval"], true);
    assert_eq!(proposal["required"]["separate_effect_actor"], true);

    for credential in [&world.reviewer, &world.reviewer_two] {
        let (status, reviewed) = call(
            &world.app,
            "POST",
            &format!("/v1/proposals/{change}/approve"),
            credential,
            None,
            Some(json!({"expected_commit": proposal["commit"]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{reviewed}");
    }

    for credential in [&world.author, &world.reviewer] {
        let (status, refused) = call(
            &world.app,
            "POST",
            &format!("/v1/proposals/{change}/apply"),
            credential,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    }
    let (status, applied) = call(
        &world.app,
        "POST",
        &format!("/v1/proposals/{change}/apply"),
        &world.executor,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{applied}");
    assert_eq!(applied["outcome"], "applied");
}
