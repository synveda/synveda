# ADR-0130: Separate session inventory from credential custody

- **Status**: Proposed
- **Date**: 2026-09-30
- **Feature(s)**: AUTH-6
- **Deciders**: Synveda maintainers and security owner

## Context

ADR-0056 keeps `console_sessions` deployment-scoped and without a tenant or
subject. A cookie selects a sealed bearer by its random secret hash; the
gateway verifies that bearer before it knows the tenant. Adding tenant
authority to that row would weaken the invariant. Scanning and opening every
token to discover its owner would make inventory unbounded credential work.
The existing 12-hour cap expires authority. The first AUTH-6 slice adds
bounded worker cleanup of expired custody rows; it does not provide inventory
or identifier revocation.

A signature and expiry check cannot discover IdP-side access-token revocation.
Only a verified issuer/token identifier can name one bearer without storing
it. The current verified `Claims` type carries neither field. ADR-0110 allows
one issuer per tenant; MEM-7 owns future multi-issuer identity migration.

## Decision

Keep `console_sessions` as pre-tenant credential custody, without tenant or
subject columns. Add a separate forced-RLS, tenant-scoped inventory keyed by a
random public session ID, with verified subject and an internal secret-hash
reference. Create custody and inventory atomically after callback verification
and tenant admission. Inventory never answers authentication. Existing
unindexed sessions remain valid until their 12-hour cap but are not listed;
do not backfill by decrypting credentials.

Self inventory exposes only public ID, configured issuer label, creation,
coarse last-use and hard-expiry times. It never returns cookie/hash, bearer,
refresh token, IP address, user-agent string or another tenant's counts. Self
revoke selects the indexed row under tenant RLS, asks a dedicated Cedar action,
deletes custody atomically and retains content-free audit. Absent and already
revoked IDs have one idempotent public result. Revoke-all uses bounded batches.
Administrator visibility and authority need an owner-approved policy choice.

For identified OIDC bearers, carry verified issuer, identifier and expiry
through the existing verification seam. Store only a digest of the exact
issuer and identifier with tenant, database revocation time and token expiry,
under forced RLS. Check it after signature and active-tenant verification but
before Cedar on every request. A database lookup failure rejects the request.
Omitted/unsupported identifiers are not silently considered revocable. No
issuer earns a 30-second revocation claim until its identifier, lifetime and
refresh-rotation contract pass live tests. Use database time and bounded
cleanup; observe-only telemetry precedes issuer-specific enforcement. No
cache or configuration may bypass enforcement once an issuer is promoted.

Run bounded, observable cleanup of expired custody and revocation rows in the
singleton maintenance worker. Cleanup failure cannot make an expired session
valid, but must raise a backlog/error signal. The security owner must still
select supported issuer identifiers, maximum token lifetime/cardinality,
administrator visibility and incident procedure before enforcement promotion.

## Options considered

1. **Separate tenant inventory and deployment credential custody** — selected:
   pre-tenant lookup cannot assert tenant authority; inventory stays bounded
   and RLS-backed.
2. **Add tenant and subject to `console_sessions`** — rejected: this reverses
   ADR-0056's credential-only schema property.
3. **Scan/decrypt every console token for inventory** — rejected: cost and
   credential handling scale with every deployment's sessions.
4. **External session or revocation cache** — rejected for the first offer:
   it adds a consistency, backup and failover plane.

## Consequences

- Self-session revocation is durable across gateways and never depends on a
  process cache; inventory remains a safe projection.
- Enforced bearer checks add one database read per request. Pre-index sessions
  are invisible until expiry; tokens without a supported identifier cannot be
  individually revoked.
- If measured lookup cost breaches the accepted envelope, revisit a bounded
  invalidation cache only with an equal or tighter measured revocation bound
  and fail-closed outage behavior.

## Compliance notes

The index and revocation rows carry no raw credentials. Tenant tables use
forced RLS and ordinary tenant transactions. Inventory/revoke uses explicit
Cedar actions and content-free audit, without conferring authority from the
inventory projection. Refresh remains provider-owned. This ADR makes no claim
about an external IdP's rotation or revocation without live evidence.
