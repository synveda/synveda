# ADR-0030: VedaFlow object store — typed content addressing, FK-closed history, compare-and-swap refs

- **Status**: Accepted
- **Date**: 2026-07-25
- **Feature(s)**: FLOW-1
- **Deciders**: sujitn

## Context

FLOW-1 is the substrate ADR-0003 committed to: "BLAKE3 content-addressed
objects/trees/commits/refs in Postgres; commits record author identity,
signature, and policy-pack snapshot hash. AC: property tests — identical
content dedups; history immutable under concurrent writers." ADR-0003
already settled *where* (native Postgres tables, not bare git repos, not a
forge) and *why* (approvals, policy checks, audit chaining, and RLS must be
transactional with the content). What it did not settle is the object model
itself, and that model is load-bearing for six features that follow it:
FLOW-2 turns `refs` rows into the derived/staged/published channels the
inject path reads; FLOW-3 hangs proposals off two refs; FLOW-7 rolls a ref
back; FLOW-8 exports commits to a real git repository with their signatures
intact.

Forces at play:

- **The crate cannot import the things it describes.** `synveda-vedaflow`
  is a middle-tier crate (seed §8, tech plan §5): it may depend on
  `synveda-types` and nothing else Synveda-side. It cannot call the PDP to
  learn which policy pack governed a commit, cannot reach `synveda-audit`
  to chain an event, and cannot use `synveda-store`'s helpers. AUD-1 solved
  the identical problem the identical way (ADR-0019): migrations live in
  `synveda-store`'s one embedded migrator, semantics live in the sibling
  crate, and every operation runs inside a transaction the caller opened
  with `rls::begin_tenant_tx`.
- **"Identical content dedups" and "isolated by RLS" pull in opposite
  directions.** True content addressing is global — the same bytes are the
  same address everywhere. Forced RLS (ADR-0009) requires every row to
  carry exactly one `tenant_id` and be invisible to every other tenant. A
  single shared row cannot satisfy both.
- **"History immutable" has to mean something a hostile DBA cannot undo.**
  AUD-1's tamper test set the bar: the attacker holds database credentials,
  disables triggers, and rewrites rows; the property must be recoverable or
  detectable, not merely conventional. A store where immutability is "the
  Rust API never issues an UPDATE" does not clear that bar.
- **"Under concurrent writers" is a claim about Postgres, not about Rust.**
  Two ingestion workers committing to the same scope's derived channel in
  the same second is the normal case once FLOW-2 lands, not an edge case.
- **Watermarks already exist and are explicitly provisional.** CTX-2 ships
  `version_hash` — BLAKE3 over (record id, `tx_from`, content) — described
  in ADR-0025 decision 7 as "recomputable content addresses FLOW-1's commit
  hashes supersede in place". FLOW-1 must produce something that can
  actually supersede it.
