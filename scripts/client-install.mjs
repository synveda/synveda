#!/usr/bin/env node
// OPS-12: install only validated client bytes; leave gateway and harness state alone.
import { execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { chmodSync, cpSync, lstatSync, mkdirSync, readFileSync, readlinkSync, realpathSync, renameSync,
  rmSync, symlinkSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, join, normalize, parse, resolve } from "node:path";
import { cliPath, nodePath, sha256, validateClient } from "./client-artifact.mjs";

const windows = process.platform === "win32";
let windowsBinary;
function native(request) {
  const output = execFileSync(windowsBinary, ["installer-state"], { input: JSON.stringify(request),
    encoding: "utf8", timeout: 15000, maxBuffer: 4 * 1024 * 1024, windowsHide: true });
  const response = JSON.parse(output);
  if (response.version !== 1) throw new Error("unsupported native installer protocol");
  return response.result;
}

const quote = (value) => `'${value.replaceAll("'", "'\\''")}'`;
function stat(path) {
  try { return lstatSync(path); } catch (error) { if (error.code === "ENOENT") return undefined; throw error; }
}

function directory(path, create = false) {
  if (windows) { native({ operation: "directory", path, create }); return; }
  const parent = dirname(path);
  if (parent !== path) directory(parent, create);
  let info = stat(path);
  if (!info && create) { mkdirSync(path, { mode: 0o700 }); info = lstatSync(path); }
  if (info && !info.isDirectory()) throw new Error(`install directory is a link or non-directory: ${path}`);
}

function plainFile(path) {
  if (windows) {
    const { bytes } = native({ operation: "read", path });
    if (typeof bytes !== "string") throw new Error("owned installer file is absent");
    return Buffer.from(bytes, "base64");
  }
  const info = lstatSync(path);
  if (!info.isFile() || info.nlink !== 1 || info.uid !== process.getuid()) throw new Error(`unowned, linked or non-regular install file: ${path}`);
  return readFileSync(path);
}

function atomicFile(path, bytes, mode) {
  if (windows) {
    const previous = native({ operation: "read", path }).bytes;
    native({ operation: "write", path, expected: previous === null ? null : sha256(Buffer.from(previous, "base64")),
      bytes: Buffer.from(bytes).toString("base64") });
    return;
  }
  const temporary = `${path}.${randomUUID()}.tmp`;
  try {
    writeFileSync(temporary, bytes, { flag: "wx", mode });
    chmodSync(temporary, mode);
    renameSync(temporary, path);
  } finally { rmSync(temporary, { force: true }); }
}

export function installClient(source, home, bin, expected = {}) {
  if (windows) {
    windowsBinary = join(source, cliPath);
    native({ operation: "tree", path: source });
  }
  if (![home, bin].every((path) => typeof path === "string" && isAbsolute(path) && normalize(path) === path &&
    path !== parse(path).root && !/[\r\n\0]/.test(path) && !(windows ? /[\\/]$/ : /\/$/).test(path))) throw new Error("install paths must be normalized absolute directories");
  const homeKey = windows ? home.toLowerCase() : home;
  const binKey = windows ? bin.toLowerCase() : bin;
  if (binKey !== join(homeKey, "bin") && (binKey === homeKey || binKey.startsWith(`${homeKey}${windows ? "\\" : "/"}`) || homeKey.startsWith(`${binKey}${windows ? "\\" : "/"}`))) {
    throw new Error("client binary directory must be outside SYNVEDA_HOME or its bin directory");
  }
  const { manifest, digest } = validateClient(source);
  if ((expected.version !== undefined && manifest.version !== expected.version) ||
      (expected.target !== undefined && manifest.target !== expected.target)) throw new Error("client archive differs from requested release or target");
  if (execFileSync(join(source, nodePath), ["--version"], { encoding: "utf8", timeout: 10_000 }).trim() !== `v${manifest.node.version}` ||
      execFileSync(join(source, cliPath), ["--version"], { encoding: "utf8", timeout: 10_000 }).trim() !== manifest.cli_version) {
    throw new Error("client executable identity mismatch");
  }
  directory(home);
  directory(bin);
  const managed = join(home, "client");
  const receipt = join(managed, "install.json");
  const current = join(managed, windows ? "current.json" : "current");
  const launcher = join(bin, windows ? "synveda.ps1" : "synveda");
  const launcherBytes = windows ? windowsLauncher(home)
    : `#!/bin/sh\nSYNVEDA_HOME=${quote(home)}\nexport SYNVEDA_HOME\nexec ${quote(join(current, "bin/synveda"))} "$@"\n`;
  const ownership = { schema_version: 1, home, bin, launcher_sha256: sha256(launcherBytes) };
  const ownershipBytes = `${JSON.stringify(ownership)}\n`;
  if (stat(managed)) {
    directory(managed);
    const info = lstatSync(managed);
    if ((!windows && (info.uid !== process.getuid() || (info.mode & 0o077) !== 0)) ||
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
      if (windows) previous = selection(current);
      else {
        if (!lstatSync(current).isSymbolicLink()) throw new Error("client current is not an owned link");
        previous = readlinkSync(current);
      }
      if (!/^releases\/[0-9a-f]{64}$/.test(previous) ||
          validateClient(join(managed, previous)).digest !== previous.slice(9)) throw new Error("client current ownership or content changed");
      if (windows) native({ operation: "tree", path: join(managed, previous) });
    }
    const destination = join(releases, digest);
    if (stat(destination)) {
      directory(destination);
      if (validateClient(destination).digest !== digest) throw new Error("immutable client release changed");
      if (windows) native({ operation: "tree", path: destination });
    } else {
      stage = join(releases, `.stage-${randomUUID()}`);
      cpSync(source, stage, { recursive: true, force: false, errorOnExist: true });
      if (windows) native({ operation: "tree", path: stage });
      else chmodSync(stage, 0o700);
      if (validateClient(stage).digest !== digest) throw new Error("staged client release changed");
      renameSync(stage, destination);
      stage = undefined;
    }
    if (stat(launcher) && sha256(plainFile(launcher)) !== ownership.launcher_sha256) throw new Error("installed launcher changed during installation");
    if (previous === undefined ? stat(current) !== undefined : (windows ? selection(current) : readlinkSync(current)) !== previous) throw new Error("client current changed during installation");
    // The stable launcher is identical across versions. Installing it before
    // the switch preserves the prior release on any interrupted upgrade.
    atomicFile(launcher, launcherBytes, 0o755);
    if (windows) {
      atomicFile(current, JSON.stringify({ schema_version: 1, release: `releases/${digest}` }) + "\n", 0o600);
      return { manifest, launcher, current: destination, digest };
    }
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

function selection(path) {
  const selected = JSON.parse(plainFile(path));
  if (selected.schema_version !== 1 || !/^releases\/[0-9a-f]{64}$/.test(selected.release)) throw new Error("invalid client selection");
  return selected.release;
}

function windowsLauncher(home) {
  const literal = `'${home.replaceAll("'", "''")}'`;
  return `\ufeff$ErrorActionPreference = 'Stop'\n$root = ${literal}\n` +
    `$selected = Get-Content -Raw -LiteralPath (Join-Path $root 'client/current.json') | ConvertFrom-Json\n` +
    `if ($selected.schema_version -ne 1 -or $selected.release -cnotmatch '^releases/[0-9a-f]{64}$') { throw 'Invalid Synveda selection' }\n` +
    `$release = Join-Path (Join-Path $root 'client') $selected.release\n` +
    `$manifestPath = Join-Path $release 'client.json'\n` +
    `if ((Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $selected.release.Substring(9)) { throw 'Changed Synveda manifest' }\n` +
    `$manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json\n` +
    `$exe = Join-Path $release 'bin/synveda.exe'\n` +
    `if ((Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $manifest.files.'bin/synveda.exe'.sha256) { throw 'Changed Synveda executable' }\n` +
    `$env:SYNVEDA_CLI = $exe\n$env:SYNVEDA_HOME = $root\n& $exe @args\nexit $LASTEXITCODE\n`;
}

if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(import.meta.filename)) {
  try {
    const [home, bin, version, target] = process.argv.slice(2);
    if (process.argv.length !== 6) throw new Error("usage: client-install.mjs HOME BIN VERSION TARGET");
    const result = installClient(resolve(import.meta.dirname, ".."), home, bin, { version, target });
    console.log(`Installed client ${result.manifest.version} (${result.manifest.target}); private Node ${result.manifest.node.version}.`);
    console.log(`CLI: ${result.launcher}\nPATH directory: ${bin}\nAdd this directory to PATH if needed; no shell configuration was edited.`);
    console.log(`Private Node: ${join(result.current, nodePath)}`);
    for (const client of ["codex", "copilot-cli"]) console.log(`${client} hook: ${join(result.current, "plugin", client, "dist/hook.mjs")}`);
    if (windows) console.log(`Native CLI for hooks: ${join(result.current, cliPath)}\nSet SYNVEDA_CLI to this absolute executable. Login with --gateway URL --profile NAME. Windows setup/vendor registration remains manual.`);
    else console.log(`Next: synveda login --gateway URL --profile NAME; then synveda setup --profile NAME.`);
    console.log("Harness registration, observation consent and vendor trust remain explicit. No deployment was downloaded or started.");
    console.log("Checksums verify integrity; this installer does not enforce publisher attestations. The Synveda CLI has no publisher OS signature/notarization.");
  } catch (error) { console.error(`install: ${error.message}`); process.exitCode = 1; }
}
