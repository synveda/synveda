---
title: "CTX-1: Hybrid retrieval"
labels:
  - epic:CTX
  - phase:1
size: L
---

# CTX-1: Hybrid retrieval

**Epic:** CTX — Context engine (read path) · **Phase:** 1 · **Size:** L

## Description

pgvector ANN + Tantivy BM25, RRF fusion; always filtered by tenant+scope+sensitivity via authz-derived predicate pushdown.

## Acceptance criteria

retrieval quality on fixture set; NO LLM calls on read path; p99 <80ms at 1M records/tenant.
