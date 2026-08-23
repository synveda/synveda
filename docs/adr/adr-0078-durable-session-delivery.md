# ADR-0078: Durable session delivery and the observe/inject/recall cutover

- **Status**: Accepted
- **Date**: 2026-08-25
- **Feature(s)**: CPR-12
- **Deciders**: Prompt 12 of the CPR programme

## Context

CPR-10 built the session ledger and CPR-11 made it readable. Neither of them
moved a single client onto it. The Claude Code adapter — the only agent
integration this product actually ships — still spoke to three global routes
that predate the whole programme:

- `POST /v1/observe`, which admits a batch of transcript deltas into
  `observe_events` keyed by an opaque `session_id: text`;
- `POST /v1/inject`, which composes a context block at the caller's home
  scope;
- `POST /v1/recall`, which searches memory and is read by `synveda mcp` and
  `synveda recall`.

So the product had **two runtime models running side by side**, exactly the
coexistence ADR-0068 decision 3 forbids, with CPR-10 and CPR-11 explicitly
recording that nothing bridged them and that the cutover was open and
unscheduled.

Three separate problems made that coexistence worse than untidy.

1. **Delivery was not durable, and the design said so.** ADR-0027 decision 7
   made the adapter's spool a *cursor*: the uuid of the last transcript entry
   a gateway 2xx had accepted, and nothing else. Everything after the cursor
   was re-derived on the next hook by re-reading the harness's transcript
   file. That is elegant and it has one fatal property — it is only
   at-least-once **while the transcript file still exists and still contains
   the entries**. A compaction rewrites it. A `/clear` truncates it. A
   project deleted between two sessions takes it. And a `Stop` hook that
   fired while the gateway was down left no local record at all that anything
   had happened: the events existed only as a byte range of somebody else's
   file that the adapter had chosen not to copy.

2. **A run an agent only read in did not exist.** ADPT-8 measured this
   against a headless Claude Code run: three runs, three `inject.ok`, **zero**
   `observe.done`. The correlation string meant a session was a label on
   somebody else's rows rather than a thing, so a run with no observations
   was indistinguishable from no run.

3. **Extraction had no session.** `observe_events` fed the whole knowledge
   pipeline — scan, extract, embed, dedup, link, promote — and the only thing
   it knew about provenance was a text label and the submitter's home scope.
   A memory formed in a shared project therefore landed in the person's own
   scope, which is both wrong and unfixable without a session to ask.

## Decision

### 1. `POST /v1/sessions/{id}/events` is the only write seam

`/v1/observe` is deleted: the route, `observe.rs`, `ObserveBody`,
`ObserveKind`, `ObserveEventId`, `observe_events`, and the store module behind
them. Everything it did that was worth keeping moves onto the session event
append, which already had the harder half — ordering, a server-assigned
sequence, per-event idempotency by the client's own id, and an immutable
append-only table.

What moves with it, unchanged in kind:

- **The redaction scan runs between validation and the insert**, as it has
  since ADR-0021 decision 1: every payload is scanned and redacted before
  anything persists, and the effective pack's redaction config picks each
  event's disposition. Raw finding text still survives in no table, response,
  metric or audit payload.
- **The work signal is enqueued in the caller's transaction**, so events,
  signals and the audit event commit or vanish together.

What changes is the vocabulary. `ObserveKind`'s three values
(`transcript_delta`, `tool_result`, `decision`) are gone; extraction now
routes on `SessionEventType`'s twelve, which is a strictly better signal —
`file.changed` and `command.executed` were both `decision` before.

### 2. A session event is the extraction unit, and the session is its provenance

The extraction worker loads `session_events` rows rather than
`observe_events`. `ExtractionInput` carries a `SessionEventId`, a real
`SessionId`, and a `SessionEventType`. The queue is renamed `session_events`
rather than reused: a queue whose name says `observe` and whose messages name
session events is a trap for the next reader.

### 3. A memory lands at the scope the run was decided at

This is the decision with product consequences, and it is the point of the
whole cutover.

`/v1/observe` wrote at `identities.scope_id` — the submitter's own principal
scope — because it had nothing else to go on. A session **knows where it
ran**: its `scope_id` is derived from its workspace and project by two
composite foreign keys and a CHECK (ADR-0076), and cannot be forged by a
client. So extraction commits records at the session's scope and decides
`MemoryWrite` there.

The consequence is the intended one: a run against a shared project produces
project memories that the project's members can compose, and a run at
somebody's own principal scope produces private ones that
`base.cedar`'s personal-scope forbid keeps private. Under the old rule every
memory from every run landed in one person's home scope, so a team using this
product accumulated a pile of individually-owned notes about shared work.

### 4. Quarantine is a withheld signal, never a mutated event

`session_events` has no UPDATE grant and must not acquire one — immutability
there is a privilege, not a discipline (ADR-0076). So a quarantined event is
**inserted like any other** and simply gets no work signal; the review state
lives in its own table, `session_event_quarantine`, and a release enqueues the
signal that admission withheld. The event row carries only `redactions`, the
scan's finding summary, which is immutable provenance and belongs on the row.

`observe_quarantine` and its `/v1/quarantine` handlers are re-anchored on the
new table rather than deleted: the review plane itself was never the problem.

### 5. Context injection is a context run

`/v1/inject` is deleted. `POST /v1/sessions/{id}/context-runs` was declared
the final shape of this endpoint by ADR-0076 decision 7 and has been serving
since CPR-10; there is no reason for two composition endpoints and one of them
could not say which run it was composing for.

`/v1/recall` is deleted with it. Its two callers — `synveda mcp`'s tool
surface and `synveda recall` — open a session (`client_name` `mcp` and `cli`
respectively) and compose a context run, which makes what was previously an
unattributable search into a governed record of who asked for what.

