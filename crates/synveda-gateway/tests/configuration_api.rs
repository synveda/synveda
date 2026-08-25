//! CPR-30 acceptance: one governed runtime Configuration path.
//!
//! Public mutations traverse the embedded PDP and typed VedaFlow command
//! layer; immutable versions and revisioned bindings then govern a real
//! context run. Store and adversarial RLS coverage live beside this test.

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
use synveda_store::{access, identities, rls, scopes, tenants};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::{GrantId, IdentityId, IdentityKind, ScopeId, TenantId, TenantStatus};
use tower::ServiceExt;

#[path = "support/configuration.rs"]
mod configuration_support;

const SECRET: &[u8] = b"cpr-30-governed-configuration";
const ADMIN: &str = "cpr30-admin";
const PROJECT_OWNER: &str = "cpr30-project-owner";

async fn serial() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install metrics recorder"))
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
        pdp: Arc::new(synveda_policy::Pdp::new().expect("build embedded PDP")),
        service_token_max_ttl: Duration::from_secs(3_600),
        search_index: Arc::new(
            synveda_retrieval::SearchIndex::open(
                std::env::temp_dir()
                    .join("synveda-cpr30-tests")
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

async fn admitted() -> Option<(AppState, Router, TenantId, ScopeId, String)> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!(
            "skipping CPR-30 configuration API test: DATABASE_URL is not set \
             (run `make dev-up` then `make db-test`)"
        );
        return None;
    };
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");
    let tenant = TenantId::new();
    tenants::create(
        &pool,
        tenant,
        &format!("cpr30-{}", tenant.as_uuid().simple()),
        "CPR-30 configuration acceptance",
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
    let principal_scope = scopes::ensure_principal_scope(&mut tx, tenant, ADMIN, ADMIN)
        .await
        .expect("create administrator principal scope");
    identities::create(
        &mut tx,
        IdentityId::new(),
        tenant,
        Some(ADMIN),
        IdentityKind::User,
        None,
        Some(ADMIN),
        principal_scope.id,
    )
    .await
    .expect("create administrator identity");
    access::create_grant(
        &mut *tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id: tenant,
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
    .expect("grant administrator");
    // Fixture bootstrap itself remains a typed VedaFlow change. It gives the
    // local project scope the permissive profile needed to prove auto-apply;
    // the public mutations below are the acceptance subject.
    configuration_support::bind_pack(&mut tx, tenant, root.id, synveda_policy::OPEN_COLLABORATION)
        .await;
    tx.commit().await.expect("commit bootstrap");
    let state = state(&url);
    let app = router(state.clone());
    let token = Hs256Verifier::new(SECRET).issue(ADMIN, tenant, Duration::from_secs(300));
    Some((state, app, tenant, root.id, token))
}

#[tokio::test]
async fn immutable_versions_bindings_and_runtime_evidence_share_one_governed_path() {
    let _guard = serial().await;
    let Some((state, app, tenant, root, token)) = admitted().await else {
        return;
    };

    let (status, workspace) = call(
        &app,
        "POST",
        "/v1/workspaces",
        &token,
        Some("cpr30-workspace"),
        Some(json!({"slug": "configuration", "display_name": "Configuration"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workspace}");
    let workspace_id = workspace["id"].as_str().expect("workspace id");
    let (status, project) = call(
        &app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/projects"),
        &token,
        Some("cpr30-project"),
        Some(json!({"slug": "runtime", "display_name": "Runtime"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().expect("project id");
    let project_scope = project["scope_id"].as_str().expect("project scope");

    let (status, templates) = call(
        &app,
        "GET",
        "/v1/configuration-templates",
        &token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{templates}");
    let personal = templates["templates"]
        .as_array()
        .expect("templates")
        .iter()
        .find(|entry| entry["name"] == "personal")
        .expect("personal template");
    let document = personal["document"].clone();

    let create_body = json!({
        "governing_scope_id": project_scope,
        "name": "project-runtime",
        "document": document,
        "source_template": "personal"
    });
    let (status, created) = call(
        &app,
        "POST",
        "/v1/configurations",
        &token,
        Some("cpr30-create"),
        Some(create_body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["outcome"], "applied", "{created}");
    let artifact_id = created["artifact_id"].as_str().expect("artifact id");
    let first_version = created["version_id"].as_str().expect("first version");
    let first_change = created["change_id"].as_str().expect("first change");
    let (replay_status, replay) = call(
        &app,
        "POST",
        "/v1/configurations",
        &token,
        Some("cpr30-create"),
        Some(create_body),
    )
    .await;
    assert_eq!(replay_status, StatusCode::OK, "{replay}");
    assert_eq!(replay["change_id"], first_change);

    let (status, bound) = call(
        &app,
        "POST",
        "/v1/configuration-bindings",
        &token,
        Some("cpr30-bind"),
        Some(json!({
            "scope_id": project_scope,
            "artifact_id": artifact_id,
            "enabled": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{bound}");
    assert_eq!(bound["outcome"], "applied", "{bound}");
    let binding_id = bound["binding_id"].as_str().expect("binding id");
    assert_eq!(bound["binding_revision"], 1);

    let mut narrowed = personal["document"].clone();
    narrowed["context"]["token_budget"] = json!(256);
    narrowed["context"]["trace_retention"] = json!("redacted");
    narrowed["context"]["channels"] = json!(["unreviewed_candidates"]);
    narrowed["advertisement"]["skills"] = json!(false);
    narrowed["advertisement"]["tools"] = json!(false);
    narrowed["capture"]["minimum_confidence_permille"] = json!(700);
    let publish_body = json!({
        "expected_current_version_id": first_version,
        "document": narrowed
    });
    let (status, published) = call(
        &app,
        "POST",
        &format!("/v1/configurations/{artifact_id}/versions"),
        &token,
        Some("cpr30-publish"),
        Some(publish_body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{published}");
    assert_eq!(published["outcome"], "applied", "{published}");
    let second_version = published["version_id"].as_str().expect("second version");

    let (status, stale) = call(
        &app,
        "POST",
        &format!("/v1/configurations/{artifact_id}/versions"),
        &token,
        Some("cpr30-stale"),
        Some(publish_body),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{stale}");
    assert_eq!(stale["kind"], "conflict", "{stale}");

    let (status, compared) = call(
        &app,
        "GET",
        &format!(
            "/v1/configurations/{artifact_id}/compare?from={first_version}&to={second_version}"
        ),
        &token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{compared}");
    assert!(
        compared["changed_fields"]
            .as_array()
            .expect("changed fields")
            .iter()
            .any(|field| field == "context.token_budget")
    );

    let (status, pinned) = call(
        &app,
        "PATCH",
        &format!("/v1/configuration-bindings/{binding_id}"),
        &token,
        Some("cpr30-pin"),
        Some(json!({
            "expected_revision": 1,
            "artifact_id": artifact_id,
            "pinned_version_id": first_version,
            "enabled": true,
            "reason": "prove exact historical selection"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{pinned}");
    assert_eq!(pinned["binding_revision"], 2);
    let (status, rolled) = call(
        &app,
        "POST",
        &format!("/v1/configuration-bindings/{binding_id}/rollback"),
        &token,
        Some("cpr30-rollback"),
        Some(json!({"expected_revision": 2, "version_id": second_version})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{rolled}");
    assert_eq!(rolled["binding_revision"], 3);

    let (status, effective) = call(
        &app,
        "GET",
        &format!("/v1/configurations/effective?scope_id={project_scope}"),
        &token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{effective}");
    assert_eq!(effective["version_id"], second_version);
    assert_eq!(effective["document"]["context"]["token_budget"], 256);
    let configuration_hash = effective["content_hash"]
        .as_str()
        .expect("effective hash")
        .to_owned();

    // Listing is decided per artifact. A caller who owns this project and
    // nothing at the tenant root sees the project Configuration rather than
    // being denied by an unrelated root-level inventory decision, and the
    // root fixture remains invisible.
    let project_scope_id: ScopeId = project_scope.parse().expect("project scope id");
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant)
        .await
        .expect("begin project-owner setup");
    let owner_scope = scopes::ensure_principal_scope(&mut tx, tenant, PROJECT_OWNER, PROJECT_OWNER)
        .await
        .expect("create project-owner principal scope");
    identities::create(
        &mut tx,
        IdentityId::new(),
        tenant,
        Some(PROJECT_OWNER),
        IdentityKind::User,
        None,
        Some(PROJECT_OWNER),
        owner_scope.id,
    )
    .await
    .expect("create project-owner identity");
    access::create_grant(
        &mut *tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id: tenant,
            scope_id: project_scope_id,
            subject: GrantSubject::Principal {
                principal_id: PROJECT_OWNER.to_owned(),
            },
            role_key: RoleKey::Owner,
            source: GrantSource::Automation,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .expect("grant project Configuration authority");
    tx.commit().await.expect("commit project-owner setup");
    let project_token =
        Hs256Verifier::new(SECRET).issue(PROJECT_OWNER, tenant, Duration::from_secs(300));
    let (status, project_configs) = call(
        &app,
        "GET",
        &format!("/v1/configurations?governing_scope_id={project_scope}&limit=20"),
        &project_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{project_configs}");
    assert_eq!(
        project_configs["artifacts"]
            .as_array()
            .expect("project Configuration page")
            .iter()
            .map(|artifact| artifact["id"].as_str().expect("artifact id"))
            .collect::<Vec<_>>(),
        vec![artifact_id]
    );

    let (status, session) = call(
        &app,
        "POST",
        "/v1/sessions",
        &token,
        Some("cpr30-session"),
        Some(json!({
            "workspace_id": workspace_id,
            "project_id": project_id,
            "client_name": "cpr30-test",
            "external_session_id": "cpr30-runtime-evidence"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{session}");
    let session_id = session["id"].as_str().expect("session id");

    let (status, appended) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{session_id}/events"),
        &token,
        None,
        Some(json!({
            "events": [{
                "event_type": "command.executed",
                "client_event_id": "cpr30-unreviewed-source",
                "occurred_at": "2026-08-25T10:00:00Z",
                "payload": {
                    "text": "CPR30-UNREVIEWED configuration evidence remains pending until reviewed."
                }
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{appended}");
    let (status, batch) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{session_id}/capture-batches"),
        &token,
        Some("cpr30-capture"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{batch}");
    let batch_id = batch["id"].as_str().expect("capture batch id");
    let sweep = capture_worker::sweep_once(
        &capture_worker::Deps {
            pool: state.pool.clone(),
            pdp: Arc::clone(&state.pdp),
            extractor: Arc::new(AnyExtractor::Deterministic(DeterministicExtractor::new())),
        },
        &capture_worker::Config {
            poll_interval: Duration::from_millis(1),
            lease_duration: Duration::from_secs(30),
            batches_per_tenant: 8,
            lease_owner: format!("cpr30-test-{}", TenantId::new()),
        },
    )
    .await
    .expect("extract configured capture batch");
    assert!(sweep.completed >= 1, "{sweep:?}");
    let (status, candidate_page) = call(
        &app,
        "GET",
        &format!("/v1/capture-candidates?batch_id={batch_id}&limit=20"),
        &token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{candidate_page}");
    let capture_candidate_id = candidate_page["candidates"][0]["id"]
        .as_str()
        .expect("capture candidate id");

    let (status, reviewed) = call(
        &app,
        "POST",
        "/v1/knowledge",
        &token,
        Some("cpr30-reviewed-knowledge"),
        Some(json!({
            "scope_id": project_scope,
            "project_id": project_id,
            "knowledge_type": "fact",
            "origin": "authored",
            "content": {
                "title": "Reviewed configuration evidence",
                "body_markdown": "CPR30-REVIEWED configuration evidence is published Knowledge.",
                "summary": "Published configuration evidence.",
                "tags": ["configuration"],
                "sensitivity": "internal",
                "confidence_permille": 950,
                "verification_metadata": {},
                "metadata": {"fixture": "CPR-30"}
            },
            "sources": [{
                "scope_id": project_scope,
                "source_type": "manual",
                "metadata": {"fixture": "CPR-30"}
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{reviewed}");
    assert_eq!(reviewed["outcome"], "applied", "{reviewed}");

    let (status, run) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{session_id}/context-runs"),
        &token,
        Some("cpr30-context"),
        Some(json!({"query": "configuration evidence", "budget_tokens": 900})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    assert_eq!(run["budget_tokens"], 256);
    assert_eq!(run["trace_retention_mode"], "redacted");
    assert_eq!(run["configuration_version_id"], second_version);
    assert_eq!(run["configuration_hash"], configuration_hash);
    assert!(
        run["rendered"]
            .as_str()
            .is_some_and(|rendered| rendered.contains("[UNREVIEWED CANDIDATE]")),
        "configured candidate channel was not rendered: {run}"
    );
    assert!(
        !run["rendered"]
            .as_str()
            .is_some_and(|rendered| rendered.contains("CPR30-REVIEWED")),
        "a disabled current-Knowledge channel supplied reviewed content: {run}"
    );
    let run_id = run["id"].as_str().expect("context run id");
    let (status, detail) = call(
        &app,
        "GET",
        &format!("/v1/context-runs/{run_id}"),
        &token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    let unreviewed = detail["selections"]
        .as_array()
        .expect("context selections")
        .iter()
        .find(|selection| selection["channel"] == "unreviewed_candidates")
        .expect("unreviewed selection remains distinct");
    assert_eq!(unreviewed["capture_candidate_id"], capture_candidate_id);
    assert!(
        detail["selections"]
            .as_array()
            .expect("context selections")
            .iter()
            .all(|selection| selection["channel"] != "current_knowledge")
    );
    assert!(unreviewed.get("knowledge_revision_id").is_none());
    assert!(
        unreviewed.get("unreviewed_candidate").is_none(),
        "redacted retention must not expose proposal plaintext: {unreviewed}"
    );

    let (status, disabled) = call(
        &app,
        "PATCH",
        &format!("/v1/configuration-bindings/{binding_id}"),
        &token,
        Some("cpr30-disable-unreviewed"),
        Some(json!({
            "expected_revision": 3,
            "artifact_id": artifact_id,
            "pinned_version_id": first_version,
            "enabled": true,
            "reason": "pin"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{disabled}");
    let (status, without_unreviewed) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{session_id}/context-runs"),
        &token,
        Some("cpr30-context-without-unreviewed"),
        Some(json!({"query": "configuration evidence", "budget_tokens": 900})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{without_unreviewed}");
    assert_eq!(
        without_unreviewed["configuration_version_id"],
        first_version
    );
    assert!(
        !without_unreviewed["rendered"]
            .as_str()
            .is_some_and(|rendered| rendered.contains("[UNREVIEWED CANDIDATE]")),
        "a configuration without the channel supplied pending content: {without_unreviewed}"
    );
    assert!(
        without_unreviewed["rendered"]
            .as_str()
            .is_some_and(|rendered| rendered.contains("CPR30-REVIEWED")),
        "the current-Knowledge-only version did not supply reviewed content: {without_unreviewed}"
    );

    let (status, pending) = call(
        &app,
        "POST",
        "/v1/configurations",
        &token,
        Some("cpr30-root-pending"),
        Some(json!({
            "governing_scope_id": root,
            "name": "tenant-review-required",
            "document": personal["document"],
            "source_template": "personal"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{pending}");
    assert_eq!(pending["outcome"], "pending_review", "{pending}");

    for (method, path) in [
        ("GET", "/v1/policy/default"),
        ("PUT", "/v1/policy/default"),
        (
            "GET",
            "/v1/admin/scopes/00000000-0000-0000-0000-000000000000/policy",
        ),
    ] {
        let (status, _) = call(&app, method, path, &token, None, None).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "old route survived: {method} {path}"
        );
    }

    let mut tx = rls::begin_tenant_tx(&state.pool, tenant)
        .await
        .expect("begin evidence checks");
    let versions = synveda_store::configuration::versions(
        &mut tx,
        tenant,
        artifact_id.parse().expect("artifact id"),
        None,
        20,
    )
    .await
    .expect("read immutable versions");
    assert_eq!(versions.len(), 2);
    let immutable = sqlx::query(
        "update configuration_versions set content_hash = content_hash where tenant_id = $1 and id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(first_version.parse::<synveda_types::ConfigurationVersionId>().expect("version id").as_uuid())
    .execute(&mut *tx)
    .await;
    assert!(immutable.is_err(), "immutable version accepted UPDATE");
    drop(tx);

    let opened = sqlx::query_scalar!(
        r#"select count(*) as "count!" from audit_log
           where tenant_id = $1 and action = 'configuration.change.opened'"#,
        tenant.as_uuid(),
    )
    .fetch_one(&state.pool)
    .await
    .expect("count Configuration audit opens");
    let applied = sqlx::query_scalar!(
        r#"select count(*) as "count!" from audit_log
           where tenant_id = $1 and action = 'configuration.change.applied'"#,
        tenant.as_uuid(),
    )
    .fetch_one(&state.pool)
    .await
    .expect("count Configuration audit applications");
    assert!(opened >= 6, "expected public Configuration change audits");
    assert!(applied >= 5, "expected applied Configuration audits");
}
