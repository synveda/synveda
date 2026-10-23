#!/usr/bin/env node
// Bind, plan and refuse PITR recovery sets without printing secrets.
// pgBackRest verifies files it finds; it does not report a missing WAL
// segment or fail its exit status for a corrupt one, so coverage is proven
// here from the repository listing before any restore writes target state.
import { existsSync, lstatSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { readPgbackrestConfig } from "./check-pgbackrest-config.mjs";

export const HEADER_FORMAT = "synveda-pitr-custody-v1";
export const MARK_FORMAT = "synveda-pitr-mark-v1";
export const CUSTODY_ENTRIES = Object.freeze([
  "keycloak_convergence_admin_password",
  "keycloak_database_password",
  "pgbackrest_cipher_pass",
  "postgres_owner_password",
  "synveda_gateway_database_url",
  "synveda_gateway_password",
  "synveda_kms_key",
  "synveda_kms_key_ref",
  "synveda_migrator_database_url",
  "synveda_migrator_password",
  "synveda_worker_database_url",
  "synveda_worker_password",
]);
const SECRET_SOURCES = CUSTODY_ENTRIES.filter((name) => name !== "pgbackrest_cipher_pass");
const PROJECT = /^synveda-(development|reference)(-acceptance-[a-z0-9](?:[a-z0-9-]{0,22}[a-z0-9])?)?$/;
const IDENTIFIER = /^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/;
const TENANT = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const IMAGE = /^[A-Za-z0-9_./:@+-]{1,512}$/;
const WAL = /^[0-9A-F]{24}$/;
const LSN = /^[0-9A-F]{1,8}\/[0-9A-F]{1,8}$/;
const LABEL = /^\d{8}-\d{6}F$/;
const UTC_SECOND = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/;
const KEK_REF = /^local:[A-Za-z0-9._:-]{1,250}$/;
const CUSTODIAN = /^[A-Za-z0-9][A-Za-z0-9._@-]{0,63}$/;
const SEGMENTS_PER_ID = 0x100;
const MAX_LISTING_BYTES = 64 * 1024 * 1024;
const MAX_SEGMENTS = 500_000;

export class Refused extends Error {
  constructor(message, status = 78) {
    super(message);
    this.status = status;
  }
}

const refuse = (message, status) => {
  throw new Refused(message, status);
};

function exactKeys(value, keys, label) {
  if (
    value === null || Array.isArray(value) || typeof value !== "object" ||
    JSON.stringify(Object.keys(value)) !== JSON.stringify(keys)
  ) refuse(`${label} shape was refused`);
}

function utcSecond(epochSeconds) {
  if (!Number.isSafeInteger(epochSeconds) || epochSeconds <= 0) refuse("backup timestamp was refused");
  return new Date(epochSeconds * 1000).toISOString().replace(/\.\d{3}Z$/, "Z");
}

export function lsnValue(lsn) {
  if (typeof lsn !== "string" || !LSN.test(lsn)) refuse("LSN was refused");
  const [high, low] = lsn.split("/");
  return (BigInt(`0x${high}`) << 32n) | BigInt(`0x${low}`);
}

/** The global segment number of a 16 MiB WAL file name and its timeline. */
export function segment(name) {
  if (typeof name !== "string" || !WAL.test(name)) refuse("WAL segment name was refused");
  const low = Number.parseInt(name.slice(16), 16);
  if (low >= SEGMENTS_PER_ID) refuse("WAL segment name was refused");
  return {
    timeline: name.slice(0, 8),
    number: Number.parseInt(name.slice(8, 16), 16) * SEGMENTS_PER_ID + low,
  };
}

export function walForLsn(timeline, lsn) {
  const value = lsnValue(lsn);
  const number = Number(value >> 24n);
  const high = Math.floor(number / SEGMENTS_PER_ID).toString(16).toUpperCase().padStart(8, "0");
  const low = (number % SEGMENTS_PER_ID).toString(16).toUpperCase().padStart(8, "0");
  return `${timeline}${high}${low}`;
}

/** Parses pgBackRest info JSON, keeping system-id exact beyond 2^53. */
export function parseInfo(text) {
  if (typeof text !== "string" || text.length > 4 * 1024 * 1024) refuse("repository information was refused");
  let value;
  try {
    // A wrong passphrase makes pgBackRest embed decrypted garbage, including
    // raw control bytes, in its status message. Those bytes are never echoed.
    const sanitized = text.replace(/[\u0000-\u001f\u007f]/g, " ");
    value = JSON.parse(sanitized, function reviver(key, parsed, context) {
      if (key === "system-id") {
        if (!context || !/^\d{10,24}$/.test(context.source ?? "")) refuse("repository system identifier was refused");
        return context.source;
      }
      return parsed;
    });
  } catch (error) {
    if (error instanceof Refused) throw error;
    refuse("repository information was not valid JSON");
  }
  if (!Array.isArray(value) || value.length !== 1 || value[0]?.name !== "synveda") {
    refuse("repository information did not contain exactly the synveda stanza");
  }
  return value[0];
}

/** Maps a stanza's repository status to one content-free diagnostic. */
export function requireRepositoryAvailable(stanza) {
  const repo = Array.isArray(stanza.repo) && stanza.repo.length === 1 ? stanza.repo[0] : undefined;
  const message = typeof repo?.status?.message === "string" ? repo.status.message : "";
  if (repo?.status?.code === 0 && stanza.status?.code === 0) {
    if (repo.cipher !== "aes-256-cbc" || stanza.cipher !== "aes-256-cbc") {
      refuse("repository is not encrypted with the required cipher");
    }
    return;
  }
  if (/\b40[13]\b/.test(message)) {
    refuse("repository credentials were refused; renew the expired or revoked object-store credential", 77);
  }
  if (/CryptoError/.test(message) && /certificate/.test(message)) {
    refuse("repository TLS certificate was not trusted; supply the private CA that issued it", 77);
  }
  if (/unable to load info file/.test(message)) {
    refuse("repository metadata did not open; the cipher passphrase is wrong or this is not the recorded repository", 77);
  }
  // pgBackRest stanza status codes: 1/3 missing stanza, 2 no valid backup.
  switch (repo?.status?.code === 0 ? stanza.status?.code : undefined) {
    case 1:
    case 3:
      refuse("repository path holds no synveda stanza; check the bucket and repository path");
      break;
    case 2:
      refuse("repository holds no valid backup for the synveda stanza");
      break;
    default:
      refuse("repository is unavailable or reported an error", 69);
  }
}

function selectedDatabase(stanza) {
  if (!Array.isArray(stanza.db) || stanza.db.length < 1) refuse("repository database history was refused");
  return stanza.db[stanza.db.length - 1];
}

function backupSummary(stanza, label) {
  const backups = Array.isArray(stanza.backup) ? stanza.backup : [];
  const candidates = backups.filter((entry) => entry?.type === "full" && entry.error === false);
  const backup = label === undefined
    ? candidates[candidates.length - 1]
    : candidates.find((entry) => entry.label === label);
  if (!backup) refuse(label === undefined ? "repository holds no valid full backup" : "recorded backup set is absent or invalid");
  if (!LABEL.test(backup.label ?? "")) refuse("backup label was refused");
  segment(backup.archive?.start);
  segment(backup.archive?.stop);
  lsnValue(backup.lsn?.start);
  lsnValue(backup.lsn?.stop);
  return {
    label: backup.label,
    start_wal: backup.archive.start,
    stop_wal: backup.archive.stop,
    start_lsn: backup.lsn.start,
    stop_lsn: backup.lsn.stop,
    started_at: utcSecond(backup.timestamp?.start),
    stopped_at: utcSecond(backup.timestamp?.stop),
  };
}

function archiveRange(stanza) {
  const archives = Array.isArray(stanza.archive) ? stanza.archive : [];
  if (archives.length !== 1 || !/^\d{2}-\d{1,4}$/.test(archives[0]?.id ?? "")) {
    refuse("repository must hold exactly one archive history for this generation");
  }
  segment(archives[0].min);
  segment(archives[0].max);
  return { id: archives[0].id, min: archives[0].min, max: archives[0].max };
}

/** Archived segment names from `repo-ls --recurse --output=json`. */
export function listedSegments(text) {
  if (typeof text !== "string" || text.length > MAX_LISTING_BYTES) refuse("WAL listing was refused");
  let listing;
  try {
    listing = JSON.parse(text);
  } catch {
    refuse("WAL listing was not valid JSON");
  }
  if (listing === null || typeof listing !== "object" || Array.isArray(listing)) refuse("WAL listing was refused");
  const names = new Set();
  for (const [path, entry] of Object.entries(listing)) {
    if (names.size > MAX_SEGMENTS) refuse("WAL listing exceeded its bound");
    const match = /^(?:[0-9A-F]{16}\/)?([0-9A-F]{24})-[0-9a-f]{40}(?:\.gz|\.zst|\.lz4|\.bz2)?$/.exec(path);
    if (!match || entry?.type !== "file") continue;
    if (!Number.isSafeInteger(entry.size) || entry.size <= 0) refuse("an archived WAL segment is empty");
    names.add(match[1]);
  }
  return names;
}

/** Requires every segment from `first` through `last` on one timeline. */
export function requireContiguous(names, first, last) {
  const start = segment(first);
  const end = segment(last);
  if (start.timeline !== end.timeline) refuse("recovery target is on another WAL timeline");
  if (end.number < start.number) refuse("recovery target precedes the base backup");
  if (end.number - start.number > MAX_SEGMENTS) refuse("recovery range exceeded its bound");
  for (let number = start.number; number <= end.number; number += 1) {
    const high = Math.floor(number / SEGMENTS_PER_ID).toString(16).toUpperCase().padStart(8, "0");
    const low = (number % SEGMENTS_PER_ID).toString(16).toUpperCase().padStart(8, "0");
    if (!names.has(`${start.timeline}${high}${low}`)) {
      refuse(`archived WAL is incomplete: segment ${start.timeline}${high}${low} is missing`);
    }
  }
}

/** Requires the text verify report to be clean; its exit status is not. */
export function requireVerified(text) {
  if (typeof text !== "string" || text.length > 1024 * 1024) refuse("repository verification report was refused");
  const lines = text.split("\n").map((line) => line.trim()).filter(Boolean);
  if (lines[0] !== "stanza: synveda" || lines[1] !== "status: ok") {
    refuse("repository verification found corrupt or unreadable backup material");
  }
  const counters = lines.filter((line) => line.startsWith("missing:"));
  if (counters.length < 2 || counters.some((line) => line !== "missing: 0, checksum invalid: 0, size invalid: 0, other: 0")) {
    refuse("repository verification found corrupt or unreadable backup material");
  }
  if (!lines.some((line) => /^backup: \d{8}-\d{6}F, status: valid,/.test(line))) {
    refuse("repository verification did not validate the backup set");
  }
}

function repositoryLocator(options, caPresent) {
  return {
    type: "s3",
    endpoint: options.get("repo1-s3-endpoint"),
    bucket: options.get("repo1-s3-bucket"),
    region: options.get("repo1-s3-region"),
    path: options.get("repo1-path"),
    uri_style: options.get("repo1-s3-uri-style") ?? "host",
    port: options.has("repo1-storage-port") ? Number(options.get("repo1-storage-port")) : 443,
    private_ca: caPresent,
    cipher: "aes-256-cbc",
  };
}

/** Parses one `key=value` metadata line per field from the source cluster. */
export function parseMetadata(text) {
  const fields = new Map();
  for (const line of String(text).split("\n")) {
    if (line === "") continue;
    const equal = line.indexOf("=");
    if (equal <= 0 || fields.has(line.slice(0, equal))) refuse("cluster metadata was refused");
    fields.set(line.slice(0, equal), line.slice(equal + 1));
  }
  const number = (name, min, max) => {
    const value = Number(fields.get(name));
    if (!/^\d{1,10}$/.test(fields.get(name) ?? "") || value < min || value > max) refuse(`cluster ${name} was refused`);
    return value;
  };
  const kekRefs = (fields.get("kek_refs") ?? "").split(",").filter(Boolean);
  if (kekRefs.length < 1 || kekRefs.length > 16 || kekRefs.some((ref) => !KEK_REF.test(ref))) {
    refuse("cluster KEK references were refused");
  }
  if (fields.get("wal_segment_size") !== "16MB") refuse("only 16 MiB WAL segments are supported");
  if (fields.get("databases") !== "keycloak,synveda") refuse("cluster must hold the synveda and keycloak databases");
  if (!/^\d+\.\d+\.\d+$/.test(fields.get("pgvector") ?? "")) refuse("cluster pgvector version was refused");
  if (!/^[0-9a-f]{64}$/.test(fields.get("migration_ledger_sha256") ?? "")) refuse("cluster migration ledger was refused");
  if (!/^\d{10,24}$/.test(fields.get("system_identifier") ?? "")) refuse("cluster system identifier was refused");
  if (fields.size !== 10) refuse("cluster metadata was refused");
  return {
    system_identifier: fields.get("system_identifier"),
    server_version_num: number("server_version_num", 170000, 179999),
    wal_segment_size: "16MB",
    pgvector: fields.get("pgvector"),
    schema_head: number("schema_head", 1, 9999),
    migration_ledger_sha256: fields.get("migration_ledger_sha256"),
    databases: ["keycloak", "synveda"],
    kek_refs: kekRefs,
    deployment_key_generations: number("deployment_key_generations", 0, 1024),
    tenant_key_generations: number("tenant_key_generations", 1, 10_000_000),
  };
}

function identity(values) {
  if (!PROJECT.test(values.project ?? "")) refuse("source project was refused", 64);
  if (!IDENTIFIER.test(values.backupId ?? "") || values.backupId.includes("--")) refuse("backup id was refused", 64);
  if (!TENANT.test(values.tenantId ?? "")) refuse("tenant UUIDv7 was refused", 64);
  for (const name of ["postgresImage", "productImage"]) {
    if (!IMAGE.test(values[name] ?? "")) refuse("image reference was refused", 64);
  }
}

/** Builds the identifiers-only custody header for the newest full backup. */
export function buildHeader({ info, metadata, options, caPresent, values, createdAt }) {
  identity(values);
  if (!CUSTODIAN.test(values.custodian ?? "")) refuse("custodian identifier was refused", 64);
  if (!UTC_SECOND.test(createdAt)) refuse("creation time was refused");
  requireRepositoryAvailable(info);
  const database = selectedDatabase(info);
  if (database["system-id"] !== metadata.system_identifier) refuse("repository and cluster system identifiers differ");
  if (database.version !== String(Math.floor(metadata.server_version_num / 10000))) {
    refuse("repository and cluster PostgreSQL majors differ");
  }
  const backup = backupSummary(info);
  const archive = archiveRange(info);
  return {
    format: HEADER_FORMAT,
    backup_id: values.backupId,
    source_project: values.project,
    tenant_id: values.tenantId,
    product_image: values.productImage,
    postgres_image: values.postgresImage,
    custodian: values.custodian,
    created_at: createdAt,
    repository: repositoryLocator(options, caPresent),
    stanza: "synveda",
    backup,
    archive_id: archive.id,
    database: metadata,
    entries: [...CUSTODY_ENTRIES],
  };
}

export function serialize(value) {
  return `${JSON.stringify(value)}\n`;
}

const HEADER_KEYS = [
  "format", "backup_id", "source_project", "tenant_id", "product_image", "postgres_image",
  "custodian", "created_at", "repository", "stanza", "backup", "archive_id", "database", "entries",
];

/** Validates an opened header against the restore request. */
export function validateHeader(header, expected) {
  exactKeys(header, HEADER_KEYS, "custody header");
  if (header.format !== HEADER_FORMAT || header.stanza !== "synveda") refuse("custody header format was refused");
  identity({
    project: header.source_project, backupId: header.backup_id, tenantId: header.tenant_id,
    postgresImage: header.postgres_image, productImage: header.product_image,
  });
  if (header.source_project !== expected.project) refuse("custody set belongs to another source project");
  if (header.backup_id !== expected.backupId) refuse("custody set belongs to another backup id");
  if (header.tenant_id !== expected.tenantId) refuse("custody set belongs to another bootstrap tenant");
  if (header.postgres_image !== expected.postgresImage) {
    refuse("selected PostgreSQL image differs from the image that took this backup");
  }
  if (header.product_image !== expected.productImage) {
    refuse("selected product image differs from the image bound to this backup; restore first, then upgrade");
  }
  if (JSON.stringify(header.entries) !== JSON.stringify(CUSTODY_ENTRIES)) refuse("custody entry set was refused");
  if (!LABEL.test(header.backup?.label ?? "")) refuse("custody backup label was refused");
  return header;
}

const VALUE_GRAMMARS = {
  synveda_kms_key: /^[0-9a-f]{64}\n$/,
  synveda_kms_key_ref: /^local:[A-Za-z0-9._:-]{1,250}\n$/,
  pgbackrest_cipher_pass: /^[\x21-\x7e]{32,1024}\n$/,
};
const URL_ROLES = {
  synveda_migrator_database_url: ["synveda_migrator", "synveda_migrator_password"],
  synveda_gateway_database_url: ["synveda_gateway", "synveda_gateway_password"],
  synveda_worker_database_url: ["synveda_worker", "synveda_worker_password"],
};

function privateEntry(directory, name) {
  const path = `${directory}/${name}`;
  let stat;
  try {
    stat = lstatSync(path);
  } catch {
    refuse("custody entry was unavailable");
  }
  if (!stat.isFile() || stat.isSymbolicLink() || (stat.mode & 0o077) !== 0 || stat.nlink !== 1 || stat.size > 4096) {
    refuse("custody entry metadata was refused");
  }
  return readFileSync(path, "utf8");
}

/** Proves the opened entries have exactly the recorded names and grammars. */
export function validateOpenedEntries(directory, header) {
  const present = readdirSync(directory).filter((name) => name !== "header.json").sort();
  if (JSON.stringify(present) !== JSON.stringify(CUSTODY_ENTRIES)) refuse("opened custody inventory was refused");
  const values = Object.fromEntries(CUSTODY_ENTRIES.map((name) => [name, privateEntry(directory, name)]));
  for (const name of CUSTODY_ENTRIES) {
    const grammar = VALUE_GRAMMARS[name] ?? (URL_ROLES[name] ? null : /^[0-9a-f]{64}\n$/);
    if (grammar && !grammar.test(values[name])) refuse(`custody entry ${name} was refused`);
  }
  for (const [name, [role, password]] of Object.entries(URL_ROLES)) {
    const expected = `postgres://${role}:${values[password].trim()}@postgres:5432/synveda\n`;
    if (values[name] !== expected) refuse(`custody entry ${name} does not match its role credential`);
  }
  if (!header.database.kek_refs.includes(values.synveda_kms_key_ref.trim())) {
    refuse("custody KEK reference is not one the backed-up database names");
  }
  if (header.database.kek_refs.length !== 1) {
    refuse("the backed-up database names more than one KEK reference; complete KEK replacement before this generation");
  }
  return values;
}

/** Proves a target repository configuration names the recorded repository. */
export function requireSameRepository(header, options, caPresent, cipherPass) {
  // Trust is local to each host: a restore may reach the same repository
  // through a different verified CA path.
  const { private_ca: _targetCa, ...locator } = repositoryLocator(options, caPresent);
  const { private_ca: _sourceCa, ...recorded } = header.repository;
  if (JSON.stringify(locator) !== JSON.stringify(recorded)) {
    refuse("target repository configuration names a different repository than the custody set");
  }
  if (options.get("repo1-cipher-pass") !== cipherPass.trim()) {
    refuse("target repository passphrase differs from the escrowed passphrase");
  }
}

export function validateMark(mark, header) {
  exactKeys(mark, ["format", "source_project", "name", "lsn", "wal", "created_at"], "restore mark");
  if (mark.format !== MARK_FORMAT || mark.source_project !== header.source_project) refuse("restore mark belongs to another source");
  if (!IDENTIFIER.test(mark.name) || !UTC_SECOND.test(mark.created_at)) refuse("restore mark was refused");
  if (walForLsn(mark.wal.slice(0, 8), mark.lsn) !== mark.wal) refuse("restore mark WAL position was refused");
  return mark;
}

/** Decides one restore target, or refuses before any target state changes. */
export function plan({ header, info, listing, verifyReport, target, mark }) {
  requireRepositoryAvailable(info);
  const database = selectedDatabase(info);
  if (database["system-id"] !== header.database.system_identifier) refuse("repository belongs to another PostgreSQL cluster");
  if (database.version !== String(Math.floor(header.database.server_version_num / 10000))) {
    refuse("repository PostgreSQL major differs from the custody set");
  }
  const backup = backupSummary(info, header.backup.label);
  if (JSON.stringify(backup) !== JSON.stringify(header.backup)) refuse("recorded backup set changed in the repository");
  const archive = archiveRange(info);
  if (archive.id !== header.archive_id) refuse("repository archive history differs from the custody set");
  requireVerified(verifyReport);
  const names = listedSegments(listing);
  if (target === "latest") {
    requireContiguous(names, backup.start_wal, archive.max);
    return { set: backup.label, type: "default", target: "", through_wal: archive.max };
  }
  const name = /^name:([a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?)$/.exec(target ?? "")?.[1];
  if (!name || !mark || mark.name !== name) refuse("recovery target must be latest or name:<recorded mark>", 64);
  validateMark(mark, header);
  if (lsnValue(mark.lsn) < lsnValue(backup.stop_lsn)) refuse("restore mark precedes the end of the base backup");
  requireContiguous(names, backup.start_wal, mark.wal);
  return { set: backup.label, type: "name", target: name, through_wal: mark.wal };
}

// ── Shell adapters ─────────────────────────────────────────────────────────

function argumentsFor(argv, expected) {
  const values = {};
  for (let index = 0; index < argv.length; index += 2) {
    const name = argv[index];
    const value = argv[index + 1];
    if (!name?.startsWith("--") || value === undefined || value === "") refuse("invalid arguments", 64);
    const key = name.slice(2);
    if (values[key] !== undefined || !expected.includes(key)) refuse("invalid arguments", 64);
    values[key] = value;
  }
  if (expected.some((key) => values[key] === undefined)) refuse("invalid arguments", 64);
  return values;
}

function readBounded(path, limit) {
  let stat;
  try {
    stat = lstatSync(path);
  } catch {
    refuse("recovery input was unavailable");
  }
  if (!stat.isFile() || stat.isSymbolicLink() || stat.size > limit) refuse("recovery input was refused");
  return readFileSync(path, "utf8");
}

function writeNew(path, text) {
  try {
    writeFileSync(path, text, { flag: "wx", mode: 0o600 });
  } catch {
    refuse("recovery output could not be created", 73);
  }
}

function readJson(path, limit = 64 * 1024) {
  const text = readBounded(path, limit);
  let value;
  try {
    value = JSON.parse(text);
  } catch {
    refuse("recovery record was not valid JSON");
  }
  if (text !== serialize(value)) refuse("recovery record was not canonical JSON");
  return value;
}

function stageSecrets(args) {
  const values = argumentsFor(args, ["secret-dir", "config", "ca", "output-dir"]);
  const options = readPgbackrestConfig(values.config, values.ca === "true");
  if (readdirSync(values["output-dir"]).length !== 0) refuse("custody staging directory must be empty", 73);
  for (const name of SECRET_SOURCES) {
    writeNew(`${values["output-dir"]}/${name}`, privateEntry(values["secret-dir"], name));
  }
  writeNew(`${values["output-dir"]}/pgbackrest_cipher_pass`, `${options.get("repo1-cipher-pass")}\n`);
  console.log("PITR custody entries staged");
}

function binding(args) {
  const values = argumentsFor(args, [
    "info", "metadata", "config", "ca", "project", "backup-id", "tenant-id",
    "postgres-image", "product-image", "custodian", "created-at", "output",
  ]);
  const caPresent = values.ca === "true";
  const header = buildHeader({
    info: parseInfo(readBounded(values.info, 4 * 1024 * 1024)),
    metadata: parseMetadata(readBounded(values.metadata, 8192)),
    options: readPgbackrestConfig(values.config, caPresent),
    caPresent,
    createdAt: values["created-at"],
    values: {
      project: values.project, backupId: values["backup-id"], tenantId: values["tenant-id"],
      postgresImage: values["postgres-image"], productImage: values["product-image"],
      custodian: values.custodian,
    },
  });
  writeNew(values.output, serialize(header));
  console.log(`PITR custody header bound to backup ${header.backup.label}`);
}

function markRecord(args) {
  const values = argumentsFor(args, ["project", "name", "lsn", "wal", "created-at", "output"]);
  if (!PROJECT.test(values.project) || !IDENTIFIER.test(values.name)) refuse("restore mark was refused", 64);
  const mark = {
    format: MARK_FORMAT, source_project: values.project, name: values.name,
    lsn: values.lsn, wal: values.wal, created_at: values["created-at"],
  };
  validateMark(mark, { source_project: values.project });
  writeNew(values.output, serialize(mark));
  console.log(`restore mark ${values.name} recorded at ${values.lsn}`);
}

function restorePlan(args) {
  const values = argumentsFor(args, [
    "opened-dir", "info", "verify", "listing", "config", "ca", "target", "marks-dir",
    "project", "backup-id", "tenant-id", "postgres-image", "product-image",
  ]);
  const header = validateHeader(readJson(`${values["opened-dir"]}/header.json`, 16 * 1024), {
    project: values.project, backupId: values["backup-id"], tenantId: values["tenant-id"],
    postgresImage: values["postgres-image"], productImage: values["product-image"],
  });
  const secrets = validateOpenedEntries(values["opened-dir"], header);
  const caPresent = values.ca === "true";
  requireSameRepository(header, readPgbackrestConfig(values.config, caPresent), caPresent, secrets.pgbackrest_cipher_pass);
  const markName = /^name:(.+)$/.exec(values.target)?.[1];
  if (markName && IDENTIFIER.test(markName) && !existsSync(`${values["marks-dir"]}/${markName}.json`)) {
    refuse(`restore mark ${markName} is not recorded; list the marks root and choose a recorded mark`, 64);
  }
  const mark = markName && IDENTIFIER.test(markName)
    ? readJson(`${values["marks-dir"]}/${markName}.json`)
    : undefined;
  const decided = plan({
    header,
    info: parseInfo(readBounded(values.info, 4 * 1024 * 1024)),
    listing: readBounded(values.listing, MAX_LISTING_BYTES),
    verifyReport: readBounded(values.verify, 1024 * 1024),
    target: values.target,
    mark,
  });
  process.stdout.write(`${decided.set}|${decided.type}|${decided.target}|${decided.through_wal}\n`);
}

export const RESTORED_FORMAT = "synveda-pitr-restored-v1";

function sameStorage(left, right) {
  return left.endpoint === right.endpoint && left.bucket === right.bucket && left.path === right.path;
}

/** Refuses archiving from a restored project into the repository it came from. */
export function requireOwnRepository(marker, options) {
  exactKeys(marker, ["format", "source_project", "backup_id", "repository", "restored_at"], "restore marker");
  if (marker.format !== RESTORED_FORMAT) refuse("restore marker was refused");
  if (sameStorage(marker.repository, repositoryLocator(options, false))) {
    refuse(`this project was restored from ${marker.source_project}; give it its own repository path before archiving`);
  }
}

function restoredMarker(args) {
  const values = argumentsFor(args, ["opened-dir", "restored-at", "output"]);
  const header = readJson(`${values["opened-dir"]}/header.json`, 16 * 1024);
  if (!UTC_SECOND.test(values["restored-at"])) refuse("restore time was refused");
  const { endpoint, bucket, path } = header.repository;
  writeNew(values.output, serialize({
    format: RESTORED_FORMAT,
    source_project: header.source_project,
    backup_id: header.backup_id,
    repository: { endpoint, bucket, path },
    restored_at: values["restored-at"],
  }));
}

function archiveGuard(args) {
  const values = argumentsFor(args, ["config", "ca", "marker"]);
  requireOwnRepository(readJson(values.marker), readPgbackrestConfig(values.config, values.ca === "true"));
}

function repositoryCheck(args) {
  const values = argumentsFor(args, ["info"]);
  const stanza = parseInfo(readBounded(values.info, 4 * 1024 * 1024));
  requireRepositoryAvailable(stanza);
  process.stdout.write(`${archiveRange(stanza).id}\n`);
}

function main(argv) {
  const [action, ...args] = argv;
  const actions = {
    "stage-secrets": stageSecrets,
    binding,
    mark: markRecord,
    plan: restorePlan,
    "repository-check": repositoryCheck,
    "restored-marker": restoredMarker,
    "archive-guard": archiveGuard,
  };
  if (!Object.hasOwn(actions, action)) refuse("invalid action", 64);
  actions[action](args);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    // Parser and filesystem errors can carry private bytes or paths.
    const known = error instanceof Refused;
    process.stderr.write(`pitr-recovery: ${known ? error.message : "recovery input was unavailable or unsafe"}\n`);
    process.exitCode = known ? error.status : 78;
  }
}
