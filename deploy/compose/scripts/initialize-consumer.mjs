#!/usr/bin/env node
// OPS-12: volume initialization only. No network, Docker API or product SQL.
import { createHash, randomUUID } from "node:crypto";
import { chmodSync, chownSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { prepare } from "./prepare-evaluation.mjs";

const fail = (message) => { throw new Error(`consumer initialization: ${message}`); };
const readJson = (file) => JSON.parse(readFileSync(file, "utf8"));
function privateDirectory(directory, uid) {
  const stat = lstatSync(directory);
  if (!stat.isDirectory() || stat.isSymbolicLink() || (stat.mode & 0o7777) !== 0o700 || stat.uid !== uid) fail("private directory ownership or mode refused");
}
function privateBytes(file, uid) {
  const stat = lstatSync(file);
  if (!stat.isFile() || stat.isSymbolicLink() || (stat.mode & 0o7777) !== 0o600 || stat.nlink !== 1 || stat.uid !== uid || stat.size > 1024 * 1024) fail("private file ownership, mode or size refused");
  return readFileSync(file);
}
function put(file, bytes, uid) {
  if (lstatSync(file, { throwIfNoEntry: false })) {
    if (!privateBytes(file, uid).equals(Buffer.from(bytes))) fail("retained configuration differs; restore matching state instead of replacing credentials");
    return;
  }
  const temporary = `${file}.${randomUUID()}.preparing`;
  try {
    writeFileSync(temporary, bytes, { mode: 0o600, flag: "wx" });
    renameSync(temporary, file);
  } finally { rmSync(temporary, { force: true }); }
}
function inventory(directory, uid, prefix = "") {
  privateDirectory(directory, uid);
  return Object.fromEntries(readdirSync(directory).sort().flatMap((name) => {
    if (!/^[a-zA-Z0-9_.-]+$/.test(name)) fail("private filename refused");
    const file = path.join(directory, name);
    const stat = lstatSync(file);
    if (stat.isSymbolicLink()) fail("private symlink refused");
    if (stat.isDirectory()) return Object.entries(inventory(file, uid, `${prefix}${name}/`));
    return [[`${prefix}${name}`, createHash("sha256").update(privateBytes(file, uid)).digest("hex")]];
  }));
}

export function retainedState(root, retainedDatabase, identity, uid) {
  const binding = path.join(root, "consumer.json");
  const seal = path.join(root, "consumer-secrets.json");
  const secretDirectory = path.join(root, "synveda-evaluation/secrets");
  if (retainedDatabase && (!existsSync(binding) || !existsSync(seal))) fail("retained PostgreSQL requires its matching installation volume and encryption keys; no credentials were generated");
  if (existsSync(binding)) {
    if (privateBytes(binding, uid).toString() !== `${JSON.stringify(identity)}\n`) fail("project, release or issuer configuration changed; keep the matching bundle/state or use a separate fresh deployment");
  } else if (readdirSync(root).some((name) => name !== ".consumer.lock")) {
    fail("unbound installation volume is not empty; preserve it for explicit recovery");
  }
  if (existsSync(seal)) {
    const expected = privateBytes(seal, uid).toString();
    if (!existsSync(secretDirectory) || expected !== `${JSON.stringify(inventory(secretDirectory, uid))}\n`) fail("retained secret set is missing or changed; restore its paired backup, never regenerate beside a database");
  }
  return { binding, seal, secretDirectory };
}

export function projectSecrets(root, projections, uid) {
  const directory = path.join(root, "projections");
  mkdirSync(directory, { mode: 0o700, recursive: true });
  privateDirectory(directory, uid);
  for (const [service, files] of Object.entries(projections)) {
    if (!/^[a-z][a-z0-9-]+$/.test(service)) fail("projection service refused");
    const target = path.join(directory, service);
    mkdirSync(target, { mode: 0o700, recursive: true });
    privateDirectory(target, uid);
    const directories = service === "worker" ? ["oidc_directory"] : [];
    if (readdirSync(target).some((name) => !Object.hasOwn(files, name) && !directories.includes(name))) fail("projection contains an unowned file");
    for (const name of directories) {
      mkdirSync(path.join(target, name), { mode: 0o700, recursive: true });
      privateDirectory(path.join(target, name), uid);
    }
    for (const [name, source] of Object.entries(files)) {
      if (name === "database_roles.json" && source === "config:database-roles") {
        put(path.join(target, name), readFileSync("/bundle/deploy/compose/configs/database/roles.reference.json"), uid);
        continue;
      }
      if (![name, source].every((value) => /^[a-z][a-z0-9_]+$/.test(value))) fail("projection filename refused");
      put(path.join(target, name), privateBytes(path.join(root, "synveda-evaluation/secrets", source), uid), uid);
    }
  }
  const oidc = path.join(root, "oidc");
  mkdirSync(oidc, { mode: 0o700, recursive: true });
  privateDirectory(oidc, uid);
  put(path.join(oidc, "issuers.json"), privateBytes(path.join(root, "synveda-evaluation/issuers.json"), uid), uid);
}

export function initialize() {
  const uid = 65532, gid = 65532, root = "/state";
  if (process.getuid() !== 0) fail("initial volume ownership requires the isolated initializer UID");
  const project = process.env.SYNVEDA_CONSUMER_PROJECT;
  if (!/^synveda-local(?:-acceptance-[a-z0-9][a-z0-9-]{0,23})?$/.test(project ?? "")) fail("unexpected Compose project; use the bundle default or an isolated acceptance project");
  const entries = readdirSync(root).filter((name) => name !== ".consumer.lock");
  const stat = lstatSync(root);
  if (!stat.isDirectory() || stat.isSymbolicLink()) fail("installation volume refused");
  // Only a new empty named volume gets ownership initialized. Never repair
  // foreign or broadened permissions on retained state implicitly.
  if (!entries.length && stat.uid === 0) {
    chownSync(root, uid, gid);
    chmodSync(root, 0o700);
  }
  privateDirectory(root, uid);
  const retainedDatabase = readdirSync("/retained-postgres").length > 0;
  process.setgroups([]);
  process.setgid(gid);
  process.setuid(uid);
  const identity = { schema_version: 1, project, manifest: readJson("/bundle/environment.json"), options: readJson("/bundle/evaluation.json") };
  const state = retainedState(root, retainedDatabase, identity, uid);
  put(state.binding, `${JSON.stringify(identity)}\n`, uid);
  process.env.SYNVEDA_HOST_STATE = root;
  process.env.SYNVEDA_HOST_BUNDLE = "/bundle";
  prepare();
  put(state.seal, `${JSON.stringify(inventory(state.secretDirectory, uid))}\n`, uid);
  projectSecrets(root, readJson("/bundle/deploy/compose/consumer-projections.json"), uid);
  console.log("Consumer initialization complete; credentials are available only through deliberate retrieval.");
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { initialize(); } catch (error) {
    console.error(error.message.startsWith("consumer initialization:") || error.message.startsWith("prepare:") ? error.message : `consumer initialization failed (${error.code ?? error.name}); preserve both named volumes and inspect their permissions/configuration`);
    process.exitCode = 78;
  }
}
