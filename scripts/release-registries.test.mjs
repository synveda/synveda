import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  assemble, assertUnused, imageNamespace, packageReference, preflight,
  publisherNamespace, releaseImages, validateRegistryManifest,
} from "./release-registries.mjs";
import { verifyRegistrySet } from "./verify-release-images.mjs";

const version = "0.0.0-local-test";
const source = "a".repeat(40);
const hash = (value) => `sha256:${createHash("sha256").update(value).digest("hex")}`;
const absent = () => { throw Object.assign(new Error("fixture"), { status: 1, stderr: "ERROR: manifest unknown" }); };
function index(reference) {
  const repo = reference.split("/").at(-1).split(":")[0];
  const manifests = ["amd64", "arm64"].flatMap((arch) => {
    const child = hash(`${repo}/${arch}`);
    return [
      { digest: child, size: 123, mediaType: "application/vnd.oci.image.manifest.v1+json", platform: { os: "linux", architecture: arch } },
      { digest: hash(`attestation/${repo}/${arch}`), size: 456, mediaType: "application/vnd.oci.image.manifest.v1+json", platform: { os: "unknown", architecture: "unknown" }, annotations: { "vnd.docker.reference.type": "attestation-manifest", "vnd.docker.reference.digest": child } },
    ];
  });
  return { schemaVersion: 2, annotations: { destination: reference }, manifests };
}
function registry(override = () => undefined) {
  const calls = [];
  const created = new Set();
  const run = (args) => {
    calls.push(args);
    const replacement = override(args);
    if (replacement !== undefined) return replacement;
    if (args[2] === "create") { created.add(args[4]); return ""; }
    if (args[2] === "inspect" && created.has(args.at(-1))) return `${JSON.stringify(index(args.at(-1)))}\n`;
    return absent();
  };
  return { calls, run };
}

test("publisher settings are validated; no namespace ownership is assumed", () => {
  assert.equal(publisherNamespace(undefined, false), "synveda-dry-run");
  assert.throws(() => publisherNamespace(undefined, true));
  assert.equal(publisherNamespace("owner-team", true), "owner-team");
  for (const value of ["", "synveda/other", "Owner", "x\nsecret", "x$(id)", "--flag", "../x"]) {
    assert.throws(() => publisherNamespace(value, true));
  }
  assert.throws(() => imageNamespace("attacker.example/synveda"));
});

test("dry run needs no credentials, performs no Docker operation and records synthetic identities", () => {
  const result = assemble(version, source, undefined, false, () => assert.fail("dry run contacted Docker"));
  assert.equal(result.published, false);
  assert.equal(result.registries.dockerhub.namespace, "docker.io/synveda-dry-run");
  assert.notEqual(result.registries.dockerhub.images.product.reference.split("@")[1], result.registries.ghcr.images.product.reference.split("@")[1]);
});

test("preflight refuses existing and ambiguous coordinates before any publication", () => {
  const calls = [];
  preflight(version, source, "owner-team", "arm64", (args) => { calls.push(args); absent(); });
  assert.equal(calls.length, 24);
  assert.ok(calls.every((args) => args[2] === "inspect"));
  assert.ok(calls.some((args) => args.at(-1) === `docker.io/owner-team/cnpg-postgres:17.11-synveda-${version}-arm64`));
  assert.throws(() => preflight(version, source, "owner-team", "x64"));
  assert.throws(() => preflight("../../bad", source, "owner-team", "amd64", () => assert.fail()));
  assert.throws(() => assertUnused("image:tag", () => "{}"), /already exists/);
  for (const stderr of ["unauthorized: not found", "denied: not found", "429: too many requests", "connection timed out", "get credentials: not found", "no such host", "ERROR: not found\nextra failure"]) {
    assert.throws(() => assertUnused("image:tag", () => { throw Object.assign(new Error(), { status: 1, stderr }); }), /cannot prove/);
  }
  for (const stderr of ["ERROR: manifest unknown", "ERROR: docker.io/owner/product:tag: not found"]) {
    assert.doesNotThrow(() => assertUnused("image:tag", () => { throw Object.assign(new Error(), { status: 1, stderr }); }));
  }
});

