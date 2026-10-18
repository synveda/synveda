#!/usr/bin/env node
// CPR-43 / ADR-0069: enforce the epoch-3 hard cut over active production
// surfaces. Historical ADRs and negative tests retain old vocabulary as
// evidence; they are deliberately outside this production-surface scanner.

import {
  existsSync,
  readFileSync,
  readdirSync,
  statSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { basename, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));
// OPS-6 / ADR-0121 and OPS-7 / ADR-0126: pin each released or reviewed
// additive migration independently.
// A checksum change is a release compatibility change, not routine SQL cleanup.
const EPOCH_3_BASELINE_SHA256 =
  "66766e8bc10e2eeea46c15350bfb7d7642fd7d360c1a0dfa64fce3ef4ab89de5";
const EPOCH_3_FORWARD_SHA256 =
  "08932e958a8a90ebd53859703ab3a93ab87adce22ec8f1efec003f11b10cd435";
const EPOCH_3_LOGIN_SHA256 =
  "313c002d6982eef961d9503135777c7f7acded91e51fc22aacd187e117de34db";
const EPOCH_3_DELIVERY_SHA256 =
  "6339e5518f08807ce7cf509e9c2e1644fb9536ebe17d2c0b4677062f41247631";

const EPOCH_3_SOURCE_SHA256 =
  "f170d4a295844c83ca5a33723586aaa0c68a10c88217afdea7f1f884672d7ab4";
const EPOCH_3_ARTIFACT_SHA256 =
  "28a9f6f3a510c50137f9c99a2b352b2065593690bd4d7fa03440db3693fef7f2";

const EPOCH_3_RECONCILE_SHA256 =
  "02e22d62c3ba2469f62a70b85a92fbf1f119663a2fdf5fe38d7f66cd62c7915d";

const EPOCH_3_GITHUB_SOURCE_SHA256 =
  "bf39fda96eefe6f9496e6078119354d8bb43955fc2fbe821325b70dabe36347f";

const EPOCH_3_TASK_STATE_SHA256 =
  "1979f8caf5f1d8bf86cef3534a8bc4beaf49d78b457fe1b377ffe94eb0eb4ef2";

const EPOCH_3_PROCEDURE_SHA256 = "61de10ae5ad1cad3a57f101691eb1d8cb353765a66d6fafd79043c41da0e7ab7";
const EPOCH_3_CONVERGENCE_SHA256 = "9901e1e9f655e980f07ac1977f74c12cd21f410a9e9037be5ad32926f3ade0b3";
const EPOCH_3_PORTABLE_SHA256 = "7de41c75f100d6d58ac31dd8a67110f1c00470364ac3b3cc130b74e705027cdc";
const PUBLISHED_MIGRATIONS = [
  "0001_context_platform.sql",
  "0002_context_restart_and_excerpt.sql",
  "0003_one_time_login_ledger.sql",
  "0004_context_delivery_dependencies.sql",
  "0005_session_source_control.sql",
  "0006_import_artifact_source_control.sql",
  "0007_inbound_git_source_reconciliation.sql",
  "0008_private_github_source_identity.sql",
  "0009_grounded_task_state_and_handoff.sql",
  "0010_verified_procedure_improvements.sql",
  "0011_portable_postgres_convergence.sql",
];
const PORTABLE_MIGRATIONS = ["0011_portable_context_platform.sql"];
// PostgreSQL 15+ schema syntax and catalogues that the PostgreSQL 14
// compatible lineage must not contain (ADR-0152).
const NEWER_POSTGRESQL_SYNTAX = [
  ["PostgreSQL 17 transaction_timeout", /\btransaction_timeout\b/iu],
  ["PostgreSQL 17 UUID extraction", /\buuid_extract_(?:version|timestamp)\b/iu],
  ["PostgreSQL 15 security_invoker view", /\bsecurity_invoker\b/iu],
  ["PostgreSQL 15 NULLS NOT DISTINCT", /\bNULLS\s+NOT\s+DISTINCT\b/iu],
  ["PostgreSQL 15 MERGE", /^\s*MERGE\s+INTO\b/imu],
  ["PostgreSQL 16 SQL/JSON constructor", /\b(?:JSON_OBJECT|JSON_ARRAY|JSON_TABLE|JSON_EXISTS|JSON_QUERY|JSON_VALUE)\s*\(/iu],
  ["PostgreSQL 15 parameter privilege", /\b(?:pg_parameter_acl|ON\s+PARAMETER)\b/iu],
  ["PostgreSQL 16 membership option", /\b(?:inherit_option|set_option)\b|WITH\s+(?:ADMIN|INHERIT|SET)\s+(?:TRUE|FALSE)/iu],
  ["pg_dump restrict meta-command", /^\\(?:un)?restrict\b/mu],
];

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
    else if (accept(relativePath(path))) output.push(path);
  }
  return output;
}

