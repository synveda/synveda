# Native consumer commands (OPS-12 candidate)

These commands are in the current source CLI and matching plugin candidate.
**They are not in published v0.4.0.** Keep using the
[published installation guide](../deploy/compose/PREBUILT.md) for that release.
Build the current CLI with `SQLX_OFFLINE=true cargo build -p synveda-cli`.
Native client-only archives and private Node runtimes remain open work.

## Local deployment

Use the extracted [plain-Compose candidate](../deploy/compose/CONSUMER.md), a
local Linux-container Docker engine and Compose 2.35+. The CLI and bundle must
have matching versions. Startup lets Compose pull the bundle's pinned images
when absent; the CLI does not build images.
From a terminal, using the built CLI on PATH:

```sh
synveda doctor --bundle /absolute/path/to/synveda-reference-0.4.0
synveda up --bundle /absolute/path/to/synveda-reference-0.4.0
synveda status --bundle /absolute/path/to/synveda-reference-0.4.0
synveda logs --bundle /absolute/path/to/synveda-reference-0.4.0 --tail 100
synveda down --bundle /absolute/path/to/synveda-reference-0.4.0
```

Without `--bundle`, the commands look in `$SYNVEDA_HOME/reference/current`
(default `~/.synveda/reference/current`). An older host-state bundle is refused;
its launcher is a different layout. `--dry-run` prints arguments without Docker
access or file writes. `doctor` checks prerequisites, file identity and ownership;
it does not establish authentication or recoverability.

The project defaults to `synveda-local`. Tests can explicitly select a fresh
`synveda-local-acceptance-<name>` using `--project-name`. On first startup, any
retained containers, networks or volume names for that project cause refusal.
A private receipt binds the original engine, project, canonical bundle path and
runtime files before Compose starts. The same command can resume a partial
start. A moved/changed bundle or different engine is refused; do not delete the
receipt to force adoption. Preserve the original artifact and inspect the
reported difference. This is not a cross-release upgrade command.

`down` stops the graph with every profile selected and retains all volumes,
identity and encryption keys. It never passes `--volumes`. Follow the candidate
guide for deliberate password retrieval, sample use and paired recovery. Direct
Compose and recovery operations must not run concurrently with these commands.

## Select a project and observation choice

Log in using the existing gateway/profile flow. Creating workspaces and projects
remains in the console's Getting started flow; it uses the same public APIs.
Run setup from the target Git repository (a subdirectory is also accepted):

```sh
synveda login --gateway http://localhost:8080 --profile evaluation
synveda setup --profile evaluation
synveda setup --profile evaluation --project PROJECT-UUID --observation off
```

Bare `setup` lists policy-visible projects. Selecting one verifies that exact
active project and its active workspace through authenticated public APIs before
writing `.synveda/config.json` at the Git root. It preserves other settings and
refuses conflicting unmanaged selection. It never stores a bearer or gateway
override in the repository. `--dry-run` verifies the API and prints the selection
without changing project configuration; ordinary credential refresh still applies.

Observation defaults to **off**. Choose `--observation on` explicitly to permit
the hooks to record this project's transcript and tool events. Setup records the
choice privately and marks the project configuration as managed. The updated
hook runtime checks both against the project, workspace and selected profile.
A copied managed config, changed selection, wrong profile, unreadable receipt
or missing config cannot enable recording. Context requests and explicit MCP
tool calls remain separately governed runtime actions.

Hooks launched below the Git root use that root's configuration. A nested Git
repository is a separate project. Select the same `SYNVEDA_PROFILE` and any
isolated `XDG_CONFIG_HOME` in the harness environment; project files do not choose
credentials. Existing manually configured adapters retain their prior contract.
Managed consent requires the matching updated CLI **and** plugin runtime.

## Register an existing adapter

For Claude Code, build/package the updated plugin as described in the
[adapter guide](../adapters/claude-code/README.md), then:

```sh
synveda adapter install --client claude-code --scope project --from /absolute/path/to/plugin
synveda adapter status --client claude-code --scope project
synveda adapter uninstall --client claude-code --scope project
```

This drives Claude's native installer (scope protocol verified with 2.1.241).
The managed route supports `project` and `local`; user-wide hooks cannot use
one project's consent for every repository. The bundle must carry the checked
receipt-aware hook contract; historical same-version bundles are refused.
Registration preserves other scopes, the shared marketplace and persistent
plugin data. Review the vendor's trust prompts and start a new session to verify
loading. Enabled registration alone is not loading or live-client evidence.

For existing registry-backed MCP clients, `--scope user` uses the registry's
user config. Project scope requires an explicit file inside the current repository:

```sh
synveda adapter install --client zed --scope project --config .zed/settings.json
synveda adapter status --client zed --scope project --config .zed/settings.json
synveda adapter uninstall --client zed --scope project --config .zed/settings.json
```

Use these relative paths from the repository root. Setup's profile and exact
workspace/project enter the MCP arguments, with `--writes tool`. Shared servers
remain unbound: provide the Synveda Session ID with each tool call. JSONC edits
preserve unrelated entries and comments. A configuration recipe is not a new
support claim; the [client matrix](CLIENT_SUPPORT.md) remains authoritative.
Codex and Copilot hook installation and trust retain their respective
[Codex](integrations/codex.md) and [Copilot](integrations/copilot-cli.md) guides.
Their compiled shared runtime understands managed observation when updated.

Every mutation retains a private ownership receipt; an interrupted install can
be retried with the same selection. An existing entry without that receipt is
not silently adopted. Removal refuses a changed entry or native marketplace
source, and preserves unrelated configuration, credentials, setup consent and
spools. Use `--dry-run` on install/removal to inspect the intended action.

Receipts live under `$XDG_CONFIG_HOME/synveda/consumer` (default
`~/.config/synveda/consumer`) with private directory/file permissions. They hold
selection and hashes, never tokens or transcripts. A bounded OS lock serializes
setup/lifecycle and both new and older plugin/MCP mutations. Do not remove its
`operations.lock` file: the OS releases the lock on process exit. Direct vendor
commands and editors do not participate; keep them idle during configuration
edits. Changed files are refused when detected before atomic replacement.
