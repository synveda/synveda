/**
 * The MCP entry point, as a process (ADPT-2, ADR-0057 decision 4).
 *
 * `mcp.test.mts` used to drive the hand-written protocol loop frame by
 * frame. There is no loop here any more — `synveda mcp` owns the protocol,
 * and its own suite drives the frames — so what is left to prove is
 * exactly what this file now does, and every case below is something that
 * would otherwise be invisible until a user's client came up empty:
 *
 * - it execs the binary the credential seam resolves, not a bare name;
 * - it passes `--writes host`, which is decision 6's whole protection
 *   against this plugin storing the same turn twice;
 * - it is a pipe and not a participant — stdin and stdout reach the
 *   server unaltered;
 * - a missing binary *says so* rather than serving an empty tool list.
 *
 * It is driven as a real subprocess rather than imported, because the
 * artifact the manifest names is a script whose whole behaviour is a side
 * effect. A test that imported it and asserted on exported constants could
 * pass with the spawn wired up wrong.
 */

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

const scratch = mkdtempSync(join(tmpdir(), "synveda-mcp-launcher-"));
const launcher = join(import.meta.dirname, "mcp-server.mjs");

interface Run {
  code: number | null;
  stdout: string;
  stderr: string;
}

/**
 * A stand-in for the CLI that records how it was invoked. Shell rather
 * than Node so the recording cannot accidentally share anything with the
 * code under test.
 */
function fakeCli(name: string, body: string): { path: string; argv: () => string[] } {
  const path = join(scratch, name);
  const argvFile = join(scratch, `${name}.argv`);
  writeFileSync(path, `#!/bin/sh\nprintf '%s\\n' "$@" > "${argvFile}"\n${body}\n`);
  chmodSync(path, 0o755);
  return {
    path,
    argv: () =>
      readFileSync(argvFile, "utf8")
        .split("\n")
        .filter((line) => line.length > 0),
  };
}

function run(cli: string, stdin = ""): Promise<Run> {
  return new Promise((resolve) => {
    const child = spawn(process.execPath, [launcher], {
      stdio: ["pipe", "pipe", "pipe"],
      env: { ...process.env, SYNVEDA_CLI: cli, XDG_STATE_HOME: scratch },
    });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk: Buffer) => (stdout += chunk.toString()));
    child.stderr.on("data", (chunk: Buffer) => (stderr += chunk.toString()));
    child.stdin.end(stdin);
    child.on("close", (code) => resolve({ code, stdout, stderr }));
  });
}

test("it execs the CLI the credential seam resolves, with --writes host", async () => {
  const cli = fakeCli("records-argv", "exit 0");
  const result = await run(cli.path);

  assert.equal(result.code, 0);
  assert.deepEqual(
    cli.argv(),
    ["mcp", "--writes", "host"],
    "ADR-0057 decision 6: this plugin's Stop hook already observes its turns, so the " +
      "model must not also be offered a write tool — the same turn would be stored twice",
  );
});

test("SYNVEDA_CLI is honoured, which is why the manifest names this file", async () => {
  // A manifest that hard-wired `"command": "synveda"` would run whatever
  // was on PATH. The tests and demos all point this at a build in the
  // working tree, and that is the seam being defended.
  const chosen = fakeCli("the-one-we-asked-for", "exit 0");
  await run(chosen.path);
  assert.deepEqual(chosen.argv(), ["mcp", "--writes", "host"]);
});

test("it is a pipe, not a participant: stdio reaches the server unaltered", async () => {
  // The frames a client sends must arrive byte for byte. A launcher that
  // buffered, re-encoded, or line-wrapped them would break the protocol in
  // a way no unit test of the Rust side could see.
  const echo = fakeCli("echoes-stdin", "cat");
  const frames = '{"jsonrpc":"2.0","id":1,"method":"tools/list"}\n{"jsonrpc":"2.0","id":2}\n';
  const result = await run(echo.path, frames);

  assert.equal(result.stdout, frames, "stdout must be the server's own, verbatim");
  assert.equal(result.code, 0);
});

test("the server's exit status is the launcher's", async () => {
  const failing = fakeCli("exits-17", "exit 17");
  assert.equal((await run(failing.path)).code, 17);
});

test("a missing binary says so, rather than silently serving no tools", async () => {
  const result = await run(join(scratch, "no-such-binary"));

  assert.notEqual(result.code, 0, "a client must not think this server started");
  assert.match(result.stderr, /`synveda` CLI was not found/);
  assert.match(result.stderr, /synveda login/, "the message has to name the fix");
  assert.equal(
    result.stdout,
    "",
    "stdout is the client's half of a protocol this process does not speak: a line " +
      "there is a parse error at the far end, not a message anybody reads",
  );
});