- **The pipeline's write path is already at the seam.** MEM-3 inserts
  records directly (ADR-0022's recorded forward obligation: "FLOW-1/2
  replace the direct records insert with the derived-channel commit"). That
  replacement is FLOW-2's; FLOW-1 must not pre-empt it, and must not ship a
  second write path to records in the meantime.
- **There is no product surface in FLOW-1.** No route, no CLI verb, no
  harness call reaches this code until FLOW-2. Anything shipped here that
  is only reachable from a test is scope that has not earned its place.

## Decision

`synveda-vedaflow` gains the object store: BLAKE3 content addressing over a
typed, length-prefixed canonical encoding; six tables in migration 0018
whose referential closure and immutability are enforced by foreign keys,
grants, and triggers rather than by convention; and ref updates that are
compare-and-swap, fast-forward-checked, and force-only through a separate
call.

Decisions, specifically:

1. **Migrations in `synveda-store`, semantics in `synveda-vedaflow`,
   operations inside the caller's tenant transaction.** Every public
   function takes `&mut PgConnection` and a `TenantId`, exactly like
   `synveda_audit::chain::append`. The caller opened the transaction with
   `rls::begin_tenant_tx`; a caller who did not writes zero rows, because
   forced RLS with an unset GUC matches nothing. This is what makes
   ADR-0003's central claim true in code: a commit, the records it
   describes, and the audit event that attests to it either all land or
   none do.

2. **The hash is `BLAKE3(domain ‖ length-prefixed fields)`, and the tenant
   is not in it.** Git's `type ‖ len ‖ NUL ‖ payload` generalised: one
   domain separator per object kind (`synveda-vedaflow-object-v1`,
   `-tree-v1`, `-commit-v1`), and every variable-length field preceded by
   its length as a big-endian `u64`. The length prefixes are not
   decoration — without them `("ab", "c")` and `("a", "bc")` hash alike,
   and an attacker who controls two adjacent fields controls the address.
   Domain separation per kind means the three hash spaces are disjoint by
   construction, so a tree hash can never be mistaken for a commit hash
   even before the type system gets involved.

   Leaving the tenant out is what keeps the hash a *content* address: an
   auditor holding the bytes, or a git mirror holding the exported object
   (FLOW-8), recomputes the same value with no access to our schema. It
   also makes the AC's property meaningful — with the tenant in the hash,
   "identical content dedups" would be true only in the trivial sense that
   the same row conflicts with itself.

3. **Dedup is within a tenant; storage is keyed `(tenant_id, hash)`.**
   Decision 2's global address, stored per tenant. Two tenants holding the
   same bytes hold two rows with the same hash, and neither can see the
   other's — the only arrangement forced RLS can express. Sharing one row
   would additionally leak: `on conflict do nothing` returning "already
   present" for content the caller never wrote is an oracle for another
   tenant's knowledge, over exactly the boundary this product sells.

4. **`objects.kind` is the asset type, and it is inside the hash.**
   `memory` | `prompt` | `skill` | `context-pack` | `policy` — the four
   managed asset types of seed §4.3 plus policy, which the tech plan §2.3
   makes an asset ("policy packs and lapses are themselves assets flowing
   through VedaFlow"). Identical bytes registered as a prompt and as a
   skill are two different objects, because FLOW-3 resolves required
   approvals from asset type × sensitivity × scope × pack, and a skill is
   executable where a prompt is not. Content that is governed differently
   is not the same content.

5. **Trees and commit parents are child tables with foreign keys, not
   array columns.** Tech plan §2.1 sketches `trees (hash, entries[])` and
   `commits (…, parents[])`; the sketch is a sketch. `vedaflow_tree_entries`
   and `vedaflow_commit_parents` each carry an ordinal and a foreign key to
   the row they reference, so a tree entry pointing at an object that does
   not exist, or a commit claiming a parent that does not exist, is
   *unrepresentable* rather than merely unwritten. Referential closure is
   the half of "history immutable" that an array column cannot give: with
   `bytea[]`, a dangling reference is a bug a batch job might find later.

   A tree entry targets an object or another tree, so scopes can nest.
   Cycles are impossible without a preimage attack: a tree's hash covers
   its children's hashes, so a tree cannot contain itself.

6. **Immutability is schema-enforced, on the `audit_log` pattern.** The
   five history tables (`vedaflow_objects`, `vedaflow_trees`,
   `vedaflow_tree_entries`, `vedaflow_commits`, `vedaflow_commit_parents`)
   grant `synveda_app` SELECT and INSERT only, and carry
   before-UPDATE/DELETE/TRUNCATE triggers that raise for every caller —
   table owner included. `vedaflow_refs` is the one mutable table
   (SELECT/INSERT/UPDATE, never DELETE): a ref is a pointer, and moving it
   is the point. The content-addressed tables need no `updated_at` and have
   none; a row that could be updated is a row whose hash could lie.

   The trigger is not proof against a principal who disables triggers —
   nothing at this layer is. It is what makes tampering require that step,
   and the AUD-1 chain is what makes the step visible once FLOW-2 gives ref
   moves an audit event.

7. **`committed_at` is in the commit hash, truncated to microseconds,
   rendered as RFC 3339 UTC.** The AUD-1 canonical-timestamp rule
   (ADR-0019 decision 2) applied verbatim: hash the value the `timestamptz`
   column will store, so recomputation from the stored row is byte-exact.
   Two commits identical in every other field but made a second apart are
   different commits, which is the git behaviour and the auditable one.

8. **`policy_snapshot_hash` is BLAKE3 over a canonical `PolicySnapshot`
   the caller supplies.** `PolicySnapshot { pack, version, config }` —
   pack name, pack version, and the pack's canonical JSON config — lives in
   `synveda-vedaflow` with its encoding, and the gateway or the pipeline
   builds one from the effective pack it already resolved. Vedaflow cannot
   import `synveda-policy` (decision 1) and must not grow a second notion
   of what a pack is; what it owns is the *rule* for turning one into
   bytes, so every caller produces the same hash for the same pack. The
   canonical JSON is `synveda-audit`'s rule restated locally — keys sorted
   bytewise at every depth, non-integer numbers rejected — because a float
   has no canonical form worth hashing.

9. **Signatures cover the commit hash, through a `CommitSigner` seam with
   an honest default.** `Unsigned` is the default and writes NULL: a commit
   that nobody signed says so, rather than carrying a signature over
   nothing. `Ed25519` signs the 32-byte commit hash — which already covers
   the tree, the parents, the author, the message, the timestamp, and the
   policy snapshot — so verification needs no re-encoding and no schema
   knowledge. `signer_key_id` is stored beside the signature so rotation is
   expressible and so a verifier can find the right public key. The seam is
   the `Extractor`/`Embedder` shape (trait + enum, static dispatch); the
   key arrives as configuration, and key *management* is deferred (TEN-4's
   per-tenant keys are its natural home).

10. **Ref updates are compare-and-swap, and racing is a result, not an
    error.** `update_ref(scope, name, expected, new)` issues
    `update … where commit_hash = $expected` and reads the affected row
    count; `expected: None` means "must not exist yet" and becomes an
    `insert … on conflict do nothing`. Zero rows affected returns
    `RefUpdate::Raced`, which the caller retries by re-reading the ref and
    re-parenting. A last-writer-wins UPDATE is precisely the lost update
    the AC forbids, and an error type would tempt callers to log and
    continue.

11. **Fast-forward is the default and force is a different function.** A
    ref may move only to a commit that has the current commit as an
    ancestor, checked by a recursive CTE over `vedaflow_commit_parents`
    (`union`, not `union all` — the DAG is acyclic by decision 5, and dedup
    bounds the walk). `force_update_ref` exists for FLOW-7's rollback and
    is a separate call with its own name, so no rollback is ever a typo. A
    non-fast-forward move through the normal call returns
    `RefUpdate::NotFastForward` and writes nothing.

12. **The AC's concurrency property is tested against real concurrent
    connections.** Not simulated, not serialised in one transaction:
    N writers on N pooled connections racing to advance one ref, asserting
    that every commit reporting success is reachable from the final ref,
    that the ref's commit count equals the number of successes, and that no
    previously-written commit's bytes changed. "Immutable under concurrent
    writers" is a claim about Postgres; the test has to be Postgres.

13. **Tables are `vedaflow_`-prefixed.** ADR-0003 and tech plan §2.1 write
    them as `objects`, `trees`, `commits`, `refs`. Unqualified in a schema
    that also holds `records`, `identities`, and `policy_packs`, three of
    those four are ambiguous to the point of hazard — `commits` in
    particular reads as a database concept. The prefix names the subsystem
    that owns them, which is also the crate that owns their semantics.

14. **No audit action, no route, no CLI verb in FLOW-1.** There is no
    governed action here yet: the object store is a substrate, and the
    first thing a person or an agent does with it is FLOW-2's channel write
    or FLOW-3's proposal. Adding `vedaflow.ref.updated` now would mean
    inventing its actor, resource, and decision context ahead of the
    surface that produces them. This is the AUD-1-shaped deferral the
    backlog uses throughout, recorded as a forward obligation below rather
    than discharged early.

    The same reasoning covers the metrics. This crate emits
    `synveda_vedaflow_{objects,trees,commits}_written_total`,
    `_ref_updates_total`, and `_verifications_total` through the `metrics`
    facade, as every crate below the gateway does (ADR-0007) — but the
    *descriptions* live in the gateway's recorder, and the gateway does not
    depend on this crate yet because nothing in it calls this crate. FLOW-2
    adds the dependency for real work and describes the counters in the
    same diff; taking the dependency now would link a crate the binary
    never calls.

## Options considered

1. **Typed content addressing over FK-closed tables, CAS refs (chosen)** —
   dedup and referential closure are database properties; the AC's two
   properties are enforced rather than tested-for. Con: six tables and a
   canonical encoding we own and must not change casually (the `-v1`
   suffixes are the migration path).
2. **One polymorphic `objects` table with a `kind` discriminator and JSONB
   payload for trees and commits** — three fewer tables, and git's own model
   (everything is an object). But ADR-0003's stated benefit is SQL over
   refs and commits alongside records and bitemporal history; that becomes
   a JSONB dig, `commits.tree` cannot be a foreign key, and the immutability
   triggers lose the ability to say what they protect.
3. **Array columns exactly as tech plan §2.1 sketches them** (`entries[]`,
   `parents[]`) — fewer joins and closer to the written plan, but a
   dangling entry or parent becomes representable, and the closure that
   makes history verifiable degrades into a convention plus a checker.
4. **Tenant inside the content hash** — every tenant's addresses become
   unique, which sounds like isolation. It is not: RLS already provides
   isolation, and the cost is that the hash stops being recomputable from
   content, which breaks the auditor's independent check, FLOW-8's export,
   and the AC's dedup property in the only sense that matters.
5. **Last-writer-wins ref updates** — one statement, no retry loop for
   callers to get right. It is the lost update the AC exists to forbid; the
   second writer's commit becomes unreachable garbage with no error
   anywhere.
6. **Optimistic refs with a monotonic version integer instead of
   compare-and-swap on the commit hash** — works, and is the usual ORM
   pattern. But the hash *is* the version: a separate counter is a second
   source of truth that can disagree with the DAG, and rollback (FLOW-7)
   would have to move it backwards or forwards arbitrarily.
7. **Defer signatures to FLOW-8, ship a NULL column** — the smaller diff,
   and FLOW-8 is where signatures are consumed. Rejected because ADR-0003
   and the feature text both say commits *record* a signature; a column
   that is structurally always NULL is a promise, not a decision. The seam
   with an honest `Unsigned` default costs little and makes the absence
   explicit rather than accidental.

## Consequences

- **Positive**: identical content dedups because the primary key says so,
  not because a code path checks; a tree entry or commit parent that points
  at nothing cannot be inserted; history cannot be updated or deleted
  through the application role at all; concurrent ref advances cannot lose
  a commit; every commit carries the pack that governed it and, when a key
  is configured, a signature over the whole of it; CTX-2's provisional
  `version_hash` watermark now has the thing that supersedes it.
- **Negative / accepted trade-offs**: we own an object model and its
  encoding, and the `-v1` domain separators are the only migration path if
  it ever changes. The ancestry check is O(history) in the worst case —
  fine at FLOW-1/2 scale, where the overwhelmingly common case is
  depth 1 (the new commit's parent *is* the current ref), and the recorded
  upgrade is a generation number or a closure table on the HIER-1 pattern
  if FLOW-4/5's automated promotions make ref moves hot. Packing and GC
  remain unaddressed, as ADR-0003 anticipated; object growth is the
  trigger. Signing-key management is configuration-shaped for now.
- **Reversal trigger**: none for the substrate — ADR-0003 closed that, and
  nothing here reopens it. Two triggers for the *model*: (a) if ancestry
  checks or ref contention show up in FLOW-4's soak test, add the
  generation number before adding a cache; (b) if SKIL-1's skill bundles
  need objects larger than the 8 MiB `objects` cap, that cap is a reviewed
  diff, not a silent bump — a governed store that accepts arbitrary blobs
  is a file server with an approval workflow.

## Compliance notes

All six tables are tenant-scoped, carry enabled + forced row-level
security keyed to the TEN-2 GUC, hold a tenant-isolation policy, and join
the adversarial RLS suite's completeness guard (ADR-0009's structural
rule). The five history tables hold no UPDATE or DELETE grant for
`synveda_app`; `vedaflow_refs` holds no DELETE. Author identity is stored
without a foreign key to `identities`, deliberately and for the reason
AUD-1 and MEM-1 give: a service identity's revocation deletes its identity
row and personal leaf (ADR-0018 decision 2), and recorded history must
neither block that deletion nor be destroyed by it. The tenant foreign key
stays — knowledge history does not outlive its tenant, and TEN-5 governs
its disposal explicitly, alongside the observe buffer and the sidecar
indexes.

`policy_snapshot_hash` is the mechanism behind ADR-0003's compliance
claim: an auditor can prove which pack, at which version, with which
configuration, governed any published asset at the moment it was created.
FLOW-1 records it; FLOW-3's approvals and FLOW-6's review surface are where
an auditor reads it. No audit action type is added here (decision 14) —
the object store has no product surface until FLOW-2, and every governed
VedaFlow action lands with the feature that introduces it.
