//! AUTHZ-2 AC at the product surface (ADR-0014 decision 8): pack listing,
//! the tenant default, per-node assignment with inheritance, and the
//! headline behaviour — switching one team's pack changes that team's
//! decisions on the very next request, per node, while its siblings keep
//! theirs. Restrictive behaviour comes from a *test policy pack* through
//! the same store + reload + assignment paths the product uses — never a
//! PDP bypass (CLAUDE.md, seed §2.2).
//!
//! Tests that need a live Postgres read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database); run them locally with
//! `make db-test`.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::{authz, telemetry};
use synveda_identity::Hs256Verifier;
use synveda_policy::{Pdp, REGULATED_STRICT};
use synveda_store::{access, policy_packs, rls, scopes};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::{GrantId, PackConfig, TenantId};
use tower::ServiceExt;

const SECRET: &[u8] = b"authz-2-test-secret";

/// Only permits hierarchy reads — assigned to one team to freeze it.
const FROZEN_PACK: &str = r#"
permit (
    principal,
    action == Synveda::Action::"ScopeRead",
    resource
) when { resource in principal.tenant };
"#;

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
            .acquire_timeout(Duration::from_secs(2))
            .connect_lazy(url)
            .expect("parse database url"),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp: Arc::new(Pdp::new().expect("build the embedded PDP")),
        service_token_max_ttl: std::time::Duration::from_secs(3600),
        search_index: Arc::new(
            synveda_retrieval::SearchIndex::open(
                std::env::temp_dir()
                    .join("synveda-gateway-tests")
                    .join(synveda_types::TenantId::new().to_string()),
            )
            .expect("open search index"),
        ),
        embedder: Arc::new(synveda_ingest::embedding::AnyEmbedder::Deterministic(
            synveda_ingest::embedding::DeterministicEmbedder::new(),
        )),
        inject_embed_timeout: std::time::Duration::from_millis(100),
        // TEN-4 (ADR-0064): a fixed test KEK, so a suite that touches a
        // sealed column seals rather than skipping. `Kms::Disabled` is the
        // production default when no key is configured.
        keys: std::sync::Arc::new(synveda_store::keys::KeyRing::new(
            synveda_crypto::Kms::Local(
                synveda_crypto::LocalKms::from_hex(&"11".repeat(32), "local:test")
                    .expect("test kek"),
            ),
        )),
    }
}

fn issue(tenant_id: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue("authz2-admin", tenant_id, Duration::from_secs(300))
}

