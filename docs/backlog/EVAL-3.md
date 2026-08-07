---
title: "EVAL-3: Public benchmark adapters"
labels:
  - epic:EVAL
  - phase:3
size: L
---

# EVAL-3: Public benchmark adapters

**Epic:** EVAL — Evaluation (functional requirement) · **Phase:** 3 · **Size:** L

## Description

LongMemEval runs end-to-end through Synveda (observe→inject/recall→judge).

LoCoMo was named here until 2026-08-07. ADR-0061 decision 1 dropped it: its
`LICENSE.txt` is CC BY-NC 4.0, granting rights "for NonCommercial purposes only",
which is precisely the use this feature's own AC describes ("Marketing artefact
too"). A benchmark run whose result we may not quote has no acceptance criterion.
The second corpus is [EVAL-7](EVAL-7.md). LongMemEval is MIT.

## Acceptance criteria

reproducible scores published in repo; tracked per release. (Marketing artefact too — every credible 2026 memory system publishes these.)
