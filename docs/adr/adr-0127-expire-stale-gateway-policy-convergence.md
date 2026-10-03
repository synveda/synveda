# ADR-0127: Expire stale gateway policy-pack convergence

- **Status**: Accepted
- **Date**: 2026-09-30
- **Feature(s)**: OPS-7
- **Deciders**: Synveda maintainers

## Context

ADR-0125 gates first traffic on a successful stored-policy load, but the
periodic refresher can fail indefinitely while its last-good Cedar compile
continues to serve. A policy edit on another gateway may be more restrictive.
The five-second default poll is not a freshness guarantee: database errors,
compile errors, slow sweeps and scheduler delay can all extend it. The chart
still admits one gateway, and an owner has not qualified a production policy
latency SLO or a multi-pod load envelope.

## Decision

Give each gateway authority generation a monotonic, process-local policy
freshness lease. The source candidate uses a provisional 30-second maximum
age, independent of the configurable 1–15-second poll interval. Initial convergence and
only a complete, successful refresh sweep renew the lease. A failed tenant or
pack, a database error, or a sweep exceeding the existing five-second
authority-check deadline does not renew it. Expiry withdraws readiness,
refuses new application traffic and cancels an in-flight application future;
the refresher keeps retrying and a later complete success may reopen the same
generation. Database-authority generation changes still require a fresh full
convergence. Measure full-sweep duration and outcome without tenant/pack
labels. Retain one-replica deployment until cross-process mutation latency,
worker policy freshness and interruption acceptance are proved.

## Options considered

1. **Bounded polling lease** — selected. It keeps the embedded Cedar decision
   path local and makes prolonged refresh failure visible and fail closed.
2. **Treat every timer tick as fresh** — rejected: a failed load would keep
   an old compile eligible without limit.
3. **Introduce LISTEN/NOTIFY now** — deferred until measured polling fails the
   owner-selected limit; notifications need their own reconnect and missed-
   event recovery protocol.

## Consequences

- A gateway may become unavailable after 30 seconds without a complete sweep,
  even when PostgreSQL itself remains reachable. A successful retry restores
  admission. A long configured poll interval cannot extend the safety bound.
- This bounds use of a last-good compile after refresh failure in this source
  candidate. It does not prove that a policy edit reaches every replica in 30
  seconds under production load, nor make the separately supervised worker
  fail closed after the same age. Those remain OPS-7 acceptance work.
- An application request canceled at expiry can have an ambiguous client
  outcome if its durable effect committed just before cancellation; existing
  idempotency and operation semantics remain necessary.

## Compliance notes

The lease only narrows when the existing embedded Cedar, forced-RLS and audit
paths may run. It is not a policy override or a second source of authority.
The sweep metric carries a closed outcome and duration, no policy source or
tenant identifier.
