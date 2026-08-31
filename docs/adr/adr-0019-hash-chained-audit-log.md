# ADR-0019: Hash-chained audit log — per-tenant BLAKE3 chains, in-transaction append, one event per audited operation

- **Status**: Accepted; key-provision exception amended by ADR-0064
- **Date**: 2026-07-19
- **Feature(s)**: AUD-1
- **Deciders**: sujitn

## Context

AUD-1 delivers seed §2.5: audit is a first-class output — every decision,
injection, recall, write, and policy change lands in a tamper-evident log.
The AC: mutating any historic row breaks chain verification. Eight features
carried "AUD-1 emission point" deferrals (ADR-0008/0009/0011/0012/0013/
0014/0015/0018, tracked in STATUS.md); this feature pays them off so the
data path (MEM-1, CTX-1..3) is born audited.

Forces at play:

- **The layering constrains the writer.** `synveda-audit` sits beside
  `policy`/`store`/`identity` and may depend only on `types`; the PDP and
  the store cannot call it. The gateway (and the CLI break-glass) are the
  only components that see actor, tenant, decision, and transaction
  together — emission is a gateway seam, like enforcement is a PDP seam.
- **An action without its audit record must not exist.** The only
  mechanism that guarantees this is the database transaction already
  wrapping every mutation (`rls::begin_tenant_tx`, ADR-0009): the event
  commits with the action or neither commits.
- **Denials roll back.** A handler's transaction is dropped on the deny
  path, so a deny event appended inside it would vanish with the rollback
  it is meant to record. Deny-path events need their own transaction.
- **Hash chains serialise.** Event N's hash covers event N−1's, so
  appends within one tenant cannot be computed concurrently. Whatever
  locks the chain head holds that lock until commit — acceptable on the
  admin plane, a real concern for the CTX-3 hot path (p99 <150ms at 1k
  sessions, seed §10).
- **jsonb is not byte-stable.** Postgres normalises jsonb (key order,
  duplicate keys, numeric rendering); hashing whatever bytes come back
  from the database is not reproducible. The hash input must be a
  canonical form both append and verify compute independently. A related
  trap: serde_json's map ordering flips from BTreeMap to insertion-order
  if any crate in the workspace enables `preserve_order` — feature
  unification would silently change our byte stream.
- **Unauthenticated traffic is unattributable and unbounded.** Recording
  failed-credential noise into a tenant chain would let anyone with a
  network path inflate and contend a tenant's audit log.
- **Tamper-evident is not tamper-proof.** A principal holding the
  database credentials can rewrite anything, chain included, by
  recomputing hashes. The chain proves integrity to a verifier who trusts
  a chain head obtained out of band; anchoring heads externally is AUD-3
  (WORM export), not AUD-1.

## Decision

Audit events form one BLAKE3 hash chain per tenant in an append-only,
RLS-forced `audit_log` table; the gateway appends exactly one event per
audited operation inside the operation's own tenant transaction (deny
paths use a short dedicated transaction), and verification recomputes the
chain from a canonical serialisation that never round-trips through jsonb.

1. **One chain per tenant, headed by a locked row.** `audit_log`
   (`tenant_id, seq` primary key) holds events; `audit_chain_heads`
   (`tenant_id` primary key, `seq`, `head_hash`) holds each chain's tip.
   Append locks the head `FOR UPDATE` (creating it on first use with
   `seq = 0` and the genesis hash), inserts the event at `seq + 1`, and
   advances the head — all inside the caller's transaction, so a rollback
   retracts the event, the head move, and the lock together. The genesis
   hash is `BLAKE3("synveda-audit-genesis-v1" ‖ tenant uuid)`: chains are
   bound to their tenant and cannot be transplanted. The head row is
   locked last in every transaction (after all other row locks) so
   lock order stays consistent across handlers.
2. **The hash covers a canonical serialisation, computed — never
   stored round-tripped.** `hash = BLAKE3("synveda-audit-event-v1" ‖
   prev_hash ‖ canonical(event))` where `canonical` is our own
   serialiser: UTF-8 JSON with object keys sorted bytewise at every
   depth (immune to `preserve_order` feature unification), timestamps as
   RFC 3339 UTC with exactly microsecond precision (truncated in Rust
   before insert, so the timestamptz round-trip is exact), and no
   non-integer numbers anywhere (append rejects float-bearing payloads).
   Verification reads typed columns plus the jsonb payload, re-canonises,
   and recomputes; any historic mutation — content, order, linkage, a
   gap, or a moved head — is named by tenant and sequence number.
