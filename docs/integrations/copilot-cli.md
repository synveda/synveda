# GitHub Copilot CLI interoperability (ADPT-9)

Copilot CLI has an **experimental context adapter**. This is a source-build
start/resume increment; it is not the verified Codex lifecycle or a Copilot
cloud-agent/VS Code adapter. `adapters/registry.json` owns the support level.
The installed CLI is 1.0.83 on macOS arm64. Native local MCP configuration and
Skill discovery work. The approved 2026-09-13 native probe captured six hook
events and synthetic Skill activation. Copilot accepted the hook output but
returned the conflicting Skill's marker, so the context-marker assertion failed.
Synveda context consumption, authenticated MCP and full lifecycle acceptance
remain unverified. An additional paid retry needs explicit approval.

## Build and connect

Follow [the Docker reference guide](../../deploy/compose/README.md) for gateway,
issuer, hosts and secret setup. Sign in with `synveda login` and select the
workspace/project in `.synveda/config.json` or `SYNVEDA_WORKSPACE` and
`SYNVEDA_PROJECT`. Hooks and MCP must use the same `SYNVEDA_PROFILE` and any
isolated `XDG_CONFIG_HOME`; keep bearer credentials out of project files.
CLI-resolved credentials pin the gateway origin. Configuration does not grant
access to a selected workspace.

```sh
pnpm install --frozen-lockfile
pnpm --filter @synveda/copilot-cli-adapter... build
pnpm --filter @synveda/copilot-cli-adapter test
```

In a trusted qualification checkout, add **.github/hooks/synveda.json**, replacing
the two absolute executable paths. Leave other hook files intact:

```json
{
  "version": 1,
  "hooks": {
    "sessionStart": [{
      "type": "command",
      "exec": "/absolute/path/to/node",
      "args": ["/absolute/path/to/synveda/adapters/copilot-cli/dist/hook.mjs", "sessionStart"],
      "timeoutSec": 12
    }]
  }
}
```

