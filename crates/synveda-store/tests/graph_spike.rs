//! GRPH-4 — the AGE traversal gate (ADR-0029).
//!
//! ADR-0001 and ADR-0004 both rest on "AGE Cypher performance is unproven
//! at 10M+ edges" and both name this spike as their reversal trigger. The
//! criteria are pre-registered in ADR-0029 and are not restated here as
//! asserts: a gate whose thresholds live in the harness is a gate that
//! moves when the harness does. This test *measures* and prints; the ADR
//! holds the verdict, and `demos/grph-4-graph-spike.sh` reads the two
//! together.
//!
//! Both scales keep out-degree at 10, so the *local* neighbourhood a
//! traversal touches is identical and only the surrounding data volume
//! grows — which is what makes G2's 1M→10M slope mean "is this
//! index-backed" rather than "is this more work".
//!
//! Every query below is built with `format!`. That is not laziness and it
//! is the finding G5 exists to record: AGE's `cypher()` takes its graph
//! name as a *name constant* and its parameter map as a *bare* `$n` (not
//! even `$1::agtype`), so a per-tenant graph name can only reach the
//! statement as text. See the doc comment on `AGE_PARAM_NOTE`.
//!
//! `#[ignore]`d: seeding 11M edges and building the property indexes takes
//! minutes. Run it alone, against a scratch database:
//!
//! ```text
//! DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/grph4 \
//!   cargo test -p synveda-store --test graph_spike -- --ignored --nocapture
//! ```
//!
//! The test drops the graphs and tables it made before it exits.

use std::time::{Duration, Instant};

use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, PgPool};

/// Out-degree held constant across scales (see the module note).
const DEGREE: i64 = 10;
/// The seed-set size GRPH-3 expands from: hybrid retrieval hands recall a
/// ranked hit set, and the graph expands around it.
const SEED_SET: usize = 10;
/// Iterations per measurement. Each draws a fresh seed set.
const QUERIES: usize = 200;
/// Iterations for the two shapes that fall off the index. They are here to
/// be recorded, not to be fast, and 200 iterations of a multi-second query
/// would own the whole run.
const TRAP_QUERIES: usize = 20;
/// Iterations for the write-path measurement (G3).
const WRITE_SAMPLES: usize = 100;

/// Recorded for the report: what AGE will and will not accept as a
/// parameter, established interactively before this harness was written.
///
/// - `cypher($1, $$…$$)` → `ERROR: a name constant is expected`
/// - `cypher('g', $$…$$, $1::agtype)` → `ERROR: third argument of cypher
///   function must be a parameter`
///
/// So: the graph name must be literal text in the statement, and the
/// parameter map must be a bare `$n` bound as `agtype` — a type sqlx has
/// no encoder for without a custom impl. A per-tenant graph therefore
/// yields a distinct statement text per tenant.
const AGE_PARAM_NOTE: &str = "graph name must be a name constant; params must be a bare $n";

struct Scale {
    name: &'static str,
    graph: &'static str,
    rel_table: &'static str,
    edges: i64,
}

impl Scale {
    fn vertices(&self) -> i64 {
        self.edges / DEGREE
    }
}

const SCALES: [Scale; 2] = [
    Scale {
        name: "1M",
        graph: "grph4_1m",
        rel_table: "grph4_rel_1m",
        edges: 1_000_000,
    },
    Scale {
        name: "10M",
        graph: "grph4_10m",
        rel_table: "grph4_rel_10m",
        edges: 10_000_000,
    },
];

/// Deterministic LCG — no ambient randomness in tests (the latency.rs rule).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 16
    }

    /// A seed set of distinct vertex ids in `1..=vertices`.
    fn seeds(&mut self, vertices: i64, n: usize) -> Vec<i64> {
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            let v = 1 + (self.next() % vertices as u64) as i64;
            if !out.contains(&v) {
                out.push(v);
            }
        }
        out
    }
}

struct Stats {
    median: Duration,
    p95: Duration,
    max: Duration,
    rows: u64,
}

/// Nearest-rank percentiles, matching the eval harness (ADR-0028).
fn stats(mut samples: Vec<Duration>, rows: u64) -> Stats {
    samples.sort_unstable();
    let pick = |p: f64| {
        let rank = ((samples.len() as f64) * p).ceil().max(1.0) as usize;
        samples[rank.min(samples.len()) - 1]
    };
    Stats {
        median: pick(0.5),
        p95: pick(0.95),
        max: *samples.last().expect("at least one sample"),
        rows,
    }
}

