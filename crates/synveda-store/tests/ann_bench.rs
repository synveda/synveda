//! TEN-3 — the dense leg's benchmark (ADR-0063).
//!
//! The AC's second clause is "benchmark vs unpartitioned recorded", and
//! ADR-0063 makes that the deliverable that decides: partitioning ships
//! only if it beats the cheaper arm by a margin the ADR fixed *before*
//! these numbers existed. This file measures; the ADR holds the verdict.
//! That split is GRPH-4's (`graph_spike.rs`), for its reason — a gate
//! whose thresholds live in the harness is a gate that moves when the
//! harness does.
//!
//! What it measures, per arm and per regime: **recall@10 against exact
//! search** and dense-leg latency (p50/p95). Recall is the number that
//! matters and the one nobody has. Post-filtering does not make the
//! query slow, it makes it *wrong* — an HNSW scan returns its
//! `ef_search` best candidates and the scope filter runs afterwards, so
//! a selective filter silently returns fewer and worse rows rather than
//! taking longer. A benchmark that only timed things would miss the
//! entire failure mode.
//!
//! Two regimes, because they are the two halves of the dense leg's
//! filter and only one of them is a tenant:
//!
//!   **broad**      every scope and tier in the tenant — the regime hash
//!                  partitioning would help, since only the tenant
//!                  predicate is doing any work.
//!   **selective**  one scope, one tier — the regime migration 0016
//!                  already flagged ("when the allowed-scope slice is
//!                  small, the planner should prefer an exact scan over
//!                  the slice to an iterative HNSW crawl"), and the one
//!                  partitioning by tenant cannot reach.
//!
//! The corpus is seeded through `records::insert` inside
//! `begin_tenant_tx`, so every row goes in under the same RLS-scoped
//! path the product writes through. Scopes and owners are synthetic
//! UUIDs: no PDP decision is simulated or skipped here, because the
//! allowed-scope slice is an *input* to this layer — the gateway decides
//! it and hands it down, and the benchmark hands down the same shape.
//!
//! `#[ignore]`d: seeding a corpus and building HNSW over it takes
//! minutes. Run it alone, against a scratch database:
//!
//! ```text
//! createdb -h localhost -U synveda ten3
//! DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/ten3 \
//!   cargo test -p synveda-store --test ann_bench -- --ignored --nocapture
//! ```
//!
//! Scale knobs (env, with the defaults a laptop can hold):
//! `SYNVEDA_BENCH_RECORDS` (20000), `SYNVEDA_BENCH_TENANTS` (8),
//! `SYNVEDA_BENCH_SCOPES` (16 per tenant), `SYNVEDA_BENCH_QUERIES` (100),
//! `SYNVEDA_BENCH_REPORT` (a path to write the JSON row to).

use std::time::{Duration, Instant};

use chrono::Utc;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::{rls, search};
use synveda_types::{
    IdentityId, RecordClass, RecordId, RecordKind, ScopeId, ScopeTier, Sensitivity, TenantId,
};
use uuid::Uuid;

/// The dimension under test. BGE-M3's, because it is the one a customer
/// runs; the 16-dim deterministic embedder is a different regime and a
/// different question.
const DIM: usize = 1024;
/// The model string the corpus is written under and the query filters on.
const MODEL: &str = "bench@1024";
/// Recall is measured at this depth — the dense leg's own top-N default.
const K: i64 = 10;
/// Every tier, for the broad regime.
const TIERS: [Sensitivity; 4] = [
    Sensitivity::Public,
    Sensitivity::Internal,
    Sensitivity::Confidential,
    Sensitivity::Restricted,
];

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(default)
}

/// A tiny LCG. Deterministic on purpose: two arms are only comparable if
/// they saw the same corpus and the same queries, and `rand` would make
/// that a seed-management problem instead of a constant.
struct Lcg(u64);

impl Lcg {
    fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }

    /// A unit vector. Normalised because the shipped embedders emit
    /// normalised vectors and the index is `vector_cosine_ops`; an
    /// unnormalised corpus would measure a distance the product never
    /// computes.
    fn unit_vector(&mut self, dim: usize) -> Vec<f32> {
        let mut v: Vec<f32> = (0..dim)
            .map(|_| (self.next_u32() as f32 / u32::MAX as f32) - 0.5)
            .collect();
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        v
    }
}

