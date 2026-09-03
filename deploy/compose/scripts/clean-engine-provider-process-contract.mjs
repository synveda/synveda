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
    "synveda-state-owner",
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
  child_process_identity_proof: "full-causal-record-hmac-sha256-v1",
  controller_group_probe: "negative-pgid-esrch-v1",
  engine_protocol: "authenticated-content-free-version-v1",
  environment_names: COLIMA_LIVE_PREPARATION_CONTRACT.environment_names,
  fixture_launch_authorized: true,
  home_policy: COLIMA_LIVE_PREPARATION_CONTRACT.home_policy,
  hostagent_protocol: "authenticated-challenge-v1",
  launch_protocol: "durable-evidence-state-veto-gate-v2",
  kind: "controlled-background-provider-v4",
  lifecycle_exposure_authorized: false,
  live_preparation_contract_sha256: COLIMA_LIVE_PREPARATION_CONTRACT_SHA256,
  max_lifetime_milliseconds: 30_000,
  optional_environment_names: COLIMA_LIVE_PREPARATION_CONTRACT.optional_environment_names,
  private_file_publication: "fsync-stage-link-no-replace-v1",
  provider_kind: "controlled-background-fake",
  root_publication: "authority-before-mkdir-owner-atomic-v1",
  root_layout: CONTROLLED_BACKGROUND_ROOT_LAYOUT,
  schema: "synveda.clean-engine.controlled-background-provider-contract.v4",
  socket_publication: "umask-0177-listen-chmod-fsync-v1",
  state_authority_gate: "synchronous-veto-only-six-checkpoint-v2",
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

export const CONTROLLED_BACKGROUND_OPERATION_KIND =
  "controlled-background-provider-create-v1";

export const CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND =
  "controlled-background-provider-cleanup-v1";

export const CONTROLLED_BACKGROUND_AUTHORITY_CHECKPOINTS = Object.freeze([
  "before-create-authority-publication",
  "before-root-publication",
  "before-controller-spawn",
  "before-start-decision-publication",
  "before-hostagent-start-delivery",
  "before-provider-identity-publication",
]);

export const CONTROLLED_BACKGROUND_RETIREMENT_AUTHORITY_CHECKPOINTS =
  Object.freeze([
    "before-retirement-plan-publication",
    "before-hostagent-shutdown-delivery",
    "before-stale-socket-unlink",
    "before-resource-unlink",
    "before-resource-rmdir",
    "before-retirement-progress-publication",
    "before-retirement-settlement-publication",
  ]);

export const CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT = Object.freeze({
  authority_checkpoint:
    "closed-effect-and-publication-frontier-with-exact-stage-identity-v1",
  create_provider_contract_sha256: CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256,
  effect_authority:
    "synchronous-state-veto-before-every-exact-effect-and-publication-frontier-v2",
  effect_checkpoints: CONTROLLED_BACKGROUND_RETIREMENT_AUTHORITY_CHECKPOINTS,
  fixture_contract: "retirement-v1-remains-fixture-only",
  kind: "controlled-background-provider-retirement-v1",
  operation_kind: CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND,
  process_stop: "authenticated-ipc-no-pid-signal-v1",
  artifact_publication: "append-only-fsync-stage-link-no-replace-strict-recovery-v2",
  provider_kind: "controlled-background-fake",
  resource_retirement: "exact-inventory-leaf-first-no-recursion-v1",
  schema: "synveda.clean-engine.controlled-background-retirement-contract.v1",
  state_integration: "mutation-journal-v2",
});

export const CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT_SHA256 =
  providerProcessDigest(providerProcessBytes(CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT));

function lowerHex(value, length) {
  return typeof value === "string" && value.length === length && /^[0-9a-f]+$/.test(value);
}

function invokeAuthorityGate(authorityGate, checkpoint, evidenceHeadSha256, revalidate) {
  if (
    typeof authorityGate !== "function" ||
    !CONTROLLED_BACKGROUND_AUTHORITY_CHECKPOINTS.includes(checkpoint) ||
    !lowerHex(evidenceHeadSha256, 64) ||
    typeof revalidate !== "function"
  ) {
    fail("controlled background authority gate was refused", 70);
  }
  const result = authorityGate(
    Object.freeze({ checkpoint, evidence_head_sha256: evidenceHeadSha256 }),
  );
  if (result !== undefined) {
    fail("controlled background authority gate returned authority", 70);
  }
  const revalidated = revalidate();
  if (revalidated !== undefined) {
    fail("controlled background authority revalidation returned authority", 70);
  }
}

