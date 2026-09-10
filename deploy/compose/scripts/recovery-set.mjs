#!/usr/bin/env node
import { createHash } from "node:crypto";
import {
  closeSync,
  constants,
  fstatSync,
  lstatSync,
  openSync,
  readFileSync,
  readSync,
  readdirSync,
  writeFileSync,
} from "node:fs";
import { isAbsolute } from "node:path";

const PROJECT = /^synveda-(development|reference)(-acceptance-[a-z0-9](?:[a-z0-9-]{0,22}[a-z0-9])?)?$/;
const BACKUP_ID = /^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/;
const IMAGE = /^[A-Za-z0-9_./:@+-]+$/;
const SHA256 = /^[0-9a-f]{64}$/;
const LSN = /^[0-9A-F]+\/[0-9A-F]+$/;
const UTC_SECOND = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/;
const UID = typeof process.getuid === "function" ? process.getuid() : undefined;
const GID = typeof process.getgid === "function" ? process.getgid() : undefined;

function fail(message, status = 78) {
  process.stderr.write(`recovery-set: ${message}\n`);
  process.exit(status);
}

function argumentsFor(argv) {
  const action = argv[2];
  if (!new Set(["snapshot-secrets", "verify"]).has(action)) fail("invalid action", 64);
  const values = {};
  for (let index = 3; index < argv.length; index += 2) {
    const name = argv[index];
    const value = argv[index + 1];
    if (!name?.startsWith("--") || value === undefined || value === "") {
      fail("invalid arguments", 64);
    }
    const key = name.slice(2);
    if (values[key] !== undefined) fail("duplicate argument", 64);
    values[key] = value;
  }
  const expected = action === "snapshot-secrets"
    ? [
        "backup-id", "database-manifest", "key-file", "key-ref-file",
        "keycloak-convergence-password-file", "output-dir", "project",
      ]
    : [
        "backup-id", "database-dir", "postgres-image", "project", "secrets-dir",
        "tenant-id",
      ];
  if (
    Object.keys(values).length !== expected.length ||
    expected.some((name) => values[name] === undefined)
  ) fail("invalid arguments", 64);
  return { action, values };
}

function exactKeys(value, keys, label) {
  if (
    value === null || Array.isArray(value) || typeof value !== "object" ||
    JSON.stringify(Object.keys(value)) !== JSON.stringify(keys)
  ) fail(`${label} shape was refused`);
}

function safePath(path, label) {
  if (!isAbsolute(path) || path.length > 1024 || /[\0\r\n]/.test(path)) {
    fail(`${label} path was refused`, 64);
  }
  return path;
}

function exactMetadata(stat, kind, mode, label, size) {
  const typeOk = kind === "file" ? stat.isFile() : stat.isDirectory();
  if (
    UID === undefined || GID === undefined || !typeOk || stat.isSymbolicLink() ||
    (stat.mode & 0o7777) !== mode || stat.uid !== UID || stat.gid !== GID ||
    (kind === "file" && stat.nlink !== 1) ||
    (size !== undefined && (stat.size < size.min || stat.size > size.max))
  ) fail(`${label} metadata was refused`);
}

function directory(path, expectedEntries, label) {
  safePath(path, label);
  let stat;
  let entries;
  try {
    stat = lstatSync(path);
    entries = readdirSync(path).sort();
  } catch {
    fail(`${label} directory was unavailable`);
  }
  exactMetadata(stat, "directory", 0o700, label);
  if (JSON.stringify(entries) !== JSON.stringify([...expectedEntries].sort())) {
    fail(`${label} inventory was refused`);
  }
}

function sameFile(left, right) {
  return left.dev === right.dev && left.ino === right.ino && left.mode === right.mode &&
    left.uid === right.uid && left.gid === right.gid && left.nlink === right.nlink &&
    left.size === right.size && left.mtimeMs === right.mtimeMs && left.ctimeMs === right.ctimeMs;
}

