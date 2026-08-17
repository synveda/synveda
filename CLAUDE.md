# Synveda

Enterprise memory & context management platform for AI agents.
Rust workspace + TypeScript adapters. Postgres-first. Governed by VedaFlow.

## Required reading (in order, before any task)
1. docs/SYNVEDA_SEED.md        — product principles & invariants (§2 is law)
2. docs/SYNVEDA_TECH_PLAN.md   — stack decisions & VedaFlow design
3. docs/SYNVEDA_FEATURES.md    — feature backlog; ALL work maps to a feature ID
4. docs/backlog/STATUS.md      — what is done, what each feature found, what it left standing
5. docs/implementation/synveda-context-platform.md
                               — the Phase 5 context-platform redesign: the base-commit
                                 inventory, the deletion map, the ordered programme and its
                                 running record. Required before any CPR work; ADR-0068's
                                 eight decisions are locked and no prompt reopens them.

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
Phase 5 — Context platform redesign, since 2026-08-17. Phase 3 is paused
mid-phase, not finished: OPS-9, OPS-10, TEN-5,6, AUD-3,4, GRPH-3, EVAL-6,
CTX-7, OPS-3,4, ADPT-3, CTX-6 and FLOW-8 are still open, and the phase's
demo goal is met except for the two live-tenant claims. What moved is the
audience. Everything above Phase 5 is built for an organisation — a tenant's
hierarchy root *must* be `kind = 'org'` (migration 0004, a row-local CHECK)
and every node under it must be a division, department, team or user — so one
person, or four sharing agent context, must declare themselves a company
before this product will hold a record. Phase 5 re-cuts that as 33 ordered
prompts on `feat/context-platform-mvp`, with the decisions locked in ADR-0068
and the running record in docs/implementation/synveda-context-platform.md.
**It is a pre-1.0 hard cut**: a fresh schema epoch, no old-data migration, no
compatibility shims, and old databases rejected with a reset instruction.
Since CPR-2 that is enforced rather than planned (ADR-0069): `schema_metadata`
carries the epoch, `synveda_store::epoch::verify` is the guard the gateway
refuses to boot past and `/readyz` re-asks per probe, `migrate` refuses a
pre-cut database before touching it, and `synveda reset --database --force`
is the only way through. **Your dev database will be refused** — reset it.
The 38-migration chain is not squashed yet; that is Prompt 33.

Phases 0, 1 and 2 are complete; SKIL-1 through SKIL-4, OPS-1, CNSL-1, ADPT-2,
CNSL-2, AUTH-4, AUTH-5, EVAL-3, OPS-2, TEN-3, TEN-4 and OPS-8 are the Phase 3
features done. 66 of 99 features delivered — see docs/backlog/STATUS.md for
what each one proved and what it left standing. (The total read 86 until
2026-08-05, when it was corrected to the 88 STATUS.md and `make
check-backlog` had both said for some time; AUTHZ-7 was filed the same day by
CNSL-2/ADR-0058, making it 89, EVAL-7 on 2026-08-07 by EVAL-3/ADR-0061,
making it 90, OPS-7 on 2026-08-10 by OPS-2/ADR-0062, making it 91, CTX-7 and
TEN-7 the same day by TEN-3/ADR-0063, making it 93, and OPS-8 filed and
delivered on 2026-08-11, making it 94. ADPT-8 was filed on 2026-08-13 by
running ADPT-1's plugin in a real Claude Code session, making it 95, and
OPS-9 the same day by asking what somebody who takes OPS-8's install actually
sees, making it 96 — this trail had stopped at 94 while the headline said 95,
which is the drift the trail exists to prevent. **OPS-10 was filed the same
day as OPS-9**, by asking how that same stranger removes it, making it 97;
the trail missed it and the headline read 96 against a checker that had said
97 since — the identical drift, one entry later. CPR-1 filed and delivered
2026-08-17, making it 98, and **CPR-2 the same day**, making it 99.
Prompts 3–33 of its programme are filed by the prompts that run them, so
this number will keep moving.)

Phase 3 was reordered on 2026-08-04 by demo-readiness (see the Sequencing
note in SYNVEDA_FEATURES.md): the demo block leads — OPS-1, CNSL-1, ADPT-2,
CNSL-2, AUTH-4,5, EVAL-3, OPS-2 — and TEN-3..6, AUD-3,4 and the rest follow.

The product is installable since OPS-1: `synveda init` — see docs/INSTALL.md.
Since OPS-8 it is installable **by somebody else**: `curl … install.sh | sh`
puts a release's CLI, gateway binary, console and compose profile on a
machine whose only prerequisite is Docker (ADR-0065). It ships binaries as
well as images because ADR-0055 decision 8 forecloses the Docker-only shape
— the bundled issuer is a `localhost` URL and RFC 6761 makes that the
container itself — so the default install runs the gateway on the host.
`init` now finds its profile by explicit > checkout > installed bundle, so a
contributor's tree still wins. The release also carries the **Claude Code
plugin** as a marketplace — `synveda plugin install` drives `claude plugin`
rather than editing its state — which found that the plugin had **never
loaded** in Claude Code: `plugin.json` declared `hooks` (a duplicate load)
and an inline `mcpServers` (silently ignored), and `~/.claude/plugins/<name>/`
is not a path Claude Code reads. ADPT-1's demo could not see any of it
because it is its own harness (ADR-0027 amendment). The installer itself
touches nothing under `~/.claude` or any client's config. Not signed, not
notarized, no Windows, no upgrade path: reinstalling is how you upgrade, and
the release workflow is the one thing that can break without a red build to
say so.
Since AUTH-4 it syncs a directory: `/scim/v2` for Entra and Okta, with
`synveda scim token issue` for the credential (ADR-0059). Nothing has
replayed a frame from a live Entra or Okta tenant yet — the vendor corpus is
transcribed from their published tables.
Since TEN-4 it has a key plane: `SYNVEDA_KMS_KEY` (mint one with `synveda kms
keygen`) wraps a data key per tenant plus one for the deployment, and the
console session tokens, the outbound directory credential and `synveda tenant
export` are sealed under them (ADR-0064). Unset means `Kms::Disabled` — `/v1`
serves exactly as before and only the surfaces needing a key refuse. Two
things this does **not** do: `records`/`record_embeddings`/the Tantivy
sidecars are not sealed (there is no BM25 or HNSW over ciphertext — that is
the volume's job, decision 7), so destroying a tenant's key is not erasure;
and the KEK lives in deployment configuration, so this defends a dumped table
and a stolen archive rather than an operator who can read the environment.

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
