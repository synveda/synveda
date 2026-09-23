#!/usr/bin/env node
// FND-1 / OPS-8: import exact OCI bytes into a loopback-only test registry.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { releaseImages } from "./release-registries.mjs";
import { verifyImages } from "./verify-release-images.mjs";

export async function fileHash(path) {
  const hash = createHash("sha256");
  for await (const bytes of createReadStream(path)) hash.update(bytes);
  return hash.digest("hex");
}
export function candidateIdentity(candidate, version, source, arch) {
  assert.ok(["amd64", "arm64"].includes(arch));
  assert.equal(candidate.version, version);
  assert.equal(candidate.source, source);
  assert.equal(candidate.arch, arch);
  assert.equal(
    candidate.source_dirty,
    false,
    "OCI candidates require a clean source checkout",
  );
  assert.match(source, /^[a-f0-9]{40}$/);
  assert.deepEqual(
    Object.keys(candidate.images).sort(),
    Object.keys(releaseImages).sort(),
  );
  for (const [name, entry] of Object.entries(candidate.images)) {
    assert.equal(entry.archive, `${releaseImages[name]}.tar`);
    assert.match(entry.sha256, /^[a-f0-9]{64}$/);
    assert.match(entry.digest, /^sha256:[a-f0-9]{64}$/);
  }
}
const run = (command, args, extra = {}) =>
  execFileSync(command, args, {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
    timeout: 600000,
    ...extra,
  });

if (process.argv[1] && resolve(process.argv[1]) === import.meta.filename) {
  const [directory, version, source, arch] = process.argv.slice(2);
  assert.ok(
    directory && version && source && ["amd64", "arm64"].includes(arch),
    "usage: docker-candidate.mjs DIRECTORY VERSION SHA ARCH",
  );
  run("sh", ["scripts/release-version.sh", version]);
  assert.equal(
    run("git", ["rev-parse", "HEAD"]).trim(),
    source,
    "OCI candidates must identify their exact source",
  );
  const sourceDirty =
    run("git", ["status", "--porcelain", "--untracked-files=normal"]).trim() !==
    "";
  assert.equal(
    sourceDirty,
    false,
    "OCI candidates require a clean source checkout",
  );
  const candidate = {
    version,
    source,
    source_dirty: sourceDirty,
    arch,
    images: {},
  };
  for (const [name, repository] of Object.entries(releaseImages)) {
    const archive = `${repository}.tar`;
    const local = `localhost:5000/synveda/${repository}:${version}-${arch}`;
    run("skopeo", [
      "copy",
      "--all",
      "--preserve-digests",
      "--dest-tls-verify=false",
      `oci-archive:${join(directory, archive)}`,
      `docker://${local}`,
    ]);
    const raw = run("skopeo", [
      "inspect",
      "--raw",
      "--tls-verify=false",
      `docker://${local}`,
    ]);
    candidate.images[name] = {
      archive,
      sha256: await fileHash(join(directory, archive)),
      digest: `sha256:${createHash("sha256").update(raw).digest("hex")}`,
    };
  }
  candidateIdentity(candidate, version, source, arch);
  const out = join(directory, "bundle");
  run(
    "bash",
    [
      "scripts/package-release.sh",
      version,
      out,
      source,
      ...Object.keys(releaseImages).map(
        (name) => candidate.images[name].digest,
      ),
    ],
    {
      stdio: "inherit",
      env: {
        ...process.env,
        SYNVEDA_IMAGE_NAMESPACE: "localhost:5000/synveda",
        SYNVEDA_LOCAL_CANDIDATE: "1",
        SYNVEDA_PACKAGE_CONSUMER_CANDIDATE: "1",
      },
    },
  );
  run("sh", ["scripts/package-chart.sh", version, out], { stdio: "inherit" });
  run("tar", [
    "-xzf",
    join(out, `synveda-reference-${version}.tar.gz`),
    "-C",
    out,
  ]);
  const bundle = join(out, `synveda-reference-${version}`);
  const manifest = JSON.parse(readFileSync(join(bundle, "environment.json")));
  const report = verifyImages(
    manifest,
    `linux/${arch}`,
    version,
    source,
    undefined,
    true,
  );
  writeFileSync(
    join(directory, "image-checks.json"),
    `${JSON.stringify(report, null, 2)}\n`,
  );
  writeFileSync(
    join(directory, "candidate.json"),
    `${JSON.stringify(candidate, null, 2)}\n`,
    { flag: "wx" },
  );
}
