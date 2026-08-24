---
title: "CPR-22: Core individual and small-team MVP acceptance"
labels:
  - epic:CPR
  - phase:5
size: L
---

# CPR-22: Core individual and small-team MVP acceptance

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** L

## Description

Prove the complete personal-and-team loop over one runtime with a realistic
PulseBoard scenario. Session evidence must become reviewable candidates;
accepted candidates must become immutable, sourced Knowledge only through
VedaFlow; project Knowledge must cross a clean-session and principal boundary;
principal Knowledge must not; and a correction must create explicit
supersession that the next context plan and its inspector can explain.

## Acceptance criteria

1. Alice creates a PulseBoard workspace/project, grants Bob project membership
   and opens a public session. Four ordinary session events state webhook
   event-id deduplication, the current `X-Request-Id` convention, Alice's
   `test-fast` preference and one incidental detail.
2. One idempotent capture snapshot yields four pending candidates with exact
   session-event evidence and zero active Knowledge. Alice publishes the first
   two at project scope, accepts the preference at her principal scope and
   dismisses the incidental detail. Every publication produces an applied
   Knowledge VedaFlow change; dismissal produces none.
3. A clean Bob session selects both project revisions with source evidence and
   does not disclose Alice's private item through rendered context, candidate/
   selection addresses, Knowledge detail or the scoped query lens.
4. Bob records that `traceparent` replaces `X-Request-Id`. A second capture
   candidate is resolved with the public replace command against the exact old
   revision. The old aggregate is superseded, the replacement is active, and
   an immutable `supersedes` relation and both sessions' source evidence remain.
5. A third clean session selects the replacement revision and never selects or
   renders the obsolete revision. Its generated context detail exposes the
   selected revision, session-event provenance, reasons, rank, token charge,
   retrieval version and rendered hash, while retaining the old item only as
   an explained `superseded` exclusion.
6. The session timeline exposes the exact context-run address and its
   content-free `Synveda supplied N knowledge items` summary. The generated
   detail shape remains the Context Inspector's only source of UI truth.
7. Database assertions cover sessions/events, capture batches/candidates/
   decisions, VedaFlow changes, Knowledge items/revisions/current states,
   explicit relations and context runs/selections. There is no record dual
   write and `/v1/observe`, `/v1/inject` and `/v1/recall` remain 404.
8. The hash chain verifies and contains session, capture, Knowledge, context
   and allowed PDP decision evidence without session or Knowledge plaintext.
   Existing cross-tenant, source-disclosure and generated-console suites stay
   green.
9. The package adds no schema, route, DTO, policy action, audit action or
   product shortcut. It is an acceptance gate over CPR-3 through CPR-21, not a
   second implementation.
10. The focused database test, isolated runnable demo, complete console suite,
    `make ci` and `make db-test` pass.

## Decision

No new ADR. ADR-0070 through ADR-0074 fix scope, access and PDP semantics;
ADR-0076 through ADR-0078 fix sessions and adapter delivery; ADR-0080 through
ADR-0084 fix Knowledge, capture and context. This package tests their composed
product invariant and makes no new architectural choice.

## Completion evidence

Delivered from `8cdd1eeda974f3e7a830091c5200added254f0b9` on 2026-08-24.

- The consolidated public-API/database acceptance is **1/1** and the complete
  capture integration binary is **4/4**. It creates 3 sessions, freezes 5
  exact-event candidates, records 5 terminal decisions and 4 applied
  Knowledge changes, leaves 3 current plus 1 superseded item, composes 2
  context runs with 3 immutable selections, verifies the audit chain and
  writes zero old records.
- Bob receives both Alice project items and their session-event sources in a
  clean run, while her principal-owned `test-fast` preference is absent from
  render, trace, detail and query. Bob's correction enters capture and the
  public replace command; the old item remains historical behind one explicit
  `supersedes` relation and the third run selects only the replacement.
- The final generated detail carries the exact fields the Context Inspector
  renders, and the session timeline names its exact run with the content-free
  supplied-item summary. Existing context isolation/planner tests are **3/3**
  and the complete real-component console suite is **179/179**.
- `demos/cpr-22-mvp-acceptance.sh`: **PASS** on an isolated epoch-2 database,
  reporting the counts above. This is deterministic public application
  acceptance; it does not claim a second proprietary live-client run. CPR-14
  remains the genuine Claude Code 2.1.241 evidence.
- `make ci`: **PASS** (the restricted first attempt could not bind two CLI
  loopback listeners; the unchanged unrestricted gate passed). Full fresh-
  scratch `make db-test`: **PASS**, including capture **4/4**, context **3/3**,
  foundation isolation **6/6** and the 1k-event gate.
- No schema, OpenAPI, generated client, Cedar action or audit vocabulary
  changed: epoch **2**, **49 migrations**, **67 operations**.
- Commit: `test(mvp): verify cross-session team knowledge loop (CPR-22)`; hash
  recorded by the CPR-13 checkpoint under the programme convention.
