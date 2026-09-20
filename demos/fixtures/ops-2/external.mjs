#!/usr/bin/env node
// OPS-11: disposable external services, provisioned independently of Helm.
// Credentials remain in memory or mode-0600 scratch files, never command args.
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomBytes } from "node:crypto";

const cluster = process.env.CLUSTER ?? "synveda-ops11";
if (!/^synveda-ops11(?:-[a-z0-9-]+)?$/.test(cluster)) throw new Error("select a disposable synveda-ops11 cluster");
for (const key of ["KEEP", "REUSE", "SKIP_BUILD"]) {
  if (![undefined, "0", "1"].includes(process.env[key])) throw new Error(`${key} must be 0 or 1`);
}
const scratch = mkdtempSync(join(tmpdir(), "synveda-ops11-"));
const env = { ...process.env, KUBECONFIG: join(scratch, "kubeconfig") };
const ns = "synveda-test";
const providers = "synveda-dependencies";
const host = `postgres.${providers}.svc.cluster.local`;
const auth = `https://keycloak.${providers}.svc.cluster.local:8443`;
const publicUrl = `http://synveda.${ns}.svc.cluster.local:8120`;
const tenant = "019b53c0-7c00-7000-8000-000000000011";
const product = "synveda/product:ops11";
const postgres = "synveda/postgres:ops11";
const keycloak = "synveda/keycloak:ops11";
const values = "demos/fixtures/ops-2/external-values.yaml";
let owned = false;
let createdCluster = false;

