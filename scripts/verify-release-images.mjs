#!/usr/bin/env node
// OPS-8 / CPR-45: immutable image smoke on a native runner. Public verification
// stays anonymous; only the internal candidate path admits the loopback registry.
import { execFileSync } from "node:child_process";
import assert from "node:assert/strict";
import { createHash, randomUUID } from "node:crypto";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  digestPattern, imageNamespace, indexDescriptors, platforms,
  releaseImages as repositories, validateIndex, validateRegistryManifest,
} from "./release-registries.mjs";
import { readConsoleArchive } from "./check-console-package.mjs";
import { checkConsoleFiles } from "../console/build/dependency-contract.mjs";
import { checkNodeFiles, nodeInventory, nodeMetadataArguments } from "./node-runtime-inventory.mjs";
import { readArchiveMember } from "./rust-archive-sbom.mjs";

export { validateIndex } from "./release-registries.mjs";
const readJson = (path) => JSON.parse(readFileSync(path, "utf8"));
const noticeFiles = ["LICENSE", "NOTICE"];
export const noticeHashes = Object.fromEntries(noticeFiles.map((name) => [name,
  createHash("sha256").update(readFileSync(new URL(`../${name}`, import.meta.url))).digest("hex")]));
const noticeCommand = ["/bin/sh", "-ec", "sha256sum /usr/share/licenses/synveda/LICENSE /usr/share/licenses/synveda/NOTICE"];
const expectedNotices = noticeFiles.map((name) => `${noticeHashes[name]}  /usr/share/licenses/synveda/${name}`).join("\n");
const repository = fileURLToPath(new URL("../", import.meta.url));
const maxConsoleStream = 34 * 1024 * 1024;
const execute = (args, timeout = 60_000, binary = false, maxBytes = binary ? maxConsoleStream : 8 * 1024 * 1024) => {
  const output = execFileSync("docker", args, {
    encoding: binary ? undefined : "utf8",
    timeout,
    maxBuffer: maxBytes,
    stdio: ["ignore", "pipe", "pipe"],
  });
  return binary ? output : output.trim();
};

export function inspectConsoleImage(imageId, platform, version, source, run = execute, root = repository) {
  assert.match(imageId, digestPattern);
  assert.ok(platforms.includes(platform), "expected a native Linux platform");
  const container = `synveda-console-check-${randomUUID()}`;
  const scratch = mkdtempSync(join(tmpdir(), "synveda-console-image-"));
  let report, failure;
  try {
    const id = run(["create", "--name", container, "--pull=never", "--platform", platform,
      "--network=none", "--read-only", "--cap-drop=ALL", "--security-opt=no-new-privileges",
      "--user=65532:65532", "--pids-limit=64", "--memory=128m", "--cpus=1",
      "--entrypoint", "/usr/local/bin/synveda", imageId, "--version"]);
    assert.match(id, /^[0-9a-f]{64}$/, "expected one stopped inspection container");
    const bytes = run(["cp", `${container}:/usr/share/synveda/console`, "-"], 60_000, true);
    assert.ok(Buffer.isBuffer(bytes) && bytes.length > 0 && bytes.length <= maxConsoleStream, "console image stream exceeds its bound");
    const archive = join(scratch, "console.tar");
    writeFileSync(archive, bytes, { flag: "wx", mode: 0o600 });
    report = { ...checkConsoleFiles(readConsoleArchive(archive, false), root, version, source), source_sha: source, version };
  } catch (error) { failure = error; }
  // Remove only this invocation's container/storage; preserve the inspection cause.
  try { run(["rm", "--force", "--volumes", container], 15_000); } catch (error) { failure ??= error; }
  try { rmSync(scratch, { recursive: true }); } catch (error) { failure ??= error; }
  if (failure) throw failure;
  return report;
}

