---
title: "CPR-44: Production hardening and maintainability cut"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-44: Production hardening and maintainability cut

**Epic:** CPR — Context-platform redesign · **Phase:** 5 · **Size:** XL

## Description

Rescan the fetched context-platform head and harden the product after the MVP.
This feature removes demonstrated maintenance burden, closes bounded defects,
and leaves an evidence-based readiness register. It does not redesign Cedar,
RLS, VedaFlow, audit, cryptography, the schema epoch or the public product
vocabulary merely because those boundaries are repetitive or large.

## Acceptance criteria

- Record the exact source head, baseline inventory and unchanged-tree results
  for `make ci`, `make db-test` and deterministic Claude acceptance. Distinguish
  unavailable live systems from failures.
- Fix reproduced security and durability defects with adversarial tests:
  directory continuations cannot cross origins or exceed a pass-wide budget;
  service-token time claims are ordered; audit verification uses one frozen
  prefix; Knowledge erasure removes downstream plaintext and live addresses;
  context planning does not reacquire the pool while holding a transaction;
  and a data-preserving uninstall preserves the deployment key.
- Split only responsibility boundaries supported by a map and behaviour tests.
  Generated OpenAPI, PDP decisions, forced RLS, VedaFlow transitions, audit
  actions, idempotency, telemetry meaning and persisted schema remain stable
  unless a separately documented defect requires a change.
- Remove confirmed dead code, stale current-model comments, speculative SDK
  placeholders, duplicate response/validation shells and unbounded metric
  labels. Preserve intentional protocol compatibility and checked invariant
  failures.
- Repair frontend request-generation races and dishonest loading/failure
  states, then extract components by independent user capability rather than
  line count. Generated API operations remain authoritative.
- Consolidate current documentation without an archive directory. Agent
  instructions become concise, stale open features use current nouns, the
  completed prompt journal is removed after unique current facts move to their
  owners, and lightweight checks prevent the same drift.
- `docs/PRODUCTION_READINESS.md` records Ready/Conditional/Not ready evidence
  and implementation-ready acceptance criteria for every remaining P0/P1.
  Missing released Helm artifacts, Helm KMS wiring, tested PITR, gateway HA,
  signing, licence ownership and external live clients are not represented by
  deterministic or local evidence.
- Focused tests pass after each commit. Final workspace, database, deployment,
  documentation, adapter, evaluation and deterministic acceptance gates pass;
  the OpenAPI and CLI contract differences are reported exactly.

## Evidence

In progress from fetched source head
`37fd12b1aa0504d18f02cd72ce7b284f672ef12f` on branch
`refactor/production-hardening-nasa`. ADR-0101 records the accepted change
boundary. Completion evidence and commit SHAs will be added only after the
final gates pass.
