# ADR-0009: Row-level security as the tenant-isolation backstop

- **Status**: Accepted
- **Date**: 2026-07-18
- **Feature(s)**: TEN-2
- **Deciders**: sujitn

## Context

TEN-1 made the tenant the root isolation boundary at the gateway; the store
still trusts its callers — `records` queries filter by id alone, and nothing
below the application layer would notice a query that forgot a tenant
predicate. TEN-2 adds the database-level backstop: RLS policies on every
tenant-scoped table, keyed to a session GUC, so an application bug cannot
cross tenants. The AC: an adversarial suite proves direct SQL with the wrong
tenant GUC returns zero rows on every table.

Forces at play:

- **Connection pooling.** The gateway holds a `PgPool`; anything set with
  session lifetime on a connection survives into the next request that
  borrows it. A leaked tenant GUC would be a cross-tenant bug introduced by
  the very feature meant to stop them.
- **The dev/compose superuser.** `POSTGRES_USER=synveda` makes the dev role
  a superuser with BYPASSRLS; superusers bypass RLS unconditionally, and
  plain table owners bypass it unless the table says otherwise. Policies
  alone protect nothing — the enforcement role model matters as much as the
  policies.
- **Views.** `records_versions` (the as-of surface) is a view; by default a
  view evaluates the base tables' RLS as the *view owner*, not the caller —
  precisely the bypass this feature exists to close.
- **Bootstrap order.** Tenant resolution (TEN-1) reads `tenants` before any
  tenant context exists. A GUC-keyed policy on `tenants` would deadlock the
  front door.
- Migrations must stay portable: they may run against clusters where roles
  already exist, and they must never embed credentials.

## Decision

1. **GUC contract.** The current tenant is asserted per transaction in the
   `synveda.tenant_id` GUC. A `synveda_current_tenant()` SQL function
   (`nullif(current_setting('synveda.tenant_id', true), '')::uuid`, stable)
   is the single reading point. Unset or empty ⇒ NULL ⇒ every policy
   denies: a connection that skipped the setup sees zero tenant-scoped
   rows. A malformed value fails the query rather than admitting rows. Fail
   closed in both directions.
2. **Transaction-local, not session-local.** The feature text says "per
   connection"; with a pool the safe realisation is per *transaction on* a
   connection: `set_config($tenant, is_local := true)` reverts on commit or
   rollback, so a pooled connection can never carry a tenant into the next
   request. `synveda_store::rls::begin_tenant_tx(pool, tenant_id)` is the
   application entry point — tenant as an explicit argument, never read
   from ambient context (the ADR-0008 layering decision). Data-path
   features (MEM-1, CTX-1..3) must reach tenant-scoped tables only through
   it.
3. **Policies.** `records` and `records_history` get
   `ENABLE ROW LEVEL SECURITY` **and** `FORCE ROW LEVEL SECURITY` (owners
   are not exempt), with one policy each, all commands, all roles:
   `USING` and `WITH CHECK` both `tenant_id = synveda_current_tenant()`.
   Reads outside the GUC tenant vanish; writes outside it raise 42501,
   which the store maps to `Error::Internal` — a cross-tenant write attempt
   is an application defect, never a caller error.
4. **`records_versions` becomes `security_invoker = on`** so as-of queries
   evaluate base-table RLS as the caller. Without it the backstop silently
   excludes the entire history surface.
5. **Enforcement role.** Migration 0003 creates `synveda_app` — NOLOGIN,
   non-superuser, no BYPASSRLS — if absent (roles are cluster-global, hence
   the guard), and grants least privilege: SELECT on `tenants`
   (control-plane read for resolution; admission stays an owner-role
   operation), DML on `records`, SELECT+INSERT on `records_history`
   (the FND-4 archive triggers run with invoker rights and must insert;
   the append-only triggers and the AUD-1 hash chain guard history
   integrity), SELECT on `records_versions`. LOGIN and credentials are
   provisioned per deployment profile (OPS-1/OPS-2), never by a migration.
   Dev, tests, and the demo reach the role via `SET LOCAL ROLE
   synveda_app` from the compose superuser connection — same enforcement
   semantics as a direct login, zero credential management.
6. **`tenants` is not GUC-keyed.** It is the tenant registry itself —
   resolution precedes tenant context. "Every table" in the AC means every
   tenant-scoped table, defined structurally as *any table with a
   `tenant_id` column*. The adversarial suite encodes that definition as a
   completeness guard: it discovers tenant-scoped tables from `pg_catalog`
   and fails if any is missing forced RLS or is not in the suite's covered
   list. A migration adding a tenant-scoped table must enable+force RLS,
   add the policy, and grant `synveda_app` in the same migration, then
   extend the suite — forgetting either breaks the build.

## Options considered

1. **Transaction-local GUC + forced RLS + NOLOGIN app role (chosen)** —
   pool-safe by construction, portable migrations, no credentials in the
   repo. Con: "per connection" becomes "per transaction"; every data-path
   call site must open its work through `begin_tenant_tx`.
2. **Session GUC set on connection acquire** — literal reading of the
   feature text, but either resets leak across pooled requests or every
   acquire pays a round-trip; one missed reset is a cross-tenant read.
   Rejected: the failure mode is the exact bug TEN-2 exists to stop.
3. **LOGIN role with password created by migration/compose** — closest to
   production shape, but embeds a credential in a migration or breaks on
   existing dev volumes (initdb scripts do not re-run). Deferred to the
   deployment profiles; `SET LOCAL ROLE` proves identical enforcement now.
4. **Per-tenant database roles** — strongest separation, but role-per-tenant
   explodes operationally at SMB scale and buys nothing over the GUC for
   the bug-backstop threat model.
5. **SECURITY DEFINER archive triggers** (so `synveda_app` needs no INSERT
   on `records_history`) — closes direct history inserts but imports
   definer/search_path pitfalls into trigger code; history integrity is
   already the append-only triggers' and the hash chain's job. Rejected.

## Consequences

- Positive: a forgotten tenant predicate anywhere above the store now
  returns zero rows or raises, instead of leaking; the as-of surface is
  covered; the schema polices its own growth via the completeness guard;
  the role model is production-shaped before any data-path feature lands.
- Negative / accepted trade-offs: dev connections (compose superuser)
  bypass RLS, so only code paths exercised under `synveda_app` — the AC
  suite, the demo, and eventually the deployed gateway — observe the
  backstop. The policy predicate costs one stable-function comparison per
  row; revisit with TEN-3 partitioning if it shows up in plans.
- **Threat model honesty**: this is a backstop against application bugs,
  not against a principal who can execute arbitrary SQL — any role may
  `set_config` any tenant id. Credential separation, per-tenant keys, and
  hostile-principal defences are TEN-4/TEN-5/OPS territory.
- Reversal trigger: if TEN-3's partition-per-tenant layout makes
  partition-level grants the primary isolation mechanism, or RLS predicate
  cost degrades the CTX-1 read path beyond its SLO, revisit policy shape
  (not the GUC contract, which callers would keep).

## Compliance notes

RLS denials surface as zero-row results and 42501 errors; both are visible
in traces via the store's instrumented spans. They are audit-relevant
(defence-in-depth trips imply an application defect) — AUD-1 must include
the 42501 → `Error::Internal` path as an emission point; tracked in
STATUS.md alongside the TEN-1 deferral. No PDP bypass is introduced: RLS is
subordinate to the PDP (seed §2.2), never a substitute for it.
