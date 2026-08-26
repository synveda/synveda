//! CPR-5 acceptance criteria at the HTTP surface (ADR-0072): members, groups,
//! grants and invitations.
//!
//! The store-level contract — inheritance, principal isolation, group
//! resolution, one-time invitations, the structural rules against direct SQL —
//! is `crates/synveda-store/tests/access.rs`. This suite proves the things only
//! the HTTP surface has: status codes, the PDP on every route, the audit
//! events, the idempotency seam, and the two properties this plane is most
//! likely to get wrong — **an invitation token that appears exactly once and
//! never in the chain**, and a members listing that says *why* somebody is on
//! it.
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
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::{GrantId, GroupId, IdentityId, IdentityKind, TenantId};
use tower::ServiceExt;

const SECRET: &[u8] = b"cpr-5-access-test-secret";
/// Bound tenant-wide `org-admin`: what every shipped pack prices the access
/// plane's mutating actions at.
const ADMIN: &str = "cpr5-admin";
/// A placed subject with no role binding: the caller every "denies without the
/// action" assertion uses.
const OUTSIDER: &str = "cpr5-outsider";
/// The person invitations in this suite are for.
const INVITEE: &str = "cpr5-invitee";

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
            // Two, not four. `cargo test` runs a suite's tests on their own
            // threads and each `#[tokio::test]` builds its own runtime, so a
            // pool cannot be shared across them — which makes the per-test
            // footprint the only lever, and the handlers here open one
            // transaction at a time.
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

