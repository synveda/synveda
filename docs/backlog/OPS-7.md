---
title: "OPS-7: Gateway horizontal scale"
labels:
  - epic:OPS
  - phase:4
size: L
---

# OPS-7: Gateway horizontal scale

**Epic:** OPS — Deployment & operations · **Phase:** 4 · **Size:** L

## Description

More than one gateway replica, serving one deployment, without a login failing
or a scope chain going stale. OPS-2's chart pins `replicas: 1` and refuses an
override until this lands.

## Why this exists

Filed 2026-08-10 by OPS-2 (ADR-0062 decision 5), which found it by reading the
seven background loops and three caches inside the process a chart was about to
replicate.

The question "is the gateway HA?" has a more useful answer than yes or no, and
the useful answer is what makes this a feature rather than a warning in a values
file.

## What is already safe, and why that is the point

Most of the concurrency work was done years earlier than it needed to be, in the
database, on purpose:

- The **audit chain** appends inside the caller's tenant transaction after
  `select seq, head_hash from audit_chain_heads where tenant_id = $1 for update`
  (`synveda-audit/src/chain.rs`). Two processes appending for one tenant
  serialize; they do not fork the chain.
- The **promotion sweep** takes `watermark_for_update` with the reason written
  beside it: "two sweepers that both acted on the same watermark would fold the
  same events twice."
- The **lapse expiry sweep** uses its stamp as an idempotency key — "two
  overlapping sweeps cannot chain one expiry twice, and the loser simply finds
  nothing to update rather than writing a duplicate event."
- The **extraction worker** is a PGMQ consumer whose `pgmq.archive` runs inside
  the tenant write transaction (ADR-0022), so racing consumers cannot duplicate
  a record.
- **Console sessions** are a table (migration `0034_console_sessions.sql`), not
  a map.
- The **search indexer** is per-process by design and converges: each index
  carries a state file and a watermark and heals from Postgres
  (`synveda-retrieval/src/index.rs`).

So this feature is not "make the gateway stateless". It is two specific pieces
of process-local state, and a ruling on load.

## The two blockers

1. **Login state lives in memory.** `LoginFlow` parks pending logins (state,
   nonce, PKCE verifier) and CLI handoff codes in a bounded in-memory store with
   a 10-minute and a 60-second TTL, and its module doc says the consequence out
   loud: "single-replica only until OPS-2 (ADR-0010)." A `/auth/callback` that
   lands on a different pod than the `/auth/login` that minted the `state` is a
   401 for a login the IdP completed — a failure that reads as an IdP problem.

2. **Scope-chain invalidation is process-local.** `ScopeChainCache` is
   read-through per `(tenant, scope)` and invalidated tenant-wide, post-commit,
   by the handler that performed the mutation (ADR-0016 decision 5). There is no
   TTL and no eviction anywhere in `synveda-store/src/scope_chain.rs`. A
   hierarchy move handled by one replica therefore leaves every other replica
   composing against the ancestry the mover has left — indefinitely, and in the
   direction that matters. It does not look like a bug: the material returned is
   material a real ancestry once permitted, so it reads as a policy decision.

## Three parts

- **A durable login and handoff store.** Move both to Postgres beside the
  console sessions already there, with a TTL sweep. The handoff code is
  one-time, state-bound and 60 seconds (ADR-0027 decision 5) — its single-use
  property has to survive being in a table read by several processes, which is
  the one part of this that is a design rather than a move.
- **Cross-process invalidation.** LISTEN/NOTIFY, or a generation column polled
  beside the pack refresher that already polls every 5s. The choice is a
  latency-versus-transport question and wants its own reversal trigger: a
  polled generation has a bounded staleness window that must be stated, and
  NOTIFY does not survive a connection the pool recycles unless it is held.
- **A loop-ownership ruling.** The five writing loops are safe on every replica.
  N replicas is still N times the sweep load on one database, and a directory
  pull is N calls to a customer's Entra tenant per interval, which is a rate
  limit rather than a correctness question. Either bound the loops to a leader
  (a Kubernetes Lease, or a Postgres advisory lock — the second needs no cluster
  API), or keep them everywhere and pace them by replica count. Also settle the
  one gap OPS-2 recorded: the **retention sweep** is the writing loop with no
  visible lock or idempotency key, so its concurrency is unverified rather than
  known-safe.

## Why Phase 4

Phase 3's demo goal asks for a Helm install and OPS-2 is one. The enterprise
profile's HA claim is about the data plane — "HA Postgres (CloudNativePG)" is
what OPS-2's own text says — and CNPG delivers it. Gateway HA is a scale and
availability question rather than a hole in what is claimed, and OPS-2 refuses
the configuration it cannot honour instead of shipping it with a warning
comment.

Two things move it forward: a deployment that cannot serve its request rate from
one gateway, or one that cannot accept a restart-shaped upgrade. If it moves, it
belongs beside OPS-6, since both are about an upgrade nobody notices.

## Acceptance criteria

- A kind-cluster test at **three replicas** proving both blockers closed: a
  login that begins on one pod and completes on another, and a hierarchy move
  performed against one pod that every replica's composition reflects within a
  stated bound.
- The bound is **stated, not implied** — whatever the invalidation transport
  turns out to be, the staleness window is a documented number and the test
  asserts it.
- The retention sweep's concurrency is verified, and the answer is recorded
  wherever it turns out to live.
- ADR-0062's single-replica pin is lifted in the chart, and the values key that
  replaces it does not accept a number the test has not covered.
