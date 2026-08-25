---
title: "CPR-37: Conflict, supersession and freshness engine"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-37: Conflict, supersession and freshness engine

**Epic:** CPR — Context-platform redesign · **Phase:** 5 · **Size:** XL

## Description

Add durable, policy-safe conflict evidence and governed resolution to current
Knowledge, make type-aware freshness one evaluated view of the exact governed
Configuration version, and expose orthogonal valid-time/transaction-time query
semantics and current-versus-history review.

## Acceptance criteria

- A bounded shared classifier proposes duplicate, support, contradiction,
  supersession or transition at capture and direct Knowledge write time, after
  exact per-item PDP decisions and without generative work on reads.
- Forced-RLS `ConflictSet` and `ConflictMember` persistence cites exact
  Knowledge revisions or a reviewable capture candidate. Denied members do not
  leak through ids, relations, reasons or counts.
- An unresolved conflicting write is `transitional` and absent from ordinary
  listing, search and ContextRun. Revision-aware resolution passes through the
  existing Knowledge/VedaFlow/PDP/audit path and supports separate truth,
  support/duplicate evidence, supersession, future transition and archival;
  merge remains the existing governed merge command.
- `FreshnessPolicy` cites the exact effective governed Configuration evidence,
  applies explicit `stale_after` first and implements type-aware interval,
  explicit-supersession, repository-change, failed-use and source-freshness
  signals without a second mutable policy table.
- Public cursor queries distinguish current, `as_of` valid time,
  `as_known_at` transaction time, `include_history` and
  `include_transitional`; defaults expose only current active visible
  Knowledge.
- The generated Knowledge console exposes conflict review,
  current-versus-history comparison, future transitions, a staleness queue and
  verification. Tests prove current projection safety, temporal truth,
  tenant/PDP isolation, resolution preconditions and no contradictory-current
  context selection.
- Migration/SQLx/OpenAPI/generated console artefacts, focused tests, acceptance
  demo, `make ci` and `make db-test` pass. ADR-0096.

## Evidence

Delivered from `ca3730f7d43b0bd6fd2a1a22d86d48093eea7395` under accepted
ADR-0096. Migration `0061_conflict_freshness.sql` adds forced-RLS conflict
sets and exact members; a shared bounded classifier and revision-aware
VedaFlow resolution keep ambiguous heads transitional until a governed
decision. The exact effective freshness Configuration is frozen as evidence,
and public Knowledge queries now compose valid time, as-known transaction
time, history and transitional-state semantics without a latest-row-wins
shortcut. The generated Knowledge Browser adds conflict comparison, future
transition, staleness and policy evidence views.

Focused evidence: Knowledge types **7/7**, lifecycle **5/5**, capture API
**4/4**, ingest **2/2**, context **3/3**, OKF **2/2**, RLS **83/83**, OpenAPI
**6/6**, and console **215/215**. The same-tenant denied-member oracle,
transitional ContextRun exclusion, future-valid transition, explicit stale
date and bitemporal projection regressions pass. The isolated
`demos/cpr-37-conflict-freshness.sh`, the **85-script** demo drift gate,
`make ci`, and full fresh-database `make db-test` on removed database
`synveda_test_85309` pass. The contract is **171 operations / 272 schemas**;
epoch 2 has **59 migration files**, **92 forced-RLS tenant tables** and **726
SQLx descriptions**. The resulting commit hash is recorded by CPR-38.
