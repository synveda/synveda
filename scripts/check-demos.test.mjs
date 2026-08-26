import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, test } from "node:test";

import { checkCorpus } from "./check-demos.mjs";

const temporary = [];
afterEach(() => {
  while (temporary.length > 0) rmSync(temporary.pop(), { recursive: true, force: true });
});

function fixture(source, name = "fixture.sh") {
  const directory = mkdtempSync(join(tmpdir(), "synveda-check-demos-"));
  temporary.push(directory);
  writeFileSync(join(directory, name), source);
  return directory;
}

function inventory() {
  return new Map([
    ["", new Set(["scope", "recall"])],
    ["scope", new Set(["list", "show"])],
    ["scope list", new Set()],
    ["scope show", new Set()],
    ["recall", new Set()],
  ]);
}

test("a deliberately reintroduced dead CLI command and route both fail the gate", () => {
  const directory = fixture(`#!/bin/sh
synveda scope list
synveda hierarchy root
curl -fsS http://127.0.0.1:8120/v1/workspaces
curl -fsS http://127.0.0.1:8120/v1/observe
`);
  const findings = checkCorpus({
    demoDir: directory,
    routes: ["/v1/workspaces"],
    cliInventory: inventory(),
  });
  assert.equal(findings.length, 2, findings.join("\n"));
  assert.match(findings[0], /synveda hierarchy/);
  assert.match(findings[1], /\/v1\/observe/);
});

test("comments explanatory output and heredoc fixtures are not executable references", () => {
  const directory = fixture(`#!/bin/sh
# synveda hierarchy root and /v1/observe are historical words.
echo "synveda hierarchy root used /v1/observe"
cat <<'JSON' >/tmp/example
{"command":"synveda hierarchy root","path":"/v1/observe"}
JSON
ROOT=$(synveda scope list)
curl -fsS "http://127.0.0.1:8120/v1/workspaces/$ROOT/projects"
`);
  const findings = checkCorpus({
    demoDir: directory,
    routes: ["/v1/workspaces/{workspace_id}/projects"],
    cliInventory: inventory(),
  });
  assert.deepEqual(findings, []);
});

test("external OIDC provider routes are outside the Synveda contract", () => {
  const directory = fixture(`#!/bin/sh
curl -fsS "$RAUTHY_URL/auth/v1/users"
curl -fsS "$GATEWAY_URL/v1/workspaces"
`);
  const findings = checkCorpus({
    demoDir: directory,
    routes: ["/v1/workspaces"],
    cliInventory: inventory(),
  });
  assert.deepEqual(findings, []);
});

test("the common built-binary aliases cannot hide a dead subcommand", () => {
  const directory = fixture(`#!/bin/sh
BIN=target/debug/synveda
"$BIN" scope show abc
"$BIN" hierarchy root
`);
  const findings = checkCorpus({
    demoDir: directory,
    routes: [],
    cliInventory: inventory(),
  });
  assert.equal(findings.length, 1, findings.join("\n"));
  assert.match(findings[0], /synveda hierarchy/);
});

test("comments cannot retain references to deleted documentation", () => {
  const directory = fixture(`#!/bin/sh
# AC (docs/backlog/DELETED.md): stale acceptance diary.
synveda scope list
`);
  const findings = checkCorpus({
    demoDir: directory,
    routes: [],
    cliInventory: inventory(),
  });
  assert.equal(findings.length, 1, findings.join("\n"));
  assert.match(findings[0], /docs\/backlog\/DELETED\.md/u);
});

test("container command arrays cannot hide a removed CLI option", () => {
  const directory = fixture(
    'command: ["/usr/local/bin/synveda", "audit", "verify", "--tenant", "abc"]\n',
    "job.yaml",
  );
  const cliInventory = new Map([
    ["", { commands: new Set(["audit"]), options: new Set() }],
    ["audit", { commands: new Set(["verify"]), options: new Set() }],
    ["audit verify", { commands: new Set(), options: new Set(["--json", "--profile"]) }],
  ]);
  const findings = checkCorpus({ demoDir: directory, routes: [], cliInventory });
  assert.equal(findings.length, 1, findings.join("\n"));
  assert.match(findings[0], /audit verify --tenant/u);
});
