import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";

const root = resolve(import.meta.dirname, "..");
const uninstaller = resolve(root, "scripts/uninstall.sh");

function fixture(t) {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-uninstall-"));
  t.after(() => rmSync(scratch, { recursive: true, force: true }));
  const home = join(scratch, "home");
  const install = join(home, ".synveda");
  const sentinel = join(install, "state/synveda-reference/secrets/synveda_kms_key");
  mkdirSync(join(install, "reference/current"), { recursive: true });
  mkdirSync(join(install, "backups/database/synveda-reference"), { recursive: true });
  mkdirSync(join(install, "state/synveda-reference/secrets"), { recursive: true });
  writeFileSync(sentinel, "must survive\n");
  writeFileSync(join(install, "reference/current/synveda-compose"), "must survive\n");
  writeFileSync(join(install, "backups/database/synveda-reference/backup"), "must survive\n");
  const env = { ...process.env, HOME: home, SYNVEDA_HOME: install };
  const run = (...args) =>
    spawnSync("/bin/sh", [uninstaller, ...args], { encoding: "utf8", env });
  return { install, run, sentinel };
}

test("default uninstall refuses without mutating artifacts or key custody", (t) => {
  const f = fixture(t);
  const result = f.run();
  assert.equal(result.status, 69);
  assert.equal(readFileSync(f.sentinel, "utf8"), "must survive\n");
  assert.equal(
    readFileSync(join(f.install, "reference/current/synveda-compose"), "utf8"),
    "must survive\n",
  );
  assert.equal(
    readFileSync(join(f.install, "backups/database/synveda-reference/backup"), "utf8"),
    "must survive\n",
  );
  assert.match(result.stderr, /refusing unproved removal/);
});

test("dry-run is successful and mutation-free", (t) => {
  const f = fixture(t);
  const result = f.run("--dry-run");
  assert.equal(result.status, 0);
  assert.equal(readFileSync(f.sentinel, "utf8"), "must survive\n");
  assert.match(result.stdout, /no mutation was attempted/);
});

test("legacy purge option is refused without touching state", (t) => {
  const f = fixture(t);
  const result = f.run("--purge");
  assert.equal(result.status, 64);
  assert.equal(readFileSync(f.sentinel, "utf8"), "must survive\n");
  assert.match(result.stderr, /accepted options/);
});

test("help documents the canonical lifecycle and succeeds", (t) => {
  const f = fixture(t);
  const result = f.run("--help");
  assert.equal(result.status, 0);
  assert.match(result.stdout, /synveda-compose down/);
  assert.equal(readFileSync(f.sentinel, "utf8"), "must survive\n");
});

test("source contains no legacy process, Docker or recursive deletion path", () => {
  const source = readFileSync(uninstaller, "utf8");
  for (const forbidden of [
    /gateway\.pid/,
    /\bkill\b/,
    /docker compose/,
    /down -v/,
    /docker volume/,
    /rm -rf/,
    /\bsudo\b/,
    /rauthy/i,
  ]) {
    assert.doesNotMatch(source, forbidden);
  }
});
