import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const HELPER = join(ROOT, "deploy/compose/scripts/recovery-set.mjs");
const BACKUP_SCRIPT = join(ROOT, "deploy/compose/postgres/synveda-logical-backup");
const RESTORE_SCRIPT = join(ROOT, "deploy/compose/postgres/synveda-logical-restore");
const POSTGRES_IMAGE = "synveda/postgres:17.11-dev";
const PROJECT = "synveda-development-acceptance-recovery";
const BACKUP_ID = "20260908-120000";
const TENANT_ID = "019b53c0-7c00-7000-8000-000000000045";

function privateDirectory(path) {
  mkdirSync(path, { recursive: true, mode: 0o700 });
  chmodSync(path, 0o700);
}

function privateFile(path, value) {
  writeFileSync(path, value, { mode: 0o600 });
  chmodSync(path, 0o600);
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function databaseSet(directory, { project = PROJECT, backupId = BACKUP_ID } = {}) {
  privateDirectory(directory);
  const synveda = Buffer.from("synveda-custom-archive\n");
  const keycloak = Buffer.from("keycloak-custom-archive\n");
  privateFile(join(directory, "synveda.dump"), synveda);
  privateFile(join(directory, "keycloak.dump"), keycloak);
  const manifest = {
    format: "synveda-logical-backup-v1",
    backup_id: backupId,
    source_project: project,
    tenant_id: TENANT_ID,
    postgres_image: POSTGRES_IMAGE,
    server_version_num: 170011,
    cluster_system_identifier: "7612345678901234567",
    start_lsn: "0/16B6C50",
    end_lsn: "0/16B6D20",
    started_at: "2026-09-08T12:00:00Z",
    completed_at: "2026-09-08T12:00:01Z",
    synveda: { bytes: synveda.length, sha256: sha256(synveda) },
    keycloak: { bytes: keycloak.length, sha256: sha256(keycloak) },
  };
  privateFile(join(directory, "manifest.json"), `${JSON.stringify(manifest)}\n`);
  return manifest;
}

function snapshotSecrets(scratch, database, keyDirectory, options = {}) {
  const key = join(scratch, "source-kms-key");
  const keyRef = join(scratch, "source-kms-ref");
  const convergencePassword = join(scratch, "source-keycloak-convergence-password");
  const secretSentinel = options.secret ?? "45".repeat(32);
  const convergenceSecret = options.convergenceSecret ?? "67".repeat(32);
  privateFile(key, `${secretSentinel}\n`);
  privateFile(keyRef, "local:recovery-test\n");
  privateFile(convergencePassword, `${convergenceSecret}\n`);
  privateDirectory(keyDirectory);
  const result = spawnSync(process.execPath, [
    HELPER,
    "snapshot-secrets",
    "--key-file", key,
    "--key-ref-file", keyRef,
    "--keycloak-convergence-password-file", convergencePassword,
    "--database-manifest", join(database, "manifest.json"),
    "--output-dir", keyDirectory,
    "--project", options.project ?? PROJECT,
    "--backup-id", options.backupId ?? BACKUP_ID,
  ], { encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
  assert.doesNotMatch(
    `${result.stdout}${result.stderr}`,
    new RegExp(`${secretSentinel}|${convergenceSecret}`),
  );
  return { key, keyRef, convergencePassword, secretSentinel, convergenceSecret };
}

function verify(database, secretDirectory, options = {}) {
  return spawnSync(process.execPath, [
    HELPER,
    "verify",
    "--database-dir", database,
    "--secrets-dir", secretDirectory,
    "--project", options.project ?? PROJECT,
    "--backup-id", options.backupId ?? BACKUP_ID,
    "--tenant-id", options.tenantId ?? TENANT_ID,
    "--postgres-image", options.postgresImage ?? POSTGRES_IMAGE,
  ], { encoding: "utf8" });
}

function executable(path, value) {
  writeFileSync(path, value, { mode: 0o700 });
  chmodSync(path, 0o700);
}

function installContainerUtilityFakes(bin) {
  executable(join(bin, "stat"), `#!${process.execPath}
const { statSync } = require("node:fs");
const [, , flag, format, path] = process.argv;
if (flag !== "-c" || !["%a", "%s", "%u"].includes(format) || !path) process.exit(64);
const stat = statSync(path);
if (format === "%a") process.stdout.write((stat.mode & 0o7777).toString(8) + "\\n");
if (format === "%s") process.stdout.write(String(stat.size) + "\\n");
if (format === "%u") process.stdout.write(String(stat.uid) + "\\n");
`);
  executable(join(bin, "sha256sum"), `#!${process.execPath}
const { createHash } = require("node:crypto");
const { readFileSync } = require("node:fs");
const path = process.argv[2];
if (!path || process.argv.length !== 3) process.exit(64);
process.stdout.write(createHash("sha256").update(readFileSync(path)).digest("hex") + "  " + path + "\\n");
`);
}

test("the recovery set snapshots required secrets separately and verifies streamed archive hashes", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-recovery-set-"));
  const database = join(scratch, "database");
  const keys = join(scratch, "keys");
  try {
    databaseSet(database);
    const { secretSentinel, convergenceSecret } = snapshotSecrets(scratch, database, keys);
    const result = verify(database, keys);
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /logical recovery set valid/);
    assert.doesNotMatch(
      `${result.stdout}${result.stderr}`,
      new RegExp(`${secretSentinel}|${convergenceSecret}`),
    );
    const keyManifest = JSON.parse(readFileSync(join(keys, "manifest.json"), "utf8"));
    assert.deepEqual(readdirSync(keys).sort(), [
      "keycloak_convergence_admin_password",
      "manifest.json",
      "synveda_kms_key",
      "synveda_kms_key_ref",
    ]);
    assert.equal(
      keyManifest.database_manifest_sha256,
      sha256(readFileSync(join(database, "manifest.json"))),
    );
    assert.notEqual(keyManifest.synveda_kms_key.sha256, secretSentinel);
    assert.notEqual(
      keyManifest.keycloak_convergence_admin_password.sha256,
      convergenceSecret,
    );
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("recovery-set validation rejects tampering, extras, unsafe modes and identity drift", () => {
  for (const mutation of ["digest", "convergence", "extra", "mode", "image", "tenant"]) {
    const scratch = mkdtempSync(join(tmpdir(), "synveda-recovery-refusal-"));
    const database = join(scratch, "database");
    const keys = join(scratch, "keys");
    try {
      databaseSet(database);
      snapshotSecrets(scratch, database, keys);
      let options = {};
      if (mutation === "digest") privateFile(join(database, "synveda.dump"), "changed\n");
      if (mutation === "convergence") {
        privateFile(join(keys, "keycloak_convergence_admin_password"), `${"89".repeat(32)}\n`);
      }
      if (mutation === "extra") privateFile(join(database, "unexpected"), "x\n");
      if (mutation === "mode") chmodSync(join(keys, "synveda_kms_key"), 0o644);
      if (mutation === "image") options = { postgresImage: "synveda/postgres:17.12-dev" };
      if (mutation === "tenant") {
        options = { tenantId: "019b53c0-7c00-7000-8000-000000000046" };
      }
      const result = verify(database, keys, options);
      assert.equal(result.status, 78, `${mutation}: ${result.stderr}`);
      assert.match(result.stderr, /recovery-set:/);
    } finally {
      rmSync(scratch, { recursive: true, force: true });
    }
  }
});

test("the logical backup uses PostgreSQL 17 custom archives without globals or password argv", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-logical-backup-"));
  const bin = join(scratch, "bin");
  const output = join(scratch, "output");
  const password = join(scratch, "owner-password");
  const keycloakPasswordFile = join(scratch, "keycloak-password");
  const log = join(scratch, "calls.log");
  const ownerPassword = "ab".repeat(32);
  const keycloakPassword = "ac".repeat(32);
  privateDirectory(bin);
  privateDirectory(output);
  privateFile(password, `${ownerPassword}\n`);
  privateFile(keycloakPasswordFile, `${keycloakPassword}\n`);
  installContainerUtilityFakes(bin);
  executable(join(bin, "pg_dump"), `#!/bin/sh
set -eu
printf 'pg_dump' >> ${JSON.stringify(log)}
destination=
for argument in "$@"; do
  printf ' <%s>' "$argument" >> ${JSON.stringify(log)}
  case "$argument" in --file=*) destination=\${argument#--file=} ;; esac
done
printf '\n' >> ${JSON.stringify(log)}
if [ "$#" -eq 1 ] && [ "$1" = --version ]; then
  printf 'pg_dump (PostgreSQL) 17.11\n'
else
  printf 'custom archive %s\n' "$destination" > "$destination"
fi
`);
  executable(join(bin, "pg_restore"), `#!/bin/sh
set -eu
printf 'pg_restore' >> ${JSON.stringify(log)}
for argument in "$@"; do printf ' <%s>' "$argument" >> ${JSON.stringify(log)}; done
printf '\n' >> ${JSON.stringify(log)}
`);
  executable(join(bin, "psql"), `#!/bin/sh
printf '170011|7612345678901234567|0/16B6C50\n'
`);
  const instrumented = join(scratch, "backup");
  const source = readFileSync(BACKUP_SCRIPT, "utf8")
    .replace(
      "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
      `PATH=${bin}:/usr/bin:/bin`,
    )
    .replace("output=/backup", `output=${output}`)
    .replace(
      "/run/secrets/postgres_owner_password owner",
      `${password} owner`,
    )
    .replace(
      "/run/secrets/keycloak_database_password keycloak",
      `${keycloakPasswordFile} keycloak`,
    );
  executable(instrumented, source);
  try {
    const result = spawnSync(instrumented, ["backup"], {
      env: {
        ...process.env,
        SYNVEDA_BACKUP_PROJECT: PROJECT,
        SYNVEDA_BACKUP_ID: BACKUP_ID,
        SYNVEDA_BACKUP_TENANT_ID: TENANT_ID,
        SYNVEDA_BACKUP_POSTGRES_IMAGE: POSTGRES_IMAGE,
      },
      encoding: "utf8",
    });
    assert.equal(result.status, 0, result.stderr);
    const calls = readFileSync(log, "utf8");
    assert.equal((calls.match(/^pg_dump .*<--dbname=(?:synveda|keycloak)>/gm) ?? []).length, 2);
    assert.match(calls, /<--format=custom>/);
    assert.match(calls, /<--create>/);
    assert.match(calls, /<--dbname=synveda>/);
    assert.match(calls, /<--dbname=keycloak>/);
    assert.match(calls, /<--username=synveda_owner> <--dbname=synveda>/);
    assert.match(calls, /<--username=keycloak> <--dbname=keycloak>/);
    assert.doesNotMatch(calls, /pg_dumpall|<--password(?:=|>)|--no-owner/);
    assert.doesNotMatch(calls, new RegExp(ownerPassword));
    assert.doesNotMatch(calls, new RegExp(keycloakPassword));
    assert.equal((calls.match(/^pg_restore <--list>/gm) ?? []).length, 2);
    assert.deepEqual(readdirSync(output).sort(), [
      "keycloak.dump",
      "manifest.json",
      "synveda.dump",
    ]);
    for (const name of readdirSync(output)) {
      assert.ok(readFileSync(join(output, name)).length > 0);
    }
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("the logical restore validates both archives before recreating either fixed database", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-logical-restore-"));
  const bin = join(scratch, "bin");
  const backup = join(scratch, "backup-set");
  const password = join(scratch, "owner-password");
  const log = join(scratch, "calls.log");
  const ownerPassword = "cd".repeat(32);
  privateDirectory(bin);
  databaseSet(backup);
  privateFile(password, `${ownerPassword}\n`);
  installContainerUtilityFakes(bin);
  executable(join(bin, "pg_restore"), `#!/bin/sh
set -eu
printf 'pg_restore' >> ${JSON.stringify(log)}
for argument in "$@"; do printf ' <%s>' "$argument" >> ${JSON.stringify(log)}; done
printf '\n' >> ${JSON.stringify(log)}
if [ "$#" -eq 1 ] && [ "$1" = --version ]; then
  printf 'pg_restore (PostgreSQL) 17.11\n'
fi
`);
  executable(join(bin, "psql"), `#!/bin/sh
printf 'psql' >> ${JSON.stringify(log)}
for argument in "$@"; do printf ' <%s>' "$argument" >> ${JSON.stringify(log)}; done
printf '\n' >> ${JSON.stringify(log)}
case " $* " in
  *"pg_stat_database"*) printf '1|1|0\n' ;;
  *"pg_class"*) printf '0\n' ;;
  *"pg_database"*) printf '1|1\n' ;;
  *"--command=ANALYZE"*) ;;
  *) exit 91 ;;
esac
`);
  const instrumented = join(scratch, "restore");
  const source = readFileSync(RESTORE_SCRIPT, "utf8")
    .replace(
      "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
      `PATH=${bin}:/usr/bin:/bin`,
    )
    .replace("backup=/backup", `backup=${backup}`)
    .replace(
      "password_file=/run/secrets/postgres_owner_password",
      `password_file=${password}`,
    );
  executable(instrumented, source);
  try {
    const result = spawnSync(instrumented, ["restore"], { encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
    const calls = readFileSync(log, "utf8");
    const lists = [...calls.matchAll(/^pg_restore <--list>/gm)].map((match) => match.index);
    const restores = [...calls.matchAll(/^pg_restore <--host=postgres>/gm)].map((match) => match.index);
    assert.equal(lists.length, 2);
    assert.equal(restores.length, 2);
    assert.ok(Math.max(...lists) < Math.min(...restores), calls);
    assert.equal((calls.match(/^psql .*<--command=ANALYZE>/gm) ?? []).length, 2);
    for (const line of calls.split("\n").filter((line) => line.startsWith("pg_restore <--host"))) {
      assert.match(line, /<--clean>/);
      assert.match(line, /<--if-exists>/);
      assert.match(line, /<--create>/);
      assert.match(line, /<--exit-on-error>/);
      assert.doesNotMatch(line, /<--password(?:=|>)/);
      assert.doesNotMatch(line, new RegExp(ownerPassword));
    }
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("recovery Compose services are private, one-shot, least-mounted and dependency-free", () => {
  const backup = readFileSync(join(ROOT, "deploy/compose/compose.backup.yaml"), "utf8");
  const restore = readFileSync(join(ROOT, "deploy/compose/compose.restore.yaml"), "utf8");
  const postgres = readFileSync(join(ROOT, "deploy/compose/postgres/Dockerfile"), "utf8");
  const lifecycle = readFileSync(join(ROOT, "deploy/compose/scripts/compose.sh"), "utf8");
  assert.doesNotMatch(backup, /^\s*ports:/m);
  assert.match(restore, /^\s+ports: !reset \[\]$/m);
  assert.doesNotMatch(restore.replace(/^\s+ports: !reset \[\]$/m, ""), /^\s*ports:/m);
  for (const source of [backup, restore]) {
    assert.doesNotMatch(source, /^\s*privileged:/m);
    assert.doesNotMatch(source, /docker\.sock|pg_dumpall|start-dev/);
    assert.match(source, /profiles: \[recovery\]/);
    assert.match(source, /cap_drop: \[ALL\]/);
    assert.match(source, /no-new-privileges:true/);
    assert.match(source, /restart: "no"/);
  }
  assert.match(backup, /source: postgres_owner_password/);
  assert.match(backup, /source: keycloak_database_password/);
  assert.doesNotMatch(
    backup,
    /synveda_(?:gateway|worker|migrator)_(?:password|database_url)|keycloak_admin|synveda_kms/,
  );
  assert.match(restore, /ports: !reset \[\]/);
  assert.equal((restore.match(/source: synveda_gateway_database_url/g) ?? []).length, 2);
  assert.doesNotMatch(restore, /synveda_migrator_database_url|\bextends:/);
  assert.match(postgres, /COPY --chmod=0555 deploy\/compose\/postgres\/synveda-logical-backup/);
  assert.match(postgres, /COPY --chmod=0555 deploy\/compose\/postgres\/synveda-logical-restore/);
  for (const marker of [
    "install_recovered_secret synveda_kms_key",
    "install_recovered_secret synveda_kms_key_ref",
    "install_recovered_secret keycloak_convergence_admin_password",
    "stop --timeout 210 gateway worker keycloak-realm-convergence keycloak",
    "run --rm --no-deps database-backup",
    "run --rm --no-deps database-restore",
    "run --rm --no-deps recovery-verify",
    "run --rm --no-deps recovery-key-refusal",
  ]) {
    assert.match(lifecycle, new RegExp(marker.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  }
  const stop = lifecycle.indexOf("stop --timeout 210 gateway worker");
  const backupRun = lifecycle.indexOf("run --rm --no-deps database-backup", stop);
  const resume = lifecycle.indexOf("resume_backup_writers", backupRun);
  assert.ok(stop >= 0 && backupRun > stop && resume > backupRun);
});
