# ADR-0011: Hierarchy store — closure table with a materialised path

- **Status**: Accepted
- **Date**: 2026-07-18
- **Feature(s)**: HIER-1
- **Deciders**: sujitn

> **Superseded by ADR-0074 (CPR-7, 2026-08-20).** `hierarchy_nodes`,
> `hierarchy_closure` and the rank vocabulary this ADR decided are deleted
> whole. The closure-table shape and its latency discipline survive
> unchanged in `scopes` + `scope_closure` (ADR-0070); what does not survive
> is the fixed `org → division → department → team → user` ladder and the
> child-outranks-parent rule. Nothing was translated: no row of
> `hierarchy_nodes` became a scope, in either direction, at any time.

## Context

The tenancy hierarchy (seed §4.1) — org → division → department → team →
user, with optional levels — is the attachment point for every governed
asset and the substrate for Cedar entities (HIER-3), scope-chain
composition (HIER-2, CTX-2), and JIT provisioning (AUTH-2). HIER-1 stores
it: closure table + materialised path per the seed and tech plan §1.1,
CRUD via admin API. AC: a 10k-node hierarchy answers ancestor/descendant
queries in under 1ms.

Forces at play:

- **Read-heavy, write-light.** Every inject resolves a scope chain;
  hierarchy edits are rare admin/directory-sync events. The layout should
  buy O(index-scan) reads and may pay for them on writes.
- **"Configurable depth"** must not become a config file (seed §2.1
  zero-config). The level vocabulary is fixed; which optional levels a
  tenant uses should fall out of the data, not out of YAML.
- **Ordering.** HIER-1 lands before the PDP (AUTHZ-1) and the audit log
  (AUD-1) — the Phase 1 topological order puts hierarchy nodes first
  because Cedar entities and JIT provisioning need them to exist.
- **Tenant isolation.** ADR-0009's structural rule: any table with a
  `tenant_id` column ships forced RLS, the policy, and least-privilege
  grants in the same migration, and extends the adversarial suite.
- **Moves must be transactional.** HIER-3's AC (move a team → authz
  reflects it in the same transaction boundary) rules out any
  eventually-consistent derived structure.

## Decision

1. **Two tables, one authority.** `hierarchy_nodes` holds the nodes
   (id = `ScopeId`, parent pointer, kind, slug, name, depth, path);
   `hierarchy_closure` holds every (ancestor, descendant, distance) pair
   including self-rows at distance 0. The closure table is the *query*
   structure — ancestor and descendant lookups are single index scans,
   which is what makes the <1ms AC hold at 10k nodes and beyond. The
   materialised `path` (slug chain, e.g. `acme/emea/payments`) is for
   human-readable display, stable subtree ordering, and prefix debugging —
   never for authorisation decisions.
2. **Closure maintenance is explicit store code, not triggers.** Create,
   move, and delete run their closure surgery as plain SQL statements
   inside the caller's transaction (`synveda_store::hierarchy`). Triggers
   earned their place for bitemporal stamping (ADR-0006) because every
   write path must be stamped; hierarchy writes have exactly one caller —
   this module — and explicit statements keep the algorithm readable,
   testable, and sqlx-checked. Callers must wrap multi-statement
   operations in a transaction (`rls::begin_tenant_tx` on the data path).
3. **Configurable depth = a fixed vocabulary plus a rank rule.** `kind` ∈
   {org 0, division 1, department 2, team 3, user 4}; a child's rank must
   be strictly greater than its parent's. Optional levels are simply
   skipped ranks (org → department is legal); users can never have
   children (nothing outranks 4); the single root per tenant is the org.
   No per-tenant depth configuration exists or is needed.
4. **Integrity lives in the schema.** One root per tenant (partial unique
   index on `parent_id is null`); sibling slugs unique
   (`unique nulls not distinct (tenant_id, parent_id, slug)`); paths
   unique per tenant; a composite FK `(tenant_id, parent_id) →
   (tenant_id, id)` makes a cross-tenant parent unrepresentable; closure
   rows cascade-delete with their nodes via composite-tenant FKs. Slugs
   reuse the tenant slug grammar (ADR-0008) and are immutable in HIER-1 —
   display names rename freely; a slug is a stable handle, and slug
   renames cascade into every descendant path (deferred until something
   needs it).
