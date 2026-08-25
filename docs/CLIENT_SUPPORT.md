# Client adapter support

This file is generated from `adapters/registry.json` by `make check-adapters`. Do not edit it by hand.

A connection recipe is not a support claim. `captured` means authentic frames replay; only `verified` means a named real client version completed the full public-API lifecycle and left persisted, audited evidence.

| Client | Level | Tested versions | Lifecycle | Principal limit |
| --- | --- | --- | --- | --- |
| Claude Code | `verified` | 2.1.220, 2.1.241 | Claude Code plugin hooks plus the plugin-owned MCP launch | Stop and PreCompact cross only the atomic local-spool boundary synchronously; SessionEnd or the next SessionStart delivers them. |
| Cursor | `experimental` | none | Cursor Hooks v1 plus MCP | No Cursor executable or authenticated client was available on 2026-08-25. |
| Visual Studio Code | `configured` | none | VS Code agent hooks Preview plus MCP | The documented Preview contract has no SessionEnd event; Stop explicitly does not mean the session became inactive. |
| Claude Desktop | `captured` | 1.25927.0 | MCP tool calls only | Authentic discovery and tool-call frames are replayed, but MCP alone does not prove session capture or end semantics. |
| Zed | `captured` | 1.13.2 | MCP tool calls only | Authentic non-Anthropic tool frames are replayed, but no session lifecycle/capture contract is available. |
| Windsurf | `configured` | none | MCP configuration only | Documented config shape only; no authentic exchange or lifecycle run is claimed. |
| Continue | `configured` | none | MCP configuration only | Documented legacy JSON config shape only; use --print for YAML-only installations. No authentic run is claimed. |

## Evidence

### Claude Code — `verified`

Contract: plugin 0.2.0 / captured Claude Code 2.1.241 hook schema. Evidence level: `live-client`.

Authentic fixtures:

- `adapters/claude-code/fixtures/manifest.json` — captured-real-client-manifest, SHA-256 `76a5b6b9d5b118f1e762f59edd400458c3cdfb1d89a0a36d8759a7b035331b57`

Conformance:

- `session_creation`: passed — `crates/synveda-gateway/tests/claude_lifecycle.rs`
- `event_delivery`: passed — `crates/synveda-gateway/tests/claude_lifecycle.rs`, `adapters/claude-code/fixtures/manifest.json`
- `context_request_delivery`: passed — `crates/synveda-gateway/tests/claude_lifecycle.rs`
- `capture`: passed — `crates/synveda-gateway/tests/claude_lifecycle.rs`
- `session_end`: passed — `crates/synveda-gateway/tests/claude_lifecycle.rs`, `adapters/claude-code/fixtures/hooks/session-end-headless.json`
- `retry_idempotency`: passed — `crates/synveda-gateway/tests/claude_lifecycle.rs`, `adapters/claude-code/src/spool.test.mts`
- `skill_advertisement_activation`: not_applicable — `adapters/claude-code/src/skills.test.mts`
- `tool_configuration`: passed — `adapters/claude-code/.mcp.json`, `crates/synveda-gateway/tests/claude_lifecycle.rs`
- `cross_session_knowledge_reuse`: passed — `crates/synveda-gateway/tests/claude_lifecycle.rs`
- `persisted_audited_outcomes`: passed — `crates/synveda-gateway/tests/claude_lifecycle.rs`

Known limits:

- Stop and PreCompact cross only the atomic local-spool boundary synchronously; SessionEnd or the next SessionStart delivers them.
- A host killed before any hook cannot be observed.
- Skill execution evidence remains host-observed at the sync/advertisement seam; a model statement alone never counts.

### Cursor — `experimental`

Contract: Hooks schema version 1 (official contract inspected 2026-08-25). Evidence level: `not-run`.

Authentic fixtures:

- None. Configuration or an inspected vendor contract is not a captured client frame.

Conformance:

- `session_creation`: not_run
- `event_delivery`: not_run
- `context_request_delivery`: not_run
- `capture`: not_run
- `session_end`: not_run
- `retry_idempotency`: not_run
- `skill_advertisement_activation`: not_run
- `tool_configuration`: not_run
- `cross_session_knowledge_reuse`: not_run
- `persisted_audited_outcomes`: not_run

Known limits:

