# ADR-0014: Policy packs — named bundles, per-node assignment, the composition seam

- **Status**: Accepted
- **Date**: 2026-07-19
- **Feature(s)**: AUTHZ-2
- **Deciders**: sujitn

## Context

AUTHZ-2 lands the product's policy packs (seed §6): `regulated-strict`,
`standard`, and `open-collaboration` as versioned Cedar bundles applied
per hierarchy node, inherited downward with override rules. Its AC —
"switching a team's pack changes inject composition in the next
session" — names a surface (inject, CTX-3) that does not exist yet, so
the AC must be discharged at the seam inject will stand on: the PDP
decision that governs which scopes' memories may compose.

What AUTHZ-1 left behind (ADR-0012): one stored pack row per tenant
(PK `tenant_id`) that replaces the embedded `bootstrap` pack wholesale;
a poll-based refresher; `bootstrap@2` carrying both the
tenant-administers-its-hierarchy permit and AUTH-2's quarantine forbid
(ADR-0013 decision 5). ADR-0012 explicitly deferred the product surface
and per-node application to AUTHZ-2.

Forces:

- **Strict by default, zero-config** (seed §2.1, §2.3): with nothing
  assigned, a tenant must land on `regulated-strict` semantics.
- **Sequencing.** Roles (AUTHZ-3), ABAC classification (AUTHZ-5), the
  scope-chain resolver (HIER-2), entity sync (HIER-3), and the whole
  memory/context surface (MEM, CTX) land later. The packs must be real —
  not throwaway — while expressible with what exists: identities placed
  in the hierarchy (AUTH-2) and the hierarchy itself.
- **Trustworthiness.** A pack name like `regulated-strict` is a promise
  to a compliance reviewer; its meaning must not vary per tenant.
- **Fail closed.** The quarantine forbid must survive any pack switch;
  a dangling or broken pack must never widen access (ADR-0012's
  last-good rule extends to assignment resolution).
