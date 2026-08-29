import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  chownSync,
  chmodSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const COMPOSE = join(ROOT, "deploy/compose");
const WRAPPER = join(COMPOSE, "scripts/compose.sh");
const DIGEST = `sha256:${"1".repeat(64)}`;
const SECRET_SENTINEL = "cpr45-secret-sentinel";
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
  const secrets = join(scratch, "secrets");
  mkdirSync(secrets, { mode: 0o700 });
  chmodSync(secrets, 0o700);
  if (processUid === 0) chownSync(secrets, owner.uid, owner.gid);
  for (const name of [...CORE_SECRETS, ...PROVIDER_SECRETS, "tls_cert", "tls_key"]) {
    writePrivate(join(secrets, name), `${SECRET_SENTINEL}-${name}`, owner);
  }
  const databaseAuthority = join(scratch, "database-authority");
  mkdirSync(databaseAuthority, { mode: 0o700 });
  chmodSync(databaseAuthority, 0o700);
  if (processUid === 0) chownSync(databaseAuthority, owner.uid, owner.gid);
  const issuers = join(scratch, "issuers.json");
  writePrivate(
    issuers,
    JSON.stringify([
      {
        issuer: "http://auth.synveda.test:8080/realms/synveda",
        client_id: "synveda",
        tenant: { static: { tenant_id: "00000000-0000-0000-0000-000000000001" } },
        login_scopes: ["openid", "profile", "email", "groups"],
      },
    ]),
    owner,
  );
  return { scratch, secrets, issuers, databaseAuthority, ...owner };
}

