# CTX-7: Dense-leg plan stability

## Problem and evidence

The epoch-3 dense Knowledge leg uses prepared `sqlx::query_file!` statements in `crates/synveda-store/queries/knowledge_semantic_16.sql` and `knowledge_semantic_1024.sql` over long-lived pooled connections. Their tenant, time, hierarchy, lifecycle, and allowed-scope filters are parameter-sensitive, while PostgreSQL may switch a prepared statement from custom to generic planning after repeated execution. Historical Record-era measurements cannot establish the current plan, recall, or latency, and no current test compares execution one with execution six on the same connection. Dense behaviour is therefore not yet a stable production claim.

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
