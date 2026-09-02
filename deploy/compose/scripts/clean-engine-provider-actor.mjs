#!/usr/bin/env node
import { spawn } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import {
  closeSync,
  constants,
  existsSync,
  fchmodSync,
  fstatSync,
  fsyncSync,
  linkSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  readdirSync,
  realpathSync,
  unlinkSync,
  writeSync,
} from "node:fs";
import { isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const MAX_ARTIFACT_BYTES = 128 * 1024;
const MAX_ARTIFACT_STAGES = 16;
const OWNER_MARKER = ".synveda-clean-engine-owner.json";
const ZERO_SHA256 = "0".repeat(64);
const ROOT_LAYOUT = Object.freeze({
  COLIMA_CACHE_HOME: "k",
  COLIMA_HOME: "c",
  DOCKER_CONFIG: "d",
  LIMA_HOME: "l",
  TMPDIR: "t",
});
const PROVIDER_ARTIFACT_NAMES = Object.freeze([
  "root-plan.json",
  "root-reservation.json",
  "root-owner.json",
  "actor-launch.json",
  "actor-witness.json",
  "actor-decision.json",
  "actor-outcome.json",
  "actor-settlement.json",
]);
const PROVIDER_SCENARIOS = Object.freeze(["fail", "hang", "orphan", "pass"]);
const ACTOR_SCRIPT = fileURLToPath(import.meta.url);
const FIXED_FAKE_COMMAND = fileURLToPath(
  new URL("./clean-engine-provider-fake-command.mjs", import.meta.url),
);

export const CONTROLLED_FAKE_PROVIDER_ADAPTER_FIELDS = Object.freeze([
  "after_decision_hold_milliseconds",
  "after_intent_hold_milliseconds",
  "after_outcome_publish_hold_milliseconds",
  "after_root_plan_hold_milliseconds",
  "after_settlement_hold_milliseconds",
  "before_decision_hold_milliseconds",
  "before_root_creation_hold_milliseconds",
  "before_root_mirror_hold_milliseconds",
  "before_witness_hold_milliseconds",
  "child_scenario",
  "close_prelink_hold_milliseconds",
  "deadline_milliseconds",
  "gate_delivery",
  "kind",
  "kill_grace_milliseconds",
  "term_grace_milliseconds",
]);

export const CONTROLLED_FAKE_PROVIDER_CONTRACT = Object.freeze({
  actor_protocol: "single-use-ipc-token-v1",
  adapter_fields: CONTROLLED_FAKE_PROVIDER_ADAPTER_FIELDS,
  child_command: "clean-engine-controlled-fake-command-v1",
  child_scenarios: PROVIDER_SCENARIOS,
  gate_deliveries: Object.freeze(["correct", "duplicate", "wrong"]),
  group_absence_probe: "negative-pgid-esrch-v1",
  kind: "controlled-fake-provider-v1",
  max_deadline_milliseconds: 30_000,
  max_hold_milliseconds: 30_000,
  max_kill_grace_milliseconds: 2_000,
  max_term_grace_milliseconds: 2_000,
  root_layout: ROOT_LAYOUT,
  schema: "synveda.clean-engine.controlled-fake-provider-contract.v1",
});

export class ControlledProviderFailure extends Error {
  constructor(message, exitStatus = 78) {
    super(message);
    this.exitStatus = exitStatus;
  }
}

function fail(message, exitStatus = 78) {
  throw new ControlledProviderFailure(message, exitStatus);
}

function canonical(value) {
  if (value === null || typeof value === "string" || typeof value === "boolean") {
    return JSON.stringify(value);
  }
  if (typeof value === "number" && Number.isSafeInteger(value)) return String(value);
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`)
      .join(",")}}`;
  }
  fail("controlled provider canonical value was refused", 70);
}

export function controlledProviderBytes(value) {
  return Buffer.from(`${canonical(value)}\n`, "utf8");
}

export function controlledProviderDigest(value) {
  return createHash("sha256").update(value).digest("hex");
}

export const CONTROLLED_FAKE_PROVIDER_CONTRACT_SHA256 = controlledProviderDigest(
  controlledProviderBytes(CONTROLLED_FAKE_PROVIDER_CONTRACT),
);

function lowerHex(value, length) {
  return typeof value === "string" && value.length === length && /^[0-9a-f]+$/.test(value);
}

function decimalString(value) {
  return typeof value === "string" && /^(?:0|[1-9][0-9]*)$/.test(value);
}

function exactKeys(value, keys, label) {
  if (value === null || Array.isArray(value) || typeof value !== "object") {
    fail(`${label} was malformed`);
  }
  if (JSON.stringify(Object.keys(value).sort()) !== JSON.stringify([...keys].sort())) {
    fail(`${label} fields were refused`);
  }
}

function exactMetadata(path, label, kind, mode, links) {
  let metadata;
  try {
    metadata = lstatSync(path, { bigint: true });
  } catch {
    fail(`${label} was unavailable`, 69);
  }
  if (
    metadata.isSymbolicLink() ||
    (kind === "directory" ? !metadata.isDirectory() : !metadata.isFile()) ||
    metadata.uid !== BigInt(process.getuid()) ||
    (metadata.mode & 0o7777n) !== BigInt(mode) ||
    (links !== undefined && metadata.nlink !== BigInt(links))
  ) {
    fail(`${label} identity was refused`);
  }
  return metadata;
}

function metadataValue(path, label, kind, mode, links) {
  const metadata = exactMetadata(path, label, kind, mode, links);
  return {
    device: String(metadata.dev),
    inode: String(metadata.ino),
    mode: mode.toString(8).padStart(4, "0"),
    path,
    uid: String(metadata.uid),
  };
}