fn ms(d: Duration) -> String {
    format!("{:.2}", d.as_secs_f64() * 1000.0)
}

/// One measured row of the report.
struct Measurement {
    scale: &'static str,
    label: &'static str,
    stats: Stats,
}

/// Time `QUERIES` executions of a statement built fresh per iteration.
///
/// The build closure receives the iteration's seed set. For the AGE
/// variants the seeds land in the statement *text* (see `AGE_PARAM_NOTE`),
/// so each iteration is a distinct statement and pays its own parse and
/// plan — which is the cost the string-built design actually incurs, not
/// an artefact of the harness.
async fn measure_built(
    pool: &PgPool,
    vertices: i64,
    lcg_seed: u64,
    iters: usize,
    build: impl Fn(&[i64]) -> String,
) -> Stats {
    let mut lcg = Lcg(lcg_seed);
    let mut samples = Vec::with_capacity(iters);
    let mut rows = 0u64;
    for _ in 0..iters {
        let seeds = lcg.seeds(vertices, SEED_SET);
        let sql = build(&seeds);
        let start = Instant::now();
        let out = sqlx::query(&sql)
            .fetch_all(pool)
            .await
            .expect("traversal query");
        samples.push(start.elapsed());
        rows = out.len() as u64;
    }
    stats(samples, rows)
}

/// Time `QUERIES` executions of one statement with bound parameters — the
/// relational baseline, where the seed set is data rather than text.
async fn measure_bound(pool: &PgPool, vertices: i64, lcg_seed: u64, sql: &str) -> Stats {
    let mut lcg = Lcg(lcg_seed);
    let mut samples = Vec::with_capacity(QUERIES);
    let mut rows = 0u64;
    for _ in 0..QUERIES {
        let seeds = lcg.seeds(vertices, SEED_SET);
        let start = Instant::now();
        let out = sqlx::query(sql)
            .bind(&seeds)
            .fetch_all(pool)
            .await
            .expect("baseline query");
        samples.push(start.elapsed());
        rows = out.len() as u64;
    }
    stats(samples, rows)
}

/// One cypher branch: a single seed, matched by property equality — the
/// only shape AGE plans with its indexes (`IN` and `OR` both fall back to
/// a full label scan; measured separately below).
fn hop1_branch(graph: &str, seed: i64) -> String {
    format!(
        "SELECT * FROM cypher('{graph}', $$ MATCH (a:Entity {{eid: {seed}}})\
         -[:RELATES_TO]->(b:Entity) RETURN b.eid $$) AS (eid agtype)"
    )
}

fn hop2_branch(graph: &str, seed: i64) -> String {
    format!(
        "SELECT * FROM cypher('{graph}', $$ MATCH (a:Entity {{eid: {seed}}})\
         -[:RELATES_TO]->(m:Entity)-[:RELATES_TO]->(b:Entity) RETURN b.eid $$) AS (eid agtype)"
    )
}

fn union_all(branches: Vec<String>) -> String {
    branches.join(" UNION ALL ")
}

