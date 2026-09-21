#!/usr/bin/env node
// OPS-12: exact native Windows archive installation, separate from vendor loading.
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { cliPath, nodePath, sha256, targetName, validateClient } from "./client-artifact.mjs";

if (process.platform !== "win32") throw new Error("native Windows execution required");
const [version, argument, report] = process.argv.slice(2);
if (!report) throw new Error("usage: check-windows-client-package.mjs VERSION ARCHIVE REPORT");
const archive = resolve(argument);
const root = resolve(import.meta.dirname, "..");
const scratch = mkdtempSync(join(tmpdir(), "synveda Windows archive '文' "));
const ps = join(process.env.SystemRoot, "System32/WindowsPowerShell/v1.0/powershell.exe");
const run = (command, args, options = {}) => execFileSync(command, args, { encoding: "utf8", timeout: 120000, maxBuffer: 8 * 1024 * 1024, ...options });
const checks = [];
try {
  const env = { ...process.env, SYNVEDA_TEST_ACL_PATH: scratch };
  delete env.PSModulePath;
  run(ps, ["-NoProfile", "-NonInteractive", "-Command", `
$ErrorActionPreference = 'Stop'
$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
$acl = [System.Security.AccessControl.DirectorySecurity]::new()
$acl.SetOwner($sid); $acl.SetAccessRuleProtection($true, $false)
foreach ($who in @($sid, [System.Security.Principal.SecurityIdentifier]::new('S-1-5-18'), [System.Security.Principal.SecurityIdentifier]::new('S-1-5-32-544'))) {
  $acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new($who, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow'))
}
Set-Acl -LiteralPath $env:SYNVEDA_TEST_ACL_PATH -AclObject $acl
`], { env });
  run("tar", ["-xf", archive, "-C", scratch]);
  const { manifest, digest } = validateClient(join(scratch, "client"));
  assert.equal(manifest.version, version);
  assert.equal(manifest.target, targetName());
  assert.ok(!Object.keys(manifest.files).some((p) => /synveda-(gateway|worker)|^console\/|^reference\//.test(p)));
  checks.push("native-identity-and-client-only-inventory");
  const assets = join(scratch, "assets");
  mkdirSync(assets);
  const asset = `synveda-client-${version}-${targetName()}.zip`;
  copyFileSync(archive, join(assets, asset));
  const checksum = `${sha256(readFileSync(archive))}  ${asset}\n`;
  writeFileSync(join(assets, "SHA256SUMS"), checksum);
  const home = join(scratch, "installed '文' client");
  const installEnv = { ...env, LOCALAPPDATA: scratch, NODE_OPTIONS: "", NODE_PATH: "",
    XDG_CONFIG_HOME: join(scratch, "config"), XDG_STATE_HOME: join(scratch, "state"),
    PATH: `${process.env.SystemRoot}\\System32;${process.env.SystemRoot}` };
  const args = ["-NoProfile", "-NonInteractive", "-File", join(root, "scripts/install.ps1"),
    "-Version", version, "-BaseUrl", pathToFileURL(assets).href, "-InstallRoot", home];
  const start = performance.now();
  run(ps, args, { env: installEnv });
  const coldMs = performance.now() - start;
  const destination = join(home, "client/releases", digest);
  const nativeCli = join(destination, cliPath);
  const nativeNode = join(destination, nodePath);
  assert.equal(validateClient(destination).digest, digest);
  assert.equal(run(ps, ["-NoProfile", "-NonInteractive", "-File", join(home, "bin/synveda.ps1"), "--version"], { env: installEnv }).trim(), manifest.cli_version);
  assert.equal(run(nativeNode, ["--version"], { env: installEnv }).trim(), `v${manifest.node.version}`);
  for (const client of ["synveda", "codex", "copilot-cli"]) {
    assert.equal(run(nativeNode, [join(destination, "plugin", client, "dist/hook.mjs")],
      { input: "{}", env: { ...installEnv, SYNVEDA_CLI: nativeCli, SYNVEDA_DISABLED: "1" } }), "");
  }
  for (const path of ["config", "state", ".claude", ".codex", ".copilot"]) assert.equal(existsSync(join(scratch, path)), false);
  checks.push("restricted-path-install-cli-and-three-hook-launches", "private-install-without-harness-or-credential-mutation");
  mkdirSync(join(home, "state"));
  writeFileSync(join(home, "state/retained"), "retain deployment state");
  const warm = performance.now();
  run(ps, args, { env: installEnv });
  const warmMs = performance.now() - warm;
  assert.equal(readFileSync(join(home, "state/retained"), "utf8"), "retain deployment state");
  checks.push("repeat-install-preserves-deployment-state");
  run(nativeNode, [join(root, "scripts/check-windows-state.mjs"), nativeCli, join(scratch, "storage.json"), join(destination, "plugin/synveda/dist")], { env: { ...env, NODE_OPTIONS: "", NODE_PATH: "" } });
  const storage = JSON.parse(readFileSync(join(scratch, "storage.json")));
  assert.equal(storage.arch, process.arch);
  assert.equal(storage.checks.length, 6);
  checks.push("native-windows-private-storage-interoperability");

  function refuse(commandArgs = args, diagnostic) {
    const child = spawnSync(ps, commandArgs, { encoding: "utf8", timeout: 120000, env: installEnv });
    assert.ifError(child.error);
    assert.notEqual(child.status, 0, child.stdout);
    if (diagnostic) assert.match(child.stderr + child.stdout, diagnostic);
  }
  const selection = readFileSync(join(home, "client/current.json"));
  writeFileSync(join(assets, "SHA256SUMS"), checksum + checksum);
  refuse();
  assert.deepEqual(readFileSync(join(home, "client/current.json")), selection);
  writeFileSync(join(assets, "SHA256SUMS"), checksum);
  const launcher = join(home, "bin/synveda.ps1");
  const originalLauncher = readFileSync(launcher);
  writeFileSync(launcher, "# user-owned change\n");
  refuse();
  assert.equal(readFileSync(launcher, "utf8"), "# user-owned change\n");
  assert.deepEqual(readFileSync(join(home, "client/current.json")), selection);
  writeFileSync(launcher, originalLauncher);
  mkdirSync(join(home, ".client-install.lock"));
  refuse();
  assert.equal(existsSync(join(home, ".client-install.lock")), true);
  rmSync(join(home, ".client-install.lock"), { recursive: true });
  checks.push("duplicate-checksum-launcher-drift-and-interrupted-lock-refusal");

  // Build malformed, checksum-matching ZIPs independently of the producer.
  // Bootstrap must reject their metadata before loading any archived code.
  for (const entries of [
    [{ name: "client/../outside" }], [{ name: "foreign/file" }], [{ name: "client/CON.txt" }],
    [{ name: "client/file:stream" }], [{ name: "client/file." }], [{ name: "client\\escape" }],
    [{ name: "client/file" }, { name: "client/FILE" }], [{ name: "client/link", mode: -1610612736 }],
  ]) {
    rmSync(join(assets, asset));
    run(ps, ["-NoProfile", "-NonInteractive", "-Command", `
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
$entries = $env:SYNVEDA_TEST_ZIP_ENTRIES | ConvertFrom-Json
$zip = [System.IO.Compression.ZipFile]::Open($env:SYNVEDA_TEST_ZIP_PATH, 'Create')
try {
  foreach ($item in $entries) {
    $entry = $zip.CreateEntry($item.name)
    if ($null -ne $item.mode) { $entry.ExternalAttributes = [int]$item.mode }
  }
} finally { $zip.Dispose() }
`], { env: { ...env, SYNVEDA_TEST_ZIP_PATH: join(assets, asset), SYNVEDA_TEST_ZIP_ENTRIES: JSON.stringify(entries) } });
    writeFileSync(join(assets, "SHA256SUMS"), `${sha256(readFileSync(join(assets, asset)))}  ${asset}\n`);
    refuse(args, /Unsafe archive component|Archive contains a link, duplicate or foreign path/);
    assert.deepEqual(readFileSync(join(home, "client/current.json")), selection);
  }
  copyFileSync(archive, join(assets, asset));
  writeFileSync(join(assets, "SHA256SUMS"), checksum);
  refuse([...args, "-BinDirectory", home.toUpperCase()], /binary directory must be outside/);
  assert.deepEqual(readFileSync(join(home, "client/current.json")), selection);
  checks.push("unsafe-zip-and-overlapping-install-root-refusal");

  writeFileSync(report, JSON.stringify({ schema_version: 1, evidence: "native-client-archive", target: manifest.target,
    version, cli_version: manifest.cli_version, source_sha: manifest.source_sha, source_tree_dirty: manifest.source_tree_dirty,
    node: manifest.node, archive_sha256: sha256(readFileSync(archive)), archive_bytes: statSync(archive).size,
    local_install_ms: Math.round(coldMs), repeat_install_ms: Math.round(warmMs), network_download_measured: false,
    real_issuer_login: false, native_harness_run: false, checks }, null, 2) + "\n");
  console.log(`Windows ${process.arch}: ${checks.length} exact-archive checks passed`);
} finally { rmSync(scratch, { recursive: true, force: true }); }
