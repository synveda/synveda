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
both files. OPS-7 / ADR-0126 now appends `0003_one_time_login_ledger.sql`;
the read-only candidate check distinguishes exact `0001`, `0002` and current
`0003` heads. The gateway refuses the earlier heads until migration stamps
`0003`. Head-specific catalogue proofs flank SQLx DDL, with exact-prefix
crash recovery. This is an implementation candidate, not a published upgrade
claim.

An isolated exact-role PostgreSQL run passed 25 focused epoch tests, including
an exact published-baseline SQLx prefix, retained tenant data and baseline
ledger row, exact `0002` advancement, read-only preflight, interrupted-stamp recovery, missing-ledger
preservation and old-catalogue drift refusal. SQLx generated-metadata
verification passes on a fresh three-head database. On 2026-09-30 the full
isolated `bash scripts/db-test.sh` suite passed at `0003`, including the
workspace integration tests, serial authority/tamper checks and all 25 epoch
cases. Its first run exposed a stale two-row count in the concurrent-install
test; that assertion now compares successful ledger rows to the embedded
migrator's length. The corrected full rerun passed and removed its disposable
volumes. This still has not tested a published application binary, populated
audit/key state or a joint restore at `0003`.

On 2026-09-29, a disposable macOS/OrbStack rehearsal used the published
v0.4.3 product image at
`sha256:071bd209cdd26497c3c928949f38a8f2114c49384bea32eb3196952377ab47f3`
and checksum-matched `synveda-reference-0.4.3.tar.gz`. The release's
`SHA256SUMS` passed GitHub attestation verification for the exact release
workflow, tag and source commit `2acc66f02625727b2ccdfe223358468bf10eef85`;
the image digest matched the checksummed release overlay. The published stack
passed its public-API sample, leaving a pending governed proposal. A linked
PostgreSQL/Keycloak/key backup verified, and its independent private restore
recovered 58 audit events, opened the original tenant key, refused a wrong key
and passed browser/OIDC/API/logout acceptance. The sample-specific rerun on
the new browser fixture returned `product-demo failed`; that client-receipt
continuity remains unverified.

The source product image built from
`dc77956b94e0f256125baa49dadbd578db8f861d` with local image ID
`sha256:75107e28325338f5aa4087d13d8330402db2149bb98d619e7e5e523bb8255b08`.
Its read-only check classified the released head as upgradeable while refusing
to serve it. On the restored copy, the old gateway and worker stopped, `0002`
applied, and the candidate's read-only check passed. The published binary
refused the advanced schema. Candidate gateway and worker became healthy from
the exact local image ID; its recovery verifier validated the continuing
81-event audit chain, opened the original key and refused the wrong key. The
published private browser/OIDC/API/logout check passed against the candidate.
A second fresh restore from the same pre-migration set recovered the original
58-event chain and key, refused the wrong key, started the published gateway
and worker, passed its schema check and browser/API acceptance. The migrated
copy and original source were never overwritten.

This is source-candidate, local logical-recovery evidence. It does not prove
the frozen audit prefix byte-for-byte after migration, complete
Knowledge/Skill/proposal continuity, failure injection, production-sized lock
or maintenance time, off-host PITR, Helm ordering or a published N-1/N pair.
Next run those checks against representative data and the OPS-5 off-host
recovery set, then publish and qualify the pair. No old binary may serve the
advanced database; rollback restores the verified recovery set.

The released-byte rehearsal above ended at `0002`, before the OPS-7 login
ledger existed. It is historical evidence for that prefix. On 2026-09-30 a
second isolated macOS/OrbStack rehearsal used the checksum-matched published
v0.4.3 reference archive (SHA-256
`ca95e99e98eab2046a223ba870b7d657591f76762aa4ef6a525eb6942415927a`)
and its pinned product image, then a local source image
`sha256:89c56dd8d63072be5d4c8210c615eb7031a81d285f664340c04965853f8e670c`.
The published public-API sample created one Knowledge item/revision, five
proposals and 52 audit events. A linked PostgreSQL/Keycloak/key recovery set
was verified before stopping all old writers. The source image's read-only
check classified exact head `0001` as upgradeable but unable to serve; after
the declared database/peer bootstrap refreshed its postmaster witness,
`0002` and `0003` applied and the source check passed. The published binary
refused the advanced schema. The current gateway and worker became healthy,
and the released browser fixture passed OIDC, API, logout and post-logout
refusal. The current recovery verifier opened the original tenant key and
refused an unrelated key. A pending login was stored as sealed bytes, survived
a gateway process restart and was consumed once by an IdP-denial callback.

The untouched pre-upgrade recovery set restored into a distinct empty project:
the published binary accepted head `0001`, the original key opened, a wrong
key was refused, and the released browser/OIDC/API/logout check passed.
The SHA-256 digest of all serialized fields of the original 52 audit rows was
identical on that restored copy and the migrated source
(`a6e6b4297dd87f870ab8ef5ce58033421361d2bcee73f54c363be3404171ec7a`).
The source's read-only `retry-review status` found the original Knowledge,
capture and pending proposal. Re-running the published sample, however, hit
an idempotency conflict: its fixed `configuration.create` key carried the
current template document rather than the document submitted before upgrade.
The CPR-45 client now reads and validates the recorded Configuration and
Sessions before resuming, preserving their original version and ownership
without resubmitting a changed body under the same idempotency key. Its rebuilt
browser image replayed the v0.4.3 receipt against the migrated source gateway;
`product-demo.mjs sample` passed and remained `learning_pending`. The sample's
complete Skill/approval journey was not seeded before backup, so Skill
continuity was outside that first drill. This is a
same-host source-candidate drill, not a published N-1/N pair or off-host PITR.

A second isolated v0.4.3 project completed the full four-person demo: reviewed
Knowledge, a distinctly approved Skill version and binding, provenance-bound
context, and a verified audit chain. Its released full-seed browser path exposed
a loopback-origin assumption that the published `sample` path did not have;
the candidate acceptance client now passes the configured origin through every
receipt check. A verified linked recovery set was taken before the source
candidate applied `0002` and `0003`. The upgraded browser read-only receipt
check passed. A new CLI rerun initially conflicted on the old context-run
idempotency key: the new gateway had included omitted optional fields in its
digest. The candidate gateway now preserves the v0.4.3 digest for omitted
options and includes non-default restart/tokenizer/required-revision options.
The rerun then verified the same Knowledge revision, Session-event provenance,
Skill version and binding, context run, and audit chain through public APIs.

An independent restore into an empty project passed the published binary's
browser/OIDC/API/logout check. Tenant-scoped read-only transactions under the
ordinary `synveda_app` role found two Knowledge items, one Skill version, one
binding, six proposals and one context run in both copies. Audit rows 1–384
matched by serialized-row checksum and chain hash between the restored and
migrated databases; each copy appended a different row 385 during its own
post-backup verification. The source and restore volumes and recovery set are retained,
with all containers stopped. This remains one macOS/OrbStack source-candidate
drill, not a published N-1/N pair or off-host PITR. Next recover an in-flight
pending login from a joint `0003` set, inject migration failures, measure
production-shaped locks/outage and run the OPS-5 off-host restore before a
supported upgrade claim.

The published v0.4.3 consumer recovery launcher pins its released product
verifier, which correctly refuses a `0003` database. The next pending-login
joint-restore drill therefore needs a head-`0003` candidate reference bundle:
park one login, back up the linked database/identity/key set, restore into an
empty project with that same candidate, then consume the state once and refuse
replay. Repeating the result with a published compatible N pair remains a
separate promotion gate.

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
