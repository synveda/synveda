#!/usr/bin/env node
// OPS-8: one closed inventory for assembly, checksums and stable publication.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { lstatSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { checkClientRelease } from "./check-client-release.mjs";
import { candidateIdentity, fileHash } from "./docker-candidate.mjs";
import { releaseImages, validateRegistryManifest } from "./release-registries.mjs";
import { noticeHashes } from "./verify-release-images.mjs";
import { checkRustImageReport, rustImageRoots } from "./rust-image-sbom.mjs";
import { checkRustArchiveReport, rustArchiveAssets, rustArchivePlans } from "./rust-archive-sbom.mjs";
import { checkConsoleImageReport, checkConsolePackage } from "./check-console-package.mjs";
import { checkProductNodeReport } from "./node-runtime-inventory.mjs";
import { checkProductPackageImageReport } from "./product-package-inventory.mjs";

export const clientTargets = [
  "darwin-arm64",
  "darwin-x86_64",
  "linux-arm64",
  "linux-x86_64",
  "windows-arm64",
  "windows-x86_64",
];
export function releaseAssets(version, qualified = false) {
  const assets = [
    `synveda-${version}-darwin-arm64.tar.gz`,
    `synveda-${version}-linux-x86_64.tar.gz`,
    `synveda-console-${version}.tar.gz`,
    `synveda-reference-${version}.tar.gz`,
    `synveda-plugin-${version}.tar.gz`,
    `synveda-${version}.tgz`,
    `synveda-images-${version}.yaml`,
    `synveda-cnpg-image-${version}.yaml`,
    `synveda-registry-images-${version}.json`,
    ...clientTargets.flatMap((target) => [
      `synveda-client-${version}-${target}.${target.startsWith("windows-") ? "zip" : "tar.gz"}`,
      `synveda-client-report-${target}.json`,
    ]),
  ];
  for (const arch of ["amd64", "arm64"]) {
    for (const kind of ["candidate", "local-images", "docker", "consumer", "kubernetes"])
      assets.push(`release-${kind}-${arch}.json`);
    if (qualified) assets.push(`release-images-${arch}.json`);
  }
  return [...assets, ...rustArchiveAssets(version)];
}
export function regularAssets(directory, names) {
  return names.map((name) => {
    const stat = lstatSync(join(directory, name));
    assert.ok(
      stat.isFile() && !stat.isSymbolicLink() && stat.size > 0,
      `missing or non-regular release asset: ${name}`,
    );
    return { name, size: stat.size };
  });
}
export function checkArchiveNotices(directory, version, run = execFileSync) {
  const archives = new Map([
    [`synveda-${version}-darwin-arm64.tar.gz`, ""],
    [`synveda-${version}-linux-x86_64.tar.gz`, ""],
    [`synveda-console-${version}.tar.gz`, "console"],
    [`synveda-reference-${version}.tar.gz`, `synveda-reference-${version}`],
    [`synveda-plugin-${version}.tar.gz`, "plugin"],
    [`synveda-${version}.tgz`, "synveda"],
  ]);
  const required = releaseAssets(version).filter((name) =>
    /\.(tar\.gz|tgz)$/.test(name) && !name.startsWith("synveda-client-"));
  assert.deepEqual([...archives.keys()].sort(), required.sort(), "archive notice policy must cover the release inventory");
  for (const [archive, directoryPrefix] of archives) {
    for (const name of ["LICENSE", "NOTICE"]) {
      const expected = readFileSync(new URL(`../${name}`, import.meta.url));
      const member = directoryPrefix ? `${directoryPrefix}/${name}` : name;
      const path = join(directory, archive);
      const listing = run("tar", ["-tvzf", path, member], {
        encoding: "utf8", timeout: 60_000, maxBuffer: 16 * 1024, stdio: ["ignore", "pipe", "pipe"],
      }).trim().split("\n");
      assert.ok(listing.length === 1 && listing[0].startsWith("-"), `${archive}: ${member} must be one regular file`);
      const actual = run("tar", ["-xOzf", path, member], { timeout: 60_000, maxBuffer: expected.length + 1, stdio: ["ignore", "pipe", "pipe"] });
      assert.deepEqual(actual, expected, `${archive}: ${member} differs from the source notice`);
    }
  }
}
export function checkCandidateImageReports(directory, version, source, consoleReport) {
  const read = (kind, arch) => JSON.parse(readFileSync(join(directory, `release-${kind}-${arch}.json`)));
  for (const arch of ["amd64", "arm64"]) {
    const platform = `linux/${arch}`;
    const candidate = read("candidate", arch);
    candidateIdentity(candidate, version, source, arch);
    const local = read("local-images", arch);
    assert.equal(local.source_sha, source);
    assert.equal(local.release_version, version);
    assert.equal(local.platform, platform);
    assert.equal(local.local_candidate, true);
    assert.equal(local.anonymous_pull, false);
    for (const name of Object.keys(rustImageRoots)) {
      const entries = local.images.filter((entry) => entry.name === name);
      assert.equal(entries.length, 1, "expected one Rust image report");
      const checked = entries[0];
      assert.equal(checked.image, `localhost:5000/synveda/${releaseImages[name]}@${candidate.images[name].digest}`);
      checkRustImageReport(candidate.images[name].rust_sbom, name, version, checked.platforms?.[platform], platform);
      assert.deepEqual(checked.rust_sbom, candidate.images[name].rust_sbom, `${name}: candidate Rust SBOM evidence differs`);
      if (name === "product") {
        checkConsoleImageReport(checked.console_inventory, consoleReport, version, source);
        checkProductNodeReport(checked.node_inventory, platform);
        checkProductPackageImageReport(checked.package_inventory, candidate.images.product.rust_sbom.debian_inventory, platform, checked.platforms?.[platform]);
      }
    }
  }
}
export function checkQualification(directory, version, source, inventory, consoleReport) {
  checkCandidateImageReports(directory, version, source, consoleReport);
  const read = (kind, arch) =>
    JSON.parse(readFileSync(join(directory, `release-${kind}-${arch}.json`)));
  for (const arch of ["amd64", "arm64"]) {
    const platform = `linux/${arch}`;
    const expectedPackages = read("candidate", arch).images.product.rust_sbom.debian_inventory;
    const image = read("images", arch);
    assert.equal(image.release_version, version);
    assert.equal(image.source_sha, source);
    assert.equal(image.platform, platform);
    assert.equal(image.anonymous_pull, true);
    for (const registry of ["dockerhub", "ghcr"]) {
      const result = image.registries?.[registry];
      assert.equal(result?.anonymous_pull, true);
      assert.equal(result?.platform, platform);
      for (const [name, entry] of Object.entries(
        inventory.registries[registry].images,
      )) {
        const checkedImage = result.images.find((checked) =>
          checked.name === name && checked.image === entry.reference &&
          checked.platforms?.[platform] === entry.platforms[platform] &&
          checked.executable_checks > 0,
        );
        assert.ok(checkedImage);
        assert.deepEqual(checkedImage.notice_sha256, noticeHashes, `${registry}/${name}: notice evidence missing or changed`);
        if (name === "product") {
          checkConsoleImageReport(checkedImage.console_inventory, consoleReport, version, source);
          checkProductNodeReport(checkedImage.node_inventory, platform);
          checkProductPackageImageReport(checkedImage.package_inventory, expectedPackages, platform, checkedImage.platforms[platform]);
        }
      }
    }
    const candidate = read("candidate", arch);
    candidateIdentity(candidate, version, source, arch);
    const images = Object.fromEntries(Object.entries(releaseImages).map(
      ([name, repo]) => [name,
        `localhost:5000/synveda/${repo}@${candidate.images[name].digest}`],
    ));
    const local = read("local-images", arch);
    assert.equal(local.release_version, version);
    assert.equal(local.source_sha, source);
    assert.equal(local.platform, platform);
    assert.equal(local.local_candidate, true);
    assert.equal(local.anonymous_pull, false);
    for (const [name, reference] of Object.entries(images)) {
      const checked = local.images.find((entry) => entry.name === name);
      assert.equal(checked?.image, reference);
      assert.equal(checked?.platforms?.[platform],
        inventory.registries.dockerhub.images[name].platforms[platform]);
      assert.ok(checked?.executable_checks > 0);
      assert.deepEqual(checked.notice_sha256, noticeHashes, `${name}: candidate notice evidence missing or changed`);
    }
    for (const kind of ["docker", "consumer", "kubernetes"]) {
      const report = read(kind, arch);
      assert.equal(report.version, version);
      assert.equal(report.source, source);
      assert.equal(report.sourceDirty, false);
      assert.deepEqual(report.images, images);
      if (kind !== "kubernetes") {
        assert.ok(
          Object.keys(report.checks).length >= (kind === "consumer" ? 20 : 7),
        );
        assert.ok(
          Object.values(report.checks).every((passed) => passed === true),
          `${kind} qualification failed`,
        );
      } else {
        assert.deepEqual(
          report.evidence.cases.map((item) => item.selection).sort(),
          [
            "bundled-external",
            "bundled-packaged",
            "external-external",
            "external-packaged",
          ],
        );
        assert.ok(
          report.evidence.cases.every(
            (item) =>
              item.persistentCredentials === true &&
              item.persistentContent === true &&
              item.uninstallReinstall === true,
          ),
        );
        for (const item of report.evidence.cases) {
          const transport = item.reinstallTransport;
          assert.ok(transport, `${item.selection}: missing reinstall transport evidence`);
          for (const name of ["directGateway", "applicationEdge", "issuerEdge"])
            assert.equal(transport[name], 200, `${item.selection}/${name}: reinstall transport failed`);
          assert.ok(Number.isInteger(transport.recoveredMs) && transport.recoveredMs >= 0 && transport.recoveredMs <= 60_000,
            `${item.selection}: reinstall transport exceeded its bound`);
          assert.ok(Number.isInteger(transport.failedProbes) && transport.failedProbes >= 0);
        }
        // The existing operations drill covers the two end-to-end ownership
        // modes; the mixed modes independently exercise install and upgrade.
        for (const selection of ["bundled-packaged", "external-external"]) {
          const operations = report.evidence.cases.find(
            (item) => item.selection === selection,
          ).dayTwo;
          for (const check of [
            "interruptedJob",
            "gatewayShutdown",
            "repeatedWorkload",
            "migration",
            "recoveryPoint",
            "backup",
            "restore",
          ])
            assert.ok(
              operations?.[check],
              `${selection}: missing ${check} evidence`,
            );
        }
        assert.equal(report.localPortForward.browserLoginLogout, true);
      }
    }
  }
}
if (process.argv[1] && resolve(process.argv[1]) === import.meta.filename) {
  const [directory, version, source, publish, phase] = process.argv.slice(2);
  assert.ok(
    ["true", "false"].includes(publish) &&
      ["assembled", "qualified"].includes(phase),
  );
  execFileSync("sh", ["scripts/release-version.sh", version]);
  const qualified = phase === "qualified";
  const names = releaseAssets(version, qualified);
  regularAssets(directory, names);
  // Client TAR/ZIP notices are verified by each native candidate before its
  // required, archive-hash-bound report is assembled here.
  checkArchiveNotices(directory, version);
  const consoleReport = checkConsolePackage(join(directory, `synveda-console-${version}.tar.gz`), version, source);
  if (!qualified) checkCandidateImageReports(directory, version, source, consoleReport);
  checkClientRelease(
    directory,
    version,
    source,
    publish === "true",
    JSON.parse(readFileSync(new URL("./node-runtimes.json", import.meta.url))),
  );
  for (const plan of rustArchivePlans(version)) checkRustArchiveReport(directory, plan, version, source);
  const inventory = JSON.parse(
    readFileSync(join(directory, `synveda-registry-images-${version}.json`)),
  );
  validateRegistryManifest(inventory, version, source);
  assert.equal(inventory.published, publish === "true");
  if (qualified) {
    assert.equal(publish, "true");
    checkQualification(directory, version, source, inventory, consoleReport);
  }
  const sums = [];
  for (const name of names)
    sums.push(`${await fileHash(join(directory, name))}  ${name}`);
  writeFileSync(join(directory, "SHA256SUMS"), `${sums.join("\n")}\n`);
  console.log(
    `Verified and checksummed ${names.length} ${phase} release assets.`,
  );
}
