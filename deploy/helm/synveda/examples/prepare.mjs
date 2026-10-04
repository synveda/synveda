#!/usr/bin/env node
// OPS-11/OPS-12: derive ordinary Helm values; private state owns retry identity.
import { createHash, randomBytes, X509Certificate } from "node:crypto";
import { spawnSync } from "node:child_process";
import { closeSync, existsSync, fsyncSync, lstatSync, mkdirSync, openSync, readFileSync, renameSync, rmSync, rmdirSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
export const json = (value) => `${JSON.stringify(value, null, 2)}\n`;
export function parseJson(bytes, label) {
  try { return JSON.parse(bytes); }
  catch { throw new Error(`${label} contains malformed JSON; preserve the file and restore matching inputs before retrying`); }
}
const digest = (value) => createHash("sha256").update(value).digest("hex");
export function imagePins(bytes, inputs) {
  // Release overlays use only nested mappings and scalar OCI references.
  // Reading that closed inventory shape keeps preparation dependency-free.
  const stack = []; const fields = {};
  for (const line of bytes.split("\n")) {
    if (!line.trim() || line.trimStart().startsWith("#")) continue;
    const match = /^( *)([a-zA-Z][a-zA-Z0-9]*):(?:\s+(.*))?$/.exec(line);
    if (!match) throw new Error("images must be the ordinary mapping-only digest overlay from the verified release");
    const depth = match[1].length;
    while (stack.length && stack.at(-1).depth >= depth) stack.pop();
    const path = [...stack.map((x) => x.key), match[2]].join(".");
    if (match[3] === undefined) stack.push({ depth, key: match[2] });
    else { if (path in fields) throw new Error("duplicate image inventory field; use one merged release overlay"); fields[path] = match[3].replace(/^['"]|['"]$/g, ""); }
  }
  const pinned = (value) => /(?:^|@)sha256:[a-f0-9]{64}$/.test(value ?? "");
  if (!pinned(fields["image.digest"]) || (inputs.identity === "bundled" && !pinned(fields["keycloak.image.digest"])) ||
      (inputs.database === "bundled" && !pinned(fields["postgres.bundled.image"])) ||
      (inputs.database === "cnpg" && !pinned(fields["postgres.image"]))) throw new Error("digest overlay lacks a pinned selected product/database/identity image; use the complete matching verified release inventory, never a version-tag fallback");
  return fields;
}
export function privatePath(path, directory = false) {
  const stat = lstatSync(path);
  if (stat.isSymbolicLink() || (directory ? !stat.isDirectory() : !stat.isFile() || stat.nlink !== 1) ||
      (stat.mode & 0o077) || stat.uid !== process.getuid()) {
    throw new Error(`private ownership/type/permissions required for ${path}; preserve it and correct access deliberately before retrying`);
  }
  return stat;
}
export function atomicPrivate(path, bytes) {
  // The state commit precedes derived files. A killed writer leaves no new identity.
  const temporary = `${path}.tmp-${process.pid}-${randomBytes(6).toString("hex")}`;
  const fd = openSync(temporary, "wx", 0o600);
  try { writeFileSync(fd, bytes); fsyncSync(fd); } finally { closeSync(fd); }
  renameSync(temporary, path);
  const parent = openSync(dirname(path), "r");
  try { fsyncSync(parent); } finally { closeSync(parent); }
}
export function options(args) {
  const result = {};
  const allowed = new Set(["output", "route", "database", "identity", "release", "namespace", "context", "storage-class", "storage-size", "existing-claim", "app-port", "identity-port", "app-url", "identity-url", "ingress-class", "app-tls-secret", "identity-tls-secret", "proxy-cidrs", "admin-user", "values", "issuer-file", "images", "tenant-id"]);
  for (let i = 0; i < args.length; i += 2) {
    const key = args[i]?.replace(/^--/, "");
    if (!allowed.has(key) || !args[i + 1] || args[i + 1].startsWith("--") || key in result) throw new Error(`unknown, duplicate or incomplete option ${args[i]}`);
    result[key] = args[i + 1];
  }
  return result;
}
function name(value, label) {
  if (!/^[a-z][a-z0-9-]{0,29}$/.test(value ?? "")) throw new Error(`${label} must be a DNS label of at most 30 characters (reserves room for chart suffixes)`);
  return value;
}
function origin(value, label) {
  let url;
  try { url = new URL(value); } catch { throw new Error(`${label} must be an HTTPS origin`); }
  if (url.protocol !== "https:" || url.username || url.password || url.pathname !== "/" || url.search || url.hash || url.origin !== value) throw new Error(`${label} must be a canonical HTTPS origin without a trailing slash, path or credentials`);
  return value;
}
function port(value, fallback) {
  const n = Number(value ?? fallback);
  if (!Number.isInteger(n) || n < 1024 || n > 65535) throw new Error("local ports must be between 1024 and 65535");
  return n;
}
function merge(a, b) {
  for (const [key, value] of Object.entries(b)) {
    if (["__proto__", "constructor", "prototype"].includes(key)) throw new Error("unsafe values key");
    a[key] = value && typeof value === "object" && !Array.isArray(value) ? merge(a[key] ?? {}, value) : value;
  }
  return a;
}
export function normalise(o) {
  const route = o.route ?? "local";
  if (!["local", "shared", "cnpg"].includes(route)) throw new Error("route must be local, shared or cnpg");
  const database = o.database ?? (route === "cnpg" ? "cnpg" : "bundled");
  const identity = o.identity ?? "bundled";
  if (!["bundled", "external", "cnpg"].includes(database) || !["bundled", "external"].includes(identity) ||
      (route === "cnpg") !== (database === "cnpg") || (route === "local" && (database !== "bundled" || identity !== "bundled"))) throw new Error("local requires bundled dependencies; select route=cnpg only for existing CNPG; shared permits bundled/external independently");
  const release = name(o.release ?? "synveda", "release");
  const namespace = name(o.namespace ?? (route === "local" ? "synveda-evaluation" : "synveda"), "namespace");
  if (!o.context || /[\x00-\x1f\x7f"'`$\\]/.test(o.context)) throw new Error("context is required without control characters or shell substitutions; supply the operator-selected kubectl context name");
  if (database === "external" && (o["storage-class"] || o["storage-size"] || o["existing-claim"])) throw new Error("external PostgreSQL has no application PVC; remove storage options");
  if (o["existing-claim"] && (database !== "bundled" || o["storage-class"] || o["storage-size"])) throw new Error("existing-claim requires bundled PostgreSQL without size/class overrides; it must contain this installation's matching data");
  const appPort = port(o["app-port"], 8120);
  const identityPort = port(o["identity-port"], 8081);
  if (appPort === identityPort) throw new Error("application and identity local ports must differ");
  if (route !== "local" && (o["app-port"] || o["identity-port"])) throw new Error("port overrides apply only to local evaluation");
  const appUrl = route === "local" ? `http://localhost:${appPort}` : origin(o["app-url"], "app-url");
  const identityUrl = route === "local" ? `http://localhost:${identityPort}` : identity === "bundled" ? origin(o["identity-url"], "identity-url") : null;
  if (route === "local" && (o["app-url"] || o["identity-url"] || o["admin-user"] || o["ingress-class"] || o["proxy-cidrs"] || o["app-tls-secret"] || o["identity-tls-secret"])) throw new Error("local uses explicit evaluation identities and loopback ports; remove shared inputs");
  if (route !== "local" && (!o["ingress-class"] || !o["app-tls-secret"] || (identity === "bundled" && (!o["admin-user"] || !o["proxy-cidrs"] || !o["identity-tls-secret"])))) throw new Error("shared HTTPS requires ingress-class and app-tls-secret; bundled identity also requires admin-user, proxy-cidrs and identity-tls-secret");
  if (identity === "external" && (!o["issuer-file"] || o["identity-url"] || o["admin-user"] || o["proxy-cidrs"] || o["identity-tls-secret"])) throw new Error("external identity requires issuer-file and no bundled identity inputs");
  const values = o.values ? parseJson(readFileSync(resolve(o.values), "utf8"), "values file") : {};
  if (!values || typeof values !== "object" || Array.isArray(values)) throw new Error("values file must contain one ordinary Helm values object");
  if (values.postgres && values.postgres.mode && values.postgres.mode !== database) throw new Error("values postgres.mode conflicts with selected database");
  // Input is an ordinary Helm overlay. All credential inputs must stay references.
  if (/"(?:password|DATABASE_URL|SYNVEDA_KMS_KEY|SYNVEDA_OIDC_ISSUERS|client_secret)"\s*:/.test(JSON.stringify(values))) throw new Error("values must contain Secret references, never credentials or issuer JSON");
  if (database === "external" && (!values.postgres?.external?.host || values.postgres.external.host.endsWith(".example.test"))) throw new Error("external database requires --values JSON with its actual postgres.external contract and Secret references");
  let issuers = null;
  if (o["issuer-file"]) {
    privatePath(resolve(o["issuer-file"]));
    issuers = parseJson(readFileSync(resolve(o["issuer-file"]), "utf8"), "private issuer file");
    if (!Array.isArray(issuers) || issuers.length !== 1 || !issuers[0] || !issuers[0].client_id || !issuers[0].audience || !issuers[0].groups_claim || "client_secret" in issuers[0]) throw new Error("issuer-file requires one HTTPS public-client trust entry with client_id, audience, groups_claim and no client_secret");
    for (const field of ["issuer", ...(issuers[0].discovery_url ? ["discovery_url"] : [])]) {
      let url;
      try { url = new URL(issuers[0][field]); } catch { throw new Error("issuer-file URLs must be valid HTTPS URLs without credentials"); }
      if (url.protocol !== "https:" || url.username || url.password || url.hash) throw new Error("issuer-file URLs must be valid HTTPS URLs without credentials");
    }
    if (identity !== "external") throw new Error("issuer-file applies only to external identity");
  }
  const tenantId = o["tenant-id"] ?? issuers?.[0]?.tenant?.static?.tenant_id ?? null;
  if (tenantId && !/^[a-f0-9]{8}-[a-f0-9]{4}-7[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/.test(tenantId)) throw new Error("tenant-id must be UUIDv7; preserve the existing issuer's static tenant binding");
  if (issuers && !tenantId) throw new Error("external issuer requires its stable tenant.static.tenant_id; generate it once with the manual preparation worksheet");
  if (issuers && o["tenant-id"] && tenantId !== issuers[0].tenant?.static?.tenant_id) throw new Error("tenant-id conflicts with issuer-file static binding");
  const storageSize = o["storage-size"] ?? "20Gi";
  if (!/^[1-9][0-9]*(Gi|Ti)$/.test(storageSize)) throw new Error("storage-size must be a positive Gi or Ti quantity");
  return { route, database, identity, release, namespace, context: o.context, appUrl, identityUrl,
    storageClass: o["storage-class"] ?? "", storageSize, existingClaim: o["existing-claim"] ?? "",
    ingressClass: o["ingress-class"] ?? "", appTlsSecret: o["app-tls-secret"] ?? "", identityTlsSecret: o["identity-tls-secret"] ?? "",
    proxyCidrs: o["proxy-cidrs"] ?? "", adminUser: o["admin-user"] ?? "", tenantId, issuers, values };
}
export function uuid7() {
  const bytes = randomBytes(16);
  bytes.writeUIntBE(Date.now(), 0, 6); bytes[6] = (bytes[6] & 15) | 0x70; bytes[8] = (bytes[8] & 63) | 0x80;
  const hex = bytes.toString("hex"); return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}
function tlsMaterial(output, host, namespace) {
  const work = join(output, `.tls-${process.pid}`);
  mkdirSync(work, { mode: 0o700 });
  const openssl = (...args) => {
    const r = spawnSync("openssl", args, { cwd: work, stdio: "pipe", timeout: 20000 });
    if (r.status !== 0) throw new Error("OpenSSL database certificate preparation failed; install OpenSSL and retry the same output directory");
  };
  try {
    writeFileSync(join(work, "ca-config"), "[req]\ndistinguished_name=dn\n[dn]\nCN=Synveda database CA\n[ca]\nbasicConstraints=critical,CA:true\nkeyUsage=critical,keyCertSign,cRLSign\nsubjectKeyIdentifier=hash\n", { mode: 0o600 });
    openssl("req", "-x509", "-newkey", "rsa:3072", "-nodes", "-days", "365", "-subj", "/CN=Synveda database CA", "-config", "ca-config", "-extensions", "ca", "-keyout", "ca.key", "-out", "ca.crt");
    openssl("req", "-new", "-newkey", "rsa:3072", "-nodes", "-subj", `/CN=${host}`, "-keyout", "server.key", "-out", "server.csr");
    writeFileSync(join(work, "extensions"), `subjectAltName=DNS:${host},DNS:${host}.${namespace}.svc,DNS:${host}.${namespace}.svc.cluster.local\nextendedKeyUsage=serverAuth\n`, { mode: 0o600 });
    openssl("x509", "-req", "-in", "server.csr", "-CA", "ca.crt", "-CAkey", "ca.key", "-CAcreateserial", "-days", "365", "-extfile", "extensions", "-out", "server.crt");
    openssl("verify", "-CAfile", "ca.crt", "-purpose", "sslserver", "server.crt");
    if (!new X509Certificate(readFileSync(join(work, "server.crt"))).checkHost(host)) throw new Error("generated database certificate SAN mismatch; preserve inputs and retry the same preparation");
    return Object.fromEntries(["ca.key", "ca.crt", "server.key", "server.crt"].map((p) => [p, readFileSync(join(work, p), "utf8")]));
  } finally { rmSync(work, { recursive: true, force: true }); }
}
export function derive(state) {
  const p = state.inputs; const r = p.release; const n = (s) => `${r}-${s}`;
  const base = p.database === "cnpg" ? "existing-cnpg.json" : `${p.database}-database.json`;
  const values = merge(JSON.parse(readFileSync(join(here, base), "utf8")), p.values);
  values.fullnameOverride = r;
  values.gateway = { ...values.gateway, publicUrl: p.appUrl, insecureDevelopmentHttp: p.route === "local", databaseExistingSecret: p.values.gateway?.databaseExistingSecret ?? n("gateway-db") };
  values.worker.databaseExistingSecret = p.values.worker?.databaseExistingSecret ?? n("worker-db");
  values.oidc.existingSecret = n("oidc"); values.kms.existingSecret = n("kms");
  values.install = { ...values.install, tenant: { id: state.tenant, slug: r, name: r } };
  if (p.database === "bundled") values.postgres.bundled = { ...values.postgres.bundled, administratorExistingSecret: n("postgres-admin"), migratorExistingSecret: n("migrator-db"), tlsExistingSecret: n("pg-ca"), size: p.storageSize, storageClass: p.storageClass, existingClaim: p.existingClaim };
  if (p.database === "cnpg") values.postgres.storage = { size: p.storageSize, storageClass: p.storageClass };
  if (p.route !== "local") values.ingress = { ...values.ingress, enabled: true, className: p.ingressClass, host: new URL(p.appUrl).hostname, tls: [{ hosts: [new URL(p.appUrl).hostname], secretName: p.appTlsSecret }] };
  if (p.identity === "bundled") {
    values.keycloak = merge(JSON.parse(readFileSync(join(here, "bundled-identity.json"), "utf8")), { keycloak: values.keycloak ?? {} }).keycloak;
    Object.assign(values.keycloak, { publicUrl: p.identityUrl, localEvaluation: p.route === "local", proxyTrustedAddresses: p.route === "local" ? "127.0.0.1/32" : p.proxyCidrs, adminExistingSecret: p.route === "local" ? "" : n("keycloak-admin"), evaluationAccountsExistingSecret: p.route === "local" ? n("evaluation-accounts") : "" });
    if (p.database !== "external") {
      values.keycloak.databaseCaExistingSecret = p.database === "cnpg" ? n("pg-ca") : n("pg-ca");
      Object.assign(values.keycloak.database, { hostname: n("pg-rw"), existingSecret: n("keycloak-db") });
    } else {
      if (!p.values.keycloak?.database?.hostname || !p.values.keycloak?.database?.existingSecret || !p.values.keycloak?.databaseCaExistingSecret) throw new Error("external DB + bundled identity requires its separate keycloak.database hostname/Secret and databaseCaExistingSecret in --values");
      const roles = values.postgres.external.roles;
      if (values.keycloak.database.hostname === values.postgres.external.host) {
        roles.forbidden_databases = [...new Set([...roles.forbidden_databases, "keycloak"])];
        roles.isolated_peer_roles = [...new Set([...roles.isolated_peer_roles, "keycloak"])];
      }
    }
    if (p.route !== "local") values.keycloak.ingress = { enabled: true, ingressClassName: p.ingressClass, tls: [{ hosts: [new URL(p.identityUrl).hostname], secretName: p.identityTlsSecret }] };
  } else { values.keycloak = { enabled: false }; }
  const items = [];
  const secret = (name, stringData) => items.push({ apiVersion: "v1", kind: "Secret", metadata: { name, namespace: p.namespace }, type: "Opaque", stringData });
  secret(n("kms"), { SYNVEDA_KMS_KEY: state.credentials.kms, SYNVEDA_KMS_KEY_REF: `${r}:primary-v1` });
  const issuer = p.issuers ?? [{ issuer: `${p.identityUrl}/realms/synveda`, ...(p.route === "local" ? { discovery_url: `http://${r}-keycloak-http:80/realms/synveda/.well-known/openid-configuration` } : {}), client_id: "synveda", audience: "synveda-api", service_audiences: ["synveda-agents"], algorithms: ["RS256"], groups_claim: "groups", login_scopes: ["openid", "profile", "email"], tenant: { static: { tenant_id: state.tenant } } }];
  secret(n("oidc"), { SYNVEDA_OIDC_ISSUERS: JSON.stringify(issuer) });
  if (p.database !== "external") {
    for (const role of ["gateway", "worker", ...(p.database === "bundled" ? ["migrator"] : [])]) {
      const password = state.credentials[role];
      const url = `postgresql://synveda_${role}:${password}@${r}-pg-rw:5432/synveda${p.database === "bundled" ? "?sslmode=verify-full&sslrootcert=/run/secrets/synveda-postgres/ca.crt" : ""}`;
      secret(values[role]?.databaseExistingSecret ?? n("migrator-db"), { password, [role === "migrator" ? "uri" : values[role].databaseUrlSecretKey ?? "DATABASE_URL"]: url });
    }
    if (p.database === "bundled") {
      secret(n("postgres-admin"), { password: state.credentials.postgres });
      secret(n("pg-ca"), { "tls.crt": state.tls["server.crt"], "tls.key": state.tls["server.key"], "ca.crt": state.tls["ca.crt"] });
    }
    if (p.identity === "bundled") secret(n("keycloak-db"), { password: state.credentials.keycloak });
  }
  if (p.identity === "bundled" && p.route !== "local") secret(n("keycloak-admin"), { username: p.adminUser, password: state.credentials.admin });
  if (p.route === "local") secret(n("evaluation-accounts"), state.accounts);
  return { values, secrets: { apiVersion: "v1", kind: "List", items } };
}
export function prepare(o) {
  if (Number(process.versions.node.split(".")[0]) < 22 || !process.getuid || process.getuid() === 0) throw new Error("preparation requires Node 22+ as an ordinary Unix account");
  if (!isAbsolute(o.output ?? "")) throw new Error("output must be an absolute private directory");
  const output = resolve(o.output);
  for (let p = dirname(output); p !== dirname(p); p = dirname(p)) if (lstatSync(p).isSymbolicLink()) throw new Error("output ancestors must not be symlinks");
  if (!existsSync(output)) mkdirSync(output, { mode: 0o700 });
  privatePath(output, true);
  const lock = join(output, "prepare.lock");
  try { mkdirSync(lock, { mode: 0o700 }); } catch { throw new Error(`preparation locked: ${lock}; wait for the other preparation, or confirm it stopped before removing only this empty lock directory and retrying`); }
  try {
    const inputs = normalise(o);
    if (!o.images) throw new Error("images is required; supply --images with the matching verified release digest overlay");
    const imageBytes = readFileSync(resolve(o.images), "utf8");
    imagePins(imageBytes, inputs);
    const evidenceHash = digest(imageBytes);
    const statePath = join(output, "state.json");
    let state;
    if (existsSync(statePath)) {
      privatePath(statePath);
      state = parseJson(readFileSync(statePath, "utf8"), "private preparation state");
      if (state.version !== 1 || json(state.inputs) !== json(inputs) || state.evidenceHash !== evidenceHash) throw new Error("saved preparation conflicts with selected inputs or image evidence; reuse the original context/namespace/release/issuer/values/overlay. For an intended change use the maintenance guide; never delete recovery state");
      if (digest(json({ tenant: state.tenant, credentials: state.credentials, tls: state.tls, accounts: state.accounts })) !== state.materialHash) throw new Error("saved private material changed or is incomplete; restore the matching original state.json from recovery before retrying");
      if (inputs.database === "bundled" && !new X509Certificate(state.tls["server.crt"]).checkHost(`${inputs.release}-pg-rw`)) throw new Error("saved database certificate does not match this release; restore original recovery material");
    } else {
      if (["values.json", "secrets.json", "release-images.yaml"].some((p) => existsSync(join(output, p)))) throw new Error("derived outputs exist without state.json; restore matching private state before retrying; credentials cannot be regenerated for retained data");
      const needed = ["kms", ...(inputs.database !== "external" ? ["gateway", "worker"] : []), ...(inputs.database === "bundled" ? ["postgres", "migrator"] : []), ...(inputs.database !== "external" && inputs.identity === "bundled" ? ["keycloak"] : []), ...(inputs.identity === "bundled" && inputs.route !== "local" ? ["admin"] : [])];
      const credentials = Object.fromEntries(needed.map((k) => [k, randomBytes(32).toString("hex")]));
      state = { version: 1, inputs, evidenceHash, tenant: inputs.tenantId ?? uuid7(), credentials, tls: inputs.database === "bundled" ? tlsMaterial(output, `${inputs.release}-pg-rw`, inputs.namespace) : null, accounts: inputs.route === "local" ? Object.fromEntries(["admin", "member", "approver", "viewer"].map((k) => [k, randomBytes(32).toString("hex")])) : null };
      state.materialHash = digest(json({ tenant: state.tenant, credentials: state.credentials, tls: state.tls, accounts: state.accounts }));
      // Validate the complete derivation before committing recovery material.
      derive(state);
      atomicPrivate(statePath, json(state));
    }
    const derived = derive(state);
    for (const [name, bytes] of Object.entries({ "values.json": json(derived.values), "secrets.json": json(derived.secrets), "release-images.yaml": imageBytes })) {
      const path = join(output, name);
      if (existsSync(path)) {
        privatePath(path);
        if (readFileSync(path, "utf8") !== bytes) throw new Error(`${name} conflicts with saved preparation; preserve both and restore the original matching file before retrying`);
      } else atomicPrivate(path, bytes);
    }
    return state;
  } finally { rmdirSync(lock); }
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const o = options(process.argv.slice(2)); const state = prepare(o);
    const p = state.inputs;
    console.log(`Prepared files only. Context: ${p.context}; namespace: ${p.namespace}; release: ${p.release}; exposure: ${p.appUrl}; database: ${p.database}; identity: ${p.identity}.`);
    console.log(`Private recovery: ${o.output}/state.json and secrets.json. Inspect values.json before applying. Credentials and tenant identity are preserved on retry.`);
  } catch (error) { console.error(error.message); process.exitCode = 78; }
}