async fn admitted_tenant(pool: &PgPool, label: &str) -> TenantId {
    let id = TenantId::new();
    let slug = format!("{label}-{}", id.as_uuid().simple());
    synveda_store::tenants::create(
        pool,
        id,
        &slug,
        "AUTHZ-2 policy routes test",
        synveda_types::TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    // Since AUTHZ-3 the policy admin plane requires a role (ADR-0015):
    // seed an `administrator` grant at the tenant root for the dev test
    // subject through the store — the CLI's bootstrap path. Enforcement
    // still runs through the PDP with this row as data.
    let mut tx = rls::begin_tenant_tx(pool, id)
        .await
        .expect("begin tenant tx");
    let root = scopes::ensure_tenant_root(&mut tx, id)
        .await
        .expect("mint root");
    access::create_grant(
        &mut *tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id: id,
            scope_id: root.id,
            subject: GrantSubject::Principal {
                principal_id: "authz2-admin".to_owned(),
            },
            role_key: RoleKey::Administrator,
            source: GrantSource::Automation,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .expect("grant administrator");
    tx.commit().await.expect("commit grant");
    id
}

async fn api(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    key: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
    if let Some(key) = key {
        builder = builder.header("idempotency-key", key);
    }
    let request = match body {
        Some(body) => {
            builder = builder.header("content-type", "application/json");
            builder.body(Body::from(body.to_string()))
        }
        None => builder.body(Body::empty()),
    }
    .expect("build request");
    let response = app.clone().oneshot(request).await.expect("send request");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read response body")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("json body")
    };
    (status, value)
}

fn node_id(body: &Value) -> String {
    body["id"].as_str().expect("scope id").to_owned()
}

async fn create_scope(app: &Router, token: &str, parent: &str, kind: &str, slug: &str) -> String {
    let (status, scope) = api(
        app,
        "POST",
        "/v1/admin/scopes",
        token,
        Some(&format!("authz2-create-{slug}")),
        Some(json!({"parent_id": parent, "kind": kind, "slug": slug, "display_name": slug})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create {slug}: {scope}");
    node_id(&scope)
}

/// The tenant root's id, read through the level listing.
async fn root_id(app: &Router, token: &str) -> String {
    let (status, level) = api(app, "GET", "/v1/admin/scopes", token, None, None).await;
    assert_eq!(status, StatusCode::OK, "{level}");
    level["parent"]["id"].as_str().expect("root id").to_owned()
}

/// The whole product surface in one flow: listing, default, per-node
/// assignment with inheritance and origin display, per-node enforcement
/// on the next request, and self-rescue from a restrictive assignment.
#[tokio::test]
async fn assignments_govern_per_node_from_the_next_request() {
    let _serial = serial().await;
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping policy routes test: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
            return;
        }
    };
    let state = state(&url);
    let pdp = Arc::clone(&state.pdp);
    let pool = state.pool.clone();
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");
    let tenant_id = admitted_tenant(&pool, "authz2").await;
    let app = router(state);
    let token = issue(tenant_id);

    let root = root_id(&app, &token).await;
    let eng = create_scope(&app, &token, &root, "org_unit", "eng").await;
    let team_a = create_scope(&app, &token, &eng, "org_unit", "team-a").await;
    let team_b = create_scope(&app, &token, &eng, "org_unit", "team-b").await;

    // The pack listing: the three embedded product packs, before any
    // stored pack exists.
    let (status, listing) = api(&app, "GET", "/v1/policy/packs", &token, None, None).await;
    assert_eq!(status, StatusCode::OK, "{listing}");
    let names: Vec<&str> = listing["packs"]
        .as_array()
        .expect("packs array")
        .iter()
        .map(|pack| pack["name"].as_str().expect("pack name"))
        .collect();
    assert_eq!(
        names,
        vec!["regulated-strict", "standard", "open-collaboration"],
        "the embedded product packs, in vocabulary order"
    );

    // Zero-config default: nothing stored, regulated-strict effective.
    let (status, default) = api(&app, "GET", "/v1/policy/default", &token, None, None).await;
    assert_eq!(status, StatusCode::OK, "{default}");
    assert_eq!(default["pack_name"], Value::Null);
    assert_eq!(default["effective"], REGULATED_STRICT);

    // A node with nothing assigned anywhere shows the embedded default.
    let (status, shown) = api(
        &app,
        "GET",
        &format!("/v1/admin/scopes/{team_a}/policy"),
        &token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{shown}");
    assert_eq!(shown["name"], REGULATED_STRICT);
    assert_eq!(shown["origin"]["kind"], "default");
    assert_eq!(shown["assignment"], Value::Null);

    // Assign `standard` at the department: both teams inherit it, and the
    // origin names the department node.
    let (status, assigned) = api(
        &app,
        "PUT",
        &format!("/v1/admin/scopes/{eng}/policy"),
        &token,
        None,
        Some(json!({"name": "standard"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{assigned}");
    let (status, shown) = api(
        &app,
        "GET",
        &format!("/v1/admin/scopes/{team_a}/policy"),
        &token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{shown}");
    assert_eq!(shown["name"], "standard");
    assert_eq!(shown["origin"]["kind"], "assigned");
    assert_eq!(shown["origin"]["scope_id"], eng.as_str());
    assert_eq!(
        shown["assignment"],
        Value::Null,
        "team-a itself carries no assignment"
    );

    // A name that denotes nothing is refused — embedded or stored only.
    for unknown in ["no-such-pack", "bootstrap"] {
        let (status, refused) = api(
            &app,
            "PUT",
            &format!("/v1/admin/scopes/{team_a}/policy"),
            &token,
            None,
            Some(json!({"name": unknown})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{unknown}: {refused}");
    }

    // Store a restrictive custom pack and reload it in — the source
    // distribution path.
    {
        let mut tx = rls::begin_tenant_tx(&pool, tenant_id)
            .await
            .expect("begin tenant tx");
        policy_packs::apply(
            &mut *tx,
            tenant_id,
            "authz2-frozen",
            FROZEN_PACK,
            &PackConfig::default(),
        )
        .await
        .expect("store pack");
        tx.commit().await.expect("commit pack");
    }
    assert_eq!(
        authz::refresh_tenant_packs(&pool, &pdp, tenant_id).await,
        "installed"
    );

    // The stored pack now shows in the listing.
    let (status, listing) = api(&app, "GET", "/v1/policy/packs", &token, None, None).await;
    assert_eq!(status, StatusCode::OK, "{listing}");
    assert!(
        listing["packs"]
            .as_array()
            .expect("packs array")
            .iter()
            .any(|pack| pack["name"] == "authz2-frozen" && pack["kind"] == "stored"),
        "the stored pack must be listed: {listing}"
    );

    // The AC: switch team-b to the frozen pack. The very next request is
    // governed by it — mutations on team-b deny, naming pack@version —
    // while team-a (still on the department's `standard`) mutates freely.
    let (status, switched) = api(
        &app,
        "PUT",
        &format!("/v1/admin/scopes/{team_b}/policy"),
        &token,
        None,
        Some(json!({"name": "authz2-frozen"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{switched}");

    let (status, denied) = api(
        &app,
        "PATCH",
        &format!("/v1/admin/scopes/{team_b}"),
        &token,
        None,
        Some(json!({"display_name": "Frozen Unit"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    let reason = denied["reason"].as_str().expect("reason");
    assert!(
        reason.contains("authz2-frozen@1"),
        "the denial must name pack@version, got: {reason}"
    );

    let (status, renamed) = api(
        &app,
        "PATCH",
        &format!("/v1/admin/scopes/{team_a}"),
        &token,
        None,
        Some(json!({"display_name": "Unit A"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the sibling team keeps its pack: {renamed}"
    );

    // Self-rescue (ADR-0014 decision 4): removing team-b's assignment is
    // decided under the *inherited* pack (`standard`), not the frozen one
    // — the node cannot seal itself. Team-b then mutates again.
    let (status, rescued) = api(
        &app,
        "DELETE",
        &format!("/v1/admin/scopes/{team_b}/policy"),
        &token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{rescued}");
    let (status, thawed) = api(
        &app,
        "PATCH",
        &format!("/v1/admin/scopes/{team_b}"),
        &token,
        None,
        Some(json!({"display_name": "Unit B"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{thawed}");

    // Deleting a nonexistent assignment is a 404, not a silent no-op.
    let (status, missing) = api(
        &app,
        "DELETE",
        &format!("/v1/admin/scopes/{team_b}/policy"),
        &token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");

    // The operations are visible in the metric contract.
    let exposition = metrics_handle().render();
    assert!(
        exposition
            .lines()
            .any(|line| line.starts_with("synveda_policy_operations_total")
                && line.contains("op=\"assign_scope_policy\"")
                && line.contains("outcome=\"ok\"")),
        "assign op missing from exposition:\n{exposition}"
    );
}

/// Cross-tenant probes of the policy surface see uniform 404s, never a
/// policy denial oracle (ADR-0012 decision 7).
#[tokio::test]
async fn cross_tenant_policy_probes_see_uniform_404() {
    let _serial = serial().await;
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping policy routes test: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
            return;
        }
    };
    let state = state(&url);
    let pool = state.pool.clone();
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");
    let victim = admitted_tenant(&pool, "authz2v").await;
    let intruder = admitted_tenant(&pool, "authz2i").await;
    let app = router(state);

    let victim_token = issue(victim);
    let root = root_id(&app, &victim_token).await;
    let org = create_scope(&app, &victim_token, &root, "org_unit", "acme").await;
    let foreign = issue(intruder);
    for (method, body) in [
        ("GET", None),
        ("PUT", Some(json!({"name": "standard"}))),
        ("DELETE", None),
    ] {
        let (status, response) = api(
            &app,
            method,
            &format!("/v1/admin/scopes/{org}/policy"),
            &foreign,
            None,
            body,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{method} probe must 404: {response}"
        );
    }
}
