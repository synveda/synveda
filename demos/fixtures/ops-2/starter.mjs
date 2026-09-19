#!/usr/bin/env node
// OPS-11: real provider matrix in a selected disposable cluster. No host kubeconfig changes.
import assert from "node:assert/strict";
import { spawnSync, spawn } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync, rmSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createHash, randomBytes } from "node:crypto";

const cluster = process.env.CLUSTER ?? "synveda-ops11-starter";
assert.match(cluster, /^synveda-ops11-starter(?:-[a-z0-9-]+)?$/);
const cases = ["cnpg-packaged", "external-packaged", "cnpg-external", "external-external"];
if (process.env.STARTER_CASE) assert.ok(cases.includes(process.env.STARTER_CASE));
for (const name of ["KEEP", "REUSE", "SKIP_BUILD"]) assert.ok([undefined, "0", "1"].includes(process.env[name]));
const scratch = mkdtempSync(join(tmpdir(), "synveda-starter-"));
const env = { ...process.env, KUBECONFIG: join(scratch, "kubeconfig") };
const chart = "deploy/helm/synveda";
const product = "synveda/product:ops11-starter";
const cnpg = "synveda/cnpg-postgres:17.11-ops11-starter";
const postgres = "synveda/postgres:ops11";
const keycloak = "synveda/keycloak:ops11";
const proxy = "synveda/proxy:2.11.4-dev";
const node = "node:22-bookworm-slim@sha256:83f487e0a63425e5b4d146fb5e5be574bcbe1b7b843d3ebafdd95eaf7767a7e5";
const tenant = "019b53c0-7c00-7000-8000-000000000012";
const report = { date: new Date().toISOString(), workload: "4 concurrent agents, each 20 batches of 5 session events and 20 context runs; deterministic extraction and lexical retrieval", cases: [] };
let owned = false;

