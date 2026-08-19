//! CPR-6 acceptance criteria at the HTTP surface (ADR-0073): decisions that
//! name the thing they are about, and grants that decide.
//!
//! The resolver's contract — the six inputs, the ordering, inheritance,
//! principal isolation, group resolution, revocation, tenancy — is
//! `crates/synveda-store/tests/anchors.rs`, and the pack-level properties are
//! `crates/synveda-policy/tests/anchors.rs`. This suite proves the two things
//! only the HTTP surface has: that a **grant alone** is now enough to use the
//! product end to end, with no role bound on the old hierarchy anywhere; and
//! that `GET /v1/me` answers what the caller may do **where they stand**, from
//! real decisions.
//!
//! Every subject in this file is an ordinary token subject with no
//! `role_bindings` row. That is the point: before CPR-6 none of these calls
//! could have succeeded.
//!
//! Tests that need a live Postgres read `DATABASE_URL` and skip with a message
//! when it is unset (CI has no database); run them locally with `make db-test`.

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
use synveda_types::TenantId;
use tower::ServiceExt;

const SECRET: &[u8] = b"cpr-6-anchors-test-secret";
/// The person who creates the workspace, and is therefore its `owner` — by a
/// grant the creation mints, never by a binding.
const FOUNDER: &str = "cpr6-founder";
/// Somebody granted one project inside the founder's workspace.
const CONTRACTOR: &str = "cpr6-contractor";
/// Somebody with nothing at all.
const STRANGER: &str = "cpr6-stranger";

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn state(url: &str) -> AppState {
    AppState {
        pool: PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(30))
            .connect_lazy(url)
            .expect("parse database url"),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: "http://127.0.0.1:8130".to_owned(),
        pdp: Arc::new(synveda_policy::Pdp::new().expect("build the embedded PDP")),
        scope_chains: Arc::new(synveda_store::ScopeChainCache::new()),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index: Arc::new(
            synveda_retrieval::SearchIndex::open(
                std::env::temp_dir()
                    .join("synveda-cpr6-tests")
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

/// A tenant with **no role binding of any kind**, and exactly one grant: the
/// founder as `owner` at the tenant root.
///
/// That grant is seeded through the store rather than through a route, and it
/// is a gap the product still has rather than a convenience for the test:
/// **nothing mints a tenant's first grant.** A brand-new tenant has a root
/// scope, no grants and no bindings, so nobody can create the first workspace —
/// under every shipped pack `WorkspaceCreate` takes an admin role or an `owner`
/// grant, and CPR-4's own suite bound `org-admin` for the same reason. What
/// changed with CPR-6 is that a **grant** now works where only a binding did,
/// which is what this suite exists to prove; where that first grant comes from
/// is admission's, and it is recorded as standing work rather than solved here.
async fn admitted_tenant() -> Option<(AppState, TenantId)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping anchor API test: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
            return None;
        }
    };
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(30))
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");
    let id = TenantId::new();
    let slug = format!("cpr6-{}", id.as_uuid().simple());
    synveda_store::tenants::create(
        &pool,
        id,
        &slug,
        "CPR-6 API test",
        synveda_types::TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    let mut tx = synveda_store::rls::begin_tenant_tx(&pool, id)
        .await
        .expect("begin tenant tx");
    let root = synveda_store::scopes::ensure_tenant_root(&mut tx, id)
        .await
        .expect("mint the tenant root");
    synveda_store::access::create_grant(
        &mut *tx,
        &synveda_store::access::NewGrant {
            id: synveda_types::GrantId::new(),
            tenant_id: id,
            scope_id: root.id,
            subject: synveda_types::access::GrantSubject::Principal {
                principal_id: FOUNDER.to_owned(),
            },
            role_key: synveda_types::access::RoleKey::Owner,
            source: synveda_types::access::GrantSource::Owner,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .expect("seed the founder's grant");
    tx.commit().await.expect("commit the seed");
    pool.close().await;
    Some((state(&url), id))
}

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

/// The founder's workspace and one project inside it.
async fn seed(app: &Router, token: &str) -> (String, String) {
    let (status, workspace) = call(
        app,
        "POST",
        "/v1/workspaces",
        Some(token),
        Some("anc-ws"),
        Some(json!({"slug": "payments", "display_name": "Payments"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workspace}");
    let workspace_id = workspace["id"].as_str().expect("id").to_owned();
    let (status, project) = call(
        app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/projects"),
        Some(token),
        Some("anc-pr"),
        Some(json!({"slug": "ledger", "display_name": "Ledger"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    (workspace_id, project["id"].as_str().expect("id").to_owned())
}

fn actions(anchor: &Value) -> &serde_json::Map<String, Value> {
    anchor["actions"].as_object().expect("actions")
}

fn anchor_for<'a>(me: &'a Value, scope_id: &str) -> &'a Value {
    me["anchors"]
        .as_array()
        .expect("anchors")
        .iter()
        .find(|anchor| anchor["scope_id"] == scope_id)
        .unwrap_or_else(|| panic!("no anchor for {scope_id}: {me}"))
}

// ── A grant is enough ────────────────────────────────────────────────────────

/// The whole path, on a **grant alone**: nobody in this tenant holds a role
/// binding, and the founder creates a workspace, creates a project inside it,
/// reads both and administers both.
///
/// Before CPR-6 every one of these calls was decided at `Resource::Tenant`
/// against `role_bindings`, so this test could not have passed with an empty
/// `role_bindings` table (ADR-0073 decision 5).
#[tokio::test]
async fn a_grant_alone_carries_the_whole_workspace_path() {
    let Some((state, tenant)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let founder = issue(FOUNDER, tenant);
    let (workspace, project) = seed(&app, &founder).await;

    for (method, path) in [
        ("GET", "/v1/workspaces".to_owned()),
        ("GET", format!("/v1/workspaces/{workspace}")),
        ("GET", format!("/v1/workspaces/{workspace}/projects")),
        ("GET", format!("/v1/projects/{project}")),
        ("GET", format!("/v1/workspaces/{workspace}/members")),
        ("GET", format!("/v1/projects/{project}/members")),
    ] {
        let (status, body) = call(&app, method, &path, Some(&founder), None, None).await;
        assert_eq!(status, StatusCode::OK, "{method} {path}: {body}");
    }

    // And the mutations the `owner` grant carries.
    let (status, body) = call(
        &app,
        "PATCH",
        &format!("/v1/workspaces/{workspace}"),
        Some(&founder),
        None,
        Some(json!({"expected_revision": 1, "display_name": "Payments Platform"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["display_name"], "Payments Platform");
}

/// Somebody holding nothing is refused everywhere, and sees nothing.
#[tokio::test]
async fn holding_nothing_reaches_nothing() {
    let Some((state, tenant)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let founder = issue(FOUNDER, tenant);
    let stranger = issue(STRANGER, tenant);
    let (workspace, project) = seed(&app, &founder).await;

    for (method, path) in [
        ("GET", format!("/v1/workspaces/{workspace}")),
        ("GET", format!("/v1/projects/{project}")),
        ("GET", format!("/v1/workspaces/{workspace}/members")),
    ] {
        let (status, body) = call(&app, method, &path, Some(&stranger), None, None).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{method} {path} must be refused: {body}"
        );
    }
    let (status, body) = call(&app, "GET", "/v1/workspaces", Some(&stranger), None, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

// ── Project-only access ──────────────────────────────────────────────────────

/// A project-only grant reaches the project and **not** the workspace above
/// it — the sentence the placement model could not make true.
#[tokio::test]
async fn a_project_grant_reaches_the_project_and_stops() {
    let Some((state, tenant)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let founder = issue(FOUNDER, tenant);
    let contractor = issue(CONTRACTOR, tenant);
    let (workspace, project) = seed(&app, &founder).await;

    let (status, grant) = call(
        &app,
        "POST",
        &format!("/v1/projects/{project}/members"),
        Some(&founder),
        Some("anc-member"),
        Some(json!({"principal_id": CONTRACTOR, "role": "member"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{grant}");

    // The project: yes.
    let (status, body) = call(
        &app,
        "GET",
        &format!("/v1/projects/{project}"),
        Some(&contractor),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The workspace above it: no, on every verb.
    for (method, path, payload) in [
        ("GET", format!("/v1/workspaces/{workspace}"), None),
        ("GET", format!("/v1/workspaces/{workspace}/members"), None),
        (
            "PATCH",
            format!("/v1/workspaces/{workspace}"),
            Some(json!({"expected_revision": 1, "display_name": "Mine now"})),
        ),
    ] {
        let (status, body) = call(&app, method, &path, Some(&contractor), None, payload).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{method} {path} must not follow a project grant upward: {body}"
        );
    }
}

/// Revoking that grant is refused on the very next request. Nothing runs and
/// nothing is invalidated: the anchors are resolved per request.
#[tokio::test]
async fn revocation_is_in_force_on_the_next_request() {
    let Some((state, tenant)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let founder = issue(FOUNDER, tenant);
    let contractor = issue(CONTRACTOR, tenant);
    let (_workspace, project) = seed(&app, &founder).await;

    let (status, grant) = call(
        &app,
        "POST",
        &format!("/v1/projects/{project}/members"),
        Some(&founder),
        Some("anc-revoke"),
        Some(json!({"principal_id": CONTRACTOR, "role": "member"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{grant}");
    let grant_id = grant["id"].as_str().expect("id").to_owned();

    let (status, _) = call(
        &app,
        "GET",
        &format!("/v1/projects/{project}"),
        Some(&contractor),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = call(
        &app,
        "DELETE",
        &format!("/v1/admin/grants/{grant_id}"),
        Some(&founder),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let (status, body) = call(
        &app,
        "GET",
        &format!("/v1/projects/{project}"),
        Some(&contractor),
        None,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the very next request must be refused: {body}"
    );
}

// ── Group-derived access ─────────────────────────────────────────────────────

/// A grant naming a group reaches its members, and taking somebody out of the
/// group takes their access with it.
#[tokio::test]
async fn a_group_grant_reaches_its_members_and_stops_when_they_leave() {
    let Some((state, tenant)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let founder = issue(FOUNDER, tenant);
    let contractor = issue(CONTRACTOR, tenant);
    let (workspace, project) = seed(&app, &founder).await;

    let (status, group) = call(
        &app,
        "POST",
        "/v1/admin/groups",
        Some(&founder),
        Some("anc-group"),
        Some(json!({
            "slug": "reviewers",
            "display_name": "Reviewers",
            "members": [CONTRACTOR],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{group}");
    let group_id = group["id"].as_str().expect("id").to_owned();

    let (status, workspace_view) = call(
        &app,
        "GET",
        &format!("/v1/workspaces/{workspace}"),
        Some(&founder),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{workspace_view}");
    let workspace_scope = workspace_view["scope_id"]
        .as_str()
        .expect("scope")
        .to_owned();

    let (status, grant) = call(
        &app,
        "POST",
        "/v1/admin/grants",
        Some(&founder),
        Some("anc-group-grant"),
        Some(json!({
            "scope_id": workspace_scope,
            "group_id": group_id,
            "role": "member",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{grant}");

    // The workspace grant reaches the project inside it, through the group.
    let (status, body) = call(
        &app,
        "GET",
        &format!("/v1/projects/{project}"),
        Some(&contractor),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Emptied: the grant still exists and reaches nobody.
    let (status, updated) = call(
        &app,
        "PATCH",
        &format!("/v1/admin/groups/{group_id}"),
        Some(&founder),
        None,
        Some(json!({"expected_revision": 1, "members": []})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");

    let (status, body) = call(
        &app,
        "GET",
        &format!("/v1/projects/{project}"),
        Some(&contractor),
        None,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "leaving the group takes the access: {body}"
    );
}

// ── /v1/me ───────────────────────────────────────────────────────────────────

/// `/v1/me` mints the caller's own scope, and it is the first anchor it
/// serves — before they have created anything at all.
#[tokio::test]
async fn me_mints_and_serves_the_callers_own_scope_first() {
    let Some((state, tenant)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let stranger = issue(STRANGER, tenant);

    let (status, me) = call(&app, "GET", "/v1/me", Some(&stranger), None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["onboarding"]["state"], "needs_workspace");

    let anchors = me["anchors"].as_array().expect("anchors");
    assert!(!anchors.is_empty(), "a caller always stands somewhere");
    assert_eq!(
        anchors[0]["source"], "principal_scope",
        "their own scope sorts first: {me}"
    );
    assert_eq!(anchors[0]["kind"], "principal");
    assert!(
        me["anchors_not_answered"].is_null(),
        "nothing was dropped, so nothing is reported"
    );

    // The second call resolves the same scope rather than minting another.
    let (status, again) = call(&app, "GET", "/v1/me", Some(&stranger), None, None).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(
        again["anchors"][0]["scope_id"], anchors[0]["scope_id"],
        "one scope per subject"
    );
}

/// `/v1/me`'s capabilities are the decisions they forecast: they move with the
/// grant and with nothing else.
///
/// The founder and the contractor differ in exactly one thing — what they were
/// granted — and their capability blocks at the same scope differ exactly
/// where the packs say they should.
#[tokio::test]
async fn me_forecasts_from_real_decisions_and_moves_with_the_grant() {
    let Some((state, tenant)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let founder = issue(FOUNDER, tenant);
    let contractor = issue(CONTRACTOR, tenant);
    let (workspace, project) = seed(&app, &founder).await;

    let (status, workspace_view) = call(
        &app,
        "GET",
        &format!("/v1/workspaces/{workspace}"),
        Some(&founder),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{workspace_view}");
    let workspace_scope = workspace_view["scope_id"]
        .as_str()
        .expect("scope")
        .to_owned();
    let (status, project_view) = call(
        &app,
        "GET",
        &format!("/v1/projects/{project}"),
        Some(&founder),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{project_view}");
    let project_scope = project_view["scope_id"].as_str().expect("scope").to_owned();

    // The founder owns the workspace.
    let (status, me) = call(&app, "GET", "/v1/me", Some(&founder), None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let owned = anchor_for(&me, &workspace_scope);
    assert_eq!(owned["roles"], json!(["owner"]));
    assert_eq!(actions(owned)["workspace.update"], json!(true));
    assert_eq!(actions(owned)["membership.grant"], json!(true));
    // And the project inside it. The founder created it, so CPR-5's rule mints
    // an `owner` grant there too and the anchor is direct — which is the fact
    // worth pinning, because it is *not* the inheritance case.
    let created = anchor_for(&me, &project_scope);
    assert_eq!(created["roles"], json!(["owner"]));
    assert_eq!(created["direct"], json!(true));
    assert_eq!(actions(created)["project.update"], json!(true));

    // The contractor is granted the project only.
    let (status, grant) = call(
        &app,
        "POST",
        &format!("/v1/projects/{project}/members"),
        Some(&founder),
        Some("anc-me-member"),
        Some(json!({"principal_id": CONTRACTOR, "role": "member"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{grant}");

    let (status, theirs) = call(&app, "GET", "/v1/me", Some(&contractor), None, None).await;
    assert_eq!(status, StatusCode::OK, "{theirs}");
    let theirs_project = anchor_for(&theirs, &project_scope);
    assert_eq!(theirs_project["roles"], json!(["member"]));
    assert_eq!(theirs_project["direct"], json!(true));
    assert_eq!(
        actions(theirs_project)["project.read"],
        json!(true),
        "a member reads the project"
    );
    assert_eq!(
        actions(theirs_project)["project.update"],
        json!(false),
        "and does not administer it"
    );
    assert!(
        theirs["anchors"]
            .as_array()
            .expect("anchors")
            .iter()
            .all(|anchor| anchor["scope_id"] != json!(workspace_scope)),
        "the workspace above the granted project is not an anchor: {theirs}"
    );
}

/// Somebody else's own scope never appears in your anchors, and never
/// decides — the privacy floor, at the surface a client actually reads.
#[tokio::test]
async fn nobody_elses_own_scope_is_ever_an_anchor() {
    let Some((state, tenant)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let founder = issue(FOUNDER, tenant);
    let stranger = issue(STRANGER, tenant);

    // Both callers exist, so both have a principal scope.
    let (status, mine) = call(&app, "GET", "/v1/me", Some(&founder), None, None).await;
    assert_eq!(status, StatusCode::OK, "{mine}");
    let (status, theirs) = call(&app, "GET", "/v1/me", Some(&stranger), None, None).await;
    assert_eq!(status, StatusCode::OK, "{theirs}");

    let my_scope = mine["anchors"][0]["scope_id"].clone();
    let their_scope = theirs["anchors"][0]["scope_id"].clone();
    assert_ne!(my_scope, their_scope);

    // The founder creates a workspace, which mints the tenant root's `owner`
    // grant for them — the widest thing this plane hands out.
    seed(&app, &founder).await;
    let (status, mine) = call(&app, "GET", "/v1/me", Some(&founder), None, None).await;
    assert_eq!(status, StatusCode::OK, "{mine}");
    assert!(
        mine["anchors"]
            .as_array()
            .expect("anchors")
            .iter()
            .all(|anchor| anchor["scope_id"] != their_scope),
        "somebody else's own scope is never applicable to my request: {mine}"
    );
}

// ── Tenancy ──────────────────────────────────────────────────────────────────

/// Another tenant's workspace and project are 404, and their grants reach
/// nothing here.
#[tokio::test]
async fn another_tenants_objects_are_not_found() {
    let Some((state, ours)) = admitted_tenant().await else {
        return;
    };
    let Some((_, theirs)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let here = issue(FOUNDER, ours);
    let there = issue(FOUNDER, theirs);
    let (their_workspace, their_project) = seed(&app, &there).await;

    for path in [
        format!("/v1/workspaces/{their_workspace}"),
        format!("/v1/projects/{their_project}"),
        format!("/v1/workspaces/{their_workspace}/members"),
    ] {
        let (status, body) = call(&app, "GET", &path, Some(&here), None, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
    }

    let (status, me) = call(&app, "GET", "/v1/me", Some(&here), None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(
        me["workspaces"].as_array().expect("workspaces").len(),
        0,
        "another tenant's workspace is not in my inventory"
    );
}