export function composeEnvironment(fixture, overrides = {}) {
  const environment = { ...process.env };
  for (const name of Object.keys(environment)) {
    if (name.startsWith("SYNVEDA_")) delete environment[name];
  }
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
    SYNVEDA_RUNTIME_UID: String(fixture.uid),
    SYNVEDA_RUNTIME_GID: String(fixture.gid),
    SYNVEDA_SECRETS_DIR: fixture.secrets,
    SYNVEDA_OIDC_ISSUERS_FILE: fixture.issuers,
    SYNVEDA_DATABASE_AUTHORITY_DIR: fixture.databaseAuthority,
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

function secretBindings(service) {
  return sorted((service.secrets ?? []).map(({ source, target }) => `${source}:${target}`));
}

function bindMount(service, target) {
  return (service.volumes ?? []).find(
    (mount) => mount.type === "bind" && mount.target === target,
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
  if (service.network_mode === "host") findings.push(`${name} uses host networking`);
  if (service.pid === "host" || service.ipc === "host") {
    findings.push(`${name} uses a host process namespace`);
  }
  if ((service.volumes ?? []).some((mount) => JSON.stringify(mount).includes("docker.sock"))) {
    findings.push(`${name} mounts the Docker socket`);
  }
  if (JSON.stringify(service.cap_drop ?? []) !== JSON.stringify(["ALL"])) {
    findings.push(`${name} does not drop all capabilities`);
  }
  if (!(service.security_opt ?? []).includes("no-new-privileges:true")) {
    findings.push(`${name} permits privilege escalation`);
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

export function canonicalComposeFindings(model, expected) {
  const findings = [];
  const services = model.services ?? {};
  const expectedServices = [
    "database-preflight",
    "gateway",
    "migrate",
    "otel-collector",
    "proxy",
    "worker",
  ];
  if (expected.postgres === "bundled") expectedServices.push("database-bootstrap", "postgres");
  if (expected.oidc === "bundled") {
    expectedServices.push("keycloak", "keycloak-database-bootstrap");
  }
  if (JSON.stringify(keys(services)) !== JSON.stringify(sorted(expectedServices))) {
    findings.push("service set does not match the selected provider row");
  }

  for (const [name, service] of Object.entries(services)) {
    findings.push(...hardeningFindings(name, service));
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
      ? [{ host_ip: "127.0.0.1", published: "8080", target: 8080 }]
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
    services.worker,
    services.migrate,
  ].filter(Boolean);
  if (new Set(product.map(({ image }) => image)).size !== 1) {
    findings.push("database preflight, gateway, worker and migration do not use one product image");
  }
  const commands = {
    "database-preflight": ["database-preflight"],
    gateway: ["gateway"],
    worker: ["worker"],
    migrate: ["migrate"],
    proxy: ["caddy", "run", "--config", "/etc/caddy/Caddyfile", "--adapter", "caddyfile"],
    "otel-collector": ["--config=/etc/otelcol/config.yaml"],
  };
  if (expected.postgres === "bundled") commands["database-bootstrap"] = ["synveda"];
  if (expected.oidc === "bundled") commands["keycloak-database-bootstrap"] = ["keycloak"];
  if (expected.oidc === "bundled") commands.keycloak = ["start", "--optimized"];
  for (const [name, command] of Object.entries(commands)) {
    if (JSON.stringify(services[name]?.command) !== JSON.stringify(command)) {
      findings.push(`${name} command drifted`);
    }
  }
  const roleContractTarget = "/etc/synveda/database/roles.json";
  const roleContractSources = new Set();
  for (const name of ["database-preflight", "gateway", "migrate", "worker"]) {
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
  if (services.proxy?.environment?.SYNVEDA_PUBLIC_PORT !== String(expected.publicPort)) {
    findings.push("proxy forwarded port differs from the selected browser port");
  }
  const expectedProxyPorts =
    expected.runtime === "development"
      ? { http: "8080", https: "8443" }
      : { http: "80", https: "443" };
  if (
    services.proxy?.environment?.SYNVEDA_PROXY_HTTP_PORT !== expectedProxyPorts.http ||
    services.proxy?.environment?.SYNVEDA_PROXY_HTTPS_PORT !== expectedProxyPorts.https
  ) {
    findings.push("proxy listener ports differ from the runtime contract");
  }
  if (services.gateway?.depends_on?.migrate?.condition !== "service_completed_successfully") {
    findings.push("gateway does not wait for migration completion");
  }
  if (services.worker?.depends_on?.migrate?.condition !== "service_completed_successfully") {
    findings.push("worker does not wait for migration completion");
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

  const expectedSecrets = {
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
  };
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
  ]);
  for (const [name, service] of Object.entries(services)) {
    for (const key of Object.keys(service.environment ?? {})) {
      if (directSecretKeys.has(key)) findings.push(`${name} receives direct secret ${key}`);
    }
  }

  const expectedNetworks = {
    "database-preflight":
      expected.postgres === "external"
        ? ["application-egress", "synveda-data"]
        : ["synveda-data"],
    gateway: ["app-backend", "application-egress", "synveda-data", "telemetry"],
    worker: ["application-egress", "synveda-data", "telemetry"],
    migrate:
      expected.postgres === "external"
        ? ["application-egress", "synveda-data"]
        : ["synveda-data"],
    "otel-collector": ["keycloak-management", "telemetry", "telemetry-egress"],
    proxy:
      expected.oidc === "bundled"
        ? ["app-backend", "identity-backend", "public-edge"]
        : ["app-backend", "public-edge"],
  };
  if (expected.postgres === "bundled") {
    expectedNetworks.postgres = ["keycloak-data", "synveda-data"];
    expectedNetworks["database-bootstrap"] = ["synveda-data"];
  }
  if (expected.oidc === "bundled") {
    expectedNetworks["keycloak-database-bootstrap"] =
      expected.postgres === "external"
        ? ["identity-egress", "keycloak-data"]
        : ["keycloak-data"];
    expectedNetworks.keycloak = [
      "identity-backend",
      "keycloak-data",
      "keycloak-management",
    ];
    if (expected.postgres === "external") expectedNetworks.keycloak.push("identity-egress");
  }
  for (const [name, networks] of Object.entries(expectedNetworks)) {
    if (JSON.stringify(keys(services[name]?.networks)) !== JSON.stringify(sorted(networks))) {
      findings.push(`${name} network boundary drifted`);
    }
  }

  if (expected.runtime === "reference") {
    if (Object.values(services).some((service) => service.build !== undefined)) {
      findings.push("reference mode contains a source build");
    }
    const oneShot = new Set([
      "database-bootstrap",
      "database-preflight",
      "keycloak-database-bootstrap",
      "migrate",
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
    for (const name of ["database-preflight", "gateway", "worker", "migrate"]) {
      if (services[name]?.build?.dockerfile !== "deploy/compose/gateway/Dockerfile") {
        findings.push(`${name} does not use the development product build`);
      }
    }
    if (services.proxy?.build?.dockerfile !== "deploy/compose/proxy/Dockerfile") {
      findings.push("proxy does not use the capability-free development build");
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
    if (expectedInternalNetworks.has(name) && network.internal !== true) {
      findings.push(`${name} is not an internal network`);
    }
    if (expectedEgressNetworks.has(name) && network.internal === true) {
      findings.push(`${name} unexpectedly blocks external dependencies`);
    }
    if (!expectedInternalNetworks.has(name) && !expectedEgressNetworks.has(name)) {
      findings.push(`unknown network ${name} entered the contract`);
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
    `${expected.runtime}-${expected.postgres}-${expected.oidc}.json`,
  );
  const reference = expected.runtime === "reference";
  expected.publicPort = reference ? 443 : 8080;
  expected.appUrl = reference ? "https://app.compose.example" : "http://app.synveda.test:8080";
  expected.authUrl = reference ? "https://auth.compose.example" : "http://auth.synveda.test:8080";
  expected.issuer =
    expected.oidc === "bundled"
      ? `${expected.authUrl}/realms/synveda`
      : "https://external-idp.compose.example/tenant";
  expected.runtimeUser = `${fixture.uid}:${fixture.gid}`;
  writePrivate(
    fixture.issuers,
    JSON.stringify([
      {
        issuer: expected.issuer,
        client_id: "synveda",
        tenant: { static: { tenant_id: "00000000-0000-0000-0000-000000000001" } },
        login_scopes: ["openid", "profile", "email", "groups"],
      },
    ]),
    fixture,
  );
  const environment = composeEnvironment(fixture, {
    SYNVEDA_COMPOSE_RUNTIME: expected.runtime,
    SYNVEDA_POSTGRES_MODE: expected.postgres,
    SYNVEDA_OIDC_MODE: expected.oidc,
    SYNVEDA_PUBLIC_SCHEME: reference ? "https" : "http",
    SYNVEDA_APP_HOST: reference ? "app.compose.example" : "app.synveda.test",
    SYNVEDA_AUTH_HOST: reference ? "auth.compose.example" : "auth.synveda.test",
    SYNVEDA_PRODUCT_IMAGE: reference
      ? `registry.compose.example/synveda/product@${DIGEST}`
      : "synveda/product:dev",
    SYNVEDA_POSTGRES_IMAGE: reference
      ? `registry.compose.example/synveda/postgres@${DIGEST}`
      : "synveda/postgres:17.11-dev",
    SYNVEDA_KEYCLOAK_IMAGE: reference
      ? `registry.compose.example/synveda/keycloak@${DIGEST}`
      : "synveda/keycloak:26.7.2-dev",
    SYNVEDA_CADDY_IMAGE: reference
      ? `registry.compose.example/synveda/proxy@${DIGEST}`
      : "synveda/proxy:2.11.4-dev",
  });
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
    "compose.reference.yaml",
    "compose.postgres.yaml",
    "compose.keycloak.yaml",
    "compose.keycloak-postgres.yaml",
    "compose.external.yaml",
    "compose.external-postgres.yaml",
  ].map((name) => readFileSync(join(COMPOSE, name), "utf8"));
  assert.doesNotMatch(canonicalFiles.join("\n"), /rauthy|temporal/i);

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
  for (const marker of [
    "admin off",
    "-Forwarded",
    "-X-Original-*",
    "-traceparent",
    "-tracestate",
    "-baggage",
    "X-Forwarded-For {remote_host}",
    "X-Forwarded-Port {$SYNVEDA_PUBLIC_PORT}",
    "max_header_size 32KB",
  ]) {
    assert.ok(caddy.includes(marker), `Caddy trust boundary is missing ${marker}`);
  }
  for (const name of ["app.dev.caddy", "app.reference.caddy"]) {
    assert.match(readFileSync(join(COMPOSE, `configs/caddy/${name}`), "utf8"), /handle \/metrics/);
  }
  for (const name of ["identity.dev.caddy", "identity.reference.caddy"]) {
    const identity = readFileSync(join(COMPOSE, `configs/caddy/${name}`), "utf8");
    assert.match(identity, /handle \/realms\/synveda\/\*/);
    assert.match(identity, /handle \/resources\/\*/);
    assert.doesNotMatch(identity, /\/admin|\/realms\/master|\/health|\/metrics/);
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
    console.log(
      `canonical Compose static shape validates: ${rows}/8 deterministic provider/runtime rows, ` +
        "role-scoped file secrets, isolated networks and reverse-proxy-only host ports; " +
        "startup convergence remains gated",
    );
  } finally {
    rmSync(fixture.scratch, { recursive: true, force: true });
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}
