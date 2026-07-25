# GRPH-4 — AGE traversal spike & graph fallback assessment

- **Date**: 2026-07-25
- **Feature**: GRPH-4 (de-risk, Phase 2 gate)
- **Criteria**: [ADR-0029](../adr/adr-0029-graph-traversal-gate.md), pre-registered before the run
- **Harness**: `crates/synveda-store/tests/graph_spike.rs` (`#[ignore]`d), `demos/grph-4-graph-spike.sh`
- **Hardware**: Apple Silicon, Postgres 17.10 + AGE 1.7.0 under OrbStack (virtualised IO)

## Summary

AGE **passes the latency gate and fails three of the other four criteria.**
Traversal speed — the thing ADR-0001 and ADR-0004 both flagged as unproven
and the thing this spike was commissioned to test — turns out not to be
AGE's problem. Its problems are query ergonomics, catalog cost, and a
head-on collision with this repo's SQL discipline.

| Criterion | Threshold | Result | |
|---|---|---|---|
| **G1** traversal latency | median ≤ 50ms @ 1M and 10M | 2-hop 8.18ms / 12.91ms | **pass**¹ |
| **G2** scale slope | 10M ≤ 3× 1M | 1.58× (2-hop), 2.45× (1-hop) | **pass**¹ |
| **G3** edge write | median ≤ 10ms | 10.42ms via Cypher @10M | **fail**² |
| **G4** tenant multiplicity | < 25,000 relations @ 1,000 tenants | 48,000 | **fail** |
| **G5** SQL discipline | no string-built SQL | graph name must be a literal | **fail** |
| **G6** transactionality + RLS | both | rollback ✓, forced RLS honoured ✓ | **pass** |

¹ Only for query forms written a specific way — see "The three traps".
² Passes at 0.01ms by bypassing Cypher on writes — see G3 below.

**Verdict: conditional go.** AGE is retained, but ADR-0004's central
decision — *multiple named graphs per tenant* — does not survive G4 and
G5 and is amended: one shared set of named graphs, tenant carried as a
property, RLS as the isolation backstop (verified below).

**Recommendation to GRPH-1, stated plainly:** the relational adjacency
baseline cleared *every* criterion, including both that AGE failed, and
was 3–8× faster on the traversals themselves with 2.5× less storage. The
burden of proof has moved: GRPH-1 should adopt plain indexed adjacency
unless a concrete requirement for Cypher-expressed traversal appears.
That decision belongs in GRPH-1's design ADR — G4's pre-registered
consequence explicitly makes graph partitioning a schema decision — and
this report is the evidence in front of it.

## Method

Both scales hold out-degree at 10, so the neighbourhood a traversal
touches is identical and only the surrounding data volume grows. That is
what makes G2's slope mean "is this index-backed" rather than "is this
more work".

- **1M scale**: 100,000 vertices, 1,000,000 edges
- **10M scale**: 1,000,000 vertices, 10,000,000 edges
- Seed set: 10 entities per query — the shape GRPH-3 issues, where hybrid
  retrieval hands recall a ranked hit set and the graph expands around it
- 200 iterations per measurement (20 for the two pathological forms),
  fresh deterministic seed set each iteration
- AGE was given its best shot: a GIN index on vertex properties (which AGE
  does **not** create for you) and `ANALYZE` on every label table

## Results

Median / p95 / max, milliseconds, client-side including round trip.

