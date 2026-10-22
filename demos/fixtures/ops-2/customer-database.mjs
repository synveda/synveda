// OPS-11: customer-operated PostgreSQL for starter.mjs's external-external case.
// A genuine PGDG server for one admitted major is provisioned only by the DBA
// script that the product image renders. Refusals run before Helm, against
// the still-empty database, so "nothing changed" is checkable.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

// Same pinned server as the db-test minimum-boundary refusal.
export const unsupportedImage = "postgres:13.23-bookworm@sha256:f0cffcc050a9f1f3c78a9968e221badc1cdd02e2ae15b1de9b12bee1ba5ea2db";
export const customerImage = (major) => `synveda/postgres-fixture:${major}-ops11`;
export const customerRoles = { migrator: "synveda_migrator", gateway: "synveda_gateway", worker: "synveda_worker", administrators: ["postgres"], administrative_memberships: [], forbidden_databases: ["postgres", "template1"], isolated_peer_roles: [] };
const loginRoles = ["synveda_migrator", "synveda_gateway", "synveda_worker"];

export function buildCustomerImage(run, major) {
  run("docker", ["build", "--target", `postgres-${major}-fixture`, "-t", customerImage(major), "-f", "deploy/compose/postgres/Dockerfile", "."], { timeout: 600000 });
}

const quote = (value) => `'${value.replaceAll("'", "''")}'`;

function dba(c, server, sql, { db = "postgres", allowFailure = false } = {}) {
  return c.k(["exec", "-i", "-n", c.providers, `deployment/${server}`, "--", "psql", "-X", "-qAt", "-v", "ON_ERROR_STOP=1", "-U", "postgres", "-d", db], { input: sql, quiet: true, allowFailure });
}

const publicRelations = (c, server) => dba(c, server, "select count(*) from pg_catalog.pg_class where relnamespace = 'public'::regnamespace and relkind <> 'e';", { db: "synveda" }).trim();

function renderSetup(c) {
  const script = c.run("docker", ["run", "--rm", "--network", "none", "-e", `SYNVEDA_DATABASE_ROLES=${JSON.stringify(customerRoles)}`, c.product, "database-setup-sql"]);
  assert.match(script, /BETWEEN 14 AND 18/, "rendered setup script lacks the server-major guard");
  for (const value of Object.values(c.passwords)) assert.ok(!script.includes(value), "setup script contains a credential");
  return script;
}

function setPasswords(c, server) {
  dba(c, server, `set password_encryption = 'scram-sha-256';\n${loginRoles.map((role) => `alter role ${role} password ${quote(c.passwords[`${role}_password`])};`).join("\n")}\n`);
}

function databaseUrl(c, role, host, database = "synveda") {
  return `postgresql://${role}:${c.passwords[`${role}_password`]}@${host}:5432/${database}?sslmode=verify-full&sslrootcert=/run/secrets/synveda-postgres/ca.crt`;
}

