# ADR-0012: The embedded Cedar PDP — facade, bootstrap pack, per-tenant store

- **Status**: Accepted
- **Date**: 2026-07-18
- **Feature(s)**: AUTHZ-1
- **Deciders**: sujitn

## Context

ADR-0002 chose Cedar, embedded in-process, as the Policy Decision Point.
AUTHZ-1 now lands it: the `authorize(subject, action, resource, context)`
facade, entities materialised from the hierarchy, a per-tenant policy
store with hot reload, and the µs-level decision AC. It also owes a debt:
ADR-0011 decision 8 left the `/v1/hierarchy/*` admin plane ungated, with
the PDP check named as AUTHZ-1's first obligation.

Forces at play:

- **Layering.** Policy knows nothing of storage and storage knows nothing
  of policy (seed §2.4); both sit directly on `synveda-types`. Whatever
  the PDP needs from Postgres (packs, hierarchy rows) must reach it
  through the caller, never through its own database access.
- **Hot path.** The decision itself must stay in the microseconds so the
  inject SLO (p99 < 150ms) never budgets for authorisation. Anything
  per-request that touches the database is the caller's cost, incurred
  where the caller already reads that data.
- **Sequencing.** Roles (AUTHZ-3), policy packs proper (AUTHZ-2), ABAC
  context (AUTHZ-5), Cedar entity sync (HIER-3), and identities beyond a
  token subject (AUTH-2) all land later. AUTHZ-1 must be enforceable now
  without inventing throwaway versions of those features.
- **Strict by default, zero-config** (seed §2.1, §2.3): a tenant with no
  stored pack must still get a sane, deny-first policy without YAML.

## Decision

1. **Domain-typed facade; Cedar never crosses the crate boundary.**
   `synveda_policy::Pdp::authorize(&Principal, Action, Resource,
   &AuthzContext)` takes and returns domain types only: a typed `Action`
   vocabulary (no free strings), `Resource::{Tenant, Scope}`, and an
   `AuthzDecision` carrying the verdict plus the pack name/version and
   determining policy ids. Cedar types are an implementation detail of
   the crate, so the OpenFGA/OPA adapters (ADR-0002, AUTHZ-6) are a new
   module behind the same signature, not an API change. `AuthzContext`
   carries what the engine cannot fetch itself (today: the resource's
   scope chain; AUTHZ-5 adds ABAC attributes here).
2. **Schema-validated policies, rejected at the boundary.** The crate
   embeds a Cedar schema (entity types `Tenant`, `Principal`, `Scope`;
   the action vocabulary). Every pack — embedded or stored — is parsed
   *and validated* against the schema when compiled, and requests are
   built schema-checked. A malformed or ill-typed pack fails at
   apply/reload time with a reason; it can never fail at decision time.
3. **The `bootstrap` pack is the embedded default.** Compiled into the
   binary as pack `bootstrap` version 1: it permits the hierarchy admin
   actions to principals of the resource's own tenant and nothing else —
   Cedar's default-deny covers every action the pack does not name.
   This preserves ADR-0011 decision 8's semantics (a tenant administers
   its own hierarchy) while converting them from "gap" to "enforced,
   versioned, logged policy". AUTHZ-2 replaces bootstrap with real packs;
   AUTHZ-3 adds roles. Deny-everything instead would brick the admin API
   and the zero-config principle; allow-all would violate strict-by-default.
4. **Entities are materialised from the hierarchy, per request, by the
   caller's data.** The caller passes the resource's scope chain (node +
   ancestors — rows it already reads for its own tenant/ownership
   checks); the crate builds the Cedar entity graph: `Scope` parents
   follow `parent_id` up to the org, the org's parent is the `Tenant`
   entity, and the `Principal`'s parent is its `Tenant`. The bootstrap
   rule is literally `resource in principal.tenant` — the entity
   hierarchy, not string comparison, and never the materialised `path`
   (ADR-0011). A process-wide synced entity cache is exactly HIER-3;
   until it lands, per-request materialisation from caller-supplied rows
   is correct by construction (the rows come from the same transaction)
   and costs the caller one closure-index scan.