| Measurement | scale | median | p95 | max |
|---|---|---|---|---|
| age 1-hop, 10 seeds (UNION ALL, indexed) | 1M | **3.81** | 4.54 | 16.31 |
| age 2-hop, 10 seeds (explicit, indexed) | 1M | **8.18** | 8.97 | 33.82 |
| age 1-hop, 10 seeds (`IN` list) | 1M | 29.88 | 30.97 | 31.30 |
| age 1..2-hop, 1 seed (`*1..2` VLE) | 1M | 408.63 | 414.34 | 666.91 |
| sql 1-hop, 10 seeds (adjacency, bound) | 1M | 0.84 | 1.11 | 2.25 |
| sql 2-hop, 10 seeds (adjacency, bound) | 1M | 2.05 | 2.78 | 3.10 |
| age single edge create (cypher) | 1M | 2.22 | 3.82 | 19.17 |
| sql single edge insert | 1M | 1.01 | 1.50 | 5.64 |
| age 1-hop, 10 seeds (UNION ALL, indexed) | 10M | **9.35** | 10.92 | 35.47 |
| age 2-hop, 10 seeds (explicit, indexed) | 10M | **12.91** | 13.99 | 16.10 |
| age 1-hop, 10 seeds (`IN` list) | 10M | 211.77 | 217.97 | 325.30 |
| age 1..2-hop, 1 seed (`*1..2` VLE) | 10M | 3736.71 | 10304.59 | 13592.41 |
| sql 1-hop, 10 seeds (adjacency, bound) | 10M | 1.24 | 1.42 | 2.25 |
| sql 2-hop, 10 seeds (adjacency, bound) | 10M | 4.84 | 7.40 | 8.07 |
| age single edge create (cypher) | 10M | 10.42 | 18.02 | 25.86 |
| sql single edge insert | 10M | 1.46 | 1.67 | 2.13 |

Storage and load, same data: at 1M edges AGE's edge table is 173 MB
against the adjacency table's 68 MB — 2.5×, measured; the 10M figures
scale from there but were not measured directly. Bulk load at 10M was
39.8s for AGE against 6.0s for the adjacency table, measured.

## The three traps

This is the spike's most useful output. **Three of the four ways a
competent person would write these queries are 20×–400× slower than the
one that works**, and nothing in the query tells you which one you wrote.

1. **`WHERE a.eid IN [...]` falls off every index.** The natural way to
   seed a traversal from a hit set plans as a hash join over the *entire*
   edge table: 29.88ms at 1M, 211.77ms at 10M, slope 7.09× — the signature
   of a scan. The `OR`-chain equivalent is worse still (117ms at 1M).
   The only form AGE plans with its indexes is single-seed property
   equality, `MATCH (a:Entity {eid: 500})`, so a 10-seed expansion must be
   written as **ten single-seed branches joined with `UNION ALL`**.
2. **Variable-length `*1..2` is unusable.** 408ms at 1M and 3.7s at 10M
   (p95 10.3s), via a `Function Scan on age_vle` that joins back against a
   full vertex scan. The identical traversal written as an explicit
   two-hop pattern is **0.43ms** — a 2000× difference for the same
   semantics. Variable-length paths are the feature that distinguishes a
   graph engine from a join, and it is the slowest thing measured here.
3. **`WHERE id(x) = <graphid>` does not use the primary key.** Matching a
   vertex by its own internal id seq-scans the label table: an edge create
   by graphid took **689ms** at 10M against 7.9ms for the same create
   matching on an indexed property. The intuition that "ids are faster
   than properties" is exactly inverted here.

Three milder landmines, each of which cost time during this spike:

- **No property index unless you build one.** Without a hand-created GIN
  index on `properties`, every seed lookup is a full label scan — 12.5ms
  at 100,000 vertices, growing linearly. AGE creates `start_id`/`end_id`
  indexes on edge tables automatically but nothing on vertex properties.
- **Bulk loading desynchronises the label sequences.** Inserting through
  the label tables (the only viable way to load at scale — AGE ships no
  bulk loader, and `cypher CREATE` would take hours at 10M) leaves each
  label's `_id_seq` behind, so the *next* Cypher `CREATE` collides on the
  primary key. `setval` after every bulk load is an obligation on GRPH-2,
  and this demo tripped over it before it was one.
- **Graph names must be at least three characters.** `create_graph('ab')`
  fails with `graph name is invalid` while `abc` succeeds; digits are
  fine at any position. Harmless under the amended single-shared-set
  design, but a trap for anything that generates graph names.

## Criterion detail

### G1, G2 — pass, conditionally