function run(command, args, { input, timeout = 120000, quiet = false, allowFailure = false } = {}) {
  const result = spawnSync(command, args, { env, input, encoding: "utf8", timeout, maxBuffer: 8 * 1024 * 1024 });
  if (allowFailure) return result;
  if (result.error || result.status !== 0) {
    if (!input && !quiet) process.stderr.write(result.stderr ?? "");
    throw new Error(`${command} ${args[0]} failed (exit ${result.status ?? "timeout"})`);
  }
  return result.stdout;
}
const k = (args, options) => run("kubectl", args, options);
const file = (name, value) => { const path = join(scratch, name); writeFileSync(path, value, { mode: 0o600 }); return path; };
const apply = (namespace, object) => k(["apply", "-n", namespace, "-f", "-"], { input: JSON.stringify(object), quiet: true });
const secret = (namespace, name, stringData) => apply(namespace, { apiVersion: "v1", kind: "Secret", metadata: { name }, type: "Opaque", stringData });
const parseYaml = (source) => JSON.parse(run("ruby", ["-rjson", "-ryaml", "-e", "puts JSON.generate(YAML.load(STDIN.read))"], { input: source }));
function merge(base, patch) {
  for (const [key, value] of Object.entries(patch)) {
    base[key] = value && typeof value === "object" && !Array.isArray(value) ? merge(base[key] ?? {}, value) : value;
  }
  return base;
}
const security = { runAsNonRoot: true, runAsUser: 65532, runAsGroup: 65532, fsGroup: 65532, seccompProfile: { type: "RuntimeDefault" } };
const restricted = { allowPrivilegeEscalation: false, readOnlyRootFilesystem: true, capabilities: { drop: ["ALL"] } };
function wait(namespace, kind, name) {
  // kubectl rollout status refuses OnDelete StatefulSets.
  const args = kind === "statefulset"
    ? ["wait", "-n", namespace, `${kind}/${name}`, "--for=jsonpath={.status.readyReplicas}=1", "--timeout=600s"]
    : ["rollout", "status", "-n", namespace, `${kind}/${name}`, "--timeout=600s"];
  k(args, { timeout: 620000 });
}
function quiesce(namespace, identityNamespace) {
  k(["scale", "-n", namespace, "deployment/synveda", "deployment/synveda-worker", "--replicas=0"]);
  k(["scale", "-n", identityNamespace, "statefulset/keycloak", "--replicas=0"]);
  wait(namespace, "deployment", "synveda"); wait(namespace, "deployment", "synveda-worker");
  k(["wait", "-n", identityNamespace, "pod/keycloak-0", "--for=delete", "--timeout=180s"], { timeout: 200000 });
}
function certificate(name, hosts) {
  run("openssl", ["req", "-new", "-newkey", "rsa:2048", "-nodes", "-keyout", join(scratch, `${name}.key`), "-out", join(scratch, `${name}.csr`), "-subj", `/CN=${hosts[0]}`], { quiet: true });
  const ext = file(`${name}.ext`, `subjectAltName=${hosts.map((h) => `DNS:${h}`).join(",")}\nextendedKeyUsage=serverAuth\n`);
  run("openssl", ["x509", "-req", "-in", join(scratch, `${name}.csr`), "-CA", join(scratch, "ca.crt"), "-CAkey", join(scratch, "ca.key"), "-CAcreateserial", "-out", join(scratch, `${name}.crt`), "-days", "2", "-sha256", "-extfile", ext], { quiet: true });
  return { "tls.crt": readFileSync(join(scratch, `${name}.crt`), "utf8"), "tls.key": readFileSync(join(scratch, `${name}.key`), "utf8") };
}
function service(namespace, name, selector, port, targetPort = port) {
  apply(namespace, { apiVersion: "v1", kind: "Service", metadata: { name }, spec: { selector, ports: [{ port, targetPort }] } });
}
function helm(namespace, values, release = "synveda", path = chart) {
  const filename = file(`${namespace}-${release}-values.json`, JSON.stringify(values));
  console.log(`Installing/upgrading ${namespace}/${release}`);
  run("helm", ["upgrade", "--install", release, path, "-n", namespace, "-f", filename, "--wait", "--wait-for-jobs", "--timeout", "15m"], { timeout: 920000 });
}
function providerDatabase(namespace, passwords, roles, ca) {
  const host = `postgres.${namespace}.svc.cluster.local`;
  secret(namespace, "provider-inputs", { ...passwords, ...certificate(namespace, [host]), "ca.crt": ca });
  apply(namespace, { apiVersion: "v1", kind: "ConfigMap", metadata: { name: "provider-config" }, data: {
    "roles.json": JSON.stringify(roles) + "\n",
    "pg_hba.conf": "local all all trust\nhost all postgres all scram-sha-256\nhostssl all all all scram-sha-256\nhostnossl all all all reject\n",
  } });
  apply(namespace, { apiVersion: "v1", kind: "PersistentVolumeClaim", metadata: { name: "provider-data" }, spec: { accessModes: ["ReadWriteOnce"], resources: { requests: { storage: "2Gi" } } } });
  const mounts = ["private", "data", "run", "tmp"].map((name, i) => ({ name, mountPath: ["/run/secrets", "/var/lib/postgresql/data", "/var/run/postgresql", "/tmp"][i] }));
  apply(namespace, { apiVersion: "apps/v1", kind: "Deployment", metadata: { name: "postgres" }, spec: { replicas: 1, strategy: { type: "Recreate" }, selector: { matchLabels: { app: "postgres" } }, template: { metadata: { labels: { app: "postgres" } }, spec: {
    automountServiceAccountToken: false, securityContext: { ...security, runAsUser: 999, runAsGroup: 999, fsGroup: 999 },
    initContainers: [{ name: "private-inputs", image: postgres, command: ["sh", "-ec", "umask 077; cp /source/* /run/secrets/; cp /config/roles.json /run/secrets/database_roles.json; cp /config/pg_hba.conf /run/secrets/pg_hba.conf; chmod 600 /run/secrets/*"], securityContext: restricted, volumeMounts: [{ name: "source", mountPath: "/source", readOnly: true }, { name: "config", mountPath: "/config", readOnly: true }, mounts[0]] }],
    containers: [{ name: "postgres", image: postgres, securityContext: restricted, args: ["postgres", "-c", "ssl=on", "-c", "ssl_cert_file=/run/secrets/tls.crt", "-c", "ssl_key_file=/run/secrets/tls.key", "-c", "hba_file=/run/secrets/pg_hba.conf"],
      env: [{ name: "POSTGRES_USER", value: "postgres" }, { name: "POSTGRES_DB", value: "postgres" }, { name: "POSTGRES_PASSWORD_FILE", value: "/run/secrets/postgres_bootstrap_password" }, { name: "PGDATA", value: "/var/lib/postgresql/data/pgdata" }],
      readinessProbe: { exec: { command: ["pg_isready", "-U", "postgres"] }, periodSeconds: 3 }, volumeMounts: mounts }],
    volumes: [{ name: "source", secret: { secretName: "provider-inputs" } }, { name: "config", configMap: { name: "provider-config" } }, { name: "data", persistentVolumeClaim: { claimName: "provider-data" } }, ...["private", "run", "tmp"].map((name) => ({ name, emptyDir: {} }))],
  } } } });
  service(namespace, "postgres", { app: "postgres" }, 5432);
  wait(namespace, "deployment", "postgres");
  k(["exec", "-n", namespace, "deployment/postgres", "--", "psql", "-X", "-v", "ON_ERROR_STOP=1", "-U", "postgres", "-d", "postgres", "-c", "REVOKE CONNECT, TEMPORARY ON DATABASE postgres, template1 FROM PUBLIC;"]);
  for (const database of ["synveda", "keycloak"]) {
    // GNU chmod needs five digits to clear the volume's inherited setgid bit.
    k(["exec", "-n", namespace, "deployment/postgres", "--", "sh", "-ec", "mkdir -p /tmp/authority; chmod 00700 /tmp/authority; exec env SYNVEDA_POSTGRES_BOOTSTRAP_URL=postgresql://postgres@postgres:5432/postgres SYNVEDA_POSTGRES_BUNDLED_CLUSTER=true SYNVEDA_DATABASE_BOOTSTRAP_PRIVATE_DIR=/run/secrets SYNVEDA_DATABASE_ROLES_FILE=/run/secrets/database_roles.json SYNVEDA_DATABASE_AUTHORITY_DIR=/tmp/authority SYNVEDA_DATABASE_REQUIRE_KEYCLOAK_PASSWORD=true /usr/local/bin/synveda-database-bootstrap " + database], { timeout: 180000, quiet: true });
  }
  return host;
}
function proxyDeployment(namespace, app, auth, identityBackend) {
  secret(namespace, "proxy-tls", certificate(`${namespace}-proxy`, [new URL(app).hostname, new URL(auth).hostname]));
  const caddy = `{
 admin off
 auto_https disable_redirects
}
${app} {
 tls /tls/tls.crt /tls/tls.key
 @public path /console* /auth/* /v1/* /scim/v2*
 handle @public {
  reverse_proxy synveda:8120
 }
 handle {
  respond 404
 }
}
${auth} {
 tls /tls/tls.crt /tls/tls.key
 @public path /realms/synveda/* /resources/*
 handle @public {
  reverse_proxy ${identityBackend}
 }
 handle {
  respond 404
 }
}
`;
  apply(namespace, { apiVersion: "v1", kind: "ConfigMap", metadata: { name: "proxy" }, data: { Caddyfile: caddy } });
  apply(namespace, { apiVersion: "apps/v1", kind: "Deployment", metadata: { name: "proxy" }, spec: { replicas: 1, selector: { matchLabels: { app: "proxy" } }, template: { metadata: { labels: { app: "proxy" } }, spec: {
    automountServiceAccountToken: false, securityContext: security,
    containers: [{ name: "proxy", image: proxy, command: ["caddy", "run", "--config", "/etc/caddy/Caddyfile", "--adapter", "caddyfile"], securityContext: restricted,
      volumeMounts: [{ name: "config", mountPath: "/etc/caddy", readOnly: true }, { name: "tls", mountPath: "/tls", readOnly: true }, { name: "data", mountPath: "/data" }, { name: "runtime", mountPath: "/config" }] }],
    volumes: [{ name: "config", configMap: { name: "proxy" } }, { name: "tls", secret: { secretName: "proxy-tls" } }, { name: "data", emptyDir: {} }, { name: "runtime", emptyDir: {} }],
  } } } });
  for (const name of ["app", "auth"]) service(namespace, name, { app: "proxy" }, 8443);
  wait(namespace, "deployment", "proxy");
}
function credentialsHash(namespace) {
  const list = JSON.parse(k(["get", "secrets", "-n", namespace, "-o", "json"], { quiet: true })).items;
  return createHash("sha256").update(JSON.stringify(list.filter((s) => s.metadata.name.startsWith("synveda-") && !s.metadata.name.endsWith("-server")).sort((a, b) => a.metadata.name.localeCompare(b.metadata.name)).map((s) => [s.metadata.name, s.data]))).digest("hex");
}
function team(namespace, phase) {
  console.log(`${namespace}: ${phase}`);
  const out = k(["exec", "-n", namespace, "team-test", "-c", "node", "--", "node", "/scripts/starter-team.mjs", phase], { timeout: 240000 });
  process.stdout.write(out);
  return JSON.parse(out.trim().split("\n").at(-1));
}
async function measuredWorkload(namespace, providers) {
  console.log(`${namespace}: measured load`);
  const nodeName = JSON.parse(k(["get", "nodes", "-o", "json"])).items[0].metadata.name;
  const samples = [];
  const sample = () => {
    const stats = JSON.parse(k(["get", "--raw", `/api/v1/nodes/${nodeName}/proxy/stats/summary`]));
    samples.push({ at: new Date().toISOString(), pods: stats.pods.filter((p) => [namespace, providers].includes(p.podRef.namespace) && p.podRef.name !== "team-test").map((p) => ({ namespace: p.podRef.namespace, pod: p.podRef.name, cpuNanoCores: p.cpu?.usageNanoCores, workingSetBytes: p.memory?.workingSetBytes })) });
  };
  sample();
  const child = spawn("kubectl", ["exec", "-n", namespace, "team-test", "-c", "node", "--", "node", "/scripts/starter-team.mjs", "load"], { env, stdio: ["ignore", "pipe", "pipe"] });
  let output = "";
  child.stdout.on("data", (bytes) => { output += bytes; });
  child.stderr.on("data", (bytes) => process.stderr.write(bytes));
  await new Promise((resolve, reject) => {
    const timer = setInterval(sample, 2000);
    const timeout = setTimeout(() => { child.kill(); }, 240000);
    const finish = (error) => { clearInterval(timer); clearTimeout(timeout); error ? reject(error) : resolve(); };
    child.on("error", finish);
    child.on("close", (code) => finish(code === 0 ? undefined : new Error(`measured workload failed (${code})`)));
  });
  sample();
  const peaks = new Map();
  for (const { pods } of samples) for (const pod of pods) {
    const previous = peaks.get(pod.pod) ?? { ...pod, cpuNanoCores: null, workingSetBytes: null };
    const peak = (key) => Number.isFinite(pod[key]) ? Math.max(previous[key] ?? 0, pod[key]) : previous[key];
    peaks.set(pod.pod, { ...pod, cpuNanoCores: peak("cpuNanoCores"), workingSetBytes: peak("workingSetBytes") });
  }
  process.stdout.write(output);
  return { workload: JSON.parse(output.trim().split("\n").at(-1)), resources: { source: "kubelet stats/summary, sampled every 2 seconds; server pod peaks, excluding test clients, operator and cluster overhead; null means unavailable; not capacity guarantees", samples: samples.length, started: samples[0].at, ended: samples.at(-1).at, peaks: [...peaks.values()] } };
}
async function mcp(namespace, shouldDeny = false) {
  const child = spawn("kubectl", ["exec", "-i", "-n", namespace, "team-test", "-c", "cli", "--", "sh", "-ec", 'export SYNVEDA_TOKEN="$(cat /work/agent-token)"; exec synveda mcp --session "$(cat /work/agent-session)"'], { env, stdio: ["pipe", "pipe", "pipe"] });
  let buffer = "";
  await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => { child.kill(); reject(new Error("MCP timeout")); }, 60000);
    const finish = (error) => { clearTimeout(timeout); child.stdin.end(); child.kill(); error ? reject(error) : resolve(); };
    child.on("error", finish);
    child.stderr.on("data", () => {});
    child.stdout.on("data", (bytes) => {
      buffer += bytes;
      for (;;) {
        const at = buffer.indexOf("\n"); if (at < 0) break;
        const line = buffer.slice(0, at); buffer = buffer.slice(at + 1);
        let response; try { response = JSON.parse(line); } catch { continue; }
        if (response.id === 1) {
          child.stdin.write(JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }) + "\n");
          child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id: 2, method: "tools/call", params: { name: "recall", arguments: { query: "starter release train Thursday" } } }) + "\n");
        }
        if (response.id === 2) {
          const denied = response.error !== undefined || response.result?.isError === true;
          if (denied !== shouldDeny || (!denied && !JSON.stringify(response.result).includes("Thursday"))) finish(new Error("MCP authority/content assertion failed"));
          else finish();
        }
      }
    });
    child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "ops11-starter", version: "1" } } }) + "\n");
  });
  console.log(`PASS MCP ${shouldDeny ? "revocation denial" : "service-identity context retrieval"}`);
}

