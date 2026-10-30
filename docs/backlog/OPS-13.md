---
title: "OPS-13: Capture retry and provider backpressure"
labels:
  - epic:OPS
  - phase:4
size: L
---

# OPS-13: Capture retry and provider backpressure

**Epic:** OPS — Deployment & operations · **Phase:** 4 · **Size:** L

## Problem and evidence

Before this slice, `capture::fail_batch` returned a failed nonterminal attempt
straight to `pending` and `capture::claim_batch` chose the oldest pending batch
without a deadline. With a one-second poll and eight claims per tenant per
sweep, an outage could spend all five attempts in seconds, every worker
process could call the provider at once, and policy or validation failures
were retried like outages. A terminal batch had no recovery: re-freezing the
same session snapshot replays the failed row. The gap is PR-05 in
[production readiness](../PRODUCTION_READINESS.md).

[ADR-0158](../adr/adr-0158-durable-capture-backoff-and-provider-slots.md) now
records the implemented source contract:

- Migration `0016`, byte-identical in both lineages, adds `next_attempt_at`,
  `terminal_reason`, `manual_retries` and `manual_retried_at` to
  `capture_batches`, replaces the attempt bound with `5 × (manual_retries + 1)`,
  makes the attempt counter monotonic in the transition trigger, admits
  `failed → pending` only as one bounded manual retry, adds due/running partial
  indexes and creates the deployment-scoped `capture_provider_slots` table.
  Measured catalogue fingerprints and regenerated SQLx metadata are pinned.
- Claims use database time only and prefer an expired running claim; the
  plan was checked to range-scan the due index and to lock nothing in the
  pending branch when an expired claim exists.
- Extractors return a closed `ProviderFailure`; only timeouts, connection
  loss, 429, 503/529 and 408/500/502/504 retry. Every other worker failure
  closes after one attempt with a stable code. `Retry-After` (seconds or
  HTTP-date) is admitted only for 429/503/529, capped at one hour and then at
  the policy's maximum delay.
- Backoff is `min(max_delay, base × 4^(n−1)) × [0.5, 1.5)` with BLAKE3 jitter
  keyed by batch and attempt, bounded by five attempts and a cycle age.
- A worker reserves an installation-wide provider slot in its claim
  transaction; saturation rolls the claim back without spending an attempt.
  Slots renew with the lease, release in the terminal transaction and expire
  in database time after a crash.
- `POST /v1/capture-batches/{id}/retry` and
  `synveda session retry-capture` reopen a failed session batch under
  `SessionWrite`, Idempotency-Key replay, source and exact-Configuration
  revalidation, two manual cycles within seven days and content-free
  `capture.batch.retry_requested` audit.
- The batch view, console Operations rows, OpenAPI, console client and SDK
  contract expose `next_attempt_at`, `terminal_reason`, `manual_retries`,
  `manual_retried_at` and `max_attempts`. New closed-label metrics cover
  queue wait, retry-cycle age, retry/terminal/saturation outcomes and slot
  use.

Evidence on 2026-10-26, source tree after v0.4.7 (not a release):

- Exact-role suite `capture_retry` (five cases) passed under the database
  gate: two workers, a restarted worker and a worker clock an hour ahead
  could not claim before the stored deadline; the deadline equalled the
  deterministic delay; two racing workers then claimed once and committed one
  candidate. 401, 400 and malformed output closed after one call each with
  `terminal_reason=permanent` and no provider body in audit. A hostile
  `Retry-After` was capped at the ten-second test maximum and an over-age cycle
  closed as `retry_age_exhausted`. Five 503s closed as `attempts_exhausted`;
  the runtime role could neither reset the fence nor reopen the row; the
  outsider got 403, a foreign tenant 404, a missing key 400; one manual retry
  reopened at attempt 5 of 10, replayed idempotently, refused a second key,
  recovered with one candidate and was audited; two manual cycles were then
  refused, and a Configuration change refused retry as stale. A limit of one
  rolled back a saturated claim with zero attempts spent, recovered a slot
  after a cancelled holder's lease and slot expired, and committed one
  candidate per batch with a maximum of one call in flight.
