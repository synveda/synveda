# ADR-0004: Multiple named AGE graphs per tenant with bitemporal edges

- **Status**: Accepted
- **Date**: 2026-07-18
- **Feature(s)**: FND-6, GRPH-1..4
- **Deciders**: sujitn

## Context

The 2026 research is unambiguous on two points (features doc §A1).
Temporal knowledge graphs won: Graphiti's bitemporal edges — validity
windows plus ingestion time — are credited with a 15-point LongMemEval gap
over flat vector storage, at ~300ms P95 with no LLM calls at retrieval
time. And multi-graph architectures lead the benchmarks: MAGMA tops LoCoMo
by maintaining specialised graphs (semantic, episodic, causal, entity)
rather than one homogeneous graph. Partitioning a single tangled graph
after the fact is expensive; naming graphs up front is cheap. Synveda's
graph layer must also honour existing invariants: records are bitemporal
(ADR-0006), recall answers as-of questions, and every traversal is
tenant-scoped and policy-filtered.

## Decision

Each tenant gets **multiple named AGE graphs**, not one: an **entity
graph** (people, systems, projects and their relations), an **episode
graph** (sessions, events, temporal sequence), and a **provenance graph**
(record → source session → extraction method → approving identity). Every
edge carries bitemporal validity — `valid_from`/`valid_to` (world time)
and transaction time — so new facts supersede old edges without erasing
history, mirroring the ADR-0006 record pattern. Graph features are
additive and degradable: retrieval and injection work with the graph off.

## Options considered

1. **Multiple named graphs per tenant (chosen)** — matches the
   MAGMA-informed state of the art; traversals stay within one semantic
   domain (recall's 1–2 hop expansion never wanders from entities into
   provenance); per-graph lifecycle (the episode graph can decay under
   MEM-6 while entities persist); clean per-tenant isolation boundary in
   AGE. Con: cross-graph questions need explicit record-level joins.
2. **One homogeneous graph, label-discriminated** — simpler to start, and
   the retrofit trap the research digest explicitly warns against:
   traversals cross semantic domains unless every query filters on labels
   (a leak-by-omission footgun, like the single-table bitemporal option
   rejected in ADR-0006), and per-domain decay or export becomes a
   label-scan instead of a graph drop.
3. **Dedicated graph database** (Neo4j, Memgraph) — stronger engines, but
   licences fail the core-path rule (GPL/BSL), and a second engine
   reintroduces the sync pipelines and multi-database tax ADR-0001
   eliminated; graph writes would no longer commit transactionally with
   records.
4. **No graph layer (vectors + FTS only)** — viable v1, but forfeits
   multi-hop recall (GRPH-3), entity dedup (GRPH-2), and the
   supersession-edge pattern (MEM-5) that the benchmark gap is attributed
   to; retrofitting per-tenant multi-graph later is the expensive path.

## Consequences

- Positive: schema matches where the research field is converging while
  staying inside Postgres; edge supersession preserves history for as-of
  recall (GRPH-1 AC); the provenance graph gives audit lineage a queryable
  shape; degradable design means AGE maturity risk never blocks the read
  path.
- Negative / accepted trade-offs: AGE Cypher performance at scale is
  unproven — accepted with an explicit gate (below); three graphs per
  tenant multiply named-graph bookkeeping (creation, migration, drop on
  tenant delete); cross-graph queries route through record IDs rather than
  a single traversal.
- Reversal trigger: the GRPH-4 spike benchmarks traversals at 1M/10M
  edges; failing its go/no-go criteria activates the Kuzu
  embedded-fallback assessment recorded there. If cross-graph joins
  dominate real recall workloads, revisit the partition boundaries (merge
  entity+episode) before adding engines.

## Compliance notes

Graph traversal is part of recall, so it sits behind the same PDP check as
every read (seed §2.2) — expansion results are filtered by tenant, scope,
and sensitivity before ranking, and the leak-test suite (EVAL-5) covers
graph paths explicitly. Named graphs are per tenant, giving TEN-5 tenant
deletion a clean unit (drop the tenant's graphs) and keeping cross-tenant
edges structurally impossible. The provenance graph underpins AUD-2's
lineage answers but never substitutes for the AUD-1 hash chain.
