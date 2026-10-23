import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";

import { parsePgbackrestConfig } from "../deploy/compose/scripts/check-pgbackrest-config.mjs";
import {
  CUSTODY_ENTRIES,
  MARK_FORMAT,
  Refused,
  buildHeader,
  listedSegments,
  parseInfo,
  parseMetadata,
  plan,
  requireContiguous,
  requireRepositoryAvailable,
  requireSameRepository,
  requireVerified,
  segment,
  serialize,
  validateHeader,
  validateOpenedEntries,
  walForLsn,
} from "../deploy/compose/scripts/pitr-recovery.mjs";

const TOOL = resolve("deploy/compose/scripts/pitr-recovery.mjs");
const CIPHER = "c".repeat(48);
const CONFIG = `[global]
repo1-type=s3
repo1-path=/synveda/source
repo1-s3-bucket=recovery-bucket
repo1-s3-endpoint=s3.example.test
repo1-s3-region=eu-west-2
repo1-s3-key=access-id
repo1-s3-key-secret=private-key-sentinel
repo1-s3-uri-style=path
repo1-storage-port=8334
repo1-cipher-type=aes-256-cbc
repo1-cipher-pass=${CIPHER}
repo1-storage-verify-tls=y
[synveda]
pg1-path=/var/lib/postgresql/data
`;
const SYSTEM_ID = "7699557602753359914";
// Captured from pgBackRest 2.59.1 against PostgreSQL 17.11 (local drill).
const INFO = `[{"archive":[{"database":{"id":1,"repo-key":1},"id":"17-1","max":"000000010000000000000007","min":"000000010000000000000001"}],"backup":[{"archive":{"start":"000000010000000000000004","stop":"000000010000000000000004"},"backrest":{"format":5,"version":"2.59.1"},"database":{"id":1,"repo-key":1},"error":false,"info":{"delta":23126966,"repository":{"delta":3009344,"size":3009344},"size":23126966},"label":"20261022-181600F","lsn":{"start":"0/4000028","stop":"0/4000120"},"prior":null,"reference":null,"timestamp":{"start":1792692960,"stop":1792692965},"type":"full"}],"cipher":"aes-256-cbc","db":[{"id":1,"repo-key":1,"system-id":${SYSTEM_ID},"version":"17"}],"name":"synveda","repo":[{"cipher":"aes-256-cbc","key":1,"status":{"code":0,"message":"ok"}}],"status":{"code":0,"lock":{"backup":{"held":false},"restore":{"held":false}},"message":"ok"}}]`;
const VERIFIED = `stanza: synveda
status: ok
  archiveId: 17-1, total WAL checked: 7, total valid WAL: 7
    missing: 0, checksum invalid: 0, size invalid: 0, other: 0
  backup: 20261022-181600F, status: valid, total files checked: 968, total valid files: 968
    missing: 0, checksum invalid: 0, size invalid: 0, other: 0
`;
const METADATA = [
  `system_identifier=${SYSTEM_ID}`, "server_version_num=170011", "wal_segment_size=16MB",
  "pgvector=0.8.7", "schema_head=12", `migration_ledger_sha256=${"d".repeat(64)}`,
  "databases=keycloak,synveda", "kek_refs=local:ops5", "deployment_key_generations=1",
  "tenant_key_generations=2", "",
].join("\n");
const VALUES = {
  project: "synveda-development-acceptance-source",
  backupId: "gen-1",
  tenantId: "019b53c0-7c00-7000-8000-000000000045",
  postgresImage: "synveda/postgres:17.11-pitr-dev",
  productImage: "synveda/product:dev",
  custodian: "ops-escrow-1",
};

function listing(numbers, timeline = "00000001") {
  return JSON.stringify(Object.fromEntries([
    [".", { type: "path" }],
    ...numbers.map((number) => {
      const name = `${timeline}${Math.floor(number / 256).toString(16).toUpperCase().padStart(8, "0")}${(number % 256).toString(16).toUpperCase().padStart(8, "0")}`;
      return [`${name.slice(0, 16)}/${name}-${"a".repeat(40)}.gz`, { type: "file", size: 16512, time: 1 }];
    }),
  ]));
}

function header() {
  return buildHeader({
    info: parseInfo(INFO),
    metadata: parseMetadata(METADATA),
    options: parsePgbackrestConfig(CONFIG, false),
    caPresent: false,
    values: VALUES,
    createdAt: "2026-10-22T18:16:10Z",
  });
}

