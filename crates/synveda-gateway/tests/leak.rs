//! AUTHZ-5's acceptance criterion (ADR-0038): **`restricted` records are
//! never injected without compliance-granted permission, proven by a
//! leak-test suite.**
//!
//! Every claim here is made from a reader's side, through
//! `POST /v1/inject`, because the criterion is about what a reader
//! *receives* — not about what a predicate contains. The material becomes
//! `restricted` through the product's own path (a classification proposal
//! two people approved, one of them compliance), which is what makes the
//! test's premise real rather than seeded: nothing else in the product can
//! mint that tier (decision 8).
//!
//! The generated half is deliberately blunt. A leak suite that asks one
//! well-chosen question proves that question; this asks the same corpus
//! back under dozens of query variants — every word, every pair, casing and
//! ordering shuffled — because a retrieval path leaks by surfacing
//! something under a phrasing nobody tried.
//!
//! EVAL-5 owns what this grows into: 10k variants nightly, the cross-tenant
//! fuzz (TEN-6), and the prompt-injection-via-memory suite. This suite
//! proves its own feature's AC and runs in seconds.
//!
//! ```text
//! acme (org)
//! ├── eng (department)
//! │   ├── platform (team)   ← the restricted runbook lives and is published here
//! │   └── payments (team)   ← reads it only under a grant that declared the tier
//! └── sales (department)
//!     └── field (team)      ← never reads it, grant or no grant
//! ```

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
use synveda_identity::{Hs256Verifier, personal_slug};
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder};
use synveda_policy::Pdp;
use synveda_retrieval::index::SearchIndex;
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::{hierarchy, identities, role_bindings, tenants};
use synveda_types::{
    HierarchyNode, Identity, IdentityId, IdentityKind, RecordClass, RecordId, RecordKind, Role,
    ScopeId, ScopeKind, Sensitivity, TenantId, TenantStatus,
};
use tower::ServiceExt;

const SECRET: &[u8] = b"authz-5-leak-secret";

/// The material the whole suite is about. Distinctive words, because the
/// generated queries are built from them and a false negative would be a
/// test that proves nothing.
const RESTRICTED: &str =
    "the vault break-glass ceremony needs two custodians and the offline shard";
const CONFIDENTIAL: &str = "the incident bridge rota is maintained by the platform leads";
const INTERNAL: &str = "deploys go out on tuesdays after the smoke suite passes";

/// The lapse window: long enough for the assertions under it, short enough
/// to wait out.
const WINDOW_SECS: u32 = 4;

// ── Harness ──────────────────────────────────────────────────────────────────

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
        pdp: Arc::new(Pdp::new().expect("build the embedded PDP")),
        scope_chains: Arc::new(synveda_store::ScopeChainCache::new()),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index: Arc::new(
            SearchIndex::open(
                std::env::temp_dir()
                    .join("synveda-authz5-leak")
                    .join(TenantId::new().to_string()),
            )
            .expect("open sidecar"),
        ),
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
                "skipping AUTHZ-5 leak suite: DATABASE_URL is not set \
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
    let slug = format!("authz5-{}", id.as_uuid().simple());
    tenants::create(
        &pool,
        id,
        &slug,
        "AUTHZ-5 leak tenant",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    Some((pool, id))
}

fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("checked by admitted_tenant")
}

struct Org {
    eng: HierarchyNode,
    platform: HierarchyNode,
    payments: HierarchyNode,
    field: HierarchyNode,
}

