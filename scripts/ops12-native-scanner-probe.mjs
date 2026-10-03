// OPS-12: temporary native resolver probe; the tool is the fixture, not a product binary.
import assert from "node:assert/strict";
import {execFileSync} from "node:child_process";
import {copyFileSync, mkdirSync, readFileSync} from "node:fs";
import {join, resolve} from "node:path";
import {downloadSyft, syftPins} from "./download-syft.mjs";
import {sha256, targetName} from "./client-artifact.mjs";
import {checkNativeBinary} from "./rust-archive-sbom.mjs";

const target = targetName();
assert.ok(["windows-x86_64", "windows-arm64"].includes(target));
const root = resolve(process.argv[2]);
mkdirSync(root, {mode: 0o700});
const scanner = await downloadSyft(target, join(root, "scanner"));
const fixture = join(root, "fixture");
mkdirSync(fixture, {mode: 0o700});
const binary = join(fixture, "synveda.exe");
copyFileSync(scanner, binary);
const bytes = readFileSync(binary);
const binaryHash = sha256(bytes);
assert.equal(binaryHash, syftPins.targets[target].binary_sha256);
checkNativeBinary(bytes, target);
const env = {PATH: process.env.PATH, SystemRoot: process.env.SystemRoot,
  HOME: root, USERPROFILE: root, XDG_CACHE_HOME: root,
  TMPDIR: root, TMP: root, TEMP: root,
  SYFT_CHECK_FOR_APP_UPDATE: "false", SYFT_CACHE_DIR: root,
  SYFT_FILE_METADATA_SELECTION: "all"};
const output = execFileSync(scanner, ["scan", `file:${binary}`,
  "--override-default-catalogers", "cargo-auditable-binary-cataloger", "--output", "spdx-json"],
  {cwd: fixture, env, timeout: 120_000, maxBuffer: 8 * 1024 * 1024});
const spdx = JSON.parse(output);
const report = {evidence: "native-scanner-tool-fixture-probe", source_sha: process.env.GITHUB_SHA,
  target, scanner_version: syftPins.version, fixture: "official Syft executable copied as synveda.exe",
  binary_sha256: binaryHash, binary_bytes: bytes.length, spdx_sha256: sha256(output),
  files: spdx.files, creators: spdx.creationInfo.creators, package_count: spdx.packages?.length ?? 0};
console.log(JSON.stringify(report, null, 2));
assert.equal(spdx.files.length, 2);
assert.equal(spdx.files[0].fileName, "");
assert.deepEqual(spdx.files[0].fileTypes, ["OTHER"]);
assert.deepEqual(spdx.files[0].checksums, [{algorithm: "SHA1", checksumValue: "0".repeat(40)}]);
assert.deepEqual(spdx.files[1].checksums.filter((sum) => sum.algorithm === "SHA256").map((sum) => sum.checksumValue), [binaryHash]);
