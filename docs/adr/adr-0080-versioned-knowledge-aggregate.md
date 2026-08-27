# ADR-0080: Stable Knowledge heads, immutable content revisions and scoped provenance

- **Status**: Accepted
- **Date**: 2026-08-24
- **Feature(s)**: CPR-15
- **Deciders**: Autonomous continuation of the context-platform programme

## Context

ADR-0068 locks two facts that this feature must turn into a persistence
model: candidates and published Knowledge are different aggregates, and
Knowledge has stable identities with immutable revisions. The schema at the
start of CPR-15 still has the model those decisions replace: a mutable
`records` current row, a trigger-copied history row, provenance embedded in a
JSON bag and one embedding attached to the mutable record id. Extraction
writes those rows directly and retrieval treats them as the product's current
truth.

That shape cannot be renamed into the target. A candidate becoming accepted
would still be one state flag on one row; a merge would still have to copy or
rewrite provenance; an edit would still update the address an embedding and a
context trace cite; and a source visible at a private scope could be exposed by
serving the shared item that cites it. ADR-0068 decision 3 also forbids the
obvious transition mechanism: no dual write, translator or fallback read may
keep `records` and Knowledge in step.

The new model must support four different histories without conflating them:

- the stable identity and current lifecycle of a Knowledge item;
- the immutable content revisions that identity has pointed at;
- the evidence sources each revision derived from, which may have narrower
  visibility than the resulting item;
- explicit relations between stable items, including supersession.

It must also keep both clocks. Valid time is the author's statement about
when the knowledge holds. Transaction time is the database's statement about
which aggregate head it held as current. The future query surface needs both
without reconstructing one from audit prose.

## Decision

### 1. A stable head points at an immutable revision

`knowledge_items` is the aggregate head. Its UUIDv7 id never changes and it
carries the tenant, governing scope, optional project, optional owning
principal, Knowledge type, origin, lifecycle state, current revision and
creation/update metadata. A project association and governing scope are
deliberately separate: Alice may keep a project-related preference at her own
principal scope, while a convention about the same project lives at the
project scope.

`knowledge_revisions` is append-only. A row holds title, Markdown body,
summary, canonical tags, sensitivity, confidence, valid-time interval,
staleness threshold, verification metadata, extension metadata, content hash,
author and database-stamped transaction time. Editing creates a new id and
moves the head; no operation rewrites the old row.

The circular fact "this head points at a revision of this item" is a deferred
composite foreign key over `(tenant_id, current_revision_id, item_id)`. A head
cannot point at another tenant's revision or another item's revision, and a
new item plus its first revision can still be inserted atomically.

### 2. Aggregate-head history carries transaction intervals

`knowledge_items_history` is the transaction-time history of the head, and
`knowledge_item_versions` is the `security_invoker` union of current and
history. A database trigger, not application code, closes the old
`tx_from`/`tx_to` interval and starts the replacement interval whenever the
current revision, scope, project, owner, type or lifecycle changes. The same
trigger refuses identity, tenant, origin and creation-provenance changes.

This applies ADR-0006's current/history pair where it belongs: to mutable
aggregate state. Content revisions do not need a second history table because
they never change. Their `transaction_time` says when they entered the
database; the head-version interval says when each one was current. Archive
and restore can therefore change lifecycle without manufacturing a content
revision, while an as-known query still sees the transition.

### 3. Hash only a canonical, integer-valued content envelope

Revision confidence is stored as integer per-mille (`0..=1000`), not a float.
Tags are normalised to lower-case, sorted and unique before persistence.
Verification and extension metadata must be JSON objects and are recursively
key-sorted for hashing. The BLAKE3-256 content hash covers every semantic
revision field, including valid time, staleness and metadata, but excludes ids,
authors and transaction time. Two encodings of one content revision therefore
have one hash; two revisions may legitimately share it, so it is indexed but
not unique.

The hash is a content identity and audit input, not an authorisation token.
Revision ids remain the addresses context selections and feedback cite.

### 4. Provenance is a scoped source plus a many-to-many link

`knowledge_sources` stores one normalised source descriptor with its own
governing scope. Its closed source vocabulary is `session_event`, `manual`,
`document`, `repository`, `url`, `okf` and `system_derived`. A descriptor may
carry the real session-event id, a bounded logical locator, a source revision,
the source's content hash and extension metadata; it never copies a session
message or arbitrary source payload.

`knowledge_revision_sources` links sources to revisions many-to-many. A merge
can retain the exact source rows from all inputs without copying them, and an
unchanged source can support later revisions. A deferred constraint requires
at least one source at commit for every revision. "Provenance unknown" is not
a representable published revision.

A source is authorised independently of the item that cites it. Store reads
take the scopes the PDP has already allowed and return only source rows in
that set. A later public source route must decide per source; it may not infer
"item visible" means "all of its evidence visible". This is what prevents a
shared merge from disclosing Alice's private session id or document locator.

### 5. Relations are explicit, append-only claims

`knowledge_relations` stores the eight initial relation names:
`supports`, `duplicates`, `contradicts`, `supersedes`, `derived_from`,
`references`, `related_to` and `transitions_to`. Each relation names stable
source and target item ids and the immutable source revision that asserted the
edge. It is append-only and tenant-bound. A supersession therefore records an
edge rather than deleting the thing it replaced; later conflict resolution
can add a new governed relation without rewriting an old claim.

