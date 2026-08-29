#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const chart = "deploy/helm/synveda";
const values = `${chart}/ci/lint-values.yaml`;

function render(extraArgs = []) {
  return spawnSync(
    "helm",
    ["template", "synveda", chart, "-f", values, ...extraArgs],
    { encoding: "utf8" },
  );
}

function requireSuccess(name, result) {
  if (result.status !== 0) {
    throw new Error(`${name} failed to render:\n${result.stderr || result.stdout}`);
  }
}

function requireRefusal(name, expected, extraArgs) {
  const result = render(extraArgs);
  if (result.status === 0) {
    throw new Error(`${name} rendered but should have been refused`);
  }
  const output = `${result.stdout}\n${result.stderr}`;
  if (!output.includes(expected)) {
    throw new Error(`${name} failed for the wrong reason:\n${output}`);
  }
}

function documents(source) {
  return source.split(/^---\s*$/m);
}

function resource(source, kind, component) {
  return documents(source).find(
    (document) =>
      new RegExp(`^kind: ${kind}$`, "m").test(document) &&
      document.includes(`app.kubernetes.io/component: ${component}`),
  );
}

function resourceByKind(source, kind) {
  return documents(source).find((document) => new RegExp(`^kind: ${kind}$`, "m").test(document));
}

function namedItem(document, name) {
  const marker = new RegExp(`^ {8}- name: ${name}$`, "m");
  const match = marker.exec(document ?? "");
  if (!match) return undefined;
  const start = match.index;
  const remainder = document.slice(start + match[0].length);
  const next = /\n {8}- name: [^\n]+/m.exec(remainder);
  return next ? document.slice(start, start + match[0].length + next.index) : document.slice(start);
}

function containerImage(document, name) {
  return document?.match(new RegExp(`\\n\\s+- name: ${name}\\n\\s+image: (\\S+)`))?.[1];
}

function requireMarkers(name, document, markers) {
  if (!document) throw new Error(`${name} resource is missing`);
  for (const marker of markers) {
    if (!document.includes(marker)) {
      throw new Error(`${name} is missing ${marker.trim()}`);
    }
  }
}

function forbidMarkers(name, document, markers) {
  for (const marker of markers) {
    if (document?.includes(marker)) {
      throw new Error(`${name} unexpectedly contains ${marker}`);
    }
  }
}

const valid = render();
requireSuccess("minimal chart", valid);

const cluster = resourceByKind(valid.stdout, "Cluster");
const gateway = resource(valid.stdout, "Deployment", "gateway");
const worker = resource(valid.stdout, "Deployment", "worker");
const install = resource(valid.stdout, "Job", "install");
const databaseRoles = resource(valid.stdout, "ConfigMap", "database-contract");
if (!cluster || !gateway || !worker || !install || !databaseRoles) {
  throw new Error("rendered chart must contain Cluster, install Job, gateway and worker resources");
}

const databaseRoleMatch =
  /^  roles\.json: \|\n    ([^\n]*)\n(?!    )/m.exec(databaseRoles);
if (!databaseRoleMatch) {
  throw new Error("database role contract must be one clipped, newline-terminated scalar line");
}
if (
  databaseRoleMatch[1] !==
  '{"migrator":"synveda_migrator","gateway":"synveda_gateway","worker":"synveda_worker","administrators":["postgres"],"administrative_memberships":[],"forbidden_databases":["postgres","template1"],"isolated_peer_roles":[]}'
) {
  throw new Error("database role contract content drifted");
}

requireMarkers("CloudNativePG Cluster", cluster, [
  "database: synveda",
  "owner: synveda_migrator",
  "enableSuperuserAccess: true",
  "revoke connect, temporary on database postgres, template1 from public",
  "alter database synveda allow_connections false",
]);
forbidMarkers("CloudNativePG Cluster", cluster, [
  "postInitApplicationSQL:",
  "create extension if not exists vector",
  "create extension if not exists btree_gin",
]);
const postInit = cluster.indexOf("postInitSQL:");
const publicRevoke = cluster.indexOf(
  "revoke connect, temporary on database postgres, template1 from public",
);
const closedApplicationDatabase = cluster.indexOf(
  "alter database synveda allow_connections false",
);
if (!(postInit >= 0 && postInit < publicRevoke && publicRevoke < closedApplicationDatabase)) {
  throw new Error("CloudNativePG must close maintenance-database PUBLIC ACLs during init");
}

const productImage = containerImage(gateway, "gateway");
if (!productImage || containerImage(worker, "worker") !== productImage) {
  throw new Error("gateway and worker do not use one product image");
}