test("assembly joins each destination independently and preserves image plus attestation bytes", () => {
  const f = registry();
  const result = assemble(version, source, "owner-team", true, f.run);
  assert.equal(f.calls.filter((args) => args[2] === "create").length, 12);
  assert.ok(f.calls.slice(0, 12).every((args) => args[2] === "inspect"));
  for (const [name, repo] of Object.entries(releaseImages)) {
    const hub = result.registries.dockerhub.images[name];
    const ghcr = result.registries.ghcr.images[name];
    assert.ok(hub.reference.startsWith(`docker.io/owner-team/${repo}@`));
    assert.notEqual(hub.reference.split("@")[1], ghcr.reference.split("@")[1]);
    assert.deepEqual(hub.platforms, ghcr.platforms);
    assert.deepEqual(hub.descriptors, ghcr.descriptors);
    assert.equal(hub.descriptors.length, 4);
    const tag = `${name === "helm_postgres" ? "17.11-synveda-" : ""}${version}`;
    assert.equal(hub.reference.split("@")[1], hash(`${JSON.stringify(index(`docker.io/owner-team/${repo}:${tag}`))}\n`), "hash exact destination response bytes");
  }
  assert.ok(f.calls.every((args) => !["build", "login", "push"].includes(args[0])));
  const stripped = structuredClone(result);
  for (const target of Object.values(stripped.registries)) {
    target.images.product.descriptors = target.images.product.descriptors.filter((entry) => JSON.parse(entry).platform.os !== "unknown");
  }
  assert.throws(() => validateRegistryManifest(stripped, version, source), /attestation/);
});

test("reused final tags and failed inspections never proceed to a manifest mutation", () => {
  const f = registry((args) => args.at(-1).includes("/proxy:") ? "{}" : undefined);
  assert.throws(() => assemble(version, source, "owner-team", true, f.run), /already exists/);
  assert.equal(f.calls.some((args) => args[2] === "create"), false);
});

test("lost attestations, missing architectures and different mirror bytes refuse a release", () => {
  for (const mutate of [
    (doc) => { doc.manifests = doc.manifests.filter((entry) => entry.platform.os !== "unknown"); },
    (doc) => { doc.manifests = doc.manifests.filter((entry) => entry.platform.architecture !== "arm64"); },
    (doc) => { doc.manifests[1].digest = hash("changed attestation"); },
  ]) {
    let ghcrCreated = false;
    const f = registry((args) => {
      if (args[2] === "create" && args[4].startsWith("ghcr.io/")) ghcrCreated = true;
      if (ghcrCreated && args[2] === "inspect" && args.at(-1).startsWith("ghcr.io/")) {
        const doc = index(args.at(-1)); mutate(doc); return JSON.stringify(doc);
      }
      return undefined;
    });
    assert.throws(() => assemble(version, source, "owner-team", true, f.run), /attestation|arm64|different build/);
  }
});

test("consumer archives and Helm overlays carry the chosen destination's own digests", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda registry 空 格 "));
  try {
    const inventory = assemble(version, source, "owner-team", false);
    packageReference(inventory, scratch);
    execFileSync("tar", ["-xzf", join(scratch, `synveda-reference-${version}.tar.gz`), "-C", scratch]);
    const bundle = join(scratch, `synveda-reference-${version}`);
    const manifest = JSON.parse(readFileSync(join(bundle, "environment.json")));
    assert.equal(manifest.image_namespace, "docker.io/owner-team");
    for (const [name, entry] of Object.entries(inventory.registries.dockerhub.images)) assert.equal(manifest.images[name], entry.reference);
    for (const file of ["product-image", "synveda-compose", "deploy/compose/.env.example"]) {
      assert.ok(readFileSync(join(bundle, file), "utf8").includes(manifest.images.product));
    }
    assert.match(readFileSync(join(scratch, `synveda-images-${version}.yaml`), "utf8"), /repository: docker.io\/owner-team\/product/);
    assert.ok(readFileSync(join(scratch, `synveda-cnpg-image-${version}.yaml`), "utf8").includes(`docker.io/owner-team/cnpg-postgres:17.11-synveda-${version}@`));
    const bad = structuredClone(inventory); bad.registries.dockerhub.namespace = "$(unsafe)";
    assert.throws(() => packageReference(bad, scratch, () => assert.fail("bad inventory reached packager")));
  } finally { rmSync(scratch, { recursive: true, force: true }); }
});

