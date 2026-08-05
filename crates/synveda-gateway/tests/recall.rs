//! CTX-5 acceptance criteria (ADR-0042): "MCP client E2E; as-of returns
//! historically accurate context (`--as-of` demo)."
//!
//! The MCP half is a TypeScript surface and lives with its client
//! (`adapters/claude-code/src/mcp.test.mts`, plus the demo's live
//! round trip). This file is the gateway half, and it is arranged around
//! the two claims that would be worth lying about:
//!
//! **The universe genuinely widened.** The centrepiece is one corpus,
//! one identity, one pack — the *real* `standard` pack, whose department
//! permit has been unreachable since ADR-0024 fixed inject's universe at
//! the chain — asked twice. `POST /v1/inject` returns nothing of the
//! sibling team's, because it never asks; `POST /v1/recall` returns it,
//! because it does. Both are the same PDP answering the same question,
//! which is what makes this a widening rather than a hole.
//!
//! **As-of rewinds the corpus and nothing else.** A fact is stated, then
//! corrected; recall at the earlier instant returns what was true then and
//! recall now returns the correction. Then the same instant is asked by a
//! reader whose access has since been withdrawn, by a reader below a tier
//! the record has since been raised to, and against material a rollback
//! has since withdrawn — and none of the three gets anything back that
//! today's decision would refuse. History is a fact about the corpus; the
//! permission to read it is decided now, every time.
//!
//! Tests need a live Postgres: they read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database), the house convention.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{DateTime, Duration as Days, Utc};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::{Hs256Verifier, personal_slug};
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder, Embedder as _};
use synveda_policy::Pdp;
use synveda_retrieval::index::SearchIndex;
use synveda_retrieval::indexer::{self, IndexerConfig};
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::{hierarchy, identities, policy_assignments, role_bindings, tenants};
use synveda_types::{
    HierarchyNode, Identity, IdentityId, IdentityKind, RecordClass, RecordId, RecordKind, Role,
    ScopeId, ScopeKind, Sensitivity, TenantId, TenantStatus,
};
use tower::ServiceExt;

const SECRET: &[u8] = b"ctx-5-test-secret";

/// The sibling team's material. Distinctive enough that a lexical query
/// finds it and a wrong answer is obvious in a failure message.
const PAYMENTS_RUNBOOK: &str = "Payments settlement mismatch procedure: freeze the reconciliation job, compare the \
     ledger tail against the acquirer statement, then page the payments on-call.";

/// Alice's own note, so a widened recall can be shown to return *more*
/// rather than *different*.
const ALICE_NOTE: &str = "Alice prefers rebase-and-merge on platform feature branches.";

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn index_root() -> std::path::PathBuf {
    std::env::temp_dir()
        .join("synveda-ctx5-tests")
        .join(TenantId::new().to_string())
}

fn state_with(url: &str, search_index: Arc<SearchIndex>, pdp: Arc<Pdp>) -> AppState {
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
        scope_chains: Arc::new(synveda_store::ScopeChainCache::new()),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index,
        embedder: Arc::new(AnyEmbedder::Deterministic(DeterministicEmbedder::new())),
        inject_embed_timeout: Duration::from_millis(100),
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
                "skipping CTX-5 test: DATABASE_URL is not set \
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
    let slug = format!("ctx5-{}", id.as_uuid().simple());
    tenants::create(&pool, id, &slug, "CTX-5 test tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((pool, id))
}

fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("checked by admitted_tenant")
}

/// The shape the widened universe is *about*: two teams under one
/// department, so `standard`'s department permit has something to reach
/// that the reader's own chain does not contain.
struct Org {
    org: HierarchyNode,
    engineering: HierarchyNode,
    platform: HierarchyNode,
    payments: HierarchyNode,
}

async fn seed_org(pool: &PgPool, tenant: TenantId) -> Org {
    let mut tx = pool.begin().await.expect("begin");
    let org = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        None,
        ScopeKind::Org,
        "acme",
        "ACME",
    )
    .await
    .expect("create org");
    let engineering = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(org.id),
        ScopeKind::Department,
        "engineering",
        "Engineering",
    )
    .await
    .expect("create department");
    let platform = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(engineering.id),
        ScopeKind::Team,
        "platform",
        "Platform",
    )
    .await
    .expect("create platform");
    let payments = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(engineering.id),
        ScopeKind::Team,
        "payments",
        "Payments",
    )
    .await
    .expect("create payments");
    hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(org.id),
        ScopeKind::Team,
        identities::QUARANTINE_SLUG,
        "Quarantine",
    )
    .await
    .expect("create quarantine");
    tx.commit().await.expect("commit hierarchy");
    Org {
        org,
        engineering,
        platform,
        payments,
    }
}

