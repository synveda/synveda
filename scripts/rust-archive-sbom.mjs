// OPS-12 / ADR-0133: scan each final archived Rust binary and bind its SPDX bytes.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { lstatSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { sha256, targetName } from "./client-artifact.mjs";
import { syftPins } from "./download-syft.mjs";
import { checkRustSpdx, nativeRustScanner } from "./rust-sbom.mjs";

const maxBinary = 256 * 1024 * 1024;
const maxSpdx = 64 * 1024 * 1024;
const servers = ["darwin-arm64", "linux-x86_64"];
const hashPattern = /^[0-9a-f]{64}$/;

export function rustArchivePlan(kind, target, version) {
  assert.ok(Object.hasOwn(syftPins.targets, target), "unreviewed native archive target");
  assert.match(version, /^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$/);
  assert.ok(kind === "client" || (kind === "server" && servers.includes(target)), "unsupported Rust archive kind or server target");
  const windows = target.startsWith("windows-");
  const base = `synveda${kind === "client" ? "-client" : ""}-${version}-${target}`;
  const binaries = kind === "client" ? [{ member: `client/bin/synveda${windows ? ".exe" : ""}`, kind: "cli", sbom: `${base}.spdx.json` }]
    : ["synveda", "synveda-gateway", "synveda-worker"].map((member, i) => ({ member, kind: ["cli", "gateway", "worker"][i], sbom: `${base}-${member}.spdx.json` }));
  return { kind, target, archive: `${base}.${windows ? "zip" : "tar.gz"}`, report: `${base}.rust-sbom.json`, binaries };
}

export function rustArchivePlans(version) {
  return [...Object.keys(syftPins.targets).sort().map((target) => rustArchivePlan("client", target, version)),
    ...servers.map((target) => rustArchivePlan("server", target, version))];
}

export function rustArchiveAssets(version) {
  return rustArchivePlans(version).flatMap((plan) => [plan.report, ...plan.binaries.map((binary) => binary.sbom)]);
}

function regularBytes(path, limit) {
  const stat = lstatSync(path);
  assert.ok(stat.isFile() && stat.nlink === 1 && stat.size > 0 && stat.size <= limit, "expected a bounded regular unlinked SBOM input");
  return readFileSync(path);
}

function archiveMember(archive, member, limit) {
  const options = { timeout: 120_000, stdio: ["ignore", "pipe", "pipe"] };
  const listing = execFileSync("tar", ["-tvf", archive, member], { ...options, encoding: "utf8", maxBuffer: 32 * 1024 }).trim().split("\n");
  assert.ok(listing.length === 1 && listing[0].startsWith("-"), "Rust archive member must be one regular file");
  const bytes = execFileSync("tar", ["-xOf", archive, member], { ...options, maxBuffer: limit });
  assert.ok(bytes.length > 0 && bytes.length <= limit, "Rust archive member exceeds its bound");
  return bytes;
}

export function checkNativeBinary(bytes, target) {
  assert.ok(Object.hasOwn(syftPins.targets, target), "unreviewed binary target");
  assert.ok(bytes.length >= 64 && bytes.length <= maxBinary, "invalid native executable size");
  const [os, arch] = target.split("-");
  if (os === "linux") {
    assert.equal(bytes.subarray(0, 6).toString("hex"), "7f454c460201", "expected a little-endian ELF64 binary");
    assert.equal(bytes.readUInt16LE(18), arch === "arm64" ? 183 : 62, "ELF architecture differs");
  } else if (os === "darwin") {
    assert.equal(bytes.readUInt32LE(0), 0xfeedfacf, "expected a Mach-O 64 binary");
    assert.equal(bytes.readUInt32LE(4), arch === "arm64" ? 0x100000c : 0x1000007, "Mach-O architecture differs");
  } else {
    assert.equal(bytes.subarray(0, 2).toString("ascii"), "MZ", "expected a PE binary");
    const offset = bytes.readUInt32LE(0x3c);
    assert.ok(offset >= 64 && offset <= 4 * 1024 * 1024 && offset + 26 <= bytes.length, "invalid PE header offset");
    assert.equal(bytes.subarray(offset, offset + 4).toString("hex"), "50450000", "expected a PE header");
    assert.equal(bytes.readUInt16LE(offset + 4), arch === "arm64" ? 0xaa64 : 0x8664, "PE architecture differs");
    assert.equal(bytes.readUInt16LE(offset + 24), 0x20b, "expected a PE32+ binary");
  }
  return { os, arch };
}

