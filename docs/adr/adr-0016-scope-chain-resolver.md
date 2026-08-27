# ADR-0016: Scope chain resolver — read-through cache over the closure table

- **Status**: Accepted
- **Date**: 2026-07-19
- **Feature(s)**: HIER-2
- **Deciders**: sujitn

> **Superseded by ADR-0074 (CPR-7, 2026-08-20).** The `ScopeChainCache`,
> its invalidation seam and its metrics are deleted with the tree they
> cached. Chains resolve per request through `scope_closure`, so "a move
> governs the very next request" is structural rather than cache-dependent.
> The latency discipline this ADR set transfers to the anchor resolver
> (ADR-0073), and ADR-0073 decision 7 holds the deferred cache if
> measurement ever asks for one.

## Context

Every governed request resolves scope chains twice: the resource's chain
(anchor node → … → org root) and the principal's placement chain (user
node → … → org root). Since AUTHZ-1 the gateway's `authz::gather` reads
both per request — up to four hierarchy queries (`node` + `ancestors`,
twice) before the PDP sees a decision context — and ADR-0012/0014/0015
all recorded the same deferral: chains are read per request "until
HIER-2/3 cache them". HIER-2 is the resolver: given an identity, the
ordered scope chain for composition (user → … → org), cached, invalidated
on hierarchy change. AC: a cache invalidation test, and p99 < 0.5ms warm.

Forces at play:

- **The chain has an exact invalidation signal.** Hierarchy mutations are
  rare, admin-rate events that all pass through a handful of gateway
  seams (HIER-1 CRUD handlers, JIT provisioning). Nothing else changes a
  chain.
- **Pack assignments and role bindings do not.** ADR-0014 decision 3 and
  ADR-0015 decision 3 promise that a pack switch or a new binding is in
  force *on the very next request*. That freshness currently rides on
  per-request reads; caching them would either break the promise or
  demand invalidation seams in the policy and roles planes too.
- **The cache must not weaken tenant isolation.** RLS does not bite on
  the dev-compose superuser connection (ADR-0009's accepted trade-off),
  and a process-global cache sits above RLS entirely — its correctness
  must not depend on it.
- **Downstream consumers.** CTX-2's composition engine (in
  `synveda-retrieval`) walks the same chain on the inject hot path
  (p99 < 150ms, seed §10). The crate dependency rule
  (types ← store ← retrieval ← gateway) decides where the resolver may
  live if retrieval is to reuse it.
- **Zero-config** (seed §2.1): no tuning knobs, no YAML.

## Decision

1. **The resolver lives in `synveda-store` (`scope_chain` module), below
   retrieval.** `ScopeChainCache::resolve(executor, tenant_id, scope_id)`
   returns the ordered chain — the node itself first, then ancestors
   nearest-first, org root last — as a shared `Arc<[HierarchyNode]>`.
   Identity → chain is the existing composition:
   `identities::by_subject(..).scope_id` → `resolve` — the identity row
   itself stays a per-request read (it derives `quarantined` from
   placement and is one unique-index lookup).
2. **A miss is one closure query.** New `hierarchy::chain` fetches node +
   ancestors in a single index scan (`distance >= 0`, ordered by
   distance) — discharging ADR-0011's "HIER-2's scope chain is one
   closure query" — and filters `tenant_id` explicitly in SQL. The cache
   key is `(tenant, scope)` and the query cannot adopt a foreign chain
   even where RLS does not bite; a cross-tenant probe reads nothing and
   caches nothing.
3. **One `RwLock` over per-tenant entries: a generation counter plus a
   chain map** — the PDP's own concurrency pattern (ADR-0012), zero new
   dependencies. Reads take the read lock; `invalidate(tenant)` takes the
   write lock, clears the tenant's chains, and bumps its generation.
4. **The generation closes the read/invalidate race.** A missing entry is
   populated read-through: snapshot the tenant's generation *before* the
   database read, insert *only if the generation is unchanged*. A
   resolver that read pre-move rows concurrently with a committed move
   finds the generation bumped and discards its stale chain (under READ
   COMMITTED a query that starts after the bump sees the new rows, so an
   unchanged generation proves freshness). Callers must not resolve after
   staging hierarchy writes in the same transaction — `gather` always
   runs before mutation, so every current caller satisfies this.
