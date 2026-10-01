#!/usr/bin/env node
// OPS-12 / ADR-0134: inspect the exact console TAR; never extract or run its code.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { lstatSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { checkConsoleFiles, maxFiles, maxFileBytes, relativePath, sha256 } from "../console/build/dependency-contract.mjs";

const repository = fileURLToPath(new URL("../", import.meta.url));
export function readConsoleArchive(archive, compressed = true) {
  const stat = lstatSync(archive);
  assert.ok(stat.isFile() && !stat.isSymbolicLink() && stat.nlink === 1 && stat.size > 0 && stat.size <= (compressed ? 16 : 34) * 1024 * 1024,
    "console archive must be a bounded regular unlinked file");
  const options = { timeout: 60_000, maxBuffer: 128 * 1024, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] };
  const names = execFileSync("tar", [compressed ? "-tzf" : "-tf", archive], options).trim().split("\n");
  const details = execFileSync("tar", [compressed ? "-tvzf" : "-tvf", archive], options).trim().split("\n");
  assert.ok(names.length > 5 && names.length <= maxFiles && details.length === names.length, "console archive entry count differs");
  assert.equal(new Set(names).size, names.length, "duplicate console archive member");
  const files = new Map();
  let total = 0;
  for (const [i, name] of names.entries()) {
    assert.ok(name === "console/" || name.startsWith("console/"), "foreign console archive root");
    if (name.endsWith("/")) {
      relativePath(name.slice(0, -1));
      assert.ok(details[i].startsWith("d"), "console archive directory must be regular");
      continue;
    }
    const path = relativePath(name.slice("console/".length));
    assert.ok(details[i].startsWith("-"), "console archive member must be regular");
    const bytes = execFileSync("tar", [compressed ? "-xOzf" : "-xOf", archive, name], { ...options, encoding: undefined, maxBuffer: maxFileBytes });
    total += bytes.length;
    assert.ok(total <= 32 * 1024 * 1024, "console archive expanded bytes exceed the aggregate bound");
    files.set(path, bytes);
  }
  return files;
}

export function checkConsolePackage(archive, version, source, root = repository) {
  const files = readConsoleArchive(archive);
  for (const name of ["LICENSE", "NOTICE"]) {
    assert.deepEqual(files.get(name), readFileSync(new URL(`../${name}`, import.meta.url)), "console first-party notice differs");
    files.delete(name);
  }
  const report = checkConsoleFiles(files, root, version, source);
  return { ...report, archive_sha256: sha256(readFileSync(archive)), source_sha: source, version };
}

export function checkConsoleImageReport(report, consoleArchive, version, source) {
  const { archive_sha256, ...expected } = consoleArchive;
  assert.match(archive_sha256, /^[0-9a-f]{64}$/, "expected inspected console archive evidence");
  assert.equal(expected.source_sha, source, "console archive source differs");
  assert.equal(expected.version, version, "console archive version differs");
  assert.match(expected.inventory_sha256, /^[0-9a-f]{64}$/, "console inventory binding missing");
  assert.deepEqual(report, expected, "product console differs from its archive");
}

if (process.argv[1] && resolve(process.argv[1]) === import.meta.filename) {
  const [archive, version, source] = process.argv.slice(2);
  assert.ok(archive && version && source, "usage: check-console-package.mjs ARCHIVE VERSION SOURCE_SHA");
  console.log(JSON.stringify(checkConsolePackage(archive, version, source)));
}
