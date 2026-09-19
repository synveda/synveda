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
Neither synthetic probe establishes authenticated context, observation delivery
or the shared SDK lifecycle.

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

### Authenticated preflight — 2026-09-13

At `24cd02d009f26f87d7a7c6b35fdf01633e1a682e`, the governed scenario passed
through the existing `acceptance-interop` Docker public edge. Both expired demo
profiles were renewed through ordinary browser OIDC/PKCE S256. Grants, policy,
retrieval, the Skill registry, adapters and SDK implementations are unchanged.
The content-free [result](../../adapters/copilot-cli/fixtures/public-edge-replay.json)
records source and transcript digests, exact resource IDs and audit traces.

Authored invocations of captured hook schemas replayed six original text/tool
events. Start returned allowed context; Stop persisted locally before delivery;
exit and resume retained one active task without duplicate events. The resumed
captured prompt concerns the earlier synthetic marker, so its empty Knowledge
selection is not recorded as an allowed-context resume pass.

The existing installer materialised the approved enabled Skill binding into
the isolated project's native Skill root. Receipt binding/version/bundle and
file identity match the available API entry; installed bytes match the immutable
file response. Copilot CLI 1.0.83 listed it as enabled. This proves governed
installation and native discovery, not native model activation.

Both unchanged SDK examples retrieved allowed context and Skill bytes, submitted
idempotent pending proposals, denied the existing foreign Session and correlated
actual response traces with content-free audit. The existing stdio test client
exercised `rmcp` through two connections: explicit same-task recall, foreign
denial, missing identity refusal and host-owned write refusal all passed.
This is an authenticated MCP replay client, not native Copilot MCP evidence.

The shared task contains eight unique events: six captured observations plus two
SDK `skill.loaded` events. Explicit Capture completed with six candidates;
task-owner end, later approved Knowledge reuse and audit verification through
sequence 938 passed. No approved Skill usage is inferred from the old synthetic
`skill.invoked` record. A local driver initially named a nonexistent Node binary
and stopped before opening a Session; using the installed executable passed.
The documented Compose smoke passed again with the same project/profile/pool.
Full CI, database tests and full Compose acceptance were not rerun. Rust and
adapter code are unchanged, so strict Clippy and the prior adapter suites were
not rerun for this evidence/documentation increment.

### Native denial correction and interrupted qualification — 2026-09-19

The user approved the prepared pair. Its first invocation ran on 2026-09-13 at
`d71641e`, using Copilot CLI 1.0.83 / gpt-5.6-luna. It consumed authenticated
hook context, loaded the exact approved Skill and used the injected Synveda
Session for successful MCP recall; the known foreign Session was denied.
The installed Skill hash was unchanged before/after activation. The invocation
took 10.17 seconds and reported one premium request under the saved 30-credit
ceiling. These counters are not a monetary estimate.

Observation delivery failed: the denied tool completion carried
`success: false`, no `result`, and `error: { code: "failure", message }`.
The parser rejected it as `tool_result_shape_unknown` and held all eight
observations without advancing the cursor. The amended ADR-0107 precedes the
narrow correction: map only this demonstrated text-error shape into the
existing failed-tool event. Unknown codes, non-text messages, success/error
contradictions and both result/error representations remain held.

The [native projection](../../adapters/copilot-cli/fixtures/governed-transcript.jsonl)
and [partial result](../../adapters/copilot-cli/fixtures/governed-probe.json)
preserve the original failure. Regression tests prove all eight observations,
out-of-order tool correlation, local Stop persistence, one delivery despite
duplicate exit hooks and no implicit task end. No Rust, public API, policy,
retrieval, Skill registry or orchestration changes are required.

All 140 adapter tests pass with zero skips on macOS arm64 Node 24.18.0 and
offline pinned Docker Linux arm64 Node 22.23.2: 105 Claude, eight Codex and 27
Copilot. Strict TypeScript compilation, formatting, dependency direction,
registry/digests, documentation, backlog and ADR gates pass. Full CI, fresh
database tests, full Compose acceptance and unchanged package checks were not
rerun. Strict Rust Clippy is not applicable because no Rust changed.

On the 2026-09-19 retry, the temporary raw captures, native saved Session,
credentials and spool were absent. The retained projected transcript still
replays, but is not a native Session backup. The reference Docker stack was
stopped; canonical `make compose-up` failed before service startup with
`hosts ownership state was refused`. Read-only inspection found matching
hosts content but a changed `dev` field in the root-owned backup witness.
`make compose-hosts-status` also refused; the plan still names the existing
`acceptance-interop` mapping. No witness or preflight gate was changed.

Next action: resolve the hosts-backup device-witness drift under CPR-45, then
prepare a replacement governed start/resume qualification with durable private
state. The original temporary Session cannot be resumed from the retained
projection. Only one prompt of the newly approved pair ran; no paid prompt ran
on retry. Keep new-Session cost allowance explicit and do not silently reuse a
lost Session's budget. Both SDKs, live delivery/audit after this fix, Capture/end
and reuse on the native task remain unverified. Native outage/compaction,
other error/result shapes and packaging remain open. Keep support experimental.

## Dependencies

Existing ADPT-1/2/4, CPR-12/23/39, ADR-0098/0106 and the Docker reference
deployment. Copilot native authentication and explicit service-cost allowance
are external prerequisites for the live evidence, not for offline tests.