export function readRustBinary(archive, member, target) {
  assert.ok(["client/bin/synveda", "client/bin/synveda.exe", "synveda", "synveda-gateway", "synveda-worker"].includes(member), "unreviewed Rust archive member");
  const bytes = archiveMember(archive, member, maxBinary);
  return { bytes, platform: checkNativeBinary(bytes, target) };
}

export function checkNativeRustSpdx(spdx, binaryHash, binary, version) {
  assert.match(binaryHash, hashPattern);
  const content = checkRustSpdx(spdx, binary.kind, version, nativeRustScanner);
  assert.ok(Array.isArray(spdx.files) && spdx.files.length === 1, "native Rust SBOM must describe exactly one binary");
  // Pinned Syft names a Windows file relative to its synthetic root. Exact
  // spelling preserves member identity without normalizing arbitrary paths.
  const fileName = binary.member === "client/bin/synveda.exe" ? "\\synveda.exe" : binary.member.split("/").at(-1);
  assert.equal(spdx.files[0].fileName, fileName, "native SBOM file differs from the archived binary");
  const sums = spdx.files[0].checksums;
  assert.ok(Array.isArray(sums) && sums.length <= 8, "expected bounded binary checksums");
  assert.deepEqual(sums.filter((sum) => sum.algorithm === "SHA256").map((sum) => sum.checksumValue), [binaryHash], "native SBOM binary hash differs");
  return content;
}

export function checkRustArchiveReport(directory, plan, version, source) {
  assert.match(source, /^[0-9a-f]{40}$/);
  const report = JSON.parse(regularBytes(join(directory, plan.report), 2 * 1024 * 1024));
  assert.equal(report.schema_version, 1);
  assert.equal(report.evidence, "native-rust-archive-sbom");
  assert.equal(report.kind, plan.kind);
  assert.equal(report.target, plan.target);
  assert.equal(report.version, version);
  assert.equal(report.source_sha, source);
  assert.equal(report.source_tree_dirty, false, "native Rust archive SBOM source must be clean");
  const archive = regularBytes(join(directory, plan.archive), 512 * 1024 * 1024);
  assert.equal(report.archive_sha256, sha256(archive), "native Rust archive hash differs");
  assert.equal(report.archive_bytes, archive.length);
  assert.deepEqual(report.scanner_download, scannerDownload(plan.target), "native scanner download pin differs");
  assert.ok(Array.isArray(report.binaries));
  assert.deepEqual(report.binaries.map((entry) => entry.member), plan.binaries.map((entry) => entry.member), "native Rust binary inventory differs");
  for (const [i, binary] of plan.binaries.entries()) {
    const checked = report.binaries[i];
    assert.match(checked.binary_sha256, hashPattern);
    assert.ok(Number.isSafeInteger(checked.binary_bytes) && checked.binary_bytes >= 64 && checked.binary_bytes <= maxBinary);
    assert.deepEqual(checked.platform, { os: plan.target.split("-")[0], arch: plan.target.split("-")[1] });
    assert.equal(checked.sbom, binary.sbom);
    const spdx = regularBytes(join(directory, binary.sbom), maxSpdx);
    assert.equal(checked.sbom_sha256, sha256(spdx), "native Rust SPDX hash differs");
    assert.equal(checked.sbom_bytes, spdx.length);
    assert.deepEqual(checked.inventory, checkNativeRustSpdx(JSON.parse(spdx), checked.binary_sha256, binary, version), "native Rust SPDX content differs from the producer report");
  }
  return report;
}

function scannerDownload(target) {
  const pin = syftPins.targets[target];
  return { version: syftPins.version, archive: pin.archive, archive_sha256: pin.sha256, binary_sha256: pin.binary_sha256 };
}

