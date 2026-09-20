#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const chart = "deploy/helm/synveda";
const values = `${chart}/ci/cnpg-values.yaml`;
const appVersion = readFileSync(`${chart}/Chart.yaml`, "utf8").match(
  /^appVersion:\s*"?([^"\s]+)"?/m,
)?.[1];
if (!appVersion) throw new Error("chart appVersion is missing");

function render(extraArgs = []) {
  return spawnSync(
    "helm",
    ["template", "synveda", chart, "--api-versions", "postgresql.cnpg.io/v1", "-f", values, ...extraArgs],
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
  if (!output.includes(expected) && !output.replaceAll("/", ".").includes(expected)) {
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
  "create database synveda with owner synveda_migrator template template0 encoding 'UTF8' allow_connections false",
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
  "create database synveda with owner synveda_migrator template template0 encoding 'UTF8' allow_connections false",
);
if (!(postInit >= 0 && postInit < publicRevoke && publicRevoke < closedApplicationDatabase)) {
  throw new Error(
    "CloudNativePG must close maintenance-database PUBLIC ACLs and create the application database closed during init",
  );
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
  '- name: SYNVEDA_INSECURE_DEVELOPMENT_HTTP\n              value: "false"',
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
  `image: ghcr.io/synveda/cnpg-postgres:17.11-synveda-${appVersion}`,
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
  "while ! /usr/local/bin/synveda-container database-preflight; do",
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
  'args: ["/usr/local/bin/synveda-container", "migrate"]',
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
  /^ {8}- name: bootstrap-private\n[\s\S]*?sizeLimit: ([0-9]+)Ki/m,
);
if (!bootstrapPrivateLimit) throw new Error("install Job lacks a bounded bootstrap-private volume");
const maxCredentialBytes = 4096;
const copiedInputBytes = 4 * maxCredentialBytes + 4096;
const escapedPgpassBytes = 3 * (2 * maxCredentialBytes + 256);
const bootstrapWorkingHeadroom = 4096;
if (
  Number.parseInt(bootstrapPrivateLimit[1], 10) * 1024 <
  copiedInputBytes + escapedPgpassBytes + bootstrapWorkingHeadroom
) {
  throw new Error("install Job bootstrap-private volume cannot hold every accepted bounded input");
}

const bootstrapSnapshotLimit = install.match(
  /^ {8}- name: bootstrap-snapshots\n[\s\S]*?sizeLimit: ([0-9]+)Mi/m,
);
if (!bootstrapSnapshotLimit || Number.parseInt(bootstrapSnapshotLimit[1], 10) !== 16) {
  throw new Error("install Job bootstrap-snapshots volume must match the bounded 16Mi Compose /tmp contract");
}

const withTenant = render([
  "--set-string",
  "install.tenant.slug=acme",
  "--set-string",
  "install.tenant.id=019b53c0-7c00-7000-8000-000000000011",
  "--set-string",
  "install.tenant.name=Acme",
]);
requireSuccess("tenant admission chart", withTenant);
const tenantInstall = resource(withTenant.stdout, "Job", "install");
const tenantStage = namedItem(tenantInstall, "tenant");
requireMarkers("tenant admission", tenantStage, [
  'args: ["tenant-converge"]',
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
const customShutdown = render(["--set", "worker.shutdownSeconds=29"]);
requireSuccess("custom worker shutdown", customShutdown);
const customWorker = resource(customShutdown.stdout, "Deployment", "worker");
if (
  !customWorker?.includes("terminationGracePeriodSeconds: 39") ||
  !customWorker.includes('- name: SYNVEDA_WORKER_SHUTDOWN_SECS\n              value: "29"')
) {
  throw new Error("custom worker pod grace must derive as shutdownSeconds + 10");
}

const developmentHttp = render([
  "--set-string",
  "gateway.publicUrl=http://synveda.example.test",
  "--set",
  "gateway.insecureDevelopmentHttp=true",
]);
requireSuccess("explicit development HTTP chart", developmentHttp);
const developmentGateway = resource(developmentHttp.stdout, "Deployment", "gateway");
requireMarkers("explicit development HTTP gateway", developmentGateway, [
  '- name: SYNVEDA_INSECURE_DEVELOPMENT_HTTP\n              value: "true"',
]);

for (const [component, document] of [
  ["gateway", gateway],
  ["worker", worker],
]) {
  requireMarkers(component, document, [
    "- name: SYNVEDA_OIDC_ISSUERS_FILE", "- name: SYNVEDA_KMS_KEY_FILE",
    "- name: SYNVEDA_KMS_KEY_REF_FILE", "name: synveda-kms",
    "key: SYNVEDA_KMS_KEY", "key: SYNVEDA_KMS_KEY_REF",
    "mountPath: /run/secrets/synveda-runtime", "startupProbe:",
  ]);
  forbidMarkers(component, document, ["secretKeyRef:", "- name: SYNVEDA_KMS_KEY\n"]);
}

for (const [name, expected, args] of [
  [
    "implicit plaintext public URL",
    "plaintext gateway.publicUrl requires gateway.insecureDevelopmentHttp=true",
    ["--set-string", "gateway.publicUrl=http://synveda.example.test"],
  ],
  [
    "string development HTTP relaxation",
    "gateway.insecureDevelopmentHttp",
    ["--set-string", "gateway.insecureDevelopmentHttp=true"],
  ],
  [
    "HTTPS with development HTTP relaxation",
    "gateway.insecureDevelopmentHttp must remain false",
    ["--set", "gateway.insecureDevelopmentHttp=true"],
  ],
  ["missing KMS Secret", "kms.existingSecret", ["--set-string", "kms.existingSecret="]],
  [
    "missing gateway database Secret",
    "gateway.databaseExistingSecret",
    ["--set-string", "gateway.databaseExistingSecret="],
  ],
  [
    "missing worker database Secret",
    "worker.databaseExistingSecret",
    ["--set-string", "worker.databaseExistingSecret="],
  ],
  [
    "shared runtime database Secret",
    "gateway and worker database Secrets must be distinct",
    ["--set-string", "gateway.databaseExistingSecret=synveda-worker-db"],
  ],
  [
    "gateway migrator Secret",
    "gateway.databaseExistingSecret must not be the migrator Secret",
    ["--set-string", "gateway.databaseExistingSecret=synveda-pg-app"],
  ],
  [
    "worker migrator Secret",
    "worker.databaseExistingSecret must not be the migrator Secret",
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
    "enabled",
    ["--set-string", "install.enabled=false"],
  ],
  [
    "oversized worker pool",
    "worker.dbMaxConnections",
    ["--set", "worker.dbMaxConnections=65"],
  ],
  [
    "unbounded worker shutdown",
    "worker.shutdownSeconds",
    ["--set", "worker.shutdownSeconds=2"],
  ],
  [
    "short install deadline",
    "install.activeDeadlineSeconds",
    ["--set", "install.activeDeadlineSeconds=299"],
  ],
  [
    "unbounded install deadline",
    "install.activeDeadlineSeconds",
    ["--set", "install.activeDeadlineSeconds=3601"],
  ],
  [
    "unbounded install retries",
    "install.backoffLimit",
    ["--set", "install.backoffLimit=7"],
  ],
  [
    "short install result retention",
    "install.ttlSecondsAfterFinished",
    ["--set", "install.ttlSecondsAfterFinished=299"],
  ],
  [
    "worker replicas",
    "replicas",
    ["--set-string", "worker.replicas=2"],
  ],
  [
    "disabled extractor",
    "extractor.kind",
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

// OPS-11: the application-only path must work with no operator API advertised.
function externalRender(extraArgs = []) {
  return spawnSync("helm", ["template", "synveda", chart, "-f", `${chart}/ci/external-values.yaml`, ...extraArgs], { encoding: "utf8" });
}
const external = externalRender();
requireSuccess("external services", external);
forbidMarkers("external services", external.stdout, [
  "kind: Cluster\n", "kind: Secret\n", "kind: ClusterRole", "kind: PersistentVolumeClaim",
  "database-bootstrap", "synveda-pg-superuser", "helm.sh/hook", "kind: StatefulSet",
]);
for (const component of ["gateway", "worker"]) {
  const deployment = resource(external.stdout, "Deployment", component);
  requireMarkers(component, deployment, [
    "replicas: 1", "type: Recreate", "startupProbe:", "automountServiceAccountToken: false",
    "SYNVEDA_DATABASE_EXPECTED_HOST", "SYNVEDA_DATABASE_EXPECTED_ROOT_CERT_FILE",
    "mountPath: /run/secrets/synveda-postgres", "runAsNonRoot: true",
  ]);
  forbidMarkers(component, deployment, ["synveda-migrator-db", "runAsUser:"]);
}
const externalJob = resource(external.stdout, "Job", "install");
requireMarkers("external Job", externalJob, ["synveda-install-1", "activeDeadlineSeconds: 900", "synveda-migrator-db", "retry $attempt/12", '"300s"']);
for (const stage of ["database-preflight", "migrate"]) {
  requireMarkers(stage, namedItem(externalJob, stage), ["SYNVEDA_DATABASE_EXPECTED_ROOT_CERT_FILE", "mountPath: /run/secrets/synveda-postgres"]);
  forbidMarkers(stage, namedItem(externalJob, stage), ["tenant-kms", "SYNVEDA_KMS_KEY_FILE"]);
}
const admitted = externalRender(["--set-string", "install.tenant.id=019b53c0-7c00-7000-8000-000000000011", "--set-string", "install.tenant.slug=acme"]);
requireSuccess("tenant convergence", admitted);
const admittedJob = resource(admitted.stdout, "Job", "install");
requireMarkers("tenant encryption-key admission", namedItem(admittedJob, "tenant"), ["SYNVEDA_KMS_KEY_FILE", "SYNVEDA_KMS_KEY_REF_FILE", "mountPath: /run/secrets/synveda-kms"]);
for (const stage of ["database-preflight", "migrate"]) {
  forbidMarkers(stage, namedItem(admittedJob, stage), ["tenant-kms", "SYNVEDA_KMS_KEY_FILE"]);
}
const digest = `sha256:${"a".repeat(64)}`;
const pinned = externalRender(["--set-string", `image.digest=${digest}`]);
requireSuccess("digest image", pinned);
if ((pinned.stdout.match(new RegExp(`image: ghcr.io/synveda/product@${digest}`, "g")) ?? []).length !== 5) {
  throw new Error("digest must select the same product image for both processes and all three Job stages");
}
const mutualTls = externalRender(["--set-string", "postgres.external.clientExistingSecret=client-cert", "--set-string", "postgres.external.clientCertSecretKey=tls.crt", "--set-string", "postgres.external.clientKeySecretKey=tls.key"]);
requireSuccess("client certificate", mutualTls);
requireMarkers("client certificate", mutualTls.stdout, ["SYNVEDA_DATABASE_EXPECTED_CLIENT_CERT_FILE", "SYNVEDA_DATABASE_EXPECTED_CLIENT_KEY_FILE", "secretName: client-cert"]);
const tei = externalRender(["--set", "embedder=tei,tei.enabled=true", "--set-string", "tei.nodeSelector.pool=models", "--set-string", "tei.podAnnotations.owner=operators"]);
requireSuccess("optional non-root TEI", tei);
requireMarkers("optional TEI", resource(tei.stdout, "Deployment", "tei"), [
  "automountServiceAccountToken: false", "runAsNonRoot: true", "runAsUser: 1000",
  "readOnlyRootFilesystem: true", "containerPort: 8080", "startupProbe:", "livenessProbe:",
  "pool: models", "owner: operators", "mountPath: /tmp",
]);
const schemaRefusals = [
  ["missing endpoint", ["--set-string", "postgres.external.host="], "host"],
  ["missing migrator Secret", ["--set-string", "postgres.external.migratorExistingSecret="], "migratorExistingSecret"],
  ["missing CA Secret", ["--set-string", "postgres.external.caExistingSecret="], "caExistingSecret"],
  ["ambiguous modes", ["--set", "postgres.mode=cnpg"], "external"],
  ["unknown setting", ["--set", "gateway.replicaCount=2"], "replicaCount"],
  ["unknown dependency", ["--set", "redis.enabled=true"], "redis"],
  ["unknown database setting", ["--set", "postgres.external.sslmode=disable"], "sslmode"],
  ["unpaired certificate", ["--set-string", "postgres.external.clientExistingSecret=client"], "client"],
  ["CNPG settings with external database", ["--set", "postgres.instances=2"], "postgres.mode=cnpg"],
  ["CNPG resources with external database", ["--set-string", "postgres.resources.requests.memory=4Gi"], "postgres.mode=cnpg"],
  ["image tag and digest", ["--set-string", "image.tag=test", "--set-string", `image.digest=${digest}`], "mutually exclusive"],
  ["ignored TEI URL", ["--set-string", "tei.url=http://tei:80"], "embedder=tei"],
  ["ignored deterministic model", ["--set-string", "embedderModel=other"], "embedder=tei"],
  ["ignored TEI workload", ["--set-string", "tei.nodeSelector.pool=models"], "tei.enabled=true"],
  ["ignored database password key", ["--set-string", "worker.databasePasswordSecretKey=unused"], "postgres.mode=cnpg"],
  ["ignored OIDC CA key", ["--set-string", "oidc.caSecretKey=unused"], "oidc.caExistingSecret"],
  ["ignored extractor key", ["--set-string", "extractor.secretKey=unused"], "extractor.kind=claude"],
  ["unacknowledged ingress TLS", ["--set", "ingress.enabled=true", "--set-string", "ingress.host=synveda.example.com"], "TLS Secret"],
  ["non-ClusterIP exposure", ["--set", "service.type=LoadBalancer"], "ClusterIP"],
  ["privileged process", ["--set", "podSecurityContext.runAsUser=0"], "runAsUser"],
];
for (const [name, args, expected] of schemaRefusals) {
  const result = externalRender(args);
  if (result.status === 0 || !`${result.stderr}${result.stdout}`.includes(expected)) {
    throw new Error(`${name} did not produce its expected refusal: ${result.stderr}`);
  }
}
const absentApi = spawnSync("helm", ["template", "synveda", chart, "-f", values], { encoding: "utf8" });
if (absentApi.status === 0 || !absentApi.stderr.includes("preinstalled CloudNativePG")) throw new Error("missing CNPG API was not refused explicitly");
console.log("ok: external mode, strict values schema, TLS/mTLS file references, digest selection, bounded Job and unsupported-mode refusals.");

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
