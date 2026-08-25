//! CPR-25 acceptance evidence for the governed MCP catalogue. The fixture
//! reports discovery and connection-test evidence through the public API; the
//! gateway never launches or calls the represented server.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder};
use synveda_policy::Pdp;
use synveda_retrieval::SearchIndex;
use synveda_store::{access, identities, rls, scopes, tenants};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::{GrantId, IdentityId, IdentityKind, ScopeId, TenantId, TenantStatus};
use tower::ServiceExt;

#[path = "support/configuration.rs"]
mod configuration_support;

const SECRET: &[u8] = b"cpr-25-trusted-mcp-registry";
const PLAINTEXT_FIXTURE: &str = "shh-cpr25-plaintext-token";

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
        search_index: Arc::new(
            SearchIndex::open(
                std::env::temp_dir()
                    .join("synveda-cpr25-tests")
                    .join(TenantId::new().to_string()),
            )
            .expect("open search sidecar"),
        ),
        embedder: Arc::new(AnyEmbedder::Deterministic(DeterministicEmbedder::new())),
        inject_embed_timeout: Duration::from_millis(100),
        keys: Arc::new(synveda_store::keys::KeyRing::new(
            synveda_crypto::Kms::Disabled,
        )),
    }
}

fn issue(subject: &str, tenant: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue(subject, tenant, Duration::from_secs(300))
}

async fn identity(pool: &PgPool, tenant: TenantId, subject: &str) {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin identity transaction");
    let own = scopes::ensure_principal_scope(&mut tx, tenant, subject, subject)
        .await
        .expect("create principal scope");
    identities::create(
        &mut tx,
        IdentityId::new(),
        tenant,
        Some(subject),
        IdentityKind::User,
        None,
        Some(subject),
        own.id,
    )
    .await
    .expect("create identity");
    tx.commit().await.expect("commit identity");
}

async fn grant(pool: &PgPool, tenant: TenantId, scope: ScopeId, subject: &str, role: RoleKey) {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin grant transaction");
    access::create_grant(
        &mut *tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id: tenant,
            scope_id: scope,
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
    .expect("grant role");
    tx.commit().await.expect("commit role");
}

async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    token: &str,
    body: Option<Value>,
    idempotency_key: Option<&str>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));
    if let Some(key) = idempotency_key {
        builder = builder.header("idempotency-key", key);
    }
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(builder.body(body).expect("build request"))
        .await
        .expect("call router");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .expect("read response");
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| panic!("response is JSON: {}", String::from_utf8_lossy(&bytes)))
    };
    (status, value)
}

struct World {
    app: Router,
    pool: PgPool,
    tenant: TenantId,
    project_id: String,
    project_scope: ScopeId,
    alice: String,
    reviewer: String,
    administrator: String,
}