function readBounded(path, label, min, max) {
  safePath(path, label);
  let descriptor;
  let before;
  let buffer;
  try {
    const pathStat = lstatSync(path);
    exactMetadata(pathStat, "file", 0o600, label, { min, max });
    descriptor = openSync(
      path,
      constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0) | (constants.O_NONBLOCK ?? 0),
    );
    before = fstatSync(descriptor);
    exactMetadata(before, "file", 0o600, label, { min, max });
    if (!sameFile(pathStat, before)) fail(`${label} identity was refused`);
    buffer = Buffer.alloc(before.size);
    let offset = 0;
    while (offset < buffer.length) {
      const count = readSync(descriptor, buffer, offset, buffer.length - offset, null);
      if (!Number.isSafeInteger(count) || count <= 0) fail(`${label} read was refused`);
      offset += count;
    }
    const after = fstatSync(descriptor);
    if (!sameFile(before, after)) fail(`${label} changed while it was read`);
    return buffer;
  } catch (error) {
    buffer?.fill(0);
    if (error?.message?.startsWith("recovery-set:")) throw error;
    fail(`${label} was unavailable`);
  } finally {
    if (descriptor !== undefined) {
      try {
        closeSync(descriptor);
      } catch {
        buffer?.fill(0);
        fail(`${label} close failed`);
      }
    }
  }
}

function hashFile(path, label, expectedBytes) {
  safePath(path, label);
  let descriptor;
  try {
    const pathStat = lstatSync(path);
    exactMetadata(pathStat, "file", 0o600, label, { min: 1, max: Number.MAX_SAFE_INTEGER });
    if (!Number.isSafeInteger(pathStat.size) || pathStat.size !== expectedBytes) {
      fail(`${label} size did not match its manifest`);
    }
    descriptor = openSync(
      path,
      constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0) | (constants.O_NONBLOCK ?? 0),
    );
    const before = fstatSync(descriptor);
    if (!sameFile(pathStat, before)) fail(`${label} identity was refused`);
    const hash = createHash("sha256");
    const chunk = Buffer.alloc(1024 * 1024);
    let total = 0;
    for (;;) {
      const count = readSync(descriptor, chunk, 0, chunk.length, null);
      if (!Number.isSafeInteger(count) || count < 0) fail(`${label} read was refused`);
      if (count === 0) break;
      hash.update(chunk.subarray(0, count));
      total += count;
    }
    chunk.fill(0);
    const after = fstatSync(descriptor);
    if (total !== expectedBytes || !sameFile(before, after)) {
      fail(`${label} changed while it was hashed`);
    }
    return hash.digest("hex");
  } catch (error) {
    if (error?.message?.startsWith("recovery-set:")) throw error;
    fail(`${label} was unavailable`);
  } finally {
    if (descriptor !== undefined) {
      try {
        closeSync(descriptor);
      } catch {
        fail(`${label} close failed`);
      }
    }
  }
}

function jsonManifest(path, label) {
  const raw = readBounded(path, label, 3, 8192);
  let value;
  try {
    value = JSON.parse(raw.toString("utf8"));
  } catch {
    raw.fill(0);
    fail(`${label} was not JSON`);
  }
  const canonical = Buffer.from(`${JSON.stringify(value)}\n`);
  if (!raw.equals(canonical)) {
    raw.fill(0);
    fail(`${label} was not canonical JSON`);
  }
  raw.fill(0);
  return value;
}

function validateDatabaseManifest(value, expected) {
  exactKeys(value, [
    "format", "backup_id", "source_project", "tenant_id", "postgres_image",
    "server_version_num", "cluster_system_identifier", "start_lsn", "end_lsn",
    "started_at", "completed_at", "synveda", "keycloak",
  ], "database manifest");
  for (const name of ["synveda", "keycloak"]) {
    exactKeys(value[name], ["bytes", "sha256"], `${name} archive manifest`);
    if (
      !Number.isSafeInteger(value[name].bytes) || value[name].bytes < 1 ||
      !SHA256.test(value[name].sha256)
    ) fail(`${name} archive manifest was refused`);
  }
  if (
    value.format !== "synveda-logical-backup-v1" ||
    value.backup_id !== expected.backupId ||
    value.source_project !== expected.project ||
    value.tenant_id !== expected.tenantId ||
    value.postgres_image !== expected.postgresImage ||
    !Number.isSafeInteger(value.server_version_num) ||
    value.server_version_num < 170000 || value.server_version_num >= 180000 ||
    !/^\d{10,24}$/.test(value.cluster_system_identifier) ||
    !LSN.test(value.start_lsn) || !LSN.test(value.end_lsn) ||
    !UTC_SECOND.test(value.started_at) || !UTC_SECOND.test(value.completed_at) ||
    !Number.isFinite(Date.parse(value.started_at)) ||
    !Number.isFinite(Date.parse(value.completed_at)) ||
    Date.parse(value.started_at) > Date.parse(value.completed_at)
  ) fail("database manifest values were refused");
  return value;
}

