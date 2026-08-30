import assert from "node:assert/strict";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  realpathSync,
  renameSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

import {
  appRouteFindings,
  authorityCleanupOrderFindings,
  caddyTrustBoundaryFindings,
  canonicalComposeFindings,
  collectorConfigFindings,
  composeEnvironment,
  composeNetworkPlan,
  developmentPortBindingFindings,
  identityGateFindings,
  keycloakConvergenceFindings,
  keycloakGenerationGateFindings,
  keycloakHealthFindings,
  keycloakRealmSupervisorFindings,
  makeComposeFixture,
  masterClientAuthorityFindings,
  reviewedKeycloakSourceFindings,
} from "./check-compose-contract.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const COMPOSE = join(ROOT, "deploy/compose");
const WRAPPER = join(COMPOSE, "scripts/compose.sh");
const GENERATOR = join(COMPOSE, "scripts/generate-secrets.sh");
const KEYCLOAK_ENTRYPOINT = join(COMPOSE, "keycloak/keycloak-entrypoint");
const KEYCLOAK_CONVERGENCE = join(COMPOSE, "keycloak/synveda-realm-converge");
const KEYCLOAK_GENERATION_GATE = join(
  COMPOSE,
  "keycloak/synveda-generation-gate",
);
const KEYCLOAK_GENERATION_GATE_SELF_TEST = join(
  COMPOSE,
  "keycloak/synveda-generation-gate-self-test",
);
const KEYCLOAK_HEALTH = join(COMPOSE, "keycloak/synveda-keycloak-health");
const KEYCLOAK_REALM_SUPERVISOR = join(
  COMPOSE,
  "keycloak/synveda-realm-supervise",
);
const KEYCLOAK_PROJECTION = join(COMPOSE, "keycloak/SynvedaKeycloakProjection.java");
const COMPOSE_DEV = join(COMPOSE, "compose.dev.yaml");
const DATABASE_BOOTSTRAP = join(COMPOSE, "postgres/synveda-database-bootstrap");
const DB_TEST = join(ROOT, "scripts/db-test.sh");
const INPUT_SNAPSHOT = join(COMPOSE, "postgres/synveda-input-snapshot.c");
const CADDYFILE = join(COMPOSE, "configs/caddy/Caddyfile");
const APP_DEV = join(COMPOSE, "configs/caddy/app.dev.caddy");
const CARGO_DATABASE_URL_WRAPPER = join(ROOT, "scripts/cargo-with-database-url-file");
const CLUSTER_AUTHORITY_CONTRACT = join(
  COMPOSE,
  "postgres/synveda-cluster-authority-contract.sql",
);
const LOCAL_AUTHORITY_CONTRACT = join(
  COMPOSE,
  "postgres/synveda-local-authority-contract.sql",
);
const CREDENTIAL_LOG_CONTRACT = join(
  COMPOSE,
  "postgres/synveda-credential-log-contract.sql",
);
const EXTENSION_CONTRACT = join(COMPOSE, "postgres/synveda-extension-contract.sql");
const EXTENSION_FINGERPRINT = join(
  ROOT,
  "crates/synveda-store/sql/extension_fingerprint.sql",
);
const IDENTITY_DEV = join(COMPOSE, "configs/caddy/identity.dev.caddy");
const STORE_RUNTIME_ROLE = join(ROOT, "crates/synveda-store/src/runtime_role.rs");
const STORE_LIB = join(ROOT, "crates/synveda-store/src/lib.rs");
const CREDENTIAL_LOG_SETTINGS = [
  ["log_min_messages", "'panic'"],
  ["log_min_error_statement", "'panic'"],
  ["log_error_verbosity", "'terse'"],
  ["log_statement", "'none'"],
  ["log_min_duration_statement", "-1"],
  ["log_min_duration_sample", "-1"],
  ["log_statement_sample_rate", "0"],
  ["log_transaction_sample_rate", "0"],
  ["log_parameter_max_length", "0"],
  ["log_parameter_max_length_on_error", "0"],
  ["debug_print_parse", "off"],
  ["debug_print_rewritten", "off"],
  ["debug_print_plan", "off"],
];

function occurrenceCount(source, token) {
  return source.split(token).length - 1;
}

function replaceOccurrence(source, token, occurrence, replacement) {
  let start = 0;
  let index = -1;
  for (let current = 0; current <= occurrence; current += 1) {
    index = source.indexOf(token, start);
    if (index < 0) return source;
    start = index + token.length;
  }
  return `${source.slice(0, index)}${replacement}${source.slice(index + token.length)}`;
}

function terminalKeycloakFenceFindings(source) {
  const findings = [];
  const start = source.indexOf("# Hold only the Keycloak-database target lock.");
  const end = source.indexOf('if wait "$terminal_lock_process"; then', start);
  const branch = start >= 0 && end > start ? source.slice(start, end) : "";
  const holder = branch.indexOf("env PGAPPNAME=cpr45-keycloak-quarantine-lock");
  const holderDatabase = branch.indexOf("--dbname keycloak", holder);
  const holderLock = branch.indexOf("synveda.compose.bootstrap.keycloak", holderDatabase);
  const holderObserved = branch.indexOf("terminal_lock_observed=", holderLock);
  const holderObservedDatabase = branch.indexOf(
    "activity.datname = 'keycloak'",
    holderObserved,
  );
  const holderLockDatabase = branch.indexOf(
    "lock.database = activity.datid",
    holderObserved,
  );
  const bootstrap = branch.indexOf(
    "compose run --rm --no-deps keycloak-database-bootstrap-main",
    holderLockDatabase,
  );
  const waiterPoll = branch.indexOf("terminal_bootstrap_lock_count=", bootstrap);
  const waiterMarker = branch.indexOf(
    "activity.application_name = 'synveda-keycloak-bootstrap-target'",
    waiterPoll,
  );
  const waiterEvent = branch.indexOf(
    "activity.wait_event_type = 'Lock' and activity.wait_event = 'advisory'",
    waiterMarker,
  );
  const waiterGranted = branch.indexOf(
    "lock.database = activity.datid and lock.granted) = 1",
    waiterEvent,
  );
  const waiterUngranted = branch.indexOf(
    "lock.database = activity.datid and not lock.granted) = 1",
    waiterGranted,
  );
  const blockerProof = branch.indexOf(
    "pg_catalog.pg_blocking_pids(activity.pid)",
    waiterUngranted,
  );
  const blockerMarker = branch.indexOf(
    "holder.application_name = 'cpr45-keycloak-quarantine-lock'",
    blockerProof,
  );
  const handshake = branch.indexOf(
    '[ "$terminal_bootstrap_waiting" = true ] || {',
    blockerMarker,
  );
  const liveness = branch.indexOf('kill -0 "$terminal_bootstrap_process"', handshake);
  const aclInjection = branch.indexOf(
    "grant select on public.cpr45_terminal_keycloak_probe to synveda_app;",
    liveness,
  );
  const holderTermination = branch.indexOf(
    "where datname = 'keycloak' and application_name = 'cpr45-keycloak-quarantine-lock'",
    aclInjection,
  );
  if (
    !(
      holder >= 0 &&
      holderDatabase > holder &&
      holderLock > holderDatabase &&
      holderObserved > holderLock &&
      holderObservedDatabase > holderObserved &&
      holderLockDatabase > holderObservedDatabase &&
      bootstrap > holderLockDatabase &&
      waiterPoll > bootstrap &&
      waiterMarker > waiterPoll &&
      waiterEvent > waiterMarker &&
      waiterGranted > waiterEvent &&
      waiterUngranted > waiterGranted &&
      blockerProof > waiterUngranted &&
      blockerMarker > blockerProof &&
      handshake > blockerMarker &&
      liveness > handshake &&
      aclInjection > liveness &&
      holderTermination > aclInjection
    )
  ) {
    findings.push(
      "terminal Keycloak fence does not prove one same-database holder, waiter and blocker before drift",
    );
  }
  return findings;
}

function keycloakAdmissionRecoveryFindings(source) {
  const findings = [];
  const helperStart = source.indexOf("assert_keycloak_admission_empty() {");
  const helperEnd = source.indexOf("\n}\n\n# Test isolation is intentional", helperStart);
  const helper =
    helperStart >= 0 && helperEnd > helperStart
      ? source.slice(helperStart, helperEnd)
      : "";
  if (helper.includes("pg_catalog.coalesce(")) {
    findings.push("database acceptance schema-qualifies SQL COALESCE syntax as a function");
  }
  const helperOid = helper.indexOf("as keycloak_database_oid");
  const helperStartup = helper.indexOf("from pg_catalog.pg_locks lock", helperOid);
  const helperSnapshot = helper.indexOf(
    "pg_catalog.pg_stat_clear_snapshot()",
    helperStartup,
  );
  const helperActivity = helper.indexOf(
    "from pg_catalog.pg_stat_activity activity",
    helperSnapshot,
  );
  const helperPrepared = helper.indexOf(
    "from pg_catalog.pg_prepared_xacts prepared",
    helperActivity,
  );
  if (
    !(
      helperOid >= 0 &&
      helperStartup > helperOid &&
      helperSnapshot > helperStartup &&
      helperActivity > helperSnapshot &&
      helperPrepared > helperActivity
    )
  ) {
    findings.push("database acceptance does not prove Keycloak admission populations in order");
  }
  for (const token of [
    "lock.locktype = 'object'",
    "lock.database = 0",
    "lock.classid = 'pg_catalog.pg_database'::pg_catalog.regclass",
    "lock.objid = :'keycloak_database_oid'::pg_catalog.oid",
    "lock.objsubid = 0",
    "lock.mode = 'RowExclusiveLock'",
    "lock.pid is not null",
    "activity.datid = :'keycloak_database_oid'::pg_catalog.oid",
    "database.oid = :'keycloak_database_oid'::pg_catalog.oid",
  ]) {
    if (!helper.includes(token)) {
      findings.push(`database acceptance admission proof lacks ${token}`);
    }
  }
  if (helper.includes("lock.granted")) {
    findings.push("database acceptance filters its zero-startup-lock proof by grant state");
  }

  const postOpenStart = source.indexOf(
    "# A refusal after the global transaction opens Keycloak",
  );
  const resumeStart = source.indexOf(
    "# Simulate interruption after the quarantine closure transaction commits",
    postOpenStart,
  );
  const acceptanceEnd = source.indexOf("# Global metadata cannot distinguish", resumeStart);
  const postOpen =
    postOpenStart >= 0 && resumeStart > postOpenStart
      ? source.slice(postOpenStart, resumeStart)
      : "";
  const postOpenSession = postOpen.indexOf("PGAPPNAME=cpr45-keycloak-post-open-session");
  const postOpenMount = postOpen.indexOf(
    '--volume "$post_open_contract:/usr/local/share/synveda/cluster-authority-contract.sql:ro"',
    postOpenSession,
  );
  const postOpenRefusal = postOpen.indexOf(
    "database-bootstrap: Keycloak role or database convergence was refused",
    postOpenMount,
  );
  const postOpenSessionExit = postOpen.indexOf(
    'wait "$post_open_session_process"',
    postOpenRefusal,
  );
  const postOpenEmpty = postOpen.indexOf(
    'assert_keycloak_admission_empty "post-open Keycloak failure"',
    postOpenSessionExit,
  );
  const postOpenClosed = postOpen.indexOf("not database.datallowconn", postOpenEmpty);
  const postOpenNoWitness = postOpen.indexOf(
    '[ ! -e "$main_authority_dir/keycloak-cluster.json" ]',
    postOpenClosed,
  );
  const postOpenRestore = postOpen.indexOf(
    "alter role keycloak login; alter database keycloak allow_connections true",
    postOpenNoWitness,
  );
  const postOpenConverge = postOpen.indexOf(
    "compose run --rm --no-deps keycloak-database-bootstrap-main",
    postOpenRestore,
  );
  if (
    !(
      postOpenSession >= 0 &&
      postOpenMount > postOpenSession &&
      postOpenRefusal > postOpenMount &&
      postOpenSessionExit > postOpenRefusal &&
      postOpenEmpty > postOpenSessionExit &&
      postOpenClosed > postOpenEmpty &&
      postOpenNoWitness > postOpenClosed &&
      postOpenRestore > postOpenNoWitness &&
      postOpenConverge > postOpenRestore
    )
  ) {
    findings.push("database acceptance does not quarantine a deterministic post-open failure");
  }
  for (const token of [
    "cp deploy/compose/postgres/synveda-cluster-authority-contract.sql",
    ":'synveda_bootstrap_target' <> 'keycloak'",
    ":'synveda_require_complete_roles' <> 'true'",
    ":'synveda_allow_target_owner_membership' <> 'false'",
    ":'synveda_allow_target_default_acl' <> 'false'",
  ]) {
    if (!postOpen.includes(token)) {
      findings.push(`post-open database acceptance lacks ${token}`);
    }
  }

  const resume =
    resumeStart >= 0 && acceptanceEnd > resumeStart
      ? source.slice(resumeStart, acceptanceEnd)
      : "";
  const resumeVisible = resume.indexOf("PGAPPNAME=cpr45-keycloak-resume-session");
  const resumeDelayed = resume.indexOf("PGOPTIONS='-c post_auth_delay=120'", resumeVisible);
  const resumeStartupProof = resume.indexOf(
    "from pg_catalog.pg_locks lock",
    resumeDelayed,
  );
  const resumeHidden = resume.indexOf(
    "where activity.pid = lock.pid",
    resumeStartupProof,
  );
  const resumeClosure = resume.indexOf(
    "begin;\nalter role keycloak nologin;\nalter database keycloak allow_connections false;\ncommit;",
    resumeHidden,
  );
  const resumeRetained = resume.indexOf("resume_retained_shape=", resumeClosure);
  const resumeBootstrap = resume.indexOf(
    "compose run --rm --no-deps keycloak-database-bootstrap-main",
    resumeRetained,
  );
  const resumeRefusal = resume.indexOf(
    "database-bootstrap: interrupted Keycloak quarantine remains closed",
    resumeBootstrap,
  );
  const resumeVisibleExit = resume.indexOf('wait "$resume_session_process"', resumeRefusal);
  const resumeStartupExit = resume.indexOf(
    'wait "$resume_startup_process"',
    resumeVisibleExit,
  );
  const resumeEmpty = resume.indexOf(
    'assert_keycloak_admission_empty "resumed Keycloak quarantine"',
    resumeStartupExit,
  );
  const resumeNoWitness = resume.indexOf(
    '[ ! -e "$main_authority_dir/keycloak-cluster.json" ]',
    resumeEmpty,
  );
  if (
    !(
      resumeVisible >= 0 &&
      resumeDelayed > resumeVisible &&
      resumeStartupProof > resumeDelayed &&
      resumeHidden > resumeStartupProof &&
      resumeClosure > resumeHidden &&
      resumeRetained > resumeClosure &&
      resumeBootstrap > resumeRetained &&
      resumeRefusal > resumeBootstrap &&
      resumeVisibleExit > resumeRefusal &&
      resumeStartupExit > resumeVisibleExit &&
      resumeEmpty > resumeStartupExit &&
      resumeNoWitness > resumeEmpty
    )
  ) {
    findings.push("database acceptance does not resume and drain both Keycloak admission populations");
  }
  for (const token of [
    "database.datallowconn",
    "lock.locktype = 'object'",
    "lock.database = 0",
    "lock.classid = 'pg_catalog.pg_database'::pg_catalog.regclass",
    "lock.objsubid = 0",
    "lock.mode = 'RowExclusiveLock'",
    "lock.pid is not null",
    "and lock.granted",
    "not exists (",
    "activity.application_name = 'cpr45-keycloak-resume-session'",
    "lock.pid = :'expected_startup_pid'::integer",
  ]) {
    if (!resume.includes(token)) {
      findings.push(`crash-resume database acceptance lacks ${token}`);
    }
  }
  return findings;
}

function databaseRoleTopologyPredicates(branch) {
  const predicates = [];
  const startToken =
    "and pg_catalog.jsonb_typeof(contract.document->'forbidden_databases') = 'array'";
  const endToken = "\n     )\n) = 1";
  let offset = 0;
  while (offset < branch.length) {
    const start = branch.indexOf(startToken, offset);
    if (start < 0) break;
    const end = branch.indexOf(endToken, start);
    if (end < 0) break;
    predicates.push(branch.slice(start, end + endToken.length).replace(/\s+/g, " ").trim());
    offset = end + endToken.length;
  }
  return predicates;
}

function clusterAuthorityContractFindings(source) {
  const findings = [];
  if (source.includes("pg_catalog.coalesce(")) {
    findings.push("cluster authority schema-qualifies SQL COALESCE syntax as a function");
  }
  for (const token of [
    "pg_catalog.has_database_privilege",
    "role.rolcreatedb",
    "role.rolcreaterole",
    "role.rolreplication",
    "role.rolbypassrls",
    "role.rolconnlimit <> -1",
    "role.rolvaliduntil is distinct from 'infinity'::timestamptz",
    "pg_catalog.pg_db_role_setting",
    "setting.setrole = 0",
    "setting.setrole = (select principal.oid from bootstrap_principal principal)",
    "membership.roleid",
    "membership.member",
    "membership.grantor",
    "grantor.rolname = session_user",
    "not membership.admin_option",
    "membership.inherit_option",
    "membership.set_option",
    "granted.rolname = 'pg_read_all_settings'",
    "granted.rolname not in (",
    "not principal.rolsuper",
    "(select count(*) from expected) between 1 and 8",
    ":'synveda_bootstrap_target' in ('synveda', 'keycloak')",
    ":'synveda_require_complete_roles' = 'true'",
    ":'synveda_allow_target_owner_membership' = 'true'",
    ":'synveda_allow_target_owner_membership' in ('true', 'false')",
    ":'synveda_allow_target_default_acl' in ('true', 'false')",
    ":'synveda_allow_target_default_acl' = 'true'",
    "database_name = :'synveda_bootstrap_database'",
    ":'synveda_bootstrap_database' not in ('synveda', 'keycloak')",
    "acl.grantee = 0",
    "pg_catalog.pg_default_acl",
    "pg_catalog.pg_parameter_acl",
    "pg_catalog.pg_shdepend",
    "pg_catalog.jsonb_array_elements(",
    "pg_catalog.jsonb_typeof(value) <> 'string'",
    "database_name = 'keycloak'",
    "forbidden.database_name <> 'keycloak'",
    "and not database.datallowconn",
    "database.datlocprovider = (select template.datlocprovider from template)",
    "settings.setdatabase = database.oid",
    "pg_catalog.pg_locks",
    "lock.locktype = 'object'",
    "lock.database = 0",
    "lock.classid = 'pg_catalog.pg_database'::pg_catalog.regclass",
    "lock.objid = :'synveda_keycloak_database_oid'::pg_catalog.oid",
    "lock.objsubid = 0",
    "lock.mode = 'RowExclusiveLock'",
    "lock.pid is not null",
    "pg_catalog.pg_stat_activity",
    "pg_catalog.pg_prepared_xacts",
    "synveda_keycloak_no_startup_locks",
    "synveda_keycloak_no_activity",
    "synveda_keycloak_no_prepared_xacts",
    "dependency.deptype = 'o'",
    "dependency.deptype = 'a'",
    "synveda_database.oid is not null",
    "keycloak_database.oid is not null",
  ]) {
    if (!source.includes(token)) findings.push(`cluster authority lacks ${token}`);
  }
  if (source.includes("pg_catalog.jsonb_array_elements_text(")) {
    findings.push("cluster authority coerces forbidden database elements to text");
  }
  const forbiddenConnectStart = source.indexOf(
    "where pg_catalog.has_database_privilege(role.oid, database.oid, 'CONNECT')",
  );
  const forbiddenConnectEnd = source.indexOf("\n) and not exists (", forbiddenConnectStart);
  const forbiddenConnect = source
    .slice(forbiddenConnectStart, forbiddenConnectEnd)
    .replace(/\s+/g, " ")
    .trim();
  const nullSafeOwnerBindings = occurrenceCount(
    source,
    "database.datdba is not distinct from",
  );
  const keycloakOid = source.indexOf("as synveda_keycloak_database_oid");
  const noStartup = source.indexOf("as synveda_keycloak_no_startup_locks", keycloakOid);
  const snapshotClear = source.indexOf("pg_catalog.pg_stat_clear_snapshot()", noStartup);
  const noActivity = source.indexOf("as synveda_keycloak_no_activity", snapshotClear);
  const noPrepared = source.indexOf("as synveda_keycloak_no_prepared_xacts", noActivity);
  if (
    !(
      keycloakOid >= 0 &&
      noStartup > keycloakOid &&
      snapshotClear > noStartup &&
      noActivity > snapshotClear &&
      noPrepared > noActivity
    ) ||
    source.slice(keycloakOid, noPrepared).includes("lock.granted")
  ) {
    findings.push("closed Keycloak startup handoff is not proved in lock/activity order");
  }
  if (
    forbiddenConnectStart < 0 ||
    forbiddenConnectEnd < 0 ||
    !forbiddenConnect.startsWith(
      "where pg_catalog.has_database_privilege(role.oid, database.oid, 'CONNECT') and not ( ( :'synveda_allow_target_default_acl' = 'true'",
    ) ||
    !forbiddenConnect.includes(
      ") or ( :'synveda_bootstrap_target' in ('synveda', 'keycloak')",
    ) ||
    !forbiddenConnect.includes(
      "from pg_catalog.pg_db_role_setting settings where settings.setdatabase = database.oid )",
    ) ||
    forbiddenConnect.includes("settings.setrole = 0") ||
    !forbiddenConnect.includes(
      "and :'synveda_keycloak_no_startup_locks' = 'true' and :'synveda_keycloak_no_activity' = 'true' and :'synveda_keycloak_no_prepared_xacts' = 'true'",
    ) ||
    !forbiddenConnect.endsWith(") )")
  ) {
    findings.push("closed Keycloak recovery is not nested inside the CONNECT exception");
  }
  if (
    nullSafeOwnerBindings !== 3 ||
    source.includes("database.datdba = case :'synveda_bootstrap_target'")
  ) {
    findings.push("closed database owner bindings are not independently NULL-safe");
  }
  return findings;
}

test("the Collector health contract is loopback-only and self-probing", () => {
  const config = readFileSync(join(COMPOSE, "configs/otel/collector.yaml"), "utf8");
  assert.deepEqual(collectorConfigFindings(config), []);

  const exposed = config.replace("endpoint: 127.0.0.1:13133", "endpoint: 0.0.0.0:13133");
  assert.ok(
    collectorConfigFindings(exposed).includes(
      "Collector health endpoint is not container-loopback-only",
    ),
  );
  const scalar = config.replace(
    /    response_body:\n      healthy: .*\n      unhealthy: .*\n/,
    "    response_body: healthy\n",
  );
  const scalarFindings = collectorConfigFindings(scalar);
  assert.ok(
    scalarFindings.includes(
      "Collector healthy response is not the content-free nop pipeline config",
    ),
  );
  assert.ok(
    scalarFindings.includes("Collector unhealthy response is not the closed empty object"),
  );
});

test("the proxy trust boundary is one closed Caddy grammar", () => {
  const source = readFileSync(CADDYFILE, "utf8");
  assert.deepEqual(caddyTrustBoundaryFindings(source), []);
  const forwardedIdentity = source.replace(
    "\theader_up X-Forwarded-For {remote_host}\n",
    "\theader_up X-Forwarded-For {remote_host}\n" +
      "\theader_up X-Forwarded-For {http.request.header.X-Forwarded-For}\n",
  );
  assert.notEqual(forwardedIdentity, source);
  assert.ok(caddyTrustBoundaryFindings(forwardedIdentity).length > 0);
});

test("the application edge is one closed Caddy route grammar", () => {
  const source = readFileSync(APP_DEV, "utf8");
  assert.deepEqual(appRouteFindings(source), []);
  const identityBypass = source.replace(
    "\thandle /metrics {",
    "\timport /etc/caddy/identity.caddy\n\thandle /metrics {",
  );
  assert.notEqual(identityBypass, source);
  assert.ok(appRouteFindings(identityBypass).length > 0);
});

test("the complete Keycloak proof executable chain is review-locked", () => {
  const entrypoint = readFileSync(KEYCLOAK_ENTRYPOINT, "utf8");
  assert.deepEqual(
    reviewedKeycloakSourceFindings("keycloak/keycloak-entrypoint", entrypoint),
    [],
  );
  for (const [name, mutated] of [
    [
      "generation rotation bypass",
      entrypoint.replace(
        '"$generation_gate" rotate >/dev/null || {\n',
        "true || {\n",
      ),
    ],
    [
      "ambient generation accepted",
      entrypoint.replace(
        '[ -z "${SYNVEDA_KEYCLOAK_GENERATION+x}" ] || {\n',
        "if false; then\n",
      ),
    ],
  ]) {
    assert.notEqual(mutated, entrypoint, `${name} mutant did not alter entrypoint`);
    assert.ok(
      reviewedKeycloakSourceFindings("keycloak/keycloak-entrypoint", mutated).length > 0,
      `review lock accepted ${name}`,
    );
  }

  const projection = readFileSync(KEYCLOAK_PROJECTION, "utf8");
  assert.deepEqual(
    reviewedKeycloakSourceFindings("keycloak/SynvedaKeycloakProjection.java", projection),
    [],
  );
  assert.deepEqual(masterClientAuthorityFindings(projection), []);
  for (const [name, mutated] of [
    [
      "legacy empty-array oracle",
      projection.replace(
        "                int statusCode = sendDiscarding(",
        "                BoundedResponse response = sendBounded(",
      ).replace(
        "                    proofDeadlineNanos\n"
          + "                );\n"
          + "                verifyForbiddenAuthorityResponse(statusCode);",
        "                    4096,\n"
          + "                    proofDeadlineNanos\n"
          + "                );\n"
          + "                verifyEmptyArrayResponse(\n"
          + "                    response.statusCode(), response.body()\n"
          + "                );",
      ),
    ],
    [
      "body-retaining transport",
      projection.replace("int statusCode = sendDiscarding(", "int statusCode = sendBounded("),
    ],
    [
      "broadened status",
      projection.replace(
        "verifyForbiddenAuthorityResponse(statusCode);",
        "if (statusCode != 403 && statusCode != 200) throw new IllegalArgumentException();",
      ),
    ],
    [
      "early 200 bypass",
      projection.replace(
        "                verifyForbiddenAuthorityResponse(statusCode);",
        "                if (statusCode == 200) return;\n"
          + "                verifyForbiddenAuthorityResponse(statusCode);",
      ),
    ],
    [
      "removed client discriminator",
      projection.replace('+ "?clientId=admin-cli",', '+ "",'),
    ],
    [
      "stage attribution drift",
      projection.replace(
        "atAuthorityStage(AuthorityStage.MASTER_CLIENTS, () -> {",
        "atAuthorityStage(AuthorityStage.MASTER_SESSION_STATS, () -> {",
      ),
    ],
  ]) {
    assert.notEqual(mutated, projection, `${name} mutant did not alter projection`);
    assert.ok(
      masterClientAuthorityFindings(mutated).length > 0,
      `master-client authority contract accepted ${name}`,
    );
  }
  assert.deepEqual(authorityCleanupOrderFindings(projection), []);
  const unguardedTokenContract = projection.replace(
    "        String refreshToken = tokenGrant.refreshToken();\n"
      + "        runAuthorityProofWithCleanup(() -> {\n"
      + "            AuthorityTokens tokens = atAuthorityStage(\n"
      + "                AuthorityStage.TOKEN_CONTRACT,\n"
      + "                () -> parseAuthorityTokenResponse(tokenGrant.response())\n"
      + "            );",
    "        String refreshToken = tokenGrant.refreshToken();\n"
      + "        AuthorityTokens tokens = atAuthorityStage(\n"
      + "            AuthorityStage.TOKEN_CONTRACT,\n"
      + "            () -> parseAuthorityTokenResponse(tokenGrant.response())\n"
      + "        );\n"
      + "        runAuthorityProofWithCleanup(() -> {",
  );
  assert.notEqual(unguardedTokenContract, projection);
  assert.deepEqual(authorityCleanupOrderFindings(unguardedTokenContract), [
    "Keycloak authority grant, guarded token contracts or cleanup order drifted",
  ]);
  const guardedRefreshContract =
    "            atAuthorityStage(\n"
    + "                AuthorityStage.REFRESH_CONTRACT,\n"
    + "                () -> verifyAuthorityRefreshContract(";
  const unguardedRefreshContract = projection
    .replace(guardedRefreshContract, "")
    .replace(
      "        String refreshToken = tokenGrant.refreshToken();\n"
        + "        runAuthorityProofWithCleanup(() -> {",
      "        String refreshToken = tokenGrant.refreshToken();\n"
        + guardedRefreshContract
        + "\n        runAuthorityProofWithCleanup(() -> {",
    );
  assert.notEqual(unguardedRefreshContract, projection);
  assert.deepEqual(authorityCleanupOrderFindings(unguardedRefreshContract), [
    "Keycloak authority grant, guarded token contracts or cleanup order drifted",
  ]);
  const collapsedTokenContract = projection.replace(
    "                AuthorityStage.TOKEN_CONTRACT,\n"
      + "                () -> parseAuthorityTokenResponse(tokenGrant.response())",
    "                AuthorityStage.TOKEN_ENVELOPE,\n"
      + "                () -> parseAuthorityTokenResponse(tokenGrant.response())",
  );
  assert.notEqual(collapsedTokenContract, projection);
  assert.ok(authorityCleanupOrderFindings(collapsedTokenContract).length > 0);
  const collapsedRefreshContract = projection.replace(
    "                AuthorityStage.REFRESH_CONTRACT,\n"
      + "                () -> verifyAuthorityRefreshContract(",
    "                AuthorityStage.TOKEN_CLAIMS,\n"
      + "                () -> verifyAuthorityRefreshContract(",
  );
  assert.notEqual(collapsedRefreshContract, projection);
  assert.ok(authorityCleanupOrderFindings(collapsedRefreshContract).length > 0);
  for (const [name, mutated] of [
    [
      "realm-state proof",
      projection.replace(
        "verifyRealmState(input, exactBoolean(args[2]));",
        "verifyTargetRealm(input);",
      ),
    ],
    [
      "audit role",
      projection.replace(
        'text(role, "name", "view-users");',
        'text(role, "name", "manage-users");',
      ),
    ],
    [
      "master audit client",
      projection.replace(
        'clientName.equals("master-realm")',
        'clientName.equals("realm-management")',
      ),
    ],
    [
      "direct audit composite",
      projection.replace(
        '        bool(role, "composite", true);',
        '        bool(role, "composite", false);',
      ),
    ],
    [
      "missing effective audit child",
      projection.replace(
        '        "query-groups", "query-users", "view-users"',
        '        "query-users", "view-users"',
      ),
    ],
    [
      "extra effective audit role",
      projection.replace(
        '        "query-groups", "query-users", "view-users"',
        '        "manage-users", "query-groups", "query-users", "view-users"',
      ),
    ],
    [
      "master client-list authority",
      projection.replace(
        '        "query-groups", "query-users", "view-users"',
        '        "query-clients", "query-groups", "query-users", "view-users"',
      ),
    ],
    [
      "effective audit composite flags",
      projection.replace(
        'bool(role, "composite", name.equals("view-users"));',
        'bool(role, "composite", false);',
      ),
    ],
    [
      "direct UUID collision",
      projection.replace(
        "                    if (!directIds.add(roleId)) {",
        "                    if (false) {",
      ),
    ],
    [
      "effective client-role UUID collision",
      projection.replace(
        "if (roleId.equals(clientId) || !ids.add(roleId)) {",
        "if (!ids.add(roleId)) {",
      ),
    ],
    [
      "direct-effective audit UUID binding",
      projection.replace(
        'if (name.equals("view-users") && !roleId.equals(directAuditRoleId)) {',
        "if (false) {",
      ),
    ],
    [
      "service-account-inclusive master inventory",
      projection.replace(
        "?first=0&max=3&briefRepresentation=true&exact=false",
        "?first=0&max=3&briefRepresentation=true&exact=true",
      ),
    ],
    [
      "self federated-identity probe",
      projection.replace(
        'masterSelfUrl + "/federated-identity"',
        "masterSelfUrl",
      ),
    ],
    [
      "exact password credential count",
      projection.replace("credentials.size() != 1", "credentials.size() < 1"),
    ],
    [
      "forbidden mutation response",
      projection.replace(
        "if (statusCode != 403) {",
        "if (statusCode != 403 && statusCode != 404) {",
      ),
    ],
    [
      "unordered exact scope membership",
      projection.replace(
        'Set.of("email", "openid", "profile")',
        'Set.of("email", "profile")',
      ),
    ],
    [
      "token response request window",
      projection.replace(
        "Math.subtractExact(responseReceivedAt, requestStartedAt) > 2",
        "Math.subtractExact(responseReceivedAt, requestStartedAt) > 20",
      ),
    ],
    [
      "signed access-token lifetime",
      projection.replace("lifetime > 61", "lifetime > 610"),
    ],
    [
      "access and ID token issuance skew",
      projection.replace("issuedAtDelta > 2", "issuedAtDelta > 20"),
    ],
    [
      "master realm visibility discriminator",
      projection.replace(
        '+ "/client-session-stats",',
        '+ "",',
      ),
    ],
    [
      "status-only master session probe",
      projection.replace(
        "HttpResponse.BodyHandlers.discarding()",
        "HttpResponse.BodyHandlers.replacing(null)",
      ),
    ],
    [
      "cleanup stage attribution",
      projection.replace(
        "new AuthorityProofRefusal(AuthorityStage.CLEANUP)",
        "new AuthorityProofRefusal(AuthorityStage.PROOF_DEADLINE)",
      ),
    ],
    [
      "bounded response subscriber",
      projection.replace(
        "ignored -> new BoundedBodySubscriber(maxBodyBytes)",
        "HttpResponse.BodyHandlers.ofByteArray()",
      ),
    ],
    [
      "absolute proof deadline",
      projection.replace(
        "AUTHORITY_PROOF_BUDGET = Duration.ofSeconds(34)",
        "AUTHORITY_PROOF_BUDGET = Duration.ofSeconds(340)",
      ),
    ],
    [
      "absolute cleanup deadline",
      projection.replace(
        "AUTHORITY_CLEANUP_BUDGET = Duration.ofSeconds(6)",
        "AUTHORITY_CLEANUP_BUDGET = Duration.ofSeconds(60)",
      ),
    ],
    [
      "replacement refresh revocation",
      projection.replace(
        "                replacementRefreshToken,\n                cleanupDeadlineNanos",
        "                refreshToken,\n                cleanupDeadlineNanos",
      ),
    ],
    [
      "strict kcadm session shape",
      projection.replace("session.size() != 6", "session.size() < 6"),
    ],
    [
      "revocation response body",
      projection.replace(
        "if (body.length == 0) {",
        "if (statusCode == 200) {",
      ),
    ],
    [
      "distinct one-shot tokens",
      projection.replace(
        "        if (accessToken.equals(idToken)\n"
          + "            || accessToken.equals(refreshToken)\n"
          + "            || idToken.equals(refreshToken)) {",
        "        if (false) {",
      ),
    ],
  ]) {
    assert.notEqual(mutated, projection, `${name} mutant did not alter projection`);
    assert.ok(
      reviewedKeycloakSourceFindings("keycloak/SynvedaKeycloakProjection.java", mutated).length
        > 0,
      `review lock accepted ${name}`,
    );
  }
});