function run(command, args, { input, timeout = 120000, quiet = false, allowFailure = false } = {}) {
  const result = spawnSync(command, args, { env, input, encoding: "utf8", timeout, maxBuffer: 8 * 1024 * 1024 });
  if (allowFailure) return result;
  if (result.error || result.status !== 0) {
    // kubectl apply can echo a refused Secret; suppress all input-bearing errors.
    if (!input && !quiet) process.stderr.write(result.stderr ?? "");
    throw new Error(`${command} ${args[0]} failed (exit ${result.status ?? "timeout"})`);
  }
  return result.stdout;
}
const k = (args, options) => run("kubectl", args, options);
const apply = (document) => k(["apply", "-f", "-"], { input: JSON.stringify(document), quiet: true });
function secret(namespace, name, stringData) {
  apply({ apiVersion: "v1", kind: "Secret", metadata: { namespace, name }, type: "Opaque", stringData });
}
function file(name, bytes) { const path = join(scratch, name); writeFileSync(path, bytes, { mode: 0o600 }); return path; }
function certificate(prefix, sans, usage = "serverAuth") {
  const key = join(scratch, `${prefix}.key`), csr = join(scratch, `${prefix}.csr`), crt = join(scratch, `${prefix}.crt`);
  run("openssl", ["req", "-new", "-newkey", "rsa:2048", "-nodes", "-keyout", key, "-out", csr, "-subj", `/CN=${sans[0]}`], { quiet: true });
  const ext = file(`${prefix}.ext`, `subjectAltName=${sans.map((name) => `DNS:${name}`).join(",")}\nextendedKeyUsage=${usage}\n`);
  run("openssl", ["x509", "-req", "-in", csr, "-CA", join(scratch, "ca.crt"), "-CAkey", join(scratch, "ca.key"), "-CAcreateserial", "-out", crt, "-days", "2", "-sha256", "-extfile", ext], { quiet: true });
  return { "tls.crt": readFileSync(crt, "utf8"), "tls.key": readFileSync(key, "utf8") };
}
const securityContext = { runAsNonRoot: true, runAsUser: 999, runAsGroup: 999, fsGroup: 999 };
const containerSecurity = { allowPrivilegeEscalation: false, readOnlyRootFilesystem: true, capabilities: { drop: ["ALL"] } };
function rollout(name, namespace = ns) { k(["rollout", "status", "-n", namespace, `deployment/${name}`, "--timeout=600s"], { timeout: 620000 }); }
function helm() {
  console.log("helm upgrade --install synveda deploy/helm/synveda -n synveda-test -f demos/fixtures/ops-2/external-values.yaml --wait --wait-for-jobs --timeout 15m");
  run("helm", ["upgrade", "--install", "synveda", "deploy/helm/synveda", "-n", ns, "-f", values, "--wait", "--wait-for-jobs", "--timeout", "15m"], { timeout: 920000 });
}
const issuer = (changes = {}) => JSON.stringify([{ issuer: `${auth}/realms/synveda`, client_id: "synveda", audience: "synveda-api", tenant: { static: { tenant_id: tenant } }, ...changes }]);
function waitClient() {
  for (let attempt = 0; attempt < 120; attempt++) {
    const result = k(["exec", "-n", ns, "synveda-install-test", "-c", "client", "--", "cat", "/work/status"], { allowFailure: true, quiet: true });
    if (result.status === 0 && result.stdout.trim() === "ready") return;
    if (result.stdout?.trim() === "failed") throw new Error("public API/PKCE fixture failed");
    run("sleep", ["3"]);
  }
  throw new Error("public API/PKCE fixture timed out");
}
function refusal(name, podSpec, expected) {
  const metadata = { namespace: ns, name: `negative-${name}` };
  k(["delete", "job", "-n", ns, metadata.name, "--ignore-not-found"], { quiet: true });
  apply({ apiVersion: "batch/v1", kind: "Job", metadata, spec: { backoffLimit: 0, activeDeadlineSeconds: 90, template: { spec: podSpec } } });
  k(["wait", "-n", ns, `job/${metadata.name}`, "--for=condition=Failed", "--timeout=120s"], { timeout: 130000 });
  const logs = k(["logs", "-n", ns, `job/${metadata.name}`, "--all-containers"], { quiet: true });
  if (!logs.includes(expected)) throw new Error(`${name} failed without the expected refusal stage`);
  for (const value of Object.values(passwords)) {
    if (logs.includes(value)) throw new Error("a negative diagnostic disclosed a credential");
  }
  console.log(`PASS ${name} refusal`);
}
const passwords = Object.fromEntries(["postgres_bootstrap_password", "synveda_migrator_password", "synveda_gateway_password", "synveda_worker_password"].map((name) => [name, randomBytes(32).toString("hex")]));
try {
  const clusters = run("kind", ["get", "clusters"]).trim().split(/\s+/);
  if (clusters.includes(cluster)) {
    if (process.env.REUSE !== "1") throw new Error("cluster exists; explicit REUSE=1 is required");
  } else {
    run("kind", ["create", "cluster", "--name", cluster, "--kubeconfig", env.KUBECONFIG, "--config", "demos/fixtures/ops-2/kind-cluster.yaml", "--wait", "120s"], { timeout: 240000 });
    owned = true;
    createdCluster = true;
  }
  file("kubeconfig", run("kind", ["get", "kubeconfig", "--name", cluster]));
  for (const namespace of [providers, ns]) {
    const existing = k(["get", "namespace", namespace, "--ignore-not-found", "-o", "name"]);
    if (existing.trim()) throw new Error("fixture namespaces already exist; select an empty disposable cluster");
  }
  owned = true;
  if (process.env.SKIP_BUILD !== "1") {
    for (const [image, dockerfile] of [[product, "deploy/compose/product/Dockerfile"], [postgres, "deploy/compose/postgres/Dockerfile"], [keycloak, "deploy/compose/keycloak/Dockerfile"]]) {
      console.log(`Building ${image} from ${dockerfile}`);
      run("docker", ["build", "-t", image, "-f", dockerfile, "."], { timeout: 1800000 });
    }
  }
  console.log("Loading source-built provider and product images");
  run("kind", ["load", "docker-image", "--name", cluster, product, postgres, keycloak], { timeout: 300000 });
  for (const name of [providers, ns]) apply({ apiVersion: "v1", kind: "Namespace", metadata: { name } });
  const apis = k(["api-resources", "--api-group=postgresql.cnpg.io", "-o", "name"]);
  if (apis.trim()) throw new Error("external acceptance must run without the CNPG API");
  run("openssl", ["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", join(scratch, "ca.key"), "-out", join(scratch, "ca.crt"), "-days", "2", "-subj", "/CN=OPS-11 disposable CA"], { quiet: true });
  const ca = readFileSync(join(scratch, "ca.crt"), "utf8");
  const pgTls = certificate("postgres", [host]);
  const clientTls = certificate("client", ["ops11-client"], "clientAuth");
  const idpTls = certificate("keycloak", [`keycloak.${providers}.svc.cluster.local`, `keycloak-alt.${providers}.svc.cluster.local`]);
  secret(providers, "provider-postgres", { ...passwords, ...pgTls, "ca.crt": ca });
  secret(ns, "synveda-postgres-client", clientTls);
  secret(providers, "provider-keycloak-tls", idpTls);
  secret(ns, "synveda-provider-ca", { "ca.crt": ca });
  const roles = { migrator: "synveda_migrator", gateway: "synveda_gateway", worker: "synveda_worker", administrators: ["postgres"], administrative_memberships: [], forbidden_databases: ["postgres", "template1"], isolated_peer_roles: [] };
  apply({ apiVersion: "v1", kind: "ConfigMap", metadata: { namespace: providers, name: "provider-roles" }, data: { "roles.json": JSON.stringify(roles) + "\n", "pg_hba.conf": "local all all trust\nhost all postgres all scram-sha-256\nhostssl all all all scram-sha-256 clientcert=verify-ca\nhostnossl all all all reject\n" } });
  console.log("Provisioning independent PostgreSQL 17 with a private-CA server certificate");
  const mounts = [{ name: "private", mountPath: "/run/secrets" }, { name: "data", mountPath: "/var/lib/postgresql/data" }, { name: "run", mountPath: "/var/run/postgresql" }, { name: "tmp", mountPath: "/tmp" }];
  apply({ apiVersion: "apps/v1", kind: "Deployment", metadata: { namespace: providers, name: "postgres" }, spec: { replicas: 1, strategy: { type: "Recreate" }, selector: { matchLabels: { app: "postgres" } }, template: { metadata: { labels: { app: "postgres" } }, spec: {
    automountServiceAccountToken: false, securityContext,
    initContainers: [{ name: "private-inputs", image: postgres, command: ["sh", "-ec", "umask 077; cp /source/* /run/secrets/; cp /roles/roles.json /run/secrets/database_roles.json; cp /roles/pg_hba.conf /run/secrets/pg_hba.conf; chmod 600 /run/secrets/*"], securityContext: containerSecurity, volumeMounts: [{ name: "source", mountPath: "/source", readOnly: true }, { name: "roles", mountPath: "/roles", readOnly: true }, mounts[0]] }],
    containers: [{ name: "postgres", image: postgres, securityContext: containerSecurity, args: ["postgres", "-c", "ssl=on", "-c", "ssl_cert_file=/run/secrets/tls.crt", "-c", "ssl_key_file=/run/secrets/tls.key", "-c", "ssl_ca_file=/run/secrets/ca.crt", "-c", "hba_file=/run/secrets/pg_hba.conf"],
      env: [{ name: "POSTGRES_USER", value: "postgres" }, { name: "POSTGRES_DB", value: "postgres" }, { name: "POSTGRES_PASSWORD_FILE", value: "/run/secrets/postgres_bootstrap_password" }, { name: "PGDATA", value: "/var/lib/postgresql/data/pgdata" }],
      readinessProbe: { exec: { command: ["pg_isready", "-U", "postgres"] }, periodSeconds: 3 }, volumeMounts: mounts }],
    volumes: [{ name: "source", secret: { secretName: "provider-postgres", defaultMode: 0o440 } }, { name: "roles", configMap: { name: "provider-roles" } }, ...["private", "data", "run", "tmp"].map((name) => ({ name, emptyDir: {} }))],
  } } } });
  for (const name of ["postgres", "postgres-bad"]) apply({ apiVersion: "v1", kind: "Service", metadata: { namespace: providers, name }, spec: { selector: { app: "postgres" }, ports: [{ port: 5432, targetPort: 5432 }] } });
  rollout("postgres", providers);
  k(["exec", "-n", providers, "deployment/postgres", "--", "env", "SYNVEDA_POSTGRES_BOOTSTRAP_URL=postgresql://postgres@postgres:5432/postgres", "SYNVEDA_POSTGRES_BUNDLED_CLUSTER=true", "SYNVEDA_DATABASE_BOOTSTRAP_PRIVATE_DIR=/run/secrets", "SYNVEDA_DATABASE_ROLES_FILE=/run/secrets/database_roles.json", "/usr/local/bin/synveda-database-bootstrap", "synveda"], { timeout: 120000 });
  for (const role of ["migrator", "gateway", "worker"]) {
    const url = `postgresql://synveda_${role}:${passwords[`synveda_${role}_password`]}@${host}:5432/synveda?sslmode=verify-full&sslrootcert=/run/secrets/synveda-postgres/ca.crt&sslcert=/run/secrets/synveda-postgres-client/tls.crt&sslkey=/run/secrets/synveda-postgres-client/tls.key`;
    secret(ns, `synveda-${role}-db`, { DATABASE_URL: url });
  }
  const idpSecrets = { keycloak_admin_username: "synveda-bootstrap" };
  for (const name of ["postgres_owner_password", "keycloak_database_password", "keycloak_admin_password", "keycloak_convergence_admin_password", "keycloak_demo_admin_password", "keycloak_demo_approver_password", "keycloak_demo_member_password", "keycloak_demo_viewer_password"]) idpSecrets[name] = randomBytes(32).toString("hex");
  secret(providers, "ops2-keycloak", idpSecrets);
  secret(ns, "ops2-keycloak", { keycloak_demo_admin_password: idpSecrets.keycloak_demo_admin_password });
  let idp = readFileSync("demos/fixtures/ops-2/keycloak.yaml", "utf8")
    .replaceAll("namespace: synveda-test", `namespace: ${providers}`)
    .replaceAll("ghcr.io/synveda/keycloak:__IMAGE_TAG__", keycloak)
    .replaceAll("http://keycloak.synveda-test.svc.cluster.local:8080", auth)
    .replace('            - { name: KC_HTTP_ENABLED, value: "true" }', '            - { name: KC_HTTP_ENABLED, value: "true" }\n            - { name: KC_HTTP_MANAGEMENT_SCHEME, value: http }\n            - { name: KC_HTTPS_CERTIFICATE_FILE, value: /tls/tls.crt }\n            - { name: KC_HTTPS_CERTIFICATE_KEY_FILE, value: /tls/tls.key }')
    .replace('            - { name: http, containerPort: 8080 }', '            - { name: http, containerPort: 8080 }\n            - { name: https, containerPort: 8443 }')
    .replace('            - { name: server-secrets, mountPath: /run/secrets, readOnly: true }', '            - { name: provider-tls, mountPath: /tls, readOnly: true }\n            - { name: server-secrets, mountPath: /run/secrets, readOnly: true }')
    .replace('        - name: server-secrets\n          emptyDir:', '        - name: provider-tls\n          secret:\n            secretName: provider-keycloak-tls\n        - name: server-secrets\n          emptyDir:')
    // The fixture's private admin convergence still uses its existing HTTP
    // endpoint. Application discovery, login and tokens use only HTTPS.
    .replace('    - { name: http, port: 8080, targetPort: http }', '    - { name: http, port: 8080, targetPort: http }\n    - { name: https, port: 8443, targetPort: https }');
  k(["apply", "-f", "-"], { input: idp, quiet: true });
  rollout("keycloak-postgres", providers); rollout("keycloak", providers);
  apply({ apiVersion: "v1", kind: "Service", metadata: { namespace: providers, name: "keycloak-alt" }, spec: { selector: { app: "keycloak" }, ports: [{ port: 8443, targetPort: 8443 }] } });
  secret(ns, "synveda-kms", { SYNVEDA_KMS_KEY: randomBytes(32).toString("hex"), SYNVEDA_KMS_KEY_REF: "local:ops11-test" });
  secret(ns, "synveda-oidc", { SYNVEDA_OIDC_ISSUERS: issuer() });
  console.log("Installing application chart on a clean database; providers are outside the release");
  helm();
  const manifest = run("helm", ["get", "manifest", "synveda", "-n", ns]);
  if (/kind: (Cluster|StatefulSet|PersistentVolumeClaim|Secret)\n/.test(manifest) || manifest.includes("synveda-pg-superuser") || manifest.includes("database-bootstrap")) throw new Error("external release owns a provider or administrator credential");
  const ssl = k(["exec", "-n", providers, "deployment/postgres", "--", "psql", "-U", "postgres", "-d", "postgres", "-Atc", "select count(*) > 0 and bool_and(s.ssl) from pg_stat_ssl s join pg_stat_activity a using(pid) where a.usename in ('synveda_gateway','synveda_worker')"]);
  if (ssl.trim() !== "t") throw new Error("runtime database connections are not TLS");
  console.log("PASS clean install and TLS runtime connections");
  apply({ apiVersion: "v1", kind: "ConfigMap", metadata: { namespace: ns, name: "install-test-scripts" }, data: { "client.sh": readFileSync("demos/fixtures/ops-2/client.sh", "utf8"), "browser.mjs": readFileSync("demos/fixtures/ops-2/browser.mjs", "utf8") } });
  let client = readFileSync("demos/fixtures/ops-2/client-pod.yaml", "utf8")
    .replaceAll("ghcr.io/synveda/product:__IMAGE_TAG__", product)
    .replaceAll("http://keycloak.synveda-test.svc.cluster.local:8080", auth)
    .replace('    - name: operator-secret', '    - name: ca\n      secret:\n        secretName: synveda-provider-ca\n    - name: operator-secret')
    .replace('        - name: IDP_URL', '        - name: NODE_EXTRA_CA_CERTS\n          value: /ca/ca.crt\n        - name: IDP_URL')
    .replace('        - { name: operator-secret, mountPath:', '        - { name: ca, mountPath: /ca, readOnly: true }\n        - { name: operator-secret, mountPath:');
  k(["delete", "pod", "-n", ns, "synveda-install-test", "--ignore-not-found"], { quiet: true });
  k(["apply", "-f", "-"], { input: client, quiet: true });
  waitClient();
  console.log("PASS real Keycloak PKCE, governed Session/context and audit round trip");

  const installedJob = JSON.parse(k(["get", "job", "synveda-install-1", "-n", ns, "-o", "json"]));
  const probeSpec = structuredClone(installedJob.spec.template.spec);
  probeSpec.containers = [probeSpec.initContainers.find((c) => c.name === "database-preflight")];
  delete probeSpec.initContainers;
  probeSpec.containers[0].command = ["/usr/local/bin/synveda-container"];
  probeSpec.containers[0].args = ["database-preflight"];
  const dbNegative = (name, url, caSecret = "synveda-provider-ca", expectedHost = host) => {
    secret(ns, `negative-${name}`, { DATABASE_URL: url });
    const spec = structuredClone(probeSpec);
    spec.volumes.find((v) => v.name === "preflight-databases").projected.sources[0].secret.name = `negative-${name}`;
    spec.volumes.find((v) => v.name === "postgres-ca").secret.secretName = caSecret;
    spec.containers[0].env.find((v) => v.name === "SYNVEDA_DATABASE_EXPECTED_HOST").value = expectedHost;
    refusal(name, spec, "connection failed");
  };
  const migratorUrl = `postgresql://synveda_migrator:${passwords.synveda_migrator_password}@${host}:5432/synveda?sslmode=verify-full&sslrootcert=/run/secrets/synveda-postgres/ca.crt&sslcert=/run/secrets/synveda-postgres-client/tls.crt&sslkey=/run/secrets/synveda-postgres-client/tls.key`;
  dbNegative("database-password", migratorUrl.replace(passwords.synveda_migrator_password, randomBytes(32).toString("hex")));
  run("openssl", ["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", join(scratch, "bad.key"), "-out", join(scratch, "bad.crt"), "-days", "2", "-subj", "/CN=untrusted disposable CA"], { quiet: true });
  secret(ns, "bad-ca", { "ca.crt": readFileSync(join(scratch, "bad.crt"), "utf8") });
  dbNegative("database-ca", migratorUrl, "bad-ca");
  const badHost = `postgres-bad.${providers}.svc.cluster.local`;
  dbNegative("database-hostname", migratorUrl.replace(host, badHost), "synveda-provider-ca", badHost);
  const gateway = JSON.parse(k(["get", "deployment", "synveda", "-n", ns, "-o", "json"]));
  const oidcSpec = structuredClone(gateway.spec.template.spec);
  oidcSpec.restartPolicy = "Never";
  const diagnostic = oidcSpec.containers[0];
  delete diagnostic.startupProbe; delete diagnostic.livenessProbe; delete diagnostic.readinessProbe;
  diagnostic.command = ["/usr/local/bin/synveda-container"]; diagnostic.args = ["issuer-diagnostic"];
  diagnostic.env.push({ name: "SYNVEDA_BOOTSTRAP_TENANT_ID", value: tenant }, { name: "SYNVEDA_OIDC_EXPECTED_ISSUER", value: `${auth}/realms/synveda` });
  const caSpec = structuredClone(oidcSpec);
  caSpec.volumes.find((v) => v.name === "oidc-ca").secret.secretName = "bad-ca";
  refusal("issuer-ca", caSpec, "OIDC");
  const alias = auth.replace("keycloak.", "keycloak-alt.");
  secret(ns, "bad-issuer", { SYNVEDA_OIDC_ISSUERS: issuer({ issuer: `${alias}/realms/synveda` }) });
  const issuerSpec = structuredClone(oidcSpec);
  issuerSpec.volumes.find((v) => v.name === "runtime-secrets").projected.sources[0].secret.name = "bad-issuer";
  issuerSpec.containers[0].env.find((v) => v.name === "SYNVEDA_OIDC_EXPECTED_ISSUER").value = `${alias}/realms/synveda`;
  refusal("issuer-canonical-name", issuerSpec, "OIDC");

  secret(ns, "synveda-oidc", { SYNVEDA_OIDC_ISSUERS: issuer({ audience: "wrong-api" }) });
  k(["rollout", "restart", "-n", ns, "deployment/synveda"]); rollout("synveda");
  const code = k(["exec", "-n", ns, "synveda-install-test", "-c", "client", "--", "sh", "-ec", 'bearer=$(synveda auth token); curl --silent --output /dev/null --write-out "%{http_code}" -H "Authorization: Bearer $bearer" "$SYNVEDA_GATEWAY/v1/me"']);
  if (code.trim() !== "401") throw new Error("invalid audience was not rejected");
  console.log("PASS actual bearer with invalid configured audience is rejected");
  secret(ns, "synveda-oidc", { SYNVEDA_OIDC_ISSUERS: issuer() });
  k(["rollout", "restart", "-n", ns, "deployment/synveda", "deployment/synveda-worker"]); rollout("synveda"); rollout("synveda-worker");
  k(["exec", "-n", ns, "synveda-install-test", "-c", "client", "--", "synveda", "whoami", "--json"], { quiet: true });
  helm();
  k(["wait", "-n", ns, "job/synveda-install-2", "--for=condition=Complete", "--timeout=300s"]);
  k(["exec", "-n", ns, "synveda-install-test", "-c", "client", "--", "synveda", "audit", "verify", "--json"], { quiet: true });
  console.log("PASS gateway/worker restart, normal Helm upgrade and migration/tenant rerun");
  console.log("OPS-11 external acceptance passed; private HTTP application access only, no cloud/OpenShift/HA claim");
} catch (error) {
  console.error(error.message);
  if (owned) {
    for (const namespace of [providers, ns]) {
      const result = k(["get", "pods", "-n", namespace, "-o", "wide"], { allowFailure: true });
      process.stderr.write(result.stdout ?? "");
    }
  }
  process.exitCode = 1;
} finally {
  if (createdCluster && process.env.KEEP !== "1") run("kind", ["delete", "cluster", "--name", cluster], { allowFailure: true });
  if (owned && (!createdCluster || process.env.KEEP === "1")) console.log(`Disposable ${cluster} retained by REUSE/KEEP; kind delete cluster --name ${cluster}`);
  rmSync(scratch, { recursive: true, force: true });
}
