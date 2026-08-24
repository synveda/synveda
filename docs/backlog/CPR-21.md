---
title: "CPR-21: Context Inspector and outcome feedback"
labels:
  - epic:CPR
  - phase:5
size: L
---

# CPR-21: Context Inspector and outcome feedback

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** L

## Description

Turn CPR-20's freshly re-authorised planner trace into the linkable Context
Inspector. A reader can move from the exact context-run entry on a session
timeline to the task, immutable selected Knowledge, visible provenance,
reason/score evidence, token budget, exclusions, implementation versions,
degradation and rendered hash that explain what Synveda supplied. Explicit
feedback names one selection and revision; retrieval alone never becomes a
positive outcome.

## Acceptance criteria

1. `/console/context-runs/{id}` is a refreshable, generated-API-backed route.
   It reads `GET /v1/context-runs/{id}` and never reconstructs a trace from
   session events, direct storage or hand-written DTOs.
2. The inspector shows the original task/query when retained; selected exact
   Knowledge revision, title/body/summary and source evidence when disclosed;
   current/stale/superseded planning state; rank, token cost, reason codes; and
   keyword, semantic, freshness, pin and current-state score contributions.
   Graph contribution is shown only when a graph version/evidence exists.
3. Requested/actual token budget, total rendered tokens, visible exclusions
   and their reasons, retrieval/embedding/index/graph versions, completion,
   degradation warnings and the rendered context hash are explicit. A token-
   budget exclusion is not rendered as a policy exclusion.
4. Full, redacted, hashes-only and disabled traces are distinguishable and do
   not imply missing content is empty content. The aggregate policy message is
   shown without a denied id, title, edge, reason or count.
5. `referenced_by_agent`, `accepted_by_user`, `helpful`, `unhelpful` and
   `caused_correction` are available only on a selected revision with a visible
   exact selection id. Each generated mutation carries both ids and an
   idempotency key, invalidates the detail after success and never marks all
   retrieved or selected revisions helpful automatically.
6. A session timeline renders `Synveda supplied N knowledge items` for a
   context run and links that exact entry to its inspector. The summary no
   longer repeats the run query on the broader timeline surface.
7. A denied or revoked detail renders only the common refusal. Hashes-only
   selections cannot grow ids or feedback controls in frontend state; authored
   pack/skill masking and every backend policy exclusion remain intact.
8. Tests prove superseded material is not presented as a current selection,
   token-budget exclusion is explained, every retention mode renders honestly,
   feedback targets the exact run/selection/revision, and session/context
   routes retain project isolation. Existing CPR-20 gateway adversarial tests
   remain the backend authority.
9. No schema, Cedar action, audit action or OpenAPI operation is added. The
   generated contract remains 67 operations; the inspector is a product view
   over the governed trace rather than another telemetry model.
10. Pure helper/wire tests, real-component server-rendered acceptance tests,
    the focused timeline/API tests, production console build, `make ci` and
    `make db-test` pass.

## Decision

No new ADR. ADR-0075 fixes the console shell, generated-client and capability-
forecast rules; ADR-0077 fixes the session timeline projection; ADR-0084 fixes
trace persistence, re-authorisation, retention and exact feedback. CPR-21 is
their product presentation and introduces no new architecture.

## Completion evidence

Delivered from `8ed8aa63f2517f52435f1ae674b407c89c4f0499` on 2026-08-24.

- `/console/context-runs/{id}` uses the generated detail and idempotent
  feedback operations. The session timeline links its exact run and the
  server now computes its content-free summary from freshly visible
  selections rather than repeating task/token detail.
- Full/redacted/hashes-only/disabled and refusal component acceptance: **6/6**;
  pure retention/matching/feedback-wire rules: **7/7**; complete console:
  **179/179**; production build: **PASS**.
- Database-backed context API: **3/3**; sessions API: **22/22**. The private-
  selection regression proves the shared timeline leaks no item/revision/
  content/reason/count and returns only the aggregate policy notice.
- `make ci`: **PASS**. Full fresh-scratch `make db-test`: **PASS**, including
  the 1k-event ingestion gate. No schema or generated-contract artefact moved;
  the schema remains epoch **2**, **49 migrations**, and OpenAPI remains **67
  operations**.
- The in-app browser runtime had no connected browser, so no interactive
  visual claim is made. The production component itself is server-rendered in
  all six disclosure states and the Vite production bundle builds.
- Commit: `feat(console): add context inspector and feedback (CPR-21)`; hash
  recorded by the CPR-22 checkpoint under the programme convention.