export function inspectNodeImage(imageId, platform, run = execute, root = repository) {
  assert.match(imageId, digestPattern);
  assert.ok(platforms.includes(platform), "expected a native Linux platform");
  const target = platform === "linux/arm64" ? "linux-arm64" : "linux-x86_64";
  const lock = readJson(join(root, "scripts/node-runtimes.json")).product;
  const container = `synveda-node-check-${randomUUID()}`;
  const scratch = mkdtempSync(join(tmpdir(), "synveda-node-image-"));
  let report, failure;
  try {
    const id = run(["create", "--name", container, "--pull=never", "--platform", platform,
      "--network=none", "--read-only", "--cap-drop=ALL", "--security-opt=no-new-privileges",
      "--user=65532:65532", "--pids-limit=64", "--memory=256m", "--cpus=1",
      "--env=NODE_OPTIONS=", "--env=NODE_PATH=", "--entrypoint", "/usr/local/bin/node", imageId, ...nodeMetadataArguments]);
    assert.match(id, /^[0-9a-f]{64}$/, "expected one stopped Node inspection container");
    const read = (path, member, limit) => {
      const maxStream = limit + 2 * 1024 * 1024;
      const bytes = run(["cp", `${container}:${path}`, "-"], 60_000, true, maxStream);
      assert.ok(Buffer.isBuffer(bytes) && bytes.length > 0 && bytes.length <= maxStream, "Node image stream exceeds its bound");
      const archive = join(scratch, `${member}.tar`);
      writeFileSync(archive, bytes, { flag: "wx", mode: 0o600 });
      return readArchiveMember(archive, member, limit);
    };
    const binary = read("/usr/local/bin/node", "node", 256 * 1024 * 1024);
    const notices = Object.fromEntries(["LICENSE", ...Object.keys(lock.supplementary_notices)].map((name) =>
      [name, read(`/usr/share/licenses/node/${name}`, name, 1024 * 1024)]));
    const files = checkNodeFiles(binary, notices, target, lock);
    const metadata = run(["start", "--attach", container], 30_000, false, 64 * 1024);
    assert.ok(typeof metadata === "string" && Buffer.byteLength(metadata) <= 64 * 1024, "Node image metadata exceeds its bound");
    const observed = JSON.parse(metadata);
    report = { ...nodeInventory(files, observed, target, lock), runtime_version: observed.versions.node };
  } catch (error) { failure = error; }
  try { run(["rm", "--force", "--volumes", container], 15_000); } catch (error) { failure ??= error; }
  try { rmSync(scratch, { recursive: true }); } catch (error) { failure ??= error; }
  if (failure) throw failure;
  return report;
}

export function validateEnvironment(manifest, version, sourceSha, local = false) {
  if (
    manifest.schema_version !== 1 ||
    manifest.deployment_contract !== "CPR-45/ADR-0102" ||
    manifest.release_version !== version ||
    manifest.source_sha !== sourceSha ||
    !/^[0-9a-f]{40}$/.test(sourceSha) ||
    !/^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$/.test(version)
  )
    throw new Error(
      "release environment does not match the expected version/source",
    );
  if (
    Object.keys(manifest.images ?? {})
      .sort()
      .join() !== Object.keys(repositories).sort().join()
  ) {
    throw new Error("release must contain exactly six first-party images");
  }
  const namespace = local && manifest.image_namespace === "localhost:5000/synveda"
    ? manifest.image_namespace : imageNamespace(manifest.image_namespace ?? "ghcr.io/synveda");
  for (const [name, repository] of Object.entries(repositories)) {
    const prefix = `${namespace}/${repository}@`;
    const image = manifest.images[name];
    if (
      typeof image !== "string" ||
      !image.startsWith(prefix) ||
      !digestPattern.test(image.slice(prefix.length))
    ) {
      throw new Error(
        `${name}: expected the fixed repository and an immutable digest`,
      );
    }
  }
  if (
    Object.keys(manifest.external_images ?? {})
      .sort()
      .join() !== "otel_collector,prometheus"
  ) {
    throw new Error(
      "release must name the Collector and Prometheus dependencies",
    );
  }
  for (const image of Object.values(manifest.external_images)) {
    if (
      typeof image !== "string" ||
      !/^[A-Za-z0-9_./:+-]+@sha256:[0-9a-f]{64}$/.test(image)
    ) {
      throw new Error("upstream image must be digest pinned");
    }
  }
}

export function requireAnonymousConfig(directory) {
  if (!directory)
    throw new Error("set DOCKER_CONFIG to a fresh anonymous directory");
  const path = join(directory, "config.json");
  if (!existsSync(path)) return;
  const config = readJson(path);
  if (
    Object.keys(config.auths ?? {}).length ||
    config.credsStore ||
    Object.keys(config.credHelpers ?? {}).length
  ) {
    throw new Error(
      "registry verification requires an anonymous Docker configuration",
    );
  }
}