async fn world() -> Option<World> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping CPR-25 Tool integration test: DATABASE_URL is not set \
                 (run make dev-up then make db-test)"
            );
            return None;
        }
    };
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .expect("connect to database");
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");
    let tenant = TenantId::new();
    tenants::create(
        &pool,
        tenant,
        &format!("cpr25-{}", tenant.as_uuid().simple()),
        "CPR-25 Tool registry test",
        TenantStatus::Active,
    )
    .await
    .expect("create tenant");
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("begin tenant bootstrap");
    let root = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("create tenant root");
    configuration_support::bind_tenant_pack(&mut tx, tenant, synveda_policy::STANDARD).await;
    tx.commit().await.expect("commit tenant bootstrap");

    for subject in ["alice", "reviewer", "administrator"] {
        identity(&pool, tenant, subject).await;
    }
    grant(&pool, tenant, root.id, "alice", RoleKey::Administrator).await;

    let app = router(state(&url));
    let alice = issue("alice", tenant);
    let reviewer = issue("reviewer", tenant);
    let administrator = issue("administrator", tenant);
    let (status, workspace) = call(
        &app,
        Method::POST,
        "/v1/workspaces",
        &alice,
        Some(json!({"slug": "pulseboard", "display_name": "PulseBoard"})),
        Some("cpr25-workspace"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workspace}");
    let workspace_id = workspace["id"].as_str().expect("workspace id");
    let (status, project) = call(
        &app,
        Method::POST,
        &format!("/v1/workspaces/{workspace_id}/projects"),
        &alice,
        Some(json!({"slug": "api", "display_name": "PulseBoard API"})),
        Some("cpr25-project"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().expect("project id").to_owned();
    let project_scope: ScopeId = project["scope_id"]
        .as_str()
        .expect("project scope")
        .parse()
        .expect("parse project scope");
    grant(&pool, tenant, project_scope, "reviewer", RoleKey::Reviewer).await;
    grant(
        &pool,
        tenant,
        project_scope,
        "administrator",
        RoleKey::Administrator,
    )
    .await;

    Some(World {
        app,
        pool,
        tenant,
        project_id,
        project_scope,
        alice,
        reviewer,
        administrator,
    })
}

fn descriptor() -> Value {
    json!({
        "source_kind": "remote_http",
        "source_reference": "https://mcp.pulseboard.test/manifest.json",
        "transport": "streamable_http",
        "endpoint": "https://mcp.pulseboard.test/mcp",
        "args": [],
        "authentication": "oauth",
        "secret_reference": "secret-ref://pulseboard/mcp-oauth",
        "requested_permissions": ["issues:read", "deployments:read"],
        "metadata": {"vendor": "PulseBoard", "fixture": "CPR-25"}
    })
}

fn capabilities(version: u8) -> Value {
    let mut tools = vec![json!({
        "name": "lookup_issue",
        "description": if version == 1 { "Read one issue" } else { "Read one issue with links" },
        "inputSchema": {
            "type": "object",
            "properties": {
                "issue_id": {"type": "string"},
                "include_links": {"type": "boolean"}
            },
            "required": ["issue_id"]
        }
    })];
    if version > 1 {
        tools.push(json!({
            "name": "deploy_release",
            "description": "Declared write-shaped tool; still grants no authority",
            "inputSchema": {"type": "object", "properties": {"tag": {"type": "string"}}}
        }));
    }
    json!({
        "protocol_version": "2026-07-28",
        "server_info": {"name": "pulseboard", "version": format!("{version}.0.0")},
        "tools": tools,
        "resources": [{"uri": "repo://pulseboard/runbooks", "name": "runbooks"}],
        "prompts": [{"name": "triage", "description": "Triage an incident", "arguments": []}],
        "extensions": {"fixture": "CPR-25", "revision": version}
    })
}

async fn approve_and_apply(world: &World, change_id: &str) -> Value {
    let (status, proposal) = call(
        &world.app,
        Method::GET,
        &format!("/v1/proposals/{change_id}"),
        &world.reviewer,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "proposal read failed: {proposal}");
    assert!(
        matches!(
            proposal["artifact_references"][0]["family"].as_str(),
            Some("tool_server" | "tool_binding")
        ),
        "proposal must name its Tool server or binding family: {proposal}"
    );
    assert!(
        proposal["artifact_references"][0]["version"]
            .as_str()
            .is_some_and(|value| !value.is_empty()),
        "proposal must bind an exact Tool version or state digest: {proposal}"
    );
    for token in [&world.reviewer, &world.administrator] {
        let (status, reviewed) = call(
            &world.app,
            Method::POST,
            &format!("/v1/proposals/{change_id}/approve"),
            token,
            Some(json!({"expected_commit": proposal["commit"]})),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "approval failed: {reviewed}");
    }
    let (status, applied) = call(
        &world.app,
        Method::POST,
        &format!("/v1/proposals/{change_id}/apply"),
        &world.administrator,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "apply failed: {applied}");
    applied
}

#[tokio::test]
async fn versions_discovery_bindings_config_and_tests_share_one_governed_path() {
    let _serial = serial().await;
    let Some(world) = world().await else { return };

    let (status, unsafe_import) = call(
        &world.app,
        Method::POST,
        "/v1/tool-servers/import-client-config",
        &world.alice,
        Some(json!({
            "governing_scope_id": world.project_scope,
            "client": "claude_code",
            "name": "unsafe",
            "server": {
                "url": "https://mcp.pulseboard.test/mcp",
                "headers": {"Authorization": format!("Bearer {PLAINTEXT_FIXTURE}")}
            },
            "capabilities": capabilities(1)
        })),
        Some("cpr25-refuse-secret"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{unsafe_import}");
    assert!(!unsafe_import.to_string().contains(PLAINTEXT_FIXTURE));

    let registration_body = json!({
        "governing_scope_id": world.project_scope,
        "name": "pulseboard-tools",
        "descriptor": descriptor(),
        "capabilities": capabilities(1)
    });
    let (status, opened) = call(
        &world.app,
        Method::POST,
        "/v1/tool-servers",
        &world.alice,
        Some(registration_body.clone()),
        Some("cpr25-register-v1"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{opened}");
    assert_eq!(opened["outcome"], "pending_review");
    let change_v1 = opened["change_id"].as_str().expect("v1 change");
    let server_id = opened["server_id"].as_str().expect("server id");
    let version_v1 = opened["version_id"].as_str().expect("version v1");

    let (status, replay) = call(
        &world.app,
        Method::POST,
        "/v1/tool-servers",
        &world.alice,
        Some(registration_body),
        Some("cpr25-register-v1"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["change_id"], change_v1);
    assert_eq!(replay["version_id"], version_v1);

    let (status, quarantined) = call(
        &world.app,
        Method::GET,
        &format!("/v1/tool-servers/{server_id}/versions/{version_v1}"),
        &world.alice,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{quarantined}");
    assert_eq!(quarantined["state"], "quarantined");
    assert_eq!(quarantined["protocol_version"], "2026-07-28");
    assert_eq!(
        quarantined["declared_capabilities_are_authorization"],
        false
    );
    assert_eq!(
        quarantined["normalized_capabilities"]["tools"]["entries"][0]["name"],
        "lookup_issue"
    );

    let applied_v1 = approve_and_apply(&world, change_v1).await;
    assert_eq!(applied_v1["outcome"], "applied");
    let (status, server) = call(
        &world.app,
        Method::GET,
        &format!("/v1/tool-servers/{server_id}"),
        &world.alice,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{server}");
    assert_eq!(server["current_version_id"], version_v1);

    let (status, unchanged) = call(
        &world.app,
        Method::POST,
        &format!("/v1/tool-servers/{server_id}/discoveries"),
        &world.alice,
        Some(json!({
            "expected_current_version_id": version_v1,
            "capabilities": capabilities(1)
        })),
        Some("cpr25-discover-unchanged"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{unchanged}");
    assert_eq!(unchanged["change_id"], change_v1);
    assert_eq!(unchanged["version_id"], version_v1);

    let (status, changed) = call(
        &world.app,
        Method::POST,
        &format!("/v1/tool-servers/{server_id}/discoveries"),
        &world.alice,
        Some(json!({
            "expected_current_version_id": version_v1,
            "capabilities": capabilities(2)
        })),
        Some("cpr25-discover-v2"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{changed}");
    assert_eq!(changed["outcome"], "pending_review");
    let change_v2 = changed["change_id"].as_str().expect("v2 change");
    let version_v2 = changed["version_id"].as_str().expect("version v2");

    let (status, still_v1) = call(
        &world.app,
        Method::GET,
        &format!("/v1/tool-servers/{server_id}"),
        &world.alice,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{still_v1}");
    assert_eq!(still_v1["current_version_id"], version_v1);
    let (status, comparison) = call(
        &world.app,
        Method::GET,
        &format!("/v1/tool-servers/{server_id}/versions/{version_v2}/diff?against={version_v1}"),
        &world.alice,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{comparison}");
    assert_eq!(comparison["tools_added"], json!(["deploy_release"]));
    assert_eq!(comparison["tools_changed"], json!(["lookup_issue"]));

    let (status, refused_binding) = call(
        &world.app,
        Method::POST,
        "/v1/tool-bindings",
        &world.alice,
        Some(json!({
            "project_id": world.project_id,
            "server_id": server_id,
            "version_id": version_v2,
            "state": "enabled"
        })),
        Some("cpr25-refuse-quarantined-binding"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{refused_binding}");

    let (status, binding_change) = call(
        &world.app,
        Method::POST,
        "/v1/tool-bindings",
        &world.alice,
        Some(json!({
            "project_id": world.project_id,
            "server_id": server_id,
            "version_id": version_v1,
            "state": "enabled"
        })),
        Some("cpr25-bind-v1"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{binding_change}");
    let binding_id = binding_change["binding_id"].as_str().expect("binding id");
    approve_and_apply(
        &world,
        binding_change["change_id"]
            .as_str()
            .expect("binding change"),
    )
    .await;

    let (status, config_v1) = call(
        &world.app,
        Method::GET,
        &format!("/v1/projects/{}/tool-config", world.project_id),
        &world.alice,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{config_v1}");
    assert_eq!(config_v1["bindings"][0]["version_id"], version_v1);
    assert_eq!(
        config_v1["configuration"]["mcpServers"]["pulseboard-tools"]["secretReference"],
        "secret-ref://pulseboard/mcp-oauth"
    );
    assert!(!config_v1.to_string().contains(PLAINTEXT_FIXTURE));

    assert_eq!(
        approve_and_apply(&world, change_v2).await["outcome"],
        "applied"
    );
    let (status, pinned_v1) = call(
        &world.app,
        Method::GET,
        &format!("/v1/projects/{}/tool-config", world.project_id),
        &world.alice,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{pinned_v1}");
    assert_eq!(pinned_v1["bindings"][0]["version_id"], version_v1);

    let (status, repin_change) = call(
        &world.app,
        Method::PATCH,
        &format!("/v1/tool-bindings/{binding_id}"),
        &world.alice,
        Some(json!({
            "expected_revision": 1,
            "version_id": version_v2,
            "state": "enabled",
            "reason": "repin"
        })),
        Some("cpr25-repin-v2"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{repin_change}");
    assert_eq!(repin_change["outcome"], "pending_review");
    let repinned = approve_and_apply(
        &world,
        repin_change["change_id"].as_str().expect("repin change"),
    )
    .await;
    assert_eq!(repinned["binding_revision"], 2);

    let safe_test = json!({
        "harness": "remote_http_adapter",
        "harness_version": "cpr25-fixture/1",
        "outcome": "passed",
        "methods": ["server/discover", "tools/list", "resources/list", "prompts/list"],
        "latency_ms": 23,
        "evidence": {"transport": "streamable_http", "executes_tools": false}
    });
    let (status, tested) = call(
        &world.app,
        Method::POST,
        &format!("/v1/tool-servers/{server_id}/versions/{version_v2}/tests"),
        &world.alice,
        Some(safe_test.clone()),
        Some("cpr25-read-only-test-1"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{tested}");
    assert_eq!(tested["methods"].as_array().map(Vec::len), Some(4));
    let (status, test_replay) = call(
        &world.app,
        Method::POST,
        &format!("/v1/tool-servers/{server_id}/versions/{version_v2}/tests"),
        &world.alice,
        Some(safe_test),
        Some("cpr25-read-only-test-1"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{test_replay}");
    assert_eq!(test_replay["id"], tested["id"]);

    let (status, execution_refused) = call(
        &world.app,
        Method::POST,
        &format!("/v1/tool-servers/{server_id}/versions/{version_v2}/tests"),
        &world.alice,
        Some(json!({
            "harness": "remote_http_adapter",
            "harness_version": "cpr25-fixture/1",
            "outcome": "passed",
            "methods": ["tools/call"],
            "evidence": {}
        })),
        Some("cpr25-refuse-execution"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{execution_refused}");

    let mut tx = rls::begin_tenant_tx(&world.pool, world.tenant)
        .await
        .expect("begin Tool-advertisement Configuration change");
    configuration_support::set_tenant_advertisement(&mut tx, world.tenant, true, false).await;
    tx.commit()
        .await
        .expect("commit Tool-advertisement Configuration change");
    let (status, suppressed_config) = call(
        &world.app,
        Method::GET,
        &format!("/v1/projects/{}/tool-config", world.project_id),
        &world.alice,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{suppressed_config}");
    assert_eq!(suppressed_config["bindings"], json!([]));
    assert_eq!(
        suppressed_config["configuration"],
        json!({"mcpServers": {}})
    );

    let (status, page) = call(
        &world.app,
        Method::GET,
        "/v1/tool-servers?limit=1",
        &world.alice,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["servers"].as_array().map(Vec::len), Some(1));

    let second_tenant = TenantId::new();
    tenants::create(
        &world.pool,
        second_tenant,
        &format!("cpr25-other-{}", second_tenant.as_uuid().simple()),
        "CPR-25 isolation tenant",
        TenantStatus::Active,
    )
    .await
    .expect("create second tenant");
    identity(&world.pool, second_tenant, "mallory").await;
    let mallory = issue("mallory", second_tenant);
    let (status, hidden) = call(
        &world.app,
        Method::GET,
        &format!("/v1/tool-servers/{server_id}"),
        &mallory,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{hidden}");

    let mut tx = rls::begin_tenant_tx(&world.pool, world.tenant)
        .await
        .expect("begin verification transaction");
    let version_count: i64 = sqlx::query_scalar!(
        r#"select count(*) as "count!" from tool_server_versions
           where tenant_id = $1 and server_id = $2"#,
        world.tenant.as_uuid(),
        uuid::Uuid::parse_str(server_id).expect("server UUID"),
    )
    .fetch_one(&mut *tx)
    .await
    .expect("count versions");
    assert_eq!(version_count, 2, "unchanged discovery created no version");
    let immutable = sqlx::query(
        "update tool_server_versions set protocol_version = 'tampered' \
         where tenant_id = $1 and id = $2",
    )
    .bind(world.tenant.as_uuid())
    .bind(uuid::Uuid::parse_str(version_v1).expect("version UUID"))
    .execute(&mut *tx)
    .await;
    assert!(immutable.is_err(), "immutable version accepted an update");
    tx.rollback().await.expect("rollback immutability probe");

    let mut tx = rls::begin_tenant_tx(&world.pool, world.tenant)
        .await
        .expect("begin audit verification");
    let actions: Vec<String> = sqlx::query_scalar!(
        r#"select action from audit_log
           where tenant_id = $1 and action like 'tool.%'
           order by seq"#,
        world.tenant.as_uuid(),
    )
    .fetch_all(&mut *tx)
    .await
    .expect("read Tool audit actions");
    for required in [
        "tool.change.opened",
        "tool.change.applied",
        "tool.test.recorded",
        "tool.configuration.generated",
    ] {
        assert!(
            actions.iter().any(|action| action == required),
            "missing {required} in {actions:?}"
        );
    }
    let plaintext_leaks: i64 = sqlx::query_scalar!(
        r#"select count(*) as "count!" from audit_log
           where tenant_id = $1 and payload::text like $2"#,
        world.tenant.as_uuid(),
        format!("%{PLAINTEXT_FIXTURE}%"),
    )
    .fetch_one(&mut *tx)
    .await
    .expect("scan audit payloads");
    assert_eq!(plaintext_leaks, 0);
    tx.commit().await.expect("commit verification");
}
