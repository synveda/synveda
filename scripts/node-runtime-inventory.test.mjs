import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { checkClientNodeArchive, checkNodeMetadata, nodePins, nodeSharedLibraries } from "./node-runtime-inventory.mjs";
import { sha256 } from "./client-artifact.mjs";
import { clientNodeFixture } from "./fixtures/client-node.mjs";

const version = "0.4.3";
const source = "a".repeat(40);
const directory = (t) => {
  const path = mkdtempSync(join(tmpdir(), "synveda-node-inventory-test-"));
  t.after(() => rmSync(path, { recursive: true, force: true })); return path;
};
const check = (f, target) => checkClientNodeArchive(f.archive, f.node, target, version, source, f.lock);

test("final TAR and actual ZIP readers bind all six Node targets and complete notices", (t) => {
  for (const target of Object.keys(nodePins.targets)) check(clientNodeFixture(directory(t), target, version, source), target);
});

test("coherently changed runtime bytes or notices refuse the original upstream pins", (t) => {
  for (const target of ["darwin-arm64", "windows-x86_64", "windows-arm64"]) {
    for (const name of ["node", "LICENSE", ...Object.keys(nodePins.supplementary_notices)]) {
      const f = clientNodeFixture(directory(t), target, version, source);
      const path = join(f.runtime, name === "node" ? f.binaryName : name);
      const bytes = readFileSync(path); bytes[bytes.length - 1] ^= 1; writeFileSync(path, bytes);
      if (name === "node") f.node.inventory.binary_sha256 = sha256(bytes);
      else f.node.inventory.notices[name].sha256 = sha256(bytes);
      f.pack();
      assert.throws(() => check(f, target), /hash differs/);
    }
  }
});

test("coherent dependency substitutions and stale manifest identities refuse", (t) => {
  const f = clientNodeFixture(directory(t), "linux-arm64", version, source);
  f.node.inventory.dependencies.openssl = "0.0.0"; f.pack();
  assert.throws(() => check(f, "linux-arm64"), /inventory differs/);
  f.node.inventory.dependencies.openssl = f.lock.dependencies.openssl; f.pack();
  assert.throws(() => checkClientNodeArchive(f.archive, f.node, "linux-arm64", version, "b".repeat(40), f.lock), /source differs/);
  const report = structuredClone(f.node); report.inventory.binary_sha256 = "0".repeat(64);
  assert.throws(() => checkClientNodeArchive(f.archive, report, "linux-arm64", version, source, f.lock), /metadata differs/);
  for (const [name, value] of [["version", "0.0.0"], ["archive", "foreign.zip"], ["archive_sha256", "0".repeat(64)]]) {
    const g = clientNodeFixture(directory(t), "linux-arm64", version, source);
    g.node[name] = value; g.pack(); assert.throws(() => check(g, "linux-arm64"), /runtime version differs|distribution/);
  }
});

test("missing, truncated, duplicate and linked notices cannot satisfy carriage", (t) => {
  for (const mode of ["missing", "truncated", "duplicate", "linked"]) {
    const f = clientNodeFixture(directory(t), "darwin-arm64", version, source);
    const path = join(f.runtime, "nbytes-LICENSE");
    if (mode === "missing" || mode === "linked") rmSync(path);
    if (mode === "truncated") writeFileSync(path, readFileSync(path).subarray(0, 64));
    if (mode === "linked") symlinkSync("LICENSE", path);
    f.pack(mode === "duplicate" ? ["client/plugin/synveda/runtime/nbytes-LICENSE"] : []);
    assert.throws(() => check(f, "darwin-arm64"), /regular file|size differs|Command failed/);
  }
});

test("native metadata rejects wrong libraries, new fields, shared linkage and foreign identity", () => {
  const observed = { versions: { node: nodePins.version, ...nodePins.dependencies, modules: "137", napi: "10", cldr: "48.0", tz: "2026c", unicode: "17.0", nghttp3: "", ngtcp2: "" },
    platform: "darwin", arch: "arm64", shared: Object.fromEntries(nodeSharedLibraries.map((name) => [name, false])) };
  const result = checkNodeMetadata(observed, "darwin-arm64");
  assert.deepEqual(result.dependencies, nodePins.dependencies);
  assert.equal(Object.hasOwn(result.dependencies, "napi"), false);
  for (const alter of [
    (m) => { m.versions.openssl = "0.0.0"; }, (m) => { delete m.versions.sqlite; },
    (m) => { m.versions.unknown = ""; }, (m) => { m.versions.ngtcp2 = "1.0.0"; },
    (m) => { m.shared.openssl = true; }, (m) => { m.arch = "x64"; }, (m) => { m.platform = "win32"; },
  ]) { const changed = structuredClone(observed); alter(changed); assert.throws(() => checkNodeMetadata(changed, "darwin-arm64")); }
});

test("the reviewed pins and supplementary notice bytes match the retained upstream probe", () => {
  const probe = JSON.parse(readFileSync(new URL("../demos/evidence/ops12-node-runtime-probe.json", import.meta.url)));
  assert.equal(nodePins.source_sha, probe.node_sources[nodePins.version].node_source_sha);
  for (const entry of probe.client_upstream_distributions) {
    const pin = nodePins.targets[entry.target];
    assert.equal(pin.sha256, entry.upstream_archive.sha256);
    assert.equal(pin.binary_sha256, entry.binary.sha256); assert.equal(pin.binary_bytes, entry.binary.bytes);
    assert.equal(pin.license_sha256, entry.licence.sha256); assert.equal(pin.license_bytes, entry.licence.bytes);
  }
  assert.equal(nodePins.product.source_sha, probe.node_sources[nodePins.product.version].node_source_sha);
  for (const entry of probe.product.upstream) {
    const pin = nodePins.product.targets[entry.target];
    assert.equal(pin.archive, entry.archive); assert.equal(pin.sha256, entry.archive_sha256);
    assert.equal(pin.binary_sha256, entry.binary.sha256); assert.equal(pin.binary_bytes, entry.binary.bytes);
    assert.equal(pin.license_sha256, entry.licence.sha256); assert.equal(pin.license_bytes, entry.licence.bytes);
  }
  for (const lock of [nodePins, nodePins.product]) {
    for (const [name, pin] of Object.entries(lock.supplementary_notices)) {
      const bytes = readFileSync(new URL(`../assets/licenses/node/${name}`, import.meta.url));
      assert.equal(sha256(bytes), pin.sha256); assert.equal(bytes.length, pin.bytes);
    }
  }
  const review = JSON.parse(readFileSync(new URL("../demos/evidence/ops12-node-source-notice-review.json", import.meta.url)));
  for (const lock of [nodePins, nodePins.product]) {
    const reviewed = review.node_sources[lock.version];
    assert.equal(lock.source_sha, reviewed.source_sha);
    for (const [name, pin] of Object.entries(reviewed.additional_notices))
      assert.deepEqual(lock.supplementary_notices[name], pin);
  }
});
