---
title: "CTX-7: Dense-leg plan stability"
labels:
  - epic:CTX
  - phase:3
size: M
---

# CTX-7: Dense-leg plan stability

**Epic:** CTX — Context engine (read path) · **Phase:** 3 · **Size:** M

## Description

The dense leg's query is a prepared statement on a long-lived pool. PostgreSQL
plans it against real parameter values for five executions per connection and
may then substitute a **generic** plan built without them — and at the shapes
measured so far the generic plan does not use `record_embeddings_hnsw_1024` at
all. It scans the tenant's whole allowed slice through `records_tenant_scope_idx`
and sorts by distance exactly. Rule on which plan the read path should run, and
make it a decision rather than a default nobody chose.

## Why this exists

Filed 2026-08-10 by TEN-3 (ADR-0063), whose benchmark found it by disagreeing
with itself. The same arm — `iterative_scan = off`, `ef_search = 100`, the same
64,000-record corpus — measured recall 0.341 at p50 5.91ms on one run and 0.868
at 50.91ms on another. Neither number was wrong. The variable was not the arm.

Two things were moving, and both were invisible to a harness that recorded one
plan per run:

- **Statistics.** A freshly seeded corpus has none, and autoanalyze arrives
  part-way through the measuring loop. Without statistics the planner declines
  HNSW; with them it takes it. Proven by deleting `pg_statistic` rows on a fixed
  corpus and re-explaining: HNSW → exact → HNSW again after `ANALYZE`.
- **Plan caching.** Holding statistics constant, the same prepared statement
  with the same arguments takes `record_embeddings_hnsw_1024` on execution 1 and
  `records_tenant_scope_idx` on execution 6, and `plan_cache_mode =
  force_custom_plan` keeps HNSW at execution 6.

The first is the benchmark's problem and TEN-3 fixed it. **The second is the
product's**, because nothing about it is confined to a benchmark: `sqlx` prepares
its statements, the gateway's pool is long-lived, and `plan_cache_mode` is
`auto` everywhere.

## What it costs, measured

TEN-3's final sweep: pgvector 0.8.6 on PostgreSQL 17.10, 64,000 records over 8
tenants, dim 1024, broad allowed-scope slice, three runs per arm on an idle
machine. Rows in `evals/scores/ten3-dense-leg.json`.

| plan | tuning | recall@10 | p50 | p95 |
|---|---|---|---|---|
| generic (exact scan) | — | **1.000** | 51.08ms | 52.70ms |
| **`auto`, what ships** | ef_search 100 | 0.856 | 50.03ms | 51.39ms |
| custom (HNSW) | ef_search 1000 | 0.773 | 29.48ms | 30.98ms |
| custom (HNSW) | ef_search 400 | 0.606 | 16.29ms | 17.61ms |
| custom (HNSW) | ef_search 100 *(shipped)* | 0.355 | 6.14ms | 9.92ms |

**This is a trade, not a defect with an obvious fix, and the first draft of this
file got that wrong.** It described the generic plan as costing latency and
scaling — true — and implied `force_custom_plan` was the remedy. At the tuning
the product actually ships, forcing custom plans would trade **recall 1.000 for
recall 0.355** to gain 8× on latency. That is the worst point on the curve, and
nothing about the plan-cache setting alone would move it: `ef_search` is the
knob that buys recall back, and it costs the latency again — 0.773 at 29.48ms is
the best HNSW arm measured, still short of exact and only 1.7× faster than it.

So the exact plan **dominates on recall** and is not even especially slow here.
Which reframes what is actually broken.

### The defect is that nothing chooses

The problem is not that the read path runs the wrong plan. It is that it runs
**both, and which one is a function of how many times a pooled connection has
executed the statement.** Two identical queries against the same corpus return
different recall depending on the age of the connection they land on. The
shipped `auto` row is not a behaviour anyone designed — 0.856 is the arithmetic
of five custom executions per connection out of twelve, and it moves with the
pool size, the request rate and the query count.

That is the thing to fix, and it is worth fixing even if the ruling turns out to
be "keep the exact plan", because a benchmark, an SLO and a customer's
reproduction all mean something different when the plan is stable.

### Two caveats that keep the exact plan from being the obvious answer

- **Scaling.** An exact scan over a tenant's allowed slice is O(tenant), which
  is the cost an ANN index exists to avoid. At 8,000 records per tenant it is
  51ms; the shape of that number at 1M is not measured and cannot be
  extrapolated from one corpus size. The product's most loaded tenant is its
  worst case.
- **The selective regime is already exact under both plans** — one scope, one
  tier, 125 rows, ~1.33ms, recall 1.000 — so whatever is ruled here changes
  nothing for the regime the PDP's slice makes narrow, which is the common one.

## What this feature has to settle

1. **Which plan the dense leg should run**, stated as a decision and set
   deliberately — `plan_cache_mode`, transaction-locally in `dense_candidates`
   alongside the `hnsw.*` GUCs it already sets, so it costs no extra round trip
   and cannot leak into the pool. The setting is one line; the ruling is not.
   `force_custom_plan` is not a fix on its own — at the shipped `ef_search` it
   is the worst point on the curve — so ruling for HNSW means ruling on
   `ef_search` in the same breath, and paying re-planning CPU on a hot path.
   Ruling for `force_generic_plan` is a legitimate answer that keeps today's
   accuracy and makes it intentional, at the cost of an index the product
   maintains and would then barely use.
2. **Whether the answer is per-query or per-deployment.** A small tenant is
   better served by the exact scan; a large one cannot afford it. If the ruling
   is "it depends on corpus size", then it belongs in configuration and the
   threshold has to be measured rather than guessed.
3. **What CTX-1's latency AC actually measured.** "p99 <80ms at 1M
   records/tenant" is a published claim and `crates/synveda-retrieval/tests/latency.rs`
   is the test behind it. That test runs 200 queries through a pool, so it is
   subject to the same switch — and it uses dim **16**, where an exact scan is
   far cheaper than at 1024. Whether its number is an HNSW number is currently
   unknown. It vacuums and analyzes, so only the plan-cache half applies.
4. **Whether anything else on the read path has the same shape.** The dense leg
   is where it was found, not necessarily where it stops: any prepared statement
   whose plan depends on an array parameter's selectivity can flip the same way.
   `unnest($4::uuid[], $5::text[])` — the PDP's decision as SQL — is exactly such
   a parameter, and it appears in more than one query.

## Acceptance criteria

- The plan the dense leg runs is **stable and asserted**, by a test that fails
  if the read path stops using the plan the ruling chose — including after the
  fifth execution on one connection, which is the case that would otherwise pass
  every time a test opens a fresh pool.
  `EXPLAIN (GENERIC_PLAN)` is how to ask for the generic one; `plan_cache_mode`
  around an EXPLAIN does not work, because EXPLAIN builds a one-shot plan.
- Recall and p50/p95 recorded for **both** plans at 1024 dimensions, on the
  TEN-3 harness, at more than one corpus size — the flip is a cost-estimate
  decision and one shape does not establish where it turns over.
- CTX-1's latency AC re-read against whichever plan its test is measuring, and
  the finding recorded wherever that number is quoted.
- An ADR carrying the ruling, since which plan the read path runs is an
  architectural choice and not an implementation detail.

## Notes

TEN-3's harness (`crates/synveda-store/tests/ann_bench.rs`) already carries
`plan_cache_mode` as an arm dimension and reports the custom and generic plans
side by side, flagging when they differ. This feature inherits the instrument
rather than building one.
