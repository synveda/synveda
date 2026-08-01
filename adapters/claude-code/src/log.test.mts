/**
 * The log is where every failure this adapter swallows goes, so the one
 * thing it must not do is lose the name of what happened.
 */

import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

const stateHome = mkdtempSync(join(tmpdir(), "synveda-state-"));
process.env.XDG_STATE_HOME = stateHome;

const { log } = await import("./log.mjs");

function lines(): Record<string, unknown>[] {
  return readFileSync(join(stateHome, "synveda", "adapter.log"), "utf8")
    .split("\n")
    .filter((line) => line.length > 0)
    .map((line) => JSON.parse(line) as Record<string, unknown>);
}

test("a field cannot rename the event it is logged under", () => {
  log("observe.done", { hook: "Stop", accepted: 3 });
  // The harness's own event name is a natural field to log; it must not
  // collide with the log's.
  log("hook.disabled", { event: "SessionEnd" });

  const written = lines();
  assert.equal(written[0]?.event, "observe.done");
  assert.equal(written[0]?.hook, "Stop");
  assert.equal(written[0]?.accepted, 3);
  assert.equal(written[1]?.event, "hook.disabled");
  assert.ok(typeof written[1]?.at === "string");
});

test("logging never throws, whatever the state directory is doing", () => {
  // A regular file standing where the state directory has to go: hostile
  // the same way on every platform, and ENOTDIR on the first syscall.
  // `/proc/nonexistent-and-unwritable` stood here until 2026-08-01, and it
  // was worse than unwritable — on Linux it made this case spin inside
  // `mkdirSync(recursive)` and never return, which is the stall that hung
  // the `typescript` job. `paths.test.mts` owns that regression now, in a
  // child process with a deadline, because a spin cannot be asserted on
  // from the thread that is spinning.
  const blocked = join(stateHome, "a-file-and-not-a-directory");
  writeFileSync(blocked, "");
  process.env.XDG_STATE_HOME = blocked;
  assert.doesNotThrow(() => {
    log("inject.failed", { reason: "deadline expired" });
  });
  process.env.XDG_STATE_HOME = stateHome;
});
