#!/usr/bin/env node
import { createHash, randomUUID } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  closeSync,
  constants,
  fchmodSync,
  fchownSync,
  fstatSync,
  ftruncateSync,
  fsyncSync,
  lstatSync,
  linkSync,
  openSync,
  readSync,
  realpathSync,
  statSync,
  unlinkSync,
  writeSync,
} from "node:fs";
import { basename, dirname, join } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import { TextDecoder } from "node:util";

const HOSTS_PATH = "/etc/hosts";
const TARGET_LIMIT = 1024 * 1024;
const TARGET_MODE = 0o644;
const RECORD_LIMIT = 2 * 1024 * 1024;
const STATE_NAME = ".synveda-hosts-state-v1.json";
const BACKUP_NAME = ".synveda-hosts-backup-v1.json";
const LOCK_NAME = ".synveda-hosts-lock-v1";
const STAGE_NAMES = Object.freeze({
  backup: ".synveda-hosts-backup-stage-v1",
  state: ".synveda-hosts-state-stage-v1",
});
const UTF8 = new TextDecoder("utf-8", { fatal: true });
const { O_CREAT, O_EXCL, O_NOFOLLOW, O_RDONLY, O_RDWR, O_WRONLY } = constants;

export class HostsFileError extends Error {
  constructor(message, status = 78, uncertain = false) {
    super(message);
    this.name = "HostsFileError";
    this.status = status;
    this.uncertain = uncertain;
  }
}

function refuse(message, status = 78, uncertain = false) {
  throw new HostsFileError(message, status, uncertain);
}

function exactKeys(value, keys) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return false;
  return JSON.stringify(Object.keys(value).sort()) === JSON.stringify([...keys].sort());
}

function validNonce(value) {
  return (
    typeof value === "string" &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value)
  );
}

function validHost(host) {
  if (typeof host !== "string" || host.length === 0 || host.length > 253) return false;
  if (!/^[a-z0-9.-]+$/.test(host) || !host.endsWith(".test")) return false;
  if (host.startsWith(".") || host.endsWith(".") || host.includes("..")) return false;
  if (!/[a-z]/.test(host) || !host.includes(".")) return false;
  return host.split(".").every(
    (label) => label.length > 0 && label.length <= 63 && !label.startsWith("-") && !label.endsWith("-"),
  );
}

function validProject(project) {
  if (typeof project !== "string" || project.length > 63) return false;
  if (project === "synveda-development") return true;
  const prefix = "synveda-development-acceptance-";
  if (!project.startsWith(prefix)) return false;
  const suffix = project.slice(prefix.length);
  return (
    suffix.length > 0 &&
    suffix.length <= 24 &&
    /^[a-z0-9][a-z0-9-]*$/.test(suffix) &&
    !suffix.endsWith("-")
  );
}

export function validateSelection(value) {
  if (!exactKeys(value, ["project", "oidc", "appHost", "authHost"])) {
    refuse("selection was refused", 64);
  }
  const selection = {
    project: value.project,
    oidc: value.oidc,
    appHost: value.appHost,
    authHost: value.authHost ?? null,
  };
  if (
    !validProject(selection.project) ||
    !new Set(["bundled", "external"]).has(selection.oidc) ||
    !validHost(selection.appHost) ||
    (selection.oidc === "bundled") !== (selection.authHost !== null) ||
    (selection.authHost !== null && !validHost(selection.authHost)) ||
    selection.appHost === selection.authHost
  ) {
    refuse("selection was refused", 64);
  }
  return Object.freeze(selection);
}

function selectionEquals(left, right) {
  return (
    left.project === right.project &&
    left.oidc === right.oidc &&
    left.appHost === right.appHost &&
    left.authHost === right.authHost
  );
}

export function expectedBlock(selectionValue) {
  const selection = validateSelection(selectionValue);
  const hosts = [selection.appHost];
  if (selection.authHost !== null) hosts.push(selection.authHost);
  return Buffer.from(
    `# BEGIN SYNVEDA ${selection.project}\n127.0.0.1 ${hosts.join(" ")}\n# END SYNVEDA ${selection.project}\n`,
    "utf8",
  );
}

export function expectedConfirmation(action, selectionValue) {
  const selection = validateSelection(selectionValue);
  if (!new Set(["install", "remove"]).has(action)) refuse("action was refused", 64);
  return `${action}:127.0.0.1:${selection.project}:${selection.appHost}:${selection.authHost ?? "-"}`;
}

function decodeHosts(bytes) {
  if (!Buffer.isBuffer(bytes) || bytes.length > TARGET_LIMIT) refuse("hosts file was refused");
  let text;
  try {
    text = UTF8.decode(bytes);
  } catch {
    refuse("hosts file encoding was refused");
  }
  if (text.includes("\0") || text.includes("\r")) refuse("hosts file encoding was refused");
  if (text.length > 0 && !text.endsWith("\n")) refuse("hosts file termination was refused");
  return text;
}

