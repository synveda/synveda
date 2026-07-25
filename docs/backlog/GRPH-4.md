---
title: "GRPH-4: AGE performance spike / graph fallback assessment"
labels:
  - epic:GRPH
  - phase:2
size: S
marker: "de-risk, Phase 2 gate"
---

# GRPH-4: AGE performance spike / graph fallback assessment

**Epic:** GRPH — Knowledge graph & relationships · **Phase:** 2 · **Size:** S · **Marker:** de-risk, Phase 2 gate

## Description

Benchmark AGE Cypher traversal at the scales ADR-0001 and ADR-0004 both flag as unproven, and decide whether the multi-graph AGE schema survives. Assess the fallback the two ADRs name as their reversal trigger, and record the conditions that would activate it.

## Acceptance criteria

report with traversal benchmarks at 1M/10M edges; go/no-go criteria recorded as ADR — recorded *before* the benchmark runs, since a spike that fixes its thresholds after seeing the numbers can only ratify the decision it was commissioned to test.
