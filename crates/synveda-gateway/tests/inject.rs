//! CTX-3 acceptance criteria (ADR-0026): the inject route's degradation
//! modes — embedder down composes sparse-only-ranked with the warning
//! header, a broken sidecar composes unranked with the warning header,
//! and only a missing store is a real failure — plus the surrounding
//! contract: the full product path (plan → embed → hybrid → compose)
//! returns a relevance-ranked, watermarked block and chains exactly one
//! `context.injected` event with the per-scope decisions aggregated and
//! no task text anywhere; a taskless inject composes recency-ordered; a
//! quarantined or unplaced caller receives the empty block (200, still
//! audited — policy outcomes are results, not errors); a request budget
//! narrows and never widens; and a bank-mode pack governs the very next
//! inject at this seam (the ADR-0014 promise, re-demonstrated
//! end-to-end).
//!
//! The latency AC lives in tests/inject_latency.rs (`--ignored`).
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
use synveda_audit::{ChainVerification, StoredEvent};
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::{Hs256Verifier, personal_slug};
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder, Embedder as _, TeiEmbedder};
use synveda_policy::Pdp;
use synveda_retrieval::index::SearchIndex;
use synveda_retrieval::indexer::{self, IndexerConfig};
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::{hierarchy, identities, policy_assignments, rls, tenants};
use synveda_types::{
    Channel, CompositionConfig, HierarchyNode, Identity, IdentityId, IdentityKind, InjectChannels,
    PackConfig, RecordClass, RecordId, RecordKind, ScopeId, ScopeKind, Sensitivity, TenantId,
    TenantStatus,
};
use synveda_vedaflow::{
    self as vedaflow, ChannelRef, ChannelWrite, MemoryAsset, PolicySnapshot, Signer,
};
use tower::ServiceExt;

const SECRET: &[u8] = b"ctx-3-test-secret";

/// A permissive stored-pack source for the bank-mode test: every grant
/// comes from this permit; the compiled base layer still rides along
/// (the PDP is never bypassed — this is a pack, not a hole).
const BLANKET: &str = "permit (principal, action, resource) when { resource in principal.tenant };";

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

/// A fresh sidecar root per call — Tantivy writers lock per directory.
fn index_root() -> std::path::PathBuf {
    std::env::temp_dir()
        .join("synveda-ctx3-tests")
        .join(TenantId::new().to_string())
}

fn state_with(url: &str, search_index: Arc<SearchIndex>, embedder: AnyEmbedder) -> AppState {
    AppState {
        pool: PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(5))
            .connect_lazy(url)
            .expect("parse database url"),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        pdp: Arc::new(Pdp::new().expect("build the embedded PDP")),
        scope_chains: Arc::new(synveda_store::ScopeChainCache::new()),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index,
        embedder: Arc::new(embedder),
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
                "skipping CTX-3 inject test: DATABASE_URL is not set \
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
    let slug = format!("ctx3-{}", id.as_uuid().simple());
    tenants::create(&pool, id, &slug, "CTX-3 test tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((pool, id))
}

fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("checked by admitted_tenant")
}

/// Seeds acme-org → eng (dept) → platform (team), plus the reserved
/// quarantine team.
async fn seed_hierarchy(
    pool: &PgPool,
    tenant: TenantId,
) -> (HierarchyNode, HierarchyNode, HierarchyNode) {
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
    let eng = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(org.id),
        ScopeKind::Department,
        "eng",
        "Engineering",
    )
    .await
    .expect("create dept");
    let platform = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(eng.id),
        ScopeKind::Team,
        "platform",
        "Platform",
    )
    .await
    .expect("create team");
    let quarantine = hierarchy::create(
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
    (org, platform, quarantine)
}

/// Provisions a user identity at the store level (the JIT shape).
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
        subject,
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

/// Inserts one record with its deterministic-model vector (the MEM-4
/// one-statement API — a record cannot exist without its embedding).
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
            class: RecordClass::Fact,
            content: content.to_owned(),
            sensitivity: Sensitivity::Internal,
            provenance: json!({"source": "ctx-3 test fixture"}),
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

