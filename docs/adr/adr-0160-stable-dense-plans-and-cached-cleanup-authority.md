# ADR-0160: Plan the dense Knowledge statement per execution and cache cleanup-authority plans

- **Status**: Accepted. Source, exact-role and same-machine benchmark
  evidence on PostgreSQL 17.11 with pgvector 0.8.7; the per-major regression
  suite is part of PostgreSQL 14–18 compatibility acceptance
- **Date**: 2026-10-28
- **Feature(s)**: Dense-leg plan stability; load and latency suite
- **Deciders**: Owner-authorized implementation

## Context

The dense Knowledge leg is one prepared statement per dimension
(`crates/synveda-store/queries/knowledge_semantic_{16,1024}.sql`). Its tenant,
scope, lifecycle, valid-time and transaction-time filters are parameters, and
it runs on long-lived pooled connections. PostgreSQL plans the first five
executions of a prepared statement with their arguments and may then adopt one
generic plan for the life of the connection. Record-era evidence (ADR-0063)
saw that switch move the dense leg between indexes; nothing had measured the
current Knowledge statement.

ADR-0024 decided that the dense query sets `hnsw.iterative_scan` and
`hnsw.ef_search` transaction-locally. That was the Record-era search module.
The current Knowledge statement sets neither, so it runs at pgvector's
defaults: `ef_search` 40 and no iterative scan.

The Knowledge tables carry restrictive source-eligibility policies. Every row
read through `knowledge_revision_embeddings` and `knowledge_revisions` calls
`synveda_source_available`, and a row whose source is unavailable falls
through to `synveda_source_cleanup_authorized`,
`synveda_artifact_cleanup_authorized` and `synveda_item_cleanup_authorized`.
Those three were `LANGUAGE sql` with a `SET search_path` clause, which
PostgreSQL before 18 neither inlines nor caches, so each such row re-planned a
join of up to eight tables even though each body opens with a database-owner
test that is false for every runtime role. Migration 0012 fixed the same
pathology in the two sibling routines.

The dense-plan benchmark (`crates/synveda-gateway/tests/dense_plans.rs`,
`demos/ctx-7-dense-plans.sh`) seeds two tenants into the one shared partial
HNSW index with clustered 1024-dimension vectors, revised items, archived and
stale heads, and session-sourced items half of whose sources are withdrawn
through the public API. It measures exact ground truth, setting arms on fresh
connections, and six executions of the unmodified product statement on one
connection, reading the plan class from `pg_prepared_statements`.

Measured on one Apple M5 Max host, PostgreSQL 17.11 and pgvector 0.8.7 at
fixture defaults, with 10,000- and 50,000-item tenants ([evidence
record](../../demos/evidence/ctx7-dense-plan-stability.json)):

- **The switch is real on the current statement.** On the base commit the
  unmodified product statement became generic at execution 6 on one pooled
  connection for 50,000-item broad search, and so did an identical SQL-level
  probe (custom 7.6–8.2 s, generic 9.3 s).
- **The generic plan is never the better one.** It scans every current and
  historical head of the tenant and filters afterwards. Across all four
  shapes it was 1.2 to 44 times slower before the routine fix and 1.6 to 56
  times slower after it (10,000-item single project: 11 ms custom, 622 ms
  generic).
- **No plan uses HNSW.** Every custom and generic plan in every arm was an
  exact, item-driven scan with recall@10 and recall@limit of 1.0, including
  arms that set `hnsw.iterative_scan = strict_order` and `ef_search` 100.
  With sorting disabled on a 4,000-item fixture the planner did choose the
  index and returned 17 of 1,000 requested rows.
- **Withdrawn sources dominated cost.** Each unavailable row cost about 35 ms
  in the three cleanup-authority routines; counting 6,041 revisions through
  RLS took 860 ms with 20 such rows. With cached plans the same count takes
  about 120 ms, leaving about 20 µs per row per restrictive policy.

