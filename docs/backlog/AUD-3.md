---
title: "AUD-3: External immutable audit retention"
labels:
  - epic:AUD
  - phase:3
size: M
---

# AUD-3: External immutable audit retention

**Epic:** AUD — Audit (functional requirement) · **Phase:** 3 · **Size:** M

## Problem and evidence

The audit chain is tenant-complete, hash-linked and exportable at a frozen
head; synveda audit verify-export checks the canonical prefix offline under
[ADR-0092](../adr/adr-0092-context-platform-audit-export.md). This makes
tampering detectable but not impossible: a database superuser can delete the
chain, and no scheduled external object-lock target or restored-chain drill
exists. The gap is tracked in
[production readiness](../PRODUCTION_READINESS.md).

## Scope

- Persist an idempotent export schedule/cursor for each tenant and freeze each
  batch at an exact audit head.
- Write versioned manifests and canonical events to an owner-selected
  object-lock-compatible target with tenant/sequence partitioning and
  retention metadata.
- Verify every uploaded object independently, detect gaps/overlap and anchor
  the previous manifest/head so a deleted database prefix is evident.
- Monitor export lag, lock/retention state, verification failures and expired
  credentials without content-bearing labels.
- Include WORM evidence in the OPS-5 restore drill.

## Non-goals

- No claim that the Postgres append trigger is WORM.
- No replay of historical Cedar decisions or resolution of Knowledge content.
- No mutable rewrite/redaction of a frozen canonical event.
- No hand-rolled signing scheme or retention/legal-hold policy.
- No replacement of the authoritative database chain with object storage.

## Architecture seam

The existing frozen-head public export and offline verifier remain the
canonical format. A durable tenant-qualified outbox/schedule records delivery
state; the external sink is downstream and receives only the content-minimised
audit payload already authorised for export. Key/signature custody, if
required, uses the reviewed key plane rather than application constants.

## Acceptance criteria

- Scheduled exports form one contiguous canonical prefix and verify offline
  from object storage with no database or gateway access.
- Object retention prevents overwrite/delete for the configured period, and a
  missing, reordered, duplicated or forged object fails verification.
- Retries and two workers produce one manifest/object identity and advance the
  cursor only after durable acknowledgement.
- Export lag and terminal failure alert within owner-approved thresholds.
- A restored database verifies the same frozen prefix and appends a valid next
  event.
- Cross-tenant credentials/paths cannot read, overwrite or infer another
  tenant's export.

## Required tests

- Deterministic format/hash/signature tests and mutation of every canonical
  field.
- Object-store retry, duplicate acknowledgement, partial upload and credential
  expiry tests.
- Real object-lock acceptance on the selected provider; local emulation is
  labelled integration-only.
- OPS-5 restore plus audit continuation.
- Multi-tenant path/policy and retention/hold tests.

## Rollout and rollback

Dual-export to a non-authoritative locked bucket, compare every prefix, then
make lag alerting operational before relying on retention. Rollback stops new
delivery without changing the Postgres chain or previously locked objects.
Format changes write a new version beside old immutable data.

## Dependencies

Compliance/security owners choose object store, region, lock mode, retention,
legal hold, signature identity and access separation. OPS-5 provides restore
evidence; OPS-3 constrains region; AUD-4 may share the durable delivery seam
but not destination semantics.
