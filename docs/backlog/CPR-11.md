---
title: "CPR-11: The session product experience"
labels:
  - epic:CPR
  - phase:5
size: L
---

# CPR-11: The session product experience

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** L

## Description

Prompt 11 of the 33-prompt context-platform programme. CPR-10 made a run a
**governed record**; this makes it a record somebody can use.

The distinction matters because they are different pieces of work and only
one of them was done. CPR-10 delivered three tables, seven routes, two Cedar
actions, four audit action types and a console page — and every one of its
fourteen acceptance tests is about whether a run is correctly stored,
decided and chained. None of them is about whether a person with a question
can get an answer.

## What existed before it

`GET /v1/sessions` served up to `limit` rows, newest first, with
`truncated: true` when there were more. The console called it once and
expanded a run in place.

Four gaps, and each is a hole rather than a polish item.

1. **A run older than one answer was unreachable.** `truncated` says *that*
   an answer was cut short. It cannot say **where to continue**. A
   deployment whose agents open a few hundred runs a week therefore had no
   API path to last Tuesday's run — not a slow one, none. The filters were
   `scope_id`, `workspace_id`, `project_id`, `status`: nothing narrowed by
   who ran a thing, which client, or when.

2. **A timeline reported one clock.** `session_events.received_at` has been
   stored since migration `0044` and served by nothing. The adapters this
   product ships **spool to disk when the gateway is unreachable** and flush
   later, so an hour of a transcript arrives at once, an hour late — and a
   reader with one clock sees a perfectly plausible transcript with no sign
   that any of it was recovered.

3. **A payload could neither be read nor be withheld.** An event's payload
   was echoed back to the client that wrote it and served by no read route.
   So the forensic question — what exactly did the agent send? — had no
   answer; and the moment one is added it is the single largest disclosure
   on this plane, priced identically to reading a project's session list.

4. **A run said how it stopped and never why.** `failed` with nothing else
   on it tells a reader something broke and gives them nowhere to go.

## What it adds

**Schema.** Migration `0045`: `sessions.end_reason`, nullable, ≤ 500
characters, forbidden on an `active` row by a CHECK.

**API.**

- `GET /v1/sessions` — keyset pagination. `cursor` in, `next_cursor` out,
  and `truncated` **deleted** rather than kept beside it. Four new filters:
  `client_name`, `principal_id`, `started_after`, `started_before`.
- `GET /v1/sessions/{id}/timeline` — every event entry gains `received_at`
  and a server-computed `delayed`.
- `GET /v1/sessions/{id}/events/{event_id}` — **new**. One event, payload
  included, decided under a **new** Cedar action `SessionDiagnostics`.
- `POST /v1/sessions/{id}/end` — takes `end_reason`; `SessionView` serves
  it; the `session.ended` audit payload carries it.

40 operations on the contract, up from 39; `docs/api/openapi.json` and
`console/src/generated/api.ts` regenerated from the handlers.

**Policy.** `SessionDiagnostics` (`session.diagnostics`) in the schema, in
`Action::ALL` and `Action::PROBED_AT_SCOPE`, and permitted in all three
shipped packs — **@19 → @20** — strictly narrower than each pack's own
`SessionRead`. Not on `base.cedar`'s governance carve-out, so the
personal-scope privacy forbid reaches it unchanged.

**Console.** A route per run (`/console/sessions/{id}`), reached by adding
one level of `:param` pattern to CPR-8's flat route table. A filter bar over
state, project, client, who and a day range. Load more. A detail page with
the run's facts, its repository and branch resolved through the project's own
attachment list, an end-reason banner for a run that did not finish, a
warning banner and per-entry warning marks, an ordered timeline showing
**both clocks** and marking what did not arrive live, and a payload
expansion offered only where the caller's forecast at that run's scope says
so — closed until clicked, and decided again by the gateway when it is.

## Decisions

All five are in **ADR-0077**. The two worth reading before touching this
code:

- **The cursor follows the last candidate a page considered, not the last
  row it served.** Rows are decided one at a time against the row (CPR-9),
  *after* they are scanned. A cursor on the last served row would end a
  listing whenever a whole page was denied, while readable rows sat below
  it. Hence: a page may be **empty and still carry a cursor**, and the
  schema says so.
- **Lateness is one flag, not three.** A locally spooled batch, a replay
  after a crash and a machine with a wrong clock produce the same two
  instants. The server reports the gap and refuses to name a cause.

## Acceptance criteria

1. A listing pages through every run exactly once, newest first, and the
   walk terminates; a cursor the listing did not issue is a 400 rather than
   a silent restart from the newest row.
2. `truncated` is absent from the response rather than kept beside
   `next_cursor`.
3. `client_name` and `principal_id` narrow, the client match is exact rather
   than a prefix, the date window is half-open, and an inverted window is a
   400.
4. Every timeline event entry carries both instants; a context run carries
   neither. An event delivered two hours after it happened is `delayed`; a
   live one is not.
5. An `adapter.warning` is counted in `event_counts` and its own sentence
   reaches the entry's summary.
6. A caller granted `member` at a project reads that run's timeline and is
   refused `session.diagnostics` **by name** on the same run; an
   administrator gets the bytes.
7. A timeline carries no payload text at all.
8. The audit chain records that an event was expanded — id, type, sequence,
   digest — and contains none of its content.
9. An event id from another run, and one nobody ever minted, are the same
   404.
10. A close records a reason, it survives a re-read, it reaches the
    `session.ended` payload, and one over its bound is refused rather than
    truncated.
11. The console renders: an active run; a completed one; a failed one with
    its reason; a delayed entry with both clocks and the gap; a delivery
    warning in a banner and in place; a refusal with not one fact about the
    run in it; and another tenant's id exactly as it renders a fictional
    one.

## Tests

- `crates/synveda-gateway/tests/sessions_api.rs` — 7 new (21 total).
- `console/src/sessions.test.tsx` — 9 UI acceptance scenarios.
- `console/src/sessions.test.mts` — 12 new derivation tests.
- `console/src/routes.test.mts` — the parameterised route, both directions.
- `crates/synveda-store/tests/rls.rs` and `tests/sessions.rs` — the
  `end_reason` column under RLS and through the lifecycle.

## Known-red at delivery

`crates/synveda-gateway/tests/explorer.rs::the_explorer_parity_corpus_is_what_the_gateway_serves`.

CNSL-2's explorer parity corpus (`console/fixtures/explorer`) is a **recording**
of what the capability probe serves, and `session.diagnostics` joins
`Action::PROBED_AT_SCOPE`, so the recording is one action out of date. CPR-10
hit exactly this for `session.read` and `session.write`.

Closed by one command, reading the diff before accepting it — it should be
exactly one line per recorded `actions` map:

```sh
SYNVEDA_RECORD_FIXTURES=1 make db-test
```

It is open because the Docker daemon on the delivering machine wedged and took
the Postgres container with it. The fixture was deliberately **not** hand-edited
to the bytes it will have: a corpus written by hand is precisely the drift this
test exists to catch, and asserting a value nobody observed is worse than a red
test somebody can see. `make ci` — which uses no database — is green.
