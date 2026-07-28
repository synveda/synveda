//! GRPH-1 acceptance criteria (ADR-0043), all three clauses: an edge
//! written through the store API reads back through the traversal API with
//! its kind, endpoints and validity intact; a supersession closes the prior
//! edge's window with both versions readable as-of; and the shipped
//! statements' plans contain no sequential scan over `graph_edges`.
//!
//! The plan assertion (decision 9) explains **the statements this crate
//! ships**, found in `src/graph.rs` by their `-- shipped-traversal:`
//! markers — not a copy of them, which is the failure mode a plan guard has
//! to survive. It runs in the ordinary suite, because a plan that regresses
//! silently is how the discipline dies on contact with the second
//! contributor.
//!
//! Decision 15's re-measurement of the traversal on the built schema —
//! with its RLS predicate, bitemporal columns, composite foreign keys and
//! tenant index, at the spike's own shape — is `#[ignore]`d and lives at
//! the bottom of this file:
//!
//! ```text
//! DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda \
//!   cargo test -p synveda-store --test graph -- --ignored --nocapture
//! ```
//!
//! These tests need a live Postgres and a connection role allowed to `SET
//! ROLE synveda_app`. They read `DATABASE_URL` and skip with a message when
//! it is unset (CI has no database); run them locally with `make db-test`
//! (dev environment up). Isolation is by freshly minted tenant, so a shared
//! dev database is fine.

use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Row, Transaction};
use synveda_store::graph::{self, EdgeState, VertexState};
use synveda_store::{rls, tenants};
use synveda_types::{Depth, Error, Graph, GraphEdgeId, GraphVertexId, TenantId, TenantStatus};
use uuid::Uuid;

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
                    "skipping graph tests: DATABASE_URL is not set \
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

/// Guarantees the next statement runs at a strictly later `now()`.
async fn tick() {
    tokio::time::sleep(Duration::from_millis(5)).await;
}

async fn admit_tenant(pool: &PgPool) -> TenantId {
    let tenant = TenantId::new();
    let slug = format!("grph-{}", tenant.as_uuid().simple());
    tenants::create(pool, tenant, &slug, "GRPH-1 fixture", TenantStatus::Active)
        .await
        .expect("create tenant");
    tenant
}

/// Fixed valid-time epoch, safely before any test's transaction time.
fn base() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

/// An instant strictly inside `[a, b)` — the FND-4 helper, for the same
/// reason: an as-of probe must sit between two server-stamped versions.
fn midpoint(a: DateTime<Utc>, b: DateTime<Utc>) -> DateTime<Utc> {
    a + (b - a) / 2
}

/// Interns a vertex named `key` in `graph` and returns its id.
async fn vertex(pool: &PgPool, tenant: TenantId, graph: Graph, key: &str) -> GraphVertexId {
    let id = GraphVertexId::new();
    graph::upsert_vertex(
        pool,
        id,
        tenant,
        graph,
        &VertexState {
            kind: "person".to_owned(),
            key: key.to_owned(),
            label: key.to_owned(),
            record_id: None,
        },
    )
    .await
    .expect("upsert vertex")
    .id
}

fn edge_state(src: GraphVertexId, dst: GraphVertexId, kind: &str) -> EdgeState {
    EdgeState {
        kind: kind.to_owned(),
        src_id: src,
        dst_id: dst,
        method: "deterministic".to_owned(),
        confidence_permille: 900,
        source_record_id: None,
        valid_from: base(),
        valid_to: None,
    }
}

/// Asserts `kind` from `src` to `dst` and returns the edge id.
async fn link(
    pool: &PgPool,
    tenant: TenantId,
    graph: Graph,
    src: GraphVertexId,
    dst: GraphVertexId,
    kind: &str,
) -> GraphEdgeId {
    let id = GraphEdgeId::new();
    graph::insert_edge(pool, id, tenant, graph, &edge_state(src, dst, kind))
        .await
        .expect("insert edge")
        .id
}

// ── The headline acceptance test ─────────────────────────────────────────────

