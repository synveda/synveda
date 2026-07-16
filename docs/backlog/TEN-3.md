---
title: "TEN-3: Tenant-partitioned storage layout"
labels:
  - epic:TEN
  - phase:3
size: M
---

# TEN-3: Tenant-partitioned storage layout

**Epic:** TEN — Multi-tenancy (functional requirement) · **Phase:** 3 · **Size:** M

## Description

Declarative partitioning by tenant hash for records/embeddings; partial HNSW indexes per partition (mitigates pgvector post-filtering).

## Acceptance criteria

filtered ANN query plan shows partition pruning; benchmark vs unpartitioned recorded.
