import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { nodePins, nodeSharedLibraries } from "../node-runtime-inventory.mjs";
import { sha256 } from "../client-artifact.mjs";

export function productNodeReport(platform, lock = nodePins.product) {
  const target = platform === "linux/arm64" ? "linux-arm64" : "linux-x86_64";
  const pin = lock.targets[target];
  return { schema_version: 1, target, upstream_source_sha: lock.source_sha,
    binary_bytes: pin.binary_bytes, binary_sha256: pin.binary_sha256,
    notices: { LICENSE: { bytes: pin.license_bytes, sha256: pin.license_sha256 },
      ...Object.fromEntries(Object.entries(lock.supplementary_notices).map(([name, pin]) => [name, { bytes: pin.bytes, sha256: pin.sha256 }])) },
    dependencies: structuredClone(lock.dependencies), shared_libraries: Object.fromEntries(nodeSharedLibraries.map((name) => [name, false])), runtime_version: lock.version };
}

export function nodeImageFixture(root, platform = "linux/arm64") {
  const lock = structuredClone(nodePins.product);
  const target = platform === "linux/arm64" ? "linux-arm64" : "linux-x86_64";
  const pin = lock.targets[target];
  const binary = Buffer.alloc(128);
  Buffer.from("7f454c460201", "hex").copy(binary); binary.writeUInt16LE(target.endsWith("arm64") ? 183 : 62, 18);
  const license = Buffer.from("fixture complete Node licence\n");
  pin.binary_bytes = binary.length; pin.binary_sha256 = sha256(binary);
  pin.license_bytes = license.length; pin.license_sha256 = sha256(license);
  mkdirSync(join(root, "scripts"), { recursive: true });
  writeFileSync(join(root, "scripts/node-runtimes.json"), JSON.stringify({ product: lock }));
  const stage = join(root, "node-fixture"); mkdirSync(stage);
  const files = new Map([["node", binary], ["LICENSE", license],
    ...Object.keys(lock.supplementary_notices).map((name) => [name, readFileSync(new URL(`../../assets/licenses/node/${name}`, import.meta.url))])]);
  const metadata = { versions: { node: lock.version, ...lock.dependencies }, platform: pin.platform, arch: pin.arch,
    shared: Object.fromEntries(nodeSharedLibraries.map((name) => [name, false])) };
  const tar = (name, extra = []) => {
    writeFileSync(join(stage, name), files.get(name));
    const path = join(stage, "stream.tar");
    execFileSync("tar", ["-cf", path, "-C", stage, name, ...extra]);
    return readFileSync(path);
  };
  return { files, metadata, lock, tar, report: productNodeReport(platform, lock) };
}