test("anonymous verification checks both destinations and refuses a synthetic or transplanted bundle", () => {
  const f = registry();
  const inventory = assemble(version, source, "owner-team", true, f.run);
  const manifest = {
    schema_version: 1, release_version: version, source_sha: source, deployment_contract: "CPR-45/ADR-0102",
    image_namespace: inventory.registries.dockerhub.namespace,
    images: Object.fromEntries(Object.entries(inventory.registries.dockerhub.images).map(([name, entry]) => [name, entry.reference])),
    external_images: { otel_collector: `upstream/otel:1@${hash("otel")}`, prometheus: `upstream/prom:1@${hash("prom")}` },
  };
  const calls = [];
  const run = (args) => {
    calls.push(args);
    if (args[0] === "info") return "linux/aarch64";
    if (args[0] === "buildx") {
      const expected = Object.values(inventory.registries).flatMap((entry) => Object.values(entry.images)).find((entry) => entry.reference === args.at(-1));
      return JSON.stringify(expected ? { manifests: expected.descriptors.map((entry) => JSON.parse(entry)) } : index(args.at(-1)));
    }
    if (args[0] === "image") {
      const familiarDigest = args[2]
        .replace(/:[^/:@]+(?=@sha256:)/, "")
        .replace(/^docker\.io\//, "");
      return JSON.stringify([{
        Os: "linux", Architecture: "arm64", Id: hash("image"),
        RepoDigests: [familiarDigest],
        Config: { Labels: {
          "org.opencontainers.image.source": "https://github.com/synveda/synveda",
          "org.opencontainers.image.revision": source,
          "org.opencontainers.image.version": version,
        } },
      }]);
    }
    if (args[0] === "run" && args.includes("/usr/local/bin/synveda")) return `synveda ${version}`;
    return "";
  };
  const report = verifyRegistrySet(manifest, inventory, "linux/arm64", version, source, run);
  assert.deepEqual(Object.keys(report.registries), ["dockerhub", "ghcr"]);
  assert.equal(calls.filter((args) => args[0] === "pull").length, 16);
  for (const mutate of [
    (digest) => digest.replace(/@sha256:.*/, `@${hash("wrong image")}`),
    (digest) => digest.replace("owner-team/product", "other/product"),
  ]) {
    const wrongLocalDigest = (args) => {
      if (args[0] !== "image") return run(args);
      const [local] = JSON.parse(run(args));
      if (args[2] === manifest.images.product) {
        local.RepoDigests = [mutate(local.RepoDigests[0])];
      }
      return JSON.stringify([local]);
    };
    assert.throws(() => verifyRegistrySet(manifest, inventory, "linux/arm64", version, source, wrongLocalDigest), /product: local image lost its release digest/);
  }
  const changed = structuredClone(manifest);
  changed.images.product = changed.images.product.replace(/@.*/, `@${hash("wrong")}`);
  for (const [environment, record] of [[changed, inventory], [manifest, { ...inventory, published: false }]]) {
    assert.throws(() => verifyRegistrySet(environment, record, "linux/arm64", version, source, () => assert.fail("invalid input reached Docker")));
  }
  assert.throws(() => validateRegistryManifest(inventory, version, "b".repeat(40)), /identity/);
  assert.throws(() => verifyRegistrySet(manifest, inventory, "linux/arm64", version, source, (args) => {
    if (args[0] === "buildx") return JSON.stringify(index("different/image:tag"));
    return run(args);
  }), /descriptors disagree/);
});
