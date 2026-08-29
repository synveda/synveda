//! CPR-20 acceptance evidence for explainable Knowledge-backed context.
//!
//! These tests use the public Knowledge, session, context and query surfaces.
//! Fixture bootstrap creates only tenants, identities and grants; no test
//! writes planner rows or active Knowledge behind the application layer.

#[path = "../../synveda-store/tests/support/tenant_fixture.rs"]
mod tenant_fixture;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

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
use synveda_policy::Pdp;
use synveda_store::{access, identities, knowledge_conflicts, rls, scopes};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::knowledge::ConflictClassification;
use synveda_types::{
    CompositionConfig, ConflictSetId, GrantId, IdentityId, IdentityKind, PackConfig, ProjectId,
    ScopeId, TenantId, TenantStatus, TraceRetentionMode,
};
use tower::ServiceExt;

#[path = "support/configuration.rs"]
mod configuration_support;

const SECRET: &[u8] = b"cpr-20-context-planning";
const ALICE: &str = "alice-cpr20@pulseboard.test";
const BOB: &str = "bob-cpr20@pulseboard.test";
const MALLORY: &str = "mallory-cpr20@pulseboard.test";
const BLANKET: &str = "permit (principal, action, resource) when { resource in principal.tenant };";

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
            .acquire_timeout(Duration::from_secs(5))
            .connect_lazy(url)
            .expect("parse database url"),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp: Arc::new(Pdp::new().expect("build embedded PDP")),
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
    role_key: RoleKey,
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
            role_key,
            source: GrantSource::Automation,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .expect("seed grant");
}

struct World {
    database_url: String,
    state: AppState,
    app: Router,
    tenant_id: TenantId,
    alice_scope: ScopeId,
    alice_token: String,
    bob_token: String,
    mallory_token: String,
    workspace_id: String,
    workspace_scope: ScopeId,
    project_id: String,
    project_scope: ScopeId,
    alice_session: String,
    bob_session: String,
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
            .expect("build request")
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
        serde_json::from_slice(&bytes).unwrap_or_else(|_| {
            panic!(
                "response should be JSON: {}",
                String::from_utf8_lossy(&bytes)
            )
        })
    };
    (status, value)
}

