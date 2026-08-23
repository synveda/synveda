---
title: "CPR-12: Durable Claude session delivery"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-12: Durable Claude session delivery

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** XL

## Description

Prompt 12 of the 33-prompt context-platform programme, and the first one
that makes something outside this repository write to the session plane.

CPR-10 built the record — `sessions`, `session_events`,
`session_context_runs`, a derived scope, a five-state lifecycle. CPR-11
made it usable — pagination, filters, two clocks, a payload route, an
address per run. Between them they are eleven routes and four tables that
**nothing wrote to**. Every observation this product actually received
still arrived at `POST /v1/observe` under a `session_id` string the adapter
invented, and every block it composed still came from `POST /v1/inject`.

Two planes claiming to be the answer to "what has this agent been doing" is
not a migration in progress; it is a product with two answers. This closes
it.

## What existed before it

**The adapter's delivery was durable in name only.** `flush.mts` appended
to a spool file and fired a delivery on the Stop hook. A delivery that
failed wrote nothing back: there was no attempt count, no last-attempt
instant, no acknowledgement state and no sequence — so a later hook could
not tell what had been sent from what had not, and the only correct thing
it could do was nothing. An offline laptop lost the session. A gateway
restart lost the batch in flight.

**The write seam was global.** `POST /v1/observe` took `session_id: text`
and landed a memory at the *caller's own home scope*, because that is all
it knew. Nothing about the request said which workspace the work belonged
to, so nothing could put the memory where the work happened.

**The three global routes had no owner.** `/v1/observe`, `/v1/inject` and
`/v1/recall` were untouched by CPR-10 and CPR-11 by design — the two
prompts were additive on purpose. That left the extraction pipeline keyed
on `observe_events`, the eval harness measuring a plane the product had
stopped describing, and `ObserveKind`'s three names beside
`SessionEventType`'s thirteen.

## What it adds

### A durable local spool

One file per run under the user's config directory, holding a versioned
envelope:

| field | why it is there |
| --- | --- |
| `spool_version` | a format that changes is refused, never guessed at |
| `client_installation_id` | which install produced this, stable across runs |
| `session_id` | the Synveda run, once opened |
| `client_event_id` | the idempotency key the server dedupes on |
| `sequence` | the client's own order, before the server assigns its own |
| `event_type` | the session vocabulary, not the observe one |
| `occurred_at` | when it happened, per the client's clock |
| `payload` | what happened |
| `payload_hash` | SHA-256 over the canonical form |
| `delivery_attempts` | how many times this was tried |
| `last_attempt_at` | when, so a retry can back off |
| `acknowledged` | the only state that permits deletion |

Persisted atomically: write a temp file in the same directory, `fsync`,
`rename`. A kill mid-write leaves either the old file or the new one and
never a half of either. **The previous format is not read** — there is no
migration path and no reader for it, per the programme's hard-cut rule.

### Hooks own delivery, the CLI owns diagnostics

| hook | what it does |
| --- | --- |
| `SessionStart` | opens or resumes a run; retries the unacknowledged backlog |
| `Stop` | records the turn's events and starts a delivery |
| `PreCompact` | records, so a compaction does not swallow the turn |
| `SessionEnd` | a **bounded** synchronous flush |

Bounded because a hook that blocks a person's editor is a hook they
disable. `SessionEnd` gets 3s and `Stop` gets 2s; what does not go stays
spooled and the next `SessionStart` retries it.

Three commands, and they diagnose rather than deliver:

```
synveda session flush                     # deliver everything now
synveda session spool status              # what is held, and why
synveda session spool purge --acknowledged
```

`purge` **requires** `--acknowledged` and offers no `--all`. A command that
can delete undelivered observations on a typo is a command that will.

### Context injection is a context run

`POST /v1/sessions/{id}/context-runs`, which ADR-0076 decision 7 had
already declared the final shape. The adapter's `SessionStart` composes
against the run it just opened.

### The cutover

Deleted: `/v1/observe`, `/v1/inject`, `/v1/recall` and their handlers; the
`observe_events` staging table, the `observe` PGMQ queue and
`observe_quarantine` (migration `0046`); `ObserveKind`; the adapter's
`flush.mts` and every call it made; `demos/ctx-5-recall.sh`.

Re-pointed: the extraction worker onto `session_events` and its own queue;
`synveda recall` and `synveda mcp` onto context runs; the eval harness onto
the session plane.

## Decisions (ADR-0078)

1. `POST /v1/sessions/{id}/events` is the only write seam.
2. A session event is the extraction unit; the queue is `session_events`.
3. **A memory lands at the scope the run was decided at**, not the
   submitter's home.
4. Quarantine is a withheld signal, never a mutated event.
5. Context injection is a context run; `/v1/recall` is deleted.
6. The spool is a durable local queue with a versioned format, hashed with
   SHA-256 because the verifier is Node.
7. Hooks own delivery, the CLI owns diagnostics.
8. The event-loss boundary is real, bounded and documented.

## Acceptance criteria

1. A spool survives a kill mid-write and reads back as either the old state
   or the new one, never as a truncated file.
2. A redelivered batch appends only what is new and answers `duplicate` for
   the rest, **at their original sequence positions**.
3. An event whose `payload_hash` does not match its payload is refused
   rather than stored.
4. `SessionEnd` returns inside its budget with either the spool flushed or
   the backlog intact — never with events dropped.
5. A `SessionStart` after a failed delivery retries it and acknowledges it.
6. `spool purge --acknowledged` deletes only acknowledged entries;
   `spool purge` alone is refused.
7. A memory extracted from a run lands at the **run's** scope and is
   readable by a workspace member who is not its author.
8. `/v1/observe`, `/v1/inject` and `/v1/recall` answer 404 by name.
9. The whole round trip — hook writes, gateway appends, worker extracts,
   context run composes — runs against a live stack.

## Tests

- `crates/synveda-cli/src/spool.rs` — 13 unit tests on the format, the
  hash and the atomic write.
- `crates/synveda-gateway/tests/session_ingest_load.rs` — the <20ms ack
  budget under load, carried over from MEM-1 and re-anchored.
- `crates/synveda-gateway/tests/session_redaction.rs` — MEM-2's scanner on
  the new seam.
- `crates/synveda-gateway/tests/context_runs.rs` — CTX-3's ladder and
  budget on the context run.
- `crates/synveda-gateway/tests/sessions_api.rs` — the append seam,
  duplicates at their original positions, and the three 404s.
- `crates/synveda-ingest/` — extraction and promotion off `session_events`.
- `adapters/claude-code/src/*.test.mts` — the spool, the four hooks and the
  bounded flush.

## The event-loss boundary

**A host client that terminates before any lifecycle hook can execute takes
the un-flushed tail with it.** No hook runs, so nothing writes; the spool
holds what the last `Stop` recorded and no more.

This is a property of the hook contract, not of this implementation.
Claude Code fires `Stop` at the end of each turn, so the boundary is "since
the last turn ended" rather than "since the session started" — usually
seconds, and bounded by one turn. A SIGKILL to the editor, a kernel panic
or a battery death inside that window loses that turn's events.

It is documented rather than closed because closing it means writing on
every token, which is a different product with a different cost. What this
design guarantees instead: **nothing that reached the spool is ever lost**,
because the spool is durable, the format is atomic and the retry is
idempotent.
