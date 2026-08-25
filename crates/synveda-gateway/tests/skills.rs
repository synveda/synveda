//! CPR-23 acceptance evidence for stable Agent Skills aggregates, immutable
//! versions, revisioned project bindings, exact-version usage, and controlled
//! non-executing tests. Every mutation enters as a typed VedaFlow change.

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
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{GrantId, IdentityId, IdentityKind, ScopeId, Tenant, TenantId, TenantStatus};
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
        search_index: Arc::new(
            SearchIndex::open(
                std::env::temp_dir()
                    .join("synveda-cpr23-tests")
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

async fn node(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: TenantId,
    parent: ScopeId,
    kind: ScopeKind,
    slug: &str,
) -> Scope {
    scopes::create(
        tx,
        &scopes::NewScope {
            id: ScopeId::new(),
            tenant_id: tenant,
            kind,
            parent_scope_id: Some(parent),
            slug: format!("{}-{}", slug, ScopeId::new().as_uuid().simple()),
            display_name: slug.to_owned(),
            attributes: json!({}),
            principal_id: None,
            created_by: None,
        },
    )
    .await
    .expect("create scope")
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
    .expect("grant project role");
    tx.commit().await.expect("commit grant");
}

struct World {
    app: Router,
    pool: PgPool,
    tenant: Tenant,
    project: ScopeId,
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
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");
    let tenant_id = TenantId::new();
    let tenant = tenants::create(
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
    let root = scopes::ensure_tenant_root(&mut tx, tenant_id)
        .await
        .expect("create tenant root");
    let workspace = node(
        &mut tx,
        tenant_id,
        root.id,
        ScopeKind::Workspace,
        "skills-workspace",
    )
    .await;
    let project = node(
        &mut tx,
        tenant_id,
        workspace.id,
        ScopeKind::Project,
        "skills-project",
    )
    .await;
    configuration_support::bind_tenant_pack(&mut tx, tenant_id, synveda_policy::STANDARD).await;
    tx.commit().await.expect("commit scopes and policy");

    for subject in ["alice", "reviewer", "administrator"] {
        user(&pool, tenant_id, subject).await;
    }
    grant(&pool, tenant_id, project.id, "alice", RoleKey::Member).await;
    grant(&pool, tenant_id, project.id, "reviewer", RoleKey::Reviewer).await;
    grant(
        &pool,
        tenant_id,
        project.id,
        "administrator",
        RoleKey::Administrator,
    )
    .await;

    let app = router(state(&url));
    Some(World {
        app,
        pool,
        tenant,
        project: project.id,
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
    for token in [&world.reviewer, &world.administrator] {
        let (status, reviewed) = call(
            &world.app,
            Method::POST,
            &format!("/v1/proposals/{change_id}/approve"),
            token,
            None,
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

    let (status, tested) = call(
        &world.app,
        Method::POST,
        &format!("/v1/skills/{skill_id}/versions/{version_v2}/tests"),
        &world.alice,
        Some(json!({"harness": "validation_sandbox"})),
        Some("validate-code-review-v2"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{tested}");
    assert_eq!(tested["outcome"], "passed");
    assert_eq!(tested["evidence"]["executes_bundle_code"], false);
    assert_eq!(
        tested["evidence"]["declared_tools_are_authorization"],
        false
    );
    let (status, test_replay) = call(
        &world.app,
        Method::POST,
        &format!("/v1/skills/{skill_id}/versions/{version_v2}/tests"),
        &world.alice,
        Some(json!({"harness": "validation_sandbox"})),
        Some("validate-code-review-v2"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{test_replay}");
    assert_eq!(test_replay["id"], tested["id"]);

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
    let second_tenant = tenants::create(
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
}
