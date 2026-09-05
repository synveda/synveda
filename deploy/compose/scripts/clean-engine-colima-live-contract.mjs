#!/usr/bin/env node
import { createHash, createHmac, timingSafeEqual } from "node:crypto";
import {
  closeSync,
  constants,
  fstatSync,
  lstatSync,
  openSync,
  readSync,
  readdirSync,
  realpathSync,
} from "node:fs";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";

const ZERO_SHA256 = "0".repeat(64);
const MAX_COMPONENTS = 16;
const HASH_CHUNK_BYTES = 1024 * 1024;

export const COLIMA_LIVE_REQUIREMENTS_SCHEMA =
  "synveda.clean-engine.colima-live-requirements.v1";
export const COLIMA_LIVE_OBSERVATION_SCHEMA =
  "synveda.clean-engine.colima-live-observation.v1";
export const COLIMA_LIVE_PUBLIC_PROJECTION_SCHEMA =
  "synveda.clean-engine.colima-live-public-projection.v1";
export const COLIMA_LIVE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA =
  "synveda.clean-engine.colima-live-pre-effect-root-observation.v1";
export const COLIMA_LIVE_FIXTURE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-pre-effect-root-observation.v1";

const ROOT_LAYOUT = Object.freeze({
  artifact_directory: "a",
  colima_cache_home: "k",
  colima_home: "c",
  docker_config: "d",
  lima_home: "l",
  temporary_directory: "t",
  toolchain_directory: "b",
});

const ENVIRONMENT_NAMES = Object.freeze([
  "COLIMA_CACHE_HOME",
  "COLIMA_DOWNLOADER",
  "COLIMA_HOME",
  "DOCKER_CONFIG",
  "HOME",
  "LANG",
  "LC_ALL",
  "LIMA_HOME",
  "PATH",
  "SSH",
  "TMPDIR",
  "XPC_SERVICE_NAME",
]);

const FORBIDDEN_ENVIRONMENT_NAMES = Object.freeze([
  "COLIMA_SAVE_CONFIG",
  "DOCKER_CERT_PATH",
  "DOCKER_CONTEXT",
  "DOCKER_HOST",
  "DOCKER_TLS_VERIFY",
  "DYLD_INSERT_LIBRARIES",
  "DYLD_LIBRARY_PATH",
  "HTTPS_PROXY",
  "HTTP_PROXY",
  "LIMA_SSH_OVER_VSOCK",
  "NO_PROXY",
  "SSH_AUTH_SOCK",
  "XDG_CACHE_HOME",
  "XDG_CONFIG_HOME",
  "https_proxy",
  "http_proxy",
  "no_proxy",
]);

const LIMA_NETWORK_CONFIG_BYTES = Buffer.from(
  "networks:\n" +
    "  user-v2:\n" +
    "    mode: user-v2\n" +
    "    gateway: 192.168.104.1\n" +
    "    netmask: 255.255.255.0\n",
  "utf8",
);

const COMPONENT_ROLES = Object.freeze([
  "colima-binary",
  "docker-cli-binary",
  "lima-guestagent",
  "lima-network-config",
  "lima-wrapper",
  "limactl-binary",
  "ssh-client",
  "ssh-keygen",
  "state-owner-node",
  "state-owner-script",
  "sw-vers",
  "system-profiler",
]);

const DIRECTORY_ROLES = Object.freeze([
  "artifact-directory",
  "colima-cache-home",
  "colima-home",
  "docker-config",
  "lima-config-directory",
  "lima-home",
  "provider-root",
  "temporary-directory",
  "toolchain-directory",
]);

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
  throw new ColimaLiveContractFailure("Colima live canonical value was refused", 70);
}

export function colimaLiveBytes(value) {
  return Buffer.from(`${canonical(value)}\n`, "utf8");
}

export function colimaLiveDigest(value) {
  return createHash("sha256").update(value).digest("hex");
}

function deepFreeze(value) {
  if (value !== null && typeof value === "object" && !Object.isFrozen(value)) {
    for (const child of Object.values(value)) deepFreeze(child);
    Object.freeze(value);
  }
  return value;
}

function component({
  expectedSha256 = ZERO_SHA256,
  expectedSize = "0",
  kind,
  location,
  maximumSize,
  modePolicy,
  provenance,
  role,
  stageRelativePath,
}) {
  return {
    expected_sha256: expectedSha256,
    expected_size: expectedSize,
    kind,
    location,
    maximum_size: maximumSize,
    mode_policy: modePolicy,
    provenance,
    role,
    stage_relative_path: stageRelativePath,
  };
}

const NETWORK_CONFIG_SHA256 = colimaLiveDigest(LIMA_NETWORK_CONFIG_BYTES);

