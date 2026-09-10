#!/usr/bin/env node
import { realpathSync } from "node:fs";
import {
  lstat,
  readFile,
  readdir,
  readlink,
  rmdir,
  unlink,
} from "node:fs/promises";
import process from "node:process";
import { fileURLToPath } from "node:url";

const MARKER = ".synveda-private-directory";
const READY = "cpr45-keycloak-realm-v3.ready";

async function privateDirectory(path, uid) {
  const metadata = await lstat(path);
  if (!metadata.isDirectory() || metadata.isSymbolicLink() || (metadata.mode & 0o777) !== 0o700 || metadata.uid !== uid) {
    throw new Error("private runtime state metadata was refused");
  }
  const markerPath = `${path}/${MARKER}`;
  const marker = await lstat(markerPath);
  if (!marker.isFile() || marker.isSymbolicLink() || (marker.mode & 0o777) !== 0o600 || marker.uid !== uid) {
    throw new Error("private runtime state ownership was refused");
  }
  return markerPath;
}

async function regularFile(path, mode, uid) {
  const metadata = await lstat(path);
  if (!metadata.isFile() || metadata.isSymbolicLink() || (metadata.mode & 0o777) !== mode || metadata.uid !== uid) {
    throw new Error("private runtime state leaf was refused");
  }
}

function validGeneration(name) {
  return /^\.generation-[A-Za-z0-9]{12}$/.test(name);
}

export async function resetStatePlan({ project, authorityDir, gateDir, uid = process.getuid() }) {
  if (!/^synveda-(?:development|reference)(?:-acceptance-[a-z0-9][a-z0-9-]{0,23})?$/.test(project)) {
    throw new Error("project identity was refused");
  }
  if (!authorityDir.endsWith(`/${project}/database-authority`) || !gateDir.endsWith(`/${project}/keycloak-public-gate`)) {
    throw new Error("project runtime state path was refused");
  }
  const authorityMarker = await privateDirectory(authorityDir, uid);
  const gateMarker = await privateDirectory(gateDir, uid);
  for (const markerPath of [authorityMarker, gateMarker]) {
    if ((await readFile(markerPath, "utf8")) !== `project:${project}\n`) {
      throw new Error("private runtime state ownership was refused");
    }
  }

  const authorityEntries = await readdir(authorityDir, { withFileTypes: true });
  const authorityNames = authorityEntries.map(({ name }) => name).sort();
  if (authorityNames.some((name) => ![MARKER, "keycloak-cluster.json"].includes(name))) {
    throw new Error("database authority state contains an unknown leaf");
  }
  const witness = authorityNames.includes("keycloak-cluster.json")
    ? `${authorityDir}/keycloak-cluster.json`
    : undefined;
  if (witness) await regularFile(witness, 0o600, uid);

  const gateEntries = await readdir(gateDir, { withFileTypes: true });
  const generations = [];
  let current;
  for (const entry of gateEntries) {
    if (entry.name === MARKER) continue;
    if (entry.name === "current") {
      if (!entry.isSymbolicLink()) throw new Error("Keycloak public selector was refused");
      const metadata = await lstat(`${gateDir}/current`);
      if (metadata.uid !== uid) throw new Error("Keycloak public selector was refused");
      current = await readlink(`${gateDir}/current`);
      continue;
    }
    if (!entry.isDirectory() || !validGeneration(entry.name)) {
      throw new Error("Keycloak public gate contains an unknown leaf");
    }
    const directory = `${gateDir}/${entry.name}`;
    const metadata = await lstat(directory);
    if (metadata.isSymbolicLink() || (metadata.mode & 0o777) !== 0o700 || metadata.uid !== uid) {
      throw new Error("Keycloak public generation was refused");
    }
    const leaves = await readdir(directory, { withFileTypes: true });
    if (leaves.some((leaf) => leaf.name !== READY || !leaf.isFile())) {
      throw new Error("Keycloak public generation contains an unknown leaf");
    }
    const ready = leaves.length === 1 ? `${directory}/${READY}` : undefined;
    if (ready) await regularFile(ready, 0o400, uid);
    generations.push({ directory, ready, name: entry.name });
  }
  if (current !== undefined && (!validGeneration(current) || !generations.some(({ name }) => name === current))) {
    throw new Error("Keycloak public selector was refused");
  }
  return {
    witness,
    current: current === undefined ? undefined : `${gateDir}/current`,
    generations: generations.sort((left, right) => left.name.localeCompare(right.name)),
  };
}

export async function applyResetState(plan) {
  if (plan.current) await unlink(plan.current);
  for (const generation of plan.generations) {
    if (generation.ready) await unlink(generation.ready);
    await rmdir(generation.directory);
  }
  if (plan.witness) await unlink(plan.witness);
}

function parseArguments(argv) {
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!key?.startsWith("--") || value === undefined || values.has(key)) return undefined;
    values.set(key, value);
  }
  const allowed = new Set(["--mode", "--project", "--authority-dir", "--gate-dir"]);
  if ([...values.keys()].some((key) => !allowed.has(key))) return undefined;
  if (!["check", "apply"].includes(values.get("--mode"))) return undefined;
  if (![values.get("--authority-dir"), values.get("--gate-dir")].every((path) => path?.startsWith("/"))) {
    return undefined;
  }
  return {
    mode: values.get("--mode"),
    project: values.get("--project"),
    authorityDir: values.get("--authority-dir"),
    gateDir: values.get("--gate-dir"),
  };
}

export async function main(argv = process.argv.slice(2)) {
  const selection = parseArguments(argv);
  if (selection === undefined) {
    console.error("compose-reset: configuration was refused");
    process.exitCode = 64;
    return;
  }
  try {
    const plan = await resetStatePlan(selection);
    if (selection.mode === "apply") await applyResetState(plan);
  } catch (error) {
    const message = error instanceof Error ? error.message : "runtime state was refused";
    console.error(`compose-reset: ${message}`);
    process.exitCode = 78;
    return;
  }
  console.log(
    selection.mode === "check"
      ? "exact project runtime reset state validated"
      : "exact project runtime state reset; secrets, issuer and KMS key retained",
  );
}

if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
