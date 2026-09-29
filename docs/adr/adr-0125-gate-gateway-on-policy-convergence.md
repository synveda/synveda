# ADR-0125: Gate gateway traffic on policy-pack convergence for its authority generation

- **Status**: Accepted
- **Date**: 2026-09-29
- **Feature(s)**: OPS-7
- **Deciders**: Synveda maintainers

## Context

The gateway's database authority sentinel opens the HTTP gate before its
background task reads and compiles stored tenant policy packs. A request in
that window can resolve a stored pack assignment from PostgreSQL while the
embedded Cedar PDP still has only its product packs. The PDP's documented
fallback is `regulated-strict`, but a tenant's custom pack can deny something
that default permits. Process restart and authority recovery reproduce the
window. The worker already waits for first convergence before readiness.

OPS-7 also named cross-process scope/entity invalidation as a generic blocker.
Current scope ancestry, identity, grants, groups, assignments and governed
Configuration are read in ordinary request transactions. The cached Cedar
fragment is reused only when the full supplied scope shape matches. A second
notification system for those fresh inputs would add work without closing a
stale decision path. Compiled stored policy packs are the cross-request state
that still needs a measured refresh bound across replicas.

## Decision

Keep database authority and policy convergence as separate prerequisites to
HTTP admission. A process-local policy-ready marker records the exact open
authority generation after every active tenant's stored packs converge
successfully. `/readyz` and every application route require both an open
authority permit and that generation's policy marker. A newly opened
generation cannot inherit an earlier marker. The marker is set only after
the bounded initial convergence returns successfully while its captured
authority permit remains open. Normal request and audit enforcement is
unchanged.

This first slice closes the startup/recovery window. It does not claim that
subsequent policy-pack edits converge across replicas within a bound. OPS-7
must measure that path and make an old compile fail closed after an owned
staleness limit before allowing multiple gateways.

## Options considered

1. **Generation-bound policy readiness** — selected. It keeps liveness
   available during database outage and makes a restarted process wait for
   its own successful policy load before serving.
2. **Delay binding the HTTP socket** — rejected because it removes the
   existing liveness/readiness diagnostic during an unavailable database.
3. **Treat local entity flushes as the freshness protocol** — rejected:
   request inputs already come from PostgreSQL, while local flushes cannot
   load another process's compiled policy source.

## Consequences

- A gateway with an invalid or unavailable stored pack stays unready rather
  than briefly serving its embedded fallback. The existing strict convergence
  retry loop remains the recovery path.
- Authority recovery requires another successful convergence even if the
  process still holds a previous compile. This may add startup delay, but
  prevents a stale compile from crossing a database-authority generation.
- Policy edit/revocation visibility after startup, multi-process mutation
  tests, bounded refresh lag and worker alignment remain open OPS-7 work.

## Compliance notes

The marker only narrows when a request may enter the existing embedded Cedar,
forced-RLS, VedaFlow and content-free audit path. It cannot grant permission,
skip a decision, or expose tenant or pack contents in a response.