requireMarkers("gateway", gateway, [
  "automountServiceAccountToken: false",
  "- name: DATABASE_URL_FILE",
  "value: /run/secrets/synveda-gateway/database_url",
  "- name: SYNVEDA_DATABASE_ROLES_FILE",
  "value: /etc/synveda/database/roles.json",
  "name: database-roles",
  "mountPath: /etc/synveda/database",
  "secretName: synveda-gateway-db",
  "path: /readyz",
]);
forbidMarkers("gateway", gateway, [
  "synveda-pg-app",
  "synveda-pg-superuser",
  "synveda-worker-db",
  "initContainers:",
  "wait-for-schema",
]);

requireMarkers("worker", worker, [
  'args: ["worker"]',
  "automountServiceAccountToken: false",
  "- name: DATABASE_URL_FILE",
  "value: /run/secrets/synveda-worker/database_url",
  "secretName: synveda-worker-db",
  "value: 127.0.0.1:8121",
  "- name: SYNVEDA_DATABASE_ROLES_FILE",
  "value: /etc/synveda/database/roles.json",
  "name: database-roles",
  "mountPath: /etc/synveda/database",
  "- worker\n",
  "- ready\n",
  "timeoutSeconds: 3",
]);
forbidMarkers("worker", worker, [
  "synveda-pg-app",
  "synveda-pg-superuser",
  "synveda-gateway-db",
  "initContainers:",
  "wait-for-schema",
]);
if (/^\s*ports:/m.test(worker) || resource(valid.stdout, "Service", "worker")) {
  throw new Error("worker health must remain private: no port or Service may render");
}
if (gateway.includes("SYNVEDA_EXTRACTOR") || !worker.includes("SYNVEDA_EXTRACTOR")) {
  throw new Error("extractor configuration is not owned exclusively by the worker");
}

const bootstrap = namedItem(install, "database-bootstrap");
const preflight = namedItem(install, "database-preflight");
const migrate = namedItem(install, "migrate");
const tenant = namedItem(install, "tenant");
if (!bootstrap || !preflight || !migrate || !tenant) {
  throw new Error("install Job is missing an ordered bootstrap/preflight/migrate/tenant stage");
}
requireMarkers("install Job", install, ["activeDeadlineSeconds: 900"]);
if (
  install.indexOf("- name: database-bootstrap") > install.indexOf("- name: database-preflight") ||
  install.indexOf("- name: database-preflight") > install.indexOf("- name: migrate") ||
  install.indexOf("- name: migrate") > install.indexOf("- name: tenant")
) {
  throw new Error("install Job authority stages are not ordered");
}

requireMarkers("database bootstrap", bootstrap, [
  "image: synveda/enterprise-postgres:17",
  'command: ["/bin/sh", "-ec"]',
  "source=/run/bootstrap-projection/$secret",
  "destination=/run/secrets/$secret",
  'cp -- "$source" "$destination"',
  'chmod 0600 "$destination"',
  "exec /usr/local/bin/synveda-database-bootstrap synveda",
  "name: SYNVEDA_DATABASE_BOOTSTRAP_PRIVATE_DIR",
  "value: /run/secrets",
  "name: synveda-pg-superuser",
  "name: bootstrap-projection",
  "mountPath: /run/bootstrap-projection",
  "name: bootstrap-private",
  "mountPath: /run/secrets",
  "name: bootstrap-snapshots",
  "mountPath: /tmp",
]);
forbidMarkers("database bootstrap", bootstrap, [
  "synveda db migrate",
  "tenant create",
  "postgres_bootstrap_password=",
  "synveda_migrator_password=",
  "synveda_gateway_password=",
  "synveda_worker_password=",
]);

requireMarkers("database preflight", preflight, [
  'args: ["database-preflight"]',
  "- name: SYNVEDA_MIGRATOR_DATABASE_URL_FILE",
  "value: /run/secrets/synveda-preflight/migrator_database_url",
  "- name: SYNVEDA_GATEWAY_DATABASE_URL_FILE",
  "value: /run/secrets/synveda-preflight/gateway_database_url",
  "- name: SYNVEDA_WORKER_DATABASE_URL_FILE",
  "value: /run/secrets/synveda-preflight/worker_database_url",
  "- name: SYNVEDA_DATABASE_ROLES_FILE",
  "value: /etc/synveda/database/roles.json",
  "name: database-roles",
  "mountPath: /etc/synveda/database",
  "name: preflight-databases",
]);
forbidMarkers("database preflight", preflight, ["synveda-pg-superuser", "bootstrap-secrets"]);

requireMarkers("migration", migrate, [
  'args: ["migrate"]',
  "- name: DATABASE_URL_FILE",
  "value: /run/secrets/synveda-migrator/database_url",
  "- name: SYNVEDA_DATABASE_ROLES_FILE",
  "value: /etc/synveda/database/roles.json",
  "name: database-roles",
  "mountPath: /etc/synveda/database",
  "name: migrator-database",
]);
forbidMarkers("migration", migrate, [
  "synveda-pg-superuser",
  "synveda-gateway-db",
  "synveda-worker-db",
  "bootstrap-secrets",
]);

