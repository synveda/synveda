#!/usr/bin/env node
// CPR-36 / ADR-0095: one application runtime across host, Compose and Helm.
// This check is intentionally database- and daemon-free. It renders the two
// Compose shapes and Helm, inspects the generated public contract, and builds
// the release profile twice so an upgrade-shaped stale file cannot survive.

import { execFileSync } from "node:child_process";
import {
  existsSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));

export const RETIRED_RUNTIME_MARKERS = [
  "/v1/observe",
  "/v1/inject",
  "/v1/recall",
  "record_embeddings",
  "hierarchy_nodes",
  "role_bindings",
  "policy_lapses",
  "demo/seed.sh",
  "organisation.txt",
];

export function serviceBlock(source, service) {
  const start = source.indexOf(`\n  ${service}:\n`);
  if (start < 0) return "";
  const bodyStart = start + `\n  ${service}:\n`.length;
  const rest = source.slice(bodyStart);
  const next = rest.search(/^  [a-zA-Z0-9_-]+:\s*$/m);
  return next < 0 ? rest : rest.slice(0, next);
}

export function retiredFindings(source) {
  const active = source
    .split("\n")
    .filter((line) => !/^\s*#/.test(line))
    .join("\n");
  return RETIRED_RUNTIME_MARKERS.filter((marker) => active.includes(marker));
}

export function hasRetiredDemoField(source) {
  return /^\s*demo:\s*bool,/m.test(source);
}

export function initCutoverFindings(source) {
  const entrypoint = source.match(
    /pub async fn init\([^)]*\)\s*->\s*Result<\(\), String>\s*\{\s*([\s\S]*?)\s*\}\s*#\[allow\(dead_code\)\]\s*async fn init_after_cutover/,
  );
  if (!entrypoint) return ["init cutover entrypoint has no isolated dormant implementation"];
  if (entrypoint[1].replace(/\s+/g, " ").trim() !== "reference_cutover_gate()") {
    return ["public init entrypoint is not a gate-only cutover refusal"];
  }
  return [];
}

export function releaseNoteFindings(source) {
  const block = source.match(/cat > notes\.md <<NOTES\r?\n([\s\S]*?)^[ \t]*NOTES[ \t]*$/mu);
  if (!block) return ["release-note block is missing"];
  const notes = block[1];
  const findings = [];
  if (notes.includes("synveda init --demo")) {
    findings.push("retired synveda init --demo command");
  }
  for (const command of ["synveda init", "synveda login", "synveda demo start"]) {
    if (notes.includes(command)) {
      findings.push(`unaccepted turnkey command ${command}`);
    }
  }
  if (!notes.includes("Docker reference deployment acceptance is pending")) {
    findings.push("Docker reference acceptance notice is missing");
  }
  return findings;
}

export function releasePostgresBuildFindings(source) {
  const marker = "      - name: Postgres\n";
  const start = source.indexOf(marker);
  if (start < 0) return ["release PostgreSQL build step is missing"];
  const next = source.indexOf("\n      - name:", start + marker.length);
  const block = source.slice(start, next < 0 ? source.length : next);
  const findings = [];
  if (!/^\s+context: \.\s*$/m.test(block)) {
    findings.push("release PostgreSQL build context is not the repository root");
  }
  if (!/^\s+file: deploy\/compose\/postgres\/Dockerfile\s*$/m.test(block)) {
    findings.push("release PostgreSQL Dockerfile is not explicit");
  }
  if (!/^\s+target: reference\s*$/m.test(block)) {
    findings.push("release PostgreSQL build does not select the reference target");
  }
  return findings;
}

export function contributorPostgresBuildFindings(source) {
  const postgres = serviceBlock(`\n${source}`, "postgres");
  if (!postgres) return ["contributor PostgreSQL service is missing"];
  const expected =
    "    build:\n" +
    "      context: ../..\n" +
    "      dockerfile: deploy/compose/postgres/Dockerfile\n" +
    "      target: development\n";
  return postgres.includes(expected)
    ? []
    : ["contributor PostgreSQL build does not select the repo-root development target"];
}

export function postgresImageTargetFindings(source) {
  const runtime = source.indexOf(" AS runtime\n");
  const development = source.indexOf("FROM runtime AS development\n");
  const reference = source.indexOf("FROM runtime AS reference\n");
  const copy =
    "COPY --chmod=0444 deploy/compose/postgres/development-initdb.sql " +
    "/docker-entrypoint-initdb.d/01-synveda-extensions.sql";
  const findings = [];
  if (!(runtime >= 0 && development > runtime && reference > development)) {
    findings.push("PostgreSQL runtime/development/reference stages are not closed and ordered");
    return findings;
  }
  const runtimeBlock = source.slice(runtime, development);
  const developmentBlock = source.slice(development, reference);
  const referenceBlock = source.slice(reference);
  if (!developmentBlock.includes(copy) || source.split(copy).length !== 2) {
    findings.push("development extension init is not isolated to one development stage");
  }
  if (/docker-entrypoint-initdb\.d|development-initdb\.sql/.test(runtimeBlock + referenceBlock)) {
    findings.push("reference PostgreSQL image inherits development initdb SQL");
  }
  if (!/^FROM runtime AS reference\s*$/m.test(referenceBlock)) {
    findings.push("reference PostgreSQL image is not the default final stage");
  }
  return findings;
}

export function developmentInitdbFindings(source) {
  const active = source
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("--"));
  const expected = [
    "create extension if not exists vector;",
    "create extension if not exists btree_gin;",
  ];
  return JSON.stringify(active) === JSON.stringify(expected)
    ? []
    : ["development initdb is not the exact two-extension prerequisite"];
}

export function shellFunctionOrderFindings(source, names) {
  const lines = source.split("\n");
  const findings = [];
  for (const name of names) {
    const definition = lines.findIndex((line) => line.trim() === `${name}() {`);
    const callPattern = new RegExp(`^\\s*${name}\\s+`);
    const call = lines.findIndex((line, index) => index !== definition && callPattern.test(line));
    if (definition < 0) findings.push(`${name} definition is missing`);
    else if (call >= 0 && call < definition) findings.push(`${name} is called before definition`);
  }
  return findings;
}

