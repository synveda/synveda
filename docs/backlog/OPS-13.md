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

Capture already has database-time leases, an incremented attempt fence, a
five-attempt terminal bound and one durable candidate set after a reclaimed
claim. One combined worker plus at most two capture-only pods passed one-node
Kind pod-loss and provider-503 drills under OPS-7/ADR-0129. These protect
database effects, but do not make external provider calls exactly once.

`capture::fail_batch` returns a failed nonterminal attempt directly to
`pending`. `capture::claim_batch` selects the oldest pending batch without a
next-attempt time, while the worker polls each second and may process eight
batches per tenant per sweep. A repeatedly failing provider can therefore
consume its five attempts rapidly; another pod can make the same retry after
restart. There is no inspectable retry schedule, queue-age SLO, manual retry
contract or provider-wide concurrency quota. The gap is PR-05 in
[production readiness](../PRODUCTION_READINESS.md).

## Scope

- Persist a database-time `next_attempt_at` for Capture and make claims skip
  work whose retry time has not arrived. Keep the existing attempt/owner fence
  and terminal failure state authoritative.
- Use bounded exponential backoff with jitter stable across worker restarts;
  distinguish retryable provider failures from permanent policy, validation
  and configuration refusals with closed error codes.
- Expose tenant-safe queue age, retry age, scheduled/terminal counts and
  provider-call saturation without tenant or principal hot metric labels.
- Add a provider-wide concurrency bound for the qualified deployment shape,
  including the combined worker and capture-only pods. Reserve provider
  capacity before raising the chart's pod count.
- Provide an inspectable, Cedar-governed manual retry of terminal work only
  after the owner selects its permission, age and audit policy.

## Non-goals

- No Temporal, PGMQ authority or second product queue. Capture rows remain
  the source of truth; optional Apalis cannot decide product retries.
- No promise of exactly-once external provider effects. The fence guarantees
  one durable candidate set, not one HTTP call.
- No automatic rollout of this retry model to Knowledge indexing, directory
  pull or Skill validation; each has a different effect and ownership model.
- No unbounded, operator-triggered replay of failed batches.

## Architecture seam

The next forward migration adds retry eligibility to the existing
`capture_batches` row and its static SQLx queries in `synveda-store`. The
tenant-scoped claim transaction chooses by database time under forced RLS;
completion and failure continue to use the exact attempt/owner lease fence.
The ingest worker classifies stable failures and schedules a bounded delay
without carrying provider error text into rows, metrics or audit.

Before implementing the provider quota or manual retry, record an ADR for the
slot ownership and expiry protocol and for the governed retry command. The
current three-pod chart bound is a deployment ceiling, not a provider-wide
quota shared with other processes or products.

## Acceptance criteria

- Two workers repeatedly seeing a retryable failure cannot reclaim the same
  batch before its stored deadline, even across restarts or clock skew.
  Backoff is bounded, deterministic for the durable batch/attempt and has no
  retry storm; an exhausted batch becomes terminal with content-free audit.
- A permanent failure closes without spending all five provider attempts;
  operator inspection names its stable state/code without credential or source
  content.
- The selected provider quota holds across the combined and capture-only
  workers through crash, lease expiry and process restart. Provider 429/503
  and network timeout do not exceed that quota.
- Queue/retry age and saturation are observable; representative failure load
  meets an owner-selected maximum retry age and queue-age objective.
- Manual retry, if offered, is idempotent, Cedar-decided, VedaFlow-governed
  where the mutation changes reviewable state, and content-free-audited.

## Required tests

- Exact-role PostgreSQL tests for due/early claims, two-worker races, restart,
  terminal transition, database-time scheduling and stale-claim refusal.
- Provider stub tests for transient/permanent classification, jitter bounds,
  429/503/timeout behavior and no credential or source text in evidence.
- Two- and three-worker deployment drills with blocked provider calls,
  process loss, retry recovery and one fenced candidate set.
- Loaded queue-age and provider-concurrency test at the declared maximum;
  alert and operator recovery exercise for an exhausted batch.

## Rollout and rollback

Ship the additive retry column and old-reader-safe query contract before
enabling delayed claims. Shadow-record proposed deadlines and queue age first;
then enable scheduling for one Capture profile under a reversible deployment
setting. A rollback must leave every pending and terminal batch visible and
reclaimable by a compatible release; it must not reset attempt fences or
erase failure evidence. Keep the one-combined/two-capture-only chart ceiling
until quota and load acceptance pass.

## Dependencies

The owner must choose provider capacity across all Synveda installations
sharing credentials, retry age/count and jitter bounds, which errors are
permanent, queue-age SLO and manual-retry authority/retention. OPS-6 owns the
published N-1/N schema compatibility window; OPS-5 owns recoverability. An
accepted ADR is required before new retry or quota state is implemented.

Next: choose the retry and provider quota contract, record the state/slot
protocol in an ADR, then add the forward migration and exact-role claim tests
before enabling scheduling in the worker.
