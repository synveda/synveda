# ADR-0013: JIT user provisioning — mapping rules, quarantine, and the provisioning seam

- **Status**: Accepted
- **Date**: 2026-07-18
- **Feature(s)**: AUTH-2
- **Deciders**: sujitn

## Context

AUTH-2 delivers seed §2.1's zero-config promise for people: a user logs
in with SSO and lands in the right place in the tenancy hierarchy with no
admin action. Concretely (SYNVEDA_FEATURES.md): on first login, map the
IdP's groups/claims to hierarchy nodes via mapping rules — convention
default `synveda-{dept}-{team}` plus an override table — and land
unmapped users in a quarantine scope with **no read rights**.

Forces at play:

- **What exists.** AUTH-1 verifies ID and bearer tokens (ADR-0010) but
  reduces a person to a `sub` string; HIER-1 stores the hierarchy
  (ADR-0011) with user-kind leaf nodes nobody yet creates; AUTHZ-1's
  Cedar facade (ADR-0012) decides with a Principal that knows only its
  tenant. Three placeholder comments (`token.rs`, `context.rs`,
  `request.rs`) already name AUTH-2 as the feature that widens them.
- **Layering (seed §8).** `synveda-identity` and `synveda-store` are
  sibling crates: identity logic may not touch storage. Whatever
  provisioning writes (nodes, identity rows) must be orchestrated above
  both — in the gateway, as tenant resolution already is.
- **Hot path (ADR-0012).** Anything per-request belongs in a
  transaction the caller already opens. Provisioning itself is rare
  (first login), but *knowing whether a caller is quarantined* is
  per-decision.
- **Strict by default (seed §2.3).** "Quarantine with no read rights"
  must be enforced by the PDP, not implied by placement. And a user who
  never completes a login must not end up *better off* than one who
  logged in and got quarantined.
- **Dev mode must survive.** The HS256 dev verifier (ADR-0008) mints
  subjects with no IdP behind them; demos, tests, and the CLI bootstrap
  ride on it. Provisioning rules keyed to IdP claims must not brick it.
- **Sequencing.** Movers/leavers (SCIM, AUTH-4; directory sync, AUTH-5),
  roles (AUTHZ-3), and real policy packs (AUTHZ-2) land later. AUTH-2
  must not front-run them.

## Decision

1. **The token verifier reports provisioning claims; their absence marks
   an out-of-band subject.** `Claims` gains
   `provisioning: Option<ProvisioningClaims>` — groups, email, display
   name. The OIDC verifier always sets `Some` (harvesting the per-issuer
   `groups_claim`, default `"groups"`, plus `email`/`name`; a token
   without the claim yields empty groups). The HS256 dev verifier sets
   `None`: its subjects are out-of-band by definition. `IssuerConfig`
   also gains `login_scopes` (default `["openid","profile","email"]`) so
   IdPs that gate the groups claim behind a scope (Rauthy) can request
   it, while IdPs that reject unknown scopes (Entra) are not forced to.
2. **Provisioning happens at login completion, in one tenant
   transaction, in the gateway.** After `/auth/callback` resolves the
   active tenant, the gateway provisions the subject if no identity row
   exists: resolve the mapping (decision 3), create the user's personal
   scope node (`ScopeKind::User`) under the mapped parent via the
   existing `hierarchy::create`, and insert the identity row — all
   inside one `rls::begin_tenant_tx`. Concurrent first logins race on
   the `(tenant_id, subject)` unique constraint; the loser retries once
   and adopts the winner's identity. Repeat logins are a single indexed
   read. The session response now carries the identity summary (id,
   scope, quarantined) so callers see where they landed.
3. **Mapping rules: override table first, then the convention,
   deterministically.** Groups are considered in lexicographic order,
   overrides before convention, first resolution wins:
   - `group_mappings` (migration 0007) maps an exact IdP group name to
     any non-user scope of the tenant. Managed at the store level for
     now (like policy packs pre-AUTHZ-2); an admin API is later surface.
   - Convention: a group matching `synveda-{dept}-{team}` (matched
     case-insensitively; multi-hyphen names try every split point) maps
     to the team node with slug `{team}` that has a department ancestor
     with slug `{dept}`. Candidate splits are validated against the
     actual hierarchy; a group whose candidates match zero or several
     teams resolves nothing (logged) and the next group is tried.
   Nothing resolves → the user lands in quarantine. Multi-team
   membership beyond placement is AUTHZ-3 (roles) territory, not a
   second parent.
4. **Quarantine is a place, and quarantined is derived from placement.**
   The quarantine scope is the org root's child with the reserved slug
   `quarantine` (created on demand as a team-kind node, name
   "Quarantine"). An identity is quarantined iff its user node's parent
   is that node — no flag column to drift. Releasing someone is
   therefore the existing, PDP-gated, audited hierarchy move (or adding
   an override and re-provisioning the next joiner correctly); movers
   proper stay with AUTH-4/5.