async fn seed_user(pool: &PgPool, tenant: TenantId, subject: &str, parent: ScopeId) -> Identity {
    let mut tx = pool.begin().await.expect("begin");
    let id = IdentityId::new();
    let leaf = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(parent),
        ScopeKind::User,
        &personal_slug(None, subject, id),
        subject,
    )
    .await
    .expect("create personal scope");
    let identity = identities::create(
        &mut tx,
        id,
        tenant,
        Some(subject),
        IdentityKind::User,
        None,
        None,
        leaf.id,
    )
    .await
    .expect("create identity");
    tx.commit().await.expect("commit user");
    identity
}

#[allow(clippy::too_many_arguments)]
async fn seed_record_at(
    pool: &PgPool,
    tenant: TenantId,
    scope: ScopeId,
    owner: IdentityId,
    content: &str,
    sensitivity: Sensitivity,
    valid_from: DateTime<Utc>,
) -> RecordId {
    let embedder = DeterministicEmbedder::new();
    let vector = embedder
        .embed(std::slice::from_ref(&content.to_owned()))
        .await
        .expect("deterministic embed")
        .remove(0);
    let id = RecordId::new();
    records::insert(
        pool,
        id,
        tenant,
        &RecordState {
            scope_id: scope,
            owner_id: owner,
            kind: RecordKind::Derived,
            class: RecordClass::Procedure,
            content: content.to_owned(),
            sensitivity,
            provenance: json!({"source": "ctx-5 test fixture"}),
            valid_from,
            valid_to: None,
        },
        &RecordEmbedding {
            model: embedder.model().to_owned(),
            vector,
        },
    )
    .await
    .expect("insert record");
    id
}

async fn seed_record(
    pool: &PgPool,
    tenant: TenantId,
    scope: ScopeId,
    owner: IdentityId,
    content: &str,
) -> RecordId {
    seed_record_at(
        pool,
        tenant,
        scope,
        owner,
        content,
        Sensitivity::Internal,
        Utc::now() - Days::days(1),
    )
    .await
}

async fn post(app: &Router, path: &str, token: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build request");
    let response = app.clone().oneshot(request).await.expect("send request");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("json body")
    };
    (status, value)
}

/// The contents a response served, for readable assertions.
fn contents(response: &Value) -> Vec<String> {
    response["entries"]
        .as_array()
        .expect("entries array")
        .iter()
        .map(|entry| entry["content"].as_str().expect("content").to_owned())
        .collect()
}

/// Builds the app and makes the sidecar current, so a lexical query has
/// something to match. The product path: nothing is written to the index
/// by hand.
async fn app_for(pool: &PgPool, tenant: TenantId) -> (Router, Arc<SearchIndex>) {
    let index = Arc::new(SearchIndex::open(index_root()).expect("open sidecar"));
    let pdp = Arc::new(Pdp::new().expect("build pdp"));
    let state = state_with(&database_url(), Arc::clone(&index), pdp);
    let app = router(state);
    sweep(pool, tenant, &index).await;
    (app, index)
}

/// Runs the CTX-1 indexer until the sidecar has caught up with the
/// corpus — the product's own maintenance path, not a test shortcut.
async fn sweep(pool: &PgPool, tenant: TenantId, index: &SearchIndex) {
    indexer::sweep_tenant(
        pool,
        index,
        tenant,
        &IndexerConfig {
            overlap: Duration::ZERO,
            ..IndexerConfig::default()
        },
    )
    .await
    .expect("sweep search index");
}

// ── The universe ────────────────────────────────────────────────────────

