//! CPR-4 acceptance criteria at the HTTP surface (ADR-0071): the workspace,
//! project and repository plane, `/v1/me`, the idempotency seam and the
//! revision precondition.
//!
//! The store-level contract — the subtype/scope transaction, the structural
//! rules against direct SQL, canonicalisation — is
//! `crates/synveda-store/tests/workspaces.rs`. This suite proves the things
//! only the HTTP surface has: status codes, the PDP on every route, the audit
//! events, and the two mechanisms that exist because this plane's callers
//! retry.
//!
//! Tests that need a live Postgres read `DATABASE_URL` and skip with a message
//! when it is unset (CI has no database); run them locally with
//! `make db-test`.

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
use synveda_types::{GrantId, TenantId};
use tower::ServiceExt;

const SECRET: &[u8] = b"cpr-4-workspaces-test-secret";
/// The subject the admin token carries — bound tenant-wide `org-admin`, which
/// is what every shipped pack prices the mutating admin planes at.
const ADMIN: &str = "cpr4-admin";
/// A placed identity with no role binding at all: the caller every "denies
/// without the action" assertion uses.
const OUTSIDER: &str = "cpr4-outsider";

/// Serialises tests: the Prometheus recorder is process-global (same rationale
/// as tests/tenant_resolution.rs).
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
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(5))
            .connect_lazy(url)
            .expect("parse database url"),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp: Arc::new(synveda_policy::Pdp::new().expect("build the embedded PDP")),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index: Arc::new(
            synveda_retrieval::SearchIndex::open(
                std::env::temp_dir()
                    .join("synveda-cpr4-tests")
                    .join(TenantId::new().to_string()),
            )
            .expect("open search index"),
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

/// Connects, migrates, admits a tenant, and binds the admin subject
/// tenant-wide `org-admin` — the CLI's bootstrap path, and what a person
/// running `synveda init` holds after their first login. Enforcement still
/// runs through the PDP with this row as data.
async fn admitted_tenant() -> Option<(AppState, TenantId)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping workspace API test: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
            return None;
        }
    };
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");
    let id = TenantId::new();
    let slug = format!("cpr4-{}", id.as_uuid().simple());
    synveda_store::tenants::create(
        &pool,
        id,
        &slug,
        "CPR-4 API test",
        synveda_types::TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    let mut tx = synveda_store::rls::begin_tenant_tx(&pool, id)
        .await
        .expect("begin tenant tx");
    let root = synveda_store::scopes::ensure_tenant_root(&mut tx, id)
        .await
        .expect("mint root");
    synveda_store::access::create_grant(
        &mut *tx,
        &synveda_store::access::NewGrant {
            id: GrantId::new(),
            tenant_id: id,
            scope_id: root.id,
            subject: synveda_types::access::GrantSubject::Principal {
                principal_id: ADMIN.to_owned(),
            },
            role_key: synveda_types::access::RoleKey::Administrator,
            source: synveda_types::access::GrantSource::Automation,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .expect("grant admin at the root");
    tx.commit().await.expect("commit grant");
    Some((state(&url), id))
}

/// One API call. `key` is the `Idempotency-Key`, when the route takes one.
async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    key: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    if let Some(key) = key {
        request = request.header("idempotency-key", key);
    }
    let request = match body {
        Some(body) => request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("build request"),
        None => request.body(Body::empty()).expect("build request"),
    };
    let response = app.clone().oneshot(request).await.expect("router responds");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

/// Creates one workspace through the API and returns it.
async fn seed_workspace(app: &Router, token: &str, slug: &str) -> Value {
    let (status, workspace) = call(
        app,
        "POST",
        "/v1/workspaces",
        Some(token),
        Some(&format!("seed-{slug}")),
        Some(json!({"slug": slug, "display_name": slug})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workspace}");
    workspace
}

async fn seed_project(app: &Router, token: &str, workspace_id: &str, slug: &str) -> Value {
    let (status, project) = call(
        app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/projects"),
        Some(token),
        Some(&format!("seed-project-{slug}")),
        Some(json!({"slug": slug, "display_name": slug})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    project
}

/// Every audit action in the tenant's chain, in order.
async fn chain_actions(state: &AppState, tenant_id: TenantId) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "select action from audit_log where tenant_id = $1 order by seq",
    )
    .bind(tenant_id.as_uuid())
    .fetch_all(&state.pool)
    .await
    .expect("read the chain")
}

// ── The whole path, once ─────────────────────────────────────────────────────

/// The end-to-end shape: nothing, then a workspace, then a project, then a
/// repository — with `/v1/me` telling the client what to do next at every
/// step, and the audit chain recording each act.
#[tokio::test]
async fn a_person_goes_from_nothing_to_a_project_with_a_repository() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);

    // Nothing exists, and the server says what is missing rather than leaving
    // the client to infer it from an empty list.
    let (status, me) = call(&app, "GET", "/v1/me", Some(&token), None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["onboarding"]["state"], "needs_workspace");
    assert_eq!(me["onboarding"]["workspace_count"], 0);
    // The tenant root exists after this call and **nobody was asked for it**:
    // `/v1/me` mints the caller's own `principal` scope (CPR-6, ADR-0073
    // decision 2), which needs a root, and the root is derived from the
    // `tenants` row. Until CPR-6 the first workspace minted it and this
    // assertion read `is_null()`; what the claim was ever about is that a
    // person is not asked to declare an organisation, and that is unchanged.
    assert!(
        !me["onboarding"]["tenant_scope_id"].is_null(),
        "the product mints the root; nobody is asked to declare one: {me}"
    );
    assert_eq!(
        me["anchors"][0]["source"], "principal_scope",
        "and the caller's own scope is where they stand: {me}"
    );
    assert_eq!(me["principal"]["subject"], ADMIN);
    assert_eq!(
        me["capabilities"]["actions"]["workspace.create"], true,
        "an org-admin may create a workspace: {me}"
    );

    let workspace = seed_workspace(&app, &token, "payments").await;
    assert_eq!(workspace["slug"], "payments");
    assert_eq!(workspace["revision"], 1);
    assert!(
        workspace["scope_id"].is_string(),
        "a workspace owns a scope: {workspace}"
    );
    let workspace_id = workspace["id"].as_str().expect("id").to_owned();

    let (_, me) = call(&app, "GET", "/v1/me", Some(&token), None, None).await;
    assert_eq!(me["onboarding"]["state"], "needs_project");
    assert!(
        me["onboarding"]["tenant_scope_id"].is_string(),
        "the first workspace minted the tenant root: {me}"
    );
    assert_eq!(me["workspaces"].as_array().expect("workspaces").len(), 1);

    let project = seed_project(&app, &token, &workspace_id, "ledger").await;
    let project_id = project["id"].as_str().expect("id").to_owned();
    assert_eq!(project["workspace_id"], workspace_id.as_str());

    let (status, repository) = call(
        &app,
        "POST",
        &format!("/v1/projects/{project_id}/repositories"),
        Some(&token),
        Some("attach-1"),
        Some(json!({
            "remote_uri": "git@github.com:Acme/payments.git",
            "default_branch": "main",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{repository}");
    assert_eq!(
        repository["canonical_uri"], "https://github.com/Acme/payments",
        "the identity is canonical, not what the caller typed"
    );
    assert_eq!(repository["provider"], "github");

    let (_, me) = call(&app, "GET", "/v1/me", Some(&token), None, None).await;
    assert_eq!(me["onboarding"]["state"], "ready");
    assert_eq!(me["projects"].as_array().expect("projects").len(), 1);

    // The chain records every act, in order, under its own action name.
    let actions = chain_actions(&state, tenant_id).await;
    for expected in [
        "workspace.created",
        "project.created",
        "project.repository.attached",
    ] {
        assert!(
            actions.iter().any(|action| action == expected),
            "the chain must record {expected}: {actions:?}"
        );
    }
    assert!(
        !actions
            .iter()
            .any(|action| action.starts_with("hierarchy.")),
        "a workspace creation must not be recorded as a hierarchy act: {actions:?}"
    );
}

// ── Idempotency ──────────────────────────────────────────────────────────────

/// The guarantee this plane exists to give a retrying client: the same key with
/// the same request returns the original resource and creates nothing.
#[tokio::test]
async fn a_replayed_creation_returns_the_original_and_creates_nothing() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let token = issue(ADMIN, tenant_id);
    let body = json!({"slug": "payments", "display_name": "Payments"});

    let (first_status, first) = call(
        &app,
        "POST",
        "/v1/workspaces",
        Some(&token),
        Some("k-1"),
        Some(body.clone()),
    )
    .await;
    assert_eq!(first_status, StatusCode::CREATED);

    let (second_status, second) = call(
        &app,
        "POST",
        "/v1/workspaces",
        Some(&token),
        Some("k-1"),
        Some(body),
    )
    .await;
    assert_eq!(
        second_status,
        StatusCode::OK,
        "a replay is 200, so a client can tell it from a creation"
    );
    assert_eq!(second["id"], first["id"], "the same resource, not a second");

    let (_, list) = call(&app, "GET", "/v1/workspaces", Some(&token), None, None).await;
    assert_eq!(
        list["workspaces"].as_array().expect("workspaces").len(),
        1,
        "the retry created nothing: {list}"
    );
}

/// A key reused for a *different* request is a conflict. The alternative is
/// answering a request the caller did not make with the resource from one they
/// did, and reporting it as success.
#[tokio::test]
async fn a_key_reused_for_a_different_request_is_a_conflict() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let token = issue(ADMIN, tenant_id);

    let (status, _) = call(
        &app,
        "POST",
        "/v1/workspaces",
        Some(&token),
        Some("k-1"),
        Some(json!({"slug": "payments", "display_name": "Payments"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, error) = call(
        &app,
        "POST",
        "/v1/workspaces",
        Some(&token),
        Some("k-1"),
        Some(json!({"slug": "ledger", "display_name": "Ledger"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert_eq!(error["kind"], "conflict");
}

/// Reformatting the body between a request and its retry does not change the
/// request, so it must not read as a conflict.
#[tokio::test]
async fn a_reordered_body_is_the_same_request() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let token = issue(ADMIN, tenant_id);

    let (status, first) = call(
        &app,
        "POST",
        "/v1/workspaces",
        Some(&token),
        Some("k-1"),
        Some(json!({"slug": "payments", "display_name": "Payments"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, second) = call(
        &app,
        "POST",
        "/v1/workspaces",
        Some(&token),
        Some("k-1"),
        Some(json!({"display_name": "Payments", "slug": "payments"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["id"], first["id"]);
}

/// The header is required, and the refusal says so — a caller that forgot
/// learns from a 400 rather than from two workspaces.
#[tokio::test]
async fn a_creation_without_an_idempotency_key_is_refused() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let token = issue(ADMIN, tenant_id);

    let (status, error) = call(
        &app,
        "POST",
        "/v1/workspaces",
        Some(&token),
        None,
        Some(json!({"slug": "payments", "display_name": "Payments"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert!(
        error["message"]
            .as_str()
            .unwrap_or_default()
            .contains("Idempotency-Key"),
        "the refusal names the header: {error}"
    );
}

/// A slug conflict is the creation's own conflict and must surface as one —
/// not be swallowed by the idempotency seam's re-lookup.
#[tokio::test]
async fn a_taken_slug_is_a_conflict_and_not_a_replay() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let token = issue(ADMIN, tenant_id);
    seed_workspace(&app, &token, "payments").await;

    let (status, error) = call(
        &app,
        "POST",
        "/v1/workspaces",
        Some(&token),
        Some("a-different-key"),
        Some(json!({"slug": "payments", "display_name": "Payments again"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
}

/// One client's key is not another's: an idempotency key is a token a client
/// mints for itself, with no coordination.
#[tokio::test]
async fn one_subjects_key_does_not_shadow_anothers() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    // A second admin, so both callers hold the action.
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin");
    let root = synveda_store::scopes::ensure_tenant_root(&mut tx, tenant_id)
        .await
        .expect("mint root");
    synveda_store::access::create_grant(
        &mut *tx,
        &synveda_store::access::NewGrant {
            id: GrantId::new(),
            tenant_id,
            scope_id: root.id,
            subject: synveda_types::access::GrantSubject::Principal {
                principal_id: "cpr4-admin-2".to_owned(),
            },
            role_key: synveda_types::access::RoleKey::Administrator,
            source: synveda_types::access::GrantSource::Automation,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .expect("grant");
    tx.commit().await.expect("commit");

    let app = router(state);
    let (status, first) = call(
        &app,
        "POST",
        "/v1/workspaces",
        Some(&issue(ADMIN, tenant_id)),
        Some("shared-key"),
        Some(json!({"slug": "payments", "display_name": "Payments"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, second) = call(
        &app,
        "POST",
        "/v1/workspaces",
        Some(&issue("cpr4-admin-2", tenant_id)),
        Some("shared-key"),
        Some(json!({"slug": "ledger", "display_name": "Ledger"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "another subject's identical key is its own: {second}"
    );
    assert_ne!(second["id"], first["id"]);
}

// ── Revision preconditions ───────────────────────────────────────────────────

/// The lost-update protection at the HTTP surface: the stale writer gets a 409
/// and nothing it sent is applied.
#[tokio::test]
async fn a_stale_revision_is_a_conflict_that_writes_nothing() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let token = issue(ADMIN, tenant_id);
    let workspace = seed_workspace(&app, &token, "payments").await;
    let id = workspace["id"].as_str().expect("id").to_owned();

    let (status, updated) = call(
        &app,
        "PATCH",
        &format!("/v1/workspaces/{id}"),
        Some(&token),
        None,
        Some(json!({"expected_revision": 1, "display_name": "Payments platform"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["revision"], 2);

    let (status, error) = call(
        &app,
        "PATCH",
        &format!("/v1/workspaces/{id}"),
        Some(&token),
        None,
        Some(json!({"expected_revision": 1, "display_name": "Payments (old)"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");

    let (_, current) = call(
        &app,
        "GET",
        &format!("/v1/workspaces/{id}"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(current["display_name"], "Payments platform");
    assert_eq!(current["revision"], 2, "a refused update bumps nothing");
}

/// The precondition is required by the wire, not defaulted — an update with no
/// precondition is a last-writer-wins update.
#[tokio::test]
async fn an_update_without_a_precondition_is_refused() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let token = issue(ADMIN, tenant_id);
    let workspace = seed_workspace(&app, &token, "payments").await;
    let id = workspace["id"].as_str().expect("id");

    let (status, error) = call(
        &app,
        "PATCH",
        &format!("/v1/workspaces/{id}"),
        Some(&token),
        None,
        Some(json!({"display_name": "Payments platform"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
}

/// An update's audit event carries the precondition it was applied under, so
/// the chain says why a refused writer's change is absent.
#[tokio::test]
async fn an_update_event_records_the_precondition() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let workspace = seed_workspace(&app, &token, "payments").await;
    let id = workspace["id"].as_str().expect("id");
    call(
        &app,
        "PATCH",
        &format!("/v1/workspaces/{id}"),
        Some(&token),
        None,
        Some(json!({"expected_revision": 1, "display_name": "Payments platform"})),
    )
    .await;

    let payload: Value = sqlx::query_scalar::<_, Value>(
        "select payload from audit_log where tenant_id = $1 and action = 'workspace.updated'",
    )
    .bind(tenant_id.as_uuid())
    .fetch_one(&state.pool)
    .await
    .expect("read the event");
    assert_eq!(payload["expected_revision"], 1);
    // `seed_workspace` names a workspace after its slug, so the "before" image
    // is the slug and the "after" is what the update sent.
    assert_eq!(payload["before"]["display_name"], "payments");
    assert_eq!(payload["after"]["display_name"], "Payments platform");
}

// ── Governance ───────────────────────────────────────────────────────────────

/// Every mutation on this plane denies without the action, and every read
/// denies without its own. The outsider is a real subject with a real token and
/// no binding — the caller a pack must be able to refuse.
#[tokio::test]
async fn every_route_denies_without_the_action() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let admin = issue(ADMIN, tenant_id);
    let outsider = issue(OUTSIDER, tenant_id);
    let workspace = seed_workspace(&app, &admin, "payments").await;
    let workspace_id = workspace["id"].as_str().expect("id").to_owned();
    let project = seed_project(&app, &admin, &workspace_id, "ledger").await;
    let project_id = project["id"].as_str().expect("id").to_owned();

    for (method, path, key, body) in [
        (
            "POST",
            "/v1/workspaces".to_owned(),
            Some("outsider-1"),
            Some(json!({"slug": "theirs", "display_name": "Theirs"})),
        ),
        (
            "PATCH",
            format!("/v1/workspaces/{workspace_id}"),
            None,
            Some(json!({"expected_revision": 1, "display_name": "Mine now"})),
        ),
        (
            "POST",
            format!("/v1/workspaces/{workspace_id}/projects"),
            Some("outsider-2"),
            Some(json!({"slug": "theirs", "display_name": "Theirs"})),
        ),
        (
            "PATCH",
            format!("/v1/projects/{project_id}"),
            None,
            Some(json!({"expected_revision": 1, "display_name": "Mine now"})),
        ),
        (
            "POST",
            format!("/v1/projects/{project_id}/repositories"),
            Some("outsider-3"),
            Some(json!({"remote_uri": "https://github.com/acme/theirs"})),
        ),
        ("GET", "/v1/workspaces".to_owned(), None, None),
        ("GET", format!("/v1/workspaces/{workspace_id}"), None, None),
        (
            "GET",
            format!("/v1/workspaces/{workspace_id}/projects"),
            None,
            None,
        ),
        ("GET", format!("/v1/projects/{project_id}"), None, None),
        (
            "GET",
            format!("/v1/projects/{project_id}/repositories"),
            None,
            None,
        ),
    ] {
        let (status, error) = call(&app, method, &path, Some(&outsider), key, body).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{method} {path} must be denied for a caller with no role: {error}"
        );
        assert_eq!(error["kind"], "policy_denied", "{method} {path}: {error}");
    }
}

/// **A replay still takes the decision.** A caller whose permission is revoked
/// between the first attempt and the retry must be refused — a replay that
/// skipped the PDP would be a cached authorisation.
#[tokio::test]
async fn a_replay_still_takes_the_pdp_decision() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let body = json!({"slug": "payments", "display_name": "Payments"});
    let (status, _) = call(
        &app,
        "POST",
        "/v1/workspaces",
        Some(&token),
        Some("k-1"),
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Revoke the grant the decision rested on.
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin");
    let grants = synveda_store::access::list_grants(
        &mut *tx,
        tenant_id,
        &synveda_store::access::GrantFilter {
            scope_id: None,
            principal_id: Some(ADMIN.to_owned()),
        },
    )
    .await
    .expect("list grants");
    let grant = grants
        .iter()
        .find(|grant| grant.role_key == synveda_types::access::RoleKey::Administrator)
        .expect("admin grant");
    synveda_store::access::revoke_grant(&mut tx, tenant_id, grant.id)
        .await
        .expect("revoke");
    tx.commit().await.expect("commit");
    state.invalidate_scopes(tenant_id);

    let (status, error) = call(
        &app,
        "POST",
        "/v1/workspaces",
        Some(&token),
        Some("k-1"),
        Some(body),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the replay must be decided again, not served from the record: {error}"
    );
}

/// A caller who may read nothing gets an answer rather than a refusal from
/// `/v1/me`: an empty inventory and a state that says so.
#[tokio::test]
async fn me_answers_a_caller_who_can_see_nothing() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    seed_workspace(&app, &issue(ADMIN, tenant_id), "payments").await;

    let (status, me) = call(
        &app,
        "GET",
        "/v1/me",
        Some(&issue(OUTSIDER, tenant_id)),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(
        me["workspaces"].as_array().expect("workspaces").len(),
        0,
        "a caller who may not read workspaces is told nothing about them: {me}"
    );
    assert_eq!(
        me["onboarding"]["state"], "needs_workspace",
        "and gets a state rather than a 403"
    );
}

/// Every route on this plane is authenticated.
#[tokio::test]
async fn every_route_refuses_an_unauthenticated_caller() {
    let _guard = serial().await;
    let Some((state, _)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let id = TenantId::new().to_string();
    for (method, path) in [
        ("GET", "/v1/me".to_owned()),
        ("GET", "/v1/workspaces".to_owned()),
        ("POST", "/v1/workspaces".to_owned()),
        ("GET", format!("/v1/workspaces/{id}")),
        ("PATCH", format!("/v1/workspaces/{id}")),
        ("GET", format!("/v1/workspaces/{id}/projects")),
        ("POST", format!("/v1/workspaces/{id}/projects")),
        ("GET", format!("/v1/projects/{id}")),
        ("PATCH", format!("/v1/projects/{id}")),
        ("GET", format!("/v1/projects/{id}/repositories")),
        ("POST", format!("/v1/projects/{id}/repositories")),
        ("DELETE", format!("/v1/projects/{id}/repositories/{id}")),
    ] {
        let (status, _) = call(&app, method, &path, None, None, None).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{method} {path} must refuse an unauthenticated caller"
        );
    }
}

/// Another tenant's workspace is a 404 and never a policy-denial oracle.
#[tokio::test]
async fn another_tenants_workspace_is_not_found() {
    let _guard = serial().await;
    let Some((state, mine)) = admitted_tenant().await else {
        return;
    };
    let Some((_, theirs)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let workspace = seed_workspace(&app, &issue(ADMIN, theirs), "payments").await;
    let id = workspace["id"].as_str().expect("id");

    let (status, error) = call(
        &app,
        "GET",
        &format!("/v1/workspaces/{id}"),
        Some(&issue(ADMIN, mine)),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{error}");
    assert_eq!(error["kind"], "not_found");
}

// ── Repositories ─────────────────────────────────────────────────────────────

/// A filesystem path is refused at the surface, with a message naming what to
/// send instead — and the refusal happens before anything is stored.
#[tokio::test]
async fn a_filesystem_path_is_refused_with_a_usable_message() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let token = issue(ADMIN, tenant_id);
    let workspace = seed_workspace(&app, &token, "payments").await;
    let project = seed_project(
        &app,
        &token,
        workspace["id"].as_str().expect("id"),
        "ledger",
    )
    .await;
    let project_id = project["id"].as_str().expect("id").to_owned();

    let (status, error) = call(
        &app,
        "POST",
        &format!("/v1/projects/{project_id}/repositories"),
        Some(&token),
        Some("attach-1"),
        Some(json!({"remote_uri": "/Users/sam/src/payments"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    let message = error["message"].as_str().unwrap_or_default();
    assert!(message.contains("local_fingerprint"), "{message}");

    let (_, list) = call(
        &app,
        "GET",
        &format!("/v1/projects/{project_id}/repositories"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert!(
        list["repositories"]
            .as_array()
            .expect("repositories")
            .is_empty(),
        "nothing was stored: {list}"
    );
}

/// Attach, list, detach — and a repeated detach is a 404 rather than a silent
/// success.
#[tokio::test]
async fn a_repository_can_be_attached_listed_and_detached() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let workspace = seed_workspace(&app, &token, "payments").await;
    let project = seed_project(
        &app,
        &token,
        workspace["id"].as_str().expect("id"),
        "ledger",
    )
    .await;
    let project_id = project["id"].as_str().expect("id").to_owned();

    let (status, attached) = call(
        &app,
        "POST",
        &format!("/v1/projects/{project_id}/repositories"),
        Some(&token),
        Some("attach-1"),
        Some(json!({"remote_uri": "https://github.com/acme/payments.git"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{attached}");
    let repository_id = attached["id"].as_str().expect("id").to_owned();

    let (_, list) = call(
        &app,
        "GET",
        &format!("/v1/projects/{project_id}/repositories"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(list["repositories"].as_array().expect("list").len(), 1);

    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/v1/projects/{project_id}/repositories/{repository_id}"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/v1/projects/{project_id}/repositories/{repository_id}"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a repeated detach reports there was nothing to detach"
    );

    let actions = chain_actions(&state, tenant_id).await;
    assert!(
        actions
            .iter()
            .any(|action| action == "project.repository.detached"),
        "{actions:?}"
    );
}

/// A credential pasted into a remote never reaches the response — nor the
/// audit payload, which is where it would do the most damage.
#[tokio::test]
async fn a_credential_in_a_remote_never_reaches_a_row_or_the_chain() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let workspace = seed_workspace(&app, &token, "payments").await;
    let project = seed_project(
        &app,
        &token,
        workspace["id"].as_str().expect("id"),
        "ledger",
    )
    .await;
    let project_id = project["id"].as_str().expect("id").to_owned();

    let (status, attached) = call(
        &app,
        "POST",
        &format!("/v1/projects/{project_id}/repositories"),
        Some(&token),
        Some("attach-1"),
        Some(json!({
            "remote_uri": "https://x-access-token:ghp_supersecret@github.com/acme/payments.git",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{attached}");
    assert_eq!(
        attached["canonical_uri"],
        "https://github.com/acme/payments"
    );
    assert!(
        !attached.to_string().contains("ghp_"),
        "the response must not echo the credential: {attached}"
    );

    let payload: Value = sqlx::query_scalar::<_, Value>(
        "select payload from audit_log \
         where tenant_id = $1 and action = 'project.repository.attached'",
    )
    .bind(tenant_id.as_uuid())
    .fetch_one(&state.pool)
    .await
    .expect("read the event");
    assert!(
        !payload.to_string().contains("ghp_"),
        "the audit chain must not carry the credential: {payload}"
    );
}

/// The same repository, described two ways, is one attachment — and the second
/// attempt is a conflict rather than a duplicate.
#[tokio::test]
async fn one_repository_described_two_ways_is_one_attachment() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let token = issue(ADMIN, tenant_id);
    let workspace = seed_workspace(&app, &token, "payments").await;
    let project = seed_project(
        &app,
        &token,
        workspace["id"].as_str().expect("id"),
        "ledger",
    )
    .await;
    let project_id = project["id"].as_str().expect("id").to_owned();

    let (status, _) = call(
        &app,
        "POST",
        &format!("/v1/projects/{project_id}/repositories"),
        Some(&token),
        Some("attach-1"),
        Some(json!({"remote_uri": "git@github.com:acme/payments.git"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, error) = call(
        &app,
        "POST",
        &format!("/v1/projects/{project_id}/repositories"),
        Some(&token),
        Some("attach-2"),
        Some(json!({"remote_uri": "https://github.com/acme/payments"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "the same repository written another way is already attached: {error}"
    );
}

/// A repository with no remote is attached by fingerprint, and needs a name
/// because a fingerprint names nothing a human reads.
#[tokio::test]
async fn a_repository_with_no_remote_is_attached_by_fingerprint() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let token = issue(ADMIN, tenant_id);
    let workspace = seed_workspace(&app, &token, "payments").await;
    let project = seed_project(
        &app,
        &token,
        workspace["id"].as_str().expect("id"),
        "ledger",
    )
    .await;
    let project_id = project["id"].as_str().expect("id").to_owned();
    let oid = "e".repeat(40);

    let (status, error) = call(
        &app,
        "POST",
        &format!("/v1/projects/{project_id}/repositories"),
        Some(&token),
        Some("attach-1"),
        Some(json!({"local_fingerprint": oid})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a name is required: {error}"
    );

    let (status, attached) = call(
        &app,
        "POST",
        &format!("/v1/projects/{project_id}/repositories"),
        Some(&token),
        Some("attach-2"),
        Some(json!({"local_fingerprint": oid, "name": "payments"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{attached}");
    assert_eq!(attached["provider"], "local");
    assert_eq!(attached["canonical_uri"], format!("git+fingerprint:{oid}"));
}

// ── Lifecycle ────────────────────────────────────────────────────────────────

/// Retiring a workspace is a status transition — there is no delete verb, and
/// a retired workspace takes no new projects.
#[tokio::test]
async fn a_retired_workspace_takes_no_new_projects() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let token = issue(ADMIN, tenant_id);
    let workspace = seed_workspace(&app, &token, "payments").await;
    let id = workspace["id"].as_str().expect("id").to_owned();

    let (status, archived) = call(
        &app,
        "PATCH",
        &format!("/v1/workspaces/{id}"),
        Some(&token),
        None,
        Some(json!({"expected_revision": 1, "status": "archived"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{archived}");
    assert_eq!(archived["status"], "archived");

    let (status, error) = call(
        &app,
        "POST",
        &format!("/v1/workspaces/{id}/projects"),
        Some(&token),
        Some("into-the-archive"),
        Some(json!({"slug": "ledger", "display_name": "Ledger"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");

    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/v1/workspaces/{id}"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::METHOD_NOT_ALLOWED,
        "there is no delete verb: retiring is a status transition"
    );
}

/// A description can be set, cleared and left alone — three requests the wire
/// can say apart.
#[tokio::test]
async fn a_description_can_be_cleared_and_left_alone() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let token = issue(ADMIN, tenant_id);
    let (status, workspace) = call(
        &app,
        "POST",
        "/v1/workspaces",
        Some(&token),
        Some("k-1"),
        Some(json!({
            "slug": "payments",
            "display_name": "Payments",
            "description": "What the payments team knows",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workspace}");
    let id = workspace["id"].as_str().expect("id").to_owned();

    let (_, renamed) = call(
        &app,
        "PATCH",
        &format!("/v1/workspaces/{id}"),
        Some(&token),
        None,
        Some(json!({"expected_revision": 1, "display_name": "Payments platform"})),
    )
    .await;
    assert_eq!(
        renamed["description"], "What the payments team knows",
        "an absent description leaves it alone"
    );

    let (_, cleared) = call(
        &app,
        "PATCH",
        &format!("/v1/workspaces/{id}"),
        Some(&token),
        None,
        Some(json!({"expected_revision": 2, "description": null})),
    )
    .await;
    assert!(
        cleared.get("description").is_none(),
        "an explicit null clears it: {cleared}"
    );
}

/// An unknown field is refused rather than ignored — `path` most of all, since
/// it is the thing this feature exists to refuse.
#[tokio::test]
async fn an_unknown_field_is_refused() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let token = issue(ADMIN, tenant_id);
    let (status, error) = call(
        &app,
        "POST",
        "/v1/workspaces",
        Some(&token),
        Some("k-1"),
        Some(json!({"slug": "payments", "display_name": "Payments", "colour": "blue"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
}
