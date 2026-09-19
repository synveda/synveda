# Claude Code adapter (ADPT-1)

A Claude Code plugin that gives a session governed memory: it composes a
context block at session start and records the transcript as the session runs.
Design and rationale: [ADR-0027](../../docs/adr/adr-0027-claude-code-adapter.md),
re-cut onto the session API by
[ADR-0078](../../docs/adr/adr-0078-durable-session-delivery.md). The
installed-client and deterministic replay evidence tiers are defined by
[ADR-0079](../../docs/adr/adr-0079-live-claude-session-acceptance.md).

The adapter decides nothing. It maps hook events to the session plane with the
caller's own bearer, and inherits whatever the PDP allows that identity
(seed §2.2).

## The seams

| Hook | Mode | What it does |
| --- | --- | --- |
| `SessionStart` | `session-start` | Opens or resumes the run, retries the backlog, `POST /v1/sessions/{id}/context-runs`; returns the block as `additionalContext` |
| `SessionStart` | `skills` | `synveda skill sync` into this plugin's own `skills/`; async, returns nothing |
| `Stop` | `turn` | Synchronously records the turn into the spool, then returns before credential or network work |
| `PreCompact` | `turn` | Synchronously records everything the transcript still holds, then returns before compaction rewrites it |
| `SessionEnd` | `turn` | Records the last turn, then a **bounded** synchronous flush, then closes the run |

`SessionStart` is the only one of the four that can contribute context —
`PreCompact`'s output becomes compaction instructions and its only
decision control is exit 2, which blocks compaction. Re-injection after a
compaction is `SessionStart` firing again with `source: "compact"`
(ADR-0027 decision 2).

Every hook exits 0, always. A dead gateway, an expired login, a
malformed transcript, or an expired deadline yields a hook that
contributes no context and returns success — **and records the events
anyway**.

## Install

From an installed release, one command — it carries this plugin already:

```sh
synveda plugin install          # --dry-run to see what it would run
```

From a checkout, build it, wrap it as a marketplace, and install that:

```sh
pnpm install && pnpm --filter @synveda/claude-code-adapter build
scripts/package-plugin.sh 0.1.0 /tmp/synveda-plugin
synveda plugin install --from /tmp/synveda-plugin/plugin
```

Then log in once:

```sh
export SYNVEDA_GATEWAY=http://app.synveda.test:8080
synveda login --gateway "$SYNVEDA_GATEWAY"
```

That URL is the canonical development Compose gateway. Use the deployment's
application URL instead for reference or external deployments; use
`http://127.0.0.1:8120` only when deliberately running the gateway binary
directly.

**Why a marketplace and not this directory.** Claude Code installs plugins
from a marketplace carrying `.claude-plugin/marketplace.json` into its own
cache. `package-plugin.sh` builds that wrapper; `synveda plugin install` hands
it to `claude plugin`.

Check it actually loaded, because installing and loading are different things:

```sh
claude plugin list                  # Status: ✔ enabled
claude plugin details synveda@synveda   # Hooks (4) … MCP servers (1)
```

Two manifest keys are why that check matters. `hooks` must **not** name
`./hooks/hooks.json` — the file is read automatically and declaring it too is
a duplicate-load error that leaves the plugin `✘ failed to load` with the
install looking perfectly healthy. And the MCP server belongs in `.mcp.json`
at this directory's root, **not** as an inline `mcpServers` in `plugin.json`,
where it is silently ignored. `package-plugin.sh` refuses a bundle that
violates either constraint.

That is the whole configuration. `synveda login` opens your browser at
the *gateway's* `/auth/login` — never the IdP's directly — so the login
runs AUTH-1 end to end: PKCE, JWKS verification, tenant resolution, and
JIT provisioning. What comes back to the CLI's loopback listener is a
one-time code, not a token; the CLI redeems it over a POST and writes
`$XDG_CONFIG_HOME/synveda/credentials.json` (mode 0600). The hooks then
call `synveda auth token --json` for a currently-valid bearer, and the
CLI refreshes it through the gateway when it expires. The adapter holds
no OAuth code of its own (ADR-0027 decisions 4 to 6).

The composed block is passed through verbatim, preceded by the resolved
Synveda Session ID. That identifier lets MCP calls use this application task.

The injected context block is budgeted. For deeper MCP recall, pass the
`session_id` supplied in that context; the tool reads the same authorised
Session and calls `POST /v1/sessions/{session_id}/knowledge-query`.
The standalone `synveda recall --query` command owns its separate Session.
There is no global
`/v1/recall` route or direct Knowledge-by-ID fetch tool.

### The MCP tool

The plugin's `.mcp.json` gives the model a `recall` tool, so it can reach past
the composed block and ask the governed corpus a question of its own. The
protocol implementation is the shared `synveda mcp` command used by supported
clients. `dist/mcp-server.mjs` is a thin launcher: it resolves `SYNVEDA_CLI` or
`synveda` on `PATH`, passes through the client's stdio, and reports a missing
CLI instead of starting with an empty tool list.
The launcher forwards workspace, project and credential profile settings.
It does not infer a task from its transport connection: multiple conversations
can use the server with their own explicit Synveda Session IDs.

