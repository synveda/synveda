# ADR-0020: Observe ingestion — RLS-staged events, PGMQ work signals, buffer-level idempotency

- **Status**: Accepted
- **Date**: 2026-07-19
- **Feature(s)**: MEM-1
- **Deciders**: sujitn

## Context

MEM-1 opens the data plane: `observe` is the write primitive (seed §3) —
"here is what happened", batched, acked in <20ms, never blocking the
session. The tech plan fixes the buffer (PGMQ, §1.1) and the path
(gateway authZ → enqueue → async pipeline, §3). The AC: a load test
sustains 1k events/s on dev hardware, and duplicate delivery does not
duplicate memories.

Forces at play:

- **Duplication originates at the client.** A hook that times out
  waiting for an ack retries; at-least-once delivery is the only honest
  contract. The documented Mem0 failure modes (features doc §A1) include
  silent duplication and ADD-only stores; MEM-5's semantic dedup handles
  near-duplicate *content*, but transport-level redelivery must die at
  the door — once two copies of one delivery enter the pipeline, every
  downstream stage has to out-guess them.
- **The pipeline behind the buffer does not exist yet.** MEM-2
  (redaction), MEM-3 (extraction), MEM-4 (embed-or-fail) land later.
  "Does not duplicate memories" can only be discharged now by a
  structural property of the buffer: what never enters twice can never
  be extracted twice.
- **PGMQ tables are not tenant-scoped.** The RLS backstop (ADR-0009)
  covers tables with a `tenant_id` column, reached via
  `rls::begin_tenant_tx`. `pgmq.q_*` tables have PGMQ's own shape and
  live outside that discipline. Transcript deltas are exactly the
  content the backstop exists to protect — sensitive by default, PII
  until MEM-2 says otherwise.
- **Idempotency needs a unique index; a queue has none.** PGMQ offers
  at-least-once delivery, not exactly-once admission. Whatever enforces
  "one idempotency key, one event" is a constraint in a real table.
- **The audit seam is live and binding.** ADR-0019 names MEM-1's
  observe an emission point: one chained event per audited operation,
  appended inside the operation's own tenant transaction. Chain appends
  serialise per tenant on the head lock — a throughput bound the ack
  path inherits.
- **Zero-config governs who may write.** A JIT-provisioned user holds
  no role bindings (ADR-0015), yet their sessions must generate
  memories (seed §2.1). ADR-0015 deferred "contributor writes" to
  MEM-1; ADR-0018's base-layer confinement forbids a service token
  everything outside its anchor subtree, with a single `MemoryRead`
  carve-out — a write action must compose with that forbid, not widen
  it.
- **Placement already answers "where".** The lifecycle (tech plan §2.3)
  commits extracted memories to the *principal's* derived channel;
  every placed principal — user or service — has a personal leaf scope
  (ADR-0013, ADR-0018 decision 2), and the Cedar principal carries it
  as `home` (ADR-0014).

## Decision

Observe events land in a tenant-scoped, RLS-forced, append-only staging
table inside the caller's tenant transaction; a single PGMQ queue
carries content-free work signals; idempotency is a unique key on the
staging table with first-writer-wins semantics; and the PDP gates the
batch with a new `MemoryWrite` action whose floor is the principal's
own personal scope.

1. **Content under RLS; the queue carries pointers.** Migration 0012
   creates `observe_events` (`id` UUIDv7, `tenant_id`, `scope_id`,
   `owner_id`, `session_id`, `idempotency_key`, `kind`, `payload`
   jsonb, `occurred_at`, `received_at`), forced-RLS on the
   `synveda.tenant_id` GUC with the standard policy, per the ADR-0009
   structural rule, and covered by the RLS completeness guard. The same
   migration runs `pgmq.create('observe')` and grants `synveda_app` the
   queue's DML. A queue message is `{tenant_id, event_id}` — nothing
   else. The staging row is the raw-observation record and the
   pipeline's provenance source; `synveda_app` gets SELECT and INSERT
   only — no UPDATE, DELETE, or TRUNCATE — so the app cannot rewrite
   what was observed. Delivery state lives in PGMQ (read/archive);
   content disposal is MEM-6/TEN-5 territory and gets its own grants
   when it lands.
