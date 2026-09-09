import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import { randomBytes } from "node:crypto";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  realpathSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { generateTestTlsChain } from "./test-certificate.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const WRAPPER = join(ROOT, "deploy/compose/scripts/compose.sh");
const PROJECT_LOCK = join(ROOT, "deploy/compose/scripts/project-lock.sh");
const SECRET_GENERATOR = join(ROOT, "deploy/compose/scripts/generate-secrets.sh");
const ISSUER_GENERATOR = join(ROOT, "deploy/compose/scripts/generate-issuer.sh");
const DIGEST = `sha256:${"1".repeat(64)}`;
const CANDIDATE_DIGEST = `sha256:${"2".repeat(64)}`;
const HOST_TRUST_CONTROLS = [
  "NODE_OPTIONS",
  "NODE_EXTRA_CA_CERTS",
  "NODE_TLS_REJECT_UNAUTHORIZED",
  "NODE_USE_SYSTEM_CA",
  "NODE_USE_ENV_PROXY",
  "SSL_CERT_FILE",
  "SSL_CERT_DIR",
  "OPENSSL_CONF",
  "OPENSSL_CONF_INCLUDE",
  "OPENSSL_MODULES",
  "OPENSSL_ENGINES",
];
const HOST_BUILD_CONTROLS = [
  "COMPOSE_BAKE",
  "COMPOSE_DOCKER_CLI_BUILD",
  "DOCKER_CLI_HINTS",
  "DOCKER_CLI_HOOKS",
  "DOCKER_BUILDKIT",
  "DOCKER_DEFAULT_PLATFORM",
  "SOURCE_DATE_EPOCH",
  "BUILDKIT_COLORS",
  "BUILDKIT_HOST",
  "BUILDKIT_PROGRESS",
  "BUILDKIT_NO_CLIENT_TOKEN",
  "BUILDKIT_TTY_LOG_LINES",
  "EXPERIMENTAL_BUILDKIT_SOURCE_POLICY",
  "BUILDX_BAKE_FILE",
  "BUILDX_BAKE_FILE_SEPARATOR",
  "BUILDX_BAKE_PATH_SEPARATOR",
  "BUILDX_BAKE_FILE_RELATIVE_PATHS",
  "BUILDX_BAKE_DISABLE_VARS_ENV_LOOKUP",
  "BUILDX_BAKE_GIT_AUTH_HEADER",
  "BUILDX_BAKE_GIT_AUTH_TOKEN",
  "BUILDX_BAKE_GIT_SSH",
  "BUILDX_BAKE_ENTITLEMENTS_FS",
  "BAKE_ALLOW_REMOTE_FS_ACCESS",
  "BAKE_CMD_CONTEXT",
  "BAKE_LOCAL_PLATFORM",
  "BUILDX_BUILDER",
  "BUILDX_CONFIG",
  "BUILDX_CPU_PROFILE",
  "BUILDX_EXPERIMENTAL",
  "BUILDX_GIT_CHECK_DIRTY",
  "BUILDX_GIT_INFO",
  "BUILDX_GIT_LABELS",
  "BUILDX_MEM_PROFILE",
  "BUILDX_METADATA_PROVENANCE",
  "BUILDX_METADATA_WARNINGS",
  "BUILDX_NO_DEFAULT_ATTESTATIONS",
  "BUILDX_NO_DEFAULT_OCI_ARTIFACT",
  "BUILDX_NO_DEFAULT_LOAD",
  "BUILDX_DEFAULT_POLICY",
];
const CANONICAL_BUILD_ENVIRONMENT = Object.freeze({
  COMPOSE_BAKE: "false",
  DOCKER_CLI_HOOKS: "false",
  DOCKER_BUILDKIT: "1",
  BUILDX_BUILDER: "default",
  BUILDKIT_PROGRESS: "plain",
  BUILDKIT_NO_CLIENT_TOKEN: "false",
  BUILDX_BAKE_DISABLE_VARS_ENV_LOOKUP: "1",
  BUILDX_GIT_CHECK_DIRTY: "false",
  BUILDX_GIT_INFO: "false",
  BUILDX_GIT_LABELS: "false",
  BUILDX_NO_DEFAULT_ATTESTATIONS: "true",
});
const SCRUBBED_BUILD_CONTROLS = HOST_BUILD_CONTROLS.filter(
  (name) => !(name in CANONICAL_BUILD_ENVIRONMENT) && name !== "BUILDX_CONFIG",
);

function shellEnvironmentAssertions() {
  const canonical = Object.entries(CANONICAL_BUILD_ENVIRONMENT).map(
    ([name, value]) => `[ "\${${name}:-}" = ${value} ] || exit 92`,
  );
  const absent = SCRUBBED_BUILD_CONTROLS.map(
    (name) => `[ "\${${name}+x}" != x ] || exit 93`,
  );
  return [...canonical, ...absent].join("\n");
}
const REAL_ENV = spawnSync("/bin/sh", ["-c", "command -v env"], {
  encoding: "utf8",
}).stdout.trim();
const REAL_OPENSSL = spawnSync("/bin/sh", ["-c", "command -v openssl"], {
  encoding: "utf8",
}).stdout.trim();

function executable(path, content) {
  writeFileSync(path, content, { mode: 0o700 });
  chmodSync(path, 0o700);
}