// One ordinary product-image process with the chart's file contract. Its logs
// are inspected in memory and never written to the report.
async function refusal(c, name, { command, host, urls = {}, roles = customerRoles, caSecret = "provider-ca", expected }) {
  const pod = `refusal-${name}`;
  const files = Object.fromEntries(loginRoles.map((role) => [`${role.slice("synveda_".length)}_database_url`, urls[role] ?? databaseUrl(c, role, host)]));
  c.secret(c.ns, pod, files);
  c.apply(c.ns, { apiVersion: "v1", kind: "ConfigMap", metadata: { name: pod }, data: { "roles.json": JSON.stringify(roles) + "\n" } });
  const env = [
    { name: "DATABASE_URL_FILE", value: "/run/secrets/urls/migrator_database_url" },
    ...loginRoles.map((role) => ({ name: `SYNVEDA_${role.slice("synveda_".length).toUpperCase()}_DATABASE_URL_FILE`, value: `/run/secrets/urls/${role.slice("synveda_".length)}_database_url` })),
    { name: "SYNVEDA_DATABASE_ROLES_FILE", value: "/etc/synveda/database/roles.json" },
    { name: "SYNVEDA_DATABASE_EXPECTED_HOST", value: host },
    { name: "SYNVEDA_DATABASE_EXPECTED_PORT", value: "5432" },
    { name: "SYNVEDA_DATABASE_EXPECTED_NAME", value: "synveda" },
    { name: "SYNVEDA_DATABASE_EXPECTED_ROOT_CERT_FILE", value: "/run/secrets/synveda-postgres/ca.crt" },
  ];
  c.apply(c.ns, { apiVersion: "v1", kind: "Pod", metadata: { name: pod }, spec: {
    restartPolicy: "Never", automountServiceAccountToken: false, securityContext: c.security,
    containers: [{ name: "product", image: c.product, imagePullPolicy: "Never", args: [command], securityContext: c.restricted, env,
      volumeMounts: [{ name: "tmp", mountPath: "/tmp" }, { name: "ca", mountPath: "/run/secrets/synveda-postgres", readOnly: true }, { name: "urls", mountPath: "/run/secrets/urls", readOnly: true }, { name: "roles", mountPath: "/etc/synveda/database", readOnly: true }] }],
    volumes: [{ name: "tmp", emptyDir: { sizeLimit: "64Mi" } }, { name: "ca", secret: { secretName: caSecret, defaultMode: 0o444, items: [{ key: "ca.crt", path: "ca.crt" }] } }, { name: "urls", secret: { secretName: pod, defaultMode: 0o444 } }, { name: "roles", configMap: { name: pod } }],
  } });
  let phase;
  for (let attempt = 0; attempt < 90; attempt++) {
    phase = c.k(["get", "pod", "-n", c.ns, pod, "-o", "jsonpath={.status.phase}"]).trim();
    if (["Succeeded", "Failed"].includes(phase)) break;
    await sleep(2000);
  }
  const logs = c.k(["logs", "-n", c.ns, pod], { quiet: true });
  c.k(["delete", "pod,secret,configmap", "-n", c.ns, pod, "--wait=true"], { quiet: true });
  assert.equal(phase, "Failed", `${name}: ${command} was not refused`);
  for (const value of Object.values(c.passwords)) assert.ok(!logs.includes(value), `${name}: the refusal disclosed a credential`);
  assert.ok(logs.includes(expected), `${name}: ${command} was refused for another reason`);
  console.log(`PASS ${name} refused by ${command} without credentials`);
  return { case: name, stage: command, refusal: expected };
}

function badCa(c) {
  const key = join(c.scratch, "untrusted-ca.key"), crt = join(c.scratch, "untrusted-ca.crt");
  c.run("openssl", ["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", key, "-out", crt, "-days", "2", "-subj", "/CN=OPS-11 untrusted CA"], { quiet: true });
  c.secret(c.ns, "untrusted-ca", { "ca.crt": readFileSync(crt, "utf8") });
}

async function unsupportedMajor(c, script) {
  const host = c.providerDatabase(c.providers, c.passwords, customerRoles, c.ca, { name: "unsupported-postgres", image: unsupportedImage, databases: [] });
  const server = "unsupported-postgres";
  const setup = dba(c, server, script, { allowFailure: true });
  assert.notEqual(setup.status, 0, "the setup script accepted PostgreSQL 13");
  assert.ok((setup.stderr ?? "").includes("Synveda admits PostgreSQL 14, 15, 16, 17 or 18; nothing was changed"), "the setup script refused PostgreSQL 13 for another reason");
  assert.equal(dba(c, server, `select count(*) from pg_catalog.pg_roles where rolname in ('synveda_app', ${loginRoles.map(quote).join(", ")});`).trim(), "0", "the PostgreSQL 13 setup refusal changed roles");
  console.log("PASS unknown-major refused by the DBA setup script before any change");
  // An operator who bypasses the script still meets the product refusal.
  dba(c, server, `set password_encryption = 'scram-sha-256';\ncreate role synveda_migrator login password ${quote(c.passwords.synveda_migrator_password)};\ncreate database synveda owner synveda_migrator;\n`);
  const result = await refusal(c, "unknown-major", { command: "migrate", host, expected: "PostgreSQL server major 13 is not supported" });
  assert.equal(publicRelations(c, server), "0", "the PostgreSQL 13 refusal changed the database");
  c.k(["delete", "deployment/unsupported-postgres", "service/unsupported-postgres", "secret/unsupported-postgres-inputs", "configmap/unsupported-postgres-config", "pvc/unsupported-postgres-data", "-n", c.providers, "--ignore-not-found", "--wait=false"], { quiet: true });
  return { ...result, setupScript: "refused before any change", publicRelationsAfter: 0 };
}

