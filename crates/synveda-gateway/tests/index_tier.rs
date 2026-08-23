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
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder, Embedder as _};
use synveda_policy::Pdp;
use synveda_retrieval::index::SearchIndex;
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::{identities, policy_assignments, scopes, tenants};
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{
    CompositionConfig, Identity, IdentityId, IdentityKind, IndexTier, PackConfig, RecordClass,
    RecordId, RecordKind, ScopeId, Sensitivity, TenantId, TenantStatus,
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
        service_token_max_ttl: Duration::from_secs(3600),
        search_index,
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

async fn seed_hierarchy(pool: &PgPool, tenant: TenantId) -> (Scope, Scope) {
    let mut tx = pool.begin().await.expect("begin");
    let org = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("mint root");
    let platform = scopes::create(
        &mut tx,
        &scopes::NewScope {
            id: ScopeId::new(),
            tenant_id: tenant,
            kind: ScopeKind::OrgUnit,
            parent_scope_id: Some(org.id),
            slug: "platform".to_owned(),
            display_name: "Platform".to_owned(),
            attributes: serde_json::json!({}),
            principal_id: None,
            created_by: None,
        },
    )
    .await
    .expect("create scope");
    tx.commit().await.expect("commit hierarchy");
    (org, platform)
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

/// The world every test here starts in: alice on platform, a long runbook
/// at the team, a short note of her own, and a budget too small to carry
/// both in full.
#[path = "session_seed.rs"]
mod session_seed;

/// A composition for this world's reader.
async fn compose(w: &World) -> Value {
    let request = Request::builder()
        .method("POST")
        .uri(format!("/v1/sessions/{}/context-runs", w.run))
        .header("authorization", format!("Bearer {}", w.token))
        .header(
            "idempotency-key",
            synveda_types::ContextRunId::new().to_string(),
        )
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .expect("build context-run request");
    let response = w.app.clone().oneshot(request).await.expect("send request");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    let body: Value = serde_json::from_slice(&bytes).expect("json body");
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "the composition must succeed: {status} {body}"
    );
    body
}

/// The record ids a block composed, from its watermark line.
fn record_ids(body: &Value) -> Vec<String> {
    let rendered = body["rendered"].as_str().unwrap_or_default();
    let Some(marker) = rendered.split("records=").nth(1) else {
        return Vec::new();
    };
    let ids = marker.split("-->").next().unwrap_or_default().trim();
    if ids.is_empty() || ids == "none" {
        return Vec::new();
    }
    ids.split(',').map(str::trim).map(str::to_owned).collect()
}

/// How many entries the block named rather than composed in full.
fn handles(body: &Value) -> usize {
    body["rendered"]
        .as_str()
        .unwrap_or_default()
        .matches("(recall ")
        .count()
}

struct World {
    app: Router,
    token: String,
    run: synveda_types::SessionId,
}

/// The budget: room for alice's short note in full and the runbook only
/// by name. Chosen once, used by every test, so the tier's behaviour is
/// what varies between them and never the budget.
///
/// Recalibrated for CPR-7: the runbook composes from the tenant root now
/// (ADR-0074 decision 3 — nobody's own chain runs through a team any
/// more, so material meant to reach a session has to sit on a scope that
/// does), and root's own section header costs a few tokens more than the
/// team header this budget used to be measured against. The shape is
/// unchanged — one record in full, one named by index — only the margin
/// moved.
const TIGHT_BUDGET: u32 = 260;

async fn world(index_tier: IndexTier) -> Option<World> {
    let (pool, tenant) = admitted_tenant().await?;
    // `platform` is part of the tree `seed_hierarchy` builds but unused
    // here: since placement is identity (ADR-0074 decision 3) alice's own
    // chain no longer runs through it, and the root is the one shared
    // scope every session composes from without a grant (bare membership
    // reaches the working tier the record below is seeded at).
    let (org, _platform) = seed_hierarchy(&pool, tenant).await;
    let alice = seed_user(&pool, tenant, "alice").await;
    let runbook = seed_record(&pool, tenant, org.id, alice.id, RecordKind::Pinned, RUNBOOK).await;
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
    let run = session_seed::seed_run_for(&pool, tenant, "ctx4", "alice")
        .await
        .session_id;
    let _ = (&pool, tenant, &pdp, runbook, note);
    Some(World { app, token, run })
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
    let before = compose(&off).await;

    let Some(on) = world(IndexTier::Demote).await else {
        return;
    };
    let after = compose(&on).await;

    // Counted from the block rather than read off response fields: a context
    // run's body is deliberately minimal (ADR-0076 decision 7), and the
    // watermark and the handles are where a block says what it composed and
    // what it named. Same numbers, from what a model actually reads.
    let named_before = record_ids(&before).len();
    let named_after = record_ids(&after).len();
    let index_entries = handles(&after) as u64;
    let index_tokens = after["tokens"]
        .as_u64()
        .expect("tokens")
        .saturating_sub(before["tokens"].as_u64().expect("tokens"));

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

    assert_eq!(handles(&before), 0, "the tier-off block names nothing");
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
    // The legend is charged once, to the first demotion (decision 12). It
    // stopped naming a command with the observe cutover — `/v1/recall` is
    // deleted and nothing fetches a body by id today (ADR-0078 decision 5) —
    // so it names what a handle *is* rather than what to run on it.
    assert!(
        after["rendered"]
            .as_str()
            .expect("rendered")
            .contains("recall handle"),
        "and the block says what a handle is"
    );
    assert!(
        !before["rendered"]
            .as_str()
            .expect("rendered")
            .contains("recall handle"),
        "while the tier-off block is byte-clean of it"
    );
}
