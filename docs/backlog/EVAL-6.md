# EVAL-6: Load & latency suite

## Problem and evidence

`crates/synveda-gateway/tests/session_ingest_load.rs` proves a focused session-event append rate and gates only its median acknowledgement latency; it explicitly leaves percentile-complete SLO evidence to this feature. There is no current mixed-workload, noisy-neighbour, or soak report for the epoch-3 session, Knowledge, context-planning, capture, and VedaFlow paths. Passing functional CI therefore does not establish a production capacity envelope.

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
