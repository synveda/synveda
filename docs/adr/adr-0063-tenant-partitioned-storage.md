# ADR-0063: the benchmark is the deliverable — pgvector 0.8 answers the post-filtering this feature was written to answer, hash-by-tenant cannot reach the predicate that actually hurts, and the partition key would cost `records` the meaning of its own primary key

- **Status**: Accepted
- **Date**: 2026-08-10
- **Feature(s)**: TEN-3
- **Deciders**: sujitn

## Context

TEN-3's text is "Declarative partitioning by tenant hash for
records/embeddings; partial HNSW indexes per partition (mitigates pgvector
post-filtering)", and its acceptance criterion is "filtered ANN query plan
shows partition pruning; benchmark vs unpartitioned recorded."

Five forces, and the first is about how to read that criterion.

**Force 1 — the AC has two halves, and the second one is the one that
decides.** A benchmark against an unpartitioned baseline, recorded, is only
worth running if it is allowed to come back negative. Written the other way
round — partition first, then measure — it is a number that can only confirm
a decision already taken, which is the instrument EVAL-3's first complete run
had and this repository has since named twice. So the honest order is
benchmark, then structure.

**Force 2 — the remedy this feature names has since shipped in the
dependency.** "Mitigates pgvector post-filtering" describes a real problem: an
HNSW scan walks the graph, returns its best `ef_search` candidates, and *then*
the filter runs, so a selective filter can leave far fewer than `limit` rows
and silently lose recall. pgvector 0.8 added **iterative index scans** for
exactly this — the scan continues past the first batch until enough rows
survive the filter, under bounds the operator sets. We already ship it:
`synveda/dev-postgres:17` carries `vector--0.8.6.sql`, and the enterprise
image installs the same PGDG package. The feature text predates it. Nothing in
this workspace has ever set `hnsw.iterative_scan`, and nothing has measured
what it would do to the query below.

**Force 3 — the dense leg filters on two things, and hash-by-tenant reaches
only the cheaper one.** `synveda-store/src/search.rs`:

```sql
from record_embeddings e
join records r on r.id = e.record_id
where e.tenant_id = $1 and e.dim = 1024 and e.model = $3
  and r.tenant_id = $1
  and (r.scope_id, r.sensitivity) in (select * from unnest($4::uuid[], $5::text[]))
order by e.embedding::vector(1024) <=> $2::real[]::vector(1024)
limit $6
```

The tenant predicate is one filter; the `(scope_id, sensitivity)` slice is the
other, and it is the PDP's decision materialised as SQL. Hash partitioning by
tenant shrinks the index a query touches by roughly `1/N` and leaves the scope
slice exactly as it was — inside a tenant, nothing has changed. Migration 0016
already knew which of the two hurts, and said so while adding
`records_tenant_scope_idx`: "when the allowed-scope slice is small, the
planner should prefer an exact scan over the slice to an iterative HNSW
crawl." So the feature's stated motivation, read against the query it is
about, is a partial remedy aimed at the easier half.

**Force 4 — this is not a change a migration can make.** PostgreSQL has no
`ALTER TABLE … INTO PARTITIONED`, and a hash partition cannot `ATTACH` an
existing table that was not built to satisfy its hash constraint. Every unique
constraint on a partitioned table must contain the partition key, so:

- `records_pk primary key (id)` becomes `(id, tenant_id)` — after which **the
  schema no longer asserts that a record id is unique on its own**;
- `record_embeddings_pk primary key (record_id)` becomes
  `(record_id, tenant_id)`, and its FK to `records (id)` must reference the new
  composite key;
- migration 0001's structural rule — "a migration that alters `records` must
  make the identical change to `records_history`, to `records_versions`, and to
  the explicit column lists in the archive trigger functions, in the same
  migration" — drags the whole bitemporal triple and both trigger functions
  into the same change.

And it rewrites the two largest tables in the product. That is the OPS-2
finding one size up: an applied migration series is append-only in practice,
and a restructure is not a migration.

**Force 5 — the other features that want partitioning want a different
one.** TEN-5's disposal wants a tenant to be *droppable*; OPS-3's residency
wants a tenant *pinned* to a plane; ADR-0009's own reversal trigger imagines
"partition-per-tenant layout [making] partition-level grants the primary
isolation mechanism". Every one of those wants a boundary **per tenant**,
which is `LIST`. Hash gives them nothing: a hash partition holds an arbitrary
set of tenants, so it can neither be dropped for one nor pinned for one. "We
should partition the storage" is two proposals with different beneficiaries,
and this feature's AC names only the ANN one.