// These checks execute the packaged tools and inspect required runtime assets.
// Full startup, PKCE, persistence and recovery belong to Compose acceptance.
export const smokeCommands = {
  product: [
    ["/usr/local/bin/synveda", "--version"],
    [
      "/bin/sh",
      "-ec",
      "test -s /usr/share/synveda/console/index.html; test -s /usr/share/synveda/console/assets/Inter-OFL.txt; test -x /usr/local/bin/synveda-gateway; test -x /usr/local/bin/synveda-worker; test -x /usr/local/bin/synveda-apalis-worker; test -x /usr/local/bin/synveda-container",
    ],
  ],
  postgres: [
    [
      "/bin/sh",
      "-ec",
      "postgres --version; test -s /usr/share/postgresql/17/extension/vector.control; test -x /usr/local/bin/synveda-database-bootstrap; test -x /usr/local/bin/synveda-logical-backup; test -x /usr/local/bin/synveda-logical-restore",
    ],
  ],
  helm_postgres: [
    [
      "/bin/sh",
      "-ec",
      "postgres --version; test -s /usr/share/postgresql/17/extension/vector.control; test -x /usr/local/bin/synveda-database-bootstrap",
    ],
  ],
  keycloak: [["/opt/keycloak/bin/kc.sh", "--version"]],
  proxy: [["/usr/bin/caddy", "version"]],
  browser_acceptance: [
    ["/usr/local/bin/synveda", "--version"],
    [
      "node",
      "--input-type=module",
      "-e",
      "import { chromium } from 'playwright-core'; import { accessSync } from 'node:fs'; accessSync(chromium.executablePath()); accessSync('console-login.mjs'); accessSync('product-demo.mjs');",
    ],
  ],
};

export function verifyImages(
  manifest,
  platform,
  version,
  sourceSha,
  run = execute,
  localCandidate = false,
  root = repository,
) {
  validateEnvironment(manifest, version, sourceSha, localCandidate);
  if (!platforms.includes(platform))
    throw new Error("expected linux/amd64 or linux/arm64");
  const native = run(["info", "--format", "{{.OSType}}/{{.Architecture}}"])
    .replace("/x86_64", "/amd64")
    .replace("/aarch64", "/arm64");
  if (native !== platform)
    throw new Error(`expected native ${platform} engine, got ${native}`);
  const images = [];
  for (const [name, image] of Object.entries({
    ...manifest.images,
    ...manifest.external_images,
  })) {
    const firstParty = Object.hasOwn(repositories, name);
    const index = JSON.parse(run(["buildx", "imagetools", "inspect", "--raw", image]));
    let descriptors;
    if (localCandidate && firstParty) {
      const native = index.manifests?.filter((entry) => entry.platform?.os === "linux");
      if (native?.length !== 1 || `${native[0].platform.os}/${native[0].platform.architecture}` !== platform || !digestPattern.test(native[0].digest)) throw new Error("candidate must contain exactly the native image");
      const attestations = index.manifests.filter((entry) => entry.platform?.os === "unknown" &&
        entry.annotations?.["vnd.docker.reference.type"] === "attestation-manifest" &&
        entry.annotations?.["vnd.docker.reference.digest"] === native[0].digest && digestPattern.test(entry.digest));
      if (attestations.length !== 1) throw new Error("candidate lost its BuildKit attestation descriptor");
      descriptors = { [platform]: native[0].digest };
    } else descriptors = validateIndex(index, image);
    run(["pull", "--platform", platform, image], 600_000);
    const [local] = JSON.parse(run(["image", "inspect", image]));
    if (
      `${local?.Os}/${local?.Architecture}` !== platform ||
      !digestPattern.test(local?.Id)
    ) {
      throw new Error(
        `${name}: pulled image has the wrong architecture or identity`,
      );
    }
    const canonicalImage = image.replace(/:[^/:@]+(?=@sha256:)/, "");
    // Docker reports Hub references by their familiar name after a pull, even
    // when the immutable input explicitly names docker.io.
    const localNames = canonicalImage.startsWith("docker.io/")
      ? [canonicalImage, canonicalImage.slice("docker.io/".length)]
      : [canonicalImage];
    if (!local.RepoDigests?.some((digest) => localNames.includes(digest)))
      throw new Error(`${name}: local image lost its release digest`);
    if (firstParty) {
      const labels = local.Config?.Labels;
      if (
        labels?.["org.opencontainers.image.source"] !==
          "https://github.com/synveda/synveda" ||
        labels?.["org.opencontainers.image.revision"] !== sourceSha ||
        labels?.["org.opencontainers.image.version"] !== version
      ) {
        throw new Error(
          `${name}: source/revision/version labels disagree with the release`,
        );
      }
    }
    const commands = [...(smokeCommands[name] ?? []), ...(firstParty ? [noticeCommand] : [])];
    for (const command of commands) {
      const container = `synveda-release-check-${randomUUID()}`;
      try {
        const output = run([
          "run",
          "--rm",
          "--name",
          container,
          "--pull=never",
          "--platform",
          platform,
          "--network=none",
          "--read-only",
          "--cap-drop=ALL",
          "--security-opt=no-new-privileges",
          "--user=65532:65532",
          "--pids-limit=256",
          "--memory=1g",
          "--cpus=2",
          "--tmpfs=/tmp:rw,nosuid,nodev,size=67108864",
          "--entrypoint",
          command[0],
          image,
          ...command.slice(1),
        ]);
        if (
          command[0] === "/usr/local/bin/synveda" &&
          output !== `synveda ${version}`
        ) {
          throw new Error(
            `${name}: compiled CLI version disagrees with the release`,
          );
        }
        if (command === noticeCommand && output !== expectedNotices) {
          throw new Error(`${name}: packaged licence/notice hashes disagree with source`);
        }
      } finally {
        // Bound cleanup even when a Docker client times out after container start.
        try {
          run(["rm", "--force", container], 15_000);
        } catch {
          /* --rm may already have removed it. */
        }
      }
    }
    images.push({
      name,
      image,
      platforms: descriptors,
      image_id: local.Id,
      executable_checks: commands.length,
      ...(firstParty ? { notice_sha256: noticeHashes } : {}),
      ...(name === "product" ? { console_inventory: inspectConsoleImage(local.Id, platform, version, sourceSha, run, root) } : {}),
      ...(name === "product" ? { node_inventory: inspectNodeImage(local.Id, platform, run, root) } : {}),
    });
  }
  return {
    schema_version: 1,
    release_version: version,
    source_sha: sourceSha,
    platform,
    anonymous_pull: !localCandidate,
    ...(localCandidate ? { local_candidate: true } : {}),
    checked_at: new Date().toISOString(),
    scope:
      "Image pull, actual console/Node inventory and isolated executable/asset smoke; no deployment or OIDC acceptance.",
    images,
  };
}

