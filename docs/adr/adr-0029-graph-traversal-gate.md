# ADR-0029: The GRPH-4 gate — pre-registered criteria for Apache AGE, and the fallback ladder

- **Status**: Proposed
- **Date**: 2026-07-25
- **Feature(s)**: GRPH-4 (gate for GRPH-1..3, MEM-5, CTX-5)
- **Deciders**: sujitn

## Context

Two accepted ADRs rest on an unmeasured assumption. ADR-0001 chose one
database engine for records, vectors, queues and graph, and recorded the
cost honestly: "AGE Cypher performance is unproven at 10M+ edges", with a
failed GRPH-4 gate activating an embedded-graph fallback as its reversal
trigger. ADR-0004 chose multiple named AGE graphs per
tenant with bitemporal edges, accepted "AGE Cypher performance at scale is
unproven", and named the same gate. GRPH-4 is that gate, and it is the
only item in Phase 2 that can invalidate an accepted architectural
decision — so it runs before GRPH-1 builds a schema on top of it.

Forces at play:

- **A spike that picks its thresholds after seeing the numbers has
  measured nothing.** Every benchmark produces a number; only a
  pre-registered threshold turns it into a decision. This ADR is written
  and committed before the harness runs, and the verdict section below is
  filled in afterwards against criteria nobody could tune to the result.
  EVAL-1 made the same argument about gates a fortnight ago and it holds
  here.
- **There is no recall SLO to measure against.** Seed §10 budgets
  `inject` (p99 < 150ms) and `observe` (ack < 20ms, pipeline lag < 60s)
  and says nothing about `recall` beyond seed §3's "richer and slower".
  Graph traversal is a recall-path cost — GRPH-3 is "1–2 hop expansion in
  recall ranking", and inject never traverses — so the gate has to derive
  the budget it is gating against, and that derivation is part of the
  decision rather than an implementation detail.
- **Latency is not the only way this can fail.** ADR-0004 mandates three
  named graphs per tenant, and AGE models a graph as a schema with a table
  per label — so tenant admission becomes DDL, and a thousand tenants
  becomes a catalog question. ADR-0001's compliance note promises that
  "reviewers can enumerate every SQL statement in the binary" and
  CLAUDE.md admits no string-built SQL, while AGE's `cypher()` takes its
  query as a literal. And TEN-2's row-level-security backstop exists
  because structural isolation arguments fail in practice; whether it
  reaches into graph storage at all is a question the gate should answer
  while it has a database to hand.
- **The interesting alternative may not be another engine.** ADR-0004
  considered and rejected "no graph layer (vectors + FTS only)" as a
  v1-viable option. If AGE misses the latency criteria, the next question
  is not automatically "which graph engine instead" — it is whether plain
  indexed adjacency in Postgres, which every option here already includes,
  clears the same bar. Measuring that costs one extra table and answers
  the cheaper question first.

## Decision

The criteria below are binding and pre-registered: **G1–G3 are
measurements with thresholds, G5–G6 are binary, and G4 is measured and
extrapolated.** A relational adjacency baseline (recursive CTE over an
indexed edge table) is measured alongside AGE as a *reference*, not as a
criterion — its role is to say what a failure means, not whether one
occurred.

**The recall budget this gate measures against.** Recall gets a **300ms
p95** target: it is the figure the 2026 field publishes for hybrid
retrieval with graph traversal and no LLM calls on the read path
(features §A1, Graphiti ~300ms P95), and it is duly "richer and slower"
than inject's 150ms p99. Decomposed over the stages recall already has,
using CTX-3's measured stage split and ADR-0024's search budget:

| Stage | Source | p95 allowance |
|---|---|---|
| PDP plan (permitted scopes) | CTX-3 measured 4.5ms | 15ms |
| query embed | MEM-4 seam; CTX-3 measured 10µs deterministic | 30ms |
| hybrid search | ADR-0024 budget at 1M records/tenant | 80ms |
| **graph expansion** | **this gate** | **150ms** |
| hydration, re-verify, compose | CTX-3 measured 2.1ms | 15ms |
| audit append | CTX-3 measured 5.8ms | 10ms |
| | | **300ms** |

### The criteria

- **G1 — traversal latency.** 1-hop and 2-hop expansion from a 10-entity
  seed set (the shape GRPH-3 will issue: hybrid retrieval hands recall a
  ranked hit set, and the graph expands around it), measured at **1M and
  10M edges in a single tenant's graph**. Threshold: **median ≤ 50ms**,
  with p95 reported against the 150ms slice above. The assert is on the
  median and the tails are reported — the HIER-1/MEM-1/CTX-1 discipline
  for IO-crossing perf criteria, because virtualised dev IO owns the
  upper percentiles and EVAL-6 owns percentile SLO enforcement on
  production-shaped hardware.