## Decision

1. **The benchmark is built first, and it is allowed to say no.** A harness
   that measures the dense leg over a seeded corpus at stated sizes and tenant
   counts, in both regimes that matter — a broad allowed-scope slice and a
   selective one — reporting recall@10 against exact search and p50/p95
   latency. It records like EVAL-3's scores do: a file that accumulates rows
   with the corpus digest, the pgvector version and the commit in each,
   because "benchmark vs unpartitioned **recorded**" is the AC's own word.

2. **Three arms, not two.** The AC names two (partitioned, unpartitioned) and
   force 2 supplies the third:

   - **A**: today's schema, as it is.
   - **B**: today's schema with `hnsw.iterative_scan` enabled and its bounds
     tuned — the remedy the feature was written to build, which arrived in the
     dependency instead.
   - **C**: hash-partitioned by `tenant_id`.

   B is measured before C is built. It costs a session setting and an
   afternoon; C costs the schema in force 4.

3. **The gate is stated here, before the numbers exist.** Partitioning ships
   only if, in the **selective** regime and against the better of A and B, it
   either raises recall@10 at equal-or-better p95, or cuts p95 by ≥25% at
   equal-or-better recall@10. A margin below that is not worth a composite
   primary key on the product's central table.

4. **If the gate is not met, the AC is amended rather than satisfied.** The
   first half — "filtered ANN query plan shows partition pruning" — cannot be
   shown by a deployment that does not partition, so a benchmark that says no
   makes the criterion unmeetable as written. That is a thing this repository
   has done before and has a way of doing: EVAL-3 amended its corpus when a
   licence made its own AC unquotable, and moved the goal rather than the
   honesty. In that case TEN-3 delivers the harness, the recorded rows and the
   plan evidence for whichever arm won, SYNVEDA_FEATURES.md records the
   measurement as the reason its text changed, and the partitioning half is
   filed as its own feature with the number that would reopen it.

5. **If it ships: hash on `tenant_id`, and the primary key's meaning is a
   stated cost.** `records.id` stops being unique by constraint and becomes
   unique by construction — UUIDv7 minted by the writer — which is true today
   and, after this, no longer enforced. Every call site that treats a record id
   as globally addressable (the audit chain's payloads, the sidecar index's
   document ids, `record_supersessions`) keeps working because ids do not
   collide, not because the database refuses. That is a real reduction in what
   the schema guarantees and it is accepted only if decision 3's gate is met.

6. **Restructuring is an operator-run repartition, not a migration.** Whatever
   ships, existing deployments reach it through an explicit, documented,
   offline step that rewrites the tables — with the outage stated in the
   runbook — and new deployments get the layout at install. OPS-6's
   expand/contract lint is where this eventually belongs; until it exists, the
   honesty is the runbook's.

7. **ADR-0009's completeness guard is extended in the same change, because it
   will fire — as designed.** `crates/synveda-store/tests/rls.rs` discovers
   every `relkind = 'r'` table in `public` carrying a `tenant_id` column and
   asserts the discovered list **equals** its covered list. Partitioning breaks
   that twice: the parent becomes `relkind = 'p'` and disappears from
   discovery, and N partitions appear that were never covered. Partitions do
   **not** inherit the parent's policies, so each partition gets enabled and
   forced RLS in the same migration and the guard learns about `'p'`. Relying
   on "nothing is granted directly on a partition" would be exactly the
   privileges-will-save-us argument ADR-0009 was written against.

8. **The pruning claim is demonstrated at execution, not assumed.** The RLS
   predicate is `synveda_current_tenant()`, a stable function, and the query
   also passes `tenant_id = $1`; so pruning here is runtime pruning, and the
   demo shows `EXPLAIN (ANALYZE)` with partitions actually removed rather than
   a plan shape that looks like it should prune.

## Options considered

1. **Partition now, because the feature text says so.** The shortest path to a
   green checkbox. Refused: forces 2 and 3 mean the text was written against a
   pgvector that no longer exists and a filter it only half addresses, and
   force 4 means the price is the product's central primary key. A feature
   specification is a hypothesis, and this one has a cheaper rival that landed
   in the meantime.

