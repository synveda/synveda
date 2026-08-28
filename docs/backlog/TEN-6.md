---
title: "TEN-6: Cross-tenant isolation test harness"
labels:
  - epic:TEN
  - phase:3
size: M
marker: "continuous"
---

# TEN-6: Cross-tenant isolation test harness

**Epic:** TEN — Multi-tenancy (functional requirement) · **Phase:** 3 · **Size:** M · **Marker:** continuous

## Problem and evidence

Forced-RLS inventory tests, foundation adversarial cases, the named context
security inventory and deterministic/10k security evaluations already cover
important tenant boundaries. Coverage is still curated: a new authenticated
route, identifier field or tenant-bearing table can land without an
automatically generated cross-tenant probe. This residual gap is described in
[production readiness](../PRODUCTION_READINESS.md) and docs/SECURITY.md.

## Scope

- Derive public authenticated operations and identifier-bearing request shapes
  from the executable route catalogue/OpenAPI, with explicit generators where
  schemas cannot express ownership.
- Exercise valid tenant-A credentials against tenant-B scopes, principals,
  workspaces, projects, sessions, Knowledge, context, capture, Skills, Tools,
  Configuration, governance, directory and audit identifiers.
- Pair API probes with direct store tests proving every tenant table remains
  enabled and forced RLS under the application role.
- Check bodies, status/error kinds, pagination, counts, timing buckets, traces
  and audit side effects for cross-tenant disclosure.
- Run deterministic coverage on every change and bounded property/fuzz cases
  on a scheduled security job.

## Non-goals

- No claim against a database superuser, compromised gateway or worker process,
  or host; those are outside the current boundary and must remain explicit.
- No random-only fuzzer whose coverage cannot be explained or reproduced.
- No replacement for PDP semantic tests, RLS inventory or targeted regression
  cases.
- No bypass policy pack or disabled RLS in test setup.

## Architecture seam

The route catalogue is the operation inventory. Tests admit two or more real
tenants, use ordinary tokens and test policy packs, and create resources
through public APIs or tenant transactions. A small machine-readable exception
list may identify bootstrap/local operations, with owner and rationale.

## Acceptance criteria

- Every authenticated public operation is probed or has a reviewed non-
  applicable reason; route additions fail CI until classified.
- Tenant-A receives the same safe 404/403/error shape for tenant-B identifiers
  as for an absent identifier where existence must be hidden.
- Denied resources do not alter counts, cursors, ordering, graph paths, score
  explanations, metrics labels or trace content.
- Direct SQL through the application role returns no tenant-B rows and cannot
  mutate them for every tenant table.
- Seeded property runs are reproducible; scheduled runs publish seed, coverage,
  failures and artifact digest and report zero leaks.

## Required tests

- Generated operation matrix plus current targeted foundation/context/RLS
  regressions.
- Multi-resource and pagination probes, including empty pages with advancing
  cursors.
- Concurrent mutation/read and revoked-authority cases.
- Restore-shaped database and new-table schema inventory tests.
- Mutation tests that remove one RLS policy/PDP row decision and prove the
  harness fails.

## Rollout and rollback

Introduce reporting before making full generated coverage gating, then require
classification and deterministic probes in pull requests. Add bounded nightly
seeds without weakening targeted tests. A flaky generator is quarantined by
seed and fixed; it is not silently removed from coverage.

## Dependencies

Security owners define the supported adversary, nightly frequency, timing-
oracle tolerance and evidence retention. The OpenAPI/route catalogue and
forced-RLS inventory must stay authoritative. EVAL security runners may consume
results but do not replace this harness.