// Predicates and findings see repo-relative forward-slash paths on every host.
function relativePath(path) {
  return relative(ROOT, path).replaceAll("\\", "/");
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
  files.push(
    ...walk(join(ROOT, "sdks"), (path) =>
      /\.(?:rs|py|ts|mts)$/u.test(path) || /(?:Cargo\.toml|package\.json)$/u.test(path),
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
  // The Kubernetes RBAC kind is unrelated to Synveda's retired authority DTO.
  // Mask only its exact API/kind pair in deployment YAML, preserving offsets;
  // a legacy type elsewhere in that same file must still fail.
  const scanned = /^deploy\/.*\.ya?ml$/u.test(file)
    ? source.replace(/(^apiVersion: rbac\.authorization\.k8s\.io\/v1\r?\nkind: )RoleBinding(?=\r?\n)/gmu, (_match, prefix) => prefix + " ".repeat("RoleBinding".length))
    : source;
  for (const [label, pattern] of RETIRED_PATTERNS) {
    pattern.lastIndex = 0;
    for (const match of scanned.matchAll(pattern)) {
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
  if (
    /\b(?:CREATE|ALTER|DROP)\s+(?:ROLE|USER|EXTENSION)\b/iu.test(source)
  ) {
    findings.push("0001_context_platform.sql contains deployment-owned role or extension DDL");
  }
  return findings;
}

export function baselineDigestFindings(source) {
  const actual = createHash("sha256").update(source).digest("hex");
  return actual === EPOCH_3_BASELINE_SHA256
    ? []
    : [
        `0001_context_platform.sql SHA-256 changed (${actual}); OPS-6 / ADR-0121 requires an explicit migration and compatibility decision`,
      ];
}

export function forwardDigestFindings(source) {
  const actual = createHash("sha256").update(source).digest("hex");
  return actual === EPOCH_3_FORWARD_SHA256
    ? []
    : [
        `0002_context_restart_and_excerpt.sql SHA-256 changed (${actual}); OPS-6 / ADR-0121 requires an explicit migration and compatibility decision`,
      ];
}

export function loginDigestFindings(source) {
  const actual = createHash("sha256").update(source).digest("hex");
  return actual === EPOCH_3_LOGIN_SHA256
    ? []
    : [
        `0003_one_time_login_ledger.sql SHA-256 changed (${actual}); OPS-7 / ADR-0126 requires an explicit migration and compatibility decision`,
      ];
}

export function deliveryDigestFindings(source) {
  const actual = createHash("sha256").update(source).digest("hex");
  return actual === EPOCH_3_DELIVERY_SHA256 ? [] : [
    `0004_context_delivery_dependencies.sql SHA-256 changed (${actual}); MEM-8 / ADR-0144 requires an explicit migration and compatibility decision`,
  ];
}

export function sourceControlDigestFindings(source) {
  const actual = createHash("sha256").update(source).digest("hex");
  return actual === EPOCH_3_SOURCE_SHA256 ? [] : [
    `0005_session_source_control.sql SHA-256 changed (${actual}); MEM-8 / ADR-0146 requires an explicit migration and compatibility decision`,
  ];
}

export function artifactControlDigestFindings(source) {
  const actual = createHash("sha256").update(source).digest("hex");
  return actual === EPOCH_3_ARTIFACT_SHA256 ? [] : [
    `0006_import_artifact_source_control.sql SHA-256 changed (${actual}); MEM-8 / ADR-0147 requires an explicit migration and compatibility decision`,
  ];
}

export function reconcileDigestFindings(source) {
  const actual = createHash("sha256").update(source).digest("hex");
  return actual === EPOCH_3_RECONCILE_SHA256 ? [] : [
    `0007_inbound_git_source_reconciliation.sql SHA-256 changed (${actual}); MEM-9 / ADR-0148 requires an explicit migration and compatibility decision`,
  ];
}

export function githubSourceDigestFindings(source) {
  const actual = createHash("sha256").update(source).digest("hex");
  return actual === EPOCH_3_GITHUB_SOURCE_SHA256 ? [] : [
    `0008_private_github_source_identity.sql SHA-256 changed (${actual}); MEM-9 / ADR-0149 requires an explicit migration and compatibility decision`,
  ];
}

export function taskStateDigestFindings(source) {
  const actual = createHash("sha256").update(source).digest("hex");
  return actual === EPOCH_3_TASK_STATE_SHA256 ? [] : [
    `0009_grounded_task_state_and_handoff.sql SHA-256 changed (${actual}); CTX-6 / ADR-0150 requires an explicit migration and compatibility decision`,
  ];
}

export function procedureDigestFindings(source) {
  const actual = createHash("sha256").update(source).digest("hex");
  return actual === EPOCH_3_PROCEDURE_SHA256 ? [] : [
    `0010_verified_procedure_improvements.sql SHA-256 changed (${actual}); SKIL-6 / ADR-0151 requires an explicit migration and compatibility decision`,
  ];
}

export function convergenceDigestFindings(source) {
  const actual = createHash("sha256").update(source).digest("hex");
  return actual === EPOCH_3_CONVERGENCE_SHA256 ? [] : [
    `0011_portable_postgres_convergence.sql SHA-256 changed (${actual}); OPS-11 / ADR-0152 requires an explicit migration and compatibility decision`,
  ];
}

export function portableDigestFindings(source) {
  const actual = createHash("sha256").update(source).digest("hex");
  return actual === EPOCH_3_PORTABLE_SHA256 ? [] : [
    `portable-migrations/0011_portable_context_platform.sql SHA-256 changed (${actual}); OPS-11 / ADR-0152 requires an explicit migration and compatibility decision`,
  ];
}

export function portableSyntaxFindings(source, file) {
  const statements = source.replace(/^\s*--.*$/gmu, "");
  return NEWER_POSTGRESQL_SYNTAX.filter(([, pattern]) => pattern.test(statements)).map(
    ([label]) => `${file}: ${label} is outside the PostgreSQL 14 compatible lineage`,
  );
}

export function lineageInventoryFindings(published, portable) {
  const findings = [];
  if (
    JSON.stringify(published.slice(0, PUBLISHED_MIGRATIONS.length)) !==
    JSON.stringify(PUBLISHED_MIGRATIONS)
  ) {
    findings.push(
      `migration inventory is ${published.join(", ") || "empty"}; expected the released epoch-3 baseline and ten forward migrations`,
    );
  }
  const tail = published.slice(PUBLISHED_MIGRATIONS.length);
  if (JSON.stringify(portable) !== JSON.stringify([...PORTABLE_MIGRATIONS, ...tail])) {
    findings.push(
      `portable migration inventory is ${portable.join(", ") || "empty"}; expected the portable baseline followed by every later published migration`,
    );
  }
  return findings;
}

export function databaseBootstrapFindings(source) {
  const findings = [];
  const normalized = source.toLowerCase().replace(/\s+/gu, " ").trim();
  const exactRoles = [
    "synveda_app",
    "synveda_migrator",
    "synveda_gateway",
    "synveda_worker",
    "keycloak",
  ];
  const safeRoleSuffix =
    "nologin inherit nosuperuser nocreatedb nocreaterole noreplication nobypassrls connection limit -1";
  const createRoleCount = (normalized.match(/\bcreate role\b/gu) ?? []).length;
  if (
    createRoleCount !== exactRoles.length ||
    exactRoles.some(
      (role) =>
        normalized.split(`create role ${role} ${safeRoleSuffix}`).length - 1 !== 1,
    )
  ) {
    findings.push(
      "deployment database bootstrap must create only the five exact initially-NOLOGIN, non-elevated product roles",
    );
  }
  if (
    /\b(?:create|alter|drop)\s+user\b/gu.test(normalized) ||
    /\bdrop\s+role\b/gu.test(normalized) ||
    /\balter\s+role\b[^;']{0,300}\b(?:superuser|createdb|createrole|replication|bypassrls)\b/gu.test(
      normalized,
    )
  ) {
    findings.push("deployment database bootstrap contains unsafe or destructive role DDL");
  }
  if (/\b(?:alter|drop)\s+extension\b/gu.test(normalized)) {
    findings.push("deployment database bootstrap contains extension mutation outside creation");
  }
  const extensionLoop =
    "select format( 'create extension %i with schema public version %l', " +
    "required.name, required.version ) from (values ('btree_gin', '1.3'), " +
    "('vector', '0.8.6')) as required(name, version) where not exists ( " +
    "select 1 from pg_catalog.pg_extension extension where extension.extname = " +
    "required.name ) \\gexec";
  if (
    !normalized.includes(extensionLoop) ||
    (normalized.match(/create extension/gu) ?? []).length !== 1
  ) {
    findings.push(
      "deployment database bootstrap must converge exactly version-pinned btree_gin and vector through the bounded identifier-safe extension loop",
    );
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
    findings.push(...retiredProductionFindings(source, relativePath(path)));
    for (const match of source.matchAll(/migrations\/(\d{4}_[A-Za-z0-9_-]+\.sql)/gu)) {
      if (![...PUBLISHED_MIGRATIONS, ...PORTABLE_MIGRATIONS].includes(match[1])) {
        const line = source.slice(0, match.index).split("\n").length;
        findings.push(`${relativePath(path)}:${line}: references absent migration ${match[1]}`);
      }
    }
  }

  for (const path of RETIRED_PATHS) {
    if (existsSync(join(ROOT, path))) findings.push(`${path}: retired source path still exists`);
  }

  const migrationDir = join(ROOT, "crates/synveda-store/migrations");
  const migrations = readdirSync(migrationDir).filter((name) => name.endsWith(".sql")).sort();
  const portableDir = join(ROOT, "crates/synveda-store/portable-migrations");
  const portableMigrations = readdirSync(portableDir).filter((name) => name.endsWith(".sql")).sort();
  findings.push(...lineageInventoryFindings(migrations, portableMigrations));
  for (const name of migrations.slice(PUBLISHED_MIGRATIONS.length)) {
    const copy = join(portableDir, name);
    if (!existsSync(copy) || !readFileSync(join(migrationDir, name)).equals(readFileSync(copy))) {
      findings.push(`portable-migrations/${name} must be a byte-identical copy of the published forward migration`);
    }
  }
  for (const name of migrations.slice(PUBLISHED_MIGRATIONS.length - 1)) {
    findings.push(...portableSyntaxFindings(readFileSync(join(migrationDir, name), "utf8"), name));
  }
  const portable = readFileSync(join(portableDir, PORTABLE_MIGRATIONS[0]), "utf8");
  findings.push(...baselineFindings(portable));
  findings.push(...portableDigestFindings(portable));
  findings.push(...portableSyntaxFindings(portable, PORTABLE_MIGRATIONS[0]));
  const baseline = readFileSync(join(migrationDir, "0001_context_platform.sql"), "utf8");
  findings.push(...baselineFindings(baseline));
  findings.push(...baselineDigestFindings(baseline));
  const forward = readFileSync(join(migrationDir, "0002_context_restart_and_excerpt.sql"), "utf8");
  findings.push(...retiredProductionFindings(forward, "0002_context_restart_and_excerpt.sql").filter((finding) => finding.includes("retired table")));
  findings.push(...forwardDigestFindings(forward));
  const login = readFileSync(join(migrationDir, "0003_one_time_login_ledger.sql"), "utf8");
  findings.push(...retiredProductionFindings(login, "0003_one_time_login_ledger.sql").filter((finding) => finding.includes("retired table")));
  findings.push(...loginDigestFindings(login));
  const delivery = readFileSync(join(migrationDir, "0004_context_delivery_dependencies.sql"), "utf8");
  findings.push(...retiredProductionFindings(delivery, "0004_context_delivery_dependencies.sql").filter((finding) => finding.includes("retired table")));
  findings.push(...deliveryDigestFindings(delivery));
  const sourceControl = readFileSync(join(migrationDir, "0005_session_source_control.sql"), "utf8");
  findings.push(...sourceControlDigestFindings(sourceControl));
  findings.push(...retiredProductionFindings(sourceControl, "0005_session_source_control.sql").filter((finding) => finding.includes("retired table")));
  const artifactControl = readFileSync(join(migrationDir, "0006_import_artifact_source_control.sql"), "utf8");
  findings.push(...artifactControlDigestFindings(artifactControl));
  findings.push(...retiredProductionFindings(artifactControl, "0006_import_artifact_source_control.sql").filter((finding) => finding.includes("retired table")));
  const reconcile = readFileSync(join(migrationDir, "0007_inbound_git_source_reconciliation.sql"), "utf8");
  findings.push(...reconcileDigestFindings(reconcile));
  findings.push(...retiredProductionFindings(reconcile, "0007_inbound_git_source_reconciliation.sql").filter((finding) => finding.includes("retired table")));
  const githubSource = readFileSync(join(migrationDir, "0008_private_github_source_identity.sql"), "utf8");
  findings.push(...githubSourceDigestFindings(githubSource));
  findings.push(...retiredProductionFindings(githubSource, "0008_private_github_source_identity.sql").filter((finding) => finding.includes("retired table")));
  const taskState = readFileSync(join(migrationDir, "0009_grounded_task_state_and_handoff.sql"), "utf8");
  findings.push(...taskStateDigestFindings(taskState));
  findings.push(...retiredProductionFindings(taskState, "0009_grounded_task_state_and_handoff.sql").filter((finding) => finding.includes("retired table")));
  const procedure = readFileSync(join(migrationDir, "0010_verified_procedure_improvements.sql"), "utf8");
  findings.push(...procedureDigestFindings(procedure));
  findings.push(...retiredProductionFindings(procedure, "0010_verified_procedure_improvements.sql").filter((finding) => finding.includes("retired table")));
  const convergence = readFileSync(join(migrationDir, "0011_portable_postgres_convergence.sql"), "utf8");
  findings.push(...convergenceDigestFindings(convergence));
  findings.push(...retiredProductionFindings(convergence, "0011_portable_postgres_convergence.sql").filter((finding) => finding.includes("retired table")));
  const databaseBootstrap = readFileSync(
    join(ROOT, "deploy/compose/postgres/synveda-database-bootstrap"),
    "utf8",
  );
  findings.push(...databaseBootstrapFindings(databaseBootstrap));

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
      ...retiredProductionFindings(source, relativePath(path)).filter((finding) =>
        finding.includes("retired table"),
      ),
    );
  }

  if (findings.length > 0) fail(findings);
  console.log(
    `context hard cut holds: ${productionFiles().length} active files, immutable epoch-3 baseline, ten forward migrations and the portable baseline, ` +
      "deployment-owned vector + btree_gin bootstrap, current OpenAPI and SQLx metadata",
  );
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
