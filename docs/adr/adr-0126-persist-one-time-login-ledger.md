# ADR-0126: Persist one-time login and CLI handoff state

- **Status**: Accepted
- **Date**: 2026-09-29
- **Feature(s)**: OPS-7
- **Deciders**: Synveda maintainers

## Context

`LoginFlow` currently parks OIDC state, PKCE verifiers, nonces, browser
correlation and CLI handoff credentials in two process-local maps. A callback
or code redemption routed to another gateway cannot complete, and a process
restart loses every pending login. Helm therefore refuses more than one
gateway. The state exists before an ID token identifies a tenant; it cannot
be placed inside a tenant RLS transaction. It contains credentials and must
not be stored in plaintext or logged. The published epoch-3 `0001` and
source `0002` migrations are immutable.

## Decision

Keep protocol construction, verification and destination validation in
`synveda-identity`, but replace production maps with an async, narrow
one-time-state interface. The gateway implements it using `synveda-store`
SQLx-checked operations on two deployment-scoped tables added by an append-only
`0003` migration. Store only SHA-256 hashes of random OIDC state and CLI codes;
seal pending protocol material and completed handoff payloads with the
deployment key, purpose-separated and bound to each row hash. Require an
available deployment key for an OIDC login to begin. Use PostgreSQL time for
the existing 10-minute pending and 60-second handoff limits, atomically delete
on consume, and serialize bounded insertion/cap enforcement in the database.
Wrong browser correlation cannot consume a pending console login; a handoff
code is consumed on the first redemption attempt, including a wrong CLI state.

The tables have no tenant key and no tenant RLS because they are accessed
before tenant admission, as `console_sessions` already is. Grant only the
ordinary application role the necessary operations. Keep the existing
issuer, active-tenant, Cedar, forced-RLS, provisioning and audit paths after
token verification unchanged. The in-memory ledger is test-only and cannot
be selected in a production build.

## Options considered

1. **Deployment-scoped PostgreSQL ledger** — selected. Atomic one-time
   consumption works across gateway processes and database recovery preserves
   unexpired states when the deployment key is also restored.
2. **Sticky routing to process-local maps** — rejected: pod loss discards the
   login and affinity makes a replica count look available without cross-pod
   completion.
3. **Redis or another shared store** — rejected for this slice: it adds a
   separate authority and backup plane to a Postgres-first deployment.

## Consequences

- All OIDC login modes now depend on the deployment key and database. A
  missing or wrong key fails closed; operators must restore the matching key
  with the database. Anonymous login attempts gain bounded database work.
- The appended migration advances the required schema head; older binaries
  must refuse it. Rollback uses a verified prior database/key recovery set,
  not an in-place down migration.
- This supplies the durable login prerequisite only. Three-pod routing,
  post-start policy freshness, worker ownership and load acceptance remain
  OPS-7 gates before lifting the one-replica chart refusal.

## Compliance notes

The pre-tenant rows contain sealed protocol/session payloads and hash-only
selectors. They do not grant application authority or bypass Cedar, tenant
RLS, VedaFlow or content-free audit. Their creation and redemption expose
bounded outcome metrics without code, state, token or payload labels. A
recovered database without its deployment key cannot complete a parked flow.
