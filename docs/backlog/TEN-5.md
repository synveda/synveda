---
title: "TEN-5: Tenant lifecycle"
labels:
  - epic:TEN
  - phase:3
size: M
---

# TEN-5: Tenant lifecycle

**Epic:** TEN — Multi-tenancy (functional requirement) · **Phase:** 3 · **Size:** M

## Problem and evidence

The epoch-3 store admits and resolves tenants, and suspended tenants fail
resolution, but there is no complete public/operator suspend-resume workflow,
portable import or ordered populated-tenant erasure. The sealed tenant export
contains current Knowledge history and audit evidence but is assembled in
memory and has no restore path. Knowledge forget is aggregate erasure, not a
tenant lifecycle. The P1 gap is recorded in
[production readiness](../PRODUCTION_READINESS.md).

## Scope

- Inventory every epoch-3 table, view, derived index and external object that
  can reference or contain tenant data.
- Add audited suspend/resume and a restart-safe lifecycle operation with
  explicit states, progress, idempotency and dry-run inventory.
- Stream a versioned, bounded export and import it into an empty admitted
  tenant/deployment while preserving immutable revisions, VedaFlow and
  canonical audit evidence.
- Erase a populated tenant in a reviewed dependency order, honoring retention
  holds and retaining only owner-approved content-free evidence.
- Produce a verifiable destruction certificate derived from measured effects,
  not a promised cascade.

## Non-goals

- No deployment-volume purge presented as tenant erasure.
- No crypto-shredding claim for plaintext or substrate-encrypted rows.
- No direct SQL shortcut around Cedar, forced RLS, VedaFlow or audit on
  application-owned lifecycle actions.
- No invented legal-hold, retention or certificate semantics.
- No requirement to introduce Temporal or a second job system.

## Architecture seam

Tenant admission remains a local administrative exception; ordinary lifecycle
requests use an explicit operator/support authority and audit chain. Durable
operations and tenant-bound transactions coordinate bounded work. The existing
sealed export/key envelope from
[ADR-0094](../adr/adr-0094-context-platform-key-and-secret-plane.md) is the
format seam, but import and streaming require a separately versioned contract.
Knowledge forget remains the lower-level aggregate primitive.

## Acceptance criteria

- A populated tenant can be suspended and resumed, with new and existing
  credentials failing closed during suspension on every replica.
- Export streams within declared file/byte/time bounds and restores into a
  clean instance with identical immutable Knowledge/VedaFlow evidence and a
  verified frozen audit prefix.
- Interrupted export, import and erasure resume without duplicate effects.
- Erasure leaves zero unapproved tenant references across the generated schema
  inventory, derived indexes, jobs, archives and caches.
- Active holds block the affected step with a stable state and content-free
  audit event.
- The certificate names operation/version, table or artifact counts, hashes
  and completion time without including content.

## Required tests

- Schema-driven property test that fails when a new tenant-bearing table is
  absent from lifecycle coverage.
- Populated end-to-end export/import/erase across Sessions, capture, Knowledge,
  context, Skills, Tools, Configuration, directory, secrets and audit.
- Cross-tenant canary rows proving no lifecycle step touches another tenant.
- Crash/restart, concurrent request, hold, wrong-key, corrupt archive and
  oversized export tests.
- Restore and audit verification against a fresh database.

## Rollout and rollback

Begin with dry-run inventory and suspend/resume, then export/import, and enable
erasure only after restore and hold semantics are approved. Preserve the source
tenant until import verification completes. Erasure is force-explicit and
irreversible; rollback before execution clears suspension, while interruption
after execution resumes forward.

## Dependencies

Privacy/legal owners must decide retention, legal holds, certificate contents
and support authority. OPS-5 supplies recoverability; AUD-3 supplies external
retention; OPS-7 supplies multi-replica propagation; key custody and export
format changes require an accepted ADR.