2. **Iterative scans only, and close the feature.** Possibly where this ends
   up, and it is decision 4's branch. Not chosen *in advance*, because "the new
   setting is enough" is exactly as unmeasured as "partitioning is necessary",
   and the AC asks for a comparison rather than a preference.

3. **`LIST` partition per tenant.** What TEN-5, OPS-3 and ADR-0009's reversal
   trigger all actually want, and the only shape that makes a tenant droppable
   or pinnable. Refused *here* because this feature's AC says hash and because
   partition-per-tenant has a planning cost that grows with tenants — but
   recorded as the more likely long-term layout, which is itself an argument
   for not spending the restructure twice.

4. **Wait for EVAL-6's SLOs.** Tempting: EVAL-6 owns percentile SLOs, and
   without one, "faster" has no threshold. Refused because decision 3 sets a
   *relative* gate, which needs no absolute SLO, and because a feature that
   waits for another feature to define its success is a feature that will be
   measured by whoever gets there first.

5. **Push the problem to OPS-4 and let a vector database own recall.** The
   `VectorIndex` trait does not exist yet, and OPS-4's own benchmark gate is
   what decides that. Refused as scope, and noted as a reason to keep TEN-3's
   harness reusable — the two features ask the same question of different
   engines.

6. **Do nothing.** Leaves the AC's second half undone and, more to the point,
   leaves an unmeasured claim ("post-filtering hurts us") in the backlog where
   somebody will eventually build against it.

## Consequences

- Positive: the product gets its first **retrieval performance harness**, which
  is a thing it has never had — CTX-1 recorded quality numbers (recall@6 0.500
  sparse, 0.792 hybrid) and no latency at all, and EVAL-6 inherits a harness
  rather than a blank page.
- Positive: whichever arm wins, the answer is a recorded row rather than an
  argument, and the losing arms are on the same page as the winner.
- Negative / accepted: if the gate is met, `records` carries a composite
  primary key and the schema stops guaranteeing that a record id means one
  record. Decision 5 states it rather than discovering it later.
- Negative / accepted: the benchmark needs a corpus large enough for HNSW to
  behave like HNSW, which is minutes of seeding and is not a per-PR job. It
  belongs beside `eval-retrieval` on the nightly.
- Negative / accepted: this ADR spends a feature's budget on measurement and
  may deliver no partitioning at all. That is the intended outcome of an AC
  whose second clause is a comparison.
- Reversal triggers:
  - **The gate in decision 3**, in either direction, is the whole trigger for
    partitioning.
  - **`LIST` replaces `HASH`** the moment TEN-5's disposal or OPS-3's residency
    needs a per-tenant boundary — at which point the restructure is paid twice
    unless TEN-3 declined to pay it once.
  - **The harness is re-run on a pgvector major bump**, because the dependency
    moving is what made this ADR necessary in the first place.

## Compliance notes

- **Seed §2.2 / the PDP:** the `(scope_id, sensitivity)` slice in the dense leg
  is the PDP's decision in SQL. Everything measured here changes *how many rows
  the scan visits*, never *which rows are permitted* — iterative scans widen
  the search, not the filter, and a partition holds a subset of the same rows.
  Any arm that improved a number by relaxing that predicate would be a policy
  bypass wearing a benchmark, and the harness records the slice it used with
  every row so that stays checkable.
- **TEN-2 / ADR-0009:** decision 7. The completeness guard firing is the
  schema policing its own growth exactly as decision 6 of that ADR intended;
  partitions get their own enabled and forced RLS rather than inheriting an
  argument.
- **ADR-0009's reversal trigger** ("if TEN-3's partition-per-tenant layout
  makes partition-level grants the primary isolation mechanism … revisit policy
  shape") is **not** fired by this ADR: hash partitions hold many tenants, so
  they cannot carry per-tenant grants. Option 3 is where that trigger would
  come due.
- **ADR-0023 / ADR-0024 (embed-or-fail, the expression index):** the partial
  HNSW indexes are expression indexes on `(embedding::vector(n))` with a
  `where dim = n` predicate; both survive being created on a partitioned parent
  and propagate per partition, which is the "partial HNSW indexes per
  partition" the feature text asks for. The deferred `records_require_embedding`
  constraint trigger is per-row and unaffected.
- **Audit (DoD item 4):** no new action type. A storage layout is not an act.
