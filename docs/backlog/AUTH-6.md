---
title: "AUTH-6: Session & token hygiene"
labels:
  - epic:AUTH
  - phase:unscheduled
size: S
---

# AUTH-6: Session & token hygiene

**Epic:** AUTH — Authentication & identity (functional requirement) · **Phase:** unscheduled · **Size:** S

## Problem and evidence

Console custody, bounded custody cleanup, CLI refresh and immediate identity
disable predate this work. On 2026-10-24 the source tree gained the
[ADR-0130 amendment 1](../adr/adr-0130-separate-session-inventory-from-credential-custody.md)
boundary. It is source-only and in no published release.

- **Schema.** Forward migration `0013`, byte-identical in both lineages, adds
  the forced-RLS `console_session_index` (subject-keyed, cascading from
  custody) and `token_revocations` (digest, kind, database times, at most 12
  hours). The head is `0013`. Catalogue pins were measured after reproducing
  the 0012 pins exactly: ACL `e497349f…`/429 rows, forced RLS `17bcceb4…`/136,
  routines and triggers unchanged. PostgreSQL 14, 17 and 18 report identical
  values.
- **API.** `GET /v1/me/console-sessions` (paged, public id, issuer and coarse
  times only), `DELETE /v1/me/console-sessions/{id}` (one idempotent 204),
  `POST /v1/me/console-sessions/revoke-all` (four 64-row batches per request,
  `{revoked, complete}`) and `POST /v1/me/credential/revoke` (`token` or
  `family` of the presented bearer). They use the tenant-only
  `LoginSessionRead`/`LoginSessionRevoke` Cedar actions and content-free
  `auth.login_session.revoked`/`auth.credential.revoked` audit. OpenAPI, the
  console client and the SDK contract digest are regenerated.
- **Clients.** `synveda auth sessions|revoke-session|revoke-all-sessions` and
  `synveda auth logout --revoke token|family`, plus a Settings ▸ Signed-in
  sessions panel in the console.
- **Enforcement.** An issuer opts in with `"revocation": {"profile":
  "bundled_keycloak", "mode": "observe"|"enforce"}`. Profile checks and the
  ledger read run after tenant resolution on every request, including
  cookie-authenticated ones. Refresh and console renewal verify the new
  bearer first. A login into a revoked family is refused as `session_revoked`.
  A ledger at its 65,536 live-row cap refuses with 429 and changes nothing.
- **Cleanup.** The maintenance worker gives every active tenant one purge
  batch per minute plus 16 shared catch-up batches. It exports sweep, purged,
  budget-hit, oldest-expired-age and fullest-tenant gauges.

Evidence, all from this Windows host:

- `cargo test -p synveda-gateway --test session_revocation` passed 12, with 1
  measurement case ignored, against a fresh PostgreSQL 17.11 database through
  the ordinary gateway role. It covers wrong tenant and principal, guessed ids,
  revoke-one and revoke-all progress, duplicate and expired-session requests,
  custody/index atomicity and absent sensitive fields. It also covers the full
  ledger, profile refusals versus `observe`, concurrent refresh replay and
  revoke, two router instances plus a rebuilt one, a held table lock (503,
  not 401) and the sweep.
- The existing `oidc_login` (19), `console_session` (13), `cli_login` (11),
  `tenant_resolution` (9), `service_identities` (8), `audit_events` (3) and
  `openapi` (7) suites passed, as did gateway lib (137) and the store `epoch`
  suite (50), which advances 0012 to 0013 on both lineages.
- Cedar `service_scope` (5) proves service tokens are denied both actions.
- The [isolated live probe](../../demos/auth-6-keycloak-revocation-probe.mjs)
  produced the [retained record](../../demos/evidence/auth6-keycloak-revocation-probe.json).
  It used Keycloak 26.7.2 from the bundled image in dev mode, the realm
  rendered from the shipped Helm template and the real `synveda` client:
  - access tokens carry bounded `jti` and `sid` with a 300-second lifetime;
  - `sid` is stable through refresh while `jti` changes and the refresh token
    rotates;
  - reuse of a rotated refresh token returns `invalid_grant` and also kills
    the legitimate successor;
  - a second login in the same browser shares the `sid`;
  - `offline_access` is refused;
  - issuer logout ends refresh, and a fresh login gets a new `sid`;
  - a client-credentials token has a `jti`, no `sid` and no refresh token.
