// OPS-12 / ADR-0135: release checks for the exact private Node distribution.
import assert from "node:assert/strict";
import { lstatSync, readFileSync } from "node:fs";
import { checkNativeBinary, readArchiveMember } from "./rust-archive-sbom.mjs";
import { sha256 } from "./client-artifact.mjs";

export const nodePins = JSON.parse(readFileSync(new URL("./node-runtimes.json", import.meta.url)));
export const nodeSharedLibraries = ["ada", "brotli", "cares", "libuv", "nghttp2", "openssl", "simdjson", "simdutf", "sqlite", "uvwasi", "zlib", "zstd"];
export const nodeMetadataArguments = ["--eval", `console.log(JSON.stringify({versions:process.versions,platform:process.platform,arch:process.arch,shared:Object.fromEntries(${JSON.stringify(nodeSharedLibraries)}.map(name=>[name,process.config.variables['node_shared_'+name]]))}))`];

function checkRegularNodeInput(path, limit) {
  const stat = lstatSync(path);
  assert.ok(stat.isFile() && stat.nlink === 1 && stat.size > 0 && stat.size <= limit, "Node input must be a bounded regular unlinked file");
}

export function readNodeFile(path, limit) {
  checkRegularNodeInput(path, limit);
  return readFileSync(path);
}

export function checkNodeFiles(binary, notices, target, lock = nodePins) {
  const pin = lock.targets[target];
  assert.ok(pin, "unreviewed Node target");
  checkNativeBinary(binary, target);
  const binaryHash = sha256(binary);
  assert.equal(binary.length, pin.binary_bytes, "Node executable size differs from upstream pin");
  assert.equal(binaryHash, pin.binary_sha256, "Node executable hash differs from upstream pin");
  const expectedNotices = { LICENSE: { bytes: pin.license_bytes, sha256: pin.license_sha256 }, ...lock.supplementary_notices };
  assert.deepEqual(Object.keys(notices).sort(), Object.keys(expectedNotices).sort(), "Node notice inventory differs");
  const checked = {};
  for (const [name, pin] of Object.entries(expectedNotices)) {
    const bytes = notices[name];
    assert.ok(Buffer.isBuffer(bytes) && bytes.length > 0 && bytes.length <= 1024 * 1024, "Node notice exceeds its bound");
    const noticeHash = sha256(bytes);
    assert.equal(bytes.length, pin.bytes, "Node notice size differs from upstream pin");
    assert.equal(noticeHash, pin.sha256, "Node notice hash differs from upstream pin");
    checked[name] = { bytes: bytes.length, sha256: noticeHash };
  }
  return { binary_bytes: binary.length, binary_sha256: binaryHash, notices: checked };
}

export function checkNodeMetadata(observed, target, lock = nodePins) {
  const pin = lock.targets[target];
  assert.ok(pin, "unreviewed Node target");
  assert.equal(observed?.platform, pin.platform, "Node metadata platform differs");
  assert.equal(observed.arch, pin.arch, "Node metadata architecture differs");
  assert.equal(observed.versions?.node, lock.version, "Node metadata version differs");
  assert.ok(Object.keys(observed.versions).length <= 40, "Node metadata version count exceeds its bound");
  // ABI/data identifiers are not libraries. Optional QUIC versions are empty in
  // this pinned build; an enabled or newly reported dependency needs review.
  const excluded = new Set(["node", "modules", "napi", "cldr", "tz", "unicode"]);
  assert.ok(Object.keys(observed.versions).every((name) => excluded.has(name) || Object.hasOwn(lock.dependencies, name) || ["nghttp3", "ngtcp2"].includes(name)), "unreviewed Node version field");
  const dependencies = Object.fromEntries(Object.entries(observed.versions).filter(([name, version]) => !excluded.has(name) && version !== ""));
  assert.deepEqual(dependencies, lock.dependencies, "Node reported dependency versions differ");
  for (const name of ["nghttp3", "ngtcp2"])
    assert.ok(observed.versions[name] === undefined || observed.versions[name] === "", "unreviewed Node QUIC dependency");
  const shared = Object.fromEntries(nodeSharedLibraries.map((name) => [name, false]));
  assert.deepEqual(observed.shared, shared, "Node shared-library contract differs");
  return { dependencies, shared_libraries: shared };
}

export function nodeInventory(files, metadata, target, lock = nodePins) {
  return { schema_version: 1, target, upstream_source_sha: lock.source_sha, ...files, ...checkNodeMetadata(metadata, target, lock) };
}

function checkNodeInventory(report, files, target, lock = nodePins) {
  const expected = { schema_version: 1, target, upstream_source_sha: lock.source_sha, ...files,
    dependencies: lock.dependencies, shared_libraries: Object.fromEntries(nodeSharedLibraries.map((name) => [name, false])) };
  assert.deepEqual(report, expected, "Node inventory differs from actual bytes or reviewed dependencies");
}

export function checkClientNodeArchive(archive, report, target, version, source, lock = nodePins) {
  const pin = lock.targets[target];
  assert.ok(pin, "unreviewed Node target");
  assert.equal(report?.version, lock.version, "archived Node runtime version differs");
  assert.equal(report.archive, pin.archive, "archived Node distribution differs");
  assert.equal(report.archive_sha256, pin.sha256, "archived Node distribution pin differs");
  checkRegularNodeInput(archive, 512 * 1024 * 1024);
  const manifest = JSON.parse(readArchiveMember(archive, "client/client.json", 256 * 1024));
  assert.equal(manifest.target, target, "archived Node target differs");
  assert.equal(manifest.version, version, "archived Node release differs");
  assert.equal(manifest.source_sha, source, "archived Node source differs");
  assert.deepEqual(manifest.node, report, "archived Node metadata differs from native report");
  const base = "client/plugin/synveda/runtime/";
  const binary = readArchiveMember(archive, `${base}node${target.startsWith("windows-") ? ".exe" : ""}`, 256 * 1024 * 1024);
  const notices = Object.fromEntries(["LICENSE", ...Object.keys(lock.supplementary_notices)].map((name) =>
    [name, readArchiveMember(archive, `${base}${name}`, 1024 * 1024)]));
  checkNodeInventory(report.inventory, checkNodeFiles(binary, notices, target, lock), target, lock);
}
