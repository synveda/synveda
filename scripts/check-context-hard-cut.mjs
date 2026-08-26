#!/usr/bin/env node
// CPR-43 / ADR-0069: enforce the epoch-3 hard cut over active production
// surfaces. Historical ledgers and negative tests retain old vocabulary as
// evidence; they are deliberately outside this scanner and classified in
// docs/implementation/context-hard-cut-inventory.md.

import {
  existsSync,
  readFileSync,
  readdirSync,
  statSync,
} from "node:fs";
import { basename, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));

const RETIRED_PATTERNS = [
  ["global runtime route", /\/v1\/(?:observe|inject|recall)\b/giu],
  ["retired aggregate or adapter DTO", /\b(?:RecordKind|RoleBinding|RecallSweepRequest|RecallIdsRequest|SearchIndex)\b/gu],
  ["compatibility deserialisation alias", /serde\s*\([^\n)]*\balias\b/giu],
  ["hidden CLI alias", /command\s*\([^\n)]*\b(?:alias|aliases|visible_alias)\b/giu],
  ["retired table", /\b(?:record_versions|record_embeddings|observe_events|observe_quarantine|hierarchy_nodes|role_bindings|policy_lapses)\b/giu],
  ["retired runtime dependency", /\b(?:tantivy|pgmq)\b/giu],
  ["retired search configuration", /\bSYNVEDA_SEARCH_INDEX_DIR\b/gu],
  ["retired telemetry/configuration name", /\b(?:inject_embed_timeout|TOKENS_PER_INJECT|INDEX_TIER_TOKENS)\b/gu],
  ["retired fixed-rank kind", /\b(?:Division|Department)\b/gu],
];

const RETIRED_PATHS = [
  "crates/synveda-types/src/record.rs",
  "crates/synveda-store/src/records.rs",
  "crates/synveda-store/src/search.rs",
  "crates/synveda-retrieval/src/index.rs",
  "crates/synveda-retrieval/src/indexer.rs",
  "crates/synveda-retrieval/src/hybrid.rs",
];

function walk(directory, accept, output = []) {
  if (!existsSync(directory)) return output;
  for (const entry of readdirSync(directory)) {
    const path = join(directory, entry);
    const stats = statSync(path);
    if (stats.isDirectory()) walk(path, accept, output);
    else if (accept(path)) output.push(path);
  }
  return output;
}

function productionFiles() {
  const files = [];
  files.push(
    ...walk(join(ROOT, "crates"), (path) =>
      path.endsWith("Cargo.toml") || (/\/src\//u.test(path) && path.endsWith(".rs")),
    ),
  );
  files.push(
    ...walk(join(ROOT, "adapters"), (path) =>
      /\/src\/.*\.(?:ts|mts)$/u.test(path) && !/\.test\.(?:ts|mts)$/u.test(path),
    ),
  );
  files.push(...walk(join(ROOT, "console", "src"), (path) => /\.(?:ts|mts|tsx)$/u.test(path)));
  files.push(
    ...walk(join(ROOT, "deploy"), (path) =>
      /(?:Dockerfile|\.(?:ya?ml|toml|json|sh))$/u.test(path),
    ),
  );
  files.push(
    ...walk(join(ROOT, "scripts"), (path) => {
      const name = basename(path);
      return (
        /\.(?:sh|mjs)$/u.test(name) &&
        !name.startsWith("check-") &&
        !name.endsWith(".test.mjs")
      );
    }),
  );
  for (const path of ["Cargo.toml", "Cargo.lock"]) files.push(join(ROOT, path));
  return [...new Set(files)].sort();
}

export function retiredProductionFindings(source, file = "fixture") {
  const findings = [];
  for (const [label, pattern] of RETIRED_PATTERNS) {
    pattern.lastIndex = 0;
    for (const match of source.matchAll(pattern)) {
      const line = source.slice(0, match.index).split("\n").length;
      findings.push(`${file}:${line}: ${label}: ${match[0]}`);
    }
  }
  return findings;
}

export function baselineFindings(source) {
  const findings = [];
  const legacyTables = retiredProductionFindings(source, "0001_context_platform.sql")
    .filter((finding) => finding.includes("retired table"));
  findings.push(...legacyTables);
  if (/\bCREATE\s+(?:TABLE|VIEW)\s+(?:IF NOT EXISTS\s+)?(?:public\.)?records\b/iu.test(source)) {
    findings.push("0001_context_platform.sql: retired table: records");
  }
  const extensions = [...source.matchAll(/CREATE EXTENSION IF NOT EXISTS\s+([a-z0-9_]+)/giu)]
    .map((match) => match[1].toLowerCase())
    .sort();
  if (JSON.stringify(extensions) !== JSON.stringify(["btree_gin", "vector"])) {
    findings.push(`baseline extensions are ${extensions.join(", ") || "none"}, expected btree_gin, vector`);
  }
  return findings;
}

function fail(findings) {
  throw new Error(`context hard cut failed:\n${findings.map((item) => `- ${item}`).join("\n")}`);
}

export function main() {
  const findings = [];
  for (const path of productionFiles()) {
    const source = readFileSync(path, "utf8");
    findings.push(...retiredProductionFindings(source, relative(ROOT, path)));
  }

  for (const path of RETIRED_PATHS) {
    if (existsSync(join(ROOT, path))) findings.push(`${path}: retired source path still exists`);
  }

  const migrationDir = join(ROOT, "crates/synveda-store/migrations");
  const migrations = readdirSync(migrationDir).filter((name) => name.endsWith(".sql")).sort();
  if (JSON.stringify(migrations) !== JSON.stringify(["0001_context_platform.sql"])) {
    findings.push(`migration inventory is ${migrations.join(", ") || "empty"}; expected one epoch-3 baseline`);
  }
  const baseline = readFileSync(join(migrationDir, "0001_context_platform.sql"), "utf8");
  findings.push(...baselineFindings(baseline));

  const epoch = readFileSync(join(ROOT, "crates/synveda-store/src/epoch.rs"), "utf8");
  if (!/pub const CURRENT_EPOCH: i32 = 3;/u.test(epoch)) {
    findings.push("crates/synveda-store/src/epoch.rs does not declare CURRENT_EPOCH = 3");
  }

  const openapi = JSON.parse(readFileSync(join(ROOT, "docs/api/openapi.json"), "utf8"));
  for (const path of ["/v1/observe", "/v1/inject", "/v1/recall"]) {
    if (openapi.paths?.[path] !== undefined) findings.push(`OpenAPI publishes ${path}`);
  }

  for (const path of walk(join(ROOT, ".sqlx"), (candidate) => candidate.endsWith(".json"))) {
    const source = readFileSync(path, "utf8");
    findings.push(
      ...retiredProductionFindings(source, relative(ROOT, path)).filter((finding) =>
        finding.includes("retired table"),
      ),
    );
  }

  if (findings.length > 0) fail(findings);
  console.log(
    `context hard cut holds: ${productionFiles().length} active files, one epoch-3 migration, ` +
      "vector + btree_gin extension shape, current OpenAPI and SQLx metadata",
  );
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
