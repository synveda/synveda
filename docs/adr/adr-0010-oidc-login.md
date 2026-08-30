# ADR-0010: OIDC login — JWKS verification and the code+PKCE flow

- **Status**: Accepted
- **Date**: 2026-07-18
- **Feature(s)**: AUTH-1
- **Deciders**: sujitn

## Context

AUTH-1 replaces the pre-AUTH-1 HS256 dev verifier (ADR-0008) with real OIDC:
any conforming IdP, a bundled reference provider, and JWKS caching with
rotation handling. CPR-45 replaces the original Rauthy fixture with Keycloak;
the application boundary remains provider-neutral. The AC is one real bundled
flow plus the same deterministic mock-Entra contract.

Forces at play:

- ADR-0008 fixed the seam: AUTH-1 is a new `TokenVerifier` implementation;
  the tenant-resolution middleware and everything behind it must not change.
- ADR-0008 also fixed the internal claims shape (`sub`, `tid`, `exp`) and
  promised AUTH-1 would "generalise claim mapping per IdP".
- Synveda is an OIDC *client*, never the source of truth for identity
  (tech plan §1.2). There is no password store and never will be.
- Licence allowlist (deny.toml): MIT/Apache-2.0/PostgreSQL. `ring`
  ("ISC AND MIT AND OpenSSL") is not admissible, which constrains the JWT
  and TLS dependency choices.
- Fail-closed is non-negotiable (seed §2.3); the gateway must never admit a
  token it cannot fully verify.

## Decision

1. **What "a Synveda session" is.** A verified bearer context accepted on
   `/v1`, exactly as TEN-1 defined it: token verifies, tenant resolves
   active, `/v1/whoami` answers. The gateway stays stateless — no session
   table, no cookies. Refresh rotation and revocation are AUTH-6; the CLI
   login UX is ADPT-1. AUTH-1's session artifact is the IdP-issued access
   token, verified per request via JWKS.
2. **`OidcVerifier` in `synveda-identity`**, implementing `TokenVerifier`.
   The trait's `verify` becomes `async` (via `async-trait`, keeping
   `Arc<dyn TokenVerifier>` object-safe): verification may need to fetch
   discovery or keys; the HS256 dev verifier is unchanged behind the async
   signature. Trust is configured per issuer: issuer URL, client id, a
   mandatory bearer audience distinct from the login client id and every
   service audience, allowed signature algorithms (default
   `["RS256"]`), and a tenant binding. Dispatch reads the *unverified*
   `iss` claim only to select the trust entry; every other claim is
   consulted only after signature verification under that entry's keys.
   Unknown issuer, unknown `kid`, algorithm outside the allowlist, or
   algorithm/key-type mismatch are all the uniform 401.
3. **Discovery and JWKS cache.** Per issuer, lazily on first use: fetch
   `{issuer}/.well-known/openid-configuration`, require the document's
   `issuer` to equal the configured value byte-for-byte, then fetch
   `jwks_uri`. Keys are cached by `kid` and replaced wholesale on refetch.
   Rotation handling: a token with an unknown `kid` triggers a refetch,
   rate-limited to one per 30 seconds per issuer, so key rotation heals on
   the next request without letting an attacker drive fetch load.
4. **Tenant binding per issuer** (the ADR-0008 "generalise" step), two
   modes: `claim` — a named claim (default `tid`, Entra's native shape)
   carries the tenant UUID; or `static` — every subject from this issuer
   belongs to one configured tenant, the natural shape for a single-org
   IdP like the dev Rauthy. Both modes end at TEN-1's unchanged
   active-tenant lookup; the middleware and store see no difference.
5. **Login flow endpoints** on a new unauthenticated auth plane:
   `GET /auth/login` (302 to the IdP with code+PKCE S256 challenge, `state`,
   `nonce`; `?issuer=` selects among multiple configured issuers, optional
   when only one) and `GET /auth/callback` (state lookup, code exchange
   with the `code_verifier`, ID-token verification including `nonce`,
   tenant resolution, JSON response carrying subject, tenant, and the
   access token). Pending logins live in a bounded in-memory store with a
   10-minute TTL — single-replica only, acceptable until the enterprise
   profile (OPS-2) makes login-state affinity a deployment concern.
6. **Dependencies.** `jsonwebtoken` v10 with the `rust_crypto` backend
   (pure-Rust RSA, no `ring` — licence-clean) for JWT + JWK verification;
   the accepted algorithm vocabulary is exactly RS256/RS384/RS512.
   `reqwest` uses `default-features = false` with `json` and the reviewed
   `native-tls` backend. Ambient proxy variables are disabled on the OIDC
   client; an explicit custom-CA/outbound-proxy contract remains open rather
   than silently inheriting host process state. `getrandom` supplies
   `state`/`nonce`/`code_verifier` entropy (32 bytes each, base64url).
   All HTTP calls carry explicit timeouts.
7. **Auth modes are mutually exclusive.** `SYNVEDA_OIDC_ISSUERS` (JSON
   array of trust entries) enables OIDC; `SYNVEDA_DEV_JWT_SECRET` keeps
   the ADR-0008 dev mode for the CLI/demos. Setting both is a startup
   error — the gateway refuses ambiguous auth configuration rather than
   composing verifiers. Neither set means `DisabledVerifier`, as today.
   The gateway's public callback URL derives from `SYNVEDA_PUBLIC_URL`
   (default `http://127.0.0.1:8120`).
8. **Observability.** New counters:
   `synveda_token_verifications_total{issuer, outcome}`,
   `synveda_jwks_refreshes_total{issuer, outcome}`,
   `synveda_oidc_logins_total{issuer, outcome}`; spans `oidc.discovery`,
   `oidc.jwks.refresh`, `oidc.verify`, `auth.login`, `auth.callback`,
   `oidc.exchange`. Metric names are constants in `synveda-identity`
   (emitting crate) and described by the gateway's recorder (ADR-0007
   layering: the facade below, the exporter above).
