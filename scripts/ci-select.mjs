#!/usr/bin/env node
// FND-1: unknown inputs select the full pipeline. No GitHub path-count limits.
import { execFileSync } from "node:child_process";
import { appendFileSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

export const stages = [
  "rust",
  "typescript",
  "sdk",
  "replay",
  "eval",
  "demo",
  "deploy",
  "docker",
  "helm",
  "cli",
];
const all = () => Object.fromEntries(stages.map((stage) => [stage, true]));

export function selectChanges(paths, full = false) {
  if (full || !paths?.length) return all();
  const selected = Object.fromEntries(stages.map((stage) => [stage, false]));
  for (const path of paths) {
    // These files are consumed by the always-on documentation/static checks.
    if (
      /^docs\/(?:adr|backlog)\/[^/]+\.md$/.test(path) ||
      [
        "AGENTS.md",
        "CONTRIBUTING.md",
        "docs/DEVELOPMENT.md",
        "docs/CI.md",
        "docs/PRODUCTION_READINESS.md",
      ].includes(path)
    )
      continue;
    if (path.startsWith("website/")) {
      selected.typescript = true;
      continue;
    }
    // Console assets also ship in Docker; Rust tests bind the generated API.
    if (path.startsWith("console/") || path.startsWith("assets/")) {
      for (const stage of ["rust", "typescript", "deploy", "docker", "helm"])
        selected[stage] = true;
      continue;
    }
    // Rust, policies, SQLx, adapters, SDKs, fixtures, packaging, manifests,
    // lockfiles and workflow/tooling edits can affect every downstream stage.
    return all();
  }
  return selected;
}

export function selectForEvent(paths, eventName) {
  const selected = selectChanges(paths, eventName !== "pull_request");
  // Native Docker/Compose qualification is required on main, merge queues and
  // release runs. PRs retain static deployment checks and live Helm tests.
  if (eventName === "pull_request") selected.docker = false;
  return selected;
}

export function changedPaths(base, head, run = execFileSync) {
  if (![base, head].every((sha) => /^[a-f0-9]{40}$/.test(sha ?? "")))
    throw new Error("missing exact PR commits");
  const mergeBase = run("git", ["merge-base", base, head], {
    encoding: "utf8",
  }).trim();
  // --no-renames includes both sides of renames; NUL handles arbitrary names.
  return run(
    "git",
    ["diff", "--name-only", "--no-renames", "-z", mergeBase, head],
    { encoding: "utf8" },
  )
    .split("\0")
    .filter(Boolean);
}

if (process.argv[1] && resolve(process.argv[1]) === import.meta.filename) {
  let paths;
  const full = process.env.GITHUB_EVENT_NAME !== "pull_request";
  if (!full) {
    try {
      paths = changedPaths(process.env.BASE_SHA, process.env.HEAD_SHA);
    } catch {
      console.log("Change comparison unavailable; selecting every PR-eligible stage.");
    }
  }
  const selected = selectForEvent(paths, process.env.GITHUB_EVENT_NAME);
  console.log(JSON.stringify({ paths: paths ?? "all", selected }, null, 2));
  if (process.env.GITHUB_OUTPUT)
    appendFileSync(
      process.env.GITHUB_OUTPUT,
      Object.entries(selected)
        .map(([key, value]) => `${key}=${value}\n`)
        .join("") +
        `version=${readFileSync("Cargo.toml", "utf8").match(/^version = "([^"]+)"/m)[1]}\n`,
    );
}