function serverFacts(c, major) {
  const server = "customer-postgres";
  const [version, versionNum] = dba(c, server, "select current_setting('server_version') || '|' || current_setting('server_version_num');").trim().split("|");
  const extensions = dba(c, server, "select extname || ' ' || extversion from pg_catalog.pg_extension order by extname;", { db: "synveda" }).trim().split("\n");
  const pkg = c.k(["exec", "-n", c.providers, `deployment/${server}`, "--", "dpkg-query", "-W", "-f", "${Version}", `postgresql-${major}-pgvector`]).trim();
  return { server: version, serverVersionNum: Number(versionNum), pgvectorPackage: pkg, extensions };
}

// Fails unless every Synveda login still has ordinary authority: no elevated
// attribute, no ownership of the public schema and one non-ADMIN capability membership.
export function verifyRuntimeAuthority(c) {
  const server = "customer-postgres";
  const attributes = dba(c, server, `select count(*) from pg_catalog.pg_roles where rolname in ('synveda_app', ${loginRoles.map(quote).join(", ")}) and (rolsuper or rolbypassrls or rolcreaterole or rolcreatedb or rolreplication);`).trim();
  assert.equal(attributes, "0", "a Synveda role gained an elevated attribute");
  const owner = dba(c, server, "select pg_catalog.pg_get_userbyid(nspowner) from pg_catalog.pg_namespace where nspname = 'public';", { db: "synveda" }).trim();
  assert.equal(owner, "synveda_migrator", "the public schema owner changed");
  const memberships = dba(c, server, `select coalesce(string_agg(member.rolname || '>' || role.rolname || ':' || m.admin_option, ',' order by member.rolname, role.rolname), '') from pg_catalog.pg_auth_members m join pg_catalog.pg_roles member on member.oid = m.member join pg_catalog.pg_roles role on role.oid = m.roleid where member.rolname in (${loginRoles.map(quote).join(", ")});`).trim();
  assert.equal(memberships, "synveda_gateway>synveda_app:false,synveda_worker>synveda_app:false", "runtime memberships changed");
  return { elevatedAttributes: 0, publicSchemaOwner: owner, runtimeMemberships: ["synveda_gateway → synveda_app (no ADMIN)", "synveda_worker → synveda_app (no ADMIN)"] };
}

export async function provisionCustomerDatabase(c) {
  const { major } = c;
  const host = c.providerDatabase(c.providers, c.passwords, customerRoles, c.ca, { name: "customer-postgres", image: customerImage(major), databases: [] });
  const server = "customer-postgres";
  const script = renderSetup(c);
  console.log(`Provisioning customer-operated PostgreSQL ${major} through the rendered DBA script (twice)`);
  dba(c, server, script); dba(c, server, script);
  setPasswords(c, server);
  assert.equal(publicRelations(c, server), "0");
  badCa(c);
  const refusals = [];

  // Extension drift: the DBA removes a required extension; migration refuses
  // before baseline DDL and a rerun of the same script restores it.
  dba(c, server, "drop extension btree_gin;", { db: "synveda" });
  refusals.push({ ...await refusal(c, "extension-mismatch", { command: "migrate", host, expected: "btree_gin 1.3" }), publicRelationsAfter: Number(publicRelations(c, server)) });
  dba(c, server, script);
  assert.equal(dba(c, server, "select extversion from pg_catalog.pg_extension where extname = 'btree_gin';", { db: "synveda" }).trim(), "1.3", "rerunning the setup script did not restore btree_gin");

  // A provider ADMIN-only management grant cannot exist on 14/15; on 16+ an
  // undeclared catalogue grant is the refusal. Either way, nothing changes.
  const providerShape = { ...customerRoles, administrative_memberships: [{ member: "postgres", grantor: "synveda_provider_grantor" }] };
  const shapeRefusal = Number(major) <= 15
    ? "PostgreSQL 14 and 15 cannot represent ADMIN-only role memberships"
    : "every protected database role must carry exactly the configured provider ADMIN-only member/grantor pairs";
  refusals.push({ ...await refusal(c, "provider-role-shape", { command: "migrate", host, roles: providerShape, expected: shapeRefusal }), publicRelationsAfter: Number(publicRelations(c, server)) });

  refusals.push(await refusal(c, "wrong-database-target", { command: "database-preflight", host, urls: { synveda_migrator: databaseUrl(c, "synveda_migrator", host, "postgres") }, expected: "SYNVEDA_MIGRATOR_DATABASE_URL_FILE does not select the deployment PostgreSQL endpoint and database" }));
  refusals.push(await refusal(c, "untrusted-certificate", { command: "database-preflight", host, caSecret: "untrusted-ca", expected: "SYNVEDA_MIGRATOR_DATABASE_URL_FILE connection failed" }));
  refusals.push(await unsupportedMajor(c, script));
  for (const entry of refusals.filter((entry) => "publicRelationsAfter" in entry)) assert.equal(entry.publicRelationsAfter, 0, `${entry.case} changed the database`);

  return { host, facts: { major: Number(major), ...serverFacts(c, major), provisioning: "synveda db setup-sql rendered by the product image's database-setup-sql role, run twice and once more after drift by the declared administrator postgres", administrator: "postgres (declared superuser of a dedicated server)", roles: customerRoles, refusals, authorityBeforeInstall: verifyRuntimeAuthority(c) } };
}

