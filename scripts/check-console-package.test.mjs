import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { checkConsolePackage } from "./check-console-package.mjs";
import { checkConsoleFiles, consoleNotices, consolePolicy, helperNoticeName, inputBindings, inputPaths,
  inventoryName, noticesName, sbomName, sha256 } from "../console/build/dependency-contract.mjs";
import { renderedConsolePackages } from "../console/build/dependency-plugin.mjs";

const repository = fileURLToPath(new URL("../", import.meta.url));
const source = "a".repeat(40), version = "0.4.3";
function fixture(t) {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-console-inventory-"));
  t.after(() => rmSync(scratch, { recursive: true, force: true }));
  const root = join(scratch, "source"), stage = join(scratch, "stage"), output = join(stage, "console");
  const helperNotice = Buffer.from("Complete generated-helper notice fixture.\n");
  for (const path of [...inputPaths, "assets/brand/fonts/InterVariable.woff2", "assets/brand/fonts/OFL.txt"]) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    copyFileSync(join(repository, path), join(root, path));
  }
  const policyPath = join(root, "console/dependency-policy.json"), policy = JSON.parse(readFileSync(policyPath));
  policy.helper_notice_sha256 = sha256(helperNotice);
  writeFileSync(policyPath, JSON.stringify(policy));
  const expected = consolePolicy(root), licence = readFileSync(new URL("./fixtures/console-runtime-LICENSE", import.meta.url));
  const rootRef = `pkg:npm/%40synveda/console@${version}`;
  const components = expected.packages.map(({ name, version }) => ({
    name, version, type: "library", purl: `pkg:npm/${name}@${version}`, "bom-ref": `pkg:npm/${name}@${version}`,
    licenses: [{ license: { id: "MIT", acknowledgement: "declared" } }],
    evidence: { licenses: [{ license: { name: "file: LICENSE", text: { content: licence.toString("base64"), contentType: "text/plain", encoding: "base64" } } }] },
  }));
  const ref = name => components.find(c => c.name === name).purl;
  const document = {
    bomFormat: "CycloneDX", specVersion: "1.6", version: 1,
    metadata: { component: { name: "console", group: "@synveda", version, type: "application", purl: rootRef, "bom-ref": rootRef },
      tools: [{ name: "rollup-plugin-sbom", version: "4.0.0" }, { name: "vite", version: expected.vite_version }, { name: "rollup", version: expected.rollup_version }] },
    components, dependencies: [
      { ref: rootRef, dependsOn: [ref("react"), ref("react-dom")] }, { ref: ref("react") },
      { ref: ref("react-dom"), dependsOn: [ref("react"), ref("scheduler")] }, { ref: ref("scheduler") },
    ],
  };
  const files = new Map([
    ["index.html", Buffer.from('<script src="assets/index.js"></script>')],
    ["assets/index.js", Buffer.from("/* synthetic packaged console */")],
    ["assets/InterVariable.woff2", readFileSync(join(root, "assets/brand/fonts/InterVariable.woff2"))],
    ["assets/Inter-OFL.txt", readFileSync(join(root, "assets/brand/fonts/OFL.txt"))],
    [helperNoticeName, helperNotice], [sbomName, Buffer.from(JSON.stringify(document))],
    [noticesName, consoleNotices(document, expected, helperNotice, readFileSync(join(root, "assets/brand/fonts/OFL.txt")))],
  ]);
  const inventory = {
    schema_version: 1, evidence: "rendered-console-dependency-inventory", version, source_sha: source,
    inputs: inputBindings(root), runtime_packages: expected.packages.map(p => ({ ...p, modules: ["index.js"] })),
    generated_helpers: { provider: "vite", version: expected.vite_version, kinds: ["commonjs", "modulepreload-polyfill"], notice_sha256: expected.helper_notice_sha256 },
  };
  function seal() {
    inventory.files = [...files].filter(([path]) => path !== inventoryName).map(([path, bytes]) =>
      ({ path, bytes: bytes.length, sha256: sha256(bytes) })).sort((a, b) => a.path.localeCompare(b.path));
    files.set(inventoryName, Buffer.from(JSON.stringify(inventory)));
  }
  seal();
  const archive = join(scratch, "console.tar.gz");
  function pack(extra = []) {
    rmSync(stage, { recursive: true, force: true });
    for (const [path, bytes] of files) { mkdirSync(dirname(join(output, path)), { recursive: true }); writeFileSync(join(output, path), bytes); }
    for (const name of ["LICENSE", "NOTICE"]) copyFileSync(join(repository, name), join(output, name));
    execFileSync("tar", ["-czf", archive, "-C", stage, "console", ...extra]);
  }
  return { root, stage, output, files, inventory, document, archive, seal, pack,
    check: () => checkConsoleFiles(files, root, version, source) };
}

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
