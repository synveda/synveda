---
title: "CPR-23: Immutable skill versions, bindings and usage"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-23: Immutable skill versions, bindings and usage

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** XL

## Description

Replace the mutable draft/channel skill registry with stable skill aggregates,
immutable content-addressed versions, project/principal bindings, evidence-
labelled usage and controlled test runs. Extend the existing Agent Skills,
scanner, quality and VedaFlow implementation; do not create a parallel registry.

## Acceptance criteria

- Agent Skills bundles validate against the official unversioned specification
  snapshot pinned in ADR-0085, preserve extension metadata and install without
  byte rewriting; declared tools remain non-authoritative metadata.
- Every content change creates a distinct immutable version and digest; version
  and file rows cannot be updated or deleted, and scan/provenance evidence is
  retained on that exact version.
- Only applied VedaFlow changes can install/update a version or create/change a
  binding. Stale current-version and binding-revision preconditions reject
  without changing active state.
- Project and principal bindings can follow current, pin, disable, re-enable
  and roll back. Rollback changes a binding, never version history.
- Available/resolve and context advertisement use the same PDP-filtered binding
  set; denied, disabled and foreign-tenant bindings leak no ids, names or counts.
- Usage records all eight required stages against an exact binding/version and
  distinguishes `host_observed` from `model_reported`, idempotently.
- Test runs use an explicit validation sandbox or identified controlled client;
  the gateway never executes bundled scripts.
- Generated public APIs cover catalogue, versions, files, bindings, usage,
  tests and rollback with keyset pagination, idempotency and revision
  preconditions. The old draft/channel routes, DTOs, store paths and duplicate
  telemetry are deleted.
- Focused unit/integration/RLS/PDP/audit/API/CLI tests, a runnable demo,
  `make ci` and `make db-test` pass.

## Delivered

Migration `0052_versioned_skills.sql` replaces the mutable draft registry and
its special checklist/override rows with stable `skills`, immutable
`skill_versions` and `skill_version_files`, revisioned `skill_bindings`, typed
VedaFlow effects, append-only `skill_usage_events` and immutable
`skill_test_runs`. Every new table is tenant-bound with enabled and forced RLS;
version/file/event/test history has no application update or delete grant.
There is no translation of old rows and no `skill/published` read or write.

The generated contract now has 85 operations, including eighteen Skill
operations for catalogue pages, exact versions/files, project or principal
bindings, availability, eight-stage usage evidence, controlled test runs and
rollback. Install, update, bind and rollback all open the same typed VedaFlow
change, repeat PDP/precondition/scan/rubric checks at apply, and emit
content-free semantic audit events. Context composition advertises the exact
resolved binding/version/digest. The CLI consumes those public operations;
declared tools remain manifest metadata and the gateway's validation sandbox
never executes bundle content.

Official compatibility is pinned to the unversioned Agent Skills specification
at upstream commit `69ef37e9424c0a7ea9dd2293b559e43ec8176379`, observed
2026-08-24. Unknown extension metadata and exact file bytes survive, while the
published name grammar and optional `compatibility`/experimental
`allowed-tools` fields are tested rather than assigned an invented version.

## Evidence

- `crates/synveda-gateway/tests/skills.rs`: 1/1 end-to-end install, reviewed
  apply, immutable update, stale rejection, binding, usage, validation and
  rollback case.
- Store RLS/immutability and completeness cases: 1/1 each; policy packs 7/7;
  OpenAPI 5/5; CLI 157/157; console 179/179; workspace clippy with warnings
  denied passes.
- `demos/cpr-23-versioned-skills.sh`: PASS against isolated epoch-2 Postgres,
  asserting one aggregate, two immutable versions, one binding, one usage
  event and one non-executing test run.
- `make ci`: PASS. `make db-test`: PASS against disposable
  `synveda_test_80706`, including migration, forced-RLS and database-backed
  gateway coverage.