/// The headline: `standard` has permitted a department-wide read since
/// AUTHZ-2, and until now nothing could perform one.
///
/// Alice is on `platform`. The payments runbook lives at `payments` —
/// a *sibling* team, not on her chain, so ADR-0024's inject universe
/// never asks about it. Her pack permits it: `resource in
/// principal.department && resource.kind != "user"`. One corpus, one
/// identity, one pack, two surfaces, and the difference between them is
/// the whole feature.
#[tokio::test]
async fn a_query_reaches_material_the_chain_never_composes() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let org = seed_org(&pool, tenant).await;
    let alice = seed_user(&pool, tenant, "alice", org.platform.id).await;
    let bea = seed_user(&pool, tenant, "bea", org.payments.id).await;

    // The real product pack, assigned at the org the product way.
    let mut tx = pool.begin().await.expect("begin");
    policy_assignments::assign(&mut *tx, tenant, org.org.id, "standard")
        .await
        .expect("assign standard");
    tx.commit().await.expect("commit assignment");

    seed_record(&pool, tenant, alice.scope_id, alice.id, ALICE_NOTE).await;
    seed_record(&pool, tenant, org.payments.id, bea.id, PAYMENTS_RUNBOOK).await;

    let (app, _index) = app_for(&pool, tenant).await;
    let token = issue("alice", tenant);

    // Surface one: the session-start block. Alice's chain is
    // alice → platform → engineering → acme; payments is not on it.
    let (status, injected) = post(
        &app,
        "/v1/inject",
        &token,
        json!({"task": "settlement mismatch procedure"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let block = injected["text"].as_str().expect("block text");
    assert!(
        !block.contains("acquirer statement"),
        "inject's universe is the chain (ADR-0024 decision 1); a sibling team's \
         material must not appear in a block. Block was:\n{block}"
    );

    // Surface two: the deep query. Same identity, same pack, same corpus.
    let (status, recalled) = post(
        &app,
        "/v1/recall",
        &token,
        json!({"query": "settlement mismatch acquirer statement"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(recalled["mode"], "query");
    let served = contents(&recalled);
    assert!(
        served.iter().any(|text| text == PAYMENTS_RUNBOOK),
        "recall's universe is every scope that could contribute (ADR-0042 \
         decision 2), and `standard` permits the department subtree — the \
         grant has been unreachable since ADR-0024 and is not any more. Got: {served:?}"
    );
    // And the decision was really taken, over more scopes than the chain.
    assert!(
        recalled["scopes_decided"].as_u64().expect("scopes_decided") > 4,
        "the walk decided beyond alice's four chain scopes: {recalled}"
    );
    assert_eq!(recalled["truncated"], false);
}

/// The same widening must not become a hole. `regulated-strict` has no
/// department permit — deny-first, no cross-team read without an explicit
/// grant (seed §2.3) — so the identical request over the identical corpus
/// must come back with alice's own material and nothing of the sibling's.
#[tokio::test]
async fn the_widened_universe_is_still_the_pdps_answer() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let org = seed_org(&pool, tenant).await;
    let alice = seed_user(&pool, tenant, "alice", org.platform.id).await;
    let bea = seed_user(&pool, tenant, "bea", org.payments.id).await;

    let mut tx = pool.begin().await.expect("begin");
    policy_assignments::assign(&mut *tx, tenant, org.org.id, "regulated-strict")
        .await
        .expect("assign regulated-strict");
    tx.commit().await.expect("commit assignment");

    seed_record(&pool, tenant, alice.scope_id, alice.id, ALICE_NOTE).await;
    seed_record(&pool, tenant, org.payments.id, bea.id, PAYMENTS_RUNBOOK).await;

    let (app, _index) = app_for(&pool, tenant).await;
    let (status, recalled) = post(
        &app,
        "/v1/recall",
        &issue("alice", tenant),
        json!({"query": "settlement mismatch acquirer statement rebase"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let served = contents(&recalled);
    assert!(
        !served.iter().any(|text| text == PAYMENTS_RUNBOOK),
        "a wider universe is more scopes *asked*, never more scopes allowed — \
         regulated-strict denies the sibling team and the answer must show it. \
         Got: {served:?}"
    );
}

/// A role binding is the other grant ADR-0024 left unreachable, and it is
/// the one an administrator actually issues. Same corpus, same pack, and
/// the difference is one row: a `viewer` binding on the sibling team.
#[tokio::test]
async fn a_role_binding_widens_the_universe_on_the_very_next_call() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let org = seed_org(&pool, tenant).await;
    seed_user(&pool, tenant, "alice", org.platform.id).await;
    let bea = seed_user(&pool, tenant, "bea", org.payments.id).await;

    let mut tx = pool.begin().await.expect("begin");
    policy_assignments::assign(&mut *tx, tenant, org.org.id, "regulated-strict")
        .await
        .expect("assign regulated-strict");
    tx.commit().await.expect("commit assignment");
    seed_record(&pool, tenant, org.payments.id, bea.id, PAYMENTS_RUNBOOK).await;

    let (app, _index) = app_for(&pool, tenant).await;
    let token = issue("alice", tenant);
    let ask = json!({"query": "settlement mismatch acquirer statement"});

    let (_, before) = post(&app, "/v1/recall", &token, ask.clone()).await;
    assert!(
        contents(&before).is_empty(),
        "no grant, nothing to read: {before}"
    );

    // The grant, written the product way.
    let mut tx = pool.begin().await.expect("begin");
    role_bindings::bind(
        &mut *tx,
        tenant,
        "alice",
        Some(org.payments.id),
        Role::Viewer,
    )
    .await
    .expect("bind viewer");
    tx.commit().await.expect("commit binding");

    let (_, after) = post(&app, "/v1/recall", &token, ask).await;
    assert!(
        contents(&after).iter().any(|text| text == PAYMENTS_RUNBOOK),
        "a binding governs the very next request (ADR-0015), and recall is \
         the surface that finally asks: {after}"
    );
}

// ── As-of ───────────────────────────────────────────────────────────────

/// The acceptance criterion: as-of returns historically accurate context.
///
/// A fact is stated, then corrected. The corrected record is a *new
/// version* of the same row, so `records` holds one truth and
/// `records_history` holds the other — which is exactly the pair FND-4
/// built and MEM-5/MEM-6 kept meaningful. Recall now returns the
/// correction; recall at the earlier instant returns what was true then.
#[tokio::test]
async fn as_of_returns_what_was_known_then_and_now_returns_the_correction() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let org = seed_org(&pool, tenant).await;
    let alice = seed_user(&pool, tenant, "alice", org.platform.id).await;

    let mut tx = pool.begin().await.expect("begin");
    policy_assignments::assign(&mut *tx, tenant, org.org.id, "standard")
        .await
        .expect("assign standard");
    tx.commit().await.expect("commit assignment");

    let id = seed_record(
        &pool,
        tenant,
        alice.scope_id,
        alice.id,
        "The deploy freeze runs from December 15th.",
    )
    .await;
    // The instant between the two truths, read from the database's clock
    // so it cannot disagree with the transaction timestamps.
    let between: DateTime<Utc> = sqlx::query_scalar!(r#"select now() as "now!""#)
        .fetch_one(&pool)
        .await
        .expect("read now");

    // The correction, through the store's own update path — a new
    // version, the old one archived with its transaction period closed.
    let embedder = DeterministicEmbedder::new();
    let corrected = "The deploy freeze runs from December 1st.";
    let vector = embedder
        .embed(std::slice::from_ref(&corrected.to_owned()))
        .await
        .expect("embed")
        .remove(0);
    records::update(
        &pool,
        id,
        &RecordState {
            scope_id: alice.scope_id,
            owner_id: alice.id,
            kind: RecordKind::Derived,
            class: RecordClass::Procedure,
            content: corrected.to_owned(),
            sensitivity: Sensitivity::Internal,
            provenance: json!({"source": "ctx-5 correction"}),
            valid_from: Utc::now() - Days::days(1),
            valid_to: None,
        },
        &RecordEmbedding {
            model: embedder.model().to_owned(),
            vector,
        },
    )
    .await
    .expect("correct record");

    let (app, _index) = app_for(&pool, tenant).await;
    let token = issue("alice", tenant);

    let (status, now) = post(&app, "/v1/recall", &token, json!({"ids": [id]})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        contents(&now),
        vec![corrected.to_owned()],
        "the present tense serves the correction"
    );

    let (status, then) = post(
        &app,
        "/v1/recall",
        &token,
        json!({"ids": [id], "as_of": between}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        contents(&then),
        vec!["The deploy freeze runs from December 15th.".to_owned()],
        "as-of serves what the database held then — the AC's \
         'historically accurate context' (ADR-0042 decision 7)"
    );
    assert_eq!(then["as_of"], json!(between), "the instant is echoed");
}

/// The swept form (ADR-0042 decision 14): a bare `as_of` is the complete
/// historical read, and it is a separate shape because a *query* cannot
/// give one.
///
/// A fact is superseded — MEM-5 closes the loser's valid window, so it
/// leaves the live corpus without leaving the database. A query as-of
/// cannot rank it: the search indexes hold current truth by construction
/// (ADR-0024 decision 4). A sweep reads the corpus itself and finds it.
/// This test is the difference between those two, asserted directly,
/// because it is the difference the AC's demo turns on.
#[tokio::test]
async fn a_bare_instant_sweeps_what_a_query_can_no_longer_rank() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let org = seed_org(&pool, tenant).await;
    let alice = seed_user(&pool, tenant, "alice", org.platform.id).await;
    let mut tx = pool.begin().await.expect("begin");
    policy_assignments::assign(&mut *tx, tenant, org.org.id, "standard")
        .await
        .expect("assign standard");
    tx.commit().await.expect("commit assignment");

    let id = seed_record(
        &pool,
        tenant,
        alice.scope_id,
        alice.id,
        "The deploy freeze runs from December 15th.",
    )
    .await;
    let between: DateTime<Utc> = sqlx::query_scalar!(r#"select now() as "now!""#)
        .fetch_one(&pool)
        .await
        .expect("read now");

    // The fact leaves the live corpus the way MEM-6 retires material: a
    // temporal delete, which archives the current version with its
    // transaction period closed and keeps history answerable (ADR-0040
    // decision 5) — the exact case decision 11 inherits.
    assert!(
        records::delete(&pool, id).await.expect("temporal delete"),
        "the record existed and is now retired"
    );

    let (app, _index) = app_for(&pool, tenant).await;
    let token = issue("alice", tenant);

    let (status, now) = post(&app, "/v1/recall", &token, json!({"ids": [id]})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        contents(&now).is_empty(),
        "the live corpus no longer holds it: {now}"
    );

    let (status, swept) = post(&app, "/v1/recall", &token, json!({"as_of": between})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(swept["mode"], "sweep");
    assert!(
        contents(&swept)
            .iter()
            .any(|text| text.contains("December 15th")),
        "a bare instant reads the corpus as it stood, including what the \
         live one has since retired (ADR-0042 decisions 11 and 14). Got: {swept}"
    );

    // And a bare recall that names no instant either is still a request
    // that has not said what it wants.
    let (status, _) = post(&app, "/v1/recall", &token, json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// The line this feature refuses to cross: as-of rewinds the corpus, not
/// the authority (ADR-0042 decision 8).
///
/// Alice reads the sibling team's runbook under a `viewer` binding, the
/// binding is withdrawn, and she asks again *at an instant when she still
/// held it*. There is no historical permission to inherit — a revoked
/// reader reads nothing, whatever timestamp they present.
#[tokio::test]
async fn as_of_never_rewinds_the_authority() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let org = seed_org(&pool, tenant).await;
    seed_user(&pool, tenant, "alice", org.platform.id).await;
    let bea = seed_user(&pool, tenant, "bea", org.payments.id).await;

    let mut tx = pool.begin().await.expect("begin");
    policy_assignments::assign(&mut *tx, tenant, org.org.id, "regulated-strict")
        .await
        .expect("assign regulated-strict");
    role_bindings::bind(
        &mut *tx,
        tenant,
        "alice",
        Some(org.payments.id),
        Role::Viewer,
    )
    .await
    .expect("bind viewer");
    tx.commit().await.expect("commit");

    let runbook = seed_record(&pool, tenant, org.payments.id, bea.id, PAYMENTS_RUNBOOK).await;
    let (app, _index) = app_for(&pool, tenant).await;
    let token = issue("alice", tenant);

    let (_, held) = post(&app, "/v1/recall", &token, json!({"ids": [runbook]})).await;
    assert_eq!(
        contents(&held),
        vec![PAYMENTS_RUNBOOK.to_owned()],
        "while the binding stands, the read succeeds"
    );
    let while_held: DateTime<Utc> = sqlx::query_scalar!(r#"select now() as "now!""#)
        .fetch_one(&pool)
        .await
        .expect("read now");

    let mut tx = pool.begin().await.expect("begin");
    let removed = role_bindings::unbind(
        &mut *tx,
        tenant,
        "alice",
        Some(org.payments.id),
        Role::Viewer,
    )
    .await
    .expect("revoke binding");
    assert!(removed, "the binding existed and is gone");
    tx.commit().await.expect("commit revocation");

    let (status, after) = post(
        &app,
        "/v1/recall",
        &token,
        json!({"ids": [runbook], "as_of": while_held}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        contents(&after).is_empty(),
        "an instant is not a credential: the PDP decides with the roles held \
         now, so a withdrawn binding cannot be walked under (ADR-0042 \
         decision 8). Got: {after}"
    );
}

/// Classification is retroactive (ADR-0042 decision 9): a record raised to
/// `confidential` is `confidential` for its own history too.
///
/// Without this rule, AUTHZ-5's whole leak suite is defeated by a query
/// parameter — the bytes are the same bytes, and an earlier version wore a
/// lower label. The tier ceiling is the strictest the record has carried
/// since the instant asked for, so the earlier version is refused.
#[tokio::test]
async fn a_reclassification_reaches_its_own_history() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let org = seed_org(&pool, tenant).await;
    seed_user(&pool, tenant, "alice", org.platform.id).await;
    let bea = seed_user(&pool, tenant, "bea", org.payments.id).await;

    // `standard` gives alice the department subtree at the *working*
    // tiers only — `confidential` is held to explicitly granted scopes
    // under every pack — which is precisely the boundary being tested.
    let mut tx = pool.begin().await.expect("begin");
    policy_assignments::assign(&mut *tx, tenant, org.org.id, "standard")
        .await
        .expect("assign standard");
    tx.commit().await.expect("commit assignment");

    let secret = seed_record(&pool, tenant, org.payments.id, bea.id, PAYMENTS_RUNBOOK).await;
    let (app, _index) = app_for(&pool, tenant).await;
    let token = issue("alice", tenant);

    let (_, before) = post(&app, "/v1/recall", &token, json!({"ids": [secret]})).await;
    assert_eq!(
        contents(&before),
        vec![PAYMENTS_RUNBOOK.to_owned()],
        "at `internal`, the department permit reaches it"
    );
    let while_internal: DateTime<Utc> = sqlx::query_scalar!(r#"select now() as "now!""#)
        .fetch_one(&pool)
        .await
        .expect("read now");

    // The organisation decides the material is confidential.
    let embedder = DeterministicEmbedder::new();
    let vector = embedder
        .embed(std::slice::from_ref(&PAYMENTS_RUNBOOK.to_owned()))
        .await
        .expect("embed")
        .remove(0);
    records::update(
        &pool,
        secret,
        &RecordState {
            scope_id: org.payments.id,
            owner_id: bea.id,
            kind: RecordKind::Derived,
            class: RecordClass::Procedure,
            content: PAYMENTS_RUNBOOK.to_owned(),
            sensitivity: Sensitivity::Confidential,
            provenance: json!({"source": "ctx-5 reclassification"}),
            valid_from: Utc::now() - Days::days(1),
            valid_to: None,
        },
        &RecordEmbedding {
            model: embedder.model().to_owned(),
            vector,
        },
    )
    .await
    .expect("reclassify");

    let (_, now) = post(&app, "/v1/recall", &token, json!({"ids": [secret]})).await;
    assert!(
        contents(&now).is_empty(),
        "the present-tense read is refused at the new tier: {now}"
    );

    let (status, historical) = post(
        &app,
        "/v1/recall",
        &token,
        json!({"ids": [secret], "as_of": while_internal}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        contents(&historical).is_empty(),
        "the bytes are the same bytes: a classification is a judgment about \
         content and it reaches the content's history, so the AUTHZ-5 leak \
         suite cannot be walked around with a timestamp (ADR-0042 decision 9). \
         Got: {historical}"
    );
}

// ── The surface ─────────────────────────────────────────────────────────

/// One route, two shapes, exclusive — and both refused honestly rather
/// than answered with an intersection nobody asked for.
#[tokio::test]
async fn the_two_shapes_are_exclusive_and_bounded() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let org = seed_org(&pool, tenant).await;
    seed_user(&pool, tenant, "alice", org.platform.id).await;
    let (app, _index) = app_for(&pool, tenant).await;
    let token = issue("alice", tenant);

    for (body, why) in [
        (
            json!({"ids": [RecordId::new()], "query": "both"}),
            "ids and query together is two questions",
        ),
        (json!({}), "neither is not a request"),
        (json!({"query": ""}), "an empty question is not a question"),
        (json!({"query": "x", "limit": 0}), "a limit of zero"),
        (json!({"query": "x", "limit": 33}), "a limit above the cap"),
    ] {
        let (status, _) = post(&app, "/v1/recall", &token, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{why}");
    }

    // The ids cap is unchanged from CTX-4 (ADR-0041 decision 7).
    let too_many: Vec<RecordId> = (0..33).map(|_| RecordId::new()).collect();
    let (status, _) = post(&app, "/v1/recall", &token, json!({"ids": too_many})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// The query form must not become the existence oracle the ids form was
/// built not to be (ADR-0041 decision 6, ADR-0042 compliance notes).
///
/// A second tenant's material, named directly and asked for by its own
/// distinctive words, comes back indistinguishable from nothing at all —
/// same status, same shape, no hint that anything was withheld.
#[tokio::test]
async fn a_query_is_not_an_existence_oracle() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let org = seed_org(&pool, tenant).await;
    seed_user(&pool, tenant, "alice", org.platform.id).await;
    let mut tx = pool.begin().await.expect("begin");
    policy_assignments::assign(&mut *tx, tenant, org.org.id, "standard")
        .await
        .expect("assign standard");
    tx.commit().await.expect("commit assignment");

    // A whole second tenant, with its own org and its own material.
    let other_id = TenantId::new();
    let slug = format!("ctx5-other-{}", other_id.as_uuid().simple());
    tenants::create(&pool, other_id, &slug, "Other", TenantStatus::Active)
        .await
        .expect("admit other tenant");
    let other = seed_org(&pool, other_id).await;
    let carol = seed_user(&pool, other_id, "carol", other.platform.id).await;
    let theirs = seed_record(
        &pool,
        other_id,
        other.platform.id,
        carol.id,
        "Zarquon protocol handshake uses a rotating nonce.",
    )
    .await;

    let (app, index) = app_for(&pool, tenant).await;
    sweep(&pool, other_id, &index).await;
    let token = issue("alice", tenant);

    let (query_status, by_query) = post(
        &app,
        "/v1/recall",
        &token,
        json!({"query": "Zarquon protocol handshake rotating nonce"}),
    )
    .await;
    let (named_status, by_id) = post(&app, "/v1/recall", &token, json!({"ids": [theirs]})).await;
    let (absent_status, by_nothing) = post(
        &app,
        "/v1/recall",
        &token,
        json!({"ids": [RecordId::new()]}),
    )
    .await;

    assert_eq!(query_status, StatusCode::OK);
    assert_eq!(named_status, StatusCode::OK);
    assert_eq!(absent_status, StatusCode::OK);
    assert!(contents(&by_query).is_empty(), "{by_query}");
    assert_eq!(
        by_id["entries"], by_nothing["entries"],
        "another tenant's record and an id that never existed must read \
         the same: a recall is not an oracle for 'does this exist'"
    );

    // Alice's own scope was still decided — the empty answer is a policy
    // outcome, not a broken request.
    assert!(by_query["scopes_decided"].as_u64().expect("decided") > 0);
}

/// ADR-0029 allotted the plan stage 15ms of a 300ms recall, and CTX-5
/// spends it on a universe wider than the chain. This is decision 17's
/// measurement: the number is asserted, not reported, because it was
/// pre-registered by a different feature and nobody could tune it to
/// this result.
///
/// `--ignored` and median-asserted, the HIER-1/MEM-1/CTX-1 discipline:
/// virtualised dev IO owns the tails and EVAL-6 owns percentile
/// enforcement on production-shaped hardware.
#[tokio::test]
#[ignore = "seeds a wide tenant; run with --ignored"]
async fn the_plan_stage_fits_the_budget_adr_0029_derived() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let org = seed_org(&pool, tenant).await;
    let alice = seed_user(&pool, tenant, "alice", org.platform.id).await;
    let mut tx = pool.begin().await.expect("begin");
    policy_assignments::assign(&mut *tx, tenant, org.org.id, "open-collaboration")
        .await
        .expect("assign open-collaboration");
    tx.commit().await.expect("commit assignment");

    // A tenant wide enough to be interesting: every scope occupied, so
    // the occupancy narrowing gives nothing away and the sweep pays full
    // price for each one.
    const TEAMS: usize = 512;
    for team in 0..TEAMS {
        let mut tx = pool.begin().await.expect("begin");
        let node = hierarchy::create(
            &mut tx,
            ScopeId::new(),
            tenant,
            Some(org.engineering.id),
            ScopeKind::Team,
            &format!("team-{team}"),
            &format!("Team {team}"),
        )
        .await
        .expect("create team");
        tx.commit().await.expect("commit team");
        seed_record(
            &pool,
            tenant,
            node.id,
            alice.id,
            &format!("Team {team} keeps its runbook here."),
        )
        .await;
    }

    let (app, _index) = app_for(&pool, tenant).await;
    let token = issue("alice", tenant);
    let ask = json!({"query": "runbook"});

    // Warm the chain cache and the pack state: the budget is a
    // warm-cache one, as inject's is (seed §10).
    let (status, warm) = post(&app, "/v1/recall", &token, ask.clone()).await;
    assert_eq!(status, StatusCode::OK);
    // The corpus is deliberately wider than the cap, so this run also
    // proves the cap binds *and* says so — a bounded answer that reads as
    // a complete one is the failure ADR-0042 decision 5 exists to prevent.
    assert_eq!(
        warm["truncated"], true,
        "512 occupied teams against a 64-scope cap must truncate: {warm}"
    );
    assert!(
        warm["scopes_considered"].as_u64().expect("considered")
            > warm["scopes_decided"].as_u64().expect("decided"),
        "and the response must report both numbers, not just the one it served: {warm}"
    );

    let mut samples: Vec<Duration> = Vec::new();
    for _ in 0..20 {
        let started = std::time::Instant::now();
        let (status, _) = post(&app, "/v1/recall", &token, ask.clone()).await;
        assert_eq!(status, StatusCode::OK);
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    let median = samples[samples.len() / 2];
    let p95 = samples[(samples.len() * 95) / 100];

    // The per-stage split, so "did it fit" is answerable per allowance
    // rather than only end to end — and so a future regression names the
    // stage that caused it instead of the total.
    let stages = stage_means(&app).await;
    println!(
        "\nCTX-5 recall over {} decided scopes ({} considered):\n  \
         whole request  median {:.1}ms  p95 {:.1}ms   (ADR-0029 budget 300ms)\n  \
         plan stage     mean   {:.1}ms                (ADR-0029 allowance 15ms)\n  \
         embed stage    mean   {:.1}ms\n  \
         search stage   mean   {:.1}ms\n  \
         admit stage    mean   {:.1}ms",
        warm["scopes_decided"],
        warm["scopes_considered"],
        median.as_secs_f64() * 1000.0,
        p95.as_secs_f64() * 1000.0,
        stages.get("plan").copied().unwrap_or_default() * 1000.0,
        stages.get("embed").copied().unwrap_or_default() * 1000.0,
        stages.get("search").copied().unwrap_or_default() * 1000.0,
        stages.get("admit").copied().unwrap_or_default() * 1000.0,
    );

    let plan = stages.get("plan").copied().unwrap_or_default();
    assert!(
        plan < 0.015,
        "the plan stage must fit ADR-0029's 15ms allowance — the one number \
         this feature was pre-registered against and could not tune. Mean was \
         {:.1}ms over {} decided scopes; if this cannot be met the lever is \
         MAX_RECALL_SCOPES (ADR-0042 decision 5), which makes the shortfall \
         visible to callers as `truncated` rather than silent.",
        plan * 1000.0,
        warm["scopes_decided"],
    );
    assert!(
        median < Duration::from_millis(300),
        "the whole recall must fit ADR-0029's derived 300ms budget; median was {median:?}"
    );
}

/// Mean seconds per recall stage, read from the gateway's own Prometheus
/// exposition — the metric an operator would read, rather than a second
/// timing path that could disagree with it.
async fn stage_means(app: &Router) -> std::collections::HashMap<String, f64> {
    let request = Request::builder()
        .method("GET")
        .uri("/metrics")
        .body(Body::empty())
        .expect("build request");
    let response = app.clone().oneshot(request).await.expect("send request");
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    let text = String::from_utf8_lossy(&bytes);
    let mut sums: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut counts: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for line in text.lines() {
        let Some((head, value)) = line.rsplit_once(' ') else {
            continue;
        };
        if !head.starts_with("synveda_recall_stage_duration_seconds_") {
            continue;
        }
        let Some(stage) = head
            .split_once("stage=\"")
            .and_then(|(_, rest)| rest.split_once('"'))
            .map(|(stage, _)| stage.to_owned())
        else {
            continue;
        };
        let Ok(parsed) = value.parse::<f64>() else {
            continue;
        };
        if head.starts_with("synveda_recall_stage_duration_seconds_sum") {
            *sums.entry(stage).or_default() += parsed;
        } else if head.starts_with("synveda_recall_stage_duration_seconds_count") {
            *counts.entry(stage).or_default() += parsed;
        }
    }
    sums.into_iter()
        .filter_map(|(stage, sum)| {
            let count = counts.get(&stage).copied().unwrap_or_default();
            (count > 0.0).then_some((stage, sum / count))
        })
        .collect()
}