5. **"No read rights" is enforced by the PDP.** The Cedar schema's
   `Principal` gains `quarantined: Bool`; the bootstrap pack becomes
   `bootstrap@2`, adding `forbid (principal, action, resource) when
   { principal.quarantined };` ahead of its existing tenant-admin
   permit. Cedar's forbid-overrides-permit keeps this rule binding on
   any future pack that forgets it, for as long as the attribute is set
   honestly.
6. **The quarantined attribute resolves at the enforcement seam, fail
   closed.** `authz::require` — already holding the caller's tenant
   transaction — reads the identity row for `(tenant, subject)` and
   sets `Principal.quarantined`:
   - identity exists → derived from placement (decision 4);
   - no identity, IdP-verified claims (`provisioning` present) → **true**:
     an IdP subject that skipped `/auth/login` cannot out-privilege one
     that logged in and got quarantined;
   - no identity, out-of-band subject (dev HS256) → false, preserving
     ADR-0012's documented bootstrap semantics until AUTHZ-2/3 replace
     them with roles.
   `TenantContext` carries the full verified `Claims` (not just the
   subject string) so the seam can tell the cases apart.

## Options considered

1. **Provision at login only + fail-closed unprovisioned rule (chosen)**
   — matches the feature's "first login" framing; the ID token is the
   one token guaranteed to carry the groups claim; no per-request write
   path. Con: an IdP subject that never logs in has no identity row —
   mitigated by treating exactly that case as quarantined.
2. **Provision on the bearer path too** — closes the same gap by
   creating identities on first API touch, but access tokens carry
   groups inconsistently across IdPs, so it would quarantine users their
   own ID token would have mapped, and it puts a write path on every
   request's cold start. Rejected.
3. **`quarantined` column on identities** — one less join, but a second
   source of truth that drifts the moment anything moves a user node;
   release would need a dedicated API. Rejected for derivation from
   placement.
4. **Identity resolution in the tenant middleware** — one seam for all
   routes, but costs a tenant transaction on every `/v1` request
   including routes that never consult the PDP; ADR-0012's doctrine
   says per-request data belongs where a transaction already exists.
   Rejected: the lookup lives in `authz::require`.
5. **First matching split for convention groups** (no hierarchy
   validation) — simpler, but `synveda-eng-data-platform` would bind to
   dept `eng`, team `data-platform` or dept `eng-data`, team `platform`
   by string luck. Validating candidates against the hierarchy makes
   the data decide; true ambiguity fails safe to the next group.
6. **Do nothing (manual admin placement)** — violates seed §2.1
   zero-config and the feature's AC outright.

## Consequences

- Positive: first login yields a correctly-scoped identity with zero
  admin action; unmapped users are contained by an enforced PDP rule,
  not a convention; the dev/demo path is untouched; HIER-2's scope
  chain and AUTHZ-3's roles get a real identity (with a personal scope
  node) to hang off; release-from-quarantine reuses the audited
  hierarchy move.
- Negative / accepted trade-offs: every PDP-gated request pays one
  indexed identity read inside its existing transaction; convention
  resolution runs one candidate-validating query per convention-shaped
  group until a match; group names that are IdP GUIDs (Entra default)
  only map via the override table — documented, since the convention
  needs names; placement is first-login-final until AUTH-4/5 own
  movers; the reserved `quarantine` slug under the root is a product
  convention an admin could occupy deliberately (it then *is* the
  quarantine bucket).
- Reversal trigger: if AUTH-5 directory sync needs continuous
  re-mapping, the mapping resolver moves from the login path into the
  sync workflow behind the same store queries; if per-decision identity
  reads show up in the inject p99 budget, fold the identity into the
  session token contract (AUTH-6) or the HIER-3 entity cache.

## Compliance notes

Provisioning creates identities and hierarchy nodes: both are AUD-1
emission points (`identity.provisioned`, plus the existing hierarchy
create events); until the hash-chained log lands they are visible in
the `identity.provision` span and `synveda_jit_provisions_total`
(outcomes: `mapped`, `quarantined`, `existing`, `error`). Provisioning
itself performs no PDP check, deliberately: it is a system write path
driven by verified IdP claims — the same trust class as tenant
admission and the future SCIM sync (AUTH-4) — not a caller action on
governed assets, and gating node creation on the not-yet-placed
principal's own permissions would both deadlock first logins and brick
JIT under the stricter AUTHZ-2/3 packs. Seed §2.2 (no path from
harness to storage bypasses the PDP) is upheld where it binds: no
governed asset is reachable through provisioning, quarantine
enforcement is a versioned policy (`bootstrap@2`) evaluated by the
same facade as every other decision, and the fail-closed unprovisioned
rule is exercised by the AC test.
RLS: `identities` and `group_mappings` ship forced RLS, policies, and
least-privilege grants in migration 0007 per the ADR-0009 structural
rule, and the TEN-2 adversarial suite's covered list grows by both.
