// CPR-45: reuse the logical recovery contract inside the release utility.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { prepare } from "./prepare-evaluation.mjs";

const [action, id] = process.argv.slice(2);
if (!["snapshot", "verify", "restore"].includes(action) || !/^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/.test(id ?? "")) throw new Error("invalid recovery action or backup id");
const root = `/state/backups/${id}`;
const state = "/state/synveda-evaluation";
const manifest = JSON.parse(readFileSync("/bundle/environment.json", "utf8"));
const run = (...args) => {
  const result = spawnSync(process.execPath, ["/bundle/deploy/compose/scripts/recovery-set.mjs", ...args], { stdio: "inherit", timeout: 60_000 });
  if (result.status !== 0) throw new Error("recovery set validation failed; preserve the set and inspect the non-secret error");
};
function privateDirectory(dir) {
  const stat = lstatSync(dir);
  if (!stat.isDirectory() || stat.isSymbolicLink() || (stat.mode & 0o077) || stat.uid !== process.getuid()) throw new Error("recovery directory permissions refused");
}
function files(dir, prefix = "") {
  privateDirectory(dir);
  return readdirSync(dir).sort().flatMap((name) => {
    if (!/^[A-Za-z0-9_.-]+$/.test(name)) throw new Error("recovery filename refused");
    const path = `${dir}/${name}`, stat = lstatSync(path);
    if (stat.isDirectory()) return files(path, `${prefix}${name}/`);
    if (!stat.isFile() || stat.isSymbolicLink() || (stat.mode & 0o077) || stat.uid !== process.getuid()) throw new Error("recovery file permissions refused");
    return [{ name: `${prefix}${name}`, sha256: createHash("sha256").update(readFileSync(path)).digest("hex") }];
  });
}
function copyPrivate(source, destination) {
  const inventory = files(source);
  for (const { name } of inventory) {
    const target = `${destination}/${name}`;
    mkdirSync(target.slice(0, target.lastIndexOf("/")), { recursive: true, mode: 0o700 });
    privateDirectory(target.slice(0, target.lastIndexOf("/")));
    if (existsSync(target)) {
      const stat = lstatSync(target);
      if (!stat.isFile() || stat.isSymbolicLink() || (stat.mode & 0o077) || stat.uid !== process.getuid()) throw new Error("existing recovery file permissions refused");
      if (!readFileSync(target).equals(readFileSync(`${source}/${name}`))) throw new Error("existing state differs from backup; refuse credential replacement");
    } else copyFileSync(`${source}/${name}`, target, 1);
  }
  return inventory;
}
privateDirectory(root);
if (action === "snapshot") {
  mkdirSync(`${root}/keys`, { mode: 0o700 });
  run("snapshot-secrets", "--key-file", `${state}/secrets/synveda_kms_key`, "--key-ref-file", `${state}/secrets/synveda_kms_key_ref`, "--keycloak-convergence-password-file", `${state}/secrets/keycloak_convergence_admin_password`, "--database-manifest", `${root}/database/manifest.json`, "--output-dir", `${root}/keys`, "--project", "synveda-evaluation", "--backup-id", id);
}
run("verify", "--database-dir", `${root}/database`, "--secrets-dir", `${root}/keys`, "--project", "synveda-evaluation", "--backup-id", id, "--tenant-id", "019b53c0-7c00-7000-8000-000000000045", "--postgres-image", manifest.images.postgres);
if (action === "snapshot") {
  copyPrivate(`${state}/secrets`, `${root}/configuration/secrets`);
  for (const name of ["installation.json", "issuers.json"]) copyFileSync(`${state}/${name}`, `${root}/configuration/${name}`, 1);
  writeFileSync(`${root}/configuration.json`, JSON.stringify(files(`${root}/configuration`)), { mode: 0o600, flag: "wx" });
} else {
  if (JSON.stringify(files(`${root}/configuration`)) !== readFileSync(`${root}/configuration.json`, "utf8")) throw new Error("backup configuration checksum mismatch");
  if (action === "restore") {
    copyPrivate(`${root}/configuration`, state);
    prepare();
  }
}
console.log(`Recovery ${action} validated. Keep databases and private configuration together; this is a local backup, not off-host recovery.`);
