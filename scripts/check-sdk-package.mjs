#!/usr/bin/env node
// ADPT-4: exercise an installed archive, without resolving the SDK from source.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const source = join(root, "sdks/typescript");
const scratch = realpathSync(mkdtempSync(join(tmpdir(), "synveda-sdk-npm-")));
const env = { ...process.env, npm_config_cache: join(scratch, "npm-cache"),
  npm_config_offline: "true", npm_config_loglevel: "silent", npm_config_ignore_scripts: "false" };
const sha256 = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");
function run(command, args, cwd) {
  return execFileSync(command, args, { cwd, env, encoding: "utf8", timeout: 120_000, maxBuffer: 8 * 1024 * 1024 });
}

try {
  const builds = [];
  for (const name of ["first", "second"]) {
    const stage = join(scratch, name);
    mkdirSync(stage);
    for (const file of ["package.json", "tsconfig.json", "src"]) cpSync(join(source, file), join(stage, file), { recursive: true });
    // Reuse locked compiler dependencies, not pnpm's location-dependent bin shim.
    mkdirSync(join(stage, "node_modules/.bin"), { recursive: true });
    for (const dependency of ["typescript", "@types"]) {
      symlinkSync(join(source, "node_modules", dependency), join(stage, "node_modules", dependency), "dir");
    }
    symlinkSync(join(source, "node_modules/typescript/bin/tsc"), join(stage, "node_modules/.bin/tsc"));
    const [packed] = JSON.parse(run("npm", ["pack", "--json", "--pack-destination", stage], stage));
    const files = new Set(packed.files.map((file) => file.path));
    for (const file of ["dist/client.mjs", "dist/client.d.mts", "dist/generated/api.d.ts", "dist/generated/contract.js"]) {
      assert.ok(files.has(file), `archive is missing ${file}`);
    }
    assert.ok(![...files].some((file) => file.includes(".test.") || file.includes("workflow") || file.includes("node_modules")));
    builds.push({ stage, archive: join(stage, packed.filename), fileCount: files.size });
  }
  assert.equal(sha256(builds[0].archive), sha256(builds[1].archive), "two clean npm builds must agree");

  const consumer = join(scratch, "consumer");
  mkdirSync(consumer);
  writeFileSync(join(consumer, "package.json"), JSON.stringify({ name: "sdk-package-check", private: true, type: "module" }));
  run("npm", ["install", "--offline", "--ignore-scripts", "--no-audit", "--no-fund", "--package-lock=false", "--engine-strict", builds[0].archive], consumer);
  const installed = join(consumer, "node_modules/@synveda/sdk");
  assert.equal(realpathSync(installed), installed, "install must extract the archive, not link the source checkout");
  const resolved = run(process.execPath, ["--input-type=module", "-e", "console.log(import.meta.resolve('@synveda/sdk'))"], consumer).trim();
  assert.equal(resolved, pathToFileURL(join(installed, "dist/client.mjs")).href);

  writeFileSync(join(consumer, "consumer.mts"), `
import { Client, type OpenSessionBody } from "@synveda/sdk";
const body: OpenSessionBody = { workspace_id: "synthetic", client_name: "package-check" };
async function read(client: Client): Promise<string> {
  const response = await client.request("get_session", { path: { session_id: "synthetic" } });
  // @ts-expect-error Generated Session identifiers are strings.
  const wrong: number = response.data.id;
  // @ts-expect-error Unknown operations must not become an untyped escape hatch.
  await client.request("unknown_operation", {});
  return response.data.id;
}
void body; void read;
`);
  run(process.execPath, [join(source, "node_modules/typescript/bin/tsc"), "--module", "NodeNext", "--target", "ES2022", "--strict", "--noEmit", "consumer.mts"], consumer);

  mkdirSync(join(consumer, "dist"));
  mkdirSync(join(scratch, "fixtures"));
  cpSync(join(root, "sdks/fixtures/wire.json"), join(scratch, "fixtures/wire.json"));
  cpSync(join(builds[0].stage, "dist/client.test.mjs"), join(consumer, "dist/client.test.mjs"));
  process.stdout.write(run(process.execPath, ["--test", "--test-timeout=30000", "dist/client.test.mjs"], consumer));
  console.log(JSON.stringify({ package: "@synveda/sdk", node: process.version, archive_sha256: sha256(builds[0].archive),
    files: builds[0].fileCount, clean_builds_identical: true, installed_exports_and_types: true }));
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
