# ADR-0008: Tenant resolution from token claims, propagated by task-local context

- **Status**: Accepted
- **Date**: 2026-07-18
- **Feature(s)**: TEN-1
- **Deciders**: sujitn

## Context

TEN-1 opens Phase 1: every request entering the gateway must resolve to
exactly one tenant before anything else happens, because tenancy is the root
isolation boundary (seed §4.1) and the middleware chain is fixed as
AuthN → tenant resolution → PDP → rate limits → audit (seed §7). The
acceptance criteria: a request without a resolvable tenant is rejected 401,
and traces carry the tenant id.

Forces at play:

- **AUTH-1 (OIDC against real IdPs) does not exist yet.** TEN-1 still needs
  real per-request resolution "from token claims", testable and demoable
  today, without shipping any unauthenticated path.
- The feature text mandates propagation "via tower middleware + task-local",
  so lower layers can see the tenant without threading a parameter through
  every signature.
- The crate layering rule (seed §8) forbids sibling imports: `store` cannot
  see `identity`, so wherever the context type lives decides who can read it.
- Fail-closed is non-negotiable (seed §2.3): a misconfigured gateway must
  reject, never admit.

## Decision

1. **Claims contract.** The bearer token is a JWT whose `sub` claim names the
   subject and whose `tid` claim (Entra ID's convention) carries the tenant
   UUID. `exp` is mandatory. AUTH-1 will generalise claim mapping per IdP;
   `tid`/`sub` stay the internal shape.
2. **Resolution.** A token resolves iff `tid` matches a row in the new
   `tenants` table with `status = 'active'`. Missing header, malformed or
   expired token, unknown tenant, and suspended tenant are all the same
   uniform `401 unauthenticated` — the gateway is not an existence oracle
   (same doctrine as `Error::NotFound`).
3. **Verification seam.** `synveda-identity` owns a `TokenVerifier` trait.
   Until AUTH-1, the only real implementation is `Hs256Verifier`: HMAC-SHA256
   (RustCrypto `hmac`/`sha2`, constant-time verify), algorithm pinned to
   HS256 — the token header's `alg` is checked, never trusted to select a
   scheme — with a shared secret from `SYNVEDA_DEV_JWT_SECRET`. It can also
   `issue()` tokens for dev/test/demo. When the secret is unset the gateway
   installs `DisabledVerifier`, which rejects every token: fail closed, ops
   plane (`/healthz`, `/readyz`, `/metrics`) unaffected.
4. **Propagation.** `synveda-identity` owns `TenantContext` (resolved
   `Tenant` + subject) and a tokio task-local; the gateway middleware wraps
   the rest of the stack in `with_tenant(...)`, and anything above the
   middle tier reads `current_tenant()`. `store` stays below the seam and
   keeps taking `TenantId` as an explicit argument (as `records` already
   does) — TEN-2's RLS GUC will be set from that explicit value.
5. **Traces & metrics.** The request span declares an empty `tenant.id`
   field, recorded on successful resolution (AC), plus a
   `synveda_tenant_resolutions_total{outcome}` counter
   (resolved/rejected/error).
6. **Tenant table.** Plain (non-bitemporal) `tenants` row: id, unique slug,
   name, status ∈ {active, suspended}, created_at. Lifecycle transitions,
   export, and destruction are TEN-5; RLS keyed on the tenant is TEN-2; no
   FK from `records` yet — referential and row-level enforcement land there.
7. **CLI tier expansion.** `synveda-cli` gains its first real commands
   (`db migrate`, `tenant create`, `token issue`) for dev bootstrap and the
   demo, so it now depends on `synveda-store` and `synveda-identity`. The
   layering script's comment calls this out as a deliberate, reviewed
   decision — this ADR is that review. The CLI remains a leaf binary;
   nothing depends on it.

## Options considered

1. **HS256 dev verifier behind a trait (chosen)** — real signature
   verification today, one env var, zero new services; swapped for OIDC/JWKS
   by AUTH-1 behind the same trait. Con: a shared symmetric secret is not
   multi-party auth — acceptable strictly as the pre-AUTH-1 dev mode, and
   fail-closed when unconfigured.
2. **Parse claims without verification until AUTH-1** — no secret to manage,
   but ships an unauthenticated path through the gateway; violates
   fail-closed. Rejected outright.
3. **Wire Rauthy OIDC now** — no interim mode at all, but drags JWKS
   caching, rotation, and discovery (the whole of AUTH-1) into TEN-1's
   scope. Rejected as scope creep; Rauthy is already running in compose
   waiting for AUTH-1.
4. **Context in `synveda-types` instead of `identity`** — would let `store`
   read the task-local too, but puts a tokio runtime dependency into the
   zero-dep domain-types crate, and implicit ambient tenancy inside storage
   is exactly what TEN-2 wants to avoid: store stays explicit.
5. **Request extensions instead of task-local** — axum-idiomatic, but only
   reachable from extractors; the feature text mandates task-local, which
   also serves layers that never see the `Request`. One mechanism, not two.

## Consequences

- Positive: the AuthN → tenant seam exists exactly where seed §7 draws it;
  AUTH-1 replaces one trait impl and touches nothing else; every Phase 1
  feature behind `/v1` inherits resolution for free; demos can mint tokens
  offline.
- Negative / accepted trade-offs: a dev-mode symmetric secret exists until
  AUTH-1 (mitigated: fail-closed default, never set in shipped config);
  suspended tenants get 401 rather than a distinct status (revisit in TEN-5
  if operators need a differentiated signal).
- Reversal trigger: AUTH-1 supersedes the HS256 mode (the `Hs256Verifier`
  then becomes test-only); if task-local propagation proves hard to reason
  about under Temporal workers (MEM-3), fall back to explicit context
  parameters above the store seam.

## Compliance notes

Resolution decisions (allow and reject) are audit-relevant events. The
hash-chained audit log is AUD-1 and does not exist yet; until it lands,
rejections are visible in traces/metrics only. AUD-1 must wire this
middleware as an emission point — tracked in STATUS.md. No PDP bypass is
introduced: `/v1/whoami` returns only the caller's own resolution result and
reads no governed assets; the three primitives will sit behind the PDP
(AUTHZ-1) on this same middleware chain.