function fixture(runtimeKind = "development") {
  assert.match(runtimeKind, /^(development|reference)$/);
  const suffix = `acceptance-life${randomBytes(6).toString("hex")}`;
  const project = `synveda-${runtimeKind}-${suffix}`;
  const scratch = realpathSync(mkdtempSync(join(tmpdir(), "synveda-compose-lifecycle-")));
  chmodSync(scratch, 0o700);
  const bin = join(scratch, "bin");
  mkdirSync(bin, { mode: 0o700 });
  const log = join(scratch, "calls.log");
  const fakeDatabaseBackup = join(bin, "fake-database-backup");
  executable(
    fakeDatabaseBackup,
    `#!${process.execPath}
const { createHash } = require("node:crypto");
const { chmodSync, writeFileSync } = require("node:fs");
const { join } = require("node:path");
const directory = process.env.SYNVEDA_BACKUP_STAGING_DIR;
const project = process.env.SYNVEDA_BACKUP_PROJECT;
const backupId = process.env.SYNVEDA_BACKUP_ID;
const tenantId = process.env.SYNVEDA_BOOTSTRAP_TENANT_ID;
const postgresImage = process.env.SYNVEDA_POSTGRES_IMAGE;
if (!directory || !project || !backupId || !tenantId || !postgresImage) process.exit(64);
const writePrivate = (name, value) => {
  const path = join(directory, name);
  writeFileSync(path, value, { mode: 0o600 });
  chmodSync(path, 0o600);
  return Buffer.from(value);
};
const synveda = writePrivate("synveda.dump", "fake synveda archive\\n");
const keycloak = writePrivate("keycloak.dump", "fake keycloak archive\\n");
const digest = (value) => createHash("sha256").update(value).digest("hex");
const manifest = {
  format: "synveda-logical-backup-v1",
  backup_id: backupId,
  source_project: project,
  tenant_id: tenantId,
  postgres_image: postgresImage,
  server_version_num: 170011,
  cluster_system_identifier: "7612345678901234567",
  start_lsn: "0/16B6C50",
  end_lsn: "0/16B6D20",
  started_at: "2026-09-08T12:00:00Z",
  completed_at: "2026-09-08T12:00:01Z",
  synveda: { bytes: synveda.length, sha256: digest(synveda) },
  keycloak: { bytes: keycloak.length, sha256: digest(keycloak) },
};
writePrivate("manifest.json", JSON.stringify(manifest) + "\\n");
`,
  );
  const fakeDocker = join(bin, "docker");
  executable(
    fakeDocker,
    `#!/bin/sh
set -eu
${shellEnvironmentAssertions()}
if [ "\${BUILDX_CONFIG+x}" = x ]; then
  [ -d "$BUILDX_CONFIG" ] && [ ! -L "$BUILDX_CONFIG" ] || exit 93
  case "$BUILDX_CONFIG" in
    "\${TMPDIR:-/tmp}"/synveda-compose-buildx.*) ;;
    *) exit 93 ;;
  esac
fi
if [ -n "\${SYNVEDA_FAKE_EXPECT_DOCKER_CONFIG:-}" ]; then
  [ "\${DOCKER_CONFIG:-}" = "$SYNVEDA_FAKE_EXPECT_DOCKER_CONFIG" ] || exit 93
fi
if [ -n "\${SYNVEDA_FAKE_EXPECT_DOCKER_AUTH_CONFIG:-}" ]; then
  [ "\${DOCKER_AUTH_CONFIG:-}" = "$SYNVEDA_FAKE_EXPECT_DOCKER_AUTH_CONFIG" ] || exit 93
fi
if [ -n "\${SYNVEDA_FAKE_EXPECT_COLLECTOR_CONFIG:-}" ] &&
  [ "\${1:-}" = compose ] && [ "\${2:-}" != version ]; then
  [ "\${SYNVEDA_RENDER_OTEL_COLLECTOR_CONFIG:-}" = "$SYNVEDA_FAKE_EXPECT_COLLECTOR_CONFIG" ] || exit 93
fi
if [ "\${NODE_OPTIONS+x}" = x ] || [ "\${NODE_EXTRA_CA_CERTS+x}" = x ] ||
  [ "\${NODE_TLS_REJECT_UNAUTHORIZED+x}" = x ] ||
  [ "\${NODE_USE_SYSTEM_CA+x}" = x ] || [ "\${NODE_USE_ENV_PROXY+x}" = x ] ||
  [ "\${SSL_CERT_FILE+x}" = x ] || [ "\${SSL_CERT_DIR+x}" = x ] ||
  [ "\${OPENSSL_CONF+x}" = x ] || [ "\${OPENSSL_CONF_INCLUDE+x}" = x ] ||
  [ "\${OPENSSL_MODULES+x}" = x ] || [ "\${OPENSSL_ENGINES+x}" = x ]; then
  exit 94
fi
if [ "\${SYNVEDA_FAKE_REQUIRE_PINNED_DOCKER:-0}" = 1 ]; then
  [ "\${DOCKER_HOST:-}" = "$SYNVEDA_FAKE_LOCAL_DOCKER_ENDPOINT" ] &&
    [ -z "\${DOCKER_CONTEXT:-}" ] || exit 97
fi
printf 'docker' >> "$SYNVEDA_FAKE_CALL_LOG"
for argument in "$@"; do printf ' <%s>' "$argument" >> "$SYNVEDA_FAKE_CALL_LOG"; done
printf '\n' >> "$SYNVEDA_FAKE_CALL_LOG"
if [ "$1" = compose ] && [ "$2" = version ]; then
  echo 2.33.1
  exit 0
fi
if [ "$1" = context ] && [ "$2" = show ]; then
  printf '%s\n' "\${SYNVEDA_FAKE_CONTEXT_NAME:-default}"
  exit 0
fi
if [ "$1" = buildx ] && [ "$2" = inspect ]; then
  [ "\${SYNVEDA_FAKE_BUILDER_COMMAND_FAIL:-0}" = 0 ] || exit 41
  case "\${SYNVEDA_FAKE_BUILDER_MODE:-valid}" in
    valid) builder_driver=docker; builder_endpoint=default; builder_status=running ;;
    remote) builder_driver=remote; builder_endpoint=tcp://127.0.0.1:47001; builder_status=running ;;
    stopped) builder_driver=docker; builder_endpoint=default; builder_status=stopped ;;
    extra-node) builder_driver=docker; builder_endpoint=default; builder_status=running ;;
    *) exit 98 ;;
  esac
  printf 'Name:          default\nDriver:        %s\n\nNodes:\n' "$builder_driver"
  printf 'Name:          default\nEndpoint:      %s\nStatus:        %s\n' \
    "$builder_endpoint" "$builder_status"
  if [ "\${SYNVEDA_FAKE_BUILDER_MODE:-valid}" = extra-node ]; then
    printf 'Name:          foreign\nEndpoint:      default\nStatus:        running\n'
  fi
  exit 0
fi
fake_restart_identity() {
  restart_identity_service=$1
  if [ "$restart_identity_service" = gateway ]; then
    restart_identity_mode=\${SYNVEDA_FAKE_GATEWAY_ID_MODE:-stable}
  elif [ -z "\${SYNVEDA_FAKE_RESTART_ID_SERVICE:-}" ] ||
    [ "\${SYNVEDA_FAKE_RESTART_ID_SERVICE:-}" = "$restart_identity_service" ]; then
    restart_identity_mode=\${SYNVEDA_FAKE_RESTART_ID_MODE:-stable}
  else
    restart_identity_mode=stable
  fi
  case "$restart_identity_mode" in
    missing) return 0 ;;
    post-missing)
      if grep -q " <restart> .* <$restart_identity_service>" "$SYNVEDA_FAKE_CALL_LOG"; then
        return 0
      fi
      ;;
    replaced)
      if grep -q " <restart> .* <$restart_identity_service>" "$SYNVEDA_FAKE_CALL_LOG"; then
        printf '%064d\n' 1
        return 0
      fi
      ;;
  esac
  case "$restart_identity_service" in
    gateway) printf '%064d\n' 0 ;;
    worker) printf '%064d\n' 4 ;;
    postgres) printf '%064d\n' 5 ;;
    keycloak) printf '%064d\n' 6 ;;
    otel-collector) printf '%064d\n' 7 ;;
    proxy) printf '%064d\n' 8 ;;
    apalis-worker) printf '%064d\n' 9 ;;
    *) exit 98 ;;
  esac
}
fake_product_image_id() {
  fake_image_reference=$1
  if [ "\${SYNVEDA_FAKE_SAME_PRODUCT_IMAGE_ID:-0}" = 1 ] ||
    [ "$fake_image_reference" = "\${SYNVEDA_FAKE_STARTING_PRODUCT_IMAGE:-}" ]; then
    printf 'sha256:%064d\n' 3
  elif [ "$fake_image_reference" = "\${SYNVEDA_FAKE_CANDIDATE_PRODUCT_IMAGE:-}" ]; then
    printf 'sha256:%064d\n' 4
  else
    exit 98
  fi
}
if [ "$1" = image ] && [ "$2" = inspect ]; then
  fake_product_image_id "$5"
  exit 0
fi
if [ "$1" = image ] && [ "$2" = pull ]; then
  [ "\${SYNVEDA_FAKE_UPGRADE_PULL_FAIL:-0}" = 0 ] || exit 52
  [ "$3" = "\${SYNVEDA_FAKE_CANDIDATE_PRODUCT_IMAGE:-}" ] || exit 98
  exit 0
fi
case " $* " in
  *" ps "*" --quiet "*" browser-acceptance "*)
    case "\${SYNVEDA_FAKE_BROWSER_ID_MODE:-exact}" in
      missing) exit 0 ;;
      malformed) printf 'not-an-id\n' ;;
      multiple) printf '%064d\n%064d\n' 2 3 ;;
      exact) printf '%064d\n' 2 ;;
    esac
    exit 0
    ;;
  *" ps "*" --quiet "*" gateway "*) fake_restart_identity gateway; exit 0 ;;
  *" ps "*" --quiet "*" postgres "*) fake_restart_identity postgres; exit 0 ;;
  *" ps "*" --quiet "*" keycloak "*) fake_restart_identity keycloak; exit 0 ;;
  *" ps "*" --quiet "*" otel-collector "*) fake_restart_identity otel-collector; exit 0 ;;
  *" ps "*" --quiet "*" apalis-worker "*) fake_restart_identity apalis-worker; exit 0 ;;
  *" ps "*" --quiet "*" worker "*) fake_restart_identity worker; exit 0 ;;
  *" ps "*" --quiet "*" proxy "*) fake_restart_identity proxy; exit 0 ;;
esac
if [ "$1" = container ] && [ "$2" = inspect ]; then
  case "$5" in
    0000000000000000000000000000000000000000000000000000000000000000)
      fake_upgrade_service=gateway
      ;;
    0000000000000000000000000000000000000000000000000000000000000004)
      fake_upgrade_service=worker
      ;;
    *) exit 98 ;;
  esac
  fake_upgrade_reference=$SYNVEDA_PRODUCT_IMAGE
  fake_upgrade_id=$(fake_product_image_id "$fake_upgrade_reference")
  case "\${SYNVEDA_FAKE_UPGRADE_RUNTIME_MODE:-valid}" in
    valid) ;;
    wrong-reference) fake_upgrade_reference=registry.invalid/product@${DIGEST} ;;
    wrong-id) fake_upgrade_id=sha256:$(printf '%064d' 9) ;;
    *) exit 98 ;;
  esac
  printf '%s|%s|%s|%s|%s|False\n' "$5" "$fake_upgrade_id" \
    "$fake_upgrade_reference" "$SYNVEDA_FAKE_PROJECT" "$fake_upgrade_service"
  exit 0
fi
if [ "$1" = container ] && [ "$2" = wait ]; then
  [ "\${SYNVEDA_FAKE_BROWSER_WAIT_ERROR:-0}" = 0 ] || exit 44
  printf '%s\n' "\${SYNVEDA_FAKE_BROWSER_EXIT:-0}"
  exit 0
fi
case " $* " in
  *" stop --timeout 210 gateway worker keycloak-realm-convergence keycloak "*)
    [ "\${SYNVEDA_FAKE_BACKUP_STOP_FAIL:-0}" = 0 ] || exit 46
    ;;
  *" run --rm --no-deps database-backup "*)
    [ "\${SYNVEDA_FAKE_BACKUP_FAIL:-0}" = 0 ] || exit 47
    ${JSON.stringify(fakeDatabaseBackup)}
    if [ "\${SYNVEDA_FAKE_LATE_DATABASE_FINAL:-0}" = 1 ]; then
      late_final=$SYNVEDA_DATABASE_BACKUP_ROOT/$SYNVEDA_BACKUP_ID
      mkdir -m 700 -- "$late_final"
      printf 'winning recovery set\n' > "$late_final/sentinel"
      chmod 600 "$late_final/sentinel"
    fi
    ;;
  *" run --rm --no-deps --no-TTY "*" product-demo.mjs "*)
    case " $* " in
      *" product-demo.mjs seed "*)
        [ "\${SYNVEDA_FAKE_PRODUCT_SEED_FAIL:-0}" = 0 ] || exit 48
        ;;
      *" product-demo.mjs verify "*)
        [ "\${SYNVEDA_FAKE_PRODUCT_VERIFY_FAIL:-0}" = 0 ] || exit 49
        upgrade_transition_count=$(grep -c \
          ' <up> <--no-build> <--pull> <never> <--detach> <--wait> .* <gateway> <worker>' \
          "$SYNVEDA_FAKE_CALL_LOG" || true)
        if [ -n "\${SYNVEDA_FAKE_UPGRADE_VERIFY_FAIL_AT:-}" ] &&
          [ "$SYNVEDA_FAKE_UPGRADE_VERIFY_FAIL_AT" = "$upgrade_transition_count" ]; then
          exit 49
        fi
        ;;
      *) exit 64 ;;
    esac
    ;;
  *" run --rm --no-deps --no-TTY --pull never migrate migration-check "*)
    printf 'compatibility-image <%s>\n' "$SYNVEDA_PRODUCT_IMAGE" >> "$SYNVEDA_FAKE_CALL_LOG"
    [ "\${SYNVEDA_FAKE_UPGRADE_COMPATIBILITY_FAIL:-0}" = 0 ] || exit 50
    ;;
esac
volume_key=postgres-data
case " $* " in
  *"browser-acceptance-state"*) volume_key=browser-acceptance-state ;;
  *"prometheus-data"*) volume_key=prometheus-data ;;
  *"apalis-data"*) volume_key=apalis-data ;;
esac
if [ "$1" = volume ] && [ "$2" = ls ]; then
  [ "\${SYNVEDA_FAKE_VOLUME_INVENTORY_ERROR:-0}" = 0 ] || exit 1
  if grep -Fq "docker <volume> <rm> <\${SYNVEDA_FAKE_PROJECT}_\${volume_key}>" \
      "$SYNVEDA_FAKE_CALL_LOG"; then
    [ "\${SYNVEDA_FAKE_POST_REMOVE_INVENTORY_ERROR:-0}" = 0 ] || exit 1
    exit 0
  fi
  case " $* " in
    *"name=^"*) query=named ;;
    *) query=labelled ;;
  esac
  case "\${SYNVEDA_FAKE_VOLUME_MODE:-absent}:$query" in
    exact:named|exact:labelled|named-only:named)
      printf '%s_%s\n' "$SYNVEDA_FAKE_PROJECT" "$volume_key"
      ;;
    labelled-wrong-name:labelled)
      printf '%s_foreign-%s\n' "$SYNVEDA_FAKE_PROJECT" "$volume_key"
      ;;
  esac
  exit 0
fi
if [ "$1" = volume ] && [ "$2" = inspect ]; then
  volume_contract=\${SYNVEDA_FAKE_VOLUME_CONTRACT:-valid}
  if [ -n "\${SYNVEDA_FAKE_VOLUME_CONTRACT_KEY:-}" ] &&
    [ "$SYNVEDA_FAKE_VOLUME_CONTRACT_KEY" != "$volume_key" ]; then
    volume_contract=valid
  fi
  case "$volume_contract" in
    inspect-error) exit 1 ;;
    valid)
      printf '%s_%s|local|local|null|%s|%s|cpr-45|%s\n' \
        "$SYNVEDA_FAKE_PROJECT" "$volume_key" "$SYNVEDA_FAKE_PROJECT" \
        "$volume_key" "$volume_key"
      ;;
    wrong-driver)
      printf '%s_%s|foreign|local|null|%s|%s|cpr-45|%s\n' \
        "$SYNVEDA_FAKE_PROJECT" "$volume_key" "$SYNVEDA_FAKE_PROJECT" \
        "$volume_key" "$volume_key"
      ;;
    wrong-scope)
      printf '%s_%s|local|global|null|%s|%s|cpr-45|%s\n' \
        "$SYNVEDA_FAKE_PROJECT" "$volume_key" "$SYNVEDA_FAKE_PROJECT" \
        "$volume_key" "$volume_key"
      ;;
    wrong-options)
      printf '%s_%s|local|local|{"type":"none"}|%s|%s|cpr-45|%s\n' \
        "$SYNVEDA_FAKE_PROJECT" "$volume_key" "$SYNVEDA_FAKE_PROJECT" \
        "$volume_key" "$volume_key"
      ;;
    drift-after-down)
      if grep -q ' <down>' "$SYNVEDA_FAKE_CALL_LOG"; then
        printf '%s_%s|foreign|local|null|%s|%s|cpr-45|%s\n' \
          "$SYNVEDA_FAKE_PROJECT" "$volume_key" "$SYNVEDA_FAKE_PROJECT" \
          "$volume_key" "$volume_key"
      else
        printf '%s_%s|local|local|null|%s|%s|cpr-45|%s\n' \
          "$SYNVEDA_FAKE_PROJECT" "$volume_key" "$SYNVEDA_FAKE_PROJECT" \
          "$volume_key" "$volume_key"
      fi
      ;;
  esac
  exit 0
fi
case " $* " in
  *" build --builder default "*)
    printf 'buildx <%s>\n' "$BUILDX_CONFIG" >> "$SYNVEDA_FAKE_CALL_LOG"
    : > "$BUILDX_CONFIG/fake-state"
    if [ "\${SYNVEDA_FAKE_BLOCK_BUILD:-0}" = 1 ]; then
      : > "$SYNVEDA_FAKE_BUILD_ENTERED"
      while [ ! -f "$SYNVEDA_FAKE_BUILD_RELEASE" ]; do /bin/sleep 0.02; done
    fi
    [ "\${SYNVEDA_FAKE_BUILD_FAIL:-0}" = 0 ] || exit 42
    ;;
  *" restart "*)
    [ "\${SYNVEDA_FAKE_RESTART_FAIL:-0}" = 0 ] || exit 42
    ;;
  *" up "*)
    case " $* " in
      *" --no-deps --force-recreate browser-acceptance "*)
        [ "\${SYNVEDA_FAKE_BROWSER_UP_FAIL:-0}" = 0 ] || exit 43
        ;;
    esac
    case " $* " in
      *" --pull never "*" --force-recreate gateway worker "*)
        printf 'product-image <%s>\n' "$SYNVEDA_PRODUCT_IMAGE" >> "$SYNVEDA_FAKE_CALL_LOG"
        upgrade_transition_count=$(grep -c \
          ' <up> <--no-build> <--pull> <never> <--detach> <--wait> .* <gateway> <worker>' \
          "$SYNVEDA_FAKE_CALL_LOG" || true)
        [ "\${SYNVEDA_FAKE_UPGRADE_UP_FAIL_AT:-0}" != "$upgrade_transition_count" ] || \
          exit 51
        case "\${SYNVEDA_FAKE_UPGRADE_UP_FAIL_FROM:-0}" in
          0) ;;
          *) [ "$upgrade_transition_count" -lt "$SYNVEDA_FAKE_UPGRADE_UP_FAIL_FROM" ] || exit 51 ;;
        esac
        ;;
    esac
    if [ "\${SYNVEDA_FAKE_BLOCK_UP:-0}" = 1 ]; then
      : > "$SYNVEDA_FAKE_UP_ENTERED"
      while [ ! -f "$SYNVEDA_FAKE_UP_RELEASE" ]; do /bin/sleep 0.02; done
    fi
    ;;
esac
exit 0
`,
  );
  const fakeNode = join(bin, "node");
  executable(
    fakeNode,
    `#!/bin/sh
set -eu
[ "\${1:-}" = --use-bundled-ca ] || exit 96
shift
if [ "\${NODE_OPTIONS+x}" = x ] || [ "\${NODE_EXTRA_CA_CERTS+x}" = x ] ||
  [ "\${NODE_TLS_REJECT_UNAUTHORIZED+x}" = x ] ||
  [ "\${NODE_USE_SYSTEM_CA+x}" = x ] || [ "\${NODE_USE_ENV_PROXY+x}" = x ] ||
  [ "\${SSL_CERT_FILE+x}" = x ] ||
  [ "\${SSL_CERT_DIR+x}" = x ] || [ "\${OPENSSL_CONF+x}" = x ] ||
  [ "\${OPENSSL_CONF_INCLUDE+x}" = x ] || [ "\${OPENSSL_MODULES+x}" = x ] ||
  [ "\${OPENSSL_ENGINES+x}" = x ]; then
  exit 95
fi
case "$1" in
  */monotonic-seconds.mjs)
    if [ -n "\${SYNVEDA_FAKE_CLOCK_FILE:-}" ]; then
      IFS= read -r now < "$SYNVEDA_FAKE_CLOCK_FILE"
      printf '%s\n' "$now"
      printf '%s\n' "$((now + SYNVEDA_FAKE_CLOCK_STEP))" > "$SYNVEDA_FAKE_CLOCK_FILE"
      exit 0
    fi
    exec ${JSON.stringify(process.execPath)} "$@"
    ;;
  */run-with-deadline.mjs)
    printf 'deadline <%s>\n' "$3" >> "$SYNVEDA_FAKE_CALL_LOG"
    if [ -n "\${SYNVEDA_FAKE_RUNNER_PID_FILE:-}" ]; then
      printf '%s\n' "$$" > "$SYNVEDA_FAKE_RUNNER_PID_FILE"
    fi
    if [ "\${SYNVEDA_FAKE_RUNNER_125:-0}" = 1 ]; then exit 125; fi
    if [ "\${SYNVEDA_FAKE_BUILD_TIMEOUT:-0}" = 1 ]; then
      case " $* " in
        *" build --builder default "*)
          runner_path=$1
          shift 3
          exec ${JSON.stringify(process.execPath)} "$runner_path" --seconds 1 "$@"
          ;;
      esac
    fi
    exec ${JSON.stringify(process.execPath)} "$@"
    ;;
  */check-compose-assets.mjs)
    printf 'node' >> "$SYNVEDA_FAKE_CALL_LOG"
    for argument in "$@"; do printf ' <%s>' "$argument" >> "$SYNVEDA_FAKE_CALL_LOG"; done
    printf '\n' >> "$SYNVEDA_FAKE_CALL_LOG"
    case " $* " in
      *" --state absent "*)
        [ "\${SYNVEDA_FAKE_ABSENT_ASSET_STATUS:-0}" = 0 ] ||
          exit "$SYNVEDA_FAKE_ABSENT_ASSET_STATUS"
        ;;
      *" --state converged "*)
        [ "\${SYNVEDA_FAKE_CONVERGED_ASSET_STATUS:-0}" = 0 ] ||
          exit "$SYNVEDA_FAKE_CONVERGED_ASSET_STATUS"
        ;;
    esac
    exit 0
    ;;
  */check-browser-seccomp.mjs)
    printf 'node' >> "$SYNVEDA_FAKE_CALL_LOG"
    for argument in "$@"; do printf ' <%s>' "$argument" >> "$SYNVEDA_FAKE_CALL_LOG"; done
    printf '\n' >> "$SYNVEDA_FAKE_CALL_LOG"
    exec ${JSON.stringify(process.execPath)} "$@"
    ;;
  */check-host-resolution.mjs|*/check-network-preflight.mjs|*/check-runtime-smoke.mjs)
    printf 'node' >> "$SYNVEDA_FAKE_CALL_LOG"
    for argument in "$@"; do printf ' <%s>' "$argument" >> "$SYNVEDA_FAKE_CALL_LOG"; done
    printf '\n' >> "$SYNVEDA_FAKE_CALL_LOG"
    if [ "\${SYNVEDA_FAKE_REAL_HOST_CHECK:-0}" = 1 ]; then
      case "$1" in
        */check-host-resolution.mjs) exec ${JSON.stringify(process.execPath)} "$@" ;;
      esac
    fi
    case " $* " in
      *" --print-docker-endpoint true "*)
        printf '%s\n' "\${SYNVEDA_FAKE_LOCAL_DOCKER_ENDPOINT:-unix:///var/run/docker.sock}"
        ;;
    esac
    exit 0
    ;;
  */manage-hosts-file.mjs)
    case " $* " in
      *" plan "*) exec ${JSON.stringify(process.execPath)} "$@" ;;
    esac
    printf 'node' >> "$SYNVEDA_FAKE_CALL_LOG"
    for argument in "$@"; do printf ' <%s>' "$argument" >> "$SYNVEDA_FAKE_CALL_LOG"; done
    printf '\n' >> "$SYNVEDA_FAKE_CALL_LOG"
    fake_hosts_status=\${SYNVEDA_FAKE_HOSTS_STATUS:-installed}
    case " $* " in
      *" --expect installed "*)
        [ "$fake_hosts_status" = installed ] || {
          printf 'hosts-file: development hosts mapping is absent, expected installed\n' >&2
          exit 78
        }
        ;;
      *" --expect absent "*)
        [ "$fake_hosts_status" = absent ] || {
          printf 'hosts-file: development hosts mapping is installed, expected absent\n' >&2
          exit 78
        }
        ;;
    esac
    printf 'development hosts mapping is %s\n' "$fake_hosts_status"
    exit 0
    ;;
  */check-tls-inputs.mjs)
    printf 'node' >> "$SYNVEDA_FAKE_CALL_LOG"
    for argument in "$@"; do printf ' <%s>' "$argument" >> "$SYNVEDA_FAKE_CALL_LOG"; done
    printf '\n' >> "$SYNVEDA_FAKE_CALL_LOG"
    if [ "\${SYNVEDA_FAKE_FAIL_TLS_CHECK:-0}" = 1 ]; then exit 91; fi
    exec ${JSON.stringify(process.execPath)} "$@"
    ;;
esac
exec ${JSON.stringify(process.execPath)} "$@"
`,
  );
  executable(
    join(bin, "env"),
    `#!/bin/sh
set -eu
if [ -n "\${SYNVEDA_FAKE_AUTHORITY_CHILD_PID_FILE:-}" ]; then
  for argument in "$@"; do
    case "$argument" in
      */generate-secrets.sh)
        printf '%s\n' "$$" > "$SYNVEDA_FAKE_AUTHORITY_CHILD_PID_FILE"
        if [ "\${SYNVEDA_FAKE_BLOCK_AUTHORITY_ENV:-0}" = 1 ]; then
          if [ "\${SYNVEDA_FAKE_AUTHORITY_IGNORE_SIGNALS:-0}" = 1 ]; then
            trap '' HUP INT TERM
          fi
          : > "$SYNVEDA_FAKE_AUTHORITY_ENV_ENTERED"
          while [ ! -f "$SYNVEDA_FAKE_AUTHORITY_ENV_RELEASE" ]; do :; done
        fi
        break
        ;;
    esac
  done
fi
exec ${JSON.stringify(REAL_ENV)} "$@"
`,
  );
  executable(
    join(bin, "openssl"),
    `#!/bin/sh
set -eu
if [ "\${SYNVEDA_FAKE_BLOCK_OPENSSL:-0}" = 1 ]; then
  : > "$SYNVEDA_FAKE_OPENSSL_ENTERED"
  while [ ! -f "$SYNVEDA_FAKE_OPENSSL_RELEASE" ]; do /bin/sleep 0.02; done
fi
exec ${JSON.stringify(REAL_OPENSSL)} "$@"
`,
  );
  const fakeRm = join(bin, "rm");
  executable(
    fakeRm,
    `#!/bin/sh
set -eu
if [ "\${SYNVEDA_FAKE_BLOCK_ASSET_CLEANUP:-0}" = 1 ]; then
  for argument in "$@"; do
    case "$argument" in
      */synveda-compose-assets.*)
        : > "$SYNVEDA_FAKE_CLEANUP_ENTERED"
        while [ ! -f "$SYNVEDA_FAKE_CLEANUP_RELEASE" ]; do /bin/sleep 0.02; done
        ;;
    esac
  done
fi
exec /bin/rm "$@"
`,
  );
  const runtime = join(scratch, project);
  mkdirSync(runtime, { mode: 0o700 });
  const secrets = join(runtime, "secrets");
  const authority = join(runtime, "database-authority");
  const gate = join(runtime, "keycloak-public-gate");
  const issuer = join(runtime, "issuers.json");
  return {
    scratch,
    bin,
    log,
    fakeDocker,
    runtime,
    secrets,
    authority,
    gate,
    issuer,
    project,
    suffix,
    runtimeKind,
  };
}

function environment(state, extra = {}) {
  const clean = { ...process.env };
  for (const name of Object.keys(clean)) {
    if (name.startsWith("SYNVEDA_") || name.startsWith("COMPOSE_")) delete clean[name];
  }
  for (const name of HOST_TRUST_CONTROLS) delete clean[name];
  for (const name of HOST_BUILD_CONTROLS) delete clean[name];
  for (const name of [
    "DATABASE_URL",
    "POSTGRES_PASSWORD",
    "KC_DB_PASSWORD",
    "KC_BOOTSTRAP_ADMIN_USERNAME",
    "KC_BOOTSTRAP_ADMIN_PASSWORD",
    "DOCKER_CONFIG",
    "DOCKER_AUTH_CONFIG",
  ]) {
    delete clean[name];
  }
  return {
    ...clean,
    PATH: `${state.bin}:${clean.PATH}`,
    TMPDIR: state.scratch,
    SYNVEDA_DOCKER_BIN: state.fakeDocker,
    SYNVEDA_FAKE_CALL_LOG: state.log,
    SYNVEDA_FAKE_PROJECT: state.project,
    SYNVEDA_COMPOSE_RUNTIME: state.runtimeKind,
    SYNVEDA_POSTGRES_MODE: "bundled",
    SYNVEDA_OIDC_MODE: "bundled",
    SYNVEDA_COMPOSE_PROFILES: "demo",
    SYNVEDA_COMPOSE_PROJECT_SUFFIX: state.suffix,
    SYNVEDA_COMPOSE_IPV4_POOL: "10.231.44.0/24",
    SYNVEDA_SECRETS_DIR: state.secrets,
    SYNVEDA_DATABASE_AUTHORITY_DIR: state.authority,
    SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR: state.gate,
    SYNVEDA_OIDC_ISSUERS_FILE: state.issuer,
    ...(state.runtimeKind === "reference"
      ? {
          SYNVEDA_PUBLIC_SCHEME: "https",
          SYNVEDA_APP_HOST: "app.lifecycle.example",
          SYNVEDA_AUTH_HOST: "auth.lifecycle.example",
          SYNVEDA_PRODUCT_IMAGE: `registry.lifecycle.example/synveda/product@${DIGEST}`,
          SYNVEDA_POSTGRES_IMAGE: `registry.lifecycle.example/synveda/postgres@${DIGEST}`,
          SYNVEDA_KEYCLOAK_IMAGE: `registry.lifecycle.example/synveda/keycloak@${DIGEST}`,
          SYNVEDA_CADDY_IMAGE: `registry.lifecycle.example/synveda/proxy@${DIGEST}`,
          SYNVEDA_BROWSER_IMAGE:
            `registry.lifecycle.example/synveda/browser-acceptance@${DIGEST}`,
          SYNVEDA_PROMETHEUS_IMAGE:
            `registry.lifecycle.example/synveda/prometheus@${DIGEST}`,
        }
      : {}),
    ...extra,
  };
}

function run(state, action, extra = {}, args = []) {
  return spawnSync(WRAPPER, [action, ...args], {
    cwd: ROOT,
    env: environment(state, extra),
    encoding: "utf8",
  });
}

function runWithoutEnvironment(state, action, names, extra = {}) {
  const env = environment(state, extra);
  for (const name of names) delete env[name];
  return spawnSync(WRAPPER, [action], { cwd: ROOT, env, encoding: "utf8" });
}

function prepareReferenceFixture(state) {
  assert.equal(state.runtimeKind, "reference");
  const preparedEnvironment = environment(state);
  for (const script of [SECRET_GENERATOR, ISSUER_GENERATOR]) {
    const result = spawnSync(script, ["--if-missing"], {
      cwd: ROOT,
      env: preparedEnvironment,
      encoding: "utf8",
    });
    assert.equal(result.status, 0, result.stderr);
  }
  const tls = generateTestTlsChain({
    commonName: "app.lifecycle.example",
    sanHosts: ["app.lifecycle.example", "auth.lifecycle.example"],
  });
  writeFileSync(join(state.secrets, "tls_cert"), tls.certificateChain, { mode: 0o600 });
  writeFileSync(join(state.secrets, "tls_key"), tls.privateKey, { mode: 0o600 });
  chmodSync(join(state.secrets, "tls_cert"), 0o600);
  chmodSync(join(state.secrets, "tls_key"), 0o600);
}

function upgradeEnvironment() {
  const starting = `registry.lifecycle.example/synveda/product-starting@${DIGEST}`;
  const candidate = `registry.lifecycle.example/synveda/product-candidate@${CANDIDATE_DIGEST}`;
  return {
    SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance",
    SYNVEDA_PRODUCT_STARTING_IMAGE: starting,
    SYNVEDA_PRODUCT_IMAGE: candidate,
    SYNVEDA_FAKE_STARTING_PRODUCT_IMAGE: starting,
    SYNVEDA_FAKE_CANDIDATE_PRODUCT_IMAGE: candidate,
  };
}

