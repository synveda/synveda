// OPS-12 / ADR-0136: corroborate maintained SPDX with installed database/notice bytes.
import assert from "node:assert/strict";

const hashPattern = /^[0-9a-f]{64}$/;
const packageName = /^[a-z0-9][a-z0-9+.-]{0,127}$/;
const versionPattern = /^[0-9A-Za-z.+:~_-]{1,256}$/;
const noticePath = /^\/usr\/share\/(?:doc\/[A-Za-z0-9+._-]+(?:\/[A-Za-z0-9+._-]+)*\/copyright|common-licenses\/[A-Za-z0-9+._-]+)$/;
export const packageDatabase = "/var/lib/dpkg/status";

function isNotice(path) {
  return typeof path === "string" && path.length <= 512 && noticePath.test(path) && !path.split("/").some(part => part === "." || part === "..");
}

function architecture(platform) {
  assert.ok(["linux/amd64", "linux/arm64"].includes(platform), "expected a native Debian product platform");
  return platform.split("/")[1];
}

function libraryOwners(platform) {
  const triplet = architecture(platform) === "arm64" ? "aarch64-linux-gnu" : "x86_64-linux-gnu";
  return Object.fromEntries(Object.entries({
    "libssl.so.3": "libssl3", "libcrypto.so.3": "libssl3", "libc.so.6": "libc6", "libstdc++.so.6.0.30": "libstdc++6",
  }).map(([name, owner]) => [`/usr/lib/${triplet}/${name}`, owner]));
}

