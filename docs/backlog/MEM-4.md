---
title: "MEM-4: Transactional embed-or-fail"
labels:
  - epic:MEM
  - phase:1
size: M
---

# MEM-4: Transactional embed-or-fail

**Epic:** MEM — Memory core (write path) · **Phase:** 1 · **Size:** M

## Description

Embedding (TEI/BGE-M3) inside the ingestion transaction boundary; partial batch failure → retry, never silent drop (the documented Mem0 failure mode).

## Acceptance criteria

chaos test kills TEI mid-batch; zero lost or embedding-less records.