5. **Per-tenant policy store: one active pack row, poll-based reload.**
   Migration 0006 adds `policy_packs` (one row per tenant: name,
   monotonically bumped `version`, Cedar `source`) with forced RLS and
   least-privilege grants per the ADR-0009 structural rule. The gateway
   runs a refresher (interval `SYNVEDA_POLICY_REFRESH_SECS`, default 5s):
   for each active tenant it reads the pack row in a tenant transaction,
   skips unchanged versions, compiles and atomically swaps in changed
   ones, and falls back to `bootstrap` where no row exists. A pack that
   fails to compile is logged and counted and the tenant *keeps its
   last-good pack* — a bad apply must not widen or brick a tenant.
   Applying packs is dev/admin plumbing for now (`synveda policy apply`,
   store API); AUTHZ-2 owns the product surface, and VedaFlow eventually
   makes packs governed assets whose commits can drive event-based
   reload (LISTEN/NOTIFY) — polling's bounded staleness is accepted
   until then.
6. **Every decision is logged with its policy version.** `authorize`
   emits, on every call, a tracing event (decision, action, resource,
   tenant, pack name@version, determining policy ids) and increments
   `synveda_authz_decisions_total{action, decision, pack}` — versions go
   to the log/trace, not to metric labels (unbounded cardinality). This
   is the AUTHZ-1 AC and an AUD-1 emission point: when the hash-chained
   log lands, the same call site emits the audit event (ADR-0002).
7. **The hierarchy admin plane is gated now.** Every `/v1/hierarchy/*`
   handler authorizes through the facade before acting — create against
   the parent scope (the tenant, for the root), reads against the
   anchor node, update/delete against the node — after the existing
   uniform-404 ownership check, so cross-tenant probes still see 404,
   never a policy denial oracle. This discharges ADR-0011 decision 8.

## Options considered

1. **Per-request entity materialisation from caller rows (chosen)** —
   correct within the caller's transaction, no sync machinery, costs one
   closure scan the handler mostly pays anyway. Con: each caller must
   remember to supply the chain; acceptable while callers are few, and
   HIER-3 replaces the mechanism wholesale.
2. **Process-wide entity cache synced from hierarchy writes** — that *is*
   HIER-3, with its transactional-consistency AC; building it here would
   front-run its design under a smaller feature.
3. **Attribute-only tenant check (no entity hierarchy)** — a
   `resource.tenant_id == principal.tenant_id` string/UUID comparison
   needs no chain, but exercises none of the machinery AUTHZ-2/3/5 will
   stand on and reduces Cedar to an if-statement.
4. **Files-on-disk policy store with inotify reload** — no RLS, no audit
   trail, drifts across nodes; rejected. Postgres row + poll is boring
   and already tenant-isolated.
5. **LISTEN/NOTIFY reload now** — better latency than polling, but adds
   a dedicated listening connection and reconnect handling for a
   property (sub-5s propagation) nothing yet needs; recorded as the
   upgrade path, driven by VedaFlow policy commits.

## Consequences

- Positive: every governed action from here on has a one-line
  enforcement seam (`pdp.authorize`); decisions are µs-level and carry
  their policy version; packs are tenant-isolated data with fail-safe
  reload; the hierarchy admin gap is closed; AUTHZ-2/3/5 extend the
  schema and packs without touching callers.
- Negative / accepted trade-offs: pack propagation lags up to the poll
  interval; per-request entity building repeats work HIER-3 will cache;
  the bootstrap pack encodes a deliberately broad rule (any tenant
  principal administers its hierarchy) until AUTHZ-2/3 narrow it; the
  refresher iterates tenants (one tenant-transaction each), fine at
  admissible tenant counts, revisited with event-based reload.
- Reversal trigger: unchanged from ADR-0002 — the AUTHZ-6 spike defines
  when relationship checks flip to OpenFGA behind this same facade.

## Compliance notes

Seed §2.2 (policy is enforced, never advisory): the facade is now real
and the first governed surface calls it on every request; no code path
reaches hierarchy mutation without a decision. Tests use test policy
packs through the same store/reload path — never a PDP bypass
(CLAUDE.md). Decisions are trace/metric-visible now and are an AUD-1
emission point (tracked in STATUS.md); the decision log carries pack
name and version for every call, allow and deny alike.
