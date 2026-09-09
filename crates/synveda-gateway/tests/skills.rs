//! CPR-23 acceptance evidence for stable Agent Skills aggregates, immutable
//! versions, revisioned project bindings, exact-version usage, and controlled
//! non-executing tests. Every mutation enters as a typed VedaFlow change.

#[path = "../../synveda-store/tests/support/tenant_fixture.rs"]
mod tenant_fixture;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, behavior_test_router as router};
use synveda_gateway::skill_validation::{ExecuteOutcome, Runtime as SkillValidationRuntime};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder};
use synveda_policy::Pdp;
use synveda_store::{
    access, identities, operations as operation_store, projects, rls, scopes, workspaces,
};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::{
    DurableOperationId, GrantId, IdentityId, IdentityKind, ProjectId, ScopeId, Tenant, TenantId,
    TenantStatus, WorkspaceId,
};
use tower::ServiceExt;

#[path = "support/configuration.rs"]
mod configuration_support;

const SECRET: &[u8] = b"cpr-23-versioned-skills";

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
        pdp: Arc::new(Pdp::new().expect("build embedded PDP")),
        service_token_max_ttl: Duration::from_secs(3600),
        embedder: Arc::new(AnyEmbedder::Deterministic(DeterministicEmbedder::new())),
        context_embed_timeout: Duration::from_millis(100),
        keys: Arc::new(synveda_store::keys::KeyRing::new(
            synveda_crypto::Kms::Disabled,
        )),
    }
}

fn issue(subject: &str, tenant: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue(subject, tenant, Duration::from_secs(300))
}

async fn user(pool: &PgPool, tenant: TenantId, subject: &str) {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant transaction");
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
        .expect("begin tenant transaction");
    access::create_grant(
        &mut tx,
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
    .expect("grant project role");
    tx.commit().await.expect("commit grant");
}

struct World {
    app: Router,
    pool: PgPool,
    tenant: Tenant,
    project: ScopeId,
    project_id: ProjectId,
    alice: String,
    reviewer: String,
    administrator: String,
}

async fn world() -> Option<World> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping CPR-23 Skill integration test: DATABASE_URL is not set \
                 (run make dev-up then make db-test)"
            );
            return None;
        }
    };
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(&url)
        .await
        .expect("connect to database");
    synveda_store::epoch::verify(&pool)
        .await
        .expect("apply migrations");
    let tenant_id = TenantId::new();
    let tenant = tenant_fixture::create(
        &pool,
        tenant_id,
        &format!("cpr23-{}", tenant_id.as_uuid().simple()),
        "CPR-23 Skill test",
        TenantStatus::Active,
    )
    .await
    .expect("create tenant");
    let mut tx = rls::begin_tenant_tx(&pool, tenant_id)
        .await
        .expect("begin scope transaction");
    scopes::ensure_tenant_root(&mut tx, tenant_id)
        .await
        .expect("create tenant root");
    let workspace = workspaces::create(
        &mut tx,
        &workspaces::NewWorkspace {
            id: WorkspaceId::new(),
            tenant_id,
            slug: format!("skills-{}", WorkspaceId::new().as_uuid().simple()),
            display_name: "Skills workspace".to_owned(),
            description: None,
            created_by: None,
        },
    )
    .await
    .expect("create workspace");
    let project = projects::create(
        &mut tx,
        &projects::NewProject {
            id: ProjectId::new(),
            tenant_id,
            workspace_id: workspace.id,
            slug: format!("skills-{}", ProjectId::new().as_uuid().simple()),
            display_name: "Skills project".to_owned(),
            description: None,
            created_by: None,
        },
    )
    .await
    .expect("create project");
    configuration_support::bind_tenant_pack(&mut tx, tenant_id, synveda_policy::STANDARD).await;
    tx.commit().await.expect("commit scopes and policy");

    for subject in ["alice", "reviewer", "administrator"] {
        user(&pool, tenant_id, subject).await;
    }
    grant(&pool, tenant_id, project.scope_id, "alice", RoleKey::Member).await;
    grant(
        &pool,
        tenant_id,
        project.scope_id,
        "reviewer",
        RoleKey::Reviewer,
    )
    .await;
    grant(
        &pool,
        tenant_id,
        project.scope_id,
        "administrator",
        RoleKey::Administrator,
    )
    .await;

    let app = router(state(&url));
    Some(World {
        app,
        pool,
        tenant,
        project: project.scope_id,
        project_id: project.id,
        alice: issue("alice", tenant_id),
        reviewer: issue("reviewer", tenant_id),
        administrator: issue("administrator", tenant_id),
    })
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
        serde_json::from_slice(&bytes).expect("response is JSON")
    };
    (status, value)
}