- **G2 — scale slope.** The same bounded-neighbourhood query at 10M edges
  costs **≤ 3× its 1M cost** (median). A local traversal is index-bound
  by nature; if ten times the data costs much more than three times the
  time, the traversal is not index-backed, and the trend is worse above
  10M — which is where a real bank's entity graph lives, not below it.
- **G3 — write cost.** A single edge create, committed inside the
  enclosing record transaction: **median ≤ 10ms**. ADR-0001's whole claim
  is that records, embeddings, graph edges, queue rows and audit rows
  commit together; GRPH-2 links every extracted record. The pipeline lag
  SLO (<60s) is generous, but an edge write costing 100ms turns a
  three-edge record into a third of a second of held locks on the
  MEM-3/MEM-4 commit path.
- **G4 — tenant multiplicity.** ADR-0004 mandates three named graphs per
  tenant. Measured: relations and catalog rows created per graph, and
  wall-clock to create a tenant's three. Ceiling: **1,000 tenants must
  add fewer than 25,000 relations and under 5 minutes of cumulative
  admission time.** Tenant creation is interactive (TEN-1), and catalog
  bloat is not a graph problem — it degrades planning for every query in
  the database.
- **G5 — SQL discipline.** Whether the traversals GRPH-3 needs can be
  expressed without composing SQL from runtime values, under sqlx
  compile-time checking (CLAUDE.md; ADR-0001 compliance note). Binary,
  with the workaround recorded if one exists.
- **G6 — transactionality and the tenant backstop.** Whether cypher
  writes roll back with the enclosing transaction, and whether TEN-2's
  forced row-level security can be applied to AGE's label tables. Binary,
  and a failure on the RLS half is recordable as an accepted risk with a
  named mitigation rather than an automatic no-go — per-tenant graphs are
  a structural boundary already — but it must be recorded, not assumed.

### The verdict rule

- **Go** — G1, G2, G3 pass at both scales; G5 and G6 pass or carry a
  recorded mitigation. ADR-0004 stands; GRPH-1 proceeds on AGE.
- **No-go on G1 or G2** — the fallback ladder, in order: (1) if the
  relational baseline clears G1/G2 on the same data, ADR-0004's rejected
  option 4 is revived — adjacency in plain Postgres, no graph engine, and
  the multi-graph schema becomes tables rather than AGE graphs; (2) only
  if *neither* clears it does the dedicated-engine assessment activate,
  because adding an engine reopens everything ADR-0001 closed and is the
  expensive answer, not the first one.
- **No-go on G3 alone** — graph-linking moves out of the record
  transaction into its own stage, which costs ADR-0001's
  commit-together property for edges and is recorded as such.
- **No-go on G4 alone** — revisit ADR-0004's partition boundaries (merge
  entity+episode, or one graph per tenant with label discrimination)
  before considering an engine change; this is a schema decision, not a
  substrate one.

## Options considered

1. **Pre-registered criteria against a derived recall budget (chosen)** —
   the thresholds exist before the numbers, and the budget is traceable to
   published figures and this repo's own measurements. Con: the recall SLO
   is invented here rather than inherited, so a later real-world number
   supersedes it.
2. **Benchmark first, judge after** — the normal way spikes are run, and
   the reason most spikes conclude that the incumbent is fine. Rejected:
   ADR-0004 is already Accepted, so an unpinned gate can only ratify it.
3. **Gate on total recall latency end-to-end** — more faithful to what a
   user feels, but recall does not exist yet (CTX-5 is unbuilt), so it
   would defer the gate behind the very features it exists to unblock.
4. **Skip the gate, build GRPH-1, measure later** — cheapest today. It is
   also exactly the retrofit trap ADR-0004 warns about in its own option 2:
   the schema is the expensive thing to move, so measuring after building
   it means the measurement can no longer change the answer.

## Consequences

- Positive: whichever way it lands, two Accepted ADRs stop resting on an
  assumption; the recall budget gets a written decomposition that CTX-5
  and EVAL-6 inherit; the relational baseline makes "AGE is slow" and
  "graphs are slow" distinguishable.
- Negative / accepted trade-offs: the criteria are calibrated on dev
  hardware under virtualised IO, so they bound *relative* behaviour well
  and absolute behaviour loosely — G1's median assert with reported tails
  is the concession, and EVAL-6 re-measures on production shapes. The
  300ms recall target is derived, not observed from users.
- Reversal trigger: EVAL-6 measuring recall on production-shaped IO
  supersedes the derived budget in this ADR; if the real decomposition
  differs materially, the graph slice is re-cut and G1 re-run against it.

## Compliance notes

The spike runs on a scratch database (the EVAL-1 pattern) with synthetic
entities only — no tenant data, no fixtures carrying content. It creates
no code path around the PDP because it creates no product code path at
all: nothing in this feature ships in the gateway. G6 exists precisely to
put the tenant-isolation question (seed §2.2, TEN-2) on the record before
GRPH-1 designs storage, and whatever it finds is carried into ADR-0004's
compliance note rather than left in a benchmark log.

## Verdict

_Filled in after the run — see below._
