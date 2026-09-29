#!/usr/bin/env node
// OPS-5: the opt-in chart binds one external ObjectStore to WAL and base backup.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";

const chart = "deploy/helm/synveda";
function render(mode, extra = [], pluginApi = true, database = "cnpg") {
  const args = ["template", "synveda", chart, "-f", `${chart}/ci/${database}-values.yaml`,
    "--api-versions", "postgresql.cnpg.io/v1"];
  if (pluginApi) args.push("--api-versions", "barmancloud.cnpg.io/v1");
  if (mode === "enabled") args.push("--set", "postgres.backup.enabled=true",
    "--set", "postgres.backup.objectStoreName=synveda-backups",
    "--set-string", "postgres.backup.schedule=0 0 2 * * *");
  return spawnSync("helm", [...args, ...extra], { encoding: "utf8" });
}
const defaultRender = render("disabled");
assert.equal(defaultRender.status, 0, defaultRender.stderr);
assert.doesNotMatch(defaultRender.stdout, /^kind: ScheduledBackup$/m);
assert.doesNotMatch(defaultRender.stdout, /barman-cloud\.cloudnative-pg\.io/);

const enabled = render("enabled");
assert.equal(enabled.status, 0, enabled.stderr);
const documents = enabled.stdout.split(/^---\s*$/m);
const cluster = documents.find((value) => /^kind: Cluster$/m.test(value));
const schedule = documents.find((value) => /^kind: ScheduledBackup$/m.test(value));
assert.ok(cluster);
assert.ok(schedule);
assert.equal(documents.filter((value) => /^kind: ScheduledBackup$/m.test(value)).length, 1);
assert.match(cluster, /plugins:\n\s+- name: barman-cloud\.cloudnative-pg\.io\n\s+isWALArchiver: true\n\s+parameters:\n\s+barmanObjectName: "synveda-backups"/);
assert.match(schedule, /cluster:\n\s+name: synveda-pg/);
assert.match(schedule, /schedule: "0 0 2 \* \* \*"/);
assert.match(schedule, /backupOwnerReference: none/);
assert.match(schedule, /method: plugin\n\s+pluginConfiguration:\n\s+name: barman-cloud\.cloudnative-pg\.io/);
assert.doesNotMatch(enabled.stdout, /ACCESS_SECRET_KEY|objects\.example\.com|replace-with-owned-bucket/);

for (const [name, extra, pluginApi, refusal, database] of [
  ["absent plugin", [], false, "preinstalled Barman Cloud plugin/API", "cnpg"],
  ["wrong database shape", [], true, "requires postgres.mode=cnpg", "external"],
  ["missing object store", ["--set", "postgres.backup.objectStoreName="], true, "existing same-namespace ObjectStore", "cnpg"],
  ["missing schedule", ["--set-string", "postgres.backup.schedule="], true, "explicit six-field", "cnpg"],
  ["malformed schedule", ["--set-string", "postgres.backup.schedule=0 2 * * *"], true, "schedule", "cnpg"],
]) {
  const result = render("enabled", extra, pluginApi, database);
  assert.notEqual(result.status, 0, name);
  assert.match(result.stderr, new RegExp(refusal), `${name}: ${result.stderr}`);
  assert.equal(result.stdout, "", `${name} emitted resources`);
}
for (const [name, extra] of [
  ["orphaned object store name", ["--set", "postgres.backup.objectStoreName=synveda-backups"]],
  ["orphaned schedule", ["--set-string", "postgres.backup.schedule=0 0 2 * * *"]],
]) {
  const result = render("disabled", extra);
  assert.notEqual(result.status, 0, name);
  assert.match(result.stderr, /settings require postgres.backup.enabled=true/);
  assert.equal(result.stdout, "", `${name} emitted resources`);
}
console.log("ok: opt-in Barman Cloud WAL and scheduled backup render/refusal contract");
