//! Acceptance for the one governed runtime Configuration path.
//!
//! Public mutations traverse the embedded PDP and typed VedaFlow command
//! layer; immutable versions and revisioned bindings then govern a real
//! context run. Store and adversarial RLS coverage live beside this test.

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
use synveda_ingest::capture_worker;
use synveda_ingest::extraction::{AnyExtractor, DeterministicExtractor};
use synveda_store::{access, identities, rls, scopes};
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

async fn admitted_with_pack(
    bind_initial_pack: bool,
) -> Option<(AppState, Router, TenantId, ScopeId, String)> {
    let url = tenant_fixture::database_url("configuration API test")?;
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
        &format!("cpr30-{}", tenant.as_uuid().simple()),
        "configuration acceptance",
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
        &mut tx,
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
    if bind_initial_pack {
        // Fixture bootstrap itself remains a typed VedaFlow change. It gives
        // the local project scope the permissive profile needed to prove
        // auto-apply; the public mutations below are the acceptance subject.
        configuration_support::bind_pack(
            &mut tx,
            tenant,
            root.id,
            synveda_policy::OPEN_COLLABORATION,
        )
        .await;
    }
    tx.commit().await.expect("commit bootstrap");
    let state = state(&url);
    let app = router(state.clone());
    let token = Hs256Verifier::new(SECRET).issue(ADMIN, tenant, Duration::from_secs(300));
    Some((state, app, tenant, root.id, token))
}

async fn admitted() -> Option<(AppState, Router, TenantId, ScopeId, String)> {
    admitted_with_pack(true).await
}

const SECOND_ADMIN: &str = "cpr30-second-admin";
const WORKSPACE_ADMIN: &str = "cpr30-workspace-admin";

/// Admit one more principal with one grant and return its token.
async fn admit(
    state: &AppState,
    tenant: TenantId,
    subject: &str,
    scope_id: ScopeId,
    role_key: RoleKey,
) -> String {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant)
        .await
        .expect("begin principal setup");
    let own_scope = scopes::ensure_principal_scope(&mut tx, tenant, subject, subject)
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
        own_scope.id,
    )
    .await
    .expect("create identity");
    access::create_grant(
        &mut tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id: tenant,
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
    .expect("grant principal");
    tx.commit().await.expect("commit principal setup");
    Hs256Verifier::new(SECRET).issue(subject, tenant, Duration::from_secs(300))
}

/// One canonical template's document, as the public API serves it.
async fn template_document(app: &Router, token: &str, name: &str) -> Value {
    let (status, templates) =
        call(app, "GET", "/v1/configuration-templates", token, None, None).await;
    assert_eq!(status, StatusCode::OK, "{templates}");
    templates["templates"]
        .as_array()
        .expect("templates")
        .iter()
        .find(|entry| entry["name"] == name)
        .unwrap_or_else(|| panic!("{name} template"))["document"]
        .clone()
}