function packageIdentity(purl, name, version, platform) {
  const match = /^pkg:deb\/debian\/([^/?#]+)@([^/?#]+)\?([^#]+)$/.exec(purl ?? "");
  assert.ok(match && purl.length <= 1024, "expected a bounded Debian package URL");
  assert.match(name, packageName); assert.match(version, versionPattern);
  assert.equal(decodeURIComponent(match[1]), name, "Debian package URL name differs");
  assert.equal(decodeURIComponent(match[2]), version, "Debian package URL version differs");
  const params = new URLSearchParams(match[3]);
  assert.deepEqual([...new Set(params.keys())].sort(), [...params.keys()].sort(), "duplicate Debian package URL qualifier");
  assert.ok([...params.keys()].every(key => ["arch", "distro", "upstream"].includes(key)), "unreviewed Debian package qualifier");
  assert.ok([architecture(platform), "all"].includes(params.get("arch")), "Debian package architecture differs");
  assert.match(params.get("distro") ?? "", /^debian-12(?:\.[0-9]+)?$/, "unreviewed product distribution");
  const upstream = (params.get("upstream") ?? name).split("@");
  assert.ok(upstream.length <= 2, "ambiguous Debian source version");
  const [source_package, source_version = version] = upstream;
  assert.match(source_package, packageName); assert.match(source_version, versionPattern);
  return { name, version, architecture: params.get("arch"), source_package, source_version };
}

export function checkProductPackagePlan(plan, platform = plan?.platform, imageManifest = plan?.image_manifest) {
  architecture(platform);
  assert.equal(plan?.schema_version, 1, "missing product package inventory");
  assert.equal(plan.platform, platform, "product package inventory platform differs");
  assert.match(imageManifest ?? "", /^sha256:[0-9a-f]{64}$/);
  assert.equal(plan.image_manifest, imageManifest, "product package inventory image differs");
  assert.ok(Array.isArray(plan.packages) && plan.packages.length > 0 && plan.packages.length <= 1024, "expected bounded installed Debian packages");
  assert.ok(Array.isArray(plan.files) && plan.files.length > 0 && plan.files.length <= 512, "expected bounded product package files");
  assert.deepEqual(plan.packages.map(p => p.name), [...new Set(plan.packages.map(p => p.name))].sort(), "ambiguous or unsorted Debian packages");
  assert.deepEqual(plan.files.map(f => f.path), [...new Set(plan.files.map(f => f.path))].sort(), "ambiguous or unsorted package files");
  const files = new Map(plan.files.map(f => [f.path, f]));
  const owners = libraryOwners(platform);
  for (const file of plan.files) {
    assert.match(file.sha256 ?? "", hashPattern); assert.notEqual(file.sha256, "0".repeat(64), "placeholder product file digest");
    const kind = file.path === packageDatabase ? "database" : isNotice(file.path) ? "notice" : Object.hasOwn(owners, file.path) ? "library" : null;
    assert.ok(kind, "unreviewed product package file path"); assert.equal(file.kind, kind);
  }
  assert.ok(files.has(packageDatabase), "missing actual package database hash");
  for (const name of ["Apache-2.0", "GPL-2", "GPL-3", "LGPL-2.1"])
    assert.ok(files.has(`/usr/share/common-licenses/${name}`), "missing complete common licence hash");
  for (const pkg of plan.packages) {
    const expected = packageIdentity(pkg.purl, pkg.name, pkg.version, platform);
    for (const [key, value] of Object.entries(expected)) assert.equal(pkg[key], value, "Debian package identity differs");
    assert.ok(isNotice(pkg.notice_path) && pkg.notice_path.startsWith("/usr/share/doc/"), "unreviewed package copyright path");
    assert.equal(files.get(pkg.notice_path)?.kind, "notice", "missing complete installed package copyright");
  }
  for (const [path, owner] of Object.entries(owners)) {
    assert.equal(files.get(path)?.kind, "library", "missing actual native library hash");
    assert.equal(files.get(path).owner, owner, "native library package owner differs");
    assert.ok(plan.packages.some(pkg => pkg.name === owner), "missing native library package identity");
  }
  for (const [name, source] of Object.entries({ libssl3: "openssl", libc6: "glibc", "libstdc++6": "gcc-12" }))
    assert.equal(plan.packages.find(pkg => pkg.name === name)?.source_package, source, "native library source package differs");
  return plan;
}

export function productPackageInventory(spdx, platform, imageManifest) {
  architecture(platform);
  assert.ok(Array.isArray(spdx?.packages) && spdx.packages.length <= 50_000, "expected bounded SPDX packages");
  assert.ok(Array.isArray(spdx.files) && spdx.files.length <= 50_000, "expected bounded SPDX files");
  assert.ok(Array.isArray(spdx.relationships) && spdx.relationships.length <= 200_000, "expected bounded SPDX ownership");
  const packageIds = new Map();
  const packages = [];
  for (const pkg of spdx.packages) {
    const refs = pkg.externalRefs ?? [];
    assert.ok(Array.isArray(refs) && refs.length <= 32, "expected bounded package references");
    const urls = refs.filter(ref => ref.referenceType === "purl" && ref.referenceLocator?.startsWith("pkg:deb/"));
    if (!urls.length) continue;
    assert.equal(urls.length, 1, "ambiguous Debian package URL");
    assert.ok(typeof pkg.SPDXID === "string" && !packageIds.has(pkg.SPDXID), "ambiguous Debian package ID");
    const prefix = "acquired package info from DPKG DB: ";
    assert.ok(typeof pkg.sourceInfo === "string" && pkg.sourceInfo.length <= 16 * 1024 && pkg.sourceInfo.startsWith(prefix), "missing maintained DPKG provenance");
    const locations = pkg.sourceInfo.slice(prefix.length).split(", ");
    assert.equal(locations[0], packageDatabase, "package belongs to another database");
    const notices = locations.filter(path => path.endsWith("/copyright"));
    assert.equal(notices.length, 1, "ambiguous installed package copyright");
    packages.push({ ...packageIdentity(urls[0].referenceLocator, pkg.name, pkg.versionInfo, platform), purl: urls[0].referenceLocator, notice_path: notices[0] });
    packageIds.set(pkg.SPDXID, pkg.name);
  }
  const owners = libraryOwners(platform);
  const ownership = new Map();
  for (const relation of spdx.relationships) {
    if (relation.relationshipType !== "CONTAINS" || !packageIds.has(relation.spdxElementId)) continue;
    const names = ownership.get(relation.relatedSpdxElement) ?? new Set(); names.add(packageIds.get(relation.spdxElementId)); ownership.set(relation.relatedSpdxElement, names);
  }
  const files = [];
  const fileIds = new Set();
  for (const file of spdx.files) {
    const path = `/${file.fileName}`;
    const kind = path === packageDatabase ? "database" : isNotice(path) ? "notice" : Object.hasOwn(owners, path) ? "library" : null;
    if (!kind) continue;
    assert.ok(typeof file.SPDXID === "string" && file.SPDXID.length <= 512 && !fileIds.has(file.SPDXID), "ambiguous product file ID"); fileIds.add(file.SPDXID);
    assert.ok(Array.isArray(file.fileTypes) && file.fileTypes.length > 0 && file.fileTypes.every(type => ["TEXT", "APPLICATION", "BINARY"].includes(type)), "package file must describe regular data");
    assert.ok(Array.isArray(file.checksums) && file.checksums.length <= 8, "expected bounded product file checksums");
    const sums = file.checksums.filter(sum => sum.algorithm === "SHA256");
    assert.equal(sums.length, 1, "missing or ambiguous product file SHA-256");
    const entry = { path, kind, sha256: sums[0].checksumValue };
    if (kind === "library") {
      assert.deepEqual([...(ownership.get(file.SPDXID) ?? [])], [owners[path]], "native library SPDX owner differs");
      entry.owner = owners[path];
    }
    files.push(entry);
  }
  return checkProductPackagePlan({ schema_version: 1, platform, image_manifest: imageManifest,
    packages: packages.sort((a, b) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0), files: files.sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0) });
}

export function checkInstalledPackageDatabase(bytes, plan) {
  checkProductPackagePlan(plan);
  assert.ok(Buffer.isBuffer(bytes) && bytes.length > 0 && bytes.length <= 4 * 1024 * 1024, "package database exceeds its bound");
  const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  const packages = [];
  const paragraphs = text.trimEnd().split(/\n\n/);
  assert.ok(paragraphs.length > 0 && paragraphs.length <= 2048, "package database row bound exceeded");
  for (const paragraph of paragraphs) {
    const fields = new Map();
    for (const line of paragraph.split("\n")) {
      if (/^[ \t]/.test(line)) continue; // Debian control-field continuations are not identity headers.
      const match = /^([A-Za-z][A-Za-z0-9-]*):[ \t]*(.*)$/.exec(line);
      assert.ok(match && !fields.has(match[1]), "ambiguous package database field"); fields.set(match[1], match[2]);
    }
    if (fields.get("Status") !== "install ok installed") {
      assert.match(fields.get("Status") ?? "", /^(?:install|hold|deinstall|purge) (?:ok|reinstreq) (?:not-installed|config-files|half-installed|unpacked|half-configured|triggers-awaited|triggers-pending)$/,
        "unreviewed package database state");
      continue;
    }
    const name = fields.get("Package"), version = fields.get("Version"), arch = fields.get("Architecture");
    assert.match(name ?? "", packageName); assert.match(version ?? "", versionPattern);
    assert.ok([architecture(plan.platform), "all"].includes(arch), "database package architecture differs");
    const source = /^(\S+)(?: \(([^()]+)\))?$/.exec(fields.get("Source") ?? name);
    assert.ok(source, "ambiguous database source package");
    packages.push({ name, version, architecture: arch, source_package: source[1], source_version: source[2] ?? version });
  }
  packages.sort((a, b) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0);
  const expected = plan.packages.map(({ name, version, architecture, source_package, source_version }) => ({ name, version, architecture, source_package, source_version }));
  assert.deepEqual(packages, expected, "SPDX and actual installed database identities differ");
}

export function checkProductPackageImageReport(report, expected, platform, imageManifest) {
  checkProductPackagePlan(expected, platform, imageManifest);
  assert.equal(report?.schema_version, 1, "missing actual product package evidence");
  assert.equal(report.platform, platform); assert.equal(report.image_manifest, imageManifest);
  assert.deepEqual(report.packages, expected.packages, "actual product packages differ from OCI SPDX");
  assert.ok(Array.isArray(report.files) && report.files.length === expected.files.length, "actual product file inventory differs");
  let total = 0;
  for (const [i, file] of report.files.entries()) {
    const { bytes, ...identity } = file;
    assert.deepEqual(identity, expected.files[i], "actual product file hash differs from OCI SPDX");
    const limit = file.kind === "notice" ? 1024 * 1024 : file.kind === "database" ? 4 * 1024 * 1024 : 32 * 1024 * 1024;
    assert.ok(Number.isSafeInteger(bytes) && bytes > 0 && bytes <= limit, "actual product file size exceeds its bound"); total += bytes;
  }
  assert.ok(total <= 64 * 1024 * 1024, "actual product package evidence exceeds its bound");
}
