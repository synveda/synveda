import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

import { validateConfig } from "./run-git-export-canary.mjs";

const runner = fileURLToPath(new URL("run-git-export-canary.mjs", import.meta.url));
const tenant = "00000000-0000-0000-0000-000000000001";
const scope = "00000000-0000-0000-0000-000000000002";
const config = {
  format: "synveda-github-canary-v1", profile: "git-canary", tenant_id: tenant,
  scope_id: scope, target: "github-canary", repository: "example-owner/canary",
  repository_id: 42, deployment_artifact: `sha256:${"d".repeat(64)}`,
  cli_sha256: "a".repeat(64), synthetic_scope: true,
  source_heads: { "prompt/published": "1".repeat(64), "context-pack/published": "2".repeat(64) },
};

test("a canary cannot accept token values, URLs, unpinned sources or non-synthetic scope", () => {
  assert.equal(validateConfig(config), config);
  for (const changed of [
    { token: "do-not-print-this-token" }, { repository: "https://github.com/example-owner/canary" },
    { repository: "example-owner/../canary" }, { repository: "example-owner/canary.git" },
    { repository_id: Number.MAX_SAFE_INTEGER + 1 }, { synthetic_scope: false },
    { cli_sha256: "unrecorded" }, { source_heads: { "prompt/published": "1".repeat(64) } },
    { trusted_keys_file: "relative/keys.json" },
  ]) assert.throws(() => validateConfig({ ...config, ...changed }), /pinned|pin/);
});

const fakeProgram = `#!/usr/bin/env node
const fs = require("node:fs"), path = require("node:path");
const args = process.argv.slice(2), name = path.basename(process.argv[1]);
const mode = process.env.CANARY_TEST_MODE;
const config = JSON.parse(fs.readFileSync(process.env.CANARY_TEST_CONFIG, "utf8"));
const log = process.env.CANARY_TEST_LOG;
const entries = fs.existsSync(log) ? fs.readFileSync(log, "utf8").trim().split("\\n").map(JSON.parse) : [];
fs.appendFileSync(log, JSON.stringify({ program: name, args,
  rawBearerPresent: Boolean(process.env.SYNVEDA_TOKEN),
  unsafeGitPresent: Boolean(process.env.GIT_SSL_NO_VERIFY),
}) + "\\n");
function output(value) { process.stdout.write(JSON.stringify(value)); }
function family(channel) { return channel.startsWith("prompt") ? "1" : "2"; }
function result(channel, outcome) {
  const digit = family(channel);
  return { target: config.target, provider: "github", repository_id: config.repository_id,
    source_head: digit.repeat(64), source_pin: null, git_ref: "refs/heads/synveda/" + channel,
    git_head: digit.repeat(40), mapping_digest: digit.repeat(64),
    destination_digest: "a".repeat(64), commits: 2, objects: 1, outcome };
}
if (name === "git") {
  if (args[0] === "status") process.exit(0);
  if (args[0] === "rev-parse") { process.stdout.write("b".repeat(40) + "\\n"); process.exit(0); }
  if (args.includes("clone")) {
    fs.mkdirSync(args.at(-1)); process.exit(0);
  }
  if (args.includes("--verify")) {
    process.stdout.write(family(args.at(-1).replace("refs/heads/synveda/", "")).repeat(40) + "\\n");
    process.exit(0);
  }
} else if (name === "gh") {
  output({ id: mode === "foreign-repository" ? 43 : 42,
    full_name: config.repository, private: mode !== "public", visibility: mode === "public" ? "public" : "private",
    archived: false, disabled: false, fork: false, size: 2 }); process.exit(0);
} else if (args[0] === "--version") {
  process.stdout.write("synveda 0.4.4\\n"); process.exit(0);
} else if (args[0] === "whoami") {
  output({ tenant: { id: mode === "foreign-tenant" ? "foreign" : config.tenant_id } }); process.exit(0);
} else if (args[0] === "configuration") {
  output({ scope_id: config.scope_id, document: { git_export_targets: [config.target],
    allowed_external_providers: mode === "disabled" ? [] : ["github"] } }); process.exit(0);
} else if (args[0] === "channel") {
  output({ scope_id: config.scope_id, channels: Object.keys(config.source_heads).map(name => ({ name,
    commit: mode === "moved-source" ? "f".repeat(64) : config.source_heads[name],
    pin: mode === "pinned-source" ? { commit: config.source_heads[name] } : null })) }); process.exit(0);
} else if (args[1] === "export") {
  if (mode === "echoed-secret") { process.stderr.write("do-not-print-this-token"); process.exit(1); }
  const channel = args[args.indexOf("--channel") + 1];
  const previous = entries.filter(row => row.args[1] === "export" && row.args.includes(channel)).length;
  const receipt = result(channel, previous ? "no_op" : "completed");
  if (mode === "changed-replay" && previous) receipt.git_head = "f".repeat(40);
  if (mode === "changed-destination" && channel.startsWith("context-pack")) receipt.destination_digest = "f".repeat(64);
  receipt.content = "this unexpected response field must not be retained";
  output(receipt); process.exit(0);
} else if (args[1] === "verify") {
  const channel = args[args.indexOf("--channel") + 1];
  output({ valid: mode !== "bad-verification", format: "synveda-git-export-v1",
    source_head: family(channel).repeat(64), mapping_digest: family(channel).repeat(64), commits: 2, objects: 1 });
  process.exit(0);
}
process.exit(10);
`;