- **Layering** (seed §2.4): policy still never touches storage; whatever
  resolution needs (assignments, the principal's placement chain) is
  caller-supplied data.

## Decision

Packs become named, versioned bundles; *assignment* of a pack to a node
becomes request-time data resolved nearest-ancestor-first; the three
product packs ship embedded; and every pack — embedded or stored —
compiles on top of an invariant base layer.

1. **Three embedded packs; `regulated-strict` is the default.**
   `regulated-strict@1`, `standard@1`, `open-collaboration@1` are
   compiled into the binary (like bootstrap was), versioned by
   constants bumped on source change. A scope with no assignment
   anywhere on its chain and no tenant default runs `regulated-strict`
   (seed §2.1). The `bootstrap` pack is retired — its hierarchy-admin
   permit moves into all three packs (unchanged, tenant-wide, until
   AUTHZ-3 narrows it with roles), which is exactly the replacement
   ADR-0012 decision 3 promised.
2. **The invariant base layer.** AUTH-2's quarantine forbid moves out of
   any one pack into an embedded `base.cedar` that `compile()` prepends
   to *every* pack source, stored custom packs included. A tenant
   authoring a custom pack can therefore never drop the fail-closed
   quarantine rule; forbid overrides permit, so prepending is safe by
   Cedar semantics. The base is part of the compiled artifact, not a
   separately versioned pack — the decision log keeps naming the
   effective pack@version.
3. **Assignment is data, not a policy swap.** New tenant-scoped tables
   (forced RLS + grants per the ADR-0009 structural rule):
   `policy_pack_assignments (tenant_id, scope_id → pack_name)` — the
   per-node override — and `policy_pack_defaults (tenant_id →
   pack_name)` — the tenant-wide default AUTHZ-1's single row becomes.
   The effective pack for a decision is a property of the *resource*:
   walk the resource node's parent chain from the node upward; the
   nearest assignment wins; else the tenant default; else
   `regulated-strict`. That walk runs in the PDP over caller-supplied
   rows (the chain it already gets, plus the assignment rows for that
   chain), read in the caller's own transaction — so switching a team's
   pack is visible to the *very next request*, beating the AC's
   next-session bound without touching the refresher. Hot reload keeps
   governing custom pack *source* changes only.
4. **Inheritance and override rules.** A node inherits the pack in
   force at its parent; assigning a pack at a node overrides it for the
   whole subtree below (until a deeper assignment); removing the
   assignment falls back to the inherited one. Assigning is itself a
   PDP-gated action (`PolicyAssign`) and an audit emission point —
   decided under the pack the node *inherits* (the resolution walk
   skips the node's own assignment): changing a node's governance is
   authorized by the surrounding governance, never by the pack being
   replaced, so a restrictive custom pack cannot seal its own node
   against reassignment. Who may assign stays tenant-wide (like
   hierarchy admin) until AUTHZ-3 lands roles; approval-gated pack
   changes arrive when VedaFlow governs packs as assets (tech
   plan §2.3).
5. **`MemoryRead` is the composition seam.** The schema and action
   vocabulary gain `MemoryRead` (applies to `Scope`): "may this
   principal's inject/recall composition include memories attached to
   this scope?" CTX-1/2/3 will ask exactly this question per candidate
   scope; the AC test asks it today through the same facade. The packs
   differ only here:
   - `regulated-strict`: own chain only — `principal in resource`. The
     principal entity is now parented to its placement scope (AUTH-2's
     identity → personal user scope), so membership is the entity
     hierarchy, never string or path comparison (ADR-0011, ADR-0012).
   - `standard`: own chain, plus department-wide sharing — resource in
     the principal's department subtree (`resource in
     principal.department`, an optional attribute the PDP derives from
     the placement chain), excluding other people's personal (`user`)
     scopes.
   - `open-collaboration`: own chain, plus tenant-wide read
     (`resource in principal.tenant`), again excluding foreign personal
     scopes. The seed's "non-restricted content" qualifier is AUTHZ-5's
     classification context; until then the personal-scope exclusion is
     the load-bearing privacy floor.
   A principal with no placement (dev HS256 subject, or pre-JIT) has no
   scope parent: `MemoryRead` denies everywhere under every pack —
   strict by default.
6. **Stored custom packs keyed by name; product names reserved.**
   `policy_packs` PK widens to `(tenant_id, name)`: a tenant may store
   several named packs, each independently versioned by `apply`, and
   assignments may reference stored or embedded names. The three
   product names (and `bootstrap`) are reserved — a check constraint
   refuses storing them — so `regulated-strict` means the same thing in
   every tenant, forever. A tenant wanting a variant names it theirs
   (`acme-strict`) and assigns it.
7. **Dangling assignments fail strict, not dark.** Resolution of an
   assigned name with no compiled pack (stored pack deleted or never
   compiled) falls back to `regulated-strict`, logs a warning, and
   counts it — never deny-everything (bricks the admin plane for a data
   error), never last-assigned (widens). The store refuses to `clear` a
   stored pack that assignments still reference, so the dangling case
   is out-of-band writes only.
8. **The product surface.** `/v1/policy/packs` (GET: embedded + stored
   packs with versions), `/v1/policy/default` (GET/PUT/DELETE: tenant
   default), `/v1/hierarchy/nodes/{id}/policy` (GET: the *effective*
   pack and where it was assigned; PUT/DELETE: the node's assignment).
   All behind tenant resolution, uniform-404 ownership first, then the
   PDP (`PolicyRead`/`PolicyAssign`). Mutations are audit emission
   points (AUD-1); until then: traces plus
   `synveda_policy_operations_total{op, outcome}`. The CLI keeps
   `synveda policy apply/clear` as dev plumbing, growing `--name`
   awareness and `assign`.

## Options considered

1. **Assignment resolved from request-time data (chosen)** — pack
   switches take effect next request; no per-node compiled artifacts;
   the refresher stays a per-source concern. Con: two more small reads
   on governed paths (assignments for the chain, the principal's
   placement chain); acceptable where callers already read the chain,
   and HIER-2/HIER-3 own caching.
2. **Effective pack compiled per node by the refresher** — precomputes
   the walk, but multiplies compiled artifacts by node count, ties
   switch latency to the poll interval (fails the AC's spirit), and
   turns every hierarchy move into a recompile fan-out. Rejected.
3. **One synthesized tenant policy where every rule is conditioned on a
   scope attribute carrying the effective pack** — single PolicySet,
   but pushes the inheritance walk into entity attributes materialised
   per request anyway, and makes the decision log's "which pack
   decided" a derived fact instead of a first-class one. Rejected.
4. **Let stored packs shadow product names** — no reservation needed,
   but `regulated-strict` would no longer be a promise, only a
   suggestion. Rejected on trustworthiness grounds.
5. **Quarantine forbid stays per-pack by convention** — one forgotten
   line in a custom pack silently un-quarantines a tenant's compromised
   identities. Rejected; invariants don't travel by convention.

## Consequences

- Positive: pack switching is immediate and per-node; the three product
  packs are real, embedded, and identical across tenants; the
  quarantine invariant survives any pack; the composition seam
  (`MemoryRead`) exists before the composition engine, so CTX-1/2/3
  arrive to a decided question; AUTHZ-3 (roles) and AUTHZ-5 (ABAC) are
  schema/pack extensions, not reshapes.
- Negative / accepted trade-offs: governed handlers pay two extra
  indexed reads (placement chain, chain assignments) until HIER-2/3
  cache them; embedded pack versions are hand-bumped constants;
  department semantics under `standard` collapse to strict when a
  hierarchy skips the department level; who-may-assign is tenant-wide
  until AUTHZ-3. Node-level assignments cannot seal themselves
  (decision 4's skip-self rule), but the *tenant default* is decided
  under itself — the top of the chain has no "above" — so a tenant
  default naming a custom pack that fails to permit `PolicyAssign`
  locks the tenant's policy plane; the store-level CLI (`synveda
  policy`, tenant-tx store calls) is the documented break-glass, and
  the product packs all permit the admin planes, so the hazard is
  confined to custom packs.
- Reversal trigger: if per-request assignment reads show up in the
  inject SLO budget (p99 > 150ms, seed §3) once CTX-3 lands, HIER-2's
  cached scope-chain resolver absorbs assignment rows into its cache
  key space; the resolution *semantics* (nearest-ancestor-first) stay.

## Compliance notes

Seed §2.2 holds: there is still exactly one enforcement seam
(`Pdp::authorize`), now deciding under the resource's effective pack;
no caller selects a pack directly. Every decision keeps logging pack
name@version (allow and deny), so an auditor can reconstruct which pack
was in force for any historical decision from the decision log alone.
Assignment mutations are new audited action types (AUD-1 emission
points, tracked in STATUS.md). Tests exercise packs through the same
store/assignment/reload paths as production — never a PDP bypass.
`policy_pack_assignments` and `policy_pack_defaults` are tenant-scoped
with forced RLS (ADR-0009); the RLS completeness guard covers them.
