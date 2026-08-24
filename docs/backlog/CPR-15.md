---
title: "CPR-15: Versioned Knowledge aggregate and provenance"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-15: Versioned Knowledge aggregate and provenance

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** XL

## Description

Add the persistence model ADR-0068 locked before any public Knowledge
mutation or browser exists: stable `KnowledgeItem` aggregates, immutable
`KnowledgeRevision` content, independently governed `KnowledgeSource`
provenance and explicit `KnowledgeRelation` edges.

The aggregate supports knowledge types `fact`, `decision`, `preference`,
`procedure`, `entity`, `episode`, `convention`, `warning` and `reference`;
origins `observed`, `asserted`, `authored` and `imported`; lifecycle states
`active`, `stale`, `superseded`, `archived`, `erasure_pending` and `erased`;
and the eight initial relation types. Revision content includes title,
Markdown body, summary, canonical tags, sensitivity, integer confidence,
valid time, transaction time, staleness, verification data, a canonical
BLAKE3 hash and extension metadata.

Normalised source rows support session events, manual authorship, documents,
repositories, URLs, OKF and system derivation. A revision has at least one
source, sources can be retained across a merge, and source visibility is
filtered at its own governed scope rather than inherited from the item.

The current retrieval projection joins the stable head to its current
revision, while a bitemporal head-history projection preserves scope,
lifecycle and pointer changes. All tenant tables are forced-RLS and immutable
surfaces are protected by grants and triggers.

This feature creates no public mutation path. It neither reads nor writes the
old record model. `records` remains temporarily for the controlled lifecycle
and API cutover only; ADR-0080 carries the explicit deletion checklist and
forbids a bridge, fallback or dual write.

## Acceptance criteria

1. A Knowledge item has a stable UUIDv7 id and a current immutable revision;
   adding a revision changes the head without changing or deleting any prior
   revision.
2. Current projection, valid-time fields and transaction-time head history
   answer current and as-known state correctly after a revision change and a
   lifecycle change.
3. Every revision carries every required content field, a canonical 64-hex
   BLAKE3 content hash and at least one normalised source; malformed ranges,
   metadata, hashes, tags and source shapes are refused by code and schema.
4. All seven source kinds are representable. A session-event source is bound
   to a real event in the same tenant, and an allowed-source-scope read omits
   a linked source at a scope the caller was not authorised to inspect.
5. All eight relation kinds round-trip and a relation cannot cross tenants or
   claim it was asserted by another item's revision.
6. Every new tenant table is enabled + forced RLS, present in the completeness
   guard and adversarially invisible under another tenant's GUC. Both new
   views are `security_invoker`.
7. Revision, source-link and relation rows reject update/delete/truncate; the
   application role has only the grants each lifecycle needs.
8. Creating Knowledge changes no row in `records`, `records_history`,
   `record_embeddings`, `record_signatures` or `record_supersessions`; no
   production code reads or writes both models.
9. Focused type/store/RLS tests, `make ci` and `make db-test` pass; new store
   paths carry tracing spans and mutation counters; the durable queue and
   implementation record name the exact schema and the controlled deletion
   boundary.

## Decision

[ADR-0080](../adr/adr-0080-versioned-knowledge-aggregate.md) — stable heads,
immutable revisions, bitemporal aggregate-head history, independently scoped
normalised sources and no bridge to records.

## Completion evidence

Delivered 2026-08-24 in migration `0047_knowledge`. The five type tests and
five Postgres acceptance tests cover immutable revisions, current/as-known
state, every source and relation shape, independently filtered provenance,
session-event scope confusion, lifecycle history and cross-tenant RLS. The
dynamic RLS inventory covers all six tables and both security-invoker views.
`demos/cpr-15-knowledge-aggregate.sh` creates and removes an isolated database
and runs the acceptance evidence without mutating an existing development
database. `make ci` and `make db-test` pass.

The package added no public action, so it added no Cedar or audit action. Its
instrumented store primitives remain unreachable from HTTP until CPR-16 puts
the PDP, VedaFlow change and hash-chained audit event in front of them.