function validateSecretManifest(value, expected) {
  exactKeys(value, [
    "format", "backup_id", "source_project", "database_manifest_sha256",
    "synveda_kms_key", "synveda_kms_key_ref", "keycloak_convergence_admin_password",
  ], "recovery-secret manifest");
  for (const name of [
    "synveda_kms_key", "synveda_kms_key_ref", "keycloak_convergence_admin_password",
  ]) {
    exactKeys(value[name], ["bytes", "sha256"], `${name} manifest`);
    if (
      !Number.isSafeInteger(value[name].bytes) || value[name].bytes < 1 ||
      !SHA256.test(value[name].sha256)
    ) fail(`${name} manifest was refused`);
  }
  if (
    value.format !== "synveda-recovery-secrets-v1" ||
    value.backup_id !== expected.backupId ||
    value.source_project !== expected.project ||
    !SHA256.test(value.database_manifest_sha256)
  ) fail("recovery-secret manifest values were refused");
  return value;
}

function validateIdentity(values) {
  if (!PROJECT.test(values.project)) fail("project was refused", 64);
  if (!BACKUP_ID.test(values["backup-id"]) || values["backup-id"].length > 64) {
    fail("backup id was refused", 64);
  }
  if (
    values["tenant-id"] !== undefined &&
    !/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(
      values["tenant-id"],
    )
  ) fail("tenant UUIDv7 was refused", 64);
}

function snapshotSecrets(values) {
  validateIdentity(values);
  directory(values["output-dir"], [], "recovery-secret output");
  const key = readBounded(values["key-file"], "KMS key", 65, 65);
  const keyRef = readBounded(values["key-ref-file"], "KMS key reference", 7, 257);
  const convergencePassword = readBounded(
    values["keycloak-convergence-password-file"], "Keycloak convergence password", 65, 65,
  );
  const keyText = key.toString("ascii");
  const keyRefText = keyRef.toString("utf8");
  const convergencePasswordText = convergencePassword.toString("ascii");
  if (
    !/^[0-9a-f]{64}\n$/.test(keyText) ||
    !/^local:[A-Za-z0-9._:-]{1,250}\n$/.test(keyRefText) ||
    !/^[0-9a-f]{64}\n$/.test(convergencePasswordText)
  ) {
    key.fill(0);
    keyRef.fill(0);
    convergencePassword.fill(0);
    fail("recovery secret material was refused");
  }
  const databaseManifest = readBounded(
    values["database-manifest"], "database manifest", 3, 8192,
  );
  const databaseManifestSha256 = createHash("sha256").update(databaseManifest).digest("hex");
  databaseManifest.fill(0);
  const keyHash = createHash("sha256").update(key).digest("hex");
  const keyRefHash = createHash("sha256").update(keyRef).digest("hex");
  const convergencePasswordHash = createHash("sha256").update(convergencePassword).digest("hex");
  try {
    writeFileSync(`${values["output-dir"]}/synveda_kms_key`, key, {
      flag: "wx",
      mode: 0o600,
    });
    writeFileSync(`${values["output-dir"]}/synveda_kms_key_ref`, keyRef, {
      flag: "wx",
      mode: 0o600,
    });
    writeFileSync(
      `${values["output-dir"]}/keycloak_convergence_admin_password`,
      convergencePassword,
      { flag: "wx", mode: 0o600 },
    );
    const manifest = {
      format: "synveda-recovery-secrets-v1",
      backup_id: values["backup-id"],
      source_project: values.project,
      database_manifest_sha256: databaseManifestSha256,
      synveda_kms_key: { bytes: key.length, sha256: keyHash },
      synveda_kms_key_ref: { bytes: keyRef.length, sha256: keyRefHash },
      keycloak_convergence_admin_password: {
        bytes: convergencePassword.length,
        sha256: convergencePasswordHash,
      },
    };
    writeFileSync(`${values["output-dir"]}/manifest.json`, `${JSON.stringify(manifest)}\n`, {
      flag: "wx",
      mode: 0o600,
    });
  } catch {
    fail("recovery secret material could not be staged");
  } finally {
    key.fill(0);
    keyRef.fill(0);
    convergencePassword.fill(0);
  }
  process.stdout.write(
    `recovery secrets staged for ${values.project} as ${values["backup-id"]}\n`,
  );
}

