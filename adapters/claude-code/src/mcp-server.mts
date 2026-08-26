#!/usr/bin/env node
/**
 * The MCP entry point (ADPT-2, ADR-0057 decision 4) — named in `.mcp.json`
 * at the plugin root, which Claude Code discovers on its own.
 *
 * It was named in `.claude-plugin/plugin.json` as `mcpServers.synveda` until
 * 2026-08-11, where Claude Code **silently ignores it**: the component
 * inventory read `MCP servers (0)` and this file was never spawned. See the
 * ADR-0027 amendment.
 *
 * # There is no protocol here any more
 *
 * CTX-5 wrote the JSON-RPC loop out by hand in `mcp.mts`, to protect
 * ADR-0027 decision 1's no-bundler, no-install-step constraint. ADR-0042
 * option 8 recorded what would reverse that — "protocol revisions churn,
 * or a second transport" — and it has: `2026-07-28` replaced the
 * negotiation handshake with per-request `_meta`, made `server/discover`
 * mandatory, and added `-32022`. The loop that file held is two revisions
 * behind, and a second implementation of one protocol is a second place
 * for the `ids` xor `query` rule to drift. So it is gone, and `synveda
 * mcp` serves this plugin exactly as it serves Claude Desktop and Cursor.
 *
 * All that is left is the exec. This file copies no bytes and parses no
 * frames: `stdio: "inherit"` hands the client's own descriptors to the
 * server, so the two talk through the kernel and nothing in between can
 * mangle a frame or add a revision of its own.
 *
 * # Why a launcher at all, rather than naming the binary in the manifest
 *
 * Decision 4 asks for two things a static `"command": "synveda"` cannot
 * both do: resolve the binary **the same way the credential code does**,
 * and *fail with a message* rather than silently serve no tools. That
 * resolution is `SYNVEDA_CLI ?? "synveda"` (`credentials.mts`), and
 * `plugin.json` is JSON — it cannot express a fallback. The variable is
 * not decoration: this package's tests and `demos/ctx-5-recall.sh` point
 * it at a build in the working tree, so a manifest hard-wired to a bare
 * `synveda` would quietly exercise whichever one was on the developer's
 * PATH.
 *
 * So the manifest still names this file, and this file is the resolution
 * plus the message. What decision 4 was deleting is the protocol, and the
 * protocol is what left.
 *
 * # `--writes host` is not a default; it is the point
 *
 * ADR-0057 decision 6: this plugin's `Stop` hook already records the turn for
 * durable delivery to session events. A `remember` tool advertised here
 * would let the model
 * store a fact by tool call while the hook independently observes the
 * transcript containing it — two rows in the same home scope, different
 * payloads, different idempotency keys, so ADR-0020 decision 2's
 * buffer-level idempotency cannot see the duplication. It is hard-coded
 * rather than configurable because there is no configuration of this
 * plugin under which the other value is right, and an environment
 * variable would only be a supported way to corrupt a personal corpus.
 */

import { spawn } from "node:child_process";

import { diagnostic, log } from "./log.mjs";

/**
 * What the user is told when the CLI is not there. The plugin already
 * depends on this binary for its bearer (ADR-0027 decision 4), so this is
 * one prerequisite failing in a second place — and it is said out loud,
 * because an empty tool list is indistinguishable from having nothing to
 * recall.
 */
const MISSING_BINARY_MESSAGE =
  "Synveda: the `synveda` CLI was not found, so governed memory is unavailable in " +
  "this session. Install it and run `synveda login` — see docs/INSTALL.md.";

/** The credential seam's own resolution (`credentials.mts`), verbatim. */
const binary = process.env.SYNVEDA_CLI ?? "synveda";

/** Decision 6: a host that already observes its own turns takes no write tool. */
const child = spawn(binary, ["mcp", "--writes", "host"], {
  stdio: "inherit",
  windowsHide: true,
});

child.on("error", (error: unknown) => {
  log("mcp.spawn_failed", { binary, error: diagnostic(error) });
  // stderr, never stdout: stdout is the client's half of a protocol this
  // process no longer speaks, and a stray line on it is a parse error at
  // the far end rather than a message anyone reads.
  process.stderr.write(`${MISSING_BINARY_MESSAGE}\n`);
  process.exit(1);
});

// The client stops this server by killing it. Without forwarding, the
// server this spawned would outlive the client still holding its stdin.
for (const signal of ["SIGINT", "SIGTERM"] as const) {
  process.on(signal, () => {
    child.kill(signal);
  });
}

child.on("exit", (code: number | null, signal: NodeJS.Signals | null) => {
  if (signal !== null) {
    log("mcp.stopped", { signal });
    process.exit(1);
  }
  process.exit(code ?? 0);
});