/// Clause one: what the store API wrote is what the traversal API returns —
/// kind, both endpoints and the valid window, unchanged.
#[test]
fn an_edge_round_trips_through_the_traversal_api() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let tenant = admit_tenant(pool).await;
        let ada = vertex(pool, tenant, Graph::Entity, "ada").await;
        let grace = vertex(pool, tenant, Graph::Entity, "grace").await;

        let written = graph::insert_edge(
            pool,
            GraphEdgeId::new(),
            tenant,
            Graph::Entity,
            &edge_state(ada, grace, "reports_to"),
        )
        .await
        .expect("insert edge");

        let mut conn = pool.acquire().await.expect("acquire");
        let found = graph::expand(
            &mut conn,
            tenant,
            Graph::Entity,
            &[ada],
            Depth::One,
            base(),
            None,
        )
        .await
        .expect("expand");

        assert_eq!(found.edges.len(), 1, "one claim was written");
        let read = &found.edges[0];
        assert_eq!(read.id, written.id);
        assert_eq!(read.state.kind, "reports_to", "kind intact");
        assert_eq!(read.state.src_id, ada, "source endpoint intact");
        assert_eq!(read.state.dst_id, grace, "target endpoint intact");
        assert_eq!(read.state.valid_from, base(), "validity intact");
        assert_eq!(read.state.valid_to, None, "still open-ended");
        assert_eq!(read.graph, Graph::Entity);
        assert_eq!(read.state.confidence_permille, 900);
        assert!(read.tx_to.is_none(), "current version");

        assert_eq!(
            found
                .reached
                .iter()
                .map(|r| r.vertex_id)
                .collect::<Vec<_>>(),
            vec![grace],
            "the seed is not its own discovery; its neighbour is"
        );
        assert_eq!(found.reached[0].hop, 1);
    });
}

/// Clause two: supersession is a closed window plus a new row (decision 4),
/// and both versions of the superseded claim stay readable as-of — the
/// property that makes the *schema* answer "what did we claim on date D"
/// rather than the code that wrote it.
#[test]
fn supersession_closes_the_window_and_both_versions_read_as_of() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let tenant = admit_tenant(pool).await;
        let ada = vertex(pool, tenant, Graph::Entity, "ada").await;
        let grace = vertex(pool, tenant, Graph::Entity, "grace").await;
        let alan = vertex(pool, tenant, Graph::Entity, "alan").await;

        let first = graph::insert_edge(
            pool,
            GraphEdgeId::new(),
            tenant,
            Graph::Entity,
            &edge_state(ada, grace, "reports_to"),
        )
        .await
        .expect("insert the original claim");
        tick().await;

        // Ada moved teams: the old claim stops holding, a new one starts.
        let moved_at = base() + chrono::Duration::days(30);
        let mut replacement = edge_state(ada, alan, "reports_to");
        replacement.valid_from = moved_at;
        let superseded = graph::supersede(
            pool,
            tenant,
            Graph::Entity,
            first.id,
            moved_at,
            GraphEdgeId::new(),
            &replacement,
        )
        .await
        .expect("supersede")
        .expect("the original claim was current");

        assert_eq!(superseded.closed.id, first.id);
        assert_eq!(
            superseded.closed.state.valid_to,
            Some(moved_at),
            "the prior claim's window is closed"
        );
        assert_eq!(superseded.replacement.state.dst_id, alan);
        assert_eq!(superseded.replacement.state.valid_from, moved_at);

        // Both versions of the *same* claim are in history.
        let versions = graph::edge_versions(pool, tenant, first.id)
            .await
            .expect("versions");
        assert_eq!(versions.len(), 2, "the open-ended version was archived");
        assert_eq!(versions[0].state.valid_to, None, "v1 was open-ended");
        assert_eq!(versions[1].state.valid_to, Some(moved_at), "v2 is closed");
        assert!(
            versions[0].tx_to.is_some(),
            "v1's transaction period closed"
        );
        assert!(versions[1].tx_to.is_none(), "v2 is current");

        // Transaction time is the *server's* clock, so the rewind instant
        // comes from the stamps the database wrote — never from this
        // process's clock, which is a different machine's in dev compose.
        let before = midpoint(versions[0].tx_from, versions[1].tx_from);

        // As-of rewinds to what the database claimed before the move.
        let then = graph::edge_as_of(pool, tenant, first.id, before)
            .await
            .expect("as-of")
            .expect("the claim existed then");
        assert_eq!(then.state.valid_to, None, "as-of reads the open version");
        assert_eq!(then.state.dst_id, grace);

        // And the traversal answers the same way at both instants: rewound,
        // Ada reports to Grace; now, to Alan.
        let mut conn = pool.acquire().await.expect("acquire");
        let now_view = graph::expand(
            &mut conn,
            tenant,
            Graph::Entity,
            &[ada],
            Depth::One,
            Utc::now(),
            None,
        )
        .await
        .expect("expand current");
        assert_eq!(
            now_view
                .reached
                .iter()
                .map(|r| r.vertex_id)
                .collect::<Vec<_>>(),
            vec![alan],
            "the closed claim no longer holds at today's valid time"
        );

        let then_view = graph::expand(
            &mut conn,
            tenant,
            Graph::Entity,
            &[ada],
            Depth::One,
            base(),
            Some(before),
        )
        .await
        .expect("expand as-of");
        assert_eq!(
            then_view
                .reached
                .iter()
                .map(|r| r.vertex_id)
                .collect::<Vec<_>>(),
            vec![grace],
            "as-of the instant before the move, the original claim is truth"
        );
    });
}

