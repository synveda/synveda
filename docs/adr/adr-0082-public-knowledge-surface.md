# ADR-0082: The public Knowledge surface reads immutable current revisions

- **Status**: Accepted
- **Date**: 2026-08-24
- **Feature(s)**: CPR-17
- **Deciders**: Autonomous continuation of the context-platform programme

## Context

CPR-15 created stable Knowledge items, immutable content revisions, normalised
sources and explicit relations. CPR-16 made every mutation a typed VedaFlow
change. Neither package exposed a public Knowledge read or write route, and the
console still renders a placeholder. The remaining user-facing record seams
therefore describe an aggregate no supported application path should create.

The public surface has to satisfy four constraints at once. A listing must
decide each item under that item's effective pack rather than applying a root
verdict to every row. Source visibility is narrower than item visibility.
Search must use current Knowledge revisions without reusing the record index
or making deterministic hash vectors sound semantic. Finally, cursor progress
must remain correct when the PDP removes every candidate in a scanned page.

CPR-18 will replace the context composer's record internals and add the scoped
recall/query lens. This package must not bring that work forward by silently
making the browser's search endpoint a global recall sweep, and it must not
delay the public record cutover by keeping aliases or translators.

## Decision

### 1. One public noun and one mutation seam

The public application exposes `/v1/knowledge` and item-scoped history,
source, usage and lifecycle routes. Creation, edit, verification,
supersession, merge, archive, restore and forget construct the existing typed
`KnowledgeCommand` and call CPR-16's command service. They never call a
Knowledge store mutator directly. Creation requires `Idempotency-Key`; every
command against an existing head requires the exact revision the caller
inspected.

There is no record DTO, record-to-Knowledge translation, fallback read, dual
write or alias. The record classification route, CLI command and public
proposal inputs that name record ids are deleted in this package. Record
storage and the record-backed context composer remain internal only until
CPR-18, where their last consumers are re-cut and deleted.

### 2. Current state is the default, history is explicit

The collection reads the `knowledge_current` security-invoker projection. With
no lifecycle filter it returns only `active` heads whose current revision is
valid now; stale-by-date remains visible but is labelled stale so a reader can
ask for it explicitly. Archived, superseded, erasure-pending and erased state
never enters the default result. History is available only through the item
history route and always names immutable revision ids.

The usage route has the final cursor-paginated envelope from its first release.
CPR-17 has no Knowledge-selecting context consumer, so a newly authored item
truthfully has no usage entries. CPR-18 populates that same contract from
`ContextSelection`; CPR-17 does not create a temporary usage table or relabel
mutation history as agent use.

### 3. Search has two honest legs

Lexical search uses the stored weighted `tsvector` on immutable current
revisions. Semantic search uses a new forced-RLS
`knowledge_revision_embeddings` sidecar keyed by revision and model, with the
same reviewed 16- and 1024-dimension pgvector index shapes as the existing
retrieval infrastructure. A restart-safe background sweep embeds immutable
revisions outside a database transaction and inserts the vectors
idempotently.

The configured TEI model enables the semantic leg. The zero-configuration
deterministic hash embedder may maintain reproducible test/index rows but its
geometry is never queried or reported as semantic; requests run lexical-only
and carry that degradation explicitly. Search fuses bounded lexical and dense
candidates with reciprocal-rank fusion, then hydrates current database truth.
It never reads or writes `records`.

### 4. The PDP filters objects, sources and edges before disclosure

Ownership is resolved under tenant RLS before any decision, so a made-up id
and another tenant's real id are the same 404. Each collection candidate is
decided with `KnowledgeRead`, its own scope chain, exact `KnowledgeItem`
entity and current revision sensitivity. There is no tenant-root fast path.

Reading an item does not grant its sources. The source route independently
decides every source scope and silently omits denied descriptors. A relation is
shown only after both endpoint items are independently readable; a denied
endpoint leaks no edge, id, title or count. Read decisions are hash-chain
audited with ids and counts, never Knowledge content or source locators.

### 5. Cursors advance over considered candidates

Ordinary listing uses `(updated_at, item_id)` keyset order. Search uses the
fused score, then the same stable tie-breakers. The opaque cursor binds the
normalised query and filters and points at the last candidate the page
considered, whether the PDP admitted it or not. A page may therefore be empty
and still carry `next_cursor`; it can never loop on denied rows or stop while
readable rows remain below them.

### 6. The console consumes only the generated contract

Every Knowledge operation is declared by the handler-derived OpenAPI 3.1
document. The Knowledge Browser uses the generated operation/type table for
search, detail, history, provenance and all mutations. It adds one linkable
item route and removes the Knowledge placeholder and record-oriented review
fixtures. No second hand-written Knowledge contract is added to
`console/src/api.mts`.

## Options considered

1. **Current revisions plus a revision embedding sidecar (chosen).** Preserves
   immutable addresses, reuses pgvector operations and isolates the browser
   from the record composer still awaiting CPR-18.
2. **Reuse the record search index.** Requires a translation or dual index key
   and would make record identity part of the new public model. Rejected.
3. **Embed synchronously inside every Knowledge command.** Gives a network
   dependency authority over governance commits and makes an unavailable
   model block an archive or verification. Rejected; indexing converges
   independently and the read reports degradation.
4. **One decision at the requested scope, then expose every result.** Repeats
   the listing defect CPR-9 fixed and cannot honour a deeper forbid. Rejected.
5. **Return all source descriptors with a visible item.** Turns shared
   conclusions into private-conversation and document existence oracles.
   Rejected.

## Consequences

- Knowledge becomes the only public memory noun while context composition
  remains on a narrowly enumerated internal record seam until CPR-18.
- A freshly written item is immediately lexically searchable. Semantic search
  converges asynchronously and reports lexical-only operation until the real
  model and vector are available.
- Search performs more PDP work than a scope-only filter. That cost is the
  enforcement boundary; future optimisation may batch reads but may not cache
  a root verdict as an item verdict.
- The public usage surface may be empty throughout CPR-17 without inventing
  evidence. Its first producer is the explainable selection aggregate in
  CPR-18.
- Generated OpenAPI and console types grow together; route/schema drift fails
  the existing contract gates.

## Compliance notes

- **Tenancy/RLS:** the embedding sidecar is tenant-bound, enabled and forced
  RLS, tenant-qualified and deleted by the revision foreign key during an
  authorised forget.
- **PDP:** every item, source scope and relation endpoint is decided before it
  contributes response data or counts.
- **VedaFlow:** HTTP mutation handlers construct commands only; CPR-16 remains
  the sole effect boundary.
- **Audit:** reads chain ids, filters, counts, mode and decision context, never
  titles, bodies, queries, locators or source payloads.
- **Semantic honesty:** deterministic hashes are not described or measured as
  semantic retrieval.