/// Converges the sidecar so the sparse leg sees everything seeded.
async fn sweep(pool: &PgPool, index: &SearchIndex, tenant: TenantId) {
    let config = IndexerConfig {
        overlap: Duration::ZERO,
        ..IndexerConfig::default()
    };
    indexer::sweep_tenant(pool, index, tenant, &config)
        .await
        .expect("sweep sidecar");
}

async fn inject(app: &Router, token: &str, body: Value) -> (StatusCode, Option<String>, Value) {
    let request = Request::builder()
        .method("POST")
        .uri("/v1/inject")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build request");
    let response = app.clone().oneshot(request).await.expect("send request");
    let status = response.status();
    let degraded_header = response
        .headers()
        .get("x-synveda-degraded")
        .map(|value| value.to_str().expect("header utf8").to_owned());
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
    (status, degraded_header, value)
}

/// The tenant's whole chain, oldest first, plus its verification.
async fn chain(pool: &PgPool, tenant_id: TenantId) -> (Vec<StoredEvent>, ChainVerification) {
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

/// Fixture: alice under platform, one pinned + two derived at the team,
/// one derived at alice's personal scope. Contents are lexically
/// disjoint so BM25 relevance is deterministic.
struct Corpus {
    pinned_team: RecordId,
    kube_team: RecordId,
    postgres_team: RecordId,
    note_user: RecordId,
}

const PINNED_CONTENT: &str = "All production deployments require a change review sign-off.";
const KUBE_CONTENT: &str = "Kubernetes rollout runbook: drain nodes before upgrading the fleet.";
const POSTGRES_CONTENT: &str = "Vacuum maintenance procedure: analyze bloated relations weekly.";
const NOTE_CONTENT: &str = "Alice prefers rebase-and-merge for her feature branches.";

async fn seed_corpus(pool: &PgPool, tenant: TenantId, platform: &HierarchyNode) -> Corpus {
    let alice = seed_user(pool, tenant, "alice", platform.id).await;
    let owner = alice.id;
    let pinned_team = seed_record(
        pool,
        tenant,
        platform.id,
        owner,
        RecordKind::Pinned,
        PINNED_CONTENT,
    )
    .await;
    let kube_team = seed_record(
        pool,
        tenant,
        platform.id,
        owner,
        RecordKind::Derived,
        KUBE_CONTENT,
    )
    .await;
    let postgres_team = seed_record(
        pool,
        tenant,
        platform.id,
        owner,
        RecordKind::Derived,
        POSTGRES_CONTENT,
    )
    .await;
    let note_user = seed_record(
        pool,
        tenant,
        alice.scope_id,
        owner,
        RecordKind::Derived,
        NOTE_CONTENT,
    )
    .await;
    // Since FLOW-2 (ADR-0031) authorship is not review: the canonical
    // team record is trusted material only once someone publishes it.
    // The governed route is exercised in tests/channels.rs; this is the
    // fixture form, with the same standing `seed_record` has.
    publish_fixture(pool, tenant, platform.id, &[pinned_team]).await;
    Corpus {
        pinned_team,
        kube_team,
        postgres_team,
        note_user,
    }
}

/// Publishes records onto a scope's `memory/published` channel.
async fn publish_fixture(pool: &PgPool, tenant: TenantId, scope: ScopeId, ids: &[RecordId]) {
    let mut tx = rls::begin_tenant_tx(pool, tenant).await.expect("tenant tx");
    let mut members = Vec::with_capacity(ids.len());
    for id in ids {
        let version = records::current(&mut *tx, *id)
            .await
            .expect("read record")
            .expect("record exists");
        let asset = MemoryAsset {
            id: version.id,
            scope_id: version.state.scope_id,
            owner_id: version.state.owner_id,
            kind: version.state.kind,
            class: version.state.class,
            content: version.state.content.clone(),
            sensitivity: version.state.sensitivity,
            valid_from: version.state.valid_from,
            valid_to: version.state.valid_to,
        };
        let object = vedaflow::put_memory(&mut tx, tenant, &asset)
            .await
            .expect("put memory object");
        members.push((asset.entry_name(), object.hash));
    }
    vedaflow::publish(
        &mut tx,
        tenant,
        &ChannelWrite {
            scope,
            channel: ChannelRef::memory(Channel::Published),
            members: &members,
            merge_parents: &[],
            author: IdentityId::new(),
            message: "ctx-3 fixture publication",
            committed_at: chrono::Utc::now(),
            policy_snapshot: &PolicySnapshot::new("regulated-strict", 6),
        },
        &Signer::Unsigned,
    )
    .await
    .expect("publish");
    tx.commit().await.expect("commit publication");
}

fn record_ids(body: &Value) -> Vec<String> {
    body["record_ids"]
        .as_array()
        .expect("record_ids array")
        .iter()
        .map(|id| id.as_str().expect("record id string").to_owned())
        .collect()
}

/// The full product path: a task over a converged corpus composes a
/// relevance-ranked, watermarked block — undegraded — and chains
/// exactly one `context.injected` event carrying the watermark, the
/// aggregated per-scope decisions, and a task hash (never task text).
#[tokio::test]
async fn inject_composes_ranked_watermarked_block_and_chains_one_event() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (_, platform, _) = seed_hierarchy(&pool, tenant).await;
    let corpus = seed_corpus(&pool, tenant, &platform).await;
    let index = Arc::new(SearchIndex::open(index_root()).expect("open sidecar"));
    sweep(&pool, &index, tenant).await;
    let app = router(state_with(
        &database_url(),
        index,
        AnyEmbedder::Deterministic(DeterministicEmbedder::new()),
    ));

    let task = "kubernetes rollout upgrade";
    let (status, degraded_header, body) = inject(
        &app,
        &issue("alice", tenant),
        json!({"task": task, "session_id": "sess-1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "inject must succeed: {body}");
    assert_eq!(degraded_header, None, "full path is undegraded");
    assert_eq!(body["degraded"], json!([]));

    let text = body["text"].as_str().expect("block text");
    assert!(
        text.contains(PINNED_CONTENT),
        "pinned material composes regardless of the task: {text}"
    );
    assert!(
        text.contains(KUBE_CONTENT),
        "the task-relevant derived record composes: {text}"
    );
    assert!(
        text.contains(NOTE_CONTENT),
        "the personal-scope derived record composes (ranked within 64): {text}"
    );
    // Relevance ordering (ADR-0025 decision 5): within the team scope,
    // the sparse-matched record ranks above the lexically-unrelated one.
    let kube_at = text.find(KUBE_CONTENT).expect("kube in text");
    let postgres_at = text.find(POSTGRES_CONTENT).expect("postgres in text");
    assert!(
        kube_at < postgres_at,
        "relevance must rank the matched record first"
    );

    // The watermark (ADR-0025 decision 7): block hash and record ids
    // ride the rendered text; the response metadata matches.
    let block_hash = body["block_hash"].as_str().expect("block hash");
    assert!(
        text.contains(block_hash),
        "the block hash rides the watermark line"
    );
    let ids = record_ids(&body);
    for id in [&corpus.pinned_team, &corpus.kube_team, &corpus.note_user] {
        assert!(ids.contains(&id.to_string()), "{id} missing from watermark");
    }
    let tokens = body["tokens"].as_u64().expect("tokens");
    let budget = body["budget_tokens"].as_u64().expect("budget");
    assert!(tokens <= budget, "tokens {tokens} within budget {budget}");
    assert_eq!(
        budget,
        u64::from(CompositionConfig::DEFAULT.budget_tokens),
        "the product default budget in force"
    );
    assert!(body["as_of"].is_string(), "the instant is echoed");

    // Exactly one chained event, and the chain verifies (AUD-1).
    let (events, verification) = chain(&pool, tenant).await;
    let injected: Vec<&StoredEvent> = events
        .iter()
        .filter(|event| event.action == "context.injected")
        .collect();
    assert_eq!(
        injected.len(),
        1,
        "one event per inject, never per candidate"
    );
    assert_eq!(verification, ChainVerification::Valid { events: 1 });
    let event = injected[0];
    assert_eq!(event.actor_subject, "alice");
    assert_eq!(event.outcome, "success");
    assert_eq!(event.payload["block_hash"], json!(block_hash));
    assert_eq!(event.payload["session_id"], json!("sess-1"));
    assert_eq!(event.payload["degraded"], json!([]));
    assert_eq!(
        event.payload["task_hash"],
        json!(blake3_hex(task)),
        "the task rides as a hash"
    );
    assert!(
        !event.payload.to_string().contains("kubernetes"),
        "no task text in the audit payload"
    );
    // The full per-entry watermark, and the aggregated decisions for
    // the whole chain (leaf → team → dept → org, all allowed).
    let entries = event.payload["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), ids.len());
    for entry in entries {
        // The VedaFlow object address of the version that composed, and
        // the channel it composed from (FLOW-2, ADR-0031 decision 11).
        assert!(
            entry["object_hash"]
                .as_str()
                .is_some_and(|hash| hash.len() == 64)
        );
        assert!(
            ["published", "derived"].contains(&entry["channel"].as_str().expect("channel label"))
        );
    }
    // And the published channel each scope was read at — tech plan
    // §2.5's "inject responses cite commit hashes", in the audit event
    // rather than the token budget.
    let channels = event.payload["channels"].as_array().expect("channels");
    assert!(
        channels
            .iter()
            .any(|channel| channel["ref"] == json!("memory/published")
                && channel["commit"]
                    .as_str()
                    .is_some_and(|commit| commit.len() == 64)),
        "the published channel commit is cited: {channels:?}"
    );
    let decisions = event.payload["decisions"].as_array().expect("decisions");
    assert_eq!(decisions.len(), 4, "one decision per chain scope");
    for decision in decisions {
        assert_eq!(decision["allowed"], json!(true));
        assert!(
            decision["pack"]
                .as_str()
                .is_some_and(|pack| pack.contains('@')),
            "pack@version on every decision"
        );
    }
}

fn blake3_hex(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex().to_string()
}

/// No task: no retrieval leg runs, and derived composes recency-ordered
/// — by design, not a degradation.
#[tokio::test]
async fn taskless_inject_composes_recency_ordered_without_retrieval() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (_, platform, _) = seed_hierarchy(&pool, tenant).await;
    let corpus = seed_corpus(&pool, tenant, &platform).await;
    // Deliberately no sweep: the sidecar is cold, and it must not matter.
    let index = Arc::new(SearchIndex::open(index_root()).expect("open sidecar"));
    let app = router(state_with(
        &database_url(),
        index,
        AnyEmbedder::Deterministic(DeterministicEmbedder::new()),
    ));

    let (status, degraded_header, body) = inject(&app, &issue("alice", tenant), json!({})).await;
    assert_eq!(status, StatusCode::OK, "taskless inject succeeds: {body}");
    assert_eq!(degraded_header, None, "taskless is not a degradation");
    assert_eq!(body["degraded"], json!([]));
    let ids = record_ids(&body);
    for id in [
        &corpus.pinned_team,
        &corpus.kube_team,
        &corpus.postgres_team,
        &corpus.note_user,
    ] {
        assert!(
            ids.contains(&id.to_string()),
            "without a ranking every readable record composes: {id}"
        );
    }
    let text = body["text"].as_str().expect("block text");
    // Recency within the team scope: postgres was seeded after kube.
    let kube_at = text.find(KUBE_CONTENT).expect("kube in text");
    let postgres_at = text.find(POSTGRES_CONTENT).expect("postgres in text");
    assert!(
        postgres_at < kube_at,
        "unranked derived orders newest valid-from first"
    );
}

/// Embedder down: the dense leg drops, the sparse leg still ranks, the
/// response carries the warning header — 200, never a failure
/// (ADR-0026 decision 4a).
#[tokio::test]
async fn embedder_down_degrades_to_sparse_only_with_warning_header() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (_, platform, _) = seed_hierarchy(&pool, tenant).await;
    let corpus = seed_corpus(&pool, tenant, &platform).await;
    let index = Arc::new(SearchIndex::open(index_root()).expect("open sidecar"));
    sweep(&pool, &index, tenant).await;
    // A TEI endpoint that refuses connections: bind, take the port, drop.
    let dead_port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.local_addr().expect("addr").port()
    };
    let app = router(state_with(
        &database_url(),
        index,
        AnyEmbedder::Tei(TeiEmbedder::new(
            TeiEmbedder::DEFAULT_MODEL.to_owned(),
            format!("http://127.0.0.1:{dead_port}"),
        )),
    ));

    let (status, degraded_header, body) = inject(
        &app,
        &issue("alice", tenant),
        json!({"task": "kubernetes rollout upgrade"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "degrades, never fails: {body}");
    assert_eq!(degraded_header.as_deref(), Some("embedder"));
    assert_eq!(body["degraded"], json!(["embedder"]));
    let ids = record_ids(&body);
    assert!(
        ids.contains(&corpus.kube_team.to_string()),
        "the sparse leg still ranks the matched record"
    );
    assert!(
        !ids.contains(&corpus.postgres_team.to_string()),
        "an unmatched derived record does not compose under a ranking"
    );
    assert!(
        ids.contains(&corpus.pinned_team.to_string()),
        "published material never depends on retrieval"
    );

    // The audit event records the degradation.
    let (events, _) = chain(&pool, tenant).await;
    let event = events
        .iter()
        .find(|event| event.action == "context.injected")
        .expect("chained inject event");
    assert_eq!(event.payload["degraded"], json!(["embedder"]));
}

/// A broken sidecar (corrupt Tantivy meta behind a valid state file):
/// retrieval errors, the block composes unranked — pinned plus
/// recency-ordered derived — with the warning header (decision 4b).
#[tokio::test]
async fn broken_sidecar_composes_unranked_with_warning_header() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (_, platform, _) = seed_hierarchy(&pool, tenant).await;
    let corpus = seed_corpus(&pool, tenant, &platform).await;
    let root = index_root();
    // Converge with a scratch manager, then corrupt the tenant's index
    // behind its valid state file: the gateway's fresh manager will
    // attempt the open and error — not heal (healing is the indexer's
    // write path, never the read path's).
    let scratch = SearchIndex::open(&root).expect("open sidecar");
    sweep(&pool, &scratch, tenant).await;
    drop(scratch);
    std::fs::write(root.join(tenant.to_string()).join("meta.json"), b"garbage")
        .expect("corrupt meta.json");
    let index = Arc::new(SearchIndex::open(&root).expect("open manager"));
    let app = router(state_with(
        &database_url(),
        index,
        AnyEmbedder::Deterministic(DeterministicEmbedder::new()),
    ));

    let (status, degraded_header, body) = inject(
        &app,
        &issue("alice", tenant),
        json!({"task": "kubernetes rollout upgrade"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "degrades, never fails: {body}");
    assert_eq!(degraded_header.as_deref(), Some("retrieval"));
    assert_eq!(body["degraded"], json!(["retrieval"]));
    let ids = record_ids(&body);
    for id in [
        &corpus.pinned_team,
        &corpus.kube_team,
        &corpus.postgres_team,
    ] {
        assert!(
            ids.contains(&id.to_string()),
            "unranked compose still serves the readable corpus: {id}"
        );
    }
}

/// Quarantined and unplaced callers receive the empty block — 200,
/// watermarked-empty, still audited with the real reason in the
/// decisions (decision 1): the inject surface is not a placement oracle.
#[tokio::test]
async fn quarantined_and_unplaced_callers_receive_the_empty_block() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (_, _, quarantine) = seed_hierarchy(&pool, tenant).await;
    seed_user(&pool, tenant, "mallory", quarantine.id).await;
    let index = Arc::new(SearchIndex::open(index_root()).expect("open sidecar"));
    let app = router(state_with(
        &database_url(),
        index,
        AnyEmbedder::Deterministic(DeterministicEmbedder::new()),
    ));

    // Quarantined: a placement chain exists, every MemoryRead denies.
    let (status, degraded_header, body) = inject(
        &app,
        &issue("mallory", tenant),
        json!({"task": "anything at all"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "empty, not an error: {body}");
    assert_eq!(degraded_header, None, "policy is not a degradation");
    assert_eq!(body["record_ids"], json!([]));
    assert_eq!(body["tokens"], json!(0));
    let (events, _) = chain(&pool, tenant).await;
    let event = events
        .iter()
        .find(|event| event.action == "context.injected" && event.actor_subject == "mallory")
        .expect("the empty inject still chains");
    let decisions = event.payload["decisions"].as_array().expect("decisions");
    assert!(!decisions.is_empty(), "the chain walk was decided");
    assert!(
        decisions
            .iter()
            .all(|decision| decision["allowed"] == json!(false)),
        "every scope denied for a quarantined caller"
    );

    // Unplaced: no identity row at all — empty block, zero decisions.
    let (status, _, body) =
        inject(&app, &issue("ghost", tenant), json!({"task": "anything"})).await;
    assert_eq!(status, StatusCode::OK, "unplaced is empty, not 403: {body}");
    assert_eq!(body["record_ids"], json!([]));
}

/// A request budget narrows the pack budget and never widens it
/// (decision 7).
#[tokio::test]
async fn request_budget_narrows_never_widens() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (_, platform, _) = seed_hierarchy(&pool, tenant).await;
    seed_corpus(&pool, tenant, &platform).await;
    let index = Arc::new(SearchIndex::open(index_root()).expect("open sidecar"));
    let app = router(state_with(
        &database_url(),
        index,
        AnyEmbedder::Deterministic(DeterministicEmbedder::new()),
    ));
    let token = issue("alice", tenant);

    let (status, _, narrowed) = inject(&app, &token, json!({"budget_tokens": 40})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(narrowed["budget_tokens"], json!(40));
    assert!(narrowed["tokens"].as_u64().expect("tokens") <= 40);

    let (status, _, widened) = inject(&app, &token, json!({"budget_tokens": 999_999})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        widened["budget_tokens"],
        json!(CompositionConfig::DEFAULT.budget_tokens),
        "the pack budget is the ceiling"
    );
}

/// The bank-mode switch at the inject seam: a `published-only` pack set
/// as the tenant default governs the very next inject — derived
/// vanishes, the scope's published channel survives (ADR-0026
/// decision 8; the ADR-0014 composition promise, end-to-end). The
/// FLOW-2 acceptance criterion over the governed publish route lives in
/// tests/channels.rs.
#[tokio::test]
async fn bank_mode_pack_governs_the_very_next_inject() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (_, platform, _) = seed_hierarchy(&pool, tenant).await;
    let corpus = seed_corpus(&pool, tenant, &platform).await;
    let index = Arc::new(SearchIndex::open(index_root()).expect("open sidecar"));
    let state = state_with(
        &database_url(),
        index,
        AnyEmbedder::Deterministic(DeterministicEmbedder::new()),
    );
    let pdp = Arc::clone(&state.pdp);
    let app = router(state);
    let token = issue("alice", tenant);

    // Before the flip: both channels compose.
    let (_, _, before) = inject(&app, &token, json!({})).await;
    assert!(record_ids(&before).contains(&corpus.kube_team.to_string()));

    // The flip: install the bank pack (what the refresher does for a
    // stored pack) and make it the tenant default.
    pdp.install_source(
        tenant,
        "bank",
        1,
        BLANKET,
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

    // The very next inject composes published-only.
    let (status, _, after) = inject(&app, &token, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let ids = record_ids(&after);
    assert!(
        ids.contains(&corpus.pinned_team.to_string()),
        "the team's published record survives bank mode"
    );
    for derived in [&corpus.kube_team, &corpus.postgres_team, &corpus.note_user] {
        assert!(
            !ids.contains(&derived.to_string()),
            "derived must not compose under published-only: {derived}"
        );
    }
}

/// Contract rejections stay honest errors: malformed input is 400,
/// missing auth is 401 — no partial-context masking (decision 4c is
/// only for dependencies, never for the caller's own mistakes).
#[tokio::test]
async fn contract_rejections_are_not_degraded() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (_, platform, _) = seed_hierarchy(&pool, tenant).await;
    seed_user(&pool, tenant, "alice", platform.id).await;
    let index = Arc::new(SearchIndex::open(index_root()).expect("open sidecar"));
    let app = router(state_with(
        &database_url(),
        index,
        AnyEmbedder::Deterministic(DeterministicEmbedder::new()),
    ));

    let (status, degraded_header, _) =
        inject(&app, &issue("alice", tenant), json!({"task": ""})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "an empty task is invalid");
    assert_eq!(degraded_header, None);

    let (status, _, _) = inject(&app, "not-a-token", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    drop(pool);
}
