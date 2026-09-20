#!/usr/bin/env node
// CPR-45: prepare files only. No Docker client, socket, provider API or network.
import { spawnSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { existsSync, lstatSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const compose = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const fail = (message) => { throw new Error(`prepare: ${message}`); };
function privateFile(file, value, preserve = false) {
  if (existsSync(file)) {
    const stat = lstatSync(file);
    if (!stat.isFile() || stat.isSymbolicLink() || (stat.mode & 0o077) || stat.uid !== process.getuid()) fail("private file permissions refused");
    if (preserve) {
      if (readFileSync(file, "utf8") !== value) fail("persisted identity/configuration differs; restore matching state or choose a new deployment");
      return;
    }
  }
  // A killed preparation must not make the next attempt reuse a partial file.
  const temporary = `${file}.${randomUUID()}.preparing`;
  try {
    writeFileSync(temporary, value, { mode: 0o600, flag: "wx" });
    renameSync(temporary, file);
  } finally {
    rmSync(temporary, { force: true });
  }
}

export function evaluationEnvironment(manifest, options, hostState, uid, gid, composePath) {
  const allowed = ["port", "subnet", "demoAccounts"];
  if (Object.keys(options).some((key) => !allowed.includes(key))) fail("unknown evaluation option; existing infrastructure uses the reference configuration");
  const port = options.port ?? 8080;
  if (!Number.isInteger(port) || port < 1024 || port > 65535 || port === 8443) fail("port must be an integer from 1024 to 65535, excluding 8443");
  const subnet = options.subnet ?? "10.231.60.0/24";
  if (!/^(10\.\d{1,3}\.\d{1,3}|172\.(1[6-9]|2\d|3[01])\.\d{1,3}|192\.168\.\d{1,3})\.0\/24$/.test(subnet) || subnet.split("/")[0].split(".").some((n) => Number(n) > 255)) fail("subnet must be a canonical private IPv4 /24");
  if (options.demoAccounts !== true && options.demoAccounts !== false) fail("demoAccounts must be explicitly true or false");
  for (const key of ["product", "postgres", "keycloak", "proxy", "browser_acceptance"]) {
    if (!/^[\w./:-]+@sha256:[a-f0-9]{64}$/.test(manifest.images?.[key] ?? "")) fail(`missing immutable ${key} image`);
  }
  if (!/^[\w./:-]+@sha256:[a-f0-9]{64}$/.test(manifest.external_images?.otel_collector ?? "")) fail("missing immutable telemetry image");
  const state = `${hostState}/synveda-evaluation`;
  const env = {
    COMPOSE_PROJECT_NAME: "synveda-evaluation",
    SYNVEDA_COMPOSE_RUNTIME: "evaluation",
    SYNVEDA_POSTGRES_MODE: "bundled", SYNVEDA_OIDC_MODE: "bundled",
    SYNVEDA_RUNTIME_UID: uid, SYNVEDA_RUNTIME_GID: gid,
    SYNVEDA_COMPOSE_RESTART_POLICY: "unless-stopped",
    SYNVEDA_APP_HOST: "localhost", SYNVEDA_AUTH_HOST: "localhost",
    SYNVEDA_PUBLIC_PORT: port, SYNVEDA_PROXY_HTTP_PORT: port, SYNVEDA_PROXY_HTTPS_PORT: 8443,
    SYNVEDA_PUBLIC_APP_URL: `http://localhost:${port}`, SYNVEDA_PUBLIC_AUTH_URL: `http://localhost:${port}`,
    SYNVEDA_OIDC_ISSUER: `http://localhost:${port}/realms/synveda`,
    SYNVEDA_INSECURE_DEVELOPMENT_HTTP: "true", SYNVEDA_KEYCLOAK_SSL_REQUIRED: "NONE",
    SYNVEDA_POSTGRES_BUNDLED_CLUSTER: "true",
    SYNVEDA_POSTGRES_BOOTSTRAP_URL: "postgresql://synveda_owner@postgres:5432/postgres",
    SYNVEDA_KEYCLOAK_DATABASE_URL: "jdbc:postgresql://postgres:5432/keycloak",
    SYNVEDA_DATABASE_EXPECTED_HOST: "postgres", SYNVEDA_DATABASE_EXPECTED_PORT: 5432, SYNVEDA_DATABASE_EXPECTED_NAME: "synveda",
    SYNVEDA_BOOTSTRAP_TENANT_ID: "019b53c0-7c00-7000-8000-000000000045",
    SYNVEDA_BOOTSTRAP_TENANT_SLUG: "synveda", SYNVEDA_BOOTSTRAP_TENANT_NAME: "Synveda",
    SYNVEDA_PRODUCT_IMAGE: manifest.images.product, SYNVEDA_POSTGRES_IMAGE: manifest.images.postgres,
    SYNVEDA_KEYCLOAK_IMAGE: manifest.images.keycloak, SYNVEDA_CADDY_IMAGE: manifest.images.proxy,
    SYNVEDA_BROWSER_IMAGE: manifest.images.browser_acceptance,
    SYNVEDA_BROWSER_SECCOMP_PROFILE: `${composePath}/browser/seccomp_profile.json`,
    SYNVEDA_OTEL_COLLECTOR_IMAGE: manifest.external_images.otel_collector,
    SYNVEDA_SECRETS_DIR: `${state}/secrets`, SYNVEDA_OIDC_ISSUERS_FILE: `${state}/issuers.json`,
    SYNVEDA_OIDC_DIRECTORY_SECRETS_DIR: `${state}/secrets/oidc-directory`,
    SYNVEDA_DATABASE_AUTHORITY_DIR: `${state}/database-authority`,
    SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR: `${state}/keycloak-public-gate`,
    SYNVEDA_DATABASE_ROLES_FILE: `${composePath}/configs/database/roles.reference.json`,
    SYNVEDA_CADDY_APP_CONFIG: `${composePath}/configs/caddy/app.evaluation.caddy`,
    SYNVEDA_CADDY_IDENTITY_CONFIG: `${composePath}/configs/caddy/identity.external.caddy`,
    SYNVEDA_RENDER_OTEL_COLLECTOR_CONFIG: `${composePath}/configs/otel/collector.yaml`,
  };
  const prefix = subnet.slice(0, -5);
  for (const [name, offset] of [["IDENTITY", 0], ["PUBLIC_EDGE", 16], ["APP_BACKEND", 32], ["DATA", 48], ["KEYCLOAK_DATA", 64], ["KEYCLOAK_MANAGEMENT", 80], ["TELEMETRY", 96], ["APPLICATION_EGRESS", 112], ["IDENTITY_EGRESS", 128], ["TELEMETRY_EGRESS", 144]]) {
    env[`SYNVEDA_RENDER_${name}_SUBNET`] = `${prefix}.${offset}/28`;
    env[`SYNVEDA_RENDER_${name}_GATEWAY`] = `${prefix}.${offset + 1}`;
  }
  env.SYNVEDA_RENDER_IDENTITY_DYNAMIC_RANGE = `${prefix}.8/29`;
  env.SYNVEDA_RENDER_PROXY_IDENTITY_ADDRESS = `${prefix}.2`;
  return env;
}

export function prepare() {
  const root = lstatSync("/state");
  if (root.isSymbolicLink() || !root.isDirectory() || (root.mode & 0o077) || root.uid !== process.getuid()) fail("state must be a private directory owned by the operator");
  const manifest = JSON.parse(readFileSync("/bundle/environment.json", "utf8"));
  const options = JSON.parse(readFileSync("/bundle/evaluation.json", "utf8"));
  const hostState = process.env.SYNVEDA_HOST_STATE;
  const hostBundle = process.env.SYNVEDA_HOST_BUNDLE;
  for (const value of [hostState, hostBundle]) if (!value || !value.startsWith("/") || /[\s'"$`\\]/.test(value)) fail("deployment paths must be absolute and contain no whitespace or shell metacharacters");
  const env = evaluationEnvironment(manifest, options, hostState, process.getuid(), process.getgid(), `${hostBundle}/deploy/compose`);
  const state = "/state/synveda-evaluation";
  mkdirSync(state, { recursive: true, mode: 0o700 });
  const stat = lstatSync(state);
  if (stat.isSymbolicLink() || !stat.isDirectory() || (stat.mode & 0o077) || stat.uid !== process.getuid()) fail("state must be a private directory owned by the operator");
  privateFile(`${state}/installation.json`, `${JSON.stringify({ port: options.port ?? 8080, subnet: options.subnet ?? "10.231.60.0/24", demoAccounts: options.demoAccounts })}\n`, true);
  const result = spawnSync("sh", [`${compose}/scripts/generate-secrets.sh`, "--if-missing"], {
    stdio: "inherit", timeout: 60_000,
    env: { PATH: process.env.PATH, SYNVEDA_COMPOSE_RUNTIME: "evaluation", SYNVEDA_POSTGRES_MODE: "bundled", SYNVEDA_SECRETS_DIR: `${state}/secrets`, SYNVEDA_DATABASE_AUTHORITY_DIR: `${state}/database-authority`, SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR: `${state}/keycloak-public-gate` },
  });
  if (result.status !== 0) fail("secret preparation failed; preserve state and inspect the preceding non-secret error");
  privateFile(`${state}/issuers.json`, JSON.stringify([{
    issuer: env.SYNVEDA_OIDC_ISSUER,
    discovery_url: "http://keycloak:8080/realms/synveda/.well-known/openid-configuration",
    client_id: "synveda", audience: "synveda-api", algorithms: ["RS256"],
    tenant: { static: { tenant_id: env.SYNVEDA_BOOTSTRAP_TENANT_ID } },
    groups_claim: "groups", login_scopes: ["openid", "profile", "email"],
  }], null, 2) + "\n", true);
  privateFile("/state/evaluation.env", Object.entries(env).map(([key, value]) => `${key}='${value}'`).join("\n") + "\n");
  privateFile("/state/evaluation-files", ["compose.yaml", "compose.postgres.yaml", "compose.keycloak.yaml", "compose.keycloak-postgres.yaml", "compose.evaluation.yaml", ...(options.demoAccounts ? ["compose.demo.yaml"] : [])].join("\n") + "\n");
  console.log(`Prepared ${env.SYNVEDA_PUBLIC_APP_URL}/console/. Secrets are preserved in the private state directory.`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try { prepare(); } catch (error) { console.error(error.message); process.exitCode = 78; }
}
