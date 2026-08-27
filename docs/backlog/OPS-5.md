---
title: "OPS-5: Backup/restore & DR"
labels:
  - epic:OPS
  - phase:4
size: M
---

# OPS-5: Backup/restore & DR

**Epic:** OPS — Deployment & operations · **Phase:** 4 · **Size:** M

## Problem and evidence

There is no production backup configuration, WAL archive, PITR restore or
recurring restore drill. Disposable database tests are not recovery evidence.
The Helm chart has no backup stanza, and the local KMS key or externally owned
Helm Secret must be restored with the database. This is a P0 gap in
[production readiness](../PRODUCTION_READINESS.md).

## Scope

- Encrypt and retain PostgreSQL base backups plus WAL in an owner-selected
  object store for the installed-host and Helm deployment shapes in scope.
- Restore to a chosen point into an isolated deployment with the exact schema
  epoch, deployment key and tenant-key custody material.
- Verify tenant admission, immutable Knowledge heads/history, session and
  capture state, VedaFlow refs, forced RLS inventory and the complete audit
  prefix after restore.
- Rebuild or verify derived pgvector indexes and report readiness only after
  the restored service passes its post-restore checks.
- Automate a recurring drill and measure achieved RPO/RTO.

## Non-goals

- No invented RPO/RTO, retention or legal-hold policy.
- No multi-region active/active or residency failover.
- No backup of an unrecoverable database without its key material.
- No destructive retention cleanup before at least one restore has verified
  the retained generation.

## Architecture seam

Postgres remains the durable source of truth; deployment tooling schedules and
restores backups. Key custody remains outside the database as required by
[ADR-0094](../adr/adr-0094-context-platform-key-and-secret-plane.md). A
post-restore verifier may use existing schema, RLS, Knowledge, audit-export and
index checks but must not bypass the gateway for application acceptance.

## Acceptance criteria

- Encrypted base backup plus WAL restores to selected instants before and
  after committed writes, within documented RPO/RTO.
- Correct database/key pairs start and open sealed data; missing, stale or
  wrong keys fail closed with actionable diagnostics.
- A frozen audit prefix verifies byte-for-byte before backup and after restore,
  and new events append to the same valid chain.
- Public-API acceptance proves Sessions, Knowledge, context and governed
  mutation against the restored deployment.
- Restore recreates no cross-tenant visibility and every tenant table remains
  enabled and forced RLS.
- A scheduled isolated drill publishes age, duration and result metrics and
  alerts on missed or failed evidence.

## Required tests

- PITR tests across multiple write points and a failed/incomplete backup.
- Joint database/KMS restore, wrong-key and lost-key tests.
- Restore followed by RLS adversarial, audit verification and index search
  checks.
- Corrupt archive, expired credential, object-store outage and retention-race
  fault injection.
- Monthly production-shaped drill outside the source cluster.

## Rollout and rollback

Enable backups in a non-production cluster, restore them independently, then
stage retention. Keep at least one previously verified generation while
changing format, provider or keys. Rollback disables new scheduling without
deleting valid backups; restoration never writes into the source cluster.

## Dependencies

The owner must choose object store/region, encryption custody, retention,
RPO/RTO, drill frequency and incident ownership. OPS-3 constrains residency;
AUD-3 constrains WORM audit retention; TEN-5 constrains tenant erasure and
legal holds.
