#!/usr/bin/env node
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  chmodSync,
  linkSync,
  mkdtempSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";

const checker = resolve("deploy/compose/scripts/check-local-builder.mjs");
const valid = `Name:          default
Driver:        docker
Last Activity: 2026-09-01 10:00:00 +0000 UTC

Nodes:
Name:          default
Endpoint:      default
Status:        running
BuildKit version: v0.26.2
Platforms:     linux/amd64, linux/arm64
Labels:
 org.mobyproject.buildkit.worker.executor: oci
GC Policy rule#0:
 All:           false
 Keep Duration: 48h0m0s
`;

function fixture(content = valid, mode = 0o600) {
  const root = mkdtempSync(join(tmpdir(), "synveda-builder-check-"));
  const input = join(root, "inspect.txt");
  writeFileSync(input, content, { mode });
  return { root, input };
}

function run(input, extra = []) {
  return spawnSync(process.execPath, [checker, "--input-file", input, ...extra], {
    encoding: "utf8",
    env: { PATH: process.env.PATH, LC_ALL: "C" },
  });
}

function fakeDocker(stdout, stderr = "", status = 0) {
  const root = mkdtempSync(join(tmpdir(), "synveda-builder-docker-"));
  const binary = join(root, "docker");
  const stdoutPath = join(root, "stdout");
  const stderrPath = join(root, "stderr");
  writeFileSync(stdoutPath, stdout, { mode: 0o600 });
  writeFileSync(stderrPath, stderr, { mode: 0o600 });
  writeFileSync(
    binary,
    `#!/bin/sh\nset -eu\n[ "$*" = "buildx inspect --timeout 20s default" ] || exit 98\n/bin/cat ${JSON.stringify(stdoutPath)}\n/bin/cat ${JSON.stringify(stderrPath)} >&2\nexit ${status}\n`,
    { mode: 0o700 },
  );
  chmodSync(binary, 0o700);
  return { binary, root };
}

function runDocker(binary) {
  return spawnSync(process.execPath, [checker, "--docker-bin", binary], {
    encoding: "utf8",
    env: { PATH: process.env.PATH, LC_ALL: "C" },
  });
}

test("the embedded default Docker builder is accepted without reproducing inspection data", () => {
  const { root, input } = fixture();
  try {
    const result = run(input);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout, "compose-builder: embedded default builder verified\n");
    assert.equal(result.stderr, "");
    assert.doesNotMatch(result.stdout, /BuildKit|Platforms|Activity/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("remote drivers, endpoints, extra nodes and unhealthy builders are refused content-free", () => {
  const mutants = [
    valid.replace("Driver:        docker", "Driver:        remote"),
    valid.replace("Driver:        docker", "Driver:        docker-container"),
    valid.replace("Driver:        docker", "Driver:        kubernetes"),
    valid.replace("Endpoint:      default", "Endpoint:      tcp://127.0.0.1:47001"),
    valid.replace("Endpoint:      default", "Endpoint:      ssh://builder.invalid"),
    valid.replace("Endpoint:      default", "Endpoint:      unix:///tmp/foreign.sock"),
    valid.replace("Status:        running", "Status:        stopped"),
    `${valid}Name:          second\nEndpoint:      default\nStatus:        running\n`,
    valid.replace("BuildKit version: v0.26.2", "Endpoint:      default"),
  ];
  for (const content of mutants) {
    const { root, input } = fixture(content);
    try {
      const result = run(input);
      assert.equal(result.status, 78);
      assert.equal(result.stdout, "");
      assert.match(result.stderr, /^compose-builder: [a-z ]+ was refused\n$/);
      assert.doesNotMatch(result.stderr, /tcp|ssh|foreign|kubernetes|remote|second/);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  }
});

test("driver options, daemon flags, files, errors, duplicates and controls fail closed", () => {
  const mutants = [
    valid.replace("Driver:        docker\n", "Driver:        docker\nDriver:        docker\n"),
    valid.replace("Last Activity:", "Last Activity: old\nLast Activity:"),
    valid.replace("BuildKit version:", "Error: secret-value\nBuildKit version:"),
    valid.replace("BuildKit version:", "Driver Options: network=host\nBuildKit version:"),
    valid.replace("BuildKit version:", "BuildKit daemon flags: --debug\nBuildKit version:"),
    valid.replace("BuildKit version:", "Flags: --debug\nBuildKit version:"),
    valid.replace("BuildKit version:", "File#0: /private/value\nBuildKit version:"),
    valid.replace("BuildKit version:", "BuildKit version:\u0000"),
    valid.replace("BuildKit version:", "BuildKit version: \u00e9"),
  ];
  for (const content of mutants) {
    const { root, input } = fixture(content);
    try {
      const result = run(input);
      assert.notEqual(result.status, 0);
      assert.equal(result.stdout, "");
      assert.match(result.stderr, /^compose-builder: /);
      assert.doesNotMatch(result.stderr, /secret-value|network=host|private\/value/);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  }
});

test("Docker inspection is byte-bounded and never reproduces Docker stderr", () => {
  const accepted = fakeDocker(valid, "secret-stderr-that-must-not-escape\n");
  const failed = fakeDocker(valid, "credential-on-failure\n", 41);
  const oversized = fakeDocker("x".repeat(64 * 1024 + 1), "oversized-secret\n");
  try {
    const result = runDocker(accepted.binary);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout, "compose-builder: embedded default builder verified\n");
    assert.equal(result.stderr, "");

    for (const fixture of [failed, oversized]) {
      const refusal = runDocker(fixture.binary);
      assert.equal(refusal.status, 69);
      assert.equal(refusal.stdout, "");
      assert.equal(
        refusal.stderr,
        "compose-builder: pinned local Docker builder was unavailable\n",
      );
      assert.doesNotMatch(refusal.stderr, /credential|secret|oversized/);
    }
  } finally {
    for (const fixture of [accepted, failed, oversized]) {
      rmSync(fixture.root, { recursive: true, force: true });
    }
  }
});

test("inspection input must be private, single-link, bounded and non-symlinked", () => {
  const permissive = fixture(valid, 0o644);
  const hardlinked = fixture();
  const linkedPath = join(hardlinked.root, "other.txt");
  linkSync(hardlinked.input, linkedPath);
  const symlinked = fixture();
  const symlinkPath = join(symlinked.root, "link.txt");
  symlinkSync(symlinked.input, symlinkPath);
  const oversized = fixture("x".repeat(64 * 1024 + 1));
  try {
    for (const input of [permissive.input, hardlinked.input, symlinkPath, oversized.input]) {
      const result = run(input);
      assert.equal(result.status, 69);
      assert.equal(result.stdout, "");
      assert.match(result.stderr, /^compose-builder: inspection input/);
    }
    chmodSync(permissive.input, 0o600);
  } finally {
    for (const root of [permissive.root, hardlinked.root, symlinked.root, oversized.root]) {
      rmSync(root, { recursive: true, force: true });
    }
  }
});

test("invalid invocation is rejected", () => {
  const result = spawnSync(process.execPath, [checker], { encoding: "utf8" });
  assert.equal(result.status, 64);
  assert.equal(result.stdout, "");
  assert.equal(
    result.stderr,
    "compose-builder: usage: check-local-builder.mjs (--docker-bin COMMAND|--input-file ABSOLUTE_PATH)\n",
  );
});
