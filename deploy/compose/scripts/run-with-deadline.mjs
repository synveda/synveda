#!/usr/bin/env node
import { spawn } from "node:child_process";
import { closeSync, fsyncSync, ftruncateSync, openSync, writeSync } from "node:fs";
import { constants as osConstants } from "node:os";

function usage() {
  process.stderr.write(
    "run-with-deadline: usage: run-with-deadline.mjs --seconds N [--status-file PATH] -- COMMAND [ARG ...]\n",
  );
  process.exit(64);
}

if (
  process.argv[2] !== "--seconds" ||
  !/^[1-9][0-9]{0,3}$/.test(process.argv[3] ?? "")
) {
  usage();
}
const seconds = Number(process.argv[3]);
if (!Number.isSafeInteger(seconds) || seconds > 3600) usage();
let cursor = 4;
let statusFile;
if (process.argv[cursor] === "--status-file") {
  statusFile = process.argv[cursor + 1];
  if (!statusFile?.startsWith("/") || /[\0\r\n]/.test(statusFile)) usage();
  cursor += 2;
}
if (process.argv[cursor] !== "--" || cursor + 1 >= process.argv.length) usage();

const command = process.argv[cursor + 1];
const args = process.argv.slice(cursor + 2);
let timedOut = false;
let requestedSignal;
let killTimer;
let groupPoll;
let forcedExitTimer;
let child;
let childClosed = false;
let childCode;
let childSignal;
let finished = false;
let groupStateUncertain = false;
let groupContractViolated = false;
let forcedStopUsed = false;

function signalGroup(signal) {
  if (child === undefined) return;
  try {
    process.kill(-child.pid, signal);
    return;
  } catch {
    if (child.exitCode !== null || child.signalCode !== null) return;
    try {
      child.kill(signal);
    } catch {
      // The child may have exited between the state check and signal.
    }
  }
}

function groupExists() {
  if (!Number.isSafeInteger(child?.pid) || child.pid <= 1) return false;
  try {
    process.kill(-child.pid, 0);
    return true;
  } catch (error) {
    return error?.code !== "ESRCH";
  }
}

function exitStatus() {
  if (groupStateUncertain || groupContractViolated || forcedStopUsed) return 125;
  if (timedOut) return 124;
  if (requestedSignal !== undefined) {
    return 128 + signalNumbers.get(requestedSignal);
  }
  if (childCode !== null && childCode !== undefined) return childCode;
  const childSignalNumber = signalNumbers.get(childSignal) ?? osConstants.signals[childSignal];
  return 128 + (Number.isInteger(childSignalNumber) ? childSignalNumber : 1);
}

function publishCleanStatus(status) {
  if (statusFile === undefined) return true;
  let descriptor;
  try {
    // r+ refuses a status path removed or substituted with a directory; it
    // never recreates a parent's already-cleaned witness after a launch race.
    descriptor = openSync(statusFile, "r+");
    ftruncateSync(descriptor, 0);
    writeSync(descriptor, `clean:${status}\n`, undefined, "utf8");
    fsyncSync(descriptor);
    closeSync(descriptor);
    return true;
  } catch {
    if (descriptor !== undefined) {
      try {
        closeSync(descriptor);
      } catch {
        // Preserve the primary witness-publication failure.
      }
    }
    process.stderr.write("run-with-deadline: clean process-group status could not be recorded\n");
    return false;
  }
}

function finish(statusOverride) {
  if (finished) return;
  finished = true;
  clearTimeout(deadline);
  if (killTimer !== undefined) clearTimeout(killTimer);
  if (groupPoll !== undefined) clearInterval(groupPoll);
  if (forcedExitTimer !== undefined) clearTimeout(forcedExitTimer);
  const status = statusOverride ?? exitStatus();
  if (groupStateUncertain || groupExists() || !publishCleanStatus(status)) {
    process.exit(125);
  }
  process.exit(status);
}

function finishAfterGroupExit() {
  if (!childClosed) return;
  if (!groupExists()) {
    finish();
    return;
  }
  if (!timedOut && requestedSignal === undefined && !groupContractViolated) {
    groupContractViolated = true;
    process.stderr.write(
      "run-with-deadline: process group remained after command exit\n",
    );
    signalGroup("SIGTERM");
    armForcedStop();
  }
  groupPoll ??= setInterval(() => {
    if (!groupExists()) finish();
  }, 25);
}

function armForcedStop() {
  if (killTimer !== undefined) return;
  killTimer = setTimeout(() => {
    if (!groupExists()) {
      finishAfterGroupExit();
      return;
    }
    forcedStopUsed = true;
    process.stderr.write(
      "run-with-deadline: forced stop bypassed command cleanup\n",
    );
    signalGroup("SIGKILL");
    // SIGKILL delivery is synchronous, but process disappearance is not.
    // Retain the runner/lock for one final bounded reap window.
    forcedExitTimer = setTimeout(() => {
      if (groupExists()) {
        groupStateUncertain = true;
        process.stderr.write(
          "run-with-deadline: process group state remained uncertain after forced stop\n",
        );
      }
      finish();
    }, 1_000);
    finishAfterGroupExit();
  }, 5_000);
}

const signalNumbers = new Map([
  ["SIGHUP", 1],
  ["SIGINT", 2],
  ["SIGTERM", 15],
]);
for (const signal of signalNumbers.keys()) {
  process.on(signal, () => {
    requestedSignal ??= signal;
    signalGroup(signal);
    armForcedStop();
    finishAfterGroupExit();
  });
}

// Install handlers before creating a detached process group. A signal in the
// spawn interval is remembered and delivered as soon as the child exists, so
// no Docker mutation can outlive the runner while its parent releases a lock.
child = spawn(command, args, {
  detached: true,
  stdio: "inherit",
});
if (requestedSignal !== undefined) signalGroup(requestedSignal);

const deadline = setTimeout(() => {
  timedOut = true;
  process.stderr.write(`run-with-deadline: command exceeded ${seconds} seconds\n`);
  signalGroup("SIGTERM");
  armForcedStop();
  finishAfterGroupExit();
}, seconds * 1_000);
deadline.unref();

child.on("error", (error) => {
  clearTimeout(deadline);
  if (killTimer !== undefined) clearTimeout(killTimer);
  if (groupPoll !== undefined) clearInterval(groupPoll);
  if (forcedExitTimer !== undefined) clearTimeout(forcedExitTimer);
  process.stderr.write(`run-with-deadline: command could not start: ${error.code ?? "error"}\n`);
  finish(69);
});

child.on("close", (code, signal) => {
  childClosed = true;
  childCode = code;
  childSignal = signal;
  clearTimeout(deadline);
  finishAfterGroupExit();
});