### 6. Retrieval starts from one current projection

`knowledge_current` is a `security_invoker` view joining each head to exactly
its current revision. It exposes the lifecycle and provenance-independent
content needed to filter and rank, and the revision table carries a stored
language-neutral `tsvector` with a GIN index. The view does not silently filter
by lifecycle, validity or policy: those are explicit predicates of the future
read service, and hiding one in a view would make as-of and transitional
queries disagree with ordinary reads.

Semantic embeddings remain keyed to immutable revision ids when the
Knowledge retrieval cutover lands. CPR-15 does not attach the old
`record_embeddings` row to a new item, does not copy its vector and does not
create an embedding over content no governed command can yet publish.

### 7. This package exposes persistence, not an application mutation path

The store module has instrumented, tenant-explicit primitives for creating a
source, creating an item and first revision, appending a revision, adding a
relation and reading current/history/source projections. They must run inside
the caller's transaction. There is no HTTP route, CLI command, adapter call,
Cedar action or audit action in CPR-15, so nothing outside the persistence
layer can create Knowledge yet. CPR-16 owns the command layer that creates a
VedaFlow change, evaluates the PDP and chains the resulting action before it
calls these primitives.

All six tenant tables have enabled and forced RLS, tenant-filtered foreign
keys and least-privilege grants in the migration that creates them. Both views
are `security_invoker`. Immutable tables refuse update, delete and truncate
for the owner as well as through the application grants.

### 8. There is no bridge to `records`

CPR-15 does not read, write, translate or synchronise `records`. The old plane
temporarily remains solely because the next two bounded packages must first
put governed mutations and public reads on Knowledge. The API/browser cutover
must then delete, rather than wrap, this checklist:

1. `records`, `records_history`, `records_versions`, `record_embeddings`,
   `record_signatures` and `record_supersessions` once no retained subsystem
   needs them;
2. `synveda_store::records` and record-shaped search, dedup, retention and
   promotion entry points replaced by Knowledge services;
3. the extraction commit path that turns model output directly into active
   records;
4. record DTOs, record fixtures, raw-record browser routes and primary
   user-facing `record`/`memory` terminology;
5. old audit query branches and graph links only where their subject is the
   replaced record aggregate, preserving genuine VedaFlow object history and
   hash-chain events as historical evidence.

No compatibility view, fallback read, dual write or record-to-Knowledge
translator may satisfy an item on that list.

## Options considered

1. **Stable head + immutable revision rows + bitemporal head history
   (chosen).** The current read is one pointer join; revision addresses never
   move; archive/restore history is preserved without fake content changes;
   and valid and transaction time remain distinct.
2. **Reuse ADR-0006's mutable current content row and archive it on update.**
   It preserves bytes but not immutable revision identity: a context trace
   would cite the stable item while its content changed underneath it. It is
   the record model under new table names and is rejected.
3. **Put lifecycle, scope and ownership on every content revision.** This
   makes every archive, restore or re-scope operation a content revision and
   changes the content hash when no content changed. It also leaves no stable
   aggregate head for revision preconditions. Rejected.
4. **Embed provenance JSON on the revision.** Easy to write and impossible to
   merge without copying, deduplicate without interpreting, or authorise
   source by source. It recreates `records.provenance` and is rejected.
5. **Make `knowledge_current` a materialised copy.** It would require another
   transactional write and another freshness invariant before any measured
   need exists. The pointer join is indexed and exact; revisit only if its
   measured retrieval plan breaches the context SLO.

## Consequences

- **Positive:** stable item and revision ids make history, supersession,
  feedback and context traces referential facts instead of JSON convention.
- **Positive:** provenance can be reused through merge and withheld at its own
  policy boundary without concealing the visible item's existence.
- **Positive:** current and as-known reads have explicit, RLS-safe projections
  and do not depend on audit-log reconstruction.
- **Negative / accepted:** six tables and two views are more schema than a
  mutable row. Each table represents a different mutability or authority
  boundary, and the RLS completeness test makes their cost visible.
- **Negative / accepted:** for two package commits, old record reads coexist
  with an unused new Knowledge store. They are not synchronised; no caller can
  observe a combined answer; and the deletion checklist and queue make the
  temporary boundary explicit.
- **Reversal trigger:** if measured current-projection plans breach the
  retrieval budget at production-scale history, add a transactionally
  maintained retrieval projection keyed by immutable revision id. Do not make
  revisions mutable or restore a record bridge.

## Compliance notes

- **Tenancy:** every table carries `tenant_id`; every cross-table reference is
  tenant-qualified; all six are enabled + forced RLS and added to the dynamic
  completeness gate; both views execute as the caller.
- **PDP:** CPR-15 adds no application read or mutation. Source reads accept
  only PDP-authorised scope ids, and the subsequent application package must
  decide before invoking them.
- **VedaFlow:** no Knowledge can be published through a public path until the
  next package wraps these primitives in one VedaFlow command lifecycle.
- **Audit:** no new action type exists in this persistence-only package. The
  command package owns content-free mutation events and their embedded PDP
  decisions.
- **Secrets:** source rows contain identifiers, hashes and bounded locators,
  never copied message bodies, source payloads, credentials or secret values.