export function evalFixtureFindings(dbTest, evalLib, ciWorkflow, evalWorkflow) {
  const findings = [];
  const fastStart = dbTest.indexOf("# Demos and evaluations need the exact");
  const hostileStart = dbTest.indexOf("enable_hostile_database_logging() {");
  const fast =
    fastStart >= 0 && hostileStart > fastStart
      ? dbTest.slice(fastStart, hostileStart)
      : "";
  if (!/product-evaluation\|evaluation\|longmemeval-evaluation\) fast_fixture=true/.test(dbTest)) {
    findings.push("evaluation tasks do not select the fast exact-role fixture");
  }
  if (
    !dbTest.includes(
      'if [ "$fast_fixture" = true ]; then\n  compose up --detach --wait postgres-main\nelse\n  compose up --detach --wait postgres-main postgres-lifecycle\nfi',
    )
  ) {
    findings.push("fast fixture does not start only the main PostgreSQL cluster");
  }
  const firstSynveda = fast.indexOf("compose run --rm --no-deps database-bootstrap-main");
  const keycloak = fast.indexOf("compose run --rm --no-deps keycloak-database-bootstrap-main");
  const secondSynveda = fast.indexOf(
    "compose run --rm --no-deps database-bootstrap-main",
    firstSynveda + 1,
  );
  if (!(firstSynveda >= 0 && keycloak > firstSynveda && secondSynveda > keycloak)) {
    findings.push("fast fixture bootstrap order is not Synveda-Keycloak-Synveda");
  }
  const preflight = fast.indexOf('run_main_database_preflight "$main_witness_file"');
  const migrate = fast.indexOf("cargo run -q -p synveda-cli --bin synveda -- db migrate");
  const dispatch = fast.indexOf('case "$db_test_task" in');
  if (!(preflight >= 0 && migrate > preflight && dispatch > migrate)) {
    findings.push("fast fixture does not preflight and migrate before dispatch");
  }
  if (!fast.includes("for _ in 1 2; do")) {
    findings.push("fast fixture does not prove idempotent migration");
  }
  if (fast.includes("main_owner_file") || fast.includes("postgres-lifecycle")) {
    findings.push("fast fixture exposes owner/lifecycle authority");
  }
  if (!fast.includes("env -u SYNVEDA_DB_TEST_SECRETS_DIR")) {
    findings.push("fast fixture child processes inherit the all-secret directory");
  }
  for (const task of ["evaluation", "longmemeval-evaluation", "product-evaluation"]) {
    if (!fast.includes(`${task})`)) findings.push(`fast fixture omits ${task} dispatch`);
  }
  if (/name:\s*Start Postgres|up[^\n]*\bpostgres\b/.test(ciWorkflow)) {
    findings.push("CI evaluation still starts the legacy PostgreSQL service");
  }
  if (/name:\s*Start Postgres|up[^\n]*\bpostgres\b/.test(evalWorkflow)) {
    findings.push("nightly evaluation still starts the legacy PostgreSQL service");
  }
  const activeCurlLines = evalLib
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("#") && /\bcurl\b/.test(line));
  if (
    activeCurlLines.length !== 1 ||
    activeCurlLines[0] !==
      'command curl --disable --noproxy \'*\' --connect-timeout 2 --max-time 30 "$@"'
  ) {
    findings.push("evaluation HTTP is not one ambient-free bounded curl boundary");
  }
  if (
    !evalLib.includes("eval_wait_ready() { # label URL PID log-file") ||
    evalLib.split('eval_process_live "$eval_ready_pid"').length - 1 !== 3 ||
    !evalLib.includes('eval_curl_probe -fsS "$eval_ready_url/readyz"') ||
    !evalLib.includes(
      'eval_wait_gateway "$EVAL_GATEWAY_URL" "$EVAL_PID" "$EVAL_STATE/gateway.log"',
    ) ||
    !evalLib.includes(
      'eval_wait_worker "$EVAL_WORKER_URL" "$EVAL_WORKER_PID" "$EVAL_STATE/worker.log"',
    )
  ) {
    findings.push("evaluation readiness is not attested to each launched child");
  }
  if (!evalLib.includes('sock.bind(("127.0.0.1", int(sys.argv[1])))')) {
    findings.push("evaluation startup does not prove loopback bind availability");
  }
  if (
    !evalLib.includes("EVAL_STATE_OWNED=0") ||
    !evalLib.includes('rm -R -- "$eval_owned_state"') ||
    evalLib.includes('rm -rf "$EVAL_STATE"')
  ) {
    findings.push("evaluation scratch cleanup is not ownership-fenced");
  }
  if (!evalLib.includes("eval_loopback_port() {") || !evalLib.includes("NO_PROXY=127.0.0.1,localhost")) {
    findings.push("evaluation bearer traffic is not pinned to proxy-free loopback");
  }
  if (
    !evalLib.includes('eval_stop_pids_with_grace 120 "$@"') ||
    !evalLib.includes('kill -KILL "$eval_stop_target"') ||
    !evalLib.includes('cp "$eval_log_file" "$eval_report_dir" 2>/dev/null || true') ||
    !evalLib.includes('rm -R -- "$eval_owned_state" || {')
  ) {
    findings.push("evaluation process and log cleanup is not bounded and best-effort");
  }
  for (const name of [
    "SYNVEDA_DB_TEST_MAIN_DATA_SUBNET",
    "SYNVEDA_DB_TEST_LIFECYCLE_DATA_SUBNET",
    "SYNVEDA_DB_TEST_MAIN_HOST_SUBNET",
    "SYNVEDA_DB_TEST_LIFECYCLE_HOST_SUBNET",
    "SYNVEDA_DB_TEST_ROLES_FILE",
    "SYNVEDA_DB_TEST_LIFECYCLE_ROLES_FILE",
    "SYNVEDA_DB_TEST_EXTERNAL_ROLES_FILE",
    "SYNVEDA_DB_TEST_MAIN_AUTHORITY_DIR",
    "SYNVEDA_DB_TEST_LIFECYCLE_AUTHORITY_DIR",
    "SYNVEDA_DB_TEST_UID",
    "SYNVEDA_DB_TEST_GID",
    "SYNVEDA_DB_TEST_SECRETS_DIR",
    "SYNVEDA_DB_TEST_POSTGRES_IMAGE",
    "SYNVEDA_DB_TEST_TASK",
  ]) {
    if (!new RegExp(`unset [^\\n]*\\b${name}\\b`).test(evalLib)) {
      findings.push(`evaluation retains fixture control input ${name}`);
    }
  }
  return findings;
}

export function sqlxPrepareFixtureFindings(dbTest) {
  const findings = [];
  if (
    !dbTest.includes(
      "workspace|demo|product-evaluation|evaluation|longmemeval-evaluation|sqlx-prepare)",
    )
  ) {
    findings.push("SQLx prepare task is not explicitly allow-listed");
  }
  if (!dbTest.includes("  sqlx-prepare) fast_fixture=true ;;")) {
    findings.push("SQLx prepare task does not select the fresh exact-role fixture");
  }
  if (!dbTest.includes('if [ "$db_test_task" = sqlx-prepare ] && [ "$#" -ne 0 ]; then')) {
    findings.push("SQLx prepare task accepts unreviewed positional arguments");
  }
  const fastStart = dbTest.indexOf("# Demos and evaluations need the exact");
  const hostileStart = dbTest.indexOf("enable_hostile_database_logging() {");
  const fast =
    fastStart >= 0 && hostileStart > fastStart
      ? dbTest.slice(fastStart, hostileStart)
      : "";
  const branchStart = fast.indexOf('  if [ "$db_test_task" = sqlx-prepare ]; then\n');
  const preflight = fast.indexOf('  run_main_database_preflight "$main_witness_file"');
  const branch =
    branchStart >= 0 && preflight > branchStart
      ? fast.slice(branchStart, preflight)
      : "";
  const migrate = branch.indexOf("cargo sqlx migrate run --no-dotenv");
  const migrationSource = branch.indexOf("--source crates/synveda-store/migrations", migrate);
  const versionProbe = branch.indexOf("sqlx_cli_banner=$(cargo sqlx --version)");
  const versionCompare = branch.indexOf(
    '[ "$sqlx_cli_banner" = "sqlx-cli-sqlx $sqlx_library_version" ]',
  );
  const versionGuard = [
    '    [ -n "$sqlx_library_version" ] \\',
    '      && [ "$sqlx_cli_banner" = "sqlx-cli-sqlx $sqlx_library_version" ] || {',
    '        echo "db-test: cargo-sqlx must exactly match the locked sqlx library" >&2',
    "        exit 69",
    "      }",
  ].join("\n");
  const versionGuardAt = branch.indexOf(versionGuard);
  const prepare = branch.indexOf(
    "cargo sqlx prepare --no-dotenv --workspace -- --all-targets",
  );
  const prepareCheck = branch.indexOf(
    "cargo sqlx prepare --check --no-dotenv --workspace -- --all-targets",
  );
  const applicationMigrate = fast.indexOf(
    "cargo run -q -p synveda-cli --bin synveda -- db migrate",
    preflight,
  );
  if (
    migrate < 0 ||
    migrationSource <= migrate ||
    prepare <= migrationSource ||
    prepareCheck <= prepare ||
    preflight <= branchStart ||
    applicationMigrate <= preflight
  ) {
    findings.push(
      "SQLx regeneration is not direct migration, prepare, check, product preflight and product migration in order",
    );
  }
  if (
    versionProbe < 0 ||
    versionCompare <= versionProbe ||
    versionGuardAt < versionProbe ||
    versionGuardAt >= migrate ||
    versionCompare >= migrate ||
    (branch.match(/\bcargo\s+/g) ?? []).length !== 4
  ) {
    findings.push(
      "SQLx regeneration permits an extra Cargo invocation or mutates before its version proof",
    );
  }
  if (
    branch.split("SYNVEDA_CARGO_DATABASE_URL_FILE=$main_migrator_file").length - 1 !== 3 ||
    branch.split("scripts/cargo-with-database-url-file").length - 1 !== 3 ||
    branch.split("env -u SYNVEDA_DB_TEST_SECRETS_DIR -u SQLX_OFFLINE").length - 1 !== 3
  ) {
    findings.push("SQLx migrate/prepare/check do not share the private migrator-file boundary");
  }
  if (
    versionProbe < 0 ||
    versionCompare < 0 ||
    !branch.includes("' Cargo.lock)")
  ) {
    findings.push("cargo-sqlx is not proved equal to the locked sqlx library");
  }
  if (/main_(?:owner|gateway|worker)_file|postgres-lifecycle/.test(branch)) {
    findings.push("SQLx prepare task exposes a non-migrator or lifecycle database target");
  }
  return findings;
}