5. **Invalidation is tenant-wide, post-commit, at every gateway seam that
   commits a hierarchy mutation**: the HIER-1 create/update/delete
   handlers and JIT provisioning's node-creating path. Uniformly — a
   fresh leaf strictly invalidates nothing (new nodes have no cached
   entry; negative results are never cached), but "any committed
   hierarchy mutation bumps the tenant" is an invariant one can hold in
   one's head, and a spurious flush costs one closure query per scope
   re-touched. Subtree-precise invalidation buys nothing at admin-edit
   rates.
6. **Nothing else is cached.** Pack assignments, role bindings, and
   identity rows stay per-request reads — each a single index scan inside
   the already-open tenant transaction — because that is what makes the
   next-request freshness promises of ADR-0014/0015 true. This closes the
   "until HIER-2/3 cache them" deferrals in favour of *chains only*;
   revisit only if CTX-1's inject latency budget demands it, with
   invalidation seams in the policy/roles planes as the price.
7. **No eviction, no TTL, no knobs.** The cache is bounded by org shape —
   O(nodes) entries of ≤ depth-5 chains per active tenant, a few MB at
   the 10k-node AC fixture. The gateway is the hierarchy's only
   production writer (the CLI does not touch it); any future
   out-of-process writer (SCIM sidecar, AUTH-5 directory sync, break-glass
   SQL) must bring an invalidation channel — LISTEN/NOTIFY is the
   recorded upgrade path (ADR-0012), and a gateway restart is the manual
   recovery.

## Options considered

1. **Read-through cache in `synveda-store`, generation-guarded,
   tenant-wide invalidation (chosen).**
2. **Cache in the gateway's `authz` module** — closest to the only
   current caller, but `synveda-retrieval` cannot import upward, so
   CTX-2 would grow a second resolver or a duplicate cache. Rejected on
   the dependency rule.
3. **A cache crate (`moka`/TTL, `dashmap`)** — buys eviction and sharded
   locking the workload does not need (entries are small, writes are
   admin-rate), at the cost of a new core-path dependency and a
   `deny.toml` licence review. Rejected: `std` suffices; boring wins.
4. **Subtree-precise invalidation** (compute the affected descendant set
   per move) — minimises spurious misses but reintroduces the closure
   surgery's complexity at the cache layer, and a miss costs one index
   scan. Rejected at admin-edit rates.
5. **Cache assignments and bindings too** — halves the remaining
   per-request reads but breaks ADR-0014/0015's next-request freshness
   unless the policy and roles planes grow invalidation seams as well.
   Rejected; recorded as decision 6's reversal condition.
6. **LISTEN/NOTIFY invalidation now** — required the moment a second
   process writes hierarchy; today there is exactly one writer and the
   channel would be a dead code path. Deferred, not rejected.

## Consequences

- Positive: warm governed requests read zero hierarchy rows (previously
  up to four queries); a warm resolve is a read-lock and an `Arc` clone,
  so the 0.5ms p99 AC holds by construction and CTX-2 inherits a
  resolver that fits inject's 150ms budget with headroom. Invalidation
  is exact for every API-path mutation: bumped after commit, before the
  mutating request's response — the next request re-reads committed
  truth.
- Negative / accepted trade-offs: tenant-wide flushes over-invalidate
  (a provisioning burst re-misses once per touched scope — one index
  scan each); a request that resolved before a concurrent move finishes
  its handling on the old chain (unchanged from today's transaction
  semantics); out-of-process hierarchy writes leave the cache stale
  until the next in-process mutation or restart (no such writer exists
  in production today — decision 7 names the trigger for the NOTIFY
  upgrade).
- The `DecisionInput` chains become shared slices (`Arc<[HierarchyNode]>`);
  `AuthzContext` borrows them unchanged.

## Compliance notes

Resolution is observable per ADR-0007: the `store.scope_chain.resolve`
span records hit/miss, `synveda_scope_chain_resolutions_total{outcome}`
counts them, and `synveda_scope_chain_invalidations_total` counts tenant
flushes. No new action type exists — resolution is not a decision, and
hierarchy mutations remain the audit emission points recorded in
ADR-0011 (wired at AUD-1). The cache introduces no path to governed
assets: it feeds the same PDP facade the same data it read per request
before (seed §2.2).
