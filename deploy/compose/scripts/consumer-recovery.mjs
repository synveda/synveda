#!/usr/bin/env node
// OPS-12: named-volume adapter for the existing logical recovery set.
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, rmdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { consumerProject, inventory, ownEmptyVolume, privateBytes, projectSecrets, retainedState } from "./initialize-consumer.mjs";
import { copyPrivate, privateDirectory, recoverySet } from "./evaluation-recovery.mjs";
import { prepare } from "./prepare-evaluation.mjs";

export function verifyConsumerConfiguration(configuration, identity, uid = process.getuid()) {
  const binding = privateBytes(`${configuration}/consumer.json`, uid);
  if (binding.toString() !== `${JSON.stringify(identity)}\n`) throw new Error("recovery source project, release or issuer differs from this bundle");
  const seal = privateBytes(`${configuration}/consumer-secrets.json`, uid);
  if (seal.toString() !== `${JSON.stringify(inventory(`${configuration}/secrets`, uid))}\n`) throw new Error("recovery configuration differs from its original secret seal");
}

export function restoreConsumerConfiguration(configuration, root, identity, uid = process.getuid()) {
  privateDirectory(root);
  if (readdirSync(root).some((name) => name !== ".consumer.lock")) throw new Error("restore installation target is not empty; preserve retained state");
  mkdirSync(`${root}/.recovery-operation`, { mode: 0o700 });
  const state = `${root}/synveda-evaluation`;
  mkdirSync(state, { mode: 0o700 });
  copyPrivate(`${configuration}/secrets`, `${state}/secrets`);
  for (const name of ["installation.json", "issuers.json"]) {
    privateBytes(`${configuration}/${name}`, uid);
    copyFileSync(`${configuration}/${name}`, `${state}/${name}`, 1);
  }
  copyFileSync(`${configuration}/consumer-secrets.json`, `${root}/consumer-secrets.json`, 1);
  writeFileSync(`${root}/consumer.json`, `${JSON.stringify(identity)}\n`, { mode: 0o600, flag: "wx" });
  retainedState(root, true, identity, uid);
}

function main() {
  const [action, id] = process.argv.slice(2);
  if (process.argv.length !== 4 || !["begin-backup", "snapshot", "verify", "restore", "unlock"].includes(action) || !/^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/.test(id ?? "") || id.includes("--")) throw new Error("invalid recovery action or backup id");
  const project = consumerProject(process.env.SYNVEDA_CONSUMER_PROJECT);
  const source = consumerProject(process.env.SYNVEDA_RECOVERY_SOURCE ?? project);
  const manifest = JSON.parse(readFileSync("/bundle/environment.json", "utf8"));
  const options = JSON.parse(readFileSync("/bundle/evaluation.json", "utf8"));
  const identity = { schema_version: 1, project: source, manifest, options };
  const root = `/recovery/${id}`, state = "/state/synveda-evaluation";
  if (action === "verify") {
    recoverySet("verify", id, { root, state, manifest, project: source });
    verifyConsumerConfiguration(`${root}/configuration`, identity);
    console.log(`Consumer recovery set verified for ${source}/${id}.`);
    return;
  }
  const retained = readdirSync("/retained-postgres").length > 0;
  if (process.getuid() !== 0) throw new Error("recovery volume ownership requires the isolated utility UID");
  if (action === "restore" && (retained || source === project || process.env.SYNVEDA_CONFIRM_RESTORE !== `${source}:${id}:${project}`)) throw new Error("restore requires exact source:backup:target confirmation and empty target databases");
  ownEmptyVolume("/state");
  if (action === "begin-backup") ownEmptyVolume("/recovery");
  process.setgroups([]); process.setgid(65532); process.setuid(65532);
  privateDirectory("/recovery");
  if (action === "unlock") {
    rmdirSync("/state/.recovery-operation");
    return;
  }
  if (action === "begin-backup" || action === "snapshot") {
    if (source !== project || !retained) throw new Error("backup requires its existing source installation and PostgreSQL volume");
    retainedState("/state", true, identity, 65532);
  }
  if (action === "begin-backup") {
    if (existsSync(root)) throw new Error("backup id already exists; recovery sets are never overwritten");
    mkdirSync("/state/.recovery-operation", { mode: 0o700 });
    mkdirSync(root, { mode: 0o700 });
    mkdirSync(`${root}/database`, { mode: 0o700 });
    return;
  }
  if (action === "snapshot") privateDirectory("/state/.recovery-operation");
  recoverySet(action === "snapshot" ? "snapshot" : "verify", id, {
    root, state, manifest, project: source,
    extraConfiguration: { "consumer.json": "/state/consumer.json", "consumer-secrets.json": "/state/consumer-secrets.json" },
  });
  verifyConsumerConfiguration(`${root}/configuration`, identity);
  if (action === "restore") {
    restoreConsumerConfiguration(`${root}/configuration`, "/state", { ...identity, project });
    process.env.SYNVEDA_HOST_STATE = "/state";
    process.env.SYNVEDA_HOST_BUNDLE = "/bundle";
    prepare();
    projectSecrets("/state", JSON.parse(readFileSync("/bundle/deploy/compose/consumer-projections.json", "utf8")), 65532);
  }
  console.log(`Consumer recovery ${action} verified for ${source}/${id}; private material was retained.`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { main(); } catch (error) {
    // Filesystem/JSON diagnostics can contain private bytes or paths.
    console.error(error.code || error instanceof SyntaxError ? "consumer recovery: private state unavailable or unsafe; preserve all recovery volumes" : `consumer recovery: ${error.message}`);
    process.exitCode = 78;
  }
}