struct Corpus {
    tenants: Vec<TenantId>,
    /// Scopes per tenant, in the order they were seeded.
    scopes: Vec<Vec<ScopeId>>,
}

async fn seed(
    pool: &PgPool,
    records_total: usize,
    tenant_count: usize,
    scope_count: usize,
) -> Corpus {
    let mut rng = Lcg(0x5EED_0000_0000_0001);
    let tenants: Vec<TenantId> = (0..tenant_count)
        .map(|_| TenantId::from(Uuid::now_v7()))
        .collect();
    let scopes: Vec<Vec<ScopeId>> = tenants
        .iter()
        .map(|_| {
            (0..scope_count)
                .map(|_| ScopeId::from(Uuid::now_v7()))
                .collect()
        })
        .collect();

    let per_tenant = records_total / tenant_count;
    let started = Instant::now();
    for (t, tenant) in tenants.iter().enumerate() {
        let mut tx = rls::begin_tenant_tx(pool, *tenant)
            .await
            .expect("open a tenant transaction");
        for i in 0..per_tenant {
            let state = RecordState {
                scope_id: scopes[t][i % scope_count],
                owner_id: IdentityId::from(Uuid::now_v7()),
                kind: RecordKind::Derived,
                class: RecordClass::Fact,
                content: format!("bench record {t}/{i}"),
                sensitivity: TIERS[i % TIERS.len()],
                provenance: serde_json::json!({"source": "ten3-bench"}),
                valid_from: Utc::now(),
                valid_to: None,
            };
            let embedding = RecordEmbedding {
                model: MODEL.to_owned(),
                vector: rng.unit_vector(DIM),
            };
            records::insert(
                &mut *tx,
                RecordId::from(Uuid::now_v7()),
                *tenant,
                &state,
                &embedding,
            )
            .await
            .expect("seed a record");
        }
        tx.commit().await.expect("commit a tenant's corpus");
        eprintln!(
            "    seeded tenant {}/{} ({} records, {:?} elapsed)",
            t + 1,
            tenant_count,
            per_tenant,
            started.elapsed()
        );
    }
    Corpus { tenants, scopes }
}

/// Exact top-K over the same filter the dense leg applies, with index
/// scans off so the answer is ground truth rather than a second ANN
/// result. The SQL is a deliberate copy of `search::dense_candidates`'
/// query: the two must agree on *what is being ranked* for recall to
/// mean anything, and a shared helper would let a future edit move both
/// at once and hide a divergence.
async fn exact_top_k(
    conn: &mut sqlx::PgConnection,
    tenant: TenantId,
    query: &[f32],
    allowed: &[ScopeTier],
) -> Vec<RecordId> {
    let scopes: Vec<Uuid> = allowed.iter().map(|a| a.scope_id.into()).collect();
    let tiers: Vec<String> = allowed.iter().map(|a| a.sensitivity.to_string()).collect();
    sqlx::query!(
        r#"select set_config('enable_indexscan', 'off', true) as "a!",
                  set_config('enable_bitmapscan', 'off', true) as "b!""#
    )
    .fetch_one(&mut *conn)
    .await
    .expect("disable index scans for the exact leg");

    sqlx::query!(
        r#"
        select e.record_id as "record_id!"
        from record_embeddings e
        join records r on r.id = e.record_id
        where e.tenant_id = $1
          and e.dim = 1024
          and e.model = $3
          and r.tenant_id = $1
          and (r.scope_id, r.sensitivity)
              in (select * from unnest($4::uuid[], $5::text[]))
        order by e.embedding::vector(1024) <=> $2::real[]::vector(1024)
        limit $6
        "#,
        tenant.as_uuid(),
        query,
        MODEL,
        &scopes,
        &tiers,
        K,
    )
    .fetch_all(&mut *conn)
    .await
    .expect("exact top-k")
    .into_iter()
    .map(|row| RecordId::from(row.record_id))
    .collect()
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx]
}

struct Measurement {
    regime: &'static str,
    recall: f64,
    p50: Duration,
    p95: Duration,
    /// How many rows the slice actually admits — recall means something
    /// different over 40 candidates than over 20,000.
    slice_rows: i64,
}