function scanBinary(scanner, archive, binary, target, version, directory, scratch) {
  const { bytes, platform } = readRustBinary(archive, binary.member, target);
  const path = join(scratch, binary.member.split("/").at(-1));
  writeFileSync(path, bytes, { flag: "wx", mode: 0o600 });
  // Select only the embedded Rust cataloger. No enrichment or registry source
  // participates, and ambient scanner configuration/credentials are excluded.
  const env = { PATH: process.env.PATH, HOME: scratch, USERPROFILE: scratch, XDG_CACHE_HOME: scratch,
    TMPDIR: scratch, TMP: scratch, TEMP: scratch, SYFT_CHECK_FOR_APP_UPDATE: "false", SYFT_CACHE_DIR: scratch };
  if (process.platform === "win32") env.SystemRoot = process.env.SystemRoot;
  const spdx = execFileSync(scanner, ["scan", `file:${path}`, "--override-default-catalogers", "cargo-auditable-binary-cataloger", "--output", "spdx-json"],
    { cwd: scratch, env, timeout: 120_000, maxBuffer: maxSpdx, stdio: ["ignore", "pipe", "pipe"] });
  const binaryHash = sha256(bytes);
  const inventory = checkNativeRustSpdx(JSON.parse(spdx), binaryHash, binary, version);
  writeFileSync(join(directory, binary.sbom), spdx, { flag: "wx", mode: 0o600 });
  return { member: binary.member, platform, binary_sha256: binaryHash, binary_bytes: bytes.length,
    sbom: binary.sbom, sbom_sha256: sha256(spdx), sbom_bytes: spdx.length, inventory };
}

export function produceRustArchiveSbom(kind, target, version, source, scanner, directory) {
  assert.equal(target, targetName(), "Rust archive scanning requires its native producer");
  assert.match(source, /^[0-9a-f]{40}$/);
  scanner = resolve(scanner);
  directory = resolve(directory);
  const root = resolve(import.meta.dirname, "..");
  assert.equal(execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(), source, "native SBOM source differs from the producer checkout");
  const dirty = execFileSync("git", ["status", "--porcelain", "--untracked-files=normal", "--", "scripts", "crates", "adapters", "docs", ".github", "Cargo.toml", "Cargo.lock"], { cwd: root, encoding: "utf8" }).trim().length > 0;
  const plan = rustArchivePlan(kind, target, version);
  const scannerBytes = regularBytes(scanner, maxBinary);
  assert.equal(scannerBytes.length, syftPins.targets[target].binary_bytes);
  assert.equal(sha256(scannerBytes), syftPins.targets[target].binary_sha256, "native Syft executable differs from the reviewed pin");
  checkNativeBinary(scannerBytes, target);
  const archive = join(directory, plan.archive);
  const archiveBytes = regularBytes(archive, 512 * 1024 * 1024);
  if (kind === "client") {
    const manifest = JSON.parse(archiveMember(archive, "client/client.json", 256 * 1024));
    assert.equal(manifest.version, version);
    assert.equal(manifest.cli_version, `synveda ${version}`);
    assert.equal(manifest.source_sha, source);
    assert.equal(manifest.target, target);
    assert.equal(manifest.source_tree_dirty, dirty);
    assert.equal(manifest.files?.[plan.binaries[0].member.slice(7)]?.sha256, sha256(readRustBinary(archive, plan.binaries[0].member, target).bytes), "archived CLI differs from the client manifest");
  }
  const scratch = mkdtempSync(join(tmpdir(), "synveda-native-sbom-"));
  try {
    const binaries = plan.binaries.map((binary) => scanBinary(scanner, archive, binary, target, version, directory, scratch));
    const report = { schema_version: 1, evidence: "native-rust-archive-sbom", kind, target, version, source_sha: source, source_tree_dirty: dirty,
      archive_sha256: sha256(archiveBytes), archive_bytes: archiveBytes.length, scanner_download: scannerDownload(target), binaries };
    writeFileSync(join(directory, plan.report), `${JSON.stringify(report, null, 2)}\n`, { flag: "wx", mode: 0o600 });
    return report;
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

if (process.argv[1] && resolve(process.argv[1]) === import.meta.filename) {
  assert.equal(process.argv.length, 8, "usage: rust-archive-sbom.mjs client|server NATIVE_TARGET VERSION SOURCE_SHA SYFT_PATH DIRECTORY");
  console.log(JSON.stringify(produceRustArchiveSbom(...process.argv.slice(2)), null, 2));
}