function verify(values) {
  validateIdentity(values);
  if (!IMAGE.test(values["postgres-image"])) fail("PostgreSQL image was refused", 64);
  directory(
    values["database-dir"], ["keycloak.dump", "manifest.json", "synveda.dump"],
    "database backup",
  );
  directory(
    values["secrets-dir"], [
      "keycloak_convergence_admin_password", "manifest.json", "synveda_kms_key",
      "synveda_kms_key_ref",
    ],
    "recovery-secret backup",
  );
  const expected = {
    backupId: values["backup-id"],
    project: values.project,
    postgresImage: values["postgres-image"],
    tenantId: values["tenant-id"],
  };
  const databaseManifestPath = `${values["database-dir"]}/manifest.json`;
  const databaseManifest = validateDatabaseManifest(
    jsonManifest(databaseManifestPath, "database manifest"), expected,
  );
  for (const name of ["synveda", "keycloak"]) {
    const actual = hashFile(
      `${values["database-dir"]}/${name}.dump`, `${name} archive`, databaseManifest[name].bytes,
    );
    if (actual !== databaseManifest[name].sha256) fail(`${name} archive digest did not match`);
  }
  const keyManifest = validateSecretManifest(
    jsonManifest(`${values["secrets-dir"]}/manifest.json`, "recovery-secret manifest"), expected,
  );
  const databaseManifestRaw = readBounded(databaseManifestPath, "database manifest", 3, 8192);
  const databaseManifestHash = createHash("sha256").update(databaseManifestRaw).digest("hex");
  databaseManifestRaw.fill(0);
  if (databaseManifestHash !== keyManifest.database_manifest_sha256) {
    fail("database and recovery-secret manifests did not match");
  }
  for (const [manifestName, fileName] of [
    ["synveda_kms_key", "synveda_kms_key"],
    ["synveda_kms_key_ref", "synveda_kms_key_ref"],
    ["keycloak_convergence_admin_password", "keycloak_convergence_admin_password"],
  ]) {
    const actual = hashFile(
      `${values["secrets-dir"]}/${fileName}`, fileName, keyManifest[manifestName].bytes,
    );
    if (actual !== keyManifest[manifestName].sha256) {
      fail(`${fileName} digest did not match`);
    }
  }
  const key = readBounded(`${values["secrets-dir"]}/synveda_kms_key`, "KMS key", 65, 65);
  const keyRef = readBounded(
    `${values["secrets-dir"]}/synveda_kms_key_ref`, "KMS key reference", 7, 257,
  );
  const convergencePassword = readBounded(
    `${values["secrets-dir"]}/keycloak_convergence_admin_password`,
    "Keycloak convergence password", 65, 65,
  );
  const materialValid = /^[0-9a-f]{64}\n$/.test(key.toString("ascii")) &&
    /^local:[A-Za-z0-9._:-]{1,250}\n$/.test(keyRef.toString("utf8")) &&
    /^[0-9a-f]{64}\n$/.test(convergencePassword.toString("ascii"));
  key.fill(0);
  keyRef.fill(0);
  convergencePassword.fill(0);
  if (!materialValid) fail("recovery secret material was refused");
  process.stdout.write(`logical recovery set valid for ${values.project} as ${values["backup-id"]}\n`);
}

const { action, values } = argumentsFor(process.argv);
if (action === "snapshot-secrets") snapshotSecrets(values);
else verify(values);