// Required traffic must keep working while an unrelated pod's ingress and the
// gateway's undeclared egress fail. Positive controls run before enforcement.
export async function enforceNetworkPolicy(c) {
  const { ns, providers } = c;
  c.k(["label", "-n", ns, "pod/team-test", "app=team-test"]);
  c.apply(providers, { apiVersion: "v1", kind: "Pod", metadata: { name: "unrelated", labels: { app: "unrelated" } }, spec: { restartPolicy: "Never", automountServiceAccountToken: false, securityContext: c.security,
    containers: [{ name: "probe", image: c.product, imagePullPolicy: "Never", command: ["sleep", "infinity"], securityContext: c.restricted }] } });
  c.k(["wait", "-n", providers, "pod/unrelated", "--for=condition=Ready", "--timeout=120s"], { timeout: 130000 });
  const gateway = `http://synveda.${ns}.svc.cluster.local:8120/v1/me`;
  const identity = `http://keycloak-http.${providers}.svc.cluster.local/realms/synveda/.well-known/openid-configuration`;
  const ingress = () => probeUrl(c, ["-n", providers, "unrelated"], gateway);
  const egress = () => probeUrl(c, ["-n", ns, "deployment/synveda"], identity);
  const before = { unrelatedIngress: ingress(), undeclaredEgress: egress() };
  assert.notEqual(before.unrelatedIngress, "000", "positive control: the unrelated pod cannot reach the gateway without policies");
  assert.notEqual(before.undeclaredEgress, "000", "positive control: the gateway cannot reach the identity backend without policies");
  const kubeDns = { namespaceSelector: { matchLabels: { "kubernetes.io/metadata.name": "kube-system" } }, podSelector: { matchLabels: { "k8s-app": "kube-dns" } } };
  c.values.networkPolicy = {
    enabled: true,
    ingress: [{ podSelector: { matchLabels: { app: "proxy" } } }, { podSelector: { matchLabels: { app: "team-test" } } }],
    dns: [kubeDns],
    database: [{ namespaceSelector: { matchLabels: { "kubernetes.io/metadata.name": providers } }, podSelector: { matchLabels: { app: "customer-postgres" } } }],
    oidc: [{ to: [{ podSelector: { matchLabels: { app: "proxy" } } }], ports: [{ protocol: "TCP", port: 8443 }] }],
  };
  // The upgrade reruns preflight, migration and tenant admission under the policies.
  c.helm(ns, c.values);
  let after;
  for (let attempt = 0; attempt < 10; attempt++) {
    after = { unrelatedIngress: ingress(), undeclaredEgress: egress() };
    if (after.unrelatedIngress === "000" && after.undeclaredEgress === "000") break;
    await sleep(3000);
  }
  assert.equal(after.unrelatedIngress, "000", "an unrelated pod still reaches the gateway");
  assert.equal(after.undeclaredEgress, "000", "the gateway still reaches an undeclared destination");
  c.team(ns, "verify");
  c.k(["delete", "pod", "-n", providers, "unrelated", "--wait=false"], { quiet: true });
  const cni = c.k(["get", "daemonset", "-n", "kube-system", "-o", "jsonpath={range .items[*]}{.metadata.name}={.spec.template.spec.containers[0].image}{\"\\n\"}{end}"]).trim().split("\n").filter((line) => !line.startsWith("kube-proxy="));
  console.log("PASS NetworkPolicy: declared traffic works; unrelated ingress and undeclared egress fail");
  return { cni, before, after, declaredTraffic: "install Job rerun, login, team reads, context and audit verification passed under the policies" };
}

function probeUrl(c, target, url) {
  const result = c.k(["exec", ...target, "--", "curl", "-s", "-o", "/dev/null", "-w", "%{http_code}", "--connect-timeout", "4", "--max-time", "8", url], { allowFailure: true });
  return (result.stdout ?? "").trim() || "000";
}