/// The caller's view of the tenant's profile choice.
async fn profile(app: &Router, token: &str) -> Value {
    let (status, me) = call(app, "GET", "/v1/me", token, None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    me["onboarding"]["profile"].clone()
}

async fn create_workspace(app: &Router, token: &str, slug: &str) -> String {
    let (status, workspace) = call(
        app,
        "POST",
        "/v1/workspaces",
        token,
        Some(&format!("cpr41-{slug}-workspace")),
        Some(json!({"slug": slug, "display_name": slug})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workspace}");
    workspace["scope_id"]
        .as_str()
        .expect("workspace scope")
        .to_owned()
}

async fn create_canonical(
    app: &Router,
    token: &str,
    key: &str,
    governing_scope_id: &str,
    template: &str,
) -> Value {
    let document = template_document(app, token, template).await;
    let (status, created) = call(
        app,
        "POST",
        "/v1/configurations",
        token,
        Some(key),
        Some(json!({
            "governing_scope_id": governing_scope_id,
            "name": key,
            "document": document,
            "source_template": template
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created
}

async fn bind(app: &Router, token: &str, key: &str, scope_id: &str, artifact_id: &str) -> Value {
    let (status, bound) = call(
        app,
        "POST",
        "/v1/configuration-bindings",
        token,
        Some(key),
        Some(json!({"scope_id": scope_id, "artifact_id": artifact_id, "enabled": true})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{bound}");
    bound
}

#[tokio::test]
async fn first_exact_profile_adoption_at_the_root_is_governed_and_closes_once_bound() {
    let _guard = serial().await;
    let Some((state, app, tenant, root, first_admin)) = admitted_with_pack(false).await else {
        return;
    };
    let root_id = root.to_string();
    let workspace_scope = create_workspace(&app, &first_admin, "pulseboard").await;

    // Any root administrator may make the choice, not only the holder of the
    // bootstrap grant, and other people's grants do not close it.
    let token = admit(&state, tenant, SECOND_ADMIN, root, RoleKey::Administrator).await;
    let before = profile(&app, &token).await;
    assert_eq!(before["state"], "open", "{before}");
    assert_eq!(before["policy_pack"], "regulated-strict", "{before}");
    assert!(before.get("template").is_none(), "{before}");

    let created = create_canonical(&app, &token, "cpr41-first-profile", &root_id, "team").await;
    assert_eq!(created["outcome"], "applied", "{created}");
    let artifact_id = created["artifact_id"].as_str().expect("artifact id");
    let change_id = created["change_id"].as_str().expect("change id");
    let (status, proposal) = call(
        &app,
        "GET",
        &format!("/v1/proposals/{change_id}"),
        &token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{proposal}");
    assert_eq!(proposal["state"], "applied", "{proposal}");
    assert_eq!(
        proposal["artifact_references"][0]["family"],
        "configuration"
    );

    // Created and not yet bound: the server names the one artifact that
    // finishes the adoption, and the root still runs the fail-safe.
    let prepared = profile(&app, &token).await;
    assert_eq!(prepared["state"], "prepared", "{prepared}");
    assert_eq!(prepared["template"], "team", "{prepared}");
    assert_eq!(prepared["artifact_id"], artifact_id, "{prepared}");
    assert_eq!(prepared["policy_pack"], "regulated-strict", "{prepared}");

    let bound = bind(&app, &token, "cpr41-first-binding", &root_id, artifact_id).await;
    assert_eq!(bound["outcome"], "applied", "{bound}");
    let chosen = profile(&app, &token).await;
    assert_eq!(chosen["state"], "chosen", "{chosen}");
    assert_eq!(chosen["template"], "team", "{chosen}");
    assert_eq!(chosen["policy_pack"], "standard", "{chosen}");
    assert!(chosen.get("artifact_id").is_none(), "{chosen}");
    assert_eq!(profile(&app, &first_admin).await["state"], "chosen");

    // Workspaces inherit the root's profile.
    let (status, effective) = call(
        &app,
        "GET",
        &format!("/v1/configurations/effective?scope_id={workspace_scope}"),
        &token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{effective}");
    assert_eq!(effective["artifact_id"], artifact_id, "{effective}");
    assert_eq!(
        effective["document"],
        template_document(&app, &token, "team").await,
        "{effective}"
    );

    // Once the root is bound, even a byte-for-byte canonical profile follows
    // the configured approval matrix. Adoption is not a general fast path.
    let second = create_canonical(&app, &token, "cpr41-second-profile", &root_id, "personal").await;
    assert_eq!(second["outcome"], "pending_review", "{second}");
    let pending_artifact = second["artifact_id"]
        .as_str()
        .expect("pending command reserves an aggregate id");
    let (status, _) = call(
        &app,
        "GET",
        &format!("/v1/configurations/{pending_artifact}"),
        &token,
        None,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "pending review must not materialise its reserved aggregate"
    );

    // Audit alone says who chose which template and pack, where, over which
    // exact document (ADR-0162 decision 5).
    let mut tx = tenant_fixture::begin(&state.pool, tenant).await;
    let events: Vec<(String, String, Value)> = sqlx::query_as(
        "select action, actor_subject, payload from audit_log
          where tenant_id = $1
            and action in ('configuration.change.opened', 'configuration.change.applied')
          order by seq",
    )
    .bind(tenant.as_uuid())
    .fetch_all(&mut *tx)
    .await
    .expect("read Configuration audit");
    tx.commit().await.expect("commit Configuration audit read");
    let opened = events
        .iter()
        .filter(|(action, _, _)| action == "configuration.change.opened")
        .count();
    let applied = events
        .iter()
        .filter(|(action, _, _)| action == "configuration.change.applied")
        .count();
    assert_eq!(opened, 3, "every attempt retains a VedaFlow change");
    assert_eq!(applied, 2, "only the root create and bind auto-apply");
    let team_hash = synveda_types::configuration::ConfigurationDocument::template(
        synveda_types::configuration::ConfigurationTemplate::Team,
    )
    .content_hash()
    .expect("hash team template");
    let adoptions: Vec<_> = events
        .iter()
        .filter(|(_, _, payload)| payload["initial_profile_adoption"] == true)
        .collect();
    assert_eq!(
        adoptions.len(),
        4,
        "the create and bind each open and apply"
    );
    for (_, actor, payload) in adoptions {
        assert_eq!(actor, SECOND_ADMIN, "{payload}");
        let adopted = &payload["adopted_profile"];
        assert_eq!(adopted["template"], "team", "{payload}");
        assert_eq!(adopted["policy_pack"], "standard", "{payload}");
        assert_eq!(adopted["target_scope_id"], root_id, "{payload}");
        assert_eq!(adopted["document_hash"], team_hash, "{payload}");
    }
}

#[tokio::test]
async fn a_workspace_profile_follows_the_live_matrix_and_leaves_the_root_choice_open() {
    let _guard = serial().await;
    let Some((state, app, tenant, root, token)) = admitted_with_pack(false).await else {
        return;
    };
    let root_id = root.to_string();
    let workspace_scope = create_workspace(&app, &token, "first-use").await;

    // The v0.4.x wizard's first profile: canonical, at a workspace, by a root
    // administrator. It no longer auto-applies.
    let at_workspace = create_canonical(
        &app,
        &token,
        "cpr41-workspace-profile",
        &workspace_scope,
        "team",
    )
    .await;
    assert_eq!(at_workspace["outcome"], "pending_review", "{at_workspace}");
    assert_eq!(profile(&app, &token).await["state"], "open");

    // A tenant onboarded that way already has a profile bound at a workspace.
    // That does not close the tenant's choice either.
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant)
        .await
        .expect("begin workspace profile fixture");
    configuration_support::bind_pack(
        &mut tx,
        tenant,
        workspace_scope.parse().expect("workspace scope id"),
        synveda_policy::OPEN_COLLABORATION,
    )
    .await;
    tx.commit().await.expect("commit workspace profile fixture");
    assert_eq!(profile(&app, &token).await["state"], "open");

    // An administrator granted only at the workspace cannot make the
    // tenant's choice.
    let workspace_admin = admit(
        &state,
        tenant,
        WORKSPACE_ADMIN,
        workspace_scope.parse().expect("workspace scope id"),
        RoleKey::Administrator,
    )
    .await;
    let workspace_view = profile(&app, &workspace_admin).await;
    assert!(workspace_view.is_null(), "{workspace_view}");
    let document = template_document(&app, &token, "personal").await;
    let (status, refused) = call(
        &app,
        "POST",
        "/v1/configurations",
        &workspace_admin,
        Some("cpr41-workspace-admin-root"),
        Some(json!({
            "governing_scope_id": root_id,
            "name": "workspace-admin-root",
            "document": document,
            "source_template": "personal"
        })),
    )
    .await;
    assert!(
        status == StatusCode::FORBIDDEN || refused["outcome"] == "pending_review",
        "{status} {refused}"
    );

    let created = create_canonical(&app, &token, "cpr41-root-profile", &root_id, "team").await;
    assert_eq!(created["outcome"], "applied", "{created}");
    let artifact_id = created["artifact_id"].as_str().expect("artifact id");
    let bound = bind(&app, &token, "cpr41-root-binding", &root_id, artifact_id).await;
    assert_eq!(bound["outcome"], "applied", "{bound}");
    assert_eq!(profile(&app, &token).await["state"], "chosen");
}

#[tokio::test]
async fn edited_document_cannot_claim_the_first_profile_adoption() {
    let _guard = serial().await;
    let Some((_state, app, _tenant, root, token)) = admitted_with_pack(false).await else {
        return;
    };
    let mut document = template_document(&app, &token, "personal").await;
    document["context"]["token_budget"] = json!(1234);

    let (status, result) = call(
        &app,
        "POST",
        "/v1/configurations",
        &token,
        Some("cpr41-edited-first-profile"),
        Some(json!({
            "governing_scope_id": root,
            "name": "not-the-canonical-profile",
            "document": document
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{result}");
    assert_eq!(result["outcome"], "pending_review", "{result}");
    let pending_artifact = result["artifact_id"]
        .as_str()
        .expect("pending command reserves an aggregate id");
    let (status, _) = call(
        &app,
        "GET",
        &format!("/v1/configurations/{pending_artifact}"),
        &token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(profile(&app, &token).await["state"], "open");
}

#[tokio::test]
async fn a_disabled_root_binding_cannot_close_the_first_profile_choice() {
    let _guard = serial().await;
    let Some((_state, app, _tenant, root, token)) = admitted_with_pack(false).await else {
        return;
    };
    let root_id = root.to_string();
    let created = create_canonical(&app, &token, "cpr41-disabled-profile", &root_id, "team").await;
    assert_eq!(created["outcome"], "applied", "{created}");
    let artifact_id = created["artifact_id"].as_str().expect("artifact id");

    let (status, disabled) = call(
        &app,
        "POST",
        "/v1/configuration-bindings",
        &token,
        Some("cpr41-disabled-binding"),
        Some(json!({"scope_id": root_id, "artifact_id": artifact_id, "enabled": false})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{disabled}");
    assert_eq!(disabled["outcome"], "pending_review", "{disabled}");
    let prepared = profile(&app, &token).await;
    assert_eq!(prepared["state"], "prepared", "{prepared}");
    assert_eq!(prepared["artifact_id"], artifact_id, "{prepared}");

    let bound = bind(&app, &token, "cpr41-enabled-binding", &root_id, artifact_id).await;
    assert_eq!(bound["outcome"], "applied", "{bound}");
    assert_eq!(profile(&app, &token).await["state"], "chosen");
}

#[tokio::test]
async fn concurrent_first_profile_requests_have_exactly_one_winner() {
    let _guard = serial().await;
    let Some((_state, app, _tenant, root, token)) = admitted_with_pack(false).await else {
        return;
    };
    let document = template_document(&app, &token, "personal").await;
    let body = |name: &str| {
        json!({
            "governing_scope_id": root,
            "name": name,
            "document": document,
            "source_template": "personal"
        })
    };
    let (left, right) = tokio::join!(
        call(
            &app,
            "POST",
            "/v1/configurations",
            &token,
            Some("cpr41-race-left"),
            Some(body("left-profile")),
        ),
        call(
            &app,
            "POST",
            "/v1/configurations",
            &token,
            Some("cpr41-race-right"),
            Some(body("right-profile")),
        )
    );
    assert_eq!(left.0, StatusCode::CREATED, "{}", left.1);
    assert_eq!(right.0, StatusCode::CREATED, "{}", right.1);
    let outcomes = [left.1["outcome"].as_str(), right.1["outcome"].as_str()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == Some("applied"))
            .count(),
        1,
        "only one concurrent request may be first: {outcomes:?}"
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == Some("pending_review"))
            .count(),
        1,
        "the loser follows the configured matrix: {outcomes:?}"
    );
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
    let (proposal_status, proposal) = call(
        &app,
        "GET",
        &format!("/v1/proposals/{first_change}"),
        &token,
        None,
        None,
    )
    .await;
    assert_eq!(proposal_status, StatusCode::OK, "{proposal}");
    assert_eq!(
        proposal["artifact_references"][0]["family"],
        "configuration"
    );
    assert_eq!(
        proposal["artifact_references"][0]["artifact_id"],
        artifact_id
    );
    assert_eq!(proposal["artifact_references"][0]["version"], first_version);
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
        &mut tx,
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
            ..capture_worker::Config::default()
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
                "metadata": {"fixture": "configuration"}
            },
            "sources": [{
                "scope_id": project_scope,
                "source_type": "manual",
                "metadata": {"fixture": "configuration"}
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
        run["rendered"].as_str().is_some_and(|rendered| rendered
            .contains(r#""kind":"unreviewed_candidate","review_state":"pending_review""#)),
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

    let mut tx = tenant_fixture::begin(&state.pool, tenant).await;
    let opened = sqlx::query_scalar!(
        r#"select count(*) as "count!" from audit_log
           where tenant_id = $1 and action = 'configuration.change.opened'"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut *tx)
    .await
    .expect("count Configuration audit opens");
    let applied = sqlx::query_scalar!(
        r#"select count(*) as "count!" from audit_log
           where tenant_id = $1 and action = 'configuration.change.applied'"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut *tx)
    .await
    .expect("count Configuration audit applications");
    assert!(opened >= 6, "expected public Configuration change audits");
    assert!(applied >= 5, "expected applied Configuration audits");
    let untyped_terminal = sqlx::query_scalar!(
        r#"select count(*) as "count!" from audit_log
           where tenant_id = $1
             and action in ('configuration.change.applied', 'configuration.change.rejected')
             and not (payload @> '{"artifact_references":[{"family":"configuration"}]}'::jsonb)"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut *tx)
    .await
    .expect("check terminal Configuration artifact references");
    tx.commit()
        .await
        .expect("commit Configuration evidence read");
    assert_eq!(
        untyped_terminal, 0,
        "terminal Configuration evidence lost its typed address"
    );
}
