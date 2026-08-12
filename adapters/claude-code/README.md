# Claude Code adapter (ADPT-1)

A Claude Code plugin that gives a session governed memory: it composes a
context block at session start and observes the transcript as the
session runs. Design and rationale: [ADR-0027](../../docs/adr/adr-0027-claude-code-adapter.md).

The adapter decides nothing. It maps hook events to the two `/v1`
primitives with the caller's own bearer, and inherits whatever the PDP
allows that identity (seed §2.2).

## The seams

| Hook | Mode | What it does |
| --- | --- | --- |
| `SessionStart` | `session-start` | `POST /v1/inject`; returns the block as `additionalContext` |
| `SessionStart` | `skills` | `synveda skill sync` into this plugin's own `skills/`; async, returns nothing |
| `Stop` | `observe` | `POST /v1/observe` with the turn's transcript delta |
| `PreCompact` | `flush` | Resends whatever a previous flush left behind |
| `SessionEnd` | `flush` | The same retry, plus spool pruning |

`SessionStart` is the only one of the four that can contribute context —
`PreCompact`'s output becomes compaction instructions and its only
decision control is exit 2, which blocks compaction. Re-injection after a
compaction is `SessionStart` firing again with `source: "compact"`
(ADR-0027 decision 2).

Every hook exits 0, always. A dead gateway, an expired login, a
malformed transcript, or an expired deadline yields a hook that
contributes no context and returns success.

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
synveda login --gateway http://127.0.0.1:8120
```

**Why a marketplace and not this directory.** Until 2026-08-11 this file said
"point Claude Code at this directory as a plugin", and `demos/adpt-1-claude-code.sh`
copies three directories into `~/.claude/plugins/synveda/`. Claude Code reads
neither. It installs plugins from a *marketplace* — a directory carrying
`.claude-plugin/marketplace.json` — into a cache it owns, tracked in
`known_marketplaces.json` and `installed_plugins.json`. `package-plugin.sh`
builds that wrapper; `synveda plugin install` hands it to `claude plugin`.

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
where it is silently ignored. This plugin shipped with both mistakes for a
year (ADR-0027 amendment, 2026-08-11); `package-plugin.sh` now refuses to
build a bundle that reintroduces either.

That is the whole configuration. `synveda login` opens your browser at
the *gateway's* `/auth/login` — never the IdP's directly — so the login
runs AUTH-1 end to end: PKCE, JWKS verification, tenant resolution, and
JIT provisioning. What comes back to the CLI's loopback listener is a
one-time code, not a token; the CLI redeems it over a POST and writes
`$XDG_CONFIG_HOME/synveda/credentials.json` (mode 0600). The hooks then
call `synveda auth token --json` for a currently-valid bearer, and the
CLI refreshes it through the gateway when it expires. The adapter holds
no OAuth code of its own (ADR-0027 decisions 4 to 6).

Since CTX-4 the composed block may carry **index entries**: material the
budget could not fit, named rather than dropped, each ending with a
`(recall <id>)` handle and preceded by one line saying what that means.
Nothing here had to change for it — the hook passes the block's text
verbatim — and an agent navigates from a handle to the body by running
`synveda recall <id>`, which is on `PATH` already because the same binary
issues this plugin's bearer.

### The MCP tool

The `mcpServers` slot of the same manifest gives the model a `recall`
tool, so it can reach past the block it was handed and ask the corpus a
question of its own.

The protocol behind it is **not in this package**. CTX-5 hand-wrote a
JSON-RPC loop here to keep the plugin dependency-free; ADR-0042 option 8
recorded what would reverse that — "protocol revisions churn, or a second
transport" — and `2026-07-28` did, replacing the negotiation handshake
with per-request `_meta`, making `server/discover` mandatory, and adding
`-32022`. Since ADR-0057 decision 4 the server is `synveda mcp`, one
implementation shared with Claude Desktop and Cursor, and
`dist/mcp-server.mjs` is a ~40-line launcher that resolves the binary the
way the credential seam does (`SYNVEDA_CLI`, else `synveda` on `PATH`),
hands it the client's own stdio, and — if the CLI is missing — says so
instead of coming up with an empty tool list.

It launches `synveda mcp --writes host`, hard-coded. This plugin's `Stop`
hook already POSTs the turn to `/v1/observe`, so a `remember` tool here
would let the model store a fact by tool call while the hook independently
observes the transcript containing it: two rows, same scope, different
payloads, and nothing downstream able to tell they were one turn
(ADR-0057 decision 6). There is no configuration of this plugin under
which the other value is right, so there is no flag for it.

### Governed skills

Since SKIL-4 a second `SessionStart` entry reconciles this plugin's own
`skills/` directory with what the registry publishes to your identity:
it writes every skill on your placement chain that policy lets you read,
and **removes** the ones it no longer serves you. That removal is what
makes a FLOW-7 rollback, or a move between teams, reach a laptop.

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
- `SYNVEDA_TIMEOUT_MS` — per-call deadline, default 3000

Per project, optional, at `.synveda/config.json`:

```json
{
  "disabled": false,
  "inject": true,
  "observe": true,
  "skills": true,
  "gateway_url": "http://127.0.0.1:8120",
  "timeout_ms": 3000,
  "budget_tokens": 4000,
  "compact_budget_tokens": 1500
}
```

A budget narrows and never widens: the effective budget is
`min(pack budget, this)` (ADR-0026 decision 7).

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
- `$XDG_STATE_HOME/synveda/sessions/` — per-session cursor and
  transcript path (ADR-0027 decision 7)
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

Observe is at-least-once over a durable cursor. The cursor — the uuid of
the last transcript entry a gateway 2xx accepted — advances only on
success, so a failed batch is resent by the next hook, where MEM-1's
buffer reports the overlap as duplicates and re-enqueues nothing
(ADR-0020 decision 2). No daemon, no local queue.

## Tests

```sh
pnpm --filter @synveda/claude-code-adapter test
```

Unit tests cover the transcript parser, the event mapping, the spool,
and the credential seam (against a stand-in CLI that refuses, hangs,
prints garbage, or is not installed — all of which resolve to "no
memory this time"); the handler suite runs both hook paths against a
mock gateway, and several cases spawn the built entry point to prove
exit 0 on every failure. Tests compile alongside the source into
`dist/`.

### The recorded-payload driver

`fixtures/` holds real hook payloads and a real session transcript
(shapes recorded from Claude Code 2.1.220, content synthetic), and
`dist/driver.mjs` replays them through the built entry point as a child
process — the same `node dist/hook.mjs <mode>` line `hooks/hooks.json`
registers. Sixteen cases: dead gateway, 401, degraded header, oversized
tool result, replayed batch, cursor resume after a failed flush, damaged
transcript line, unreadable payload. Every one must exit 0.

```sh
node dist/driver.mjs                                    # mock gateway
node dist/driver.mjs --gateway URL --token BEARER       # live gateway
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

A clean HOME, the prebuilt plugin, `synveda login` against live Rauthy,
and a session that receives its watermarked block and contributes its
turn back — timed against ADPT-1's two-minute budget, then joined in one
verifying audit chain by session id.
