# ADR-0146: Govern Session source removal at a tenant admission boundary

- **Status**: Accepted
- **Date**: 2026-10-14
- **Feature(s)**: Source lineage and lifecycle invalidation, Explainable Knowledge context planning and scoped query, Session-based capture batches and reviewable candidates, Session compression assist
- **Deciders**: Owner-authorized implementation; GPT-6 Astra planning review

## Context

Private delivery proof and exact stored-policy replay identify the evidence a
receipt used. They do not serialize removal with a response assembled earlier,
prevent an unchanged event from returning after retention, or erase one source's
dependent revision while preserving a later independently supported revision.

## Decision

Use one transaction-scoped tenant admission advisory lock. Ordinary tenant
transactions acquire its shared mode before reading product state. Source
removal, authority changes and delivered-head invalidation acquire its exclusive
mode at transaction entry, before authorization or domain locks. Never upgrade a
shared lock. Resolve current stored Cedar and current anchors after waiting.
Provider work runs outside the transaction; its output re-enters admission and
revalidates exact inputs before persistence. A response admitted before removal
may finish afterwards; already delivered external bytes cannot be recalled.

Session-event withdrawal and forgetting are distinct typed VedaFlow Knowledge
effects, independent of item Forget. Their scope comes from the actual Session,
with independent SessionWrite and ProposalOpen decisions (and KnowledgeForget
for physical erasure) and the current
Knowledge approval matrix at internal sensitivity. A pending change removes
nothing. Effects bind the exact inspected public payload hash. Forget queues a
domain-specific source-cleanup operation; withdrawal retains inaccessible
content. Neither quarantine release nor retention lifts an applied decision.

Retain a content-free, non-cascading suppression identity over tenant, original
Session, client-event-ID digest and original raw source-payload hash, with the
public hash, governing scope, applied change and operation identities. Check the
alias before append duplicate return and insertion, including after event
retention. A new source id alone cannot defeat suppression. Explicit reimport
must create a separately governed materialization; it cannot clear the old
suppression or restore old receipts and pending approvals.

Eligibility follows every exact revision's normalized source links, including
descriptors not disclosed to the reader. This differs from revoking access to an
undisclosed descriptor. A flat multi-source revision does not prove independent
support and is withheld when any supporting source is removed. Clean later
revisions and unrelated candidates/items remain eligible. Per-event checks cover
Session diagnostics, quarantine, checkpoints and Capture; per-revision checks
cover history, sources, conflicts, exports, context and proposal diffs. New
derivatives retain source roots before publication. Manifest version 2 records
only bounded restart evidence actually delivered; coverage-only checkpoint
addresses are not copied-content roots. Immutable headers permit independent
Edit and item Forget without resolving a suppressed old body. New nonempty
ContextRuns require same-transaction dependency proof at commit; pre-proof
legacy copies are withheld and block source erasure as lineage_unproven.


Cleanup reuses durable operations, attempts and outbox with a closed
source_cleanup version and domain-specific SQL, never Skill joins. Each bounded
chunk verifies the immutable applied decision and a database-time live lease and
monotonic attempt before changing content, then commits progress and audit
together. Suppression remains effective during retries, holds and requester
departure. One-way definer primitives verify this approved operation and attempt;
caller GUCs alone grant no erasure authority. Preserve revision identities and
hashes while removing affected plaintext/search/embedding projections; preserve
clean revisions. Discover delivery and checkpoint copies before disposing their
source roots. Retained under hold is inaccessible, not physically erased. Cleanup never
claims a source-root deletion: original revision/event identities and hashes
remain, with one-way content clearing. Item Forget retains its separate checked
in-flight capability through its content-free tombstone and immutable link
headers, including after deleting the aggregate or its product link rows.

Additive head 0005 preserves all earlier migration bytes and fingerprints. New
catalogue fingerprints are measured independently and SQLx/public contracts are
generated from a fresh current database. Audit and telemetry retain ids, hashes,
closed states and bounded counts only. Independent exports, remote Git history,
external harness histories and backups keep their separate retention contracts.

## Options considered

1. **Tenant admission RW gate (chosen).** A bounded coarse fence covers absent
   aliases, all derivative families and immutable events without new row-lock
   privileges. Removal briefly contends with unrelated tenant admissions.
2. **Event locks plus family-specific authority locks.** Adds privilege and lock
   ordering complexity before a complete boundary exists.
3. **Asynchronous invalidation alone.** Leaves assembled responses and stale
   workers able to admit content after logical removal.

## Consequences

No network/provider waits may hold the gate. The gate is a consistency boundary,
not an authorization engine. Physical erasure is a separate inspectable operation,
and cannot be described as immediate removal of external copies. Deterministic
barrier tests must cover both ordering directions, stale provider/index results,
append after retention, crash/reclaim, exact policy after waits, every trace mode
and independent remaining evidence. The fourteen ordinary-role source-control
cases pass, together with current-head RLS, prior-head migration, genuine
supported pgvector profiles and fresh generated-contract checks. This qualifies
the Session source family; ImportArtifact roots require a separate extension.

## Compliance notes

Embedded Cedar, forced RLS, typed VedaFlow, static SQLx checked store SQL and
content-free audit remain mandatory. No public endpoint grants direct SQL access
or changes the meaning of existing item Forget.
