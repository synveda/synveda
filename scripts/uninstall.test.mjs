import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";

const root = resolve(import.meta.dirname, "..");
const uninstaller = resolve(root, "scripts/uninstall.sh");

function fixture(t) {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-uninstall-"));
  t.after(() => rmSync(scratch, { recursive: true, force: true }));

  const home = join(scratch, "home");
  const install = join(home, ".synveda");
  const installedBin = join(scratch, "installed-bin");
  const fakeBin = join(scratch, "fake-bin");
  const dockerLog = join(scratch, "docker.log");
  const key = join(install, "data/kms.key");

  for (const path of [
    join(install, "bin"),
    join(install, "console"),
    join(install, "profile"),
    join(install, "plugin"),
    join(install, "data/cache"),
    installedBin,
    fakeBin,
  ]) {
    mkdirSync(path, { recursive: true });
  }
  writeFileSync(join(install, "bin/synveda-gateway"), "gateway\n");
  writeFileSync(join(install, "bin/synveda-worker"), "worker\n");
  writeFileSync(join(install, "console/index.html"), "console\n");
  writeFileSync(join(install, "profile/docker-compose.yml"), "name: synveda\n");
  writeFileSync(join(install, "plugin/manifest.json"), "{}\n");
  writeFileSync(key, `${"ab".repeat(32)}\n`, { mode: 0o600 });
  writeFileSync(join(install, "data/gateway.pid"), "not-a-pid\n");
  writeFileSync(join(install, "data/gateway.log"), "runtime log\n");
  writeFileSync(join(install, "data/gateway.config"), "runtime config\n");
  writeFileSync(join(install, "data/.runtime"), "hidden runtime state\n");
  writeFileSync(join(install, "data/cache/transient"), "cache\n");
  writeFileSync(join(installedBin, "synveda"), "cli\n");

  const docker = join(fakeBin, "docker");
  writeFileSync(
    docker,
    '#!/bin/sh\nprintf "%s\\n" "$*" >> "$SYNVEDA_TEST_DOCKER_LOG"\nexit "${SYNVEDA_TEST_DOCKER_EXIT:-0}"\n',
  );
  chmodSync(docker, 0o755);

  const env = {
    ...process.env,
    HOME: home,
    PATH: `${fakeBin}:/usr/bin:/bin`,
    SYNVEDA_BIN: installedBin,
    SYNVEDA_HOME: install,
    SYNVEDA_TEST_DOCKER_LOG: dockerLog,
  };
  const run = (...args) =>
    execFileSync("/bin/sh", [uninstaller, ...args], {
      encoding: "utf8",
      env,
    });
  const runResult = (...args) =>
    spawnSync("/bin/sh", [uninstaller, ...args], {
      encoding: "utf8",
      env,
    });

  return { dockerLog, env, install, installedBin, key, run, runResult };
}

test("default uninstall keeps the key with the persistent volumes", (t) => {
  const f = fixture(t);
  const keyBefore = readFileSync(f.key, "utf8");

  const output = f.run();

  assert.equal(readFileSync(f.key, "utf8"), keyBefore);
  assert.equal(statSync(f.key).mode & 0o777, 0o600);
  assert.deepEqual(readdirSync(dirname(f.key)), ["kms.key"]);
  assert.equal(existsSync(join(f.install, "profile")), false);
  assert.equal(existsSync(join(f.installedBin, "synveda")), false);
  assert.equal(
    readFileSync(f.dockerLog, "utf8").trim(),
    `compose -f ${join(f.install, "profile/docker-compose.yml")} down`,
  );
  assert.match(output, /preserved .*data\/kms\.key.*retained volumes/);
  assert.match(output, /Your data is still here, in three Docker volumes/);
  assert.match(output, /Its key is still here/);

  const second = f.run();
  assert.equal(readFileSync(f.key, "utf8"), keyBefore);
  assert.match(second, /Nothing else was installed here/);
});

test("explicit purge destroys both volumes and their key", (t) => {
  const f = fixture(t);

  const output = f.run("--purge");

  assert.equal(existsSync(f.install), false);
  assert.equal(existsSync(f.key), false);
  assert.equal(
    readFileSync(f.dockerLog, "utf8").trim(),
    `compose -f ${join(f.install, "profile/docker-compose.yml")} down -v`,
  );
  assert.match(output, /volumes and .*data\/kms\.key are gone/);
  assert.match(output, /explicit purge/);
});

test("purge dry-run reports key destruction without changing anything", (t) => {
  const f = fixture(t);
  const keyBefore = readFileSync(f.key, "utf8");

  const output = f.run("--purge", "--dry-run");

  assert.equal(readFileSync(f.key, "utf8"), keyBefore);
  assert.equal(existsSync(join(f.install, "profile/docker-compose.yml")), true);
  assert.equal(existsSync(join(f.install, "data/gateway.log")), true);
  assert.equal(existsSync(join(f.installedBin, "synveda")), true);
  assert.equal(existsSync(f.dockerLog), false);
  assert.match(output, /would run: docker compose down -v.*DESTROYS the volumes/);
  assert.match(output, /would remove .*data.*kms\.key; explicit purge/);
  assert.match(output, /Nothing was changed/);
});

test("failed volume purge fails closed and preserves the key", (t) => {
  const f = fixture(t);
  const keyBefore = readFileSync(f.key, "utf8");
  f.env.SYNVEDA_TEST_DOCKER_EXIT = "1";

  const result = f.runResult("--purge");

  assert.equal(result.status, 1);
  assert.equal(readFileSync(f.key, "utf8"), keyBefore);
  assert.deepEqual(readdirSync(dirname(f.key)), ["kms.key"]);
  assert.match(result.stdout, /down -v failed; volumes may remain/);
  assert.match(result.stdout, /Purge did not complete/);
  assert.match(result.stdout, /their key remains/);
});

test("default uninstall refuses to traverse a linked data directory", (t) => {
  const f = fixture(t);
  const linkedData = join(dirname(f.install), "linked-data");
  const linkedKey = join(linkedData, "kms.key");
  const linkedState = join(linkedData, "gateway.log");
  mkdirSync(linkedData, { recursive: true });
  writeFileSync(linkedKey, `${"cd".repeat(32)}\n`, { mode: 0o600 });
  writeFileSync(linkedState, "must survive\n");
  rmSync(join(f.install, "data"), { recursive: true });
  symlinkSync(linkedData, join(f.install, "data"));

  const result = f.runResult();

  assert.equal(result.status, 1);
  assert.equal(readFileSync(linkedKey, "utf8"), `${"cd".repeat(32)}\n`);
  assert.equal(readFileSync(linkedState, "utf8"), "must survive\n");
  assert.match(result.stdout, /refusing to traverse a symlink/);
  assert.match(result.stdout, /Uninstall was incomplete/);
});