export const COLIMA_LIVE_REQUIREMENTS = deepFreeze({
  authorizations: {
    execution_authorized: false,
    finalization_eligible: false,
    lifecycle_exposure_authorized: false,
  },
  command_template: [
    "<colima-binary>",
    "start",
    "<provider-profile>",
    "--foreground",
    "--runtime",
    "docker",
    "--vm-type",
    "vz",
    "--arch",
    "aarch64",
    "--hostname",
    "<lima-instance>",
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
    "--mount-type",
    "virtiofs",
    "--mount-inotify=false",
    "--ssh-agent=false",
    "--ssh-config=false",
    "--ssh-port",
    "0",
    "--activate=true",
    "--kubernetes=false",
    "--template=false",
    "--binfmt=false",
    "--vz-rosetta=false",
    "--network-address=false",
    "--network-host-addresses=false",
    "--network-preferred-route=false",
    "--save-config=false",
    "--force-disk-image=false",
    "--downloader",
    "native",
    "--port-forwarder",
    "ssh",
    "--disk-image",
    "<receipt-owned-disk-image>",
  ],
  components: [
    component({
      expectedSha256: "980ad8bf61a4ca370243f4cb41401a61276dcd2c2502bee7b9b86f9250169f34",
      expectedSize: "15656320",
      kind: "executable",
      location: "private-toolchain",
      maximumSize: "67108864",
      modePolicy: "private-executable-0500",
      provenance: "colima-v0.10.3-darwin-arm64-release",
      role: "colima-binary",
      stageRelativePath: "b/colima",
    }),
    component({
      kind: "executable",
      location: "private-toolchain",
      maximumSize: "268435456",
      modePolicy: "private-executable-0500",
      provenance: "selected-docker-cli-host-observation",
      role: "docker-cli-binary",
      stageRelativePath: "b/docker",
    }),
    component({
      expectedSha256: "d3fda5670ef5fcf14094efec95d410021cd4c585a2a1b6a16a97131f73fbe2f1",
      expectedSize: "7275764",
      kind: "data",
      location: "private-artifact",
      maximumSize: "16777216",
      modePolicy: "private-data-0400",
      provenance: "lima-v2.2.0-darwin-arm64-release",
      role: "lima-guestagent",
      stageRelativePath: "a/lima-guestagent.Linux-aarch64.gz",
    }),
    component({
      expectedSha256: NETWORK_CONFIG_SHA256,
      expectedSize: String(LIMA_NETWORK_CONFIG_BYTES.length),
      kind: "configuration",
      location: "private-lima-config",
      maximumSize: "4096",
      modePolicy: "private-data-0400",
      provenance: "synveda-user-v2-network-config-v1",
      role: "lima-network-config",
      stageRelativePath: "l/_config/networks.yaml",
    }),
    component({
      expectedSha256: "88aeac60dbcb69ec675c0ce9af24d5f66255f4899fa73320402db57925500832",
      expectedSize: "1607",
      kind: "executable",
      location: "private-toolchain",
      maximumSize: "4096",
      modePolicy: "private-executable-0500",
      provenance: "lima-v2.2.0-darwin-arm64-release",
      role: "lima-wrapper",
      stageRelativePath: "b/lima",
    }),
    component({
      expectedSha256: "f19a4fca3875e1017a5285672be4a62699c1e55918fb6a7afce86a14199e10d9",
      expectedSize: "32669616",
      kind: "executable",
      location: "private-toolchain",
      maximumSize: "67108864",
      modePolicy: "private-executable-0500",
      provenance: "lima-v2.2.0-darwin-arm64-release",
      role: "limactl-binary",
      stageRelativePath: "b/limactl",
    }),
    component({
      kind: "executable",
      location: "private-toolchain",
      maximumSize: "16777216",
      modePolicy: "private-executable-0500",
      provenance: "macos-build-bound-system-helper",
      role: "ssh-client",
      stageRelativePath: "b/ssh",
    }),
    component({
      kind: "executable",
      location: "private-toolchain",
      maximumSize: "16777216",
      modePolicy: "private-executable-0500",
      provenance: "macos-build-bound-system-helper",
      role: "ssh-keygen",
      stageRelativePath: "b/ssh-keygen",
    }),
    component({
      kind: "executable",
      location: "state-owner-runtime",
      maximumSize: "268435456",
      modePolicy: "external-executable-not-writable",
      provenance: "source-closure-bound-state-owner",
      role: "state-owner-node",
      stageRelativePath: "external",
    }),
    component({
      kind: "source",
      location: "state-owner-runtime",
      maximumSize: "2097152",
      modePolicy: "external-source-not-writable",
      provenance: "source-closure-bound-state-owner",
      role: "state-owner-script",
      stageRelativePath: "external",
    }),
    component({
      kind: "executable",
      location: "private-toolchain",
      maximumSize: "16777216",
      modePolicy: "private-executable-0500",
      provenance: "macos-build-bound-system-helper",
      role: "sw-vers",
      stageRelativePath: "b/sw_vers",
    }),
    component({
      kind: "executable",
      location: "private-toolchain",
      maximumSize: "33554432",
      modePolicy: "private-executable-0500",
      provenance: "macos-build-bound-system-helper",
      role: "system-profiler",
      stageRelativePath: "b/system_profiler",
    }),
  ],
  environment: {
    forbidden_names: FORBIDDEN_ENVIRONMENT_NAMES,
    home_policy: "private-hmac-path-and-physical-directory-identity-v1",
    names: ENVIRONMENT_NAMES,
    path_policy: "receipt-private-toolchain-only-v1",
    serialized_home_value: false,
  },
  host: {
    architecture: "arm64",
    minimum_product_version: "13.0.0",
    os_build_policy: "exact-private-observation-v1",
    platform: "darwin",
    system_library_policy: "exact-os-build-trusted-boundary-v1",
    virtualization: "apple-virtualization-framework-vz",
  },
  legacy_preparation_contract_sha256:
    "fb364b1cd89e7534b10dbd69d1092c93e64d17746b11692dec3b4252f83cbf51",
  provider_class: "colima-vz-docker-live",
  provider_kind: "colima",
  release_artifacts: {
    colima: {
      architecture: "arm64",
      name: "colima-Darwin-arm64",
      sha256: "980ad8bf61a4ca370243f4cb41401a61276dcd2c2502bee7b9b86f9250169f34",
      size: "15656320",
      source_revision: "00f6c297e92a82c04a4ab507db0a61435650d7e8",
      tag: "v0.10.3",
      url: "https://github.com/abiosoft/colima/releases/download/v0.10.3/colima-Darwin-arm64",
      version: "0.10.3",
    },
    disk_image: {
      architecture: "aarch64",
      format: "raw.gz",
      name: "ubuntu-24.04-minimal-cloudimg-arm64-docker.raw.gz",
      sha256: "1fc0354f4f99734ce3886628cc7af8b0437c1a1d391b126bd09cba0df35ee53f",
      sha512:
        "32242674b046b5057e60c4aba334b51e3665f05412cda89ed081cc2de153ae5c41f6b105b5c442cbe48d78e2cc21e9ba1950e406b6fb4fc2fd1dd2259240abbd",
      size: "332354401",
      tag: "v0.10.4",
      url: "https://github.com/abiosoft/colima-core/releases/download/v0.10.4/ubuntu-24.04-minimal-cloudimg-arm64-docker.raw.gz",
    },
    lima: {
      architecture: "arm64",
      name: "lima-2.2.0-Darwin-arm64.tar.gz",
      sha256: "bbdef91774885a0d05f7b048c4eb89ae2bcf3a0c252ae7ca7934e63df76d93c3",
      size: "37586365",
      source_revision: "de0816ea4bdc5267b428ab21025889b8dd785526",
      tag: "v2.2.0",
      url: "https://github.com/lima-vm/lima/releases/download/v2.2.0/lima-2.2.0-Darwin-arm64.tar.gz",
      version: "2.2.0",
    },
  },
  root_layout: ROOT_LAYOUT,
  schema: COLIMA_LIVE_REQUIREMENTS_SCHEMA,
  source_disk_image: {
    copy_mode: "0400",
    copy_relative_path: "a/colima-disk-image.raw.gz",
    copy_semantics: "distinct-no-replace-state-owned-publication-required-v1",
    local_file_only: true,
    maximum_size: "536870912",
  },
});

export const COLIMA_LIVE_REQUIREMENTS_SHA256 = colimaLiveDigest(
  colimaLiveBytes(COLIMA_LIVE_REQUIREMENTS),
);

export class ColimaLiveContractFailure extends Error {
  constructor(message, exitStatus = 78) {
    super(message);
    this.exitStatus = exitStatus;
  }
}

function fail(message, exitStatus = 78) {
  throw new ColimaLiveContractFailure(message, exitStatus);
}

function exactKeys(value, keys, label) {
  if (value === null || Array.isArray(value) || typeof value !== "object") {
    fail(`${label} was malformed`);
  }
  if (canonical(Object.keys(value).sort()) !== canonical([...keys].sort())) {
    fail(`${label} fields were refused`);
  }
}

function exactArray(value, expected, label) {
  if (!Array.isArray(value) || canonical(value) !== canonical(expected)) {
    fail(`${label} was refused`);
  }
}

function lowerHex(value, length) {
  return typeof value === "string" && value.length === length && /^[0-9a-f]+$/.test(value);
}

function decimal(value) {
  return typeof value === "string" && /^(?:0|[1-9][0-9]*)$/.test(value);
}

function integerFromDecimal(value, label) {
  if (!decimal(value)) fail(`${label} was refused`);
  const parsed = Number.parseInt(value, 10);
  if (!Number.isSafeInteger(parsed)) fail(`${label} was refused`);
  return parsed;
}

function safeRelativePath(value) {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    !isAbsolute(value) &&
    !value.split(/[\\/]/u).some((part) => part === "" || part === "." || part === "..")
  );
}

function exactStagedPath(entry) {
  const name = entry.stage_relative_path.split("/").at(-1);
  if (name === undefined || name.length === 0) return false;
  switch (entry.location) {
    case "private-artifact":
      return entry.stage_relative_path === `${ROOT_LAYOUT.artifact_directory}/${name}`;
    case "private-lima-config":
      return (
        entry.stage_relative_path === `${ROOT_LAYOUT.lima_home}/_config/networks.yaml`
      );
    case "private-toolchain":
      return entry.stage_relative_path === `${ROOT_LAYOUT.toolchain_directory}/${name}`;
    case "state-owner-runtime":
      return entry.stage_relative_path === "external";
    default:
      return false;
  }
}

