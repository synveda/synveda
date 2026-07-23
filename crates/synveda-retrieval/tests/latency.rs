//! CTX-1 latency AC (ADR-0024 decision 9): hybrid search over one
//! million records in a single tenant, asserted as MEDIAN under the
//! 80ms budget with the tail reported — the HIER-1/MEM-1 discipline:
//! Docker Desktop's virtualised IO owns the upper percentiles, and
//! EVAL-6 owns percentile SLO enforcement on production-shaped IO.
//!
//! `#[ignore]`d: seeding, HNSW build, and sidecar indexing take
//! minutes, and the run drops/recreates the shared dim-16 ANN index —
//! run it alone, against the dev stack:
//!
//! ```text
//! DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda \
//!   cargo test -p synveda-retrieval --test latency -- --ignored --nocapture
//! ```
//!
//! The test vacuums its own debt (seed + delete churn) before exiting.

use std::time::{Duration, Instant};

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_retrieval::hybrid::{QueryVector, SearchFilter, SearchRequest, hybrid_search};
use synveda_retrieval::index::SearchIndex;
use synveda_retrieval::indexer::{self, IndexerConfig};
use synveda_store::rls;
use synveda_types::{IdentityId, ScopeId, Sensitivity, TenantId};
use uuid::Uuid;

const TOTAL_RECORDS: usize = 1_000_000;
const BATCH: usize = 2_000;
const SCOPES: usize = 100;
/// The caller's "chain": a 4-scope slice (~4% of the corpus) — the
/// selective-predicate regime the iterative HNSW scan exists for.
const CHAIN_SCOPES: usize = 4;
const QUERIES: usize = 200;
const MODEL: &str = "loadtest@1";
const BUDGET: Duration = Duration::from_millis(80);

const VOCAB: [&str; 16] = [
    "deploy",
    "rollback",
    "vacuum",
    "index",
    "pool",
    "audit",
    "tenant",
    "policy",
    "vector",
    "queue",
    "cache",
    "shard",
    "replica",
    "backup",
    "migration",
    "latency",
];

/// Deterministic LCG (no ambient randomness in tests).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 16
    }

    fn word(&mut self) -> &'static str {
        VOCAB[(self.next() % VOCAB.len() as u64) as usize]
    }

    fn unit_vector(&mut self) -> Vec<f32> {
        let mut vector: Vec<f32> = (0..16)
            .map(|_| (self.next() % 2000) as f32 / 1000.0 - 1.0)
            .collect();
        let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-6);
        for value in &mut vector {
            *value /= norm;
        }
        vector
    }
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() as f64 * p) as usize).min(sorted.len() - 1)]
}

