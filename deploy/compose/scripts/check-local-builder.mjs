#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { closeSync, constants, fstatSync, openSync, readFileSync } from "node:fs";

const MAX_BYTES = 64 * 1024;

function fail(message, status = 78) {
  process.stderr.write(`compose-builder: ${message}\n`);
  process.exit(status);
}

function readPrivateInput(path) {
  let descriptor;
  try {
    descriptor = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW);
    const before = fstatSync(descriptor, { bigint: true });
    if (
      !before.isFile() ||
      before.nlink !== 1n ||
      before.uid !== BigInt(process.getuid()) ||
      (before.mode & 0o7777n) !== 0o600n ||
      before.size < 1n ||
      before.size > BigInt(MAX_BYTES)
    ) {
      fail("inspection input was refused", 69);
    }
    const input = readFileSync(descriptor);
    const after = fstatSync(descriptor, { bigint: true });
    if (
      after.dev !== before.dev ||
      after.ino !== before.ino ||
      after.size !== before.size ||
      after.mode !== before.mode ||
      after.nlink !== 1n
    ) {
      fail("inspection input changed while it was read", 69);
    }
    return input;
  } catch {
    fail("inspection input was unavailable", 69);
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

function inspectDocker(binary) {
  const result = spawnSync(binary, ["buildx", "inspect", "--timeout", "20s", "default"], {
    encoding: null,
    env: { ...process.env, LANG: "C", LC_ALL: "C" },
    killSignal: "SIGKILL",
    maxBuffer: MAX_BYTES,
    timeout: 25_000,
  });
  if (
    result.error !== undefined ||
    result.signal !== null ||
    result.status !== 0 ||
    !Buffer.isBuffer(result.stdout) ||
    result.stdout.length < 1 ||
    result.stdout.length > MAX_BYTES
  ) {
    fail("pinned local Docker builder was unavailable", 69);
  }
  return result.stdout;
}

let input;
if (
  process.argv.length === 4 &&
  process.argv[2] === "--input-file" &&
  process.argv[3]?.startsWith("/") &&
  !/[\0\r\n]/.test(process.argv[3])
) {
  input = readPrivateInput(process.argv[3]);
} else if (
  process.argv.length === 4 &&
  process.argv[2] === "--docker-bin" &&
  process.argv[3] !== "" &&
  !/[\0\r\n]/.test(process.argv[3])
) {
  input = inspectDocker(process.argv[3]);
} else {
  fail(
    "usage: check-local-builder.mjs (--docker-bin COMMAND|--input-file ABSOLUTE_PATH)",
    64,
  );
}

if (
  [...input].some((byte) => byte !== 0x09 && byte !== 0x0a && (byte < 0x20 || byte > 0x7e))
) {
  fail("inspection output was malformed", 69);
}
const text = input.toString("ascii");
const lines = text.endsWith("\n") ? text.slice(0, -1).split("\n") : text.split("\n");
if (
  lines.some((line) =>
    /^[\t ]*(?:Error|Driver Options|BuildKit daemon flags|Flags|File#[^:]*):/.test(line),
  )
) {
  fail("builder extension field was refused");
}

const nodesIndex = lines.findIndex((line) => /^Nodes:\s*$/.test(line));
if (nodesIndex < 1 || lines.some((line, index) => index !== nodesIndex && /^Nodes:\s*$/.test(line))) {
  fail("builder node section was refused");
}

function exactField(section, name, expected) {
  const matches = section.filter((line) => {
    const match = line.match(/^([A-Za-z][A-Za-z0-9 ]*):\s*(.*?)\s*$/);
    return match?.[1] === name;
  });
  if (matches.length !== 1) fail(`builder ${name.toLowerCase()} field was refused`);
  const value = matches[0].match(/^([A-Za-z][A-Za-z0-9 ]*):\s*(.*?)\s*$/)?.[2];
  if (value !== expected) fail(`builder ${name.toLowerCase()} was refused`);
}

const builder = lines.slice(0, nodesIndex);
const nodes = lines.slice(nodesIndex + 1);
exactField(builder, "Name", "default");
exactField(builder, "Driver", "docker");
exactField(nodes, "Name", "default");
exactField(nodes, "Endpoint", "default");
exactField(nodes, "Status", "running");

if (builder.filter((line) => /^Last Activity:\s*/.test(line)).length > 1) {
  fail("builder last activity field was refused");
}

// A second node always introduces another node-level Name. Refuse it without
// interpreting BuildKit/plugin-specific fields below the one accepted node.
const nodeNames = nodes.filter((line) => /^Name:\s*/.test(line));
if (nodeNames.length !== 1) fail("builder node count was refused");

process.stdout.write("compose-builder: embedded default builder verified\n");
