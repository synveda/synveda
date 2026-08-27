# CNSL-3: Audit temporal and disclosure views

## Problem and evidence

Advanced ▸ Audit already lists and filters governed audit events, verifies the tenant chain, and downloads a frozen content-free export. The public API also already implements `/v1/audit/knowledge` and `/v1/audit/disclosures`, including valid-time versus transaction-time and the distinction between recorded disclosure and authority. The console does not expose those two investigative views or their cursor history, so the remaining gap is drill-down, not a new generic audit explorer or historical Cedar replay.

## Scope

- Extend the existing Audit page with “what was this subject recorded as served?” and “who was recorded as served this Knowledge?” workflows backed only by the generated audit operations.
- Render `known`, `outside_time`, and `unresolved` separately with exact sequence, time, action, revision/content hashes, valid/transaction intervals, notes, and truncation state.
- Render recorded disclosures separately from authority events and make clear that neither list reconstructs everyone who could have seen content.
- Add cursor navigation, deep links from Session/context/Knowledge identifiers, loading/error/empty states, and accessible copy/export of the query parameters and content-free result.
- Require current independent KnowledgeRead before linking to or displaying any retained revision content; audit evidence itself stays content-free.

## Non-goals

- Rebuilding the delivered event filter, chain verifier, or frozen export.
- Historical Cedar re-execution, counterfactual authority claims, or treating AuditRead as KnowledgeRead.
- Reconstructing erased/unretained content from hashes, exposing denied counts/details, or displaying secrets/plaintext content in audit rows.
- A general SQL/report builder or cross-tenant search.

## Architecture seam

The console uses only generated `get_audit_knowledge`, `list_audit_disclosures`, and existing audit operations. The gateway remains the evidence interpreter and independently authorizes AuditRead and any Knowledge content fetch. UI state stores query parameters/cursors, not a parallel evidence model; route catalogue, OpenAPI, and generated client remain exact peers.

## Acceptance criteria

- An authorized operator can answer what a subject was recorded as served at a chosen valid/as-known instant and page older evidence without conflating unresolved or outside-time rows with `known`.
- An authorized operator can inspect a Knowledge item's recorded disclosures and authority events for a window, with truncation and the non-counterfactual limitation visible.
- Revoked KnowledgeRead leaves content and content links unavailable while permitted content-free audit evidence remains correctly rendered.
- Erased/hash-only evidence is explicitly unresolved and is never reconstructed or silently dropped from counts.
- Deep links and pagination preserve exact filters and cannot cross tenants or bypass generated API authorization/errors.

## Required tests

- Console reader-visible tests for known/outside-time/unresolved, disclosure/authority separation, empty, truncated, paged, and erased evidence.
- Generated-client operation/parameter tests and deep-link round trips.
- Gateway-backed AuditRead/KnowledgeRead allow/deny/revoke and cross-tenant browser flow.
- Accessibility, time-zone/instant, cursor, loading, cancellation, and non-secret error tests.
- Regression tests proving the existing chain verify/export and event filters remain unchanged.

## Rollout and rollback

Add the two views behind a console feature flag and enable them for audit administrators after usability/security review. Rollback hides the new views and deep links; the existing governed endpoints, event explorer, verifier, and export remain available and no evidence is mutated.

## Dependencies

No new backend architecture is required: ADR-0092 and the generated audit operations define the seam. The product owner must confirm whether the remaining temporal/disclosure drill-down justifies keeping CNSL-3 open and approve user-facing terminology, default time window, page bounds, and authorized content-link behaviour.
