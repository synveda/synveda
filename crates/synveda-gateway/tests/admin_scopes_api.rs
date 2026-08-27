//! CPR-7 acceptance criteria at the HTTP surface (ADR-0074): the scope
//! admin plane — and the negative space around it.
//!
//! The affirmative half: the six `/v1/admin/scopes` routes create, read,
//! rename, archive, **move** and walk governed scopes, each mutation
//! PDP-decided and audited, creation idempotent, a move decided at both
//! ends. The negative half is this prompt's own: every `/v1/hierarchy`
//! route answers **404** (deleted, not aliased — the pre-1.0 contract),
//! and every old scope kind (`org`, `division`, `department`, `team`,
//! `user`) fails validation **by name**. The store-level contract is
//! `crates/synveda-store/tests/scopes.rs`; this suite proves the things
//! only the HTTP surface has.
//!
//! Tests that need a live Postgres read `DATABASE_URL` and skip with a
//! message when it is unset; run them locally with `make db-test`.

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

const SECRET: &[u8] = b"cpr-7-admin-scopes-test-secret";
/// The subject the admin token carries — an `administrator` grant at the
/// tenant root scope, minted the way the JIT admin-group convention now
/// mints it (ADR-0074 decision 4: the operator door).
const ADMIN: &str = "cpr7-admin";
/// A subject with an identity and no grant at all: the caller every
/// denial assertion uses.
const OUTSIDER: &str = "cpr7-outsider";

/// Serialises tests: the Prometheus recorder is process-global.
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
        pdp: Arc::new(synveda_policy::Pdp::new().expect("build the embedded Pdp")),
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

