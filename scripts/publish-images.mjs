#!/usr/bin/env node
// OPS-8: copy this run's qualified OCI bytes; never rebuild in a publishing job.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { candidateIdentity, fileHash } from "./docker-candidate.mjs";
import {
  preflight,
  publisherNamespace,
  releaseImages,
} from "./release-registries.mjs";

export function copyPlan(candidate, namespace) {
  const result = [];
  for (const prefix of [
    `docker.io/${publisherNamespace(namespace, true)}`,
    "ghcr.io/synveda",
  ]) {
    for (const [name, repository] of Object.entries(releaseImages)) {
      const version =
        name === "helm_postgres"
          ? `17.11-synveda-${candidate.version}`
          : candidate.version;
      result.push({
        archive: candidate.images[name].archive,
        reference: `${prefix}/${repository}:${version}-${candidate.arch}`,
      });
    }
  }
  return result;
}
if (process.argv[1] && resolve(process.argv[1]) === import.meta.filename) {
  const [directory, version, source, arch] = process.argv.slice(2);
  const candidate = JSON.parse(readFileSync(join(directory, "candidate.json")));
  candidateIdentity(candidate, version, source, arch);
  // Validate every input before the first remote write.
  for (const entry of Object.values(candidate.images))
    assert.equal(
      await fileHash(join(directory, entry.archive)),
      entry.sha256,
      "OCI archive changed after qualification",
    );
  const namespace = process.env.DOCKERHUB_NAMESPACE;
  preflight(version, source, namespace, arch);
  for (const entry of copyPlan(candidate, namespace)) {
    execFileSync(
      "skopeo",
      [
        "copy",
        "--all",
        "--preserve-digests",
        `oci-archive:${join(directory, entry.archive)}`,
        `docker://${entry.reference}`,
      ],
      { stdio: "inherit", timeout: 600000 },
    );
  }
}
