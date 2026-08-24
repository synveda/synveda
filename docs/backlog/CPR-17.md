---
title: "CPR-17: Public Knowledge API, search and browser"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-17: Public Knowledge API, search and browser

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** XL

## Description

Expose the versioned Knowledge aggregate through one generated public API and
replace the console placeholder with a Knowledge Browser. Reads use current
immutable revisions by default, support filtered lexical and honest semantic
search, and decide every candidate, source and relationship endpoint through
the PDP. Mutations map onto CPR-16's VedaFlow command service and never call a
store writer directly.

Complete the public noun cutover: delete record classification and public
proposal inputs that name record ids, plus record-oriented browser DTOs,
fixtures and documentation. Preserve only the internal record-backed context
composition seam that CPR-18 explicitly owns; add no translation, alias,
fallback or dual read.

## Acceptance criteria

1. The thirteen Knowledge collection, item, history, sources, usage and
   lifecycle operations are mounted and declared in generated OpenAPI with a
   common error envelope.
2. Creation requires `Idempotency-Key`; retries replay one VedaFlow change and
   changed requests conflict. Every existing-item mutation requires the exact
   current revision and returns CPR-16's change/outcome/result envelope.
3. Collection reads are cursor-paginated and filter by workspace, project,
   scope subtree, owner, type, origin, lifecycle, tag, source, update range and
   staleness. Default results are current, active and policy-visible only.
4. Lexical search uses the current revision's stored search document. A real
   configured semantic model searches immutable revision embeddings and fuses
   the two legs; deterministic hashing is labelled lexical-only, never
   semantic.
5. Every candidate is decided as its exact Knowledge item and sensitivity.
   Denied sources and relation endpoints leak no descriptor, edge, id or
   count; another tenant's real id is indistinguishable from fiction.
6. History exposes immutable revision ids and content hashes. Usage returns a
   truthful cursor envelope and invents no use before CPR-18's
   `ContextSelection` producer exists.
7. The Knowledge Browser uses generated operations for search/filters,
   current content, history, provenance, verification, relationships and
   create/edit/verify/merge/supersede/archive/restore/forget flows.
8. The old raw-record browser/review fixtures, proposal classification route
   and CLI command, eval client call and public `record_ids` proposal input are
   deleted. Old runtime routes remain 404 and no record translation exists.
9. The embedding sidecar is forced-RLS, immutable-revision keyed and removed
   by authorised erasure. Search/indexing carry tracing and metrics; reads and
   mutations have content-free audit evidence.
10. Focused store/gateway/console/OpenAPI/RLS tests, a runnable demo,
    `make ci` and `make db-test` pass.

## Decision

[ADR-0082](../adr/adr-0082-public-knowledge-surface.md) — read immutable
current Knowledge directly, fuse lexical search with an honest revision
embedding sidecar, decide each disclosed object, and keep the CPR-18 context
cutover boundary explicit.

## Completion evidence

Delivered from `f2a7c5c` on 2026-08-24.

- Migration `0049_knowledge_search` adds the immutable-revision embedding
  sidecar with enabled and forced RLS. The generated OpenAPI contract now
  contains **53 operations**, including all thirteen Knowledge operation
  groups, and the generated TypeScript operation table is its only console
  contract.
- `crates/synveda-gateway/tests/knowledge_lifecycle.rs` proves public create
  replay/conflict, current/detail/history/source/usage reads, per-source
  privacy, filters, lexical search, semantic degradation honesty, immutable
  edit/verify, merge, supersede, archive/restore/forget, another tenant's real
  id as 404, sidecar erasure and all removed raw-record inputs. OpenAPI is
  **5/5**, console is **151/151**, the dynamic RLS inventory is **84/84**, and
  the full database suite passes.
- The console now has `/console/knowledge` and
  `/console/knowledge/{item_id}` with current content, revision history,
  independently visible provenance, relationships and all governed lifecycle
  actions. This also subsumes and closes **CNSL-4**; there is no second Memory
  browser.
- Deleted: the proposal classification route and CLI/eval callers; generic
  proposal `record_ids` and `effect`; record publication through channels;
  memory channel history/rollback/pin aliases; raw-record review fixtures;
  and seven record-oriented public integration suites. The internal
  session-event extraction/context record projection is the one remaining
  controlled seam and belongs to CPR-18; it is neither public nor a Knowledge
  dual write.
- `demos/cpr-17-knowledge-browser.sh` passes against a disposable database and
  reports one Knowledge item and zero old records. Focused tests, `make ci`
  and `make db-test` pass.