With the disciplined forms, 2-hop expansion from 10 seeds costs 12.91ms
median at 10M edges against a 50ms threshold and a 150ms p95 slice of the
recall budget. The slope from 1M to 10M is 1.58×, well inside the 3×
bound and consistent with an index-backed local traversal. There is real
headroom here.

The conditionality is the point, though: the same criteria measured on the
natural query forms fail outright (211.77ms, slope 7.09×). **G1 passes
only if GRPH-1 makes the disciplined forms the only ones that exist** —
which means a query builder that emits `UNION ALL` branches and explicit
hop patterns, never `IN` and never `*n..m`, and a test that fails if a
plan contains a sequential scan over a label table.

### G3 — fail as measured, pass with the mitigation

10.42ms median at 10M against a 10ms threshold: a marginal failure, and
the pre-registered rule for a G3-only failure was to move graph-linking
out of the record transaction, costing ADR-0001's commit-together
property. That is not necessary. Measured server-side at 10M edges:

| Edge write form | mean |
|---|---|
| Cypher `CREATE`, endpoints matched by property | 7.90ms |
| Cypher `CREATE`, endpoints matched by `id()` | 689.05ms |
| Direct `INSERT` into the label table | **0.01ms** |

Writing edges as plain SQL inserts into AGE's own label tables — which is
what the bulk loader does, and what Cypher `MATCH` reads back correctly —
is 790× faster than the Cypher that AGE exists to provide, and keeps the
write inside the record's transaction. GRPH-2 should write edges this way
and keep the label sequences in step. The cause of Cypher's ~8ms is not
isolated here; it is not the property lookup (GIN, ~0.03ms) and not the
insert (0.01ms).

### G4 — fail

One tenant's three graphs cost **48 relations** and 19.9ms of DDL. At
1,000 tenants that is 48,000 relations against a 25,000 ceiling — and 48
is a *floor*, measured with one vertex label and one edge label per graph,
where ADR-0004's schema has several of each. DDL time extrapolates to
19.9s for 1,000 tenants, comfortably inside the 5-minute bound; the
catalog is the problem, not the clock.

### G5 — fail

Established interactively, before the harness was written:

- `cypher($1, $$…$$)` → `ERROR: a name constant is expected`
- `cypher('g', $$…$$, $1::agtype)` → `ERROR: third argument of cypher
  function must be a parameter`

The graph name must be literal text in the statement, and the parameter
map must be a bare `$n` bound as `agtype` — a type sqlx has no encoder for
without a custom implementation. **With one graph per tenant, the
statement text necessarily varies per tenant**, which is string-built SQL
(CLAUDE.md admits none, ever) and breaks ADR-0001's compliance promise
that reviewers can enumerate every SQL statement in the binary. It also
means N tenants × M query shapes distinct prepared statements.

This is the criterion that, with G4, condemns per-tenant graphs
specifically — not AGE.

### G6 — pass, and it is what makes the fix safe

- **Transactionality**: a Cypher `CREATE` inside `BEGIN … ROLLBACK`
  leaves nothing behind. ADR-0001's commit-together claim holds.
- **RLS**: forced row-level security on AGE label tables is honoured by
  Cypher traversals when the connection is a non-superuser. Verified with
  the real predicate shape, not just `USING (false)` — one shared graph,
  tenant carried as a vertex property, policy keyed to TEN-2's
  per-transaction GUC:

  | Session | rows via Cypher |
  |---|---|
  | `synveda.tenant_id = 'tenant-a'` | 50 (its own) |
  | `synveda.tenant_id = 'tenant-b'` | 50 (its own) |
  | GUC unset | **0** — fails closed |

  A deployment note falls out: the non-superuser app role needs `USAGE`
  on `ag_catalog` and `SELECT` on its catalog tables (OPS-1/OPS-2).

## What ADR-0004 must change

G4 and G5 both point at the same clause, and G6 shows the way out.

- **Drop per-tenant graph instantiation.** Keep the multi-graph semantic
  partitioning (entity / episode / provenance) that the MAGMA research
  supports — but as one shared set, not one set per tenant. Relations fall
  from 48-per-tenant to 48 total, and the graph name becomes a
  compile-time constant, so statements are static and sqlx-checkable.
