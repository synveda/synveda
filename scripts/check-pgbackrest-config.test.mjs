import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";

const CHECK = resolve("deploy/compose/scripts/check-pgbackrest-config.mjs");
const VALID = `[global]
pg1-path=/var/lib/postgresql/data
repo1-type=s3
repo1-path=/synveda
repo1-s3-bucket=backup-bucket
repo1-s3-endpoint=s3.example.test
repo1-s3-region=eu-west-2
repo1-s3-key=access-id
repo1-s3-key-secret=private-key-sentinel
repo1-cipher-type=aes-256-cbc
repo1-cipher-pass=${"a".repeat(48)}
repo1-storage-verify-tls=y
`;

function check(source) {
  const dir = mkdtempSync(join(tmpdir(), "synveda-pgbackrest-config-"));
  const path = join(dir, "pgbackrest.conf");
  try {
    writeFileSync(path, source, { mode: 0o600 });
    chmodSync(path, 0o600);
    return spawnSync(process.execPath, [CHECK, path], { encoding: "utf8" });
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test("S3 backup configuration accepts static and automatic AWS credentials", () => {
  assert.equal(check(VALID).status, 0);
  const auto = VALID.replace(
    "repo1-s3-key=access-id\nrepo1-s3-key-secret=private-key-sentinel\n",
    "repo1-s3-key-type=auto\n",
  );
  assert.equal(check(auto).status, 0);
});

test("S3 backup configuration refuses weaker or ambiguous authority", () => {
  const cases = [
    VALID.replace("repo1-type=s3", "repo1-type=posix"),
    VALID.replace("repo1-cipher-type=aes-256-cbc", "repo1-cipher-type=none"),
    VALID.replace("repo1-storage-verify-tls=y", "repo1-storage-verify-tls=n"),
    VALID.replace(`repo1-cipher-pass=${"a".repeat(48)}`, "repo1-cipher-pass=short"),
    VALID.replace("repo1-s3-bucket=backup-bucket\n", ""),
    VALID.replace("repo1-path=/synveda", "repo1-path=/"),
    VALID + "repo1-s3-key-secret=duplicate\n",
    VALID + "[global:backup]\nexpire-auto=y\n",
    VALID + "repo1-s3-process-cmd=/bin/sh\n",
  ];
  for (const source of cases) {
    const result = check(source);
    assert.equal(result.status, 78, result.stderr);
    assert.doesNotMatch(result.stderr + result.stdout, /private-key-sentinel/);
  }
});