function prepareExternalPostgresFixture(state) {
  mkdirSync(state.secrets, { recursive: true, mode: 0o700 });
  chmodSync(state.secrets, 0o700);
  const directorySecrets = join(state.secrets, "oidc-directory");
  mkdirSync(directorySecrets, { mode: 0o700 });
  chmodSync(directorySecrets, 0o700);
  const roleUrls = {
    synveda_migrator_database_url:
      "postgresql://synveda_migrator:migrator-secret@database.lifecycle.example:5432/synveda?sslmode=verify-full&sslrootcert=/run/secrets/postgres_root_ca",
    synveda_gateway_database_url:
      "postgresql://synveda_gateway:gateway-secret@database.lifecycle.example:5432/synveda?sslmode=verify-full&sslrootcert=/run/secrets/postgres_root_ca",
    synveda_worker_database_url:
      "postgresql://synveda_worker:worker-secret@database.lifecycle.example:5432/synveda?sslmode=verify-full&sslrootcert=/run/secrets/postgres_root_ca",
  };
  const inputs = {
    ...roleUrls,
    synveda_kms_key: "11".repeat(32),
    synveda_kms_key_ref: "compose/external",
    postgres_root_ca: "-----BEGIN CERTIFICATE-----\nfixture\n-----END CERTIFICATE-----",
  };
  if (state.runtimeKind === "reference") {
    const tls = generateTestTlsChain({
      commonName: "app.lifecycle.example",
      sanHosts: ["app.lifecycle.example"],
    });
    inputs.tls_cert = tls.certificateChain.trimEnd();
    inputs.tls_key = tls.privateKey.trimEnd();
  }
  for (const [name, value] of Object.entries(inputs)) {
    writeFileSync(join(state.secrets, name), `${value}\n`, { mode: 0o600 });
    chmodSync(join(state.secrets, name), 0o600);
  }
  writeFileSync(
    state.issuer,
    `${JSON.stringify([
      {
        issuer: "https://identity.lifecycle.example/realms/synveda",
        client_id: "synveda",
        audience: "synveda-api",
        tenant: { static: { tenant_id: "019b53c0-7c00-7000-8000-000000000045" } },
        login_scopes: ["openid", "profile", "email"],
      },
    ])}\n`,
    { mode: 0o600 },
  );
  chmodSync(state.issuer, 0o600);
  return {
    SYNVEDA_POSTGRES_MODE: "external",
    SYNVEDA_OIDC_MODE: "external",
    SYNVEDA_COMPOSE_PROFILES: "",
    SYNVEDA_OIDC_ISSUER: "https://identity.lifecycle.example/realms/synveda",
    SYNVEDA_DATABASE_ROLES_FILE: join(
      ROOT,
      "deploy/compose/configs/database/roles.external-oidc.json",
    ),
    SYNVEDA_DATABASE_EXPECTED_HOST: "database.lifecycle.example",
    SYNVEDA_DATABASE_EXPECTED_PORT: "5432",
    SYNVEDA_DATABASE_EXPECTED_NAME: "synveda",
  };
}

function recoveryRoots(state) {
  const database = join(state.scratch, "database-backups");
  const secrets = join(state.scratch, "recovery-secrets");
  for (const directory of [database, secrets]) {
    mkdirSync(directory, { mode: 0o700 });
    chmodSync(directory, 0o700);
  }
  return {
    database,
    secrets,
    environment: {
      SYNVEDA_DATABASE_BACKUP_ROOT: database,
      SYNVEDA_RECOVERY_SECRETS_ROOT: secrets,
    },
  };
}

function privateDirectoryForLifecycleTest(path) {
  mkdirSync(path, { recursive: true, mode: 0o700 });
  chmodSync(path, 0o700);
}

function ambientHostTrust(sentinel = "cpr45-private-host-trust-sentinel") {
  return Object.fromEntries(HOST_TRUST_CONTROLS.map((name) => [name, sentinel]));
}

function ambientHostBuild(sentinel = "cpr45-private-host-build-sentinel") {
  return Object.fromEntries(HOST_BUILD_CONTROLS.map((name) => [name, sentinel]));
}

test("reference evidence refuses ambient host trust before Node or project locking", () => {
  const actions = ["config", "up", "smoke", "restart-gateway"];
  for (const [index, name] of HOST_TRUST_CONTROLS.entries()) {
    const state = fixture("reference");
    const lockFile = join(
      "/tmp",
      `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
      `${state.project}.lock`,
    );
    const sentinel = `cpr45-private-host-trust-sentinel-${index}`;
    try {
      const refused = run(state, actions[index % actions.length], {
        [name]: sentinel,
      });
      assert.equal(refused.status, 78, `${name}: ${refused.stderr}`);
      assert.match(refused.stderr, /ambient host trust configuration is not accepted/);
      assert.ok(!`${refused.stdout}${refused.stderr}`.includes(sentinel));
      assert.equal(existsSync(state.log), false, `${name} reached a child process`);
      assert.equal(existsSync(lockFile), false, `${name} acquired the project lock`);
    } finally {
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }
});

test("development scrubs ambient host trust before every lifecycle child", () => {
  const state = fixture();
  try {
    const result = run(state, "up", ambientHostTrust());
    assert.equal(result.status, 0, result.stderr);
    assert.match(readFileSync(state.log, "utf8"), / <up>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("development build refuses ambient host build controls before children or locking", () => {
  for (const [index, name] of HOST_BUILD_CONTROLS.entries()) {
    for (const value of ["", `cpr45-private-host-build-sentinel-${index}`]) {
      const state = fixture();
      const lockFile = join(
        "/tmp",
        `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
        `${state.project}.lock`,
      );
      try {
        const refused = run(state, "up", { [name]: value });
        assert.equal(refused.status, 78, `${name}: ${refused.stderr}`);
        assert.match(
          refused.stderr,
          /ambient host build configuration is not accepted for development builds/,
        );
        assert.ok(!`${refused.stdout}${refused.stderr}`.includes(value) || value === "");
        assert.equal(existsSync(state.log), false, `${name} reached a child process`);
        assert.equal(existsSync(lockFile), false, `${name} acquired the project lock`);
      } finally {
        rmSync(state.scratch, { recursive: true, force: true });
      }
    }
  }
});