function fixture(mode, check) {
  const directory = mkdtempSync(join(tmpdir(), "synveda-github-canary-test-"));
  try {
    for (const name of ["git", "gh", "synveda"]) {
      writeFileSync(join(directory, name), fakeProgram);
      chmodSync(join(directory, name), 0o700);
    }
    const manifest = join(directory, "canary.json");
    const cliSha = createHash("sha256").update(fakeProgram).digest("hex");
    writeFileSync(manifest, JSON.stringify({ ...config, cli_sha256: cliSha }));
    const output = join(directory, "evidence");
    const log = join(directory, "commands.jsonl");
    const result = spawnSync(process.execPath, [runner, manifest, output], {
      cwd: resolve(fileURLToPath(new URL("..", import.meta.url))), encoding: "utf8", timeout: 30_000,
      env: { ...process.env, PATH: `${directory}:${process.env.PATH}`,
        SYNVEDA_BIN: join(directory, "synveda"), SYNVEDA_TOKEN: "do-not-print-this-token",
        GIT_SSL_NO_VERIFY: "1", CANARY_TEST_MODE: mode,
        CANARY_TEST_CONFIG: manifest, CANARY_TEST_LOG: log, TMPDIR: directory, TMP: directory, TEMP: directory },
    });
    const commands = existsSync(log)
      ? readFileSync(log, "utf8").trim().split("\n").map(JSON.parse) : [];
    check({ result, output, commands, directory });
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

test("missing operator inputs return PENDING, never a passing live gate", () => {
  const result = spawnSync(process.execPath, [runner], { encoding: "utf8" });
  assert.equal(result.status, 77);
  assert.match(result.stderr, /PENDING/);
});

test("both families use four public exports and independent clones; evidence records exact limits", () => {
  fixture("pass", ({ result, output, commands, directory }) => {
    assert.equal(result.status, 0, result.stderr);
    const report = JSON.parse(readFileSync(join(output, "report.json"), "utf8"));
    assert.equal(report.evidence_tier, "live_github_export_replay");
    assert.equal(report.channels.length, 2);
    assert.equal(report.not_measured.length, 6);
    assert.ok(report.channels.every(row => row.verified && row.replay.outcome === "no_op"));
    assert.equal(commands.filter(row => row.args[1] === "export").length, 4);
    assert.equal(commands.filter(row => row.args.includes("clone")).length, 2);
    assert.ok(commands.every(row => !row.rawBearerPresent && !row.unsafeGitPresent));
    assert.equal(existsSync(join(output, "partial.json")), false);
    assert.equal(readFileSync(join(output, "report.json"), "utf8").includes("unexpected response"), false);
    const scratch = commands.find(row => row.args.includes("clone")).args.at(-1);
    assert.ok(scratch.startsWith(directory));
    assert.equal(existsSync(scratch), false);
  });
});

for (const mode of ["public", "foreign-repository", "foreign-tenant", "disabled", "moved-source", "pinned-source"]) {
  test(`${mode} refuses before any export or clone`, () => {
    fixture(mode, ({ result, output, commands }) => {
      assert.notEqual(result.status, 0);
      assert.equal(existsSync(output), false);
      assert.equal(commands.some(row => row.args[1] === "export" || row.args.includes("clone")), false);
    });
  });
}

for (const mode of ["changed-replay", "changed-destination", "bad-verification", "echoed-secret"]) {
  test(`${mode} retains incomplete evidence without a success report or echoed data`, () => {
    fixture(mode, ({ result, output, commands }) => {
      assert.notEqual(result.status, 0);
      assert.equal(existsSync(join(output, "report.json")), false);
      const partial = readFileSync(join(output, "partial.json"), "utf8");
      assert.equal(JSON.parse(partial).evidence_tier, "incomplete_live_github_canary");
      assert.equal((result.stdout + result.stderr + partial).includes("do-not-print-this-token"), false);
      for (const clone of commands.filter(row => row.args.includes("clone"))) {
        assert.equal(existsSync(clone.args.at(-1)), false);
      }
    });
  });
}
