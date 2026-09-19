# ADR-0107: Copilot context uses the existing authenticated Session runtime

- **Status**: Accepted; amended 2026-09-19
- **Date**: 2026-09-12
- **Feature(s)**: ADPT-9
- **Deciders**: User-directed Copilot adapter continuation

## Context

Copilot CLI 1.0.83 is installed. Its native `mcp add` command writes a local
Synveda server entry, and `skill list` discovers a synthetic project Skill.
The official [hook reference](https://docs.github.com/en/copilot/reference/hooks-reference)
documents a start/resume context seam. These observations do not establish a
live authenticated lifecycle. The initial model probe rejected a 0.5-credit
ceiling as below the CLI's minimum of 30. Automatic approval review then
rejected the 30-credit run because that service-cost allowance was not explicit.

ADR-0098 permits experimental contract targeting but requires authentic frames
and persisted/audited outcomes for verification. ADR-0106 already provides the
public-API runtime, origin-bound credentials and stable task identity we need.

## Decision

Implement the documented camelCase `sessionStart` boundary
using the existing Session runtime and Copilot's top-level `additionalContext`
output. Extend its closed client identity with `copilot-cli`; namespace native
IDs as `copilot-cli:<native-id>`. Preserve the binding across other task starts,
resume and transport/process exit. The application owner explicitly requests
Capture and ends its Synveda Session.

Bound stdin and execution, validate the fields used at the entry point, and
honour project opt-out. Translate additional seams only from authentic frames;
a vendor SDK event type alone is not evidence of the on-disk transcript format.
Do not infer post-compaction reinjection from `preCompact`, whose documented
output is ignored. Authored tests must remain labelled contract tests.

The 2026-09-13 captured CLI 1.0.83 frames establish `agentStop.transcriptPath`,
native event UUIDs, user/assistant text, a tool execution pair and same-UUID
resume after `sessionEnd`. Translate those transcript shapes into the existing
event mapper and spool. `agentStop` records locally before any network work;
`sessionEnd` flushes within a two-second credential/delivery budget and retains
the binding. Only Claude's existing runtime owns automatic task closure.
Start/resume reuses the saved transcript path and retries pending events.

Share the existing Codex bounded file reader through the narrow runtime export:
regular files only, no final symlink, at most 8 MiB and 20,000 JSONL records.
Require the native transcript header to match the hook's Session UUID. Reject
malformed/partial records, invalid identities and unknown content/result shapes
without advancing the cursor. Use native execution events for tool calls, not
assistant tool intentions or uncorrelated hook callbacks. Whitelist content
fields; omit system instructions, reasoning, encrypted data and diagnostics.
The native `skill.invoked` record proves host activation but does not establish
a governed Synveda binding/version, so it emits no typed Skill-usage claim.

The authenticated CLI 1.0.83 run on 2026-09-13 captured a denied MCP completion
with `success: false`, no `result`, and `error: { code: "failure", message }`.
Accept that demonstrated text-error shape as an ordinary failed tool result,
correlated by the existing native call ID. Keep its native event ID for replay;
do not copy other error fields. A successful completion with an error, both
result/error representations, unknown error codes or non-text error messages
remain held. This closes the observed whole-transcript hold after an expected
workspace denial without changing error semantics or adding a fallback parser.

Reuse native `copilot mcp add` and Synveda's existing Skill `--root` override.
Keep the server unbound for multiple tasks and pass the injected Synveda
`session_id` explicitly. MCP uses the maintained `rmcp` server and public API;
Python and TypeScript applications reuse that same Session. No external MCP
server manager is required. Following the clean native qualification, package
the existing compiled runtime through ADR-0065 amendment 10's archive and
installer contract. Client configuration remains an explicit setup step.

## Options considered

1. A bounded start-context adapter over the existing runtime — selected; useful
   authenticated behaviour can be tested without guessing transcript layouts.
2. Copy the Codex transcript parser or infer events from SDK types — rejected;
   neither proves the installed Copilot contract or event ownership.
3. Rebuild authentication, Skill distribution or MCP configuration — rejected;
   existing components already supply those workflows.
4. Claim parity from a configuration file — rejected by ADR-0098.

## Consequences

- Positive: one explicit application identity, credential boundary and context
  path across harness and SDK calls, with no new runtime dependency.
- Accepted trade-off: verification is limited to CLI 1.0.83 / gpt-5.6-luna,
  macOS arm64 and the documented source-build/Docker-Keycloak setup. The clean
  2026-09-19 native start/resume and shared SDK workflow passed with 18 unique
  events, 16 Capture candidates and audit verification through sequence 1179.
  The last assistant event required a subsequent native reopen/exit to become
  deliverable. Earlier missing-hook and parser failures remain pinned.
  Native outage/compaction and execution from a published installation remain
  unqualified. Local archive replay has its own acceptance gate. The approved
  Skill's exact binding/file and activation path are evidenced separately;
  native activation alone still emits no typed Skill-usage event.
- Reversal trigger: authentic frames demonstrate another necessary seam; add
  only that translation and its replay, then run ADR-0098 qualification.

## Compliance notes

No Rust, SQL, policy, RLS, retrieval, registry governance, VedaFlow or audit
implementation changes. All product reads and writes remain authenticated
public-API operations. Local context output carries task identity, never
authority. Native host approval and Synveda PDP checks remain separate.