function validateRequirementShape(value) {
  exactKeys(
    value,
    [
      "authorizations",
      "command_template",
      "components",
      "environment",
      "host",
      "legacy_preparation_contract_sha256",
      "provider_class",
      "provider_kind",
      "release_artifacts",
      "root_layout",
      "schema",
      "source_disk_image",
    ],
    "Colima live requirements",
  );
  exactKeys(
    value.authorizations,
    ["execution_authorized", "finalization_eligible", "lifecycle_exposure_authorized"],
    "Colima live requirements authorizations",
  );
  if (
    value.schema !== COLIMA_LIVE_REQUIREMENTS_SCHEMA ||
    value.provider_class !== "colima-vz-docker-live" ||
    value.provider_kind !== "colima" ||
    value.authorizations.execution_authorized !== false ||
    value.authorizations.lifecycle_exposure_authorized !== false ||
    value.authorizations.finalization_eligible !== false ||
    !lowerHex(value.legacy_preparation_contract_sha256, 64)
  ) {
    fail("Colima live requirements identity was refused");
  }
  exactArray(value.command_template, COLIMA_LIVE_REQUIREMENTS.command_template,
    "Colima live command template");
  exactKeys(
    value.environment,
    ["forbidden_names", "home_policy", "names", "path_policy", "serialized_home_value"],
    "Colima live environment policy",
  );
  exactArray(value.environment.names, ENVIRONMENT_NAMES, "Colima live environment names");
  exactArray(
    value.environment.forbidden_names,
    FORBIDDEN_ENVIRONMENT_NAMES,
    "Colima live forbidden environment names",
  );
  if (
    value.environment.home_policy !==
      "private-hmac-path-and-physical-directory-identity-v1" ||
    value.environment.path_policy !== "receipt-private-toolchain-only-v1" ||
    value.environment.serialized_home_value !== false
  ) {
    fail("Colima live environment policy was refused");
  }
  exactKeys(
    value.host,
    [
      "architecture",
      "minimum_product_version",
      "os_build_policy",
      "platform",
      "system_library_policy",
      "virtualization",
    ],
    "Colima live host policy",
  );
  if (
    value.host.architecture !== "arm64" ||
    value.host.minimum_product_version !== "13.0.0" ||
    value.host.os_build_policy !== "exact-private-observation-v1" ||
    value.host.platform !== "darwin" ||
    value.host.system_library_policy !== "exact-os-build-trusted-boundary-v1" ||
    value.host.virtualization !== "apple-virtualization-framework-vz"
  ) {
    fail("Colima live host policy was refused");
  }
  if (canonical(value.root_layout) !== canonical(ROOT_LAYOUT)) {
    fail("Colima live root layout was refused");
  }
  if (!Array.isArray(value.components) || value.components.length > MAX_COMPONENTS) {
    fail("Colima live component closure was refused");
  }
  exactArray(
    value.components.map((entry) => entry?.role),
    COMPONENT_ROLES,
    "Colima live component role order",
  );
  for (const entry of value.components) {
    exactKeys(
      entry,
      [
        "expected_sha256",
        "expected_size",
        "kind",
        "location",
        "maximum_size",
        "mode_policy",
        "provenance",
        "role",
        "stage_relative_path",
      ],
      "Colima live component requirement",
    );
    const expectedSize = integerFromDecimal(entry.expected_size, "component expected size");
    const maximumSize = integerFromDecimal(entry.maximum_size, "component maximum size");
    if (
      !lowerHex(entry.expected_sha256, 64) ||
      maximumSize < 1 ||
      expectedSize > maximumSize ||
      (entry.expected_sha256 === ZERO_SHA256) !== (expectedSize === 0) ||
      !new Set(["configuration", "data", "executable", "source"]).has(entry.kind) ||
      !new Set(["private-artifact", "private-lima-config", "private-toolchain", "state-owner-runtime"]).has(entry.location) ||
      !new Set([
        "external-executable-not-writable",
        "external-source-not-writable",
        "private-data-0400",
        "private-executable-0500",
      ]).has(entry.mode_policy) ||
      typeof entry.provenance !== "string" ||
      !/^[a-z0-9][a-z0-9.-]{2,127}$/u.test(entry.provenance) ||
      (entry.stage_relative_path !== "external" && !safeRelativePath(entry.stage_relative_path)) ||
      !exactStagedPath(entry) ||
      (entry.location === "state-owner-runtime") !==
        (entry.stage_relative_path === "external")
    ) {
      fail("Colima live component requirement was refused");
    }
  }
  exactKeys(
    value.release_artifacts,
    ["colima", "disk_image", "lima"],
    "Colima live release artifacts",
  );
  exactKeys(
    value.release_artifacts.colima,
    ["architecture", "name", "sha256", "size", "source_revision", "tag", "url", "version"],
    "Colima release artifact",
  );
  exactKeys(
    value.release_artifacts.lima,
    ["architecture", "name", "sha256", "size", "source_revision", "tag", "url", "version"],
    "Lima release artifact",
  );
  exactKeys(
    value.release_artifacts.disk_image,
    ["architecture", "format", "name", "sha256", "sha512", "size", "tag", "url"],
    "Colima disk release artifact",
  );
  for (const release of [value.release_artifacts.colima, value.release_artifacts.lima]) {
    if (
      release.architecture !== "arm64" ||
      !lowerHex(release.sha256, 64) ||
      integerFromDecimal(release.size, "release artifact size") < 1 ||
      !lowerHex(release.source_revision, 40) ||
      typeof release.name !== "string" ||
      typeof release.tag !== "string" ||
      typeof release.version !== "string" ||
      typeof release.url !== "string" ||
      !release.url.startsWith("https://github.com/")
    ) {
      fail("Colima live release artifact was refused");
    }
  }
  const disk = value.release_artifacts.disk_image;
  if (
    disk.architecture !== "aarch64" ||
    disk.format !== "raw.gz" ||
    !lowerHex(disk.sha256, 64) ||
    !lowerHex(disk.sha512, 128) ||
    integerFromDecimal(disk.size, "disk release size") < 1 ||
    typeof disk.name !== "string" ||
    typeof disk.tag !== "string" ||
    typeof disk.url !== "string" ||
    !disk.url.startsWith("https://github.com/abiosoft/colima-core/releases/")
  ) {
    fail("Colima live disk release artifact was refused");
  }
  exactKeys(
    value.source_disk_image,
    ["copy_mode", "copy_relative_path", "copy_semantics", "local_file_only", "maximum_size"],
    "Colima live source disk policy",
  );
  if (
    value.source_disk_image.copy_mode !== "0400" ||
    value.source_disk_image.copy_relative_path !==
      `${ROOT_LAYOUT.artifact_directory}/colima-disk-image.raw.gz` ||
    value.source_disk_image.copy_semantics !==
      "distinct-no-replace-state-owned-publication-required-v1" ||
    value.source_disk_image.local_file_only !== true ||
    integerFromDecimal(value.source_disk_image.maximum_size, "disk maximum size") <
      integerFromDecimal(disk.size, "disk release size")
  ) {
    fail("Colima live source disk policy was refused");
  }
  const componentByRole = new Map(value.components.map((entry) => [entry.role, entry]));
  const stagedPaths = value.components
    .filter((entry) => entry.stage_relative_path !== "external")
    .map((entry) => entry.stage_relative_path);
  const colima = componentByRole.get("colima-binary");
  const network = componentByRole.get("lima-network-config");
  if (
    new Set(stagedPaths).size !== stagedPaths.length ||
    colima.expected_sha256 !== value.release_artifacts.colima.sha256 ||
    colima.expected_size !== value.release_artifacts.colima.size ||
    network.expected_sha256 !== NETWORK_CONFIG_SHA256 ||
    network.expected_size !== String(LIMA_NETWORK_CONFIG_BYTES.length)
  ) {
    fail("Colima live release/component binding was refused");
  }
  return value;
}

