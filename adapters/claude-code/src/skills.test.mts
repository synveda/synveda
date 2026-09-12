/**
 * The skills seam (SKIL-4, ADR-0054 decisions 16–18) against a stand-in
 * CLI.
 *
 * Two things are being asserted, and only one of them is the happy path.
 *
 * The first is that this adapter **delegates**: it spawns
 * `synveda skill sync` and does not resolve, write, validate a path or
 * decide a directory layout itself. The test reads the argv the stand-in
 * saw, because that argv is the whole of the adapter's contribution.
 *
 * The second is that it will not name a root it was not given. `sync`
 * removes directories, so a plugin root guessed from a default would be a
 * `remove_dir_all` somewhere a person keeps their own work — the adapter
 * does nothing at all rather than guess.
 */

import assert from "node:assert/strict";
import { chmodSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, test } from "node:test";
import { startGateway } from "./mock-gateway.mjs";
import { loadConfig } from "./config.mjs";

process.env.XDG_STATE_HOME = mkdtempSync(join(tmpdir(), "synveda-skills-state-"));

const { governedRoot, syncSkills, distributionScope } = await import("./skills.mjs");

const principalScope = "00000000-0000-4000-8000-000000000001";
const gateway = await startGateway(() => ({
  status: 200,
  body: { anchors: [{ kind: "principal", source: "principal_scope", scope_id: principalScope }] },
}));
after(gateway.close);
process.env.SYNVEDA_GATEWAY = gateway.url;
process.env.SYNVEDA_TOKEN = "synthetic-skill-test-token";
delete process.env.SYNVEDA_WORKSPACE;
delete process.env.SYNVEDA_PROJECT;

const binDir = mkdtempSync(join(tmpdir(), "synveda-fake-skill-cli-"));

function fakeCli(name: string, body: string): string {
  const path = join(binDir, name);
  writeFileSync(path, `#!${process.execPath}\n${body}\n`, "utf8");
  chmodSync(path, 0o755);
  return path;
}

async function withEnv<T>(
  environment: Record<string, string | undefined>,
  body: () => Promise<T>,
): Promise<T> {
  const previous = new Map<string, string | undefined>();
  for (const [key, value] of Object.entries(environment)) {
    previous.set(key, process.env[key]);
    if (value === undefined) delete process.env[key];
    else process.env[key] = value;
  }
  try {
    return await body();
  } finally {
    for (const [key, value] of previous) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  }
}

test("the governed root is the plugin's own skills directory", async () => {
  await withEnv({ CLAUDE_PLUGIN_ROOT: "/opt/plugins/synveda" }, async () => {
    assert.equal(governedRoot(), join("/opt/plugins/synveda", "skills"));
  });
});

test("without a plugin root there is no directory to reconcile, and nothing runs", async () => {
  const record = join(binDir, "must-not-run.json");
  const cli = fakeCli(
    "must-not-run.cjs",
    `require("node:fs").writeFileSync(${JSON.stringify(record)}, "ran");`,
  );
  await withEnv(
    { CLAUDE_PLUGIN_ROOT: undefined, SYNVEDA_CLI: cli, SYNVEDA_PROFILE: undefined },
    syncSkills,
  );
  assert.throws(() => readFileSync(record, "utf8"), /ENOENT/);
});

test("the sync is delegated to the CLI, into the plugin's own root", async () => {
  const pluginRoot = mkdtempSync(join(tmpdir(), "synveda-plugin-"));
  const record = join(binDir, "argv.json");
  const cli = fakeCli(
    "sync.cjs",
    `require("node:fs").writeFileSync(${JSON.stringify(record)}, JSON.stringify(process.argv.slice(2)));
     console.log(JSON.stringify({ root: "x", available: 2, written: [{skill:"a"}], unchanged: ["b"], removed: [] }));`,
  );
  await withEnv(
    { CLAUDE_PLUGIN_ROOT: pluginRoot, SYNVEDA_CLI: cli, SYNVEDA_PROFILE: undefined },
    syncSkills,
  );
  const argv = JSON.parse(readFileSync(record, "utf8")) as string[];
  assert.deepEqual(argv, [
    "skill",
    "sync",
    "--scope",
    principalScope,
    "--client",
    "claude-code",
    "--root",
    join(pluginRoot, "skills"),
    "--json",
  ]);
});

test("an explicit project selects its scope and never falls back when refused or mismatched", () => {
  const personal = { kind: "principal", source: "principal_scope", scope_id: principalScope };
  const me = { anchors: [personal], projects: [{ id: "p1", workspace_id: "w1", scope_id: "s1" }] };
  const config = { ...loadConfig(undefined), workspaceId: "w1", projectId: "p1" };
  assert.equal(distributionScope(me, config), "s1");
  assert.equal(distributionScope(me, { ...config, projectId: "denied" }), undefined);
  assert.equal(distributionScope(me, { ...config, workspaceId: "w2" }), undefined);
  assert.equal(distributionScope(me, { ...config, projectId: undefined }), principalScope);
  assert.equal(distributionScope({}, { ...config, projectId: undefined }), undefined);
});

test("a profile is passed through, because a sync runs as somebody", async () => {
  const pluginRoot = mkdtempSync(join(tmpdir(), "synveda-plugin-profile-"));
  const record = join(binDir, "argv-profile.json");
  const cli = fakeCli(
    "sync-profile.cjs",
    `require("node:fs").writeFileSync(${JSON.stringify(record)}, JSON.stringify(process.argv.slice(2)));
     console.log("{}");`,
  );
  await withEnv(
    { CLAUDE_PLUGIN_ROOT: pluginRoot, SYNVEDA_CLI: cli, SYNVEDA_PROFILE: "work" },
    syncSkills,
  );
  const argv = JSON.parse(readFileSync(record, "utf8")) as string[];
  assert.deepEqual(argv.slice(-2), ["--profile", "work"]);
});

test("a CLI that fails, hangs up, or is not installed costs the session nothing", async () => {
  const pluginRoot = mkdtempSync(join(tmpdir(), "synveda-plugin-fail-"));
  const cases = [
    fakeCli("refuses.cjs", `console.error("not logged in"); process.exit(1);`),
    fakeCli("garbage.cjs", `console.log("<!doctype html>");`),
    join(binDir, "definitely-not-installed"),
  ];
  for (const cli of cases) {
    // The assertion is that this resolves at all: every caller is a hook
    // that must exit 0 (ADR-0027 decision 3), so a throw here would be a
    // broken session rather than a missing skill.
    await withEnv(
      { CLAUDE_PLUGIN_ROOT: pluginRoot, SYNVEDA_CLI: cli, SYNVEDA_PROFILE: undefined },
      syncSkills,
    );
  }
});
