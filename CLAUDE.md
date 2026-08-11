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
- Crate dependency rule (seed §8; tech plan §5 adds synveda-vedaflow, ADR-0064
  adds synveda-crypto): types ← crypto ← {policy, store, identity, audit}
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
through SKIL-4, OPS-1, CNSL-1, ADPT-2, CNSL-2, AUTH-4, AUTH-5, EVAL-3 and
OPS-2 and TEN-3 are the Phase 3 features done so far. 62 of 93 features delivered — see docs/backlog/STATUS.md for what each
one proved and what it left standing. (The total read 86 until 2026-08-05,
when it was corrected to the 88 STATUS.md and `make check-backlog` had both
said for some time; AUTHZ-7 was filed the same day by CNSL-2/ADR-0058, making
it 89, EVAL-7 on 2026-08-07 by EVAL-3/ADR-0061, making it 90, OPS-7 on
2026-08-10 by OPS-2/ADR-0062, making it 91, and CTX-7 and TEN-7 the same
day by TEN-3/ADR-0063, making it 93.)

Phase 3 was reordered on 2026-08-04 by demo-readiness (see the Sequencing
note in SYNVEDA_FEATURES.md): the demo block leads — OPS-1, CNSL-1, ADPT-2,
CNSL-2, AUTH-4,5, EVAL-3, OPS-2 — and TEN-3..6, AUD-3,4 and the rest follow.

The product is installable since OPS-1: `synveda init` — see docs/INSTALL.md.
Since AUTH-4 it syncs a directory: `/scim/v2` for Entra and Okta, with
`synveda scim token issue` for the credential (ADR-0059). Nothing has
replayed a frame from a live Entra or Okta tenant yet — the vendor corpus is
transcribed from their published tables.
Since CNSL-1 it has a browser: the gateway serves the admin console from its
own origin at `/console/`, which needs `pnpm --filter @synveda/console build`.
A missing bundle is not a boot failure — the route 404s and the rest of the
product runs, because a static asset must not be a dependency of the audit
log (crates/synveda-gateway/src/console.rs).

Phase demo goal: Entra/Okta live, spec-compliant governed skills into Claude
Code + Cursor, LongMemEval scores published, Helm install.
(LongMemEval scores ARE published since 2026-08-09 — docs/BENCHMARKS.md, QA
0.300 and retrieval recall 0.357 over 10 of 500 instances, with the corpus
digest, both model versions and the commit in the row. Ten instances is a
first data point rather than a benchmark claim; `make eval-longmemeval-full`
is the run somebody schedules.)
(Read "LoCoMo/LongMemEval" until 2026-08-07, when EVAL-3/ADR-0061 found
LoCoMo's corpus is CC BY-NC 4.0 — a licence that withholds exactly the
published commercial claim this goal names. The second benchmark is EVAL-7.)
(ADPT-2 recorded its acceptance corpus from Claude Desktop and **Zed** —
Cursor stays an `install` target because this goal names it, but nothing has
replayed a real Cursor frame. See ADR-0057 amendment 2.)
(Helm install IS done since 2026-08-10 — `deploy/helm/synveda`, installed
into a kind cluster by `demos/ops-2-helm-install.sh`, which asserts a
governed round trip, a CloudNativePG failover and a live RLS backstop
rather than readiness. Two limits belong beside the claim: the gateway is
**one replica** and the chart refuses to render a second until OPS-7, and
the test's issuer is the bundled Rauthy at a Service DNS name — so this
profile has not met a live Entra or Okta tenant either. It is also the
first thing that ever asked the gateway *image* to serve, which closes
ADR-0055 decision 8's open residue.)

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
