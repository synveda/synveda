# ADR-0081: Knowledge changes are VedaFlow proposals with typed effects

- **Status**: Accepted
- **Date**: 2026-08-24
- **Feature(s)**: CPR-16
- **Deciders**: Autonomous continuation of the context-platform programme

## Context

CPR-15 created a persistence seam and deliberately made it unreachable from
the application. The next seam must support create, edit, verify, supersede,
merge, archive, restore and forget without creating a direct write API or a
second review engine. ADR-0068 is explicit: a permissive personal policy may
auto-apply a VedaFlow change, but it may never skip the change, the PDP,
immutable versioning or audit.

The existing VedaFlow proposal is already the repository's one review
workflow: an immutable content-addressed commit, a mutable one-shot lifecycle,
append-only approvals and a matrix resolved from asset, sensitivity, scope and
effective policy pack. Its original effects publish a channel, grant a lapse
or classify a record. A Knowledge mutation changes an aggregate rather than a
channel, but that is another effect of the same governed change, not grounds
for another proposal table.

Two constraints make simply storing a command object insufficient. First,
approvals must bind the exact proposed bytes even when a change waits for
review. Second, `forget` must remove plaintext everywhere, including proposed
or applied command payloads, without destroying the content-free VedaFlow and
audit evidence that proves what happened.

## Decision

### 1. Extend the one VedaFlow vocabulary