export function verifyRegistrySet(manifest, inventory, platform, version, sourceSha, run = execute, root = repository) {
  validateEnvironment(manifest, version, sourceSha);
  validateRegistryManifest(inventory, version, sourceSha);
  if (!inventory.published) throw new Error("dry-run registry inventory is not installable");
  const primary = inventory.registries.dockerhub;
  if (manifest.image_namespace !== primary.namespace || Object.keys(repositories).some(
    (name) => manifest.images[name] !== primary.images[name].reference,
  )) throw new Error("consumer bundle must use the inspected Docker Hub destination digests");
  const registries = {};
  for (const [registry, target] of Object.entries(inventory.registries)) {
    const images = Object.fromEntries(Object.entries(target.images).map(([name, entry]) => [name, entry.reference]));
    const inspect = (args, timeout, binary, maxBytes) => {
      const raw = run(args, timeout, binary, maxBytes);
      if (args[0] === "buildx") {
        const expected = Object.values(target.images).find((entry) => entry.reference === args.at(-1));
        if (expected && JSON.stringify(indexDescriptors(JSON.parse(raw))) !== JSON.stringify(expected.descriptors)) {
          throw new Error("anonymous registry descriptors disagree with the assembled inventory");
        }
      }
      return raw;
    };
    registries[registry] = verifyImages({ ...manifest, image_namespace: target.namespace, images }, platform, version, sourceSha, inspect, false, root);
  }
  return { ...registries.dockerhub, registries };
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  try {
    const [bundle, platform, version, sourceSha, report, inventoryPath, ...extra] =
      process.argv.slice(2);
    if (!report || extra.length)
      throw new Error(
        "usage: verify-release-images.mjs BUNDLE PLATFORM VERSION SOURCE_SHA REPORT [REGISTRY_INVENTORY]",
      );
    requireAnonymousConfig(process.env.DOCKER_CONFIG);
    if (
      readFileSync(join(bundle, "version"), "utf8").trim() !== version ||
      readFileSync(join(bundle, "source-sha"), "utf8").trim() !== sourceSha
    ) {
      throw new Error("archive identity does not match the workflow");
    }
    const manifest = readJson(join(bundle, "environment.json"));
    const result = inventoryPath
      ? verifyRegistrySet(manifest, readJson(inventoryPath), platform, version, sourceSha)
      : verifyImages(manifest, platform, version, sourceSha);
    writeFileSync(report, `${JSON.stringify(result, null, 2)}\n`);
    console.log(
      `Verified ${result.images.length} anonymous digest pulls per registry on ${platform}; report: ${report}`,
    );
  } catch (error) {
    console.error(`release-images: ${error.message}`);
    process.exitCode = 1;
  }
}