try {
  const existing = run("kind", ["get", "clusters"]).trim().split(/\s+/).includes(cluster);
  if (existing) assert.equal(process.env.REUSE, "1", "explicit REUSE=1 required");
  else run("kind", ["create", "cluster", "--name", cluster, "--kubeconfig", env.KUBECONFIG, "--config", "demos/fixtures/ops-2/kind-cluster.yaml", "--wait", "120s"], { timeout: 240000 });
  owned = !existing;
  file("kubeconfig", run("kind", ["get", "kubeconfig", "--name", cluster]));
  if (process.env.SKIP_BUILD !== "1") {
    for (const [image, source] of [[product, "product"], [postgres, "postgres"], [keycloak, "keycloak"], [proxy, "proxy"]]) run("docker", ["build", "-t", image, "-f", `deploy/compose/${source}/Dockerfile`, "."], { timeout: 1200000 });
    run("docker", ["build", "-t", cnpg, "-f", "deploy/helm/postgres/Dockerfile", "."], { timeout: 600000 });
  }
  // The fixture's public, digest-pinned Node image is pulled by Kubernetes.
  // Import only source-built images: Docker may retain an incomplete upstream
  // multi-platform index which kind's all-platforms archive importer rejects.
  for (const image of [product, postgres, cnpg, keycloak, proxy]) run("kind", ["load", "docker-image", "--name", cluster, image], { timeout: 600000 });
  const operator = file("cnpg.yaml", run("curl", ["-fLsS", "https://github.com/cloudnative-pg/cloudnative-pg/releases/download/v1.30.0/cnpg-1.30.0.yaml"]));
  assert.equal(createHash("sha256").update(readFileSync(operator)).digest("hex"), "f8bede43fe4ee0d478c2355b204a36876b2ae4faac60f2a9452280b293da3b88");
  k(["apply", "--server-side", "-f", operator]);
  k(["wait", "-n", "cnpg-system", "deployment/cnpg-controller-manager", "--for=condition=Available", "--timeout=300s"], { timeout: 320000 });
  run("openssl", ["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", join(scratch, "ca.key"), "-out", join(scratch, "ca.crt"), "-days", "2", "-subj", "/CN=OPS-11 disposable starter CA"], { quiet: true });
  const ca = readFileSync(join(scratch, "ca.crt"), "utf8");
  report.kubernetes = JSON.parse(k(["version", "-o", "json"])).serverVersion.gitVersion;
  report.engine = run("docker", ["info", "--format", "{{.NCPU}} CPUs; {{.MemTotal}} bytes engine RAM"]).trim();
  for (const selected of process.env.STARTER_CASE ? [process.env.STARTER_CASE] : cases) {
    const [database, identity] = selected.split("-");
    const ns = `starter-${selected}`, providers = `${ns}-providers`;
    for (const namespace of [ns, providers]) {
      assert.equal(k(["get", "namespace", namespace, "--ignore-not-found", "-o", "name"]).trim(), "", "fixture namespace exists; select a clean cluster");
      k(["create", "namespace", namespace]);
    }
    const app = `https://app.${ns}.svc.cluster.local:8443`, auth = `https://auth.${ns}.svc.cluster.local:8443`;
    const passwords = Object.fromEntries(["postgres_bootstrap_password", "synveda_migrator_password", "synveda_gateway_password", "synveda_worker_password", "keycloak_database_password"].map((key) => [key, randomBytes(32).toString("hex")]));
    const roles = { migrator: "synveda_migrator", gateway: "synveda_gateway", worker: "synveda_worker", administrators: ["postgres"], administrative_memberships: [], forbidden_databases: ["keycloak", "postgres", "template1"], isolated_peer_roles: ["keycloak"] };
    let externalHost;
    if (database === "external" || identity === "external") externalHost = providerDatabase(providers, passwords, roles, ca);
    const identityNs = identity === "packaged" ? ns : providers;
    const dbHost = database === "cnpg" && identity === "packaged" ? "synveda-pg-rw" : externalHost;
    secret(identityNs, "synveda-keycloak-db", { password: passwords.keycloak_database_password });
    const admin = { username: "starter-bootstrap", password: randomBytes(32).toString("hex") };
    secret(identityNs, "synveda-keycloak-admin", admin);
    secret(ns, "test-admin", admin);
    for (const namespace of [ns, providers]) secret(namespace, "provider-ca", { "ca.crt": ca });
    for (const role of ["gateway", "worker", ...(database === "external" ? ["migrator"] : [])]) {
      const host = database === "cnpg" ? "synveda-pg-rw" : externalHost;
      const tls = database === "external" ? "?sslmode=verify-full&sslrootcert=/run/secrets/synveda-postgres/ca.crt" : "";
      secret(ns, `synveda-${role}-db`, { DATABASE_URL: `postgresql://synveda_${role}:${passwords[`synveda_${role}_password`]}@${host}:5432/synveda${tls}`, password: passwords[`synveda_${role}_password`] });
    }
    secret(ns, "synveda-kms", { SYNVEDA_KMS_KEY: randomBytes(32).toString("hex"), SYNVEDA_KMS_KEY_REF: "local:ops11-starter-disposable" });
    secret(ns, "synveda-oidc", { SYNVEDA_OIDC_ISSUERS: JSON.stringify([{ issuer: `${auth}/realms/synveda`, client_id: "synveda", audience: "synveda-api", service_audiences: ["synveda-agents"], tenant: { static: { tenant_id: tenant } } }]) });
    const base = parseYaml(readFileSync(`${chart}/ci/${database}-values.yaml`, "utf8"));
    const values = merge(base, { fullnameOverride: "synveda", image: { repository: "synveda/product", tag: "ops11-starter", pullPolicy: "Never" }, gateway: { publicUrl: app, dbMaxConnections: 10 }, worker: { dbMaxConnections: 5 }, oidc: { caExistingSecret: "provider-ca" },
      postgres: database === "cnpg" ? { mode: "cnpg", image: cnpg, instances: 1, retain: true, primaryUpdateStrategy: "unsupervised", storage: { size: "2Gi" }, resources: { requests: { cpu: "250m", memory: "512Mi" } } } : { mode: "external", external: { host: externalHost, caExistingSecret: "provider-ca", roles } },
      install: { tenant: { id: tenant, slug: "starter", name: "Starter acceptance" } },
      keycloak: { enabled: true, fullnameOverride: "keycloak", image: { repository: "synveda/keycloak", tag: "ops11", pullPolicy: "Never" }, publicUrl: auth, proxyTrustedAddresses: "10.244.0.0/16", adminExistingSecret: "synveda-keycloak-admin", databaseCaExistingSecret: database === "cnpg" && identity === "packaged" ? "synveda-pg-ca" : "provider-ca", database: { hostname: dbHost, existingSecret: "synveda-keycloak-db" } },
    });
    const backend = `keycloak-http.${identityNs}.svc.cluster.local:80`;
    proxyDeployment(ns, app, auth, backend);
    if (identity === "external") {
      // Render the same initial realm, but the independent provider release owns it.
      const providerValues = merge(parseYaml(readFileSync(`${chart}/values.yaml`, "utf8")).keycloak, values.keycloak);
      const realmValues = structuredClone(values);
      realmValues.postgres = merge(parseYaml(readFileSync(`${chart}/ci/external-values.yaml`, "utf8")).postgres,
        { external: { host: externalHost, caExistingSecret: "provider-ca", roles } });
      const rendered = run("helm", ["template", "synveda", chart, "-f", file(`${ns}-realm-values.json`, JSON.stringify(realmValues))]);
      const realm = rendered.split(/^---\s*$/m).find((doc) => /name: synveda-keycloak-realm\n/.test(doc) && /^kind: ConfigMap$/m.test(doc));
      assert.ok(realm);
      k(["apply", "-n", providers, "-f", "-"], { input: realm, quiet: true });
      helm(providers, providerValues, "synveda", `${chart}/charts/keycloakx-7.3.2.tgz`);
      values.keycloak.enabled = false;
    }
    helm(ns, values);
    const secretHash = credentialsHash(ns);
    const claimIds = JSON.parse(k(["get", "pvc", "-n", ns, "-o", "json"])).items.map((p) => p.metadata.uid).sort();
    apply(ns, { apiVersion: "v1", kind: "ConfigMap", metadata: { name: "team-scripts" }, data: { "starter-team.mjs": readFileSync("demos/fixtures/ops-2/starter-team.mjs", "utf8") } });
    apply(ns, { apiVersion: "v1", kind: "Pod", metadata: { name: "team-test" }, spec: { restartPolicy: "Never", automountServiceAccountToken: false, securityContext: security,
      containers: [{ name: "node", image: node, command: ["sleep", "infinity"], securityContext: restricted,
        env: [{ name: "APP", value: app }, { name: "AUTH", value: auth }, { name: "ADMIN_URL", value: `http://${backend}` }, { name: "NODE_EXTRA_CA_CERTS", value: "/ca/ca.crt" }],
        volumeMounts: [{ name: "work", mountPath: "/work" }, { name: "scripts", mountPath: "/scripts", readOnly: true }, { name: "ca", mountPath: "/ca", readOnly: true }, { name: "admin", mountPath: "/admin", readOnly: true }] },
        { name: "cli", image: product, command: ["sleep", "infinity"], securityContext: restricted, env: [{ name: "SYNVEDA_GATEWAY", value: app }, { name: "SSL_CERT_FILE", value: "/ca/ca.crt" }], volumeMounts: [{ name: "work", mountPath: "/work" }, { name: "ca", mountPath: "/ca", readOnly: true }] }],
      volumes: [{ name: "work", emptyDir: {} }, { name: "scripts", configMap: { name: "team-scripts" } }, { name: "ca", secret: { secretName: "provider-ca" } }, { name: "admin", secret: { secretName: "test-admin" } }],
    } });
    k(["wait", "-n", ns, "pod/team-test", "--for=condition=Ready", "--timeout=120s"], { timeout: 130000 });
    const seed = team(ns, "seed");
    await mcp(ns);
    const { workload, resources } = await measuredWorkload(ns, providers);
    console.log(`${ns}: planned pod recreation`);
    // Quiesce clients for this single-instance maintenance test. Preserve the
    // provider's shutdown grace; force deletion would not prove safe recovery.
    quiesce(ns, identityNs);
    if (database === "cnpg") k(["delete", "pod", "-n", ns, "-l", "cnpg.io/cluster=synveda-pg", "--timeout=360s"], { timeout: 380000 });
    if (externalHost) { k(["rollout", "restart", "-n", providers, "deployment/postgres"]); wait(providers, "deployment", "postgres"); }
    if (database === "cnpg") k(["wait", "-n", ns, "cluster/synveda-pg", "--for=condition=Ready", "--timeout=360s"], { timeout: 380000 });
    k(["scale", "-n", identityNs, "statefulset/keycloak", "--replicas=1"]);
    wait(identityNs, "statefulset", "keycloak");
    k(["scale", "-n", ns, "deployment/synveda", "deployment/synveda-worker", "--replicas=1"]);
    wait(ns, "deployment", "synveda"); wait(ns, "deployment", "synveda-worker");
    team(ns, "verify");
    helm(ns, values);
    assert.equal(credentialsHash(ns), secretHash, "credentials changed during upgrade");
    team(ns, "verify");
    run("helm", ["uninstall", "synveda", "-n", ns, "--wait", "--timeout", "5m"], { timeout: 320000 });
    assert.deepEqual(JSON.parse(k(["get", "pvc", "-n", ns, "-o", "json"])).items.map((p) => p.metadata.uid).sort(), claimIds);
    helm(ns, values);
    assert.equal(credentialsHash(ns), secretHash, "credentials changed during reinstall");
    team(ns, "verify");
    const revoke = team(ns, "revoke");
    await mcp(ns, true);
    report.cases.push({ selection: selected, seed, workload, revoke, resources, persistentCredentials: true, persistentContent: true, uninstallReinstall: true });
    console.log(`PASS ${selected}: install, PKCE/team/service/MCP, recreation, upgrade, retained reinstall and revocation`);
    quiesce(ns, identityNs);
    k(["delete", "namespace", ns, providers, "--wait=true", "--timeout=180s"], { timeout: 200000 });
  }
  mkdirSync("demos/evidence", { recursive: true });
  writeFileSync("demos/evidence/ops11-starter.json", JSON.stringify(report, null, 2) + "\n");
  console.log("PASS starter matrix; content-free measurements: demos/evidence/ops11-starter.json");
} finally {
  if (process.env.KEEP === "1") console.log(`KEEP=1: ${cluster}; private operator scratch: ${scratch}`);
  else {
    if (owned) run("kind", ["delete", "cluster", "--name", cluster], { timeout: 180000, allowFailure: true });
    rmSync(scratch, { recursive: true, force: true });
  }
}
