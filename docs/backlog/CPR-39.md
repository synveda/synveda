# CPR-39: Second verified client

## Problem and evidence

ADR-0098's conformance model is already implemented: `adapters/registry.json` is authoritative, CI checks it, and generated onboarding/support views distinguish configured, captured, experimental, unsupported, and verified evidence. Claude Code 2.1.241 is the only verified lifecycle. Claude Desktop 1.25927.0 and Zed 1.13.2 are captured tool protocols only. Cursor is experimental because its Hooks v1 contract appears sufficient, but this environment has no Cursor executable, authenticated account, or authentic frame; VS Code 1.133 Preview lacks `SessionEnd` and documents that `Stop` is not session inactivity. Codex CLI 0.152.0 now has captured, replayed MCP 2025-06-18 and native start/tool/Stop/exit/resume frames, with a minimal Session runtime translator; a second verified lifecycle remains open.

## Scope

The 2026-09-12 [interoperability plan](../INTEROPERABILITY_PLAN.md) selects
installed Codex CLI 0.152.0 for the next qualification attempt. Its current
vendor documentation provides Session IDs and SessionEnd. Actual MCP and ten
native lifecycle frames are digest-pinned. Normal trusted-hook review yielded
startup, prompt, tool pair, Stop, exit and resume of the same native identity.
The minimal translator now reuses the existing Session/spool runtime and keeps
the task open at runtime exit; deterministic replay passes. No complete live
Synveda lifecycle is claimed. Copilot CLI and Pi remain
separate later candidates, not inferred support from VS Code or MCP alone.

- Acquire one named non-Claude-Code client/version with a credible complete lifecycle; installed Codex CLI is the current candidate and must satisfy the same registry criteria.
- Capture authentic versioned client frames, digest-pin them, implement the adapter without inventing missing events, and retain exact limitations.
- Run the real client through session creation, event delivery, context request, capture, session end, retry/idempotency, available Skill/Tool seams, cross-session Knowledge reuse, and persisted audited outcomes over public APIs.
- Update the authoritative registry and regenerate projections only from that evidence; keep unavailable criteria `not_applicable`, failed, or incomplete with reasons.
- Keep the existing conformance checker, generated support matrix/onboarding, and Claude/captured evidence current while completing the live run.

## Non-goals

- Treating MCP configuration, authored/mock frames, protocol replay, vendor documentation, or installed-but-unauthenticated software as live verification.
- Weakening or deleting a conformance criterion to upgrade a client.
- Relabelling Claude Desktop/Zed tool-only captures or VS Code's incomplete lifecycle as verified.
- Claiming cloud and local editions share lifecycle evidence without testing the exact edition/version.

## Architecture seam

The client-specific adapter is a public-API client and declares host-versus-MCP write ownership. `adapters/registry.json` remains the sole support authority; generated documentation and console onboarding are projections. Authentic fixtures are immutable digest-pinned inputs to replay, while the live result records the named binary version, timestamp, environment, and persisted server/audit outcomes.

## Acceptance criteria

- A second named real client version completes every applicable ADR-0098 criterion in one authentic lifecycle and is recorded as `verified` with criterion-level evidence.
- Session creation/events/context/capture/end and retry/idempotency produce the expected persisted, tenant-isolated, hash-chained outcomes without duplicate writes.
- Available Skill/Tool seams are tested honestly; a missing trustworthy callback remains `not_applicable` with a reason and is never inferred from model text.
- Cross-session authorized Knowledge reuse is demonstrated through public APIs, with deny/revoke and outage/recovery evidence.
- Registry validation, generated support matrix/onboarding, authentic-fixture digests, deterministic replay, and the runnable live-result demo agree on client/version/status.

## Required tests

- Registry forgery/drift, support-level invariant, generated projection, and fixture-digest checks in CI.
- Authentic-frame replay for every exposed lifecycle boundary, malformed/reordered/duplicate frames, and write-owner configuration.
- Database-backed adapter lifecycle with ordinary tenant transactions, Cedar allow/deny/revoke, forced RLS, audit, and cross-tenant isolation.
- Installed authenticated live-client run pinned to the exact binary/version, plus outage, restart, retry, capture, and cross-session probes.
- Negative test proving a configured/captured/incomplete client cannot be promoted to `verified`.

## Rollout and rollback

Move the candidate only through experimental/configured to captured and then verified as evidence accumulates; generated views must never lead the registry. Canary the adapter for the exact tested version. On vendor drift or failed revalidation, lower the support level and state the failing criterion while retaining prior dated evidence and fixtures.

## Dependencies

The installed Codex 0.152.0 binary and native model authentication work with
GPT-5.5/low after normal project/hook trust review. Fresh canonical Keycloak
acceptance passed following the completed hosts handoff. Native hooks consumed
allowed context and the approved Skill; authenticated MCP recall worked after
allowing the isolated XDG profile directory into the native MCP environment.
A resumed native ID retained its Synveda Session. Authentic failed/successful
text-only MCP result frames are now pinned and translated, including namespaces
and native error status. Exit work uses one two-second credential/delivery
budget beneath the observed three-second host cap; hung-credential and stalled-
append tests prove retained events and retry.
The rebuilt public-proxy workflow passed on one native Session shared by both
SDKs: 14 unique persisted events, exact Skill bytes, pending/idempotent proposals,
foreign-workspace denial and correlated content-free audit. Capture completed
with 12 eligible events and 12 candidates, explicit task-owner end succeeded,
and another application Session reused approved Knowledge. Audit verification
passed through sequence 427; lifecycle end event 422 was confirmed separately
because that build's Session audit filter omitted nested lifecycle identities.
`adapters/codex/fixtures/keycloak-qualification.json` retains the result and source
digests; the registry records passed criteria but stays `captured`.

The shared Session audit filter correction now passes 42 focused exact-role DB
tests and live Keycloak queries after canonical Compose up/smoke: all 23 saved
events, including open/end, return across 12 pages with unchanged hashes. The
earlier captured receipt remains unchanged. Next: qualify native outage/
recovery and compaction/reinjection from authentic frames. Non-text MCP results
and installation packaging remain explicit limits. Preserve the conformance gate;
Copilot CLI and Pi executables were unavailable, and VS Code is not a substitute.

**Continuation checkpoint (2026-09-12, source `b4f4856`)**

Retained-data canonical Compose smoke, all six Codex adapter tests and eight
conformance/fixture checks pass. No native outage or compaction run started:
automatic execution approval rejected sending the isolated fixture's synthetic
approved Skill, retrieved context and test transcript to OpenAI GPT-5.5 without
specific owner consent. That approval is pending; credentials remain local.
This is an execution-approval blocker, not a failed Synveda lifecycle test.

After approval, start a new synthetic native task with the existing trusted
hooks and ordinary Keycloak profile. Pause only the owned gateway during one
resumed turn, restore it even if the probe fails, then prove durable retry and
unique persisted events on the same Synveda Session. Capture authentic native
compaction frames before changing the Codex event filter; reuse the existing
PreCompact persistence and SessionStart context composition. The filter
currently accepts startup/resume, Stop and runtime exit only. Do not promote
support or substitute authored frames for the blocked live evidence.
