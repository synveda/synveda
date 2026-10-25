# ADR-0157: Operator tenant lifecycle with fenced suspension, a sealed portable archive and measured erasure

- **Status**: Accepted (suspension, inventory, archive and restore are
  source-tested); erasure is accepted only for synthetic fixtures until the
  owner decisions below exist
- **Date**: 2026-10-25
- **Feature(s)**: Tenant lifecycle
- **Deciders**: Owner-authorized implementation

## Context

Tenant admission is a local administrative act: `synveda tenant create` and
`converge` run as the exact migrator principal, the only role with write
grants on `tenants`, and chain a break-glass `tenant.created` event.
Resolution reads `tenants.status` uncached on every request, so a suspended
row already refuses new requests on every replica. Nothing changes that row
after admission, and nothing fences work admitted before such a change: the
request middleware has already resolved the tenant, and worker effects open
their own tenant transactions after provider calls.

Every ordinary tenant transaction already acquires the shared mode of one
transaction-scoped tenant admission advisory lock before reading product state
([ADR-0146](adr-0146-session-source-control-and-admission.md)). The
epoch-3 catalogue at head `0013` holds 115 tenant-bearing tables, every one
with `tenant_id` and forced RLS, and seven deployment tables. Its only foreign
key cycles are seven `INITIALLY DEFERRED` pairs; `scopes` and
`source_observations` reference themselves. About 170 triggers enforce
append-only rows and closed transitions, and several of them refuse deletion
unconditionally.

The sealed tenant export (`synveda-context-export-2`) holds Knowledge history
and the audit chain, is assembled in memory, and has no import. Knowledge
Forget and Session/artifact cleanup erase aggregates through owner-only
`SECURITY DEFINER` functions gated by a running durable operation. Searchable
Knowledge is not application-encrypted, so destroying a tenant key cannot
prove tenant erasure. Retention, legal holds, certificate contents and support
authority have no owner decision.

## Decision

### 1. Authority

Suspension, resumption, inventory, archive export and archive import are
deployment-operator acts. They run through the local CLI under the exact
migrator principal, proved on the same connection as every write (the
admission seam), at the current schema epoch and migration head. Each chains a
break-glass event on the affected tenant with the operating-system user and a
closed reason code (`incident`, `maintenance`, `offboarding`, `recovery`,
`administrative`); free text is not accepted. A tenant's own principals cannot
resume it: suspension makes their credentials unresolvable, and the runtime
roles hold only `SELECT` on `tenants`.

Archive export and import are the exception to the migrator principal: they
run as a configured database administrator from the role contract, proved on
the same transaction to be that login, not entered through `SET ROLE`, and to
bypass row security. Thirteen tables carry restrictive source-eligibility
policies that hide withdrawn, suppressed or erased rows from every product
role, including the forced-RLS owner, yet those rows are retained evidence and
foreign-key parents. Only a bypassing reader sees the whole tenant; every
statement still names the tenant explicitly. This is the deployment's existing
logical-backup and reset authority, not a new grant. Import additionally
suspends table triggers and writes every table, so the supported envelope is
an administrator that is a superuser or owns the tables; source evidence uses
the bundled superuser only.

These acts remain distinct from admission. Admission creates a row and never
transitions it; lifecycle commands never create a row except by restoring a
verified archive, and each transition names its required prior state. Cedar
continues to decide every tenant read and write through the public API.
Lifecycle acts never return tenant content except inside an archive sealed for
that tenant.

### 2. States and suspension

`tenants.status` keeps `active` and `suspended`. Suspend and resume are
idempotent: repeating a completed transition reports the current state and
chains nothing.

Suspension sets a bounded lock wait, acquires the *exclusive* tenant admission
lock, updates the row, chains `tenant.suspended` and commits. The lock waits
for every transaction admitted earlier; a busy tenant refuses with a stable
`tenant_busy` reason and changes nothing. After commit:

- every new request, fresh login, console session, refresh and service bearer
  fails tenant resolution with the existing uniform 401 on every replica;
- every ordinary tenant transaction re-reads the status after acquiring the
  shared admission lock and refuses with a typed *tenant unavailable* error,
  so an already-resolved request or a worker effect after a provider call
  cannot commit. The two transaction-local settings fold into one statement,
  keeping three statements per admission;
