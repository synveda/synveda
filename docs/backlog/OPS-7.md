---
title: "OPS-7: Gateway horizontal scale"
labels:
  - epic:OPS
  - phase:4
size: L
---

# OPS-7: Gateway horizontal scale

**Epic:** OPS — Deployment & operations · **Phase:** 4 · **Size:** L

## Problem and evidence

Helm pins one gateway replica and Recreate. Pending OIDC login/CLI handoff
state is process-local in crates/synveda-identity/src/flow.rs, and PDP entity
invalidation is process-local in the gateway. Capture, Knowledge indexing,
directory pull and relaxation expiry now run in a separate supervised core
worker, but only one worker replica is supported and concurrent-worker recovery
has not been proved. Worker SIGTERM withdraws readiness and performs a bounded
cancel/join, while gateway readiness still remains true during graceful request
shutdown. Claimed-work termination remains unproved. These gaps are recorded in
[production readiness](../PRODUCTION_READINESS.md) and the one-replica refusal
is governed by [ADR-0062](../adr/adr-0062-enterprise-profile-and-helm-chart.md).

## Scope

- Persist one-time login state and CLI handoff redemption with database time,
  TTL and atomic consume semantics.
- Propagate scope, grant, policy and entity invalidation across replicas within
  a stated bound.
- Prove every core-worker job's multi-replica ownership, lease, idempotency and
  provider-concurrency behaviour before lifting the one-worker limit.
- Withdraw gateway readiness on termination, drain requests, and prove that
  core workers finish or safely release claimed work, flush telemetry and exit
  within a bounded grace period.
- Lift the chart refusal only after a three-replica acceptance passes.

## Non-goals

- No distributed cache, second queue product or orchestration framework merely
  to lift the replica limits.
- No weakening of fresh PDP decisions, forced RLS or live re-authorisation on
  idempotent replay.
- No claim that CloudNativePG replication makes the request plane available.
- No unbounded sticky-session dependency presented as high availability.

## Architecture seam

Login state belongs beside durable console sessions in synveda-store. Scope
and grant mutations already invalidate local PDP entities; add one
cross-process generation or notification contract rather than a second
authorisation cache. Existing durable batch/job tables remain the worker seam.
Gateway readiness owns request drain. The core-worker supervisor owns task
cancellation and bounded join; each durable aggregate owns its claim/recovery
semantics. Helm owns the separate replica and termination settings.

## Acceptance criteria

- Three replicas complete a login begun on one pod and callback/handoff on
  another; the same state/code cannot be redeemed twice.
- A scope move, grant revoke, identity disable and Configuration/policy change
  become visible to every replica within the documented bound, with no stale
  permit after the bound.
- Three gateway replicas and separately scaled core-worker replicas produce one
  durable effect, respect provider concurrency and recover every lease after
  pod loss.
- SIGTERM makes the affected process's readiness fail first, drains gateway
  requests, and lets core workers finish or safely release claimed work before
  exiting inside the pod grace period.
- Sustained traffic loses a pod and a rolling upgrade without incorrect
  decisions or avoidable login failure.
- The chart accepts only tested replica counts and no longer requires Recreate.

## Required tests

- Three-pod kind acceptance with cross-pod login and direct per-pod requests.
- Mutation/invalidation latency tests for every authority-changing family.
- Worker race, lost-ack, lease-expiry, provider-outage and pod-kill tests.
- Connection-pool/load/soak evidence at the maximum supported replica count.
- Exact shutdown subprocess and Kubernetes termination-sequence tests.

## Rollout and rollback

Ship durable login and invalidation while still pinned to one replica, observe
lag, then canary two and three replicas. Retain one-replica/Recreate as the
rollback until the full acceptance is stable. Rollback must leave durable
state readable and may reduce replicas without discarding jobs.

## Dependencies

The owner must define availability, invalidation staleness, drain and provider-
concurrency limits. Choose a Postgres generation/notification mechanism and
worker-leadership model in an ADR. OPS-5 and OPS-6 provide recovery and upgrade
discipline.