5. **Moves are closure surgery under row locks.** `move_node` locks the
   node and the new parent (`for update`), rejects rank violations and
   moves under one's own subtree (closure lookup), then: deletes closure
   rows linking outside-ancestors to the subtree, cross-joins the new
   parent's ancestry with the subtree to reinsert them, and updates
   `depth` and `path` for the subtree in single statements. Deletes are
   leaf-only (children present → conflict); subtree deletion is an
   explicit later feature, not a cascade surprise.
6. **RLS per the structural rule.** Both tables get enabled+forced RLS
   keyed to `synveda_current_tenant()`, `synveda_app` gets
   select/insert/update/delete on `hierarchy_nodes` and
   select/insert/delete on `hierarchy_closure` (closure rows are only
   ever inserted and deleted), and the TEN-2 adversarial suite's covered
   list grows by both tables — all in migration 0004.
7. **SSD-era planner cost model.** Postgres's default
   `random_page_cost = 4.0` models spinning disks; at the 10k-node AC
   fixture it makes the planner prefer a full seq scan + hash join over
   the closure-driven nested loop for descendant listings (~1.5ms vs
   ~0.2ms measured), and the same misplan would tax every later hot read
   path. Migration 0005 sets `random_page_cost = 1.1` (the standard SSD
   setting) database-scoped, so every deployment that applies migrations
   gets it; operators with exotic storage override per role or with a
   later `ALTER DATABASE` (OPS-1/OPS-2 own deployment profiles).
8. **Admin API now, PDP gate next feature.** CRUD lands on
   `/v1/hierarchy/*` behind tenant resolution. Until AUTHZ-1 embeds
   Cedar, any authenticated principal of the tenant can administer its
   hierarchy — the same seam-first sequencing as ADR-0008's dev verifier:
   the PDP check slots into the already-single chokepoint
   (`hierarchy` handlers) when it exists, and AUTHZ-1's AC must cover
   these routes. This is a sequencing gap inside the trust boundary of a
   tenant's own token, not a PDP bypass: no governed asset (memory,
   context, skill, prompt) is reachable through it. The handlers also
   check node ownership explicitly (uniform 404), so the API stays
   tenant-correct even on connections where the RLS backstop does not
   bite (the dev-compose superuser, ADR-0009).

## Options considered

1. **Closure table + materialised path (chosen)** — O(1)-ish reads both
   directions, transactional moves, path for humans. Con: O(subtree ×
   ancestors) rows to maintain on move; acceptable at admin-edit rates.
2. **Materialised path only** (`path like 'prefix/%'`) — fewer moving
   parts, but ancestor queries need path parsing, prefix indexes are
   text-size sensitive, and every move rewrites the subtree's key. The
   seed names both structures; path-only was rejected as the primary.
3. **Recursive CTEs over `parent_id`** — no derived state at all, but
   per-query recursion cost scales with depth and the <1ms AC would ride
   on planner behaviour rather than an index scan. Rejected for the hot
   path; the parent pointer stays as the adjacency ground truth.
4. **`ltree` extension** — purpose-built, but adds an extension
   dependency for what two plain tables express, and its label grammar
   would constrain slugs. Rejected: boring plain SQL wins.
5. **Closure maintained by triggers** — survives any future writer, but
   hides the algorithm from sqlx compile-time checking and from the
   module that owns the invariant. Rejected while this module is the
   only writer.

## Consequences

- Positive: ancestor/descendant queries are index scans (AC holds with
  headroom); HIER-2's scope chain is one closure query; HIER-3 can sync
  Cedar entities inside the same transaction as the move; the schema
  rejects cross-tenant edges, duplicate roots, and sibling collisions
  without application help.
- Negative / accepted trade-offs: closure storage is O(nodes × mean
  depth) — trivial at enterprise org shapes (5 levels); moves rewrite
  subtree closure rows and paths under row locks, serialising concurrent
  edits of the same subtree (correct, and admin-rate); slug immutability
  is a product limitation until a rename feature owns the path cascade.
- Reversal trigger: if directory sync (AUTH-5/SCIM) turns out to bulk-move
  subtrees at rates where closure rewriting dominates, revisit with a
  batch rebuild path (truncate-and-rebuild per tenant is simple and
  correct) before reaching for triggers or ltree.

## Compliance notes

Hierarchy CRUD is an audit emission point (node created/renamed/moved/
deleted): events are wired when AUD-1's hash-chained log lands, tracked
in STATUS.md alongside the TEN-1 deferral; until then operations are
visible in traces (`store.hierarchy.*` spans) and the
`synveda_hierarchy_operations_total` counter. The admin surface's PDP
gate is AUTHZ-1's first obligation (decision 7). No path to governed
assets bypasses the PDP.
