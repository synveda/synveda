# ADR-0018: Service identities — IdP-issued client-credentials tokens, service-kind identities, base-layer scope confinement

- **Status**: Accepted
- **Date**: 2026-07-19
- **Feature(s)**: AUTH-3
- **Deciders**: sujitn

## Context

AUTH-3 delivers seed §5's service identities: OAuth2 client-credentials
with scoped, short-lived tokens; every headless agent runs *as* an
identity at a hierarchy node, never as a shared key. The AC: an agent
token with team scope cannot call org-scope endpoints.

Forces at play:

- **Synveda is an OIDC client, never a token authority** (ADR-0010,
  tech plan §1.2). There is no password store and never will be; the
  same doctrine covers client secrets. Whatever issues service tokens,
  it is not the gateway.
- **The principal is `(tenant, subject)`** (ADR-0012), the identity row
  is keyed the same way (ADR-0013), and role bindings are subject-keyed
  precisely so any verified subject — human, dev, or headless — can be
  bound (ADR-0015). A service identity that is anything *other* than an
  identity row would fork every seam built on that shape.
- **The fail-closed rule must distinguish credential classes.** An access
  token carrying any configured service audience is a service credential,
  even when it also carries the primary API audience. It must resolve to one
  active registered service identity before tenant admission; an unknown,
  departed or user-kind subject is a uniform 401, not a role-free user.
- **Enforcement belongs in the PDP** (seed §2.2): "team scope cannot
  call org-scope endpoints" must be a policy the decision log can name,
  not a gateway if-statement. And it must survive custom packs —
  invariants don't travel by convention (ADR-0014 decision 2).
- **The schema constrains placement.** `identities` carries
  `unique (tenant_id, scope_id)` and derives quarantine from the
  placement node's parent; `ScopeKind::User`'s doc comment already
  reads "a person's (or service identity's) personal scope".
- **Roles must not be the mechanism.** Since AUTHZ-3 a role-free
  subject already has no administrative power anywhere, so the AC is
  vacuously true for an unbound agent. The feature's substance is the
  stronger property: the token's scope confines the agent *even when
  roles are bound to its subject* — a leaked or over-privileged agent
  credential stays inside its subtree.
- **Agents are the product's read path.** CTX-1/2/3 will compose
  context for exactly these identities; confinement must not sever the
  membership floor (own-chain `MemoryRead`, ADR-0014 decision 5) or
  agents would inject nothing from dept/org published scopes.

## Decision

Service tokens are IdP-issued client-credentials access tokens verified
through the existing per-issuer path; a service identity is an identity
row of a new `service` kind placed like a user under its anchor node;
and the anchor subtree confines every decision through a base-layer
forbid over a new `token_scope` principal attribute.

1. **Tokens come from the IdP.** A headless agent is an OAuth2 client
   of the tenant's IdP (Rauthy in dev/SMB, Entra/Okta app registrations
   in enterprise); it obtains access tokens via the client-credentials
   grant and presents them as bearer tokens. `OidcVerifier` verifies
   them exactly like user bearer tokens — same issuer trust entries,
   same JWKS cache and rotation handling. `IssuerConfig` gains
   `service_audiences` (default empty). Bearer audiences form a closed set:
   every value must be the primary API audience or a configured service
   audience; the login client id and unknown or duplicate audiences are
   refused. Presence of any service audience classifies the whole token as a
   service credential, including mixed primary+service arrays, and tenant
   admission requires its exact active `kind = service` identity. Only this
   service credential class may fall back from a missing `sub` to `azp`;
   primary API bearers and interactive ID tokens never do. Synveda stores no
   client secrets and mints
   nothing. The dev HS256 mode needs no change: kind is a property of
   the identity row (decision 2), not the token, so
   `synveda token issue` with a registered service subject exercises
   the same seam semantics.
2. **A service identity is an identity row with `kind = 'service'`,
   placed like a user.** Migration 0010 adds
   `kind text not null default 'user'` (checked `user|service`) to
   `identities`, and a `delete` grant for revocation. Registration
   creates a `ScopeKind::User` personal leaf under the anchor node and
   the identity row pointing at it, in one tenant transaction — the
   exact JIT shape (ADR-0013 decision 2), which keeps
   `identities_scope_unique`, the quarantine derivation, subject
   uniqueness, and the RLS coverage list all unchanged. The anchor may
   be any non-user scope except the quarantine node (registering an
   agent into quarantine is refused as an operator error; the kind-rank
   rule already refuses user-kind anchors). Re-anchoring an agent is
   the existing PDP-gated hierarchy move of its personal leaf;
   revocation deletes the row and the leaf.
3. **Registration is a PDP-gated surface plus a CLI break-glass.**
   `POST/GET /v1/service-identities` and
   `GET/DELETE /v1/service-identities/{id}`, behind uniform-404 then
   the PDP like every governed route. The action vocabulary gains
   `ServiceIdentityManage` (create/delete, applies to the anchor
   scope) and `ServiceIdentityRead` (get on the anchor; list on the
   tenant resource); the product packs bump to `@3`, gating manage on
   `steward`/`org-admin` over the bound subtree and read on those plus
   `auditor` — the established admin-plane shape (ADR-0015
   decision 4). `synveda service register/list/remove` is the direct
   store path for dev bootstrap and break-glass, like
   `synveda role bind`.
