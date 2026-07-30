---
title: "EVAL-2: Extraction quality suite"
labels:
  - epic:EVAL
  - phase:2
size: M
---

# EVAL-2: Extraction quality suite

**Epic:** EVAL — Evaluation (functional requirement) · **Phase:** 2 · **Size:** M

## Description

Labelled transcript fixtures → precision/recall per memory class; hallucinated-memory rate (HaluMem-style).

## Acceptance criteria

One labelled corpus under `evals/fixtures/extraction/` read by both the eval harness and MEM-3's unit test, so a format change breaks both loudly; `make eval` reports per-class precision and recall for every `RecordClass` the corpus exercises, plus macro averages, measured over the real observe→extract→serve path and never over seeded records; the report carries produced/expected/matched per class, the unmatched-record list, and the pipeline's own committed counts read from the audit chain, so a shortfall between what was committed and what a reader is served is its own number rather than absorbed into recall; `hallucination_rate` measured from fixture-declared bait and gated at zero; a real product change that degrades quality (a retention horizon cutting served records while the pipeline still commits them) fails the gate naming the axis, the baseline, the measurement, and the delta, and the attribution column says why; the >2pt tolerance is a declared slack in the committed baseline, not a rule in code; deterministic macro precision ≥0.90; the live-model run measures the same corpus on demand against its own baseline, recording the model the API served; nightly workflow; demo script. Written 2026-07-30 (EVAL-2, ADR-0046): the feature text named a dashboard and a threshold but no axis, no path, and no artefact. The lens is the load-bearing part — extraction quality is a property of a record set, and an inject block cannot express one.

## Design

[ADR-0046](../adr/adr-0046-extraction-quality-suite.md) — the recall sweep as the lens, one corpus with two readers, and a gate with declared slack.