/// Connects, migrates, admits a tenant, and gives the admin subject the
/// `administrator` grant at the tenant root — the operator door ADR-0073
/// recorded as missing and ADR-0074 decision 4 built.
async fn admitted_tenant() -> Option<(AppState, TenantId)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping admin scopes API test: DATABASE_URL is not set \
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
    let slug = format!("cpr7-{}", id.as_uuid().simple());
    synveda_store::tenants::create(
        &pool,
        id,
        &slug,
        "CPR-7 API test",
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
            id: synveda_types::GrantId::new(),
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
    .expect("grant administrator at the root");
    // The outsider has an identity — and, through it, an own scope — and
    // nothing else: the cutover's reading of "unmapped".
    let own = synveda_store::scopes::ensure_principal_scope(&mut tx, id, OUTSIDER, OUTSIDER)
        .await
        .expect("mint outsider own scope");
    synveda_store::identities::create(
        &mut tx,
        synveda_types::IdentityId::new(),
        id,
        Some(OUTSIDER),
        synveda_types::IdentityKind::User,
        None,
        None,
        own.id,
    )
    .await
    .expect("create outsider identity");
    tx.commit().await.expect("commit fixture");
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

async fn create_scope(app: &Router, token: &str, parent: &str, slug: &str) -> Value {
    let (status, scope) = call(
        app,
        "POST",
        "/v1/admin/scopes",
        Some(token),
        Some(&format!("seed-{slug}-{parent}")),
        Some(json!({
            "parent_id": parent,
            "kind": "org_unit",
            "slug": slug,
            "display_name": slug,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{scope}");
    scope
}

/// Every route the old hierarchy plane served answers **404** — deleted,
/// not aliased (the pre-1.0 hard cut; ADR-0074 decision 1).
#[tokio::test]
async fn every_old_hierarchy_route_is_gone() {
    let Some((state, tenant)) = admitted_tenant().await else {
        return;
    };
    let _guard = serial().await;
    let app = router(state);
    let token = issue(ADMIN, tenant);
    let made_up = "11111111-1111-1111-1111-111111111111";
    for (method, path) in [
        ("POST", "/v1/hierarchy/nodes"),
        ("GET", "/v1/hierarchy/root"),
        ("GET", &format!("/v1/hierarchy/nodes/{made_up}")),
        ("PATCH", &format!("/v1/hierarchy/nodes/{made_up}")),
        ("DELETE", &format!("/v1/hierarchy/nodes/{made_up}")),
        ("GET", &format!("/v1/hierarchy/nodes/{made_up}/children")),
        ("GET", &format!("/v1/hierarchy/nodes/{made_up}/ancestors")),
        ("GET", &format!("/v1/hierarchy/nodes/{made_up}/descendants")),
        ("GET", &format!("/v1/hierarchy/nodes/{made_up}/policy")),
        ("PUT", &format!("/v1/hierarchy/nodes/{made_up}/policy")),
        ("GET", &format!("/v1/hierarchy/nodes/{made_up}/roles")),
        ("PUT", &format!("/v1/hierarchy/nodes/{made_up}/roles")),
        (
            "GET",
            &format!("/v1/hierarchy/nodes/{made_up}/capabilities"),
        ),
        ("GET", &format!("/v1/hierarchy/nodes/{made_up}/curators")),
        ("PUT", &format!("/v1/hierarchy/nodes/{made_up}/curators")),
        ("GET", "/v1/roles/bindings"),
        ("PUT", "/v1/roles/bindings"),
    ] {
        let (status, body) = call(
            &app,
            method,
            path,
            Some(&token),
            None,
            Some(json!({"parent_id": made_up, "kind": "org", "slug": "x", "name": "x"})),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{method} {path} must be gone with the hierarchy, got {status}: {body}"
        );
    }
}

/// Every old scope kind fails validation **by name** — `org`,
/// `division`, `department`, `team`, `user` — with the shape vocabulary
/// answering in their place.
#[tokio::test]
async fn every_old_scope_kind_fails_validation_by_name() {
    let Some((state, tenant)) = admitted_tenant().await else {
        return;
    };
    let _guard = serial().await;
    let app = router(state);
    let token = issue(ADMIN, tenant);
    let (_, root_level) = call(&app, "GET", "/v1/admin/scopes", Some(&token), None, None).await;
    let root = root_level["parent"]["id"]
        .as_str()
        .expect("the tenant root is served")
        .to_owned();
    for kind in ["org", "division", "department", "team", "user"] {
        let (status, body) = call(
            &app,
            "POST",
            "/v1/admin/scopes",
            Some(&token),
            Some(&format!("old-kind-{kind}")),
            Some(json!({
                "parent_id": root,
                "kind": kind,
                "slug": format!("old-{kind}"),
                "display_name": kind,
            })),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "the old kind {kind:?} must fail validation, got {status}: {body}"
        );
        let message = body["message"].as_str().unwrap_or_default();
        assert!(
            message.contains(kind),
            "the refusal should name {kind}: {message}"
        );
    }
    // The five shapes are the vocabulary that answers in their place —
    // one of them creates cleanly.
    let (status, _) = call(
        &app,
        "POST",
        "/v1/admin/scopes",
        Some(&token),
        Some("shape-vocabulary-works"),
        Some(json!({
            "parent_id": root,
            "kind": "org_unit",
            "slug": "shape-check",
            "display_name": "shape check",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
}

/// The whole plane from nothing: the level listing, creation under the
/// root, show with a path, ancestors, descendants — and a rename and an
/// archive through PATCH.
#[tokio::test]
async fn the_six_routes_walk_create_read_and_change_scopes() {
    let Some((state, tenant)) = admitted_tenant().await else {
        return;
    };
    let _guard = serial().await;
    let app = router(state);
    let token = issue(ADMIN, tenant);

    // The unfiltered level: the root and its children (none yet).
    let (status, level) = call(&app, "GET", "/v1/admin/scopes", Some(&token), None, None).await;
    assert_eq!(status, StatusCode::OK, "{level}");
    let root = level["parent"]["id"]
        .as_str()
        .expect("root served")
        .to_owned();
    assert_eq!(level["parent"]["kind"], "tenant");

    // Create an org unit and a workspace under it.
    let unit = create_scope(&app, &token, &root, "eng").await;
    assert_eq!(unit["kind"], "org_unit");
    let (status, workspace) = call(
        &app,
        "POST",
        "/v1/admin/scopes",
        Some(&token),
        Some("seed-ws-eng"),
        Some(json!({
            "parent_id": unit["id"],
            "kind": "workspace",
            "slug": "platform",
            "display_name": "Platform",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workspace}");

    // The level under the unit lists the workspace.
    let (status, children) = call(
        &app,
        "GET",
        &format!(
            "/v1/admin/scopes?parent_id={}",
            unit["id"].as_str().unwrap()
        ),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{children}");
    assert_eq!(
        children["scopes"]
            .as_array()
            .expect("scopes list")
            .iter()
            .filter(|scope| scope["slug"] == "platform")
            .count(),
        1
    );

    // Show serves the path.
    let (status, detail) = call(
        &app,
        "GET",
        &format!("/v1/admin/scopes/{}", workspace["id"].as_str().unwrap()),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(
        detail["path"],
        format!(
            "{}/eng/platform",
            level["parent"]["slug"].as_str().expect("root slug")
        )
    );

    // Ancestors walk to the root; descendants cover the subtree.
    let (status, ancestors) = call(
        &app,
        "GET",
        &format!(
            "/v1/admin/scopes/{}/ancestors",
            workspace["id"].as_str().unwrap()
        ),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ancestors}");
    assert_eq!(
        ancestors["scopes"].as_array().map(Vec::len),
        Some(2),
        "eng then the root, nearest first"
    );
    let (status, subtree) = call(
        &app,
        "GET",
        &format!("/v1/admin/scopes/{}/descendants", root),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{subtree}");
    let structural: Vec<&Value> = subtree["scopes"]
        .as_array()
        .expect("scopes list")
        .iter()
        .filter(|scope| scope["kind"] != "principal")
        .collect();
    assert_eq!(
        structural.len(),
        2,
        "eng and platform, the root itself excluded — the outsider's own \
         principal scope hangs at the root too and is no less theirs for it"
    );

    // Rename and archive through PATCH.
    let (status, renamed) = call(
        &app,
        "PATCH",
        &format!("/v1/admin/scopes/{}", unit["id"].as_str().unwrap()),
        Some(&token),
        None,
        Some(json!({"display_name": "Engineering"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["display_name"], "Engineering");
    let (status, archived) = call(
        &app,
        "PATCH",
        &format!("/v1/admin/scopes/{}", workspace["id"].as_str().unwrap()),
        Some(&token),
        None,
        Some(json!({"status": "archived"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{archived}");
    assert_eq!(archived["status"], "archived");
}

/// Creation is idempotent under `Idempotency-Key` (the CPR-4 discipline,
/// kept): same key + same body replays with 200, a different body is 409.
#[tokio::test]
async fn creation_is_idempotent_and_a_key_reuse_conflicts() {
    let Some((state, tenant)) = admitted_tenant().await else {
        return;
    };
    let _guard = serial().await;
    let app = router(state);
    let token = issue(ADMIN, tenant);
    let (_, level) = call(&app, "GET", "/v1/admin/scopes", Some(&token), None, None).await;
    let root = level["parent"]["id"].as_str().unwrap().to_owned();
    let body = json!({
        "parent_id": root,
        "kind": "org_unit",
        "slug": "idem",
        "display_name": "Idem",
    });

    let key = "cpr7-idem-key-1";
    let (first, created) = call(
        &app,
        "POST",
        "/v1/admin/scopes",
        Some(&token),
        Some(key),
        Some(body.clone()),
    )
    .await;
    assert_eq!(first, StatusCode::CREATED, "{created}");
    let (replay, replayed) = call(
        &app,
        "POST",
        "/v1/admin/scopes",
        Some(&token),
        Some(key),
        Some(body.clone()),
    )
    .await;
    assert_eq!(replay, StatusCode::OK, "{replayed}");
    assert_eq!(
        created["id"], replayed["id"],
        "the replay is the same scope"
    );

    let (conflict, _) = call(
        &app,
        "POST",
        "/v1/admin/scopes",
        Some(&token),
        Some(key),
        Some(json!({
            "parent_id": root,
            "kind": "org_unit",
            "slug": "different",
            "display_name": "Different",
        })),
    )
    .await;
    assert_eq!(conflict, StatusCode::CONFLICT);

    // And creation without a key at all is refused with a sentence.
    let (no_key, no_key_body) = call(
        &app,
        "POST",
        "/v1/admin/scopes",
        Some(&token),
        None,
        Some(body),
    )
    .await;
    assert_eq!(no_key, StatusCode::BAD_REQUEST, "{no_key_body}");
}

/// A **move** is the one mutation decided twice — at the scope and at the
/// destination — audited with both ends, refused into its own subtree.
#[tokio::test]
async fn a_move_is_decided_at_both_ends_and_refused_into_its_own_subtree() {
    let Some((state, tenant)) = admitted_tenant().await else {
        return;
    };
    let _guard = serial().await;
    let pool = state.pool.clone();
    let app = router(state);
    let token = issue(ADMIN, tenant);
    let (_, level) = call(&app, "GET", "/v1/admin/scopes", Some(&token), None, None).await;
    let root = level["parent"]["id"].as_str().unwrap().to_owned();
    let unit = create_scope(&app, &token, &root, "move-src").await;
    let child = create_scope(&app, &token, unit["id"].as_str().unwrap(), "move-child").await;

    // A legal move: the child out to sit beside its old parent.
    let (status, moved) = call(
        &app,
        "PATCH",
        &format!("/v1/admin/scopes/{}", child["id"].as_str().unwrap()),
        Some(&token),
        None,
        Some(json!({"parent_scope_id": root})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["parent_scope_id"], root);

    // A move into the scope's own subtree is refused — the cycle guard.
    // `deep` stays under `unit`, so moving `unit` under `deep` is a cycle
    // the store must refuse even though both ends are administrator-held.
    let deep = create_scope(&app, &token, unit["id"].as_str().unwrap(), "move-deep").await;
    let (status, cycle) = call(
        &app,
        "PATCH",
        &format!("/v1/admin/scopes/{}", unit["id"].as_str().unwrap()),
        Some(&token),
        None,
        Some(json!({"parent_scope_id": deep["id"]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{cycle}");

    // The audit event names both ends.
    let rows: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "select action, payload from audit_log where tenant_id = $1 and action = 'scope.updated'",
    )
    .bind(tenant.as_uuid())
    .fetch_all(&pool)
    .await
    .expect("read audit events");
    let moving = rows
        .iter()
        .find(|(_, payload)| payload["moved_to"].is_object())
        .expect("a move chains an event carrying both ends");
    assert_eq!(
        moving.1["moved_to"]["id"].as_str(),
        Some(root.as_str()),
        "the destination the subtree landed in: {}",
        moving.1
    );
    assert_eq!(
        moving.1["moved_from"]["id"].as_str(),
        unit["id"].as_str(),
        "and the parent it left, which is the other half of the act: {}",
        moving.1
    );
}

/// The outsider — an identity, an own scope, no grant — mutates nothing
/// and reads nothing: `ScopeRead` is owner/administrator under every
/// shipped pack, and the denials chain `authz.decision` events at the
/// respond seam.
#[tokio::test]
async fn an_ungranted_caller_mutates_nothing() {
    let Some((state, tenant)) = admitted_tenant().await else {
        return;
    };
    let _guard = serial().await;
    let app = router(state);
    let admin = issue(ADMIN, tenant);
    let outsider = issue(OUTSIDER, tenant);
    let (_, level) = call(&app, "GET", "/v1/admin/scopes", Some(&admin), None, None).await;
    let root = level["parent"]["id"].as_str().unwrap().to_owned();

    // Not even the level listing: the plane is administrators', and an
    // ungranted caller is policy-denied, not 404'd — the tenant is theirs.
    let (status, body) = call(&app, "GET", "/v1/admin/scopes", Some(&outsider), None, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    let (status, body) = call(
        &app,
        "POST",
        "/v1/admin/scopes",
        Some(&outsider),
        Some("outsider-create"),
        Some(json!({
            "parent_id": root,
            "kind": "org_unit",
            "slug": "nope",
            "display_name": "Nope",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, _) = call(
        &app,
        "PATCH",
        &format!("/v1/admin/scopes/{root}"),
        Some(&outsider),
        None,
        Some(json!({"display_name": "Nope"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // And a foreign tenant's scope is a 404, never a denial oracle.
    let (status, _) = call(
        &app,
        "GET",
        "/v1/admin/scopes/22222222-2222-2222-2222-222222222222",
        Some(&admin),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Without a credential, nothing on the plane answers.
#[tokio::test]
async fn the_plane_refuses_an_absent_credential() {
    let Some((state, _)) = admitted_tenant().await else {
        return;
    };
    let _guard = serial().await;
    let app = router(state);
    for (method, path) in [
        ("GET", "/v1/admin/scopes"),
        ("POST", "/v1/admin/scopes"),
        (
            "GET",
            "/v1/admin/scopes/00000000-0000-0000-0000-000000000000",
        ),
        (
            "PATCH",
            "/v1/admin/scopes/00000000-0000-0000-0000-000000000000",
        ),
    ] {
        let (status, _) = call(&app, method, path, None, None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}");
    }
}
