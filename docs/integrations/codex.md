# Codex CLI

Synveda gives Codex CLI approved project context at the start of each session
and again after compaction, records what happens in the session for later
review, and offers a `recall` tool over MCP. Codex CLI 0.152.0 on macOS arm64
completed the full lifecycle against a live server; the
[client support matrix](../CLIENT_SUPPORT.md) lists what else is and is not
qualified.

## Set up

Follow [Connect an AI coding agent](../AGENT_SETUP.md). The Codex step, run at
the root of your Git repository after `synveda setup`, is:

```sh
synveda adapter install --client codex
```

It does two things:

1. Adds Synveda's `SessionStart`, `PreCompact`, `Stop` and `SessionEnd` hooks to
   `.codex/hooks.json`, keeping any hooks you already have. The entries name
   paths on your machine; keep them out of version control.
2. Runs `codex mcp add synveda -- <synveda> mcp --writes host` to register the
   MCP server for your user. If `codex` is not on PATH, the command prints this
   line for you to run yourself.

Start `codex` in the repository, trust the project when asked and approve the
Synveda hooks with `/hooks`; Codex asks again whenever a hook changes. `/mcp`
lists the `synveda` server. Codex versions that still gate hooks behind a
feature flag also need `hooks = true` under `[features]` in
`.codex/config.toml`. To undo both changes, run
`synveda adapter uninstall --client codex`.

If you sign in with an isolated `XDG_CONFIG_HOME`, Codex does not pass it to MCP
servers by default. Add `env_vars = ["XDG_CONFIG_HOME"]` to the
`[mcp_servers.synveda]` table in `~/.codex/config.toml`. See the vendor
[MCP environment contract](https://learn.chatgpt.com/docs/extend/mcp).

## How it works

| Hook | What Synveda does |
| --- | --- |
| `SessionStart` | Opens or resumes the Synveda Session and returns the allowed context, starting with the **Synveda Session ID**. After compaction (`source: "compact"`) it composes fresh context, using `compact_budget_tokens` from `.synveda/config.json` when set. |
| `PreCompact`, `Stop` | Record the turn's text, command calls and results, and text-only MCP results in a local spool. |
| `SessionEnd` | Delivers the spooled events, within the host's three-second limit. A later start delivers anything left over. |

- **MCP recall.** The MCP server is shared between conversations, so pass the
  Synveda Session ID from the context block to each `recall` call. Codex's own
  thread ID is a different identifier.
- **One writer.** The hooks record the session, so the MCP server runs with
  `--writes host` and offers `recall` only. For context without recording, set
  `"observe": false` in `.synveda/config.json`.
- **Ending a task.** Codex exiting does not end the Synveda Session. Whoever
  owns the task asks for Capture and ends the Session through the
  [SDKs](../../sdks/README.md) or the public API.
- **Opting out.** `SYNVEDA_DISABLED=1`, or `"disabled": true` in
  `.synveda/config.json`, turns the hooks off.

Use Codex's normal tool approval for MCP calls; Synveda separately checks every
API call against your access.

## A dedicated task without hooks

For a single Codex run that should write to one application task, bind the MCP
server to a stable task key instead of using hooks:

```sh
codex \
  -c 'mcp_servers.synveda.command="/absolute/path/to/synveda"' \
  -c 'mcp_servers.synveda.args=["mcp","--task","issue-123","--project","PROJECT_ID"]'
```

Reuse the key only for that same task, and never put a fixed task key in shared
configuration. If the application already opened a Synveda Session, use
`--session SESSION_ID` instead of `--task`. Do not combine this with the hooks.

## Approved Skills

Codex reads Skills from `.agents/skills` and `.codex/skills`. Install the Skills
your team approved for a scope with:

```sh
synveda skill sync --scope SCOPE_ID --client codex
```

A Skill file on disk grants no extra authority.

## Running from a source checkout

Contributors who change the adapter build it with the locked dependencies:

```sh
pnpm install --frozen-lockfile
pnpm --filter @synveda/codex-adapter... build
```

Then merge these entries into `.codex/hooks.json` by hand, with absolute paths
to your `synveda` binary, Node 22 or newer and the built hook:

```json
{
  "hooks": {
    "SessionStart": [{"hooks": [{"type": "command", "command": "SYNVEDA_CLI=/absolute/path/to/synveda /absolute/path/to/node /absolute/path/to/synveda/adapters/codex/dist/hook.mjs", "timeout": 12}]}],
    "PreCompact": [{"hooks": [{"type": "command", "command": "SYNVEDA_CLI=/absolute/path/to/synveda /absolute/path/to/node /absolute/path/to/synveda/adapters/codex/dist/hook.mjs", "timeout": 12}]}],
    "Stop": [{"hooks": [{"type": "command", "command": "SYNVEDA_CLI=/absolute/path/to/synveda /absolute/path/to/node /absolute/path/to/synveda/adapters/codex/dist/hook.mjs", "timeout": 12}]}],
    "SessionEnd": [{"hooks": [{"type": "command", "command": "SYNVEDA_CLI=/absolute/path/to/synveda /absolute/path/to/node /absolute/path/to/synveda/adapters/codex/dist/hook.mjs", "timeout": 3}]}]
  }
}
```

`make plugin-package-check` replays the captured lifecycle through a packaged
archive. See the vendor [hook contract](https://learn.chatgpt.com/docs/hooks).

## Known limits

- Non-text MCP results are held, not recorded.
- A Codex process killed before a hook runs loses that turn.
- Skill file reads are observed, but automatic Skill activation is not claimed.
- Other Codex versions and platforms, including Windows hook execution, are not
  yet qualified.

The dated test runs behind these statements, and how to repeat them, are in the
[qualification record](codex-evidence.md).
