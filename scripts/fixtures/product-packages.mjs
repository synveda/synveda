import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { productPackageInventory } from "../product-package-inventory.mjs";
import { sha256 } from "../client-artifact.mjs";

// A small standards-shaped SPDX/database pair keeps source, ownership and bytes
// independent in refusal tests; it is never native package qualification.
export function productPackageFixture(arch = "arm64", imageManifest = `sha256:${"1".repeat(64)}`) {
  const bytes = new Map();
  const packages = [
    { name: "libc6", source: "glibc", version: "2.36-9+deb12u14" },
    { name: "libssl3", source: "openssl", version: "3.0.20-1~deb12u2" },
    { name: "libstdc++6", source: "gcc-12", version: "12.2.0-14+deb12u1" },
  ];
  const spdx = { packages: [], files: [], relationships: [] };
  const add = (path, contents, owner, type = "TEXT") => {
    bytes.set(path, Buffer.from(contents));
    const SPDXID = `SPDXRef-File-${spdx.files.length}`;
    spdx.files.push({ fileName: path.slice(1), SPDXID, fileTypes: [type], checksums: [{ algorithm: "SHA256", checksumValue: sha256(contents) }] });
    if (owner) spdx.relationships.push({ spdxElementId: `SPDXRef-Package-${owner}`, relatedSpdxElement: SPDXID, relationshipType: "CONTAINS" });
  };
  for (const pkg of packages) {
    const notice = `/usr/share/doc/${pkg.name}/copyright`;
    add(notice, Buffer.from(`Full upstream notice for ${pkg.name}\nCopyright fixture author\nComplete terms\n`));
    spdx.packages.push({ name: pkg.name, versionInfo: pkg.version, SPDXID: `SPDXRef-Package-${pkg.name}`,
      sourceInfo: `acquired package info from DPKG DB: /var/lib/dpkg/status, ${notice}`,
      externalRefs: [{ referenceType: "purl", referenceLocator: `pkg:deb/debian/${encodeURIComponent(pkg.name)}@${encodeURIComponent(pkg.version)}?arch=${arch}&distro=debian-12.15&upstream=${pkg.source}` }] });
  }
  const status = packages.map(pkg => `Package: ${pkg.name}\nStatus: install ok installed\nArchitecture: ${arch}\nVersion: ${pkg.version}\nSource: ${pkg.source}\nDescription: fixture\n continued description\n`).join("\n");
  add("/var/lib/dpkg/status", Buffer.from(status));
  for (const name of ["Apache-2.0", "GPL-2", "GPL-3", "LGPL-2.1"]) add(`/usr/share/common-licenses/${name}`, Buffer.from(`Complete ${name} fixture terms\n`));
  const triplet = arch === "arm64" ? "aarch64-linux-gnu" : "x86_64-linux-gnu";
  for (const [name, owner] of [["libssl.so.3", "libssl3"], ["libcrypto.so.3", "libssl3"], ["libc.so.6", "libc6"], ["libstdc++.so.6.0.30", "libstdc++6"]]) {
    const elf = Buffer.alloc(96); Buffer.from("7f454c460201", "hex").copy(elf); elf.writeUInt16LE(arch === "arm64" ? 183 : 62, 18);
    add(`/usr/lib/${triplet}/${name}`, elf, owner, "BINARY");
  }
  const plan = productPackageInventory(spdx, `linux/${arch}`, imageManifest);
  const report = { ...plan, files: plan.files.map(file => ({ ...file, bytes: bytes.get(file.path).length })) };
  const tar = (path, extra = []) => {
    const root = mkdtempSync(join(tmpdir(), "synveda-package-file-fixture-"));
    try {
      const member = path.split("/").at(-1);
      mkdirSync(join(root, "data")); writeFileSync(join(root, "data", member), bytes.get(path));
      return execFileSync("tar", ["-cf", "-", "-C", join(root, "data"), member, ...extra]);
    } finally { rmSync(root, { recursive: true, force: true }); }
  };
  return { spdx, plan, report, bytes, tar };
}
