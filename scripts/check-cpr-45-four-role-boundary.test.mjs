import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const AUTHORITY_ROOTS = [
  "adapters",
  "console",
  "crates",
  "demos",
  "deploy",
  "policies",
  "scripts",
].map((path) => join(ROOT, path));
const ROOT_AUTHORITY_FILES = [
  ".dockerignore",
  "Cargo.lock",
  "Cargo.toml",
  "deny.toml",
  "package.json",
  "pnpm-lock.yaml",
  "pnpm-workspace.yaml",
  "rust-toolchain.toml",
  "tsconfig.base.json",
].map((path) => join(ROOT, path));
const ALLOWED_PATHS = new Set([
  join(
    ROOT,
    "deploy/compose/scripts/clean-engine-live-provider-effect-fixture-blueprint.mjs",
  ),
  join(ROOT, "scripts/check-cpr-45-four-role-boundary.test.mjs"),
  join(ROOT, "scripts/cpr-45-four-role-fixture.test.mjs"),
  join(
    ROOT,
    "scripts/clean-engine-live-provider-effect-fixture-blueprint.test.mjs",
  ),
  join(ROOT, "scripts/fixtures/cpr-45-four-role/harness.mjs"),
  join(ROOT, "scripts/fixtures/cpr-45-four-role/protocol.mjs"),
  join(ROOT, "scripts/fixtures/cpr-45-four-role/role.mjs"),
]);
const EXCLUDED_DIRECTORIES = new Set([
  ".git",
  "build",
  "coverage",
  "dist",
  "node_modules",
  "target",
]);

function sourcePaths(root) {
  const paths = [];
  const visit = (path) => {
    for (const entry of readdirSync(path, { withFileTypes: true })) {
      const childPath = join(path, entry.name);
      if (entry.isDirectory() && !EXCLUDED_DIRECTORIES.has(entry.name)) {
        visit(childPath);
      } else if (entry.isFile()) paths.push(childPath);
    }
  };
  visit(root);
  return paths;
}

const FORBIDDEN_REFERENCE =
  /cpr-45-four-role|cpr45-four-role|four-role-fixture/u;

function textSource(path) {
  const bytes = readFileSync(path);
  if (bytes.includes(0)) return null;
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return null;
  }
}

test("repository authority surfaces cannot depend on the four-role spike", () => {
  const authorityPaths = [
    ...AUTHORITY_ROOTS.flatMap(sourcePaths),
    ...ROOT_AUTHORITY_FILES,
  ].filter((path) => !ALLOWED_PATHS.has(path));
  assert.ok(authorityPaths.length > 100, "authority scan was unexpectedly small");
  for (const path of authorityPaths) {
    const source = textSource(path);
    if (source !== null) {
      assert.doesNotMatch(source, FORBIDDEN_REFERENCE, path);
    }
  }
  assert.doesNotMatch(
    readFileSync(join(ROOT, "Makefile"), "utf8"),
    /scripts\/fixtures\/cpr-45-four-role/u,
    "Makefile must not execute the fixture runtime directly",
  );
});
