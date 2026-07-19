# ADR-0017: Cedar entity sync — a fragment store fed by the scope-chain cache

- **Status**: Accepted
- **Date**: 2026-07-19
- **Feature(s)**: HIER-3
- **Deciders**: sujitn

## Context

ADR-0012 decision 4 materialised Cedar entities per request from
caller-supplied hierarchy rows and named its own replacement: "a
process-wide synced entity cache is exactly HIER-3". HIER-2 (ADR-0016)
then moved the *rows* into a process-wide cache — warm governed requests
read no hierarchy queries — but every decision still rebuilds the Cedar
entity graph from those rows: uid formatting and parsing, attribute
expressions, and entity construction, per node, per call. HIER-3 closes
the loop: hierarchy changes stream into the Cedar entity store
transactionally. AC: move a team between departments → authz decisions
reflect it in the same transaction boundary.

Forces at play:

- **The transactional AC is a freshness property, not a latency one.**
  A committed move must govern the very next decision; a rolled-back
  move must never be visible to any decision. Whatever holds the built
  entities must inherit exactly the chain cache's freshness — a second,
  independently invalidated cache would be a second chance to serve
  stale authority.
- **Layering** (seed §2.4): policy knows nothing of storage, so the
  entity store cannot read the database on a miss; whatever it builds
  from must keep arriving through the caller. Cedar types never cross
  the crate boundary (ADR-0012 decision 1), so built entities can only
  live inside `synveda-policy`.
- **Identity freshness stays per-request.** The principal entity carries
  `quarantined`, `home`, and `department` — derived from the identity
  row and placement chain that ADR-0016 decision 6 deliberately kept as
  per-request reads. Caching principal entities would break the
  next-request freshness promises of ADR-0013/0015.
- **Bounded memory, no knobs** (ADR-0016 decision 7): the store must be
  O(nodes) per tenant. A (user × resource)-keyed cache of full decision
  snapshots would be the fastest warm path and is rejected on this
  force alone — its cardinality is workload-shaped, and bounding it
  means eviction knobs.
- **Cedar's representation** (cedar-policy 4.x): `Entity` is a plain
  deep-clone value and `Entities::from_entities`/`add_entities` always
  recompute transitive closure. There is no cheap way to append one
  per-request principal to a big prebuilt snapshot, which rules out
  whole-tenant `Entities` replication; small per-decision sets built
  from prebuilt entities are the shape the API rewards.

## Decision

1. **The entity store lives inside `synveda-policy`
   (`entity_store` module), one per `Pdp`.** It maps
   `tenant → (chain-head scope → fragment)`, where a fragment is the
   built Cedar entities for one scope chain: one `Scope` entity per
   node plus the chain's `Tenant` entity. The per-tenant entry count is
   bounded by scopes actually resolved — O(nodes), the ADR-0016 bound.
2. **A fragment's validity is the chain it was built from — checked by
   shape, not by generation.** Each fragment records its source chain's
   entity-relevant shape: the ordered `(id, parent_id, tenant_id,
   kind)` rows. On lookup the caller's chain (the one `gather` resolved
   from the scope-chain cache inside the request's transaction) is
   compared against the stored shape: equal → the built entities are
   correct for that chain by construction; different → rebuild from the
   supplied chain and replace the entry. Freshness is therefore
   *inherited*, transitively, from ADR-0016: the chain cache is
   invalidated post-commit at every hierarchy-mutating seam, the fresh
   chain has a new shape, and a stale fragment can never be served —
   not even by a racing request that repopulates the store with
   pre-move data after the flush (its entry loses the next shape
   comparison and is rebuilt). No generation plumbing, no second
   invalidation protocol, no ordering requirements between caches.
3. **The shape excludes display fields deliberately.** `name`, `slug`,
   `path`, and `depth` do not reach Cedar entities (ADR-0011: the
   materialised path is never used for authorisation), so a rename
   keeps every fragment valid: the chain cache re-reads, the shape
   matches, nothing rebuilds. Only edges the entity graph encodes —
   parentage, tenancy, kind — invalidate.
4. **Per decision: two fragment lookups, one small assembly.** The
   resource chain's fragment and the principal placement chain's
   fragment are fetched (or rebuilt), their entities deduplicated by
   uid (chains share ancestors), the principal's tenant entity is
   ensured, and the principal entity is built fresh per request —
   `quarantined`, `home`, `department` keep riding the per-request
   identity read (ADR-0016 decision 6). `Entities::from_entities` then
   runs over this ≤ chain-length set as before. What the store removes
   is the per-node reconstruction; what it keeps per-request is
   everything whose freshness contract demands it.
