#!/usr/bin/env node
// OPS-12: one build result, two independently inspected registry destinations.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const releaseImages = Object.freeze({
  product: "product",
  postgres: "postgres",
  keycloak: "keycloak",
  proxy: "proxy",
  browser_acceptance: "browser-acceptance",
  helm_postgres: "cnpg-postgres",
});
export const platforms = ["linux/amd64", "linux/arm64"];
export const digestPattern = /^sha256:[0-9a-f]{64}$/;
const runDocker = (args) => execFileSync("docker", args, {
  encoding: "utf8", timeout: 120_000, maxBuffer: 8 * 1024 * 1024,
  stdio: ["ignore", "pipe", "pipe"],
});
const digest = (bytes) => `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
const readJson = (file) => JSON.parse(readFileSync(file, "utf8"));

export function imageNamespace(value) {
  if (value !== "ghcr.io/synveda" && !/^docker\.io\/[a-z0-9][a-z0-9_-]{1,253}$/.test(value ?? "")) {
    throw new Error("image namespace must be ghcr.io/synveda or docker.io/<owner namespace>");
  }
  return value;
}

export function publisherNamespace(value, publish) {
  if (!value && !publish) return "synveda-dry-run";
  imageNamespace(`docker.io/${value ?? ""}`);
  return value;
}

function identity(version, sourceSha) {
  if (!/^[0-9a-f]{40}$/.test(sourceSha ?? "")) throw new Error("expected a full source SHA");
  // Reuse the release's bounded SemVer grammar before any registry operation.
  execFileSync("sh", [fileURLToPath(new URL("release-version.sh", import.meta.url)), version], { stdio: "pipe" });
}

function tagFor(name, version) {
  return name === "helm_postgres" ? `17.11-synveda-${version}` : version;
}

export function validateIndex(index, image) {
  if (!Array.isArray(index?.manifests)) throw new Error(`${image}: missing multi-platform index`);
  const result = {};
  for (const platform of platforms) {
    const matches = index.manifests.filter((entry) => `${entry.platform?.os}/${entry.platform?.architecture}` === platform);
    if (matches.length !== 1 || !digestPattern.test(matches[0].digest)) {
      throw new Error(`${image}: expected exactly one ${platform} manifest`);
    }
    result[platform] = matches[0].digest;
  }
  return result;
}

export function indexDescriptors(index) {
  // Compare all descriptors, including SBOM/provenance attestations. Only the
  // outer index's repository-specific annotations may differ across registries.
  const stable = (value) => Array.isArray(value) ? value.map(stable)
    : value && typeof value === "object" ? Object.fromEntries(Object.keys(value).sort().map((key) => [key, stable(value[key])])) : value;
  return index.manifests.map((entry) => JSON.stringify(stable(entry))).sort();
}

function validateAttestations(index, platformDigests, reference) {
  for (const child of Object.values(platformDigests)) {
    const entries = index.manifests.filter((entry) => entry.platform?.os === "unknown" &&
      entry.annotations?.["vnd.docker.reference.type"] === "attestation-manifest" &&
      entry.annotations?.["vnd.docker.reference.digest"] === child && digestPattern.test(entry.digest));
    if (entries.length !== 1) throw new Error(`${reference}: missing or ambiguous BuildKit attestation descriptor for ${child}`);
  }
}

export function assertUnused(reference, run = runDocker) {
  try {
    run(["buildx", "imagetools", "inspect", "--raw", reference]);
  } catch (error) {
    // Authentication, DNS, throttling and transport failures must never be
    // interpreted as permission to overwrite an unknown registry coordinate.
    const stderr = String(error.stderr ?? "").trim();
    if (error.status === 1 && /(?:manifest unknown|manifest .* not found|: not found)$/i.test(stderr) &&
        !/unauthorized|denied|forbidden|429|timeout|no such host|connection|credential/i.test(stderr)) return;
    throw new Error(`cannot prove tag is unused: ${reference}; check registry access before retrying`);
  }
  throw new Error(`immutable release tag already exists: ${reference}; inspect the partial release, do not overwrite it`);
}

export function preflight(version, sourceSha, namespace, arch, run = runDocker) {
  identity(version, sourceSha);
  if (!["amd64", "arm64"].includes(arch)) throw new Error("expected native amd64 or arm64");
  const hub = `docker.io/${publisherNamespace(namespace, true)}`;
  for (const prefix of [hub, "ghcr.io/synveda"]) {
    for (const [name, repo] of Object.entries(releaseImages)) {
      const ref = `${prefix}/${repo}:${tagFor(name, version)}`;
      assertUnused(ref, run);
      assertUnused(`${ref}-${arch}`, run);
    }
  }
}

export function assemble(version, sourceSha, namespace, publish, run = runDocker) {
  identity(version, sourceSha);
  const targets = { dockerhub: `docker.io/${publisherNamespace(namespace, publish)}`, ghcr: "ghcr.io/synveda" };
  const result = { schema_version: 1, release_version: version, source_sha: sourceSha, published: publish, registries: {} };
  if (publish) {
    // Check every final tag before the first mutation. Architecture jobs have
    // already independently refused existing architecture tags.
    for (const prefix of Object.values(targets)) for (const [name, repo] of Object.entries(releaseImages)) {
      assertUnused(`${prefix}/${repo}:${tagFor(name, version)}`, run);
    }
  }
  for (const [registry, prefix] of Object.entries(targets)) {
    const images = {};
    for (const [name, repo] of Object.entries(releaseImages)) {
      const ref = `${prefix}/${repo}:${tagFor(name, version)}`;
      let raw;
      if (publish) {
        run(["buildx", "imagetools", "create", "--tag", ref, `${ref}-amd64`, `${ref}-arm64`]);
        raw = run(["buildx", "imagetools", "inspect", "--raw", ref]);
      } else {
        raw = JSON.stringify({
          annotations: { "synveda.dry-run": prefix },
          manifests: platforms.map((platform) => ({
            digest: digest(`${repo}/${platform}`), size: 1,
            mediaType: "application/vnd.oci.image.manifest.v1+json",
            platform: { os: "linux", architecture: platform.split("/")[1] },
          })),
        });
      }
      const index = JSON.parse(raw);
      const platformDigests = validateIndex(index, ref);
      if (publish) validateAttestations(index, platformDigests, ref);
      images[name] = { reference: `${prefix}/${repo}@${digest(raw)}`, platforms: platformDigests, descriptors: indexDescriptors(index) };
    }
    result.registries[registry] = { namespace: prefix, images };
  }
  validateRegistryManifest(result, version, sourceSha);
  return result;
}

export function validateRegistryManifest(manifest, version, sourceSha) {
  identity(version, sourceSha);
  if (manifest?.schema_version !== 1 || manifest.release_version !== version || manifest.source_sha !== sourceSha ||
      typeof manifest.published !== "boolean" || Object.keys(manifest.registries ?? {}).sort().join() !== "dockerhub,ghcr") {
    throw new Error("registry inventory does not match release identity");
  }
  for (const [registry, target] of Object.entries(manifest.registries)) {
    imageNamespace(target.namespace);
    if (registry === "ghcr" ? target.namespace !== "ghcr.io/synveda" : !target.namespace.startsWith("docker.io/")) {
      throw new Error("registry inventory has the wrong destination");
    }
    if (Object.keys(target.images ?? {}).sort().join() !== Object.keys(releaseImages).sort().join()) throw new Error("registry inventory requires all six images");
    for (const [name, repo] of Object.entries(releaseImages)) {
      const entry = target.images[name];
      const prefix = `${target.namespace}/${repo}@`;
      if (!entry?.reference?.startsWith(prefix) || !digestPattern.test(entry.reference.slice(prefix.length)) ||
          !Array.isArray(entry.descriptors) || entry.descriptors.some((value) => typeof value !== "string")) throw new Error(`invalid immutable registry entry: ${name}`);
      const index = { manifests: entry.descriptors.map((value) => JSON.parse(value)) };
      if (JSON.stringify(validateIndex(index, name)) !== JSON.stringify(entry.platforms)) throw new Error(`platform evidence disagrees: ${name}`);
      if (manifest.published) validateAttestations(index, entry.platforms, entry.reference);
      const mirror = manifest.registries.ghcr.images[name];
      if (JSON.stringify(entry.descriptors) !== JSON.stringify(mirror.descriptors)) throw new Error(`${name}: registries contain different build/attestation descriptors`);
    }
  }
}

export function packageReference(manifest, output, run = execFileSync) {
  validateRegistryManifest(manifest, manifest.release_version, manifest.source_sha);
  const primary = manifest.registries.dockerhub;
  run("bash", [fileURLToPath(new URL("package-release.sh", import.meta.url)), manifest.release_version, output, manifest.source_sha,
    ...Object.keys(releaseImages).map((name) => primary.images[name].reference.split("@")[1])], {
    stdio: "inherit", env: { ...process.env, SYNVEDA_IMAGE_NAMESPACE: primary.namespace },
  });
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const [command, ...args] = process.argv.slice(2);
    if (command === "namespace" && args.length === 1 && ["true", "false"].includes(args[0])) {
      console.log(publisherNamespace(process.env.DOCKERHUB_NAMESPACE, args[0] === "true"));
    } else if (command === "preflight" && args.length === 3) {
      if (!process.env.DOCKERHUB_USERNAME || !process.env.DOCKERHUB_TOKEN) throw new Error("publishing requires DOCKERHUB_USERNAME and DOCKERHUB_TOKEN in the protected release environment");
      preflight(args[0], args[1], process.env.DOCKERHUB_NAMESPACE, args[2]);
    } else if (command === "assemble" && args.length === 4 && ["true", "false"].includes(args[2])) {
      const result = assemble(args[0], args[1], process.env.DOCKERHUB_NAMESPACE, args[2] === "true");
      writeFileSync(args[3], `${JSON.stringify(result, null, 2)}\n`, { flag: "wx" });
    } else if (command === "package" && args.length === 2) {
      packageReference(readJson(args[0]), args[1]);
    } else {
      throw new Error("usage: release-registries.mjs namespace PUBLISH | preflight VERSION SHA ARCH | assemble VERSION SHA PUBLISH OUTPUT | package INVENTORY OUTPUT_DIRECTORY");
    }
  } catch (error) {
    console.error(`release-registries: ${error.message}`);
    process.exitCode = 1;
  }
}
