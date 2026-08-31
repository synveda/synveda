import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chownSync,
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  realpathSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { generateTestTlsChain } from "./test-certificate.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const COMPOSE = join(ROOT, "deploy/compose");
const WRAPPER = join(COMPOSE, "scripts/compose.sh");
const DIGEST = `sha256:${"1".repeat(64)}`;
const SECRET_SENTINEL = "cpr45-secret-sentinel";
const KEYCLOAK_SECURITY_CHAIN_SHA256 = new Map([
  [
    "keycloak/Dockerfile",
    "b480f34ec0e1ae4c39202cab9947bd7af8cda43b5f7ffbb0f8986d52c4150ed6",
  ],
  [
    "keycloak/keycloak-entrypoint",
    "80e78357463d4a861e559e2b4e62488ff9c77eac6ee9504203e6aa03137c8023",
  ],
  [
    "keycloak/SynvedaKeycloakProjection.java",
    "9887d8596a845c54feaf6c2d2ab9aa1df4705963b6cb26bed057795538f16eac",
  ],
  [
    "keycloak/SynvedaKeycloakProjectionSelfTest.java",
    "e8f791897c92636610cd4d18e8a8a7bdc934cc32b0e4dc84c80411fa9d9d9608",
  ],
  [
    "keycloak/synveda-projection-self-test",
    "1c8558d91323b44862bc9cc2b2ef989c6a1c477ff8e19714a6e0d7d132f421e7",
  ],
  [
    "keycloak/synveda-authority-stage",
    "2edb3693873ace9417e1d6bd0f1f636221c5c508f06c043b91e73fb3f0e3c320",
  ],
  [
    "keycloak/synveda-authority-stage-self-test",
    "8cfba1b70515b7b7f9c91fb70c328ecfebe493f3d85b92d6866b86d4f5e59cf6",
  ],
  [
    "keycloak/synveda-audience-mapper.json",
    "98daa726e9a149df5e9d21094f8ad4d9a8cc97ef41eff79f54ce97ee63f149cc",
  ],
  [
    "keycloak/synveda-groups-mapper.json",
    "c8c940bc4b7096ed62da1613b6c94b2f34f568b3ac42da21cc75ca606ea2ee8a",
  ],
  [
    "keycloak/synveda-user-profile.json",
    "7f38f5f3e142ac0a8ca5d0a3c03cc5b97f0849079f5a3f3f99065fb40455933f",
  ],
  [
    "keycloak/synveda-realm-converge",
    "d2d676b35a5142694b333e5b9af2801f93442c9fa29fe05f694e15e818a3f8ef",
  ],
  [
    "keycloak/synveda-generation-gate",
    "a22e490ab24c61fb193da663f26ff6fc1c7d7b1f02b6e6e4947ce36eb42a65b4",
  ],
  [
    "keycloak/synveda-generation-gate-self-test",
    "3fa0655639ffa5408bf55d9242984a94b209add2829c8105c58f55aae7fe7b63",
  ],
  [
    "keycloak/synveda-realm-supervise",
    "95be5bf83220687724471ffa7253dda310ef346b3468d2e89b36c7cd175e4013",
  ],
  [
    "keycloak/synveda-keycloak-health",
    "ddf3948d01469a65d31513f2f84db20a90c9dc999c46705989af8d17ea8cdb88",
  ],
  [
    "postgres/synveda-input-snapshot.c",
    "1897a1324e50451bd8eab6fd6a6c2aad4f95df660266c6dca66fd32daefd1560",
  ],
]);
const COLLECTOR_HEALTH_BODY = JSON.stringify({
  receivers: { nop: {} },
  exporters: { nop: {} },
  service: { pipelines: { traces: { receivers: ["nop"], exporters: ["nop"] } } },
});

const CORE_SECRETS = [
  "synveda_migrator_database_url",
  "synveda_gateway_database_url",
  "synveda_worker_database_url",
  "synveda_kms_key",
  "synveda_kms_key_ref",
];
const PROVIDER_SECRETS = [
  "postgres_owner_password",
  "synveda_migrator_password",
  "synveda_gateway_password",
  "synveda_worker_password",
  "keycloak_database_password",
  "keycloak_admin_username",
  "keycloak_admin_password",
  "keycloak_convergence_admin_password",
  "keycloak_demo_admin_password",
  "keycloak_demo_member_password",
];
const CONTAINER_PROXY_ENVIRONMENT = Object.freeze([
  "HTTP_PROXY",
  "http_proxy",
  "HTTPS_PROXY",
  "https_proxy",
  "NO_PROXY",
  "no_proxy",
  "FTP_PROXY",
  "ftp_proxy",
  "ALL_PROXY",
  "all_proxy",
]);
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

function writePrivate(path, value, owner) {
  writeFileSync(path, `${value}\n`, { mode: 0o600 });
  chmodSync(path, 0o600);
  if (process.getuid?.() === 0) chownSync(path, owner.uid, owner.gid);
}

export function makeComposeFixture() {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-compose-contract-"));
  const processUid = process.getuid?.() ?? 65532;
  const processGid = process.getgid?.() ?? 65532;
  const owner = {
    uid: processUid === 0 ? 65532 : processUid,
    gid: processUid === 0 ? 65532 : processGid,
  };
  chmodSync(scratch, 0o700);
  if (processUid === 0) chownSync(scratch, owner.uid, owner.gid);
  const defaultRuntimeState = join(scratch, "synveda-development");
  mkdirSync(defaultRuntimeState, { mode: 0o700 });
  chmodSync(defaultRuntimeState, 0o700);
  if (processUid === 0) chownSync(defaultRuntimeState, owner.uid, owner.gid);
  const secrets = join(defaultRuntimeState, "secrets");
  mkdirSync(secrets, { mode: 0o700 });
  chmodSync(secrets, 0o700);
  if (processUid === 0) chownSync(secrets, owner.uid, owner.gid);
  const oidcDirectorySecrets = join(secrets, "oidc-directory");
  mkdirSync(oidcDirectorySecrets, { mode: 0o700 });
  chmodSync(oidcDirectorySecrets, 0o700);
  if (processUid === 0) chownSync(oidcDirectorySecrets, owner.uid, owner.gid);
  for (const name of [...CORE_SECRETS, ...PROVIDER_SECRETS]) {
    writePrivate(join(secrets, name), `${SECRET_SENTINEL}-${name}`, owner);
  }
  const tls = generateTestTlsChain({
    commonName: "app.compose.example",
    sanHosts: ["app.compose.example", "auth.compose.example"],
  });
  writePrivate(join(secrets, "tls_cert"), tls.certificateChain.trimEnd(), owner);
  writePrivate(join(secrets, "tls_key"), tls.privateKey.trimEnd(), owner);
  const issuers = join(defaultRuntimeState, "issuers.json");
  writePrivate(
    issuers,
    JSON.stringify([
      {
        issuer: "http://auth.synveda.test:8080/realms/synveda",
        client_id: "synveda",
        audience: "synveda-api",
        tenant: { static: { tenant_id: "00000000-0000-0000-0000-000000000001" } },
        login_scopes: ["openid", "profile", "email"],
      },
    ]),
    owner,
  );
  return { scratch, secrets, issuers, ...owner };
}

export function composeEnvironment(fixture, overrides = {}) {
  const environment = { ...process.env };
  for (const name of Object.keys(environment)) {
    if (name.startsWith("SYNVEDA_")) delete environment[name];
  }
  for (const name of HOST_TRUST_CONTROLS) delete environment[name];
  for (const name of [
    "DATABASE_URL",
    "POSTGRES_PASSWORD",
    "KC_DB_PASSWORD",
    "KC_BOOTSTRAP_ADMIN_USERNAME",
    "KC_BOOTSTRAP_ADMIN_PASSWORD",
  ]) {
    delete environment[name];
  }
  const postgresMode = overrides.SYNVEDA_POSTGRES_MODE ?? "bundled";
  const oidcMode = overrides.SYNVEDA_OIDC_MODE ?? "bundled";
  const runtime = ["development", "reference"].includes(
    overrides.SYNVEDA_COMPOSE_RUNTIME,
  )
    ? overrides.SYNVEDA_COMPOSE_RUNTIME
    : "development";
  const requestedSuffix = overrides.SYNVEDA_COMPOSE_PROJECT_SUFFIX ?? "";
  const safeSuffix = /^acceptance-[a-z0-9][a-z0-9-]{0,23}$/.test(requestedSuffix)
    ? `-${requestedSuffix}`
    : "";
  const project = `synveda-${runtime}${safeSuffix}`;
  const runtimeState = join(fixture.scratch, project);
  const projectSecrets = join(runtimeState, "secrets");
  const projectIssuers = join(runtimeState, "issuers.json");
  const databaseAuthority = join(runtimeState, "database-authority");
  const keycloakPublicGate = join(runtimeState, "keycloak-public-gate");
  for (const directory of [runtimeState, databaseAuthority, keycloakPublicGate]) {
    mkdirSync(directory, { recursive: true, mode: 0o700 });
    chmodSync(directory, 0o700);
    if (process.getuid?.() === 0) chownSync(directory, fixture.uid, fixture.gid);
  }
  if (!existsSync(projectSecrets)) {
    mkdirSync(projectSecrets, { mode: 0o700 });
    chmodSync(projectSecrets, 0o700);
    if (process.getuid?.() === 0) chownSync(projectSecrets, fixture.uid, fixture.gid);
    const projectOidcDirectory = join(projectSecrets, "oidc-directory");
    mkdirSync(projectOidcDirectory, { mode: 0o700 });
    chmodSync(projectOidcDirectory, 0o700);
    if (process.getuid?.() === 0) {
      chownSync(projectOidcDirectory, fixture.uid, fixture.gid);
    }
    for (const name of [...CORE_SECRETS, ...PROVIDER_SECRETS, "tls_cert", "tls_key"]) {
      writePrivate(
        join(projectSecrets, name),
        readFileSync(join(fixture.secrets, name), "utf8").trimEnd(),
        fixture,
      );
    }
  }
  if (!existsSync(projectIssuers)) {
    writePrivate(
      projectIssuers,
      readFileSync(fixture.issuers, "utf8").trimEnd(),
      fixture,
    );
  }
  const externalRoleContract =
    postgresMode === "external"
      ? join(
          COMPOSE,
          "configs/database",
          oidcMode === "bundled" ? "roles.reference.json" : "roles.external-oidc.json",
        )
      : undefined;
  return {
    ...environment,
    COMPOSE_DISABLE_ENV_FILE: "0",
    COMPOSE_PROFILES: "ambient-profile-must-be-cleared",
    LC_ALL: "en_US.UTF-8",
    SYNVEDA_COMPOSE_IPV4_POOL: "172.30.240.0/24",
    SYNVEDA_RUNTIME_UID: String(fixture.uid),
    SYNVEDA_RUNTIME_GID: String(fixture.gid),
    SYNVEDA_SECRETS_DIR: projectSecrets,
    SYNVEDA_OIDC_ISSUERS_FILE: projectIssuers,
    SYNVEDA_DATABASE_AUTHORITY_DIR: databaseAuthority,
    SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR: keycloakPublicGate,
    ...(externalRoleContract === undefined
      ? {}
      : { SYNVEDA_DATABASE_ROLES_FILE: externalRoleContract }),
    ...overrides,
  };
}

function sorted(values) {
  return [...values].sort();
}

function keys(value) {
  return sorted(Object.keys(value ?? {}));
}

function canonicalJson(value) {
  if (Array.isArray(value)) return value.map(canonicalJson);
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.keys(value)
        .sort()
        .map((key) => [key, canonicalJson(value[key])]),
    );
  }
  return value;
}

function sameJson(left, right) {
  return JSON.stringify(canonicalJson(left)) === JSON.stringify(canonicalJson(right));
}

export function composeNetworkPlan(pool) {
  const match = /^(\d{1,3}\.\d{1,3}\.\d{1,3})\.0\/24$/.exec(pool);
  if (match === null) throw new Error("network plan requires a canonical /24 pool");
  const prefix = match[1];
  const ordinary = (offset) => ({
    subnet: `${prefix}.${offset}/28`,
    gateway: `${prefix}.${offset + 1}`,
  });
  return {
    "identity-backend": {
      ...ordinary(0),
      ip_range: `${prefix}.8/29`,
    },
    "public-edge": ordinary(16),
    "app-backend": ordinary(32),
    "synveda-data": ordinary(48),
    "keycloak-data": ordinary(64),
    "keycloak-management": ordinary(80),
    telemetry: ordinary(96),
    "application-egress": ordinary(112),
    "identity-egress": ordinary(128),
    "telemetry-egress": ordinary(144),
  };
}

export function developmentPortBindingFindings(source) {
  const markers = [
    "      - target: ${SYNVEDA_DEV_HTTP_PORT:-8080}",
    "        published: ${SYNVEDA_DEV_HTTP_PORT:-8080}",
    "        host_ip: 127.0.0.1",
    "        protocol: tcp",
  ];
  const offsets = markers.map((marker) => source.indexOf(marker));
  if (
    markers.some((marker) => occurrenceCount(source, marker) !== 1) ||
    offsets.some((offset) => offset < 0) ||
    offsets.some((offset, index) => index > 0 && offset <= offsets[index - 1])
  ) {
    return ["development proxy does not bind one identical container and host port"];
  }
  return [];
}

function occurrenceCount(source, token) {
  return source.split(token).length - 1;
}

export function reviewedKeycloakSourceFindings(relative, source) {
  const expected = KEYCLOAK_SECURITY_CHAIN_SHA256.get(relative);
  if (expected === undefined) return ["unknown Keycloak executable-chain input"];
  return createHash("sha256").update(source).digest("hex") === expected
    ? []
    : [`${relative} differs from the reviewed Keycloak executable chain`];
}

export function keycloakGenerationGateFindings(source) {
  const findings = [];
  const requireOnce = (token, finding) => {
    if (occurrenceCount(source, token) !== 1) findings.push(finding);
  };
  const requireOrder = (tokens, finding) => {
    let offset = -1;
    for (const token of tokens) {
      const next = source.indexOf(token, offset + 1);
      if (next < 0) {
        findings.push(finding);
        return;
      }
      offset = next;
    }
  };
  for (const [token, finding] of [
    [
      '[ "$(file_mode "$public_gate_dir")" = 700 ] && \\',
      "generation gate does not require a mode-0700 root",
    ],
    [
      '[ "$(file_owner "$public_gate_dir")" = "$(id -u)" ] || refuse',
      "generation gate does not require an owned root",
    ],
    [
      '[ "${#generation_suffix}" -eq 12 ] || return 1',
      "generation identifier length drifted",
    ],
    [
      'case "$generation_suffix" in',
      "generation identifier grammar drifted",
    ],
    [
      'captured_generation=$(readlink "$current_link") || return 1',
      "current generation is not captured from one symlink",
    ],
    [
      'mv -Tf -- "$staged_link" "$current_link" || {',
      "current generation rotation is not atomic",
    ],
    [
      'close_current_selector "$previous_generation" || fail',
      "generation rotation does not close the selector before withdrawal",
    ],
    [
      'if ! is_current_generation "$generation"; then',
      "publication lacks the post-rename generation fence",
    ],
    [
      '! is_current_generation "$generation" || return 1',
      "current generation can be retired",
    ],
    [
      '! chmod 0400 "$gate_candidate"; then',
      "generation publication does not create a mode-0400 witness",
    ],
  ]) {
    requireOnce(token, finding);
  }
  requireOrder(
    [
      "rotate_generation() {",
      'previous_generation=$(capture_generation) || refuse',
      'close_current_selector "$previous_generation" || fail',
      'withdraw_generation "$previous_generation" || fail',
      'generation_path=$(mktemp -d "$public_gate_dir/.generation-XXXXXXXXXXXX")',
      'ln -s -- "$generation" "$staged_link"',
      'mv -Tf -- "$staged_link" "$current_link"',
      'is_current_generation "$generation" || fail',
    ],
    "generation rotation does not close, stage, swap and witness in order",
  );
  requireOrder(
    [
      "publish_generation() {",
      'is_current_generation "$generation" || return 1',
      'gate_candidate=$(mktemp "$generation_path/.ready.XXXXXXXXXXXX")',
      'is_current_generation "$generation" || {',
      'mv -Tf -- "$gate_candidate" "$generation_ready"',
      'if ! is_current_generation "$generation"; then',
      'rm -f -- "$generation_ready"',
    ],
    "generation publication is not fenced before and after atomic rename",
  );
  for (const [block, finding] of [
    [
      [
        "valid_generation() {",
        '    case "${1:-}" in',
        "        .generation-*) generation_suffix=${1#.generation-} ;;",
        "        *) return 1 ;;",
        "    esac",
        '    [ "${#generation_suffix}" -eq 12 ] || return 1',
        '    case "$generation_suffix" in',
        "        *[!A-Za-z0-9]*) return 1 ;;",
        "    esac",
        "}",
      ].join("\n"),
      "generation identifier validator body drifted",
    ],
    [
      [
        "validate_generation_directory() {",
        '    valid_generation "$1" || return 1',
        "    generation_path=$public_gate_dir/$1",
        '    [ ! -L "$generation_path" ] && [ -d "$generation_path" ] && \\',
        '        [ "$(file_mode "$generation_path")" = 700 ] && \\',
        '        [ "$(file_owner "$generation_path")" = "$(id -u)" ]',
        "}",
      ].join("\n"),
      "generation directory witness body drifted",
    ],
    [
      [
        "close_current_selector() {",
        '    is_current_generation "$1" || return 1',
        '    rm -f -- "$current_link" || return 1',
        '    [ ! -e "$current_link" ] && [ ! -L "$current_link" ]',
        "}",
      ].join("\n"),
      "current generation selector closure body drifted",
    ],
    [
      [
        '    [ ! -L "$gate_candidate" ] && [ -f "$gate_candidate" ] && \\',
        '        [ "$(file_mode "$gate_candidate")" = 400 ] && \\',
        '        [ "$(file_owner "$gate_candidate")" = "$(id -u)" ] || {',
        '        rm -f -- "$gate_candidate"',
        "        return 1",
        "    }",
        "    gate_value=",
        '    IFS= read -r gate_value < "$gate_candidate" || [ -n "$gate_value" ]',
        '    [ "$gate_value" = "$contract" ] || {',
        '        rm -f -- "$gate_candidate"',
        "        return 1",
        "    }",
        "    gate_extra=",
        '    if IFS= read -r gate_extra < <(tail -n +2 "$gate_candidate"); then',
        '        rm -f -- "$gate_candidate"',
        "        return 1",
        "    fi",
      ].join("\n"),
      "generation publication witness body drifted",
    ],
    [
      [
        "ready_generation() {",
        "    generation=$(capture_generation) || return 1",
        '    is_current_generation "$generation" || return 1',
        "    generation_ready=$public_gate_dir/$generation/$contract.ready",
        '    [ ! -L "$generation_ready" ] && [ -f "$generation_ready" ] && \\',
        '        [ "$(file_mode "$generation_ready")" = 400 ] && \\',
        '        [ "$(file_owner "$generation_ready")" = "$(id -u)" ] || return 1',
        "    gate_value=",
        '    IFS= read -r gate_value < "$generation_ready" || [ -n "$gate_value" ]',
        '    [ "$gate_value" = "$contract" ] || return 1',
        "    gate_extra=",
        '    ! IFS= read -r gate_extra < <(tail -n +2 "$generation_ready")',
        "}",
      ].join("\n"),
      "selected generation readiness witness body drifted",
    ],
  ]) {
    if (occurrenceCount(source, block) !== 1) findings.push(finding);
  }
  if (/\brm\s+-rf\b|\beval\b/.test(source)) {
    findings.push("generation gate contains an unbounded mutation primitive");
  }
  return findings;
}

