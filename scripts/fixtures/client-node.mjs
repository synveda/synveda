import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { nodePins, nodeSharedLibraries } from "../node-runtime-inventory.mjs";
import { sha256 } from "../client-artifact.mjs";

// Deliberate non-executable headers exercise archive/platform refusals without
// downloading or running foreign code. Fixture pins never enter production.
export function clientNodeFixture(directory, target, version, source) {
  const lock = structuredClone(nodePins);
  const pin = lock.targets[target];
  lock.targets = { [target]: pin };
  const binary = Buffer.alloc(128);
  const arm = target.endsWith("arm64");
  if (target.startsWith("darwin-")) {
    binary.writeUInt32LE(0xfeedfacf, 0); binary.writeUInt32LE(arm ? 0x100000c : 0x1000007, 4);
  } else if (target.startsWith("linux-")) {
    Buffer.from("7f454c460201", "hex").copy(binary); binary.writeUInt16LE(arm ? 183 : 62, 18);
  } else {
    binary.write("MZ"); binary.writeUInt32LE(64, 0x3c); binary.write("PE\0\0", 64);
    binary.writeUInt16LE(arm ? 0xaa64 : 0x8664, 68); binary.writeUInt16LE(0x20b, 88);
  }
  pin.binary_bytes = binary.length; pin.binary_sha256 = sha256(binary);
  const license = Buffer.from(target.startsWith("windows-") ? "fixture licence\r\n" : "fixture licence\n");
  pin.license_bytes = license.length; pin.license_sha256 = sha256(license);
  const notices = { LICENSE: license, ...Object.fromEntries(Object.keys(lock.supplementary_notices).map((name) =>
    [name, readFileSync(new URL(`../../assets/licenses/node/${name}`, import.meta.url))])) };
  const stage = join(directory, `stage-${target}`);
  const runtime = join(stage, "client/plugin/synveda/runtime"); mkdirSync(runtime, { recursive: true });
  const binaryName = target.startsWith("windows-") ? "node.exe" : "node";
  writeFileSync(join(runtime, binaryName), binary);
  for (const [name, bytes] of Object.entries(notices)) writeFileSync(join(runtime, name), bytes);
  const node = { version: lock.version, archive: pin.archive, archive_sha256: pin.sha256, inventory: {
    schema_version: 1, target, upstream_source_sha: lock.source_sha, binary_bytes: binary.length, binary_sha256: sha256(binary),
    notices: Object.fromEntries(Object.entries(notices).map(([name, bytes]) => [name, { bytes: bytes.length, sha256: sha256(bytes) }])),
    dependencies: structuredClone(lock.dependencies), shared_libraries: Object.fromEntries(nodeSharedLibraries.map((name) => [name, false])),
  } };
  const archive = join(directory, `synveda-client-${version}-${target}.${target.startsWith("windows-") ? "zip" : "tar.gz"}`);
  const pack = (extra = []) => {
    writeFileSync(join(stage, "client/client.json"), JSON.stringify({ version, target, source_sha: source, node }));
    const zip = target.startsWith("windows-");
    execFileSync(zip && process.platform === "linux" ? "bsdtar" : "tar",
      [...(zip ? ["--format=zip", "-cf"] : ["-czf"]), archive, "-C", stage, "client", ...extra]);
  };
  pack();
  return { lock, node, archive, runtime, binaryName, pack };
}
