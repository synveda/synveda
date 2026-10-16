//! Shared ordinary-role context/source public-API fixture.
#![allow(dead_code, unused_imports)]

//! CPR-20 acceptance evidence for explainable Knowledge-backed context.
//!
//! These tests use the public Knowledge, session, context and query surfaces.
//! Fixture bootstrap creates only tenants, identities and grants; no test
//! writes planner rows or active Knowledge behind the application layer.

#[path = "../../../synveda-store/tests/support/tenant_fixture.rs"]
pub(crate) mod tenant_fixture;

pub(crate) use std::sync::{Arc, OnceLock};
pub(crate) use std::time::{Duration, Instant};

pub(crate) use axum::Router;
pub(crate) use axum::body::Body;
pub(crate) use axum::http::{Request, StatusCode};
pub(crate) use http_body_util::BodyExt;
pub(crate) use metrics_exporter_prometheus::PrometheusHandle;
pub(crate) use serde_json::{Value, json};
pub(crate) use sqlx::postgres::PgPoolOptions;
pub(crate) use synveda_gateway::app::{AppState, behavior_test_router as router};
pub(crate) use synveda_gateway::telemetry;
pub(crate) use synveda_identity::Hs256Verifier;
pub(crate) use synveda_policy::Pdp;
pub(crate) use synveda_store::{access, identities, knowledge_conflicts, rls, scopes};
pub(crate) use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
pub(crate) use synveda_types::configuration::ContextOptimizationMode;
pub(crate) use synveda_types::knowledge::ConflictClassification;
pub(crate) use synveda_types::{
    CompositionConfig, ConflictSetId, GrantId, IdentityId, IdentityKind, PackConfig, ProjectId,
    ScopeId, TenantId, TenantStatus, TraceRetentionMode,
};
pub(crate) use tower::ServiceExt;

#[path = "configuration.rs"]
pub(crate) mod configuration_support;

pub(crate) const SECRET: &[u8] = b"cpr-20-context-planning";
pub(crate) const ALICE: &str = "alice-cpr20@pulseboard.test";
pub(crate) const BOB: &str = "bob-cpr20@pulseboard.test";
pub(crate) const MALLORY: &str = "mallory-cpr20@pulseboard.test";
pub(crate) const BLANKET: &str =
    "permit (principal, action, resource) when { resource in principal.tenant };";

pub(crate) async fn serial() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

pub(crate) fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

pub(crate) fn state(url: &str) -> AppState {
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
        git_sources: None,
        git_exports: None,
        keys: Arc::new(synveda_store::keys::KeyRing::new(
            synveda_crypto::Kms::Disabled,
        )),
    }
}

pub(crate) async fn seed_identity(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    subject: &str,
) -> ScopeId {
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

pub(crate) async fn seed_grant(
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

pub(crate) struct World {
    pub(crate) database_url: String,
    pub(crate) state: AppState,
    pub(crate) app: Router,
    pub(crate) tenant_id: TenantId,
    pub(crate) alice_scope: ScopeId,
    pub(crate) alice_token: String,
    pub(crate) bob_token: String,
    pub(crate) mallory_token: String,
    pub(crate) workspace_id: String,
    pub(crate) workspace_scope: ScopeId,
    pub(crate) project_id: String,
    pub(crate) project_scope: ScopeId,
    pub(crate) alice_session: String,
    pub(crate) bob_session: String,
}

impl World {
    pub(crate) async fn install_policy(
        &self,
        name: &str,
        version: i64,
        source: &str,
        config: PackConfig,
    ) {
        self.state
            .pdp
            .compile_check(name, source)
            .expect("validate fixture policy");
        let mut tx = rls::begin_tenant_write_tx(&self.state.pool, self.tenant_id)
            .await
            .unwrap();
        let current = synveda_store::policy_packs::get(&mut *tx, self.tenant_id, name)
            .await
            .unwrap();
        let pack = match current {
            Some(pack)
                if pack.version == version && pack.source == source && pack.config == config =>
            {
                pack
            }
            _ => {
                synveda_store::policy_packs::apply(&mut *tx, self.tenant_id, name, source, &config)
                    .await
                    .expect("persist ordinary tenant policy fixture")
            }
        };
        assert_eq!(pack.version, version);
        tx.commit().await.expect("commit policy fixture");
        self.state
            .pdp
            .install_source(self.tenant_id, name, pack.version, source, config)
            .expect("compile exact persisted fixture policy");
    }
}

pub(crate) async fn call(
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

pub(crate) async fn admitted_world() -> Option<World> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping CPR-20 context tests: DATABASE_URL is not set \
                 (run `make db-test`)"
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

pub(crate) fn replace_gateway_pool(world: &mut World, max_connections: u32) {
    let mut state = world.state.clone();
    state.pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(5))
        .connect_lazy(&world.database_url)
        .expect("parse database url");
    world.app = router(state.clone());
    world.state = state;
}

pub(crate) fn content(title: &str, body: &str, stale_after: Option<&str>) -> Value {
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
pub(crate) async fn create_knowledge(
    world: &World,
    key: &str,
    scope_id: ScopeId,
    project_id: Option<&str>,
    owner: Option<&str>,
    title: &str,
    body: &str,
    stale_after: Option<&str>,
) -> (String, String) {
    create_knowledge_with_content(
        world,
        key,
        scope_id,
        project_id,
        owner,
        content(title, body, stale_after),
    )
    .await
}

pub(crate) async fn create_knowledge_with_content(
    world: &World,
    key: &str,
    scope_id: ScopeId,
    project_id: Option<&str>,
    owner: Option<&str>,
    revision_content: Value,
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
            "content": revision_content,
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

pub(crate) async fn support_relation(
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

pub(crate) struct Corpus {
    pub(crate) active_id: String,
    pub(crate) active_revision: String,
    pub(crate) stale_id: String,
    pub(crate) stale_revision: String,
    pub(crate) superseded_id: String,
    pub(crate) private_id: String,
    pub(crate) private_revision: String,
}

pub(crate) async fn corpus(world: &World) -> Corpus {
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
    let (stale_id, stale_revision) = create_knowledge(
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
        stale_revision,
        superseded_id,
        private_id,
        private_revision,
    }
}

pub(crate) async fn context_run(
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

pub(crate) async fn detail(world: &World, token: &str, run_id: &str) -> Value {
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

pub(crate) async fn audit_events(world: &World) -> Vec<synveda_audit::StoredEvent> {
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

pub(crate) async fn set_trace_mode(world: &World, mode: TraceRetentionMode) {
    let name = format!("cpr20-{}", mode.as_str().replace('_', "-"));
    world
        .install_policy(
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
        .await;
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
