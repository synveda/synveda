# ADR-0107: Copilot context uses the existing authenticated Session runtime

- **Status**: Accepted
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

Implement only the documented camelCase `sessionStart` boundary in this batch,
using the existing Session runtime and Copilot's top-level `additionalContext`
output. Extend its closed client identity with `copilot-cli`; namespace native
IDs as `copilot-cli:<native-id>`. Preserve the binding across other task starts,
resume and transport/process exit. The application owner explicitly requests
Capture and ends its Synveda Session.

Bound stdin and execution, validate the fields used at the entry point, and
honour project opt-out. Disable observation in this entry point and supply no
transcript reader: a vendor SDK event type is not evidence of the on-disk
transcript format. Do not infer post-compaction reinjection from `preCompact`,
whose documented output is ignored. Translate other seams only after capturing
authentic frames. Authored tests must remain labelled contract tests.

Reuse native `copilot mcp add` and Synveda's existing Skill `--root` override.
Keep the server unbound for multiple tasks and pass the injected Synveda
`session_id` explicitly. MCP uses the maintained `rmcp` server and public API;
Python and TypeScript applications reuse that same Session. No external MCP
server manager is required. This source-build experiment is not added to the
release archive before its native qualification.

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
- Accepted trade-off: experimental, source-build support. Automatic observation,
  native context consumption, compaction, Skill activation and the full audit
  workflow remain unqualified until an authorised native run completes.
- Reversal trigger: authentic frames demonstrate another necessary seam; add
  only that translation and its replay, then run ADR-0098 qualification.

## Compliance notes

No Rust, SQL, policy, RLS, retrieval, registry governance, VedaFlow or audit
implementation changes. All product reads and writes remain authenticated
public-API operations. Local context output carries task identity, never
authority. Native host approval and Synveda PDP checks remain separate.
