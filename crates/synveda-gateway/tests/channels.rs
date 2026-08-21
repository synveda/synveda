//! FLOW-2 acceptance criterion (ADR-0031): **the "bank mode" switch
//! (published-only) flips composition instantly** — over real channel
//! refs, not the `RecordKind` stand-in ADR-0025 shipped and recorded a
//! reversal trigger against.
//!
//! The AC test walks the whole feature end to end on the product
//! surfaces: extracted memories reach `memory/derived` through the
//! pipeline, a curator publishes one of them onto `memory/published`
//! through `POST /v1/channels/{scope}/publish` under the PDP, an inject
//! composes both channels with the published one unmarked, the pack
//! flips to `published-only`, and the very next inject — same token,
//! same session, no restart — composes the published record alone.
//!
//! Around it: the PDP gate (a contributor is refused, a curator is
//! not), the read requirement that keeps a curator out of a teammate's
//! personal scope, same-scope-only publication, publication binding
//! bytes rather than ids, the `vedaflow.channel.published` audit event,
//! and `GET /v1/channels`.
//!
//! Tests need a live Postgres: they read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database), the house convention.

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
use synveda_audit::StoredEvent;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder};
use synveda_policy::Pdp;
use synveda_retrieval::index::SearchIndex;
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::{access, identities, policy_assignments, scopes, tenants};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{
    Channel, CompositionConfig, GrantId, Identity, IdentityId, IdentityKind, InjectChannels,
    PackConfig, RecordClass, RecordId, RecordKind, ScopeId, Sensitivity, TenantId, TenantStatus,
};
use tower::ServiceExt;

const SECRET: &[u8] = b"flow-2-test-secret";

/// What the curator publishes: the team's canonical procedure.
const PROCEDURE: &str = "deploys go out on tuesdays after the release review";
/// What stays on the derived channel: unreviewed pipeline output.
const UNREVIEWED: &str = "someone mentioned deploying on a friday once";

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn index_root() -> std::path::PathBuf {
    std::env::temp_dir()
        .join("synveda-flow2-tests")
        .join(TenantId::new().to_string())
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
        pdp: Arc::new(Pdp::new().expect("build the embedded PDP")),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index: Arc::new(SearchIndex::open(index_root()).expect("open sidecar")),
        embedder: Arc::new(AnyEmbedder::Deterministic(DeterministicEmbedder::new())),
        inject_embed_timeout: Duration::from_millis(100),
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

fn issue(subject: &str, tenant_id: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue(subject, tenant_id, Duration::from_secs(300))
}

async fn admitted_tenant() -> Option<(PgPool, TenantId)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping FLOW-2 channel test: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
            return None;
        }
    };
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");
    let id = TenantId::new();
    let slug = format!("flow2-{}", id.as_uuid().simple());
    tenants::create(&pool, id, &slug, "FLOW-2 test tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((pool, id))
}

fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("checked by admitted_tenant")
}

/// root → an org unit → two workspaces. The second exists so the
/// same-scope rule has somewhere to be violated.
///
/// The two leaves are **`workspace`-shaped**, which is the LOCAL cell of
/// the approval matrix — one curator publishes directly there, which is
/// what the old `team` rank meant and what these tests are about. An
/// `org_unit` is SHARED and costs a curator *and* an administrator; the
/// suite that asserts *that* price is `context_packs`.
async fn unit(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    parent: ScopeId,
    slug: &str,
    display: &str,
    kind: ScopeKind,
) -> Scope {
    scopes::create(
        tx,
        &scopes::NewScope {
            id: ScopeId::new(),
            tenant_id: tenant,
            kind,
            parent_scope_id: Some(parent),
            slug: slug.to_owned(),
            display_name: display.to_owned(),
            attributes: serde_json::json!({}),
            principal_id: None,
            created_by: None,
        },
    )
    .await
    .expect("create org unit")
}

async fn seed_scopes(pool: &PgPool, tenant: TenantId) -> (Scope, Scope) {
    let mut tx = pool.begin().await.expect("begin");
    let root = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("mint root");
    let eng = unit(
        &mut tx,
        tenant,
        root.id,
        "eng",
        "Engineering",
        ScopeKind::OrgUnit,
    )
    .await;
    let platform = unit(
        &mut tx,
        tenant,
        eng.id,
        "platform",
        "Platform",
        ScopeKind::Workspace,
    )
    .await;
    let payments = unit(
        &mut tx,
        tenant,
        eng.id,
        "payments",
        "Payments",
        ScopeKind::Workspace,
    )
    .await;
    tx.commit().await.expect("commit scopes");
    (platform, payments)
}

