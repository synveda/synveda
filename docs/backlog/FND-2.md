---
title: "FND-2: Dev environment"
labels:
  - epic:FND
  - phase:0
size: S
---

# FND-2: Dev environment

**Epic:** FND — Foundation · **Phase:** 0 · **Size:** S

## Description

docker-compose: Postgres 17 + pgvector + AGE + PGMQ, Rauthy, Temporal, TEI (BGE-M3), Jaeger.

## Acceptance criteria

`make dev-up && make smoke` passes.