/// A missed supersession leaves nothing behind: the replacement is selected
/// from the closing row, so a caller who names a claim that is already
/// closed does not get a second claim asserting the same thing.
#[test]
fn a_supersession_that_matches_nothing_inserts_nothing() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let tenant = admit_tenant(pool).await;
        let ada = vertex(pool, tenant, Graph::Entity, "ada").await;
        let grace = vertex(pool, tenant, Graph::Entity, "grace").await;
        let replacement_id = GraphEdgeId::new();

        let missed = graph::supersede(
            pool,
            tenant,
            Graph::Entity,
            GraphEdgeId::new(), // no such claim
            base() + chrono::Duration::days(1),
            replacement_id,
            &edge_state(ada, grace, "reports_to"),
        )
        .await
        .expect("supersede a claim that is not there");
        assert!(missed.is_none(), "nothing matched");
        assert!(
            graph::edge(pool, tenant, replacement_id)
                .await
                .expect("read")
                .is_none(),
            "and nothing was inserted"
        );
    });
}

/// Expansion is undirected — a seed matches either endpoint — and the
/// second hop reaches the second ring and stops there. `Depth` has no third
/// variant, so "and stops there" is a property of the type, not of a
/// counter this test could catch off by one.
#[test]
fn expansion_is_undirected_and_two_hops_reach_the_second_ring() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let tenant = admit_tenant(pool).await;
        let a = vertex(pool, tenant, Graph::Entity, "a").await;
        let b = vertex(pool, tenant, Graph::Entity, "b").await;
        let c = vertex(pool, tenant, Graph::Entity, "c").await;
        let d = vertex(pool, tenant, Graph::Entity, "d").await;

        // a → b → c → d: the third ring must stay out of reach.
        link(pool, tenant, Graph::Entity, a, b, "knows").await;
        link(pool, tenant, Graph::Entity, b, c, "knows").await;
        link(pool, tenant, Graph::Entity, c, d, "knows").await;

        let mut conn = pool.acquire().await.expect("acquire");
        let one = graph::expand(
            &mut conn,
            tenant,
            Graph::Entity,
            &[a],
            Depth::One,
            base(),
            None,
        )
        .await
        .expect("one hop");
        assert_eq!(
            one.reached.iter().map(|r| r.vertex_id).collect::<Vec<_>>(),
            vec![b]
        );

        let two = graph::expand(
            &mut conn,
            tenant,
            Graph::Entity,
            &[a],
            Depth::Two,
            base(),
            None,
        )
        .await
        .expect("two hops");
        let reached: Vec<GraphVertexId> = two.reached.iter().map(|r| r.vertex_id).collect();
        assert!(reached.contains(&b) && reached.contains(&c), "b and c");
        assert!(!reached.contains(&d), "the third ring is out of reach");
        assert_eq!(two.edges.len(), 2, "a→b and b→c, each once");
        for hit in &two.reached {
            let expected = if hit.vertex_id == b { 1 } else { 2 };
            assert_eq!(hit.hop, expected, "reported at its nearest hop");
        }

        // Undirected: seeding the *target* finds the same claim.
        let inbound = graph::expand(
            &mut conn,
            tenant,
            Graph::Entity,
            &[b],
            Depth::One,
            base(),
            None,
        )
        .await
        .expect("inbound");
        let reached: Vec<GraphVertexId> = inbound.reached.iter().map(|r| r.vertex_id).collect();
        assert!(reached.contains(&a), "a seed matches either endpoint");
        assert!(reached.contains(&c));
    });
}

