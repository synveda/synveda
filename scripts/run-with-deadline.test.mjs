import assert from "node:assert/strict";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import {
  chmodSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import test from "node:test";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const RUNNER = join(ROOT, "deploy/compose/scripts/run-with-deadline.mjs");

test("the lifecycle deadline returns the child status", () => {
  const result = spawnSync(
    process.execPath,
    [RUNNER, "--seconds", "2", "--", "/bin/sh", "-c", "exit 7"],
    { encoding: "utf8" },
  );
  assert.equal(result.status, 7, result.stderr);
});

test("the lifecycle deadline publishes a clean completion witness", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-deadline-status-"));
  const statusFile = join(scratch, "status");
  try {
    writeFileSync(statusFile, "", { mode: 0o600 });
    const result = spawnSync(
      process.execPath,
      [
        RUNNER,
        "--seconds", "2",
        "--status-file", statusFile,
        "--", "/bin/sh", "-c", "exit 7",
      ],
      { encoding: "utf8" },
    );
    assert.equal(result.status, 7, result.stderr);
    assert.equal(readFileSync(statusFile, "utf8"), "clean:7\n");
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("the lifecycle deadline terminates a hung process group", () => {
  const started = Date.now();
  const result = spawnSync(
    process.execPath,
    [RUNNER, "--seconds", "1", "--", "/bin/sh", "-c", "/bin/sleep 30"],
    { encoding: "utf8", timeout: 8_000 },
  );
  assert.equal(result.status, 124, result.stderr);
  assert.match(result.stderr, /command exceeded 1 seconds/);
  assert.ok(Date.now() - started < 7_000);
});

test("a clean timeout is witnessed only after the process group is gone", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-deadline-timeout-status-"));
  const statusFile = join(scratch, "status");
  try {
    writeFileSync(statusFile, "", { mode: 0o600 });
    const result = spawnSync(
      process.execPath,
      [
        RUNNER,
        "--seconds", "1",
        "--status-file", statusFile,
        "--", "/bin/sh", "-c", "/bin/sleep 30",
      ],
      { encoding: "utf8", timeout: 8_000 },
    );
    assert.equal(result.status, 124, result.stderr);
    assert.equal(readFileSync(statusFile, "utf8"), "clean:124\n");
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("signal handlers are installed before the detached child is spawned", () => {
  const source = readFileSync(RUNNER, "utf8");
  assert.ok(source.indexOf("process.on(signal") >= 0);
  assert.ok(source.indexOf("process.on(signal") < source.indexOf("child = spawn(command"));
});

test("the lifecycle deadline escalates against a TERM-ignoring process group", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-deadline-forced-status-"));
  const statusFile = join(scratch, "status");
  const started = Date.now();
  try {
    writeFileSync(statusFile, "", { mode: 0o600 });
    const result = spawnSync(
      process.execPath,
      [
        RUNNER,
        "--seconds", "1",
        "--status-file", statusFile,
        "--",
        "/bin/sh", "-c", "trap '' HUP INT TERM; while :; do /bin/sleep 1; done",
      ],
      { encoding: "utf8", timeout: 9_000 },
    );
    assert.equal(result.status, 125, result.stderr);
    assert.match(result.stderr, /command exceeded 1 seconds/);
    assert.match(result.stderr, /forced stop bypassed command cleanup/);
    assert.equal(readFileSync(statusFile, "utf8"), "clean:125\n");
    assert.ok(Date.now() - started >= 5_000);
    assert.ok(Date.now() - started < 8_500);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("a TERM-responsive leader cannot orphan a TERM-ignoring group member", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-deadline-group-"));
  const leader = join(scratch, "leader.sh");
  const member = join(scratch, "member.sh");
  const memberPidFile = join(scratch, "member.pid");
  let memberPid;
  try {
    writeFileSync(
      member,
      `#!/bin/sh
trap '' HUP INT TERM
printf '%s\n' "$$" > "$1"
while :; do /bin/sleep 1; done
`,
      { mode: 0o700 },
    );
    writeFileSync(
      leader,
      `#!/bin/sh
set -eu
trap 'exit 0' TERM
"$1" "$2" &
while [ ! -f "$2" ]; do /bin/sleep 0.02; done
wait
`,
      { mode: 0o700 },
    );
    chmodSync(leader, 0o700);
    chmodSync(member, 0o700);
    const result = spawnSync(
      process.execPath,
      [RUNNER, "--seconds", "1", "--", leader, member, memberPidFile],
      { encoding: "utf8", timeout: 9_000 },
    );
    assert.equal(result.status, 125, result.stderr);
    assert.match(result.stderr, /forced stop bypassed command cleanup/);
    memberPid = Number(readFileSync(memberPidFile, "utf8").trim());
    assert.ok(Number.isSafeInteger(memberPid) && memberPid > 1);
    assert.throws(() => process.kill(memberPid, 0), { code: "ESRCH" });
  } finally {
    if (Number.isSafeInteger(memberPid) && memberPid > 1) {
      try {
        process.kill(memberPid, "SIGKILL");
      } catch {
        // Expected once the runner has killed the complete process group.
      }
    }
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("a normally exiting leader cannot orphan its process-group member", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-deadline-orphan-"));
  const leader = join(scratch, "leader.sh");
  const member = join(scratch, "member.sh");
  const memberPidFile = join(scratch, "member.pid");
  let memberPid;
  try {
    writeFileSync(
      member,
      `#!/bin/sh
trap '' HUP INT TERM
printf '%s\n' "$$" > "$1"
while :; do /bin/sleep 1; done
`,
      { mode: 0o700 },
    );
    writeFileSync(
      leader,
      `#!/bin/sh
set -eu
"$1" "$2" &
while [ ! -f "$2" ]; do /bin/sleep 0.02; done
exit 0
`,
      { mode: 0o700 },
    );
    chmodSync(leader, 0o700);
    chmodSync(member, 0o700);
    const result = spawnSync(
      process.execPath,
      [RUNNER, "--seconds", "8", "--", leader, member, memberPidFile],
      { encoding: "utf8", timeout: 9_000 },
    );
    assert.equal(result.status, 125, result.stderr);
    assert.match(result.stderr, /process group remained after command exit/);
    memberPid = Number(readFileSync(memberPidFile, "utf8").trim());
    assert.ok(Number.isSafeInteger(memberPid) && memberPid > 1);
    assert.throws(() => process.kill(memberPid, 0), { code: "ESRCH" });
  } finally {
    if (Number.isSafeInteger(memberPid) && memberPid > 1) {
      try {
        process.kill(memberPid, "SIGKILL");
      } catch {
        // Expected once the runner has killed the complete process group.
      }
    }
    rmSync(scratch, { recursive: true, force: true });
  }
});
