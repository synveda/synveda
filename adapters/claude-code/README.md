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
`PreCompact` has no context-injection output and its only decision
control is exit 2, which blocks compaction. Re-injection after a
compaction is `SessionStart` firing again with `source: "compact"`
(ADR-0027 decision 2).

Every hook exits 0, always. A dead gateway, an expired login, a
malformed transcript, or an expired deadline yields a hook that
contributes no context and returns success.

## Install

```sh
pnpm install && pnpm --filter @synveda/claude-code-adapter build
```

Then point Claude Code at this directory as a plugin, and give the hooks
a bearer:

```sh
export SYNVEDA_TOKEN="$(synveda token issue --tenant "$TENANT" --subject "$SUBJECT")"
export SYNVEDA_GATEWAY=http://127.0.0.1:8120   # optional; this is the default
```

`SYNVEDA_TOKEN` is the step-1 seam. Step 2 replaces it with `synveda
login` and a credentials file the CLI refreshes — see
[`src/credentials.mts`](src/credentials.mts) and ADR-0027 decisions 4
to 6. The MCP recall tool is CTX-5/ADPT-2 and lands in this same
manifest as `mcpServers`.

## Configuration

Environment (highest precedence):

- `SYNVEDA_DISABLED=1` — the adapter does nothing at all
- `SYNVEDA_GATEWAY` — gateway base URL
- `SYNVEDA_TOKEN` — bearer for `/v1` (step 1 only)
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

## Files it writes

Nothing inside your project.

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

Unit tests cover the transcript parser, the event mapping, and the
spool; the handler suite runs both hook paths against a mock gateway,
and the last few cases spawn the built entry point to prove exit 0 on
every failure. Tests compile alongside the source into `dist/`.