function refusal(fn, pattern) {
  assert.throws(fn, (error) => error instanceof Refused && pattern.test(error.message));
}

test("repository information keeps a 64-bit system identifier exact", () => {
  assert.equal(parseInfo(INFO).db[0]["system-id"], SYSTEM_ID);
  assert.notEqual(String(JSON.parse(INFO)[0].db[0]["system-id"]), SYSTEM_ID);
});

test("repository failures map to content-free operator diagnostics", () => {
  const stanza = (code, message, stanzaCode = 99) => ({
    name: "synveda", cipher: "aes-256-cbc", status: { code: stanzaCode },
    repo: [{ cipher: "aes-256-cbc", status: { code, message } }],
  });
  const garbage = `unable to load info file 'backup.info': FormatError: ${String.fromCharCode(1, 255)}SENTINEL-BYTES`;
  for (const [input, pattern] of [
    [stanza(99, "[ProtocolError] HTTP request failed with 403 (Forbidden)"), /credentials were refused/],
    [stanza(99, "[CryptoError] unable to verify certificate presented by 's3'"), /TLS certificate was not trusted/],
    [stanza(99, garbage), /cipher passphrase is wrong/],
    [stanza(0, "ok", 2), /no valid backup/],
    [stanza(0, "ok", 1), /no synveda stanza/],
    [stanza(99, "[HostConnectError] timeout"), /unavailable/],
  ]) {
    assert.throws(() => requireRepositoryAvailable(input), (error) =>
      error instanceof Refused && pattern.test(error.message) && !error.message.includes("SENTINEL"));
  }
  assert.doesNotThrow(() => requireRepositoryAvailable(parseInfo(INFO)));
  const raw = `[{"archive":[],"backup":[],"cipher":"aes-256-cbc","db":[],"name":"synveda","repo":[{"cipher":"aes-256-cbc","key":1,"status":{"code":99,"message":"[FormatError] unable to load info file 'backup.info': ${String.fromCharCode(1, 10, 31)}SENTINEL"}}],"status":{"code":99,"message":"other"}}]`;
  assert.throws(() => requireRepositoryAvailable(parseInfo(raw)), (error) =>
    error instanceof Refused && /cipher passphrase is wrong/.test(error.message) && !error.message.includes("SENTINEL"));
});

test("a verify report is clean only when every counter is zero, whatever its exit status", () => {
  assert.doesNotThrow(() => requireVerified(VERIFIED));
  const corrupt = VERIFIED.replace("status: ok", "status: error")
    .replace("total valid WAL: 7\n    missing: 0, checksum invalid: 0, size invalid: 0, other: 0", "total valid WAL: 6\n    missing: 0, checksum invalid: 0, size invalid: 0, other: 1");
  refusal(() => requireVerified(corrupt), /corrupt or unreadable/);
  refusal(() => requireVerified(VERIFIED.replace("other: 0\n  backup", "other: 1\n  backup")), /corrupt/);
  refusal(() => requireVerified(""), /corrupt/);
});

test("WAL arithmetic matches PostgreSQL file naming across a high-word boundary", () => {
  assert.equal(walForLsn("00000001", "0/5000090"), "000000010000000000000005");
  assert.equal(walForLsn("00000001", "1/FF000028"), "0000000100000001000000FF");
  assert.equal(segment("0000000100000001000000FF").number + 1, segment("000000010000000200000000").number);
  assert.doesNotThrow(() => requireContiguous(listedSegments(listing([510, 511, 512])), "0000000100000001000000FE", "000000010000000200000000"));
});

test("a missing archived segment is refused by name before restore", () => {
  const names = listedSegments(listing([1, 2, 3, 4, 5, 7]));
  refusal(() => requireContiguous(names, "000000010000000000000004", "000000010000000000000007"), /segment 000000010000000000000006 is missing/);
  assert.doesNotThrow(() => requireContiguous(names, "000000010000000000000004", "000000010000000000000005"));
  refusal(() => requireContiguous(names, "000000010000000000000004", "000000020000000000000005"), /another WAL timeline/);
  refusal(() => listedSegments(JSON.stringify({ [`0000000100000000/000000010000000000000001-${"a".repeat(40)}.gz`]: { type: "file", size: 0 } })), /empty/);
});

