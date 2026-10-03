# ADR-0128: Stop worker execution when policy convergence expires

- **Status**: Accepted
- **Date**: 2026-09-30
- **Feature(s)**: OPS-7
- **Deciders**: Synveda maintainers

## Context

ADR-0127 expires a gateway's policy-ready generation after a provisional
30 seconds without a complete successful sweep. The core worker and optional
Apalis transport leaf also compile stored Cedar packs locally, but today they
can continue using a last-good compile after every later refresh fails. Worker
readiness alone cannot stop a claimed unit already executing under that
compile. Product leases and attempt fences survive cancellation, but
multi-worker interruption and provider effects still need deployment tests.

## Decision

Use the same process-local, monotonic policy-convergence lease for the gateway,
core worker and Apalis leaf. All worker admission and execution checks require
the exact authority generation that loaded the packs, so a reopened database
gate cannot reuse an older compile. All three pass that generation to the
refresher; only a complete successful sweep renews the lease. Core
worker readiness requires the lease. At expiry the supervisor withdraws
readiness, cancels its governed task futures, joins them within its existing
bounded shutdown reserve and retries initial convergence before accepting new
work. The optional Apalis leaf refuses new dispatch and task execution when
the lease expires, cancels in-flight governed futures, withdraws readiness and
exits nonzero so its next process must converge before serving again. Keep the
30-second limit provisional and keep all replica limits unchanged.

## Options considered

1. **Shared lease with supervised cancellation** — selected. The worker uses
   the same freshness rule as HTTP, and cancellation respects existing durable
   claim fences.
2. **Readiness-only expiry** — rejected because an already claimed job can
   continue a provider call under stale policy.
3. **Treat last-good compile as indefinitely valid** — rejected because a
   restrictive stored-policy edit may never reach the worker.

## Consequences

- A prolonged refresh failure can interrupt work even while PostgreSQL is
  otherwise available. Claims recover through their existing leases and
  idempotent operation fences; the interrupted provider outcome can remain
  ambiguous and needs separate acceptance.
- A successful core-worker restart of its generation can restore readiness
  without restarting the process. The experimental Apalis process takes the
  simpler fail-closed exit path.
- This source behavior does not establish a loaded multi-pod mutation latency
  bound, multi-worker ownership, or safe external provider effects. Those
  remain OPS-7 and capture reliability acceptance work.

## Compliance notes

The lease narrows when the embedded PDP, forced-RLS transactions and governed
mutations may run. It grants no authority. Cancellation adds no content to
telemetry or audit and does not bypass existing claim or evidence checks.