4. **Confinement is a base-layer forbid over `Principal.token_scope`.**
   The Cedar `Principal` gains an optional `token_scope: Scope`
   attribute; `base.cedar` gains: forbid every action whose resource is
   not a scope inside `token_scope`, unless the action is `MemoryRead`
   on the principal's own chain. The enforcement seam (`authz::gather`)
   sets the attribute for service identities to the *anchor* — the
   placement chain's second node, already resolved by the scope-chain
   cache, so confinement costs zero extra reads. Semantics:
   - Everything outside the anchor subtree is denied *regardless of
     roles* — a tenant-wide `org-admin` binding on an agent subject
     cannot escape the subtree. Forbid overrides permit; a custom pack
     cannot drop the rule (ADR-0014 decision 2).
   - Tenant-plane resources (`Resource::Tenant`: tenant defaults,
     tenant-wide bindings, root creation) are never inside any scope
     subtree: service tokens can never act on the tenant plane.
   - The one carve-out is exactly the role-free membership floor: own-
     chain `MemoryRead`, so a team agent composes team → department →
     org published context on inject like any placed member. The
     carve-out grants nothing a role grants — it is the floor every
     placed principal already holds.
   A service identity whose placement chain cannot be resolved is
   treated as quarantined (fail closed), never as unconfined.
5. **Short-lived is enforced, not assumed.** `Claims` gains the token's
   lifetime (`exp − iat`, when the token carries `iat`); at the
   enforcement seam a service identity's token is refused — uniform
   401, counted — when the lifetime is unknown or exceeds
   `SYNVEDA_SERVICE_TOKEN_MAX_TTL_SECS` (default 3600). User tokens
   are untouched: session length is the IdP's business; the cap is the
   product's teeth for the feature's "short-lived" promise where
   credentials are headless. `/v1/whoami` (no PDP, introspection only)
   remains reachable by an over-long token; every governed route is
   not.
6. **Observability and audit.** Registration surface:
   `synveda_service_identity_operations_total{op, outcome}` and spans
   `service_identity.{register,list,get,remove}`; seam rejections:
   `synveda_service_token_rejections_total{reason}` — both described in
   the gateway recorder per ADR-0007. Register/remove and token
   rejections are AUD-1 emission points, recorded in STATUS.md with
   the others. Decisions already log pack@version and effective roles
   on every call; nothing about the decision log changes.

## Options considered

1. **IdP-issued client-credentials tokens (chosen)** — one token
   authority, one verification path, secrets live where secret
   lifecycle tooling already exists; enterprise IdPs all speak the
   grant. Con: per-issuer `service_audiences` config, and dev demos
   must register a client in Rauthy first.
2. **Gateway-minted service tokens** — no IdP coupling, but the gateway
   becomes a token authority with a client-secret store, violating the
   ADR-0010 doctrine that sold OIDC in the first place; two token
   authorities make "which verifier admitted this" ambiguous again
   (the rejected verifier-chaining shape).
3. **Confinement as a gateway middleware check** — a subtree test
   before the PDP would be a second enforcement seam outside policy
   (seed §2.2), invisible to the decision log and to golden tests.
   Rejected.
4. **Confinement in the product packs** — a stored custom pack that
   forgot the rule would un-confine every agent in the tenant;
   invariants don't travel by convention (ADR-0014 decision 2).
   Rejected for the base layer.
5. **A separate `service_identities` table** — clean separation, but
   duplicates the FK/unique/RLS machinery and forces the seam into two
   lookups per request (which table is this subject in?). The kind
   column keeps one seam read. Rejected.
6. **Identity row pointing directly at the anchor node** — no leaf
   node, but breaks `identities_scope_unique` (one identity per node —
   two agents per team impossible), muddles the parent-based quarantine
   derivation, and leaves agents with no personal scope for MEM-1's
   derived-channel writes. Rejected.
7. **`token_scope` = the personal leaf** — confines the agent so hard
   that a role bound at its own team could never act at the team.
   Rejected: the anchor is the token's scope; the leaf is placement
   plumbing.
8. **Per-token scope narrowing via OAuth `scope` parameters** — lets
   one client mint differently-scoped tokens, but demands IdP scope
   registration and claim mapping per IdP. Deferred, not rejected:
   the seam would intersect the claimed scope with the anchor,
   narrowing only.

## Consequences

- Positive: the AC holds with defense in depth — an agent credential
  is confined by registration, not by the absence of roles; an
  unregistered or wrong-kind service credential is rejected before tenant
  use; agents are full identities (bindable, movable, quarantinable,
  composable) with no parallel machinery; the token path, the seam,
  and the caches are all reused unchanged.
- Negative / accepted trade-offs: real IdPs need `service_audiences`
  configured per issuer; agents can never act on the tenant plane
  (accepted: agents do not administer tenants); the base layer now
  names one action (`MemoryRead`) — the price of not severing the
  composition floor; the TTL cap requires `iat` in service tokens
  (fail closed when absent); IdP client lifecycle (secret rotation,
  disabling) stays IdP-side, and Synveda-side revocation is
  registration removal, next-request effective.
- Reversal trigger: if a deployment needs per-token narrowing, option
  8's scope-claim intersection slots into the same seam; if agents
  legitimately need tenant-plane automation, revisit the tenant-plane
  exclusion with an explicit, audited exemption design — not by
  widening the forbid.

## Compliance notes

Seed §2.2 holds: confinement is a versioned policy evaluated by the
same facade as every decision — one enforcement seam, decision-logged
with pack@version, golden-testable across the role×action matrix. No
new tenant-scoped table: `identities` is already RLS-forced and
covered; the kind column changes no isolation property. Registration
mutations and seam rejections are AUD-1 emission points (tracked in
STATUS.md). Tests register service identities through the same store
rows and verify through the same facade — never a PDP bypass; the
mock-IdP fixture grows a client-credentials grant so the token path is
CI-clean end to end, with the live-Rauthy half in the demo script
(ADR-0010's test doctrine).
