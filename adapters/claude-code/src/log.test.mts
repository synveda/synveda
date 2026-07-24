/**
 * The log is where every failure this adapter swallows goes, so the one
 * thing it must not do is lose the name of what happened.
 */

import assert from "node:assert/strict";
import { mkdtempSync, readFileSync } from "node:fs";
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
  process.env.XDG_STATE_HOME = "/proc/nonexistent-and-unwritable";
  assert.doesNotThrow(() => {
    log("inject.failed", { reason: "deadline expired" });
  });
  process.env.XDG_STATE_HOME = stateHome;
});