9. **Test strategy.** The mock-Entra half of the AC runs CI-clean: an
   in-process mock IdP (axum fixture) serving discovery, JWKS, authorize,
   and token endpoints, signing RS256 with a checked-in test-only key,
   Entra-shaped issuer and `tid` claim; the test drives the full PKCE
   dance including rotation (new `kid` → refetch) and the negative paths
   (wrong issuer, wrong audience, expired, bad nonce, replayed state).
   The bundled-provider half runs through the Compose Keycloak realm and the
   same provider-neutral diagnostic. Discovery proves metadata and keys, not
   token-endpoint client-registration behaviour: an actual authorization-code
   exchange remains required evidence because providers do not advertise
   public-client authentication support consistently.

## Options considered

1. **`openidconnect` crate for the whole flow** — complete and correct,
   but a large dependency tree for what is two GETs, one form POST, and
   JWT verification; its type-state API is the opposite of "boring,
   explicit code". Rejected; we keep the flow readable in our own ~200
   lines over `jsonwebtoken` + `reqwest`.
2. **`jsonwebtoken` v9 / `jwt-simple` / `josekit`** — v9 verifies through
   `ring` (licence inadmissible); `josekit` binds OpenSSL (same); pure-Rust
   `jwt-simple` is viable but `jsonwebtoken` v10's `rust_crypto` backend is
   the maintained mainstream choice. Chosen: `jsonwebtoken` v10.
3. **Custom `tid` claim in dev Rauthy instead of static tenant binding** —
   keeps "tenant from token claims" literal, but forces admin-API
   gymnastics (custom scope + user attribute) into every dev bootstrap and
   demo, for no enterprise fidelity: real single-org IdPs don't carry a
   Synveda tenant UUID either. Static binding per issuer models that
   reality; Entra-style IdPs use claim mode.
4. **Cookie-based browser sessions at the callback** — meaningful only
   with a browser UI to serve; the console is Phase 3 (CNSL). A cookie
   session would add CSRF surface and state the seed's architecture
   doesn't ask for yet. The callback returns the session material instead.
5. **Verifier chaining (OIDC + HS256 simultaneously)** — convenient for
   mixed dev setups, but two simultaneously-valid token authorities widen
   the auth surface and make "which verifier admitted this request"
   ambiguous in audit. Rejected in favour of mutually-exclusive modes.

## Consequences

- Positive: real SSO against any compliant IdP with zero core changes —
  the ADR-0008 seam held; per-issuer trust entries give AUTH-4/5 (SCIM,
  directory sync) and multi-IdP enterprises a place to stand; the PKCE
  flow is the one ADPT-1's `synveda login` will drive.
- Negative / accepted trade-offs: native TLS is present, but explicit custom
  CA and outbound-proxy configuration are not yet implemented; in-memory
  pending-login state pins login flows to one
  replica until OPS-2; the RustCrypto `rsa` crate carries the Marvin
  advisory (RUSTSEC-2023-0071) — acceptable because Synveda performs
  public-key verification only, never RSA private-key operations; if
  cargo-deny flags it, the ignore entry cites exactly that.
- Reversal trigger: if a deployment needs opaque access tokens
  (introspection instead of JWKS), a second `TokenVerifier` implementation
  slots in behind the same trait; if pending-login state must survive
  replicas before OPS-2, it moves to Postgres.

## Compliance notes

Login completions and rejections, like tenant resolutions, are audit
events; until AUD-1 lands they are visible in traces and
`synveda_oidc_logins_total` only — AUD-1 must wire `/auth/callback` and
the verifier as emission points (the resequenced Phase 1 order places
AUD-1 immediately after the identity block to bound exactly this debt).
No PDP bypass: the auth plane issues no governed content; `/v1` stays
behind AuthN → tenant → (AUTHZ-1) PDP unchanged. The dev HS256 mode
remains for CLI/demo bootstrap only and is refused whenever OIDC is
configured.
