# Synveda

Enterprise memory & context management platform for AI agents.
Rust workspace + TypeScript adapters. Postgres-first. Governed by VedaFlow.

## Required reading (in order, before any task)
1. docs/SYNVEDA_SEED.md        — product principles & invariants (§2 is law)
2. docs/SYNVEDA_TECH_PLAN.md   — stack decisions & VedaFlow design
3. docs/SYNVEDA_FEATURES.md    — feature backlog; ALL work maps to a feature ID
4. docs/backlog/STATUS.md      — what is done, what each feature found, what it left standing

## Working rules
- Every task references a feature ID (e.g. FND-1). Branch: feat/<ID>.
  Commit messages include the ID: "FND-1: scaffold rust workspace".
- A feature is done ONLY when its acceptance criteria in SYNVEDA_FEATURES.md pass,
  demonstrated by a test or a runnable demo script under demos/.
- Never create a code path that bypasses the PDP (seed §2.2), even in tests —
  use a test policy pack instead.
- Architectural choices get an ADR in docs/adr/ (copy 0000-template.md) BEFORE
  implementation.
- Crate dependency rule (seed §8; tech plan §5 adds synveda-vedaflow): types ← {policy, store, identity, audit}
  ← retrieval/ingest ← gateway. Nothing imports upward. Adapters/SDKs depend only
  on the public API.
- Licences: MIT/Apache-2.0/PostgreSQL only in the core path. cargo-deny enforces.
- Prefer boring, explicit code over cleverness. This product sells trustworthiness.
- cargo fmt + clippy -D warnings must be clean before any commit.
- sqlx compile-time checked queries only; no string-built SQL, ever.

## Definition of done (every feature)
1. Acceptance criteria met and demonstrated
2. Tests written (unit + the AC test)
3. Tracing spans + metrics on new paths
4. Audit events emitted for any new action type
5. docs/backlog/STATUS.md updated

## Current phase
Phase 3 — Enterprise (wk 11–16). Phases 0, 1 and 2 are complete; SKIL-1
through SKIL-4 and OPS-1 are the Phase 3 features done so far. 54 of 86
features delivered — see docs/backlog/STATUS.md for what each one proved and
what it left standing.

Phase 3 was reordered on 2026-08-04 by demo-readiness (see the Sequencing
note in SYNVEDA_FEATURES.md): the demo block leads — OPS-1, CNSL-1, ADPT-2,
CNSL-2, AUTH-4,5, EVAL-3, OPS-2 — and TEN-3..6, AUD-3,4 and the rest follow.

The product is installable since OPS-1: `synveda init` — see docs/INSTALL.md.

Phase demo goal: Entra/Okta live, spec-compliant governed skills into Claude
Code + Cursor, LoCoMo/LongMemEval scores published, Helm install.

## Commands
- make dev-up      — start Postgres(+pgvector+PGMQ), Rauthy, Temporal, TEI, Jaeger
- make smoke       — end-to-end health check
- make dev-down    — stop; state persists in named volumes
- make ci          — exactly what .github/workflows/ci.yml runs; green here == green there
- make db-test     — the full suite against DATABASE_URL (tests needing Postgres skip without it)
- make eval        — the eval harness against a live stack, gated by evals/baseline.json
- make eval-check  — parse suite + corpora + baseline with no stack (part of `make ci`)
- make eval-retrieval / eval-security / eval-extraction-live — the nightly and
  live-model gates, each with its own baseline