3. **Append-only is enforced in the schema, not promised by the app.**
   Both tables are RLS-forced on the `synveda.tenant_id` GUC like every
   tenant-scoped table (ADR-0009). `synveda_app` gets INSERT/SELECT on
   `audit_log` (no UPDATE, DELETE, or TRUNCATE) and INSERT/SELECT/UPDATE
   on `audit_chain_heads`; BEFORE UPDATE/DELETE/TRUNCATE triggers raise
   unconditionally on `audit_log`, so even the table owner must disable
   triggers to mutate history — which is exactly what the AC tamper test
   does to simulate a database-level attacker, and what verification then
   catches. `audit_log` carries no foreign key to `tenants`: the chain
   outlives tenant lifecycle transitions (TEN-5 retention/erasure will
   govern audit disposal explicitly, with its own destruction
   certificate).
4. **One event per audited operation, carrying its decision.** A
   mutation emits one semantic event (`hierarchy.node.created`,
   `role.bound`, `service_identity.revoked`, ...) whose payload embeds
   the authorizing decision's context (pack name@version, determining
   policies, effective roles) — the decision is recorded without a
   second row. Standalone `authz.decision` events exist where no
   semantic event does: every denial, and every allowed admin-plane
   read. Read handlers therefore commit their transactions (the read
   event is a write). Per-candidate composition sweeps (CTX-2's
   `MemoryRead` fan-out) will aggregate into the single inject event —
   the request-level record, with candidate decisions summarised — not
   one chain row per candidate; the full per-call detail stays in the
   structured decision log and traces (ADR-0012), which remain
   unchanged.
5. **Success events append at the mutation seam; error events append at
   the `respond` seam.** Each mutating handler appends inside its tenant
   transaction immediately before commit. The per-plane `respond`
   helpers — the funnel every handler result already flows through —
   classify errors and append in a fresh short tenant transaction (the
   handler's own transaction is already rolled back): `PolicyDenied` →
   `authz.decision`/deny; the store's RLS-backstop marker (a new
   `rls::is_backstop_trip` helper interprets the `Error::Internal`
   message — the taxonomy stays coarse per FND-3) → `store.rls.denied`;
   a service-token seam rejection → `auth.token.rejected`. If the audit
   append itself fails on this path, the original error still reaches
   the caller — the failure is logged and counted
   (`synveda_audit_append_failures_total`), never masked.
6. **Only attributable events enter a chain.** Tenant resolution
   failures for verified tokens naming a suspended tenant are audited on
   that tenant's chain (`tenant.resolution.denied`); resolution successes
   are not events (every subsequent event proves resolution), and
   unauthenticated failures — no verified subject, no resolvable tenant —
   stay in metrics and traces where they live today. JIT provisioning
   audits `identity.provisioned` only when an identity row is created
   (mapped, admin, or quarantined outcome — not `existing` logins), and
   the admin-group binding upsert audits `role.bound` only when the
   binding row is first created, not on every login's no-op upsert.
