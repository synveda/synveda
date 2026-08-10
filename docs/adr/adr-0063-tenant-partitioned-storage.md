# ADR-0063: the benchmark is the deliverable — pgvector 0.8 answers the post-filtering this feature was written to answer, hash-by-tenant cannot reach the predicate that actually hurts, and the partition key would cost `records` the meaning of its own primary key

- **Status**: Accepted, **amended 2026-08-10 within the hour** (force 2 was
  wrong on a fact; the conclusion it supported gets stronger, not weaker)
- **Date**: 2026-08-10
- **Feature(s)**: TEN-3
- **Deciders**: sujitn

## Amendment (2026-08-10): the remedy did not merely ship — we already use it

Force 2 said that pgvector 0.8's iterative index scans arrived after this
feature was written and that "nothing in this workspace has ever set
`hnsw.iterative_scan`". **The second half is false.** CTX-1 adopted it when
the dense leg was built: ADR-0024 decision 5 says the query "sets
`hnsw.iterative_scan = relaxed_order` transaction-locally so
scope/sensitivity post-filters keep yielding candidates instead of starving
the limit", and `synveda-store/src/search.rs` does exactly that on every
dense query, together with `hnsw.ef_search = 100`, inside the caller's tenant
transaction.

I found it by reading fifteen lines above the query I had already quoted, on
the way to writing the harness this ADR asks for. The claim was specific and
wrong and would have shaped a benchmark, so it is corrected here rather than
quietly fixed.

**What changes.** The feature text's premise — "partial HNSW indexes per
partition (mitigates pgvector post-filtering)" — is not merely dated. The
post-filtering it names was answered *inside this product*, by the feature
that built the dense leg, before TEN-3's turn came. Partitioning is therefore
a second remedy for a problem that already has one, and the burden on
decision 3's gate is higher, not lower.

**What survives unchanged.** Force 3 (hash-by-tenant reaches the tenant
predicate and not the scope slice) — and it matters more now, because the
scope slice is precisely what iterative scanning already exists to survive.
Force 4 (the structural cost, and the composite primary key). Force 5 (the
features that want partitioning want `LIST`). Force 1, and the gate in
decision 3.

**What the arms become.** Decision 2's arm B was "turn iterative scanning
on", which is a no-op against a product that already turns it on. It is
replaced by **tuning what we already set**: `ef_search` is the constant `100`
and `iterative_scan` is `relaxed_order` with no `max_scan_tuples` bound, and
none of those three has ever been measured against a corpus. That is the
cheap arm now — a session setting and a sweep, against a schema change that
costs `records` its primary key.

## Correction (2026-08-10): three of the four findings below measured the planner, not the tuning

**The table in the next section is withdrawn except for finding 1.** It was
not measuring what its column headings say.

The benchmark disagreed with itself first. Re-running the `off` / `ef_search
100` arm on the same 64,000-record corpus produced recall **0.868** at p50
**50.91ms**, where the table records **0.341** at **5.91ms** for that same arm.
Both runs were honest. The variable was not the arm.

Two things move the dense leg's plan, and a harness that recorded one plan per
run could see neither.

**Statistics.** A freshly seeded corpus has none, and autoanalyze arrives
part-way through the measuring loop — the probe corpus recorded
`last_autoanalyze` timestamps *during* its own run. Holding the plan cache
fixed and deleting `pg_statistic` on a settled corpus reproduces it exactly:
with statistics the planner takes `record_embeddings_hnsw_1024`, without them
it takes `records_tenant_scope_idx` and sorts the whole slice, and `ANALYZE`
puts it back. So every arm's early queries planned against a table PostgreSQL
believed was empty, and its later ones did not, with the crossover set by the
autovacuum naptime.

**Plan caching.** Holding statistics constant, the same prepared statement with
the same arguments takes `record_embeddings_hnsw_1024` on execution 1 and
`records_tenant_scope_idx` on execution 6 — PostgreSQL's custom-to-generic plan
switch. `plan_cache_mode = force_custom_plan` keeps HNSW at execution 6. The
generic plan is *exact*: it agrees with the enable_indexscan-off ground truth on
all ten of ten.

So the arms were blends of two different queries over two different indexes,
mixed in a ratio set by the autovacuum naptime and by how many times each
pooled connection had executed the statement. Neither is the arm.

**What falls.** Finding 2 — "iterative scanning is worth 2.6x recall and 8.6x
latency (0.341 → 0.878, 5.9ms → 50.9ms)" — is the clearest casualty, because
that contrast is now reproducible with *iterative scanning held constant*.
Varying only `plan_cache_mode` at the shipped `DenseTuning`: `auto` gives recall
0.871 at p50 51.44ms, `force_custom_plan` gives 0.526 at 6.69ms. Same knob
settings, same corpus; the 8.6x was the plan. Findings 3 and 4 — `ef_search`
400's six free points, and 1000 being non-monotonically worse — are
broad-regime numbers from the same blends and are unsupported rather than
disproved. Finding 4's anomaly in particular needs no `max_scan_tuples`
explanation once the blend ratio is free to vary between arms.

**What survives, and is now stronger.** Finding 1. The selective regime is
exact under **both** plan shapes — `records_tenant_scope_idx`, 125 rows, ~1.3ms,
recall 1.000 — so no plan-cache or statistics subtlety can reach it. That is
the finding decision 3's gate is written against, and it is the one that says
hash-by-tenant cannot reach the regime that decides.

