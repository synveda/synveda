import assert from "node:assert/strict";
import test from "node:test";
import { productPackageFixture } from "./fixtures/product-packages.mjs";
import { checkInstalledPackageDatabase, checkProductPackageImageReport, checkProductPackagePlan, packageDatabase, productPackageInventory } from "./product-package-inventory.mjs";
import { inspectProductPackages } from "./verify-release-images.mjs";
import { sha256 } from "./client-artifact.mjs";

const digest = `sha256:${"1".repeat(64)}`;

test("actual package identities, notice hashes and library ownership bind each native platform", () => {
  for (const arch of ["amd64", "arm64"]) {
    const f = productPackageFixture(arch);
    assert.equal(f.plan.packages.length, 3); assert.equal(f.plan.files.length, 12);
    checkInstalledPackageDatabase(f.bytes.get(packageDatabase), f.plan);
    checkProductPackageImageReport(f.report, f.plan, `linux/${arch}`, digest);
    assert.throws(() => checkProductPackagePlan(f.plan, arch === "arm64" ? "linux/amd64" : "linux/arm64", digest), /platform differs/);
    assert.throws(() => checkProductPackagePlan(f.plan, `linux/${arch}`, `sha256:${"2".repeat(64)}`), /image differs/);
  }
});

test("SPDX envelopes cannot hide missing versions, copyrights, native hashes or ownership", () => {
  const controls = [
    s => { s.packages[0].versionInfo = "0"; },
    s => { s.packages[0].externalRefs.push(s.packages[0].externalRefs[0]); },
    s => { s.packages[0].externalRefs[0].referenceLocator += "&arch=arm64"; },
    s => { s.packages[0].externalRefs[0].referenceLocator = s.packages[0].externalRefs[0].referenceLocator.replace("arch=arm64", "arch=amd64"); },
    s => { s.packages[0].sourceInfo = "acquired package info from DPKG DB: /foreign/status"; },
    s => { s.packages.push(structuredClone(s.packages[0])); },
    s => { s.files = s.files.filter(f => !f.fileName.endsWith("libssl3/copyright")); },
    s => { s.files = s.files.filter(f => !f.fileName.endsWith("common-licenses/GPL-2")); },
    s => { s.files = s.files.filter(f => f.fileName !== "var/lib/dpkg/status"); },
    s => { s.files = s.files.filter(f => !f.fileName.endsWith("libcrypto.so.3")); },
    s => { s.files[0].checksums = []; },
    s => { s.files[0].checksums.push(s.files[0].checksums[0]); },
    s => { s.files[0].checksums[0].checksumValue = "0".repeat(64); },
    s => { s.files[0].fileTypes = ["OTHER"]; },
    s => { s.files.push(structuredClone(s.files[0])); },
    s => { s.files[1].SPDXID = s.files[0].SPDXID; },
    s => { s.packages[0].sourceInfo = "acquired package info from DPKG DB: /var/lib/dpkg/status, /usr/share/doc/../foreign/copyright"; },
    s => { s.relationships[0].spdxElementId = "SPDXRef-Package-libc6"; },
    s => { s.relationships = []; },
  ];
  for (const change of controls) {
    const f = productPackageFixture(); change(f.spdx);
    assert.throws(() => productPackageInventory(f.spdx, "linux/arm64", digest));
  }
});

test("full package database corroboration refuses omissions, changed sources and duplicate identities", () => {
  const f = productPackageFixture(); const original = f.bytes.get(packageDatabase).toString();
  for (const changed of [original.replace("Version: 2.36-9+deb12u14", "Version: 0"), original.replace("Source: glibc", "Source: foreign"),
    original + original.split("\n\n")[0] + "\n\n", original.replace("Package: libc6", "Package: libc6\nPackage: libc6"),
    original.replace("Status: install ok installed", "Status: install ok unpacked")])
    assert.throws(() => checkInstalledPackageDatabase(Buffer.from(changed), f.plan));
  const changed = { ...f.plan, packages: f.plan.packages.slice(1) };
  assert.throws(() => checkProductPackagePlan(changed), /missing native library package/);
});

test("stopped product inspection rehashes full bytes and never starts the container", () => {
  const f = productPackageFixture(); const calls = [];
  const run = (args, timeout, binary, maxBytes) => {
    calls.push(args);
    if (args[0] === "create") return "c".repeat(64);
    if (args[0] === "cp") {
      assert.ok(binary && timeout > 0 && timeout <= 60_000 && maxBytes <= 34 * 1024 * 1024);
      return f.tar(args[1].split(":")[1]);
    }
    assert.equal(args[0], "rm"); return "";
  };
  assert.deepEqual(inspectProductPackages(digest, "linux/arm64", f.plan, run), f.report);
  const create = calls[0], name = create[create.indexOf("--name") + 1];
  assert.deepEqual(calls.at(-1), ["rm", "--force", "--volumes", name]);
  for (const arg of ["--pull=never", "--network=none", "--read-only", "--cap-drop=ALL", "--security-opt=no-new-privileges", "--user=65532:65532"])
    assert.ok(create.includes(arg));
  assert.equal(calls.filter(args => args[0] === "cp").length, 12);
});

