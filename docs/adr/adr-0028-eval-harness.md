# ADR-0028: The eval harness — an unprivileged client, scenarios as data, a committed baseline as the gate

- **Status**: Accepted
- **Date**: 2026-07-25
- **Feature(s)**: EVAL-1
- **Deciders**: sujitn

## Context

EVAL-1 is the last feature of the Phase-1 spine: "Rust runner + fixtures;
executes scenario suites against a live stack; CI-integrated with
regression gates on the five axes: accuracy, latency, tokens, recall,
abstention." It arrived with no acceptance criteria, so this ADR writes
them (recorded in SYNVEDA_FEATURES.md and docs/backlog/EVAL-1.md) as well
as deciding the shape.

Forces at play:

- **Evaluation is a functional requirement here, not a report.** The
  epic's own name says so. A memory product's failure mode is quiet:
  context that is subtly worse, a budget that silently truncates, a
  policy change that withholds material nobody notices is gone. Tests
  answer "did it break"; evals answer "did it get worse", and only a
  gate makes the second question consequential.
- **The stack already measures itself in five places, none of them
  joined.** `extraction_precision.rs` (per-class precision),
  `retrieval/tests/quality.rs` (recall@6 per leg),
  `inject_latency.rs` (1k sessions at 50/s), `retrieval/tests/latency.rs`,
  and `compose.rs`'s `tokens_per_inject` are all proto-evals. They guard
  their own crates well and were each explicitly deferred *forward* —
  STATUS points at EVAL-2, EVAL-4, and EVAL-6 at nine separate places for
  the targets. EVAL-1 is the skeleton those grow into, not their
  replacement.
- **An eval that can reach the store can cheat.** Seed §2.2 admits no
  code path from harness to storage that bypasses the PDP, and an eval
  is exactly a harness. A runner that links `synveda-store` can seed
  memory no policy allowed and read memory no chain permits — and would
  then report quality the product cannot deliver.
- **The suite must be cheap enough to run nightly.** The full dev
  compose includes Rauthy and TEI (a model download). A harness that
  needs an IdP to run is a harness that runs monthly.
- **PR CI is deliberately database-free.** `ci.yml` sets
  `SQLX_OFFLINE=true` and every Postgres-needing test skips without
  `DATABASE_URL`. That is a property worth keeping: it is why CI is
  fast.
- **Fixtures are the product of the later features.** EVAL-2 brings
  labelled transcripts, EVAL-3 brings LoCoMo/LongMemEval, EVAL-5 brings
  10k generated policy-leak variants. Whatever EVAL-1 chooses as a
  scenario format is what those three inherit.

## Decision

`crates/synveda-eval` is a **runner with no Synveda dependencies at all**
that drives a live stack over `/v1` only, executes **scenarios declared
as JSON data**, reports the five axes, and fails against a **committed
baseline**. `make eval` runs it locally; a nightly workflow runs it
against the dev compose; PR CI is untouched.

Decisions, specifically:

1. **A separate crate whose allowed dependency set is empty.**
   `check-crate-deps.mjs` gets `"synveda-eval": []` — the same standing
   the seed gives adapters and SDKs ("depend only on the public API").
   The runner defines its own wire structs and speaks HTTP. This is not
   tidiness: it is the only way to make "the eval cannot bypass the PDP"
   a property of the build rather than a promise in a comment. It also
   means the harness can be pointed at a deployed gateway it has no
   source access to, which is what an eval of a *deployment* requires.
2. **Scenarios are data.** `evals/scenarios/*.json`: named actors, a
   seed phase, a probe, and expectations. Adding a scenario is not a code
   change, which is what lets EVAL-2/4/5 grow the suite by adding files.
   The runner validates the format and refuses an unknown field rather
   than ignoring it — a silently-ignored expectation is an eval that
   passes for the wrong reason.
3. **Memory is seeded through the product, never behind it.** The seed
   phase posts `/v1/observe` batches as the owning actor and waits for
   the pipeline (MEM-1 → redaction → extraction → embedding) to make them
   records. It is slower than an `INSERT` and that is the point: an eval
   that seeds behind the API measures a system nobody runs. The wait is
   bounded and a scenario whose seed never lands fails as a scenario, not
   as a crash.
