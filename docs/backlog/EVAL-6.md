# EVAL-6: Load & latency suite

## Problem and evidence

`crates/synveda-gateway/tests/session_ingest_load.rs` proves a focused session-event append rate and gates only its median acknowledgement latency; it explicitly leaves percentile-complete SLO evidence to this feature. There is no current mixed-workload, noisy-neighbour, or soak report for the epoch-3 session, Knowledge, context-planning, capture, and VedaFlow paths. Passing functional CI therefore does not establish a production capacity envelope.

### Current state (2026-10-28)

The harness is implemented and source-tested; the [capacity envelope](../CAPACITY.md)
holds the proposal and measured tables, and [the operator guide](../../evals/load/README.md)
the commands.

- `synveda-eval load` drives seeded open-loop schedules for agents in a
  measured and a second tenant through the public API: Session open, append,
  end and Capture; candidate acceptance; ContextRuns; Knowledge search;
  private Knowledge create and edit through two named approvals and a
  distinct publisher; a stale reviewed edit that must be refused at apply;
  and cross-tenant Session reads that must be refused. Latency is measured
  from the scheduled time, so waiting behind a slow call is charged rather
  than omitted, alongside service time, generator lag and unsent calls.
- Reports are content-free and checkpointed per window with HDR histograms
  saved for resume; each window carries gateway and worker Prometheus
  deltas (HTTP by route, admission wait, pool acquire, planner stages,
  Capture queue wait, extractor time, indexing lag), pool and slot gauges,
  stub-provider service time, process RSS/CPU and database size and
  connections. Three bounded signals were added to the product for this:
  `synveda_db_pool_acquire_seconds`, `synveda_db_pool_connections{state}`
  and `synveda_knowledge_index_lag_seconds`.
- `evals/load.sh` runs a profile on a fresh exact-role fixture with release
  builds, the reference 8-connection pools, a loopback 1024-dimension stub
  provider whose delay and error share each phase sets, a database CPU
  fault hook and a database sampler. `synveda-eval load-check` refuses a
  report with a missing axis, metric family or profile mismatch, and any
  baseline that differs from its reviewed ledger entry.
- Profiles: `smoke` (nightly Extended Tests job), `first-offer-1x`,
  `first-offer-10x` and `soak-24h`. The 24-hour soak has not been run.
- Measured on one M5 Max host ([evidence](../../demos/evidence/eval6-load-capacity.json)):
  at 1x, appends p95 25–28 ms and search inside budget, ContextRun p95 86 ms
  at baseline rising to 196 ms as data accumulated (p99 179–229 ms against a
  150 ms budget), isolation held in every phase, process memory flat and
  storage about 3.9 GB/day with no purge; a 20-fold second tenant pushed
  the measured tenant's ContextRun p99 to 0.95 s in one run and 7.4 s in
  another (pool acquire p99 16 ms against 1.7 s), so tails under
  contention did not reproduce on this host; provider slowdown, stall and outage degraded to the deadline or to
  lexical and indexing caught up afterwards. At 10x the pool saturated:
  9% of appends refused with `429`, append p99 5.2 s, ContextRun p99 7.1 s,
  gateway CPU about 0.18 cores.

Remaining before this feature closes: owner approval of the proposed
envelope and budgets in [the capacity page](../CAPACITY.md); the unrun soak
(`make eval-load LOAD_PROFILE=soak-24h EVAL_LOAD_REPORT=<path>` on a host
that stays awake about 25 hours with roughly 30 GB free; it must show no
unbounded RSS, connection, queue-age or indexing-lag trend and a storage
slope explained by retention, which is currently none); a first reviewed
baseline in `evals/load/baselines/ledger.json` from an observation run on
production-shaped Linux hardware; the same profiles against the Compose
reference images with RS256/JWKS bearers and a live BGE-M3 provider; and a
decision on cross-class or per-tenant process concurrency, because the
measured tenant's tail degrades under a 20-fold noisy tenant.

## Scope

- Build a repeatable public-API workload covering session open, event append, context runs, Knowledge query, session end, capture, and candidate acceptance.
- Exercise multiple tenants and scope shapes at an owner-declared small-team load and at 10x that load, with fixed corpus, embedding model, database, gateway, and hardware metadata.
- Report throughput and p50/p95/p99 latency together with pool wait, error rate, capture queue age, indexing lag, CPU, memory, database size, and audit/storage growth.
- Run burst, steady-state, noisy-neighbour, database/provider fault, recovery, and 24-hour soak profiles. Keep fault injection bounded and deterministic.
- Commit an immutable reviewed baseline and a machine-readable report whose thresholds fail the release gate without rewriting prior evidence.

## Non-goals

- Replacing focused correctness, policy, RLS, or adapter conformance tests.
- Claiming multi-replica capacity before the deployment shape has multi-replica evidence.
- Treating a developer laptop result, skipped dependency, or model-provider outage as a production SLO.
- Weakening thresholds or updating a baseline solely to make a regression pass.

## Architecture seam

Drive only the versioned public API with ordinary OIDC user and service identities. Place workload generation and reports under `evals/` or `demos/`; production crates expose only bounded-cardinality metrics needed to observe the existing paths. Use the executable route catalogue as the operation inventory, and keep content out of metrics and audit assertions.

## Acceptance criteria

- The declared workload meets its reviewed throughput and p95/p99 budgets at 1x and records an honest result at 10x.
- A 24-hour run has no unbounded memory, connection, queue-age, index-lag, or storage-growth trend; any bounded slope has a documented retention explanation.
- One tenant's overload does not let another tenant cross the declared latency/error budget or bypass PDP, RLS, or audit.
- Database and provider faults produce causal, non-secret errors, bounded backlogs, and measured recovery without duplicate governed effects.
- Every report names the commit, configuration, corpus, model, deployment topology, hardware, sample count, and unavailable prerequisite.

## Required tests

- Deterministic smoke profile suitable for CI, including metric-name and report-schema assertions.
- Database-backed 1x/10x burst and steady-state profiles with percentile assertions.
- Noisy-neighbour isolation and database/provider fault-and-recovery tests.
- Twenty-four-hour soak runner with leak/slope checks and a resumable, content-free report.
- Regression check that refuses missing axes, configuration drift, or an unreviewed baseline rewrite.

## Rollout and rollback

Land the harness and non-gating observation run first, review the environment and thresholds, then promote the stable profile to the release gate. Roll back a bad harness or threshold by reverting that change while retaining the last accepted report; do not erase a product failure. Load-only credentials, tenants, and data must be isolated and disposable.

## Dependencies

CTX-7 must settle dense-plan behaviour before dense latency is used as a stable claim. The owner must approve the workload mix, session/event/corpus/retention shape, hardware topology, team-size definition, and SLO budgets. Multi-replica claims additionally depend on the corresponding deployment-readiness evidence and available database/model services.
