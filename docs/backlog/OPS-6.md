---
title: "OPS-6: Upgrade and rollback discipline"
labels:
  - epic:OPS
  - phase:4
size: S
---

# OPS-6: Upgrade and rollback discipline

**Epic:** OPS — Deployment & operations · **Phase:** 4 · **Size:** S

## Problem and evidence

The pre-1.0 epoch-3 hard cut intentionally refuses older databases and the
Helm gateway still upgrades with Recreate. There is no N-1 compatibility
window, production upgrade/rollback drill or application/database ordering
contract. [ADR-0069](../adr/adr-0069-schema-epoch-and-local-reset.md) governs
the current reset boundary; it is not a post-1.0 migration strategy. The gap
is tracked in [production readiness](../PRODUCTION_READINESS.md).

OPS-11 / ADR-0112 found and fixed concurrent epoch stamping after SQLx released
its migration lock. The existing lock now covers preflight, DDL and stamping
on one owned connection; database tests cover concurrent install and cancelled
stamping, and the Kind drill runs two ordinary migrators. Same-source Helm
updates and retained reinstall preserve identities/content. Public v0.2.0
predates epoch 3 and has no current chart/reference set, so it is not a
supported N-1. The release owner must first supply a published compatible pair;
then run the declared upgrade and failure-recovery matrix. Do not manufacture
compatibility by resetting an existing database.

Published v0.4.0 and v0.4.3 are not a compatible pair: both report epoch 3 and
baseline revision 3, but the latter changes `0001_context_platform.sql`
(`source_payload_hash`). Their published checksums remain immutable; v0.4.0 is
still refused. ADR-0121 selects v0.4.3 as the first forward-upgrade source.
The source candidate restores its exact `0001` bytes and moves CTX-6/CTX-8 DDL
to transactional `0002_context_restart_and_excerpt.sql`. The fast gate pins
both files. The read-only candidate check distinguishes an exact v0.4.3 head
from the current head; the gateway refuses the former until migration stamps
`0002`. Old/current catalogue proofs flank SQLx DDL, with exact-prefix crash
recovery. This is an implementation candidate, not a published upgrade claim.

An isolated exact-role PostgreSQL run passed 24 focused epoch tests, including
an exact published-baseline SQLx prefix, retained tenant data and baseline
ledger row, read-only preflight, interrupted-stamp recovery, missing-ledger
preservation and old-catalogue drift refusal. SQLx generated-metadata
verification passes on a fresh two-head database, and the full isolated
`bash scripts/db-test.sh` suite passes, including serial authority tests. This
does not yet test a published v0.4.3 application binary,
populated Knowledge/audit/key state or a joint restore.

The next action is to exercise the transition with released v0.4.3
application/Keycloak/key material and a candidate image on a restored copy.
Inject failures before and after DDL, prove audit/history/key continuity,
measure lock time and the maintenance window, and complete the OPS-5 joint
recovery drill. Only then publish and qualify the N-1/N pair. No old binary may
serve the advanced database; rollback restores the verified recovery set.

## Scope

- Define the first supported schema/application compatibility window.
- Require expand/backfill/contract sequencing where simultaneous N-1/N serving
  is promised, with bounded resumable backfills and preflight checks.
- Test installed-host and Helm upgrade, failure and rollback using production-
  shaped data and key custody.
- Refuse an incompatible binary before readiness or traffic.
- Record an explicit outage when a safe zero-downtime path is unavailable.

## Non-goals

- No compatibility shim or data translator for retired pre-1.0 epochs.
- No claim of zero downtime while OPS-7 keeps one gateway replica and Recreate.
- No rollback by reversing destructive SQL after it has removed information.
- No manual edits to the epoch baseline, SQLx cache or generated API contract.

## Architecture seam

The provider administrator provisions extensions and the exact role contract;
schema admission/migration uses the separate ordinary migrator. Runtime
requests use the non-BYPASSRLS application roles. Migration metadata, SQLx
queries, OpenAPI/client generation and release compatibility are one reviewed
change. Backfills use durable bounded state and do not hide authorisation or
tenant work behind application handlers.

## Acceptance criteria

- A declared N-1 dataset and key set upgrades to N without data, audit or
  tenant-isolation loss, and the supported mixed-version period is tested.
- Failure at every migration phase either resumes safely or restores through
  the measured OPS-5 procedure within the owner-approved window.
- Incompatible schema/binary pairs fail readiness before serving.
- Backfills are bounded, observable, idempotent and safe under interruption.
- Contract/SQLx/RLS checks pass before and after; any API change is versioned
  and generated.
- True zero-downtime acceptance includes rolling multi-replica traffic after
  OPS-7; until then the documented result is a measured maintenance window.

## Required tests

- Upgrade matrix for every supported N-1/N binary/schema pair.
- Failure injection before/after DDL, during backfill and before contract.
- Production-sized lock-duration, pool-pressure and request-latency tests.
- Backup restore and rollback rehearsal with audit-prefix verification.
- CI lint for migration ordering and prohibited destructive operations, kept
  small enough to produce actionable findings.

## Rollout and rollback

Canary on a restored copy, then one non-production deployment, then production.
Retain the previous binary and verified backup until the compatibility window
closes. Contract cleanup ships only after every old binary is excluded.
Rollback means run the still-compatible binary or restore; never fabricate
down-migrations for irreversible changes.

## Dependencies

OPS-5 is required for recovery; OPS-7 is required for a zero-request-downtime
claim. The owner must define the supported release window, outage budget,
minimum dataset for rehearsal and the date the pre-1.0 reset policy ends.
