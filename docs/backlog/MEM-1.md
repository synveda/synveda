---
title: "MEM-1: observe API + PGMQ buffer"
labels:
  - epic:MEM
  - phase:1
size: M
---

# MEM-1: observe API + PGMQ buffer

**Epic:** MEM — Memory core (write path) · **Phase:** 1 · **Size:** M

## Description

Batched transcript/event ingestion; ack <20ms; idempotency keys.

## Acceptance criteria

load test 1k events/s on dev hardware; duplicate delivery does not duplicate memories.
