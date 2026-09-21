#!/usr/bin/env node
// OPS-12: native extracted-archive execution, not native harness qualification.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { nodePath, sha256, targetName, validateClient } from "./client-artifact.mjs";

const root = resolve(import.meta.dirname, "..");
const [version, archiveArgument, report] = process.argv.slice(2);
if (!report || process.argv.length !== 5) throw new Error("usage: check-client-package.mjs VERSION ARCHIVE REPORT");
const archive = resolve(archiveArgument);
const scratch = realpathSync(mkdtempSync(join(tmpdir(), "synveda-client-check-")));
const checks = [];
const run = (command, args, options = {}) => execFileSync(command, args, { encoding: "utf8", timeout: 120_000,
  maxBuffer: 8 * 1024 * 1024, env: { ...process.env, NODE_OPTIONS: "", NODE_PATH: "" }, ...options });
const start = performance.now();
try {
  run("tar", ["-xzf", archive, "-C", scratch]);
  const client = join(scratch, "client");
  const { manifest, digest } = validateClient(client);
  assert.equal(manifest.version, version);
  assert.equal(manifest.target, targetName());
  assert.ok(!Object.keys(manifest.files).some((path) => /synveda-(gateway|worker)|^console\/|^reference\//.test(path)));
  checks.push("native-identity-and-client-only-inventory");
  const tools = join(scratch, "tools");
  mkdirSync(tools);
  for (const tool of ["sh", "uname", "curl", "tar", "awk", "grep", "cut", "mktemp", "rm", "shasum"]) {
    const executable = run("/bin/sh", ["-c", `command -v ${tool}`]).trim();
    symlinkSync(executable, join(tools, tool));
  }
  const assets = join(scratch, "assets");
  mkdirSync(assets);
  const assetName = `synveda-client-${version}-${manifest.target}.tar.gz`;
  symlinkSync(archive, join(assets, assetName));
  writeFileSync(join(assets, "SHA256SUMS"), `${sha256(readFileSync(archive))}  ${assetName}\n`);
  const home = join(scratch, "user home ' ✓");
  const installed = join(home, ".synveda");
  const env = { HOME: home, PATH: tools, SYNVEDA_HOME: installed, SYNVEDA_BIN: join(installed, "bin"),
    SYNVEDA_BASE_URL: `file://${assets}`, SYNVEDA_VERSION: version, SYNVEDA_INSTALL_MODE: "client" };
  const cold = performance.now();
  run("/bin/sh", [join(root, "scripts/install.sh")], { env });
  const coldMs = performance.now() - cold;
  const current = join(installed, "client/current");
  assert.equal(validateClient(realpathSync(current)).digest, digest);
  for (const path of [".claude", ".codex", ".copilot", ".config/synveda", ".local/state/synveda"]) assert.equal(existsSync(join(home, path)), false);
  assert.equal(run(join(installed, "bin/synveda"), ["--version"], { env }).trim(), manifest.cli_version);
  const pluginPlan = run(join(installed, "bin/synveda"), ["plugin", "install", "--scope", "project", "--dry-run"], { env, cwd: scratch });
  assert.ok(pluginPlan.includes(join(current, "plugin")), "installed CLI must discover the private-runtime marketplace");
  assert.equal(run(join(current, nodePath), ["--version"], { env }).trim(), `v${manifest.node.version}`);
  for (const clientName of ["synveda", "codex", "copilot-cli"]) {
    assert.equal(run(join(current, nodePath), [join(current, "plugin", clientName, "dist/hook.mjs")], { env, input: "{}" }), "");
  }
  assert.equal(statSync(join(installed, "client")).mode & 0o777, 0o700);
  assert.equal(statSync(join(installed, "client/install.json")).mode & 0o777, 0o600);
  checks.push("restricted-path-install-cli-and-three-hook-launches", "private-install-without-harness-or-credential-mutation");
  mkdirSync(join(installed, "state"));
  writeFileSync(join(installed, "state/retain"), "retained deployment bytes");
  const warm = performance.now();
  run("/bin/sh", [join(root, "scripts/install.sh")], { env });
  const warmMs = performance.now() - warm;
  assert.equal(readFileSync(join(installed, "state/retain"), "utf8"), "retained deployment bytes");
  assert.equal(validateClient(realpathSync(current)).digest, digest);
  checks.push("repeat-install-preserves-deployment-state");
  // Add test-only inputs to a separate extracted copy, after validating the
  // shipped inventory. Hooks and their dependencies remain the packaged bytes.
  const replay = join(scratch, "replay");
  cpSync(client, replay, { recursive: true });
  const helpers = join(replay, "plugin/claude-code/dist");
  cpSync(join(replay, "plugin/codex/node_modules/@synveda/claude-code-adapter/dist"), helpers, { recursive: true });
  cpSync(join(root, "adapters/claude-code/dist/mock-gateway.mjs"), join(helpers, "mock-gateway.mjs"));
  for (const adapter of ["codex", "copilot-cli"]) {
    const runtime = join(replay, "plugin", adapter);
    cpSync(join(root, "adapters", adapter, "fixtures"), join(runtime, "fixtures"), { recursive: true });
    for (const test of ["hook.test.mjs", "transcript.test.mjs"]) cpSync(join(root, "adapters", adapter, "dist", test), join(runtime, "dist", test));
    process.stdout.write(run(join(replay, nodePath), ["--test", "--test-timeout=60000", "dist/hook.test.mjs", "dist/transcript.test.mjs"], { cwd: runtime }));
    checks.push(`${adapter}-extracted-lifecycle-replay`);
  }
  const result = { schema_version: 1, evidence: "native-client-archive", target: manifest.target,
    version, cli_version: manifest.cli_version, source_sha: manifest.source_sha, source_tree_dirty: manifest.source_tree_dirty, node: manifest.node,
    archive_sha256: sha256(readFileSync(archive)), archive_bytes: statSync(archive).size,
    local_install_ms: Math.round(coldMs), repeat_install_ms: Math.round(warmMs), total_ms: Math.round(performance.now() - start),
    network_download_measured: false, real_issuer_login: false, native_harness_run: false, checks };
  writeFileSync(report, `${JSON.stringify(result, null, 2)}\n`);
  console.log(JSON.stringify(result));
} finally { rmSync(scratch, { recursive: true, force: true }); }
