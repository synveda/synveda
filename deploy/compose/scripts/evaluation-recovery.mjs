// CPR-45: reuse the logical recovery contract inside the release utility.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { prepare } from "./prepare-evaluation.mjs";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";

const run = (...args) => {
  const result = spawnSync(process.execPath, [fileURLToPath(new URL("./recovery-set.mjs", import.meta.url)), ...args], { stdio: "inherit", timeout: 60_000 });
  if (result.status !== 0) throw new Error("recovery set validation failed; preserve the set and inspect the non-secret error");
};
export function privateDirectory(dir) {
  const stat = lstatSync(dir);
  if (!stat.isDirectory() || stat.isSymbolicLink() || (stat.mode & 0o7777) !== 0o700 || stat.uid !== process.getuid() || stat.gid !== process.getgid()) throw new Error("recovery directory permissions refused");
}
function privateConfigurationFile(file) {
  const stat = lstatSync(file);
  if (!stat.isFile() || stat.isSymbolicLink() || (stat.mode & 0o7777) !== 0o600 || stat.uid !== process.getuid() || stat.gid !== process.getgid() || stat.nlink !== 1 || stat.size > 1024 * 1024) throw new Error("recovery file permissions or bounds refused");
  return stat;
}
export function configurationFiles(dir, prefix = "", budget = { entries: 0, bytes: 0 }) {
  privateDirectory(dir);
  if (prefix.length > 256 || readdirSync(dir).length > 128) throw new Error("recovery configuration bounds refused");
  return readdirSync(dir).sort().flatMap((name) => {
    if (++budget.entries > 256) throw new Error("recovery configuration entry bound refused");
    if (!/^[A-Za-z0-9_.-]+$/.test(name)) throw new Error("recovery filename refused");
    const path = `${dir}/${name}`, stat = lstatSync(path);
    if (stat.isDirectory()) return configurationFiles(path, `${prefix}${name}/`, budget);
    privateConfigurationFile(path);
    budget.bytes += stat.size;
    if (budget.bytes > 16 * 1024 * 1024) throw new Error("recovery configuration byte bound refused");
    return [{ name: `${prefix}${name}`, sha256: createHash("sha256").update(readFileSync(path)).digest("hex") }];
  });
}
export function copyPrivate(source, destination) {
  const inventory = configurationFiles(source);
  // Empty credential directories are part of the existing preparation
  // contract even though the configuration inventory hashes files only.
  function directories(from, to) {
    mkdirSync(to, { recursive: true, mode: 0o700 });
    privateDirectory(to);
    for (const entry of readdirSync(from, { withFileTypes: true })) {
      if (entry.isDirectory()) directories(`${from}/${entry.name}`, `${to}/${entry.name}`);
    }
  }
  directories(source, destination);
  for (const { name } of inventory) {
    const target = `${destination}/${name}`;
    mkdirSync(target.slice(0, target.lastIndexOf("/")), { recursive: true, mode: 0o700 });
    privateDirectory(target.slice(0, target.lastIndexOf("/")));
    if (existsSync(target)) {
      privateConfigurationFile(target);
      if (!readFileSync(target).equals(readFileSync(`${source}/${name}`))) throw new Error("existing state differs from backup; refuse credential replacement");
    } else copyFileSync(`${source}/${name}`, target, 1);
  }
  return inventory;
}
export function recoverySet(action, id, { root, state, manifest, project = "synveda-evaluation", extraConfiguration = {} }) {
  if (!["snapshot", "verify"].includes(action) || !/^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/.test(id ?? "") || id.includes("--")) throw new Error("invalid recovery action or backup id");
  privateDirectory(root);
  if (action === "snapshot") {
    mkdirSync(`${root}/keys`, { mode: 0o700 });
    run("snapshot-secrets", "--key-file", `${state}/secrets/synveda_kms_key`, "--key-ref-file", `${state}/secrets/synveda_kms_key_ref`, "--keycloak-convergence-password-file", `${state}/secrets/keycloak_convergence_admin_password`, "--database-manifest", `${root}/database/manifest.json`, "--output-dir", `${root}/keys`, "--project", project, "--backup-id", id);
  }
  run("verify", "--database-dir", `${root}/database`, "--secrets-dir", `${root}/keys`, "--project", project, "--backup-id", id, "--tenant-id", "019b53c0-7c00-7000-8000-000000000045", "--postgres-image", manifest.images.postgres);
  if (action === "snapshot") {
    copyPrivate(`${state}/secrets`, `${root}/configuration/secrets`);
    for (const name of ["installation.json", "issuers.json"]) {
      privateConfigurationFile(`${state}/${name}`);
      copyFileSync(`${state}/${name}`, `${root}/configuration/${name}`, 1);
    }
    for (const [name, source] of Object.entries(extraConfiguration)) {
      if (!/^[a-z-]+\.json$/.test(name)) throw new Error("recovery configuration name refused");
      privateConfigurationFile(source);
      copyFileSync(source, `${root}/configuration/${name}`, 1);
    }
    writeFileSync(`${root}/configuration.json`, JSON.stringify(configurationFiles(`${root}/configuration`)), { mode: 0o600, flag: "wx" });
  } else {
    privateConfigurationFile(`${root}/configuration.json`);
    if (JSON.stringify(configurationFiles(`${root}/configuration`)) !== readFileSync(`${root}/configuration.json`, "utf8")) throw new Error("backup configuration checksum mismatch");
  }
  // Configuration must carry the very keys linked to this database pair, not
  // merely an independently self-consistent configuration inventory.
  for (const name of ["synveda_kms_key", "synveda_kms_key_ref", "keycloak_convergence_admin_password"]) {
    if (!readFileSync(`${root}/configuration/secrets/${name}`).equals(readFileSync(`${root}/keys/${name}`))) throw new Error("backup configuration differs from paired recovery keys");
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [action, id] = process.argv.slice(2);
  if (!["snapshot", "verify", "restore"].includes(action)) throw new Error("invalid recovery action");
  const root = `/state/backups/${id}`, state = "/state/synveda-evaluation";
  const manifest = JSON.parse(readFileSync("/bundle/environment.json", "utf8"));
  recoverySet(action === "restore" ? "verify" : action, id, { root, state, manifest });
  if (action === "restore") { copyPrivate(`${root}/configuration`, state); prepare(); }
  console.log(`Recovery ${action} validated. Keep databases and private configuration together; this is a local backup, not off-host recovery.`);
}