export function demoFixtureFindings(dbTest, demoHarness, ciWorkflow) {
  const findings = [];
  if (
    !dbTest.includes(
      "workspace|demo|product-evaluation|evaluation|longmemeval-evaluation|sqlx-prepare)",
    ) ||
    !dbTest.includes("  demo|product-evaluation|evaluation|longmemeval-evaluation) fast_fixture=true ;;")
  ) {
    findings.push("demo task does not select the fresh exact-role fixture");
  }
  const dispatchStart = dbTest.indexOf("    demo)\n");
  const dispatchEnd = dbTest.indexOf("    product-evaluation)\n", dispatchStart);
  const dispatch =
    dispatchStart >= 0 && dispatchEnd > dispatchStart
      ? dbTest.slice(dispatchStart, dispatchEnd)
      : "";
  for (const marker of [
    "SYNVEDA_EXACT_ROLE_DEMO=1",
    "SYNVEDA_CARGO_DATABASE_URL_FILE=$main_gateway_file",
    "SQLX_OFFLINE=true",
    "SYNVEDA_DATABASE_ROLES_FILE=$roles_file",
    "SYNVEDA_TEST_DATABASE_URL_FILE=$main_gateway_file",
    "SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE=$main_migrator_file",
    'scripts/cargo-with-database-url-file sh "$demo_script" "$@"',
  ]) {
    if (!dispatch.includes(marker)) findings.push(`demo dispatch omits ${marker}`);
  }
  if (
    !demoHarness.includes("if [ \"${SYNVEDA_EXACT_ROLE_DEMO:-}\" != 1 ]; then") ||
    !demoHarness.includes("SYNVEDA_DB_TEST_TASK=demo") ||
    !demoHarness.includes('bash "$DEMO_REPO_ROOT/scripts/db-test.sh" "$0" "$@"')
  ) {
    findings.push("shared demo harness does not re-exec through the exact-role fixture");
  }
  for (const retired of [
    "deploy/compose/docker-compose.yml",
    "createdb -U synveda",
    "postgres://synveda:synveda-dev",
  ]) {
    if (demoHarness.includes(retired)) {
      findings.push(`shared demo harness retains ${retired}`);
    }
  }
  if (
    !ciWorkflow.includes("- run: demos/ops-9-beta-demo.sh") ||
    !demoHarness.includes('SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE:-') ||
    !demoHarness.includes('SYNVEDA_DATABASE_ROLES_FILE:-')
  ) {
    findings.push("beta demo CI path is not guarded by the exact-role fixture contract");
  }
  return findings;
}

