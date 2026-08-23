/**
 * The path helpers are arithmetic on strings and need no test of their
 * own. `ensureDir` has one property that does: it comes back.
 */

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { chmodSync, mkdtempSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { ensureDir } from "./paths.mjs";

function scratch(): string {
  return mkdtempSync(join(tmpdir(), "synveda-paths-"));
}

test("a whole missing chain is created, and creating it again is not an error", () => {
  const deep = join(scratch(), "one", "two", "three");
  ensureDir(deep);
  assert.ok(statSync(deep).isDirectory());
  if (process.platform !== "win32") assert.equal(statSync(deep).mode & 0o777, 0o700);
  assert.doesNotThrow(() => {
    ensureDir(deep);
  });
  if (process.platform !== "win32") {
    chmodSync(deep, 0o755);
    ensureDir(deep);
    assert.equal(statSync(deep).mode & 0o777, 0o700, "an existing state dir is tightened");
  }
});

test("a file standing in the way is an error the caller can catch", () => {
  const blocker = join(scratch(), "a-file");
  writeFileSync(blocker, "");
  assert.throws(() => {
    ensureDir(join(blocker, "child"));
  });
});

test("a directory procfs will never permit returns instead of spinning", () => {
  // This runs in a child process with a deadline, and it has to. The
  // defect it pins is a spin, not a failure: `mkdirSync(dir, { recursive:
  // true })` alternates ENOENT on `/proc/x` with EEXIST on `/proc` at
  // ~500,000 syscalls a second and never returns, so an assertion written
  // in the spinning thread is an assertion that is never reached. That is
  // how this arrived — a job that hung for six hours and reported
  // `cancelled`, which every survey of what was *failing* in CI answered
  // cleanly.
  //
  // SIGKILL rather than the default SIGTERM: the loop is inside a
  // synchronous native call, so no JavaScript handler can run, and a
  // signal this test relies on must not be one the child could ignore.
  //
  // Off Linux there is no `/proc` and the walk refuses somewhere else.
  // That is the same pass: the property asserted is that control returns,
  // never which errno it returns with.
  const module = new URL("./paths.mjs", import.meta.url).href;
  const script = [
    `import { ensureDir } from ${JSON.stringify(module)};`,
    `try { ensureDir("/proc/x/y"); } catch { /* refusing is fine; hanging is not */ }`,
    `process.stdout.write("returned");`,
  ].join("\n");

  let stdout = "";
  try {
    stdout = execFileSync(process.execPath, ["--input-type=module", "-e", script], {
      timeout: 5_000,
      killSignal: "SIGKILL",
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch (error) {
    assert.fail(
      `ensureDir("/proc/x/y") did not return within 5s — the recursive-mkdir spin is back: ${String(error)}`,
    );
  }
  assert.equal(stdout, "returned");
});