It launches `synveda mcp --writes host`, hard-coded. This plugin's `Stop`
hook already records the turn as session events, so a `remember` tool here
would let the model store a fact by tool call while the hook independently
records the transcript containing it: two rows, same run, different
payloads, and nothing downstream able to tell they were one turn
(ADR-0057 decision 6). There is no configuration of this plugin under
which the other value is right, so there is no flag for it.

### Governed skills

A second `SessionStart` entry reconciles this plugin's own `skills/` directory
with approved enabled bindings at the configured project's scope, otherwise
the authenticated principal's scope. It supplies that exact `--scope` to the
existing CLI sync. A refused explicit project never falls back to a different
scope. Successful sync writes the selected immutable versions and removes
owned installations no longer advertised at that distribution scope.

It writes into `${CLAUDE_PLUGIN_ROOT}/skills/` and never into
`~/.claude/skills/` — a reconcile prunes, and the only directory this
product may prune is one it created. Your own skills folder stays yours;
`synveda skill install <name>` is still how you put a governed bundle
there by hand, and a skill installed both ways exists twice, with the
client's own precedence deciding which loads.

The entry is `async: true` and does no work on the inject path. A client
reads its skills folder when it starts, so what a session syncs is
loaded by the *next* one — which is why the composed block names the
skills available to you as well: the block is current where the folder
is one session behind. `synveda skill available` is the same list from a
terminal.

### Without a login

`SYNVEDA_TOKEN` overrides the CLI entirely — for CI, for demos, and for
the HS256 dev bearer (ADR-0008):

```sh
export SYNVEDA_TOKEN="$(synveda token issue --tenant "$TENANT" --subject "$SUBJECT")"
export SYNVEDA_GATEWAY=http://127.0.0.1:8120   # optional; this is the default
```

## Configuration

Environment (highest precedence):

- `SYNVEDA_DISABLED=1` — the adapter does nothing at all
- `SYNVEDA_GATEWAY` — gateway base URL
- `SYNVEDA_TOKEN` — bearer for `/v1`, bypassing the CLI
- `SYNVEDA_PROFILE` — which credential profile to use, default `default`
- `SYNVEDA_CLI` — path to the `synveda` binary, default from `PATH`
- `SYNVEDA_WORKSPACE` — the workspace runs belong to. Optional: with one
  workspace the adapter asks `/v1/me` and takes the answer; with more than one
  it needs telling, because guessing would put one team's transcript in
  another team's scope
- `SYNVEDA_PROJECT` — the project runs belong to. Optional, but it must be
  explicit when project-scoped context is required; project list order is not
  an identity
- `SYNVEDA_TIMEOUT_MS` — per-call deadline, default 3000

Per project, optional, at `.synveda/config.json`:

```json
{
  "disabled": false,
  "inject": true,
  "observe": true,
  "skills": true,
  "gateway_url": "http://127.0.0.1:8120",
  "workspace_id": "0198e4c1-0000-7000-8000-000000000001",
  "project_id": "0198e4c1-0000-7000-8000-000000000002",
  "timeout_ms": 3000,
  "budget_tokens": 4000,
  "compact_budget_tokens": 1500
}
```

A budget narrows and never widens: the effective budget is
`min(pack budget, this)` (ADR-0026 decision 7).

`workspace_id` and `project_id` are safe to set in a checked-out repository, unlike
`gateway_url`: naming a workspace inside a tenant you are already
authenticated to cannot redirect a credential anywhere.

`gateway_url` here applies only when no `synveda login` credential is in
play. A credential names the gateway it was issued for and that one
wins — this file lives inside a checked-out repository, and a
`gateway_url` in it must not be able to send your bearer to a host of
the repository's choosing.

## Files it writes

Nothing inside your project.

- `$XDG_CONFIG_HOME/synveda/credentials.json` — written by `synveda
  login`, mode 0600, keyed by profile. `synveda auth logout` removes it;
  the tokens themselves stay valid at the issuer until they expire
- `$XDG_STATE_HOME/synveda/spool/` — the durable spool: one file per
  conversation, holding every recorded event and whether this deployment has
  it (ADR-0078 decision 6). `synveda session spool status` reads it
- `$XDG_CONFIG_HOME/synveda/installation-id` — a random id for this
  installation, so two machines running this client can be told apart. Not a
  hostname and not a username
- `$XDG_STATE_HOME/synveda/adapter.log` — diagnostics, JSON lines.
  Never stdout: for `SessionStart`, stdout is context the model reads
- `$XDG_STATE_HOME/synveda/disclosed/` — the one-shot per-project
  disclosure marker
