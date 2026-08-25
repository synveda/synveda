# ADR-0090: policy relaxations are immutable typed grants inside the one PDP and VedaFlow path

- **Status**: Accepted
- **Date**: 2026-08-25
- **Feature(s)**: CPR-31
- **Deciders**: autonomous context-platform continuation

## Context

The pre-cut lapse plane stores a mutable-in-name grant projection for
`memory.read`, uses a bespoke `lapse` proposal effect and public effect route,
and selects grantees by a placement chain that no longer defines identity.
Knowledge deliberately does not inherit it. Meanwhile the four new artifact
families independently implement the intended auto-apply rule: each opens a
typed `apply` proposal, resolves the live matrix, and applies only when no
requirement remains. Keeping the old plane would leave two governance models
and no controlled relaxation over current Knowledge.

## Decision

1. **A relaxation is a stable aggregate with immutable versions.** The
   aggregate owns the current version and terminal revocation state. A version
   binds canonical terms, creator, exact approver identities, VedaFlow change,
   effective Configuration version/digest and a BLAKE3 content hash. Revision
   publishes another immutable version under an exact head precondition.

2. **The initial permission vocabulary is intentionally one member:
   `knowledge.read`.** Terms name one provisioned identity, one target scope,
   a sensitivity ceiling, requested start/end and reason. The hard expiry is
   the earlier of the requested end and the applying profile's absolute
   window ceiling. Principal-shaped targets are structurally refused. Adding
   another action is a future reviewed schema, Cedar and threat-model change,
   never free-form policy input.

3. **Create, revise and revoke are `Policy/apply` changes.** One typed change
   projection binds the complete command to the content-free object manifest
   reviewers see. The application service repeats ownership,
   `RelaxationWrite`, `ProposalOpen`, payload/hash, live approval and stale-head
   checks before applying. Revocation narrows authority but still uses this
   path; there is no direct emergency mutation API beside it.

4. **Auto-apply is matrix arithmetic, not a route.** The personal
   `open-collaboration` profile has an empty Policy cell, while `standard`
   requires one administrator and `regulated-strict` two distinct
   administrators. In every case the same proposal, typed effect, immutable
   projection, PDP decisions and audit events exist before the outcome is
   `applied`, `pending_review` or `rejected`.

5. **Configuration narrows standing authority on every request.** The
   immutable Configuration document gains an explicit relaxation policy:
   enabled, maximum duration and allowed action set. Application freezes the
   exact version used to calculate the hard expiry. Request gathering also
   resolves the current target Configuration and omits a standing grant when
   that profile now disables its action; configuration may narrow, never
   grant.

6. **Cedar still decides.** The gateway loads only rows for the authenticated
   identity whose effective window contains the database clock. The PDP
   independently matches subject, action, target scope and sensitivity and
   supplies a required `context.relaxed` only for `KnowledgeRead`. The base
   layer's permit covers Scope or KnowledgeItem resources outside principal
   scopes. Quarantine, service-token confinement and personal-scope forbids
   remain overriding forbids. No handler turns a deny into an allow.

7. **Expiry is authority-free and fail-closed.** Reads compare the immutable
   hard expiry with `now()`; no worker is needed to end access. A background
   sweep only writes an idempotent bookkeeping stamp and a content-free
   `policy.relaxation.expired` audit event. It cannot extend or restore a
   version.

8. **The old lapse plane is deleted, not translated.** The migration drops
   `policy_lapses` and its pack configuration column. Public lapse routes,
   CLI/console DTOs, store/type modules, old plan markers and production reads
   leave together. Historical VedaFlow/audit literals may remain only where
   an already-recorded event must still parse until the final schema squash;
   they confer no runtime authority and have no callable surface.

## Options considered

1. **Typed immutable relaxation inside Cedar/VedaFlow (chosen).** One workflow,
   one decision point, exact reviewable scope and bounded authority.
2. **Rename the lapse row and keep its effect route.** Preserves a second
   workflow, placement semantics and old Knowledge gap. Rejected.
3. **Consult relaxations after a Cedar denial.** A second authorization engine
   that makes determining policies incomplete. Rejected.
4. **Accept arbitrary Cedar or arbitrary action strings.** More expressive,
   but reviewers could not reason about a bounded permission delta and a typo
   could widen a new action. Rejected.
5. **Expire by background revocation.** Fails open when the worker is down.
   Rejected; the database-time predicate is the authority.

## Consequences

- Positive: personal convenience and governed review are outcomes of one
  immutable change path; cross-scope Knowledge sharing is explicit,
  inspectable and automatically time-bounded.
- Negative / accepted: the first release relaxes Knowledge reads only and a
  pending review can age past its requested window, at which point application
  rejects it rather than inventing a later window.
- Reversal trigger: a second action has a concrete product case and threat
  model → add it to the closed vocabulary, schema and Cedar tests in one ADR.

## Compliance notes

- **PDP/VedaFlow:** `RelaxationWrite` and `ProposalOpen` gate every command;
  `Policy/apply` is the only effect; `KnowledgeRead` is permitted only by the
  embedded PDP's matched context.
- **RLS:** aggregate, immutable version and typed change tables are
  tenant-bound with composite foreign keys and forced RLS.
- **Audit:** open/apply/reject/revoke/expire events carry ids, hashes, windows,
  permission names and approval identities, never Knowledge content.
- **Secrets/privacy:** no secret field exists; a relaxation cannot target a
  principal scope, and policy-denied Knowledge remains absent from listings,
  traces, counts and source views.