5. **One gateway seam invalidates both caches.**
   `AppState::invalidate_hierarchy(tenant)` replaces the four direct
   `scope_chains.invalidate` call sites (hierarchy create/update/delete,
   JIT provisioning) and additionally flushes the tenant's fragments
   (`Pdp::flush_entities`). The flush is hygiene, not correctness —
   decision 2 makes stale service impossible without it — but it keeps
   deleted scopes from lingering as dead entries and gives future
   out-of-process writers (AUTH-4/5) a single seam to call. The
   LISTEN/NOTIFY upgrade path recorded in ADR-0016 decision 7 covers
   this store for free: the same channel that would flush chains
   flushes fragments.
6. **Observability per ADR-0007.** Fragment resolutions are counted
   (`synveda_cedar_entity_fragments_total{outcome="hit"|"rebuild"}`) and
   tenant flushes too (`synveda_cedar_entity_flushes_total`), described
   in the gateway alongside the scope-chain counters. Decisions already
   log pack and version on every call; nothing about the decision log
   changes.

## Options considered

1. **Fragment store validated by chain shape (chosen)** — inherits
   ADR-0016's transactional freshness with zero new invalidation
   protocol; O(nodes); rebuild cost is one chain's entity construction.
   Con: `Entities::from_entities` (closure + schema validation over the
   small merged set) still runs per decision.
2. **Whole-tenant `Entities` snapshot, gateway-pushed on mutation** —
   the literal "entity store synced on write", but Cedar offers no
   cheap way to add the per-request principal to a snapshot: `Entities`
   deep-clones per decision, O(tenant) on the hot path, worse than
   rebuilding per request at the 10k-node fixture. Rejected on Cedar's
   representation.
3. **Full decision-snapshot cache keyed (resource, principal, identity
   state)** — zero build cost on hit, but (user × resource) cardinality
   is unbounded without eviction knobs, and caching principal entities
   trades away ADR-0013/0015's next-request identity freshness.
   Rejected on the bounded-memory and identity-freshness forces.
4. **Generation-guarded entity cache (ADR-0016's own pattern,
   duplicated)** — sound, but the entity store's input is the *cached
   chain*, not the database, so the generation snapshot would have to
   be taken before chain resolution and threaded through `gather` into
   the PDP: cross-crate plumbing to re-prove what shape comparison
   proves locally. Rejected as the strictly more complex equivalent.
5. **Making `authorize` async and letting the PDP resolve chains
   itself via an injected resolver trait** — would centralise entity
   supply, but puts a storage-shaped callback inside the policy crate
   (layering, seed §2.4) and forces async through a µs-level sync
   facade. Rejected.

## Consequences

- Positive: the AC holds end to end — a committed move is reflected by
  the very next decision (chain cache post-commit flush → shape drift →
  rebuild), a rolled-back move never reaches any fragment (it never
  enters the chain cache); warm decisions stop re-parsing uids and
  rebuilding entity attributes per node; renames no longer disturb the
  entity layer at all; CTX-2/3's per-candidate `MemoryRead` calls
  inherit prebuilt fragments; ADR-0012's "per-request entity building
  repeats work HIER-3 will cache" deferral closes.
- Negative / accepted trade-offs: `Entities::from_entities` still runs
  per decision over the merged small set (closure + schema validation)
  — accepted; removing it needs the rejected option 3's cardinality.
  Fragments for deleted scopes persist until the next tenant flush or
  restart — unreachable for decisions, memory-only. The shape
  comparison is O(depth) per lookup — trivial at ≤ 5 levels, and the
  price of having no second invalidation protocol.
- Reversal trigger: if CTX-1's inject latency budget shows the
  per-decision `from_entities` cost dominating at production fan-out,
  revisit option 3 with an explicit eviction design — not before.

## Compliance notes

No new action type exists — fragment resolution is not a decision, and
hierarchy mutations remain the audit emission points recorded in
ADR-0011 (wired at AUD-1). Every decision still passes through the one
`authorize` chokepoint with its pack version logged (seed §2.2,
ADR-0012 decision 6); the entity store feeds that facade the same graph
it built per request before, never a new path to governed assets. Tests
exercise the store through the same facade with test chains — never a
PDP bypass (CLAUDE.md).
