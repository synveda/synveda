#!/usr/bin/env node
/**
 * The hook entry point (ADR-0027 decisions 1, 2 and 3).
 *
 * One artifact for all four events, dispatched by the payload's own
 * `hook_event_name` and cross-checked against the mode named in
 * `hooks/hooks.json`.
 *
 * It exits 0 unconditionally. A memory system that can break the user's
 * session is worse than one that occasionally has no memory, so there is
 * no path here that returns a non-zero status, and none that emits a
 * blocking decision — exit 2 on `PreCompact` would block compaction
 * itself.
 *
 * Nothing but the hook result is ever written to stdout: for
 * `SessionStart`, stdout is context the model reads.
 */

import { randomUUID } from "node:crypto";
import { closeSync, openSync, writeSync } from "node:fs";
import { isAbsolute, join } from "node:path";

import { loadConfig } from "./config.mjs";
import { turn } from "./turn.mjs";
import { diagnostic, log } from "./log.mjs";
import { ensureDir } from "./paths.mjs";
import { sessionStart } from "./session-start.mjs";
import { syncSkills } from "./skills.mjs";
import type { HookInput, HookOutput } from "./types.mjs";

/**
 * A hard ceiling on the whole hook, above the per-call deadline: if anything
 * hangs that the request deadline does not cover, the process still leaves.
 *
 * Above `SessionEnd`'s own flush budget, so the deadline that fires first is
 * the flush's — a watchdog that beat it would kill the process between the
 * append and the spool write, which is the one window where an acknowledgement
 * can be lost.
 */
const WATCHDOG_MS = 10_000;

type Mode = "start" | "turn" | "skills" | "none";

process.on("uncaughtException", (error: unknown) => {
  log("hook.uncaught", { error: diagnostic(error) });
  process.exit(0);
});
process.on("unhandledRejection", (reason: unknown) => {
  log("hook.unhandled_rejection", { reason: diagnostic(reason) });
  process.exit(0);
});

const watchdog = setTimeout(() => {
  log("hook.watchdog", { ms: WATCHDOG_MS });
  process.exit(0);
}, WATCHDOG_MS);
watchdog.unref();

await main();

async function main(): Promise<void> {
  const hookStarted = Date.now();
  const input = await readInput();
  // Nothing parseable arrived at all. The mode argument alone would be
  // enough to go on, and that is exactly the trap: without a payload
  // there is no session id to correlate by and no `cwd` to read the
  // project's own configuration from — so a project that turned the
  // adapter off would be captured anyway (decision 13). A hook with no
  // input does nothing.
  if (input === undefined) {
    log("hook.no_payload", { argv: process.argv[2] });
    return;
  }
  const mode = resolveMode(process.argv[2], input.hook_event_name);
  if (mode === "none") {
    log("hook.unrecognised", { argv: process.argv[2], hook: input.hook_event_name });
    return;
  }

  const config = loadConfig(input.cwd);
  if (config.disabled) {
    log("hook.disabled", { hook: input.hook_event_name });
    return;
  }

  try {
    if (mode === "skills") {
      // Writes a directory and no context. It emits nothing at all, which
      // is not an omission: this entry is `async` precisely so that the
      // session never waits on it, and a hook that returned context from
      // there would be racing the one that does (SKIL-4, ADR-0054
      // decision 18).
      if (config.skills) await syncSkills();
      else log("skills.disabled", {});
    } else {
      emit(mode === "start" ? await sessionStart(input, config) : await turn(input, config));
    }
  } catch (error) {
    log("hook.failed", { hook: input.hook_event_name, error: diagnostic(error) });
  }
  log("hook.done", {
    hook: input.hook_event_name,
    mode,
    elapsed_ms: Date.now() - hookStarted,
  });
}

/**
 * The payload is ground truth — it says which event actually fired. The
 * argument from `hooks.json` selects which of the two SessionStart handlers is
 * registered and cross-checks every other registration.
 *
 * Since SKIL-4 there is one exception, and it is the argument's first
 * load-bearing use: **two entries ride `SessionStart`** — the inject that
 * returns context and the skills sync that writes a directory — so for
 * `skills` the argument decides and the event only cross-checks it. Any
 * other event carrying that argument is a misconfiguration, and doing
 * nothing is the right answer to one.
 */
function resolveMode(argument: string | undefined, event: string | undefined): Mode {
  if (argument === "skills" && event === "SessionStart") return "skills";
  if (argument === "session-start" && event === "SessionStart") return "start";
  if (
    argument === "turn" &&
    (event === "Stop" || event === "PreCompact" || event === "SessionEnd")
  ) {
    return "turn";
  }
  return "none";
}

/** The hook payload, or `undefined` when none arrived that this can read. */
async function readInput(): Promise<HookInput | undefined> {
  // No stdin to speak of (a human running the binary by hand): there is
  // nothing to do, and blocking on a terminal would hang the watchdog out.
  if (process.stdin.isTTY === true) return undefined;
  let raw = "";
  try {
    process.stdin.setEncoding("utf8");
    for await (const piece of process.stdin) raw += String(piece);
  } catch (error) {
    log("hook.stdin_failed", { error: diagnostic(error) });
    return undefined;
  }
  try {
    const parsed: unknown = JSON.parse(raw);
    if (parsed !== null && typeof parsed === "object" && !Array.isArray(parsed)) {
      captureInput(raw, parsed as HookInput);
      return parsed as HookInput;
    }
  } catch {
    // The harness sent something this version does not understand
    // (decision 9): do nothing, quietly, and let the session proceed.
  }
  log("hook.stdin_unparsed", { bytes: raw.length });
  return undefined;
}

/**
 * Opt-in raw-frame capture for the separately run CPR-14 live gate.
 *
 * Off unless an absolute scratch directory is named. The gate needs the real
 * client's bytes to update a replay corpus when Claude's private protocol
 * moves; writing them into the project or normal logs would turn a diagnostic
 * facility into a second transcript store. Files are private and the live
 * runner removes the whole scratch tree after producing its redacted report.
 */
function captureInput(raw: string, input: HookInput): void {
  const directory = process.env.SYNVEDA_CAPTURE_DIR;
  if (directory === undefined || !isAbsolute(directory)) return;
  try {
    ensureDir(directory);
    const event =
      typeof input.hook_event_name === "string"
        ? input.hook_event_name.replace(/[^A-Za-z0-9._-]/g, "_")
        : "unknown";
    const name = String(Date.now()) + "-" + event + "-" + randomUUID() + ".json";
    const path = join(directory, name);
    const handle = openSync(path, "wx", 0o600);
    try {
      writeSync(handle, raw);
    } finally {
      closeSync(handle);
    }
    log("fixture.captured", { event, bytes: Buffer.byteLength(raw) });
  } catch (error) {
    log("fixture.capture_failed", { error: diagnostic(error) });
  }
}

function emit(output: HookOutput): void {
  if (Object.keys(output).length === 0) return;
  process.stdout.write(JSON.stringify(output));
}
