# ADR-0100: the packaged demo is a resumable public-API client

- **Status**: Accepted
- **Date**: 2026-08-26
- **Feature(s)**: CPR-41
- **Deciders**: Autonomous continuation of the context-platform programme

## Context

CPR-22 proves the PulseBoard personal/team loop, but only as an acceptance
test. The packaged CLI still has no product walkthrough. The retired demo
seeders opened the store and depended on hierarchy, role-binding and global
observe/recall surfaces; CPR-13 and CPR-36 correctly deleted them. Recreating
that pattern would give the demo a second application service layer and make
its evidence incomparable with the console and adapters.

There is also a fresh-tenant bootstrap cycle in ADR-0089's fail-safe. With no
Configuration binding, `regulated-strict` requires two administrators to
apply a Configuration change, while the first real login can have provisioned
only one administrator. A demo cannot select even an exact built-in profile
through the public API, and direct SQL or a demo-only policy bypass would
violate the platform's central governance claim.

## Decision

1. **`synveda demo` is an authenticated application client.** `demo start`
   assumes normal installation and login are complete, then creates every
   workspace, project, repository, session, event, capture decision,
   Knowledge change, Skill, Tool and OKF job through supported public HTTP
   operations. It imports no store crate, starts no hidden gateway and mints
   no identity or token.
2. **The first exact profile adoption has one narrow VedaFlow outcome rule.**
   A principal whose live decision includes `administrator` may auto-apply
   only (a) the tenant's first Configuration artifact when its document is
   byte-for-byte one canonical built-in template with truthful template
   provenance, and (b) the first binding when it points that sole artifact at
   its governing scope. Both operations still create immutable objects and a
   typed proposal, run the PDP, re-authorise the effect and append normal
   audit events. A modified document, another artifact/binding or another
   role follows the live approval matrix. The tenant root row serialises the
   absence test so concurrent requests have exactly one winner.
3. **Profiles remain governed data.** `personal`, `team` and `governed` copy
   the canonical `personal`, `team` and `enterprise` Configuration templates;
   they do not select another binary, schema, route set or runtime branch.
4. **Alice is the authenticated caller; Bob is never fabricated.** Team mode
   uses a distinct supplied credential profile (or an existing profile named
   `bob`) and grants that real principal project membership through the public
   API. Without one, it returns a one-time invitation, strips the secret from
   local state and runs the clean-session reuse as Alice while explicitly
   refusing a teammate-verification claim.
5. **The walkthrough is resumable and reset preserves evidence.** Stable
   per-step idempotency keys and a private, atomically replaced mode-0600 XDG
   receipt make interrupted runs resumable. The receipt contains only public
   responses over fixed synthetic content and never persists invitation
   tokens or accept URLs. `demo reset --force` archives receipt-owned product
   aggregates through public APIs; immutable revisions, proposals and audit
   history remain. It is not a database reset.
6. **Sessions use the complete lifecycle.** The walkthrough creates and
   closes each run through the two-phase session endpoint, explicitly invokes
   candidate extraction/review, and uses clean subsequent sessions for reuse
   and supersession evidence.
7. **Governance outcomes stay visible.** Knowledge publication uses capture
   decisions or the Knowledge command API. Skill and Tool changes use their
   existing typed VedaFlow commands; when a profile requires review, the demo
   reports the Advanced Reviews address and never presents the unreviewed
   version as pinned, bound or active. OKF v0.2 import materialises candidates
   and deterministic export uses the public project API.
8. **Retrieval claims name the implementation.** A deterministic hash
   embedder is labelled as lexical-only. The walkthrough points to the
   supported TEI/BGE-M3 option for a semantic demonstration and never renames
   hashing as semantic retrieval.

## Options considered

1. **Resumable public-API client plus the narrow first-profile rule (chosen).**
   Exercises the actual product and closes the genuine fresh-tenant cycle
   without adding another authority path.
2. **Bundle a database seeder.** Rejected: it bypasses PDP, VedaFlow and audit
   and would immediately drift from the public contract.
3. **Run an embedded gateway/store service in the CLI.** Rejected: that is a
   second application implementation and violates the crate/client boundary.
4. **Auto-create a fake Bob or save an invitation token.** Rejected: it would
   fabricate live team evidence or persist bearer authority in ordinary demo
   state.
5. **Loosen Skill/Tool approvals for demo content.** Rejected: a recognisable
   fixture name is not authority and must not weaken the configured matrix.

## Consequences

- Positive: the one-command tour and every adapter/console path share one
  generated contract and one PDP/VedaFlow/RLS/audit implementation; failures
  are resumable and governance limitations remain honest.
- Negative / accepted trade-offs: installation/login remain explicit
  prerequisites; real teammate evidence needs a second credential; strict
  Skill/Tool matrices can leave those demo changes pending until reviewers
  act; reset intentionally retains historical evidence.
- Reversal trigger: the public application gains a governed first-tenant
  operator-enrolment workflow that can adopt Configuration before ordinary
  login → remove the first-profile outcome rule and use that workflow.

## Compliance notes

- **PDP/VedaFlow:** every product mutation enters the existing public command
  path. The first-profile rule changes only the calculated outstanding set
  after live `ConfigurationWrite` and `ProposalOpen` allows; it does not skip
  proposal creation, effect re-authorisation or audit.
- **RLS:** no table or schema changes. The root row lock and every ordinary
  store operation execute inside the tenant-bound transaction.
- **Audit/secrets:** standard action events retain ids, hashes and governance
  outcomes. One-time invite material is shown once and never logged or stored;
  no Tool secret value is part of the fixture.
