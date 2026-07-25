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
use synveda_store::{policy_packs, rls, role_bindings};
use synveda_types::{PackConfig, Role, TenantId};
use tower::ServiceExt;

const SECRET: &[u8] = b"authz-2-test-secret";

/// Only permits hierarchy reads — assigned to one team to freeze it.
const FROZEN_PACK: &str = r#"
permit (
    principal,
    action == Synveda::Action::"HierarchyRead",
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
        pdp: Arc::new(Pdp::new().expect("build the embedded PDP")),
        scope_chains: Arc::new(synveda_store::ScopeChainCache::new()),
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
    // seed a tenant-wide org-admin binding for the dev test subject
    // through the store — the CLI's bootstrap path. Enforcement still
    // runs through the PDP with this row as data.
    let mut tx = rls::begin_tenant_tx(pool, id)
        .await
        .expect("begin tenant tx");
    role_bindings::bind(&mut *tx, id, "authz2-admin", None, Role::OrgAdmin)
        .await
        .expect("bind admin");
    tx.commit().await.expect("commit binding");
    id
}

async fn api(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
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
    body["id"].as_str().expect("node id").to_owned()
}

async fn create_node(
    app: &Router,
    token: &str,
    parent: Option<&str>,
    kind: &str,
    slug: &str,
) -> String {
    let mut body = json!({"kind": kind, "slug": slug, "name": slug});
    if let Some(parent) = parent {
        body["parent_id"] = json!(parent);
    }
    let (status, node) = api(app, "POST", "/v1/hierarchy/nodes", token, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "create {slug}: {node}");
    node_id(&node)
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

    let org = create_node(&app, &token, None, "org", "acme").await;
    let eng = create_node(&app, &token, Some(&org), "department", "eng").await;
    let team_a = create_node(&app, &token, Some(&eng), "team", "team-a").await;
    let team_b = create_node(&app, &token, Some(&eng), "team", "team-b").await;

    // The pack listing: the three embedded product packs, before any
    // stored pack exists.
    let (status, listing) = api(&app, "GET", "/v1/policy/packs", &token, None).await;
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
    let (status, default) = api(&app, "GET", "/v1/policy/default", &token, None).await;
    assert_eq!(status, StatusCode::OK, "{default}");
    assert_eq!(default["pack_name"], Value::Null);
    assert_eq!(default["effective"], REGULATED_STRICT);

    // A node with nothing assigned anywhere shows the embedded default.
    let (status, shown) = api(
        &app,
        "GET",
        &format!("/v1/hierarchy/nodes/{team_a}/policy"),
        &token,
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
        &format!("/v1/hierarchy/nodes/{eng}/policy"),
        &token,
        Some(json!({"name": "standard"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{assigned}");
    let (status, shown) = api(
        &app,
        "GET",
        &format!("/v1/hierarchy/nodes/{team_a}/policy"),
        &token,
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
            &format!("/v1/hierarchy/nodes/{team_a}/policy"),
            &token,
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
    let (status, listing) = api(&app, "GET", "/v1/policy/packs", &token, None).await;
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
        &format!("/v1/hierarchy/nodes/{team_b}/policy"),
        &token,
        Some(json!({"name": "authz2-frozen"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{switched}");

    let (status, denied) = api(
        &app,
        "PATCH",
        &format!("/v1/hierarchy/nodes/{team_b}"),
        &token,
        Some(json!({"name": "Frozen Team"})),
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
        &format!("/v1/hierarchy/nodes/{team_a}"),
        &token,
        Some(json!({"name": "Team A"})),
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
        &format!("/v1/hierarchy/nodes/{team_b}/policy"),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{rescued}");
    let (status, thawed) = api(
        &app,
        "PATCH",
        &format!("/v1/hierarchy/nodes/{team_b}"),
        &token,
        Some(json!({"name": "Team B"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{thawed}");

    // Deleting a nonexistent assignment is a 404, not a silent no-op.
    let (status, missing) = api(
        &app,
        "DELETE",
        &format!("/v1/hierarchy/nodes/{team_b}/policy"),
        &token,
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
                && line.contains("op=\"assign_node_policy\"")
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

    let org = create_node(&app, &issue(victim), None, "org", "acme").await;
    let foreign = issue(intruder);
    for (method, body) in [
        ("GET", None),
        ("PUT", Some(json!({"name": "standard"}))),
        ("DELETE", None),
    ] {
        let (status, response) = api(
            &app,
            method,
            &format!("/v1/hierarchy/nodes/{org}/policy"),
            &foreign,
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
