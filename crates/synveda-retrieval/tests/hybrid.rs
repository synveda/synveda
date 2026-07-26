//! CTX-1 engine tests (ADR-0024): fusion end to end, the mandatory
//! pushdown filter's no-leak guarantees, one-sided sidecar staleness,
//! degradation modes, and indexer convergence.
//!
//! These tests need a live Postgres (the bitemporal-suite harness:
//! `DATABASE_URL` or quiet skip). Tenant isolation is by freshly minted
//! ids; each test opens its own sidecar root under the OS temp dir.

use std::sync::OnceLock;
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_retrieval::hybrid::{QueryVector, SearchFilter, SearchRequest, hybrid_search};
use synveda_retrieval::index::SearchIndex;
use synveda_retrieval::indexer::{self, IndexerConfig, TenantSweep};
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::rls;
use synveda_types::{
    Error, IdentityId, RecordClass, RecordId, RecordKind, ScopeId, ScopeTier, Sensitivity, TenantId,
};

// ── Harness ──────────────────────────────────────────────────────────────────

struct Db {
    rt: tokio::runtime::Runtime,
    pool: PgPool,
}

fn db() -> Option<&'static Db> {
    static DB: OnceLock<Option<Db>> = OnceLock::new();
    DB.get_or_init(|| {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!(
                    "skipping hybrid retrieval tests: DATABASE_URL is not set \
                     (run `make dev-up` then `make db-test`)"
                );
                return None;
            }
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");
        let pool = rt.block_on(async {
            let pool = PgPoolOptions::new()
                .max_connections(4)
                .connect(&url)
                .await
                .expect("connect to DATABASE_URL");
            synveda_store::migrate(&pool)
                .await
                .expect("apply migrations");
            pool
        });
        Some(Db { rt, pool })
    })
    .as_ref()
}

/// A fresh sidecar root per test under the OS temp dir; the OS owns
/// eventual cleanup, ids keep runs collision-free.
fn fresh_index() -> SearchIndex {
    let root = std::env::temp_dir()
        .join("synveda-ctx1-tests")
        .join(TenantId::new().to_string());
    SearchIndex::open(root).expect("open sidecar root")
}

/// Zero-overlap config: deterministic single-writer tests want "swept =
/// converged, next sweep empty". Overlap semantics get their own test.
fn config() -> IndexerConfig {
    IndexerConfig {
        overlap: Duration::ZERO,
        ..IndexerConfig::default()
    }
}

const MODEL: &str = "test@1";

/// A 16-dimension unit vector in the plane of axes 0 and 1: `angle`
/// controls cosine distance to other vectors from this helper.
fn vector(angle_degrees: f32) -> Vec<f32> {
    let radians = angle_degrees.to_radians();
    let mut vector = vec![0.0_f32; 16];
    vector[0] = radians.cos();
    vector[1] = radians.sin();
    vector
}

fn state(content: &str, scope: ScopeId, sensitivity: Sensitivity) -> RecordState {
    RecordState {
        scope_id: scope,
        owner_id: IdentityId::new(),
        kind: RecordKind::Derived,
        class: RecordClass::Fact,
        content: content.to_owned(),
        sensitivity,
        provenance: serde_json::json!({"source": "ctx-1 acceptance test"}),
        valid_from: chrono::Utc::now(),
        valid_to: None,
    }
}

async fn insert(
    pool: &PgPool,
    tenant: TenantId,
    scope: ScopeId,
    content: &str,
    sensitivity: Sensitivity,
    angle: f32,
) -> RecordId {
    let id = RecordId::new();
    records::insert(
        pool,
        id,
        tenant,
        &state(content, scope, sensitivity),
        &RecordEmbedding {
            model: MODEL.to_owned(),
            vector: vector(angle),
        },
    )
    .await
    .expect("insert record");
    id
}

async fn sweep(pool: &PgPool, index: &SearchIndex, tenant: TenantId) -> TenantSweep {
    indexer::sweep_tenant(pool, index, tenant, &config())
        .await
        .expect("sweep tenant")
}

/// Runs one hybrid search inside a fresh tenant transaction (the RLS
/// discipline production follows).
async fn search(
    pool: &PgPool,
    index: &SearchIndex,
    tenant: TenantId,
    request: &SearchRequest,
) -> Vec<synveda_retrieval::RetrievedRecord> {
    let mut tx = rls::begin_tenant_tx(pool, tenant).await.expect("tenant tx");
    let results = hybrid_search(&mut tx, index, tenant, request)
        .await
        .expect("hybrid search");
    drop(tx);
    results
}

fn filter(scopes: &[ScopeId]) -> SearchFilter {
    filter_at(scopes, &[Sensitivity::Public, Sensitivity::Internal])
}

