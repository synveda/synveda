import assert from "node:assert/strict";
import { test } from "node:test";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { chmodSync, linkSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { recoverySet, configurationFiles } from "../deploy/compose/scripts/evaluation-recovery.mjs";
import { inventory } from "../deploy/compose/scripts/initialize-consumer.mjs";
import { restoreConsumerConfiguration, verifyConsumerConfiguration } from "../deploy/compose/scripts/consumer-recovery.mjs";

const project = "synveda-local-acceptance-recovery";
const id = "paired-test";
const manifest = { images: { postgres: `test/postgres@sha256:${"a".repeat(64)}` } };
const identity = { schema_version: 1, project, manifest, options: { port: 8080, demoAccounts: true } };
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const put = (file, bytes) => writeFileSync(file, bytes, { mode: 0o600 });
const json = (file, value) => put(file, `${JSON.stringify(value)}\n`);
const directory = (path) => mkdirSync(path, { recursive: true, mode: 0o700 });
function fixture(t) {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-consumer-recovery-"));
  chmodSync(scratch, 0o700);
  t.after(() => rmSync(scratch, { recursive: true, force: true }));
  const root = join(scratch, id), state = join(scratch, "source"), target = join(scratch, "target");
  directory(`${root}/database`); directory(`${state}/secrets`); directory(target);
  directory(`${state}/secrets/oidc-directory`);
  for (const name of ["synveda_kms_key", "keycloak_convergence_admin_password"]) put(`${state}/secrets/${name}`, `${"12".repeat(32)}\n`);
  put(`${state}/secrets/synveda_kms_key_ref`, "local:test\n");
  json(`${state}/installation.json`, identity.options); json(`${state}/issuers.json`, [{ issuer: "http://localhost:8080/realms/synveda" }]);
  json(`${scratch}/consumer.json`, identity);
  json(`${scratch}/consumer-secrets.json`, inventory(`${state}/secrets`, process.getuid()));
  const dump = Buffer.from("fixture logical archive");
  for (const name of ["synveda", "keycloak"]) put(`${root}/database/${name}.dump`, dump);
  json(`${root}/database/manifest.json`, {
    format: "synveda-logical-backup-v1", backup_id: id, source_project: project,
    tenant_id: "019b53c0-7c00-7000-8000-000000000045", postgres_image: manifest.images.postgres,
    server_version_num: 170011, cluster_system_identifier: "7612345678901234567",
    start_lsn: "0/16B6C50", end_lsn: "0/16B6D20", started_at: "2026-09-21T12:00:00Z", completed_at: "2026-09-21T12:00:01Z",
    synveda: { bytes: dump.length, sha256: hash(dump) }, keycloak: { bytes: dump.length, sha256: hash(dump) },
  });
  const settings = { root, state, manifest, project, extraConfiguration: { "consumer.json": `${scratch}/consumer.json`, "consumer-secrets.json": `${scratch}/consumer-secrets.json` } };
  recoverySet("snapshot", id, settings);
  return { scratch, root, state, target, settings, configuration: `${root}/configuration` };
}

test("consumer recovery reuses the paired set and restores original sealed bytes into an empty project", (t) => {
  const f = fixture(t);
  recoverySet("verify", id, f.settings);
  verifyConsumerConfiguration(f.configuration, identity);
  const targetIdentity = { ...identity, project: "synveda-local-acceptance-restored" };
  restoreConsumerConfiguration(f.configuration, f.target, targetIdentity);
  assert.deepEqual(inventory(`${f.target}/synveda-evaluation/secrets`, process.getuid()), inventory(`${f.state}/secrets`, process.getuid()));
  assert.deepEqual(JSON.parse(readFileSync(`${f.target}/consumer.json`)), targetIdentity);
  assert.equal(readFileSync(`${f.target}/consumer-secrets.json`, "utf8"), readFileSync(`${f.configuration}/consumer-secrets.json`, "utf8"));
  assert.deepEqual(readdirSync(`${f.target}/synveda-evaluation/secrets/oidc-directory`), []);
  assert.ok(readdirSync(f.target).includes(".recovery-operation"));
  assert.throws(() => restoreConsumerConfiguration(f.configuration, f.target, targetIdentity), /not empty/);
  assert.throws(() => recoverySet("snapshot", id, f.settings), /EEXIST/);
});

test("recovery refuses issuer/release/source drift, lost seals and altered configuration", (t) => {
  const f = fixture(t);
  for (const changed of [{ ...identity, project: "synveda-local" }, { ...identity, manifest: {} }, { ...identity, options: { port: 9090 } }]) {
    assert.throws(() => verifyConsumerConfiguration(f.configuration, changed), /differs/);
  }
  put(`${f.configuration}/secrets/synveda_kms_key`, `${"23".repeat(32)}\n`);
  assert.throws(() => verifyConsumerConfiguration(f.configuration, identity), /seal/);
  assert.throws(() => recoverySet("verify", id, f.settings), /checksum mismatch/);
  // Even a recomputed configuration inventory cannot substitute another key
  // for the separately linked key set.
  put(`${f.root}/configuration.json`, JSON.stringify(configurationFiles(f.configuration)));
  assert.throws(() => recoverySet("verify", id, f.settings), /paired recovery keys/);
  rmSync(`${f.configuration}/consumer-secrets.json`);
  assert.throws(() => verifyConsumerConfiguration(f.configuration, identity));
  assert.deepEqual(readdirSync(f.target), []);
});

test("recovery configuration refuses symlinks, hardlinks and unsafe private modes", (t) => {
  const f = fixture(t), file = `${f.configuration}/issuers.json`, bytes = readFileSync(file);
  chmodSync(file, 0o644);
  assert.throws(() => recoverySet("verify", id, f.settings), /permissions/);
  rmSync(file); symlinkSync(`${f.state}/issuers.json`, file);
  assert.throws(() => recoverySet("verify", id, f.settings), /permissions/);
  rmSync(file); linkSync(`${f.state}/issuers.json`, file);
  assert.throws(() => recoverySet("verify", id, f.settings), /permissions/);
  rmSync(file); put(file, bytes);
  recoverySet("verify", id, f.settings);
});

function shellFixture(t, scenario = "") {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-recovery-shell-"));
  t.after(() => rmSync(scratch, { recursive: true, force: true }));
  const log = join(scratch, "calls.jsonl");
  writeFileSync(join(scratch, "docker"), `#!${process.execPath}
const fs=require('node:fs'), a=process.argv.slice(2);
fs.appendFileSync(process.env.RECOVERY_TEST_LOG, JSON.stringify(a)+'\\n');
if(a[0]==='volume'&&a[1]==='inspect') {
 const name=a.at(-1); console.log(name.slice(0,name.lastIndexOf('_'))+':'+name.slice(name.lastIndexOf('_')+1));
}
if(a[0]==='volume'&&a[1]==='ls') {
 if(process.env.RECOVERY_TEST_SCENARIO==='retained') console.log('retained-installation');
 if(process.env.RECOVERY_TEST_SCENARIO==='unavailable') process.exit(69);
}
if(a.includes('snapshot')&&process.env.RECOVERY_TEST_SCENARIO==='interrupted') process.exit(78);
`, { mode: 0o755 });
  const env = { PATH: `${scratch}:${process.env.PATH}`, DOCKER_HOST: "unix:///test.sock", COMPOSE_PROJECT_NAME: project,
    RECOVERY_TEST_LOG: log, RECOVERY_TEST_SCENARIO: scenario };
  return { env, run: (args, extra = {}) => spawnSync("sh", [resolve("deploy/compose/scripts/consumer-recovery.sh"), ...args], { env: { ...env, ...extra }, encoding: "utf8" }),
    calls: () => { try { return readFileSync(log, "utf8").trim().split("\n").map(JSON.parse); } catch { return []; } } };
}

test("consumer restore refuses missing confirmation, retained targets and failed enumeration before mutations", (t) => {
  const source = "synveda-local";
  for (const scenario of ["confirmation", "retained", "unavailable"]) {
    const f = shellFixture(t, scenario);
    const r = f.run(["restore", id, source], scenario === "confirmation" ? {} : { SYNVEDA_CONFIRM_RESTORE: `${source}:${id}:${project}` });
    assert.notEqual(r.status, 0, scenario);
    assert.equal(f.calls().some((args) => args[0] === "compose"), false, scenario);
  }
});

test("consumer recovery rejects host-state projects and unsafe IDs before Docker", (t) => {
  const f = shellFixture(t);
  for (const name of ["synveda-evaluation", "synveda-development", "synveda-local-acceptance-bad--name", `synveda-local-acceptance-${"a".repeat(25)}`]) {
    assert.equal(f.run(["backup", id], { COMPOSE_PROJECT_NAME: name }).status, 78);
  }
  for (const badId of ["../escape", "-prefix", "double--dash", "a".repeat(65)]) assert.equal(f.run(["backup", badId]).status, 78);
  assert.deepEqual(f.calls(), []);
});

test("consumer restore validates before copying, reconverges authority, verifies keys, then starts writers", (t) => {
  const f = shellFixture(t), source = "synveda-local";
  const r = f.run(["restore", id, source], { SYNVEDA_CONFIRM_RESTORE: `${source}:${id}:${project}` });
  assert.equal(r.status, 0, r.stderr);
  const calls = f.calls().map((args) => args.join(" "));
  const steps = ["recovery-check verify", "recovery-state restore", "--wait-timeout 180 postgres", "run --rm --no-deps database-restore", "run --rm --no-deps recovery-verify", "run --rm --no-deps recovery-key-refusal", "recovery-state unlock", "--wait-timeout 600"];
  let previous = -1;
  for (const step of steps) {
    const index = calls.findIndex((line) => line.includes(step));
    assert.ok(index > previous, step); previous = index;
  }
  assert.equal(calls.filter((line) => line.endsWith("run --rm --no-deps database-bootstrap")).length, 2);
  assert.equal(calls.filter((line) => line.endsWith("run --rm --no-deps keycloak-database-bootstrap")).length, 2);
  assert.equal(calls.some((line) => line.includes("volume inspect") && /_(installation|postgres-data)$/.test(line)), false, "restore needs only the recovery volume");
});

test("consumer backup quiesces all profiles and leaves an interrupted marker for inspection", (t) => {
  for (const scenario of ["", "interrupted"]) {
    const f = shellFixture(t, scenario), r = f.run(["backup", id]);
    assert.equal(r.status, scenario ? 78 : 0, r.stderr);
    const calls = f.calls().map((args) => args.join(" "));
    const stop = calls.findIndex((line) => line.includes("--profile * stop --timeout 210"));
    const dump = calls.findIndex((line) => line.endsWith("run --rm --no-deps database-backup"));
    assert.ok(stop > 0 && dump > stop);
    assert.ok(calls.some((line) => line.includes("up -d --no-deps --no-build --wait --wait-timeout 180 postgres")));
    assert.equal(calls.some((line) => line.includes("recovery-state unlock")), scenario === "");
    if (scenario) assert.match(r.stderr, /interrupted/);
  }
});
