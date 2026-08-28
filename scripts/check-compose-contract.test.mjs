import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import {
  chmodSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

import {
  canonicalComposeFindings,
  collectorConfigFindings,
  composeEnvironment,
  makeComposeFixture,
} from "./check-compose-contract.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const COMPOSE = join(ROOT, "deploy/compose");
const WRAPPER = join(COMPOSE, "scripts/compose.sh");
const GENERATOR = join(COMPOSE, "scripts/generate-secrets.sh");
const KEYCLOAK_ENTRYPOINT = join(COMPOSE, "keycloak/keycloak-entrypoint");

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

function fakeDocker(fixture) {
  const path = join(fixture.scratch, "docker");
  const argumentsFile = join(fixture.scratch, "docker-arguments");
  writeFileSync(
    path,
    `#!/bin/sh
[ -z "\${COMPOSE_PROFILES+x}" ] || exit 97
[ "\${COMPOSE_DISABLE_ENV_FILE:-}" = 1 ] || exit 98
if [ "$1" = compose ] && [ "$2" = version ]; then
  echo 2.24.0
  exit 0
fi
printf '%s\\n' "$@" > "$SYNVEDA_FAKE_DOCKER_ARGUMENTS"
`,
    { mode: 0o700 },
  );
  chmodSync(path, 0o700);
  return { path, argumentsFile };
}

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
      join(COMPOSE, "compose.external.yaml"),
    ]);
    assert.equal(args[args.indexOf("-p") + 1], "synveda-development");
    assert.deepEqual(args.slice(-2), ["config", "--quiet"]);
  } finally {
    rmSync(fixture.scratch, { recursive: true, force: true });
  }
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
      ["SYNVEDA_DEV_HTTP_PORT", "65536", "canonical integer"],
      ["SYNVEDA_IDENTITY_SUBNET", "10.foo.bar.0/24", "canonical private IPv4"],
      ["SYNVEDA_IDENTITY_SUBNET", "172.30.45.0./24", "canonical private IPv4"],
      ["SYNVEDA_IDENTITY_SUBNET", "172.32.0.0/24", "must be private"],
      ["SYNVEDA_IDENTITY_SUBNET", "192.168.001.0/24", "canonical private IPv4"],
      ["SYNVEDA_PROXY_IDENTITY_ADDRESS", "172.30.45.999", "canonical IPv4"],
      ["SYNVEDA_PROXY_IDENTITY_ADDRESS", "172.30.45.2.", "canonical IPv4"],
      ["SYNVEDA_PROXY_IDENTITY_ADDRESS", "172.30.46.2", "configured /24"],
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