### 6. The spool is a durable local queue with a versioned format

The adapter's spool stops being a cursor into somebody else's file and becomes
**the record of what happened**. One file per session under
`$XDG_STATE_HOME/synveda/spool/`, holding every field this feature's
acceptance names:

| Field | Where | Why there |
|---|---|---|
| `spool_version` | header | The format's own version. |
| `client_installation_id` | header | Constant for the file's life. |
| `session_id` | header | The Synveda session; absent until one is opened. |
| `client_event_id` | entry | The idempotency unit the API keys on. |
| `sequence` | entry | The **client's** local order. |
| `event_type` | entry | One of the twelve. |
| `occurred_at` | entry | The client's statement about when. |
| `payload` | entry | The content. |
| `payload_hash` | entry | SHA-256 over the canonical encoding. |
| `delivery_attempts` | entry | How many times delivery was tried. |
| `last_attempt_at` | entry | When the last one was. |
| `acknowledged` | entry | Whether the gateway has it. |

The three header fields are constant per file and are written once rather than
repeated on every entry. Everything else is per entry, because everything else
varies per entry.

**`sequence` here is the client's, not the server's.** The server assigns its
own on append and that one is authoritative for ordering a timeline. The
spool's is what makes a bounded flush deterministic and what lets
`spool status` say "events 7 through 19 are unacknowledged" rather than
counting.

**The hash is SHA-256 and not BLAKE3**, which is the one place this format
diverges from the rest of the product. The writer is Node with no dependencies
beyond `@types/node`, and `node:crypto` has no BLAKE3. This hash's job is
detecting local corruption of a file the adapter wrote and the CLI reads; the
authoritative digest of an event is the server's BLAKE3, computed on append
over the canonical payload, and nothing about that changes.

**Persistence is atomic**: write a temporary file in the same directory,
`fsync` it, then `rename` over the target. A hook killed mid-write leaves the
previous good file, never half of a new one.

**Nothing reads the previous format.** A `~/.local/state/synveda/sessions/`
file from before this cut is not migrated, not parsed and not consulted; the
directory is removed on sight. The old format held a cursor and no events, so
there is nothing in one to recover — translating it would produce an empty
spool with extra steps.

### 7. Hooks own delivery, and the CLI owns the diagnostics

| Hook | What it does |
|---|---|
| `SessionStart` | Opens or resumes the Synveda session, **retries the backlog**, composes a context run, returns it as context. |
| `Stop` | Records the turn's events into the spool and returns; delivery is attempted after the record is durable, never before. |
| `SessionEnd` | A **bounded** synchronous flush — a fixed deadline and a fixed attempt count, because a hook that blocks a client's exit indefinitely is worse than one that leaves a backlog. |
| next `SessionStart` | Retries whatever the last one could not acknowledge. |

Three CLI commands make the spool something a person can act on rather than a
directory they are told about:

    synveda session flush                      # deliver every unacknowledged event
    synveda session spool status               # what is held, per session
    synveda session spool purge --acknowledged # delete what the gateway has

`purge` deletes **only acknowledged entries**, and the flag is required rather
than defaulted. There is no `--all` and no unqualified `purge`: the one
irreversible thing this plane can do to an undelivered observation is delete
it, and a command that does that by default is a command that eventually does
it by accident.

### 8. The event-loss boundary is real, is bounded, and is documented

If the host client terminates without running **any** lifecycle hook — `kill
-9`, a panic in the harness, a machine losing power mid-turn — the events of
the turn in flight are lost. They were never handed to a hook, so no code of
this product's ever saw them.

This is not fixable from inside a hook contract, and pretending otherwise
would be worse than stating it:

- A daemon watching the transcript file would fix it, and ADR-0027 decision 1
  ruled out a background process for reasons that still hold — it is a second
  thing to install, supervise, upgrade and debug, and it observes projects
  whose hooks are disabled.
- Writing the spool from `PreToolUse`/`PostToolUse` would narrow the window to
  one tool call and would put this adapter in the path of **every** tool
  invocation, which is a latency budget it should not be spending.

What the design *does* guarantee is that everything a hook has been handed is
durable before delivery is attempted, and survives an unreachable gateway, a
killed hook, a compaction and a reboot. The boundary is therefore "the turn in
flight when the client died", not "everything since the gateway went down".
`README.md` and `docs/INSTALL.md` state it in those words.

## Consequences

- The product has **one** runtime write path, one composition endpoint, and
  one identity for a run. `observe_events`, `observe_quarantine`, the
  `observe` queue, `ObserveKind` and `ObserveEventId` are gone.
- Memories change scope. A deployment that ran the old model has no data to
  migrate — the epoch guard refuses pre-cut databases — so this is a change of
  rule rather than a change of rows.
- The spool is a real queue on the user's disk, and therefore a thing that can
  grow. `spool status` reports its size, `purge --acknowledged` bounds it, and
  a session's file is removed when every entry in it is acknowledged and the
  session is closed.
- The eval harness now opens sessions. Its scores are not comparable across
  this commit: extraction routes on a different vocabulary and records land at
  a different scope. `docs/BENCHMARKS.md` records that the published row
  predates the cut, and re-measurement is Prompt 32's.
- `make ci` keeps the epoch at **2**. This migration adds and drops rather
  than rewriting the chain: the epoch marker exists to refuse databases from
  before the cut, not to keep the chain tidy, and CPR-10 and CPR-11 both set
  the precedent for additive change within it. The create-then-drop pairs it
  leaves behind are Prompt 33's squash, which is where CPR-9's three
  unreachable pre-epoch statements already wait.