#[tokio::test]
#[ignore = "TEN-3 benchmark: seeds a corpus and builds HNSW; run alone against a scratch database"]
async fn dense_leg_recall_and_latency() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping the TEN-3 benchmark");
        return;
    };
    let records_total = env_usize("SYNVEDA_BENCH_RECORDS", 20_000);
    let tenant_count = env_usize("SYNVEDA_BENCH_TENANTS", 8);
    let scope_count = env_usize("SYNVEDA_BENCH_SCOPES", 16);
    let queries = env_usize("SYNVEDA_BENCH_QUERIES", 100);

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("connect to the scratch database");
    synveda_store::MIGRATOR
        .run(&pool)
        .await
        .expect("migrate the scratch database");

    eprintln!("=== seeding {records_total} records over {tenant_count} tenants ===");
    let corpus = seed(&pool, records_total, tenant_count, scope_count).await;

    let mut rng = Lcg(0xB0B0_0000_0000_0007);
    let mut measurements = Vec::new();

    for regime in ["broad", "selective"] {
        let mut hits = 0usize;
        let mut truth_total = 0usize;
        let mut timings: Vec<Duration> = Vec::with_capacity(queries);
        let mut slice_rows = 0i64;

        for q in 0..queries {
            let t = q % corpus.tenants.len();
            let tenant = corpus.tenants[t];
            let allowed: Vec<ScopeTier> = match regime {
                "broad" => corpus.scopes[t]
                    .iter()
                    .flat_map(|scope| {
                        TIERS.iter().map(|tier| ScopeTier {
                            scope_id: *scope,
                            sensitivity: *tier,
                        })
                    })
                    .collect(),
                _ => vec![ScopeTier {
                    scope_id: corpus.scopes[t][q % scope_count],
                    sensitivity: Sensitivity::Internal,
                }],
            };
            let query = rng.unit_vector(DIM);

            let mut tx = rls::begin_tenant_tx(&pool, tenant)
                .await
                .expect("tenant tx");
            let started = Instant::now();
            let approx = search::dense_candidates(&mut tx, tenant, MODEL, &query, &allowed, K)
                .await
                .expect("dense candidates");
            timings.push(started.elapsed());
            tx.rollback().await.expect("close the measuring tx");

            let mut tx = rls::begin_tenant_tx(&pool, tenant)
                .await
                .expect("tenant tx");
            let truth = exact_top_k(&mut tx, tenant, &query, &allowed).await;
            if slice_rows == 0 {
                slice_rows = truth.len() as i64;
            }
            tx.rollback().await.expect("close the truth tx");

            truth_total += truth.len();
            hits += approx
                .iter()
                .filter(|hit| truth.contains(&hit.record_id))
                .count();
        }

        timings.sort();
        measurements.push(Measurement {
            regime,
            recall: if truth_total == 0 {
                0.0
            } else {
                hits as f64 / truth_total as f64
            },
            p50: percentile(&timings, 0.50),
            p95: percentile(&timings, 0.95),
            slice_rows,
        });
    }

    eprintln!("\n=== TEN-3 arm A (as shipped: iterative_scan=relaxed_order, ef_search=100) ===");
    eprintln!(
        "corpus {records_total} records / {tenant_count} tenants / {scope_count} scopes, dim {DIM}"
    );
    for m in &measurements {
        eprintln!(
            "  {:<10} recall@{K} {:.3}   p50 {:>7.2}ms   p95 {:>7.2}ms   (truth depth {})",
            m.regime,
            m.recall,
            m.p50.as_secs_f64() * 1000.0,
            m.p95.as_secs_f64() * 1000.0,
            m.slice_rows,
        );
    }

    if let Ok(path) = std::env::var("SYNVEDA_BENCH_REPORT") {
        let rows: Vec<serde_json::Value> = measurements
            .iter()
            .map(|m| {
                serde_json::json!({
                    "regime": m.regime,
                    "recall_at_k": m.recall,
                    "p50_ms": m.p50.as_secs_f64() * 1000.0,
                    "p95_ms": m.p95.as_secs_f64() * 1000.0,
                    "truth_depth": m.slice_rows,
                })
            })
            .collect();
        let report = serde_json::json!({
            "benchmark": "ten3-dense-leg",
            "arm": "A",
            "arm_description": "as shipped: hnsw.iterative_scan=relaxed_order, hnsw.ef_search=100",
            "k": K,
            "dim": DIM,
            "corpus": {
                "records": records_total,
                "tenants": tenant_count,
                "scopes_per_tenant": scope_count,
                "queries": queries,
            },
            "measurements": rows,
        });
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&report).expect("render the report"),
        )
        .expect("write the report");
        eprintln!("\nwrote {path}");
    }
}
