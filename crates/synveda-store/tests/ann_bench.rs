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
//!
//! Arm knobs (env), each defaulting to the shipped `DenseTuning`:
//! `SYNVEDA_BENCH_ITERATIVE` (`off`|`relaxed_order`|`strict_order`),
//! `SYNVEDA_BENCH_EF_SEARCH` (100), `SYNVEDA_BENCH_MAX_SCAN_TUPLES`
//! (`default` — the server's own bound, which pgvector ships as 20,000
//! and no deployment of this product has ever overridden), and
//! `SYNVEDA_BENCH_PLAN_CACHE` (`auto`, what the product runs —
//! `force_custom_plan` is the arm that keeps HNSW past the fifth
//! execution; see [`PlanCacheMode`]).

use std::time::{Duration, Instant};

use chrono::Utc;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
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

/// The arm, from the environment. Arms are separate process runs rather
/// than a loop, so nothing an arm sets can leak into the next one — and
/// so a sweep is a shell loop somebody can read.
fn tuning_from_env() -> search::DenseTuning {
    let default = search::DenseTuning::default();
    search::DenseTuning {
        ef_search: env_parsed("SYNVEDA_BENCH_EF_SEARCH", default.ef_search),
        iterative_scan: match std::env::var("SYNVEDA_BENCH_ITERATIVE").as_deref() {
            Ok("off") => search::IterativeScan::Off,
            Ok("strict_order") => search::IterativeScan::StrictOrder,
            Ok("relaxed_order") => search::IterativeScan::RelaxedOrder,
            Ok(other) => panic!(
                "SYNVEDA_BENCH_ITERATIVE must be off|relaxed_order|strict_order, got {other}"
            ),
            Err(_) => default.iterative_scan,
        },
        max_scan_tuples: match std::env::var("SYNVEDA_BENCH_MAX_SCAN_TUPLES").as_deref() {
            // The word, rather than leaving the variable out: a sweep row
            // that means "the bound nobody has ever set" should say so
            // where it is read, not by an absence the reader has to notice.
            Ok("default") => None,
            Ok(raw) => Some(raw.parse().unwrap_or_else(|_| {
                panic!("SYNVEDA_BENCH_MAX_SCAN_TUPLES must be `default` or a number, got {raw:?}")
            })),
            Err(_) => default.max_scan_tuples,
        },
    }
}

/// PostgreSQL's `plan_cache_mode`, as an arm of its own.
///
/// It is here because TEN-3's first sweep measured it by accident. A
/// prepared statement is planned against its actual parameters for five
/// executions; from the sixth PostgreSQL may substitute a **generic**
/// plan built without them, and at this corpus shape the generic plan
/// drops `record_embeddings_hnsw_1024` entirely and scans the tenant's
/// whole allowed slice exactly instead. Same arm, same GUCs, a different
/// index. A run that does not pin this is not measuring HNSW tuning — it
/// is measuring how many of its pooled connections had reached their
/// sixth execution, which is a property of the pool and the query count.
///
/// `Auto` is what the product runs today and stays the default here, so
/// the arm that ships is still the arm measured by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlanCacheMode {
    Auto,
    ForceCustomPlan,
    ForceGenericPlan,
}

impl PlanCacheMode {
    fn as_guc(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::ForceCustomPlan => "force_custom_plan",
            Self::ForceGenericPlan => "force_generic_plan",
        }
    }
}

fn plan_cache_mode_from_env() -> PlanCacheMode {
    match std::env::var("SYNVEDA_BENCH_PLAN_CACHE").as_deref() {
        Ok("auto") => PlanCacheMode::Auto,
        Ok("force_custom_plan") => PlanCacheMode::ForceCustomPlan,
        Ok("force_generic_plan") => PlanCacheMode::ForceGenericPlan,
        Ok(other) => panic!(
            "SYNVEDA_BENCH_PLAN_CACHE must be auto|force_custom_plan|force_generic_plan, \
             got {other}"
        ),
        Err(_) => PlanCacheMode::Auto,
    }
}

/// Which pgvector produced these numbers, and which PostgreSQL.
///
/// The harness records it because the harness is what talked to the
/// server — the same reason EVAL-3's reports carry their model versions
/// "as served" rather than having the publisher guess them afterwards.
/// Without it a row cannot honour ADR-0063's own reversal trigger, "the
/// harness is re-run on a pgvector major bump", and
/// `scripts/publish-ann-bench.mjs` refuses to publish one — by hand least
/// of all, which is the edit that would make the field decorative.
async fn server_versions(pool: &PgPool) -> (String, String) {
    let row = sqlx::query!(
        r#"select (select extversion from pg_extension where extname = 'vector')
                    as "pgvector",
                  current_setting('server_version') as "postgres!""#
    )
    .fetch_one(pool)
    .await
    .expect("read the server versions");
    let pgvector = row.pgvector.expect(
        "pgvector is not installed in this database — the dense leg has no index to measure",
    );
    (pgvector, row.postgres)
}