test("the identity edge stays closed until the exact Keycloak witness is present", () => {
  const source = readFileSync(IDENTITY_DEV, "utf8");
  assert.deepEqual(identityGateFindings(source), []);
  for (const [name, mutated] of [
    [
      "witness filename drift",
      source.replace("cpr45-keycloak-realm-v3.ready", "stale.ready"),
    ],
    [
      "writable-state path drift",
      source.replace(
        "/run/synveda/keycloak-public-gate",
        "/tmp/keycloak-public-gate",
      ),
    ],
    [
      "generation selector removed",
      source.replace("/keycloak-public-gate/current", "/keycloak-public-gate"),
    ],
    ["fail-open fallback", source.replace("respond 503", "reverse_proxy keycloak:8080")],
    ["cacheable refusal", source.replace('header Cache-Control "no-store"\n', "")],
    [
      "operator path exposure",
      source.replace("/resources/*", "/resources/* /admin/*"),
    ],
    [
      "logout confirmation path removed",
      source.replace(
        " /realms/synveda/protocol/openid-connect/logout/logout-confirm",
        "",
      ),
    ],
    [
      "additive ungated handler",
      source.replace(
        "\t@identity_ready {",
        "\thandle /realms/synveda/* {\n\t\treverse_proxy keycloak:8080\n\t}\n\t@identity_ready {",
      ),
    ],
    [
      "alternate-form additive ungated handler",
      source.replace(
        "\t@identity_ready {",
        "\thandle /realms/synveda/* {\n\t\treverse_proxy http://keycloak:8080\n\t}\n\t@identity_ready {",
      ),
    ],
    [
      "additive imported route",
      source.replace(
        "\t@identity_ready {",
        "\timport operator_identity_bypass\n\t@identity_ready {",
      ),
    ],
    [
      "redirect before closed handler",
      source.replace(
        "\t@identity_ready {",
        "\tredir /realms/synveda/* https://example.invalid{uri}\n\t@identity_ready {",
      ),
    ],
  ]) {
    assert.notEqual(mutated, source, `${name} mutant did not alter the fixture`);
    assert.ok(identityGateFindings(mutated).length > 0, `gate accepted ${name}`);
  }
});

test("the Keycloak generation gate fences rotation, publication and retirement", () => {
  const source = readFileSync(KEYCLOAK_GENERATION_GATE, "utf8");
  assert.deepEqual(keycloakGenerationGateFindings(source), []);
  for (const [name, mutated] of [
    [
      "widened root mode",
      source.replace(
        '[ "$(file_mode "$public_gate_dir")" = 700 ] && \\\n',
        '[ "$(file_mode "$public_gate_dir")" = 755 ] && \\\n',
      ),
    ],
    [
      "non-atomic current swap",
      source.replace(
        'mv -Tf -- "$staged_link" "$current_link" || {',
        'cp -P -- "$staged_link" "$current_link" || {',
      ),
    ],
    [
      "selector closure delayed",
      source.replace(
        '        close_current_selector "$previous_generation" || fail\n' +
          '        withdraw_generation "$previous_generation" || fail\n',
        '        withdraw_generation "$previous_generation" || fail\n' +
          '        close_current_selector "$previous_generation" || fail\n',
      ),
    ],
    [
      "post-publication fence removed",
      source.replace(
        '    if ! is_current_generation "$generation"; then\n',
        "    if false; then\n",
      ),
    ],
    [
      "current generation retirement allowed",
      source.replace(
        '    ! is_current_generation "$generation" || return 1\n',
        "    true\n",
      ),
    ],
    [
      "selector closure short-circuited",
      source.replace(
        'close_current_selector() {\n    is_current_generation "$1" || return 1',
        "close_current_selector() {\n    return 0",
      ),
    ],
    [
      "generation grammar widened",
      source.replace("*[!A-Za-z0-9]*) return 1", "*[!A-Za-z0-9_-]*) return 1"),
    ],
    [
      "generation directory mode widened",
      source.replace(
        '[ "$(file_mode "$generation_path")" = 700 ]',
        '[ "$(file_mode "$generation_path")" = 755 ]',
      ),
    ],
    [
      "publication witness made group-readable",
      source.replace('! chmod 0400 "$gate_candidate"', '! chmod 0440 "$gate_candidate"'),
    ],
    [
      "publication rename weakened",
      source.replace(
        'mv -Tf -- "$gate_candidate" "$generation_ready"',
        'mv -f -- "$gate_candidate" "$generation_ready"',
      ),
    ],
  ]) {
    assert.notEqual(mutated, source, `${name} mutant did not alter the fixture`);
    assert.ok(
      keycloakGenerationGateFindings(mutated).length > 0,
      `generation gate accepted ${name}`,
    );
  }
});

test("the Keycloak generation race self-test executes against the reviewed gate", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-generation-race-"));
  const gate = join(scratch, "synveda-generation-gate");
  const selfTest = join(scratch, "synveda-generation-gate-self-test");
  const gateRoot = join(scratch, "gate-root");
  const bin = join(scratch, "bin");
  try {
    mkdirSync(bin, { mode: 0o700 });
    writeFileSync(
      join(bin, "mv"),
      `#!/bin/sh
set -eu
[ "$#" -eq 4 ] && [ "$1" = -Tf ] && [ "$2" = -- ] || exit 64
exec python3 -c 'import os,sys; os.replace(sys.argv[1],sys.argv[2])' "$3" "$4"
`,
      { mode: 0o700 },
    );
    const gateSource = readFileSync(KEYCLOAK_GENERATION_GATE, "utf8").replace(
      "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
      `PATH=${bin}:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin`,
    );
    writeFileSync(gate, gateSource, {
      mode: 0o700,
    });
    const harness = readFileSync(KEYCLOAK_GENERATION_GATE_SELF_TEST, "utf8")
      .replaceAll("/opt/keycloak/bin/synveda-generation-gate", gate)
      .replaceAll("/tmp/synveda-generation-gate-self-test", gateRoot);
    writeFileSync(selfTest, harness, { mode: 0o700 });
    const result = spawnSync("bash", [selfTest], {
      encoding: "utf8",
      env: { PATH: process.env.PATH ?? "/usr/bin:/bin" },
    });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout, "");
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("the Keycloak realm supervisor is fail-closed and generation-fenced", () => {
  const source = readFileSync(KEYCLOAK_REALM_SUPERVISOR, "utf8");
  assert.deepEqual(keycloakRealmSupervisorFindings(source), []);
  for (const [name, mutated] of [
    [
      "startup withdrawal removed",
      source.replace("withdraw_current_generation || {\n", "true || {\n"),
    ],
    [
      "management proof removed",
      source.replace(
        "/opt/keycloak/bin/synveda-keycloak-health network",
        "/bin/true",
      ),
    ],
    [
      "generation argument removed",
      source.replace(
        'synveda-realm-converge "$current_generation" &',
        "synveda-realm-converge &",
      ),
    ],
    [
      "readiness ignored",
      source.replace(
        'if [ "$child_status" -eq 0 ] && $gate ready',
        'if [ "$child_status" -eq 0 ] && true',
      ),
    ],
    [
      "failure latch removed",
      source.replace("failed_generation=$current_generation\n", "failed_generation=\n"),
    ],
    [
      "cleanup withdrawal bypassed",
      source.replace("    if ! withdraw_current_generation; then", "    if false; then"),
    ],
    [
      "withdrawal helper bypassed",
      source.replace(
        '    $gate withdraw "$observed" >/dev/null 2>&1 || return 1',
        "    return 0",
      ),
    ],
    [
      "new-generation withdrawal bypassed",
      source.replace(
        '        $gate withdraw "$current_generation" >/dev/null 2>&1 || {\n' +
          '            echo "keycloak-supervisor: new generation could not be closed" >&2',
        '        true || {\n' +
          '            echo "keycloak-supervisor: new generation could not be closed" >&2',
      ),
    ],
    [
      "degraded-generation withdrawal bypassed",
      source.replace(
        '    if ! management_ready; then\n' +
          '        $gate withdraw "$current_generation" >/dev/null 2>&1 || {',
        "    if ! management_ready; then\n        true || {",
      ),
    ],
    [
      "child bypasses entrypoint",
      source.replace(
        "    /opt/keycloak/bin/keycloak-entrypoint \\\n",
        "    /bin/true \\\n",
      ),
    ],
    [
      "degraded-child withdrawal bypassed",
      source.replace(
        '            $gate withdraw "$current_generation" >/dev/null 2>&1 || true\n' +
          '            kill -TERM "$child_pid"',
        '            true\n' + '            kill -TERM "$child_pid"',
      ),
    ],
    [
      "post-child degraded withdrawal bypassed",
      source.replace(
        '    if [ "$dependency_degraded" = true ]; then\n' +
          "        # Convergence suppresses signals while its bounded cleanup settles\n" +
          "        # sessions and may have reached publication after the pre-kill\n" +
          "        # withdrawal. Close the still-current generation again after wait.\n" +
          '        $gate withdraw "$current_generation" >/dev/null 2>&1 || {',
        '    if [ "$dependency_degraded" = true ]; then\n        true || {',
      ),
    ],
    [
      "failed-generation withdrawal bypassed",
      source.replace(
        '    $gate withdraw "$current_generation" >/dev/null 2>&1 || {\n' +
          '        echo "keycloak-supervisor: failed generation could not be closed" >&2',
        '    true || {\n' +
          '        echo "keycloak-supervisor: failed generation could not be closed" >&2',
      ),
    ],
  ]) {
    assert.notEqual(mutated, source, `${name} mutant did not alter the fixture`);
    assert.ok(
      keycloakRealmSupervisorFindings(mutated).length > 0,
      `realm supervisor accepted ${name}`,
    );
  }
});

test("the Keycloak management health proof uses the bounded HTTP status contract", () => {
  const source = readFileSync(KEYCLOAK_HEALTH, "utf8");
  assert.deepEqual(keycloakHealthFindings(source), []);
  for (const [name, mutated] of [
    [
      "status widened",
      source.replace(
        '[[ "$status_line" =~ ^HTTP/1\\.[01]\\ 200\\  ]]',
        '[[ "$status_line" =~ ^HTTP/1\\.[01]\\ 2[0-9][0-9]\\  ]]',
      ),
    ],
    [
      "incomplete headers accepted",
      source.replace('[ "$headers_complete" = true ] || return 1', "true"),
    ],
    [
      "response bound widened",
      source.replace('"$response_size" -le 65536', '"$response_size" -le 65537'),
    ],
    [
      "body oracle added",
      source.replace(
        "validate_response() (\n",
        "validate_response() (\n    grep -q status \"$1\" || return 1\n",
      ),
    ],
    ["local host widened", source.replace("health_host=127.0.0.1", "health_host=0.0.0.0")],
    ["network host redirected", source.replace("health_host=keycloak", "health_host=localhost")],
    ["management port drifted", source.replace("/dev/tcp/$host/9000", "/dev/tcp/$host/8080")],
    ["management endpoint drifted", source.replace("GET /health/ready", "GET /health/live")],
    [
      "management timeout widened",
      source.replace("--kill-after=1s 4s \\", "--kill-after=1s 5s \\"),
    ],
  ]) {
    assert.notEqual(mutated, source, `${name} mutant did not alter the fixture`);
    assert.ok(
      keycloakHealthFindings(mutated).length > 0,
      `Keycloak health accepted ${name}`,
    );
  }
});

test("development uses one exact browser and container issuer port", () => {
  const source = readFileSync(COMPOSE_DEV, "utf8");
  assert.deepEqual(developmentPortBindingFindings(source), []);
  const hostOnlyTranslation = source.replace(
    "      - target: ${SYNVEDA_DEV_HTTP_PORT:-8080}",
    "      - target: 8080",
  );
  assert.notEqual(hostOnlyTranslation, source);
  assert.deepEqual(developmentPortBindingFindings(hostOnlyTranslation), [
    "development proxy does not bind one identical container and host port",
  ]);

  const fixture = makeComposeFixture();
  try {
    const output = join(fixture.scratch, "custom-development-port.json");
    writeFileSync(
      fixture.issuers,
      JSON.stringify([
        {
          issuer: "http://auth.synveda.test:18083/realms/synveda",
          client_id: "synveda",
          audience: "synveda-api",
          tenant: { static: { tenant_id: "00000000-0000-0000-0000-000000000001" } },
          login_scopes: ["openid", "profile", "email"],
        },
      ]),
      { mode: 0o600 },
    );
    const result = spawnSync(WRAPPER, ["config", "--output", output], {
      cwd: ROOT,
      env: composeEnvironment(fixture, { SYNVEDA_DEV_HTTP_PORT: "18083" }),
      encoding: "utf8",
    });
    assert.equal(result.status, 0, result.stderr);
    const model = JSON.parse(readFileSync(output, "utf8"));
    assert.deepEqual(model.services.proxy.ports, [
      {
        mode: "ingress",
        target: 18083,
        published: "18083",
        protocol: "tcp",
        host_ip: "127.0.0.1",
      },
    ]);
    assert.equal(model.services.proxy.environment.SYNVEDA_PUBLIC_PORT, "18083");
    assert.equal(model.services.proxy.environment.SYNVEDA_PROXY_HTTP_PORT, "18083");
    assert.equal(
      model.services.keycloak.environment.KC_HOSTNAME,
      "http://auth.synveda.test:18083",
    );
    assert.equal(
      model.services["issuer-diagnostic"].environment.SYNVEDA_OIDC_EXPECTED_ISSUER,
      "http://auth.synveda.test:18083/realms/synveda",
    );
  } finally {
    rmSync(fixture.scratch, { recursive: true, force: true });
  }
});

test("Keycloak convergence publishes only after bounded proof and cleanup", () => {
  const source = readFileSync(KEYCLOAK_CONVERGENCE, "utf8");
  assert.deepEqual(keycloakConvergenceFindings(source), []);

  const wrongAuditClient = source.replace(
    "--cclientid master-realm",
    "--cclientid realm-management",
  );
  assert.notEqual(wrongAuditClient, source);
  assert.ok(
    keycloakConvergenceFindings(wrongAuditClient).includes(
      "permanent authority audit client drifted",
    ),
  );

  const publication = "if publish_public_gate; then";
  const withoutPublication = source.replace(publication, "# publication moved");
  const publicationBeforeCleanup = withoutPublication.replace(
    '    if ! rm -f -- "$state_dir"/*; then',
    `    ${publication}\n        :\n    fi\n    if ! rm -f -- "$state_dir"/*; then`,
  );
  const withoutFinalAuthorization = replaceOccurrence(
    source,
    "publish_gate_on_exit=true",
    1,
    "# final publication authorization moved",
  );
  const authorizationBeforeProof = withoutFinalAuthorization.replace(
    'try_complete_projection "$authority_config" || {',
    'publish_gate_on_exit=true\ntry_complete_projection "$authority_config" || {',
  );

  const mutants = [
    [
      "late proof-function override",
      source.replace(
        "trap cleanup EXIT",
        "try_prove_scoped_authority() { return 0; }\ntrap cleanup EXIT",
      ),
    ],
    [
      "additive direct gate write",
      source.replace(
        "    unset bootstrap_password convergence_password\n",
        "    printf '%s\\n' \"$contract\" > \"$public_gate\"\n" +
          "    unset bootstrap_password convergence_password\n",
      ),
    ],
    [
      "alternate publication authorization",
      source.replace(
        "trap 'signal_exit 143' TERM\n",
        "trap 'signal_exit 143' TERM\n" +
          "publish_gate_on_exit=$(printf true)\nexit 0\n",
      ),
    ],
    [
      "gate withdrawal early return",
      source.replace(
        "withdraw_public_gate() {\n",
        "withdraw_public_gate() {\n    return 0\n",
      ),
    ],
    [
      "non-atomic gate publication",
      source.replace(
        '"$generation_gate" publish "$generation" >/dev/null 2>&1',
        'cp -- "$state_dir/realm.json" "$public_gate"',
      ),
    ],
    [
      "initial gate withdrawal",
      source.replace("withdraw_public_gate || {", "true || {"),
    ],
    ["EXIT trap", source.replace("trap cleanup EXIT", "# EXIT trap removed")],
    [
      "HUP trap",
      source.replace("trap 'signal_exit 129' HUP", "# HUP trap removed"),
    ],
    [
      "INT trap",
      source.replace("trap 'signal_exit 130' INT", "# INT trap removed"),
    ],
    [
      "TERM trap",
      source.replace("trap 'signal_exit 143' TERM", "# TERM trap removed"),
    ],
    [
      "cleanup signal suppression",
      source.replace("trap '' HUP INT TERM", "trap - HUP INT TERM"),
    ],
    [
      "EXIT trap released before signal suppression",
      source.replace(
        "trap '' HUP INT TERM\n    trap - EXIT",
        "trap - EXIT\n    trap '' HUP INT TERM",
      ),
    ],
    [
      "authenticated quarantine update",
      source.replace(
        'if run_admin_quarantine "$quarantine_config" "$state_dir"; then',
        "if true; then",
      ),
    ],
    [
      "authenticated quarantine early success",
      source.replace("\nquarantine() {\n", "\nquarantine() {\n    return 0\n"),
    ],
    [
      "fresh-authority quarantine early success",
      source.replace(
        "quarantine_with_fresh_authority() {\n",
        "quarantine_with_fresh_authority() {\n    return 0\n",
      ),
    ],
    [
      "fresh-authority quarantine proof",
      source.replace(
        'if run_admin_quarantine "$recovery_config" "$recovery_dir"; then',
        'if run_admin_quarantine "$permanent_config" "$recovery_dir"; then',
      ),
    ],
    ["private cleanup after publication", publicationBeforeCleanup],
    ["final authorization before open proof", authorizationBeforeProof],
    [
      "complete projection proof short-circuited",
      source.replace(
        'try_complete_projection "$authority_config" || {',
        'true || try_complete_projection "$authority_config" || {',
      ),
    ],
    [
      "scoped token proof short-circuited",
      source.replace(
        'try_project_quiet scoped-token admin-token "$scoped_config" || return',
        'true || try_project_quiet scoped-token admin-token "$scoped_config" || return',
      ),
    ],
    [
      "complete group proof short-circuited",
      source.replace(
        'try_project_quiet complete-group group "$state_dir/complete-groups.json" || return',
        'true || try_project_quiet complete-group group "$state_dir/complete-groups.json" || return',
      ),
    ],
    [
      "publication outside cleanup",
      `${source}\npublish_public_gate\n`,
    ],
    [
      "bootstrap negative response probe",
      source.replace(
        "SynvedaKeycloakProjection bootstrap-refused",
        "SynvedaKeycloakProjection admin-token",
      ),
    ],
    [
      "one-shot authority response probe",
      source.replace(
        'SynvedaKeycloakProjection admin-authority-login "$scoped_user_id" \\',
        'SynvedaKeycloakProjection admin-token "$scoped_user_id" \\',
      ),
    ],
    [
      "one-shot authority proof short-circuited",
      source.replace(
        "if KC_CLI_PASSWORD=$convergence_password \\",
        "if true || KC_CLI_PASSWORD=$convergence_password \\",
      ),
    ],
    [
      "one-shot authority subject",
      source.replace(
        'admin-authority-login "$scoped_user_id" \\',
        'admin-authority-login "$bootstrap_user_id" \\',
      ),
    ],
    [
      "one-shot authority issuer",
      source.replace(
        "SYNVEDA_PROBE_ISSUER=$public_auth_url/realms/master",
        "SYNVEDA_PROBE_ISSUER=$admin_url/realms/master",
      ),
    ],
    [
      "one-shot authority credential",
      source.replace(
        "KC_CLI_PASSWORD=$convergence_password",
        "KC_CLI_PASSWORD=$bootstrap_password",
      ),
    ],
    [
      "one-shot authority bootstrap identity environment",
      source.replace(
        "        SYNVEDA_PROBE_BOOTSTRAP_USERNAME=$bootstrap_username \\\n",
        "",
      ),
    ],
    [
      "one-shot authority raw status",
      source.replace("scoped_status=$?", "scoped_status=1"),
    ],
    [
      "one-shot authority stage classifier",
      source.replace(
        ". /opt/keycloak/bin/synveda-authority-stage",
        ". /tmp/unreviewed-authority-stage",
      ),
    ],
    [
      "one-shot authority capture cleanup",
      source.replace(
        'synveda_finish_scoped_authority_probe "$state_dir" "$scoped_status"',
        'return "$scoped_status"',
      ),
    ],
    [
      "one-shot authority command-scoped secret",
      source.replace(
        "    if KC_CLI_PASSWORD=$convergence_password \\\n",
        "    KC_CLI_PASSWORD=$convergence_password\n" +
          "    export KC_CLI_PASSWORD\n" +
          "    if /usr/bin/timeout --foreground --signal=TERM --kill-after=1s 55s \\\n",
      ),
    ],
    [
      "permanent authentication client",
      source.replace(
        "authentication_client=admin-cli",
        "authentication_client=other-client",
      ),
    ],
    [
      "custom proof-client residue",
      source.replace(
        "authentication_client=admin-cli",
        "authentication_client=admin-cli\nauthority_proof_client=synveda-convergence-proof",
      ),
    ],
    [
      "legacy custom authentication client",
      source.replace(
        "authentication_client=admin-cli",
        "authentication_client=admin-cli\nlegacy=synveda-convergence-cli",
      ),
    ],
    [
      "cleanup authentication bypass",
      source.replace(
        'if run_cleanup_kcadm "$cleanup_auth_config" config credentials \\',
        'if true || run_cleanup_kcadm "$cleanup_auth_config" config credentials \\',
      ),
    ],
    [
      "direct quarantine Java mode",
      source.replace(
        'SynvedaKeycloakProjection admin-quarantine "$quarantine_config" \\',
        'SynvedaKeycloakProjection bootstrap-refused "$quarantine_config" \\',
      ),
    ],
    [
      "retired bootstrap password guard",
      source.replace('[ "${bootstrap_password+x}" = x ] || {', ': || {'),
    ],
    [
      "permanent role projection short-circuited",
      source.replace(
        "try_project_quiet effective-target-roles effective-roles \\",
        "true || try_project_quiet effective-target-roles effective-roles \\",
      ),
    ],
    [
      "permanent audit role projection short-circuited",
      source.replace(
        "try_project_quiet effective-audit-role effective-audit-role \\",
        "true || try_project_quiet effective-audit-role effective-audit-role \\",
      ),
    ],
    [
      "target role assignment widened",
      source.replace(
        "--rolename manage-realm --rolename manage-clients --rolename manage-users",
        "--rolename manage-realm --rolename manage-clients --rolename manage-users --rolename manage-events",
      ),
    ],
    [
      "audit role assignment widened",
      source.replace("--rolename view-users", "--rolename manage-users"),
    ],
    [
      "audit client assignment drifted",
      source.replace("--cclientid master-realm", "--cclientid realm-management"),
    ],
    [
      "existing admin session close bypassed",
      source.replace(
        'close_admin_session "$auth_config" "$state_dir" || return 70',
        ": || return 70",
      ),
    ],
    [
      "failed authentication session settlement bypassed",
      source.replace(
        'settle_failed_admin_session "$auth_config" "$state_dir" || return 70',
        ": || return 70",
      ),
    ],
    [
      "cleanup session close bypassed",
      source.replace(
        'if close_admin_session "$session_config" "$state_dir"; then',
        "if true; then",
      ),
    ],
    [
      "bootstrap deletion implementation",
      source.replace(
        "project_quiet bootstrap-user-delete admin-bootstrap-delete \\",
        'admin_quiet "$bootstrap_config" bootstrap-user-delete delete \\',
      ),
    ],
    [
      "bootstrap deletion identity",
      source.replace(
        '"$bootstrap_config" "$bootstrap_user_id"\n    close_admin_session',
        '"$bootstrap_config" "$permanent_user_id"\n    close_admin_session',
      ),
    ],
    [
      "post-rename candidate verification",
      source.replace(
        "    fi\n    require_current_generation\n}\n\nrequire_current_generation || exit 75",
        "    fi\n    return 0\n}\n\nrequire_current_generation || exit 75",
      ),
    ],
  ];
  const digestFinding =
    "Keycloak convergence source differs from the reviewed executable";
  const expectedFindings = new Map([
    ["late proof-function override", "scoped convergence authority proof body drifted"],
    [
      "additive direct gate write",
      "realm convergence writes public gate outside atomic publication",
    ],
    [
      "alternate publication authorization",
      "realm convergence publication authorization grammar drifted",
    ],
    ["gate withdrawal early return", "public gate withdrawal body drifted"],
    ["non-atomic gate publication", "atomic public gate publication body drifted"],
    [
      "initial gate withdrawal",
      "realm convergence withdrawal can be short-circuited",
    ],
    ["EXIT trap", "realm convergence signal contract lacks trap cleanup EXIT"],
    [
      "HUP trap",
      "realm convergence signal contract lacks trap 'signal_exit 129' HUP",
    ],
    [
      "INT trap",
      "realm convergence signal contract lacks trap 'signal_exit 130' INT",
    ],
    [
      "TERM trap",
      "realm convergence signal contract lacks trap 'signal_exit 143' TERM",
    ],
    ["cleanup signal suppression", "cleanup signal suppression can be bypassed"],
    [
      "EXIT trap released before signal suppression",
      "realm convergence cleanup/quarantine/publication order drifted",
    ],
    [
      "authenticated quarantine update",
      "authenticated quarantine update can be short-circuited",
    ],
    ["authenticated quarantine early success", "authenticated quarantine body drifted"],
    [
      "fresh-authority quarantine early success",
      "fresh-authority quarantine body drifted",
    ],
    ["fresh-authority quarantine proof", "fresh-authority quarantine body drifted"],
    [
      "private cleanup after publication",
      "realm convergence cleanup/quarantine/publication order drifted",
    ],
    [
      "final authorization before open proof",
      "realm enablement is not fully proved before publication is authorized",
    ],
    [
      "complete projection proof short-circuited",
      "complete projection proof can be short-circuited",
    ],
    [
      "scoped token proof short-circuited",
      "scoped convergence authority proof body drifted",
    ],
    [
      "complete group proof short-circuited",
      "complete managed projection proof body drifted",
    ],
    [
      "publication outside cleanup",
      "realm convergence permits publication outside bounded cleanup",
    ],
    [
      "bootstrap negative response probe",
      "bootstrap retirement does not use the exact response probe",
    ],
    [
      "one-shot authority response probe",
      "scoped convergence authority proof body drifted",
    ],
    [
      "one-shot authority proof short-circuited",
      "scoped convergence authority proof body drifted",
    ],
    [
      "one-shot authority subject",
      "scoped convergence authority proof body drifted",
    ],
    [
      "one-shot authority issuer",
      "scoped convergence authority proof body drifted",
    ],
    [
      "one-shot authority credential",
      "scoped convergence authority proof body drifted",
    ],
    [
      "one-shot authority bootstrap identity environment",
      "scoped convergence authority proof body drifted",
    ],
    [
      "permanent authentication client",
      "permanent authority authentication client drifted",
    ],
    [
      "custom proof-client residue",
      "custom proof-only authority client residue remains reachable",
    ],
    [
      "legacy custom authentication client",
      "legacy convergence authentication client remains reachable",
    ],
    [
      "cleanup authentication bypass",
      "cleanup quarantine authentication body drifted",
    ],
    [
      "direct quarantine Java mode",
      "direct administrative quarantine body drifted",
    ],
    ["retired bootstrap password guard", "authenticated quarantine body drifted"],
    [
      "one-shot authority raw status",
      "scoped convergence authority proof body drifted",
    ],
    [
      "one-shot authority stage classifier",
      "scoped authority does not use the reviewed content-free stage classifier",
    ],
    [
      "one-shot authority capture cleanup",
      "scoped convergence authority proof body drifted",
    ],
    [
      "one-shot authority command-scoped secret",
      "scoped convergence authority proof body drifted",
    ],
    [
      "permanent role projection short-circuited",
      "permanent convergence authority projection body drifted",
    ],
    [
      "permanent audit role projection short-circuited",
      "permanent convergence authority projection body drifted",
    ],
    ["target role assignment widened", "permanent authority target role assignment drifted"],
    ["audit role assignment widened", "permanent authority audit role assignment drifted"],
    ["audit client assignment drifted", "permanent authority audit client drifted"],
    [
      "existing admin session close bypassed",
      "administrative authentication session lifecycle drifted",
    ],
    [
      "failed authentication session settlement bypassed",
      "administrative authentication session lifecycle drifted",
    ],
    [
      "cleanup session close bypassed",
      "realm convergence cleanup/quarantine/publication order drifted",
    ],
    [
      "bootstrap deletion implementation",
      "bootstrap retirement does not use the exact-ID administrative delete",
    ],
    [
      "bootstrap deletion identity",
      "bootstrap retirement is not exactly deleted and refused before its complete witness",
    ],
    ["post-rename candidate verification", "atomic public gate publication body drifted"],
  ]);
  for (const [name, mutated] of mutants) {
    assert.notEqual(mutated, source, `${name} mutant did not alter the fixture`);
    const expectedFinding = expectedFindings.get(name);
    assert.ok(expectedFinding, `${name} mutant lacks a named semantic finding`);
    const semanticFindings = keycloakConvergenceFindings(mutated).filter(
      (finding) => finding !== digestFinding,
    );
    assert.ok(
      semanticFindings.includes(expectedFinding),
      `convergence lifecycle accepted ${name}; findings: ${semanticFindings.join("; ")}`,
    );
  }
});

function fakeDocker(fixture) {
  const path = join(fixture.scratch, "docker");
  const argumentsFile = join(fixture.scratch, "docker-arguments");
  writeFileSync(
    path,
    `#!/bin/sh
[ -z "\${COMPOSE_PROFILES+x}" ] || exit 97
[ "\${COMPOSE_DISABLE_ENV_FILE:-}" = 1 ] || exit 98
if [ "$1" = compose ] && [ "$2" = version ]; then
  echo "\${SYNVEDA_FAKE_COMPOSE_VERSION:-2.33.1}"
  exit 0
fi
if [ -n "\${SYNVEDA_FAKE_DOCKER_ENVIRONMENT_FILE:-}" ]; then
  printf '%s\n%s\n' \
    "$SYNVEDA_DATABASE_AUTHORITY_DIR" \
    "$SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR" \
    > "$SYNVEDA_FAKE_DOCKER_ENVIRONMENT_FILE"
fi
printf '%s\\n' "$@" > "$SYNVEDA_FAKE_DOCKER_ARGUMENTS"
`,
    { mode: 0o700 },
  );
  chmodSync(path, 0o700);
  return { path, argumentsFile };
}

test("the selector requires Compose 2.33.1 for gateway priority", () => {
  for (const version of ["2.32.9", "2.33.0", "not-a-version"]) {
    const fixture = makeComposeFixture();
    try {
      const fake = fakeDocker(fixture);
      const result = spawnSync(WRAPPER, ["config"], {
        cwd: ROOT,
        env: composeEnvironment(fixture, {
          SYNVEDA_DOCKER_BIN: fake.path,
          SYNVEDA_FAKE_DOCKER_ARGUMENTS: fake.argumentsFile,
          SYNVEDA_FAKE_COMPOSE_VERSION: version,
        }),
        encoding: "utf8",
      });
      assert.equal(result.status, 69, `${version}: ${result.stderr}`);
      assert.match(result.stderr, /Docker Compose/);
    } finally {
      rmSync(fixture.scratch, { recursive: true, force: true });
    }
  }
});

test("the selector builds one exact external-Postgres/bundled-Keycloak file set", () => {
  const fixture = makeComposeFixture();
  try {
    const fake = fakeDocker(fixture);
    const output = execFileSync(WRAPPER, ["config"], {
      cwd: ROOT,
      env: composeEnvironment(fixture, {
        SYNVEDA_DOCKER_BIN: fake.path,
        SYNVEDA_FAKE_DOCKER_ARGUMENTS: fake.argumentsFile,
        SYNVEDA_POSTGRES_MODE: "external",
        SYNVEDA_OIDC_MODE: "bundled",
        SYNVEDA_POSTGRES_BOOTSTRAP_URL:
          "postgresql://bootstrap@database.compose.example:5432/postgres",
        SYNVEDA_KEYCLOAK_DATABASE_URL:
          "jdbc:postgresql://database.compose.example:5432/keycloak",
      }),
      encoding: "utf8",
    });
    assert.match(output, /synveda-development \(external PostgreSQL, bundled OIDC\)/);
    const args = readFileSync(fake.argumentsFile, "utf8").trim().split("\n");
    const selected = args.filter((value, index) => args[index - 1] === "-f");
    assert.deepEqual(selected, [
      join(COMPOSE, "compose.yaml"),
      join(COMPOSE, "compose.dev.yaml"),
      join(COMPOSE, "compose.keycloak.yaml"),
      join(COMPOSE, "compose.keycloak-external-postgres.yaml"),
      join(COMPOSE, "compose.external-postgres.yaml"),
      join(COMPOSE, "compose.external.yaml"),
    ]);
    assert.equal(args[args.indexOf("-p") + 1], "synveda-development");
    assert.deepEqual(args.slice(-2), ["config", "--quiet"]);
  } finally {
    rmSync(fixture.scratch, { recursive: true, force: true });
  }
});

test("acceptance projects receive distinct authority and public-gate state", () => {
  const fixture = makeComposeFixture();
  try {
    const fake = fakeDocker(fixture);
    const resolved = [];
    for (const suffix of ["acceptance-alpha", "acceptance-beta"]) {
      const environmentFile = join(fixture.scratch, `${suffix}.environment`);
      const result = spawnSync(WRAPPER, ["config"], {
        cwd: ROOT,
        env: composeEnvironment(fixture, {
          SYNVEDA_COMPOSE_PROJECT_SUFFIX: suffix,
          SYNVEDA_DOCKER_BIN: fake.path,
          SYNVEDA_FAKE_DOCKER_ARGUMENTS: fake.argumentsFile,
          SYNVEDA_FAKE_DOCKER_ENVIRONMENT_FILE: environmentFile,
        }),
        encoding: "utf8",
      });
      assert.equal(result.status, 0, result.stderr);
      const [authority, gate] = readFileSync(environmentFile, "utf8").trim().split("\n");
      assert.match(authority, new RegExp(`/synveda-development-${suffix}/database-authority$`));
      assert.match(gate, new RegExp(`/synveda-development-${suffix}/keycloak-public-gate$`));
      resolved.push({ authority, gate });
    }
    assert.notEqual(resolved[0].authority, resolved[1].authority);
    assert.notEqual(resolved[0].gate, resolved[1].gate);
  } finally {
    rmSync(fixture.scratch, { recursive: true, force: true });
  }
});

test("reference and acceptance projects require an explicit IPv4 pool", () => {
  const fixture = makeComposeFixture();
  try {
    const fake = fakeDocker(fixture);
    for (const overrides of [
      {
        SYNVEDA_COMPOSE_RUNTIME: "development",
        SYNVEDA_COMPOSE_PROJECT_SUFFIX: "acceptance-pool-proof",
      },
      {
        SYNVEDA_COMPOSE_RUNTIME: "reference",
        SYNVEDA_PUBLIC_SCHEME: "https",
        SYNVEDA_APP_HOST: "app.compose.example",
        SYNVEDA_AUTH_HOST: "auth.compose.example",
      },
    ]) {
      const environment = composeEnvironment(fixture, {
        SYNVEDA_DOCKER_BIN: fake.path,
        SYNVEDA_FAKE_DOCKER_ARGUMENTS: fake.argumentsFile,
        ...overrides,
      });
      delete environment.SYNVEDA_COMPOSE_IPV4_POOL;
      const result = spawnSync(WRAPPER, ["config"], {
        cwd: ROOT,
        env: environment,
        encoding: "utf8",
      });
      assert.equal(result.status, 64, result.stderr);
      assert.match(result.stderr, /require an explicit SYNVEDA_COMPOSE_IPV4_POOL/);
    }
    assert.equal(existsSync(fake.argumentsFile), false);
  } finally {
    rmSync(fixture.scratch, { recursive: true, force: true });
  }
});

test("external PostgreSQL requires an explicit topology-specific role contract", () => {
  const fixture = makeComposeFixture();
  try {
    const fake = fakeDocker(fixture);
    const environment = composeEnvironment(fixture, {
      SYNVEDA_DOCKER_BIN: fake.path,
      SYNVEDA_FAKE_DOCKER_ARGUMENTS: fake.argumentsFile,
      SYNVEDA_POSTGRES_MODE: "external",
      SYNVEDA_OIDC_MODE: "external",
      SYNVEDA_OIDC_ISSUER: "https://external-idp.example/tenant",
    });
    delete environment.SYNVEDA_DATABASE_ROLES_FILE;
    const result = spawnSync(WRAPPER, ["config"], {
      cwd: ROOT,
      env: environment,
      encoding: "utf8",
    });
    assert.equal(result.status, 78);
    assert.match(result.stderr, /external PostgreSQL requires an explicit topology-specific/);
    assert.equal(existsSync(fake.argumentsFile), false);
  } finally {
    rmSync(fixture.scratch, { recursive: true, force: true });
  }
});

test("bundled role contracts declare the exact existing database topology", () => {
  const reference = JSON.parse(
    readFileSync(join(COMPOSE, "configs/database/roles.reference.json"), "utf8"),
  );
  const externalOidc = JSON.parse(
    readFileSync(join(COMPOSE, "configs/database/roles.external-oidc.json"), "utf8"),
  );
  assert.deepEqual(reference.forbidden_databases, ["keycloak", "postgres", "template1"]);
  assert.deepEqual(reference.isolated_peer_roles, ["keycloak"]);
  assert.deepEqual(externalOidc.forbidden_databases, ["postgres", "template1"]);
  assert.deepEqual(externalOidc.isolated_peer_roles, []);
});

test("the selector rejects unsafe shape before invoking Docker", () => {
  const fixture = makeComposeFixture();
  try {
    const fake = fakeDocker(fixture);
    for (const [key, value, expected] of [
      ["SYNVEDA_COMPOSE_RUNTIME", "compose", "development|reference"],
      ["SYNVEDA_POSTGRES_MODE", "automatic", "bundled|external"],
      ["SYNVEDA_OIDC_MODE", "rauthy", "bundled|external"],
      ["SYNVEDA_COMPOSE_PROFILES", "deployed", "unsupported profile"],
      ["SYNVEDA_RUNTIME_UID", "0", "non-zero decimal integers"],
      ["SYNVEDA_RUNTIME_UID", "00", "non-zero decimal integers"],
      ["SYNVEDA_RUNTIME_GID", "020", "non-zero decimal integers"],
      ["SYNVEDA_COMPOSE_PROJECT_SUFFIX", "yes", "project suffix"],
      ["SYNVEDA_COMPOSE_PROJECT_SUFFIX", "acceptance-aa/../../x", "project suffix"],
      ["SYNVEDA_COMPOSE_PROJECT_SUFFIX", "acceptance-ok\ninvalid", "project suffix"],
      ["SYNVEDA_COMPOSE_PROJECT_SUFFIX", "acceptance-ä", "project suffix"],
      ["SYNVEDA_APP_HOST", "localhost", "lower-case DNS names"],
      ["SYNVEDA_APP_HOST", "app..synveda.test", "lower-case DNS names"],
      ["SYNVEDA_APP_HOST", "app.synveda.test\ninvalid", "lower-case DNS names"],
      ["SYNVEDA_APP_HOST", "äpp.synveda.test", "lower-case DNS names"],
      ["SYNVEDA_AUTH_HOST", "auth-.synveda.test", "lower-case DNS names"],
      ["SYNVEDA_DEV_HTTP_PORT", "00", "canonical integer"],
      ["SYNVEDA_DEV_HTTP_PORT", "1023", "1024 through 65535"],
      ["SYNVEDA_DEV_HTTP_PORT", "8443", "reserved port 8443"],
      ["SYNVEDA_DEV_HTTP_PORT", "65536", "canonical integer"],
      ["SYNVEDA_COMPOSE_IPV4_POOL", "10.foo.bar.0/24", "canonical private IPv4"],
      ["SYNVEDA_COMPOSE_IPV4_POOL", "172.30.240.0./24", "canonical private IPv4"],
      ["SYNVEDA_COMPOSE_IPV4_POOL", "172.32.0.0/24", "must be private"],
      ["SYNVEDA_COMPOSE_IPV4_POOL", "192.168.001.0/24", "canonical private IPv4"],
      ["SYNVEDA_COMPOSE_IPV4_POOL", "172.30.240.1/24", "canonical private IPv4"],
      ["SYNVEDA_COMPOSE_IPV4_POOL", "172.30.240.0/23", "private /24"],
      ["SYNVEDA_COMPOSE_IPV4_POOL", "172.30.240.0/25", "private /24"],
      ["SYNVEDA_COMPOSE_IPV4_POOL", "2001:db8::/24", "canonical private IPv4"],
      [
        "SYNVEDA_OTEL_COLLECTOR_IMAGE",
        `collector.example/otel@sha256:${"1".repeat(64)}\nunpinned:latest`,
        "closed OCI reference",
      ],
      ["SYNVEDA_PRODUCT_IMAGE", "répo.example/product:dev", "closed OCI reference"],
    ]) {
      const result = spawnSync(WRAPPER, ["config"], {
        cwd: ROOT,
        env: composeEnvironment(fixture, {
          SYNVEDA_DOCKER_BIN: fake.path,
          SYNVEDA_FAKE_DOCKER_ARGUMENTS: fake.argumentsFile,
          [key]: value,
        }),
        encoding: "utf8",
      });
      assert.equal(result.status, 64, `${key}: ${result.stderr}`);
      assert.match(result.stderr, new RegExp(expected.replace("|", "\\|")));
    }
  } finally {
    rmSync(fixture.scratch, { recursive: true, force: true });
  }
});

test("the selector rejects unsafe external issuer URLs without echoing input", () => {
  const fixture = makeComposeFixture();
  try {
    const fake = fakeDocker(fixture);
    for (const issuer of [
      "https://external-idp.example/tenant?token=cpr45-issuer-sentinel",
      "https://user:cpr45-issuer-sentinel@external-idp.example/tenant",
      "https://external-idp.example/tenant#cpr45-issuer-sentinel",
      "https://external-idp.example/tenant cpr45-issuer-sentinel",
    ]) {
      const result = spawnSync(WRAPPER, ["config"], {
        cwd: ROOT,
        env: composeEnvironment(fixture, {
          SYNVEDA_DOCKER_BIN: fake.path,
          SYNVEDA_FAKE_DOCKER_ARGUMENTS: fake.argumentsFile,
          SYNVEDA_OIDC_MODE: "external",
          SYNVEDA_OIDC_ISSUER: issuer,
        }),
        encoding: "utf8",
      });
      assert.equal(result.status, 64, result.stderr);
      assert.match(result.stderr, /credentials, whitespace, a query or a fragment/);
      assert.doesNotMatch(`${result.stdout}${result.stderr}`, /cpr45-issuer-sentinel/);
      assert.equal(existsSync(fake.argumentsFile), false);
    }
  } finally {
    rmSync(fixture.scratch, { recursive: true, force: true });
  }
});

test("the selector rejects direct secrets and permissive secret files", () => {
  const fixture = makeComposeFixture();
  try {
    const fake = fakeDocker(fixture);
    let result;
    for (const setting of [
      "DATABASE_URL",
      "SYNVEDA_MIGRATOR_DATABASE_URL",
      "SYNVEDA_GATEWAY_DATABASE_URL",
      "SYNVEDA_WORKER_DATABASE_URL",
      "SYNVEDA_KMS_KEY",
      "SYNVEDA_KMS_KEY_REF",
      "POSTGRES_PASSWORD",
      "KC_DB_PASSWORD",
      "KC_BOOTSTRAP_ADMIN_USERNAME",
      "KC_BOOTSTRAP_ADMIN_PASSWORD",
      "SYNVEDA_KEYCLOAK_CONVERGENCE_PASSWORD",
    ]) {
      result = spawnSync(WRAPPER, ["config"], {
        cwd: ROOT,
        env: composeEnvironment(fixture, {
          SYNVEDA_DOCKER_BIN: fake.path,
          SYNVEDA_FAKE_DOCKER_ARGUMENTS: fake.argumentsFile,
          [setting]: "postgres://secret-sentinel",
        }),
        encoding: "utf8",
      });
      assert.equal(result.status, 78);
      assert.doesNotMatch(result.stderr, /secret-sentinel/);
      assert.match(result.stderr, new RegExp(`direct secret setting ${setting} is forbidden`));
    }

    chmodSync(join(fixture.secrets, "synveda_gateway_database_url"), 0o640);
    result = spawnSync(WRAPPER, ["config"], {
      cwd: ROOT,
      env: composeEnvironment(fixture, {
        SYNVEDA_DOCKER_BIN: fake.path,
        SYNVEDA_FAKE_DOCKER_ARGUMENTS: fake.argumentsFile,
      }),
      encoding: "utf8",
    });
    assert.equal(result.status, 78);
    assert.match(result.stderr, /synveda_gateway_database_url file must have mode 0600/);

    chmodSync(join(fixture.secrets, "synveda_gateway_database_url"), 0o600);
    chmodSync(join(fixture.secrets, "keycloak_convergence_admin_password"), 0o640);
    result = spawnSync(WRAPPER, ["config"], {
      cwd: ROOT,
      env: composeEnvironment(fixture, {
        SYNVEDA_DOCKER_BIN: fake.path,
        SYNVEDA_FAKE_DOCKER_ARGUMENTS: fake.argumentsFile,
      }),
      encoding: "utf8",
    });
    assert.equal(result.status, 78);
    assert.match(
      result.stderr,
      /keycloak_convergence_admin_password file must have mode 0600/,
    );
  } finally {
    rmSync(fixture.scratch, { recursive: true, force: true });
  }
});

test("the selector rejects aliases between secrets, authority state and issuer input", () => {
  const fixture = makeComposeFixture();
  try {
    const fake = fakeDocker(fixture);
    const baseEnvironment = composeEnvironment(fixture, {
      SYNVEDA_DOCKER_BIN: fake.path,
      SYNVEDA_FAKE_DOCKER_ARGUMENTS: fake.argumentsFile,
    });
    const gate = baseEnvironment.SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR;
    for (const name of readdirSync(fixture.secrets)) {
      const source = join(fixture.secrets, name);
      const target = join(gate, name);
      if (statSync(source).isDirectory()) {
        mkdirSync(target, { mode: 0o700 });
        for (const child of readdirSync(source)) {
          const childSource = join(source, child);
          assert.ok(statSync(childSource).isFile());
          const childTarget = join(target, child);
          writeFileSync(childTarget, readFileSync(childSource), { mode: 0o600 });
          chmodSync(childTarget, 0o600);
        }
      } else {
        writeFileSync(target, readFileSync(source), { mode: 0o600 });
        chmodSync(target, 0o600);
      }
    }
    let result = spawnSync(WRAPPER, ["config"], {
      cwd: ROOT,
      env: { ...baseEnvironment, SYNVEDA_SECRETS_DIR: gate },
      encoding: "utf8",
    });
    assert.equal(result.status, 78);
    assert.match(result.stderr, /secret-and-keycloak-public-gate directories/);
    assert.equal(existsSync(fake.argumentsFile), false);

    const aliasedIssuer = join(gate, "issuers.json");
    writeFileSync(aliasedIssuer, readFileSync(fixture.issuers), { mode: 0o600 });
    chmodSync(aliasedIssuer, 0o600);
    result = spawnSync(WRAPPER, ["config"], {
      cwd: ROOT,
      env: { ...baseEnvironment, SYNVEDA_OIDC_ISSUERS_FILE: aliasedIssuer },
      encoding: "utf8",
    });
    assert.equal(result.status, 78);
    assert.match(result.stderr, /issuer-configuration file must not be inside/);
    assert.equal(existsSync(fake.argumentsFile), false);
  } finally {
    rmSync(fixture.scratch, { recursive: true, force: true });
  }
});

test("the selector keeps private inputs out of the development build context", () => {
  const fixture = makeComposeFixture();
  try {
    const fake = fakeDocker(fixture);
    const forbiddenRoot = join(ROOT, "cpr45-build-context-sentinel");
    for (const override of [
      { SYNVEDA_SECRETS_DIR: join(forbiddenRoot, "secrets") },
      { SYNVEDA_OIDC_ISSUERS_FILE: join(forbiddenRoot, "issuers.json") },
      {
        SYNVEDA_DATABASE_AUTHORITY_DIR: join(
          forbiddenRoot,
          "synveda-development",
          "database-authority",
        ),
      },
      {
        SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR: join(
          forbiddenRoot,
          "synveda-development",
          "keycloak-public-gate",
        ),
      },
    ]) {
      const result = spawnSync(WRAPPER, ["config"], {
        cwd: ROOT,
        env: composeEnvironment(fixture, {
          SYNVEDA_DOCKER_BIN: fake.path,
          SYNVEDA_FAKE_DOCKER_ARGUMENTS: fake.argumentsFile,
          ...override,
        }),
        encoding: "utf8",
      });
      assert.equal(result.status, 78, result.stderr);
      assert.match(result.stderr, /outside the Docker build context/);
      assert.doesNotMatch(`${result.stdout}${result.stderr}`, /cpr45-build-context-sentinel/);
      assert.equal(existsSync(fake.argumentsFile), false);
      assert.equal(existsSync(forbiddenRoot), false);
    }
  } finally {
    rmSync(fixture.scratch, { recursive: true, force: true });
  }
});

test("the selector normalizes the Keycloak database endpoint without leaking input", () => {
  const fixture = makeComposeFixture();
  try {
    const fake = fakeDocker(fixture);
    for (const overrides of [
      {
        SYNVEDA_KEYCLOAK_DATABASE_URL:
          "jdbc:postgresql://user:cpr45-jdbc-sentinel@database.example:5432/keycloak",
      },
      {
        SYNVEDA_POSTGRES_MODE: "external",
        SYNVEDA_KEYCLOAK_DATABASE_URL:
          "jdbc:postgresql://database.example:5432/keycloak?password=cpr45-jdbc-sentinel",
      },
      {
        SYNVEDA_POSTGRES_MODE: "external",
        SYNVEDA_KEYCLOAK_DATABASE_URL:
          "jdbc:postgresql://database.example\ninvalid:5432/keycloak",
      },
      {
        SYNVEDA_POSTGRES_MODE: "external",
        SYNVEDA_KEYCLOAK_DATABASE_URL:
          "jdbc:postgresql://database.example:5432/keycloak\ninvalid!",
      },
      {
        SYNVEDA_POSTGRES_MODE: "external",
        SYNVEDA_KEYCLOAK_DATABASE_URL: "jdbc:postgresql://database.example:5432/kéycloak",
      },
      {
        SYNVEDA_OIDC_MODE: "external",
        SYNVEDA_OIDC_ISSUER: "https://external-idp.example/tenant",
        SYNVEDA_KEYCLOAK_DATABASE_URL:
          "jdbc:postgresql://database.example:5432/cpr45-jdbc-sentinel",
      },
    ]) {
      const result = spawnSync(WRAPPER, ["config"], {
        cwd: ROOT,
        env: composeEnvironment(fixture, {
          SYNVEDA_DOCKER_BIN: fake.path,
          SYNVEDA_FAKE_DOCKER_ARGUMENTS: fake.argumentsFile,
          ...overrides,
        }),
        encoding: "utf8",
      });
      assert.equal(result.status, 64, result.stderr);
      assert.doesNotMatch(`${result.stdout}${result.stderr}`, /cpr45-jdbc-sentinel/);
    }
  } finally {
    rmSync(fixture.scratch, { recursive: true, force: true });
  }
});

test("the static selector bounds but does not interpret issuer configuration", () => {
  const fixture = makeComposeFixture();
  try {
    writeFileSync(fixture.issuers, '{"decoy":{"issuer":"not-the-selected-issuer"}}\n', {
      mode: 0o600,
    });
    chmodSync(fixture.issuers, 0o600);
    const fake = fakeDocker(fixture);
    const result = spawnSync(WRAPPER, ["config"], {
      cwd: ROOT,
      env: composeEnvironment(fixture, {
        SYNVEDA_DOCKER_BIN: fake.path,
        SYNVEDA_FAKE_DOCKER_ARGUMENTS: fake.argumentsFile,
      }),
      encoding: "utf8",
    });
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /configuration valid/);
  } finally {
    rmSync(fixture.scratch, { recursive: true, force: true });
  }
});