- **Carry `tenant_id` as a property** with an RLS policy keyed to the
  TEN-2 GUC, plus an index on the tenant property. This replaces the
  structural "cross-tenant edges are impossible" argument with an enforced
  one — which is what TEN-2 exists to say is the stronger guarantee.
- **What is lost, recorded honestly**: TEN-5's tenant deletion stops being
  "drop the tenant's graphs" and becomes a delete by tenant property;
  ADR-0004's per-graph decay lifecycle (MEM-6) likewise becomes predicated
  rather than structural.

## Fallback assessment

**Not activated.** The pre-registered trigger was a G1 or G2 failure, and
neither fired: AGE's traversal latency is inside budget with headroom.
Adding an engine is the expensive answer to a question that was not asked.

The finding that matters for any future activation is that **there is no
drop-in, licence-compatible, embedded property-graph engine to fall back
to.** The embedded engine the original ADRs named as the fallback is no
longer actively maintained and is struck from the ladder. Of what remains,
the mature property-graph engines fail the core-path licence rule
(MIT / Apache-2.0 / PostgreSQL, enforced by cargo-deny) — Neo4j is
GPL/commercial, Memgraph and SurrealDB are BSL — which is what ADR-0004's
option 3 already recorded. The tech plan's own noted candidate, IndraDB,
is MPL-2.0: outside the allowlist, so it would need an explicit
cargo-deny exception and an ADR of its own. Oxigraph clears the licence
but is RDF/SPARQL rather than a property graph, so the data model does not
fit. And any of them reintroduces a second engine, which reopens
everything ADR-0001 closed: two-phase commit between the record write and
the graph write on every ingestion event, a separate backup/DR and
encryption-at-rest story, no equivalent of the TEN-2 RLS backstop that G6
just proved works on AGE, and — if embedded — a stateful gateway that
OPS-2's horizontally-scaled profile is not designed for.

So the ladder runs inside Postgres, and this spike has already measured
its first rung:

1. **Plain indexed adjacency.** 1.24ms (1-hop) and 4.84ms (2-hop) medians
   at 10M edges, 3–8× faster than AGE with 2.5× less storage, bound
   parameters, and no catalog cost. This is ADR-0004's rejected option 4,
   and on this evidence it is the strongest option for GRPH-3's fixed
   1–2 hop requirement.
2. **A materialised k-hop closure table.** The HIER-1 pattern — closure
   table plus materialised path, already in this repo for the org
   hierarchy — generalised to the entity graph: precompute the bounded
   neighbourhood and trade write amplification and storage for a single
   indexed read. The boring answer if adjacency joins ever stop clearing
   budget.
3. **A dedicated engine, with its own ADR and an explicit licence
   exception.** Only for a requirement the first two structurally cannot
   serve.

**Recorded trigger conditions** — revisit the ladder if any of these
become true:

1. Traversal depth beyond 2 hops, or genuinely variable-length paths,
   become a product requirement (AGE's VLE is 0.4–3.7s and there is no
   disciplined rewrite for unbounded depth).
2. Graph expansion medians exceed the 150ms recall slice at production
   scale, as re-measured by EVAL-6 on production-shaped IO.
3. A single deployment's entity graph exceeds ~100M edges — an order
   beyond what this spike measured, where the 1.58× slope may not hold.

Triggers 2 and 3 are absorbed by rungs 1 and 2. Trigger 1 is the only one
neither can serve — and it is equally the one AGE cannot serve. If it ever
fires, the question is not "which fallback" but "accept a licence
exception and a second engine, or bound the product requirement", and that
is a decision for a fresh ADR with the licence position re-checked at the
time.

## Reproducing

```sh
demos/grph-4-graph-spike.sh          # scratch database, seeds, measures, drops
```

or against a database of your own:

```sh
DATABASE_URL=postgres://…/scratch \
  cargo test -p synveda-store --test graph_spike -- --ignored --nocapture
```

The run takes about three minutes, most of it seeding 11M edges.
