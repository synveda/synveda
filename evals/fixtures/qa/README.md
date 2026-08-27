# The Q&A corpus (EVAL-4)

Each corpus appends source events to real sessions, requests capture, accepts
reviewable candidates through the Knowledge/VedaFlow command path, and probes a
clean session through `POST /v1/sessions/{id}/context-runs`. The separate
session-scoped Knowledge evaluation lens supplies the enumeration denominator;
a budgeted context run is never abused as a sweep.

Shared material names an exact current scope alias from `evals/lib.sh`:
principal, project, workspace or tenant. There is no fixed team/department/org
fixed hierarchy and no direct data seed.

## Format

```json
{
  "corpus": "acme-engineering",
  "note": "why this corpus exists",
  "reader": "qa-reader",
  "seed": [
    {
      "actor": "qa-reader",
      "session_id": "qa:acme:principal",
      "visibility": "principal",
      "events": [
        {"key": "own-testing", "event_type": "message.user", "text": "…"}
      ]
    },
    {
      "actor": "qa-project",
      "session_id": "qa:acme:project",
      "visibility": "project",
      "publish_scope": "payments",
      "events": [
        {"key": "project-retries", "event_type": "message.assistant", "text": "…"}
      ]
    }
  ],
  "questions": [
    {
      "name": "project-retry-policy",
      "task": "what does payments cap card retries at",
      "needs": "lexical",
      "expect_knowledge": ["project-retries"],
      "must_not_contain": ["…"],
      "budget_tokens": 260
    }
  ]
}
```

- `visibility` is one of `principal`, `project`, `workspace`, `tenant` and
  drives the `qa_scope_*` metric.
- `publish_scope` is forbidden for `principal` and required for shared
  placements. It names an environment alias, not an implied hierarchy node.
- `expect_knowledge` names seeded event keys. Grading joins the appended
  `event_id` to accepted Knowledge item IDs and current block addresses; text
  containment is not attribution.
- `needs` is `lexical` or `semantic`. Semantic questions must be true
  paraphrases with no content-word overlap; lexical questions must overlap.
- A taskless question cannot be semantic, a zero budget is refused, and all
  input structs reject unknown fields.

## Ranking measurements

`retrieval_precision` includes only questions with an explicit narrow budget
whose resulting block actually selected fewer items than the diagnostic lens
served. This keeps corpus size and scope distance out of a metric that claims
to measure ranking.

The deterministic embedder is lexical-only, so semantic questions are skipped
and counted on `make eval`. `make eval-retrieval` runs the same corpus through
the pinned BGE-M3/TEI path and its separate baseline.

Grow the suite with additional corpora and actors. The evaluation lens is
cursor-paginated; the runner follows cursors up to its explicit corpus limit and
fails repeated cursors rather than silently truncating evidence.
