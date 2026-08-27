# CPR-39: Second verified client

## Problem and evidence

ADR-0098's conformance model is already implemented: `adapters/registry.json` is authoritative, CI checks it, and generated onboarding/support views distinguish configured, captured, experimental, unsupported, and verified evidence. Claude Code 2.1.241 is the only verified lifecycle. Claude Desktop 1.25927.0 and Zed 1.13.2 are captured tool protocols only. Cursor is experimental because its Hooks v1 contract appears sufficient, but this environment has no Cursor executable, authenticated account, or authentic frame; VS Code 1.133 Preview lacks `SessionEnd` and documents that `Stop` is not session inactivity. A second verified client remains externally blocked.

## Scope

- Acquire one named non-Claude-Code client/version with a credible complete lifecycle; Cursor local IDE is the current candidate, while any substitute must satisfy the same registry criteria.
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

Completion requires externally supplied access to a proprietary candidate executable, authenticated account/credential, and a stable local lifecycle contract. The owner must choose and provision that client and approve the tested edition/version. If Cursor remains unavailable or its live hooks fail, another client must independently meet the full gate; VS Code's currently documented lifecycle is not an acceptable fallback.
