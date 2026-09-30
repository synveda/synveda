import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { checkRustImageReport, checkRustStatement, inspectRustImageSbom, requiredRustPackages } from "./rust-image-sbom.mjs";

const source = "a".repeat(40);
const version = "0.4.3";
const digest = `sha256:${"b".repeat(64)}`;
const requirements = {
  "synveda-cli": version, "synveda-gateway": version, "synveda-apalis": version,
  "cedar-policy": "4.11.2", "cedar-policy-core": "4.11.2", "sqlx": "0.8.6", "sqlx-postgres": "0.8.6",
};
const statement = () => ({
  _type: "https://in-toto.io/Statement/v0.1",
  subject: [{ name: "native-image", digest: { sha256: digest.slice(7) } }],
  predicateType: "https://spdx.dev/Document",
  predicate: {
    spdxVersion: "SPDX-2.3", creationInfo: { creators: ["Tool: syft-v1.51.0"] },
    packages: Object.entries(requirements).map(([name, versionInfo]) => ({
      name, versionInfo, externalRefs: [{ referenceType: "purl", referenceLocator: `pkg:cargo/${name}@${versionInfo}` }],
    })),
  },
});

function fixture(t, mutate = () => {}) {
  const root = mkdtempSync(join(tmpdir(), "synveda-rust-sbom-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const stage = join(root, "oci");
  const blobs = join(stage, "blobs/sha256");
  mkdirSync(blobs, { recursive: true });
  const blob = (value, mediaType) => {
    const bytes = Buffer.from(JSON.stringify(value));
    const sha = createHash("sha256").update(bytes).digest("hex");
    writeFileSync(join(blobs, sha), bytes);
    return { mediaType, digest: `sha256:${sha}`, size: bytes.length };
  };
  const config = { os: "linux", architecture: "arm64", config: { Labels: {
    "org.opencontainers.image.source": "https://github.com/synveda/synveda",
    "org.opencontainers.image.revision": source, "org.opencontainers.image.version": version,
  } } };
  const doc = statement();
  const native = { schemaVersion: 2, layers: [] };
  const attestation = { schemaVersion: 2, layers: [] };
  const index = { schemaVersion: 2, manifests: [] };
  const data = { config, doc, native, attestation, index };
  const controls = mutate(data) ?? {};
  native.config = blob(config, "application/vnd.oci.image.config.v1+json");
  const image = { ...blob(native, "application/vnd.oci.image.manifest.v1+json"), platform: { os: "linux", architecture: "arm64" } };
  doc.subject[0].digest.sha256 = controls.foreignSubject ?? image.digest.slice(7);
  attestation.config = native.config;
  attestation.layers = [{ ...blob(doc, "application/vnd.in-toto+json"), annotations: { "in-toto.io/predicate-type": "https://spdx.dev/Document" } }];
  controls.mutateAttestation?.(attestation);
  const evidence = { ...blob(attestation, "application/vnd.oci.image.manifest.v1+json"),
    platform: { os: "unknown", architecture: "unknown" }, annotations: {
      "vnd.docker.reference.type": "attestation-manifest", "vnd.docker.reference.digest": image.digest,
    } };
  index.manifests = [image, evidence];
  controls.finalize?.({ ...data, image, evidence });
  const wrapper = { schemaVersion: 2, manifests: [blob(index, "application/vnd.oci.image.index.v1+json")] };
  writeFileSync(join(stage, "index.json"), JSON.stringify(wrapper));
  const archive = join(root, "image.tar");
  const pack = (extra = []) => execFileSync("tar", ["-cf", archive, "-C", stage, "index.json", "blobs", ...extra]);
  pack();
  return { archive, image, doc, blobs, stage, pack };
}
const inspect = (f, arch = "arm64", expectedSource = source) => inspectRustImageSbom(f.archive, arch, "product", version, expectedSource);

test("runtime requirements follow the locked Cedar/SQLx graph and image roots", () => {
  assert.deepEqual(requiredRustPackages("product", version), requirements);
  assert.equal(Object.keys(requiredRustPackages("browser_acceptance", version)).length, 5);
  assert.throws(() => requiredRustPackages("postgres", version), /Rust-bearing image/);
});

test("actual OCI blobs bind Cargo inventory to the native image and exact source", (t) => {
  const f = fixture(t);
  const report = inspect(f);
  assert.equal(report.image_manifest, f.image.digest);
  assert.equal(report.cargo_package_count, 7);
  assert.deepEqual(report.required_packages, requirements);
  checkRustImageReport(report, "product", version, f.image.digest);
  const cli = execFileSync(process.execPath, [fileURLToPath(new URL("./rust-image-sbom.mjs", import.meta.url)),
    f.archive, "arm64", "product", version, source], { encoding: "utf8" });
  assert.deepEqual(JSON.parse(cli), report);
  assert.throws(() => inspect(f, "amd64"), /another platform/);
  assert.throws(() => inspect(f, "arm64", "c".repeat(40)), /source binding/);
  assert.throws(() => checkRustImageReport(report, "product", version, digest), /binding differs/);
});

test("valid SPDX envelopes cannot hide absent, mismatched or ambiguous runtime Cargo content", () => {
  const controls = [
    (s) => { s.predicate.packages = []; },
    (s) => { s.predicate.packages.forEach((p) => { p.externalRefs = []; }); },
    (s) => { s.predicate.packages = s.predicate.packages.filter((p) => p.name !== "cedar-policy"); },
    (s) => { s.predicate.packages.find((p) => p.name === "sqlx").versionInfo = "0.0.0"; },
    (s) => { const p = s.predicate.packages.find((p) => p.name === "sqlx"); p.versionInfo = "0.0.0"; p.externalRefs[0].referenceLocator = "pkg:cargo/sqlx@0.0.0"; },
    (s) => { const p = structuredClone(s.predicate.packages.find((p) => p.name === "cedar-policy")); p.versionInfo = "0.0.0"; p.externalRefs[0].referenceLocator = "pkg:cargo/cedar-policy@0.0.0"; s.predicate.packages.push(p); },
    (s) => { s.predicate.creationInfo.creators = ["Tool: syft-v0.1.0"]; },
    (s) => { s.subject = []; },
    (s) => { s.subject[0].digest.sha256 = "e".repeat(64); },
    (s) => { s.subject.push({ digest: { sha256: "e".repeat(64) } }); },
    (s) => { s.predicateType = "https://slsa.dev/provenance/v1"; },
  ];
  assert.equal(checkRustStatement(statement(), digest, "product", version).cargo_package_count, 7);
  for (const damage of controls) {
    const s = statement(); damage(s);
    assert.throws(() => checkRustStatement(s, digest, "product", version));
  }
});

test("Cargo package URLs decode build metadata without relaxing version identity", () => {
  const s = statement();
  s.predicate.packages.push({ name: "serde_yaml", versionInfo: "0.9.34+deprecated",
    externalRefs: [{ referenceType: "purl", referenceLocator: "pkg:cargo/serde_yaml@0.9.34%2Bdeprecated" }] });
  assert.equal(checkRustStatement(s, digest, "product", version).cargo_package_count, 8);
  s.predicate.packages.at(-1).externalRefs[0].referenceLocator = "pkg:cargo/serde_yaml@0.9.34%XXdeprecated";
  assert.throws(() => checkRustStatement(s, digest, "product", version), URIError);
});

test("foreign subjects, multiple images and missing or ambiguous attestations fail before qualification", (t) => {
  const controls = [
    () => ({ foreignSubject: "e".repeat(64) }),
    () => ({ mutateAttestation: (a) => { a.subject = { digest }; } }),
    () => ({ mutateAttestation: (a) => { a.layers = []; } }),
    () => ({ mutateAttestation: (a) => { a.layers.push(a.layers[0]); } }),
    () => ({ finalize: ({ index, image }) => index.manifests.push(image) }),
    () => ({ finalize: ({ index, evidence }) => index.manifests.push(evidence) }),
    () => ({ finalize: ({ evidence }) => { evidence.annotations["vnd.docker.reference.digest"] = digest; } }),
    ({ config }) => { config.architecture = "amd64"; },
  ];
  for (const damage of controls) assert.throws(() => inspect(fixture(t, damage)));
});

test("changed blobs, duplicate tar members and links cannot supply OCI evidence", (t) => {
  const changed = fixture(t);
  const member = `blobs/sha256/${changed.image.digest.slice(7)}`;
  const original = readFileSync(join(changed.stage, member), "utf8");
  writeFileSync(join(changed.stage, member), original.replace('"layers":[]', '"layers":{}'));
  changed.pack();
  assert.throws(() => inspect(changed), /blob digest/);
  const duplicate = fixture(t);
  duplicate.pack([`blobs/sha256/${duplicate.image.digest.slice(7)}`]);
  assert.throws(() => inspect(duplicate), /one regular file/);
  const linked = fixture(t);
  const indexPath = join(linked.stage, "index.json");
  writeFileSync(join(linked.stage, "other.json"), readFileSync(indexPath));
  rmSync(indexPath); symlinkSync("other.json", indexPath);
  linked.pack(["other.json"]);
  assert.throws(() => inspect(linked), /one regular file/);
});
