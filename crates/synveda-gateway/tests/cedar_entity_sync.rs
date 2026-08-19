//! The HIER-3 AC at the product surface (ADR-0017): move a team between
//! departments and authz decisions reflect it in the same transaction
//! boundary — the mutating request commits, the unified seam flushes the
//! chain cache and the entity fragments, and the very next decision is
//! made against the new hierarchy. Demonstrated twice:
//!
//! 1. At the HTTP surface: a steward bound at one department moves a
//!    team out of it, and the steward's own authority over that team is
//!    gone on the very next request.
//! 2. At the composition seam (`MemoryRead`, what CTX-2/3 will ask): a
//!    member's department read of the team flips when an admin moves it
//!    back — chains resolved through the same scope-chain cache the
//!    handlers use, decisions through the same facade, `standard`'s
//!    department rule as the probe. Never a PDP bypass (CLAUDE.md).
//!
//! Tests that need a live Postgres read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database); run them locally with
//! `make dev-up` then `make db-test`.

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
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_policy::ScopeNode;
use synveda_policy::{Action, AuthzContext, Pdp, Principal, Resource, STANDARD};
use synveda_store::{rls, role_bindings};
use synveda_types::{HierarchyNode, Role, ScopeId, Sensitivity, TenantId};
use tower::ServiceExt;

const SECRET: &[u8] = b"hier-3-test-secret";
const ADMIN: &str = "hier3-admin";
const STEWARD: &str = "hier3-steward";

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

fn issue(tenant_id: TenantId, subject: &str) -> String {
    Hs256Verifier::new(SECRET).issue(subject, tenant_id, Duration::from_secs(300))
}