test("the custody header binds repository, backup, images and database identity", () => {
  const bound = header();
  assert.equal(bound.backup.label, "20261022-181600F");
  assert.equal(bound.database.system_identifier, SYSTEM_ID);
  assert.deepEqual(bound.entries, CUSTODY_ENTRIES);
  assert.equal(bound.repository.bucket, "recovery-bucket");
  assert.doesNotMatch(serialize(bound), /private-key-sentinel|c{48}|access-id/);
  const expected = { ...VALUES };
  assert.doesNotThrow(() => validateHeader(JSON.parse(serialize(bound)), expected));
  refusal(() => validateHeader(bound, { ...expected, postgresImage: "postgres:17" }), /PostgreSQL image differs/);
  refusal(() => validateHeader(bound, { ...expected, productImage: "synveda/product:other" }), /product image differs/);
  refusal(() => validateHeader(bound, { ...expected, tenantId: "019b53c0-7c00-7000-8000-000000000046" }), /another bootstrap tenant/);
  refusal(() => validateHeader({ ...bound, extra: 1 }, expected), /shape was refused/);
  const otherCluster = METADATA.replace(SYSTEM_ID, "7699557602753359915");
  refusal(() => buildHeader({
    info: parseInfo(INFO), metadata: parseMetadata(otherCluster), options: parsePgbackrestConfig(CONFIG, false),
    caPresent: false, values: VALUES, createdAt: "2026-10-22T18:16:10Z",
  }), /system identifiers differ/);
  refusal(() => parseMetadata(METADATA.replace("16MB", "64MB")), /16 MiB/);
});

test("restore planning proves coverage for latest and recorded named targets", () => {
  const bound = header();
  const input = { header: bound, info: parseInfo(INFO), verifyReport: VERIFIED };
  assert.deepEqual(plan({ ...input, listing: listing([1, 2, 3, 4, 5, 6, 7]), target: "latest" }),
    { set: "20261022-181600F", type: "default", target: "", through_wal: "000000010000000000000007" });
  refusal(() => plan({ ...input, listing: listing([1, 2, 3, 4, 5, 7]), target: "latest" }), /segment 000000010000000000000006 is missing/);
  const mark = {
    format: MARK_FORMAT, source_project: VALUES.project, name: "point-1", lsn: "0/5000090",
    wal: "000000010000000000000005", created_at: "2026-10-22T18:16:05Z",
  };
  assert.deepEqual(plan({ ...input, listing: listing([4, 5]), target: "name:point-1", mark }),
    { set: "20261022-181600F", type: "name", target: "point-1", through_wal: "000000010000000000000005" });
  refusal(() => plan({ ...input, listing: listing([4]), target: "name:point-1", mark }), /missing/);
  refusal(() => plan({ ...input, listing: listing([4, 5]), target: "name:point-1", mark: { ...mark, lsn: "0/4000020", wal: "000000010000000000000004" } }), /precedes the end of the base backup/);
  refusal(() => plan({ ...input, listing: listing([4, 5]), target: "name:other", mark }), /recorded mark/);
  refusal(() => plan({ ...input, listing: listing([4, 5]), target: "time:2026-10-22" }), /latest or name/);
  const moved = parseInfo(INFO.replace('"stop":"0/4000120"', '"stop":"0/4000200"'));
  refusal(() => plan({ ...input, info: moved, listing: listing([4, 5, 6, 7]), target: "latest" }), /changed in the repository/);
});

function openedDirectory(overrides = {}) {
  const dir = mkdtempSync(join(tmpdir(), "synveda-pitr-opened-"));
  chmodSync(dir, 0o700);
  const hex = (character) => `${character.repeat(64)}\n`;
  const values = {
    keycloak_convergence_admin_password: hex("1"), keycloak_database_password: hex("2"),
    pgbackrest_cipher_pass: `${CIPHER}\n`, postgres_owner_password: hex("3"),
    synveda_gateway_password: hex("4"), synveda_kms_key: hex("5"), synveda_kms_key_ref: "local:ops5\n",
    synveda_migrator_password: hex("6"), synveda_worker_password: hex("7"),
  };
  for (const [url, role, password] of [
    ["synveda_migrator_database_url", "synveda_migrator", "6"],
    ["synveda_gateway_database_url", "synveda_gateway", "4"],
    ["synveda_worker_database_url", "synveda_worker", "7"],
  ]) values[url] = `postgres://${role}:${password.repeat(64)}@postgres:5432/synveda\n`;
  for (const [name, value] of Object.entries({ ...values, ...overrides })) {
    writeFileSync(join(dir, name), value, { mode: 0o600 });
    chmodSync(join(dir, name), 0o600);
  }
  writeFileSync(join(dir, "header.json"), serialize(header()), { mode: 0o600 });
  return dir;
}

