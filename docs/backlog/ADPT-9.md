# ADPT-9: GitHub Copilot CLI adapter

## Problem and evidence

The installed Copilot CLI 1.0.83 exposes MCP, Skills and lifecycle hooks but has
no qualified Synveda lifecycle. The starting checkout is
`668b4f21e7f3dc10c902c42f396c28c601f47a50`. Native local configuration and Skill
discovery are available without model execution; full context consumption,
observations, task recovery and audit evidence require a real client run.
The approved 2026-09-13 probe at `ae3d860` captured six authentic hook frames and
native synthetic Skill activation. Its marker assertion failed after a
conflicting Skill was invoked; the process's zero exit is not a context pass.
The captured start payload replays through the existing adapter unchanged.
The separately approved corrected retry at `e8282c9` consumed an unpredictable
hook-only marker with `source: "resume"` and the same native Session UUID.
Both captured invocations replay through the adapter without production changes.

## Scope

1. Reuse the shared authenticated Session runtime for bounded start/resume
   context, explicit task binding, and documented MCP/approved Skill setup.
   Add authored contract tests and an experimental registry entry.
2. After native-run cost approval, capture authentic hooks/MCP/transcript frames,
   add only demonstrated observation/compaction translation, and exercise one
   task with both SDKs against the Docker/Keycloak public edge. Package the
   qualified runtime using the existing archive path.

## Non-goals

No generic plugin system, orchestration, new retrieval/policy implementation,
external MCP management requirement, guessed transcript parser, or Copilot
cloud-agent/VS Code support claim. No release or verified label from mocks.

## Architecture seam

[ADR-0107](../adr/adr-0107-copilot-context-adapter.md) extends ADR-0106's narrow
Session-runtime export with a closed Copilot client identity. Ordinary public
API enforcement and explicit application task ownership remain authoritative.
The existing Skill root override and native MCP configuration need no new
installer or protocol dependency.

## Acceptance criteria

Start and resume return allowed context and the same Synveda Session ID;
runtime/transport exit does not end it. The named real client loads an approved
Skill, recalls context, submits a proposal with the Python/TypeScript task,
denies a foreign workspace and correlates the actual audit trace. Durable
observations, retries, Capture, explicit end and later Knowledge reuse must
meet each applicable ADR-0098 criterion before promotion to verified.

## Required tests

Authored start/resume HTTP contracts, namespace/client and gateway isolation,
opt-out, malformed/oversized input, unavailable gateway and denied credentials;
Claude/Codex regression suites; Node 22 Docker execution. Native lifecycle and
MCP frames must be digest-pinned separately from authored fixtures. Live Docker
acceptance must use ordinary Keycloak identities and public APIs.

## Rollout and rollback

Source-build experiment first; remove its project hook to roll back. Keep
server-owned Sessions and local binding state intact. CLI 1.0.83's first probe
failed its hook-marker assertion after invoking a conflicting synthetic Skill.
The separately approved retry at 2026-09-13T07:28:35Z returned the exact fresh
hook-only marker in 5.03 seconds, invoked no tools and resumed the same native
Session after the earlier runtime ended. Its saved limit remained 30 credits;
the native counter increased from one to two cumulative premium requests.
An invalid wildcard denial flag was rejected before the retry ran, with no new
hook or transcript events; documented shell/write/URL denial rules replaced it.
The earlier failure and successful resume are separately digest-pinned.
Authenticated native context/observation delivery and the shared SDK lifecycle
remain unverified; synthetic hook consumption is not that workflow.

The observation increment starts at `d22f8fd305a989544d237a3c87d68ddacdfa2af3`.
The amended ADR-0107 records the decision before implementation. Captured
user/assistant text and actual tool execution pairs now use the existing event
mapper and spool. `agentStop` records locally; `sessionEnd` shares a two-second
budget across credentials and delivery and retains the application task.
Start/resume reads the saved transcript and retries stable event IDs. The
Codex bounded file reader is shared through the existing runtime export, with
no new runtime dependency or protocol framework.

All 136 adapter tests pass on macOS arm64 Node 24.18.0 and pinned offline Docker
Linux arm64 Node 22.23.2, zero skips: 105 Claude, eight Codex and 23 Copilot.
The existing Codex archive passes eight extracted regression tests on both
runtimes. Original private native transcripts also replay offline into the
same four initial and six total observations. The documented
`acceptance-interop` Compose smoke passes against the running Keycloak stack.
This establishes stack health and deterministic adapter behaviour, not native
Synveda authentication or full lifecycle conformance. No new paid model prompt
was run. Earlier native evidence and its failed first marker remain preserved.

Formatting, strict TypeScript compilation, dependency direction, registry,
documentation, backlog and ADR gates pass. Rust is unchanged; strict Clippy is
not applicable. Full CI, database, full Compose acceptance and complete native
qualification were not rerun. Copilot packaging remains deferred until qualification.

Next action: qualify the real hook and MCP client against the ordinary
Docker/Keycloak public edge using one shared Python/TypeScript Session. Reuse
the approved Skill installer and verify exact binding/version attribution;
exercise proposal submission, foreign-workspace denial, persisted audit
correlation, retry, explicit Capture/end and later Knowledge reuse. Current
observation evidence is replay only; compaction, non-text/unknown failure-result
formats and authentic governed Skill usage remain unqualified. Native
`skill.invoked` has no Synveda binding identity and is not promoted into a typed
usage event. The two prior synthetic-prompt allowances are consumed; prepare
the complete governed scenario before seeking any further paid-run allowance.

## Dependencies

Existing ADPT-1/2/4, CPR-12/23/39, ADR-0098/0106 and the Docker reference
deployment. Copilot native authentication and explicit service-cost allowance
are external prerequisites for the live evidence, not for offline tests.