function syncDirectory(path) {
  let descriptor;
  try {
    descriptor = openSync(
      path,
      constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW,
    );
    fsyncSync(descriptor);
  } catch {
    fail("controlled provider directory sync failed", 70);
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

function sameIdentity(left, right) {
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.uid === right.uid &&
    left.mode === right.mode &&
    left.nlink === right.nlink
  );
}

function assertOpenedPath(
  descriptor,
  path,
  label,
  kind,
  mode,
  links = kind === "file" ? 1n : undefined,
) {
  const opened = fstatSync(descriptor, { bigint: true });
  const named = exactMetadata(
    path,
    label,
    kind,
    mode,
    links === undefined ? undefined : Number(links),
  );
  if (!sameIdentity(opened, named)) fail(`${label} identity changed`);
  return opened;
}

function secureCreatedDirectory(path, label) {
  let descriptor;
  try {
    descriptor = openSync(
      path,
      constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW,
    );
    fchmodSync(descriptor, 0o700);
    return assertOpenedPath(descriptor, path, label, "directory", 0o700);
  } catch (error) {
    if (error instanceof ControlledProviderFailure) throw error;
    fail(`${label} identity could not be secured`, 70);
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

function writeAll(descriptor, bytes) {
  let offset = 0;
  while (offset < bytes.length) {
    const written = writeSync(descriptor, bytes, offset, bytes.length - offset);
    if (written < 1) fail("controlled provider artifact write failed", 70);
    offset += written;
  }
}

export function publishControlledProviderArtifact(providerDirectory, name, value) {
  if (!PROVIDER_ARTIFACT_NAMES.includes(name)) {
    fail("controlled provider artifact name was refused", 64);
  }
  exactMetadata(providerDirectory, "controlled provider state", "directory", 0o700);
  const path = join(providerDirectory, name);
  const stagePath = join(
    providerDirectory,
    `.artifact-stage-${randomBytes(16).toString("hex")}`,
  );
  const bytes = controlledProviderBytes(value);
  let descriptor;
  let stageIdentity;
  let linked = false;
  try {
    descriptor = openSync(
      stagePath,
      constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW,
      0o600,
    );
    fchmodSync(descriptor, 0o600);
    stageIdentity = assertOpenedPath(
      descriptor,
      stagePath,
      "controlled provider artifact stage",
      "file",
      0o600,
    );
    writeAll(descriptor, bytes);
    fsyncSync(descriptor);
    const staged = assertOpenedPath(
      descriptor,
      stagePath,
      "controlled provider artifact stage",
      "file",
      0o600,
    );
    linkSync(stagePath, path);
    linked = true;
    const finalMetadata = exactMetadata(
      path,
      `controlled provider ${name}`,
      "file",
      0o600,
      2,
    );
    if (
      finalMetadata.dev !== staged.dev ||
      finalMetadata.ino !== staged.ino ||
      finalMetadata.uid !== staged.uid ||
      finalMetadata.mode !== staged.mode
    ) {
      fail("controlled provider artifact link identity changed");
    }
    closeSync(descriptor);
    descriptor = undefined;
    syncDirectory(providerDirectory);
    unlinkSync(stagePath);
    syncDirectory(providerDirectory);
  } catch (error) {
    if (error instanceof ControlledProviderFailure) throw error;
    if (error?.code === "EEXIST") fail("controlled provider artifact already existed", 73);
    fail("controlled provider artifact publication failed", 70);
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
    if (!linked && existsSync(stagePath)) {
      try {
        const stage = lstatSync(stagePath, { bigint: true });
        if (
          stage.isFile() &&
          !stage.isSymbolicLink() &&
          stageIdentity !== undefined &&
          stage.dev === stageIdentity.dev &&
          stage.ino === stageIdentity.ino &&
          stage.uid === BigInt(process.getuid()) &&
          stage.nlink === 1n &&
          (stage.mode & 0o7777n) === 0o600n
        ) {
          unlinkSync(stagePath);
          syncDirectory(providerDirectory);
        }
      } catch {
        // A changed or linked stage is durable uncertain evidence.
      }
    }
  }
  const current = readCanonical(path, `controlled provider ${name}`);
  if (!current.bytes.equals(bytes)) fail("controlled provider artifact publication changed", 70);
  return Object.freeze({ bytes, path, sha256: controlledProviderDigest(bytes), value });
}

function readCanonical(path, label, expectedLinks = 1) {
  const metadata = exactMetadata(path, label, "file", 0o600, expectedLinks);
  if (metadata.size < 2n || metadata.size > BigInt(MAX_ARTIFACT_BYTES)) {
    fail(`${label} size was refused`);
  }
  let bytes;
  try {
    bytes = readFileSync(path);
  } catch {
    fail(`${label} was unavailable`, 69);
  }
  const after = exactMetadata(path, label, "file", 0o600, expectedLinks);
  if (
    after.dev !== metadata.dev ||
    after.ino !== metadata.ino ||
    after.size !== metadata.size ||
    after.mode !== metadata.mode
  ) {
    fail(`${label} changed while it was read`);
  }
  let value;
  try {
    value = JSON.parse(bytes.toString("utf8"));
  } catch {
    fail(`${label} was not canonical JSON`);
  }
  if (!controlledProviderBytes(value).equals(bytes)) fail(`${label} was not canonical JSON`);
  return Object.freeze({ bytes, metadata, path, sha256: controlledProviderDigest(bytes), value });
}

function pathsOverlap(left, right) {
  const fromLeft = relative(left, right);
  const fromRight = relative(right, left);
  const nested = (value) => value === "" || (!value.startsWith(`..${sep}`) && value !== "..");
  return nested(fromLeft) || nested(fromRight);
}

function assertNoSymlinkComponents(path, label) {
  const parsed = path.split(sep);
  let current = sep;
  for (const component of parsed) {
    if (component === "") continue;
    current = join(current, component);
    let metadata;
    try {
      metadata = lstatSync(current, { bigint: true });
    } catch {
      fail(`${label} component was unavailable`, 69);
    }
    if (metadata.isSymbolicLink()) fail(`${label} symlink component was refused`);
  }
}

export function planControlledProviderRoot({ fixtureId, providerBase, repoRoot, stateBase }) {
  if (
    !lowerHex(fixtureId, 32) ||
    typeof providerBase !== "string" ||
    !isAbsolute(providerBase) ||
    typeof repoRoot !== "string" ||
    typeof stateBase !== "string"
  ) {
    fail("controlled provider root arguments were refused", 64);
  }
  if (
    typeof process.getuid !== "function" ||
    typeof process.geteuid !== "function" ||
    process.getuid() === 0 ||
    process.getuid() !== process.geteuid() ||
    !new Set(["darwin", "linux"]).has(process.platform)
  ) {
    fail("controlled provider requires one non-root POSIX identity", 69);
  }
  let canonicalBase;
  try {
    canonicalBase = realpathSync(providerBase);
  } catch {
    fail("controlled provider base was unavailable", 69);
  }
  if (canonicalBase !== providerBase || resolve(providerBase) !== providerBase) {
    fail("controlled provider base must use its canonical absolute path");
  }
  assertNoSymlinkComponents(providerBase, "controlled provider base");
  const base = metadataValue(providerBase, "controlled provider base", "directory", 0o700);
  const canonicalRepo = realpathSync(repoRoot);
  const canonicalState = realpathSync(stateBase);
  if (pathsOverlap(providerBase, canonicalRepo) || pathsOverlap(providerBase, canonicalState)) {
    fail("controlled provider base overlapped protected state");
  }
  const rootKey = `sv-c45-${fixtureId.slice(0, 16)}`;
  const providerProfile = rootKey;
  const rootPath = join(providerBase, rootKey);
  const longestSocket = join(rootPath, ROOT_LAYOUT.LIMA_HOME, `colima-${providerProfile}`, "ha.sock");
  if (Buffer.byteLength(longestSocket, "utf8") > 103) {
    fail("controlled provider socket path bound was exceeded");
  }
  return Object.freeze({
    base,
    fixture_id: fixtureId,
    layout: ROOT_LAYOUT,
    ownership_nonce: randomBytes(32).toString("hex"),
    provider: "colima",
    provider_profile: providerProfile,
    provider_resource: `synveda-cpr45-${fixtureId}`,
    root_key: rootKey,
    root_path: rootPath,
    schema: "synveda.clean-engine.provider-root-plan.v1",
    unix_socket_path_limit_bytes: 103,
  });
}

export function controlledProviderIntent(fixtureId, rootPlan, ownershipNonce) {
  validateRootPlan(rootPlan, fixtureId);
  if (!lowerHex(ownershipNonce, 64) || ownershipNonce !== rootPlan.ownership_nonce) {
    fail("controlled provider ownership nonce was refused", 64);
  }
  return Object.freeze({
    cleanup_command: "colima-delete-data-force",
    ownership_nonce: ownershipNonce,
    preexisting_docker_context: "absent",
    preexisting_provider_instance: "absent",
    preexisting_provider_root: "absent",
    preexisting_resource: "absent",
    provider_contract_sha256: CONTROLLED_FAKE_PROVIDER_CONTRACT_SHA256,
    provider_profile: rootPlan.provider_profile,
    provider_resource: rootPlan.provider_resource,
    provider_root_key: rootPlan.root_key,
    provider_root_plan_sha256: controlledProviderDigest(controlledProviderBytes(rootPlan)),
  });
}

export function controlledProviderIntendedReceipt(
  fixtureId,
  rootPlan,
  sourceHeadSha256,
  sourceSequence,
) {
  if (
    !lowerHex(sourceHeadSha256, 64) ||
    !Number.isSafeInteger(sourceSequence) ||
    sourceSequence < 0
  ) {
    fail("controlled provider intended receipt source was refused", 64);
  }
  return Object.freeze({
    fixture_id: fixtureId,
    outcome: "intent",
    phase: "provider-create-intent",
    previous_sha256: sourceHeadSha256,
    result: controlledProviderIntent(fixtureId, rootPlan, rootPlan.ownership_nonce),
    schema: "synveda.clean-engine.receipt.v2",
    sequence: sourceSequence + 1,
  });
}

export function validateControlledProviderAdapter(adapter) {
  exactKeys(adapter, CONTROLLED_FAKE_PROVIDER_ADAPTER_FIELDS, "controlled provider adapter");
  const bounded = (value, maximum, minimum = 0) =>
    Number.isSafeInteger(value) && value >= minimum && value <= maximum;
  if (
    adapter.kind !== CONTROLLED_FAKE_PROVIDER_CONTRACT.kind ||
    !PROVIDER_SCENARIOS.includes(adapter.child_scenario) ||
    !CONTROLLED_FAKE_PROVIDER_CONTRACT.gate_deliveries.includes(adapter.gate_delivery) ||
    !bounded(
      adapter.before_decision_hold_milliseconds,
      CONTROLLED_FAKE_PROVIDER_CONTRACT.max_hold_milliseconds,
    ) ||
    !bounded(
      adapter.before_root_creation_hold_milliseconds,
      CONTROLLED_FAKE_PROVIDER_CONTRACT.max_hold_milliseconds,
    ) ||
    !bounded(
      adapter.before_root_mirror_hold_milliseconds,
      CONTROLLED_FAKE_PROVIDER_CONTRACT.max_hold_milliseconds,
    ) ||
    !bounded(
      adapter.before_witness_hold_milliseconds,
      CONTROLLED_FAKE_PROVIDER_CONTRACT.max_hold_milliseconds,
    ) ||
    !bounded(
      adapter.after_decision_hold_milliseconds,
      CONTROLLED_FAKE_PROVIDER_CONTRACT.max_hold_milliseconds,
    ) ||
    !bounded(
      adapter.after_intent_hold_milliseconds,
      CONTROLLED_FAKE_PROVIDER_CONTRACT.max_hold_milliseconds,
    ) ||
    !bounded(
      adapter.after_outcome_publish_hold_milliseconds,
      CONTROLLED_FAKE_PROVIDER_CONTRACT.max_hold_milliseconds,
    ) ||
    !bounded(
      adapter.after_root_plan_hold_milliseconds,
      CONTROLLED_FAKE_PROVIDER_CONTRACT.max_hold_milliseconds,
    ) ||
    !bounded(
      adapter.after_settlement_hold_milliseconds,
      CONTROLLED_FAKE_PROVIDER_CONTRACT.max_hold_milliseconds,
    ) ||
    !bounded(
      adapter.close_prelink_hold_milliseconds,
      CONTROLLED_FAKE_PROVIDER_CONTRACT.max_hold_milliseconds,
    ) ||
    !bounded(
      adapter.deadline_milliseconds,
      CONTROLLED_FAKE_PROVIDER_CONTRACT.max_deadline_milliseconds,
      100,
    ) ||
    !bounded(
      adapter.term_grace_milliseconds,
      CONTROLLED_FAKE_PROVIDER_CONTRACT.max_term_grace_milliseconds,
      10,
    ) ||
    !bounded(
      adapter.kill_grace_milliseconds,
      CONTROLLED_FAKE_PROVIDER_CONTRACT.max_kill_grace_milliseconds,
      10,
    )
  ) {
    fail("controlled provider adapter was refused", 64);
  }
}

function validateMetadataValue(value, expected, label) {
  exactKeys(value, ["device", "inode", "mode", "path", "uid"], label);
  if (
    !decimalString(value.device) ||
    !decimalString(value.inode) ||
    !decimalString(value.uid) ||
    value.mode !== expected.mode ||
    value.path !== expected.path
  ) {
    fail(`${label} was refused`);
  }
}

function validateRootPlan(value, fixtureId) {
  exactKeys(
    value,
    [
      "base",
      "fixture_id",
      "layout",
      "ownership_nonce",
      "provider",
      "provider_profile",
      "provider_resource",
      "root_key",
      "root_path",
      "schema",
      "unix_socket_path_limit_bytes",
    ],
    "controlled provider root plan",
  );
  const key = `sv-c45-${fixtureId.slice(0, 16)}`;
  if (
    value.schema !== "synveda.clean-engine.provider-root-plan.v1" ||
    value.fixture_id !== fixtureId ||
    value.provider !== "colima" ||
    !lowerHex(value.ownership_nonce, 64) ||
    value.provider_profile !== key ||
    value.provider_resource !== `synveda-cpr45-${fixtureId}` ||
    value.root_key !== key ||
    value.root_path !== join(value.base?.path ?? "", key) ||
    value.unix_socket_path_limit_bytes !== 103 ||
    canonical(value.layout) !== canonical(ROOT_LAYOUT)
  ) {
    fail("controlled provider root plan was refused");
  }
  validateMetadataValue(value.base, { mode: "0700", path: value.base.path }, "provider base");
  const current = metadataValue(value.base.path, "controlled provider base", "directory", 0o700);
  if (canonical(current) !== canonical(value.base)) fail("controlled provider base identity changed");
}

export function providerRootPreflight(rootPlan) {
  validateRootPlan(rootPlan, rootPlan.fixture_id);
  try {
    lstatSync(rootPlan.root_path);
    return "collision";
  } catch (error) {
    if (error?.code === "ENOENT") return "absent";
    fail("controlled provider root preflight was unavailable", 69);
  }
}

function rootReservationValue(rootPlan, intentSha256, ownershipNonce) {
  return {
    fixture_id: rootPlan.fixture_id,
    ownership_nonce: ownershipNonce,
    provider_intent_sha256: intentSha256,
    provider_root_plan_sha256: controlledProviderDigest(controlledProviderBytes(rootPlan)),
    root_path: rootPlan.root_path,
    schema: "synveda.clean-engine.provider-root-reservation.v1",
  };
}

export function publishProviderRootReservation(
  providerDirectory,
  rootPlan,
  intentSha256,
  ownershipNonce,
) {
  if (!lowerHex(intentSha256, 64) || !lowerHex(ownershipNonce, 64)) {
    fail("controlled provider root reservation binding was refused", 64);
  }
  return publishControlledProviderArtifact(
    providerDirectory,
    "root-reservation.json",
    rootReservationValue(rootPlan, intentSha256, ownershipNonce),
  );
}

function createOwnerMarker({ intentSha256, ownershipNonce, providerContractSha256, rootPlan }) {
  const rootPath = rootPlan.root_path;
  const stagePath = join(rootPath, `.owner-stage-${ownershipNonce.slice(0, 32)}`);
  const finalPath = join(rootPath, OWNER_MARKER);
  let descriptor;
  let stageIdentity;
  let linked = false;
  try {
    descriptor = openSync(
      stagePath,
      constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW,
      0o600,
    );
    fchmodSync(descriptor, 0o600);
    const markerMetadata = assertOpenedPath(
      descriptor,
      stagePath,
      "controlled provider owner stage",
      "file",
      0o600,
    );
    stageIdentity = markerMetadata;
    const roots = Object.entries(ROOT_LAYOUT)
      .map(([environment, leaf]) => ({
        environment,
        relative_path: leaf,
        ...metadataValue(join(rootPath, leaf), `controlled provider ${environment}`, "directory", 0o700),
      }))
      .sort((left, right) => left.environment.localeCompare(right.environment));
    const value = {
      base: rootPlan.base,
      fixture_id: rootPlan.fixture_id,
      marker: {
        device: String(markerMetadata.dev),
        inode: String(markerMetadata.ino),
        mode: "0600",
        relative_path: OWNER_MARKER,
        uid: String(markerMetadata.uid),
      },
      ownership_nonce: ownershipNonce,
      provider: rootPlan.provider,
      provider_contract_sha256: providerContractSha256,
      provider_intent_sha256: intentSha256,
      provider_profile: rootPlan.provider_profile,
      provider_resource: rootPlan.provider_resource,
      provider_root_plan_sha256: controlledProviderDigest(controlledProviderBytes(rootPlan)),
      root: metadataValue(rootPath, "controlled provider root", "directory", 0o700),
      roots,
      schema: "synveda.clean-engine.provider-root-owner.v1",
    };
    const bytes = controlledProviderBytes(value);
    writeAll(descriptor, bytes);
    fsyncSync(descriptor);
    const staged = assertOpenedPath(
      descriptor,
      stagePath,
      "controlled provider owner stage",
      "file",
      0o600,
    );
    linkSync(stagePath, finalPath);
    linked = true;
    const finalMetadata = exactMetadata(
      finalPath,
      "controlled provider owner marker",
      "file",
      0o600,
      2,
    );
    if (
      finalMetadata.dev !== staged.dev ||
      finalMetadata.ino !== staged.ino ||
      finalMetadata.uid !== staged.uid ||
      finalMetadata.mode !== staged.mode
    ) {
      fail("controlled provider owner marker link identity changed");
    }
    closeSync(descriptor);
    descriptor = undefined;
    syncDirectory(rootPath);
    unlinkSync(stagePath);
    syncDirectory(rootPath);
    const parsed = readCanonical(finalPath, "controlled provider owner marker");
    if (!parsed.bytes.equals(bytes)) fail("controlled provider owner marker changed", 70);
    return parsed;
  } catch (error) {
    if (error instanceof ControlledProviderFailure) throw error;
    if (error?.code === "EEXIST") fail("controlled provider owner marker collided", 73);
    fail("controlled provider owner marker publication failed", 70);
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
    if (!linked && existsSync(stagePath)) {
      // The root is already receipt-reserved. Remove only the inode opened by
      // this publication; a collision, replacement or linked stage is evidence.
      try {
        const metadata = lstatSync(stagePath, { bigint: true });
        if (
          stageIdentity !== undefined &&
          metadata.isFile() &&
          !metadata.isSymbolicLink() &&
          metadata.dev === stageIdentity.dev &&
          metadata.ino === stageIdentity.ino &&
          metadata.uid === stageIdentity.uid &&
          metadata.mode === stageIdentity.mode &&
          metadata.nlink === 1n
        ) {
          unlinkSync(stagePath);
          syncDirectory(rootPath);
        }
      } catch {
        // Preserve uncertain state for explicit recovery.
      }
    }
  }
}

export function createOwnedProviderRoot({
  intentSha256,
  ownershipNonce,
  providerContractSha256,
  rootPlan,
}) {
  validateRootPlan(rootPlan, rootPlan.fixture_id);
  if (
    !lowerHex(intentSha256, 64) ||
    !lowerHex(ownershipNonce, 64) ||
    providerContractSha256 !== CONTROLLED_FAKE_PROVIDER_CONTRACT_SHA256
  ) {
    fail("controlled provider root ownership binding was refused", 64);
  }
  try {
    mkdirSync(rootPlan.root_path, { mode: 0o700 });
  } catch (error) {
    if (error?.code === "EEXIST") return Object.freeze({ outcome: "collision" });
    fail("controlled provider root creation failed", 70);
  }
  secureCreatedDirectory(rootPlan.root_path, "controlled provider root");
  syncDirectory(rootPlan.base.path);
  for (const leaf of Object.values(ROOT_LAYOUT).sort()) {
    const path = join(rootPlan.root_path, leaf);
    mkdirSync(path, { mode: 0o700 });
    secureCreatedDirectory(path, "controlled provider environment root");
  }
  syncDirectory(rootPlan.root_path);
  const marker = createOwnerMarker({
    intentSha256,
    ownershipNonce,
    providerContractSha256,
    rootPlan,
  });
  return Object.freeze({ marker, outcome: "owned" });
}

export function publishOwnedProviderRootMirror(providerDirectory, marker) {
  const mirror = publishControlledProviderArtifact(
    providerDirectory,
    "root-owner.json",
    marker.value,
  );
  if (!mirror.bytes.equals(marker.bytes)) fail("controlled provider root mirror changed", 70);
  return mirror;
}

export function recoverOwnedProviderRootMirror({
  intentSha256,
  ownershipNonce,
  providerDirectory,
  reservation,
  rootPlan,
}) {
  validateRootPlan(rootPlan, rootPlan.fixture_id);
  validateReservation(reservation.value, rootPlan, intentSha256, ownershipNonce);
  const stageName = ownerStageName(ownershipNonce);
  const linkedOwnerStage = readdirSync(rootPlan.root_path).includes(stageName);
  let marker = readCanonical(
    join(rootPlan.root_path, OWNER_MARKER),
    "controlled provider owner marker",
    linkedOwnerStage ? 2 : 1,
  );
  assertRootOwner(
    marker.value,
    rootPlan,
    intentSha256,
    ownershipNonce,
    linkedOwnerStage,
  );
  if (linkedOwnerStage) {
    const stagePath = join(rootPlan.root_path, stageName);
    const before = exactMetadata(
      stagePath,
      "controlled provider owner stage",
      "file",
      0o600,
      2,
    );
    const owner = exactMetadata(
      join(rootPlan.root_path, OWNER_MARKER),
      "controlled provider owner marker",
      "file",
      0o600,
      2,
    );
    if (before.dev !== owner.dev || before.ino !== owner.ino) {
      fail("controlled provider owner stage link was refused");
    }
    try {
      unlinkSync(stagePath);
      syncDirectory(rootPlan.root_path);
    } catch {
      fail("controlled provider owner stage recovery failed", 70);
    }
    const stable = readCanonical(
      join(rootPlan.root_path, OWNER_MARKER),
      "controlled provider owner marker",
    );
    if (!stable.bytes.equals(marker.bytes)) {
      fail("controlled provider owner marker changed during recovery");
    }
    marker = stable;
    assertRootOwner(marker.value, rootPlan, intentSha256, ownershipNonce);
  }
  const mirror = publishOwnedProviderRootMirror(providerDirectory, marker);
  if (!mirror.bytes.equals(marker.bytes)) {
    fail("controlled provider recovered root mirror changed", 70);
  }
  return mirror;
}

export function inspectControlledProviderArtifacts(providerDirectory) {
  exactMetadata(providerDirectory, "controlled provider state", "directory", 0o700);
  const entries = readdirSync(providerDirectory).sort();
  const stages = entries.filter((entry) => /^\.artifact-stage-[0-9a-f]{32}$/.test(entry));
  const finals = entries.filter((entry) => PROVIDER_ARTIFACT_NAMES.includes(entry));
  if (
    stages.length > MAX_ARTIFACT_STAGES ||
    entries.some((entry) => !PROVIDER_ARTIFACT_NAMES.includes(entry) && !stages.includes(entry))
  ) {
    fail("controlled provider state inventory was refused");
  }
  const linkedFinals = new Map();
  for (const stageName of stages) {
    const stagePath = join(providerDirectory, stageName);
    const stage = exactMetadata(stagePath, "controlled provider artifact stage", "file", 0o600);
    if (stage.nlink === 1n) continue;
    if (stage.nlink !== 2n) fail("controlled provider artifact stage links were refused");
    const matches = finals.filter((name) => {
      const candidate = lstatSync(join(providerDirectory, name), { bigint: true });
      return candidate.dev === stage.dev && candidate.ino === stage.ino;
    });
    if (matches.length !== 1 || linkedFinals.has(matches[0])) {
      fail("controlled provider artifact stage link was refused");
    }
    linkedFinals.set(matches[0], stageName);
  }
  const artifacts = {};
  for (const name of finals) {
    const metadata = lstatSync(join(providerDirectory, name), { bigint: true });
    const expectedLinks = linkedFinals.has(name) ? 2 : 1;
    if (metadata.nlink !== BigInt(expectedLinks)) {
      fail("controlled provider artifact final links were refused");
    }
    artifacts[name] = readCanonical(join(providerDirectory, name), name, expectedLinks);
  }
  return Object.freeze(artifacts);
}

function requireArtifact(artifacts, name) {
  const artifact = artifacts[name];
  if (artifact === undefined) fail(`controlled provider ${name} was unavailable`, 69);
  return artifact;
}

function validateReservation(value, rootPlan, intentSha256, ownershipNonce) {
  exactKeys(
    value,
    [
      "fixture_id",
      "ownership_nonce",
      "provider_intent_sha256",
      "provider_root_plan_sha256",
      "root_path",
      "schema",
    ],
    "controlled provider root reservation",
  );
  if (
    value.schema !== "synveda.clean-engine.provider-root-reservation.v1" ||
    value.fixture_id !== rootPlan.fixture_id ||
    value.ownership_nonce !== ownershipNonce ||
    value.provider_intent_sha256 !== intentSha256 ||
    value.provider_root_plan_sha256 !== controlledProviderDigest(controlledProviderBytes(rootPlan)) ||
    value.root_path !== rootPlan.root_path
  ) {
    fail("controlled provider root reservation was refused");
  }
}

function ownerStageName(ownershipNonce) {
  return `.owner-stage-${ownershipNonce.slice(0, 32)}`;
}

function assertRootOwner(
  value,
  rootPlan,
  intentSha256,
  ownershipNonce,
  allowLinkedOwnerStage = false,
) {
  exactKeys(
    value,
    [
      "base",
      "fixture_id",
      "marker",
      "ownership_nonce",
      "provider",
      "provider_contract_sha256",
      "provider_intent_sha256",
      "provider_profile",
      "provider_resource",
      "provider_root_plan_sha256",
      "root",
      "roots",
      "schema",
    ],
    "controlled provider root owner",
  );
  if (
    value.schema !== "synveda.clean-engine.provider-root-owner.v1" ||
    value.fixture_id !== rootPlan.fixture_id ||
    value.ownership_nonce !== ownershipNonce ||
    value.provider !== "colima" ||
    value.provider_contract_sha256 !== CONTROLLED_FAKE_PROVIDER_CONTRACT_SHA256 ||
    value.provider_intent_sha256 !== intentSha256 ||
    value.provider_profile !== rootPlan.provider_profile ||
    value.provider_resource !== rootPlan.provider_resource ||
    value.provider_root_plan_sha256 !== controlledProviderDigest(controlledProviderBytes(rootPlan)) ||
    canonical(value.base) !== canonical(rootPlan.base) ||
    !Array.isArray(value.roots) ||
    value.roots.length !== Object.keys(ROOT_LAYOUT).length
  ) {
    fail("controlled provider root owner was refused");
  }
  validateMetadataValue(value.root, { mode: "0700", path: rootPlan.root_path }, "provider root");
  const rootMetadata = metadataValue(rootPlan.root_path, "controlled provider root", "directory", 0o700);
  if (canonical(rootMetadata) !== canonical(value.root)) fail("controlled provider root identity changed");
  const expectedRoots = Object.entries(ROOT_LAYOUT)
    .map(([environment, leaf]) => {
      const path = join(rootPlan.root_path, leaf);
      const metadata = metadataValue(path, `controlled provider ${environment}`, "directory", 0o700);
      return { environment, relative_path: leaf, ...metadata };
    })
    .sort((left, right) => left.environment.localeCompare(right.environment));
  if (canonical(expectedRoots) !== canonical(value.roots)) fail("controlled provider roots changed");
  exactKeys(value.marker, ["device", "inode", "mode", "relative_path", "uid"], "owner marker");
  const rootEntries = readdirSync(rootPlan.root_path).sort();
  const stageName = ownerStageName(ownershipNonce);
  const linkedOwnerStage = rootEntries.includes(stageName);
  if (linkedOwnerStage && !allowLinkedOwnerStage) {
    fail("controlled provider owner stage remained linked");
  }
  const markerPath = join(rootPlan.root_path, OWNER_MARKER);
  const markerMetadata = exactMetadata(
    markerPath,
    "controlled provider owner marker",
    "file",
    0o600,
    linkedOwnerStage ? 2 : 1,
  );
  if (linkedOwnerStage) {
    const stageMetadata = exactMetadata(
      join(rootPlan.root_path, stageName),
      "controlled provider owner stage",
      "file",
      0o600,
      2,
    );
    if (
      stageMetadata.dev !== markerMetadata.dev ||
      stageMetadata.ino !== markerMetadata.ino
    ) {
      fail("controlled provider owner stage link was refused");
    }
  }
  if (
    value.marker.relative_path !== OWNER_MARKER ||
    value.marker.mode !== "0600" ||
    value.marker.device !== String(markerMetadata.dev) ||
    value.marker.inode !== String(markerMetadata.ino) ||
    value.marker.uid !== String(markerMetadata.uid)
  ) {
    fail("controlled provider owner marker identity changed");
  }
  const expectedRootEntries = [
    ...Object.values(ROOT_LAYOUT),
    OWNER_MARKER,
    ...(linkedOwnerStage ? [stageName] : []),
  ].sort();
  if (canonical(rootEntries) !== canonical(expectedRootEntries)) {
    fail("controlled provider root inventory was refused");
  }
  for (const [environment, leaf] of Object.entries(ROOT_LAYOUT)) {
    if (environment === "TMPDIR") {
      inspectFakeEffectInventory(rootPlan);
    } else if (readdirSync(join(rootPlan.root_path, leaf)).length !== 0) {
      fail("controlled provider environment root was not empty");
    }
  }
}

function inspectFakeEffectInventory(rootPlan) {
  const temporary = join(rootPlan.root_path, ROOT_LAYOUT.TMPDIR);
  const entries = readdirSync(temporary).sort();
  const stages = entries.filter((entry) => /^\.fake-effect-stage-[0-9a-f]{32}$/.test(entry));
  const hasFinal = entries.includes("fake-effect.json");
  if (
    stages.length > 1 ||
    entries.some((entry) => entry !== "fake-effect.json" && !stages.includes(entry))
  ) {
    fail("controlled provider temporary inventory was refused");
  }
  let linkedStage = false;
  for (const stageName of stages) {
    const stage = exactMetadata(
      join(temporary, stageName),
      "controlled provider effect stage",
      "file",
      0o600,
    );
    if (stage.nlink === 1n) continue;
    if (stage.nlink !== 2n || !hasFinal || linkedStage) {
      fail("controlled provider effect stage links were refused");
    }
    const final = lstatSync(join(temporary, "fake-effect.json"), { bigint: true });
    if (final.dev !== stage.dev || final.ino !== stage.ino) {
      fail("controlled provider effect stage link was refused");
    }
    linkedStage = true;
  }
  if (hasFinal && stages.length === 1 && !linkedStage) {
    fail("controlled provider effect stage was independent of the final effect");
  }
  if (hasFinal) {
    exactMetadata(
      join(temporary, "fake-effect.json"),
      "controlled provider fake effect",
      "file",
      0o600,
      linkedStage ? 2 : 1,
    );
  }
  return hasFinal ? (linkedStage ? 2 : 1) : undefined;
}

function validateWitness(value, bindings) {
  exactKeys(
    value,
    [
      "actor_nonce",
      "actor_launch_sha256",
      "actor_pgid",
      "actor_pid",
      "fixture_id",
      "provider_contract_sha256",
      "provider_intent_sha256",
      "provider_root_plan_sha256",
      "provider_root_owner_sha256",
      "scenario",
      "schema",
      "slot_sequence",
      "slot_sha256",
      "start_token_sha256",
    ],
    "controlled provider actor witness",
  );
  if (
    value.schema !== "synveda.clean-engine.provider-actor-witness.v1" ||
    value.actor_launch_sha256 !== bindings.launchSha256 ||
    value.fixture_id !== bindings.fixtureId ||
    value.provider_contract_sha256 !== CONTROLLED_FAKE_PROVIDER_CONTRACT_SHA256 ||
    value.provider_intent_sha256 !== bindings.intentSha256 ||
    value.provider_root_plan_sha256 !== bindings.rootPlanSha256 ||
    value.provider_root_owner_sha256 !== bindings.rootOwnerSha256 ||
    !PROVIDER_SCENARIOS.includes(value.scenario) ||
    value.slot_sequence !== bindings.slotSequence ||
    value.slot_sha256 !== bindings.slotSha256 ||
    !lowerHex(value.actor_nonce, 64) ||
    !lowerHex(value.start_token_sha256, 64) ||
    !decimalString(value.actor_pid) ||
    value.actor_pgid !== value.actor_pid ||
    Number(value.actor_pid) < 2
  ) {
    fail("controlled provider actor witness was refused");
  }
}

function actorLaunchValue(bindings) {
  return {
    fixture_id: bindings.fixtureId,
    provider_intent_sha256: bindings.intentSha256,
    provider_root_owner_sha256: bindings.rootOwnerSha256,
    provider_root_plan_sha256: bindings.rootPlanSha256,
    schema: "synveda.clean-engine.provider-actor-launch.v1",
    slot_sequence: bindings.slotSequence,
    slot_sha256: bindings.slotSha256,
  };
}

function validateActorLaunch(value, bindings) {
  exactKeys(
    value,
    [
      "fixture_id",
      "provider_intent_sha256",
      "provider_root_owner_sha256",
      "provider_root_plan_sha256",
      "schema",
      "slot_sequence",
      "slot_sha256",
    ],
    "controlled provider actor launch",
  );
  if (canonical(value) !== canonical(actorLaunchValue(bindings))) {
    fail("controlled provider actor launch was refused");
  }
}

function validateDecision(value, witness) {
  exactKeys(
    value,
    [
      "actor_witness_sha256",
      "decision",
      "fixture_id",
      "provider_intent_sha256",
      "provider_root_owner_sha256",
      "schema",
      "start_token_sha256",
    ],
    "controlled provider actor decision",
  );
  if (
    value.schema !== "synveda.clean-engine.provider-actor-decision.v1" ||
    !new Set(["abort", "start"]).has(value.decision) ||
    value.fixture_id !== witness.value.fixture_id ||
    value.actor_witness_sha256 !== witness.sha256 ||
    value.provider_intent_sha256 !== witness.value.provider_intent_sha256 ||
    value.provider_root_owner_sha256 !== witness.value.provider_root_owner_sha256 ||
    value.start_token_sha256 !==
      (value.decision === "start" ? witness.value.start_token_sha256 : ZERO_SHA256)
  ) {
    fail("controlled provider actor decision was refused");
  }
}

function readControlledEffect(rootPlan, witness, decision, required) {
  const effectPath = join(rootPlan.root_path, ROOT_LAYOUT.TMPDIR, "fake-effect.json");
  const expectedLinks = inspectFakeEffectInventory(rootPlan);
  if (expectedLinks === undefined) {
    if (readdirSync(join(rootPlan.root_path, ROOT_LAYOUT.TMPDIR)).length !== 0) {
      fail("controlled provider effect publication remained uncertain", 73);
    }
    if (required) fail("controlled provider fake effect was unavailable", 69);
    return undefined;
  }
  const effect = readCanonical(
    effectPath,
    "controlled provider fake effect",
    expectedLinks,
  );
  exactKeys(
    effect.value,
    [
      "environment_keys",
      "fixture_id",
      "provider_intent_sha256",
      "provider_root_owner_sha256",
      "scenario",
      "schema",
      "witness_sha256",
    ],
    "controlled provider fake effect",
  );
  const environmentKeys = [
    "COLIMA_CACHE_HOME",
    "COLIMA_HOME",
    "DOCKER_CONFIG",
    "LANG",
    "LC_ALL",
    "LIMA_HOME",
    "TMPDIR",
  ];
  if (process.platform === "darwin") environmentKeys.push("__CF_USER_TEXT_ENCODING");
  environmentKeys.sort();
  if (
    effect.value.schema !== "synveda.clean-engine.controlled-fake-effect.v1" ||
    effect.value.fixture_id !== witness.value.fixture_id ||
    effect.value.provider_intent_sha256 !== witness.value.provider_intent_sha256 ||
    effect.value.provider_root_owner_sha256 !== witness.value.provider_root_owner_sha256 ||
    effect.value.scenario !== witness.value.scenario ||
    effect.value.witness_sha256 !== witness.sha256 ||
    canonical(effect.value.environment_keys) !== canonical(environmentKeys) ||
    decision.value.decision !== "start"
  ) {
    fail("controlled provider fake effect was refused");
  }
  return effect;
}

function validateEffect(rootPlan, outcome, witness, decision) {
  const effect = readControlledEffect(rootPlan, witness, decision, true);
  if (outcome.value.effect_sha256 !== effect.sha256) {
    fail("controlled provider fake effect was refused");
  }
  return effect;
}

function validateOutcome(value, witness, decision) {
  exactKeys(
    value,
    [
      "actor_decision_sha256",
      "actor_witness_sha256",
      "child_exit_code",
      "child_signal",
      "effect_sha256",
      "fixture_id",
      "outcome",
      "safe_code",
      "schema",
    ],
    "controlled provider actor outcome",
  );
  if (
    value.schema !== "synveda.clean-engine.provider-actor-outcome.v1" ||
    value.fixture_id !== witness.value.fixture_id ||
    value.actor_witness_sha256 !== witness.sha256 ||
    value.actor_decision_sha256 !== decision.sha256 ||
    !new Set(["failed", "passed"]).has(value.outcome) ||
    !new Set(["child-failed", "none"]).has(value.safe_code) ||
    !lowerHex(value.effect_sha256, 64) ||
    !(value.child_exit_code === null || Number.isSafeInteger(value.child_exit_code)) ||
    !(value.child_signal === null || typeof value.child_signal === "string") ||
    (value.outcome === "passed" &&
      (value.safe_code !== "none" || value.child_exit_code !== 0 || value.child_signal !== null)) ||
    (value.outcome === "failed" &&
      (value.safe_code !== "child-failed" ||
        (value.child_exit_code === 0 && value.child_signal === null)))
  ) {
    fail("controlled provider actor outcome was refused");
  }
}

function validateSettlement(value, witness, decision, outcome, effect) {
  exactKeys(
    value,
    [
      "actor_decision_sha256",
      "actor_effect_sha256",
      "actor_outcome_sha256",
      "actor_pgid",
      "actor_witness_sha256",
      "disposition",
      "fixture_id",
      "group_absent",
      "group_probe",
      "schema",
      "termination_reason",
    ],
    "controlled provider actor settlement",
  );
  const baseValid = !(
    value.schema !== "synveda.clean-engine.provider-actor-settlement.v1" ||
    value.fixture_id !== witness.value.fixture_id ||
    value.actor_witness_sha256 !== witness.sha256 ||
    value.actor_decision_sha256 !== decision.sha256 ||
    !lowerHex(value.actor_effect_sha256, 64) ||
    value.actor_effect_sha256 !== (effect?.sha256 ?? ZERO_SHA256) ||
    value.actor_outcome_sha256 !== (outcome?.sha256 ?? ZERO_SHA256) ||
    value.actor_pgid !== witness.value.actor_pgid ||
    !new Set(["aborted", "completed", "terminated"]).has(value.disposition) ||
    !new Set([
      "actor-exited",
      "deadline",
      "descendant-remained",
      "gate-refused",
      "none",
      "recovered",
    ]).has(
      value.termination_reason,
    ) ||
    value.group_absent !== true ||
    value.group_probe !== "esrch"
  );
  let semanticValid = false;
  if (decision.value.decision === "abort") {
    semanticValid =
      outcome === undefined &&
      value.actor_effect_sha256 === ZERO_SHA256 &&
      value.disposition === "aborted" &&
      new Set(["none", "recovered"]).has(value.termination_reason);
  } else if (value.disposition === "completed") {
    semanticValid =
      outcome !== undefined &&
      witness.value.scenario !== "orphan" &&
      new Set(["none", "recovered"]).has(value.termination_reason);
  } else if (value.disposition === "terminated") {
    semanticValid =
      (value.termination_reason === "deadline" && outcome === undefined) ||
      (value.termination_reason === "descendant-remained" &&
        witness.value.scenario === "orphan" && outcome !== undefined) ||
      (new Set(["actor-exited", "gate-refused"]).has(value.termination_reason) &&
        outcome === undefined) ||
      (value.termination_reason === "recovered" &&
        (outcome === undefined || witness.value.scenario === "orphan"));
  }
  if (!baseValid || !semanticValid) {
    fail("controlled provider actor settlement was refused");
  }
}

function controlledFailureCode(result, label) {
  exactKeys(
    result,
    ["cleanup_required", "collision_resource", "resource_disposition", "safe_code"],
    label,
  );
  const collision =
    result.cleanup_required === true &&
    result.collision_resource === "provider" &&
    result.resource_disposition === "foreign-preserved" &&
    result.safe_code === "resource-collision";
  const nonCollision =
    result.cleanup_required === true &&
    result.collision_resource === "none" &&
    result.resource_disposition === "receipt-owned-or-absent" &&
    new Set(["child-failed", "child-timeout", "evidence-refused"]).has(result.safe_code);
  if (!collision && !nonCollision) fail(`${label} was refused`);
  return result.safe_code;
}

function actorFailureCode(settlement, outcome) {
  if (settlement.value.termination_reason === "deadline") return "child-timeout";
  if (settlement.value.disposition === "completed" && outcome?.value.outcome === "failed") {
    return "child-failed";
  }
  return "evidence-refused";
}

export function validateControlledProviderArtifactChain({
  artifacts,
  close,
  fixtureId,
  intentReceipt,
  slot,
  terminalReceipt,
}) {
  const names = Object.keys(artifacts);
  if (names.length === 0) {
    if (close !== undefined && close.value.operation_evidence_sha256 !== ZERO_SHA256) {
      fail("empty provider state carried operation evidence");
    }
    return Object.freeze({ contract: "synchronous-fake", operationEvidenceSha256: ZERO_SHA256 });
  }
  const rootPlanArtifact = requireArtifact(artifacts, "root-plan.json");
  validateRootPlan(rootPlanArtifact.value, fixtureId);
  if (slot === undefined) fail("controlled provider root plan lacked a mutation slot");
  const reconstructedIntent = controlledProviderIntendedReceipt(
    fixtureId,
    rootPlanArtifact.value,
    slot.value.source_head_sha256,
    slot.value.source_sequence,
  );
  const reconstructedIntentBytes = controlledProviderBytes(reconstructedIntent);
  const reconstructedIntentSha256 = controlledProviderDigest(reconstructedIntentBytes);
  if (slot.value.intent_receipt_sha256 !== reconstructedIntentSha256) {
    fail("controlled provider root plan was outside its intended receipt");
  }
  const intent = intentReceipt?.phase === "provider-create-intent" ? intentReceipt : undefined;
  if (
    intent !== undefined &&
    !controlledProviderBytes(intent).equals(reconstructedIntentBytes)
  ) {
    fail("controlled provider intent binding was refused");
  }
  const allowedPrefix = [
    "root-plan.json",
    "root-reservation.json",
    "root-owner.json",
    "actor-launch.json",
    "actor-witness.json",
    "actor-decision.json",
  ];
  for (let index = 1; index < allowedPrefix.length; index += 1) {
    if (artifacts[allowedPrefix[index]] !== undefined && artifacts[allowedPrefix[index - 1]] === undefined) {
      fail("controlled provider artifact chain had a gap");
    }
  }
  let operationEvidenceSha256 = rootPlanArtifact.sha256;
  if (intent !== undefined) {
    const intentSha256 = reconstructedIntentSha256;
    if (
      intent.result.provider_root_plan_sha256 !== rootPlanArtifact.sha256 ||
      intent.result.provider_profile !== rootPlanArtifact.value.provider_profile ||
      slot.value.intent_receipt_sha256 !== intentSha256
    ) {
      fail("controlled provider intent binding was refused");
    }
    const reservation = artifacts["root-reservation.json"];
    if (reservation !== undefined) {
      validateReservation(
        reservation.value,
        rootPlanArtifact.value,
        intentSha256,
        intent.result.ownership_nonce,
      );
      operationEvidenceSha256 = reservation.sha256;
    }
    const rootOwner = artifacts["root-owner.json"];
    if (rootOwner !== undefined) {
      if (reservation === undefined) fail("controlled provider root owner lacked a reservation");
      assertRootOwner(
        rootOwner.value,
        rootPlanArtifact.value,
        intentSha256,
        intent.result.ownership_nonce,
      );
      const externalMarker = readCanonical(
        join(rootPlanArtifact.value.root_path, OWNER_MARKER),
        "controlled provider owner marker",
      );
      if (!externalMarker.bytes.equals(rootOwner.bytes)) {
        fail("controlled provider owner mirror was refused");
      }
      operationEvidenceSha256 = rootOwner.sha256;
    }
    const launch = artifacts["actor-launch.json"];
    if (launch !== undefined) {
      if (rootOwner === undefined) fail("controlled provider actor launch lacked root authority");
      validateActorLaunch(launch.value, {
        fixtureId,
        intentSha256,
        rootPlanSha256: rootPlanArtifact.sha256,
        rootOwnerSha256: rootOwner.sha256,
        slotSequence: slot.value.journal_sequence,
        slotSha256: controlledProviderDigest(slot.bytes),
      });
      operationEvidenceSha256 = launch.sha256;
    }
    const witness = artifacts["actor-witness.json"];
    if (witness !== undefined) {
      if (launch === undefined) fail("controlled provider actor lacked launch authority");
      validateWitness(witness.value, {
        fixtureId,
        intentSha256,
        launchSha256: launch.sha256,
        rootPlanSha256: rootPlanArtifact.sha256,
        rootOwnerSha256: rootOwner.sha256,
        slotSequence: slot.value.journal_sequence,
        slotSha256: controlledProviderDigest(slot.bytes),
      });
      operationEvidenceSha256 = witness.sha256;
    }
    const decision = artifacts["actor-decision.json"];
    if (decision !== undefined) {
      if (witness === undefined) fail("controlled provider decision lacked a witness");
      validateDecision(decision.value, witness);
      operationEvidenceSha256 = decision.sha256;
    }
    const outcome = artifacts["actor-outcome.json"];
    let effect;
    if (outcome !== undefined) {
      if (decision === undefined || decision.value.decision !== "start") {
        fail("controlled provider outcome lacked a start decision");
      }
      validateOutcome(outcome.value, witness, decision);
      effect = validateEffect(rootPlanArtifact.value, outcome, witness, decision);
      operationEvidenceSha256 = outcome.sha256;
    }
    const settlement = artifacts["actor-settlement.json"];
    if (settlement !== undefined) {
      if (decision === undefined) fail("controlled provider settlement lacked a decision");
      if (effect === undefined) {
        effect = readControlledEffect(rootPlanArtifact.value, witness, decision, false);
      }
      validateSettlement(settlement.value, witness, decision, outcome, effect);
      operationEvidenceSha256 = settlement.sha256;
    }
  }
  if (intent === undefined && names.some((name) => name !== "root-plan.json")) {
    fail("controlled provider artifacts were outside their intent");
  }
  const reservation = artifacts["root-reservation.json"];
  const rootOwner = artifacts["root-owner.json"];
  const launch = artifacts["actor-launch.json"];
  const witness = artifacts["actor-witness.json"];
  const decision = artifacts["actor-decision.json"];
  const outcome = artifacts["actor-outcome.json"];
  const settlement = artifacts["actor-settlement.json"];
  if (terminalReceipt?.phase === "preflight-refused") {
    exactKeys(
      terminalReceipt.result,
      ["cleanup_required", "collision_resource", "resource_disposition", "safe_code"],
      "controlled provider preflight result",
    );
    if (
      intent !== undefined ||
      names.length !== 1 ||
      terminalReceipt.result.cleanup_required !== false ||
      terminalReceipt.result.collision_resource !== "provider" ||
      terminalReceipt.result.resource_disposition !== "foreign-preserved" ||
      terminalReceipt.result.safe_code !== "resource-collision" ||
      operationEvidenceSha256 !== rootPlanArtifact.sha256
    ) {
      fail("controlled provider preflight terminal evidence was refused");
    }
  } else if (terminalReceipt?.phase === "provider-create-passed") {
    if (
      intent === undefined ||
      reservation === undefined ||
      rootOwner === undefined ||
      launch === undefined ||
      witness === undefined ||
      decision?.value.decision !== "start" ||
      outcome?.value.outcome !== "passed" ||
      settlement?.value.disposition !== "completed" ||
      terminalReceipt.result?.engine_identity_sha256 !== settlement.sha256 ||
      operationEvidenceSha256 !== settlement.sha256
    ) {
      fail("controlled provider passing terminal evidence was refused");
    }
  } else if (terminalReceipt?.phase === "provider-create-failed") {
    const failureCode = controlledFailureCode(
      terminalReceipt.result,
      "controlled provider failure result",
    );
    const preReservationFailure =
      reservation === undefined &&
      rootOwner === undefined &&
      launch === undefined &&
      witness === undefined &&
      decision === undefined &&
      outcome === undefined &&
      settlement === undefined &&
      operationEvidenceSha256 === rootPlanArtifact.sha256;
    const preRootFailure =
      reservation !== undefined &&
      rootOwner === undefined &&
      launch === undefined &&
      witness === undefined &&
      decision === undefined &&
      outcome === undefined &&
      settlement === undefined &&
      operationEvidenceSha256 === reservation.sha256;
    const preLaunchFailure =
      reservation !== undefined &&
      rootOwner !== undefined &&
      launch === undefined &&
      witness === undefined &&
      decision === undefined &&
      outcome === undefined &&
      settlement === undefined &&
      operationEvidenceSha256 === rootOwner.sha256;
    const actorFailure =
      settlement !== undefined && operationEvidenceSha256 === settlement.sha256;
    const pathValid =
      (preReservationFailure &&
        failureCode === "evidence-refused") ||
      (preRootFailure &&
        new Set(["evidence-refused", "resource-collision"]).has(failureCode)) ||
      (preLaunchFailure && failureCode === "evidence-refused") ||
      (actorFailure &&
        new Set([actorFailureCode(settlement, outcome), "evidence-refused"]).has(failureCode));
    if (intent === undefined || !pathValid) {
      fail("controlled provider failing terminal evidence was refused");
    }
  } else if (terminalReceipt?.phase === "execution-failed") {
    const failureCode = controlledFailureCode(
      terminalReceipt.result,
      "controlled provider execution failure result",
    );
    const intentDelta = terminalReceipt.sequence - (intent?.sequence ?? -1);
    const preEffectFailure =
      intentDelta === 1 &&
      reservation === undefined &&
      rootOwner === undefined &&
      launch === undefined &&
      witness === undefined &&
      decision === undefined &&
      outcome === undefined &&
      settlement === undefined &&
      operationEvidenceSha256 === rootPlanArtifact.sha256;
    const postSettlementFailure =
      intentDelta === 2 &&
      settlement !== undefined &&
      settlement.value.disposition === "completed" &&
      outcome?.value.outcome === "passed" &&
      operationEvidenceSha256 === settlement.sha256;
    if (
      intent === undefined ||
      failureCode !== "evidence-refused" ||
      (!preEffectFailure && !postSettlementFailure)
    ) {
      fail("controlled provider execution failure evidence was refused");
    }
  }
  if (close !== undefined && close.value.operation_evidence_sha256 !== operationEvidenceSha256) {
    fail("controlled provider close evidence was refused");
  }
  return Object.freeze({
    contract: "controlled-fake",
    operationEvidenceSha256,
    rootPlan: rootPlanArtifact,
    reservation,
    rootOwner,
    launch,
    witness,
    decision,
    outcome,
    settlement,
  });
}

export function probeControlledProcessGroup(pgid) {
  if (!Number.isSafeInteger(pgid) || pgid < 2) return "unknown";
  try {
    process.kill(-pgid, 0);
    return "present";
  } catch (error) {
    if (error?.code === "ESRCH") return "absent";
    return "unknown";
  }
}

function delay(milliseconds) {
  return new Promise((resolvePromise) => setTimeout(resolvePromise, milliseconds));
}

async function waitForControlledGroupAbsence(pgid, milliseconds) {
  const deadline = Date.now() + milliseconds;
  while (Date.now() <= deadline) {
    if (probeControlledProcessGroup(pgid) === "absent") return true;
    await delay(10);
  }
  return probeControlledProcessGroup(pgid) === "absent";
}

async function waitForClose(closed, timeoutValue, milliseconds) {
  let timer;
  const timed = new Promise((resolvePromise) => {
    timer = setTimeout(() => resolvePromise(timeoutValue), milliseconds);
  });
  try {
    return await Promise.race([closed, timed]);
  } finally {
    clearTimeout(timer);
  }
}

function actorEnvironment() {
  return Object.freeze({ LANG: "C", LC_ALL: "C" });
}

function actorArgs(bindings, adapter) {
  return [
    "actor",
    bindings.providerDirectory,
    bindings.rootPath,
    bindings.fixtureId,
    bindings.intentSha256,
    bindings.rootOwnerSha256,
    bindings.rootPlanSha256,
    bindings.launchSha256,
    bindings.slotSha256,
    String(bindings.slotSequence),
    adapter.child_scenario,
    String(adapter.after_outcome_publish_hold_milliseconds),
    String(adapter.deadline_milliseconds),
    String(adapter.term_grace_milliseconds),
  ];
}

export async function launchControlledFakeActor(bindings, adapter) {
  validateControlledProviderAdapter(adapter);
  if (
    !isAbsolute(bindings.providerDirectory) ||
    !isAbsolute(bindings.rootPath) ||
    !lowerHex(bindings.fixtureId, 32) ||
    !lowerHex(bindings.intentSha256, 64) ||
    !lowerHex(bindings.rootOwnerSha256, 64) ||
    !lowerHex(bindings.rootPlanSha256, 64) ||
    !lowerHex(bindings.slotSha256, 64) ||
    !Number.isSafeInteger(bindings.slotSequence) ||
    bindings.slotSequence < 0
  ) {
    fail("controlled provider actor bindings were refused", 64);
  }
  const launch = publishControlledProviderArtifact(
    bindings.providerDirectory,
    "actor-launch.json",
    actorLaunchValue(bindings),
  );
  bindings = Object.freeze({ ...bindings, launchSha256: launch.sha256 });
  const rawStartToken = randomBytes(32).toString("hex");
  const startTokenSha256 = controlledProviderDigest(Buffer.from(rawStartToken, "utf8"));
  const deadlineAt = Date.now() + adapter.deadline_milliseconds;
  const actor = spawn(process.execPath, [ACTOR_SCRIPT, ...actorArgs(bindings, adapter)], {
    cwd: bindings.rootPath,
    detached: true,
    env: actorEnvironment(),
    stdio: ["ignore", "ignore", "ignore", "ipc"],
  });
  const closed = new Promise((resolvePromise) => {
    actor.once("error", () => resolvePromise({ code: null, signal: "spawn-error" }));
    actor.once("close", (code, signal) => resolvePromise({ code, signal }));
  });
  let actorReportedReason;
  const outcomeReady = new Promise((resolvePromise) => {
    actor.on("message", (message) => {
      if (
        message?.type === "outcome-ready" &&
        lowerHex(message.outcome_sha256, 64)
      ) {
        resolvePromise(message.outcome_sha256);
      }
      if (message?.type === "gate-refused") actorReportedReason = "gate-refused";
    });
  });
  const ready = await new Promise((resolvePromise, rejectPromise) => {
    const timeout = setTimeout(
      () => rejectPromise(new ControlledProviderFailure("controlled provider actor readiness expired", 73)),
      Math.min(adapter.deadline_milliseconds, 5_000),
    );
    actor.once("message", (message) => {
      clearTimeout(timeout);
      resolvePromise(message);
    });
    actor.once("error", () => {
      clearTimeout(timeout);
      rejectPromise(new ControlledProviderFailure("controlled provider actor failed to start", 70));
    });
    actor.once("close", (code, signal) => {
      clearTimeout(timeout);
      rejectPromise(
        new ControlledProviderFailure(
          `controlled provider actor closed before readiness (${code ?? "signal"}:${signal ?? "none"})`,
          70,
        ),
      );
    });
  }).catch(async (error) => {
    if (Number.isSafeInteger(actor.pid)) {
      actor.kill("SIGKILL");
      await closed;
    }
    throw error;
  });
  exactKeys(ready, ["actor_nonce", "actor_pgid", "actor_pid", "schema"], "actor readiness");
  if (
    ready.schema !== "synveda.clean-engine.provider-actor-ready.v1" ||
    !lowerHex(ready.actor_nonce, 64) ||
    ready.actor_pid !== String(actor.pid) ||
    ready.actor_pgid !== String(actor.pid) ||
    probeControlledProcessGroup(actor.pid) !== "present"
  ) {
    actor.kill("SIGKILL");
    await closed;
    fail("controlled provider actor readiness was refused", 70);
  }
  try {
    await delay(adapter.before_witness_hold_milliseconds);
    const durableRootPlan = readCanonical(
      join(bindings.providerDirectory, "root-plan.json"),
      "controlled provider root plan",
    );
    const durableRootOwner = readCanonical(
      join(bindings.providerDirectory, "root-owner.json"),
      "controlled provider root owner",
    );
    validateRootPlan(durableRootPlan.value, bindings.fixtureId);
    assertRootOwner(
      durableRootOwner.value,
      durableRootPlan.value,
      bindings.intentSha256,
      durableRootOwner.value.ownership_nonce,
    );
    if (
      actor.exitCode !== null ||
      actor.signalCode !== null ||
      !actor.connected ||
      durableRootPlan.value.root_path !== bindings.rootPath ||
      durableRootPlan.sha256 !== bindings.rootPlanSha256 ||
      durableRootOwner.sha256 !== bindings.rootOwnerSha256 ||
      probeControlledProcessGroup(actor.pid) !== "present"
    ) {
      fail("controlled provider actor witness gate was refused", 73);
    }
  } catch (error) {
    if (actor.exitCode === null && actor.signalCode === null) actor.kill("SIGKILL");
    await closed;
    throw error;
  }
  const witnessValue = {
    actor_nonce: ready.actor_nonce,
    actor_launch_sha256: launch.sha256,
    actor_pgid: ready.actor_pgid,
    actor_pid: ready.actor_pid,
    fixture_id: bindings.fixtureId,
    provider_contract_sha256: CONTROLLED_FAKE_PROVIDER_CONTRACT_SHA256,
    provider_intent_sha256: bindings.intentSha256,
    provider_root_plan_sha256: bindings.rootPlanSha256,
    provider_root_owner_sha256: bindings.rootOwnerSha256,
    scenario: adapter.child_scenario,
    schema: "synveda.clean-engine.provider-actor-witness.v1",
    slot_sequence: bindings.slotSequence,
    slot_sha256: bindings.slotSha256,
    start_token_sha256: startTokenSha256,
  };
  const witness = publishControlledProviderArtifact(
    bindings.providerDirectory,
    "actor-witness.json",
    witnessValue,
  );
  let decision;
  let settled = false;
  const publishDecision = (choice) => {
    if (decision !== undefined) fail("controlled provider actor decision was already published", 73);
    const value = {
      actor_witness_sha256: witness.sha256,
      decision: choice,
      fixture_id: bindings.fixtureId,
      provider_intent_sha256: bindings.intentSha256,
      provider_root_owner_sha256: bindings.rootOwnerSha256,
      schema: "synveda.clean-engine.provider-actor-decision.v1",
      start_token_sha256: choice === "start" ? startTokenSha256 : ZERO_SHA256,
    };
    decision = publishControlledProviderArtifact(
      bindings.providerDirectory,
      "actor-decision.json",
      value,
    );
    if (actor.connected) {
      if (choice === "start") {
        const token =
          adapter.gate_delivery === "wrong"
            ? `${rawStartToken.slice(0, -1)}${rawStartToken.endsWith("0") ? "1" : "0"}`
            : rawStartToken;
        const message = { decision_sha256: decision.sha256, token, type: "start" };
        actor.send(message);
        if (adapter.gate_delivery === "duplicate") actor.send(message);
      } else {
        actor.send({ decision_sha256: decision.sha256, type: "abort" });
      }
    }
    return decision;
  };
  const settle = async () => {
    if (settled) fail("controlled provider actor was already settled", 73);
    if (decision === undefined) fail("controlled provider actor lacked a decision", 73);
    settled = true;
    let closeState;
    const remaining = Math.max(0, deadlineAt - Date.now());
    const deadline = Symbol("deadline");
    let deadlineTimer;
    const deadlinePromise = new Promise((resolvePromise) => {
      deadlineTimer = setTimeout(() => resolvePromise(deadline), remaining);
    });
    const observed = await Promise.race([
      closed,
      outcomeReady,
      deadlinePromise,
    ]);
    clearTimeout(deadlineTimer);
    if (
      (observed === deadline || typeof observed === "string") &&
      actor.exitCode === null &&
      actor.signalCode === null &&
      actor.connected
    ) {
      actor.send({ type: "terminate" });
      const closeDeadline = Symbol("close-deadline");
      const closedAfterTermination = await waitForClose(
        closed,
        closeDeadline,
        adapter.term_grace_milliseconds + adapter.kill_grace_milliseconds + 500,
      );
      if (closedAfterTermination === closeDeadline) {
        fail("controlled provider actor did not terminate its group", 73);
      }
      closeState = await closed;
    } else {
      closeState = typeof observed === "object" ? observed : await closed;
    }
    if (
      !(await waitForControlledGroupAbsence(
        actor.pid,
        adapter.term_grace_milliseconds + adapter.kill_grace_milliseconds + 500,
      ))
    ) {
      fail("controlled provider actor absence was not proved", 73);
    }
    const artifacts = inspectControlledProviderArtifacts(bindings.providerDirectory);
    const outcome = artifacts["actor-outcome.json"];
    if (typeof observed === "string" && outcome?.sha256 !== observed) {
      fail("controlled provider actor outcome acknowledgement was refused", 73);
    }
    const rootPlan = readCanonical(
      join(bindings.providerDirectory, "root-plan.json"),
      "controlled provider root plan",
    );
    let effect;
    if (outcome === undefined) {
      effect = readControlledEffect(rootPlan.value, witness, decision, false);
    } else {
      validateOutcome(outcome.value, witness, decision);
      effect = validateEffect(rootPlan.value, outcome, witness, decision);
    }
    let terminationReason;
    if (decision.value.decision === "abort") {
      terminationReason = "none";
    } else if (outcome !== undefined) {
      terminationReason =
        adapter.child_scenario === "orphan" ? "descendant-remained" : "none";
    } else if (observed === deadline) {
      terminationReason = "deadline";
    } else {
      terminationReason = actorReportedReason ?? "actor-exited";
    }
    let disposition = decision.value.decision === "abort" ? "aborted" : "completed";
    if (terminationReason !== "none" || outcome === undefined) disposition = "terminated";
    if (decision.value.decision === "abort") disposition = "aborted";
    const settlementValue = {
      actor_decision_sha256: decision.sha256,
      actor_effect_sha256: effect?.sha256 ?? ZERO_SHA256,
      actor_outcome_sha256: outcome?.sha256 ?? ZERO_SHA256,
      actor_pgid: String(actor.pid),
      actor_witness_sha256: witness.sha256,
      disposition,
      fixture_id: bindings.fixtureId,
      group_absent: true,
      group_probe: "esrch",
      schema: "synveda.clean-engine.provider-actor-settlement.v1",
      termination_reason: terminationReason,
    };
    const settlement = publishControlledProviderArtifact(
      bindings.providerDirectory,
      "actor-settlement.json",
      settlementValue,
    );
    return Object.freeze({ closeState, decision, outcome, settlement, witness });
  };
  return Object.freeze({
    abort: () => publishDecision("abort"),
    actorPid: actor.pid,
    settle,
    start: () => publishDecision("start"),
    witness,
  });
}

export function publishRecoveredAbort(providerDirectory, witness) {
  const value = {
    actor_witness_sha256: witness.sha256,
    decision: "abort",
    fixture_id: witness.value.fixture_id,
    provider_intent_sha256: witness.value.provider_intent_sha256,
    provider_root_owner_sha256: witness.value.provider_root_owner_sha256,
    schema: "synveda.clean-engine.provider-actor-decision.v1",
    start_token_sha256: ZERO_SHA256,
  };
  return publishControlledProviderArtifact(providerDirectory, "actor-decision.json", value);
}

export function publishRecoveredSettlement(
  providerDirectory,
  witness,
  decision,
  outcome,
  rootPlan,
) {
  if (probeControlledProcessGroup(Number(witness.value.actor_pgid)) !== "absent") {
    fail("controlled provider actor group remained present or unknown", 73);
  }
  let effect;
  if (outcome === undefined) {
    effect = readControlledEffect(rootPlan.value, witness, decision, false);
  } else {
    validateOutcome(outcome.value, witness, decision);
    effect = validateEffect(rootPlan.value, outcome, witness, decision);
  }
  const value = {
    actor_decision_sha256: decision.sha256,
    actor_effect_sha256: effect?.sha256 ?? ZERO_SHA256,
    actor_outcome_sha256: outcome?.sha256 ?? ZERO_SHA256,
    actor_pgid: witness.value.actor_pgid,
    actor_witness_sha256: witness.sha256,
    disposition:
      decision.value.decision === "abort"
        ? "aborted"
        : outcome === undefined || witness.value.scenario === "orphan"
          ? "terminated"
          : "completed",
    fixture_id: witness.value.fixture_id,
    group_absent: true,
    group_probe: "esrch",
    schema: "synveda.clean-engine.provider-actor-settlement.v1",
    termination_reason: "recovered",
  };
  return publishControlledProviderArtifact(providerDirectory, "actor-settlement.json", value);
}

function parseActorArguments(argv) {
  const [
    mode,
    providerDirectory,
    rootPath,
    fixtureId,
    intentSha256,
    rootOwnerSha256,
    rootPlanSha256,
    launchSha256,
    slotSha256,
    slotSequenceText,
    scenario,
    afterOutcomePublishHoldText,
    deadlineText,
    termGraceText,
  ] = argv.slice(2);
  const slotSequence = Number(slotSequenceText);
  const afterOutcomePublishHoldMilliseconds = Number(afterOutcomePublishHoldText);
  const deadlineMilliseconds = Number(deadlineText);
  const termGraceMilliseconds = Number(termGraceText);
  if (
    mode !== "actor" ||
    !isAbsolute(providerDirectory ?? "") ||
    !isAbsolute(rootPath ?? "") ||
    !lowerHex(fixtureId, 32) ||
    !lowerHex(intentSha256, 64) ||
    !lowerHex(rootOwnerSha256, 64) ||
    !lowerHex(rootPlanSha256, 64) ||
    !lowerHex(launchSha256, 64) ||
    !lowerHex(slotSha256, 64) ||
    !Number.isSafeInteger(slotSequence) ||
    slotSequence < 0 ||
    !PROVIDER_SCENARIOS.includes(scenario) ||
    !Number.isSafeInteger(afterOutcomePublishHoldMilliseconds) ||
    afterOutcomePublishHoldMilliseconds < 0 ||
    afterOutcomePublishHoldMilliseconds > CONTROLLED_FAKE_PROVIDER_CONTRACT.max_hold_milliseconds ||
    !Number.isSafeInteger(deadlineMilliseconds) ||
    deadlineMilliseconds < 100 ||
    deadlineMilliseconds > CONTROLLED_FAKE_PROVIDER_CONTRACT.max_deadline_milliseconds ||
    !Number.isSafeInteger(termGraceMilliseconds) ||
    termGraceMilliseconds < 10 ||
    termGraceMilliseconds > CONTROLLED_FAKE_PROVIDER_CONTRACT.max_term_grace_milliseconds
  ) {
    fail("controlled provider actor arguments were refused", 64);
  }
  return {
    afterOutcomePublishHoldMilliseconds,
    deadlineMilliseconds,
    fixtureId,
    intentSha256,
    providerDirectory,
    rootOwnerSha256,
    rootPlanSha256,
    launchSha256,
    rootPath,
    scenario,
    slotSequence,
    slotSha256,
    termGraceMilliseconds,
  };
}

function actorOutcomeValue(argumentsValue, decisionSha256, witnessSha256, childState) {
  const effect = readCanonical(
    join(argumentsValue.rootPath, ROOT_LAYOUT.TMPDIR, "fake-effect.json"),
    "controlled provider fake effect",
  );
  const passed = childState.code === 0 && childState.signal === null;
  return {
    actor_decision_sha256: decisionSha256,
    actor_witness_sha256: witnessSha256,
    child_exit_code: childState.code,
    child_signal: childState.signal,
    effect_sha256: effect.sha256,
    fixture_id: argumentsValue.fixtureId,
    outcome: passed ? "passed" : "failed",
    safe_code: passed ? "none" : "child-failed",
    schema: "synveda.clean-engine.provider-actor-outcome.v1",
  };
}

async function actorMain() {
  const argumentsValue = parseActorArguments(process.argv);
  process.umask(0o077);
  if (typeof process.send !== "function" || process.pid < 2) {
    fail("controlled provider actor IPC was unavailable", 70);
  }
  const actorNonce = randomBytes(32).toString("hex");
  let child;
  let descendant;
  let started = false;
  let terminating = false;
  const childStopped = (processValue) =>
    processValue === undefined ||
    processValue.exitCode !== null ||
    processValue.signalCode !== null;
  const maybeExitAfterTermination = () => {
    if (terminating && childStopped(child) && childStopped(descendant)) process.exit(75);
  };
  const terminateOwnGroup = () => {
    if (terminating) return;
    terminating = true;
    try {
      process.kill(-process.pid, "SIGTERM");
    } catch {
      process.exit(75);
    }
    setTimeout(() => {
      try {
        process.kill(-process.pid, "SIGKILL");
      } catch {
        process.exit(75);
      }
    }, argumentsValue.termGraceMilliseconds + 100);
    maybeExitAfterTermination();
  };
  process.on("SIGTERM", terminateOwnGroup);
  process.on("SIGHUP", terminateOwnGroup);
  process.on("SIGINT", terminateOwnGroup);
  process.on("disconnect", () => {
    if (started || child !== undefined || descendant !== undefined) terminateOwnGroup();
    else process.exit(75);
  });
  setTimeout(terminateOwnGroup, argumentsValue.deadlineMilliseconds).unref();
  process.send({
    actor_nonce: actorNonce,
    actor_pgid: String(process.pid),
    actor_pid: String(process.pid),
    schema: "synveda.clean-engine.provider-actor-ready.v1",
  });
  process.on("message", (message) => {
    if (message !== null && typeof message === "object" && message.type === "terminate") {
      if (started) terminateOwnGroup();
      else process.exit(75);
      return;
    }
    if (terminating) return;
    if (started || message === null || typeof message !== "object") return;
    if (message.type === "abort") {
      process.disconnect();
      process.exit(75);
    }
    if (
      message.type !== "start" ||
      !lowerHex(message.token, 64) ||
      !lowerHex(message.decision_sha256, 64)
    ) {
      process.send?.({ type: "gate-refused" });
      process.disconnect();
      process.exit(75);
    }
    const decision = readCanonical(
      join(argumentsValue.providerDirectory, "actor-decision.json"),
      "controlled provider actor decision",
    );
    const witness = readCanonical(
      join(argumentsValue.providerDirectory, "actor-witness.json"),
      "controlled provider actor witness",
    );
    const rootOwner = readCanonical(
      join(argumentsValue.providerDirectory, "root-owner.json"),
      "controlled provider root owner",
    );
    const externalOwner = readCanonical(
      join(argumentsValue.rootPath, OWNER_MARKER),
      "controlled provider external root owner",
    );
    const rootPlan = readCanonical(
      join(argumentsValue.providerDirectory, "root-plan.json"),
      "controlled provider root plan",
    );
    const launch = readCanonical(
      join(argumentsValue.providerDirectory, "actor-launch.json"),
      "controlled provider actor launch",
    );
    validateRootPlan(rootPlan.value, argumentsValue.fixtureId);
    assertRootOwner(
      rootOwner.value,
      rootPlan.value,
      argumentsValue.intentSha256,
      rootOwner.value.ownership_nonce,
    );
    validateActorLaunch(launch.value, {
      fixtureId: argumentsValue.fixtureId,
      intentSha256: argumentsValue.intentSha256,
      rootOwnerSha256: argumentsValue.rootOwnerSha256,
      rootPlanSha256: argumentsValue.rootPlanSha256,
      slotSequence: argumentsValue.slotSequence,
      slotSha256: argumentsValue.slotSha256,
    });
    const gateChecks = [
      decision.sha256 === message.decision_sha256,
      decision.value.decision === "start",
      decision.value.actor_witness_sha256 === witness.sha256,
      decision.value.fixture_id === argumentsValue.fixtureId,
      decision.value.provider_intent_sha256 === argumentsValue.intentSha256,
      decision.value.provider_root_owner_sha256 === argumentsValue.rootOwnerSha256,
      decision.value.start_token_sha256 ===
        controlledProviderDigest(Buffer.from(message.token, "utf8")),
      witness.value.actor_nonce === actorNonce,
      witness.value.actor_launch_sha256 === argumentsValue.launchSha256,
      witness.value.actor_pid === String(process.pid),
      witness.value.slot_sha256 === argumentsValue.slotSha256,
      witness.value.slot_sequence === argumentsValue.slotSequence,
      argumentsValue.rootPath === rootPlan.value.root_path,
      rootOwner.sha256 === argumentsValue.rootOwnerSha256,
      rootPlan.sha256 === argumentsValue.rootPlanSha256,
      launch.sha256 === argumentsValue.launchSha256,
      witness.value.provider_root_plan_sha256 === argumentsValue.rootPlanSha256,
      rootOwner.bytes.equals(externalOwner.bytes),
    ];
    if (gateChecks.includes(false)) {
      process.send?.({ type: "gate-refused" });
      process.disconnect();
      process.exit(75);
    }
    started = true;
    const childEnv = {
      COLIMA_CACHE_HOME: join(argumentsValue.rootPath, ROOT_LAYOUT.COLIMA_CACHE_HOME),
      COLIMA_HOME: join(argumentsValue.rootPath, ROOT_LAYOUT.COLIMA_HOME),
      DOCKER_CONFIG: join(argumentsValue.rootPath, ROOT_LAYOUT.DOCKER_CONFIG),
      LANG: "C",
      LC_ALL: "C",
      LIMA_HOME: join(argumentsValue.rootPath, ROOT_LAYOUT.LIMA_HOME),
      TMPDIR: join(argumentsValue.rootPath, ROOT_LAYOUT.TMPDIR),
    };
    if (argumentsValue.scenario === "orphan") {
      descendant = spawn(
        process.execPath,
        [
          FIXED_FAKE_COMMAND,
          "descendant",
          argumentsValue.rootPath,
          argumentsValue.fixtureId,
          argumentsValue.intentSha256,
          argumentsValue.rootOwnerSha256,
          witness.sha256,
        ],
        {
          cwd: argumentsValue.rootPath,
          detached: false,
          env: childEnv,
          stdio: ["ignore", "ignore", "ignore", "ipc"],
        },
      );
      descendant.once("error", terminateOwnGroup);
      descendant.once("close", maybeExitAfterTermination);
    }
    child = spawn(
      process.execPath,
      [
        FIXED_FAKE_COMMAND,
        argumentsValue.scenario,
        argumentsValue.rootPath,
        argumentsValue.fixtureId,
        argumentsValue.intentSha256,
        argumentsValue.rootOwnerSha256,
        witness.sha256,
      ],
      {
        cwd: argumentsValue.rootPath,
        detached: false,
        env: childEnv,
        stdio: ["ignore", "ignore", "ignore", "ipc"],
      },
    );
    child.once("error", terminateOwnGroup);
    child.once("close", (code, signal) => {
      if (terminating) {
        maybeExitAfterTermination();
        return;
      }
      try {
        const outcome = actorOutcomeValue(
          argumentsValue,
          decision.sha256,
          witness.sha256,
          { code, signal },
        );
        publishControlledProviderArtifact(
          argumentsValue.providerDirectory,
          "actor-outcome.json",
          outcome,
        );
        const acknowledgeOutcome = () => {
          process.send?.({
            outcome_sha256: controlledProviderDigest(controlledProviderBytes(outcome)),
            type: "outcome-ready",
          });
        };
        if (argumentsValue.afterOutcomePublishHoldMilliseconds === 0) acknowledgeOutcome();
        else setTimeout(acknowledgeOutcome, argumentsValue.afterOutcomePublishHoldMilliseconds);
      } catch {
        terminateOwnGroup();
      }
    });
  });
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  actorMain().catch((error) => {
    if (error instanceof ControlledProviderFailure) process.exit(error.exitStatus);
    process.exit(70);
  });
}
