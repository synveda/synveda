// OPS-12 / ADR-0133: one locked Cargo-content floor for images and final binaries.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

export const rustRoots = {
  product: ["synveda-cli", "synveda-gateway", "synveda-apalis"],
  browser_acceptance: ["synveda-cli"],
  cli: ["synveda-cli"],
  gateway: ["synveda-gateway"],
  worker: ["synveda-gateway"],
};
export const rustScanner = "syft-v1.51.0";
export const nativeRustScanner = "syft-1.54.0";
const criticalPackages = ["cedar-policy", "cedar-policy-core", "sqlx", "sqlx-postgres"];

// Cargo writes this closed package table. Refuse ambiguous critical versions
// rather than guessing when the locked build changes.
export function requiredRustPackages(name, version) {
  assert.ok(Object.hasOwn(rustRoots, name), "expected a Rust-bearing artifact");
  const lock = readFileSync(new URL("../Cargo.lock", import.meta.url), "utf8").replace(/\r\n/g, "\n");
  const requirements = Object.fromEntries(rustRoots[name].map((root) => [root, version]));
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

export function checkRustSpdx(spdx, name, version, scanner = rustScanner) {
  assert.ok([rustScanner, nativeRustScanner].includes(scanner), "expected a reviewed scanner identity");
  assert.equal(spdx?.spdxVersion, "SPDX-2.3");
  const creators = spdx.creationInfo?.creators;
  assert.ok(Array.isArray(creators) && creators.length > 0 && creators.length <= 16, "expected bounded SPDX creators");
  assert.ok(creators.includes(`Tool: ${scanner}`), "SBOM scanner differs from the reviewed pin");
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

export function checkRustContentReport(report, name, version, scanner = rustScanner) {
  assert.equal(report.scanner, scanner);
  assert.equal(report.spdx_version, "SPDX-2.3");
  const required = requiredRustPackages(name, version);
  assert.ok(Number.isSafeInteger(report.package_count) && report.package_count > 0 && report.package_count <= 50_000);
  assert.ok(Number.isSafeInteger(report.cargo_package_count) && report.cargo_package_count >= Object.keys(required).length && report.cargo_package_count <= report.package_count);
  assert.ok(Number.isSafeInteger(report.cargo_identity_count) && report.cargo_identity_count >= Object.keys(required).length && report.cargo_identity_count <= report.cargo_package_count);
  assert.deepEqual(report.required_packages, required, "runtime Cargo requirements differ from the source lock");
}