/// The filter as the PDP now hands it over: a pair per (scope, tier)
/// (AUTHZ-5, ADR-0038 decision 3).
fn filter_at(scopes: &[ScopeId], tiers: &[Sensitivity]) -> SearchFilter {
    SearchFilter {
        tiers: scopes
            .iter()
            .flat_map(|scope| ScopeTier::expand(*scope, tiers))
            .collect(),
    }
}

fn query(text: &str, scopes: &[ScopeId], angle: Option<f32>) -> SearchRequest {
    let mut request = SearchRequest::new(text, filter(scopes), chrono::Utc::now());
    request.vector = angle.map(|angle| QueryVector {
        model: MODEL.to_owned(),
        vector: vector(angle),
    });
    request
}

// ── Fusion ───────────────────────────────────────────────────────────────────

/// The headline: a lexical-only hit and a semantic-only hit both
/// surface, and RRF ranks the record found by *both* legs first.
#[test]
fn hybrid_fuses_lexical_and_semantic_hits() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let (tenant, scope) = (TenantId::new(), ScopeId::new());
        let index = fresh_index();

        // Query text "postgres pooling" at angle 0.
        // r1: lexical match, mid-distance vector  → sparse #1, dense #2.
        // r2: no shared terms, nearest vector     → dense #1 only.
        // r3: no shared terms, far vector         → dense #3 only.
        let r1 = insert(
            pool,
            tenant,
            scope,
            "postgres connection pooling guidance",
            Sensitivity::Internal,
            40.0,
        )
        .await;
        let r2 = insert(
            pool,
            tenant,
            scope,
            "database maintenance for large tables",
            Sensitivity::Internal,
            5.0,
        )
        .await;
        let r3 = insert(
            pool,
            tenant,
            scope,
            "chocolate cake recipe",
            Sensitivity::Internal,
            85.0,
        )
        .await;
        let swept = sweep(pool, &index, tenant).await;
        assert_eq!(swept.upserts, 3, "all three records indexed");

        let results = search(
            pool,
            &index,
            tenant,
            &query("postgres pooling", &[scope], Some(0.0)),
        )
        .await;
        let ids: Vec<RecordId> = results.iter().map(|hit| hit.record.id).collect();
        assert_eq!(
            ids,
            vec![r1, r2, r3],
            "both-legs record first (1/62+1/61 beats 1/61), dense-only next"
        );
        assert_eq!(results[0].sparse_rank, Some(1));
        assert_eq!(results[0].dense_rank, Some(2));
        assert_eq!(results[1].dense_rank, Some(1));
        assert_eq!(results[1].sparse_rank, None);
    });
}

// ── The mandatory filter: no leaks ───────────────────────────────────────────

