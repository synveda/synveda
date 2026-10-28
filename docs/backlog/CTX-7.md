# CTX-7: Dense-leg plan stability

## Problem and evidence

The epoch-3 dense Knowledge leg uses prepared `sqlx::query_file!` statements in `crates/synveda-store/queries/knowledge_semantic_16.sql` and `knowledge_semantic_1024.sql` over long-lived pooled connections. Their tenant, time, hierarchy, lifecycle, and allowed-scope filters are parameter-sensitive, while PostgreSQL may switch a prepared statement from custom to generic planning after repeated execution. Historical Record-era measurements cannot establish the current plan, recall, or latency, and no current test compares execution one with execution six on the same connection. Dense behaviour is therefore not yet a stable production claim.

### Current state (2026-10-28)

[ADR-0160](../adr/adr-0160-stable-dense-plans-and-cached-cleanup-authority.md)
records the implemented policy and its measured trade:

- `knowledge_search::semantic_candidates` sets `plan_cache_mode` to
  `force_custom_plan` transaction-locally around the unchanged static
  statement and restores the previous value; no HNSW setting is applied.
- Additive migration `0018` (both lineages, routine fingerprint
  `31179e8d…`, SQLx metadata regenerated) re-creates the three
  cleanup-authority routines as plpgsql with cached generic plans and
  byte-identical bodies.
- `crates/synveda-gateway/tests/dense_plans.rs` holds four default database
  tests and the operator benchmark (`demos/ctx-7-dense-plans.sh`). The tests
  prove six custom executions with exact ordering even under a session
  default of `force_generic_plan` (they fail with the policy removed),
  restoration of the caller's setting, no carry-over to the next
  transaction or tenant on the same connection, tenant, scope, source,
  lifecycle and as-known semantics against seeding facts across reused
  connections and an empty allowlist, and a drift check that fails on a
  generic or HNSW plan or a changed statement digest. The suite is in the
  PostgreSQL 14–18 compatibility task list.
- The [evidence record](../../demos/evidence/ctx7-dense-plan-stability.json)
  compares the base commit and the change at 10,000 and 50,000 items on one
  M5 Max host, PostgreSQL 17.11 and pgvector 0.8.7: the base commit's
  product statement went generic at execution 6 for 50,000-item broad
  search; the generic plan was 1.2–44 times slower before and 1.6–56 times
  after the routine fix; no plan used HNSW and recall was exact; the
  planner's dense leg fell from 229 to 55 ms p50 at 10,000 items.
- Full ContextRuns over the same corpora, built optimised, are dominated
  by the rest of the planner rather than the dense leg: p50 0.7 s at 1,000
  items, 1.3 s at 10,000 and 2.7 s at 50,000, mostly candidate retrieval
  and selection, with Cedar under 10 ms.

Remaining before this feature closes: owner approval of the corpus,
selectivity and latency bounds proposed with the capacity envelope,
hosted PostgreSQL 14–18 runs of the extended compatibility task, the same
benchmark on production-shaped Linux hardware with BGE-M3 vectors, and a
canary on a non-sensitive production-shaped corpus. Reducing the planner's
per-candidate cost is separate planner work that the envelope depends on.

## Scope

- Build an epoch-3 Knowledge benchmark at 1024 dimensions across multiple tenant corpus sizes and broad/selective allowed-scope shapes, with statistics refreshed before measurement.
- Capture `EXPLAIN (ANALYZE, BUFFERS)` evidence for custom and generic plans and compare execution one with execution six on the same prepared connection.
- Measure recall@k, p50/p95/p99 latency, planning time, rows visited, buffers, and index use for exact and HNSW alternatives at reviewed tuning values.
- Choose a stable plan policy deliberately, including any transaction-local `plan_cache_mode` or HNSW setting, and record the accuracy/latency/scaling trade in an accepted ADR.
- Audit other critical prepared read queries with selectivity-sensitive array or hierarchy filters and add assertions where the same switch is material.

## Non-goals

- Reusing Record-era table names, benchmark numbers, or a deleted ANN harness as current evidence.
- Forcing HNSW merely because an index exists, or forcing exact scan without measuring the supported largest tenant.
- Weakening PDP candidate authorization, forced RLS, deterministic ordering, or current Knowledge time semantics.
- Treating a single corpus size, 16-dimensional test vector, or fresh connection as a production plan guarantee.

## Architecture seam

Keep SQL static in `synveda-store` and the gateway authorization boundary unchanged. Any planner setting is applied transaction-locally on the same checked-out connection and reset by transaction end so it cannot leak through the pool. The benchmark uses current Knowledge revisions, embeddings, scopes, and ordinary tenant transactions; plan evidence remains test/report data, not runtime API output.

## Acceptance criteria

- Executions one and six of the same dense query use the chosen plan class and return the same deterministic result ordering for identical inputs.
- Recall and latency meet owner-approved bounds at every supported 1024-dimensional corpus/selectivity shape, with custom and generic evidence retained for comparison.
- A plan assertion fails if statistics, SQL, index definitions, PostgreSQL/pgvector changes, or pool reuse reintroduce an unreviewed switch.
- The chosen tuning is bounded, transaction-local, observable without high-cardinality labels, and does not persist on a reused connection.
- Existing context and Knowledge latency claims are revalidated against the plan they actually execute.

## Required tests

- Database benchmark that runs `ANALYZE`, executes one through six on one connection, and records custom/generic `EXPLAIN` plans at 1024 dimensions.
- Broad and selective allowed-scope cases over at least two representative corpus sizes, including exact ground truth for recall@k.
- Pool-reuse and transaction-cleanup test proving planner settings do not leak between requests or tenants.
- Stable ordering, empty allowed-scope, temporal/lifecycle filter, PDP post-filter, and cross-tenant RLS regression tests.
- Review test or script that detects material plan-shape drift after PostgreSQL, pgvector, schema, or query changes.

## Rollout and rollback

Observe both plan classes in the benchmark first, then ship the accepted policy behind a deployment setting only if the decision requires configurability. Canary against a production-shaped non-sensitive corpus before making it the default. Rollback restores the prior explicit policy, not PostgreSQL `auto`; retain both reports so the accuracy/latency trade remains visible.

## Dependencies

The owner must declare supported corpus sizes, scope selectivities, embedding model/dimension, PostgreSQL/pgvector versions, hardware, recall floor, and latency budgets. The accepted ADR is required before changing runtime planner settings. EVAL-6 depends on this stable plan for dense-leg load claims.
