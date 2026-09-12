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

## Captured hook adapter

The workspace now includes `adapters/codex`. Build with
`pnpm --filter @synveda/codex-adapter... build` after the normal frozen-lockfile
install. Sign in with `synveda login` and select the target through
`SYNVEDA_WORKSPACE`/`SYNVEDA_PROJECT` or `.synveda/config.json`, as with the
existing adapter. CLI credentials pin the gateway; project settings cannot
redirect them. `SYNVEDA_DISABLED=1` disables the hooks.

For a controlled qualification checkout, enable `hooks = true` in the
`[features]` table of its `.codex/config.toml`. Merge these entries into its
`.codex/hooks.json`, replacing the Node and built hook paths:

```json
{
  "hooks": {
    "SessionStart": [{"hooks": [{"type": "command", "command": "/absolute/path/to/node /absolute/path/to/synveda/adapters/codex/dist/hook.mjs", "timeout": 12}]}],
    "PreCompact": [{"hooks": [{"type": "command", "command": "/absolute/path/to/node /absolute/path/to/synveda/adapters/codex/dist/hook.mjs", "timeout": 12}]}],
    "Stop": [{"hooks": [{"type": "command", "command": "/absolute/path/to/node /absolute/path/to/synveda/adapters/codex/dist/hook.mjs", "timeout": 12}]}],
    "SessionEnd": [{"hooks": [{"type": "command", "command": "/absolute/path/to/node /absolute/path/to/synveda/adapters/codex/dist/hook.mjs", "timeout": 3}]}]
  }
}
```

Use the normal project trust and `/hooks` review flow to activate that exact
source. Do not disable hook trust. Codex 0.152.0 was exercised with GPT-5.5 and
low reasoning effort; `--ignore-user-config` did not load these trusted hooks.
See the [vendor hook contract](https://learn.chatgpt.com/docs/hooks).

With hooks observing turns, configure the MCP command as
`synveda mcp --writes host --project <project-id>`. Leave it unbound and pass
the **Synveda Session ID** returned in SessionStart context to each recall
call. Do not combine the hook-created Session with a separate `--task` key;
the dedicated tool-only recipe above is a different write-owner arrangement.

If login uses an isolated `XDG_CONFIG_HOME`, allow that existing variable into
the native MCP subprocess with `env_vars = ["XDG_CONFIG_HOME"]` in
`[mcp_servers.synveda]`, and select the same `--profile` as the hooks. The
qualification run also forwarded `SYNVEDA_INSECURE_DEVELOPMENT_HTTP` for the
documented local HTTP deployment. Do not put a bearer in project configuration.
Codex filters subprocess environments; a working hook login alone does not
prove that its MCP process sees the same profile. See the
[native MCP environment contract](https://learn.chatgpt.com/docs/extend/mcp).

The native ID maps to `codex:<native-id>` in the existing local spool. Stop and PreCompact
records user/assistant text, command calls/results and text-only MCP results;
runtime exit
flushes them. A resumed invocation uses the same Synveda Session. The task
owner explicitly requests Capture and ends that Session through the public
API/SDK when finished. No native hook establishes final task completion.
After compaction, `SessionStart` with `source: "compact"` composes fresh allowed
context through the same API, using `compact_budget_tokens` from
`.synveda/config.json` when configured. PostCompact needs no additional hook
registration. The adapter accepts the vendor's manual/auto trigger vocabulary;
the live evidence below distinguishes which paths actually ran.

## Evidence limits

The original protocol capture deliberately had no Synveda credential and
received the server's sign-in error. A later real Keycloak run exercised native
context consumption, approved Skill file reading, authenticated MCP recall and
resume of the same task. Both SDKs used that Session through the public proxy;
14 unique events persisted, Capture completed, the task owner explicitly ended
it and a separate application Session reused allowed Knowledge. The audit chain
verified through sequence 427. The digest-pinned content-free result is
`adapters/codex/fixtures/keycloak-qualification.json`.

The adapter translates captured startup/resume/compact, PreCompact, Stop/exit,
command results and text-only MCP results. Unknown output/status shapes are held. Native exit gets
one two-second deadline for credentials and delivery below the observed
three-second host cap.

The follow-up `fixtures/recovery-qualification.json` under `adapters/codex`
records native outage/recovery and manual compaction on Session
`01a09702-10aa-79b2-ae1a-fca231deb5af`. With only the owned gateway paused,
Codex completed a turn and retained five pending events. Resume delivered all
five once. The manual compact-start hook initially returned no context; after
the filter correction it returned allowed context and the same Session ID.
Both SDKs then completed their workflows and Codex resumed again. All 25 unique
events persisted, Capture produced 23 candidates, explicit end and cross-session
reuse passed, and the audit chain verified through sequence 655, including
Session open/end in the Session-filtered history.

An `exec resume` probe using `model_auto_compact_token_limit=1000` completed a
normal turn but emitted no compaction hooks. It does not qualify automatic
compaction. The setting is documented in the
[OpenAI configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference);
its observed behaviour on this invocation is the limit of the evidence.
Automatic compaction/reinjection, live revoke/re-authorisation, non-text MCP
results, installation packaging and other versions/platforms remain unqualified. Reads
over 8 MiB/20,000 records are held; unfinished turns can be lost if no hook runs
before host death. Skill file reading is observed, but automatic activation is
not claimed. The audit `session_id` filter now matches both delivery identities
and lifecycle snapshots. For a completed history across pages, hold `until`
fixed: audit reads append their own evidence. The earlier qualification receipt
retains the original filter failure; the correction and its validation are
recorded in the [interoperability plan](../INTEROPERABILITY_PLAN.md).
CPR-39 remains open until all applicable ADR-0098 criteria pass.

GitHub Copilot CLI and Pi remain separate, unqualified candidates. They have
no Synveda lifecycle adapter in this checkout. The VS Code registry entry is
not evidence for Copilot CLI, and MCP support alone is not a complete lifecycle.