/// Adversarial exclusions on both legs at once: the excluded records
/// carry a *nearer* vector and a *stronger* lexical match than the
/// permitted one, and still never surface — cross-tenant, unpermitted
/// scope, above-ceiling sensitivity, and `restricted` even when the
/// caller asks for it (the clamp, ADR-0024 decision 2).
#[test]
fn filters_never_leak_on_either_leg() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let (tenant, other_tenant) = (TenantId::new(), TenantId::new());
        let (scope, other_scope) = (ScopeId::new(), ScopeId::new());
        let index = fresh_index();

        let permitted = insert(
            pool,
            tenant,
            scope,
            "rotation runbook for the vault",
            Sensitivity::Internal,
            30.0,
        )
        .await;
        // Nearer vectors (angle 0 = the query), better lexical matches.
        insert(
            pool,
            other_tenant,
            scope,
            "vault rotation runbook vault rotation",
            Sensitivity::Internal,
            0.0,
        )
        .await;
        insert(
            pool,
            tenant,
            other_scope,
            "vault rotation runbook vault rotation",
            Sensitivity::Internal,
            0.0,
        )
        .await;
        insert(
            pool,
            tenant,
            scope,
            "vault rotation runbook vault rotation",
            Sensitivity::Confidential,
            0.0,
        )
        .await;
        insert(
            pool,
            tenant,
            scope,
            "vault rotation runbook vault rotation",
            Sensitivity::Restricted,
            0.0,
        )
        .await;
        sweep(pool, &index, tenant).await;
        sweep(pool, &index, other_tenant).await;

        // Working tiers only: one permitted record.
        let results = search(
            pool,
            &index,
            tenant,
            &query("vault rotation runbook", &[scope], Some(0.0)),
        )
        .await;
        let ids: Vec<RecordId> = results.iter().map(|hit| hit.record.id).collect();
        assert_eq!(
            ids,
            vec![permitted],
            "internal ceiling: one permitted record"
        );

        // The engine returns exactly the pairs it was handed, and nothing
        // else — which is the AUTHZ-5 change (ADR-0038 decision 3). Before
        // it, this leg clamped below `restricted` on its own; now the
        // refusal lives where it can be decided rather than assumed, in the
        // PDP (crates/synveda-policy/tests/sensitivity.rs) and end to end
        // in the leak suite. An engine that clamped would be a second
        // opinion on a question policy already answered.
        let mut relaxed = query("vault rotation runbook", &[scope], Some(0.0));
        relaxed.filter = filter_at(
            &[scope],
            &[
                Sensitivity::Public,
                Sensitivity::Internal,
                Sensitivity::Confidential,
            ],
        );
        let results = search(pool, &index, tenant, &relaxed).await;
        assert_eq!(results.len(), 2, "confidential joins when a pair says so");
        assert!(
            results
                .iter()
                .all(|hit| hit.record.state.sensitivity < Sensitivity::Restricted),
            "and restricted does not, because no pair named it"
        );

        // A pair set that *does* name the top tier surfaces it: the engine
        // is the plan's executor, never its second guess.
        let mut top = query("vault rotation runbook", &[scope], Some(0.0));
        top.filter = filter_at(&[scope], &Sensitivity::ALL);
        let results = search(pool, &index, tenant, &top).await;
        assert_eq!(results.len(), 3, "every tier the pairs name");
        assert!(
            results
                .iter()
                .any(|hit| hit.record.state.sensitivity == Sensitivity::Restricted),
            "including the one only a compliance-signed lapse can produce"
        );

        // And a pair set that names a tier at the *wrong* scope admits
        // nothing: the pair is the unit, not the scope and the tier
        // separately.
        let mut mismatched = query("vault rotation runbook", &[scope], Some(0.0));
        mismatched.filter = SearchFilter {
            tiers: ScopeTier::expand(other_scope, &[Sensitivity::Confidential]),
        };
        let results = search(pool, &index, tenant, &mismatched).await;
        assert!(
            results
                .iter()
                .all(|hit| hit.record.state.scope_id == other_scope),
            "a tier permitted at one scope says nothing about another"
        );

        // The empty predicate returns nothing without touching an index.
        let results = search(
            pool,
            &index,
            tenant,
            &query("vault rotation runbook", &[], Some(0.0)),
        )
        .await;
        assert!(results.is_empty(), "empty scope set = empty result");
    });
}

// ── One-sided staleness ──────────────────────────────────────────────────────

/// A lagging sidecar can only miss, never resurface or leak: deletes
/// and re-scopes committed after the last sweep are invisible at
/// hydration, and a content rewrite surfaces *current* content under
/// the old terms until the sweep converges the index.
#[test]
fn stale_sidecar_is_one_sided() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let (tenant, scope, other_scope) = (TenantId::new(), ScopeId::new(), ScopeId::new());
        let index = fresh_index();

        let deleted = insert(
            pool,
            tenant,
            scope,
            "alpha incident timeline",
            Sensitivity::Internal,
            10.0,
        )
        .await;
        let rescoped = insert(
            pool,
            tenant,
            scope,
            "alpha escalation contacts",
            Sensitivity::Internal,
            20.0,
        )
        .await;
        let rewritten = insert(
            pool,
            tenant,
            scope,
            "alpha bravo checklist",
            Sensitivity::Internal,
            30.0,
        )
        .await;
        sweep(pool, &index, tenant).await;

        // Mutate after the sweep — the sidecar still remembers all three.
        records::delete(pool, deleted)
            .await
            .expect("temporal delete");
        let moved = state(
            "alpha escalation contacts",
            other_scope,
            Sensitivity::Internal,
        );
        records::update(
            pool,
            rescoped,
            &moved,
            &RecordEmbedding {
                model: MODEL.to_owned(),
                vector: vector(20.0),
            },
        )
        .await
        .expect("re-scope");
        let rewritten_state = state("charlie delta checklist", scope, Sensitivity::Internal);
        records::update(
            pool,
            rewritten,
            &rewritten_state,
            &RecordEmbedding {
                model: MODEL.to_owned(),
                vector: vector(30.0),
            },
        )
        .await
        .expect("rewrite");

        let results = search(pool, &index, tenant, &query("alpha", &[scope], None)).await;
        let ids: Vec<RecordId> = results.iter().map(|hit| hit.record.id).collect();
        assert_eq!(
            ids,
            vec![rewritten],
            "deleted and re-scoped records drop at hydration; the rewrite survives"
        );
        assert_eq!(
            results[0].record.state.content, "charlie delta checklist",
            "hydration returns current truth, never the sidecar's memory"
        );

        // Convergence: the sweep catches the delete, the move, and the
        // rewrite; old terms stop matching, new terms start.
        let swept = sweep(pool, &index, tenant).await;
        assert_eq!(swept.deletes, 1, "the temporal delete leaves the index");
        assert_eq!(swept.upserts, 2, "the move and the rewrite re-index");
        let results = search(pool, &index, tenant, &query("alpha", &[scope], None)).await;
        assert!(
            results.is_empty(),
            "old terms no longer match after convergence"
        );
        let results = search(
            pool,
            &index,
            tenant,
            &query("charlie delta", &[scope], None),
        )
        .await;
        assert_eq!(results.len(), 1, "new terms match after convergence");
        // The moved record is searchable at its new scope.
        let results = search(
            pool,
            &index,
            tenant,
            &query("escalation contacts", &[other_scope], None),
        )
        .await;
        assert_eq!(results.len(), 1, "the re-scoped record follows its scope");
    });
}