- No Cursor executable or authenticated client was available on 2026-08-25.
- No authentic Cursor lifecycle frame is committed; generated MCP configuration is not verification.
- Cloud agents omit sessionStart and sessionEnd and therefore do not satisfy this lifecycle.

### Visual Studio Code — `configured`

Contract: VS Code 1.133 Preview hook reference (inspected 2026-08-25). Evidence level: `not-run`.

Authentic fixtures:

- None. Configuration or an inspected vendor contract is not a captured client frame.

Conformance:

- `session_creation`: not_run
- `event_delivery`: not_run
- `context_request_delivery`: not_run
- `capture`: not_run
- `session_end`: not_run
- `retry_idempotency`: not_run
- `skill_advertisement_activation`: not_run
- `tool_configuration`: not_run
- `cross_session_knowledge_reuse`: not_run
- `persisted_audited_outcomes`: not_run

Known limits:

- The documented Preview contract has no SessionEnd event; Stop explicitly does not mean the session became inactive.
- VS Code 1.133.0 was installed locally, but no authenticated agent profile or real run was available.
- MCP configuration alone does not provide reliable capture lifecycle semantics.

### Claude Desktop — `captured`

Contract: MCP 2025-11-25 captured. Evidence level: `captured-protocol`.

Authentic fixtures:

- `crates/synveda-cli/fixtures/mcp/claude-desktop-probe.json` — captured-client-frames, SHA-256 `08defbfbddbe99a48271ffa9ca203e658782e431d601ae97478b532981e5a022`
- `crates/synveda-cli/fixtures/mcp/claude-desktop-agent.json` — captured-client-frames, SHA-256 `8ac8fac8d07913a562f440933ae8cbd77ebbacb26e70f851304ab0498e6389d9`

Conformance:

- `session_creation`: not_run
- `event_delivery`: not_run
- `context_request_delivery`: not_run
- `capture`: not_run
- `session_end`: not_run
- `retry_idempotency`: not_run
- `skill_advertisement_activation`: not_run
- `tool_configuration`: not_run
- `cross_session_knowledge_reuse`: not_run
- `persisted_audited_outcomes`: not_run

Known limits:

- Authentic discovery and tool-call frames are replayed, but MCP alone does not prove session capture or end semantics.

### Zed — `captured`

Contract: MCP 2025-11-25 captured. Evidence level: `captured-protocol`.

Authentic fixtures:

- `crates/synveda-cli/fixtures/mcp/zed.json` — captured-client-frames, SHA-256 `8248caaa969cdae1606ba3c6c0ba39ee30b89702f29b49c901a3181e7d81cb3b`

Conformance:

- `session_creation`: not_run
- `event_delivery`: not_run
- `context_request_delivery`: not_run
- `capture`: not_run
- `session_end`: not_run
- `retry_idempotency`: not_run
- `skill_advertisement_activation`: not_run
- `tool_configuration`: not_run
- `cross_session_knowledge_reuse`: not_run
- `persisted_audited_outcomes`: not_run

Known limits:

- Authentic non-Anthropic tool frames are replayed, but no session lifecycle/capture contract is available.

### Windsurf — `configured`

Contract: not established. Evidence level: `not-run`.

Authentic fixtures:

- None. Configuration or an inspected vendor contract is not a captured client frame.

Conformance:

- `session_creation`: not_run
- `event_delivery`: not_run
- `context_request_delivery`: not_run
- `capture`: not_run
- `session_end`: not_run
- `retry_idempotency`: not_run
- `skill_advertisement_activation`: not_run
- `tool_configuration`: not_run
- `cross_session_knowledge_reuse`: not_run
- `persisted_audited_outcomes`: not_run

Known limits:

- Documented config shape only; no authentic exchange or lifecycle run is claimed.

### Continue — `configured`

Contract: not established. Evidence level: `not-run`.

Authentic fixtures:

- None. Configuration or an inspected vendor contract is not a captured client frame.

Conformance:

- `session_creation`: not_run
- `event_delivery`: not_run
- `context_request_delivery`: not_run
- `capture`: not_run
- `session_end`: not_run
- `retry_idempotency`: not_run
- `skill_advertisement_activation`: not_run
- `tool_configuration`: not_run
- `cross_session_knowledge_reuse`: not_run
- `persisted_audited_outcomes`: not_run

Known limits:

- Documented legacy JSON config shape only; use --print for YAML-only installations. No authentic run is claimed.