export function keycloakRealmSupervisorFindings(source) {
  const findings = [];
  const required = [
    "trap cleanup EXIT",
    "trap signal_shutdown HUP INT TERM",
    "withdraw_current_generation || {",
    "/opt/keycloak/bin/synveda-keycloak-health network",
    'synveda-realm-converge "$current_generation" &',
    'if ! $gate is-current "$current_generation"',
    'if ! management_ready; then',
    'kill -TERM "$child_pid"',
    'if [ "$child_status" -eq 0 ] && $gate ready',
    'failed_generation=$current_generation',
  ];
  for (const token of required) {
    if (!source.includes(token)) findings.push(`realm supervisor lacks ${token}`);
  }
  if (occurrenceCount(source, "failed_generation=$current_generation") !== 1) {
    findings.push("realm supervisor failure latch is not single-assignment per generation");
  }
  const startupWithdrawal = source.indexOf("withdraw_current_generation || {");
  const loop = source.indexOf("while :; do");
  if (startupWithdrawal < 0 || loop < 0 || startupWithdrawal >= loop) {
    findings.push("realm supervisor does not close inherited readiness before watching");
  }
  for (const [block, finding] of [
    [
      [
        "withdraw_current_generation() {",
        "    observed=$($gate capture 2>/dev/null) || return 0",
        '    $gate withdraw "$observed" >/dev/null 2>&1 || return 1',
        "}",
      ].join("\n"),
      "realm supervisor current-generation withdrawal helper drifted",
    ],
    [
      [
        "cleanup() {",
        "    entry_status=$?",
        "    trap '' HUP INT TERM",
        "    trap - EXIT",
        '    if [ -n "${child_pid:-}" ]; then',
        '        kill -TERM "$child_pid" 2>/dev/null || true',
        '        wait "$child_pid" 2>/dev/null || true',
        "    fi",
        "    if ! withdraw_current_generation; then",
        '        echo "keycloak-supervisor: public gate withdrawal failed" >&2',
        "        entry_status=70",
        "    fi",
        '    exit "$entry_status"',
        "}",
      ].join("\n"),
      "realm supervisor cleanup and withdrawal body drifted",
    ],
    [
      [
        '    if [ "$observed_generation" != "$current_generation" ]; then',
        "        current_generation=$observed_generation",
        "        failed_generation=",
        '        $gate withdraw "$current_generation" >/dev/null 2>&1 || {',
        '            echo "keycloak-supervisor: new generation could not be closed" >&2',
        "            exit 70",
        "        }",
        "    fi",
      ].join("\n"),
      "realm supervisor new-generation withdrawal body drifted",
    ],
    [
      [
        "    if ! management_ready; then",
        '        $gate withdraw "$current_generation" >/dev/null 2>&1 || {',
        '            if $gate is-current "$current_generation" >/dev/null 2>&1; then',
        '                echo "keycloak-supervisor: degraded generation could not be closed" >&2',
        "                exit 70",
        "            fi",
        "        }",
        "        sleep 2",
        "        continue",
        "    fi",
      ].join("\n"),
      "realm supervisor degraded-generation withdrawal body drifted",
    ],
    [
      [
        "    /opt/keycloak/bin/keycloak-entrypoint \\",
        '        synveda-realm-converge "$current_generation" &',
        "    child_pid=$!",
      ].join("\n"),
      "realm supervisor child bypasses the reviewed entrypoint",
    ],
    [
      [
        "        if ! management_ready; then",
        "            dependency_degraded=true",
        '            $gate withdraw "$current_generation" >/dev/null 2>&1 || true',
        '            kill -TERM "$child_pid" 2>/dev/null || true',
        "            break",
        "        fi",
      ].join("\n"),
      "realm supervisor degraded-child withdrawal body drifted",
    ],
    [
      [
        '    if [ "$dependency_degraded" = true ]; then',
        "        # Convergence suppresses signals while its bounded cleanup settles",
        "        # sessions and may have reached publication after the pre-kill",
        "        # withdrawal. Close the still-current generation again after wait.",
        '        $gate withdraw "$current_generation" >/dev/null 2>&1 || {',
        '            if $gate is-current "$current_generation" >/dev/null 2>&1; then',
        '                echo "keycloak-supervisor: post-child degraded generation could not be closed" >&2',
        "                exit 70",
        "            fi",
        "        }",
        "        sleep 2",
        "        continue",
        "    fi",
      ].join("\n"),
      "realm supervisor post-child degraded withdrawal body drifted",
    ],
    [
      [
        '    $gate withdraw "$current_generation" >/dev/null 2>&1 || {',
        '        echo "keycloak-supervisor: failed generation could not be closed" >&2',
        "        exit 70",
        "    }",
        "    failed_generation=$current_generation",
      ].join("\n"),
      "realm supervisor failed-generation withdrawal body drifted",
    ],
  ]) {
    if (occurrenceCount(source, block) !== 1) findings.push(finding);
  }
  if (/\bsynveda-realm-converge\s*&/.test(source)) {
    findings.push("realm supervisor launches convergence without an immutable generation fence");
  }
  if (/\brm\s+-rf\b|\beval\b/.test(source)) {
    findings.push("realm supervisor contains an unbounded mutation primitive");
  }
  return findings;
}

