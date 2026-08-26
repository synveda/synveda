//! The HIER-3 AC at the product surface (ADR-0017), re-cut onto the scope
//! substrate (CPR-7, ADR-0074): move an org unit between parents and
//! authz decisions reflect it in the same transaction boundary — the
//! mutating request commits, the unified seam flushes the entity
//! fragments, and the very next decision is made against the new tree.
//! Demonstrated twice:
//!
//! 1. At the HTTP surface: an administrator granted at one org unit
//!    moves a sibling out of it, and the mover's own authority over that
//!    scope is gone on the very next request.
//! 2. At the composition seam (`KnowledgeRead`, what CTX-2/3 will ask): a
//!    member's grant reaches a scope while it lives in the granted
//!    subtree and stops the moment an admin moves it out — chains
//!    resolved through the store, decisions through the same embedded
//!    facade, `standard`'s content-role rule as the probe. Never a PDP
//!    bypass (CLAUDE.md).
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
use synveda_policy::{Action, AuthzContext, Pdp, Principal, Resource, STANDARD, ScopeNode};
use synveda_store::{access, identities, rls, scopes};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::anchor::{AnchorSource, ScopeAnchor};
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{GrantId, IdentityId, IdentityKind, ScopeId, Sensitivity, TenantId};
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
        service_token_max_ttl: std::time::Duration::from_secs(3600),
        embedder: Arc::new(synveda_ingest::embedding::AnyEmbedder::Deterministic(
            synveda_ingest::embedding::DeterministicEmbedder::new(),
        )),
        context_embed_timeout: std::time::Duration::from_millis(100),
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
    // Seed the admin's authority through the store — the CLI's bootstrap
    // path (ADR-0015); enforcement still runs through the PDP.
    let mut tx = rls::begin_tenant_tx(pool, id)
        .await
        .expect("begin tenant tx");
    let root = scopes::ensure_tenant_root(&mut tx, id)
        .await
        .expect("mint root");
    grant(&mut tx, id, ADMIN, root.id, RoleKey::Administrator).await;
    tx.commit().await.expect("commit grant");
    id
}

