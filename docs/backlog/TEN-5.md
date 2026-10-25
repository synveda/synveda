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

- `synveda tenant archive export|verify|import` implement the sealed
  streaming `synveda.tenant-archive.v1` format. Export and import run as a
  configured database administrator that bypasses row security, because
  thirteen restrictive eligibility policies hide retained evidence from every
  product role. Export streams one read-only snapshot in authenticated COPY
  frames bound to archive ID, header digest, index and kind, with a sealed
  manifest of per-table ordered digests and counts and the frozen audit head,
  under `--max-bytes`, `--timeout-seconds` and a 32 MiB frame bound, and
  publishes by a never-overwriting rename. Import authenticates everything
  first, requires drained writers, restores in one transaction with deferred
  constraints and user triggers suspended, recomputes every digest and the
  audit chain, chains `tenant.restored` after the frozen prefix and leaves the
  tenant suspended; reruns report `already_restored`.

Evidence: store lifecycle and coverage cases, the RLS and policy-pack suites,
a gateway case covering an existing bearer, a fresh bearer and a stored console
session for two tenants, and a cross-process CLI case covering busy refusal,
idempotent suspension, ordinary-role inventory, policy repair while suspended,
wrong-KEK and missing-KEK refusals and audited resumption. The serial
lifecycle phase runs a fresh-storage round trip: a populated source (scopes,
workspace, project, principal and identity, a Session with events and their
projections, a Knowledge item with two revisions, provenance and an embedding,
a policy pack, a provisioned key and sealed secret, and audit) is exported,
dropped, and restored into a newly created database. All 114 portable tables
match their source digests read as an administrator, the frozen audit prefix
is identical and followed by `tenant.restored`, the secret opens only under
the original KEK, embeddings are absent and the generated lexical document is
recomputed. Refusals cover a non-administrator, wrong KEK, oversized, flipped,
truncated, appended, header-tampered, stale-schema (`0013`), connected-writer
and repeated imports, with nothing written. Offline unit cases cover
reordered, duplicated, retagged and manifest-less frames. The fixture archive
was 45,268 bytes for 43 rows in 22 frames.

- `synveda tenant erase` deletes a suspended tenant under the interim
  fixture rules, behind `--force` and `--fixture-rules`. Forward migration
  `0015` adds the `erasing`/`erased` states, the `tenant_erasure` operation
  kind and an owner-only exception, valid only for an erasing tenant, in the
  nineteen deletion guards and twelve restrictive policies. The order comes
  from the live foreign-key catalogue with declared units for the self
  reference, the five current-version pairs and the deferrable source
  cluster. Each bounded step records measured counts under the operation's
  lease; a timed-out run hands the lease back and a crashed one is reclaimed
  after expiry as a new attempt. Revision holds block before any deletion.
  The final step counts hidden rows and stores a content-free receipt
  carried by `tenant.erased`. Erasing and erased tenants refuse custody,
  suspension, resumption and archive export.
- `compose.sh tenant COMMAND...` runs suspend, resume, inventory and erase in
  a migrator one-shot, and archive export, verify and import in a separate
  one-shot with an operator-supplied administrator URL file and archive
  directory; import stops and recreates the writers. Helm adds an opt-in
  per-revision Job for the migrator commands under its own network policy and
  refuses to render an unacknowledged erase.

Erasure evidence: a database test derives the plan from the live catalogue
and checks it against every foreign key; the guards refuse the owner for a
suspended tenant and the exception refuses an ordinary role. A store case
interrupts erasure without releasing the lease, refuses a second worker while
the lease is live, reclaims it after expiry, refuses the first worker and
finishes with no row counted twice. A cross-process CLI case erases a
populated tenant beside a populated canary: a console login, a provisioned
key and sealed secret, and a revision hidden from every role by unproven
lineage. It covers the unacknowledged, active-tenant, timeout and
erasing-state refusals, and resumes after the timeout as attempt 2. The
receipt matches the pre-erasure inventory plus the hidden revision, every
erased table reads zero, the audit chain verifies, the canary and its
console custody are unchanged, and a rerun prints the same receipt. A held
revision stops erasure with nothing deleted and leaves the tenant resumable.
Compose lifecycle tests drive the action against a fake Docker and render
both one-shots with real Compose. The Helm contract check covers the default,
suspension, unacknowledged-erase and erase renders.

## Scope

Remaining work, in order:

1. **Owner decisions for erasure.** Record retention, legal holds,
   certificate contents and support authority in an ADR amendment, then
   remove the fixture-rules acknowledgement or change the retained set.
2. **Archive evidence gaps.** Seed a governed Session withdrawal so a hidden
   row is in the archive and assert it survives restore; prove embedding
   reconvergence and search through the public API after resumption; restore
   across PostgreSQL majors and from the portable onto the published lineage;
   measure a large tenant against the declared bounds; run on published
   images and native CLI archives.
3. **Live wrapper evidence.** Run the Compose and Helm one-shots against a
   deployed stack; decide whether Helm should wrap archive export and import
   with an explicitly provided administrator Secret.
4. **Erasure scale and coverage.** Measure a large tenant, populate Skills,
   Tools, Configuration, capture and directory rows before erasing, and
   bound the source cluster in more than one step if real tenants exceed it.

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
  verified frozen audit prefix (source-tested on a small fixture; scale,
  hidden-row, cross-major and published-artifact evidence open).
- Interrupted export, import and erasure resume without duplicate effects
  (export and import are atomic and rerunnable; erasure resumes under a
  reclaimed lease and counts each row once).
- Erasure leaves zero unapproved tenant references across the generated schema
  inventory, derived indexes, jobs, archives and caches (source-tested for
  the schema inventory, hidden rows and console custody under the fixture
  rules; Git export repositories, archives and backups are named, not
  removed).
- Active holds block the affected step with a stable state and content-free
  audit event (revision holds block erasure before any deletion).
- The certificate names operation/version, table or artifact counts, hashes
  and completion time without including content (the receipt does; its final
  contents await the owner decision).

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