7. **The CLI break-glass audits itself.** Every mutating CLI command
   (`tenant create`, `policy apply/clear`, `role bind/unbind`,
   `service register/remove`) appends its event in the same transaction
   as its write, with actor kind `break_glass` and the OS username as
   best-effort subject. Actor kinds are exactly `subject` (any
   authenticated bearer — whether it was a user or a service identity is
   the identities table's knowledge, joined at query time by AUD-2) and
   `break_glass` (unauthenticated store-level access; attribution is
   honest about being weaker there). Events carry the OTel trace id when
   one is live, linking every chain row to its trace.
   ADR-0064 amendment 3 narrows this rule only for external-KMS key
   provisioning: the key commit is followed by a chain-head-serialized exact
   witness transaction whose rerun repairs the crash gap after proving unwrap
   custody. Every other CLI mutation retains same-transaction append.
8. **The audit crate is functions, not a sink.** `synveda_audit::append`
   takes the caller's connection (`&mut PgConnection`) plus tenant and
   event; `verify` walks a tenant's chain in one snapshot and reports the
   first divergence; `tail` serves the CLI and demo. No `AppState` field,
   no trait object, no background machinery. `blake3` joins the workspace
   dependencies (Apache-2.0; VedaFlow's content addressing needs it
   anyway, tech plan §2.1). Spans (`audit.append`, `audit.verify`) and
   metrics (`synveda_audit_events_total{action,outcome}`,
   `synveda_audit_append_failures_total`) follow ADR-0007: the facade
   below, the recorder and descriptions in the gateway.

## Options considered

1. **In-transaction append with a locked chain head (chosen)** —
   atomicity with the action it records, rollback-safe sequencing,
   boring SQL. Con: per-tenant append serialisation; the head lock is
   held until commit. Acceptable on the admin plane where transactions
   are short; the hot read path gets an explicit upgrade path (below).
2. **Async buffered appender (channel + single writer)** — no lock on
   the request path, group-commit throughput. Rejected for AUD-1: an
   acknowledged mutation could crash before its event persists,
   breaking "no action without its record" — the property the feature
   exists to establish. Recorded as the CTX-3 upgrade path for
   *decision* events only (read-path decisions are not mutations;
   bounded, metric-visible loss on crash is a defensible trade there —
   mutation events stay in-transaction forever).
3. **Postgres triggers computing the chain in-database** — no
   application code path could forget to chain. Rejected: the hash
   function would live in the database (BLAKE3 means an untrusted
   extension or plpgsql reimplementation; both violate the boring-stack
   rule), and events would be assembled from row images rather than the
   richer request context (actor, decision, trace) only the gateway has.
4. **Hashing the stored jsonb directly** — no canonical serialiser to
   maintain. Rejected: jsonb normalisation makes the byte stream
   unreproducible across write and read; verification would have to
   trust Postgres's rendering stability across versions — exactly the
   wrong trust anchor for a tamper-evidence feature.
5. **One global chain instead of per-tenant** — simpler head
   management, total order across the deployment. Rejected: every
   tenant's appends would contend on one lock; export/erasure per tenant
   (TEN-5, AUD-3) would need chain surgery; and RLS-scoped verification
   per tenant would be impossible.
6. **A reserved system tenant chaining unauthenticated failures** —
   completeness, but records attacker-controlled garbage into a
   hash-chained, append-only store: an unauthenticated flood becomes
   permanent storage growth and head-lock contention. Rejected;
   pre-authentication visibility stays in metrics/traces (decision 6).
7. **Auditing every PDP call as its own event** — maximal fidelity,
   but the admin plane makes several authorize calls per request
   (uniform-404 probes, assign-side checks) and CTX-2 will make one per
   candidate record; chains would grow by an order of magnitude with no
   added answerability — "who did what" is one operation. The decision
   log already records every call (ADR-0012). Rejected in favour of
   decision 4.

## Consequences

- Positive: the eight deferred emission points close; every admin
  mutation, denial, provisioning, and break-glass action is now a
  chained, tenant-scoped, verifiable record carrying its decision
  context; MEM-1 and CTX-1..3 land on a live audit seam instead of
  retrofitting one; the AC's tamper-evidence is enforced by schema +
  verification, not convention.
- Negative / accepted trade-offs: admin-plane requests gain one chain
  append (and reads gain a commit); appends within a tenant serialise on
  the head lock; deny-path events cost a second short transaction;
  break-glass attribution is only as strong as OS-level identity; the
  canonical serialiser is ours to maintain (fenced by unit tests and the
  tamper suite); chain verification is O(chain length) — fine for the
  CLI and CI, AUD-3 owns scalable offline verification.
- Reversal trigger: if CTX-3's latency AC shows the head lock or the
  synchronous append dominating inject p99, activate option 2's buffered
  appender for read-path decision events (mutation events stay
  in-transaction); if canonical-serialisation maintenance produces a
  second incompatibility bug, revisit hashing a stored canonical text
  column as the source of truth instead.

## Compliance notes

Seed §2.5 lands: allow/deny decisions, writes, and policy changes are
recorded tamper-evidently; §2.2 is untouched — emission observes
decisions, it never makes them, and no PDP bypass is introduced (the
tamper test mutates rows via raw SQL with triggers disabled, explicitly
modelling a database-credentialed attacker, not an API path). Tenant
isolation: both audit tables are RLS-forced on the same GUC as every
tenant-scoped table; the TEN-2 adversarial suite extends to them; chains
never cross tenants and genesis hashes bind each chain to its tenant.
The audit log is itself auditable: verification is deterministic,
side-effect-free, and exposed via `synveda audit verify` for operators
and auditors ahead of AUD-2's query surface.
