#!/usr/bin/env node
// OPS-8/CPR-39/ADPT-9: replay both clients through the extracted archive.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
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
  const members = run("tar", ["-tzf", join(scratch, `synveda-plugin-${version}.tar.gz`)], scratch).trim().split("\n");
  assert.ok(members.every((name) => !name.split("/").some((part) => part.startsWith("._"))), "archive must not carry filesystem metadata members");
  run("tar", ["-xzf", join(scratch, `synveda-plugin-${version}.tar.gz`), "-C", scratch], scratch);
  for (const directory of ["plugin", "plugin/synveda", "plugin/codex", "plugin/copilot-cli",
    "plugin/codex/node_modules/@synveda/claude-code-adapter", "plugin/copilot-cli/node_modules/@synveda/claude-code-adapter"]) {
    for (const name of ["LICENSE", "NOTICE"]) {
      assert.deepEqual(readFileSync(join(scratch, directory, name)), readFileSync(join(root, name)));
    }
  }
  const consumer = JSON.parse(readFileSync(join(scratch, "plugin/synveda/consumer-setup.json"), "utf8"));
  assert.equal(consumer.contract, "OPS-12/ADR-0116");
  assert.equal(consumer.version, 1);
  assert.equal(consumer.config_sha256, createHash("sha256").update(readFileSync(join(scratch, "plugin/synveda/dist/config.mjs"))).digest("hex"));
  const claude = join(scratch, "plugin/synveda/dist");
  const claudeFiles = readdirSync(claude, { withFileTypes: true });
  assert.ok(claudeFiles.every((file) => file.isFile() && file.name.endsWith(".mjs") &&
    !file.name.includes(".test.") && !["mock-gateway.mjs", "driver.mjs", "types.mjs"].includes(file.name)),
  "Claude must carry ordinary runtime modules only");
  for (const file of claudeFiles) {
    assert.deepEqual(readFileSync(join(claude, file.name)), readFileSync(join(root, "adapters/claude-code/dist", file.name)));
  }
  assert.equal(run(process.execPath, [join(claude, "hook.mjs")], scratch, { input: "{}" }), "");
  // Add the test only after checking the shipped set; its imports still resolve
  // to the actual extracted runtime modules.
  cpSync(join(root, "adapters/claude-code/dist/config.test.mjs"), join(claude, "config.test.mjs"));
  process.stdout.write(run(process.execPath, ["--test", "dist/config.test.mjs"], join(scratch, "plugin/synveda")));
  const runtimes = [];
  for (const client of ["codex", "copilot-cli"]) {
    const runtime = join(scratch, "plugin", client);
    const shared = join(runtime, "node_modules/@synveda/claude-code-adapter");
    const files = readdirSync(runtime, { recursive: true, withFileTypes: true });
    assert.ok(files.every((file) => file.isDirectory() || file.isFile()), "archive must not carry symlinks");
    assert.ok(files.every((file) => !file.name.includes(".test.") && !["src", "fixtures"].includes(file.name)));
    for (const directory of [runtime, shared]) {
      const manifest = JSON.parse(readFileSync(join(directory, "package.json"), "utf8"));
      assert.equal(manifest.version, version);
      assert.equal(manifest.private, true);
      assert.equal(manifest.license, "Apache-2.0");
      assert.equal(manifest.devDependencies, undefined);
      assert.equal(manifest.scripts, undefined);
      assert.deepEqual(manifest.dependencies,
        directory === runtime ? { "@synveda/claude-code-adapter": version } : undefined);
    }
    const hook = join(runtime, "dist/hook.mjs");
    assert.equal(createRequire(hook).resolve("@synveda/claude-code-adapter/session-runtime"),
      join(shared, "dist/session-runtime.mjs"), "runtime must resolve inside the extracted archive");
    for (const file of ["hook.mjs", "transcript.mjs"]) {
      assert.deepEqual(readFileSync(join(runtime, "dist", file)), readFileSync(join(root, "adapters", client, "dist", file)));
    }
    for (const file of readdirSync(join(shared, "dist"))) {
      assert.deepEqual(readFileSync(join(shared, "dist", file)), readFileSync(join(root, "adapters/claude-code/dist", file)));
    }
    // Import before placing any test helpers beside either package. Empty input
    // has no product effect; every runtime dependency must already load.
    assert.equal(run(process.execPath, [hook], scratch, { input: "{}" }), "");
    runtimes.push({ client, runtime, shared, fileCount: files.filter((file) => file.isFile()).length });
  }

  // These are test-only inputs, copied after checking both shipped file sets.
  // Production hooks/readers remain the bytes extracted from the archive.
  const helpers = join(scratch, "plugin/claude-code/dist");
  cpSync(join(runtimes[0].shared, "dist"), helpers, { recursive: true });
  cpSync(join(root, "adapters/claude-code/dist/mock-gateway.mjs"), join(helpers, "mock-gateway.mjs"));
  for (const { client, runtime, fileCount } of runtimes) {
    cpSync(join(root, "adapters", client, "fixtures"), join(runtime, "fixtures"), { recursive: true });
    for (const file of ["hook.test.mjs", "transcript.test.mjs"]) {
      cpSync(join(root, "adapters", client, "dist", file), join(runtime, "dist", file));
    }
    process.stdout.write(run(process.execPath, ["--test", "--test-timeout=60000",
      "dist/hook.test.mjs", "dist/transcript.test.mjs"], runtime));
    console.log(JSON.stringify({ evidence: "installed-archive-replay", node: process.version,
      version, client, runtime_files: fileCount, shared_runtime_local: true, native_client_run: false }));
  }
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
