# Client adapter support

This file is generated from `adapters/registry.json` by `make check-adapters`. Do not edit it by hand.

A connection recipe is not a support claim. `captured` means authentic frames replay; only `verified` means a named real client version completed the full public-API lifecycle and left persisted, audited evidence.

| Client | Level | Tested versions | Lifecycle | Principal limit |
| --- | --- | --- | --- | --- |
| Claude Code | `verified` | 2.1.220, 2.1.241 | Claude Code plugin hooks plus the plugin-owned MCP launch | Stop and PreCompact cross only the atomic local-spool boundary synchronously; SessionEnd or the next SessionStart delivers them. |
| Cursor | `experimental` | none | Cursor Hooks v1 plus MCP | No Cursor executable or authenticated client was available on 2026-08-25. |
| Visual Studio Code | `configured` | none | VS Code agent hooks Preview plus MCP | The documented Preview contract has no SessionEnd event; Stop explicitly does not mean the session became inactive. |
| Codex CLI | `captured` | 0.152.0 | MCP and minimal native hook translation over the existing Session runtime; live lifecycle unverified | Authentic MCP and native hook start/tool/stop/end/resume frames are captured; complete authenticated Synveda lifecycle remains unverified. |
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
- A spool is pinned to its first authenticated gateway origin; a profile switch to another deployment holds the run instead of sending it.
- Payload hashes detect accidental corruption but do not authenticate local state against an attacker with arbitrary write access to the same account.
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

### Codex CLI — `captured`

Contract: Codex CLI 0.152.0 native hooks / MCP 2025-06-18. Evidence level: `captured-protocol`.

Authentic fixtures:

- `crates/synveda-cli/fixtures/mcp/codex.json` — captured-client-frames, SHA-256 `d807a328550a652430c35a2c4b65f0c29b193447b8255a7e9c2fcf2d49d71289`
- `adapters/codex/fixtures/lifecycle.json` — captured-client-hooks, SHA-256 `3bb629ff3d42681b64c1eb8abd21404aebef87bf986f084e90e44dea02b66a39`
- `adapters/codex/fixtures/transcript.jsonl` — captured-client-transcript, SHA-256 `57d88963968808bba39b344115604df9e49fd4e21355bef65bc797401dbf9915`

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

- Authentic MCP and native hook start/tool/stop/end/resume frames are captured; complete authenticated Synveda lifecycle remains unverified.
- Use native Codex MCP configuration; Synveda does not write its TOML file.
- SessionEnd reason=other is followed by resume of the same Codex session; it cannot automatically close a Synveda task. Explicit application ownership remains required.
- Both .agents/skills and .codex/skills were loaded by the installed client; no Skill-path migration is needed for this version.
- Native hook capture requires normal project/hook trust loading. --ignore-user-config did not emit hooks in the probe; CLI 0.152.0 was exercised with GPT-5.5 at low reasoning effort.
- The hook adapter replays startup/resume, durable Stop, outage/retry and runtime exit with a synthetic HTTP responder; native authenticated context consumption and Capture/end/audit qualification remain open.
- Compaction and non-command tool-result formats are unqualified. The native reader holds transcripts over 8 MiB/20,000 records; a host killed before any delivery hook can lose the unfinished turn.

### Claude Desktop — `captured`

Contract: MCP 2025-11-25 captured. Evidence level: `captured-protocol`.

Authentic fixtures:

- `crates/synveda-cli/fixtures/mcp/claude-desktop-probe.json` — captured-client-frames, SHA-256 `71b69ccbfefdd67a0d0aafa7b2ae8692892d49a909769710ac06c1ee91b421eb`
- `crates/synveda-cli/fixtures/mcp/claude-desktop-agent.json` — captured-client-frames, SHA-256 `cd54bc373b89495bd730da8bf6656d51a06ff238a2330b925ff274cdda6e68ac`

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

- `crates/synveda-cli/fixtures/mcp/zed.json` — captured-client-frames, SHA-256 `ebb6ce26f329d653cba2f4061160708c34e9ce17dbd82d0bde8830e03fc3b3d5`

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