export function lifecyclePeerWitnessFindings(dbTest, dbTestCompose) {
  const findings = [];
  const service = serviceBlock(`\n${dbTestCompose}`, "keycloak-database-bootstrap-lifecycle");
  const synvedaBootstraps = [
    serviceBlock(`\n${dbTestCompose}`, "database-bootstrap-main"),
    serviceBlock(`\n${dbTestCompose}`, "database-bootstrap-lifecycle"),
  ];
  const postgresAnchorStart = dbTestCompose.indexOf("x-postgres: &postgres\n");
  const postgresAnchorEnd = dbTestCompose.indexOf("\nx-bootstrap: &bootstrap\n");
  const postgresAnchor =
    postgresAnchorStart >= 0 && postgresAnchorEnd > postgresAnchorStart
      ? dbTestCompose.slice(postgresAnchorStart, postgresAnchorEnd)
      : "";
  const postgresMain = serviceBlock(`\n${dbTestCompose}`, "postgres-main");
  const postgresLifecycle = serviceBlock(`\n${dbTestCompose}`, "postgres-lifecycle");
  const externalBootstrap = serviceBlock(
    `\n${dbTestCompose}`,
    "database-bootstrap-external-lifecycle",
  );
  for (const marker of [
    "    <<: *bootstrap\n",
    '    command: ["keycloak"]\n',
    "postgres://synveda_owner@postgres-lifecycle:5432/postgres",
    'SYNVEDA_POSTGRES_BUNDLED_CLUSTER: "true"',
    "SYNVEDA_DATABASE_AUTHORITY_DIR: /run/synveda/database-authority",
    "source: postgres_owner_password",
    "source: keycloak_database_password",
    "source: ${SYNVEDA_DB_TEST_ROLES_FILE:?set SYNVEDA_DB_TEST_ROLES_FILE}",
    "source: ${SYNVEDA_DB_TEST_LIFECYCLE_AUTHORITY_DIR:?set SYNVEDA_DB_TEST_LIFECYCLE_AUTHORITY_DIR}",
    "lifecycle-data: {}",
  ]) {
    if (!service.includes(marker)) {
      findings.push(`lifecycle Keycloak witness service is missing ${marker.trim()}`);
    }
  }
  for (const forbidden of [
    "postgres-main",
    "main-data",
    "synveda_migrator_password",
    "synveda_gateway_password",
    "synveda_worker_password",
    "SYNVEDA_DB_TEST_LIFECYCLE_ROLES_FILE",
  ]) {
    if (service.includes(forbidden)) {
      findings.push(`lifecycle Keycloak witness service receives ${forbidden}`);
    }
  }
  for (const [index, synvedaBootstrap] of synvedaBootstraps.entries()) {
    for (const marker of [
      'SYNVEDA_DATABASE_REQUIRE_KEYCLOAK_PASSWORD: "true"',
      "source: postgres_owner_password",
      "source: synveda_migrator_password",
      "source: synveda_gateway_password",
      "source: synveda_worker_password",
      "source: keycloak_database_password",
    ]) {
      if (!synvedaBootstrap.includes(marker)) {
        findings.push(
          `${index === 0 ? "main" : "lifecycle"} Synveda bootstrap credential set is missing ${marker}`,
        );
      }
    }
  }

  const collisionStart = dbTest.indexOf(
    "# The reference topology uses one PostgreSQL server but five independent",
  );
  const collisionEnd = dbTest.indexOf(
    "# The same deployment bootstrap owns both fixtures.",
    collisionStart,
  );
  const collisionEvidence =
    collisionStart >= 0 && collisionEnd > collisionStart
      ? dbTest.slice(collisionStart, collisionEnd)
      : "";
  for (const marker of [
    "--network none",
    "tr -d '\\n'",
    "SYNVEDA_DATABASE_REQUIRE_KEYCLOAK_PASSWORD=true",
    "target=/run/secrets/postgres_bootstrap_password",
    "target=/run/secrets/synveda_migrator_password",
    "target=/run/secrets/synveda_gateway_password",
    "target=/run/secrets/synveda_worker_password",
    "target=/run/secrets/keycloak_database_password",
    "database-bootstrap: database credentials must be pairwise distinct",
    'fresh_before=$(catalog_fingerprint postgres-main postgres)',
    'fresh_after=$(catalog_fingerprint postgres-main postgres)',
  ]) {
    if (!collisionEvidence.includes(marker)) {
      findings.push(`database credential-collision evidence is missing ${marker}`);
    }
  }
  if (
    postgresAnchor.includes("external_provider_password") ||
    postgresMain.includes("external_provider_password")
  ) {
    findings.push("main PostgreSQL inherits the external-provider credential");
  }
  if (
    !postgresLifecycle.includes(
      "source: external_provider_password\n        target: external_provider_password",
    ) ||
    !externalBootstrap.includes(
      "source: external_provider_password\n        target: postgres_bootstrap_password",
    ) ||
    dbTestCompose.split("source: external_provider_password").length - 1 !== 2
  ) {
    findings.push("external-provider credential is not confined to its two lifecycle consumers");
  }

  const secretGenerator = dbTest.indexOf(
    "SYNVEDA_SECRETS_DIR=$secret_dir deploy/compose/scripts/generate-secrets.sh",
  );
  const externalGenerate = dbTest.indexOf(
    "external_provider_password=$(openssl rand -hex 32)",
    secretGenerator,
  );
  const externalWrite = dbTest.indexOf(
    'printf \'%s\\n\' "$external_provider_password" > "$secret_dir/external_provider_password"',
    externalGenerate,
  );
  const externalUnset = dbTest.indexOf("unset external_provider_password", externalWrite);
  const externalMode = dbTest.indexOf(
    'chmod 600 "$secret_dir/external_provider_password"',
    externalUnset,
  );
  if (
    !(
      secretGenerator >= 0 &&
      externalGenerate > secretGenerator &&
      externalWrite > externalGenerate &&
      externalUnset > externalWrite &&
      externalMode > externalUnset
    ) ||
    /(?:^|\n)\s*(?:cp|mv|ln)\s+[^\n]*password[^\n]*external_provider_password/.test(dbTest)
  ) {
    findings.push("external-provider credential is not generated independently and mode-confined");
  }

  const start = dbTest.indexOf("# A wrong-cluster preflight negative control needs a genuine");
  const end = dbTest.indexOf("# The lifecycle cluster must not already contain", start);
  const branch = start >= 0 && end > start ? dbTest.slice(start, end) : "";
  const before = branch.indexOf(
    "lifecycle_peer_before=$(catalog_fingerprint postgres-lifecycle synveda)",
  );
  const converge = branch.indexOf(
    "compose run --rm --no-deps keycloak-database-bootstrap-lifecycle",
  );
  const witness = branch.indexOf(
    "lifecycle_witness_file=$lifecycle_authority_dir/keycloak-cluster.json",
  );
  const membershipProof = branch.indexOf(
    "from pg_catalog.pg_auth_members membership",
    witness,
  );
  const membershipGranted = branch.indexOf("granted.rolname = 'keycloak'", membershipProof);
  const membershipMember = branch.indexOf("member.rolname = 'keycloak'", membershipGranted);
  const membershipGrantor = branch.indexOf("grantor.rolname = 'keycloak'", membershipMember);
  const membershipProofEnd = branch.indexOf(
    ") then 1 else 0 end;",
    membershipGrantor,
  );
  const scrub = branch.indexOf("drop database keycloak with (force);");
  const dropRole = branch.indexOf("drop role keycloak;");
  const after = branch.indexOf(
    "lifecycle_peer_after=$(catalog_fingerprint postgres-lifecycle synveda)",
  );
  const equality = branch.indexOf(
    '[ "$lifecycle_peer_before" = "$lifecycle_peer_after" ] || {',
  );
  if (
    !(
      before >= 0 &&
      converge > before &&
      witness > converge &&
      membershipProof > witness &&
      membershipGranted > membershipProof &&
      membershipMember > membershipGranted &&
      membershipGrantor > membershipMember &&
      membershipProofEnd > membershipGrantor &&
      scrub > membershipProofEnd &&
      dropRole > scrub &&
      after > dropRole &&
      equality > after
    )
  ) {
    findings.push(
      "lifecycle peer witness is not converged, retained and catalog-restored in order",
    );
  }
  if (
    branch.split("compose run --rm --no-deps keycloak-database-bootstrap-lifecycle").length -
      1 !==
    1
  ) {
    findings.push("lifecycle Keycloak witness convergence is not a single bounded invocation");
  }
  if (!branch.includes('assert_database_secrets_absent "$lifecycle_witness_file"')) {
    findings.push("lifecycle peer witness is not checked for secret disclosure");
  }
  if (branch.includes("revoke keycloak from synveda_owner")) {
    findings.push("lifecycle peer cleanup retains a warning-producing nonexistent membership");
  }

  const restartStart = dbTest.indexOf("# A server restart changes the writable generation marker");
  const restartEnd = dbTest.indexOf("# A later inherited grant", restartStart);
  const restartBranch =
    restartStart >= 0 && restartEnd > restartStart
      ? dbTest.slice(restartStart, restartEnd)
      : "";
  const restart = restartBranch.indexOf("compose restart postgres-main");
  const refreshedPort = restartBranch.indexOf("main_port=$(published_port postgres-main)");
  const refusal = restartBranch.indexOf(
    "expect_main_preflight_refusal stale-after-restart",
  );
  const refreshedUrls = [
    "main_owner_file",
    "main_migrator_file",
    "main_gateway_file",
    "main_worker_file",
  ].every((file) =>
    restartBranch
      .slice(refreshedPort, refusal)
      .includes(`\"$main_port\" synveda \"$${file}\"`),
  );
  if (!(restart >= 0 && refreshedPort > restart && refusal > refreshedPort && refreshedUrls)) {
    findings.push("restart witness test does not refresh every dynamic-port database URL");
  }
  if (
    dbTest.includes(
      "revoke synveda_app from synveda_gateway, synveda_worker\n  granted by cpr45_external_bootstrap;",
    )
  ) {
    findings.push("external-provider cleanup retains a warning-producing nonexistent grantor edge");
  }
  return findings;
}

export function evalSignalTrapFindings(run, longmemeval) {
  const findings = [];
  for (const [name, source] of [
    ["evaluation", run],
    ["LongMemEval", longmemeval],
  ]) {
    for (const trap of [
      "trap 'eval_finish $?' EXIT",
      "trap 'eval_finish 130' INT",
      "trap 'eval_finish 143' TERM",
    ]) {
      if (!source.includes(trap)) findings.push(`${name} lacks ${trap}`);
    }
    if (source.includes("trap eval_down EXIT INT TERM")) {
      findings.push(`${name} uses an ambiguous shared cleanup trap`);
    }
  }
  return findings;
}

export function localDockerCopySources(source) {
  const sources = [];
  for (const line of source.split("\n")) {
    const instruction = line.trim();
    if (!instruction.startsWith("COPY ") || instruction.startsWith("COPY --from=")) continue;
    const fields = instruction.slice("COPY ".length).trim().split(/\s+/);
    if (fields.length < 2 || fields.some((field) => field.includes("$"))) continue;
    sources.push(...fields.slice(0, -1).filter((field) => !field.startsWith("--")));
  }
  return sources;
}

export function missingLocalDockerCopySources(source, pathExists) {
  return localDockerCopySources(source).filter((path) => !pathExists(path));
}

export function missingWorkspaceManifestCopies(source, manifests) {
  const copied = new Set(localDockerCopySources(source));
  return manifests.filter((manifest) => !copied.has(manifest));
}

export function suppressesCargoBuildFailure(source) {
  return /cargo build[^\n]*\|\|\s*true/.test(source);
}