/// The named graph is a discriminator the traversal cannot omit (decision
/// 2), and the schema will not let one claim join two of them (decision 7):
/// the same key in two graphs is two vertices, and an edge between them
/// cannot be represented.
#[test]
fn a_traversal_cannot_cross_a_named_graph() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let tenant = admit_tenant(pool).await;
        let ada_entity = vertex(pool, tenant, Graph::Entity, "ada").await;
        let grace_entity = vertex(pool, tenant, Graph::Entity, "grace").await;
        let ada_episode = vertex(pool, tenant, Graph::Episode, "ada").await;
        assert_ne!(
            ada_entity, ada_episode,
            "the same key in two graphs is two vertices"
        );

        link(
            pool,
            tenant,
            Graph::Entity,
            ada_entity,
            grace_entity,
            "reports_to",
        )
        .await;

        let mut conn = pool.acquire().await.expect("acquire");
        let episode_view = graph::expand(
            &mut conn,
            tenant,
            Graph::Episode,
            &[ada_entity],
            Depth::Two,
            base(),
            None,
        )
        .await
        .expect("expand the episode graph");
        assert!(
            episode_view.edges.is_empty(),
            "an entity claim is invisible to the episode graph"
        );

        // And the write side refuses it structurally: the composite foreign
        // key has no row to point at.
        let crossing = graph::insert_edge(
            pool,
            GraphEdgeId::new(),
            tenant,
            Graph::Entity,
            &edge_state(ada_entity, ada_episode, "mentions"),
        )
        .await;
        assert!(
            crossing.is_err(),
            "an edge between two graphs is unrepresentable"
        );
    });
}

/// The seed set is bounded and the empty case never reaches the database
/// (decision 9, and CTX-1's "no unfiltered code path" inherited).
#[test]
fn expansion_bounds_its_seed_set_and_shortcuts_the_empty_one() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let tenant = admit_tenant(pool).await;
        let mut conn = pool.acquire().await.expect("acquire");

        let empty = graph::expand(
            &mut conn,
            tenant,
            Graph::Entity,
            &[],
            Depth::Two,
            base(),
            None,
        )
        .await
        .expect("an empty seed set is not an error");
        assert!(empty.edges.is_empty() && empty.reached.is_empty());

        let too_many: Vec<GraphVertexId> = (0..=graph::MAX_EXPANSION_SEEDS)
            .map(|_| GraphVertexId::new())
            .collect();
        let refused = graph::expand(
            &mut conn,
            tenant,
            Graph::Entity,
            &too_many,
            Depth::One,
            base(),
            None,
        )
        .await;
        assert!(
            matches!(refused, Err(Error::Invalid { .. })),
            "past the bound is a named error, not a slow query: {refused:?}"
        );
    });
}

/// A vertex is identity, and identity converges (decision 5): a second
/// upsert of the same `(graph, kind, key)` returns the first writer's id,
/// refreshes the label, and never unlinks a backing record it does not know
/// about.
#[test]
fn vertices_converge_on_their_resolution_key() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let tenant = admit_tenant(pool).await;

        let first = graph::upsert_vertex(
            pool,
            GraphVertexId::new(),
            tenant,
            Graph::Entity,
            &VertexState {
                kind: "person".to_owned(),
                key: "ada-lovelace".to_owned(),
                label: "Ada".to_owned(),
                record_id: None,
            },
        )
        .await
        .expect("first mention");

        let second = graph::upsert_vertex(
            pool,
            GraphVertexId::new(), // a fresh id, deliberately
            tenant,
            Graph::Entity,
            &VertexState {
                kind: "person".to_owned(),
                key: "ada-lovelace".to_owned(),
                label: "Ada Lovelace".to_owned(),
                record_id: None,
            },
        )
        .await
        .expect("second mention");

        assert_eq!(second.id, first.id, "first writer wins the identifier");
        assert_eq!(
            second.state.label, "Ada Lovelace",
            "the newest observation names the thing best"
        );
        assert_eq!(
            graph::vertices(pool, tenant, &[first.id])
                .await
                .expect("read back")
                .len(),
            1,
            "and there is only ever one row"
        );
    });
}

