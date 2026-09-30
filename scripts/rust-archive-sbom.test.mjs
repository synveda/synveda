import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { sha256, targetName } from "./client-artifact.mjs";
import { downloadSyft, syftPins } from "./download-syft.mjs";
import { checkNativeBinary, checkNativeRustSpdx, checkRustArchiveReport, readRustBinary, rustArchiveAssets, rustArchivePlan, rustArchivePlans } from "./rust-archive-sbom.mjs";

const version = "0.4.3";
const source = "a".repeat(40);
const bytesFor = (target) => {
  const bytes = Buffer.alloc(128);
  const arm = target.endsWith("arm64");
  if (target.startsWith("linux-")) {
    Buffer.from("7f454c460201", "hex").copy(bytes);
    bytes.writeUInt16LE(arm ? 183 : 62, 18);
  } else if (target.startsWith("darwin-")) {
    bytes.writeUInt32LE(0xfeedfacf, 0);
    bytes.writeUInt32LE(arm ? 0x100000c : 0x1000007, 4);
  } else {
    bytes.write("MZ"); bytes.writeUInt32LE(64, 0x3c);
    Buffer.from("50450000", "hex").copy(bytes, 64);
    bytes.writeUInt16LE(arm ? 0xaa64 : 0x8664, 68);
    bytes.writeUInt16LE(0x20b, 88);
  }
  return bytes;
};
const spdxFor = (binary, binaryHash) => ({
  spdxVersion: "SPDX-2.3", creationInfo: { creators: ["Tool: syft-1.51.0"] },
  files: [{ fileName: binary.member.split("/").at(-1), checksums: [{ algorithm: "SHA256", checksumValue: binaryHash }] }],
  packages: Object.entries({ [binary.kind === "cli" ? "synveda-cli" : "synveda-gateway"]: version,
    "cedar-policy": "4.11.2", "cedar-policy-core": "4.11.2", sqlx: "0.8.6", "sqlx-postgres": "0.8.6" }).map(([name, versionInfo]) => ({
    name, versionInfo, externalRefs: [{ referenceType: "purl", referenceLocator: `pkg:cargo/${name}@${versionInfo}` }],
  })),
});

