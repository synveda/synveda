---
title: "CPR-38: Bounded graph-augmented retrieval"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-38: Bounded graph-augmented retrieval

**Epic:** CPR — Context-platform redesign · **Phase:** 5 · **Size:** XL

## Description

Expand already-authorised lexical, semantic, pinned and freshness-aware
Knowledge anchors across the canonical immutable Knowledge-relation graph under
hard governed bounds, and retain only re-authorised explainable paths in
ContextRun.

## Acceptance criteria

- The existing Knowledge planner supplies authorised anchors; graph-only
  enumeration is impossible. Expansion permits only supporting relation types,
  at no more than two hops, with bounded anchors, fan-out, total candidates,
  time and graph-token work.
- PDP runs before anchors, before each adjacency expansion, after expansion and
  before an inspector renders path detail. A denied node suppresses its whole
  path, id, edge, reason and count; only the aggregate policy-exclusion message
  may remain.
- Candidates persist separate anchor, edge-weight, hop-penalty, freshness,
  current-state and final components plus an immutable visible evidence path.
  `contradicts` can produce a zero-weight warning and never supporting rank.
- Full, redacted, hashes-only and disabled traces retain/expose the correct path
  shape. Context Inspector shows anchors, directed relation steps, hop/weight
  evidence and degradation without hydrating content outside full mode.
- Graph storage failure or time/fan-out/candidate/token exhaustion preserves the
  lexical/vector result and records an exact degradation. Superseded, stale and
  transitional Knowledge cannot become current through an edge.
- The dead Record-era graph/linker/runtime and direct traversal API are deleted
  without translation. The remaining GRPH-3 product objective is subsumed by
  this feature; focused PDP/RLS/trace/ranking tests, a multi-hop evaluation
  fixture, demo, `make ci` and `make db-test` pass. ADR-0097.

## Evidence

Delivered 2026-08-25 from `7951d77` under ADR-0097 and migration
`0062_bounded_graph_retrieval`. Governed Configuration supplies closed bounds;
ContextRun v2 expands only already-authorised current Knowledge anchors over
the canonical immutable relation graph, re-authorises every frontier, endpoint
and retained path, and records separate score components plus exact or hashed
immutable path evidence. Storage/time and every work bound fall back to the
lexical/vector result with a named degradation. The public acceptance proves a
two-hop answer is absent with graph disabled and selected with it enabled, then
proves a denied private endpoint contributes no id, revision, content, count or
path. The Record graph, linker, runtime tests and obsolete GRPH-1/GRPH-4 demos
were deleted without translation; GRPH-3 closes by subsumption. Focused types,
Knowledge, Configuration, ContextRun, RLS, OpenAPI and console suites,
`demos/cpr-38-bounded-graph.sh`, `make ci` and full fresh-database
`make db-test` pass. The resulting commit hash is recorded by CPR-39.