/// Confidence is refused outside its range with a message that names the
/// number, before the CHECK constraint has to (the MEM-5 discipline).
#[test]
fn confidence_outside_the_range_is_a_named_error() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let tenant = admit_tenant(pool).await;
        let a = vertex(pool, tenant, Graph::Entity, "a").await;
        let b = vertex(pool, tenant, Graph::Entity, "b").await;

        let mut state = edge_state(a, b, "knows");
        state.confidence_permille = 1001;
        let refused =
            graph::insert_edge(pool, GraphEdgeId::new(), tenant, Graph::Entity, &state).await;
        match refused {
            Err(Error::Invalid { message }) => {
                assert!(message.contains("1001"), "names the number: {message}");
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
    });
}

// ── Clause three: the shipped statements' plans ──────────────────────────────

/// Vertices and edges the plan fixture seeds.
///
/// The size is load-bearing and was found by measurement, not chosen.
/// Below roughly 25,000 edges a sequential scan is **the correct plan** for
/// the second hop's join — the planner is right, the table fits in a
/// handful of pages, and asserting an index scan there would be asserting a
/// worse plan. The discipline decision 9 protects is about the traversal
/// staying index-backed at the scale a real graph lives at, so the fixture
/// sits an order of magnitude past that crossover: at 200,000 edges the
/// sequential alternative costs ~8× the indexed one, which is margin enough
/// that this guard reports regressions rather than hardware.
///
/// If this test ever fails on a nearly empty `graph_edges`, read the
/// printed plan before believing it: a seq scan over a 20,000-row table is
/// not the regression this is looking for.
const PLAN_VERTICES: i64 = 20_000;
const PLAN_EDGES: i64 = 200_000;
/// Edges rewritten once, so `graph_edges_history` — the other half of the
/// as-of statements' view — is not an empty table when its plan is read.
const PLAN_HISTORY_ROWS: i64 = 20_000;

/// The seed-set size GRPH-3 will issue: hybrid retrieval hands recall a
/// ranked hit set and the graph expands around it (ADR-0029 G1).
const SEED_SET: usize = 10;

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

    /// `n` distinct picks from `pool`.
    fn pick(&mut self, pool: &[GraphVertexId], n: usize) -> Vec<GraphVertexId> {
        let mut out: Vec<GraphVertexId> = Vec::with_capacity(n);
        while out.len() < n {
            let candidate = pool[(self.next() as usize) % pool.len()];
            if !out.contains(&candidate) {
                out.push(candidate);
            }
        }
        out
    }
}

/// Begins a transaction with the tenant GUC set, then demotes it to
/// `synveda_app` — the shape data-path code must use, and the only way to
/// read a plan or a duration with the RLS predicate actually applied (the
/// compose superuser bypasses it). `SET LOCAL ROLE` reverts with the
/// transaction, like the GUC.
async fn app_tx(pool: &PgPool, tenant: TenantId) -> Transaction<'static, Postgres> {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant transaction");
    sqlx::raw_sql("set local role synveda_app")
        .execute(&mut *tx)
        .await
        .expect("SET ROLE synveda_app (the test connection must be allowed to)");
    tx
}

