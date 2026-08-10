---
title: "TEN-7: LIST partitioning per tenant"
labels:
  - epic:TEN
  - phase:4
size: L
---

# TEN-7: LIST partitioning per tenant

**Epic:** TEN — Multi-tenancy (functional requirement) · **Phase:** 4 · **Size:** L

## Description

The partitioning half TEN-3 measured and declined — as `LIST` rather than the
`HASH` its text asked for. A hash partition holds an arbitrary set of tenants,
so it can be neither dropped for one nor pinned for one, and every feature that
wants partitioning wants exactly one of those.

## Why this exists

Filed 2026-08-10 by TEN-3 (ADR-0063 decision 4), whose benchmark was allowed to
come back negative and did.

The gate was fixed before the numbers existed: partitioning ships only if, in
the **selective** regime and against the better of the tuned arms, it raises
recall@10 at equal-or-better p95 or cuts p95 by ≥25% at equal-or-better
recall@10. Measured over 64,000 records / 8 tenants / dim 1024, three runs per
arm: that regime is recall **1.000** at **1.65ms p95**, in all ten arms, on
`records_tenant_scope_idx`. It never touches the HNSW index, so shrinking that
index by `1/N` cannot reach it, and there is no recall left to raise.

**The measurement also showed the gate was written in the wrong regime**, which
is the part worth carrying forward. Where hash partitioning would do something
is the **broad** regime under a custom plan: the HNSW index holds every tenant's
vectors, ~7 of 8 candidates are discarded by the tenant predicate, and recall is
0.355 at `ef_search` 100. Splitting the index is a real remedy for that — the
"mitigates pgvector post-filtering" the original text named.

That is not enough to justify it, because of CTX-7: **the product does not use
the HNSW index on that path today.** What ships measures 0.856 at 50.03ms, a
blend that is mostly the generic plan's exact scan. Partitioning cannot be
justified against a plan the read path may stop using, or may never have been
using in production.

## What would reopen it

Either is sufficient, and they are independent.

1. **The boundary trigger.** TEN-5's disposal needs a tenant to be *droppable*,
   or OPS-3's residency needs one *pinned* to a plane. Neither is an ANN
   question and neither is served by hash at all. If either lands as a
   requirement, this feature is the layout it needs, and the ANN argument below
   is irrelevant to it.

2. **The ANN trigger, which is conditional on CTX-7 and has a number.** CTX-7
   must first rule that the dense leg runs the custom plan — that is, that the
   read path actually uses `record_embeddings_hnsw_1024`. Only then does
   post-filtering by tenant cost anything. After that ruling, this reopens if
   broad-regime recall@10 stays **below 0.90 at p95 ≤ 30ms** on a corpus of at
   least 8 tenants, using the best tuning TEN-3's harness can find. The
   measured ceiling today is **0.773 at 30.98ms p95** (`ef_search` 1000), so on
   current evidence the trigger *would* fire — but only behind CTX-7, and only
   if tuning has genuinely been exhausted first, because tuning costs a session
   setting and this costs the primary key.

Both `max_scan_tuples` and `strict_order` are already ruled out as cheaper
alternatives: the bound moves nothing at any `ef_search` (0.355→0.365,
0.606→0.586, 0.773→0.775, one of them the wrong way and all inside the spread),
and `strict_order` is not separable from `relaxed_order` at equal latency.

## What it costs

ADR-0063 force 4, unchanged by anything measured since:

- `records_pk primary key (id)` becomes `(id, tenant_id)`, after which **the
  schema no longer asserts that a record id is unique on its own**. Every call
  site treating a record id as globally addressable — the audit chain's
  payloads, the sidecar index's document ids, `record_supersessions` — keeps
  working because ids do not collide, not because the database refuses.
- `record_embeddings_pk` becomes composite too, and its FK must reference the
  new key.
- Migration 0001's structural rule drags `records_history`, `records_versions`
  and both archive trigger functions into the same change.
- It rewrites the two largest tables in the product, and PostgreSQL has no
  `ALTER TABLE … INTO PARTITIONED`. This is an **operator-run offline
  repartition with the outage stated in a runbook**, not a migration.
- Partition-per-tenant has a planning cost that grows with tenant count, which
  is the specific reason ADR-0063 refused `LIST` for its own purposes and the
  first thing this feature has to measure.

## Acceptance criteria

- Whichever trigger reopened it, met and stated — the boundary requirement
  satisfied, or the ANN numbers measured on TEN-3's harness with tuning shown
  to be exhausted first.
- **Filtered ANN query plan shows partition pruning** at `EXPLAIN (ANALYZE)`
  with partitions actually removed, not a plan shape that looks like it should
  prune. The RLS predicate is a stable function and the query also passes
  `tenant_id = $1`, so this is runtime pruning (ADR-0063 decision 8).
- TEN-3's harness re-run across the change, publishing rows for the partitioned
  layout beside the unpartitioned ones already in
  `evals/scores/ten3-dense-leg.json` — the comparison the original AC asked for,
  finally made against something that exists.
- Every partition carries its **own enabled and forced RLS**, and ADR-0009's
  completeness guard in `crates/synveda-store/tests/rls.rs` learns about
  `relkind = 'p'`. Partitions do not inherit policies, and relying on "nothing
  is granted directly on a partition" is the privileges-will-save-us argument
  that ADR was written against.
- Planning cost measured against tenant count, so the layout's own scaling limit
  is a number rather than a worry.
- A runbook for the repartition, with the outage stated.