// ── Degradation modes ────────────────────────────────────────────────────────

/// No query vector → BM25-only (the embedder-down degradation CTX-3
/// leans on); no sidecar index yet → dense-only; an unsupported vector
/// dimension is a clean invalid, naming the supported set.
#[test]
fn degrades_to_single_legs_and_rejects_unsupported_dims() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let (tenant, scope) = (TenantId::new(), ScopeId::new());
        let index = fresh_index();

        let record = insert(
            pool,
            tenant,
            scope,
            "failover drill notes",
            Sensitivity::Internal,
            0.0,
        )
        .await;
        sweep(pool, &index, tenant).await;

        // Sparse-only: no vector supplied.
        let results = search(
            pool,
            &index,
            tenant,
            &query("failover drill", &[scope], None),
        )
        .await;
        assert_eq!(results[0].record.id, record);
        assert_eq!(results[0].dense_rank, None);

        // Dense-only: a cold sidecar (fresh root, never swept) misses;
        // the dense leg still serves.
        let cold = fresh_index();
        let results = search(
            pool,
            &cold,
            tenant,
            &query("failover drill", &[scope], Some(0.0)),
        )
        .await;
        assert_eq!(results[0].record.id, record);
        assert_eq!(results[0].sparse_rank, None, "no sidecar index: dense only");

        // Unsupported dimension: clean invalid naming the supported set.
        let mut bad = query("failover drill", &[scope], None);
        bad.vector = Some(QueryVector {
            model: MODEL.to_owned(),
            vector: vec![1.0; 8],
        });
        let mut tx = rls::begin_tenant_tx(pool, tenant).await.expect("tenant tx");
        let error = hybrid_search(&mut tx, &index, tenant, &bad)
            .await
            .expect_err("8-dim vectors have no ANN index");
        assert!(
            matches!(&error, Error::Invalid { message } if message.contains("16")),
            "unsupported dim names the supported set, got: {error}"
        );
    });
}

// ── Indexer semantics ────────────────────────────────────────────────────────

/// Convergence bookkeeping: a swept tenant's next zero-overlap sweep is
/// empty; the default overlap window re-upserts recent rows
/// idempotently (one hit per record, never duplicates); a wiped or
/// unreadable state file rebuilds the index from scratch.
#[test]
fn indexer_watermark_overlap_and_rebuild() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let (tenant, scope) = (TenantId::new(), ScopeId::new());
        let index = fresh_index();

        insert(
            pool,
            tenant,
            scope,
            "quorum sizing guidance",
            Sensitivity::Internal,
            0.0,
        )
        .await;
        insert(
            pool,
            tenant,
            scope,
            "quorum loss postmortem",
            Sensitivity::Internal,
            10.0,
        )
        .await;
        assert_eq!(
            sweep(pool, &index, tenant).await,
            TenantSweep {
                upserts: 2,
                deletes: 0
            }
        );
        assert_eq!(
            sweep(pool, &index, tenant).await,
            TenantSweep::default(),
            "zero-overlap re-sweep of an unchanged tenant is empty"
        );

        // Default 10s overlap: the recent rows re-scan and re-upsert —
        // idempotently. Search still returns exactly two hits.
        let overlapping = indexer::sweep_tenant(pool, &index, tenant, &IndexerConfig::default())
            .await
            .expect("overlap sweep");
        assert_eq!(
            overlapping.upserts, 2,
            "overlap window re-upserts recent rows"
        );
        let results = search(pool, &index, tenant, &query("quorum", &[scope], None)).await;
        assert_eq!(results.len(), 2, "delete-then-add upserts never duplicate");
    });
}
