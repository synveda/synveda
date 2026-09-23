#!/usr/bin/env node
// OPS-12: native CLI/Node private-storage interoperability, no issuer/harness claim.
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, linkSync, mkdirSync, mkdtempSync, readFileSync, rmSync, truncateSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

if (process.platform !== "win32") throw new Error("Windows native execution is required");
const [binary, report, runtimeRoot] = process.argv.slice(2);
if (!binary || !report) throw new Error("usage: check-windows-state.mjs CLI REPORT [CLAUDE_DIST]");
const root = mkdtempSync(join(tmpdir(), "synveda private '文' "));
const dist = resolve(runtimeRoot ?? "adapters/claude-code/dist");
const checks = [];
const sha = (bytes) => createHash("sha256").update(bytes).digest("hex");
function powershell(path, script, target) {
  const env = { ...process.env, SYNVEDA_TEST_ACL_PATH: path, SYNVEDA_TEST_ACL_TARGET: target ?? "" };
  delete env.PSModulePath;
  // The independent .NET ACL oracle can cold-start slowly on hosted arm64.
  // This setup allowance does not change CLI or hook operation deadlines.
  return execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", `$ErrorActionPreference = 'Stop'; ${script}`],
    { encoding: "utf8", timeout: 60000, env });
}
function seal(path) {
  powershell(path, `
$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
$item = Get-Item -LiteralPath $env:SYNVEDA_TEST_ACL_PATH -Force
if ($item.PSIsContainer) { $acl = [System.Security.AccessControl.DirectorySecurity]::new(); $inherit = [System.Security.AccessControl.InheritanceFlags]'ContainerInherit,ObjectInherit' }
else { $acl = [System.Security.AccessControl.FileSecurity]::new(); $inherit = [System.Security.AccessControl.InheritanceFlags]::None }
$acl.SetOwner($sid); $acl.SetAccessRuleProtection($true, $false)
foreach ($identity in @($sid, [System.Security.Principal.SecurityIdentifier]::new('S-1-5-18'), [System.Security.Principal.SecurityIdentifier]::new('S-1-5-32-544'))) {
  $acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new($identity, 'FullControl', $inherit, 'None', 'Allow'))
}
Set-Acl -LiteralPath $item.FullName -AclObject $acl
`);
}
const execute = (args) => execFileSync(resolve(binary), args, { encoding: "utf8", timeout: 10000 });
function request(request) {
  const child = spawnSync(resolve(binary), ["private-state"], {
    input: JSON.stringify({ version: 1, request }), encoding: "utf8", timeout: 5000, maxBuffer: 24 * 1024 * 1024,
  });
  if (child.status !== 0) {
    assert.equal(child.stdout, "", "refusal emits no private bytes");
    assert.ok(!child.stderr.includes("PRIVATE_PAYLOAD_SENTINEL"), "refusal is content-free");
    throw new Error(`private request refused: ${child.stderr}`);
  }
  const response = JSON.parse(child.stdout);
  assert.equal(response.architecture, process.arch === "arm64" ? "aarch64" : "x86_64", "CLI and Node must both execute natively");
  return response.result;
}
try {
  seal(root);
  process.env.SYNVEDA_CLI = resolve(binary);
  process.env.XDG_CONFIG_HOME = join(root, "config");
  process.env.XDG_STATE_HOME = join(root, "state");
  process.env.LOCALAPPDATA = root;
  delete process.env.SYNVEDA_TOKEN;
  delete process.env.SYNVEDA_GATEWAY;
  const spool = await import(pathToFileURL(join(dist, "spool.mjs")));
  const { installationId } = await import(pathToFileURL(join(dist, "install-id.mjs")));
  const id = installationId();
  assert.equal(installationId(), id);
  assert.equal(readFileSync(join(root, "config/synveda/installation-id"), "utf8"), id);
  assert.equal(spool.claimDisclosure(root), true);
  assert.equal(spool.claimDisclosure(root), false);
  checks.push("private-installation-identity-and-single-disclosure");

  const value = spool.newSpool("interop", "claude-code", id);
  value.transcript_path = "C:/project/transcript 文.jsonl";
  value.model = "fixture-model";
  value.recorded_through = "event-2";
  value.gateway_url = "http://127.0.0.1:1";
  spool.record(value, [1, 2].map((i) => ({ event_type: "message.user", client_event_id: `event-${i}`,
    occurred_at: "2026-09-21T00:00:00Z", payload: { text: "PRIVATE_PAYLOAD_SENTINEL", order: i } })));
  assert.equal(spool.saveSpool(value), true);
  const path = spool.spoolFile("interop");
  powershell(path, `
$acl = Get-Acl -LiteralPath $env:SYNVEDA_TEST_ACL_PATH
if (-not $acl.AreAccessRulesProtected) { throw 'unprotected ACL' }
$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
if ($acl.GetOwner([System.Security.Principal.SecurityIdentifier]).Value -ne $sid) { throw 'wrong owner' }
foreach ($rule in $acl.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier])) {
  if ($rule.IdentityReference.Value -notin @($sid, 'S-1-5-18', 'S-1-5-32-544') -or $rule.IsInherited) { throw 'unexpected access' }
}`);
  const status = execute(["session", "spool", "status", "--json"]);
  assert.ok(!status.includes("PRIVATE_PAYLOAD_SENTINEL"));
  assert.ok(status.includes("interop"));
  spool.acknowledge(value, new Map([["event-1", "appended"]]));
  assert.equal(spool.saveSpool(value), true);
  execute(["session", "spool", "purge", "--acknowledged"]);
  const rewritten = spool.loadSpool("interop");
  assert.ok(rewritten, "Node reads the Rust rewrite");
  assert.equal(rewritten.entries.length, 1);
  assert.equal(rewritten.entries[0].client_event_id, "event-2");
  for (const key of ["transcript_path", "model", "recorded_through", "gateway_url"]) assert.equal(rewritten[key], value[key]);
  checks.push("node-write-rust-status-purge-node-read-format-preservation");

  const stale = spool.loadSpool("interop");
  spool.record(rewritten, [{ event_type: "message.user", client_event_id: "event-3", occurred_at: "2026-09-21T00:00:01Z", payload: { text: "newer" } }]);
  assert.equal(spool.saveSpool(rewritten), true);
  const newer = readFileSync(path);
  assert.equal(spool.saveSpool(stale), false);
  stale.entries.forEach((e) => { e.acknowledged = true; });
  assert.equal(spool.retireIfComplete(stale), false);
  assert.deepEqual(readFileSync(path), newer);
  rewritten.entries.forEach((e) => { e.acknowledged = true; });
  rewritten.close_requested = true;
  assert.equal(spool.saveSpool(rewritten), true);
  execute(["session", "spool", "purge", "--acknowledged"]);
  assert.equal(spool.loadSpool("interop").close_requested, true, "purge retains an owed close");
  const complete = spool.loadSpool("interop");
  complete.close_requested = false;
  assert.equal(spool.saveSpool(complete), true);
  assert.equal(spool.retireIfComplete(complete), false, "an active native binding is retained");
  complete.closed = true;
  assert.equal(spool.saveSpool(complete), true);
  assert.equal(spool.retireIfComplete(complete), true);
  assert.equal(existsSync(path), false);
  checks.push("stale-write-and-retirement-refusal-owed-close-retention");

  const retained = spool.newSpool("retained", "claude-code", id);
  spool.record(retained, [{ event_type: "message.user", client_event_id: "one", occurred_at: "2026-09-21T00:00:00Z", payload: { text: "PRIVATE_PAYLOAD_SENTINEL" } }]);
  assert.equal(spool.saveSpool(retained), true);
  const retainedPath = spool.spoolFile("retained");
  const before = readFileSync(retainedPath);
  const alias = join(root, "state/synveda/spool/alias.json");
  linkSync(retainedPath, alias);
  assert.equal(spool.loadSpool("retained"), undefined);
  assert.equal(spool.saveSpool(retained), false);
  assert.throws(() => request({ operation: "remove_spool", name: basename(retainedPath), expected: sha(before) }));
  assert.deepEqual(readFileSync(retainedPath), before);
  rmSync(alias);
  powershell(retainedPath, `
$acl = Get-Acl -LiteralPath $env:SYNVEDA_TEST_ACL_PATH
$acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new([System.Security.Principal.SecurityIdentifier]::new('S-1-1-0'), 'Read', 'Allow'))
Set-Acl -LiteralPath $env:SYNVEDA_TEST_ACL_PATH -AclObject $acl
`);
  assert.equal(spool.loadSpool("retained"), undefined);
  assert.equal(spool.saveSpool(retained), false);
  assert.deepEqual(readFileSync(retainedPath), before);
  seal(retainedPath);
  truncateSync(retainedPath, 16 * 1024 * 1024 + 1);
  assert.equal(spool.loadSpool("retained"), undefined);
  assert.equal(spool.saveSpool(retained), false);
  writeFileSync(retainedPath, before);
  const junction = join(root, "junction");
  powershell(junction, "New-Item -ItemType Junction -Path $env:SYNVEDA_TEST_ACL_PATH -Target $env:SYNVEDA_TEST_ACL_TARGET | Out-Null", join(root, "state"));
  process.env.XDG_STATE_HOME = junction;
  assert.throws(() => request({ operation: "list_spools" }));
  process.env.XDG_STATE_HOME = join(root, "state");
  rmSync(junction);
  checks.push("hardlink-broad-acl-oversize-and-junction-refusal");

  const key = sha(Buffer.from(root));
  const receipts = join(root, "config/synveda/consumer");
  mkdirSync(receipts);
  seal(receipts);
  const receiptPath = join(receipts, `setup-${key}.json`);
  const receipt = Buffer.from(JSON.stringify({ version: 1, root, selection: { observation: "on", workspace: "workspace", project: "project", profile: "default" } }));
  writeFileSync(receiptPath, receipt);
  seal(receiptPath);
  assert.equal(request({ operation: "read_receipt", key }).bytes, receipt.toString("base64"));
  linkSync(receiptPath, join(receipts, "receipt-alias"));
  assert.throws(() => request({ operation: "read_receipt", key }));
  for (const name of ["../credentials.json", "C:/foreign.json", "CON.json", "file.json:stream"]) {
    assert.throws(() => request({ operation: "read_spool", name }));
  }
  assert.throws(() => request({ operation: "read_credentials" }));
  checks.push("private-receipt-read-and-protocol-confinement");
  writeFileSync(report, JSON.stringify({ schema_version: 1, evidence: "native-windows-private-state",
    arch: process.arch, platform: process.platform, cli_version: execute(["--version"]).trim(),
    node: process.version, real_issuer: false, native_harness: false, checks }, null, 2) + "\n");
  console.log(`Windows ${process.arch}: ${checks.length} private-state checks passed`);
} finally { rmSync(root, { recursive: true, force: true }); }
