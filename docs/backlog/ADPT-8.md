---
title: "ADPT-8: Observation that survives a session that does not wait"
labels:
  - epic:ADPT
  - phase:4
size: M
---

# ADPT-8: Observation that survives a session that does not wait

**Epic:** ADPT — Adapters & SDKs · **Phase:** 4 · **Size:** M

## Description

Before CPR-14, a Claude Code session that was not a person at a terminal —
`claude -p`, CI, a script, an agent harness — read governed memory and wrote
none of it back. Injection was synchronous and landed; every write path was
`async: true`, so the harness did not wait for it and the process exited first.
CPR-14 closed that gap with a synchronous **local durability** boundary and
deferred network delivery, then proved it through the installed real client.

## Why this exists

Filed 2026-08-13 by running ADPT-1's plugin in a real Claude Code session
(2.1.228) against an installed v0.1.3, rather than by reading the code.

Three headless sessions produced three `inject.ok` and **zero** `observe.done`.
Not an error — the hook never ran at all, and every session exited 0:

```
05:47:06  inject.ok  records=3  tokens=206  elapsed_ms=31
05:48:47  inject.ok  records=3  tokens=206  elapsed_ms=26
05:48:56  inject.ok  records=3  tokens=206  elapsed_ms=29
```

Interactive sessions the previous day, same machine, observed correctly:

```
13:19:30  observe.done  hook=SessionEnd  events=2  accepted=2
13:25:28  observe.done  hook=Stop        events=5  accepted=5
```

## What the old manifest said

`hooks/hooks.json`, as shipped:

| event | verb | timeout | async |
|---|---|---|---|
| SessionStart | `session-start` | 5 | **false** |
| SessionStart | `skills` | 10 | true |
| Stop | `observe` | 5 | true |
| PreCompact | `flush` | 5 | true |
| SessionEnd | `flush` | 5 | true |

Only the read path is synchronous. That is a deliberate and defensible choice
for an interactive session — observation must not make somebody wait — and it
is the whole of the failure everywhere else.

## What is not wrong

Establishing this mattered, because the fix depends on it:

- **The hook works.** Run by hand against the same session's transcript, exactly
  as Claude Code would invoke it, it accepted both turns:
  `observe.done hook=Stop events=2 accepted=2 duplicates=0`. That produced
  `memory.observed` → `memory.extracted` on the chain, which stayed valid.
- **The turns are not destroyed.** Claude Code's transcript stays on disk, and
  the session spool holds `session_id`, `updated_at` and `transcript_path` with
  **no `cursor`** — the field the observe hook advances after sending. An absent
  cursor means "nothing sent yet", not "nothing to send".

So the data survives, addressably, and nothing ever collects it — because the
only things that would (`observe`, `flush`) are the async hooks that did not run.

## Why this is a feature and not a one-line change

The obvious fix considered in 2026-08-13 — make the whole `Stop` operation
synchronous — would tax every interactive turn up to
5s to protect the headless case, which inverts the trade-off the current
manifest deliberately makes. Three candidates, and choosing wants an ADR
amendment rather than a commit message:

- **A catch-up flush at the next `SessionStart`.** Costs interactive users
  nothing, since that hook already runs synchronously and already talks to the
  gateway. Makes headless observation *eventually consistent* rather than lost:
  a spool with an unadvanced cursor and a transcript still on disk is exactly
  the input this needs. Its weakness is the tail — a session that is never
  followed by another in the same project is never collected — and whether that
  matters depends on whether the spool is swept by something else.
- **A synchronous `SessionEnd` only.** Bounds the wait to once per session
  rather than once per turn, and `SessionEnd` is the point at which nobody is
  waiting on a response anyway. Weakness: `SessionEnd` is not guaranteed on a
  killed process, which is how CI ends things.
- **A synchronous `Stop`.** Complete, and the most expensive. Named here so the
  decision records why it was not taken.

The first two compose, and probably should.

## What to check while implementing

- Whether `Stop` fires at all under `-p`, or only `SessionEnd` — today's
  evidence cannot distinguish "fired and was killed" from "never fired", because
  a killed async hook logs nothing either way. The spool's missing cursor says
  it did not *complete*; it does not say it did not start.
- Whether the spool is swept, and by what. If nothing prunes it, an
  eventually-consistent design has an unbounded directory; if something prunes
  it aggressively, the catch-up window is shorter than it looks.
  `prune()` runs on `SessionEnd` — which is one of the hooks that does not run
  here, so its behaviour under this failure is itself unverified.
- Idempotency, which is what makes any of this safe to retry: `memory.observed`
  reports `duplicates`, and the manual re-run reported `duplicates=0` against a
  session whose earlier turns had never been sent. A catch-up flush must lean on
  that property deliberately rather than incidentally.

## Resolution (2026-08-24)

The installed authenticated Claude Code **2.1.241** gate resolved the open
question: with `Stop` still async, a successful headless run emitted only its
SessionStart hooks, exited, and left the Synveda session active with zero
events. Claude Code kills async hooks which are still running when print mode
tears down.

The fix does not make a turn wait for observation delivery:

- `Stop` and `PreCompact` are synchronous only through transcript conversion
  and the spool's atomic temp-write → `fsync` → rename. They resolve no
  credential and make no network request.
- `SessionEnd` performs the bounded flush and close; the next SessionStart and
  `synveda session flush` recover anything still pending.
- The local-only Stop contract is asserted with a failed gateway: events are
  durable, `delivery_attempts` remains zero, and no append request exists.

The genuine rerun loaded plugin **0.2.0** through the supported marketplace
path, reported four hooks and one MCP server enabled, composed one context run,
persisted four authentic user/tool/assistant events and ended the same run. The
chain, rather than `adapter.log`, contains the governed session actions. Stop's
local boundary measured **8ms**; gateway append happened at SessionEnd and
measured **28ms**. ADR-0027 amendment 3 records the revised seam.

## Acceptance criteria — met

- A **headless** `claude -p` session's turns reach the audit chain —
  `memory.observed` with `accepted > 0` for that `session_id` — without any
  manual step.
- An interactive turn does **not** wait for observation to complete. Whatever
  the mechanism, this is asserted rather than assumed, because it is the
  property the current design is spending correctness to buy.
- The demo asserts the vendor's own view of the outcome rather than the
  adapter's: the chain, not `adapter.log`. ADR-0027's amendment already records
  that ADPT-1's demo is its own harness and therefore cannot see what the real
  one does — this AC must not repeat that.
- Whatever is chosen, the **tail case is stated**: which sessions are still
  never observed, and why that is acceptable. A design that quietly loses the
  last session of a project is the same silent gap in a smaller box.

The remaining tail is a host killed before any lifecycle hook receives and
writes the in-flight turn. Nothing that reached the spool is lost; the replay
gate proves outage recovery, overlapping duplicate delivery and exactly-once
server persistence.
