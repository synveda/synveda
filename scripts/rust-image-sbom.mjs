// OPS-12 / ADR-0133: qualify Cargo content in the actual exported OCI attestation.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { lstatSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

export const rustImageRoots = {
  product: ["synveda-cli", "synveda-gateway", "synveda-apalis"],
  browser_acceptance: ["synveda-cli"],
};
export const sbomGenerator = "docker.io/docker/buildkit-syft-scanner:1.12.0@sha256:ae4f3b554449e7e25548e7d8ccc029d17357348e30c6e3df01b92bc93654d6a9";
const scanner = "syft-v1.51.0";
const digestPattern = /^sha256:[0-9a-f]{64}$/;
const spdxPredicate = "https://spdx.dev/Document";
const criticalPackages = ["cedar-policy", "cedar-policy-core", "sqlx", "sqlx-postgres"];
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");

// Cargo writes this closed package table. Refuse ambiguous critical versions
// rather than guessing when the locked build changes.
export function requiredRustPackages(name, version) {
  assert.ok(Object.hasOwn(rustImageRoots, name), "expected a Rust-bearing image");
  const lock = readFileSync(new URL("../Cargo.lock", import.meta.url), "utf8");
  const requirements = Object.fromEntries(rustImageRoots[name].map((root) => [root, version]));
  for (const name of criticalPackages) {
    const versions = lock.split("[[package]]").flatMap((block) => {
      if (!block.includes(`\nname = "${name}"\n`)) return [];
      return [...block.matchAll(/^version = "([^"\n]+)"$/gm)].map((match) => match[1]);
    });
    assert.equal(versions.length, 1, `locked critical package must be unambiguous: ${name}`);
    requirements[name] = versions[0];
  }
  return requirements;
}

