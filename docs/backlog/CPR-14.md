---
title: "CPR-14: Live Claude Code session acceptance gate"
labels:
  - epic:CPR
  - phase:5
size: L
---

# CPR-14: Live Claude Code session acceptance gate

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** L

## Description

CPR-12 put the Claude Code adapter, extraction and context composition on the
session plane and deleted the old observe/inject/recall plane. Its tests proved
the new pieces, but not their most important join: a current installed Claude
Code client discovering the packaged plugin, invoking its real hooks, receiving
session context, persisting its own activity and ending the same governed run.

This feature makes that claim an acceptance gate with two explicitly different
evidence tiers (ADR-0079):

1. `make claude-acceptance` is deterministic **replay/live-gateway** evidence.
   Genuine, identified Claude Code hook frames and transcripts pass through the
   same built child process registered by the plugin, public session routes,
   embedded PDP, current Postgres schema, ingestion worker, timeline and audit
   chain. It runs in ordinary CI without a Claude credential.
2. `make claude-acceptance-live` is **live-client** evidence. It packages the
   marketplace, installs it through `synveda plugin install`, verifies Claude
   Code's own enabled-plugin, four-hook and MCP-server report, and invokes a
   deterministic headless session using the real authenticated `claude`
   executable. Only this tier closes the live criterion.

The live runner isolates HOME, Claude configuration, Synveda configuration and
raw captures in a scratch directory; creates the principal and first
administrator grant through production JIT provisioning; creates workspace and
project through public routes; and writes all governed data through the session
API. It records exact client, plugin, Synveda and OS versions and deletes its
temporary credentials, configuration and raw frames afterwards.

The replay exercises the harder delivery case rather than a second happy path.
It takes the gateway away after a turn reaches the spool, proves two entries
remain pending, restores the gateway, commits one pending event without updating
the local acknowledgement, then lets the next SessionStart redeliver the
overlap. The first response must be `duplicate` at its original sequence, the
second `appended` at the next one, and the database must contain one row per
client event id.

## Current evidence

Delivered 2026-08-24. Deterministic captured/mock and replay/live-gateway tiers
pass, and the installed authenticated **Claude Code 2.1.241** executable passed
the separately named live-client gate with Synveda plugin **0.2.0**. Claude
Code itself reported the plugin enabled with four hooks and one MCP server. Its
real SessionStart composed exactly one context run; real user, Read invocation,
tool result and assistant activity produced four ordered session events; and
real Stop plus SessionEnd flushed and ended the same run with reason `other`.
The timeline, separately authorised diagnostic payload, verifying audit chain,
spool hashes, server hashes, ordering and acknowledgement assertions passed
without comparing model prose.

The run isolated HOME and Claude/Synveda configuration while handing only the
native macOS credential into the disposable profile through a private 0600
temporary file. It recorded client **5,526ms**, SessionStart **72ms**, Stop
**8ms**, SessionEnd **53ms**, append **28ms** and context-run **15ms** on macOS
26.5.2 (25F84), Darwin 25.5.0 arm64. Replay remains the deterministic outage
and idempotency proof in CI. A process killed before any lifecycle hook writes
the in-flight turn can still lose that tail; the feature makes no broader claim.

## Acceptance criteria

1. A current authenticated real `claude` executable installs the packaged
   Synveda marketplace through the supported command and reports the plugin
   enabled with four hooks and one MCP server.
2. One deterministic headless real-client run creates or resumes exactly one
   Synveda session and obtains context only through
   `POST /v1/sessions/{id}/context-runs`.
3. Authentic user, assistant, tool invocation and tool-result activity reaches
   ordered `session_events`; normal completion flushes the final events and
   `SessionEnd` leaves the run ended with the expected reason.
4. The timeline shows event types, lengths, ordering and delivery timing without
   message text. Raw payload diagnostics remain separately authorised.
5. The verifying audit chain contains the expected session actions and no
   prompt, response, tool result, credential or other sensitive payload.
6. Client event ids, spool SHA-256 checksums, server BLAKE3 payload hashes,
   delivery attempts, acknowledgement state and server sequences agree.
7. During a gateway outage, pending spool entries survive; the next eligible
   lifecycle hook recovers them; a deliberately lost acknowledgement produces a
   duplicate at its original sequence; every event exists exactly once; and an
   acknowledged purge retains pending entries.
8. Every replay fixture validates against the committed fixture contract, is
   bound to genuine capture provenance and a client version, and contains no
   credentials, personal paths or private content.
9. Replay uses the same public session and adapter paths and runs in ordinary CI
   under the explicit replay name; it never reports itself as live execution.
10. SessionStart, Stop, SessionEnd, append, context-run and bounded backlog
    recovery durations are measured against existing limits without raising
    them.
11. Tenant, acting principal and governed scope cannot be client supplied;
    cross-project and cross-tenant identifiers do not disclose existence; and
    the setup performs no direct governed database mutation or PDP bypass.

## Boundary

A host killed before any lifecycle hook writes the in-flight turn to the spool
can lose that tail. This feature does not claim otherwise. Its guarantee begins
when a hook has durably written an event.