async fn seed_hierarchy(pool: &PgPool, tenant: TenantId) -> Org {
    let mut tx = pool.begin().await.expect("begin");
    let seeded = {
        let mut node = async |parent: Option<ScopeId>, kind, slug: &str, name: &str| {
            hierarchy::create(&mut tx, ScopeId::new(), tenant, parent, kind, slug, name)
                .await
                .expect("create node")
        };
        let org = node(None, ScopeKind::Org, "acme", "ACME").await;
        let eng = node(Some(org.id), ScopeKind::Department, "eng", "Engineering").await;
        let platform = node(Some(eng.id), ScopeKind::Team, "platform", "Platform").await;
        let payments = node(Some(eng.id), ScopeKind::Team, "payments", "Payments").await;
        let sales = node(Some(org.id), ScopeKind::Department, "sales", "Sales").await;
        let field = node(Some(sales.id), ScopeKind::Team, "field", "Field").await;
        node(
            Some(org.id),
            ScopeKind::Team,
            identities::QUARANTINE_SLUG,
            "Quarantine",
        )
        .await;
        Org {
            eng,
            platform,
            payments,
            field,
        }
    };
    tx.commit().await.expect("commit hierarchy");
    seeded
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

async fn bind(pool: &PgPool, tenant: TenantId, subject: &str, scope: ScopeId, role: Role) {
    let mut tx = pool.begin().await.expect("begin");
    role_bindings::bind(&mut *tx, tenant, subject, Some(scope), role)
        .await
        .expect("bind role");
    tx.commit().await.expect("commit binding");
}

async fn seed_record(
    pool: &PgPool,
    tenant: TenantId,
    scope: ScopeId,
    owner: IdentityId,
    content: &str,
    sensitivity: Sensitivity,
) -> RecordId {
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
            provenance: json!({"source": "authz-5 leak suite"}),
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

async fn post(app: &Router, token: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build request");
    call(app, request).await
}

/// One reader's composed block for one task, as text.
async fn block_for(app: &Router, token: &str, task: Option<&str>) -> String {
    let mut body = json!({"session_id": "authz-5-leak"});
    if let Some(task) = task {
        body["task"] = json!(task);
    }
    let (status, response) = post(app, token, "/v1/inject", body).await;
    assert_eq!(status, StatusCode::OK, "inject failed: {response}");
    response["text"].as_str().expect("block text").to_owned()
}

async fn approve(app: &Router, token: &str, proposal: &str) {
    let (status, body) = post(
        app,
        token,
        &format!("/v1/proposals/{proposal}/approve"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "approval failed: {body}");
}

/// The proposal's id, wherever the surface that opened it put it:
/// `/v1/proposals` flattens a summary keyed `id`, `/v1/lapses` answers
/// `proposal_id`.
fn proposal_id(body: &Value) -> String {
    body["id"]
        .as_str()
        .or_else(|| body["proposal_id"].as_str())
        .unwrap_or_else(|| panic!("no proposal id in {body}"))
        .to_owned()
}

/// Every query a reader might plausibly ask this corpus back with: each
/// word alone, each adjacent pair, the whole line, and casing variants —
/// plus the taskless session start, which ranks nothing and composes by
/// recency (a retrieval-free path that must not leak either).
fn query_variants(corpus: &[&str]) -> Vec<Option<String>> {
    let mut queries: Vec<Option<String>> = vec![None];
    for line in corpus {
        let words: Vec<&str> = line
            .split_whitespace()
            .filter(|word| word.len() > 3)
            .collect();
        queries.push(Some((*line).to_owned()));
        queries.push(Some(line.to_uppercase()));
        for word in &words {
            queries.push(Some((*word).to_owned()));
            queries.push(Some(word.to_uppercase()));
        }
        for pair in words.windows(2) {
            queries.push(Some(pair.join(" ")));
            // Reversed: fusion is rank-based and word order should not be
            // the thing standing between a reader and someone else's
            // material.
            queries.push(Some(format!("{} {}", pair[1], pair[0])));
        }
    }
    queries
}

// ── The acceptance criterion ─────────────────────────────────────────────────

/// **`restricted` records are never injected without compliance-granted
/// permission.**
///
/// The record earns its tier through the product path — a classification
/// proposal whose requirement the invariant floor priced at compliance plus
/// two distinct approvers — and is then published, so a lapse would have
/// something to disclose. Between those two facts and the grant, nobody
/// receives it under any phrasing.
#[tokio::test(flavor = "multi_thread")]
async fn restricted_never_reaches_a_reader_without_a_compliance_signed_grant() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let app = router(state(&database_url()));
    let org = seed_hierarchy(&pool, tenant).await;

    // Platform: the owner, the curator who publishes and classifies, and
    // the compliance officer the floor requires.
    let sam = seed_user(&pool, tenant, "sam", org.platform.id).await;
    seed_user(&pool, tenant, "cara", org.platform.id).await;
    seed_user(&pool, tenant, "cleo", org.platform.id).await;
    bind(&pool, tenant, "cara", org.platform.id, Role::Curator).await;
    bind(&pool, tenant, "cleo", org.platform.id, Role::Compliance).await;
    // Cleo also holds a content role, because approving a `restricted`
    // publication means reading it: the review surface shows content, and
    // the publication effect asks its own read (ADR-0038 decision 10).
    bind(&pool, tenant, "cleo", org.platform.id, Role::Curator).await;
    let cara = issue("cara", tenant);
    let cleo = issue("cleo", tenant);

    // Three tiers of material at platform, all `internal` to begin with:
    // the top tier does not exist yet, because nothing in this product can
    // conjure it (decision 8).
    let secret = seed_record(
        &pool,
        tenant,
        org.platform.id,
        sam.id,
        RESTRICTED,
        Sensitivity::Internal,
    )
    .await;
    let sensitive = seed_record(
        &pool,
        tenant,
        org.platform.id,
        sam.id,
        CONFIDENTIAL,
        Sensitivity::Confidential,
    )
    .await;
    seed_record(
        &pool,
        tenant,
        org.platform.id,
        sam.id,
        INTERNAL,
        Sensitivity::Internal,
    )
    .await;

    // ── The tier is earned, not seeded ──────────────────────────────────
    let (status, opened) = post(
        &app,
        &cara,
        "/v1/proposals",
        json!({
            "scope_id": org.platform.id,
            "record_ids": [secret],
            "title": "classify the break-glass ceremony as restricted",
            "effect": "classify",
            "sensitivity": "restricted",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "opening the classification: {opened}"
    );
    let classification = proposal_id(&opened);
    let outstanding = opened["outstanding"].as_str().unwrap_or_default();
    assert!(
        outstanding.contains("compliance"),
        "the invariant floor prices the top tier in compliance approvals: {opened}"
    );

    // One curator is not enough, and the refusal says what is missing.
    approve(&app, &cara, &classification).await;
    let (status, refused) = post(
        &app,
        &cara,
        &format!("/v1/proposals/{classification}/classify"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "one approver: {refused}");
    assert!(
        refused["message"]
            .as_str()
            .unwrap_or_default()
            .contains("compliance"),
        "the refusal names the role the floor requires: {refused}"
    );

    approve(&app, &cleo, &classification).await;
    let (status, classified) = post(
        &app,
        &cara,
        &format!("/v1/proposals/{classification}/classify"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "running the effect: {classified}");
    assert_eq!(classified["sensitivity"], "restricted");
    assert_eq!(
        classified["records"][0]["was"], "internal",
        "the event carries the tier it left, which is what prices it"
    );

    // ── And published, so a grant would have something to disclose ──────
    // A `restricted` publication resolves the same floor: this is the write
    // side of the same rule, and it is why the read side can be a mirror.
    let (status, publication) = post(
        &app,
        &cara,
        "/v1/proposals",
        json!({
            "scope_id": org.platform.id,
            "record_ids": [secret, sensitive],
            "title": "publish the platform runbooks",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "opening the publication: {publication}"
    );
    let publication = proposal_id(&publication);
    approve(&app, &cara, &publication).await;
    approve(&app, &cleo, &publication).await;
    let (status, published) = post(
        &app,
        &cara,
        &format!("/v1/proposals/{publication}/publish"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "publishing: {published}");

    // ── The sweep: nobody receives it, under any phrasing ───────────────
    seed_user(&pool, tenant, "priya", org.payments.id).await;
    seed_user(&pool, tenant, "fay", org.field.id).await;
    seed_user(&pool, tenant, "nadia", org.eng.id).await;
    seed_user(&pool, tenant, "omar", org.eng.id).await;
    bind(&pool, tenant, "nadia", org.eng.id, Role::Steward).await;
    bind(&pool, tenant, "omar", org.eng.id, Role::Steward).await;
    let priya = issue("priya", tenant);
    let fay = issue("fay", tenant);
    let nadia = issue("nadia", tenant);
    let sam_token = issue("sam", tenant);

    let corpus = [RESTRICTED, CONFIDENTIAL, INTERNAL];
    let queries = query_variants(&corpus);
    assert!(queries.len() > 40, "a blunt sweep, not one lucky question");

    // Every reader in the org, including the record's own author and a
    // steward of the department above it. The author's inclusion is the
    // sharp edge stated in decision 7: the tier means what it says.
    for (who, token) in [
        ("priya (payments)", &priya),
        ("fay (another department)", &fay),
        ("nadia (steward above platform)", &nadia),
        ("sam (the record's own author)", &sam_token),
    ] {
        for query in &queries {
            let block = block_for(&app, token, query.as_deref()).await;
            assert!(
                !block.contains(RESTRICTED),
                "{who} received restricted material for query {query:?}:\n{block}"
            );
        }
    }

    // The sweep is only meaningful if the *rest* of the corpus does travel:
    // a suite where nothing composes proves nothing.
    let sams_block = block_for(&app, &sam_token, Some("tuesdays smoke suite")).await;
    assert!(
        sams_block.contains(INTERNAL),
        "the working tier still composes for its own team: {sams_block}"
    );

    // ── The grant, and the tier it must declare ─────────────────────────
    // A lapse that declares only the working tier changes nothing about the
    // top one: the same reader, the same window, the same target.
    let (status, weak) = post(
        &app,
        &nadia,
        "/v1/lapses",
        json!({
            "scope_id": org.platform.id,
            "grantee_scope_id": org.payments.id,
            "action": "memory.read",
            "duration_secs": WINDOW_SECS,
            "reason": "joint incident review, working tier",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "opening the working-tier lapse: {weak}"
    );
    let weak = proposal_id(&weak);
    approve(&app, &nadia, &weak).await;
    approve(&app, &issue("omar", tenant), &weak).await;
    let (status, granted) = post(
        &app,
        &nadia,
        &format!("/v1/proposals/{weak}/lapse"),
        json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "granting the working-tier lapse: {granted}"
    );

    for query in &queries {
        let block = block_for(&app, &priya, query.as_deref()).await;
        assert!(
            !block.contains(RESTRICTED),
            "a working-tier grant is not a door to the top tier ({query:?}):\n{block}"
        );
    }

    // ── The grant that does declare it ──────────────────────────────────
    let (status, strong) = post(
        &app,
        &nadia,
        "/v1/lapses",
        json!({
            "scope_id": org.platform.id,
            "grantee_scope_id": org.payments.id,
            "action": "memory.read",
            "max_sensitivity": "restricted",
            "duration_secs": WINDOW_SECS,
            "reason": "the vault incident: payments needs the ceremony",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "opening the restricted lapse: {strong}"
    );
    let strong_id = proposal_id(&strong);
    assert!(
        strong["outstanding"]
            .as_str()
            .unwrap_or_default()
            .contains("compliance"),
        "declaring the top tier is what pulls the floor in: {strong}"
    );

    // Two stewards are no longer enough: this is the AC's
    // "compliance-granted permission", and nobody wrote a rule for it.
    approve(&app, &nadia, &strong_id).await;
    approve(&app, &issue("omar", tenant), &strong_id).await;
    let (status, refused) = post(
        &app,
        &nadia,
        &format!("/v1/proposals/{strong_id}/lapse"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "stewards alone: {refused}");
    assert!(
        refused["message"]
            .as_str()
            .unwrap_or_default()
            .contains("compliance"),
        "and the refusal names what is missing: {refused}"
    );

    approve(&app, &cleo, &strong_id).await;
    let (status, granted) = post(
        &app,
        &nadia,
        &format!("/v1/proposals/{strong_id}/lapse"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "granting: {granted}");

    // Now — and only now — the reader receives it, marked twice: the
    // section as lapsed, the line as restricted.
    let under_grant = block_for(&app, &priya, Some("vault break-glass ceremony")).await;
    assert!(
        under_grant.contains(RESTRICTED),
        "the grant the floor priced is the one that discloses: {under_grant}"
    );
    assert!(
        under_grant.contains("[restricted]"),
        "the line says what it carries: {under_grant}"
    );
    assert!(
        under_grant.contains("[lapse]"),
        "and the section says how it got here: {under_grant}"
    );

    // The grant reaches its grantee and nobody else — a different
    // department's reader is unaffected by any of it.
    for query in &queries {
        let block = block_for(&app, &fay, query.as_deref()).await;
        assert!(
            !block.contains(RESTRICTED),
            "a grant to payments said nothing about sales ({query:?}):\n{block}"
        );
    }

    // ── Expiry: nobody acts, and the door closes ────────────────────────
    tokio::time::sleep(Duration::from_secs(u64::from(WINDOW_SECS) + 1)).await;
    for query in &queries {
        let block = block_for(&app, &priya, query.as_deref()).await;
        assert!(
            !block.contains(RESTRICTED),
            "the window closed with nobody acting ({query:?}):\n{block}"
        );
    }
}

/// The other half of the tier's meaning, and the one a reader is most
/// likely to meet: `confidential` material is held to explicitly granted
/// scopes, and a binding is what grants it (decision 4).
#[tokio::test(flavor = "multi_thread")]
async fn confidential_material_takes_an_explicit_grant_and_a_binding_is_one() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let app = router(state(&database_url()));
    let org = seed_hierarchy(&pool, tenant).await;

    // On the reader's **own team**, deliberately. A binding at another
    // scope would not help here and the test would be about the wrong
    // thing: the inject universe is the caller's chain, and it widens by
    // lapse and by nothing else (ADR-0024 decision 2, ADR-0037
    // decision 13). What AUTHZ-5 decides is what a reader may see *at the
    // scopes it already composes*.
    let mia = seed_user(&pool, tenant, "mia", org.payments.id).await;
    seed_user(&pool, tenant, "priya", org.payments.id).await;
    seed_record(
        &pool,
        tenant,
        org.payments.id,
        mia.id,
        CONFIDENTIAL,
        Sensitivity::Confidential,
    )
    .await;
    // Working-tier material at the same scope, to prove the block is not
    // simply empty.
    seed_record(
        &pool,
        tenant,
        org.payments.id,
        mia.id,
        INTERNAL,
        Sensitivity::Internal,
    )
    .await;

    let priya = issue("priya", tenant);
    let queries = query_variants(&[CONFIDENTIAL]);
    for query in &queries {
        let block = block_for(&app, &priya, query.as_deref()).await;
        assert!(
            !block.contains(CONFIDENTIAL),
            "membership alone does not reach confidential ({query:?}):\n{block}"
        );
    }

    // A content-role binding at her own team is the explicit grant, and it
    // is in force on the very next request.
    bind(&pool, tenant, "priya", org.payments.id, Role::Contributor).await;
    let bound = block_for(&app, &priya, Some("incident bridge rota")).await;
    assert!(
        bound.contains(CONFIDENTIAL),
        "the binding reaches confidential: {bound}"
    );
    assert!(
        bound.contains("[confidential]"),
        "and the line says so: {bound}"
    );

    // The caller may narrow past what policy allows, never widen
    // (decision 12): asking for `internal` drops the tier it just gained.
    let (status, narrowed) = post(
        &app,
        &priya,
        "/v1/inject",
        json!({
            "session_id": "authz-5-leak",
            "task": "incident bridge rota",
            "max_sensitivity": "internal",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "narrowed inject failed: {narrowed}");
    let text = narrowed["text"].as_str().expect("block text");
    assert!(
        !text.contains(CONFIDENTIAL),
        "a caller asking for less gets less: {text}"
    );
    assert!(
        text.contains(INTERNAL),
        "and still gets what it asked for: {text}"
    );
}
