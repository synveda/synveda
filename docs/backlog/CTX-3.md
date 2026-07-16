---
title: "CTX-3: inject API"
labels:
  - epic:CTX
  - phase:1
size: M
---

# CTX-3: inject API

**Epic:** CTX — Context engine (read path) · **Phase:** 1 · **Size:** M

## Description

Session-start contract; warm-cache p99 <150ms; graceful degradation (partial context + warning header rather than failure).

## Acceptance criteria

latency SLO under 1k concurrent sessions; degradation modes tested.
