---
title: "OPS-4: Vector index scale decision"
labels:
  - epic:OPS
  - phase:3
size: M
---

# OPS-4: Vector index scale decision

**Epic:** OPS — Deployment & operations · **Phase:** 3 · **Size:** M

## Problem and evidence

The epoch-3 runtime uses PostgreSQL plus pgvector directly; there is no Qdrant
dependency and no current VectorIndex trait. Current scale, filtered-plan and
rebuild evidence is not sufficient to justify a second persistence system.
[ADR-0063](../adr/adr-0063-tenant-partitioned-storage.md) establishes the
important rule: benchmark the actual selective and broad tenant-filtered
workloads before changing storage shape.

## Scope

This feature reopens only after a reviewed scale/latency/recall trigger. It
then compares tuned pgvector with one production-shaped Qdrant deployment
using identical immutable Knowledge revision candidates, tenant/scope filters,
model generations and retrieval metrics. A shared interface is extracted only
after both concrete paths exist and share a real contract.

## Non-goals

- No vector-database wrapper for hypothetical future backends.
- No movement of source-of-truth Knowledge, authorisation or audit out of
  Postgres.
- No replacement of Cedar or forced RLS with Qdrant payload filters.
- No benchmark on synthetic unfiltered nearest-neighbour queries alone.
- No default change solely because one backend wins a microbenchmark.

## Architecture seam

synveda-store remains authoritative. synveda-retrieval may request bounded
candidate IDs from an index, but every returned Knowledge item/revision is
re-read and independently authorised through the ordinary tenant transaction
before persistence or rendering. Index generations carry exact model,
dimension and source-watermark evidence.

## Acceptance criteria

- A pre-registered benchmark covers representative tenant sizes, selective and
  broad scope filters, concurrent writes, rebuilds and provider outages at
  1M/5M/20M vectors or an owner-approved equivalent.
- Both paths meet identical recall, latency, freshness, isolation and
  deterministic tie-break contracts.
- Cross-tenant probes prove no denied ID, count, score or timing-dependent
  result becomes observable.
- Rebuild, dual-read comparison, atomic generation cutover and rollback are
  measured and documented.
- The operational cost and failure modes of the extra service are included in
  the decision ADR; the default changes only if the registered gate passes.

## Required tests

- Shared retrieval conformance tests over both implementations.
- Property tests for score ordering, pagination and generation cutover.
- Network partition, stale index, duplicate delivery and partial rebuild tests.
- Existing PDP/RLS adversarial and context-quality evaluations on both paths.
- Query-plan evidence and cost/soak reports, not only unit benchmarks.

## Rollout and rollback

Run Qdrant as a non-authoritative shadow index, compare results without serving
them, then canary by governed deployment configuration. Rollback serves the
last verified pgvector generation and stops shadow indexing; no Knowledge or
audit row is lost.

## Dependencies

A measured trigger, supported corpus/model envelope, hosting cost, backup
policy and on-call ownership are prerequisites. Security must approve the
remote-index threat model. If no trigger exists, close or retitle this item
rather than adding the trait.
