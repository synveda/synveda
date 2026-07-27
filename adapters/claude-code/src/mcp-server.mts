#!/usr/bin/env node
/**
 * The MCP entry point (CTX-5, ADR-0042 decision 15) — the third artifact
 * in this package beside the hook and the driver, named in
 * `.claude-plugin/plugin.json` as `mcpServers.synveda`.
 *
 * Thin on purpose: the protocol lives in `mcp.mts` so it can be driven
 * frame by frame from a test without a process. All this file owns is the
 * shebang, the stdio loop, and the guarantee that a crash here does not
 * take the user's client with it.
 */

import { main } from "./mcp.mjs";
import { log } from "./log.mjs";

process.on("uncaughtException", (error: unknown) => {
  log("mcp.uncaught", { error: String(error) });
  process.exit(1);
});
process.on("unhandledRejection", (reason: unknown) => {
  log("mcp.unhandled_rejection", { reason: String(reason) });
  process.exit(1);
});

await main();
