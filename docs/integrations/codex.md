# Codex CLI interoperability (CPR-39)

Codex CLI **0.152.0** has authentic Synveda MCP protocol evidence from
2026-09-12. `adapters/registry.json` classifies it as `captured`, not `verified`.
`crates/synveda-cli/fixtures/mcp/codex.json` retains its actual initialization,
tool discovery and recall frames. Existing `rmcp` serves its negotiated
MCP 2025-06-18 protocol without a new transport adapter.

The isolated native run loaded both `.agents/skills` and `.codex/skills`.
Synveda's existing Codex Skill target therefore remains unchanged. Use
`synveda skill sync --scope <scope-id> --client codex` to materialise versions
made available by approved bindings. A local file does not grant tool authority.

## Explicit task binding

Sign in through `synveda login` first, using the documented deployment gateway
and issuer. Codex launches the local Synveda MCP server; that server calls the
authenticated public API. Synveda is not managing external MCP servers.

For one dedicated Codex invocation, give the task a stable application key:

```sh
codex \
  -c 'mcp_servers.synveda.command="/absolute/path/to/synveda"' \
  -c 'mcp_servers.synveda.args=["mcp","--task","issue-123","--project","PROJECT-UUID"]'
```

Replace the executable, project and task key. Reuse the key only for that same
application task across transport restarts. Do not put a fixed task key in a
shared global server configuration. If the application already opened a
Synveda Session, use `--session <session-uuid>` instead. For a server shared
between tasks, leave it unbound and pass each task's Synveda `session_id` in
tool arguments. Codex's own thread ID is not a Synveda Session UUID.

Use Codex's normal MCP approval controls. Headless `exec` with approval policy
`never` initially refused recall; a fixture-only per-tool `approval_mode =
"approve"` allowed that read to reach Synveda. This is host tool approval;
Synveda still authenticates and authorises the API call separately. See the
[vendor MCP configuration](https://learn.chatgpt.com/docs/extend/mcp?surface=cli).

Synveda does not currently write Codex TOML configuration. The native
`codex mcp add synveda -- /absolute/path/to/synveda mcp` command can register an
unbound shared server; the caller must then supply a Synveda Session ID.

## Evidence limits

The native protocol capture deliberately had no Synveda credential and
received the server's actionable sign-in error. Authenticated API, task
isolation and audit behavior are covered separately by the gateway/SDK
acceptance suite. Neither result establishes a complete Codex lifecycle.

The isolated headless probe emitted no lifecycle hook frames after trying
inline overrides and a project hooks file with the hooks feature enabled.
Do not translate undocumented or missing events, or infer SessionEnd from an
MCP disconnect. Next qualification requires a trusted hook source in the
installed client's normal hook review flow, followed by captured start,
observation, context, Capture, end and resume behavior against Keycloak.
The [vendor hook contract](https://learn.chatgpt.com/docs/hooks) describes that
trust flow. CPR-39 remains open until all ADR-0098 criteria pass.

GitHub Copilot CLI and Pi remain separate, unqualified candidates. They have
no Synveda lifecycle adapter in this checkout. The VS Code registry entry is
not evidence for Copilot CLI, and MCP support alone is not a complete lifecycle.