test("development build keeps temporary and Docker config roots outside the source context", () => {
  const cases = [
    {
      extra: { TMPDIR: ROOT },
      diagnostic: /temporary root is not accepted inside the build context/,
    },
    {
      extra: { DOCKER_CONFIG: ROOT },
      diagnostic: /Docker registry configuration is not accepted inside the build context/,
    },
    {
      extra: { DOCKER_CONFIG: "" },
      diagnostic: /Docker registry configuration path was refused/,
    },
    {
      extra: { HOME: ROOT },
      diagnostic: /Docker registry configuration is not accepted inside the build context/,
    },
    {
      extra(state) {
        const fakeHome = join(state.scratch, "home");
        mkdirSync(fakeHome, { mode: 0o700 });
        symlinkSync(ROOT, join(fakeHome, ".docker"), "dir");
        return { HOME: fakeHome };
      },
      diagnostic: /Docker registry configuration is not accepted inside the build context/,
    },
  ];

  for (const testCase of cases) {
    const state = fixture();
    const lockFile = join(
      "/tmp",
      `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
      `${state.project}.lock`,
    );
    try {
      const extra =
        typeof testCase.extra === "function" ? testCase.extra(state) : testCase.extra;
      const refused = run(state, "up", extra);
      assert.equal(refused.status, 78, refused.stderr);
      assert.match(refused.stderr, testCase.diagnostic);
      assert.equal(existsSync(state.log), false, "a refused path reached Docker");
      assert.equal(existsSync(lockFile), false, "a refused path acquired the project lock");
    } finally {
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }

  const unsetHome = fixture();
  try {
    const refused = runWithoutEnvironment(unsetHome, "up", ["HOME"]);
    assert.equal(refused.status, 78, refused.stderr);
    assert.match(refused.stderr, /Docker registry configuration path was refused/);
    assert.equal(existsSync(unsetHome.log), false);
  } finally {
    rmSync(unsetHome.scratch, { recursive: true, force: true });
  }

  const linkedConfig = fixture();
  try {
    const dockerConfig = join(linkedConfig.scratch, "linked-docker-config");
    mkdirSync(dockerConfig, { mode: 0o700 });
    symlinkSync(join(ROOT, "README.md"), join(dockerConfig, "config.json"));
    const refused = run(linkedConfig, "up", { DOCKER_CONFIG: dockerConfig });
    assert.equal(refused.status, 78, refused.stderr);
    assert.match(refused.stderr, /Docker registry configuration file was refused/);
    assert.equal(existsSync(linkedConfig.log), false);
  } finally {
    rmSync(linkedConfig.scratch, { recursive: true, force: true });
  }
});

test("non-build actions scrub host build controls and preserve opaque registry auth", () => {
  const state = fixture();
  const dockerConfig = join(state.scratch, "opaque-docker-config");
  const dockerAuth = '{"auths":{"registry.invalid":{"auth":"cpr45-auth-sentinel"}}}';
  try {
    assert.equal(run(state, "up").status, 0);
    writeFileSync(state.log, "");
    const result = run(state, "down", {
      ...ambientHostBuild(),
      DOCKER_CONFIG: dockerConfig,
      DOCKER_AUTH_CONFIG: dockerAuth,
      SYNVEDA_FAKE_EXPECT_DOCKER_CONFIG: dockerConfig,
      SYNVEDA_FAKE_EXPECT_DOCKER_AUTH_CONFIG: dockerAuth,
    });
    assert.equal(result.status, 0, result.stderr);
    assert.match(readFileSync(state.log, "utf8"), / <down>/);
    assert.doesNotMatch(`${result.stdout}${result.stderr}`, /cpr45-auth-sentinel/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }

  const reference = fixture("reference");
  try {
    prepareReferenceFixture(reference);
    const result = run(reference, "config", ambientHostBuild());
    assert.equal(result.status, 0, result.stderr);
    assert.match(readFileSync(reference.log, "utf8"), / <config> <--quiet>/);
  } finally {
    rmSync(reference.scratch, { recursive: true, force: true });
  }
});

test("development build preserves opaque registry authentication outside the source", () => {
  const state = fixture();
  const dockerConfig = join(state.scratch, "opaque-docker-config");
  const dockerAuth = '{"auths":{"registry.invalid":{"auth":"cpr45-build-auth-sentinel"}}}';
  mkdirSync(dockerConfig, { mode: 0o700 });
  writeFileSync(join(dockerConfig, "config.json"), '{"auths":{}}\n', { mode: 0o600 });
  try {
    const result = run(state, "up", {
      DOCKER_CONFIG: dockerConfig,
      DOCKER_AUTH_CONFIG: dockerAuth,
      SYNVEDA_FAKE_EXPECT_DOCKER_CONFIG: dockerConfig,
      SYNVEDA_FAKE_EXPECT_DOCKER_AUTH_CONFIG: dockerAuth,
    });
    assert.equal(result.status, 0, result.stderr);
    const calls = readFileSync(state.log, "utf8");
    assert.match(calls, / <build> <--builder> <default>/);
    assert.match(calls, / <up> <--no-build>/);
    assert.doesNotMatch(`${result.stdout}${result.stderr}`, /cpr45-build-auth-sentinel/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("invalid reference TLS blocks startup actions and releases the exact-project lock", () => {
  const state = fixture("reference");
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  try {
    prepareReferenceFixture(state);
    writeFileSync(join(state.secrets, "tls_key"), "invalid-reference-key\n", {
      mode: 0o600,
    });
    for (const action of ["up", "smoke", "restart-gateway"]) {
      writeFileSync(state.log, "");
      const refused = run(state, action);
      assert.equal(refused.status, 78, `${action}: ${refused.stderr}`);
      assert.match(refused.stderr, /compose-tls: PEM structure was refused/);
      assert.equal(existsSync(lockFile), false, `${action} retained its lock`);
      const calls = readFileSync(state.log, "utf8");
      assert.match(calls, /check-tls-inputs\.mjs/);
      assert.doesNotMatch(
        calls,
        /^docker(?: |$)| <up>| <restart>| <down>| <volume>/m,
        `${action} reached Docker after TLS refusal`,
      );
    }
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("reference config preserves output and passes a non-replenishing TLS runway", () => {
  for (const { step, expected } of [
    { step: "10", expected: "230" },
    { step: "-10", expected: "240" },
  ]) {
    const state = fixture("reference");
    const clock = join(state.scratch, "clock");
    const output = join(state.scratch, `render-${expected}.json`);
    try {
      prepareReferenceFixture(state);
      writeFileSync(clock, "1000\n", { mode: 0o600 });
      writeFileSync(state.log, "");
      const result = spawnSync(WRAPPER, ["config", "--output", output], {
        cwd: ROOT,
        env: environment(state, {
          SYNVEDA_COMPOSE_LIFECYCLE_TIMEOUT_SECONDS: "240",
          SYNVEDA_FAKE_CLOCK_FILE: clock,
          SYNVEDA_FAKE_CLOCK_STEP: step,
        }),
        encoding: "utf8",
      });
      assert.equal(result.status, 0, result.stderr);
      const calls = readFileSync(state.log, "utf8");
      assert.match(
        calls,
        new RegExp(`check-tls-inputs\\.mjs[^\\n]*<--valid-for-seconds> <${expected}>`),
      );
      assert.ok(calls.includes(`<--output> <${output}>`), calls);
    } finally {
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }
});

test("reference teardown and confirmed reset exclude semantic TLS validation", () => {
  const state = fixture("reference");
  try {
    prepareReferenceFixture(state);
    writeFileSync(join(state.secrets, "tls_cert"), "semantically-invalid-certificate\n", {
      mode: 0o600,
    });
    writeFileSync(join(state.secrets, "tls_key"), "semantically-invalid-key\n", {
      mode: 0o600,
    });

    writeFileSync(state.log, "");
    const down = run(state, "down", {
      ...ambientHostTrust(),
      SYNVEDA_FAKE_FAIL_TLS_CHECK: "1",
    });
    assert.equal(down.status, 0, down.stderr);
    let calls = readFileSync(state.log, "utf8");
    assert.match(calls, / <down>/);
    assert.doesNotMatch(calls, /check-tls-inputs\.mjs/);

    writeFileSync(state.log, "");
    const reset = run(state, "reset", {
      ...ambientHostTrust(),
      SYNVEDA_CONFIRM_RESET: state.project,
      SYNVEDA_FAKE_FAIL_TLS_CHECK: "1",
    });
    assert.equal(reset.status, 0, reset.stderr);
    calls = readFileSync(state.log, "utf8");
    assert.match(calls, / <down>/);
    assert.doesNotMatch(calls, /check-tls-inputs\.mjs|docker <volume> <rm>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("canonical up prepares once, reruns convergence, and keeps credentials stable", () => {
  const state = fixture();
  try {
    const first = run(state, "up");
    assert.equal(first.status, 0, first.stderr);
    const password = readFileSync(join(state.secrets, "keycloak_demo_admin_password"), "utf8");
    const issuer = readFileSync(state.issuer, "utf8");
    assert.doesNotMatch(`${first.stdout}${first.stderr}${readFileSync(state.log, "utf8")}`, new RegExp(password.trim()));

    const second = run(state, "up");
    assert.equal(second.status, 0, second.stderr);
    assert.equal(readFileSync(join(state.secrets, "keycloak_demo_admin_password"), "utf8"), password);
    assert.equal(readFileSync(state.issuer, "utf8"), issuer);
    assert.match(second.stdout, /existing secret set validated/);
    assert.match(second.stdout, /existing project-scoped issuer configuration validated/);

    const calls = readFileSync(state.log, "utf8");
    const resolver = calls.indexOf("check-host-resolution.mjs");
    const network = calls.indexOf("check-network-preflight.mjs");
    const existingAssets = calls.indexOf("<--state> <existing>");
    const build = calls.indexOf(" <build> <--builder> <default>", existingAssets);
    const up = calls.indexOf(" <up>", build);
    const convergedAssets = calls.indexOf("<--state> <converged>");
    assert.ok(
      resolver >= 0 && network > resolver && existingAssets > network &&
        build > existingAssets && up > build && convergedAssets > up,
      calls,
    );
    assert.match(
      calls,
      / <up> <--no-build> <--detach> <--wait> <--wait-timeout> <900> <--force-recreate>/,
    );
    assert.doesNotMatch(calls, / <up> <--build>/);
    const buildxDirectories = [...calls.matchAll(/^buildx <(.+)>$/gm)].map(
      (match) => match[1],
    );
    assert.equal(buildxDirectories.length, 2, calls);
    for (const directory of buildxDirectories) {
      assert.ok(!directory.startsWith(`${ROOT}/`), directory);
      assert.equal(existsSync(directory), false, `${directory} was not cleaned`);
    }
    assert.doesNotMatch(calls, /--remove-orphans/);
    assert.match(calls, /compose\.demo\.yaml/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("acceptance up proves initial absence atomically and ordinary up stays rerunnable", () => {
  const state = fixture();
  try {
    const accepted = run(state, "up", {}, ["--initial-assets", "absent"]);
    assert.equal(accepted.status, 0, accepted.stderr);
    let calls = readFileSync(state.log, "utf8");
    const absent = calls.indexOf("<--state> <absent>");
    const build = calls.indexOf(" <build> <--builder> <default>");
    const up = calls.indexOf(" <up> <--no-build>");
    assert.ok(absent >= 0 && build > absent && up > build, calls);

    writeFileSync(state.log, "");
    const rerun = run(state, "up");
    assert.equal(rerun.status, 0, rerun.stderr);
    calls = readFileSync(state.log, "utf8");
    assert.match(calls, /<--state> <existing>/);
    assert.doesNotMatch(calls, /<--state> <absent>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("acceptance up refuses present assets and reference absence remains browser-scoped", () => {
  const state = fixture();
  try {
    const present = run(
      state,
      "up",
      { SYNVEDA_FAKE_ABSENT_ASSET_STATUS: "78" },
      ["--initial-assets", "absent"],
    );
    assert.equal(present.status, 78, present.stderr);
    const calls = readFileSync(state.log, "utf8");
    assert.match(calls, /<--state> <absent>/);
    assert.doesNotMatch(calls, / <build>| <up>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }

  const reference = fixture("reference");
  try {
    prepareReferenceFixture(reference);
    const refused = run(reference, "up", {}, ["--initial-assets", "absent"]);
    assert.equal(refused.status, 64, refused.stderr);
    assert.match(
      refused.stderr,
      /initial absence requires a suffixed development or reference browser-acceptance project/,
    );
    assert.equal(existsSync(reference.log), false);
  } finally {
    rmSync(reference.scratch, { recursive: true, force: true });
  }
});

test("browser acceptance is a clean one-shot whose exact result gates startup", () => {
  const state = fixture();
  try {
    const result = run(
      state,
      "up",
      { SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance" },
      ["--initial-assets", "absent"],
    );
    assert.equal(result.status, 0, result.stderr);
    const calls = readFileSync(state.log, "utf8");
    const seccomp = calls.indexOf("check-browser-seccomp.mjs");
    const absent = calls.indexOf("<--state> <absent>");
    const build = calls.indexOf(" <build> <--builder> <default>");
    const up = calls.indexOf(" <up> <--no-build>");
    const browserUp = calls.indexOf(" <up> <--no-build>", up + 1);
    const identity = calls.indexOf("<ps> <--all> <--quiet> <--no-trunc> <browser-acceptance>");
    const wait = calls.indexOf("docker <container> <wait>");
    const converged = calls.indexOf("<--state> <converged>", wait);
    const smoke = calls.indexOf("check-runtime-smoke.mjs", converged);
    assert.ok(
      seccomp >= 0 && absent > seccomp && build > absent && up > build &&
        browserUp > up && identity > browserUp && wait > identity &&
        converged > wait && smoke > converged,
      calls,
    );
    assert.match(
      calls,
      / <up> <--no-build> <--detach> <--wait> <--wait-timeout> <900> <--force-recreate> <--scale> <browser-acceptance=0>/,
    );
    assert.match(
      calls,
      / <up> <--no-build> <--detach> <--no-deps> <--force-recreate> <browser-acceptance>/,
    );
    assert.match(calls, /compose\.browser-acceptance\.yaml/);
    assert.match(calls, /<--profile> <browser-acceptance>/);
    assert.match(calls, /<--browser> <true>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("reference browser acceptance is pull-only and gates smoke after exact success", () => {
  const state = fixture("reference");
  try {
    prepareReferenceFixture(state);
    const result = run(
      state,
      "up",
      { SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance" },
      ["--initial-assets", "absent"],
    );
    assert.equal(result.status, 0, result.stderr);
    const calls = readFileSync(state.log, "utf8");
    const absent = calls.indexOf("<--state> <absent>");
    const up = calls.indexOf(" <up> <--no-build>");
    const browserUp = calls.indexOf(" <up> <--no-build>", up + 1);
    const wait = calls.indexOf("docker <container> <wait>", browserUp);
    const converged = calls.indexOf("<--state> <converged>", wait);
    const smoke = calls.indexOf("check-runtime-smoke.mjs", converged);
    assert.ok(
      absent >= 0 && up > absent && browserUp > up && wait > browserUp &&
        converged > wait && smoke > converged,
      calls,
    );
    assert.match(calls, /compose\.browser-acceptance\.yaml/);
    assert.doesNotMatch(calls, /compose\.browser-acceptance\.dev\.yaml/);
    assert.doesNotMatch(calls, / <build>|docker <context> <show>|docker <buildx>/);
    assert.match(calls, /<--runtime> <reference>/);
    assert.match(calls, /<--browser> <true>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("browser acceptance rejects unsafe selectors before Docker mutation", () => {
  const cases = [
    {
      extra: { SYNVEDA_COMPOSE_PROFILES: "browser-acceptance" },
      diagnostic: /requires demo,browser-acceptance and optional apalis only/,
    },
    {
      extra: { SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance,semantic" },
      diagnostic: /unsupported profile; allowed: demo,browser-acceptance/,
    },
    {
      extra: { SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance,demo" },
      diagnostic: /duplicate profile was refused/,
    },
    {
      extra: {
        SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance",
        SYNVEDA_COMPOSE_PROJECT_SUFFIX: "",
      },
      diagnostic:
        /initial absence requires a suffixed development or reference browser-acceptance project/,
    },
    {
      extra: {
        SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance",
        SYNVEDA_POSTGRES_MODE: "external",
      },
      diagnostic: /demo profile requires bundled PostgreSQL and bundled OIDC/,
    },
    {
      extra: {
        SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance",
        SYNVEDA_OIDC_MODE: "external",
      },
      diagnostic: /demo profile requires bundled PostgreSQL and bundled OIDC/,
    },
  ];
  for (const testCase of cases) {
    const state = fixture();
    try {
      const refused = run(state, "up", testCase.extra, ["--initial-assets", "absent"]);
      assert.equal(refused.status, 64, refused.stderr);
      assert.match(refused.stderr, testCase.diagnostic);
      assert.equal(existsSync(state.log), false);
    } finally {
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }

  const missingAbsence = fixture();
  try {
    const refused = run(missingAbsence, "up", {
      SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance",
    });
    assert.equal(refused.status, 64, refused.stderr);
    assert.match(refused.stderr, /requires --initial-assets absent/);
    assert.equal(existsSync(missingAbsence.log), false);
  } finally {
    rmSync(missingAbsence.scratch, { recursive: true, force: true });
  }

  const reference = fixture("reference");
  try {
    prepareReferenceFixture(reference);
    const refused = run(
      reference,
      "up",
      {
        SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance",
        SYNVEDA_BROWSER_IMAGE: "synveda/browser-acceptance:1.62.1-dev",
      },
      ["--initial-assets", "absent"],
    );
    assert.equal(refused.status, 64, refused.stderr);
    assert.match(refused.stderr, /reference browser acceptance image must use an OCI sha256 digest/);
    const calls = readFileSync(reference.log, "utf8");
    assert.doesNotMatch(calls, / <build>| <up>| <down>|docker <volume> <rm>/);
  } finally {
    rmSync(reference.scratch, { recursive: true, force: true });
  }
});

test("a known browser acceptance failure releases the project lock and never claims success", () => {
  const state = fixture();
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  try {
    const refused = run(
      state,
      "up",
      {
        SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance",
        SYNVEDA_FAKE_BROWSER_EXIT: "78",
      },
      ["--initial-assets", "absent"],
    );
    assert.equal(refused.status, 78, refused.stderr);
    assert.match(refused.stderr, /browser acceptance failed/);
    assert.doesNotMatch(refused.stdout, /services converged/);
    assert.equal(existsSync(lockFile), false);
    const calls = readFileSync(state.log, "utf8");
    assert.match(calls, /docker <container> <wait>/);
    assert.doesNotMatch(calls, /<--state> <converged>|check-runtime-smoke\.mjs/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("an uncertain browser container start retains the exact project lock", () => {
  const state = fixture();
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  try {
    const refused = run(
      state,
      "up",
      {
        SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance",
        SYNVEDA_FAKE_BROWSER_UP_FAIL: "1",
      },
      ["--initial-assets", "absent"],
    );
    assert.equal(refused.status, 43, refused.stderr);
    assert.equal(existsSync(lockFile), true);
    const calls = readFileSync(state.log, "utf8");
    assert.match(
      calls,
      / <up> <--no-build> <--detach> <--no-deps> <--force-recreate> <browser-acceptance>/,
    );
    assert.doesNotMatch(calls, /docker <container> <wait>|<--state> <converged>/);
  } finally {
    if (existsSync(lockFile)) rmSync(lockFile);
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("failed build retains its exact lock and removes only private Buildx state", () => {
  const state = fixture();
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  let marker;
  try {
    const failed = run(state, "up", { SYNVEDA_FAKE_BUILD_FAIL: "1" });
    assert.equal(failed.status, 42, failed.stderr);
    assert.match(failed.stderr, /Docker mutation state is uncertain \(compose-build\)/);
    const calls = readFileSync(state.log, "utf8");
    assert.match(calls, / <build> <--builder> <default>/);
    assert.doesNotMatch(calls, / <up>|<--state> <converged>/);
    const directory = calls.match(/^buildx <(.+)>$/m)?.[1];
    assert.ok(directory, calls);
    assert.equal(existsSync(directory), false, `${directory} was not cleaned`);
    marker = readFileSync(lockFile, "utf8");
    assert.equal(marker, `${state.project}:${failed.pid}\n`);

    writeFileSync(state.log, "");
    const blocked = run(state, "up");
    assert.equal(blocked.status, 75, blocked.stderr);
    assert.match(blocked.stderr, /another lifecycle or authority action owns/);
    assert.equal(readFileSync(state.log, "utf8"), "");
  } finally {
    if (marker !== undefined && existsSync(lockFile) && readFileSync(lockFile, "utf8") === marker) {
      rmSync(lockFile);
    }
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("timed-out build retains its exact lock and removes private Buildx state", () => {
  const state = fixture();
  const entered = join(state.scratch, "build-entered");
  const release = join(state.scratch, "build-release");
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  let marker;
  try {
    const timedOut = run(state, "up", {
      SYNVEDA_FAKE_BLOCK_BUILD: "1",
      SYNVEDA_FAKE_BUILD_ENTERED: entered,
      SYNVEDA_FAKE_BUILD_RELEASE: release,
      SYNVEDA_FAKE_BUILD_TIMEOUT: "1",
    });
    assert.equal(timedOut.status, 124, timedOut.stderr);
    assert.match(timedOut.stderr, /command exceeded 1 seconds/);
    assert.match(timedOut.stderr, /Docker mutation state is uncertain \(compose-build\)/);
    assert.equal(existsSync(entered), true);
    const calls = readFileSync(state.log, "utf8");
    assert.match(calls, / <build> <--builder> <default>/);
    assert.doesNotMatch(calls, / <up>|<--state> <converged>/);
    const directory = calls.match(/^buildx <(.+)>$/m)?.[1];
    assert.ok(directory, calls);
    assert.equal(existsSync(directory), false, `${directory} was not cleaned`);
    marker = readFileSync(lockFile, "utf8");
    assert.equal(marker, `${state.project}:${timedOut.pid}\n`);

    const blocked = run(state, "up");
    assert.equal(blocked.status, 75, blocked.stderr);
    assert.match(blocked.stderr, /another lifecycle or authority action owns/);
  } finally {
    writeFileSync(release, "continue\n", { mode: 0o600 });
    if (marker !== undefined && existsSync(lockFile) && readFileSync(lockFile, "utf8") === marker) {
      rmSync(lockFile);
    }
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("interrupted build retains its exact lock and removes private Buildx state", async () => {
  const state = fixture();
  const entered = join(state.scratch, "build-entered");
  const release = join(state.scratch, "build-release");
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  let marker;
  try {
    const wrapper = spawn(WRAPPER, ["up"], {
      cwd: ROOT,
      env: environment(state, {
        SYNVEDA_FAKE_BLOCK_BUILD: "1",
        SYNVEDA_FAKE_BUILD_ENTERED: entered,
        SYNVEDA_FAKE_BUILD_RELEASE: release,
      }),
      detached: true,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let errorOutput = "";
    wrapper.stderr.setEncoding("utf8");
    wrapper.stderr.on("data", (chunk) => { errorOutput += chunk; });
    wrapper.stdout.resume();
    const deadline = Date.now() + 8_000;
    while (!existsSync(entered)) {
      assert.ok(Date.now() < deadline, `timed out waiting for ${entered}`);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
    process.kill(wrapper.pid, "SIGTERM");
    const [status, signal] = await once(wrapper, "close");
    assert.equal(signal, null);
    assert.equal(status, 143, errorOutput);
    assert.match(errorOutput, /Docker mutation state is uncertain \(compose-build\)/);
    const calls = readFileSync(state.log, "utf8");
    assert.match(calls, / <build> <--builder> <default>/);
    assert.doesNotMatch(calls, / <up>|<--state> <converged>/);
    const directory = calls.match(/^buildx <(.+)>$/m)?.[1];
    assert.ok(directory, calls);
    assert.equal(existsSync(directory), false, `${directory} was not cleaned`);
    marker = readFileSync(lockFile, "utf8");
    assert.equal(marker, `${state.project}:${wrapper.pid}\n`);

    const blocked = run(state, "up");
    assert.equal(blocked.status, 75, blocked.stderr);
    assert.match(blocked.stderr, /another lifecycle or authority action owns/);
  } finally {
    writeFileSync(release, "continue\n", { mode: 0o600 });
    if (marker !== undefined && existsSync(lockFile) && readFileSync(lockFile, "utf8") === marker) {
      rmSync(lockFile);
    }
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("development build refuses a non-default context after endpoint pinning", () => {
  const state = fixture();
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  try {
    const refused = run(state, "up", {
      SYNVEDA_FAKE_CONTEXT_NAME: "remote-builder-context",
    });
    assert.equal(refused.status, 69, refused.stderr);
    assert.match(refused.stderr, /pinned Docker build context was refused/);
    const calls = readFileSync(state.log, "utf8");
    assert.match(calls, /docker <context> <show>/);
    assert.doesNotMatch(calls, / <build>| <up>/);
    assert.equal(existsSync(lockFile), false);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("development build requires one running embedded default builder before mutation", () => {
  for (const mode of ["remote", "stopped", "extra-node"]) {
    const state = fixture();
    const lockFile = join(
      "/tmp",
      `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
      `${state.project}.lock`,
    );
    try {
      const refused = run(state, "up", { SYNVEDA_FAKE_BUILDER_MODE: mode });
      assert.equal(refused.status, 78, `${mode}: ${refused.stderr}`);
      assert.match(refused.stderr, /^compose-builder: /m);
      assert.doesNotMatch(refused.stderr, /remote|stopped|foreign|tcp/);
      const calls = readFileSync(state.log, "utf8");
      assert.match(calls, /docker <buildx> <inspect> <--timeout> <20s> <default>/);
      assert.doesNotMatch(calls, / <build>| <up>/);
      assert.equal(existsSync(lockFile), false);
    } finally {
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }
});

test("reference startup is pull-only and never enters the source-build boundary", () => {
  const state = fixture("reference");
  try {
    prepareReferenceFixture(state);
    const result = run(state, "up");
    assert.equal(result.status, 0, result.stderr);
    const calls = readFileSync(state.log, "utf8");
    assert.match(
      calls,
      / <up> <--no-build> <--detach> <--wait> <--wait-timeout> <900> <--force-recreate>/,
    );
    assert.doesNotMatch(calls, / <build>| <--build>|docker <context> <show>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("up proves created proxy closure and keeps deterministic refusal recoverable", () => {
  const state = fixture();
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  try {
    const refused = run(state, "up", {
      SYNVEDA_FAKE_CONVERGED_ASSET_STATUS: "78",
    });
    assert.equal(refused.status, 78, refused.stderr);
    assert.doesNotMatch(refused.stdout, /canonical Compose services converged/);
    const calls = readFileSync(state.log, "utf8");
    const existing = calls.indexOf("<--state> <existing>");
    const up = calls.indexOf(" <up>", existing);
    const converged = calls.indexOf("<--state> <converged>", up);
    assert.ok(existing >= 0 && up > existing && converged > up, calls);
    assert.equal(existsSync(lockFile), false);

    writeFileSync(state.log, "");
    const down = run(state, "down");
    assert.equal(down.status, 0, down.stderr);
    const recoveryCalls = readFileSync(state.log, "utf8");
    assert.match(recoveryCalls, /<--state> <existing>/);
    assert.match(recoveryCalls, /<--state> <stopped>/);
    assert.doesNotMatch(recoveryCalls, /<--state> <converged>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("unavailable post-create asset evidence retains the exact project lock", () => {
  const state = fixture();
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  let marker;
  try {
    const refused = run(state, "up", {
      SYNVEDA_FAKE_CONVERGED_ASSET_STATUS: "69",
    });
    assert.equal(refused.status, 69, refused.stderr);
    assert.match(refused.stderr, /Docker mutation state is uncertain \(compose-up\)/);
    marker = readFileSync(lockFile, "utf8");
    assert.match(marker, new RegExp(`^${state.project}:[1-9][0-9]*\\n$`));
  } finally {
    if (
      marker !== undefined && existsSync(lockFile) &&
      readFileSync(lockFile, "utf8") === marker
    ) {
      rmSync(lockFile);
    }
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("down retains data and reset requires exact confirmation", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "up").status, 0);
    const secret = readFileSync(join(state.secrets, "synveda_kms_key"), "utf8");
    const issuer = readFileSync(state.issuer, "utf8");

    const down = run(state, "down");
    assert.equal(down.status, 0, down.stderr);
    assert.equal(existsSync(state.secrets), true);
    assert.equal(readFileSync(join(state.secrets, "synveda_kms_key"), "utf8"), secret);

    const refused = run(state, "reset", { SYNVEDA_FAKE_VOLUME_MODE: "exact" });
    assert.equal(refused.status, 64);
    assert.match(refused.stderr, new RegExp(`SYNVEDA_CONFIRM_RESET=${state.project}`));

    const reset = run(state, "reset", {
      SYNVEDA_FAKE_VOLUME_MODE: "exact",
      SYNVEDA_CONFIRM_RESET: state.project,
    });
    assert.equal(reset.status, 0, reset.stderr);
    assert.equal(readFileSync(join(state.secrets, "synveda_kms_key"), "utf8"), secret);
    assert.equal(readFileSync(state.issuer, "utf8"), issuer);
    const calls = readFileSync(state.log, "utf8");
    assert.match(
      calls,
      new RegExp(`docker <volume> <rm> <${state.project}_postgres-data>`),
    );
    assert.doesNotMatch(calls, /down> <-v>|prune|system/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("observability selects one private overlay and routes exact smoke evidence", () => {
  const state = fixture();
  const profile = { SYNVEDA_COMPOSE_PROFILES: "observability" };
  try {
    const up = run(state, "up", profile);
    assert.equal(up.status, 0, up.stderr);
    let calls = readFileSync(state.log, "utf8");
    assert.match(calls, /compose\.observability\.yaml>/);
    assert.match(calls, /<--profile> <observability>/);

    writeFileSync(state.log, "");
    const smoke = run(state, "smoke", profile);
    assert.equal(smoke.status, 0, smoke.stderr);
    calls = readFileSync(state.log, "utf8");
    assert.match(
      calls,
      /check-runtime-smoke\.mjs>.*<--observability> <true>.*<--prometheus-url> <http:\/\/127\.0\.0\.1:9090>/,
    );
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("experimental Apalis selects only its private process overlay and exact smoke evidence", () => {
  const state = fixture();
  const profile = { SYNVEDA_COMPOSE_PROFILES: "apalis" };
  try {
    const up = run(state, "up", profile);
    assert.equal(up.status, 0, up.stderr);
    let calls = readFileSync(state.log, "utf8");
    assert.match(calls, /compose\.apalis\.yaml>/);
    assert.match(calls, /compose\.apalis\.dev\.yaml>/);
    assert.match(calls, /<--profile> <apalis>/);
    assert.doesNotMatch(calls, /compose\.observability\.yaml>|compose\.external\.yaml>/);

    writeFileSync(state.log, "");
    const smoke = run(state, "smoke", profile);
    assert.equal(smoke.status, 0, smoke.stderr);
    calls = readFileSync(state.log, "utf8");
    assert.match(calls, /check-runtime-smoke\.mjs>.*<--apalis> <true>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("experimental Apalis refuses external providers and unproved lifecycle actions", () => {
  for (const extra of [
    { SYNVEDA_COMPOSE_PROFILES: "apalis", SYNVEDA_POSTGRES_MODE: "external" },
    { SYNVEDA_COMPOSE_PROFILES: "apalis", SYNVEDA_OIDC_MODE: "external" },
  ]) {
    const state = fixture();
    try {
      const refused = run(state, "config", extra);
      assert.equal(refused.status, 64, refused.stderr);
      assert.match(refused.stderr, /experimental Apalis requires bundled PostgreSQL and bundled OIDC/);
      assert.equal(existsSync(state.log), false);
    } finally {
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }

  for (const action of [
    "backup",
    "restore-smoke",
    "upgrade-smoke",
    "restart-gateway",
  ]) {
    const state = fixture();
    try {
      const refused = run(state, action, { SYNVEDA_COMPOSE_PROFILES: "apalis" });
      assert.equal(refused.status, 69, `${action}: ${refused.stderr}`);
      assert.match(refused.stderr, /experimental Apalis is unavailable for this lifecycle action/);
      assert.equal(existsSync(state.log), false);
    } finally {
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }
});

test("experimental Apalis down retains its queue and confirmed reset removes it before product data", () => {
  const state = fixture();
  const profile = { SYNVEDA_COMPOSE_PROFILES: "apalis" };
  try {
    assert.equal(run(state, "up", profile).status, 0);
    writeFileSync(state.log, "");
    const down = run(state, "down", {
      ...profile,
      SYNVEDA_FAKE_VOLUME_MODE: "exact",
    });
    assert.equal(down.status, 0, down.stderr);
    assert.doesNotMatch(readFileSync(state.log, "utf8"), /<volume> <rm>/);

    writeFileSync(state.log, "");
    const reset = run(state, "reset", {
      ...profile,
      SYNVEDA_FAKE_VOLUME_MODE: "exact",
      SYNVEDA_CONFIRM_RESET: state.project,
    });
    assert.equal(reset.status, 0, reset.stderr);
    const calls = readFileSync(state.log, "utf8");
    const queueRemoval = calls.indexOf(
      `docker <volume> <rm> <${state.project}_apalis-data>`,
    );
    const productRemoval = calls.indexOf(
      `docker <volume> <rm> <${state.project}_postgres-data>`,
    );
    assert.ok(queueRemoval >= 0 && productRemoval > queueRemoval, calls);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("external OTLP selects one TLS exporter overlay and composes with local metrics", () => {
  for (const [profiles, configName] of [
    ["demo", "collector.external.yaml"],
    ["observability", "collector.external.observability.yaml"],
  ]) {
    const state = fixture();
    const collectorConfig = join(ROOT, "deploy/compose/configs/otel", configName);
    try {
      const up = run(state, "up", {
        SYNVEDA_COMPOSE_PROFILES: profiles,
        SYNVEDA_OTLP_MODE: "external",
        SYNVEDA_OTLP_EXPORT_ENDPOINT: "telemetry.lifecycle.example:4317",
        SYNVEDA_RENDER_OTEL_COLLECTOR_CONFIG: "/tmp/ambient-injection.yaml",
        SYNVEDA_FAKE_EXPECT_COLLECTOR_CONFIG: collectorConfig,
      });
      assert.equal(up.status, 0, up.stderr);
      const calls = readFileSync(state.log, "utf8");
      assert.equal(calls.match(/compose\.otlp-external\.yaml>/g)?.length, 3, calls);
      assert.doesNotMatch(calls, /ambient-injection/);
      if (profiles === "observability") {
        assert.match(calls, /compose\.observability\.yaml>/);
      }
    } finally {
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }
});

test("external OTLP refuses ambiguous selectors and endpoints before Docker", () => {
  const cases = [
    [{ SYNVEDA_OTLP_MODE: "remote" }, /SYNVEDA_OTLP_MODE must be discard\|external/],
    [
      { SYNVEDA_OTLP_MODE: "external" },
      /SYNVEDA_OTLP_EXPORT_ENDPOINT must be a lower-case DNS name and canonical TCP port/,
    ],
    [
      { SYNVEDA_OTLP_EXPORT_ENDPOINT: "telemetry.lifecycle.example:4317" },
      /accepted only when SYNVEDA_OTLP_MODE=external/,
    ],
    ...[
      "https://telemetry.lifecycle.example:4317",
      "TELEMETRY.lifecycle.example:4317",
      "127.0.0.1:4317",
      "telemetry.lifecycle.example",
      "telemetry.lifecycle.example:04317",
      "telemetry.lifecycle.example:0",
      "telemetry.lifecycle.example:65536",
      "user@telemetry.lifecycle.example:4317",
      "telemetry.lifecycle.example:4317/path",
    ].map((endpoint) => [
      { SYNVEDA_OTLP_MODE: "external", SYNVEDA_OTLP_EXPORT_ENDPOINT: endpoint },
      /SYNVEDA_OTLP_EXPORT_ENDPOINT must be a lower-case DNS name and canonical TCP port/,
    ]),
  ];
  for (const [extra, diagnostic] of cases) {
    const state = fixture();
    try {
      const refused = run(state, "config", extra);
      assert.equal(refused.status, 64, refused.stderr);
      assert.match(refused.stderr, diagnostic);
      assert.equal(existsSync(state.log), false, "refused input reached Docker");
    } finally {
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }
});

test("external PostgreSQL and OIDC start through the same product graph with strict TLS inputs", () => {
  const state = fixture();
  try {
    const external = prepareExternalPostgresFixture(state);
    const before = Object.fromEntries(
      [
        "synveda_migrator_database_url",
        "synveda_gateway_database_url",
        "synveda_worker_database_url",
        "synveda_kms_key",
        "postgres_root_ca",
      ].map((name) => [name, readFileSync(join(state.secrets, name), "utf8")]),
    );
    const up = run(state, "up", external);
    assert.equal(up.status, 0, up.stderr);
    const calls = readFileSync(state.log, "utf8");
    assert.match(calls, /compose\.external-postgres\.yaml>/);
    assert.match(calls, /compose\.external\.yaml>/);
    assert.doesNotMatch(calls, /compose\.postgres\.yaml>|compose\.keycloak\.yaml>/);
    assert.match(calls, / <up> <--no-build> <--detach> <--wait>/);
    for (const [name, value] of Object.entries(before)) {
      assert.equal(readFileSync(join(state.secrets, name), "utf8"), value, name);
    }
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("external PostgreSQL refuses unsafe CA files before Compose mutation", () => {
  const mutations = [
    ["missing", (path) => rmSync(path)],
    ["empty", (path) => writeFileSync(path, "", { mode: 0o600 })],
    ["public", (path) => chmodSync(path, 0o644)],
    [
      "symlink",
      (path, state) => {
        const target = join(state.runtime, "root-ca-target");
        writeFileSync(target, "fixture\n", { mode: 0o600 });
        rmSync(path);
        symlinkSync(target, path);
      },
    ],
  ];
  for (const [name, mutate] of mutations) {
    const state = fixture();
    try {
      const external = prepareExternalPostgresFixture(state);
      mutate(join(state.secrets, "postgres_root_ca"), state);
      const refused = run(state, "up", external);
      assert.equal(refused.status, 78, `${name}: ${refused.stderr}`);
      assert.match(refused.stderr, /required postgres_root_ca file/);
      const calls = existsSync(state.log) ? readFileSync(state.log, "utf8") : "";
      assert.doesNotMatch(calls, / <build>| <up>/, name);
    } finally {
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }
});

test("external PostgreSQL keeps bundled identity and owned-data actions out of scope", () => {
  for (const [action, extra, diagnostic] of [
    [
      "up",
      { SYNVEDA_POSTGRES_MODE: "external", SYNVEDA_COMPOSE_PROFILES: "" },
      /executable external PostgreSQL currently requires external OIDC/,
    ],
    [
      "reset",
      {
        SYNVEDA_POSTGRES_MODE: "external",
        SYNVEDA_OIDC_MODE: "external",
        SYNVEDA_COMPOSE_PROFILES: "",
      },
      /acceptance, restore and reset are unavailable/,
    ],
  ]) {
    const state = fixture();
    try {
      const refused = run(state, action, extra);
      assert.equal(refused.status, 69, refused.stderr);
      assert.match(refused.stderr, diagnostic);
      assert.equal(existsSync(state.log), false);
    } finally {
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }

  const state = fixture();
  try {
    const refused = spawnSync(SECRET_GENERATOR, ["--if-missing"], {
      cwd: ROOT,
      env: environment(state, { SYNVEDA_POSTGRES_MODE: "external" }),
      encoding: "utf8",
    });
    assert.equal(refused.status, 69, refused.stderr);
    assert.match(refused.stderr, /credentials and CA are operator-owned/);
    assert.equal(existsSync(state.secrets), false);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("observability refuses an unpinned image and unsafe operator ports", () => {
  for (const [extra, diagnostic] of [
    [
      { SYNVEDA_PROMETHEUS_IMAGE: "prom/prometheus:v3.13.3-distroless" },
      /Prometheus image must use an OCI sha256 digest/,
    ],
    [{ SYNVEDA_PROMETHEUS_PORT: "80" }, /1024 through 65535 distinct/],
    [{ SYNVEDA_PROMETHEUS_PORT: "08080" }, /canonical integer/],
    [{ SYNVEDA_PROMETHEUS_PORT: "8080" }, /1024 through 65535 distinct/],
    [{ SYNVEDA_PROMETHEUS_PORT: "65536" }, /1024 through 65535 distinct/],
  ]) {
    const state = fixture();
    try {
      const preparedEnvironment = environment(state, {
        SYNVEDA_COMPOSE_PROFILES: "observability",
      });
      for (const script of [SECRET_GENERATOR, ISSUER_GENERATOR]) {
        const prepared = spawnSync(script, ["--if-missing"], {
          cwd: ROOT,
          env: preparedEnvironment,
          encoding: "utf8",
        });
        assert.equal(prepared.status, 0, prepared.stderr);
      }
      const refused = run(state, "config", {
        SYNVEDA_COMPOSE_PROFILES: "observability",
        ...extra,
      });
      assert.equal(refused.status, 64, refused.stderr);
      assert.match(refused.stderr, diagnostic);
      assert.equal(existsSync(state.log), false, "refused input reached Docker");
    } finally {
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }
});

test("observability down retains metrics and confirmed reset removes them before data", () => {
  const state = fixture();
  const profile = { SYNVEDA_COMPOSE_PROFILES: "observability" };
  try {
    assert.equal(run(state, "up", profile).status, 0);
    writeFileSync(state.log, "");
    const down = run(state, "down", {
      ...profile,
      SYNVEDA_FAKE_VOLUME_MODE: "exact",
    });
    assert.equal(down.status, 0, down.stderr);
    assert.doesNotMatch(readFileSync(state.log, "utf8"), /<volume> <rm>/);

    writeFileSync(state.log, "");
    const reset = run(state, "reset", {
      ...profile,
      SYNVEDA_FAKE_VOLUME_MODE: "exact",
      SYNVEDA_CONFIRM_RESET: state.project,
    });
    assert.equal(reset.status, 0, reset.stderr);
    const calls = readFileSync(state.log, "utf8");
    const metricsRemoval = calls.indexOf(
      `docker <volume> <rm> <${state.project}_prometheus-data>`,
    );
    const dataRemoval = calls.indexOf(
      `docker <volume> <rm> <${state.project}_postgres-data>`,
    );
    assert.ok(metricsRemoval >= 0 && dataRemoval > metricsRemoval, calls);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("observability down refuses post-stop metrics-volume drift", () => {
  const state = fixture();
  const profile = { SYNVEDA_COMPOSE_PROFILES: "observability" };
  try {
    assert.equal(run(state, "up", profile).status, 0);
    writeFileSync(state.log, "");
    const refused = run(state, "down", {
      ...profile,
      SYNVEDA_FAKE_VOLUME_MODE: "exact",
      SYNVEDA_FAKE_VOLUME_CONTRACT: "drift-after-down",
      SYNVEDA_FAKE_VOLUME_CONTRACT_KEY: "prometheus-data",
    });
    assert.equal(refused.status, 78, refused.stderr);
    assert.match(refused.stderr, /exact project prometheus-data volume contract was refused/);
    assert.doesNotMatch(readFileSync(state.log, "utf8"), /<volume> <rm>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("browser acceptance reset removes its private receipt and credential volume first", () => {
  const state = fixture();
  const profiles = {
    SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance",
  };
  try {
    assert.equal(
      run(state, "up", profiles, ["--initial-assets", "absent"]).status,
      0,
    );
    writeFileSync(state.log, "");
    const reset = run(state, "reset", {
      ...profiles,
      SYNVEDA_FAKE_VOLUME_MODE: "exact",
      SYNVEDA_CONFIRM_RESET: state.project,
    });
    assert.equal(reset.status, 0, reset.stderr);
    const calls = readFileSync(state.log, "utf8");
    const browserRemoval = calls.indexOf(
      `docker <volume> <rm> <${state.project}_browser-acceptance-state>`,
    );
    const dataRemoval = calls.indexOf(
      `docker <volume> <rm> <${state.project}_postgres-data>`,
    );
    assert.ok(browserRemoval >= 0 && dataRemoval > browserRemoval, calls);
    assert.doesNotMatch(calls, /down> <-v>|prune|system/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("browser acceptance reset refuses a foreign fixture-state volume before shutdown", () => {
  const state = fixture();
  const profiles = {
    SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance",
  };
  try {
    assert.equal(
      run(state, "up", profiles, ["--initial-assets", "absent"]).status,
      0,
    );
    writeFileSync(state.log, "");
    const refused = run(state, "reset", {
      ...profiles,
      SYNVEDA_FAKE_VOLUME_MODE: "exact",
      SYNVEDA_FAKE_VOLUME_CONTRACT: "wrong-driver",
      SYNVEDA_FAKE_VOLUME_CONTRACT_KEY: "browser-acceptance-state",
      SYNVEDA_CONFIRM_RESET: state.project,
    });
    assert.equal(refused.status, 78, refused.stderr);
    assert.match(
      refused.stderr,
      /exact project browser-acceptance-state volume contract was refused/,
    );
    assert.doesNotMatch(readFileSync(state.log, "utf8"), / <down>|<volume> <rm>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("browser-profile down refuses a foreign fixture-state volume before shutdown", () => {
  const state = fixture();
  const profiles = {
    SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance",
  };
  try {
    assert.equal(
      run(state, "up", profiles, ["--initial-assets", "absent"]).status,
      0,
    );
    writeFileSync(state.log, "");
    const refused = run(state, "down", {
      ...profiles,
      SYNVEDA_FAKE_VOLUME_MODE: "exact",
      SYNVEDA_FAKE_VOLUME_CONTRACT: "wrong-driver",
      SYNVEDA_FAKE_VOLUME_CONTRACT_KEY: "browser-acceptance-state",
    });
    assert.equal(refused.status, 78, refused.stderr);
    assert.match(
      refused.stderr,
      /exact project browser-acceptance-state volume contract was refused/,
    );
    assert.doesNotMatch(readFileSync(state.log, "utf8"), / <down>|<volume> <rm>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("reset refuses unavailable, ambiguous, or drifted volume authority before mutation", () => {
  const cases = [
    {
      extra: { SYNVEDA_FAKE_VOLUME_INVENTORY_ERROR: "1" },
      status: 69,
      error: /volume inventory was unavailable/,
    },
    {
      extra: { SYNVEDA_FAKE_VOLUME_MODE: "named-only" },
      status: 78,
      error: /exact project postgres-data volume inventory was refused/,
    },
    {
      extra: { SYNVEDA_FAKE_VOLUME_MODE: "labelled-wrong-name" },
      status: 78,
      error: /exact project postgres-data volume inventory was refused/,
    },
    ...["wrong-driver", "wrong-scope", "wrong-options"].map((contract) => ({
      extra: {
        SYNVEDA_FAKE_VOLUME_MODE: "exact",
        SYNVEDA_FAKE_VOLUME_CONTRACT: contract,
      },
      status: 78,
      error: /exact project postgres-data volume contract was refused/,
    })),
    {
      extra: {
        SYNVEDA_FAKE_VOLUME_MODE: "exact",
        SYNVEDA_FAKE_VOLUME_CONTRACT: "inspect-error",
      },
      status: 69,
      error: /exact project postgres-data volume inspection failed/,
    },
  ];

  for (const item of cases) {
    const state = fixture();
    try {
      assert.equal(run(state, "up").status, 0);
      writeFileSync(state.log, "");
      const refused = run(state, "reset", {
        SYNVEDA_CONFIRM_RESET: state.project,
        ...item.extra,
      });
      assert.equal(refused.status, item.status, refused.stderr);
      assert.match(refused.stderr, item.error);
      const calls = readFileSync(state.log, "utf8");
      assert.doesNotMatch(calls, / <down>|docker <volume> <rm>/, calls);
    } finally {
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }
});

test("reset preserves authority state when the volume contract changes after shutdown", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "up").status, 0);
    writeFileSync(state.log, "");
    const refused = run(state, "reset", {
      SYNVEDA_CONFIRM_RESET: state.project,
      SYNVEDA_FAKE_VOLUME_MODE: "exact",
      SYNVEDA_FAKE_VOLUME_CONTRACT: "drift-after-down",
    });
    assert.equal(refused.status, 78, refused.stderr);
    assert.match(
      refused.stderr,
      /exact project postgres-data volume contract was refused/,
    );
    assert.equal(existsSync(state.authority), true);
    assert.equal(existsSync(state.gate), true);
    const calls = readFileSync(state.log, "utf8");
    assert.match(calls, / <down>/);
    assert.doesNotMatch(calls, /docker <volume> <rm>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("reset refuses an unavailable post-removal inventory before authority deletion", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "up").status, 0);
    writeFileSync(state.log, "");
    const refused = run(state, "reset", {
      SYNVEDA_CONFIRM_RESET: state.project,
      SYNVEDA_FAKE_VOLUME_MODE: "exact",
      SYNVEDA_FAKE_POST_REMOVE_INVENTORY_ERROR: "1",
    });
    assert.equal(refused.status, 69, refused.stderr);
    assert.match(
      refused.stderr,
      /named project postgres-data volume inventory was unavailable/,
    );
    assert.equal(existsSync(state.authority), true);
    assert.equal(existsSync(state.gate), true);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("hosts plan is content-free and smoke is routed through bounded checkers", () => {
  const state = fixture();
  try {
    const plan = run(state, "hosts-plan");
    assert.equal(plan.status, 0, plan.stderr);
    assert.equal(
      plan.stdout,
      `# BEGIN SYNVEDA ${state.project}\n127.0.0.1 app.synveda.test auth.synveda.test\n# END SYNVEDA ${state.project}\n`,
    );
    assert.equal(run(state, "up").status, 0);
    writeFileSync(state.log, "");
    const smoke = run(state, "smoke");
    assert.equal(smoke.status, 0, smoke.stderr);
    const calls = readFileSync(state.log, "utf8");
    const existingAssets = calls.indexOf("<--state> <existing>");
    const convergedAssets = calls.indexOf("<--state> <converged>");
    const runtimeSmoke = calls.indexOf("check-runtime-smoke.mjs");
    assert.ok(
      existingAssets >= 0 && convergedAssets > existingAssets &&
        runtimeSmoke > convergedAssets,
      calls,
    );
    assert.match(calls, /docker .* <ps> <--all> <--format> <json>/);
    assert.match(calls, /node <.*check-runtime-smoke\.mjs>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("owned development host state precedes every Docker prerequisite", () => {
  for (const action of ["resolver-check", "up", "smoke", "restart-gateway"]) {
    const state = fixture();
    const lockFile = join(
      "/tmp",
      `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
      `${state.project}.lock`,
    );
    try {
      const refused = run(state, action, { SYNVEDA_FAKE_HOSTS_STATUS: "absent" });
      assert.equal(refused.status, 78, `${action}: ${refused.stderr}`);
      assert.match(refused.stderr, /hosts mapping is absent, expected installed/);
      const calls = readFileSync(state.log, "utf8");
      assert.match(
        calls,
        /manage-hosts-file\.mjs> <status>[^\n]*<--expect> <installed>/,
      );
      assert.doesNotMatch(
        calls,
        /check-host-resolution\.mjs|check-network-preflight\.mjs|check-compose-assets\.mjs|check-runtime-smoke\.mjs|^docker/m,
      );
      assert.equal(existsSync(lockFile), false, `${action} retained a certain preflight lock`);
    } finally {
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }
});

test("up distinguishes owned status, endpoint pin, full resolution, and mutation order", () => {
  const state = fixture();
  try {
    const result = run(state, "up");
    assert.equal(result.status, 0, result.stderr);
    const calls = readFileSync(state.log, "utf8");
    assert.equal(
      [...calls.matchAll(/manage-hosts-file\.mjs> <status>[^\n]*<--expect> <installed>/g)]
        .length,
      2,
      calls,
    );
    const firstStatus = calls.indexOf("manage-hosts-file.mjs> <status>");
    const endpointPin = calls.indexOf(
      "check-host-resolution.mjs> <--docker-only> <true> <--print-docker-endpoint> <true>",
    );
    const repeatedStatus = calls.indexOf("manage-hosts-file.mjs> <status>", firstStatus + 1);
    const fullResolver = calls.indexOf(
      "check-host-resolution.mjs> <--runtime> <development>",
      endpointPin + 1,
    );
    const network = calls.indexOf("check-network-preflight.mjs>", fullResolver + 1);
    const build = calls.indexOf(" <build> <--builder> <default>", network + 1);
    const up = calls.indexOf(" <up> <--no-build>", build + 1);
    assert.ok(
      firstStatus >= 0 &&
        endpointPin > firstStatus &&
        repeatedStatus > endpointPin &&
        fullResolver > repeatedStatus &&
        network > fullResolver &&
        build > network &&
        up > build,
      calls,
    );
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("logical backup stops writers, publishes a linked recovery set, resumes, and smokes", () => {
  const state = fixture();
  const roots = recoveryRoots(state);
  const backupId = "lifecycle-success";
  try {
    assert.equal(run(state, "up").status, 0);
    const kms = readFileSync(join(state.secrets, "synveda_kms_key"), "utf8").trim();
    const convergence = readFileSync(
      join(state.secrets, "keycloak_convergence_admin_password"),
      "utf8",
    ).trim();
    writeFileSync(state.log, "");
    const result = run(state, "backup", {
      ...roots.environment,
      SYNVEDA_BACKUP_ID: backupId,
    });
    assert.equal(result.status, 0, result.stderr);
    const calls = readFileSync(state.log, "utf8");
    const preflight = calls.indexOf("check-runtime-smoke.mjs");
    const stop = calls.indexOf(
      " <stop> <--timeout> <210> <gateway> <worker> <keycloak-realm-convergence> <keycloak>",
    );
    const dump = calls.indexOf(" <run> <--rm> <--no-deps> <database-backup>", stop);
    const resume = calls.indexOf(
      " <up> <--no-build> <--detach> <--wait> <--wait-timeout>",
      dump,
    );
    const postflight = calls.indexOf("check-runtime-smoke.mjs", resume);
    assert.ok(
      preflight >= 0 && stop > preflight && dump > stop && resume > dump && postflight > resume,
      calls,
    );
    assert.deepEqual(
      readdirSync(join(roots.database, backupId)).sort(),
      ["keycloak.dump", "manifest.json", "synveda.dump"],
    );
    assert.deepEqual(
      readdirSync(join(roots.secrets, backupId)).sort(),
      [
        "keycloak_convergence_admin_password",
        "manifest.json",
        "synveda_kms_key",
        "synveda_kms_key_ref",
      ],
    );
    assert.doesNotMatch(`${result.stdout}${result.stderr}${calls}`, new RegExp(`${kms}|${convergence}`));
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("logical backup always attempts writer resumption after ordinary stop or dump failure", () => {
  for (const failure of ["stop", "dump"]) {
    const state = fixture();
    const roots = recoveryRoots(state);
    try {
      assert.equal(run(state, "up").status, 0);
      writeFileSync(state.log, "");
      const result = run(state, "backup", {
        ...roots.environment,
        SYNVEDA_BACKUP_ID: `lifecycle-${failure}-failure`,
        ...(failure === "stop"
          ? { SYNVEDA_FAKE_BACKUP_STOP_FAIL: "1" }
          : { SYNVEDA_FAKE_BACKUP_FAIL: "1" }),
      });
      assert.equal(result.status, failure === "stop" ? 46 : 47, result.stderr);
      assert.match(result.stderr, /publication did not complete/);
      const calls = readFileSync(state.log, "utf8");
      const stop = calls.indexOf(" <stop> <--timeout> <210>");
      const dump = calls.indexOf(" <run> <--rm> <--no-deps> <database-backup>");
      const resume = calls.indexOf(
        " <up> <--no-build> <--detach> <--wait> <--wait-timeout>",
        stop,
      );
      assert.ok(stop >= 0 && resume > stop, calls);
      if (failure === "stop") assert.equal(dump, -1, calls);
      else assert.ok(dump > stop && resume > dump, calls);
      assert.equal(existsSync(join(roots.database, `lifecycle-${failure}-failure`)), false);
      assert.equal(existsSync(join(roots.secrets, `lifecycle-${failure}-failure`)), false);
    } finally {
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }
});

test("logical backup never nests into a destination that appears during publication", () => {
  const state = fixture();
  const roots = recoveryRoots(state);
  const backupId = "late-destination";
  try {
    assert.equal(run(state, "up").status, 0);
    writeFileSync(state.log, "");
    const result = run(state, "backup", {
      ...roots.environment,
      SYNVEDA_BACKUP_ID: backupId,
      SYNVEDA_FAKE_LATE_DATABASE_FINAL: "1",
    });
    assert.equal(result.status, 73, result.stderr);
    assert.match(result.stderr, /destination appeared before publication/);
    assert.match(result.stderr, /publication did not complete/);
    assert.deepEqual(
      readdirSync(join(roots.database, backupId)),
      ["sentinel"],
      "the winning destination was modified or received a nested stage",
    );
    assert.equal(
      readFileSync(join(roots.database, backupId, "sentinel"), "utf8"),
      "winning recovery set\n",
    );
    assert.equal(existsSync(join(roots.secrets, backupId)), true);
    const calls = readFileSync(state.log, "utf8");
    assert.match(
      calls,
      / <up> <--no-build> <--detach> <--wait> <--wait-timeout> <3600> <--no-recreate> <keycloak> <keycloak-realm-convergence> <worker> <gateway>/,
    );
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("restore confirmation is refused before target generation, locking, or Docker", () => {
  const state = fixture();
  const roots = recoveryRoots(state);
  try {
    const result = run(state, "restore-smoke", {
      ...roots.environment,
      SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance",
      SYNVEDA_RESTORE_SOURCE_PROJECT: "synveda-development-acceptance-source",
      SYNVEDA_BACKUP_ID: "refused-restore",
    });
    assert.equal(result.status, 64, result.stderr);
    assert.match(result.stderr, /requires SYNVEDA_CONFIRM_RESTORE=/);
    assert.equal(existsSync(state.log), false);
    assert.equal(existsSync(state.secrets), false);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("restore refuses a target tenant that is not the backed-up source tenant", () => {
  const source = fixture();
  const target = fixture();
  const roots = recoveryRoots(source);
  const backupId = "tenant-mismatch";
  try {
    assert.equal(run(source, "up").status, 0);
    const backedUp = run(source, "backup", {
      ...roots.environment,
      SYNVEDA_BACKUP_ID: backupId,
    });
    assert.equal(backedUp.status, 0, backedUp.stderr);
    const refused = run(target, "restore-smoke", {
      ...roots.environment,
      SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance",
      SYNVEDA_RESTORE_SOURCE_PROJECT: source.project,
      SYNVEDA_BACKUP_ID: backupId,
      SYNVEDA_BOOTSTRAP_TENANT_ID: "019b53c0-7c00-7000-8000-000000000046",
      SYNVEDA_CONFIRM_RESTORE: `${source.project}:${backupId}:${target.project}`,
    });
    assert.equal(refused.status, 78, refused.stderr);
    assert.match(refused.stderr, /database manifest values were refused/);
    const calls = existsSync(target.log) ? readFileSync(target.log, "utf8") : "";
    assert.doesNotMatch(calls, / <config>| <build>| <up>| <run>/);
  } finally {
    rmSync(target.scratch, { recursive: true, force: true });
    rmSync(source.scratch, { recursive: true, force: true });
  }
});

test("restore refusal preserves every existing target recovery credential", () => {
  const source = fixture();
  const target = fixture();
  const roots = recoveryRoots(source);
  const backupId = "existing-target-refusal";
  try {
    assert.equal(run(source, "up").status, 0);
    const backedUp = run(source, "backup", {
      ...roots.environment,
      SYNVEDA_BACKUP_ID: backupId,
    });
    assert.equal(backedUp.status, 0, backedUp.stderr);
    assert.equal(run(target, "up").status, 0);
    const names = [
      "synveda_kms_key",
      "synveda_kms_key_ref",
      "keycloak_convergence_admin_password",
    ];
    const before = new Map(
      names.map((name) => [name, readFileSync(join(target.secrets, name))]),
    );
    writeFileSync(target.log, "");

    const refused = run(target, "restore-smoke", {
      ...roots.environment,
      SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance",
      SYNVEDA_RESTORE_SOURCE_PROJECT: source.project,
      SYNVEDA_BACKUP_ID: backupId,
      SYNVEDA_CONFIRM_RESTORE: `${source.project}:${backupId}:${target.project}`,
      SYNVEDA_FAKE_ABSENT_ASSET_STATUS: "78",
    });
    assert.equal(refused.status, 78, refused.stderr);
    for (const name of names) {
      assert.deepEqual(
        readFileSync(join(target.secrets, name)),
        before.get(name),
        `${name} changed before target absence was proved`,
      );
    }
    const calls = readFileSync(target.log, "utf8");
    assert.match(calls, /check-compose-assets\.mjs>.*<--state> <absent>/);
    assert.doesNotMatch(calls, / <build>| <up>| <run>/);
  } finally {
    rmSync(target.scratch, { recursive: true, force: true });
    rmSync(source.scratch, { recursive: true, force: true });
  }
});

test("recovery roots are separated and repository-safe after physical canonicalization", () => {
  for (const scenario of ["overlap", "repository"]) {
    const state = fixture();
    const physical = join(state.scratch, "physical-recovery");
    const aliasA = join(state.scratch, "alias-a");
    const aliasB = join(state.scratch, "alias-b");
    const repositoryDirectory = join(
      ROOT,
      `.cpr45-recovery-root-${randomBytes(8).toString("hex")}`,
    );
    try {
      assert.equal(run(state, "up").status, 0);
      writeFileSync(state.log, "");
      privateDirectoryForLifecycleTest(physical);
      privateDirectoryForLifecycleTest(join(physical, "database"));
      privateDirectoryForLifecycleTest(join(physical, "database", "secrets"));
      symlinkSync(physical, aliasA);
      let databaseRoot = join(aliasA, "database");
      let secretRoot;
      if (scenario === "overlap") {
        symlinkSync(physical, aliasB);
        secretRoot = join(aliasB, "database", "secrets");
      } else {
        privateDirectoryForLifecycleTest(repositoryDirectory);
        symlinkSync(ROOT, aliasB);
        secretRoot = join(aliasB, repositoryDirectory.slice(ROOT.length + 1));
      }
      const refused = run(state, "backup", {
        SYNVEDA_DATABASE_BACKUP_ROOT: databaseRoot,
        SYNVEDA_RECOVERY_SECRETS_ROOT: secretRoot,
        SYNVEDA_BACKUP_ID: `canonical-${scenario}`,
      });
      assert.equal(refused.status, 78, `${scenario}: ${refused.stderr}`);
      assert.match(
        refused.stderr,
        scenario === "overlap"
          ? /canonical database and recovery-secret roots must not overlap/
          : /canonical recovery roots inside the repository must use deploy\/compose\/backups/,
      );
      const calls = existsSync(state.log) ? readFileSync(state.log, "utf8") : "";
      assert.doesNotMatch(calls, / <stop>|database-backup/);
    } finally {
      rmSync(repositoryDirectory, { recursive: true, force: true });
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }
});

test("logical recovery fails closed for external PostgreSQL or OIDC", () => {
  for (const action of ["backup", "restore-smoke"]) {
    for (const [name, value] of [
      ["SYNVEDA_POSTGRES_MODE", "external"],
      ["SYNVEDA_OIDC_MODE", "external"],
    ]) {
      const state = fixture();
      try {
        const result = run(state, action, {
          SYNVEDA_COMPOSE_PROFILES:
            action === "restore-smoke" ? "demo,browser-acceptance" : "",
          [name]: value,
        });
        assert.equal(
          result.status,
          69,
          `${action}/${name}: ${result.stderr}`,
        );
        assert.match(
          result.stderr,
          /logical recovery currently requires bundled/,
        );
        assert.equal(existsSync(state.log), false);
      } finally {
        rmSync(state.scratch, { recursive: true, force: true });
      }
    }
  }
});

test("isolated restore verifies recovered state before graph convergence and browser login", () => {
  const source = fixture();
  const target = fixture();
  const roots = recoveryRoots(source);
  const backupId = "lifecycle-restore";
  try {
    assert.equal(run(source, "up").status, 0);
    const backedUp = run(source, "backup", {
      ...roots.environment,
      SYNVEDA_BACKUP_ID: backupId,
    });
    assert.equal(backedUp.status, 0, backedUp.stderr);
    const recoveredSecrets = join(roots.secrets, backupId);
    writeFileSync(target.log, "");
    const restored = run(target, "restore-smoke", {
      ...roots.environment,
      SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance",
      SYNVEDA_RESTORE_SOURCE_PROJECT: source.project,
      SYNVEDA_BACKUP_ID: backupId,
      SYNVEDA_CONFIRM_RESTORE: `${source.project}:${backupId}:${target.project}`,
    });
    assert.equal(restored.status, 0, restored.stderr);
    for (const name of [
      "synveda_kms_key",
      "synveda_kms_key_ref",
      "keycloak_convergence_admin_password",
    ]) {
      assert.equal(
        readFileSync(join(target.secrets, name), "utf8"),
        readFileSync(join(recoveredSecrets, name), "utf8"),
        `${name} was not installed into the restored target`,
      );
    }
    const calls = readFileSync(target.log, "utf8");
    const firstSynvedaBootstrap = calls.indexOf(
      " <up> <--no-build> <--detach> <--wait> <--wait-timeout> <3600> <--force-recreate> <database-bootstrap>",
    );
    const firstKeycloakBootstrap = calls.indexOf(
      " <up> <--no-build> <--detach> <--wait> <--wait-timeout> <3600> <--force-recreate> <keycloak-database-bootstrap>",
      firstSynvedaBootstrap,
    );
    const databaseRestore = calls.indexOf(
      " <run> <--rm> <--no-deps> <database-restore>",
      firstKeycloakBootstrap,
    );
    const secondSynvedaBootstrap = calls.indexOf(
      " <up> <--no-build> <--detach> <--wait> <--wait-timeout> <3600> <--force-recreate> <database-bootstrap>",
      firstSynvedaBootstrap + 1,
    );
    const secondKeycloakBootstrap = calls.indexOf(
      " <up> <--no-build> <--detach> <--wait> <--wait-timeout> <3600> <--force-recreate> <keycloak-database-bootstrap>",
      firstKeycloakBootstrap + 1,
    );
    const correctKey = calls.indexOf(
      " <run> <--rm> <--no-deps> <recovery-verify>",
      secondKeycloakBootstrap,
    );
    const wrongKey = calls.indexOf(
      " <run> <--rm> <--no-deps> <recovery-key-refusal>",
      correctKey,
    );
    const graph = calls.indexOf(
      " <up> <--no-build> <--detach> <--wait> <--wait-timeout> <3600> <--no-recreate> <--scale> <browser-acceptance=0>",
      wrongKey,
    );
    const browser = calls.indexOf(
      " <up> <--no-build> <--detach> <--no-deps> <--force-recreate> <browser-acceptance>",
      graph,
    );
    assert.ok(
      firstSynvedaBootstrap >= 0 && firstKeycloakBootstrap > firstSynvedaBootstrap &&
        databaseRestore > firstKeycloakBootstrap &&
        secondSynvedaBootstrap > databaseRestore &&
        secondKeycloakBootstrap > secondSynvedaBootstrap &&
        correctKey > secondKeycloakBootstrap && wrongKey > correctKey &&
        graph > wrongKey && browser > graph,
      calls,
    );
    assert.match(calls, /compose\.restore\.yaml/);
    const secretSentinels = [
      "synveda_kms_key",
      "keycloak_convergence_admin_password",
    ].map((name) => readFileSync(join(recoveredSecrets, name), "utf8").trim());
    assert.doesNotMatch(
      `${restored.stdout}${restored.stderr}${calls}`,
      new RegExp(secretSentinels.join("|")),
    );
  } finally {
    rmSync(target.scratch, { recursive: true, force: true });
    rmSync(source.scratch, { recursive: true, force: true });
  }
});

test("recovery actions never require or remove development host ownership", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "up").status, 0);
    writeFileSync(state.log, "");
    const down = run(state, "down", { SYNVEDA_FAKE_HOSTS_STATUS: "absent" });
    assert.equal(down.status, 0, down.stderr);
    assert.doesNotMatch(readFileSync(state.log, "utf8"), /manage-hosts-file\.mjs/);
    writeFileSync(state.log, "");
    const reset = run(state, "reset", {
      SYNVEDA_FAKE_HOSTS_STATUS: "absent",
      SYNVEDA_CONFIRM_RESET: state.project,
    });
    assert.equal(reset.status, 0, reset.stderr);
    assert.doesNotMatch(readFileSync(state.log, "utf8"), /manage-hosts-file\.mjs/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("an acceptance project can recover by preserving data and dropping only the browser one-shot", () => {
  const state = fixture();
  const browserProfiles = {
    SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance",
    SYNVEDA_FAKE_VOLUME_MODE: "exact",
  };
  try {
    const started = run(state, "up", browserProfiles, ["--initial-assets", "absent"]);
    assert.equal(started.status, 0, started.stderr);
    writeFileSync(state.log, "");
    const stopped = run(state, "down", browserProfiles);
    assert.equal(stopped.status, 0, stopped.stderr);
    const resumed = run(state, "up", { SYNVEDA_COMPOSE_PROFILES: "demo" });
    assert.equal(resumed.status, 0, resumed.stderr);
    const smoke = run(state, "smoke", { SYNVEDA_COMPOSE_PROFILES: "demo" });
    assert.equal(smoke.status, 0, smoke.stderr);
    const calls = readFileSync(state.log, "utf8");
    const down = calls.indexOf(" <down> <--timeout> <900>");
    const up = calls.indexOf(" <up> <--no-build>", down);
    const runtimeSmoke = calls.lastIndexOf("check-runtime-smoke.mjs");
    assert.ok(down >= 0 && up > down && runtimeSmoke > up, calls);
    assert.match(
      calls,
      new RegExp(`docker <volume> <rm> <${state.project}_browser-acceptance-state>`),
    );
    assert.doesNotMatch(
      calls,
      new RegExp(`docker <volume> <rm> <${state.project}_postgres-data>`),
    );
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("external OIDC owns only the selected development application hostname", () => {
  const state = fixture();
  try {
    const result = run(state, "hosts-status", {
      SYNVEDA_OIDC_MODE: "external",
      SYNVEDA_COMPOSE_PROFILES: "",
      SYNVEDA_APP_HOST: "external-app.synveda.test",
    });
    assert.equal(result.status, 0, result.stderr);
    const calls = readFileSync(state.log, "utf8");
    assert.match(
      calls,
      /manage-hosts-file\.mjs> <status> <--runtime> <development> <--project> <[^>]+> <--oidc> <external> <--app-host> <external-app\.synveda\.test>/,
    );
    assert.doesNotMatch(calls, /--auth-host/);

    const refused = run(state, "hosts-install", {
      SYNVEDA_OIDC_MODE: "external",
      SYNVEDA_COMPOSE_PROFILES: "",
      SYNVEDA_APP_HOST: "external-app.synveda.test",
      auth_host: "ambient.injected.test",
    });
    assert.equal(refused.status, 64, refused.stderr);
    assert.match(
      refused.stderr,
      new RegExp(`install:127\\.0\\.0\\.1:${state.project}:external-app\\.synveda\\.test:-`),
    );
    assert.doesNotMatch(refused.stderr, /ambient\.injected/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("reference mode never reads or mutates managed hosts-file state", () => {
  const state = fixture("reference");
  try {
    const status = run(state, "hosts-status");
    assert.equal(status.status, 0, status.stderr);
    assert.match(status.stdout, /operator DNS and has no managed hosts-file state/);
    assert.equal(existsSync(state.log), false);

    const install = run(state, "hosts-install", {
      SYNVEDA_CONFIRM_HOSTS_INSTALL: "irrelevant",
    });
    assert.equal(install.status, 64, install.stderr);
    assert.match(install.stderr, /reference mode never manages \/etc\/hosts/);
    assert.equal(existsSync(state.log), false);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("host mutation actions require exact configuration-bound confirmation before elevation", () => {
  for (const action of ["hosts-install", "hosts-remove"]) {
    const state = fixture();
    try {
      const refused = run(state, action);
      assert.equal(refused.status, 64, refused.stderr);
      assert.match(
        refused.stderr,
        new RegExp(
          `${action === "hosts-install" ? "install" : "remove"}:127\\.0\\.0\\.1:` +
            `${state.project}:app\\.synveda\\.test:auth\\.synveda\\.test`,
        ),
      );
      assert.equal(existsSync(state.log), false);
    } finally {
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }
});

test("host elevation argv uses only a fixed root-controlled Node candidate and terminal confirmation", () => {
  const source = readFileSync(WRAPPER, "utf8");
  const start = source.indexOf('if [ "$action" = hosts-install ] || [ "$action" = hosts-remove ]; then');
  const end = source.indexOf("run_hosts_ownership_preflight()", start);
  assert.ok(start >= 0 && end > start);
  const elevation = source.slice(start, end);
  const candidateLine = elevation
    .split("\n")
    .find((line) => line.includes("for node_candidate in"));
  assert.equal(candidateLine?.trim(), "for node_candidate in /usr/bin/node /usr/local/bin/node; do");
  assert.match(
    elevation,
    /\/usr\/bin\/node\) node_components='\/ \/usr \/usr\/bin \/usr\/bin\/node'/,
  );
  assert.match(
    elevation,
    /\/usr\/local\/bin\/node\) node_components='\/ \/usr \/usr\/local \/usr\/local\/bin \/usr\/local\/bin\/node'/,
  );
  assert.match(elevation, /\[ ! -L "\$node_candidate" \]/);
  assert.match(elevation, /\/usr\/bin\/find/);
  assert.match(elevation, /\/usr\/bin\/uname/);
  assert.match(elevation, /\/usr\/bin\/awk/);
  assert.match(elevation, /node_component_acl_free\(\)/);
  assert.match(elevation, /\/bin\/ls -lde "\$node_acl_component"/);
  assert.match(elevation, /getfacl/);
  assert.match(elevation, /\/usr\/bin\/env -i LC_ALL=C PATH=\/usr\/bin:\/bin/);
  assert.match(elevation, /!\/\^\(user\|group\|other\)::\[rwx-\]\[rwx-\]\[rwx-\]\$\//);
  assert.match(
    elevation,
    /-type "\$node_component_type" -user root[\s\\]+! -perm -0020 ! -perm -0002 -print/,
  );
  assert.doesNotMatch(elevation, /process\.execPath|\$node_runner/);
  assert.match(
    elevation,
    /set -- \/usr\/bin\/sudo -- \/usr\/bin\/env -i "\$node_executable"[\s\\]+"\$hosts_manager" "\$hosts_mutation"[\s\\]+--runtime development --project "\$project" --oidc "\$oidc_mode"[\s\\]+--app-host "\$app_host"/,
  );
  const authAppend = elevation.indexOf('set -- "$@" --auth-host "$auth_host"');
  const confirmationAppend = elevation.indexOf('set -- "$@" --confirm "$expected_hosts_confirmation"');
  const execution = elevation.indexOf('exec "$@"');
  assert.ok(authAppend >= 0 && confirmationAppend > authAppend && execution > confirmationAppend);
});

test("gateway restart is locked, health-gated, and smoke-checked on both sides", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "up").status, 0);
    writeFileSync(state.log, "");

    const restarted = run(state, "restart-gateway");
    assert.equal(restarted.status, 0, restarted.stderr);
    assert.match(restarted.stdout, /canonical Compose gateway restart passed/);

    const calls = readFileSync(state.log, "utf8");
    const initialAssets = calls.indexOf("<--state> <existing>");
    const preflightAssets = calls.indexOf("<--state> <converged>", initialAssets);
    const preflightResolver = calls.indexOf("check-host-resolution.mjs", preflightAssets + 1);
    const preflightSmoke = calls.indexOf("check-runtime-smoke.mjs");
    const restart = calls.indexOf(" <restart> <--no-deps> <--timeout> <30> <gateway>");
    const healthWait = calls.indexOf(
      " <up> <--no-build> <--detach> <--wait> <--wait-timeout> <120> <--no-deps> <--no-recreate> <gateway>",
    );
    const postflightAssets = calls.indexOf("<--state> <converged>", preflightAssets + 1);
    const postflightResolver = calls.indexOf("check-host-resolution.mjs", postflightAssets + 1);
    const postflightSmoke = calls.indexOf("check-runtime-smoke.mjs", preflightSmoke + 1);
    assert.ok(initialAssets >= 0 && preflightAssets > initialAssets, calls);
    assert.ok(preflightResolver > preflightAssets, calls);
    assert.ok(preflightSmoke > preflightResolver, calls);
    assert.ok(restart > preflightSmoke, calls);
    assert.ok(healthWait > restart, calls);
    assert.ok(postflightAssets > healthWait, calls);
    assert.ok(postflightResolver > postflightAssets, calls);
    assert.ok(postflightSmoke > postflightResolver, calls);
    assert.equal((calls.match(/<ps> <--all> <--quiet> <--no-trunc> <gateway>/g) ?? []).length, 2);
    assert.match(calls, /deadline <45>[\s\S]*<restart>/);
    assert.match(calls, /deadline <125>[\s\S]*<up> <--no-build> <--detach> <--wait>/);
    assert.doesNotMatch(calls, /<restart>[^\n]*<900>/);
    assert.doesNotMatch(calls, /<restart>[^\n]*<(postgres|keycloak|worker|proxy|otel-collector)>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("reference upgrade smoke checks and leaves starting to candidate to starting to candidate", () => {
  const state = fixture("reference");
  prepareReferenceFixture(state);
  const upgrade = upgradeEnvironment();
  try {
    const result = run(state, "upgrade-smoke", upgrade);
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /same-schema product upgrade smoke passed/);
    assert.match(result.stdout, /candidate gateway and worker remain running/);

    const calls = readFileSync(state.log, "utf8");
    assert.equal(
      (calls.match(/<image> <pull>/g) ?? []).length,
      1,
      calls,
    );
    assert.match(
      calls,
      new RegExp(`compatibility-image <${upgrade.SYNVEDA_PRODUCT_IMAGE}>`),
    );
    assert.equal((calls.match(/<migration-check>/g) ?? []).length, 1, calls);
    const transitions = [...calls.matchAll(/^product-image <([^>]+)>$/gm)].map(
      (match) => match[1],
    );
    assert.deepEqual(transitions, [
      upgrade.SYNVEDA_PRODUCT_IMAGE,
      upgrade.SYNVEDA_PRODUCT_STARTING_IMAGE,
      upgrade.SYNVEDA_PRODUCT_IMAGE,
    ]);
    assert.equal((calls.match(/<product-demo\.mjs> <verify>/g) ?? []).length, 4, calls);
    assert.equal(
      (calls.match(/<--force-recreate> <browser-acceptance>/g) ?? []).length,
      4,
      calls,
    );
    assert.equal(
      (calls.match(
        /<up> <--no-build> <--pull> <never> <--detach> <--wait> <--wait-timeout> <3600> <--no-deps> <--force-recreate> <gateway> <worker>/g,
      ) ?? []).length,
      3,
      calls,
    );
    assert.doesNotMatch(calls, / <restart>| <down>|<volume> <rm>/);
    assert.doesNotMatch(
      calls,
      /<--force-recreate> <(?:postgres|keycloak|otel-collector|proxy|migrate|tenant-convergence)>/,
    );
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("upgrade compatibility refusal leaves the starting product and releases the lock", () => {
  const state = fixture("reference");
  prepareReferenceFixture(state);
  const upgrade = upgradeEnvironment();
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  try {
    const refused = run(state, "upgrade-smoke", {
      ...upgrade,
      SYNVEDA_FAKE_UPGRADE_COMPATIBILITY_FAIL: "1",
    });
    assert.equal(refused.status, 50, refused.stderr);
    assert.equal(existsSync(lockFile), false);
    const calls = readFileSync(state.log, "utf8");
    assert.equal((calls.match(/compatibility-image/g) ?? []).length, 1, calls);
    assert.doesNotMatch(calls, /^product-image/m);
    assert.doesNotMatch(calls, /<force-recreate> <gateway> <worker>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("upgrade smoke refuses two digest references resolving to one image before transition", () => {
  const state = fixture("reference");
  prepareReferenceFixture(state);
  const upgrade = upgradeEnvironment();
  try {
    const refused = run(state, "upgrade-smoke", {
      ...upgrade,
      SYNVEDA_FAKE_SAME_PRODUCT_IMAGE_ID: "1",
    });
    assert.equal(refused.status, 78, refused.stderr);
    assert.match(refused.stderr, /resolve to one image/);
    const calls = readFileSync(state.log, "utf8");
    assert.doesNotMatch(calls, /<migration-check>|^product-image/m);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("a failed rollback transition restores the last verified candidate", () => {
  const state = fixture("reference");
  prepareReferenceFixture(state);
  const upgrade = upgradeEnvironment();
  try {
    const failed = run(state, "upgrade-smoke", {
      ...upgrade,
      SYNVEDA_FAKE_UPGRADE_UP_FAIL_AT: "2",
    });
    assert.equal(failed.status, 51, failed.stderr);
    assert.match(failed.stderr, /restored the last verified product image/);
    const transitions = [
      ...readFileSync(state.log, "utf8").matchAll(/^product-image <([^>]+)>$/gm),
    ].map((match) => match[1]);
    assert.deepEqual(transitions, [
      upgrade.SYNVEDA_PRODUCT_IMAGE,
      upgrade.SYNVEDA_PRODUCT_STARTING_IMAGE,
      upgrade.SYNVEDA_PRODUCT_IMAGE,
    ]);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("failed upgrade recovery retains the exact project lock", () => {
  const state = fixture("reference");
  prepareReferenceFixture(state);
  const upgrade = upgradeEnvironment();
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  let marker;
  try {
    const failed = run(state, "upgrade-smoke", {
      ...upgrade,
      SYNVEDA_FAKE_UPGRADE_UP_FAIL_FROM: "1",
    });
    assert.equal(failed.status, 51, failed.stderr);
    assert.match(failed.stderr, /recovery to the last verified product image failed/);
    assert.match(failed.stderr, /Docker mutation state is uncertain/);
    marker = readFileSync(lockFile, "utf8");
    assert.equal(marker, `${state.project}:${failed.pid}\n`);
  } finally {
    if (marker !== undefined && existsSync(lockFile) && readFileSync(lockFile, "utf8") === marker) {
      rmSync(lockFile);
    }
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("gateway restart refuses an exhausted postflight budget before mutation", () => {
  const state = fixture();
  const clock = join(state.scratch, "clock");
  writeFileSync(clock, "1000\n", { mode: 0o600 });
  try {
    assert.equal(run(state, "up").status, 0);
    writeFileSync(state.log, "");
    const refused = run(state, "restart-gateway", {
      SYNVEDA_COMPOSE_LIFECYCLE_TIMEOUT_SECONDS: "240",
      SYNVEDA_FAKE_CLOCK_FILE: clock,
      SYNVEDA_FAKE_CLOCK_STEP: "10",
    });
    assert.equal(refused.status, 124, refused.stderr);
    assert.match(refused.stderr, /insufficient lifecycle budget remains/);
    assert.doesNotMatch(readFileSync(state.log, "utf8"), / <restart>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("gateway restart refuses a missing or replaced exact container identity", () => {
  for (const mode of ["missing", "post-missing", "replaced"]) {
    const state = fixture();
    const lockFile = join(
      "/tmp",
      `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
      `${state.project}.lock`,
    );
    let marker;
    try {
      assert.equal(run(state, "up").status, 0);
      writeFileSync(state.log, "");
      const refused = run(state, "restart-gateway", {
        SYNVEDA_FAKE_GATEWAY_ID_MODE: mode,
      });
      assert.equal(refused.status, 78, `${mode}: ${refused.stderr}`);
      assert.match(refused.stderr, /gateway container identity/);
      const calls = readFileSync(state.log, "utf8");
      if (mode === "missing") {
        assert.doesNotMatch(calls, / <restart>/);
        assert.equal(existsSync(lockFile), false);
      } else {
        assert.match(calls, / <restart>/);
        marker = readFileSync(lockFile, "utf8");
        assert.equal(marker, `${state.project}:${refused.pid}\n`);
      }
    } finally {
      if (marker !== undefined && existsSync(lockFile) && readFileSync(lockFile, "utf8") === marker) {
        rmSync(lockFile);
      }
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }
});

test("an uncertain gateway restart retains the exact project lock", () => {
  const state = fixture();
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  let marker;
  try {
    assert.equal(run(state, "up").status, 0);
    writeFileSync(state.log, "");

    const failed = run(state, "restart-gateway", { SYNVEDA_FAKE_RESTART_FAIL: "1" });
    assert.equal(failed.status, 42, failed.stderr);
    assert.match(failed.stderr, /Docker mutation state is uncertain \(compose-restart-gateway\)/);
    marker = readFileSync(lockFile, "utf8");
    assert.equal(marker, `${state.project}:${failed.pid}\n`);
    assert.match(readFileSync(state.log, "utf8"), / <restart> <--no-deps>/);

    const blocked = run(state, "restart-gateway");
    assert.equal(blocked.status, 75, blocked.stderr);
    assert.doesNotMatch(readFileSync(state.log, "utf8"), /--no-recreate/);
  } finally {
    if (marker !== undefined && existsSync(lockFile) && readFileSync(lockFile, "utf8") === marker) {
      rmSync(lockFile);
    }
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("Compose acceptance holds one project lock across the fixed restart matrix", () => {
  const state = fixture();
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  try {
    const accepted = run(state, "acceptance", {
      SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance",
    });
    assert.equal(accepted.status, 0, accepted.stderr);
    assert.match(accepted.stdout, /canonical Compose acceptance passed/);
    assert.match(accepted.stdout, /services remain running/);
    assert.equal(existsSync(lockFile), false);

    const calls = readFileSync(state.log, "utf8");
    const restarts = [
      " <restart> <--no-deps> <--timeout> <60> <postgres>",
      " <restart> <--no-deps> <--timeout> <45> <keycloak>",
      " <restart> <--no-deps> <--timeout> <15> <otel-collector>",
      " <restart> <--no-deps> <--timeout> <85> <worker>",
      " <restart> <--no-deps> <--timeout> <30> <gateway>",
      " <restart> <--no-deps> <--timeout> <15> <proxy>",
    ];
    let previous = -1;
    for (const restart of restarts) {
      const position = calls.indexOf(restart);
      assert.ok(position > previous, calls);
      previous = position;
    }
    const productSeed =
      "<run> <--rm> <--no-deps> <--no-TTY> <--entrypoint> <node> " +
      "<browser-acceptance> <product-demo.mjs> <seed>";
    const productVerify =
      "<run> <--rm> <--no-deps> <--no-TTY> <--entrypoint> <node> " +
      "<browser-acceptance> <product-demo.mjs> <verify>";
    assert.equal((calls.match(/<product-demo\.mjs>/g) ?? []).length, 2, calls);
    assert.ok(calls.indexOf(productSeed) >= 0, calls);
    assert.ok(calls.indexOf(productSeed) < calls.indexOf(restarts[0]), calls);
    assert.ok(calls.indexOf(productVerify) > previous, calls);
    assert.doesNotMatch(
      calls,
      / <restart>[^\n]*<(?:database-bootstrap|database-preflight|keycloak-database-bootstrap|keycloak-realm-convergence|migrate|tenant-convergence|issuer-diagnostic|browser-acceptance)>/,
    );
    assert.match(
      calls,
      /<--no-recreate> <postgres> <keycloak> <keycloak-realm-convergence> <otel-collector> <worker> <gateway> <proxy>/,
    );
    assert.match(
      calls,
      /<--no-recreate> <keycloak> <keycloak-realm-convergence> <otel-collector> <worker> <gateway> <proxy>/,
    );
    assert.equal((calls.match(/check-runtime-smoke\.mjs/g) ?? []).length, 8);
    assert.equal(
      (calls.match(/<up> <--no-build> <--detach> <--no-deps> <--force-recreate> <browser-acceptance>/g) ?? []).length,
      2,
    );
    assert.equal((calls.match(/docker <container> <wait>/g) ?? []).length, 2);
    for (const service of ["postgres", "keycloak", "otel-collector", "worker", "gateway", "proxy"]) {
      const identity = new RegExp(
        `<ps> <--all> <--quiet> <--no-trunc> <${service}>`,
        "g",
      );
      assert.equal((calls.match(identity) ?? []).length, 2, service);
    }
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("Compose acceptance executes the Apalis canary and restarts its worker", () => {
  const state = fixture();
  try {
    const accepted = run(state, "acceptance", {
      SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance,apalis",
    });
    assert.equal(accepted.status, 0, accepted.stderr);
    assert.match(accepted.stdout, /canonical Compose acceptance passed/);

    const calls = readFileSync(state.log, "utf8");
    assert.match(calls, /compose\.apalis\.yaml>/);
    assert.match(calls, /compose\.apalis\.dev\.yaml>/);
    const productSeed =
      "<run> <--rm> <--no-deps> <--no-TTY> <--entrypoint> <node> " +
      "<browser-acceptance> <product-demo.mjs> <seed>";
    const apalisRestart =
      " <restart> <--no-deps> <--timeout> <35> <apalis-worker>";
    const productVerify =
      "<run> <--rm> <--no-deps> <--no-TTY> <--entrypoint> <node> " +
      "<browser-acceptance> <product-demo.mjs> <verify>";
    const seedPosition = calls.indexOf(productSeed);
    const restartPosition = calls.indexOf(apalisRestart);
    const verifyPosition = calls.indexOf(productVerify);
    assert.ok(
      seedPosition >= 0 && restartPosition > seedPosition && verifyPosition > restartPosition,
      calls,
    );
    assert.equal((calls.match(/<product-demo\.mjs>/g) ?? []).length, 2, calls);
    assert.equal((calls.match(/<--apalis> <true>/g) ?? []).length, 9, calls);
    assert.equal((calls.match(/ <restart>[^\n]*<apalis-worker>/g) ?? []).length, 1, calls);
    assert.match(
      calls,
      /<--no-recreate> <postgres> <keycloak> <keycloak-realm-convergence> <otel-collector> <worker> <gateway> <proxy> <apalis-worker>/,
    );
    assert.equal(
      (calls.match(/<ps> <--all> <--quiet> <--no-trunc> <apalis-worker>/g) ?? []).length,
      2,
      calls,
    );
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("a known product seed refusal stops before the fixed restart checks", () => {
  const state = fixture();
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  try {
    const refused = run(state, "acceptance", {
      SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance",
      SYNVEDA_FAKE_PRODUCT_SEED_FAIL: "1",
    });
    assert.equal(refused.status, 48, refused.stderr);
    assert.equal(existsSync(lockFile), false);
    const calls = readFileSync(state.log, "utf8");
    assert.match(calls, /<product-demo\.mjs>/);
    assert.equal((calls.match(/ <restart>/g) ?? []).length, 0, calls);
    assert.doesNotMatch(calls, /product-demo\.mjs> <(?:start|status)>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("a post-restart product verification refusal is propagated", () => {
  const state = fixture();
  try {
    const refused = run(state, "acceptance", {
      SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance",
      SYNVEDA_FAKE_PRODUCT_VERIFY_FAIL: "1",
    });
    assert.equal(refused.status, 49, refused.stderr);
    const calls = readFileSync(state.log, "utf8");
    assert.equal((calls.match(/ <restart>/g) ?? []).length, 6, calls);
    assert.match(calls, /<product-demo\.mjs> <seed>/);
    assert.match(calls, /<product-demo\.mjs> <verify>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("Compose acceptance refuses a non-browser or non-fresh invocation before Docker", () => {
  for (const extra of [
    { SYNVEDA_COMPOSE_PROFILES: "demo" },
    {
      SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance",
      SYNVEDA_COMPOSE_PROJECT_SUFFIX: "",
    },
  ]) {
    const state = fixture();
    try {
      const refused = run(state, "acceptance", extra);
      assert.equal(refused.status, 64, refused.stderr);
      assert.equal(existsSync(state.log), false);
    } finally {
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }
});

test("Compose acceptance stops on replaced restart identity and retains the lock", () => {
  const state = fixture();
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  let marker;
  try {
    const refused = run(state, "acceptance", {
      SYNVEDA_COMPOSE_PROFILES: "demo,browser-acceptance",
      SYNVEDA_FAKE_RESTART_ID_SERVICE: "worker",
      SYNVEDA_FAKE_RESTART_ID_MODE: "replaced",
    });
    assert.equal(refused.status, 78, refused.stderr);
    assert.match(refused.stderr, /worker container identity changed during restart/);
    const calls = readFileSync(state.log, "utf8");
    assert.match(calls, / <restart>[^\n]*<worker>/);
    assert.doesNotMatch(calls, / <restart>[^\n]*<(?:gateway|proxy)>/);
    marker = readFileSync(lockFile, "utf8");
    assert.equal(marker, `${state.project}:${refused.pid}\n`);
  } finally {
    if (marker !== undefined && existsSync(lockFile) && readFileSync(lockFile, "utf8") === marker) {
      rmSync(lockFile);
    }
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("an interrupted Docker mutation retains its exact lock through signal-reentrant cleanup", async () => {
  const state = fixture();
  const entered = join(state.scratch, "up-entered");
  const release = join(state.scratch, "up-release");
  const cleanupEntered = join(state.scratch, "cleanup-entered");
  const cleanupRelease = join(state.scratch, "cleanup-release");
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  let expectedOwnerMarker;
  const blockingEnvironment = environment(state, {
    SYNVEDA_FAKE_BLOCK_UP: "1",
    SYNVEDA_FAKE_UP_ENTERED: entered,
    SYNVEDA_FAKE_UP_RELEASE: release,
    SYNVEDA_FAKE_BLOCK_ASSET_CLEANUP: "1",
    SYNVEDA_FAKE_CLEANUP_ENTERED: cleanupEntered,
    SYNVEDA_FAKE_CLEANUP_RELEASE: cleanupRelease,
  });
  async function waitFor(path) {
    const deadline = Date.now() + 8_000;
    while (!existsSync(path)) {
      assert.ok(Date.now() < deadline, `timed out waiting for ${path}`);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
  }
  try {
    const first = spawn(WRAPPER, ["up"], {
      cwd: ROOT,
      env: blockingEnvironment,
      detached: true,
      stdio: ["ignore", "pipe", "pipe"],
    });
    expectedOwnerMarker = `${state.project}:${first.pid}\n`;
    let firstError = "";
    first.stderr.setEncoding("utf8");
    first.stderr.on("data", (chunk) => { firstError += chunk; });
    await waitFor(entered);
    const concurrent = run(state, "up");
    assert.equal(concurrent.status, 75, concurrent.stderr);
    assert.match(concurrent.stderr, /another lifecycle or authority action owns/);

    process.kill(first.pid, "SIGTERM");
    await waitFor(cleanupEntered);
    // Cleanup ignores re-entrant lifecycle signals while removing only its
    // private staging file. Mutation uncertainty deliberately retains the lock.
    process.kill(first.pid, "SIGHUP");
    writeFileSync(cleanupRelease, "");
    const [signalCode, signal] = await once(first, "close");
    assert.equal(signal, null);
    assert.equal(signalCode, 143, firstError);
    assert.equal(existsSync(release), false);
    assert.match(firstError, /retained exact-project lock because Docker mutation state is uncertain/);
    assert.equal(existsSync(lockFile), true);
    assert.equal(readFileSync(lockFile, "utf8"), expectedOwnerMarker);

    const blocked = run(state, "up");
    assert.equal(blocked.status, 75, blocked.stderr);
    assert.match(blocked.stderr, /another lifecycle or authority action owns/);

    // This unique fixture owns the exact marker, and the spawning process has
    // closed, so the test may perform the documented operator recovery step.
    rmSync(lockFile);
    expectedOwnerMarker = undefined;
    const resumed = run(state, "up");
    assert.equal(resumed.status, 0, resumed.stderr);
  } finally {
    if (
      expectedOwnerMarker !== undefined && existsSync(lockFile) &&
      readFileSync(lockFile, "utf8") === expectedOwnerMarker
    ) {
      rmSync(lockFile);
    }
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("one elapsed lifecycle budget bounds all sequential preparation phases", () => {
  const state = fixture();
  const clock = join(state.scratch, "clock");
  writeFileSync(clock, "1000\n", { mode: 0o600 });
  try {
    const result = run(state, "up", {
      SYNVEDA_COMPOSE_LIFECYCLE_TIMEOUT_SECONDS: "240",
      SYNVEDA_FAKE_CLOCK_FILE: clock,
      SYNVEDA_FAKE_CLOCK_STEP: "40",
    });
    assert.equal(result.status, 124, result.stderr);
    assert.match(result.stderr, /whole-operation lifecycle deadline expired/);
    const calls = readFileSync(state.log, "utf8");
    const budgets = [...calls.matchAll(/deadline <([0-9]+)>/g)].map((match) =>
      Number(match[1]),
    );
    assert.deepEqual(budgets, [200, 160, 120, 80, 40]);
    assert.match(calls, /check-host-resolution\.mjs/);
    assert.match(calls, /check-network-preflight\.mjs/);
    assert.doesNotMatch(calls, / <up>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("bounded-child uncertainty before Docker mutation retains the exact project lock", () => {
  const state = fixture();
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  let marker;
  try {
    const uncertain = run(state, "up", { SYNVEDA_FAKE_RUNNER_125: "1" });
    assert.equal(uncertain.status, 125, uncertain.stderr);
    assert.match(uncertain.stderr, /bounded child process group was not cleanly reaped/);
    marker = readFileSync(lockFile, "utf8");
    assert.equal(marker, `${state.project}:${uncertain.pid}\n`);

    const blocked = run(state, "up");
    assert.equal(blocked.status, 75, blocked.stderr);
    assert.match(blocked.stderr, /another lifecycle or authority action owns/);
  } finally {
    if (marker !== undefined && existsSync(lockFile) && readFileSync(lockFile, "utf8") === marker) {
      rmSync(lockFile);
    }
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("an uncatchably killed deadline runner cannot orphan an authority writer past lock release", async () => {
  const state = fixture();
  const runnerPidFile = join(state.scratch, "runner.pid");
  const childPidFile = join(state.scratch, "authority-child.pid");
  const entered = join(state.scratch, "openssl-entered");
  const release = join(state.scratch, "openssl-release");
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  let childPid;
  let marker;
  async function waitForFile(path) {
    const deadline = Date.now() + 8_000;
    while (!existsSync(path)) {
      assert.ok(Date.now() < deadline, `timed out waiting for ${path}`);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
  }
  async function waitForExit(pid) {
    const deadline = Date.now() + 8_000;
    while (true) {
      try {
        process.kill(pid, 0);
      } catch (error) {
        if (error.code === "ESRCH") return;
        throw error;
      }
      assert.ok(Date.now() < deadline, `timed out waiting for process ${pid}`);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
  }
  try {
    const wrapper = spawn(WRAPPER, ["up"], {
      cwd: ROOT,
      env: environment(state, {
        SYNVEDA_FAKE_RUNNER_PID_FILE: runnerPidFile,
        SYNVEDA_FAKE_AUTHORITY_CHILD_PID_FILE: childPidFile,
        SYNVEDA_FAKE_BLOCK_OPENSSL: "1",
        SYNVEDA_FAKE_OPENSSL_ENTERED: entered,
        SYNVEDA_FAKE_OPENSSL_RELEASE: release,
      }),
      stdio: ["ignore", "pipe", "pipe"],
    });
    let errorOutput = "";
    wrapper.stderr.setEncoding("utf8");
    wrapper.stderr.on("data", (chunk) => { errorOutput += chunk; });
    wrapper.stdout.resume();
    await waitForFile(entered);
    const runnerPid = Number(readFileSync(runnerPidFile, "utf8").trim());
    childPid = Number(readFileSync(childPidFile, "utf8").trim());
    assert.ok(Number.isSafeInteger(runnerPid) && runnerPid > 1);
    assert.ok(Number.isSafeInteger(childPid) && childPid > 1);
    process.kill(runnerPid, "SIGKILL");
    const [wrapperStatus, wrapperSignal] = await once(wrapper, "exit");
    assert.equal(wrapperSignal, null);
    assert.equal(wrapperStatus, 137, errorOutput);
    assert.match(errorOutput, /bounded child process group was not cleanly reaped/);
    marker = readFileSync(lockFile, "utf8");
    assert.equal(marker, `${state.project}:${wrapper.pid}\n`);

    const blocked = run(state, "up");
    assert.equal(blocked.status, 75, blocked.stderr);
    writeFileSync(release, "continue\n", { mode: 0o600 });
    await waitForExit(childPid);
    childPid = undefined;

    assert.equal(readFileSync(lockFile, "utf8"), marker);
    rmSync(lockFile);
    marker = undefined;
    const resumed = run(state, "up");
    assert.equal(resumed.status, 0, resumed.stderr);
  } finally {
    writeFileSync(release, "continue\n", { mode: 0o600 });
    if (Number.isSafeInteger(childPid) && childPid > 1) {
      try {
        process.kill(-childPid, "SIGKILL");
      } catch {
        // The unique authority process group normally exits after release.
      }
    }
    if (marker !== undefined && existsSync(lockFile) && readFileSync(lockFile, "utf8") === marker) {
      rmSync(lockFile);
    }
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("an uncatchably killed authority writer retains its exact project lock", async () => {
  const state = fixture();
  const childPidFile = join(state.scratch, "authority-child.pid");
  const entered = join(state.scratch, "authority-entered");
  const release = join(state.scratch, "authority-release");
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  let marker;
  try {
    const wrapper = spawn(WRAPPER, ["up"], {
      cwd: ROOT,
      env: environment(state, {
        SYNVEDA_FAKE_AUTHORITY_CHILD_PID_FILE: childPidFile,
        SYNVEDA_FAKE_BLOCK_AUTHORITY_ENV: "1",
        SYNVEDA_FAKE_AUTHORITY_ENV_ENTERED: entered,
        SYNVEDA_FAKE_AUTHORITY_ENV_RELEASE: release,
      }),
      stdio: ["ignore", "pipe", "pipe"],
    });
    let errorOutput = "";
    wrapper.stderr.setEncoding("utf8");
    wrapper.stderr.on("data", (chunk) => { errorOutput += chunk; });
    wrapper.stdout.resume();
    const deadline = Date.now() + 8_000;
    while (!existsSync(entered)) {
      assert.ok(Date.now() < deadline, `timed out waiting for ${entered}`);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
    const childPid = Number(readFileSync(childPidFile, "utf8").trim());
    assert.ok(Number.isSafeInteger(childPid) && childPid > 1);
    process.kill(childPid, "SIGKILL");
    const [wrapperStatus, wrapperSignal] = await once(wrapper, "exit");
    assert.equal(wrapperSignal, null);
    assert.equal(wrapperStatus, 137, errorOutput);
    assert.match(errorOutput, /bounded child process group was not cleanly reaped/);
    marker = readFileSync(lockFile, "utf8");
    assert.equal(marker, `${state.project}:${wrapper.pid}\n`);

    const blocked = run(state, "up");
    assert.equal(blocked.status, 75, blocked.stderr);
    assert.match(blocked.stderr, /another lifecycle or authority action owns/);
  } finally {
    writeFileSync(release, "continue\n", { mode: 0o600 });
    if (marker !== undefined && existsSync(lockFile) && readFileSync(lockFile, "utf8") === marker) {
      rmSync(lockFile);
    }
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("a catchable parent signal retains the lock when authority cleanup needs SIGKILL", async () => {
  const state = fixture();
  const childPidFile = join(state.scratch, "authority-child.pid");
  const entered = join(state.scratch, "authority-entered");
  const release = join(state.scratch, "authority-release");
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  let marker;
  try {
    const wrapper = spawn(WRAPPER, ["up"], {
      cwd: ROOT,
      env: environment(state, {
        SYNVEDA_FAKE_AUTHORITY_CHILD_PID_FILE: childPidFile,
        SYNVEDA_FAKE_BLOCK_AUTHORITY_ENV: "1",
        SYNVEDA_FAKE_AUTHORITY_IGNORE_SIGNALS: "1",
        SYNVEDA_FAKE_AUTHORITY_ENV_ENTERED: entered,
        SYNVEDA_FAKE_AUTHORITY_ENV_RELEASE: release,
      }),
      stdio: ["ignore", "pipe", "pipe"],
    });
    let errorOutput = "";
    wrapper.stderr.setEncoding("utf8");
    wrapper.stderr.on("data", (chunk) => { errorOutput += chunk; });
    wrapper.stdout.resume();
    const deadline = Date.now() + 8_000;
    while (!existsSync(entered)) {
      assert.ok(Date.now() < deadline, `timed out waiting for ${entered}`);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
    process.kill(wrapper.pid, "SIGTERM");
    const [wrapperStatus, wrapperSignal] = await once(wrapper, "exit");
    assert.equal(wrapperSignal, null);
    assert.equal(wrapperStatus, 143, errorOutput);
    assert.match(errorOutput, /forced stop bypassed command cleanup/);
    assert.match(errorOutput, /bounded child process group was not cleanly reaped/);
    marker = readFileSync(lockFile, "utf8");
    assert.equal(marker, `${state.project}:${wrapper.pid}\n`);

    const blocked = run(state, "up");
    assert.equal(blocked.status, 75, blocked.stderr);
    assert.match(blocked.stderr, /another lifecycle or authority action owns/);
  } finally {
    writeFileSync(release, "continue\n", { mode: 0o600 });
    if (marker !== undefined && existsSync(lockFile) && readFileSync(lockFile, "utf8") === marker) {
      rmSync(lockFile);
    }
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("runner launch publishes pending uncertainty before its PID", () => {
  const source = readFileSync(WRAPPER, "utf8");
  const pending = source.indexOf("bounded_runner_pending=true");
  const launch = source.indexOf(
    '"$node_runner" "$script_dir/run-with-deadline.mjs"',
    pending,
  );
  const pid = source.indexOf("bounded_runner_pid=$!", launch);
  const clear = source.indexOf("bounded_runner_pending=false", pid);
  const handler = source.indexOf('elif [ "$bounded_runner_pending" = true ]');
  const retain = source.indexOf("lifecycle_child_uncertain=true", handler);
  assert.ok(pending >= 0 && launch > pending && pid > launch && clear > pid);
  assert.ok(handler >= 0 && retain > handler);
});

test("runner completion witness is settled before PID state is cleared", () => {
  const source = readFileSync(WRAPPER, "utf8");
  const staged = source.indexOf("bounded_status_file=$(mktemp");
  const launch = source.indexOf("--status-file \"$bounded_status_file\"", staged);
  const waiting = source.indexOf("bounded_runner_waiting=true", launch);
  const wait = source.indexOf('wait "$bounded_runner_pid" || bounded_status=$?', waiting);
  const settlement = source.indexOf('settle_bounded_runner "$bounded_status"', wait);
  const killed = source.indexOf('if [ "$bounded_status" -ge 128 ]', settlement);
  const settled = source.indexOf("bounded_runner_waiting=false", killed);
  const cleared = source.indexOf("bounded_runner_pid=", settled);
  assert.ok(staged >= 0 && launch > staged && waiting > launch && wait > waiting);
  assert.ok(settlement > wait && killed > settlement && settled > killed);
  assert.ok(cleared > settled);
  assert.match(source, /recorded_bounded_status" = "clean:\$settled_status"/);
  assert.match(
    source,
    /if \[ "\$settled_status" -eq 125 \] \|\| \[ "\$bounded_group_clean" != true \]; then\n\s+lifecycle_child_uncertain=true/,
  );
  assert.match(
    source,
    /wait "\$bounded_runner_pid" 2>\/dev\/null \|\| signal_wait_status=\$\?[\s\S]*settle_bounded_runner "\$signal_wait_status"/,
  );
});

test("the project lock publishes and owns one verified hard-link inode", () => {
  const source = readFileSync(PROJECT_LOCK, "utf8");
  const link = source.indexOf('ln "$project_lock_claim_file" "$project_lock_file"');
  const verify = source.indexOf(
    '"$project_lock_claim_identity" ]; then',
    link,
  );
  const identity = source.indexOf("project_lock_identity=$project_lock_claim_identity", verify);
  const owned = source.indexOf("project_lock_owned=true", identity);
  assert.ok(link >= 0 && verify > link && identity > verify && owned > identity);
  assert.doesNotMatch(source, /mkdir[^\n]*\$project_lock_file/);
});

test("down refuses a remote Docker endpoint before mutation", () => {
  const state = fixture();
  try {
    const prepared = run(state, "up");
    assert.equal(prepared.status, 0, prepared.stderr);
    writeFileSync(state.log, "");
    const refused = run(state, "down", {
      DOCKER_HOST: "tcp://remote.example:2376",
      SYNVEDA_FAKE_REAL_HOST_CHECK: "1",
    });
    assert.equal(refused.status, 69, refused.stderr);
    assert.match(refused.stderr, /local Docker endpoint is required/);
    const calls = readFileSync(state.log, "utf8");
    assert.doesNotMatch(calls, / <down>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("the validated local Docker socket is pinned across later lifecycle calls", () => {
  const state = fixture();
  try {
    const endpoint = `unix://${join(state.scratch, "docker.sock")}`;
    const result = run(state, "up", {
      DOCKER_CONTEXT: "context-that-may-change",
      SYNVEDA_FAKE_LOCAL_DOCKER_ENDPOINT: endpoint,
      SYNVEDA_FAKE_REQUIRE_PINNED_DOCKER: "1",
    });
    assert.equal(result.status, 0, result.stderr);
    assert.match(readFileSync(state.log, "utf8"), / <up>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("an abandoned exact-project lock fails closed", () => {
  const state = fixture();
  const lockRoot = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
  );
  const lockFile = join(lockRoot, `${state.project}.lock`);
  const ownerMarker = `${state.project}:999999\n`;
  let fixtureLockCreated = false;
  try {
    mkdirSync(lockRoot, { recursive: true, mode: 0o700 });
    writeFileSync(lockFile, ownerMarker, { mode: 0o600 });
    fixtureLockCreated = true;
    const refused = run(state, "up");
    assert.equal(refused.status, 75, refused.stderr);
    assert.match(refused.stderr, /another lifecycle or authority action owns/);
    assert.equal(existsSync(lockFile), true);
  } finally {
    if (
      fixtureLockCreated && existsSync(lockFile) &&
      readFileSync(lockFile, "utf8") === ownerMarker
    ) {
      rmSync(lockFile);
    }
    rmSync(state.scratch, { recursive: true, force: true });
  }
});