/// Seeds `vertices` vertices and `edges` edges of `Graph::Entity` for
/// `tenant`, at the spike's shape: each source relates to `edges/vertices`
/// distinct targets. Runs as the connecting role (the compose superuser),
/// because `analyze` needs ownership; every *measured* statement then runs
/// as `synveda_app`.
///
/// Identifiers are **derived arithmetically** from the ordinal and the
/// tenant — `<16 hex of tenant><16 hex of n>` — rather than minted and
/// joined back. The first version of this fixture resolved endpoints by
/// joining the edge series against the vertex table on a computed key, and
/// the planner chose a nested loop over that expression: 200,000 edges took
/// five and a half minutes, and the cost depended on the vertex count in a
/// way that made a fixture's runtime a coin toss. With ids as a function of
/// the ordinal there is no join to plan, and the seed is deterministic
/// besides.
async fn seed_corpus(pool: &PgPool, tenant: TenantId, vertices: i64, edges: i64) {
    sqlx::query!(
        r#"
        insert into graph_vertices (id, tenant_id, graph, kind, key, label)
        select (substring(replace($1::uuid::text, '-', '') from 1 for 16)
                || lpad(to_hex(n), 16, '0'))::uuid,
               $1, 'entity', 'person', 'v' || n, 'v' || n
        from generate_series(1::bigint, $2::bigint) n
        "#,
        tenant.as_uuid(),
        vertices,
    )
    .execute(pool)
    .await
    .expect("seed vertices");

    // dst mixes in e/vertices so a source's edges land on that many
    // *distinct* targets: a pure `e * prime % vertices` gives every edge of
    // a vertex the same destination, and a 2-hop over that measures nothing
    // (the spike's fixture, restated on the product schema).
    sqlx::query!(
        r#"
        insert into graph_edges
            (id, tenant_id, graph, kind, src_id, dst_id, method,
             confidence_permille, valid_from, tx_from)
        select (substring(replace($1::uuid::text, '-', '') from 1 for 16)
                || lpad(to_hex(1 + e + $2::bigint), 16, '0'))::uuid,
               $1, 'entity', 'knows',
               (substring(replace($1::uuid::text, '-', '') from 1 for 16)
                || lpad(to_hex(1 + (e % $2::bigint)), 16, '0'))::uuid,
               (substring(replace($1::uuid::text, '-', '') from 1 for 16)
                || lpad(to_hex(1 + ((e * 7919 + (e / $2::bigint) * 104729)
                                    % $2::bigint)), 16, '0'))::uuid,
               'grph-1 fixture', 900, $4, now()
        from generate_series(1::bigint, $3::bigint) e
        where (e % $2::bigint)
              <> ((e * 7919 + (e / $2::bigint) * 104729) % $2::bigint)
        "#,
        tenant.as_uuid(),
        vertices,
        edges,
        base(),
    )
    .execute(pool)
    .await
    .expect("seed edges");

    for table in ["graph_vertices", "graph_edges", "graph_edges_history"] {
        sqlx::raw_sql(&format!("analyze {table}"))
            .execute(pool)
            .await
            .expect("analyze");
    }
}

/// Rewrites `rows` of the tenant's edges so the history table holds real
/// versions. Each update archives one row through the trigger.
async fn seed_history(pool: &PgPool, tenant: TenantId, rows: i64) {
    sqlx::query!(
        r#"
        update graph_edges set confidence_permille = 901
        where id in (
            select id from graph_edges
            where tenant_id = $1 and graph = 'entity'
            order by id
            limit $2
        )
        "#,
        tenant.as_uuid(),
        rows,
    )
    .execute(pool)
    .await
    .expect("archive a slice of the corpus");
    sqlx::raw_sql("analyze graph_edges_history")
        .execute(pool)
        .await
        .expect("analyze history");
}

/// A deterministic sample of the corpus's vertex ids, for seed sets.
async fn vertex_pool(pool: &PgPool, tenant: TenantId, n: i64) -> Vec<GraphVertexId> {
    sqlx::query_scalar!(
        r#"
        select id as "id!" from graph_vertices
        where tenant_id = $1 and graph = 'entity'
        order by key
        limit $2
        "#,
        tenant.as_uuid(),
        n,
    )
    .fetch_all(pool)
    .await
    .expect("sample vertices")
    .into_iter()
    .map(GraphVertexId::from_uuid)
    .collect()
}

/// The traversal statements **this crate ships**, read out of `src/graph.rs`
/// by the `-- shipped-traversal:` marker each carries, keyed by that
/// marker's label.
///
/// Reading the source rather than restating the SQL is the whole point: a
/// plan guard that explains a copy of the query proves nothing about the
/// query. `include_str!` resolves at compile time, so this can never lag the
/// module it guards.
fn shipped_statements() -> BTreeMap<String, String> {
    const SOURCE: &str = include_str!("../src/graph.rs");
    const MARKER: &str = "-- shipped-traversal:";

    let mut found = BTreeMap::new();
    for chunk in SOURCE.split("r#\"").skip(1) {
        let Some(sql) = chunk.split("\"#").next() else {
            continue;
        };
        let Some(marker_line) = sql.lines().find(|line| line.trim().starts_with(MARKER)) else {
            continue;
        };
        let label = marker_line.trim()[MARKER.len()..].trim().to_owned();
        assert!(
            found.insert(label.clone(), sql.to_owned()).is_none(),
            "two shipped statements share the label {label:?}"
        );
    }

    // The guard that makes the marker load-bearing: every statement inside
    // `expand` must carry one, so a fifth traversal cannot be added without
    // joining the plan assertion.
    let body = SOURCE
        .split_once("pub async fn expand(")
        .expect("expand exists")
        .1
        .split_once("fn fold(")
        .expect("fold follows expand")
        .0;
    assert_eq!(
        body.matches("sqlx::query_as!").count(),
        found.len(),
        "a traversal statement in `expand` is missing its `{MARKER}` marker, \
         so the plan assertion would silently skip it"
    );
    found
}

