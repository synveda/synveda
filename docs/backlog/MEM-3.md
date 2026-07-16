---
title: "MEM-3: Extraction pipeline"
labels:
  - epic:MEM
  - phase:1
size: L
---

# MEM-3: Extraction pipeline

**Epic:** MEM — Memory core (write path) · **Phase:** 1 · **Size:** L

## Description

Temporal workflow: classify into fact/decision/preference/procedure/entity/episode; Extractor trait (Claude API + vLLM impls); summarise-at-write.

## Acceptance criteria

extraction precision measured on a labelled fixture set ≥ target (see EVAL-2); every record carries provenance (session, method, model version, confidence).
