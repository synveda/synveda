//! CTX-4 acceptance criteria (ADR-0041): "token cost of index tier
//! measured; agent can navigate index→body in a live Claude Code
//! session."
//!
//! Both halves over the real product surfaces, never a seeded row and
//! never a composed block built by a harness: a corpus is written, a
//! budget too small to carry it is set on the caller's pack, and
//! `POST /v1/inject` is asked. The measurement is the same corpus asked
//! twice — with the tier `off`, which is the product as it behaved before
//! CTX-4, and with it `demote` — and the numbers reported are the ones
//! the acceptance criterion names.
//!
//! The navigation half is the whole point and is asserted as a round
//! trip: the handle the block printed is fed back to `POST /v1/recall`
//! and the body comes out, with nothing carried between the two calls but
//! the id itself.
//!
//! And the half that matters more than either: a handle is a **name**,
//! not a capability (ADR-0041 decision 5). The same id, the same block,
//! the same session — and a role binding withdrawn between the inject and
//! the recall — must not come back.
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
use synveda_audit::ChainVerification;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::{Hs256Verifier, personal_slug};
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder, Embedder as _};
use synveda_policy::Pdp;
use synveda_retrieval::index::SearchIndex;
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::{hierarchy, identities, policy_assignments, rls, tenants};
use synveda_types::{
    CompositionConfig, HierarchyNode, Identity, IdentityId, IdentityKind, IndexTier, PackConfig,
    RecordClass, RecordId, RecordKind, ScopeId, ScopeKind, Sensitivity, TenantId, TenantStatus,
};
use tower::ServiceExt;

const SECRET: &[u8] = b"ctx-4-test-secret";

/// A permissive stored-pack source: every grant comes from this permit,
/// and the compiled base layer still rides along — this is a pack, not a
/// hole (seed §2.2).
const BLANKET: &str = "permit (principal, action, resource) when { resource in principal.tenant };";

/// A runbook nobody summarised: the shape a context pack (PRMT-2) or a
/// skill (SKIL-1) will have, written as a memory record because those
/// asset kinds do not exist yet. Long enough that naming it is cheaper
/// than showing it, which is the only condition CTX-4 demotes under.
const RUNBOOK: &str = "Payments incident runbook. \
    On a settlement mismatch alert, first freeze the reconciliation job from the operator \
    console, then compare the ledger tail against the acquirer statement for the affected \
    window, then page the payments on-call and the finance controller together, because a \
    mismatch that survives one reconciliation cycle becomes a regulatory reporting item \
    within twenty-four hours. Do not restart the settlement worker before the comparison \
    is recorded: the worker rewrites its cursor on boot and the window is then \
    unrecoverable from the application side.";

const SHORT_NOTE: &str = "Alice prefers rebase-and-merge for her feature branches.";

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn index_root() -> std::path::PathBuf {
    std::env::temp_dir()
        .join("synveda-ctx4-tests")
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
                "skipping CTX-4 test: DATABASE_URL is not set \
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
    let slug = format!("ctx4-{}", id.as_uuid().simple());
    tenants::create(&pool, id, &slug, "CTX-4 test tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((pool, id))
}

fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("checked by admitted_tenant")
}

async fn seed_hierarchy(pool: &PgPool, tenant: TenantId) -> (HierarchyNode, HierarchyNode) {
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
    let platform = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(org.id),
        ScopeKind::Team,
        "platform",
        "Platform",
    )
    .await
    .expect("create team");
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
    (org, platform)
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