export function productImageFindings(source) {
  const findings = [];
  const cargoBuilds = source
    .split("\n")
    .filter((line) => !line.trimStart().startsWith("#") && /\bcargo build\b/.test(line));
  if (
    cargoBuilds.length !== 2 ||
    cargoBuilds.some((line) => !/\bcargo build --locked\b/.test(line))
  ) {
    findings.push("release Cargo builds are not exactly two locked invocations");
  }
  const stages = [...source.matchAll(/^FROM\s+.*$/gim)];
  const finalStage = stages.length > 0 ? source.slice(stages.at(-1).index) : "";
  const finalActive = finalStage.replace(/^[ \t]*#.*(?:\r?\n|$)/gm, "");
  if (!/^FROM\s+\S+\s+AS\s+runtime\s*$/im.test(finalActive)) {
    findings.push("final stage is not the named runtime stage");
  }
  const users = [...finalActive.matchAll(/^\s*USER\s+([^\s#]+)/gim)].map(
    (match) => match[1],
  );
  const runtimeUser = users.at(-1);
  if (!runtimeUser || !/^[1-9][0-9]*:[1-9][0-9]*$/.test(runtimeUser)) {
    findings.push("final runtime user is not an explicit non-zero UID:GID");
  }
  for (const [binary, instruction] of [
    [
      "synveda-gateway",
      "COPY --from=build /src/target/release/synveda-gateway /usr/local/bin/synveda-gateway",
    ],
    [
      "synveda-worker",
      "COPY --from=build /src/target/release/synveda-worker /usr/local/bin/synveda-worker",
    ],
    ["synveda", "COPY --from=build /src/target/release/synveda /usr/local/bin/synveda"],
    [
      "synveda-container",
      "COPY --chmod=0755 deploy/compose/gateway/synveda-container /usr/local/bin/synveda-container",
    ],
  ]) {
    if (!finalActive.includes(instruction)) {
      findings.push(`final runtime stage omits ${binary}`);
    }
  }
  if (!finalActive.includes('ENTRYPOINT ["/usr/local/bin/synveda-container"]')) {
    findings.push("role-neutral entrypoint is missing");
  }
  if (!finalActive.includes('CMD ["gateway"]')) {
    findings.push("default gateway role is missing");
  }
  if (/^\s*HEALTHCHECK\b/m.test(finalActive)) {
    findings.push("image hard-codes a role-specific healthcheck");
  }
  if (!/^\s*STOPSIGNAL\s+SIGTERM\s*$/m.test(finalActive)) {
    findings.push("SIGTERM stop signal is missing");
  }
  return findings;
}

export function productTestSupportFindings(dockerfile, gatewayMain, workerMain) {
  const findings = [];
  const supportManifest =
    "crates/synveda-gateway/test-support-enabler/Cargo.toml";
  if (!localDockerCopySources(dockerfile).includes(supportManifest)) {
    findings.push("dependency cache omits the gateway test-support manifest");
  }
  if (
    !dockerfile.includes(
      "mkdir -p crates/synveda-gateway/test-support-enabler/src",
    ) ||
    !dockerfile.includes(
      ": > crates/synveda-gateway/test-support-enabler/src/lib.rs",
    )
  ) {
    findings.push("dependency cache omits the gateway test-support target stub");
  }
  if (
    /^\s*RUN\b[^\n]*cargo build[^\n]*(?:--all-features|test-support)/im.test(
      dockerfile,
    ) ||
    /cargo build[^\n]*(?:--all-features|test-support)/i.test(dockerfile)
  ) {
    findings.push("product image enables gateway test support");
  }
  const guard =
    '#[cfg(all(feature = "test-support", not(test), not(debug_assertions)))]';
  if (!gatewayMain.includes(guard)) {
    findings.push("gateway release binary lacks the test-support refusal");
  }
  if (!workerMain.includes(guard)) {
    findings.push("worker release binary lacks the test-support refusal");
  }
  return findings;
}

export function dockerignoreFindings(source) {
  const rules = new Set(
    source
      .split("\n")
      .map((line) => line.trim())
      .filter((line) => line && !line.startsWith("#")),
  );
  const findings = [];
  for (const required of [
    ".git",
    ".git/**",
    ".agents",
    ".agents/**",
    ".codex",
    ".codex/**",
    "target",
    "target/**",
    "node_modules",
    "node_modules/**",
    ".env",
    ".env.*",
    "**/.env",
    "**/.env.*",
    "deploy/compose/secrets",
    "deploy/compose/secrets/**",
    "deploy/compose/runtime",
    "deploy/compose/runtime/**",
    "deploy/compose/backups",
    "deploy/compose/backups/**",
  ]) {
    if (!rules.has(required)) findings.push(`build context includes ${required}`);
  }
  const allowedNegations = new Set(["!**/.env.example"]);
  for (const rule of rules) {
    if (rule.startsWith("!") && !allowedNegations.has(rule)) {
      findings.push(`unreviewed build-context re-inclusion ${rule}`);
    }
  }
  return findings;
}

export function productLauncherFindings(source) {
  const findings = [];
  const active = source.replace(/^[ \t]*#.*(?:\r?\n|$)/gm, "");
  const caseLabels = active
    .split("\n")
    .map((line) => line.trimStart())
    .filter((line) => !/^[a-zA-Z_][a-zA-Z0-9_]*\(\)[ \t]*\{/.test(line))
    .map((line) => line.match(/^([^)]*)\)/)?.[1]?.trim())
    .filter((label) => label !== undefined);
  const expectedCaseLabels = [
    "gateway",
    "worker",
    "database-preflight",
    "migrate",
    "probe",
    "gateway",
    "worker",
    "*",
    "live",
    "ready",
    "*",
    "*",
  ];
  if (JSON.stringify(caseLabels) !== JSON.stringify(expectedCaseLabels)) {
    findings.push("launcher case vocabulary is not closed and ordered");
  }
  const roleMatches = [
    ...active.matchAll(
      /^ {4}(gateway|worker|database-preflight|migrate|probe|\*)\)[ \t]*$/gm,
    ),
  ];
  const labels = roleMatches.map(
    (match) => match[1],
  );
  if (
    JSON.stringify(labels) !==
    JSON.stringify(["gateway", "worker", "database-preflight", "migrate", "probe", "*"])
  ) {
    findings.push("launcher role vocabulary is not closed and ordered");
  }

  const roleBlock = (role) => {
    const position = roleMatches.findIndex((match) => match[1] === role);
    if (position < 0) return "";
    const current = roleMatches[position];
    const next = roleMatches[position + 1];
    const start = current.index + current[0].length;
    return active.slice(start, next?.index);
  };
  const gateway = roleBlock("gateway");
  const worker = roleBlock("worker");
  const databasePreflight = roleBlock("database-preflight");
  const migrate = roleBlock("migrate");
  const probe = roleBlock("probe").replace(/\\\r?\n\s*/g, " ");
  const unknown = roleBlock("*").replace(/\s+/g, " ").trim();

  if (!gateway.includes('[ "$#" -eq 1 ] || usage')) {
    findings.push("gateway role does not enforce exact arity");
  }
  if (!gateway.includes("exec /usr/local/bin/synveda-gateway")) {
    findings.push("gateway role does not exec the gateway binary");
  }
  if (!worker.includes('[ "$#" -eq 1 ] || usage')) {
    findings.push("worker role does not enforce exact arity");
  }
  if (!worker.includes("exec /usr/local/bin/synveda-worker")) {
    findings.push("worker role does not exec the worker binary");
  }
  if (!databasePreflight.includes('[ "$#" -eq 1 ] || usage')) {
    findings.push("database-preflight role does not enforce exact arity");
  }
  if (!databasePreflight.includes("exec /usr/local/bin/synveda db preflight")) {
    findings.push("database-preflight role does not exec the target verification command");
  }
  if (!migrate.includes('[ "$#" -eq 1 ] || usage')) {
    findings.push("migrate role does not enforce exact arity");
  }
  if (!migrate.includes("exec /usr/local/bin/synveda db migrate")) {
    findings.push("migrate role does not exec the migration command");
  }
  if (!probe.includes('[ "$#" -eq 3 ] || usage')) {
    findings.push("probe role does not enforce exact arity");
  }
  if (!/gateway\)\s+port=8120\s+;;/.test(probe)) {
    findings.push("gateway probe does not select the fixed 8120 port");
  }
  if (!/worker\)\s+port=8121\s+;;/.test(probe)) {
    findings.push("worker probe does not select the fixed 8121 port");
  }
  if (!/live\)\s+path=healthz\s+;;/.test(probe)) {
    findings.push("live probe does not select /healthz");
  }
  if (!/ready\)\s+path=readyz\s+;;/.test(probe)) {
    findings.push("ready probe does not select /readyz");
  }
  if (!probe.includes('"http://127.0.0.1:${port}/${path}"')) {
    findings.push("probe does not use the selected fixed loopback endpoint");
  }
  if (!/exec \/usr\/bin\/curl\s+--disable(?:\s|$)/.test(probe)) {
    findings.push("probe permits curl configuration loading");
  }
  if (!/--noproxy\s+['"]\*['"]/.test(probe)) {
    findings.push("probe permits inherited proxy routing");
  }
  for (const option of ["--connect-timeout 1", "--max-time 2", "--fail"]) {
    if (!probe.includes(option)) findings.push(`probe is missing ${option}`);
  }
  if (unknown !== "usage ;; esac") {
    findings.push("unknown role does not fail through usage");
  }
  if (!/^#!\/bin\/sh\s*$/m.test(source) || !/^set -eu\s*$/m.test(active)) {
    findings.push("launcher shell or fail-closed options are missing");
  }
  if (/\beval\b/.test(active) || /\$\(|`/.test(active)) {
    findings.push("launcher evaluates input");
  }
  if (/\b(?:compose|saas|enterprise)\b/.test(active)) {
    findings.push("launcher branches on deployment shape");
  }
  return findings;
}

function renderedResource(documents, kind, component) {
  return documents.find(
    (document) =>
      new RegExp(`^kind: ${kind}$`, "m").test(document) &&
      document.includes(`app.kubernetes.io/component: ${component}`),
  );
}

function renderedContainerBlock(document, name, nextMarker) {
  if (!document) return "";
  const start = document.indexOf(`- name: ${name}\n`);
  if (start < 0) return "";
  const end = document.indexOf(nextMarker, start);
  return document.slice(start, end < 0 ? document.length : end);
}

export function helmContractFindings(rendered) {
  const findings = [];
  const documents = rendered.split(/^---\s*$/m);
  const cluster = documents.find((document) => /^kind: Cluster$/m.test(document));
  const gateway = renderedResource(documents, "Deployment", "gateway");
  const worker = renderedResource(documents, "Deployment", "worker");
  const install = renderedResource(documents, "Job", "install");
  const databaseRoles = renderedResource(documents, "ConfigMap", "database-contract");

  if (!cluster || !/^\s+owner: synveda_migrator$/m.test(cluster)) {
    findings.push("CloudNativePG application owner is not synveda_migrator");
  }
  const postInit = cluster?.indexOf("postInitSQL:") ?? -1;
  const publicRevoke =
    cluster?.indexOf(
      "revoke connect, temporary on database postgres, template1 from public",
    ) ?? -1;
  const applicationInit = cluster?.indexOf("postInitApplicationSQL:") ?? -1;
  if (!(postInit >= 0 && postInit < publicRevoke && applicationInit === -1)) {
    findings.push(
      "CloudNativePG does not close PUBLIC maintenance-database access or still creates extensions as the application owner",
    );
  }
  if (!gateway) findings.push("gateway Deployment is missing");
  if (!worker) findings.push("worker Deployment is missing");
  if (!install) findings.push("install Job is missing");
  for (const [name, document] of [
    ["gateway", gateway],
    ["worker", worker],
  ]) {
    if (document?.includes("initContainers:")) {
      findings.push(`${name} retains a blocking startup init container`);
    }
    if (document?.includes("wait-for-schema")) {
      findings.push(`${name} retains the retired schema-wait loop`);
    }
  }
  if (
    !databaseRoles ||
    !databaseRoles.includes(
      '{"migrator":"synveda_migrator","gateway":"synveda_gateway","worker":"synveda_worker","administrators":["postgres"],"administrative_memberships":[],"forbidden_databases":["postgres","template1"],"isolated_peer_roles":[]}',
    )
  ) {
    findings.push("database role contract ConfigMap is missing or drifted");
  }

  for (const [name, document, secretPath] of [
    ["gateway", gateway, "/run/secrets/synveda-gateway/database_url"],
    ["worker", worker, "/run/secrets/synveda-worker/database_url"],
  ]) {
    if (!document) continue;
    for (const marker of [
      "- name: DATABASE_URL_FILE",
      `value: ${secretPath}`,
      "- name: SYNVEDA_DATABASE_ROLES_FILE",
      "value: /etc/synveda/database/roles.json",
      "- name: database-roles",
      "mountPath: /etc/synveda/database",
      "readOnly: true",
    ]) {
      if (!document.includes(marker)) findings.push(`${name} is missing ${marker}`);
    }
    if (document.includes("SYNVEDA_EXPECTED_DATABASE_ROLE")) {
      findings.push(`${name} retains the obsolete inferred-role setting`);
    }
    const forbidden =
      name === "gateway"
        ? ["synveda-pg-app", "synveda-pg-superuser", "synveda-worker-db"]
        : ["synveda-pg-app", "synveda-pg-superuser", "synveda-gateway-db"];
    for (const secret of forbidden) {
      if (document.includes(secret)) findings.push(`${name} receives forbidden Secret ${secret}`);
    }
  }

  if (gateway && worker) {
    const image = (document, name) =>
      document.match(new RegExp(`\\n\\s+- name: ${name}\\n\\s+image: (\\S+)`))?.[1];
    if (!image(gateway, "gateway") || image(gateway, "gateway") !== image(worker, "worker")) {
      findings.push("gateway and worker do not use one product image");
    }
  }

  if (install) {
    const deadline = /\n\s+activeDeadlineSeconds:\s+(\d+)\s*(?:\n|$)/.exec(install)?.[1];
    if (!deadline || Number(deadline) < 300 || Number(deadline) > 3600) {
      findings.push("install Job does not have the deployment-contract deadline");
    }
    const order = [
      "- name: database-bootstrap\n",
      "- name: database-preflight\n",
      "- name: migrate\n",
      "- name: tenant\n",
    ].map((marker) => install.indexOf(marker));
    if (order.some((index) => index < 0) || order.some((index, i) => i > 0 && index <= order[i - 1])) {
      findings.push("install Job does not order bootstrap, preflight, migrate and tenant");
    }

    const bootstrap = renderedContainerBlock(
      install,
      "database-bootstrap",
      "- name: database-preflight\n",
    );
    const preflight = renderedContainerBlock(install, "database-preflight", "- name: migrate\n");
    const migrate = renderedContainerBlock(install, "migrate", "containers:\n");
    const tenant = renderedContainerBlock(install, "tenant", "volumes:\n");

    for (const marker of [
      "synveda-database-bootstrap",
      "exec /usr/local/bin/synveda-database-bootstrap synveda",
      "synveda-pg-superuser",
    ]) {
      if (!bootstrap.includes(marker)) findings.push(`database bootstrap is missing ${marker}`);
    }
    for (const marker of [
      'args: ["database-preflight"]',
      "SYNVEDA_MIGRATOR_DATABASE_URL_FILE",
      "SYNVEDA_GATEWAY_DATABASE_URL_FILE",
      "SYNVEDA_WORKER_DATABASE_URL_FILE",
      "SYNVEDA_DATABASE_ROLES_FILE",
      "/etc/synveda/database/roles.json",
      "mountPath: /etc/synveda/database",
      "readOnly: true",
    ]) {
      if (!preflight.includes(marker)) findings.push(`database preflight is missing ${marker}`);
    }
    for (const [name, block] of [
      ["migrate", migrate],
      ["tenant", tenant],
    ]) {
      for (const marker of [
        "DATABASE_URL_FILE",
        "/run/secrets/synveda-migrator/database_url",
        "SYNVEDA_DATABASE_ROLES_FILE",
        "/etc/synveda/database/roles.json",
        "mountPath: /etc/synveda/database",
        "readOnly: true",
      ]) {
        if (!block.includes(marker)) findings.push(`${name} is missing ${marker}`);
      }
      if (block.includes("SYNVEDA_EXPECTED_DATABASE_ROLE")) {
        findings.push(`${name} retains the obsolete inferred-role setting`);
      }
    }
    for (const [name, block] of [
      ["preflight", preflight],
      ["migrate", migrate],
      ["tenant", tenant],
    ]) {
      if (block.includes("synveda-pg-superuser") || block.includes("postgres_bootstrap_password")) {
        findings.push(`${name} receives the PostgreSQL superuser credential`);
      }
    }
  }

  return findings;
}

function fail(message) {
  throw new Error(`deployment convergence: ${message}`);
}

function read(relative) {
  return readFileSync(join(ROOT, relative), "utf8");
}

function run(command, args, options = {}) {
  try {
    return execFileSync(command, args, {
      cwd: ROOT,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      ...options,
    });
  } catch (error) {
    const stderr = error?.stderr?.toString().trim();
    fail(`${command} ${args.join(" ")} failed${stderr ? `: ${stderr}` : ""}`);
  }
}

function checkCompose(relative, release) {
  const source = read(relative);
  if (!release) {
    const buildFindings = contributorPostgresBuildFindings(source);
    if (buildFindings.length > 0) fail(`${relative} ${buildFindings.join(", ")}`);
  }
  const gateway = serviceBlock(`\n${source}`, "gateway");
  if (!gateway) fail(`${relative} has no gateway service`);
  if (!gateway.includes("synveda_gateway")) {
    fail(`${relative} does not connect the gateway as synveda_gateway`);
  }
  if (/postgres:\/\/synveda:/.test(gateway)) {
    fail(`${relative} hands the database-owner DSN to the gateway`);
  }
  if (!gateway.includes("SYNVEDA_GATEWAY_DATABASE_URL")) {
    fail(`${relative} cannot receive a separately provisioned runtime DSN`);
  }
  if (!gateway.includes('SYNVEDA_WORKER_DATABASE_URL: ""')) {
    fail(`${relative} leaves the worker DSN in the gateway environment`);
  }
  if (
    !gateway.includes(
      '"/usr/local/bin/synveda-container", "probe", "gateway", "live"',
    )
  ) {
    fail(`${relative} does not attach the gateway role-specific liveness probe`);
  }
  const worker = serviceBlock(`\n${source}`, "worker");
  if (!worker) fail(`${relative} has no worker service`);
  if (!worker.includes("synveda_worker")) {
    fail(`${relative} does not connect the worker as synveda_worker`);
  }
  if (/postgres:\/\/synveda:/.test(worker)) {
    fail(`${relative} hands the database-owner DSN to the worker`);
  }
  if (!worker.includes("SYNVEDA_WORKER_DATABASE_URL")) {
    fail(`${relative} cannot receive a separately provisioned worker DSN`);
  }
  if (!worker.includes('SYNVEDA_GATEWAY_DATABASE_URL: ""')) {
    fail(`${relative} leaves the gateway DSN in the worker environment`);
  }
  if (!worker.includes('command: ["worker"]')) {
    fail(`${relative} does not select the worker image command`);
  }
  if (!worker.includes("stop_grace_period: 85s")) {
    fail(`${relative} can kill the worker before its bounded drain completes`);
  }
  if (release && !worker.includes("restart: unless-stopped")) {
    fail(`${relative} does not restart a failed installed worker`);
  }
  if (!worker.includes('"/usr/local/bin/synveda-container", "probe", "worker", "ready"')) {
    fail(`${relative} does not attach the worker readiness probe`);
  }
  if (/^\s*ports:/m.test(worker)) {
    fail(`${relative} publishes the worker health surface`);
  }
  const gatewayImage = gateway.match(/^\s*image:\s*(\S+)/m)?.[1];
  const workerImage = worker.match(/^\s*image:\s*(\S+)/m)?.[1];
  if (!gatewayImage || gatewayImage !== workerImage) {
    fail(`${relative} does not use one image for gateway and worker`);
  }
  if (release && /^\s*build:/m.test(source)) {
    fail(`${relative} is a release manifest with a source build`);
  }
  const retired = retiredFindings(source);
  if (retired.length) fail(`${relative} retains ${retired.join(", ")}`);

  // Compose parsing and interpolation are different failure modes. Rendering
  // with interpolation disabled checks the manifest without exposing the
  // restricted .env values that `synveda init` may have written beside it.
  run("docker", ["compose", "-f", relative, "config", "--no-interpolate"]);
}

function checkHelm() {
  const rendered = run("helm", [
    "template",
    "synveda",
    "deploy/helm/synveda",
    "-f",
    "deploy/helm/synveda/ci/full-values.yaml",
  ]);
  if (!rendered.includes("replicas: 1")) fail("Helm no longer pins one gateway replica");
  const contractFindings = helmContractFindings(rendered);
  if (contractFindings.length) fail(contractFindings.join("; "));
  if (!rendered.includes("synveda-pg-app")) fail("Helm does not use CloudNativePG's app Secret");
  if (!rendered.includes("/readyz")) fail("Helm lost the schema-epoch readiness check");
  const documents = rendered.split(/^---\s*$/m);
  const resource = (kind, component) => renderedResource(documents, kind, component);
  const containerImage = (document, name) =>
    document?.match(new RegExp(`\\n\\s+- name: ${name}\\n\\s+image: (\\S+)`))?.[1];
  const gateway = resource("Deployment", "gateway");
  const worker = resource("Deployment", "worker");
  if (!gateway || !worker) fail("Helm does not render separate gateway and worker Deployments");
  if (containerImage(gateway, "gateway") !== containerImage(worker, "worker")) {
    fail("Helm does not use one image for gateway and worker");
  }
  for (const marker of [
    'args: ["worker"]',
    "- name: DATABASE_URL_FILE",
    "value: /run/secrets/synveda-worker/database_url",
    "value: 127.0.0.1:8121",
    "- name: SYNVEDA_DATABASE_ROLES_FILE",
    "value: /etc/synveda/database/roles.json",
    "mountPath: /etc/synveda/database",
    "- worker\n",
    "- live\n",
    "- ready\n",
    "timeoutSeconds: 3",
  ]) {
    if (!worker.includes(marker)) fail(`Helm worker is missing ${marker.trim()}`);
  }
  if (/^\s*ports:/m.test(worker) || resource("Service", "worker")) {
    fail("Helm publishes the worker health surface");
  }
  if (gateway.includes("SYNVEDA_EXTRACTOR") || !worker.includes("SYNVEDA_EXTRACTOR")) {
    fail("Helm does not assign extractor configuration exclusively to the worker");
  }
  for (const secret of ["synveda-dev", "Synveda-Demo-Passw0rd", "API-Key synveda-dev"]) {
    if (rendered.includes(secret)) fail(`Helm rendered a plaintext credential marker: ${secret}`);
  }
  const retired = retiredFindings(rendered);
  if (retired.length) fail(`rendered Helm retains ${retired.join(", ")}`);
}

function checkPublicContract() {
  const openapi = JSON.parse(read("docs/api/openapi.json"));
  for (const path of [
    "/v1/sessions",
    "/v1/sessions/{session_id}/events",
    "/v1/knowledge",
    "/v1/capture-candidates",
    "/v1/context-runs/{id}",
    "/v1/configurations/effective",
  ]) {
    if (!openapi.paths[path]) fail(`generated OpenAPI is missing ${path}`);
  }
  for (const path of ["/v1/observe", "/v1/inject", "/v1/recall"]) {
    if (openapi.paths[path]) fail(`generated OpenAPI resurrected ${path}`);
  }

  const initSource = read("crates/synveda-cli/src/init.rs");
  const cli = `${read("crates/synveda-cli/src/main.rs")}\n${initSource}`;
  if (hasRetiredDemoField(cli)) {
    fail("the removed init demo switch remains in the CLI model");
  }
  const cutoverFindings = initCutoverFindings(initSource);
  if (cutoverFindings.length > 0) {
    fail(`the withdrawn init contract drifted: ${cutoverFindings.join(", ")}`);
  }
  const gatewayMain = read("crates/synveda-gateway/src/main.rs");
  for (const marker of [
    "capture_worker::run",
    "knowledge_index::run",
    "run_expiry_sweep",
    "directory_sync::run",
  ]) {
    if (gatewayMain.includes(marker)) fail(`gateway still owns worker loop ${marker}`);
  }
  const worker = read("crates/synveda-gateway/src/worker.rs");
  for (const marker of [
    "capture_worker::run",
    "knowledge_index::run",
    "run_expiry_sweep",
    "directory_sync::run",
  ]) {
    if (!worker.includes(marker)) fail(`worker does not own loop ${marker}`);
  }
}

function checkReleaseNotes() {
  const workflow = read(".github/workflows/release.yml");
  const findings = [
    ...releaseNoteFindings(workflow),
    ...releasePostgresBuildFindings(workflow),
  ];
  if (findings.length > 0) {
    fail(`release workflow contains ${findings.join(", ")}`);
  }
}

function checkProductImageInputs() {
  const relative = "deploy/compose/gateway/Dockerfile";
  const source = read(relative);
  const missing = missingLocalDockerCopySources(source, (path) =>
    existsSync(join(ROOT, path)),
  );
  if (missing.length > 0) {
    fail(`${relative} copies missing build inputs: ${missing.join(", ")}`);
  }
  const manifests = readdirSync(join(ROOT, "crates"), { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => `crates/${entry.name}/Cargo.toml`)
    .filter((path) => existsSync(join(ROOT, path)))
    .sort();
  const omitted = missingWorkspaceManifestCopies(source, manifests);
  if (omitted.length > 0) {
    fail(`${relative} omits workspace manifests from its cache stage: ${omitted.join(", ")}`);
  }
  if (!localDockerCopySources(source).includes("adapters/registry.json")) {
    fail(`${relative} omits the CLI's embedded adapters/registry.json`);
  }
  if (suppressesCargoBuildFailure(source)) {
    fail(`${relative} suppresses a dependency-cache cargo build failure`);
  }
  const imageFindings = productImageFindings(source);
  if (imageFindings.length > 0) {
    fail(`${relative} violates the product image contract: ${imageFindings.join(", ")}`);
  }
  const testSupportFindings = productTestSupportFindings(
    source,
    read("crates/synveda-gateway/src/main.rs"),
    read("crates/synveda-gateway/src/bin/synveda-worker.rs"),
  );
  if (testSupportFindings.length > 0) {
    fail(
      `${relative} violates the test-support boundary: ${testSupportFindings.join(", ")}`,
    );
  }

  const launcherRelative = "deploy/compose/gateway/synveda-container";
  const launcher = read(launcherRelative);
  const launcherFindings = productLauncherFindings(launcher);
  if (launcherFindings.length > 0) {
    fail(`${launcherRelative} violates the launcher contract: ${launcherFindings.join(", ")}`);
  }
  run("sh", ["-n", launcherRelative]);

  const ignoreFindings = dockerignoreFindings(read(".dockerignore"));
  if (ignoreFindings.length > 0) {
    fail(`.dockerignore violates the build-context contract: ${ignoreFindings.join(", ")}`);
  }

  const postgresRelative = "deploy/compose/postgres/Dockerfile";
  const postgresImage = read(postgresRelative);
  const postgresMissing = missingLocalDockerCopySources(postgresImage, (path) =>
    existsSync(join(ROOT, path)),
  );
  if (postgresMissing.length > 0) {
    fail(`${postgresRelative} copies missing repo-root inputs: ${postgresMissing.join(", ")}`);
  }
  const postgresTargetFindings = postgresImageTargetFindings(postgresImage);
  if (postgresTargetFindings.length > 0) {
    fail(`${postgresRelative} violates target isolation: ${postgresTargetFindings.join(", ")}`);
  }
  const composePgvectorPin = "postgresql-17-pgvector=0.8.6-1.pgdg12+1";
  if (postgresImage.split(composePgvectorPin).length - 1 !== 1) {
    fail(`${postgresRelative} does not install exactly ${composePgvectorPin}`);
  }
  const helmPostgresRelative = "deploy/helm/postgres/Dockerfile";
  const helmPostgresImage = read(helmPostgresRelative);
  const helmPgvectorPin = "postgresql-17-pgvector=0.8.6-1.pgdg11+1";
  if (helmPostgresImage.split(helmPgvectorPin).length - 1 !== 1) {
    fail(`${helmPostgresRelative} does not install exactly ${helmPgvectorPin}`);
  }
  const initdbFindings = developmentInitdbFindings(
    read("deploy/compose/postgres/development-initdb.sql"),
  );
  if (initdbFindings.length > 0) {
    fail(`development PostgreSQL init violates its boundary: ${initdbFindings.join(", ")}`);
  }

  const functionOrderFindings = shellFunctionOrderFindings(read("scripts/db-test.sh"), [
    "private_evidence_file",
    "assert_database_secrets_absent",
  ]);
  if (functionOrderFindings.length > 0) {
    fail(`scripts/db-test.sh violates shell function ordering: ${functionOrderFindings.join(", ")}`);
  }
  const evalFindings = evalFixtureFindings(
    read("scripts/db-test.sh"),
    read("evals/lib.sh"),
    read(".github/workflows/ci.yml"),
    read(".github/workflows/eval.yml"),
  );
  if (evalFindings.length > 0) {
    fail(`evaluation fixture violates the exact-role contract: ${evalFindings.join(", ")}`);
  }
  const sqlxFindings = sqlxPrepareFixtureFindings(read("scripts/db-test.sh"));
  if (sqlxFindings.length > 0) {
    fail(`SQLx prepare fixture violates the exact-role contract: ${sqlxFindings.join(", ")}`);
  }
  const demoFindings = demoFixtureFindings(
    read("scripts/db-test.sh"),
    read("demos/lib/current-platform-demo.sh"),
    read(".github/workflows/ci.yml"),
  );
  if (demoFindings.length > 0) {
    fail(`demo fixture violates the exact-role contract: ${demoFindings.join(", ")}`);
  }
  const lifecycleWitnessFindings = lifecyclePeerWitnessFindings(
    read("scripts/db-test.sh"),
    read("deploy/compose/compose.db-test.yaml"),
  );
  if (lifecycleWitnessFindings.length > 0) {
    fail(
      `lifecycle peer-witness fixture violates the exact-role contract: ${lifecycleWitnessFindings.join(", ")}`,
    );
  }
  const evalTrapFindings = evalSignalTrapFindings(
    read("evals/run.sh"),
    read("evals/run-longmemeval.sh"),
  );
  if (evalTrapFindings.length > 0) {
    fail(`evaluation signal cleanup drifted: ${evalTrapFindings.join(", ")}`);
  }
}

function checkReleaseUpgradeShape() {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-deploy-check-"));
  try {
    const version = "0.2.0";
    run("bash", ["scripts/package-release.sh", version, scratch]);
    const stage = join(scratch, `synveda-profile-${version}`);
    const stale = join(stage, "retired-demo-sentinel");
    writeFileSync(stale, "must be removed by replacement\n");
    run("bash", ["scripts/package-release.sh", version, scratch]);
    if (existsSync(stale)) fail("a repeated release package retained a stale profile file");

    const archive = join(scratch, `synveda-profile-${version}.tar.gz`);
    const entries = run("tar", ["-tzf", archive]).split("\n").filter(Boolean);
    const expected = new Set([
      `synveda-profile-${version}/`,
      `synveda-profile-${version}/docker-compose.yml`,
      `synveda-profile-${version}/rauthy/`,
      `synveda-profile-${version}/rauthy/config.toml`,
      `synveda-profile-${version}/version`,
    ]);
    for (const entry of entries) {
      if (!expected.has(entry)) fail(`release profile contains unexpected entry ${entry}`);
    }
    if (entries.some((entry) => entry.includes("/demo/"))) {
      fail("release profile still packages the retired demo seeder");
    }
    const packaged = readFileSync(join(stage, "docker-compose.yml"), "utf8");
    if (!serviceBlock(`\n${packaged}`, "gateway").includes("synveda_gateway")) {
      fail("packaged release drifted from the least-privilege gateway DSN");
    }
    if (!serviceBlock(`\n${packaged}`, "worker").includes("synveda_worker")) {
      fail("packaged release drifted from the least-privilege worker DSN");
    }
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

export function main() {
  checkCompose("deploy/compose/docker-compose.yml", false);
  checkCompose("deploy/release/docker-compose.yml", true);
  checkHelm();
  checkProductImageInputs();
  checkPublicContract();
  checkReleaseNotes();
  checkReleaseUpgradeShape();
  console.log(
    "deployment convergence holds: 2 Compose renders, Helm render, product image inputs, " +
      "current OpenAPI, distinct Compose/Helm runtime DSNs, three-role Helm preflight and " +
      "repeatable release replacement",
  );
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}