async fn admitted_tenant() -> Option<(AppState, TenantId)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping access API test: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
            return None;
        }
    };
    // One connection, and it is closed before the test body runs: this pool
    // exists to admit a tenant and bind a role, and keeping it open for the
    // rest of the test is what pushed a `--workspace` run past Postgres's
    // `max_connections`.
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
    let slug = format!("cpr5-{}", id.as_uuid().simple());
    synveda_store::tenants::create(
        &pool,
        id,
        &slug,
        "CPR-5 API test",
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

async fn provision_identity(state: &AppState, tenant_id: TenantId, subject: &str) -> String {
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin identity fixture");
    let scope = synveda_store::scopes::ensure_principal_scope(&mut tx, tenant_id, subject, subject)
        .await
        .expect("create principal scope");
    let identity = synveda_store::identities::create(
        &mut tx,
        IdentityId::new(),
        tenant_id,
        Some(subject),
        IdentityKind::User,
        None,
        Some(subject),
        scope.id,
    )
    .await
    .expect("create identity");
    tx.commit().await.expect("commit identity fixture");
    identity.id.to_string()
}

async fn seed_directory_group(state: &AppState, tenant_id: TenantId) -> GroupId {
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin directory group fixture");
    let group = synveda_store::access::sync_directory_group(
        &mut tx,
        GroupId::new(),
        tenant_id,
        "entra",
        "entra-group-engineering",
        Some("protocol-external-engineering"),
        "entra-engineering",
        "Engineering",
        &[],
    )
    .await
    .expect("create directory group fixture");
    tx.commit().await.expect("commit directory group fixture");
    group.id
}

/// A workspace with one project, through the API.
async fn seed(app: &Router, token: &str) -> (String, String) {
    let (status, workspace) = call(
        app,
        "POST",
        "/v1/workspaces",
        Some(token),
        Some("seed-ws"),
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
        Some("seed-pr"),
        Some(json!({"slug": "ledger", "display_name": "Ledger"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    (workspace_id, project["id"].as_str().expect("id").to_owned())
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

/// Every audit payload in the tenant's chain, concatenated — for sweeping.
async fn chain_text(state: &AppState, tenant_id: TenantId) -> String {
    sqlx::query_scalar::<_, Value>(
        "select payload from audit_log where tenant_id = $1 order by seq",
    )
    .bind(tenant_id.as_uuid())
    .fetch_all(&state.pool)
    .await
    .expect("read the chain")
    .iter()
    .map(std::string::ToString::to_string)
    .collect::<Vec<_>>()
    .join("\n")
}

// ── Ownership ────────────────────────────────────────────────────────────────

/// Creating a workspace makes its creator its `owner`, in the same transaction:
/// a collaboration space nobody is a member of is not one.
#[tokio::test]
async fn creating_a_workspace_makes_its_creator_the_owner() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, project_id) = seed(&app, &token).await;

    let (status, members) = call(
        &app,
        "GET",
        &format!("/v1/workspaces/{workspace_id}/members"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{members}");
    let list = members["members"].as_array().expect("members");
    // Exactly one grant is *written here*, and it is the creator's. The
    // harness's bootstrap `administrator` at the tenant root shows up too
    // and must show up marked `inherited` — a member list that hid it
    // would be hiding real authority over the workspace.
    let here: Vec<_> = list.iter().filter(|m| m["inherited"] == false).collect();
    assert_eq!(
        here.len(),
        1,
        "one grant written at the workspace: {members}"
    );
    assert_eq!(here[0]["principal_id"], ADMIN);
    assert_eq!(here[0]["role"], "owner");
    assert_eq!(
        here[0]["source"], "owner",
        "the one source no route hands out"
    );
    assert_eq!(list[0]["inherited"], false, "nearest first");
    assert!(
        list.iter().any(|m| m["inherited"] == true
            && m["role"] == "administrator"
            && m["source"] == "automation"),
        "the tenant root's bootstrap grant reaches the workspace: {members}"
    );

    // The project has its own owner grant *and* inherits both the
    // workspace's owner and the root's administrator — with no row written
    // at the project for either.
    let (status, members) = call(
        &app,
        "GET",
        &format!("/v1/projects/{project_id}/members"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{members}");
    let list = members["members"].as_array().expect("members");
    assert_eq!(
        list.len(),
        3,
        "own owner grant plus the two inherited ones: {members}"
    );
    assert_eq!(list[0]["inherited"], false, "nearest first");
    assert!(
        list[1..].iter().all(|m| m["inherited"] == true),
        "everything above the project is inherited: {members}"
    );

    let actions = chain_actions(&state, tenant_id).await;
    assert_eq!(
        actions.iter().filter(|a| *a == "access.granted").count(),
        2,
        "one owner grant per created scope: {actions:?}"
    );
}

// ── Invitations ──────────────────────────────────────────────────────────────

/// The whole invitation path: issue, see it outstanding, redeem it with the
/// recipient's own credential, and find them in the member list with the source
/// that says where the access came from.
#[tokio::test]
async fn an_invitation_is_issued_redeemed_and_visible_as_its_own_source() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, project_id) = seed(&app, &token).await;

    let (status, created) = call(
        &app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/invites"),
        Some(&token),
        Some("inv-1"),
        Some(json!({"role": "member", "email": "sam@example.com"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let invite_token = created["token"].as_str().expect("token").to_owned();
    assert!(
        invite_token.starts_with("synveda_invite_v1."),
        "the token is greppable: {invite_token}"
    );
    assert!(
        created["accept_url"]
            .as_str()
            .expect("accept_url")
            .ends_with(&format!("/v1/invites/{invite_token}/accept")),
        "a copyable URL is enough for a local deployment: {created}"
    );
    assert_eq!(created["invite"]["status"], "pending");

    let (status, listed) = call(
        &app,
        "GET",
        &format!("/v1/workspaces/{workspace_id}/invites"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["invites"].as_array().expect("invites").len(), 1);
    assert!(
        !listed.to_string().contains(&invite_token),
        "the listing must never carry the token: {listed}"
    );

    // Redeemed with the recipient's *own* credential.
    let invitee = issue(INVITEE, tenant_id);
    let (status, accepted) = call(
        &app,
        "POST",
        &format!("/v1/invites/{invite_token}/accept"),
        Some(&invitee),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{accepted}");
    assert_eq!(accepted["grant"]["role"], "member");
    assert_eq!(accepted["grant"]["source"], "invite");
    assert!(
        accepted["grant"]["invite_id"].is_string(),
        "the grant names the invitation it came from: {accepted}"
    );

    // And it is in force at the project inside the workspace.
    let (_, members) = call(
        &app,
        "GET",
        &format!("/v1/projects/{project_id}/members"),
        Some(&token),
        None,
        None,
    )
    .await;
    let invited = members["members"]
        .as_array()
        .expect("members")
        .iter()
        .find(|entry| entry["principal_id"] == INVITEE)
        .expect("the invitee holds access at the project");
    assert_eq!(invited["source"], "invite");
    assert_eq!(invited["inherited"], true);

    let actions = chain_actions(&state, tenant_id).await;
    for expected in ["access.invite.created", "access.invite.accepted"] {
        assert!(
            actions.iter().any(|action| action == expected),
            "the chain must record {expected}: {actions:?}"
        );
    }
}

/// **The token appears once and never lands in the chain.** This is the
/// property the whole invitation design rests on, so it is swept for rather
/// than argued.
#[tokio::test]
async fn the_invitation_token_never_reaches_the_audit_chain() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, _) = seed(&app, &token).await;

    let (_, created) = call(
        &app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/invites"),
        Some(&token),
        Some("inv-sweep"),
        Some(json!({"role": "viewer"})),
    )
    .await;
    let invite_token = created["token"].as_str().expect("token").to_owned();
    let secret = invite_token
        .rsplit('.')
        .next()
        .expect("the secret half")
        .to_owned();

    let invitee = issue(INVITEE, tenant_id);
    let (status, _) = call(
        &app,
        "POST",
        &format!("/v1/invites/{invite_token}/accept"),
        Some(&invitee),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let chain = chain_text(&state, tenant_id).await;
    assert!(
        !chain.contains(&secret),
        "the invitation secret reached the audit chain"
    );
    assert!(
        !chain.contains(&invite_token),
        "the invitation token reached the audit chain"
    );

    // And the database stores a hash rather than the token.
    let stored: Vec<u8> =
        sqlx::query_scalar("select token_hash from pending_invites where tenant_id = $1")
            .bind(tenant_id.as_uuid())
            .fetch_one(&state.pool)
            .await
            .expect("read the hash");
    assert_eq!(stored.len(), 32);
    assert_ne!(
        stored,
        invite_token.as_bytes(),
        "the token itself must never be stored"
    );
}

/// One-time: the same link redeemed by a second person is refused, and by the
/// same person is a replay with `200` rather than a punishment for a retry.
#[tokio::test]
async fn an_invitation_link_works_once_and_a_retry_is_not_a_second_redemption() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, _) = seed(&app, &token).await;

    let (_, created) = call(
        &app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/invites"),
        Some(&token),
        Some("inv-once"),
        Some(json!({"role": "member"})),
    )
    .await;
    let link = created["token"].as_str().expect("token").to_owned();

    let invitee = issue(INVITEE, tenant_id);
    let (first, accepted) = call(
        &app,
        "POST",
        &format!("/v1/invites/{link}/accept"),
        Some(&invitee),
        None,
        None,
    )
    .await;
    assert_eq!(first, StatusCode::CREATED, "{accepted}");

    let (again, replay) = call(
        &app,
        "POST",
        &format!("/v1/invites/{link}/accept"),
        Some(&invitee),
        None,
        None,
    )
    .await;
    assert_eq!(again, StatusCode::OK, "the same person replays: {replay}");
    assert_eq!(replay["grant"]["id"], accepted["grant"]["id"]);

    let thief = issue("cpr5-thief", tenant_id);
    let (stolen, body) = call(
        &app,
        "POST",
        &format!("/v1/invites/{link}/accept"),
        Some(&thief),
        None,
        None,
    )
    .await;
    assert_eq!(stolen, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["kind"], "conflict");
}

/// A withdrawn invitation is not redeemable, and the withdrawal is on the
/// chain.
#[tokio::test]
async fn a_withdrawn_invitation_cannot_be_redeemed() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, _) = seed(&app, &token).await;

    let (_, created) = call(
        &app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/invites"),
        Some(&token),
        Some("inv-withdraw"),
        Some(json!({"role": "member"})),
    )
    .await;
    let link = created["token"].as_str().expect("token").to_owned();
    let invite_id = created["invite"]["id"].as_str().expect("id").to_owned();

    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/v1/workspaces/{workspace_id}/invites/{invite_id}"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let invitee = issue(INVITEE, tenant_id);
    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/invites/{link}/accept"),
        Some(&invitee),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let (_, listed) = call(
        &app,
        "GET",
        &format!("/v1/workspaces/{workspace_id}/invites"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(listed["invites"][0]["status"], "revoked");
    assert!(
        chain_actions(&state, tenant_id)
            .await
            .iter()
            .any(|action| action == "access.invite.revoked")
    );
}

/// An invitation may not stand forever, and the refusal says the ceiling in
/// the unit a person thinks in.
#[tokio::test]
async fn an_invitation_cannot_outlive_the_product_ceiling() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, _) = seed(&app, &token).await;

    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/invites"),
        Some(&token),
        Some("inv-forever"),
        Some(json!({"role": "member", "expires_in_secs": 60 * 60 * 24 * 365})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("days"),
        "{body}"
    );
}

/// A string that is not one of this product's tokens is refused by shape,
/// before any lookup — and the refusal never echoes what was presented.
#[tokio::test]
async fn a_string_that_is_not_an_invitation_is_refused_by_shape() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let invitee = issue(INVITEE, tenant_id);
    let (status, body) = call(
        &app,
        "POST",
        "/v1/invites/hunter2/accept",
        Some(&invitee),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        !body.to_string().contains("hunter2"),
        "the refusal quoted what was presented: {body}"
    );
}

/// Anybody with a valid credential may redeem an invitation — that is what the
/// token being the authority means — but the invariant floor still runs, so a
/// caller with no roles at all is admitted and one whose token is absent is
/// not.
#[tokio::test]
async fn redeeming_needs_the_token_rather_than_a_role() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, _) = seed(&app, &token).await;
    let (_, created) = call(
        &app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/invites"),
        Some(&token),
        Some("inv-roleless"),
        Some(json!({"role": "viewer"})),
    )
    .await;
    let link = created["token"].as_str().expect("token").to_owned();

    // No credential at all: refused before anything else.
    let (status, _) = call(
        &app,
        "POST",
        &format!("/v1/invites/{link}/accept"),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A subject with no binding of any kind: admitted, because the token is
    // the authority.
    let roleless = issue(OUTSIDER, tenant_id);
    let (status, accepted) = call(
        &app,
        "POST",
        &format!("/v1/invites/{link}/accept"),
        Some(&roleless),
        None,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "a person this product invited must not be turned away for want of a role: {accepted}"
    );
}

// ── Groups and grants ────────────────────────────────────────────────────────

/// A group is created, granted at a workspace, and its members hold that grant
/// at the project inside it — with the entry saying which group it came
/// through.
#[tokio::test]
async fn a_group_grant_reaches_its_members_and_says_so() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, project_id) = seed(&app, &token).await;
    let robin = provision_identity(&state, tenant_id, "cpr5-robin").await;
    let kim = provision_identity(&state, tenant_id, "cpr5-kim").await;
    let sam = provision_identity(&state, tenant_id, "cpr5-sam").await;

    let (status, group) = call(
        &app,
        "POST",
        "/v1/admin/groups",
        Some(&token),
        Some("grp-1"),
        Some(json!({
            "slug": "engineering",
            "display_name": "Engineering",
            "members": [robin, kim],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{group}");
    let group_id = group["id"].as_str().expect("id").to_owned();
    assert_eq!(group["members"].as_array().expect("members").len(), 2);
    assert_eq!(group["source"], "direct");
    assert_eq!(group["revision"], 1);

    // Grant the group at the workspace's scope.
    let (_, workspace) = call(
        &app,
        "GET",
        &format!("/v1/workspaces/{workspace_id}"),
        Some(&token),
        None,
        None,
    )
    .await;
    let workspace_scope = workspace["scope_id"].as_str().expect("scope").to_owned();
    let (status, grant) = call(
        &app,
        "POST",
        "/v1/admin/grants",
        Some(&token),
        Some("grant-1"),
        Some(json!({"scope_id": workspace_scope, "group_id": group_id, "role": "member"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{grant}");
    assert_eq!(grant["subject_kind"], "group");

    let (_, members) = call(
        &app,
        "GET",
        &format!("/v1/projects/{project_id}/members"),
        Some(&token),
        None,
        None,
    )
    .await;
    let through_group: Vec<&Value> = members["members"]
        .as_array()
        .expect("members")
        .iter()
        .filter(|entry| entry["via_group"].is_object())
        .collect();
    assert_eq!(through_group.len(), 2, "{members}");
    for entry in through_group {
        assert_eq!(entry["via_group"]["slug"], "engineering");
        assert_eq!(entry["inherited"], true);
    }

    // Somebody joins the group; nothing is granted again.
    let (status, updated) = call(
        &app,
        "PATCH",
        &format!("/v1/admin/groups/{group_id}"),
        Some(&token),
        None,
        Some(json!({
            "expected_revision": 1,
            "members": [robin, kim, sam],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["revision"], 2);
    let (_, members) = call(
        &app,
        "GET",
        &format!("/v1/projects/{project_id}/members"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(
        members["members"]
            .as_array()
            .expect("members")
            .iter()
            .filter(|entry| entry["via_group"].is_object())
            .count(),
        3
    );

    // The chain records the group's whole life, and the membership event
    // carries the *difference* rather than the list.
    let actions = chain_actions(&state, tenant_id).await;
    for expected in [
        "access.group.created",
        "access.group.updated",
        "access.granted",
    ] {
        assert!(
            actions.iter().any(|action| action == expected),
            "the chain must record {expected}: {actions:?}"
        );
    }
    let chain = chain_text(&state, tenant_id).await;
    assert!(chain.contains(&format!(r#""added":["{sam}"]"#)), "{chain}");
}

/// Directory-owned group authority has one public mutation surface. It is
/// idempotent, carries stable provider evidence, and the ordinary grant route
/// cannot create or revoke the same row behind the adapter's back.
#[tokio::test]
async fn a_directory_access_assignment_is_governed_and_source_owned() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, _) = seed(&app, &token).await;
    let group_id = seed_directory_group(&state, tenant_id).await;
    let (_, workspace) = call(
        &app,
        "GET",
        &format!("/v1/workspaces/{workspace_id}"),
        Some(&token),
        None,
        None,
    )
    .await;
    let scope_id = workspace["scope_id"].as_str().expect("scope id");
    let body = json!({"scope_id": scope_id, "group_id": group_id, "role": "member"});

    let (status, refused) = call(
        &app,
        "POST",
        "/v1/admin/grants",
        Some(&token),
        Some("ordinary-directory-grant"),
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");

    let (status, created) = call(
        &app,
        "POST",
        "/v1/directory/access-assignments",
        Some(&token),
        Some("directory-assignment-1"),
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["source"], "directory");
    assert_eq!(created["directory_source"], "entra");
    assert_eq!(created["directory_resource_id"], "entra-group-engineering");
    let grant_id = created["id"].as_str().expect("grant id");

    let (status, replay) = call(
        &app,
        "POST",
        "/v1/directory/access-assignments",
        Some(&token),
        Some("directory-assignment-1"),
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["id"], grant_id);

    let (status, refused) = call(
        &app,
        "DELETE",
        &format!("/v1/admin/grants/{grant_id}"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");

    let (status, body) = call(
        &app,
        "DELETE",
        &format!("/v1/directory/access-assignments/{grant_id}"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let chain = chain_text(&state, tenant_id).await;
    assert!(chain.contains("directory-access-assignment"), "{chain}");
    assert!(chain.contains("entra-group-engineering"), "{chain}");
}

/// A stale `expected_revision` is a 409 that writes nothing — membership
/// included.
#[tokio::test]
async fn a_stale_group_precondition_is_a_conflict_that_writes_nothing() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let robin = provision_identity(&state, tenant_id, "cpr5-robin").await;
    let app = router(state);
    let token = issue(ADMIN, tenant_id);

    let (_, group) = call(
        &app,
        "POST",
        "/v1/admin/groups",
        Some(&token),
        Some("grp-stale"),
        Some(json!({"slug": "eng", "display_name": "Eng", "members": [robin]})),
    )
    .await;
    let group_id = group["id"].as_str().expect("id").to_owned();

    let (status, _) = call(
        &app,
        "PATCH",
        &format!("/v1/admin/groups/{group_id}"),
        Some(&token),
        None,
        Some(json!({"expected_revision": 1, "display_name": "Engineering"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = call(
        &app,
        "PATCH",
        &format!("/v1/admin/groups/{group_id}"),
        Some(&token),
        None,
        Some(json!({"expected_revision": 1, "members": []})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let (_, listed) = call(&app, "GET", "/v1/admin/groups", Some(&token), None, None).await;
    assert_eq!(
        listed["groups"][0]["members"]
            .as_array()
            .expect("members")
            .len(),
        1,
        "the refused update must not have emptied the group: {listed}"
    );
}

/// A project-only grant, added and removed through the members routes — and the
/// removal refused when the authority is somewhere else, with the place to go.
#[tokio::test]
async fn a_project_member_is_added_and_removed_and_an_inherited_one_is_not() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, project_id) = seed(&app, &token).await;
    let (_, workspace) = call(
        &app,
        "GET",
        &format!("/v1/workspaces/{workspace_id}"),
        Some(&token),
        None,
        None,
    )
    .await;
    let workspace_scope = workspace["scope_id"].as_str().expect("scope").to_owned();

    let (status, grant) = call(
        &app,
        "POST",
        &format!("/v1/projects/{project_id}/members"),
        Some(&token),
        Some("mem-1"),
        Some(json!({"principal_id": "cpr5-robin", "role": "member"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{grant}");
    assert_eq!(grant["source"], "direct");

    // Somebody whose only authority here comes from the workspace above.
    let (status, inherited) = call(
        &app,
        "POST",
        "/v1/admin/grants",
        Some(&token),
        Some("mem-inherited"),
        Some(json!({
            "scope_id": workspace_scope,
            "principal_id": "cpr5-kim",
            "role": "member",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inherited}");

    // The path names the scope; a body that could name another one would make
    // the path a suggestion.
    let (status, body) = call(
        &app,
        "POST",
        &format!("/v1/projects/{project_id}/members"),
        Some(&token),
        Some("mem-scoped"),
        Some(json!({
            "principal_id": "cpr5-kim",
            "role": "member",
            "scope_id": "00000000-0000-0000-0000-000000000000",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // Kim's access comes from the workspace; removing it *here* would leave
    // the access in place, so it is refused and the refusal names where the
    // grant actually is.
    let (status, body) = call(
        &app,
        "DELETE",
        &format!("/v1/projects/{project_id}/members/cpr5-kim"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains(&workspace_scope),
        "{body}"
    );

    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/v1/projects/{project_id}/members/cpr5-robin"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, members) = call(
        &app,
        "GET",
        &format!("/v1/projects/{project_id}/members"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert!(!members.to_string().contains("cpr5-robin"), "{members}");
    assert!(
        chain_actions(&state, tenant_id)
            .await
            .iter()
            .any(|action| action == "access.revoked")
    );
}

/// Revoking a grant by id removes exactly what it named.
#[tokio::test]
async fn a_grant_is_revoked_by_id() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, _) = seed(&app, &token).await;
    let (_, workspace) = call(
        &app,
        "GET",
        &format!("/v1/workspaces/{workspace_id}"),
        Some(&token),
        None,
        None,
    )
    .await;
    let scope = workspace["scope_id"].as_str().expect("scope").to_owned();

    let (_, grant) = call(
        &app,
        "POST",
        "/v1/admin/grants",
        Some(&token),
        Some("grant-revoke"),
        Some(json!({"scope_id": scope, "principal_id": "cpr5-robin", "role": "viewer"})),
    )
    .await;
    let grant_id = grant["id"].as_str().expect("id").to_owned();

    let (status, listed) = call(
        &app,
        "GET",
        &format!("/v1/admin/grants?scope_id={scope}"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(
        listed["grants"].as_array().expect("grants").len(),
        2,
        "the owner grant and this one: {listed}"
    );

    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/v1/admin/grants/{grant_id}"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/v1/admin/grants/{grant_id}"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "revoking twice is a 404");
}

// ── Idempotency ──────────────────────────────────────────────────────────────

/// The plane's rule, on this plane: same key and same body replays with 200,
/// same key and a different body is a 409, and no key at all is refused with a
/// message naming the header.
#[tokio::test]
async fn creation_on_this_plane_is_idempotent() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let token = issue(ADMIN, tenant_id);
    let body = json!({"slug": "eng", "display_name": "Eng"});

    let (first, created) = call(
        &app,
        "POST",
        "/v1/admin/groups",
        Some(&token),
        Some("k-1"),
        Some(body.clone()),
    )
    .await;
    assert_eq!(first, StatusCode::CREATED, "{created}");

    let (again, replay) = call(
        &app,
        "POST",
        "/v1/admin/groups",
        Some(&token),
        Some("k-1"),
        Some(body),
    )
    .await;
    assert_eq!(again, StatusCode::OK, "{replay}");
    assert_eq!(replay["id"], created["id"]);

    let (reused, body) = call(
        &app,
        "POST",
        "/v1/admin/groups",
        Some(&token),
        Some("k-1"),
        Some(json!({"slug": "sales", "display_name": "Sales"})),
    )
    .await;
    assert_eq!(reused, StatusCode::CONFLICT, "{body}");

    let (missing, body) = call(
        &app,
        "POST",
        "/v1/admin/groups",
        Some(&token),
        None,
        Some(json!({"slug": "ops", "display_name": "Ops"})),
    )
    .await;
    assert_eq!(missing, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("Idempotency-Key"),
        "{body}"
    );
}

/// An invitation creation replayed with the same key is a **409 naming the
/// invitation**, not a 200 with the token missing: a token is shown once, and a
/// replay that could not re-show it must say so rather than serve a broken
/// body.
#[tokio::test]
async fn a_replayed_invitation_creation_says_the_token_cannot_be_re_served() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, _) = seed(&app, &token).await;
    let body = json!({"role": "member"});

    let (first, created) = call(
        &app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/invites"),
        Some(&token),
        Some("inv-replay"),
        Some(body.clone()),
    )
    .await;
    assert_eq!(first, StatusCode::CREATED, "{created}");

    let (again, refusal) = call(
        &app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/invites"),
        Some(&token),
        Some("inv-replay"),
        Some(body),
    )
    .await;
    assert_eq!(again, StatusCode::CONFLICT, "{refusal}");
    let message = refusal["message"].as_str().unwrap_or_default();
    assert!(message.contains("shown once"), "{refusal}");
    assert!(
        !refusal.to_string().contains("synveda_invite_v1"),
        "the refusal must not re-serve the token: {refusal}"
    );
}

// ── The PDP ──────────────────────────────────────────────────────────────────

/// Every route on this plane denies without its action, and refuses without a
/// credential. Written as a sweep, because a plane with fourteen operations is
/// a plane where one gets forgotten.
#[tokio::test]
async fn every_route_denies_without_the_action_and_refuses_without_a_credential() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let admin = issue(ADMIN, tenant_id);
    let outsider = issue(OUTSIDER, tenant_id);
    let (workspace_id, project_id) = seed(&app, &admin).await;
    let (_, group) = call(
        &app,
        "POST",
        "/v1/admin/groups",
        Some(&admin),
        Some("grp-pdp"),
        Some(json!({"slug": "eng", "display_name": "Eng"})),
    )
    .await;
    let group_id = group["id"].as_str().expect("id").to_owned();
    let (_, invite) = call(
        &app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/invites"),
        Some(&admin),
        Some("inv-pdp"),
        Some(json!({"role": "member"})),
    )
    .await;
    let invite_id = invite["invite"]["id"].as_str().expect("id").to_owned();
    // Real ids for the two admin-grant routes. Since CPR-6 they resolve what
    // they are about **before** they decide — a grant names its scope and a
    // revocation names the grant (ADR-0073 decision 3) — so a made-up id is a
    // 404 rather than a denial, which is the ownership-first order ADR-0012
    // decision 7 sets and would make this sweep assert nothing.
    let (_, workspace_view) = call(
        &app,
        "GET",
        &format!("/v1/workspaces/{workspace_id}"),
        Some(&admin),
        None,
        None,
    )
    .await;
    let workspace_scope = workspace_view["scope_id"]
        .as_str()
        .expect("scope")
        .to_owned();
    let (_, seeded_grant) = call(
        &app,
        "POST",
        "/v1/admin/grants",
        Some(&admin),
        Some("grant-pdp"),
        Some(json!({
            "scope_id": workspace_scope,
            "principal_id": "cpr5-someone",
            "role": "viewer",
        })),
    )
    .await;
    let grant_id = seeded_grant["id"].as_str().expect("id").to_owned();

    // (method, path, body) — every operation except `invite.accept`, which the
    // packs deliberately permit to everybody (the token is the authority) and
    // which has its own test above.
    let routes: Vec<(&str, String, Option<Value>)> = vec![
        (
            "GET",
            format!("/v1/workspaces/{workspace_id}/members"),
            None,
        ),
        (
            "GET",
            format!("/v1/workspaces/{workspace_id}/invites"),
            None,
        ),
        (
            "POST",
            format!("/v1/workspaces/{workspace_id}/invites"),
            Some(json!({"role": "member"})),
        ),
        (
            "DELETE",
            format!("/v1/workspaces/{workspace_id}/invites/{invite_id}"),
            None,
        ),
        ("GET", format!("/v1/projects/{project_id}/members"), None),
        (
            "POST",
            format!("/v1/projects/{project_id}/members"),
            Some(json!({"principal_id": "x", "role": "member"})),
        ),
        (
            "DELETE",
            format!("/v1/projects/{project_id}/members/{ADMIN}"),
            None,
        ),
        ("GET", "/v1/admin/groups".to_owned(), None),
        (
            "POST",
            "/v1/admin/groups".to_owned(),
            Some(json!({"slug": "ops", "display_name": "Ops"})),
        ),
        (
            "PATCH",
            format!("/v1/admin/groups/{group_id}"),
            Some(json!({"expected_revision": 1, "display_name": "Ops"})),
        ),
        ("GET", "/v1/admin/grants".to_owned(), None),
        (
            "POST",
            "/v1/admin/grants".to_owned(),
            Some(json!({"scope_id": workspace_scope, "principal_id": "x", "role": "member"})),
        ),
        ("DELETE", format!("/v1/admin/grants/{grant_id}"), None),
    ];

    for (method, path, body) in routes {
        let (denied, response) = call(
            &app,
            method,
            &path,
            Some(&outsider),
            Some("pdp-key"),
            body.clone(),
        )
        .await;
        assert_eq!(
            denied,
            StatusCode::FORBIDDEN,
            "{method} {path} must deny a caller with no roles, got {denied}: {response}"
        );
        assert_eq!(response["kind"], "policy_denied", "{method} {path}");

        let (anonymous, _) = call(&app, method, &path, None, Some("pdp-key"), body).await;
        assert_eq!(
            anonymous,
            StatusCode::UNAUTHORIZED,
            "{method} {path} must refuse a request with no credential"
        );
    }
}

/// A replay still takes the decision: a caller whose authority was revoked
/// between the attempt and the retry is refused, because a replay that skipped
/// the PDP would be a cached authorisation.
#[tokio::test]
async fn a_replay_still_takes_the_pdp_decision() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let body = json!({"slug": "eng", "display_name": "Eng"});

    let (first, _) = call(
        &app,
        "POST",
        "/v1/admin/groups",
        Some(&token),
        Some("k-revoked"),
        Some(body.clone()),
    )
    .await;
    assert_eq!(first, StatusCode::CREATED);

    // The grant goes away between the two calls.
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
    let grant_id = grants
        .iter()
        .find(|grant| grant.role_key == RoleKey::Administrator)
        .map(|grant| grant.id)
        .expect("the admin grant");
    synveda_store::access::revoke_grant(&mut tx, tenant_id, grant_id)
        .await
        .expect("revoke");
    tx.commit().await.expect("commit");

    let (replay, response) = call(
        &app,
        "POST",
        "/v1/admin/groups",
        Some(&token),
        Some("k-revoked"),
        Some(body),
    )
    .await;
    assert_eq!(
        replay,
        StatusCode::FORBIDDEN,
        "a replay must be decided, not cached: {response}"
    );
}

/// Another tenant's group, grant and invitation are 404s rather than 403s: no
/// existence oracle across tenants.
#[tokio::test]
async fn another_tenants_rows_are_not_found_rather_than_forbidden() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let Some((_, other_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state);
    let token = issue(ADMIN, tenant_id);
    let other = issue(ADMIN, other_id);

    let (workspace_id, _) = seed(&app, &other).await;
    let (_, group) = call(
        &app,
        "POST",
        "/v1/admin/groups",
        Some(&other),
        Some("grp-other"),
        Some(json!({"slug": "eng", "display_name": "Eng"})),
    )
    .await;
    let group_id = group["id"].as_str().expect("id").to_owned();

    for (method, path, body) in [
        (
            "GET",
            format!("/v1/workspaces/{workspace_id}/members"),
            None,
        ),
        (
            "PATCH",
            format!("/v1/admin/groups/{group_id}"),
            Some(json!({"expected_revision": 1, "display_name": "Mine"})),
        ),
    ] {
        let (status, response) = call(&app, method, &path, Some(&token), None, body).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{method} {path} must be a 404 across tenants, got {status}: {response}"
        );
    }

    // And the listings show this tenant's nothing rather than the other's.
    let (_, groups) = call(&app, "GET", "/v1/admin/groups", Some(&token), None, None).await;
    assert!(groups["groups"].as_array().expect("groups").is_empty());
}
