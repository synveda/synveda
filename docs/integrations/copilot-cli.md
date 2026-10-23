# GitHub Copilot CLI

Synveda gives GitHub Copilot CLI approved project context at the start of each
session, records what happens in the session for later review, and offers a
`recall` tool over MCP. Copilot CLI 1.0.83 on macOS arm64 completed the full
lifecycle against a live server; the [client support matrix](../CLIENT_SUPPORT.md)
lists what else is and is not qualified. This guide covers no Copilot cloud
agent or VS Code integration.

## Set up

Follow [Connect an AI coding agent](../AGENT_SETUP.md). The Copilot step, run at
the root of your Git repository after `synveda setup`, is:

```sh
synveda adapter install --client copilot-cli
```

It does two things:

1. Writes `./.github/hooks/synveda.json`. Copilot loads repository hooks only from
   the Git repository root, so the command refuses to run outside a repository.
   The file names paths on your machine; keep it out of version control.
2. Runs `copilot mcp add synveda -- <synveda> mcp --writes host` to register the
   MCP server for your user. If `copilot` is not on PATH, the command prints
   this line for you to run yourself.

Start `copilot` in the repository and trust the folder when asked. `/mcp show`
lists the `synveda` server. To undo both changes, run
`synveda adapter uninstall --client copilot-cli`.

## How it works

| Hook | What Synveda does |
| --- | --- |
| `sessionStart` | Opens or resumes the Synveda Session for this conversation and returns the allowed context as `additionalContext`. The context starts with the **Synveda Session ID**. |
| `agentStop` | Records the turn's text, tool calls and known text results in a local spool. It makes no network call. |
| `sessionEnd` | Delivers the spooled events, within a two-second budget. A later start delivers anything left over. |

- **MCP recall.** The MCP server is shared between conversations, so pass the
  Synveda Session ID from the context block to each `recall` call. Copilot's
  own session UUID is a different identifier.
- **One writer.** The hooks record the session, so the MCP server runs with
  `--writes host` and offers `recall` only. For context without recording, set
  `"observe": false` in `.synveda/config.json`.
- **Ending a task.** Copilot exiting does not end the Synveda Session. Whoever
  owns the task asks for Capture and ends the Session through the
  [SDKs](../../sdks/README.md) or the public API.
- **Opting out.** `SYNVEDA_DISABLED=1`, or `"disabled": true` in
  `.synveda/config.json`, turns the hooks off.
- **Profiles.** If you sign in with `--profile NAME`, run `synveda setup` with
  the same profile; the hooks and MCP server then use it too.

The hook reads at most 64 KiB from stdin and stops after ten seconds. The
transcript reader accepts only regular files, at most 8 MiB and 20,000 lines
per read. Malformed, foreign or unrecognised transcript content is held, with a
content-free diagnostic, rather than guessed at. System instructions,
reasoning and encrypted fields are never recorded.

## Approved Skills

Install the Skills your team approved for a scope into Copilot's project Skill
folder, after reviewing the dry run:

```sh
synveda skill sync --scope SCOPE_ID --client copilot-cli --root "$PWD/.github/skills" --dry-run
synveda skill sync --scope SCOPE_ID --client copilot-cli --root "$PWD/.github/skills"
copilot skill list --json
```

Sync fetches exact approved versions, verifies their content hashes and removes
only unchanged copies it installed when an approval is withdrawn. A Skill file
on disk grants no extra authority, and Copilot listing it is not evidence that
the model used it.

## Running from a source checkout

Contributors who change the adapter build it with the locked dependencies:

```sh
pnpm install --frozen-lockfile
pnpm --filter @synveda/copilot-cli-adapter... build
pnpm --filter @synveda/copilot-cli-adapter test
```

Then write `./.github/hooks/synveda.json` by hand, with absolute paths to Node 22
or newer, the built hook and your `synveda` binary:

```json
{
  "version": 1,
  "hooks": {
    "sessionStart": [{
      "type": "command",
      "exec": "/absolute/path/to/node",
      "args": ["/absolute/path/to/synveda/adapters/copilot-cli/dist/hook.mjs", "sessionStart"],
      "env": { "SYNVEDA_CLI": "/absolute/path/to/synveda" },
      "timeoutSec": 12
    }],
    "agentStop": [{
      "type": "command",
      "exec": "/absolute/path/to/node",
      "args": ["/absolute/path/to/synveda/adapters/copilot-cli/dist/hook.mjs", "agentStop"],
      "env": { "SYNVEDA_CLI": "/absolute/path/to/synveda" },
      "timeoutSec": 12
    }],
    "sessionEnd": [{
      "type": "command",
      "exec": "/absolute/path/to/node",
      "args": ["/absolute/path/to/synveda/adapters/copilot-cli/dist/hook.mjs", "sessionEnd"],
      "env": { "SYNVEDA_CLI": "/absolute/path/to/synveda" },
      "timeoutSec": 5
    }]
  }
}
```

For a deliberately trusted headless run, Copilot loads repository hooks in
prompt mode only with `GITHUB_COPILOT_PROMPT_MODE_REPO_HOOKS=true`; never set it
globally. See the vendor [hook reference](https://docs.github.com/en/copilot/reference/hooks-reference)
and [Skill discovery](https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-skills).

## Known limits

- Native outage recovery and compaction are not qualified. Copilot's
  `preCompact` is a notification only, so no context is re-injected after
  compaction.
- Unknown or non-text tool-result formats are held, not recorded.
- The final assistant message of a session is delivered at the next start or
  exit of the same Copilot session.
- A Copilot process killed before `agentStop` loses that turn.
- Other Copilot versions and platforms, and Windows hook execution, are not
  yet qualified.

The dated test runs behind these statements are in the
[qualification record](copilot-cli-evidence.md).
