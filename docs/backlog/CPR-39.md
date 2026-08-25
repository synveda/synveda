---
title: "CPR-39: Adapter conformance and second verified client"
labels:
  - epic:CPR
  - phase:5
size: L
---

# CPR-39: Adapter conformance and second verified client

**Epic:** CPR — Context-platform redesign · **Phase:** 5 · **Size:** L

## Description

Make client support an evidence-bearing data model rather than prose scattered
between installers, fixtures and the console. Verify a second real client only
when a named version completes the full public-API lifecycle.

## Acceptance criteria

- One data registry defines `configured`, `captured`, `verified`,
  `experimental` and `unsupported`; tested versions, configuration generator,
  lifecycle mechanism/events, limitations, authentic digest-pinned fixtures
  and criterion-level conformance evidence are explicit.
- `verified` is mechanically refused unless one named real version passed
  session creation, events, context delivery, capture, end, retry/idempotency,
  available Skill/Tool seams, cross-session Knowledge reuse and persisted,
  hash-chained audit outcomes through public APIs.
- CLI MCP configuration, generated console onboarding and the public support
  matrix project the same registry. A connection recipe or authored fixture
  cannot upgrade a support level. Stale vendor conditionals and fixtures that
  borrow an external client's name are deleted.
- Authentic Claude Code, Claude Desktop and Zed evidence remains labelled and
  digest-pinned. Cursor is run live and becomes verified if the actual client
  passes; otherwise the exact unavailable or insufficient criterion is
  retained and no replay is described as live. VS Code is the fallback only if
  its actual lifecycle passes the same gate.
- Focused registry forgery/drift, CLI config, MCP corpus, adapter and console
  tests pass; the gate runs in `make ci`; a runnable demo prints the generated
  matrix and the external live-client result.

## Current evidence and blocker

Implemented 2026-08-25 from `12e393a` under ADR-0098. The single
`adapters/registry.json` authority, generated support matrix/onboarding,
configuration projection and CI truthfulness gate are present. Claude Code
2.1.241 remains the only `verified` lifecycle, based on CPR-14's genuine live
run plus deterministic outage evidence. Claude Desktop 1.25927.0 and Zed
1.13.2 remain `captured`, not lifecycle-verified.

The official Cursor Hooks v1 local-IDE contract now exposes the required start,
turn, tool, compact, stop and end boundaries, so Cursor is a viable
`experimental` target rather than structurally unsupported. No Cursor
executable or authenticated Cursor client is available in this environment and
there is no authentic Cursor frame to replay. Installed VS Code 1.133.0 is not
an honest substitute: its Preview hook reference has no SessionEnd event and
states that Stop does not mean the session is inactive; the local profile also
has no authenticated agent. The second-live-client criterion therefore remains
externally blocked and this feature stays open. No replay or generated config
is represented as live verification.
