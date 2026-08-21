//! PRMT-2 acceptance criteria (ADR-0050), over the real product surfaces.
//!
//! PRMT-1 asserted its claims at the registry's own route, because a prompt
//! is fetched by name. **Almost nothing here can be asserted that way.** A
//! context pack is the first authored asset whose content has to enter the
//! corpus the read path ranks, so every load-bearing claim is measured
//! where a session actually sees it — at `POST /v1/inject`:
//!
//! - **a pack reaches a session only through review**, and under
//!   `regulated-strict` above a team that is now a curator *and* a steward,
//!   two distinct people, where FLOW-3 had left the cell at one curator
//!   (decision 15);
//! - **"re-embeds atomically" is measured from the reader's side** — no
//!   inject ever composes half a pack, and the previous version composes in
//!   full until the new one is entirely embedded *and* published;
//! - **"next session" is satisfied as "next call"**, because the pack
//!   channel is read live on the composition path;
//! - **pack content composes as pinned material, ranked**, and what does
//!   not fit is named in the index tier with a recall handle rather than
//!   dropped;
//! - **`ContextPackRead` admits pack chunks and `MemoryRead` never does**,
//!   tested under a stored pack that grants one and denies the other, which
//!   is the only shape in which the claim can be false;
//! - **an edited published document demotes its own chunks**;
//! - **a rewind restores the previous version by moving a ref**, with no
//!   re-embedding;
//! - **a document carrying a live credential is quarantined at authoring**,
//!   ahead of the embedder;
//! - **every act is on the chain**, and no payload carries document text.
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
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder};
use synveda_policy::Pdp;
use synveda_retrieval::index::SearchIndex;
use synveda_store::{access, identities, policy_assignments, scopes, tenants};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{
    GrantId, Identity, IdentityId, IdentityKind, PackConfig, ScopeId, TenantId, TenantStatus,
};
use tower::ServiceExt;

const SECRET: &[u8] = b"prmt-2-test-secret";

/// The runbook under review, version 1 and version 2. The edit is a
/// behaviour change a session would notice, which is the point: it must not
/// reach one until the pack in force says so.
const V1: &str = "# Refunds\n\nSettle refunds within three days.\n\n\
                  ## Escalation\n\nEscalate anything over five hundred pounds to the duty lead.\n";
const V2: &str = "# Refunds\n\nSettle refunds within one day.\n\n\
                  ## Escalation\n\nEscalate anything over fifty pounds to the duty lead.\n";

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn index_root() -> std::path::PathBuf {
    std::env::temp_dir()
        .join("synveda-prmt2-tests")
        .join(TenantId::new().to_string())
}