test("the secret generator is private, content-free and overwrite-safe", () => {
  const scratch = realpathSync(mkdtempSync(join(tmpdir(), "synveda-secret-generator-")));
  const secrets = join(scratch, "secrets");
  const tracedRoot = join(scratch, "traced");
  const tracedSecrets = join(tracedRoot, "secrets");
  const authority = join(scratch, "synveda-development", "database-authority");
  const tracedAuthority = join(
    tracedRoot,
    "synveda-development",
    "database-authority",
  );
  const gate = join(scratch, "synveda-development", "keycloak-public-gate");
  const tracedGate = join(
    tracedRoot,
    "synveda-development",
    "keycloak-public-gate",
  );
  const aliasRoot = join(scratch, "alias");
  const aliasAuthority = join(
    aliasRoot,
    "synveda-development",
    "database-authority",
  );
  const aliasGate = join(
    aliasRoot,
    "synveda-development",
    "keycloak-public-gate",
  );
  try {
    const aliasRefusal = spawnSync(GENERATOR, [], {
      cwd: ROOT,
      env: {
        ...process.env,
        SYNVEDA_SECRETS_DIR: aliasGate,
        SYNVEDA_DATABASE_AUTHORITY_DIR: aliasAuthority,
        SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR: aliasGate,
      },
      encoding: "utf8",
    });
    assert.equal(aliasRefusal.status, 73);
    assert.match(aliasRefusal.stderr, /dedicated secrets leaf/);
    assert.equal(existsSync(aliasGate), false);

    const firstResult = spawnSync(GENERATOR, [], {
      cwd: ROOT,
      env: {
        ...process.env,
        SYNVEDA_SECRETS_DIR: secrets,
        SYNVEDA_DATABASE_AUTHORITY_DIR: authority,
        SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR: gate,
      },
      encoding: "utf8",
    });
    assert.equal(firstResult.status, 0, firstResult.stderr);
    const first = `${firstResult.stdout}${firstResult.stderr}`;
    const files = readdirSync(secrets)
      .filter((name) => !name.startsWith(".") && statSync(join(secrets, name)).isFile())
      .sort();
    assert.equal(files.length, 13);
    for (const name of files) {
      const value = readFileSync(join(secrets, name), "utf8").trim();
      assert.ok(value.length > 0, `${name} is empty`);
      assert.ok(!first.includes(value), `${name} value reached stdout`);
      assert.equal(statSync(join(secrets, name)).mode & 0o777, 0o600);
      assert.match(first, new RegExp(`generated ${name}(?:\\n|$)`));
    }
    assert.equal(statSync(secrets).mode & 0o777, 0o700);
    assert.equal(statSync(authority).mode & 0o777, 0o700);
    assert.equal(statSync(gate).mode & 0o777, 0o700);
    assert.equal(statSync(join(secrets, "oidc-directory")).mode & 0o777, 0o700);
    assert.equal(statSync(join(secrets, ".synveda-private-directory")).mode & 0o777, 0o600);

    mkdirSync(tracedRoot, { mode: 0o700 });
    const traced = spawnSync("/bin/sh", ["-x", GENERATOR], {
      cwd: ROOT,
      env: {
        ...process.env,
        SYNVEDA_SECRETS_DIR: tracedSecrets,
        SYNVEDA_DATABASE_AUTHORITY_DIR: tracedAuthority,
        SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR: tracedGate,
      },
      encoding: "utf8",
    });
    assert.equal(traced.status, 0, traced.stderr);
    assert.match(traced.stderr, /(?:^|\n)\+ set \+x(?:\n|$)/);
    const traceOutput = `${traced.stdout}${traced.stderr}`;
    for (const name of readdirSync(tracedSecrets).filter(
      (entry) => !entry.startsWith(".") && statSync(join(tracedSecrets, entry)).isFile(),
    )) {
      const value = readFileSync(join(tracedSecrets, name), "utf8").trim();
      assert.ok(value.length > 0, `${name} is empty after traced execution`);
      assert.ok(!traceOutput.includes(value), `${name} value reached shell trace`);
    }

    const before = readFileSync(join(secrets, "synveda_kms_key"), "utf8");
    const refusal = spawnSync(GENERATOR, [], {
      cwd: ROOT,
      env: {
        ...process.env,
        SYNVEDA_SECRETS_DIR: secrets,
        SYNVEDA_DATABASE_AUTHORITY_DIR: authority,
        SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR: gate,
      },
      encoding: "utf8",
    });
    assert.equal(refusal.status, 73);
    assert.match(refusal.stderr, /refusing to replace an existing secret set/);
    assert.equal(readFileSync(join(secrets, "synveda_kms_key"), "utf8"), before);

    const unconfirmed = spawnSync(GENERATOR, ["--force"], {
      cwd: ROOT,
      env: {
        ...process.env,
        SYNVEDA_SECRETS_DIR: secrets,
        SYNVEDA_DATABASE_AUTHORITY_DIR: authority,
        SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR: gate,
      },
      encoding: "utf8",
    });
    assert.equal(unconfirmed.status, 73);
    assert.match(unconfirmed.stderr, /SYNVEDA_CONFIRM_SECRET_REPLACEMENT=synveda-development/);
    assert.equal(readFileSync(join(secrets, "synveda_kms_key"), "utf8"), before);

    const forced = spawnSync(GENERATOR, ["--force"], {
      cwd: ROOT,
      env: {
        ...process.env,
        SYNVEDA_SECRETS_DIR: secrets,
        SYNVEDA_DATABASE_AUTHORITY_DIR: authority,
        SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR: gate,
        SYNVEDA_CONFIRM_SECRET_REPLACEMENT: "synveda-development",
      },
      encoding: "utf8",
    });
    assert.equal(forced.status, 0, forced.stderr);
    const after = readFileSync(join(secrets, "synveda_kms_key"), "utf8");
    assert.notEqual(after, before);
    assert.ok(!`${forced.stdout}${forced.stderr}`.includes(after.trim()));
    assert.match(forced.stdout, /preserved previous secret set/);
    assert.equal(
      readFileSync(
        join(scratch, "synveda-development", "previous-secrets", "synveda_kms_key"),
        "utf8",
      ),
      before,
    );
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("the secret generator refuses broad, shared and symlinked targets before mutation", () => {
  const scratch = realpathSync(mkdtempSync(join(tmpdir(), "synveda-secret-path-")));
  const authority = join(scratch, "state", "synveda-development", "database-authority");
  const gate = join(scratch, "state", "synveda-development", "keycloak-public-gate");
  const invoke = (secrets, authorityPath = authority, gatePath = gate) =>
    spawnSync(GENERATOR, [], {
      cwd: ROOT,
      env: {
        ...process.env,
        SYNVEDA_SECRETS_DIR: secrets,
        SYNVEDA_DATABASE_AUTHORITY_DIR: authorityPath,
        SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR: gatePath,
      },
      encoding: "utf8",
    });
  try {
    for (const target of ["/", ROOT, COMPOSE]) {
      const before = statSync(target);
      const result = invoke(target);
      assert.equal(result.status, 73, result.stderr);
      const after = statSync(target);
      assert.equal(after.mode, before.mode);
      assert.equal(after.ino, before.ino);
      assert.equal(existsSync(authority), false);
      assert.equal(existsSync(gate), false);
    }

    const sharedRoot = join(scratch, "shared");
    const sharedSecrets = join(sharedRoot, "secrets");
    mkdirSync(sharedSecrets, { recursive: true, mode: 0o750 });
    chmodSync(sharedSecrets, 0o750);
    const sharedBefore = statSync(sharedSecrets);
    const sharedResult = invoke(sharedSecrets);
    assert.equal(sharedResult.status, 73, sharedResult.stderr);
    assert.match(sharedResult.stderr, /metadata was refused/);
    const sharedAfter = statSync(sharedSecrets);
    assert.equal(sharedAfter.mode, sharedBefore.mode);
    assert.equal(sharedAfter.ino, sharedBefore.ino);
    assert.deepEqual(readdirSync(sharedSecrets), []);
    assert.equal(existsSync(authority), false);
    assert.equal(existsSync(gate), false);

    const overlapRoot = join(scratch, "overlap");
    const overlapSecrets = join(overlapRoot, "secrets");
    const overlapAuthority = join(
      overlapSecrets,
      "synveda-development",
      "database-authority",
    );
    const overlapGate = join(
      overlapRoot,
      "synveda-development",
      "keycloak-public-gate",
    );
    const overlapResult = invoke(overlapSecrets, overlapAuthority, overlapGate);
    assert.equal(overlapResult.status, 73, overlapResult.stderr);
    assert.match(overlapResult.stderr, /secret-and-database-authority directories/);
    assert.equal(existsSync(overlapSecrets), false);
    assert.equal(existsSync(overlapGate), false);

    const realAncestor = join(scratch, "real-ancestor");
    const linkedAncestor = join(scratch, "linked-ancestor");
    mkdirSync(realAncestor, { mode: 0o700 });
    symlinkSync(realAncestor, linkedAncestor, "dir");
    const symlinkResult = invoke(join(linkedAncestor, "secrets"));
    assert.equal(symlinkResult.status, 73, symlinkResult.stderr);
    assert.match(symlinkResult.stderr, /ancestors must not be symlinks/);
    assert.deepEqual(readdirSync(realAncestor), []);
    assert.equal(existsSync(authority), false);
    assert.equal(existsSync(gate), false);

    const inContextResult = invoke(join(ROOT, "private", "secrets"));
    assert.equal(inContextResult.status, 73, inContextResult.stderr);
    assert.match(inContextResult.stderr, /ignored Compose roots/);
    assert.equal(existsSync(join(ROOT, "private")), false);
    assert.equal(existsSync(authority), false);
    assert.equal(existsSync(gate), false);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("the Keycloak entrypoint reads bounded files and rejects direct ambiguity", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-keycloak-entrypoint-"));
  try {
	    const snapshotPrefix = "keycloak-entrypoint.";
	    const snapshotsBefore = new Set(
	      readdirSync("/tmp").filter((entry) => entry.startsWith(snapshotPrefix)),
	    );
	    const assertSnapshotsClean = () => {
	      assert.deepEqual(
	        new Set(readdirSync("/tmp").filter((entry) => entry.startsWith(snapshotPrefix))),
	        snapshotsBefore,
	        "Keycloak entrypoint retained a private secret snapshot",
	      );
	    };
	    const source = readFileSync(KEYCLOAK_ENTRYPOINT, "utf8");
	    assert.doesNotMatch(source, /exec 3< "\$path"|head -c 4097/);
	    assert.match(source, /rm -f -- "\$secret_snapshot" \|\| cleanup_status=1/);
	    assert.match(source, /cleanup_secret_snapshot \|\| \{\n\s*echo [^\n]+\n\s*exit 70/);
	    assert.match(
	      source,
	      /\/usr\/bin\/timeout --foreground --signal=TERM --kill-after=1s 5s \\\n+\s*\/opt\/keycloak\/bin\/synveda-input-snapshot "\$path" "\$secret_snapshot"/,
	    );
	    const snapshotHelper = join(scratch, "synveda-input-snapshot");
	    execFileSync("cc", [
	      "-std=c11",
	      "-O2",
	      "-Wall",
	      "-Wextra",
	      "-Werror",
	      INPUT_SNAPSHOT,
	      "-o",
	      snapshotHelper,
	    ]);
	    chmodSync(snapshotHelper, 0o700);
	    const fakeTimeout = join(scratch, "timeout");
	    writeFileSync(
	      fakeTimeout,
	      `#!/bin/sh
[ "$1" = --foreground ] && [ "$2" = --signal=TERM ] && \
  [ "$3" = --kill-after=1s ] && [ "$4" = 5s ] || exit 96
shift 4
exec "$@"
`,
	      { mode: 0o700 },
	    );
	    chmodSync(fakeTimeout, 0o700);
    const child = join(scratch, "kc.sh");
    writeFileSync(
      child,
      `#!/bin/sh
set +x
[ "$KC_DB_PASSWORD" = "$EXPECTED_DB_PASSWORD" ] || exit 91
[ "$KC_BOOTSTRAP_ADMIN_USERNAME" = "$EXPECTED_ADMIN_USERNAME" ] || exit 92
[ "$KC_BOOTSTRAP_ADMIN_PASSWORD" = "$EXPECTED_ADMIN_PASSWORD" ] || exit 93
if [ "\${PROVE_ERREXIT:-}" = 1 ]; then
  false
  printf 'continued after failure\\n' > "$ERREXIT_MARKER"
fi
printf 'keycloak child invoked: %s\\n' "$*"
`,
      { mode: 0o700 },
    );
    chmodSync(child, 0o700);
    const publicGateDir = join(scratch, "keycloak-public-gate");
    mkdirSync(publicGateDir, { mode: 0o700 });
    chmodSync(publicGateDir, 0o700);
    const publicGate = join(publicGateDir, "cpr45-keycloak-realm-v3.ready");
    const generationGate = join(scratch, "synveda-generation-gate");
    writeFileSync(
      generationGate,
      `#!/bin/sh
[ "$#" -eq 1 ] && [ "$1" = rotate ] || exit 96
rm -f -- "$SYNVEDA_TEST_PUBLIC_GATE"
printf '.generation-testfixture\\n'
`,
      { mode: 0o700 },
    );
    chmodSync(generationGate, 0o700);
    const fixtureSource = source.replace(
      "generation_gate=/opt/keycloak/bin/synveda-generation-gate",
      `generation_gate=${JSON.stringify(generationGate)}`,
    );
    const entrypoint = join(scratch, "keycloak-entrypoint");
	    writeFileSync(
	      entrypoint,
	      fixtureSource
	        .replace("/opt/keycloak/bin/kc.sh", child)
	        .replaceAll("/opt/keycloak/bin/synveda-input-snapshot", snapshotHelper)
	        .replaceAll("/usr/bin/timeout", fakeTimeout),
      { mode: 0o700 },
    );
    chmodSync(entrypoint, 0o700);

    const values = {
      db: "cpr45-keycloak-db-sentinel",
      username: "cpr45-keycloak-user-sentinel",
      password: "cpr45-keycloak-admin-sentinel",
    };
    const files = {
      db: join(scratch, "db"),
      username: join(scratch, "username"),
      password: join(scratch, "password"),
    };
    for (const name of Object.keys(files)) {
      writeFileSync(files[name], `${values[name]}\n`, { mode: 0o600 });
      chmodSync(files[name], 0o600);
    }
    const environment = {
      ...process.env,
      KC_DB_PASSWORD_FILE: files.db,
      KC_BOOTSTRAP_ADMIN_USERNAME_FILE: files.username,
      KC_BOOTSTRAP_ADMIN_PASSWORD_FILE: files.password,
      EXPECTED_DB_PASSWORD: values.db,
      EXPECTED_ADMIN_USERNAME: values.username,
      EXPECTED_ADMIN_PASSWORD: values.password,
      SYNVEDA_TEST_PUBLIC_GATE: publicGate,
    };
    for (const startupSetting of [
      "BASH_ENV",
      "ENV",
      "SHELLOPTS",
      "PS4",
      "CDPATH",
      "GLOBIGNORE",
    ]) {
      delete environment[startupSetting];
    }
    for (const direct of [
      "KC_DB_PASSWORD",
      "KC_BOOTSTRAP_ADMIN_USERNAME",
      "KC_BOOTSTRAP_ADMIN_PASSWORD",
    ]) {
      delete environment[direct];
    }

    let result = spawnSync(entrypoint, ["start", "--optimized"], {
      env: environment,
      encoding: "utf8",
    });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout, "keycloak child invoked: start --optimized\n");
    assertSnapshotsClean();
    for (const value of Object.values(values)) {
      assert.ok(!`${result.stdout}${result.stderr}`.includes(value));
    }

    writeFileSync(publicGate, "cpr45-keycloak-realm-v3\n", { mode: 0o400 });
    result = spawnSync(entrypoint, ["start", "--optimized"], {
      env: environment,
      encoding: "utf8",
    });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout, "keycloak child invoked: start --optimized\n");
    assert.equal(existsSync(publicGate), false, "Keycloak start retained a stale gate");
    assertSnapshotsClean();

    writeFileSync(publicGate, "cpr45-keycloak-realm-v3\n", { mode: 0o400 });
    for (const unsupported of [
      ["show-config"],
      ["start-dev"],
      ["start"],
      ["start", "--optimized", "--features=preview"],
    ]) {
      result = spawnSync(entrypoint, unsupported, {
        env: environment,
        encoding: "utf8",
      });
      assert.equal(result.status, 64, `${unsupported.join(" ")}: ${result.stderr}`);
      assert.match(result.stderr, /supported|unsupported command/);
      assert.equal(result.stdout, "");
      assert.equal(
        existsSync(publicGate),
        true,
        `${unsupported.join(" ")} reached generation rotation`,
      );
    }

    const errexitMarker = join(scratch, "errexit-marker");
    result = spawnSync(entrypoint, ["start", "--optimized"], {
      env: { ...environment, ERREXIT_MARKER: errexitMarker, PROVE_ERREXIT: "1" },
      encoding: "utf8",
    });
    assert.equal(result.status, 1, result.stderr);
    assert.equal(existsSync(errexitMarker), false, "privileged Bash child lost errexit");
    assertSnapshotsClean();

    const startupMarker = join(scratch, "startup-marker");
    const bashEnv = join(scratch, "bash-env");
    writeFileSync(
      bashEnv,
      `printf 'bash-env-executed\n' >> "$SYNVEDA_STARTUP_MARKER"
printf 'startup leak: '
while IFS= read -r secret_line; do printf '%s' "$secret_line"; done < "$KC_DB_PASSWORD_FILE"
printf '\n'
`,
      { mode: 0o700 },
    );
    chmodSync(bashEnv, 0o700);
    result = spawnSync(entrypoint, ["start", "--optimized"], {
      env: {
        ...environment,
        BASH_ENV: bashEnv,
        ENV: bashEnv,
        SHELLOPTS: "xtrace",
        PS4: '$(printf ps4-executed >>"$SYNVEDA_STARTUP_MARKER")',
        CDPATH: scratch,
        GLOBIGNORE: "*",
        SYNVEDA_STARTUP_MARKER: startupMarker,
      },
      encoding: "utf8",
    });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout, "keycloak child invoked: start --optimized\n");
    assert.equal(result.stderr, "");
    assert.equal(existsSync(startupMarker), false, "Bash startup input executed");
    assertSnapshotsClean();
    for (const value of Object.values(values)) {
      assert.ok(!`${result.stdout}${result.stderr}`.includes(value));
    }

    const cleanupFailureEntrypoint = join(scratch, "keycloak-entrypoint-cleanup-failure");
    writeFileSync(
      cleanupFailureEntrypoint,
      fixtureSource
        .replace('[ "$cleanup_status" -eq 0 ]', "false")
        .replace("/opt/keycloak/bin/kc.sh", child)
        .replaceAll("/opt/keycloak/bin/synveda-input-snapshot", snapshotHelper)
        .replaceAll("/usr/bin/timeout", fakeTimeout),
      { mode: 0o700 },
    );
    chmodSync(cleanupFailureEntrypoint, 0o700);
    result = spawnSync(cleanupFailureEntrypoint, ["start", "--optimized"], {
      env: environment,
      encoding: "utf8",
    });
    assert.equal(result.status, 70, result.stderr);
    assert.equal(result.stdout, "");
    assert.match(result.stderr, /secret snapshot cleanup failed/);
    for (const value of Object.values(values)) {
      assert.ok(!`${result.stdout}${result.stderr}`.includes(value));
    }
    assertSnapshotsClean();

    result = spawnSync(entrypoint, ["start", "--optimized"], {
      env: { ...environment, KC_DB_PASSWORD: "cpr45-direct-secret-sentinel" },
      encoding: "utf8",
    });
	    assert.equal(result.status, 78);
	    assert.match(result.stderr, /direct KC_DB_PASSWORD is forbidden/);
	    assert.doesNotMatch(`${result.stdout}${result.stderr}`, /cpr45-direct-secret-sentinel/);
	    assertSnapshotsClean();

    result = spawnSync(entrypoint, ["start", "--optimized"], {
      env: {
        ...environment,
        SYNVEDA_KEYCLOAK_CONVERGENCE_PASSWORD: "cpr45-direct-convergence-sentinel",
      },
      encoding: "utf8",
    });
	    assert.equal(result.status, 78);
	    assert.match(
      result.stderr,
      /direct SYNVEDA_KEYCLOAK_CONVERGENCE_PASSWORD is forbidden/,
    );
	    assert.doesNotMatch(
      `${result.stdout}${result.stderr}`,
      /cpr45-direct-convergence-sentinel/,
    );
	    assertSnapshotsClean();

    writeFileSync(files.db, "first-line\nsecond-line\n", { mode: 0o600 });
    result = spawnSync(entrypoint, ["start", "--optimized"], {
      env: environment,
      encoding: "utf8",
    });
	    assert.equal(result.status, 78);
	    assert.match(result.stderr, /must contain one line/);
	    assertSnapshotsClean();

	    writeFileSync(files.db, "é".repeat(3000), { mode: 0o600 });
	    result = spawnSync(entrypoint, ["start", "--optimized"], {
          env: environment,
          encoding: "utf8",
        });
	    assert.equal(result.status, 78);
	    assert.match(result.stderr, /could not be snapshotted safely/);
	    assertSnapshotsClean();

    writeFileSync(files.db, Buffer.from([0x61, 0x62, 0x63, 0x00, 0x64, 0x65, 0x66]), {
      mode: 0o600,
    });
	    result = spawnSync(entrypoint, ["start", "--optimized"], {
          env: environment,
          encoding: "utf8",
        });
	    assert.equal(result.status, 78);
	    assert.match(result.stderr, /contains a NUL byte/);
	    assertSnapshotsClean();

	    const symlinkTarget = join(scratch, "symlink-target");
	    writeFileSync(symlinkTarget, "cpr45-keycloak-symlink-sentinel\n", { mode: 0o600 });
	    rmSync(files.db);
	    execFileSync("ln", ["-s", symlinkTarget, files.db]);
	    result = spawnSync(entrypoint, ["start", "--optimized"], {
          env: environment,
          encoding: "utf8",
        });
	    assert.equal(result.status, 78);
	    assert.match(result.stderr, /could not be snapshotted safely/);
	    assert.doesNotMatch(`${result.stdout}${result.stderr}`, /symlink-sentinel/);
	    assert.ok(!`${result.stdout}${result.stderr}`.includes(files.db));
	    assertSnapshotsClean();

	    rmSync(files.db);
	    execFileSync("mkfifo", [files.db]);
	    const started = Date.now();
	    result = spawnSync(entrypoint, ["start", "--optimized"], {
	      env: environment,
	      encoding: "utf8",
	      timeout: 2000,
	    });
	    assert.equal(result.status, 78);
	    assert.equal(result.signal, null, "writerless FIFO reached the test timeout");
	    assert.ok(Date.now() - started < 2000, "writerless FIFO blocked the entrypoint");
	    assert.equal(result.stdout, "");
	    assert.ok(!result.stderr.includes(files.db));
	    assertSnapshotsClean();
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("database convergence passes runtime passwords only as COPY data", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-database-bootstrap-"));
  try {
    const secrets = join(scratch, "secrets");
    const authority = join(scratch, "authority");
    const bin = join(scratch, "bin");
    const calls = join(scratch, "psql-calls");
    const sql = join(scratch, "psql-input");
    writeFileSync(calls, "", { mode: 0o600 });
    writeFileSync(sql, "", { mode: 0o600 });
    execFileSync("mkdir", ["-m", "700", secrets, authority, bin]);

    const values = {
      postgres_bootstrap_password: "cpr45-bootstrap\\password:sentinel",
      synveda_migrator_password: "cpr45-migrator-password-sentinel",
      synveda_gateway_password: "cpr45-gateway-password-sentinel",
      synveda_worker_password: "cpr45-worker-password-sentinel",
      keycloak_database_password: "cpr45-keycloak-password-sentinel",
    };
    for (const [name, value] of Object.entries(values)) {
      writeFileSync(join(secrets, name), `${value}\n`, { mode: 0o600 });
      chmodSync(join(secrets, name), 0o600);
    }
    const roleContract = join(secrets, "database_roles.json");
    writeFileSync(
      roleContract,
      readFileSync(join(COMPOSE, "configs/database/roles.reference.json"), "utf8"),
      { mode: 0o600 },
    );
    chmodSync(roleContract, 0o600);
    const expectedPgpass = join(scratch, "expected-pgpass");
    const escapedBootstrapPassword = values.postgres_bootstrap_password
      .replaceAll("\\", "\\\\")
      .replaceAll(":", "\\:");
    writeFileSync(
      expectedPgpass,
      ["postgres", "synveda", "keycloak"]
        .map(
          (database) =>
            `database.compose.example:5432:${database}:bootstrap:${escapedBootstrapPassword}\n`,
        )
        .join(""),
      { mode: 0o600 },
    );
    chmodSync(expectedPgpass, 0o600);

    const fakePsql = join(bin, "psql");
    writeFileSync(
      fakePsql,
      `#!/bin/sh
if [ "\${PGPASSWORD+x}" = x ]; then
  printf 'credential-env:direct\n' >> "$SYNVEDA_TEST_PSQL_CALLS"
else
  printf 'credential-env:unset\n' >> "$SYNVEDA_TEST_PSQL_CALLS"
fi
if [ -n "\${PGPASSFILE:-}" ] && [ -f "$PGPASSFILE" ] && [ ! -L "$PGPASSFILE" ]; then
  printf 'credential-file:regular\n' >> "$SYNVEDA_TEST_PSQL_CALLS"
  case "$(LC_ALL=C ls -ld "$PGPASSFILE")" in
    -rw-------*) printf 'credential-mode:0600\n' >> "$SYNVEDA_TEST_PSQL_CALLS" ;;
    *) printf 'credential-mode:unsafe\n' >> "$SYNVEDA_TEST_PSQL_CALLS" ;;
  esac
  if cmp -s "$PGPASSFILE" "$SYNVEDA_TEST_EXPECTED_PGPASS_FILE"; then
    printf 'credential-content:escaped\n' >> "$SYNVEDA_TEST_PSQL_CALLS"
  else
    printf 'credential-content:mismatch\n' >> "$SYNVEDA_TEST_PSQL_CALLS"
  fi
else
  printf 'credential-file:missing\n' >> "$SYNVEDA_TEST_PSQL_CALLS"
fi
printf 'pgpass:%s\n' "$PGPASSFILE" >> "$SYNVEDA_TEST_PSQL_CALLS"
printf '%s\n' "$@" >> "$SYNVEDA_TEST_PSQL_CALLS"
case "$*" in
  *"-tAc select 1"*) exit 0 ;;
  *"-tAc select exists (select 1 from pg_catalog.pg_database where datname ="*)
    printf 'f\n'
    exit 0
    ;;
esac
input=$(sed -n '1,1200p')
printf '%s\n' "$input" >> "$SYNVEDA_TEST_PSQL_INPUT"
case "$input" in
  *"select control.system_identifier::text"*"pg_control_system"*)
    printf '7536657783470215051\n'
    exit 0
    ;;
  *"end || ':' || coalesce"*)
    printf 'absent:0\n'
    exit 0
    ;;
  *"select database.oid::bigint"*"owner.rolname = 'keycloak'"*)
    printf '16385\n'
    exit 0
    ;;
  *"then 'absent'"*"else 'unsafe'"*)
    printf 'absent\n'
    exit 0
    ;;
esac
if [ "\${SYNVEDA_TEST_PSQL_FORCE_FAILURE:-false}" = true ]; then
  case "$input" in
    *'\\copy pg_temp.'*'alter role '*' with login inherit password %L'*)
      echo "ERROR: forced content-free convergence failure" >&2
      exit 1
      ;;
    *) exit 0 ;;
  esac
fi
case "$input" in
  *'"cluster_system_identifier"'*'pg_postmaster_start_time'*)
    printf '%s\n' '{"version":1,"database":"keycloak","cluster_system_identifier":"7536657783470215051","postmaster_started_at":"2026-08-28T12:34:56.123456Z","database_oid":16385}'
    ;;
esac
exit 0
`,
      { mode: 0o700 },
    );
    chmodSync(fakePsql, 0o700);
    const fakeTimeout = join(bin, "timeout");
    writeFileSync(
      fakeTimeout,
      `#!/bin/sh
[ "$1" = --foreground ] && [ "$2" = --signal=TERM ] || exit 96
	case "$3:$4" in
	  --kill-after=5s:300s|--kill-after=1s:6s|--kill-after=1s:5s) ;;
	  *) exit 96 ;;
	esac
shift 4
exec "$@"
`,
      { mode: 0o700 },
    );
    chmodSync(fakeTimeout, 0o700);
    const fakeSync = join(bin, "sync");
    writeFileSync(fakeSync, "#!/bin/sh\n[ \"$#\" -eq 1 ]\n", { mode: 0o700 });
	    chmodSync(fakeSync, 0o700);
	    const fakeSnapshot = join(scratch, "synveda-input-snapshot");
	    execFileSync("cc", [
	      "-std=c11",
	      "-O2",
	      "-Wall",
	      "-Wextra",
	      "-Werror",
	      INPUT_SNAPSHOT,
	      "-o",
	      fakeSnapshot,
	    ]);
	    chmodSync(fakeSnapshot, 0o700);

	    const bootstrap = join(scratch, "synveda-database-bootstrap");
	    writeFileSync(
	      bootstrap,
	      readFileSync(DATABASE_BOOTSTRAP, "utf8")
	        .replaceAll("/run/secrets", secrets)
	        .replaceAll("/usr/local/bin/synveda-input-snapshot", fakeSnapshot),
      { mode: 0o700 },
    );
    chmodSync(bootstrap, 0o700);
    const environment = {
      ...process.env,
      PATH: `${bin}:${process.env.PATH}`,
      SYNVEDA_POSTGRES_BOOTSTRAP_URL:
        "postgresql://bootstrap@database.compose.example:5432/postgres",
      SYNVEDA_TEST_PSQL_CALLS: calls,
      SYNVEDA_TEST_PSQL_INPUT: sql,
      SYNVEDA_TEST_EXPECTED_PGPASS_FILE: expectedPgpass,
      SYNVEDA_DATABASE_BOOTSTRAP_PRIVATE_DIR: secrets,
      SYNVEDA_DATABASE_AUTHORITY_DIR: authority,
      SYNVEDA_DATABASE_ROLES_FILE: roleContract,
      SYNVEDA_POSTGRES_BUNDLED_CLUSTER: "true",
    };
    for (const name of Object.keys(environment)) {
      if (name === "PSQLRC" || name.startsWith("PG")) delete environment[name];
    }

    for (const validator of ["validate-synveda-passwords", "validate-keycloak-password"]) {
      const result = spawnSync(bootstrap, [validator], { env: environment, encoding: "utf8" });
      assert.equal(result.status, 0, result.stderr);
      assert.equal(result.stdout, "");
      assert.equal(result.stderr, "");
    }

    const completeCredentialEnvironment = {
      ...environment,
      SYNVEDA_DATABASE_REQUIRE_KEYCLOAK_PASSWORD: "true",
    };
    let credentialResult = spawnSync(bootstrap, ["validate-synveda-passwords"], {
      env: completeCredentialEnvironment,
      encoding: "utf8",
    });
    assert.equal(credentialResult.status, 0, credentialResult.stderr);
    assert.equal(credentialResult.stdout, "");
    assert.equal(credentialResult.stderr, "");

    const credentialNames = Object.keys(values);
    for (let first = 0; first < credentialNames.length; first += 1) {
      for (let second = first + 1; second < credentialNames.length; second += 1) {
        const firstName = credentialNames[first];
        const secondName = credentialNames[second];
        const collisionTarget =
          firstName === "postgres_bootstrap_password" ? firstName : secondName;
        const collisionSource =
          collisionTarget === firstName ? secondName : firstName;
        for (const [encoding, encodedCollision] of [
          ["no newline", values[collisionSource]],
          ["CRLF", `${values[collisionSource]}\r\n`],
        ]) {
          writeFileSync(join(secrets, collisionTarget), encodedCollision, {
            mode: 0o600,
          });
          chmodSync(join(secrets, collisionTarget), 0o600);
          credentialResult = spawnSync(bootstrap, ["validate-synveda-passwords"], {
            env: completeCredentialEnvironment,
            encoding: "utf8",
          });
          assert.equal(
            credentialResult.status,
            78,
            `${firstName}/${secondName} ${encoding} credential collision: ${credentialResult.stderr}`,
          );
          assert.equal(credentialResult.stdout, "");
          assert.equal(
            credentialResult.stderr,
            "database-bootstrap: database credentials must be pairwise distinct\n",
          );
          for (const value of Object.values(values)) {
            assert.ok(!credentialResult.stderr.includes(value));
          }
        }
        writeFileSync(
          join(secrets, collisionTarget),
          `${values[collisionTarget]}\n`,
          { mode: 0o600 },
        );
        chmodSync(join(secrets, collisionTarget), 0o600);
      }
    }

    credentialResult = spawnSync(bootstrap, ["validate-synveda-passwords"], {
      env: {
        ...environment,
        SYNVEDA_DATABASE_REQUIRE_KEYCLOAK_PASSWORD: "yes",
      },
      encoding: "utf8",
    });
    assert.equal(credentialResult.status, 78);
    assert.equal(credentialResult.stdout, "");
    assert.equal(
      credentialResult.stderr,
      "database-bootstrap: SYNVEDA_DATABASE_REQUIRE_KEYCLOAK_PASSWORD must be true or false\n",
    );

    const invalidCredentials = [
      { value: Buffer.alloc(0), label: "empty" },
      { value: Buffer.from("first-line\nsecond-line\n"), label: "multiline" },
      { value: Buffer.from("nul\0sentinel\n"), label: "NUL" },
      { value: Buffer.from("colon:sentinel\n"), label: "unsupported" },
      { value: Buffer.from(`md5${"a".repeat(32)}\n`), label: "MD5 verifier" },
      { value: Buffer.from("SCRAM-SHA-256$sentinel\n"), label: "SCRAM verifier" },
      { value: Buffer.from(`${"a".repeat(4097)}\n`), label: "oversized" },
    ];
    const gatewayPassword = join(secrets, "synveda_gateway_password");
    for (const { value, label } of invalidCredentials) {
      writeFileSync(gatewayPassword, value, { mode: 0o600 });
      chmodSync(gatewayPassword, 0o600);
      const result = spawnSync(bootstrap, ["validate-synveda-passwords"], {
        env: environment,
        encoding: "utf8",
      });
      assert.equal(result.status, 78, `${label} credential refusal: ${result.stderr}`);
      assert.equal(result.stdout, "");
      assert.doesNotMatch(result.stderr, /(?:first-line|second-line|sentinel|md5a{32})/);
      assert.ok(!result.stderr.includes(gatewayPassword));
    }
    writeFileSync(gatewayPassword, `${values.synveda_gateway_password}\n`, { mode: 0o600 });
    chmodSync(gatewayPassword, 0o600);

    for (const [name, value] of [
      ["PGHOSTADDR", "203.0.113.17"],
      ["PGSERVICE", "cpr45-routing-sentinel"],
      ["PGSSLCERTMODE", "cpr45-routing-sentinel"],
      ["PGGSSENCMODE", "cpr45-routing-sentinel"],
      ["PGGSSDELEGATION", "cpr45-routing-sentinel"],
    ]) {
      writeFileSync(calls, "", { mode: 0o600 });
      const result = spawnSync(bootstrap, ["synveda"], {
        env: { ...environment, [name]: value },
        encoding: "utf8",
      });
      assert.equal(result.status, 78, `${name} was not refused`);
      assert.equal(result.stdout, "");
      assert.match(result.stderr, /ambient PostgreSQL connection settings are forbidden/);
      assert.doesNotMatch(`${result.stdout}${result.stderr}`, /cpr45-routing-sentinel|203\.0\.113\.17/);
      assert.equal(readFileSync(calls, "utf8"), "", `${name} reached psql`);
    }

    writeFileSync(calls, "", { mode: 0o600 });
    let externalResult = spawnSync(bootstrap, ["synveda"], {
      env: { ...environment, SYNVEDA_POSTGRES_BUNDLED_CLUSTER: "false" },
      encoding: "utf8",
    });
    assert.equal(externalResult.status, 78);
    assert.equal(externalResult.stdout, "");
    assert.match(
      externalResult.stderr,
      /external PostgreSQL mutation is unavailable until the authenticated TLS bootstrap contract is implemented/,
    );
    assert.equal(readFileSync(calls, "utf8"), "", "external refusal reached psql");

    for (const target of ["synveda", "keycloak"]) {
      writeFileSync(calls, "", { mode: 0o600 });
      writeFileSync(sql, "", { mode: 0o600 });
      const result = spawnSync(bootstrap, [target], { env: environment, encoding: "utf8" });
      assert.equal(result.status, 0, result.stderr);
      assert.equal(result.stdout, `${target} database convergence complete\n`);
      const callLog = readFileSync(calls, "utf8");
      const observable = `${result.stdout}${result.stderr}${callLog}${readFileSync(sql, "utf8")}`;
      for (const value of Object.values(values)) {
        assert.ok(!observable.includes(value), `${target} exposed ${value}`);
      }
      assert.doesNotMatch(callLog, /password/i);
      assert.match(callLog, /credential-env:unset/);
      assert.match(callLog, /credential-file:regular/);
      assert.match(callLog, /credential-mode:0600/);
      assert.match(callLog, /credential-content:escaped/);
      assert.doesNotMatch(
        callLog,
        /credential-(?:env:direct|file:missing|mode:unsafe|content:mismatch)/,
      );
      const pgpassPaths = callLog
        .split("\n")
        .filter((line) => line.startsWith("pgpass:"))
        .map((line) => line.slice("pgpass:".length));
      assert.ok(pgpassPaths.length > 0, `${target} never supplied a private pgpass file`);
      for (const path of pgpassPaths) {
        assert.equal(existsSync(path), false, `${target} retained ${path}`);
      }
      const sqlInput = readFileSync(sql, "utf8");
      assert.match(sqlInput, new RegExp(`create database ${target}`));
      assert.match(sqlInput, /set local password_encryption = 'scram-sha-256'/);
      assert.match(sqlInput, /current_setting\('password_encryption'\) = 'scram-sha-256'/);
      assert.match(sqlInput, /\\i \/usr\/local\/share\/synveda\/credential-log-contract\.sql/);
      assert.match(sqlInput, /exception when query_canceled or assert_failure or others/);
      assert.match(sqlInput, /pg_catalog\.octet_length\(credential\.secret\) not between 1 and 4096/);
      assert.match(sqlInput, /credential\.secret !~ '\^\[A-Za-z0-9\._~-\]\+\$'/);
      assert.ok(
        sqlInput.includes(
          "credential.secret ~ '^(md5[0-9A-Fa-f]{32}|SCRAM-SHA-256[$])'",
        ),
      );
      assert.doesNotMatch(sqlInput, /\\getenv\s+\S*password/i);
      if (target === "synveda") {
        for (const role of ["migrator", "gateway", "worker"]) {
          assert.ok(
            sqlInput.includes(
              `\\copy pg_temp.synveda_${role}_credential(secret) from '/tmp/synveda-database-bootstrap/synveda_${role}_password'`,
            ),
          );
        }
      } else {
        assert.ok(
          sqlInput.includes(
            "\\copy pg_temp.keycloak_credential(secret) from '/tmp/synveda-database-bootstrap/keycloak_database_password'",
          ),
        );
        const witnessPath = join(authority, "keycloak-cluster.json");
        assert.equal(
          readFileSync(witnessPath, "utf8"),
          '{"version":1,"database":"keycloak","cluster_system_identifier":"7536657783470215051","postmaster_started_at":"2026-08-28T12:34:56.123456Z","database_oid":16385}\n',
        );
        assert.equal(statSync(witnessPath).mode & 0o777, 0o600);
      }
    }

    for (const target of ["synveda", "keycloak"]) {
      writeFileSync(calls, "", { mode: 0o600 });
      writeFileSync(sql, "", { mode: 0o600 });
      const result = spawnSync(bootstrap, [target], {
        env: { ...environment, SYNVEDA_TEST_PSQL_FORCE_FAILURE: "true" },
        encoding: "utf8",
      });
      assert.equal(result.status, 1);
      assert.match(result.stderr, /forced content-free convergence failure/);
      const failedSql = readFileSync(sql, "utf8");
      assert.match(failedSql, /\\copy pg_temp\.(?:synveda_migrator|keycloak)_credential/);
      assert.match(failedSql, /alter role .* with login inherit password %L/);
      const observable = `${result.stdout}${result.stderr}${readFileSync(calls, "utf8")}${failedSql}`;
      for (const value of Object.values(values)) {
        assert.ok(!observable.includes(value), `${target} failure exposed ${value}`);
      }
      if (target === "keycloak") {
        assert.equal(existsSync(join(authority, "keycloak-cluster.json")), false);
      }
    }

    let result = spawnSync(bootstrap, ["synveda"], {
      env: {
        ...environment,
        SYNVEDA_POSTGRES_BOOTSTRAP_URL:
          "postgresql://bootstrap:cpr45-url-secret@database.compose.example:5432/postgres",
      },
      encoding: "utf8",
    });
    assert.equal(result.status, 78);
    assert.match(result.stderr, /must not contain a password/);
    assert.doesNotMatch(`${result.stdout}${result.stderr}`, /cpr45-url-secret/);

    writeFileSync(calls, "", { mode: 0o600 });
    result = spawnSync(bootstrap, ["synveda"], {
      env: {
        ...environment,
        SYNVEDA_POSTGRES_BOOTSTRAP_URL:
          "postgresql://bootstrap@decoy:cpr45-argv-sentinel@database.compose.example:5432/postgres",
      },
      encoding: "utf8",
    });
    assert.equal(result.status, 78);
    assert.equal(result.stdout, "");
    assert.match(result.stderr, /bootstrap URL authority is invalid/);
    assert.doesNotMatch(`${result.stdout}${result.stderr}`, /cpr45-argv-sentinel/);
    assert.equal(readFileSync(calls, "utf8"), "");

  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("the database test bridge consumes one private file without rendering it", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-cargo-database-url-"));
  try {
    const path = join(scratch, "gateway-url");
    const sentinel = "postgresql://gateway:cpr45-wrapper-sentinel@database.test:5432/synveda";
    writeFileSync(path, `${sentinel}\n`, { mode: 0o600 });
    chmodSync(path, 0o600);

    let result = spawnSync(
      CARGO_DATABASE_URL_WRAPPER,
      [
        "/bin/sh",
        "-c",
        '[ "$DATABASE_URL" = "$EXPECTED_DATABASE_URL" ] && [ "${SYNVEDA_CARGO_DATABASE_URL_FILE+x}" != x ]',
      ],
      {
        env: {
          ...process.env,
          EXPECTED_DATABASE_URL: sentinel,
          SYNVEDA_CARGO_DATABASE_URL_FILE: path,
        },
        encoding: "utf8",
      },
    );
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout, "");
    assert.equal(result.stderr, "");

    result = spawnSync("/bin/sh", ["-x", CARGO_DATABASE_URL_WRAPPER, "true"], {
      env: {
        ...process.env,
        SYNVEDA_CARGO_DATABASE_URL_FILE: path,
      },
      encoding: "utf8",
    });
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stderr, /(?:^|\n)\+ set \+x(?:\n|$)/);
    assert.doesNotMatch(`${result.stdout}${result.stderr}`, /cpr45-wrapper-sentinel/);
    assert.ok(!`${result.stdout}${result.stderr}`.includes(path));

    result = spawnSync(CARGO_DATABASE_URL_WRAPPER, ["true"], {
      env: {
        ...process.env,
        DATABASE_URL: "postgresql://direct:cpr45-direct-wrapper-sentinel@database.test/db",
        SYNVEDA_CARGO_DATABASE_URL_FILE: path,
      },
      encoding: "utf8",
    });
    assert.equal(result.status, 78);
    assert.equal(result.stdout, "");
    assert.match(result.stderr, /direct DATABASE_URL is forbidden/);
    assert.doesNotMatch(`${result.stdout}${result.stderr}`, /cpr45-(?:direct-)?wrapper-sentinel/);
    assert.ok(!result.stderr.includes(path));

    result = spawnSync(CARGO_DATABASE_URL_WRAPPER, ["true"], {
      env: {
        ...process.env,
        SYNVEDA_CARGO_DATABASE_URL_FILE: join(scratch, "missing-cpr45-path-sentinel"),
      },
      encoding: "utf8",
    });
    assert.equal(result.status, 78);
    assert.equal(result.stdout, "");
    assert.match(result.stderr, /private URL file is unavailable/);
    assert.doesNotMatch(`${result.stdout}${result.stderr}`, /missing-cpr45-path-sentinel/);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("database authority helper contracts are copied and fail closed", () => {
  const credentialLog = readFileSync(CREDENTIAL_LOG_CONTRACT, "utf8");
  for (const [name, value] of CREDENTIAL_LOG_SETTINGS) {
    assert.ok(credentialLog.includes(`set ${name} = ${value};`), `${name} is not closed`);
    assert.ok(
      credentialLog.includes(`current_setting('${name}')`),
      `${name} is not verified after SET`,
    );
  }
  for (const setting of [
    "shared_preload_libraries",
    "session_preload_libraries",
    "local_preload_libraries",
    "default_table_access_method",
    "client_encoding",
    "jit",
  ]) {
    const expected = {
      shared_preload_libraries: "''",
      session_preload_libraries: "''",
      local_preload_libraries: "''",
      default_table_access_method: "'heap'",
      client_encoding: "'UTF8'",
      jit: "'off'",
    }[setting];
    assert.ok(
      credentialLog.includes(`current_setting('${setting}') = ${expected}`),
      `${setting} is not proved inside each credential session`,
    );
  }
  assert.ok(credentialLog.includes("current_user = session_user"));
  assert.ok(credentialLog.includes("current_setting('role') = 'none'"));

  const extension = readFileSync(EXTENSION_CONTRACT, "utf8");
  assert.match(extension, /extension\.extname = 'btree_gin'[\s\S]*extension\.extversion = '1\.3'/);
  assert.match(extension, /extension\.extname = 'vector'[\s\S]*extension\.extversion = '0\.8\.6'/);
  const extensionSearchPath = extension.indexOf("set search_path = pg_catalog, public;");
  const fingerprintInclude = extension.indexOf(
    "\\i /usr/local/share/synveda/extension-fingerprint-assert.psql",
  );
  assert.ok(extensionSearchPath >= 0 && fingerprintInclude > extensionSearchPath);
  assert.equal(
    occurrenceCount(extension, "\\i /usr/local/share/synveda/extension-fingerprint-assert.psql"),
    1,
  );
  assert.match(extension, /\\if :synveda_extension_safe/);
  assert.match(extension, /exact extension fingerprint was refused/);

  const fingerprint = readFileSync(EXTENSION_FINGERPRINT, "utf8");
  for (const digest of [
    "de5d37023e87c8306c325d8b361c08220a7d77e2cd59e2407ebe01caa881577d",
    "1a4cf221e73829cba2b8eb8b659e951670d04c5eb13578cfa21d06624b3eb178",
    "5b1552a857b437d8a0c3274d3344feaed14a4033ce0ebcdcef238ea99f84b980",
  ]) {
    assert.ok(fingerprint.includes(digest), `extension fingerprint lacks ${digest}`);
  }
  assert.match(fingerprint, /extension\.extname in \('plpgsql', 'btree_gin', 'vector'\)/);
  assert.match(fingerprint, /routine\.proowner = extension\.owner_oid/);
  assert.match(fingerprint, /language\.lanowner = extension\.owner_oid/);
  assert.match(fingerprint, /pg_identify_object_as_address/);
  for (const bound of [
    /from pg_catalog\.pg_extension extension[\s\S]*limit 4/,
    /extension_members as materialized[\s\S]*limit 387/,
    /operator_family_oids as materialized[\s\S]*limit 54/,
    /access_operators as materialized[\s\S]*limit 182/,
    /support_functions as materialized[\s\S]*limit 200/,
  ]) {
    assert.match(fingerprint, bound);
  }
  assert.doesNotMatch(fingerprint, /select\s+1\s*\/\s*0/i);
  assert.doesNotMatch(fingerprint, /^\\/m);
  assert.doesNotMatch(
    fingerprint,
    /::reg(?:class|procedure|operator|collation)::text|pg_get_function_|format_type/,
  );

  const runtimeRole = readFileSync(STORE_RUNTIME_ROLE, "utf8");
  assert.match(runtimeRole, /if safe != Some\(true\)/);
  const storeLib = readFileSync(STORE_LIB, "utf8");
  const preMigrationFingerprint = storeLib.indexOf(
    "verify_migration_extension_prerequisites_connection",
  );
  const migrationRun = storeLib.indexOf("MIGRATOR\n        .run");
  assert.ok(preMigrationFingerprint >= 0 && migrationRun > preMigrationFingerprint);

  const clusterAuthority = readFileSync(CLUSTER_AUTHORITY_CONTRACT, "utf8");
  assert.deepEqual(clusterAuthorityContractFindings(clusterAuthority), []);
  for (const [name, mutated] of [
    [
      "schema-qualified COALESCE pseudo-function",
      clusterAuthority.replace("coalesce((", "pg_catalog.coalesce(("),
    ],
    [
      "forbidden element coercion",
      clusterAuthority.replace(
        "pg_catalog.jsonb_array_elements(",
        "pg_catalog.jsonb_array_elements_text(",
      ),
    ],
    [
      "forbidden element type guard removal",
      clusterAuthority.replace("pg_catalog.jsonb_typeof(value) <> 'string'", "false"),
    ],
    [
      "bundled Keycloak peer requirement removal",
      clusterAuthority.replaceAll("database_name = 'keycloak'", "database_name = 'removed-peer'"),
    ],
    [
      "Keycloak maintenance database isolation removal",
      clusterAuthority.replace(
        "forbidden.database_name <> 'keycloak'",
        "forbidden.database_name = 'keycloak'",
      ),
    ],
    [
      "closed Keycloak peer connection gate removal",
      clusterAuthority.replace("and not database.datallowconn", "and database.datallowconn"),
    ],
    [
      "closed Keycloak target recovery removal",
      clusterAuthority.replace(
        ":'synveda_bootstrap_target' in ('synveda', 'keycloak')\n       and forbidden.database_name = 'keycloak'",
        ":'synveda_bootstrap_target' = 'synveda'\n       and forbidden.database_name = 'keycloak'",
      ),
    ],
    [
      "closed Keycloak recovery grouping removal",
      clusterAuthority.replace(
        "     and not (\n       (\n         :'synveda_allow_target_default_acl'",
        "     and not (\n         :'synveda_allow_target_default_acl'",
      ),
    ],
    [
      "closed Keycloak peer template shape removal",
      clusterAuthority.replace(
        "database.datlocprovider = (select template.datlocprovider from template)",
        "true",
      ),
    ],
    [
      "closed Keycloak peer database setting refusal removal",
      clusterAuthority.replace("settings.setdatabase = database.oid", "false"),
    ],
    [
      "closed Keycloak peer per-role setting scope removal",
      clusterAuthority.replace(
        "where settings.setdatabase = database.oid\n       )",
        "where settings.setdatabase = database.oid\n            and settings.setrole = 0\n       )",
      ),
    ],
    [
      "closed Keycloak peer active session refusal removal",
      clusterAuthority.replace(
        ":'synveda_keycloak_no_activity' = 'true'",
        ":'synveda_keycloak_no_activity' = 'false'",
      ),
    ],
    [
      "closed Keycloak startup lock class removal",
      clusterAuthority.replace(
        "lock.classid = 'pg_catalog.pg_database'::pg_catalog.regclass",
        "lock.classid = 0",
      ),
    ],
    [
      "closed Keycloak startup lock grant filter addition",
      clusterAuthority.replace("and lock.pid is not null", "and lock.pid is not null\n     and lock.granted"),
    ],
    [
      "closed Keycloak startup lock refusal removal",
      clusterAuthority.replace(
        ":'synveda_keycloak_no_startup_locks' = 'true'",
        ":'synveda_keycloak_no_startup_locks' = 'false'",
      ),
    ],
    [
      "closed Keycloak prepared transaction refusal removal",
      clusterAuthority.replace(
        ":'synveda_keycloak_no_prepared_xacts' = 'true'",
        ":'synveda_keycloak_no_prepared_xacts' = 'false'",
      ),
    ],
    [
      "closed Keycloak NULL-safe owner binding removal",
      clusterAuthority.replace(
        "database.datdba is not distinct from case :'synveda_bootstrap_target'",
        "database.datdba = case :'synveda_bootstrap_target'",
      ),
    ],
    [
      "bootstrap target-specific role setting refusal removal",
      clusterAuthority.replace(
        "setting.setrole = (select principal.oid from bootstrap_principal principal)",
        "false",
      ),
    ],
    [
      "bootstrap maintenance database requirement removal",
      clusterAuthority.replace(
        "database_name = :'synveda_bootstrap_database'",
        "database_name = 'removed-maintenance-database'",
      ),
    ],
    [
      "target owner phase gate removal",
      clusterAuthority.replace(
        ":'synveda_allow_target_owner_membership' = 'true'",
        ":'synveda_allow_target_owner_membership' = 'false'",
      ),
    ],
    [
      "target default-ACL phase gate removal",
      clusterAuthority.replaceAll(
        ":'synveda_allow_target_default_acl' = 'true'",
        ":'synveda_allow_target_default_acl' = 'false'",
      ),
    ],
    [
      "absent Synveda dependency target guard removal",
      clusterAuthority.replaceAll("synveda_database.oid is not null", "true"),
    ],
    [
      "absent Keycloak dependency target guard removal",
      clusterAuthority.replace("keycloak_database.oid is not null", "true"),
    ],
    [
      "external read-only settings membership removal",
      clusterAuthority.replaceAll("granted.rolname = 'pg_read_all_settings'", "false"),
    ],
    [
      "external superuser refusal removal",
      clusterAuthority.replace("and not principal.rolsuper", "and principal.rolsuper"),
    ],
    [
      "external provenance requirement removal",
      clusterAuthority.replace(
        "(select count(*) from expected) between 1 and 8",
        "(select count(*) from expected) between 0 and 8",
      ),
    ],
  ]) {
    assert.notEqual(mutated, clusterAuthority, `${name} mutant did not alter the fixture`);
    assert.ok(
      clusterAuthorityContractFindings(mutated).length > 0,
      `cluster authority gate accepted ${name}`,
    );
  }
  const sharedDependencyGuard = clusterAuthority.slice(
    clusterAuthority.indexOf("-- pg_shdepend is default-deny"),
  );
  assert.match(
    sharedDependencyGuard,
    /role\.rolname in \([\s\S]*\)\n     and not \([\s\S]*role\.rolname = 'keycloak'/,
  );

  const databaseBootstrap = readFileSync(DATABASE_BOOTSTRAP, "utf8");
  const keycloakHandoff = databaseBootstrap.slice(
    databaseBootstrap.indexOf("-- Keycloak shares the cluster only after"),
    databaseBootstrap.indexOf("-- MUTATION BOUNDARY: every persistent Keycloak"),
  );
  for (const token of [
    "with expected_admin as (",
    "contract.document->'administrative_memberships'",
    "expected.member_name = member.rolname",
    "expected.grantor_name = grantor.rolname",
    "grantor.rolname in ('synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker')",
  ]) {
    assert.ok(keycloakHandoff.includes(token), `Keycloak hand-off lacks ${token}`);
  }
  assert.doesNotMatch(
    keycloakHandoff,
    /granted\.rolname = 'synveda_migrator'[\s\S]*member\.rolname = session_user/,
  );

  const localAuthority = readFileSync(LOCAL_AUTHORITY_CONTRACT, "utf8");
  for (const token of [
    "database.datname = pg_catalog.current_database()",
    "allowed_dependency(dbid, classid, objid, objsubid, refobjid, deptype)",
    "dependency.refclassid = 'pg_catalog.pg_authid'::regclass",
    "attribute.attnum > 0",
    "not attribute.attisdropped",
    "object.relowner = target.owner_oid",
    "object.proowner = target.owner_oid",
    "object.typowner = target.owner_oid",
    "runtime.rolname in ('synveda_gateway', 'synveda_worker')",
    "database.datname = 'keycloak' and owner.rolname = 'keycloak'",
    `'o'::"char"`,
    `'a'::"char"`,
  ]) {
    assert.ok(localAuthority.includes(token), `local authority lacks ${token}`);
  }
  assert.match(
    localAuthority,
    /dependency\.dbid = target\.oid[\s\S]*and not exists \([\s\S]*allowed\.deptype = dependency\.deptype/,
  );

  for (const dockerfile of [
    join(COMPOSE, "postgres/Dockerfile"),
    join(ROOT, "deploy/helm/postgres/Dockerfile"),
  ]) {
    const image = readFileSync(dockerfile, "utf8");
    const isHelmImage = dockerfile.includes("deploy/helm/postgres");
    assert.doesNotMatch(image, /deploy\/compose\/postgres\/initdb/);
    if (isHelmImage) {
      assert.doesNotMatch(image, /docker-entrypoint-initdb\.d|development-initdb\.sql/);
    } else {
      const development = image.indexOf("FROM runtime AS development\n");
      const reference = image.indexOf("FROM runtime AS reference\n");
      const developmentCopy =
        "COPY --chmod=0444 deploy/compose/postgres/development-initdb.sql " +
        "/docker-entrypoint-initdb.d/01-synveda-extensions.sql";
      assert.ok(development >= 0 && reference > development);
      assert.equal(occurrenceCount(image, developmentCopy), 1);
      assert.ok(image.slice(development, reference).includes(developmentCopy));
      assert.doesNotMatch(image.slice(reference), /docker-entrypoint-initdb\.d|development-initdb\.sql/);
    }
    const expectedBuilder = isHelmImage
      ? "FROM rust:1.96.0-bullseye@sha256:7069898d5edfc11b0ba498ecefbcc5438f6390b3ce0be11a9750cf39cab7e02f AS snapshot-builder"
      : "FROM rust:1.96.0-bookworm@sha256:5e2214abe154fe26e39f64488952e5c991eeed1d6d6da7cc8381ae83927f0cfc AS snapshot-builder";
    assert.equal(occurrenceCount(image, expectedBuilder), 1);
    const expectedPgvector = isHelmImage
      ? "postgresql-17-pgvector=0.8.6-1.pgdg11+1"
      : "postgresql-17-pgvector=0.8.6-1.pgdg12+1";
    assert.equal(occurrenceCount(image, expectedPgvector), 1);
    assert.doesNotMatch(image, /apt-get install[^\n]*(?:gcc|libc6-dev)/);
    for (const line of [
      "COPY --chmod=0555 deploy/compose/postgres/synveda-database-bootstrap /usr/local/bin/synveda-database-bootstrap",
      "COPY deploy/compose/postgres/synveda-input-snapshot.c /tmp/synveda-input-snapshot.c",
      "COPY --from=snapshot-builder --chmod=0555 /usr/local/bin/synveda-input-snapshot /usr/local/bin/synveda-input-snapshot",
      "COPY --chmod=0444 deploy/compose/postgres/synveda-cluster-authority-contract.sql /usr/local/share/synveda/cluster-authority-contract.sql",
      "COPY --chmod=0444 deploy/compose/postgres/synveda-local-authority-contract.sql /usr/local/share/synveda/local-authority-contract.sql",
      "COPY --chmod=0444 deploy/compose/postgres/synveda-extension-contract.sql /usr/local/share/synveda/extension-contract.sql",
      "COPY --chmod=0444 crates/synveda-store/sql/extension_fingerprint.sql /usr/local/share/synveda/extension-fingerprint.sql",
      "COPY --chmod=0444 deploy/compose/postgres/synveda-credential-log-contract.sql /usr/local/share/synveda/credential-log-contract.sql",
      "cp /usr/local/share/synveda/extension-fingerprint.sql \\",
      "printf '%s\\n' '\\gset synveda_extension_' \\",
      ">> /usr/local/share/synveda/extension-fingerprint-assert.psql;",
      "chmod 0444 /usr/local/share/synveda/extension-fingerprint-assert.psql",
      "test -x /usr/bin/timeout;",
      "install -d -m 0555 /usr/local/share/synveda;",
      "cc -std=c11 -O2 -Wall -Wextra -Werror \\",
      "printf 'snapshot-probe\\n' > /tmp/synveda-snapshot-input \\",
      "/usr/local/bin/synveda-input-snapshot \\",
    ]) {
      assert.equal(occurrenceCount(image, line), 1, `${dockerfile} does not contain exactly ${line}`);
    }
  }
	  const helmPostgres = readFileSync(join(ROOT, "deploy/helm/postgres/Dockerfile"), "utf8");
	  assert.match(
	    helmPostgres,
	    /ARG CNPG_BASE=ghcr\.io\/cloudnative-pg\/postgresql:17@sha256:fa6e2b2e14d19a109cc142cf857d328420bb7f1656b08c96e08be377692247ab/,
	  );
});

test("the mounted-input snapshot helper opens one bounded non-following descriptor", async () => {
  const source = readFileSync(INPUT_SNAPSHOT, "utf8");
  for (const token of [
    "lstat(argv[1], &path_before)",
    "O_RDONLY | O_NONBLOCK | O_NOFOLLOW | O_CLOEXEC",
    "fstat(source, &opened_before)",
    "same_source(&path_before, &opened_before)",
    "SNAPSHOT_MAX_BYTES + 1U",
    "same_source(&opened_before, &opened_after)",
    "same_source(&opened_after, &path_after)",
    "O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC",
	    "fsync(destination)",
	    "destination_created = 1",
	    "if (!success && destination_created)",
    "(void)unlink(argv[2])",
  ]) {
    assert.ok(source.includes(token), `snapshot helper lacks ${token}`);
  }
  assert.doesNotMatch(source, /(?:printf|fprintf|perror|strerror)\s*\(/);

  const scratch = mkdtempSync(join(tmpdir(), "synveda-input-snapshot-"));
  try {
    const helper = join(scratch, "synveda-input-snapshot");
    execFileSync("cc", [
      "-std=c11",
      "-O2",
      "-Wall",
      "-Wextra",
      "-Werror",
      INPUT_SNAPSHOT,
      "-o",
      helper,
    ]);
    const input = join(scratch, "input");
    const output = join(scratch, "output");
    const exact = Buffer.alloc(4096, 0x61);
    writeFileSync(input, exact, { mode: 0o600 });
    let result = spawnSync(helper, [input, output], { encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(readFileSync(output), exact);
    assert.equal(statSync(output).mode & 0o777, 0o600);

    rmSync(output);
    writeFileSync(input, Buffer.alloc(4097, 0x62), { mode: 0o600 });
    result = spawnSync(helper, [input, output], { encoding: "utf8" });
    assert.notEqual(result.status, 0);
    assert.equal(`${result.stdout}${result.stderr}`, "");
    assert.equal(existsSync(output), false);

	    const target = join(scratch, "target");
	    const preserved = Buffer.from("preexisting-output-sentinel");
	    writeFileSync(output, preserved, { mode: 0o600 });
	    writeFileSync(target, "snapshot-secret-sentinel", { mode: 0o600 });
    rmSync(input);
    execFileSync("ln", ["-s", target, input]);
    result = spawnSync(helper, [input, output], { encoding: "utf8" });
	    assert.notEqual(result.status, 0);
	    assert.equal(`${result.stdout}${result.stderr}`, "");
	    assert.deepEqual(readFileSync(output), preserved);

    rmSync(input);
    execFileSync("mkfifo", [input]);
    const started = Date.now();
    result = spawnSync(helper, [input, output], { encoding: "utf8", timeout: 2000 });
    assert.notEqual(result.status, 0);
    assert.equal(result.signal, null, "writerless FIFO reached the test timeout");
	    assert.ok(Date.now() - started < 2000, "writerless FIFO blocked the helper");
	    assert.equal(`${result.stdout}${result.stderr}`, "");
	    assert.deepEqual(readFileSync(output), preserved);

	    rmSync(input);
	    writeFileSync(input, Buffer.alloc(32, 0x63), { mode: 0o600 });
	    result = spawnSync(helper, [input, output], { encoding: "utf8" });
	    assert.notEqual(result.status, 0);
	    assert.equal(`${result.stdout}${result.stderr}`, "");
	    assert.deepEqual(readFileSync(output), preserved);

	    const forcedFailureSource = source
	      .replace(
	        "int main(int argc, char **argv) {",
	        "static int test_force_failure(int descriptor) {\n" +
	          "    (void)fsync(descriptor);\n" +
	          "    return 1;\n" +
	          "}\n\n" +
	          "int main(int argc, char **argv) {",
	      )
	      .replace("fsync(destination) < 0", "test_force_failure(destination)");
	    const forcedFailureC = join(scratch, "forced-failure.c");
	    const forcedFailureHelper = join(scratch, "forced-failure");
	    writeFileSync(forcedFailureC, forcedFailureSource, { mode: 0o600 });
	    execFileSync("cc", [
	      "-std=c11",
	      "-O2",
	      "-Wall",
	      "-Wextra",
	      "-Werror",
	      forcedFailureC,
	      "-o",
	      forcedFailureHelper,
	    ]);
	    const partial = join(scratch, "partial");
	    result = spawnSync(forcedFailureHelper, [input, partial], { encoding: "utf8" });
	    assert.notEqual(result.status, 0);
	    assert.equal(`${result.stdout}${result.stderr}`, "");
	    assert.equal(existsSync(partial), false, "helper-owned partial output survived failure");

	    const marker = join(scratch, "source-opened");
	    const raceSource = source.replace(
	      "    while (length < sizeof(bytes)) {",
	      `    int marker = open(${JSON.stringify(marker)}, O_WRONLY | O_CREAT | O_EXCL, S_IRUSR | S_IWUSR);\n` +
	        "    if (marker >= 0) { (void)close(marker); }\n" +
	        "    (void)sleep(1);\n\n" +
	        "    while (length < sizeof(bytes)) {",
	    );
	    const raceC = join(scratch, "race.c");
	    const raceHelper = join(scratch, "race-helper");
	    writeFileSync(raceC, raceSource, { mode: 0o600 });
	    execFileSync("cc", [
	      "-std=c11",
	      "-O2",
	      "-Wall",
	      "-Wextra",
	      "-Werror",
	      raceC,
	      "-o",
	      raceHelper,
	    ]);
	    const raceOutput = join(scratch, "race-output");
	    const waitForMarker = async (child) => {
	      for (let attempt = 0; attempt < 100; attempt += 1) {
	        if (existsSync(marker)) return;
	        if (child.exitCode !== null) break;
	        await new Promise((resolvePromise) => setTimeout(resolvePromise, 10));
	      }
	      child.kill("SIGKILL");
	      throw new Error("instrumented snapshot helper did not reach the opened-source marker");
	    };
	    const runRace = async (mutate) => {
	      rmSync(marker, { force: true });
	      rmSync(raceOutput, { force: true });
	      const child = spawn(raceHelper, [input, raceOutput], {
	        stdio: ["ignore", "pipe", "pipe"],
	      });
	      const stdout = [];
	      const stderr = [];
	      child.stdout.on("data", (chunk) => stdout.push(chunk));
	      child.stderr.on("data", (chunk) => stderr.push(chunk));
	      await waitForMarker(child);
	      mutate();
	      const [code, signal] = await once(child, "close");
	      assert.notEqual(code, 0);
	      assert.equal(signal, null);
	      assert.equal(Buffer.concat(stdout).length + Buffer.concat(stderr).length, 0);
	      assert.equal(existsSync(raceOutput), false);
	    };

	    writeFileSync(input, Buffer.alloc(100, 0x64), { mode: 0o600 });
	    await runRace(() => writeFileSync(input, Buffer.alloc(101, 0x65), { mode: 0o600 }));

	    const replacedInput = join(scratch, "replaced-input");
	    writeFileSync(input, Buffer.alloc(100, 0x66), { mode: 0o600 });
	    await runRace(() => {
	      renameSync(input, replacedInput);
	      writeFileSync(input, Buffer.alloc(100, 0x67), { mode: 0o600 });
	    });
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

function databaseBootstrapOrderingFindings(source) {
  const findings = [];
  if (source.includes("pg_catalog.coalesce(")) {
    findings.push("database bootstrap schema-qualifies SQL COALESCE syntax as a function");
  }
  const distinctFunctionStart = source.indexOf("validate_distinct_credentials() {");
  const distinctFunctionEnd = source.indexOf("\n}\n\nvalidate_role_contract_file", distinctFunctionStart);
  const distinctFunction =
    distinctFunctionStart >= 0 && distinctFunctionEnd > distinctFunctionStart
      ? source.slice(distinctFunctionStart, distinctFunctionEnd)
      : "";
  for (const marker of [
    'first_value=$(read_secret database_credential "$first")',
    'candidate_value=$(read_secret database_credential "$candidate")',
    '[ "$first_value" = "$candidate_value" ]',
    "database-bootstrap: database credentials must be pairwise distinct",
    "unset candidate_value",
    "unset first_value",
  ]) {
    if (!distinctFunction.includes(marker)) {
      findings.push(`database credential comparison lacks ${marker}`);
    }
  }
  const synvedaValidatorStart = source.indexOf("    validate-synveda-passwords)");
  const keycloakValidatorStart = source.indexOf("    validate-keycloak-password)");
  const validatorEnd = source.indexOf("\nesac\n\nbundled_cluster=", keycloakValidatorStart);
  const synvedaValidator = source.slice(synvedaValidatorStart, keycloakValidatorStart);
  const keycloakValidator = source.slice(keycloakValidatorStart, validatorEnd);
  for (const marker of [
    "SYNVEDA_DATABASE_REQUIRE_KEYCLOAK_PASSWORD",
    "snapshot_bootstrap_password_for_validator",
    '"$snapshot_dir/postgres_bootstrap_password"',
    '"$snapshot_dir/synveda_migrator_password"',
    '"$snapshot_dir/synveda_gateway_password"',
    '"$snapshot_dir/synveda_worker_password"',
    '"$snapshot_dir/keycloak_database_password"',
  ]) {
    if (!synvedaValidator.includes(marker)) {
      findings.push(`Synveda credential-set validation lacks ${marker}`);
    }
  }
  for (const marker of [
    "snapshot_bootstrap_password_for_validator",
    '"$snapshot_dir/postgres_bootstrap_password"',
    '"$snapshot_dir/keycloak_database_password"',
    "validate_distinct_credentials",
  ]) {
    if (!keycloakValidator.includes(marker)) {
      findings.push(`Keycloak credential-set validation lacks ${marker}`);
    }
  }
  const externalRefusal = source.indexOf(
    "external PostgreSQL mutation is unavailable until the authenticated TLS bootstrap contract is implemented",
  );
  const roleContractSnapshot = source.indexOf(
    'snapshot_input "database role contract"',
  );
  if (!(externalRefusal >= 0 && externalRefusal < roleContractSnapshot)) {
    findings.push("external PostgreSQL mutation is not refused before mounted input reads");
  }
  const readiness = source.indexOf("attempt=1");
  const boundedPsql = source.indexOf(
    'command timeout --foreground --signal=TERM --kill-after=5s 300s psql "$@"',
  );
  const boundedReadiness = source.indexOf(
    'command timeout --foreground --signal=TERM --kill-after=1s 6s psql "$@"',
  );
  const readinessCall = source.indexOf(
    'while ! bootstrap_psql_readiness --dbname="$bootstrap_url"',
  );
  const readinessAttempts = source.indexOf('if [ "$attempt" -ge 20 ]; then');
  if (!(boundedPsql >= 0 && boundedPsql < readiness)) {
    findings.push("bootstrap psql clients do not have the exact wall-clock deadline");
  }
  if (
    !(
      boundedReadiness >= 0 &&
      boundedReadiness < readiness &&
      readinessCall > readiness &&
      readinessAttempts > readinessCall
    )
  ) {
    findings.push("bootstrap readiness does not have the exact bounded retry deadline");
  }
  const pinnedSearchPath = source.indexOf(
    "export PGOPTIONS='-c search_path=pg_catalog",
  );
  if (!(pinnedSearchPath >= 0 && pinnedSearchPath < readiness)) {
    findings.push("bootstrap does not pin pg_catalog search_path before readiness");
  }
  const startupOptions = source.slice(pinnedSearchPath, readiness);
  for (const setting of [
    "-c role=none",
    "-c session_preload_libraries=",
    "-c local_preload_libraries=",
    "-c default_table_access_method=heap",
    "-c client_encoding=UTF8",
    "-c jit=off",
  ]) {
    if (!startupOptions.includes(setting)) {
      findings.push(`bootstrap does not neutralize ${setting.slice(3)} at connection startup`);
    }
  }
	  if (occurrenceCount(source, "reset role;") !== 14) {
    findings.push("bootstrap does not reset and prove the principal at every target boundary");
  }
  if (occurrenceCount(source, ") using heap;") !== 4 || occurrenceCount(source, ") using heap on commit drop;") !== 4) {
    findings.push("bootstrap does not pin every temporary staging table to heap");
  }
  const bootstrapPasswordRead = source.indexOf(
    "bootstrap_password=$(read_secret postgres_bootstrap_password",
  );
  for (const ambient of [
    "PGHOSTADDR",
    "PGHOST",
    "PGPORT",
    "PGUSER",
    "PGDATABASE",
    "PGSERVICE",
    "PGSERVICEFILE",
    "PGOPTIONS",
    "PGCONNECT_TIMEOUT",
    "PGSSLMODE",
    "PGSSLROOTCERT",
    "PGSSLCERTMODE",
    "PGGSSENCMODE",
    "PGGSSDELEGATION",
    "PGCHANNELBINDING",
    "PGTARGETSESSIONATTRS",
  ]) {
    const guard = source.indexOf(`[ "\${${ambient}+x}" = x ]`);
    if (!(guard >= 0 && guard < bootstrapPasswordRead)) {
      findings.push(`bootstrap does not refuse ambient ${ambient} before reading its password`);
    }
  }
  if (source.includes("printf '*:*:*:*:'")) {
    findings.push("bootstrap pgpass still contains wildcard routing fields");
  }
  for (const field of [
    '"$bootstrap_host"',
    '"$bootstrap_port"',
    '"$pgpass_database"',
    '"$bootstrap_user"',
  ]) {
    if (!source.includes(field)) findings.push(`bootstrap pgpass omits exact field ${field}`);
  }
  if (
    occurrenceCount(
      source,
      "\\getenv synveda_bootstrap_database SYNVEDA_POSTGRES_BOOTSTRAP_DATABASE",
    ) !== 7
  ) {
    findings.push("bootstrap does not import its exact maintenance database in every SQL session");
  }
  if (
    occurrenceCount(
      source,
      "\\getenv bootstrap_system_identifier SYNVEDA_POSTGRES_BOOTSTRAP_SYSTEM_IDENTIFIER",
    ) !== 7
  ) {
    findings.push("bootstrap does not re-prove its pinned cluster in every mutating SQL session");
  }
  const quarantineStart = source.indexOf("quarantine_keycloak_database() {");
  const quarantineEnd = source.indexOf("\nattempt=1", quarantineStart);
  const quarantine =
    quarantineStart >= 0 && quarantineEnd > quarantineStart
      ? source.slice(quarantineStart, quarantineEnd)
      : "";
  const quarantineClusterLock = quarantine.indexOf(
    "pg_advisory_lock(pg_catalog.hashtext('synveda.compose.bootstrap.cluster'))",
  );
  const quarantineSystemImport = quarantine.indexOf(
    "\\getenv bootstrap_system_identifier SYNVEDA_POSTGRES_BOOTSTRAP_SYSTEM_IDENTIFIER",
  );
  const quarantineDatabaseImport = quarantine.indexOf(
    "\\getenv synveda_bootstrap_database SYNVEDA_POSTGRES_BOOTSTRAP_DATABASE",
  );
  const quarantineOidImport = quarantine.indexOf(
    "\\getenv keycloak_database_oid SYNVEDA_KEYCLOAK_DATABASE_OID",
  );
  const quarantineIdentity = quarantine.indexOf(
    "control.system_identifier::text = :'bootstrap_system_identifier'",
  );
  const quarantineBegin = quarantine.indexOf("begin;", quarantineIdentity);
  const quarantineNoLogin = quarantine.indexOf(
    "alter role keycloak nologin;",
    quarantineBegin,
  );
  const quarantineClose = quarantine.indexOf(
    "alter database keycloak allow_connections false;",
    quarantineNoLogin,
  );
  const quarantineTransactionalOid = quarantine.indexOf(
    "database.oid = :'keycloak_database_oid'::pg_catalog.oid",
    quarantineClose,
  );
  const quarantineCommit = quarantine.indexOf("commit;", quarantineTransactionalOid);
  const quarantineStartupTerminate = quarantine.indexOf(
    "pg_terminate_backend(startup.pid, 5000)",
    quarantineCommit,
  );
  const quarantineActivityTerminate = quarantine.indexOf(
    "pg_terminate_backend(activity.pid, 5000)",
    quarantineStartupTerminate,
  );
  const quarantineActivityTerminationTarget = quarantine.indexOf(
    "where activity.datid = :'keycloak_database_oid'::pg_catalog.oid",
    quarantineActivityTerminate,
  );
  const quarantineFirstZeroStartup = quarantine.indexOf(
    "from pg_catalog.pg_locks lock",
    quarantineActivityTerminationTarget,
  );
  const quarantineFirstSnapshotClear = quarantine.indexOf(
    "pg_catalog.pg_stat_clear_snapshot()",
    quarantineFirstZeroStartup,
  );
  const quarantineFirstZeroActivity = quarantine.indexOf(
    "where activity.datid = :'keycloak_database_oid'::pg_catalog.oid",
    quarantineFirstSnapshotClear,
  );
  const quarantineFirstZeroPrepared = quarantine.indexOf(
    "from pg_catalog.pg_prepared_xacts prepared",
    quarantineFirstZeroActivity,
  );
  const quarantineSecondBegin = quarantine.indexOf("begin;", quarantineFirstZeroPrepared);
  const quarantineSecondNoLogin = quarantine.indexOf(
    "alter role keycloak nologin;",
    quarantineSecondBegin,
  );
  const quarantineSecondClose = quarantine.indexOf(
    "alter database keycloak allow_connections false;",
    quarantineSecondNoLogin,
  );
  const quarantineSecondOid = quarantine.indexOf(
    "database.oid = :'keycloak_database_oid'::pg_catalog.oid",
    quarantineSecondClose,
  );
  const quarantineSecondCommit = quarantine.indexOf("commit;", quarantineSecondOid);
  const quarantineFinalZeroStartup = quarantine.indexOf(
    "from pg_catalog.pg_locks lock",
    quarantineSecondCommit,
  );
  const quarantineFinalSnapshotClear = quarantine.indexOf(
    "pg_catalog.pg_stat_clear_snapshot()",
    quarantineFinalZeroStartup,
  );
  const quarantineFinalZeroActivity = quarantine.indexOf(
    "where activity.datid = :'keycloak_database_oid'::pg_catalog.oid",
    quarantineFinalSnapshotClear,
  );
  const quarantineFinalZeroPrepared = quarantine.indexOf(
    "from pg_catalog.pg_prepared_xacts prepared",
    quarantineFinalZeroActivity,
  );
  const quarantineFinalShape = quarantine.indexOf(
    "database.oid = :'keycloak_database_oid'::pg_catalog.oid",
    quarantineFinalZeroPrepared,
  );
  if (
    !(
      quarantineClusterLock >= 0 &&
      quarantineSystemImport > quarantineClusterLock &&
      quarantineDatabaseImport > quarantineClusterLock &&
      quarantineOidImport > quarantineClusterLock &&
      quarantineIdentity > quarantineOidImport &&
      quarantineBegin > quarantineIdentity &&
      quarantineNoLogin > quarantineBegin &&
      quarantineClose > quarantineNoLogin &&
      quarantineTransactionalOid > quarantineClose &&
      quarantineCommit > quarantineTransactionalOid &&
      quarantineStartupTerminate > quarantineCommit &&
      quarantineActivityTerminate > quarantineStartupTerminate &&
      quarantineActivityTerminationTarget > quarantineActivityTerminate &&
      quarantineFirstZeroStartup > quarantineActivityTerminationTarget &&
      quarantineFirstSnapshotClear > quarantineFirstZeroStartup &&
      quarantineFirstZeroActivity > quarantineFirstSnapshotClear &&
      quarantineFirstZeroPrepared > quarantineFirstZeroActivity &&
      quarantineSecondBegin > quarantineFirstZeroPrepared &&
      quarantineSecondNoLogin > quarantineSecondBegin &&
      quarantineSecondClose > quarantineSecondNoLogin &&
      quarantineSecondOid > quarantineSecondClose &&
      quarantineSecondCommit > quarantineSecondOid &&
      quarantineFinalZeroStartup > quarantineSecondCommit &&
      quarantineFinalSnapshotClear > quarantineFinalZeroStartup &&
      quarantineFinalZeroActivity > quarantineFinalSnapshotClear &&
      quarantineFinalZeroPrepared > quarantineFinalZeroActivity &&
      quarantineFinalShape > quarantineFinalZeroPrepared
    )
  ) {
    findings.push(
      "Keycloak quarantine does not maintenance-lock, close, drain, reclose and prove zero sessions in order",
    );
  }
  if (
    quarantine.includes("synveda.compose.bootstrap.keycloak") ||
    quarantine.includes("\\connect keycloak")
  ) {
    findings.push("Keycloak quarantine claims a target-database lock from maintenance scope");
  }
  const quarantineIdentityGuard = quarantine.slice(quarantineIdentity, quarantineBegin);
  for (const token of [
    "database.oid = :'keycloak_database_oid'::pg_catalog.oid",
    "database.datname = 'keycloak'",
    "owner.rolname = 'keycloak'",
    "database.datallowconn",
  ]) {
    if (!quarantineIdentityGuard.includes(token)) {
      findings.push(`Keycloak quarantine pre-mutation identity lacks ${token}`);
    }
  }
  for (const token of [
    "not pg_catalog.pg_is_in_recovery()",
    "pg_catalog.current_setting('transaction_read_only') = 'off'",
    "pg_catalog.current_database() = :'synveda_bootstrap_database'",
    "current_user = session_user",
    "pg_catalog.current_setting('role') = 'none'",
    "pg_catalog.current_setting('shared_preload_libraries') = ''",
    "pg_catalog.current_setting('session_preload_libraries') = ''",
    "pg_catalog.current_setting('local_preload_libraries') = ''",
    "pg_catalog.current_setting('default_table_access_method') = 'heap'",
    "pg_catalog.current_setting('client_encoding') = 'UTF8'",
    "pg_catalog.current_setting('jit') = 'off'",
    "not exists (select 1 from pg_catalog.pg_event_trigger)",
    "database.datname = 'keycloak'",
    "owner.rolname = 'keycloak'",
    "not database.datallowconn",
    "not role.rolcanlogin",
    "left join pg_catalog.pg_roles grantee on grantee.oid = acl.grantee",
    "acl.privilege_type in ('CREATE', 'CONNECT', 'TEMPORARY')",
    "acl.privilege_type = 'CONNECT'",
    "and not acl.is_grantable",
    "member.rolname = session_user",
    "granted.rolname = 'keycloak'",
  ]) {
    if (!quarantine.includes(token)) {
      findings.push(`Keycloak quarantine identity or closure proof lacks ${token}`);
    }
  }
  if (quarantine.includes("activity.pid <> pg_catalog.pg_backend_pid()")) {
    findings.push("Keycloak quarantine excludes a target backend from termination");
  }
  if (
    occurrenceCount(quarantine, "pg_catalog.pg_stat_clear_snapshot()") !== 2 ||
    occurrenceCount(quarantine, "from pg_catalog.pg_prepared_xacts prepared") !== 2 ||
    occurrenceCount(quarantine, "from pg_catalog.pg_locks lock") !== 3 ||
    occurrenceCount(quarantine, "lock.locktype = 'object'") !== 3 ||
    occurrenceCount(quarantine, "lock.database = 0") !== 3 ||
    occurrenceCount(
      quarantine,
      "lock.classid = 'pg_catalog.pg_database'::pg_catalog.regclass",
    ) !== 3 ||
    occurrenceCount(
      quarantine,
      "lock.objid = :'keycloak_database_oid'::pg_catalog.oid",
    ) !== 3 ||
    occurrenceCount(quarantine, "lock.objsubid = 0") !== 3 ||
    occurrenceCount(quarantine, "lock.mode = 'RowExclusiveLock'") !== 3 ||
    occurrenceCount(quarantine, "lock.pid is not null") !== 3 ||
    occurrenceCount(quarantine, "pg_catalog.bool_and(") !== 2 ||
    occurrenceCount(quarantine, "lock.pid <> pg_catalog.pg_backend_pid()") !== 1 ||
    occurrenceCount(quarantine, "alter role keycloak nologin;") !== 2 ||
    occurrenceCount(quarantine, "alter database keycloak allow_connections false;") !== 2
  ) {
    findings.push("Keycloak quarantine does not repeat its bounded drain and closure proof exactly");
  }
  if (quarantine.includes("lock.granted")) {
    findings.push("Keycloak quarantine filters the startup-lock population by grant state");
  }
  const clusterIdentity = source.indexOf("bootstrap_cluster_system_identifier=$(bootstrap_psql");
  const firstTargetBranch = source.indexOf('if [ "$target" = synveda ]; then');
  if (!(clusterIdentity > readiness && clusterIdentity < firstTargetBranch)) {
    findings.push("bootstrap does not pin a writable cluster identity before target inspection");
  }
  for (const credential of [
    "synveda_migrator_password",
    "synveda_gateway_password",
    "synveda_worker_password",
    "keycloak_database_password",
  ]) {
    if (!source.includes(`snapshot_input ${credential} /run/secrets/${credential}`)) {
      findings.push(`${credential} is not snapshotted through the bounded input helper`);
    }
    if (source.includes(`\\copy pg_temp.${credential}`) && source.includes(`from '/run/secrets/${credential}'`)) {
      findings.push(`${credential} is copied from its mutable mount`);
    }
  }
  const snapshotHelper = source.slice(
    source.indexOf("snapshot_input()"),
    source.indexOf("read_secret()"),
  );
  for (const required of [
    "command timeout --foreground --signal=TERM --kill-after=1s 5s",
	    "/usr/local/bin/synveda-input-snapshot",
	    'file_mode "$temporary"',
	    'file_owner "$temporary"',
	  ]) {
    if (!snapshotHelper.includes(required)) {
      findings.push(`bounded input snapshot omits ${required}`);
    }
  }
  if (snapshotHelper.includes("exec 3<") || snapshotHelper.includes("head -c 4097")) {
    findings.push("bounded input snapshot still performs a blocking shell open");
  }

  const synvedaBranch = source.slice(
    source.indexOf('if [ "$target" = synveda ]; then'),
    source.indexOf("else\n    keycloak_state_record="),
  );
  const keycloakBranch = source.slice(source.indexOf("else\n    keycloak_state_record="));
  const keycloakStateCapture = keycloakBranch.indexOf("keycloak_state_record=$(bootstrap_psql");
  const keycloakStateStartupProof = keycloakBranch.indexOf(
    "as keycloak_no_startup_locks",
    keycloakStateCapture,
  );
  const keycloakStateSnapshotClear = keycloakBranch.indexOf(
    "pg_catalog.pg_stat_clear_snapshot()",
    keycloakStateStartupProof,
  );
  const keycloakStateActivityProof = keycloakBranch.indexOf(
    "as keycloak_no_activity",
    keycloakStateSnapshotClear,
  );
  const keycloakStatePreparedProof = keycloakBranch.indexOf(
    "as keycloak_no_prepared_xacts",
    keycloakStateActivityProof,
  );
  const keycloakAtomicStateOid = keycloakBranch.indexOf(
    "end || ':' || coalesce(",
    keycloakStateCapture,
  );
  const keycloakClosedStartupUse = keycloakBranch.indexOf(
    ":'keycloak_no_startup_locks' = 'true'",
    keycloakStatePreparedProof,
  );
  const keycloakClosedActivityUse = keycloakBranch.indexOf(
    ":'keycloak_no_activity' = 'true'",
    keycloakClosedStartupUse,
  );
  const keycloakClosedPreparedUse = keycloakBranch.indexOf(
    ":'keycloak_no_prepared_xacts' = 'true'",
    keycloakClosedActivityUse,
  );
  const keycloakQuarantinedState = keycloakBranch.indexOf(
    "then 'quarantined'",
    keycloakStateCapture,
  );
  const keycloakQuarantinedCase = keycloakBranch.indexOf(
    "        quarantined)",
    keycloakQuarantinedState,
  );
  const keycloakQuarantinedEnd = keycloakBranch.indexOf(
    "        absent|closed)",
    keycloakQuarantinedCase,
  );
  const keycloakQuarantinedDrain = keycloakBranch.indexOf(
    "if ! quarantine_keycloak_database; then",
    keycloakQuarantinedCase,
  );
  const keycloakQuarantinedRefusal = keycloakBranch.indexOf(
    "interrupted Keycloak quarantine remains closed",
    keycloakQuarantinedDrain,
  );
  const keycloakQuarantinedExit = keycloakBranch.indexOf(
    "            exit 1",
    keycloakQuarantinedRefusal,
  );
  const keycloakInitialOidExport = keycloakBranch.indexOf(
    "export SYNVEDA_KEYCLOAK_INITIAL_STATE SYNVEDA_KEYCLOAK_INITIAL_DATABASE_OID",
    keycloakAtomicStateOid,
  );
  const keycloakEarlySystemImport = keycloakBranch.indexOf(
    "\\getenv bootstrap_system_identifier SYNVEDA_POSTGRES_BOOTSTRAP_SYSTEM_IDENTIFIER",
    keycloakInitialOidExport,
  );
  const keycloakEarlyOidImport = keycloakBranch.indexOf(
    "\\getenv keycloak_initial_database_oid SYNVEDA_KEYCLOAK_INITIAL_DATABASE_OID",
    keycloakEarlySystemImport,
  );
  const keycloakEarlyOidProof = keycloakBranch.indexOf(
    "database.oid = :'keycloak_initial_database_oid'::pg_catalog.oid",
    keycloakEarlyOidImport,
  );
  const keycloakEarlyAuthority = keycloakBranch.indexOf(
    "\\i /usr/local/share/synveda/local-authority-contract.sql",
    keycloakEarlyOidProof,
  );
  const keycloakFirstLocalGuard = keycloakBranch.indexOf(
    "Keycloak local authority preflight was refused",
    keycloakInitialOidExport,
  );
  const keycloakEarlyQuarantine = keycloakBranch.lastIndexOf(
    "if ! quarantine_keycloak_database; then",
    keycloakFirstLocalGuard,
  );
  const keycloakGlobalOidCapture = keycloakBranch.indexOf("keycloak_database_oid=$(");
  const keycloakGlobalLock = keycloakBranch.indexOf(
    "pg_advisory_lock(pg_catalog.hashtext('synveda.compose.bootstrap.cluster'))",
    keycloakGlobalOidCapture,
  );
  const keycloakGlobalOidSelect = keycloakBranch.indexOf(
    "select database.oid::bigint",
    keycloakGlobalLock,
  );
  const keycloakGlobalOidContinuity = keycloakBranch.indexOf(
    "or database.oid = :'keycloak_initial_database_oid'::pg_catalog.oid",
    keycloakGlobalOidSelect,
  );
  const keycloakLockedStateImport = keycloakBranch.indexOf(
    "\\getenv keycloak_initial_state SYNVEDA_KEYCLOAK_INITIAL_STATE",
    keycloakGlobalLock,
  );
  const keycloakLockedOidImport = keycloakBranch.indexOf(
    "\\getenv keycloak_initial_database_oid SYNVEDA_KEYCLOAK_INITIAL_DATABASE_OID",
    keycloakLockedStateImport,
  );
  const keycloakLockedContinuity = keycloakBranch.indexOf(
    "database.oid = :'keycloak_initial_database_oid'::pg_catalog.oid",
    keycloakLockedOidImport,
  );
  const keycloakGlobalMutation = keycloakBranch.indexOf(
    "-- MUTATION BOUNDARY: every persistent Keycloak",
    keycloakGlobalLock,
  );
  const keycloakGlobalOutputClose = keycloakBranch.indexOf(
    "\\o /dev/null",
    keycloakGlobalOidSelect,
  );
  const keycloakGlobalOpenBegin = keycloakBranch.indexOf(
    "begin;\nrevoke connect on database keycloak",
    keycloakGlobalOutputClose,
  );
  const keycloakGlobalOpen = keycloakBranch.indexOf(
    "alter database keycloak allow_connections true;",
    keycloakGlobalOpenBegin,
  );
  const keycloakGlobalFailure = keycloakBranch.indexOf(
    "Keycloak role or database convergence was refused",
    keycloakGlobalOpen,
  );
  const keycloakGlobalFailureCase = keycloakBranch.lastIndexOf(
    'case "$keycloak_database_oid" in',
    keycloakGlobalFailure,
  );
  const keycloakGlobalFailureUncaptured = keycloakBranch.indexOf(
    "Keycloak quarantine identity was not captured",
    keycloakGlobalFailureCase,
  );
  const keycloakGlobalFailureQuarantine = keycloakBranch.lastIndexOf(
    "if ! quarantine_keycloak_database; then",
    keycloakGlobalFailure,
  );
  const keycloakGlobalInvalid = keycloakBranch.indexOf(
    "Keycloak database identity was refused",
    keycloakGlobalFailure,
  );
  const keycloakGlobalInvalidPin = keycloakBranch.lastIndexOf(
    "pin_existing_keycloak_database_oid",
    keycloakGlobalInvalid,
  );
  const keycloakGlobalInvalidQuarantine = keycloakBranch.lastIndexOf(
    "if ! quarantine_keycloak_database; then",
    keycloakGlobalInvalid,
  );
  const keycloakOidExport = keycloakBranch.indexOf(
    "export SYNVEDA_KEYCLOAK_DATABASE_OID",
    keycloakGlobalOidSelect,
  );
  const keycloakTargetStart = keycloakBranch.indexOf(
    "keycloak_witness=",
    keycloakOidExport,
  );
  const keycloakTargetLock = keycloakBranch.indexOf(
    "pg_advisory_lock(pg_catalog.hashtext('synveda.compose.bootstrap.keycloak'))",
    keycloakTargetStart,
  );
  const keycloakTargetOidImport = keycloakBranch.indexOf(
    "\\getenv keycloak_database_oid SYNVEDA_KEYCLOAK_DATABASE_OID",
    keycloakTargetLock,
  );
  if (
    !(
      keycloakStateCapture >= 0 &&
      keycloakStateStartupProof > keycloakStateCapture &&
      keycloakStateSnapshotClear > keycloakStateStartupProof &&
      keycloakStateActivityProof > keycloakStateSnapshotClear &&
      keycloakStatePreparedProof > keycloakStateActivityProof &&
      keycloakAtomicStateOid > keycloakStatePreparedProof &&
      keycloakClosedStartupUse > keycloakStatePreparedProof &&
      keycloakClosedActivityUse > keycloakClosedStartupUse &&
      keycloakClosedPreparedUse > keycloakClosedActivityUse &&
      keycloakClosedPreparedUse < keycloakAtomicStateOid &&
      keycloakQuarantinedState > keycloakStateCapture &&
      keycloakQuarantinedCase > keycloakQuarantinedState &&
      keycloakQuarantinedEnd > keycloakQuarantinedCase &&
      keycloakQuarantinedDrain > keycloakQuarantinedCase &&
      keycloakQuarantinedDrain < keycloakQuarantinedEnd &&
      keycloakQuarantinedRefusal > keycloakQuarantinedDrain &&
      keycloakQuarantinedRefusal < keycloakQuarantinedEnd &&
      keycloakQuarantinedExit > keycloakQuarantinedRefusal &&
      keycloakQuarantinedExit < keycloakQuarantinedEnd &&
      keycloakQuarantinedExit < keycloakGlobalOidCapture &&
      keycloakInitialOidExport > keycloakAtomicStateOid &&
      keycloakEarlySystemImport > keycloakInitialOidExport &&
      keycloakEarlyOidImport > keycloakEarlySystemImport &&
      keycloakEarlyOidProof > keycloakEarlyOidImport &&
      keycloakEarlyAuthority > keycloakEarlyOidProof &&
      keycloakEarlyAuthority < keycloakFirstLocalGuard &&
      keycloakFirstLocalGuard > keycloakInitialOidExport &&
      keycloakEarlyQuarantine > keycloakInitialOidExport &&
      keycloakEarlyQuarantine < keycloakFirstLocalGuard &&
      keycloakGlobalOidCapture >= 0 &&
      keycloakGlobalLock > keycloakGlobalOidCapture &&
      keycloakLockedStateImport > keycloakGlobalLock &&
      keycloakLockedOidImport > keycloakLockedStateImport &&
      keycloakLockedContinuity > keycloakLockedOidImport &&
      keycloakGlobalMutation > keycloakLockedContinuity &&
      keycloakGlobalOidSelect > keycloakGlobalLock &&
      keycloakGlobalOidContinuity > keycloakGlobalOidSelect &&
      keycloakGlobalOutputClose > keycloakGlobalOidContinuity &&
      keycloakGlobalOpenBegin > keycloakGlobalOutputClose &&
      keycloakGlobalOpen > keycloakGlobalOpenBegin &&
      keycloakGlobalFailure > keycloakGlobalOpen &&
      keycloakGlobalFailureCase > keycloakGlobalOpen &&
      keycloakGlobalFailureUncaptured > keycloakGlobalFailureCase &&
      keycloakGlobalFailureQuarantine > keycloakGlobalFailureUncaptured &&
      keycloakGlobalFailureQuarantine < keycloakGlobalFailure &&
      keycloakGlobalInvalid > keycloakGlobalFailure &&
      keycloakGlobalInvalidPin > keycloakGlobalFailure &&
      keycloakGlobalInvalidQuarantine > keycloakGlobalInvalidPin &&
      keycloakGlobalInvalidQuarantine < keycloakGlobalInvalid &&
      keycloakOidExport > keycloakGlobalOidSelect &&
      keycloakTargetStart > keycloakOidExport &&
      keycloakTargetLock > keycloakTargetStart &&
      keycloakTargetOidImport > keycloakTargetLock
    )
  ) {
    findings.push(
      "Keycloak does not pin and re-prove its database OID across global, target and quarantine sessions",
    );
  }
  if (
    keycloakBranch
      .slice(keycloakGlobalOpen, keycloakGlobalFailure)
      .includes("pin_existing_keycloak_database_oid")
  ) {
    findings.push("Keycloak pre-open global refusal repins and quarantines an uncaptured target");
  }
  const keycloakStateClassifier = keycloakBranch.slice(
    keycloakStateCapture,
    keycloakAtomicStateOid,
  );
  if (keycloakStateClassifier.includes("lock.granted")) {
    findings.push("Keycloak initial state filters the startup-lock population by grant state");
  }
  if (
    occurrenceCount(
      source,
      "\\getenv keycloak_database_oid SYNVEDA_KEYCLOAK_DATABASE_OID",
    ) !== 2
  ) {
    findings.push("Keycloak database OID is not imported in the target and quarantine sessions");
  }

  for (const [name, branch, database, owner, targetLock] of [
    [
      "Synveda",
      synvedaBranch,
      "synveda",
      "synveda_migrator",
      "synveda.compose.bootstrap.synveda",
    ],
    [
      "Keycloak",
      keycloakBranch,
      "keycloak",
      "keycloak",
      "synveda.compose.bootstrap.keycloak",
    ],
  ]) {
    const topologyPredicates = databaseRoleTopologyPredicates(branch);
    const expectedTopologyPredicates = 2;
    if (
      topologyPredicates.length !== expectedTopologyPredicates ||
      new Set(topologyPredicates).size !== 1
    ) {
      findings.push(`${name} global and target role-contract predicates differ`);
    }
    const firstSafeShape = branch.indexOf("not database.dathasloginevt");
    const firstConnect = branch.indexOf(`\\connect ${database}`);
    const clusterLock = branch.indexOf("synveda.compose.bootstrap.cluster");
    const clusterIdentityGuard = branch.indexOf(
      "\\getenv bootstrap_system_identifier SYNVEDA_POSTGRES_BOOTSTRAP_SYSTEM_IDENTIFIER",
    );
    const mutation = branch.indexOf(`-- MUTATION BOUNDARY: every persistent ${name}`);
    const globalRoleContractCreate = branch.indexOf(
      "create temporary table pg_temp.synveda_database_roles",
      clusterLock,
    );
    const globalEventTriggerRefusal = branch.indexOf(
      "select 1 / case when not exists (\n  select 1 from pg_catalog.pg_event_trigger",
      clusterLock,
    );
    const passwordValidator = branch.indexOf(
      name === "Synveda"
        ? "\\! /usr/local/bin/synveda-database-bootstrap validate-synveda-passwords"
        : "\\! /usr/local/bin/synveda-database-bootstrap validate-keycloak-password",
      clusterLock,
    );
    const passwordValidatorStatus = branch.indexOf("\\if :SHELL_ERROR", passwordValidator);
    if (!(firstSafeShape >= 0 && firstSafeShape < firstConnect)) {
      findings.push(`${name} does not prove login-trigger absence before first connect`);
    }
    const initialShape = branch.slice(0, firstConnect);
    for (const token of [
      "database.datacl is null",
      "not database.datallowconn",
      "then 'closed'",
      "else 'terminal'",
      "administrator.rolname = session_user",
      "acl.grantor = database.datdba",
    ]) {
      if (!initialShape.includes(token)) {
        findings.push(`${name} initial database shape lacks ${token}`);
      }
    }
    const firstLocal = branch.slice(firstConnect, clusterLock);
    for (const token of [
      "pg_catalog.pg_event_trigger",
      "pg_catalog.pg_attribute",
      "pg_catalog.pg_default_acl",
      "pg_catalog.pg_largeobject_metadata",
      "namespace.nspname !~ '^pg_'",
      "\\i /usr/local/share/synveda/local-authority-contract.sql",
    ]) {
      if (!firstLocal.includes(token)) findings.push(`${name} first local guard lacks ${token}`);
    }
    if (!(clusterIdentityGuard >= 0 && clusterIdentityGuard < clusterLock && clusterLock < mutation)) {
      findings.push(`${name} mutation boundary does not follow the cluster lock`);
      continue;
    }
    if (
      !(
        globalEventTriggerRefusal > clusterLock &&
        globalEventTriggerRefusal < globalRoleContractCreate &&
        globalRoleContractCreate < mutation
      )
    ) {
      findings.push(`${name} global session creates temporary state before event-trigger refusal`);
    }
    const clusterGuard = branch.slice(clusterLock, mutation);
    for (const token of [
      `owner.rolname = '${owner}'`,
      "database.datallowconn",
      "not database.datistemplate",
      "not database.dathasloginevt",
      "database.datconnlimit = -1",
      "pg_catalog.pg_char_to_encoding('UTF8')",
      "database.datlocprovider",
      "database.datcollate",
      "database.datctype",
      "database.datlocale",
      "database.daticurules",
      "database.datcollversion",
      "database.dattablespace",
      "settings.setdatabase = database.oid",
      "pg_catalog.aclexplode",
      "pg_catalog.pg_default_acl",
      "pg_catalog.pg_largeobject_metadata",
      "pg_catalog.has_parameter_privilege",
      "shared_preload_libraries",
      "session_preload_libraries",
      "local_preload_libraries",
      "\\i /usr/local/share/synveda/cluster-authority-contract.sql",
    ]) {
      if (!clusterGuard.includes(token)) findings.push(`${name} cluster guard lacks ${token}`);
    }
    if (/settings\.setdatabase = database\.oid\s+and settings\.setrole = 0/.test(branch)) {
      findings.push(`${name} exact target shape ignores per-role database settings`);
    }
    for (const [setting] of CREDENTIAL_LOG_SETTINGS) {
      if (!clusterGuard.includes(`('${setting}')`)) {
        findings.push(`${name} cluster guard lacks SET authority for ${setting}`);
      }
    }
    if (
      !(
        passwordValidator > clusterLock &&
        passwordValidatorStatus > passwordValidator &&
        passwordValidatorStatus < mutation
      )
    ) {
      findings.push(`${name} password content is not checked after guards and before mutation`);
    }
    const createDatabase = branch.indexOf(`create database ${database}`, mutation);
    const createDatabaseEnd = branch.indexOf("\n)", createDatabase);
    if (
      createDatabase < mutation ||
      !branch.slice(createDatabase, createDatabaseEnd).includes("allow_connections false")
    ) {
      findings.push(`${name} database is not created closed`);
    }
    const normalAclBegin = branch.indexOf("begin;", createDatabase);
    const normalPublicRevoke = branch.indexOf(
      `revoke connect, temporary on database ${database} from public;`,
      normalAclBegin,
    );
    const normalAdministratorGrant = branch.indexOf(
      `select format('grant connect on database ${database} to %I', session_user)`,
      normalPublicRevoke,
    );
    const normalAllowConnections = branch.indexOf(
      `alter database ${database} allow_connections true;`,
      normalAdministratorGrant,
    );
    const normalOwnerCleanup = branch.indexOf(
      `revoke ${owner} from current_user granted by current_user;`,
      normalAllowConnections,
    );
    const normalAclCommit = branch.indexOf("commit;", normalOwnerCleanup);
    if (
      !(
        normalAclBegin > createDatabase &&
        normalPublicRevoke > normalAclBegin &&
        normalAdministratorGrant > normalPublicRevoke &&
        normalAllowConnections > normalAdministratorGrant &&
        normalOwnerCleanup > normalAllowConnections &&
        normalAclCommit > normalOwnerCleanup
      )
    ) {
      findings.push(`${name} normal ACL convergence is not one complete transaction`);
    }
    if (
      occurrenceCount(
        branch,
        "\\i /usr/local/share/synveda/cluster-authority-contract.sql",
      ) !== 3
    ) {
      findings.push(`${name} does not include exactly three cluster authority contracts`);
    }
    if (
      occurrenceCount(
        branch,
        "\\i /usr/local/share/synveda/credential-log-contract.sql",
      ) !== 2
    ) {
      findings.push(`${name} does not include exactly two credential log contracts`);
    }
    if (
      occurrenceCount(branch, "\\i /usr/local/share/synveda/local-authority-contract.sql") !== 2
    ) {
      findings.push(`${name} does not include exactly two local authority contracts`);
    }

    const targetConnect = branch.indexOf(`\\connect ${database}`, mutation);
    const firstCredentialLog = branch.indexOf(
      "\\i /usr/local/share/synveda/credential-log-contract.sql",
      targetConnect,
    );
    const targetClusterLock = branch.indexOf(
      "synveda.compose.bootstrap.cluster",
      firstCredentialLog,
    );
    const targetApplicationMarker = branch.indexOf(
      "'application_name', 'synveda-keycloak-bootstrap-target', false",
      targetClusterLock,
    );
    const targetClusterIdentityGuard = branch.lastIndexOf(
      "\\getenv bootstrap_system_identifier SYNVEDA_POSTGRES_BOOTSTRAP_SYSTEM_IDENTIFIER",
      targetClusterLock,
    );
    const targetLockPosition = branch.indexOf(targetLock, targetConnect);
    const targetBegin = branch.indexOf("begin;", targetLockPosition);
    const transactionalPrincipalGuard = branch.indexOf(
      "select 1 / case when current_user = session_user\n  and current_setting('role') = 'none'\n  and not exists (select 1 from pg_catalog.pg_event_trigger)",
      targetBegin,
    );
    if (
      !(
        targetConnect > mutation &&
        targetClusterIdentityGuard > targetConnect &&
        firstCredentialLog > targetConnect &&
        firstCredentialLog < targetClusterLock &&
        targetClusterLock < targetLockPosition &&
        targetLockPosition < targetBegin
      )
    ) {
      findings.push(`${name} credential logger is not closed before the target transaction`);
    }
    if (
      name === "Keycloak" &&
      !(
        targetApplicationMarker > targetClusterLock &&
        targetApplicationMarker < targetLockPosition
      )
    ) {
      findings.push("Keycloak target application marker is not between its ordered locks");
    }

    const alterSchema = branch.indexOf(`alter schema public owner to ${owner}`, targetLockPosition);
    const ownerGrant = branch.indexOf(`grant ${owner} to current_user`, targetLockPosition);
    const credentialCopy = branch.indexOf(
      name === "Synveda"
        ? "\\copy pg_temp.synveda_migrator_credential"
        : "\\copy pg_temp.keycloak_credential",
      targetLockPosition,
    );
    const targetRoleContract = branch.indexOf(
      "\\copy pg_temp.synveda_database_roles",
      targetLockPosition,
    );
    const targetRoleContractCreate = branch.indexOf(
      "create temporary table pg_temp.synveda_database_roles",
      targetLockPosition,
    );
    const targetEventTriggerRefusal = branch.indexOf(
      "select 1 / case when not exists (\n  select 1 from pg_catalog.pg_event_trigger",
      targetLockPosition,
    );
    const targetClusterProof = branch.indexOf(
      "\\i /usr/local/share/synveda/cluster-authority-contract.sql",
      targetRoleContract,
    );
    const secondCredentialLog = branch.indexOf(
      "\\i /usr/local/share/synveda/credential-log-contract.sql",
      firstCredentialLog + 1,
    );
    const isolationMarker =
      name === "Synveda"
        ? "pg_catalog.has_database_privilege('synveda_migrator', database.oid, 'CONNECT')"
        : "pg_catalog.has_database_privilege('keycloak', database.oid, 'CONNECT')";
    const finalIsolationCheck = branch.indexOf(isolationMarker, alterSchema);
    const finalIsolationEnd = branch.indexOf(") then 1 else 0 end;", finalIsolationCheck);
    const passwordEncryption = branch.indexOf(
      "set local password_encryption = 'scram-sha-256';",
      secondCredentialLog,
    );
    const credentialTable = branch.indexOf("create temporary table pg_temp.", passwordEncryption);
    if (
      !(
        ownerGrant > transactionalPrincipalGuard &&
        targetRoleContractCreate > ownerGrant &&
        alterSchema > targetClusterProof
      )
    ) {
      findings.push(`${name} target-local mutation does not follow its lock`);
      continue;
    }
    if (name === "Keycloak") {
      const initialStateExport = branch.indexOf(
        "SYNVEDA_KEYCLOAK_INITIAL_STATE=$keycloak_state",
      );
      const initialStateImport = branch.indexOf(
        "\\getenv keycloak_initial_state SYNVEDA_KEYCLOAK_INITIAL_STATE",
        targetBegin,
      );
      const pristineGuard = branch.slice(initialStateImport, ownerGrant);
      if (
        !(
          initialStateExport >= 0 &&
          initialStateExport < targetConnect &&
          initialStateImport > transactionalPrincipalGuard &&
          initialStateImport < ownerGrant
        )
      ) {
        findings.push("Keycloak closed-state classification is not carried into local preflight");
      }
      for (const token of [
        ":'keycloak_initial_state' = 'terminal'",
        "owner.rolname = 'pg_database_owner'",
        "dependency.refclassid = 'pg_catalog.pg_namespace'::regclass",
        "namespace.nspname = 'public'",
        "from pg_catalog.pg_shdepend dependency",
        "select 1 from pg_catalog.pg_default_acl",
        "select 1 from pg_catalog.pg_largeobject_metadata",
        "select 1 from pg_catalog.pg_foreign_data_wrapper",
        "select 1 from pg_catalog.pg_publication",
        "select 1 from pg_catalog.pg_subscription",
        "dependency.classid = 'pg_catalog.pg_cast'::regclass",
        "extension.extname = 'plpgsql'",
      ]) {
        if (!pristineGuard.includes(token)) {
          findings.push(`Keycloak closed recovery local guard lacks ${token}`);
        }
      }
      if (
        !source.includes("quarantine_keycloak_database() {") ||
        !source.includes("alter role keycloak nologin;") ||
        !source.includes("alter database keycloak allow_connections false;") ||
        !source.includes("if ! quarantine_keycloak_database; then")
      ) {
        findings.push("Keycloak target-local refusal is not fail-closed into quarantine");
      }
    }
    const targetCompletePhase = branch.lastIndexOf(
      "\\set synveda_require_complete_roles true",
      targetClusterProof,
    );
    const targetName = branch.lastIndexOf(
      `\\set synveda_bootstrap_target ${database}`,
      targetClusterProof,
    );
    const targetOwnerPhase = branch.lastIndexOf(
      "\\set synveda_allow_target_owner_membership true",
      targetClusterProof,
    );
    if (
      !(
        targetEventTriggerRefusal > targetLockPosition &&
        targetEventTriggerRefusal < targetBegin &&
        transactionalPrincipalGuard > targetBegin &&
        transactionalPrincipalGuard < ownerGrant &&
        ownerGrant < targetRoleContractCreate &&
        targetRoleContractCreate < targetRoleContract &&
        targetRoleContract > targetLockPosition &&
        targetClusterProof > targetRoleContract &&
        targetClusterProof > targetBegin &&
        targetCompletePhase > targetBegin &&
        targetCompletePhase < targetClusterProof &&
        targetName > targetBegin &&
        targetName < targetClusterProof &&
        targetOwnerPhase > targetName &&
        targetOwnerPhase < targetClusterProof &&
        targetClusterProof < alterSchema
      )
    ) {
      findings.push(`${name} does not repeat the exact cluster authority proof under the target lock`);
    }
    const targetGuard = branch.slice(ownerGrant, alterSchema);
    for (const token of [
      "pg_catalog.pg_event_trigger",
      "pg_catalog.pg_attribute",
      "pg_catalog.pg_default_acl",
      "pg_catalog.pg_largeobject_metadata",
      "namespace.nspname !~ '^pg_'",
      "\\i /usr/local/share/synveda/local-authority-contract.sql",
    ]) {
      if (!targetGuard.includes(token)) findings.push(`${name} target guard lacks ${token}`);
    }
    if (
      !(
        finalIsolationCheck > alterSchema &&
        finalIsolationEnd < secondCredentialLog &&
        secondCredentialLog < passwordEncryption &&
        passwordEncryption < credentialTable &&
        credentialTable < credentialCopy
      )
    ) {
      findings.push(`${name} credential logger is not reclosed immediately before COPY`);
    }
    if (!(credentialCopy > alterSchema)) {
      findings.push(`${name} credential COPY does not follow every non-secret target check`);
    }

    const ownerRevoke = branch.indexOf(
      `revoke ${owner} from session_user granted by session_user`,
      credentialCopy,
    );
    const commit = branch.indexOf("commit;", credentialCopy);
    if (!(ownerRevoke > credentialCopy && ownerRevoke < commit)) {
      findings.push(`${name} temporary owner membership is not removed before commit`);
    }

    if (name === "Synveda") {
      const extensionInclude = "\\i /usr/local/share/synveda/extension-contract.sql";
      const partialExtension = branch.indexOf(
        "\\set synveda_require_complete_extensions false",
        targetLockPosition,
      );
      const partialExtensionInclude = branch.indexOf(extensionInclude, partialExtension);
      const createExtension = branch.indexOf("create extension %I", ownerGrant);
      const completeExtension = branch.indexOf(
        "\\set synveda_require_complete_extensions true",
        createExtension,
      );
      const completeExtensionInclude = branch.indexOf(extensionInclude, completeExtension);
      if (occurrenceCount(branch, extensionInclude) !== 3) {
        findings.push("Synveda does not include exactly three extension contracts");
      }
      if (
        !(
          partialExtension > targetLockPosition &&
          partialExtension > ownerGrant &&
          partialExtensionInclude > partialExtension &&
          partialExtensionInclude < alterSchema
        )
      ) {
        findings.push("Synveda partial extension contract does not precede schema mutation");
      }
      if (
        !(
          createExtension > ownerGrant &&
          completeExtension > createExtension &&
          completeExtensionInclude > completeExtension &&
          completeExtensionInclude < secondCredentialLog
        )
      ) {
        findings.push("Synveda complete extension contract does not follow exact installation");
      }
    }
  }
  return findings;
}

test("the terminal Keycloak fixture proves one target-database blocker", () => {
  const source = readFileSync(DB_TEST, "utf8");
  assert.deepEqual(terminalKeycloakFenceFindings(source), []);
  const mutateFence = (mutator) => {
    const start = source.indexOf("# Hold only the Keycloak-database target lock.");
    const end = source.indexOf('if wait "$terminal_lock_process"; then', start);
    assert.ok(start >= 0 && end > start, "terminal Keycloak fence fixture is missing");
    return `${source.slice(0, start)}${mutator(source.slice(start, end))}${source.slice(end)}`;
  };
  for (const mutated of [
    mutateFence((branch) => branch.replace("--dbname keycloak", "--dbname postgres")),
    mutateFence((branch) =>
      branch.replace("activity.datname = 'keycloak'", "activity.datname = 'postgres'"),
    ),
    mutateFence((branch) => branch.replace("lock.database = activity.datid", "true")),
    mutateFence((branch) =>
      branch.replace(
        "activity.application_name = 'synveda-keycloak-bootstrap-target'",
        "activity.application_name = ''",
      ),
    ),
    mutateFence((branch) =>
      branch.replace(
        "lock.database = activity.datid and lock.granted) = 1",
        "lock.database = activity.datid) >= 0",
      ),
    ),
    mutateFence((branch) =>
      branch.replace(
        "lock.database = activity.datid and not lock.granted) = 1",
        "lock.database = activity.datid) >= 0",
      ),
    ),
    mutateFence((branch) =>
      branch.replace("pg_catalog.pg_blocking_pids(activity.pid)", "array[]::integer[]"),
    ),
    mutateFence((branch) =>
      branch.replace(
        "where datname = 'keycloak' and application_name = 'cpr45-keycloak-quarantine-lock'",
        "where datname = 'postgres' and application_name = 'cpr45-keycloak-quarantine-lock'",
      ),
    ),
  ]) {
    assert.ok(terminalKeycloakFenceFindings(mutated).length > 0);
  }
});

test("database acceptance covers post-open and pre-pgstat Keycloak quarantine", () => {
  const source = readFileSync(DB_TEST, "utf8");
  assert.deepEqual(keycloakAdmissionRecoveryFindings(source), []);
  const mutatePostOpen = (mutator) => {
    const start = source.indexOf("# A refusal after the global transaction opens Keycloak");
    const end = source.indexOf(
      "# Simulate interruption after the quarantine closure transaction commits",
      start,
    );
    assert.ok(start >= 0 && end > start, "post-open acceptance fixture is missing");
    return `${source.slice(0, start)}${mutator(source.slice(start, end))}${source.slice(end)}`;
  };
  const mutateResume = (mutator) => {
    const start = source.indexOf(
      "# Simulate interruption after the quarantine closure transaction commits",
    );
    const end = source.indexOf("# Global metadata cannot distinguish", start);
    assert.ok(start >= 0 && end > start, "crash-resume acceptance fixture is missing");
    return `${source.slice(0, start)}${mutator(source.slice(start, end))}${source.slice(end)}`;
  };
  for (const [name, mutated] of [
    [
      "ordered zero-proof COALESCE pseudo-function",
      source.replace("select coalesce((", "select pg_catalog.coalesce(("),
    ],
    [
      "ordered zero-proof startup class",
      source.replace(
        "lock.classid = 'pg_catalog.pg_database'::pg_catalog.regclass",
        "lock.classid = 0",
      ),
    ],
    [
      "post-open contract mount",
      mutatePostOpen((branch) => branch.replace(
        '$post_open_contract:/usr/local/share/synveda/cluster-authority-contract.sql:ro',
        '$post_open_contract:/tmp/cluster-authority-contract.sql:ro',
      )),
    ],
    [
      "post-open terminal refusal",
      mutatePostOpen((branch) => branch.replace(
        "database-bootstrap: Keycloak role or database convergence was refused",
        "database-bootstrap: removed post-open refusal",
      )),
    ],
    [
      "pre-pgstat delay",
      mutateResume((branch) =>
        branch.replace("PGOPTIONS='-c post_auth_delay=120'", "PGOPTIONS="),
      ),
    ],
    [
      "crash-resume closure",
      mutateResume((branch) => branch.replace(
        "begin;\nalter role keycloak nologin;\nalter database keycloak allow_connections false;\ncommit;",
        "begin;\nalter role keycloak login;\nalter database keycloak allow_connections true;\ncommit;",
      )),
    ],
    [
      "crash-resume refusal",
      mutateResume((branch) => branch.replace(
        "database-bootstrap: interrupted Keycloak quarantine remains closed",
        "database-bootstrap: removed crash-resume refusal",
      )),
    ],
  ]) {
    assert.notEqual(mutated, source, `${name} mutant did not alter the fixture`);
    assert.ok(
      keycloakAdmissionRecoveryFindings(mutated).length > 0,
      `database acceptance gate accepted ${name} mutation`,
    );
  }
});

test("database convergence proves an existing database shape before mutation", () => {
  const source = readFileSync(DATABASE_BOOTSTRAP, "utf8");
  assert.match(source, /^set -eu\n(?:#[^\n]*\n)*set \+x/m);
  assert.deepEqual(databaseBootstrapOrderingFindings(source), []);

  const removableTokens = [
    "reset role;",
    "snapshot_bootstrap_password_for_validator",
    "snapshot_input synveda_gateway_password",
    "SYNVEDA_DATABASE_REQUIRE_KEYCLOAK_PASSWORD",
    "validate_distinct_credentials",
    'first_value=$(read_secret database_credential "$first")',
    'candidate_value=$(read_secret database_credential "$candidate")',
    '[ "$first_value" = "$candidate_value" ]',
    "database-bootstrap: database credentials must be pairwise distinct",
    "not database.dathasloginevt",
    "database.datlocprovider",
    "database.datcollate",
    "database.datctype",
    "database.datlocale",
    "database.daticurules",
    "database.datcollversion",
    "database.dattablespace",
    "settings.setdatabase = database.oid",
    "pg_catalog.pg_event_trigger",
    "pg_catalog.pg_attribute",
    "pg_catalog.pg_default_acl",
    "pg_catalog.pg_largeobject_metadata",
    "namespace.nspname !~ '^pg_'",
    "SYNVEDA_KEYCLOAK_INITIAL_STATE",
    "SYNVEDA_KEYCLOAK_INITIAL_DATABASE_OID",
    "end || ':' || coalesce(",
    "dependency.classid = 'pg_catalog.pg_cast'::regclass",
    "quarantine_keycloak_database",
    "\\i /usr/local/share/synveda/local-authority-contract.sql",
    "\\i /usr/local/share/synveda/cluster-authority-contract.sql",
    "shared_preload_libraries",
    "\\i /usr/local/share/synveda/credential-log-contract.sql",
    "-- MUTATION BOUNDARY: every persistent Synveda",
  ];
  for (const [index, token] of removableTokens.entries()) {
    const mutated = source.replaceAll(token, `removed-token-${index}`);
    assert.ok(
      databaseBootstrapOrderingFindings(mutated).length > 0,
      `ordering gate accepted removal of ${token}`,
    );
  }

  const mutateQuarantine = (mutator) => {
    const start = source.indexOf("quarantine_keycloak_database() {");
    const end = source.indexOf("\nattempt=1", start);
    assert.ok(start >= 0 && end > start, "quarantine fixture is missing");
    return `${source.slice(0, start)}${mutator(source.slice(start, end))}${source.slice(end)}`;
  };
  const mutateKeycloakState = (mutator) => {
    const start = source.indexOf("keycloak_state_record=$(bootstrap_psql");
    const end = source.indexOf('\n    case "$keycloak_state_record"', start);
    assert.ok(start >= 0 && end > start, "Keycloak state classifier fixture is missing");
    return `${source.slice(0, start)}${mutator(source.slice(start, end))}${source.slice(end)}`;
  };
  const mutateQuarantinedCase = (mutator) => {
    const start = source.indexOf("        quarantined)");
    const end = source.indexOf("        absent|closed)", start);
    assert.ok(start >= 0 && end > start, "quarantined state fixture is missing");
    return `${source.slice(0, start)}${mutator(source.slice(start, end))}${source.slice(end)}`;
  };
  const quarantineClusterLock =
    "select pg_catalog.pg_advisory_lock(pg_catalog.hashtext('synveda.compose.bootstrap.cluster'));";
  const quarantineTargetLock =
    "select pg_catalog.pg_advisory_lock(pg_catalog.hashtext('synveda.compose.bootstrap.keycloak'));";
  const keycloakTargetMarker =
    "select pg_catalog.set_config(\n" +
    "  'application_name', 'synveda-keycloak-bootstrap-target', false\n" +
    ")\n" +
    "\\g /dev/null\n";

  for (const [name, mutated] of [
    [
      "database-bootstrap COALESCE pseudo-function",
      source.replace("select 1 / case when coalesce(", "select 1 / case when pg_catalog.coalesce("),
    ],
    [
      "Keycloak locked initial identity after mutation boundary",
      (() => {
        const initialOidImport =
          "\\getenv keycloak_initial_database_oid SYNVEDA_KEYCLOAK_INITIAL_DATABASE_OID\n";
        const withoutImport = replaceOccurrence(source, initialOidImport, 2, "");
        const boundary =
          "-- MUTATION BOUNDARY: every persistent Keycloak refusal above is read-only.\n";
        return withoutImport.replace(boundary, `${boundary}${initialOidImport}`);
      })(),
    ],
    [
      "missing Keycloak quarantine maintenance lock",
      mutateQuarantine((branch) =>
        branch.replace(quarantineClusterLock, "select true;"),
      ),
    ],
    [
      "spurious Keycloak quarantine target lock in maintenance scope",
      mutateQuarantine((branch) =>
        branch.replace(
          quarantineClusterLock,
          `${quarantineClusterLock}\n${quarantineTargetLock}`,
        ),
      ),
    ],
    [
      "missing Keycloak quarantine maintenance identity",
      mutateQuarantine((branch) =>
        branch.replace(
          "pg_catalog.current_database() = :'synveda_bootstrap_database'",
          "pg_catalog.current_database() <> :'synveda_bootstrap_database'",
        ),
      ),
    ],
    [
      "missing Keycloak quarantine system identity",
      mutateQuarantine((branch) =>
        branch.replace(
          "control.system_identifier::text = :'bootstrap_system_identifier'",
          "control.system_identifier::text <> :'bootstrap_system_identifier'",
        ),
      ),
    ],
    [
      "Keycloak quarantine commit before closure",
      mutateQuarantine((branch) => {
        const withoutCommit = replaceOccurrence(branch, "commit;", 0, "");
        return withoutCommit.replace(
          "alter role keycloak nologin;",
          "commit;\nalter role keycloak nologin;",
        );
      }),
    ],
    [
      "missing bounded Keycloak session termination",
      mutateQuarantine((branch) =>
        branch.replace(
          "pg_catalog.pg_terminate_backend(activity.pid, 5000)",
          "false",
        ),
      ),
    ],
    [
      "missing bounded Keycloak startup termination",
      mutateQuarantine((branch) =>
        branch.replace("pg_catalog.pg_terminate_backend(startup.pid, 5000)", "false"),
      ),
    ],
    [
      "missing first Keycloak startup-lock zero proof",
      mutateQuarantine((branch) =>
        replaceOccurrence(
          branch,
          "from pg_catalog.pg_locks lock",
          1,
          "from pg_catalog.pg_roles lock",
        ),
      ),
    ],
    [
      "missing final Keycloak startup-lock zero proof",
      mutateQuarantine((branch) =>
        replaceOccurrence(
          branch,
          "from pg_catalog.pg_locks lock",
          2,
          "from pg_catalog.pg_roles lock",
        ),
      ),
    ],
    [
      "Keycloak quarantine filters startup locks by grant state",
      mutateQuarantine((branch) =>
        branch.replace("and lock.pid is not null", "and lock.pid is not null\n       and lock.granted"),
      ),
    ],
    [
      "missing Keycloak drain zero-session barrier",
      mutateQuarantine((branch) =>
        replaceOccurrence(
          branch,
          "where activity.datid = :'keycloak_database_oid'::pg_catalog.oid",
          1,
          "where false",
        ),
      ),
    ],
    [
      "missing Keycloak second closure transaction",
      mutateQuarantine((branch) =>
        replaceOccurrence(branch, "alter role keycloak nologin;", 1, "select true;"),
      ),
    ],
    [
      "missing Keycloak prepared-transaction refusal",
      mutateQuarantine((branch) =>
        branch.replaceAll("from pg_catalog.pg_prepared_xacts prepared", "from pg_catalog.pg_roles prepared"),
      ),
    ],
    [
      "missing final Keycloak zero-session proof",
      mutateQuarantine((branch) =>
        replaceOccurrence(
          branch,
          "where activity.datid = :'keycloak_database_oid'::pg_catalog.oid",
          2,
          "where false",
        ),
      ),
    ],
    [
      "Keycloak closed classifier omits startup-lock proof",
      mutateKeycloakState((branch) =>
        branch.replace(":'keycloak_no_startup_locks' = 'true'", "true"),
      ),
    ],
    [
      "Keycloak closed classifier filters startup locks by grant state",
      mutateKeycloakState((branch) =>
        branch.replace(
          "and lock.pid is not null\n))::text as keycloak_no_startup_locks",
          "and lock.pid is not null\n     and lock.granted\n))::text as keycloak_no_startup_locks",
        ),
      ),
    ],
    [
      "quarantined Keycloak state omits its drain",
      mutateQuarantinedCase((branch) =>
        branch.replace("if ! quarantine_keycloak_database; then", "if ! true; then"),
      ),
    ],
    [
      "quarantined Keycloak state continues into convergence",
      mutateQuarantinedCase((branch) => {
        const refusal = branch.indexOf(
          "echo \"database-bootstrap: interrupted Keycloak quarantine remains closed\" >&2",
        );
        const exit = branch.indexOf("            exit 1\n", refusal);
        assert.ok(refusal >= 0 && exit > refusal, "quarantined terminal exit is missing");
        return `${branch.slice(0, exit)}${branch.slice(exit + "            exit 1\n".length)}`;
      }),
    ],
    [
      "Keycloak quarantine excludes one target backend",
      mutateQuarantine((branch) =>
        branch.replace(
          "where activity.datid = :'keycloak_database_oid'::pg_catalog.oid;",
          "where activity.datid = :'keycloak_database_oid'::pg_catalog.oid\n" +
            "   and activity.pid <> pg_catalog.pg_backend_pid();",
        ),
      ),
    ],
    [
      "Keycloak target marker after its target lock",
      (() => {
        const withoutMarker = source.replace(keycloakTargetMarker, "");
        return replaceOccurrence(
          withoutMarker,
          quarantineTargetLock,
          0,
          `${quarantineTargetLock}\n${keycloakTargetMarker.trimEnd()}`,
        );
      })(),
    ],
    [
      "Keycloak database OID captured after opening admission",
      (() => {
        const captureStart = source.indexOf("-- Capture the exact target before the transaction");
        const openBegin = source.indexOf(
          "begin;\nrevoke connect on database keycloak",
          captureStart,
        );
        assert.ok(captureStart >= 0 && openBegin > captureStart, "pre-open OID capture is missing");
        const capture = source.slice(captureStart, openBegin);
        const withoutCapture = `${source.slice(0, captureStart)}${source.slice(openBegin)}`;
        const openCommit = withoutCapture.indexOf("commit;", captureStart);
        assert.ok(openCommit > captureStart, "Keycloak open commit is missing");
        const insertion = openCommit + "commit;".length;
        return `${withoutCapture.slice(0, insertion)}\n${capture}${withoutCapture.slice(insertion)}`;
      })(),
    ],
    [
      "Keycloak global failure repins an uncaptured OID",
      source.replace(
        '        case "$keycloak_database_oid" in\n' +
          "            ''|0|*[!0-9]*|???????????*)\n" +
          '                echo "database-bootstrap: Keycloak quarantine identity was not captured" >&2',
        '        keycloak_database_oid=$(pin_existing_keycloak_database_oid) || keycloak_database_oid=\n' +
          '        case "$keycloak_database_oid" in\n' +
          "            ''|0|*[!0-9]*|???????????*)\n" +
          '                echo "database-bootstrap: Keycloak quarantine identity was not captured" >&2',
      ),
    ],
    [
      "Keycloak global failure omits quarantine",
      replaceOccurrence(
        source,
        "if ! quarantine_keycloak_database; then",
        2,
        "if ! true; then",
      ),
    ],
    [
      "Synveda owner grant immediately after BEGIN",
      (() => {
        const grant =
          "grant synveda_migrator to current_user\n  with admin false, inherit true, set true granted by current_user;\n";
        const withoutGrant = replaceOccurrence(source, grant, 1, "");
        const targetConnect = withoutGrant.indexOf("\\connect synveda", withoutGrant.indexOf("-- MUTATION BOUNDARY: every persistent Synveda"));
        const begin = withoutGrant.indexOf("begin;", targetConnect) + "begin;".length;
        return `${withoutGrant.slice(0, begin)}\n${grant}${withoutGrant.slice(begin + 1)}`;
      })(),
    ],
    [
      "Synveda cluster helper after mutation boundary",
      (() => {
        const include = "\\i /usr/local/share/synveda/cluster-authority-contract.sql\n";
        const withoutInclude = replaceOccurrence(source, include, 0, "");
        const boundary = "-- MUTATION BOUNDARY: every persistent Synveda refusal above is read-only.\n";
        return withoutInclude.replace(boundary, `${boundary}${include}`);
      })(),
    ],
    [
      "Synveda second logger before final isolation guard",
      (() => {
        const include = "\\i /usr/local/share/synveda/credential-log-contract.sql\n";
        const withoutInclude = replaceOccurrence(source, include, 1, "");
        const isolation = "-- The bundled cluster owns every database ACL.";
        return withoutInclude.replace(isolation, `${include}${isolation}`);
      })(),
    ],
    [
      "missing Synveda locked partial extension contract",
      replaceOccurrence(
        source,
        "\\i /usr/local/share/synveda/extension-contract.sql",
        1,
        "-- removed locked partial extension contract",
      ),
    ],
    [
      "missing Synveda complete extension contract",
      replaceOccurrence(
        source,
        "\\i /usr/local/share/synveda/extension-contract.sql",
        2,
        "-- removed complete extension contract",
      ),
    ],
    [
      "decoy cluster helper after both branches",
      `${replaceOccurrence(
        source,
        "\\i /usr/local/share/synveda/cluster-authority-contract.sql\n",
        0,
        "",
      )}\n\\i /usr/local/share/synveda/cluster-authority-contract.sql\n`,
    ],
    [
      "Synveda target external-provider predicate drift",
      replaceOccurrence(
        source,
        ":'bundled_cluster' <> 'true'\n       or contract.document->'forbidden_databases' in (",
        1,
        ":'bundled_cluster' = 'false'\n       or contract.document->'forbidden_databases' in (",
      ),
    ],
  ]) {
    assert.notEqual(mutated, source, `${name} mutant did not alter the fixture`);
    assert.ok(
      databaseBootstrapOrderingFindings(mutated).length > 0,
      `ordering gate accepted ${name}`,
    );
  }
});

test("model findings reject privilege, port, command and secret regressions", () => {
  const projectName = "fixture";
  const networkPlan = composeNetworkPlan("172.30.240.0/24");
  const network = (name, internal = false) => ({
    name: `${projectName}_${name}`,
    ipam: { config: [networkPlan[name]] },
    labels: {
      "com.synveda.contract": "cpr-45",
      "com.synveda.network": name,
    },
    ...(internal ? { internal: true } : {}),
  });
  const base = {
    secrets: {
      synveda_gateway_database_url: {
        file: "/fixture/secrets/synveda_gateway_database_url",
      },
      synveda_kms_key: { file: "/fixture/secrets/synveda_kms_key" },
      synveda_kms_key_ref: { file: "/fixture/secrets/synveda_kms_key_ref" },
      synveda_migrator_database_url: {
        file: "/fixture/secrets/synveda_migrator_database_url",
      },
      synveda_worker_database_url: {
        file: "/fixture/secrets/synveda_worker_database_url",
      },
    },
    services: {
      "database-preflight": {
        command: ["database-preflight"],
        image: "product",
        user: "1:1",
        cap_drop: ["ALL"],
        security_opt: ["no-new-privileges:true"],
        read_only: true,
        init: true,
        pids_limit: 1,
        restart: "no",
        environment: {
          SYNVEDA_MIGRATOR_DATABASE_URL_FILE:
            "/run/secrets/synveda_migrator_database_url",
          SYNVEDA_GATEWAY_DATABASE_URL_FILE:
            "/run/secrets/synveda_gateway_database_url",
          SYNVEDA_WORKER_DATABASE_URL_FILE:
            "/run/secrets/synveda_worker_database_url",
          SYNVEDA_DATABASE_ROLES_FILE: "/etc/synveda/database/roles.json",
          RUST_LOG: "info",
        },
        secrets: [
          {
            source: "synveda_migrator_database_url",
            target: "synveda_migrator_database_url",
          },
          {
            source: "synveda_gateway_database_url",
            target: "synveda_gateway_database_url",
          },
          {
            source: "synveda_worker_database_url",
            target: "synveda_worker_database_url",
          },
        ],
        volumes: [
          {
            type: "bind",
            source: "/fixture/database-roles.json",
            target: "/etc/synveda/database/roles.json",
            read_only: true,
          },
        ],
        networks: {
          "application-egress": { gw_priority: 1 },
          "synveda-data": {},
        },
        build: { dockerfile: "deploy/compose/gateway/Dockerfile" },
      },
      gateway: {
        command: ["gateway"],
        image: "product",
        user: "1:1",
        cap_drop: ["ALL"],
        security_opt: ["no-new-privileges:true"],
        read_only: true,
        init: true,
        pids_limit: 1,
        restart: "no",
        environment: {
          DATABASE_URL_FILE: "/run/secrets/database_url",
          SYNVEDA_KMS_KEY_FILE: "/run/secrets/kms_key",
          SYNVEDA_KMS_KEY_REF_FILE: "/run/secrets/kms_key_ref",
          SYNVEDA_PUBLIC_URL: "http://app.synveda.test:8080",
          SYNVEDA_INSECURE_DEVELOPMENT_HTTP: "true",
          SYNVEDA_OIDC_ISSUERS_FILE: "/etc/synveda/oidc/issuers.json",
          SYNVEDA_DATABASE_ROLES_FILE: "/etc/synveda/database/roles.json",
          SYNVEDA_LISTEN_ADDR: "0.0.0.0:8120",
          OTEL_EXPORTER_OTLP_ENDPOINT: "http://otel-collector:4317",
          RUST_LOG: "info",
        },
        healthcheck: {
          test: [
            "CMD",
            "/usr/local/bin/synveda-container",
            "probe",
            "gateway",
            "ready",
          ],
          interval: "5s",
          timeout: "3s",
          retries: 24,
        },
        depends_on: {
          migrate: { condition: "service_completed_successfully" },
          "issuer-diagnostic": { condition: "service_completed_successfully" },
        },
        secrets: [
          { source: "synveda_gateway_database_url", target: "database_url" },
          { source: "synveda_kms_key", target: "kms_key" },
          { source: "synveda_kms_key_ref", target: "kms_key_ref" },
        ],
        volumes: [
          {
            type: "bind",
            source: "/fixture/database-roles.json",
            target: "/etc/synveda/database/roles.json",
            read_only: true,
          },
          {
            type: "bind",
            source: "/fixture/issuers.json",
            target: "/etc/synveda/oidc/issuers.json",
            read_only: true,
          },
        ],
        networks: {
          "app-backend": {},
          "application-egress": { gw_priority: 1 },
          "synveda-data": {},
          telemetry: {},
        },
      },
      worker: {
        command: ["worker"],
        image: "product",
        user: "1:1",
        cap_drop: ["ALL"],
        security_opt: ["no-new-privileges:true"],
        read_only: true,
        init: true,
        pids_limit: 1,
        restart: "no",
        stop_grace_period: "1m25s",
        environment: {
          DATABASE_URL_FILE: "/run/secrets/database_url",
          SYNVEDA_KMS_KEY_FILE: "/run/secrets/kms_key",
          SYNVEDA_KMS_KEY_REF_FILE: "/run/secrets/kms_key_ref",
          SYNVEDA_DATABASE_ROLES_FILE: "/etc/synveda/database/roles.json",
          SYNVEDA_OIDC_ISSUERS_FILE: "/etc/synveda/oidc/issuers.json",
          SYNVEDA_WORKER_LISTEN_ADDR: "127.0.0.1:8121",
          OTEL_EXPORTER_OTLP_ENDPOINT: "http://otel-collector:4317",
          RUST_LOG: "info",
        },
        healthcheck: {
          test: [
            "CMD",
            "/usr/local/bin/synveda-container",
            "probe",
            "worker",
            "ready",
          ],
          interval: "5s",
          timeout: "3s",
          retries: 24,
        },
        depends_on: { migrate: { condition: "service_completed_successfully" } },
        secrets: [
          { source: "synveda_worker_database_url", target: "database_url" },
          { source: "synveda_kms_key", target: "kms_key" },
          { source: "synveda_kms_key_ref", target: "kms_key_ref" },
        ],
        volumes: [
          {
            type: "bind",
            source: "/fixture/database-roles.json",
            target: "/etc/synveda/database/roles.json",
            read_only: true,
          },
          {
            type: "bind",
            source: "/fixture/issuers.json",
            target: "/etc/synveda/oidc/issuers.json",
            read_only: true,
          },
          {
            type: "bind",
            source: "/fixture/oidc-directory",
            target: "/run/secrets/oidc_directory",
            read_only: true,
          },
        ],
        networks: {
          "application-egress": { gw_priority: 1 },
          "synveda-data": {},
          telemetry: {},
        },
      },
      "issuer-diagnostic": {
        command: ["issuer-diagnostic"],
        image: "product",
        user: "1:1",
        cap_drop: ["ALL"],
        security_opt: ["no-new-privileges:true"],
        read_only: true,
        init: true,
        pids_limit: 1,
        restart: "no",
        environment: {
          SYNVEDA_OIDC_ISSUERS_FILE: "/etc/synveda/oidc/issuers.json",
          SYNVEDA_OIDC_EXPECTED_ISSUER:
            "https://external-idp.example/tenant",
          SYNVEDA_PUBLIC_URL: "http://app.synveda.test:8080",
          SYNVEDA_INSECURE_DEVELOPMENT_HTTP: "true",
          RUST_LOG: "info",
        },
        depends_on: { proxy: { condition: "service_healthy" } },
        volumes: [
          {
            type: "bind",
            source: "/fixture/issuers.json",
            target: "/etc/synveda/oidc/issuers.json",
            read_only: true,
          },
        ],
        networks: {
          "app-backend": {},
          "application-egress": { gw_priority: 1 },
        },
        build: { dockerfile: "deploy/compose/gateway/Dockerfile" },
      },
      migrate: {
        command: ["migrate"],
        image: "product",
        user: "1:1",
        cap_drop: ["ALL"],
        security_opt: ["no-new-privileges:true"],
        read_only: true,
        init: true,
        pids_limit: 1,
        restart: "no",
        environment: {
          DATABASE_URL_FILE: "/run/secrets/database_url",
          SYNVEDA_DATABASE_ROLES_FILE: "/etc/synveda/database/roles.json",
          RUST_LOG: "info",
        },
        depends_on: {
          "database-preflight": { condition: "service_completed_successfully" },
        },
        secrets: [{ source: "synveda_migrator_database_url", target: "database_url" }],
        volumes: [
          {
            type: "bind",
            source: "/fixture/database-roles.json",
            target: "/etc/synveda/database/roles.json",
            read_only: true,
          },
        ],
        networks: {
          "application-egress": { gw_priority: 1 },
          "synveda-data": {},
        },
        build: { dockerfile: "deploy/compose/gateway/Dockerfile" },
      },
      proxy: {
        command: ["caddy", "run", "--config", "/etc/caddy/Caddyfile", "--adapter", "caddyfile"],
        image: "proxy-provider",
        user: "1:1",
        cap_drop: ["ALL"],
        security_opt: ["no-new-privileges:true"],
        read_only: true,
        init: true,
        pids_limit: 1,
        restart: "no",
        environment: {
          SYNVEDA_APP_HOST: "app.synveda.test",
          SYNVEDA_PUBLIC_PORT: "8080",
          SYNVEDA_PROXY_HTTP_PORT: "8080",
          SYNVEDA_PROXY_HTTPS_PORT: "8443",
          XDG_CONFIG_HOME: "/tmp/caddy-config",
          XDG_DATA_HOME: "/tmp/caddy-data",
        },
        healthcheck: {
          test: [
            "CMD",
            "caddy",
            "validate",
            "--config",
            "/etc/caddy/Caddyfile",
            "--adapter",
            "caddyfile",
          ],
          interval: "10s",
          timeout: "6s",
          retries: 6,
        },
        volumes: [
          {
            type: "bind",
            source: "/fixture/Caddyfile",
            target: "/etc/caddy/Caddyfile",
            read_only: true,
          },
          {
            type: "bind",
            source: "/fixture/app.caddy",
            target: "/etc/caddy/app.caddy",
            read_only: true,
          },
          {
            type: "bind",
            source: "/fixture/identity.external.caddy",
            target: "/etc/caddy/identity.caddy",
            read_only: true,
          },
        ],
        build: { dockerfile: "deploy/compose/proxy/Dockerfile" },
        ports: [{ host_ip: "127.0.0.1", published: "8080", target: 8080 }],
        networks: { "app-backend": {}, "public-edge": { gw_priority: 1 } },
      },
      "otel-collector": {
        command: ["--config=/etc/otelcol/config.yaml"],
        image: "collector-provider",
        user: "1:1",
        cap_drop: ["ALL"],
        security_opt: ["no-new-privileges:true"],
        read_only: true,
        init: true,
        pids_limit: 1,
        restart: "no",
        healthcheck: {
          test: [
            "CMD",
            "/otelcol-contrib",
            "validate",
            "--config=/etc/otelcol/config.yaml",
          ],
          interval: "30s",
          timeout: "5s",
          retries: 3,
        },
        volumes: [
          {
            type: "bind",
            source: "/fixture/collector.yaml",
            target: "/etc/otelcol/config.yaml",
            read_only: true,
          },
        ],
        networks: {
          "keycloak-management": {},
          telemetry: {},
          "telemetry-egress": { gw_priority: 1 },
        },
      },
    },
    networks: {
      "app-backend": network("app-backend", true),
      "application-egress": network("application-egress"),
      "keycloak-management": network("keycloak-management", true),
      "public-edge": network("public-edge"),
      "synveda-data": network("synveda-data", true),
      telemetry: network("telemetry", true),
      "telemetry-egress": network("telemetry-egress"),
    },
  };
  base.services.gateway.build = { dockerfile: "deploy/compose/gateway/Dockerfile" };
  base.services.worker.build = { dockerfile: "deploy/compose/gateway/Dockerfile" };
  const expected = {
    runtime: "development",
    postgres: "external",
    oidc: "external",
    appHost: "app.synveda.test",
    authHost: undefined,
    appUrl: "http://app.synveda.test:8080",
    authUrl: undefined,
    projectName,
    networkPool: "172.30.240.0/24",
    networkPlan,
    proxyIdentityAddress: "172.30.240.2",
    productImage: "product",
    postgresImage: "postgres-provider",
    keycloakImage: "keycloak-provider",
    caddyImage: "proxy-provider",
    otelCollectorImage: "collector-provider",
    issuer: "https://external-idp.example/tenant",
    publicPort: 8080,
    runtimeUser: "1:1",
    caddyFile: "/fixture/Caddyfile",
    caddyAppConfig: "/fixture/app.caddy",
    caddyIdentityConfig: "/fixture/identity.external.caddy",
    collectorConfig: "/fixture/collector.yaml",
    issuerFile: "/fixture/issuers.json",
    oidcDirectorySecrets: "/fixture/oidc-directory",
    databaseRolesFile: "/fixture/database-roles.json",
    databaseAuthorityDir: "/fixture/database-authority",
    keycloakPublicGateDir: "/fixture/keycloak-public-gate",
  };
  assert.deepEqual(canonicalComposeFindings(base, expected), []);

  const bundled = structuredClone(base);
  for (const name of [
    "postgres_owner_password",
    "keycloak_database_password",
    "keycloak_admin_username",
    "keycloak_admin_password",
    "keycloak_convergence_admin_password",
  ]) {
    bundled.secrets[name] = { file: `/fixture/secrets/${name}` };
  }
  bundled.networks["identity-backend"] = network("identity-backend", true);
  bundled.networks["identity-egress"] = network("identity-egress");
  bundled.networks["keycloak-data"] = network("keycloak-data", true);
  bundled.services["database-preflight"].environment.SYNVEDA_DATABASE_REQUIRED_PEER =
    "keycloak";
  bundled.services["database-preflight"].environment.SYNVEDA_DATABASE_PEER_WITNESS_FILE =
    "/run/synveda/database-authority/keycloak-cluster.json";
  bundled.services["database-preflight"].environment.SYNVEDA_DATABASE_EXPECTED_HOST =
    "database.compose.example";
  bundled.services["database-preflight"].environment.SYNVEDA_DATABASE_EXPECTED_PORT =
    "5432";
  bundled.services["database-preflight"].environment.SYNVEDA_DATABASE_EXPECTED_NAME =
    "synveda";
  bundled.services["database-preflight"].depends_on = {
    "keycloak-database-bootstrap": {
      condition: "service_completed_successfully",
    },
  };
  bundled.services["database-preflight"].volumes.push({
    type: "bind",
    source: "/fixture/database-authority",
    target: "/run/synveda/database-authority",
    read_only: true,
  });
  bundled.services.gateway.environment.SYNVEDA_DATABASE_REQUIRED_PEER = "keycloak";
  bundled.services.worker.environment.SYNVEDA_DATABASE_REQUIRED_PEER = "keycloak";
  bundled.services["issuer-diagnostic"].environment.SYNVEDA_OIDC_EXPECTED_ISSUER =
    "http://auth.synveda.test:8080/realms/synveda";
  bundled.services["issuer-diagnostic"].depends_on["keycloak-realm-convergence"] = {
    condition: "service_healthy",
  };
  bundled.services.proxy.depends_on = {
    "keycloak-realm-convergence": {
      condition: "service_healthy",
    },
  };
  bundled.services.proxy.environment.SYNVEDA_AUTH_HOST = "auth.synveda.test";
  bundled.services.proxy.networks["app-backend"] = {
    aliases: ["auth.synveda.test"],
  };
  bundled.services.proxy.networks["identity-backend"] = {
    ipv4_address: "172.30.240.2",
  };
  bundled.services.proxy.volumes.find(
    ({ target }) => target === "/etc/caddy/identity.caddy",
  ).source = "/fixture/identity.caddy";
  bundled.services.proxy.volumes.push({
      type: "bind",
      source: "/fixture/keycloak-public-gate",
      target: "/run/synveda/keycloak-public-gate",
      read_only: true,
  });
  const hardenedOneShot = (overrides) => ({
    user: "1:1",
    cap_drop: ["ALL"],
    security_opt: ["no-new-privileges:true"],
    read_only: true,
    init: true,
    pids_limit: 1,
    restart: "no",
    ...overrides,
  });
  bundled.services["keycloak-database-bootstrap"] = hardenedOneShot({
    entrypoint: ["/usr/local/bin/synveda-database-bootstrap"],
    command: ["keycloak"],
    image: "postgres-provider",
    environment: {
      SYNVEDA_DATABASE_AUTHORITY_DIR: "/run/synveda/database-authority",
      SYNVEDA_DATABASE_ROLES_FILE: "/run/secrets/database_roles.json",
      SYNVEDA_POSTGRES_BOOTSTRAP_URL:
        "postgresql://bootstrap@database.compose.example:5432/postgres",
      SYNVEDA_POSTGRES_BUNDLED_CLUSTER: "false",
    },
    secrets: [
      {
        source: "postgres_owner_password",
        target: "postgres_bootstrap_password",
      },
      {
        source: "keycloak_database_password",
        target: "keycloak_database_password",
      },
    ],
    volumes: [
      {
        type: "bind",
        source: "/fixture/database-roles.json",
        target: "/run/secrets/database_roles.json",
        read_only: true,
      },
      {
        type: "bind",
        source: "/fixture/database-authority",
        target: "/run/synveda/database-authority",
      },
    ],
    networks: {
      "identity-egress": { gw_priority: 1 },
      "keycloak-data": {},
    },
  });
  bundled.services.keycloak = hardenedOneShot({
    command: ["start", "--optimized"],
    image: "keycloak-provider",
    pids_limit: 512,
    stop_grace_period: "45s",
    mem_limit: "2g",
    cpus: 2,
    tmpfs: [
      "/tmp:rw,noexec,nosuid,nodev,mode=1777,size=128m",
      "/opt/keycloak/data/tmp:rw,noexec,nosuid,nodev,mode=0700,size=128m,uid=1,gid=1",
    ],
    environment: {
      KC_DB: "postgres",
      KC_DB_URL: "jdbc:postgresql://database.compose.example:5432/keycloak",
      KC_DB_USERNAME: "keycloak",
      KC_DB_PASSWORD_FILE: "/run/secrets/keycloak_database_password",
      KC_BOOTSTRAP_ADMIN_USERNAME_FILE: "/run/secrets/keycloak_admin_username",
      KC_BOOTSTRAP_ADMIN_PASSWORD_FILE: "/run/secrets/keycloak_admin_password",
      KC_HOSTNAME: "http://auth.synveda.test:8080",
      KC_HOSTNAME_STRICT: "true",
      KC_HEALTH_ENABLED: "true",
      KC_METRICS_ENABLED: "true",
      KC_HTTP_ENABLED: "true",
      KC_HTTP_PORT: "8080",
      KC_HTTP_MANAGEMENT_PORT: "9000",
      KC_PROXY_HEADERS: "xforwarded",
      KC_PROXY_TRUSTED_ADDRESSES: "172.30.240.2/32",
      KC_CACHE: "local",
      KC_LOG_LEVEL_ORG_KEYCLOAK_SERVICES: "warn",
      SYNVEDA_KEYCLOAK_PUBLIC_GATE_PATH:
        "/run/synveda/keycloak-public-gate",
    },
    healthcheck: {
      test: ["CMD", "/opt/keycloak/bin/synveda-keycloak-health", "local"],
      interval: "10s",
      timeout: "6s",
      retries: 30,
      start_period: "30s",
    },
    depends_on: {
      "keycloak-database-bootstrap": {
        condition: "service_completed_successfully",
      },
    },
    secrets: [
      {
        source: "keycloak_database_password",
        target: "keycloak_database_password",
      },
      { source: "keycloak_admin_username", target: "keycloak_admin_username" },
      { source: "keycloak_admin_password", target: "keycloak_admin_password" },
    ],
    volumes: [
      {
        type: "bind",
        source: "/fixture/keycloak-public-gate",
        target: "/run/synveda/keycloak-public-gate",
      },
    ],
    networks: {
      "identity-backend": {},
      "identity-egress": { gw_priority: 1 },
      "keycloak-data": {},
      "keycloak-management": {},
    },
  });
  bundled.services["keycloak-realm-convergence"] = hardenedOneShot({
    command: ["synveda-realm-supervise"],
    image: "keycloak-provider",
    pids_limit: 256,
    stop_grace_period: "3m30s",
    mem_limit: "512m",
    cpus: 1,
    tmpfs: [
      "/tmp:rw,noexec,nosuid,nodev,mode=0700,size=64m,uid=1,gid=1",
    ],
    environment: {
      KC_BOOTSTRAP_ADMIN_USERNAME_FILE: "/run/secrets/keycloak_admin_username",
      KC_BOOTSTRAP_ADMIN_PASSWORD_FILE: "/run/secrets/keycloak_admin_password",
      SYNVEDA_KEYCLOAK_CONVERGENCE_PASSWORD_FILE:
        "/run/secrets/keycloak_convergence_admin_password",
      SYNVEDA_PUBLIC_APP_URL: "http://app.synveda.test:8080",
      SYNVEDA_PUBLIC_AUTH_URL: "http://auth.synveda.test:8080",
      SYNVEDA_KEYCLOAK_SSL_REQUIRED: "NONE",
      SYNVEDA_KEYCLOAK_PUBLIC_GATE_PATH:
        "/run/synveda/keycloak-public-gate",
    },
    depends_on: {
      keycloak: { condition: "service_healthy" },
    },
    healthcheck: {
      test: ["CMD", "/opt/keycloak/bin/synveda-generation-gate", "ready"],
      interval: "5s",
      timeout: "3s",
      retries: 36,
      start_period: "15s",
    },
    secrets: [
      { source: "keycloak_admin_username", target: "keycloak_admin_username" },
      { source: "keycloak_admin_password", target: "keycloak_admin_password" },
      {
        source: "keycloak_convergence_admin_password",
        target: "keycloak_convergence_admin_password",
      },
    ],
    volumes: [
      {
        type: "bind",
        source: "/fixture/keycloak-public-gate",
        target: "/run/synveda/keycloak-public-gate",
      },
    ],
    networks: { "identity-backend": {}, "keycloak-management": {} },
  });
  const bundledExpected = {
    ...expected,
    oidc: "bundled",
    authHost: "auth.synveda.test",
    authUrl: "http://auth.synveda.test:8080",
    issuer: "http://auth.synveda.test:8080/realms/synveda",
    caddyIdentityConfig: "/fixture/identity.caddy",
  };
  assert.deepEqual(canonicalComposeFindings(bundled, bundledExpected), []);

  const bundledMutants = [];
  const substitutedConvergenceImage = structuredClone(bundled);
  substitutedConvergenceImage.services["keycloak-realm-convergence"].image =
    "attacker-provider";
  bundledMutants.push(["convergence image substituted", substitutedConvergenceImage]);

  const gateWritingEntrypoint = structuredClone(bundled);
  gateWritingEntrypoint.services["keycloak-realm-convergence"].entrypoint = [
    "/bin/bash",
    "-c",
    "touch /run/synveda/keycloak-public-gate/cpr45-keycloak-realm-v3.ready",
  ];
  bundledMutants.push(["convergence entrypoint writes gate", gateWritingEntrypoint]);

  const gateWritingHealthcheck = structuredClone(bundled);
  gateWritingHealthcheck.services.keycloak.healthcheck.test = [
    "CMD-SHELL",
    "touch /run/synveda/keycloak-public-gate/cpr45-keycloak-realm-v3.ready",
  ];
  bundledMutants.push(["Keycloak healthcheck writes gate", gateWritingHealthcheck]);

  const gateWritingHook = structuredClone(bundled);
  gateWritingHook.services.keycloak.post_start = [
    {
      command:
        "touch /run/synveda/keycloak-public-gate/cpr45-keycloak-realm-v3.ready",
    },
  ];
  bundledMutants.push(["Keycloak lifecycle hook writes gate", gateWritingHook]);

  const overriddenIssuerHost = structuredClone(bundled);
  overriddenIssuerHost.services.gateway.extra_hosts = [
    "auth.synveda.test=203.0.113.5",
  ];
  bundledMutants.push(["gateway issuer host overridden", overriddenIssuerHost]);

  const proxiedIssuerTraffic = structuredClone(bundled);
  proxiedIssuerTraffic.services.gateway.environment.HTTP_PROXY =
    "http://attacker.invalid:8080";
  bundledMutants.push(["gateway receives ambient HTTP proxy", proxiedIssuerTraffic]);

  const postgresTrustControl = structuredClone(bundled);
  postgresTrustControl.services["keycloak-database-bootstrap"].environment.POSTGRES_HOST_AUTH_METHOD =
    "trust";
  bundledMutants.push(["PostgreSQL trust control injected", postgresTrustControl]);

  const engineSocket = structuredClone(bundled);
  engineSocket.services.keycloak.use_api_socket = true;
  bundledMutants.push(["Keycloak receives engine API socket", engineSocket]);

  const unconfinedSeccomp = structuredClone(bundled);
  unconfinedSeccomp.services.keycloak.security_opt.push("seccomp=unconfined");
  bundledMutants.push(["Keycloak disables seccomp", unconfinedSeccomp]);

  const missingIssuerAlias = structuredClone(bundled);
  delete missingIssuerAlias.services.proxy.networks["app-backend"].aliases;
  bundledMutants.push(["browser issuer alias removed", missingIssuerAlias]);

  const driftedProxyAddress = structuredClone(bundled);
  driftedProxyAddress.services.proxy.networks["identity-backend"].ipv4_address =
    "172.30.240.3";
  bundledMutants.push(["trusted proxy address drifted", driftedProxyAddress]);

  const driftedIdentitySubnet = structuredClone(bundled);
  driftedIdentitySubnet.networks["identity-backend"].ipam.config[0].subnet =
    "172.30.240.16/28";
  bundledMutants.push(["identity subnet drifted", driftedIdentitySubnet]);

  const missingIdentityGateway = structuredClone(bundled);
  delete missingIdentityGateway.networks["identity-backend"].ipam.config[0].gateway;
  bundledMutants.push(["identity gateway removed", missingIdentityGateway]);

  const missingIdentityRange = structuredClone(bundled);
  delete missingIdentityRange.networks["identity-backend"].ipam.config[0].ip_range;
  bundledMutants.push(["identity dynamic range removed", missingIdentityRange]);

  const widenedIdentityRange = structuredClone(bundled);
  widenedIdentityRange.networks["identity-backend"].ipam.config[0].ip_range =
    "172.30.240.0/28";
  bundledMutants.push(["identity dynamic range includes proxy", widenedIdentityRange]);

  const auxiliaryProxyReservation = structuredClone(bundled);
  auxiliaryProxyReservation.networks["identity-backend"].ipam.config[0].aux_addresses = {
    "synveda-proxy": "172.30.240.2",
  };
  bundledMutants.push(["conflicting auxiliary proxy reservation", auxiliaryProxyReservation]);

  const missingEgressIpam = structuredClone(bundled);
  delete missingEgressIpam.networks["telemetry-egress"].ipam;
  bundledMutants.push(["egress IPAM removed", missingEgressIpam]);

  const extraIdentityRange = structuredClone(bundled);
  extraIdentityRange.networks["identity-backend"].ipam.config.push({
    subnet: "fd00::/64",
  });
  bundledMutants.push(["second identity IPAM range", extraIdentityRange]);

  const customNetworkName = structuredClone(bundled);
  customNetworkName.networks["identity-backend"].name = "shared-identity";
  bundledMutants.push(["identity network name escaped project", customNetworkName]);

  const secondStaticAddress = structuredClone(bundled);
  secondStaticAddress.services.keycloak.networks["identity-backend"].ipv4_address =
    "172.30.240.3";
  bundledMutants.push(["second static identity endpoint", secondStaticAddress]);

  const missingDefaultGateway = structuredClone(bundled);
  delete missingDefaultGateway.services.keycloak.networks["identity-egress"].gw_priority;
  bundledMutants.push(["identity egress gateway priority removed", missingDefaultGateway]);

  const ordinaryPrioritySubstitution = structuredClone(bundled);
  delete ordinaryPrioritySubstitution.services.proxy.networks["public-edge"].gw_priority;
  ordinaryPrioritySubstitution.services.proxy.networks["public-edge"].priority = 1;
  bundledMutants.push(["ordinary priority substituted for gateway priority", ordinaryPrioritySubstitution]);

  const widenedTrustedProxy = structuredClone(bundled);
  widenedTrustedProxy.services.keycloak.environment.KC_PROXY_TRUSTED_ADDRESSES =
    "172.30.240.0/28";
  bundledMutants.push(["trusted proxy widened to subnet", widenedTrustedProxy]);

  for (const [setting, value] of [
    ["SYNVEDA_APP_HOST", "wrong-app.synveda.test"],
    ["SYNVEDA_AUTH_HOST", "wrong-auth.synveda.test"],
  ]) {
    const driftedProxyHost = structuredClone(bundled);
    driftedProxyHost.services.proxy.environment[setting] = value;
    bundledMutants.push([`${setting} drifted`, driftedProxyHost]);
  }
  const gateInSecrets = structuredClone(bundled);
  for (const service of ["proxy", "keycloak", "keycloak-realm-convergence"]) {
    const mount = gateInSecrets.services[service].volumes.find(
      ({ target }) => target === "/run/synveda/keycloak-public-gate",
    );
    mount.source = "/fixture/secrets";
  }
  bundledMutants.push(["gate source aliases secrets", gateInSecrets]);

  const gateAliasesAuthority = structuredClone(bundled);
  for (const service of ["proxy", "keycloak", "keycloak-realm-convergence"]) {
    const mount = gateAliasesAuthority.services[service].volumes.find(
      ({ target }) => target === "/run/synveda/keycloak-public-gate",
    );
    mount.source = "/fixture/database-authority";
  }
  bundledMutants.push(["gate source aliases authority state", gateAliasesAuthority]);

  const extraProxyBind = structuredClone(bundled);
  extraProxyBind.services.proxy.volumes.push({
    type: "bind",
    source: "/fixture/operator-bypass.caddy",
    target: "/etc/caddy/operator-bypass.caddy",
    read_only: true,
  });
  bundledMutants.push(["extra proxy bind", extraProxyBind]);

  for (const service of ["issuer-diagnostic", "otel-collector"]) {
    const leakedSecret = structuredClone(bundled);
    leakedSecret.services[service].secrets = [
      { source: "keycloak_admin_password", target: "operator_password" },
    ];
    bundledMutants.push([`${service} receives an admin secret`, leakedSecret]);
  }

  const writableProxyGate = structuredClone(bundled);
  writableProxyGate.services.proxy.volumes.find(
    ({ target }) => target === "/run/synveda/keycloak-public-gate",
  ).read_only = false;
  bundledMutants.push(["proxy gate becomes writable", writableProxyGate]);

  const injectedDependencyRestart = structuredClone(bundled);
  injectedDependencyRestart.services[
    "keycloak-realm-convergence"
  ].depends_on.keycloak.restart = true;
  bundledMutants.push([
    "dependency restart metadata injected",
    injectedDependencyRestart,
  ]);

  const optionalDependency = structuredClone(bundled);
  optionalDependency.services["keycloak-realm-convergence"].depends_on.keycloak.required =
    false;
  bundledMutants.push(["realm convergence dependency made optional", optionalDependency]);

  const substitutedIdentitySource = structuredClone(bundled);
  substitutedIdentitySource.services.proxy.volumes.find(
    ({ target }) => target === "/etc/caddy/identity.caddy",
  ).source = "/fixture/app.caddy";
  bundledMutants.push(["identity config source substituted", substitutedIdentitySource]);

  const injectedConfig = structuredClone(bundled);
  injectedConfig.configs = {
    identity_bypass: { file: "/fixture/operator-bypass.caddy" },
  };
  injectedConfig.services.proxy.configs = [
    { source: "identity_bypass", target: "/etc/caddy/operator-bypass.caddy" },
  ];
  bundledMutants.push(["Compose config injected into proxy", injectedConfig]);

  const sharedNamespace = structuredClone(bundled);
  sharedNamespace.services.gateway.pid = "service:keycloak";
  bundledMutants.push(["gateway shares Keycloak PID namespace", sharedNamespace]);

  const legacyLink = structuredClone(bundled);
  legacyLink.services.gateway.links = ["keycloak"];
  bundledMutants.push(["gateway adds a legacy Keycloak link", legacyLink]);

  const missingDependency = structuredClone(bundled);
  delete missingDependency.services["issuer-diagnostic"].depends_on[
    "keycloak-realm-convergence"
  ];
  bundledMutants.push(["issuer convergence dependency removed", missingDependency]);

  const reverseCycle = structuredClone(bundled);
  reverseCycle.services.keycloak.depends_on["issuer-diagnostic"] = {
    condition: "service_completed_successfully",
  };
  bundledMutants.push(["reverse dependency cycle added", reverseCycle]);

  const keycloakResourceDrift = structuredClone(bundled);
  keycloakResourceDrift.services.keycloak.pids_limit = 511;
  bundledMutants.push(["Keycloak PID budget drifted", keycloakResourceDrift]);

  const keycloakMemoryDrift = structuredClone(bundled);
  keycloakMemoryDrift.services.keycloak.mem_limit = "3g";
  bundledMutants.push(["Keycloak memory budget drifted", keycloakMemoryDrift]);

  const keycloakTmpfsDrift = structuredClone(bundled);
  keycloakTmpfsDrift.services.keycloak.tmpfs[0] = "/tmp:rw,size=128m";
  bundledMutants.push(["Keycloak private tmpfs drifted", keycloakTmpfsDrift]);

  const supervisorResourceDrift = structuredClone(bundled);
  supervisorResourceDrift.services["keycloak-realm-convergence"].cpus = 2;
  bundledMutants.push(["realm supervisor CPU budget drifted", supervisorResourceDrift]);

  const supervisorKillSignal = structuredClone(bundled);
  supervisorKillSignal.services["keycloak-realm-convergence"].stop_signal = "SIGKILL";
  bundledMutants.push(["realm supervisor graceful stop bypassed", supervisorKillSignal]);

  const duplicatedKeycloak = structuredClone(bundled);
  duplicatedKeycloak.services.keycloak.scale = 2;
  bundledMutants.push(["Keycloak scale duplicated", duplicatedKeycloak]);

  const duplicatedSupervisor = structuredClone(bundled);
  duplicatedSupervisor.services["keycloak-realm-convergence"].deploy = {
    replicas: 2,
  };
  bundledMutants.push(["realm supervisor replicas duplicated", duplicatedSupervisor]);

  for (const [name, mutated] of bundledMutants) {
    assert.ok(
      canonicalComposeFindings(mutated, bundledExpected).length > 0,
      `bundled model accepted ${name}`,
    );
  }

  const externalIdentityAttachment = structuredClone(base);
  externalIdentityAttachment.services.proxy.networks["app-backend"] = {
    aliases: ["auth.synveda.test"],
  };
  assert.ok(
    canonicalComposeFindings(externalIdentityAttachment, expected).includes(
      "proxy network boundary drifted",
    ),
  );

  const wiringRegression = structuredClone(base);
  wiringRegression.services.gateway.user = "2345:2346";
  wiringRegression.services["issuer-diagnostic"].environment.SYNVEDA_PUBLIC_URL =
    "http://wrong-app.synveda.test:8080";
  wiringRegression.secrets.synveda_kms_key.file =
    "/fixture/secrets/synveda_kms_key_ref";
  delete wiringRegression.services.migrate.environment.DATABASE_URL_FILE;
  wiringRegression.services.migrate.secrets[0].target = "renamed_database_url";
  wiringRegression.services["otel-collector"].healthcheck.test = [
    "CMD",
    "/otelcol-contrib",
    "not-a-health-probe",
  ];
  const wiringFindings = canonicalComposeFindings(wiringRegression, expected);
  assert.ok(
    wiringFindings.includes("gateway runtime UID:GID differs from the validated secret owner"),
  );
  assert.ok(
    wiringFindings.includes(
      "issuer diagnostic public URL differs from the selected browser URL",
    ),
  );
  assert.ok(wiringFindings.includes("synveda_kms_key secret file source drifted"));
  assert.ok(
    wiringFindings.includes("migrate secret mounts are not role-scoped or have drifted targets"),
  );
  assert.ok(
    wiringFindings.includes(
      "migrate DATABASE_URL_FILE does not consume its mounted secret target",
    ),
  );
  assert.ok(
    wiringFindings.includes("Collector health does not validate its mounted configuration"),
  );

  base.services.gateway.privileged = true;
  base.services.worker.ports = [{ published: "8121", target: 8121 }];
  base.services.migrate.command = ["shell"];
  base.services.gateway.environment.DATABASE_URL = "not-allowed";
  const findings = canonicalComposeFindings(base, expected);
  assert.ok(findings.includes("gateway is privileged"));
  assert.ok(findings.includes("a non-proxy service publishes a host port"));
  assert.ok(findings.includes("migrate command drifted"));
  assert.ok(findings.includes("gateway receives direct secret DATABASE_URL"));
});
