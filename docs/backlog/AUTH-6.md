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

## Scope

- Inventory active Synveda console sessions for the current principal and
  authorised administrators without exposing bearer or refresh material.
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

- A revoked console session and revocable user/service token fail every public
  API within 30 seconds or a stricter owner-approved bound.
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

Identity/security owners must choose the revocation bound, supported issuer
claims, administrator visibility, device-binding scope and incident override.
OPS-7 supplies multi-replica acceptance; live Entra/Okta evidence requires
external tenants and credentials.
