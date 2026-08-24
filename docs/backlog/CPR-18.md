---
title: "CPR-18: Session-based capture batches and reviewable candidates"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-18: Session-based capture batches and reviewable candidates

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** XL

## Description

Replace the final internal session-event-to-record extraction writer with a
candidate-only pipeline over the session ledger. One durable `CaptureBatch`
owns a bounded extraction attempt; its `CaptureCandidate` rows retain proposed
Knowledge content, exact source events, independently visible duplicate and
conflict matches, and the decision/result state required by New Learnings.

Extraction never publishes. Accept, edit, merge and replace enter CPR-16's
typed Knowledge command service and its existing VedaFlow review engine.
Dismiss changes only candidate state. Repeated requests for one session replay
the same batch rather than re-running a model or creating duplicate rows.

## Acceptance criteria

1. `CaptureBatch` and `CaptureCandidate` are tenant-bound, forced-RLS
   aggregates. Candidate states are `pending`, `accepted`,
   `edited_and_accepted`, `merged`, `replaced`, `dismissed` and `failed`.
2. Every candidate records proposed Knowledge type, title, Markdown body,
   summary, proposed scope, confidence, exact source session-event ids,
   visible duplicate/conflict matches, status and decision/result metadata.
3. Explicit and session-end extraction select potentially durable content,
   classify it, validate bounded model output, compare against visible current
   Knowledge and create candidates only. They write no Knowledge or records.
4. Source event links are constrained to the same tenant and session as the
   batch. Fictional, cross-session and cross-tenant source ids are refused;
   policy-denied match ids and counts are never persisted or disclosed.
5. `POST /v1/sessions/{session_id}/capture-batches`, capture batch/candidate
   collections and detail, candidate accept/merge/replace/dismiss and batch
   accept are mounted in generated OpenAPI. Growing collections use opaque
   cursors and the common error envelope.
6. Accept and edit-and-accept call Knowledge create; merge calls Knowledge
   merge; replace calls governed Knowledge supersession. The resulting
   VedaFlow change, Knowledge item and revision ids and pending-review outcome
   are retained on the candidate without creating a second review workflow.
7. Extraction and every decision are retry-idempotent. A batch accept makes
   progress per candidate and can resume safely after a pending review or
   failure; no applied change is duplicated.
8. The old record extraction writer and direct-active extraction tests,
   fixtures and documentation are deleted. Unreviewed candidates cannot enter
   ordinary Knowledge reads or current context.
9. PDP decisions precede session, source, scope and match disclosure. Important
   transitions are traced, metered and hash-chain audited without candidate or
   source content.
10. Focused store/gateway/OpenAPI/RLS tests, a runnable demo, `make ci` and
    `make db-test` pass.

## Decision

[ADR-0083](../adr/adr-0083-session-capture-candidates.md) locks the
candidate-only extraction boundary and use of the existing VedaFlow Knowledge
command seam before runtime implementation.

## Completion evidence

Delivered from `2d845b0f8a43d66f802286df922b820bf1bf25cf` on 2026-08-24.

Migration `0050_capture_candidates` adds six tenant-bound,
enabled-and-forced-RLS tables for leased batches, frozen source events,
candidates, independently visible matches, exact source links and durable
decision intent/results. Composite constraints bind every source to the
batch's tenant and session; append-only/transition triggers protect evidence,
and authorised Knowledge erasure scrubs candidate and request plaintext while
retaining only ids and hashes.

The database-leased capture worker re-authorises the session principal before
model work, validates and rescans every bounded extractor result, independently
decides every Knowledge neighbour, and persists candidates only. Terminal
session close and the explicit public route freeze the same canonical event
snapshot idempotently. The old PGMQ `session_events` queue and record/embed/
dedup/link/channel extraction writer are deleted; no bridge or dual write
replaces them.

The generated OpenAPI contract has 62 operations, including the nine capture
operation groups. Accept/edit, merge and replace use CPR-16's typed Knowledge
commands and ordinary VedaFlow proposal/application path; dismiss creates no
Knowledge. Durable decision intent plus Knowledge idempotency converges retries
and concurrent callers, while whole-batch acceptance resumes child by child.
Candidate plaintext requires both exact session read and Knowledge-read
authority at its proposed destination, and persisted matches are re-authorised
again before disclosure.

Acceptance evidence:

- `crates/synveda-gateway/tests/capture_api.rs`: 3/3, covering candidate-only
  extraction, same-session provenance, every decision kind, strict-profile
  pending review, retry/concurrency invariants, erasure, match revocation and
  cross-tenant 404s;
- deterministic Claude lifecycle 2/2 with the live-client case separately
  ignored, Knowledge lifecycle 4/4 and session redaction 2/2;
- extractor/types/audit focused suites: 64/64, 213/213 and 20/20;
- OpenAPI 5/5, console 151/151, Claude adapter 96/96 and RLS 84/84;
- `demos/cpr-18-session-capture.sh` PASS: 8 reviewable candidates, 8 governed
  changes and zero old records or extraction queues;
- `make db-test` PASS against fresh migration 0050 and `make ci` PASS.

The deliberately temporary record-backed context composer remains read-only
and receives no accepted-Knowledge translation. The explainable
Knowledge-backed context-planning package owns that final retrieval cutover;
New Learnings owns the candidate console next.
