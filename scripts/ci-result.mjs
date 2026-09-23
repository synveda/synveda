#!/usr/bin/env node
// FND-1: this is the only branch-protection check. Missing evidence fails closed.
import { resolve } from "node:path";
import { stages } from "./ci-select.mjs";

export const requiredJobs = {
  "build-test": "rust",
  typescript: "typescript",
  sdk: "sdk",
  replay: "replay",
  eval: "eval",
  demo: "demo",
  deploy: "deploy",
  docker: "docker",
  helm: "helm",
  "helm-operations": "helm",
  cli: "cli",
};

export function ciResult(needs) {
  const errors = [];
  const expected = [
    "changes",
    "check",
    "licences",
    ...Object.keys(requiredJobs),
  ].sort();
  if (JSON.stringify(Object.keys(needs).sort()) !== JSON.stringify(expected))
    errors.push("CI Result needs list differs from the required job inventory");
  for (const job of ["changes", "check", "licences"]) {
    if (needs[job]?.result !== "success")
      errors.push(
        `${job}: expected success, got ${needs[job]?.result ?? "missing"}`,
      );
  }
  const selected = needs.changes?.outputs ?? {};
  for (const stage of stages)
    if (!["true", "false"].includes(selected[stage]))
      errors.push(`invalid selection: ${stage}`);
  for (const [job, stage] of Object.entries(requiredJobs)) {
    const result = needs[job]?.result;
    if (result === "success") continue;
    if (result === "skipped" && selected[stage] === "false") continue;
    errors.push(
      `${job}: required=${selected[stage] ?? "unknown"}, result=${result ?? "missing"}`,
    );
  }
  return errors;
}

if (process.argv[1] && resolve(process.argv[1]) === import.meta.filename) {
  try {
    const errors = ciResult(JSON.parse(process.env.CI_NEEDS ?? "{}"));
    if (errors.length) throw new Error(errors.join("\n"));
    console.log(
      "CI Result: all selected checks passed; every skip was expected.",
    );
  } catch (error) {
    console.error(`CI Result failed: ${error.message}`);
    process.exitCode = 1;
  }
}