/// A direct grant write — the bootstrap shape, silent in the chain.
async fn grant(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    subject: &str,
    scope: ScopeId,
    role: RoleKey,
) {
    access::create_grant(
        &mut *tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id: tenant,
            scope_id: scope,
            subject: GrantSubject::Principal {
                principal_id: subject.to_owned(),
            },
            role_key: role,
            source: GrantSource::Automation,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .expect("create grant");
}

/// One org unit under a parent, seeded through the store.
async fn unit(tx: &mut sqlx::PgConnection, tenant: TenantId, parent: ScopeId, slug: &str) -> Scope {
    scopes::create(
        tx,
        &scopes::NewScope {
            id: ScopeId::new(),
            tenant_id: tenant,
            kind: ScopeKind::OrgUnit,
            parent_scope_id: Some(parent),
            slug: slug.to_owned(),
            display_name: slug.to_owned(),
            attributes: serde_json::json!({}),
            principal_id: None,
            created_by: None,
        },
    )
    .await
    .expect("create org unit")
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

/// One `KnowledgeRead` decision for `member` on `target`, with the resource
/// chain, the member's own chain and their anchors resolved from the
/// store — the inputs the gateway's `gather` assembles for the same
/// decision (ADR-0073), rebuilt here through public surfaces because the
/// composition engine will feed the facade exactly this shape.
async fn member_reads(
    state: &AppState,
    tenant_id: TenantId,
    member: &Principal,
    target: ScopeId,
    granted: ScopeId,
) -> bool {
    let mut conn = state.pool.acquire().await.expect("acquire connection");
    // The resource's chain: the scope and its ancestors, nearest first.
    let target_scope = scopes::get(&mut *conn, tenant_id, target)
        .await
        .expect("read target")
        .expect("target exists");
    let mut chain = vec![ScopeNode::from_scope(&target_scope, false)];
    for ancestor in scopes::ancestors(&mut *conn, tenant_id, target)
        .await
        .expect("resolve resource chain")
    {
        chain.push(ScopeNode::from_scope(&ancestor, false));
    }
    // The member's own chain.
    let own = scopes::get(
        &mut *conn,
        tenant_id,
        member.scope_id.expect("member has an own scope"),
    )
    .await
    .expect("read own scope")
    .expect("own scope exists");
    let mut own_chain = vec![ScopeNode::from_scope(&own, false)];
    for ancestor in scopes::ancestors(
        &mut *conn,
        tenant_id,
        member.scope_id.expect("member has an own scope"),
    )
    .await
    .expect("resolve own chain")
    {
        own_chain.push(ScopeNode::from_scope(&ancestor, false));
    }
    // Their anchors: the grant the world wrote for them, and the tenant
    // root that is always applicable (roles empty there — the resolver's
    // own shape).
    let granted_scope = scopes::get(&mut *conn, tenant_id, granted)
        .await
        .expect("read granted scope")
        .expect("granted scope exists");
    let anchors = vec![
        ScopeAnchor {
            scope_id: granted_scope.id,
            kind: granted_scope.kind,
            parent_scope_id: granted_scope.parent_scope_id,
            depth: 1,
            source: AnchorSource::Grant,
            roles: vec![RoleKey::Member],
            granted_at: vec![granted_scope.id],
            via_groups: Vec::new(),
        },
        ScopeAnchor {
            scope_id: granted_scope
                .parent_scope_id
                .expect("the grant hangs under the root"),
            kind: ScopeKind::Tenant,
            parent_scope_id: None,
            depth: 0,
            source: AnchorSource::TenantRoot,
            roles: Vec::new(),
            granted_at: Vec::new(),
            via_groups: Vec::new(),
        },
    ];
    state
        .pdp
        .authorize(
            member,
            Action::KnowledgeRead,
            Resource::Scope(target),
            &AuthzContext {
                scopes: &chain,
                principal_scopes: &own_chain,
                anchors: &anchors,
                default_pack: Some(STANDARD),
                sensitivity: Some(Sensitivity::WORKING),
                ..Default::default()
            },
        )
        .expect("authorize")
        .allowed
}

/// The HIER-3 AC end to end: a scope moved between parents is decided
/// against the new tree on the very next request — the mover that moved
/// it loses it, and the granted read follows the scope home again — with
/// the entity fragments serving the warm calls.
#[tokio::test]
async fn a_scope_move_governs_the_very_next_decision() {
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

    // Seed the tree through the store: a root, two org units, and the
    // scopes the move will carry between them.
    let mut tx = rls::begin_tenant_tx(&pool, tenant_id)
        .await
        .expect("begin tenant tx");
    let root = scopes::ensure_tenant_root(&mut tx, tenant_id)
        .await
        .expect("mint root");
    let dept_x = unit(&mut tx, tenant_id, root.id, "dept-x").await;
    let dept_y = unit(&mut tx, tenant_id, root.id, "dept-y").await;
    let team_b = unit(&mut tx, tenant_id, dept_x.id, "team-b").await;
    // A member with a grant at dept-x: authority over exactly that
    // subtree (ADR-0015 decision 3).
    grant(
        &mut tx,
        tenant_id,
        STEWARD,
        dept_x.id,
        RoleKey::Administrator,
    )
    .await;
    grant(&mut tx, tenant_id, "alice", dept_x.id, RoleKey::Member).await;
    let alice = scopes::ensure_principal_scope(&mut tx, tenant_id, "alice", "Alice")
        .await
        .expect("mint alice's scope");
    identities::create(
        &mut tx,
        IdentityId::new(),
        tenant_id,
        Some("alice"),
        IdentityKind::User,
        None,
        None,
        alice.id,
    )
    .await
    .expect("create alice's identity");
    tx.commit().await.expect("commit fixture");

    // The mover governs team-b while it lives under dept-x.
    let (status, renamed) = api(
        &app,
        "PATCH",
        &format!("/v1/admin/scopes/{}", team_b.id),
        &steward,
        Some(json!({"display_name": "Payments"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the mover renames a scope in their subtree: {renamed}"
    );

    // The mover cannot move it *out*: a move is decided at both ends
    // (ADR-0074 decision 5), and dept-y is not in their subtree. Holding
    // one end of a reorganisation is exactly half the authority it needs.
    let (status, refused) = api(
        &app,
        "PATCH",
        &format!("/v1/admin/scopes/{}", team_b.id),
        &steward,
        Some(json!({"parent_scope_id": dept_y.id})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "one end of a move is half the authority: {refused}"
    );

    // The tenant admin holds both ends, so the move lands — and that very
    // commit takes team-b out of the steward's granted subtree.
    let (status, moved) = api(
        &app,
        "PATCH",
        &format!("/v1/admin/scopes/{}", team_b.id),
        &admin,
        Some(json!({"parent_scope_id": dept_y.id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the admin moves it out: {moved}");

    // The same request that succeeded a moment ago is denied on the
    // very next call: the decision rides the post-move tree — no
    // refresh interval, no eventual consistency.
    let (status, denied) = api(
        &app,
        "PATCH",
        &format!("/v1/admin/scopes/{}", team_b.id),
        &steward,
        Some(json!({"display_name": "Payments Platform"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the mover's authority must not survive the move: {denied}"
    );

    // The composition seam (`standard`'s content-role rule): alice's
    // grant is at dept-x; team-b now lives under dept-y, so her read of
    // it denies — warm the decision twice so the repeat serves prebuilt
    // fragments.
    let member = Principal {
        tenant_id,
        subject: "alice".to_owned(),
        quarantined: false,
        scope_id: Some(alice.id),
        token_scope: None,
    };
    assert!(!member_reads(&state, tenant_id, &member, team_b.id, dept_x.id).await);
    assert!(!member_reads(&state, tenant_id, &member, team_b.id, dept_x.id).await);

    // The admin brings team-b home; the handler's unified seam flushes
    // the entity fragments post-commit (ADR-0017 decision 5), and the
    // very next decision sees the scope back inside alice's grant.
    let (status, back) = api(
        &app,
        "PATCH",
        &format!("/v1/admin/scopes/{}", team_b.id),
        &admin,
        Some(json!({"parent_scope_id": dept_x.id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin moves it back: {back}");
    assert!(
        member_reads(&state, tenant_id, &member, team_b.id, dept_x.id).await,
        "the granted read must follow the scope on the very next decision"
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