Use Copilot's normal folder trust and hook review. For a deliberately trusted
headless qualification checkout, the vendor documents
`GITHUB_COPILOT_PROMPT_MODE_REPO_HOOKS=true` to load repository hooks in prompt
mode. Do not set it globally for arbitrary repositories. The adapter accepts
the native camelCase payload and returns top-level `additionalContext`, with
the allowed context and its explicit **Synveda Session ID**. It bounds stdin
to 64 KiB and execution to ten seconds. `SYNVEDA_DISABLED=1` or the project's
`disabled` setting opts out. See the
[vendor hook contract](https://docs.github.com/en/copilot/reference/hooks-reference).

Register Synveda using the native configuration command:

```sh
copilot mcp add synveda -- /absolute/path/to/synveda mcp --writes tool
```

This configures Synveda **as an MCP server**. No external MCP server management
is required. The existing maintained `rmcp` server handles the protocol and
calls Synveda's authenticated public API. Use normal Copilot tool approval;
Synveda independently enforces each API operation. Native CLI configuration
uses `type: "local"`, a command/args array and a tools list. Synveda does not
currently write that format; the native command owns it.

Leave this shared MCP server unbound. Pass the hook's Synveda `session_id` to
each recall/observe call; the native Copilot UUID is not a Synveda Session UUID.
Do not also configure `--task` with a different task key. This context-only hook
does not observe transcripts, so `--writes tool` enables explicit model-requested
observations. If an application/SDK owns observation delivery for the task,
use `--writes host` instead to prevent a second writer. Explicit observations
are not an automatic transcript or an approved Knowledge publication.

## Approved Skills and SDK handoff

The existing installer supports a custom root; no new Skill target is needed:

```sh
synveda skill sync --scope SCOPE-UUID --client copilot-cli --root "$PWD/.github/skills" --dry-run
synveda skill sync --scope SCOPE-UUID --client copilot-cli --root "$PWD/.github/skills"
copilot skill list --json
```

`--root` is an existing override; the client string labels its local receipts.
Use one managed root/scope, review the dry run and keep any materialised content
within its allowed project. The API resolves enabled approved bindings, fetches
exact immutable versions and verifies their content hashes; sync removes only
its unchanged managed copies when bindings disappear. The native
[Skill discovery contract](https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-skills)
includes **.github/skills**. Local discovery is not evidence of model activation
or continuing tool authority after a policy change.

Pass the injected Synveda Session ID to the existing
[Python and TypeScript workflow examples](../../sdks/README.md). They retrieve
context and an approved Skill, submit a proposal, test foreign-workspace denial
and correlate actual response trace IDs with audit events. Copilot hooks do
not end this application task when its process or MCP transport exits. The
task owner explicitly requests Capture and ends the Session when finished.

## Evidence and remaining qualification

Authored tests exercise the hook process against the shared mock HTTP gateway:
authentication, stable namespaced task identity, start/resume, another task's
start, fresh context requests, denied/empty context, outages, gateway/client
isolation, missing login and malformed/oversized input. These verify adapter
behaviour, not actual Cedar/RLS enforcement or native model consumption.

On 2026-09-12, all 121 tests across Claude Code, Codex and Copilot passed with zero skips on
macOS arm64 Node 24.18.0 and offline Linux arm64 Docker Node 22.23.2. Eight are
the new Copilot contract cases. The Docker run uses the pinned image
`node@sha256:7725a5c2c83eed1d36258c66efae14b1ceccd021db9ed1d9559d3335ed3d68ed`,
an ordinary UID, a read-only checkout and an executable temporary filesystem
for synthetic CLI test stubs. Formatting, generated support, documentation,
backlog, ADR and dependency gates pass. The existing plugin archive also passes
its eight extracted Codex regression tests with the changed shared runtime;
the archive contains no Copilot runtime yet. On 2026-09-13, all twelve Copilot
tests passed on those same host/Docker runtimes, with zero skips. These include
the authentic start payload, native event correlations, fixture hashes and the
negative marker outcome. The existing entry point accepted the captured
`source: "new"` frame without production-code changes. No Rust code changed.

Native local checks used only an owned scratch checkout and isolated
`COPILOT_HOME`. `copilot mcp add ... --json` produced the expected local command
entry, and `copilot skill list --json` listed the synthetic project Skill as
enabled. No account credential or user Skill contents are captured here.

The user-approved native probe ran at 2026-09-13T07:00:16Z on CLI 1.0.83,
auto-routed to `gpt-5.6-luna`, with a saved 30-credit Session limit. It exited
zero in 9.17 seconds and reported one premium request. That counter is retained
as reported, not converted into a monetary price. Remote export and the Synveda
MCP server were disabled for this synthetic boundary check; no Synveda login or
public-API lifecycle was exercised by the native client.

The [captured lifecycle](../../adapters/copilot-cli/fixtures/lifecycle.json)
records `userPromptSubmitted`, `sessionStart` (`source: "new"`), `preToolUse`,
`postToolUse`, `agentStop` and `sessionEnd` (`reason: "complete"`). Its native
`hook.end` confirms successful acceptance of top-level `additionalContext`.
The [transcript projection](../../adapters/copilot-cli/fixtures/transcript.jsonl)
retains stable native event IDs, user/assistant text, the correlated tool pair
and `skill.invoked` with `trigger: "agent-invoked"`. Private path prefixes are
replaced, and hidden reasoning, encrypted content, system instructions and
unrelated diagnostics are omitted with the exact projection declared in the
lifecycle file. Registry digests and replay tests check both files.

The marker assertion **failed**. A leftover synthetic project Skill instructed
the model to return `SYNVEDA_COPILOT_SKILL_OK`, which it did after invoking the
native `skill` tool. That differs from the context hook's expected marker.
The empty `--available-tools=` flag did not prevent this tool invocation. This
is useful evidence that native Skill activation is observable, but it does not
attribute that Skill to an approved Synveda binding/version or verify hook
context consumption. A successful process exit does not override the failure.

The two owned synthetic Skills have been removed from the scratch project.
A prepared retry uses a fresh unpredictable hook marker, excludes the Skill
tool, denies tool execution and resumes the same native Session under its saved
30-credit limit. Automatic approval review rejected that retry as another
billable request outside the one-probe approval; **the retry did not run**.
After additional approval, require the exact fresh marker and the same Session
ID before translating further lifecycle seams. Next qualify observations, outage recovery, compaction and
approved Skill usage with the same SDK task against the ordinary Docker/Keycloak
public edge. Require cross-workspace denial and persisted audit correlation,
Capture, explicit end and Knowledge reuse before registry promotion. Vendor
`preCompact` is notification-only; no post-compaction reinjection is claimed.
Only then extend the existing release archive and installation checks.

Full CI, the database suite, live Compose acceptance, packaged installation and
complete native lifecycle qualification were not run for this evidence increment.
