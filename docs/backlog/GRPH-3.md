---
title: "GRPH-3: Graph-augmented recall"
labels:
  - epic:GRPH
  - phase:3
size: M
---

# GRPH-3: Graph-augmented recall

**Epic:** GRPH — Knowledge graph & relationships · **Phase:** 3 · **Size:** M

## Description

1–2 hop expansion in recall ranking; degradable (retrieval works with graph off).

## Acceptance criteria

multi-hop question set improves vs vector-only baseline; feature-flagged.

## Evidence

Delivered by CPR-38 on 2026-08-25 from `7951d77` under ADR-0097. The
Record-era feature flag and graph were deleted rather than carried forward:
the governed Configuration document now enables or disables anchor-first
Knowledge-relation expansion and fixes hop, fan-out, candidate, time and token
bounds. The public ContextRun acceptance runs the same query with expansion
off and on, misses the second-hop answer without it and selects the exact
current revision with its two authorised evidence steps when enabled. A
private endpoint is then introduced and contributes no address, path, count or
reason. `demos/cpr-38-bounded-graph.sh`, `make ci` and fresh-database
`make db-test` pass. The resulting feature commit is recorded by CPR-39.
