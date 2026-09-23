#!/usr/bin/env node
// OPS-11/FND-1: the external fixture may reuse only this job's source builds.
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const tags = [
  "synveda/product:ops11",
  "synveda/postgres:ops11",
  "synveda/keycloak:ops11",
];

export function verifyStarterImageReuse(report, imageId) {
  if (!report?.cases?.some((entry) => entry.selection === "external-external"))
    throw new Error("completed external-external starter evidence is missing");
  if (!Array.isArray(report.images))
    throw new Error("starter image evidence is missing");
  for (const tag of tags) {
    const matches = report.images.filter((entry) => entry.tag === tag);
    if (matches.length !== 1 || !/^sha256:[a-f0-9]{64}$/.test(matches[0].localId))
      throw new Error(`starter image evidence is invalid for ${tag}`);
    if (imageId(tag) !== matches[0].localId)
      throw new Error(`starter image changed before external acceptance: ${tag}`);
  }
  return tags;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const report = JSON.parse(readFileSync("demos/evidence/ops11-operations-external-external.json", "utf8"));
  verifyStarterImageReuse(report, (tag) => execFileSync("docker",
    ["image", "inspect", "--format", "{{.Id}}", tag],
    { encoding: "utf8", timeout: 30_000 }).trim());
  console.log("source-built starter images match the external fixture");
}