/// Set transaction-locally, the way `dense_candidates` sets its own GUCs
/// — so nothing an arm chooses can outlive its transaction into the pool.
async fn set_plan_cache_mode(conn: &mut sqlx::PgConnection, mode: PlanCacheMode) {
    sqlx::query!(
        r#"select set_config('plan_cache_mode', $1, true) as "mode!""#,
        mode.as_guc(),
    )
    .fetch_one(&mut *conn)
    .await
    .expect("set plan_cache_mode");
}

/// A numeric knob from the environment. Unset takes the default; set to
/// something that will not parse is a **panic** rather than a silent
/// fallback.
///
/// This harness has already lost a run to measuring something other than
/// what its label said, and the fallback is how that happens twice: a
/// `SYNVEDA_BENCH_EF_SEARCH=4OO` with a letter in it would produce a row
/// that reads like arm B and holds arm A's numbers, in a table whose job
/// is to decide a change to the product's central primary key.
fn env_parsed<T: std::str::FromStr>(name: &str, default: T) -> T {
    match std::env::var(name) {
        Ok(raw) => raw
            .parse()
            .unwrap_or_else(|_| panic!("{name} must parse as a number, got {raw:?}")),
        Err(_) => default,
    }
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

    // One task per tenant. Serial seeding measured ~300 records/s, which
    // is one round trip per record and puts a corpus large enough to ask
    // this feature's question half an hour away. Tenants are independent
    // transactions, so the concurrency is free of ordering questions —
    // and this stays on the product's own `records::insert` rather than
    // inventing a bulk path, so nothing here can seed a row the write
    // path would not have written (migration 0001's structural rule is
    // about exactly that drift). ADR-0020's COPY-based admission trigger
    // is still due; it belongs to the ingest path, not to a benchmark.
    let mut tasks = Vec::with_capacity(tenant_count);
    for (t, tenant) in tenants.iter().copied().enumerate() {
        let pool = pool.clone();
        let tenant_scopes = scopes[t].clone();
        // Each tenant's stream is seeded from its index, so a corpus is
        // reproducible whatever order the tasks happen to run in.
        let mut rng = Lcg(0x5EED_0000_0000_0001 ^ (t as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        tasks.push(tokio::spawn(async move {
            let mut tx = rls::begin_tenant_tx(&pool, tenant)
                .await
                .expect("open a tenant transaction");
            for i in 0..per_tenant {
                let state = RecordState {
                    // Scope and tier must vary independently. Striding
                    // both by `i` correlated them — with 16 scopes and 4
                    // tiers, `i % 4` is a function of `i % 16`, so every
                    // scope held exactly one tier and a (scope, tier)
                    // slice was empty three times in four. The plan is
                    // what caught it: `Rows Removed by Filter: 500` under
                    // a `Limit (actual rows=0)`.
                    scope_id: tenant_scopes[i % tenant_scopes.len()],
                    owner_id: IdentityId::from(Uuid::now_v7()),
                    kind: RecordKind::Derived,
                    class: RecordClass::Fact,
                    content: format!("bench record {t}/{i}"),
                    sensitivity: TIERS[(i / tenant_scopes.len()) % TIERS.len()],
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
                    tenant,
                    &state,
                    &embedding,
                )
                .await
                .expect("seed a record");
            }
            tx.commit().await.expect("commit a tenant's corpus");
        }));
    }
    for (t, task) in tasks.into_iter().enumerate() {
        task.await.expect("a tenant's seeding task");
        eprintln!(
            "    seeded tenant {}/{} ({} records, {:?} elapsed)",
            t + 1,
            tenant_count,
            per_tenant,
            started.elapsed()
        );
    }
    eprintln!(
        "    {} records in {:?} ({:.0}/s)",
        per_tenant * tenant_count,
        started.elapsed(),
        (per_tenant * tenant_count) as f64 / started.elapsed().as_secs_f64()
    );

    // A corpus is not measurable until the planner knows it is there.
    // Without this, the early queries of a run plan against a table
    // PostgreSQL still believes is empty, and at that estimate it declines
    // `record_embeddings_hnsw_1024` and scans the slice exactly — so the
    // arm's HNSW GUCs govern nothing at all. autoanalyze does arrive on
    // its own, part-way through the measuring loop, which makes the blend
    // a function of the autovacuum naptime rather than of the arm.
    // CTX-1's latency harness has done this since it was written
    // (`crates/synveda-retrieval/tests/latency.rs`).
    let analyze_started = Instant::now();
    sqlx::raw_sql("vacuum (analyze) records, record_embeddings")
        .execute(pool)
        .await
        .expect("analyze the corpus before measuring it");
    eprintln!("    analyzed in {:?}", analyze_started.elapsed());

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

/// Assert the measured transaction really held the arm it is labelled
/// with, by reading the GUCs back inside it.
///
/// This is `env_parsed`'s panic on the far side of the seam: that one
/// guards what the arm was *asked* for, this guards what it *got*. A
/// tuning field that stopped reaching pgvector — a typo'd GUC name, a
/// binding that silently no-ops — would not fail anything. It would
/// publish a row labelled `max_scan_tuples 100000` holding the numbers
/// for 20,000, which is the same defect as the run this harness already
/// lost: a benchmark measuring something other than its own label, and
/// passing.
///
/// `None` is asserted differently and deliberately. It means *the
/// server's own bound*, so what it must equal is what the server would
/// have said anyway — `pg_settings.reset_val`. That makes the assertion
/// "the product did not overwrite an operator's value", which is the
/// property `DenseTuning`'s default is chosen for.
async fn assert_tuning_reached_the_server(
    conn: &mut sqlx::PgConnection,
    tuning: search::DenseTuning,
    plan_cache: PlanCacheMode,
) {
    let held = sqlx::query!(
        r#"select current_setting('hnsw.iterative_scan') as "iterative!",
                  current_setting('hnsw.ef_search') as "ef_search!",
                  current_setting('hnsw.max_scan_tuples') as "bound!",
                  current_setting('plan_cache_mode') as "plan_cache!",
                  (select reset_val from pg_settings
                    where name = 'hnsw.max_scan_tuples') as "bound_reset!""#
    )
    .fetch_one(&mut *conn)
    .await
    .expect("read the dense leg's GUCs back");

    assert_eq!(
        held.plan_cache,
        plan_cache.as_guc(),
        "the measured transaction held a different plan_cache_mode than the arm asked for"
    );

    assert_eq!(
        held.iterative,
        tuning.iterative_scan.as_guc(),
        "the measured transaction held a different hnsw.iterative_scan than the arm asked for"
    );
    assert_eq!(
        held.ef_search,
        tuning.ef_search.to_string(),
        "the measured transaction held a different hnsw.ef_search than the arm asked for"
    );
    assert_eq!(
        held.bound,
        match tuning.max_scan_tuples {
            Some(bound) => bound.to_string(),
            None => held.bound_reset.clone(),
        },
        "the measured transaction held a different hnsw.max_scan_tuples than the arm asked for"
    );
}

/// The **custom** plan: the one PostgreSQL builds against this query's
/// actual parameter values, executed, with real row counts.
///
/// The AC asks for plan evidence, and it is the only thing that
/// distinguishes the two explanations for a fast, exact selective
/// regime: an HNSW crawl that iterative scanning rescued, or the
/// planner declining the index and scanning the slice through
/// `records_tenant_scope_idx` — which is what migration 0016 predicted
/// in as many words. Those have opposite consequences for partitioning,
/// so the harness reads the plan rather than inferring it from a number.
///
/// It is only half the evidence: see [`explain_generic`] for the other
/// plan the same statement gets, and for why a run under
/// `plan_cache_mode = auto` uses both.
async fn explain_dense(
    conn: &mut sqlx::PgConnection,
    tenant: TenantId,
    query: &[f32],
    allowed: &[ScopeTier],
    tuning: search::DenseTuning,
) -> String {
    let scopes: Vec<Uuid> = allowed.iter().map(|a| a.scope_id.into()).collect();
    let tiers: Vec<String> = allowed.iter().map(|a| a.sensitivity.to_string()).collect();
    // The same transaction-local tuning `search::dense_candidates` sets,
    // so this explains the query the product runs and not a cousin.
    sqlx::query!(
        r#"select set_config('hnsw.iterative_scan', $1, true) as "a!",
                  set_config('hnsw.ef_search', $2, true) as "b!",
                  set_config('hnsw.max_scan_tuples', $3, true) as "c!""#,
        tuning.iterative_scan.as_guc(),
        tuning.ef_search.to_string(),
        tuning.max_scan_tuples.map(|bound| bound.to_string()),
    )
    .fetch_one(&mut *conn)
    .await
    .expect("set the dense leg's GUCs");

    // `query_scalar!` rather than `query!`: EXPLAIN names its column
    // "QUERY PLAN", which is not a Rust identifier, and it cannot be
    // aliased because EXPLAIN is a statement rather than a subquery.
    sqlx::query_scalar!(
        r#"
        explain (analyze, costs off, timing off, summary off)
        select e.record_id
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
    .expect("explain the dense leg")
    .into_iter()
    .flatten()
    .map(|line| elide_vectors(&line))
    .collect::<Vec<_>>()
    .join("\n")
}

/// The **generic** plan: the one PostgreSQL builds without this query's
/// parameter values, and the one a pooled connection settles into from
/// its sixth execution of the statement onward.
///
/// It matters because it is a different plan over a different index, and
/// because most of a run's queries get it: sqlx prepares its statements
/// and the pool is long-lived, so five executions per connection are
/// custom and the rest are this. An arm labelled for its HNSW GUCs whose
/// queries mostly ran *this* plan is not measuring HNSW at all.
///
/// `EXPLAIN (GENERIC_PLAN)` (PostgreSQL 16) is the only way to ask. Two
/// things that look like they would work do not, and this harness printed
/// both before the numbers caught it:
///
/// - `plan_cache_mode = force_generic_plan` around an EXPLAIN changes
///   nothing, because it governs *cached* plans and EXPLAIN builds a
///   one-shot plan. It returns the custom plan wearing the wrong label.
/// - Executing an EXPLAIN six times does not reach the sixth-execution
///   switch either, because EXPLAIN re-plans every time.
///
/// It goes through `raw_sql` because GENERIC_PLAN needs the `$n` to
/// arrive **unbound** — the simple query protocol — and the `query!`
/// macro would bind them. The SQL is a static string with no
/// interpolation, so the no-string-built-SQL rule holds in substance:
/// nothing here is assembled from a value. There is no ANALYZE, because
/// a generic plan has no parameter values to execute with; row counts
/// are estimates and the shape is the point.
async fn explain_generic(conn: &mut sqlx::PgConnection) -> String {
    sqlx::raw_sql(
        r#"
        explain (generic_plan, costs off)
        select e.record_id
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
    )
    .fetch_all(&mut *conn)
    .await
    .expect("explain the generic plan")
    .iter()
    .map(|row| row.get::<String, _>(0))
    .collect::<Vec<_>>()
    .join("\n")
}

/// A 1024-dimension literal renders as ~14kB of plan text and buries the
/// node it is attached to. The shape is what a plan is read for.
fn elide_vectors(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.find("'[") {
        out.push_str(&rest[..start]);
        match rest[start..].find("]'") {
            Some(end) => {
                out.push_str("'[…]'");
                rest = &rest[start + end + 2..];
            }
            None => {
                rest = &rest[start..];
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// How a bound reads in a header. `None` is neither "0" nor "20000": it
/// is *the server's own*, which is the arm every deployment has been
/// running and the one a row has to be able to name.
fn describe_bound(bound: Option<u32>) -> String {
    bound.map_or_else(|| "default".to_owned(), |bound| bound.to_string())
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
    /// The plan when it is built against this query's parameters, and
    /// when it is built without them. Two fields rather than one because
    /// they name different indexes, and a run under `auto` uses both.
    plan_custom: String,
    plan_generic: String,
    /// Queries whose slice admitted nothing. A recall computed over
    /// these is EVAL-3's empty-block bug: it passes because there was
    /// nothing to find, not because everything was found.
    empty_slices: usize,
}

#[tokio::test]
#[ignore = "TEN-3 benchmark: seeds a corpus and builds HNSW; run alone against a scratch database"]
async fn dense_leg_recall_and_latency() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping the TEN-3 benchmark");
        return;
    };
    let records_total = env_parsed("SYNVEDA_BENCH_RECORDS", 20_000);
    let tenant_count = env_parsed("SYNVEDA_BENCH_TENANTS", 8);
    let scope_count = env_parsed("SYNVEDA_BENCH_SCOPES", 16);
    let queries = env_parsed("SYNVEDA_BENCH_QUERIES", 100);
    let tuning = tuning_from_env();
    let plan_cache = plan_cache_mode_from_env();

    // Seeding runs a task per tenant; measurement is deliberately serial
    // on one connection, so a latency number is engine cost rather than
    // pool contention.
    let pool = PgPoolOptions::new()
        .max_connections(tenant_count.clamp(1, 16) as u32)
        .connect(&url)
        .await
        .expect("connect to the scratch database");
    synveda_store::MIGRATOR
        .run(&pool)
        .await
        .expect("migrate the scratch database");

    let (pgvector_version, postgres_version) = server_versions(&pool).await;
    eprintln!("=== pgvector {pgvector_version} on PostgreSQL {postgres_version} ===");

    eprintln!("=== seeding {records_total} records over {tenant_count} tenants ===");
    let corpus = seed(&pool, records_total, tenant_count, scope_count).await;

    let mut rng = Lcg(0xB0B0_0000_0000_0007);
    let mut measurements = Vec::new();

    for regime in ["broad", "selective"] {
        let mut hits = 0usize;
        let mut truth_total = 0usize;
        let mut timings: Vec<Duration> = Vec::with_capacity(queries);
        let mut slice_rows = 0i64;
        let mut plan_custom = String::new();
        let mut plan_generic = String::new();
        let mut empty_slices = 0usize;

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
            set_plan_cache_mode(&mut tx, plan_cache).await;
            let started = Instant::now();
            let approx =
                search::dense_candidates(&mut tx, tenant, MODEL, &query, &allowed, K, tuning)
                    .await
                    .expect("dense candidates");
            timings.push(started.elapsed());
            // Once per regime, after the timed region and before the
            // rollback: the arm's label is worth exactly as much as the
            // GUCs the measured transaction actually held.
            if q == 0 {
                assert_tuning_reached_the_server(&mut tx, tuning, plan_cache).await;
            }
            tx.rollback().await.expect("close the measuring tx");

            let mut tx = rls::begin_tenant_tx(&pool, tenant)
                .await
                .expect("tenant tx");
            let truth = exact_top_k(&mut tx, tenant, &query, &allowed).await;
            if slice_rows == 0 {
                slice_rows = truth.len() as i64;
            }
            tx.rollback().await.expect("close the truth tx");

            // One pair of plans per regime, from the first query, on a
            // transaction of its own so the exact leg's
            // `enable_indexscan = off` cannot leak into what it reports —
            // and so the plan_cache_mode it sets per plan cannot leak into
            // the arm's own measuring transactions.
            if plan_custom.is_empty() {
                let mut tx = rls::begin_tenant_tx(&pool, tenant)
                    .await
                    .expect("tenant tx");
                plan_custom = explain_dense(&mut tx, tenant, &query, &allowed, tuning).await;
                plan_generic = explain_generic(&mut tx).await;
                tx.rollback().await.expect("close the explain tx");
            }

            if truth.is_empty() {
                empty_slices += 1;
            }
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
            plan_custom,
            plan_generic,
            empty_slices,
        });
    }

    eprintln!(
        "\n=== TEN-3 dense leg: iterative_scan={} ef_search={} max_scan_tuples={} \
         plan_cache_mode={} ===",
        tuning.iterative_scan.as_guc(),
        tuning.ef_search,
        describe_bound(tuning.max_scan_tuples),
        plan_cache.as_guc(),
    );
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
        assert_eq!(
            m.empty_slices, 0,
            "{} regime: {} of {} queries had a slice that admitted nothing. Recall over an \
             empty slice is EVAL-3's empty-block bug — it passes because there was nothing to \
             find. Fix the corpus, not this assertion.",
            m.regime, m.empty_slices, queries
        );
    }
    for m in &measurements {
        eprintln!(
            "\n--- {} regime, custom plan (built with this query's parameters) ---\n{}",
            m.regime, m.plan_custom
        );
        eprintln!(
            "\n--- {} regime, generic plan (built without them) ---\n{}",
            m.regime, m.plan_generic
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
                    "plan_custom": m.plan_custom,
                    "plan_generic": m.plan_generic,
                    "empty_slices": m.empty_slices,
                })
            })
            .collect();
        let report = serde_json::json!({
            "benchmark": "ten3-dense-leg",
            "tuning": {
                "iterative_scan": tuning.iterative_scan.as_guc(),
                "ef_search": tuning.ef_search,
                // `null` and the server's 20,000 are the same behaviour and
                // not the same fact: the row has to say which one it is, or
                // a later pgvector changing its default would silently
                // relabel every row already recorded.
                "max_scan_tuples": tuning.max_scan_tuples,
                "is_shipped_default": tuning == search::DenseTuning::default(),
            },
            // Not part of `tuning`, because the product does not set it —
            // it is the arm this harness added after discovering that
            // leaving it alone let the planner choose a different index
            // part-way through a run.
            "plan_cache_mode": plan_cache.as_guc(),
            // The engine that produced the numbers, recorded where it was
            // read rather than reconstructed at publish time.
            "pgvector_version": pgvector_version,
            "postgres_version": postgres_version,
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
