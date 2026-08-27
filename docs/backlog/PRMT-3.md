# PRMT-3: Prompt experiment evidence

## Problem and evidence

The current Prompt vocabulary deliberately contains only `draft` and `published`, and runtime resolution can pin an exact published VedaFlow commit/object hash. There is no experiment aggregate, session-stable assignment, explicit outcome contract, or report. Adding mutable “A/B channels” would create a second publication truth and make a session's prompt depend on moving refs rather than recorded evidence.

## Scope

- Define a governed prompt experiment that references two exact already-published Prompt commits/object hashes: baseline and candidate.
- Allocate a bounded percentage of eligible Sessions deterministically from tenant, experiment, session, and a versioned salt; persist one immutable assignment before prompt use.
- Resolve only the assigned exact commit, reauthorize PromptRead at use time, and record assignment, model/configuration identifiers, and prompt hash without prompt content in audit.
- Accept explicit, typed outcome events tied to the Session and assignment; aggregate predeclared metrics, counts, exclusions, uncertainty, and exposure by variant.
- Produce an automatic report and an explicit stop/promote decision input; never promote a Prompt automatically.

## Non-goals

- New draft/published channel variants, moving-head assignment, client-selected arms, or random reassignment on retry.
- Inferring success from free-form model text, optimizing arbitrary user content, or becoming a general experimentation platform.
- Sending draft prompts to runtime, weakening PromptRead, or including prompt/session content in metrics or audit.
- Automatic VedaFlow approval, publication, rollback, or statistical claims below the declared sample threshold.

## Architecture seam

Add a stable experiment aggregate and immutable assignment/outcome evidence beside the Prompt registry; governed mutations apply through typed VedaFlow effects. Session-scoped gateway resolution maps an assignment to the existing exact Prompt resolver. Aggregation reads append-only evidence under PDP/RLS and exposes only bounded, tenant-scoped reports.

## Acceptance criteria

- Two published Prompt commits run concurrently at the declared allocation, and one Session always receives the same exact variant across retries and restarts.
- Draft, deleted, revoked, or unauthorized variants are never served; failure uses the declared baseline/fail-closed policy and is auditable.
- The report names exact commits, allocation/salt version, eligibility window, sample counts, exclusions, outcome definition, uncertainty, model/configuration, and data completeness.
- Duplicate outcomes are idempotent, late outcomes follow a declared cutoff, and absent feedback is not counted as success or failure.
- Stopping an experiment sends all new eligible Sessions to baseline while preserving prior assignments and evidence.

## Required tests

- Deterministic allocation, boundary percentages, salt/version, restart, concurrency, and distribution-property tests.
- Prompt draft/published/pinned/revoked/deleted plus Cedar allow/deny and cross-tenant RLS matrix.
- Idempotent assignment/outcome, late/missing outcome, report denominator, confidence, and redaction tests.
- VedaFlow proposal/approval/effect and audit-chain tests for create, start, stop, and promotion input.
- Public-API demo with two exact prompt versions and a reproducible report.

## Rollout and rollback

Start with report-only storage and 0% candidate exposure, then canary at a bounded owner-approved percentage after privacy/statistical review. An emergency stop assigns all new Sessions to baseline immediately. Rollback disables experiment resolution but retains immutable assignments/outcomes and leaves ordinary published Prompt resolution unchanged.

## Dependencies

An accepted ADR must replace the title's channel implication with exact-commit experiment semantics and fix failure/stop behaviour. The owner must predeclare the primary outcome, eligibility, allocation, minimum sample/uncertainty rule, duration, late-event cutoff, retention/privacy terms, model controls, and human promotion authority.
