#!/usr/bin/env node
// Feature-tracker IDs belong in docs/backlog and commit subjects only. Code,
// comments and documentation describe features in words.

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const STATUS = "docs/backlog/STATUS.md";

// Directories and files that may keep IDs, and why.
const SKIPPED = [
  /^docs\/backlog\//, // the tracker itself
  /(^|\/)fixtures\//, // captured, digest-pinned evidence
  /(^|\/)evidence\//, // dated release and qualification records
  /\.jsonl$/,
  /^evals\/.*\.json$/, // eval data and baselines tag the suite they measure
  /^crates\/synveda-store\/(migrations|portable-migrations)\//, // published, checksum-pinned SQL
  /^\.sqlx\//,
];

// Byte-pinned files whose remaining IDs cannot change without re-pinning.
const PINNED = new Map([
  ["crates/synveda-store/sql/extension_fingerprint.sql", 1], // SQLx query hash
  ["crates/synveda-store/src/sessions.rs", 1], // SQL comment inside a SQLx-checked query
  ["scripts/db-test.sh", 8], // digest-pinned by check-deploy-convergence
  ["scripts/product-evaluation.mjs", 2], // compares the eval suite's feature tag
  ["scripts/run-git-export-canary.mjs", 1], // writes the canary evidence's feature tag
  ["deploy/compose/keycloak/Dockerfile", 1], // Keycloak security-chain digests
  ["deploy/compose/keycloak/SynvedaKeycloakProjection.java", 1],
  ["deploy/compose/keycloak/keycloak-entrypoint", 1],
  ["deploy/compose/keycloak/synveda-generation-gate", 1],
  ["deploy/compose/keycloak/synveda-realm-converge", 1],
]);

export function featureIds(status) {
  return new Set([...status.matchAll(/^- \[[ x]\] \[?([A-Z]+-\d+):/gm)].map((m) => m[1]));
}

/** Lines naming a known feature ID, ignoring links to tracker files. */
export function findings(text, ids) {
  const found = [];
  text.split(/\r?\n/).forEach((line, index) => {
    for (const match of line.matchAll(/\b([A-Z]+-\d+)\b/g)) {
      const before = line.slice(0, match.index);
      const after = line.slice(match.index + match[1].length);
      if (!ids.has(match[1]) || (before.endsWith("backlog/") && after.startsWith(".md"))) continue;
      found.push({ line: index + 1, id: match[1] });
    }
  });
  return found;
}

export function check(root = ".") {
  const ids = featureIds(readFileSync(`${root}/${STATUS}`, "utf8"));
  const files = execFileSync("git", ["-C", root, "ls-files", "-z"], { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 })
    .split("\0")
    .filter((path) => path && !SKIPPED.some((pattern) => pattern.test(path)));
  const problems = [];
  for (const path of files) {
    let text;
    try {
      text = readFileSync(`${root}/${path}`, "utf8");
    } catch {
      continue;
    }
    const hits = findings(text, ids);
    const allowed = PINNED.get(path) ?? 0;
    if (hits.length > allowed) {
      problems.push(...hits.map((hit) => `${path}:${hit.line}: ${hit.id}`));
    }
  }
  return problems;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const problems = check();
  if (problems.length) {
    console.error("Feature IDs belong in docs/backlog and commit subjects only. Describe the feature in words:");
    for (const problem of problems) console.error(`  ${problem}`);
    process.exitCode = 1;
  } else {
    console.log("No feature IDs outside the tracker.");
  }
}
