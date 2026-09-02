#!/usr/bin/env node
import { spawn } from "node:child_process";
import { createHash, createHmac, randomBytes, timingSafeEqual } from "node:crypto";
import {
  chmodSync,
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
  rmdirSync,
  unlinkSync,
  writeSync,
} from "node:fs";
import { createConnection } from "node:net";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const MAX_ARTIFACT_BYTES = 128 * 1024;
const MAX_EVIDENCE_ENTRIES = 96;
const MAX_INVENTORY_ENTRIES = 64;
const MAX_PROTOCOL_BYTES = 4096;
const OWNER_MARKER = ".synveda-background-provider-owner.json";
const ZERO_SHA256 = "0".repeat(64);
const CONTROLLER_SCRIPT = fileURLToPath(
  new URL("../../../scripts/fixtures/clean-engine-background-controller.mjs", import.meta.url),
);
const HOSTAGENT_SCRIPT = fileURLToPath(
  new URL("../../../scripts/fixtures/clean-engine-background-hostagent.mjs", import.meta.url),
);

export const CONTROLLED_BACKGROUND_ROOT_LAYOUT = Object.freeze({
  COLIMA_CACHE_HOME: "k",
  COLIMA_HOME: "c",
  DOCKER_CONFIG: "d",
  LIMA_HOME: "l",
  TMPDIR: "t",
});

const CONTROLLED_BACKGROUND_CREATION_ARTIFACTS = Object.freeze([
  "background-create-authority.json",
  "background-toolchain.json",
  "controller-launch-decision.json",
  "controller-witness.json",
  "provider-start-decision.json",
  "hostagent-witness.json",
  "engine-witness.json",
  "context-witness.json",
  "controller-settlement.json",
  "provider-identity.json",
]);
const CONTROLLED_BACKGROUND_ARTIFACTS = Object.freeze([
  ...CONTROLLED_BACKGROUND_CREATION_ARTIFACTS,
  "provider-retirement-plan.json",
  "provider-retirement-settlement.json",
]);

export const COLIMA_LIVE_PREPARATION_CONTRACT = Object.freeze({
  application: Object.freeze({
    source_revision: "00f6c297e92a82c04a4ab507db0a61435650d7e8",
    tag: "v0.10.3",
    version: "0.10.3",
  }),
  command: Object.freeze([
    "colima",
    "start",
    "<receipt-owned-profile>",
    "--foreground",
    "--runtime",
    "docker",
    "--vm-type",
    "vz",
    "--arch",
    "aarch64",
    "--cpus",
    "4",
    "--memory",
    "6",
    "--disk",
    "40",
    "--root-disk",
    "20",
    "--mount",
    "none",
    "--ssh-agent=false",
    "--ssh-config=false",
    "--activate=true",
    "--kubernetes=false",
    "--template=false",
    "--binfmt=false",
    "--network-address=false",
    "--network-host-addresses=false",
    "--save-config=false",
    "--port-forwarder",
    "ssh",
    "--disk-image",
    "<digest-bound-receipt-owned-image>",
  ]),
  controller_semantics: "waits-after-background-lima-start",
  environment_names: Object.freeze([
    "COLIMA_CACHE_HOME",
    "COLIMA_HOME",
    "DOCKER_CONFIG",
    "HOME",
    "LANG",
    "LC_ALL",
    "LIMA_HOME",
    "PATH",
    "TMPDIR",
  ]),
  helper_closure: "unresolved-blocking",
  home_policy: "ambient-inherited-unchanged",
  lima: Object.freeze({
    source_revision: "de0816ea4bdc5267b428ab21025889b8dd785526",
    tag: "v2.2.0",
    version: "2.2.0",
  }),
  lima_start_semantics: "background-hostagent",
  optional_environment_names: Object.freeze(["__CF_USER_TEXT_ENCODING"]),
  process_identities: Object.freeze([
    "synveda-actor",
    "colima-controller",
    "lima-hostagent",
    "guest-engine",
    "docker-context",
  ]),
  provider: "colima",
  resource_identities: Object.freeze([
    "provider-profile",
    "lima-instance",
    "disk-image",
    "lima-hostagent-socket",
    "docker-engine-socket",
    "docker-engine",
    "docker-context",
    "provider-root",
  ]),
  root_layout: CONTROLLED_BACKGROUND_ROOT_LAYOUT,
  schema: "synveda.clean-engine.colima-live-preparation-contract.v1",
  start_authorized: false,
  target_host: Object.freeze({
    architecture: "arm64",
    os_version_gate: "unresolved-blocking",
    platform: "darwin",
  }),
  toolchain_requirements: Object.freeze([
    "colima-binary",
    "limactl-binary",
    "docker-cli-binary",
    "lima-guestagent",
    "colima-disk-image",
    "selected-host-helper-closure",
  ]),
});

export const COLIMA_LIVE_PREPARATION_CONTRACT_SHA256 = providerProcessDigest(
  providerProcessBytes(COLIMA_LIVE_PREPARATION_CONTRACT),
);

export const CONTROLLED_BACKGROUND_PROVIDER_CONTRACT = Object.freeze({
  artifact_order: CONTROLLED_BACKGROUND_CREATION_ARTIFACTS,
  controller_group_probe: "negative-pgid-esrch-v1",
  engine_protocol: "authenticated-content-free-version-v1",
  environment_names: COLIMA_LIVE_PREPARATION_CONTRACT.environment_names,
  fixture_launch_authorized: true,
  home_policy: COLIMA_LIVE_PREPARATION_CONTRACT.home_policy,
  hostagent_protocol: "authenticated-challenge-v1",
  launch_protocol: "durable-evidence-controller-start-gate-v1",
  kind: "controlled-background-provider-v2",
  lifecycle_exposure_authorized: false,
  live_preparation_contract_sha256: COLIMA_LIVE_PREPARATION_CONTRACT_SHA256,
  max_lifetime_milliseconds: 30_000,
  optional_environment_names: COLIMA_LIVE_PREPARATION_CONTRACT.optional_environment_names,
  provider_kind: "controlled-background-fake",
  root_layout: CONTROLLED_BACKGROUND_ROOT_LAYOUT,
  schema: "synveda.clean-engine.controlled-background-provider-contract.v2",
  toolchain_roles: Object.freeze(["controller-script", "hostagent-script", "node-runtime"]),
});

export class ProviderProcessContractFailure extends Error {
  constructor(message, exitStatus = 78) {
    super(message);
    this.exitStatus = exitStatus;
  }
}

function fail(message, exitStatus = 78) {
  throw new ProviderProcessContractFailure(message, exitStatus);
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
  fail("provider process canonical value was refused", 70);
}

export function providerProcessBytes(value) {
  return Buffer.from(`${canonical(value)}\n`, "utf8");
}

export function providerProcessDigest(value) {
  return createHash("sha256").update(value).digest("hex");
}