async fn seed_record(
    pool: &PgPool,
    tenant: TenantId,
    scope: ScopeId,
    owner: IdentityId,
    kind: RecordKind,
    content: &str,
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
            kind,
            class: RecordClass::Procedure,
            content: content.to_owned(),
            sensitivity: Sensitivity::Internal,
            provenance: json!({"source": "ctx-4 test fixture"}),
            valid_from: chrono::Utc::now(),
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

/// Installs a pack carrying a composition config and makes it the tenant
/// default — the budget and the index tier the whole suite turns on. The
/// ADR-0025 decision 3 machinery, used as the product uses it: the config
/// rides the pack, and the pack governs the very next request.
async fn apply_pack(
    pool: &PgPool,
    pdp: &Pdp,
    tenant: TenantId,
    name: &str,
    version: i64,
    budget_tokens: u32,
    index_tier: IndexTier,
) {
    pdp.install_source(
        tenant,
        name,
        version,
        BLANKET,
        PackConfig {
            composition: Some(CompositionConfig {
                budget_tokens,
                index_tier,
                ..CompositionConfig::DEFAULT
            }),
            ..Default::default()
        },
    )
    .expect("install pack");
    policy_assignments::set_default(pool, tenant, name)
        .await
        .expect("set default pack");
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

/// The world every test here starts in: alice on platform, a long runbook
/// at the team, a short note of her own, and a budget too small to carry
/// both in full.
struct World {
    pool: PgPool,
    tenant: TenantId,
    app: Router,
    token: String,
    pdp: Arc<Pdp>,
    runbook: RecordId,
    note: RecordId,
}

/// The budget: room for alice's short note in full and the runbook only
/// by name. Chosen once, used by every test, so the tier's behaviour is
/// what varies between them and never the budget.
const TIGHT_BUDGET: u32 = 240;

async fn world(index_tier: IndexTier) -> Option<World> {
    let (pool, tenant) = admitted_tenant().await?;
    let (org, platform) = seed_hierarchy(&pool, tenant).await;
    let alice = seed_user(&pool, tenant, "alice", platform.id).await;
    let runbook = seed_record(
        &pool,
        tenant,
        platform.id,
        alice.id,
        RecordKind::Pinned,
        RUNBOOK,
    )
    .await;
    let note = seed_record(
        &pool,
        tenant,
        alice.scope_id,
        alice.id,
        RecordKind::Derived,
        SHORT_NOTE,
    )
    .await;
    let pdp = Arc::new(Pdp::new().expect("build the embedded PDP"));
    apply_pack(
        &pool,
        &pdp,
        tenant,
        &format!("ctx4-{}", tenant.as_uuid().simple()),
        1,
        TIGHT_BUDGET,
        index_tier,
    )
    .await;
    let index = Arc::new(SearchIndex::open(index_root()).expect("open sidecar"));
    let app = router(state_with(&database_url(), index, Arc::clone(&pdp)));
    let token = issue("alice", tenant);
    let _ = org;
    Some(World {
        pool,
        tenant,
        app,
        token,
        pdp,
        runbook,
        note,
    })
}

// ── The acceptance criterion, first half: the cost, measured ──────────

/// **"Token cost of index tier measured."**
///
/// The same corpus, the same budget, the same caller — asked once under
/// the product as it behaved before CTX-4 (`index_tier: off`) and once
/// with the tier on. The assertion is not that the tier is free; it is
/// that the trade is the one ADR-0041 decision 2 describes, and the
/// numbers are printed so the measurement is a measurement.
#[tokio::test]
async fn the_index_tiers_token_cost_is_measured() {
    let Some(off) = world(IndexTier::Off).await else {
        return;
    };
    let (status, before) = post(&off.app, "/v1/inject", &off.token, json!({})).await;
    assert_eq!(status, StatusCode::OK);

    let Some(on) = world(IndexTier::Demote).await else {
        return;
    };
    let (status, after) = post(&on.app, "/v1/inject", &on.token, json!({})).await;
    assert_eq!(status, StatusCode::OK);

    let named_before = before["record_ids"].as_array().expect("ids").len();
    let named_after = after["record_ids"].as_array().expect("ids").len();
    let index_tokens = after["index_tokens"].as_u64().expect("index_tokens");
    let index_entries = after["index_entries"].as_u64().expect("index_entries");

    // The measurement the AC asks for, reported rather than merely
    // asserted — a number a reviewer can read.
    println!(
        "CTX-4 index tier at a {TIGHT_BUDGET}-token budget: \
         records named {named_before} -> {named_after}; \
         block tokens {} -> {}; index tier cost {index_tokens} tokens \
         across {index_entries} entries ({}% of the block)",
        before["tokens"],
        after["tokens"],
        index_tokens * 100 / after["tokens"].as_u64().expect("tokens").max(1),
    );

    assert_eq!(
        before["index_entries"], 0,
        "the pre-CTX-4 product names nothing"
    );
    assert_eq!(before["index_tokens"], 0);
    assert_eq!(
        before["skipped_over_budget"].as_u64(),
        None,
        "skipped_over_budget is an audit-payload field, not a response one"
    );
    assert_eq!(index_entries, 1, "one record was named rather than dropped");
    assert!(
        named_after > named_before,
        "the tier's whole purpose: the agent learns of {named_after} records \
         where it learned of {named_before}"
    );
    assert!(
        index_tokens > 0,
        "and it cost something — a free feature would be a suspicious one"
    );
    assert!(
        after["tokens"].as_u64().expect("tokens") <= u64::from(TIGHT_BUDGET),
        "the budget still bounds the block"
    );
    // The legend is charged once, to the first demotion (decision 12).
    assert!(
        after["text"]
            .as_str()
            .expect("text")
            .contains("synveda recall"),
        "and the block says how to navigate"
    );
    assert!(
        !before["text"]
            .as_str()
            .expect("text")
            .contains("synveda recall"),
        "while the tier-off block is byte-clean of it"
    );
}

// ── The acceptance criterion, second half: index → body ───────────────

/// **"Agent can navigate index→body."**
///
/// The round trip, with nothing carried between the calls but the id the
/// block printed: inject names the runbook and elides it, the handle goes
/// back to `POST /v1/recall`, and the body comes out in full with its
/// labels. This is what `synveda recall <id>` runs, and what the MCP tool
/// CTX-5 adds will run.
#[tokio::test]
async fn an_agent_navigates_from_the_index_entry_to_the_body() {
    let Some(w) = world(IndexTier::Demote).await else {
        return;
    };
    let (status, block) = post(&w.app, "/v1/inject", &w.token, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let text = block["text"].as_str().expect("text").to_owned();

    // The block names it and does not carry it.
    assert!(
        text.contains(&format!("(recall {})", w.runbook)),
        "the handle is in the rendered text — that is what makes it navigable:\n{text}"
    );
    assert!(
        !text.contains("the window is then"),
        "and the runbook's tail did not compose:\n{text}"
    );
    // The short note did compose in full: decision 2 never demotes what
    // demoting would not help.
    assert!(
        text.contains(SHORT_NOTE),
        "the near record is in full:\n{text}"
    );
    let tiers = block["tiers"].as_array().expect("tiers");
    assert!(
        tiers.iter().any(|tier| tier == "index") && tiers.iter().any(|tier| tier == "body"),
        "the response says which is which: {tiers:?}"
    );

    // Now the navigation, with the id lifted out of the text exactly as an
    // agent would lift it.
    let handle = handle_in(&text).expect("an index entry's handle");
    assert_eq!(handle, w.runbook.to_string(), "the id the block printed");
    let (status, recalled) = post(
        &w.app,
        "/v1/recall",
        &w.token,
        json!({ "ids": [handle], "session_id": "ctx-4-live" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let entries = recalled["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 1, "the body came back");
    assert_eq!(entries[0]["record_id"], json!(w.runbook));
    assert_eq!(
        entries[0]["content"],
        json!(RUNBOOK),
        "in full — recall does not truncate (ADR-0041 decision 7)"
    );
    // The labels the tech plan §3 asks for: channel, provenance, validity.
    assert_eq!(entries[0]["channel"], json!("derived"));
    assert_eq!(entries[0]["kind"], json!("pinned"));
    assert_eq!(entries[0]["class"], json!("procedure"));
    assert!(entries[0]["provenance"].is_object());
    assert!(entries[0]["valid_from"].is_string());
    assert!(
        entries[0]["object_hash"]
            .as_str()
            .is_some_and(|hash| hash.len() == 64),
        "and the same content address the block watermarked"
    );
    assert_eq!(recalled["requested"], json!(1));

    // One `context.recalled` event, chained, carrying counts and
    // addresses and no content (decision 8).
    let (events, verification) = chain(&w.pool, w.tenant).await;
    assert!(
        matches!(verification, ChainVerification::Valid { .. }),
        "chain verifies: {verification}"
    );
    let recall_events: Vec<_> = events
        .iter()
        .filter(|event| event.action == "context.recalled")
        .collect();
    assert_eq!(recall_events.len(), 1, "one event per recall");
    let payload = &recall_events[0].payload;
    assert_eq!(payload["requested"], json!(1));
    assert_eq!(payload["served"], json!(1));
    assert_eq!(payload["session_id"], json!("ctx-4-live"));
    assert_eq!(payload["entries"][0]["record_id"], json!(w.runbook));
    let rendered = serde_json::to_string(payload).expect("payload json");
    assert!(
        !rendered.contains("settlement mismatch"),
        "no record content on the chain (the ADR-0021 discipline): {rendered}"
    );
}

// ── A handle is a name, not a capability (decision 5) ─────────────────

/// The decision the whole surface rests on. Alice injects, gets a handle,
/// and *then* loses the access that produced it — the pack at the org is
/// replaced with one that permits nothing. The very next recall of the
/// very same id serves nothing.
///
/// Nothing was revoked about the handle, because a handle is not a thing
/// that can be revoked. It is a name, and the name stopped resolving
/// because the decision behind it changed.
#[tokio::test]
async fn a_handle_stops_resolving_when_the_decision_behind_it_changes() {
    let Some(w) = world(IndexTier::Demote).await else {
        return;
    };
    let (_, block) = post(&w.app, "/v1/inject", &w.token, json!({})).await;
    let handle = handle_in(block["text"].as_str().expect("text")).expect("a handle");

    // It resolves now.
    let (status, before) = post(&w.app, "/v1/recall", &w.token, json!({ "ids": [handle] })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(before["entries"].as_array().expect("entries").len(), 1);

    // The tenant's pack is replaced with one that grants nothing — the
    // ADR-0014 freshness path, thrown for real. No restart, no cache
    // flush, and nothing whatsoever touching the handle.
    let locked = format!("ctx4-locked-{}", w.tenant.as_uuid().simple());
    w.pdp
        .install_source(
            w.tenant,
            &locked,
            1,
            // A pack that permits nothing: only the compiled base layer's
            // own floor survives, and alice's team is not her own chain
            // head.
            "permit (principal, action, resource) when { false };",
            PackConfig::default(),
        )
        .expect("install locked pack");
    policy_assignments::set_default(&w.pool, w.tenant, &locked)
        .await
        .expect("set locked pack as default");

    // The very next recall — same id, same token, same session.
    let (status, after) = post(&w.app, "/v1/recall", &w.token, json!({ "ids": [handle] })).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a policy outcome on a read is a result, not an error"
    );
    assert!(
        after["entries"].as_array().expect("entries").is_empty(),
        "the runbook is not served: the plan that admitted it is gone, and \
         the handle never carried authority of its own"
    );
    assert_eq!(
        after["requested"],
        json!(1),
        "the caller is told what it asked for, never which ids were refused"
    );
}

/// The uniform refusal (decision 6): an id that never existed, an id from
/// another tenant, and an id the caller may not read are indistinguishable
/// from here. A recall must not be an oracle for "does this record exist".
#[tokio::test]
async fn refusals_are_uniform_and_silent() {
    let Some(w) = world(IndexTier::Demote).await else {
        return;
    };
    // Another tenant's record, seeded the same way.
    let Some((other_pool, other_tenant)) = admitted_tenant().await else {
        return;
    };
    let (_, other_team) = seed_hierarchy(&other_pool, other_tenant).await;
    let bob = seed_user(&other_pool, other_tenant, "bob", other_team.id).await;
    let foreign = seed_record(
        &other_pool,
        other_tenant,
        other_team.id,
        bob.id,
        RecordKind::Pinned,
        "another tenant's runbook entirely",
    )
    .await;

    let nonexistent = RecordId::new();
    let (status, response) = post(
        &w.app,
        "/v1/recall",
        &w.token,
        json!({ "ids": [nonexistent, foreign, w.note] }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let entries = response["entries"].as_array().expect("entries");
    assert_eq!(
        entries.len(),
        1,
        "only alice's own note came back; the other two read identically"
    );
    assert_eq!(entries[0]["record_id"], json!(w.note));
    assert_eq!(response["requested"], json!(3));
    let rendered = serde_json::to_string(&response).expect("json");
    assert!(
        !rendered.contains(&foreign.to_string()) && !rendered.contains(&nonexistent.to_string()),
        "neither refused id is echoed, which is what keeps this from \
         answering 'that one exists': {rendered}"
    );

    // And the same silence on the chain (decision 8).
    let (events, verification) = chain(&w.pool, w.tenant).await;
    assert!(
        matches!(verification, ChainVerification::Valid { .. }),
        "{verification}"
    );
    let payload = &events
        .iter()
        .find(|event| event.action == "context.recalled")
        .expect("a recall event")
        .payload;
    assert_eq!(payload["requested"], json!(3));
    assert_eq!(payload["served"], json!(1));
    let rendered = serde_json::to_string(payload).expect("json");
    assert!(
        !rendered.contains(&foreign.to_string()),
        "the trail records the counts, never the refused ids: {rendered}"
    );
}

/// The surface's own bounds (decision 7): a recall naming nothing, and one
/// naming more than the cap, are contract rejections rather than
/// expensive answers.
#[tokio::test]
async fn the_id_cap_bounds_the_surface() {
    let Some(w) = world(IndexTier::Demote).await else {
        return;
    };
    let (status, _) = post(&w.app, "/v1/recall", &w.token, json!({ "ids": [] })).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "naming nothing is a mistake"
    );

    let too_many: Vec<RecordId> = (0..33).map(|_| RecordId::new()).collect();
    let (status, body) = post(&w.app, "/v1/recall", &w.token, json!({ "ids": too_many })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|message| message.contains("32")),
        "and the refusal names the cap: {body}"
    );
}

fn handle_in(text: &str) -> Option<String> {
    let start = text.find("(recall ")? + "(recall ".len();
    let rest = &text[start..];
    let end = rest.find(')')?;
    Some(rest[..end].to_owned())
}

/// The tenant's whole chain, oldest first, plus its verification.
async fn chain(
    pool: &PgPool,
    tenant_id: TenantId,
) -> (
    Vec<synveda_audit::StoredEvent>,
    synveda_audit::ChainVerification,
) {
    let mut tx = rls::begin_tenant_tx(pool, tenant_id)
        .await
        .expect("begin tenant tx");
    let mut events = synveda_audit::tail(&mut tx, tenant_id, 100)
        .await
        .expect("read chain");
    events.reverse();
    let verification = synveda_audit::verify(&mut tx, tenant_id)
        .await
        .expect("verify chain");
    (events, verification)
}
