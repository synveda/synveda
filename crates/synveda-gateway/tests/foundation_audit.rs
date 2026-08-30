//! The foundation audit (CPR-9): adversarial coverage of the cutover Prompts
//! 1–7 left standing.
//!
//! Every suite before this one proves that the plane it belongs to works. This
//! one asks the opposite question of all of them at once — **what does a
//! caller learn that they were never granted?** — and it asks it with valid
//! identifiers rather than invented ones, because an id that does not exist
//! anywhere is the easy case and the one every plane already handles.
//!
//! The three adversaries:
//!
//! 1. **Another tenant.** Real workspace, project, scope, group, grant and
//!    invitation ids, minted in a second admitted tenant by a caller who holds
//!    `administrator` there, presented with the first tenant's bearer. Every
//!    one must be a 404 — never a 403, never a 409, never a 500, and never a
//!    body that differs from the one an id nobody ever minted produces. A
//!    status code that distinguishes "yours but forbidden" from "not yours" is
//!    an existence oracle, and so is a *message* that does.
//!
//! 2. **Another workspace, inside one tenant.** The case tenancy does not
//!    cover: two workspaces, and a member of one probing the other. RLS is no
//!    help here — both rows are the same tenant's — so this is entirely the
//!    PDP's and the anchor resolver's to get right.
//!
//! 3. **Somebody else's own scope.** The `principal`-shaped scope ADR-0073
//!    decision 5 makes a base-layer forbid, probed by a tenant administrator —
//!    the one caller who can reach everything else.
//!
//! And the three channels a denial can leak through even when the status code
//! is right: **counts** (a listing's length, an onboarding tally), **errors**
//! (a message naming what the caller may not see) and **navigation
//! capabilities** (`/v1/me`'s anchors and the capability probe, which the
//! console renders its menu from).
//!
//! Tests that need a live Postgres read `DATABASE_URL` and skip with a message
//! when it is unset (CI has no database); run them locally with `make db-test`.

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
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::{GrantId, TenantId};
use tower::ServiceExt;

const SECRET: &[u8] = b"cpr-9-foundation-audit-secret";
/// Holds `administrator` at the tenant root: the bootstrap operator.
const ADMIN: &str = "cpr9-admin";
/// Holds `member` at one workspace and nothing else. The caller the whole
/// second half of this suite is about.
const MEMBER: &str = "cpr9-member";
/// Holds nothing anywhere, and is not provisioned.
const OUTSIDER: &str = "cpr9-outsider";

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
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(30))
            .connect_lazy(url)
            .expect("parse database url"),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp: Arc::new(synveda_policy::Pdp::new().expect("build the embedded PDP")),
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

fn issue(subject: &str, tenant_id: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue(subject, tenant_id, Duration::from_secs(300))
}

