#!/usr/bin/env node
// OPS-8/CPR-39: reuse captured lifecycle tests through the extracted archive.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { cpSync, mkdtempSync, readFileSync, readdirSync, realpathSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const version = JSON.parse(readFileSync(join(root, "adapters/codex/package.json"), "utf8")).version;
const scratch = realpathSync(mkdtempSync(join(tmpdir(), "synveda-plugin-package-")));
function run(command, args, cwd, extra = {}) {
  return execFileSync(command, args, { cwd, encoding: "utf8", timeout: 120_000,
    maxBuffer: 8 * 1024 * 1024, env: { ...process.env, NODE_PATH: "" }, ...extra });
}

try {
  run("bash", ["scripts/package-plugin.sh", version, scratch], root);
  run("tar", ["-xzf", join(scratch, `synveda-plugin-${version}.tar.gz`), "-C", scratch], scratch);
  const codex = join(scratch, "plugin/codex");
  const shared = join(codex, "node_modules/@synveda/claude-code-adapter");
  const files = readdirSync(codex, { recursive: true, withFileTypes: true });
  assert.ok(files.every((file) => file.isDirectory() || file.isFile()), "archive must not carry symlinks");
  assert.ok(files.every((file) => !file.name.includes(".test.") && !["src", "fixtures"].includes(file.name)));
  for (const directory of [codex, shared]) {
    const manifest = JSON.parse(readFileSync(join(directory, "package.json"), "utf8"));
    assert.equal(manifest.version, version);
    assert.equal(manifest.private, true);
    assert.equal(manifest.devDependencies, undefined);
    assert.equal(manifest.scripts, undefined);
  }
  const hook = join(codex, "dist/hook.mjs");
  assert.equal(createRequire(hook).resolve("@synveda/claude-code-adapter/session-runtime"),
    join(shared, "dist/session-runtime.mjs"), "runtime must resolve inside the extracted archive");
  for (const file of ["hook.mjs", "transcript.mjs"]) {
    assert.deepEqual(readFileSync(join(codex, "dist", file)), readFileSync(join(root, "adapters/codex/dist", file)));
  }
  for (const file of readdirSync(join(shared, "dist"))) {
    assert.deepEqual(readFileSync(join(shared, "dist", file)), readFileSync(join(root, "adapters/claude-code/dist", file)));
  }
  // Import before placing any test helpers beside the package. Empty input has
  // no product effect, but every runtime dependency must already load correctly.
  assert.equal(run(process.execPath, [hook], scratch, { input: "{}" }), "");

  // These are test-only inputs, copied after checking the shipped file set.
  // Production hooks/readers remain the bytes extracted from the archive.
  cpSync(join(root, "adapters/codex/fixtures"), join(codex, "fixtures"), { recursive: true });
  for (const file of ["hook.test.mjs", "transcript.test.mjs"]) {
    cpSync(join(root, "adapters/codex/dist", file), join(codex, "dist", file));
  }
  const helpers = join(scratch, "plugin/claude-code/dist");
  cpSync(join(shared, "dist"), helpers, { recursive: true });
  cpSync(join(root, "adapters/claude-code/dist/mock-gateway.mjs"), join(helpers, "mock-gateway.mjs"));
  process.stdout.write(run(process.execPath, ["--test", "--test-timeout=60000",
    "dist/hook.test.mjs", "dist/transcript.test.mjs"], codex));
  console.log(JSON.stringify({ evidence: "installed-archive-replay", node: process.version,
    version, codex_runtime_files: files.filter((file) => file.isFile()).length,
    shared_runtime_local: true, native_client_run: false }));
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