test("changed full notices, native bytes and duplicate members refuse with owned cleanup", () => {
  for (const kind of ["notice", "database", "library"]) {
    const f = productPackageFixture(); const file = f.plan.files.find(file => file.kind === kind); const calls = [];
    f.bytes.get(file.path)[0] ^= 1;
    const run = args => { calls.push(args); return args[0] === "create" ? "c".repeat(64) : args[0] === "cp" ? f.tar(args[1].split(":")[1]) : ""; };
    assert.throws(() => inspectProductPackages(digest, "linux/arm64", f.plan, run), /hash differs/);
    assert.equal(calls.at(-1)[0], "rm");
  }
  const f = productPackageFixture();
  assert.throws(() => inspectProductPackages(digest, "linux/arm64", f.plan, args => args[0] === "create" ? "c".repeat(64)
    : args[0] === "cp" ? f.tar(args[1].split(":")[1], [args[1].split("/").at(-1)]) : ""), /one regular file/);
});

test("inspection/cleanup failures and stream bounds cannot retain passing package evidence", () => {
  const cause = new Error("package read failed"), cleanup = new Error("cleanup failed");
  for (const stage of ["create", "cp"]) {
    const f = productPackageFixture(); const calls = [];
    assert.throws(() => inspectProductPackages(digest, "linux/arm64", f.plan, args => {
      calls.push(args); if (args[0] === stage) throw cause; if (args[0] === "rm") throw cleanup; return "c".repeat(64);
    }), error => error === cause);
    assert.equal(calls.at(-1)[0], "rm");
  }
  const f = productPackageFixture();
  assert.throws(() => inspectProductPackages(digest, "linux/arm64", f.plan, args => {
    if (args[0] === "create") return "c".repeat(64); if (args[0] === "cp") return f.tar(args[1].split(":")[1]); throw cleanup;
  }), error => error === cleanup);
  for (const value of [Buffer.alloc(0), "invalid stream", Buffer.alloc(34 * 1024 * 1024 + 1)])
    assert.throws(() => inspectProductPackages(digest, "linux/arm64", f.plan, args => args[0] === "create" ? "c".repeat(64) : args[0] === "cp" ? value : ""), /stream exceeds/);
});

test("assembly refuses coherently changed image evidence against the original OCI notice inventory", () => {
  const f = productPackageFixture(); const changed = structuredClone(f.report);
  changed.files.find(file => file.kind === "notice").sha256 = "2".repeat(64);
  assert.throws(() => checkProductPackageImageReport(changed, f.plan, "linux/arm64", digest), /hash differs/);
  changed.files[0].bytes = 0;
  assert.throws(() => checkProductPackageImageReport(changed, f.plan, "linux/arm64", digest), /size exceeds/);
  const noFile = { ...f.report, files: f.report.files.slice(1) };
  assert.throws(() => checkProductPackageImageReport(noFile, f.plan, "linux/arm64", digest), /inventory differs/);
});

test("coherent library and database digests cannot hide foreign ELF or package identities", () => {
  for (const kind of ["library", "database"]) {
    const f = productPackageFixture(); const file = f.plan.files.find(file => file.kind === kind);
    if (kind === "library") f.bytes.get(file.path).writeUInt16LE(62, 18);
    else f.bytes.set(file.path, Buffer.from(f.bytes.get(file.path).toString().replace("Source: glibc", "Source: foreign")));
    file.sha256 = sha256(f.bytes.get(file.path));
    assert.throws(() => inspectProductPackages(digest, "linux/arm64", f.plan, args => args[0] === "create" ? "c".repeat(64)
      : args[0] === "cp" ? f.tar(args[1].split(":")[1]) : ""), kind === "library" ? /ELF architecture differs/ : /database identities differ/);
  }
});

test("the package inspection deadline refuses further copies and still removes its container", t => {
  const f = productPackageFixture(), calls = [];
  t.mock.timers.enable({ apis: ["Date"] });
  assert.throws(() => inspectProductPackages(digest, "linux/arm64", f.plan, args => {
    calls.push(args); if (args[0] === "create") { t.mock.timers.tick(180_001); return "c".repeat(64); } return "";
  }), /deadline exceeded/);
  assert.deepEqual(calls.map(args => args[0]), ["create", "rm"]);
});