| Tenant | Shape (limit) | Base commit, custom p50 | Base commit, generic p50 | Decision, p50 / p99 |
| --- | --- | --- | --- | --- |
| 10,000 | Planner scope chain (96) | 229 ms | 1,840 ms | 55 / 97 ms |
| 10,000 | One project (96) | 100 ms | 1,825 ms | 13 / 14 ms |
| 10,000 | Knowledge search (1,000) | 1,529 ms | 1,862 ms | 403 / 411 ms |
| 10,000 | Planner chain as known (96) | 61 ms | 1,791 ms | 59 / 61 ms |
| 50,000 | Planner scope chain (96) | 1,102 ms | 8,801 ms | 268 / 273 ms |
| 50,000 | One project (96) | 202 ms | 8,847 ms | 66 / 68 ms |
| 50,000 | Knowledge search (1,000) | 7,407 ms | 9,130 ms | 2,095 / 2,110 ms |
| 50,000 | Planner chain as known (96) | 331 ms | 8,735 ms | 293 / 297 ms |

The as-known shape holds no withdrawn-source rows and is the control for the
routine fix.

## Decision

1. **The dense statement always plans with its arguments.**
   `knowledge_search::semantic_candidates` reads the transaction's current
   `plan_cache_mode`, sets it to `force_custom_plan` transaction-locally,
   executes the unchanged static statement and restores the previous value in
   the same transaction. Nothing persists past the statement, the transaction
   or the pooled connection. No HNSW setting is applied: within the measured
   envelope the planner never chooses the HNSW index, and the dense leg is an
   exact, item-driven scan with exact ordering.
2. **The three cleanup-authority routines keep their plans.** Additive
   migration 0018, byte-identical in both lineages, re-creates them as
   `plpgsql` with `SET plan_cache_mode TO 'force_generic_plan'`, keeping each
   body byte for byte as 0012 did for their siblings. Grants, policies and
   triggers are unchanged; only the routine catalogue fingerprint moves.
3. **HNSW reaching the dense plan is drift, not an optimisation.** A default
   database test fails if any execution of the product statement is generic,
   if its plan walks the HNSW index, or if the statement text changes, and
   asks for the benchmark to be re-run. An HNSW scan at pgvector's defaults
   returns at most `ef_search` candidates before the tenant, scope, lifecycle
   and source filters run; forced on the 4,000-item fixture it returned 17
   of 1,000 requested rows.

## Options considered

1. **Leave `plan_cache_mode` at `auto`** — rejected: the measured generic
   plan scans every current and historical head and was slower in every
   shape, and the switch depends on how often a pooled connection ran the
   statement.
2. **`force_generic_plan`** — rejected for the same measured plan.
3. **Apply `force_custom_plan` for the whole tenant transaction** — rejected:
   it would change planning for every later statement on the request path.
4. **Iterative HNSW scans with a bounded `ef_search`** — not adopted: the
   planner does not choose HNSW within the envelope, so the setting would be
   inert; forcing HNSW would trade exact recall for a post-filtered
   approximate scan of a multi-tenant index, which is the vector index scale
   decision's question, not this one.
5. **Change or drop the restrictive policies** — rejected: they are the
   source-withdrawal boundary. The routine change keeps every predicate.
6. **Raw `SET LOCAL` in a multi-statement string** — rejected: product SQL
   stays static and SQLx-checked.

## Consequences

- Positive: the dense leg runs the same exact plan on every execution of a
  pooled connection, with deterministic ordering, and avoids the generic
  plan's full-history scan; every Knowledge read that meets a withdrawn
  source stops paying a re-plan per row. At 10,000 items the planner's dense
  leg falls from 229 ms to 55 ms p50.
- Negative / accepted trade-offs: each dense execution re-plans (about 1 ms
  planning on the measured host) and costs two extra round trips for the
  setting. The dense leg remains exact, so its cost grows with the rows inside
  the admissible scope set; the per-row restrictive-policy cost (about 20 µs
  per policy on the measured host) is the limiting term.
- Reversal trigger: the context planner's admissible scope set exceeding
  about 2,500 Knowledge rows per tenant (the dense leg then passes 100 ms on
  the measured host), any shape whose exact scan exceeds its budget at the
  supported corpus, a PostgreSQL or pgvector change that makes the planner
  choose HNSW (the drift test fails), or more than about 0.25% of a tenant's
  Knowledge behind withdrawn sources. Any of these reopens the vector index
  scale decision with this benchmark as its baseline.

## Compliance notes

Cedar decisions, forced RLS, the restrictive source-eligibility predicates,
VedaFlow and audit are unchanged. The setting is transaction-local and
restored, and a database test proves it does not reach the next statement,
transaction or tenant on the same pooled connection. Migration 0018 changes
no grant, policy or trigger; the authority preflight pins the new routine
fingerprint on every admitted PostgreSQL major.
