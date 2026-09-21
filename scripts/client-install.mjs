#!/usr/bin/env node
// OPS-12: install only validated client bytes; leave gateway and harness state alone.
import { execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { chmodSync, cpSync, lstatSync, mkdirSync, readFileSync, readlinkSync, realpathSync, renameSync,
  rmSync, symlinkSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, join, normalize, resolve } from "node:path";
import { nodePath, sha256, validateClient } from "./client-artifact.mjs";

const quote = (value) => `'${value.replaceAll("'", "'\\''")}'`;
function stat(path) {
  try { return lstatSync(path); } catch (error) { if (error.code === "ENOENT") return undefined; throw error; }
}

function directory(path, create = false) {
  const parent = dirname(path);
  if (parent !== path) directory(parent, create);
  let info = stat(path);
  if (!info && create) { mkdirSync(path, { mode: 0o700 }); info = lstatSync(path); }
  if (info && !info.isDirectory()) throw new Error(`install directory is a link or non-directory: ${path}`);
}

function plainFile(path) {
  const info = lstatSync(path);
  if (!info.isFile() || info.nlink !== 1 || info.uid !== process.getuid()) throw new Error(`unowned, linked or non-regular install file: ${path}`);
  return readFileSync(path);
}

function atomicFile(path, bytes, mode) {
  const temporary = `${path}.${randomUUID()}.tmp`;
  try {
    writeFileSync(temporary, bytes, { flag: "wx", mode });
    chmodSync(temporary, mode);
    renameSync(temporary, path);
  } finally { rmSync(temporary, { force: true }); }
}

export function installClient(source, home, bin, expected = {}) {
  if (![home, bin].every((path) => typeof path === "string" && isAbsolute(path) && normalize(path) === path &&
    path !== "/" && !/[\r\n\0]/.test(path) && !path.endsWith("/"))) throw new Error("install paths must be normalized absolute directories");
  if (bin !== join(home, "bin") && (bin === home || bin.startsWith(`${home}/`) || home.startsWith(`${bin}/`))) {
    throw new Error("client binary directory must be outside SYNVEDA_HOME or its bin directory");
  }
  const { manifest, digest } = validateClient(source);
  if ((expected.version !== undefined && manifest.version !== expected.version) ||
      (expected.target !== undefined && manifest.target !== expected.target)) throw new Error("client archive differs from requested release or target");
  if (execFileSync(join(source, nodePath), ["--version"], { encoding: "utf8", timeout: 10_000 }).trim() !== `v${manifest.node.version}` ||
      execFileSync(join(source, "bin/synveda"), ["--version"], { encoding: "utf8", timeout: 10_000 }).trim() !== manifest.cli_version) {
    throw new Error("client executable identity mismatch");
  }
  directory(home);
  directory(bin);
  const managed = join(home, "client");
  const receipt = join(managed, "install.json");
  const current = join(managed, "current");
  const launcher = join(bin, "synveda");
  const launcherBytes = `#!/bin/sh\nSYNVEDA_HOME=${quote(home)}\nexport SYNVEDA_HOME\nexec ${quote(join(current, "bin/synveda"))} "$@"\n`;
  const ownership = { schema_version: 1, home, bin, launcher_sha256: sha256(launcherBytes) };
  const ownershipBytes = `${JSON.stringify(ownership)}\n`;
  if (stat(managed)) {
    directory(managed);
    const info = lstatSync(managed);
    if (info.uid !== process.getuid() || (info.mode & 0o077) !== 0 ||
        plainFile(receipt).toString() !== ownershipBytes) throw new Error("client install ownership mismatch");
  } else if (stat(launcher)) {
    throw new Error("refusing to replace an unowned synveda launcher");
  }
  if (stat(launcher) && sha256(plainFile(launcher)) !== ownership.launcher_sha256) throw new Error("installed launcher changed; refusing replacement");
  // Create only the selected installation roots. No credential or spool path
  // is created here; their existing CLI/runtime implementations own privacy.
  directory(home, true);
  const lock = join(home, ".client-install.lock");
  try { mkdirSync(lock, { mode: 0o700 }); }
  catch (error) { if (error.code === "EEXIST") throw new Error(`client installer is busy or interrupted; inspect ${lock}`); throw error; }
  let stage;
  let linkStage;
  try {
    // Recheck ownership under the lock: another completed installer may have
    // created this root between preflight and acquisition.
    if (stat(managed)) {
      directory(managed);
      if (plainFile(receipt).toString() !== ownershipBytes) throw new Error("client install ownership changed");
    } else {
      mkdirSync(managed, { mode: 0o700 });
      atomicFile(receipt, ownershipBytes, 0o600);
    }
    directory(bin, true);
    const releases = join(managed, "releases");
    directory(releases, true);
    let previous;
    if (stat(current)) {
      if (!lstatSync(current).isSymbolicLink()) throw new Error("client current is not an owned link");
      previous = readlinkSync(current);
      if (!/^releases\/[0-9a-f]{64}$/.test(previous) ||
          validateClient(join(managed, previous)).digest !== previous.slice(9)) throw new Error("client current ownership or content changed");
    }
    const destination = join(releases, digest);
    if (stat(destination)) {
      directory(destination);
      if (validateClient(destination).digest !== digest) throw new Error("immutable client release changed");
    } else {
      stage = join(releases, `.stage-${randomUUID()}`);
      cpSync(source, stage, { recursive: true, force: false, errorOnExist: true });
      chmodSync(stage, 0o700);
      if (validateClient(stage).digest !== digest) throw new Error("staged client release changed");
      renameSync(stage, destination);
      stage = undefined;
    }
    if (stat(launcher) && sha256(plainFile(launcher)) !== ownership.launcher_sha256) throw new Error("installed launcher changed during installation");
    if (previous === undefined ? stat(current) !== undefined : readlinkSync(current) !== previous) throw new Error("client current changed during installation");
    // The stable launcher is identical across versions. Installing it before
    // the switch preserves the prior release on any interrupted upgrade.
    atomicFile(launcher, launcherBytes, 0o755);
    linkStage = join(managed, `.current-${randomUUID()}`);
    symlinkSync(`releases/${digest}`, linkStage);
    renameSync(linkStage, current);
    linkStage = undefined;
    return { manifest, launcher, current, digest };
  } finally {
    if (stage) rmSync(stage, { recursive: true, force: true });
    if (linkStage) rmSync(linkStage, { force: true });
    rmSync(lock, { recursive: true });
  }
}

if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(import.meta.filename)) {
  try {
    const [home, bin, version, target] = process.argv.slice(2);
    if (process.argv.length !== 6) throw new Error("usage: client-install.mjs HOME BIN VERSION TARGET");
    const result = installClient(resolve(import.meta.dirname, ".."), home, bin, { version, target });
    console.log(`Installed client ${result.manifest.version} (${result.manifest.target}); private Node ${result.manifest.node.version}.`);
    console.log(`CLI: ${result.launcher}\nAdd ${quote(bin)} to PATH if needed; no shell configuration was edited.`);
    console.log(`Private Node: ${join(result.current, nodePath)}`);
    for (const client of ["codex", "copilot-cli"]) console.log(`${client} hook: ${join(result.current, "plugin", client, "dist/hook.mjs")}`);
    console.log(`Next: synveda login --gateway URL --profile NAME; then synveda setup --profile NAME.`);
    console.log("Harness registration, observation consent and vendor trust remain explicit. No deployment was downloaded or started.");
    console.log("Checksums verify integrity; this installer does not enforce publisher attestations. The Synveda CLI has no publisher OS signature/notarization.");
  } catch (error) { console.error(`install: ${error.message}`); process.exitCode = 1; }
}
