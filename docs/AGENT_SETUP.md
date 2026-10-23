# Connect an AI coding agent

Synveda gives Claude Code, Codex CLI and GitHub Copilot CLI a governed memory
that your team reviews. This guide connects one of them, on your own machine,
to a Synveda server that is already running. It takes four commands.

You need:

- **The server's URL**, for example `https://synveda.example.com`, or
  `http://localhost:8080` if you followed [Try it locally](../README.md#try-it-locally).
- **An account on that server.** If you can sign in to its web console, you
  have one.
- **The [GitHub CLI](https://cli.github.com) (`gh`).** The installer uses it to
  check that the release was published by this repository. Run `gh auth login`
  once if you have never used it.
- **Your agent**: [Claude Code](https://docs.anthropic.com/en/docs/claude-code),
  [Codex CLI](https://github.com/openai/codex) or
  [GitHub Copilot CLI](https://docs.github.com/en/copilot/how-tos/copilot-cli),
  installed and signed in.

## 1. Install the Synveda CLI

macOS or Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/synveda/synveda/main/scripts/install.sh | sh
```

Windows (PowerShell):

```powershell
powershell -NoProfile -Command "irm https://raw.githubusercontent.com/synveda/synveda/main/scripts/install.ps1 | iex"
```

The installer downloads the latest release for your system and checks its
signature and checksums before running anything from it. It installs into
`~/.synveda` (Windows: `%LOCALAPPDATA%\SynvedaClient`) and adds the `synveda`
command to your PATH. It does not change any agent's settings. You need no
Node.js, Docker or compiler; the release carries its own runtime.

Open a new terminal and check:

```sh
synveda --version
```

Use the same version as your server. To install a specific version, run
`curl ... | SYNVEDA_VERSION=0.4.7 sh`, or download `install.ps1` and run
`./install.ps1 -Version 0.4.7`. To leave your PATH alone, set
`SYNVEDA_NO_MODIFY_PATH=1` (Windows: `-NoModifyEnvironment`).

On Windows, if PowerShell says running scripts is disabled, allow local scripts
once with `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned`.

## 2. Sign in

```sh
synveda login --gateway https://synveda.example.com
synveda whoami
```

`login` opens your browser to sign in and saves a credential for your user
account only. Plain `http://` URLs are accepted only for `localhost`.

## 3. Choose the project for your repository

Run these from the root of the Git repository you work in:

```sh
synveda setup                                        # lists the projects you can use
synveda setup --project PROJECT_ID --observation on
```

`setup` writes `.synveda/config.json` in the repository. `--observation on`
lets the agent's hooks record this repository's sessions. With `off` (the
default) the agent still receives context, but nothing is recorded. Recorded
sessions only become *proposed* learnings; a person reviews them before any
later session can use them.

## 4. Connect your agent

Still in the repository root, run the line for your agent:

| Agent | Command | What it changes |
| --- | --- | --- |
| Claude Code | `synveda adapter install --client claude-code` | Registers the Synveda plugin (hooks and MCP server) for this repository. |
| Codex CLI | `synveda adapter install --client codex` | Adds the Synveda hooks to `.codex/hooks.json`, keeping your own hooks, and runs `codex mcp add synveda`. |
| GitHub Copilot CLI | `synveda adapter install --client copilot-cli` | Writes `./.github/hooks/synveda.json` and runs `copilot mcp add synveda`. |

Add `--dry-run` to see the changes first. Rerunning the command is safe.
These commands need Synveda 0.4.7 or later. With v0.4.6, add
`--scope project` for Claude Code, and for Codex and Copilot follow that
release's
[Codex](https://github.com/synveda/synveda/blob/v0.4.6/docs/integrations/codex.md) and
[Copilot](https://github.com/synveda/synveda/blob/v0.4.6/docs/integrations/copilot-cli.md)
guides.

Then start your agent in the repository and accept its prompts:

- **Claude Code:** start a new session and trust the plugin. `claude plugin list`
  shows `synveda@synveda` as enabled.
- **Codex CLI:** trust the project when asked, then approve the Synveda hooks
  with `/hooks`. `/mcp` lists the `synveda` server.
- **Copilot CLI:** trust the folder when asked. `/mcp show` lists the `synveda`
  server.

The hook files written for Codex and Copilot contain paths on your machine, so
keep them out of version control.

## 5. Check that it works

Start a session and ask the agent something about your project. At the start
of each session the agent receives a Synveda context block that begins with a
**Synveda Session ID**. With observation on, the session appears under
**Sessions** in the web console.

`synveda adapter status --client NAME` shows whether the connection is in place
and unchanged.

## Update or disconnect

- **Update:** run the install command from step 1 again, then rerun
  `synveda adapter install --client NAME` for each connected agent.
- **Disconnect an agent:** `synveda adapter uninstall --client NAME`. It removes
  only what Synveda added and keeps your sessions, settings and credentials.
- **Remove the CLI:** disconnect your agents, then delete `~/.synveda`
  (Windows: `%LOCALAPPDATA%\SynvedaClient`) and the PATH line the installer
  added to your shell profile (Windows: your user `Path` variable).

## Other MCP clients

Cursor, VS Code, Claude Desktop, Zed, Windsurf and Continue connect through an
MCP server entry rather than hooks:

```sh
synveda adapter install --client cursor
```

They can recall knowledge but do not record sessions on their own. The
[client support matrix](CLIENT_SUPPORT.md) lists every client, the versions
that were tested and what each one can do.

## Troubleshooting

| Message or symptom | What to do |
| --- | --- |
| `synveda: command not found` | Open a new terminal. If it persists, add `~/.synveda/bin` (Windows: `%LOCALAPPDATA%\SynvedaClient\bin`) to PATH. |
| `the GitHub CLI (gh) is required` | Install `gh` from <https://cli.github.com> and rerun the installer. |
| `run synveda setup --project ID ... first` | Run step 3 in the same repository. |
| `MCP server not registered: ...` | The hooks are installed, but the agent's own `mcp add` failed or is not on PATH. Run the command printed in the message. |
| `an existing registration has no managed ownership receipt` | You set Synveda up by hand earlier. Remove that entry (or the hook file it names), then rerun the command. |
| `registration changed outside this command` | Someone edited the Synveda entries. Restore or remove them, then rerun. |
| The agent gets no Synveda context | Run `synveda whoami`. If your sign-in expired, run `synveda login` again. Make sure you started the agent at the repository root. |
| Copilot ignores the hooks | Copilot loads hooks only from the Git repository root and only in a trusted folder. |

## Building from source

Contributors who run the adapters from a source checkout follow the adapter
guides instead: [Claude Code](../adapters/claude-code/README.md),
[Codex CLI](integrations/codex.md) and [Copilot CLI](integrations/copilot-cli.md).