export function keycloakHealthFindings(source) {
  const findings = [];
  for (const token of [
    "validate_response() (",
    "head -c 65537",
    '[ "$response_size" -gt 0 ] && [ "$response_size" -le 65536 ]',
    "IFS= read -r -d '' nul_prefix",
    '[[ "$status_line" =~ ^HTTP/1\\.[01]\\ 200\\  ]]',
    '[ "$headers_complete" = true ] || return 1',
    'if [ "$mode" = self-test ]; then',
    "validate_response \"$response\" && exit 1",
  ]) {
    if (!source.includes(token)) findings.push(`Keycloak health proof lacks ${token}`);
  }
  if (/grep[^\n]*status|\[\[[^\n]*\$body[^\n]*status/.test(source)) {
    findings.push("Keycloak health adds a weaker response-body status oracle");
  }
  for (const [token, finding] of [
    [
      [
        "    local)",
        '        [ "$#" -eq 1 ] || exit 64',
        "        health_host=127.0.0.1",
        "        ;;",
      ].join("\n"),
      "Keycloak local health authority drifted",
    ],
    [
      [
        "    network)",
        '        [ "$#" -eq 1 ] || exit 64',
        "        health_host=keycloak",
        "        ;;",
      ].join("\n"),
      "Keycloak network health authority drifted",
    ],
    [
      "/usr/bin/timeout --foreground --signal=TERM --kill-after=1s 4s \\",
      "Keycloak management health timeout drifted",
    ],
    [
      'exec 3<>"/dev/tcp/$host/9000"',
      "Keycloak management health port drifted",
    ],
    [
      'printf "GET /health/ready HTTP/1.1\\r\\nHost: localhost\\r\\nConnection: close\\r\\n\\r\\n" >&3',
      "Keycloak management health endpoint drifted",
    ],
  ]) {
    if (occurrenceCount(source, token) !== 1) findings.push(finding);
  }
  return findings;
}

export function masterClientAuthorityFindings(source) {
  const expectedStage =
    "            atAuthorityStage(AuthorityStage.MASTER_CLIENTS, () -> {\n"
    + "                int statusCode = sendDiscarding(\n"
    + "                    client,\n"
    + "                    authorisedGet(\n"
    + '                        "http://keycloak:8080/admin/realms/master/clients"\n'
    + '                            + "?clientId=admin-cli",\n'
    + "                        token\n"
    + "                    ),\n"
    + "                    proofDeadlineNanos\n"
    + "                );\n"
    + "                verifyForbiddenAuthorityResponse(statusCode);\n"
    + "            });\n";
  const startMarker =
    "            atAuthorityStage(AuthorityStage.MASTER_CLIENTS, () -> {";
  const start = source.indexOf(startMarker);
  const end = source.indexOf(
    "            atAuthorityStage(AuthorityStage.MASTER_SESSION_STATS, () -> {",
    start + 1,
  );
  if (start < 0 || end < 0 || end <= start) {
    return ["Keycloak master-client authority stage drifted"];
  }
  const stage = source.slice(start, end);
  if (
    occurrenceCount(source, startMarker) !== 1 ||
    stage !== expectedStage
  ) {
    return ["Keycloak master-client authority is not an exact body-free refusal"];
  }
  return [];
}

export function authorityCleanupOrderFindings(source) {
  const markers = [
    "        AuthorityTokenGrant tokenGrant = atAuthorityStage(\n"
      + "            AuthorityStage.TOKEN_ENVELOPE,\n"
      + "            () -> parseAuthorityTokenGrant(\n"
      + "                tokenResponse.statusCode(),\n"
      + "                tokenResponse.body()\n"
      + "            )\n"
      + "        );",
    "        String refreshToken = tokenGrant.refreshToken();",
    "        runAuthorityProofWithCleanup(() -> {",
    "            AuthorityTokens tokens = atAuthorityStage(\n"
      + "                AuthorityStage.TOKEN_CONTRACT,\n"
      + "                () -> parseAuthorityTokenResponse(tokenGrant.response())",
    "            atAuthorityStage(\n"
      + "                AuthorityStage.REFRESH_CONTRACT,\n"
      + "                () -> verifyAuthorityRefreshContract(",
    "        }, () -> revokeAndVerifyRefreshRefused(client, refreshToken));",
  ];
  const offsets = markers.map((marker) => source.indexOf(marker));
  if (
    markers.some((marker) => occurrenceCount(source, marker) !== 1) ||
    offsets.some((offset) => offset < 0) ||
    offsets.some((offset, index) => index > 0 && offset <= offsets[index - 1])
  ) {
    return [
      "Keycloak authority grant, guarded token contracts or cleanup order drifted",
    ];
  }
  return [];
}

function secretBindings(service) {
  return sorted((service.secrets ?? []).map(({ source, target }) => `${source}:${target}`));
}

function bindMount(service, target) {
  return (service.volumes ?? []).find(
    (mount) => mount.type === "bind" && mount.target === target,
  );
}

function bindMountTargets(service) {
  return sorted(
    (service.volumes ?? [])
      .filter(({ type }) => type === "bind")
      .map(({ target, read_only }) => `${target}:${read_only === true ? "ro" : "rw"}`),
  );
}

function nonBindMountTargets(service) {
  return sorted(
    (service.volumes ?? [])
      .filter(({ type }) => type !== "bind")
      .map(
        ({ type, target, read_only }) =>
          `${type}:${target}:${read_only === true ? "ro" : "rw"}`,
      ),
  );
}

function dependencyBindings(service) {
  return sorted(
    Object.entries(service.depends_on ?? {}).map(
      ([name, dependency]) =>
        `${name}:${dependency.condition}:${dependency.restart === true ? "restart" : "no-restart"}:${dependency.required === false ? "optional" : "required"}`,
    ),
  );
}

function pathsOverlap(left, right) {
  const resolvedLeft = resolve(left);
  const resolvedRight = resolve(right);
  return (
    resolvedLeft === resolvedRight ||
    resolvedLeft.startsWith(`${resolvedRight}/`) ||
    resolvedRight.startsWith(`${resolvedLeft}/`)
  );
}

function publishedPorts(model) {
  return Object.entries(model.services).flatMap(([service, config]) =>
    (config.ports ?? []).map((port) => ({ service, ...port })),
  );
}

function hardeningFindings(name, service) {
  const findings = [];
  if (service.privileged === true) findings.push(`${name} is privileged`);
  if (service.use_api_socket !== undefined) {
    findings.push(`${name} receives the container-engine API socket`);
  }
  if (service.network_mode !== undefined) findings.push(`${name} bypasses explicit networks`);
  if (service.pid !== undefined || service.ipc !== undefined) {
    findings.push(`${name} shares a process or IPC namespace`);
  }
  for (const resolverOverride of ["extra_hosts", "dns", "dns_search", "dns_opt"]) {
    if (service[resolverOverride] !== undefined) {
      findings.push(`${name} overrides trusted name resolution with ${resolverOverride}`);
    }
  }
  for (const mechanism of ["links", "external_links", "volumes_from"]) {
    if ((service[mechanism] ?? []).length > 0) {
      findings.push(`${name} uses implicit dependency mechanism ${mechanism}`);
    }
  }
  if ((service.volumes ?? []).some((mount) => JSON.stringify(mount).includes("docker.sock"))) {
    findings.push(`${name} mounts the Docker socket`);
  }
  if (JSON.stringify(service.cap_drop ?? []) !== JSON.stringify(["ALL"])) {
    findings.push(`${name} does not drop all capabilities`);
  }
  if (
    JSON.stringify(service.security_opt ?? []) !==
    JSON.stringify(["no-new-privileges:true"])
  ) {
    findings.push(`${name} security options drifted from no-new-privileges`);
  }
  if (service.read_only !== true) findings.push(`${name} root filesystem is writable`);
  if (service.init !== true) findings.push(`${name} lacks init signal forwarding`);
  if (!Number.isInteger(service.pids_limit) || service.pids_limit <= 0) {
    findings.push(`${name} has no positive PID bound`);
  }
  if (name !== "postgres") {
    const [uid, gid] = String(service.user ?? "").split(":").map(Number);
    if (!Number.isInteger(uid) || !Number.isInteger(gid) || uid <= 0 || gid <= 0) {
      findings.push(`${name} does not use an explicit non-root UID:GID`);
    }
    if ((service.cap_add ?? []).length > 0) findings.push(`${name} adds capabilities`);
  } else if (
    JSON.stringify(sorted(service.cap_add ?? [])) !==
    JSON.stringify(sorted(["CHOWN", "DAC_OVERRIDE", "FOWNER", "SETGID", "SETUID"]))
  ) {
    findings.push("postgres root-at-start capability exception drifted");
  }
  return findings;
}

function normalizedByteSize(value) {
  if (Number.isSafeInteger(value) && value >= 0) return value;
  const match = /^(\d+)([kmgt])?$/i.exec(String(value ?? ""));
  if (match === null) return undefined;
  const powers = { "": 0, k: 1, m: 2, g: 3, t: 4 };
  const bytes = Number(match[1]) * 1024 ** powers[(match[2] ?? "").toLowerCase()];
  return Number.isSafeInteger(bytes) ? bytes : undefined;
}

export function collectorConfigFindings(config) {
  const findings = [];
  if (!config.includes("extensions:\n  health_check:\n    endpoint: 127.0.0.1:13133\n")) {
    findings.push("Collector health endpoint is not container-loopback-only");
  }
  if (!config.includes(`    response_body:\n      healthy: '${COLLECTOR_HEALTH_BODY}'\n`)) {
    findings.push("Collector healthy response is not the content-free nop pipeline config");
  }
  if (!config.includes("      unhealthy: '{}'\n")) {
    findings.push("Collector unhealthy response is not the closed empty object");
  }
  if (!config.includes("service:\n  extensions: [health_check]\n")) {
    findings.push("Collector health extension is not enabled");
  }
  return findings;
}

export function caddyTrustBoundaryFindings(config) {
  const contract = `{
\tadmin off
\thttp_port {$SYNVEDA_PROXY_HTTP_PORT}
\thttps_port {$SYNVEDA_PROXY_HTTPS_PORT}
\tservers {
\t\tmax_header_size 32KB
\t\ttimeouts {
\t\t\tread_body 30s
\t\t\tread_header 10s
\t\t\twrite 60s
\t\t\tidle 2m
\t\t}
\t}
}

(synveda_response_headers) {
\theader {
\t\t-Server
\t\tX-Content-Type-Options nosniff
\t\tX-Frame-Options DENY
\t\tReferrer-Policy no-referrer
\t\tPermissions-Policy "camera=(), microphone=(), geolocation=()"
\t}
}

(synveda_upstream) {
\theader_up -Forwarded
\theader_up -X-Forwarded-*
\theader_up -X-Real-IP
\theader_up -X-Original-*
\theader_up -X-Remote-User
\theader_up -X-Authenticated-User
\theader_up -X-User
\theader_up -Remote-User
\theader_up -traceparent
\theader_up -tracestate
\theader_up -baggage
\theader_up -b3
\theader_up -X-B3-*
\theader_up -uber-trace-id
\theader_up -ot-tracer-*
\theader_up X-Forwarded-For {remote_host}
\theader_up X-Forwarded-Host {host}
\theader_up X-Forwarded-Proto {scheme}
\theader_up X-Forwarded-Port {$SYNVEDA_PUBLIC_PORT}
\ttransport http {
\t\tdial_timeout 5s
\t\tresponse_header_timeout 30s
\t\tkeepalive_idle_conns_per_host 32
\t}
}

import /etc/caddy/app.caddy
import /etc/caddy/identity.caddy`;
  return config.replaceAll("\r\n", "\n").trimEnd() === contract
    ? []
    : ["Caddy trust boundary differs from the closed grammar"];
}

export function appRouteFindings(config) {
  const routeBody = `\timport synveda_response_headers
\trequest_body {
\t\tmax_size 16MB
\t}
\thandle /metrics {
\t\trespond 404
\t}
\thandle {
\t\treverse_proxy gateway:8120 {
\t\t\timport synveda_upstream
\t\t}
\t}`;
  const developmentContract = `http://{$SYNVEDA_APP_HOST} {
${routeBody}
}`;
  const referenceContract = `https://{$SYNVEDA_APP_HOST} {
\ttls /run/secrets/tls_cert /run/secrets/tls_key
\timport synveda_response_headers
\theader Strict-Transport-Security "max-age=31536000; includeSubDomains"
\trequest_body {
\t\tmax_size 16MB
\t}
\thandle /metrics {
\t\trespond 404
\t}
\thandle {
\t\treverse_proxy gateway:8120 {
\t\t\timport synveda_upstream
\t\t}
\t}
}`;
  const normalized = config.replaceAll("\r\n", "\n").trimEnd();
  return normalized === developmentContract || normalized === referenceContract
    ? []
    : ["application proxy differs from the closed route grammar"];
}

export function identityGateFindings(config) {
  const findings = [];
  const publicPaths = "/realms/synveda/.well-known/openid-configuration /realms/synveda/protocol/openid-connect/auth /realms/synveda/protocol/openid-connect/token /realms/synveda/protocol/openid-connect/certs /realms/synveda/protocol/openid-connect/logout /realms/synveda/protocol/openid-connect/logout/logout-confirm /realms/synveda/login-actions/* /realms/synveda/account /realms/synveda/account/* /resources/*";
  const readyMatcher = `\t@identity_ready {
\t\tpath ${publicPaths}
\t\tfile {
\t\t\troot /run/synveda/keycloak-public-gate/current
\t\t\ttry_files cpr45-keycloak-realm-v3.ready
\t\t}
\t}`;
  const readyHandler = `\thandle @identity_ready {
\t\treverse_proxy keycloak:8080 {
\t\t\timport synveda_upstream
\t\t}
\t}`;
  const closedHandler = `\thandle @identity_path {
\t\theader Cache-Control "no-store"
\t\theader Pragma "no-cache"
\t\trespond 503
\t}`;
  const fallbackHandler = `\thandle {
\t\trespond 404
\t}`;
  const identityPaths = `\t@identity_path path ${publicPaths}`;
  const routeBody = `${readyMatcher}
${identityPaths}
${readyHandler}
${closedHandler}
${fallbackHandler}`;
  const developmentContract = `http://{$SYNVEDA_AUTH_HOST} {
\timport synveda_response_headers
\trequest_body {
\t\tmax_size 2MB
\t}
${routeBody}
}`;
  const referenceContract = `https://{$SYNVEDA_AUTH_HOST} {
\ttls /run/secrets/tls_cert /run/secrets/tls_key
\timport synveda_response_headers
\theader Strict-Transport-Security "max-age=31536000; includeSubDomains"
\trequest_body {
\t\tmax_size 2MB
\t}
${routeBody}
}`;
  const normalized = config.replaceAll("\r\n", "\n").trimEnd();
  if (normalized !== developmentContract && normalized !== referenceContract) {
    findings.push("identity proxy differs from the closed route grammar");
  }
  for (const [name, block] of [
    ["ready matcher", readyMatcher],
    ["ready handler", readyHandler],
    ["closed handler", closedHandler],
  ]) {
    if (config.split(block).length - 1 !== 1) {
      findings.push(`identity public gate ${name} drifted`);
    }
  }
  if (config.split(identityPaths).length - 1 !== 1) {
    findings.push("identity public gate closed-path matcher drifted");
  }
  const ordered = [
    config.indexOf(readyMatcher),
    config.indexOf(identityPaths),
    config.indexOf(readyHandler),
    config.indexOf(closedHandler),
  ];
  if (
    ordered.some((index) => index < 0) ||
    ordered.some((index, position) => position > 0 && index <= ordered[position - 1])
  ) {
    findings.push("identity public gate handler order drifted");
  }
  if (/\/admin|\/realms\/master|\/health|\/metrics/.test(config)) {
    findings.push("identity proxy exposes an operator-only Keycloak path");
  }
  const reverseProxyDirectives = config
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.startsWith("reverse_proxy "));
  if (
    reverseProxyDirectives.length !== 1 ||
    reverseProxyDirectives.some((line) => line !== "reverse_proxy keycloak:8080 {")
  ) {
    findings.push("identity proxy has an additive ungated upstream route");
  }
  const imports = sorted(
    config
      .split("\n")
      .map((line) => line.trim())
      .filter((line) => line.startsWith("import ")),
  );
  if (
    JSON.stringify(imports) !==
    JSON.stringify(sorted(["import synveda_response_headers", "import synveda_upstream"]))
  ) {
    findings.push("identity proxy imports are outside the closed route grammar");
  }
  return findings;
}

export function keycloakConvergenceFindings(source) {
  const findings = [];
  if (
    createHash("sha256").update(source).digest("hex") !==
    KEYCLOAK_SECURITY_CHAIN_SHA256.get("keycloak/synveda-realm-converge")
  ) {
    findings.push("Keycloak convergence source differs from the reviewed executable");
  }
  const trimmedLines = source.split("\n").map((line) => line.trim());
  const requireOnce = (token, finding) => {
    if (occurrenceCount(source, token) !== 1) findings.push(finding);
  };
  const requireLineCount = (line, count, finding) => {
    if (trimmedLines.filter((candidate) => candidate === line).length !== count) {
      findings.push(finding);
    }
  };
  const requireOrder = (markers, finding) => {
    let previous = -1;
    for (const marker of markers) {
      const offset = source.indexOf(marker, previous + 1);
      if (offset < 0) {
        findings.push(finding);
        return;
      }
      previous = offset;
    }
  };

  requireOnce("withdraw_public_gate || {", "realm convergence does not withdraw before authority use");
  requireLineCount(
    "withdraw_public_gate || {",
    1,
    "realm convergence withdrawal can be short-circuited",
  );
  requireOrder(
    [
      "withdraw_public_gate || {",
      "state_dir=$(mktemp -d",
      "trap cleanup EXIT",
      'if authenticate "$permanent_config"',
    ],
    "realm convergence withdrawal, cleanup and authentication order drifted",
  );
  for (const trap of [
    "trap cleanup EXIT",
    "trap 'signal_exit 129' HUP",
    "trap 'signal_exit 130' INT",
    "trap 'signal_exit 143' TERM",
  ]) {
    requireOnce(trap, `realm convergence signal contract lacks ${trap}`);
    requireLineCount(trap, 1, `realm convergence signal contract can bypass ${trap}`);
  }
  for (const [line, count, finding] of [
    ["trap '' HUP INT TERM", 1, "cleanup signal suppression can be bypassed"],
    ["trap - EXIT", 1, "cleanup EXIT trap release can be bypassed"],
    ["if publish_public_gate; then", 1, "public gate publication can be bypassed"],
    ["publish_gate_on_exit=true", 2, "publication authorization can be bypassed"],
    [
      'try_complete_projection "$authority_config" || {',
      1,
      "complete projection proof can be short-circuited",
    ],
    [
      'admin_quiet "$authority_config" realm-enable update "realms/$realm" -s enabled=true',
      1,
      "realm enablement command can be short-circuited",
    ],
    ["prove_bootstrap_refused || {", 1, "bootstrap refusal proof can be short-circuited"],
    [
      'if run_admin_quarantine "$quarantine_config" "$state_dir"; then',
      1,
      "authenticated quarantine update can be short-circuited",
    ],
    [
      'if run_admin_quarantine "$recovery_config" "$recovery_dir"; then',
      1,
      "fresh-authority quarantine update can be short-circuited",
    ],
    [
      "authentication_client=admin-cli",
      1,
      "permanent authority authentication client drifted",
    ],
    [
      'project_quiet bootstrap-user-delete admin-bootstrap-delete \\',
      1,
      "bootstrap retirement does not use the exact-ID administrative delete",
    ],
    [
      "--rolename manage-realm --rolename manage-clients --rolename manage-users",
      1,
      "permanent authority target role assignment drifted",
    ],
    [
      "--rolename view-users",
      1,
      "permanent authority audit role assignment drifted",
    ],
    [
      '--uid "$permanent_user_id" --cclientid master-realm \\',
      1,
      "permanent authority audit client drifted",
    ],
    [
      'try_prove_user_profile "$complete_config" complete || return',
      1,
      "complete projection omits the exact managed user profile",
    ],
    [
      'admin_quiet "$authority_config" user-profile-update update "users/profile" \\',
      1,
      "managed user profile replacement can be bypassed",
    ],
    [
      'prove_user_profile "$authority_config" repaired',
      1,
      "managed user profile readback can be bypassed",
    ],
    [
      '-r "$realm" -n -f "$user_profile_contract"',
      1,
      "managed user profile replacement can merge untrusted policy",
    ],
    [
      'if [ "$user_profile_valid" = false ] && \\',
      1,
      "untrusted demo marker provenance is not refused",
    ],
  ]) {
    requireLineCount(line, count, finding);
  }
  if (source.includes("realm-management")) {
    findings.push("nonexistent master realm-management client remains reachable");
  }
  if (source.includes("synveda-convergence-cli")) {
    findings.push("legacy convergence authentication client remains reachable");
  }
  if (source.includes("manage-events")) {
    findings.push("unused Keycloak event-management authority remains reachable");
  }
  for (const residue of [
    "synveda-convergence-proof",
    "authority_proof_client=",
    "proof_config=",
    "reconcile_authority_proof_client()",
    "security.admin.console",
  ]) {
    if (source.includes(residue)) {
      findings.push("custom proof-only authority client residue remains reachable");
      break;
    }
  }
  if (/[^\n]*>+\s*"\$public_gate"/.test(source)) {
    findings.push("realm convergence writes public gate outside atomic publication");
  }
  const publicationAssignments = trimmedLines.filter((line) =>
    line.startsWith("publish_gate_on_exit="),
  );
  if (
    JSON.stringify(publicationAssignments) !==
    JSON.stringify([
      "publish_gate_on_exit=false",
      "publish_gate_on_exit=true",
      "publish_gate_on_exit=true",
    ])
  ) {
    findings.push("realm convergence publication authorization grammar drifted");
  }

  const cleanupStart = source.indexOf("cleanup() {");
  const cleanupEnd = source.indexOf("\nsignal_exit() {", cleanupStart);
  const cleanup =
    cleanupStart >= 0 && cleanupEnd > cleanupStart
      ? source.slice(cleanupStart, cleanupEnd)
      : "";
  const cleanupMarkers = [
    "trap '' HUP INT TERM",
    "trap - EXIT",
    'if ! "$generation_gate" is-current "$generation" >/dev/null 2>&1; then',
    "generation_stale=true",
    'if [ "$cleanup_status" -ne 0 ] && ! withdraw_public_gate; then',
    'if [ "$cleanup_status" -ne 0 ] && [ "$realm_known" = true ] && \\',
    '[ "$generation_stale" = false ]; then',
    "if ! quarantine; then",
    "session_close_failed=false",
    "for session_config in \\",
    'if close_admin_session "$session_config" "$state_dir"; then',
    'if [ "$session_close_failed" = true ]; then',
    'if ! rm -f -- "$state_dir"/*; then',
    'if ! rmdir -- "$state_dir" 2>/dev/null; then',
    '[ "$publish_gate_on_exit" = true ] || cleanup_status=70',
    "if publish_public_gate; then",
    'if [ "$cleanup_status" -ne 0 ]; then',
    "! quarantine_with_fresh_authority; then",
    "unset bootstrap_password convergence_password",
    'exit "$cleanup_status"',
  ];
  const cleanupOffsets = cleanupMarkers.map((marker) => cleanup.indexOf(marker));
  if (
    cleanup.length === 0 ||
    cleanupOffsets.some((offset) => offset < 0) ||
    cleanupOffsets.some(
      (offset, index) => index > 0 && offset <= cleanupOffsets[index - 1],
    )
  ) {
    findings.push("realm convergence cleanup/quarantine/publication order drifted");
  }
  if (occurrenceCount(source, "publish_public_gate") !== 2) {
    findings.push("realm convergence permits publication outside bounded cleanup");
  }
  requireOrder(
    [
      'if try_prove_user_profile "$authority_config" initial; then',
      "user_profile_valid=true",
      'refresh_demo_inventory "$authority_config"',
      '[ "$user_profile_valid" = true ] && \\',
      'try_complete_projection "$permanent_config"; then',
    ],
    "trusted profile, marker inventory and fast-path order drifted",
  );
  requireOrder(
    [
      'admin_quiet "$authority_config" realm-disable update "realms/$realm" -s enabled=false',
      'project_quiet realm-disabled realm-state "$realm_json" false',
      'admin_quiet "$authority_config" realm-update update "realms/$realm" \\',
      'admin_quiet "$authority_config" user-profile-update update "users/profile" \\',
      'prove_user_profile "$authority_config" repaired',
      'refresh_demo_inventory "$authority_config"',
      'if [ "$user_profile_valid" = false ] && \\',
      'client_id=$(query_client_id "$authority_config" "$realm" "$client")',
    ],
    "closed-realm profile repair, provenance and application mutation order drifted",
  );

  const exactFunction = (startToken, endToken, expectedLines, finding) => {
    const anchoredStart = `\n${startToken}`;
    const uniqueStart = occurrenceCount(source, anchoredStart) === 1;
    const anchoredOffset = uniqueStart ? source.indexOf(anchoredStart) : -1;
    const start = anchoredOffset >= 0 ? anchoredOffset + 1 : -1;
    const end = source.indexOf(endToken, start + startToken.length);
    const actualLines =
      start >= 0 && end > start
        ? source
            .slice(start, end)
            .split("\n")
            .map((line) => line.trim())
            .filter(Boolean)
        : [];
    if (JSON.stringify(actualLines) !== JSON.stringify(expectedLines)) {
      findings.push(finding);
    }
  };

  exactFunction(
    "cleanup_authenticate() {",
    "\nrun_admin_quarantine() {",
    [
      "cleanup_authenticate() {",
      "cleanup_auth_config=$1",
      "cleanup_auth_username=$2",
      "cleanup_auth_password=$3",
      "cleanup_auth_client=$4",
      "cleanup_auth_dir=$5",
      "[ ! -e \"$cleanup_auth_config\" ] && [ ! -L \"$cleanup_auth_config\" ] || return 1",
      "rm -f -- \"$cleanup_auth_dir/auth.out\"",
      "KC_CLI_PASSWORD=$cleanup_auth_password",
      "export KC_CLI_PASSWORD",
      "if run_cleanup_kcadm \"$cleanup_auth_config\" config credentials \\",
      "--server \"$admin_url\" --realm master --client \"$cleanup_auth_client\" \\",
      "--user \"$cleanup_auth_username\" \\",
      ">\"$cleanup_auth_dir/auth.out\" 2>&1; then",
      "cleanup_auth_status=0",
      "else",
      "cleanup_auth_status=1",
      "fi",
      "unset KC_CLI_PASSWORD",
      "rm -f -- \"$cleanup_auth_dir/auth.out\"",
      "if [ \"$cleanup_auth_status\" -ne 0 ]; then",
      "if [ -e \"$cleanup_auth_config\" ] || [ -L \"$cleanup_auth_config\" ]; then",
      "settle_failed_admin_session \\",
      "\"$cleanup_auth_config\" \"$cleanup_auth_dir\" || return 1",
      "rm -f -- \"$cleanup_auth_config\" || return 1",
      "[ ! -e \"$cleanup_auth_config\" ] && \\",
      "[ ! -L \"$cleanup_auth_config\" ] || return 1",
      "fi",
      "fi",
      "return \"$cleanup_auth_status\"",
      "}",
    ],
    "cleanup quarantine authentication body drifted",
  );
  exactFunction(
    "run_admin_quarantine() {",
    "\nclose_admin_session() {",
    [
      "run_admin_quarantine() {",
      "quarantine_config=$1",
      "quarantine_dir=$2",
      "rm -f -- \"$quarantine_dir/quarantine.out\" \"$quarantine_dir/quarantine.error\"",
      "if /usr/bin/timeout --foreground --signal=TERM --kill-after=1s 10s \\",
      "/usr/bin/java -cp '/opt/keycloak/bin:/opt/keycloak/lib/lib/main/*' \\",
      "SynvedaKeycloakProjection admin-quarantine \"$quarantine_config\" \\",
      ">\"$quarantine_dir/quarantine.out\" \\",
      "2>\"$quarantine_dir/quarantine.error\"; then",
      "quarantine_status=0",
      "else",
      "quarantine_status=1",
      "fi",
      "rm -f -- \"$quarantine_dir/quarantine.out\" \"$quarantine_dir/quarantine.error\"",
      "return \"$quarantine_status\"",
      "}",
    ],
    "direct administrative quarantine body drifted",
  );
  exactFunction(
    "close_admin_session() {",
    "\nsettle_failed_admin_session() {",
    [
      "close_admin_session() {",
      "close_config=$1",
      "close_dir=$2",
      "[ ! -L \"$close_config\" ] && [ -f \"$close_config\" ] || return 1",
      "rm -f -- \"$close_dir/session-close.out\" \"$close_dir/session-close.error\"",
      "if /usr/bin/timeout --foreground --signal=TERM --kill-after=1s 9s \\",
      "/usr/bin/java -cp '/opt/keycloak/bin:/opt/keycloak/lib/lib/main/*' \\",
      "SynvedaKeycloakProjection admin-session-close \"$close_config\" \\",
      ">\"$close_dir/session-close.out\" \\",
      "2>\"$close_dir/session-close.error\"; then",
      "close_status=0",
      "else",
      "close_status=1",
      "fi",
      "[ ! -s \"$close_dir/session-close.out\" ] || close_status=1",
      "rm -f -- \"$close_dir/session-close.out\" \"$close_dir/session-close.error\"",
      "return \"$close_status\"",
      "}",
    ],
    "administrative session closure body drifted",
  );
  exactFunction(
    "settle_failed_admin_session() {",
    "\ntry_admin_to_file() {",
    [
      "settle_failed_admin_session() {",
      "failed_config=$1",
      "failed_dir=$2",
      "[ ! -L \"$failed_config\" ] && [ -f \"$failed_config\" ] || return 1",
      "rm -f -- \"$failed_dir/session-settle.out\" \"$failed_dir/session-settle.error\"",
      "if /usr/bin/timeout --foreground --signal=TERM --kill-after=1s 9s \\",
      "/usr/bin/java -cp '/opt/keycloak/bin:/opt/keycloak/lib/lib/main/*' \\",
      "SynvedaKeycloakProjection admin-session-settle-failed \"$failed_config\" \\",
      ">\"$failed_dir/session-settle.out\" \\",
      "2>\"$failed_dir/session-settle.error\"; then",
      "failed_status=0",
      "else",
      "failed_status=1",
      "fi",
      "[ ! -s \"$failed_dir/session-settle.out\" ] || failed_status=1",
      "rm -f -- \"$failed_dir/session-settle.out\" \"$failed_dir/session-settle.error\"",
      "return \"$failed_status\"",
      "}",
    ],
    "failed authentication session settlement body drifted",
  );
  exactFunction(
    "authenticate() {",
    "\ntry_prove_exact_permanent_authority() {",
    [
      "authenticate() {",
      "auth_config=$1",
      "auth_username=$2",
      "auth_password=$3",
      "auth_client=${4:-admin-cli}",
      "if [ -e \"$auth_config\" ] || [ -L \"$auth_config\" ]; then",
      "close_admin_session \"$auth_config\" \"$state_dir\" || return 70",
      "rm -f -- \"$auth_config\" || return 70",
      "[ ! -e \"$auth_config\" ] && [ ! -L \"$auth_config\" ] || return 70",
      "fi",
      "rm -f -- \"$state_dir/auth.out\"",
      "KC_CLI_PASSWORD=$auth_password",
      "export KC_CLI_PASSWORD",
      "if run_kcadm \"$auth_config\" config credentials --server \"$admin_url\" \\",
      "--realm master --client \"$auth_client\" --user \"$auth_username\" \\",
      ">\"$state_dir/auth.out\" 2>&1; then",
      "auth_status=0",
      "else",
      "auth_status=1",
      "fi",
      "unset KC_CLI_PASSWORD",
      "rm -f -- \"$state_dir/auth.out\"",
      "if [ \"$auth_status\" -ne 0 ]; then",
      "if [ -e \"$auth_config\" ] || [ -L \"$auth_config\" ]; then",
      "settle_failed_admin_session \"$auth_config\" \"$state_dir\" || return 70",
      "rm -f -- \"$auth_config\" || return 70",
      "[ ! -e \"$auth_config\" ] && [ ! -L \"$auth_config\" ] || return 70",
      "fi",
      "fi",
      "return \"$auth_status\"",
      "}",
    ],
    "administrative authentication session lifecycle drifted",
  );
  exactFunction(
    "try_prove_exact_permanent_authority() {",
    "\nprove_exact_permanent_authority() {",
    [
      "try_prove_exact_permanent_authority() {",
      "inspector_config=$1",
      "inspector_user_id=$2",
      "try_admin_to_file \"$inspector_config\" \"$state_dir/direct-role-mapping.json\" \\",
      "get \"users/$inspector_user_id/role-mappings\" -r master || return",
      "try_project_to_file \"$state_dir/direct-role-mapping.out\" \\",
      "direct-role-mapping \"$state_dir/direct-role-mapping.json\" || return",
      "target_client_line=",
      "audit_client_line=",
      "audit_role_line=",
      "exec 3< \"$state_dir/direct-role-mapping.out\"",
      "IFS= read -r target_client_line <&3 || [ -n \"$target_client_line\" ]",
      "IFS= read -r audit_client_line <&3 || [ -n \"$audit_client_line\" ]",
      "IFS= read -r audit_role_line <&3 || [ -n \"$audit_role_line\" ]",
      "if IFS= read -r extra_client_line <&3; then",
      "exec 3<&-",
      "return 1",
      "fi",
      "exec 3<&-",
      "case \"$target_client_line\" in",
      "target-client=*) target_client_id=${target_client_line#target-client=} ;;",
      "*) return 1 ;;",
      "esac",
      "case \"$audit_client_line\" in",
      "audit-client=*) audit_client_id=${audit_client_line#audit-client=} ;;",
      "*) return 1 ;;",
      "esac",
      "case \"$audit_role_line\" in",
      "audit-role=*) audit_role_id=${audit_role_line#audit-role=} ;;",
      "*) return 1 ;;",
      "esac",
      "is_uuid \"$target_client_id\" && is_uuid \"$audit_client_id\" && \\",
      "is_uuid \"$audit_role_id\" && \\",
      "[ \"$target_client_id\" != \"$audit_client_id\" ] && \\",
      "[ \"$target_client_id\" != \"$audit_role_id\" ] && \\",
      "[ \"$audit_client_id\" != \"$audit_role_id\" ] || return",
      "try_admin_to_file \"$inspector_config\" \"$state_dir/permanent-groups-verify.json\" \\",
      "get \"users/$inspector_user_id/groups\" -r master \\",
      "-q first=0 -q max=1 -q briefRepresentation=true || return",
      "try_project_quiet permanent-groups-empty empty-array \\",
      "\"$state_dir/permanent-groups-verify.json\" || return",
      "try_admin_to_file \"$inspector_config\" \"$state_dir/effective-realm-roles.json\" \\",
      "get \"users/$inspector_user_id/role-mappings/realm/composite\" -r master \\",
      "-q briefRepresentation=false || return",
      "try_project_quiet effective-realm-roles-empty empty-array \\",
      "\"$state_dir/effective-realm-roles.json\" || return",
      "try_admin_to_file \"$inspector_config\" \"$state_dir/effective-target-roles.json\" \\",
      "get \"users/$inspector_user_id/role-mappings/clients/$target_client_id/composite\" \\",
      "-r master -q briefRepresentation=false || return",
      "try_project_quiet effective-target-roles effective-roles \\",
      "\"$state_dir/effective-target-roles.json\" \"$target_client_id\" || return",
      "try_admin_to_file \"$inspector_config\" \"$state_dir/effective-audit-role.json\" \\",
      "get \"users/$inspector_user_id/role-mappings/clients/$audit_client_id/composite\" \\",
      "-r master -q briefRepresentation=false || return",
      "try_project_quiet effective-audit-role effective-audit-role \\",
      "\"$state_dir/effective-audit-role.json\" \"$audit_client_id\" \\",
      "\"$audit_role_id\"",
      "}",
    ],
    "permanent convergence authority projection body drifted",
  );

  exactFunction(
    "quarantine() {",
    "\nquarantine_with_fresh_authority() {",
    [
      "quarantine() {",
      "quarantine_candidate=1",
      "while [ \"$quarantine_candidate\" -le 2 ]; do",
      "case \"$quarantine_candidate\" in",
      "1)",
      "quarantine_config=$state_dir/quarantine-permanent.config",
      "quarantine_username=$convergence_username",
      "quarantine_password=$convergence_password",
      "quarantine_client=$authentication_client",
      ";;",
      "2)",
      "[ \"$bootstrap_authenticated\" = true ] && \\",
      "[ -f \"$bootstrap_config\" ] && \\",
      "[ \"${bootstrap_password+x}\" = x ] || {",
      "quarantine_candidate=$((quarantine_candidate + 1))",
      "continue",
      "}",
      "quarantine_config=$state_dir/quarantine-bootstrap.config",
      "quarantine_username=$bootstrap_username",
      "quarantine_password=$bootstrap_password",
      "quarantine_client=admin-cli",
      ";;",
      "esac",
      "quarantine_succeeded=false",
      "if cleanup_authenticate \"$quarantine_config\" \"$quarantine_username\" \\",
      "\"$quarantine_password\" \"$quarantine_client\" \"$state_dir\"; then",
      "if run_admin_quarantine \"$quarantine_config\" \"$state_dir\"; then",
      "quarantine_succeeded=true",
      "fi",
      "if close_admin_session \"$quarantine_config\" \"$state_dir\"; then",
      "rm -f -- \"$quarantine_config\" || return 1",
      "[ ! -e \"$quarantine_config\" ] && \\",
      "[ ! -L \"$quarantine_config\" ] || return 1",
      "[ \"$quarantine_succeeded\" = true ] && return 0",
      "fi",
      "fi",
      "quarantine_candidate=$((quarantine_candidate + 1))",
      "done",
      "return 1",
      "}",
    ],
    "authenticated quarantine body drifted",
  );
  exactFunction(
    "quarantine_with_fresh_authority() {",
    "\ncleanup() {",
    [
      "quarantine_with_fresh_authority() {",
      "recovery_dir=$(mktemp -d /tmp/synveda-keycloak-recovery.XXXXXX) || return 1",
      "chmod 700 \"$recovery_dir\"",
      "recovery_config=$recovery_dir/permanent.config",
      "recovery_quarantined=false",
      "recovery_session_closed=false",
      "if cleanup_authenticate \"$recovery_config\" \"$convergence_username\" \\",
      "\"$convergence_password\" \"$authentication_client\" \"$recovery_dir\"; then",
      "if run_admin_quarantine \"$recovery_config\" \"$recovery_dir\"; then",
      "recovery_quarantined=true",
      "fi",
      "if close_admin_session \"$recovery_config\" \"$recovery_dir\"; then",
      "recovery_session_closed=true",
      "fi",
      "fi",
      "recovery_cleanup=true",
      "rm -f -- \"$recovery_dir\"/* || recovery_cleanup=false",
      "rmdir -- \"$recovery_dir\" 2>/dev/null || recovery_cleanup=false",
      "[ \"$recovery_quarantined\" = true ] && \\",
      "[ \"$recovery_session_closed\" = true ] && \\",
      "[ \"$recovery_cleanup\" = true ]",
      "}",
    ],
    "fresh-authority quarantine body drifted",
  );

  requireOrder(
    [
      'permanent_user_id=$(query_user_id "$bootstrap_config" "$convergence_username")',
      'admin_quiet "$bootstrap_config" convergence-roles-add add-roles',
      'admin_quiet "$bootstrap_config" convergence-audit-role-add add-roles',
      'prove_exact_permanent_authority "$bootstrap_config" "$permanent_user_id"',
      'authenticate "$permanent_config" "$convergence_username" "$convergence_password"',
      '"$permanent_config" "$permanent_user_id" "$bootstrap_user_id"',
      'write_realm_witness "$bootstrap_config" pending "$realm_json"',
      'project_quiet bootstrap-user-delete admin-bootstrap-delete \\',
    ],
    "permanent authority is not fully projected before bootstrap retirement",
  );
  requireOrder(
    [
      'project_quiet bootstrap-user-delete admin-bootstrap-delete \\',
      '"$bootstrap_config" "$bootstrap_user_id"',
      'close_admin_session "$bootstrap_config" "$state_dir" || {',
      'rm -f -- "$bootstrap_config"',
      "bootstrap_authenticated=false",
      'if authenticate "$state_dir/bootstrap-recheck.config"',
      "prove_bootstrap_refused || {",
      'prove_scoped_authority "$permanent_config" "$permanent_user_id" retired',
      'admin_to_file "$permanent_config" "$realm_json" retirement-target-read',
      'write_realm_witness "$permanent_config" complete',
      'prove_scoped_authority "$permanent_config" "$permanent_user_id"',
    ],
    "bootstrap retirement is not exactly deleted and refused before its complete witness",
  );
  requireOrder(
    [
      "# This is deliberately the only mutation that opens authentication.",
      "require_current_generation_or_exit",
      'admin_quiet "$authority_config" realm-enable',
      'admin_to_file "$authority_config" "$realm_json" open-realm-read',
      'project_quiet open-realm realm "$realm_json" true',
      'try_complete_projection "$authority_config" || {',
      "require_current_generation_or_exit",
      "publish_gate_on_exit=true",
    ],
    "realm enablement is not fully proved before publication is authorized",
  );
  if (occurrenceCount(source, "publish_gate_on_exit=true") !== 2) {
    findings.push("realm convergence success paths do not have a closed publication set");
  }
  requireOnce(
    "SynvedaKeycloakProjection bootstrap-refused",
    "bootstrap retirement does not use the exact response probe",
  );
  requireOnce(
    'SynvedaKeycloakProjection admin-authority-login "$scoped_user_id"',
    "scoped authority does not use the subject-bound OpenID proof",
  );
  requireOnce(
    ". /opt/keycloak/bin/synveda-authority-stage",
    "scoped authority does not use the reviewed content-free stage classifier",
  );
  exactFunction(
    "withdraw_public_gate() {",
    "\npublish_public_gate() {",
    [
      "withdraw_public_gate() {",
      '"$generation_gate" withdraw "$generation" >/dev/null 2>&1',
      "}",
    ],
    "public gate withdrawal body drifted",
  );
  exactFunction(
    "publish_public_gate() {",
    "\n\nrequire_current_generation || exit 75",
    [
      "publish_public_gate() {",
      "require_current_generation || return",
      'if ! "$generation_gate" publish "$generation" >/dev/null 2>&1; then',
      "require_current_generation || true",
      "return 1",
      "fi",
      "require_current_generation",
      "}",
    ],
    "atomic public gate publication body drifted",
  );
  exactFunction(
    "try_prove_scoped_authority() {",
    "\nprove_scoped_authority() {",
    [
      "try_prove_scoped_authority() {",
      "scoped_config=$1",
      "scoped_user_id=$2",
      "scoped_bootstrap_user_id=$3",
      "scoped_authority_stage=role-proof",
      "try_prove_exact_permanent_authority \"$scoped_config\" \"$scoped_user_id\" || return",
      "scoped_authority_stage=stored-admin-token",
      "try_project_quiet scoped-token admin-token \"$scoped_config\" || return",
      "scoped_authority_stage=capture-setup",
      "rm -f -- \"$state_dir/scoped-authority.out\" \\",
      "\"$state_dir/scoped-authority.error\" 2>/dev/null || return",
      "[ ! -e \"$state_dir/scoped-authority.out\" ] && \\",
      "[ ! -L \"$state_dir/scoped-authority.out\" ] && \\",
      "[ ! -e \"$state_dir/scoped-authority.error\" ] && \\",
      "[ ! -L \"$state_dir/scoped-authority.error\" ] || return",
      "# The Java helper gives its proof phase an absolute",
      "# 34-second bound. Revocation, refresh refusal and the unexpected-success",
      "# cleanup path share a six-second absolute deadline. This outer bound",
      "# leaves fifteen seconds for JVM startup, bounded parsing and teardown.",
      "if KC_CLI_PASSWORD=$convergence_password \\",
      "SYNVEDA_PROBE_USERNAME=$convergence_username \\",
      "SYNVEDA_PROBE_BOOTSTRAP_USERNAME=$bootstrap_username \\",
      "SYNVEDA_PROBE_ISSUER=$public_auth_url/realms/master \\",
      "/usr/bin/timeout --foreground --signal=TERM --kill-after=1s 55s \\",
      "/usr/bin/java -cp '/opt/keycloak/bin:/opt/keycloak/lib/lib/main/*' \\",
      "SynvedaKeycloakProjection admin-authority-login \"$scoped_user_id\" \\",
      "\"$scoped_bootstrap_user_id\" \\",
      ">\"$state_dir/scoped-authority.out\" \\",
      "2>\"$state_dir/scoped-authority.error\"; then",
      "scoped_status=0",
      "else",
      "scoped_status=$?",
      "fi",
      "synveda_finish_scoped_authority_probe \"$state_dir\" \"$scoped_status\"",
      "}",
    ],
    "scoped convergence authority proof body drifted",
  );
  exactFunction(
    "try_complete_projection() {",
    "\nrequire_current_generation_or_exit\nuser_profile_valid=false",
    [
      "try_complete_projection() {",
      "complete_config=$1",
      "complete_bootstrap_user_id=$bootstrap_user_id",
      "complete_permanent_user_id=$permanent_user_id",
      'try_admin_to_file "$complete_config" "$state_dir/complete-realm.json" \\',
      'get "realms/$realm" || return',
      'try_project_quiet complete-realm realm "$state_dir/complete-realm.json" true \\',
      '"$ssl_value" || return',
      'read_witness_state "$state_dir/complete-realm.json" || return',
      '[ "$witness_state" = complete ] && \\',
      '[ "$witness_bootstrap_user_id" = "$complete_bootstrap_user_id" ] && \\',
      '[ "$witness_permanent_user_id" = "$complete_permanent_user_id" ] || return',
      'try_prove_user_profile "$complete_config" complete || return',
      'try_admin_to_file "$complete_config" "$state_dir/complete-client-query.json" \\',
      'get clients -r "$realm" -q "clientId=$client" || return',
      'try_project_to_file "$state_dir/complete-client-id.out" client-id \\',
      '"$state_dir/complete-client-query.json" "$client" || return',
      "complete_line=",
      'exec 3< "$state_dir/complete-client-id.out"',
      'IFS= read -r complete_line <&3 || [ -n "$complete_line" ]',
      "if IFS= read -r complete_extra <&3; then",
      "exec 3<&-",
      "return 1",
      "fi",
      "exec 3<&-",
      'case "$complete_line" in',
      "client=*) complete_client_id=${complete_line#client=} ;;",
      "*) return 1 ;;",
      "esac",
      'is_uuid "$complete_client_id" || return',
      'try_admin_to_file "$complete_config" "$state_dir/complete-client.json" \\',
      'get "clients/$complete_client_id" -r "$realm" || return',
      'try_project_quiet complete-client client "$state_dir/complete-client.json" \\',
      '"$public_app_url" || return',
      'try_admin_to_file "$complete_config" "$state_dir/complete-mappers.json" \\',
      'get "clients/$complete_client_id/protocol-mappers/models" -r "$realm" || return',
      'try_project_quiet complete-mappers mappers "$state_dir/complete-mappers.json" || return',
      'try_admin_to_file "$complete_config" "$state_dir/complete-groups.json" \\',
      'get groups -r "$realm" -q search=synveda-admins -q exact=false \\',
      "-q briefRepresentation=false || return",
      'try_project_quiet complete-group group "$state_dir/complete-groups.json" || return',
      'complete_group_id=$(projection_value "$state_dir/complete-groups.json" \\',
      "group group) || return",
      'is_uuid "$complete_group_id" || return',
      'try_admin_to_file "$complete_config" \\',
      '"$state_dir/complete-admin-group-role-mappings.json" \\',
      'get "groups/$complete_group_id/role-mappings" -r "$realm" || return',
      'try_project_quiet complete-admin-group-role-mappings empty-role-mapping \\',
      '"$state_dir/complete-admin-group-role-mappings.json" || return',
      'try_admin_to_file "$complete_config" "$state_dir/complete-admin-group-children.json" \\',
      'get "groups/$complete_group_id/children" -r "$realm" \\',
      "-q first=0 -q max=1 || return",
      'try_project_quiet complete-admin-group-children empty-array \\',
      '"$state_dir/complete-admin-group-children.json" || return',
      'try_prove_scoped_authority "$complete_config" \\',
      '"$complete_permanent_user_id" retired || return',
      "}",
    ],
    "complete managed projection proof body drifted",
  );
  return findings;
}

export function canonicalComposeFindings(model, expected) {
  const findings = [];
  const services = model.services ?? {};
  for (const [name, service] of Object.entries(services)) {
    if (service?.stop_signal !== undefined && service.stop_signal !== "SIGTERM") {
      findings.push(`${name} overrides graceful termination with a non-TERM signal`);
    }
    if (service?.scale !== undefined && service.scale !== 1) {
      findings.push(`${name} overrides the single-host replica contract`);
    }
    if (service?.deploy?.replicas !== undefined && service.deploy.replicas !== 1) {
      findings.push(`${name} overrides the single-host deploy replica contract`);
    }
  }
  const expectedTopLevelSecrets = {
    synveda_gateway_database_url: "synveda_gateway_database_url",
    synveda_kms_key: "synveda_kms_key",
    synveda_kms_key_ref: "synveda_kms_key_ref",
    synveda_migrator_database_url: "synveda_migrator_database_url",
    synveda_worker_database_url: "synveda_worker_database_url",
  };
  if (expected.postgres === "bundled") {
    Object.assign(expectedTopLevelSecrets, {
      postgres_owner_password: "postgres_owner_password",
      synveda_gateway_password: "synveda_gateway_password",
      synveda_migrator_password: "synveda_migrator_password",
      synveda_worker_password: "synveda_worker_password",
    });
  }
  if (expected.oidc === "bundled") {
    Object.assign(expectedTopLevelSecrets, {
      postgres_owner_password: "postgres_owner_password",
      keycloak_database_password: "keycloak_database_password",
      keycloak_admin_username: "keycloak_admin_username",
      keycloak_admin_password: "keycloak_admin_password",
      keycloak_convergence_admin_password: "keycloak_convergence_admin_password",
    });
  }
  if (expected.demo === true) {
    Object.assign(expectedTopLevelSecrets, {
      keycloak_demo_admin_password: "keycloak_demo_admin_password",
      keycloak_demo_member_password: "keycloak_demo_member_password",
    });
  }
  if (expected.runtime === "reference") {
    Object.assign(expectedTopLevelSecrets, {
      synveda_tls_cert: "tls_cert",
      synveda_tls_key: "tls_key",
    });
  }
  if (
    JSON.stringify(keys(model.secrets)) !==
    JSON.stringify(sorted(Object.keys(expectedTopLevelSecrets)))
  ) {
    findings.push("top-level secret set differs from the selected provider row");
  }
  const secretParents = new Set();
  for (const [name, filename] of Object.entries(expectedTopLevelSecrets)) {
    const file = model.secrets?.[name]?.file;
    if (typeof file !== "string" || !file.endsWith(`/${filename}`)) {
      findings.push(`${name} secret file source drifted`);
    } else {
      secretParents.add(file.slice(0, -(filename.length + 1)));
    }
  }
  if (secretParents.size !== 1) {
    findings.push("Compose secrets do not resolve from one selected private directory");
  }
  const expectedServices = [
    "database-preflight",
    "gateway",
    "issuer-diagnostic",
    "migrate",
    "otel-collector",
    "proxy",
    "tenant-convergence",
    "worker",
  ];
  if (expected.postgres === "bundled") expectedServices.push("database-bootstrap", "postgres");
  if (expected.oidc === "bundled") {
    expectedServices.push(
      "keycloak",
      "keycloak-database-bootstrap",
      "keycloak-realm-convergence",
    );
  }
  if (JSON.stringify(keys(services)) !== JSON.stringify(sorted(expectedServices))) {
    findings.push("service set does not match the selected provider row");
  }
  if (keys(model.configs).length !== 0) {
    findings.push("top-level Compose configs entered the closed deployment contract");
  }

  for (const [name, service] of Object.entries(services)) {
    findings.push(...hardeningFindings(name, service));
    if (
      CONTAINER_PROXY_ENVIRONMENT.some(
        (key) => service?.environment?.[key] !== "",
      )
    ) {
      findings.push(`${name} ambient proxy environment is not closed`);
    }
    if ((service.configs ?? []).length > 0) {
      findings.push(`${name} mounts an unreviewed Compose config`);
    }
    if (service.profiles !== undefined) findings.push(`${name} is unexpectedly profile-gated`);
    if (name !== "postgres" && service.user !== expected.runtimeUser) {
      findings.push(`${name} runtime UID:GID differs from the validated secret owner`);
    }
  }

  const ports = publishedPorts(model);
  if (ports.some(({ service }) => service !== "proxy")) {
    findings.push("a non-proxy service publishes a host port");
  }
  const portShape = ports.map(({ host_ip, published, target }) => ({
    host_ip: host_ip ?? null,
    published: String(published),
    target,
  }));
  const expectedPorts =
    expected.runtime === "development"
      ? [
          {
            host_ip: "127.0.0.1",
            published: String(expected.publicPort),
            target: expected.publicPort,
          },
        ]
      : [
          { host_ip: null, published: "80", target: 80 },
          { host_ip: null, published: "443", target: 443 },
        ];
  if (JSON.stringify(portShape) !== JSON.stringify(expectedPorts)) {
    findings.push("proxy port exposure does not match the runtime mode");
  }

  const product = [
    services["database-preflight"],
    services.gateway,
    services["issuer-diagnostic"],
    services["tenant-convergence"],
    services.worker,
    services.migrate,
  ].filter(Boolean);
  if (new Set(product.map(({ image }) => image)).size !== 1) {
    findings.push(
      "database preflight, migration, tenant convergence, issuer diagnostic, gateway and worker do not use one product image",
    );
  }
  const expectedImages = {
    "database-preflight": expected.productImage,
    gateway: expected.productImage,
    "issuer-diagnostic": expected.productImage,
    "tenant-convergence": expected.productImage,
    worker: expected.productImage,
    migrate: expected.productImage,
    proxy: expected.caddyImage,
    "otel-collector": expected.otelCollectorImage,
  };
  if (expected.postgres === "bundled") {
    expectedImages.postgres = expected.postgresImage;
    expectedImages["database-bootstrap"] = expected.postgresImage;
  }
  if (expected.oidc === "bundled") {
    expectedImages["keycloak-database-bootstrap"] = expected.postgresImage;
    expectedImages.keycloak = expected.keycloakImage;
    expectedImages["keycloak-realm-convergence"] = expected.keycloakImage;
  }
  for (const [name, image] of Object.entries(expectedImages)) {
    if (services[name]?.image !== image) findings.push(`${name} image reference drifted`);
  }
  const commands = {
    "database-preflight": ["database-preflight"],
    "issuer-diagnostic": ["issuer-diagnostic"],
    "tenant-convergence": ["tenant-converge"],
    gateway: ["gateway"],
    worker: ["worker"],
    migrate: ["migrate"],
    proxy: ["caddy", "run", "--config", "/etc/caddy/Caddyfile", "--adapter", "caddyfile"],
    "otel-collector": ["--config=/etc/otelcol/config.yaml"],
  };
  if (expected.postgres === "bundled") commands["database-bootstrap"] = ["synveda"];
  if (expected.oidc === "bundled") commands["keycloak-database-bootstrap"] = ["keycloak"];
  if (expected.oidc === "bundled") commands.keycloak = ["start", "--optimized"];
  if (expected.oidc === "bundled") {
    commands["keycloak-realm-convergence"] = ["synveda-realm-supervise"];
  }
  for (const [name, command] of Object.entries(commands)) {
    if (JSON.stringify(services[name]?.command) !== JSON.stringify(command)) {
      findings.push(`${name} command drifted`);
    }
  }
  const expectedEntrypoints = {};
  if (expected.postgres === "bundled") {
    expectedEntrypoints["database-bootstrap"] = [
      "/usr/local/bin/synveda-database-bootstrap",
    ];
  }
  if (expected.oidc === "bundled") {
    expectedEntrypoints["keycloak-database-bootstrap"] = [
      "/usr/local/bin/synveda-database-bootstrap",
    ];
  }
  for (const name of expectedServices) {
    const actual = services[name]?.entrypoint;
    const entrypoint = expectedEntrypoints[name];
    if (
      (entrypoint === undefined && actual !== undefined && actual !== null) ||
      (entrypoint !== undefined &&
        JSON.stringify(actual) !== JSON.stringify(entrypoint))
    ) {
      findings.push(`${name} entrypoint drifted`);
    }
    if (
      services[name]?.post_start !== undefined ||
      services[name]?.pre_stop !== undefined
    ) {
      findings.push(`${name} adds an unreviewed lifecycle hook`);
    }
  }
  const expectedHealthchecks = {
    gateway: {
      test: ["CMD", "/usr/local/bin/synveda-container", "probe", "gateway", "ready"],
      interval: "5s",
      timeout: "3s",
      retries: 24,
    },
    worker: {
      test: ["CMD", "/usr/local/bin/synveda-container", "probe", "worker", "ready"],
      interval: "5s",
      timeout: "3s",
      retries: 24,
    },
    proxy: {
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
    "otel-collector": {
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
  };
  if (expected.postgres === "bundled") {
    expectedHealthchecks.postgres = {
      test: ["CMD-SHELL", "pg_isready -U synveda_owner -d postgres"],
      interval: "5s",
      timeout: "3s",
      retries: 24,
    };
  }
  if (expected.oidc === "bundled") {
    expectedHealthchecks.keycloak = {
      test: ["CMD", "/opt/keycloak/bin/synveda-keycloak-health", "local"],
      interval: "10s",
      timeout: "6s",
      retries: 30,
      start_period: "30s",
    };
    expectedHealthchecks["keycloak-realm-convergence"] = {
      test: ["CMD", "/opt/keycloak/bin/synveda-generation-gate", "ready"],
      interval: "5s",
      timeout: "3s",
      retries: 36,
      start_period: "15s",
    };
  }
  for (const name of expectedServices) {
    const actual = services[name]?.healthcheck;
    const healthcheck = expectedHealthchecks[name];
    if (
      (healthcheck === undefined && actual !== undefined) ||
      (healthcheck !== undefined &&
        (JSON.stringify(keys(actual)) !== JSON.stringify(keys(healthcheck)) ||
          Object.entries(healthcheck).some(
            ([key, value]) => JSON.stringify(actual?.[key]) !== JSON.stringify(value),
          )))
    ) {
      findings.push(`${name} healthcheck drifted`);
    }
  }
  const roleContractTarget = "/etc/synveda/database/roles.json";
  const roleContractSources = new Set();
  for (const name of [
    "database-preflight",
    "gateway",
    "migrate",
    "tenant-convergence",
    "worker",
  ]) {
    if (services[name]?.environment?.SYNVEDA_DATABASE_ROLES_FILE !== roleContractTarget) {
      findings.push(`${name} database role contract setting drifted`);
    }
    if (services[name]?.environment?.SYNVEDA_EXPECTED_DATABASE_ROLE !== undefined) {
      findings.push(`${name} retains the obsolete inferred-role setting`);
    }
    const mount = bindMount(services[name] ?? {}, roleContractTarget);
    if (!mount || mount.read_only !== true) {
      findings.push(`${name} database role contract mount is absent or writable`);
    } else {
      roleContractSources.add(mount.source);
    }
  }
  if (roleContractSources.size !== 1) {
    findings.push("product phases do not mount one byte-identical database role contract");
  }
  if (services.worker?.stop_grace_period !== "1m25s") {
    findings.push("worker stop grace is shorter than its bounded drain");
  }
  if (services.gateway?.healthcheck?.test?.at(-1) !== "ready") {
    findings.push("gateway health does not use readiness");
  }
  if (services.worker?.healthcheck?.test?.at(-1) !== "ready") {
    findings.push("worker health does not use readiness");
  }
  if (services.gateway?.environment?.SYNVEDA_PUBLIC_URL !== expected.appUrl) {
    findings.push("gateway public URL differs from the selected browser URL");
  }
  if (
    services["issuer-diagnostic"]?.environment?.SYNVEDA_PUBLIC_URL !==
    expected.appUrl
  ) {
    findings.push("issuer diagnostic public URL differs from the selected browser URL");
  }
  const tenantEnvironment = services["tenant-convergence"]?.environment ?? {};
  if (tenantEnvironment.SYNVEDA_BOOTSTRAP_TENANT_ID !== expected.bootstrapTenantId) {
    findings.push("tenant convergence bootstrap tenant ID drifted");
  }
  if (
    tenantEnvironment.SYNVEDA_BOOTSTRAP_TENANT_SLUG !== expected.bootstrapTenantSlug
  ) {
    findings.push("tenant convergence bootstrap tenant slug drifted");
  }
  if (
    tenantEnvironment.SYNVEDA_BOOTSTRAP_TENANT_NAME !== expected.bootstrapTenantName
  ) {
    findings.push("tenant convergence bootstrap tenant name drifted");
  }
  if (
    services["issuer-diagnostic"]?.environment?.SYNVEDA_BOOTSTRAP_TENANT_ID !==
    expected.bootstrapTenantId
  ) {
    findings.push("issuer diagnostic bootstrap tenant ID drifted");
  }
  const expectedInsecureDevelopmentHttp =
    expected.runtime === "development" ? "true" : "false";
  for (const name of ["gateway", "issuer-diagnostic"]) {
    if (
      services[name]?.environment?.SYNVEDA_INSECURE_DEVELOPMENT_HTTP !==
      expectedInsecureDevelopmentHttp
    ) {
      findings.push(`${name} plaintext transport policy drifted`);
    }
  }
  if (services.proxy?.environment?.SYNVEDA_APP_HOST !== expected.appHost) {
    findings.push("proxy virtual hosts differ from the selected browser origins");
  }
  if (
    (expected.oidc === "bundled" &&
      services.proxy?.environment?.SYNVEDA_AUTH_HOST !== expected.authHost) ||
    (expected.oidc === "external" &&
      Object.hasOwn(services.proxy?.environment ?? {}, "SYNVEDA_AUTH_HOST"))
  ) {
    findings.push("proxy identity host differs from the selected OIDC mode");
  }
  const issuerTarget = "/etc/synveda/oidc/issuers.json";
  const issuerSources = new Set();
  for (const name of ["gateway", "issuer-diagnostic", "worker"]) {
    if (services[name]?.environment?.SYNVEDA_OIDC_ISSUERS_FILE !== issuerTarget) {
      findings.push(`${name} issuer-file setting drifted`);
    }
    const mount = bindMount(services[name] ?? {}, issuerTarget);
    if (!mount || mount.read_only !== true) {
      findings.push(`${name} issuer configuration mount is absent or writable`);
    } else {
      issuerSources.add(mount.source);
    }
  }
  if (issuerSources.size !== 1) {
    findings.push("gateway, worker and diagnostic do not mount one issuer configuration");
  }
  const directoryCredentialTarget = "/run/secrets/oidc_directory";
  const directoryCredentialMount = bindMount(
    services.worker ?? {},
    directoryCredentialTarget,
  );
  if (!directoryCredentialMount || directoryCredentialMount.read_only !== true) {
    findings.push("worker OIDC directory credential mount is absent or writable");
  }
  if (
    expected.oidc === "bundled" &&
    services.keycloak?.environment?.KC_HOSTNAME !== expected.authUrl
  ) {
    findings.push("Keycloak hostname differs from the selected browser issuer origin");
  }
  if (expected.oidc === "bundled") {
    const expectedJdbc =
      expected.postgres === "bundled"
        ? "jdbc:postgresql://postgres:5432/keycloak"
        : "jdbc:postgresql://database.compose.example:5432/keycloak";
    if (services.keycloak?.environment?.KC_DB_URL !== expectedJdbc) {
      findings.push("Keycloak database endpoint differs from the selected provider");
    }
    if (
      services.keycloak?.environment?.KC_HOSTNAME_STRICT !== "true" ||
      services.keycloak?.environment?.KC_PROXY_HEADERS !== "xforwarded" ||
      services.keycloak?.environment?.KC_PROXY_TRUSTED_ADDRESSES !==
        `${expected.proxyIdentityAddress}/32`
    ) {
      findings.push("Keycloak hostname/proxy trust contract drifted");
    }
    if (
      services["keycloak-realm-convergence"]?.environment
        ?.SYNVEDA_PUBLIC_APP_URL !== expected.appUrl ||
      services["keycloak-realm-convergence"]?.environment
        ?.SYNVEDA_PUBLIC_AUTH_URL !== expected.authUrl ||
      services["keycloak-realm-convergence"]?.environment
        ?.SYNVEDA_KEYCLOAK_SSL_REQUIRED !==
        (expected.runtime === "development" ? "NONE" : "EXTERNAL")
    ) {
      findings.push("Keycloak realm convergence runtime contract drifted");
    }
  }
  const expectedDatabaseEndpoint =
    expected.postgres === "bundled"
      ? { host: "postgres", port: "5432", database: "synveda" }
      : expected.oidc === "bundled"
        ? { host: "database.compose.example", port: "5432", database: "synveda" }
        : undefined;
  const preflightEnvironment = services["database-preflight"]?.environment ?? {};
  if (
    expectedDatabaseEndpoint !== undefined &&
    (preflightEnvironment.SYNVEDA_DATABASE_EXPECTED_HOST !== expectedDatabaseEndpoint.host ||
      preflightEnvironment.SYNVEDA_DATABASE_EXPECTED_PORT !== expectedDatabaseEndpoint.port ||
      preflightEnvironment.SYNVEDA_DATABASE_EXPECTED_NAME !== expectedDatabaseEndpoint.database)
  ) {
    findings.push("database target preflight is not bound to the selected PostgreSQL endpoint");
  }
  if (
    expectedDatabaseEndpoint === undefined &&
    (preflightEnvironment.SYNVEDA_DATABASE_EXPECTED_HOST !== undefined ||
      preflightEnvironment.SYNVEDA_DATABASE_EXPECTED_PORT !== undefined ||
      preflightEnvironment.SYNVEDA_DATABASE_EXPECTED_NAME !== undefined)
  ) {
    findings.push("database target preflight has an unexpected endpoint binding");
  }
  if (
    preflightEnvironment.SYNVEDA_DATABASE_REQUIRED_PEER !==
    (expected.oidc === "bundled" ? "keycloak" : undefined)
  ) {
    findings.push("database target preflight peer requirement differs from the OIDC topology");
  }
  for (const processName of ["gateway", "worker"]) {
    if (
      services[processName]?.environment?.SYNVEDA_DATABASE_REQUIRED_PEER !==
      (expected.oidc === "bundled" ? "keycloak" : undefined)
    ) {
      findings.push(
        `${processName} runtime peer requirement differs from the OIDC topology`,
      );
    }
  }
  if (
    preflightEnvironment.SYNVEDA_DATABASE_PEER_WITNESS_FILE !==
    (expected.oidc === "bundled"
      ? "/run/synveda/database-authority/keycloak-cluster.json"
      : undefined)
  ) {
    findings.push("database target preflight witness differs from the OIDC topology");
  }
  const authorityTarget = "/run/synveda/database-authority";
  const authorityMounts = Object.entries(services)
    .map(([name, service]) => ({ name, mount: bindMount(service, authorityTarget) }))
    .filter(({ mount }) => mount !== undefined);
  if (expected.oidc === "bundled") {
    const writer = authorityMounts.find(
      ({ name }) => name === "keycloak-database-bootstrap",
    )?.mount;
    const reader = authorityMounts.find(({ name }) => name === "database-preflight")?.mount;
    if (
      authorityMounts.length !== 2 ||
      !writer ||
      writer.read_only === true ||
      !reader ||
      reader.read_only !== true ||
      writer.source !== reader.source
    ) {
      findings.push(
        "database authority witness must have one shared read-write producer and read-only preflight mount",
      );
    }
  } else if (authorityMounts.length !== 0) {
    findings.push("external OIDC topology unexpectedly mounts database authority state");
  }
  const publicGateTarget = "/run/synveda/keycloak-public-gate";
  const publicGateMounts = Object.entries(services)
    .map(([name, service]) => ({ name, mount: bindMount(service, publicGateTarget) }))
    .filter(({ mount }) => mount !== undefined);
  if (expected.oidc === "bundled") {
    const proxyGate = publicGateMounts.find(({ name }) => name === "proxy")?.mount;
    const keycloakGate = publicGateMounts.find(({ name }) => name === "keycloak")?.mount;
    const convergenceGate = publicGateMounts.find(
      ({ name }) => name === "keycloak-realm-convergence",
    )?.mount;
    if (
      publicGateMounts.length !== 3 ||
      !proxyGate ||
      proxyGate.read_only !== true ||
      !keycloakGate ||
      keycloakGate.read_only === true ||
      !convergenceGate ||
      convergenceGate.read_only === true ||
      new Set(
        [proxyGate, keycloakGate, convergenceGate].map(({ source }) => source),
      ).size !== 1
    ) {
      findings.push(
        "Keycloak public gate must have two shared writers and one read-only proxy reader",
      );
    }
    if (services["keycloak-realm-convergence"]?.stop_grace_period !== "3m30s") {
      findings.push("realm convergence stop grace is shorter than deferred proof and cleanup");
    }
    const [runtimeUid, runtimeGid] = expected.runtimeUser.split(":");
    const keycloakResources = [
      [
        "keycloak",
        services.keycloak,
        512,
        2 * 1024 ** 3,
        2,
        "45s",
        [
          "/tmp:rw,noexec,nosuid,nodev,mode=1777,size=128m",
          `/opt/keycloak/data/tmp:rw,noexec,nosuid,nodev,mode=0700,size=128m,uid=${runtimeUid},gid=${runtimeGid}`,
        ],
      ],
      [
        "keycloak-realm-convergence",
        services["keycloak-realm-convergence"],
        256,
        512 * 1024 ** 2,
        1,
        "3m30s",
        [
          `/tmp:rw,noexec,nosuid,nodev,mode=0700,size=64m,uid=${runtimeUid},gid=${runtimeGid}`,
        ],
      ],
    ];
    for (const [name, service, pids, memory, cpus, grace, tmpfs] of keycloakResources) {
      if (
        service?.pids_limit !== pids ||
        normalizedByteSize(service?.mem_limit) !== memory ||
        Number(service?.cpus) !== cpus ||
        service?.stop_grace_period !== grace ||
        JSON.stringify(sorted(service?.tmpfs ?? [])) !== JSON.stringify(sorted(tmpfs))
      ) {
        findings.push(`${name} resource and private-tmpfs boundary drifted`);
      }
    }
    for (const name of ["keycloak", "keycloak-realm-convergence"]) {
      if (
        services[name]?.environment?.SYNVEDA_KEYCLOAK_PUBLIC_GATE_PATH !==
        publicGateTarget
      ) {
        findings.push(`${name} generation-gate path drifted`);
      }
    }
  } else if (publicGateMounts.length !== 0) {
    findings.push("external OIDC topology unexpectedly mounts the Keycloak public gate");
  }

  const expectedBindTargets = Object.fromEntries(
    expectedServices.map((name) => [name, []]),
  );
  expectedBindTargets.proxy = [
    "/etc/caddy/Caddyfile:ro",
    "/etc/caddy/app.caddy:ro",
    "/etc/caddy/identity.caddy:ro",
  ];
  expectedBindTargets["database-preflight"] = [
    "/etc/synveda/database/roles.json:ro",
  ];
  expectedBindTargets.migrate = ["/etc/synveda/database/roles.json:ro"];
  expectedBindTargets["tenant-convergence"] = [
    "/etc/synveda/database/roles.json:ro",
  ];
  expectedBindTargets["issuer-diagnostic"] = [
    "/etc/synveda/oidc/issuers.json:ro",
  ];
  expectedBindTargets.gateway = [
    "/etc/synveda/database/roles.json:ro",
    "/etc/synveda/oidc/issuers.json:ro",
  ];
  expectedBindTargets.worker = [
    "/etc/synveda/database/roles.json:ro",
    "/etc/synveda/oidc/issuers.json:ro",
    "/run/secrets/oidc_directory:ro",
  ];
  expectedBindTargets["otel-collector"] = ["/etc/otelcol/config.yaml:ro"];
  if (expected.postgres === "bundled") {
    expectedBindTargets["database-bootstrap"] = [
      "/run/secrets/database_roles.json:ro",
    ];
  }
  if (expected.oidc === "bundled") {
    expectedBindTargets.proxy.push("/run/synveda/keycloak-public-gate:ro");
    expectedBindTargets["database-preflight"].push(
      "/run/synveda/database-authority:ro",
    );
    expectedBindTargets["keycloak-database-bootstrap"] = [
      "/run/secrets/database_roles.json:ro",
      "/run/synveda/database-authority:rw",
    ];
    expectedBindTargets.keycloak = ["/run/synveda/keycloak-public-gate:rw"];
    expectedBindTargets["keycloak-realm-convergence"] = [
      "/run/synveda/keycloak-public-gate:rw",
    ];
  }
  for (const [name, targets] of Object.entries(expectedBindTargets)) {
    if (
      JSON.stringify(bindMountTargets(services[name] ?? {})) !==
      JSON.stringify(sorted(targets))
    ) {
      findings.push(`${name} bind-mount target set or access mode drifted`);
    }
  }
  const expectedNonBindTargets = Object.fromEntries(
    expectedServices.map((name) => [name, []]),
  );
  if (expected.postgres === "bundled") {
    expectedNonBindTargets.postgres = ["volume:/var/lib/postgresql/data:rw"];
  }
  for (const [name, targets] of Object.entries(expectedNonBindTargets)) {
    if (
      JSON.stringify(nonBindMountTargets(services[name] ?? {})) !==
      JSON.stringify(sorted(targets))
    ) {
      findings.push(`${name} non-bind mount set or access mode drifted`);
    }
  }
  const expectedBindSources = {
    proxy: {
      "/etc/caddy/Caddyfile": expected.caddyFile,
      "/etc/caddy/app.caddy": expected.caddyAppConfig,
      "/etc/caddy/identity.caddy": expected.caddyIdentityConfig,
    },
    "database-preflight": {
      "/etc/synveda/database/roles.json": expected.databaseRolesFile,
    },
    migrate: {
      "/etc/synveda/database/roles.json": expected.databaseRolesFile,
    },
    "tenant-convergence": {
      "/etc/synveda/database/roles.json": expected.databaseRolesFile,
    },
    "issuer-diagnostic": {
      "/etc/synveda/oidc/issuers.json": expected.issuerFile,
    },
    gateway: {
      "/etc/synveda/database/roles.json": expected.databaseRolesFile,
      "/etc/synveda/oidc/issuers.json": expected.issuerFile,
    },
    worker: {
      "/etc/synveda/database/roles.json": expected.databaseRolesFile,
      "/etc/synveda/oidc/issuers.json": expected.issuerFile,
      "/run/secrets/oidc_directory": expected.oidcDirectorySecrets,
    },
    "otel-collector": {
      "/etc/otelcol/config.yaml": expected.collectorConfig,
    },
  };
  if (expected.postgres === "bundled") {
    expectedBindSources["database-bootstrap"] = {
      "/run/secrets/database_roles.json": expected.databaseRolesFile,
    };
  }
  if (expected.oidc === "bundled") {
    expectedBindSources.proxy["/run/synveda/keycloak-public-gate"] =
      expected.keycloakPublicGateDir;
    expectedBindSources["database-preflight"]["/run/synveda/database-authority"] =
      expected.databaseAuthorityDir;
    expectedBindSources["keycloak-database-bootstrap"] = {
      "/run/secrets/database_roles.json": expected.databaseRolesFile,
      "/run/synveda/database-authority": expected.databaseAuthorityDir,
    };
    expectedBindSources.keycloak = {
      "/run/synveda/keycloak-public-gate": expected.keycloakPublicGateDir,
    };
    expectedBindSources["keycloak-realm-convergence"] = {
      "/run/synveda/keycloak-public-gate": expected.keycloakPublicGateDir,
    };
  }
  for (const [name, sources] of Object.entries(expectedBindSources)) {
    for (const [target, source] of Object.entries(sources)) {
      if (bindMount(services[name] ?? {}, target)?.source !== source) {
        findings.push(`${name} ${target} bind source drifted`);
      }
    }
  }

  const privatePaths = [];
  if (secretParents.size === 1) {
    privatePaths.push(["secret directory", [...secretParents][0]]);
  }
  for (const source of roleContractSources) {
    privatePaths.push(["database role contract", source]);
  }
  for (const source of issuerSources) {
    privatePaths.push(["issuer contract", source]);
  }
  if (expected.oidc === "bundled") {
    for (const source of new Set(authorityMounts.map(({ mount }) => mount.source))) {
      privatePaths.push(["database authority state", source]);
    }
    for (const source of new Set(publicGateMounts.map(({ mount }) => mount.source))) {
      privatePaths.push(["Keycloak public gate", source]);
    }
  }
  for (let left = 0; left < privatePaths.length; left += 1) {
    for (let right = left + 1; right < privatePaths.length; right += 1) {
      const [leftName, leftPath] = privatePaths[left];
      const [rightName, rightPath] = privatePaths[right];
      if (pathsOverlap(leftPath, rightPath)) {
        findings.push(`${leftName} overlaps ${rightName}`);
      }
    }
  }
  if (services.proxy?.environment?.SYNVEDA_PUBLIC_PORT !== String(expected.publicPort)) {
    findings.push("proxy forwarded port differs from the selected browser port");
  }
  const expectedProxyPorts =
    expected.runtime === "development"
      ? { http: String(expected.publicPort), https: "8443" }
      : { http: "80", https: "443" };
  if (
    services.proxy?.environment?.SYNVEDA_PROXY_HTTP_PORT !== expectedProxyPorts.http ||
    services.proxy?.environment?.SYNVEDA_PROXY_HTTPS_PORT !== expectedProxyPorts.https
  ) {
    findings.push("proxy listener ports differ from the runtime contract");
  }
  if (
    services.gateway?.depends_on?.["tenant-convergence"]?.condition !==
    "service_completed_successfully"
  ) {
    findings.push("gateway does not wait for tenant convergence");
  }
  if (
    services.gateway?.depends_on?.["issuer-diagnostic"]?.condition !==
    "service_completed_successfully"
  ) {
    findings.push("gateway does not wait for issuer diagnostic completion");
  }
  if (services["issuer-diagnostic"]?.depends_on?.proxy?.condition !== "service_healthy") {
    findings.push("issuer diagnostic does not wait for proxy readiness");
  }
  if (
    services["issuer-diagnostic"]?.environment?.SYNVEDA_OIDC_EXPECTED_ISSUER !==
    expected.issuer
  ) {
    findings.push("issuer diagnostic is not bound to the selected exact issuer");
  }
  if (
    services.worker?.depends_on?.["tenant-convergence"]?.condition !==
    "service_completed_successfully"
  ) {
    findings.push("worker does not wait for tenant convergence");
  }
  if (
    services.worker?.depends_on?.["issuer-diagnostic"]?.condition !==
    "service_completed_successfully"
  ) {
    findings.push("worker does not wait for issuer diagnostic completion");
  }
  if (
    services["tenant-convergence"]?.depends_on?.migrate?.condition !==
    "service_completed_successfully"
  ) {
    findings.push("tenant convergence does not wait for migration completion");
  }
  if (
    services.migrate?.depends_on?.["database-preflight"]?.condition !==
    "service_completed_successfully"
  ) {
    findings.push("migration does not wait for database target preflight");
  }
  if (
    expected.postgres === "bundled" &&
    services["database-preflight"]?.depends_on?.["database-bootstrap"]?.condition !==
      "service_completed_successfully"
  ) {
    findings.push("database target preflight does not wait for database bootstrap completion");
  }
  if (
    expected.oidc === "bundled" &&
    services.keycloak?.depends_on?.["keycloak-database-bootstrap"]?.condition !==
      "service_completed_successfully"
  ) {
    findings.push("Keycloak does not wait for database bootstrap completion");
  }
  if (
    expected.oidc === "bundled" &&
    (services["keycloak-realm-convergence"]?.depends_on?.keycloak?.condition !==
      "service_healthy" ||
      services["keycloak-realm-convergence"]?.depends_on?.keycloak?.restart !== undefined)
  ) {
    findings.push("realm supervisor dependency boundary drifted");
  }
  if (
    expected.oidc === "bundled" &&
    services["issuer-diagnostic"]?.depends_on?.["keycloak-realm-convergence"]
      ?.condition !== "service_healthy"
  ) {
    findings.push("issuer diagnostic does not wait for realm convergence");
  }
  if (
    expected.oidc === "bundled" &&
    services.proxy?.depends_on?.["keycloak-realm-convergence"]?.condition !==
      "service_healthy"
  ) {
    findings.push("proxy does not wait for realm convergence");
  }
  if (
    expected.oidc === "bundled" &&
    services["database-preflight"]?.depends_on?.["keycloak-database-bootstrap"]?.condition !==
      "service_completed_successfully"
  ) {
    findings.push("database target preflight does not wait for Keycloak database bootstrap");
  }
  if (
    expected.postgres === "bundled" &&
    expected.oidc === "bundled" &&
    services["keycloak-database-bootstrap"]?.depends_on?.postgres?.condition !==
      "service_healthy"
  ) {
    findings.push("Keycloak database bootstrap does not wait for bundled PostgreSQL readiness");
  }

  const expectedDependencies = Object.fromEntries(
    expectedServices.map((name) => [name, []]),
  );
  expectedDependencies.gateway = [
    "issuer-diagnostic:service_completed_successfully:no-restart:required",
    "tenant-convergence:service_completed_successfully:no-restart:required",
  ];
  expectedDependencies.worker = [
    "issuer-diagnostic:service_completed_successfully:no-restart:required",
    "tenant-convergence:service_completed_successfully:no-restart:required",
  ];
  expectedDependencies.migrate = [
    "database-preflight:service_completed_successfully:no-restart:required",
  ];
  expectedDependencies["tenant-convergence"] = [
    "migrate:service_completed_successfully:no-restart:required",
  ];
  expectedDependencies["issuer-diagnostic"] = [
    "proxy:service_healthy:no-restart:required",
  ];
  if (expected.postgres === "bundled") {
    expectedDependencies["database-bootstrap"] = [
      "postgres:service_healthy:no-restart:required",
    ];
    expectedDependencies["database-preflight"].push(
      "database-bootstrap:service_completed_successfully:no-restart:required",
    );
  }
  if (expected.oidc === "bundled") {
    expectedDependencies["database-preflight"].push(
      "keycloak-database-bootstrap:service_completed_successfully:no-restart:required",
    );
    expectedDependencies.keycloak = [
      "keycloak-database-bootstrap:service_completed_successfully:no-restart:required",
    ];
    expectedDependencies["keycloak-realm-convergence"] = [
      "keycloak:service_healthy:no-restart:required",
    ];
    expectedDependencies["issuer-diagnostic"].push(
      "keycloak-realm-convergence:service_healthy:no-restart:required",
    );
    expectedDependencies.proxy = [
      "keycloak-realm-convergence:service_healthy:no-restart:required",
    ];
  }
  if (expected.postgres === "bundled" && expected.oidc === "bundled") {
    expectedDependencies["keycloak-database-bootstrap"] = [
      "database-bootstrap:service_completed_successfully:no-restart:required",
      "postgres:service_healthy:no-restart:required",
    ];
  }
  for (const [name, dependencies] of Object.entries(expectedDependencies)) {
    if (
      JSON.stringify(dependencyBindings(services[name] ?? {})) !==
      JSON.stringify(sorted(dependencies))
    ) {
      findings.push(`${name} dependency set or restart metadata drifted`);
    }
  }

  const expectedSecrets = Object.fromEntries(expectedServices.map((name) => [name, []]));
  Object.assign(expectedSecrets, {
    "database-preflight": [
      "synveda_gateway_database_url:synveda_gateway_database_url",
      "synveda_migrator_database_url:synveda_migrator_database_url",
      "synveda_worker_database_url:synveda_worker_database_url",
    ],
    gateway: [
      "synveda_gateway_database_url:database_url",
      "synveda_kms_key:kms_key",
      "synveda_kms_key_ref:kms_key_ref",
    ],
    worker: [
      "synveda_kms_key:kms_key",
      "synveda_kms_key_ref:kms_key_ref",
      "synveda_worker_database_url:database_url",
    ],
    migrate: ["synveda_migrator_database_url:database_url"],
    "tenant-convergence": [
      "synveda_kms_key:kms_key",
      "synveda_kms_key_ref:kms_key_ref",
      "synveda_migrator_database_url:database_url",
    ],
  });
  if (expected.postgres === "bundled") {
    expectedSecrets.postgres = ["postgres_owner_password:postgres_owner_password"];
    expectedSecrets["database-bootstrap"] = [
      "postgres_owner_password:postgres_bootstrap_password",
      "synveda_gateway_password:synveda_gateway_password",
      "synveda_migrator_password:synveda_migrator_password",
      "synveda_worker_password:synveda_worker_password",
    ];
    if (expected.oidc === "bundled") {
      expectedSecrets["database-bootstrap"].push(
        "keycloak_database_password:keycloak_database_password",
      );
    }
  }
  if (expected.oidc === "bundled") {
    expectedSecrets["keycloak-database-bootstrap"] = [
      "keycloak_database_password:keycloak_database_password",
      "postgres_owner_password:postgres_bootstrap_password",
    ];
    expectedSecrets.keycloak = [
      "keycloak_admin_password:keycloak_admin_password",
      "keycloak_admin_username:keycloak_admin_username",
      "keycloak_database_password:keycloak_database_password",
    ];
    expectedSecrets["keycloak-realm-convergence"] = [
      "keycloak_admin_password:keycloak_admin_password",
      "keycloak_admin_username:keycloak_admin_username",
      "keycloak_convergence_admin_password:keycloak_convergence_admin_password",
    ];
    if (expected.demo === true) {
      expectedSecrets["keycloak-realm-convergence"].push(
        "keycloak_demo_admin_password:keycloak_demo_admin_password",
        "keycloak_demo_member_password:keycloak_demo_member_password",
      );
    }
  }
  if (expected.runtime === "reference") {
    expectedSecrets.proxy = [
      "synveda_tls_cert:tls_cert",
      "synveda_tls_key:tls_key",
    ];
  }
  for (const [name, bindings] of Object.entries(expectedSecrets)) {
    if (
      JSON.stringify(secretBindings(services[name] ?? {})) !== JSON.stringify(sorted(bindings))
    ) {
      findings.push(`${name} secret mounts are not role-scoped or have drifted targets`);
    }
  }
  if (expected.postgres === "bundled") {
    const bindings = [
      "postgres_owner_password:postgres_bootstrap_password",
      "synveda_gateway_password:synveda_gateway_password",
      "synveda_migrator_password:synveda_migrator_password",
      "synveda_worker_password:synveda_worker_password",
    ];
    if (expected.oidc === "bundled") {
      bindings.push("keycloak_database_password:keycloak_database_password");
    }
    if (
      JSON.stringify(secretBindings(services["database-bootstrap"] ?? {})) !==
      JSON.stringify(sorted(bindings))
    ) {
      findings.push("Synveda database bootstrap secret boundary drifted");
    }
    const requiresKeycloakPassword =
      services["database-bootstrap"]?.environment
        ?.SYNVEDA_DATABASE_REQUIRE_KEYCLOAK_PASSWORD;
    if (
      (expected.oidc === "bundled" && requiresKeycloakPassword !== "true") ||
      (expected.oidc === "external" && requiresKeycloakPassword !== undefined)
    ) {
      findings.push("Synveda database bootstrap credential-set boundary drifted");
    }
  }
  if (expected.oidc === "bundled") {
    const bindings = sorted([
      "keycloak_database_password:keycloak_database_password",
      "postgres_owner_password:postgres_bootstrap_password",
    ]);
    if (
      JSON.stringify(secretBindings(services["keycloak-database-bootstrap"] ?? {})) !==
      JSON.stringify(bindings)
    ) {
      findings.push("Keycloak database bootstrap secret boundary drifted");
    }
    if (
      JSON.stringify(secretBindings(services["keycloak-realm-convergence"] ?? {})) !==
      JSON.stringify(
        sorted([
          "keycloak_admin_password:keycloak_admin_password",
          "keycloak_admin_username:keycloak_admin_username",
          "keycloak_convergence_admin_password:keycloak_convergence_admin_password",
          ...(expected.demo === true
            ? [
                "keycloak_demo_admin_password:keycloak_demo_admin_password",
                "keycloak_demo_member_password:keycloak_demo_member_password",
              ]
            : []),
        ]),
      )
    ) {
      findings.push("Keycloak realm convergence secret boundary drifted");
    }
  }
  const expectedSecretFiles = {
    "database-preflight": {
      SYNVEDA_GATEWAY_DATABASE_URL_FILE: "/run/secrets/synveda_gateway_database_url",
      SYNVEDA_MIGRATOR_DATABASE_URL_FILE: "/run/secrets/synveda_migrator_database_url",
      SYNVEDA_WORKER_DATABASE_URL_FILE: "/run/secrets/synveda_worker_database_url",
    },
    gateway: {
      DATABASE_URL_FILE: "/run/secrets/database_url",
      SYNVEDA_KMS_KEY_FILE: "/run/secrets/kms_key",
      SYNVEDA_KMS_KEY_REF_FILE: "/run/secrets/kms_key_ref",
    },
    worker: {
      DATABASE_URL_FILE: "/run/secrets/database_url",
      SYNVEDA_KMS_KEY_FILE: "/run/secrets/kms_key",
      SYNVEDA_KMS_KEY_REF_FILE: "/run/secrets/kms_key_ref",
    },
    migrate: { DATABASE_URL_FILE: "/run/secrets/database_url" },
    "tenant-convergence": {
      DATABASE_URL_FILE: "/run/secrets/database_url",
      SYNVEDA_KMS_KEY_FILE: "/run/secrets/kms_key",
      SYNVEDA_KMS_KEY_REF_FILE: "/run/secrets/kms_key_ref",
    },
  };
  for (const [name, settings] of Object.entries(expectedSecretFiles)) {
    for (const [setting, path] of Object.entries(settings)) {
      if (services[name]?.environment?.[setting] !== path) {
        findings.push(`${name} ${setting} does not consume its mounted secret target`);
      }
    }
  }
  if (
    expected.postgres === "bundled" &&
    (JSON.stringify(secretBindings(services.postgres ?? {})) !==
      JSON.stringify(["postgres_owner_password:postgres_owner_password"]) ||
      services.postgres?.environment?.POSTGRES_PASSWORD_FILE !==
        "/run/secrets/postgres_owner_password")
  ) {
    findings.push("PostgreSQL secret mount or file setting drifted");
  }
  if (
    expected.oidc === "bundled" &&
    JSON.stringify(secretBindings(services.keycloak ?? {})) !==
      JSON.stringify(
        sorted([
          "keycloak_admin_password:keycloak_admin_password",
          "keycloak_admin_username:keycloak_admin_username",
          "keycloak_database_password:keycloak_database_password",
        ]),
      )
  ) {
    findings.push("Keycloak secret mounts or targets drifted");
  }
  if (
    expected.oidc === "bundled" &&
    (services.keycloak?.environment?.KC_DB_PASSWORD_FILE !==
      "/run/secrets/keycloak_database_password" ||
      services.keycloak?.environment?.KC_BOOTSTRAP_ADMIN_USERNAME_FILE !==
        "/run/secrets/keycloak_admin_username" ||
      services.keycloak?.environment?.KC_BOOTSTRAP_ADMIN_PASSWORD_FILE !==
        "/run/secrets/keycloak_admin_password")
  ) {
    findings.push("Keycloak file-secret settings drifted");
  }
  if (
    expected.oidc === "bundled" &&
    (services["keycloak-realm-convergence"]?.environment
      ?.KC_BOOTSTRAP_ADMIN_USERNAME_FILE !== "/run/secrets/keycloak_admin_username" ||
      services["keycloak-realm-convergence"]?.environment
        ?.KC_BOOTSTRAP_ADMIN_PASSWORD_FILE !== "/run/secrets/keycloak_admin_password" ||
      services["keycloak-realm-convergence"]?.environment
        ?.SYNVEDA_KEYCLOAK_CONVERGENCE_PASSWORD_FILE !==
        "/run/secrets/keycloak_convergence_admin_password" ||
      services["keycloak-realm-convergence"]?.environment
        ?.SYNVEDA_KEYCLOAK_DEMO_ENABLED !== (expected.demo === true ? "true" : undefined) ||
      services["keycloak-realm-convergence"]?.environment
        ?.SYNVEDA_KEYCLOAK_DEMO_ADMIN_PASSWORD_FILE !==
        (expected.demo === true ? "/run/secrets/keycloak_demo_admin_password" : undefined) ||
      services["keycloak-realm-convergence"]?.environment
        ?.SYNVEDA_KEYCLOAK_DEMO_MEMBER_PASSWORD_FILE !==
        (expected.demo === true ? "/run/secrets/keycloak_demo_member_password" : undefined))
  ) {
    findings.push("Keycloak realm convergence file-secret settings drifted");
  }
  if (
    expected.oidc === "bundled" &&
    services.keycloak?.environment?.KC_LOG_LEVEL_ORG_KEYCLOAK_SERVICES !== "warn"
  ) {
    findings.push("Keycloak bootstrap-identifier log suppression drifted");
  }

  const directSecretKeys = new Set([
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
    "SYNVEDA_KEYCLOAK_DEMO_ADMIN_PASSWORD",
    "SYNVEDA_KEYCLOAK_DEMO_MEMBER_PASSWORD",
  ]);
  for (const [name, service] of Object.entries(services)) {
    for (const key of Object.keys(service.environment ?? {})) {
      if (directSecretKeys.has(key)) findings.push(`${name} receives direct secret ${key}`);
    }
  }
  const expectedEnvironmentKeys = {
    proxy: [
      "SYNVEDA_APP_HOST",
      "SYNVEDA_PUBLIC_PORT",
      "SYNVEDA_PROXY_HTTP_PORT",
      "SYNVEDA_PROXY_HTTPS_PORT",
      "XDG_CONFIG_HOME",
      "XDG_DATA_HOME",
    ],
    "database-preflight": [
      "RUST_LOG",
      "SYNVEDA_DATABASE_ROLES_FILE",
      "SYNVEDA_GATEWAY_DATABASE_URL_FILE",
      "SYNVEDA_MIGRATOR_DATABASE_URL_FILE",
      "SYNVEDA_WORKER_DATABASE_URL_FILE",
    ],
    migrate: ["DATABASE_URL_FILE", "RUST_LOG", "SYNVEDA_DATABASE_ROLES_FILE"],
    "tenant-convergence": [
      "DATABASE_URL_FILE",
      "RUST_LOG",
      "SYNVEDA_BOOTSTRAP_TENANT_ID",
      "SYNVEDA_BOOTSTRAP_TENANT_NAME",
      "SYNVEDA_BOOTSTRAP_TENANT_SLUG",
      "SYNVEDA_DATABASE_ROLES_FILE",
      "SYNVEDA_KMS_KEY_FILE",
      "SYNVEDA_KMS_KEY_REF_FILE",
    ],
    "issuer-diagnostic": [
      "RUST_LOG",
      "SYNVEDA_BOOTSTRAP_TENANT_ID",
      "SYNVEDA_INSECURE_DEVELOPMENT_HTTP",
      "SYNVEDA_OIDC_EXPECTED_ISSUER",
      "SYNVEDA_OIDC_ISSUERS_FILE",
      "SYNVEDA_PUBLIC_URL",
    ],
    gateway: [
      "DATABASE_URL_FILE",
      "OTEL_EXPORTER_OTLP_ENDPOINT",
      "RUST_LOG",
      "SYNVEDA_DATABASE_ROLES_FILE",
      "SYNVEDA_KMS_KEY_FILE",
      "SYNVEDA_KMS_KEY_REF_FILE",
      "SYNVEDA_LISTEN_ADDR",
      "SYNVEDA_INSECURE_DEVELOPMENT_HTTP",
      "SYNVEDA_OIDC_ISSUERS_FILE",
      "SYNVEDA_PUBLIC_URL",
    ],
    worker: [
      "DATABASE_URL_FILE",
      "OTEL_EXPORTER_OTLP_ENDPOINT",
      "RUST_LOG",
      "SYNVEDA_DATABASE_ROLES_FILE",
      "SYNVEDA_KMS_KEY_FILE",
      "SYNVEDA_KMS_KEY_REF_FILE",
      "SYNVEDA_OIDC_ISSUERS_FILE",
      "SYNVEDA_WORKER_LISTEN_ADDR",
    ],
    "otel-collector": [],
  };
  if (expected.postgres === "bundled" || expected.oidc === "bundled") {
    expectedEnvironmentKeys["database-preflight"].push(
      "SYNVEDA_DATABASE_EXPECTED_HOST",
      "SYNVEDA_DATABASE_EXPECTED_NAME",
      "SYNVEDA_DATABASE_EXPECTED_PORT",
    );
  }
  if (expected.postgres === "bundled") {
    expectedEnvironmentKeys.postgres = [
      "POSTGRES_DB",
      "POSTGRES_PASSWORD_FILE",
      "POSTGRES_USER",
    ];
    expectedEnvironmentKeys["database-bootstrap"] = [
      "SYNVEDA_DATABASE_ROLES_FILE",
      "SYNVEDA_POSTGRES_BOOTSTRAP_URL",
      "SYNVEDA_POSTGRES_BUNDLED_CLUSTER",
    ];
    if (expected.oidc === "bundled") {
      expectedEnvironmentKeys["database-bootstrap"].push(
        "SYNVEDA_DATABASE_REQUIRE_KEYCLOAK_PASSWORD",
      );
    }
  }
  if (expected.oidc === "bundled") {
    expectedEnvironmentKeys.proxy.push("SYNVEDA_AUTH_HOST");
    expectedEnvironmentKeys["database-preflight"].push(
      "SYNVEDA_DATABASE_PEER_WITNESS_FILE",
      "SYNVEDA_DATABASE_REQUIRED_PEER",
    );
    expectedEnvironmentKeys.gateway.push("SYNVEDA_DATABASE_REQUIRED_PEER");
    expectedEnvironmentKeys.worker.push("SYNVEDA_DATABASE_REQUIRED_PEER");
    expectedEnvironmentKeys["keycloak-database-bootstrap"] = [
      "SYNVEDA_DATABASE_AUTHORITY_DIR",
      "SYNVEDA_DATABASE_ROLES_FILE",
      "SYNVEDA_POSTGRES_BOOTSTRAP_URL",
      "SYNVEDA_POSTGRES_BUNDLED_CLUSTER",
    ];
    expectedEnvironmentKeys.keycloak = [
      "KC_BOOTSTRAP_ADMIN_PASSWORD_FILE",
      "KC_BOOTSTRAP_ADMIN_USERNAME_FILE",
      "KC_CACHE",
      "KC_DB",
      "KC_DB_PASSWORD_FILE",
      "KC_DB_URL",
      "KC_DB_USERNAME",
      "KC_HEALTH_ENABLED",
      "KC_HOSTNAME",
      "KC_HOSTNAME_STRICT",
      "KC_HTTP_ENABLED",
      "KC_HTTP_MANAGEMENT_PORT",
      "KC_HTTP_PORT",
      "KC_LOG_LEVEL_ORG_KEYCLOAK_SERVICES",
      "KC_METRICS_ENABLED",
      "KC_PROXY_HEADERS",
      "KC_PROXY_TRUSTED_ADDRESSES",
      "SYNVEDA_KEYCLOAK_PUBLIC_GATE_PATH",
    ];
    expectedEnvironmentKeys["keycloak-realm-convergence"] = [
      "KC_BOOTSTRAP_ADMIN_PASSWORD_FILE",
      "KC_BOOTSTRAP_ADMIN_USERNAME_FILE",
      "SYNVEDA_KEYCLOAK_CONVERGENCE_PASSWORD_FILE",
      "SYNVEDA_KEYCLOAK_SSL_REQUIRED",
      "SYNVEDA_KEYCLOAK_PUBLIC_GATE_PATH",
      "SYNVEDA_PUBLIC_APP_URL",
      "SYNVEDA_PUBLIC_AUTH_URL",
    ];
    if (expected.demo === true) {
      expectedEnvironmentKeys["keycloak-realm-convergence"].push(
        "SYNVEDA_KEYCLOAK_DEMO_ADMIN_PASSWORD_FILE",
        "SYNVEDA_KEYCLOAK_DEMO_ENABLED",
        "SYNVEDA_KEYCLOAK_DEMO_MEMBER_PASSWORD_FILE",
      );
    }
  }
  for (const expectedKeys of Object.values(expectedEnvironmentKeys)) {
    expectedKeys.push(...CONTAINER_PROXY_ENVIRONMENT);
  }
  for (const name of expectedServices) {
    if (
      JSON.stringify(keys(services[name]?.environment)) !==
      JSON.stringify(sorted(expectedEnvironmentKeys[name] ?? []))
    ) {
      findings.push(`${name} environment key set drifted`);
    }
  }

  const expectedNetworks = {
    "database-preflight": { "synveda-data": {} },
    gateway: {
      "app-backend": {},
      "application-egress": { gw_priority: 1 },
      "synveda-data": {},
      telemetry: {},
    },
    "issuer-diagnostic": {
      "app-backend": {},
      "application-egress": { gw_priority: 1 },
    },
    worker: {
      "application-egress": { gw_priority: 1 },
      "synveda-data": {},
      telemetry: {},
    },
    migrate: { "synveda-data": {} },
    "tenant-convergence": { "synveda-data": {} },
    "otel-collector": {
      "keycloak-management": {},
      telemetry: {},
      "telemetry-egress": { gw_priority: 1 },
    },
    proxy: {
      "app-backend":
        expected.oidc === "bundled" ? { aliases: [expected.authHost] } : {},
      ...(expected.oidc === "bundled"
        ? { "identity-backend": { ipv4_address: expected.proxyIdentityAddress } }
        : {}),
      "public-edge": { gw_priority: 1 },
    },
  };
  if (expected.postgres === "external") {
    expectedNetworks["database-preflight"]["application-egress"] = { gw_priority: 1 };
    expectedNetworks.migrate["application-egress"] = { gw_priority: 1 };
    expectedNetworks["tenant-convergence"]["application-egress"] = { gw_priority: 1 };
  }
  if (expected.postgres === "bundled") {
    expectedNetworks.postgres = { "keycloak-data": {}, "synveda-data": {} };
    expectedNetworks["database-bootstrap"] = { "synveda-data": {} };
  }
  if (expected.oidc === "bundled") {
    expectedNetworks["keycloak-database-bootstrap"] =
      expected.postgres === "external"
        ? { "identity-egress": { gw_priority: 1 }, "keycloak-data": {} }
        : { "keycloak-data": {} };
    expectedNetworks.keycloak = {
      "identity-backend": {},
      "keycloak-data": {},
      "keycloak-management": {},
    };
    expectedNetworks["keycloak-realm-convergence"] = {
      "identity-backend": {},
      "keycloak-management": {},
    };
    if (expected.postgres === "external") {
      expectedNetworks.keycloak["identity-egress"] = { gw_priority: 1 };
    }
  }
  for (const [name, networks] of Object.entries(expectedNetworks)) {
    if (!sameJson(services[name]?.networks, networks)) findings.push(`${name} network boundary drifted`);
  }
  if (expected.oidc === "bundled") {
    if (services.keycloak?.environment?.KC_PROXY_TRUSTED_ADDRESSES !== `${expected.proxyIdentityAddress}/32`) {
      findings.push("bundled Keycloak trusted proxy address drifted");
    }
  }

  if (expected.runtime === "reference") {
    if (Object.values(services).some((service) => service.build !== undefined)) {
      findings.push("reference mode contains a source build");
    }
    const oneShot = new Set([
      "database-bootstrap",
      "database-preflight",
      "issuer-diagnostic",
      "keycloak-database-bootstrap",
      "migrate",
      "tenant-convergence",
    ]);
    for (const [name, service] of Object.entries(services)) {
      if (!oneShot.has(name) && service.restart !== "unless-stopped") {
        findings.push(`${name} lacks the reference restart policy`);
      }
      if (oneShot.has(name) && service.restart !== "no") {
        findings.push(`${name} one-shot restart policy drifted`);
      }
    }
    if (
      JSON.stringify(secretBindings(services.proxy ?? {})) !==
      JSON.stringify(["synveda_tls_cert:tls_cert", "synveda_tls_key:tls_key"])
    ) {
      findings.push("reference proxy lacks certificate-file secrets");
    }
    if (
      JSON.stringify(services.proxy?.sysctls ?? {}) !==
      JSON.stringify({ "net.ipv4.ip_unprivileged_port_start": "80" })
    ) {
      findings.push("reference proxy unprivileged-port boundary drifted");
    }
    if (
      Object.entries(services).some(
        ([name, service]) => name !== "proxy" && service.sysctls !== undefined,
      )
    ) {
      findings.push("a non-proxy service changes kernel namespace settings");
    }
  } else {
    const developmentProductBuilds = [
      "database-preflight",
      "issuer-diagnostic",
      "gateway",
      "worker",
      "migrate",
      "tenant-convergence",
    ];
    for (const name of developmentProductBuilds) {
      if (services[name]?.build?.dockerfile !== "deploy/compose/gateway/Dockerfile") {
        findings.push(`${name} does not use the development product build`);
      }
    }
    if (services.proxy?.build?.dockerfile !== "deploy/compose/proxy/Dockerfile") {
      findings.push("proxy does not use the capability-free development build");
    }
    if (
      expected.postgres === "bundled" &&
      services["database-bootstrap"]?.build?.dockerfile !==
        "deploy/compose/postgres/Dockerfile"
    ) {
      findings.push("bundled PostgreSQL does not use the development provider build");
    }
    if (
      expected.oidc === "bundled" &&
      services.keycloak?.build?.dockerfile !== "deploy/compose/keycloak/Dockerfile"
    ) {
      findings.push("bundled Keycloak does not use the development optimized build");
    }
    if (
      expected.oidc === "bundled" &&
      services["keycloak-database-bootstrap"]?.build?.dockerfile !==
        "deploy/compose/postgres/Dockerfile"
    ) {
      findings.push("Keycloak database bootstrap does not use the development provider build");
    }
    const developmentBuilds = [...developmentProductBuilds, "proxy"];
    if (expected.postgres === "bundled") developmentBuilds.push("database-bootstrap");
    if (expected.oidc === "bundled") {
      developmentBuilds.push("keycloak", "keycloak-database-bootstrap");
    }
    for (const name of developmentBuilds) {
      const buildArguments = services[name]?.build?.args;
      if (
        JSON.stringify(keys(buildArguments)) !==
          JSON.stringify(sorted(CONTAINER_PROXY_ENVIRONMENT)) ||
        CONTAINER_PROXY_ENVIRONMENT.some((key) => buildArguments?.[key] !== "")
      ) {
        findings.push(`${name} ambient proxy build arguments are not closed`);
      }
    }
    if (Object.values(services).some((service) => service.sysctls !== undefined)) {
      findings.push("development mode changes kernel namespace settings");
    }
    if (Object.values(services).some((service) => service.restart !== "no")) {
      findings.push("development mode hides failure behind a restart policy");
    }
  }

  const expectedInternalNetworks = new Set([
    "app-backend",
    "identity-backend",
    "keycloak-data",
    "keycloak-management",
    "synveda-data",
    "telemetry",
  ]);
  const expectedEgressNetworks = new Set([
    "application-egress",
    "identity-egress",
    "public-edge",
    "telemetry-egress",
  ]);
  const expectedNetworkNames = [
    "app-backend",
    "application-egress",
    "keycloak-management",
    "public-edge",
    "synveda-data",
    "telemetry",
    "telemetry-egress",
  ];
  if (expected.postgres === "bundled" || expected.oidc === "bundled") {
    expectedNetworkNames.push("keycloak-data");
  }
  if (expected.oidc === "bundled") {
    expectedNetworkNames.push("identity-backend");
  }
  if (expected.postgres === "external" && expected.oidc === "bundled") {
    expectedNetworkNames.push("identity-egress");
  }
  if (
    JSON.stringify(keys(model.networks)) !==
    JSON.stringify(sorted(expectedNetworkNames))
  ) {
    findings.push("network set differs from the closed deployment contract");
  }
  for (const [name, network] of Object.entries(model.networks ?? {})) {
    if (!expectedInternalNetworks.has(name) && !expectedEgressNetworks.has(name)) {
      findings.push(`unknown network ${name} entered the contract`);
      continue;
    }
    const expectedNetwork = {
      name: `${expected.projectName}_${name}`,
      ipam: { config: [expected.networkPlan[name]] },
      labels: {
        "com.synveda.contract": "cpr-45",
        "com.synveda.network": name,
      },
      ...(expectedInternalNetworks.has(name) ? { internal: true } : {}),
    };
    if (!sameJson(network, expectedNetwork)) {
      findings.push(`${name} IPAM, ownership or isolation contract drifted`);
    }
  }
  if (
    JSON.stringify(services["otel-collector"]?.healthcheck?.test) !==
    JSON.stringify([
      "CMD",
      "/otelcol-contrib",
      "validate",
      "--config=/etc/otelcol/config.yaml",
    ])
  ) {
    findings.push("Collector health does not validate its mounted configuration");
  }

  const rendered = JSON.stringify(model);
  if (/rauthy|temporal/i.test(rendered)) findings.push("canonical render contains retired runtime");
  if (rendered.includes(SECRET_SENTINEL)) findings.push("secret content entered Compose output");
  return findings;
}

function render(fixture, expected) {
  const output = join(
    fixture.scratch,
    `${expected.runtime}-${expected.postgres}-${expected.oidc}${expected.demo === true ? "-demo" : ""}.json`,
  );
  const reference = expected.runtime === "reference";
  expected.publicPort = reference ? 443 : (expected.devPort ?? 8080);
  expected.appHost = reference ? "app.compose.example" : "app.synveda.test";
  expected.authHost =
    expected.oidc === "bundled"
      ? reference
        ? "auth.compose.example"
        : "auth.synveda.test"
      : undefined;
  expected.appUrl = reference
    ? "https://app.compose.example"
    : `http://app.synveda.test:${expected.publicPort}`;
  expected.authUrl =
    expected.oidc === "bundled"
      ? reference
        ? "https://auth.compose.example"
        : `http://auth.synveda.test:${expected.publicPort}`
      : undefined;
  expected.projectName = `synveda-${expected.runtime}`;
  expected.bootstrapTenantId = "019b53c0-7c00-7000-8000-000000000045";
  expected.bootstrapTenantSlug = "reference";
  expected.bootstrapTenantName = "Synveda Reference";
  expected.networkPool = "172.30.240.0/24";
  expected.networkPlan = composeNetworkPlan(expected.networkPool);
  expected.proxyIdentityAddress = "172.30.240.2";
  expected.productImage = reference
    ? `registry.compose.example/synveda/product@${DIGEST}`
    : "synveda/product:dev";
  expected.postgresImage = reference
    ? `registry.compose.example/synveda/postgres@${DIGEST}`
    : "synveda/postgres:17.11-dev";
  expected.keycloakImage = reference
    ? `registry.compose.example/synveda/keycloak@${DIGEST}`
    : "synveda/keycloak:26.7.2-dev";
  expected.caddyImage = reference
    ? `registry.compose.example/synveda/proxy@${DIGEST}`
    : "synveda/proxy:2.11.4-dev";
  expected.otelCollectorImage =
    "otel/opentelemetry-collector-contrib:0.159.0@sha256:1f2c54a30e713fac6b3ae77a1ec84010c2007e29ced8ec666214fc2f6739c1cc";
  expected.issuer =
    expected.oidc === "bundled"
      ? `${expected.authUrl}/realms/synveda`
      : "https://external-idp.compose.example/tenant";
  expected.runtimeUser = `${fixture.uid}:${fixture.gid}`;
  expected.caddyFile = join(COMPOSE, "configs/caddy/Caddyfile");
  expected.caddyAppConfig = join(
    COMPOSE,
    `configs/caddy/app.${reference ? "reference" : "dev"}.caddy`,
  );
  expected.caddyIdentityConfig = join(
    COMPOSE,
    expected.oidc === "bundled"
      ? `configs/caddy/identity.${reference ? "reference" : "dev"}.caddy`
      : "configs/caddy/identity.external.caddy",
  );
  expected.collectorConfig = join(COMPOSE, "configs/otel/collector.yaml");
  expected.databaseRolesFile = join(
    COMPOSE,
    "configs/database",
    expected.oidc === "bundled" ? "roles.reference.json" : "roles.external-oidc.json",
  );
  const canonicalScratch = realpathSync(fixture.scratch);
  expected.databaseAuthorityDir = join(
    canonicalScratch,
    `synveda-${expected.runtime}`,
    "database-authority",
  );
  expected.keycloakPublicGateDir = join(
    canonicalScratch,
    `synveda-${expected.runtime}`,
    "keycloak-public-gate",
  );
  writePrivate(
    fixture.issuers,
    JSON.stringify([
      {
        issuer: expected.issuer,
        client_id: "synveda",
        audience: "synveda-api",
        tenant: { static: { tenant_id: expected.bootstrapTenantId } },
        login_scopes: ["openid", "profile", "email"],
      },
    ]),
    fixture,
  );
  const environment = composeEnvironment(fixture, {
    SYNVEDA_COMPOSE_RUNTIME: expected.runtime,
    SYNVEDA_POSTGRES_MODE: expected.postgres,
    SYNVEDA_OIDC_MODE: expected.oidc,
    SYNVEDA_PUBLIC_SCHEME: reference ? "https" : "http",
    SYNVEDA_APP_HOST: expected.appHost,
    ...(expected.authHost === undefined ? {} : { SYNVEDA_AUTH_HOST: expected.authHost }),
    ...(reference ? {} : { SYNVEDA_DEV_HTTP_PORT: String(expected.publicPort) }),
    SYNVEDA_PRODUCT_IMAGE: expected.productImage,
    SYNVEDA_POSTGRES_IMAGE: expected.postgresImage,
    SYNVEDA_KEYCLOAK_IMAGE: expected.keycloakImage,
    SYNVEDA_CADDY_IMAGE: expected.caddyImage,
    SYNVEDA_OTEL_COLLECTOR_IMAGE: expected.otelCollectorImage,
    ...(expected.demo === true ? { SYNVEDA_COMPOSE_PROFILES: "demo" } : {}),
  });
  expected.issuerFile = realpathSync(environment.SYNVEDA_OIDC_ISSUERS_FILE);
  expected.oidcDirectorySecrets = realpathSync(
    join(environment.SYNVEDA_SECRETS_DIR, "oidc-directory"),
  );
  if (expected.oidc === "external") {
    environment.SYNVEDA_OIDC_ISSUER = expected.issuer;
  }
  if (expected.postgres === "external" && expected.oidc === "bundled") {
    environment.SYNVEDA_POSTGRES_BOOTSTRAP_URL =
      "postgresql://bootstrap@database.compose.example:5432/postgres";
    environment.SYNVEDA_KEYCLOAK_DATABASE_URL =
      "jdbc:postgresql://database.compose.example:5432/keycloak";
  }
  const result = spawnSync(WRAPPER, ["config", "--output", output], {
    cwd: ROOT,
    env: environment,
    encoding: "utf8",
  });
  const processOutput = `${result.stdout ?? ""}\n${result.stderr ?? ""}`;
  assert.ok(!processOutput.includes(SECRET_SENTINEL), "wrapper output contains a secret sentinel");
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  return JSON.parse(readFileSync(output, "utf8"));
}

function checkStaticInputs() {
  const canonicalFiles = [
    "compose.yaml",
    "compose.dev.yaml",
    "compose.postgres.dev.yaml",
    "compose.keycloak.dev.yaml",
    "compose.reference.yaml",
    "compose.postgres.yaml",
    "compose.keycloak.yaml",
    "compose.keycloak-postgres.yaml",
    "compose.keycloak-external-postgres.yaml",
    "compose.demo.yaml",
    "compose.external.yaml",
    "compose.external-postgres.yaml",
  ].map((name) => readFileSync(join(COMPOSE, name), "utf8"));
  assert.doesNotMatch(canonicalFiles.join("\n"), /rauthy|temporal/i);
  assert.deepEqual(
    developmentPortBindingFindings(
      readFileSync(join(COMPOSE, "compose.dev.yaml"), "utf8"),
    ),
    [],
    "development proxy port binding drifted",
  );

  for (const relative of KEYCLOAK_SECURITY_CHAIN_SHA256.keys()) {
    assert.deepEqual(
      reviewedKeycloakSourceFindings(
        relative,
        readFileSync(join(COMPOSE, relative), "utf8"),
      ),
      [],
      `${relative} executable-chain input drifted`,
    );
  }
  assert.deepEqual(
    masterClientAuthorityFindings(
      readFileSync(
        join(COMPOSE, "keycloak/SynvedaKeycloakProjection.java"),
        "utf8",
      ),
    ),
    [],
    "Keycloak master-client authority refusal drifted",
  );
  assert.deepEqual(
    authorityCleanupOrderFindings(
      readFileSync(
        join(COMPOSE, "keycloak/SynvedaKeycloakProjection.java"),
        "utf8",
      ),
    ),
    [],
    "Keycloak post-grant cleanup ordering drifted",
  );

  const databaseBootstrap = readFileSync(
    join(COMPOSE, "postgres/synveda-database-bootstrap"),
    "utf8",
  );
  for (const marker of [
    "SYNVEDA_DATABASE_REQUIRE_KEYCLOAK_PASSWORD",
    "validate_distinct_credentials",
    'first_value=$(read_secret database_credential "$first")',
    'candidate_value=$(read_secret database_credential "$candidate")',
    '[ "$first_value" = "$candidate_value" ]',
    "database-bootstrap: database credentials must be pairwise distinct",
  ]) {
    assert.ok(
      databaseBootstrap.includes(marker),
      `database credential boundary is missing ${marker}`,
    );
  }

  const caddy = readFileSync(join(COMPOSE, "configs/caddy/Caddyfile"), "utf8");
  assert.deepEqual(caddyTrustBoundaryFindings(caddy), []);
  assert.equal(
    readFileSync(join(COMPOSE, "configs/caddy/identity.external.caddy"), "utf8"),
    "# External OIDC providers retain their own public identity edge.\n",
  );
  for (const name of ["app.dev.caddy", "app.reference.caddy"]) {
    assert.deepEqual(
      appRouteFindings(readFileSync(join(COMPOSE, `configs/caddy/${name}`), "utf8")),
      [],
      `${name} route drifted`,
    );
  }
  for (const name of ["identity.dev.caddy", "identity.reference.caddy"]) {
    const identity = readFileSync(join(COMPOSE, `configs/caddy/${name}`), "utf8");
    assert.deepEqual(identityGateFindings(identity), [], `${name} gate drifted`);
  }

  const collector = readFileSync(join(COMPOSE, "configs/otel/collector.yaml"), "utf8");
  assert.deepEqual(collectorConfigFindings(collector), []);
  assert.match(collector, /memory_limiter:/);
  assert.match(collector, /batch:/);
  assert.match(collector, /exporters:\n  nop: \{\}/);
  assert.doesNotMatch(collector, /debug:|logging:/);
  assert.doesNotMatch(collector, /^\s+address:/m);

  const keycloak = readFileSync(join(COMPOSE, "keycloak/Dockerfile"), "utf8");
  assert.match(
    keycloak,
    /FROM rust:1\.96\.0-bookworm@sha256:5e2214abe154fe26e39f64488952e5c991eeed1d6d6da7cc8381ae83927f0cfc AS snapshot-builder/,
  );
  assert.match(keycloak, /keycloak:26\.7\.2@sha256:9d1f1b2b/);
  assert.match(keycloak, /KC_DB=postgres/);
  assert.match(keycloak, /KC_HEALTH_ENABLED=true/);
  assert.match(keycloak, /KC_METRICS_ENABLED=true/);
  assert.match(keycloak, /KC_FEATURES_DISABLED=identity-brokering-api,twitter-broker/);
  assert.match(keycloak, /ENV KC_LOG_LEVEL_ORG_KEYCLOAK_SERVICES=warn/);
  assert.match(keycloak, /kc\.sh build/);
  assert.match(
    keycloak,
    /COPY --from=snapshot-builder --chmod=0555 \/opt\/synveda-input-snapshot \/opt\/keycloak\/bin\/synveda-input-snapshot/,
  );
  for (const marker of [
    "COPY --chmod=0555 deploy/compose/keycloak/synveda-generation-gate /opt/keycloak/bin/synveda-generation-gate",
    "COPY --chmod=0555 deploy/compose/keycloak/synveda-generation-gate-self-test /opt/keycloak/bin/synveda-generation-gate-self-test",
    "COPY --chmod=0555 deploy/compose/keycloak/synveda-keycloak-health /opt/keycloak/bin/synveda-keycloak-health",
    "COPY --chmod=0555 deploy/compose/keycloak/synveda-realm-converge /opt/keycloak/bin/synveda-realm-converge",
    "COPY --chmod=0555 deploy/compose/keycloak/synveda-realm-supervise /opt/keycloak/bin/synveda-realm-supervise",
    "RUN /opt/keycloak/bin/synveda-generation-gate-self-test",
    "RUN /opt/keycloak/bin/synveda-keycloak-health self-test",
  ]) {
    assert.ok(keycloak.includes(marker), `Keycloak image omits ${marker}`);
  }
  assert.match(keycloak, /RUN test -x \/usr\/bin\/timeout/);
  assert.match(keycloak, /\/opt\/keycloak\/bin\/synveda-input-snapshot \\\n+\s*\/tmp\/synveda-snapshot-input \/tmp\/synveda-snapshot-output/);
  assert.doesNotMatch(keycloak, /apt-get install[^\n]*(?:gcc|libc6-dev)/);
  assert.doesNotMatch(keycloak, /start-dev|--features[^\n]*preview/);

  const keycloakEntrypoint = readFileSync(
    join(COMPOSE, "keycloak/keycloak-entrypoint"),
    "utf8",
  );
  assert.match(keycloakEntrypoint, /^#!\/bin\/bash -p\n/);
  assert.match(
    keycloakEntrypoint,
    /^unset BASH_ENV ENV CDPATH GLOBIGNORE PS4$/m,
  );
  assert.match(keycloakEntrypoint, /^set \+x\nset -eu$/m);
  for (const marker of [
    "generation_gate=/opt/keycloak/bin/synveda-generation-gate",
    '[ -z "${SYNVEDA_KEYCLOAK_GENERATION+x}" ] || {',
    'case "${1:-}" in',
    '[ "$#" -eq 2 ] && [ "$2" = --optimized ] || {',
    'echo "keycloak-entrypoint: only start --optimized is supported" >&2',
    '"$generation_gate" rotate >/dev/null || {',
    'synveda-realm-supervise|synveda-realm-converge) ;;',
    'echo "keycloak-entrypoint: unsupported command was refused" >&2',
    'if [ "${1:-}" = synveda-realm-supervise ]; then',
    "exec /opt/keycloak/bin/synveda-realm-supervise",
    'if [ "${1:-}" = synveda-realm-converge ]; then',
    '[ "$#" -eq 2 ] || {',
    '"$generation_gate" is-current "$SYNVEDA_KEYCLOAK_GENERATION"',
    '"$generation_gate" withdraw "$SYNVEDA_KEYCLOAK_GENERATION"',
  ]) {
    assert.ok(
      keycloakEntrypoint.includes(marker),
      `Keycloak generation dispatch is missing ${marker}`,
    );
  }
  assert.match(
    keycloakEntrypoint,
    /^exec \/bin\/bash -p -e \/opt\/keycloak\/bin\/kc\.sh "\$@"$/m,
  );
  assert.match(
    keycloakEntrypoint,
    /^PATH=\/opt\/keycloak\/bin:\/usr\/local\/sbin:\/usr\/local\/bin:\/usr\/sbin:\/usr\/bin:\/sbin:\/bin$/m,
  );
  assert.match(keycloakEntrypoint, /rm -f -- "\$secret_snapshot" \|\| cleanup_status=1/);
  assert.match(
    keycloakEntrypoint,
    /cleanup_secret_snapshot \|\| \{\n\s*echo "keycloak-entrypoint: secret snapshot cleanup failed" >&2\n\s*exit 70/,
  );

  const keycloakConvergence = readFileSync(
    join(COMPOSE, "keycloak/synveda-realm-converge"),
    "utf8",
  );
  const keycloakGenerationGate = readFileSync(
    join(COMPOSE, "keycloak/synveda-generation-gate"),
    "utf8",
  );
  const keycloakRealmSupervisor = readFileSync(
    join(COMPOSE, "keycloak/synveda-realm-supervise"),
    "utf8",
  );
  const keycloakHealth = readFileSync(
    join(COMPOSE, "keycloak/synveda-keycloak-health"),
    "utf8",
  );
  assert.deepEqual(
    keycloakGenerationGateFindings(keycloakGenerationGate),
    [],
    "Keycloak generation gate drifted",
  );
  assert.deepEqual(
    keycloakRealmSupervisorFindings(keycloakRealmSupervisor),
    [],
    "Keycloak realm supervisor drifted",
  );
  assert.deepEqual(
    keycloakHealthFindings(keycloakHealth),
    [],
    "Keycloak management health proof drifted",
  );
  assert.deepEqual(
    keycloakConvergenceFindings(keycloakConvergence),
    [],
    "Keycloak convergence lifecycle drifted",
  );

  const product = readFileSync(join(COMPOSE, "gateway/Dockerfile"), "utf8");
  const productBuilds = product
    .split("\n")
    .filter((line) => !line.trimStart().startsWith("#") && /\bcargo build\b/.test(line));
  assert.equal(productBuilds.length, 2);
  for (const build of productBuilds) assert.match(build, /\bcargo build --locked\b/);

  const proxy = readFileSync(join(COMPOSE, "proxy/Dockerfile"), "utf8");
  assert.match(proxy, /caddy:2\.11\.4-alpine@sha256:5f5c8640aae0/);
  assert.match(proxy, /setcap -r \/usr\/bin\/caddy/);
  assert.match(proxy, /test -z "\$\(getcap \/usr\/bin\/caddy\)"/);

  const postgres = readFileSync(join(COMPOSE, "postgres/Dockerfile"), "utf8");
  assert.match(postgres, /postgres:17\.11-bookworm@sha256:051f7b7b/);
  assert.equal(
    postgres.split("postgresql-17-pgvector=0.8.6-1.pgdg12+1").length - 1,
    1,
  );

  const example = readFileSync(join(COMPOSE, ".env.example"), "utf8");
  assert.doesNotMatch(
    example,
    /^(?:DATABASE_URL|POSTGRES_PASSWORD|KC_DB_PASSWORD|SYNVEDA_KMS_KEY|ANTHROPIC_API_KEY)=/m,
  );
}

export function main() {
  checkStaticInputs();
  const fixture = makeComposeFixture();
  try {
    let rows = 0;
    for (const runtime of ["development", "reference"]) {
      for (const [postgres, oidc] of [
        ["bundled", "bundled"],
        ["bundled", "external"],
        ["external", "bundled"],
        ["external", "external"],
      ]) {
        const expected = { runtime, postgres, oidc };
        const first = render(fixture, expected);
        const findings = canonicalComposeFindings(first, expected);
        assert.deepEqual(findings, [], `${runtime}/${postgres}/${oidc}: ${findings.join("; ")}`);
        const second = render(fixture, expected);
        assert.deepEqual(second, first, `${runtime}/${postgres}/${oidc} render is not deterministic`);
        rows += 1;
      }
    }
    const customPortExpected = {
      runtime: "development",
      postgres: "bundled",
      oidc: "bundled",
      devPort: 18083,
    };
    const customPort = render(fixture, customPortExpected);
    const customPortFindings = canonicalComposeFindings(
      customPort,
      customPortExpected,
    );
    assert.deepEqual(
      customPortFindings,
      [],
      `development/custom-port: ${customPortFindings.join("; ")}`,
    );
    const demoExpected = {
      runtime: "development",
      postgres: "bundled",
      oidc: "bundled",
      demo: true,
    };
    const demo = render(fixture, demoExpected);
    const demoFindings = canonicalComposeFindings(demo, demoExpected);
    assert.deepEqual(
      demoFindings,
      [],
      `development/demo: ${demoFindings.join("; ")}`,
    );
    console.log(
      `canonical Compose static shape validates: ${rows}/8 deterministic provider/runtime rows, ` +
        "one bundled demo profile, one exact custom development issuer port, role-scoped file secrets, " +
        "isolated networks, reverse-proxy-only host ports and closed runtime/build proxy injection; " +
        "live clean-start and browser acceptance remain pending",
    );
  } finally {
    rmSync(fixture.scratch, { recursive: true, force: true });
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}
