---
title: "EVAL-7: A second public benchmark"
labels:
  - epic:EVAL
  - phase:4
size: M
---

# EVAL-7: A second public benchmark

**Epic:** EVAL — Evaluation (functional requirement) · **Phase:** 4 · **Size:** M

## Description

A second externally-comparable memory benchmark, published under EVAL-3's two-tier
discipline. LoCoMo was to be that benchmark and cannot be.

## Why this exists

Filed 2026-08-07 by EVAL-3 (ADR-0061 decision 1), which found it by reading a
licence before writing an adapter.

`snap-research/locomo`'s `LICENSE.txt` is Creative Commons
**Attribution-NonCommercial 4.0 International**. It grants rights "for
NonCommercial purposes only" and defines NonCommercial as material "not primarily
intended for or directed towards commercial advantage or monetary compensation."

EVAL-3's own acceptance criterion says the scores are a "Marketing artefact too —
every credible 2026 memory system publishes these." That is the use the licence
withholds, stated in the feature text that would have relied on it. A benchmark
run whose result we may not quote has no acceptance criterion, so the corpus was
dropped rather than run quietly — running it internally to improve a commercial
product is arguably the same commercial use, publication or not, and would have
been an accepted legal risk rather than an avoided one.

LongMemEval is MIT (Copyright (c) 2024 Di Wu) and carries no such restriction.
EVAL-3 ships it, so the phase's demo goal is met; this feature is the second data
point, not the first.

## What it cost, and what closed the gap

**Nothing in the build would have caught this.** CLAUDE.md's licence rule names
MIT/Apache-2.0/PostgreSQL for the core path and `cargo-deny` enforces it — over
crates. A corpus is data. A non-commercially-licensed dataset therefore got as far
as being named in a feature specification and a published phase demo goal, and
would have got as far as a published score, without touching a check.

ADR-0061's compliance note closes that where the build can see it:
`make check-corpus-licences` asserts that every directory under `evals/fixtures/`
carrying third-party material has a licence file naming a permitted licence. That
check exists because of this finding; this feature is the corpus it cost.

## Two paths, either sufficient

- **Written permission from Snap Research** for commercial benchmark use, recorded
  in the repository beside the corpus rather than in somebody's memory of an
  email. This is the path that restores LoCoMo specifically, and it is also
  ADR-0061 reversal trigger (e).
- **A permissively-licensed substitute** in LoCoMo's slot. This needs a candidate
  found and licence-checked before the feature can commit to one, which is the
  main reason the size is honest at M rather than S.

## Why Phase 4

Neither path is work we control: one waits on a grant from a third party, the
other on a corpus that may not exist yet. Scheduling it into Phase 3 would put a
dependency on someone else's goodwill in front of the procurement block.

## Acceptance criteria

- A second published benchmark score under EVAL-3's two-tier discipline: a
  deterministic tier that gates and a model-judged tier that is published and
  gates nothing, against a baseline keyed to both the reader and judge models as
  the API served them.
- The corpus arrives in **EVAL-3's format, or the reason it cannot is recorded** —
  ADR-0047 reversal trigger (f) inherited rather than escaped, for the same reason
  EVAL-3 inherited it: a fourth corpus format is where the vocabulary stops being
  shared.
- **The licence permits the use, and the evidence is in the repository.**
  `make check-corpus-licences` passes over the new corpus without an exemption —
  or, if the corpus is LoCoMo under a grant, the grant is a file and the check
  reads it.