- worker sweeps already enumerate active tenants only. A lease held when
  suspension commits expires normally and is reclaimed after resumption; at
  most the attempt that was in flight is consumed;
- responses committed before suspension and bytes already delivered are not
  recalled.

A best-effort audit append for a refused resolution uses a separate
evidence-only transaction that takes the shared lock but does not require an
active tenant; it writes only the content-free audit chain.

Resumption repeats the exclusive lock and then revalidates authority before
flipping the row: the stored Cedar packs for that tenant must compile with
this build, and every stored tenant key generation must unwrap under the
configured KMS. A tenant that would fail the process-wide policy freshness
sweep, or whose sealed secrets cannot open, stays suspended with a stable
reason (`policy_invalid`, `key_unavailable`).

### 3. Inventory and coverage

A static classification in the store names every tenant-bearing table, its
archive treatment (`portable`, `derived` or `excluded`), its restore position
and any parent-first export order. A database test compares it with the live
catalogue and fails when a table with `tenant_id` is unclassified, a classified
table is missing, forced RLS is absent, or the restore order violates a
non-deferrable foreign key. The dry-run inventory counts each table for one
tenant under forced RLS with a per-table cap, reports deployment-table and
external-object references, and changes nothing.

Derived data is excluded from the archive and rebuilt after restore: Knowledge
revision embeddings are reconverged by the existing index sweep, and the
generated full-text column is recomputed by PostgreSQL. Console session index
rows are excluded because they point at deployment-sealed bearer custody;
restored users sign in again. Application-owned files under the Git export
root are re-exportable from governed evidence and are not archived. Remote Git
repositories, identity-provider accounts, delivered client bytes, previous
exports and deployment backups are customer- or deployment-owned copies
outside this operation and are named, never changed.

### 4. Archive format `synveda.tenant-archive.v1`

The archive is one file written to a private temporary path and published by
an atomic rename only after its final frame is durable. It contains:

1. an eight-byte magic and a bounded cleartext JSON header: format, archive ID,
   tenant ID and slug, schema epoch, baseline revision and migration head,
   product version, database snapshot time, the tenant key generation and its
   KEK-wrapped data key with its KEK reference, and the declared frame bound;
2. a fresh per-archive data key sealed by that tenant key generation for this
   archive ID;
3. length-prefixed frames sealed by the archive key with the existing
   XChaCha20-Poly1305 envelope. Each frame's row binding names the archive ID,
   the header digest, its index and whether it is the final frame, so a
   changed header, reordered, duplicated, truncated or appended frame refuses;
4. data frames holding PostgreSQL COPY text for one table segment, never
   splitting a row, and a final manifest frame with every table's ordered
   column list, row count and BLAKE3 digest, the frozen audit head and the
   total row count.

Export reads one administrator `REPEATABLE READ READ ONLY` snapshot, pins COPY formatting
settings, orders rows by primary key (parents first where a table references
itself) and bounds frame bytes, total bytes and wall time. Memory is bounded
by one frame. An interrupted export leaves no published archive; rerunning
with the same output path either finds a complete archive and verifies it or
writes a new one. The audit chain in the archive is the exact frozen prefix up
to the snapshot head.

The archive is openable only where the KMS holds the KEK that wraps the
named tenant key generation. Moving to a deployment with a different KEK uses
the existing custody bundle and KEK replacement
([ADR-0154](adr-0154-joint-pitr-recovery-and-kek-custody.md)); a portable
archive does not carry an independent recovery key.

### 5. Restore target and import

The only supported target is the same tenant ID in a deployment at the same
schema epoch, baseline revision and migration head. Restore onto the same
PostgreSQL major and lineage is qualified by source tests; another admitted
major or the other lineage is accepted by the format but remains
unqualified until evidence exists. Tenant-ID or slug collision,
remapping, a different schema identity and an unavailable KEK refuse before
any write.