async fn seed_age(pool: &PgPool, scale: &Scale) {
    let (graph, vertices, edges) = (scale.graph, scale.vertices(), scale.edges);
    eprintln!("  seeding AGE graph {graph}: {vertices} vertices, {edges} edges");

    sqlx::query(&format!("SELECT create_graph('{graph}')"))
        .execute(pool)
        .await
        .expect("create_graph");
    for stmt in [
        format!("SELECT create_vlabel('{graph}','Entity')"),
        format!("SELECT create_elabel('{graph}','RELATES_TO')"),
    ] {
        sqlx::query(&stmt)
            .execute(pool)
            .await
            .expect("create label");
    }

    // Bulk load through the label tables. `cypher CREATE` is per-row and
    // would take hours at 10M — AGE ships no bulk loader, which is itself
    // a GRPH-1 obligation (the ingestion pipeline writes one edge at a
    // time, but backfill and re-index do not).
    sqlx::query(&format!(
        "INSERT INTO {graph}.\"Entity\" (id, properties)
         SELECT _graphid(_label_id('{graph}','Entity')::int, n),
                ('{{\"eid\": ' || n || ', \"kind\": \"person\"}}')::agtype
         FROM generate_series(1::bigint, {vertices}::bigint) n"
    ))
    .execute(pool)
    .await
    .expect("bulk vertices");

    // dst mixes in n/vertices so a source's DEGREE edges land on DEGREE
    // *distinct* targets: a pure `n * prime % vertices` gives every edge of
    // a vertex the same destination, and a 2-hop over that measures nothing.
    sqlx::query(&format!(
        "INSERT INTO {graph}.\"RELATES_TO\" (id, start_id, end_id, properties)
         SELECT _graphid(_label_id('{graph}','RELATES_TO')::int, n),
                _graphid(_label_id('{graph}','Entity')::int, 1 + (n % {vertices})),
                _graphid(_label_id('{graph}','Entity')::int,
                         1 + ((n * 7919 + (n / {vertices}) * 104729) % {vertices})),
                '{{\"rel\": \"works_with\"}}'::agtype
         FROM generate_series(1::bigint, {edges}::bigint) n"
    ))
    .execute(pool)
    .await
    .expect("bulk edges");

    for stmt in [
        format!("SELECT setval('{graph}.\"Entity_id_seq\"', {vertices})"),
        format!("SELECT setval('{graph}.\"RELATES_TO_id_seq\"', {edges})"),
    ] {
        sqlx::query(&stmt).execute(pool).await.expect("setval");
    }

    // The property index AGE does *not* create for you. Without it every
    // seed lookup is a full label scan (12.5ms at 100k vertices, and it
    // grows linearly) — recorded as a GRPH-1 obligation.
    sqlx::query(&format!(
        "CREATE INDEX {graph}_entity_props ON {graph}.\"Entity\" USING gin (properties)"
    ))
    .execute(pool)
    .await
    .expect("gin property index");

    for t in ["Entity", "RELATES_TO"] {
        sqlx::query(&format!("ANALYZE {graph}.\"{t}\""))
            .execute(pool)
            .await
            .expect("analyze");
    }
}

async fn seed_rel(pool: &PgPool, scale: &Scale) {
    let (table, vertices, edges) = (scale.rel_table, scale.vertices(), scale.edges);
    eprintln!("  seeding relational baseline {table}");
    sqlx::query(&format!(
        "CREATE TABLE {table} (src bigint not null, dst bigint not null,
                               props jsonb not null default '{{}}'::jsonb)"
    ))
    .execute(pool)
    .await
    .expect("create baseline table");
    sqlx::query(&format!(
        "INSERT INTO {table} (src, dst)
         SELECT 1 + (n % {vertices}),
                1 + ((n * 7919 + (n / {vertices}) * 104729) % {vertices})
         FROM generate_series(1::bigint, {edges}::bigint) n"
    ))
    .execute(pool)
    .await
    .expect("bulk baseline edges");
    for stmt in [
        format!("CREATE INDEX {table}_src ON {table} (src)"),
        format!("CREATE INDEX {table}_dst ON {table} (dst)"),
        format!("ANALYZE {table}"),
    ] {
        sqlx::query(&stmt)
            .execute(pool)
            .await
            .expect("baseline index");
    }
}

/// G3: a single edge create, committed — the shape GRPH-2 runs per record.
async fn measure_writes(pool: &PgPool, scale: &Scale) -> (Stats, Stats) {
    let (graph, table, vertices) = (scale.graph, scale.rel_table, scale.vertices());
    let mut lcg = Lcg(0x5150);
    let mut age = Vec::with_capacity(WRITE_SAMPLES);
    let mut rel = Vec::with_capacity(WRITE_SAMPLES);
    for _ in 0..WRITE_SAMPLES {
        let pair = lcg.seeds(vertices, 2);
        let (a, b) = (pair[0], pair[1]);

        let sql = format!(
            "SELECT * FROM cypher('{graph}', $$ MATCH (a:Entity {{eid: {a}}}), (b:Entity {{eid: {b}}})
             CREATE (a)-[:RELATES_TO {{rel: 'spike'}}]->(b) $$) AS (v agtype)"
        );
        let start = Instant::now();
        sqlx::query(&sql)
            .execute(pool)
            .await
            .expect("cypher edge create");
        age.push(start.elapsed());

        let start = Instant::now();
        sqlx::query(&format!("INSERT INTO {table} (src, dst) VALUES ($1, $2)"))
            .bind(a)
            .bind(b)
            .execute(pool)
            .await
            .expect("baseline edge insert");
        rel.push(start.elapsed());
    }
    (
        stats(age, WRITE_SAMPLES as u64),
        stats(rel, WRITE_SAMPLES as u64),
    )
}