4. **Five axes, each with a direction.** `accuracy` (every
   `must_contain` present, no `must_not_contain`), `recall` (fraction of
   the scenario's expected records that reached the block), `abstention`
   (of the scenarios that must compose nothing, the fraction that did),
   `tokens` (the block's own count), `latency` (probe wall clock,
   reported as median and p95). The first three are higher-better, the
   last two lower-better, and the gate knows which is which. Abstention
   is first-class because a memory system that invents context is worse
   than one that stays quiet, and no ordinary test asserts that.
5. **The gate is a committed baseline.** `evals/baseline.json` holds a
   bound per axis; the runner compares, and a breach fails the run naming
   the axis, the baseline, the measurement, and the delta. `--update-baseline`
   rewrites the file so a deliberate change is a reviewable diff, not a
   number edited in code. A baseline nobody can see move is not a
   baseline.
6. **Actors are dev-mode identities.** HS256 dev bearers (ADR-0008) for
   service identities registered at fixed scopes (AUTH-3), so the harness
   needs Postgres and the gateway and nothing else — no IdP, no model
   server. The OIDC path is AUTH-1's and ADPT-1's to prove, and both do.
   The deterministic extractor and embedder are the default for the same
   reason: a nightly failure should mean someone changed the code, not
   that a model drifted.
7. **Bootstrap is a script, the runner is a client.** `evals/bootstrap.sh`
   admits a fresh tenant, builds the hierarchy, registers the actors, and
   prints the environment as JSON; the runner consumes it. The privileged
   half stays in the same shell idiom every demo already uses, and the
   Rust half stays a thing that only knows how to call two endpoints.
   A fresh tenant per run is what makes two runs comparable.
8. **`make eval` locally, nightly in CI, PR CI untouched.** The nightly
   workflow brings up Postgres from the dev compose, runs the suite, and
   fails on the gate — the same precedent EVAL-5 is already specified
   with ("AC: nightly"). Keeping it off the PR path preserves the
   database-free CI that makes pull requests fast, and the trade is
   explicit: a regression is caught within a day, not within a merge.
9. **EVAL-1 owns no targets.** The baseline it ships covers only what its
   thin suite measures. Extraction precision stays EVAL-2's, retrieval
   and injection quality EVAL-4's, the zero-tolerance security gate
   EVAL-5's, percentile SLOs under load EVAL-6's. What EVAL-1 fixes is
   the *shape* those arrive in.

## Options considered

1. **`synveda eval` as a CLI subcommand** — one binary, credentials and
   bootstrap already in-process, no new crate. Rejected on decision 1:
   the CLI links store, identity, policy, and audit because its
   dev-bootstrap commands must run when no gateway does. An eval that
   inherits those can seed and read around the PDP, and the one thing an
   eval must never do is measure a path the product does not have.
2. **Keep the proto-evals as `#[ignore]`d integration tests and add a
   gate to them** — no new crate, no new format, and the fixtures already
   exist. Rejected: they link crate internals (that is why they are
   good unit-level guards), they cannot be pointed at a deployed stack,
   and `cargo test` has no vocabulary for "worse than last time".
3. **A Python or k6 harness** — the ecosystem where eval tooling lives,
   and k6 is what EVAL-6 will want for load shapes. Rejected for the
   skeleton: another runtime in the core path for a runner that makes two
   HTTP calls, against a repo that is Rust plus one TypeScript adapter by
   design. EVAL-6 may still bring k6 for load profiles specifically.
4. **Seed memory with direct inserts** — fast, deterministic, no pipeline
   wait. Rejected per decision 3; the pipeline is part of what is being
   evaluated, and CTX-3's demo already shows how long it takes to notice
   when it is not.
5. **Gate on every PR with compose services** — the strongest gate, and
   what EVAL-4's "before merge" AC will eventually want. Rejected for now
   per decision 8: it makes every pull request depend on Postgres, and
   EVAL-1's thin suite is not yet worth that toll. Reversal trigger:
   EVAL-4 lands with composition-quality scenarios, whose whole point is
   to block a merge.

## Consequences

- Positive: the five axes exist in one place with one vocabulary, and a
  regression in any of them is a failure with a name and a number. The
  harness is architecturally incapable of measuring a path the product
  does not expose, and can be pointed at any deployment. Scenarios are
  files, so the later EVAL features add coverage without touching the
  runner. Phase 1 closes with the loop that proves it: observe → extract
  → embed → inject, measured.
- Negative / accepted trade-offs: a nightly gate catches a regression
  within a day rather than before a merge; the suite needs a live stack,
  so it cannot run in the PR job that everything else runs in; seeding
  through the pipeline makes a run take seconds per scenario rather than
  milliseconds; the runner duplicates a handful of wire structs the
  gateway also defines, which is the price of the empty dependency set
  (and the same price the TypeScript adapter already pays); and dev-mode
  bearers mean the harness exercises the HS256 auth path, not OIDC.
- Reversal triggers: EVAL-4's composition scenarios land → move the gate
  onto the PR path (option 5); the duplicated wire structs drift from the
  gateway's → publish them from ADPT-3's OpenAPI instead of hand-writing
  them; scenario JSON outgrows what a human can read → a scenario DSL,
  not a bigger JSON.

## Compliance notes

- The PDP stays unbypassable, structurally: the runner has no Synveda
  crate dependencies, holds no service identity of its own, and reaches
  the stack only through `/v1` with an actor's bearer. Every measurement
  it reports is a measurement of the governed path (seed §2.2).
- Tenancy: the bootstrap admits a fresh tenant per run and every actor
  is an identity inside it; the runner never names a tenant, because the
  token does (ADR-0008).
- Audit: no new action types. The scenarios drive `/v1/observe` and
  `/v1/inject`, which chain `memory.observed` and `context.injected`
  already; an eval run is indistinguishable in the chain from the
  sessions it imitates, which is correct — it *is* those sessions.
- Secrets: fixtures are documentation-only content under the same
  discipline as the redaction and extraction fixtures — no credentials,
  real or synthetic-but-live-format, ever.
- Observability (DoD #3): the runner is a client and emits its own
  timings into the report rather than into the stack's telemetry; the
  gateway spans for its calls are the ordinary inject/observe spans.
