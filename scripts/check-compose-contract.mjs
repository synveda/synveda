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
  return { scratch, secrets, issuers, ...owner };
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
  return {
    ...environment,
    COMPOSE_DISABLE_ENV_FILE: "0",
    COMPOSE_PROFILES: "ambient-profile-must-be-cleared",
    LC_ALL: "en_US.UTF-8",
    SYNVEDA_RUNTIME_UID: String(fixture.uid),
    SYNVEDA_RUNTIME_GID: String(fixture.gid),
    SYNVEDA_SECRETS_DIR: fixture.secrets,
    SYNVEDA_OIDC_ISSUERS_FILE: fixture.issuers,
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
  const expectedServices = ["gateway", "migrate", "otel-collector", "proxy", "worker"];
  if (expected.postgres === "bundled") expectedServices.push("postgres");
  if (expected.oidc === "bundled") expectedServices.push("keycloak");
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

  const product = [services.gateway, services.worker, services.migrate].filter(Boolean);
  if (new Set(product.map(({ image }) => image)).size !== 1) {
    findings.push("gateway, worker and migration do not use one product image");
  }
  const commands = {
    gateway: ["gateway"],
    worker: ["worker"],
    migrate: ["migrate"],
    proxy: ["caddy", "run", "--config", "/etc/caddy/Caddyfile", "--adapter", "caddyfile"],
    "otel-collector": ["--config=/etc/otelcol/config.yaml"],
  };
  if (expected.oidc === "bundled") commands.keycloak = ["start", "--optimized"];
  for (const [name, command] of Object.entries(commands)) {
    if (JSON.stringify(services[name]?.command) !== JSON.stringify(command)) {
      findings.push(`${name} command drifted`);
    }
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

  const expectedSecrets = {
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
  const expectedSecretFiles = {
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

  const directSecretKeys = new Set([
    "DATABASE_URL",
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
    gateway: ["app-backend", "application-egress", "synveda-data", "telemetry"],
    worker: ["application-egress", "synveda-data", "telemetry"],
    migrate: ["application-egress", "synveda-data"],
    "otel-collector": ["keycloak-management", "telemetry", "telemetry-egress"],
    proxy:
      expected.oidc === "bundled"
        ? ["app-backend", "identity-backend", "public-edge"]
        : ["app-backend", "public-edge"],
  };
  if (expected.postgres === "bundled") {
    expectedNetworks.postgres = ["keycloak-data", "synveda-data"];
  }
  if (expected.oidc === "bundled") {
    expectedNetworks.keycloak = [
      "identity-backend",
      "identity-egress",
      "keycloak-data",
      "keycloak-management",
    ];
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
    for (const [name, service] of Object.entries(services)) {
      if (name !== "migrate" && service.restart !== "unless-stopped") {
        findings.push(`${name} lacks the reference restart policy`);
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
    for (const name of ["gateway", "worker", "migrate"]) {
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
    expectedNetworkNames.push("identity-backend", "identity-egress");
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
      "--config=http://127.0.0.1:13133/",
    ])
  ) {
    findings.push("Collector health does not probe its running private endpoint");
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
    "compose.external.yaml",
  ].map((name) => readFileSync(join(COMPOSE, name), "utf8"));
  assert.doesNotMatch(canonicalFiles.join("\n"), /rauthy|temporal/i);

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
  assert.match(keycloak, /keycloak:26\.7\.2@sha256:9d1f1b2b/);
  assert.match(keycloak, /KC_DB=postgres/);
  assert.match(keycloak, /KC_HEALTH_ENABLED=true/);
  assert.match(keycloak, /KC_METRICS_ENABLED=true/);
  assert.match(keycloak, /KC_FEATURES_DISABLED=identity-brokering-api,twitter-broker/);
  assert.match(keycloak, /kc\.sh build/);
  assert.doesNotMatch(keycloak, /start-dev|--features[^\n]*preview/);

  const proxy = readFileSync(join(COMPOSE, "proxy/Dockerfile"), "utf8");
  assert.match(proxy, /caddy:2\.11\.4-alpine@sha256:5f5c8640aae0/);
  assert.match(proxy, /setcap -r \/usr\/bin\/caddy/);
  assert.match(proxy, /test -z "\$\(getcap \/usr\/bin\/caddy\)"/);

  const postgres = readFileSync(join(COMPOSE, "postgres/Dockerfile"), "utf8");
  assert.match(postgres, /postgres:17\.11-bookworm@sha256:051f7b7b/);

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
