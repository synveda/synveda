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

At 64,000 records over 8 tenants, dim 1024, broad allowed-scope slice, the same
`DenseTuning` the product ships, varying only `plan_cache_mode`:

| plan_cache_mode | recall@10 | p50 | p95 |
|---|---|---|---|
| `auto` (what ships) | 0.871 | 51.44ms | 53.20ms |
| `force_custom_plan` | 0.526 | 6.69ms | 25.18ms |

Read that carefully, because the sign is not the one a defect usually has.
**The generic plan is exact, so it returns better answers.** What it costs is
latency — roughly 8× here — and, more seriously, *scaling*: an exact scan over a
tenant's allowed slice is O(tenant), which is the cost an ANN index exists to
avoid. The gap therefore widens with every record a customer adds, and the
product's most loaded tenant is its worst case.

The selective regime is unaffected and stays exact under both plans: one scope,
one tier, 125 rows, ~1.3ms, recall 1.000.

## What this feature has to settle

1. **Which plan the dense leg should run**, stated as a decision. The cheap
   candidate is `plan_cache_mode = force_custom_plan` set transaction-locally in
   `dense_candidates`, alongside the `hnsw.*` GUCs it already sets — no extra
   round trip, no leak into the pool. It is cheap enough to look obvious and
   should not be taken on those grounds: forcing custom plans means re-planning
   every dense query, which is real CPU on a hot path, and the numbers above say
   the generic plan is *more accurate*. A product may legitimately prefer exact
   answers at 51ms.
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

- The plan the dense leg runs is **asserted rather than assumed**, by a test
  that fails if the read path stops using the index it is supposed to use.
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