2. **Idempotency at the buffer, first-writer-wins, duplicate = success.**
   `unique (tenant_id, idempotency_key)`; the batch insert is
   `ON CONFLICT DO NOTHING RETURNING`, and only rows actually inserted
   are enqueued — a redelivered event cannot reach the pipeline twice,
   which is the AC discharged structurally rather than behaviourally.
   Duplicates are reported per event (`status: duplicate`, carrying the
   *original* event's id, resolved in the same transaction) and the
   batch acks 202: for an at-least-once client, redelivery is the
   success case, never an error. A key seen with different content is
   still a duplicate — the first delivery won; retry-with-mutation is a
   client defect surfaced by the per-event report. Keys are
   client-supplied and mandatory: only the client can distinguish a
   retry from a genuinely new event with identical content.
3. **`MemoryWrite` joins the vocabulary; packs bump to `@4`.** The
   schema gains `action MemoryWrite appliesTo { resource: [Scope] }`.
   All three product packs add two rules, identical across packs:
   - *The floor, role-free*: `principal has home && resource ==
     principal.home` — every placed principal may write its own
     personal scope; nothing else. Deliberately narrower than the
     `MemoryRead` floor (`principal in resource`): reading composes up
     the chain, writing lands at home. An unplaced principal has no
     `home` and is denied — fail closed.
   - *The content-role grant*: `resource in principal.tenant &&
     context.roles.containsAny(["contributor", "curator"]) &&
     resource.kind != "user"` — writes beyond home require an explicit
     content-role binding on the resource's chain, and foreign personal
     scopes stay closed. This discharges ADR-0015's contributor-writes
     marker. Steward/org-admin/auditor/viewer grant no content write —
     the same least-privilege doctrine the read rules already follow.
     Write governance is pack-uniform: sharing *defaults* differ per
     pack on the read side; writes beyond home always take an explicit
     grant. If a pack ever wants looser write defaults, that is a pack
     change, not a vocabulary change.
   The base layer is untouched: a service identity's home leaf and any
   scope a role could grant lie inside or outside its anchor subtree
   exactly as the confinement forbid already decides; the `MemoryRead`
   carve-out stays the only carve-out.
4. **Observe writes home, and only home, in MEM-1.** `POST /v1/observe`
   takes no scope parameter; the resource is the caller's placement
   leaf, resolved at the enforcement seam like every governed request.
   Events are stamped with that `scope_id` and the caller's identity as
   `owner_id`. Team/department-scoped writes arrive with VedaFlow
   promotion (FLOW-3+) through the grant in decision 3 — the policy
   exists before the surface, the AUTHZ-2 precedent.
5. **One chained `memory.observed` event per batch.** Appended inside
   the ingest transaction immediately before commit (ADR-0019
   decisions 4/5), payload carrying the authorizing decision context,
   `session_id`, accepted/duplicate counts, and the first and last
   accepted event ids (UUIDv7 is time-ordered; the pair brackets the
   batch's staging rows without writing hundreds of ids into the
   chain). An all-duplicates batch still chains — it is an operation
   with an outcome. Denials ride the existing `respond` rejection seam
   unchanged. The API surface: batch caps of 256 events and 64 KiB
   payload per event, keys and session ids ≤ 200 chars, kinds exactly
   `transcript_delta | tool_result | decision` (seed §3), a 20 MiB
   body limit on this route, all-or-nothing validation (a malformed
   batch is 422 with the offending index; nothing partial persists).
6. **The ack path is enqueue-only and the throughput bound is named.**
   Per batch: validate in-process → `begin_tenant_tx` → one multi-row
   `unnest` insert → one duplicate-resolve select (skipped when no
   duplicates) → one `pgmq.send_batch` → audit append → commit. No
   embedding, no extraction, no LLM, no network beyond Postgres. The
   per-tenant chain head serialises concurrent same-tenant batches;
   sustained 1k events/s arrives as ~10 paced 100-event batches/s,
   well inside the lock's budget, and the AC load test models exactly
   that (paced open-loop, not a synchronized burst). If a real
   deployment needs concurrent per-tenant bursts beyond this, the
   recorded upgrade is ADR-0019 option 2's buffered appender — for
   read-path decision events; observe events are mutations and stay
   in-transaction.
7. **The consumer contract is forward-declared.** MEM-2/3 workers
   `pgmq.read` the signal, open `begin_tenant_tx` for the named tenant,
   load the staging row, process, and `pgmq.archive` the message.
   Visibility timeouts and retry policy are the consumer's (Temporal's)
   concern; the buffer's only promises are durability, ordering per
   enqueue, and single admission per idempotency key.