- A local three-process drill ran one combined and two Capture-only
  `synveda-worker` processes under the exact worker login with a ceiling of
  two. Maximum in-flight calls stayed at two; the SIGKILLed Capture-only
  owner's slot was refilled 3.962 s later (four-second lease); only that
  owner's batch was called twice; 503 and 429 (`Retry-After: 1`) recovered
  1.091 s and 1.094 s after first claim and a 10-second client timeout in
  10.867 s; all six batches held one candidate and finished 15.828 s after
  readiness; survivors drained on SIGTERM. Retained in
  [the source acceptance record](../../demos/evidence/ops13-capture-backpressure-source-acceptance.json).
- A fresh, disposable one-node Kind cluster installed from this source by
  `demos/ops-2-helm-install.sh` created its database at head `0016` and passed
  that demo's login, round trip, audit and CNPG failover checks. Against it,
  `OPS7_CLUSTER=synveda-ops13 demos/ops-7-worker-recovery.sh
  capture-backpressure` ran three gateway pods, one combined and two
  Capture-only worker pods with a ceiling of two: two of three batches ran,
  the third waited with no attempt spent and a worker counted saturation. The
  slot-holding pod was force-deleted; its slot was refilled 60 s later (one
  lease) with never more than two calls in flight. All three batches
  completed once (attempts `1,1,2`, four provider calls, one cancelled); a 429
  with `Retry-After: 1` and a 503 each stored a deadline and retried once. The
  script restored the one-gateway, one-worker release and the cluster was
  deleted. Two earlier runs used graceful deletion and failed the drill's own
  checks: a terminating pod drains for up to `worker.shutdownSeconds` with its
  call and slot live, and the 45 s client timeout then frees slots as
  ordinary retryable failures before any lease expires. The mode now deletes
  crash-style and sets `extractor.timeoutSeconds=120`.
- Extractor HTTP tests cover every status class, transport loss, a client
  timeout and hint capping; unit tests cover jitter stability, delay bounds,
  policy ordering and refusal rules.
- The full exact-role gate passed on the final tree: 131 binaries, 2,147
  passed, 0 failed, 36 ignored, including both 0015-to-0016 lineage upgrades
  and the drill in its serial phase (ceiling held; lost owner's slot refilled
  in 4.463 s).
- The PostgreSQL 14, 15, 16 and 18 compatibility tasks
  (`SYNVEDA_DB_TEST_TASK=postgres-<major>`) passed on official 14.24, 15.19,
  16.15 and 18.6 servers provisioned through the DBA setup script, each
  running `capture_api` and `capture_retry`, including the three-process
  drill, under the exact roles.
- A claim-path probe (300 completed batches, deterministic extractor) ran
  three alternating rounds against this tree and the `3ebabb66` baseline.
  Idle-poll p95 was 1.28–1.56 ms against 1.20–1.56 ms; claim-to-completion
  varied more between rounds of one tree (baseline p50 19–50 ms) than between
  trees. Idle polls and deterministic claims issue the same statements as
  before; a provider-bound claim adds two to its claim transaction, one per
  renewal and one release.

Hosted CI on 2026-10-30: the drill failed 3 of 17 database-acceptance job
attempts, one of them a rerun, with two blocked batches called twice instead
of one. A blocked survivor must hold its call through saturation, owner loss,
lease expiry and refill. In passing runs that took up to 10.0 s against the
drill's 10-second client timeout; when it ran longer, the survivor's call
timed out and retried. A local run with a 3-second timeout reproduced the same
assertion twice. The drill now uses a 30-second client timeout and reports
per-batch calls and timings when that assertion fails. Locally it then passed
three idle runs and three with every core saturated, each about 20 s longer
because the stalled call now waits 30 s. That load stretched the survivor only
to 6.0 s, so no local run reached the CI duration. If the drill fails again
with every survivor well under 30 s, suspect lease renewal, whose timeout is a
third of the four-second lease. Not run: the PostgreSQL compatibility tasks,
which also run the drill.