export function validateColimaLiveRequirements(value) {
  validateRequirementShape(value);
  if (!colimaLiveBytes(value).equals(colimaLiveBytes(COLIMA_LIVE_REQUIREMENTS))) {
    fail("pinned Colima live requirements were refused");
  }
  return value;
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

function assertNoSymlinkComponents(path, label) {
  if (!isAbsolute(path) || resolve(path) !== path) fail(`${label} path was refused`);
  const parts = path.split(sep);
  let current = sep;
  for (const part of parts) {
    if (part === "") continue;
    current = join(current, part);
    let metadata;
    try {
      metadata = lstatSync(current, { bigint: true });
    } catch {
      fail(`${label} component was unavailable`, 69);
    }
    if (metadata.isSymbolicLink()) fail(`${label} symlink component was refused`);
  }
}

function mode(metadata) {
  return (metadata.mode & 0o7777n).toString(8).padStart(4, "0");
}

function directoryIdentity(path, label, { privateDirectory = false, revealPath = true } = {}) {
  assertNoSymlinkComponents(path, label);
  let first;
  let second;
  try {
    first = lstatSync(path, { bigint: true });
    second = lstatSync(path, { bigint: true });
  } catch {
    fail(`${label} was unavailable`, 69);
  }
  const currentUid = typeof process.getuid === "function" ? BigInt(process.getuid()) : -1n;
  const permissions = first.mode & 0o7777n;
  if (
    !first.isDirectory() ||
    !sameMetadata(first, second) ||
    (permissions & 0o022n) !== 0n ||
    (privateDirectory && (currentUid < 0n || first.uid !== currentUid || permissions !== 0o700n))
  ) {
    fail(`${label} identity was refused`);
  }
  const identity = {
    device: String(first.dev),
    inode: String(first.ino),
    mode: mode(first),
    uid: String(first.uid),
  };
  if (revealPath) identity.path = path;
  return Object.freeze(identity);
}

function validateMode(metadata, policy, label) {
  const permissions = metadata.mode & 0o7777n;
  const currentUid = typeof process.getuid === "function" ? BigInt(process.getuid()) : -1n;
  switch (policy) {
    case "private-executable-0500":
      if (currentUid < 0n || metadata.uid !== currentUid || permissions !== 0o500n) {
        fail(`${label} mode was refused`);
      }
      return;
    case "private-data-0400":
      if (currentUid < 0n || metadata.uid !== currentUid || permissions !== 0o400n) {
        fail(`${label} mode was refused`);
      }
      return;
    case "external-executable-not-writable":
      if ((permissions & 0o111n) === 0n || (permissions & 0o022n) !== 0n) {
        fail(`${label} mode was refused`);
      }
      return;
    case "external-source-not-writable":
      if ((permissions & 0o400n) === 0n || (permissions & 0o022n) !== 0n) {
        fail(`${label} mode was refused`);
      }
      return;
    default:
      fail(`${label} mode policy was refused`, 70);
  }
}

function openedFileIdentity(path, requirement, label, algorithms = ["sha256"]) {
  assertNoSymlinkComponents(path, label);
  let descriptor;
  try {
    const namedBefore = lstatSync(path, { bigint: true });
    descriptor = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW);
    const before = fstatSync(descriptor, { bigint: true });
    if (
      !before.isFile() ||
      before.isSymbolicLink() ||
      !sameMetadata(before, namedBefore) ||
      before.nlink !== 1n ||
      before.size < 1n ||
      before.size > BigInt(integerFromDecimal(requirement.maximum_size, `${label} maximum size`))
    ) {
      fail(`${label} identity was refused`);
    }
    validateMode(before, requirement.mode_policy, label);
    const hashes = new Map(algorithms.map((algorithm) => [algorithm, createHash(algorithm)]));
    const buffer = Buffer.allocUnsafe(HASH_CHUNK_BYTES);
    let offset = 0;
    for (;;) {
      const count = readSync(descriptor, buffer, 0, buffer.length, offset);
      if (count === 0) break;
      const chunk = buffer.subarray(0, count);
      for (const hash of hashes.values()) hash.update(chunk);
      offset += count;
    }
    const after = fstatSync(descriptor, { bigint: true });
    const namedAfter = lstatSync(path, { bigint: true });
    if (!sameMetadata(before, after) || !sameMetadata(before, namedAfter) || BigInt(offset) !== before.size) {
      fail(`${label} changed while it was read`);
    }
    const digests = Object.fromEntries(
      [...hashes.entries()].map(([algorithm, hash]) => [algorithm, hash.digest("hex")]),
    );
    if (
      requirement.expected_sha256 !== ZERO_SHA256 &&
      (digests.sha256 !== requirement.expected_sha256 ||
        String(before.size) !== requirement.expected_size)
    ) {
      fail(`${label} release identity was refused`);
    }
    return Object.freeze({
      device: String(before.dev),
      inode: String(before.ino),
      links: String(before.nlink),
      mode: mode(before),
      path,
      role: requirement.role,
      sha256: digests.sha256,
      ...(digests.sha512 === undefined ? {} : { sha512: digests.sha512 }),
      size: String(before.size),
      uid: String(before.uid),
    });
  } catch (error) {
    if (error instanceof ColimaLiveContractFailure) throw error;
    fail(`${label} was unavailable`, 69);
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

function identityDigest(value) {
  return colimaLiveDigest(colimaLiveBytes(value));
}

function observedDirectoryIdentity(value) {
  return {
    device: value.device,
    inode: value.inode,
    mode: value.mode,
    path: value.path,
    uid: value.uid,
  };
}

function validateObservedMode(value, policy, label) {
  if (!/^[0-7]{4}$/u.test(value.mode) || !decimal(value.uid)) {
    fail(`${label} mode was refused`);
  }
  const permissions = Number.parseInt(value.mode, 8);
  const currentUid = typeof process.getuid === "function" ? String(process.getuid()) : null;
  switch (policy) {
    case "private-executable-0500":
      if (currentUid === null || value.uid !== currentUid || permissions !== 0o500) {
        fail(`${label} mode was refused`);
      }
      return;
    case "private-data-0400":
      if (currentUid === null || value.uid !== currentUid || permissions !== 0o400) {
        fail(`${label} mode was refused`);
      }
      return;
    case "external-executable-not-writable":
      if ((permissions & 0o111) === 0 || (permissions & 0o022) !== 0) {
        fail(`${label} mode was refused`);
      }
      return;
    case "external-source-not-writable":
      if ((permissions & 0o400) === 0 || (permissions & 0o022) !== 0) {
        fail(`${label} mode was refused`);
      }
      return;
    default:
      fail(`${label} mode policy was refused`, 70);
  }
}

function withParentIdentity(identity, label) {
  const parent = directoryIdentity(dirname(identity.path), `${label} parent`, {
    privateDirectory: false,
  });
  return Object.freeze({ ...identity, parent_identity_sha256: identityDigest(parent) });
}

function isWithin(parent, child) {
  const rel = relative(parent, child);
  return rel !== "" && rel !== ".." && !rel.startsWith(`..${sep}`) && !isAbsolute(rel);
}

function exactDirectoryEntries(path, expected, label) {
  let entries;
  try {
    entries = readdirSync(path).sort();
  } catch {
    fail(`${label} inventory was unavailable`, 69);
  }
  exactArray(entries, [...expected].sort(), `${label} inventory`);
}

function directoryPaths(providerRoot) {
  return Object.freeze({
    "artifact-directory": join(providerRoot, ROOT_LAYOUT.artifact_directory),
    "colima-cache-home": join(providerRoot, ROOT_LAYOUT.colima_cache_home),
    "colima-home": join(providerRoot, ROOT_LAYOUT.colima_home),
    "docker-config": join(providerRoot, ROOT_LAYOUT.docker_config),
    "lima-config-directory": join(providerRoot, ROOT_LAYOUT.lima_home, "_config"),
    "lima-home": join(providerRoot, ROOT_LAYOUT.lima_home),
    "provider-root": providerRoot,
    "temporary-directory": join(providerRoot, ROOT_LAYOUT.temporary_directory),
    "toolchain-directory": join(providerRoot, ROOT_LAYOUT.toolchain_directory),
  });
}

function admissionMetadataEqual(left, right) {
  return (
    sameMetadata(left, right) &&
    left.mtimeNs === right.mtimeNs &&
    left.ctimeNs === right.ctimeNs
  );
}

function admissionEntryKind(metadata) {
  if (metadata.isDirectory()) return "directory";
  if (metadata.isFile()) return "file";
  if (metadata.isSymbolicLink()) return "symlink";
  if (metadata.isSocket()) return "socket";
  if (metadata.isFIFO()) return "fifo";
  if (metadata.isCharacterDevice()) return "character-device";
  if (metadata.isBlockDevice()) return "block-device";
  return "other";
}

function admissionEntryIdentity(metadata) {
  return Object.freeze({
    ctime_nanoseconds: String(metadata.ctimeNs),
    device: String(metadata.dev),
    inode: String(metadata.ino),
    kind: admissionEntryKind(metadata),
    links: String(metadata.nlink),
    mode: mode(metadata),
    mtime_nanoseconds: String(metadata.mtimeNs),
    size: String(metadata.size),
    uid: String(metadata.uid),
  });
}

function optionalNoFollowMetadata(path, label) {
  try {
    return lstatSync(path, { bigint: true });
  } catch (error) {
    if (error?.code === "ENOENT") return undefined;
    fail(`${label} presence was unavailable`, 69);
  }
}

function recordedDirectoryMatches(metadata, recorded) {
  return (
    metadata.isDirectory() &&
    !metadata.isSymbolicLink() &&
    String(metadata.dev) === recorded.device &&
    String(metadata.ino) === recorded.inode &&
    mode(metadata) === recorded.mode &&
    String(metadata.uid) === recorded.uid
  );
}

function captureAdmissionTarget(parent, targetName, role, baseEntries) {
  if (
    typeof targetName !== "string" ||
    targetName.length < 1 ||
    targetName.includes(sep) ||
    targetName === "." ||
    targetName === ".."
  ) {
    fail("Colima live pre-effect target name was refused", 70);
  }
  assertNoSymlinkComponents(parent.path, `Colima live ${role} parent`);
  let descriptor;
  try {
    const namedBefore = lstatSync(parent.path, { bigint: true });
    descriptor = openSync(
      parent.path,
      constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW,
    );
    const openedBefore = fstatSync(descriptor, { bigint: true });
    if (
      !admissionMetadataEqual(namedBefore, openedBefore) ||
      !recordedDirectoryMatches(openedBefore, parent) ||
      openedBefore.uid !== BigInt(process.getuid()) ||
      (openedBefore.mode & 0o7777n) !== 0o700n
    ) {
      fail(`Colima live ${role} parent identity was refused`);
    }
    const targetPath = join(parent.path, targetName);
    const targetBefore = optionalNoFollowMetadata(
      targetPath,
      `Colima live ${role}`,
    );
    const inventory = readdirSync(parent.path).sort();
    exactArray(
      inventory,
      [...baseEntries, ...(targetBefore === undefined ? [] : [targetName])].sort(),
      `Colima live ${role} parent inventory`,
    );
    const targetAfter = optionalNoFollowMetadata(
      targetPath,
      `Colima live ${role}`,
    );
    const openedAfter = fstatSync(descriptor, { bigint: true });
    const namedAfter = lstatSync(parent.path, { bigint: true });
    if (
      !admissionMetadataEqual(namedBefore, openedAfter) ||
      !admissionMetadataEqual(namedBefore, namedAfter) ||
      (targetBefore === undefined) !== (targetAfter === undefined) ||
      (targetBefore !== undefined &&
        !admissionMetadataEqual(targetBefore, targetAfter))
    ) {
      fail(`Colima live ${role} observation changed`, 73);
    }
    return Object.freeze({
      disposition:
        targetBefore === undefined ? "observed-absent" : "foreign-collision",
      entryIdentity:
        targetBefore === undefined
          ? undefined
          : admissionEntryIdentity(targetBefore),
      parentIdentity: Object.freeze({
        path: parent.path,
        role: `${role}-parent`,
        ...admissionEntryIdentity(openedBefore),
      }),
      role,
      targetName,
      targetPath,
    });
  } catch (error) {
    if (error instanceof ColimaLiveContractFailure) throw error;
    fail(`Colima live ${role} observation was unavailable`, 69);
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

function captureAdmissionRoots(observation) {
  const directories = new Map(
    observation.directories.map((entry) => [entry.role, entry]),
  );
  const providerProfile = observation.provider_profile;
  const limaInstance = `colima-${providerProfile}`;
  return Object.freeze([
    captureAdmissionTarget(
      directories.get("colima-home"),
      providerProfile,
      "colima-profile-root",
      [],
    ),
    captureAdmissionTarget(
      directories.get("lima-home"),
      limaInstance,
      "lima-instance-root",
      ["_config"],
    ),
  ]);
}

function admissionInventory(value) {
  return Object.freeze({
    colimaProfilePresent:
      value[0].disposition === "foreign-collision",
    limaInstancePresent:
      value[1].disposition === "foreign-collision",
  });
}

function admissionHmac(bindingKey, fixtureId, role, purpose, schema, value) {
  return createHmac("sha256", bindingKey)
    .update(schema, "utf8")
    .update("\0", "ascii")
    .update(fixtureId, "ascii")
    .update("\0", "ascii")
    .update(role, "ascii")
    .update("\0", "ascii")
    .update(purpose, "ascii")
    .update("\0", "ascii")
    .update(colimaLiveBytes(value))
    .digest("hex");
}

function admissionRootProjection(root, input, schema) {
  const parentIdentityHmac = admissionHmac(
    input.binding_key,
    input.fixture_id,
    root.role,
    "parent-identity",
    schema,
    root.parentIdentity,
  );
  const targetPathHmac = admissionHmac(
    input.binding_key,
    input.fixture_id,
    root.role,
    "target-path",
    schema,
    root.targetPath,
  );
  const targetEntryIdentityHmac =
    root.entryIdentity === undefined
      ? ZERO_SHA256
      : admissionHmac(
          input.binding_key,
          input.fixture_id,
          root.role,
          "target-entry-identity",
          schema,
          { path_hmac_sha256: targetPathHmac, ...root.entryIdentity },
        );
  return Object.freeze({
    disposition: root.disposition,
    parent_identity_hmac_sha256: parentIdentityHmac,
    role: root.role,
    target_entry_identity_hmac_sha256: targetEntryIdentityHmac,
    target_path_hmac_sha256: targetPathHmac,
  });
}

function observePreEffectRoots(
  requirements,
  value,
  input,
  { evidenceClass, schema, testCheckpoint },
) {
  validateObservationShape(value, requirements);
  validateBuildInput(input, requirements);
  validateObservationInputBinding(value, input);
  const first = captureAdmissionRoots(value);
  if (testCheckpoint !== undefined) {
    testCheckpoint("after-first-root-sample");
  }
  const rebuilt = buildObservation(requirements, input, admissionInventory(first));
  const expected = colimaLiveBytes(value);
  const actual = colimaLiveBytes(rebuilt);
  if (expected.length !== actual.length || !timingSafeEqual(expected, actual)) {
    fail("Colima live observation changed during pre-effect revalidation", 73);
  }
  const second = captureAdmissionRoots(value);
  const firstProjection = first.map((root) =>
    admissionRootProjection(root, input, schema),
  );
  const secondProjection = second.map((root) =>
    admissionRootProjection(root, input, schema),
  );
  const firstBytes = colimaLiveBytes(firstProjection);
  const secondBytes = colimaLiveBytes(secondProjection);
  if (
    firstBytes.length !== secondBytes.length ||
    !timingSafeEqual(firstBytes, secondBytes)
  ) {
    fail("Colima live pre-effect root observation changed", 73);
  }
  const rootSetDisposition = firstProjection.some(
    (root) => root.disposition === "foreign-collision",
  )
    ? "foreign-collision"
    : "observed-absent";
  return deepFreeze({
    evidence_class: evidenceClass,
    planned_names: {
      lima_instance: `colima-${value.provider_profile}`,
      provider_profile: value.provider_profile,
    },
    preparation_observation_sha256: colimaLiveDigest(expected),
    requirements_sha256: colimaLiveDigest(colimaLiveBytes(requirements)),
    root_observations: firstProjection,
    root_set_disposition: rootSetDisposition,
    schema,
  });
}

function validateHost(host, requirements) {
  exactKeys(
    host,
    ["architecture", "boot_session_sha256", "build_version", "kernel_release", "platform", "product_version"],
    "Colima live host observation",
  );
  const versionPattern = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/u;
  const observedMatch = versionPattern.exec(host.product_version);
  const minimumMatch = versionPattern.exec(requirements.host.minimum_product_version);
  if (
    host.architecture !== requirements.host.architecture ||
    host.platform !== requirements.host.platform ||
    observedMatch === null ||
    minimumMatch === null ||
    !/^[0-9A-Z][0-9A-Z.-]{2,31}$/u.test(host.build_version) ||
    !versionPattern.test(host.kernel_release) ||
    !lowerHex(host.boot_session_sha256, 64)
  ) {
    fail("Colima live host observation was refused", 69);
  }
  const observed = observedMatch.slice(1).map(Number);
  const minimum = minimumMatch.slice(1).map(Number);
  for (let index = 0; index < observed.length; index += 1) {
    if (observed[index] > minimum[index]) break;
    if (observed[index] < minimum[index]) {
      fail("Colima live host version was refused", 69);
    }
  }
  return Object.freeze({ ...host });
}

function expectedEnvironment(providerRoot, home) {
  const paths = directoryPaths(providerRoot);
  return Object.freeze({
    COLIMA_CACHE_HOME: paths["colima-cache-home"],
    COLIMA_DOWNLOADER: "native",
    COLIMA_HOME: paths["colima-home"],
    DOCKER_CONFIG: paths["docker-config"],
    HOME: home,
    LANG: "C",
    LC_ALL: "C",
    LIMA_HOME: paths["lima-home"],
    PATH: paths["toolchain-directory"],
    SSH: join(paths["toolchain-directory"], "ssh"),
    TMPDIR: paths["temporary-directory"],
    XPC_SERVICE_NAME: "0",
  });
}

function validateEnvironment(environment, providerRoot, home) {
  exactKeys(environment, ENVIRONMENT_NAMES, "Colima live execution environment");
  const expected = expectedEnvironment(providerRoot, home);
  if (canonical(environment) !== canonical(expected)) {
    fail("Colima live execution environment was refused");
  }
  return expected;
}

function expandCommand(requirements, componentPaths, providerProfile, diskPath) {
  const replacements = new Map([
    ["<colima-binary>", componentPaths["colima-binary"]],
    ["<lima-instance>", `colima-${providerProfile}`],
    ["<provider-profile>", providerProfile],
    ["<receipt-owned-disk-image>", diskPath],
  ]);
  return Object.freeze(
    requirements.command_template.map((argument) => replacements.get(argument) ?? argument),
  );
}

function validateBuildInput(input, requirements) {
  exactKeys(
    input,
    [
      "binding_key",
      "component_paths",
      "environment",
      "fixture_id",
      "host",
      "provider_profile",
      "provider_root",
      "receipt_owned_disk_image_path",
      "source_disk_image_path",
    ],
    "Colima live observation input",
  );
  if (!Buffer.isBuffer(input.binding_key) || input.binding_key.length < 32) {
    fail("Colima live observation binding key was refused", 64);
  }
  if (!lowerHex(input.fixture_id, 32)) fail("Colima live fixture identity was refused", 64);
  if (input.provider_profile !== `synveda-cpr45-${input.fixture_id}`) {
    fail("Colima live provider profile was refused", 64);
  }
  exactKeys(input.component_paths, COMPONENT_ROLES, "Colima live component paths");
  exactKeys(input.environment, ENVIRONMENT_NAMES, "Colima live input environment");
  validateHost(input.host, requirements);
  for (const path of Object.values(input.component_paths)) {
    if (typeof path !== "string" || !isAbsolute(path) || resolve(path) !== path) {
      fail("Colima live component path was refused", 64);
    }
  }
  for (const path of [
    input.provider_root,
    input.receipt_owned_disk_image_path,
    input.source_disk_image_path,
  ]) {
    if (typeof path !== "string" || !isAbsolute(path) || resolve(path) !== path) {
      fail("Colima live provider path was refused", 64);
    }
  }
  if (
    input.receipt_owned_disk_image_path !==
      join(input.provider_root, requirements.source_disk_image.copy_relative_path) ||
    isWithin(input.provider_root, input.source_disk_image_path) ||
    input.source_disk_image_path === input.receipt_owned_disk_image_path
  ) {
    fail("Colima live disk path separation was refused");
  }
}

function validateObservationInputBinding(value, input) {
  const directories = new Map(
    value.directories.map((entry) => [entry.role, entry.path]),
  );
  const components = Object.fromEntries(
    value.components.map((entry) => [entry.role, entry.path]),
  );
  const observedVariables = Object.fromEntries(
    value.environment.variables.map((entry) => [entry.name, entry.value]),
  );
  const inputVariables = Object.fromEntries(
    Object.entries(input.environment).filter(([name]) => name !== "HOME"),
  );
  const homePathHmac = createHmac("sha256", input.binding_key)
    .update(
      `home\0${input.fixture_id}\0${input.environment.HOME}`,
      "utf8",
    )
    .digest("hex");
  if (
    input.fixture_id !== value.fixture_id ||
    input.provider_profile !== value.provider_profile ||
    input.provider_root !== directories.get("provider-root") ||
    input.receipt_owned_disk_image_path !==
      value.receipt_owned_disk_image.path ||
    input.source_disk_image_path !== value.source_disk_image.path ||
    canonical(input.component_paths) !== canonical(components) ||
    canonical(inputVariables) !== canonical(observedVariables) ||
    canonical(input.host) !== canonical(value.host) ||
    homePathHmac !== value.environment.home.path_hmac_sha256
  ) {
    fail("Colima live observation input binding was refused", 73);
  }
}

function buildObservation(requirements, input, preEffectInventory = undefined) {
  validateRequirementShape(requirements);
  validateBuildInput(input, requirements);
  let providerRoot;
  try {
    providerRoot = realpathSync(input.provider_root);
  } catch {
    fail("Colima live provider root was unavailable", 69);
  }
  if (providerRoot !== input.provider_root) fail("Colima live provider root was not canonical");
  const directoriesByRole = directoryPaths(providerRoot);
  const directories = DIRECTORY_ROLES.map((role) => Object.freeze({
    ...directoryIdentity(directoriesByRole[role], `Colima live ${role}`, {
      privateDirectory: true,
    }),
    role,
  }));
  for (const requirement of requirements.components) {
    if (requirement.stage_relative_path === "external") continue;
    const expected = join(providerRoot, requirement.stage_relative_path);
    if (input.component_paths[requirement.role] !== expected) {
      fail("Colima live staged component path was refused");
    }
  }
  const stagedToolchainNames = requirements.components
    .filter((entry) => entry.location === "private-toolchain")
    .map((entry) => entry.stage_relative_path.split("/").at(-1));
  const stagedArtifactNames = requirements.components
    .filter((entry) => entry.location === "private-artifact")
    .map((entry) => entry.stage_relative_path.split("/").at(-1));
  stagedArtifactNames.push(requirements.source_disk_image.copy_relative_path.split("/").at(-1));
  exactDirectoryEntries(providerRoot, Object.values(ROOT_LAYOUT), "Colima live provider root");
  exactDirectoryEntries(directoriesByRole["toolchain-directory"], stagedToolchainNames,
    "Colima live toolchain directory");
  exactDirectoryEntries(directoriesByRole["artifact-directory"], stagedArtifactNames,
    "Colima live artifact directory");
  const colimaProfilePresent = preEffectInventory?.colimaProfilePresent ?? false;
  const limaInstancePresent = preEffectInventory?.limaInstancePresent ?? false;
  if (
    typeof colimaProfilePresent !== "boolean" ||
    typeof limaInstancePresent !== "boolean" ||
    (preEffectInventory !== undefined &&
      canonical(Object.keys(preEffectInventory).sort()) !==
        canonical(["colimaProfilePresent", "limaInstancePresent"]))
  ) {
    fail("Colima live pre-effect inventory was refused", 70);
  }
  exactDirectoryEntries(
    directoriesByRole["lima-home"],
    ["_config", ...(limaInstancePresent ? [`colima-${input.provider_profile}`] : [])],
    "Colima live Lima home",
  );
  exactDirectoryEntries(directoriesByRole["lima-config-directory"], ["networks.yaml"],
    "Colima live Lima config directory");
  for (const role of ["colima-cache-home", "docker-config", "temporary-directory"]) {
    exactDirectoryEntries(directoriesByRole[role], [], `Colima live ${role}`);
  }
  exactDirectoryEntries(
    directoriesByRole["colima-home"],
    colimaProfilePresent ? [input.provider_profile] : [],
    "Colima live colima-home",
  );
  const components = requirements.components.map((requirement) =>
    withParentIdentity(
      openedFileIdentity(
        input.component_paths[requirement.role],
        requirement,
        `Colima live ${requirement.role}`,
      ),
      `Colima live ${requirement.role}`,
    ),
  );
  const diskRequirement = {
    expected_sha256: requirements.release_artifacts.disk_image.sha256,
    expected_size: requirements.release_artifacts.disk_image.size,
    maximum_size: requirements.source_disk_image.maximum_size,
    mode_policy: "private-data-0400",
    role: "source-disk-image",
  };
  const sourceDisk = withParentIdentity(
    openedFileIdentity(
      input.source_disk_image_path,
      diskRequirement,
      "Colima live source disk image",
      ["sha256", "sha512"],
    ),
    "Colima live source disk image",
  );
  const copiedDisk = withParentIdentity(
    openedFileIdentity(
      input.receipt_owned_disk_image_path,
      { ...diskRequirement, role: "receipt-owned-disk-image" },
      "Colima live receipt-owned disk image",
      ["sha256", "sha512"],
    ),
    "Colima live receipt-owned disk image",
  );
  if (
    sourceDisk.sha512 !== requirements.release_artifacts.disk_image.sha512 ||
    copiedDisk.sha512 !== requirements.release_artifacts.disk_image.sha512 ||
    (sourceDisk.device === copiedDisk.device && sourceDisk.inode === copiedDisk.inode)
  ) {
    fail("Colima live disk image identity was refused");
  }
  const home = input.environment.HOME;
  if (typeof home !== "string" || !isAbsolute(home) || resolve(home) !== home) {
    fail("Colima live HOME was refused", 69);
  }
  if (home === providerRoot || isWithin(home, providerRoot) || isWithin(providerRoot, home)) {
    fail("Colima live HOME/provider overlap was refused");
  }
  const homeIdentity = directoryIdentity(home, "Colima live HOME", {
    privateDirectory: false,
    revealPath: false,
  });
  const environment = validateEnvironment(input.environment, providerRoot, home);
  const serializedVariables = Object.entries(environment)
    .filter(([name]) => name !== "HOME")
    .map(([name, value]) => Object.freeze({ name, value }));
  const host = validateHost(input.host, requirements);
  const command = expandCommand(
    requirements,
    input.component_paths,
    input.provider_profile,
    input.receipt_owned_disk_image_path,
  );
  const observation = {
    authorizations: { ...requirements.authorizations },
    command,
    components,
    directories,
    environment: {
      home: {
        directory_identity: homeIdentity,
        path_hmac_sha256: createHmac("sha256", input.binding_key)
          .update(`home\0${input.fixture_id}\0${home}`, "utf8")
          .digest("hex"),
      },
      names: [...ENVIRONMENT_NAMES],
      variables: serializedVariables,
    },
    fixture_id: input.fixture_id,
    host,
    host_probe_authority: "preparation-input-only-not-live-admission-v1",
    provider_class: requirements.provider_class,
    provider_kind: requirements.provider_kind,
    provider_profile: input.provider_profile,
    provider_root_identity_sha256: identityDigest(
      directories.find((entry) => entry.role === "provider-root"),
    ),
    receipt_owned_disk_image: copiedDisk,
    requirements_sha256: colimaLiveDigest(colimaLiveBytes(requirements)),
    schema: COLIMA_LIVE_OBSERVATION_SCHEMA,
    source_disk_image: sourceDisk,
  };
  validateObservationShape(observation, requirements);
  for (const directory of directories) {
    const current = directoryIdentity(directory.path, `Colima live ${directory.role}`, {
      privateDirectory: true,
    });
    if (identityDigest(current) !== identityDigest({
      device: directory.device,
      inode: directory.inode,
      mode: directory.mode,
      path: directory.path,
      uid: directory.uid,
    })) {
      fail("Colima live directory identity changed");
    }
  }
  return deepFreeze(observation);
}

function validateFileObservation(value, label, { sha512 = false } = {}) {
  exactKeys(
    value,
    [
      "device",
      "inode",
      "links",
      "mode",
      "parent_identity_sha256",
      "path",
      "role",
      "sha256",
      ...(sha512 ? ["sha512"] : []),
      "size",
      "uid",
    ],
    label,
  );
  if (
    !isAbsolute(value.path) ||
    resolve(value.path) !== value.path ||
    !decimal(value.device) ||
    !decimal(value.inode) ||
    value.links !== "1" ||
    !/^[0-7]{4}$/u.test(value.mode) ||
    !lowerHex(value.parent_identity_sha256, 64) ||
    !lowerHex(value.sha256, 64) ||
    (sha512 && !lowerHex(value.sha512, 128)) ||
    integerFromDecimal(value.size, `${label} size`) < 1 ||
    !decimal(value.uid)
  ) {
    fail(`${label} was refused`);
  }
}

function validateObservationShape(value, requirements) {
  validateRequirementShape(requirements);
  exactKeys(
    value,
    [
      "authorizations",
      "command",
      "components",
      "directories",
      "environment",
      "fixture_id",
      "host",
      "host_probe_authority",
      "provider_class",
      "provider_kind",
      "provider_profile",
      "provider_root_identity_sha256",
      "receipt_owned_disk_image",
      "requirements_sha256",
      "schema",
      "source_disk_image",
    ],
    "Colima live observation",
  );
  if (
    value.schema !== COLIMA_LIVE_OBSERVATION_SCHEMA ||
    value.provider_class !== requirements.provider_class ||
    value.provider_kind !== requirements.provider_kind ||
    value.host_probe_authority !== "preparation-input-only-not-live-admission-v1" ||
    value.requirements_sha256 !== colimaLiveDigest(colimaLiveBytes(requirements)) ||
    !lowerHex(value.fixture_id, 32) ||
    value.provider_profile !== `synveda-cpr45-${value.fixture_id}` ||
    !lowerHex(value.provider_root_identity_sha256, 64) ||
    canonical(value.authorizations) !== canonical(requirements.authorizations)
  ) {
    fail("Colima live observation identity was refused");
  }
  exactArray(
    value.components.map((entry) => entry?.role),
    COMPONENT_ROLES,
    "Colima live observed component role order",
  );
  const requirementByRole = new Map(
    requirements.components.map((entry) => [entry.role, entry]),
  );
  for (const entry of value.components) {
    validateFileObservation(entry, "Colima live component");
    const requirement = requirementByRole.get(entry.role);
    validateObservedMode(entry, requirement.mode_policy, `Colima live ${entry.role}`);
    if (
      (requirement.expected_sha256 !== ZERO_SHA256 &&
        (entry.sha256 !== requirement.expected_sha256 ||
          entry.size !== requirement.expected_size)) ||
      integerFromDecimal(entry.size, `Colima live ${entry.role} size`) >
        integerFromDecimal(requirement.maximum_size, `Colima live ${entry.role} maximum size`)
    ) {
      fail(`Colima live ${entry.role} identity was refused`);
    }
  }
  exactArray(
    value.directories.map((entry) => entry?.role),
    DIRECTORY_ROLES,
    "Colima live observed directory role order",
  );
  for (const entry of value.directories) {
    exactKeys(entry, ["device", "inode", "mode", "path", "role", "uid"],
      "Colima live directory observation");
    if (
      !isAbsolute(entry.path) ||
      resolve(entry.path) !== entry.path ||
      !decimal(entry.device) ||
      !decimal(entry.inode) ||
      entry.mode !== "0700" ||
      !decimal(entry.uid)
    ) {
      fail("Colima live directory observation was refused");
    }
  }
  const directoriesByRole = new Map(value.directories.map((entry) => [entry.role, entry]));
  const providerRoot = directoriesByRole.get("provider-root").path;
  const expectedDirectories = directoryPaths(providerRoot);
  for (const [role, expectedPath] of Object.entries(expectedDirectories)) {
    if (directoriesByRole.get(role).path !== expectedPath) {
      fail("Colima live directory layout was refused");
    }
  }
  if (
    value.provider_root_identity_sha256 !==
    identityDigest(directoriesByRole.get("provider-root"))
  ) {
    fail("Colima live provider root identity was refused");
  }
  const componentIdentities = new Set();
  for (const entry of value.components) {
    const requirement = requirementByRole.get(entry.role);
    const identity = `${entry.device}:${entry.inode}`;
    if (componentIdentities.has(identity)) {
      fail("Colima live duplicate component identity was refused");
    }
    componentIdentities.add(identity);
    if (requirement.stage_relative_path === "external") {
      if (entry.path === providerRoot || isWithin(providerRoot, entry.path)) {
        fail(`Colima live ${entry.role} external path was refused`);
      }
      continue;
    }
    const expectedPath = join(providerRoot, requirement.stage_relative_path);
    if (entry.path !== expectedPath) {
      fail(`Colima live ${entry.role} staged path was refused`);
    }
    const parentRole =
      requirement.location === "private-toolchain"
        ? "toolchain-directory"
        : requirement.location === "private-artifact"
          ? "artifact-directory"
          : "lima-config-directory";
    if (
      entry.parent_identity_sha256 !==
      identityDigest(observedDirectoryIdentity(directoriesByRole.get(parentRole)))
    ) {
      fail(`Colima live ${entry.role} parent identity was refused`);
    }
  }
  exactKeys(value.environment, ["home", "names", "variables"],
    "Colima live observed environment");
  exactKeys(value.environment.home, ["directory_identity", "path_hmac_sha256"],
    "Colima live observed HOME");
  exactKeys(value.environment.home.directory_identity, ["device", "inode", "mode", "uid"],
    "Colima live observed HOME directory");
  if (
    !decimal(value.environment.home.directory_identity.device) ||
    !decimal(value.environment.home.directory_identity.inode) ||
    !/^[0-7]{4}$/u.test(value.environment.home.directory_identity.mode) ||
    !decimal(value.environment.home.directory_identity.uid) ||
    !lowerHex(value.environment.home.path_hmac_sha256, 64)
  ) {
    fail("Colima live observed HOME was refused");
  }
  exactArray(value.environment.names, ENVIRONMENT_NAMES, "Colima live observed environment names");
  const expectedVariableNames = ENVIRONMENT_NAMES.filter((name) => name !== "HOME");
  exactArray(
    value.environment.variables.map((entry) => entry?.name),
    expectedVariableNames,
    "Colima live observed variable order",
  );
  for (const variable of value.environment.variables) {
    exactKeys(variable, ["name", "value"], "Colima live observed variable");
    if (
      typeof variable.value !== "string" ||
      variable.value.length < 1 ||
      variable.value.includes("\0") ||
      variable.value.includes("\n")
    ) {
      fail("Colima live observed variable was refused");
    }
  }
  const observedVariables = Object.fromEntries(
    value.environment.variables.map((entry) => [entry.name, entry.value]),
  );
  const expectedVariables = Object.fromEntries(
    Object.entries(expectedEnvironment(providerRoot, "<private-home>")).filter(
      ([name]) => name !== "HOME",
    ),
  );
  if (canonical(observedVariables) !== canonical(expectedVariables)) {
    fail("Colima live observed environment binding was refused");
  }
  validateHost(value.host, requirements);
  validateFileObservation(value.source_disk_image, "Colima live source disk observation", {
    sha512: true,
  });
  validateFileObservation(
    value.receipt_owned_disk_image,
    "Colima live receipt-owned disk observation",
    { sha512: true },
  );
  for (const [disk, label] of [
    [value.source_disk_image, "source disk"],
    [value.receipt_owned_disk_image, "receipt-owned disk"],
  ]) {
    validateObservedMode(disk, "private-data-0400", `Colima live ${label}`);
    if (
      disk.sha256 !== requirements.release_artifacts.disk_image.sha256 ||
      disk.sha512 !== requirements.release_artifacts.disk_image.sha512 ||
      disk.size !== requirements.release_artifacts.disk_image.size ||
      integerFromDecimal(disk.size, `Colima live ${label} size`) >
        integerFromDecimal(
          requirements.source_disk_image.maximum_size,
          "Colima live disk maximum size",
        )
    ) {
      fail(`Colima live ${label} identity was refused`);
    }
  }
  if (
    value.source_disk_image.role !== "source-disk-image" ||
    value.receipt_owned_disk_image.role !== "receipt-owned-disk-image" ||
    value.receipt_owned_disk_image.path !==
      join(providerRoot, requirements.source_disk_image.copy_relative_path) ||
    value.source_disk_image.path === providerRoot ||
    isWithin(providerRoot, value.source_disk_image.path) ||
    (value.source_disk_image.device === value.receipt_owned_disk_image.device &&
      value.source_disk_image.inode === value.receipt_owned_disk_image.inode) ||
    value.receipt_owned_disk_image.parent_identity_sha256 !==
      identityDigest(
        observedDirectoryIdentity(directoriesByRole.get("artifact-directory")),
      )
  ) {
    fail("Colima live disk path separation was refused");
  }
  const componentPaths = Object.fromEntries(value.components.map((entry) => [entry.role, entry.path]));
  exactArray(
    value.command,
    expandCommand(
      requirements,
      componentPaths,
      value.provider_profile,
      value.receipt_owned_disk_image.path,
    ),
    "Colima live observed command",
  );
  return value;
}

export function buildColimaLiveObservation(input) {
  validateColimaLiveRequirements(COLIMA_LIVE_REQUIREMENTS);
  return buildObservation(COLIMA_LIVE_REQUIREMENTS, input);
}

export function validateColimaLiveObservation(value) {
  validateColimaLiveRequirements(COLIMA_LIVE_REQUIREMENTS);
  return validateObservationShape(value, COLIMA_LIVE_REQUIREMENTS);
}

export function revalidateColimaLiveObservation(value, input) {
  validateColimaLiveObservation(value);
  const rebuilt = buildColimaLiveObservation(input);
  const expected = colimaLiveBytes(value);
  const actual = colimaLiveBytes(rebuilt);
  if (expected.length !== actual.length || !timingSafeEqual(expected, actual)) {
    fail("Colima live observation changed during revalidation", 73);
  }
  return value;
}

export function observeColimaLivePreEffectRoots(value, input) {
  validateColimaLiveRequirements(COLIMA_LIVE_REQUIREMENTS);
  return observePreEffectRoots(COLIMA_LIVE_REQUIREMENTS, value, input, {
    evidenceClass: "production-pinned",
    schema: COLIMA_LIVE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
  });
}

export function authorizeColimaLiveObservation(value) {
  validateColimaLiveObservation(value);
  fail("Colima live execution remains disabled after preparation observation", 69);
}

export function colimaLivePublicProjection(value) {
  validateColimaLiveObservation(value);
  return deepFreeze({
    authorizations: { ...value.authorizations },
    host: { ...value.host },
    observation_sha256: colimaLiveDigest(colimaLiveBytes(value)),
    provider_class: value.provider_class,
    provider_kind: value.provider_kind,
    requirements_sha256: value.requirements_sha256,
    schema: COLIMA_LIVE_PUBLIC_PROJECTION_SCHEMA,
  });
}

// This seam exists only so deterministic fixtures can exercise file-identity and
// revalidation behavior without committing hundreds of megabytes of release assets.
// A future state adapter must accept only COLIMA_LIVE_REQUIREMENTS_SHA256.
export function buildColimaLiveObservationForTest(requirements, input) {
  return buildObservation(requirements, input);
}

export function validateColimaLiveObservationForTest(requirements, value) {
  return validateObservationShape(value, requirements);
}

export function revalidateColimaLiveObservationForTest(requirements, value, input) {
  validateObservationShape(value, requirements);
  const rebuilt = buildObservation(requirements, input);
  const expected = colimaLiveBytes(value);
  const actual = colimaLiveBytes(rebuilt);
  if (expected.length !== actual.length || !timingSafeEqual(expected, actual)) {
    fail("Colima live fixture observation changed during revalidation", 73);
  }
  return value;
}

// This fixture-only seam exercises the same bounded no-follow root observer
// without requiring the pinned production binaries and disk image. Its result
// is not production evidence and no supported lifecycle imports it.
export function observeColimaLivePreEffectRootsForTest(
  requirements,
  value,
  input,
  testCheckpoint = undefined,
) {
  if (testCheckpoint !== undefined && typeof testCheckpoint !== "function") {
    fail("Colima live fixture checkpoint was refused", 64);
  }
  return observePreEffectRoots(requirements, value, input, {
    evidenceClass: "fixture-only",
    schema: COLIMA_LIVE_FIXTURE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
    testCheckpoint,
  });
}

export function authorizeColimaLiveObservationForTest(requirements, value) {
  validateObservationShape(value, requirements);
  fail("Colima live execution remains disabled after preparation observation", 69);
}

export function colimaLivePublicProjectionForTest(requirements, value) {
  validateObservationShape(value, requirements);
  return deepFreeze({
    authorizations: { ...value.authorizations },
    host: { ...value.host },
    observation_sha256: colimaLiveDigest(colimaLiveBytes(value)),
    provider_class: value.provider_class,
    provider_kind: value.provider_kind,
    requirements_sha256: value.requirements_sha256,
    schema: COLIMA_LIVE_PUBLIC_PROJECTION_SCHEMA,
  });
}

export function colimaLiveLimaNetworkConfigBytesForTest() {
  return Buffer.from(LIMA_NETWORK_CONFIG_BYTES);
}