/// G4: what one tenant's three graphs (ADR-0004) cost in catalog terms.
async fn measure_tenant_cost(pool: &PgPool) -> (Duration, i64) {
    let graphs = [
        "grph4_ten_entity",
        "grph4_ten_episode",
        "grph4_ten_provenance",
    ];
    let start = Instant::now();
    for g in graphs {
        sqlx::query(&format!("SELECT create_graph('{g}')"))
            .execute(pool)
            .await
            .expect("create tenant graph");
        sqlx::query(&format!("SELECT create_vlabel('{g}','Entity')"))
            .execute(pool)
            .await
            .expect("vlabel");
        sqlx::query(&format!("SELECT create_elabel('{g}','RELATES_TO')"))
            .execute(pool)
            .await
            .expect("elabel");
    }
    let elapsed = start.elapsed();

    let relations: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_class c
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = ANY($1)",
    )
    .bind(&graphs.map(String::from)[..])
    .fetch_one(pool)
    .await
    .expect("count relations");

    for g in graphs {
        sqlx::query(&format!("SELECT drop_graph('{g}', true)"))
            .execute(pool)
            .await
            .expect("drop tenant graph");
    }
    (elapsed, relations)
}

async fn cleanup(pool: &PgPool) {
    for scale in &SCALES {
        let _ = sqlx::query(&format!("SELECT drop_graph('{}', true)", scale.graph))
            .execute(pool)
            .await;
        let _ = sqlx::query(&format!("DROP TABLE IF EXISTS {}", scale.rel_table))
            .execute(pool)
            .await;
    }
}