test("opened custody entries must be the exact consistent credential set", () => {
  const bound = header();
  for (const [overrides, pattern] of [
    [{}, null],
    [{ extra_secret: "x\n" }, /inventory was refused/],
    [{ synveda_gateway_database_url: `postgres://synveda_gateway:${"9".repeat(64)}@postgres:5432/synveda\n` }, /does not match its role credential/],
    [{ synveda_kms_key_ref: "local:another\n" }, /KEK reference is not one/],
    [{ synveda_kms_key: "not-a-key\n" }, /synveda_kms_key was refused/],
  ]) {
    const dir = openedDirectory(overrides);
    try {
      if (pattern) refusal(() => validateOpenedEntries(dir, bound), pattern);
      else assert.equal(validateOpenedEntries(dir, bound).synveda_kms_key_ref, "local:ops5\n");
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  }
  const dir = openedDirectory();
  try {
    const mixed = { ...bound, database: { ...bound.database, kek_refs: ["local:old", "local:ops5"] } };
    refusal(() => validateOpenedEntries(dir, mixed), /more than one KEK reference/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a target repository configuration must name the escrowed repository and passphrase", () => {
  const bound = header();
  const options = parsePgbackrestConfig(CONFIG, false);
  assert.doesNotThrow(() => requireSameRepository(bound, options, false, `${CIPHER}\n`));
  const viaCa = parsePgbackrestConfig(CONFIG.replace(
    "repo1-storage-verify-tls=y\n",
    "repo1-storage-verify-tls=y\nrepo1-storage-ca-file=/var/run/postgresql/pgbackrest-ca.pem\n",
  ), true);
  assert.doesNotThrow(() => requireSameRepository(bound, viaCa, true, `${CIPHER}\n`));
  refusal(() => requireSameRepository(bound, parsePgbackrestConfig(CONFIG.replace("recovery-bucket", "other-bucket"), false), false, `${CIPHER}\n`), /different repository/);
  refusal(() => requireSameRepository(bound, options, false, `${"d".repeat(48)}\n`), /passphrase differs/);
});

test("the planning command refuses without echoing repository secrets", () => {
  const dir = openedDirectory();
  const work = mkdtempSync(join(tmpdir(), "synveda-pitr-cli-"));
  try {
    const files = {
      config: CONFIG.replace("recovery-bucket", "other-bucket"), info: INFO, verify: VERIFIED,
      listing: listing([4, 5, 6, 7]),
    };
    for (const [name, value] of Object.entries(files)) writeFileSync(join(work, name), value, { mode: 0o600 });
    const result = spawnSync(process.execPath, [
      TOOL, "plan", "--opened-dir", dir, "--info", join(work, "info"), "--verify", join(work, "verify"),
      "--listing", join(work, "listing"), "--config", join(work, "config"), "--ca", "false",
      "--target", "latest", "--marks-dir", work, "--project", VALUES.project, "--backup-id", VALUES.backupId,
      "--tenant-id", VALUES.tenantId, "--postgres-image", VALUES.postgresImage, "--product-image", VALUES.productImage,
    ], { encoding: "utf8" });
    assert.equal(result.status, 78, result.stderr);
    assert.match(result.stderr, /names a different repository/);
    assert.doesNotMatch(`${result.stdout}${result.stderr}`, /private-key-sentinel|c{48}|access-id/);
    writeFileSync(join(work, "config"), CONFIG);
    const accepted = spawnSync(process.execPath, [
      TOOL, "plan", "--opened-dir", dir, "--info", join(work, "info"), "--verify", join(work, "verify"),
      "--listing", join(work, "listing"), "--config", join(work, "config"), "--ca", "false",
      "--target", "latest", "--marks-dir", work, "--project", VALUES.project, "--backup-id", VALUES.backupId,
      "--tenant-id", VALUES.tenantId, "--postgres-image", VALUES.postgresImage, "--product-image", VALUES.productImage,
    ], { encoding: "utf8" });
    assert.equal(accepted.status, 0, accepted.stderr);
    assert.equal(accepted.stdout, "20261022-181600F|default||000000010000000000000007\n");
  } finally {
    rmSync(dir, { recursive: true, force: true });
    rmSync(work, { recursive: true, force: true });
  }
});