function escapeRegex(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function containsAlias(text, host) {
  const pattern = new RegExp(
    `(?:^|[^a-z0-9.-])${escapeRegex(host)}\\.?(?=$|[^a-z0-9.-])`,
    "i",
  );
  return pattern.test(text);
}

function markerPresent(text) {
  return text.split("\n").some((line) => /#\s*(?:BEGIN|END)\s+SYNVEDA(?:\s|$)/i.test(line));
}

export function classifyHostsBytes(bytes, selectionValue) {
  const selection = validateSelection(selectionValue);
  const text = decodeHosts(bytes);
  const block = expectedBlock(selection).toString("utf8");
  const first = text.indexOf(block);
  const second = first < 0 ? -1 : text.indexOf(block, first + block.length);
  const aliases = [selection.appHost, ...(selection.authHost === null ? [] : [selection.authHost])];

  if (first < 0) {
    if (markerPresent(text)) refuse("managed marker collision was refused");
    if (aliases.some((host) => containsAlias(text, host))) {
      refuse("unmanaged hostname collision was refused");
    }
    return Object.freeze({ state: "absent" });
  }
  if (
    second >= 0 ||
    first + block.length !== text.length ||
    (first > 0 && text[first - 1] !== "\n")
  ) {
    refuse("managed block collision was refused");
  }
  const unrelated = text.slice(0, first);
  if (markerPresent(unrelated)) refuse("managed marker collision was refused");
  if (aliases.some((host) => containsAlias(unrelated, host))) {
    refuse("unmanaged hostname collision was refused");
  }
  return Object.freeze({ state: "exact", offset: first });
}

function statIdentity(stat) {
  return {
    dev: stat.dev,
    ino: stat.ino,
    nlink: stat.nlink,
    uid: stat.uid,
    gid: stat.gid,
    mode: stat.mode,
    size: stat.size,
    mtimeNs: stat.mtimeNs,
    ctimeNs: stat.ctimeNs,
  };
}

function sameIdentity(left, right) {
  return Object.keys(left).every((key) => left[key] === right[key]);
}

function readDescriptor(fd, limit) {
  const chunks = [];
  let total = 0;
  for (;;) {
    const chunk = Buffer.allocUnsafe(Math.min(64 * 1024, limit + 1 - total));
    const count = readSync(fd, chunk, 0, chunk.length, null);
    if (count === 0) break;
    total += count;
    if (total > limit) refuse("file size was refused");
    chunks.push(chunk.subarray(0, count));
  }
  return Buffer.concat(chunks, total);
}

function lstatIfPresent(path) {
  try {
    return lstatSync(path, { bigint: true });
  } catch (error) {
    if (error?.code === "ENOENT") return undefined;
    refuse("file metadata was unavailable", 69);
  }
}

function modeOf(stat) {
  return Number(stat.mode & 0o7777n);
}

function permissionTriplet(mode, shift) {
  const value = (mode >> shift) & 0o7;
  return `${value & 0o4 ? "r" : "-"}${value & 0o2 ? "w" : "-"}${value & 0o1 ? "x" : "-"}`;
}

export function linuxAclOutputIsBase(output, mode) {
  if (typeof output !== "string" || !Number.isInteger(mode)) return false;
  const lines = output.endsWith("\n") ? output.slice(0, -1).split("\n") : output.split("\n");
  if (lines.some((line) => line.length === 0 || line.startsWith("#"))) return false;
  const expected = new Set([
    `user::${permissionTriplet(mode, 6)}`,
    `group::${permissionTriplet(mode, 3)}`,
    `other::${permissionTriplet(mode, 0)}`,
  ]);
  return lines.length === expected.size && lines.every((line) => expected.delete(line)) && expected.size === 0;
}

function trustedLinuxAclTool() {
  for (const candidate of ["/usr/bin/getfacl", "/bin/getfacl"]) {
    let resolved;
    try {
      resolved = realpathSync(candidate);
    } catch {
      continue;
    }
    if (resolved !== "/usr/bin/getfacl" && resolved !== "/bin/getfacl") continue;
    const rootStat = statSync("/", { bigint: true });
    if (!rootStat.isDirectory() || Number(rootStat.uid) !== 0 || (modeOf(rootStat) & 0o022) !== 0) {
      continue;
    }
    const components = resolved.split("/").filter(Boolean);
    let current = "";
    let trusted = true;
    for (let index = 0; index < components.length; index += 1) {
      current += `/${components[index]}`;
      const stat = statSync(current, { bigint: true });
      if (
        Number(stat.uid) !== 0 ||
        (modeOf(stat) & 0o022) !== 0 ||
        (index === components.length - 1 ? !stat.isFile() : !stat.isDirectory())
      ) {
        trusted = false;
        break;
      }
    }
    if (trusted && (modeOf(statSync(resolved, { bigint: true })) & 0o111) !== 0) return resolved;
  }
  refuse("Linux ACL inspector was unavailable", 69);
}

function runAclInspector(command, arguments_, label) {
  const result = spawnSync(command, arguments_, {
    encoding: "utf8",
    env: { LC_ALL: "C", PATH: "/usr/bin:/bin" },
    timeout: 5_000,
    maxBuffer: 64 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
  });
  if (result.error !== undefined || result.signal !== null || result.status !== 0) {
    refuse(`${label} ACL inspection was unavailable`, 69);
  }
  return result.stdout;
}

function assertNoAccessAcl(path, label, expectedIdentity) {
  const before = lstatIfPresent(path);
  if (before === undefined || before.isSymbolicLink()) refuse(`${label} was unavailable`, 69);
  const beforeIdentity = statIdentity(before);
  if (expectedIdentity !== undefined && !sameIdentity(beforeIdentity, expectedIdentity)) {
    refuse(`${label} changed before ACL inspection`, 75);
  }
  if (process.platform === "darwin") {
    const output = runAclInspector("/bin/ls", ["-lde", path], label);
    const lines = output.endsWith("\n") ? output.slice(0, -1).split("\n") : output.split("\n");
    const token = lines[0]?.split(/\s+/, 1)[0];
    if (
      lines.length !== 1 ||
      typeof token !== "string" ||
      !/^[bcdlps-][rwxStTs-]{9}@?$/.test(token)
    ) {
      refuse(`${label} access ACL was refused`);
    }
  } else if (process.platform === "linux") {
    const output = runAclInspector(
      trustedLinuxAclTool(),
      ["-c", "-p", "-n", "--", path],
      label,
    );
    if (!linuxAclOutputIsBase(output, modeOf(before))) refuse(`${label} access ACL was refused`);
  } else {
    refuse("ACL inspection platform was refused", 69);
  }
  const after = lstatIfPresent(path);
  if (after === undefined || !sameIdentity(beforeIdentity, statIdentity(after))) {
    refuse(`${label} changed during ACL inspection`, 75);
  }
}

function validateRegular(stat, expectedUid, expectedMode, label) {
  if (!stat.isFile() || stat.nlink !== 1n || Number(stat.uid) !== expectedUid) {
    refuse(`${label} authority was refused`);
  }
  const mode = modeOf(stat);
  if (expectedMode === undefined) {
    if ((mode & 0o7022) !== 0) refuse(`${label} permissions were refused`);
  } else if (mode !== expectedMode) {
    refuse(`${label} permissions were refused`);
  }
}

function readSnapshot(path, limit, expectedUid, expectedMode, label) {
  const before = lstatIfPresent(path);
  if (before === undefined || before.isSymbolicLink()) refuse(`${label} was unavailable`, 69);
  validateRegular(before, expectedUid, expectedMode, label);
  let fd;
  try {
    fd = openSync(path, O_RDONLY | O_NOFOLLOW);
    const opened = fstatSync(fd, { bigint: true });
    validateRegular(opened, expectedUid, expectedMode, label);
    if (before.dev !== opened.dev || before.ino !== opened.ino) {
      refuse(`${label} changed during inspection`, 75);
    }
    if (opened.size > BigInt(limit)) refuse(`${label} size was refused`);
    const bytes = readDescriptor(fd, limit);
    const after = fstatSync(fd, { bigint: true });
    if (!sameIdentity(statIdentity(opened), statIdentity(after)) || after.size !== BigInt(bytes.length)) {
      refuse(`${label} changed during inspection`, 75);
    }
    return Object.freeze({ bytes, stat: statIdentity(after) });
  } catch (error) {
    if (error instanceof HostsFileError) throw error;
    refuse(`${label} was unavailable`, 69);
  } finally {
    if (fd !== undefined) {
      const closing = fd;
      fd = undefined;
      closeDescriptor(closing, label);
    }
  }
}

function resolveManagedPaths(targetPath, expectedUid, aclInspector) {
  let parent;
  try {
    parent = realpathSync(dirname(targetPath));
  } catch {
    refuse("hosts parent directory was unavailable", 69);
  }
  const parentStat = statSync(parent, { bigint: true });
  const parentMode = modeOf(parentStat);
  if (
    !parentStat.isDirectory() ||
    Number(parentStat.uid) !== expectedUid ||
    (parentMode & 0o0022) !== 0
  ) {
    refuse("hosts parent directory authority was refused");
  }
  aclInspector(parent, "hosts parent directory");
  return Object.freeze({
    parent,
    target: join(parent, basename(targetPath)),
    state: join(parent, STATE_NAME),
    backup: join(parent, BACKUP_NAME),
    lock: join(parent, LOCK_NAME),
    assertNoAcl: aclInspector,
  });
}

function fsyncDirectory(parent) {
  let fd;
  try {
    fd = openSync(parent, O_RDONLY);
    fsyncSync(fd);
  } catch {
    refuse("hosts parent directory sync failed", 70, true);
  } finally {
    if (fd !== undefined) {
      const closing = fd;
      fd = undefined;
      closeDescriptor(closing, "hosts parent directory", true);
    }
  }
}

function writeAll(fd, bytes) {
  let offset = 0;
  while (offset < bytes.length) {
    const count = writeSync(fd, bytes, offset, bytes.length - offset, null);
    if (count <= 0) refuse("staged write failed", 70);
    offset += count;
  }
}

function closeDescriptor(fd, label, uncertain = false, hook) {
  try {
    hook?.();
    closeSync(fd);
  } catch {
    refuse(`${label} close failed`, 70, uncertain);
  }
}

function unlinkIfSameInode(path, identity) {
  if (identity === undefined) return false;
  const current = lstatIfPresent(path);
  if (
    current !== undefined &&
    current.isFile() &&
    current.dev === identity.dev &&
    current.ino === identity.ino
  ) {
    unlinkSync(path);
    return true;
  }
  return false;
}

function stagePath(paths, kind, nonce) {
  const name = STAGE_NAMES[kind];
  if (name === undefined || !validNonce(nonce)) refuse("stage identity was refused", 70, true);
  return join(paths.parent, `${name}-${nonce}`);
}

function stageFile(paths, bytes, mode, uid, gid, label, kind, nonce, hooks) {
  const stage = stagePath(paths, kind, nonce);
  let fd;
  let created = false;
  let createdIdentity;
  try {
    paths.assertNoAcl(paths.parent, "hosts parent directory");
    fd = openSync(stage, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0o600);
    created = true;
    createdIdentity = statIdentity(fstatSync(fd, { bigint: true }));
    paths.assertNoAcl(stage, `${label} stage`, createdIdentity);
    writeAll(fd, bytes);
    fchownSync(fd, uid, gid);
    fchmodSync(fd, mode);
    fsyncSync(fd);
    const closing = fd;
    fd = undefined;
    closeDescriptor(closing, `${label} stage`, true, hooks.beforeStageClose);
    const snapshot = readSnapshot(stage, Math.max(bytes.length, 1), uid, mode, `${label} stage`);
    if (!snapshot.bytes.equals(bytes)) refuse(`${label} stage verification failed`, 70);
    return { path: stage, snapshot };
  } catch (error) {
    let failure = error;
    if (fd !== undefined) {
      const closing = fd;
      fd = undefined;
      try {
        closeDescriptor(closing, `${label} stage`, true);
      } catch (closeError) {
        failure = closeError;
      }
    }
    if (created) {
      try {
        if (!unlinkIfSameInode(stage, createdIdentity)) {
          refuse(`${label} stage cleanup failed`, 70, true);
        }
        fsyncDirectory(paths.parent);
      } catch {
        refuse(`${label} stage cleanup failed`, 70, true);
      }
    }
    if (failure instanceof HostsFileError) throw failure;
    refuse(`${label} staging failed`, 70);
  }
}

function publishExclusive(paths, path, bytes, mode, uid, gid, label, kind, lock, hooks = {}) {
  if (lstatIfPresent(path) !== undefined) refuse(`${label} already exists`, 75);
  assertLock(paths, lock, uid);
  const stage = stageFile(
    paths,
    bytes,
    mode,
    uid,
    gid,
    label,
    kind,
    parseLock(lock.bytes).nonce,
    hooks,
  );
  let stageExists = true;
  try {
    hooks.afterStagePrepared?.(kind);
    assertLock(paths, lock, uid);
    linkSync(stage.path, path);
    hooks.afterStageLinked?.(kind);
    unlinkSync(stage.path);
    stageExists = false;
    fsyncDirectory(paths.parent);
    const published = readSnapshot(path, Math.max(bytes.length, 1), uid, mode, label);
    if (!published.bytes.equals(bytes) || published.stat.ino !== stage.snapshot.stat.ino) {
      refuse(`${label} publication verification failed`, 70, true);
    }
  } catch (error) {
    if (error instanceof HostsFileError) throw error;
    refuse(`${label} publication failed`, error?.code === "EEXIST" ? 75 : 70);
  } finally {
    if (stageExists) {
      try {
        if (!unlinkIfSameInode(stage.path, stage.snapshot.stat)) {
          refuse(`${label} stage cleanup failed`, 70, true);
        }
        fsyncDirectory(paths.parent);
      } catch {
        refuse(`${label} stage cleanup failed`, 70, true);
      }
    }
  }
}

function removeExact(path, snapshot, paths, label) {
  const current = readSnapshot(
    path,
    Math.max(snapshot.bytes.length, 1),
    Number(snapshot.stat.uid),
    modeOf(snapshot.stat),
    label,
  );
  if (!current.bytes.equals(snapshot.bytes) || !sameIdentity(current.stat, snapshot.stat)) {
    refuse(`${label} changed before removal`, 75);
  }
  try {
    unlinkSync(path);
    fsyncDirectory(paths.parent);
  } catch (error) {
    if (error instanceof HostsFileError) throw error;
    refuse(`${label} removal failed`, 70, true);
  }
}

function parseJson(bytes, label) {
  let value;
  try {
    value = JSON.parse(UTF8.decode(bytes));
  } catch {
    refuse(`${label} was refused`);
  }
  return value;
}

function serializeRecord(value) {
  return Buffer.from(`${JSON.stringify(value)}\n`, "utf8");
}

function backupRecord(selection, nonce, source, targetSnapshot) {
  return {
    format: "synveda-hosts-backup-v1",
    nonce,
    selection,
    target: {
      uid: Number(targetSnapshot.stat.uid),
      gid: Number(targetSnapshot.stat.gid),
      mode: modeOf(targetSnapshot.stat),
    },
    source: source.toString("base64"),
  };
}

function digest(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function fileWitness(stat) {
  return {
    dev: stat.dev.toString(),
    ino: stat.ino.toString(),
    nlink: stat.nlink.toString(),
    uid: stat.uid.toString(),
    gid: stat.gid.toString(),
    mode: stat.mode.toString(),
    size: stat.size.toString(),
    mtimeNs: stat.mtimeNs.toString(),
    ctimeNs: stat.ctimeNs.toString(),
  };
}

function validFileWitness(value) {
  const keys = ["dev", "ino", "nlink", "uid", "gid", "mode", "size", "mtimeNs", "ctimeNs"];
  return (
    exactKeys(value, keys) &&
    keys.every((key) => typeof value[key] === "string" && /^(?:0|[1-9][0-9]*)$/.test(value[key]))
  );
}

function witnessMatches(stat, witness) {
  return Object.entries(fileWitness(stat)).every(([key, value]) => witness[key] === value);
}

function publicRecord(selection, nonce, targetSnapshot, installed, backupSnapshot) {
  return {
    format: "synveda-hosts-state-v1",
    nonce,
    selection,
    target: {
      uid: Number(targetSnapshot.stat.uid),
      gid: Number(targetSnapshot.stat.gid),
      mode: modeOf(targetSnapshot.stat),
    },
    installedDigest: digest(installed),
    backupWitness: fileWitness(backupSnapshot.stat),
  };
}

function validatePublicRecord(value, selection) {
  if (
    !exactKeys(value, ["format", "nonce", "selection", "target", "installedDigest", "backupWitness"]) ||
    value.format !== "synveda-hosts-state-v1" ||
    !exactKeys(value.target, ["uid", "gid", "mode"]) ||
    !/^[0-9a-f]{64}$/.test(value.installedDigest) ||
    !validFileWitness(value.backupWitness)
  ) {
    refuse("hosts ownership state was refused");
  }
  const storedSelection = validateSelection(value.selection);
  if (!selectionEquals(storedSelection, selection) || !validNonce(value.nonce)) {
    refuse("hosts ownership state was refused");
  }
  for (const key of ["uid", "gid", "mode"]) {
    if (!Number.isSafeInteger(value.target[key]) || value.target[key] < 0) {
      refuse("hosts ownership state was refused");
    }
  }
  return {
    nonce: value.nonce,
    selection: storedSelection,
    target: value.target,
    installedDigest: value.installedDigest,
    backupWitness: value.backupWitness,
  };
}

function strictBase64(value) {
  if (typeof value !== "string" || value.length % 4 !== 0 || !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(value)) {
    refuse("hosts recovery state was refused");
  }
  const bytes = Buffer.from(value, "base64");
  if (bytes.toString("base64") !== value || bytes.length > TARGET_LIMIT) {
    refuse("hosts recovery state was refused");
  }
  return bytes;
}

function validateBackupRecord(value, selection) {
  if (
    !exactKeys(value, ["format", "nonce", "selection", "target", "source"]) ||
    value.format !== "synveda-hosts-backup-v1" ||
    !validNonce(value.nonce) ||
    !exactKeys(value.target, ["uid", "gid", "mode"])
  ) {
    refuse("hosts recovery state was refused");
  }
  const storedSelection = validateSelection(value.selection);
  if (!selectionEquals(storedSelection, selection)) refuse("hosts recovery state was refused");
  for (const key of ["uid", "gid", "mode"]) {
    if (!Number.isSafeInteger(value.target[key]) || value.target[key] < 0) {
      refuse("hosts recovery state was refused");
    }
  }
  const source = strictBase64(value.source);
  decodeHosts(source);
  if (classifyHostsBytes(source, selection).state !== "absent") {
    refuse("hosts recovery source was refused");
  }
  return { nonce: value.nonce, selection: storedSelection, target: value.target, source };
}

function inspectSidecars(paths, selection, expectedUid, openBackup) {
  const stateStat = lstatIfPresent(paths.state);
  const backupStat = lstatIfPresent(paths.backup);
  if (stateStat === undefined && backupStat === undefined) return undefined;
  if (backupStat === undefined) refuse("hosts recovery state is incomplete");
  paths.assertNoAcl(paths.backup, "hosts recovery state");
  validateRegular(backupStat, expectedUid, 0o600, "hosts recovery state");

  let backup;
  let backupSnapshot;
  if (openBackup) {
    backupSnapshot = readSnapshot(paths.backup, RECORD_LIMIT, expectedUid, 0o600, "hosts recovery state");
    backup = validateBackupRecord(parseJson(backupSnapshot.bytes, "hosts recovery state"), selection);
  }
  if (stateStat === undefined) return { backup, backupSnapshot, backupStat, public: undefined };
  paths.assertNoAcl(paths.state, "hosts ownership state");
  const stateSnapshot = readSnapshot(paths.state, 64 * 1024, expectedUid, undefined, "hosts ownership state");
  const publicState = validatePublicRecord(parseJson(stateSnapshot.bytes, "hosts ownership state"), selection);
  if (
    Number(stateSnapshot.stat.uid) !== publicState.target.uid ||
    Number(stateSnapshot.stat.gid) !== publicState.target.gid ||
    modeOf(stateSnapshot.stat) !== publicState.target.mode
  ) {
    refuse("hosts ownership state audience was refused");
  }
  if (backup !== undefined) {
    if (
      backup.nonce !== publicState.nonce ||
      backup.target.uid !== publicState.target.uid ||
      backup.target.gid !== publicState.target.gid ||
      backup.target.mode !== publicState.target.mode ||
      digest(installedBytes(backup.source, selection)) !== publicState.installedDigest ||
      !witnessMatches(backupSnapshot.stat, publicState.backupWitness)
    ) {
      refuse("hosts ownership state was refused");
    }
  }
  return { backup, backupSnapshot, backupStat, public: publicState, stateSnapshot };
}

function installedBytes(source, selection) {
  decodeHosts(source);
  const block = expectedBlock(selection);
  if (source.length + block.length > TARGET_LIMIT) {
    refuse("installed hosts file size was refused");
  }
  return Buffer.concat([source, block]);
}

function inspectStatus(paths, selection, expectedUid) {
  if (lstatIfPresent(paths.lock) !== undefined) {
    refuse("hosts mapping mutation is in progress or requires recovery", 75);
  }
  const target = readSnapshot(paths.target, TARGET_LIMIT, expectedUid, TARGET_MODE, "hosts file");
  const classified = classifyHostsBytes(target.bytes, selection);
  const managed = inspectSidecars(paths, selection, expectedUid, false);
  if (managed === undefined) {
    if (classified.state !== "absent") refuse("unowned hosts mapping was refused");
    return "absent";
  }
  if (managed.public === undefined || classified.state !== "exact") {
    refuse("hosts mapping recovery is incomplete", 75);
  }
  if (
    Number(target.stat.uid) !== managed.public.target.uid ||
    Number(target.stat.gid) !== managed.public.target.gid ||
    modeOf(target.stat) !== managed.public.target.mode ||
    digest(target.bytes) !== managed.public.installedDigest ||
    !witnessMatches(managed.backupStat, managed.public.backupWitness)
  ) {
    refuse("hosts ownership state was refused");
  }
  return "installed";
}

function lockRecord(pid, nonce) {
  return Buffer.from(`synveda-hosts-lock-v1 ${pid} ${nonce}\n`, "ascii");
}

function parseLock(bytes) {
  const match = /^synveda-hosts-lock-v1 ([1-9][0-9]*) ([^\n]+)\n$/.exec(bytes.toString("ascii"));
  if (match === null || Number(match[1]) > 2147483647 || !validNonce(match[2])) {
    refuse("hosts mutation lock was refused");
  }
  return { pid: Number(match[1]), nonce: match[2] };
}

function processExists(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    if (error?.code === "ESRCH") return false;
    return true;
  }
}

function removeStaleStage(paths, kind, nonce, expectedUid) {
  const path = stagePath(paths, kind, nonce);
  const before = lstatIfPresent(path);
  if (before === undefined) return;
  paths.assertNoAcl(path, "stale hosts stage");
  const identity = statIdentity(before);
  const mode = modeOf(before);
  if (
    before.isSymbolicLink() ||
    !before.isFile() ||
    Number(before.uid) !== expectedUid ||
    (mode & 0o7022) !== 0 ||
    before.size > BigInt(RECORD_LIMIT) ||
    (before.nlink !== 1n && before.nlink !== 2n)
  ) {
    refuse("stale hosts stage authority was refused", 75);
  }
  if (before.nlink === 2n) {
    const publishedPath = kind === "backup" ? paths.backup : kind === "state" ? paths.state : undefined;
    const published = publishedPath === undefined ? undefined : lstatIfPresent(publishedPath);
    if (
      published === undefined ||
      !published.isFile() ||
      published.dev !== before.dev ||
      published.ino !== before.ino ||
      published.nlink !== 2n
    ) {
      refuse("stale hosts stage linkage was refused", 75);
    }
  }
  if (!unlinkIfSameInode(path, identity)) refuse("stale hosts stage changed", 75);
  fsyncDirectory(paths.parent);
}

function removeStaleStages(paths, nonce, expectedUid) {
  for (const kind of Object.keys(STAGE_NAMES)) {
    removeStaleStage(paths, kind, nonce, expectedUid);
  }
}

function acquireLock(paths, expectedUid, expectedGid) {
  const existing = lstatIfPresent(paths.lock);
  if (existing !== undefined) {
    paths.assertNoAcl(paths.lock, "hosts mutation lock");
    const snapshot = readSnapshot(paths.lock, 256, expectedUid, 0o600, "hosts mutation lock");
    const owner = parseLock(snapshot.bytes);
    if (processExists(owner.pid)) refuse("another hosts mapping mutation is active", 75);
    removeStaleStages(paths, owner.nonce, expectedUid);
    removeExact(paths.lock, snapshot, paths, "stale hosts mutation lock");
  }
  const nonce = randomUUID();
  const bytes = lockRecord(process.pid, nonce);
  let fd;
  let createdIdentity;
  try {
    paths.assertNoAcl(paths.parent, "hosts parent directory");
    fd = openSync(paths.lock, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0o600);
    createdIdentity = statIdentity(fstatSync(fd, { bigint: true }));
    paths.assertNoAcl(paths.lock, "hosts mutation lock", createdIdentity);
    writeAll(fd, bytes);
    fchownSync(fd, expectedUid, expectedGid);
    fchmodSync(fd, 0o600);
    fsyncSync(fd);
    const closing = fd;
    fd = undefined;
    closeDescriptor(closing, "hosts mutation lock", true);
    fsyncDirectory(paths.parent);
  } catch (error) {
    let failure = error;
    if (fd !== undefined) {
      const closing = fd;
      fd = undefined;
      try {
        closeDescriptor(closing, "hosts mutation lock", true);
      } catch (closeError) {
        failure = closeError;
      }
    }
    if (createdIdentity !== undefined) {
      try {
        if (!unlinkIfSameInode(paths.lock, createdIdentity)) {
          refuse("hosts mutation lock cleanup failed", 70, true);
        }
        fsyncDirectory(paths.parent);
      } catch {
        refuse("hosts mutation lock cleanup failed", 70, true);
      }
    }
    if (failure instanceof HostsFileError) throw failure;
    refuse("hosts mutation lock was unavailable", failure?.code === "EEXIST" ? 75 : 70);
  }
  const snapshot = readSnapshot(paths.lock, 256, expectedUid, 0o600, "hosts mutation lock");
  const parsed = parseLock(snapshot.bytes);
  if (parsed.pid !== process.pid || parsed.nonce !== nonce || !snapshot.bytes.equals(bytes)) {
    refuse("hosts mutation lock ownership was refused", 75);
  }
  return snapshot;
}

function assertLock(paths, lock, expectedUid) {
  const current = readSnapshot(paths.lock, 256, expectedUid, 0o600, "hosts mutation lock");
  if (!current.bytes.equals(lock.bytes) || !sameIdentity(current.stat, lock.stat)) {
    refuse("hosts mutation lock ownership changed", 75);
  }
}

function sameAuthority(left, right) {
  return ["dev", "ino", "nlink", "uid", "gid", "mode"].every((key) => left[key] === right[key]);
}

function writeAllAt(fd, bytes, position, hooks) {
  let offset = 0;
  while (offset < bytes.length) {
    const requested = hooks.afterTargetPartialWrite === undefined ? bytes.length - offset : 1;
    const count = writeSync(fd, bytes, offset, requested, position + offset);
    if (count <= 0) refuse("hosts file write failed", 70, true);
    offset += count;
    if (offset < bytes.length) hooks.afterTargetPartialWrite?.();
  }
}

function assertTargetWritable(paths, target, expectedUid, hooks) {
  if ((modeOf(target.stat) & 0o200) === 0) refuse("hosts file writability was refused", 70);
  let fd;
  try {
    hooks.beforeWritablePreflight?.();
    fd = openSync(paths.target, O_RDWR | O_NOFOLLOW);
    const opened = fstatSync(fd, { bigint: true });
    validateRegular(opened, expectedUid, modeOf(target.stat), "hosts file");
    if (!sameIdentity(statIdentity(opened), target.stat)) {
      refuse("hosts file changed before writability preflight", 75);
    }
  } catch (error) {
    if (error instanceof HostsFileError) throw error;
    refuse("hosts file writability was refused", 70);
  } finally {
    if (fd !== undefined) {
      const closing = fd;
      fd = undefined;
      closeDescriptor(closing, "hosts file");
    }
  }
}

function syncTarget(paths, target, expectedUid, hooks) {
  let fd;
  try {
    hooks.beforeTargetSync?.();
    fd = openSync(paths.target, O_RDWR | O_NOFOLLOW);
    const openedStat = fstatSync(fd, { bigint: true });
    validateRegular(openedStat, expectedUid, modeOf(target.stat), "hosts file");
    const opened = statIdentity(openedStat);
    if (!sameIdentity(opened, target.stat)) refuse("hosts file changed before sync", 75, true);
    fsyncSync(fd);
    const synced = statIdentity(fstatSync(fd, { bigint: true }));
    if (!sameIdentity(synced, target.stat)) refuse("hosts file changed during sync", 75, true);
    hooks.afterTargetSync?.();
    const closing = fd;
    fd = undefined;
    closeDescriptor(closing, "hosts file", true, hooks.beforeTargetClose);
  } catch (error) {
    if (error instanceof HostsFileError) {
      error.uncertain = true;
      throw error;
    }
    refuse("hosts file sync failed", 70, true);
  } finally {
    if (fd !== undefined) {
      const closing = fd;
      fd = undefined;
      closeDescriptor(closing, "hosts file", true);
    }
  }
}

function mutateInPlace(paths, original, nextBytes, lock, expectedUid, hooks = {}) {
  const mode = modeOf(original.stat);
  let fd;
  let mutationStarted = false;
  try {
    hooks.beforeTargetRevalidate?.();
    fd = openSync(paths.target, O_RDWR | O_NOFOLLOW);
    const opened = fstatSync(fd, { bigint: true });
    validateRegular(opened, expectedUid, mode, "hosts file");
    if (!sameIdentity(statIdentity(opened), original.stat)) {
      refuse("hosts file changed before replacement", 75);
    }
    const currentBytes = readDescriptor(fd, TARGET_LIMIT);
    const revalidated = statIdentity(fstatSync(fd, { bigint: true }));
    if (!currentBytes.equals(original.bytes) || !sameIdentity(revalidated, original.stat)) {
      refuse("hosts file changed before replacement", 75);
    }
    assertLock(paths, lock, expectedUid);

    if (
      nextBytes.length >= original.bytes.length &&
      nextBytes.subarray(0, original.bytes.length).equals(original.bytes)
    ) {
      mutationStarted = true;
      writeAllAt(fd, nextBytes.subarray(original.bytes.length), original.bytes.length, hooks);
    } else if (
      nextBytes.length < original.bytes.length &&
      original.bytes.subarray(0, nextBytes.length).equals(nextBytes)
    ) {
      mutationStarted = true;
      ftruncateSync(fd, nextBytes.length);
    } else {
      refuse("hosts file mutation shape was refused", 70);
    }
    hooks.afterTargetMutation?.();
    fsyncSync(fd);
    const after = statIdentity(fstatSync(fd, { bigint: true }));
    if (!sameAuthority(original.stat, after) || after.size !== BigInt(nextBytes.length)) {
      refuse("hosts file authority changed during mutation", 75, true);
    }
    const closing = fd;
    fd = undefined;
    closeDescriptor(closing, "hosts file", true, hooks.beforeTargetClose);
    const installed = readSnapshot(paths.target, TARGET_LIMIT, expectedUid, mode, "hosts file");
    if (!installed.bytes.equals(nextBytes) || !sameAuthority(original.stat, installed.stat)) {
      refuse("hosts file mutation verification failed", 70, true);
    }
    return installed;
  } catch (error) {
    if (error instanceof HostsFileError) {
      if (mutationStarted && !error.uncertain) error.uncertain = true;
      throw error;
    }
    refuse("hosts file mutation failed", 70, mutationStarted);
  } finally {
    if (fd !== undefined) {
      const closing = fd;
      fd = undefined;
      closeDescriptor(closing, "hosts file", mutationStarted);
    }
  }
}

function prepareManagedState(paths, selection, target, expectedUid, expectedGid, lock, hooks) {
  let managed = inspectSidecars(paths, selection, expectedUid, true);
  if (managed === undefined) {
    if (classifyHostsBytes(target.bytes, selection).state !== "absent") {
      refuse("unowned hosts mapping was refused");
    }
    const nonce = randomUUID();
    const backup = backupRecord(selection, nonce, target.bytes, target);
    const installed = installedBytes(target.bytes, selection);
    publishExclusive(
      paths,
      paths.backup,
      serializeRecord(backup),
      0o600,
      expectedUid,
      expectedGid,
      "hosts recovery state",
      "backup",
      lock,
      hooks,
    );
    hooks.afterBackupPublished?.();
    const backupSnapshot = readSnapshot(
      paths.backup,
      RECORD_LIMIT,
      expectedUid,
      0o600,
      "hosts recovery state",
    );
    publishExclusive(
      paths,
      paths.state,
      serializeRecord(publicRecord(selection, nonce, target, installed, backupSnapshot)),
      modeOf(target.stat),
      Number(target.stat.uid),
      Number(target.stat.gid),
      "hosts ownership state",
      "state",
      lock,
      hooks,
    );
    hooks.afterStatePublished?.();
    managed = inspectSidecars(paths, selection, expectedUid, true);
  } else if (managed.public === undefined) {
    const backup = managed.backup;
    const installed = installedBytes(backup.source, selection);
    const targetForRecord = {
      stat: {
        uid: BigInt(backup.target.uid),
        gid: BigInt(backup.target.gid),
        mode: BigInt(backup.target.mode),
      },
    };
    const backupSnapshot = readSnapshot(
      paths.backup,
      RECORD_LIMIT,
      expectedUid,
      0o600,
      "hosts recovery state",
    );
    publishExclusive(
      paths,
      paths.state,
      serializeRecord(
        publicRecord(selection, backup.nonce, targetForRecord, installed, backupSnapshot),
      ),
      backup.target.mode,
      backup.target.uid,
      backup.target.gid,
      "hosts ownership state",
      "state",
      lock,
      hooks,
    );
    hooks.afterStatePublished?.();
    managed = inspectSidecars(paths, selection, expectedUid, true);
  }
  return managed;
}

function targetMatchesRecord(target, managed, selection, allowInterruptedPrefix = false) {
  const source = managed.backup.source;
  const installed = installedBytes(source, selection);
  const metadata = managed.backup.target;
  if (
    Number(target.stat.uid) !== metadata.uid ||
    Number(target.stat.gid) !== metadata.gid ||
    modeOf(target.stat) !== metadata.mode
  ) {
    refuse("hosts file metadata drift was refused");
  }
  if (target.bytes.equals(source)) return { state: "source", source, installed };
  if (target.bytes.equals(installed)) return { state: "installed", source, installed };
  if (
    allowInterruptedPrefix &&
    target.bytes.length > source.length &&
    target.bytes.length < installed.length &&
    installed.subarray(0, target.bytes.length).equals(target.bytes)
  ) {
    return { state: "interrupted-prefix", source, installed };
  }
  refuse("hosts file drift was refused");
}

function installMapping(paths, selection, expectedUid, expectedGid, lock, hooks) {
  let target = readSnapshot(paths.target, TARGET_LIMIT, expectedUid, TARGET_MODE, "hosts file");
  const existing = inspectSidecars(paths, selection, expectedUid, true);
  if (existing === undefined) {
    if (classifyHostsBytes(target.bytes, selection).state !== "absent") {
      refuse("unowned hosts mapping was refused");
    }
    assertTargetWritable(paths, target, expectedUid, hooks);
  } else {
    const existingMatch = targetMatchesRecord(target, existing, selection, true);
    if (existingMatch.state === "installed" && existing.public !== undefined) {
      syncTarget(paths, target, expectedUid, hooks);
      return "installed";
    }
    if (existingMatch.state !== "installed") {
      assertTargetWritable(paths, target, expectedUid, hooks);
    }
  }
  const managed = prepareManagedState(
    paths,
    selection,
    target,
    expectedUid,
    expectedGid,
    lock,
    hooks,
  );
  const match = targetMatchesRecord(target, managed, selection, true);
  if (match.state === "installed") {
    syncTarget(paths, target, expectedUid, hooks);
    return "installed";
  }
  target = mutateInPlace(paths, target, match.installed, lock, expectedUid, hooks);
  if (!target.bytes.equals(match.installed)) refuse("hosts installation verification failed", 70, true);
  return "installed";
}

function removeMapping(paths, selection, expectedUid, lock, hooks) {
  const target = readSnapshot(paths.target, TARGET_LIMIT, expectedUid, TARGET_MODE, "hosts file");
  const managed = inspectSidecars(paths, selection, expectedUid, true);
  if (managed === undefined) {
    if (classifyHostsBytes(target.bytes, selection).state !== "absent") {
      refuse("unowned hosts mapping was refused");
    }
    return "absent";
  }
  const match = targetMatchesRecord(target, managed, selection, true);
  let restored = target;
  if (match.state !== "source") {
    assertTargetWritable(paths, target, expectedUid, hooks);
    restored = mutateInPlace(paths, target, match.source, lock, expectedUid, hooks);
  } else {
    syncTarget(paths, target, expectedUid, hooks);
  }
  if (!restored.bytes.equals(match.source)) refuse("hosts removal verification failed", 70, true);
  if (managed.stateSnapshot !== undefined) {
    removeExact(paths.state, managed.stateSnapshot, paths, "hosts ownership state");
    hooks.afterStateRemoved?.();
  }
  const refreshed = readSnapshot(paths.backup, RECORD_LIMIT, expectedUid, 0o600, "hosts recovery state");
  removeExact(paths.backup, refreshed, paths, "hosts recovery state");
  return "absent";
}

export function manageHostsPathForTest(targetPath, action, selectionValue, confirmation, options = {}) {
  const selection = validateSelection(selectionValue);
  const expectedUid = options.expectedUid ?? process.getuid?.();
  const expectedGid = options.expectedGid ?? process.getgid?.();
  if (!Number.isInteger(expectedUid) || !Number.isInteger(expectedGid)) {
    refuse("platform identity was unavailable", 69);
  }
  const aclInspector = options.aclInspector ?? assertNoAccessAcl;
  const paths = resolveManagedPaths(targetPath, expectedUid, aclInspector);
  aclInspector(paths.target, "hosts file");
  if (action === "status") return inspectStatus(paths, selection, expectedUid);
  if (!new Set(["install", "remove"]).has(action)) refuse("action was refused", 64);
  if (confirmation !== expectedConfirmation(action, selection)) {
    refuse("exact hosts mapping confirmation was refused", 64);
  }
  const lock = acquireLock(paths, expectedUid, expectedGid);
  let uncertain = false;
  try {
    if (action === "install") {
      return installMapping(paths, selection, expectedUid, expectedGid, lock, options.hooks ?? {});
    }
    return removeMapping(paths, selection, expectedUid, lock, options.hooks ?? {});
  } catch (error) {
    uncertain = error instanceof HostsFileError && error.uncertain;
    throw error;
  } finally {
    if (!uncertain) {
      assertLock(paths, lock, expectedUid);
      removeExact(paths.lock, lock, paths, "hosts mutation lock");
    }
  }
}

function parseArguments(argv) {
  const action = argv[0];
  if (!new Set(["plan", "status", "install", "remove"]).has(action)) return undefined;
  const values = new Map();
  for (let index = 1; index < argv.length; index += 2) {
    const name = argv[index];
    const value = argv[index + 1];
    if (!name?.startsWith("--") || value === undefined || values.has(name)) return undefined;
    values.set(name, value);
  }
  const allowed = new Set(["--runtime", "--project", "--oidc", "--app-host", "--auth-host", "--confirm", "--expect"]);
  if ([...values.keys()].some((key) => !allowed.has(key))) return undefined;
  if (values.get("--runtime") !== "development") return undefined;
  const oidc = values.get("--oidc");
  const selection = {
    project: values.get("--project"),
    oidc,
    appHost: values.get("--app-host"),
    authHost: values.get("--auth-host") ?? null,
  };
  try {
    validateSelection(selection);
  } catch {
    return undefined;
  }
  const confirmation = values.get("--confirm");
  const expect = values.get("--expect") ?? "any";
  if (action === "plan") {
    if (confirmation !== undefined || values.has("--expect")) return undefined;
  } else if (action === "status") {
    if (confirmation !== undefined || !new Set(["any", "installed", "absent"]).has(expect)) return undefined;
  } else if (confirmation === undefined || values.has("--expect")) {
    return undefined;
  }
  return { action, selection, confirmation, expect };
}

export function main(argv = process.argv.slice(2)) {
  const parsed = parseArguments(argv);
  if (parsed === undefined) refuse("configuration was refused", 64);
  if (Number(process.versions.node.split(".")[0]) < 22 || !new Set(["darwin", "linux"]).has(process.platform)) {
    refuse("Node 22 or newer on macOS or Linux is required", 69);
  }
  if (parsed.action === "plan") {
    process.stdout.write(expectedBlock(parsed.selection));
    return;
  }
  if (parsed.action !== "status" && process.geteuid?.() !== 0) {
    refuse("install and remove require the narrow helper to run as root", 77);
  }
  const state = manageHostsPathForTest(
    HOSTS_PATH,
    parsed.action,
    parsed.selection,
    parsed.confirmation,
    { expectedUid: 0, expectedGid: 0 },
  );
  if (parsed.action === "status" && parsed.expect !== "any" && state !== parsed.expect) {
    refuse(`development hosts mapping is ${state}, expected ${parsed.expect}`);
  }
  console.log(`development hosts mapping is ${state}`);
}

let directEntrypoint = false;
if (process.argv[1]) {
  try {
    directEntrypoint = realpathSync(process.argv[1]) === fileURLToPath(import.meta.url);
  } catch {
    directEntrypoint = false;
  }
}

if (directEntrypoint) {
  try {
    main();
  } catch (error) {
    if (error instanceof HostsFileError) {
      console.error(`hosts-file: ${error.message}`);
      process.exitCode = error.status;
    } else {
      console.error("hosts-file: unexpected failure");
      process.exitCode = 70;
    }
  }
}
