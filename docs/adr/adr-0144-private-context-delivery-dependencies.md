# ADR-0144: Retain private delivery proof independently of diagnostic traces

- **Status**: Accepted
- **Date**: 2026-10-14
- **Feature(s)**: MEM-8, CPR-20, CTX-6
- **Deciders**: Owner-authorized implementation; GPT-6 Astra planning review

## Context

ADR-0141 closes saved delivery without sufficient current source proof. Optional
trace retention discards exact addresses, while authored packs and Skill
advertisements have no complete replay projection. The planner already holds
their immutable addresses. Erasure finds retained delivery copies through trace
selections, which cannot cover hashes-only or disabled traces.

The Session event ledger is unsuitable for an internal index: it exposes payload
diagnostics, rejects append after closure, and its raw sequence continuity defines
checkpoint coverage. Event and run timestamps use transaction-start time and
cannot authenticate an internal producer. Reusing it would weaken those contracts.

## Decision

Add one private immutable tenant-bound dependency manifest per new ContextRun,
inserted with its run, audit and idempotency receipt. Version 1 is a closed typed
shape containing only addresses, digests and closed enums: exact selected
Knowledge/candidate identities, disclosed provenance, graph paths, checkpoint/tail
references, authored chunks/document hashes, resolved channels/pins and immutable
Skill bindings/versions/bundle/object hashes. Bound the serialized manifest to
512 KiB, selections to 1,024, graph steps to 2,048, channels to 64, authored
chunks to 1,024, Skill advertisements to 32 and tail events to 16. Limit exhaustion
refuses admission explicitly; it cannot disable proof. No dependency target FK
may null or delete evidence when a source is erased.

The table has forced tenant RLS and insert/select application grants, no public
endpoint or trace projection, and the lifetime of its owning ContextRun. Its
hash binds canonical manifest bytes, which bind the original block/configuration.
Never backfill old receipts from optional traces or infer an authenticated
producer from caller-supplied events. Missing/unknown/malformed proof refuses
nonempty saved delivery with ADR-0141's uniform 409; safe empty receipts remain
replayable. Current source admission precedes replay even in hashes-only/disabled
modes. Diagnostic retention continues to narrow public detail independently.

Independently authorize exact revisions and disclosed provenance. Private source
descriptors absent from the offered contribution do not revoke an independently
approved shared conclusion. Recheck current heads, freshness and channel/binding
identity before replay. Do not silently substitute newer bytes under the old key.

Extend governed item erasure with a narrowly checked trigger to clear ContextRun
copies addressed by these manifests, including graph dependencies. It runs only
inside the existing approved erasure primitive and verified operation/tombstone
boundary; it grants no ordinary DELETE/UPDATE authority or caller-setting bypass.
Content-free dependency identities/digests may remain as erasure evidence.

This slice does not establish source-level suppression, full concurrent authority
revocation ordering or resumable physical source cleanup. Those remain MEM-8 work
before persistent outcome learning. Already delivered external bytes remain
outside the service's admission boundary.

## Options considered

1. **Private per-run manifest (chosen).** Reuses existing ContextRun identity and
   PostgreSQL authority without a second graph or optional diagnostic dependence.
2. **Internal Session events.** Changes closure, diagnostics and checkpoint
   sequencing, with inadequate timestamp-based producer proof.
3. **Recover lineage from rendered prose or optional traces.** Cannot prove all
   source families or safely recover discarded identities.

## Consequences

Saved replay becomes possible under every retention mode when exact proof remains
eligible. The extra bounded row costs storage and authorization work. New additive
head `0004` requires measured ACL/routine/trigger/RLS fingerprints, exact prior-head
proofs and genuine supported pgvector profiles; published migrations are unchanged.

## Compliance notes

All authoritative SQL remains static and SQLx checked in synveda-store. Cedar,
forced RLS, VedaFlow erasure and content-free audit remain mandatory. Dependency
metadata is never exposed through a hashes-only receipt, metrics or logs.
