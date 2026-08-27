---
title: "TEN-7: Tenant storage partition decision"
labels:
  - epic:TEN
  - phase:4
size: L
---

# TEN-7: Tenant storage partition decision

**Epic:** TEN — Multi-tenancy (functional requirement) · **Phase:** 4 · **Size:** L

## Problem and evidence

The current epoch-3 schema is unpartitioned and its dominant tables are
Sessions, capture, immutable Knowledge revisions/embeddings, context and the
audit chain. Earlier partition measurements do not name the shipped tables or
queries. No current benchmark or lifecycle requirement justifies
rewriting primary and foreign keys. [ADR-0063](../adr/adr-0063-tenant-partitioned-storage.md)
keeps the valid rule: partitioning is a measured response to a current
boundary, not a default architecture.

## Scope

This item remains dormant until either tenant lifecycle/residency requires a
physically droppable or placeable tenant unit, or representative scale shows a
reviewed PostgreSQL planning/maintenance limit that tuning cannot solve. Once
triggered, inventory the actual epoch-3 large tables and compare unpartitioned
storage with LIST partitioning for the specific tables implicated.

## Non-goals

- No partitioning of every tenant table by convention.
- No compatibility work for retired storage or migration epochs.
- No primary-key/FK rewrite justified only by hypothetical enterprise scale.
- No weakening of global identifier checks, forced RLS or audit ordering.
- No claim that partitioning alone implements tenant erasure or residency.

## Architecture seam

Partitioning stays inside synveda-store and PostgreSQL migrations/operations.
Application DTOs and public APIs do not change. Parent and child partitions
must retain explicit privileges and enabled/forced RLS. Knowledge index
generation, bitemporal views, audit sequencing and erasure inventory must
continue to address exact tenant-qualified rows.

## Acceptance criteria

- The reopening trigger, representative corpus and pre-registered success gate
  are recorded before schema work.
- EXPLAIN ANALYZE proves partition pruning on the real query shapes under the
  application role and RLS, not only an administrative session.
- Planning time, write amplification, autovacuum, index build, backup/restore,
  attach/detach and tenant-count scaling are measured.
- Every partition and parent passes privileges/RLS completeness and cross-
  tenant adversarial tests.
- Tenant disposal or placement, if the trigger, proves the full lifecycle
  boundary beyond physical table detach.
- An operator runbook states lock/outage, disk headroom, verification and
  reversal steps.

## Required tests

- Representative load/soak and query-plan comparison before/after.
- Schema tests covering relkind partitioned parents and every child.
- Cross-tenant reads/writes, Knowledge search, session pagination, context,
  audit append/verify and backup restore.
- Migration rehearsal on production-sized restored data with failure injection.
- Planning-cost sweep over the supported tenant-count envelope.

## Rollout and rollback

Shadow or copy into a separately verified partitioned layout; do not rewrite
the only production copy in place. Cut over only after canonical counts,
hashes, constraints and performance pass. Retain the old layout through the
rollback window; reversal restores the verified old tables rather than
attempting destructive repartition in place.

## Dependencies

A TEN-5, OPS-3 or measured performance trigger is mandatory. OPS-5/OPS-6
supply recovery and migration discipline. The owner must set tenant/corpus
scale, maintenance window, storage headroom and whether physical per-tenant
placement is a product requirement.
