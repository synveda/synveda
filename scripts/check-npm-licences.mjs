#!/usr/bin/env node
// Enforces CLAUDE.md's licence rule on the npm side (CNSL-1, ADR-0056
// decision 8) — the gate `cargo deny` has given the Rust side since FND-3.
//
// It has not mattered until now: adapters/claude-code, adapters/mcp-server
// and sdks/typescript between them declare typescript and @types/node as
// devDependencies and no runtime dependency at all. The console is the first
// package in this repo with a real runtime dependency tree, so it is the
// first one where the rule has anything to enforce against.
//
// Two lists, because they answer different questions.
//
//   * SHIPPED — what reaches a deployment. Held to exactly deny.toml's
//     allowlist, with no exception mechanism at all. This is CLAUDE.md's
//     "core path" applied literally: a dependency whose bytes a customer
//     runs is governed by the product's licence policy, full stop.
//
//   * BUILD — what turns source into that bundle and never leaves CI or a
//     developer's laptop. A wider permissive set, plus narrow per-package
//     exceptions each carrying the reason it is there. Same discipline
//     deny.toml uses: widen one package at a time, never the default.
//
// Run by `make ci` beside check-crate-deps.mjs.
import { execFileSync } from "node:child_process";

/// deny.toml's `allow`, verbatim. Anything a deployment runs lives here.
const SHIPPED = ["MIT", "Apache-2.0", "PostgreSQL", "Unicode-3.0"];

/// Build tooling. The additions over SHIPPED are the three permissive,
/// OSI-approved licences that are ubiquitous in the npm toolchain and that
/// deny.toml already admits per-crate on the Rust side (BSD-3-Clause) or
/// would if a crate had needed them.
const BUILD = [...SHIPPED, "ISC", "0BSD", "BSD-2-Clause", "BSD-3-Clause"];

/// Narrow, annotated exceptions — build-time only, one package each.
const BUILD_EXCEPTIONS = {
  // Browser-support *data* (the browserslist tables), not code: CC-BY-4.0
  // is an attribution licence over a dataset, it is consumed at build time
  // to decide which transforms Babel applies, and none of it is emitted
  // into the bundle. Added with CNSL-1.
  "caniuse-lite": ["CC-BY-4.0"],
};

function licences(prodOnly) {
  const args = ["licenses", "list", "--json"];
  if (prodOnly) args.push("--prod");
  const raw = execFileSync("pnpm", args, {
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  });
  // A workspace with no dependencies of that kind prints nothing.
  const parsed = raw.trim() ? JSON.parse(raw) : {};
  const found = new Map();
  for (const [licence, packages] of Object.entries(parsed)) {
    for (const pkg of packages) {
      const existing = found.get(pkg.name) ?? new Set();
      existing.add(licence);
      found.set(pkg.name, existing);
    }
  }
  return found;
}

const shipped = licences(true);
const all = licences(false);

let failed = false;

for (const [name, found] of shipped) {
  for (const licence of found) {
    if (!SHIPPED.includes(licence)) {
      console.error(
        `FAIL: ${name} is ${licence} and reaches a deployment — the core path is ` +
          `${SHIPPED.join(" / ")} only (CLAUDE.md). There is no exception list for ` +
          `shipped dependencies: replace it, or move it to a devDependency if it ` +
          `is genuinely build-time.`,
      );
      failed = true;
    }
  }
}

for (const [name, found] of all) {
  if (shipped.has(name)) continue;
  for (const licence of found) {
    if (BUILD.includes(licence)) continue;
    if (BUILD_EXCEPTIONS[name]?.includes(licence)) continue;
    console.error(
      `FAIL: build dependency ${name} is ${licence}, which is not admitted. ` +
        `Add a narrow, annotated entry to BUILD_EXCEPTIONS in ` +
        `scripts/check-npm-licences.mjs — do not widen BUILD.`,
    );
    failed = true;
  }
}

// An exception nobody needs is an exception nobody re-reads. Same reasoning
// as the annotations in deny.toml: the list is only useful while every line
// on it is load-bearing.
for (const name of Object.keys(BUILD_EXCEPTIONS)) {
  if (!all.has(name)) {
    console.error(
      `FAIL: BUILD_EXCEPTIONS names ${name}, which is no longer a dependency — remove it.`,
    );
    failed = true;
  }
}

if (failed) process.exit(1);
console.log(
  `npm licences hold for ${all.size} packages ` +
    `(${shipped.size} shipped, ${all.size - shipped.size} build-only)`,
);
