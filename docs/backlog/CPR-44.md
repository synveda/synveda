# CPR-44: Production hardening and maintainability cut

## Problem and evidence

The fetched context-platform head passed its existing gates but retained
reproduced correctness defects, high-coupling modules, misleading operational
and client claims, duplicate project memory and no consolidated production
readiness decision. The review started from
`37fd12b1aa0504d18f02cd72ce7b284f672ef12f`; ADR-0101 records the fixed
architectural boundary and `docs/PRODUCTION_READINESS.md` records gaps that a
refactor cannot honestly close.

## Scope

- Fix reproduced bounded-work, token, audit, erasure, database-connection,
  uninstall-key, console-state, metric-cardinality and shutdown defects.
- Extract cohesive context, Knowledge, Skill, Tool and response-finishing
  responsibilities without changing their trust boundaries.
- Reduce accidental public Rust surface, confirmed dead code, stale comments,
  duplicate response shells and obsolete SDK/documentation placeholders.
- Consolidate feature state and current documentation, add focused drift
  checks, and leave an evidence-backed readiness register.

## Non-goals

- No replacement for Cedar, forced RLS, VedaFlow, the audit chain, key model or
  schema epoch.
- No generic workflow/service framework, arbitrary line-count split, public API
  redesign, provider implementation, licence choice or unsupported readiness
  claim.
- No relabelling of deterministic fixtures as live client or recovery evidence.

## Architecture seam

Changes stay within current feature modules and public contracts. Gateway
handlers continue to decide through Cedar and transact under forced RLS;
governed mutations continue through VedaFlow and content-free audit. Persistence
semantics remain in `synveda-store`, generated API artefacts remain derived,
and operational gaps map to current open briefs rather than new runtime layers.

## Acceptance criteria

- Baseline source, inventory and authoritative gate results are recorded, with
  unavailable external prerequisites distinguished from product failures.
- Every reproduced defect has a behaviour or adversarial regression test.
- Structural extracts reduce a demonstrated responsibility/coupling boundary;
  OpenAPI operations/schemas, CLI behaviour, persisted schema, PDP/RLS,
  VedaFlow, audit, idempotency and telemetry semantics remain stable except for
  the documented Knowledge-erasure response correction.
- Current documentation has one feature inventory, open briefs use this shape,
  completed diaries and the prompt ledger are deleted, and internal links plus
  executable release/support claims are checked.
- The final readiness verdict remains honest while every P0/P1 gap has a scoped
  acceptance slice and owner/external dependency.
- The final working tree is clean and the complete requested gate set passes.

## Required tests

- Focused Rust/TypeScript/database regressions for each semantic fix and
  extracted capability.
- OpenAPI/client parity, documentation/backlog/ADR, adapter, deployment,
  tenancy/security and demo drift checks.
- Full workspace format, Clippy, build/test, dependency/licence, evaluation,
  database and deterministic Claude acceptance gates.
- Live acceptance only with an installed authenticated proprietary client;
  otherwise record the missing prerequisite without substitution.

## Rollout and rollback

Land one concern per reviewable commit. Structural commits must be reversible
without data migration; semantic fixes retain their prior tests plus focused
regressions. Do not change benchmark floors to accept regressions. The branch
does not merge to main or publish until final verification; normal git revert
is the rollback for each isolated commit.

## Dependencies

Final completion depends on local Docker/Postgres/Helm/Node/Rust prerequisites
for the requested gates and normal remote push access. Live client evidence
depends on a valid proprietary-client credential. Release signing, key custody,
backup/PITR, availability/SLO, platform support and licence decisions remain
with the owners named in `docs/PRODUCTION_READINESS.md`.