**What it cost and what changed.** The harness now `vacuum (analyze)`s its
corpus before measuring, carries `plan_cache_mode` as an arm dimension, and
records the custom and generic plans side by side, flagging when they name
different indexes. Two instruments that looked like they would show the generic
plan and do not are documented in `explain_generic`, because both were written
here first: `plan_cache_mode` around an EXPLAIN governs cached plans and EXPLAIN
builds a one-shot plan, and executing an EXPLAIN six times does not reach the
switch because EXPLAIN re-plans each time. `EXPLAIN (GENERIC_PLAN)` is the one
that works.

**And the product half is not this feature's.** That the dense leg abandons its
ANN index after five executions on a pooled connection is a read-path question
with a blast radius well beyond tenancy — it bears on CTX-1's published "p99
<80ms at 1M records/tenant" — so it is filed as **CTX-7** rather than absorbed
here. TEN-3 keeps the harness and the corrected arms.

The gate in decision 3 remains unapplied. It is applied to the re-run, not to
anything above.

## Measurements (2026-08-10, WITHDRAWN — see the correction above): arms A and B

64,000 records over 8 tenants, 16 scopes, dim 1024, recall@10 against exact
search, 100 queries per regime. `demos/ten-3-dense-leg-sweep.sh`, fresh
database per arm.

| iterative_scan | ef_search | broad recall | broad p50 | selective recall | selective p50 |
|---|---|---|---|---|---|
| off | 100 | 0.341 | 5.91ms | 0.990 | 1.31ms |
| relaxed_order | 100 *(shipped)* | 0.878 | 50.87ms | 1.000 | 1.33ms |
| relaxed_order | 400 | **0.939** | 50.16ms | 1.000 | 1.24ms |
| relaxed_order | 1000 | 0.761 | 30.48ms | 0.998 | 1.19ms |

Four things, in the order they matter.

1. **The selective regime never touches HNSW.** Its plan is
   `records_tenant_scope_idx` → 125 rows → exact sort: perfect recall, 1.3ms,
   and unmoved by every tuning above. Migration 0016 predicted this in
   prose; it is now measured. **Nothing in this table is a reason to
   partition**, because partitioning by tenant cannot reach the regime that
   is already exact and cannot improve the one number it could touch by more
   than tuning already does.
2. **Iterative scanning is worth 2.6x recall and 8.6x latency** (0.341 →
   0.878, 5.9ms → 50.9ms). ADR-0024 decision 5 adopted it on reasoning in
   Phase 1; this is the first number against it, and it is a large one in
   both directions. Anything that later trades it away has to price both.
3. **`ef_search` 100 → 400 is six points of recall for free** — 0.878 →
   0.939 at 50.9ms → 50.2ms. The scan is already running past `ef_search`,
   so raising it changes the batch size rather than the work. The shipped
   default is leaving that on the table.
4. **And it is not monotonic: 1000 is *worse* than 100** (0.761) while also
   being *faster* (30.5ms p50, 51.9ms p95). Faster and worse together means
   the scan is stopping early — pgvector bounds an iterative scan with
   `hnsw.max_scan_tuples` and `hnsw.scan_mem_multiplier`, and a large first
   batch spends that budget sooner. So arm B is a two-dimensional sweep, not
   a one-dimensional one, and the amendment above under-specified it: the
   third unmeasured constant is a bound nobody has set at all.

**What these numbers do not yet support.** Recall carries ±3 points of
run-to-run variance at this corpus size (the shipped default measured 0.847
and then 0.878), because record ids are UUIDv7 and each run builds a
different graph. The 0.878 → 0.939 gap is about twice that and the 1000-arm
gap larger still, but every row here is n=1. Repeats, and a `max_scan_tuples`
axis, come before the gate in decision 3 is applied to anything.

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

**Force 2 — the remedy this feature names is already in the product.**
(Corrected by the amendment above; the original text claimed only that
pgvector had grown the feature and that we had never used it.)
"Mitigates pgvector post-filtering" describes a real problem: an HNSW scan
walks the graph, returns its best `ef_search` candidates, and *then* the
filter runs, so a selective filter can leave far fewer than `limit` rows and
silently lose recall. pgvector 0.8 added **iterative index scans** for exactly
this, and **CTX-1 adopted them** — ADR-0024 decision 5, and
`dense_candidates` setting `hnsw.iterative_scan = relaxed_order` and
`hnsw.ef_search = 100` transaction-locally on every dense query. We ship
pgvector 0.8.6 (`synveda/dev-postgres:17` carries `vector--0.8.6.sql`; the
enterprise image installs the same PGDG package).

So the question TEN-3 inherits is not "should we mitigate post-filtering" but
"is the mitigation we already run insufficient enough to be worth a partition
key". Nobody has measured those two constants against a corpus, which makes
tuning them the cheapest thing on the table and the thing partitioning has to
beat.

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

   - **A**: today's dense leg exactly as it ships — iterative scanning on,
     `relaxed_order`, `ef_search = 100`, no `max_scan_tuples`.
   - **B**: the same, with those three tuned. A sweep of `ef_search`,
     `relaxed_order` against `strict_order`, and a `max_scan_tuples` bound.
     They are constants nobody has measured, and they are the knob the
     product already has for this exact problem.
   - **C**: hash-partitioned by `tenant_id`.

   B is measured before C is built. It costs a sweep; C costs the schema in
   force 4.

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
