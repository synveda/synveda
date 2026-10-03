# ADR-0131: Partition policy-convergence leases by tenant

- **Status**: Proposed
- **Date**: 2026-09-30
- **Feature(s)**: OPS-7
- **Deciders**: Synveda maintainers

## Context

ADR-0127/0128 renew one process lease only after every active tenant's stored
Cedar pack refresh succeeds. This correctly closes stale decisions, but one
invalid or slow tenant pack eventually withdraws every gateway and worker,
including unrelated tenants. A provisional idle 32-tenant/four-pack exact-role
probe completed well inside the five-second sweep deadline; it did not test
failure isolation, traffic or a supported maximum. The first small-team offer
has one customer tenant, while a medium-team or dedicated deployment needs an
explicit tenant and pack envelope before admission.

The gateway's current outer admission gate runs before bearer verification has
identified a tenant. Core-worker and optional Apalis work also uses that same
process lease. Merely treating a partial sweep as successful would let a
failed tenant's last-good compile keep authorising work indefinitely. Tenant
isolation must include in-flight work, not just new requests or readiness.

## Decision

Propose separate process and tenant policy-freshness leases for an OPS-7
implementation slice. A successful bounded enumeration and one bounded
refresh attempt for every active tenant keep the process lease fresh even if
some tenant-local attempts fail. A failed enumeration, database authority
drift or a sweep that cannot attempt every tenant within its work bound keeps
the existing process-wide fail-closed response. Each active tenant's lease
starts closed for the current authority generation and renews only after that
tenant's stored sources are read and compiled successfully. A tenant-local
read or compile failure does not renew that tenant's lease and cannot renew it
by another tenant's success. After the existing provisional 30-second age,
that tenant's governed work closes while
other successfully refreshed tenants remain eligible. A later complete
tenant refresh may reopen it. Suspended tenants remain unresolvable.

The implementation must identify and check the tenant lease after verified
credential/tenant resolution and before any governed action, then carry a
tenant-specific cancellation permit through the application future. Claimed
worker and Apalis effects require the same tenant check at claim, execution
and cancellation; pre-tenant login, public health and process readiness retain
the process gate. A newly active tenant cannot serve from a missing local
compile. Database generation changes invalidate every tenant lease before
traffic resumes.

The refresher must bound enumeration, per-tenant work and total work at an
owner-selected active-tenant/pack ceiling. A slow tenant must not prevent
healthy tenants from getting their own refresh attempt before their lease
expires. The admission path must refuse a new tenant or pack that would exceed
the supported envelope; the provisional 32 × 4 probe is measurement, not that
limit. Until the implementation and adversarial acceptance pass, ADR-0127/0128
and their global complete-sweep rule remain in force.

## Options considered

1. **Separate process and tenant leases** — proposed. Preserves fail-closed
   Cedar freshness for the affected tenant while containing tenant-local pack
   failures. It requires changes across gateway, core worker and Apalis.
2. **Keep only the global lease** — safe and current, but an invalid pack or
   slow tenant closes unrelated tenants. It remains the rollback behavior.
3. **Renew the global lease after any partial sweep** — rejected: last-good
   compiles for failed tenants could serve without a bound.
4. **Increase the lease or sweep timeout** — rejected as a substitute for work
   bounds or isolation; it would extend stale-decision exposure.

## Consequences

- Readiness will mean the process can perform tenant-specific admission checks;
  it cannot promise every tenant is healthy. Tenant-specific request failures
  need a stable, non-disclosing unavailable response and tenant-safe aggregate
  telemetry for closed leases and refresh outcomes.
- The owner must choose the tenant/pack ceiling and loaded mutation bound.
  Tests must cover two tenants with one invalid/blocked pack, stale in-flight
  HTTP and provider work, new tenant activation, suspension, recovery,
  database-enumeration failure and authority-generation rollover. A failed
  tenant must be denied after the bound while another keeps serving; a failed
  enumeration must close all tenants. The chosen maximum needs a load probe
  with real request and worker competition.
- Revisit this design if a tenant lease cannot be checked and cancelled at
  every governed effect boundary; retain the current global fail-closed gate
  rather than shipping partial isolation.

## Compliance notes

The proposal changes only when an existing Cedar decision may run. It grants
no authority and cannot bypass forced RLS, VedaFlow or content-free audit.
Keep tenant IDs out of hot metric labels and avoid policy source in telemetry.
