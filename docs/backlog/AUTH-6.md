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

Console sessions are sealed in Postgres with a bounded lifetime, CLI refresh
exists, service tokens have bounded issued/expiry times and identity disable
withdraws authority. Synveda has no general access-token revocation list,
session inventory/revoke API or tested revocation-within-bound guarantee;
refresh rotation remains provider-dependent. This P1 gap is recorded in
[production readiness](../PRODUCTION_READINESS.md).

The 2026-09-30 senior Rust/architecture pass found that expired console
credential rows had no cleanup caller. The read path already refused them,
but retained ciphertext could accumulate. It also confirmed that ADR-0056
intentionally keeps `console_sessions` without tenant/subject columns and
re-verifies the bearer on every request. A broad
inventory scan or treating that row as identity would violate this contract.
[ADR-0130](../adr/adr-0130-separate-session-inventory-from-credential-custody.md)
selects a separate tenant-scoped, credential-free index and request-time
identifier revocation. The combined maintenance worker now deletes at most 256
expired custody rows per minute with skip-locked coordination across replicas;
successful/failed sweeps and removed rows have separate content-free counters.
No AUTH-6 revocation enforcement or inventory is implemented yet.
The [2026-09-30 Rust review](../RUST_ARCHITECTURE_REVIEW_2026-09-30.md)
also found that one 256-row purge batch per minute lacked retention-lag
evidence. Each successful sweep now observes the indexed oldest expired age
in a deployment-wide gauge (zero when no expired row remains); a failed sweep
leaves the last observation and increments the error counter. The fixed
service rate still has no proven arrival envelope or retention bound. Measure
age under representative load and use a bounded drain budget or a proven
admission envelope before claiming bounded credential retention.

The first candidate issuer contract is now the exact bundled Keycloak realm:
verified access-token `jti` for one bearer, `sid` for an interactive session
family, numeric `iat`/`exp` with at most five minutes of access lifetime,
rotating refresh tokens with zero reuse, no offline access, and an eight-hour
maximum SSO session. Revocation evidence for a family remains for 12 hours.
The OIDC verifier now carries the exact configured issuer, bounded optional
`jti`/`sid`, and numeric token times only after signature and audience checks.
A signed mock-issuer fixture covers changed per-token ID with stable family,
malformed optional identifiers, redacted debug output and signature tampering.
It does not prove a Keycloak refresh or enable revocation.
The first product slice is self-only inventory/revoke; administrator access
remains a separately reviewed authority/disclosure change. Entra, Okta and
other external issuers retain login support but have no revocation promise
until their exact token and refresh contracts pass live acceptance. None of
the Keycloak revocation behavior is implemented or qualified yet.

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

Next: prove the signed `jti`/`sid` and refresh rotation/replay contract with
the bundled Synveda Keycloak realm, then append a forward migration
for the tenant index/revocation ledger, wire atomic console-session creation,
and add Cedar-governed self inventory/revoke. Promote bearer-family enforcement
only after the complete Keycloak and cross-replica acceptance passes; missing
required claims must fail closed in that profile. Measure the 30-second target
and ledger capacity before advertising revocation.