fn bundle(version: &str, instruction: &str) -> Value {
    json!({
        "governing_scope_id": Value::Null,
        "name": "code-review",
        "sensitivity": "internal",
        "files": [
            {
                "path": "SKILL.md",
                "content": format!(
                    "---\nname: code-review\ndescription: Review a diff and report actionable defects. Use when a change needs review.\nlicense: Apache-2.0\ncompatibility: Requires git.\nmetadata:\n  version: {version}\nallowed-tools: Read Bash(git diff *)\n---\n\n# Code Review\n\n## When to use\n\nUse this skill when a user asks for a code review.\n\n## Steps\n\n1. Read the diff.\n2. Check correctness and security.\n3. Report evidence and fixes.\n\n## Output\n\nReturn findings ordered by severity.\n\n{instruction}\n"
                )
            },
            {
                "path": "scripts/check.sh",
                "content": "#!/bin/sh\nprintf '%s\\n' checked\n"
            }
        ],
        "provenance": {
            "kind": "authored",
            "reference": "tests/fixtures/code-review",
            "revision": version,
            "metadata": {"fixture": "cpr-23"}
        }
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
    assert_eq!(proposal["artifact_references"][0]["family"], "skill");
    assert!(
        proposal["artifact_references"][0]["version"]
            .as_str()
            .is_some_and(|value| !value.is_empty()),
        "proposal must bind an exact Skill version or state digest: {proposal}"
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

async fn open_install(world: &World) -> Value {
    let mut body = bundle(
        "1.0.0",
        "Never execute a declared tool unless the host separately authorises it.",
    );
    body["governing_scope_id"] = json!(world.project);
    let (status, opened) = call(
        &world.app,
        Method::POST,
        "/v1/skills",
        &world.alice,
        Some(body),
        Some("install-code-review-v1"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{opened}");
    assert_eq!(opened["outcome"], "pending_review");
    opened
}

async fn open_update(world: &World, skill_id: &str, current: &str, key: &str) -> Value {
    let source = bundle("2.0.0", "Also identify missing regression coverage.");
    let body = json!({
        "expected_current_version_id": current,
        "sensitivity": source["sensitivity"].clone(),
        "files": source["files"].clone(),
        "provenance": source["provenance"].clone(),
    });
    let (status, opened) = call(
        &world.app,
        Method::PATCH,
        &format!("/v1/skills/{skill_id}"),
        &world.alice,
        Some(body),
        Some(key),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{opened}");
    assert_eq!(opened["outcome"], "pending_review");
    opened
}

async fn install_skill(world: &World) -> (String, String) {
    let opened = open_install(world).await;
    let applied = approve_and_apply(
        world,
        opened["change_id"]
            .as_str()
            .expect("Skill install change id"),
    )
    .await;
    (
        applied["skill_id"]
            .as_str()
            .expect("installed Skill id")
            .to_owned(),
        applied["version_id"]
            .as_str()
            .expect("installed Skill version id")
            .to_owned(),
    )
}

async fn enqueue_validation(world: &World, skill: &str, version: &str, key: &str) -> Value {
    let (status, operation) = call(
        &world.app,
        Method::POST,
        &format!("/v1/skills/{skill}/versions/{version}/validation-operations"),
        &world.alice,
        Some(json!({"harness": "validation_sandbox"})),
        Some(key),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{operation}");
    operation
}

async fn dispatch_claim(
    pool: &PgPool,
    tenant: TenantId,
    owner: &str,
) -> Option<operation_store::DispatchSelection> {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin dispatch claim");
    let selected = operation_store::claim_dispatch(&mut tx, tenant, owner, 30, 60)
        .await
        .expect("claim outbox delivery");
    tx.commit().await.expect("commit dispatch claim");
    selected
}

async fn make_dispatch_due(pool: &PgPool, tenant: TenantId, operation: DurableOperationId) {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin dispatch schedule adjustment");
    let changed = sqlx::query!(
        r#"update operation_outbox
           set next_dispatch_at = statement_timestamp(), updated_at = statement_timestamp()
           where tenant_id = $1 and operation_id = $2 and state = 'pending'"#,
        tenant.as_uuid(),
        operation.as_uuid(),
    )
    .execute(&mut *tx)
    .await
    .expect("make dispatch due")
    .rows_affected();
    assert_eq!(changed, 1, "one pending outbox row becomes due");
    tx.commit().await.expect("commit due dispatch");
}

#[tokio::test]
async fn immutable_versions_bindings_usage_and_tests_share_one_governed_path() {
    let _serial = serial().await;
    let Some(world) = world().await else { return };

    let installed = open_install(&world).await;
    let change = installed["change_id"].as_str().expect("change id");
    let skill_id = installed["skill_id"].as_str().expect("skill id");
    let version_v1 = installed["version_id"].as_str().expect("version id");

    let (status, before) = call(
        &world.app,
        Method::GET,
        "/v1/skills",
        &world.alice,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{before}");
    assert_eq!(
        before["skills"].as_array().map(Vec::len),
        Some(0),
        "a pending VedaFlow change is not an installed Skill"
    );

    let applied = approve_and_apply(&world, change).await;
    assert_eq!(applied["outcome"], "applied");
    assert_eq!(applied["version_id"], version_v1);

    let (status, exact) = call(
        &world.app,
        Method::GET,
        &format!("/v1/skills/{skill_id}/versions/{version_v1}"),
        &world.alice,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{exact}");
    assert_eq!(exact["ordinal"], 1);
    assert_eq!(exact["declared_tools_are_authorization"], false);
    assert_eq!(exact["manifest"]["allowed-tools"][0], "Read");

    let (status, file) = call(
        &world.app,
        Method::GET,
        &format!("/v1/skills/{skill_id}/versions/{version_v1}/files/scripts/check.sh"),
        &world.alice,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{file}");
    assert_eq!(file["content"], "#!/bin/sh\nprintf '%s\\n' checked\n");

    let first_update =
        open_update(&world, skill_id, version_v1, "update-code-review-v2-first").await;
    let stale_update =
        open_update(&world, skill_id, version_v1, "update-code-review-v2-stale").await;
    let first_change = first_update["change_id"].as_str().expect("first change");
    let version_v2 = first_update["version_id"].as_str().expect("v2");
    let stale_change = stale_update["change_id"].as_str().expect("stale change");

    assert_eq!(
        approve_and_apply(&world, first_change).await["outcome"],
        "applied"
    );
    let rejected = approve_and_apply(&world, stale_change).await;
    assert_eq!(rejected["outcome"], "rejected");

    let (status, versions) = call(
        &world.app,
        Method::GET,
        &format!("/v1/skills/{skill_id}/versions"),
        &world.alice,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{versions}");
    assert_eq!(versions["versions"].as_array().map(Vec::len), Some(2));
    assert_eq!(versions["versions"][0]["id"], version_v2);
    assert_eq!(versions["versions"][1]["id"], version_v1);

    let (status, old_file) = call(
        &world.app,
        Method::GET,
        &format!("/v1/skills/{skill_id}/versions/{version_v1}/files/SKILL.md"),
        &world.alice,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{old_file}");
    assert!(
        old_file["content"]
            .as_str()
            .is_some_and(|content| content.contains("version: 1.0.0")),
        "the old immutable bytes remain available"
    );

    let (status, binding_change) = call(
        &world.app,
        Method::POST,
        "/v1/skill-bindings",
        &world.alice,
        Some(json!({
            "scope_id": world.project,
            "skill_id": skill_id,
            "pinned_version_id": Value::Null,
            "enabled": true,
        })),
        Some("bind-code-review"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{binding_change}");
    let binding_id = binding_change["binding_id"].as_str().expect("binding id");
    assert_eq!(
        approve_and_apply(
            &world,
            binding_change["change_id"]
                .as_str()
                .expect("binding change")
        )
        .await["outcome"],
        "applied"
    );

    let (status, available) = call(
        &world.app,
        Method::GET,
        &format!("/v1/skills/available?scope_id={}", world.project),
        &world.alice,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{available}");
    assert_eq!(available["skills"][0]["version"]["id"], version_v2);
    assert_eq!(
        available["skills"][0]["version"]["declared_tools_are_authorization"],
        false
    );

    let (status, usage) = call(
        &world.app,
        Method::POST,
        "/v1/skill-usage",
        &world.alice,
        Some(json!({
            "binding_id": binding_id,
            "version_id": version_v2,
            "client_event_id": "adapter-usage-1",
            "stage": "activated",
            "evidence": "model_reported",
            "metadata": {"client": "fixture"},
            "occurred_at": chrono::Utc::now(),
        })),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{usage}");
    let (status, replay) = call(
        &world.app,
        Method::POST,
        "/v1/skill-usage",
        &world.alice,
        Some(json!({
            "binding_id": binding_id,
            "version_id": version_v2,
            "client_event_id": "adapter-usage-1",
            "stage": "activated",
            "evidence": "model_reported",
            "metadata": {"client": "fixture"},
            "occurred_at": usage["occurred_at"].clone(),
        })),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["id"], usage["id"]);

    let test_uri = format!("/v1/skills/{skill_id}/versions/{version_v2}/tests");
    let test_body = json!({"harness": "validation_sandbox"});
    let (tested_left, tested_right) = tokio::join!(
        call(
            &world.app,
            Method::POST,
            &test_uri,
            &world.alice,
            Some(test_body.clone()),
            Some("validate-code-review-v2"),
        ),
        call(
            &world.app,
            Method::POST,
            &test_uri,
            &world.alice,
            Some(test_body),
            Some("validate-code-review-v2"),
        )
    );
    let test_statuses = [tested_left.0, tested_right.0];
    assert_eq!(
        test_statuses
            .iter()
            .filter(|status| **status == StatusCode::CREATED)
            .count(),
        1,
        "one concurrent Skill test creates the result: {test_statuses:?}; left={}; right={}",
        tested_left.1,
        tested_right.1,
    );
    assert_eq!(
        test_statuses
            .iter()
            .filter(|status| **status == StatusCode::OK)
            .count(),
        1,
        "the concurrent duplicate replays: {test_statuses:?}; left={}; right={}",
        tested_left.1,
        tested_right.1,
    );
    assert_eq!(tested_left.1["id"], tested_right.1["id"]);
    assert_eq!(tested_left.1["outcome"], "passed");
    assert_eq!(tested_left.1["evidence"]["executes_bundle_code"], false);
    assert_eq!(
        tested_left.1["evidence"]["declared_tools_are_authorization"],
        false
    );
    let (status, test_runs) =
        call(&world.app, Method::GET, &test_uri, &world.alice, None, None).await;
    assert_eq!(status, StatusCode::OK, "{test_runs}");
    let run_ids = test_runs["runs"]
        .as_array()
        .expect("test runs")
        .iter()
        .map(|run| run["id"].as_str().expect("test run id"))
        .collect::<Vec<_>>();
    assert_eq!(run_ids.len(), 1, "one row per idempotency key: {test_runs}");
    assert_eq!(
        run_ids[0],
        tested_left.1["id"].as_str().expect("concurrent run id")
    );

    let (status, rollback_change) = call(
        &world.app,
        Method::POST,
        &format!("/v1/skill-bindings/{binding_id}/rollback"),
        &world.alice,
        Some(json!({
            "expected_revision": 1,
            "version_id": version_v1,
        })),
        Some("rollback-code-review-v1"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{rollback_change}");
    let rollback = approve_and_apply(
        &world,
        rollback_change["change_id"]
            .as_str()
            .expect("rollback change"),
    )
    .await;
    assert_eq!(rollback["binding_revision"], 2);

    let (status, rolled_back) = call(
        &world.app,
        Method::GET,
        &format!("/v1/skills/available?scope_id={}", world.project),
        &world.alice,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rolled_back}");
    assert_eq!(rolled_back["skills"][0]["version"]["id"], version_v1);

    let mut tx = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin Skill-advertisement Configuration change");
    configuration_support::set_tenant_advertisement(&mut tx, world.tenant.id, false, true).await;
    tx.commit()
        .await
        .expect("commit Skill-advertisement Configuration change");
    let (status, suppressed) = call(
        &world.app,
        Method::GET,
        &format!("/v1/skills/available?scope_id={}", world.project),
        &world.alice,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{suppressed}");
    assert_eq!(suppressed["skills"], json!([]));

    let second_tenant_id = TenantId::new();
    let second_tenant = tenant_fixture::create(
        &world.pool,
        second_tenant_id,
        &format!("cpr23-other-{}", second_tenant_id.as_uuid().simple()),
        "CPR-23 isolation tenant",
        TenantStatus::Active,
    )
    .await
    .expect("create second tenant");
    user(&world.pool, second_tenant.id, "mallory").await;
    let mallory = issue("mallory", second_tenant.id);
    let (status, denied) = call(
        &world.app,
        Method::GET,
        &format!("/v1/skills/{skill_id}"),
        &mallory,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{denied}");

    let mut tx = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin verification transaction");
    let version_count: i64 = sqlx::query_scalar!(
        r#"select count(*) as "count!" from skill_versions
         where tenant_id = $1 and skill_id = $2"#,
        world.tenant.id.as_uuid(),
        uuid::Uuid::parse_str(skill_id).expect("skill UUID"),
    )
    .fetch_one(&mut *tx)
    .await
    .expect("count immutable versions");
    assert_eq!(
        version_count, 2,
        "the rejected stale update created no version"
    );
    let actions: Vec<String> = sqlx::query_scalar!(
        r#"select action from audit_log
           where tenant_id = $1 and action like 'skill.%'
           order by seq"#,
        world.tenant.id.as_uuid(),
    )
    .fetch_all(&mut *tx)
    .await
    .expect("read Skill audit actions");
    for required in [
        "skill.change.opened",
        "skill.change.applied",
        "skill.change.rejected",
        "skill.usage.recorded",
        "skill.test.recorded",
    ] {
        assert!(
            actions.iter().any(|action| action == required),
            "missing {required} in {actions:?}"
        );
    }
    assert_eq!(
        actions
            .iter()
            .filter(|action| action.as_str() == "skill.test.recorded")
            .count(),
        1,
        "the losing idempotency transaction must not retain an audit event: {actions:?}"
    );
    let untyped_terminal: i64 = sqlx::query_scalar!(
        r#"select count(*) as "count!" from audit_log
           where tenant_id = $1
             and action in ('skill.change.applied', 'skill.change.rejected')
             and not (payload @> '{"artifact_references":[{"family":"skill"}]}'::jsonb)"#,
        world.tenant.id.as_uuid(),
    )
    .fetch_one(&mut *tx)
    .await
    .expect("check terminal Skill artifact references");
    assert_eq!(
        untyped_terminal, 0,
        "terminal Skill evidence lost its typed address"
    );
}

#[tokio::test]
async fn durable_skill_validation_is_atomic_authorized_and_single_effect() {
    let _guard = serial().await;
    let Some(world) = world().await else {
        return;
    };
    let (skill_id, version_id) = install_skill(&world).await;
    let operation =
        enqueue_validation(&world, &skill_id, &version_id, "durable-validation-native").await;
    let operation_id: DurableOperationId = operation["id"]
        .as_str()
        .expect("operation id")
        .parse()
        .expect("operation UUID");
    assert_eq!(operation["state"], "pending");
    assert_eq!(operation["progress_percent"], 0);

    let (status, replay) = call(
        &world.app,
        Method::POST,
        &format!("/v1/skills/{skill_id}/versions/{version_id}/validation-operations"),
        &world.alice,
        Some(json!({"harness": "validation_sandbox"})),
        Some("durable-validation-native"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["id"], operation["id"]);

    let mut inspect = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin atomicity inspection");
    let rows = sqlx::query!(
        r#"select
             (select count(*) from durable_operations
              where tenant_id = $1 and id = $2) as "operations!",
             (select count(*) from operation_outbox
              where tenant_id = $1 and operation_id = $2) as "outbox!""#,
        world.tenant.id.as_uuid(),
        operation_id.as_uuid(),
    )
    .fetch_one(&mut *inspect)
    .await
    .expect("inspect atomic operation/outbox");
    assert_eq!((rows.operations, rows.outbox), (1, 1));
    inspect
        .rollback()
        .await
        .expect("finish atomicity inspection");

    let foreign_id = TenantId::new();
    tenant_fixture::create(
        &world.pool,
        foreign_id,
        &format!("cpr45-foreign-{}", foreign_id.as_uuid().simple()),
        "CPR-45 operation isolation",
        TenantStatus::Active,
    )
    .await
    .expect("create foreign tenant");
    let foreign_runtime = SkillValidationRuntime::new(
        world.pool.clone(),
        Arc::new(Pdp::new().expect("build foreign executor PDP")),
    )
    .with_worker_id("foreign-worker");
    assert_eq!(
        synveda_gateway::skill_validation::execute_operation(
            &foreign_runtime,
            foreign_id,
            Some(operation_id),
            1,
        )
        .await
        .expect("execute under foreign tenant"),
        ExecuteOutcome::NotClaimed,
    );

    let first = SkillValidationRuntime::new(
        world.pool.clone(),
        Arc::new(Pdp::new().expect("build first executor PDP")),
    )
    .with_worker_id("native-worker-a");
    let second = SkillValidationRuntime::new(
        world.pool.clone(),
        Arc::new(Pdp::new().expect("build second executor PDP")),
    )
    .with_worker_id("native-worker-b");
    let (left, right) = tokio::join!(
        synveda_gateway::skill_validation::execute_operation(
            &first,
            world.tenant.id,
            Some(operation_id),
            1,
        ),
        synveda_gateway::skill_validation::execute_operation(
            &second,
            world.tenant.id,
            Some(operation_id),
            1,
        )
    );
    let outcomes = [left.expect("first worker"), right.expect("second worker")];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == ExecuteOutcome::Succeeded)
            .count(),
        1,
        "exactly one worker commits the effect: {outcomes:?}"
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == ExecuteOutcome::NotClaimed)
            .count(),
        1,
        "the second worker observes the fence: {outcomes:?}"
    );

    let (status, completed) = call(
        &world.app,
        Method::GET,
        &format!("/v1/operations/{operation_id}"),
        &world.alice,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(completed["state"], "succeeded");
    assert_eq!(completed["progress_percent"], 100);
    assert_eq!(completed["attempts"], 1);
    assert!(completed.get("lease_owner").is_none());
    assert!(completed.get("requested_by").is_none());

    let (status, listed) = call(
        &world.app,
        Method::GET,
        &format!("/v1/operations?project_id={}&limit=8", world.project_id),
        &world.alice,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["operations"][0]["id"], operation["id"]);

    user(&world.pool, world.tenant.id, "operation-outsider").await;
    let outsider = issue("operation-outsider", world.tenant.id);
    for uri in [
        format!("/v1/operations/{operation_id}"),
        format!("/v1/operations?project_id={}&limit=8", world.project_id),
    ] {
        let (status, _) = call(&world.app, Method::GET, &uri, &outsider, None, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "operation visibility leaked");
    }

    let cancelled =
        enqueue_validation(&world, &skill_id, &version_id, "durable-validation-cancel").await;
    let cancelled_id: DurableOperationId = cancelled["id"]
        .as_str()
        .expect("cancelled operation id")
        .parse()
        .expect("cancelled operation UUID");
    for _ in 0..2 {
        let (status, view) = call(
            &world.app,
            Method::POST,
            &format!("/v1/operations/{cancelled_id}/cancel"),
            &world.alice,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{view}");
        assert_eq!(view["state"], "cancelled");
    }
    assert_eq!(
        synveda_gateway::skill_validation::execute_operation(
            &first,
            world.tenant.id,
            Some(cancelled_id),
            1,
        )
        .await
        .expect("cancelled operation is not executable"),
        ExecuteOutcome::NotClaimed,
    );

    let racing = enqueue_validation(
        &world,
        &skill_id,
        &version_id,
        "durable-validation-cancel-race",
    )
    .await;
    let racing_id: DurableOperationId = racing["id"]
        .as_str()
        .expect("racing operation id")
        .parse()
        .expect("racing operation UUID");
    let racing_worker = SkillValidationRuntime::new(
        world.pool.clone(),
        Arc::new(Pdp::new().expect("build racing executor PDP")),
    )
    .with_worker_id("native-worker-race");
    let racing_cancel_path = format!("/v1/operations/{racing_id}/cancel");
    let (execution, cancellation) = tokio::join!(
        synveda_gateway::skill_validation::execute_operation(
            &racing_worker,
            world.tenant.id,
            Some(racing_id),
            1,
        ),
        call(
            &world.app,
            Method::POST,
            &racing_cancel_path,
            &world.alice,
            None,
            None,
        )
    );
    let execution = execution.expect("finish cancellation race");
    assert!(matches!(
        execution,
        ExecuteOutcome::Succeeded | ExecuteOutcome::NotClaimed | ExecuteOutcome::LeaseLost
    ));
    assert_eq!(cancellation.0, StatusCode::OK, "{}", cancellation.1);
    assert!(matches!(
        cancellation.1["state"].as_str(),
        Some("succeeded" | "cancelled")
    ));
    let mut race_inspect = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin race inspection");
    let race = sqlx::query!(
        r#"select operation.state,
                  (select count(*) from skill_test_runs run
                   where run.tenant_id = operation.tenant_id
                     and run.operation_id = operation.id) as "effects!"
           from durable_operations operation
           where operation.tenant_id = $1 and operation.id = $2"#,
        world.tenant.id.as_uuid(),
        racing_id.as_uuid(),
    )
    .fetch_one(&mut *race_inspect)
    .await
    .expect("inspect cancellation race");
    assert_eq!(Some(race.state.as_str()), cancellation.1["state"].as_str());
    assert_eq!(race.effects, if race.state == "succeeded" { 1 } else { 0 });
    race_inspect
        .rollback()
        .await
        .expect("finish race inspection");

    let mut foreign = rls::begin_tenant_tx(&world.pool, foreign_id)
        .await
        .expect("begin foreign inspection");
    let hidden = sqlx::query!(
        r#"select
             (select count(*) from durable_operations where id = $1) as "operations!",
             (select count(*) from operation_attempts where operation_id = $1) as "attempts!",
             (select count(*) from operation_outbox where operation_id = $1) as "outbox!",
             (select count(*) from skill_test_runs where operation_id = $1) as "effects!""#,
        operation_id.as_uuid(),
    )
    .fetch_one(&mut *foreign)
    .await
    .expect("forced RLS inspection");
    assert_eq!(
        (
            hidden.operations,
            hidden.attempts,
            hidden.outbox,
            hidden.effects
        ),
        (0, 0, 0, 0),
        "forced RLS hides operation, attempts, outbox and effect"
    );
    foreign.rollback().await.expect("finish foreign inspection");
}

#[tokio::test]
async fn operation_outbox_fences_fast_consumers_and_bounded_submission_failures() {
    let _guard = serial().await;
    let Some(world) = world().await else {
        return;
    };
    let (skill_id, version_id) = install_skill(&world).await;

    let acknowledgement_lost = enqueue_validation(
        &world,
        &skill_id,
        &version_id,
        "durable-acknowledgement-lost",
    )
    .await;
    let acknowledgement_lost_id: DurableOperationId = acknowledgement_lost["id"]
        .as_str()
        .expect("acknowledgement-loss operation id")
        .parse()
        .expect("acknowledgement-loss operation UUID");
    let mut first_delivery_tx = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin first unacknowledged delivery");
    let first_delivery = operation_store::claim_dispatch(
        &mut first_delivery_tx,
        world.tenant.id,
        "crashing-dispatcher",
        1,
        60,
    )
    .await
    .expect("claim first unacknowledged delivery")
    .expect("first delivery selected");
    let first_claim = match first_delivery {
        operation_store::DispatchSelection::Claimed(claim) => claim,
        operation_store::DispatchSelection::DeadLettered(_) => {
            panic!("a fresh operation cannot exhaust dispatch")
        }
    };
    assert_eq!(first_claim.operation_id, acknowledgement_lost_id);
    assert_eq!(first_claim.dispatch_attempt, 1);
    first_delivery_tx
        .commit()
        .await
        .expect("commit unacknowledged delivery");
    let mut lost_ack = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin lost acknowledgement");
    assert_eq!(
        operation_store::acknowledge_dispatch(&mut lost_ack, &first_claim)
            .await
            .expect("write acknowledgement before simulated loss"),
        operation_store::DispatchAcknowledgement::Acknowledged,
    );
    lost_ack
        .rollback()
        .await
        .expect("simulate acknowledgement-write failure");
    let mut old_transaction = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin before outbox lease expiry");
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let reclaimed = operation_store::claim_dispatch(
        &mut old_transaction,
        world.tenant.id,
        "recovery-dispatcher",
        30,
        60,
    )
    .await
    .expect("reclaim unacknowledged delivery")
    .expect("expired delivery selected");
    assert!(matches!(
        reclaimed,
        operation_store::DispatchSelection::Claimed(ref claim)
            if claim.operation_id == acknowledgement_lost_id && claim.dispatch_attempt == 2
    ));
    old_transaction
        .commit()
        .await
        .expect("commit reclaimed delivery");
    let mut cleanup = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin acknowledgement-loss cleanup");
    operation_store::request_cancellation(&mut cleanup, world.tenant.id, acknowledgement_lost_id)
        .await
        .expect("cancel acknowledgement-loss fixture")
        .expect("acknowledgement-loss operation exists");
    cleanup
        .commit()
        .await
        .expect("commit acknowledgement-loss cleanup");
    let mut duplicate_terminal = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin duplicate terminal delivery");
    assert_eq!(
        operation_store::recover_interrupted_delivery(
            &mut duplicate_terminal,
            world.tenant.id,
            acknowledgement_lost_id,
            "delivery_interrupted",
        )
        .await
        .expect("consume duplicate terminal delivery"),
        Some(synveda_types::operation::OperationState::Cancelled),
    );
    duplicate_terminal
        .commit()
        .await
        .expect("commit duplicate terminal delivery");

    let fast = enqueue_validation(&world, &skill_id, &version_id, "durable-fast-consumer").await;
    let fast_id: DurableOperationId = fast["id"]
        .as_str()
        .expect("fast operation id")
        .parse()
        .expect("fast operation UUID");
    let (left, right) = tokio::join!(
        dispatch_claim(&world.pool, world.tenant.id, "dispatcher-a"),
        dispatch_claim(&world.pool, world.tenant.id, "dispatcher-b")
    );
    let selections = [left, right];
    assert_eq!(
        selections.iter().filter(|entry| entry.is_some()).count(),
        1,
        "two dispatchers must produce one fenced claim: {selections:?}"
    );
    let dispatch = match selections
        .into_iter()
        .flatten()
        .next()
        .expect("dispatch claim")
    {
        operation_store::DispatchSelection::Claimed(claim) => claim,
        operation_store::DispatchSelection::DeadLettered(_) => {
            panic!("a fresh operation cannot exhaust dispatch")
        }
    };
    assert_eq!(dispatch.operation_id, fast_id);

    let mut claim_tx = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin fast execution claim");
    let execution = operation_store::claim_skill_validation_execution(
        &mut claim_tx,
        world.tenant.id,
        "fast-worker",
        30,
        Some(fast_id),
        3,
    )
    .await
    .expect("claim fast execution")
    .expect("fast execution selected");
    let execution = match execution {
        operation_store::ExecutionSelection::Claimed(claim) => claim,
        operation_store::ExecutionSelection::DeadLettered(_) => {
            panic!("a fresh execution cannot be dead-lettered")
        }
    };
    claim_tx
        .commit()
        .await
        .expect("commit fast execution claim");

    let mut fail_tx = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin fast execution retry");
    assert_eq!(
        operation_store::fail_execution(
            &mut fail_tx,
            &execution,
            "fast-worker",
            "dependency_unavailable",
            Some(1),
            3,
        )
        .await
        .expect("schedule fast-consumer retry"),
        synveda_types::operation::OperationState::Failed,
    );
    let claimed_state = sqlx::query_scalar!(
        r#"select state from operation_outbox
           where tenant_id = $1 and operation_id = $2"#,
        world.tenant.id.as_uuid(),
        fast_id.as_uuid(),
    )
    .fetch_one(&mut *fail_tx)
    .await
    .expect("read in-flight outbox");
    assert_eq!(claimed_state, "claimed");
    fail_tx.commit().await.expect("commit product retry");

    let mut acknowledge = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin delayed acknowledgement");
    assert_eq!(
        operation_store::acknowledge_dispatch(&mut acknowledge, &dispatch)
            .await
            .expect("acknowledge delayed submission"),
        operation_store::DispatchAcknowledgement::Acknowledged,
    );
    let rescheduled = sqlx::query!(
        r#"select state, next_dispatch_at, submission_failures
           from operation_outbox where tenant_id = $1 and operation_id = $2"#,
        world.tenant.id.as_uuid(),
        fast_id.as_uuid(),
    )
    .fetch_one(&mut *acknowledge)
    .await
    .expect("inspect fast-consumer reschedule");
    assert_eq!(rescheduled.state, "pending");
    assert!(rescheduled.next_dispatch_at.is_some());
    assert_eq!(rescheduled.submission_failures, 0);
    acknowledge
        .commit()
        .await
        .expect("commit delayed acknowledgement");
    // The operation ledger owns retry timing; transport scheduling cannot
    // make this failed business operation eligible before that deadline.
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let redispatched = dispatch_claim(&world.pool, world.tenant.id, "recovery-dispatcher")
        .await
        .expect("fast-consumer retry is dispatchable");
    assert!(matches!(
        redispatched,
        operation_store::DispatchSelection::Claimed(ref claim)
            if claim.operation_id == fast_id && claim.dispatch_attempt == 2
    ));
    let mut cleanup = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin fast-operation cleanup");
    operation_store::request_cancellation(&mut cleanup, world.tenant.id, fast_id)
        .await
        .expect("cancel fast-operation fixture")
        .expect("fast operation exists");
    cleanup
        .commit()
        .await
        .expect("commit fast-operation cleanup");

    let interrupted =
        enqueue_validation(&world, &skill_id, &version_id, "durable-fast-recovery").await;
    let interrupted_id: DurableOperationId = interrupted["id"]
        .as_str()
        .expect("interrupted operation id")
        .parse()
        .expect("interrupted operation UUID");
    let interrupted_dispatch =
        match dispatch_claim(&world.pool, world.tenant.id, "interrupted-dispatcher")
            .await
            .expect("interrupted dispatch selected")
        {
            operation_store::DispatchSelection::Claimed(claim) => claim,
            operation_store::DispatchSelection::DeadLettered(_) => {
                panic!("a fresh interrupted dispatch cannot be dead-lettered")
            }
        };
    assert_eq!(interrupted_dispatch.operation_id, interrupted_id);
    let mut recover = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin fast delivery recovery");
    assert_eq!(
        operation_store::recover_interrupted_delivery(
            &mut recover,
            world.tenant.id,
            interrupted_id,
            "delivery_interrupted",
        )
        .await
        .expect("recover fast delivery"),
        Some(synveda_types::operation::OperationState::Pending),
    );
    recover
        .commit()
        .await
        .expect("commit fast delivery recovery");
    let mut delayed_ack = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin post-recovery acknowledgement");
    assert_eq!(
        operation_store::acknowledge_dispatch(&mut delayed_ack, &interrupted_dispatch)
            .await
            .expect("fence post-recovery acknowledgement"),
        operation_store::DispatchAcknowledgement::LostFence,
    );
    delayed_ack
        .commit()
        .await
        .expect("commit post-recovery acknowledgement");
    assert!(matches!(
        dispatch_claim(&world.pool, world.tenant.id, "post-recovery-dispatcher")
            .await
            .expect("recovered delivery is immediately dispatchable"),
        operation_store::DispatchSelection::Claimed(ref claim)
            if claim.operation_id == interrupted_id && claim.dispatch_attempt == 2
    ));
    let mut cleanup = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin interrupted-operation cleanup");
    operation_store::request_cancellation(&mut cleanup, world.tenant.id, interrupted_id)
        .await
        .expect("cancel interrupted-operation fixture")
        .expect("interrupted operation exists");
    cleanup
        .commit()
        .await
        .expect("commit interrupted-operation cleanup");

    let live = enqueue_validation(
        &world,
        &skill_id,
        &version_id,
        "durable-live-consumer-deferral",
    )
    .await;
    let live_id: DurableOperationId = live["id"]
        .as_str()
        .expect("live operation id")
        .parse()
        .expect("live operation UUID");
    let live_dispatch = match dispatch_claim(&world.pool, world.tenant.id, "uncertain-dispatcher")
        .await
        .expect("live dispatch selected")
    {
        operation_store::DispatchSelection::Claimed(claim) => claim,
        operation_store::DispatchSelection::DeadLettered(_) => {
            panic!("a fresh live dispatch cannot be dead-lettered")
        }
    };
    assert_eq!(live_dispatch.operation_id, live_id);
    let mut live_execution = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin live execution claim");
    assert!(matches!(
        operation_store::claim_skill_validation_execution(
            &mut live_execution,
            world.tenant.id,
            "live-consumer",
            30,
            Some(live_id),
            3,
        )
        .await
        .expect("claim live execution"),
        Some(operation_store::ExecutionSelection::Claimed(_))
    ));
    live_execution
        .commit()
        .await
        .expect("commit live execution claim");
    let mut uncertain = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin uncertain live submission");
    assert_eq!(
        operation_store::defer_dispatch(&mut uncertain, &live_dispatch, 2, "queue_unavailable",)
            .await
            .expect("defer uncertain live submission"),
        operation_store::DispatchDeferral::ConsumerRunning,
    );
    let live_outbox = sqlx::query!(
        r#"select state, next_dispatch_at, submission_failures
           from operation_outbox where tenant_id = $1 and operation_id = $2"#,
        world.tenant.id.as_uuid(),
        live_id.as_uuid(),
    )
    .fetch_one(&mut *uncertain)
    .await
    .expect("inspect live-consumer outbox");
    assert_eq!(live_outbox.state, "dispatched");
    assert!(live_outbox.next_dispatch_at.is_none());
    assert_eq!(live_outbox.submission_failures, 0);
    uncertain
        .commit()
        .await
        .expect("commit uncertain live submission");
    let mut cleanup = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin live-operation cleanup");
    operation_store::request_cancellation(&mut cleanup, world.tenant.id, live_id)
        .await
        .expect("cancel live-operation fixture")
        .expect("live operation exists");
    cleanup
        .commit()
        .await
        .expect("commit live-operation cleanup");

    let reset =
        enqueue_validation(&world, &skill_id, &version_id, "durable-submission-reset").await;
    let reset_id: DurableOperationId = reset["id"]
        .as_str()
        .expect("reset operation id")
        .parse()
        .expect("reset operation UUID");
    for attempt in 1..=2 {
        let selected = dispatch_claim(
            &world.pool,
            world.tenant.id,
            &format!("failing-dispatcher-{attempt}"),
        )
        .await
        .expect("failing dispatch selected");
        let claim = match selected {
            operation_store::DispatchSelection::Claimed(claim) => claim,
            operation_store::DispatchSelection::DeadLettered(_) => {
                panic!("submission failed too early")
            }
        };
        assert_eq!(claim.operation_id, reset_id);
        let mut tx = rls::begin_tenant_tx(&world.pool, world.tenant.id)
            .await
            .expect("begin failed submission");
        assert_eq!(
            operation_store::defer_dispatch(&mut tx, &claim, 1, "queue_unavailable",)
                .await
                .expect("defer failed submission"),
            operation_store::DispatchDeferral::RetryScheduled,
        );
        tx.commit().await.expect("commit failed submission");
        make_dispatch_due(&world.pool, world.tenant.id, reset_id).await;
    }
    let selected = dispatch_claim(&world.pool, world.tenant.id, "healthy-dispatcher")
        .await
        .expect("healthy dispatch selected");
    let claim = match selected {
        operation_store::DispatchSelection::Claimed(claim) => claim,
        operation_store::DispatchSelection::DeadLettered(_) => {
            panic!("two failed submissions cannot exhaust the budget")
        }
    };
    let mut tx = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin successful submission");
    assert_eq!(
        operation_store::acknowledge_dispatch(&mut tx, &claim)
            .await
            .expect("acknowledge successful submission"),
        operation_store::DispatchAcknowledgement::Acknowledged,
    );
    let submitted = sqlx::query!(
        r#"select state, dispatch_attempts, submission_failures
           from operation_outbox where tenant_id = $1 and operation_id = $2"#,
        world.tenant.id.as_uuid(),
        reset_id.as_uuid(),
    )
    .fetch_one(&mut *tx)
    .await
    .expect("inspect successful submission");
    assert_eq!(submitted.state, "dispatched");
    assert_eq!(submitted.dispatch_attempts, 3);
    assert_eq!(submitted.submission_failures, 0);
    tx.commit().await.expect("commit successful submission");
    let mut cleanup = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin reset-operation cleanup");
    operation_store::request_cancellation(&mut cleanup, world.tenant.id, reset_id)
        .await
        .expect("cancel reset-operation fixture")
        .expect("reset operation exists");
    cleanup
        .commit()
        .await
        .expect("commit reset-operation cleanup");

    let stale = enqueue_validation(
        &world,
        &skill_id,
        &version_id,
        "durable-stale-delivery-recovery",
    )
    .await;
    let stale_id: DurableOperationId = stale["id"]
        .as_str()
        .expect("stale delivery operation id")
        .parse()
        .expect("stale delivery operation UUID");
    let stale_claim = match dispatch_claim(&world.pool, world.tenant.id, "stale-dispatcher")
        .await
        .expect("stale delivery selected")
    {
        operation_store::DispatchSelection::Claimed(claim) => claim,
        operation_store::DispatchSelection::DeadLettered(_) => {
            panic!("a fresh stale-delivery fixture cannot be dead-lettered")
        }
    };
    assert_eq!(stale_claim.operation_id, stale_id);
    let mut stale_ack = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin stale delivery acknowledgement");
    let aged = sqlx::query!(
        r#"
        update operation_outbox
        set state = 'dispatched', lease_owner = null, lease_expires_at = null,
            next_dispatch_at = null,
            dispatched_at = statement_timestamp() - interval '61 seconds',
            submission_failures = 0, last_error_code = null, updated_at = now()
        where tenant_id = $1 and operation_id = $2 and state = 'claimed'
          and lease_owner = $3 and dispatch_attempts = $4
        "#,
        world.tenant.id.as_uuid(),
        stale_id.as_uuid(),
        &stale_claim.lease_owner,
        stale_claim.dispatch_attempt,
    )
    .execute(&mut *stale_ack)
    .await
    .expect("age an acknowledged delivery")
    .rows_affected();
    assert_eq!(aged, 1);
    stale_ack
        .commit()
        .await
        .expect("commit stale delivery acknowledgement");
    let mut stale_recovery = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin stale delivery recovery");
    let stale_reclaimed = operation_store::claim_dispatch(
        &mut stale_recovery,
        world.tenant.id,
        "stale-recovery-dispatcher",
        30,
        60,
    )
    .await
    .expect("reclaim stale delivery")
    .expect("stale delivery is dispatchable");
    assert!(matches!(
        stale_reclaimed,
        operation_store::DispatchSelection::Claimed(ref claim)
            if claim.operation_id == stale_id && claim.dispatch_attempt == 2
    ));
    stale_recovery
        .commit()
        .await
        .expect("commit stale delivery recovery");
    let mut cleanup = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin stale-delivery cleanup");
    operation_store::request_cancellation(&mut cleanup, world.tenant.id, stale_id)
        .await
        .expect("cancel stale-delivery fixture")
        .expect("stale-delivery operation exists");
    cleanup
        .commit()
        .await
        .expect("commit stale-delivery cleanup");

    let exhausted = enqueue_validation(
        &world,
        &skill_id,
        &version_id,
        "durable-submission-exhausted",
    )
    .await;
    let exhausted_id: DurableOperationId = exhausted["id"]
        .as_str()
        .expect("exhausted operation id")
        .parse()
        .expect("exhausted operation UUID");
    for attempt in 1..=3 {
        let selected = dispatch_claim(
            &world.pool,
            world.tenant.id,
            &format!("exhausted-dispatcher-{attempt}"),
        )
        .await
        .expect("exhausted dispatch selected");
        let claim = match selected {
            operation_store::DispatchSelection::Claimed(claim) => claim,
            operation_store::DispatchSelection::DeadLettered(_) => {
                panic!("defer owns immediate terminalization")
            }
        };
        assert_eq!(claim.operation_id, exhausted_id);
        let mut tx = rls::begin_tenant_tx(&world.pool, world.tenant.id)
            .await
            .expect("begin exhausted submission");
        let outcome = operation_store::defer_dispatch(&mut tx, &claim, 1, "queue_unavailable")
            .await
            .expect("defer exhausted submission");
        if attempt < 3 {
            assert_eq!(outcome, operation_store::DispatchDeferral::RetryScheduled);
        } else {
            assert!(matches!(
                outcome,
                operation_store::DispatchDeferral::DeadLettered(_)
            ));
        }
        tx.commit().await.expect("commit exhausted submission");
        if attempt < 3 {
            make_dispatch_due(&world.pool, world.tenant.id, exhausted_id).await;
        }
    }

    let expired = enqueue_validation(
        &world,
        &skill_id,
        &version_id,
        "durable-expired-running-dispatch",
    )
    .await;
    let expired_id: DurableOperationId = expired["id"]
        .as_str()
        .expect("expired operation id")
        .parse()
        .expect("expired operation UUID");
    let mut running = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin expiring execution");
    assert!(matches!(
        operation_store::claim_skill_validation_execution(
            &mut running,
            world.tenant.id,
            "expired-worker",
            1,
            Some(expired_id),
            3,
        )
        .await
        .expect("claim expiring execution"),
        Some(operation_store::ExecutionSelection::Claimed(_))
    ));
    running.commit().await.expect("commit expiring execution");

    let mut old_transaction = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("begin before lease expiry");
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let first_expired = operation_store::claim_dispatch(
        &mut old_transaction,
        world.tenant.id,
        "expired-dispatcher-1",
        30,
        60,
    )
    .await
    .expect("statement-time dispatch claim")
    .expect("expired operation is dispatchable");
    let first_expired = match first_expired {
        operation_store::DispatchSelection::Claimed(claim) => claim,
        operation_store::DispatchSelection::DeadLettered(_) => {
            panic!("an unsubmitted operation cannot be dead-lettered")
        }
    };
    assert_eq!(first_expired.operation_id, expired_id);
    old_transaction
        .commit()
        .await
        .expect("commit statement-time claim");
    for (attempt, claim) in (1..=3).map(|attempt| {
        if attempt == 1 {
            (attempt, Some(first_expired.clone()))
        } else {
            (attempt, None)
        }
    }) {
        let claim = match claim {
            Some(claim) => claim,
            None => {
                make_dispatch_due(&world.pool, world.tenant.id, expired_id).await;
                match dispatch_claim(
                    &world.pool,
                    world.tenant.id,
                    &format!("expired-dispatcher-{attempt}"),
                )
                .await
                .expect("expired dispatch selected")
                {
                    operation_store::DispatchSelection::Claimed(claim) => claim,
                    operation_store::DispatchSelection::DeadLettered(_) => {
                        panic!("defer owns expired-operation terminalization")
                    }
                }
            }
        };
        let mut tx = rls::begin_tenant_tx(&world.pool, world.tenant.id)
            .await
            .expect("begin expired submission failure");
        let outcome = operation_store::defer_dispatch(&mut tx, &claim, 1, "queue_unavailable")
            .await
            .expect("defer expired submission");
        if attempt < 3 {
            assert_eq!(outcome, operation_store::DispatchDeferral::RetryScheduled);
        } else {
            assert!(matches!(
                outcome,
                operation_store::DispatchDeferral::DeadLettered(_)
            ));
        }
        tx.commit()
            .await
            .expect("commit expired submission failure");
    }
}
