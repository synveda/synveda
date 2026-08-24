---
title: "CPR-19: New Learnings lightweight review workflow"
labels:
  - epic:CPR
  - phase:5
size: L
---

# CPR-19: New Learnings lightweight review workflow

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** L

## Description

Replace the New Learnings placeholder with the primary personal/team review
experience over CPR-18's capture batches and candidates. Candidates remain on
their own side of the publication boundary. Accept, edit, merge and replace
call the generated public capture commands and therefore CPR-16's one typed
VedaFlow Knowledge seam; dismiss publishes nothing. A stricter policy's
pending change moves to Advanced Reviews without turning that enterprise
review engine into a second candidate inbox.

## Acceptance criteria

1. Candidates are grouped by durable capture batch, with project, exact
   session and decision-state filters, honest batch progress and cursor-based
   continuation for both growing collections.
2. Each card names proposed type, content, confidence, sensitivity and private,
   project or workspace placement distinctly. It shows duplicate, conflict and
   possible-supersession indications without inventing a match the API omitted.
3. Exact source event ids resolve through the public session timeline into a
   source-conversation preview. Raw payload is fetched only on demand and only
   when the exact run's capability forecast offers `session.diagnostics`.
4. Every existing-item comparison is freshly read through
   `GET /v1/knowledge/{id}`. A revoked/denied match therefore renders the
   gateway's refusal rather than stale content retained in frontend state.
5. Accept, edit-and-accept, merge, replace, change-scope-and-accept and dismiss
   are present. They use only generated public operations, require an
   idempotency key and carry exact Knowledge revision preconditions for merge
   and replacement.
6. The publication picker offers only the principal, relevant project and
   relevant workspace anchors whose `/v1/me` forecast says
   `knowledge.write: true`. This is an offer only; the gateway repeats the real
   PDP decision and a stale forecast cannot grant authority.
7. Replacement is described and executed as governed supersession; history is
   never deleted. Dismissal explicitly publishes no Knowledge.
8. Applied outcomes link to resulting Knowledge. Pending-review outcomes link
   to Advanced Reviews and state that the candidate is not active Knowledge;
   rejection and failure do not claim state changed.
9. Advanced Reviews remains the one comprehensive VedaFlow review surface.
   The New Learnings placeholder and its stale “not built” test are deleted;
   no proposal or quarantine UI is duplicated.
10. Pure wire/scope/progress tests, real-component server rendering tests, the
    production console build and `make ci` pass. Database tests are not
    required because this package changes no schema, store, policy or gateway
    behaviour already proved by CPR-18.

## Decision

No new ADR. ADR-0075 fixes the product shell and generated-client rule;
ADR-0081 fixes the one governed Knowledge mutation seam; ADR-0082 fixes fresh
Knowledge reads and per-object disclosure; ADR-0083 fixes the candidate-only
capture boundary and command contract. CPR-19 is their product presentation,
not a new architectural choice.

## Completion evidence

Delivered from `e778a6041bc6b56621c9aeb313ca2757da2b9471` on 2026-08-24.

`console/src/Learnings.tsx` replaces the placeholder route. The page groups
the generated batch and candidate collections, resolves exact source events
against the session timeline, gates raw evidence on the run's
`session.diagnostics` forecast, and re-reads every visible match through the
generated Knowledge detail operation. Publication destinations come only from
positive `knowledge.write` anchors and are named private/project/workspace.

All five terminal command families plus scope change use generated,
idempotency-keyed operations. Placement changes send explicit nulls where
required; merge and replacement use the exact revision retained by extraction.
Applied results link to Knowledge, pending results link to Advanced Reviews,
and dismissed candidates state that nothing was published. The proposal
review engine is unchanged and no capture-specific proposal model exists.

Acceptance evidence:

- `console/src/learnings.test.mts`: 8 pure filter, scope, generated-wire,
  precondition, grouping and outcome cases;
- `console/src/learnings.test.tsx`: 6 real-component rendering cases covering
  every action, source evidence, comparison, scope denial, read-only sessions,
  applied/pending outcomes and dismissal;
- complete console suite: 165/165;
- production TypeScript/Vite build: PASS;
- `make ci`: PASS, including Rust/clippy, dependency/licence/backlog/ADR/API
  drift, Helm, deterministic evaluation parsing, console 165/165 and Claude
  adapter 96/96.

No migration, SQLx metadata, OpenAPI shape, Cedar action or audit action
changed. CPR-18's database/API suite remains the authority for mutation,
PDP, RLS and audit behaviour; this package adds no alternative path around it.
