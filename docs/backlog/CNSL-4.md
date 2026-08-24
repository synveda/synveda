---
title: "CNSL-4: Knowledge browser"
labels:
  - epic:CNSL
  - phase:4
size: M
---

# CNSL-4: Knowledge browser

**Epic:** CNSL — Admin console (Phase 3) · **Phase:** 4 · **Size:** M

## Description

Delivered and subsumed by CPR-17's Knowledge Browser. The original feature was
filed against mutable per-scope records and channel pin/retire operations.
Those nouns and operations were deleted by the Phase 5 hard cut. Their product
objective survives as scope-filtered immutable Knowledge revisions with
independently authorised provenance, validity/history and governed lifecycle
commands.

## Acceptance criteria

No direct-mutation path exists: create, edit, verify, merge, supersede,
archive, restore and forget all execute as typed VedaFlow changes. The browser
uses only generated public API operations. Acceptance evidence is
`crates/synveda-gateway/tests/knowledge_lifecycle.rs`,
`console/src/knowledge.test.mts` and
`demos/cpr-17-knowledge-browser.sh`; all pass on 2026-08-24.