/// `explain (format json)` for one shipped statement, with the parameters
/// the traversal binds. Five-parameter statements are the as-of pair.
async fn plan_of(
    tx: &mut Transaction<'static, Postgres>,
    sql: &str,
    tenant: TenantId,
    seeds: &[GraphVertexId],
    valid_at: DateTime<Utc>,
    as_of: DateTime<Utc>,
) -> serde_json::Value {
    let seed_uuids: Vec<Uuid> = seeds.iter().map(|id| id.as_uuid()).collect();
    let explained = format!("explain (format json) {sql}");
    let query = sqlx::query(&explained)
        .bind(tenant.as_uuid())
        .bind(Graph::Entity.as_str())
        .bind(&seed_uuids)
        .bind(valid_at);
    let query = if sql.contains("$5") {
        query.bind(as_of)
    } else {
        query
    };
    let row = query
        .fetch_one(&mut **tx)
        .await
        .expect("explain the shipped statement");
    row.try_get::<serde_json::Value, _>(0).expect("plan json")
}

/// Every `(node type, relation)` pair the plan touches, flattened.
fn scan_nodes(plan: &serde_json::Value, out: &mut Vec<(String, String)>) {
    match plan {
        serde_json::Value::Array(items) => {
            for item in items {
                scan_nodes(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            if let (Some(node), Some(relation)) = (
                map.get("Node Type").and_then(|v| v.as_str()),
                map.get("Relation Name").and_then(|v| v.as_str()),
            ) {
                out.push((node.to_owned(), relation.to_owned()));
            }
            for value in map.values() {
                scan_nodes(value, out);
            }
        }
        _ => {}
    }
}

/// Clause three (decision 9): none of the four shipped traversal statements
/// plans a sequential scan over `graph_edges` — nor over
/// `graph_edges_history`, which the as-of pair reads through the versions
/// view and which migration 0026 indexes for exactly this reason.
#[test]
fn the_shipped_traversal_statements_never_plan_a_sequential_scan() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let tenant = admit_tenant(pool).await;
        seed_corpus(pool, tenant, PLAN_VERTICES, PLAN_EDGES).await;
        seed_history(pool, tenant, PLAN_HISTORY_ROWS).await;

        let statements = shipped_statements();
        assert_eq!(
            statements.len(),
            4,
            "expected the four (depth × time mode) statements, found: {:?}",
            statements.keys().collect::<Vec<_>>()
        );

        let pool_ids = vertex_pool(pool, tenant, 500).await;
        let seeds = Lcg(0x6741_5048).pick(&pool_ids, SEED_SET);
        let mut tx = app_tx(pool, tenant).await;

        for (label, sql) in &statements {
            let plan = plan_of(&mut tx, sql, tenant, &seeds, base(), Utc::now()).await;
            let mut nodes = Vec::new();
            scan_nodes(&plan, &mut nodes);
            assert!(
                !nodes.is_empty(),
                "{label}: the plan named no relation at all"
            );
            eprintln!("  {label}: {nodes:?}");
            for (node, relation) in &nodes {
                assert!(
                    !(node == "Seq Scan"
                        && (relation == "graph_edges" || relation == "graph_edges_history")),
                    "{label} plans a sequential scan over {relation} \
                     — the adjacency discipline has regressed (ADR-0043 decision 9). \
                     Full plan: {plan}"
                );
            }
        }
    });
}

// ── Decision 15: the measurement, re-taken on the shipped schema ────────────

/// The spike's 1M row, for comparison: synthetic `(src, dst)` adjacency with
/// no tenant column, no RLS, no bitemporal predicate and no foreign keys
/// (docs/spikes/grph-4-age-traversal.md).
const SPIKE_1M_HOP1_MS: f64 = 0.84;
const SPIKE_1M_HOP2_MS: f64 = 2.05;