/// A person: their own principal scope under the tenant root, carrying
/// the identity row (CPR-7, ADR-0074 decision 3).
async fn seed_user(pool: &PgPool, tenant: TenantId, subject: &str) -> Identity {
    let mut tx = pool.begin().await.expect("begin");
    let own = scopes::ensure_principal_scope(&mut tx, tenant, subject, subject)
        .await
        .expect("mint principal scope");
    let identity = identities::create(
        &mut tx,
        IdentityId::new(),
        tenant,
        Some(subject),
        IdentityKind::User,
        None,
        None,
        own.id,
    )
    .await
    .expect("create identity");
    tx.commit().await.expect("commit user");
    identity
}

async fn bind(pool: &PgPool, tenant: TenantId, subject: &str, scope: ScopeId, role: RoleKey) {
    let mut tx = pool.begin().await.expect("begin");
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
    tx.commit().await.expect("commit grant");
}

async fn seed_record(
    pool: &PgPool,
    tenant: TenantId,
    scope: ScopeId,
    owner: IdentityId,
    kind: RecordKind,
    content: &str,
) -> RecordId {
    let id = RecordId::new();
    records::insert(
        pool,
        id,
        tenant,
        &RecordState {
            scope_id: scope,
            owner_id: owner,
            kind,
            class: RecordClass::Procedure,
            content: content.to_owned(),
            sensitivity: Sensitivity::Internal,
            provenance: json!({"source": "flow-2 acceptance test"}),
            valid_from: chrono::Utc::now() - chrono::Duration::hours(1),
            valid_to: None,
        },
        &RecordEmbedding {
            model: DeterministicEmbedder::MODEL.to_owned(),
            vector: vec![0.25; 16],
        },
    )
    .await
    .expect("insert record");
    id
}

async fn call(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(request).await.expect("route responds");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, body)
}

async fn publish(app: &Router, token: &str, scope: ScopeId, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(format!("/v1/channels/{scope}/publish"))
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build publish request");
    call(app, request).await
}

async fn list_channels(app: &Router, token: &str, scope: ScopeId) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(format!("/v1/channels/{scope}"))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("build list request");
    call(app, request).await
}

async fn inject(app: &Router, token: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri("/v1/inject")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"session_id": "flow-2"}).to_string()))
        .expect("build inject request");
    call(app, request).await
}

fn record_ids(body: &Value) -> Vec<String> {
    body["record_ids"]
        .as_array()
        .expect("record_ids array")
        .iter()
        .map(|id| id.as_str().expect("record id string").to_owned())
        .collect()
}

async fn events(pool: &PgPool, tenant: TenantId, action: &str) -> Vec<StoredEvent> {
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("tenant tx");
    let mut all = synveda_audit::tail(&mut tx, tenant, 200)
        .await
        .expect("read chain");
    all.reverse();
    all.into_iter()
        .filter(|event| event.action == action)
        .collect()
}

// ── The acceptance criterion ─────────────────────────────────────────────────