## Options considered

1. **RLS-staged content + pointer queue (chosen)** — content stays
   under the tenant backstop, idempotency gets a real constraint, the
   queue does the one thing queues do well. Con: two writes per event
   (row + signal) and a staging table that grows until MEM-6; both
   accepted — the writes share one transaction, and growth is the
   provenance record the pipeline needs anyway.
2. **Payload in the queue message** — one write, the "obvious" PGMQ
   shape. Rejected: transcript content would live in a non-RLS table
   outside `begin_tenant_tx` discipline, invisible to the completeness
   guard; and admission dedup would still need a keyed table, so the
   simplicity is illusory.
3. **Per-tenant queues** — stronger isolation optics. Rejected: queue
   count tracks tenant count with create/drop lifecycle to manage,
   consumers must discover queues dynamically, and it buys nothing once
   messages carry no content.
4. **Duplicate → 409 Conflict** — the `records` 23505 precedent.
   Rejected: for at-least-once delivery a retry is *correct client
   behaviour*; erroring it teaches clients to ignore errors. 202 with
   per-event status keeps the ack idempotent.
5. **Defer all dedup to MEM-5** — one dedup mechanism instead of two.
   Rejected: MEM-5 is semantic (embedding/minhash near-dup of content);
   transport redelivery is exact, cheap to kill at admission, and the
   field-report failure mode says relying on downstream ranking to
   absorb duplicates is how systems rot.
6. **Role-gated observe (contributor required)** — a literal reading of
   ADR-0015's marker. Rejected: JIT users hold no bindings; requiring a
   role for personal-scope writes breaks zero-config (seed §2.1). The
   marker is discharged at shared scopes, where an explicit grant is
   exactly right.
7. **`MemoryWrite` floor as own chain (`principal in resource`)** —
   symmetric with the read floor. Rejected: it would let any member
   write at team/department/org without any grant; reads compose
   upward, writes do not.
8. **Server-derived idempotency keys (content hash fallback)** — saves
   adapters a field. Rejected: identical content at different moments
   is legitimately two events; only the sender knows the difference, so
   the key is the sender's to mint.

## Consequences

- Positive: the pipeline is born idempotent and born audited — MEM-2/3
  consume a buffer that has already shed redelivery noise and chained
  every admission; content never leaves RLS coverage; the write seam
  (`MemoryWrite`) exists with its floor and grant semantics before any
  richer write surface, mirroring how `MemoryRead` preceded CTX; the
  queue infrastructure was already deployed and smoke-tested, so MEM-1
  adds no new moving part.
- Negative / accepted trade-offs: every event costs a staging row plus
  a queue row (one transaction, two tables); per-tenant chain appends
  serialise observe batches — burst absorption is bounded until the
  recorded upgrade; the idempotency horizon equals staging-row
  lifetime, so MEM-6/TEN-5 disposal must account for the dedup window
  when it defines retention (recorded here as an obligation on those
  features); staging rows hold pre-redaction content until MEM-2
  inserts itself between buffer and extraction — they are
  tenant-isolated and app-immutable, but redaction-before-persistence
  (seed §6) is honestly *not yet true* and lands with MEM-2.
- Reversal trigger: if paced-batch throughput stops covering real
  harness traffic (many small unbatchable acks per tenant per second),
  activate the buffered appender for observe's audit events per
  ADR-0019 option 2's boundary discussion — or revisit batch shaping in
  the adapters before touching audit semantics. If `unnest` insert cost
  dominates at TEN-3 partition scale, revisit COPY-based admission.

## Compliance notes

Seed §2.2 holds: every observe batch passes `Pdp::authorize` with a
versioned action; there is no path from adapter to storage that skips
it, and tests exercise denial through the same facade (quarantined,
unplaced, out-of-anchor service tokens). The RLS structural rule
(ADR-0009) is satisfied in the same migration that creates the table,
and the completeness guard extends to `observe_events`; PGMQ's tables
deliberately carry no tenant content, which is why they may live
outside the GUC discipline. ADR-0019's emission obligation is met: one
`memory.observed` event per batch, in-transaction, with deny paths on
the existing rejection seam. The ack SLO is asserted by the AC load
test at 1k events/s with the delta-over-baseline discipline available
if dev-hardware jitter demands it.
