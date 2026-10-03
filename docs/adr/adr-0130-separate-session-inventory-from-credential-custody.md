# ADR-0130: Separate session inventory from credential custody

- **Status**: Accepted
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
it. Revoking that one identifier does not stop a refresh token from minting a
different bearer; a session-family identifier is also needed for session
revocation. The current verified `Claims` type carries neither field.
[RFC 7519](https://www.rfc-editor.org/rfc/rfc7519#section-4.1.7) makes `jti`
optional. ADR-0110 allows one issuer per tenant; MEM-7 owns future
multi-issuer identity migration.

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
The first release offers self inventory and self revoke only. Administrator
inventory/revoke requires a separate Cedar action and reviewed disclosure
contract before it is enabled.

For identified OIDC bearers, carry verified issuer, per-token identifier,
session-family identifier when required, issued-at and expiry through the
existing verification seam. Store only domain-separated digests of tenant,
exact issuer, identifier kind and value, with database revocation time and a
bounded expiry, under forced RLS. The request path checks the applicable
token and family digests after signature and active-tenant verification but
before Cedar on every request. A database lookup failure rejects the request.
The refresh endpoint must verify its returned access token and refuse a
revoked family before returning it. No cache or configuration may bypass
enforcement once an issuer is promoted.

The first **candidate enforcement profile** is the deployment-owned,
single-tenant bundled Keycloak realm. It requires a signed, nonempty, at-most
256-byte `jti` for each access JWT; interactive tokens additionally require a
signed, nonempty, at-most 256-byte `sid` stable across refresh. Both are
case-sensitive opaque values, never subject or tenant identity. Require
numeric `iat` and `exp`, `0 < exp - iat <= 300` seconds, the existing exact
issuer/resource-audience checks, refresh-token rotation with zero reuse, an
eight-hour maximum SSO session and no offline-access token in this profile.
The bundled [Compose realm](../../deploy/compose/keycloak/synveda-realm-converge)
and [Helm realm](../../deploy/helm/synveda/templates/keycloak-realm.yaml)
currently configure those lifetimes and rotation; the local Keycloak
projection checks `jti` and `sid` token shapes. Revoked `jti` entries remain
until at least token expiry plus verifier leeway; revoked `sid` entries remain
for 12 hours from revocation, beyond the configured eight-hour SSO maximum.
The implementation must reject a missing/malformed claim or excessive token
lifetime when this profile is enforced. A fresh login with a new `sid` may
proceed after an old family was revoked.

This is a selected contract, **not yet a supported revocation claim**. Promote
it only after live bundled-Keycloak tests prove `sid` continuity through
refresh, used-refresh-token replay refusal, ordinary and service-token
`jti` shapes, logout and cross-replica denial. The acceptance target is denial
of every request within 30 seconds of a committed Synveda revoke while the
database is available; no such bound is currently qualified. The
[Keycloak administration guide](https://www.keycloak.org/docs/latest/server_admin/index.html)
documents the opt-in refresh-token rotation and token-lifespan controls.

Other OIDC issuers continue to authenticate under the existing contract but
have no Synveda bearer/session revocation promise. Promotion requires an
explicit issuer-specific claim mapping and live token/refresh evidence; never
guess `jti` from a differently named field. Microsoft's
[access-token reference](https://learn.microsoft.com/en-us/entra/identity-platform/access-token-claims-reference)
calls `uti` its per-token identifier while warning that applications should
not assume optional claims are present. Okta documents `jti` for its access
JWTs, but [rotation and reuse behavior](https://developer.okta.com/docs/guides/refresh-tokens/main/)
depends on client/server configuration. Neither provider is promoted by this
ADR.

Run bounded, observable cleanup of expired custody and revocation rows in the
singleton maintenance worker. Cleanup failure cannot make an expired session
valid, but must raise a backlog/error signal. The first release has no
incident bypass of enforced revocation. Quantitative revocation-row capacity
and cleanup lag must be established under EVAL-6 before promotion; a full
ledger may refuse a new revoke but must never report it as successful.

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
5. **Treat every OIDC `jti` as a universal revocation contract** — rejected:
   the claim is optional, provider naming differs, and a new token after
   refresh would escape single-token revocation.

## Consequences

- Self-session revocation is durable across gateways and never depends on a
  process cache; inventory remains a safe projection.
- Enforced bearer checks add one database read per request. Pre-index sessions
  are invisible until expiry. A profile with missing required identifiers is
  refused instead of quietly accepting an unrevocable token; unqualified
  issuers retain their existing authentication contract without a revocation
  claim.
- If measured lookup cost breaches the accepted envelope, revisit a bounded
  invalidation cache only with an equal or tighter measured revocation bound
  and fail-closed outage behavior.

## Compliance notes

The index and revocation rows carry no raw credentials. Tenant tables use
forced RLS and ordinary tenant transactions. Inventory/revoke uses explicit
Cedar actions and content-free audit, without conferring authority from the
inventory projection. Refresh remains provider-owned. This ADR makes no claim
about an external IdP's rotation or revocation without live evidence. The
Keycloak profile remains a candidate until its complete acceptance passes.