#[tokio::test]
#[ignore = "GRPH-4 spike: seeds 11M edges; run alone against a scratch database"]
async fn age_traversal_gate() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping the GRPH-4 spike");
        return;
    };

    // One connection: the gate measures engine cost, not pool behaviour.
    // `search_path` carries ag_catalog because every AGE call needs it —
    // itself a note for GRPH-1, since the product pool must do the same.
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|conn, _| {
            Box::pin(async move {
                conn.execute("LOAD 'age'; SET search_path = ag_catalog, \"$user\", public;")
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .expect("connect to the scratch database");

    cleanup(&pool).await;

    let mut report: Vec<Measurement> = Vec::new();
    let mut seed_times: Vec<(&str, Duration, Duration)> = Vec::new();

    for scale in &SCALES {
        eprintln!("\n=== scale {} ===", scale.name);
        let start = Instant::now();
        seed_age(&pool, scale).await;
        let age_seed = start.elapsed();
        let start = Instant::now();
        seed_rel(&pool, scale).await;
        let rel_seed = start.elapsed();
        seed_times.push((scale.name, age_seed, rel_seed));

        let v = scale.vertices();
        let g = scale.graph;
        let t = scale.rel_table;

        // AGE at its best: N single-seed branches, each index-backed.
        report.push(Measurement {
            scale: scale.name,
            label: "age 1-hop, 10 seeds (UNION ALL, indexed)",
            stats: measure_built(&pool, v, 0x1101, QUERIES, |seeds| {
                union_all(seeds.iter().map(|s| hop1_branch(g, *s)).collect())
            })
            .await,
        });
        report.push(Measurement {
            scale: scale.name,
            label: "age 2-hop, 10 seeds (explicit, indexed)",
            stats: measure_built(&pool, v, 0x2202, QUERIES, |seeds| {
                union_all(seeds.iter().map(|s| hop2_branch(g, *s)).collect())
            })
            .await,
        });

        // The two shapes a reasonable person writes first, both of which
        // fall off the index. Single-seed for the VLE form: at 1M it is
        // already ~900ms and 200 iterations of that is the whole run.
        report.push(Measurement {
            scale: scale.name,
            label: "age 1-hop, 10 seeds (IN list — natural cypher)",
            stats: measure_built(&pool, v, 0x3303, TRAP_QUERIES, |seeds| {
                let list = seeds
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "SELECT * FROM cypher('{g}', $$ MATCH (a:Entity)-[:RELATES_TO]->(b:Entity)
                     WHERE a.eid IN [{list}] RETURN b.eid $$) AS (eid agtype)"
                )
            })
            .await,
        });
        report.push(Measurement {
            scale: scale.name,
            label: "age 1..2-hop, 1 seed (*1..2 VLE — natural cypher)",
            stats: measure_built(&pool, v, 0x4404, TRAP_QUERIES, |seeds| {
                let s = seeds[0];
                format!(
                    "SELECT * FROM cypher('{g}', $$ MATCH (a:Entity {{eid: {s}}})
                     -[:RELATES_TO*1..2]->(b:Entity) RETURN b.eid $$) AS (eid agtype)"
                )
            })
            .await,
        });

        // The reference: plain indexed adjacency, seeds bound as data.
        report.push(Measurement {
            scale: scale.name,
            label: "sql  1-hop, 10 seeds (adjacency, bound)",
            stats: measure_bound(
                &pool,
                v,
                0x1101,
                &format!("SELECT dst FROM {t} WHERE src = ANY($1)"),
            )
            .await,
        });
        report.push(Measurement {
            scale: scale.name,
            label: "sql  2-hop, 10 seeds (adjacency, bound)",
            stats: measure_bound(
                &pool,
                v,
                0x2202,
                &format!(
                    "SELECT e2.dst FROM {t} e1 JOIN {t} e2 ON e2.src = e1.dst
                     WHERE e1.src = ANY($1)"
                ),
            )
            .await,
        });

        let (age_w, rel_w) = measure_writes(&pool, scale).await;
        report.push(Measurement {
            scale: scale.name,
            label: "age single edge create (cypher)",
            stats: age_w,
        });
        report.push(Measurement {
            scale: scale.name,
            label: "sql  single edge insert",
            stats: rel_w,
        });
    }

    let (tenant_ddl, tenant_relations) = measure_tenant_cost(&pool).await;

    println!("\n╔═══ GRPH-4 · AGE traversal gate (ADR-0029) ═══════════════════════════════╗");
    println!("  seed set {SEED_SET} · out-degree {DEGREE} · {QUERIES} queries per row");
    println!("  AGE note: {AGE_PARAM_NOTE}\n");
    println!(
        "  {:<48} {:>6} {:>9} {:>9} {:>9} {:>7}",
        "measurement", "scale", "median", "p95", "max", "rows"
    );
    for r in &report {
        println!(
            "  {:<48} {:>6} {:>8}ms {:>8}ms {:>8}ms {:>7}",
            r.label,
            r.scale,
            ms(r.stats.median),
            ms(r.stats.p95),
            ms(r.stats.max),
            r.stats.rows
        );
    }

    println!("\n  G2 slope (10M median ÷ 1M median):");
    for label in [
        "age 1-hop, 10 seeds (UNION ALL, indexed)",
        "age 2-hop, 10 seeds (explicit, indexed)",
        "age 1-hop, 10 seeds (IN list — natural cypher)",
        "sql  1-hop, 10 seeds (adjacency, bound)",
        "sql  2-hop, 10 seeds (adjacency, bound)",
    ] {
        let at = |scale: &str| {
            report
                .iter()
                .find(|r| r.label == label && r.scale == scale)
                .map(|r| r.stats.median.as_secs_f64())
        };
        if let (Some(a), Some(b)) = (at("1M"), at("10M")) {
            println!("  {label:<48} {:>6.2}x", b / a);
        }
    }

    println!("\n  G4 tenant cost (ADR-0004's three graphs per tenant):");
    println!("    create 3 graphs + labels     {:>8}ms", ms(tenant_ddl));
    println!("    relations per tenant         {tenant_relations:>8}");
    println!(
        "    extrapolated to 1,000 tenants {:>7} relations, {:.1}s DDL",
        tenant_relations * 1000,
        tenant_ddl.as_secs_f64() * 1000.0
    );

    println!("\n  seeding (context, not a criterion):");
    for (scale, age, rel) in &seed_times {
        println!(
            "    {scale:<4} age {:>9}ms   adjacency {:>9}ms",
            ms(*age),
            ms(*rel)
        );
    }
    println!("╚═════════════════════════════════════════════════════════════════════════╝\n");

    cleanup(&pool).await;
}