for (const [name, block] of [
  ["database preflight", preflight],
  ["migration", migrate],
  ["tenant", tenant],
]) {
  if (containerImage(`\n${block}`, name === "database preflight" ? "database-preflight" : name === "migration" ? "migrate" : "tenant") !== productImage) {
    throw new Error(`${name} does not use the same immutable product image as the runtimes`);
  }
}

requireMarkers("install Job Secret projection", install, [
  "automountServiceAccountToken: false",
  "name: bootstrap-projection",
  "name: bootstrap-private",
  "name: bootstrap-snapshots",
  "medium: Memory",
  "sizeLimit: 64Ki",
  "name: synveda-pg-superuser",
  "path: postgres_bootstrap_password",
  "name: synveda-pg-app",
  "path: synveda_migrator_password",
  "name: synveda-gateway-db",
  "path: synveda_gateway_password",
  "name: synveda-worker-db",
  "path: synveda_worker_password",
  "path: migrator_database_url",
  "path: gateway_database_url",
  "path: worker_database_url",
  "secretName: synveda-pg-app",
]);

const bootstrapPrivateLimit = install.match(
  /name: bootstrap-private[\s\S]*?sizeLimit: ([0-9]+)Ki/,
);
if (!bootstrapPrivateLimit) fail("install Job lacks a bounded bootstrap-private volume");
const maxCredentialBytes = 4096;
const copiedInputBytes = 4 * maxCredentialBytes + 4096;
const escapedPgpassBytes = 3 * (2 * maxCredentialBytes + 256);
const bootstrapWorkingHeadroom = 4096;
if (
  Number.parseInt(bootstrapPrivateLimit[1], 10) * 1024 <
  copiedInputBytes + escapedPgpassBytes + bootstrapWorkingHeadroom
) {
  fail("install Job bootstrap-private volume cannot hold every accepted bounded input");
}

const bootstrapSnapshotLimit = install.match(
  /name: bootstrap-snapshots[\s\S]*?sizeLimit: ([0-9]+)Mi/,
);
if (!bootstrapSnapshotLimit || Number.parseInt(bootstrapSnapshotLimit[1], 10) !== 16) {
  fail("install Job bootstrap-snapshots volume must match the bounded 16Mi Compose /tmp contract");
}

const withTenant = render([
  "--set-string",
  "install.tenant.slug=acme",
  "--set-string",
  "install.tenant.name=Acme",
]);
requireSuccess("tenant admission chart", withTenant);
const tenantInstall = resource(withTenant.stdout, "Job", "install");
const tenantStage = namedItem(tenantInstall, "tenant");
requireMarkers("tenant admission", tenantStage, [
  "/usr/local/bin/synveda tenant create",
  "- name: DATABASE_URL_FILE",
  "value: /run/secrets/synveda-migrator/database_url",
  "- name: SYNVEDA_DATABASE_ROLES_FILE",
  "value: /etc/synveda/database/roles.json",
  "name: database-roles",
  "name: migrator-database",
]);
forbidMarkers("tenant admission", tenantStage, [
  "synveda-pg-superuser",
  "synveda-gateway-db",
  "synveda-worker-db",
  "bootstrap-secrets",
]);

if (
  !worker.includes("terminationGracePeriodSeconds: 85") ||
  !worker.includes('- name: SYNVEDA_WORKER_SHUTDOWN_SECS\n              value: "75"')
) {
  throw new Error("default worker pod grace must remain ten seconds beyond its join bound");
}
const customShutdown = render(["--set-string", "worker.shutdownSeconds=29"]);
requireSuccess("custom worker shutdown", customShutdown);
const customWorker = resource(customShutdown.stdout, "Deployment", "worker");
if (
  !customWorker?.includes("terminationGracePeriodSeconds: 39") ||
  !customWorker.includes('- name: SYNVEDA_WORKER_SHUTDOWN_SECS\n              value: "29"')
) {
  throw new Error("custom worker pod grace must derive as shutdownSeconds + 10");
}

for (const [component, document] of [
  ["gateway", gateway],
  ["worker", worker],
]) {
  for (const [envName, secretKey] of [
    ["SYNVEDA_KMS_KEY", "SYNVEDA_KMS_KEY"],
    ["SYNVEDA_KMS_KEY_REF", "SYNVEDA_KMS_KEY_REF"],
  ]) {
    const pattern = new RegExp(
      `- name: ${envName}\\n\\s+valueFrom:\\n\\s+secretKeyRef:\\n\\s+name: synveda-kms\\n\\s+key: ${secretKey}`,
    );
    if (!pattern.test(document)) {
      throw new Error(`${component} does not source ${envName} from synveda-kms/${secretKey}`);
    }
  }
}