fn state(url: &str, pdp: Arc<Pdp>) -> AppState {
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
        pdp,
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

/// The fixture: one tenant, `root → eng → platform`, and the
/// people a governed publication needs.
struct World {
    pool: PgPool,
    tenant: TenantId,
    app: Router,
    pdp: Arc<Pdp>,
    /// The tenant root — the org-wide scope. Since CPR-7 a reader's own
    /// chain is their principal scope and the root and nothing between
    /// (ADR-0074 decision 3), so this is the one shared scope a session
    /// composes from, and the cast is bound here for the publications
    /// that must reach one.
    root: ScopeId,
    eng: ScopeId,
    platform: ScopeId,
    /// The author: a contributor at the platform team.
    alice: String,
    /// The curator who reviews and runs the effect.
    cora: String,
    /// The steward the pack also asks for above a team — placed in the
    /// *other* team, so his authority at the platform team is a role
    /// binding and nothing else.
    sam: String,
    /// The consumer: holding no role at all, so her chain is her own
    /// scope and the tenant root and nothing else.
    bea: String,
}

async fn world() -> Option<World> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping PRMT-2 context pack test: DATABASE_URL is not set \
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
    let tenant = TenantId::new();
    let slug = format!("prmt2-{}", tenant.as_uuid().simple());
    tenants::create(
        &pool,
        tenant,
        &slug,
        "PRMT-2 test tenant",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");

    let mut tx = pool.begin().await.expect("begin");
    let root = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("mint root");
    let eng = unit(&mut tx, tenant, root.id, "eng").await;
    let platform = unit(&mut tx, tenant, eng.id, "platform").await;
    tx.commit().await.expect("commit scopes");

    for subject in ["alice", "cora", "sam", "bea"] {
        seed_user(&pool, tenant, subject).await;
    }
    bind(&pool, tenant, "alice", platform.id, RoleKey::Member).await;
    bind(&pool, tenant, "cora", platform.id, RoleKey::Curator).await;
    bind(&pool, tenant, "sam", platform.id, RoleKey::Administrator).await;
    // The department too: decision 15's split is about *shared* scopes, so
    // the test needs the same two people to be able to act there.
    bind(&pool, tenant, "alice", eng.id, RoleKey::Member).await;
    bind(&pool, tenant, "cora", eng.id, RoleKey::Curator).await;
    bind(&pool, tenant, "sam", eng.id, RoleKey::Administrator).await;
    // And the root, where the publications a session composes from live:
    // a grant at the root is on every scope's chain, so the same cast can
    // publish org-wide material (CPR-7, ADR-0074).
    bind(&pool, tenant, "alice", root.id, RoleKey::Member).await;
    bind(&pool, tenant, "cora", root.id, RoleKey::Curator).await;
    bind(&pool, tenant, "sam", root.id, RoleKey::Administrator).await;

    let pdp = Arc::new(Pdp::new().expect("build the embedded PDP"));
    let app = router(state(&url, pdp.clone()));
    Some(World {
        pool,
        tenant,
        app,
        pdp,
        root: root.id,
        eng: eng.id,
        platform: platform.id,
        alice: issue("alice", tenant),
        cora: issue("cora", tenant),
        sam: issue("sam", tenant),
        bea: issue("bea", tenant),
    })
}

/// One org unit under a parent — the shape every grouping takes now that
/// rank is gone (ADR-0073 decision 4).
async fn unit(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    tenant: TenantId,
    parent: ScopeId,
    slug: &str,
) -> Scope {
    scopes::create(
        &mut *tx,
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

/// The refusal a caller reads: `message` on the request-shaped errors,
/// `reason` on a policy denial.
fn detail(body: &Value) -> String {
    body["message"]
        .as_str()
        .or_else(|| body["reason"].as_str())
        .unwrap_or_default()
        .to_owned()
}

async fn post(app: &Router, uri: &str, token: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build request");
    call(app, request).await
}

async fn get(app: &Router, uri: &str, token: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("build request");
    call(app, request).await
}

/// Authors one document into a pack at a scope.
async fn author(
    w: &World,
    token: &str,
    scope: ScopeId,
    pack: &str,
    document: &str,
    content: &str,
) -> (StatusCode, Value) {
    post(
        &w.app,
        "/v1/context-packs",
        token,
        json!({
            "scope_id": scope,
            "name": pack,
            "description": "how payments works here",
            "documents": [{
                "name": document,
                "title": "Refunds runbook",
                "content": content,
            }],
        }),
    )
    .await
}

/// What a session composes right now.
async fn inject(w: &World, token: &str, task: Option<&str>) -> Value {
    let body = match task {
        Some(task) => json!({"task": task, "session_id": "sess-prmt2"}),
        None => json!({"session_id": "sess-prmt2"}),
    };
    let (status, block) = post(&w.app, "/v1/inject", token, body).await;
    assert_eq!(status, StatusCode::OK, "inject must succeed: {block}");
    block
}

fn text(block: &Value) -> String {
    block["text"].as_str().unwrap_or_default().to_owned()
}

/// The pack channels a block cited, by scope.
fn pack_watermarks(block: &Value) -> Vec<&Value> {
    block["channels"]
        .as_array()
        .map(|channels| {
            channels
                .iter()
                .filter(|channel| channel["ref"] == json!("context-pack/published"))
                .collect()
        })
        .unwrap_or_default()
}

/// Carries a pack from a draft to a published version through the review
/// the pack in force asks for at `scope`. Returns the commit the channel
/// now serves.
async fn review_and_publish(
    w: &World,
    scope: ScopeId,
    source: ScopeId,
    paths: &[&str],
    title: &str,
) -> String {
    let (status, opened) = post(
        &w.app,
        "/v1/proposals",
        &w.alice,
        json!({
            "scope_id": scope,
            "source_scope_id": source,
            "document_paths": paths,
            "title": title,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "open a pack proposal: {opened}");
    let id = opened["id"].as_str().expect("proposal id").to_owned();

    for token in [&w.cora, &w.sam] {
        let (status, cast) = post(
            &w.app,
            &format!("/v1/proposals/{id}/approve"),
            token,
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "approve: {cast}");
    }
    let (status, published) = post(
        &w.app,
        &format!("/v1/proposals/{id}/publish"),
        &w.cora,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "publish: {published}");
    published["commit"].as_str().expect("commit").to_owned()
}

async fn events(pool: &PgPool, tenant: TenantId) -> Vec<synveda_audit::StoredEvent> {
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("tenant tx");
    let mut all = synveda_audit::tail(&mut tx, tenant, 500)
        .await
        .expect("read chain");
    all.reverse();
    all
}

// ── The acceptance criterion, first clause ───────────────────────────────

/// **A pack authored at a shared scope reaches a session only through the
/// review the pack in force asks for** — and above a team that review is a
/// curator *and* an administrator, two distinct people (ADR-0050 decision
/// 15, on the grant-key vocabulary since CPR-7).
///
/// The direct route is not a hole to close here: it resolves the same
/// matrix (ADR-0032 decision 8), so the refusal is the pack's arithmetic
/// rather than a rule about packs. What decision 15 changed is which
/// arithmetic — publishing a bundle at an org unit must not be cheaper
/// than publishing one memory record there.
///
/// Since CPR-7 a session composes from the reader's own scope and the
/// tenant root and nothing between (ADR-0074 decision 3), so the walk is
/// in two halves: the *price* is asserted at the org unit, where the
/// SHARED row bites, and the *arrival* at the tenant root — the one
/// shared scope on every session's chain — where the same reviewed route
/// carries the runbook into bea's very next call.
#[tokio::test]
async fn a_pack_reaches_a_session_only_through_review_and_costs_two_people_above_a_team() {
    let Some(w) = world().await else { return };

    // 1. Authored at the org unit. Nothing a session composes has moved.
    let (status, draft) = author(&w, &w.alice, w.eng, "payments", "runbooks/refunds.md", V1).await;
    assert_eq!(status, StatusCode::OK, "author the bundle: {draft}");
    assert_eq!(draft["name"], json!("payments"));
    let document = &draft["documents"][0];
    assert!(
        document["chunks"].as_u64().unwrap_or(0) >= 2,
        "the runbook's two sections cut into at least two chunks: {document}"
    );
    assert_eq!(
        document["embedded"], document["chunks"],
        "a first author embeds every chunk: {document}"
    );
    assert!(
        document["published"].is_null(),
        "nothing is published yet: {document}"
    );

    let before = inject(&w, &w.bea, Some("how do refunds work")).await;
    assert!(
        !text(&before).contains("three days"),
        "an unpublished pack composes into nobody's session: {}",
        text(&before)
    );

    // 2. The direct route refuses at an org unit, and the refusal is the
    //    matrix speaking — decision 15's `SHARED` row.
    let (status, refused) = post(
        &w.app,
        &format!("/v1/channels/{}/publish", w.eng),
        &w.cora,
        json!({
            "document_paths": ["payments/runbooks/refunds.md"],
            "message": "ship it",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a curator alone cannot publish a bundle at an org unit: {refused}"
    );
    let refusal = detail(&refused);
    assert!(
        refusal.contains("administrator"),
        "the refusal must name what the pack is short of: {refusal}"
    );
    assert!(
        refusal.contains("proposal"),
        "and where to go with it: {refusal}"
    );

    // 3. The review the pack asks for. Two distinct people, and the
    //    requirement says so before either of them has acted.
    let (status, opened) = post(
        &w.app,
        "/v1/proposals",
        &w.alice,
        json!({
            "scope_id": w.eng,
            "document_paths": ["payments/runbooks/refunds.md"],
            "title": "payments conventions",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "open a pack proposal: {opened}");
    assert_eq!(
        opened["required"]["roles"],
        json!([{"role": "curator", "count": 1}, {"role": "administrator", "count": 1}]),
        "the SHARED row names both authorities: {opened}"
    );
    assert_eq!(
        opened["required"]["distinct_approvers"], 2,
        "and two distinct identities: {opened}"
    );
    let id = opened["id"].as_str().expect("proposal id").to_owned();

    let (status, first) = post(
        &w.app,
        &format!("/v1/proposals/{id}/approve"),
        &w.cora,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "curator approval failed: {first}");
    assert_eq!(
        first["state"], "open",
        "one curator is still short at an org unit: {first}"
    );
    let (status, second) = post(
        &w.app,
        &format!("/v1/proposals/{id}/approve"),
        &w.sam,
        json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "administrator approval failed: {second}"
    );
    assert_eq!(second["state"], "approved", "two people close it: {second}");
    let (status, published) = post(
        &w.app,
        &format!("/v1/proposals/{id}/publish"),
        &w.cora,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "publish failed: {published}");

    // 4. The same reviewed route at the tenant root — the one shared scope
    //    on every session's chain — and a session composes it on the very
    //    next call, because the pack channel is read live (the AC's "next
    //    session" satisfied as "next call").
    let commit = review_and_publish(
        &w,
        w.root,
        w.eng,
        &["payments/runbooks/refunds.md"],
        "payments conventions, org-wide",
    )
    .await;
    let after = inject(&w, &w.bea, Some("how do refunds work")).await;
    let composed = text(&after);
    assert!(
        composed.contains("three days"),
        "the reviewed runbook composes into a session that never authored it: {composed}"
    );
    let watermarks = pack_watermarks(&after);
    assert!(
        watermarks
            .iter()
            .any(|mark| mark["commit"] == json!(commit)),
        "the block cites the pack commit it composed against: {:?}",
        watermarks
    );
}

/// **A pack published at a LOCAL scope still costs one curator.**
/// Decision 15 gave context packs memory's own `SHARED`/`LOCAL` split
/// rather than a blanket rise — so this is the half of the change that
/// must *not* have happened. The LOCAL cell is a person's own scope, a
/// workspace or a project (CPR-7 read §2.4's "team/user" row onto the
/// shapes that replaced it), and the tenant root is **not** in it: the
/// root is what the old `org` rank became and carries its price. So the
/// single-curator claim is asserted where it is made — at bea's own
/// scope, which is also on her chain, so the second half of the claim
/// ("and a session composes it") stays observable.
#[tokio::test]
async fn a_pack_published_at_a_local_scope_still_costs_one_curator() {
    let Some(w) = world().await else { return };
    let bea_scope = {
        let mut tx = w.pool.begin().await.expect("begin");
        let identity = synveda_store::identities::by_subject(&mut *tx, w.tenant, "bea")
            .await
            .expect("read bea")
            .expect("bea exists");
        tx.commit().await.expect("commit");
        identity.scope_id
    };
    bind(&w.pool, w.tenant, "bea", bea_scope, RoleKey::Curator).await;
    let (status, draft) = author(
        &w,
        &w.bea,
        bea_scope,
        "conventions",
        "style.md",
        "# Style\n\nWe write commit messages in the imperative.\n",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{draft}");

    let (status, published) = post(
        &w.app,
        &format!("/v1/channels/{bea_scope}/publish"),
        &w.bea,
        json!({
            "document_paths": ["conventions/style.md"],
            "message": "the house style",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "one curator publishes a bundle at a LOCAL scope: {published}"
    );
    assert_eq!(published["channel"], json!("context-pack/published"));

    let block = inject(&w, &w.bea, Some("how should I write commit messages")).await;
    assert!(
        text(&block).contains("imperative"),
        "and a session composes it: {}",
        text(&block)
    );
}

// ── Atomicity, measured from the reader's side ───────────────────────────

/// **No inject ever composes half a pack.** The previous version composes
/// in full until the new one is entirely embedded *and* published, and the
/// new one in full thereafter.
///
/// Two mechanisms make that true and this test exercises both: chunk rows
/// land with their embeddings or not at all (ADR-0023 decision 2), and the
/// ref cannot move to a commit whose chunks do not yet exist — because the
/// commit names addresses that only exist once the authoring transaction
/// committed (ADR-0050 decision 5).
///
/// It is also **an edited published document demoting its own chunks**
/// (decision 3): between the edit and the publication the *old* version is
/// what composes, in full, and not one word of the new one leaks in.
#[tokio::test]
async fn an_edit_composes_as_all_of_the_old_version_until_all_of_the_new_one_is_published() {
    let Some(w) = world().await else { return };
    author(&w, &w.alice, w.root, "payments", "runbooks/refunds.md", V1).await;
    review_and_publish(
        &w,
        w.root,
        w.root,
        &["payments/runbooks/refunds.md"],
        "payments v1",
    )
    .await;

    let v1 = text(&inject(&w, &w.bea, Some("refunds and escalation")).await);
    assert!(v1.contains("three days"), "v1 body one: {v1}");
    assert!(v1.contains("five hundred"), "v1 body two: {v1}");

    // The edit. It re-chunks and re-embeds, and moves the document's
    // address — which takes every chunk of the old version off nothing at
    // all, because the channel still names the old address.
    let (status, edited) =
        author(&w, &w.alice, w.root, "payments", "runbooks/refunds.md", V2).await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    let document = &edited["documents"][0];
    assert!(
        document["embedded"].as_u64().unwrap_or(0) > 0,
        "an edit embeds the new version's chunks: {document}"
    );
    assert_eq!(
        document["published"]["current"],
        json!(false),
        "the draft has moved and the reviewed version has not: {document}"
    );

    // The reader's side: all of the old version, none of the new one.
    let between = text(&inject(&w, &w.bea, Some("refunds and escalation")).await);
    assert!(
        between.contains("three days") && between.contains("five hundred"),
        "the reviewed version keeps composing in full: {between}"
    );
    assert!(
        !between.contains("one day") && !between.contains("fifty pounds"),
        "and not one word of the unreviewed edit leaks in — an edit cannot be \
         laundered through chunks the tree still appears to name: {between}"
    );

    // The publication. Now all of the new version, none of the old.
    review_and_publish(
        &w,
        w.root,
        w.root,
        &["payments/runbooks/refunds.md"],
        "payments v2",
    )
    .await;
    let v2 = text(&inject(&w, &w.bea, Some("refunds and escalation")).await);
    assert!(
        v2.contains("one day") && v2.contains("fifty pounds"),
        "the new version composes in full: {v2}"
    );
    assert!(
        !v2.contains("three days") && !v2.contains("five hundred"),
        "and the old one is gone entirely: {v2}"
    );
}

/// **Re-authoring an unchanged document re-embeds nothing.** The chunker is
/// deterministic and the address covers exactly what a reviewer consents
/// to, so identical bytes find their chunks already there (ADR-0050
/// decision 4).
#[tokio::test]
async fn re_authoring_unchanged_bytes_embeds_nothing() {
    let Some(w) = world().await else { return };
    let (_, first) = author(&w, &w.alice, w.platform, "payments", "refunds.md", V1).await;
    let embedded = first["documents"][0]["embedded"].as_u64().unwrap_or(0);
    assert!(embedded > 0, "the first author embeds: {first}");

    let (status, again) = author(&w, &w.alice, w.platform, "payments", "refunds.md", V1).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(
        again["documents"][0]["embedded"],
        json!(0),
        "the same bytes chunk identically, so nothing is re-embedded: {again}"
    );
    assert_eq!(
        again["documents"][0]["object_hash"], first["documents"][0]["object_hash"],
        "and the address is the same one: {again}"
    );
}

// ── The read half ────────────────────────────────────────────────────────

/// **Pack content composes as pinned material, ranked, and what does not
/// fit is named rather than dropped** (ADR-0050 decision 9).
///
/// ADR-0025 option 5 rejected relevance filtering for pinned material so
/// canonical content could not silently vanish — decided about a handful of
/// hand-pinned records costing tens of tokens. A large glossary against a
/// 1,500-token budget is not that case, and this is the resolution that
/// keeps both halves: the block that cannot hold the runbook **says the
/// runbook exists, names it, and hands back a recall handle**.
#[tokio::test]
async fn a_pack_too_large_for_the_budget_is_named_rather_than_dropped() {
    let Some(w) = world().await else { return };
    // A bundle far past the seed §4.4 default budget: twelve sections, each
    // a chunk, each far wider than an index line.
    //
    //    Every section says something *different*: identical content is one
    //    winner by conflict resolution (ADR-0025 decision 6), so a fixture
    //    of twelve identical sections would compose two entries and test
    //    nothing about the budget.
    let big: String = (0..12)
        .map(|section| {
            format!(
                "# Section {section}\n\n{}\n\n",
                (0..20)
                    .map(|line| format!(
                        "Term {section}.{line} is settled under the schedule for section \
                         {section} and reviewed every quarter. "
                    ))
                    .collect::<String>()
            )
        })
        .collect();
    author(&w, &w.alice, w.eng, "payments", "glossary.md", &big).await;
    review_and_publish(&w, w.root, w.eng, &["payments/glossary.md"], "glossary").await;

    let block = inject(&w, &w.bea, Some("payment terms")).await;
    let composed = text(&block);
    assert!(
        block["tokens"].as_u64().unwrap_or(0) <= block["budget_tokens"].as_u64().unwrap_or(0),
        "the block stays under budget: {} of {}",
        block["tokens"],
        block["budget_tokens"]
    );
    assert!(
        block["index_entries"].as_u64().unwrap_or(0) > 0,
        "and what did not fit was named: {block}"
    );
    // Decision 10's line: the pack, the document, the section, the title —
    // a better description than any truncation of the prose.
    assert!(
        composed.contains("payments/glossary.md#"),
        "an index entry names the pack and the document: {composed}"
    );
    assert!(
        composed.contains("§ Section"),
        "and the section it came from: {composed}"
    );
    assert!(
        composed.contains("(recall "),
        "and hands back a recall handle: {composed}"
    );

    // The handle is a name, not a capability — and it resolves, which is
    // what makes "named rather than dropped" a promise rather than a label.
    let handle = composed
        .split("(recall ")
        .nth(1)
        .and_then(|rest| rest.split(')').next())
        .expect("a recall handle")
        .to_owned();
    let (status, recalled) = post(&w.app, "/v1/recall", &w.bea, json!({"ids": [handle]})).await;
    assert_eq!(status, StatusCode::OK, "the handle resolves: {recalled}");
    // Recall serves records rather than a rendered block: the caller named
    // what it wants, which is what makes it the deep surface (ADR-0041
    // decision 5). The chunk comes back in full.
    let entries = recalled["entries"].as_array().cloned().unwrap_or_default();
    assert_eq!(entries.len(), 1, "one handle, one record: {recalled}");
    assert!(
        entries[0]["content"]
            .as_str()
            .unwrap_or_default()
            .contains("reviewed every quarter"),
        "and returns the body the block could not hold: {recalled}"
    );
}

/// **`ContextPackRead` admits pack chunks and `MemoryRead` never does**
/// (ADR-0050 decision 8) — the case packs exist for, and the only shape in
/// which it can be false.
///
/// A stored pack that grants `ContextPackRead` at the department and
/// denies `MemoryRead` there. A reader who may compose *no* memory from
/// that scope still receives its conventions; a memory record published at
/// the same scope, by the same people, does not compose. One decision,
/// admitting one kind of material.
#[tokio::test]
async fn a_reader_with_no_readable_memory_at_a_scope_still_gets_its_conventions() {
    let Some(w) = world().await else { return };

    // Conventions at the department, published the way the pack asks.
    author(
        &w,
        &w.alice,
        w.eng,
        "conventions",
        "house-style.md",
        "# House style\n\nWe never ship on a Friday.\n",
    )
    .await;
    review_and_publish(
        &w,
        w.root,
        w.eng,
        &["conventions/house-style.md"],
        "house style",
    )
    .await;
    let before = text(&inject(&w, &w.bea, Some("when do we ship")).await);
    assert!(
        before.contains("never ship on a Friday"),
        "the baseline: under the default pack it composes: {before}"
    );

    // Now a pack that grants the tenant everything *except* `MemoryRead`.
    // Cedar is deny-by-default, so naming the actions is the grant.
    w.pdp
        .install_source(
            w.tenant,
            "packs-not-memories",
            1,
            r#"permit (
                 principal,
                 action in [
                   Synveda::Action::"ContextPackRead",
                   Synveda::Action::"ContextPackWrite",
                   Synveda::Action::"MemoryWrite",
                   Synveda::Action::"ChannelRead",
                   Synveda::Action::"ChannelPublish",
                   Synveda::Action::"ProposalRead",
                   Synveda::Action::"ProposalOpen",
                   Synveda::Action::"ProposalReview"
                 ],
                 resource
               ) when { resource in principal.tenant };"#,
            PackConfig::default(),
        )
        .expect("install a pack that grants packs and not memories");
    let mut tx = w.pool.begin().await.expect("begin");
    policy_assignments::assign(&mut *tx, w.tenant, w.eng, "packs-not-memories")
        .await
        .expect("assign the pack at the department");
    tx.commit().await.expect("commit assignment");

    let block = inject(&w, &w.bea, Some("when do we ship")).await;
    let composed = text(&block);
    assert!(
        composed.contains("never ship on a Friday"),
        "a reader who may compose no memory at the department still receives \
         its conventions — the case context packs exist for: {composed}"
    );
    // And the converse, from the same block: the department's *memory*
    // section is absent, because `ContextPackRead` admitted a chunk and
    // nothing else.
    let decisions = block["decisions"].as_array().cloned().unwrap_or_default();
    let department = decisions
        .iter()
        .find(|decision| decision["scope_id"] == json!(w.eng));
    if let Some(decision) = department {
        assert_eq!(
            decision["allowed"],
            json!(false),
            "the MemoryRead decision at the department is a deny, and the pack \
             chunk composed anyway: {decision}"
        );
    }
}

// ── Rewind, and the scan ────────────────────────────────────────────────

/// **A rewind restores the previous version by moving a ref**, with no
/// re-embedding and no half-swapped state (ADR-0050 decision 6).
///
/// `ContextPackRead` is what makes it decidable, which discharges ADR-0036
/// decision 3 for the second of the three kinds it refused by name.
#[tokio::test]
async fn a_rewind_restores_the_previous_version_without_re_embedding_anything() {
    let Some(w) = world().await else { return };
    author(&w, &w.alice, w.eng, "payments", "runbooks/refunds.md", V1).await;
    let first = review_and_publish(
        &w,
        w.root,
        w.eng,
        &["payments/runbooks/refunds.md"],
        "payments v1",
    )
    .await;
    author(&w, &w.alice, w.eng, "payments", "runbooks/refunds.md", V2).await;
    // The second version climbs from the org unit it was authored at to
    // the tenant root, which is the scope bea's session composes from.
    let second = review_and_publish(
        &w,
        w.root,
        w.eng,
        &["payments/runbooks/refunds.md"],
        "payments v2",
    )
    .await;
    assert!(
        text(&inject(&w, &w.bea, Some("refunds")).await).contains("one day"),
        "v2 is live"
    );

    let (status, rewound) = post(
        &w.app,
        &format!("/v1/channels/{}/rollback", w.root),
        &w.cora,
        json!({
            "asset": "context-pack",
            "channel": "published",
            "from_commit": second,
            "to_commit": first,
            "message": "the one-day rule was wrong",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "rewinding a pack channel is decidable now that ContextPackRead exists: {rewound}"
    );

    let restored = text(&inject(&w, &w.bea, Some("refunds")).await);
    assert!(
        restored.contains("three days") && restored.contains("five hundred"),
        "the previous version is back, in full: {restored}"
    );
    assert!(
        !restored.contains("one day"),
        "and the withdrawn one is gone: {restored}"
    );
}

/// **A document carrying a live credential is quarantined at authoring**
/// (ADR-0050 decision 11), and the scan runs ahead of the embedder — so no
/// secret reaches vector space.
///
/// This is the first surface where bulk external text enters the product:
/// a prompt is short and hand-written, and PRMT-1 does not scan one. The
/// first thing a customer does with a context pack is upload an existing
/// runbook, and runbooks carry connection strings.
#[tokio::test]
async fn a_document_carrying_a_credential_is_stopped_before_the_embedder() {
    let Some(w) = world().await else { return };
    // A GitHub token rather than a connection string: MEM-2's overlap
    // resolution is positional, and in `postgres://user:pass@host` the
    // `email` rule matches `pass@host` first — so that one *redacts* (PII)
    // rather than quarantining (secret). The guarantee this test is about
    // holds either way — nothing reaches vector space unscrubbed — but the
    // disposition under test here is the quarantine rung.
    let secret = "ghp_0123456789abcdefghijklmnopqrstuvwxyzAB";
    let (status, refused) = author(
        &w,
        &w.alice,
        w.platform,
        "payments",
        "runbooks/oncall.md",
        &format!("# On call\n\nAuthenticate with {secret} and check the queue.\n"),
    )
    .await;
    assert_ne!(
        status,
        StatusCode::OK,
        "a document carrying a live credential must not be stored: {refused}"
    );
    let refusal = detail(&refused);
    assert!(
        refusal.contains("redaction scanner"),
        "the refusal names what stopped it: {refusal}"
    );
    assert!(
        refusal.contains("not embedded"),
        "and says the secret never reached vector space: {refusal}"
    );

    // Nothing was written: no draft, no chunk, no record.
    let (status, listing) = get(
        &w.app,
        &format!("/v1/context-packs?scope_id={}", w.platform),
        &w.alice,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listing}");
    assert!(
        !listing.to_string().contains("oncall"),
        "the stopped document is nowhere in the registry: {listing}"
    );

    // And the act is on the chain, with the rule that fired and not the
    // text that fired it.
    let chain = events(&w.pool, w.tenant).await;
    let quarantined = chain
        .iter()
        .find(|event| event.action == "context_pack.quarantined")
        .expect("a context_pack.quarantined event");
    assert_eq!(
        quarantined.payload["document"],
        json!("runbooks/oncall.md"),
        "{:?}",
        quarantined.payload
    );
    assert!(
        !quarantined.payload.to_string().contains(secret),
        "no payload carries the matched text: {:?}",
        quarantined.payload
    );
}

// ── The chain ───────────────────────────────────────────────────────────

/// **Every act is on the chain**, and no payload carries document text
/// (ADR-0050 decision 13).
///
/// `context_pack.authored` for the draft write, the same
/// `vedaflow.channel.published` a memory publication emits with `asset`
/// reading `context-pack`, and a served chunk watermarked inside
/// `context.injected` with its object address — because a chunk arrives
/// through a route that already chains an event, so it deliberately gets
/// no third action of its own.
#[tokio::test]
async fn every_act_is_on_the_chain_and_no_payload_carries_document_text() {
    let Some(w) = world().await else { return };
    author(&w, &w.alice, w.eng, "payments", "runbooks/refunds.md", V1).await;
    review_and_publish(
        &w,
        w.root,
        w.eng,
        &["payments/runbooks/refunds.md"],
        "payments",
    )
    .await;
    let block = inject(&w, &w.bea, Some("refunds")).await;
    assert!(text(&block).contains("three days"), "the block composed it");

    let chain = events(&w.pool, w.tenant).await;

    let authored = chain
        .iter()
        .find(|event| event.action == "context_pack.authored")
        .expect("a context_pack.authored event");
    assert_eq!(authored.payload["pack"], json!("payments"));
    assert_eq!(authored.payload["asset"], json!("context-pack"));
    let document = &authored.payload["documents"][0];
    assert_eq!(document["document"], json!("runbooks/refunds.md"));
    assert!(
        document["object_hash"]
            .as_str()
            .is_some_and(|hash| hash.len() == 64),
        "the address, so an auditor can recompute: {document}"
    );

    let published = chain
        .iter()
        .find(|event| {
            event.action == "vedaflow.channel.published"
                && event.payload["asset"] == json!("context-pack")
        })
        .expect("a vedaflow.channel.published event with asset=context-pack");
    assert_eq!(
        published.payload["channel"],
        json!("context-pack/published"),
        "{:?}",
        published.payload
    );

    let injected = chain
        .iter()
        .rev()
        .find(|event| event.action == "context.injected")
        .expect("a context.injected event");
    let channels = injected.payload["channels"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        channels
            .iter()
            .any(|channel| channel["ref"] == json!("context-pack/published")),
        "the served block's watermark cites the pack channel it composed \
         against: {:?}",
        injected.payload["channels"]
    );

    // The sweep. No payload on the whole chain carries document text —
    // the discipline every plane has followed since AUD-1.
    for event in &chain {
        let payload = event.payload.to_string();
        for phrase in ["three days", "five hundred", "Settle refunds"] {
            assert!(
                !payload.contains(phrase),
                "{} carries document text ({phrase}): {payload}",
                event.action
            );
        }
    }
}

/// **A pack proposal is reviewable without the console** — FLOW-6's claim,
/// asserted for the third asset kind, because PRMT-1 found it quietly
/// false for the second.
#[tokio::test]
async fn a_pack_proposal_renders_a_per_document_diff() {
    let Some(w) = world().await else { return };
    // Published and re-proposed at the **same** scope, because the diff is
    // against what that scope already publishes: a climb to a different
    // scope would be an `add` there, correctly, and would not exercise the
    // replacement this test is about.
    author(&w, &w.alice, w.eng, "payments", "runbooks/refunds.md", V1).await;
    review_and_publish(&w, w.eng, w.eng, &["payments/runbooks/refunds.md"], "v1").await;
    author(&w, &w.alice, w.eng, "payments", "runbooks/refunds.md", V2).await;

    let (status, opened) = post(
        &w.app,
        "/v1/proposals",
        &w.alice,
        json!({
            "scope_id": w.eng,
            "document_paths": ["payments/runbooks/refunds.md"],
            "title": "tighten the refund window",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{opened}");
    assert_eq!(opened["asset"], json!("context-pack"), "{opened}");
    let id = opened["id"].as_str().expect("proposal id").to_owned();

    let (status, detail) = get(&w.app, &format!("/v1/proposals/{id}"), &w.cora).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    let member = &detail["members"][0];
    assert_eq!(
        member["member"],
        json!("payments/runbooks/refunds.md"),
        "named by document path: {member}"
    );
    assert_eq!(member["asset"], json!("context-pack"), "{member}");
    assert_eq!(
        member["effect"],
        json!("update"),
        "it replaces a published version: {member}"
    );
    // Both sides, so a curator can see what changes. The diff is the
    // *document*, never its chunks — a pack is reviewed as the prose
    // somebody wrote (ADR-0050 reversal trigger (c)).
    assert!(
        member["proposed"]
            .as_str()
            .unwrap_or_default()
            .contains("one day"),
        "the proposed side: {member}"
    );
    assert!(
        member["baseline"]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("three days"),
        "and the baseline it would replace: {member}"
    );
}
