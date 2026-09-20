// OPS-11: opt-in loopback evaluation, with no Kubernetes access or API writes.
import { randomBytes } from "node:crypto";
import { spawnSync } from "node:child_process";
import { existsSync, lstatSync, readFileSync, writeFileSync } from "node:fs";

const output = "/output";
const stat = lstatSync(output);
if (!stat.isDirectory() || stat.isSymbolicLink() || (stat.mode & 0o077) || stat.uid !== process.getuid()) {
  throw new Error("output must be a private directory owned by the operator");
}
if (["secrets.json", "values.json"].some((name) => existsSync(`${output}/${name}`))) {
  for (const name of ["secrets.json", "values.json"]) {
    if (!existsSync(`${output}/${name}`)) throw new Error("incomplete preparation; preserve the directory and inspect it before retrying in a new empty directory");
    const saved = lstatSync(`${output}/${name}`);
    if (!saved.isFile() || saved.isSymbolicLink() || (saved.mode & 0o077) || saved.uid !== process.getuid()) throw new Error("existing configuration permissions refused");
  }
  console.log("Existing configuration preserved; passwords were not rotated.");
  process.exit(0);
}
const password = () => randomBytes(32).toString("hex");
const write = (name, value) => writeFileSync(`${output}/${name}`, value, { mode: 0o600, flag: "wx" });
const openssl = (...args) => {
  const result = spawnSync("openssl", args, { cwd: "/tmp", stdio: "pipe", timeout: 15000 });
  if (result.status !== 0) throw new Error("local certificate preparation failed");
};
openssl("req", "-x509", "-newkey", "rsa:3072", "-nodes", "-days", "365", "-subj", "/CN=Synveda local evaluation", "-keyout", "ca.key", "-out", "ca.crt");
openssl("req", "-new", "-newkey", "rsa:3072", "-nodes", "-subj", "/CN=synveda-pg-rw", "-keyout", "server.key", "-out", "server.csr");
writeFileSync("/tmp/extensions", "subjectAltName=DNS:synveda-pg-rw,DNS:synveda-pg-rw.synveda-evaluation.svc.cluster.local\nextendedKeyUsage=serverAuth\n", { mode: 0o600 });
openssl("x509", "-req", "-in", "server.csr", "-CA", "ca.crt", "-CAkey", "ca.key", "-CAcreateserial", "-days", "365", "-extfile", "extensions", "-out", "server.crt");
const secrets = [];
const secret = (name, stringData) => secrets.push({ apiVersion: "v1", kind: "Secret", metadata: { name, namespace: "synveda-evaluation" }, type: "Opaque", stringData });
secret("synveda-postgres-admin", { password: password() });
for (const role of ["migrator", "gateway", "worker"]) {
  const value = password();
  secret(`synveda-${role}-db`, { password: value, uri: `postgresql://synveda_${role}:${value}@synveda-pg-rw:5432/synveda?sslmode=verify-full&sslrootcert=/run/secrets/synveda-postgres/ca.crt` });
}
secret("synveda-pg-tls", { "tls.crt": readFileSync("/tmp/server.crt", "utf8"), "tls.key": readFileSync("/tmp/server.key", "utf8"), "ca.crt": readFileSync("/tmp/ca.crt", "utf8") });
secret("synveda-keycloak-db", { password: password() });
secret("synveda-kms", { SYNVEDA_KMS_KEY: password(), SYNVEDA_KMS_KEY_REF: "local-evaluation-v1" });
const tenant = "019b53c0-7c00-7000-8000-000000000045";
secret("synveda-oidc", { SYNVEDA_OIDC_ISSUERS: JSON.stringify([{
  issuer: "http://localhost:8081/realms/synveda", discovery_url: "http://synveda-keycloak-http:80/realms/synveda/.well-known/openid-configuration",
  client_id: "synveda", audience: "synveda-api", algorithms: ["RS256"], tenant: { static: { tenant_id: tenant } }, groups_claim: "groups", login_scopes: ["openid", "profile", "email"],
}]) });
const accounts = Object.fromEntries(["admin", "member", "approver", "viewer"].map((role) => [role, password()]));
secret("synveda-evaluation-accounts", accounts);
const values = {
  fullnameOverride: "synveda", embedder: "deterministic",
  gateway: { publicUrl: "http://localhost:8120", insecureDevelopmentHttp: true, databaseExistingSecret: "synveda-gateway-db", databaseUrlSecretKey: "uri" },
  worker: { databaseExistingSecret: "synveda-worker-db", databaseUrlSecretKey: "uri" },
  oidc: { existingSecret: "synveda-oidc" }, kms: { existingSecret: "synveda-kms" },
  postgres: { mode: "bundled", bundled: { administratorExistingSecret: "synveda-postgres-admin", migratorExistingSecret: "synveda-migrator-db", tlsExistingSecret: "synveda-pg-tls" } },
  keycloak: { enabled: true, localEvaluation: true, publicUrl: "http://localhost:8081", proxyTrustedAddresses: "127.0.0.1/32", databaseCaExistingSecret: "synveda-pg-tls", evaluationAccountsExistingSecret: "synveda-evaluation-accounts", database: { hostname: "synveda-pg-rw", existingSecret: "synveda-keycloak-db" } },
  install: { tenant: { id: tenant, slug: "synveda", name: "Synveda" } },
};
// secrets.json is the completion boundary; never rewrite it on another render.
write("values.json", `${JSON.stringify(values, null, 2)}\n`);
write("secrets.json", `${JSON.stringify({ apiVersion: "v1", kind: "List", items: secrets }, null, 2)}\n`);
console.log("Prepared opt-in evaluation identities, database TLS and encryption material. No cluster was contacted.");
