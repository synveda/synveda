import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import test from "node:test";
import { checkConsolePackage } from "./check-console-package.mjs";
import { consolePolicy, helperNoticeName, noticesName, sbomName, sha256 } from "../console/build/dependency-contract.mjs";
import { consolePackageFixture as fixture } from "./fixtures/console-package.mjs";
import { renderedConsolePackages } from "../console/build/dependency-plugin.mjs";

const source = "a".repeat(40), version = "0.4.3";

test("packaged console validates actual files, complete notices, dependency graph and source bindings", t => {
  const f = fixture(t); f.pack();
  const report = checkConsolePackage(f.archive, version, source, f.root);
  assert.deepEqual(report.packages, consolePolicy(f.root).packages);
  assert.equal(report.archive_sha256, sha256(readFileSync(f.archive)));
  assert.equal(report.files, f.files.size);
});

test("valid-looking SBOMs cannot omit, substitute or duplicate runtime packages and complete licences", t => {
  const f = fixture(t), original = structuredClone(f.document);
  for (const mutate of [
    d => d.components.pop(), d => d.components.push(structuredClone(d.components[0])),
    d => { d.components[0].version = "0.0.0"; }, d => { d.components[0].name = "foreign"; },
    d => { d.components[0].purl = "pkg:npm/foreign@1.0.0"; },
    d => { d.components[0].evidence.licenses[0].license.text.content = Buffer.from("MIT").toString("base64"); },
    d => { d.components[0].evidence.licenses = []; }, d => { d.components[0].licenses[0].license.id = "ISC"; },
    d => { d.dependencies[0].dependsOn = []; }, d => { d.dependencies[0].dependsOn.push("foreign"); },
    d => { d.metadata.tools[0].version = "3.2.2"; }, d => { d.metadata.component.version = "0.0.0"; },
  ]) {
    const changed = structuredClone(original); mutate(changed);
    f.files.set(sbomName, Buffer.from(JSON.stringify(changed))); f.seal();
    assert.throws(f.check);
  }
});

test("archive binding detects altered chunks and closed inventory omissions", t => {
  const f = fixture(t); f.files.set("assets/index.js", Buffer.from("changed output"));
  assert.throws(f.check, /output inventory differs/);
  f.files.set("foreign.js", Buffer.from("foreign"));
  assert.throws(f.check, /output inventory differs/);
});

test("coherently rehashed notices and font bytes cannot replace their reviewed sources", t => {
  const f = fixture(t), original = new Map(f.files);
  for (const path of [noticesName, helperNoticeName, "assets/Inter-OFL.txt", "assets/InterVariable.woff2"]) {
    f.files.set(path, Buffer.from("changed but rehashed")); f.seal();
    assert.throws(f.check);
    for (const [name, bytes] of original) f.files.set(name, bytes);
  }
});

test("missing source, wrong source and stale build inputs refuse qualification", t => {
  const f = fixture(t);
  for (const id of [null, "b".repeat(40), "main"]) {
    f.inventory.source_sha = id; f.seal();
    assert.throws(f.check, /source differs or is unqualified/);
  }
  f.inventory.source_sha = source; f.seal();
  writeFileSync(join(f.root, "console/vite.config.ts"), "changed source config\n");
  assert.throws(f.check, /input bindings differ/);
});

test("duplicate archive headers and linked members refuse before inventory validation", t => {
  const f = fixture(t); f.pack(["console/sbom.cdx.json"]);
  assert.throws(() => checkConsolePackage(f.archive, version, source, f.root), /duplicate/);
  f.pack(); rmSync(join(f.output, sbomName)); symlinkSync("index.html", join(f.output, sbomName));
  execFileSync("tar", ["-czf", f.archive, "-C", f.stage, "console"]);
  assert.throws(() => checkConsolePackage(f.archive, version, source, f.root), /must be regular/);
});

test("unknown virtual modules, foreign physical sources and traversal-shaped output paths refuse", t => {
  const f = fixture(t);
  const chunk = id => ({ x: { type: "chunk", code: "fixture", modules: { [id]: { renderedLength: 1 } } } });
  for (const id of ["\0unreviewed-helper", "virtual:foreign", "/tmp/foreign.js", "\0/tmp/foreign.js?commonjs-module"])
    assert.throws(() => renderedConsolePackages(chunk(id)));
  f.files.set("../foreign.js", Buffer.from("foreign")); f.seal();
  assert.throws(f.check, /safe relative/);
});

test("oversized console members are refused without needing a product service", t => {
  const f = fixture(t); f.files.set("assets/index.js", Buffer.alloc(8 * 1024 * 1024 + 1)); f.seal();
  assert.throws(f.check, /file size differs/);
});