Import first authenticates and checks every frame, the manifest, the declared
bounds and the target's column lists without writing. It requires the
gateway and worker sessions to be drained, as an installed-schema migration
does. It then restores in one administrator transaction: insert the tenant as
`suspended`, take the exclusive admission lock, defer deferrable constraints,
suspend user triggers on the restored tables, COPY every table in restore
order, check the deferred constraints, re-enable the triggers, recompute every
table's ordered COPY digest and row count in the same transaction, re-verify
the audit chain to exactly the frozen head and chain `tenant.restored` after
it. Suspending triggers is required because about 55 insert triggers derive
projection rows the archive already carries exactly, and lease-guard triggers
refuse historical rows outside their original operation. Other sessions never
observe the suspension: it is reversed inside the transaction, and the drained
writers would block on the table locks anyway. Any difference rolls back; an interrupted
import leaves nothing to clean and may simply be rerun. A rerun against an
already restored tenant whose chain records the same archive and manifest
digests reports the existing restore; anything else is a collision. The tenant
stays suspended until an operator resumes it after independent verification
through ordinary roles. The source is never changed by export or import.

### 6. Erasure

Erasure is irreversible deletion of a suspended tenant's rows in a reviewed
dependency order, in bounded steps under the tenant's durable operation lease
and attempt fence, with a receipt derived from measured deletions and a
residual inventory. It requires a schema change: two further states
(`erasing`, `erased`), a lifecycle operation kind, and a narrow owner-only
delete permission for that operation inside the trigger functions that
currently refuse deletion. It honours the existing revision `legal_hold` and
`retention_hold` flags by stopping in a held state.

Until the owner decides retention, holds, certificate contents and support
authority, erasure is exercised only against synthetic fixtures with these
explicit rules: the retained evidence is the tenant tombstone row (ID, slug
and status), the content-free audit chain and the lifecycle operation and
attempt rows; everything else, including Knowledge erasure tombstones,
is deleted; deployment backups, archives and remote copies are named in the
receipt and left unchanged. No production erasure command ships before those
decisions are recorded in an amendment to this ADR.

## Options considered

1. **Operator CLI under the migrator principal with a choke-point status
   check (chosen)** — reuses admission's exact role proof, the admission lock
   and break-glass attribution; one indexed lookup per tenant transaction with
   no extra round trip covers handlers and every worker effect.
2. **Check tenant status at each worker effect** — smaller hot-path change, but
   leaves already-resolved requests unfenced and depends on every future effect
   remembering the check.
3. **Public tenant-administrator suspend API** — useful for incident response,
   but resumption still needs an out-of-band authority, and no Cedar principal
   exists above a tenant. Deferred until an owner asks for it.
4. **Restore with triggers active, or with `session_replication_role`** —
   active projection and guard triggers duplicate or refuse archived rows, and
   the replication role also disables foreign-key enforcement. Transactional
   trigger suspension with writers drained keeps foreign keys enforced.
5. **Archive as the forced-RLS migrator** — silently omits rows hidden by the
   restrictive eligibility policies. A policy-level lifecycle authorization on
   those policies is the reversal path if an administrator credential must not
   be used.
6. **Multi-transaction staged import with a restoring state** — gives chunk
   resume, but needs erasure-grade cleanup for an abandoned stage. A single
   transaction is atomic and bounded by the declared archive size.
7. **Extend `synveda-context-export-2`** — it is in-memory, Knowledge-only and
   has no import contract; it remains readable but is not the portable format.
8. **Crypto-shredding as erasure** — searchable rows are not application
   encrypted; rejected.

## Consequences

- Positive: suspension is effective for new and in-flight work on every replica
  without a cache or new process; restoration is atomic and proves exact rows,
  revisions, provenance and audit prefix; the coverage test fails when a new
  tenant-bearing table is added without classification.
- Negative / accepted trade-offs: every ordinary tenant transaction performs
  one more indexed read; archive export and import need the database
  administrator credential; import needs writers drained and holds one
  transaction for the archive's size;
  a busy tenant can refuse suspension until its admitted work drains; only the
  same tenant ID at the same schema head is restorable; restored console
  sessions are not portable.
- Reversal trigger: a measured p99 regression in tenant transaction admission
  above the context budget, an archive above the declared single-transaction
  bound, or an owner requirement for cross-head or remapped restore.

## Compliance notes

Lifecycle events carry tenant, archive and operation identifiers, counts,
digests, reason codes and the operating-system user, never tenant content,
paths or secret material. Forced RLS confines suspension, resumption and
inventory to the tenant GUC. Archive export and import run as a configured
database administrator that bypasses row security; there, every statement
carries an explicit tenant predicate and the tenant GUC is still set. No
runtime role gains a privilege. The archive
is sealed under keys the KMS already custodies.