export const CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256 = providerProcessDigest(
  providerProcessBytes(CONTROLLED_BACKGROUND_PROVIDER_CONTRACT),
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

function exactArray(value, expected, label) {
  if (!Array.isArray(value) || canonical(value) !== canonical(expected)) {
    fail(`${label} was refused`);
  }
}

export function validateColimaLivePreparationContract(value) {
  if (canonical(value) !== canonical(COLIMA_LIVE_PREPARATION_CONTRACT)) {
    fail("Colima live preparation contract was refused");
  }
  return value;
}

export function validateColimaLiveHostEligibility(value, host) {
  validateColimaLivePreparationContract(value);
  exactKeys(host, ["architecture", "platform"], "Colima live host eligibility");
  if (
    host.architecture !== COLIMA_LIVE_PREPARATION_CONTRACT.target_host.architecture ||
    host.platform !== COLIMA_LIVE_PREPARATION_CONTRACT.target_host.platform
  ) {
    fail("Colima live host shape was refused", 69);
  }
  return host;
}

export function authorizeColimaLiveStart(
  value,
  host = { architecture: process.arch, platform: process.platform },
) {
  validateColimaLiveHostEligibility(value, host);
  fail("Colima live start remains blocked by the unresolved toolchain closure", 69);
}

function sameMetadata(left, right) {
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.mode === right.mode &&
    left.nlink === right.nlink &&
    left.size === right.size &&
    left.uid === right.uid
  );
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
    fail("provider process directory sync failed", 70);
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

function pathsOverlap(left, right) {
  const fromLeft = relative(left, right);
  const fromRight = relative(right, left);
  const nested = (value) => value === "" || (!value.startsWith(`..${sep}`) && value !== "..");
  return nested(fromLeft) || nested(fromRight);
}

function assertNoSymlinkComponents(path, label) {
  const components = path.split(sep);
  let current = sep;
  for (const component of components) {
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

function openedFile(
  path,
  label,
  maximumBytes = MAX_ARTIFACT_BYTES,
  expectedLinks = new Set([1n]),
  requirePrivate = false,
) {
  let descriptor;
  try {
    descriptor = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW);
    const before = fstatSync(descriptor, { bigint: true });
    const named = lstatSync(path, { bigint: true });
    if (
      !before.isFile() ||
      before.isSymbolicLink() ||
      !sameMetadata(before, named) ||
      !expectedLinks.has(before.nlink) ||
      (requirePrivate && before.uid !== BigInt(process.getuid())) ||
      (requirePrivate && (before.mode & 0o7777n) !== 0o600n) ||
      before.size < 1n ||
      before.size > BigInt(maximumBytes)
    ) {
      fail(`${label} identity was refused`);
    }
    const bytes = readFileSync(descriptor);
    const after = fstatSync(descriptor, { bigint: true });
    if (!sameMetadata(before, after)) fail(`${label} changed while it was read`);
    return { bytes, metadata: before };
  } catch (error) {
    if (error instanceof ProviderProcessContractFailure) throw error;
    fail(`${label} was unavailable`, 69);
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

function artifactNameAllowed(name) {
  return (
    CONTROLLED_BACKGROUND_ARTIFACTS.includes(name) ||
    /^retirement-step-[0-9]{2}\.json$/.test(name)
  );
}

function artifactStageName(name, sha256) {
  const nonce = randomBytes(16).toString("hex");
  return `.provider-process-stage-${name.slice(0, -".json".length)}-${sha256}-${nonce}`;
}

function parseArtifactStageName(name) {
  const match = /^\.provider-process-stage-([a-z0-9-]+)-([0-9a-f]{64})-([0-9a-f]{32})$/.exec(
    name,
  );
  if (match === null) return undefined;
  const targetName = `${match[1]}.json`;
  if (!artifactNameAllowed(targetName)) return undefined;
  return Object.freeze({ sha256: match[2], targetName });
}

function openedStage(path, label) {
  let descriptor;
  try {
    descriptor = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW);
    const before = fstatSync(descriptor, { bigint: true });
    const named = lstatSync(path, { bigint: true });
    if (
      !before.isFile() ||
      before.isSymbolicLink() ||
      !sameMetadata(before, named) ||
      before.uid !== BigInt(process.getuid()) ||
      !new Set([1n, 2n]).has(before.nlink) ||
      (before.mode & 0o7777n) !== 0o600n ||
      before.size > BigInt(MAX_ARTIFACT_BYTES)
    ) {
      fail(`${label} identity was refused`);
    }
    const bytes = readFileSync(descriptor);
    const after = fstatSync(descriptor, { bigint: true });
    if (!sameMetadata(before, after)) fail(`${label} changed while it was read`);
    return { bytes, metadata: before };
  } catch (error) {
    if (error instanceof ProviderProcessContractFailure) throw error;
    fail(`${label} was unavailable`, 69);
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

function artifactStages(directory, targetName) {
  const stages = [];
  for (const name of readdirSync(directory).sort()) {
    if (!name.startsWith(".provider-process-stage-")) continue;
    const parsed = parseArtifactStageName(name);
    if (parsed === undefined) fail("provider process stage name was refused");
    const stage = openedStage(join(directory, name), "provider process artifact stage");
    if (parsed.targetName === targetName) {
      stages.push(Object.freeze({ ...parsed, ...stage, name, path: join(directory, name) }));
    }
  }
  return stages;
}

function removeExactStage(directory, stage) {
  const current = openedStage(stage.path, "provider process artifact stage");
  if (!sameMetadata(current.metadata, stage.metadata) || !current.bytes.equals(stage.bytes)) {
    fail("provider process artifact stage changed");
  }
  try {
    unlinkSync(stage.path);
    syncDirectory(directory);
  } catch {
    fail("provider process artifact stage retirement failed", 70);
  }
}

function reconcileArtifactPublication(directory, name, expectedBytes) {
  const expectedSha256 = providerProcessDigest(expectedBytes);
  const finalPath = join(directory, name);
  let final;
  if (existsSync(finalPath)) {
    final = openedFile(
      finalPath,
      name,
      MAX_ARTIFACT_BYTES,
      new Set([1n, 2n]),
      true,
    );
    if (!final.bytes.equals(expectedBytes)) fail(`${name} changed`);
  }
  const stages = artifactStages(directory, name);
  for (const stage of stages) {
    if (stage.sha256 !== expectedSha256) {
      fail("provider process artifact stage digest was refused");
    }
    const linkedToFinal =
      final !== undefined &&
      stage.metadata.dev === final.metadata.dev &&
      stage.metadata.ino === final.metadata.ino;
    if (stage.metadata.nlink === 2n && !linkedToFinal) {
      fail("provider process artifact stage link was foreign");
    }
    if (stage.metadata.nlink === 1n && stage.bytes.equals(expectedBytes) && final === undefined) {
      try {
        linkSync(stage.path, finalPath);
        syncDirectory(directory);
      } catch (error) {
        if (error?.code !== "EEXIST") fail(`${name} recovery publication failed`, 70);
      }
      final = openedFile(
        finalPath,
        name,
        MAX_ARTIFACT_BYTES,
        new Set([1n, 2n]),
        true,
      );
      if (!final.bytes.equals(expectedBytes)) fail(`${name} changed`);
      const linkedStage = openedStage(stage.path, "provider process artifact stage");
      if (
        linkedStage.metadata.dev !== final.metadata.dev ||
        linkedStage.metadata.ino !== final.metadata.ino ||
        linkedStage.metadata.nlink !== 2n ||
        !linkedStage.bytes.equals(expectedBytes)
      ) {
        fail("provider process artifact recovery link changed");
      }
      removeExactStage(directory, { ...stage, ...linkedStage });
      final = openedFile(finalPath, name, MAX_ARTIFACT_BYTES, new Set([1n]), true);
      continue;
    }
    if (stage.metadata.nlink === 1n && !stage.bytes.equals(expectedBytes)) {
      removeExactStage(directory, stage);
      continue;
    }
    if (
      stage.metadata.nlink === 1n &&
      stage.bytes.equals(expectedBytes) &&
      final !== undefined
    ) {
      removeExactStage(directory, stage);
      continue;
    }
    const current = openedStage(stage.path, "provider process artifact stage");
    if (
      final === undefined ||
      current.metadata.dev !== final.metadata.dev ||
      current.metadata.ino !== final.metadata.ino ||
      !current.bytes.equals(expectedBytes)
    ) {
      fail("provider process artifact stage identity was refused");
    }
    removeExactStage(directory, { ...stage, ...current });
    final = openedFile(finalPath, name, MAX_ARTIFACT_BYTES, new Set([1n]), true);
  }
  return final;
}

function canonicalArtifact(path, label) {
  let opened = openedFile(
    path,
    label,
    MAX_ARTIFACT_BYTES,
    new Set([1n, 2n]),
    true,
  );
  if (artifactNameAllowed(basename(path))) {
    opened = reconcileArtifactPublication(dirname(path), basename(path), opened.bytes);
  }
  if (opened === undefined || opened.metadata.nlink !== 1n) {
    fail(`${label} publication was incomplete`);
  }
  let value;
  try {
    value = JSON.parse(opened.bytes.toString("utf8"));
  } catch {
    fail(`${label} was not canonical JSON`);
  }
  if (!providerProcessBytes(value).equals(opened.bytes)) fail(`${label} was not canonical JSON`);
  return Object.freeze({
    bytes: opened.bytes,
    metadata: opened.metadata,
    path,
    sha256: providerProcessDigest(opened.bytes),
    value,
  });
}

function writeExclusive(path, bytes) {
  let descriptor;
  try {
    descriptor = openSync(
      path,
      constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW,
      0o600,
    );
    fchmodSync(descriptor, 0o600);
    let offset = 0;
    while (offset < bytes.length) {
      const written = writeSync(descriptor, bytes, offset, bytes.length - offset, offset);
      if (written < 1) fail("provider process artifact write failed", 70);
      offset += written;
    }
    fsyncSync(descriptor);
  } catch {
    fail("provider process artifact write failed", 70);
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

function publishArtifact(directory, name, value) {
  if (!artifactNameAllowed(name)) {
    fail("provider process artifact name was refused", 64);
  }
  const finalPath = join(directory, name);
  const valueBytes = providerProcessBytes(value);
  const recovered = reconcileArtifactPublication(directory, name, valueBytes);
  if (recovered !== undefined) return canonicalArtifact(finalPath, name);
  const stageName = artifactStageName(name, providerProcessDigest(valueBytes));
  const stagePath = join(directory, stageName);
  writeExclusive(stagePath, valueBytes);
  try {
    linkSync(stagePath, finalPath);
    syncDirectory(directory);
  } catch (error) {
    if (error?.code === "EEXIST") {
      const existing = reconcileArtifactPublication(directory, name, valueBytes);
      if (existing !== undefined) return canonicalArtifact(finalPath, name);
    }
    fail(`${name} publication failed`, 70);
  }
  reconcileArtifactPublication(directory, name, valueBytes);
  return canonicalArtifact(finalPath, name);
}

function validateEvidenceDirectoryInventory(evidenceDirectory) {
  secureDirectory(evidenceDirectory, "controlled background evidence directory");
  const names = readdirSync(evidenceDirectory);
  if (names.length > MAX_EVIDENCE_ENTRIES) {
    fail("controlled background evidence capacity was exceeded");
  }
  for (const name of names) {
    if (artifactNameAllowed(name)) continue;
    const stage = parseArtifactStageName(name);
    if (stage === undefined) fail("controlled background evidence entry was refused");
    openedStage(join(evidenceDirectory, name), "provider process artifact stage");
  }
}

function secureDirectory(path, label, expectedUid = BigInt(process.getuid())) {
  let metadata;
  try {
    metadata = lstatSync(path, { bigint: true });
  } catch {
    fail(`${label} was unavailable`, 69);
  }
  if (
    metadata.isSymbolicLink() ||
    !metadata.isDirectory() ||
    metadata.uid !== expectedUid ||
    metadata.nlink < 2n ||
    (metadata.mode & 0o7777n) !== 0o700n
  ) {
    fail(`${label} identity was refused`);
  }
  return metadata;
}

function createDirectory(path, parent, label) {
  try {
    mkdirSync(path, { mode: 0o700 });
    chmodSync(path, 0o700);
    syncDirectory(parent);
  } catch {
    fail(`${label} creation failed`, 70);
  }
  return secureDirectory(path, label);
}

function privateFileIdentity(path, label, relativePath) {
  const opened = openedFile(path, label);
  if (
    opened.metadata.uid !== BigInt(process.getuid()) ||
    (opened.metadata.mode & 0o7777n) !== 0o600n
  ) {
    fail(`${label} identity was refused`);
  }
  return {
    device: String(opened.metadata.dev),
    inode: String(opened.metadata.ino),
    kind: "file",
    links: String(opened.metadata.nlink),
    mode: "0600",
    relative_path: relativePath,
    sha256: providerProcessDigest(opened.bytes),
    size: String(opened.metadata.size),
    uid: String(opened.metadata.uid),
  };
}

function executableArtifact(path, role) {
  let canonicalPath;
  try {
    canonicalPath = realpathSync(path);
  } catch {
    fail(`${role} was unavailable`, 69);
  }
  const opened = openedFile(canonicalPath, role, 256 * 1024 * 1024);
  return Object.freeze({
    bytes: opened.bytes,
    identity: {
      device: String(opened.metadata.dev),
      inode: String(opened.metadata.ino),
      links: String(opened.metadata.nlink),
      mode: (opened.metadata.mode & 0o7777n).toString(8).padStart(4, "0"),
      path: canonicalPath,
      role,
      sha256: providerProcessDigest(opened.bytes),
      size: String(opened.metadata.size),
      uid: String(opened.metadata.uid),
    },
  });
}

function controlledToolchain(fixtureId) {
  const controller = executableArtifact(CONTROLLER_SCRIPT, "controller-script");
  const hostagent = executableArtifact(HOSTAGENT_SCRIPT, "hostagent-script");
  const node = executableArtifact(process.execPath, "node-runtime");
  return Object.freeze({
    execution: Object.freeze({
      controllerSource: controller.bytes.toString("utf8"),
      hostagentSource: hostagent.bytes.toString("utf8"),
    }),
    value: {
      components: [controller.identity, hostagent.identity, node.identity].sort((left, right) =>
        left.role.localeCompare(right.role),
      ),
      contract_sha256: CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256,
      fixture_id: fixtureId,
      provider_kind: "controlled-background-fake",
      schema: "synveda.clean-engine.background-toolchain.v1",
    },
  });
}

function controlledEnvironment(rootPath) {
  const home = process.env.HOME;
  if (typeof home !== "string" || home === "" || !isAbsolute(home)) {
    fail("ambient HOME was unavailable", 69);
  }
  const environment = {
    COLIMA_CACHE_HOME: join(rootPath, CONTROLLED_BACKGROUND_ROOT_LAYOUT.COLIMA_CACHE_HOME),
    COLIMA_HOME: join(rootPath, CONTROLLED_BACKGROUND_ROOT_LAYOUT.COLIMA_HOME),
    DOCKER_CONFIG: join(rootPath, CONTROLLED_BACKGROUND_ROOT_LAYOUT.DOCKER_CONFIG),
    HOME: home,
    LANG: "C",
    LC_ALL: "C",
    LIMA_HOME: join(rootPath, CONTROLLED_BACKGROUND_ROOT_LAYOUT.LIMA_HOME),
    PATH: "/usr/bin:/bin:/usr/sbin:/sbin",
    TMPDIR: join(rootPath, CONTROLLED_BACKGROUND_ROOT_LAYOUT.TMPDIR),
  };
  if (process.platform === "darwin" && typeof process.env.__CF_USER_TEXT_ENCODING === "string") {
    environment.__CF_USER_TEXT_ENCODING = process.env.__CF_USER_TEXT_ENCODING;
  }
  exactArray(
    Object.keys(environment).sort(),
    expectedEnvironmentNames(),
    "controlled background environment",
  );
  return environment;
}

export function controlledBackgroundEnvironmentNames({
  hasCfUserTextEncoding,
  platform,
}) {
  if (
    !new Set(["darwin", "linux"]).has(platform) ||
    typeof hasCfUserTextEncoding !== "boolean"
  ) {
    fail("controlled background environment host was refused", 64);
  }
  const names = [...COLIMA_LIVE_PREPARATION_CONTRACT.environment_names];
  if (platform === "darwin" && hasCfUserTextEncoding) {
    names.push("__CF_USER_TEXT_ENCODING");
  }
  return names.sort();
}

export function controlledBackgroundEngineArchitecture(nodeArchitecture) {
  if (nodeArchitecture === "arm64") return "aarch64";
  if (nodeArchitecture === "x64") return "x86_64";
  fail("controlled background host architecture was refused", 69);
}

function expectedEnvironmentNames() {
  return controlledBackgroundEnvironmentNames({
    hasCfUserTextEncoding: typeof process.env.__CF_USER_TEXT_ENCODING === "string",
    platform: process.platform,
  });
}

function rootKey(fixtureId) {
  return `svb-${fixtureId.slice(0, 12)}`;
}

function contextKey(fixtureId) {
  return providerProcessDigest(Buffer.from(`context\0${fixtureId}`, "utf8")).slice(0, 32);
}

function rootPaths(base, fixtureId) {
  const root = join(base, rootKey(fixtureId));
  const profile = rootKey(fixtureId);
  const instance = `colima-${profile}`;
  const colimaProfile = join(root, CONTROLLED_BACKGROUND_ROOT_LAYOUT.COLIMA_HOME, profile);
  const limaInstance = join(root, CONTROLLED_BACKGROUND_ROOT_LAYOUT.LIMA_HOME, instance);
  const contextDirectory = join(
    root,
    CONTROLLED_BACKGROUND_ROOT_LAYOUT.DOCKER_CONFIG,
    "contexts",
    "meta",
    contextKey(fixtureId),
  );
  return Object.freeze({
    base,
    colimaProfile,
    contextDirectory,
    contextFile: join(contextDirectory, "meta.json"),
    controllerConfig: join(
      root,
      CONTROLLED_BACKGROUND_ROOT_LAYOUT.TMPDIR,
      "controller-config.json",
    ),
    controllerReady: join(root, CONTROLLED_BACKGROUND_ROOT_LAYOUT.TMPDIR, "controller-ready.json"),
    diskImage: join(limaInstance, "basedisk"),
    engineSocket: join(colimaProfile, "docker.sock"),
    haSocket: join(limaInstance, "ha.sock"),
    hostagentConfig: join(root, CONTROLLED_BACKGROUND_ROOT_LAYOUT.TMPDIR, "hostagent-config.json"),
    limaInstance,
    ownerMarker: join(root, OWNER_MARKER),
    pidRecord: join(limaInstance, "ha.pid"),
    profile,
    root,
  });
}

function validateCreateBindings(value) {
  exactKeys(
    value,
    [
      "create_intent_sha256",
      "create_slot_sequence",
      "create_slot_sha256",
      "ownership_nonce",
      "source_head_sha256",
      "source_sequence",
      "state_integration",
    ],
    "controlled background create bindings",
  );
  if (
    !lowerHex(value.create_intent_sha256, 64) ||
    !Number.isSafeInteger(value.create_slot_sequence) ||
    value.create_slot_sequence < 0 ||
    value.create_slot_sequence > 63 ||
    !lowerHex(value.create_slot_sha256, 64) ||
    !lowerHex(value.ownership_nonce, 64) ||
    !lowerHex(value.source_head_sha256, 64) ||
    !Number.isSafeInteger(value.source_sequence) ||
    value.source_sequence < 0 ||
    value.source_sequence > 63 ||
    value.state_integration !== "fixture-only"
  ) {
    fail("controlled background create bindings were refused", 64);
  }
}

function createAuthorityValue({ bindings, evidenceDirectory, fixtureId, paths }) {
  return {
    base: directoryIdentity(paths.base, "controlled background provider base"),
    create_intent_sha256: bindings.create_intent_sha256,
    create_slot_sequence: bindings.create_slot_sequence,
    create_slot_sha256: bindings.create_slot_sha256,
    evidence_directory: directoryIdentity(
      evidenceDirectory,
      "controlled background evidence directory",
    ),
    fixture_id: fixtureId,
    ownership_nonce: bindings.ownership_nonce,
    provider_contract_sha256: CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256,
    provider_profile: paths.profile,
    provider_root_path: paths.root,
    root_preexisting: "absent",
    schema: "synveda.clean-engine.background-create-authority.v1",
    source_head_sha256: bindings.source_head_sha256,
    source_sequence: bindings.source_sequence,
    state_integration: bindings.state_integration,
  };
}

function validateCreateAuthority(value, evidenceDirectory, fixtureId, paths) {
  exactKeys(
    value,
    [
      "base",
      "create_intent_sha256",
      "create_slot_sequence",
      "create_slot_sha256",
      "evidence_directory",
      "fixture_id",
      "ownership_nonce",
      "provider_contract_sha256",
      "provider_profile",
      "provider_root_path",
      "root_preexisting",
      "schema",
      "source_head_sha256",
      "source_sequence",
      "state_integration",
    ],
    "controlled background create authority",
  );
  validateCreateBindings({
    create_intent_sha256: value.create_intent_sha256,
    create_slot_sequence: value.create_slot_sequence,
    create_slot_sha256: value.create_slot_sha256,
    ownership_nonce: value.ownership_nonce,
    source_head_sha256: value.source_head_sha256,
    source_sequence: value.source_sequence,
    state_integration: value.state_integration,
  });
  validateDirectoryIdentity(value.base, "controlled background create base");
  validateDirectoryIdentity(
    value.evidence_directory,
    "controlled background create evidence directory",
  );
  if (
    value.schema !== "synveda.clean-engine.background-create-authority.v1" ||
    value.fixture_id !== fixtureId ||
    !lowerHex(value.ownership_nonce, 64) ||
    value.provider_contract_sha256 !== CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256 ||
    value.provider_profile !== paths.profile ||
    value.provider_root_path !== paths.root ||
    value.root_preexisting !== "absent" ||
    canonical(value.base) !==
      canonical(directoryIdentity(paths.base, "controlled background provider base")) ||
    canonical(value.evidence_directory) !==
      canonical(
        directoryIdentity(
          evidenceDirectory,
          "controlled background evidence directory",
        ),
      )
  ) {
    fail("controlled background create authority was refused");
  }
}

function validateControlledBackgroundRoots({ evidenceDirectory, fixtureId, providerBase }) {
  if (
    !lowerHex(fixtureId, 32) ||
    typeof providerBase !== "string" ||
    !isAbsolute(providerBase) ||
    typeof evidenceDirectory !== "string" ||
    !isAbsolute(evidenceDirectory) ||
    typeof process.getuid !== "function" ||
    typeof process.geteuid !== "function" ||
    process.getuid() === 0 ||
    process.getuid() !== process.geteuid() ||
    !new Set(["darwin", "linux"]).has(process.platform)
  ) {
    fail("controlled background root arguments were refused", 64);
  }
  let canonicalBase;
  let canonicalEvidence;
  try {
    canonicalBase = realpathSync(providerBase);
    canonicalEvidence = realpathSync(evidenceDirectory);
  } catch {
    fail("controlled background roots were unavailable", 69);
  }
  if (
    canonicalBase !== providerBase ||
    canonicalEvidence !== evidenceDirectory ||
    resolve(providerBase) !== providerBase ||
    resolve(evidenceDirectory) !== evidenceDirectory
  ) {
    fail("controlled background roots must use canonical absolute paths");
  }
  secureDirectory(providerBase, "controlled background provider base");
  secureDirectory(evidenceDirectory, "controlled background evidence directory");
  assertNoSymlinkComponents(providerBase, "controlled background provider base");
  assertNoSymlinkComponents(evidenceDirectory, "controlled background evidence directory");
  if (pathsOverlap(providerBase, evidenceDirectory)) {
    fail("controlled background roots overlapped");
  }
  const paths = rootPaths(providerBase, fixtureId);
  if (Buffer.byteLength(paths.haSocket, "utf8") > 103) {
    fail("controlled background socket path bound was exceeded");
  }
  return paths;
}

export function planControlledBackgroundProviderCreate({
  bindings,
  evidenceDirectory,
  fixtureId,
  providerBase,
}) {
  validateCreateBindings(bindings);
  const paths = validateControlledBackgroundRoots({
    evidenceDirectory,
    fixtureId,
    providerBase,
  });
  validateEvidenceDirectoryInventory(evidenceDirectory);
  for (const name of readdirSync(evidenceDirectory)) {
    if (name === "background-create-authority.json") continue;
    const stage = parseArtifactStageName(name);
    if (stage?.targetName !== "background-create-authority.json") {
      fail("controlled background pre-authority evidence inventory was refused", 73);
    }
  }
  if (pathEntryExists(paths.root)) {
    fail("controlled background provider root collided", 73);
  }
  const value = createAuthorityValue({
    bindings,
    evidenceDirectory,
    fixtureId,
    paths,
  });
  const authority = publishArtifact(
    evidenceDirectory,
    "background-create-authority.json",
    value,
  );
  validateCreateAuthority(authority.value, evidenceDirectory, fixtureId, paths);
  return Object.freeze({ authority, paths });
}

function rootOwnerValue(paths, fixtureId, ownershipNonce, createAuthoritySha256) {
  return {
    create_authority_sha256: createAuthoritySha256,
    fixture_id: fixtureId,
    ownership_nonce: ownershipNonce,
    provider_contract_sha256: CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256,
    provider_kind: "controlled-background-fake",
    provider_profile: paths.profile,
    root_path: paths.root,
    schema: "synveda.clean-engine.background-provider-root-owner.v2",
  };
}

function publishPrivateFile(path, value) {
  const valueBytes = providerProcessBytes(value);
  writeExclusive(path, valueBytes);
  syncDirectory(dirname(path));
  return Object.freeze({
    bytes: valueBytes,
    sha256: providerProcessDigest(valueBytes),
    value,
  });
}

function prepareControlledBackgroundRoot({ authority, evidenceDirectory, fixtureId, providerBase }) {
  const paths = validateControlledBackgroundRoots({
    evidenceDirectory,
    fixtureId,
    providerBase,
  });
  validateCreateAuthority(authority.value, evidenceDirectory, fixtureId, paths);
  if (pathEntryExists(paths.root)) {
    fail("controlled background provider root collided", 73);
  }
  createDirectory(paths.root, providerBase, "controlled background provider root");
  const ownershipNonce = authority.value.ownership_nonce;
  const owner = publishPrivateFile(
    paths.ownerMarker,
    rootOwnerValue(paths, fixtureId, ownershipNonce, authority.sha256),
  );
  for (const leaf of Object.values(CONTROLLED_BACKGROUND_ROOT_LAYOUT).sort()) {
    createDirectory(join(paths.root, leaf), paths.root, `controlled background ${leaf} root`);
  }
  createDirectory(
    paths.colimaProfile,
    dirname(paths.colimaProfile),
    "controlled background Colima profile",
  );
  createDirectory(
    paths.limaInstance,
    dirname(paths.limaInstance),
    "controlled background Lima instance",
  );
  const dockerConfig = join(paths.root, CONTROLLED_BACKGROUND_ROOT_LAYOUT.DOCKER_CONFIG);
  const contexts = join(dockerConfig, "contexts");
  const meta = join(contexts, "meta");
  createDirectory(contexts, dockerConfig, "controlled background context root");
  createDirectory(meta, contexts, "controlled background context metadata root");
  createDirectory(paths.contextDirectory, meta, "controlled background context directory");
  const disk = publishPrivateFile(paths.diskImage, {
    fixture_id: fixtureId,
    payload: "non-bootable-controlled-background-disk",
    schema: "synveda.clean-engine.background-disk-fixture.v1",
  });
  const context = publishPrivateFile(paths.contextFile, {
    context_name: paths.profile,
    endpoint: `unix://${paths.engineSocket}`,
    fixture_id: fixtureId,
    schema: "synveda.clean-engine.background-docker-context.v1",
    tls_material: "absent",
  });
  return Object.freeze({ context, disk, owner, ownershipNonce, paths });
}

function proof(secret, action, challenge, processIdentity) {
  return createHmac("sha256", secret)
    .update(
      `synveda.clean-engine.background-provider.v1\0${action}\0${challenge}\0${processIdentity}`,
    )
    .digest("hex");
}

function proofEquals(left, right) {
  return (
    lowerHex(left, 64) &&
    lowerHex(right, 64) &&
    timingSafeEqual(Buffer.from(left, "ascii"), Buffer.from(right, "ascii"))
  );
}

function controllerStartProofIdentity(processIdentity, startDecisionSha256) {
  return `${processIdentity}\0${startDecisionSha256}`;
}

function requestSocket(path, request, timeoutMilliseconds = 2_000) {
  return new Promise((resolvePromise, rejectPromise) => {
    let response = Buffer.alloc(0);
    let settled = false;
    const socket = createConnection({ path });
    const finish = (error, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      socket.destroy();
      if (error === undefined) resolvePromise(value);
      else rejectPromise(error);
    };
    const timeout = setTimeout(
      () => finish(new ProviderProcessContractFailure("provider process probe timed out", 69)),
      timeoutMilliseconds,
    );
    socket.once("error", () =>
      finish(new ProviderProcessContractFailure("provider process socket was unavailable", 69)),
    );
    socket.on("data", (chunk) => {
      response = Buffer.concat([response, chunk]);
      if (response.length > MAX_PROTOCOL_BYTES) {
        finish(new ProviderProcessContractFailure("provider process response was refused"));
      }
    });
    socket.once("connect", () => {
      socket.end(`${canonical(request)}\n`);
    });
    socket.once("end", () => {
      if (settled) return;
      try {
        const newline = response.indexOf(0x0a);
        if (newline < 1 || newline !== response.length - 1) {
          fail("provider process response was malformed");
        }
        const value = JSON.parse(response.subarray(0, newline).toString("utf8"));
        if (!providerProcessBytes(value).equals(response)) {
          fail("provider process response was not canonical JSON");
        }
        finish(undefined, value);
      } catch (error) {
        finish(
          error instanceof ProviderProcessContractFailure
            ? error
            : new ProviderProcessContractFailure("provider process response was refused"),
        );
      }
    });
  });
}

function socketIdentity(path, label, relativePath) {
  let metadata;
  try {
    metadata = lstatSync(path, { bigint: true });
  } catch {
    fail(`${label} was unavailable`, 69);
  }
  if (
    metadata.isSymbolicLink() ||
    !metadata.isSocket() ||
    metadata.uid !== BigInt(process.getuid()) ||
    metadata.nlink !== 1n ||
    (metadata.mode & 0o7777n) !== 0o600n
  ) {
    fail(`${label} identity was refused`);
  }
  return {
    device: String(metadata.dev),
    inode: String(metadata.ino),
    kind: "socket",
    links: "1",
    mode: "0600",
    relative_path: relativePath,
    sha256: ZERO_SHA256,
    size: String(metadata.size),
    uid: String(metadata.uid),
  };
}

function validateEnvironmentKeys(value, label) {
  exactArray(value, expectedEnvironmentNames(), label);
}

function validateHostagentProbe(value, expected, secret, challenge) {
  exactKeys(
    value,
    [
      "challenge_sha256",
      "fixture_id",
      "pid",
      "process_instance_sha256",
      "profile",
      "proof_sha256",
      "schema",
    ],
    "controlled background hostagent probe",
  );
  if (
    value.schema !== "synveda.clean-engine.background-hostagent-probe.v1" ||
    value.fixture_id !== expected.fixtureId ||
    value.profile !== expected.profile ||
    value.pid !== expected.pid ||
    value.process_instance_sha256 !== expected.processIdentity ||
    value.challenge_sha256 !== providerProcessDigest(Buffer.from(challenge, "ascii")) ||
    !proofEquals(
      value.proof_sha256,
      proof(secret, "hostagent-probe", challenge, expected.processIdentity),
    )
  ) {
    fail("controlled background hostagent probe was refused");
  }
}

function validateEngineProbe(value, expected, secret, challenge) {
  exactKeys(
    value,
    [
      "api_version",
      "architecture",
      "challenge_sha256",
      "fixture_id",
      "name",
      "operating_system",
      "process_instance_sha256",
      "proof_sha256",
      "schema",
      "server_id",
      "version",
    ],
    "controlled background Engine probe",
  );
  if (
    value.schema !== "synveda.clean-engine.background-engine-probe.v1" ||
    value.fixture_id !== expected.fixtureId ||
    value.process_instance_sha256 !== expected.processIdentity ||
    value.name !== `synveda-cpr45-${expected.fixtureId}` ||
    value.operating_system !== "linux" ||
    !new Set(["aarch64", "x86_64"]).has(value.architecture) ||
    value.api_version !== "1.52" ||
    value.version !== "29.4.0-fake" ||
    !lowerHex(value.server_id, 64) ||
    value.challenge_sha256 !== providerProcessDigest(Buffer.from(challenge, "ascii")) ||
    !proofEquals(
      value.proof_sha256,
      proof(secret, "engine-version", challenge, expected.processIdentity),
    )
  ) {
    fail("controlled background Engine probe was refused");
  }
}

function stableHostagentProbe(value) {
  return {
    fixture_id: value.fixture_id,
    pid: value.pid,
    process_instance_sha256: value.process_instance_sha256,
    profile: value.profile,
    schema: value.schema,
  };
}

function stableEngineProbe(value) {
  return {
    api_version: value.api_version,
    architecture: value.architecture,
    fixture_id: value.fixture_id,
    name: value.name,
    operating_system: value.operating_system,
    process_instance_sha256: value.process_instance_sha256,
    schema: value.schema,
    server_id: value.server_id,
    version: value.version,
  };
}

async function probeHostagent(paths, fixtureId, secret, pid, processIdentity) {
  const challenge = randomBytes(32).toString("hex");
  const value = await requestSocket(paths.haSocket, { action: "probe", challenge });
  validateHostagentProbe(
    value,
    { fixtureId, pid, processIdentity, profile: paths.profile },
    secret,
    challenge,
  );
  return value;
}

async function probeEngine(paths, fixtureId, secret, processIdentity) {
  const challenge = randomBytes(32).toString("hex");
  const value = await requestSocket(paths.engineSocket, { action: "version", challenge });
  validateEngineProbe(value, { fixtureId, processIdentity }, secret, challenge);
  return value;
}

function probeProcessGroup(pgid) {
  if (!Number.isSafeInteger(pgid) || pgid < 2) fail("controller process group was refused", 64);
  try {
    process.kill(-pgid, 0);
    return "present";
  } catch (error) {
    if (error?.code === "ESRCH") return "absent";
    if (error?.code === "EPERM") return "unknown";
    fail("controller process group probe failed", 69);
  }
}

async function waitForCanonicalArtifact(path, label, timeoutMilliseconds = 8_000) {
  const deadline = Date.now() + timeoutMilliseconds;
  while (Date.now() < deadline) {
    if (pathEntryExists(path)) {
      try {
        return canonicalArtifact(path, label);
      } catch {
        // The fixed child writes its private readiness file before fsync. A
        // reader may observe that final name while its bounded frame is partial.
      }
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 10));
  }
  fail("controlled background readiness timed out", 69);
}

async function waitForGroupAbsent(pgid, timeoutMilliseconds = 8_000) {
  const deadline = Date.now() + timeoutMilliseconds;
  while (Date.now() < deadline) {
    if (probeProcessGroup(pgid) === "absent") return;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 10));
  }
  fail("controller process group remained present", 69);
}

function requestControllerStart(controller, expected, secret, timeoutMilliseconds = 8_000) {
  return new Promise((resolvePromise, rejectPromise) => {
    const challenge = randomBytes(32).toString("hex");
    let settled = false;
    const finish = (error, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      controller.off("message", onMessage);
      controller.off("error", onError);
      controller.off("close", onClose);
      if (error === undefined) resolvePromise(value);
      else rejectPromise(error);
    };
    const onError = () =>
      finish(new ProviderProcessContractFailure("controller start channel failed", 69));
    const onClose = () =>
      finish(new ProviderProcessContractFailure("controller closed before start", 69));
    const onMessage = (value) => {
      try {
        exactKeys(
          value,
          [
            "challenge_sha256",
            "fixture_id",
            "hostagent_pid",
            "hostagent_process_instance_sha256",
            "process_instance_sha256",
            "proof_sha256",
            "schema",
            "start_decision_sha256",
          ],
          "controlled background controller start acknowledgement",
        );
        if (
          value.schema !== "synveda.clean-engine.background-controller-start.v1" ||
          value.fixture_id !== expected.fixtureId ||
          value.process_instance_sha256 !== expected.processIdentity ||
          value.start_decision_sha256 !== expected.startDecisionSha256 ||
          value.challenge_sha256 !== providerProcessDigest(Buffer.from(challenge, "ascii")) ||
          !Number.isSafeInteger(value.hostagent_pid) ||
          value.hostagent_pid < 2 ||
          !lowerHex(value.hostagent_process_instance_sha256, 64) ||
          !proofEquals(
            value.proof_sha256,
            proof(
              secret,
              "controller-start-accepted",
              challenge,
              controllerStartProofIdentity(
                expected.processIdentity,
                expected.startDecisionSha256,
              ),
            ),
          )
        ) {
          fail("controlled background controller start acknowledgement was refused");
        }
        finish(undefined, value);
      } catch (error) {
        finish(
          error instanceof ProviderProcessContractFailure
            ? error
            : new ProviderProcessContractFailure(
                "controlled background controller start acknowledgement was refused",
              ),
        );
      }
    };
    const timeout = setTimeout(
      () => finish(new ProviderProcessContractFailure("controller start timed out", 69)),
      timeoutMilliseconds,
    );
    controller.once("error", onError);
    controller.once("close", onClose);
    controller.on("message", onMessage);
    if (!controller.connected) {
      finish(new ProviderProcessContractFailure("controller channel was unavailable", 69));
      return;
    }
    controller.send(
      {
        action: "start",
        challenge,
        start_decision_sha256: expected.startDecisionSha256,
        proof_sha256: proof(
          secret,
          "controller-start",
          challenge,
          controllerStartProofIdentity(
            expected.processIdentity,
            expected.startDecisionSha256,
          ),
        ),
      },
      (error) => {
        if (error !== null && error !== undefined) onError();
      },
    );
  });
}

function requestControllerShutdown(controller, expected, secret, timeoutMilliseconds = 15_000) {
  return new Promise((resolvePromise, rejectPromise) => {
    const challenge = randomBytes(32).toString("hex");
    let settled = false;
    const finish = (error, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      controller.off("message", onMessage);
      controller.off("error", onError);
      controller.off("close", onClose);
      if (error === undefined) resolvePromise(value);
      else rejectPromise(error);
    };
    const onError = () =>
      finish(new ProviderProcessContractFailure("controller channel failed", 69));
    const onClose = () =>
      finish(new ProviderProcessContractFailure("controller closed before acknowledgement", 69));
    const onMessage = (value) => {
      try {
        exactKeys(
          value,
          [
            "challenge_sha256",
            "fixture_id",
            "process_instance_sha256",
            "proof_sha256",
            "schema",
            "start_was_pending",
          ],
          "controlled background controller shutdown acknowledgement",
        );
        if (
          value.schema !== "synveda.clean-engine.background-controller-shutdown.v2" ||
          value.fixture_id !== expected.fixtureId ||
          value.process_instance_sha256 !== expected.processIdentity ||
          value.challenge_sha256 !== providerProcessDigest(Buffer.from(challenge, "ascii")) ||
          typeof value.start_was_pending !== "boolean" ||
          !proofEquals(
            value.proof_sha256,
            proof(
              secret,
              "controller-shutdown-accepted",
              challenge,
              expected.processIdentity,
            ),
          )
        ) {
          fail("controlled background controller shutdown acknowledgement was refused");
        }
        finish(undefined, value);
      } catch (error) {
        finish(
          error instanceof ProviderProcessContractFailure
            ? error
            : new ProviderProcessContractFailure(
                "controlled background controller shutdown acknowledgement was refused",
              ),
        );
      }
    };
    const timeout = setTimeout(
      () => finish(new ProviderProcessContractFailure("controller shutdown timed out", 69)),
      timeoutMilliseconds,
    );
    controller.once("error", onError);
    controller.once("close", onClose);
    controller.on("message", onMessage);
    if (!controller.connected) {
      finish(new ProviderProcessContractFailure("controller channel was unavailable", 69));
      return;
    }
    controller.send(
      {
        action: "shutdown",
        challenge,
        proof_sha256: proof(
          secret,
          "controller-shutdown",
          challenge,
          expected.processIdentity,
        ),
      },
      (error) => {
        if (error !== null && error !== undefined) onError();
      },
    );
  });
}

async function boundedControllerClose(controllerClosed, timeoutMilliseconds = 2_000) {
  return new Promise((resolvePromise, rejectPromise) => {
    const timeout = setTimeout(
      () => rejectPromise(new ProviderProcessContractFailure("controller close timed out", 69)),
      timeoutMilliseconds,
    );
    controllerClosed.then((value) => {
      clearTimeout(timeout);
      resolvePromise(value);
    });
  });
}

function validateControllerReady(value, expected, secret) {
  exactKeys(
    value,
    [
      "controller_environment_keys",
      "controller_pid",
      "controller_process_instance_sha256",
      "controller_script_sha256",
      "fixture_id",
      "node_sha256",
      "proof_sha256",
      "schema",
      "working_directory",
    ],
    "controlled background controller readiness",
  );
  validateEnvironmentKeys(value.controller_environment_keys, "controller environment");
  if (
    value.schema !== "synveda.clean-engine.background-controller-ready.v2" ||
    value.fixture_id !== expected.fixtureId ||
    value.controller_pid !== expected.controllerPid ||
    !lowerHex(value.controller_process_instance_sha256, 64) ||
    value.controller_script_sha256 !== expected.controllerScriptSha256 ||
    value.node_sha256 !== expected.nodeSha256 ||
    value.working_directory !== expected.workingDirectory ||
    !proofEquals(
      value.proof_sha256,
      proof(
        secret,
        "controller-ready",
        expected.fixtureId,
        value.controller_process_instance_sha256,
      ),
    )
  ) {
    fail("controlled background controller readiness was refused");
  }
}

function validatePidRecord(value, expected, toolchain) {
  exactKeys(
    value,
    [
      "environment_keys",
      "fixture_id",
      "hostagent_script_sha256",
      "instance_nonce_sha256",
      "node_sha256",
      "pid",
      "ppid",
      "process_instance_sha256",
      "profile",
      "schema",
      "working_directory",
    ],
    "controlled background hostagent PID record",
  );
  validateEnvironmentKeys(value.environment_keys, "hostagent environment");
  const components = new Map(toolchain.components.map((component) => [component.role, component]));
  if (
    value.schema !== "synveda.clean-engine.background-hostagent-witness.v1" ||
    value.fixture_id !== expected.fixtureId ||
    value.profile !== expected.profile ||
    value.pid !== expected.pid ||
    value.ppid !== expected.controllerPid ||
    value.process_instance_sha256 !== expected.processIdentity ||
    value.instance_nonce_sha256 !== expected.instanceNonceSha256 ||
    value.hostagent_script_sha256 !== components.get("hostagent-script")?.sha256 ||
    value.node_sha256 !== components.get("node-runtime")?.sha256 ||
    value.working_directory !== expected.workingDirectory
  ) {
    fail("controlled background hostagent PID record was refused");
  }
}

function revalidateHostagentPidRecord(paths, fixtureId, evidence, instanceNonce) {
  const pidRecord = canonicalArtifact(
    paths.pidRecord,
    "controlled background hostagent PID record",
  );
  const identity = privateFileIdentity(
    paths.pidRecord,
    "controlled background hostagent PID record",
    relative(paths.root, paths.pidRecord),
  );
  if (canonical(identity) !== canonical(evidence.hostagentWitness.value.pid_record)) {
    fail("controlled background hostagent PID identity changed");
  }
  validatePidRecord(
    pidRecord.value,
    {
      controllerPid: evidence.controllerWitness.value.controller_pid,
      fixtureId,
      instanceNonceSha256: providerProcessDigest(Buffer.from(instanceNonce, "ascii")),
      pid: pidRecord.value.pid,
      processIdentity: evidence.hostagentWitness.value.process_instance_sha256,
      profile: paths.profile,
      workingDirectory: paths.root,
    },
    evidence.toolchain.value,
  );
  return pidRecord;
}

function contextWitnessValue(root, context, engineSocketIdentity, providerIdentity) {
  const contextValue = context.value;
  exactKeys(
    contextValue,
    ["context_name", "endpoint", "fixture_id", "schema", "tls_material"],
    "controlled background Docker context",
  );
  if (
    contextValue.schema !== "synveda.clean-engine.background-docker-context.v1" ||
    contextValue.fixture_id !== providerIdentity.fixtureId ||
    contextValue.context_name !== root.paths.profile ||
    contextValue.endpoint !== `unix://${root.paths.engineSocket}` ||
    contextValue.tls_material !== "absent"
  ) {
    fail("controlled background Docker context was refused");
  }
  return {
    context_file: privateFileIdentity(
      root.paths.contextFile,
      "controlled background context file",
      relative(root.paths.root, root.paths.contextFile),
    ),
    context_name: root.paths.profile,
    endpoint: contextValue.endpoint,
    engine_socket_sha256: providerProcessDigest(providerProcessBytes(engineSocketIdentity)),
    fixture_id: providerIdentity.fixtureId,
    schema: "synveda.clean-engine.background-context-witness.v1",
    tls_material: "absent",
  };
}

export async function launchControlledBackgroundProvider({
  beforeDetachHoldMilliseconds = 0,
  beforeStartHoldMilliseconds = 0,
  evidenceDirectory,
  fixtureId,
  maximumLifetimeMilliseconds = 30_000,
  providerBase,
  requireShutdownDuringStart = false,
}) {
  if (
    CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.fixture_launch_authorized !== true ||
    CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.lifecycle_exposure_authorized !== false
  ) {
    fail("controlled background fixture launch contract was refused", 70);
  }
  if (
    !Number.isSafeInteger(maximumLifetimeMilliseconds) ||
    maximumLifetimeMilliseconds < 1_000 ||
    maximumLifetimeMilliseconds > CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.max_lifetime_milliseconds
  ) {
    fail("controlled background lifetime was refused", 64);
  }
  if (
    !Number.isSafeInteger(beforeDetachHoldMilliseconds) ||
    beforeDetachHoldMilliseconds < 0 ||
    beforeDetachHoldMilliseconds > 5_000 ||
    !Number.isSafeInteger(beforeStartHoldMilliseconds) ||
    beforeStartHoldMilliseconds < 0 ||
    beforeStartHoldMilliseconds > 5_000 ||
    typeof requireShutdownDuringStart !== "boolean"
  ) {
    fail("controlled background pre-start hold was refused", 64);
  }
  const engineArchitecture = controlledBackgroundEngineArchitecture(process.arch);
  const authority = canonicalArtifact(
    join(evidenceDirectory, "background-create-authority.json"),
    "background-create-authority.json",
  );
  validateEvidenceDirectoryInventory(evidenceDirectory);
  exactArray(
    readdirSync(evidenceDirectory).sort(),
    ["background-create-authority.json"],
    "controlled background pre-launch evidence inventory",
  );
  const root = prepareControlledBackgroundRoot({
    authority,
    evidenceDirectory,
    fixtureId,
    providerBase,
  });
  const toolchainBundle = controlledToolchain(fixtureId);
  const toolchainValue = toolchainBundle.value;
  const toolchain = publishArtifact(evidenceDirectory, "background-toolchain.json", toolchainValue);
  const components = new Map(
    toolchainValue.components.map((component) => [component.role, component]),
  );
  const hostagentConfig = {
    engine_architecture: engineArchitecture,
    engine_socket: root.paths.engineSocket,
    fixture_id: fixtureId,
    ha_socket: root.paths.haSocket,
    hostagent_script_sha256: components.get("hostagent-script").sha256,
    instance_nonce: root.ownershipNonce,
    maximum_lifetime_milliseconds: maximumLifetimeMilliseconds,
    node_sha256: components.get("node-runtime").sha256,
    pid_record: root.paths.pidRecord,
    profile: root.paths.profile,
    schema: "synveda.clean-engine.background-hostagent-config.v1",
    working_directory: root.paths.root,
  };
  const hostagentConfigArtifact = publishPrivateFile(root.paths.hostagentConfig, hostagentConfig);
  const controllerNonce = randomBytes(32).toString("hex");
  const hostagentSourceBase64 = Buffer.from(
    toolchainBundle.execution.hostagentSource,
    "utf8",
  ).toString("base64");
  const controllerConfig = {
    before_detach_hold_milliseconds: beforeDetachHoldMilliseconds,
    controller_launch_decision: join(evidenceDirectory, "controller-launch-decision.json"),
    controller_nonce: controllerNonce,
    controller_ready: root.paths.controllerReady,
    controller_script_sha256: components.get("controller-script").sha256,
    controller_witness: join(evidenceDirectory, "controller-witness.json"),
    create_authority: join(evidenceDirectory, "background-create-authority.json"),
    create_authority_sha256: authority.sha256,
    create_intent_sha256: authority.value.create_intent_sha256,
    create_slot_sha256: authority.value.create_slot_sha256,
    fixture_id: fixtureId,
    hostagent_config: root.paths.hostagentConfig,
    hostagent_config_sha256: hostagentConfigArtifact.sha256,
    hostagent_source_base64: hostagentSourceBase64,
    hostagent_source_sha256: components.get("hostagent-script").sha256,
    instance_nonce: root.ownershipNonce,
    maximum_lifetime_milliseconds: maximumLifetimeMilliseconds,
    node_sha256: components.get("node-runtime").sha256,
    provider_contract_sha256: CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256,
    provider_start_decision: join(evidenceDirectory, "provider-start-decision.json"),
    require_shutdown_during_start: requireShutdownDuringStart,
    root_owner: root.paths.ownerMarker,
    root_owner_sha256: root.owner.sha256,
    schema: "synveda.clean-engine.background-controller-config.v2",
    working_directory: root.paths.root,
  };
  const controllerConfigArtifact = publishPrivateFile(
    root.paths.controllerConfig,
    controllerConfig,
  );
  const controllerLaunchDecision = publishArtifact(
    evidenceDirectory,
    "controller-launch-decision.json",
    {
      controller_config_sha256: controllerConfigArtifact.sha256,
      controller_nonce_sha256: providerProcessDigest(Buffer.from(controllerNonce, "ascii")),
      create_authority_sha256: authority.sha256,
      decision: "launch-waiting",
      fixture_id: fixtureId,
      hostagent_config_sha256: hostagentConfigArtifact.sha256,
      root_owner_sha256: root.owner.sha256,
      schema: "synveda.clean-engine.background-controller-launch-decision.v1",
      toolchain_sha256: toolchain.sha256,
    },
  );
  const controller = spawn(
    components.get("node-runtime").path,
    [
      "--input-type=module",
      "--eval",
      toolchainBundle.execution.controllerSource,
      root.paths.controllerConfig,
    ],
    {
      cwd: root.paths.root,
      detached: true,
      env: controlledEnvironment(root.paths.root),
      stdio: ["ignore", "ignore", "ignore", "ipc"],
    },
  );
  const controllerClosed = new Promise((resolvePromise) => {
    controller.once("error", (error) => resolvePromise({ error, signal: null, status: null }));
    controller.once("close", (status, signal) =>
      resolvePromise({ error: undefined, signal, status }),
    );
  });
  if (!Number.isSafeInteger(controller.pid) || controller.pid < 2) {
    fail("controlled background controller PID was unavailable", 69);
  }
  let controllerProcessIdentity;
  try {
    const ready = await waitForCanonicalArtifact(
      root.paths.controllerReady,
      "controlled background controller readiness",
    );
    validateControllerReady(
      ready.value,
      {
        controllerPid: controller.pid,
        controllerScriptSha256: components.get("controller-script").sha256,
        fixtureId,
        nodeSha256: components.get("node-runtime").sha256,
        workingDirectory: root.paths.root,
      },
      controllerNonce,
    );
    controllerProcessIdentity = ready.value.controller_process_instance_sha256;
    const controllerWitnessValue = {
      argv_contract: "repository-digest-bound-eval-controller-v1",
      controller_config_sha256: controllerConfigArtifact.sha256,
      controller_nonce_sha256: providerProcessDigest(Buffer.from(controllerNonce, "ascii")),
      controller_pgid: controller.pid,
      controller_pid: controller.pid,
      controller_process_instance_sha256: controllerProcessIdentity,
      controller_launch_decision_sha256: controllerLaunchDecision.sha256,
      create_authority_sha256: authority.sha256,
      execution_protocol: "authenticated-ipc-start-shutdown-v2",
      fixture_id: fixtureId,
      hostagent_config_sha256: hostagentConfigArtifact.sha256,
      root_owner_sha256: root.owner.sha256,
      schema: "synveda.clean-engine.background-controller-witness.v2",
      toolchain_sha256: toolchain.sha256,
    };
    const controllerWitness = publishArtifact(
      evidenceDirectory,
      "controller-witness.json",
      controllerWitnessValue,
    );
    const startDecision = publishArtifact(
      evidenceDirectory,
      "provider-start-decision.json",
      {
        controller_witness_sha256: controllerWitness.sha256,
        create_authority_sha256: authority.sha256,
        create_intent_sha256: authority.value.create_intent_sha256,
        create_slot_sha256: authority.value.create_slot_sha256,
        decision: "start",
        fixture_id: fixtureId,
        schema: "synveda.clean-engine.background-provider-start-decision.v1",
      },
    );
    if (beforeStartHoldMilliseconds > 0) {
      await new Promise((resolvePromise) =>
        setTimeout(resolvePromise, beforeStartHoldMilliseconds),
      );
    }
    const started = await requestControllerStart(
      controller,
      {
        fixtureId,
        processIdentity: controllerProcessIdentity,
        startDecisionSha256: startDecision.sha256,
      },
      controllerNonce,
    );
    if (probeProcessGroup(controller.pid) !== "present") {
      fail("controlled background controller disappeared before settlement", 69);
    }
    const earlyControllerShutdown = requireShutdownDuringStart
      ? requestControllerShutdown(
          controller,
          { fixtureId, processIdentity: controllerProcessIdentity },
          controllerNonce,
        ).then(
          (value) => ({ error: undefined, value }),
          (error) => ({ error, value: undefined }),
        )
      : undefined;
    const pidRecord = await waitForCanonicalArtifact(
      root.paths.pidRecord,
      "controlled background hostagent PID record",
    );
    validatePidRecord(
      pidRecord.value,
      {
        controllerPid: controller.pid,
        fixtureId,
        instanceNonceSha256: providerProcessDigest(
          Buffer.from(root.ownershipNonce, "ascii"),
        ),
        pid: started.hostagent_pid,
        processIdentity: started.hostagent_process_instance_sha256,
        profile: root.paths.profile,
        workingDirectory: root.paths.root,
      },
      toolchainValue,
    );
    const haSocket = socketIdentity(
      root.paths.haSocket,
      "controlled background hostagent socket",
      relative(root.paths.root, root.paths.haSocket),
    );
    const engineSocket = socketIdentity(
      root.paths.engineSocket,
      "controlled background Engine socket",
      relative(root.paths.root, root.paths.engineSocket),
    );
    const hostagentProbe = await probeHostagent(
      root.paths,
      fixtureId,
      root.ownershipNonce,
      started.hostagent_pid,
      started.hostagent_process_instance_sha256,
    );
    const engineProbe = await probeEngine(
      root.paths,
      fixtureId,
      root.ownershipNonce,
      started.hostagent_process_instance_sha256,
    );
    const hostagentWitnessValue = {
      authenticated_probe_sha256: providerProcessDigest(providerProcessBytes(hostagentProbe)),
      controller_witness_sha256: controllerWitness.sha256,
      fixture_id: fixtureId,
      instance_nonce_sha256: providerProcessDigest(
        Buffer.from(root.ownershipNonce, "ascii"),
      ),
      pid_record: privateFileIdentity(
        root.paths.pidRecord,
        "controlled background hostagent PID record",
        relative(root.paths.root, root.paths.pidRecord),
      ),
      process_instance_sha256: started.hostagent_process_instance_sha256,
      schema: "synveda.clean-engine.background-hostagent-identity.v2",
      socket: haSocket,
      start_decision_sha256: startDecision.sha256,
    };
    const hostagentWitness = publishArtifact(
      evidenceDirectory,
      "hostagent-witness.json",
      hostagentWitnessValue,
    );
    const engineWitnessValue = {
      api_version: engineProbe.api_version,
      architecture: engineProbe.architecture,
      authenticated_probe_sha256: providerProcessDigest(providerProcessBytes(engineProbe)),
      fixture_id: fixtureId,
      hostagent_witness_sha256: hostagentWitness.sha256,
      name: engineProbe.name,
      operating_system: engineProbe.operating_system,
      process_instance_sha256: started.hostagent_process_instance_sha256,
      schema: "synveda.clean-engine.background-engine-identity.v1",
      server_id: engineProbe.server_id,
      socket: engineSocket,
      version: engineProbe.version,
    };
    const engineWitness = publishArtifact(
      evidenceDirectory,
      "engine-witness.json",
      engineWitnessValue,
    );
    const contextWitness = publishArtifact(
      evidenceDirectory,
      "context-witness.json",
      contextWitnessValue(root, root.context, engineSocket, { fixtureId }),
    );
    const shutdownOutcome =
      earlyControllerShutdown === undefined
        ? {
            error: undefined,
            value: await requestControllerShutdown(
              controller,
              { fixtureId, processIdentity: controllerProcessIdentity },
              controllerNonce,
            ),
          }
        : await earlyControllerShutdown;
    if (shutdownOutcome.error !== undefined) throw shutdownOutcome.error;
    const shutdownAcknowledgement = shutdownOutcome.value;
    const closed = await boundedControllerClose(controllerClosed);
    await waitForGroupAbsent(controller.pid);
    if (closed.error !== undefined || closed.signal !== null || closed.status !== 0) {
      fail("controlled background controller settlement was refused", 69);
    }
    const hostagentAfterController = await probeHostagent(
      root.paths,
      fixtureId,
      root.ownershipNonce,
      started.hostagent_pid,
      started.hostagent_process_instance_sha256,
    );
    const engineAfterController = await probeEngine(
      root.paths,
      fixtureId,
      root.ownershipNonce,
      started.hostagent_process_instance_sha256,
    );
    const controllerSettlementValue = {
      controller_group_absent: true,
      controller_group_probe: "esrch",
      controller_shutdown: "authenticated-ipc",
      controller_witness_sha256: controllerWitness.sha256,
      engine_after_controller_sha256: providerProcessDigest(
        providerProcessBytes(stableEngineProbe(engineAfterController)),
      ),
      fixture_id: fixtureId,
      hostagent_after_controller_sha256: providerProcessDigest(
        providerProcessBytes(stableHostagentProbe(hostagentAfterController)),
      ),
      hostagent_disposition: "authenticated-running",
      provider_start_decision_sha256: startDecision.sha256,
      schema: "synveda.clean-engine.background-controller-settlement.v2",
      shutdown_during_start: shutdownAcknowledgement.start_was_pending,
    };
    const controllerSettlement = publishArtifact(
      evidenceDirectory,
      "controller-settlement.json",
      controllerSettlementValue,
    );
    const creationInventory = inspectRootInventory(root.paths);
    const providerIdentityValue = {
      create_authority_sha256: authority.sha256,
      context_witness_sha256: contextWitness.sha256,
      controller_launch_decision_sha256: controllerLaunchDecision.sha256,
      controller_settlement_sha256: controllerSettlement.sha256,
      controller_witness_sha256: controllerWitness.sha256,
      engine_witness_sha256: engineWitness.sha256,
      fixture_id: fixtureId,
      hostagent_witness_sha256: hostagentWitness.sha256,
      provider_contract_sha256: CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256,
      provider_kind: "controlled-background-fake",
      provider_profile: root.paths.profile,
      provider_root: creationInventory.root,
      provider_root_inventory: creationInventory.entries,
      resources: {
        docker_context: "fake-owned",
        engine: "fake-authenticated",
        engine_socket: "fake-owned",
        hostagent: "fake-authenticated",
        hostagent_socket: "fake-owned",
        provider_root: "owned",
      },
      root_owner_sha256: root.owner.sha256,
      schema: "synveda.clean-engine.background-provider-identity.v2",
      start_decision_sha256: startDecision.sha256,
      toolchain_sha256: toolchain.sha256,
    };
    const providerIdentity = publishArtifact(
      evidenceDirectory,
      "provider-identity.json",
      providerIdentityValue,
    );
    return Object.freeze({
      contextWitness,
      controllerLaunchDecision,
      controllerSettlement,
      controllerWitness,
      createAuthority: authority,
      engineWitness,
      evidenceDirectory,
      fixtureId,
      hostagentPid: started.hostagent_pid,
      hostagentWitness,
      instanceNonce: root.ownershipNonce,
      paths: root.paths,
      providerIdentity,
      root,
      startDecision,
      toolchain,
    });
  } catch (error) {
    try {
      if (controller.connected) controller.disconnect();
      await boundedControllerClose(controllerClosed);
    } catch {
      // Both fixed fixtures have bounded lifetimes. The original closed error
      // remains authoritative when their cooperative IPC cleanup cannot run.
    }
    throw error;
  }
}

function readCreationArtifacts(evidenceDirectory) {
  validateEvidenceDirectoryInventory(evidenceDirectory);
  const artifacts = {};
  for (const name of CONTROLLED_BACKGROUND_CREATION_ARTIFACTS) {
    artifacts[name] = canonicalArtifact(join(evidenceDirectory, name), name);
  }
  validateEvidenceDirectoryInventory(evidenceDirectory);
  return artifacts;
}

function validateToolchain(value, fixtureId, { revalidateCurrent = false } = {}) {
  exactKeys(
    value,
    ["components", "contract_sha256", "fixture_id", "provider_kind", "schema"],
    "controlled background toolchain",
  );
  if (
    value.schema !== "synveda.clean-engine.background-toolchain.v1" ||
    value.fixture_id !== fixtureId ||
    value.provider_kind !== "controlled-background-fake" ||
    value.contract_sha256 !== CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256 ||
    !Array.isArray(value.components) ||
    value.components.length !== CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.toolchain_roles.length
  ) {
    fail("controlled background toolchain was refused");
  }
  exactArray(
    value.components.map((component) => component.role),
    [...CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.toolchain_roles].sort(),
    "controlled background toolchain roles",
  );
  for (const component of value.components) {
    exactKeys(
      component,
      ["device", "inode", "links", "mode", "path", "role", "sha256", "size", "uid"],
      "controlled background toolchain component",
    );
    if (
      !decimalString(component.device) ||
      !decimalString(component.inode) ||
      component.links !== "1" ||
      !/^[0-7]{4}$/.test(component.mode) ||
      typeof component.path !== "string" ||
      !isAbsolute(component.path) ||
      !lowerHex(component.sha256, 64) ||
      !decimalString(component.size) ||
      !decimalString(component.uid)
    ) {
      fail("controlled background toolchain component was refused");
    }
  }
  if (revalidateCurrent) {
    const current = controlledToolchain(fixtureId).value.components;
    if (canonical(current) !== canonical(value.components)) {
      fail("controlled background toolchain component changed");
    }
  }
}

function validateResourceIdentity(value, kind, label) {
  exactKeys(
    value,
    ["device", "inode", "kind", "links", "mode", "relative_path", "sha256", "size", "uid"],
    label,
  );
  if (
    value.kind !== kind ||
    !decimalString(value.device) ||
    !decimalString(value.inode) ||
    !decimalString(value.links) ||
    !/^[0-7]{4}$/.test(value.mode) ||
    typeof value.relative_path !== "string" ||
    value.relative_path === "" ||
    isAbsolute(value.relative_path) ||
    value.relative_path.split(sep).some((part) => part === "" || part === "." || part === "..") ||
    !lowerHex(value.sha256, 64) ||
    !decimalString(value.size) ||
    !decimalString(value.uid) ||
    (kind === "socket" && value.sha256 !== ZERO_SHA256)
  ) {
    fail(`${label} was refused`);
  }
}

function validateCreationArtifactChain(
  artifacts,
  fixtureId,
  { revalidateCurrentToolchain = false } = {},
) {
  const createAuthority = artifacts["background-create-authority.json"];
  const toolchain = artifacts["background-toolchain.json"];
  const controllerLaunchDecision = artifacts["controller-launch-decision.json"];
  const controllerWitness = artifacts["controller-witness.json"];
  const providerStartDecision = artifacts["provider-start-decision.json"];
  const hostagentWitness = artifacts["hostagent-witness.json"];
  const engineWitness = artifacts["engine-witness.json"];
  const contextWitness = artifacts["context-witness.json"];
  const controllerSettlement = artifacts["controller-settlement.json"];
  const providerIdentity = artifacts["provider-identity.json"];
  const recordedProviderRoot = providerIdentity.value?.provider_root?.path;
  if (typeof recordedProviderRoot !== "string" || !isAbsolute(recordedProviderRoot)) {
    fail("controlled background provider root was refused");
  }
  const recordedPaths = rootPaths(dirname(recordedProviderRoot), fixtureId);
  validateCreateAuthority(
    createAuthority.value,
    dirname(createAuthority.path),
    fixtureId,
    recordedPaths,
  );
  validateToolchain(toolchain.value, fixtureId, {
    revalidateCurrent: revalidateCurrentToolchain,
  });
  exactKeys(
    controllerLaunchDecision.value,
    [
      "controller_config_sha256",
      "controller_nonce_sha256",
      "create_authority_sha256",
      "decision",
      "fixture_id",
      "hostagent_config_sha256",
      "root_owner_sha256",
      "schema",
      "toolchain_sha256",
    ],
    "controlled background controller launch decision",
  );
  if (
    controllerLaunchDecision.value.schema !==
      "synveda.clean-engine.background-controller-launch-decision.v1" ||
    controllerLaunchDecision.value.fixture_id !== fixtureId ||
    controllerLaunchDecision.value.decision !== "launch-waiting" ||
    controllerLaunchDecision.value.create_authority_sha256 !== createAuthority.sha256 ||
    !lowerHex(controllerLaunchDecision.value.controller_config_sha256, 64) ||
    !lowerHex(controllerLaunchDecision.value.controller_nonce_sha256, 64) ||
    !lowerHex(controllerLaunchDecision.value.hostagent_config_sha256, 64) ||
    !lowerHex(controllerLaunchDecision.value.root_owner_sha256, 64) ||
    controllerLaunchDecision.value.toolchain_sha256 !== toolchain.sha256
  ) {
    fail("controlled background controller launch decision was refused");
  }
  exactKeys(
    controllerWitness.value,
    [
      "argv_contract",
      "controller_config_sha256",
      "controller_nonce_sha256",
      "controller_pgid",
      "controller_pid",
      "controller_process_instance_sha256",
      "controller_launch_decision_sha256",
      "create_authority_sha256",
      "execution_protocol",
      "fixture_id",
      "hostagent_config_sha256",
      "root_owner_sha256",
      "schema",
      "toolchain_sha256",
    ],
    "controlled background controller witness",
  );
  if (
    controllerWitness.value.schema !== "synveda.clean-engine.background-controller-witness.v2" ||
    controllerWitness.value.fixture_id !== fixtureId ||
    controllerWitness.value.argv_contract !== "repository-digest-bound-eval-controller-v1" ||
    controllerWitness.value.execution_protocol !== "authenticated-ipc-start-shutdown-v2" ||
    !lowerHex(controllerWitness.value.controller_config_sha256, 64) ||
    !lowerHex(controllerWitness.value.controller_nonce_sha256, 64) ||
    !Number.isSafeInteger(controllerWitness.value.controller_pid) ||
    controllerWitness.value.controller_pid < 2 ||
    controllerWitness.value.controller_pgid !== controllerWitness.value.controller_pid ||
    !lowerHex(controllerWitness.value.controller_process_instance_sha256, 64) ||
    !lowerHex(controllerWitness.value.hostagent_config_sha256, 64) ||
    !lowerHex(controllerWitness.value.root_owner_sha256, 64) ||
    controllerWitness.value.controller_launch_decision_sha256 !==
      controllerLaunchDecision.sha256 ||
    controllerWitness.value.create_authority_sha256 !== createAuthority.sha256 ||
    controllerWitness.value.controller_config_sha256 !==
      controllerLaunchDecision.value.controller_config_sha256 ||
    controllerWitness.value.controller_nonce_sha256 !==
      controllerLaunchDecision.value.controller_nonce_sha256 ||
    controllerWitness.value.hostagent_config_sha256 !==
      controllerLaunchDecision.value.hostagent_config_sha256 ||
    controllerWitness.value.root_owner_sha256 !==
      controllerLaunchDecision.value.root_owner_sha256 ||
    controllerWitness.value.toolchain_sha256 !== toolchain.sha256
  ) {
    fail("controlled background controller witness was refused");
  }
  exactKeys(
    providerStartDecision.value,
    [
      "controller_witness_sha256",
      "create_authority_sha256",
      "create_intent_sha256",
      "create_slot_sha256",
      "decision",
      "fixture_id",
      "schema",
    ],
    "controlled background provider start decision",
  );
  if (
    providerStartDecision.value.schema !==
      "synveda.clean-engine.background-provider-start-decision.v1" ||
    providerStartDecision.value.fixture_id !== fixtureId ||
    providerStartDecision.value.decision !== "start" ||
    providerStartDecision.value.controller_witness_sha256 !== controllerWitness.sha256 ||
    providerStartDecision.value.create_authority_sha256 !== createAuthority.sha256 ||
    providerStartDecision.value.create_intent_sha256 !==
      createAuthority.value.create_intent_sha256 ||
    providerStartDecision.value.create_slot_sha256 !==
      createAuthority.value.create_slot_sha256
  ) {
    fail("controlled background provider start decision was refused");
  }
  exactKeys(
    hostagentWitness.value,
    [
      "authenticated_probe_sha256",
      "controller_witness_sha256",
      "fixture_id",
      "instance_nonce_sha256",
      "pid_record",
      "process_instance_sha256",
      "schema",
      "socket",
      "start_decision_sha256",
    ],
    "controlled background hostagent witness",
  );
  validateResourceIdentity(
    hostagentWitness.value.pid_record,
    "file",
    "controlled background hostagent PID identity",
  );
  validateResourceIdentity(
    hostagentWitness.value.socket,
    "socket",
    "controlled background hostagent socket identity",
  );
  if (
    hostagentWitness.value.schema !== "synveda.clean-engine.background-hostagent-identity.v2" ||
    hostagentWitness.value.fixture_id !== fixtureId ||
    hostagentWitness.value.controller_witness_sha256 !== controllerWitness.sha256 ||
    hostagentWitness.value.start_decision_sha256 !== providerStartDecision.sha256 ||
    !lowerHex(hostagentWitness.value.authenticated_probe_sha256, 64) ||
    !lowerHex(hostagentWitness.value.instance_nonce_sha256, 64) ||
    !lowerHex(hostagentWitness.value.process_instance_sha256, 64)
  ) {
    fail("controlled background hostagent witness was refused");
  }
  exactKeys(
    engineWitness.value,
    [
      "api_version",
      "architecture",
      "authenticated_probe_sha256",
      "fixture_id",
      "hostagent_witness_sha256",
      "name",
      "operating_system",
      "process_instance_sha256",
      "schema",
      "server_id",
      "socket",
      "version",
    ],
    "controlled background Engine witness",
  );
  validateResourceIdentity(
    engineWitness.value.socket,
    "socket",
    "controlled background Engine socket identity",
  );
  if (
    engineWitness.value.schema !== "synveda.clean-engine.background-engine-identity.v1" ||
    engineWitness.value.fixture_id !== fixtureId ||
    engineWitness.value.hostagent_witness_sha256 !== hostagentWitness.sha256 ||
    engineWitness.value.process_instance_sha256 !==
      hostagentWitness.value.process_instance_sha256 ||
    engineWitness.value.api_version !== "1.52" ||
    engineWitness.value.version !== "29.4.0-fake" ||
    engineWitness.value.operating_system !== "linux" ||
    !new Set(["aarch64", "x86_64"]).has(engineWitness.value.architecture) ||
    engineWitness.value.name !== `synveda-cpr45-${fixtureId}` ||
    !lowerHex(engineWitness.value.authenticated_probe_sha256, 64) ||
    !lowerHex(engineWitness.value.server_id, 64)
  ) {
    fail("controlled background Engine witness was refused");
  }
  exactKeys(
    contextWitness.value,
    [
      "context_file",
      "context_name",
      "endpoint",
      "engine_socket_sha256",
      "fixture_id",
      "schema",
      "tls_material",
    ],
    "controlled background context witness",
  );
  validateResourceIdentity(
    contextWitness.value.context_file,
    "file",
    "controlled background context file identity",
  );
  const contextEndpointPath =
    typeof contextWitness.value.endpoint === "string" &&
    contextWitness.value.endpoint.startsWith("unix://")
      ? contextWitness.value.endpoint.slice("unix://".length)
      : "";
  const recordedEngineSocket =
    typeof recordedProviderRoot === "string" && isAbsolute(recordedProviderRoot)
      ? join(recordedProviderRoot, engineWitness.value.socket.relative_path)
      : "";
  if (
    contextWitness.value.schema !== "synveda.clean-engine.background-context-witness.v1" ||
    contextWitness.value.fixture_id !== fixtureId ||
    contextWitness.value.context_name !== rootKey(fixtureId) ||
    !isAbsolute(contextEndpointPath) ||
    contextEndpointPath !== recordedEngineSocket ||
    contextWitness.value.engine_socket_sha256 !==
      providerProcessDigest(providerProcessBytes(engineWitness.value.socket)) ||
    contextWitness.value.tls_material !== "absent"
  ) {
    fail("controlled background context witness was refused");
  }
  exactKeys(
    controllerSettlement.value,
    [
      "controller_group_absent",
      "controller_group_probe",
      "controller_shutdown",
      "controller_witness_sha256",
      "engine_after_controller_sha256",
      "fixture_id",
      "hostagent_after_controller_sha256",
      "hostagent_disposition",
      "provider_start_decision_sha256",
      "schema",
      "shutdown_during_start",
    ],
    "controlled background controller settlement",
  );
  if (
    controllerSettlement.value.schema !==
      "synveda.clean-engine.background-controller-settlement.v2" ||
    controllerSettlement.value.fixture_id !== fixtureId ||
    controllerSettlement.value.controller_group_absent !== true ||
    controllerSettlement.value.controller_group_probe !== "esrch" ||
    controllerSettlement.value.controller_shutdown !== "authenticated-ipc" ||
    controllerSettlement.value.controller_witness_sha256 !== controllerWitness.sha256 ||
    controllerSettlement.value.provider_start_decision_sha256 !==
      providerStartDecision.sha256 ||
    controllerSettlement.value.hostagent_disposition !== "authenticated-running" ||
    !lowerHex(controllerSettlement.value.engine_after_controller_sha256, 64) ||
    !lowerHex(controllerSettlement.value.hostagent_after_controller_sha256, 64) ||
    typeof controllerSettlement.value.shutdown_during_start !== "boolean"
  ) {
    fail("controlled background controller settlement was refused");
  }
  exactKeys(
    providerIdentity.value,
    [
      "create_authority_sha256",
      "context_witness_sha256",
      "controller_launch_decision_sha256",
      "controller_settlement_sha256",
      "controller_witness_sha256",
      "engine_witness_sha256",
      "fixture_id",
      "hostagent_witness_sha256",
      "provider_contract_sha256",
      "provider_kind",
      "provider_profile",
      "provider_root",
      "provider_root_inventory",
      "resources",
      "root_owner_sha256",
      "schema",
      "start_decision_sha256",
      "toolchain_sha256",
    ],
    "controlled background provider identity",
  );
  exactKeys(
    providerIdentity.value.resources,
    [
      "docker_context",
      "engine",
      "engine_socket",
      "hostagent",
      "hostagent_socket",
      "provider_root",
    ],
    "controlled background provider resources",
  );
  validateDirectoryIdentity(
    providerIdentity.value.provider_root,
    "controlled background provider creation root",
  );
  if (!Array.isArray(providerIdentity.value.provider_root_inventory)) {
    fail("controlled background provider creation inventory was refused");
  }
  for (const entry of providerIdentity.value.provider_root_inventory) {
    if (!new Set(["directory", "file", "socket"]).has(entry.kind)) {
      fail("controlled background provider creation inventory kind was refused");
    }
    validateResourceIdentity(
      entry,
      entry.kind,
      "controlled background provider creation inventory",
    );
  }
  const creationByPath = new Map(
    providerIdentity.value.provider_root_inventory.map((entry) => [entry.relative_path, entry]),
  );
  const creationIdentityMatches = (path, expected) => {
    const current = creationByPath.get(relative(recordedPaths.root, path));
    return current !== undefined && canonical(current) === canonical(expected);
  };
  if (
    providerIdentity.value.schema !== "synveda.clean-engine.background-provider-identity.v2" ||
    providerIdentity.value.fixture_id !== fixtureId ||
    providerIdentity.value.provider_kind !== "controlled-background-fake" ||
    providerIdentity.value.provider_profile !== rootKey(fixtureId) ||
    providerIdentity.value.provider_root.path !== recordedPaths.root ||
    new Set(
      providerIdentity.value.provider_root_inventory.map((entry) => entry.relative_path),
    ).size !== providerIdentity.value.provider_root_inventory.length ||
    canonical(
      providerIdentity.value.provider_root_inventory
        .map((entry) => entry.relative_path)
        .sort(),
    ) !== canonical(expectedRootPaths(recordedPaths)) ||
    providerIdentity.value.provider_root_inventory.some(
      (entry) =>
        entry.device !== providerIdentity.value.provider_root.device ||
        entry.uid !== providerIdentity.value.provider_root.uid,
    ) ||
    !creationIdentityMatches(
      recordedPaths.pidRecord,
      hostagentWitness.value.pid_record,
    ) ||
    !creationIdentityMatches(recordedPaths.haSocket, hostagentWitness.value.socket) ||
    !creationIdentityMatches(recordedPaths.engineSocket, engineWitness.value.socket) ||
    !creationIdentityMatches(recordedPaths.contextFile, contextWitness.value.context_file) ||
    creationByPath.get(OWNER_MARKER)?.sha256 !== providerIdentity.value.root_owner_sha256 ||
    creationByPath.get(relative(recordedPaths.root, recordedPaths.controllerConfig))?.sha256 !==
      controllerWitness.value.controller_config_sha256 ||
    creationByPath.get(relative(recordedPaths.root, recordedPaths.hostagentConfig))?.sha256 !==
      controllerWitness.value.hostagent_config_sha256 ||
    providerIdentity.value.provider_contract_sha256 !==
      CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256 ||
    providerIdentity.value.create_authority_sha256 !== createAuthority.sha256 ||
    providerIdentity.value.controller_launch_decision_sha256 !==
      controllerLaunchDecision.sha256 ||
    providerIdentity.value.start_decision_sha256 !== providerStartDecision.sha256 ||
    providerIdentity.value.toolchain_sha256 !== toolchain.sha256 ||
    providerIdentity.value.controller_witness_sha256 !== controllerWitness.sha256 ||
    providerIdentity.value.hostagent_witness_sha256 !== hostagentWitness.sha256 ||
    providerIdentity.value.engine_witness_sha256 !== engineWitness.sha256 ||
    providerIdentity.value.context_witness_sha256 !== contextWitness.sha256 ||
    providerIdentity.value.controller_settlement_sha256 !== controllerSettlement.sha256 ||
    providerIdentity.value.root_owner_sha256 !== controllerWitness.value.root_owner_sha256 ||
    canonical(providerIdentity.value.resources) !==
      canonical({
        docker_context: "fake-owned",
        engine: "fake-authenticated",
        engine_socket: "fake-owned",
        hostagent: "fake-authenticated",
        hostagent_socket: "fake-owned",
        provider_root: "owned",
      })
  ) {
    fail("controlled background provider identity was refused");
  }
  return Object.freeze({
    createAuthority,
    contextWitness,
    controllerLaunchDecision,
    controllerSettlement,
    controllerWitness,
    engineWitness,
    hostagentWitness,
    providerIdentity,
    providerStartDecision,
    toolchain,
  });
}

export function inspectControlledBackgroundProvider(
  evidenceDirectory,
  fixtureId,
  { expectedCreateBindings, revalidateCurrentToolchain = false } = {},
) {
  if (!lowerHex(fixtureId, 32)) fail("controlled background fixture was refused", 64);
  const artifacts = readCreationArtifacts(evidenceDirectory);
  const evidence = validateCreationArtifactChain(artifacts, fixtureId, {
    revalidateCurrentToolchain,
  });
  if (expectedCreateBindings !== undefined) {
    validateCreateBindings(expectedCreateBindings);
    const recorded = {
      create_intent_sha256: evidence.createAuthority.value.create_intent_sha256,
      create_slot_sequence: evidence.createAuthority.value.create_slot_sequence,
      create_slot_sha256: evidence.createAuthority.value.create_slot_sha256,
      ownership_nonce: evidence.createAuthority.value.ownership_nonce,
      source_head_sha256: evidence.createAuthority.value.source_head_sha256,
      source_sequence: evidence.createAuthority.value.source_sequence,
      state_integration: evidence.createAuthority.value.state_integration,
    };
    if (canonical(recorded) !== canonical(expectedCreateBindings)) {
      fail("controlled background create authority binding changed");
    }
  }
  return evidence;
}

function pathDepth(path) {
  return path.split(sep).length;
}

function expectedRootPaths(paths) {
  return [
    relative(paths.root, paths.ownerMarker),
    CONTROLLED_BACKGROUND_ROOT_LAYOUT.COLIMA_CACHE_HOME,
    CONTROLLED_BACKGROUND_ROOT_LAYOUT.COLIMA_HOME,
    relative(paths.root, paths.colimaProfile),
    relative(paths.root, paths.engineSocket),
    CONTROLLED_BACKGROUND_ROOT_LAYOUT.DOCKER_CONFIG,
    join(CONTROLLED_BACKGROUND_ROOT_LAYOUT.DOCKER_CONFIG, "contexts"),
    join(CONTROLLED_BACKGROUND_ROOT_LAYOUT.DOCKER_CONFIG, "contexts", "meta"),
    relative(paths.root, paths.contextDirectory),
    relative(paths.root, paths.contextFile),
    CONTROLLED_BACKGROUND_ROOT_LAYOUT.LIMA_HOME,
    relative(paths.root, paths.limaInstance),
    relative(paths.root, paths.diskImage),
    relative(paths.root, paths.haSocket),
    relative(paths.root, paths.pidRecord),
    CONTROLLED_BACKGROUND_ROOT_LAYOUT.TMPDIR,
    relative(paths.root, paths.controllerConfig),
    relative(paths.root, paths.controllerReady),
    relative(paths.root, paths.hostagentConfig),
  ].sort();
}

function inventoryIdentity(path, rootPath, rootDevice) {
  let metadata;
  try {
    metadata = lstatSync(path, { bigint: true });
  } catch {
    fail("controlled background inventory was unavailable", 69);
  }
  const relativePath = relative(rootPath, path);
  const kind = metadata.isDirectory()
    ? "directory"
    : metadata.isFile()
      ? "file"
      : metadata.isSocket()
        ? "socket"
        : "unsupported";
  if (
    kind === "unsupported" ||
    metadata.isSymbolicLink() ||
    metadata.dev !== rootDevice ||
    metadata.uid !== BigInt(process.getuid()) ||
    (kind === "directory" && (metadata.mode & 0o7777n) !== 0o700n) ||
    (kind !== "directory" && (metadata.mode & 0o7777n) !== 0o600n) ||
    (kind !== "directory" && metadata.nlink !== 1n)
  ) {
    fail("controlled background inventory identity was refused");
  }
  let sha256 = ZERO_SHA256;
  if (kind === "file") {
    sha256 = privateFileIdentity(path, "controlled background inventory file", relativePath).sha256;
  }
  return {
    device: String(metadata.dev),
    inode: String(metadata.ino),
    kind,
    links: String(metadata.nlink),
    mode: (metadata.mode & 0o7777n).toString(8).padStart(4, "0"),
    relative_path: relativePath,
    sha256,
    size: String(metadata.size),
    uid: String(metadata.uid),
  };
}

function scanRootInventory(paths) {
  const rootMetadata = secureDirectory(paths.root, "controlled background provider root");
  const entries = [];
  const walk = (directory) => {
    for (const name of readdirSync(directory).sort()) {
      const path = join(directory, name);
      const identity = inventoryIdentity(path, paths.root, rootMetadata.dev);
      entries.push(identity);
      if (entries.length > MAX_INVENTORY_ENTRIES) {
        fail("controlled background inventory capacity was exceeded");
      }
      if (identity.kind === "directory") walk(path);
    }
  };
  walk(paths.root);
  return Object.freeze({
    entries: entries.sort((left, right) => left.relative_path.localeCompare(right.relative_path)),
    root: {
      device: String(rootMetadata.dev),
      inode: String(rootMetadata.ino),
      mode: "0700",
      path: paths.root,
      uid: String(rootMetadata.uid),
    },
  });
}

function inspectRootInventory(paths) {
  const inventory = scanRootInventory(paths);
  if (
    canonical(inventory.entries.map((entry) => entry.relative_path).sort()) !==
    canonical(expectedRootPaths(paths))
  ) {
    fail("controlled background root inventory was refused");
  }
  return inventory;
}

function creationInventoryForPlanning(paths, providerIdentity, allowMissingSockets) {
  const current = scanRootInventory(paths);
  const expected = {
    entries: providerIdentity.provider_root_inventory,
    root: providerIdentity.provider_root,
  };
  if (canonical(current.root) !== canonical(expected.root)) {
    fail("controlled background provider creation identity changed");
  }
  const expectedByPath = new Map(
    expected.entries.map((entry) => [entry.relative_path, entry]),
  );
  const currentByPath = new Map(
    current.entries.map((entry) => [entry.relative_path, entry]),
  );
  for (const entry of current.entries) {
    const created = expectedByPath.get(entry.relative_path);
    const unchanged =
      created !== undefined &&
      (entry.kind === "directory"
        ? exactProgressiveIdentity(entry, created)
        : canonical(entry) === canonical(created));
    if (!unchanged) {
      fail(
        `controlled background provider creation identity changed at ${entry.relative_path}`,
      );
    }
  }
  for (const entry of expected.entries) {
    if (
      !currentByPath.has(entry.relative_path) &&
      !(allowMissingSockets && entry.kind === "socket")
    ) {
      fail("controlled background provider creation identity changed");
    }
  }
  return Object.freeze(expected);
}

function retirementSteps(inventory) {
  const sockets = inventory.entries
    .filter((entry) => entry.kind === "socket")
    .sort((left, right) => {
      const leftHostagent = left.relative_path.endsWith(`${sep}ha.sock`) ? 0 : 1;
      const rightHostagent = right.relative_path.endsWith(`${sep}ha.sock`) ? 0 : 1;
      return (
        leftHostagent - rightHostagent ||
        left.relative_path.localeCompare(right.relative_path)
      );
    });
  if (sockets.length !== 2) fail("controlled background socket inventory was refused");
  const files = inventory.entries
    .filter((entry) => entry.kind === "file" && entry.relative_path !== OWNER_MARKER)
    .sort(
      (left, right) =>
        pathDepth(right.relative_path) - pathDepth(left.relative_path) ||
        left.relative_path.localeCompare(right.relative_path),
    );
  const directories = inventory.entries
    .filter((entry) => entry.kind === "directory")
    .sort(
      (left, right) =>
        pathDepth(right.relative_path) - pathDepth(left.relative_path) ||
        left.relative_path.localeCompare(right.relative_path),
    );
  return [
    {
      action: "authenticated-hostagent-stop",
      resources: sockets.map((entry) => entry.relative_path),
    },
    ...files.map((entry) => ({ action: "unlink", resources: [entry.relative_path] })),
    ...directories.map((entry) => ({ action: "rmdir", resources: [entry.relative_path] })),
    { action: "unlink-owner", resources: [OWNER_MARKER] },
    { action: "rmdir-root", resources: ["."] },
  ].map((step, sequence) => ({ ...step, sequence }));
}

function directoryIdentity(path, label) {
  const metadata = secureDirectory(path, label);
  return {
    device: String(metadata.dev),
    inode: String(metadata.ino),
    mode: "0700",
    path,
    uid: String(metadata.uid),
  };
}

function validateRetirementBindings(value) {
  exactKeys(
    value,
    [
      "cleanup_intent_sha256",
      "cleanup_slot_sequence",
      "cleanup_slot_sha256",
      "create_close_sha256",
      "create_slot_sha256",
      "source_head_sha256",
      "source_sequence",
    ],
    "controlled background retirement bindings",
  );
  if (
    !lowerHex(value.cleanup_intent_sha256, 64) ||
    !Number.isSafeInteger(value.cleanup_slot_sequence) ||
    value.cleanup_slot_sequence < 1 ||
    value.cleanup_slot_sequence > 63 ||
    !lowerHex(value.cleanup_slot_sha256, 64) ||
    !lowerHex(value.create_close_sha256, 64) ||
    !lowerHex(value.create_slot_sha256, 64) ||
    !lowerHex(value.source_head_sha256, 64) ||
    !Number.isSafeInteger(value.source_sequence) ||
    value.source_sequence < 1 ||
    value.source_sequence > 63
  ) {
    fail("controlled background retirement bindings were refused", 64);
  }
}

function validateRootOwner(value, expected) {
  exactKeys(
    value,
    [
      "create_authority_sha256",
      "fixture_id",
      "ownership_nonce",
      "provider_contract_sha256",
      "provider_kind",
      "provider_profile",
      "root_path",
      "schema",
    ],
    "controlled background root owner",
  );
  if (
    value.schema !== "synveda.clean-engine.background-provider-root-owner.v2" ||
    value.fixture_id !== expected.fixtureId ||
    value.create_authority_sha256 !== expected.createAuthoritySha256 ||
    !lowerHex(value.ownership_nonce, 64) ||
    value.provider_contract_sha256 !== CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256 ||
    value.provider_kind !== "controlled-background-fake" ||
    value.provider_profile !== expected.paths.profile ||
    value.root_path !== expected.paths.root
  ) {
    fail("controlled background root owner was refused");
  }
}

export async function planControlledBackgroundRetirement({
  bindings,
  evidenceDirectory,
  fixtureId,
  providerBase,
}) {
  validateRetirementBindings(bindings);
  let canonicalBase;
  try {
    canonicalBase = realpathSync(providerBase);
  } catch {
    fail("controlled background provider base was unavailable", 69);
  }
  if (canonicalBase !== providerBase || resolve(providerBase) !== providerBase) {
    fail("controlled background provider base was refused");
  }
  const paths = rootPaths(providerBase, fixtureId);
  const evidence = inspectControlledBackgroundProvider(evidenceDirectory, fixtureId, {
    revalidateCurrentToolchain: true,
  });
  if (bindings.create_slot_sha256 !== evidence.createAuthority.value.create_slot_sha256) {
    fail("controlled background create slot binding was refused");
  }
  const owner = canonicalArtifact(paths.ownerMarker, "controlled background root owner");
  validateRootOwner(owner.value, {
    createAuthoritySha256: evidence.createAuthority.sha256,
    fixtureId,
    paths,
  });
  if (
    owner.sha256 !== evidence.providerIdentity.value.root_owner_sha256 ||
    owner.sha256 !== evidence.controllerWitness.value.root_owner_sha256
  ) {
    fail("controlled background root owner binding was refused");
  }
  const instanceNonce = owner.value.ownership_nonce;
  const pidRecord = revalidateHostagentPidRecord(
    paths,
    fixtureId,
    evidence,
    instanceNonce,
  );
  const presence = processPresence(pidRecord.value.pid);
  if (presence === "unknown") {
    fail("controlled background hostagent identity was unavailable", 69);
  }
  if (presence === "present") {
    const hostagentProbe = await probeHostagent(
      paths,
      fixtureId,
      instanceNonce,
      pidRecord.value.pid,
      evidence.hostagentWitness.value.process_instance_sha256,
    );
    const engineProbe = await probeEngine(
      paths,
      fixtureId,
      instanceNonce,
      evidence.hostagentWitness.value.process_instance_sha256,
    );
    if (
      providerProcessDigest(providerProcessBytes(stableHostagentProbe(hostagentProbe))) !==
        evidence.controllerSettlement.value.hostagent_after_controller_sha256 ||
      providerProcessDigest(providerProcessBytes(stableEngineProbe(engineProbe))) !==
        evidence.controllerSettlement.value.engine_after_controller_sha256
    ) {
      fail("controlled background running identity changed");
    }
  }
  const inventory = creationInventoryForPlanning(
    paths,
    evidence.providerIdentity.value,
    presence === "absent",
  );
  const planValue = {
    base: directoryIdentity(providerBase, "controlled background provider base"),
    cleanup_intent_sha256: bindings.cleanup_intent_sha256,
    cleanup_slot_sequence: bindings.cleanup_slot_sequence,
    cleanup_slot_sha256: bindings.cleanup_slot_sha256,
    create_close_sha256: bindings.create_close_sha256,
    create_operation_evidence_sha256: evidence.providerIdentity.sha256,
    create_slot_sha256: bindings.create_slot_sha256,
    fixture_id: fixtureId,
    provider_contract_sha256: CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256,
    provider_effect_sha256: ZERO_SHA256,
    provider_identity_sha256: evidence.providerIdentity.sha256,
    provider_root_owner_sha256: owner.sha256,
    provider_root_plan_sha256: ZERO_SHA256,
    retirement_steps: retirementSteps(inventory),
    root: inventory.root,
    root_inventory: inventory.entries,
    schema: "synveda.clean-engine.controlled-background-provider-retirement-plan.v1",
    source_head_sha256: bindings.source_head_sha256,
    source_sequence: bindings.source_sequence,
    state_integration: "not-authorized",
  };
  const plan = publishArtifact(
    evidenceDirectory,
    "provider-retirement-plan.json",
    planValue,
  );
  return Object.freeze({ evidence, instanceNonce, paths, plan });
}

function validateDirectoryIdentity(value, label) {
  exactKeys(value, ["device", "inode", "mode", "path", "uid"], label);
  if (
    !decimalString(value.device) ||
    !decimalString(value.inode) ||
    value.mode !== "0700" ||
    typeof value.path !== "string" ||
    !isAbsolute(value.path) ||
    !decimalString(value.uid)
  ) {
    fail(`${label} was refused`);
  }
}

function validateRetirementPlan(value, evidence, paths, planSha256) {
  exactKeys(
    value,
    [
      "base",
      "cleanup_intent_sha256",
      "cleanup_slot_sequence",
      "cleanup_slot_sha256",
      "create_close_sha256",
      "create_operation_evidence_sha256",
      "create_slot_sha256",
      "fixture_id",
      "provider_contract_sha256",
      "provider_effect_sha256",
      "provider_identity_sha256",
      "provider_root_owner_sha256",
      "provider_root_plan_sha256",
      "retirement_steps",
      "root",
      "root_inventory",
      "schema",
      "source_head_sha256",
      "source_sequence",
      "state_integration",
    ],
    "controlled background retirement plan",
  );
  validateDirectoryIdentity(value.base, "controlled background retirement base");
  validateDirectoryIdentity(value.root, "controlled background retirement root");
  validateRetirementBindings({
    cleanup_intent_sha256: value.cleanup_intent_sha256,
    cleanup_slot_sequence: value.cleanup_slot_sequence,
    cleanup_slot_sha256: value.cleanup_slot_sha256,
    create_close_sha256: value.create_close_sha256,
    create_slot_sha256: value.create_slot_sha256,
    source_head_sha256: value.source_head_sha256,
    source_sequence: value.source_sequence,
  });
  if (
    value.schema !==
      "synveda.clean-engine.controlled-background-provider-retirement-plan.v1" ||
    value.fixture_id !== evidence.providerIdentity.value.fixture_id ||
    value.provider_contract_sha256 !== CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256 ||
    value.provider_effect_sha256 !== ZERO_SHA256 ||
    value.provider_identity_sha256 !== evidence.providerIdentity.sha256 ||
    value.create_operation_evidence_sha256 !== evidence.providerIdentity.sha256 ||
    value.provider_root_owner_sha256 !== evidence.providerIdentity.value.root_owner_sha256 ||
    value.provider_root_plan_sha256 !== ZERO_SHA256 ||
    value.root.path !== paths.root ||
    value.base.path !== paths.base ||
    value.state_integration !== "not-authorized" ||
    !lowerHex(planSha256, 64) ||
    !Array.isArray(value.root_inventory) ||
    !Array.isArray(value.retirement_steps) ||
    canonical(value.root) !== canonical(evidence.providerIdentity.value.provider_root) ||
    canonical(value.root_inventory) !==
      canonical(evidence.providerIdentity.value.provider_root_inventory)
  ) {
    fail("controlled background retirement plan was refused");
  }
  for (const entry of value.root_inventory) {
    if (!new Set(["directory", "file", "socket"]).has(entry.kind)) {
      fail("controlled background retirement inventory kind was refused");
    }
    validateResourceIdentity(entry, entry.kind, "controlled background retirement inventory");
  }
  if (
    new Set(value.root_inventory.map((entry) => entry.relative_path)).size !==
      value.root_inventory.length ||
    canonical(value.root_inventory.map((entry) => entry.relative_path).sort()) !==
      canonical(expectedRootPaths(paths))
  ) {
    fail("controlled background retirement inventory paths were refused");
  }
  const expectedSteps = retirementSteps({ entries: value.root_inventory });
  if (canonical(value.retirement_steps) !== canonical(expectedSteps)) {
    fail("controlled background retirement order was refused");
  }
}

function exactProgressiveIdentity(current, planned) {
  if (
    current.kind !== planned.kind ||
    current.device !== planned.device ||
    current.inode !== planned.inode ||
    current.mode !== planned.mode ||
    current.relative_path !== planned.relative_path ||
    current.sha256 !== planned.sha256 ||
    current.uid !== planned.uid
  ) {
    return false;
  }
  if (current.kind === "directory") {
    return BigInt(current.links) >= 2n && BigInt(current.links) <= BigInt(planned.links);
  }
  return current.links === planned.links && current.size === planned.size;
}

function completedResourceSet(plan, completedCount) {
  const completed = new Set();
  for (const step of plan.retirement_steps.slice(0, completedCount)) {
    for (const resource of step.resources) {
      if (resource !== ".") completed.add(resource);
    }
  }
  return completed;
}

function assertRootSubset(paths, plan, completedCount, additionallyCompleted = new Set()) {
  const completed = completedResourceSet(plan, completedCount);
  for (const resource of additionallyCompleted) completed.add(resource);
  const rootRemoved = plan.retirement_steps
    .slice(0, completedCount)
    .some((step) => step.action === "rmdir-root");
  const currentBase = directoryIdentity(paths.base, "controlled background provider base");
  if (canonical(currentBase) !== canonical(plan.base)) {
    fail("controlled background provider base identity changed");
  }
  if (rootRemoved) {
    try {
      lstatSync(paths.root);
      fail("controlled background provider root reappeared");
    } catch (error) {
      if (error instanceof ProviderProcessContractFailure) throw error;
      if (error?.code !== "ENOENT") fail("controlled background root absence was unavailable", 69);
    }
    return Object.freeze({ entries: [], rootAbsent: true });
  }
  const current = scanRootInventory(paths);
  if (canonical(current.root) !== canonical(plan.root)) {
    fail("controlled background provider root identity changed");
  }
  const expected = plan.root_inventory.filter(
    (entry) => !completed.has(entry.relative_path),
  );
  if (
    canonical(current.entries.map((entry) => entry.relative_path)) !==
    canonical(expected.map((entry) => entry.relative_path).sort())
  ) {
    fail("controlled background retirement subset was refused");
  }
  const plannedByPath = new Map(plan.root_inventory.map((entry) => [entry.relative_path, entry]));
  for (const entry of current.entries) {
    const planned = plannedByPath.get(entry.relative_path);
    if (planned === undefined || !exactProgressiveIdentity(entry, planned)) {
      fail(`controlled background retirement identity changed at ${entry.relative_path}`);
    }
  }
  return Object.freeze({ entries: current.entries, rootAbsent: false });
}

function stepIdentitySha256(plan, step) {
  const byPath = new Map(plan.root_inventory.map((entry) => [entry.relative_path, entry]));
  const identities = step.resources.map((resource) => {
    if (resource === ".") return plan.root;
    const identity = byPath.get(resource);
    if (identity === undefined) fail("controlled background retirement resource was refused");
    return identity;
  });
  return providerProcessDigest(providerProcessBytes(identities));
}

function readProgress(evidenceDirectory, planArtifact) {
  validateEvidenceDirectoryInventory(evidenceDirectory);
  for (const name of readdirSync(evidenceDirectory)) {
    const stage = parseArtifactStageName(name);
    if (stage === undefined || !stage.targetName.startsWith("retirement-step-")) continue;
    const sequence = Number.parseInt(stage.targetName.slice("retirement-step-".length, -5), 10);
    if (
      !Number.isSafeInteger(sequence) ||
      sequence < 0 ||
      sequence >= planArtifact.value.retirement_steps.length
    ) {
      fail("controlled background retirement progress stage exceeded its plan");
    }
  }
  const names = readdirSync(evidenceDirectory)
    .filter((name) => /^retirement-step-[0-9]{2}\.json$/.test(name))
    .sort();
  const progress = [];
  let previousSha256 = planArtifact.sha256;
  for (const [sequence, name] of names.entries()) {
    if (name !== `retirement-step-${String(sequence).padStart(2, "0")}.json`) {
      fail("controlled background retirement progress was not contiguous");
    }
    const artifact = canonicalArtifact(join(evidenceDirectory, name), name);
    const step = planArtifact.value.retirement_steps[sequence];
    exactKeys(
      artifact.value,
      [
        "action",
        "fixture_id",
        "plan_sha256",
        "previous_sha256",
        "recovered_absence",
        "resource_identity_sha256",
        "resources",
        "schema",
        "sequence",
      ],
      "controlled background retirement progress",
    );
    if (
      step === undefined ||
      artifact.value.schema !== "synveda.clean-engine.provider-retirement-step.v1" ||
      artifact.value.fixture_id !== planArtifact.value.fixture_id ||
      artifact.value.plan_sha256 !== planArtifact.sha256 ||
      artifact.value.previous_sha256 !== previousSha256 ||
      artifact.value.sequence !== sequence ||
      artifact.value.action !== step.action ||
      canonical(artifact.value.resources) !== canonical(step.resources) ||
      artifact.value.resource_identity_sha256 !== stepIdentitySha256(planArtifact.value, step) ||
      typeof artifact.value.recovered_absence !== "boolean"
    ) {
      fail("controlled background retirement progress was refused");
    }
    progress.push(artifact);
    previousSha256 = artifact.sha256;
  }
  for (const name of readdirSync(evidenceDirectory)) {
    const stage = parseArtifactStageName(name);
    if (stage === undefined || !stage.targetName.startsWith("retirement-step-")) continue;
    const sequence = Number.parseInt(stage.targetName.slice("retirement-step-".length, -5), 10);
    if (sequence !== progress.length) {
      fail("controlled background retirement progress stage was not the next slot");
    }
  }
  return progress;
}

function progressValue(planArtifact, progress, recoveredAbsence) {
  const sequence = progress.length;
  const step = planArtifact.value.retirement_steps[sequence];
  if (step === undefined) fail("controlled background retirement was already complete");
  return {
    action: step.action,
    fixture_id: planArtifact.value.fixture_id,
    plan_sha256: planArtifact.sha256,
    previous_sha256: progress.at(-1)?.sha256 ?? planArtifact.sha256,
    recovered_absence: recoveredAbsence,
    resource_identity_sha256: stepIdentitySha256(planArtifact.value, step),
    resources: step.resources,
    schema: "synveda.clean-engine.provider-retirement-step.v1",
    sequence,
  };
}

function progressName(progress) {
  return `retirement-step-${String(progress.length).padStart(2, "0")}.json`;
}

function progressStages(evidenceDirectory, progress) {
  return artifactStages(evidenceDirectory, progressName(progress));
}

function selectStagedProgressRecovery(
  evidenceDirectory,
  planArtifact,
  progress,
  defaultRecoveredAbsence,
) {
  const stages = progressStages(evidenceDirectory, progress);
  if (stages.length === 0) return defaultRecoveredAbsence;
  const candidates = new Map(
    [false, true].map((recoveredAbsence) => {
      const bytes = providerProcessBytes(
        progressValue(planArtifact, progress, recoveredAbsence),
      );
      return [providerProcessDigest(bytes), recoveredAbsence];
    }),
  );
  const dispositions = new Set();
  for (const stage of stages) {
    const disposition = candidates.get(stage.sha256);
    if (disposition === undefined) {
      fail("controlled background retirement progress stage was refused");
    }
    dispositions.add(disposition);
  }
  if (dispositions.size !== 1) {
    fail("controlled background retirement progress stages disagreed");
  }
  return [...dispositions][0];
}

function publishProgress(evidenceDirectory, planArtifact, progress, recoveredAbsence) {
  const name = progressName(progress);
  return publishArtifact(
    evidenceDirectory,
    name,
    progressValue(planArtifact, progress, recoveredAbsence),
  );
}

function processPresence(pid) {
  if (!Number.isSafeInteger(pid) || pid < 2) fail("hostagent PID was refused");
  try {
    process.kill(pid, 0);
    return "present";
  } catch (error) {
    if (error?.code === "ESRCH") return "absent";
    if (error?.code === "EPERM") return "unknown";
    fail("hostagent process probe failed", 69);
  }
}

function pathEntryExists(path) {
  try {
    lstatSync(path);
    return true;
  } catch (error) {
    if (error?.code === "ENOENT") return false;
    fail("controlled background resource presence was unavailable", 69);
  }
}

function syncSocketAbsence(paths) {
  for (const parent of new Set([dirname(paths.haSocket), dirname(paths.engineSocket)])) {
    syncDirectory(parent);
  }
  if (pathEntryExists(paths.haSocket) || pathEntryExists(paths.engineSocket)) {
    fail("controlled background socket absence changed");
  }
}

async function waitForHostagentAbsence(pid, paths, timeoutMilliseconds = 5_000) {
  const deadline = Date.now() + timeoutMilliseconds;
  while (Date.now() < deadline) {
    if (
      processPresence(pid) === "absent" &&
      !pathEntryExists(paths.haSocket) &&
      !pathEntryExists(paths.engineSocket)
    ) {
      return;
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 10));
  }
  fail("controlled background hostagent remained present", 69);
}

async function stopHostagent(paths, planArtifact, evidence, instanceNonce) {
  const pidRecord = revalidateHostagentPidRecord(
    paths,
    planArtifact.value.fixture_id,
    evidence,
    instanceNonce,
  );
  const pid = pidRecord.value.pid;
  const processIdentity = evidence.hostagentWitness.value.process_instance_sha256;
  const socketStep = planArtifact.value.retirement_steps[0];
  if (socketStep?.action !== "authenticated-hostagent-stop" || socketStep.resources.length !== 2) {
    fail("controlled background socket retirement plan was refused");
  }
  const presence = processPresence(pid);
  const presentResources = socketStep.resources.filter((resource) =>
    pathEntryExists(join(paths.root, resource)),
  );
  if (presence === "absent") {
    const completed = new Set(
      socketStep.resources.filter((resource) => !presentResources.includes(resource)),
    );
    assertRootSubset(paths, planArtifact.value, 0, completed);
    const plannedByPath = new Map(
      planArtifact.value.root_inventory.map((entry) => [entry.relative_path, entry]),
    );
    for (const resource of presentResources) {
      const path = join(paths.root, resource);
      const planned = plannedByPath.get(resource);
      const current = inventoryIdentity(path, paths.root, BigInt(planArtifact.value.root.device));
      if (planned === undefined || !exactProgressiveIdentity(current, planned)) {
        fail("controlled background stale socket identity changed");
      }
      try {
        unlinkSync(path);
        syncDirectory(dirname(path));
      } catch {
        fail("controlled background stale socket retirement failed", 70);
      }
      completed.add(resource);
      assertRootSubset(paths, planArtifact.value, 0, completed);
    }
    syncSocketAbsence(paths);
    assertRootSubset(paths, planArtifact.value, 1);
    return true;
  }
  if (presence !== "present") {
    fail("controlled background hostagent identity remained uncertain", 73);
  }
  if (presentResources.length !== socketStep.resources.length) {
    if (presentResources.length === 0) {
      fail("controlled background hostagent identity remained uncertain", 73);
    }
    fail("controlled background socket retirement was partial");
  }
  assertRootSubset(paths, planArtifact.value, 0);
  await probeHostagent(
    paths,
    planArtifact.value.fixture_id,
    instanceNonce,
    pid,
    processIdentity,
  );
  await probeEngine(
    paths,
    planArtifact.value.fixture_id,
    instanceNonce,
    processIdentity,
  );
  const challenge = randomBytes(32).toString("hex");
  const response = await requestSocket(paths.haSocket, {
    action: "shutdown",
    challenge,
    proof_sha256: proof(instanceNonce, "hostagent-shutdown", challenge, processIdentity),
  });
  exactKeys(
    response,
    [
      "challenge_sha256",
      "fixture_id",
      "process_instance_sha256",
      "proof_sha256",
      "schema",
    ],
    "controlled background shutdown acknowledgement",
  );
  if (
    response.schema !== "synveda.clean-engine.background-hostagent-shutdown.v1" ||
    response.fixture_id !== planArtifact.value.fixture_id ||
    response.process_instance_sha256 !== processIdentity ||
    response.challenge_sha256 !== providerProcessDigest(Buffer.from(challenge, "ascii")) ||
    !proofEquals(
      response.proof_sha256,
      proof(instanceNonce, "hostagent-shutdown-accepted", challenge, processIdentity),
    )
  ) {
    fail("controlled background shutdown acknowledgement was refused");
  }
  await waitForHostagentAbsence(pid, paths);
  syncSocketAbsence(paths);
  assertRootSubset(paths, planArtifact.value, 1);
  return false;
}

function resourceExists(paths, resource) {
  if (resource === ".") return pathEntryExists(paths.root);
  return pathEntryExists(join(paths.root, resource));
}

function syncDeletionAbsence(paths, step) {
  if (step.resources.length !== 1) {
    fail("controlled background deletion step was refused");
  }
  const resource = step.resources[0];
  const path = resource === "." ? paths.root : join(paths.root, resource);
  syncDirectory(dirname(path));
  if (pathEntryExists(path)) {
    fail("controlled background deletion absence changed");
  }
}

function deleteStep(paths, plan, step) {
  if (step.resources.length !== 1) fail("controlled background deletion step was refused");
  const resource = step.resources[0];
  const path = resource === "." ? paths.root : join(paths.root, resource);
  if (resource === ".") {
    if (
      canonical(directoryIdentity(path, "controlled background retirement root")) !==
      canonical(plan.root)
    ) {
      fail("controlled background deletion identity changed");
    }
  } else {
    const planned = plan.root_inventory.find((entry) => entry.relative_path === resource);
    const current = inventoryIdentity(path, paths.root, BigInt(plan.root.device));
    if (planned === undefined || !exactProgressiveIdentity(current, planned)) {
      fail("controlled background deletion identity changed");
    }
  }
  if (step.action === "unlink" || step.action === "unlink-owner") {
    try {
      unlinkSync(path);
    } catch {
      fail("controlled background unlink failed", 70);
    }
    return;
  }
  if (step.action === "rmdir" || step.action === "rmdir-root") {
    if (readdirSync(path).length !== 0) {
      fail("controlled background directory was not empty");
    }
    try {
      rmdirSync(path);
    } catch {
      fail("controlled background directory retirement failed", 70);
    }
    return;
  }
  fail("controlled background deletion action was refused");
}

function validateRetirementSettlement(value, planArtifact, evidence, progress) {
  exactKeys(
    value,
    [
      "cleanup_plan_sha256",
      "cleanup_slot_sha256",
      "create_operation_evidence_sha256",
      "final_progress_sha256",
      "fixture_id",
      "outcome",
      "provider_identity_sha256",
      "provider_kind",
      "resources",
      "result_receipt_authorized",
      "root_disposition",
      "safe_code",
      "schema",
      "source_closure",
      "state_integration",
    ],
    "controlled background retirement settlement",
  );
  exactKeys(
    value.resources,
    [
      "docker_context",
      "engine",
      "engine_socket",
      "hostagent",
      "hostagent_socket",
      "provider_root",
    ],
    "controlled background retired resources",
  );
  const retiredResources = {
    docker_context: "retired",
    engine: "retired",
    engine_socket: "retired",
    hostagent: "retired",
    hostagent_socket: "retired",
    provider_root: "retired",
  };
  if (
    value.schema !==
      "synveda.clean-engine.controlled-background-provider-retirement-settlement.v1" ||
    value.fixture_id !== planArtifact.value.fixture_id ||
    value.provider_kind !== "controlled-background-fake" ||
    value.cleanup_plan_sha256 !== planArtifact.sha256 ||
    value.cleanup_slot_sha256 !== planArtifact.value.cleanup_slot_sha256 ||
    value.create_operation_evidence_sha256 !== evidence.providerIdentity.sha256 ||
    value.provider_identity_sha256 !== evidence.providerIdentity.sha256 ||
    progress.length !== planArtifact.value.retirement_steps.length ||
    value.final_progress_sha256 !== progress.at(-1)?.sha256 ||
    value.outcome !== "passed" ||
    value.root_disposition !== "retired" ||
    value.safe_code !== "none" ||
    value.result_receipt_authorized !== false ||
    value.source_closure !== "state-integration-required" ||
    value.state_integration !== "not-authorized" ||
    canonical(value.resources) !== canonical(retiredResources)
  ) {
    fail("controlled background retirement settlement was refused");
  }
}

export async function retireControlledBackgroundProvider({
  crashAfterDeleteSyscallSequence,
  crashAfterDeleteSequence,
  crashAfterHostagentSettlement = false,
  evidenceDirectory,
  fixtureId,
  providerBase,
  stopAfterSequence,
}) {
  if (
    crashAfterDeleteSyscallSequence !== undefined &&
    (!Number.isSafeInteger(crashAfterDeleteSyscallSequence) ||
      crashAfterDeleteSyscallSequence < 1)
  ) {
    fail("controlled background syscall crash sequence was refused", 64);
  }
  if (
    crashAfterDeleteSequence !== undefined &&
    (!Number.isSafeInteger(crashAfterDeleteSequence) || crashAfterDeleteSequence < 1)
  ) {
    fail("controlled background crash sequence was refused", 64);
  }
  if (typeof crashAfterHostagentSettlement !== "boolean") {
    fail("controlled background hostagent crash point was refused", 64);
  }
  if (
    stopAfterSequence !== undefined &&
    (!Number.isSafeInteger(stopAfterSequence) || stopAfterSequence < 0)
  ) {
    fail("controlled background stop sequence was refused", 64);
  }
  const paths = rootPaths(providerBase, fixtureId);
  const evidence = inspectControlledBackgroundProvider(evidenceDirectory, fixtureId);
  const planArtifact = canonicalArtifact(
    join(evidenceDirectory, "provider-retirement-plan.json"),
    "provider-retirement-plan.json",
  );
  validateRetirementPlan(planArtifact.value, evidence, paths, planArtifact.sha256);
  let progress = readProgress(evidenceDirectory, planArtifact);
  if (progress.length > planArtifact.value.retirement_steps.length) {
    fail("controlled background retirement progress overflowed");
  }
  const settlementName = "provider-retirement-settlement.json";
  const settlementPath = join(evidenceDirectory, settlementName);
  const settlementStages = artifactStages(evidenceDirectory, settlementName);
  const settlementExists = pathEntryExists(settlementPath);
  if (settlementExists || settlementStages.length > 0) {
    if (progress.length !== planArtifact.value.retirement_steps.length) {
      fail("controlled background retirement settlement preceded completion");
    }
    assertRootSubset(paths, planArtifact.value, progress.length);
    if (settlementExists) {
      const settlement = canonicalArtifact(settlementPath, settlementName);
      validateRetirementSettlement(settlement.value, planArtifact, evidence, progress);
      return Object.freeze({
        cleanup_operation_evidence_sha256: settlement.sha256,
        complete: true,
        completed_steps: progress.length,
        create_operation_evidence_sha256: evidence.providerIdentity.sha256,
        settlement,
      });
    }
  }
  if (progress.length === 0) {
    const firstStep = planArtifact.value.retirement_steps[0];
    if (
      progressStages(evidenceDirectory, progress).length > 0 &&
      firstStep.resources.some((resource) => resourceExists(paths, resource))
    ) {
      fail("controlled background retirement progress stage preceded mutation");
    }
    const marker = canonicalArtifact(paths.ownerMarker, "controlled background root owner");
    validateRootOwner(marker.value, {
      createAuthoritySha256: evidence.createAuthority.sha256,
      fixtureId,
      paths,
    });
    if (marker.sha256 !== planArtifact.value.provider_root_owner_sha256) {
      fail("controlled background root owner changed");
    }
    const recoveredAbsence = await stopHostagent(
      paths,
      planArtifact,
      evidence,
      marker.value.ownership_nonce,
    );
    if (crashAfterHostagentSettlement) {
      fail("simulated controlled background hostagent-settlement crash", 75);
    }
    const progressDisposition = selectStagedProgressRecovery(
      evidenceDirectory,
      planArtifact,
      progress,
      recoveredAbsence,
    );
    progress.push(
      publishProgress(evidenceDirectory, planArtifact, progress, progressDisposition),
    );
    if (stopAfterSequence === 0) {
      return Object.freeze({ complete: false, completed_steps: progress.length });
    }
  }
  while (progress.length < planArtifact.value.retirement_steps.length) {
    const sequence = progress.length;
    const step = planArtifact.value.retirement_steps[sequence];
    const present = step.resources.map((resource) => resourceExists(paths, resource));
    if (present.some((value) => value) && !present.every((value) => value)) {
      fail("controlled background retirement step was partial");
    }
    if (
      progressStages(evidenceDirectory, progress).length > 0 &&
      present.some((value) => value)
    ) {
      fail("controlled background retirement progress stage preceded mutation");
    }
    const recoveredAbsence = present.every((value) => !value);
    if (recoveredAbsence) {
      syncDeletionAbsence(paths, step);
      assertRootSubset(paths, planArtifact.value, sequence + 1);
    } else {
      assertRootSubset(paths, planArtifact.value, sequence);
      deleteStep(paths, planArtifact.value, step);
      if (crashAfterDeleteSyscallSequence === sequence) {
        fail("simulated controlled background pre-fsync retirement crash", 75);
      }
      syncDeletionAbsence(paths, step);
      assertRootSubset(paths, planArtifact.value, sequence + 1);
      if (crashAfterDeleteSequence === sequence) {
        fail("simulated controlled background retirement crash", 75);
      }
    }
    const progressDisposition = selectStagedProgressRecovery(
      evidenceDirectory,
      planArtifact,
      progress,
      recoveredAbsence,
    );
    progress.push(
      publishProgress(evidenceDirectory, planArtifact, progress, progressDisposition),
    );
    if (stopAfterSequence === sequence) {
      return Object.freeze({ complete: false, completed_steps: progress.length });
    }
  }
  assertRootSubset(paths, planArtifact.value, progress.length);
  const settlementValue = {
    cleanup_plan_sha256: planArtifact.sha256,
    cleanup_slot_sha256: planArtifact.value.cleanup_slot_sha256,
    create_operation_evidence_sha256: evidence.providerIdentity.sha256,
    final_progress_sha256: progress.at(-1).sha256,
    fixture_id: fixtureId,
    outcome: "passed",
    provider_identity_sha256: evidence.providerIdentity.sha256,
    provider_kind: "controlled-background-fake",
    resources: {
      docker_context: "retired",
      engine: "retired",
      engine_socket: "retired",
      hostagent: "retired",
      hostagent_socket: "retired",
      provider_root: "retired",
    },
    result_receipt_authorized: false,
    root_disposition: "retired",
    safe_code: "none",
    schema: "synveda.clean-engine.controlled-background-provider-retirement-settlement.v1",
    source_closure: "state-integration-required",
    state_integration: "not-authorized",
  };
  const settlement = publishArtifact(
    evidenceDirectory,
    "provider-retirement-settlement.json",
    settlementValue,
  );
  validateRetirementSettlement(settlement.value, planArtifact, evidence, progress);
  assertRootSubset(paths, planArtifact.value, progress.length);
  return Object.freeze({
    cleanup_operation_evidence_sha256: settlement.sha256,
    complete: true,
    completed_steps: progress.length,
    create_operation_evidence_sha256: evidence.providerIdentity.sha256,
    settlement,
  });
}

export function controlledBackgroundOperationEvidence({ action, evidenceDirectory, fixtureId }) {
  const evidence = inspectControlledBackgroundProvider(evidenceDirectory, fixtureId);
  if (action === "provider-create") return evidence.providerIdentity.sha256;
  if (action === "provider-cleanup") {
    const settlement = canonicalArtifact(
      join(evidenceDirectory, "provider-retirement-settlement.json"),
      "provider-retirement-settlement.json",
    );
    const planArtifact = canonicalArtifact(
      join(evidenceDirectory, "provider-retirement-plan.json"),
      "provider-retirement-plan.json",
    );
    const paths = rootPaths(planArtifact.value.base.path, fixtureId);
    validateRetirementPlan(planArtifact.value, evidence, paths, planArtifact.sha256);
    const progress = readProgress(evidenceDirectory, planArtifact);
    validateRetirementSettlement(settlement.value, planArtifact, evidence, progress);
    assertRootSubset(paths, planArtifact.value, progress.length);
    return settlement.sha256;
  }
  fail("controlled background operation action was refused", 64);
}