/// **AC: the bank-mode switch (published-only) flips composition
/// instantly** — over channel refs a curator actually moved.
#[tokio::test]
async fn bank_mode_flips_composition_over_real_channels() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (_platform, _) = seed_scopes(&pool, tenant).await;
    let alice = seed_user(&pool, tenant, "alice").await;
    // The material sits at alice's **own scope**, which is where the
    // pipeline puts every extracted memory (MEM-1, ADR-0020 decision 3)
    // and — since CPR-7 (ADR-0074 decision 3) — the one scope besides the
    // tenant root that a session with no grants composes from. She is a
    // curator there, because a principal scope inherits nothing and the
    // LOCAL cell of `regulated-strict` asks for one wherever the material
    // sits.
    bind(&pool, tenant, "alice", alice.scope_id, RoleKey::Curator).await;
    let procedure = seed_record(
        &pool,
        tenant,
        alice.scope_id,
        alice.id,
        RecordKind::Derived,
        PROCEDURE,
    )
    .await;
    let unreviewed = seed_record(
        &pool,
        tenant,
        alice.scope_id,
        alice.id,
        RecordKind::Derived,
        UNREVIEWED,
    )
    .await;

    let state = state(&database_url());
    let pdp = Arc::clone(&state.pdp);
    let app = router(state);
    let token = issue("alice", tenant);

    // Nothing is published yet: both records compose, both unreviewed.
    let (status, before) = inject(&app, &token).await;
    assert_eq!(status, StatusCode::OK);
    let ids = record_ids(&before);
    assert!(ids.contains(&procedure.to_string()) && ids.contains(&unreviewed.to_string()));
    assert_eq!(
        before["text"]
            .as_str()
            .expect("text")
            .matches("[unreviewed]")
            .count(),
        2,
        "nothing is trusted before anyone publishes it: {}",
        before["text"]
    );

    // The curator publishes one of them — the governed route, under the
    // PDP, on the scope the material lives at.
    let (status, published) = publish(
        &app,
        &token,
        alice.scope_id,
        json!({"record_ids": [procedure], "message": "reviewed at the release meeting"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "publish: {published}");
    assert_eq!(published["channel"], json!("memory/published"));
    assert_eq!(published["members"], json!(1));
    assert_eq!(published["added"], json!(1));
    assert!(published["parent"].is_null(), "the channel's first commit");
    let commit = published["commit"]
        .as_str()
        .expect("commit hash")
        .to_owned();
    assert_eq!(commit.len(), 64);

    // The next inject composes it as reviewed material — no marker —
    // while the unpublished record still says it is unreviewed.
    let (status, after) = inject(&app, &token).await;
    assert_eq!(status, StatusCode::OK);
    let text = after["text"].as_str().expect("text");
    assert!(
        text.contains(&format!("- [procedure] {PROCEDURE}\n")),
        "published material renders unmarked: {text}"
    );
    assert!(
        text.contains(&format!("{UNREVIEWED} [unreviewed]")),
        "the rest is still unreviewed: {text}"
    );

    // The flip: a published-only pack as the tenant default. Nothing
    // else changes — same token, same session, no restart.
    pdp.install_source(
        tenant,
        "bank",
        1,
        "permit (principal, action, resource) when { resource in principal.tenant };",
        PackConfig {
            composition: Some(CompositionConfig {
                budget_tokens: CompositionConfig::DEFAULT.budget_tokens,
                channels: InjectChannels::PublishedOnly,
                ..CompositionConfig::DEFAULT
            }),
            ..Default::default()
        },
    )
    .expect("install bank pack");
    policy_assignments::set_default(&pool, tenant, "bank")
        .await
        .expect("set default pack");

    // The very next inject composes the published channel alone.
    let (status, banked) = inject(&app, &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        record_ids(&banked),
        vec![procedure.to_string()],
        "only what a curator published survives bank mode"
    );
    let text = banked["text"].as_str().expect("text");
    assert!(
        !text.contains("[unreviewed]"),
        "and nothing in the block is unreviewed: {text}"
    );

    // The block cites the commit the curator made.
    let injected = events(&pool, tenant, "context.injected").await;
    let last = injected.last().expect("an inject event");
    let channels = last.payload["channels"].as_array().expect("channels");
    assert!(
        channels
            .iter()
            .any(|channel| channel["commit"] == json!(commit)
                && channel["scope_id"] == json!(alice.scope_id)),
        "the block cites the published commit: {channels:?}"
    );
    let entries = last.payload["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["channel"], json!("published"));
}

// ── The governed surface ─────────────────────────────────────────────────────

/// Publishing is a curator's act (seed §5). A contributor — who may
/// *write* memories at the same scope — cannot declare them reviewed,
/// and the refusal is a denial, not a silent no-op.
#[tokio::test]
async fn publishing_requires_a_curator_not_merely_a_writer() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (platform, _) = seed_scopes(&pool, tenant).await;
    let bob = seed_user(&pool, tenant, "bob").await;
    bind(&pool, tenant, "bob", platform.id, RoleKey::Member).await;
    let record = seed_record(
        &pool,
        tenant,
        platform.id,
        bob.id,
        RecordKind::Derived,
        PROCEDURE,
    )
    .await;

    let app = router(state(&database_url()));
    let token = issue("bob", tenant);
    let (status, body) = publish(
        &app,
        &token,
        platform.id,
        json!({"record_ids": [record], "message": "self-service trust"}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "denied: {body}");

    // Nothing moved: the channel does not exist, so nothing composes as
    // published either.
    let (status, listed) = list_channels(&app, &token, platform.id).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "and a contributor cannot read the channel plane either: {listed}"
    );

    // Promote bob and the very same call succeeds — the difference is
    // the role, not the request.
    bind(&pool, tenant, "bob", platform.id, RoleKey::Curator).await;
    let (status, body) = publish(
        &app,
        &token,
        platform.id,
        json!({"record_ids": [record], "message": "reviewed"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "curator publishes: {body}");
}

/// Nobody publishes material they cannot read (ADR-0031 decision 12).
/// A curator at a team holds `ChannelPublish` on a teammate's personal
/// leaf — the binding is on that leaf's chain — but the privacy floor
/// denies them `MemoryRead` there, so the publish is refused. This is
/// the rule that keeps the user→team climb inside FLOW-3's proposal
/// rather than letting a curator reach into a personal scope.
#[tokio::test]
async fn a_curator_cannot_publish_a_personal_scope_they_cannot_read() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (platform, _) = seed_scopes(&pool, tenant).await;
    let alice = seed_user(&pool, tenant, "alice").await;
    let carol = seed_user(&pool, tenant, "carol").await;
    // Both are curators at the same team. Alice is additionally a curator
    // **at her own scope**, which is where her memory lives: a principal
    // scope inherits nothing (CPR-5, ADR-0072), so the team grant reaches
    // the team's material and not hers. Under `regulated-strict` the LOCAL
    // cell asks for a curator wherever the material sits, so this is the
    // grant that lets her stand behind her own memory — and carol, who has
    // no grant there, is refused by the privacy floor before the matrix is
    // even consulted.
    bind(&pool, tenant, "alice", platform.id, RoleKey::Curator).await;
    bind(&pool, tenant, "carol", platform.id, RoleKey::Curator).await;
    bind(&pool, tenant, "alice", alice.scope_id, RoleKey::Curator).await;
    // The record lives at alice's personal scope — where the pipeline
    // puts every extracted memory (MEM-1, ADR-0020 decision 3).
    let record = seed_record(
        &pool,
        tenant,
        alice.scope_id,
        alice.id,
        RecordKind::Derived,
        PROCEDURE,
    )
    .await;

    let app = router(state(&database_url()));
    let body = json!({"record_ids": [record], "message": "reviewed"});

    let (status, refused) =
        publish(&app, &issue("carol", tenant), alice.scope_id, body.clone()).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a teammate's personal scope stays private: {refused}"
    );

    // Alice's own memory, on her own channel, is hers to stand behind:
    // the membership floor grants her the read the privacy floor denies
    // carol.
    let (status, own) = publish(&app, &issue("alice", tenant), alice.scope_id, body).await;
    assert_eq!(status, StatusCode::OK, "her own scope, her own call: {own}");
    assert_eq!(own["added"], json!(1));
    assert_ne!(carol.scope_id, alice.scope_id, "distinct personal leaves");
}

/// Publication is same-scope: a record living at another team is not
/// this scope's to declare reviewed. Climbing scopes is FLOW-5, with the
/// target scope's approvers — so this refuses rather than half-publishes.
#[tokio::test]
async fn publishing_another_scopes_record_is_refused_whole() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (platform, payments) = seed_scopes(&pool, tenant).await;
    let alice = seed_user(&pool, tenant, "alice").await;
    bind(&pool, tenant, "alice", platform.id, RoleKey::Curator).await;
    bind(&pool, tenant, "alice", payments.id, RoleKey::Curator).await;
    let mine = seed_record(
        &pool,
        tenant,
        platform.id,
        alice.id,
        RecordKind::Derived,
        PROCEDURE,
    )
    .await;
    let theirs = seed_record(
        &pool,
        tenant,
        payments.id,
        alice.id,
        RecordKind::Derived,
        UNREVIEWED,
    )
    .await;

    let app = router(state(&database_url()));
    let token = issue("alice", tenant);
    let (status, body) = publish(
        &app,
        &token,
        platform.id,
        json!({"record_ids": [mine, theirs], "message": "both please"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "refused: {body}");
    assert!(
        body["message"].to_string().contains(&theirs.to_string()),
        "and says which record: {body}"
    );
    assert!(
        !body["message"].to_string().contains(&mine.to_string()),
        "naming only what was refused: {body}"
    );

    // Nothing partial landed: the platform channel was never created.
    let (status, listed) = list_channels(&app, &token, platform.id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        listed["channels"].as_array().expect("channels").len(),
        0,
        "a refused publish leaves no channel behind: {listed}"
    );
}

/// Publication binds bytes, not ids (ADR-0031 decision 5): the tree
/// entry names the address of the version that was reviewed, and the
/// audit event records it.
#[tokio::test]
async fn publication_records_the_address_it_reviewed() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (platform, _) = seed_scopes(&pool, tenant).await;
    let alice = seed_user(&pool, tenant, "alice").await;
    bind(&pool, tenant, "alice", platform.id, RoleKey::Curator).await;
    let record = seed_record(
        &pool,
        tenant,
        platform.id,
        alice.id,
        RecordKind::Derived,
        PROCEDURE,
    )
    .await;

    let app = router(state(&database_url()));
    let token = issue("alice", tenant);
    let (status, body) = publish(
        &app,
        &token,
        platform.id,
        json!({"record_ids": [record], "message": "reviewed at the release meeting"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "publish: {body}");
    let address = body["published"][0]["object_hash"]
        .as_str()
        .expect("object hash")
        .to_owned();

    // The audit event carries who, what, where from, where to, and the
    // pack — and no record content.
    let published = events(&pool, tenant, "vedaflow.channel.published").await;
    assert_eq!(published.len(), 1, "one event per publication");
    let event = &published[0];
    assert_eq!(event.actor_subject, "alice");
    assert_eq!(event.outcome, "success");
    assert_eq!(event.payload["channel"], json!("memory/published"));
    assert_eq!(event.payload["asset"], json!("memory"));
    assert_eq!(event.payload["added"], json!(1));
    assert_eq!(event.payload["records"][0]["object_hash"], json!(address));
    assert!(event.payload["parent"].is_null());
    assert!(
        event.payload["authz"]["pack"]
            .as_str()
            .is_some_and(|pack| pack.contains('@')),
        "the pack that governed the act: {}",
        event.payload
    );
    assert!(
        !event.payload.to_string().contains(PROCEDURE),
        "ids and addresses, never content: {}",
        event.payload
    );

    // Republishing the same content is idempotent in the tree, and
    // still an act somebody took — a second commit, `added: 0`.
    let (status, again) = publish(
        &app,
        &token,
        platform.id,
        json!({"record_ids": [record], "message": "re-confirmed"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "republish: {again}");
    assert_eq!(again["added"], json!(0), "nothing new was admitted");
    assert_eq!(again["members"], json!(1), "the set is unchanged");
    assert_eq!(
        again["parent"],
        json!(body["commit"]),
        "and it fast-forwards from the first publication"
    );
    assert_eq!(
        events(&pool, tenant, "vedaflow.channel.published")
            .await
            .len(),
        2,
        "the act audits even when it changed nothing"
    );
}

/// `GET /v1/channels/{scope}` shows the standing channels. Refs
/// materialise on first write, so a scope nobody has committed to
/// answers 200 with an empty list — not a 404 (ADR-0031 decision 2).
#[tokio::test]
async fn channel_listing_shows_refs_that_exist_and_nothing_else() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (platform, _) = seed_scopes(&pool, tenant).await;
    let alice = seed_user(&pool, tenant, "alice").await;
    bind(&pool, tenant, "alice", platform.id, RoleKey::Curator).await;
    let record = seed_record(
        &pool,
        tenant,
        platform.id,
        alice.id,
        RecordKind::Derived,
        PROCEDURE,
    )
    .await;

    let app = router(state(&database_url()));
    let token = issue("alice", tenant);

    let (status, empty) = list_channels(&app, &token, platform.id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(empty["channels"].as_array().expect("channels").len(), 0);

    publish(
        &app,
        &token,
        platform.id,
        json!({"record_ids": [record], "message": "reviewed"}),
    )
    .await;

    let (status, listed) = list_channels(&app, &token, platform.id).await;
    assert_eq!(status, StatusCode::OK);
    let channels = listed["channels"].as_array().expect("channels");
    assert_eq!(channels.len(), 1, "only what exists: {listed}");
    assert_eq!(channels[0]["name"], json!("memory/published"));
    assert_eq!(channels[0]["channel"], json!(Channel::Published));
    assert_eq!(channels[0]["entries"], json!(1));
    assert_eq!(channels[0]["updated_by"], json!(alice.id));
    // `staged` has no writer until FLOW-3, so it is genuinely absent
    // rather than manufactured empty.
    assert!(
        !listed.to_string().contains("memory/staged"),
        "no channel is conjured before something writes it: {listed}"
    );
}