async fn admitted_world() -> Option<World> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping CPR-20 context tests: DATABASE_URL is not set \
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
    synveda_store::epoch::verify(&pool)
        .await
        .expect("apply migrations");
    let tenant_id = TenantId::new();
    tenant_fixture::create(
        &pool,
        tenant_id,
        &format!("cpr20-{}", tenant_id.as_uuid().simple()),
        "CPR-20 context planning",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    let mut tx = rls::begin_tenant_tx(&pool, tenant_id)
        .await
        .expect("begin bootstrap");
    let root = scopes::ensure_tenant_root(&mut tx, tenant_id)
        .await
        .expect("mint tenant root");
    let alice_scope = seed_identity(&mut tx, tenant_id, ALICE).await;
    seed_identity(&mut tx, tenant_id, BOB).await;
    seed_identity(&mut tx, tenant_id, MALLORY).await;
    seed_grant(&mut tx, tenant_id, root.id, ALICE, RoleKey::Administrator).await;
    configuration_support::bind_pack(&mut tx, tenant_id, root.id, synveda_policy::STANDARD).await;
    tx.commit().await.expect("commit bootstrap");

    let state = state(&url);
    let app = router(state.clone());
    let verifier = Hs256Verifier::new(SECRET);
    let alice_token = verifier.issue(ALICE, tenant_id, Duration::from_secs(300));
    let bob_token = verifier.issue(BOB, tenant_id, Duration::from_secs(300));
    let mallory_token = verifier.issue(MALLORY, tenant_id, Duration::from_secs(300));

    let (status, workspace) = call(
        &app,
        "POST",
        "/v1/workspaces",
        &alice_token,
        Some("cpr20-workspace"),
        Some(json!({"slug": "pulseboard", "display_name": "PulseBoard"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workspace}");
    let workspace_id = workspace["id"].as_str().expect("workspace id").to_owned();
    let workspace_scope: ScopeId = workspace["scope_id"]
        .as_str()
        .expect("workspace scope")
        .parse()
        .expect("parse workspace scope");
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin member grant");
    seed_grant(&mut tx, tenant_id, workspace_scope, BOB, RoleKey::Member).await;
    tx.commit().await.expect("commit member grant");

    let (status, project) = call(
        &app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/projects"),
        &alice_token,
        Some("cpr20-project"),
        Some(json!({"slug": "api", "display_name": "PulseBoard API"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().expect("project id").to_owned();
    let project_scope: ScopeId = project["scope_id"]
        .as_str()
        .expect("project scope")
        .parse()
        .expect("parse project scope");

    let open = |subject: &str, token: &str, key: &str| {
        let app = app.clone();
        let workspace_id = workspace_id.clone();
        let project_id = project_id.clone();
        let token = token.to_owned();
        let subject = subject.to_owned();
        let key = key.to_owned();
        async move {
            let (status, session) = call(
                &app,
                "POST",
                "/v1/sessions",
                &token,
                Some(&key),
                Some(json!({
                    "workspace_id": workspace_id,
                    "project_id": project_id,
                    "client_name": "cpr20-test",
                    "external_session_id": format!("{subject}-{key}"),
                    "task_summary": "Explain project Knowledge"
                })),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{session}");
            session["id"].as_str().expect("session id").to_owned()
        }
    };
    let alice_session = open(ALICE, &alice_token, "cpr20-alice-session").await;
    let bob_session = open(BOB, &bob_token, "cpr20-bob-session").await;

    Some(World {
        database_url: url,
        state,
        app,
        tenant_id,
        alice_scope,
        alice_token,
        bob_token,
        mallory_token,
        workspace_id,
        workspace_scope,
        project_id,
        project_scope,
        alice_session,
        bob_session,
    })
}

fn replace_gateway_pool(world: &mut World, max_connections: u32) {
    let mut state = world.state.clone();
    state.pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(5))
        .connect_lazy(&world.database_url)
        .expect("parse database url");
    world.app = router(state.clone());
    world.state = state;
}

fn content(title: &str, body: &str, stale_after: Option<&str>) -> Value {
    let mut value = json!({
        "title": title,
        "body_markdown": body,
        "summary": body,
        "tags": ["pulseboard", "context"],
        "sensitivity": "internal",
        "confidence_permille": 950,
        "verification_metadata": {},
        "metadata": {"fixture": "CPR-20"}
    });
    if let Some(at) = stale_after {
        value["valid_from"] = json!("2019-01-01T00:00:00Z");
        value["stale_after"] = json!(at);
    }
    value
}

#[allow(clippy::too_many_arguments)]
async fn create_knowledge(
    world: &World,
    key: &str,
    scope_id: ScopeId,
    project_id: Option<&str>,
    owner: Option<&str>,
    title: &str,
    body: &str,
    stale_after: Option<&str>,
) -> (String, String) {
    let (status, created) = call(
        &world.app,
        "POST",
        "/v1/knowledge",
        &world.alice_token,
        Some(key),
        Some(json!({
            "scope_id": scope_id,
            "project_id": project_id,
            "owner_principal_id": owner,
            "knowledge_type": "convention",
            "origin": "authored",
            "content": content(title, body, stale_after),
            "sources": [{
                "scope_id": scope_id,
                "source_type": "manual",
                "metadata": {"fixture": "CPR-20"}
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["outcome"], "applied", "{created}");
    (
        created["knowledge_item_id"]
            .as_str()
            .expect("Knowledge item id")
            .to_owned(),
        created["revision_id"]
            .as_str()
            .expect("revision id")
            .to_owned(),
    )
}

async fn support_relation(
    world: &World,
    key: &str,
    challenger: (&str, &str),
    current: (&str, &str),
) -> ConflictSetId {
    let conflict_id = ConflictSetId::new();
    let project_id: ProjectId = world.project_id.parse().expect("project id");
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .expect("begin controlled conflict evidence fixture");
    knowledge_conflicts::create(
        &mut tx,
        &knowledge_conflicts::NewConflictSet {
            id: conflict_id,
            tenant_id: world.tenant_id,
            scope_id: world.project_scope,
            project_id: Some(project_id),
            classification: ConflictClassification::Support,
            challenger_item_id: Some(challenger.0.parse().expect("challenger item")),
            challenger_revision_id: Some(challenger.1.parse().expect("challenger revision")),
            capture_candidate_id: None,
            matches: &[knowledge_conflicts::MatchedRevision {
                item_id: current.0.parse().expect("current item"),
                revision_id: current.1.parse().expect("current revision"),
                classification: ConflictClassification::Support,
                similarity_permille: 700,
                reason_code: "bounded_graph_fixture".to_owned(),
            }],
            created_by: ALICE,
        },
    )
    .await
    .expect("create controlled conflict evidence");
    tx.commit().await.expect("commit conflict evidence");
    let (status, result) = call(
        &world.app,
        "POST",
        &format!("/v1/knowledge-conflicts/{conflict_id}/resolve"),
        &world.alice_token,
        Some(key),
        Some(json!({
            "expected_revision": 1,
            "resolution": "support",
            "reason": "CPR-38 governed graph fixture"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{result}");
    assert_eq!(result["outcome"], "applied", "{result}");
    conflict_id
}

struct Corpus {
    active_id: String,
    active_revision: String,
    stale_id: String,
    superseded_id: String,
    private_id: String,
    private_revision: String,
}

async fn corpus(world: &World) -> Corpus {
    let (active_id, active_revision) = create_knowledge(
        world,
        "cpr20-active",
        world.project_scope,
        Some(&world.project_id),
        None,
        "Current correlation convention",
        "PulseBoard correlation header uses traceparent on public requests.",
        None,
    )
    .await;
    let (stale_id, _) = create_knowledge(
        world,
        "cpr20-stale",
        world.project_scope,
        Some(&world.project_id),
        None,
        "Stale correlation note",
        "PulseBoard correlation once used an unverified experimental header.",
        Some("2020-01-01T00:00:00Z"),
    )
    .await;
    let (superseded_id, superseded_revision) = create_knowledge(
        world,
        "cpr20-superseded-source",
        world.project_scope,
        Some(&world.project_id),
        None,
        "Obsolete correlation header convention",
        "PulseBoard correlation header uses X-Request-Id.",
        None,
    )
    .await;
    let (status, replacement) = call(
        &world.app,
        "POST",
        &format!("/v1/knowledge/{superseded_id}/supersede"),
        &world.alice_token,
        Some("cpr20-supersede"),
        Some(json!({
            "expected_revision_id": superseded_revision,
            "scope_id": world.project_scope,
            "project_id": world.project_id,
            "knowledge_type": "convention",
            "origin": "authored",
            "content": content(
                "Replacement correlation convention",
                "PulseBoard correlation uses the W3C traceparent header.",
                None
            )
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{replacement}");
    assert_eq!(replacement["outcome"], "applied", "{replacement}");

    let (private_id, private_revision) = create_knowledge(
        world,
        "cpr20-private",
        world.alice_scope,
        None,
        Some(ALICE),
        "Private quick-test command",
        "My private quick-test command is just test-fast-secret.",
        None,
    )
    .await;
    Corpus {
        active_id,
        active_revision,
        stale_id,
        superseded_id,
        private_id,
        private_revision,
    }
}

async fn context_run(
    world: &World,
    token: &str,
    session: &str,
    key: &str,
    query: &str,
    budget: Option<u32>,
) -> Value {
    let mut body = json!({"query": query});
    if let Some(budget) = budget {
        body["budget_tokens"] = json!(budget);
    }
    let (status, run) = call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{session}/context-runs"),
        token,
        Some(key),
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    run
}

async fn detail(world: &World, token: &str, run_id: &str) -> Value {
    let (status, value) = call(
        &world.app,
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

async fn audit_events(world: &World) -> Vec<synveda_audit::StoredEvent> {
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .expect("begin audit read");
    let mut events = synveda_audit::tail(&mut tx, world.tenant_id, 500)
        .await
        .expect("read audit chain");
    tx.commit().await.expect("commit audit read");
    events.reverse();
    events
}

async fn set_trace_mode(world: &World, mode: TraceRetentionMode) {
    let name = format!("cpr20-{}", mode.as_str().replace('_', "-"));
    world
        .state
        .pdp
        .install_source(
            world.tenant_id,
            &name,
            1,
            BLANKET,
            PackConfig {
                composition: Some(CompositionConfig {
                    trace_retention: mode,
                    ..CompositionConfig::DEFAULT
                }),
                ..PackConfig::default()
            },
        )
        .expect("install trace-mode pack");
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .expect("begin trace-mode Configuration");
    configuration_support::bind_tenant_pack(&mut tx, world.tenant_id, &name).await;
    let root = scopes::tenant_root(&mut *tx, world.tenant_id)
        .await
        .expect("read trace-mode root")
        .expect("trace-mode root exists");
    configuration_support::set_trace_retention(&mut tx, world.tenant_id, root.id, mode).await;
    tx.commit().await.expect("commit trace-mode Configuration");
}

#[tokio::test]
async fn planner_selects_only_current_knowledge_and_feedback_names_one_revision() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let corpus = corpus(&world).await;
    let run = context_run(
        &world,
        &world.alice_token,
        &world.alice_session,
        "cpr20-current-run",
        "PulseBoard correlation header",
        None,
    )
    .await;
    let rendered = run["rendered"].as_str().expect("delivered block");
    assert!(rendered.contains("traceparent"), "{rendered}");
    assert!(!rendered.contains("experimental header"), "{rendered}");
    assert!(!rendered.contains("X-Request-Id"), "{rendered}");
    assert_eq!(run["retrieval_version"], "knowledge-planner-v2");
    assert_eq!(run["index_version"], "knowledge-search-v1");
    assert_eq!(run["graph_version"], "knowledge-relations-v1");

    let run_id = run["id"].as_str().expect("run id");
    let before = detail(&world, &world.alice_token, run_id).await;
    assert_eq!(before["feedback"], json!([]));
    let candidates = before["candidates"].as_array().expect("candidates");
    let stale = candidates
        .iter()
        .find(|entry| entry["knowledge_item_id"] == corpus.stale_id)
        .expect("stale candidate retained");
    assert_eq!(stale["exclusion_reason"], "stale");
    let superseded = candidates
        .iter()
        .find(|entry| entry["knowledge_item_id"] == corpus.superseded_id)
        .expect("superseded candidate retained");
    assert_eq!(superseded["exclusion_reason"], "superseded");
    let selections = before["selections"].as_array().expect("selections");
    assert!(
        selections
            .iter()
            .any(|entry| entry["knowledge_item_id"] == corpus.active_id),
        "current Knowledge was not selected: {before}"
    );
    assert!(selections.iter().all(|entry| {
        entry["knowledge_item_id"] != corpus.stale_id
            && entry["knowledge_item_id"] != corpus.superseded_id
    }));
    let selected = selections
        .iter()
        .find(|entry| entry["knowledge_revision_id"] == corpus.active_revision)
        .expect("active exact revision selected");
    let selection_id = selected["id"].as_str().expect("selection id");

    let (timeline_status, timeline) = call(
        &world.app,
        "GET",
        &format!("/v1/sessions/{}/timeline", world.alice_session),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(timeline_status, StatusCode::OK, "{timeline}");
    let timeline_run = timeline["entries"]
        .as_array()
        .expect("timeline entries")
        .iter()
        .find(|entry| entry["kind"] == "context_run" && entry["id"] == run_id)
        .expect("this context run on its session timeline");
    let noun = if selections.len() == 1 {
        "knowledge item"
    } else {
        "knowledge items"
    };
    assert_eq!(
        timeline_run["summary"],
        format!("Synveda supplied {} {noun}", selections.len())
    );

    let feedback_body = json!({
        "context_selection_id": selection_id,
        "knowledge_revision_id": corpus.active_revision,
        "feedback_type": "referenced_by_agent"
    });
    let (created, feedback) = call(
        &world.app,
        "POST",
        &format!("/v1/context-runs/{run_id}/feedback"),
        &world.alice_token,
        Some("cpr20-feedback"),
        Some(feedback_body.clone()),
    )
    .await;
    assert_eq!(created, StatusCode::CREATED, "{feedback}");
    assert_eq!(feedback["knowledge_revision_id"], corpus.active_revision);
    let (replay, replayed) = call(
        &world.app,
        "POST",
        &format!("/v1/context-runs/{run_id}/feedback"),
        &world.alice_token,
        Some("cpr20-feedback"),
        Some(feedback_body),
    )
    .await;
    assert_eq!(replay, StatusCode::OK, "{replayed}");
    assert_eq!(replayed["id"], feedback["id"]);

    let events = audit_events(&world).await;
    let run_events: Vec<_> = events
        .iter()
        .filter(|event| event.payload["context_run_id"].as_str() == Some(run_id))
        .collect();
    let actions: Vec<&str> = run_events
        .iter()
        .map(|event| event.action.as_str())
        .collect();
    for expected in [
        "context.candidates.retrieved",
        "context.selections.made",
        "session.context.composed",
        "context.feedback.recorded",
    ] {
        assert!(
            actions.contains(&expected),
            "missing {expected}: {actions:?}"
        );
    }
    let delivered = run_events
        .iter()
        .find(|event| event.action == "session.context.composed")
        .expect("delivery event");
    assert!(
        delivered.payload["knowledge"]
            .as_array()
            .expect("Knowledge refs")
            .iter()
            .any(|entry| {
                entry["knowledge_item_id"] == corpus.active_id
                    && entry["knowledge_revision_id"] == corpus.active_revision
                    && entry["content_hash"].is_string()
            }),
        "delivery names the immutable selection: {}",
        delivered.payload
    );
    let audited = run_events
        .iter()
        .map(|event| event.payload.to_string())
        .collect::<String>();
    for secret in [
        "PulseBoard correlation header uses traceparent",
        "PulseBoard correlation header",
        "Current correlation convention",
    ] {
        assert!(
            !audited.contains(secret),
            "context audit metadata contains task or Knowledge content: {audited}"
        );
    }

    let (wrong_status, _) = call(
        &world.app,
        "POST",
        &format!("/v1/context-runs/{run_id}/feedback"),
        &world.alice_token,
        Some("cpr20-feedback-wrong-revision"),
        Some(json!({
            "context_selection_id": selection_id,
            "knowledge_revision_id": synveda_types::KnowledgeRevisionId::new(),
            "feedback_type": "helpful"
        })),
    )
    .await;
    assert_eq!(wrong_status, StatusCode::NOT_FOUND);

    let budgeted = context_run(
        &world,
        &world.alice_token,
        &world.alice_session,
        "cpr20-budget-run",
        "PulseBoard correlation header",
        Some(1),
    )
    .await;
    assert_eq!(budgeted["selection_count"], 0);
    let budget_detail = detail(
        &world,
        &world.alice_token,
        budgeted["id"].as_str().expect("budget run id"),
    )
    .await;
    assert!(
        budget_detail["candidates"]
            .as_array()
            .expect("budget candidates")
            .iter()
            .any(|entry| entry["exclusion_reason"] == "token_budget"),
        "token-budget exclusion is not explainable: {budget_detail}"
    );
}

#[tokio::test]
async fn bounded_graph_improves_two_hop_recall_and_denied_endpoints_leave_no_trace() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let (anchor_id, anchor_revision) = create_knowledge(
        &world,
        "cpr38-anchor",
        world.project_scope,
        Some(&world.project_id),
        None,
        "Quasar Almanac release pointer",
        "The unique quasar-almanac request is governed by the release playbook.",
        None,
    )
    .await;
    let (middle_id, middle_revision) = create_knowledge(
        &world,
        "cpr38-middle",
        world.project_scope,
        Some(&world.project_id),
        None,
        "Release playbook owner",
        "The release playbook delegates the final verification to the deployment checklist.",
        None,
    )
    .await;
    let (answer_id, answer_revision) = create_knowledge(
        &world,
        "cpr38-answer",
        world.project_scope,
        Some(&world.project_id),
        None,
        "Deployment checklist verification",
        "The final verification command is cargo test --workspace --locked.",
        None,
    )
    .await;
    support_relation(
        &world,
        "cpr38-middle-supports-answer",
        (&middle_id, &middle_revision),
        (&answer_id, &answer_revision),
    )
    .await;
    support_relation(
        &world,
        "cpr38-anchor-supports-middle",
        (&anchor_id, &anchor_revision),
        (&middle_id, &middle_revision),
    )
    .await;

    let root = {
        let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
            .await
            .expect("begin graph configuration");
        let root = scopes::tenant_root(&mut *tx, world.tenant_id)
            .await
            .expect("read tenant root")
            .expect("tenant root");
        configuration_support::set_graph_enabled(&mut tx, world.tenant_id, root.id, false).await;
        tx.commit()
            .await
            .expect("disable graph through Configuration");
        root.id
    };
    let baseline = context_run(
        &world,
        &world.alice_token,
        &world.alice_session,
        "cpr38-vector-only",
        "quasar-almanac",
        None,
    )
    .await;
    assert_eq!(baseline["graph_version"], Value::Null, "{baseline}");
    assert!(
        !baseline["rendered"]
            .as_str()
            .expect("baseline rendered")
            .contains("cargo test --workspace --locked"),
        "the enumeration-free baseline unexpectedly found the two-hop answer: {baseline}"
    );

    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .expect("begin graph enable");
    configuration_support::set_graph_enabled(&mut tx, world.tenant_id, root, true).await;
    tx.commit()
        .await
        .expect("enable graph through Configuration");
    let expanded = context_run(
        &world,
        &world.alice_token,
        &world.alice_session,
        "cpr38-two-hop",
        "quasar-almanac",
        None,
    )
    .await;
    assert_eq!(expanded["graph_version"], "knowledge-relations-v1");
    assert!(
        expanded["rendered"]
            .as_str()
            .expect("expanded rendered")
            .contains("cargo test --workspace --locked"),
        "two-hop Knowledge answer was not delivered: {expanded}"
    );
    let expanded_detail = detail(
        &world,
        &world.alice_token,
        expanded["id"].as_str().expect("expanded run id"),
    )
    .await;
    let answer = expanded_detail["candidates"]
        .as_array()
        .expect("expanded candidates")
        .iter()
        .find(|candidate| candidate["knowledge_item_id"] == answer_id)
        .expect("two-hop answer candidate");
    assert_eq!(answer["reason_codes"][0], "graph_expansion", "{answer}");
    assert_eq!(answer["graph_path"].as_array().expect("path").len(), 2);
    assert_eq!(answer["graph_path"][0]["relation_type"], "supports");
    assert_eq!(answer["graph_path"][1]["relation_type"], "supports");
    assert_eq!(answer["scores"]["edge_weight_micros"], 1_400_000);
    assert_eq!(answer["scores"]["hop_penalty_micros"], 200_000);
    assert!(
        expanded_detail["selections"]
            .as_array()
            .expect("expanded selections")
            .iter()
            .any(|selection| {
                selection["knowledge_item_id"] == answer_id
                    && selection["graph_path"]
                        .as_array()
                        .is_some_and(|path| path.len() == 2)
            })
    );

    let (private_id, private_revision) = create_knowledge(
        &world,
        "cpr38-private-endpoint",
        world.alice_scope,
        None,
        Some(ALICE),
        "Private graph endpoint",
        "The private graph secret is nebula-seven.",
        None,
    )
    .await;
    support_relation(
        &world,
        "cpr38-anchor-supports-private",
        (&anchor_id, &anchor_revision),
        (&private_id, &private_revision),
    )
    .await;
    let bob_run = context_run(
        &world,
        &world.bob_token,
        &world.bob_session,
        "cpr38-denied-endpoint",
        "quasar-almanac",
        None,
    )
    .await;
    let bob_detail = detail(
        &world,
        &world.bob_token,
        bob_run["id"].as_str().expect("Bob run id"),
    )
    .await;
    let disclosure = bob_detail.to_string();
    for denied in [&private_id, &private_revision, "nebula-seven"] {
        assert!(
            !disclosure.contains(denied),
            "denied graph endpoint leaked: {disclosure}"
        );
    }
    assert!(bob_detail["policy_exclusion_message"].is_string());
    assert!(
        bob_detail["candidates"]
            .as_array()
            .expect("Bob visible candidates")
            .iter()
            .all(|candidate| candidate["knowledge_item_id"] != private_id)
    );

    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .expect("begin hashes-only graph trace");
    configuration_support::set_trace_retention(
        &mut tx,
        world.tenant_id,
        root,
        TraceRetentionMode::HashesOnly,
    )
    .await;
    tx.commit().await.expect("set hashes-only graph trace");
    let hashes = context_run(
        &world,
        &world.alice_token,
        &world.alice_session,
        "cpr38-hashes-only",
        "quasar-almanac",
        None,
    )
    .await;
    let hashes_detail = detail(
        &world,
        &world.alice_token,
        hashes["id"].as_str().expect("hash run id"),
    )
    .await;
    let hashed_path = hashes_detail["candidates"]
        .as_array()
        .expect("hash candidates")
        .iter()
        .find(|candidate| {
            candidate["graph_path"]
                .as_array()
                .is_some_and(|path| path.len() == 2)
        })
        .and_then(|candidate| candidate["graph_path"].as_array())
        .expect("hash-only two-hop path");
    assert!(hashed_path.iter().all(|step| {
        step["relation_id"].is_null()
            && step["from_item_id"].is_null()
            && step["to_item_id"].is_null()
            && step["relation_hash"]
                .as_str()
                .is_some_and(|hash| hash.len() == 64)
    }));
}

#[tokio::test]
async fn graph_expansion_reuses_the_planner_connection_when_the_pool_has_one_slot() {
    let _guard = serial().await;
    let Some(mut world) = admitted_world().await else {
        return;
    };
    let (anchor_id, anchor_revision) = create_knowledge(
        &world,
        "cpr44-single-pool-anchor",
        world.project_scope,
        Some(&world.project_id),
        None,
        "Single-pool comet pointer",
        "The unique single-pool-comet request points to a governed checklist.",
        None,
    )
    .await;
    let (answer_id, answer_revision) = create_knowledge(
        &world,
        "cpr44-single-pool-answer",
        world.project_scope,
        Some(&world.project_id),
        None,
        "Governed checklist response",
        "The checklist answer is connection-snapshot-preserved.",
        None,
    )
    .await;
    support_relation(
        &world,
        "cpr44-single-pool-relation",
        (&anchor_id, &anchor_revision),
        (&answer_id, &answer_revision),
    )
    .await;

    replace_gateway_pool(&mut world, 1);
    let run = tokio::time::timeout(
        Duration::from_secs(5),
        context_run(
            &world,
            &world.alice_token,
            &world.alice_session,
            "cpr44-single-pool-run",
            "single-pool-comet",
            None,
        ),
    )
    .await
    .expect("one-connection graph planning completes");
    assert_eq!(run["graph_version"], "knowledge-relations-v1", "{run}");
    assert!(
        run["rendered"]
            .as_str()
            .expect("rendered context")
            .contains("connection-snapshot-preserved"),
        "the graph answer was lost to a nested pool wait: {run}"
    );
    assert!(
        run["degraded"]
            .as_array()
            .expect("degradation list")
            .iter()
            .all(|value| { value != "graph_time_budget_exceeded" && value != "graph_unavailable" }),
        "single-connection graph planning degraded: {run}"
    );
    let run_detail = detail(
        &world,
        &world.alice_token,
        run["id"].as_str().expect("context run id"),
    )
    .await;
    let answer = run_detail["candidates"]
        .as_array()
        .expect("context candidates")
        .iter()
        .find(|candidate| candidate["knowledge_item_id"] == answer_id)
        .expect("graph-expanded answer candidate");
    assert_eq!(
        answer["graph_path"].as_array().expect("graph path").len(),
        1,
        "{answer}"
    );
}

#[tokio::test]
async fn graph_time_budget_degrades_to_anchor_results_and_the_planner_transaction_continues() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    create_knowledge(
        &world,
        "cpr44-time-budget-anchor",
        world.project_scope,
        Some(&world.project_id),
        None,
        "Graph timeout fallback marker",
        "The lexical anchor remains available as timeout-fallback-anchor.",
        None,
    )
    .await;
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .expect("begin graph time-budget Configuration");
    let root = scopes::tenant_root(&mut *tx, world.tenant_id)
        .await
        .expect("read tenant root")
        .expect("tenant root");
    configuration_support::set_graph_time_budget(&mut tx, world.tenant_id, root.id, 1).await;
    tx.commit()
        .await
        .expect("commit graph time-budget Configuration");

    let run = tokio::time::timeout(
        Duration::from_secs(5),
        context_run(
            &world,
            &world.alice_token,
            &world.alice_session,
            "cpr44-time-budget-run",
            "timeout-fallback-anchor",
            None,
        ),
    )
    .await
    .expect("timed-out graph stage rolls back its savepoint");
    assert!(
        run["degraded"]
            .as_array()
            .expect("degradation list")
            .iter()
            .any(|value| value == "graph_time_budget_exceeded"),
        "the graph timeout was not reported: {run}"
    );
    assert!(
        run["rendered"]
            .as_str()
            .expect("rendered context")
            .contains("timeout-fallback-anchor"),
        "the lexical fallback disappeared after graph timeout: {run}"
    );
    detail(
        &world,
        &world.alice_token,
        run["id"].as_str().expect("persisted context run id"),
    )
    .await;
}

#[tokio::test]
async fn denied_private_knowledge_leaks_no_address_content_count_or_block_fingerprint() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let corpus = corpus(&world).await;
    let run = context_run(
        &world,
        &world.alice_token,
        &world.alice_session,
        "cpr20-private-run",
        "private quick-test command",
        None,
    )
    .await;
    assert!(
        run["rendered"]
            .as_str()
            .expect("owner delivery")
            .contains("test-fast-secret")
    );
    let run_id = run["id"].as_str().expect("run id");

    let bob_detail = detail(&world, &world.bob_token, run_id).await;
    let disclosure = bob_detail.to_string();
    assert!(!disclosure.contains(&corpus.private_id), "{disclosure}");
    assert!(
        !disclosure.contains(&corpus.private_revision),
        "{disclosure}"
    );
    assert!(!disclosure.contains("test-fast-secret"), "{disclosure}");
    assert_eq!(bob_detail["run"]["rendered"], Value::Null);
    assert_eq!(bob_detail["run"]["candidate_count"], 0);
    assert_eq!(bob_detail["run"]["selection_count"], 0);
    assert_eq!(bob_detail["run"]["entry_count"], 0);
    assert_eq!(bob_detail["run"]["tokens"], 0);
    assert_eq!(
        bob_detail["run"]["block_hash"],
        blake3::hash(b"").to_hex().to_string()
    );
    assert!(bob_detail["policy_exclusion_message"].is_string());

    let (timeline_status, timeline) = call(
        &world.app,
        "GET",
        &format!("/v1/sessions/{}/timeline", world.alice_session),
        &world.bob_token,
        None,
        None,
    )
    .await;
    assert_eq!(timeline_status, StatusCode::OK, "{timeline}");
    let timeline_run = timeline["entries"]
        .as_array()
        .expect("timeline entries")
        .iter()
        .find(|entry| entry["kind"] == "context_run" && entry["id"] == run_id)
        .expect("shared session run is timeline-visible");
    assert_eq!(
        timeline_run["summary"],
        "Synveda supplied 0 knowledge items. Some context detail is unavailable under current policy."
    );
    let timeline_disclosure = timeline_run.to_string();
    assert!(!timeline_disclosure.contains(&corpus.private_id));
    assert!(!timeline_disclosure.contains(&corpus.private_revision));
    assert!(!timeline_disclosure.contains("test-fast-secret"));

    let (list_status, listing) = call(
        &world.app,
        "GET",
        "/v1/context-runs?limit=100",
        &world.bob_token,
        None,
        None,
    )
    .await;
    assert_eq!(list_status, StatusCode::OK, "{listing}");
    let listed = listing["runs"]
        .as_array()
        .expect("runs")
        .iter()
        .find(|entry| entry["id"] == run_id)
        .expect("shared session run is list-visible");
    assert_eq!(listed["candidate_count"], 0);
    assert_eq!(listed["selection_count"], 0);
    assert_eq!(listed["entry_count"], 0);
    assert_eq!(listed["tokens"], 0);
    assert_eq!(listed["rendered"], Value::Null);
    assert!(!listing.to_string().contains("test-fast-secret"));

    let (query_status, query) = call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{}/knowledge-query", world.bob_session),
        &world.bob_token,
        None,
        Some(json!({"query": "private quick-test command", "limit": 20})),
    )
    .await;
    assert_eq!(query_status, StatusCode::OK, "{query}");
    assert_eq!(query["items"], json!([]), "{query}");

    let (mallory_status, _) = call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{}/knowledge-query", world.alice_session),
        &world.mallory_token,
        None,
        Some(json!({"query": "correlation"})),
    )
    .await;
    assert_eq!(mallory_status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn retention_modes_and_diagnostic_query_have_distinct_disclosure() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let corpus = corpus(&world).await;

    for mode in [
        TraceRetentionMode::Full,
        TraceRetentionMode::Redacted,
        TraceRetentionMode::HashesOnly,
        TraceRetentionMode::Disabled,
    ] {
        set_trace_mode(&world, mode).await;
        let run = context_run(
            &world,
            &world.alice_token,
            &world.alice_session,
            &format!("cpr20-mode-{}", mode.as_str()),
            "PulseBoard correlation header",
            None,
        )
        .await;
        let value = detail(
            &world,
            &world.alice_token,
            run["id"].as_str().expect("run id"),
        )
        .await;
        assert_eq!(value["run"]["trace_retention_mode"], mode.as_str());
        match mode {
            TraceRetentionMode::Full => {
                assert!(value["run"]["rendered"].is_string());
                assert!(value["run"]["query"].is_string());
                assert!(value["selections"][0]["revision"].is_object());
                assert!(value["selections"][0]["knowledge_revision_id"].is_string());
            }
            TraceRetentionMode::Redacted => {
                assert_eq!(value["run"]["rendered"], Value::Null);
                assert_eq!(value["run"]["query"], Value::Null);
                assert_eq!(value["selections"][0]["revision"], Value::Null);
                assert!(value["selections"][0]["knowledge_revision_id"].is_string());
            }
            TraceRetentionMode::HashesOnly => {
                assert_eq!(value["run"]["rendered"], Value::Null);
                assert_eq!(value["run"]["query"], Value::Null);
                assert_eq!(value["selections"][0]["knowledge_revision_id"], Value::Null);
                assert!(value["selections"][0]["content_hash"].is_string());
            }
            TraceRetentionMode::Disabled => {
                assert_eq!(value["run"]["rendered"], Value::Null);
                assert_eq!(value["run"]["query"], Value::Null);
                assert_eq!(value["candidates"], json!([]));
                assert_eq!(value["selections"], json!([]));
                assert_eq!(value["run"]["candidate_count"], 0);
                assert_eq!(value["run"]["selection_count"], 0);
            }
        }
    }

    set_trace_mode(&world, TraceRetentionMode::Full).await;
    let (ordinary_status, ordinary) = call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{}/knowledge-query", world.alice_session),
        &world.alice_token,
        None,
        Some(json!({"query": "PulseBoard correlation", "limit": 20})),
    )
    .await;
    assert_eq!(ordinary_status, StatusCode::OK, "{ordinary}");
    assert!(
        ordinary["items"]
            .as_array()
            .expect("query items")
            .iter()
            .any(|entry| entry["knowledge"]["id"] == corpus.active_id),
        "ordinary query did not return current Knowledge: {ordinary}"
    );
    assert!(ordinary.get("next_cursor").is_none());
    let (wrong_shape, _) = call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{}/knowledge-query", world.alice_session),
        &world.alice_token,
        None,
        Some(json!({"query": "correlation", "ids": [corpus.active_id]})),
    )
    .await;
    assert_eq!(wrong_shape, StatusCode::BAD_REQUEST);

    let (evaluation_status, first) = call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{}/knowledge-evaluation", world.alice_session),
        &world.alice_token,
        None,
        Some(json!({"limit": 1})),
    )
    .await;
    assert_eq!(evaluation_status, StatusCode::OK, "{first}");
    assert_eq!(first["retrieval_mode"], "listing");
    let cursor = first["next_cursor"]
        .as_str()
        .expect("more than one current item");
    let as_of = first["as_of"].as_str().expect("bound as-of");
    let (next_status, next) = call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{}/knowledge-evaluation", world.alice_session),
        &world.alice_token,
        None,
        Some(json!({"limit": 1, "cursor": cursor, "as_of": as_of})),
    )
    .await;
    assert_eq!(next_status, StatusCode::OK, "{next}");
    assert_eq!(next["as_of"], first["as_of"]);

    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .expect("begin diagnostics Configuration");
    configuration_support::bind_tenant_pack(&mut tx, world.tenant_id, synveda_policy::STANDARD)
        .await;
    tx.commit().await.expect("commit diagnostics Configuration");
    let (member_diagnostics, _) = call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{}/knowledge-evaluation", world.bob_session),
        &world.bob_token,
        None,
        Some(json!({"ids": [corpus.active_id], "limit": 1})),
    )
    .await;
    assert_eq!(member_diagnostics, StatusCode::FORBIDDEN);

    // Keep fixture fields pinned: both subtype ids and governed scopes are
    // derived and were not accepted from a context request body.
    assert!(!world.workspace_id.is_empty());
    assert_ne!(world.workspace_scope, world.project_scope);
}