export function checkRustStatement(statement, imageManifest, name, version) {
  assert.match(imageManifest, digestPattern);
  assert.ok(["https://in-toto.io/Statement/v0.1", "https://in-toto.io/Statement/v1"].includes(statement?._type), "expected an in-toto statement");
  assert.equal(statement.predicateType, spdxPredicate, "expected an SPDX predicate");
  assert.ok(Array.isArray(statement.subject) && statement.subject.length > 0 && statement.subject.length <= 8, "expected bounded attestation subjects");
  for (const subject of statement.subject)
    assert.equal(subject.digest?.sha256, imageManifest.slice(7), "SBOM subject differs from the native image");
  const spdx = statement.predicate;
  assert.equal(spdx?.spdxVersion, "SPDX-2.3");
  assert.ok(spdx.creationInfo?.creators?.includes(`Tool: ${scanner}`), "SBOM scanner differs from the reviewed pin");
  assert.ok(Array.isArray(spdx.packages) && spdx.packages.length > 0 && spdx.packages.length <= 50_000, "expected a bounded SPDX package inventory");
  const cargo = new Map();
  let cargoCount = 0;
  for (const pkg of spdx.packages) {
    const refs = pkg.externalRefs ?? [];
    assert.ok(Array.isArray(refs) && refs.length <= 32, "expected bounded package references");
    let hasCargo = false;
    for (const ref of refs) {
      if (ref.referenceType !== "purl" || !ref.referenceLocator?.startsWith("pkg:cargo/")) continue;
      const match = /^pkg:cargo\/([a-zA-Z0-9_-]+)@([^?/#]+)$/.exec(ref.referenceLocator);
      assert.ok(match, "invalid Cargo package URL");
      assert.equal(pkg.name, match[1], "Cargo name and package URL disagree");
      // Package URLs percent-encode SemVer build metadata (for example +deprecated).
      assert.equal(pkg.versionInfo, decodeURIComponent(match[2]), "Cargo version and package URL disagree");
      const versions = cargo.get(pkg.name) ?? new Set();
      versions.add(pkg.versionInfo);
      cargo.set(pkg.name, versions);
      hasCargo = true;
    }
    if (hasCargo) cargoCount += 1;
  }
  const required = requiredRustPackages(name, version);
  for (const [pkg, expected] of Object.entries(required))
    assert.deepEqual([...cargo.get(pkg) ?? []], [expected], `missing or wrong-version runtime Cargo package: ${pkg}`);
  return { scanner, spdx_version: spdx.spdxVersion, package_count: spdx.packages.length,
    cargo_package_count: cargoCount, cargo_identity_count: [...cargo.values()].reduce((count, versions) => count + versions.size, 0), required_packages: required };
}

function readArchiveJson(archive, member, maxBytes) {
  // Read fixed data members only, without unpacking or executing archive paths.
  const listing = execFileSync("tar", ["-tvf", archive, member], { encoding: "utf8", timeout: 120_000, maxBuffer: 64 * 1024 }).trim().split("\n");
  assert.ok(listing.length === 1 && listing[0].startsWith("-"), "OCI member must be one regular file");
  const bytes = execFileSync("tar", ["-xOf", archive, member], { timeout: 120_000, maxBuffer: maxBytes });
  assert.ok(bytes.length > 0 && bytes.length <= maxBytes, "OCI member exceeds its bound");
  return { bytes, json: JSON.parse(bytes.toString("utf8")) };
}

function readBlob(archive, descriptor) {
  assert.match(descriptor?.digest ?? "", digestPattern);
  assert.ok(Number.isSafeInteger(descriptor.size) && descriptor.size > 0 && descriptor.size <= 64 * 1024 * 1024, "OCI JSON blob exceeds its bound");
  const result = readArchiveJson(archive, `blobs/sha256/${descriptor.digest.slice(7)}`, descriptor.size);
  assert.equal(result.bytes.length, descriptor.size, "OCI blob size differs from its descriptor");
  assert.equal(`sha256:${hash(result.bytes)}`, descriptor.digest, "OCI blob digest differs from its descriptor");
  return result;
}

export function inspectRustImageSbom(archive, arch, name, version, source) {
  assert.ok(["amd64", "arm64"].includes(arch), "expected a native Linux platform");
  assert.match(source, /^[0-9a-f]{40}$/);
  assert.ok(Object.hasOwn(rustImageRoots, name), "expected a Rust-bearing image");
  const stat = lstatSync(archive);
  assert.ok(stat.isFile() && stat.size > 0 && stat.size <= 20 * 1024 ** 3, "expected a bounded regular OCI archive");
  let index = readArchiveJson(archive, "index.json", 2 * 1024 * 1024).json;
  // Buildx's OCI layout may wrap the platform index in one root descriptor.
  if (index.manifests?.length === 1 && index.manifests[0].mediaType === "application/vnd.oci.image.index.v1+json")
    index = readBlob(archive, index.manifests[0]).json;
  assert.equal(index.schemaVersion, 2);
  assert.ok(Array.isArray(index.manifests) && index.manifests.length <= 8, "expected a bounded native OCI index");
  const native = index.manifests.filter((entry) => entry.platform?.os === "linux");
  assert.equal(native.length, 1, "expected exactly one native image");
  assert.equal(native[0].platform.architecture, arch, "SBOM belongs to another platform");
  const attestations = index.manifests.filter((entry) => entry.annotations?.["vnd.docker.reference.type"] === "attestation-manifest" &&
    entry.annotations?.["vnd.docker.reference.digest"] === native[0].digest && entry.platform?.os === "unknown");
  assert.equal(attestations.length, 1, "missing or ambiguous native attestation descriptor");
  const image = readBlob(archive, native[0]).json;
  const config = readBlob(archive, image.config).json;
  assert.equal(config.os, "linux");
  assert.equal(config.architecture, arch);
  const labels = config.config?.Labels;
  assert.equal(labels?.["org.opencontainers.image.source"], "https://github.com/synveda/synveda");
  assert.equal(labels?.["org.opencontainers.image.revision"], source, "Rust SBOM source binding differs");
  assert.equal(labels?.["org.opencontainers.image.version"], version);
  const manifest = readBlob(archive, attestations[0]).json;
  if (manifest.subject) assert.equal(manifest.subject.digest, native[0].digest, "OCI attestation subject differs from the native image");
  assert.ok(Array.isArray(manifest.layers) && manifest.layers.length <= 8, "expected bounded attestation layers");
  const sboms = manifest.layers.filter((entry) => entry.mediaType === "application/vnd.in-toto+json" &&
    entry.annotations?.["in-toto.io/predicate-type"] === spdxPredicate);
  assert.equal(sboms.length, 1, "missing or ambiguous SPDX layer");
  const statement = readBlob(archive, sboms[0]);
  const content = checkRustStatement(statement.json, native[0].digest, name, version);
  return { schema_version: 1, image_manifest: native[0].digest, attestation_manifest: attestations[0].digest,
    statement_sha256: hash(statement.bytes), ...content };
}

export function checkRustImageReport(report, name, version, imageManifest) {
  assert.equal(report?.schema_version, 1, "missing Rust SBOM evidence");
  assert.match(report.image_manifest, digestPattern);
  assert.match(imageManifest ?? "", digestPattern, "missing native image binding");
  assert.equal(report.image_manifest, imageManifest, "Rust SBOM native image binding differs");
  assert.match(report.attestation_manifest, digestPattern);
  assert.match(report.statement_sha256, /^[0-9a-f]{64}$/);
  assert.equal(report.scanner, scanner);
  assert.equal(report.spdx_version, "SPDX-2.3");
  const required = requiredRustPackages(name, version);
  assert.ok(Number.isSafeInteger(report.package_count) && report.package_count <= 50_000);
  assert.ok(Number.isSafeInteger(report.cargo_package_count) && report.cargo_package_count >= Object.keys(required).length && report.cargo_package_count <= report.package_count);
  assert.ok(Number.isSafeInteger(report.cargo_identity_count) && report.cargo_identity_count >= Object.keys(required).length && report.cargo_identity_count <= report.cargo_package_count);
  assert.deepEqual(report.required_packages, required, "runtime Cargo requirements differ from the source lock");
}

if (process.argv[1] && resolve(process.argv[1]) === import.meta.filename) {
  assert.equal(process.argv.length, 7, "usage: rust-image-sbom.mjs OCI_ARCHIVE amd64|arm64 product|browser_acceptance VERSION SOURCE_SHA");
  console.log(JSON.stringify(inspectRustImageSbom(...process.argv.slice(2)), null, 2));
}