#[test]
#[ignore = "seeds 1M records and rebuilds the shared dim-16 ANN index; run alone against the dev stack"]
fn hybrid_median_under_budget_at_one_million_records() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must point at the dev stack");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    rt.block_on(async {
        let pool: PgPool = PgPoolOptions::new()
            .max_connections(4)
            .connect(&url)
            .await
            .expect("connect");
        synveda_store::migrate(&pool).await.expect("migrations");

        let tenant = TenantId::new();
        let owner = IdentityId::new();
        let scopes: Vec<ScopeId> = (0..SCOPES).map(|_| ScopeId::new()).collect();
        let mut lcg = Lcg(0x5EED_CAFE_D00D_F00D);

        // ── Seed: 1M records + embeddings, HNSW dropped for bulk load ────────
        eprintln!("seeding {TOTAL_RECORDS} records in batches of {BATCH}…");
        sqlx::raw_sql("drop index if exists record_embeddings_hnsw_16")
            .execute(&pool)
            .await
            .expect("drop hnsw index for bulk load");
        let seed_started = Instant::now();
        for batch_index in 0..(TOTAL_RECORDS / BATCH) {
            let ids: Vec<Uuid> = (0..BATCH).map(|_| Uuid::now_v7()).collect();
            let batch_scopes: Vec<Uuid> = (0..BATCH)
                .map(|offset| scopes[(batch_index * BATCH + offset) % SCOPES].as_uuid())
                .collect();
            let contents: Vec<String> = (0..BATCH)
                .map(|offset| {
                    format!(
                        "record {} notes about {} {} {}",
                        batch_index * BATCH + offset,
                        lcg.word(),
                        lcg.word(),
                        lcg.word(),
                    )
                })
                .collect();
            // One statement per batch: records and their embeddings land
            // together, so the embed-or-fail backstop (ADR-0023) holds.
            // Vectors are synthesised server-side from the id hash —
            // positive-orthant, never zero, deterministic.
            sqlx::query!(
                r#"
                with new_records as (
                    insert into records
                        (id, tenant_id, scope_id, owner_id, kind, class, content,
                         sensitivity, provenance, valid_from, valid_to, tx_from)
                    select u.id, $1, u.scope_id, $2, 'derived', 'fact', u.content,
                           'internal', '{}'::jsonb, now(), null, now()
                    from unnest($3::uuid[], $4::uuid[], $5::text[])
                        as u(id, scope_id, content)
                    returning id, tenant_id
                )
                insert into record_embeddings (record_id, tenant_id, model, dim, embedding)
                select nr.id, nr.tenant_id, $6, 16,
                    (
                        select ('[' || string_agg(
                            ((((hashtextextended(nr.id::text, s) % 1000) + 1001))::float8
                                / 2000.0)::text,
                            ',' order by s) || ']')
                        from generate_series(1, 16) s
                    )::vector
                from new_records nr
                "#,
                tenant.as_uuid(),
                owner.as_uuid(),
                &ids,
                &batch_scopes,
                &contents,
                MODEL,
            )
            .execute(&pool)
            .await
            .expect("seed batch");
            if (batch_index + 1) % 50 == 0 {
                eprintln!(
                    "  {} / {TOTAL_RECORDS} ({:.0?})",
                    (batch_index + 1) * BATCH,
                    seed_started.elapsed()
                );
            }
        }
        eprintln!("seeded in {:.0?}; building HNSW…", seed_started.elapsed());
        let hnsw_started = Instant::now();
        {
            // Session-level SET: must share a connection with the build.
            let mut conn = pool.acquire().await.expect("acquire build connection");
            sqlx::raw_sql("set maintenance_work_mem = '512MB'")
                .execute(&mut *conn)
                .await
                .expect("maintenance_work_mem");
            // Serial build: parallel workers allocate the budget as a
            // dynamic shared memory segment, which overflows the dev
            // container's small /dev/shm.
            sqlx::raw_sql("set max_parallel_maintenance_workers = 0")
                .execute(&mut *conn)
                .await
                .expect("max_parallel_maintenance_workers");
            sqlx::raw_sql(
                "create index record_embeddings_hnsw_16 on record_embeddings \
                 using hnsw ((embedding::vector(16)) vector_cosine_ops) where dim = 16",
            )
            .execute(&mut *conn)
            .await
            .expect("rebuild hnsw index");
        }
        eprintln!(
            "HNSW built in {:.0?}; building the sidecar…",
            hnsw_started.elapsed()
        );

        let index = SearchIndex::open(
            std::env::temp_dir()
                .join("synveda-ctx1-latency")
                .join(tenant.to_string()),
        )
        .expect("open sidecar root");
        let sweep_started = Instant::now();
        let swept = indexer::sweep_tenant(&pool, &index, tenant, &IndexerConfig::default())
            .await
            .expect("initial sweep");
        assert_eq!(swept.upserts as usize, TOTAL_RECORDS, "sidecar converged");
        eprintln!(
            "sidecar indexed {} docs in {:.0?}",
            swept.upserts,
            sweep_started.elapsed()
        );
        sqlx::raw_sql("vacuum (analyze) records, record_embeddings")
            .execute(&pool)
            .await
            .expect("pre-measure vacuum");

        // ── The Docker-link baseline (delta-over-baseline discipline) ────────
        let mut baseline = Vec::with_capacity(20);
        for _ in 0..20 {
            let start = Instant::now();
            sqlx::query_scalar!("select 1")
                .fetch_one(&pool)
                .await
                .expect("baseline round-trip");
            baseline.push(start.elapsed());
        }
        baseline.sort_unstable();
        let baseline_median = baseline[baseline.len() / 2];

        // ── Measure: 200 hybrid searches over a 4-scope chain slice ─────────
        let chain: Vec<ScopeId> = scopes.iter().copied().take(CHAIN_SCOPES).collect();
        let mut latencies = Vec::with_capacity(QUERIES);
        for _ in 0..QUERIES {
            let text = format!("{} {}", lcg.word(), lcg.word());
            let mut request = SearchRequest::new(
                text,
                SearchFilter {
                    scopes: chain.clone(),
                    max_sensitivity: Sensitivity::Internal,
                },
            );
            request.vector = Some(QueryVector {
                model: MODEL.to_owned(),
                vector: lcg.unit_vector(),
            });
            let start = Instant::now();
            let mut tx = rls::begin_tenant_tx(&pool, tenant)
                .await
                .expect("tenant tx");
            let results = hybrid_search(&mut tx, &index, tenant, &request)
                .await
                .expect("hybrid search");
            drop(tx);
            latencies.push(start.elapsed());
            assert!(!results.is_empty(), "1M-record corpus must yield hits");
            assert!(
                results
                    .iter()
                    .all(|hit| chain.contains(&hit.record.state.scope_id)),
                "every hit stays inside the pushed-down scope slice"
            );
        }
        latencies.sort_unstable();
        let p50 = percentile(&latencies, 0.50);
        let p95 = percentile(&latencies, 0.95);
        let p99 = percentile(&latencies, 0.99);
        eprintln!(
            "hybrid over {TOTAL_RECORDS} records, {CHAIN_SCOPES}-scope slice, {QUERIES} queries: \
             p50 {p50:.2?}, p95 {p95:.2?}, p99 {p99:.2?} \
             (select-1 baseline {baseline_median:.2?}; tails reported, not asserted — EVAL-6)"
        );

        // ── The tenant's debt: delete synthetic rows, vacuum ────────────────
        // The bitemporal triggers would archive a million deletes into
        // history; this is synthetic load-test data on the dev database,
        // so the cleanup suspends triggers for its own session only.
        eprintln!("cleaning up the load tenant…");
        {
            let mut conn = pool.acquire().await.expect("acquire cleanup connection");
            sqlx::raw_sql("set session_replication_role = replica")
                .execute(&mut *conn)
                .await
                .expect("suspend triggers for synthetic-data cleanup");
            sqlx::query!(
                "delete from record_embeddings where tenant_id = $1",
                tenant.as_uuid()
            )
            .execute(&mut *conn)
            .await
            .expect("cleanup record_embeddings");
            sqlx::query!("delete from records where tenant_id = $1", tenant.as_uuid())
                .execute(&mut *conn)
                .await
                .expect("cleanup records");
            sqlx::query!(
                "delete from records_history where tenant_id = $1",
                tenant.as_uuid()
            )
            .execute(&mut *conn)
            .await
            .expect("cleanup records_history");
            sqlx::raw_sql("set session_replication_role = default")
                .execute(&mut *conn)
                .await
                .expect("restore triggers");
        }
        sqlx::raw_sql("vacuum (analyze) records, record_embeddings, records_history")
            .execute(&pool)
            .await
            .expect("post-run vacuum");

        assert!(
            p50 < BUDGET,
            "hybrid retrieval median {p50:.2?} exceeds the {BUDGET:.0?} budget at \
             {TOTAL_RECORDS} records/tenant (select-1 baseline {baseline_median:.2?})"
        );
    });
}
