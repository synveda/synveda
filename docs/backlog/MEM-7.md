# MEM-7: Identity stitching

## Problem and evidence

Authentication verifies OIDC per issuer, but persistent user lookup is currently `identities.by_subject(tenant, subject)` and an identity stores only one optional subject. A human represented by different stable subjects across trusted issuers or harnesses therefore becomes separate principals, while equal subject strings from different issuers can collide inside one tenant. Email/display-name heuristics would turn account similarity into authority and private-Knowledge access.

## Scope

- Add immutable, issuer-qualified subject bindings `(tenant, issuer, subject) -> user IdentityId`, with one active principal resolved at authentication.
- Provide explicit governed link and unlink commands that require owner-approved proof or approval, record before/after binding evidence, and fail on collisions, departed/sealed identities, or tenant mismatch.
- Let linked user subjects share one existing Identity/principal scope so Sessions from different harnesses compose through normal authorized Knowledge; do not copy or rewrite artifacts.
- Preserve service identities as separate principals. An optional “operated by” provenance relationship must not confer grants or user-private Knowledge.
- Define recovery, issuer migration, departed/rehire, compromised account, unlink, and reauthentication behaviour before enabling links.

## Non-goals

- Linking by email, display name, behavioural similarity, model inference, or same-device observation.
- Cross-tenant identities, anonymous-user merging, service-to-user authority inheritance, or bulk content ownership rewrites.
- Translating pre-epoch data, bypassing Cedar/RLS, or making an adapter the identity authority.
- Hiding which issuer-qualified subjects are linked from authorized administrators or affected users.

## Architecture seam

OIDC verification yields the exact issuer and subject; provisioning resolves their binding in `synveda-store` before constructing `IdentityContext`. A stable user `IdentityId` and its principal scope remain the placement/authority spine. Link mutations are typed governed effects with dedicated Cedar actions, forced RLS, content-free audit, bounded metrics, and reauthorization on every later request.

## Acceptance criteria

- Two explicitly linked issuer-qualified user subjects resolve to one IdentityId and retrieve the same authorized cross-session Knowledge without duplicating Sessions or artifacts.
- Equal `sub` values from different issuers remain distinct unless an explicit link succeeds; email/display-name equality alone never changes authority.
- Linking/unlinking is atomic, idempotent, conflict-safe, fully audited, and immediately reflected after token reauthentication/cache expiry.
- Deny, revoke, collision, departed/rehire, compromised-subject, service-identity, and cross-tenant cases fail closed without leaking another binding.
- Unlink preserves historical audit/provenance, gives future requests the declared principal outcome, and never silently transfers grants or content.

## Required tests

- Store tests for issuer-qualified uniqueness, concurrent link/unlink, forced RLS, immutability/history, and service/user separation.
- OIDC integration matrix with two issuers, equal/different subjects, key rotation, reauthentication, directory adoption, and departed/rehire cases.
- Cedar actions for view/link/unlink/recover with allow, deny, revoke, and cross-tenant assertions.
- Cross-harness two-session Knowledge composition test plus proof that artifact ownership rows are unchanged.
- Audit-chain, cache invalidation, redaction, and account-recovery abuse tests.

## Rollout and rollback

Introduce issuer-qualified bindings and shadow resolution first, detecting collisions without changing principals. Enable explicit links for one trusted issuer pair after review. Rollback disables new link mutations and resolves only owner-approved retained bindings; never split an established principal or move artifacts automatically.

## Dependencies

An accepted ADR must fix binding history, principal continuity, proof/approval, unlink, recovery, issuer migration, and service-account semantics. Identity/security owners must approve trusted issuer/subject stability, administrator override, user visibility/consent, reauthentication/cache bounds, incident response, and audit retention.