for (const [name, expected, args] of [
  ["missing KMS Secret", "kms.existingSecret is required", ["--set-string", "kms.existingSecret="]],
  [
    "missing gateway database Secret",
    "gateway.databaseExistingSecret is required",
    ["--set-string", "gateway.databaseExistingSecret="],
  ],
  [
    "missing worker database Secret",
    "worker.databaseExistingSecret is required",
    ["--set-string", "worker.databaseExistingSecret="],
  ],
  [
    "shared runtime database Secret",
    "gateway and worker database Secrets must be distinct",
    ["--set-string", "gateway.databaseExistingSecret=synveda-worker-db"],
  ],
  [
    "gateway migrator Secret",
    "gateway.databaseExistingSecret must not be the CloudNativePG migrator Secret",
    ["--set-string", "gateway.databaseExistingSecret=synveda-pg-app"],
  ],
  [
    "worker migrator Secret",
    "worker.databaseExistingSecret must not be the CloudNativePG migrator Secret",
    ["--set-string", "worker.databaseExistingSecret=synveda-pg-app"],
  ],
  [
    "gateway superuser Secret",
    "gateway.databaseExistingSecret must not be the CloudNativePG superuser Secret",
    ["--set-string", "gateway.databaseExistingSecret=synveda-pg-superuser"],
  ],
  [
    "worker superuser Secret",
    "worker.databaseExistingSecret must not be the CloudNativePG superuser Secret",
    ["--set-string", "worker.databaseExistingSecret=synveda-pg-superuser"],
  ],
  [
    "gateway key alias",
    "gateway database URL and password Secret keys must be distinct",
    ["--set-string", "gateway.databasePasswordSecretKey=DATABASE_URL"],
  ],
  [
    "worker key alias",
    "worker database URL and password Secret keys must be distinct",
    ["--set-string", "worker.databasePasswordSecretKey=DATABASE_URL"],
  ],
  [
    "optional install bypass",
    "install.enabled was removed",
    ["--set-string", "install.enabled=false"],
  ],
  [
    "oversized worker pool",
    "worker.dbMaxConnections must be between 1 and 64",
    ["--set-string", "worker.dbMaxConnections=65"],
  ],
  [
    "unbounded worker shutdown",
    "worker.shutdownSeconds must be between 3 and 300",
    ["--set-string", "worker.shutdownSeconds=2"],
  ],
  [
    "short install deadline",
    "install.activeDeadlineSeconds must be between 300 and 3600",
    ["--set-string", "install.activeDeadlineSeconds=299"],
  ],
  [
    "unbounded install deadline",
    "install.activeDeadlineSeconds must be between 300 and 3600",
    ["--set-string", "install.activeDeadlineSeconds=3601"],
  ],
  [
    "unbounded install retries",
    "install.backoffLimit must be between 0 and 6",
    ["--set-string", "install.backoffLimit=7"],
  ],
  [
    "short install result retention",
    "install.ttlSecondsAfterFinished must be between 300 and 604800",
    ["--set-string", "install.ttlSecondsAfterFinished=299"],
  ],
  [
    "worker replicas",
    "worker replicas are not configurable",
    ["--set-string", "worker.replicas=2"],
  ],
  [
    "disabled extractor",
    "extractor.kind must be one of deterministic|claude|vllm",
    ["--set-string", "extractor.kind=off"],
  ],
  [
    "vLLM without a model",
    "extractor.model is empty",
    [
      "--set-string",
      "extractor.kind=vllm",
      "--set-string",
      "extractor.baseUrl=http://vllm.example:8000",
    ],
  ],
]) {
  requireRefusal(name, expected, args);
}

const scratch = mkdtempSync(join(tmpdir(), "synveda-helm-secret-read-"));
try {
  for (const [name, value] of [
    ["without-newline", "postgres://runtime:opaque@db/synveda"],
    ["with-newline", "postgres://runtime:opaque@db/synveda\n"],
  ]) {
    const path = join(scratch, name);
    writeFileSync(path, value, { mode: 0o600 });
    const read = spawnSync(
      "sh",
      [
        "-ec",
        'DATABASE_URL=; IFS= read -r DATABASE_URL < "$1" || [ -n "$DATABASE_URL" ]; [ "$DATABASE_URL" = "postgres://runtime:opaque@db/synveda" ]',
        "sh",
        path,
      ],
      { encoding: "utf8" },
    );
    if (read.status !== 0) {
      throw new Error(`runtime Secret read failed for ${name}: ${read.stderr}`);
    }
  }
} finally {
  rmSync(scratch, { recursive: true, force: true });
}

console.log(
  "ok: Helm renders one migrator-owned database, mandatory three-role preflight, narrow migration/tenant authority, and distinct file-only gateway/worker credentials.",
);