function fixture(t, kind = "server", target = "darwin-arm64") {
  const directory = mkdtempSync(join(tmpdir(), "synveda-archive-sbom-test-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const plan = rustArchivePlan(kind, target, version);
  const stage = join(directory, "stage"); mkdirSync(stage);
  const docs = [];
  const binaries = plan.binaries.map((binary, i) => {
    const bytes = bytesFor(target); bytes[100] = i;
    mkdirSync(join(stage, binary.member, ".."), { recursive: true });
    writeFileSync(join(stage, binary.member), bytes);
    const binaryHash = sha256(bytes);
    const doc = spdxFor(binary, binaryHash); docs.push(doc);
    const spdx = Buffer.from(JSON.stringify(doc));
    writeFileSync(join(directory, binary.sbom), spdx);
    return { member: binary.member, platform: checkNativeBinary(bytes, target), binary_sha256: binaryHash, binary_bytes: bytes.length,
      sbom: binary.sbom, sbom_sha256: sha256(spdx), sbom_bytes: spdx.length, inventory: checkNativeRustSpdx(doc, binaryHash, binary, version) };
  });
  const archive = join(directory, plan.archive);
  const pack = (extra = []) => execFileSync("tar", ["-czf", archive, "-C", stage, ...plan.binaries.map((binary) => binary.member), ...extra]);
  pack();
  const pin = syftPins.targets[target];
  const report = { schema_version: 1, evidence: "native-rust-archive-sbom", kind, target, version, source_sha: source, source_tree_dirty: false,
    archive_sha256: sha256(readFileSync(archive)), archive_bytes: readFileSync(archive).length,
    scanner_download: { version: "1.51.0", archive: pin.archive, archive_sha256: pin.sha256, binary_sha256: pin.binary_sha256 }, binaries };
  const write = () => writeFileSync(join(directory, plan.report), JSON.stringify(report));
  const writeDoc = (i) => {
    const bytes = Buffer.from(JSON.stringify(docs[i]));
    writeFileSync(join(directory, plan.binaries[i].sbom), bytes);
    report.binaries[i].sbom_sha256 = sha256(bytes); report.binaries[i].sbom_bytes = bytes.length; write();
  };
  write();
  return { directory, plan, report, docs, stage, pack, archive, write, writeDoc };
}
const check = (f, expectedSource = source) => checkRustArchiveReport(f.directory, f.plan, version, expectedSource);

test("one closed sidecar inventory covers six clients and both three-binary servers", () => {
  const plans = rustArchivePlans(version);
  assert.equal(plans.filter((plan) => plan.kind === "client").length, 6);
  assert.equal(plans.filter((plan) => plan.kind === "server").length, 2);
  assert.equal(rustArchiveAssets(version).length, 20);
  assert.equal(new Set(rustArchiveAssets(version)).size, 20);
  assert.throws(() => rustArchivePlan("server", "windows-arm64", version), /unsupported/);
  assert.throws(() => rustArchivePlan("client", "linux-other", version), /unreviewed/);
  assert.throws(() => rustArchivePlan("client", "linux-arm64", "../0.4.3"));
  assert.equal(Object.keys(syftPins.targets).length, 6);
  for (const pin of Object.values(syftPins.targets)) {
    assert.match(pin.sha256, /^[0-9a-f]{64}$/); assert.match(pin.binary_sha256, /^[0-9a-f]{64}$/);
  }
});

test("ELF, Mach-O and PE headers bind all six native architectures without execution", () => {
  for (const target of Object.keys(syftPins.targets)) {
    assert.deepEqual(checkNativeBinary(bytesFor(target), target), { os: target.split("-")[0], arch: target.split("-")[1] });
    const wrong = target.endsWith("arm64") ? target.replace("arm64", "x86_64") : target.replace("x86_64", "arm64");
    assert.throws(() => checkNativeBinary(bytesFor(wrong), target), /architecture differs/);
    assert.throws(() => checkNativeBinary(bytesFor(target).subarray(0, 32), target), /size/);
  }
  const pe = bytesFor("windows-arm64"); pe.writeUInt32LE(0xffffffff, 0x3c);
  assert.throws(() => checkNativeBinary(pe, "windows-arm64"), /offset/);
  assert.throws(() => checkNativeBinary(Buffer.alloc(128), "linux-arm64"), /ELF64/);
});

test("actual archive members and actual SPDX bytes bind each final executable", (t) => {
  for (const kind of ["client", "server"]) {
    const f = fixture(t, kind);
    assert.deepEqual(check(f), f.report);
    for (const binary of f.plan.binaries) assert.equal(sha256(readRustBinary(f.archive, binary.member, f.plan.target).bytes),
      f.report.binaries.find((entry) => entry.member === binary.member).binary_sha256);
    assert.throws(() => check(f, "b".repeat(40)), /strictly equal/);
  }
});

test("linked, duplicate, missing and foreign-platform archive members fail", (t) => {
  const duplicate = fixture(t); duplicate.pack(["synveda-worker"]);
  assert.throws(() => readRustBinary(duplicate.archive, "synveda-worker", duplicate.plan.target), /one regular file/);
  const linked = fixture(t); rmSync(join(linked.stage, "synveda-worker")); symlinkSync("synveda", join(linked.stage, "synveda-worker")); linked.pack();
  assert.throws(() => readRustBinary(linked.archive, "synveda-worker", linked.plan.target), /one regular file/);
  const missing = fixture(t); assert.throws(() => readRustBinary(missing.archive, "client/bin/synveda", missing.plan.target));
  assert.throws(() => readRustBinary(missing.archive, "synveda", "linux-arm64"), /ELF64/);
  assert.throws(() => readRustBinary(missing.archive, "../../file", missing.plan.target), /unreviewed/);
});

test("another binary cannot satisfy missing, wrong-version or foreign-hash Cargo content", (t) => {
  for (const damage of [
    (doc) => { doc.packages = []; },
    (doc) => { doc.packages = doc.packages.filter((pkg) => pkg.name !== "cedar-policy"); },
    (doc) => { doc.packages.find((pkg) => pkg.name === "sqlx").versionInfo = "0.0.0"; },
    (doc) => { doc.packages.forEach((pkg) => { pkg.externalRefs = []; }); },
    (doc) => { doc.files[0].checksums[0].checksumValue = "b".repeat(64); },
    (doc) => { doc.files[0].fileName = "synveda-gateway"; },
    (doc) => { doc.files.push(doc.files[0]); },
    (doc) => { doc.creationInfo.creators = ["Tool: syft-v1.51.0"]; },
    (doc) => { doc.creationInfo.creators = "Tool: syft-1.51.0"; },
  ]) {
    const f = fixture(t); damage(f.docs[2]); f.writeDoc(2);
    assert.throws(() => check(f));
  }
});

test("changed archives, reports, scanner pins and sidecars cannot qualify", (t) => {
  for (const damage of [
    (f) => { f.report.source_tree_dirty = true; f.write(); },
    (f) => { f.report.target = "darwin-x86_64"; f.write(); },
    (f) => { f.report.archive_sha256 = "b".repeat(64); f.write(); },
    (f) => { f.report.scanner_download.binary_sha256 = "b".repeat(64); f.write(); },
    (f) => { f.report.binaries.pop(); f.write(); },
    (f) => { f.report.binaries[0].inventory.cargo_identity_count = 999; f.write(); },
    (f) => { writeFileSync(join(f.directory, f.plan.binaries[0].sbom), "{}\n"); },
    (f) => { const path = join(f.directory, f.plan.binaries[0].sbom); rmSync(path); symlinkSync(f.plan.binaries[1].sbom, path); },
    (f) => { writeFileSync(f.archive, "changed archive"); },
  ]) { const f = fixture(t); damage(f); assert.throws(() => check(f)); }
});

test("native scanner download refuses a foreign host and untrusted bytes before tool creation", async (t) => {
  const directory = mkdtempSync(join(tmpdir(), "synveda-syft-download-test-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const foreign = Object.keys(syftPins.targets).find((target) => target !== targetName());
  await assert.rejects(downloadSyft(foreign, join(directory, "foreign")), /native host/);
  const savedFetch = globalThis.fetch;
  t.after(() => { globalThis.fetch = savedFetch; });
  globalThis.fetch = async () => ({ ok: true, body: [Buffer.from("untrusted bytes")] });
  await assert.rejects(downloadSyft(targetName(), join(directory, "small")), /archive size differs/);
  globalThis.fetch = async () => ({ ok: true, body: [Buffer.alloc(syftPins.targets[targetName()].bytes)] });
  await assert.rejects(downloadSyft(targetName(), join(directory, "wrong")), /checksum differs/);
});