async fn admitted_tenant(pool: &PgPool) -> TenantId {
    let id = TenantId::new();
    let slug = format!("hier3-{}", id.as_uuid().simple());
    synveda_store::tenants::create(
        pool,
        id,
        &slug,
        "HIER-3 entity sync test",
        synveda_types::TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    // Seed the admin binding through the store — the CLI's bootstrap
    // path (ADR-0015); enforcement still runs through the PDP.
    let mut tx = rls::begin_tenant_tx(pool, id)
        .await
        .expect("begin tenant tx");
    role_bindings::bind(&mut *tx, id, ADMIN, None, Role::OrgAdmin)
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

async fn create_node(
    app: &Router,
    token: &str,
    parent: Option<ScopeId>,
    kind: &str,
    slug: &str,
) -> HierarchyNode {
    let mut body = json!({"kind": kind, "slug": slug, "name": slug});
    if let Some(parent) = parent {
        body["parent_id"] = json!(parent);
    }
    let (status, node) = api(app, "POST", "/v1/hierarchy/nodes", token, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "create {slug}: {node}");
    serde_json::from_value(node).expect("hierarchy node")
}

/// One `MemoryRead` decision for `member` on `target`, with both chains
/// resolved through the gateway's scope-chain cache — exactly what the
/// composition engine will feed the facade (ADR-0014 decision 5,
/// ADR-0016).
async fn member_reads(
    state: &AppState,
    tenant_id: TenantId,
    member: &Principal,
    target: ScopeId,
) -> bool {
    let mut conn = state.pool.acquire().await.expect("acquire connection");
    let scopes = state
        .scope_chains
        .resolve(&mut *conn, tenant_id, target)
        .await
        .expect("resolve resource chain")
        .expect("resource exists");
    let principal_scopes = state
        .scope_chains
        .resolve(
            &mut *conn,
            tenant_id,
            member.scope_id.expect("member is placed"),
        )
        .await
        .expect("resolve placement chain")
        .expect("placement exists");
    state
        .pdp
        .authorize(
            member,
            Action::MemoryRead,
            Resource::Scope(target),
            &AuthzContext {
                scopes: &ScopeNode::from_hierarchy_chain(&scopes),
                principal_scopes: &ScopeNode::from_hierarchy_chain(&principal_scopes),
                default_pack: Some(STANDARD),
                sensitivity: Some(Sensitivity::WORKING),
                ..Default::default()
            },
        )
        .expect("authorize")
        .allowed
}

/// The HIER-3 AC end to end: a team moved between departments is
/// decided against the new hierarchy on the very next request — the
/// steward that moved it loses it, and the department read follows the
/// team home again — with the entity fragments serving the warm calls.
#[tokio::test]
async fn a_team_move_governs_the_very_next_decision() {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping cedar entity sync test: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
            return;
        }
    };
    let state = state(&url);
    let pool = state.pool.clone();
    let handle = state.metrics.clone();
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");
    let tenant_id = admitted_tenant(&pool).await;
    let app = router(state.clone());
    let admin = issue(tenant_id, ADMIN);
    let steward = issue(tenant_id, STEWARD);

    let org = create_node(&app, &admin, None, "org", "acme").await;
    let dept_x = create_node(&app, &admin, Some(org.id), "department", "dept-x").await;
    let dept_y = create_node(&app, &admin, Some(org.id), "department", "dept-y").await;
    let team_a = create_node(&app, &admin, Some(dept_x.id), "team", "team-a").await;
    let team_b = create_node(&app, &admin, Some(dept_x.id), "team", "team-b").await;
    let alice = create_node(&app, &admin, Some(team_a.id), "user", "alice-user").await;

    // A steward bound at dept-x: authority over exactly that subtree
    // (ADR-0015 decision 3).
    let mut tx = rls::begin_tenant_tx(&pool, tenant_id)
        .await
        .expect("begin tenant tx");
    role_bindings::bind(&mut *tx, tenant_id, STEWARD, Some(dept_x.id), Role::Steward)
        .await
        .expect("bind steward");
    tx.commit().await.expect("commit binding");

    // The steward governs team-b while it lives under dept-x.
    let (status, renamed) = api(
        &app,
        "PATCH",
        &format!("/v1/hierarchy/nodes/{}", team_b.id),
        &steward,
        Some(json!({"name": "Payments"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "steward renames own team: {renamed}"
    );

    // The steward moves team-b to dept-y — and that very commit takes
    // team-b out of the steward's bound subtree.
    let (status, moved) = api(
        &app,
        "PATCH",
        &format!("/v1/hierarchy/nodes/{}", team_b.id),
        &steward,
        Some(json!({"parent_id": dept_y.id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "steward moves team out: {moved}");

    // The same request that succeeded a moment ago is denied on the
    // very next call: the decision rides the post-move hierarchy — no
    // refresh interval, no eventual consistency.
    let (status, denied) = api(
        &app,
        "PATCH",
        &format!("/v1/hierarchy/nodes/{}", team_b.id),
        &steward,
        Some(json!({"name": "Payments Platform"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the mover's authority must not survive the move: {denied}"
    );

    // The composition seam (`standard`'s department rule): alice sits in
    // team-a under dept-x; team-b now lives in dept-y, so her
    // department read of it denies — warm the decision twice so the
    // repeat serves prebuilt fragments.
    let member = Principal {
        tenant_id,
        subject: "alice".to_owned(),
        quarantined: false,
        scope_id: Some(alice.id),
        token_scope: None,
    };
    assert!(!member_reads(&state, tenant_id, &member, team_b.id).await);
    assert!(!member_reads(&state, tenant_id, &member, team_b.id).await);

    // The admin brings team-b home; the handler's unified seam flushes
    // chains and fragments post-commit (ADR-0017 decision 5), and the
    // very next decision sees the team back in alice's department.
    let (status, back) = api(
        &app,
        "PATCH",
        &format!("/v1/hierarchy/nodes/{}", team_b.id),
        &admin,
        Some(json!({"parent_id": dept_x.id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin moves team back: {back}");
    assert!(
        member_reads(&state, tenant_id, &member, team_b.id).await,
        "the department read must follow the team on the very next decision"
    );

    // The entity store carried the warm calls and rebuilt on the moves.
    let rendered = handle.render();
    assert!(
        rendered.contains(r#"synveda_cedar_entity_fragments_total{outcome="hit"}"#),
        "warm decisions must serve prebuilt fragments"
    );
    assert!(
        rendered.contains(r#"synveda_cedar_entity_fragments_total{outcome="rebuild"}"#),
        "reshaped chains must rebuild their fragments"
    );
    assert!(
        rendered.contains("synveda_cedar_entity_flushes_total"),
        "the mutating handlers must flush the tenant's fragments"
    );
}
