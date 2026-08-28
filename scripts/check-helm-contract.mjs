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

const valid = render();
requireSuccess("minimal chart", valid);

function resource(source, kind, component) {
  return source
    .split(/^---\s*$/m)
    .find(
      (document) =>
        new RegExp(`^kind: ${kind}$`, "m").test(document) &&
        document.includes(`app.kubernetes.io/component: ${component}`),
    );
}

function containerImage(document, name) {
  return document?.match(
    new RegExp(`\\n\\s+- name: ${name}\\n\\s+image: (\\S+)`),
  )?.[1];
}

const gateway = resource(valid.stdout, "Deployment", "gateway");
const worker = resource(valid.stdout, "Deployment", "worker");
if (!gateway || !worker) {
  throw new Error("rendered chart must contain separate gateway and worker Deployments");
}
if (containerImage(gateway, "gateway") !== containerImage(worker, "worker")) {
  throw new Error("gateway and worker do not use the same product image");
}
for (const marker of [
  'args: ["worker"]',
  "- name: DATABASE_URL_FILE",
  "value: /run/secrets/synveda-worker/database_url",
  "secretName: synveda-worker-db",
  "value: 127.0.0.1:8121",
  "- name: SYNVEDA_EXPECTED_DATABASE_ROLE",
  "value: synveda_worker",
  'IFS= read -r DATABASE_URL < /run/secrets/synveda-worker/database_url || [ -n "$DATABASE_URL" ]',
  "- worker\n",
  "- ready\n",
  "timeoutSeconds: 3",
]) {
  if (!worker.includes(marker)) {
    throw new Error(`rendered worker is missing ${marker.trim()}`);
  }
}
if (/^\s*ports:/m.test(worker) || resource(valid.stdout, "Service", "worker")) {
  throw new Error("worker health must remain private: no port or Service may render");
}
if (gateway.includes("SYNVEDA_EXTRACTOR") || !worker.includes("SYNVEDA_EXTRACTOR")) {
  throw new Error("extractor configuration is not owned exclusively by the worker");
}
const install = resource(valid.stdout, "Job", "install");
for (const marker of [
  "create role synveda_worker",
  "alter role synveda_worker",
  "grant synveda_app to synveda_worker",
  "name: synveda-worker-db",
  "mountPath: /run/secrets/synveda-worker",
  "path: database_url",
  "ADMIN_DATABASE_IDENTITY=$(PGDATABASE=\"$DATABASE_URL\" psql",
  "WORKER_DATABASE_IDENTITY=$(PGDATABASE=\"$WORKER_DATABASE_URL\" psql",
  "pg_catalog.pg_control_system()",
  "database.oid::text",
  "pg_catalog.pg_postmaster_start_time()",
  "pg_catalog.pg_is_in_recovery()",
  "current_setting('transaction_read_only')::boolean",
  '[ -z "$ADMIN_DATABASE_IDENTITY" ]',
  '[ -z "$WORKER_DATABASE_IDENTITY" ]',
  "the install and worker credentials must target one live PostgreSQL primary instance and database",
  "select 1 from pg_catalog.pg_namespace",
  "where nspowner = roles.oid",
  "select 1 from pg_catalog.pg_class objects",
  "select 1 from pg_catalog.pg_proc routines",
]) {
  if (!install?.includes(marker)) {
    throw new Error(`install job does not converge the worker role: missing ${marker}`);
  }
}
if (
  install?.includes("-v worker_password=") ||
  !install?.includes("\\getenv worker_password SYNVEDA_WORKER_PASSWORD")
) {
  throw new Error("install job must read the worker password inside psql, never from argv");
}
if ((install?.match(/or not membership\.inherit_option/g) ?? []).length < 2) {
  throw new Error(
    "install job must reject unsafe duplicate synveda_app grants for both runtime roles",
  );
}
for (const [component, document] of [
  ["install", install],
  ["gateway", gateway],
  ["worker", worker],
]) {
  if (/psql\s+"\$(?:DATABASE_URL|WORKER_DATABASE_URL)"/.test(document)) {
    throw new Error(`${component} must pass PostgreSQL URLs through libpq environment, not argv`);
  }
}
if (!worker.includes("automountServiceAccountToken: false")) {
  throw new Error("worker must not receive an unused Kubernetes API token");
}
if (
  !worker.includes("terminationGracePeriodSeconds: 85") ||
  !worker.includes('- name: SYNVEDA_WORKER_SHUTDOWN_SECS\n              value: "75"')
) {
  throw new Error("default worker pod grace must remain ten seconds beyond its join bound");
}
const customShutdown = render([
  "--set-string",
  "worker.shutdownSeconds=29",
]);
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
      throw new Error(
        `rendered ${component} does not source ${envName} from synveda-kms/${secretKey}`,
      );
    }
  }
}

requireRefusal(
  "missing KMS Secret",
  "kms.existingSecret is required",
  ["--set-string", "kms.existingSecret="],
);
requireRefusal(
  "worker owner Secret",
  "worker.databaseExistingSecret must not be the CloudNativePG app Secret",
  ["--set-string", "worker.databaseExistingSecret=synveda-pg-app"],
);
requireRefusal(
  "oversized worker pool",
  "worker.dbMaxConnections must be between 1 and 64",
  ["--set-string", "worker.dbMaxConnections=65"],
);
requireRefusal(
  "unbounded worker shutdown",
  "worker.shutdownSeconds must be between 1 and 300",
  ["--set-string", "worker.shutdownSeconds=301"],
);
requireRefusal(
  "missing worker database Secret",
  "worker.databaseExistingSecret is required",
  ["--set-string", "worker.databaseExistingSecret="],
);

const scratch = mkdtempSync(join(tmpdir(), "synveda-helm-secret-read-"));
try {
  for (const [name, value] of [
    ["without-newline", "postgres://worker:opaque@db/synveda"],
    ["with-newline", "postgres://worker:opaque@db/synveda\n"],
  ]) {
    const path = join(scratch, name);
    writeFileSync(path, value, { mode: 0o600 });
    const read = spawnSync(
      "sh",
      [
        "-ec",
        'DATABASE_URL=; IFS= read -r DATABASE_URL < "$1" || [ -n "$DATABASE_URL" ]; [ "$DATABASE_URL" = "postgres://worker:opaque@db/synveda" ]',
        "sh",
        path,
      ],
      { encoding: "utf8" },
    );
    if (read.status !== 0) {
      throw new Error(`worker Secret read failed for ${name}: ${read.stderr}`);
    }
  }
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
requireRefusal(
  "worker replicas",
  "worker replicas are not configurable",
  ["--set-string", "worker.replicas=2"],
);
requireRefusal(
  "disabled extractor",
  "extractor.kind must be one of deterministic|claude|vllm",
  ["--set-string", "extractor.kind=off"],
);
requireRefusal(
  "vLLM without a model",
  "extractor.model is empty",
  [
    "--set-string",
    "extractor.kind=vllm",
    "--set-string",
    "extractor.baseUrl=http://vllm.example:8000",
  ],
);

console.log("ok: Helm renders one image as separate gateway/worker processes, binds bootstrap and worker credentials to one database, keeps worker health private, and refuses invalid runtime secrets or replicas.");