- `$XDG_CONFIG_HOME/synveda/skills/claude-code/<name>.json` — one install
  receipt per governed skill, written *outside* the bundle because a file
  no reviewer approved inside a directory a client walks is a
  modification (ADR-0051 option 7). It is also the record of what this
  product wrote, which is what bounds what a sync may remove
- `${CLAUDE_PLUGIN_ROOT}/skills/<name>/` — the governed bundles
  themselves, byte-identical to the reviewed commit and non-executable

## Delivery

**Record first, deliver later.** Stop and PreCompact copy the transcript delta
into a local spool, `fsync` it and return without resolving a credential or
contacting the gateway. This synchronous local boundary survives successful
headless teardown without putting gateway latency into an interactive turn.
SessionEnd performs a bounded flush; the next `SessionStart` or `synveda
session flush` delivers anything it could not acknowledge. An unreachable
gateway, an expired login, a compaction or a reboot therefore costs no event
which reached the spool.

Delivery is idempotent per event. Each event carries the transcript entry's own
uuid as its `client_event_id`, so a redelivered batch that overlaps a previous
one appends only what is new and comes back `duplicate` for the rest, at their
original positions.

Three commands make the spool something you can act on:

```sh
synveda session spool status                # what is held, and since when
synveda session flush                       # deliver it now
synveda session spool purge --acknowledged  # reclaim the disk
```

`purge` deletes only events this deployment has already answered for, and the
flag is required rather than assumed. There is no flag that deletes
undelivered ones.

### The event-loss boundary

If Claude Code terminates without running **any** lifecycle hook — `kill -9`, a
harness crash, a machine losing power mid-turn — the events of the turn in
flight are lost. They were never handed to a hook, so no code in this adapter
ever saw them.

This is not fixable from inside a hook contract, and the two ways to narrow it
were both rejected with reasons that still hold: a background daemon watching
the transcript file is a second thing to install, supervise and debug, and it
would observe projects whose hooks are disabled (ADR-0027 decision 1); and
recording from `PreToolUse`/`PostToolUse` would put this adapter in the path of
every tool call, which is a latency budget it should not be spending.

What the design does guarantee is that **everything a hook has been handed is
durable before delivery is attempted**. The boundary is therefore "the turn in
flight when the client died", not "everything since the gateway went down".

## Tests

```sh
pnpm --filter @synveda/claude-code-adapter test
```

Unit tests cover the transcript parser, the event mapping, the durable spool,
and the credential seam (against a stand-in CLI that refuses, hangs, prints
garbage, or is not installed — all of which resolve to "no memory this time");
the handler suite runs both hook paths against a mock gateway and asserts what
the spool holds after each failure, not only that nothing crashed. Several
cases spawn the built entry point to prove exit 0 on every failure. Tests
compile alongside the source into `dist/`.

### The recorded-payload driver

`fixtures/` holds genuine captured hook payloads and session transcripts
(Claude Code 2.1.220 and 2.1.241, private content and paths replaced), and
`dist/driver.mjs` replays them through the built entry point as a child
process — the same `node dist/hook.mjs <mode>` line `hooks/hooks.json`
registers. `fixtures/manifest.json` binds every byte to exact client version,
capture provenance, sanitisation and SHA-256; the fixture schema and denylist
are checked in every adapter test. Fifteen cases: dead gateway, degraded header, refused composition,
a turn kept on disk when nothing could be delivered, the next start draining
that backlog, a redelivered batch answered `duplicate`, a compaction that
cannot eat a turn, a close owed over a backlog, a damaged transcript line, an
unreadable payload, and a stale hook argument. Every one must exit 0.

```sh
node dist/driver.mjs                                                  # mock gateway
node dist/driver.mjs --gateway URL --token BEARER --workspace ID --project ID
```

The mock run is part of `npm test`. The live run is the last section of
`demos/adpt-1-claude-code.sh`, and it earns its keep: the mock cannot
tell you that a payload the adapter could not parse still injected,
because a mock is only ever asked what the client asks it.

The gateway half of the login lives in
`crates/synveda-gateway/tests/cli_login.rs`: the loopback allowlist, the
single-use state-bound handoff code, the refresh grant, and the rule
that no token ever appears in a redirect URL.

## The acceptance demo

```sh
demos/adpt-1-claude-code.sh
```

The deterministic gate replays authentic captured frames through a live
gateway and verifies that a session receives its watermarked block and
contributes its turn back within ADPT-1's two-minute budget. The resulting
events join the run's verifying audit chain.

CPR-14 adds the current session-plane acceptance targets:

```sh
make claude-acceptance       # authentic frames, real gateway/PDP/Postgres; CI
make claude-acceptance-live  # installed marketplace + real authenticated client
```

The first is always labelled replay. The second's runner exits 77 when the
executable or authentication is unavailable (`make` surfaces that as recipe
`Error 77`). On 2026-08-24 installed authenticated Claude Code **2.1.241**
passed with plugin **0.2.0**: four hooks and one MCP server enabled, one context
run, four ordered user/tool/assistant events and a normal ended session. Replay
remains distinct and does not stand in for future live-client versions.
