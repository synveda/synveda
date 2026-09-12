# ADPT-9: GitHub Copilot CLI adapter

## Problem and evidence

The installed Copilot CLI 1.0.83 exposes MCP, Skills and lifecycle hooks but has
no qualified Synveda lifecycle. The starting checkout is
`668b4f21e7f3dc10c902c42f396c28c601f47a50`. Native local configuration and Skill
discovery are available without model execution; full context consumption,
observations, task recovery and audit evidence require a real client run.

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
server-owned Sessions and local binding state intact. Full native qualification
is blocked: the CLI refused the initial 0.5-credit cap (minimum 30), and automatic
approval review rejected a run allowing 30 credits without explicit cost
authorisation. No model execution or authentic lifecycle capture succeeded.
All 121 adapter tests, including eight new Copilot contract cases, pass on
macOS arm64 Node 24.18.0 and offline Docker Linux arm64 Node 22.23.2, zero skips.
Formatting, registry/evidence, documentation, backlog, ADR and dependency gates
pass. The existing archive passes its eight extracted Codex regression tests.
Rust is unchanged; strict Clippy is not applicable to this increment.
Full CI, database and live Compose/native qualification have not been rerun.
The initial sandbox loopback denial, one corrected log-assertion failure and
the first Docker run's non-executable temporary directory are not passes.
Next action: request approval for one bounded synthetic native prompt with the
minimum 30-credit ceiling. That first
probe establishes the actual seam; it is not full conformance by itself.

## Dependencies

Existing ADPT-1/2/4, CPR-12/23/39, ADR-0098/0106 and the Docker reference
deployment. Copilot native authentication and explicit service-cost allowance
are external prerequisites for the live evidence, not for offline tests.