function invokeRetirementAuthorityGate(authorityGate, checkpointValue, revalidate) {
  const publicationDispositions = new Set([
    "absent",
    "final",
    "linked-complete",
    "not-applicable",
    "redundant-complete",
    "redundant-partial",
    "staged-complete",
    "staged-partial",
  ]);
  const publicationPhases = new Set([
    "before-final-consumption",
    "before-final-link",
    "before-partial-stage-removal",
    "before-stage-removal",
    "before-stage-write",
    "not-applicable",
  ]);
  if (
    typeof authorityGate !== "function" ||
    checkpointValue === null ||
    Array.isArray(checkpointValue) ||
    typeof checkpointValue !== "object" ||
    typeof revalidate !== "function"
  ) {
    fail("controlled background retirement authority gate was refused", 70);
  }
  exactKeys(
    checkpointValue,
    [
      "checkpoint",
      "cleanup_intent_sha256",
      "cleanup_operation_plan_sha256",
      "cleanup_plan_sha256",
      "cleanup_slot_sequence",
      "cleanup_slot_sha256",
      "completed_steps",
      "create_close_sha256",
      "create_settlement_sha256",
      "create_slot_sha256",
      "next_action",
      "next_resources",
      "operation_kind",
      "provider_identity_sha256",
      "publication_disposition",
      "publication_expected_sha256",
      "publication_phase",
      "publication_stage_declared_sha256",
      "publication_stage_identity_sha256",
      "publication_stage_sha256",
      "publication_target_name",
      "resource_identity_sha256",
      "retirement_contract_sha256",
      "source_head_sha256",
      "source_sequence",
    ],
    "controlled background retirement authority checkpoint",
  );
  validateStateRetirementBindings({
    cleanup_intent_sha256: checkpointValue.cleanup_intent_sha256,
    cleanup_operation_plan_sha256:
      checkpointValue.cleanup_operation_plan_sha256,
    cleanup_slot_sequence: checkpointValue.cleanup_slot_sequence,
    cleanup_slot_sha256: checkpointValue.cleanup_slot_sha256,
    create_close_sha256: checkpointValue.create_close_sha256,
    create_settlement_sha256: checkpointValue.create_settlement_sha256,
    create_slot_sha256: checkpointValue.create_slot_sha256,
    source_head_sha256: checkpointValue.source_head_sha256,
    source_sequence: checkpointValue.source_sequence,
  });
  if (
    !CONTROLLED_BACKGROUND_RETIREMENT_AUTHORITY_CHECKPOINTS.includes(
      checkpointValue.checkpoint,
    ) ||
    !lowerHex(checkpointValue.cleanup_plan_sha256, 64) ||
    !Number.isSafeInteger(checkpointValue.completed_steps) ||
    checkpointValue.completed_steps < 0 ||
    typeof checkpointValue.next_action !== "string" ||
    !/^[a-z][a-z0-9-]{0,63}$/.test(checkpointValue.next_action) ||
    !Array.isArray(checkpointValue.next_resources) ||
    checkpointValue.next_resources.length < 1 ||
    checkpointValue.next_resources.some(
      (resource) => typeof resource !== "string" || resource.length < 1,
    ) ||
    checkpointValue.operation_kind !==
      CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND ||
    !publicationDispositions.has(checkpointValue.publication_disposition) ||
    !lowerHex(checkpointValue.publication_expected_sha256, 64) ||
    !publicationPhases.has(checkpointValue.publication_phase) ||
    !lowerHex(checkpointValue.publication_stage_declared_sha256, 64) ||
    !lowerHex(checkpointValue.publication_stage_identity_sha256, 64) ||
    !lowerHex(checkpointValue.publication_stage_sha256, 64) ||
    typeof checkpointValue.publication_target_name !== "string" ||
    checkpointValue.publication_target_name.length < 1 ||
    !lowerHex(checkpointValue.provider_identity_sha256, 64) ||
    !lowerHex(checkpointValue.resource_identity_sha256, 64) ||
    checkpointValue.retirement_contract_sha256 !==
      CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT_SHA256
  ) {
    fail("controlled background retirement authority gate was refused", 70);
  }
  const allowedPublicationFrontiers = new Map([
    ["before-final-consumption", new Set(["final"])],
    ["before-final-link", new Set(["staged-complete"])],
    [
      "before-partial-stage-removal",
      new Set(["redundant-partial", "staged-partial"]),
    ],
    [
      "before-stage-removal",
      new Set(["linked-complete", "redundant-complete"]),
    ],
    ["before-stage-write", new Set(["absent"])],
    ["not-applicable", new Set(["not-applicable"])],
  ]);
  const stageAbsent = new Set(["absent", "final", "not-applicable"]).has(
    checkpointValue.publication_disposition,
  );
  const publicationIsNotApplicable =
    checkpointValue.publication_disposition === "not-applicable";
  if (
    !allowedPublicationFrontiers
      .get(checkpointValue.publication_phase)
      ?.has(checkpointValue.publication_disposition) ||
    stageAbsent !==
      (checkpointValue.publication_stage_declared_sha256 === ZERO_SHA256 &&
        checkpointValue.publication_stage_identity_sha256 === ZERO_SHA256 &&
        checkpointValue.publication_stage_sha256 === ZERO_SHA256) ||
    publicationIsNotApplicable !==
      (checkpointValue.publication_expected_sha256 === ZERO_SHA256 &&
        checkpointValue.publication_target_name === "not-applicable") ||
    (!publicationIsNotApplicable &&
      (!artifactNameAllowed(checkpointValue.publication_target_name) ||
        checkpointValue.publication_expected_sha256 === ZERO_SHA256))
  ) {
    fail("controlled background retirement publication authority was refused", 70);
  }
  const effectCheckpointActions = new Map([
    ["before-hostagent-shutdown-delivery", new Set(["authenticated-hostagent-stop"])],
    ["before-stale-socket-unlink", new Set(["unlink-stale-socket"])],
    ["before-resource-unlink", new Set(["unlink", "unlink-owner"])],
    ["before-resource-rmdir", new Set(["rmdir", "rmdir-root"])],
  ]);
  const effectActions = effectCheckpointActions.get(checkpointValue.checkpoint);
  const publicationCheckpoint = new Map([
    [
      "before-retirement-plan-publication",
      {
        action: "publish-retirement-plan",
        target: (name) => name === "provider-retirement-plan.json",
      },
    ],
    [
      "before-retirement-progress-publication",
      {
        action: "publish-retirement-progress",
        target: (name) => /^retirement-step-[0-9]{2}\.json$/.test(name),
      },
    ],
    [
      "before-retirement-settlement-publication",
      {
        action: "publish-retirement-settlement",
        target: (name) => name === "provider-retirement-settlement.json",
      },
    ],
  ]).get(checkpointValue.checkpoint);
  if (
    (effectActions !== undefined &&
      (!effectActions.has(checkpointValue.next_action) ||
        checkpointValue.publication_phase !== "not-applicable")) ||
    (publicationCheckpoint !== undefined &&
      (checkpointValue.next_action !== publicationCheckpoint.action ||
        checkpointValue.publication_phase === "not-applicable" ||
        !publicationCheckpoint.target(checkpointValue.publication_target_name))) ||
    (effectActions === undefined && publicationCheckpoint === undefined)
  ) {
    fail("controlled background retirement checkpoint semantics were refused", 70);
  }
  const result = authorityGate(Object.freeze({
    ...checkpointValue,
    next_resources: Object.freeze([...checkpointValue.next_resources]),
  }));
  if (result !== undefined) {
    fail("controlled background retirement authority gate returned authority", 70);
  }
  const revalidated = revalidate();
  if (revalidated !== undefined) {
    fail("controlled background retirement authority revalidation returned authority", 70);
  }
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

function noFollowEntryIdentity(path, label) {
  let before;
  try {
    before = lstatSync(path, { bigint: true });
  } catch {
    fail(`${label} was unavailable`, 69);
  }
  const after = lstatSync(path, { bigint: true });
  if (!sameMetadata(before, after)) fail(`${label} identity changed`);
  const kind = before.isDirectory()
    ? "directory"
    : before.isFile()
      ? "file"
      : before.isSymbolicLink()
        ? "symlink"
        : before.isSocket()
          ? "socket"
          : "other";
  return Object.freeze({
    device: String(before.dev),
    inode: String(before.ino),
    kind,
    links: String(before.nlink),
    mode: (before.mode & 0o7777n).toString(8).padStart(4, "0"),
    path,
    sha256: ZERO_SHA256,
    size: String(before.size),
    uid: String(before.uid),
  });
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

function reconcileArtifactPublication(
  directory,
  name,
  expectedBytes,
  { reassertAuthority } = {},
) {
  if (reassertAuthority !== undefined && typeof reassertAuthority !== "function") {
    fail("provider process publication authority was refused", 70);
  }
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
      if (reassertAuthority?.() !== undefined) {
        fail("provider process publication authority returned a value", 70);
      }
      try {
        const currentStage = openedStage(stage.path, "provider process artifact stage");
        if (
          !sameMetadata(currentStage.metadata, stage.metadata) ||
          !currentStage.bytes.equals(expectedBytes)
        ) {
          fail("provider process artifact stage changed");
        }
        linkSync(stage.path, finalPath);
        syncDirectory(directory);
      } catch (error) {
        if (error instanceof ProviderProcessContractFailure) throw error;
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

function publishArtifact(directory, name, value, { reassertAuthority } = {}) {
  if (!artifactNameAllowed(name)) {
    fail("provider process artifact name was refused", 64);
  }
  if (reassertAuthority !== undefined && typeof reassertAuthority !== "function") {
    fail("provider process publication authority was refused", 70);
  }
  const finalPath = join(directory, name);
  const valueBytes = providerProcessBytes(value);
  const recovered = reconcileArtifactPublication(directory, name, valueBytes, {
    reassertAuthority,
  });
  if (recovered !== undefined) return canonicalArtifact(finalPath, name);
  const stageName = artifactStageName(name, providerProcessDigest(valueBytes));
  const stagePath = join(directory, stageName);
  writeExclusive(stagePath, valueBytes);
  if (reassertAuthority?.() !== undefined) {
    fail("provider process publication authority returned a value", 70);
  }
  try {
    const staged = openedStage(stagePath, "provider process artifact stage");
    if (!staged.bytes.equals(valueBytes) || staged.metadata.nlink !== 1n) {
      fail("provider process artifact stage changed");
    }
    linkSync(stagePath, finalPath);
    syncDirectory(directory);
  } catch (error) {
    if (error instanceof ProviderProcessContractFailure) throw error;
    if (error?.code === "EEXIST") {
      const existing = reconcileArtifactPublication(directory, name, valueBytes, {
        reassertAuthority,
      });
      if (existing !== undefined) return canonicalArtifact(finalPath, name);
    }
    fail(`${name} publication failed`, 70);
  }
  reconcileArtifactPublication(directory, name, valueBytes, {
    reassertAuthority,
  });
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

export function validateControlledBackgroundProviderOperationPlan(value) {
  exactKeys(
    value,
    [
      "evidence_directory",
      "fixture_id",
      "ownership_nonce",
      "provider_base",
      "provider_contract_sha256",
      "provider_kind",
      "provider_profile",
      "provider_resource",
      "provider_root_key",
      "provider_root_path",
      "schema",
      "state_integration",
    ],
    "controlled background operation plan",
  );
  if (
    value.schema !== "synveda.clean-engine.background-create-operation-plan.v1" ||
    !lowerHex(value.fixture_id, 32) ||
    !lowerHex(value.ownership_nonce, 64) ||
    value.provider_contract_sha256 !== CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256 ||
    value.provider_kind !== "controlled-background-fake" ||
    value.provider_profile !== rootKey(value.fixture_id) ||
    value.provider_resource !== `synveda-cpr45-${value.fixture_id}` ||
    value.provider_root_key !== rootKey(value.fixture_id) ||
    value.state_integration !== "mutation-journal-v2"
  ) {
    fail("controlled background operation plan was refused", 64);
  }
  validateDirectoryIdentity(value.provider_base, "controlled background operation base");
  validateDirectoryIdentity(
    value.evidence_directory,
    "controlled background operation evidence directory",
  );
  const paths = validateControlledBackgroundRoots({
    evidenceDirectory: value.evidence_directory.path,
    fixtureId: value.fixture_id,
    providerBase: value.provider_base.path,
  });
  if (
    canonical(value.provider_base) !==
      canonical(directoryIdentity(paths.base, "controlled background provider base")) ||
    canonical(value.evidence_directory) !==
      canonical(
        directoryIdentity(
          value.evidence_directory.path,
          "controlled background evidence directory",
        ),
      ) ||
    value.provider_root_path !== paths.root
  ) {
    fail("controlled background operation plan identity changed");
  }
  return paths;
}

export function planControlledBackgroundProviderOperation({
  evidenceDirectory,
  fixtureId,
  ownershipNonce,
  providerBase,
}) {
  if (!lowerHex(ownershipNonce, 64)) {
    fail("controlled background operation ownership nonce was refused", 64);
  }
  const paths = validateControlledBackgroundRoots({
    evidenceDirectory,
    fixtureId,
    providerBase,
  });
  validateEvidenceDirectoryInventory(evidenceDirectory);
  if (readdirSync(evidenceDirectory).length !== 0 || pathEntryExists(paths.root)) {
    fail("controlled background operation preflight was refused", 73);
  }
  const value = {
    evidence_directory: directoryIdentity(
      evidenceDirectory,
      "controlled background evidence directory",
    ),
    fixture_id: fixtureId,
    ownership_nonce: ownershipNonce,
    provider_base: directoryIdentity(providerBase, "controlled background provider base"),
    provider_contract_sha256: CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256,
    provider_kind: "controlled-background-fake",
    provider_profile: paths.profile,
    provider_resource: `synveda-cpr45-${fixtureId}`,
    provider_root_key: paths.profile,
    provider_root_path: paths.root,
    schema: "synveda.clean-engine.background-create-operation-plan.v1",
    state_integration: "mutation-journal-v2",
  };
  validateControlledBackgroundProviderOperationPlan(value);
  return Object.freeze(value);
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
    !new Set(["fixture-only", "mutation-journal-v2"]).has(value.state_integration)
  ) {
    fail("controlled background create bindings were refused", 64);
  }
}

function createBindingsFromAuthority(value) {
  return Object.freeze({
    create_intent_sha256: value.create_intent_sha256,
    create_slot_sequence: value.create_slot_sequence,
    create_slot_sha256: value.create_slot_sha256,
    ownership_nonce: value.ownership_nonce,
    source_head_sha256: value.source_head_sha256,
    source_sequence: value.source_sequence,
    state_integration: value.state_integration,
  });
}

function validateAuthorityCheckpointFrontier(prefix, expectedStage, paths) {
  const finalPaths = new Set(
    prefix.residual.root_inventory
      .filter(
        (entry) =>
          typeof entry.relative_path === "string" &&
          parsePrivateStageName(basename(entry.relative_path)) === undefined,
      )
      .map((entry) => entry.relative_path),
  );
  const has = (path) => finalPaths.has(relative(paths.root, path));
  const ready = has(paths.controllerReady);
  const pid = has(paths.pidRecord);
  const exactRoot =
    canonical([...finalPaths].sort()) === canonical(expectedRootPaths(paths));
  const noPrivatePublication = prefix.residual.private_publications.length === 0;
  if (
    prefix.effectFrontier.effect !== expectedStage ||
    prefix.effectFrontier.disposition !== "complete"
  ) {
    fail("controlled background authority checkpoint effect frontier was refused", 73);
  }
  const accepted =
    (expectedStage === "create-authority" &&
      prefix.replaySafe &&
      prefix.residual.root_disposition === "absent" &&
      prefix.residual.controller_presence === "not-started" &&
      prefix.residual.hostagent_presence === "not-started" &&
      prefix.residual.sockets === "absent" &&
      noPrivatePublication) ||
    (expectedStage === "controller-launch-decision" &&
      prefix.residual.root_disposition === "owned" &&
      prefix.residual.controller_presence === "unattested" &&
      prefix.residual.hostagent_presence === "not-started" &&
      !ready &&
      !pid &&
      prefix.residual.sockets === "absent" &&
      noPrivatePublication) ||
    (expectedStage === "controller-witness" &&
      prefix.residual.root_disposition === "owned" &&
      prefix.residual.controller_presence === "observed-present" &&
      prefix.residual.hostagent_presence === "not-started" &&
      ready &&
      !pid &&
      prefix.residual.sockets === "absent" &&
      noPrivatePublication) ||
    (expectedStage === "provider-start-decision" &&
      prefix.residual.root_disposition === "owned" &&
      prefix.residual.controller_presence === "observed-present" &&
      prefix.residual.hostagent_presence === "unattested" &&
      ready &&
      !pid &&
      prefix.residual.sockets === "absent" &&
      noPrivatePublication) ||
    (expectedStage === "controller-settlement" &&
      prefix.residual.root_disposition === "owned" &&
      prefix.residual.controller_presence === "proved-absent" &&
      prefix.residual.hostagent_presence === "observed-present" &&
      prefix.residual.sockets === "present" &&
      exactRoot &&
      noPrivatePublication);
  if (!accepted) {
    fail("controlled background authority checkpoint frontier was refused", 73);
  }
}

function captureAuthorityCheckpointPrefix({
  evidenceDirectory,
  expectedArtifacts,
  expectedCreateBindings,
  expectedHeadSha256,
  expectedRootInventorySha256,
  expectedStage,
  fixtureId,
  providerBase,
}) {
  const prefix = inspectControlledBackgroundProviderPrefix(evidenceDirectory, fixtureId, {
    expectedCreateBindings,
    providerBase,
    revalidateCurrentToolchain: true,
  });
  if (
    prefix.evidenceHeadSha256 !== expectedHeadSha256 ||
    prefix.evidenceStage !== expectedStage ||
    prefix.pendingPublication !== undefined
  ) {
    fail("controlled background authority checkpoint prefix was refused", 73);
  }
  const artifactNames = Object.keys(prefix.artifacts).sort();
  if (
    expectedArtifacts === null ||
    Array.isArray(expectedArtifacts) ||
    typeof expectedArtifacts !== "object" ||
    canonical(artifactNames) !== canonical(Object.keys(expectedArtifacts).sort())
  ) {
    fail("controlled background authority checkpoint artifacts were refused", 73);
  }
  for (const name of artifactNames) {
    const current = prefix.artifacts[name];
    const expected = expectedArtifacts[name];
    if (
      expected === undefined ||
      !current.bytes.equals(expected.bytes) ||
      !sameMetadata(current.metadata, expected.metadata)
    ) {
      fail("controlled background authority checkpoint artifact changed", 73);
    }
  }
  if (
    expectedRootInventorySha256 !== undefined &&
    prefix.residual.inventory_sha256 !== expectedRootInventorySha256
  ) {
    fail("controlled background authority checkpoint root changed", 73);
  }
  validateAuthorityCheckpointFrontier(prefix, expectedStage, rootPaths(providerBase, fixtureId));
  return prefix;
}

function revalidateAuthorityCheckpointPrefix(argumentsValue, expectedPrefix) {
  const current = captureAuthorityCheckpointPrefix(argumentsValue);
  if (current.residualSha256 !== expectedPrefix.residualSha256) {
    fail("controlled background authority checkpoint prefix changed", 73);
  }
}

function stagedAuthorityReassertion({
  authorityGate,
  checkpoint,
  checkpointArguments,
  evidenceHeadSha256,
  expectedPrefix,
  targetName,
  targetValue,
}) {
  const targetSha256 = providerProcessDigest(providerProcessBytes(targetValue));
  return () => {
    const result = authorityGate(
      Object.freeze({ checkpoint, evidence_head_sha256: evidenceHeadSha256 }),
    );
    if (result !== undefined) {
      fail("controlled background authority gate returned authority", 70);
    }
    const current = inspectControlledBackgroundProviderPrefix(
      checkpointArguments.evidenceDirectory,
      checkpointArguments.fixtureId,
      {
        expectedCreateBindings: checkpointArguments.expectedCreateBindings,
        providerBase: checkpointArguments.providerBase,
        revalidateCurrentToolchain: true,
      },
    );
    if (
      current.evidenceHeadSha256 !== expectedPrefix.evidenceHeadSha256 ||
      current.evidencePrefixSha256 !== expectedPrefix.evidencePrefixSha256 ||
      current.evidenceStage !== expectedPrefix.evidenceStage ||
      canonical(current.residual) !== canonical(expectedPrefix.residual) ||
      current.pendingPublication?.target_name !== targetName ||
      current.pendingPublication?.actual_sha256 !== targetSha256 ||
      current.pendingPublication?.declared_sha256 !== targetSha256 ||
      current.pendingPublication?.disposition !== "staged-complete" ||
      canonical(Object.keys(current.artifacts).sort()) !==
        canonical(Object.keys(expectedPrefix.artifacts).sort())
    ) {
      fail("controlled background staged authority checkpoint changed", 73);
    }
    for (const name of Object.keys(expectedPrefix.artifacts)) {
      const expected = expectedPrefix.artifacts[name];
      const artifact = current.artifacts[name];
      if (
        artifact === undefined ||
        !artifact.bytes.equals(expected.bytes) ||
        !sameMetadata(artifact.metadata, expected.metadata)
      ) {
        fail("controlled background staged authority artifact changed", 73);
      }
    }
  };
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

function planControlledBackgroundProviderCreateImpl(
  {
    bindings,
    evidenceDirectory,
    fixtureId,
    operationPlan,
    providerBase,
  },
  authorityGate,
) {
  if (authorityGate !== undefined && typeof authorityGate !== "function") {
    fail("controlled background create-authority gate was refused", 70);
  }
  validateCreateBindings(bindings);
  if (
    (authorityGate === undefined && bindings.state_integration !== "fixture-only") ||
    (authorityGate !== undefined && bindings.state_integration !== "mutation-journal-v2")
  ) {
    fail("controlled background create integration was refused", 64);
  }
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
  if (authorityGate !== undefined) {
    validateControlledBackgroundProviderOperationPlan(operationPlan);
    if (
      operationPlan.fixture_id !== fixtureId ||
      operationPlan.ownership_nonce !== bindings.ownership_nonce ||
      operationPlan.provider_root_path !== paths.root ||
      canonical(operationPlan.provider_base) !==
        canonical(directoryIdentity(providerBase, "controlled background provider base")) ||
      canonical(operationPlan.evidence_directory) !==
        canonical(
          directoryIdentity(
            evidenceDirectory,
            "controlled background evidence directory",
          ),
        )
    ) {
      fail("controlled background create operation plan binding was refused", 73);
    }
  } else if (operationPlan !== undefined) {
    fail("controlled background fixture operation plan was refused", 64);
  }
  assertMutationJournalOperationOpen(evidenceDirectory, bindings);
  const value = createAuthorityValue({
    bindings,
    evidenceDirectory,
    fixtureId,
    paths,
  });
  const reassertAuthority = authorityGate === undefined
    ? undefined
    : () => {
        const result = authorityGate(Object.freeze({
          authority_sha256: providerProcessDigest(providerProcessBytes(value)),
          checkpoint: "before-create-authority-publication",
          evidence_head_sha256: ZERO_SHA256,
        }));
        if (result !== undefined) {
          fail("controlled background create-authority gate returned authority", 70);
        }
        assertMutationJournalOperationOpen(evidenceDirectory, bindings);
        const currentPaths = validateControlledBackgroundRoots({
          evidenceDirectory,
          fixtureId,
          providerBase,
        });
        if (currentPaths.root !== paths.root || pathEntryExists(paths.root)) {
          fail("controlled background create-authority root changed", 73);
        }
        const prefix = inspectCreationEvidencePrefix(evidenceDirectory);
        if (
          Object.keys(prefix.artifacts).length !== 0 ||
          prefix.pendingPublication?.target_name !== "background-create-authority.json" ||
          prefix.pendingPublication?.actual_sha256 !==
            providerProcessDigest(providerProcessBytes(value)) ||
          prefix.pendingPublication?.disposition !== "staged-complete"
        ) {
          fail("controlled background create-authority prefix changed", 73);
        }
      };
  const authority = publishArtifact(
    evidenceDirectory,
    "background-create-authority.json",
    value,
    { reassertAuthority },
  );
  validateCreateAuthority(authority.value, evidenceDirectory, fixtureId, paths);
  return Object.freeze({ authority, paths });
}

export function planControlledBackgroundProviderCreate(argumentsValue) {
  return planControlledBackgroundProviderCreateImpl(argumentsValue, undefined);
}

export function planControlledBackgroundProviderCreateWithAuthorityGate(
  argumentsValue,
  authorityGate,
) {
  return planControlledBackgroundProviderCreateImpl(argumentsValue, authorityGate);
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

function privateStageTargetSha256(path) {
  return providerProcessDigest(Buffer.from(basename(path), "utf8"));
}

function privateStageName(path, valueSha256) {
  return `.background-private-stage-${privateStageTargetSha256(path)}-${valueSha256}-${randomBytes(16).toString("hex")}`;
}

function parsePrivateStageName(name) {
  const match = /^\.background-private-stage-([0-9a-f]{64})-([0-9a-f]{64})-([0-9a-f]{32})$/.exec(
    name,
  );
  if (match === null) return undefined;
  return Object.freeze({ target_sha256: match[1], value_sha256: match[2] });
}

function privateFileStages(path) {
  const directory = dirname(path);
  const targetSha256 = privateStageTargetSha256(path);
  const stages = [];
  for (const name of readdirSync(directory).sort()) {
    const parsed = parsePrivateStageName(name);
    if (parsed?.target_sha256 !== targetSha256) continue;
    const stage = openedStage(join(directory, name), "controlled background private-file stage");
    stages.push(Object.freeze({ ...parsed, ...stage, name, path: join(directory, name) }));
  }
  if (stages.length > 1) fail("controlled background private-file stages were ambiguous");
  return stages;
}

function removePrivateFileStage(path, stage) {
  const current = openedStage(stage.path, "controlled background private-file stage");
  if (!sameMetadata(current.metadata, stage.metadata) || !current.bytes.equals(stage.bytes)) {
    fail("controlled background private-file stage changed");
  }
  try {
    unlinkSync(stage.path);
    syncDirectory(dirname(path));
  } catch {
    fail("controlled background private-file stage retirement failed", 70);
  }
}

function reconcilePrivateFilePublication(path, expectedBytes) {
  const expectedSha256 = providerProcessDigest(expectedBytes);
  let final;
  if (pathEntryExists(path)) {
    final = openedFile(
      path,
      "controlled background private file",
      MAX_ARTIFACT_BYTES,
      new Set([1n, 2n]),
      true,
    );
    if (!final.bytes.equals(expectedBytes)) {
      fail("controlled background private file changed");
    }
  }
  const stages = privateFileStages(path);
  for (const stage of stages) {
    if (stage.value_sha256 !== expectedSha256) {
      fail("controlled background private-file stage digest was refused");
    }
    let linkedToFinal =
      final !== undefined &&
      stage.metadata.dev === final.metadata.dev &&
      stage.metadata.ino === final.metadata.ino;
    if (stage.metadata.nlink === 2n && !linkedToFinal) {
      fail("controlled background private-file stage link was foreign");
    }
    if (stage.metadata.nlink === 1n && !stage.bytes.equals(expectedBytes)) {
      removePrivateFileStage(path, stage);
      continue;
    }
    if (stage.metadata.nlink === 1n && final === undefined) {
      try {
        linkSync(stage.path, path);
        syncDirectory(dirname(path));
      } catch (error) {
        if (error?.code !== "EEXIST") {
          fail("controlled background private-file recovery publication failed", 70);
        }
      }
      final = openedFile(
        path,
        "controlled background private file",
        MAX_ARTIFACT_BYTES,
        new Set([1n, 2n]),
        true,
      );
      if (!final.bytes.equals(expectedBytes)) {
        fail("controlled background private file changed");
      }
      const linkedStage = openedStage(
        stage.path,
        "controlled background private-file stage",
      );
      linkedToFinal =
        linkedStage.metadata.dev === final.metadata.dev &&
        linkedStage.metadata.ino === final.metadata.ino;
      if (
        linkedStage.metadata.nlink !== 2n ||
        !linkedToFinal ||
        !linkedStage.bytes.equals(expectedBytes)
      ) {
        fail("controlled background private-file recovery link changed");
      }
      removePrivateFileStage(path, { ...stage, ...linkedStage });
      final = openedFile(
        path,
        "controlled background private file",
        MAX_ARTIFACT_BYTES,
        new Set([1n]),
        true,
      );
      continue;
    }
    if (stage.metadata.nlink === 1n && final !== undefined) {
      removePrivateFileStage(path, stage);
      continue;
    }
    const current = openedStage(stage.path, "controlled background private-file stage");
    const currentFinal = openedFile(
      path,
      "controlled background private file",
      MAX_ARTIFACT_BYTES,
      new Set([2n]),
      true,
    );
    if (
      current.metadata.dev !== currentFinal.metadata.dev ||
      current.metadata.ino !== currentFinal.metadata.ino ||
      !current.bytes.equals(expectedBytes)
    ) {
      fail("controlled background private-file recovery link changed");
    }
    removePrivateFileStage(path, { ...stage, ...current });
    final = openedFile(
      path,
      "controlled background private file",
      MAX_ARTIFACT_BYTES,
      new Set([1n]),
      true,
    );
  }
  return final;
}

function publishPrivateFile(path, value) {
  const valueBytes = providerProcessBytes(value);
  const recovered = reconcilePrivateFilePublication(path, valueBytes);
  if (recovered === undefined) {
    const stagePath = join(
      dirname(path),
      privateStageName(path, providerProcessDigest(valueBytes)),
    );
    writeExclusive(stagePath, valueBytes);
    try {
      linkSync(stagePath, path);
      syncDirectory(dirname(path));
    } catch (error) {
      if (error?.code !== "EEXIST") {
        fail("controlled background private-file publication failed", 70);
      }
    }
    reconcilePrivateFilePublication(path, valueBytes);
  }
  const final = openedFile(
    path,
    "controlled background private file",
    MAX_ARTIFACT_BYTES,
    new Set([1n]),
    true,
  );
  if (!final.bytes.equals(valueBytes)) fail("controlled background private file changed");
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

function artifactProof(secret, action, value) {
  return createHmac("sha256", secret)
    .update(`synveda.clean-engine.background-provider-artifact.v1\0${action}\0`)
    .update(providerProcessBytes(value))
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

function requestSocket(
  path,
  request,
  timeoutMilliseconds = 2_000,
  beforeWrite,
) {
  if (beforeWrite !== undefined && typeof beforeWrite !== "function") {
    fail("provider process request boundary was refused", 70);
  }
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
      try {
        const result = beforeWrite?.();
        if (result !== undefined) {
          fail("provider process request boundary returned authority", 70);
        }
        socket.end(`${canonical(request)}\n`);
      } catch (error) {
        finish(
          error instanceof ProviderProcessContractFailure
            ? error
            : new ProviderProcessContractFailure(
                "provider process request boundary was refused",
                70,
              ),
        );
      }
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

function processObservation(disposition) {
  if (disposition === "present") return "observed-present";
  if (disposition === "absent") return "proved-absent";
  if (disposition === "unknown") return "unattested";
  fail("controlled background process observation was refused", 70);
}

async function waitForCanonicalArtifact(path, label, timeoutMilliseconds = 8_000) {
  const deadline = Date.now() + timeoutMilliseconds;
  while (Date.now() < deadline) {
    if (pathEntryExists(path)) {
      try {
        return canonicalArtifact(path, label);
      } catch {
        // The fixed child publishes through a durable hard link. A reader may
        // observe the linked final before the private stage name is retired.
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
    const onClose = (status, signal) =>
      finish(
        new ProviderProcessContractFailure(
          `controller closed before start: ${status ?? signal ?? "unknown"}`,
          69,
        ),
      );
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
      "controller_config_sha256",
      "controller_launch_decision_sha256",
      "controller_pgid",
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
    value.schema !== "synveda.clean-engine.background-controller-ready.v3" ||
    value.fixture_id !== expected.fixtureId ||
    value.controller_config_sha256 !== expected.controllerConfigSha256 ||
    value.controller_launch_decision_sha256 !== expected.controllerLaunchDecisionSha256 ||
    value.controller_pgid !== expected.controllerPgid ||
    value.controller_pid !== expected.controllerPid ||
    value.controller_pgid !== value.controller_pid ||
    !lowerHex(value.controller_process_instance_sha256, 64) ||
    value.controller_script_sha256 !== expected.controllerScriptSha256 ||
    value.node_sha256 !== expected.nodeSha256 ||
    value.working_directory !== expected.workingDirectory ||
    !proofEquals(
      value.proof_sha256,
      artifactProof(secret, "controller-ready", {
        controller_config_sha256: value.controller_config_sha256,
        controller_environment_keys: value.controller_environment_keys,
        controller_launch_decision_sha256: value.controller_launch_decision_sha256,
        controller_pgid: value.controller_pgid,
        controller_pid: value.controller_pid,
        controller_process_instance_sha256: value.controller_process_instance_sha256,
        controller_script_sha256: value.controller_script_sha256,
        fixture_id: value.fixture_id,
        node_sha256: value.node_sha256,
        schema: value.schema,
        working_directory: value.working_directory,
      }),
    )
  ) {
    fail("controlled background controller readiness was refused");
  }
}

function validatePidRecord(value, expected, toolchain, secret) {
  exactKeys(
    value,
    [
      "environment_keys",
      "controller_witness_sha256",
      "fixture_id",
      "hostagent_config_sha256",
      "hostagent_script_sha256",
      "instance_nonce_sha256",
      "node_sha256",
      "pid",
      "ppid",
      "process_instance_sha256",
      "profile",
      "proof_sha256",
      "provider_start_decision_sha256",
      "schema",
      "working_directory",
    ],
    "controlled background hostagent PID record",
  );
  validateEnvironmentKeys(value.environment_keys, "hostagent environment");
  const components = new Map(toolchain.components.map((component) => [component.role, component]));
  if (
    value.schema !== "synveda.clean-engine.background-hostagent-pid.v2" ||
    value.fixture_id !== expected.fixtureId ||
    value.controller_witness_sha256 !== expected.controllerWitnessSha256 ||
    value.hostagent_config_sha256 !== expected.hostagentConfigSha256 ||
    value.profile !== expected.profile ||
    value.pid !== expected.pid ||
    value.ppid !== expected.controllerPid ||
    value.process_instance_sha256 !== expected.processIdentity ||
    value.provider_start_decision_sha256 !== expected.providerStartDecisionSha256 ||
    value.instance_nonce_sha256 !== expected.instanceNonceSha256 ||
    value.hostagent_script_sha256 !== components.get("hostagent-script")?.sha256 ||
    value.node_sha256 !== components.get("node-runtime")?.sha256 ||
    value.working_directory !== expected.workingDirectory ||
    !proofEquals(
      value.proof_sha256,
      artifactProof(secret, "hostagent-pid", {
        controller_witness_sha256: value.controller_witness_sha256,
        environment_keys: value.environment_keys,
        fixture_id: value.fixture_id,
        hostagent_config_sha256: value.hostagent_config_sha256,
        hostagent_script_sha256: value.hostagent_script_sha256,
        instance_nonce_sha256: value.instance_nonce_sha256,
        node_sha256: value.node_sha256,
        pid: value.pid,
        ppid: value.ppid,
        process_instance_sha256: value.process_instance_sha256,
        profile: value.profile,
        provider_start_decision_sha256: value.provider_start_decision_sha256,
        schema: value.schema,
        working_directory: value.working_directory,
      }),
    )
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
      controllerWitnessSha256: evidence.controllerWitness.sha256,
      controllerPid: evidence.controllerWitness.value.controller_pid,
      fixtureId,
      hostagentConfigSha256: evidence.controllerWitness.value.hostagent_config_sha256,
      instanceNonceSha256: providerProcessDigest(Buffer.from(instanceNonce, "ascii")),
      pid: pidRecord.value.pid,
      processIdentity: evidence.hostagentWitness.value.process_instance_sha256,
      profile: paths.profile,
      providerStartDecisionSha256: evidence.providerStartDecision.sha256,
      workingDirectory: paths.root,
    },
    evidence.toolchain.value,
    instanceNonce,
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

function assertMutationJournalOperationOpen(evidenceDirectory, bindings) {
  if (bindings.state_integration !== "mutation-journal-v2") return;
  if (basename(evidenceDirectory) !== "provider") {
    fail("controlled background state evidence directory was refused", 73);
  }
  const runDirectory = dirname(evidenceDirectory);
  secureDirectory(runDirectory, "controlled background state run directory");
  const sequence = String(bindings.create_slot_sequence).padStart(2, "0");
  const slot = readCanonicalArtifactOnly(
    join(runDirectory, `.mutation-slot-${sequence}`),
    "controlled background state mutation slot",
  );
  if (slot.sha256 !== bindings.create_slot_sha256) {
    fail("controlled background state mutation slot changed", 73);
  }
  if (
    pathEntryExists(join(runDirectory, `.mutation-operation-${sequence}`)) ||
    pathEntryExists(join(runDirectory, `.mutation-close-${sequence}`)) ||
    readdirSync(runDirectory).some((name) => /^\.mutation-stage-[0-9a-f]{32}$/.test(name))
  ) {
    fail("controlled background state operation was already settling", 73);
  }
}

async function launchControlledBackgroundProviderImpl({
  beforeDetachHoldMilliseconds = 0,
  beforeIdentityProbeHoldMilliseconds = 0,
  beforeStartDecisionHoldMilliseconds = 0,
  beforeStartHoldMilliseconds = 0,
  evidenceDirectory,
  fixtureId,
  maximumLifetimeMilliseconds = 30_000,
  providerBase,
  requireShutdownDuringStart = false,
}, authorityGate, allowedStateIntegrations) {
  if (typeof authorityGate !== "function") {
    fail("controlled background authority gate was refused", 70);
  }
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
    !Number.isSafeInteger(beforeIdentityProbeHoldMilliseconds) ||
    beforeIdentityProbeHoldMilliseconds < 0 ||
    beforeIdentityProbeHoldMilliseconds > 5_000 ||
    !Number.isSafeInteger(beforeStartDecisionHoldMilliseconds) ||
    beforeStartDecisionHoldMilliseconds < 0 ||
    beforeStartDecisionHoldMilliseconds > 5_000 ||
    !Number.isSafeInteger(beforeStartHoldMilliseconds) ||
    beforeStartHoldMilliseconds < 0 ||
    beforeStartHoldMilliseconds > 5_000 ||
    typeof requireShutdownDuringStart !== "boolean"
  ) {
    fail("controlled background pre-start hold was refused", 64);
  }
  const engineArchitecture = controlledBackgroundEngineArchitecture(process.arch);
  const preliminaryPrefix = inspectCreationEvidencePrefix(evidenceDirectory);
  const preliminaryAuthority =
    preliminaryPrefix.artifacts["background-create-authority.json"];
  if (preliminaryAuthority === undefined) {
    fail("background-create-authority.json was unavailable", 69);
  }
  if (!allowedStateIntegrations.has(preliminaryAuthority.value.state_integration)) {
    fail("controlled background launch integration was refused", 73);
  }
  const stateIntegrated = preliminaryAuthority.value.state_integration === "mutation-journal-v2";
  if (stateIntegrated && preliminaryPrefix.pendingPublication !== undefined) {
    fail("controlled background state authority publication was incomplete", 73);
  }
  assertMutationJournalOperationOpen(evidenceDirectory, preliminaryAuthority.value);
  const authority = stateIntegrated
    ? preliminaryAuthority
    : canonicalArtifact(
        join(evidenceDirectory, "background-create-authority.json"),
        "background-create-authority.json",
      );
  const effectiveAuthorityGate = stateIntegrated
    ? (checkpoint) => {
        assertMutationJournalOperationOpen(evidenceDirectory, authority.value);
        return authorityGate(checkpoint);
      }
    : authorityGate;
  validateEvidenceDirectoryInventory(evidenceDirectory);
  exactArray(
    readdirSync(evidenceDirectory).sort(),
    ["background-create-authority.json"],
    "controlled background pre-launch evidence inventory",
  );
  const expectedCreateBindings = createBindingsFromAuthority(authority.value);
  const rootCheckpointArguments = {
    evidenceDirectory,
    expectedArtifacts: { "background-create-authority.json": authority },
    expectedCreateBindings,
    expectedHeadSha256: authority.sha256,
    expectedStage: "create-authority",
    fixtureId,
    providerBase,
  };
  const rootCheckpoint = captureAuthorityCheckpointPrefix(rootCheckpointArguments);
  invokeAuthorityGate(
    effectiveAuthorityGate,
    "before-root-publication",
    authority.sha256,
    () => revalidateAuthorityCheckpointPrefix(rootCheckpointArguments, rootCheckpoint),
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
  const controllerCheckpointArguments = {
    evidenceDirectory,
    expectedArtifacts: {
      "background-create-authority.json": authority,
      "background-toolchain.json": toolchain,
      "controller-launch-decision.json": controllerLaunchDecision,
    },
    expectedCreateBindings,
    expectedHeadSha256: controllerLaunchDecision.sha256,
    expectedStage: "controller-launch-decision",
    fixtureId,
    providerBase,
  };
  const controllerCheckpoint = captureAuthorityCheckpointPrefix(
    controllerCheckpointArguments,
  );
  invokeAuthorityGate(
    effectiveAuthorityGate,
    "before-controller-spawn",
    controllerLaunchDecision.sha256,
    () =>
      revalidateAuthorityCheckpointPrefix(
        controllerCheckpointArguments,
        controllerCheckpoint,
      ),
  );
  const controller = spawn(
    components.get("node-runtime").path,
    [
      "--input-type=module",
      "--eval",
      toolchainBundle.execution.controllerSource,
      root.paths.controllerConfig,
      controllerConfigArtifact.sha256,
      controllerLaunchDecision.sha256,
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
        controllerConfigSha256: controllerConfigArtifact.sha256,
        controllerLaunchDecisionSha256: controllerLaunchDecision.sha256,
        controllerPgid: controller.pid,
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
      argv_contract: "repository-digest-bound-eval-controller-v3",
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
    const startDecisionCheckpointArguments = {
      evidenceDirectory,
      expectedArtifacts: {
        "background-create-authority.json": authority,
        "background-toolchain.json": toolchain,
        "controller-launch-decision.json": controllerLaunchDecision,
        "controller-witness.json": controllerWitness,
      },
      expectedCreateBindings,
      expectedHeadSha256: controllerWitness.sha256,
      expectedStage: "controller-witness",
      fixtureId,
      providerBase,
    };
    const startDecisionCheckpoint = captureAuthorityCheckpointPrefix(
      startDecisionCheckpointArguments,
    );
    if (beforeStartDecisionHoldMilliseconds > 0) {
      await new Promise((resolvePromise) =>
        setTimeout(resolvePromise, beforeStartDecisionHoldMilliseconds),
      );
    }
    const startDecisionValue = {
      controller_witness_sha256: controllerWitness.sha256,
      create_authority_sha256: authority.sha256,
      create_intent_sha256: authority.value.create_intent_sha256,
      create_slot_sha256: authority.value.create_slot_sha256,
      decision: "start",
      fixture_id: fixtureId,
      schema: "synveda.clean-engine.background-provider-start-decision.v1",
    };
    const startDecision = publishArtifact(
      evidenceDirectory,
      "provider-start-decision.json",
      startDecisionValue,
      {
        reassertAuthority: stagedAuthorityReassertion({
          authorityGate: effectiveAuthorityGate,
          checkpoint: "before-start-decision-publication",
          checkpointArguments: startDecisionCheckpointArguments,
          evidenceHeadSha256: controllerWitness.sha256,
          expectedPrefix: startDecisionCheckpoint,
          targetName: "provider-start-decision.json",
          targetValue: startDecisionValue,
        }),
      },
    );
    if (beforeStartHoldMilliseconds > 0) {
      await new Promise((resolvePromise) =>
        setTimeout(resolvePromise, beforeStartHoldMilliseconds),
      );
    }
    const hostagentStartCheckpointArguments = {
      evidenceDirectory,
      expectedArtifacts: {
        "background-create-authority.json": authority,
        "background-toolchain.json": toolchain,
        "controller-launch-decision.json": controllerLaunchDecision,
        "controller-witness.json": controllerWitness,
        "provider-start-decision.json": startDecision,
      },
      expectedCreateBindings,
      expectedHeadSha256: startDecision.sha256,
      expectedRootInventorySha256: startDecisionCheckpoint.residual.inventory_sha256,
      expectedStage: "provider-start-decision",
      fixtureId,
      providerBase,
    };
    const hostagentStartCheckpoint = captureAuthorityCheckpointPrefix(
      hostagentStartCheckpointArguments,
    );
    invokeAuthorityGate(
      effectiveAuthorityGate,
      "before-hostagent-start-delivery",
      startDecision.sha256,
      () =>
        revalidateAuthorityCheckpointPrefix(
          hostagentStartCheckpointArguments,
          hostagentStartCheckpoint,
        ),
    );
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
        controllerWitnessSha256: controllerWitness.sha256,
        controllerPid: controller.pid,
        fixtureId,
        hostagentConfigSha256: hostagentConfigArtifact.sha256,
        instanceNonceSha256: providerProcessDigest(
          Buffer.from(root.ownershipNonce, "ascii"),
        ),
        pid: started.hostagent_pid,
        processIdentity: started.hostagent_process_instance_sha256,
        profile: root.paths.profile,
        providerStartDecisionSha256: startDecision.sha256,
        workingDirectory: root.paths.root,
      },
      toolchainValue,
      root.ownershipNonce,
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
    const identityCheckpointArguments = {
      evidenceDirectory,
      expectedArtifacts: {
        "background-create-authority.json": authority,
        "background-toolchain.json": toolchain,
        "context-witness.json": contextWitness,
        "controller-launch-decision.json": controllerLaunchDecision,
        "controller-settlement.json": controllerSettlement,
        "controller-witness.json": controllerWitness,
        "engine-witness.json": engineWitness,
        "hostagent-witness.json": hostagentWitness,
        "provider-start-decision.json": startDecision,
      },
      expectedCreateBindings,
      expectedHeadSha256: controllerSettlement.sha256,
      expectedRootInventorySha256: providerProcessDigest(
        providerProcessBytes(creationInventory),
      ),
      expectedStage: "controller-settlement",
      fixtureId,
      providerBase,
    };
    const identityCheckpoint = captureAuthorityCheckpointPrefix(
      identityCheckpointArguments,
    );
    if (beforeIdentityProbeHoldMilliseconds > 0) {
      await new Promise((resolvePromise) =>
        setTimeout(resolvePromise, beforeIdentityProbeHoldMilliseconds),
      );
    }
    const hostagentAtIdentity = await probeHostagent(
      root.paths,
      fixtureId,
      root.ownershipNonce,
      started.hostagent_pid,
      started.hostagent_process_instance_sha256,
    );
    const engineAtIdentity = await probeEngine(
      root.paths,
      fixtureId,
      root.ownershipNonce,
      started.hostagent_process_instance_sha256,
    );
    revalidateAuthorityCheckpointPrefix(
      identityCheckpointArguments,
      identityCheckpoint,
    );
    if (
      providerProcessDigest(providerProcessBytes(stableHostagentProbe(hostagentAtIdentity))) !==
        controllerSettlementValue.hostagent_after_controller_sha256 ||
      providerProcessDigest(providerProcessBytes(stableEngineProbe(engineAtIdentity))) !==
        controllerSettlementValue.engine_after_controller_sha256 ||
      canonical(inspectRootInventory(root.paths)) !== canonical(creationInventory)
    ) {
      fail("controlled background provider identity authority changed", 73);
    }
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
    const providerIdentityBytes = providerProcessBytes(providerIdentityValue);
    validateCreationArtifactChain(
      {
        "background-create-authority.json": authority,
        "background-toolchain.json": toolchain,
        "context-witness.json": contextWitness,
        "controller-launch-decision.json": controllerLaunchDecision,
        "controller-settlement.json": controllerSettlement,
        "controller-witness.json": controllerWitness,
        "engine-witness.json": engineWitness,
        "hostagent-witness.json": hostagentWitness,
        "provider-identity.json": Object.freeze({
          bytes: providerIdentityBytes,
          path: join(evidenceDirectory, "provider-identity.json"),
          sha256: providerProcessDigest(providerIdentityBytes),
          value: providerIdentityValue,
        }),
        "provider-start-decision.json": startDecision,
      },
      fixtureId,
      { revalidateCurrentToolchain: true },
    );
    const providerIdentity = publishArtifact(
      evidenceDirectory,
      "provider-identity.json",
      providerIdentityValue,
      {
        reassertAuthority: stagedAuthorityReassertion({
          authorityGate: effectiveAuthorityGate,
          checkpoint: "before-provider-identity-publication",
          checkpointArguments: identityCheckpointArguments,
          evidenceHeadSha256: controllerSettlement.sha256,
          expectedPrefix: identityCheckpoint,
          targetName: "provider-identity.json",
          targetValue: providerIdentityValue,
        }),
      },
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

export async function launchControlledBackgroundProvider(argumentsValue) {
  return launchControlledBackgroundProviderImpl(
    argumentsValue,
    () => undefined,
    new Set(["fixture-only"]),
  );
}

export async function launchControlledBackgroundProviderWithAuthorityGate(
  argumentsValue,
  authorityGate,
) {
  return launchControlledBackgroundProviderImpl(
    argumentsValue,
    authorityGate,
    new Set(["fixture-only", "mutation-journal-v2"]),
  );
}

function readCreationArtifacts(evidenceDirectory) {
  const prefix = inspectCreationEvidencePrefix(evidenceDirectory, {
    allowRetirementEvidence: true,
  });
  if (
    Object.keys(prefix.artifacts).length !==
      CONTROLLED_BACKGROUND_CREATION_ARTIFACTS.length ||
    CONTROLLED_BACKGROUND_CREATION_ARTIFACTS.some(
      (name) => prefix.artifacts[name] === undefined,
    ) ||
    (prefix.pendingPublication !== undefined &&
      prefix.pendingPublication.disposition !== "linked-complete")
  ) {
    fail("controlled background provider evidence was incomplete");
  }
  return prefix.artifacts;
}

function readCanonicalArtifactOnly(path, label, expectedLinks = new Set([1n])) {
  const opened = openedFile(path, label, MAX_ARTIFACT_BYTES, expectedLinks, true);
  let value;
  try {
    value = JSON.parse(opened.bytes.toString("utf8"));
  } catch {
    fail(`${label} was not canonical JSON`);
  }
  if (!providerProcessBytes(value).equals(opened.bytes)) {
    fail(`${label} was not canonical JSON`);
  }
  return Object.freeze({
    bytes: opened.bytes,
    metadata: opened.metadata,
    path,
    sha256: providerProcessDigest(opened.bytes),
    value,
  });
}

function inspectCreationEvidencePrefix(
  evidenceDirectory,
  { allowRetirementEvidence = false } = {},
) {
  secureDirectory(evidenceDirectory, "controlled background evidence directory");
  const entries = readdirSync(evidenceDirectory).sort();
  if (entries.length > MAX_EVIDENCE_ENTRIES) {
    fail("controlled background evidence capacity was exceeded");
  }
  const finals = entries
    .filter((name) => CONTROLLED_BACKGROUND_CREATION_ARTIFACTS.includes(name))
    .sort(
      (left, right) =>
        CONTROLLED_BACKGROUND_CREATION_ARTIFACTS.indexOf(left) -
        CONTROLLED_BACKGROUND_CREATION_ARTIFACTS.indexOf(right),
    );
  const stages = [];
  for (const name of entries) {
    if (CONTROLLED_BACKGROUND_CREATION_ARTIFACTS.includes(name)) continue;
    const parsed = parseArtifactStageName(name);
    if (
      allowRetirementEvidence &&
      ((artifactNameAllowed(name) &&
        !CONTROLLED_BACKGROUND_CREATION_ARTIFACTS.includes(name)) ||
        (parsed !== undefined &&
          artifactNameAllowed(parsed.targetName) &&
          !CONTROLLED_BACKGROUND_CREATION_ARTIFACTS.includes(parsed.targetName)))
    ) {
      continue;
    }
    if (
      parsed === undefined ||
      !CONTROLLED_BACKGROUND_CREATION_ARTIFACTS.includes(parsed.targetName)
    ) {
      fail("controlled background create-prefix evidence entry was refused");
    }
    const stage = openedStage(
      join(evidenceDirectory, name),
      "controlled background create-prefix stage",
    );
    stages.push(Object.freeze({ ...parsed, ...stage, name, path: join(evidenceDirectory, name) }));
  }
  if (stages.length > 1) {
    fail("controlled background create-prefix stages were ambiguous");
  }
  const indexes = finals.map((name) => CONTROLLED_BACKGROUND_CREATION_ARTIFACTS.indexOf(name));
  if (indexes.some((value, index) => value !== index)) {
    fail("controlled background creation artifact chain had a gap");
  }
  const linkedFinals = new Map();
  for (const stage of stages) {
    const targetIndex = CONTROLLED_BACKGROUND_CREATION_ARTIFACTS.indexOf(stage.targetName);
    const targetPath = join(evidenceDirectory, stage.targetName);
    const hasTarget = finals.includes(stage.targetName);
    if (stage.metadata.nlink === 2n) {
      if (!hasTarget || targetIndex !== finals.length - 1) {
        fail("controlled background create-prefix stage link was foreign");
      }
      const target = openedFile(
        targetPath,
        stage.targetName,
        MAX_ARTIFACT_BYTES,
        new Set([2n]),
        true,
      );
      if (
        target.metadata.dev !== stage.metadata.dev ||
        target.metadata.ino !== stage.metadata.ino ||
        !target.bytes.equals(stage.bytes) ||
        providerProcessDigest(stage.bytes) !== stage.sha256
      ) {
        fail("controlled background create-prefix stage link was refused");
      }
      linkedFinals.set(stage.targetName, stage.name);
    } else if (targetIndex !== finals.length || hasTarget) {
      fail("controlled background create-prefix stage order was refused");
    }
  }
  const artifacts = {};
  for (const name of finals) {
    artifacts[name] = readCanonicalArtifactOnly(
      join(evidenceDirectory, name),
      name,
      new Set([linkedFinals.has(name) ? 2n : 1n]),
    );
  }
  const pending = stages[0];
  let pendingPublication;
  if (pending !== undefined) {
    const actualSha256 = providerProcessDigest(pending.bytes);
    let canonicalComplete = false;
    try {
      const value = JSON.parse(pending.bytes.toString("utf8"));
      canonicalComplete = providerProcessBytes(value).equals(pending.bytes);
    } catch {
      canonicalComplete = false;
    }
    if (canonicalComplete && actualSha256 !== pending.sha256) {
      fail("controlled background create-prefix stage digest was refused");
    }
    if (actualSha256 === pending.sha256 && !canonicalComplete) {
      fail("controlled background complete create-prefix stage was malformed");
    }
    pendingPublication = Object.freeze({
      actual_sha256: actualSha256,
      declared_sha256: pending.sha256,
      device: String(pending.metadata.dev),
      disposition:
        pending.metadata.nlink === 2n
          ? "linked-complete"
          : canonicalComplete
            ? "staged-complete"
            : "staged-partial",
      links: Number(pending.metadata.nlink),
      inode: String(pending.metadata.ino),
      mode: (pending.metadata.mode & 0o7777n).toString(8).padStart(4, "0"),
      name: pending.name,
      size: String(pending.metadata.size),
      target_name: pending.targetName,
      uid: String(pending.metadata.uid),
    });
  }
  return Object.freeze({
    artifacts: Object.freeze(artifacts),
    pendingArtifact: pending,
    pendingPublication,
  });
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
  const recordedProviderRoot = createAuthority.value?.provider_root_path;
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
    controllerWitness.value.argv_contract !== "repository-digest-bound-eval-controller-v3" ||
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

function partialPrivateArtifact(path, label) {
  return readCanonicalArtifactOnly(path, label, new Set([1n, 2n]));
}

function validateCreationArtifactPrefix(
  artifacts,
  fixtureId,
  { expectedCreateBindings, providerBase, revalidateCurrentToolchain = false } = {},
) {
  const names = Object.keys(artifacts);
  if (names.length === 0) {
    return Object.freeze({ evidence: Object.freeze({}), paths: undefined });
  }
  const createAuthority = artifacts["background-create-authority.json"];
  if (createAuthority === undefined) {
    fail("controlled background create authority was unavailable", 69);
  }
  const recordedRoot = createAuthority.value?.provider_root_path;
  const recordedBase = createAuthority.value?.base?.path;
  if (
    typeof recordedRoot !== "string" ||
    !isAbsolute(recordedRoot) ||
    typeof recordedBase !== "string" ||
    !isAbsolute(recordedBase)
  ) {
    fail("controlled background provider root was refused");
  }
  if (providerBase !== undefined && providerBase !== recordedBase) {
    fail("controlled background provider base binding changed");
  }
  const paths = rootPaths(recordedBase, fixtureId);
  if (paths.root !== recordedRoot) {
    fail("controlled background provider root was refused");
  }
  validateCreateAuthority(createAuthority.value, dirname(createAuthority.path), fixtureId, paths);
  if (expectedCreateBindings !== undefined) {
    validateCreateBindings(expectedCreateBindings);
    const recorded = {
      create_intent_sha256: createAuthority.value.create_intent_sha256,
      create_slot_sequence: createAuthority.value.create_slot_sequence,
      create_slot_sha256: createAuthority.value.create_slot_sha256,
      ownership_nonce: createAuthority.value.ownership_nonce,
      source_head_sha256: createAuthority.value.source_head_sha256,
      source_sequence: createAuthority.value.source_sequence,
      state_integration: createAuthority.value.state_integration,
    };
    if (canonical(recorded) !== canonical(expectedCreateBindings)) {
      fail("controlled background create authority binding changed");
    }
  }
  const toolchain = artifacts["background-toolchain.json"];
  if (toolchain !== undefined) {
    validateToolchain(toolchain.value, fixtureId, {
      revalidateCurrent: revalidateCurrentToolchain,
    });
  }
  const controllerLaunchDecision = artifacts["controller-launch-decision.json"];
  let rootOwner;
  let hostagentConfig;
  let controllerConfig;
  if (controllerLaunchDecision !== undefined) {
    if (toolchain === undefined) fail("controlled background launch lacked its toolchain");
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
      controllerLaunchDecision.value.toolchain_sha256 !== toolchain.sha256 ||
      !lowerHex(controllerLaunchDecision.value.controller_config_sha256, 64) ||
      !lowerHex(controllerLaunchDecision.value.controller_nonce_sha256, 64) ||
      !lowerHex(controllerLaunchDecision.value.hostagent_config_sha256, 64) ||
      !lowerHex(controllerLaunchDecision.value.root_owner_sha256, 64)
    ) {
      fail("controlled background controller launch decision was refused");
    }
    rootOwner = partialPrivateArtifact(paths.ownerMarker, "controlled background root owner");
    validateRootOwner(rootOwner.value, {
      createAuthoritySha256: createAuthority.sha256,
      fixtureId,
      paths,
    });
    hostagentConfig = partialPrivateArtifact(
      paths.hostagentConfig,
      "controlled background hostagent config",
    );
    controllerConfig = partialPrivateArtifact(
      paths.controllerConfig,
      "controlled background controller config",
    );
    if (
      rootOwner.sha256 !== controllerLaunchDecision.value.root_owner_sha256 ||
      hostagentConfig.sha256 !== controllerLaunchDecision.value.hostagent_config_sha256 ||
      controllerConfig.sha256 !== controllerLaunchDecision.value.controller_config_sha256 ||
      hostagentConfig.value.schema !== "synveda.clean-engine.background-hostagent-config.v1" ||
      hostagentConfig.value.fixture_id !== fixtureId ||
      hostagentConfig.value.instance_nonce !== createAuthority.value.ownership_nonce ||
      hostagentConfig.value.working_directory !== paths.root ||
      controllerConfig.value.schema !== "synveda.clean-engine.background-controller-config.v2" ||
      controllerConfig.value.fixture_id !== fixtureId ||
      controllerConfig.value.create_authority_sha256 !== createAuthority.sha256 ||
      controllerConfig.value.hostagent_config_sha256 !== hostagentConfig.sha256 ||
      controllerConfig.value.root_owner_sha256 !== rootOwner.sha256 ||
      controllerConfig.value.working_directory !== paths.root
    ) {
      fail("controlled background controller launch inputs were refused");
    }
  }
  const controllerWitness = artifacts["controller-witness.json"];
  if (controllerWitness !== undefined) {
    if (controllerLaunchDecision === undefined || controllerConfig === undefined) {
      fail("controlled background controller witness lacked launch authority");
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
      controllerWitness.value.argv_contract !== "repository-digest-bound-eval-controller-v3" ||
      controllerWitness.value.execution_protocol !== "authenticated-ipc-start-shutdown-v2" ||
      controllerWitness.value.controller_launch_decision_sha256 !== controllerLaunchDecision.sha256 ||
      controllerWitness.value.create_authority_sha256 !== createAuthority.sha256 ||
      controllerWitness.value.controller_config_sha256 !== controllerConfig.sha256 ||
      controllerWitness.value.controller_nonce_sha256 !==
        controllerLaunchDecision.value.controller_nonce_sha256 ||
      controllerWitness.value.hostagent_config_sha256 !== hostagentConfig.sha256 ||
      controllerWitness.value.root_owner_sha256 !== rootOwner.sha256 ||
      controllerWitness.value.toolchain_sha256 !== toolchain.sha256 ||
      !Number.isSafeInteger(controllerWitness.value.controller_pid) ||
      controllerWitness.value.controller_pid < 2 ||
      controllerWitness.value.controller_pgid !== controllerWitness.value.controller_pid ||
      !lowerHex(controllerWitness.value.controller_process_instance_sha256, 64)
    ) {
      fail("controlled background controller witness was refused");
    }
    const controllerReady = partialPrivateArtifact(
      paths.controllerReady,
      "controlled background controller readiness",
    );
    validateControllerReady(
      controllerReady.value,
      {
        controllerConfigSha256: controllerConfig.sha256,
        controllerLaunchDecisionSha256: controllerLaunchDecision.sha256,
        controllerPgid: controllerWitness.value.controller_pgid,
        controllerPid: controllerWitness.value.controller_pid,
        controllerScriptSha256: controllerConfig.value.controller_script_sha256,
        fixtureId,
        nodeSha256: controllerConfig.value.node_sha256,
        workingDirectory: paths.root,
      },
      controllerConfig.value.controller_nonce,
    );
    if (
      controllerReady.value.controller_process_instance_sha256 !==
      controllerWitness.value.controller_process_instance_sha256
    ) {
      fail("controlled background controller readiness changed");
    }
  }
  const providerStartDecision = artifacts["provider-start-decision.json"];
  if (providerStartDecision !== undefined) {
    if (controllerWitness === undefined) {
      fail("controlled background provider start lacked a controller witness");
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
  }
  const hostagentWitness = artifacts["hostagent-witness.json"];
  if (hostagentWitness !== undefined) {
    if (
      providerStartDecision === undefined ||
      controllerWitness === undefined ||
      controllerConfig === undefined ||
      toolchain === undefined
    ) {
      fail("controlled background hostagent witness lacked start authority");
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
    const pidRecord = partialPrivateArtifact(
      paths.pidRecord,
      "controlled background hostagent PID record",
    );
    validatePidRecord(
      pidRecord.value,
      {
        controllerWitnessSha256: controllerWitness.sha256,
        controllerPid: controllerWitness.value.controller_pid,
        fixtureId,
        hostagentConfigSha256: hostagentConfig.sha256,
        instanceNonceSha256: providerProcessDigest(
          Buffer.from(createAuthority.value.ownership_nonce, "ascii"),
        ),
        pid: pidRecord.value.pid,
        processIdentity: hostagentWitness.value.process_instance_sha256,
        profile: paths.profile,
        providerStartDecisionSha256: providerStartDecision.sha256,
        workingDirectory: paths.root,
      },
      toolchain.value,
      createAuthority.value.ownership_nonce,
    );
    if (
      canonical(privateFileIdentity(
        paths.pidRecord,
        "controlled background hostagent PID record",
        relative(paths.root, paths.pidRecord),
      )) !== canonical(hostagentWitness.value.pid_record)
    ) {
      fail("controlled background hostagent PID identity changed");
    }
    if (
      pathEntryExists(paths.haSocket) &&
      canonical(
        socketIdentity(
          paths.haSocket,
          "controlled background hostagent socket",
          relative(paths.root, paths.haSocket),
        ),
      ) !== canonical(hostagentWitness.value.socket)
    ) {
      fail("controlled background hostagent socket identity changed");
    }
  }
  const engineWitness = artifacts["engine-witness.json"];
  if (engineWitness !== undefined) {
    if (hostagentWitness === undefined) {
      fail("controlled background Engine witness lacked a hostagent witness");
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
    if (
      pathEntryExists(paths.engineSocket) &&
      canonical(
        socketIdentity(
          paths.engineSocket,
          "controlled background Engine socket",
          relative(paths.root, paths.engineSocket),
        ),
      ) !== canonical(engineWitness.value.socket)
    ) {
      fail("controlled background Engine socket identity changed");
    }
  }
  const contextWitness = artifacts["context-witness.json"];
  if (contextWitness !== undefined) {
    if (engineWitness === undefined) {
      fail("controlled background context witness lacked an Engine witness");
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
    if (
      contextWitness.value.schema !== "synveda.clean-engine.background-context-witness.v1" ||
      contextWitness.value.fixture_id !== fixtureId ||
      contextWitness.value.context_name !== paths.profile ||
      contextWitness.value.endpoint !== `unix://${paths.engineSocket}` ||
      contextWitness.value.engine_socket_sha256 !==
        providerProcessDigest(providerProcessBytes(engineWitness.value.socket)) ||
      contextWitness.value.tls_material !== "absent"
    ) {
      fail("controlled background context witness was refused");
    }
    if (
      canonical(
        privateFileIdentity(
          paths.contextFile,
          "controlled background context file",
          relative(paths.root, paths.contextFile),
        ),
      ) !== canonical(contextWitness.value.context_file)
    ) {
      fail("controlled background context file identity changed");
    }
  }
  const controllerSettlement = artifacts["controller-settlement.json"];
  if (controllerSettlement !== undefined) {
    if (
      contextWitness === undefined ||
      engineWitness === undefined ||
      hostagentWitness === undefined ||
      providerStartDecision === undefined ||
      controllerWitness === undefined
    ) {
      fail("controlled background controller settlement lacked process evidence");
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
  }
  if (artifacts["provider-identity.json"] !== undefined) {
    return Object.freeze({
      evidence: validateCreationArtifactChain(artifacts, fixtureId, {
        revalidateCurrentToolchain,
      }),
      paths,
    });
  }
  return Object.freeze({
    evidence: Object.freeze({
      createAuthority,
      toolchain,
      controllerLaunchDecision,
      controllerWitness,
      providerStartDecision,
      hostagentWitness,
      engineWitness,
      contextWitness,
      controllerSettlement,
    }),
    paths,
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

export function inspectControlledBackgroundProviderPrefix(
  evidenceDirectory,
  fixtureId,
  {
    expectedCreateBindings,
    providerBase,
    revalidateCurrentToolchain = false,
  } = {},
) {
  if (
    !lowerHex(fixtureId, 32) ||
    typeof providerBase !== "string" ||
    typeof revalidateCurrentToolchain !== "boolean"
  ) {
    fail("controlled background prefix arguments were refused", 64);
  }
  if (expectedCreateBindings !== undefined) validateCreateBindings(expectedCreateBindings);
  const expectedPaths = validateControlledBackgroundRoots({
    evidenceDirectory,
    fixtureId,
    providerBase,
  });
  const prefix = inspectCreationEvidencePrefix(evidenceDirectory);
  if (
    !pathEntryExists(expectedPaths.root) &&
    Object.keys(prefix.artifacts).some(
      (name) => name !== "background-create-authority.json",
    )
  ) {
    fail("controlled background provider root disappeared after preparation");
  }
  const validated = validateCreationArtifactPrefix(prefix.artifacts, fixtureId, {
    expectedCreateBindings,
    providerBase,
    revalidateCurrentToolchain,
  });
  if (prefix.pendingPublication?.disposition === "staged-complete") {
    let pendingValue;
    try {
      pendingValue = JSON.parse(prefix.pendingArtifact.bytes.toString("utf8"));
    } catch {
      fail("controlled background complete create-prefix stage was malformed");
    }
    validateCreationArtifactPrefix(
      {
        ...prefix.artifacts,
        [prefix.pendingPublication.target_name]: Object.freeze({
          bytes: prefix.pendingArtifact.bytes,
          metadata: prefix.pendingArtifact.metadata,
          path: join(evidenceDirectory, prefix.pendingPublication.target_name),
          sha256: prefix.pendingPublication.actual_sha256,
          value: pendingValue,
        }),
      },
      fixtureId,
      { expectedCreateBindings, providerBase, revalidateCurrentToolchain },
    );
  }
  let authenticatedRoot = false;
  if (validated.paths !== undefined && pathEntryExists(expectedPaths.root)) {
    try {
      const rootMetadata = lstatSync(expectedPaths.root, { bigint: true });
      if (
        !rootMetadata.isSymbolicLink() &&
        rootMetadata.isDirectory() &&
        rootMetadata.uid === BigInt(process.getuid()) &&
        (rootMetadata.mode & 0o7777n) === 0o700n &&
        pathEntryExists(expectedPaths.ownerMarker)
      ) {
        const owner = readCanonicalArtifactOnly(
          expectedPaths.ownerMarker,
          "controlled background progressive root owner",
          new Set([1n, 2n]),
        );
        validateRootOwner(owner.value, {
          createAuthoritySha256: prefix.artifacts["background-create-authority.json"].sha256,
          fixtureId,
          paths: expectedPaths,
        });
        authenticatedRoot =
          owner.value.ownership_nonce ===
          prefix.artifacts["background-create-authority.json"].value.ownership_nonce;
      }
    } catch (error) {
      if (!(error instanceof ProviderProcessContractFailure)) throw error;
      authenticatedRoot = false;
    }
  }
  const unownedRootCollision =
    pathEntryExists(expectedPaths.root) &&
    (validated.paths === undefined || !authenticatedRoot);
  const collisionRoot = unownedRootCollision
    ? noFollowEntryIdentity(
        expectedPaths.root,
        "controlled background unowned provider collision",
      )
    : undefined;
  const residual =
    unownedRootCollision
      ? Object.freeze({
          controller_presence:
            prefix.artifacts["controller-launch-decision.json"] === undefined
              ? "not-started"
              : "unattested",
          hostagent_presence:
            prefix.artifacts["provider-start-decision.json"] === undefined
              ? "not-started"
              : "unattested",
          inventory_sha256: providerProcessDigest(providerProcessBytes(collisionRoot)),
          private_publications: Object.freeze([]),
          root: collisionRoot,
          root_disposition: "ownership-pending",
          root_inventory: Object.freeze([collisionRoot]),
          sockets: "uninspected",
          static_root_identity_sha256: providerProcessDigest(
            providerProcessBytes({ entries: [collisionRoot], root: collisionRoot }),
          ),
        })
      : validated.paths === undefined
      ? Object.freeze({
          controller_presence: "not-started",
          hostagent_presence: "not-started",
          inventory_sha256: ZERO_SHA256,
          private_publications: Object.freeze([]),
          root: undefined,
          root_disposition: "absent",
          root_inventory: Object.freeze([]),
          sockets: "absent",
          static_root_identity_sha256: ZERO_SHA256,
        })
      : inspectProgressiveRoot(
          validated.paths,
          prefix.artifacts["background-create-authority.json"],
          prefix.artifacts,
        );
  const providerIdentity = prefix.artifacts["provider-identity.json"];
  if (
    providerIdentity !== undefined &&
    validated.paths !== undefined &&
    residual.static_root_identity_sha256 !==
      staticRootIdentitySha256(
        validated.paths,
        providerIdentity.value.provider_root,
        providerIdentity.value.provider_root_inventory,
      )
  ) {
    fail("controlled background provider static root identity changed", 73);
  }
  const effectFrontier = unownedRootCollision
    ? Object.freeze({ disposition: "complete", effect: "provider-root-collision" })
    : validateProgressiveEffectPrefix(expectedPaths, prefix, residual);
  if (
    residual.root_disposition === "absent" &&
    Object.keys(prefix.artifacts).some(
      (name) => name !== "background-create-authority.json",
    ) ||
    (residual.root_disposition === "absent" &&
      prefix.pendingPublication !== undefined &&
      prefix.pendingPublication.target_name !== "background-create-authority.json")
  ) {
    fail("controlled background provider root disappeared after preparation");
  }
  const artifactNames = Object.keys(prefix.artifacts).sort(
    (left, right) =>
      CONTROLLED_BACKGROUND_CREATION_ARTIFACTS.indexOf(left) -
      CONTROLLED_BACKGROUND_CREATION_ARTIFACTS.indexOf(right),
  );
  const artifactHead = artifactNames.at(-1);
  const evidenceHeadSha256 =
    artifactHead === undefined ? ZERO_SHA256 : prefix.artifacts[artifactHead].sha256;
  const stage =
    artifactHead === undefined
      ? "empty"
      : artifactHead
          .replace(/^background-/, "")
          .replace(/\.json$/, "")
          .replace(/^provider-identity$/, "provider-identity");
  const evidencePrefix = artifactNames.map((name) => {
    const artifact = prefix.artifacts[name];
    return {
      device: String(artifact.metadata.dev),
      inode: String(artifact.metadata.ino),
      links: String(artifact.metadata.nlink),
      mode: (artifact.metadata.mode & 0o7777n).toString(8).padStart(4, "0"),
      name,
      sha256: artifact.sha256,
      size: String(artifact.metadata.size),
      uid: String(artifact.metadata.uid),
    };
  });
  const replaySafe =
    artifactNames.length === 1 &&
    artifactNames[0] === "background-create-authority.json" &&
    prefix.pendingPublication === undefined &&
    residual.root_disposition === "absent";
  const residualValue = {
    contract: "controlled-background-fake",
    create_bindings: expectedCreateBindings ?? null,
    effect_frontier: effectFrontier,
    evidence_directory: directoryIdentity(
      evidenceDirectory,
      "controlled background evidence directory",
    ),
    evidence_prefix: evidencePrefix,
    evidence_head_sha256: evidenceHeadSha256,
    evidence_stage: stage,
    fixture_id: fixtureId,
    pending_publication: prefix.pendingPublication ?? null,
    provider_base: directoryIdentity(providerBase, "controlled background provider base"),
    provider_contract_sha256: CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256,
    replay_safe: replaySafe,
    residual: {
      ...residual,
      root: residual.root ?? null,
    },
  };
  const evidencePrefixSha256 = providerProcessDigest(providerProcessBytes(evidencePrefix));
  return Object.freeze({
    artifacts: prefix.artifacts,
    contract: "controlled-background-fake",
    effectFrontier,
    evidenceHeadSha256,
    evidencePrefixSha256,
    evidenceStage: stage,
    pendingPublication: prefix.pendingPublication,
    replaySafe,
    residual,
    residualSha256: providerProcessDigest(providerProcessBytes(residualValue)),
  });
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

function progressiveRootContract(paths) {
  const staticEntries = [
    paths.ownerMarker,
    ...Object.values(CONTROLLED_BACKGROUND_ROOT_LAYOUT)
      .sort()
      .map((leaf) => join(paths.root, leaf)),
    paths.colimaProfile,
    paths.limaInstance,
    join(paths.root, CONTROLLED_BACKGROUND_ROOT_LAYOUT.DOCKER_CONFIG, "contexts"),
    join(paths.root, CONTROLLED_BACKGROUND_ROOT_LAYOUT.DOCKER_CONFIG, "contexts", "meta"),
    paths.contextDirectory,
    paths.diskImage,
    paths.contextFile,
    paths.hostagentConfig,
    paths.controllerConfig,
  ];
  const dynamicEntries = [
    paths.controllerReady,
    paths.haSocket,
    paths.engineSocket,
    paths.pidRecord,
  ];
  const kinds = new Map();
  for (const path of staticEntries) {
    kinds.set(
      relative(paths.root, path),
      new Set([
        paths.ownerMarker,
        paths.diskImage,
        paths.contextFile,
        paths.hostagentConfig,
        paths.controllerConfig,
      ]).has(path)
        ? "file"
        : "directory",
    );
  }
  kinds.set(relative(paths.root, paths.controllerReady), "file");
  kinds.set(relative(paths.root, paths.haSocket), "socket");
  kinds.set(relative(paths.root, paths.engineSocket), "socket");
  kinds.set(relative(paths.root, paths.pidRecord), "file");
  return Object.freeze({ dynamicEntries, kinds, staticEntries });
}

function staticRootIdentitySha256(paths, root, entries) {
  if (root === undefined) return ZERO_SHA256;
  const dynamicPaths = new Set(
    progressiveRootContract(paths).dynamicEntries.map((path) =>
      relative(paths.root, path),
    ),
  );
  const staticEntries = entries
    .filter(
      (entry) =>
        !dynamicPaths.has(entry.relative_path) &&
        parsePrivateStageName(basename(entry.relative_path)) === undefined,
    )
    .map((entry) =>
      entry.kind === "directory"
        ? {
            device: entry.device,
            inode: entry.inode,
            kind: entry.kind,
            mode: entry.mode,
            relative_path: entry.relative_path,
            uid: entry.uid,
          }
        : entry,
    )
    .sort((left, right) => left.relative_path.localeCompare(right.relative_path));
  return providerProcessDigest(
    providerProcessBytes({ entries: staticEntries, root }),
  );
}

function progressiveFile(path, rootPath, rootDevice, isStage) {
  const label = isStage
    ? "controlled background progressive private-file stage"
    : "controlled background progressive private file";
  const opened = isStage
    ? openedStage(path, label)
    : openedFile(path, label, MAX_ARTIFACT_BYTES, new Set([1n, 2n]), true);
  if (opened.metadata.dev !== rootDevice) {
    fail("controlled background progressive file crossed a filesystem boundary");
  }
  const sha256 = providerProcessDigest(opened.bytes);
  return Object.freeze({
    artifact: Object.freeze({
      bytes: opened.bytes,
      metadata: opened.metadata,
      path,
      sha256,
    }),
    identity: Object.freeze({
      device: String(opened.metadata.dev),
      inode: String(opened.metadata.ino),
      kind: "file",
      links: String(opened.metadata.nlink),
      mode: "0600",
      relative_path: relative(rootPath, path),
      sha256,
      size: String(opened.metadata.size),
      uid: String(opened.metadata.uid),
    }),
  });
}

function inspectProgressiveRoot(paths, createAuthority, artifacts) {
  if (!pathEntryExists(paths.root)) {
    return Object.freeze({
      controller_presence:
        artifacts["controller-launch-decision.json"] === undefined
          ? "not-started"
          : "unattested",
      hostagent_presence:
        artifacts["provider-start-decision.json"] === undefined
          ? "not-started"
          : "unattested",
      inventory_sha256: ZERO_SHA256,
      private_publications: Object.freeze([]),
      root: undefined,
      root_disposition: "absent",
      root_inventory: Object.freeze([]),
      sockets: "absent",
      static_root_identity_sha256: ZERO_SHA256,
    });
  }
  const rootMetadata = secureDirectory(paths.root, "controlled background progressive root");
  const contract = progressiveRootContract(paths);
  const entries = [];
  const stages = [];
  const walk = (directory) => {
    for (const name of readdirSync(directory).sort()) {
      const path = join(directory, name);
      const relativePath = relative(paths.root, path);
      const metadata = lstatSync(path, { bigint: true });
      const parsedStage = parsePrivateStageName(name);
      if (parsedStage !== undefined) {
        const possibleTargets = [...contract.kinds.entries()].filter(
          ([target, kind]) =>
            kind === "file" &&
            dirname(target) === dirname(relativePath) &&
            privateStageTargetSha256(join(paths.root, target)) === parsedStage.target_sha256,
        );
        if (possibleTargets.length !== 1) {
          fail("controlled background progressive stage target was refused");
        }
        const [target] = possibleTargets[0];
        const file = progressiveFile(path, paths.root, rootMetadata.dev, true);
        stages.push(Object.freeze({ ...file, parsed: parsedStage, target }));
        entries.push(file.identity);
      } else {
        const kind = contract.kinds.get(relativePath);
        if (kind === undefined) {
          fail("controlled background progressive root inventory was refused");
        }
        if (kind === "directory") {
          if (
            metadata.isSymbolicLink() ||
            !metadata.isDirectory() ||
            metadata.uid !== BigInt(process.getuid()) ||
            metadata.dev !== rootMetadata.dev ||
            (metadata.mode & 0o7777n) !== 0o700n
          ) {
            fail("controlled background progressive directory identity was refused");
          }
          entries.push({
            device: String(metadata.dev),
            inode: String(metadata.ino),
            kind: "directory",
            links: String(metadata.nlink),
            mode: "0700",
            relative_path: relativePath,
            sha256: ZERO_SHA256,
            size: String(metadata.size),
            uid: String(metadata.uid),
          });
          walk(path);
        } else if (kind === "file") {
          entries.push(progressiveFile(path, paths.root, rootMetadata.dev, false).identity);
        } else {
          if (
            metadata.isSymbolicLink() ||
            !metadata.isSocket() ||
            metadata.uid !== BigInt(process.getuid()) ||
            metadata.dev !== rootMetadata.dev ||
            metadata.nlink !== 1n ||
            (metadata.mode & 0o7777n) !== 0o600n
          ) {
            fail("controlled background progressive socket identity was refused");
          }
          entries.push({
            device: String(metadata.dev),
            inode: String(metadata.ino),
            kind: "socket",
            links: "1",
            mode: "0600",
            relative_path: relativePath,
            sha256: ZERO_SHA256,
            size: String(metadata.size),
            uid: String(metadata.uid),
          });
        }
      }
      if (entries.length > MAX_INVENTORY_ENTRIES) {
        fail("controlled background progressive inventory capacity was exceeded");
      }
    }
  };
  walk(paths.root);
  if (stages.length > 1) {
    fail("controlled background progressive stages were ambiguous");
  }
  const entriesByPath = new Map(entries.map((entry) => [entry.relative_path, entry]));
  const stagesByTarget = new Map();
  for (const candidate of stages) {
    const target = entriesByPath.get(candidate.target);
    const linked = target !== undefined && target.inode === candidate.identity.inode;
    let value;
    let canonicalComplete = false;
    try {
      value = JSON.parse(candidate.artifact.bytes.toString("utf8"));
      canonicalComplete = providerProcessBytes(value).equals(candidate.artifact.bytes);
    } catch {
      canonicalComplete = false;
    }
    if (
      (candidate.identity.links === "2" && !linked) ||
      (candidate.identity.links === "1" && linked) ||
      (target !== undefined && !linked)
    ) {
      fail("controlled background progressive stage link was refused");
    }
    if (canonicalComplete && candidate.artifact.sha256 !== candidate.parsed.value_sha256) {
      fail("controlled background progressive stage digest was refused");
    }
    if (
      (candidate.artifact.sha256 === candidate.parsed.value_sha256 && !canonicalComplete) ||
      (linked && !canonicalComplete)
    ) {
      fail("controlled background complete progressive stage was malformed");
    }
    const publication = Object.freeze({
      actual_sha256: candidate.artifact.sha256,
      declared_sha256: candidate.parsed.value_sha256,
      device: candidate.identity.device,
      disposition:
        candidate.identity.links === "2"
          ? "linked-complete"
          : canonicalComplete
            ? "staged-complete"
            : "staged-partial",
      inode: candidate.identity.inode,
      links: Number(candidate.identity.links),
      mode: candidate.identity.mode,
      name: basename(candidate.identity.relative_path),
      size: candidate.identity.size,
      target_path: candidate.target,
      uid: candidate.identity.uid,
    });
    stagesByTarget.set(
      candidate.target,
      Object.freeze({ ...candidate, canonicalComplete, publication, value }),
    );
  }
  for (const entry of entries) {
    if (entry.kind !== "file" || parsePrivateStageName(basename(entry.relative_path)) !== undefined) {
      continue;
    }
    if (entry.links === "2" && !stagesByTarget.has(entry.relative_path)) {
      fail("controlled background progressive file link was refused");
    }
  }
  const progressiveArtifact = (path, label) => {
    const relativePath = relative(paths.root, path);
    if (entriesByPath.has(relativePath)) return partialPrivateArtifact(path, label);
    const stage = stagesByTarget.get(relativePath);
    if (stage?.canonicalComplete !== true) return undefined;
    return Object.freeze({
      bytes: stage.artifact.bytes,
      metadata: stage.artifact.metadata,
      path,
      sha256: stage.artifact.sha256,
      value: stage.value,
    });
  };
  let prefixOpen = true;
  for (const path of contract.staticEntries) {
    const relativePath = relative(paths.root, path);
    const present = entriesByPath.has(relativePath);
    const staged = stagesByTarget.has(relativePath);
    if (!prefixOpen && (present || staged)) {
      fail("controlled background progressive root had a causal gap");
    }
    if (!present || staged) prefixOpen = false;
  }
  const owner = entriesByPath.get(relative(paths.root, paths.ownerMarker));
  const ownerArtifact = progressiveArtifact(
    paths.ownerMarker,
    "controlled background progressive root owner",
  );
  let rootDisposition = "ownership-pending";
  if (ownerArtifact !== undefined) {
    validateRootOwner(ownerArtifact.value, {
      createAuthoritySha256: createAuthority.sha256,
      fixtureId: createAuthority.value.fixture_id,
      paths,
    });
  }
  if (owner !== undefined) {
    rootDisposition = "owned";
  }
  const disk = progressiveArtifact(
    paths.diskImage,
    "controlled background progressive disk fixture",
  );
  if (disk !== undefined) {
    if (
      canonical(disk.value) !== canonical({
        fixture_id: createAuthority.value.fixture_id,
        payload: "non-bootable-controlled-background-disk",
        schema: "synveda.clean-engine.background-disk-fixture.v1",
      })
    ) {
      fail("controlled background progressive disk fixture was refused");
    }
  }
  const context = progressiveArtifact(
    paths.contextFile,
    "controlled background progressive context",
  );
  if (context !== undefined) {
    if (
      canonical(context.value) !== canonical({
        context_name: paths.profile,
        endpoint: `unix://${paths.engineSocket}`,
        fixture_id: createAuthority.value.fixture_id,
        schema: "synveda.clean-engine.background-docker-context.v1",
        tls_material: "absent",
      })
    ) {
      fail("controlled background progressive context was refused");
    }
  }
  const hostagentConfig = progressiveArtifact(
    paths.hostagentConfig,
    "controlled background progressive hostagent config",
  );
  if (hostagentConfig !== undefined) {
    exactKeys(
      hostagentConfig.value,
      [
        "engine_architecture",
        "engine_socket",
        "fixture_id",
        "ha_socket",
        "hostagent_script_sha256",
        "instance_nonce",
        "maximum_lifetime_milliseconds",
        "node_sha256",
        "pid_record",
        "profile",
        "schema",
        "working_directory",
      ],
      "controlled background progressive hostagent config",
    );
    if (
      hostagentConfig.value.schema !== "synveda.clean-engine.background-hostagent-config.v1" ||
      hostagentConfig.value.fixture_id !== createAuthority.value.fixture_id ||
      hostagentConfig.value.engine_architecture !==
        controlledBackgroundEngineArchitecture(process.arch) ||
      hostagentConfig.value.engine_socket !== paths.engineSocket ||
      hostagentConfig.value.ha_socket !== paths.haSocket ||
      hostagentConfig.value.instance_nonce !== createAuthority.value.ownership_nonce ||
      hostagentConfig.value.pid_record !== paths.pidRecord ||
      hostagentConfig.value.profile !== paths.profile ||
      hostagentConfig.value.working_directory !== paths.root ||
      !lowerHex(hostagentConfig.value.hostagent_script_sha256, 64) ||
      !lowerHex(hostagentConfig.value.node_sha256, 64) ||
      !Number.isSafeInteger(hostagentConfig.value.maximum_lifetime_milliseconds) ||
      hostagentConfig.value.maximum_lifetime_milliseconds < 1_000 ||
      hostagentConfig.value.maximum_lifetime_milliseconds >
        CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.max_lifetime_milliseconds
    ) {
      fail("controlled background progressive hostagent config was refused");
    }
  }
  const controllerConfig = progressiveArtifact(
    paths.controllerConfig,
    "controlled background progressive controller config",
  );
  if (controllerConfig !== undefined) {
    exactKeys(
      controllerConfig.value,
      [
        "before_detach_hold_milliseconds",
        "controller_launch_decision",
        "controller_nonce",
        "controller_ready",
        "controller_script_sha256",
        "controller_witness",
        "create_authority",
        "create_authority_sha256",
        "create_intent_sha256",
        "create_slot_sha256",
        "fixture_id",
        "hostagent_config",
        "hostagent_config_sha256",
        "hostagent_source_base64",
        "hostagent_source_sha256",
        "instance_nonce",
        "maximum_lifetime_milliseconds",
        "node_sha256",
        "provider_contract_sha256",
        "provider_start_decision",
        "require_shutdown_during_start",
        "root_owner",
        "root_owner_sha256",
        "schema",
        "working_directory",
      ],
      "controlled background progressive controller config",
    );
    if (
      controllerConfig.value.schema !== "synveda.clean-engine.background-controller-config.v2" ||
      controllerConfig.value.fixture_id !== createAuthority.value.fixture_id ||
      controllerConfig.value.create_authority_sha256 !== createAuthority.sha256 ||
      controllerConfig.value.create_intent_sha256 !==
        createAuthority.value.create_intent_sha256 ||
      controllerConfig.value.create_slot_sha256 !== createAuthority.value.create_slot_sha256 ||
      controllerConfig.value.hostagent_config !== paths.hostagentConfig ||
      controllerConfig.value.hostagent_config_sha256 !== hostagentConfig?.sha256 ||
      controllerConfig.value.maximum_lifetime_milliseconds !==
        hostagentConfig?.value.maximum_lifetime_milliseconds ||
      controllerConfig.value.controller_launch_decision !==
        join(dirname(createAuthority.path), "controller-launch-decision.json") ||
      controllerConfig.value.controller_ready !== paths.controllerReady ||
      controllerConfig.value.controller_witness !==
        join(dirname(createAuthority.path), "controller-witness.json") ||
      controllerConfig.value.create_authority !== createAuthority.path ||
      controllerConfig.value.instance_nonce !== createAuthority.value.ownership_nonce ||
      controllerConfig.value.provider_contract_sha256 !==
        CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256 ||
      controllerConfig.value.provider_start_decision !==
        join(dirname(createAuthority.path), "provider-start-decision.json") ||
      controllerConfig.value.root_owner !== paths.ownerMarker ||
      controllerConfig.value.root_owner_sha256 !== ownerArtifact?.sha256 ||
      controllerConfig.value.working_directory !== paths.root ||
      !Number.isSafeInteger(controllerConfig.value.before_detach_hold_milliseconds) ||
      controllerConfig.value.before_detach_hold_milliseconds < 0 ||
      controllerConfig.value.before_detach_hold_milliseconds > 5_000 ||
      !lowerHex(controllerConfig.value.controller_nonce, 64) ||
      !lowerHex(controllerConfig.value.controller_script_sha256, 64) ||
      !lowerHex(controllerConfig.value.hostagent_source_sha256, 64) ||
      !lowerHex(controllerConfig.value.node_sha256, 64) ||
      !Number.isSafeInteger(controllerConfig.value.maximum_lifetime_milliseconds) ||
      controllerConfig.value.maximum_lifetime_milliseconds < 1_000 ||
      controllerConfig.value.maximum_lifetime_milliseconds >
        CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.max_lifetime_milliseconds ||
      typeof controllerConfig.value.require_shutdown_during_start !== "boolean"
    ) {
      fail("controlled background progressive controller config was refused");
    }
  }
  if (hostagentConfig !== undefined || controllerConfig !== undefined) {
    const toolchain = artifacts["background-toolchain.json"];
    if (toolchain === undefined) {
      fail("controlled background progressive config lacked its toolchain");
    }
    const components = new Map(
      toolchain.value.components.map((component) => [component.role, component]),
    );
    if (
      hostagentConfig === undefined ||
      hostagentConfig.value.hostagent_script_sha256 !==
        components.get("hostagent-script")?.sha256 ||
      hostagentConfig.value.node_sha256 !== components.get("node-runtime")?.sha256
    ) {
      fail("controlled background progressive hostagent config toolchain changed");
    }
    if (controllerConfig !== undefined) {
      let hostagentSource;
      try {
        hostagentSource = Buffer.from(controllerConfig.value.hostagent_source_base64, "base64");
      } catch {
        fail("controlled background progressive hostagent source was refused");
      }
      if (
        typeof controllerConfig.value.hostagent_source_base64 !== "string" ||
        hostagentSource.toString("base64") !== controllerConfig.value.hostagent_source_base64 ||
        providerProcessDigest(hostagentSource) !==
          controllerConfig.value.hostagent_source_sha256 ||
        controllerConfig.value.hostagent_source_sha256 !==
          components.get("hostagent-script")?.sha256 ||
        controllerConfig.value.controller_script_sha256 !==
          components.get("controller-script")?.sha256 ||
        controllerConfig.value.node_sha256 !== components.get("node-runtime")?.sha256
      ) {
        fail("controlled background progressive controller config toolchain changed");
      }
    }
  }
  const root = {
    device: String(rootMetadata.dev),
    inode: String(rootMetadata.ino),
    mode: "0700",
    path: paths.root,
    uid: String(rootMetadata.uid),
  };
  const orderedEntries = entries.sort((left, right) =>
    left.relative_path.localeCompare(right.relative_path));
  const ready = progressiveArtifact(
    paths.controllerReady,
    "controlled background progressive controller readiness",
  );
  let controllerPresence =
    artifacts["controller-launch-decision.json"] === undefined
      ? "not-started"
      : "unattested";
  if (ready !== undefined) {
    if (
      controllerConfig === undefined ||
      artifacts["controller-launch-decision.json"] === undefined
    ) {
      fail("controlled background controller readiness lacked launch authority");
    }
    validateControllerReady(
      ready.value,
      {
        controllerConfigSha256: controllerConfig.sha256,
        controllerLaunchDecisionSha256:
          artifacts["controller-launch-decision.json"].sha256,
        controllerPgid: ready.value.controller_pgid,
        controllerPid: ready.value.controller_pid,
        controllerScriptSha256: controllerConfig.value.controller_script_sha256,
        fixtureId: createAuthority.value.fixture_id,
        nodeSha256: controllerConfig.value.node_sha256,
        workingDirectory: paths.root,
      },
      controllerConfig.value.controller_nonce,
    );
    // The child's fsynced readiness file is itself the durable process-group
    // identity.  Recovery must be able to prove that group absent even when
    // the parent died before publishing its later controller witness.  A
    // canonical-complete private stage is equally durable; positive or EPERM
    // probes remain conservatively blocking.
    controllerPresence = processObservation(probeProcessGroup(ready.value.controller_pid));
  }
  const pid = progressiveArtifact(
    paths.pidRecord,
    "controlled background progressive hostagent PID record",
  );
  let hostagentPresence =
    artifacts["provider-start-decision.json"] === undefined
      ? "not-started"
      : "unattested";
  if (pid !== undefined) {
    const controllerWitness = artifacts["controller-witness.json"];
    const providerStartDecision = artifacts["provider-start-decision.json"];
    const toolchain = artifacts["background-toolchain.json"];
    if (
      controllerWitness === undefined ||
      providerStartDecision === undefined ||
      toolchain === undefined
    ) {
      fail("controlled background hostagent PID record lacked start authority");
    }
    validatePidRecord(
      pid.value,
      {
        controllerWitnessSha256: controllerWitness.sha256,
        controllerPid: controllerWitness.value.controller_pid,
        fixtureId: createAuthority.value.fixture_id,
        hostagentConfigSha256: hostagentConfig.sha256,
        instanceNonceSha256: providerProcessDigest(
          Buffer.from(createAuthority.value.ownership_nonce, "ascii"),
        ),
        pid: pid.value.pid,
        processIdentity: pid.value.process_instance_sha256,
        profile: paths.profile,
        providerStartDecisionSha256: providerStartDecision.sha256,
        workingDirectory: paths.root,
      },
      toolchain.value,
      createAuthority.value.ownership_nonce,
    );
    // The fsynced PID record precedes the parent-owned host-agent witness.  It
    // therefore remains the only exact negative-probe handle after a crash in
    // that publication window.  Never infer absence from elapsed lifetime.
    hostagentPresence = processObservation(processPresence(pid.value.pid));
  }
  const socketCount = [paths.haSocket, paths.engineSocket].filter((path) =>
    entriesByPath.has(relative(paths.root, path))).length;
  const privatePublications = [...stagesByTarget.values()]
    .map((stage) => stage.publication)
    .sort((left, right) => left.target_path.localeCompare(right.target_path));
  return Object.freeze({
    controller_presence: controllerPresence,
    hostagent_presence: hostagentPresence,
    inventory_sha256: providerProcessDigest(providerProcessBytes({ entries: orderedEntries, root })),
    private_publications: Object.freeze(privatePublications),
    root: Object.freeze(root),
    root_disposition: rootDisposition,
    root_inventory: Object.freeze(orderedEntries),
    sockets: socketCount === 0 ? "absent" : socketCount === 2 ? "present" : "partial",
    static_root_identity_sha256: staticRootIdentitySha256(
      paths,
      root,
      orderedEntries,
    ),
  });
}

function validateProgressiveEffectPrefix(paths, prefix, residual) {
  if (
    Number(prefix.pendingPublication !== undefined) + residual.private_publications.length >
    1
  ) {
    fail("controlled background pending publications were ambiguous");
  }
  const finalRootEntries = new Set(
    residual.root_inventory
      .filter((entry) => parsePrivateStageName(basename(entry.relative_path)) === undefined)
      .map((entry) => entry.relative_path),
  );
  const pendingRootTargets = new Set(
    residual.private_publications.map((publication) => publication.target_path),
  );
  const evidenceStatus = (name) => {
    if (
      prefix.pendingPublication?.target_name === name &&
      prefix.pendingPublication.disposition !== "linked-complete"
    ) {
      return "pending";
    }
    return prefix.artifacts[name] === undefined ? "absent" : "complete";
  };
  const rootStatus = (path) => {
    const relativePath = relative(paths.root, path);
    if (pendingRootTargets.has(relativePath)) return "pending";
    return finalRootEntries.has(relativePath) ? "complete" : "absent";
  };
  const effects = [
    ["create-authority", evidenceStatus("background-create-authority.json")],
    ["provider-root", residual.root_disposition === "absent" ? "absent" : "complete"],
    ...progressiveRootContract(paths).staticEntries
      .slice(0, -2)
      .map((path, index) => [
        `provider-root-step-${String(index).padStart(2, "0")}`,
        rootStatus(path),
      ]),
    ["toolchain", evidenceStatus("background-toolchain.json")],
    ["hostagent-config", rootStatus(paths.hostagentConfig)],
    ["controller-config", rootStatus(paths.controllerConfig)],
    ["controller-launch-decision", evidenceStatus("controller-launch-decision.json")],
    ["controller-ready", rootStatus(paths.controllerReady)],
    ["controller-witness", evidenceStatus("controller-witness.json")],
    ["provider-start-decision", evidenceStatus("provider-start-decision.json")],
    ["hostagent-pid", rootStatus(paths.pidRecord)],
    ["hostagent-witness", evidenceStatus("hostagent-witness.json")],
    ["engine-witness", evidenceStatus("engine-witness.json")],
    ["context-witness", evidenceStatus("context-witness.json")],
    ["controller-settlement", evidenceStatus("controller-settlement.json")],
    ["provider-identity", evidenceStatus("provider-identity.json")],
  ];
  let prefixClosed = false;
  let frontier = Object.freeze({ disposition: "complete", effect: "empty" });
  for (const [name, status] of effects) {
    if (prefixClosed && status !== "absent") {
      fail(`controlled background effect ${name} had a causal gap`);
    }
    if (!prefixClosed && status === "complete") {
      frontier = Object.freeze({ disposition: "complete", effect: name });
    } else if (!prefixClosed) {
      if (status === "pending") {
        frontier = Object.freeze({ disposition: "pending", effect: name });
      }
      prefixClosed = true;
    }
  }
  const socketPresent = [paths.haSocket, paths.engineSocket].some(
    (path) => rootStatus(path) !== "absent",
  );
  if (socketPresent && evidenceStatus("provider-start-decision.json") !== "complete") {
    fail("controlled background socket state preceded start authority");
  }
  return frontier;
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

function validateStateRetirementBindings(value) {
  exactKeys(
    value,
    [
      "cleanup_intent_sha256",
      "cleanup_operation_plan_sha256",
      "cleanup_slot_sequence",
      "cleanup_slot_sha256",
      "create_close_sha256",
      "create_settlement_sha256",
      "create_slot_sha256",
      "source_head_sha256",
      "source_sequence",
    ],
    "controlled background state retirement bindings",
  );
  if (
    !lowerHex(value.cleanup_intent_sha256, 64) ||
    !lowerHex(value.cleanup_operation_plan_sha256, 64) ||
    !Number.isSafeInteger(value.cleanup_slot_sequence) ||
    value.cleanup_slot_sequence < 1 ||
    value.cleanup_slot_sequence > 63 ||
    !lowerHex(value.cleanup_slot_sha256, 64) ||
    !lowerHex(value.create_close_sha256, 64) ||
    !lowerHex(value.create_settlement_sha256, 64) ||
    !lowerHex(value.create_slot_sha256, 64) ||
    !lowerHex(value.source_head_sha256, 64) ||
    !Number.isSafeInteger(value.source_sequence) ||
    value.source_sequence < 1 ||
    value.source_sequence > 63
  ) {
    fail("controlled background state retirement bindings were refused", 64);
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
  if (evidence.createAuthority.value.state_integration !== "fixture-only") {
    fail("controlled background retirement integration was refused", 73);
  }
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

function stateRetirementPlanValue({
  bindings,
  evidence,
  fixtureId,
  inventory,
  owner,
  pidRecord,
  providerBase,
}) {
  return Object.freeze({
    base: directoryIdentity(providerBase, "controlled background provider base"),
    cleanup_intent_sha256: bindings.cleanup_intent_sha256,
    cleanup_operation_plan_sha256: bindings.cleanup_operation_plan_sha256,
    cleanup_slot_sequence: bindings.cleanup_slot_sequence,
    cleanup_slot_sha256: bindings.cleanup_slot_sha256,
    controller_pgid: evidence.controllerWitness.value.controller_pgid,
    create_close_sha256: bindings.create_close_sha256,
    create_settlement_sha256: bindings.create_settlement_sha256,
    create_slot_sha256: bindings.create_slot_sha256,
    fixture_id: fixtureId,
    hostagent_pid: pidRecord.value.pid,
    provider_contract_sha256: CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256,
    provider_identity_sha256: evidence.providerIdentity.sha256,
    provider_root_owner_sha256: owner.sha256,
    retirement_contract_sha256: CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT_SHA256,
    retirement_steps: retirementSteps(inventory),
    root: inventory.root,
    root_inventory: inventory.entries,
    schema: "synveda.clean-engine.controlled-background-provider-retirement-plan.v2",
    source_head_sha256: bindings.source_head_sha256,
    source_sequence: bindings.source_sequence,
    state_integration: "mutation-journal-v2",
  });
}

export async function planControlledBackgroundRetirementWithAuthorityGate(
  {
    bindings,
    evidenceDirectory,
    fixtureId,
    providerBase,
  },
  authorityGate,
) {
  validateStateRetirementBindings(bindings);
  const paths = validateControlledBackgroundRoots({
    evidenceDirectory,
    fixtureId,
    providerBase,
  });
  const evidence = inspectControlledBackgroundProvider(evidenceDirectory, fixtureId, {
    revalidateCurrentToolchain: false,
  });
  if (evidence.createAuthority.value.state_integration !== "mutation-journal-v2") {
    fail("controlled background state retirement integration was refused", 73);
  }
  if (bindings.create_slot_sha256 !== evidence.createAuthority.value.create_slot_sha256) {
    fail("controlled background state create slot binding was refused");
  }
  inspectStateArtifactPublication(
    evidenceDirectory,
    "provider-retirement-plan.json",
    "controlled background state retirement plan",
  );
  for (const name of readdirSync(evidenceDirectory)) {
    const targetName = parseArtifactStageName(name)?.targetName ?? name;
    if (
      /^(?:retirement-step-[0-9]{2}|provider-retirement-settlement)\.json$/.test(
        targetName,
      )
    ) {
      fail("controlled background state retirement evidence preceded its plan");
    }
  }
  const owner = readCanonicalArtifactOnly(
    paths.ownerMarker,
    "controlled background root owner",
  );
  validateRootOwner(owner.value, {
    createAuthoritySha256: evidence.createAuthority.sha256,
    fixtureId,
    paths,
  });
  if (
    owner.sha256 !== evidence.providerIdentity.value.root_owner_sha256 ||
    owner.sha256 !== evidence.controllerWitness.value.root_owner_sha256
  ) {
    fail("controlled background state root owner binding was refused");
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
    fail("controlled background state hostagent identity was unavailable", 69);
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
      fail("controlled background state running identity changed");
    }
  }
  if (
    probeProcessGroup(evidence.controllerWitness.value.controller_pgid) !==
    "absent"
  ) {
    fail("controlled background state controller identity remained uncertain", 73);
  }
  const inventory = creationInventoryForPlanning(
    paths,
    evidence.providerIdentity.value,
    presence === "absent",
  );
  const planValue = stateRetirementPlanValue({
    bindings,
    evidence,
    fixtureId,
    inventory,
    owner,
    pidRecord,
    providerBase,
  });
  const expectedPlanSha256 = providerProcessDigest(providerProcessBytes(planValue));
  const revalidate = (expectedPublication) => () => {
    const currentEvidence = inspectControlledBackgroundProvider(
      evidenceDirectory,
      fixtureId,
      { revalidateCurrentToolchain: false },
    );
    if (currentEvidence.createAuthority.value.state_integration !== "mutation-journal-v2") {
      fail("controlled background state create evidence changed", 73);
    }
    for (const name of Object.keys(evidence)) {
      if (!sameArtifactIdentity(currentEvidence[name], evidence[name])) {
        fail("controlled background state create evidence changed", 73);
      }
    }
    const currentOwner = readCanonicalArtifactOnly(
      paths.ownerMarker,
      "controlled background root owner",
    );
    validateRootOwner(currentOwner.value, {
      createAuthoritySha256: evidence.createAuthority.sha256,
      fixtureId,
      paths,
    });
    if (!sameArtifactIdentity(currentOwner, owner)) {
      fail("controlled background state root owner changed", 73);
    }
    if (
      canonical(
        directoryIdentity(providerBase, "controlled background provider base"),
      ) !== canonical(planValue.base)
    ) {
      fail("controlled background state provider base changed", 73);
    }
    const currentPidRecord = revalidateHostagentPidRecord(
      paths,
      fixtureId,
      currentEvidence,
      instanceNonce,
    );
    const currentPresence = processPresence(currentPidRecord.value.pid);
    if (
      currentPidRecord.value.pid !== pidRecord.value.pid ||
      currentPresence === "unknown"
    ) {
      fail("controlled background state hostagent identity was unavailable", 69);
    }
    const currentInventory = creationInventoryForPlanning(
      paths,
      currentEvidence.providerIdentity.value,
      currentPresence === "absent",
    );
    if (canonical(currentInventory) !== canonical(inventory)) {
      fail("controlled background state retirement inventory changed", 73);
    }
    if (
      probeProcessGroup(currentEvidence.controllerWitness.value.controller_pgid) !==
      "absent"
    ) {
      fail("controlled background state controller identity remained uncertain", 73);
    }
    const currentPublication = inspectStateArtifactPublication(
      evidenceDirectory,
      "provider-retirement-plan.json",
      "controlled background state retirement plan",
    );
    if (!sameStatePublication(currentPublication, expectedPublication)) {
      fail("controlled background state retirement plan publication changed", 73);
    }
  };
  const plan = publishStateRetirementArtifact(
    evidenceDirectory,
    "provider-retirement-plan.json",
    planValue,
    (publication, phase) => {
      invokeRetirementAuthorityGate(
        authorityGate,
        Object.freeze({
          checkpoint: "before-retirement-plan-publication",
          ...stateRetirementAuthorityBindingFields(bindings),
          cleanup_plan_sha256: expectedPlanSha256,
          completed_steps: 0,
          next_action: "publish-retirement-plan",
          next_resources: Object.freeze(["provider-retirement-plan.json"]),
          provider_identity_sha256: evidence.providerIdentity.sha256,
          ...statePublicationAuthorityFields(
            publication,
            phase,
            "provider-retirement-plan.json",
            expectedPlanSha256,
          ),
          resource_identity_sha256: expectedPlanSha256,
          retirement_contract_sha256:
            CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT_SHA256,
        }),
        revalidate(publication),
      );
    },
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

function validateStateRetirementPlan(value, evidence, paths, planSha256) {
  exactKeys(
    value,
    [
      "base",
      "cleanup_intent_sha256",
      "cleanup_operation_plan_sha256",
      "cleanup_slot_sequence",
      "cleanup_slot_sha256",
      "controller_pgid",
      "create_close_sha256",
      "create_settlement_sha256",
      "create_slot_sha256",
      "fixture_id",
      "hostagent_pid",
      "provider_contract_sha256",
      "provider_identity_sha256",
      "provider_root_owner_sha256",
      "retirement_contract_sha256",
      "retirement_steps",
      "root",
      "root_inventory",
      "schema",
      "source_head_sha256",
      "source_sequence",
      "state_integration",
    ],
    "controlled background state retirement plan",
  );
  validateDirectoryIdentity(value.base, "controlled background state retirement base");
  validateDirectoryIdentity(value.root, "controlled background state retirement root");
  validateStateRetirementBindings({
    cleanup_intent_sha256: value.cleanup_intent_sha256,
    cleanup_operation_plan_sha256: value.cleanup_operation_plan_sha256,
    cleanup_slot_sequence: value.cleanup_slot_sequence,
    cleanup_slot_sha256: value.cleanup_slot_sha256,
    create_close_sha256: value.create_close_sha256,
    create_settlement_sha256: value.create_settlement_sha256,
    create_slot_sha256: value.create_slot_sha256,
    source_head_sha256: value.source_head_sha256,
    source_sequence: value.source_sequence,
  });
  if (
    value.schema !==
      "synveda.clean-engine.controlled-background-provider-retirement-plan.v2" ||
    value.fixture_id !== evidence.providerIdentity.value.fixture_id ||
    value.provider_contract_sha256 !== CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256 ||
    value.provider_identity_sha256 !== evidence.providerIdentity.sha256 ||
    value.provider_root_owner_sha256 !== evidence.providerIdentity.value.root_owner_sha256 ||
    value.retirement_contract_sha256 !==
      CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT_SHA256 ||
    value.root.path !== paths.root ||
    value.base.path !== paths.base ||
    value.state_integration !== "mutation-journal-v2" ||
    evidence.createAuthority.value.state_integration !== "mutation-journal-v2" ||
    value.create_slot_sha256 !== evidence.createAuthority.value.create_slot_sha256 ||
    value.controller_pgid !== evidence.controllerWitness.value.controller_pgid ||
    !Number.isSafeInteger(value.hostagent_pid) ||
    value.hostagent_pid < 2 ||
    providerProcessDigest(
      providerProcessBytes({
        fixture_id: value.fixture_id,
        pid: value.hostagent_pid,
        process_instance_sha256:
          evidence.hostagentWitness.value.process_instance_sha256,
        profile: paths.profile,
        schema: "synveda.clean-engine.background-hostagent-probe.v1",
      }),
    ) !== evidence.controllerSettlement.value.hostagent_after_controller_sha256 ||
    !lowerHex(planSha256, 64) ||
    !Array.isArray(value.root_inventory) ||
    !Array.isArray(value.retirement_steps) ||
    canonical(value.root) !== canonical(evidence.providerIdentity.value.provider_root) ||
    canonical(value.root_inventory) !==
      canonical(evidence.providerIdentity.value.provider_root_inventory)
  ) {
    fail("controlled background state retirement plan was refused");
  }
  for (const entry of value.root_inventory) {
    if (!new Set(["directory", "file", "socket"]).has(entry.kind)) {
      fail("controlled background state retirement inventory kind was refused");
    }
    validateResourceIdentity(
      entry,
      entry.kind,
      "controlled background state retirement inventory",
    );
  }
  if (
    new Set(value.root_inventory.map((entry) => entry.relative_path)).size !==
      value.root_inventory.length ||
    canonical(value.root_inventory.map((entry) => entry.relative_path).sort()) !==
      canonical(expectedRootPaths(paths)) ||
    canonical(value.retirement_steps) !==
      canonical(retirementSteps({ entries: value.root_inventory }))
  ) {
    fail("controlled background state retirement order was refused");
  }
}

function retirementPlanVersion(plan) {
  if (
    plan.schema ===
      "synveda.clean-engine.controlled-background-provider-retirement-plan.v1"
  ) {
    return 1;
  }
  if (
    plan.schema ===
      "synveda.clean-engine.controlled-background-provider-retirement-plan.v2"
  ) {
    return 2;
  }
  fail("controlled background retirement plan version was refused");
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
  assertNoSymlinkComponents(paths.base, "controlled background provider base");
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
  assertNoSymlinkComponents(paths.root, "controlled background provider root");
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

function validateRetirementProgressArtifact(
  artifact,
  planArtifact,
  sequence,
  previousSha256,
) {
  const planVersion = retirementPlanVersion(planArtifact.value);
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
    artifact.value.schema !==
      `synveda.clean-engine.provider-retirement-step.v${planVersion}` ||
    artifact.value.fixture_id !== planArtifact.value.fixture_id ||
    artifact.value.plan_sha256 !== planArtifact.sha256 ||
    artifact.value.previous_sha256 !== previousSha256 ||
    artifact.value.sequence !== sequence ||
    artifact.value.action !== step.action ||
    canonical(artifact.value.resources) !== canonical(step.resources) ||
    artifact.value.resource_identity_sha256 !==
      stepIdentitySha256(planArtifact.value, step) ||
    typeof artifact.value.recovered_absence !== "boolean"
  ) {
    fail("controlled background retirement progress was refused");
  }
}

function readProgress(evidenceDirectory, planArtifact) {
  retirementPlanVersion(planArtifact.value);
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
    validateRetirementProgressArtifact(
      artifact,
      planArtifact,
      sequence,
      previousSha256,
    );
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
  const planVersion = retirementPlanVersion(planArtifact.value);
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
    schema: `synveda.clean-engine.provider-retirement-step.v${planVersion}`,
    sequence,
  };
}

function progressName(progress) {
  return `retirement-step-${String(progress.length).padStart(2, "0")}.json`;
}

function progressStages(evidenceDirectory, progress) {
  return artifactStages(evidenceDirectory, progressName(progress));
}

function canonicalStageArtifact(stage, label) {
  let value;
  try {
    value = JSON.parse(stage.bytes.toString("utf8"));
  } catch {
    fail(`${label} was not canonical JSON`);
  }
  if (
    !providerProcessBytes(value).equals(stage.bytes) ||
    providerProcessDigest(stage.bytes) !== stage.sha256
  ) {
    fail(`${label} was not canonical JSON`);
  }
  return Object.freeze({
    bytes: stage.bytes,
    metadata: stage.metadata,
    path: stage.path,
    sha256: stage.sha256,
    value,
  });
}

function statePublicationStageSnapshot(stage, targetName) {
  const actualSha256 = providerProcessDigest(stage.bytes);
  let canonicalComplete = false;
  try {
    const value = JSON.parse(stage.bytes.toString("utf8"));
    canonicalComplete = providerProcessBytes(value).equals(stage.bytes);
  } catch {
    canonicalComplete = false;
  }
  const value = {
    actual_sha256: actualSha256,
    canonical_complete: canonicalComplete,
    declared_sha256: stage.sha256,
    device: String(stage.metadata.dev),
    inode: String(stage.metadata.ino),
    links: String(stage.metadata.nlink),
    mode: (stage.metadata.mode & 0o7777n).toString(8).padStart(4, "0"),
    name: stage.name,
    size: String(stage.metadata.size),
    target_name: targetName,
    uid: String(stage.metadata.uid),
  };
  return Object.freeze({
    ...value,
    identity_sha256: providerProcessDigest(providerProcessBytes(value)),
  });
}

function inspectStateArtifactPublication(evidenceDirectory, targetName, label) {
  const stages = artifactStages(evidenceDirectory, targetName);
  if (stages.length > 1) {
    fail(`${label} publication stages were ambiguous`);
  }
  const stage = stages[0];
  const stageSnapshot =
    stage === undefined
      ? undefined
      : statePublicationStageSnapshot(stage, targetName);
  const finalPath = join(evidenceDirectory, targetName);
  const finalExists = pathEntryExists(finalPath);
  let artifact;
  if (finalExists) {
    artifact = readCanonicalArtifactOnly(
      finalPath,
      label,
      new Set([stage?.metadata.nlink === 2n ? 2n : 1n]),
    );
  }
  if (stage !== undefined && stage.metadata.nlink === 2n) {
    if (
      artifact === undefined ||
      artifact.metadata.dev !== stage.metadata.dev ||
      artifact.metadata.ino !== stage.metadata.ino ||
      !artifact.bytes.equals(stage.bytes) ||
      stageSnapshot.canonical_complete !== true ||
      stageSnapshot.actual_sha256 !== stageSnapshot.declared_sha256
    ) {
      fail(`${label} publication link was refused`);
    }
  }
  if (
    stage !== undefined &&
    stage.metadata.nlink === 1n &&
    artifact !== undefined &&
    artifact.metadata.dev === stage.metadata.dev &&
    artifact.metadata.ino === stage.metadata.ino
  ) {
    fail(`${label} publication stage was refused`);
  }
  const disposition =
    stage === undefined
      ? artifact === undefined
        ? "absent"
        : "final"
      : stage.metadata.nlink === 2n
        ? "linked-complete"
        : artifact === undefined
          ? stageSnapshot.canonical_complete
            ? "staged-complete"
            : "staged-partial"
          : stageSnapshot.canonical_complete
            ? "redundant-complete"
            : "redundant-partial";
  let stagedArtifact;
  if (stageSnapshot?.canonical_complete === true) {
    stagedArtifact = canonicalStageArtifact(stage, `${label} stage`);
  }
  return Object.freeze({
    artifact,
    disposition,
    rawStage: stage,
    stage: stagedArtifact,
    stageSnapshot,
  });
}

function sameStatePublication(left, right) {
  const sameOptionalArtifact =
    (left.artifact === undefined && right.artifact === undefined) ||
    sameArtifactIdentity(left.artifact, right.artifact);
  return (
    left.disposition === right.disposition &&
    sameOptionalArtifact &&
    canonical(left.stageSnapshot ?? null) ===
      canonical(right.stageSnapshot ?? null) &&
    (left.rawStage === undefined) === (right.rawStage === undefined) &&
    (left.rawStage === undefined ||
      (left.rawStage.bytes.equals(right.rawStage.bytes) &&
        sameMetadata(left.rawStage.metadata, right.rawStage.metadata)))
  );
}

function assertStatePublicationExpected(publication, expectedBytes, label) {
  const expectedSha256 = providerProcessDigest(expectedBytes);
  if (
    publication.artifact !== undefined &&
    !publication.artifact.bytes.equals(expectedBytes)
  ) {
    fail(`${label} changed`);
  }
  if (
    publication.stageSnapshot?.declared_sha256 !== undefined &&
    publication.stageSnapshot.declared_sha256 !== expectedSha256
  ) {
    fail(`${label} stage digest was refused`);
  }
  if (
    publication.stage !== undefined &&
    !publication.stage.bytes.equals(expectedBytes)
  ) {
    fail(`${label} stage changed`);
  }
  return expectedSha256;
}

function statePublicationAuthorityFields(
  publication,
  phase,
  targetName = "not-applicable",
  expectedSha256 = ZERO_SHA256,
) {
  return Object.freeze({
    publication_disposition: publication.disposition,
    publication_expected_sha256: expectedSha256,
    publication_phase: phase,
    publication_stage_declared_sha256:
      publication.stageSnapshot?.declared_sha256 ?? ZERO_SHA256,
    publication_stage_identity_sha256:
      publication.stageSnapshot?.identity_sha256 ?? ZERO_SHA256,
    publication_stage_sha256:
      publication.stageSnapshot?.actual_sha256 ?? ZERO_SHA256,
    publication_target_name: targetName,
  });
}

function stateRetirementAuthorityBindingFields(value) {
  return Object.freeze({
    cleanup_intent_sha256: value.cleanup_intent_sha256,
    cleanup_operation_plan_sha256: value.cleanup_operation_plan_sha256,
    cleanup_slot_sequence: value.cleanup_slot_sequence,
    cleanup_slot_sha256: value.cleanup_slot_sha256,
    create_close_sha256: value.create_close_sha256,
    create_settlement_sha256: value.create_settlement_sha256,
    create_slot_sha256: value.create_slot_sha256,
    operation_kind: CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND,
    source_head_sha256: value.source_head_sha256,
    source_sequence: value.source_sequence,
  });
}

function publishStateRetirementArtifact(
  evidenceDirectory,
  targetName,
  value,
  authorizeMutation,
) {
  if (
    typeof authorizeMutation !== "function" ||
    (targetName !== "provider-retirement-plan.json" &&
      targetName !== "provider-retirement-settlement.json" &&
      !/^retirement-step-[0-9]{2}\.json$/.test(targetName))
  ) {
    fail("controlled background state publication authority was refused", 70);
  }
  const expectedBytes = providerProcessBytes(value);
  const expectedSha256 = providerProcessDigest(expectedBytes);
  for (let attempt = 0; attempt < 8; attempt += 1) {
    const observed = inspectStateArtifactPublication(
      evidenceDirectory,
      targetName,
      `controlled background state ${targetName}`,
    );
    if (observed.artifact !== undefined && !observed.artifact.bytes.equals(expectedBytes)) {
      fail(`controlled background state ${targetName} changed`);
    }
    if (observed.stageSnapshot?.declared_sha256 !== undefined &&
      observed.stageSnapshot.declared_sha256 !== expectedSha256) {
      fail(`controlled background state ${targetName} stage digest was refused`);
    }
    if (observed.disposition === "final") {
      const result = authorizeMutation(observed, "before-final-consumption");
      if (result !== undefined) {
        fail("controlled background state publication authority returned a value", 70);
      }
      const verified = inspectStateArtifactPublication(
        evidenceDirectory,
        targetName,
        `controlled background state ${targetName}`,
      );
      if (!sameStatePublication(observed, verified)) {
        fail(`controlled background state ${targetName} publication changed`, 73);
      }
      return observed.artifact;
    }
    if (observed.disposition === "absent") {
      const result = authorizeMutation(observed, "before-stage-write");
      if (result !== undefined) {
        fail("controlled background state publication authority returned a value", 70);
      }
      const verified = inspectStateArtifactPublication(
        evidenceDirectory,
        targetName,
        `controlled background state ${targetName}`,
      );
      if (!sameStatePublication(observed, verified)) {
        fail(`controlled background state ${targetName} publication changed`, 73);
      }
      const stagePath = join(
        evidenceDirectory,
        artifactStageName(targetName, expectedSha256),
      );
      writeExclusive(stagePath, expectedBytes);
      syncDirectory(evidenceDirectory);
      continue;
    }
    if (
      observed.disposition === "staged-partial" ||
      observed.disposition === "redundant-partial"
    ) {
      const result = authorizeMutation(observed, "before-partial-stage-removal");
      if (result !== undefined) {
        fail("controlled background state publication authority returned a value", 70);
      }
      const verified = inspectStateArtifactPublication(
        evidenceDirectory,
        targetName,
        `controlled background state ${targetName}`,
      );
      if (!sameStatePublication(observed, verified)) {
        fail(`controlled background state ${targetName} publication changed`, 73);
      }
      removeExactStage(evidenceDirectory, observed.rawStage);
      continue;
    }
    if (observed.disposition === "staged-complete") {
      if (!observed.stage.bytes.equals(expectedBytes)) {
        fail(`controlled background state ${targetName} stage changed`);
      }
      const result = authorizeMutation(observed, "before-final-link");
      if (result !== undefined) {
        fail("controlled background state publication authority returned a value", 70);
      }
      const verified = inspectStateArtifactPublication(
        evidenceDirectory,
        targetName,
        `controlled background state ${targetName}`,
      );
      if (!sameStatePublication(observed, verified)) {
        fail(`controlled background state ${targetName} publication changed`, 73);
      }
      try {
        linkSync(observed.rawStage.path, join(evidenceDirectory, targetName));
        syncDirectory(evidenceDirectory);
      } catch (error) {
        if (error?.code !== "EEXIST") {
          fail(`controlled background state ${targetName} publication failed`, 70);
        }
      }
      continue;
    }
    if (
      observed.disposition === "linked-complete" ||
      observed.disposition === "redundant-complete"
    ) {
      if (!observed.stage.bytes.equals(expectedBytes)) {
        fail(`controlled background state ${targetName} stage changed`);
      }
      const result = authorizeMutation(observed, "before-stage-removal");
      if (result !== undefined) {
        fail("controlled background state publication authority returned a value", 70);
      }
      const verified = inspectStateArtifactPublication(
        evidenceDirectory,
        targetName,
        `controlled background state ${targetName}`,
      );
      if (!sameStatePublication(observed, verified)) {
        fail(`controlled background state ${targetName} publication changed`, 73);
      }
      removeExactStage(evidenceDirectory, observed.rawStage);
      continue;
    }
    fail(`controlled background state ${targetName} publication was refused`);
  }
  fail(`controlled background state ${targetName} publication did not converge`, 75);
}

function inspectStateRetirementProgress(evidenceDirectory, planArtifact) {
  validateEvidenceDirectoryInventory(evidenceDirectory);
  const progressStagesByName = readdirSync(evidenceDirectory)
    .map((name) => ({ name, parsed: parseArtifactStageName(name) }))
    .filter(({ parsed }) => parsed?.targetName.startsWith("retirement-step-"));
  if (progressStagesByName.length > 1) {
    fail("controlled background state retirement progress stages were ambiguous");
  }
  for (const { parsed } of progressStagesByName) {
    const sequence = Number.parseInt(
      parsed.targetName.slice("retirement-step-".length, -5),
      10,
    );
    if (
      !Number.isSafeInteger(sequence) ||
      sequence < 0 ||
      sequence >= planArtifact.value.retirement_steps.length
    ) {
      fail("controlled background state retirement progress stage exceeded its plan");
    }
  }
  const names = readdirSync(evidenceDirectory)
    .filter((name) => /^retirement-step-[0-9]{2}\.json$/.test(name))
    .sort();
  if (names.length > planArtifact.value.retirement_steps.length) {
    fail("controlled background state retirement progress overflowed");
  }
  const progress = [];
  let previousSha256 = planArtifact.sha256;
  let pending;
  for (const [sequence, name] of names.entries()) {
    if (name !== `retirement-step-${String(sequence).padStart(2, "0")}.json`) {
      fail("controlled background state retirement progress was not contiguous");
    }
    const publication = inspectStateArtifactPublication(
      evidenceDirectory,
      name,
      "controlled background state retirement progress",
    );
    if (publication.artifact === undefined) {
      fail("controlled background state retirement progress was unavailable", 69);
    }
    validateRetirementProgressArtifact(
      publication.artifact,
      planArtifact,
      sequence,
      previousSha256,
    );
    assertStatePublicationExpected(
      publication,
      providerProcessBytes(
        progressValue(
          planArtifact,
          progress,
          publication.artifact.value.recovered_absence,
        ),
      ),
      "controlled background state retirement progress",
    );
    if (publication.disposition !== "final") {
      if (sequence !== names.length - 1) {
        fail("controlled background state retirement progress stage was not terminal");
      }
      pending = Object.freeze({
        artifact: publication.artifact,
        publication,
        recoveredAbsence: publication.artifact.value.recovered_absence,
        sequence,
      });
      break;
    }
    progress.push(publication.artifact);
    previousSha256 = publication.artifact.sha256;
  }
  const stageEntry = progressStagesByName[0];
  if (stageEntry !== undefined && pending === undefined) {
    const expectedName = progressName(progress);
    if (stageEntry.parsed.targetName !== expectedName) {
      fail("controlled background state retirement progress stage was not the next slot");
    }
    const publication = inspectStateArtifactPublication(
      evidenceDirectory,
      expectedName,
      "controlled background state retirement progress",
    );
    if (publication.rawStage === undefined) {
      fail("controlled background state retirement progress stage was unavailable", 69);
    }
    const stagedArtifact = publication.stage;
    if (stagedArtifact !== undefined) {
      validateRetirementProgressArtifact(
        stagedArtifact,
        planArtifact,
        progress.length,
        previousSha256,
      );
    }
    let recoveredAbsence = stagedArtifact?.value.recovered_absence ?? null;
    if (stagedArtifact === undefined) {
      const candidateByDigest = new Map(
        [false, true].map((candidate) => {
          const candidateBytes = providerProcessBytes(
            progressValue(planArtifact, progress, candidate),
          );
          return [providerProcessDigest(candidateBytes), candidate];
        }),
      );
      recoveredAbsence =
        candidateByDigest.get(publication.stageSnapshot.declared_sha256) ?? null;
    }
    if (typeof recoveredAbsence !== "boolean") {
      fail("controlled background state retirement progress stage was refused");
    }
    assertStatePublicationExpected(
      publication,
      providerProcessBytes(
        progressValue(planArtifact, progress, recoveredAbsence),
      ),
      "controlled background state retirement progress",
    );
    pending = Object.freeze({
      artifact: stagedArtifact,
      publication,
      recoveredAbsence,
      sequence: progress.length,
    });
  }
  return Object.freeze({ pending, progress: Object.freeze(progress) });
}

function sameStateRetirementProgress(left, right) {
  return (
    left.progress.length === right.progress.length &&
    left.progress.every((artifact, index) =>
      sameArtifactIdentity(artifact, right.progress[index])) &&
    (left.pending === undefined) === (right.pending === undefined) &&
    (left.pending === undefined ||
      (left.pending.sequence === right.pending.sequence &&
        left.pending.recoveredAbsence === right.pending.recoveredAbsence &&
        sameStatePublication(
          left.pending.publication,
          right.pending.publication,
        )))
  );
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

async function stopHostagent(
  paths,
  planArtifact,
  evidence,
  instanceNonce,
  authorizeEffect,
) {
  const planVersion = retirementPlanVersion(planArtifact.value);
  if (
    (authorizeEffect !== undefined && typeof authorizeEffect !== "function") ||
    (planVersion === 2 && typeof authorizeEffect !== "function")
  ) {
    fail("controlled background stop authority was refused", 70);
  }
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
      authorizeEffect?.({
        additionallyCompleted: new Set(completed),
        checkpoint: "before-stale-socket-unlink",
        resource,
        step: socketStep,
      });
      const current = inventoryIdentity(
        path,
        paths.root,
        BigInt(planArtifact.value.root.device),
      );
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
  const challenge = providerProcessDigest(
    providerProcessBytes({
      cleanup_plan_sha256:
        planVersion === 2
          ? planArtifact.sha256
          : ZERO_SHA256,
      nonce: randomBytes(32).toString("hex"),
      schema: "synveda.clean-engine.background-shutdown-challenge.v1",
    }),
  );
  const response = await requestSocket(paths.haSocket, {
    action: "shutdown",
    challenge,
    proof_sha256: proof(instanceNonce, "hostagent-shutdown", challenge, processIdentity),
  }, 2_000, () => {
    authorizeEffect?.({
      additionallyCompleted: new Set(),
      checkpoint: "before-hostagent-shutdown-delivery",
      resource: socketStep.resources[0],
      step: socketStep,
    });
    const currentPidRecord = revalidateHostagentPidRecord(
      paths,
      planArtifact.value.fixture_id,
      evidence,
      instanceNonce,
    );
    if (
      currentPidRecord.value.pid !== pid ||
      processPresence(pid) !== "present"
    ) {
      fail("controlled background hostagent identity changed", 73);
    }
    assertRootSubset(paths, planArtifact.value, 0);
    const plannedByPath = new Map(
      planArtifact.value.root_inventory.map((entry) => [entry.relative_path, entry]),
    );
    for (const resource of socketStep.resources) {
      const planned = plannedByPath.get(resource);
      const current = inventoryIdentity(
        join(paths.root, resource),
        paths.root,
        BigInt(planArtifact.value.root.device),
      );
      if (planned === undefined || !exactProgressiveIdentity(current, planned)) {
        fail("controlled background shutdown socket identity changed", 73);
      }
    }
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

function deleteStep(paths, plan, step, authorizeEffect) {
  if (step.resources.length !== 1) fail("controlled background deletion step was refused");
  const planVersion = retirementPlanVersion(plan);
  if (
    (authorizeEffect !== undefined && typeof authorizeEffect !== "function") ||
    (planVersion === 2 && typeof authorizeEffect !== "function")
  ) {
    fail("controlled background deletion authority was refused", 70);
  }
  const resource = step.resources[0];
  const path = resource === "." ? paths.root : join(paths.root, resource);
  authorizeEffect?.({
    additionallyCompleted: new Set(),
    checkpoint:
      step.action === "unlink" || step.action === "unlink-owner"
        ? "before-resource-unlink"
        : "before-resource-rmdir",
    resource,
    step,
  });
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

function validateStateRetirementSettlement(
  value,
  planArtifact,
  evidence,
  progress,
) {
  exactKeys(
    value,
    [
      "cleanup_operation_plan_sha256",
      "cleanup_plan_sha256",
      "cleanup_slot_sha256",
      "create_close_sha256",
      "create_settlement_sha256",
      "final_progress_sha256",
      "fixture_id",
      "outcome",
      "provider_identity_sha256",
      "provider_kind",
      "resources",
      "result_receipt_authorized",
      "retirement_contract_sha256",
      "root_disposition",
      "safe_code",
      "schema",
      "source_closure",
      "state_integration",
    ],
    "controlled background state retirement settlement",
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
    "controlled background state retired resources",
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
      "synveda.clean-engine.controlled-background-provider-retirement-settlement.v2" ||
    value.fixture_id !== planArtifact.value.fixture_id ||
    value.provider_kind !== "controlled-background-fake" ||
    value.cleanup_operation_plan_sha256 !==
      planArtifact.value.cleanup_operation_plan_sha256 ||
    value.cleanup_plan_sha256 !== planArtifact.sha256 ||
    value.cleanup_slot_sha256 !== planArtifact.value.cleanup_slot_sha256 ||
    value.create_close_sha256 !== planArtifact.value.create_close_sha256 ||
    value.create_settlement_sha256 !== planArtifact.value.create_settlement_sha256 ||
    value.provider_identity_sha256 !== evidence.providerIdentity.sha256 ||
    value.retirement_contract_sha256 !==
      CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT_SHA256 ||
    progress.length !== planArtifact.value.retirement_steps.length ||
    value.final_progress_sha256 !== progress.at(-1)?.sha256 ||
    value.outcome !== "passed" ||
    value.root_disposition !== "retired" ||
    value.safe_code !== "none" ||
    value.result_receipt_authorized !== false ||
    value.source_closure !== "state-authority-reasserted" ||
    value.state_integration !== "mutation-journal-v2" ||
    canonical(value.resources) !== canonical(retiredResources)
  ) {
    fail("controlled background state retirement settlement was refused");
  }
}

function stateRetirementSettlementValue(planArtifact, evidence, progress) {
  return Object.freeze({
    cleanup_operation_plan_sha256:
      planArtifact.value.cleanup_operation_plan_sha256,
    cleanup_plan_sha256: planArtifact.sha256,
    cleanup_slot_sha256: planArtifact.value.cleanup_slot_sha256,
    create_close_sha256: planArtifact.value.create_close_sha256,
    create_settlement_sha256: planArtifact.value.create_settlement_sha256,
    final_progress_sha256: progress.at(-1)?.sha256,
    fixture_id: planArtifact.value.fixture_id,
    outcome: "passed",
    provider_identity_sha256: evidence.providerIdentity.sha256,
    provider_kind: "controlled-background-fake",
    resources: Object.freeze({
      docker_context: "retired",
      engine: "retired",
      engine_socket: "retired",
      hostagent: "retired",
      hostagent_socket: "retired",
      provider_root: "retired",
    }),
    result_receipt_authorized: false,
    retirement_contract_sha256: CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT_SHA256,
    root_disposition: "retired",
    safe_code: "none",
    schema:
      "synveda.clean-engine.controlled-background-provider-retirement-settlement.v2",
    source_closure: "state-authority-reasserted",
    state_integration: "mutation-journal-v2",
  });
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
  if (evidence.createAuthority.value.state_integration !== "fixture-only") {
    fail("controlled background retirement integration was refused", 73);
  }
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

function sameArtifactIdentity(left, right) {
  return (
    left !== undefined &&
    right !== undefined &&
    left.bytes.equals(right.bytes) &&
    sameMetadata(left.metadata, right.metadata)
  );
}

function stateRetirementResourceIdentitySha256(plan, resources) {
  const byPath = new Map(
    plan.root_inventory.map((entry) => [entry.relative_path, entry]),
  );
  const identities = resources.map((resource) => {
    if (resource === ".") return plan.root;
    const identity = byPath.get(resource);
    if (identity === undefined) {
      fail("controlled background state retirement resource was refused");
    }
    return identity;
  });
  return providerProcessDigest(providerProcessBytes(identities));
}

function observeStateRetirementProcesses(
  evidence,
  plan,
  {
    hostagentRetirementRecorded = false,
    requireHostagentAbsent = false,
  } = {},
) {
  let hostagentPresence = "retired-by-authorized-progress";
  if (!hostagentRetirementRecorded) {
    const currentHostagentPresence = processPresence(plan.hostagent_pid);
    if (
      currentHostagentPresence === "unknown" ||
      (requireHostagentAbsent && currentHostagentPresence !== "absent")
    ) {
      fail("controlled background state hostagent identity remained uncertain", 73);
    }
    hostagentPresence = processObservation(currentHostagentPresence);
  }
  return Object.freeze({
    controller_pgid: plan.controller_pgid,
    controller_presence: "retired-before-plan-publication",
    controller_process_instance_sha256:
      evidence.controllerWitness.value.controller_process_instance_sha256,
    hostagent_pid: plan.hostagent_pid,
    hostagent_presence: hostagentPresence,
    hostagent_process_instance_sha256:
      evidence.hostagentWitness.value.process_instance_sha256,
  });
}

function readStateRetirementPlan(evidenceDirectory, evidence, paths) {
  const publication = inspectStateArtifactPublication(
    evidenceDirectory,
    "provider-retirement-plan.json",
    "controlled background state retirement plan",
  );
  if (publication.disposition !== "final" || publication.artifact === undefined) {
    fail("controlled background state retirement plan publication was incomplete", 69);
  }
  validateStateRetirementPlan(
    publication.artifact.value,
    evidence,
    paths,
    publication.artifact.sha256,
  );
  return publication.artifact;
}

function revalidateStateRetirementExecution({
  additionallyCompleted = new Set(),
  evidence,
  evidenceDirectory,
  expectedPendingPublication,
  expectedProgress,
  fixtureId,
  planArtifact,
  providerBase,
  requireHostagentAbsent = false,
  subsetCompletedCount,
}) {
  const paths = validateControlledBackgroundRoots({
    evidenceDirectory,
    fixtureId,
    providerBase,
  });
  const currentEvidence = inspectControlledBackgroundProvider(
    evidenceDirectory,
    fixtureId,
    { revalidateCurrentToolchain: false },
  );
  for (const name of Object.keys(evidence)) {
    if (!sameArtifactIdentity(currentEvidence[name], evidence[name])) {
      fail("controlled background state create evidence changed", 73);
    }
  }
  const currentPlan = readStateRetirementPlan(
    evidenceDirectory,
    currentEvidence,
    paths,
  );
  if (!sameArtifactIdentity(currentPlan, planArtifact)) {
    fail("controlled background state retirement plan changed", 73);
  }
  const currentPrefix = inspectStateRetirementProgress(
    evidenceDirectory,
    currentPlan,
  );
  if (
    currentPrefix.progress.length !== expectedProgress.length ||
    currentPrefix.progress.some(
      (artifact, index) => !sameArtifactIdentity(artifact, expectedProgress[index]),
    ) ||
    (expectedPendingPublication === undefined) !==
      (currentPrefix.pending === undefined) ||
    (expectedPendingPublication !== undefined &&
      !sameStatePublication(
        currentPrefix.pending.publication,
        expectedPendingPublication,
      ))
  ) {
    fail("controlled background state retirement progress changed", 73);
  }
  observeStateRetirementProcesses(currentEvidence, currentPlan.value, {
    hostagentRetirementRecorded: expectedProgress.length > 0,
    requireHostagentAbsent,
  });
  assertRootSubset(
    paths,
    planArtifact.value,
    subsetCompletedCount,
    additionallyCompleted,
  );
}

function invokeStateRetirementEffect({
  additionallyCompleted,
  authorityGate,
  checkpoint,
  evidence,
  evidenceDirectory,
  expectedProgress,
  fixtureId,
  planArtifact,
  providerBase,
  resources,
  subsetCompletedCount,
  nextAction,
}) {
  invokeRetirementAuthorityGate(
    authorityGate,
    Object.freeze({
      checkpoint,
      ...stateRetirementAuthorityBindingFields(planArtifact.value),
      cleanup_plan_sha256: planArtifact.sha256,
      completed_steps: expectedProgress.length,
      next_action: nextAction,
      next_resources: Object.freeze([...resources]),
      provider_identity_sha256: evidence.providerIdentity.sha256,
      ...statePublicationAuthorityFields(
        { disposition: "not-applicable" },
        "not-applicable",
      ),
      resource_identity_sha256: stateRetirementResourceIdentitySha256(
        planArtifact.value,
        resources,
      ),
      retirement_contract_sha256: CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT_SHA256,
    }),
    () =>
      revalidateStateRetirementExecution({
        additionallyCompleted,
        evidence,
        evidenceDirectory,
        expectedProgress,
        fixtureId,
        planArtifact,
        providerBase,
        requireHostagentAbsent:
          checkpoint !== "before-hostagent-shutdown-delivery",
        subsetCompletedCount,
      }),
  );
}

function publishStateRetirementProgress({
  authorityGate,
  evidence,
  evidenceDirectory,
  fixtureId,
  pending,
  planArtifact,
  progress,
  providerBase,
  recoveredAbsence,
}) {
  const step = planArtifact.value.retirement_steps[progress.length];
  if (step === undefined) {
    fail("controlled background state retirement was already complete");
  }
  const value = progressValue(planArtifact, progress, recoveredAbsence);
  const bytes = providerProcessBytes(value);
  if (pending?.artifact !== undefined && !pending.artifact.bytes.equals(bytes)) {
    fail("controlled background state retirement progress stage disagreed");
  }
  const revalidate = (publication) => () =>
    revalidateStateRetirementExecution({
      evidence,
      evidenceDirectory,
      expectedPendingPublication:
        publication.disposition === "absent" ||
        publication.disposition === "final"
          ? undefined
          : publication,
      expectedProgress:
        publication.disposition === "final"
          ? [...progress, publication.artifact]
          : progress,
      fixtureId,
      planArtifact,
      providerBase,
      requireHostagentAbsent: true,
      subsetCompletedCount: progress.length + 1,
    });
  return publishStateRetirementArtifact(
    evidenceDirectory,
    progressName(progress),
    value,
    (publication, phase) =>
      invokeRetirementAuthorityGate(
        authorityGate,
        Object.freeze({
          checkpoint: "before-retirement-progress-publication",
          ...stateRetirementAuthorityBindingFields(planArtifact.value),
          cleanup_plan_sha256: planArtifact.sha256,
          completed_steps: progress.length,
          next_action: "publish-retirement-progress",
          next_resources: Object.freeze([...step.resources]),
          provider_identity_sha256: evidence.providerIdentity.sha256,
          ...statePublicationAuthorityFields(
            publication,
            phase,
            progressName(progress),
            providerProcessDigest(bytes),
          ),
          resource_identity_sha256: stepIdentitySha256(
            planArtifact.value,
            step,
          ),
          retirement_contract_sha256:
            CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT_SHA256,
        }),
        revalidate(publication),
      ),
  );
}

function reassertStateRetirementFinalConsumption({
  authorityGate,
  evidence,
  evidenceDirectory,
  fixtureId,
  planArtifact,
  prefix,
  providerBase,
}) {
  if (prefix.pending !== undefined) return;
  const completedSteps = prefix.progress.length;
  const nextStep = planArtifact.value.retirement_steps[completedSteps];
  const additionallyCompleted = new Set(
    (nextStep?.resources ?? []).filter(
      (resource) => !resourceExists(rootPaths(providerBase, fixtureId), resource),
    ),
  );
  let checkpoint;
  let nextAction;
  let nextResources;
  let publicationTargetName;
  let publishedArtifact;
  let resourceIdentitySha256;
  if (completedSteps === 0) {
    checkpoint = "before-retirement-plan-publication";
    nextAction = "publish-retirement-plan";
    nextResources = Object.freeze(["provider-retirement-plan.json"]);
    publicationTargetName = "provider-retirement-plan.json";
    publishedArtifact = planArtifact;
    resourceIdentitySha256 = planArtifact.sha256;
  } else {
    const sequence = completedSteps - 1;
    const step = planArtifact.value.retirement_steps[sequence];
    checkpoint = "before-retirement-progress-publication";
    nextAction = "publish-retirement-progress";
    nextResources = Object.freeze([...step.resources]);
    publicationTargetName = `retirement-step-${String(sequence).padStart(2, "0")}.json`;
    publishedArtifact = prefix.progress[sequence];
    resourceIdentitySha256 = stepIdentitySha256(planArtifact.value, step);
  }
  const publication = inspectStateArtifactPublication(
    evidenceDirectory,
    publicationTargetName,
    `controlled background state ${publicationTargetName}`,
  );
  if (
    publication.disposition !== "final" ||
    !sameArtifactIdentity(publication.artifact, publishedArtifact)
  ) {
    fail("controlled background state retirement consumption frontier changed", 73);
  }
  invokeRetirementAuthorityGate(
    authorityGate,
    Object.freeze({
      checkpoint,
      ...stateRetirementAuthorityBindingFields(planArtifact.value),
      cleanup_plan_sha256: planArtifact.sha256,
      completed_steps: completedSteps === 0 ? 0 : completedSteps - 1,
      next_action: nextAction,
      next_resources: nextResources,
      provider_identity_sha256: evidence.providerIdentity.sha256,
      ...statePublicationAuthorityFields(
        publication,
        "before-final-consumption",
        publicationTargetName,
        publishedArtifact.sha256,
      ),
      resource_identity_sha256: resourceIdentitySha256,
      retirement_contract_sha256: CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT_SHA256,
    }),
    () =>
      revalidateStateRetirementExecution({
        additionallyCompleted,
        evidence,
        evidenceDirectory,
        expectedProgress: prefix.progress,
        fixtureId,
        planArtifact,
        providerBase,
        requireHostagentAbsent: completedSteps > 0,
        subsetCompletedCount: completedSteps,
      }),
  );
}

function validateStateRetirementArguments({
  crashAfterDeleteSyscallSequence,
  crashAfterDeleteSequence,
  crashAfterHostagentSettlement,
  stopAfterSequence,
}) {
  if (
    crashAfterDeleteSyscallSequence !== undefined &&
    (!Number.isSafeInteger(crashAfterDeleteSyscallSequence) ||
      crashAfterDeleteSyscallSequence < 1)
  ) {
    fail("controlled background state syscall crash sequence was refused", 64);
  }
  if (
    crashAfterDeleteSequence !== undefined &&
    (!Number.isSafeInteger(crashAfterDeleteSequence) ||
      crashAfterDeleteSequence < 1)
  ) {
    fail("controlled background state crash sequence was refused", 64);
  }
  if (typeof crashAfterHostagentSettlement !== "boolean") {
    fail("controlled background state hostagent crash point was refused", 64);
  }
  if (
    stopAfterSequence !== undefined &&
    (!Number.isSafeInteger(stopAfterSequence) || stopAfterSequence < 0)
  ) {
    fail("controlled background state stop sequence was refused", 64);
  }
}

export async function retireControlledBackgroundProviderWithAuthorityGate(
  {
    crashAfterDeleteSyscallSequence,
    crashAfterDeleteSequence,
    crashAfterHostagentSettlement = false,
    evidenceDirectory,
    fixtureId,
    providerBase,
    stopAfterSequence,
  },
  authorityGate,
) {
  validateStateRetirementArguments({
    crashAfterDeleteSyscallSequence,
    crashAfterDeleteSequence,
    crashAfterHostagentSettlement,
    stopAfterSequence,
  });
  if (typeof authorityGate !== "function") {
    fail("controlled background state retirement authority gate was refused", 70);
  }
  const paths = validateControlledBackgroundRoots({
    evidenceDirectory,
    fixtureId,
    providerBase,
  });
  const evidence = inspectControlledBackgroundProvider(evidenceDirectory, fixtureId);
  if (evidence.createAuthority.value.state_integration !== "mutation-journal-v2") {
    fail("controlled background state retirement integration was refused", 73);
  }
  const planArtifact = readStateRetirementPlan(
    evidenceDirectory,
    evidence,
    paths,
  );
  let prefix = inspectStateRetirementProgress(evidenceDirectory, planArtifact);
  if (prefix.progress.length > planArtifact.value.retirement_steps.length) {
    fail("controlled background state retirement progress overflowed");
  }
  const initialSettlementPublication = inspectStateArtifactPublication(
    evidenceDirectory,
    "provider-retirement-settlement.json",
    "controlled background state retirement settlement",
  );
  if (initialSettlementPublication.disposition !== "absent") {
    if (
      prefix.pending !== undefined ||
      prefix.progress.length !== planArtifact.value.retirement_steps.length
    ) {
      fail("controlled background state retirement settlement preceded completion");
    }
    if (initialSettlementPublication.artifact !== undefined) {
      validateStateRetirementSettlement(
        initialSettlementPublication.artifact.value,
        planArtifact,
        evidence,
        prefix.progress,
      );
    }
    if (initialSettlementPublication.stage !== undefined) {
      validateStateRetirementSettlement(
        initialSettlementPublication.stage.value,
        planArtifact,
        evidence,
        prefix.progress,
      );
    }
  }
  if (initialSettlementPublication.disposition === "absent") {
    reassertStateRetirementFinalConsumption({
      authorityGate,
      evidence,
      evidenceDirectory,
      fixtureId,
      planArtifact,
      prefix,
      providerBase,
    });
  }
  while (prefix.progress.length < planArtifact.value.retirement_steps.length) {
    const sequence = prefix.progress.length;
    const step = planArtifact.value.retirement_steps[sequence];
    const present = step.resources.map((resource) => resourceExists(paths, resource));
    if (prefix.pending !== undefined) {
      if (present.some(Boolean)) {
        fail("controlled background state retirement progress stage preceded mutation");
      }
      if (typeof prefix.pending.recoveredAbsence !== "boolean") {
        fail("controlled background state retirement progress stage was refused");
      }
      if (sequence === 0) {
        syncSocketAbsence(paths);
      } else {
        syncDeletionAbsence(paths, step);
      }
      assertRootSubset(paths, planArtifact.value, sequence + 1);
      publishStateRetirementProgress({
        authorityGate,
        evidence,
        evidenceDirectory,
        fixtureId,
        pending: prefix.pending,
        planArtifact,
        progress: prefix.progress,
        providerBase,
        recoveredAbsence: prefix.pending.recoveredAbsence,
      });
      prefix = inspectStateRetirementProgress(evidenceDirectory, planArtifact);
      if (stopAfterSequence === sequence) {
        return Object.freeze({
          complete: false,
          completed_steps: prefix.progress.length,
        });
      }
      continue;
    }
    if (sequence === 0) {
      const marker = readCanonicalArtifactOnly(
        paths.ownerMarker,
        "controlled background root owner",
      );
      validateRootOwner(marker.value, {
        createAuthoritySha256: evidence.createAuthority.sha256,
        fixtureId,
        paths,
      });
      if (marker.sha256 !== planArtifact.value.provider_root_owner_sha256) {
        fail("controlled background state root owner changed");
      }
      const authorizeEffect = ({
        additionallyCompleted,
        checkpoint,
        resource,
        step: authorizedStep,
      }) =>
        invokeStateRetirementEffect({
          additionallyCompleted,
          authorityGate,
          checkpoint,
          evidence,
          evidenceDirectory,
          expectedProgress: prefix.progress,
          fixtureId,
          nextAction:
            checkpoint === "before-stale-socket-unlink"
              ? "unlink-stale-socket"
              : authorizedStep.action,
          planArtifact,
          providerBase,
          resources:
            checkpoint === "before-stale-socket-unlink"
              ? [resource]
              : authorizedStep.resources,
          subsetCompletedCount: sequence,
        });
      const recoveredAbsence = await stopHostagent(
        paths,
        planArtifact,
        evidence,
        marker.value.ownership_nonce,
        authorizeEffect,
      );
      if (crashAfterHostagentSettlement) {
        fail("simulated controlled background state hostagent-settlement crash", 75);
      }
      assertRootSubset(paths, planArtifact.value, sequence + 1);
      publishStateRetirementProgress({
        authorityGate,
        evidence,
        evidenceDirectory,
        fixtureId,
        planArtifact,
        progress: prefix.progress,
        providerBase,
        recoveredAbsence,
      });
      prefix = inspectStateRetirementProgress(evidenceDirectory, planArtifact);
      if (stopAfterSequence === sequence) {
        return Object.freeze({
          complete: false,
          completed_steps: prefix.progress.length,
        });
      }
      continue;
    }
    if (present.some(Boolean) && !present.every(Boolean)) {
      fail("controlled background state retirement step was partial");
    }
    const recoveredAbsence = present.every((value) => !value);
    if (recoveredAbsence) {
      syncDeletionAbsence(paths, step);
      assertRootSubset(paths, planArtifact.value, sequence + 1);
    } else {
      assertRootSubset(paths, planArtifact.value, sequence);
      deleteStep(paths, planArtifact.value, step, ({ checkpoint }) =>
        invokeStateRetirementEffect({
          additionallyCompleted: new Set(),
          authorityGate,
          checkpoint,
          evidence,
          evidenceDirectory,
          expectedProgress: prefix.progress,
          fixtureId,
          nextAction: step.action,
          planArtifact,
          providerBase,
          resources: step.resources,
          subsetCompletedCount: sequence,
        }),
      );
      if (crashAfterDeleteSyscallSequence === sequence) {
        fail("simulated controlled background state pre-fsync retirement crash", 75);
      }
      syncDeletionAbsence(paths, step);
      assertRootSubset(paths, planArtifact.value, sequence + 1);
      if (crashAfterDeleteSequence === sequence) {
        fail("simulated controlled background state retirement crash", 75);
      }
    }
    publishStateRetirementProgress({
      authorityGate,
      evidence,
      evidenceDirectory,
      fixtureId,
      planArtifact,
      progress: prefix.progress,
      providerBase,
      recoveredAbsence,
    });
    prefix = inspectStateRetirementProgress(evidenceDirectory, planArtifact);
    if (stopAfterSequence === sequence) {
      return Object.freeze({
        complete: false,
        completed_steps: prefix.progress.length,
      });
    }
  }
  assertRootSubset(paths, planArtifact.value, prefix.progress.length);
  const expectedSettlementValue = stateRetirementSettlementValue(
    planArtifact,
    evidence,
    prefix.progress,
  );
  const expectedSettlementBytes = providerProcessBytes(expectedSettlementValue);
  const expectedSettlementSha256 = providerProcessDigest(expectedSettlementBytes);
  const revalidateSettlement = (expectedPublication) => () => {
    revalidateStateRetirementExecution({
      evidence,
      evidenceDirectory,
      expectedProgress: prefix.progress,
      fixtureId,
      planArtifact,
      providerBase,
      requireHostagentAbsent: true,
      subsetCompletedCount: prefix.progress.length,
    });
    const current = inspectStateArtifactPublication(
      evidenceDirectory,
      "provider-retirement-settlement.json",
      "controlled background state retirement settlement",
    );
    if (!sameStatePublication(current, expectedPublication)) {
      fail("controlled background state retirement settlement changed", 73);
    }
  };
  const settlement = publishStateRetirementArtifact(
    evidenceDirectory,
    "provider-retirement-settlement.json",
    expectedSettlementValue,
    (publication, phase) =>
        invokeRetirementAuthorityGate(
          authorityGate,
          Object.freeze({
            checkpoint: "before-retirement-settlement-publication",
            ...stateRetirementAuthorityBindingFields(planArtifact.value),
            cleanup_plan_sha256: planArtifact.sha256,
            completed_steps: prefix.progress.length,
            next_action: "publish-retirement-settlement",
            next_resources: Object.freeze([
              "provider-retirement-settlement.json",
            ]),
            provider_identity_sha256: evidence.providerIdentity.sha256,
            ...statePublicationAuthorityFields(
              publication,
              phase,
              "provider-retirement-settlement.json",
              expectedSettlementSha256,
            ),
            resource_identity_sha256: expectedSettlementSha256,
            retirement_contract_sha256:
              CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT_SHA256,
          }),
          revalidateSettlement(publication),
        ),
  );
  validateStateRetirementSettlement(
    settlement.value,
    planArtifact,
    evidence,
    prefix.progress,
  );
  assertRootSubset(paths, planArtifact.value, prefix.progress.length);
  return Object.freeze({
    cleanup_operation_evidence_sha256: settlement.sha256,
    complete: true,
    completed_steps: prefix.progress.length,
    create_operation_evidence_sha256: evidence.providerIdentity.sha256,
    settlement,
  });
}

function stateArtifactIdentitySha256(artifact) {
  if (artifact === undefined) return ZERO_SHA256;
  const metadata = artifact.metadata;
  return providerProcessDigest(providerProcessBytes({
    actual_sha256: artifact.sha256,
    device: String(metadata.dev),
    inode: String(metadata.ino),
    links: String(metadata.nlink),
    mode: (metadata.mode & 0o7777n).toString(8).padStart(4, "0"),
    size: String(metadata.size),
    uid: String(metadata.uid),
  }));
}

function statePublicationObservation(publication) {
  if (publication === undefined) {
    return Object.freeze({
      disposition: "absent",
      final_identity_sha256: ZERO_SHA256,
      final_sha256: ZERO_SHA256,
      stage_actual_sha256: ZERO_SHA256,
      stage_declared_sha256: ZERO_SHA256,
      stage_identity_sha256: ZERO_SHA256,
    });
  }
  return Object.freeze({
    disposition: publication.disposition,
    final_identity_sha256: stateArtifactIdentitySha256(publication.artifact),
    final_sha256: publication.artifact?.sha256 ?? ZERO_SHA256,
    stage_actual_sha256:
      publication.stageSnapshot?.actual_sha256 ?? ZERO_SHA256,
    stage_declared_sha256:
      publication.stageSnapshot?.declared_sha256 ?? ZERO_SHA256,
    stage_identity_sha256:
      publication.stageSnapshot?.identity_sha256 ?? ZERO_SHA256,
  });
}

function currentRetirementRootObservation(expectedEntries, current) {
  const currentPaths = new Set(current.entries.map((entry) => entry.relative_path));
  const absentResources = expectedEntries
    .map((entry) => entry.relative_path)
    .filter((resource) => !currentPaths.has(resource))
    .sort();
  if (current.rootAbsent) absentResources.push(".");
  return Object.freeze({
    absentResources: Object.freeze(absentResources),
    inventorySha256: providerProcessDigest(providerProcessBytes({
      entries: current.entries,
      root: current.rootAbsent ? null : current.root,
    })),
    rootDisposition: current.rootAbsent ? "retired" : "owned",
  });
}

function inspectPlannedRetirementRoot(
  paths,
  plan,
  completedSteps,
  nextStep,
  pendingProgress,
) {
  let current;
  if (nextStep === undefined) {
    current = assertRootSubset(paths, plan, completedSteps);
  } else {
    const absent = nextStep.resources.filter(
      (resource) => !resourceExists(paths, resource),
    );
    if (
      completedSteps > 0 &&
      absent.length > 0 &&
      absent.length !== nextStep.resources.length
    ) {
      fail("controlled background state retirement step was partial");
    }
    if (pendingProgress !== undefined && absent.length !== nextStep.resources.length) {
      fail("controlled background state retirement progress stage preceded mutation");
    }
    current = assertRootSubset(
      paths,
      plan,
      completedSteps,
      new Set(absent),
    );
  }
  return currentRetirementRootObservation(
    plan.root_inventory,
    Object.freeze({
      entries: current.entries,
      root: current.rootAbsent ? undefined : plan.root,
      rootAbsent: current.rootAbsent,
    }),
  );
}

function retirementPrefixResult({
  cleanupPlan,
  cleanupStage,
  completedSteps,
  evidence,
  expectedCleanupPlanSha256,
  nextStep,
  pendingProgress,
  planPublication,
  processResidual,
  progress,
  providerSettlement,
  rootObservation,
  settlementPublication,
}) {
  const planObservation = statePublicationObservation(planPublication);
  const progressObservation = statePublicationObservation(
    pendingProgress?.publication,
  );
  const settlementObservation = statePublicationObservation(
    settlementPublication,
  );
  const nextPresence = Object.freeze(
    (nextStep?.resources ?? []).map((resource) => ({
      present: !rootObservation.absentResources.includes(resource),
      resource,
    })),
  );
  const nextDisposition =
    nextPresence.length === 0
      ? "complete"
      : nextPresence.every(({ present }) => present)
        ? "all-present"
        : nextPresence.every(({ present }) => !present)
          ? "all-absent"
          : "partial";
  const cleanupPlanSha256 =
    cleanupPlan?.sha256 ??
    planPublication.stageSnapshot?.declared_sha256 ??
    expectedCleanupPlanSha256 ??
    ZERO_SHA256;
  const finalProgressSha256 = progress.at(-1)?.sha256 ?? ZERO_SHA256;
  const providerSettlementSha256 = providerSettlement?.sha256 ?? ZERO_SHA256;
  const observation = Object.freeze({
    absent_resources: rootObservation.absentResources,
    cleanup_plan_sha256: cleanupPlanSha256,
    cleanup_stage: cleanupStage,
    completed_steps: completedSteps,
    create_evidence_head_sha256: evidence.providerIdentity.sha256,
    final_progress_sha256: finalProgressSha256,
    next_step:
      nextStep === undefined
        ? null
        : Object.freeze({
            action: nextStep.action,
            disposition: nextDisposition,
            resource_identity_sha256: stepIdentitySha256(
              cleanupPlan.value,
              nextStep,
            ),
            resources: Object.freeze([...nextStep.resources]),
            sequence: nextStep.sequence,
          }),
    plan_publication: planObservation,
    process_residual: processResidual,
    progress_publication: progressObservation,
    progress_publication_recovered_absence:
      pendingProgress?.recoveredAbsence ?? null,
    progress_publication_sequence: pendingProgress?.sequence ?? null,
    provider_settlement_sha256: providerSettlementSha256,
    remaining_inventory_sha256: rootObservation.inventorySha256,
    retirement_contract_sha256: CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT_SHA256,
    root_disposition: rootObservation.rootDisposition,
    schema: "synveda.clean-engine.retirement-prefix-observation.v2",
    settlement_publication: settlementObservation,
  });
  return Object.freeze({
    cleanupPlan,
    cleanupPlanPublication: planPublication,
    cleanupPlanSha256,
    cleanupStage,
    completedSteps,
    createEvidenceHeadSha256: evidence.providerIdentity.sha256,
    evidence,
    finalProgressSha256,
    observation,
    observationSha256: providerProcessDigest(providerProcessBytes(observation)),
    pendingProgress,
    processResidual,
    providerSettlement,
    providerSettlementPublication: settlementPublication,
    providerSettlementSha256,
    remainingInventorySha256: rootObservation.inventorySha256,
    rootDisposition: rootObservation.rootDisposition,
  });
}

function assertStateRetirementPlanBindings(plan, expectedBindings) {
  if (
    expectedBindings !== undefined &&
    canonical({
      cleanup_intent_sha256: plan.cleanup_intent_sha256,
      cleanup_operation_plan_sha256: plan.cleanup_operation_plan_sha256,
      cleanup_slot_sequence: plan.cleanup_slot_sequence,
      cleanup_slot_sha256: plan.cleanup_slot_sha256,
      create_close_sha256: plan.create_close_sha256,
      create_settlement_sha256: plan.create_settlement_sha256,
      create_slot_sha256: plan.create_slot_sha256,
      source_head_sha256: plan.source_head_sha256,
      source_sequence: plan.source_sequence,
    }) !== canonical(expectedBindings)
  ) {
    fail("controlled background state retirement bindings changed");
  }
}

export function inspectControlledBackgroundRetirementPrefix(
  evidenceDirectory,
  fixtureId,
  { expectedBindings, providerBase } = {},
) {
  if (expectedBindings === undefined) {
    fail("controlled background state retirement inspection bindings were required", 64);
  }
  validateStateRetirementBindings(expectedBindings);
  const paths = validateControlledBackgroundRoots({
    evidenceDirectory,
    fixtureId,
    providerBase,
  });
  const evidence = inspectControlledBackgroundProvider(evidenceDirectory, fixtureId);
  if (evidence.createAuthority.value.state_integration !== "mutation-journal-v2") {
    fail("controlled background state retirement inspection was refused", 73);
  }
  const planPublication = inspectStateArtifactPublication(
    evidenceDirectory,
    "provider-retirement-plan.json",
    "controlled background state retirement plan",
  );
  const planArtifact = planPublication.artifact ?? planPublication.stage;
  if (planPublication.disposition !== "final") {
    const unexpected = readdirSync(evidenceDirectory).some((name) => {
      const targetName = parseArtifactStageName(name)?.targetName ?? name;
      return /^(?:retirement-step-[0-9]{2}|provider-retirement-settlement)\.json$/.test(
        targetName,
      );
    });
    if (unexpected) {
      fail("controlled background state retirement evidence preceded its plan");
    }
    const owner = readCanonicalArtifactOnly(
      paths.ownerMarker,
      "controlled background root owner",
    );
    validateRootOwner(owner.value, {
      createAuthoritySha256: evidence.createAuthority.sha256,
      fixtureId,
      paths,
    });
    if (
      owner.sha256 !== evidence.providerIdentity.value.root_owner_sha256 ||
      owner.sha256 !== evidence.controllerWitness.value.root_owner_sha256
    ) {
      fail("controlled background state root owner binding was refused");
    }
    const pidRecord = revalidateHostagentPidRecord(
      paths,
      fixtureId,
      evidence,
      owner.value.ownership_nonce,
    );
    const hostagentPresence = processPresence(pidRecord.value.pid);
    const controllerPresence = probeProcessGroup(
      evidence.controllerWitness.value.controller_pgid,
    );
    if (hostagentPresence === "unknown" || controllerPresence !== "absent") {
      fail("controlled background state process identity remained uncertain", 73);
    }
    const inventory = creationInventoryForPlanning(
      paths,
      evidence.providerIdentity.value,
      hostagentPresence === "absent",
    );
    const expectedPlanValue = stateRetirementPlanValue({
      bindings: expectedBindings,
      evidence,
      fixtureId,
      inventory,
      owner,
      pidRecord,
      providerBase,
    });
    const expectedPlanBytes = providerProcessBytes(expectedPlanValue);
    assertStatePublicationExpected(
      planPublication,
      expectedPlanBytes,
      "controlled background state retirement plan",
    );
    if (planArtifact !== undefined) {
      validateStateRetirementPlan(
        planArtifact.value,
        evidence,
        paths,
        planArtifact.sha256,
      );
      assertStateRetirementPlanBindings(planArtifact.value, expectedBindings);
    }
    const current = scanRootInventory(paths);
    const rootObservation = currentRetirementRootObservation(
      evidence.providerIdentity.value.provider_root_inventory,
      Object.freeze({ ...current, rootAbsent: false }),
    );
    const repeatedOwner = readCanonicalArtifactOnly(
      paths.ownerMarker,
      "controlled background root owner",
    );
    validateRootOwner(repeatedOwner.value, {
      createAuthoritySha256: evidence.createAuthority.sha256,
      fixtureId,
      paths,
    });
    const repeatedPidRecord = revalidateHostagentPidRecord(
      paths,
      fixtureId,
      evidence,
      repeatedOwner.value.ownership_nonce,
    );
    const repeatedHostagentPresence = processPresence(
      repeatedPidRecord.value.pid,
    );
    const repeatedControllerPresence = probeProcessGroup(
      evidence.controllerWitness.value.controller_pgid,
    );
    creationInventoryForPlanning(
      paths,
      evidence.providerIdentity.value,
      repeatedHostagentPresence === "absent",
    );
    const repeatedCurrent = scanRootInventory(paths);
    const repeatedRootObservation = currentRetirementRootObservation(
      evidence.providerIdentity.value.provider_root_inventory,
      Object.freeze({ ...repeatedCurrent, rootAbsent: false }),
    );
    if (
      repeatedHostagentPresence !== hostagentPresence ||
      repeatedControllerPresence !== controllerPresence ||
      canonical(repeatedRootObservation) !== canonical(rootObservation)
    ) {
      fail("controlled background state retirement observation changed", 73);
    }
    const repeatedEvidence = inspectControlledBackgroundProvider(
      evidenceDirectory,
      fixtureId,
    );
    const repeatedPlanPublication = inspectStateArtifactPublication(
      evidenceDirectory,
      "provider-retirement-plan.json",
      "controlled background state retirement plan",
    );
    if (
      canonical(
        directoryIdentity(providerBase, "controlled background provider base"),
      ) !== canonical(expectedPlanValue.base) ||
      !sameArtifactIdentity(owner, repeatedOwner) ||
      !sameArtifactIdentity(pidRecord, repeatedPidRecord) ||
      Object.keys(evidence).some((name) =>
        !sameArtifactIdentity(evidence[name], repeatedEvidence[name])) ||
      !sameStatePublication(planPublication, repeatedPlanPublication) ||
      readdirSync(evidenceDirectory).some((name) => {
        const targetName = parseArtifactStageName(name)?.targetName ?? name;
        return /^(?:retirement-step-[0-9]{2}|provider-retirement-settlement)\.json$/.test(
          targetName,
        );
      })
    ) {
      fail("controlled background state retirement observation changed", 73);
    }
    assertStatePublicationExpected(
      repeatedPlanPublication,
      expectedPlanBytes,
      "controlled background state retirement plan",
    );
    return retirementPrefixResult({
      cleanupPlan: planArtifact,
      cleanupStage:
        planPublication.disposition === "absent"
          ? "not-started"
          : "plan-publication-pending",
      completedSteps: 0,
      evidence,
      expectedCleanupPlanSha256: providerProcessDigest(expectedPlanBytes),
      nextStep: undefined,
      pendingProgress: undefined,
      planPublication,
      processResidual: Object.freeze({
        controller_pgid: evidence.controllerWitness.value.controller_pgid,
        controller_presence: processObservation(controllerPresence),
        controller_process_instance_sha256:
          evidence.controllerWitness.value.controller_process_instance_sha256,
        hostagent_pid: pidRecord.value.pid,
        hostagent_presence: processObservation(hostagentPresence),
        hostagent_process_instance_sha256:
          evidence.hostagentWitness.value.process_instance_sha256,
      }),
      progress: Object.freeze([]),
      providerSettlement: undefined,
      rootObservation,
      settlementPublication: undefined,
    });
  }
  validateStateRetirementPlan(
    planArtifact.value,
    evidence,
    paths,
    planArtifact.sha256,
  );
  assertStateRetirementPlanBindings(planArtifact.value, expectedBindings);
  const progressPrefix = inspectStateRetirementProgress(
    evidenceDirectory,
    planArtifact,
  );
  const completedSteps = progressPrefix.progress.length;
  const nextStep = planArtifact.value.retirement_steps[completedSteps];
  const rootObservation = inspectPlannedRetirementRoot(
    paths,
    planArtifact.value,
    completedSteps,
    nextStep,
    progressPrefix.pending,
  );
  const processOptions = {
    hostagentRetirementRecorded: completedSteps > 0,
    requireHostagentAbsent:
      completedSteps > 0 ||
      nextStep?.resources.every((resource) =>
        rootObservation.absentResources.includes(resource),
      ) === true,
  };
  const processResidual = observeStateRetirementProcesses(
    evidence,
    planArtifact.value,
    processOptions,
  );
  const repeatedRootObservation = inspectPlannedRetirementRoot(
    paths,
    planArtifact.value,
    completedSteps,
    nextStep,
    progressPrefix.pending,
  );
  const repeatedProcessResidual = observeStateRetirementProcesses(
    evidence,
    planArtifact.value,
    processOptions,
  );
  if (
    canonical(repeatedRootObservation) !== canonical(rootObservation) ||
    canonical(repeatedProcessResidual) !== canonical(processResidual)
  ) {
    fail("controlled background state retirement observation changed", 73);
  }
  const settlementPublication = inspectStateArtifactPublication(
    evidenceDirectory,
    "provider-retirement-settlement.json",
    "controlled background state retirement settlement",
  );
  const providerSettlement =
    settlementPublication.artifact ?? settlementPublication.stage;
  if (settlementPublication.disposition !== "absent") {
    if (nextStep !== undefined || progressPrefix.pending !== undefined) {
      fail("controlled background state retirement settlement preceded completion");
    }
    assertStatePublicationExpected(
      settlementPublication,
      providerProcessBytes(
        stateRetirementSettlementValue(
          planArtifact,
          evidence,
          progressPrefix.progress,
        ),
      ),
      "controlled background state retirement settlement",
    );
    if (settlementPublication.artifact !== undefined) {
      validateStateRetirementSettlement(
        settlementPublication.artifact.value,
        planArtifact,
        evidence,
        progressPrefix.progress,
      );
    }
    if (settlementPublication.stage !== undefined) {
      validateStateRetirementSettlement(
        settlementPublication.stage.value,
        planArtifact,
        evidence,
        progressPrefix.progress,
      );
    }
  }
  const cleanupStage =
    settlementPublication.disposition !== "absent" &&
    settlementPublication.disposition !== "final"
      ? "settlement-publication-pending"
      : settlementPublication.disposition === "final"
        ? "settled"
        : progressPrefix.pending !== undefined
          ? "progress-publication-pending"
          : nextStep === undefined
            ? "progress-complete"
            : "retiring";
  const repeatedEvidence = inspectControlledBackgroundProvider(
    evidenceDirectory,
    fixtureId,
  );
  const repeatedPlanPublication = inspectStateArtifactPublication(
    evidenceDirectory,
    "provider-retirement-plan.json",
    "controlled background state retirement plan",
  );
  const repeatedProgressPrefix = inspectStateRetirementProgress(
    evidenceDirectory,
    planArtifact,
  );
  const repeatedSettlementPublication = inspectStateArtifactPublication(
    evidenceDirectory,
    "provider-retirement-settlement.json",
    "controlled background state retirement settlement",
  );
  if (
    Object.keys(evidence).some((name) =>
      !sameArtifactIdentity(evidence[name], repeatedEvidence[name])) ||
    !sameStatePublication(planPublication, repeatedPlanPublication) ||
    !sameStateRetirementProgress(progressPrefix, repeatedProgressPrefix) ||
    !sameStatePublication(
      settlementPublication,
      repeatedSettlementPublication,
    )
  ) {
    fail("controlled background state retirement observation changed", 73);
  }
  if (repeatedSettlementPublication.disposition !== "absent") {
    assertStatePublicationExpected(
      repeatedSettlementPublication,
      providerProcessBytes(
        stateRetirementSettlementValue(
          planArtifact,
          evidence,
          progressPrefix.progress,
        ),
      ),
      "controlled background state retirement settlement",
    );
  }
  return retirementPrefixResult({
    cleanupPlan: planArtifact,
    cleanupStage,
    completedSteps,
    evidence,
    nextStep,
    pendingProgress: progressPrefix.pending,
    planPublication,
    processResidual,
    progress: progressPrefix.progress,
    providerSettlement,
    rootObservation,
    settlementPublication,
  });
}

export function controlledBackgroundOperationEvidence({ action, evidenceDirectory, fixtureId }) {
  const evidence = inspectControlledBackgroundProvider(evidenceDirectory, fixtureId);
  if (evidence.createAuthority.value.state_integration !== "fixture-only") {
    fail("controlled background operation evidence integration was refused", 73);
  }
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
