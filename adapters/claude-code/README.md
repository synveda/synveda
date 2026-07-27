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

```sh
pnpm install && pnpm --filter @synveda/claude-code-adapter build
```

Then point Claude Code at this directory as a plugin, and log in once:

```sh
synveda login --gateway http://127.0.0.1:8120
```

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

The MCP recall tool is CTX-5/ADPT-2 and lands in this same manifest as
`mcpServers`.

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