Add `knowledge` to `AssetKind` and `apply` to `ProposalEffect`. A Knowledge
command stores one content-free manifest as a VedaFlow object and opens one
ordinary `vedaflow_proposals` row. That proposal id is the command's `change
id`; its existing approval rows, live requirement resolution, rejection and
one-shot close semantics remain the only review engine.

The manifest contains the command kind, affected stable ids, expected revision
ids and a BLAKE3 hash of the canonical payload. It contains no title, body,
summary, locator, source payload or credential. Approvals therefore bind the
payload hash without turning immutable VedaFlow object storage into a place
`forget` cannot erase.

Both review rendering and effect execution independently canonicalise and
re-hash the typed payload and compare the command, target ids and digest with
this manifest. Storage corruption or privileged tampering therefore fails
before a Knowledge write, even though the database also makes the rows
immutable in the ordinary application path.

`knowledge_changes` is a typed effect projection keyed one-to-one to the
proposal. It holds the canonical payload while review is pending and the
resulting item/revision/operation ids after application. It is not a second
workflow: state and approvals are read from the proposal, and a database
constraint makes a projection without its VedaFlow proposal impossible.

### 2. A command is authorised, proposed, then conditionally applied

Every command first resolves all named items under RLS, returning the same 404
for fiction and another tenant. It then decides `KnowledgeWrite` at every
affected object/scope; forget additionally decides the separable
`KnowledgeForget` action. Opening the VedaFlow change also requires
`ProposalOpen` at the target scope. Multi-item merge and supersession decide
each input and the output scope; authority over one item never smuggles
authority over another.

After the proposal and projection exist, the effective approval matrix is
resolved for `AssetKind::Knowledge`, maximum sensitivity and target scope
shape. No outstanding requirement means the effect is applied in the same
transaction and the proposal closes `applied`. Otherwise it remains open and
the command returns `pending_review`. A rejection closes the same proposal and
returns `rejected`; there is no knowledge-specific inbox.

`standard` and `open-collaboration` retain their existing local auto-apply
semantics; `regulated-strict` requires review at every scope. Restricted
content still inherits the invariant approval floor. These are policy answers,
not edition branches.

### 3. Commands have exact aggregate semantics

- create mints one stable item and first immutable revision;
- edit and verify append a revision and require the expected current revision;
- supersede creates the replacement, writes an explicit `supersedes` relation
  and marks the replaced item superseded;
- merge creates one result, carries every source link from every input, writes
  `derived_from` relations and marks the merged inputs superseded;
- archive and restore move only lifecycle state and require a revision
  precondition;
- forget first marks `erasure_pending`, then runs a durable erasure operation.

An apply revalidates existence, lifecycle and revision preconditions against
current state. Approval binds old bytes, not a promise that the world will
stand still; drift rejects the change rather than applying reviewed content to
a different head.

### 4. Long work uses one reusable durable operation ledger

`durable_operations` is a tenant-bound, forced-RLS job ledger with a typed
kind, input hash, pending/running/succeeded/failed/blocked lifecycle,
attempt/lease fields and content-free result metadata. Forget is its first
consumer. Claiming is compare-and-swap and retry-safe; completion is terminal.
Later import, re-index, re-encryption and export work reuse this ledger rather
than inventing noun-specific job tables.

The erasure policy seam returns allow or hold before destructive work. A hold
leaves the item `erasure_pending`, records the operation `blocked`, and retains
content. An allowed operation removes every revision's plaintext, lexical and
future embedding state, removes unshared owned source descriptors and all
affected pending payloads, and invalidates retrieval state. It then removes
the live aggregate and writes `knowledge_erasure_tombstones`: ids, timestamps,
revision ids and hashes, change/operation ids and no content.

Database deletion guards open only inside the erasure transaction through a
transaction-local setting that ordinary application code cannot set through
any public path. The command worker is the sole production caller.

### 5. Audit records decisions and transitions, never content

Proposal, applied, rejected, erasure queued/blocked/completed transitions are
hash-chained in the same transaction as their state. Payloads carry ids,
hashes, counts, command kind, policy snapshot and PDP decision context; no
Knowledge body, source locator or secret is copied to the chain, logs or
metrics.

## Options considered

1. **Reuse VedaFlow proposals with a typed Knowledge effect (chosen).** One
   review model and one approval matrix; the effect differs, not governance.
2. **Create `knowledge_proposals`.** Easier locally and immediately violates
   the one-review-engine invariant. Rejected.
3. **Publish Knowledge through `memory/published` channels.** It would make the
   channel and the aggregate head two answers to what is current, and keep the
   record model under a new noun. Rejected.
4. **Store the full command in immutable VedaFlow objects.** Review is simple,
   but authorised erasure cannot remove the plaintext. Rejected; the object
   binds a content-free manifest and payload hash instead.
5. **Hard-delete synchronously inside the request.** It gives retries no
   durable address and cannot grow to embedding/index/source cleanup. Rejected.

## Consequences

- Every successful Knowledge mutation has a VedaFlow proposal/change id even
  when policy auto-applies it immediately.
- Existing advanced proposal review can see pending Knowledge changes; the
  same public proposal surface renders the exact typed payload and runs its
  `apply` effect; the later unified-review package extends presentation rather
  than workflow.
- There is one temporary coexistence in vocabulary: old `memory` effects serve
  the controlled read/composition plane while Knowledge is the only new
  aggregate effect. There is no translation or dual write between them.
- The gateway no longer starts the old extraction, promotion or retention
  loops. VedaFlow classification and context-pack chunk materialisation remain
  only for the controlled old read/composition interval; CPR-16's feature
  record names their CPR-17/18 deletion points.
- Erasing an item first rejects every other open change that names it, then
  clears their payloads. The review queue therefore never contains an open,
  uninspectable effect whose target has disappeared.
- Forget deliberately sacrifices erased immutable content while preserving
  immutable identifiers, hashes, governance and audit proof; that exception is
  narrow, explicit and adversarially tested.

## Compliance notes

- **Tenancy/RLS:** every new table is tenant-bound, enabled and forced RLS and
  enters the completeness gate.
- **PDP:** item ownership is checked before the decision; every input/output
  scope is decided, and forget has a distinct authority.
- **VedaFlow:** no application path calls a Knowledge store mutation without a
  proposal and content-addressed manifest in the same transaction.
- **Audit:** every important transition is chained atomically and carries no
  plaintext.
- **Secrets/erasure:** command manifests are content-free; erasure removes all
  retained Knowledge and command plaintext while keeping only hashes and ids.
