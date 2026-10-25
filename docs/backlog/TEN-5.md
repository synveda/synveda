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

The epoch-3 store admits and resolves tenants. Before this work nothing
changed a tenant's status after admission, and nothing fenced work admitted
before such a change. The sealed tenant export holds current Knowledge history
and audit evidence but is assembled in memory and has no restore path.
Knowledge Forget and Session/artifact cleanup are aggregate erasure, not a
tenant lifecycle. The P1 gap is recorded in
[production readiness](../PRODUCTION_READINESS.md).
[ADR-0157](../adr/adr-0157-tenant-lifecycle-suspension-archive-and-erasure.md)
records the lifecycle decisions.

### Delivered so far

Operator suspension, resumption and dry-run inventory are source-tested
([operator guide](../TENANT_LIFECYCLE.md)):

- `synveda tenant suspend|resume` run under the exact migrator principal,
  take the tenant's exclusive admission lock, are idempotent, take a closed
  reason code and chain `tenant.suspended`/`tenant.resumed` break-glass events.
  A busy tenant refuses with `tenant_busy` after the bounded wait.
- Every ordinary tenant transaction re-reads the tenant status after taking
  the shared admission lock. Work admitted before a suspension finishes;
  any later transaction, including a request already past resolution or a
  worker effect after a provider call, refuses as *tenant unavailable*. The
  two transaction settings were folded into one statement, so admission still
  issues three statements.
- Operator key custody, break-glass policy repair and best-effort audit
  evidence use separate custody transactions that require an admitted but not
  an active tenant; inventory and verification reads use a read-only snapshot.
- Resumption compiles every stored pack with this build and unwraps every
  stored tenant key generation before flipping the row, refusing with
  `policy_invalid` or `key_unavailable`.
- `synveda tenant inventory` counts all 116 tenant-bearing tables under forced
  RLS with a per-table cap and names deployment tables and external copies.
  A database test fails when the catalogue gains an unclassified
  tenant-bearing table, loses a classified one, lacks forced RLS, or when the
  pinned restore order violates an immediately checked foreign key.
- Forward migration `0014`, byte-identical in both lineages, makes the eleven
  cycle-closing and self-referencing tenant foreign keys deferrable but
  initially immediate, so ordinary behavior is unchanged and a single
  transaction restore can defer them.

Evidence: store lifecycle and coverage cases, the RLS and policy-pack suites,
a gateway case covering an existing bearer, a fresh bearer and a stored console
session for two tenants, and a cross-process CLI case covering busy refusal,
idempotent suspension, ordinary-role inventory, policy repair while suspended,
wrong-KEK and missing-KEK refusals and audited resumption.

## Scope

Remaining work, in order:

1. **Portable archive `synveda.tenant-archive.v1`.** Stream one
   `REPEATABLE READ READ ONLY` snapshot to a private temporary file in sealed,
   length-prefixed COPY frames bound to the archive ID, header digest and frame
   index, with a final manifest of per-table ordered column lists, row counts
   and BLAKE3 digests and the frozen audit head; publish by atomic rename.
   Declare and enforce frame, total byte and wall-time bounds.
2. **Import.** Same tenant ID into a deployment at the same epoch, baseline
   revision and head on either lineage. Authenticate the whole archive before
   writing; refuse corrupt, truncated, oversized, wrong-KEK, wrong-schema and
   colliding archives. Restore in one migrator transaction with deferred
   constraints, recompute every digest and the audit chain to the frozen
   head, chain `tenant.restored` after it and leave the tenant suspended.
   Exclude derived embeddings and console session index rows; prove index
   reconstruction through the public search API after resumption.
3. **Erasure.** Add `erasing` and `erased` states, a lifecycle operation kind
   and an owner-only delete permission inside the immutable-row trigger
   functions; delete in the pinned reverse order in bounded leased steps that
   honour revision holds, and derive the receipt from measured deletions and a
   residual inventory. Fixture rules only until the owner decisions below.
4. **Compose and Helm one-shots** that run the lifecycle commands with the
   migrator credential on the private database network.

## Non-goals

- No deployment-volume purge presented as tenant erasure.
- No crypto-shredding claim for plaintext or substrate-encrypted rows.
- No direct SQL shortcut around Cedar, forced RLS, VedaFlow or audit on
  application-owned lifecycle actions.
- No invented legal-hold, retention or certificate semantics.
- No requirement to introduce Temporal or a second job system.

## Architecture seam

Lifecycle acts are deployment-operator commands under the exact migrator
principal, distinct from admission by command, audit action and required
prior status. The tenant admission advisory lock is the fence: transitions
take its exclusive mode and every ordinary tenant transaction re-reads the
status under its shared mode. Durable operations and attempts fence erasure
steps; the sealed export key envelope of ADR-0094 is extended by a separately
versioned archive format. Knowledge Forget remains the lower-level aggregate
primitive.

## Acceptance criteria

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

- Schema-driven test that fails when a tenant-bearing table is absent from
  lifecycle coverage (delivered).
- Two-tenant suspension across existing and fresh credentials, in-flight work
  and processes (delivered).
- Populated export/import/erase across Sessions, capture, Knowledge, context,
  Skills, Tools, Configuration, directory, secrets and audit, with
  cross-tenant canaries.
- Crash/restart, concurrent request, hold, wrong-key, corrupt, truncated,
  oversized and colliding archive tests.
- Restore and audit verification against a fresh database.

## Rollout and rollback

Suspension and resumption are reversible and shipped first. Export/import
follow; erasure is enabled only after restore and hold semantics are approved.
Preserve the source tenant until import verification completes. Erasure is
force-explicit and irreversible; rollback before execution clears suspension,
while interruption after execution resumes forward.

## Dependencies

Privacy/legal owners must decide retention, legal holds, certificate contents
and whether a support authority other than the deployment operator may run
lifecycle commands. The joint PITR and KEK custody contract
([ADR-0154](../adr/adr-0154-joint-pitr-recovery-and-kek-custody.md)) supplies
key recoverability for archives; external audit retention and multi-replica
propagation remain separate open work.