/// An admitted tenant whose root carries one `administrator` grant for
/// [`ADMIN`] — the bootstrap door CPR-7 gave the `synveda-admins` convention,
/// seeded by hand here and said so (ADR-0074 decision 4).
async fn admitted_tenant() -> Option<(AppState, TenantId)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping foundation audit test: DATABASE_URL is not set \
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
    synveda_store::epoch::verify(&pool)
        .await
        .expect("apply migrations");
    let id = TenantId::new();
    let slug = format!("cpr9-{}", id.as_uuid().simple());
    tenant_fixture::create(
        &pool,
        id,
        &slug,
        "CPR-9 foundation audit",
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
        &mut tx,
        &synveda_store::access::NewGrant {
            id: GrantId::new(),
            tenant_id: id,
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
    .expect("grant administrator at the root");
    tx.commit().await.expect("commit grant");
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

/// A workspace with one project.
async fn seed(app: &Router, token: &str, slug: &str) -> (String, String) {
    let (status, workspace) = call(
        app,
        "POST",
        "/v1/workspaces",
        Some(token),
        Some(&format!("ws-{slug}")),
        Some(json!({"slug": slug, "display_name": slug})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workspace}");
    let workspace_id = workspace["id"].as_str().expect("id").to_owned();
    let (status, project) = call(
        app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/projects"),
        Some(token),
        Some(&format!("pr-{slug}")),
        Some(json!({"slug": "ledger", "display_name": "Ledger"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    (workspace_id, project["id"].as_str().expect("id").to_owned())
}

/// Grants `role` to `subject` at `scope_id`, through the public plane.
async fn grant(app: &Router, token: &str, scope_id: &str, subject: &str, role: &str, key: &str) {
    let (status, body) = call(
        app,
        "POST",
        "/v1/admin/grants",
        Some(token),
        Some(key),
        Some(json!({
            "scope_id": scope_id,
            "principal_id": subject,
            "role": role,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

/// The scope a workspace owns, read back through the API.
async fn scope_of(app: &Router, token: &str, workspace_id: &str) -> String {
    let (status, body) = call(
        app,
        "GET",
        &format!("/v1/workspaces/{workspace_id}"),
        Some(token),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["scope_id"].as_str().expect("scope_id").to_owned()
}

// ── 1. Another tenant, with valid identifiers ────────────────────────────────

/// Every per-object route on the context-platform plane, probed with a **real**
/// id belonging to a second tenant.
///
/// The assertion is deliberately stronger than "not 200". A foreign id must
/// produce the *same* answer as an id nobody ever minted — same status and the
/// same error kind — because a caller who can tell the two apart can enumerate
/// another tenant's inventory one uuid at a time without ever reading a row.
#[tokio::test]
async fn a_valid_identifier_from_another_tenant_is_indistinguishable_from_a_fictional_one() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let Some((_, other_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let mine = issue(ADMIN, tenant_id);
    let theirs = issue(ADMIN, other_id);

    // The other tenant's real inventory, made by their own administrator.
    let (their_workspace, their_project) = seed(&app, &theirs, "theirs").await;
    let their_scope = scope_of(&app, &theirs, &their_workspace).await;
    let (_, group) = call(
        &app,
        "POST",
        "/v1/admin/groups",
        Some(&theirs),
        Some("grp-theirs"),
        Some(json!({"slug": "eng", "display_name": "Eng"})),
    )
    .await;
    let their_group = group["id"].as_str().expect("id").to_owned();
    let (status, grants) = call(&app, "GET", "/v1/admin/grants", Some(&theirs), None, None).await;
    assert_eq!(status, StatusCode::OK, "{grants}");
    let their_grant = grants["grants"][0]["id"]
        .as_str()
        .expect("the other tenant holds at least one grant")
        .to_owned();
    let (status, invite) = call(
        &app,
        "POST",
        &format!("/v1/workspaces/{their_workspace}/invites"),
        Some(&theirs),
        Some("inv-theirs"),
        Some(json!({"role": "member"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{invite}");
    let their_invite = invite["invite"]["id"].as_str().expect("id").to_owned();
    let their_token = invite["token"].as_str().expect("token").to_owned();

    // A uuid nobody ever minted, and a **well-formed** token nobody ever
    // issued: the control. The token keeps the real one's shape and swaps only
    // its secret — a malformed string would be refused by grammar before the
    // lookup, which would compare a 400 against a 404 and prove nothing.
    let fiction = uuid::Uuid::now_v7().to_string();
    let fictional_token = {
        let (head, secret) = their_token
            .rsplit_once('.')
            .expect("an invitation token carries its secret in the last segment");
        format!("{head}.{}", "A".repeat(secret.len()))
    };

    let probes: Vec<(&str, String, String, Option<Value>)> = vec![
        (
            "GET",
            format!("/v1/workspaces/{their_workspace}"),
            format!("/v1/workspaces/{fiction}"),
            None,
        ),
        (
            "PATCH",
            format!("/v1/workspaces/{their_workspace}"),
            format!("/v1/workspaces/{fiction}"),
            Some(json!({"expected_revision": 1, "display_name": "Taken"})),
        ),
        (
            "GET",
            format!("/v1/workspaces/{their_workspace}/projects"),
            format!("/v1/workspaces/{fiction}/projects"),
            None,
        ),
        (
            "POST",
            format!("/v1/workspaces/{their_workspace}/projects"),
            format!("/v1/workspaces/{fiction}/projects"),
            Some(json!({"slug": "taken", "display_name": "Taken"})),
        ),
        (
            "GET",
            format!("/v1/projects/{their_project}"),
            format!("/v1/projects/{fiction}"),
            None,
        ),
        (
            "PATCH",
            format!("/v1/projects/{their_project}"),
            format!("/v1/projects/{fiction}"),
            Some(json!({"expected_revision": 1, "display_name": "Taken"})),
        ),
        (
            "GET",
            format!("/v1/projects/{their_project}/repositories"),
            format!("/v1/projects/{fiction}/repositories"),
            None,
        ),
        (
            "POST",
            format!("/v1/projects/{their_project}/repositories"),
            format!("/v1/projects/{fiction}/repositories"),
            Some(json!({"remote_uri": "https://github.com/acme/taken"})),
        ),
        (
            "GET",
            format!("/v1/workspaces/{their_workspace}/members"),
            format!("/v1/workspaces/{fiction}/members"),
            None,
        ),
        (
            "GET",
            format!("/v1/projects/{their_project}/members"),
            format!("/v1/projects/{fiction}/members"),
            None,
        ),
        (
            "POST",
            format!("/v1/projects/{their_project}/members"),
            format!("/v1/projects/{fiction}/members"),
            Some(json!({"principal_id": "x", "role": "member"})),
        ),
        (
            "GET",
            format!("/v1/workspaces/{their_workspace}/invites"),
            format!("/v1/workspaces/{fiction}/invites"),
            None,
        ),
        (
            "DELETE",
            format!("/v1/workspaces/{their_workspace}/invites/{their_invite}"),
            format!("/v1/workspaces/{fiction}/invites/{fiction}"),
            None,
        ),
        (
            "DELETE",
            format!("/v1/admin/grants/{their_grant}"),
            format!("/v1/admin/grants/{fiction}"),
            None,
        ),
        (
            "PATCH",
            format!("/v1/admin/groups/{their_group}"),
            format!("/v1/admin/groups/{fiction}"),
            Some(json!({"expected_revision": 1, "display_name": "Taken"})),
        ),
        (
            "GET",
            format!("/v1/admin/scopes/{their_scope}"),
            format!("/v1/admin/scopes/{fiction}"),
            None,
        ),
        (
            "PATCH",
            format!("/v1/admin/scopes/{their_scope}"),
            format!("/v1/admin/scopes/{fiction}"),
            Some(json!({"display_name": "Taken"})),
        ),
        (
            "GET",
            format!("/v1/admin/scopes/{their_scope}/ancestors"),
            format!("/v1/admin/scopes/{fiction}/ancestors"),
            None,
        ),
        (
            "GET",
            format!("/v1/admin/scopes/{their_scope}/descendants"),
            format!("/v1/admin/scopes/{fiction}/descendants"),
            None,
        ),
        (
            "GET",
            format!("/v1/capabilities?scopes={their_scope}"),
            format!("/v1/capabilities?scopes={fiction}"),
            None,
        ),
        (
            "POST",
            format!("/v1/invites/{their_token}/accept"),
            format!("/v1/invites/{fictional_token}/accept"),
            None,
        ),
    ];

    for (method, real, fictional, body) in probes {
        let (real_status, real_body) = call(
            &app,
            method,
            &real,
            Some(&mine),
            Some("probe-real"),
            body.clone(),
        )
        .await;
        let (fake_status, fake_body) = call(
            &app,
            method,
            &fictional,
            Some(&mine),
            Some("probe-fake"),
            body,
        )
        .await;
        assert_eq!(
            real_status,
            StatusCode::NOT_FOUND,
            "{method} {real} used another tenant's real id and answered {real_status}: \
             {real_body}"
        );
        assert_eq!(
            real_status, fake_status,
            "{method} {real} answers {real_status} for a real foreign id and {fake_status} \
             for a fictional one — the difference is an existence oracle"
        );
        assert_eq!(
            real_body["kind"], fake_body["kind"],
            "{method} {real} distinguishes a foreign id from a fictional one by error kind:\n  \
             foreign:   {real_body}\n  fictional: {fake_body}"
        );
    }

    // And nothing the other tenant made is counted anywhere in this one.
    for (path, key) in [
        ("/v1/workspaces", "workspaces"),
        ("/v1/admin/groups", "groups"),
        ("/v1/admin/grants", "grants"),
        ("/v1/admin/scopes", "scopes"),
    ] {
        let (status, body) = call(&app, "GET", path, Some(&mine), None, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let rows = body[key].as_array().expect("a listing envelope");
        let text = serde_json::to_string(rows).expect("serialise");
        for foreign in [&their_workspace, &their_project, &their_scope, &their_group] {
            assert!(
                !text.contains(foreign.as_str()),
                "GET {path} leaked the other tenant's {foreign}: {text}"
            );
        }
    }

    // `/v1/me` counts what this caller may read, and the other tenant is not
    // in it — including the tallies the console renders onboarding from.
    let (status, me) = call(&app, "GET", "/v1/me", Some(&mine), None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(
        me["onboarding"]["workspace_count"], 0,
        "a tenant that made nothing counts nothing: {me}"
    );
    assert_eq!(me["onboarding"]["project_count"], 0, "{me}");
    let text = serde_json::to_string(&me).expect("serialise");
    for foreign in [&their_workspace, &their_project, &their_group] {
        assert!(
            !text.contains(foreign.as_str()),
            "/v1/me leaked the other tenant's {foreign}"
        );
    }
}

/// An invitation is a bearer-shaped secret, so the cross-tenant case gets its
/// own test rather than only a row in the table above: redeeming one must not
/// merely fail, it must leave the invitation **spendable by its real
/// recipient**. A refusal that consumed the token would be a denial-of-service
/// anybody holding a leaked link could inflict on the person it was for.
#[tokio::test]
async fn an_invitation_cannot_be_redeemed_from_another_tenant_and_survives_the_attempt() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let Some((_, other_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let mine = issue(ADMIN, tenant_id);
    let theirs = issue(ADMIN, other_id);

    let (their_workspace, _) = seed(&app, &theirs, "theirs").await;
    let (status, invite) = call(
        &app,
        "POST",
        &format!("/v1/workspaces/{their_workspace}/invites"),
        Some(&theirs),
        Some("inv-1"),
        Some(json!({"role": "member"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{invite}");
    let token = invite["token"].as_str().expect("token").to_owned();

    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/invites/{token}/accept"),
        Some(&mine),
        None,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a foreign tenant redeemed an invitation: {body}"
    );

    // The rightful recipient still gets it.
    let recipient = issue(MEMBER, other_id);
    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/invites/{token}/accept"),
        Some(&recipient),
        None,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the cross-tenant attempt spent somebody else's invitation: {body}"
    );
}

// ── 2. Another workspace, inside one tenant ──────────────────────────────────

/// The case tenancy cannot help with: two workspaces in **one** tenant, and a
/// member of the first probing the second.
///
/// Both rows pass RLS — they are the same tenant's — so every refusal here is
/// the PDP's and the anchor resolver's. The member holds `member` at workspace
/// A's scope and nothing at B's, so B must be invisible in the listings, absent
/// from the navigation capabilities, and a 403 on the object routes rather than
/// a 200.
#[tokio::test]
async fn a_member_of_one_workspace_learns_nothing_about_another() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let admin = issue(ADMIN, tenant_id);
    let member = issue(MEMBER, tenant_id);

    let (mine, my_project) = seed(&app, &admin, "mine").await;
    let (theirs, their_project) = seed(&app, &admin, "theirs").await;
    let my_scope = scope_of(&app, &admin, &mine).await;
    let their_scope = scope_of(&app, &admin, &theirs).await;
    grant(&app, &admin, &my_scope, MEMBER, "member", "grant-member").await;

    // What the member may read: their own workspace and its project.
    let (status, me) = call(&app, "GET", "/v1/me", Some(&member), None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let visible: Vec<&str> = me["workspaces"]
        .as_array()
        .expect("workspaces")
        .iter()
        .map(|w| w["id"].as_str().expect("id"))
        .collect();
    assert!(
        visible.contains(&mine.as_str()),
        "a member cannot see the workspace they are a member of: {me}"
    );
    assert!(
        !visible.contains(&theirs.as_str()),
        "/v1/me showed a member a workspace they hold nothing at: {me}"
    );
    assert_eq!(
        me["onboarding"]["workspace_count"], 1,
        "the onboarding tally counted a workspace this caller may not read: {me}"
    );
    // The symptom a person actually met. `needs_workspace` is what the console
    // routes somebody into the first-run wizard on, so an invited member was
    // asked to create the workspace they had just been added to.
    assert_ne!(
        me["onboarding"]["state"],
        json!("needs_workspace"),
        "a member of a workspace was told to create their first one: {me}"
    );

    let projects: Vec<&str> = me["projects"]
        .as_array()
        .expect("projects")
        .iter()
        .map(|p| p["id"].as_str().expect("id"))
        .collect();
    assert!(projects.contains(&my_project.as_str()), "{me}");
    assert!(
        !projects.contains(&their_project.as_str()),
        "/v1/me leaked another workspace's project: {me}"
    );

    // The navigation capabilities name only scopes this caller stands at.
    let anchors = serde_json::to_string(&me["anchors"]).expect("serialise");
    assert!(
        !anchors.contains(their_scope.as_str()),
        "/v1/me's anchors named a scope the caller holds nothing at: {anchors}"
    );

    // The listing agrees with `/v1/me`, and the object routes refuse.
    let (status, list) = call(&app, "GET", "/v1/workspaces", Some(&member), None, None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let listed: Vec<&str> = list["workspaces"]
        .as_array()
        .expect("workspaces")
        .iter()
        .map(|w| w["id"].as_str().expect("id"))
        .collect();
    assert_eq!(
        listed,
        vec![mine.as_str()],
        "GET /v1/workspaces disagrees with /v1/me about what this caller may read"
    );

    for (method, path) in [
        ("GET", format!("/v1/workspaces/{theirs}")),
        ("GET", format!("/v1/workspaces/{theirs}/projects")),
        ("GET", format!("/v1/workspaces/{theirs}/members")),
        ("GET", format!("/v1/projects/{their_project}")),
        ("GET", format!("/v1/projects/{their_project}/members")),
    ] {
        let (status, body) = call(&app, method, &path, Some(&member), None, None).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{method} {path} answered {status} to a caller who holds nothing there: {body}"
        );
    }
}

/// The capability probe is what the console builds its menu from, so a scope a
/// caller may not read must not become readable *through the probe* — and the
/// probe must not become a metadata oracle for anybody holding a scope id
/// (ADR-0058 decision 3).
#[tokio::test]
async fn the_capability_probe_offers_no_plane_it_cannot_name() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let admin = issue(ADMIN, tenant_id);
    let member = issue(MEMBER, tenant_id);

    let (mine, _) = seed(&app, &admin, "mine").await;
    let (theirs, _) = seed(&app, &admin, "theirs").await;
    let my_scope = scope_of(&app, &admin, &mine).await;
    let their_scope = scope_of(&app, &admin, &theirs).await;
    grant(&app, &admin, &my_scope, MEMBER, "member", "grant-member").await;

    // A member probing the workspace they hold: answered, with the node detail
    // their `ScopeRead` verdict admits.
    let (status, body) = call(
        &app,
        "GET",
        &format!("/v1/capabilities?scopes={my_scope}"),
        Some(&member),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let answer = &body["capabilities"][0];
    assert_eq!(answer["scope_id"], json!(my_scope), "{body}");

    // The same probe against a scope they hold nothing at. It is the same
    // tenant, so ownership does not refuse it — what must be true is that
    // every verdict is false and no node detail is served, because a probe
    // that answered `true` here would put a plane in the menu that the act
    // behind it refuses.
    let (status, body) = call(
        &app,
        "GET",
        &format!("/v1/capabilities?scopes={their_scope}"),
        Some(&member),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let answer = &body["capabilities"][0];
    let actions = answer["actions"].as_object().expect("actions");
    let permitted: Vec<&String> = actions
        .iter()
        .filter(|(_, allowed)| allowed.as_bool() == Some(true))
        .map(|(name, _)| name)
        .collect();
    assert!(
        permitted.is_empty(),
        "the probe offered {permitted:?} at a scope this caller holds nothing at: {body}"
    );
    assert!(
        answer.get("scope_path").is_none(),
        "the probe served the path of a node this caller may not read: {body}"
    );
    assert!(
        answer.get("pack").is_none(),
        "the probe served the governance of a node this caller may not read: {body}"
    );
    assert_eq!(
        answer["roles"].as_array().map(Vec::len),
        Some(0),
        "the probe named roles at a scope this caller holds nothing at: {body}"
    );
}

// ── 3. Somebody else's own scope ─────────────────────────────────────────────

/// A `principal`-shaped scope is somebody's own, and ADR-0073 decision 5 makes
/// that a base-layer forbid rather than a pack rule — so the caller who can
/// reach everything else in the tenant must still not reach it.
///
/// Probed with the **tenant administrator**, deliberately: a test that used an
/// outsider would pass under any rule that merely required a role.
#[tokio::test]
async fn a_tenant_administrator_cannot_reach_somebody_elses_own_scope() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let admin = issue(ADMIN, tenant_id);
    let member = issue(MEMBER, tenant_id);

    // `/v1/me` is what mints a caller's own scope (ADR-0073 decision 2).
    let (status, me) = call(&app, "GET", "/v1/me", Some(&member), None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let their_own = me["anchors"]
        .as_array()
        .expect("anchors")
        .iter()
        .find(|anchor| anchor["kind"] == json!("principal"))
        .and_then(|anchor| anchor["scope_id"].as_str())
        .expect("the caller's own scope is an anchor of theirs")
        .to_owned();

    // The administrator can enumerate the tree — the scope exists and they can
    // see that it does. What they must not get is a decision that permits
    // anything *at* it.
    let (status, body) = call(
        &app,
        "GET",
        &format!("/v1/capabilities?scopes={their_own}"),
        Some(&admin),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let actions = body["capabilities"][0]["actions"]
        .as_object()
        .expect("actions");
    let permitted: Vec<&String> = actions
        .iter()
        .filter(|(_, allowed)| allowed.as_bool() == Some(true))
        .map(|(name, _)| name)
        .collect();
    assert!(
        permitted.is_empty(),
        "a tenant administrator was offered {permitted:?} at somebody else's own scope — \
         the privacy forbid is a base-layer rule no role lifts (ADR-0073 decision 5): {body}"
    );
}

// ── 4. The unprovisioned caller ──────────────────────────────────────────────

/// A subject with no identity, no grant and no scope. Every plane must answer
/// them, and every answer must be empty rather than an error — the console
/// renders `/v1/me` before it knows anything, and a 500 here is a product that
/// cannot show its own front door.
#[tokio::test]
async fn a_caller_who_holds_nothing_is_answered_with_nothing_rather_than_an_error() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let admin = issue(ADMIN, tenant_id);
    let outsider = issue(OUTSIDER, tenant_id);

    let (workspace, project) = seed(&app, &admin, "private").await;

    let (status, me) = call(&app, "GET", "/v1/me", Some(&outsider), None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["workspaces"].as_array().map(Vec::len), Some(0), "{me}");
    assert_eq!(me["projects"].as_array().map(Vec::len), Some(0), "{me}");
    assert_eq!(
        me["onboarding"]["workspace_count"], 0,
        "the onboarding tally counted workspaces this caller cannot read: {me}"
    );
    let text = serde_json::to_string(&me).expect("serialise");
    assert!(!text.contains(&workspace), "/v1/me leaked a workspace id");
    assert!(!text.contains(&project), "/v1/me leaked a project id");

    // And every listing agrees.
    for (path, key) in [
        ("/v1/workspaces", "workspaces"),
        ("/v1/admin/grants", "grants"),
    ] {
        let (status, body) = call(&app, "GET", path, Some(&outsider), None, None).await;
        assert!(
            status == StatusCode::OK && body[key].as_array().is_some_and(Vec::is_empty)
                || status == StatusCode::FORBIDDEN,
            "GET {path} answered {status} with {body} — an outsider gets an empty listing \
             or a refusal, never somebody else's rows"
        );
    }
}