- Through two native source gateways sharing that database, a revoked bearer
  and family were refused on the first request after commit (4–12 ms) for the
  console cookie, the public API on the other gateway and the native CLI.
  Gateway refresh returned 401, a re-login into the same issuer session
  returned `session_revoked`, a fresh issuer session was admitted and a sibling
  of a single revoked bearer survived.
- Measured on a debug build with Docker Desktop loopback, sequential
  in-process `whoami`, 4,000 samples per mode over two runs: the lookup adds
  four statements per request. p50 went from 3.10 to 5.47–5.53 ms, p95 from
  3.35 to 5.82–5.95 ms and p99 from 3.50–3.52 to 6.23–6.36 ms. Issuers without
  a profile add nothing. The roughly 2.4 ms is mostly Docker Desktop round
  trips; this is not a production latency or capacity figure.
- Environment-only local failures: the policy approval golden fails on CRLF
  bytes. Two CLI unit tests, in `database_preflight` and `settings` (files this
  work did not touch), fail on native Windows path handling; whether they also
  fail on the base commit was not checked. `make db-test`, the hosted
  `Database acceptance — workspace` job and Linux CI were not run.

## Scope

- Inventory active Synveda console sessions for the current principal without
  exposing bearer or refresh material; defer administrator inventory/revoke.
- Revoke one or all sessions and audit the action.
- Persist bounded revocation evidence for verifiable issuer/token identifiers
  and enforce it in the existing credential-verification path using database
  time.
- Exercise refresh rotation/replay semantics offered by each supported IdP.
- Preserve immediate fail-closed identity/service disable.

## Non-goals

- No general API-key product.
- No storage of raw bearer tokens or unbounded revocation rows.
- No device binding until the security owner defines supported devices,
  recovery and privacy requirements.
- No claim that Synveda can impose refresh semantics an external IdP does not
  expose.

## Architecture seam

Token signature/claim verification remains in synveda-identity; tenant and
credential resolution in the gateway adds a bounded revocation lookup before
authority is used. Console-session rows remain the browser session source.
Revocation mutations use the PDP, tenant transactions and content-free audit;
cleanup is TTL-bounded and safe across replicas.

## Acceptance criteria

- A revoked console session and a token in the enforced bundled-Keycloak
  profile fail every public API within 30 seconds of commit under a healthy
  database; no external issuer inherits this claim.
- Reusing a rotated refresh credential fails where the IdP contract supports
  rotation, and the result is distinguishable from transient provider outage.
- Session inventory reveals only safe device/client/time metadata and never
  token material or another tenant's counts.
- Revoke-one, revoke-all, expiry and identity disable are idempotent, audited
  and use database time.
- Multi-replica and restart tests preserve revocation and one-time semantics.
- Revocation storage has explicit TTL/cardinality bounds and observable purge
  failure.

## Required tests

- Console, CLI, service-token and directory-identity revocation cases.
- Cross-tenant/unauthorised session inventory probes.
- Clock-skew, missing token identifier, expired row and provider-outage cases.
- Concurrent refresh/revoke/replay and multi-replica tests.
- Live tests for every IdP whose rotation/revocation behaviour is claimed.

## Rollout and rollback

Ship inventory and observe-only revocation telemetry before enforcement, then
enable by issuer. Keep existing session expiry as the safe fallback, but do
not disable enforcement after claiming the bound except under an audited
incident procedure. Schema additions remain readable if an issuer is rolled
back to observe-only.

## Dependencies

The first candidate claim/lifetime/session contract and self-only scope are
selected in ADR-0130. EVAL-6 must establish revocation-row capacity and
cleanup lag. OPS-7 supplies multi-replica acceptance; external issuer
promotion requires its own live tenant, credentials and claim mapping.

Next, before promotion:

1. Push for hosted CI, including `Database acceptance — workspace`. Pushing
   needs owner approval.
2. Run the probe against the production-mode bundled stack: the Compose
   converge path and published images on a separate supported host, or after
   the `synveda-development-acceptance-interop` owner hands off the hostnames.
3. Measure denial against the 30-second target under the load and latency
   suite, together with ledger capacity and cleanup lag.
4. The owner decides whether bundled issuer documents ship
   `"mode": "observe"` and when `enforce` becomes the bundled default.

Multi-host replicas need the replica acceptance. External issuers need their
own profile and live evidence.