## Scope

- Owner adoption or replacement of the provisional values: 30 s base,
  30 min maximum delay, 2 h cycle age, five attempts per cycle, provider
  concurrency 3, two manual retries within seven days and `SessionWrite`
  authority.
- A loaded queue-age and retry-age objective with alert thresholds at the
  declared maximum.

## Non-goals

- No Temporal, PGMQ authority or second product queue. Capture rows remain
  the source of truth; optional Apalis cannot decide product retries.
- No promise of exactly-once external provider effects. The fence guarantees
  one durable candidate set, not one HTTP call.
- No automatic rollout of this retry model to Knowledge indexing, directory
  pull or Skill validation; each has a different effect and ownership model.
- No coordination of provider capacity across separate installations that
  share an account.
- No unbounded, operator-triggered replay of failed batches.

## Architecture seam

ADR-0158 owns the schedule, classification, slot and manual-retry protocol.
`synveda-store::capture` holds the static SQL for claim, fail, reopen and
slot reservation; `synveda-ingest::retry` is the pure schedule and
`Retry-After` parser; `synveda-ingest::capture_worker` classifies failures,
reserves capacity in the claim transaction and records metrics;
`synveda-gateway::capture` owns the governed retry route. Helm renders
`worker.capture.*` and `extractor.timeoutSeconds` into every worker pod. The
chart keeps one combined worker and at most two Capture-only pods.

## Acceptance criteria

- Met in source: two workers repeatedly seeing a retryable failure cannot
  reclaim before the stored deadline, across restart and a skewed worker
  clock; backoff is bounded and deterministic; exhausted batches close with
  content-free audit.
- Met in source: permanent failures close after one provider call with a
  stable code and terminal reason, without credential or source content.
- Met locally and on one-node Kind: the provider ceiling holds across one
  combined and two Capture-only workers through owner loss, lease expiry, 429
  and 503 (locally also a client timeout). Open: multi-node and loaded runs.
- Partly met: queue-wait, retry-cycle, outcome and saturation signals exist.
  Open: an owner-selected queue-age objective and alert thresholds under
  representative load.
- Met in source: manual retry is idempotent, Cedar-decided, revalidating,
  bounded and audited; it changes no reviewable state, so it is not a
  VedaFlow change.

## Required tests

- Done: exact-role cases for due/early claims, two-worker races, restart,
  skewed worker clock, terminal transitions, retry-age and attempt
  exhaustion, saturation rollback, slot expiry recovery, fence and reopen
  refusal, manual retry authority, replay, bounds and staleness.
- Done: provider stub tests for every status class, transport loss, timeout,
  `Retry-After` parsing and capping, jitter bounds and content-free evidence.
- Done locally and on one-node source-image Kind: three-worker drill with
  blocked calls, process loss, retry recovery and one candidate per batch.
- Open: loaded queue-age and provider-concurrency run at the declared
  maximum with alert and operator exercise for an exhausted batch.

## Rollout and rollback

`0016` is additive and forward-only; an older binary refuses the new head, so
rollback is the restore-based path of ADR-0156. Pending rows from before the
migration are due immediately; failed rows keep their codes and gain no
terminal reason. Set `worker.capture.*` identically on every worker; lowering
`providerConcurrency` stops new reservations above the limit at once while
existing holders finish or expire. Keep the one-combined/two-Capture-only
chart ceiling until load acceptance passes.

## Dependencies

The owner must adopt or replace the provisional retry, capacity and
manual-retry values and choose the queue-age objective; capacity shared with
other installations needs its own decision. OPS-6 owns the published N-1/N
compatibility window and OPS-5 recoverability. Next: the owner adopts or
replaces the provisional values and names a queue-age objective, then
measure queue age at the declared maximum against it.
