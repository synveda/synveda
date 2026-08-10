---
title: "TEN-3: Dense-leg retrieval benchmark"
labels:
  - epic:TEN
  - phase:3
size: M
---

# TEN-3: Dense-leg retrieval benchmark

**Epic:** TEN — Multi-tenancy (functional requirement) · **Phase:** 3 · **Size:** M

## Description

A recall-and-latency harness for the dense leg over a seeded corpus, at stated
sizes and tenant counts, in both filter regimes; arms recorded with the corpus,
the pgvector version and the commit in each row.

## Amended 2026-08-10 (ADR-0063 decision 4)

This feature read *"Tenant-partitioned storage layout — declarative partitioning
by tenant hash for records/embeddings; partial HNSW indexes per partition
(mitigates pgvector post-filtering). AC: filtered ANN query plan shows partition
pruning; benchmark vs unpartitioned recorded."*

ADR-0063 read the AC's two halves in the honest order — the second clause is a
comparison, and a comparison is only worth running if it is allowed to come back
negative — and fixed the gate before any number existed. It came back negative.

In the regime the gate was written for, recall is **1.000 at 1.65ms p95** on an
exact scan through `records_tenant_scope_idx` that never touches the HNSW index.
There is no recall to raise and nothing for a `1/N` smaller index to cut. The
first clause, "filtered ANN query plan shows partition pruning", cannot be shown
by a deployment that does not partition, so it is amended rather than satisfied.

The partitioning half is **TEN-7**, as `LIST` rather than `HASH`, with the
numbers that would reopen it.

## Acceptance criteria

- recall@10 against exact search and p50/p95 for every arm, **three runs each**,
  in both filter regimes.
- Rows published with the corpus, the pgvector version and the commit in each,
  and re-checked by CI.
- The plan each arm ran shown at EXPLAIN rather than assumed — both the plan
  built with the query's parameters and the one built without them.

## Design

The split between measuring and judging is deliberate and is GRPH-4's, for its
reason: a gate whose thresholds live in the harness is a gate that moves when
the harness does. `crates/synveda-store/tests/ann_bench.rs` measures; ADR-0063
holds the verdict; `scripts/publish-ann-bench.mjs` records.

Two regimes, because they are the two halves of the dense leg's filter and only
one of them is a tenant: **broad** (every scope and tier in the tenant — the
regime hash partitioning would help) and **selective** (one scope, one tier —
the regime migration 0016 predicted the planner would scan exactly, and the one
partitioning by tenant cannot reach).