test("the selector rejects direct secrets and permissive secret files", () => {
  const fixture = makeComposeFixture();
  try {
    const fake = fakeDocker(fixture);
    let result = spawnSync(WRAPPER, ["config"], {
      cwd: ROOT,
      env: composeEnvironment(fixture, {
        SYNVEDA_DOCKER_BIN: fake.path,
        SYNVEDA_FAKE_DOCKER_ARGUMENTS: fake.argumentsFile,
        DATABASE_URL: "postgres://secret-sentinel",
      }),
      encoding: "utf8",
    });
    assert.equal(result.status, 78);
    assert.doesNotMatch(result.stderr, /secret-sentinel/);
    assert.match(result.stderr, /direct secret setting DATABASE_URL is forbidden/);

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
  const scratch = mkdtempSync(join(tmpdir(), "synveda-secret-generator-"));
  const secrets = join(scratch, "secrets");
  try {
    const firstResult = spawnSync(GENERATOR, [], {
      cwd: ROOT,
      env: { ...process.env, SYNVEDA_SECRETS_DIR: relative(COMPOSE, secrets) },
      encoding: "utf8",
    });
    assert.equal(firstResult.status, 0, firstResult.stderr);
    const first = `${firstResult.stdout}${firstResult.stderr}`;
    const files = readdirSync(secrets).sort();
    assert.equal(files.length, 12);
    for (const name of files) {
      const value = readFileSync(join(secrets, name), "utf8").trim();
      assert.ok(value.length > 0, `${name} is empty`);
      assert.ok(!first.includes(value), `${name} value reached stdout`);
      assert.equal(statSync(join(secrets, name)).mode & 0o777, 0o600);
      assert.match(first, new RegExp(`generated ${name}(?:\\n|$)`));
    }
    assert.equal(statSync(secrets).mode & 0o777, 0o700);

    const before = readFileSync(join(secrets, "synveda_kms_key"), "utf8");
    const refusal = spawnSync(GENERATOR, [], {
      cwd: ROOT,
      env: { ...process.env, SYNVEDA_SECRETS_DIR: relative(COMPOSE, secrets) },
      encoding: "utf8",
    });
    assert.equal(refusal.status, 73);
    assert.match(refusal.stderr, /refusing to overwrite/);
    assert.equal(readFileSync(join(secrets, "synveda_kms_key"), "utf8"), before);

    const forced = spawnSync(GENERATOR, ["--force"], {
      cwd: ROOT,
      env: { ...process.env, SYNVEDA_SECRETS_DIR: relative(COMPOSE, secrets) },
      encoding: "utf8",
    });
    assert.equal(forced.status, 0, forced.stderr);
    const after = readFileSync(join(secrets, "synveda_kms_key"), "utf8");
    assert.notEqual(after, before);
    assert.ok(!`${forced.stdout}${forced.stderr}`.includes(after.trim()));
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("the Keycloak entrypoint reads bounded files and rejects direct ambiguity", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-keycloak-entrypoint-"));
  try {
    const child = join(scratch, "kc.sh");
    writeFileSync(
      child,
      `#!/bin/sh
[ "$KC_DB_PASSWORD" = "$EXPECTED_DB_PASSWORD" ] || exit 91
[ "$KC_BOOTSTRAP_ADMIN_USERNAME" = "$EXPECTED_ADMIN_USERNAME" ] || exit 92
[ "$KC_BOOTSTRAP_ADMIN_PASSWORD" = "$EXPECTED_ADMIN_PASSWORD" ] || exit 93
printf 'keycloak child invoked: %s\\n' "$*"
`,
      { mode: 0o700 },
    );
    chmodSync(child, 0o700);
    const entrypoint = join(scratch, "keycloak-entrypoint");
    writeFileSync(
      entrypoint,
      readFileSync(KEYCLOAK_ENTRYPOINT, "utf8").replace("/opt/keycloak/bin/kc.sh", child),
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
    };
    for (const direct of [
      "KC_DB_PASSWORD",
      "KC_BOOTSTRAP_ADMIN_USERNAME",
      "KC_BOOTSTRAP_ADMIN_PASSWORD",
    ]) {
      delete environment[direct];
    }

    let result = spawnSync(entrypoint, ["show-config"], {
      env: environment,
      encoding: "utf8",
    });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout, "keycloak child invoked: show-config\n");
    for (const value of Object.values(values)) {
      assert.ok(!`${result.stdout}${result.stderr}`.includes(value));
    }

    result = spawnSync(entrypoint, ["show-config"], {
      env: { ...environment, KC_DB_PASSWORD: "cpr45-direct-secret-sentinel" },
      encoding: "utf8",
    });
    assert.equal(result.status, 78);
    assert.match(result.stderr, /direct KC_DB_PASSWORD is forbidden/);
    assert.doesNotMatch(`${result.stdout}${result.stderr}`, /cpr45-direct-secret-sentinel/);

    writeFileSync(files.db, "first-line\nsecond-line\n", { mode: 0o600 });
    result = spawnSync(entrypoint, ["show-config"], { env: environment, encoding: "utf8" });
    assert.equal(result.status, 78);
    assert.match(result.stderr, /must contain one line/);

    writeFileSync(files.db, "é".repeat(3000), { mode: 0o600 });
    result = spawnSync(entrypoint, ["show-config"], { env: environment, encoding: "utf8" });
    assert.equal(result.status, 78);
    assert.match(result.stderr, /exceeds 4096 bytes/);

    writeFileSync(files.db, Buffer.from([0x61, 0x62, 0x63, 0x00, 0x64, 0x65, 0x66]), {
      mode: 0o600,
    });
    result = spawnSync(entrypoint, ["show-config"], { env: environment, encoding: "utf8" });
    assert.equal(result.status, 78);
    assert.match(result.stderr, /contains a NUL byte/);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("model findings reject privilege, port, command and secret regressions", () => {
  const base = {
    services: {
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
        },
        healthcheck: { test: ["ready"] },
        depends_on: { migrate: { condition: "service_completed_successfully" } },
        secrets: [
          { source: "synveda_gateway_database_url", target: "database_url" },
          { source: "synveda_kms_key", target: "kms_key" },
          { source: "synveda_kms_key_ref", target: "kms_key_ref" },
        ],
        networks: {
          "app-backend": {},
          "application-egress": {},
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
        },
        healthcheck: { test: ["ready"] },
        depends_on: { migrate: { condition: "service_completed_successfully" } },
        secrets: [
          { source: "synveda_worker_database_url", target: "database_url" },
          { source: "synveda_kms_key", target: "kms_key" },
          { source: "synveda_kms_key_ref", target: "kms_key_ref" },
        ],
        networks: { "application-egress": {}, "synveda-data": {}, telemetry: {} },
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
        environment: { DATABASE_URL_FILE: "/run/secrets/database_url" },
        secrets: [{ source: "synveda_migrator_database_url", target: "database_url" }],
        networks: { "application-egress": {}, "synveda-data": {} },
        build: { dockerfile: "deploy/compose/gateway/Dockerfile" },
      },
      proxy: {
        command: ["caddy", "run", "--config", "/etc/caddy/Caddyfile", "--adapter", "caddyfile"],
        user: "1:1",
        cap_drop: ["ALL"],
        security_opt: ["no-new-privileges:true"],
        read_only: true,
        init: true,
        pids_limit: 1,
        restart: "no",
        environment: {
          SYNVEDA_PUBLIC_PORT: "8080",
          SYNVEDA_PROXY_HTTP_PORT: "8080",
          SYNVEDA_PROXY_HTTPS_PORT: "8443",
        },
        build: { dockerfile: "deploy/compose/proxy/Dockerfile" },
        ports: [{ host_ip: "127.0.0.1", published: "8080", target: 8080 }],
        networks: { "app-backend": {}, "public-edge": {} },
      },
      "otel-collector": {
        command: ["--config=/etc/otelcol/config.yaml"],
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
            "--config=http://127.0.0.1:13133/",
          ],
        },
        networks: { "keycloak-management": {}, telemetry: {}, "telemetry-egress": {} },
      },
    },
    networks: {
      "app-backend": { internal: true },
      "application-egress": {},
      "keycloak-management": { internal: true },
      "public-edge": {},
      "synveda-data": { internal: true },
      telemetry: { internal: true },
      "telemetry-egress": {},
    },
  };
  base.services.gateway.build = { dockerfile: "deploy/compose/gateway/Dockerfile" };
  base.services.worker.build = { dockerfile: "deploy/compose/gateway/Dockerfile" };
  const expected = {
    runtime: "development",
    postgres: "external",
    oidc: "external",
    appUrl: "http://app.synveda.test:8080",
    authUrl: "http://auth.synveda.test:8080",
    publicPort: 8080,
    runtimeUser: "1:1",
  };
  assert.deepEqual(canonicalComposeFindings(base, expected), []);

  const wiringRegression = structuredClone(base);
  wiringRegression.services.gateway.user = "2345:2346";
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
    wiringFindings.includes("migrate secret mounts are not role-scoped or have drifted targets"),
  );
  assert.ok(
    wiringFindings.includes(
      "migrate DATABASE_URL_FILE does not consume its mounted secret target",
    ),
  );
  assert.ok(
    wiringFindings.includes("Collector health does not probe its running private endpoint"),
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
