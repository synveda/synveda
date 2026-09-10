import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  developmentResolutionIsExact,
  localDockerEndpoint,
  normalizeAddresses,
  parseVersion,
  versionAtLeast,
} from "../deploy/compose/scripts/check-host-resolution.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const CHECKER = join(ROOT, "deploy/compose/scripts/check-host-resolution.mjs");

test("development resolver accepts exactly one loopback IPv4 answer", () => {
  assert.equal(developmentResolutionIsExact([{ address: "127.0.0.1", family: 4 }]), true);
  assert.equal(
    developmentResolutionIsExact([
      { address: "127.0.0.1", family: 4 },
      { address: "127.0.0.1", family: 4 },
    ]),
    true,
  );
  for (const answers of [
    [],
    [{ address: "127.0.0.2", family: 4 }],
    [{ address: "::1", family: 6 }],
    [
      { address: "127.0.0.1", family: 4 },
      { address: "::1", family: 6 },
    ],
    [
      { address: "127.0.0.1", family: 4 },
      { address: "192.0.2.1", family: 4 },
    ],
  ]) {
    assert.equal(developmentResolutionIsExact(answers), false, JSON.stringify(answers));
  }
});

test("address normalization is deterministic and duplicate-free", () => {
  assert.deepEqual(
    normalizeAddresses([
      { address: "::1", family: 6 },
      { address: "127.0.0.1", family: 4 },
      { address: "::1", family: 6 },
    ]),
    [
      { address: "127.0.0.1", family: 4 },
      { address: "::1", family: 6 },
    ],
  );
});

test("only local Unix Docker endpoints are accepted", () => {
  assert.equal(localDockerEndpoint("unix:///var/run/docker.sock"), true);
  assert.equal(localDockerEndpoint("unix:///Users/example/.docker/run/docker.sock"), true);
  for (const endpoint of [
    "",
    "tcp://127.0.0.1:2375",
    "ssh://operator@example.invalid",
    "npipe:////./pipe/docker_engine",
    "unix://relative.sock",
    "unix:///path with space/docker.sock",
  ]) {
    assert.equal(localDockerEndpoint(endpoint), false, endpoint);
  }
});

test("Docker Engine version gate is semantic and fail closed", () => {
  assert.deepEqual(parseVersion("28.0.0"), [28, 0, 0]);
  assert.deepEqual(parseVersion("28.1.2-desktop.1"), [28, 1, 2]);
  assert.equal(parseVersion("v28.0.0"), undefined);
  assert.equal(versionAtLeast([28, 0, 0], [28, 0, 0]), true);
  assert.equal(versionAtLeast([28, 0, 1], [28, 0, 0]), true);
  assert.equal(versionAtLeast([27, 5, 1], [28, 0, 0]), false);
});

test("Engine version is checked through the exact resolved Unix endpoint", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-docker-endpoint-"));
  const fakeDocker = join(scratch, "docker");
  const endpoint = `unix://${join(scratch, "docker.sock")}`;
  writeFileSync(
    fakeDocker,
    `#!/bin/sh
set -eu
case "\${1:-}:\${2:-}" in
  context:inspect) printf '%s\n' "$SYNVEDA_TEST_ENDPOINT" ;;
  version:--format)
    [ "\${DOCKER_HOST:-}" = "$SYNVEDA_TEST_ENDPOINT" ] &&
      [ -z "\${DOCKER_CONTEXT:-}" ] || exit 97
    printf '28.0.0\n'
    ;;
  *) exit 98 ;;
esac
`,
    { mode: 0o700 },
  );
  chmodSync(fakeDocker, 0o700);
  try {
    const result = spawnSync(
      process.execPath,
      [
        CHECKER,
        "--docker-only",
        "true",
        "--print-docker-endpoint",
        "true",
        "--docker-bin",
        fakeDocker,
      ],
      {
        encoding: "utf8",
        env: {
          ...process.env,
          DOCKER_CONTEXT: "volatile-context",
          SYNVEDA_TEST_ENDPOINT: endpoint,
        },
      },
    );
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout, `${endpoint}\n`);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});
