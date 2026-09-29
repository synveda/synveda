# ADR-0124: Withdraw gateway admission before draining HTTP

- **Status**: Accepted
- **Date**: 2026-09-29
- **Feature(s)**: OPS-7
- **Deciders**: Synveda maintainers

## Context

The authority gate closes on a database or runtime-role refusal and revokes
requests already using that authority. Process termination is different:
the gateway must stop taking new application work while allowing requests
admitted under still-valid authority to finish. Today SIGTERM sends one stop
signal to the HTTP server, authority sentinel and background tasks at once.
`/readyz` can still say ready until the listener closes, and ending the
sentinel can revoke an otherwise valid in-flight request. Helm probes every
five seconds and gives the gateway a ten-second outer margin beyond its
configured shutdown time. OPS-7 cannot claim a drained rolling upgrade on
that sequence.

This is a single-replica lifecycle correction. Durable OIDC state,
cross-process invalidation and multi-worker lease proof remain separate OPS-7
requirements before the chart may accept another replica.

## Decision

Add a one-way process admission signal separate from database authority. On
SIGTERM/SIGINT or another supervisor exit, withdraw readiness and refuse new
application requests with 503. Already-admitted requests continue under the
normal live authority permit. On an ordinary signal, allow up to ten seconds
for a deployment readiness probe to remove the endpoint, capped so at least
one second of cooperative HTTP cleanup and one second of forced-join reserve
remain inside the configured gateway shutdown bound. Then stop accepting HTTP
connections, await the HTTP server and its in-flight requests, stop the
authority/background supervisors, close the database pool and flush telemetry.
An authority refusal still revokes in-flight work immediately.

Set Helm's gateway readiness failure threshold to one: a locally withdrawn
gate or failed authority proof must remove the pod on the next five-second
probe. Keep the chart at one gateway replica with `Recreate`. Give Compose
the same ten-second outer stop margin as Helm. A fixed, bounded window is
adequate for this first deployment seam; multi-replica acceptance must test
the actual termination sequence and revise the interval if measured probe
and request-drain evidence requires it.

## Options considered

1. **Separate admission from authority** — selected. It preserves the
   authority gate's security semantics and gives termination a one-way,
   process-local route for refusing new work without canceling old work.
2. **Close the authority gate on SIGTERM** — rejected because it revokes
   requests that should complete during drain.
3. **Stop the listener immediately** — rejected because `/readyz` cannot be
   observed failing before traffic stops and an ingress may still route new
   connections while its old endpoint state remains ready.

## Consequences

- A normal default shutdown may spend ten of its thirty seconds withdrawing
  from routing before HTTP drain. Shorter configured bounds reduce that
  window; they do not imply a qualified Kubernetes drain.
- A failed or stuck HTTP request can consume the remaining cooperative bound;
  the forced-join reserve still cancels the supervisor and the process reports
  the hard deadline.
- The phase remains OPS-7 open until real in-flight, pod-loss, cross-replica
  and rolling-upgrade acceptance passes. If production probe timing or load
  makes ten seconds insufficient, revise the window and outer grace from
  measured evidence rather than silently claiming availability.

## Compliance notes

The admission signal has no permission semantics. Every admitted request
still uses the embedded Cedar PDP, ordinary forced-RLS transactions and
content-free audit; a database-authority refusal continues to fail closed.
The 503 body is generic and discloses no tenant or login state.