const MEASURE_EDGES: i64 = 1_000_000;
const MEASURE_VERTICES: i64 = MEASURE_EDGES / 10;
const MEASURE_QUERIES: usize = 200;
/// ADR-0029 G1's own threshold, which this measurement re-takes on the
/// schema that shipped: **median** ≤ 50ms, tails reported against the
/// 150ms slice the recall decomposition reserves for graph expansion.
const MEDIAN_BUDGET: Duration = Duration::from_millis(50);
const EXPANSION_SLICE: Duration = Duration::from_millis(150);

struct Stats {
    median: Duration,
    p95: Duration,
    max: Duration,
    rows: usize,
}

/// Nearest-rank percentiles, matching the eval harness (ADR-0028) and the
/// spike.
fn stats(mut samples: Vec<Duration>, rows: usize) -> Stats {
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

/// Decision 15: the spike measured a synthetic adjacency table; this
/// measures `graph_edges` **as built** — RLS predicate applied (the queries
/// run as `synveda_app`), bitemporal columns, composite foreign keys, tenant
/// index — at the spike's own shape, 10 seeds and out-degree 10.
///
/// The median is asserted and the tails are reported: virtualised dev IO
/// owns the upper percentiles, and EVAL-6 owns percentile SLO enforcement on
/// production-shaped hardware (the HIER-1/MEM-1/CTX-1 discipline).
#[test]
#[ignore = "seeds a million edges; run alone with --ignored --nocapture"]
fn traversal_medians_on_the_shipped_schema() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let tenant = admit_tenant(pool).await;

        eprintln!(
            "seeding {MEASURE_EDGES} edges over {MEASURE_VERTICES} vertices \
             (out-degree 10) — this takes a few minutes"
        );
        let seeding = Instant::now();
        seed_corpus(pool, tenant, MEASURE_VERTICES, MEASURE_EDGES).await;
        eprintln!("  seeded in {:.1?}", seeding.elapsed());

        let pool_ids = vertex_pool(pool, tenant, 5_000).await;
        let mut tx = app_tx(pool, tenant).await;

        let mut results = Vec::new();
        for (label, depth, baseline) in [
            ("1-hop", Depth::One, SPIKE_1M_HOP1_MS),
            ("2-hop", Depth::Two, SPIKE_1M_HOP2_MS),
        ] {
            let mut lcg = Lcg(0x4752_5048);
            let mut samples = Vec::with_capacity(MEASURE_QUERIES);
            let mut rows = 0;
            for _ in 0..MEASURE_QUERIES {
                let seeds = lcg.pick(&pool_ids, SEED_SET);
                let started = Instant::now();
                let found =
                    graph::expand(&mut tx, tenant, Graph::Entity, &seeds, depth, base(), None)
                        .await
                        .expect("expand");
                samples.push(started.elapsed());
                rows = found.edges.len();
            }
            results.push((label, stats(samples, rows), baseline));
        }

        eprintln!(
            "\nGRPH-1 traversal on the shipped schema \
             ({MEASURE_EDGES} edges / {MEASURE_VERTICES} vertices, \
             {SEED_SET} seeds, RLS on, {MEASURE_QUERIES} queries)\n"
        );
        eprintln!("  shape    median      p95      max    edges   spike (synthetic)");
        for (label, stats, baseline) in &results {
            eprintln!(
                "  {label:<7} {:>6}ms {:>6}ms {:>6}ms {:>8}   {baseline:>6.2}ms",
                ms(stats.median),
                ms(stats.p95),
                ms(stats.max),
                stats.rows,
            );
        }
        eprintln!(
            "\n  median asserted ≤ {}ms (ADR-0029 G1); p95 reported against the \
             {}ms expansion slice; tails are dev IO's and EVAL-6's, not this \
             test's.",
            ms(MEDIAN_BUDGET),
            ms(EXPANSION_SLICE),
        );
        eprintln!(
            "  The spike column is NOT like-for-like and is printed for scale, \
             not for a ratio: it measured a directed join projecting one bigint \
             column, while this expands UNDIRECTED at both hops and returns whole \
             edge rows — the 2-hop shape above answers a question roughly four \
             times larger, and most of its cost is materialising those rows, not \
             finding them.\n"
        );

        for (label, stats, _) in &results {
            assert!(
                stats.median <= MEDIAN_BUDGET,
                "{label} median {} exceeds the {}ms budget at {MEASURE_EDGES} edges",
                ms(stats.median),
                ms(MEDIAN_BUDGET),
            );
        }
    });
}
